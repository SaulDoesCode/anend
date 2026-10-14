//! How well a head does on labelled texts: accuracy, macro-F1, per-class precision and
//! recall, the confusion matrix, log-loss and expected calibration error for class heads;
//! squared and absolute error and R² for regression heads. Sums run in example order in
//! f64, so the numbers are as deterministic as the predictions.

use serde::{Deserialize, Serialize};

use super::math::ln64;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClassStat {
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub support: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub n: usize,
    pub accuracy: f64,
    pub macro_f1: f64,
    pub classes: Vec<ClassStat>,
    /// `confusion[truth][predicted]`
    pub confusion: Vec<Vec<u32>>,
    /// mean −ln p(truth) (single label) or mean binary cross-entropy (multi label)
    pub log_loss: f64,
    /// expected calibration error over 15 equal-width confidence bins
    pub ece: f64,
    pub mse: f64,
    pub mae: f64,
    pub r2: f64,
}

fn f1(tp: f64, fp: f64, fneg: f64) -> (f64, f64, f64) {
    let p = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
    let r = if tp + fneg > 0.0 { tp / (tp + fneg) } else { 0.0 };
    let f = if p + r > 0.0 { 2.0 * p * r / (p + r) } else { 0.0 };
    (p, r, f)
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

/// a single-label head: each row of `probs` sums to 1, `truth` the class index
pub fn single(probs: &[Vec<f32>], truth: &[usize]) -> Metrics {
    let k = probs.first().map_or(0, |p| p.len());
    let n = probs.len();
    let mut confusion = vec![vec![0u32; k]; k];
    let mut log_loss = 0f64;
    const BINS: usize = 15;
    let mut bins = [(0f64, 0f64, 0u32); BINS];
    for (p, &t) in probs.iter().zip(truth) {
        let pred = argmax(p);
        confusion[t][pred] += 1;
        log_loss -= ln64(p[t].max(1e-7) as f64);
        let conf = p[pred] as f64;
        let b = ((conf * BINS as f64) as usize).min(BINS - 1);
        bins[b].0 += conf;
        bins[b].1 += (pred == t) as u8 as f64;
        bins[b].2 += 1;
    }
    let mut classes = Vec::with_capacity(k);
    let mut correct = 0u32;
    for (c, row) in confusion.iter().enumerate() {
        let tp = row[c] as f64;
        let fp = confusion.iter().map(|r| r[c]).sum::<u32>() as f64 - tp;
        let support = row.iter().sum::<u32>();
        let (precision, recall, f) = f1(tp, fp, support as f64 - tp);
        correct += row[c];
        classes.push(ClassStat {
            precision,
            recall,
            f1: f,
            support,
        });
    }
    let present: Vec<&ClassStat> = classes.iter().filter(|c| c.support > 0).collect();
    let nf = n.max(1) as f64;
    Metrics {
        n,
        accuracy: correct as f64 / nf,
        macro_f1: present.iter().map(|c| c.f1).sum::<f64>() / present.len().max(1) as f64,
        classes,
        confusion,
        log_loss: log_loss / nf,
        ece: bins
            .iter()
            .map(|&(c, a, m)| if m > 0 { (c - a).abs() } else { 0.0 })
            .sum::<f64>()
            / nf,
        ..Metrics::default()
    }
}

/// a multi-label head: `truth` rows of 0/1 per label, decided at `threshold`
pub fn multi(probs: &[Vec<f32>], truth: &[Vec<f32>], threshold: f32) -> Metrics {
    let k = probs.first().map_or(0, |p| p.len());
    let n = probs.len();
    let mut counts = vec![(0f64, 0f64, 0f64, 0u32); k];
    let (mut exact, mut ll) = (0u32, 0f64);
    for (p, t) in probs.iter().zip(truth) {
        let mut all = true;
        for c in 0..k {
            let (pred, real) = (p[c] >= threshold, t[c] >= 0.5);
            all &= pred == real;
            let e = &mut counts[c];
            match (pred, real) {
                (true, true) => e.0 += 1.0,
                (true, false) => e.1 += 1.0,
                (false, true) => e.2 += 1.0,
                _ => {}
            }
            e.3 += real as u32;
            let q = (p[c] as f64).clamp(1e-7, 1.0 - 1e-7);
            ll -= if real { ln64(q) } else { ln64(1.0 - q) };
        }
        exact += all as u32;
    }
    let classes: Vec<ClassStat> = counts
        .iter()
        .map(|&(tp, fp, fneg, support)| {
            let (precision, recall, f) = f1(tp, fp, fneg);
            ClassStat {
                precision,
                recall,
                f1: f,
                support,
            }
        })
        .collect();
    Metrics {
        n,
        accuracy: exact as f64 / n.max(1) as f64,
        macro_f1: classes.iter().map(|c| c.f1).sum::<f64>() / k.max(1) as f64,
        classes,
        log_loss: ll / (n * k).max(1) as f64,
        ..Metrics::default()
    }
}

/// a regression head
pub fn regress(preds: &[Vec<f32>], truth: &[Vec<f32>]) -> Metrics {
    let (mut se, mut ae, mut cnt, mut mean) = (0f64, 0f64, 0f64, 0f64);
    for t in truth.iter().flatten() {
        mean += *t as f64;
        cnt += 1.0;
    }
    mean /= cnt.max(1.0);
    let mut var = 0f64;
    for (p, t) in preds.iter().zip(truth) {
        for (&a, &b) in p.iter().zip(t) {
            let e = a as f64 - b as f64;
            se += e * e;
            ae += e.abs();
            var += (b as f64 - mean) * (b as f64 - mean);
        }
    }
    Metrics {
        n: preds.len(),
        mse: se / cnt.max(1.0),
        mae: ae / cnt.max(1.0),
        r2: if var > 0.0 { 1.0 - se / var } else { 0.0 },
        ..Metrics::default()
    }
}

impl Metrics {
    /// the confusion matrix as aligned text, rows the truth
    pub fn confusion_table(&self, names: &[String]) -> String {
        let w = names.iter().map(|s| s.len()).max().unwrap_or(4).max(5);
        let mut s = format!("{:>w$} |", "");
        for n in names {
            s += &format!(" {:>w$}", n);
        }
        s.push('\n');
        for (i, row) in self.confusion.iter().enumerate() {
            s += &format!("{:>w$} |", names.get(i).map_or("?", |n| n.as_str()));
            for c in row {
                s += &format!(" {:>w$}", c);
            }
            s.push('\n');
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_label() {
        let p = vec![vec![0.9, 0.1], vec![0.2, 0.8], vec![0.6, 0.4], vec![0.3, 0.7]];
        let m = single(&p, &[0, 1, 1, 1]);
        assert_eq!(m.confusion, vec![vec![1, 0], vec![1, 2]]);
        assert_eq!(m.accuracy, 0.75);
        assert_eq!(m.classes[0].precision, 0.5);
        assert!((m.classes[1].recall - 2.0 / 3.0).abs() < 1e-12);
        assert!(m.log_loss > 0.0 && m.ece >= 0.0);
        let r = regress(&[vec![1.0], vec![2.0]], &[vec![1.0], vec![3.0]]);
        assert_eq!((r.mse, r.mae), (0.5, 0.5));
        let mm = multi(&[vec![0.9, 0.2]], &[vec![1.0, 0.0]], 0.5);
        assert_eq!(mm.accuracy, 1.0);
    }
}
