//! Generators shaped by a classifier: per author of core/corpus/authors, a small
//! byte-level generator learns to write like them; an author classifier is the critic.
//! For each author the example draws passages plainly, with the critic reranking, with
//! the critic guiding sampling, and after one reinforcement round on the critic's
//! scores, and reports how often the critic agrees the passage is that author's, with
//! timings.
//!
//! `cargo run --release -p anend-core --example generate [scale] [authors]`
//! `scale` multiplies training steps (default 1: a few minutes on 4 cores);
//! `authors` limits how many authors get a generator (default all 6).
//!
//! The sizes are chosen to finish in minutes on a 4-core CPU, a measured compute limit
//! (see `bench`), not a capability limit: larger generators write better and cost more.

use std::time::Instant;

use anend_core::classify::generate::{self, Constraints, Critic, GenOpts, Guidance, ReinforceOpts, Sampling, Want};
use anend_core::classify::{
    Classifier, Config, Example, Flow, GenSpec, HeadSpec, Pool, Pooling, TrainOpts, kernels, pool, train,
};

const AUTHORS: [&str; 6] = ["austen", "chesterton", "melville", "milton", "shakespeare", "whitman"];

fn passages(text: &str, len: usize, step: usize) -> Vec<String> {
    let mut v = Vec::new();
    for region in text.split("\n* * *\n") {
        let b = region.trim().as_bytes();
        let mut at = 0;
        while at + len <= b.len() {
            v.push(String::from_utf8_lossy(&b[at..at + len]).into_owned());
            at += step;
        }
    }
    v
}

fn agreement(critic: &Classifier, c: usize, cands: &[generate::Candidate]) -> f64 {
    let hits = cands
        .iter()
        .filter(|k| critic.predict(&k.text(), &[])[0].top == c)
        .count();
    hits as f64 / cands.len().max(1) as f64
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scale: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let n_authors: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(6).min(6);
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/corpus/authors");
    let texts: Vec<String> = AUTHORS
        .iter()
        .map(|a| std::fs::read_to_string(format!("{dir}/{a}.txt")).expect("corpus"))
        .collect();
    println!(
        "{} threads, {} kernels, scale {scale}",
        pool::available(),
        kernels::simd_name()
    );

    // the critic: an author classifier (no gematria features: a general transformer)
    let t0 = Instant::now();
    let mut data = Vec::new();
    for (c, t) in texts.iter().enumerate() {
        data.extend(passages(t, 200, 150).into_iter().map(|p| Example::class(&p, c)));
    }
    let ccfg = Config {
        d_model: 96,
        head_dim: 32,
        heads: 3,
        layers: 3,
        d_ff: 256,
        max_len: 128,
        stride: 63,
        dropout: 0.1,
        ..Config::small().gematria(false)
    };
    let mut critic = Classifier::new(ccfg, &[HeadSpec::single("author", &AUTHORS)], 7).unwrap();
    let o = TrainOpts {
        epochs: 6,
        batch: 16,
        lr: 2e-3,
        val_fraction: 0.1,
        max_steps: (900.0 * scale) as usize,
        ..TrainOpts::default()
    };
    let rep = train::train(&mut critic, &data, &o, |_| Flow::Go).unwrap();
    println!(
        "critic: {} weights, {} steps in {:.1}s, validation accuracy {:.1}%",
        critic.net.weight_count(),
        rep.steps,
        t0.elapsed().as_secs_f64(),
        100.0 * rep.eval.as_ref().map_or(0.0, |e| e.accuracy())
    );

    let gcfg = Config {
        d_model: 128,
        head_dim: 32,
        heads: 4,
        layers: 3,
        d_ff: 352,
        max_len: 128,
        stride: 63,
        dropout: 0.05,
        causal: true,
        pooling: Pooling::Mean,
        ..Config::small().gematria(false)
    };
    let mut totals = [0f64; 4];
    for (c, t) in texts.iter().enumerate().take(n_authors) {
        let t1 = Instant::now();
        let corpus: Vec<Example> = passages(t, 400, 200).iter().map(|p| Example::generate(p)).collect();
        let mut g = Classifier::with_gen(gcfg.clone(), &[], Some(GenSpec::new("next")), 11 + c as u64).unwrap();
        let o = TrainOpts {
            epochs: 30,
            batch: 16,
            lr: 2e-3,
            val_fraction: 0.05,
            patience: 0,
            label_smoothing: 0.0,
            max_steps: (500.0 * scale) as usize,
            ..TrainOpts::default()
        };
        let rep = train::train(&mut g, &corpus, &o, |_| Flow::Go).unwrap();
        let bpb = rep.eval.as_ref().and_then(|e| e.bits_per_byte).unwrap_or(f64::NAN);
        println!(
            "\n== {} == generator {} weights, {} steps in {:.1}s ({:.0} tokens/s), held-out {:.2} bits/byte",
            AUTHORS[c],
            g.net.weight_count(),
            rep.steps,
            t1.elapsed().as_secs_f64(),
            rep.tokens_per_sec,
            bpb
        );
        let critics = [Critic::new(&critic, 0, Want::Class(c), 1.0)];
        let opts = GenOpts {
            prompt: b"The ".to_vec(),
            sampling: Sampling {
                temperature: 0.8,
                top_p: 0.95,
                max_len: 120,
                seed: 5,
                ..Sampling::default()
            },
            candidates: 12,
            ..GenOpts::default()
        };
        let none = Constraints::default();
        let t2 = Instant::now();
        let plain = generate::draw(&g, &critics, &none, &opts).unwrap();
        let tp = t2.elapsed().as_secs_f64();
        let ranked = generate::generate(&g, &critics, &none, &opts).unwrap();
        let t3 = Instant::now();
        let guided = generate::draw(
            &g,
            &critics,
            &none,
            &GenOpts {
                guidance: Some(Guidance {
                    top_k: 4,
                    strength: 3.0,
                    every: 4,
                }),
                ..opts.clone()
            },
        )
        .unwrap();
        let tg = t3.elapsed().as_secs_f64();
        let t4 = Instant::now();
        let rw = generate::rewards(&plain, &opts.prompt, 4.0);
        let rr = generate::reinforce(
            &mut g,
            &rw,
            &corpus,
            &ReinforceOpts {
                steps: 12,
                lr: 5e-4,
                ..ReinforceOpts::default()
            },
        )
        .unwrap();
        let after = generate::draw(
            &g,
            &critics,
            &none,
            &GenOpts {
                sampling: Sampling {
                    seed: 6,
                    ..opts.sampling.clone()
                },
                ..opts.clone()
            },
        )
        .unwrap();
        let tr = t4.elapsed().as_secs_f64();
        let a = [
            agreement(&critic, c, &plain),
            agreement(&critic, c, &ranked[..3.min(ranked.len())]),
            agreement(&critic, c, &guided),
            agreement(&critic, c, &after),
        ];
        for (t, x) in totals.iter_mut().zip(a) {
            *t += x;
        }
        println!(
            "critic agrees: plain {:.0}% ({:.1}s), top 3 reranked {:.0}%, guided {:.0}% ({:.1}s), after reinforce {:.0}% ({:.1}s; good {:.2}->{:.2}, bad {:.2}->{:.2} nats/byte)",
            100.0 * a[0],
            tp,
            100.0 * a[1],
            100.0 * a[2],
            tg,
            100.0 * a[3],
            tr,
            rr.good_before,
            rr.good_after,
            rr.bad_before,
            rr.bad_after
        );
        println!("best:   The {}", ranked[0].text().replace('\n', " "));
        println!("plain:  The {}", plain[0].text().replace('\n', " "));
        println!("guided: The {}", guided[0].text().replace('\n', " "));
    }
    let n = n_authors as f64;
    println!(
        "\nmean agreement over {n_authors} authors: plain {:.0}%, reranked top 3 {:.0}%, guided {:.0}%, after reinforce {:.0}%",
        100.0 * totals[0] / n,
        100.0 * totals[1] / n,
        100.0 * totals[2] / n,
        100.0 * totals[3] / n
    );
    let _ = Pool::new(1);
}
