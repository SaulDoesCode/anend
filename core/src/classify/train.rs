//! Training: labelled examples, AdamW with linear warmup and cosine decay, global-norm
//! clipping, dropout, label smoothing, class balancing, early stopping on a validation
//! split with the best weights kept, and feedback (a yes or no on one result) applied as
//! a few incremental steps.
//!
//! Determinism: an optimizer step's gradient is the sum of per-example gradients added
//! in example order (`((0 + g₀) + g₁) + …`). Threads compute examples side by side into
//! their own buffers, a wave of one example per thread at a time, and the buffers are
//! added in order, so the thread count changes the speed only. Dropout masks and window
//! crops are keyed by (seed, step, example), not by which thread runs what.

use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::device::{Device, Trainer};
use super::kernels::{self as k, softmax_rows};
use super::math::{self, Rng, key, mix};
use super::metrics::{self, Metrics};
use super::model::{BlockT, END, Gains, HeadKind, Mode, Net, Trace};
use super::pool::{self, Pool};
use super::text::{self, Window};
use super::{Classifier, FEEDBACK_KEPT};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Target {
    /// single label: this class
    Class(usize),
    /// single label: not this class (pushes its probability down, the rest up)
    Not(usize),
    /// single label: a distribution over the classes
    Soft(Vec<f32>),
    /// multi label: each label's truth, 0 to 1
    Labels(Vec<f32>),
    /// multi label: one label known to hold or not, the others unknown
    Label(usize, bool),
    /// regression: the values
    Values(Vec<f32>),
    /// generative head: predict each next byte (and the end) of the example's bytes,
    /// counting only the bytes from index `from` on (a prompt before it is context)
    Next { from: usize },
    /// generative head: make each next byte from `from` on less likely
    /// (unlikelihood, −ln(1 − p)), for samples judged bad
    Avoid { from: usize },
}

impl Target {
    /// the class an example is about, for balancing
    pub fn class(&self) -> Option<usize> {
        match self {
            Target::Class(c) | Target::Not(c) | Target::Label(c, _) => Some(*c),
            _ => None,
        }
    }

    /// whether the target is for the generative head
    pub fn generative(&self) -> bool {
        matches!(self, Target::Next { .. } | Target::Avoid { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Example {
    pub text: String,
    pub head: usize,
    pub target: Target,
    /// how much this example counts (feedback can weigh more than a list member)
    pub weight: f32,
    /// inputs from other classifiers; empty means zeros
    pub extra: Vec<f32>,
    /// the data as bytes when it isn't text (pictures, binary); used instead of `text`
    /// when not empty
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw: Vec<u8>,
    /// more targets on other heads of the same model, trained in the same pass
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<(usize, Target)>,
}

impl Example {
    /// a text of class `class` for head 0, weight 1
    pub fn class(text: &str, class: usize) -> Example {
        Example::new(text, 0, Target::Class(class))
    }

    pub fn new(text: &str, head: usize, target: Target) -> Example {
        Example {
            text: text.into(),
            head,
            target,
            weight: 1.0,
            extra: Vec::new(),
            raw: Vec::new(),
            also: Vec::new(),
        }
    }

    /// an example over bytes that need not be text
    pub fn bytes(raw: &[u8], head: usize, target: Target) -> Example {
        let mut e = Example::new("", head, target);
        e.raw = raw.to_vec();
        e
    }

    /// a text for the generative head to learn whole
    pub fn generate(text: &str) -> Example {
        Example::new(text, 0, Target::Next { from: 0 })
    }

    /// another head's target in the same pass
    pub fn and(mut self, head: usize, target: Target) -> Example {
        self.also.push((head, target));
        self
    }

    /// the data: `raw` if set, else the text's bytes
    pub fn data(&self) -> &[u8] {
        if self.raw.is_empty() {
            self.text.as_bytes()
        } else {
            &self.raw
        }
    }

    /// every (head, target) of the example, the main one first
    pub fn targets(&self) -> impl Iterator<Item = (usize, &Target)> {
        std::iter::once((self.head, &self.target)).chain(self.also.iter().map(|(h, t)| (*h, t)))
    }

    pub fn weighted(mut self, w: f32) -> Example {
        self.weight = w;
        self
    }

    pub fn with_extra(mut self, e: Vec<f32>) -> Example {
        self.extra = e;
        self
    }
}

/// the loss of one head's logits against a target, and d(loss)/d(logits)
///
/// Single label: log-softmax `(l − max) − ln Σ e^(l − max)`, loss `−dot(q, log p)` with
/// q the target smoothed as `ε/K + (1 − ε)·onehot`; gradient `p − q`. `Not(c)`:
/// `−ln(1 − p_c)`, gradient `p_c·(δ − p)/(1 − p_c)`. Multi label: mean over labels of
/// `softplus(z) − z·y`. Regression: mean squared error.
pub fn head_loss(kind: HeadKind, logits: &[f32], t: &Target, smoothing: f32) -> Result<(f32, Vec<f32>), String> {
    let kw = logits.len();
    let kf = kw as f32;
    let bad = || Err(format!("target {t:?} does not fit a {kind:?} head of {kw}"));
    if t.generative() {
        return bad();
    }
    match kind {
        HeadKind::Single => {
            let m = logits.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            let sh: Vec<f32> = logits.iter().map(|l| l - m).collect();
            let mut p = sh.clone();
            softmax_rows(&mut p, kw);
            let lse = math::ln(k::sum(&sh.iter().map(|&v| math::exp(v)).collect::<Vec<_>>()));
            let logp: Vec<f32> = sh.iter().map(|v| v - lse).collect();
            let q: Vec<f32> = match t {
                Target::Class(c) if *c < kw => (0..kw)
                    .map(|i| smoothing / kf + if i == *c { 1.0 - smoothing } else { 0.0 })
                    .collect(),
                Target::Soft(q) if q.len() == kw => q.clone(),
                Target::Not(c) if *c < kw => {
                    let pc = p[*c];
                    let r = (1.0 - pc).max(1e-6);
                    let g = (0..kw)
                        .map(|i| pc * (if i == *c { 1.0 } else { 0.0 } - p[i]) / r)
                        .collect();
                    return Ok((-math::ln(r), g));
                }
                _ => return bad(),
            };
            Ok((-k::dot(&q, &logp), p.iter().zip(&q).map(|(a, b)| a - b).collect()))
        }
        HeadKind::Multi => {
            let sm = |y: f32| y * (1.0 - smoothing) + smoothing / 2.0;
            match t {
                Target::Labels(y) if y.len() == kw => {
                    let ys: Vec<f32> = y.iter().map(|&v| sm(v)).collect();
                    let ls: Vec<f32> = logits
                        .iter()
                        .zip(&ys)
                        .map(|(&z, &y)| math::softplus(z) - z * y)
                        .collect();
                    let g = logits
                        .iter()
                        .zip(&ys)
                        .map(|(&z, &y)| (math::sigmoid(z) - y) / kf)
                        .collect();
                    Ok((k::sum(&ls) / kf, g))
                }
                Target::Class(c) if *c < kw => {
                    let y = (0..kw).map(|i| (i == *c) as u8 as f32).collect();
                    head_loss(kind, logits, &Target::Labels(y), smoothing)
                }
                Target::Label(c, yes) if *c < kw => {
                    let (z, y) = (logits[*c], sm(*yes as u8 as f32));
                    let mut g = vec![0.0; kw];
                    g[*c] = math::sigmoid(z) - y;
                    Ok((math::softplus(z) - z * y, g))
                }
                _ => bad(),
            }
        }
        HeadKind::Regress => match t {
            Target::Values(y) if y.len() == kw => {
                let d: Vec<f32> = logits.iter().zip(y).map(|(a, b)| a - b).collect();
                Ok((k::dot(&d, &d) / kf, d.iter().map(|v| 2.0 * v / kf).collect()))
            }
            _ => bad(),
        },
    }
}

/// the generative loss of one window: at each position t whose next token is a byte
/// (or SEP, read as END) at data index ≥ `from`, cross-entropy of the next token
/// (`Next`) or −ln(1 − p) (`Avoid`), averaged over those positions. `offset` is the
/// data index of the window's first byte. Returns (loss, d(loss)/d(logits) [n ×
/// GEN_OUT], positions counted).
pub fn gen_loss(net: &Net, logits: &[f32], tokens: &[u32], offset: usize, t: &Target) -> (f32, Vec<f32>, usize) {
    let n = tokens.len();
    let out = net.cfg.gen_out();
    let mut g = vec![0f32; n * out];
    let (from, avoid) = match t {
        Target::Next { from } => (*from, false),
        Target::Avoid { from } => (*from, true),
        _ => return (0.0, g, 0),
    };
    let mut rows = Vec::new();
    for p in 0..n.saturating_sub(1) {
        let next = tokens[p + 1];
        let class = match next {
            b if b < 256 => b as usize,
            // a vocabulary: every token is a class (the end token included)
            t if net.lexicon.is_some() => t as usize,
            text::SEP => END,
            _ => continue,
        };
        if offset + p >= from {
            if class >= out {
                continue;
            }
            rows.push((p, class));
        }
    }
    let count = rows.len();
    let mut loss = 0f32;
    for &(p, class) in &rows {
        let row = &logits[p * out..(p + 1) * out];
        let tg = if avoid {
            Target::Not(class)
        } else {
            Target::Class(class)
        };
        let (l, d) = head_loss(HeadKind::Single, row, &tg, 0.0).expect("fits");
        loss += l;
        let inv = 1.0 / count as f32;
        g[p * out..(p + 1) * out]
            .iter_mut()
            .zip(&d)
            .for_each(|(a, b)| *a = b * inv);
    }
    (if count > 0 { loss / count as f32 } else { 0.0 }, g, count)
}

/// whether a text falls in the validation share: a seeded hash of its bytes, so the
/// same text always lands on the same side
pub fn is_validation(text: &str, fraction: f32, seed: u64) -> bool {
    is_validation_bytes(text.as_bytes(), fraction, seed)
}

pub fn is_validation_bytes(data: &[u8], fraction: f32, seed: u64) -> bool {
    let h = mix(math::fnv(seed, data)) >> 40;
    (h as f32) * (1.0 / 16_777_216.0) < fraction
}

/// indices of training and validation examples
pub fn split(data: &[Example], fraction: f32, seed: u64) -> (Vec<usize>, Vec<usize>) {
    (0..data.len()).partition(|&i| !is_validation_bytes(data[i].data(), fraction, seed))
}

// ---------------------------------------------------------------- optimizer

/// AdamW's moments for every weight
#[derive(Clone, Debug)]
pub struct Adam {
    m: Net,
    v: Net,
    b1t: f32,
    b2t: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct AdamOpts {
    pub b1: f32,
    pub b2: f32,
    pub eps: f32,
    pub weight_decay: f32,
}

impl Adam {
    pub fn new(net: &Net) -> Adam {
        Adam {
            m: net.zeros_like(),
            v: net.zeros_like(),
            b1t: 1.0,
            b2t: 1.0,
        }
    }

    /// whether the moments still match the net's shapes (they don't after growth)
    pub fn fits(&self, net: &Net) -> bool {
        let a = self.m.params();
        let b = net.params();
        a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x.1.len() == y.1.len())
    }

    /// one step, per element in this order: g′ = g·scale; w −= (lr·wd)·w if the
    /// tensor decays; m = b1·m + (1−b1)·g′; v = b2·v + (1−b2)·g′²;
    /// w −= lr·((m/(1−b1ᵗ)) / (√(v/(1−b2ᵗ)) + eps)). Tensors with `train[i]` false
    /// stay as they are. Tensors update in parallel on the pool (each element is independent).
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        net: &mut Net,
        g: &Net,
        lr: f32,
        o: &AdamOpts,
        scale: f32,
        train: &[bool],
        pool: Option<&Pool>,
    ) {
        self.b1t *= o.b1;
        self.b2t *= o.b2;
        let (c1, c2) = (1.0 - self.b1t, 1.0 - self.b2t);
        let decay = lr * o.weight_decay;
        type Slot<'a> = Mutex<(bool, &'a mut Vec<f32>, &'a Vec<f32>, &'a mut Vec<f32>, &'a mut Vec<f32>)>;
        let slots: Vec<Slot> = net
            .params_mut()
            .into_iter()
            .zip(g.params())
            .zip(self.m.params_mut().into_iter().zip(self.v.params_mut()))
            .map(|(((kind, w), (_, gt)), ((_, m), (_, v)))| Mutex::new((kind.decays(), w, gt, m, v)))
            .collect();
        let one = |i: usize| {
            if !train.get(i).copied().unwrap_or(true) {
                return;
            }
            let mut s = slots[i].lock().unwrap();
            let (dec, w, gt, m, v) = &mut *s;
            for j in 0..w.len() {
                let gj = gt[j] * scale;
                if *dec {
                    w[j] -= decay * w[j];
                }
                m[j] = o.b1 * m[j] + (1.0 - o.b1) * gj;
                v[j] = o.b2 * v[j] + (1.0 - o.b2) * (gj * gj);
                w[j] -= lr * ((m[j] / c1) / ((v[j] / c2).sqrt() + o.eps));
            }
        };
        match pool {
            Some(p) => p.run(slots.len(), &one),
            None => (0..slots.len()).for_each(one),
        }
    }
}

/// the clipping factor: min(1, clip/‖g‖) with ‖g‖² summed tensor by tensor in order
pub fn clip_scale(g: &Net, clip: f32, train: &[bool]) -> (f32, f32) {
    let mut s = 0f32;
    for (i, (_, t)) in g.params().into_iter().enumerate() {
        if train.get(i).copied().unwrap_or(true) {
            s += k::dot(t, t);
        }
    }
    let norm = s.sqrt();
    let scale = if clip > 0.0 && norm > clip { clip / norm } else { 1.0 };
    (scale, norm)
}

/// the learning rate at `step` of `total`: linear warmup over `warmup` steps, then a
/// cosine from `lr` down to `lr·min_ratio` (computed in f64 with `math::cos64`)
pub fn lr_at(step: usize, total: usize, warmup: usize, lr: f32, min_ratio: f32) -> f32 {
    if step < warmup {
        return lr * (step + 1) as f32 / warmup as f32;
    }
    let span = total.saturating_sub(warmup).max(1) as f64;
    let p = ((step - warmup) as f64 / span).min(1.0);
    let lo = lr as f64 * min_ratio as f64;
    (lo + (lr as f64 - lo) * 0.5 * (1.0 + math::cos64(std::f64::consts::PI * p))) as f32
}

/// which tensors train: everything, or the heads only when the trunk is frozen
/// (params end with the heads' pairs, then num, then the generative pair)
pub fn trainable(net: &Net, freeze_trunk: bool) -> Vec<bool> {
    let n = net.params().len();
    let heads = 2 * net.heads.len();
    (0..n)
        .map(|i| !freeze_trunk || (i >= n - 3 - heads && i < n - 3) || i >= n - 2)
        .collect()
}

// ---------------------------------------------------------------- gradients

/// one window of a training text: the whole text if it fits, else a crop placed by the key
pub fn crop(net: &Net, text: &str, key: u64) -> Window {
    crop_bytes(net, text.as_bytes(), key).0
}

/// `crop` over bytes, with the crop's start; a causal model's crop ends in SEP only
/// where the data ends
pub fn crop_bytes(net: &Net, b: &[u8], key: u64) -> (Window, usize) {
    let room = net.cfg.room();
    if net.lexicon.is_some() {
        return crop_seq(net, &net.sequence(b, 0).0, key);
    }
    let causal = net.cfg.causal;
    if b.len() <= room {
        return (text::window_with(b, causal, true), 0);
    }
    let start = (mix(key) % (b.len() - room + 1) as u64) as usize;
    let end = start + room == b.len();
    (
        text::window_with(&b[start..start + room], causal, !causal || end),
        start,
    )
}

/// every window of a text forward, and the mean of their logits (in window order)
/// `crop` over a token sequence (`Net::sequence`), the same windows as `crop_bytes`
pub fn crop_seq(net: &Net, ts: &[u32], key: u64) -> (Window, usize) {
    let room = net.cfg.room();
    let causal = net.cfg.causal;
    if ts.len() <= room {
        return (net.window_tokens(ts, true), 0);
    }
    let start = (mix(key) % (ts.len() - room + 1) as u64) as usize;
    let end = start + room == ts.len();
    (net.window_tokens(&ts[start..start + room], !causal || end), start)
}

/// an example's data as the token sequence the net reads, and its targets with each
/// generative target's `from` (a byte index) as a token index
pub fn sequence_of(net: &Net, ex: &Example) -> (Vec<u32>, Vec<(usize, Target)>) {
    let first = from_of(ex);
    let (ts, at) = net.sequence(ex.data(), first.unwrap_or(0));
    let moved = |f: usize| if Some(f) == first { at } else { f };
    let targets = ex
        .targets()
        .map(|(h, t)| {
            let t = match t {
                Target::Next { from } => Target::Next { from: moved(*from) },
                Target::Avoid { from } => Target::Avoid { from: moved(*from) },
                t => t.clone(),
            };
            (h, t)
        })
        .collect();
    (ts, targets)
}

fn from_of(ex: &Example) -> Option<usize> {
    ex.targets().find_map(|(_, t)| match t {
        Target::Next { from } | Target::Avoid { from } => Some(*from),
        _ => None,
    })
}

pub fn forward_text(net: &Net, text: &[u8], extra: &[f32], gains: &Gains) -> (Vec<Trace>, Vec<Vec<f32>>) {
    forward_windows(net, &net.windows_of(text).0, extra, gains)
}

/// windows forward, and the mean of their logits (in window order)
pub fn forward_windows(net: &Net, ws: &[Window], extra: &[f32], gains: &Gains) -> (Vec<Trace>, Vec<Vec<f32>>) {
    let trs: Vec<Trace> = ws
        .iter()
        .map(|w| net.forward(w, extra, gains, Mode::EVAL, None))
        .collect();
    let nh = net.heads.len();
    let mut avg: Vec<Vec<f32>> = net.heads.iter().map(|h| vec![0.0; h.width()]).collect();
    for tr in &trs {
        for (h, a) in avg.iter_mut().enumerate().take(nh) {
            a.iter_mut().zip(net.logits(tr, h)).for_each(|(x, y)| *x += y);
        }
    }
    let nw = trs.len() as f32;
    avg.iter_mut().flatten().for_each(|v| *v /= nw);
    (trs, avg)
}

/// backward for `forward_text`: each window gets d(loss)/d(mean logits)/windows; returns
/// d(loss)/d(extra) summed over the windows in order
pub fn backward_text(net: &Net, trs: &[Trace], dlogits: &[Vec<f32>], tp: &[BlockT], g: &mut Net) -> Vec<f32> {
    backward_text_gen(net, trs, dlogits, &[], tp, g)
}

/// `backward_text` with each window's d(loss)/d(generative logits) (empty for none)
pub fn backward_text_gen(
    net: &Net,
    trs: &[Trace],
    dlogits: &[Vec<f32>],
    dgen: &[Vec<f32>],
    tp: &[BlockT],
    g: &mut Net,
) -> Vec<f32> {
    let nw = trs.len() as f32;
    let dl: Vec<Vec<f32>> = dlogits.iter().map(|d| d.iter().map(|v| v / nw).collect()).collect();
    let mut dextra = vec![0f32; net.cfg.extra];
    for (i, tr) in trs.iter().enumerate() {
        let dg = dgen.get(i).map_or(&[][..], |v| &v[..]);
        let de = net.backward_gen(tr, &dl, dg, tp, g);
        dextra.iter_mut().zip(de).for_each(|(a, b)| *a += b);
    }
    dextra
}

/// the loss weight of the head a target is for
fn head_weight(net: &Net, head: usize, t: &Target) -> f32 {
    if t.generative() {
        net.generator.as_ref().map_or(1.0, |g| g.weight)
    } else {
        net.heads[head].spec.weight
    }
}

/// how an example's gradient is taken
#[derive(Clone, Copy)]
enum How {
    /// one keyed crop, dropout as configured (training)
    Crop(u64),
    /// every window, loss on the averaged logits, no dropout (feedback)
    Whole,
}

struct Item {
    ex: usize,
    coef: f32,
    how: How,
}

fn zeros_for(net: &Net, e: &[f32]) -> Vec<f32> {
    if e.is_empty() {
        vec![0.0; net.cfg.extra]
    } else {
        e.to_vec()
    }
}

/// adds `into += scale·d`, creating it when empty
fn add_scaled(into: &mut Vec<f32>, d: &[f32], scale: f32) {
    if into.is_empty() {
        *into = vec![0.0; d.len()];
    }
    into.iter_mut().zip(d).for_each(|(a, b)| *a += b * scale);
}

/// adds coef·∇loss of one example (every target, each times its head's weight) into g;
/// returns (loss, tokens)
fn example_grad(net: &Net, tp: &[BlockT], ex: &Example, it: &Item, smoothing: f32, g: &mut Net) -> (f32, usize) {
    let extra = zeros_for(net, &ex.extra);
    let mut dls = vec![Vec::new(); net.heads.len()];
    let mut loss = 0f32;
    match it.how {
        How::Crop(kk) => {
            let (ts, targets) = sequence_of(net, ex);
            let (w, start) = crop_seq(net, &ts, kk);
            let mode = Mode {
                dropout: net.cfg.dropout,
                key: kk,
            };
            let tr = net.forward(&w, &extra, &net.cfg.gains, mode, None);
            let mut dgen = Vec::new();
            for (h, t) in targets.iter().map(|(h, t)| (*h, t)) {
                let hw = head_weight(net, h, t);
                if t.generative() {
                    let (l, d, _) = gen_loss(net, &net.gen_logits(&tr), &w.tokens, start, t);
                    loss += hw * l;
                    add_scaled(&mut dgen, &d, hw * it.coef);
                } else {
                    let kind = net.heads[h].spec.kind;
                    let (l, d) = head_loss(kind, &net.logits(&tr, h), t, smoothing).expect("checked target");
                    loss += hw * l;
                    add_scaled(&mut dls[h], &d, hw * it.coef);
                }
            }
            net.backward_gen(&tr, &dls, &dgen, tp, g);
            (loss, w.len())
        }
        How::Whole => {
            let (ts, targets) = sequence_of(net, ex);
            let (ws, starts) = net.windows_over(&ts);
            let (trs, avg) = forward_windows(net, &ws, &extra, &net.cfg.gains);
            let nw = trs.len() as f32;
            let mut dgen: Vec<Vec<f32>> = Vec::new();
            for (h, t) in targets.iter().map(|(h, t)| (*h, t)) {
                let hw = head_weight(net, h, t);
                if t.generative() {
                    dgen.resize(trs.len(), Vec::new());
                    for (i, tr) in trs.iter().enumerate() {
                        let (l, d, _) = gen_loss(net, &net.gen_logits(tr), tr.tokens(), starts[i], t);
                        loss += hw * l / nw;
                        add_scaled(&mut dgen[i], &d, hw * it.coef / nw);
                    }
                } else {
                    let kind = net.heads[h].spec.kind;
                    let (l, d) = head_loss(kind, &avg[h], t, smoothing).expect("checked target");
                    loss += hw * l;
                    add_scaled(&mut dls[h], &d, hw * it.coef);
                }
            }
            let tokens = trs.iter().map(|t| t.n).sum();
            backward_text_gen(net, &trs, &dls, &dgen, tp, g);
            (loss, tokens)
        }
    }
}

/// the summed gradient of a batch into `total`; returns (Σ coef·loss, tokens)
fn batch_grad(
    net: &Net,
    data: &[Example],
    items: &[Item],
    smoothing: f32,
    pool: &Pool,
    bufs: &[Mutex<Net>],
    total: &mut Net,
) -> (f64, usize) {
    total.fill(0.0);
    let tp = net.transposes();
    let (mut loss, mut tokens) = (0f64, 0usize);
    for wave in items.chunks(bufs.len()) {
        let res: Vec<Mutex<(f32, usize)>> = wave.iter().map(|_| Mutex::new((0.0, 0))).collect();
        pool.run(wave.len(), &|j| {
            let mut b = bufs[j].lock().unwrap();
            b.fill(0.0);
            *res[j].lock().unwrap() = example_grad(net, &tp, &data[wave[j].ex], &wave[j], smoothing, &mut b);
        });
        // total += each buffer in example order, tensors split across the pool
        {
            let guards: Vec<_> = bufs[..wave.len()].iter().map(|b| b.lock().unwrap()).collect();
            let srcs: Vec<Vec<&Vec<f32>>> = guards
                .iter()
                .map(|g| g.params().into_iter().map(|(_, t)| t).collect())
                .collect();
            let dst: Vec<Mutex<&mut Vec<f32>>> = total.params_mut().into_iter().map(|(_, t)| Mutex::new(t)).collect();
            pool.run(dst.len(), &|ti| {
                let mut d = dst[ti].lock().unwrap();
                for s in &srcs {
                    d.iter_mut().zip(s[ti].iter()).for_each(|(a, b)| *a += b);
                }
            });
        }
        for (j, it) in wave.iter().enumerate() {
            let (l, t) = *res[j].lock().unwrap();
            loss += (it.coef * l) as f64;
            tokens += t;
        }
    }
    (loss, tokens)
}

fn buffers(net: &Net, n: usize) -> Vec<Mutex<Net>> {
    (0..n).map(|_| Mutex::new(net.zeros_like())).collect()
}

// ---------------------------------------------------------------- evaluation

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Evaluation {
    /// weighted mean loss, no label smoothing
    pub loss: f64,
    /// per head, when the data had examples for it
    pub heads: Vec<Option<Metrics>>,
    /// the generative head's mean cross-entropy in bits per predicted byte
    #[serde(default)]
    pub bits_per_byte: Option<f64>,
}

impl Evaluation {
    /// the accuracy of the first head with metrics
    pub fn accuracy(&self) -> f64 {
        self.heads.iter().flatten().next().map_or(0.0, |m| m.accuracy)
    }
}

/// unsteered predictions on the examples `idx` of `data`, with loss and metrics per head
pub fn evaluate(clf: &Classifier, data: &[Example], idx: &[usize], pool: &Pool) -> Evaluation {
    let net = &clf.net;
    type Slot = Mutex<Option<(f32, Vec<f32>)>>;
    let slots: Vec<Slot> = idx.iter().map(|_| Mutex::new(None)).collect();
    let gen_nats: Vec<Mutex<(f64, usize)>> = idx.iter().map(|_| Mutex::new((0.0, 0))).collect();
    pool.run(idx.len(), &|i| {
        let ex = &data[idx[i]];
        let extra = zeros_for(net, &ex.extra);
        let mut total = 0f32;
        let mut main = Vec::new();
        let (ts, targets) = sequence_of(net, ex);
        let (ws, starts) = net.windows_over(&ts);
        let (trs, avg) = forward_windows(net, &ws, &extra, &net.cfg.gains);
        for (j, (h, t)) in targets.iter().map(|(h, t)| (*h, t)).enumerate() {
            let hw = head_weight(net, h, t);
            if t.generative() {
                let (mut nats, mut count) = (0f64, 0usize);
                for (w, tr) in trs.iter().enumerate() {
                    let (l, _, c) = gen_loss(net, &net.gen_logits(tr), tr.tokens(), starts[w], t);
                    nats += l as f64 * c as f64;
                    count += c;
                }
                if count > 0 {
                    total += hw * (nats / count as f64) as f32;
                }
                if matches!(t, Target::Next { .. }) {
                    let mut g = gen_nats[i].lock().unwrap();
                    g.0 += nats;
                    g.1 += count;
                }
            } else {
                let (l, _) = head_loss(net.heads[h].spec.kind, &avg[h], t, 0.0).unwrap_or((0.0, vec![]));
                total += hw * l;
            }
            if j == 0 && !t.generative() {
                main = avg[h].clone();
            }
        }
        *slots[i].lock().unwrap() = Some((total, main));
    });
    let (mut loss, mut wsum) = (0f64, 0f64);
    let nh = net.heads.len();
    let mut probs: Vec<Vec<Vec<f32>>> = vec![Vec::new(); nh];
    let mut truth_c: Vec<Vec<usize>> = vec![Vec::new(); nh];
    let mut truth_v: Vec<Vec<Vec<f32>>> = vec![Vec::new(); nh];
    for (i, s) in slots.into_iter().enumerate() {
        let (l, logits) = s.into_inner().unwrap().unwrap();
        let ex = &data[idx[i]];
        loss += ex.weight as f64 * l as f64;
        wsum += ex.weight as f64;
        let h = ex.head;
        if ex.target.generative() {
            continue;
        }
        match (net.heads[h].spec.kind, &ex.target) {
            (HeadKind::Single, Target::Class(c)) => {
                let mut p = logits;
                softmax_rows(&mut p, net.heads[h].width());
                probs[h].push(p);
                truth_c[h].push(*c);
            }
            (HeadKind::Multi, Target::Labels(y)) => {
                probs[h].push(logits.iter().map(|&z| math::sigmoid(z)).collect());
                truth_v[h].push(y.clone());
            }
            (HeadKind::Regress, Target::Values(y)) => {
                probs[h].push(logits);
                truth_v[h].push(y.clone());
            }
            _ => {}
        }
    }
    let heads = (0..nh)
        .map(|h| {
            (!probs[h].is_empty()).then(|| match net.heads[h].spec.kind {
                HeadKind::Single => metrics::single(&probs[h], &truth_c[h]),
                HeadKind::Multi => metrics::multi(&probs[h], &truth_v[h], 0.5),
                HeadKind::Regress => metrics::regress(&probs[h], &truth_v[h]),
            })
        })
        .collect();
    let (nats, count) = gen_nats
        .into_iter()
        .map(|m| m.into_inner().unwrap())
        .fold((0f64, 0usize), |a, b| (a.0 + b.0, a.1 + b.1));
    Evaluation {
        loss: if wsum > 0.0 { loss / wsum } else { 0.0 },
        heads,
        bits_per_byte: (count > 0).then(|| nats / count as f64 / std::f64::consts::LN_2),
    }
}

// ---------------------------------------------------------------- training

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Balance {
    None,
    /// each class's examples weigh n/(K·n_class)
    Weights,
    /// epochs draw examples with probability ∝ 1/n_class
    Sampling,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrainOpts {
    pub epochs: usize,
    pub batch: usize,
    pub lr: f32,
    /// the cosine ends at lr·min_lr
    pub min_lr: f32,
    /// share of all steps spent warming up
    pub warmup: f32,
    pub weight_decay: f32,
    pub b1: f32,
    pub b2: f32,
    pub eps: f32,
    /// global gradient norm limit (0 for none)
    pub clip: f32,
    pub label_smoothing: f32,
    pub balance: Balance,
    /// share of examples held out for early stopping (by text hash)
    pub val_fraction: f32,
    /// evaluations without a better validation loss before stopping
    pub patience: usize,
    /// steps between evaluations (0: once per epoch)
    pub eval_every: usize,
    pub seed: u64,
    /// 0 for every core
    pub threads: usize,
    /// train the heads only
    pub freeze_trunk: bool,
    /// stop after this many steps (0: no limit)
    pub max_steps: usize,
    /// keep an exponential moving average of the weights with this decay (0: off);
    /// evaluation, early stopping and the result use the average
    pub ema: f32,
    /// where the forward and backward passes run (the CPU unless asked; see `device`)
    #[serde(default)]
    pub device: Device,
}

impl Default for TrainOpts {
    fn default() -> TrainOpts {
        TrainOpts {
            epochs: 20,
            batch: 16,
            lr: 1e-3,
            min_lr: 0.1,
            warmup: 0.05,
            weight_decay: 0.05,
            b1: 0.9,
            b2: 0.98,
            eps: 1e-8,
            clip: 1.0,
            label_smoothing: 0.05,
            balance: Balance::Weights,
            val_fraction: 0.1,
            patience: 4,
            eval_every: 0,
            seed: 0,
            threads: 0,
            freeze_trunk: false,
            max_steps: 0,
            ema: 0.0,
            device: Device::Cpu,
        }
    }
}

impl TrainOpts {
    fn adam(&self) -> AdamOpts {
        AdamOpts {
            b1: self.b1,
            b2: self.b2,
            eps: self.eps,
            weight_decay: self.weight_decay,
        }
    }

    pub fn pool(&self) -> Pool {
        Pool::new(if self.threads == 0 {
            pool::available()
        } else {
            self.threads
        })
    }
}

/// what the progress callback sees
#[derive(Clone, Debug)]
pub struct Progress<'a> {
    pub epoch: usize,
    pub step: usize,
    pub steps: usize,
    pub lr: f32,
    /// this step's weighted training loss
    pub loss: f64,
    pub grad_norm: f32,
    pub tokens_per_sec: f64,
    /// set on evaluation steps
    pub eval: Option<&'a Evaluation>,
}

/// the callback's answer
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Go,
    Stop,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EvalLog {
    pub epoch: usize,
    pub step: usize,
    pub train_loss: f64,
    pub eval: Evaluation,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrainReport {
    pub steps: usize,
    pub epochs: usize,
    pub train_examples: usize,
    pub val_examples: usize,
    /// the step whose weights were kept
    pub best_step: usize,
    pub best_loss: Option<f64>,
    pub stopped_early: bool,
    pub cancelled: bool,
    pub history: Vec<EvalLog>,
    /// the kept weights on the validation examples
    pub eval: Option<Evaluation>,
    pub seconds: f64,
    pub tokens: u64,
    pub tokens_per_sec: f64,
}

/// checks every example against the classifier's heads
pub fn check(clf: &Classifier, data: &[Example]) -> Result<(), String> {
    for (i, ex) in data.iter().enumerate() {
        for (head, t) in ex.targets() {
            if t.generative() {
                if clf.net.generator.is_none() {
                    return Err(format!("example {i}: the model has no generative head"));
                }
                continue;
            }
            let h = clf.net.heads.get(head).ok_or(format!("example {i}: no head {head}"))?;
            head_loss(h.spec.kind, &vec![0.0; h.width()], t, 0.0).map_err(|e| format!("example {i}: {e}"))?;
        }
        if !ex.extra.is_empty() && ex.extra.len() != clf.net.cfg.extra {
            return Err(format!(
                "example {i}: {} extra inputs, the classifier takes {}",
                ex.extra.len(),
                clf.net.cfg.extra
            ));
        }
        if !(ex.weight >= 0.0 && ex.weight.is_finite()) {
            return Err(format!("example {i}: weight must be a finite non-negative number"));
        }
    }
    Ok(())
}

/// trains on `data` with a hash split for validation
pub fn train(
    clf: &mut Classifier,
    data: &[Example],
    o: &TrainOpts,
    cb: impl FnMut(&Progress) -> Flow,
) -> Result<TrainReport, String> {
    let (tr, va) = split(data, o.val_fraction, o.seed);
    fit(clf, data, &tr, &va, o, cb)
}

/// trains on `data[train]`, early stopping on `data[val]` (may be empty)
pub fn fit(
    clf: &mut Classifier,
    data: &[Example],
    train: &[usize],
    val: &[usize],
    o: &TrainOpts,
    mut cb: impl FnMut(&Progress) -> Flow,
) -> Result<TrainReport, String> {
    check(clf, data)?;
    if train.is_empty() {
        return Err("no training examples".into());
    }
    let started = Instant::now();
    let pool = o.pool();
    // class counts and balancing weights, per head
    let mut counts: Vec<Vec<f32>> = clf.net.heads.iter().map(|h| vec![0.0; h.width()]).collect();
    for &i in train {
        if let (Some(c), false) = (data[i].target.class(), data[i].target.generative()) {
            counts[data[i].head][c] += 1.0;
        }
    }
    let class_w: Vec<Vec<f32>> = counts
        .iter()
        .map(|cs| {
            let n: f32 = cs.iter().sum();
            let present = cs.iter().filter(|&&c| c > 0.0).count() as f32;
            cs.iter()
                .map(|&c| if c > 0.0 { n / (present * c) } else { 0.0 })
                .collect()
        })
        .collect();
    clf.meta.class_counts = counts.clone();
    let weight_of = |i: usize| -> f32 {
        let ex = &data[i];
        let cw = match (o.balance, ex.target.class()) {
            (Balance::Weights, Some(c)) => class_w[ex.head][c],
            _ => 1.0,
        };
        ex.weight * cw
    };
    let batch = o.batch.max(1);
    let per_epoch = train.len().div_ceil(batch);
    let mut total = per_epoch * o.epochs.max(1);
    if o.max_steps > 0 {
        total = total.min(o.max_steps);
    }
    let warmup = ((total as f32 * o.warmup) as usize).max(1);
    let train_mask = trainable(&clf.net, o.freeze_trunk);
    let mut adam = Adam::new(&clf.net);
    let bufs = buffers(&clf.net, pool.threads());
    let mut dev = Trainer::open(o.device, &clf.net)?;
    let mut grad = clf.net.zeros_like();
    let mut report = TrainReport {
        train_examples: train.len(),
        val_examples: val.len(),
        ..TrainReport::default()
    };
    let mut best: Option<(f64, Net, usize)> = None;
    let mut bad_evals = 0;
    let (mut step, mut tokens, mut since_loss, mut since_n) = (0usize, 0u64, 0f64, 0usize);
    let base_steps = clf.meta.steps;
    // the average: a = d·a + (1 − d)·w per element after every step
    let mut ema = (o.ema > 0.0).then(|| clf.net.clone());
    'epochs: for epoch in 0..o.epochs.max(1) {
        let mut r = Rng::new(key(&[o.seed, 0xe90c, epoch as u64]));
        let order: Vec<usize> = if o.balance == Balance::Sampling {
            let w: Vec<f64> = train
                .iter()
                .map(|&i| match data[i].target.class() {
                    Some(c) => 1.0 / counts[data[i].head][c].max(1.0) as f64,
                    None => 1.0,
                })
                .collect();
            let mut cum = Vec::with_capacity(w.len());
            let mut acc = 0f64;
            for x in &w {
                acc += x;
                cum.push(acc);
            }
            (0..train.len())
                .map(|_| {
                    let u = r.unit() as f64 * acc;
                    train[cum.partition_point(|&c| c <= u).min(train.len() - 1)]
                })
                .collect()
        } else {
            let mut v = train.to_vec();
            r.shuffle(&mut v);
            v
        };
        for chunk in order.chunks(batch) {
            if step >= total {
                break 'epochs;
            }
            let wsum: f32 = chunk.iter().map(|&i| weight_of(i)).sum();
            if wsum <= 0.0 {
                step += 1;
                continue;
            }
            let items: Vec<Item> = chunk
                .iter()
                .map(|&i| Item {
                    ex: i,
                    coef: weight_of(i) / wsum,
                    how: How::Crop(key(&[o.seed, base_steps + step as u64, i as u64])),
                })
                .collect();
            let (loss, tk) = match dev.as_mut() {
                Some(dv) => {
                    let crops: Vec<(usize, f32, u64)> = items
                        .iter()
                        .map(|it| match it.how {
                            How::Crop(kk) => (it.ex, it.coef, kk),
                            How::Whole => unreachable!("training crops"),
                        })
                        .collect();
                    dv.batch(&clf.net, data, &crops, o.label_smoothing, &mut grad)
                }
                None => batch_grad(&clf.net, data, &items, o.label_smoothing, &pool, &bufs, &mut grad),
            };
            tokens += tk as u64;
            let (scale, norm) = clip_scale(&grad, o.clip, &train_mask);
            let lr = lr_at(step, total, warmup, o.lr, o.min_lr);
            adam.step(&mut clf.net, &grad, lr, &o.adam(), scale, &train_mask, Some(&pool));
            if let Some(a) = ema.as_mut() {
                for ((_, av), (_, w)) in a.params_mut().into_iter().zip(clf.net.params()) {
                    av.iter_mut()
                        .zip(w)
                        .for_each(|(x, y)| *x = o.ema * *x + (1.0 - o.ema) * y);
                }
            }
            step += 1;
            since_loss += loss;
            since_n += 1;
            let tps = tokens as f64 / started.elapsed().as_secs_f64().max(1e-9);
            let mut flow = cb(&Progress {
                epoch,
                step,
                steps: total,
                lr,
                loss,
                grad_norm: norm,
                tokens_per_sec: tps,
                eval: None,
            });
            let end_of_epoch = chunk.as_ptr_range().end == order.as_ptr_range().end;
            let due = if o.eval_every > 0 {
                step.is_multiple_of(o.eval_every)
            } else {
                end_of_epoch
            };
            if (due || step == total) && !val.is_empty() && flow == Flow::Go {
                if let Some(a) = ema.as_mut() {
                    std::mem::swap(&mut clf.net, a);
                }
                let ev = evaluate(clf, data, val, &pool);
                let current = clf.net.clone();
                if let Some(a) = ema.as_mut() {
                    std::mem::swap(&mut clf.net, a);
                }
                report.history.push(EvalLog {
                    epoch,
                    step,
                    train_loss: since_loss / since_n.max(1) as f64,
                    eval: ev.clone(),
                });
                since_loss = 0.0;
                since_n = 0;
                flow = cb(&Progress {
                    epoch,
                    step,
                    steps: total,
                    lr,
                    loss,
                    grad_norm: norm,
                    tokens_per_sec: tps,
                    eval: Some(&ev),
                });
                if best.as_ref().is_none_or(|b| ev.loss < b.0) {
                    best = Some((ev.loss, current, step));
                    bad_evals = 0;
                } else {
                    bad_evals += 1;
                    if o.patience > 0 && bad_evals >= o.patience {
                        report.stopped_early = true;
                        report.epochs = epoch + 1;
                        break 'epochs;
                    }
                }
            }
            if flow == Flow::Stop {
                report.cancelled = true;
                report.epochs = epoch + 1;
                break 'epochs;
            }
        }
        report.epochs = epoch + 1;
    }
    report.steps = step;
    report.best_step = step;
    if let Some(a) = ema {
        clf.net = a;
    }
    if let Some((l, net, s)) = best {
        report.best_loss = Some(l);
        report.best_step = s;
        clf.net = net;
    }
    if !val.is_empty() {
        report.eval = Some(evaluate(clf, data, val, &pool));
    }
    clf.meta.steps += step as u64;
    clf.opt = None;
    report.seconds = started.elapsed().as_secs_f64();
    report.tokens = tokens;
    report.tokens_per_sec = tokens as f64 / report.seconds.max(1e-9);
    Ok(report)
}

// ---------------------------------------------------------------- feedback

/// a player's yes or no on one result of one classifier
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Feedback {
    pub text: String,
    pub head: usize,
    pub class: usize,
    /// yes: the text is of this class (or has this label); no: it isn't
    pub yes: bool,
    pub weight: f32,
    pub extra: Vec<f32>,
}

impl Feedback {
    pub fn new(text: &str, class: usize, yes: bool) -> Feedback {
        Feedback {
            text: text.into(),
            head: 0,
            class,
            yes,
            weight: 1.0,
            extra: Vec::new(),
        }
    }

    /// the example it amounts to for a head of this kind
    pub fn example(&self, kind: HeadKind) -> Result<Example, String> {
        let target = match (kind, self.yes) {
            (HeadKind::Single, true) => Target::Class(self.class),
            (HeadKind::Single, false) => Target::Not(self.class),
            (HeadKind::Multi, yes) => Target::Label(self.class, yes),
            (HeadKind::Regress, _) => return Err("yes/no feedback needs a class or label head".into()),
        };
        Ok(Example {
            text: self.text.clone(),
            head: self.head,
            target,
            weight: self.weight,
            extra: self.extra.clone(),
            raw: Vec::new(),
            also: Vec::new(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackOpts {
    pub steps: usize,
    pub lr: f32,
    /// replayed examples per step, drawn from earlier feedback and the given data
    pub replay: usize,
    /// change the heads only
    pub freeze_trunk: bool,
    pub clip: f32,
    pub seed: u64,
    pub threads: usize,
}

impl Default for FeedbackOpts {
    fn default() -> FeedbackOpts {
        FeedbackOpts {
            steps: 4,
            lr: 3e-4,
            replay: 8,
            freeze_trunk: false,
            clip: 1.0,
            seed: 0,
            threads: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedbackReport {
    /// the probability of each fed-back class before and after (unsteered)
    pub before: Vec<f32>,
    pub after: Vec<f32>,
    pub losses: Vec<f64>,
}

/// the unsteered probability of a feedback's class
pub fn feedback_prob(clf: &Classifier, f: &Feedback) -> f32 {
    let extra = zeros_for(&clf.net, &f.extra);
    let l = &clf.run(&f.text, &extra, &clf.net.cfg.gains, None).logits[f.head];
    match clf.net.heads[f.head].spec.kind {
        HeadKind::Single => {
            let mut p = l.clone();
            softmax_rows(&mut p, l.len());
            p[f.class]
        }
        _ => math::sigmoid(l[f.class]),
    }
}

impl Classifier {
    /// applies yes/no feedback on this classifier's own weights: the feedback becomes
    /// weighted examples (loss on the window-averaged logits the player saw), mixed with
    /// replayed earlier feedback and `replay` data, for `steps` AdamW steps at a flat
    /// rate. The feedback is kept in `meta.feedback` for later replay.
    pub fn feedback(
        &mut self,
        fb: &[Feedback],
        replay: &[Example],
        o: &FeedbackOpts,
    ) -> Result<FeedbackReport, String> {
        let mut data: Vec<Example> = Vec::new();
        for f in fb {
            let kind = self.net.heads.get(f.head).ok_or("no such head")?.spec.kind;
            data.push(f.example(kind)?);
        }
        let fresh = data.len();
        // newer feedback on the same text, head and class supersedes the old
        let same =
            |a: &Example, b: &Example| a.text == b.text && a.head == b.head && a.target.class() == b.target.class();
        self.meta
            .feedback
            .retain(|old| !data[..fresh].iter().any(|n| same(n, old)));
        data.extend(self.meta.feedback.iter().cloned());
        data.extend(replay.iter().cloned());
        check(self, &data)?;
        let pool = Pool::new(if o.threads == 0 { pool::available() } else { o.threads });
        let mut rep = FeedbackReport {
            before: fb.iter().map(|f| feedback_prob(self, f)).collect(),
            ..FeedbackReport::default()
        };
        if !self.opt.as_ref().is_some_and(|a| a.fits(&self.net)) {
            self.opt = Some(Adam::new(&self.net));
        }
        let mask = trainable(&self.net, o.freeze_trunk);
        let bufs = buffers(&self.net, pool.threads());
        let mut grad = self.net.zeros_like();
        let adam_o = AdamOpts {
            b1: 0.9,
            b2: 0.98,
            eps: 1e-8,
            weight_decay: 0.0,
        };
        for _ in 0..o.steps {
            let mut r = Rng::new(key(&[o.seed, 0xfeed, self.meta.steps]));
            let mut pick: Vec<usize> = (0..fresh).collect();
            if data.len() > fresh {
                pick.extend((0..o.replay).map(|_| fresh + r.below(data.len() - fresh)));
            }
            let wsum: f32 = pick.iter().map(|&i| data[i].weight).sum();
            if wsum <= 0.0 {
                break;
            }
            let items: Vec<Item> = pick
                .iter()
                .map(|&i| Item {
                    ex: i,
                    coef: data[i].weight / wsum,
                    how: How::Whole,
                })
                .collect();
            let (loss, _) = batch_grad(&self.net, &data, &items, 0.0, &pool, &bufs, &mut grad);
            let (scale, _) = clip_scale(&grad, o.clip, &mask);
            let adam = self.opt.as_mut().expect("set above");
            adam.step(&mut self.net, &grad, o.lr, &adam_o, scale, &mask, Some(&pool));
            self.meta.steps += 1;
            rep.losses.push(loss);
        }
        rep.after = fb.iter().map(|f| feedback_prob(self, f)).collect();
        let mut kept: Vec<Example> = data[..fresh].to_vec();
        kept.append(&mut self.meta.feedback);
        kept.truncate(FEEDBACK_KEPT);
        self.meta.feedback = kept;
        Ok(rep)
    }
}

/// a few AdamW steps at a flat rate on whole examples: every step takes the first
/// `fresh` examples and `o.replay` drawn from the rest (keyed by seed and step), the
/// loss on all windows. What `Classifier::feedback` and generator reinforcement share.
/// Returns each step's loss.
pub fn tune(clf: &mut Classifier, data: &[Example], fresh: usize, o: &FeedbackOpts) -> Result<Vec<f64>, String> {
    check(clf, data)?;
    let pool = Pool::new(if o.threads == 0 { pool::available() } else { o.threads });
    if !clf.opt.as_ref().is_some_and(|a| a.fits(&clf.net)) {
        clf.opt = Some(Adam::new(&clf.net));
    }
    let mask = trainable(&clf.net, o.freeze_trunk);
    let bufs = buffers(&clf.net, pool.threads());
    let mut grad = clf.net.zeros_like();
    let adam_o = AdamOpts {
        b1: 0.9,
        b2: 0.98,
        eps: 1e-8,
        weight_decay: 0.0,
    };
    let mut losses = Vec::new();
    for _ in 0..o.steps {
        let mut r = Rng::new(key(&[o.seed, 0x7e5e, clf.meta.steps]));
        let mut pick: Vec<usize> = (0..fresh.min(data.len())).collect();
        if data.len() > fresh {
            pick.extend((0..o.replay).map(|_| fresh + r.below(data.len() - fresh)));
        }
        let wsum: f32 = pick.iter().map(|&i| data[i].weight).sum();
        if wsum <= 0.0 {
            break;
        }
        let items: Vec<Item> = pick
            .iter()
            .map(|&i| Item {
                ex: i,
                coef: data[i].weight / wsum,
                how: How::Whole,
            })
            .collect();
        let (loss, _) = batch_grad(&clf.net, data, &items, 0.0, &pool, &bufs, &mut grad);
        let (scale, _) = clip_scale(&grad, o.clip, &mask);
        let adam = clf.opt.as_mut().expect("set above");
        adam.step(&mut clf.net, &grad, o.lr, &adam_o, scale, &mask, Some(&pool));
        clf.meta.steps += 1;
        losses.push(loss);
    }
    Ok(losses)
}

/// the generative head's negative log-likelihood of `data[from..]` given what comes
/// before it (in nats, summed), and how many predictions that is (the end included)
pub fn nll(clf: &Classifier, data: &[u8], from: usize) -> (f64, usize) {
    let net = &clf.net;
    let (ts, from) = net.sequence(data, from);
    let (ws, starts) = net.windows_over(&ts);
    let extra = vec![0.0; net.cfg.extra];
    let (mut nats, mut count) = (0f64, 0usize);
    let mut seen = 0usize;
    for (w, &st) in ws.iter().zip(&starts) {
        let tr = net.forward(w, &extra, &clf.gains(), Mode::EVAL, None);
        // windows overlap: count each predicted byte once, in the first window holding it
        let t = Target::Next { from: from.max(seen) };
        let (l, _, c) = gen_loss(net, &net.gen_logits(&tr), &w.tokens, st, &t);
        nats += l as f64 * c as f64;
        count += c;
        seen = st + w.len() - 1;
    }
    (nats, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loss_gradients() {
        let l = [0.3f32, -1.2, 0.8, 0.1];
        let cases = [
            (HeadKind::Single, Target::Class(2), 0.1),
            (HeadKind::Single, Target::Not(0), 0.0),
            (HeadKind::Single, Target::Soft(vec![0.1, 0.2, 0.3, 0.4]), 0.0),
            (HeadKind::Multi, Target::Labels(vec![1.0, 0.0, 1.0, 0.0]), 0.1),
            (HeadKind::Multi, Target::Label(1, true), 0.0),
            (HeadKind::Regress, Target::Values(vec![1.0, 0.0, -1.0, 2.0]), 0.0),
        ];
        for (kind, t, sm) in cases {
            let (_, g) = head_loss(kind, &l, &t, sm).unwrap();
            for i in 0..4 {
                let (mut a, mut b) = (l, l);
                a[i] += 1e-2;
                b[i] -= 1e-2;
                let num = (head_loss(kind, &a, &t, sm).unwrap().0 - head_loss(kind, &b, &t, sm).unwrap().0) / 2e-2;
                assert!((num - g[i]).abs() < 1e-3, "{kind:?} {t:?} {i}: {num} {}", g[i]);
            }
        }
        assert!(head_loss(HeadKind::Single, &l, &Target::Values(vec![0.0; 4]), 0.0).is_err());
        assert!(head_loss(HeadKind::Single, &l, &Target::Class(9), 0.0).is_err());
    }

    #[test]
    fn schedule_and_split() {
        assert!((lr_at(0, 100, 10, 1.0, 0.1) - 0.1).abs() < 1e-6);
        assert_eq!(lr_at(9, 100, 10, 1.0, 0.1), 1.0);
        assert!((lr_at(100, 100, 10, 1.0, 0.1) - 0.1).abs() < 1e-6);
        assert!((lr_at(55, 100, 10, 1.0, 0.1) - 0.55).abs() < 1e-3);
        let data: Vec<Example> = (0..1000).map(|i| Example::class(&format!("text {i}"), 0)).collect();
        let (t, v) = split(&data, 0.2, 1);
        assert!((150..250).contains(&v.len()) && t.len() + v.len() == 1000);
        assert_eq!(split(&data, 0.2, 1), (t, v));
    }
}
