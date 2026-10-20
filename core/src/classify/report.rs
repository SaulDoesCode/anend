//! `gpu.report`: what this machine's GPU is, whether it computes the CPU's bits, and how
//! fast it trains and reads texts next to the CPU, as one JSON value the user writes to
//! a file and pastes back. Everything in it is measured on the machine it runs on; a
//! build without the `gpu` feature, or a machine without an adapter, still reports the
//! CPU's side and says why the GPU's is missing.

use std::time::Instant;

use serde_json::{Value, json};

use super::device::{self, Device, Runner};
use super::pool::{self, Pool};
use super::train::{self, Example, TrainOpts};
use super::{Classifier, Flow, HeadSpec, kernels};

/// how much the report measures
#[derive(Clone, Debug)]
pub struct Opts {
    /// presets to time (see `classifiers::preset`)
    pub sizes: Vec<String>,
    /// texts to predict on each device
    pub texts: usize,
    /// optimizer steps in the fixed training run
    pub steps: usize,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            sizes: vec!["compact".into(), "small".into()],
            texts: 10_000,
            steps: 40,
        }
    }
}

/// a text for the fixed workloads: deterministic, 20 to about 400 bytes, words from one
/// of two vocabularies so a classifier has something to learn
fn text(i: usize) -> (String, usize) {
    const A: [&str; 8] = ["river", "stone", "moss", "fern", "heron", "reed", "clay", "dusk"];
    const B: [&str; 8] = ["copper", "gear", "valve", "rivet", "piston", "wire", "bolt", "lamp"];
    let class = i % 2;
    let words = if class == 0 { &A } else { &B };
    let mut h = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let len = 20 + (h % 380) as usize;
    let mut s = String::new();
    while s.len() < len {
        h ^= h << 13;
        h ^= h >> 7;
        h ^= h << 17;
        s.push_str(words[(h % 8) as usize]);
        s.push(if h.is_multiple_of(11) { '\n' } else { ' ' });
    }
    (s, class)
}

fn data(n: usize) -> Vec<Example> {
    (0..n)
        .map(|i| {
            let (t, c) = text(i);
            Example::class(&t, c)
        })
        .collect()
}

fn opts(device: Device, steps: usize) -> TrainOpts {
    TrainOpts {
        epochs: 1000,
        batch: 8,
        lr: 2e-3,
        val_fraction: 0.0,
        patience: 0,
        max_steps: steps,
        seed: 7,
        threads: 0,
        device,
        ..TrainOpts::default()
    }
}

fn fresh(size: &str) -> Result<Classifier, String> {
    let cfg = crate::classifiers::preset(size)
        .ok_or_else(|| format!("no preset {size:?}"))?
        .gematria(true);
    Classifier::new(cfg, &[HeadSpec::single("report", &["a", "b"])], 11)
}

/// the GPU's side of the report: the adapter, its limits and the probe
#[cfg(feature = "gpu")]
fn adapter() -> Value {
    use super::gpu::{self, Precision, probe};
    let g = match gpu::shared() {
        Ok(g) => g,
        Err(e) => return json!({ "error": e }),
    };
    let i = &g.info;
    let l = &g.limits;
    let probes: Vec<Value> = [Precision::Exact, Precision::Fast]
        .into_iter()
        .map(|p| {
            let t = Instant::now();
            let r = probe::probe(g, p);
            json!({
                "precision": format!("{p:?}").to_lowercase(),
                "exact": r.exact(),
                "max_rel": r.max_rel(),
                "seconds": t.elapsed().as_secs_f64(),
                "kernels": r.checks.iter().map(|c| json!({
                    "kernel": c.kernel, "values": c.values, "differ": c.differ,
                    "max_ulps": c.max_ulps, "max_rel": c.max_rel,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "name": i.name,
        "vendor": format!("{:#06x}", i.vendor),
        "device": format!("{:#06x}", i.device),
        "device_type": format!("{:?}", i.device_type),
        "backend": format!("{:?}", i.backend),
        "driver": i.driver,
        "driver_info": i.driver_info,
        "limits": {
            "max_buffer_size": l.max_buffer_size,
            "max_storage_buffer_binding_size": l.max_storage_buffer_binding_size,
            "max_uniform_buffer_binding_size": l.max_uniform_buffer_binding_size,
            "min_uniform_buffer_offset_alignment": l.min_uniform_buffer_offset_alignment,
            "max_storage_buffers_per_shader_stage": l.max_storage_buffers_per_shader_stage,
            "max_compute_workgroup_size": [l.max_compute_workgroup_size_x, l.max_compute_workgroup_size_y, l.max_compute_workgroup_size_z],
            "max_compute_invocations_per_workgroup": l.max_compute_invocations_per_workgroup,
            "max_compute_workgroup_storage_size": l.max_compute_workgroup_storage_size,
            "max_compute_workgroups_per_dimension": l.max_compute_workgroups_per_dimension,
            "all": format!("{l:?}"),
        },
        "probe": probes,
    })
}

#[cfg(not(feature = "gpu"))]
fn adapter() -> Value {
    json!({ "error": "this build has no GPU backend (build with the cargo feature `gpu`)" })
}

/// the fixed training run on one device: seconds, and the trained weights
fn train_on(size: &str, device: Device, steps: usize, data: &[Example]) -> Result<(f64, Vec<u8>, Value), String> {
    let mut clf = fresh(size)?;
    let t = Instant::now();
    let r = train::train(&mut clf, data, &opts(device, steps), |_| Flow::Go)?;
    let secs = t.elapsed().as_secs_f64();
    let v = json!({
        "seconds": secs,
        "steps": r.steps,
        "tokens_per_sec": r.tokens as f64 / secs.max(1e-9),
    });
    Ok((secs, clf.to_bytes(), v))
}

/// predicting `texts` on one device: seconds and each text's probabilities
fn predict_on(clf: &Classifier, device: Device, texts: &[&str], pool: &Pool) -> Result<(f64, Vec<Vec<f32>>), String> {
    let t = Instant::now();
    let probs: Vec<Vec<f32>> = match device {
        Device::Cpu => clf
            .predict_many(texts, &[], pool)
            .into_iter()
            .map(|mut o| o.swap_remove(0).probs)
            .collect(),
        _ => {
            let r: Runner = Runner::open(device, &clf.net)?.ok_or("no GPU")?;
            let bytes: Vec<&[u8]> = texts.iter().map(|t| t.as_bytes()).collect();
            let zero = vec![0f32; clf.net.cfg.extra];
            let extras: Vec<&[f32]> = texts.iter().map(|_| zero.as_slice()).collect();
            r.text_runs(&clf.net, &bytes, &extras, &clf.gains())
                .into_iter()
                .map(|(l, _)| clf.read(&l).swap_remove(0).probs)
                .collect()
        }
    };
    Ok((t.elapsed().as_secs_f64(), probs))
}

/// how far a device's probabilities are from the CPU's
fn compare(cpu: &[Vec<f32>], other: &[Vec<f32>]) -> Value {
    let (mut differ, mut max) = (0usize, 0f32);
    for (a, b) in cpu.iter().zip(other) {
        if a.iter().zip(b).any(|(x, y)| x.to_bits() != y.to_bits()) {
            differ += 1;
        }
        for (x, y) in a.iter().zip(b) {
            max = max.max((x - y).abs());
        }
    }
    json!({ "texts_differing": differ, "max_abs_difference": max })
}

/// the whole report
pub fn report(o: &Opts) -> Value {
    let started = Instant::now();
    let threads = pool::available();
    let gpu_ok = Device::Gpu.check();
    let mut sizes = Vec::new();
    let train_data = data(64);
    let texts: Vec<(String, usize)> = (0..o.texts).map(|i| text(i + 1_000_000)).collect();
    let texts: Vec<&str> = texts.iter().map(|(t, _)| t.as_str()).collect();
    let cpu_pool = Pool::new(threads);
    for size in &o.sizes {
        let mut v = json!({ "preset": size });
        let res: Result<(), String> = (|| {
            let (_, cpu_bytes, cpu) = train_on(size, Device::Cpu, o.steps, &train_data)?;
            let clf = Classifier::from_bytes(&cpu_bytes)?;
            v["weights"] = json!(clf.net.params().iter().map(|(_, p)| p.len()).sum::<usize>());
            v["train"] = json!({ "cpu": cpu });
            let (cpu_secs, cpu_probs) = predict_on(&clf, Device::Cpu, &texts, &cpu_pool)?;
            v["predict"] = json!({
                "texts": texts.len(),
                "cpu": { "seconds": cpu_secs, "texts_per_sec": texts.len() as f64 / cpu_secs.max(1e-9) },
            });
            if gpu_ok.is_ok() {
                for (name, dev) in [("gpu", Device::Gpu), ("gpu_fast", Device::GpuFast)] {
                    match train_on(size, dev, o.steps, &train_data) {
                        Ok((_, bytes, mut t)) => {
                            t["same_weights_as_cpu"] = json!(bytes == cpu_bytes);
                            v["train"][name] = t;
                        }
                        Err(e) => v["train"][name] = json!({ "error": e }),
                    }
                    match predict_on(&clf, dev, &texts, &cpu_pool) {
                        Ok((secs, probs)) => {
                            let mut p = json!({
                                "seconds": secs,
                                "texts_per_sec": texts.len() as f64 / secs.max(1e-9),
                                "speedup_over_cpu": cpu_secs / secs.max(1e-9),
                            });
                            p["against_cpu"] = compare(&cpu_probs, &probs);
                            v["predict"][name] = p;
                        }
                        Err(e) => v["predict"][name] = json!({ "error": e }),
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = res {
            v["error"] = json!(e);
        }
        sizes.push(v);
    }
    json!({
        "report": "anend gpu.report",
        "version": 1,
        "anend": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "debug_build": cfg!(debug_assertions),
        "cpu": { "threads": threads, "kernels": kernels::simd_name() },
        "gpu_build": cfg!(feature = "gpu"),
        "gpu": adapter(),
        "gpu_note": gpu_ok.err(),
        "reading": device::reading(device::choice(), true),
        "workload": {
            "train": format!("{} steps of batch 8 on 64 two-class texts, seed 7", o.steps),
            "predict": format!("{} texts of 20 to about 400 bytes", o.texts),
        },
        "sizes": sizes,
        "seconds": started.elapsed().as_secs_f64(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_measures_and_compares() {
        let r = report(&Opts {
            sizes: vec!["tiny".into()],
            texts: 12,
            steps: 2,
        });
        eprintln!("{r:#}");
        let s = &r["sizes"][0];
        assert!(s.get("error").is_none(), "{r}");
        assert!(s["train"]["cpu"]["seconds"].as_f64().is_some(), "{r}");
        assert_eq!(s["predict"]["texts"], 12, "{r}");
        if Device::Gpu.check().is_ok() {
            // the probe ran, and exact precision on an exact device gives the CPU's bits
            assert!(
                r["gpu"]["probe"][0]["kernels"]
                    .as_array()
                    .is_some_and(|k| !k.is_empty()),
                "{r}"
            );
            if r["gpu"]["probe"][0]["exact"] == true {
                assert_eq!(s["predict"]["gpu"]["against_cpu"]["texts_differing"], 0, "{r}");
                assert_eq!(s["train"]["gpu"]["same_weights_as_cpu"], true, "{r}");
            }
            assert!(
                s["predict"]["gpu_fast"]["against_cpu"]["max_abs_difference"]
                    .as_f64()
                    .unwrap()
                    < 1e-3,
                "{r}"
            );
        } else {
            assert!(r["gpu_note"].is_string(), "{r}");
        }
    }
}
