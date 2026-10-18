//! Tokens per second on the CPU and on the GPU, for the spec presets, the size the
//! machine profile picks for an "auto" trunk, and a ladder of larger trunks that only
//! stops where the device runs out of memory or a window takes longer than the time
//! limit. Meant for the user's own hardware:
//!
//! ```text
//! cargo run --release -p anend-core --features gpu --example gpu_bench [seconds-per-case] [max-window-seconds]
//! ```
//!
//! Prints the device, the exactness probe, then per case: weights, window length,
//! forward tokens/s (CPU on every core, GPU exact and fast), whether the GPU's
//! generative logits equal the CPU's bit for bit, decoding tokens/s with the KV cache,
//! and training tokens/s (forward + backward) where the device trains.

use std::time::{Duration, Instant};

use anend_core::classify::bench::{self, LADDER, Task};
use anend_core::classify::gpu::{self, Precision, net::GpuDecoder, net::GpuNet};
use anend_core::classify::model::{Config, Decoder, Gains, GenSpec, Mode, Net, Pooling};
use anend_core::classify::{Pool, pool, text};

fn rate(per_case: Duration, mut f: impl FnMut() -> usize) -> f64 {
    f(); // warm-up (pipelines compile, buffers settle)
    let t = Instant::now();
    let (mut tokens, mut runs) = (0, 0);
    while runs == 0 || t.elapsed() < per_case {
        tokens += f();
        runs += 1;
    }
    tokens as f64 / t.elapsed().as_secs_f64()
}

fn sized(d: usize, layers: usize, heads: usize, ff: usize, max_len: usize) -> Config {
    Config {
        d_model: d,
        head_dim: d / heads,
        heads,
        layers,
        d_ff: ff,
        max_len,
        stride: max_len / 2,
        causal: true,
        pooling: Pooling::Mean,
        dropout: 0.0,
        ..Config::small().gematria(false)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let per_case = Duration::from_secs_f64(args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2.0));
    let max_window = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let g = match gpu::shared() {
        Ok(g) => g,
        Err(e) => {
            println!("no GPU: {e}");
            return;
        }
    };
    println!("device: {}", g.describe());
    println!("largest buffer: {} M floats", g.max_floats() / 1_000_000);
    for prec in [Precision::Exact, Precision::Fast] {
        print!("{}", gpu::probe::probe(g, prec).summary());
    }
    let threads = pool::available();
    let cpu_pool = Pool::new(threads);
    println!(
        "CPU: {threads} threads, {} kernels",
        anend_core::classify::kernels::simd_name()
    );

    let profile = bench::load_or_measure(
        &std::env::temp_dir().join("anend-machine-profile.json"),
        Duration::from_secs(2),
    );
    let auto = bench::suggest(
        &profile,
        &Task {
            examples: 2000,
            epochs: 20,
            max_len: 256,
            seconds: 600.0,
        },
    );
    let mut cases: Vec<(String, Config)> = vec![
        (
            "small".into(),
            Config {
                causal: true,
                ..Config::small().gematria(false)
            },
        ),
        (
            "base".into(),
            Config {
                causal: true,
                ..Config::base().gematria(false)
            },
        ),
        (
            format!("auto (d {} × {} layers)", auto.d_model, auto.layers),
            Config {
                causal: true,
                ..auto.gematria(false)
            },
        ),
    ];
    for &(d, l, h, ff) in &LADDER {
        cases.push((format!("ladder d {d}"), sized(d, l, h, ff, 256)));
    }
    for &(d, l, h, ff) in &[
        (384, 8, 6, 1024),
        (512, 8, 8, 1408),
        (768, 12, 12, 2048),
        (1024, 16, 16, 2816),
        (1536, 24, 16, 4096),
        (2048, 24, 16, 5632),
        (3072, 32, 24, 8192),
        (4096, 32, 32, 11008),
    ] {
        cases.push((format!("large d {d} × {l}"), sized(d, l, h, ff, 512)));
    }
    let text: Vec<u8> = (0..20_000)
        .map(|i| b"the quick brown fox jumps over a lazy dog. "[i % 43])
        .collect();
    println!(
        "\n{:<28} {:>9} {:>5} {:>10} {:>10} {:>10} {:>6} {:>9} {:>9}",
        "case", "weights", "len", "cpu tok/s", "gpu exact", "gpu fast", "same", "cpu dec/s", "gpu dec/s"
    );
    let mut cpu_ok = true;
    for (name, cfg) in cases {
        let weights_est = cfg.layers * (cfg.d_model * (3 * cfg.d_model + cfg.d_model + 3 * cfg.d_ff));
        if weights_est * 4 * 3 > g.max_floats() * 4 * 8 {
            println!("{name:<28} skipped: larger than the device's buffers");
            continue;
        }
        let net = match Net::with_gen(cfg.clone(), &[], Some(GenSpec::new("next")), 1) {
            Ok(n) => n,
            Err(e) => {
                println!("{name:<28} {e}");
                continue;
            }
        };
        let n = cfg.max_len;
        let w = text::window_with(&text[..cfg.room()], true, false);
        let gains = Gains::default();
        let cpu_t = if cpu_ok {
            let t = Instant::now();
            let tr = net.forward(&w, &[], &gains, Mode::EVAL, Some(&cpu_pool));
            drop(net.gen_logits(&tr));
            let one = t.elapsed().as_secs_f64();
            if one > max_window {
                cpu_ok = false;
                f64::NAN
            } else {
                rate(per_case, || {
                    let tr = net.forward(&w, &[], &gains, Mode::EVAL, Some(&cpu_pool));
                    drop(net.gen_logits(&tr));
                    n
                })
            }
        } else {
            f64::NAN
        };
        let mut gpu_t = [f64::NAN; 2];
        let mut same = "-";
        let mut gdec = f64::NAN;
        for (i, prec) in [Precision::Exact, Precision::Fast].into_iter().enumerate() {
            let gn = match GpuNet::new(g, &net, prec) {
                Ok(gn) => gn,
                Err(e) => {
                    println!("{name:<28} {e}");
                    break;
                }
            };
            let t = Instant::now();
            let (_, gl) = gn.forward(&net, &w, &gains, true);
            if t.elapsed().as_secs_f64() > max_window {
                break;
            }
            if i == 0 && cpu_t.is_finite() {
                let tr = net.forward(&w, &[], &gains, Mode::EVAL, Some(&cpu_pool));
                same = if gl.as_deref() == Some(&net.gen_logits(&tr)[..]) {
                    "yes"
                } else {
                    "no"
                };
            }
            gpu_t[i] = rate(per_case, || {
                drop(gn.forward(&net, &w, &gains, true));
                n
            });
            if i == 1 {
                gdec = rate(per_case, || {
                    let mut dec = GpuDecoder::new(&gn, &net, gains, b"the ");
                    for &b in &text[..32] {
                        dec.push(b);
                    }
                    36
                });
            }
        }
        let cdec = if cpu_t.is_finite() {
            rate(per_case, || {
                let mut dec = Decoder::new(&net, gains, b"the ");
                for &b in &text[..32] {
                    dec.push(b);
                }
                36
            })
        } else {
            f64::NAN
        };
        println!(
            "{name:<28} {:>8.1}M {n:>5} {cpu_t:>10.0} {:>10.0} {:>10.0} {same:>6} {cdec:>9.0} {gdec:>9.0}",
            net.weight_count() as f64 / 1e6,
            gpu_t[0],
            gpu_t[1],
        );
        if gpu_t[1].is_nan() && !cpu_t.is_finite() {
            println!("(stopping: windows take longer than {max_window} s on both)");
            break;
        }
    }
}
