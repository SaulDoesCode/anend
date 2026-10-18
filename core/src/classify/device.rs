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
