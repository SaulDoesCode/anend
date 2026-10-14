//! Steering: the player's knobs on a trained classifier, which training never touches.
//! Per head a logit bias per class, a temperature and a decision threshold; for the
//! whole classifier a gain per input channel. Applied at inference only, with a bias
//! metric that says how far the knobs move predictions from the model as trained.

use serde::{Deserialize, Serialize};

use super::math;
use super::model::{Gains, Head, HeadKind};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadSteer {
    /// added to each class's logit after the temperature
    pub bias: Vec<f32>,
    /// logits are divided by it: above 1 softens, below 1 sharpens
    pub temperature: f32,
    /// the probability at which a text counts as a member (of the top class, or of each
    /// label for multi-label heads)
    pub threshold: f32,
}

impl HeadSteer {
    pub fn neutral(width: usize) -> HeadSteer {
        HeadSteer {
            bias: vec![0.0; width],
            temperature: 1.0,
            threshold: 0.5,
        }
    }

    pub fn is_neutral(&self) -> bool {
        self.bias.iter().all(|&b| b == 0.0) && self.temperature == 1.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Steer {
    /// multiplies the trained channel gains
    pub gains: Gains,
    pub heads: Vec<HeadSteer>,
}

impl Steer {
    pub fn neutral(heads: &[Head]) -> Steer {
        Steer {
            gains: Gains::default(),
            heads: heads.iter().map(|h| HeadSteer::neutral(h.width())).collect(),
        }
    }
}

/// a head's output on a text: what the model said, and what the steering makes of it
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Output {
    /// window-averaged logits as trained (with the steered input gains)
    pub logits: Vec<f32>,
    /// probabilities (class and multi-label heads) or values (regression), steered
    pub probs: Vec<f32>,
    /// the most likely class
    pub top: usize,
    /// classes this text is a member of under the threshold
    pub members: Vec<usize>,
}

/// the steered reading of a head's logits: `l/T + bias`, then softmax, sigmoids or as is
pub fn apply(kind: HeadKind, s: &HeadSteer, logits: &[f32]) -> Output {
    let z: Vec<f32> = logits.iter().zip(&s.bias).map(|(l, b)| l / s.temperature + b).collect();
    let probs = match kind {
        HeadKind::Single => {
            let mut p = z;
            super::kernels::softmax_rows(&mut p, logits.len());
            p
        }
        HeadKind::Multi => z.iter().map(|&v| math::sigmoid(v)).collect(),
        HeadKind::Regress => logits.iter().zip(&s.bias).map(|(l, b)| l + b).collect(),
    };
    let mut top = 0;
    for (i, &p) in probs.iter().enumerate() {
        if p > probs[top] {
            top = i;
        }
    }
    let members = match kind {
        HeadKind::Single => {
            if probs[top] >= s.threshold {
                vec![top]
            } else {
                vec![]
            }
        }
        HeadKind::Multi => (0..probs.len()).filter(|&i| probs[i] >= s.threshold).collect(),
        HeadKind::Regress => vec![],
    };
    Output {
        logits: logits.to_vec(),
        probs,
        top,
        members,
    }
}

/// d(loss)/d(logits) from d(loss)/d(steered output), for gradients through a stack
pub fn back(kind: HeadKind, s: &HeadSteer, out: &Output, dprobs: &[f32]) -> Vec<f32> {
    match kind {
        HeadKind::Single => {
            let c = super::kernels::dot(&out.probs, dprobs);
            out.probs
                .iter()
                .zip(dprobs)
                .map(|(p, d)| p * (d - c) / s.temperature)
                .collect()
        }
        HeadKind::Multi => out
            .probs
            .iter()
            .zip(dprobs)
            .map(|(p, d)| d * (p * (1.0 - p)) / s.temperature)
            .collect(),
        HeadKind::Regress => dprobs.to_vec(),
    }
}

/// how far steering moves a head's predictions from the unsteered model, and how skewed
/// the classes were in training
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BiasReport {
    pub texts: usize,
    /// mean KL(steered ‖ unsteered) in nats (class heads; summed over labels for multi)
    pub mean_kl: f64,
    pub max_kl: f64,
    /// share of texts whose decision changed (top class, or any label's membership)
    pub flipped: f64,
    /// mean change in each class's probability
    pub shift: Vec<f64>,
    /// the training class shares
    pub prior: Vec<f64>,
    /// KL(prior ‖ uniform): 0 for balanced training data
    pub prior_skew: f64,
    /// largest over smallest class count (∞ when a class had no examples)
    pub imbalance: f64,
}

/// compares the steered and unsteered outputs of one head over the same texts
pub fn bias(kind: HeadKind, plain: &[Output], steered: &[Output], counts: &[f32]) -> BiasReport {
    let n = plain.len();
    let k = plain.first().map_or(counts.len(), |o| o.probs.len());
    let mut r = BiasReport {
        texts: n,
        shift: vec![0.0; k],
        ..BiasReport::default()
    };
    for (p, q) in plain.iter().zip(steered) {
        let kl = match kind {
            HeadKind::Single => kl(&q.probs, &p.probs),
            HeadKind::Multi => q
                .probs
                .iter()
                .zip(&p.probs)
                .map(|(&a, &b)| kl(&[a, 1.0 - a], &[b, 1.0 - b]))
                .sum(),
            HeadKind::Regress => 0.0,
        };
        r.mean_kl += kl;
        r.max_kl = r.max_kl.max(kl);
        let changed = match kind {
            HeadKind::Single => p.top != q.top || p.members != q.members,
            _ => p.members != q.members,
        };
        r.flipped += changed as u8 as f64;
        for (s, (a, b)) in r.shift.iter_mut().zip(q.probs.iter().zip(&p.probs)) {
            *s += (*a - *b) as f64;
        }
    }
    let nf = n.max(1) as f64;
    r.mean_kl /= nf;
    r.flipped /= nf;
    r.shift.iter_mut().for_each(|s| *s /= nf);
    let total: f64 = counts.iter().map(|&c| c as f64).sum();
    if total > 0.0 {
        r.prior = counts.iter().map(|&c| c as f64 / total).collect();
        let u = 1.0 / counts.len() as f64;
        r.prior_skew = r
            .prior
            .iter()
            .filter(|&&p| p > 0.0)
            .map(|&p| p * math::ln64(p / u))
            .sum();
        let (lo, hi) = counts
            .iter()
            .fold((f32::INFINITY, 0f32), |(l, h), &c| (l.min(c), h.max(c)));
        r.imbalance = if lo > 0.0 { (hi / lo) as f64 } else { f64::INFINITY };
    }
    r
}

/// KL(q ‖ p) in nats
fn kl(q: &[f32], p: &[f32]) -> f64 {
    q.iter()
        .zip(p)
        .filter(|(a, _)| **a > 0.0)
        .map(|(&a, &b)| a as f64 * math::ln64((a as f64) / (b as f64).max(1e-12)))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steering_moves_and_is_measured() {
        let s0 = HeadSteer::neutral(3);
        let l = [2.0, 1.0, 0.0];
        let plain = apply(HeadKind::Single, &s0, &l);
        assert_eq!(plain.top, 0);
        let mut s = s0.clone();
        s.bias = vec![0.0, 0.0, 3.0];
        let st = apply(HeadKind::Single, &s, &l);
        assert_eq!(st.top, 2);
        s.temperature = 100.0;
        s.bias = vec![0.0; 3];
        let soft = apply(HeadKind::Single, &s, &l);
        assert!(soft.probs[0] < 0.35 && soft.members.is_empty());
        let r = bias(
            HeadKind::Single,
            &[plain.clone(), plain.clone()],
            &[st, plain],
            &[10.0, 10.0, 80.0],
        );
        assert_eq!(r.flipped, 0.5);
        assert!(r.mean_kl > 0.0 && r.shift[2] > 0.0 && r.shift[0] < 0.0);
        assert!(r.prior_skew > 0.0 && r.imbalance == 8.0);
    }
}
