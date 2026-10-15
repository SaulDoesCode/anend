//! A one-time machine benchmark: how fast this computer trains models of a ladder of
//! sizes, and how the cost of a forward pass grows with sequence length. `suggest`
//! turns the measurement into a model size for a task and a time budget, which the
//! declarative spec uses when sizes are left out.
//!
//! Limits here are measured compute, not convenience: a size is suggested when the
//! training it implies fits the budget on this machine. The GPU backend (backlog, see
//! `kernels`) is the main lever that would lift them.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::model::{Config, Gains, HeadSpec, Mode, Net, Pooling};
use super::pool;
use super::text;
use super::train::{self, Example, TrainOpts};
use super::{Classifier, Flow};

/// one rung of the size ladder
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rung {
    pub d_model: usize,
    pub layers: usize,
    pub heads: usize,
    pub d_ff: usize,
    pub weights: usize,
    /// training throughput at 128-token windows, all threads
    pub tokens_per_sec: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MachineProfile {
    pub threads: usize,
    pub simd: String,
    pub ladder: Vec<Rung>,
    /// (tokens, milliseconds) for one forward pass of the second rung at that length:
    /// attention makes it grow faster than linearly
    pub forward_ms: Vec<(usize, f64)>,
}

/// the sizes measured, smallest first
pub const LADDER: [(usize, usize, usize, usize); 6] = [
    (32, 2, 2, 96),
    (64, 2, 2, 176),
    (96, 3, 3, 256),
    (128, 4, 4, 352),
    (192, 4, 6, 512),
    (256, 6, 8, 704),
];

fn config(d: usize, layers: usize, heads: usize, ff: usize, max_len: usize) -> Config {
    Config {
        d_model: d,
        head_dim: d / heads,
        heads,
        layers,
        d_ff: ff,
        max_len,
        stride: max_len / 2,
        dropout: 0.0,
        pooling: Pooling::Mean,
        ..Config::tiny().gematria(false)
    }
}

/// measures the ladder, spending about `per_rung` on each size (rungs that take more
/// than four budgets for one step end the ladder: bigger ones would not be suggested)
pub fn measure(per_rung: Duration) -> MachineProfile {
    let threads = pool::available();
    let text: String = (0..64).map(|i| format!("word{} ", i * 7919 % 1000)).collect();
    let data: Vec<Example> = (0..64).map(|i| Example::class(&text[i..i + 300], i % 2)).collect();
    let mut ladder = Vec::new();
    for &(d, layers, heads, ff) in &LADDER {
        let mut clf = Classifier::new(config(d, layers, heads, ff, 128), &[HeadSpec::single("k", &["a", "b"])], 1).unwrap();
        let started = Instant::now();
        let mut tokens = 0u64;
        let mut steps = 0;
        while started.elapsed() < per_rung || steps == 0 {
            let o = TrainOpts {
                epochs: 1,
                batch: 16,
                max_steps: 1,
                val_fraction: 0.0,
                seed: steps,
                ..TrainOpts::default()
            };
            let r = train::train(&mut clf, &data, &o, |_| Flow::Go).unwrap();
            tokens += r.tokens;
            steps += 1;
        }
        let secs = started.elapsed().as_secs_f64();
        ladder.push(Rung {
            d_model: d,
            layers,
            heads,
            d_ff: ff,
            weights: clf.net.weight_count(),
            tokens_per_sec: tokens as f64 / secs,
        });
        if steps == 1 && secs > 4.0 * per_rung.as_secs_f64() {
            break;
        }
    }
    let (d, layers, heads, ff) = LADDER[1];
    let mut forward_ms = Vec::new();
    for len in [64usize, 128, 256, 512, 1024] {
        let net = Net::new(config(d, layers, heads, ff, len), &[HeadSpec::single("k", &["a"])], 1).unwrap();
        let w = text::window(&vec![b'a'; len - 2]);
        let started = Instant::now();
        let mut n = 0;
        while n == 0 || started.elapsed() < per_rung / 5 {
            drop(net.forward(&w, &[], &Gains::default(), Mode::EVAL, None));
            n += 1;
        }
        forward_ms.push((len, started.elapsed().as_secs_f64() * 1e3 / n as f64));
    }
    MachineProfile {
        threads,
        simd: super::kernels::simd_name().into(),
        ladder,
        forward_ms,
    }
}

/// the profile saved at `path`, or a fresh measurement saved there
pub fn load_or_measure(path: &std::path::Path, per_rung: Duration) -> MachineProfile {
    if let Some(p) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
        return p;
    }
    let p = measure(per_rung);
    if let Ok(j) = serde_json::to_vec_pretty(&p) {
        let _ = std::fs::write(path, j);
    }
    p
}

/// what a model is for, to size it
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// training examples
    pub examples: usize,
    /// passes over them
    pub epochs: usize,
    /// tokens per example window
    pub max_len: usize,
    /// seconds of training the person will wait
    pub seconds: f64,
}

/// the largest measured size whose training (examples × epochs × max_len tokens at the
/// measured rate, scaled for the window length by the forward-pass curve) fits the
/// budget; the smallest rung when none does
pub fn suggest(p: &MachineProfile, t: &Task) -> Config {
    let tokens = (t.examples * t.epochs.max(1) * t.max_len) as f64;
    // longer windows cost more per token (attention); scale from the 128-token rate
    let per_token = |len: usize| -> f64 {
        let at = |l: usize| p.forward_ms.iter().find(|x| x.0 >= l).or(p.forward_ms.last()).map_or(1.0, |x| x.1 / x.0 as f64);
        at(len) / at(128).max(1e-12)
    };
    let slow = per_token(t.max_len).max(1.0);
    let mut best = &p.ladder[0];
    for r in &p.ladder {
        if tokens * slow / r.tokens_per_sec.max(1e-9) <= t.seconds {
            best = r;
        }
    }
    config(best.d_model, best.layers, best.heads, best.d_ff, t.max_len.max(8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_within_budget() {
        let p = MachineProfile {
            threads: 4,
            simd: "test".into(),
            ladder: LADDER
                .iter()
                .enumerate()
                .map(|(i, &(d, l, h, f))| Rung {
                    d_model: d,
                    layers: l,
                    heads: h,
                    d_ff: f,
                    weights: 0,
                    tokens_per_sec: 100_000.0 / (i + 1) as f64,
                })
                .collect(),
            forward_ms: vec![(64, 1.0), (128, 2.0), (256, 5.0)],
        };
        let t = Task {
            examples: 1000,
            epochs: 10,
            max_len: 128,
            seconds: 60.0,
        };
        // 1.28 M tokens in 60 s needs ≥ 21 k tokens/s: rung 4 (25 k) fits, rung 5 (20 k) doesn't
        assert_eq!(suggest(&p, &t).d_model, LADDER[3].0);
        assert_eq!(suggest(&p, &Task { seconds: 1.0, ..t }).d_model, LADDER[0].0);
        // longer windows cost more per token
        assert!(suggest(&p, &Task { max_len: 256, ..t }).d_model < LADDER[3].0);
        assert!(suggest(&p, &t).validate().is_ok());
    }
}
