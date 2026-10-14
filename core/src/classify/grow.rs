//! Growing and shrinking a trained classifier. Widening and deepening preserve what
//! the classifier computes, so training picks up where it was:
//!
//! - **widen the feed-forward** (Net2WiderNet): a new SwiGLU unit copies an existing
//!   unit's gate and up weights, and the two split the old unit's outgoing row as α and
//!   1 − α (α drawn in [¼, ¾) so the twins get different gradients and drift apart).
//!   Same output up to rounding.
//! - **add attention heads**: fresh query, key and value weights, and zero rows in the
//!   output projection, so the new head adds exactly 0 until it learns.
//! - **deepen**: a new block whose two output projections are zero is the identity
//!   (x + 0 + 0), exactly.
//! - **prune**: drop the least important feed-forward units (mean |activation| × outgoing
//!   norm on sample texts) and heads (mean output norm × projection norm); accepted only
//!   if validation accuracy holds within a tolerance, otherwise reverted.
//!
//! `Grower` decides when: grow while accuracy is under target or falls (after new data),
//! prune when it has stayed above target with margin for several evaluations.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::Classifier;
use super::math::{Rng, key};
use super::model::{Block, Mode, Net};
use super::pool::Pool;
use super::train::{self, Evaluation, Example, Flow, Progress, TrainOpts, TrainReport};

/// rebuilds a row-major [rows × cols] matrix with columns picked from the old one
/// (`Some(c)`) or made fresh (`None` → `fresh(row)`)
fn columns(
    m: &[f32],
    rows: usize,
    cols: usize,
    pick: &[Option<usize>],
    mut fresh: impl FnMut(usize) -> f32,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(rows * pick.len());
    for r in 0..rows {
        for p in pick {
            out.push(match p {
                Some(c) => m[r * cols + c],
                None => fresh(r),
            });
        }
    }
    out
}

/// adds `add` SwiGLU units to block `b`, each a split copy of a random existing unit
pub fn widen_ff(net: &mut Net, b: usize, add: usize, seed: u64) {
    let d = net.cfg.d_model;
    let mut r = Rng::new(key(&[seed, 0x91de, b as u64]));
    let blk = &mut net.blocks[b];
    let ff = blk.ff;
    let mut pick: Vec<Option<usize>> = (0..ff).map(Some).collect();
    for _ in 0..add {
        let u = r.below(pick.len()); // any unit so far, twins included
        let src = pick[u].expect("all picks are copies");
        let alpha = 0.25 + 0.5 * r.unit();
        // the new unit's outgoing row (1 − α)·w, the source keeps α·w
        let row: Vec<f32> = blk.wd[u * d..(u + 1) * d].to_vec();
        blk.wd.extend(row.iter().map(|w| w * (1.0 - alpha)));
        blk.wd[u * d..(u + 1) * d].iter_mut().for_each(|w| *w *= alpha);
        pick.push(Some(src));
    }
    blk.wg = columns(&blk.wg, d, ff, &pick, |_| 0.0);
    blk.wu = columns(&blk.wu, d, ff, &pick, |_| 0.0);
    blk.ff = pick.len();
}

/// adds `add` attention heads to block `b`; their output rows are zero
pub fn add_heads(net: &mut Net, b: usize, add: usize, seed: u64) {
    let (d, dh) = (net.cfg.d_model, net.cfg.head_dim);
    let mut r = Rng::new(key(&[seed, 0x4ead, b as u64]));
    let blk = &mut net.blocks[b];
    let a = blk.heads * dh;
    let na = a + add * dh;
    let mut pick = Vec::with_capacity(3 * na);
    for s in 0..3 {
        pick.extend((0..a).map(|c| Some(s * a + c)));
        pick.extend((a..na).map(|_| None));
    }
    let std = 3f32.sqrt() / (d as f32).sqrt();
    blk.wqkv = columns(&blk.wqkv, d, 3 * a, &pick, |_| r.symmetric(std));
    blk.wo.extend(std::iter::repeat_n(0.0, add * dh * d));
    blk.heads += add;
}

/// inserts an identity block (zero output projections) before block `at`
pub fn deepen(net: &mut Net, at: usize, seed: u64) {
    let mut r = Rng::new(key(&[seed, 0xdee9, at as u64, net.blocks.len() as u64]));
    let blk = Block::new(&net.cfg, &mut r, true);
    net.blocks.insert(at.min(net.blocks.len()), blk);
    net.cfg.layers = net.blocks.len();
}

/// keeps only the listed SwiGLU units of block `b` (in the order given)
pub fn prune_ff(net: &mut Net, b: usize, keep: &[usize]) {
    let d = net.cfg.d_model;
    let blk = &mut net.blocks[b];
    let pick: Vec<Option<usize>> = keep.iter().map(|&u| Some(u)).collect();
    blk.wg = columns(&blk.wg, d, blk.ff, &pick, |_| 0.0);
    blk.wu = columns(&blk.wu, d, blk.ff, &pick, |_| 0.0);
    blk.wd = keep.iter().flat_map(|&u| blk.wd[u * d..(u + 1) * d].to_vec()).collect();
    blk.ff = keep.len();
}

/// keeps only the listed heads of block `b`
pub fn prune_heads(net: &mut Net, b: usize, keep: &[usize]) {
    let (d, dh) = (net.cfg.d_model, net.cfg.head_dim);
    let blk = &mut net.blocks[b];
    let a = blk.heads * dh;
    let mut pick = Vec::new();
    for s in 0..3 {
        for &h in keep {
            pick.extend((0..dh).map(|i| Some(s * a + h * dh + i)));
        }
    }
    blk.wqkv = columns(&blk.wqkv, d, 3 * a, &pick, |_| 0.0);
    blk.wo = keep
        .iter()
        .flat_map(|&h| blk.wo[h * dh * d..(h + 1) * dh * d].to_vec())
        .collect();
    blk.heads = keep.len();
}

/// per block, how much each SwiGLU unit and each head matters on some texts
#[derive(Clone, Debug, PartialEq)]
pub struct Importance {
    pub ff: Vec<Vec<f64>>,
    pub heads: Vec<Vec<f64>>,
}

fn norm(v: impl Iterator<Item = f32>) -> f64 {
    v.map(|x| (x as f64) * (x as f64)).sum::<f64>().sqrt()
}

/// unit u: mean over tokens of |activation| times ‖outgoing row‖; head h: mean over
/// tokens of ‖its output‖ times ‖its projection rows‖. Texts run in parallel and add
/// up in text order.
pub fn importance(net: &Net, texts: &[&str], pool: &Pool) -> Importance {
    let (d, dh) = (net.cfg.d_model, net.cfg.head_dim);
    let empty = || Importance {
        ff: net.blocks.iter().map(|b| vec![0.0; b.ff]).collect(),
        heads: net.blocks.iter().map(|b| vec![0.0; b.heads]).collect(),
    };
    let extra = vec![0.0; net.cfg.extra];
    let slots: Vec<Mutex<Option<(Importance, usize)>>> = texts.iter().map(|_| Mutex::new(None)).collect();
    pool.run(texts.len(), &|i| {
        let mut imp = empty();
        let mut tokens = 0;
        for w in super::text::windows(texts[i], net.cfg.max_len, net.cfg.stride) {
            let tr = net.forward(&w, &extra, &net.cfg.gains, Mode::EVAL, None);
            tokens += tr.n;
            for (l, b) in net.blocks.iter().enumerate() {
                for row in tr.act(l).chunks_exact(b.ff) {
                    imp.ff[l].iter_mut().zip(row).for_each(|(a, x)| *a += x.abs() as f64);
                }
                let a = b.heads * dh;
                for row in tr.ctx(l).chunks_exact(a) {
                    for h in 0..b.heads {
                        imp.heads[l][h] += norm(row[h * dh..(h + 1) * dh].iter().copied());
                    }
                }
            }
        }
        *slots[i].lock().unwrap() = Some((imp, tokens));
    });
    let mut total = empty();
    let mut tokens = 0usize;
    for s in slots {
        let (imp, t) = s.into_inner().unwrap().unwrap();
        tokens += t;
        for (a, b) in total.ff.iter_mut().flatten().zip(imp.ff.iter().flatten()) {
            *a += b;
        }
        for (a, b) in total.heads.iter_mut().flatten().zip(imp.heads.iter().flatten()) {
            *a += b;
        }
    }
    let t = tokens.max(1) as f64;
    for (l, b) in net.blocks.iter().enumerate() {
        for u in 0..b.ff {
            total.ff[l][u] = total.ff[l][u] / t * norm(b.wd[u * d..(u + 1) * d].iter().copied());
        }
        for h in 0..b.heads {
            total.heads[l][h] = total.heads[l][h] / t * norm(b.wo[h * dh * d..(h + 1) * dh * d].iter().copied());
        }
    }
    total
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PruneOpts {
    /// share of each block's SwiGLU units to drop
    pub fraction: f32,
    /// also drop each block's least important head when it is under half the mean
    pub heads: bool,
    pub min_ff: usize,
    /// how much validation accuracy may fall for the pruning to stand
    pub tolerance: f64,
}

impl Default for PruneOpts {
    fn default() -> PruneOpts {
        PruneOpts {
            fraction: 0.2,
            heads: true,
            min_ff: 8,
            tolerance: 0.01,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PruneReport {
    pub accepted: bool,
    pub before: f64,
    pub after: f64,
    pub units: usize,
    pub heads: usize,
}

/// drops the least important units (and heads), keeps the result only if accuracy on
/// `data[val]` stays within the tolerance
pub fn prune(clf: &mut Classifier, data: &[Example], val: &[usize], o: &PruneOpts, pool: &Pool) -> PruneReport {
    let before_eval = train::evaluate(clf, data, val, pool);
    let texts: Vec<&str> = val.iter().take(64).map(|&i| data[i].text.as_str()).collect();
    let imp = importance(&clf.net, &texts, pool);
    let saved = clf.net.clone();
    let mut rep = PruneReport {
        before: before_eval.accuracy(),
        ..PruneReport::default()
    };
    for l in 0..clf.net.blocks.len() {
        let ff = clf.net.blocks[l].ff;
        let drop = ((ff as f32 * o.fraction) as usize).min(ff.saturating_sub(o.min_ff));
        if drop > 0 {
            let mut order: Vec<usize> = (0..ff).collect();
            order.sort_by(|&a, &b| imp.ff[l][a].total_cmp(&imp.ff[l][b]).then(a.cmp(&b)));
            let mut keep: Vec<usize> = order[drop..].to_vec();
            keep.sort_unstable();
            prune_ff(&mut clf.net, l, &keep);
            rep.units += drop;
        }
        let hs = &imp.heads[l];
        if o.heads && hs.len() > 1 {
            let mean = hs.iter().sum::<f64>() / hs.len() as f64;
            let (worst, &w) = hs.iter().enumerate().min_by(|a, b| a.1.total_cmp(b.1)).unwrap();
            if w < 0.5 * mean {
                let keep: Vec<usize> = (0..hs.len()).filter(|&h| h != worst).collect();
                prune_heads(&mut clf.net, l, &keep);
                rep.heads += 1;
            }
        }
    }
    rep.after = train::evaluate(clf, data, val, pool).accuracy();
    rep.accepted = rep.after >= rep.before - o.tolerance;
    if !rep.accepted {
        clf.net = saved;
    } else {
        clf.meta.log.push(format!(
            "pruned {} units, {} heads: {:.4} -> {:.4}",
            rep.units, rep.heads, rep.before, rep.after
        ));
    }
    rep
}

// ---------------------------------------------------------------- policy

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Keep,
    /// widen every block's feed-forward
    Widen,
    /// one more head in every block
    AddHead,
    /// one more block at the top
    Deepen,
    /// try pruning (kept only if accuracy holds)
    Prune,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grower {
    /// the validation accuracy aimed for
    pub target: f64,
    /// accuracy above target + margin counts as spare capacity
    pub margin: f64,
    /// evaluations in a row with spare capacity before pruning
    pub hold: usize,
    /// a fall from the best accuracy this large counts as falling
    pub drop: f64,
    /// evaluations to wait after any change
    pub cooldown: usize,
    /// feed-forward growth per widening, as a share of the current width
    pub widen_by: f32,
    pub max_layers: usize,
    pub max_ff: usize,
    pub max_heads: usize,
    pub prune: PruneOpts,
    best: f64,
    streak: usize,
    wait: usize,
    escalation: usize,
}

impl Grower {
    pub fn new(target: f64) -> Grower {
        Grower {
            target,
            margin: 0.03,
            hold: 3,
            drop: 0.03,
            cooldown: 1,
            widen_by: 0.5,
            max_layers: 8,
            max_ff: 2048,
            max_heads: 16,
            prune: PruneOpts::default(),
            best: 0.0,
            streak: 0,
            wait: 0,
            escalation: 0,
        }
    }

    /// new data arrived: what counts as falling starts over
    pub fn data_added(&mut self) {
        self.best = 0.0;
        self.streak = 0;
    }

    fn allowed(&self, a: Action, net: &Net) -> bool {
        match a {
            Action::Widen => net.blocks.iter().any(|b| b.ff < self.max_ff),
            Action::AddHead => net.blocks.iter().any(|b| b.heads < self.max_heads),
            Action::Deepen => net.blocks.len() < self.max_layers,
            _ => true,
        }
    }

    /// what to do after an evaluation with this accuracy
    pub fn observe(&mut self, acc: f64, net: &Net) -> Action {
        let fell = acc < self.best - self.drop;
        self.best = self.best.max(acc);
        if self.wait > 0 {
            self.wait -= 1;
            return Action::Keep;
        }
        if acc < self.target || fell {
            self.streak = 0;
            const LADDER: [Action; 3] = [Action::Widen, Action::AddHead, Action::Deepen];
            for i in 0..LADDER.len() {
                let a = LADDER[(self.escalation + i) % LADDER.len()];
                if self.allowed(a, net) {
                    self.escalation += i + 1;
                    self.wait = self.cooldown;
                    return a;
                }
            }
            return Action::Keep;
        }
        self.escalation = 0;
        if acc >= self.target + self.margin {
            self.streak += 1;
            if self.streak >= self.hold {
                self.streak = 0;
                self.wait = self.cooldown;
                return Action::Prune;
            }
        } else {
            self.streak = 0;
        }
        Action::Keep
    }

    /// carries out an action; returns a line for the log (empty for Keep)
    pub fn apply(
        &self,
        clf: &mut Classifier,
        a: Action,
        data: &[Example],
        val: &[usize],
        seed: u64,
        pool: &Pool,
    ) -> String {
        let s = key(&[seed, clf.meta.steps]);
        let line = match a {
            Action::Keep => return String::new(),
            Action::Widen => {
                for l in 0..clf.net.blocks.len() {
                    let ff = clf.net.blocks[l].ff;
                    let add = ((ff as f32 * self.widen_by) as usize)
                        .max(4)
                        .min(self.max_ff.saturating_sub(ff));
                    widen_ff(&mut clf.net, l, add, s);
                }
                format!(
                    "widened feed-forward to {:?}",
                    clf.net.blocks.iter().map(|b| b.ff).collect::<Vec<_>>()
                )
            }
            Action::AddHead => {
                for l in 0..clf.net.blocks.len() {
                    if clf.net.blocks[l].heads < self.max_heads {
                        add_heads(&mut clf.net, l, 1, s);
                    }
                }
                format!(
                    "heads now {:?}",
                    clf.net.blocks.iter().map(|b| b.heads).collect::<Vec<_>>()
                )
            }
            Action::Deepen => {
                let at = clf.net.blocks.len();
                deepen(&mut clf.net, at, s);
                format!("deepened to {} blocks", clf.net.blocks.len())
            }
            Action::Prune => {
                let r = prune(clf, data, val, &self.prune, pool);
                return format!(
                    "pruning {} ({} units, {} heads; accuracy {:.4} -> {:.4})",
                    if r.accepted { "kept" } else { "reverted" },
                    r.units,
                    r.heads,
                    r.before,
                    r.after
                );
            }
        };
        clf.meta.log.push(line.clone());
        clf.opt = None;
        line
    }
}

/// a round of a growth cycle: what training did, the evaluation, the decision
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Round {
    pub train: TrainReport,
    pub eval: Evaluation,
    pub action: Action,
    pub note: String,
}

/// alternates training (`o.epochs` per round) and growth decisions for `rounds` rounds
#[allow(clippy::too_many_arguments)]
pub fn cycle(
    clf: &mut Classifier,
    data: &[Example],
    train_idx: &[usize],
    val: &[usize],
    o: &TrainOpts,
    grower: &mut Grower,
    rounds: usize,
    mut cb: impl FnMut(&Progress) -> Flow,
) -> Result<Vec<Round>, String> {
    let pool = o.pool();
    let mut out = Vec::new();
    for r in 0..rounds {
        let mut ro = o.clone();
        ro.seed = key(&[o.seed, r as u64]);
        let mut stop = false;
        let rep = train::fit(clf, data, train_idx, val, &ro, |p| {
            let f = cb(p);
            stop |= f == Flow::Stop;
            f
        })?;
        let eval = train::evaluate(clf, data, val, &pool);
        let action = grower.observe(eval.accuracy(), &clf.net);
        let note = grower.apply(clf, action, data, val, o.seed, &pool);
        out.push(Round {
            train: rep,
            eval,
            action,
            note,
        });
        if stop {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::tests::{synthetic, tiny_clf};
    use crate::classify::{Flow, TrainOpts};

    fn outputs(clf: &Classifier, texts: &[&str]) -> Vec<f32> {
        texts
            .iter()
            .flat_map(|t| clf.logits(t, &[]).concat())
            .chain(texts.iter().flat_map(|t| clf.represent(t)))
            .collect()
    }

    const TEXTS: [&str; 3] = [
        "apple granite",
        "the slate and the plum were in the cellar, all of them",
        "x",
    ];

    #[test]
    fn widening_and_deepening_preserve_outputs() {
        let data = synthetic(40, 11);
        let mut clf = tiny_clf(11);
        let o = TrainOpts {
            epochs: 2,
            threads: 1,
            ..TrainOpts::default()
        };
        train::train(&mut clf, &data, &o, |_| Flow::Go).unwrap();
        let before = outputs(&clf, &TEXTS);
        let mut w = clf.clone();
        for l in 0..w.net.blocks.len() {
            widen_ff(&mut w.net, l, 7, 3);
        }
        assert_eq!(w.net.blocks[0].ff, Config::tiny().d_ff + 7);
        let after = outputs(&w, &TEXTS);
        for (a, b) in before.iter().zip(&after) {
            assert!((a - b).abs() <= 1e-5 * (1.0 + a.abs()), "{a} {b}");
        }
        // heads and blocks: exactly the same bits
        let mut h = clf.clone();
        add_heads(&mut h.net, 0, 2, 4);
        deepen(&mut h.net, 1, 5);
        deepen(&mut h.net, 0, 6);
        assert_eq!((h.net.blocks.len(), h.net.blocks[1].heads), (4, 4));
        assert_eq!(outputs(&h, &TEXTS), before);
        // and they still train, and survive a save
        train::train(&mut h, &data, &o, |_| Flow::Go).unwrap();
        assert!(h.net.blocks[0].wd.iter().any(|&v| v != 0.0));
        assert_eq!(Classifier::from_bytes(&h.to_bytes()).unwrap().net, h.net);
    }

    use crate::classify::Config;

    #[test]
    fn pruning_is_kept_or_reverted() {
        let data = synthetic(60, 12);
        let mut clf = tiny_clf(12);
        let o = TrainOpts {
            epochs: 4,
            lr: 3e-3,
            threads: 2,
            val_fraction: 0.3,
            ..TrainOpts::default()
        };
        train::train(&mut clf, &data, &o, |_| Flow::Go).unwrap();
        let (_, val) = train::split(&data, 0.3, o.seed);
        let pool = Pool::new(2);
        // removing every unit but a few: whatever happens, the rule holds
        let po = PruneOpts {
            fraction: 0.5,
            min_ff: 4,
            ..PruneOpts::default()
        };
        let saved = clf.net.clone();
        let r = prune(&mut clf, &data, &val, &po, &pool);
        assert!(r.units > 0);
        if r.accepted {
            assert!(r.after >= r.before - po.tolerance);
            assert!(clf.net.blocks[0].ff < saved.blocks[0].ff);
        } else {
            assert_eq!(clf.net, saved);
        }
        // a pruning that must fail (no tolerance and a crippled net) is reverted
        let strict = PruneOpts {
            fraction: 1.0,
            min_ff: 1,
            tolerance: -1.0,
            ..PruneOpts::default()
        };
        let saved = clf.net.clone();
        assert!(!prune(&mut clf, &data, &val, &strict, &pool).accepted);
        assert_eq!(clf.net, saved);
        // pruning to a subset keeps the kept units' function
        let mut p = clf.clone();
        let all: Vec<usize> = (0..p.net.blocks[0].ff).collect();
        prune_ff(&mut p.net, 0, &all);
        assert_eq!(p.net, clf.net);
    }

    #[test]
    fn grower_policy() {
        let net = tiny_clf(1).net;
        let mut g = Grower::new(0.9);
        g.cooldown = 0;
        assert_eq!(g.observe(0.5, &net), Action::Widen);
        assert_eq!(g.observe(0.6, &net), Action::AddHead);
        assert_eq!(g.observe(0.7, &net), Action::Deepen);
        assert_eq!(g.observe(0.95, &net), Action::Keep);
        assert_eq!(g.observe(0.95, &net), Action::Keep);
        assert_eq!(g.observe(0.95, &net), Action::Prune);
        // a fall after new data grows again even above target
        assert_eq!(g.observe(0.91, &net), Action::Widen);
        let mut g = Grower::new(0.9);
        g.max_layers = 2;
        g.max_ff = 24;
        g.max_heads = 2;
        assert_eq!(g.observe(0.1, &net), Action::Keep);
    }

    #[test]
    fn a_growth_cycle_grows_under_target() {
        let data = synthetic(40, 13);
        let mut clf = tiny_clf(13);
        let (tr, va) = train::split(&data, 0.3, 0);
        let o = TrainOpts {
            epochs: 1,
            threads: 2,
            ..TrainOpts::default()
        };
        let mut g = Grower::new(1.01);
        g.cooldown = 0;
        let rounds = cycle(&mut clf, &data, &tr, &va, &o, &mut g, 3, |_| Flow::Go).unwrap();
        let acts: Vec<Action> = rounds.iter().map(|r| r.action).collect();
        assert_eq!(acts, vec![Action::Widen, Action::AddHead, Action::Deepen]);
        assert_eq!(clf.net.blocks.len(), 3);
        assert!(clf.net.blocks[0].ff > Config::tiny().d_ff && clf.net.blocks[0].heads == 3);
        assert_eq!(clf.meta.log.len(), 3);
    }
}
