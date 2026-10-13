//! Training throughput of the classifier presets, and where the time goes.
//!
//! `cargo run --release -p anend-core --example classify_bench [small|base|tiny] [steps] [simd]`
//! (`simd` skips the portable run), or `... classify_bench gemm` for the product kernels alone

use std::time::Instant;

use anend_core::classify::{Classifier, Config, Example, Flow, HeadSpec, TrainOpts, kernels, train};

fn corpus() -> Vec<String> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/corpus/authors");
    let mut texts = Vec::new();
    for a in ["austen", "chesterton", "melville", "milton", "shakespeare", "whitman"] {
        let t = std::fs::read_to_string(format!("{dir}/{a}.txt")).expect("corpus");
        texts.extend(
            t.as_bytes()
                .chunks(1200)
                .map(|c| String::from_utf8_lossy(c).into_owned()),
        );
    }
    texts
}

/// GFLOP/s of the product shapes a `small` window uses
fn gemms() {
    use kernels::{gemm, rows, transposed};
    let shapes = [
        ("x·Wqkv 256x128x384", 256, 128, 384, false),
        ("x·Wg 256x128x352", 256, 128, 352, false),
        ("act·Wd 256x352x128", 256, 352, 128, false),
        ("q·kT 256x32x256", 256, 32, 256, false),
        ("P·v 256x256x32", 256, 256, 32, false),
        ("dW xT·dy 128x256x384", 128, 256, 384, true),
        ("dW actT·dd 352x256x128", 352, 256, 128, true),
    ];
    for (name, n, k, m, t) in shapes {
        let a: Vec<f32> = (0..n * k).map(|i| (i % 7) as f32 * 0.1).collect();
        let b: Vec<f32> = (0..k * m).map(|i| (i % 5) as f32 * 0.1).collect();
        let mut y = vec![0f32; n * m];
        let reps = (2e9 / (2 * n * k * m) as f64).max(3.0) as usize;
        let av = if t { transposed(&a, n) } else { rows(&a, k) };
        let start = Instant::now();
        for _ in 0..reps {
            gemm(&mut y, av, &b, n, k, m, None);
        }
        let s = start.elapsed().as_secs_f64();
        println!("  {name:>24}: {:6.1} GFLOP/s", (2 * n * k * m * reps) as f64 / s / 1e9);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "gemm") {
        for portable in [true, false] {
            kernels::force_portable(portable);
            println!("{}", kernels::simd_name());
            gemms();
        }
        return;
    }
    let preset = args.get(1).map_or("small", |s| s.as_str());
    let steps: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);
    let cfg = match preset {
        "base" => Config::base(),
        "tiny" => Config::tiny(),
        _ => Config::small(),
    };
    let classes = ["a", "b", "c", "d", "e", "f"];
    let data: Vec<Example> = corpus()
        .iter()
        .enumerate()
        .map(|(i, t)| Example::class(t, i % 6))
        .collect();
    let probe = Classifier::new(cfg.clone(), &[HeadSpec::single("x", &classes)], 1).unwrap();
    println!(
        "{preset}: d={} heads={}x{} layers={} ff={} max_len={} weights={} simd={}",
        cfg.d_model,
        cfg.heads,
        cfg.head_dim,
        cfg.layers,
        cfg.d_ff,
        cfg.max_len,
        probe.net.weight_count(),
        kernels::simd_name()
    );
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let mut runs = vec![(true, 1), (false, 1)];
    if threads > 1 {
        runs.push((false, threads));
    }
    // "simd": skip the portable run
    if args.get(3).is_some_and(|a| a == "simd") {
        runs.remove(0);
    }
    for (portable, th) in runs {
        kernels::force_portable(portable);
        let mut clf = Classifier::new(cfg.clone(), &[HeadSpec::single("x", &classes)], 1).unwrap();
        let o = TrainOpts {
            epochs: 1,
            batch: 16,
            max_steps: steps,
            val_fraction: 0.0,
            threads: th,
            ..TrainOpts::default()
        };
        kernels::profile(true);
        let t = Instant::now();
        let rep = train::train(&mut clf, &data, &o, |_| Flow::Go).unwrap();
        let wall = t.elapsed().as_secs_f64();
        kernels::profile(false);
        println!(
            "  {:>8} x{th}: {:>8.0} tokens/s  ({} steps, {} tokens, {:.2}s)",
            if portable { "portable" } else { kernels::simd_name() },
            rep.tokens as f64 / wall,
            rep.steps,
            rep.tokens,
            wall
        );
        let busy = wall * th as f64;
        let mut known = 0.0;
        for (name, s, calls) in kernels::profile_report() {
            known += s;
            println!("      {name:>10}: {:5.1}%  ({calls} calls)", 100.0 * s / busy);
        }
        println!(
            "      {:>10}: {:5.1}%  (everything else, incl. idle threads)",
            "rest",
            100.0 * (busy - known) / busy
        );
    }
    kernels::force_portable(false);
}
