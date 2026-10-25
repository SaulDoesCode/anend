//! The GPU compute backend (cargo feature `gpu`): the engine's kernels as WGSL compute
//! shaders run through wgpu (Vulkan, Metal, DX12, GL natively; the same WGSL is valid
//! WebGPU, so a browser build can use it later).
//!
//! - [`Gpu`]: a device and queue, the shader module in two precisions, buffers, and
//!   [`Run`], which records a list of kernel dispatches and submits them at once.
//! - [`net::GpuNet`]: a [`Net`](super::Net)'s trunk resident on the device — forward
//!   (inference and training), the generative head, a KV-cache decoder, and backward.
//! - [`probe`]: runs every kernel on random data on the device and on the CPU and reports
//!   whether the device matched bit for bit.
//!
//! The CPU path stays the reference. Shaders follow its operation order exactly (see
//! `kernels.wgsl`), so on a device that rounds each operation once the results are the
//! CPU's bits; whether a device does is a measured fact ([`probe`]), not an assumption.
//! [`Precision::Exact`] guards against the usual differences (fused multiply-adds,
//! approximate division and square root) at some cost; [`Precision::Fast`] uses the
//! plain operators and stays within a small tolerance of the CPU.

pub mod net;
pub mod probe;
#[cfg(test)]
pub(crate) mod tests;
pub mod train;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use wgpu::util::DeviceExt;

/// how the shaders do their arithmetic
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Precision {
    /// every product laundered so it cannot fuse into an add, division and square root
    /// correctly rounded in integer arithmetic: the CPU's bits on an IEEE device
    #[default]
    Exact,
    /// plain operators: the driver may fuse multiply-adds and approximate ÷ and √
    Fast,
}

/// counters of what the device was asked to do, for profiling (`gpu_profile`): read
/// with [`Profile::take`]
#[derive(Default)]
pub struct Profile {
    pub buffers: std::sync::atomic::AtomicU64,
    pub buffer_bytes: std::sync::atomic::AtomicU64,
    pub uploads: std::sync::atomic::AtomicU64,
    pub dispatches: std::sync::atomic::AtomicU64,
    pub bind_groups: std::sync::atomic::AtomicU64,
    pub submits: std::sync::atomic::AtomicU64,
    pub reads: std::sync::atomic::AtomicU64,
    /// nanoseconds building command buffers (bind groups, passes), waiting for the
    /// device, and copying results out of mapped memory
    pub ns_encode: std::sync::atomic::AtomicU64,
    /// nanoseconds making buffers and writing into them
    pub ns_upload: std::sync::atomic::AtomicU64,
    pub ns_wait: std::sync::atomic::AtomicU64,
    pub ns_readback: std::sync::atomic::AtomicU64,
}

pub static PROFILE: Profile = Profile {
    buffers: std::sync::atomic::AtomicU64::new(0),
    buffer_bytes: std::sync::atomic::AtomicU64::new(0),
    uploads: std::sync::atomic::AtomicU64::new(0),
    dispatches: std::sync::atomic::AtomicU64::new(0),
    bind_groups: std::sync::atomic::AtomicU64::new(0),
    submits: std::sync::atomic::AtomicU64::new(0),
    reads: std::sync::atomic::AtomicU64::new(0),
    ns_encode: std::sync::atomic::AtomicU64::new(0),
    ns_upload: std::sync::atomic::AtomicU64::new(0),
    ns_wait: std::sync::atomic::AtomicU64::new(0),
    ns_readback: std::sync::atomic::AtomicU64::new(0),
};

fn count(c: &std::sync::atomic::AtomicU64, n: u64) {
    c.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}

impl Profile {
    /// the counts since the last take, as (name, value) pairs; zeroes them
    pub fn take(&self) -> Vec<(&'static str, u64)> {
        use std::sync::atomic::Ordering::Relaxed;
        [
            ("buffers", &self.buffers),
            ("buffer_bytes", &self.buffer_bytes),
            ("uploads", &self.uploads),
            ("dispatches", &self.dispatches),
            ("bind_groups", &self.bind_groups),
            ("submits", &self.submits),
            ("reads", &self.reads),
            ("ns_encode", &self.ns_encode),
            ("ns_upload", &self.ns_upload),
            ("ns_wait", &self.ns_wait),
            ("ns_readback", &self.ns_readback),
        ]
        .into_iter()
        .map(|(n, c)| (n, c.swap(0, Relaxed)))
        .collect()
    }
}

/// per kernel: dispatches and nanoseconds, gathered when `ANEND_GPU_PROFILE_OPS=1` (each
/// dispatch then runs alone and is waited for, so the totals are slower than a real run
/// and only their proportions mean anything)
pub static OP_NS: Mutex<std::collections::BTreeMap<&'static str, (u64, u64)>> =
    Mutex::new(std::collections::BTreeMap::new());

fn profiling_ops() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("ANEND_GPU_PROFILE_OPS").is_ok_and(|v| v == "1"))
}

/// a place in a buffer: the buffer and an offset in floats
pub type At<'a> = (&'a Buf, usize);

/// a device buffer of f32
pub struct Buf {
    pub(crate) b: wgpu::Buffer,
    pub len: usize,
}

impl Buf {
    pub fn bytes(&self) -> u64 {
        self.b.size()
    }
}

/// the parameters every kernel reads (`struct P` in the shader): 20 u32 then 4 f32
#[derive(Clone, Copy, Default)]
pub(crate) struct Params {
    pub u: [u32; 20],
    pub f: [f32; 4],
}

const PARAM_BYTES: u64 = 96;

impl Params {
    pub fn new(u: &[u32]) -> Params {
        let mut p = Params::default();
        p.u[..u.len()].copy_from_slice(u);
        p
    }

    pub fn f(mut self, f: &[f32]) -> Params {
        self.f[..f.len()].copy_from_slice(f);
        self
    }

    fn bytes(&self) -> [u8; PARAM_BYTES as usize] {
        let mut out = [0u8; PARAM_BYTES as usize];
        for (i, v) in self.u.iter().enumerate() {
            // u3.w is the exact path's zero; never anything else
            let v = if i == 15 { 0 } else { *v };
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        for (i, v) in self.f.iter().enumerate() {
            out[80 + i * 4..84 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// an f32 as a WGSL hexadecimal literal holding exactly its value
fn hex(v: f32) -> String {
    let b = v.to_bits();
    let sign = if b >> 31 == 1 { "-" } else { "" };
    let e = ((b >> 23) & 0xff) as i32;
    let m = b & 0x7f_ffff;
    if e == 0 && m == 0 {
        return format!("{sign}0.0f");
    }
    assert!(e != 0 && e != 255, "normal constants only");
    format!("{sign}0x1.{:06x}p{}f", m << 1, e - 127)
}

/// the shader source for a precision: constants, the arithmetic, then the kernels
pub fn source(prec: Precision) -> String {
    let consts = [
        ("EXP_LO", -87.0f32),
        ("EXP_HI", 88.72f32),
        ("LOG2E", std::f32::consts::LOG2_E),
        ("ROUND", 12_582_912.0f32),
        ("LN2_HI", 0.693_359_4f32),
        ("LN2_LO", -2.121_944_4e-4f32),
        ("C6", 1.0f32 / 6.0),
        ("C24", 1.0f32 / 24.0),
        ("C120", 1.0f32 / 120.0),
        ("C720", 1.0f32 / 720.0),
        ("C5040", 1.0f32 / 5040.0),
    ];
    let mut s = String::new();
    for (n, v) in consts {
        s += &format!("const {n}: f32 = {};\n", hex(v));
    }
    s += match prec {
        Precision::Exact => {
            "fn mul(a: f32, b: f32) -> f32 { return bitcast<f32>(bitcast<u32>(a * b) ^ p.u3.w); }\n\
             fn div(a: f32, b: f32) -> f32 { return div_sw(a, b); }\n\
             fn sqrtf(x: f32) -> f32 { return sqrt_sw(x); }\n"
        }
        Precision::Fast => {
            "fn mul(a: f32, b: f32) -> f32 { return a * b; }\n\
             fn div(a: f32, b: f32) -> f32 { return a / b; }\n\
             fn sqrtf(x: f32) -> f32 { return sqrt(x); }\n"
        }
    };
    s + include_str!("kernels.wgsl")
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub limits: wgpu::Limits,
    layout: wgpu::BindGroupLayout,
    playout: wgpu::PipelineLayout,
    modules: [OnceLock<wgpu::ShaderModule>; 2],
    pipes: Mutex<HashMap<(&'static str, Precision), wgpu::ComputePipeline>>,
    dummy: wgpu::Buffer,
    align: u64,
}

// Proving `Gpu: Send + Sync` structurally walks wgpu's types deep enough on some
// backends to hit the compiler's recursion limit (the future-incompatible
// `recursion_depth_exceeding_limit` warning on `SHARED`). Every field is Send + Sync on
// native targets (wgpu's handles are, the rest is std); the test below checks each field's
// type on its own, so the static needs no structural proof.
// SAFETY: see above. On wasm wgpu's handles are not Send, so there the structural
// proof stays (and fails, as it should: the browser build has no shared GPU).
#[cfg(not(target_arch = "wasm32"))]
unsafe impl Send for Gpu {}
#[cfg(not(target_arch = "wasm32"))]
unsafe impl Sync for Gpu {}

static SHARED: OnceLock<Result<Gpu, String>> = OnceLock::new();

#[cfg(test)]
#[allow(dead_code)]
fn fields_are_send_and_sync() {
    fn both<T: Send + Sync>() {}
    both::<wgpu::Device>();
    both::<wgpu::Queue>();
    both::<wgpu::AdapterInfo>();
    both::<wgpu::Limits>();
    both::<wgpu::BindGroupLayout>();
    both::<wgpu::PipelineLayout>();
    both::<OnceLock<wgpu::ShaderModule>>();
    both::<Mutex<HashMap<(&'static str, Precision), wgpu::ComputePipeline>>>();
    both::<wgpu::Buffer>();
}

/// the process's GPU, opened on first use; `Err` says why there is none (no adapter, or
/// `ANEND_GPU=off`). `WGPU_BACKEND` (vulkan, metal, dx12, gl) and `WGPU_ADAPTER_NAME`
/// pick among several.
pub fn shared() -> Result<&'static Gpu, String> {
    SHARED.get_or_init(Gpu::new).as_ref().map_err(|e| e.clone())
}

impl Gpu {
    pub fn new() -> Result<Gpu, String> {
        if std::env::var("ANEND_GPU").is_ok_and(|v| v == "off" || v == "0") {
            return Err("the GPU is turned off (ANEND_GPU=off)".into());
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = match std::env::var("WGPU_ADAPTER_NAME") {
            Ok(name) => {
                let all = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
                let name = name.to_lowercase();
                all.into_iter()
                    .find(|a| a.get_info().name.to_lowercase().contains(&name))
                    .ok_or(format!("no GPU adapter named like {name:?}"))?
            }
            Err(_) => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            }))
            .map_err(|e| format!("no GPU adapter: {e}"))?,
        };
        let info = adapter.get_info();
        // everything the adapter offers: model sizes are bounded by the device, not by us
        let limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("anend classify"),
            required_features: wgpu::Features::empty(),
            required_limits: limits.clone(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("GPU device {}: {e}", info.name))?;
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(PARAM_BYTES),
            },
            count: None,
        }];
        entries.extend((1..=6).map(storage));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kernels"),
            entries: &entries,
        });
        let playout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kernels"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let dummy = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("unused binding"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let align = (limits.min_uniform_buffer_offset_alignment as u64).max(PARAM_BYTES);
        Ok(Gpu {
            device,
            queue,
            info,
            limits,
            layout,
            playout,
            modules: [OnceLock::new(), OnceLock::new()],
            pipes: Mutex::new(HashMap::new()),
            dummy,
            align,
        })
    }

    /// "name (backend, device type)"
    pub fn describe(&self) -> String {
        format!(
            "{} ({:?}, {:?})",
            self.info.name, self.info.backend, self.info.device_type
        )
    }

    /// the largest buffer the device takes, in f32s
    pub fn max_floats(&self) -> usize {
        self.limits
            .max_storage_buffer_binding_size
            .min(self.limits.max_buffer_size) as usize
            / 4
    }

    fn module(&self, prec: Precision) -> &wgpu::ShaderModule {
        self.modules[prec as usize].get_or_init(|| {
            self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kernels.wgsl"),
                source: wgpu::ShaderSource::Wgsl(source(prec).into()),
            })
        })
    }

    fn pipeline(&self, entry: &'static str, prec: Precision) -> wgpu::ComputePipeline {
        let mut pipes = self.pipes.lock().unwrap();
        pipes
            .entry((entry, prec))
            .or_insert_with(|| {
                self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&self.playout),
                    module: self.module(prec),
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    cache: None,
                })
            })
            .clone()
    }

    /// a buffer holding `data`
    pub fn upload(&self, data: &[f32]) -> Buf {
        if data.is_empty() {
            return self.zeros(0);
        }
        let t = std::time::Instant::now();
        count(&PROFILE.buffers, 1);
        count(&PROFILE.uploads, 1);
        count(&PROFILE.buffer_bytes, (data.len() * 4) as u64);
        let b = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: f32_bytes(data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        });
        count(&PROFILE.ns_upload, t.elapsed().as_nanos() as u64);
        Buf { b, len: data.len() }
    }

    /// a zeroed buffer of n floats (at least one, as bindings cannot be empty)
    pub fn zeros(&self, n: usize) -> Buf {
        let t = std::time::Instant::now();
        count(&PROFILE.buffers, 1);
        count(&PROFILE.buffer_bytes, (n.max(1) * 4) as u64);
        let b = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (n.max(1) * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        count(&PROFILE.ns_upload, t.elapsed().as_nanos() as u64);
        Buf { b, len: n }
    }

    /// overwrites floats from `off` (lands before the next submitted run)
    pub fn write(&self, buf: &Buf, off: usize, data: &[f32]) {
        if !data.is_empty() {
            let t = std::time::Instant::now();
            debug_assert!(off + data.len() <= buf.len.max(1));
            count(&PROFILE.uploads, 1);
            self.queue.write_buffer(&buf.b, (off * 4) as u64, f32_bytes(data));
            count(&PROFILE.ns_upload, t.elapsed().as_nanos() as u64);
        }
    }

    /// a buffer's contents
    pub fn read(&self, buf: &Buf) -> Vec<f32> {
        let mut r = self.run(Precision::Exact);
        let t = r.read(buf, 0, buf.len);
        r.submit().swap_remove(t)
    }

    pub fn run(&self, prec: Precision) -> Run<'_> {
        Run {
            g: self,
            prec,
            params: Vec::new(),
            recs: Vec::new(),
            reads: 0,
        }
    }
}

fn f32_bytes(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding and any bytes are valid u8s
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

enum Rec {
    Dispatch {
        entry: &'static str,
        pipe: wgpu::ComputePipeline,
        bufs: [Option<wgpu::Buffer>; 6],
        slot: u64,
        groups: [u32; 3],
    },
    Copy {
        src: wgpu::Buffer,
        so: u64,
        dst: wgpu::Buffer,
        doff: u64,
        size: u64,
    },
    Read {
        src: wgpu::Buffer,
        off: u64,
        len: usize,
    },
}

/// a list of kernel dispatches (and buffer copies and reads) submitted together, in
/// order; each dispatch sees the writes of the ones before it
pub struct Run<'g> {
    pub(crate) g: &'g Gpu,
    pub prec: Precision,
    params: Vec<u8>,
    recs: Vec<Rec>,
    reads: usize,
}

/// workgroups for `n` items of `per` each, as an (x, y) grid within the 65535 limit
pub(crate) fn grid(n: usize, per: usize) -> [u32; 3] {
    let g = n.div_ceil(per).max(1);
    let x = g.min(65_535);
    [x as u32, g.div_ceil(x) as u32, 1]
}

impl<'g> Run<'g> {
    pub fn gpu(&self) -> &'g Gpu {
        self.g
    }

    pub(crate) fn op(&mut self, entry: &'static str, bufs: &[&Buf], p: Params, groups: [u32; 3]) {
        if groups.contains(&0) {
            return;
        }
        let slot = self.params.len() as u64;
        self.params.extend_from_slice(&p.bytes());
        self.params.resize((slot + self.g.align) as usize, 0);
        let mut bs: [Option<wgpu::Buffer>; 6] = Default::default();
        for (i, b) in bufs.iter().enumerate() {
            bs[i] = Some(b.b.clone());
        }
        let pipe = self.g.pipeline(entry, self.prec);
        self.recs.push(Rec::Dispatch {
            entry,
            pipe,
            bufs: bs,
            slot,
            groups,
        });
    }

    /// copies `len` floats between buffers
    pub fn copy(&mut self, src: &Buf, so: usize, dst: &Buf, doff: usize, len: usize) {
        if len > 0 {
            self.recs.push(Rec::Copy {
                src: src.b.clone(),
                so: (so * 4) as u64,
                dst: dst.b.clone(),
                doff: (doff * 4) as u64,
                size: (len * 4) as u64,
            });
        }
    }

    /// reads `len` floats from `off` once the run has executed; returns the index of the
    /// result in `submit`'s list
    pub fn read(&mut self, buf: &Buf, off: usize, len: usize) -> usize {
        self.recs.push(Rec::Read {
            src: buf.b.clone(),
            off: (off * 4) as u64,
            len,
        });
        self.reads += 1;
        self.reads - 1
    }

    /// submits everything and waits for the reads (and only then returns)
    pub fn submit(self) -> Vec<Vec<f32>> {
        let g = self.g;
        let started = std::time::Instant::now();
        count(&PROFILE.submits, 1);
        count(&PROFILE.reads, self.reads as u64);
        count(
            &PROFILE.dispatches,
            self.recs.iter().filter(|r| matches!(r, Rec::Dispatch { .. })).count() as u64,
        );
        let ub = (!self.params.is_empty()).then(|| {
            g.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("params"),
                contents: &self.params,
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });
        let mut enc = g
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let mut staging = Vec::new();
        let by_op = profiling_ops();
        let mut i = 0;
        while i < self.recs.len() {
            if let Rec::Dispatch { .. } = self.recs[i] {
                let ub = ub.as_ref().expect("params");
                let mut groups = Vec::new();
                while let Some(Rec::Dispatch { bufs, .. }) = self.recs.get(i) {
                    if by_op && !groups.is_empty() {
                        break;
                    }
                    let mut entries = vec![wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: ub,
                            offset: 0,
                            size: wgpu::BufferSize::new(PARAM_BYTES),
                        }),
                    }];
                    for (j, b) in bufs.iter().enumerate() {
                        entries.push(wgpu::BindGroupEntry {
                            binding: j as u32 + 1,
                            resource: b.as_ref().unwrap_or(&g.dummy).as_entire_binding(),
                        });
                    }
                    count(&PROFILE.bind_groups, 1);
                    groups.push(g.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &g.layout,
                        entries: &entries,
                    }));
                    i += 1;
                }
                let start = i - groups.len();
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                for (bg, rec) in groups.iter().zip(&self.recs[start..i]) {
                    if let Rec::Dispatch { pipe, slot, groups, .. } = rec {
                        pass.set_pipeline(pipe);
                        pass.set_bind_group(0, bg, &[*slot as u32]);
                        pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
                    }
                }
                drop(pass);
                if by_op && let Rec::Dispatch { entry, .. } = &self.recs[start] {
                    // this dispatch alone, waited for, its time added to its kernel's
                    let done = std::mem::replace(
                        &mut enc,
                        g.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None }),
                    );
                    let t = std::time::Instant::now();
                    g.queue.submit([done.finish()]);
                    g.device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU poll");
                    let mut m = OP_NS.lock().unwrap_or_else(|e| e.into_inner());
                    let e = m.entry(entry).or_default();
                    e.0 += 1;
                    e.1 += t.elapsed().as_nanos() as u64;
                }
                continue;
            }
            match &self.recs[i] {
                Rec::Copy {
                    src,
                    so,
                    dst,
                    doff,
                    size,
                } => enc.copy_buffer_to_buffer(src, *so, dst, *doff, *size),
                Rec::Read { src, off, len } => {
                    let size = (*len as u64 * 4).max(4);
                    let st = g.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("read"),
                        size,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    if *len > 0 {
                        enc.copy_buffer_to_buffer(src, *off, &st, 0, size);
                    }
                    staging.push((st, *len));
                }
                Rec::Dispatch { .. } => unreachable!(),
            }
            i += 1;
        }
        g.queue.submit([enc.finish()]);
        count(&PROFILE.ns_encode, started.elapsed().as_nanos() as u64);
        if staging.is_empty() {
            return Vec::new();
        }
        let waited = std::time::Instant::now();
        for (st, _) in &staging {
            st.map_async(wgpu::MapMode::Read, .., |r| r.expect("GPU read"));
        }
        g.device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU poll");
        count(&PROFILE.ns_wait, waited.elapsed().as_nanos() as u64);
        let copied = std::time::Instant::now();
        let out = staging
            .into_iter()
            .map(|(st, len)| {
                let v = {
                    let view = st.get_mapped_range(..).expect("mapped");
                    view[..len * 4]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| f32::from_le_bytes(*c))
                        .collect()
                };
                st.unmap();
                v
            })
            .collect();
        count(&PROFILE.ns_readback, copied.elapsed().as_nanos() as u64);
        out
    }

    // ------------------------------------------------------------ kernels

    /// y (+)= A·B for `z` products, each output one chain in ascending p; causal limits
    /// row r's chain to p ≤ t0 + r (attention over the positions so far)
    #[allow(clippy::too_many_arguments)]
    pub fn gemm(
        &mut self,
        a: View,
        b: View,
        y: View,
        n: usize,
        k: usize,
        m: usize,
        z: usize,
        acc: bool,
        causal: Option<usize>,
    ) {
        if n == 0 || m == 0 || z == 0 {
            return;
        }
        let flags = acc as u32 | if causal.is_some() { 2 } else { 0 };
        let u = [
            n as u32,
            k as u32,
            m as u32,
            flags,
            a.off as u32,
            a.s0 as u32,
            a.s1 as u32,
            a.zs as u32,
            b.off as u32,
            b.s0 as u32,
            b.s1 as u32,
            b.zs as u32,
            y.off as u32,
            y.s0 as u32,
            y.zs as u32,
            0,
            causal.unwrap_or(0) as u32,
            z as u32,
            b.per as u32,
        ];
        let p = Params::new(&u);
        if n < 16 {
            self.op("gemm_small", &[a.buf, b.buf, y.buf], p, grid(n * m * z, 64));
        } else {
            self.op(
                "gemm",
                &[a.buf, b.buf, y.buf],
                p,
                [m.div_ceil(64) as u32, n.div_ceil(64) as u32, z as u32],
            );
        }
    }

    /// RMSNorm of `rows` rows of d from x[x_off..] into y[y_off..]; with a trace, x̂ and
    /// rstd too
    #[allow(clippy::too_many_arguments)]
    pub fn norm_fwd(
        &mut self,
        x: (&Buf, usize),
        g: &Buf,
        y: (&Buf, usize),
        trace: Option<(At, At)>,
        rows: usize,
        d: usize,
        eps: f32,
    ) {
        let (xh, rs) = trace.unwrap_or(((y.0, 0), (y.0, 0)));
        let tr = trace.is_some();
        let p = Params::new(&[
            rows as u32,
            d as u32,
            x.1 as u32,
            y.1 as u32,
            tr as u32,
            rs.1 as u32,
            xh.1 as u32,
        ])
        .f(&[eps]);
        if tr {
            self.op("norm_fwd", &[x.0, g, y.0, xh.0, rs.0], p, grid(rows, 8));
        } else {
            self.op("norm_fwd", &[x.0, g, y.0], p, grid(rows, 8));
        }
    }

    /// RMSNorm backward: dx += …; dg += Σ dy·x̂ (rows in order)
    #[allow(clippy::too_many_arguments)]
    pub fn norm_back(
        &mut self,
        dy: (&Buf, usize),
        xhat: (&Buf, usize),
        rstd: (&Buf, usize),
        g: &Buf,
        dx: (&Buf, usize),
        dg: (&Buf, usize),
        rows: usize,
        d: usize,
    ) {
        let p = Params::new(&[
            rows as u32,
            d as u32,
            dy.1 as u32,
            xhat.1 as u32,
            rstd.1 as u32,
            dx.1 as u32,
        ]);
        self.op("norm_back", &[dy.0, xhat.0, g, rstd.0, dx.0], p, grid(rows, 8));
        self.col_acc(dy, Some(xhat), dg, rows, d);
    }

    /// dst[i] += Σ_r a[r][i]·b[r][i] (or Σ_r a[r][i] without b), rows in order
    pub fn col_acc(&mut self, a: (&Buf, usize), b: Option<(&Buf, usize)>, dst: (&Buf, usize), rows: usize, d: usize) {
        let (bb, plain) = match b {
            Some(b) => (b, 0),
            None => ((a.0, 0), 1),
        };
        let p = Params::new(&[rows as u32, d as u32, a.1 as u32, bb.1 as u32, plain, dst.1 as u32]);
        if plain == 1 {
            self.op("col_acc", &[a.0, &self.g.dummy_buf(), dst.0], p, grid(d, 64));
        } else {
            self.op("col_acc", &[a.0, bb.0, dst.0], p, grid(d, 64));
        }
    }

    /// softmax of rows in place; causal rows (n_q per group, first at position t0) stop
    /// at their position
    pub fn softmax(&mut self, x: (&Buf, usize), rows: usize, width: usize, causal: Option<(usize, usize)>) {
        let (c, nq, t0) = match causal {
            Some((nq, t0)) => (1, nq, t0),
            None => (0, 1, 0),
        };
        let p = Params::new(&[rows as u32, width as u32, x.1 as u32, 0, c, nq as u32, t0 as u32]);
        self.op("softmax", &[x.0], p, grid(rows, 8));
    }

    pub fn softmax_back(&mut self, pr: (&Buf, usize), dp: (&Buf, usize), rows: usize, width: usize) {
        let p = Params::new(&[rows as u32, width as u32, pr.1 as u32, dp.1 as u32]);
        self.op("softmax_back", &[pr.0, dp.0], p, grid(rows, 8));
    }

    /// causal scores S[h][t][s] = dot(q, k)·scale for s ≤ t0 + t, else 0
    #[allow(clippy::too_many_arguments)]
    pub fn scores(
        &mut self,
        q: (&Buf, usize, usize),
        k: (&Buf, usize, usize),
        s: (&Buf, usize),
        nq: usize,
        width: usize,
        dh: usize,
        heads: usize,
        t0: usize,
        scale: f32,
        per: usize,
    ) {
        let p = Params::new(&[
            nq as u32,
            width as u32,
            dh as u32,
            heads as u32,
            q.1 as u32,
            q.2 as u32,
            k.1 as u32,
            k.2 as u32,
            s.1 as u32,
            t0 as u32,
            per as u32,
        ])
        .f(&[scale]);
        self.op("scores", &[q.0, k.0, s.0], p, grid(heads * nq * width, 64));
    }

    /// elementwise op (see `ew` in the shader) over `count` items
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ew(
        &mut self,
        op: u32,
        count: usize,
        a: (&Buf, usize),
        b: Option<(&Buf, usize)>,
        y: Option<(&Buf, usize)>,
        f: f32,
        width: usize,
    ) {
        let d = self.g.dummy_buf();
        let (bb, yy) = (b.unwrap_or((&d, 0)), y.unwrap_or((&d, 0)));
        let p = Params::new(&[count as u32, op, a.1 as u32, bb.1 as u32, yy.1 as u32, width as u32]).f(&[f]);
        self.op("ew", &[a.0, bb.0, yy.0], p, grid(count, 64));
    }

    /// y = a + b
    pub fn add(&mut self, a: &Buf, b: &Buf, y: &Buf, count: usize) {
        self.ew(0, count, (a, 0), Some((b, 0)), Some((y, 0)), 0.0, 0);
    }

    /// a += b
    pub fn add_into(&mut self, a: (&Buf, usize), b: (&Buf, usize), count: usize) {
        self.ew(1, count, a, Some(b), None, 0.0, 0);
    }

    /// a *= b elementwise
    pub fn mul_into(&mut self, a: &Buf, b: &Buf, count: usize) {
        self.ew(2, count, (a, 0), Some((b, 0)), None, 0.0, 0);
    }

    /// a *= f
    pub fn scale(&mut self, a: (&Buf, usize), f: f32, count: usize) {
        self.ew(3, count, a, None, None, f, 0);
    }

    /// a = f
    pub fn fill(&mut self, a: (&Buf, usize), f: f32, count: usize) {
        self.ew(4, count, a, None, None, f, 0);
    }

    /// rows of a (count in all) += b, b one row of `width`
    pub fn add_rows(&mut self, a: &Buf, b: &Buf, count: usize, width: usize) {
        self.ew(9, count, (a, 0), Some((b, 0)), None, 0.0, width);
    }

    pub fn swiglu(&mut self, g: &Buf, u: &Buf, s: &Buf, act: &Buf, count: usize) {
        self.op("swiglu", &[g, u, s, act], Params::new(&[count as u32]), grid(count, 64));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn swiglu_back(&mut self, dact: &Buf, g: &Buf, s: &Buf, u: &Buf, dg: &Buf, du: &Buf, count: usize) {
        self.op(
            "swiglu_back",
            &[dact, g, s, u, dg, du],
            Params::new(&[count as u32]),
            grid(count, 64),
        );
    }

    /// rotates rows of 2·half in place; row r is position t0 + (r mod per)
    #[allow(clippy::too_many_arguments)]
    pub fn rope(
        &mut self,
        x: (&Buf, usize),
        cos: &Buf,
        sin: &Buf,
        rows: usize,
        half: usize,
        per: usize,
        t0: usize,
        back: bool,
    ) {
        let p = Params::new(&[rows as u32, half as u32, x.1 as u32, per as u32, t0 as u32, back as u32]);
        self.op("rope", &[x.0, cos, sin], p, grid(rows * half, 64));
    }

    /// dst[d + z·dzs + r·drs + c] = src[s + z·szs + r·srs + c] for c < width
    #[allow(clippy::too_many_arguments)]
    pub fn copy_strided(
        &mut self,
        src: (&Buf, usize, usize, usize),
        dst: (&Buf, usize, usize, usize),
        width: usize,
        rows: usize,
        zs: usize,
    ) {
        let p = Params::new(&[
            width as u32,
            rows as u32,
            zs as u32,
            0,
            src.1 as u32,
            src.2 as u32,
            src.3 as u32,
            0,
            dst.1 as u32,
            dst.2 as u32,
            dst.3 as u32,
        ]);
        self.op("copy", &[src.0, dst.0], p, grid(width * rows * zs, 64));
    }

    // ------------------------------------------------------------ packed windows

    /// a kernel over packed windows: its buffers, then the pack's table at binding 6
    fn op_packed(&mut self, entry: &'static str, bufs: &[&Buf], pk: &Packed, mut p: Params, groups: [u32; 3]) {
        let dummy = self.g.dummy_buf();
        let mut all: Vec<&Buf> = bufs.to_vec();
        while all.len() < 5 {
            all.push(&dummy);
        }
        all.push(pk.table);
        p.u[16] = pk.at as u32;
        p.u[17] = pk.heads as u32;
        p.u[18] = pk.kv as u32;
        self.op(entry, &all, p, groups);
    }

    /// queries, keys and values of every window out of the projected rows `qkv`
    /// [n][(H + 2KV)·dh], each window's heads apart (`copy_strided` per window)
    pub fn split_packed(&mut self, pk: &Packed, qkv: &Buf, q: &Buf, k: &Buf, v: &Buf) {
        let w3 = (pk.heads + 2 * pk.kv) * pk.dh;
        let p = Params::new(&[pk.n as u32, w3 as u32, pk.dh as u32]);
        self.op_packed("split_v", &[qkv, q, k, v], pk, p, grid(pk.n * w3, 64));
    }

    /// rotary positions on every window's [heads][pos][dh] rows (`rope` per window)
    pub fn rope_packed(&mut self, pk: &Packed, x: &Buf, cos: &Buf, sin: &Buf, heads: usize) {
        let half = pk.dh / 2;
        let p = Params::new(&[pk.n as u32, half as u32, heads as u32]);
        self.op_packed("rope_v", &[x, cos, sin], pk, p, grid(pk.n * heads * half, 64));
    }

    /// every window's scaled scores (`gemm` then `scale`, or causal `scores`)
    pub fn qk_packed(&mut self, pk: &Packed, q: &Buf, k: &Buf, s: &Buf, causal: bool, scale: f32) {
        let p = Params::new(&[
            pk.n as u32,
            causal as u32,
            pk.dh as u32,
            pk.longest as u32,
            (pk.heads / pk.kv) as u32,
        ])
        .f(&[scale]);
        self.op_packed("qk_v", &[q, k, s], pk, p, grid(pk.n * pk.heads * pk.longest, 64));
    }

    /// softmax of every window's score rows (`softmax` per window)
    pub fn softmax_packed(&mut self, pk: &Packed, s: &Buf, causal: bool) {
        let p = Params::new(&[pk.n as u32, causal as u32]);
        self.op_packed("softmax_v", &[s], pk, p, grid(pk.n * pk.heads, 8));
    }

    /// every window's probabilities times its values into the packed rows of `ctx`
    /// [n][H·dh] (`gemm` per window)
    pub fn pv_packed(&mut self, pk: &Packed, s: &Buf, v: &Buf, ctx: &Buf, causal: bool) {
        let p = Params::new(&[pk.n as u32, causal as u32, pk.dh as u32, 0, (pk.heads / pk.kv) as u32]);
        self.op_packed("pv_v", &[s, v, ctx], pk, p, grid(pk.n * pk.heads * pk.dh, 64));
    }
}

/// windows packed one after another, as the packed attention kernels see them: `n` rows
/// in all, the longest window `longest` rows, and the table (at [row] the row's window;
/// from `at`, per window its first row, length and the sum of the squared lengths
/// before it, all as f32 integers)
pub struct Packed<'a> {
    pub table: &'a Buf,
    pub at: usize,
    pub n: usize,
    pub longest: usize,
    pub heads: usize,
    pub kv: usize,
    pub dh: usize,
}

/// the table of `Packed` for windows of these lengths
pub fn packed_table(lens: &[usize]) -> Vec<f32> {
    let n: usize = lens.iter().sum();
    let mut t = Vec::with_capacity(n + 3 * lens.len());
    for (w, &m) in lens.iter().enumerate() {
        t.extend(std::iter::repeat_n(w as f32, m));
    }
    let (mut s, mut sq) = (0usize, 0usize);
    for &m in lens {
        assert!(s < 1 << 24 && sq < 1 << 24, "a pack's table holds integers below 2^24");
        t.extend([s as f32, m as f32, sq as f32]);
        s += m;
        sq += m * m;
    }
    t
}

impl Gpu {
    fn dummy_buf(&self) -> Buf {
        Buf {
            b: self.dummy.clone(),
            len: 4,
        }
    }
}

/// an operand of `Run::gemm`: offset, two strides (A: row and p; B: p and column; y: row)
/// and the stride between batched products
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub buf: &'a Buf,
    pub off: usize,
    pub s0: usize,
    pub s1: usize,
    pub zs: usize,
    /// batched products: product z reads this operand's slice z / per (B only)
    pub per: usize,
}

impl<'a> View<'a> {
    /// a row-major matrix with rows of `cols`
    pub fn rows(buf: &'a Buf, cols: usize) -> View<'a> {
        View {
            buf,
            off: 0,
            s0: cols,
            s1: 1,
            zs: 0,
            per: 1,
        }
    }

    /// the transpose of a row-major matrix with rows of `cols`
    pub fn t(buf: &'a Buf, cols: usize) -> View<'a> {
        View {
            buf,
            off: 0,
            s0: 1,
            s1: cols,
            zs: 0,
            per: 1,
        }
    }

    pub fn at(mut self, off: usize) -> View<'a> {
        self.off = off;
        self
    }

    pub fn batch(mut self, zs: usize) -> View<'a> {
        self.zs = zs;
        self
    }

    pub fn per(mut self, per: usize) -> View<'a> {
        self.per = per;
        self
    }

    /// a strided view: offset, two strides, batch stride
    pub fn of(buf: &'a Buf, off: usize, s0: usize, s1: usize, zs: usize) -> View<'a> {
        View {
            buf,
            off,
            s0,
            s1,
            zs,
            per: 1,
        }
    }
}
