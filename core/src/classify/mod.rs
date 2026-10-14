//! Classifiers trained on lists: transformer encoders over a text's bytes and its
//! gematria, written here from the matrix products up and deterministic to the bit.
//! The same data, configuration and seed give the same weights and outputs on any
//! thread count and any CPU (see `kernels` and `math` for how), so a classifier is a
//! repeatable measurement of a text, like its sums, rather than a guess.
//!
//! | module | holds |
//! |---|---|
//! | `text` | bytes to tokens, the gematria channels, windows, the side vector |
//! | `model` | the network: embedder, encoder blocks, pooler, heads; forward and backward |
//! | `kernels` | the numeric kernels (products, norms, softmax), portable and AVX2 |
//! | `math` | exp, ln, tanh and friends from + − × ÷; random numbers and hashes |
//! | `pool` | the thread pool |
//! | `train` | examples, AdamW, the training loop, evaluation, per-classifier feedback |
//! | `metrics` | accuracy, F1, confusion, log-loss, calibration |
//! | `steer` | the player's knobs (bias, temperature, threshold, gains) and the bias metric |
//! | `grow` | widening, deepening and pruning, and the policy deciding when |
//! | `compose` | classifiers feeding classifiers, outputs as lists, feedback through a stack |
//! | `generate` | sampling from a generative head, critics, constraints, guidance, reinforcement |
//! | `io` | the versioned binary file format |
//!
//! Every numeric operation goes through `kernels` and `math`: they are the backend
//! boundary a GPU implementation would replace.

pub mod compose;
pub mod generate;
pub mod grow;
pub mod io;
pub mod kernels;
pub mod math;
pub mod metrics;
pub mod model;
pub mod pool;
pub mod steer;
pub mod text;
pub mod train;

use serde::{Deserialize, Serialize};

pub use model::{Config, Gains, GenSpec, HeadKind, HeadSpec, Net, Pooling};
pub use pool::Pool;
pub use steer::{BiasReport, HeadSteer, Output, Steer};
pub use train::{Example, Flow, Progress, Target, TrainOpts, TrainReport};

use model::Mode;
use text::Window;

/// what a classifier remembers about itself besides its weights
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub name: String,
    pub seed: u64,
    /// optimizer steps taken over its life
    pub steps: u64,
    /// training examples per class, per head, from the last training run
    pub class_counts: Vec<Vec<f32>>,
    /// the most recent feedback, replayed by later feedback steps
    pub feedback: Vec<Example>,
    /// growth and pruning events, oldest first
    pub log: Vec<String>,
}

/// how many feedback examples a classifier keeps
pub const FEEDBACK_KEPT: usize = 256;

#[derive(Clone, Debug)]
pub struct Classifier {
    pub net: Net,
    pub steer: Steer,
    pub meta: Meta,
    /// optimizer moments carried between feedback calls (not saved)
    pub(crate) opt: Option<train::Adam>,
}

/// one text through a classifier, windows averaged
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// per head, the mean of the windows' logits
    pub logits: Vec<Vec<f32>>,
    /// the mean of the windows' pooled vectors
    pub pooled: Vec<f32>,
    pub windows: usize,
}

impl Classifier {
    pub fn new(cfg: Config, heads: &[HeadSpec], seed: u64) -> Result<Classifier, String> {
        Classifier::with_gen(cfg, heads, None, seed)
    }

    /// a model with pooled heads and optionally a generative head (causal configs only)
    pub fn with_gen(cfg: Config, heads: &[HeadSpec], generator: Option<GenSpec>, seed: u64) -> Result<Classifier, String> {
        let net = Net::with_gen(cfg, heads, generator, seed)?;
        let steer = Steer::neutral(&net.heads);
        Ok(Classifier {
            meta: Meta {
                seed,
                class_counts: net.heads.iter().map(|h| vec![0.0; h.width()]).collect(),
                ..Meta::default()
            },
            net,
            steer,
            opt: None,
        })
    }

    pub fn config(&self) -> &Config {
        &self.net.cfg
    }

    /// class names of a head
    pub fn classes(&self, head: usize) -> &[String] {
        &self.net.heads[head].spec.classes
    }

    /// the input gains in force: trained times steered
    pub fn gains(&self) -> Gains {
        self.net.cfg.gains.times(&self.steer.gains)
    }

    pub fn windows(&self, text: &str) -> Vec<Window> {
        self.windows_of(text.as_bytes())
    }

    /// the windows of any bytes (causal windows for a causal model)
    pub fn windows_of(&self, data: &[u8]) -> Vec<Window> {
        let c = &self.net.cfg;
        text::windows_of(data, c.max_len, c.stride, c.causal)
    }

    /// every window of a text through the net with the given gains, averaged in window
    /// order; windows run in parallel on the pool, or a lone window's big products do
    pub fn run(&self, text: &str, extra: &[f32], gains: &Gains, pool: Option<&Pool>) -> Run {
        self.run_bytes(text.as_bytes(), extra, gains, pool)
    }

    /// `run` over any bytes
    pub fn run_bytes(&self, data: &[u8], extra: &[f32], gains: &Gains, pool: Option<&Pool>) -> Run {
        let ws = self.windows_of(data);
        let heads = self.net.heads.len();
        let one = |w: &Window, p: Option<&Pool>| {
            let mut tr = self.net.forward(w, extra, gains, Mode::EVAL, p);
            let l: Vec<Vec<f32>> = (0..heads).map(|h| self.net.logits(&tr, h)).collect();
            (l, std::mem::take(&mut tr.pooled))
        };
        let outs: Vec<(Vec<Vec<f32>>, Vec<f32>)> = match pool {
            Some(p) if ws.len() > 1 && p.threads() > 1 => {
                let slots: Vec<std::sync::Mutex<Option<_>>> = ws.iter().map(|_| Default::default()).collect();
                p.run(ws.len(), &|i| *slots[i].lock().unwrap() = Some(one(&ws[i], None)));
                slots.into_iter().map(|s| s.into_inner().unwrap().unwrap()).collect()
            }
            _ => ws.iter().map(|w| one(w, pool)).collect(),
        };
        let nw = outs.len() as f32;
        let mut logits: Vec<Vec<f32>> = outs[0].0.iter().map(|l| vec![0.0; l.len()]).collect();
        let mut pooled = vec![0f32; outs[0].1.len()];
        for (l, p) in &outs {
            for (acc, x) in logits.iter_mut().zip(l) {
                acc.iter_mut().zip(x).for_each(|(a, b)| *a += b);
            }
            pooled.iter_mut().zip(p).for_each(|(a, b)| *a += b);
        }
        logits.iter_mut().flatten().for_each(|v| *v /= nw);
        pooled.iter_mut().for_each(|v| *v /= nw);
        Run {
            logits,
            pooled,
            windows: outs.len(),
        }
    }

    /// the logits as trained: no steering at all
    pub fn logits(&self, text: &str, extra: &[f32]) -> Vec<Vec<f32>> {
        self.run(text, extra, &self.net.cfg.gains, None).logits
    }

    /// every head's steered output on a text
    pub fn predict(&self, text: &str, extra: &[f32]) -> Vec<Output> {
        self.predict_on(text, extra, None)
    }

    pub fn predict_on(&self, text: &str, extra: &[f32], pool: Option<&Pool>) -> Vec<Output> {
        self.predict_bytes(text.as_bytes(), extra, pool)
    }

    /// every head's steered output on any bytes
    pub fn predict_bytes(&self, data: &[u8], extra: &[f32], pool: Option<&Pool>) -> Vec<Output> {
        let r = self.run_bytes(data, extra, &self.gains(), pool);
        self.read(&r.logits)
    }

    /// steered outputs from window-averaged logits
    pub fn read(&self, logits: &[Vec<f32>]) -> Vec<Output> {
        logits
            .iter()
            .enumerate()
            .map(|(h, l)| steer::apply(self.net.heads[h].spec.kind, &self.steer.heads[h], l))
            .collect()
    }

    /// many texts in parallel (each text's windows in order on one thread); `extras` is
    /// empty or one per text
    pub fn predict_many(&self, texts: &[&str], extras: &[Vec<f32>], pool: &Pool) -> Vec<Vec<Output>> {
        let gains = self.gains();
        self.map_texts(texts, extras, pool, |t, e| {
            self.read(&self.run(t, e, &gains, None).logits)
        })
    }

    /// the pooled vector of a text, windows averaged: a deterministic embedding
    pub fn represent(&self, text: &str) -> Vec<f32> {
        let extra = vec![0.0; self.net.cfg.extra];
        self.run(text, &extra, &self.gains(), None).pooled
    }

    pub fn represent_many(&self, texts: &[&str], pool: &Pool) -> Vec<Vec<f32>> {
        let gains = self.gains();
        self.map_texts(texts, &[], pool, |t, e| self.run(t, e, &gains, None).pooled)
    }

    fn map_texts<T: Send>(
        &self,
        texts: &[&str],
        extras: &[Vec<f32>],
        pool: &Pool,
        f: impl Fn(&str, &[f32]) -> T + Sync,
    ) -> Vec<T> {
        let zero = vec![0.0; self.net.cfg.extra];
        let slots: Vec<std::sync::Mutex<Option<T>>> = texts.iter().map(|_| std::sync::Mutex::new(None)).collect();
        pool.run(texts.len(), &|i| {
            let e = extras.get(i).unwrap_or(&zero);
            *slots[i].lock().unwrap() = Some(f(texts[i], e));
        });
        slots.into_iter().map(|s| s.into_inner().unwrap().unwrap()).collect()
    }

    /// the bias metric for a head over some texts: steered against unsteered
    pub fn bias(&self, head: usize, texts: &[&str], extras: &[Vec<f32>], pool: &Pool) -> BiasReport {
        let kind = self.net.heads[head].spec.kind;
        let neutral = HeadSteer::neutral(self.net.heads[head].width());
        let trained = self.net.cfg.gains;
        let plain: Vec<Output> = self.map_texts(texts, extras, pool, |t, e| {
            steer::apply(kind, &neutral, &self.run(t, e, &trained, None).logits[head])
        });
        let steered: Vec<Output> = self
            .predict_many(texts, extras, pool)
            .into_iter()
            .map(|mut o| o.swap_remove(head))
            .collect();
        let counts = self.meta.class_counts.get(head).cloned().unwrap_or_default();
        steer::bias(kind, &plain, &steered, &counts)
    }
}

/// cosine of two vectors (0 when either is zero)
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (ab, aa, bb) = (kernels::dot(a, b), kernels::dot(a, a), kernels::dot(b, b));
    if aa == 0.0 || bb == 0.0 {
        0.0
    } else {
        ab / (aa.sqrt() * bb.sqrt())
    }
}

#[cfg(test)]
mod tests;
