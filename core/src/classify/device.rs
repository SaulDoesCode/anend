//! Where a run's numbers are computed. The CPU is the default and the reference:
//! bit-deterministic on every machine. The GPU (cargo feature `gpu`, see `gpu`) is an
//! explicit choice per run — a training option, a generation option, a spec field —
//! and never the default, so saved models keep answering on the exact CPU path unless
//! someone asks otherwise.

use serde::{Deserialize, Serialize};

use super::model::Net;
use super::train::Example;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    /// the CPU kernels (the reference)
    #[default]
    Cpu,
    /// the GPU in exact precision: the CPU's bits on a device the probe finds exact
    Gpu,
    /// the GPU with plain operators: faster, within a small tolerance of the CPU
    GpuFast,
}

impl Device {
    pub fn is_gpu(self) -> bool {
        self != Device::Cpu
    }

    /// whether this build and machine can run on the device; the reason when not
    pub fn check(self) -> Result<(), String> {
        if !self.is_gpu() {
            return Ok(());
        }
        #[cfg(feature = "gpu")]
        {
            super::gpu::shared().map(|_| ())
        }
        #[cfg(not(feature = "gpu"))]
        {
            Err("this build has no GPU backend (build with the cargo feature `gpu`)".into())
        }
    }

    #[cfg(feature = "gpu")]
    pub fn precision(self) -> super::gpu::Precision {
        match self {
            Device::GpuFast => super::gpu::Precision::Fast,
            _ => super::gpu::Precision::Exact,
        }
    }
}

/// a net's training state on the device (uninhabited in builds without the GPU)
#[cfg(feature = "gpu")]
pub struct Trainer {
    t: super::gpu::train::Trainer,
    buf: Net,
}

#[cfg(not(feature = "gpu"))]
pub enum Trainer {}

impl Trainer {
    /// the device's trainer for a run on `device`, None on the CPU
    pub fn open(device: Device, net: &Net) -> Result<Option<Trainer>, String> {
        device.check()?;
        if !device.is_gpu() {
            return Ok(None);
        }
        #[cfg(feature = "gpu")]
        {
            let g = super::gpu::shared()?;
            Ok(Some(Trainer {
                t: super::gpu::train::Trainer::new(g, net, device.precision())?,
                buf: net.zeros_like(),
            }))
        }
        #[cfg(not(feature = "gpu"))]
        {
            let _ = net;
            unreachable!("checked")
        }
    }

    /// a batch of cropped examples (index, coefficient, crop key) on the device: the
    /// gradient into `total`, as `train::batch_grad` sums it; returns (Σ coef·loss, tokens)
    pub fn batch(
        &mut self,
        net: &Net,
        data: &[Example],
        items: &[(usize, f32, u64)],
        smoothing: f32,
        total: &mut Net,
    ) -> (f64, usize) {
        #[cfg(feature = "gpu")]
        {
            use super::model::Mode;
            use super::train::{crop_seq, sequence_of};
            total.fill(0.0);
            self.t.begin(net);
            let (mut loss, mut tokens) = (0f64, 0usize);
            for &(i, coef, kk) in items {
                let ex = &data[i];
                let buf = &mut self.buf;
                buf.fill(0.0);
                let extra = if ex.extra.is_empty() {
                    vec![0.0; net.cfg.extra]
                } else {
                    ex.extra.clone()
                };
                let (ts, targets) = sequence_of(net, ex);
                let (w, start) = crop_seq(net, &ts, kk);
                let mode = Mode {
                    dropout: net.cfg.dropout,
                    key: kk,
                };
                let (l, t) = self.t.example(
                    net,
                    &w,
                    start,
                    targets.iter().map(|(h, t)| (*h, t)),
                    &extra,
                    mode,
                    coef,
                    smoothing,
                    buf,
                );
                for ((_, d), (_, s)) in total.params_mut().into_iter().zip(buf.params()) {
                    d.iter_mut().zip(s.iter()).for_each(|(a, b)| *a += b);
                }
                loss += (coef * l) as f64;
                tokens += t;
            }
            self.t.finish(total);
            (loss, tokens)
        }
        #[cfg(not(feature = "gpu"))]
        {
            let _ = (net, data, items, smoothing, total);
            match *self {}
        }
    }
}

/// one token at a time through a generative net: `model::Decoder` on the CPU,
/// `gpu::net::GpuDecoder` on the device
pub trait Decode {
    /// the logits for the next token
    fn logits(&self) -> &[f32];
    fn push_token(&mut self, t: u32);
}

impl Decode for super::model::Decoder<'_> {
    fn logits(&self) -> &[f32] {
        super::model::Decoder::logits(self)
    }

    fn push_token(&mut self, t: u32) {
        super::model::Decoder::push_token(self, t)
    }
}

#[cfg(feature = "gpu")]
impl Decode for super::gpu::net::GpuDecoder<'_> {
    fn logits(&self) -> &[f32] {
        super::gpu::net::GpuDecoder::logits(self)
    }

    fn push_token(&mut self, t: u32) {
        super::gpu::net::GpuDecoder::push_token(self, t)
    }
}

/// a net uploaded for inference on the device (uninhabited without the GPU)
#[cfg(feature = "gpu")]
pub struct Runner {
    pub gn: super::gpu::net::GpuNet,
}

#[cfg(not(feature = "gpu"))]
pub enum Runner {}

impl Runner {
    /// the net on `device`, None on the CPU
    pub fn open(device: Device, net: &Net) -> Result<Option<Runner>, String> {
        device.check()?;
        if !device.is_gpu() {
            return Ok(None);
        }
        #[cfg(feature = "gpu")]
        {
            let g = super::gpu::shared()?;
            Ok(Some(Runner {
                gn: super::gpu::net::GpuNet::new(g, net, device.precision())?,
            }))
        }
        #[cfg(not(feature = "gpu"))]
        {
            let _ = net;
            unreachable!("checked")
        }
    }

    /// every head's logits averaged over the windows of `data` (`train::forward_text`'s
    /// mean), the trunk on the device
    pub fn text_logits(&self, net: &Net, data: &[u8], extra: &[f32], gains: &super::model::Gains) -> Vec<Vec<f32>> {
        #[cfg(feature = "gpu")]
        {
            self.gn.text_logits(net, data, extra, gains)
        }
        #[cfg(not(feature = "gpu"))]
        {
            let _ = (net, data, extra, gains);
            match *self {}
        }
    }

    /// many texts at once, their windows packed on the device: per text, each head's
    /// logits and the pooled vector averaged over its windows (`Classifier::run_bytes`)
    pub fn text_runs(
        &self,
        net: &Net,
        texts: &[&[u8]],
        extras: &[&[f32]],
        gains: &super::model::Gains,
    ) -> Vec<(Vec<Vec<f32>>, Vec<f32>)> {
        #[cfg(feature = "gpu")]
        {
            self.gn
                .text_runs(net, texts, extras, gains)
                .into_iter()
                .map(|r| (r.logits, r.pooled))
                .collect()
        }
        #[cfg(not(feature = "gpu"))]
        {
            let _ = (net, texts, extras, gains);
            match *self {}
        }
    }

    /// a decoder on the device that has read the start token and `prompt`
    pub fn decoder<'a>(&'a self, net: &'a Net, gains: super::model::Gains, prompt: &[u8]) -> Box<dyn Decode + 'a> {
        #[cfg(feature = "gpu")]
        {
            Box::new(super::gpu::net::GpuDecoder::new(&self.gn, net, gains, prompt))
        }
        #[cfg(not(feature = "gpu"))]
        {
            let _ = (net, gains, prompt);
            match *self {}
        }
    }
}

// ---------------------------------------------------------------- reading texts

/// where classifiers read texts (predict, search, indexing, buckets, tints): the user's
/// choice, for this process (`classify.device`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// the GPU in exact precision when there is one that isn't a software renderer and
    /// the probe finds it bit-exact on every kernel (so the results are the CPU's), else
    /// the CPU
    #[default]
    Auto,
    Cpu,
    /// the GPU in exact precision, whatever the probe found
    Gpu,
    /// the GPU with plain operators
    GpuFast,
}

impl Choice {
    pub fn parse(s: &str) -> Option<Choice> {
        Some(match s.trim() {
            "auto" | "" => Choice::Auto,
            "cpu" => Choice::Cpu,
            "gpu" => Choice::Gpu,
            "gpu_fast" | "gpu fast" => Choice::GpuFast,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Choice::Auto => "auto",
            Choice::Cpu => "cpu",
            Choice::Gpu => "gpu",
            Choice::GpuFast => "gpu_fast",
        }
    }
}

static CHOICE: std::sync::Mutex<Option<Choice>> = std::sync::Mutex::new(None);

/// the process's choice: as set, else `ANEND_CLASSIFY_DEVICE`, else auto
pub fn choice() -> Choice {
    if let Some(c) = *CHOICE.lock().unwrap_or_else(|e| e.into_inner()) {
        return c;
    }
    std::env::var("ANEND_CLASSIFY_DEVICE")
        .ok()
        .and_then(|v| Choice::parse(&v))
        .unwrap_or_default()
}

pub fn set_choice(c: Choice) {
    *CHOICE.lock().unwrap_or_else(|e| e.into_inner()) = Some(c);
}

/// where reading runs, and why when it isn't where it was asked to
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reading {
    pub choice: Choice,
    pub device: Device,
    /// the results are the CPU's bits (the CPU, or exact precision on a device the
    /// probe finds exact)
    pub exact: bool,
    /// the GPU it runs on
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    /// why it runs somewhere else than asked, or why auto chose the CPU
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Reading {
    pub fn cpu(choice: Choice, note: Option<String>) -> Reading {
        Reading {
            choice,
            device: Device::Cpu,
            exact: true,
            gpu: None,
            note,
        }
    }
}

/// whether the shared GPU is bit-exact on every kernel in exact precision (the probe,
/// run once a process); Err when there's no GPU
#[cfg(feature = "gpu")]
pub fn probe_exact() -> Result<bool, String> {
    static EXACT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let g = super::gpu::shared()?;
    Ok(*EXACT.get_or_init(|| super::gpu::probe::probe(g, super::gpu::Precision::Exact).exact()))
}

/// where reading runs for `choice`; `allowed` is false where the GPU isn't to be used (a
/// shared hub without `--gpu`)
pub fn reading(choice: Choice, allowed: bool) -> Reading {
    if choice == Choice::Cpu {
        return Reading::cpu(choice, None);
    }
    if !allowed {
        return Reading::cpu(choice, Some("this hub reads on its CPU".into()));
    }
    let want = if choice == Choice::GpuFast {
        Device::GpuFast
    } else {
        Device::Gpu
    };
    if let Err(e) = want.check() {
        let note = match choice {
            Choice::Auto => format!("no GPU: {e}; so the CPU reads"),
            _ => format!("no GPU here: {e}; read on the CPU"),
        };
        return Reading::cpu(choice, Some(note));
    }
    #[cfg(feature = "gpu")]
    {
        let g = match super::gpu::shared() {
            Ok(g) => g,
            Err(e) => return Reading::cpu(choice, Some(format!("no GPU here: {e}; read on the CPU"))),
        };
        let name = g.describe();
        let exact = want == Device::Gpu && probe_exact().unwrap_or(false);
        if choice == Choice::Auto {
            if g.info.device_type == wgpu::DeviceType::Cpu {
                return Reading::cpu(
                    choice,
                    Some(format!("the only GPU is a software renderer, {name}, so the CPU reads")),
                );
            }
            if !exact {
                return Reading::cpu(
                    choice,
                    Some(format!(
                        "{name} isn't bit-exact on every kernel, so the CPU reads (choose the GPU to use it anyway)"
                    )),
                );
            }
        }
        Reading {
            choice,
            device: want,
            exact,
            gpu: Some(name),
            note: None,
        }
    }
    #[cfg(not(feature = "gpu"))]
    {
        unreachable!("checked")
    }
}

/// nets on the device for reading, kept between calls: uploading a net's weights costs
/// more than reading a few texts with it. Keyed by the caller's name for the net and a
/// fingerprint of its weights, so a retrained net is uploaded again.
pub fn runner(key: &str, device: Device, net: &Net) -> Result<Option<std::sync::Arc<Runner>>, String> {
    if !device.is_gpu() {
        return Ok(None);
    }
    #[cfg(feature = "gpu")]
    {
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};
        type Kept = Mutex<HashMap<(String, u64, Device), Arc<Runner>>>;
        static KEPT: std::sync::OnceLock<Kept> = std::sync::OnceLock::new();
        let kept = KEPT.get_or_init(Default::default);
        let k = (key.to_string(), fingerprint(net), device);
        if let Some(r) = kept.lock().unwrap_or_else(|e| e.into_inner()).get(&k) {
            return Ok(Some(r.clone()));
        }
        let r = Arc::new(Runner::open(device, net)?.expect("a GPU device"));
        let mut all = kept.lock().unwrap_or_else(|e| e.into_inner());
        // a few nets at a time: the rest are uploaded again when next used
        if all.len() >= 16 {
            all.clear();
        }
        all.insert(k, r.clone());
        Ok(Some(r))
    }
    #[cfg(not(feature = "gpu"))]
    {
        let _ = (key, net);
        unreachable!("checked")
    }
}

/// a cheap hash of a net's shapes and a sample of its weights (every 61st value of each
/// tensor, and its last)
#[cfg(feature = "gpu")]
fn fingerprint(net: &Net) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    for (_, p) in net.params() {
        eat(p.len() as u64);
        for v in p.iter().step_by(61).chain(p.last()) {
            eat(v.to_bits() as u64);
        }
    }
    h
}

/// a decoder for `net` on the runner's device, or on the CPU without one
pub fn decoder<'a>(
    runner: Option<&'a Runner>,
    net: &'a Net,
    gains: super::model::Gains,
    prompt: &[u8],
) -> Box<dyn Decode + 'a> {
    match runner {
        Some(r) => r.decoder(net, gains, prompt),
        None => Box::new(super::model::Decoder::new(net, gains, prompt)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_parse_and_the_gpu_needs_the_feature() {
        let d: Device = serde_json::from_str("\"gpu_fast\"").unwrap();
        assert_eq!(d, Device::GpuFast);
        assert_eq!(Device::default(), Device::Cpu);
        assert!(Device::Cpu.check().is_ok());
        #[cfg(not(feature = "gpu"))]
        assert!(Device::Gpu.check().unwrap_err().contains("feature"));
    }
}
