//! Classifiers built on classifiers. A `Graph` holds classifiers in order; each may
//! take some earlier classifiers' steered outputs as its extra inputs, so the graph is
//! acyclic by construction. Outputs turn back into lists (`memberships`), so a
//! classifier's verdicts can be the lists the next one is trained on.
//!
//! Feedback on a stacked result can stay with one classifier (`Classifier::feedback`,
//! the default: yes or no on that classifier's own weights) or run end to end
//! (`Graph::feedback`): the loss on the judged classifier backpropagates through its
//! extra inputs into the classifiers that produced them, scaled by each edge's coupling
//! (0 leaves the upstream classifier alone). `Graph::contributions` says how much each
//! classifier had to do with a result, so a player can be asked about each one.

use serde::{Deserialize, Serialize};

use super::device::Runner;
use super::kernels as k;
use super::model::{BlockT, HeadKind, Net, Trace};
use super::pool::Pool;
use super::steer::{self, Output};
use super::train::{self, Adam, AdamOpts, Example, Feedback, FeedbackOpts, Target};
use super::{Classifier, FEEDBACK_KEPT};

/// one classifier's head feeding another classifier
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: usize,
    pub head: usize,
    /// how much of the downstream gradient reaches `from` in end-to-end feedback
    pub coupling: f32,
}

impl Edge {
    pub fn new(from: usize, head: usize) -> Edge {
        Edge {
            from,
            head,
            coupling: 1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub clf: Classifier,
    pub inputs: Vec<Edge>,
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
}

/// one node on a tape: traces of its windows, its extra inputs, mean logits, steered outputs
type Taped = (Vec<Trace>, Vec<f32>, Vec<Vec<f32>>, Vec<Output>);

/// a forward pass through the nodes a result depends on
struct Tape {
    runs: Vec<Option<Taped>>,
}

/// what one classifier had to do with a result
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contribution {
    pub node: usize,
    pub name: String,
    /// ‖d(−ln p)/d(weights)‖ for the judged class
    pub grad_norm: f64,
    /// grad_norm over the sum of them
    pub share: f64,
    /// for the judged classifier's direct inputs: |Σ input·gradient| over its edge, as
    /// a share of all its extra inputs' (0 for the rest)
    pub input_share: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StackReport {
    /// the judged class's unsteered probability before and after
    pub before: f32,
    pub after: f32,
    /// per node, the gradient norm of the last step (0 for nodes not reached)
    pub grad_norms: Vec<f64>,
    pub losses: Vec<f64>,
}

impl Graph {
    /// adds a classifier fed by `inputs`; their widths must add up to its `extra`
    pub fn add(&mut self, name: &str, clf: Classifier, inputs: Vec<Edge>) -> Result<usize, String> {
        let mut width = 0;
        for e in &inputs {
            let n = self
                .nodes
                .get(e.from)
                .ok_or(format!("input {} is not an earlier node", e.from))?;
            width += n.clf.net.heads.get(e.head).ok_or("no such head")?.width();
        }
        if width != clf.net.cfg.extra {
            return Err(format!(
                "inputs give {width} values, the classifier takes {}",
                clf.net.cfg.extra
            ));
        }
        self.nodes.push(Node {
            name: name.into(),
            clf,
            inputs,
        });
        Ok(self.nodes.len() - 1)
    }

    /// the nodes `node` depends on, itself included
    fn needed(&self, node: usize) -> Vec<bool> {
        let mut need = vec![false; self.nodes.len()];
        need[node] = true;
        for i in (0..=node).rev() {
            if need[i] {
                for e in &self.nodes[i].inputs {
                    need[e.from] = true;
                }
            }
        }
        need
    }

    fn extra_from(&self, i: usize, outs: &[Option<Vec<Output>>]) -> Vec<f32> {
        let mut x = Vec::new();
        for e in &self.nodes[i].inputs {
            x.extend_from_slice(&outs[e.from].as_ref().expect("inputs come first")[e.head].probs);
        }
        x
    }

    /// the steered outputs of `node` and of everything it depends on (None elsewhere)
    pub fn outputs(&self, node: usize, text: &str) -> Vec<Option<Vec<Output>>> {
        let need = self.needed(node);
        let mut outs: Vec<Option<Vec<Output>>> = vec![None; self.nodes.len()];
        for i in 0..=node {
            if need[i] {
                let x = self.extra_from(i, &outs);
                outs[i] = Some(self.nodes[i].clf.predict(text, &x));
            }
        }
        outs
    }

    pub fn predict(&self, node: usize, text: &str) -> Vec<Output> {
        self.outputs(node, text).swap_remove(node).expect("computed")
    }

    /// `node`'s steered outputs and its pooled vector (what `Classifier::represent`
    /// gives: the trunk never sees the extra inputs) from one pass, its upstream run first
    pub fn read_node(&self, node: usize, text: &str) -> (Vec<Output>, Vec<f32>) {
        let need = self.needed(node);
        let mut outs: Vec<Option<Vec<Output>>> = vec![None; self.nodes.len()];
        for i in 0..node {
            if need[i] {
                let x = self.extra_from(i, &outs);
                outs[i] = Some(self.nodes[i].clf.predict(text, &x));
            }
        }
        let x = self.extra_from(node, &outs);
        let clf = &self.nodes[node].clf;
        let r = clf.run(text, &x, &clf.gains(), None);
        (clf.read(&r.logits), r.pooled)
    }

    /// `read_node` for many texts: each node reads every text before the next one
    /// starts, on its runner's device when it has one (all the texts' windows packed),
    /// else on the pool. The same values as `read_node` text by text, bit for bit on the
    /// CPU and on a device the probe finds exact.
    pub fn read_node_many(
        &self,
        node: usize,
        texts: &[&str],
        runners: &[Option<&Runner>],
        pool: &Pool,
    ) -> Vec<(Vec<Output>, Vec<f32>)> {
        let need = self.needed(node);
        let n = texts.len();
        let mut outs: Vec<Vec<Option<Vec<Output>>>> = vec![vec![None; self.nodes.len()]; n];
        let mut pooled: Vec<Vec<f32>> = vec![Vec::new(); n];
        for i in 0..=node {
            if !need[i] {
                continue;
            }
            let clf = &self.nodes[i].clf;
            let gains = clf.gains();
            let xs: Vec<Vec<f32>> = outs.iter().map(|o| self.extra_from(i, o)).collect();
            let runs: Vec<(Vec<Vec<f32>>, Vec<f32>)> = match runners.get(i).copied().flatten() {
                Some(r) => {
                    let bytes: Vec<&[u8]> = texts.iter().map(|t| t.as_bytes()).collect();
                    let xr: Vec<&[f32]> = xs.iter().map(Vec::as_slice).collect();
                    r.text_runs(&clf.net, &bytes, &xr, &gains)
                }
                None => {
                    let slots: Vec<std::sync::Mutex<Option<_>>> = texts.iter().map(|_| Default::default()).collect();
                    pool.run(n, &|t| {
                        let r = clf.run(texts[t], &xs[t], &gains, None);
                        *slots[t].lock().unwrap() = Some((r.logits, r.pooled));
                    });
                    slots.into_iter().map(|s| s.into_inner().unwrap().unwrap()).collect()
                }
            };
            for (t, (logits, p)) in runs.into_iter().enumerate() {
                outs[t][i] = Some(clf.read(&logits));
                if i == node {
                    pooled[t] = p;
                }
            }
        }
        outs.into_iter()
            .zip(pooled)
            .map(|(mut o, p)| (o.swap_remove(node).expect("computed"), p))
            .collect()
    }

    /// the extra inputs `node` gets for a text
    pub fn extra(&self, node: usize, text: &str) -> Vec<f32> {
        let outs = self.outputs(node, text);
        self.extra_from(node, &outs)
    }

    /// examples for `node` with their extra inputs filled in from the graph
    pub fn with_extras(&self, node: usize, data: &[Example], pool: &Pool) -> Vec<Example> {
        let slots: Vec<std::sync::Mutex<Vec<f32>>> = data.iter().map(|_| Default::default()).collect();
        pool.run(data.len(), &|i| {
            *slots[i].lock().unwrap() = self.extra(node, &data[i].text)
        });
        data.iter()
            .zip(slots)
            .map(|(e, s)| e.clone().with_extra(s.into_inner().unwrap()))
            .collect()
    }

    fn tape(&self, node: usize, text: &str) -> Tape {
        let need = self.needed(node);
        let mut runs: Vec<Option<_>> = (0..self.nodes.len()).map(|_| None).collect();
        let mut outs: Vec<Option<Vec<Output>>> = vec![None; self.nodes.len()];
        for i in 0..=node {
            if !need[i] {
                continue;
            }
            let clf = &self.nodes[i].clf;
            let x = self.extra_from(i, &outs);
            let (trs, avg) = train::forward_text(&clf.net, text.as_bytes(), &x, &clf.gains());
            let o = clf.read(&avg);
            outs[i] = Some(o.clone());
            runs[i] = Some((trs, x, avg, o));
        }
        Tape { runs }
    }

    /// gradients of a loss on `node`'s head logits for every node it reaches, through
    /// the edges scaled by their coupling (or by 1 when `full`); returns the per-node
    /// gradients and, for `node`, d(loss)/d(extra)
    fn grads(&self, node: usize, tape: &Tape, head: usize, dl: Vec<f32>, full: bool) -> (Vec<Option<Net>>, Vec<f32>) {
        let n = self.nodes.len();
        let mut pending: Vec<Option<Vec<Vec<f32>>>> = vec![None; n];
        let mut grads: Vec<Option<Net>> = (0..n).map(|_| None).collect();
        let mut first_dextra = Vec::new();
        let mut d0: Vec<Vec<f32>> = self.nodes[node].clf.net.heads.iter().map(|_| Vec::new()).collect();
        d0[head] = dl;
        pending[node] = Some(d0);
        for i in (0..=node).rev() {
            let Some(dls) = pending[i].take() else { continue };
            let Some((trs, ..)) = &tape.runs[i] else { continue };
            let clf = &self.nodes[i].clf;
            let tp: Vec<BlockT> = clf.net.transposes();
            let mut g = clf.net.zeros_like();
            let dextra = train::backward_text(&clf.net, trs, &dls, &tp, &mut g);
            grads[i] = Some(g);
            if i == node {
                first_dextra = dextra.clone();
            }
            let mut at = 0;
            for e in &self.nodes[i].inputs {
                let up = &self.nodes[e.from].clf;
                let w = up.net.heads[e.head].width();
                let c = if full { 1.0 } else { e.coupling };
                if c != 0.0 {
                    let dp: Vec<f32> = dextra[at..at + w].iter().map(|v| v * c).collect();
                    let uo = &tape.runs[e.from].as_ref().expect("taped").3[e.head];
                    let dlu = steer::back(up.net.heads[e.head].spec.kind, &up.steer.heads[e.head], uo, &dp);
                    let p = pending[e.from].get_or_insert_with(|| up.net.heads.iter().map(|_| Vec::new()).collect());
                    if p[e.head].is_empty() {
                        p[e.head] = dlu;
                    } else {
                        p[e.head].iter_mut().zip(dlu).for_each(|(a, b)| *a += b);
                    }
                }
                at += w;
            }
        }
        (grads, first_dextra)
    }

    fn target(&self, node: usize, f: &Feedback) -> Result<Target, String> {
        let kind = self.nodes[node]
            .clf
            .net
            .heads
            .get(f.head)
            .ok_or("no such head")?
            .spec
            .kind;
        Ok(f.example(kind)?.target)
    }

    /// how much each classifier had to do with `node` putting `text` in `class`
    pub fn contributions(
        &self,
        node: usize,
        head: usize,
        class: usize,
        text: &str,
    ) -> Result<Vec<Contribution>, String> {
        let tape = self.tape(node, text);
        let clf = &self.nodes[node].clf;
        let kind = clf.net.heads.get(head).ok_or("no such head")?.spec.kind;
        let target = if kind == HeadKind::Multi {
            Target::Label(class, true)
        } else {
            Target::Class(class)
        };
        let (_, _, avg, _) = tape.runs[node].as_ref().expect("taped");
        let (_, dl) = train::head_loss(kind, &avg[head], &target, 0.0)?;
        let (grads, dextra) = self.grads(node, &tape, head, dl, true);
        let norms: Vec<f64> = grads
            .iter()
            .map(|g| {
                g.as_ref().map_or(0.0, |g| {
                    g.params().iter().map(|(_, t)| k::dot(t, t) as f64).sum::<f64>().sqrt()
                })
            })
            .collect();
        let total: f64 = norms.iter().sum();
        let x = &tape.runs[node].as_ref().expect("taped").1;
        let mut edge_attr = vec![0f64; self.nodes.len()];
        let mut at = 0;
        for e in &self.nodes[node].inputs {
            let w = self.nodes[e.from].clf.net.heads[e.head].width();
            let s: f32 = x[at..at + w].iter().zip(&dextra[at..at + w]).map(|(a, b)| a * b).sum();
            edge_attr[e.from] += s.abs() as f64;
            at += w;
        }
        let attr_total: f64 = edge_attr.iter().sum();
        Ok((0..self.nodes.len())
            .filter(|&i| grads[i].is_some())
            .map(|i| Contribution {
                node: i,
                name: self.nodes[i].name.clone(),
                grad_norm: norms[i],
                share: if total > 0.0 { norms[i] / total } else { 0.0 },
                input_share: if attr_total > 0.0 {
                    edge_attr[i] / attr_total
                } else {
                    0.0
                },
            })
            .collect())
    }

    /// end-to-end feedback on `node`'s result: each step backpropagates the loss through
    /// the graph (edges scaled by coupling) and steps every reached classifier's own
    /// optimizer. The feedback is remembered by the judged classifier.
    pub fn feedback(&mut self, node: usize, f: &Feedback, o: &FeedbackOpts) -> Result<StackReport, String> {
        let target = self.target(node, f)?;
        let kind = self.nodes[node].clf.net.heads[f.head].spec.kind;
        let prob = |g: &Graph| -> f32 {
            let x = g.extra(node, &f.text);
            let c = &g.nodes[node].clf;
            train::feedback_prob(c, &Feedback { extra: x, ..f.clone() })
        };
        let mut rep = StackReport {
            before: prob(self),
            grad_norms: vec![0.0; self.nodes.len()],
            ..StackReport::default()
        };
        let adam_o = AdamOpts {
            b1: 0.9,
            b2: 0.98,
            eps: 1e-8,
            weight_decay: 0.0,
        };
        for _ in 0..o.steps {
            let tape = self.tape(node, &f.text);
            let (_, _, avg, _) = tape.runs[node].as_ref().expect("taped");
            let (loss, dl) = train::head_loss(kind, &avg[f.head], &target, 0.0)?;
            let dl: Vec<f32> = dl.iter().map(|v| v * f.weight).collect();
            rep.losses.push(loss as f64);
            let (grads, _) = self.grads(node, &tape, f.head, dl, false);
            drop(tape);
            for (i, g) in grads.into_iter().enumerate() {
                let Some(g) = g else { continue };
                let clf = &mut self.nodes[i].clf;
                let mask = train::trainable(&clf.net, o.freeze_trunk);
                let (scale, norm) = train::clip_scale(&g, o.clip, &mask);
                rep.grad_norms[i] = norm as f64;
                if !clf.opt.as_ref().is_some_and(|a| a.fits(&clf.net)) {
                    clf.opt = Some(Adam::new(&clf.net));
                }
                clf.opt
                    .as_mut()
                    .expect("set")
                    .step(&mut clf.net, &g, o.lr, &adam_o, scale, &mask, None);
                clf.meta.steps += 1;
            }
        }
        rep.after = prob(self);
        let x = self.extra(node, &f.text);
        let ex = Example {
            text: f.text.clone(),
            head: f.head,
            target,
            weight: f.weight,
            extra: x,
            raw: Vec::new(),
            also: Vec::new(),
        };
        let m = &mut self.nodes[node].clf.meta;
        m.feedback
            .retain(|e| !(e.text == ex.text && e.head == ex.head && e.target.class() == ex.target.class()));
        m.feedback.insert(0, ex);
        m.feedback.truncate(FEEDBACK_KEPT);
        Ok(rep)
    }
}

/// a head's outputs over some texts as lists: for each class, the indices of the texts
/// that are members of it
pub fn memberships(outs: &[Output], classes: usize) -> Vec<Vec<usize>> {
    let mut lists = vec![Vec::new(); classes];
    for (i, o) in outs.iter().enumerate() {
        for &c in &o.members {
            lists[c].push(i);
        }
    }
    lists
}

/// predicts a head over texts and returns its lists (see `memberships`)
pub fn lists(clf: &Classifier, head: usize, texts: &[&str], extras: &[Vec<f32>], pool: &Pool) -> Vec<Vec<usize>> {
    let outs: Vec<Output> = clf
        .predict_many(texts, extras, pool)
        .into_iter()
        .map(|mut o| o.swap_remove(head))
        .collect();
    memberships(&outs, clf.net.heads[head].width())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::tests::{synthetic, tiny_clf};
    use crate::classify::{Config, Flow, HeadSpec, TrainOpts};

    fn stack() -> (Graph, Vec<Example>) {
        let data = synthetic(60, 21);
        let o = TrainOpts {
            epochs: 3,
            lr: 3e-3,
            threads: 2,
            ..TrainOpts::default()
        };
        let mut a = tiny_clf(21);
        train::train(&mut a, &data, &o, |_| Flow::Go).unwrap();
        let mut g = Graph::default();
        g.add("kind", a, vec![]).unwrap();
        let cfg = Config {
            extra: 2,
            ..Config::tiny()
        };
        let b = Classifier::new(cfg.clone(), &[HeadSpec::single("hard", &["soft", "hard"])], 22).unwrap();
        assert!(g.add("bad", b.clone(), vec![]).is_err());
        assert!(g.add("bad", b.clone(), vec![Edge::new(5, 0)]).is_err());
        g.add("hard", b, vec![Edge::new(0, 0)]).unwrap();
        let pool = Pool::new(2);
        let bdata = g.with_extras(1, &data, &pool);
        assert!(bdata.iter().all(|e| e.extra.len() == 2));
        train::train(&mut g.nodes[1].clf, &bdata, &o, |_| Flow::Go).unwrap();
        (g, data)
    }

    #[test]
    fn outputs_become_lists() {
        let (g, data) = stack();
        let texts: Vec<&str> = data.iter().map(|e| e.text.as_str()).collect();
        let pool = Pool::new(2);
        let l = lists(&g.nodes[0].clf, 0, &texts, &[], &pool);
        assert_eq!(l.len(), 2);
        let right = l[1].iter().filter(|&&i| data[i].target == Target::Class(1)).count();
        assert!(right as f32 >= 0.8 * l[1].len() as f32 && !l[1].is_empty());
        let outs = g.outputs(1, "slate");
        assert!(outs[0].is_some() && outs[1].is_some());
        assert_eq!(g.predict(1, "slate"), outs[1].clone().unwrap());
    }

    #[test]
    fn stacked_feedback_reaches_upstream_by_coupling() {
        let (g0, _) = stack();
        let t = "plum quartz grape";
        let c = g0.contributions(1, 0, 1, t).unwrap();
        assert_eq!(c.len(), 2);
        assert!((c.iter().map(|x| x.share).sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(c[0].grad_norm > 0.0 && c[0].input_share == 1.0);
        let o = FeedbackOpts {
            steps: 4,
            lr: 2e-3,
            ..FeedbackOpts::default()
        };
        let no = Feedback::new(t, 1, false);
        let mut g = g0.clone();
        let r = g.feedback(1, &no, &o).unwrap();
        assert!(r.after < r.before, "{r:?}");
        assert!(r.grad_norms[0] > 0.0 && g.nodes[0].clf.net != g0.nodes[0].clf.net);
        assert_eq!(g.nodes[1].clf.meta.feedback.len(), 1);
        // the same feedback again gives the same bits
        let mut again = g0.clone();
        again.feedback(1, &no, &o).unwrap();
        assert_eq!(again.nodes[0].clf.to_bytes(), g.nodes[0].clf.to_bytes());
        assert_eq!(again.nodes[1].clf.to_bytes(), g.nodes[1].clf.to_bytes());
        // coupling 0: the upstream classifier is left alone
        let mut frozen = g0.clone();
        frozen.nodes[1].inputs[0].coupling = 0.0;
        frozen.feedback(1, &no, &o).unwrap();
        assert_eq!(frozen.nodes[0].clf.net, g0.nodes[0].clf.net);
        assert!(frozen.nodes[1].clf.net != g0.nodes[1].clf.net);
    }

    #[test]
    fn stack_gradient_matches_finite_differences() {
        let (mut g, _) = stack();
        g.nodes[0].clf.steer.heads[0].temperature = 1.7;
        g.nodes[0].clf.steer.heads[0].bias = vec![0.3, -0.2];
        let t = "plum quartz";
        let loss = |g: &Graph| -> f32 {
            let tape = g.tape(1, t);
            let avg = &tape.runs[1].as_ref().unwrap().2;
            train::head_loss(HeadKind::Single, &avg[0], &Target::Class(1), 0.0)
                .unwrap()
                .0
        };
        let tape = g.tape(1, t);
        let avg = tape.runs[1].as_ref().unwrap().2.clone();
        let (_, dl) = train::head_loss(HeadKind::Single, &avg[0], &Target::Class(1), 0.0).unwrap();
        let (grads, _) = g.grads(1, &tape, 0, dl, true);
        drop(tape);
        let ga: Vec<Vec<f32>> = grads[0]
            .as_ref()
            .unwrap()
            .params()
            .into_iter()
            .map(|(_, t)| t.clone())
            .collect();
        let eps = 4e-3;
        let mut checked = 0;
        for (ti, gt) in ga.iter().enumerate() {
            let Some(i) = gt.iter().position(|v| v.abs() > 1e-4) else {
                continue;
            };
            let orig = g.nodes[0].clf.net.params()[ti].1[i];
            g.nodes[0].clf.net.params_mut()[ti].1[i] = orig + eps;
            let up = loss(&g);
            g.nodes[0].clf.net.params_mut()[ti].1[i] = orig - eps;
            let dn = loss(&g);
            g.nodes[0].clf.net.params_mut()[ti].1[i] = orig;
            let num = (up - dn) / (2.0 * eps);
            assert!(
                (num - gt[i]).abs() <= 2e-3 + 5e-2 * num.abs().max(gt[i].abs()),
                "tensor {ti}: {num} vs {}",
                gt[i]
            );
            checked += 1;
        }
        assert!(checked > 5);
    }
}
