//! The exactness probe: every kernel on random data, on the device and on the CPU
//! reference (`kernels`, `math`, `model::Rope`), compared bit for bit. A device that
//! matches everywhere in `Precision::Exact` gives the CPU's results for whole models
//! too (the model code only chains these kernels); one that does not is still usable
//! within the reported tolerance.

use super::{Buf, Gpu, Precision, View};
use crate::classify::kernels::{self as k, rows, transposed};
use crate::classify::math::Rng;
use crate::classify::model::Rope;

/// one kernel's comparison
#[derive(Clone, Debug)]
pub struct Check {
    pub kernel: &'static str,
    pub values: usize,
    /// values whose bits differ
    pub differ: usize,
    /// the largest difference in units in the last place
    pub max_ulps: u64,
    /// the largest |gpu − cpu| / max(|cpu|, 1e-6)
    pub max_rel: f32,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub device: String,
    pub precision: Precision,
    pub checks: Vec<Check>,
}

impl Report {
    /// every kernel matched bit for bit
    pub fn exact(&self) -> bool {
        self.checks.iter().all(|c| c.differ == 0)
    }

    pub fn max_rel(&self) -> f32 {
        self.checks.iter().map(|c| c.max_rel).fold(0.0, f32::max)
    }

    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} {:?}: {}\n",
            self.device,
            self.precision,
            if self.exact() {
                "bit-exact on every kernel".to_string()
            } else {
                format!("not bit-exact (largest relative difference {:.2e})", self.max_rel())
            }
        );
        for c in &self.checks {
            s += &format!(
                "  {:<14} {:>7} values  {:>6} differ  max {:>4} ulp  rel {:.1e}\n",
                c.kernel, c.values, c.differ, c.max_ulps, c.max_rel
            );
        }
        s
    }
}

fn ulps(a: f32, b: f32) -> u64 {
    if a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()) {
        return 0;
    }
    let key = |x: f32| {
        let b = x.to_bits() as i64;
        if b < 0x8000_0000 { b } else { 0x8000_0000 - b }
    };
    (key(a) - key(b)).unsigned_abs()
}

/// compares device values against the reference
pub fn compare(kernel: &'static str, gpu: &[f32], cpu: &[f32]) -> Check {
    assert_eq!(gpu.len(), cpu.len(), "{kernel}: lengths");
    let mut c = Check {
        kernel,
        values: cpu.len(),
        differ: 0,
        max_ulps: 0,
        max_rel: 0.0,
    };
    for (&g, &r) in gpu.iter().zip(cpu) {
        if g.to_bits() != r.to_bits() {
            c.differ += 1;
            c.max_ulps = c.max_ulps.max(ulps(g, r));
            let rel = (g - r).abs() / r.abs().max(1e-6);
            c.max_rel = c.max_rel.max(if rel.is_nan() { f32::INFINITY } else { rel });
        }
    }
    c
}

fn rand(n: usize, seed: u64, scale: f32) -> Vec<f32> {
    let mut r = Rng::new(seed);
    (0..n).map(|_| r.symmetric(scale)).collect()
}

/// runs the probe on a device in one precision
pub fn probe(g: &Gpu, prec: Precision) -> Report {
    let mut checks = Vec::new();
    let up = |v: &[f32]| g.upload(v);
    let read = |b: &Buf| g.read(b);

    // products: tiled and per-output paths, A plain and transposed, fresh and accumulating
    let (mut gy, mut cy) = (Vec::new(), Vec::new());
    for (i, &(n, kk, m)) in [
        (1, 64, 300),
        (5, 7, 3),
        (16, 64, 48),
        (37, 19, 71),
        (130, 129, 131),
        (64, 256, 96),
    ]
    .iter()
    .enumerate()
    {
        let a = rand(n * kk, 10 + i as u64, 1.0);
        let b = rand(kk * m, 20 + i as u64, 1.0);
        let y0 = rand(n * m, 30 + i as u64, 1.0);
        let at = k::transpose(&a, n, kk);
        let (ba, bat, bb) = (up(&a), up(&at), up(&b));
        for (acc, trans) in [(false, false), (true, false), (true, true)] {
            let by = up(&y0);
            let mut r = g.run(prec);
            let av = if trans { View::t(&bat, n) } else { View::rows(&ba, kk) };
            r.gemm(av, View::rows(&bb, m), View::rows(&by, m), n, kk, m, 1, acc, None);
            r.submit();
            gy.extend(read(&by));
            let mut want = if acc { y0.clone() } else { vec![0.0; n * m] };
            let sa = if trans { transposed(&at, n) } else { rows(&a, kk) };
            k::gemm(&mut want, sa, &b, n, kk, m, None);
            cy.extend(want);
        }
    }
    checks.push(compare("gemm", &gy, &cy));

    // dot and sum (a dot with ones) by the 8-lane rule
    let (nr, len) = (40, 203);
    let a = rand(nr * len, 40, 1.0);
    let b = rand(nr * len, 41, 1.0);
    let ones = vec![1.0; nr * len];
    let (ba, bb, bo, out) = (up(&a), up(&b), up(&ones), g.zeros(nr * 2));
    let mut r = g.run(prec);
    let p = super::Params::new(&[nr as u32, len as u32]);
    r.op("dot_rows", &[&ba, &bb, &out], p, super::grid(nr, 64));
    let out2 = g.zeros(nr);
    r.op("dot_rows", &[&ba, &bo, &out2], p, super::grid(nr, 64));
    r.submit();
    let want: Vec<f32> = (0..nr)
        .map(|i| k::dot(&a[i * len..(i + 1) * len], &b[i * len..(i + 1) * len]))
        .collect();
    checks.push(compare("dot", &read(&out)[..nr], &want));
    let want: Vec<f32> = (0..nr).map(|i| k::sum(&a[i * len..(i + 1) * len])).collect();
    checks.push(compare("sum", &read(&out2), &want));

    // RMSNorm forward and backward
    let (nr, d) = (37, 133);
    let x = rand(nr * d, 50, 2.0);
    let gn = rand(d, 51, 1.0);
    let dy = rand(nr * d, 52, 1.0);
    let dx0 = rand(nr * d, 53, 1.0);
    let dg0 = rand(d, 54, 1.0);
    let (bx, bg, by, bh, bs) = (up(&x), up(&gn), g.zeros(nr * d), g.zeros(nr * d), g.zeros(nr));
    let (bdy, bdx, bdg) = (up(&dy), up(&dx0), up(&dg0));
    let mut r = g.run(prec);
    r.norm_fwd((&bx, 0), &bg, (&by, 0), Some(((&bh, 0), (&bs, 0))), nr, d, 1e-5);
    r.norm_back((&bdy, 0), (&bh, 0), (&bs, 0), &bg, (&bdx, 0), (&bdg, 0), nr, d);
    r.submit();
    let (cy, nm) = k::norm_fwd(&x, &gn, d, 1e-5);
    let mut gv = read(&by);
    gv.extend(read(&bh));
    gv.extend(read(&bs));
    let mut cv = cy.clone();
    cv.extend(&nm.xhat);
    cv.extend(&nm.rstd);
    checks.push(compare("norm", &gv, &cv));
    let (mut cdx, mut cdg) = (dx0.clone(), dg0.clone());
    // backward from the device's own forward, so the comparison isolates the backward
    let gnm = k::Norm {
        xhat: read(&bh),
        rstd: read(&bs),
    };
    k::norm_back(&dy, &gnm, &gn, d, &mut cdx, &mut cdg);
    let mut gv = read(&bdx);
    gv.extend(read(&bdg));
    cdx.extend(cdg);
    checks.push(compare("norm_back", &gv, &cdx));

    // softmax rows (full and causal) and its backward
    let (nr, w) = (29, 77);
    let x = rand(nr * w, 60, 6.0);
    let dp = rand(nr * w, 61, 1.0);
    let (bx, bc, bd) = (up(&x), up(&x), up(&dp));
    let mut r = g.run(prec);
    r.softmax((&bx, 0), nr, w, None);
    r.softmax((&bc, 0), nr, w, Some((nr, 3)));
    r.submit();
    let mut cs = x.clone();
    k::softmax_rows(&mut cs, w);
    let mut cc = x.clone();
    for (t, row) in cc.chunks_exact_mut(w).enumerate() {
        let len = (t + 4).min(w);
        k::softmax_rows(&mut row[..len], len);
    }
    let mut gv = read(&bx);
    gv.extend(read(&bc));
    let mut cv = cs.clone();
    cv.extend(cc);
    checks.push(compare("softmax", &gv, &cv));
    let gp = read(&bx);
    let mut r = g.run(prec);
    let bp = up(&gp);
    r.softmax_back((&bp, 0), (&bd, 0), nr, w);
    r.submit();
    let mut cd = dp.clone();
    k::softmax_back_rows(&gp, &mut cd, w);
    checks.push(compare("softmax_back", &read(&bd), &cd));

    // elementwise: exp, sigmoid, ÷, √ over a wide range
    let mut x = rand(4000, 70, 1.0);
    x.iter_mut()
        .enumerate()
        .for_each(|(i, v)| *v *= [0.01, 1.0, 10.0, 90.0][i % 4]);
    x.extend([0.0, -0.0, 88.7, -86.9, 1e-30, -1e-30]);
    let y = rand(x.len(), 71, 3.0);
    let pos: Vec<f32> = x.iter().map(|v| v.abs() + 1e-3).collect();
    let n = x.len();
    let (bx, by, bp) = (up(&x), up(&y), up(&pos));
    let (be, bsg, bdv, bsq) = (up(&x), g.zeros(n), g.zeros(n), g.zeros(n));
    let mut r = g.run(prec);
    r.ew(5, n, (&be, 0), None, None, 0.0, 0);
    r.ew(6, n, (&bx, 0), None, Some((&bsg, 0)), 0.0, 0);
    r.ew(7, n, (&by, 0), Some((&bp, 0)), Some((&bdv, 0)), 0.0, 0);
    r.ew(8, n, (&bp, 0), None, Some((&bsq, 0)), 0.0, 0);
    r.submit();
    let mut ce = x.clone();
    k::exp_slice(&mut ce);
    checks.push(compare("exp", &read(&be), &ce));
    let mut cs = vec![0.0; n];
    k::sigmoid_slice(&x, &mut cs);
    checks.push(compare("sigmoid", &read(&bsg), &cs));
    let cd: Vec<f32> = y.iter().zip(&pos).map(|(a, b)| a / b).collect();
    checks.push(compare("divide", &read(&bdv), &cd));
    let cq: Vec<f32> = pos.iter().map(|v| v.sqrt()).collect();
    checks.push(compare("sqrt", &read(&bsq), &cq));

    // SwiGLU forward and backward
    let n = 1500;
    let (gg, uu, da) = (rand(n, 80, 5.0), rand(n, 81, 2.0), rand(n, 82, 1.0));
    let (bg, bu, bs, ba, bda, bdg, bdu) = (
        up(&gg),
        up(&uu),
        g.zeros(n),
        g.zeros(n),
        up(&da),
        g.zeros(n),
        g.zeros(n),
    );
    let mut r = g.run(prec);
    r.swiglu(&bg, &bu, &bs, &ba, n);
    r.swiglu_back(&bda, &bg, &bs, &bu, &bdg, &bdu, n);
    r.submit();
    let mut s = vec![0.0; n];
    k::sigmoid_slice(&gg, &mut s);
    let gs = read(&bs);
    let mut cv: Vec<f32> = (0..n).map(|i| (gg[i] * s[i]) * uu[i]).collect();
    cv.extend((0..n).map(|i| (da[i] * uu[i]) * (gs[i] * (1.0 + gg[i] * (1.0 - gs[i])))));
    cv.extend((0..n).map(|i| da[i] * (gg[i] * gs[i])));
    let mut gv = read(&ba);
    gv.extend(read(&bdg));
    gv.extend(read(&bdu));
    checks.push(compare("swiglu", &gv, &cv));

    // rotary positions, forward and back
    let (len, dh) = (50, 32);
    let rope = Rope::new(len, dh, 10_000.0);
    let x = rand(len * dh, 90, 1.0);
    let (cos, sin) = rope.tables();
    let (bc, bs, bx, bb) = (up(cos), up(sin), up(&x), up(&x));
    let mut r = g.run(prec);
    r.rope((&bx, 0), &bc, &bs, len, dh / 2, len, 0, false);
    r.rope((&bb, 0), &bc, &bs, len, dh / 2, len, 0, true);
    r.submit();
    let (mut cf, mut cb) = (x.clone(), x.clone());
    rope.apply(&mut cf, len);
    rope.back(&mut cb, len);
    let mut gv = read(&bx);
    gv.extend(read(&bb));
    cf.extend(cb);
    checks.push(compare("rope", &gv, &cf));

    // causal attention scores (8-lane dots)
    let (nq, dh2) = (20, 24);
    let q = rand(2 * nq * dh2, 100, 1.0);
    let kk = rand(2 * nq * dh2, 101, 1.0);
    let (bq, bk, bsc) = (up(&q), up(&kk), g.zeros(2 * nq * nq));
    let scale = 1.0 / (dh2 as f32).sqrt();
    let mut r = g.run(prec);
    r.scores(
        (&bq, 0, nq * dh2),
        (&bk, 0, nq * dh2),
        (&bsc, 0),
        nq,
        nq,
        dh2,
        2,
        0,
        scale,
        1,
    );
    r.submit();
    let mut cv = vec![0f32; 2 * nq * nq];
    for h in 0..2 {
        for t in 0..nq {
            for s in 0..=t {
                let qb = h * nq * dh2 + t * dh2;
                let kb = h * nq * dh2 + s * dh2;
                cv[(h * nq + t) * nq + s] = k::dot(&q[qb..qb + dh2], &kk[kb..kb + dh2]) * scale;
            }
        }
    }
    checks.push(compare("scores", &read(&bsc), &cv));

    Report {
        device: g.describe(),
        precision: prec,
        checks,
    }
}
