use super::probe::probe;
use super::*;

/// the device, or None (and a note) where there is none: tests skip, they do not fail
pub(crate) fn device() -> Option<&'static Gpu> {
    match shared() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping GPU test: {e}");
            None
        }
    }
}

#[test]
fn kernels_match_the_cpu() {
    let Some(g) = device() else { return };
    for prec in [Precision::Exact, Precision::Fast] {
        let rep = probe(g, prec);
        eprintln!("{}", rep.summary());
        assert!(rep.max_rel() < 1e-4, "{}", rep.summary());
    }
}

use super::net::{GpuDecoder, GpuNet};
use crate::classify::model::{Config, Decoder, Gains, GenSpec, HeadSpec, Mode, Net, Pooling};
use crate::classify::text;

/// equal bits where the probe found the device exact, else within `tol` relative
fn close(what: &str, got: &[f32], want: &[f32], exact: bool) {
    assert_eq!(got.len(), want.len(), "{what}: lengths");
    let c = probe::compare("model", got, want);
    if exact {
        assert_eq!(c.differ, 0, "{what}: {c:?}");
    } else {
        let scale = want.iter().fold(1e-3f32, |a, b| a.max(b.abs()));
        for (a, b) in got.iter().zip(want) {
            assert!((a - b).abs() <= 1e-3 * scale, "{what}: {a} vs {b}");
        }
    }
}

fn configs() -> Vec<(&'static str, Config)> {
    let base = Config {
        d_model: 48,
        head_dim: 12,
        heads: 4,
        layers: 2,
        d_ff: 80,
        max_len: 40,
        stride: 19,
        pooling: Pooling::Mean,
        ..Config::tiny().gematria(false)
    };
    vec![
        (
            "causal",
            Config {
                causal: true,
                ..base.clone()
            },
        ),
        (
            "causal gqa qk-norm bias",
            Config {
                causal: true,
                kv_heads: 2,
                qk_norm: true,
                qkv_bias: true,
                ..base.clone().gematria(true)
            },
        ),
        (
            "encoder",
            Config {
                causal: false,
                pooling: Pooling::Attention,
                qk_norm: true,
                ..base.clone()
            },
        ),
        (
            "tiny",
            Config {
                causal: true,
                pooling: Pooling::Mean,
                ..Config::tiny()
            },
        ),
    ]
}

fn exact_device(g: &Gpu, prec: Precision) -> bool {
    probe(g, prec).exact()
}

#[test]
fn trunk_and_heads_match_the_cpu() {
    let Some(g) = device() else { return };
    let text = b"The quick brown fox, 1234 jumps over the lazy dog again and again.";
    for prec in [Precision::Exact, Precision::Fast] {
        let exact = exact_device(g, prec);
        for (name, cfg) in configs() {
            let generator = cfg.causal.then(|| GenSpec::new("next"));
            let heads = [HeadSpec::single("c", &["a", "b", "c"])];
            let mut net = Net::with_gen(cfg.clone(), &heads, generator, 3).unwrap();
            if cfg.qkv_bias {
                for b in &mut net.blocks {
                    b.bqkv
                        .iter_mut()
                        .enumerate()
                        .for_each(|(i, v)| *v = (i % 7) as f32 * 0.01);
                }
            }
            // zero-initialised output projections would hide the blocks
            for b in &mut net.blocks {
                b.wo.iter_mut()
                    .enumerate()
                    .for_each(|(i, v)| *v += ((i % 13) as f32 - 6.0) * 0.003);
                b.wd.iter_mut()
                    .enumerate()
                    .for_each(|(i, v)| *v += ((i % 11) as f32 - 5.0) * 0.003);
            }
            let gn = GpuNet::new(g, &net, prec).unwrap();
            let gains = Gains::default();
            for len in [3, 20, text.len().min(cfg.room())] {
                let w = text::window_with(&text[..len], cfg.causal, !cfg.causal);
                let tr = net.forward(&w, &[], &gains, Mode::EVAL, None);
                let (_, gl) = gn.forward(&net, &w, &gains, true);
                if cfg.causal {
                    close(
                        &format!("{name} {prec:?} gen {len}"),
                        &gl.unwrap(),
                        &net.gen_logits(&tr),
                        exact,
                    );
                }
                let hl = gn.head_logits(&net, &w, &[], &gains);
                close(
                    &format!("{name} {prec:?} head {len}"),
                    &hl[0],
                    &net.logits(&tr, 0),
                    exact,
                );
            }
        }
    }
}

#[test]
fn decoder_matches_the_cpu() {
    let Some(g) = device() else { return };
    for prec in [Precision::Exact, Precision::Fast] {
        let exact = exact_device(g, prec);
        for (name, cfg) in configs().into_iter().filter(|c| c.1.causal) {
            let net = Net::with_gen(cfg.clone(), &[], Some(GenSpec::new("next")), 5).unwrap();
            let gn = GpuNet::new(g, &net, prec).unwrap();
            let mut cpu = Decoder::new(&net, Gains::default(), b"ab");
            let mut gpu = GpuDecoder::new(&gn, &net, Gains::default(), b"ab");
            close(&format!("{name} prompt"), gpu.logits(), cpu.logits(), exact);
            // past max_len both restart from the last half window
            for i in 0..cfg.max_len + 5 {
                let b = b"hello, world "[i % 13];
                cpu.push(b);
                gpu.push(b);
                close(&format!("{name} {prec:?} step {i}"), gpu.logits(), cpu.logits(), exact);
            }
            assert_eq!(gpu.tokens(), cpu.tokens());
        }
    }
}

#[test]
fn training_matches_the_cpu() {
    use crate::classify::device::Device;
    use crate::classify::train::{self, Example, Target, TrainOpts};
    use crate::classify::{Classifier, Flow};
    let Some(g) = device() else { return };
    let exact = exact_device(g, Precision::Exact);
    for (name, cfg) in configs() {
        let generator = cfg.causal.then(|| GenSpec::new("next"));
        let heads = [HeadSpec::single("c", &["a", "b"])];
        let cfg = Config { dropout: 0.1, ..cfg };
        let fresh = Classifier::with_gen(cfg.clone(), &heads, generator, 9).unwrap();
        let words = [
            "alpha beta",
            "gamma, delta",
            "epsilon zeta eta theta iota kappa lambda mu",
        ];
        let data: Vec<Example> = (0..12)
            .map(|i| {
                let ex = Example::class(&format!("{} {i}", words[i % 3]), i % 2);
                if cfg.causal {
                    ex.and(0, Target::Next { from: 0 })
                } else {
                    ex
                }
            })
            .collect();
        let all: Vec<usize> = (0..data.len()).collect();
        let run = |device: Device| {
            let mut c = fresh.clone();
            let o = TrainOpts {
                epochs: 2,
                batch: 4,
                lr: 3e-3,
                val_fraction: 0.0,
                threads: 2,
                device,
                ..TrainOpts::default()
            };
            train::fit(&mut c, &data, &all, &[], &o, |_| Flow::Go).unwrap();
            c
        };
        let cpu = run(Device::Cpu);
        let gpu = run(Device::Gpu);
        let flat = |c: &Classifier| -> Vec<f32> { c.net.params().into_iter().flat_map(|(_, t)| t.clone()).collect() };
        close(&format!("{name} trained weights"), &flat(&gpu), &flat(&cpu), exact);
        assert_ne!(flat(&cpu), flat(&fresh), "{name}: training moved the weights");
    }
}
