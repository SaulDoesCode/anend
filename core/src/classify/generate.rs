//! Generation, and generation shaped by classifiers ("the skinner box").
//!
//! A model with a generative head (`Net::generator`, causal) writes bytes one at a time
//! through `Decoder`. `sample` picks each byte greedily or by temperature, top-k and
//! top-p from a random stream keyed by (seed, candidate index), so the same seed gives
//! the same bytes on any thread count and CPU.
//!
//! Critics are other models (any `Judge`: a classifier, a node of a `Graph`, a stored
//! stack) each with what is wanted of one of its heads and a weight. `generate` draws
//! candidates, scores each by every critic, drops those breaking the `Constraints`
//! (must / must-not classes, sum ranges, length, bytes that must or mustn't appear) and
//! ranks the rest. `Guidance` reweights the top next bytes during sampling by the
//! critics' scores of the prefix extended by each. `reinforce` closes the loop: the
//! generator is fine-tuned on samples weighted by reward (critic scores relative to
//! their mean, or a person's yes/no): likelihood for good ones, unlikelihood for bad.

use serde::{Deserialize, Serialize};

use super::Classifier;
use super::compose::Graph;
use super::kernels::softmax_rows;
use super::math::{self, Rng, key};
use super::model::{Decoder, END, GEN_OUT};
use super::pool::{self, Pool};
use super::steer::Output;
use super::train::{self, Example, FeedbackOpts, Target};

// ---------------------------------------------------------------- sampling

/// how each next byte is picked
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sampling {
    /// 0 picks the most likely byte (greedy); otherwise logits are divided by it
    pub temperature: f32,
    /// keep only the k most likely bytes (0: all)
    pub top_k: usize,
    /// keep the fewest most likely bytes whose probabilities reach p (1: all)
    pub top_p: f32,
    /// bytes to write at most (the prompt not counted)
    pub max_len: usize,
    /// a byte that ends the output (kept in it); END always ends it
    pub stop: Option<u8>,
    pub seed: u64,
}

impl Default for Sampling {
    fn default() -> Sampling {
        Sampling {
            temperature: 0.8,
            top_k: 0,
            top_p: 0.95,
            max_len: 200,
            stop: None,
            seed: 0,
        }
    }
}

impl Sampling {
    pub fn greedy(max_len: usize) -> Sampling {
        Sampling {
            temperature: 0.0,
            max_len,
            ..Sampling::default()
        }
    }

    pub fn check(&self) -> Result<(), String> {
        if !(self.temperature >= 0.0 && self.temperature.is_finite()) {
            return Err("temperature must be 0 or more".into());
        }
        if !(self.top_p > 0.0 && self.top_p <= 1.0) {
            return Err("top_p must be in (0, 1]".into());
        }
        Ok(())
    }
}

/// picks a next token (0..GEN_OUT) from logits; `adjust` (if any) is added to the
/// scaled logits of the candidates it names. Ties go to the lower token.
pub fn pick(logits: &[f32], s: &Sampling, r: &mut Rng) -> usize {
    if s.temperature == 0.0 {
        return argmax(logits);
    }
    let z: Vec<f32> = logits.iter().map(|l| l / s.temperature).collect();
    let mut order: Vec<usize> = (0..z.len()).collect();
    order.sort_by(|&a, &b| z[b].total_cmp(&z[a]).then(a.cmp(&b)));
    if s.top_k > 0 {
        order.truncate(s.top_k);
    }
    let mut p: Vec<f32> = order.iter().map(|&i| z[i]).collect();
    let w = p.len();
    softmax_rows(&mut p, w);
    if s.top_p < 1.0 {
        let mut acc = 0f32;
        let mut keep = p.len();
        for (i, &v) in p.iter().enumerate() {
            acc += v;
            if acc >= s.top_p {
                keep = i + 1;
                break;
            }
        }
        order.truncate(keep);
        p.truncate(keep);
        let t: f32 = p.iter().sum();
        p.iter_mut().for_each(|v| *v /= t);
    }
    let u = r.unit();
    let mut acc = 0f32;
    for (i, &v) in p.iter().enumerate() {
        acc += v;
        if u < acc {
            return order[i];
        }
    }
    *order.last().expect("at least one")
}

fn argmax(v: &[f32]) -> usize {
    let mut best = 0;
    for (i, &x) in v.iter().enumerate() {
        if x > v[best] {
            best = i;
        }
    }
    best
}

/// ln softmax(logits)[i]
fn log_prob(logits: &[f32], i: usize) -> f32 {
    let m = logits.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let e: Vec<f32> = logits.iter().map(|&l| math::exp(l - m)).collect();
    (logits[i] - m) - math::ln(super::kernels::sum(&e))
}

/// one sample: the written bytes (prompt excluded) and their log-probability under the
/// generator (unguided, temperature 1). `index` keys the random stream.
pub fn sample(generator: &Classifier, prompt: &[u8], s: &Sampling, index: u64) -> (Vec<u8>, f32) {
    sample_guided(generator, prompt, s, index, None)
}

/// reweights scaled next-byte logits in place, given the bytes written so far
type Guide<'a> = &'a dyn Fn(&[u8], &mut [f32]);

fn sample_guided(
    generator: &Classifier,
    prompt: &[u8],
    s: &Sampling,
    index: u64,
    guide: Option<Guide>,
) -> (Vec<u8>, f32) {
    let mut r = Rng::new(key(&[s.seed, index, 0x5a3e]));
    let mut dec = Decoder::new(&generator.net, generator.gains(), prompt);
    let mut out = Vec::new();
    let mut logp = 0f32;
    while out.len() < s.max_len {
        let mut l = dec.logits().to_vec();
        let base = l.clone();
        if let Some(g) = guide {
            g(&out, &mut l);
        }
        let t = pick(&l, s, &mut r);
        logp += log_prob(&base, t);
        if t == END {
            break;
        }
        let b = t as u8;
        out.push(b);
        if s.stop == Some(b) {
            break;
        }
        dec.push(b);
    }
    (out, logp)
}

// ---------------------------------------------------------------- critics

/// anything that reads bytes into steered head outputs
pub trait Judge: Sync {
    fn judge(&self, data: &[u8]) -> Vec<Output>;
}

impl Judge for Classifier {
    fn judge(&self, data: &[u8]) -> Vec<Output> {
        self.predict_bytes(data, &vec![0.0; self.net.cfg.extra], None)
    }
}

/// one node of a graph of classifiers, its upstream run first
pub struct Node<'a> {
    pub graph: &'a Graph,
    pub node: usize,
}

impl Judge for Node<'_> {
    fn judge(&self, data: &[u8]) -> Vec<Output> {
        self.graph.predict(self.node, &String::from_utf8_lossy(data))
    }
}

impl Judge for crate::classifiers::Stack {
    fn judge(&self, data: &[u8]) -> Vec<Output> {
        self.graph.predict(self.node, &String::from_utf8_lossy(data))
    }
}

/// what a critic wants of its head, as a score in [0, 1]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Want {
    /// the class's (or label's) probability
    Class(usize),
    /// one minus it
    Not(usize),
    /// a regression value near `value`: exp(−|v − value| / scale)
    Near { index: usize, value: f32, scale: f32 },
    /// a regression value in [lo, hi]: 1 inside, exp(−distance / scale) outside
    Range { index: usize, lo: f32, hi: f32, scale: f32 },
}

impl Want {
    pub fn score(&self, o: &Output) -> f32 {
        let get = |i: usize| o.probs.get(i).copied().unwrap_or(0.0);
        match *self {
            Want::Class(c) => get(c),
            Want::Not(c) => 1.0 - get(c),
            Want::Near { index, value, scale } => math::exp(-(get(index) - value).abs() / scale.max(1e-6)),
            Want::Range { index, lo, hi, scale } => {
                let v = get(index);
                let d = if v < lo {
                    lo - v
                } else if v > hi {
                    v - hi
                } else {
                    0.0
                };
                math::exp(-d / scale.max(1e-6))
            }
        }
    }
}

/// a model judging candidates
pub struct Critic<'a> {
    pub judge: &'a dyn Judge,
    pub head: usize,
    pub want: Want,
    pub weight: f32,
}

impl<'a> Critic<'a> {
    pub fn new(judge: &'a dyn Judge, head: usize, want: Want, weight: f32) -> Critic<'a> {
        Critic {
            judge,
            head,
            want,
            weight,
        }
    }
}

/// a running sum a constraint can bound
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Measure {
    /// Σ (byte + 1), `crate::measure`
    Bytes,
    /// Σ (256 − byte)
    BytesReversed,
    /// letter sums a = 1 .. z = 26 and reversed, `crate::simd`
    Ordinal,
    Reversed,
    /// the number of bytes
    Length,
}

impl Measure {
    pub fn of(self, data: &[u8]) -> u64 {
        match self {
            Measure::Bytes => crate::measure::byte_sums(data).bytes,
            Measure::BytesReversed => crate::measure::byte_sums(data).reversed,
            Measure::Ordinal => crate::simd::sums(data).ordinal,
            Measure::Reversed => crate::simd::sums(data).reversed,
            Measure::Length => data.len() as u64,
        }
    }
}

/// a critic's verdict that must (or must not) hold: `class` among its head's members
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gate {
    pub critic: usize,
    pub class: usize,
    pub member: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SumRange {
    pub measure: Measure,
    pub lo: u64,
    pub hi: u64,
    /// compare the sum modulo this (0: the sum itself)
    #[serde(default)]
    pub modulo: u64,
}

/// what a kept candidate must satisfy
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Constraints {
    pub min_len: usize,
    /// 0: no limit
    pub max_len: usize,
    pub gates: Vec<Gate>,
    pub sums: Vec<SumRange>,
    pub contains: Vec<Vec<u8>>,
    pub excludes: Vec<Vec<u8>>,
}

fn has(data: &[u8], pat: &[u8]) -> bool {
    pat.is_empty() || data.windows(pat.len()).any(|w| w == pat)
}

impl Constraints {
    /// the constraints a candidate breaks, in words (empty: it passes)
    pub fn broken(&self, data: &[u8], outs: &[Vec<Output>]) -> Vec<String> {
        let mut v = Vec::new();
        if data.len() < self.min_len {
            v.push(format!("shorter than {}", self.min_len));
        }
        if self.max_len > 0 && data.len() > self.max_len {
            v.push(format!("longer than {}", self.max_len));
        }
        for g in &self.gates {
            let member = outs
                .get(g.critic)
                .is_some_and(|o| o.first().is_some_and(|o| o.members.contains(&g.class)));
            if member != g.member {
                v.push(format!(
                    "critic {} {} class {}",
                    g.critic,
                    if g.member { "lacks" } else { "has" },
                    g.class
                ));
            }
        }
        for s in &self.sums {
            let mut x = s.measure.of(data);
            if s.modulo > 0 {
                x %= s.modulo;
            }
            if x < s.lo || x > s.hi {
                v.push(format!("{:?} {x} outside {}..={}", s.measure, s.lo, s.hi));
            }
        }
        for p in &self.contains {
            if !has(data, p) {
                v.push(format!("lacks {:?}", String::from_utf8_lossy(p)));
            }
        }
        for p in &self.excludes {
            if !p.is_empty() && has(data, p) {
                v.push(format!("has {:?}", String::from_utf8_lossy(p)));
            }
        }
        v
    }
}

/// sampling-time steering by the critics
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Guidance {
    /// how many of the generator's most likely next bytes the critics weigh; the rest
    /// are dropped on guided steps
    pub top_k: usize,
    /// added to each weighed byte's scaled logit: strength × Σ weight·ln(score)
    pub strength: f32,
    /// guide every n-th byte (1: every byte); critics cost a pass each per weighed byte
    pub every: usize,
}

impl Default for Guidance {
    fn default() -> Guidance {
        Guidance {
            top_k: 4,
            strength: 2.0,
            every: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GenOpts {
    pub prompt: Vec<u8>,
    pub sampling: Sampling,
    /// candidates drawn
    pub candidates: usize,
    /// candidates returned at most (0: all that pass)
    pub keep: usize,
    pub guidance: Option<Guidance>,
    /// critics see the prompt too (for continuations); otherwise only the written bytes
    pub judge_prompt: bool,
    /// 0 for every core
    pub threads: usize,
}

impl Default for GenOpts {
    fn default() -> GenOpts {
        GenOpts {
            prompt: Vec::new(),
            sampling: Sampling::default(),
            candidates: 8,
            keep: 0,
            guidance: None,
            judge_prompt: false,
            threads: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// the draw's index (its random stream)
    pub index: u64,
    /// the written bytes (no prompt)
    pub bytes: Vec<u8>,
    /// per critic, its score in [0, 1]
    pub scores: Vec<f32>,
    /// Σ weight·ln(score) over Σ weight (0 without critics)
    pub score: f32,
    /// the generator's log-probability of the bytes
    pub logp: f32,
    /// constraints broken (empty: kept)
    pub broken: Vec<String>,
}

impl Candidate {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    pub fn passed(&self) -> bool {
        self.broken.is_empty()
    }
}

fn combine(critics: &[Critic], scores: &[f32]) -> f32 {
    let wsum: f32 = critics.iter().map(|c| c.weight).sum();
    if wsum <= 0.0 {
        return 0.0;
    }
    let s: f32 = critics
        .iter()
        .zip(scores)
        .map(|(c, &v)| c.weight * math::ln(v.max(1e-6)))
        .sum();
    s / wsum
}

/// every critic's outputs and score on some bytes
fn judge_all(critics: &[Critic], data: &[u8]) -> (Vec<Vec<Output>>, Vec<f32>) {
    let outs: Vec<Vec<Output>> = critics.iter().map(|c| c.judge.judge(data)).collect();
    let scores = critics
        .iter()
        .zip(&outs)
        .map(|(c, o)| o.get(c.head).map_or(0.0, |o| c.want.score(o)))
        .collect();
    (outs, scores)
}

/// draws `opts.candidates` candidates in parallel and judges them all; each candidate
/// is a function of (generator, critics, opts, its index) only, so the thread count
/// changes nothing
pub fn draw(
    generator: &Classifier,
    critics: &[Critic],
    c: &Constraints,
    opts: &GenOpts,
) -> Result<Vec<Candidate>, String> {
    opts.sampling.check()?;
    if generator.net.generator.is_none() {
        return Err("the model has no generative head".into());
    }
    let pool = Pool::new(if opts.threads == 0 {
        pool::available()
    } else {
        opts.threads
    });
    let one = |i: u64| -> Candidate {
        let shown = |out: &[u8]| -> Vec<u8> {
            if opts.judge_prompt {
                [&opts.prompt[..], out].concat()
            } else {
                out.to_vec()
            }
        };
        let guide = |out: &[u8], l: &mut [f32]| {
            let Some(g) = &opts.guidance else { return };
            if critics.is_empty() || !out.len().is_multiple_of(g.every.max(1)) {
                return;
            }
            let mut order: Vec<usize> = (0..GEN_OUT).collect();
            order.sort_by(|&a, &b| l[b].total_cmp(&l[a]).then(a.cmp(&b)));
            order.truncate(g.top_k.max(1));
            let mut adj = vec![f32::NEG_INFINITY; GEN_OUT];
            for &t in &order {
                let ext = if t == END {
                    out.to_vec()
                } else {
                    [out, &[t as u8]].concat()
                };
                let (_, sc) = judge_all(critics, &shown(&ext));
                // the scaled logit is raised by strength·score, so temperature still applies
                adj[t] = l[t] + g.strength * combine(critics, &sc) * opts.sampling.temperature.max(1e-3);
            }
            l.copy_from_slice(&adj);
        };
        let (bytes, logp) = sample_guided(
            generator,
            &opts.prompt,
            &opts.sampling,
            i,
            if opts.guidance.is_some() { Some(&guide) } else { None },
        );
        let seen = shown(&bytes);
        let (outs, scores) = judge_all(critics, &seen);
        Candidate {
            index: i,
            score: combine(critics, &scores),
            broken: c.broken(&seen, &outs),
            bytes,
            scores,
            logp,
        }
    };
    let slots: Vec<std::sync::Mutex<Option<Candidate>>> = (0..opts.candidates).map(|_| Default::default()).collect();
    pool.run(opts.candidates, &|i| *slots[i].lock().unwrap() = Some(one(i as u64)));
    Ok(slots.into_iter().map(|s| s.into_inner().unwrap().unwrap()).collect())
}

/// the candidates that pass the constraints, best score first (ties: lower index)
pub fn generate(
    generator: &Classifier,
    critics: &[Critic],
    c: &Constraints,
    opts: &GenOpts,
) -> Result<Vec<Candidate>, String> {
    let mut v: Vec<Candidate> = draw(generator, critics, c, opts)?
        .into_iter()
        .filter(|c| c.passed())
        .collect();
    v.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.index.cmp(&b.index)));
    if opts.keep > 0 {
        v.truncate(opts.keep);
    }
    Ok(v)
}

// ---------------------------------------------------------------- reinforcement

/// a sample with how good it was: above 0 is learned toward (likelihood, weighted by
/// the reward), below 0 away from (unlikelihood, weighted by −reward)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rewarded {
    pub prompt: Vec<u8>,
    pub bytes: Vec<u8>,
    pub reward: f32,
}

impl Rewarded {
    /// a person's yes on a sample
    pub fn yes(bytes: &[u8]) -> Rewarded {
        Rewarded {
            prompt: Vec::new(),
            bytes: bytes.to_vec(),
            reward: 1.0,
        }
    }

    pub fn no(bytes: &[u8]) -> Rewarded {
        Rewarded {
            reward: -1.0,
            ..Rewarded::yes(bytes)
        }
    }
}

/// rewards from critic scores: each candidate's score minus the mean score (so the
/// better half is learned toward, the worse half away from), times `scale`; candidates
/// that broke constraints get −scale
pub fn rewards(cands: &[Candidate], prompt: &[u8], scale: f32) -> Vec<Rewarded> {
    let ok: Vec<&Candidate> = cands.iter().filter(|c| c.passed()).collect();
    let mean = if ok.is_empty() {
        0.0
    } else {
        ok.iter().map(|c| c.score).sum::<f32>() / ok.len() as f32
    };
    cands
        .iter()
        .map(|c| Rewarded {
            prompt: prompt.to_vec(),
            bytes: c.bytes.clone(),
            reward: if c.passed() { (c.score - mean) * scale } else { -scale },
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReinforceOpts {
    pub steps: usize,
    pub lr: f32,
    /// rewards with a smaller magnitude are skipped
    pub min_reward: f32,
    /// weight of the replayed examples (the generator's own training texts, which keep
    /// it from drifting off its data)
    pub replay_weight: f32,
    /// replayed examples per step
    pub replay: usize,
    pub freeze_trunk: bool,
    pub clip: f32,
    pub seed: u64,
    pub threads: usize,
}

impl Default for ReinforceOpts {
    fn default() -> ReinforceOpts {
        ReinforceOpts {
            steps: 8,
            lr: 3e-4,
            min_reward: 1e-3,
            replay_weight: 0.5,
            replay: 8,
            freeze_trunk: false,
            clip: 1.0,
            seed: 0,
            threads: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReinforceReport {
    /// mean nats per byte of the rewarded (>0) samples, before and after
    pub good_before: f64,
    pub good_after: f64,
    /// the same for the penalised (<0) samples
    pub bad_before: f64,
    pub bad_after: f64,
    pub losses: Vec<f64>,
    pub used: usize,
}

fn mean_nll(generator: &Classifier, s: &[&Rewarded]) -> f64 {
    let (mut n, mut c) = (0f64, 0usize);
    for r in s {
        let data = [&r.prompt[..], &r.bytes[..]].concat();
        let (a, b) = train::nll(generator, &data, r.prompt.len());
        n += a;
        c += b;
    }
    if c == 0 { 0.0 } else { n / c as f64 }
}

/// fine-tunes the generator on rewarded samples (reward-weighted likelihood, and
/// unlikelihood for negative rewards), with `replay` texts mixed in
pub fn reinforce(
    generator: &mut Classifier,
    samples: &[Rewarded],
    replay: &[Example],
    o: &ReinforceOpts,
) -> Result<ReinforceReport, String> {
    let used: Vec<&Rewarded> = samples.iter().filter(|s| s.reward.abs() >= o.min_reward).collect();
    let good: Vec<&Rewarded> = used.iter().copied().filter(|s| s.reward > 0.0).collect();
    let bad: Vec<&Rewarded> = used.iter().copied().filter(|s| s.reward < 0.0).collect();
    let mut data: Vec<Example> = used
        .iter()
        .map(|s| {
            let from = s.prompt.len();
            let t = if s.reward > 0.0 {
                Target::Next { from }
            } else {
                Target::Avoid { from }
            };
            Example::bytes(&[&s.prompt[..], &s.bytes[..]].concat(), 0, t).weighted(s.reward.abs())
        })
        .collect();
    let fresh = data.len();
    data.extend(replay.iter().map(|e| e.clone().weighted(e.weight * o.replay_weight)));
    let mut rep = ReinforceReport {
        good_before: mean_nll(generator, &good),
        bad_before: mean_nll(generator, &bad),
        used: fresh,
        ..ReinforceReport::default()
    };
    if fresh > 0 {
        let fo = FeedbackOpts {
            steps: o.steps,
            lr: o.lr,
            replay: o.replay,
            freeze_trunk: o.freeze_trunk,
            clip: o.clip,
            seed: o.seed,
            threads: o.threads,
        };
        rep.losses = train::tune(generator, &data, fresh, &fo)?;
    }
    rep.good_after = mean_nll(generator, &good);
    rep.bad_after = mean_nll(generator, &bad);
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::model::{Config, GenSpec, HeadSpec, Pooling};
    use crate::classify::{Flow, TrainOpts};

    /// a tiny generator (tests only: sizes chosen for debug-build speed)
    pub fn tiny_generator(seed: u64) -> Classifier {
        let cfg = Config {
            causal: true,
            pooling: Pooling::Mean,
            max_len: 40,
            stride: 19,
            dropout: 0.0,
            ..Config::tiny().gematria(false)
        };
        Classifier::with_gen(cfg, &[], Some(GenSpec::new("next")), seed).unwrap()
    }

    fn corpus() -> Vec<Example> {
        let words = ["ab", "abc", "abcd"];
        (0..24)
            .map(|i| Example::generate(&format!("{}.", words[i % 3])))
            .collect()
    }

    fn trained(threads: usize) -> Classifier {
        let mut g = tiny_generator(3);
        let o = TrainOpts {
            epochs: 40,
            batch: 8,
            lr: 1e-2,
            val_fraction: 0.0,
            threads,
            seed: 1,
            label_smoothing: 0.0,
            ..TrainOpts::default()
        };
        train::train(&mut g, &corpus(), &o, |_| Flow::Go).unwrap();
        g
    }

    #[test]
    fn learns_sequences_and_samples_deterministically() {
        let g = trained(2);
        assert_eq!(g.to_bytes(), trained(3).to_bytes(), "thread count changed the weights");
        let (out, _) = sample(&g, b"", &Sampling::greedy(10), 0);
        assert!(out.starts_with(b"ab"), "greedy {:?}", String::from_utf8_lossy(&out));
        let s = Sampling {
            temperature: 1.0,
            top_k: 5,
            top_p: 0.9,
            max_len: 12,
            stop: Some(b'.'),
            seed: 9,
        };
        let a: Vec<_> = (0..6).map(|i| sample(&g, b"", &s, i)).collect();
        let b: Vec<_> = (0..6).map(|i| sample(&g, b"", &s, i)).collect();
        assert_eq!(a, b);
        assert!(a.iter().all(|(o, _)| o.len() <= 12));
        assert!(a.iter().filter(|(o, _)| o.starts_with(b"ab")).count() >= 4, "{a:?}");
        // the sampling seed matters
        let other: Vec<_> = (0..6)
            .map(|i| sample(&g, b"", &Sampling { seed: 10, ..s.clone() }, i))
            .collect();
        assert!(a.iter().zip(&other).all(|(x, y)| x.0.len() <= 12 && y.0.len() <= 12));
        let p = train::evaluate(&g, &corpus(), &(0..24).collect::<Vec<_>>(), &Pool::new(1));
        assert!(p.bits_per_byte.unwrap() < 1.5, "{:?}", p.bits_per_byte);
    }

    #[test]
    fn critics_rank_constrain_guide_and_reinforce() {
        let mut g = trained(1);
        // a "critic" that likes texts containing 'd': a classifier trained on it
        let mut critic = Classifier::new(
            Config::tiny().gematria(false),
            &[HeadSpec::single("d", &["no", "yes"])],
            2,
        )
        .unwrap();
        let data: Vec<Example> = (0..32)
            .map(|i| {
                let t = ["ab.", "abc.", "abcd.", "abd.", "cd.", "ba."][i % 6];
                Example::class(t, t.contains('d') as usize)
            })
            .collect();
        let o = TrainOpts {
            epochs: 30,
            batch: 8,
            lr: 1e-2,
            val_fraction: 0.0,
            threads: 1,
            ..TrainOpts::default()
        };
        train::train(&mut critic, &data, &o, |_| Flow::Go).unwrap();
        assert!(critic.judge(b"abcd.")[0].probs[1] > 0.7);
        let critics = [Critic::new(&critic, 0, Want::Class(1), 1.0)];
        let opts = GenOpts {
            sampling: Sampling {
                temperature: 1.2,
                max_len: 8,
                stop: Some(b'.'),
                seed: 4,
                ..Sampling::default()
            },
            candidates: 12,
            threads: 2,
            ..GenOpts::default()
        };
        let all = draw(&g, &critics, &Constraints::default(), &opts).unwrap();
        assert_eq!(
            all,
            draw(
                &g,
                &critics,
                &Constraints::default(),
                &GenOpts {
                    threads: 1,
                    ..opts.clone()
                }
            )
            .unwrap()
        );
        let ranked = generate(&g, &critics, &Constraints::default(), &opts).unwrap();
        assert!(ranked.windows(2).all(|w| w[0].score >= w[1].score));
        // constraints: must contain 'd' and be at most 6 bytes
        let c = Constraints {
            max_len: 6,
            contains: vec![b"d".to_vec()],
            ..Constraints::default()
        };
        let kept = generate(&g, &critics, &c, &opts).unwrap();
        assert!(kept.iter().all(|k| k.bytes.contains(&b'd') && k.bytes.len() <= 6));
        let sums = Constraints {
            sums: vec![SumRange {
                measure: Measure::Length,
                lo: 3,
                hi: 4,
                modulo: 0,
            }],
            ..Constraints::default()
        };
        assert!(
            generate(&g, &critics, &sums, &opts)
                .unwrap()
                .iter()
                .all(|k| (3..=4).contains(&k.bytes.len()))
        );
        // guidance raises the mean critic score
        let mean = |v: &[Candidate]| v.iter().map(|c| c.scores[0]).sum::<f32>() / v.len() as f32;
        let guided = draw(
            &g,
            &critics,
            &Constraints::default(),
            &GenOpts {
                guidance: Some(Guidance {
                    top_k: 4,
                    strength: 4.0,
                    every: 1,
                }),
                ..opts.clone()
            },
        )
        .unwrap();
        assert!(
            mean(&guided) > mean(&all),
            "guided {} plain {}",
            mean(&guided),
            mean(&all)
        );
        // reinforcement: the rewarded samples get likelier, the penalised less likely
        let rw = rewards(&all, b"", 1.0);
        let rep = reinforce(
            &mut g,
            &rw,
            &corpus(),
            &ReinforceOpts {
                steps: 6,
                lr: 3e-3,
                threads: 1,
                ..ReinforceOpts::default()
            },
        )
        .unwrap();
        assert!(rep.used > 0 && rep.good_after < rep.good_before, "{rep:?}");
        assert!(rep.bad_after > rep.bad_before, "{rep:?}");
        // a person's yes and no
        let ro = ReinforceOpts {
            threads: 1,
            lr: 3e-3,
            steps: 4,
            ..ReinforceOpts::default()
        };
        let before = train::nll(&g, b"abcd.", 0);
        reinforce(&mut g, &[Rewarded::yes(b"abcd.")], &[], &ro).unwrap();
        assert!(train::nll(&g, b"abcd.", 0).0 < before.0);
        let before = train::nll(&g, b"ba.", 0);
        reinforce(&mut g, &[Rewarded::no(b"ba.")], &[], &ro).unwrap();
        assert!(train::nll(&g, b"ba.", 0).0 > before.0);
    }
}
