//! Proof of concept: which author wrote an arbitrary selection of text.
//!
//! Trains a classifier on passages from six public domain authors (core/corpus/authors).
//! Of each author's ten excerpt regions, two are held out for testing and one more for
//! early stopping; the test is arbitrary 300–600 character selections from the two
//! unseen regions.
//!
//! `cargo run --release -p anend-core --example authors [epochs] [save-path]`

use std::time::Instant;

use anend_core::classify::{
    Classifier, Config, Example, Flow, HeadSpec, Pool, TrainOpts, cosine, kernels, math::Rng, pool, train,
};

const AUTHORS: [&str; 6] = ["austen", "chesterton", "melville", "milton", "shakespeare", "whitman"];
const HELD_OUT: [usize; 2] = [2, 7];
const VALIDATION: usize = 5;

/// a selection of `len` bytes or so starting near `at`, on word boundaries
fn selection(text: &str, at: usize, len: usize) -> String {
    let b = text.as_bytes();
    let mut s = at.min(b.len().saturating_sub(len));
    while s > 0 && !b[s - 1].is_ascii_whitespace() {
        s -= 1;
    }
    let mut e = (s + len).min(b.len());
    while e < b.len() && !b[e].is_ascii_whitespace() {
        e += 1;
    }
    text[s..e].trim().to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let epochs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(16);
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/corpus/authors");
    let mut train_data = Vec::new();
    let mut val_idx = Vec::new();
    let mut test = Vec::new();
    let mut rng = Rng::new(2026);
    for (c, a) in AUTHORS.iter().enumerate() {
        let text = std::fs::read_to_string(format!("{dir}/{a}.txt")).expect("corpus file");
        for (r, region) in text.split("\n* * *\n").enumerate() {
            let region = region.trim();
            if HELD_OUT.contains(&r) {
                for _ in 0..20 {
                    let len = 300 + rng.below(301);
                    let at = rng.below(region.len().saturating_sub(len).max(1));
                    test.push(Example::class(&selection(region, at, len), c));
                }
            } else {
                // overlapping passages, each a little longer than a window
                let mut at = 0;
                while at + 200 < region.len() {
                    if r == VALIDATION {
                        val_idx.push(train_data.len());
                    }
                    train_data.push(Example::class(&selection(region, at, 500), c));
                    at += 250;
                }
            }
        }
    }
    let cfg = Config::small();
    let heads = [HeadSpec::single("author", &AUTHORS)];
    let mut clf = Classifier::new(cfg.clone(), &heads, 7).unwrap();
    let threads = pool::available();
    println!(
        "{} training and {} validation passages, {} held-out selections of 300-600 characters; {} weights, {} threads, {} kernels",
        train_data.len() - val_idx.len(),
        val_idx.len(),
        test.len(),
        clf.net.weight_count(),
        threads,
        kernels::simd_name()
    );
    let o = TrainOpts {
        epochs,
        batch: 16,
        lr: 6e-4,
        warmup: 0.05,
        weight_decay: 0.1,
        ema: 0.99,
        patience: 6,
        seed: 11,
        ..TrainOpts::default()
    };
    let started = Instant::now();
    let train_idx: Vec<usize> = (0..train_data.len()).filter(|i| !val_idx.contains(i)).collect();
    let rep = train::fit(&mut clf, &train_data, &train_idx, &val_idx, &o, |p| {
        if let Some(e) = p.eval {
            println!(
                "  epoch {:>2} step {:>4}/{}: val loss {:.3}, accuracy {:.3}, {:.0} tokens/s",
                p.epoch + 1,
                p.step,
                p.steps,
                e.loss,
                e.accuracy(),
                p.tokens_per_sec
            );
        }
        Flow::Go
    })
    .unwrap();
    println!(
        "trained {} steps in {:.1}s ({:.0} tokens/s); kept step {} (val loss {:.3}){}",
        rep.steps,
        started.elapsed().as_secs_f64(),
        rep.tokens_per_sec,
        rep.best_step,
        rep.best_loss.unwrap_or(f64::NAN),
        if rep.stopped_early { ", stopped early" } else { "" }
    );

    let pool = Pool::new(threads);
    let t = Instant::now();
    let idx: Vec<usize> = (0..test.len()).collect();
    let ev = train::evaluate(&clf, &test, &idx, &pool);
    let m = ev.heads[0].as_ref().unwrap();
    println!(
        "\nheld-out selections: accuracy {:.3}, macro-F1 {:.3}, log-loss {:.3}, ECE {:.3} ({:.2}s to classify {})",
        m.accuracy,
        m.macro_f1,
        m.log_loss,
        m.ece,
        t.elapsed().as_secs_f64(),
        test.len()
    );
    for (a, s) in AUTHORS.iter().zip(&m.classes) {
        println!(
            "  {a:>12}: precision {:.2} recall {:.2} F1 {:.2} ({})",
            s.precision, s.recall, s.f1, s.support
        );
    }
    let names: Vec<String> = AUTHORS.iter().map(|s| s.to_string()).collect();
    println!("\nconfusion (rows: true author)\n{}", m.confusion_table(&names));

    println!("some selections:");
    for ex in test.iter().step_by(23).take(6) {
        let out = &clf.predict(&ex.text, &[])[0];
        let mut ranked: Vec<(usize, f32)> = out.probs.iter().copied().enumerate().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        let shown: String = ex.text.chars().take(90).collect::<String>().replace('\n', " ");
        let truth = match ex.target {
            anend_core::classify::Target::Class(c) => AUTHORS[c],
            _ => "?",
        };
        println!("  \"{shown}...\" ({truth})");
        let top: Vec<String> = ranked
            .iter()
            .take(3)
            .map(|(c, p)| format!("{} {:.2}", AUTHORS[*c], p))
            .collect();
        println!("      -> {}", top.join(", "));
    }

    // the trunk as an embedding: same-author selections should sit closer
    let texts: Vec<&str> = test.iter().map(|e| e.text.as_str()).collect();
    let reps = clf.represent_many(&texts, &pool);
    let label = |i: usize| match test[i].target {
        anend_core::classify::Target::Class(c) => c,
        _ => usize::MAX,
    };
    let (mut same, mut ns, mut diff, mut nd, mut nn_right) = (0f64, 0, 0f64, 0, 0);
    for i in 0..reps.len() {
        let mut best = (f32::NEG_INFINITY, 0);
        for j in 0..reps.len() {
            if i == j {
                continue;
            }
            let c = cosine(&reps[i], &reps[j]);
            if label(i) == label(j) {
                same += c as f64;
                ns += 1;
            } else {
                diff += c as f64;
                nd += 1;
            }
            if c > best.0 {
                best = (c, j);
            }
        }
        nn_right += (label(best.1) == label(i)) as usize;
    }
    println!(
        "\nrepresent(): mean cosine same author {:.3}, different authors {:.3}; nearest neighbour is by the same author {:.1}% of the time",
        same / ns as f64,
        diff / nd as f64,
        100.0 * nn_right as f64 / reps.len() as f64
    );
    if let Some(path) = args.get(2) {
        clf.save(std::path::Path::new(path)).unwrap();
        println!("saved to {path}");
    }
}
