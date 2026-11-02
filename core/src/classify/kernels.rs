//! The numeric kernels, each with a portable path and (on x86_64, picked at run time) an
//! explicit AVX2 path that gives the same bits. Two summation rules cover everything:
//!
//! - **matrix products** add their products one by one in ascending order of the
//!   contraction index, onto the output's current value: `y = ((y + a₀b₀) + a₁b₁) + …`.
//!   Each output element is its own chain, so the kernels vectorise *across* outputs
//!   (register tiles of 4 rows × 16 columns) without reordering any chain, and threads
//!   split output rows.
//! - **vector reductions** (`sum`, `dot`) keep 8 lanes: element i goes to lane i mod 8 in
//!   order, then the lanes fold as `((l0+l4)+(l1+l5)) + ((l2+l6)+(l3+l7))`.
//!
//! Every multiply is rounded before its add (no fused multiply-add on any path).
//!
//! This module (with `math`) is the backend boundary: the model, training and growth
//! code reach the numbers only through `gemm`/`matmul`, `transpose`, `dot`, `sum`, `axpy`,
//! `norm_fwd`/`norm_back`, `softmax_rows`/`softmax_back_rows`, `exp_slice` and
//! `sigmoid_slice`. A GPU backend (wgpu/WebGPU compute shaders) would implement the same
//! list. To stay bit-exact there it must keep each output's chain in order (one thread
//! per output element, looping over p; no split-k or tree reductions), keep the 8-lane
//! fold for `dot`/`sum`, and stop the shader compiler fusing multiply-adds (WGSL gives no
//! guarantee; storing the product through a bitcast or an integer round trip before the
//! add is the usual guard). exp/ln must be the polynomials of `math`, never the
//! built-ins.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::Instant;

use super::pool::{Pool, Shards};

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

static LEVEL: AtomicU8 = AtomicU8::new(0);

/// which kernels run: 1 portable, 2 AVX2
fn level() -> u8 {
    match LEVEL.load(Ordering::Relaxed) {
        0 => {
            let l = detect();
            LEVEL.store(l, Ordering::Relaxed);
            l
        }
        l => l,
    }
}

fn detect() -> u8 {
    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("avx2") {
        return 2;
    }
    1
}

/// turns the explicit SIMD paths off (true) or back to what the CPU offers (false);
/// for benchmarks and the tests that hold both paths to the same bits
pub fn force_portable(on: bool) {
    LEVEL.store(if on { 1 } else { detect() }, Ordering::Relaxed);
}

/// "avx2" or "portable"
pub fn simd_name() -> &'static str {
    if level() == 2 { "avx2" } else { "portable" }
}

// ---------------------------------------------------------------- profiling

/// kernel families the profiler times
pub const OPS: [&str; 4] = ["gemm", "transpose", "norm", "softmax"];
static PROFILE: AtomicBool = AtomicBool::new(false);
static NANOS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static CALLS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

/// starts (and zeroes) or stops timing the kernels; times add up over all threads
pub fn profile(on: bool) {
    if on {
        NANOS.iter().chain(&CALLS).for_each(|a| a.store(0, Ordering::Relaxed));
    }
    PROFILE.store(on, Ordering::Relaxed);
}

/// (kernel family, seconds, calls) since `profile(true)`
pub fn profile_report() -> Vec<(&'static str, f64, u64)> {
    OPS.iter()
        .enumerate()
        .map(|(i, n)| {
            (
                *n,
                NANOS[i].load(Ordering::Relaxed) as f64 * 1e-9,
                CALLS[i].load(Ordering::Relaxed),
            )
        })
        .collect()
}

#[inline(always)]
fn timed<T>(op: usize, f: impl FnOnce() -> T) -> T {
    if !PROFILE.load(Ordering::Relaxed) {
        return f();
    }
    let t = Instant::now();
    let r = f();
    NANOS[op].fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
    CALLS[op].fetch_add(1, Ordering::Relaxed);
    r
}

// ---------------------------------------------------------------- scratch buffers

thread_local! {
    /// buffers given back on this thread, for reuse: a forward and backward pass allocate
    /// the same sizes over and over, and fresh pages from the system cost more than the
    /// arithmetic that fills them
    static FREE: std::cell::RefCell<Vec<Vec<f32>>> = const { std::cell::RefCell::new(Vec::new()) };
}

const KEEP_MIN: usize = 1024;
const KEEP_MAX: usize = 512;

/// a zeroed buffer of n floats, reusing one this thread gave back when it can
pub fn zeros(n: usize) -> Vec<f32> {
    if n >= KEEP_MIN {
        let found = FREE.with(|f| {
            let mut f = f.borrow_mut();
            let best = f
                .iter()
                .enumerate()
                .filter(|(_, v)| v.capacity() >= n)
                .min_by_key(|(_, v)| v.capacity())
                .map(|(i, _)| i);
            best.map(|i| f.swap_remove(i))
        });
        if let Some(mut v) = found {
            v.clear();
            v.resize(n, 0.0);
            return v;
        }
    }
    vec![0.0; n]
}

/// gives a buffer back for `zeros` to reuse
pub fn recycle(v: Vec<f32>) {
    if v.capacity() >= KEEP_MIN {
        FREE.with(|f| {
            let mut f = f.borrow_mut();
            if f.len() < KEEP_MAX {
                f.push(v);
            }
        });
    }
}

// ---------------------------------------------------------------- reductions

#[inline(always)]
pub fn fold8(l: &[f32; 8]) -> f32 {
    ((l[0] + l[4]) + (l[1] + l[5])) + ((l[2] + l[6]) + (l[3] + l[7]))
}

/// Σ aᵢbᵢ by the 8-lane rule
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    #[cfg(target_arch = "x86_64")]
    if level() == 2 {
        // SAFETY: AVX2 was detected
        return unsafe { dot_avx2(a, b) };
    }
    dot_portable(a, b)
}

fn dot_portable(a: &[f32], b: &[f32]) -> f32 {
    let mut l = [0f32; 8];
    let ((ca, ra), (cb, rb)) = (a.as_chunks::<8>(), b.as_chunks::<8>());
    for (x, y) in ca.iter().zip(cb) {
        for i in 0..8 {
            l[i] += x[i] * y[i];
        }
    }
    for (i, (x, y)) in ra.iter().zip(rb).enumerate() {
        l[i] += x * y;
    }
    fold8(&l)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn dot_avx2(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() / 8 * 8;
    let mut acc = _mm256_setzero_ps();
    let mut i = 0;
    while i < n {
        let x = unsafe { _mm256_loadu_ps(a.as_ptr().add(i)) };
        let y = unsafe { _mm256_loadu_ps(b.as_ptr().add(i)) };
        acc = _mm256_add_ps(acc, _mm256_mul_ps(x, y));
        i += 8;
    }
    let mut l = [0f32; 8];
    unsafe { _mm256_storeu_ps(l.as_mut_ptr(), acc) };
    for (j, (x, y)) in a[n..].iter().zip(&b[n..]).enumerate() {
        l[j] += x * y;
    }
    fold8(&l)
}

/// Σ aᵢ by the 8-lane rule
pub fn sum(a: &[f32]) -> f32 {
    let mut l = [0f32; 8];
    let (c, r) = a.as_chunks::<8>();
    for x in c {
        for i in 0..8 {
            l[i] += x[i];
        }
    }
    for (i, x) in r.iter().enumerate() {
        l[i] += x;
    }
    fold8(&l)
}

/// e^x of each element in place (`math::exp`); the AVX2 build of the same code
/// vectorises it 8 wide
pub fn exp_slice(x: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    if level() == 2 {
        // SAFETY: AVX2 was detected
        unsafe { exp_slice_avx2(x) };
        return;
    }
    exp_slice_portable(x)
}

#[inline(always)]
fn exp_slice_portable(x: &mut [f32]) {
    for v in x {
        *v = super::math::exp(*v);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn exp_slice_avx2(x: &mut [f32]) {
    exp_slice_portable(x)
}

/// the logistic function of each element (`math::sigmoid`) into `out`
pub fn sigmoid_slice(x: &[f32], out: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    if level() == 2 {
        // SAFETY: AVX2 was detected
        unsafe { sigmoid_slice_avx2(x, out) };
        return;
    }
    sigmoid_slice_portable(x, out)
}

#[inline(always)]
fn sigmoid_slice_portable(x: &[f32], out: &mut [f32]) {
    for (o, v) in out.iter_mut().zip(x) {
        *o = super::math::sigmoid(*v);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn sigmoid_slice_avx2(x: &[f32], out: &mut [f32]) {
    sigmoid_slice_portable(x, out)
}

/// y += a·x, elementwise
pub fn axpy(y: &mut [f32], a: f32, x: &[f32]) {
    debug_assert_eq!(y.len(), x.len());
    for (y, x) in y.iter_mut().zip(x) {
        *y += a * x;
    }
}

// ---------------------------------------------------------------- matrix products

/// operand A of a product, read as A[r][p] = data[r·rs + p·ps], so one kernel serves
/// both A and Aᵀ
#[derive(Clone, Copy)]
pub struct Strided<'a> {
    pub data: &'a [f32],
    pub rs: usize,
    pub ps: usize,
}

impl Strided<'_> {
    #[inline(always)]
    fn at(&self, r: usize, p: usize) -> f32 {
        self.data[r * self.rs + p * self.ps]
    }
}

/// a row-major [rows × cols] A
pub fn rows(data: &[f32], cols: usize) -> Strided<'_> {
    Strided { data, rs: cols, ps: 1 }
}

/// Aᵀ of a row-major A whose rows are `cols` long: Aᵀ[r][p] = A[p][r]
pub fn transposed(data: &[f32], cols: usize) -> Strided<'_> {
    Strided { data, rs: 1, ps: cols }
}

/// products above this many multiply-adds split their rows across a pool
pub const PAR_MIN: usize = 1 << 21;

/// y[n×m] += A[n×k] · b[k×m], every chain in ascending p (see the module notes);
/// y and b row-major with rows of m
pub fn gemm(y: &mut [f32], a: Strided, b: &[f32], n: usize, k: usize, m: usize, pool: Option<&Pool>) {
    debug_assert!(y.len() >= n * m && b.len() >= k * m);
    match pool {
        Some(pool) if pool.threads() > 1 && n * k * m >= PAR_MIN && n >= 8 => {
            // any split gives the same bits; whole tiles keep the kernels busy
            let per = (n.div_ceil(pool.threads() * 2)).div_ceil(MR) * MR;
            let parts = n.div_ceil(per);
            let sh = Shards::new(&mut y[..n * m]);
            pool.run(parts, &|t| {
                let r0 = t * per;
                let rn = per.min(n - r0);
                // SAFETY: each task owns rows r0..r0+rn
                let yp = unsafe { sh.part(r0 * m, rn * m) };
                let ap = Strided {
                    data: &a.data[r0 * a.rs..],
                    ..a
                };
                gemm_serial(yp, ap, b, rn, k, m);
            });
        }
        _ => gemm_serial(y, a, b, n, k, m),
    }
}

fn gemm_serial(y: &mut [f32], a: Strided, b: &[f32], n: usize, k: usize, m: usize) {
    timed(0, || {
        #[cfg(target_arch = "x86_64")]
        if level() == 2 {
            // SAFETY: AVX2 was detected
            unsafe { gemm_avx2(y, a, b, n, k, m) };
            return;
        }
        gemm_portable(y, a, b, n, k, m)
    })
}

fn gemm_portable(y: &mut [f32], a: Strided, b: &[f32], n: usize, k: usize, m: usize) {
    for r in 0..n {
        let yr = &mut y[r * m..(r + 1) * m];
        for p in 0..k {
            axpy(yr, a.at(r, p), &b[p * m..(p + 1) * m]);
        }
    }
}

/// rows per register tile: 6 rows × 16 columns is 12 accumulators, 2 loads of b and a
/// broadcast, within the 16 AVX registers
const MR: usize = 6;

/// A packed for the register tiles: tile t holds rows t·MR.., p-major, zero padded:
/// `out[(t·k + p)·MR + i] = A[t·MR + i][p]`
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn pack(a: Strided, n: usize, k: usize) -> Vec<f32> {
    let tiles = n.div_ceil(MR);
    let mut out = vec![0f32; tiles * k * MR];
    for t in 0..tiles {
        let panel = &mut out[t * k * MR..(t + 1) * k * MR];
        for i in 0..MR.min(n - t * MR) {
            let r = t * MR + i;
            if a.ps == 1 {
                let row = &a.data[r * a.rs..r * a.rs + k];
                for (p, &v) in row.iter().enumerate() {
                    panel[p * MR + i] = v;
                }
            } else {
                for p in 0..k {
                    panel[p * MR + i] = a.at(r, p);
                }
            }
        }
    }
    out
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn gemm_avx2(y: &mut [f32], a: Strided, b: &[f32], n: usize, k: usize, m: usize) {
    let yp = y.as_mut_ptr();
    let bp = b.as_ptr();
    let c16 = m / 16 * 16;
    let c8 = m / 8 * 8;
    let pa = pack(a, n, k);
    let tiles = n.div_ceil(MR);
    // column strips outside, row tiles inside: a strip of b (k × 16) stays in L1 while
    // every packed tile of A streams past it
    let mut c = 0;
    while c < c16 {
        for t in 0..tiles {
            let (r, rows) = (t * MR, MR.min(n - t * MR));
            let pp = unsafe { pa.as_ptr().add(t * k * MR) };
            unsafe {
                if rows == MR {
                    let mut acc = [[_mm256_setzero_ps(); 2]; MR];
                    for (i, row) in acc.iter_mut().enumerate() {
                        row[0] = _mm256_loadu_ps(yp.add((r + i) * m + c));
                        row[1] = _mm256_loadu_ps(yp.add((r + i) * m + c + 8));
                    }
                    for p in 0..k {
                        let b0 = _mm256_loadu_ps(bp.add(p * m + c));
                        let b1 = _mm256_loadu_ps(bp.add(p * m + c + 8));
                        for (i, row) in acc.iter_mut().enumerate() {
                            let av = _mm256_broadcast_ss(&*pp.add(p * MR + i));
                            row[0] = _mm256_add_ps(row[0], _mm256_mul_ps(av, b0));
                            row[1] = _mm256_add_ps(row[1], _mm256_mul_ps(av, b1));
                        }
                    }
                    for (i, row) in acc.iter().enumerate() {
                        _mm256_storeu_ps(yp.add((r + i) * m + c), row[0]);
                        _mm256_storeu_ps(yp.add((r + i) * m + c + 8), row[1]);
                    }
                } else {
                    for i in 0..rows {
                        let mut v0 = _mm256_loadu_ps(yp.add((r + i) * m + c));
                        let mut v1 = _mm256_loadu_ps(yp.add((r + i) * m + c + 8));
                        for p in 0..k {
                            let av = _mm256_broadcast_ss(&*pp.add(p * MR + i));
                            v0 = _mm256_add_ps(v0, _mm256_mul_ps(av, _mm256_loadu_ps(bp.add(p * m + c))));
                            v1 = _mm256_add_ps(v1, _mm256_mul_ps(av, _mm256_loadu_ps(bp.add(p * m + c + 8))));
                        }
                        _mm256_storeu_ps(yp.add((r + i) * m + c), v0);
                        _mm256_storeu_ps(yp.add((r + i) * m + c + 8), v1);
                    }
                }
            }
        }
        c += 16;
    }
    if c < c8 {
        for t in 0..tiles {
            let (r, rows) = (t * MR, MR.min(n - t * MR));
            let pp = unsafe { pa.as_ptr().add(t * k * MR) };
            unsafe {
                let mut acc = [_mm256_setzero_ps(); MR];
                for (i, v) in acc.iter_mut().enumerate().take(rows) {
                    *v = _mm256_loadu_ps(yp.add((r + i) * m + c));
                }
                for p in 0..k {
                    let b0 = _mm256_loadu_ps(bp.add(p * m + c));
                    for (i, v) in acc.iter_mut().enumerate().take(rows) {
                        let av = _mm256_broadcast_ss(&*pp.add(p * MR + i));
                        *v = _mm256_add_ps(*v, _mm256_mul_ps(av, b0));
                    }
                }
                for (i, v) in acc.iter().enumerate().take(rows) {
                    _mm256_storeu_ps(yp.add((r + i) * m + c), *v);
                }
            }
        }
        c += 8;
    }
    if c < m {
        for r in 0..n {
            tail_cols(y, a, b, r, c, k, m);
        }
    }
}

/// columns c0.. of row r, scalar chains
#[inline(always)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn tail_cols(y: &mut [f32], a: Strided, b: &[f32], r: usize, c0: usize, k: usize, m: usize) {
    for c in c0..m {
        let mut v = y[r * m + c];
        for p in 0..k {
            v += a.at(r, p) * b[p * m + c];
        }
        y[r * m + c] = v;
    }
}

/// y = A · b into a fresh buffer
pub fn matmul(a: Strided, b: &[f32], n: usize, k: usize, m: usize, pool: Option<&Pool>) -> Vec<f32> {
    let mut y = zeros(n * m);
    gemm(&mut y, a, b, n, k, m, pool);
    y
}

/// the transpose of a row-major [rows × cols] matrix
pub fn transpose(a: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    timed(1, || transpose_inner(a, rows, cols))
}

fn transpose_inner(a: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    let mut t = zeros(rows * cols);
    const B: usize = 16;
    for r0 in (0..rows).step_by(B) {
        for c0 in (0..cols).step_by(B) {
            for r in r0..(r0 + B).min(rows) {
                for c in c0..(c0 + B).min(cols) {
                    t[c * rows + r] = a[r * cols + c];
                }
            }
        }
    }
    t
}

// ---------------------------------------------------------------- row kernels

/// RMSNorm, row by row: rstd = 1/√(dot(x, x)/d + eps), x̂ = x·rstd, y = x̂·g
pub struct Norm {
    pub xhat: Vec<f32>,
    pub rstd: Vec<f32>,
}

pub fn norm_fwd(x: &[f32], g: &[f32], d: usize, eps: f32) -> (Vec<f32>, Norm) {
    timed(2, || norm_fwd_inner(x, g, d, eps))
}

fn norm_fwd_inner(x: &[f32], g: &[f32], d: usize, eps: f32) -> (Vec<f32>, Norm) {
    let n = x.len() / d;
    let mut y = zeros(n * d);
    let mut xhat = zeros(n * d);
    let mut rstd = vec![0f32; n];
    for r in 0..n {
        let xr = &x[r * d..(r + 1) * d];
        let s = 1.0 / (dot(xr, xr) / d as f32 + eps).sqrt();
        rstd[r] = s;
        for i in 0..d {
            let h = xr[i] * s;
            xhat[r * d + i] = h;
            y[r * d + i] = h * g[i];
        }
    }
    (y, Norm { xhat, rstd })
}

/// adds into dx; dg += Σ rows dy·x̂ in row order;
/// dx += rstd·(dx̂ − x̂·(dot(dx̂, x̂)/d)) with dx̂ = dy·g
pub fn norm_back(dy: &[f32], nm: &Norm, g: &[f32], d: usize, dx: &mut [f32], dg: &mut [f32]) {
    timed(2, || norm_back_inner(dy, nm, g, d, dx, dg))
}

fn norm_back_inner(dy: &[f32], nm: &Norm, g: &[f32], d: usize, dx: &mut [f32], dg: &mut [f32]) {
    let n = dy.len() / d;
    let mut dh = vec![0f32; d];
    for r in 0..n {
        let (dyr, xh) = (&dy[r * d..(r + 1) * d], &nm.xhat[r * d..(r + 1) * d]);
        for i in 0..d {
            dh[i] = dyr[i] * g[i];
            dg[i] += dyr[i] * xh[i];
        }
        let c = dot(&dh, xh) / d as f32;
        let s = nm.rstd[r];
        let dxr = &mut dx[r * d..(r + 1) * d];
        for i in 0..d {
            dxr[i] += s * (dh[i] - xh[i] * c);
        }
    }
}

/// softmax of each row in place: e = exp(x − max), p = e / sum(e)
pub fn softmax_rows(x: &mut [f32], width: usize) {
    timed(3, || softmax_rows_inner(x, width))
}

fn softmax_rows_inner(x: &mut [f32], width: usize) {
    for row in x.chunks_exact_mut(width) {
        let m = row.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        for v in row.iter_mut() {
            *v -= m;
        }
        exp_slice(row);
        let s = sum(row);
        for v in row.iter_mut() {
            *v /= s;
        }
    }
}

/// softmax backward per row: dx = p·(dp − dot(p, dp)), written over dp
pub fn softmax_back_rows(p: &[f32], dp: &mut [f32], width: usize) {
    timed(3, || softmax_back_inner(p, dp, width))
}

fn softmax_back_inner(p: &[f32], dp: &mut [f32], width: usize) {
    for (pr, dr) in p.chunks_exact(width).zip(dp.chunks_exact_mut(width)) {
        let c = dot(pr, dr);
        for (d, &q) in dr.iter_mut().zip(pr) {
            *d = q * (*d - c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::math::Rng;

    fn rand(n: usize, seed: u64) -> Vec<f32> {
        let mut r = Rng::new(seed);
        (0..n).map(|_| r.symmetric(1.0)).collect()
    }

    fn bits(v: &[f32]) -> Vec<u32> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    /// the reference: each output's chain written out plainly
    fn naive(a: Strided, b: &[f32], y0: &[f32], n: usize, k: usize, m: usize) -> Vec<f32> {
        let mut y = y0.to_vec();
        for r in 0..n {
            for c in 0..m {
                let mut v = y[r * m + c];
                for p in 0..k {
                    v += a.at(r, p) * b[p * m + c];
                }
                y[r * m + c] = v;
            }
        }
        y
    }

    #[test]
    fn simd_and_portable_paths_agree_bit_for_bit() {
        let pool = Pool::new(3);
        for &(n, k, m) in &[
            (1, 1, 1),
            (5, 7, 3),
            (9, 33, 40),
            (16, 64, 48),
            (37, 19, 71),
            (130, 129, 131),
        ] {
            let a = rand(n * k, 1);
            let b = rand(k * m, 2);
            let y0 = rand(n * m, 3);
            let at = transpose(&a, n, k);
            let want = naive(rows(&a, k), &b, &y0, n, k, m);
            for portable in [true, false] {
                force_portable(portable);
                for p in [None, Some(&pool)] {
                    let mut y = y0.clone();
                    gemm(&mut y, rows(&a, k), &b, n, k, m, p);
                    assert_eq!(bits(&y), bits(&want), "{n}x{k}x{m} portable={portable}");
                    let mut y = y0.clone();
                    gemm(&mut y, transposed(&at, n), &b, n, k, m, p);
                    assert_eq!(bits(&y), bits(&want), "transposed {n}x{k}x{m}");
                }
                let x = rand(k * 3 + 5, 4);
                let z = rand(k * 3 + 5, 5);
                assert_eq!(dot(&x, &z).to_bits(), dot_portable(&x, &z).to_bits());
            }
            force_portable(false);
        }
        // elementwise kernels
        let mut x = rand(1003, 11);
        x.iter_mut().for_each(|v| *v *= 100.0);
        x.extend([f32::NAN, -1e9, 1e9, 0.0, -0.0, 88.7, -86.9]);
        let mut p = vec![0f32; x.len()];
        let mut q = p.clone();
        force_portable(true);
        sigmoid_slice(&x, &mut p);
        let mut e1 = x.clone();
        exp_slice(&mut e1);
        force_portable(false);
        sigmoid_slice(&x, &mut q);
        let mut e2 = x.clone();
        exp_slice(&mut e2);
        assert_eq!(bits(&p), bits(&q));
        assert_eq!(bits(&e1), bits(&e2));
        let scalar: Vec<f32> = x.iter().map(|&v| crate::classify::math::exp(v)).collect();
        assert_eq!(bits(&e1), bits(&scalar));
        // a product big enough to go parallel
        let (n, k, m) = (256, 128, 96);
        let (a, b) = (rand(n * k, 6), rand(k * m, 7));
        let one = matmul(rows(&a, k), &b, n, k, m, None);
        assert_eq!(bits(&one), bits(&matmul(rows(&a, k), &b, n, k, m, Some(&pool))));
    }

    /// d/dx of Σ c·f(x) by central differences
    fn numeric(f: &dyn Fn(&[f32]) -> Vec<f32>, x: &[f32], c: &[f32], i: usize) -> f32 {
        let e = 1e-2;
        let (mut a, mut b) = (x.to_vec(), x.to_vec());
        a[i] += e;
        b[i] -= e;
        (dot(&f(&a), c) - dot(&f(&b), c)) / (2.0 * e)
    }

    #[test]
    fn row_kernel_gradients() {
        let (n, d) = (3, 11);
        let x = rand(n * d, 8);
        let g = rand(d, 9);
        let c = rand(n * d, 10);
        // RMSNorm: input and gains
        let (_, nm) = norm_fwd(&x, &g, d, 1e-5);
        let mut dx = vec![0f32; n * d];
        let mut dg = vec![0f32; d];
        norm_back(&c, &nm, &g, d, &mut dx, &mut dg);
        for (i, &a) in dx.iter().enumerate() {
            let num = numeric(&|v| norm_fwd(v, &g, d, 1e-5).0, &x, &c, i);
            assert!((num - a).abs() < 2e-3, "norm dx {i}: {num} {a}");
        }
        for (i, &a) in dg.iter().enumerate() {
            let num = numeric(&|gv| norm_fwd(&x, gv, d, 1e-5).0, &g, &c, i);
            assert!((num - a).abs() < 2e-3, "norm dg {i}");
        }
        // softmax rows
        let sm = |v: &[f32]| {
            let mut p = v.to_vec();
            softmax_rows(&mut p, d);
            p
        };
        let p = sm(&x);
        let mut dp = c.clone();
        softmax_back_rows(&p, &mut dp, d);
        for (i, &a) in dp.iter().enumerate() {
            let num = numeric(&sm, &x, &c, i);
            assert!((num - a).abs() < 1e-3, "softmax {i}");
        }
        // SiLU-gated unit (g·σ(g))·u, as the model's backward writes it
        for &gv in &[-3.0f32, -0.5, 0.0, 0.7, 4.0] {
            let s = crate::classify::math::sigmoid(gv);
            let ana = s * (1.0 + gv * (1.0 - s));
            let f = |v: f32| v * crate::classify::math::sigmoid(v);
            let num = (f(gv + 1e-2) - f(gv - 1e-2)) / 2e-2;
            assert!((num - ana).abs() < 1e-3);
        }
    }
}
