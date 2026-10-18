//! Training on the device: one example's forward with its dropout masks, the losses
//! and the heads on the CPU, then backward through the trunk on the device, every
//! operation in `Net::backward_gen`'s order. The trunk's gradients (blocks, final norm,
//! generative head) stay on the device and are summed over a batch in example order,
//! as `train::batch_grad` sums its per-example buffers; the embedding, pooling and head
//! gradients are computed on the CPU into the caller's per-example buffer. After the
//! batch, `finish` reads the trunk's sums into the CPU gradient, and the optimizer
//! steps on the CPU (`GpuNet::refresh` uploads the new weights).

use std::sync::Mutex;

use super::net::{Acts, GpuNet, pooled};
use super::{Buf, Gpu, Precision, Run, View};
use crate::classify::kernels as k;
use crate::classify::math::key;
use crate::classify::model::{Mode, Net, Pooling, dropout_mask};
use crate::classify::text::{NUM, Window};
use crate::classify::train::{Target, gen_loss, head_loss};

/// a block's gradient tensors on the device
struct GradBlock {
    n1: Buf,
    wqkv: Buf,
    bqkv: Buf,
    qn: Buf,
    kn: Buf,
    wo: Buf,
    n2: Buf,
    wg: Buf,
    wu: Buf,
    wd: Buf,
}

struct Grads {
    blocks: Vec<GradBlock>,
    nf: Buf,
    gen_w: Buf,
    gen_b: Buf,
}

impl Grads {
    fn new(g: &Gpu, net: &Net) -> Grads {
        let z = |v: &Vec<f32>| g.zeros(v.len());
        Grads {
            blocks: net
                .blocks
                .iter()
                .map(|b| GradBlock {
                    n1: z(&b.n1),
                    wqkv: z(&b.wqkv),
                    bqkv: z(&b.bqkv),
                    qn: z(&b.qn),
                    kn: z(&b.kn),
                    wo: z(&b.wo),
                    n2: z(&b.n2),
                    wg: z(&b.wg),
                    wu: z(&b.wu),
                    wd: z(&b.wd),
                })
                .collect(),
            nf: z(&net.nf),
            gen_w: z(&net.gen_w),
            gen_b: z(&net.gen_b),
        }
    }

    fn all(&self) -> Vec<&Buf> {
        let mut v = Vec::new();
        for b in &self.blocks {
            v.extend([&b.n1, &b.wqkv, &b.bqkv, &b.qn, &b.kn, &b.wo, &b.n2, &b.wg, &b.wu, &b.wd]);
        }
        v.extend([&self.nf, &self.gen_w, &self.gen_b]);
        v
    }

    /// the CPU tensors these mirror, in the same order
    fn of(net: &mut Net) -> Vec<&mut Vec<f32>> {
        let mut v = Vec::new();
        for b in &mut net.blocks {
            v.extend([
                &mut b.n1,
                &mut b.wqkv,
                &mut b.bqkv,
                &mut b.qn,
                &mut b.kn,
                &mut b.wo,
                &mut b.n2,
                &mut b.wg,
                &mut b.wu,
                &mut b.wd,
            ]);
        }
        v.extend([&mut net.nf, &mut net.gen_w, &mut net.gen_b]);
        v
    }
}

/// backward's scratch for one window of up to max_len rows
struct Back {
    dx: Buf,
    dd: Buf,
    dact: Buf,
    dg: Buf,
    du: Buf,
    dbn: Buf,
    dout: Buf,
    dctx: Buf,
    ds: Buf,
    dq: Buf,
    dq0: Buf,
    dkh: Buf,
    dvh: Buf,
    dkg: Buf,
    dk0: Buf,
    dqkv: Buf,
    da: Buf,
}

struct State {
    acts: Vec<Acts>,
    x: [Buf; 2],
    xf: Buf,
    xhf: Buf,
    rsf: Buf,
    genl: Buf,
    dxf: Buf,
    dgen: Buf,
    back: Back,
    ex: Grads,
    total: Grads,
}

/// a net's training state on the device
pub struct Trainer {
    pub gn: GpuNet,
    st: Mutex<State>,
}

impl Trainer {
    pub fn new(g: &'static Gpu, net: &Net, prec: Precision) -> Result<Trainer, String> {
        let gn = GpuNet::new(g, net, prec)?;
        let cfg = &net.cfg;
        let (n, d, dh) = (cfg.max_len, cfg.d_model, cfg.head_dim);
        let hmax = net.blocks.iter().map(|b| b.heads).max().unwrap_or(1);
        let kvmax = net.blocks.iter().map(|b| cfg.kv_of(b.heads)).max().unwrap_or(1);
        let ffmax = net.blocks.iter().map(|b| b.ff).max().unwrap_or(1);
        let w3max = net.blocks.iter().map(|b| b.qkv_width(cfg)).max().unwrap_or(1);
        let go = cfg.gen_out();
        let masks = cfg.dropout > 0.0;
        let st = State {
            acts: (0..net.blocks.len()).map(|l| gn.acts(l, n, masks)).collect(),
            x: [g.zeros(n * d), g.zeros(n * d)],
            xf: g.zeros(n * d),
            xhf: g.zeros(n * d),
            rsf: g.zeros(n),
            genl: g.zeros(n * go),
            dxf: g.zeros(n * d),
            dgen: g.zeros(n * go),
            back: Back {
                dx: g.zeros(n * d),
                dd: g.zeros(n * d),
                dact: g.zeros(n * ffmax),
                dg: g.zeros(n * ffmax),
                du: g.zeros(n * ffmax),
                dbn: g.zeros(n * d),
                dout: g.zeros(n * d),
                dctx: g.zeros(n * hmax * dh),
                ds: g.zeros(hmax * n * n),
                dq: g.zeros(hmax * n * dh),
                dq0: g.zeros(hmax * n * dh),
                dkh: g.zeros(hmax * n * dh),
                dvh: g.zeros(hmax * n * dh),
                dkg: g.zeros(2 * kvmax * n * dh),
                dk0: g.zeros(kvmax * n * dh),
                dqkv: g.zeros(n * w3max),
                da: g.zeros(n * d),
            },
            ex: Grads::new(g, net),
            total: Grads::new(g, net),
        };
        Ok(Trainer { gn, st: Mutex::new(st) })
    }

    /// a new batch: the net's current weights uploaded, the sums zeroed
    pub fn begin(&self, net: &Net) {
        self.gn.refresh(net);
        let st = self.st.lock().unwrap();
        let mut r = self.gn.g.run(self.gn.prec);
        for b in st.total.all() {
            r.fill((b, 0), 0.0, b.len);
        }
        r.submit();
    }

    /// reads the batch's trunk gradient sums into `total` (whose trunk tensors the CPU
    /// left at zero)
    pub fn finish(&self, total: &mut Net) {
        let st = self.st.lock().unwrap();
        let bufs = st.total.all();
        let mut r = self.gn.g.run(self.gn.prec);
        let ids: Vec<usize> = bufs.iter().map(|b| r.read(b, 0, b.len)).collect();
        let mut got = r.submit();
        for (dst, i) in Grads::of(total).into_iter().zip(ids) {
            *dst = std::mem::take(&mut got[i]);
        }
    }

    /// one example: adds coef·∇loss into the device's sums (trunk) and into `g` (the
    /// rest); returns the weighted loss and tokens, as `train::example_grad`
    #[allow(clippy::too_many_arguments)]
    pub fn example<'t>(
        &self,
        net: &Net,
        w: &Window,
        start: usize,
        targets: impl Iterator<Item = (usize, &'t Target)>,
        extra: &[f32],
        mode: Mode,
        coef: f32,
        smoothing: f32,
        g: &mut Net,
    ) -> (f32, usize) {
        let st = &mut *self.st.lock().unwrap();
        let gn = &self.gn;
        let gpu = gn.g;
        let cfg = &net.cfg;
        let (d, n) = (cfg.d_model, w.len());
        let go = cfg.gen_out();
        let mask = |site: u64, len: usize| {
            (mode.dropout > 0.0).then(|| dropout_mask(key(&[mode.key, site]), len, mode.dropout))
        };
        // forward
        let mut x = net.embed(w, &cfg.gains);
        let m0 = mask(1, n * d);
        if let Some(m) = &m0 {
            x.iter_mut().zip(m).for_each(|(a, b)| *a *= b);
        }
        gpu.write(&st.x[0], 0, &x);
        for (l, a) in st.acts.iter().enumerate() {
            let l1 = l as u64 + 1;
            for (site, buf) in [(l1 << 8 | 2, &a.m1), (l1 << 8 | 3, &a.m2)] {
                if let (Some(buf), Some(m)) = (buf, mask(site, n * d)) {
                    gpu.write(buf, 0, &m);
                }
            }
        }
        let mut r = gpu.run(gn.prec);
        for (l, a) in st.acts.iter().enumerate() {
            let (xi, xo) = (&st.x[l % 2], &st.x[(l + 1) % 2]);
            gn.block_fwd(&mut r, l, xi, xo, a, n);
        }
        let xl = &st.x[st.acts.len() % 2];
        let has_gen = net.generator.is_some();
        gn.top_fwd(
            &mut r,
            xl,
            &st.xf,
            Some((&st.xhf, &st.rsf)),
            has_gen.then_some(&st.genl),
            n,
        );
        let t_xf = r.read(&st.xf, 0, n * d);
        let t_gl = has_gen.then(|| r.read(&st.genl, 0, n * go));
        let mut out = r.submit();
        let xf = std::mem::take(&mut out[t_xf]);
        let genl = t_gl.map(|i| std::mem::take(&mut out[i]));
        // the heads' input as `Net::forward` builds it
        let (pool_p, mut pooled_v) = pooled(net, &xf, n);
        let mp = mask(0xff01, pooled_v.len());
        if let Some(m) = &mp {
            pooled_v.iter_mut().zip(m).for_each(|(a, b)| *a *= b);
        }
        let mut z = pooled_v.clone();
        if cfg.side {
            z.extend(w.side.iter().map(|v| v * cfg.gains.side));
        }
        z.extend(extra.iter().map(|v| v * cfg.gains.extra));
        // losses
        let mut dls = vec![Vec::new(); net.heads.len()];
        let mut dgen: Vec<f32> = Vec::new();
        let mut loss = 0f32;
        let add_scaled = |into: &mut Vec<f32>, dv: &[f32], s: f32| {
            if into.is_empty() {
                *into = vec![0.0; dv.len()];
            }
            into.iter_mut().zip(dv).for_each(|(a, b)| *a += b * s);
        };
        for (h, t) in targets {
            if t.generative() {
                let hw = net.generator.as_ref().map_or(1.0, |g| g.weight);
                let (l, dv, _) = gen_loss(net, genl.as_ref().expect("generative head"), &w.tokens, start, t);
                loss += hw * l;
                add_scaled(&mut dgen, &dv, hw * coef);
            } else {
                let hd = &net.heads[h];
                let hw = hd.spec.weight;
                let mut y = k::matmul(k::rows(&z, z.len()), &hd.w, 1, z.len(), hd.width(), None);
                y.iter_mut().zip(&hd.b).for_each(|(a, b)| *a += b);
                let (l, dv) = head_loss(hd.spec.kind, &y, t, smoothing).expect("checked target");
                loss += hw * l;
                add_scaled(&mut dls[h], &dv, hw * coef);
            }
        }
        // heads and pooling backward (CPU), as `Net::backward_gen` begins
        let zl = z.len();
        let p = cfg.pooled();
        let mut dz = vec![0f32; zl];
        for (h, dl) in dls.iter().enumerate() {
            if dl.is_empty() {
                continue;
            }
            let (hd, gh) = (&net.heads[h], &mut g.heads[h]);
            let kw = hd.width();
            k::gemm(&mut gh.w, k::transposed(&z, 1), dl, zl, 1, kw, None);
            gh.b.iter_mut().zip(dl).for_each(|(a, b)| *a += b);
            for (j, v) in dz.iter_mut().enumerate() {
                *v += k::dot(&hd.w[j * kw..(j + 1) * kw], dl);
            }
        }
        let mut dpool = dz[..p].to_vec();
        if let Some(m) = &mp {
            dpool.iter_mut().zip(m).for_each(|(a, b)| *a *= b);
        }
        let mut dxf = vec![0f32; n * d];
        let add_mean = |dxf: &mut [f32], dm: &[f32]| {
            let dmn: Vec<f32> = dm.iter().map(|v| v / n as f32).collect();
            for row in dxf.chunks_exact_mut(d) {
                row.iter_mut().zip(&dmn).for_each(|(a, b)| *a += b);
            }
        };
        match cfg.pooling {
            Pooling::Cls => dxf[..d].copy_from_slice(&dpool),
            Pooling::Mean => add_mean(&mut dxf, &dpool),
            Pooling::ClsMean => {
                dxf[..d].copy_from_slice(&dpool[..d]);
                add_mean(&mut dxf, &dpool[d..]);
            }
            Pooling::Attention => {
                let mut dp: Vec<f32> = xf.chunks_exact(d).map(|r| k::dot(r, &dpool)).collect();
                for (row, &pp) in dxf.chunks_exact_mut(d).zip(&pool_p) {
                    k::axpy(row, pp, &dpool);
                }
                k::softmax_back_rows(&pool_p, &mut dp, n);
                for (t, &de) in dp.iter().enumerate() {
                    k::axpy(&mut g.pool_q, de, &xf[t * d..(t + 1) * d]);
                    k::axpy(&mut dxf[t * d..(t + 1) * d], de, &net.pool_q);
                }
            }
        }
        gpu.write(&st.dxf, 0, &dxf);
        if !dgen.is_empty() {
            gpu.write(&st.dgen, 0, &dgen);
        }
        // the trunk backward on the device
        let mut r = gpu.run(gn.prec);
        for b in st.ex.all() {
            r.fill((b, 0), 0.0, b.len);
        }
        if !dgen.is_empty() {
            let (gw, _) = gn.gen_bufs().expect("generative head");
            r.gemm(
                View::t(&st.xf, d),
                View::rows(&st.dgen, go),
                View::rows(&st.ex.gen_w, go),
                d,
                n,
                go,
                1,
                true,
                None,
            );
            r.col_acc((&st.dgen, 0), None, (&st.ex.gen_b, 0), n, go);
            r.gemm(
                View::rows(&st.dgen, go),
                View::t(gw, go),
                View::rows(&st.dxf, d),
                n,
                go,
                d,
                1,
                true,
                None,
            );
        }
        let bk = &st.back;
        r.fill((&bk.dx, 0), 0.0, n * d);
        r.norm_back(
            (&st.dxf, 0),
            (&st.xhf, 0),
            (&st.rsf, 0),
            gn.nf_buf(),
            (&bk.dx, 0),
            (&st.ex.nf, 0),
            n,
            d,
        );
        for l in (0..st.acts.len()).rev() {
            self.block_back(&mut r, net, l, &st.acts[l], bk, &st.ex.blocks[l], n);
        }
        for (dst, src) in st.total.all().into_iter().zip(st.ex.all()) {
            r.add_into((dst, 0), (src, 0), src.len);
        }
        let t_dx = r.read(&bk.dx, 0, n * d);
        let mut dx = r.submit().swap_remove(t_dx);
        if let Some(m) = &m0 {
            dx.iter_mut().zip(m).for_each(|(a, b)| *a *= b);
        }
        embed_back(net, w, &dx, g);
        (loss, n)
    }

    #[allow(clippy::too_many_arguments)]
    fn block_back(&self, r: &mut Run, net: &Net, l: usize, t: &Acts, bk: &Back, gb: &GradBlock, n: usize) {
        let cfg = &net.cfg;
        let b = &net.blocks[l];
        let wb = self.gn.block_bufs(l);
        let (d, dh, ff) = (cfg.d_model, cfg.head_dim, b.ff);
        let (h, kv) = (b.heads, cfg.kv_of(b.heads));
        let (aw, w3, per) = (h * dh, b.qkv_width(cfg), h / kv);
        let scale = 1.0 / (dh as f32).sqrt();
        // feed-forward
        r.copy(&bk.dx, 0, &bk.dd, 0, n * d);
        if let Some(m) = &t.m2 {
            r.mul_into(&bk.dd, m, n * d);
        }
        r.gemm(
            View::t(&t.act, ff),
            View::rows(&bk.dd, d),
            View::rows(&gb.wd, d),
            ff,
            n,
            d,
            1,
            true,
            None,
        );
        r.gemm(
            View::rows(&bk.dd, d),
            View::t(wb.wd, d),
            View::rows(&bk.dact, ff),
            n,
            d,
            ff,
            1,
            false,
            None,
        );
        r.swiglu_back(&bk.dact, &t.gg, &t.s, &t.u, &bk.dg, &bk.du, n * ff);
        r.gemm(
            View::t(&t.bn, d),
            View::rows(&bk.dg, ff),
            View::rows(&gb.wg, ff),
            d,
            n,
            ff,
            1,
            true,
            None,
        );
        r.gemm(
            View::t(&t.bn, d),
            View::rows(&bk.du, ff),
            View::rows(&gb.wu, ff),
            d,
            n,
            ff,
            1,
            true,
            None,
        );
        r.gemm(
            View::rows(&bk.dg, ff),
            View::t(wb.wg, ff),
            View::rows(&bk.dbn, d),
            n,
            ff,
            d,
            1,
            false,
            None,
        );
        r.gemm(
            View::rows(&bk.du, ff),
            View::t(wb.wu, ff),
            View::rows(&bk.dbn, d),
            n,
            ff,
            d,
            1,
            true,
            None,
        );
        r.norm_back(
            (&bk.dbn, 0),
            (&t.xh2, 0),
            (&t.rs2, 0),
            wb.n2,
            (&bk.dx, 0),
            (&gb.n2, 0),
            n,
            d,
        );
        // attention
        r.copy(&bk.dx, 0, &bk.dout, 0, n * d);
        if let Some(m) = &t.m1 {
            r.mul_into(&bk.dout, m, n * d);
        }
        r.gemm(
            View::t(&t.ctx, aw),
            View::rows(&bk.dout, d),
            View::rows(&gb.wo, d),
            aw,
            n,
            d,
            1,
            true,
            None,
        );
        r.gemm(
            View::rows(&bk.dout, d),
            View::t(wb.wo, d),
            View::rows(&bk.dctx, aw),
            n,
            d,
            aw,
            1,
            false,
            None,
        );
        let dco = View::of(&bk.dctx, 0, aw, 1, dh);
        let nn = n * n;
        let hs = n * dh;
        // ds = dco·vᵀ, dv = pᵀ·dco, per head
        r.gemm(
            dco,
            View::of(&t.v, 0, 1, dh, hs).per(per),
            View::of(&bk.ds, 0, n, 1, nn),
            n,
            dh,
            n,
            h,
            false,
            None,
        );
        r.gemm(
            View::of(&t.p, 0, 1, n, nn),
            View::of(&bk.dctx, 0, aw, 1, dh),
            View::of(&bk.dvh, 0, dh, 1, hs),
            n,
            n,
            dh,
            h,
            false,
            None,
        );
        r.softmax_back((&t.p, 0), (&bk.ds, 0), h * n, n);
        r.scale((&bk.ds, 0), scale, h * nn);
        r.gemm(
            View::of(&bk.ds, 0, n, 1, nn),
            View::of(&t.k, 0, dh, 1, hs).per(per),
            View::of(&bk.dq, 0, dh, 1, hs),
            n,
            n,
            dh,
            h,
            false,
            None,
        );
        r.gemm(
            View::of(&bk.ds, 0, 1, n, nn),
            View::of(&t.q, 0, dh, 1, hs),
            View::of(&bk.dkh, 0, dh, 1, hs),
            n,
            n,
            dh,
            h,
            false,
            None,
        );
        r.rope(
            (&bk.dq, 0),
            self.gn.cos_buf(),
            self.gn.sin_buf(),
            h * n,
            dh / 2,
            n,
            0,
            true,
        );
        let dq = if cfg.qk_norm {
            r.fill((&bk.dq0, 0), 0.0, h * hs);
            r.norm_back(
                (&bk.dq, 0),
                (&t.qxh, 0),
                (&t.qrs, 0),
                wb.qn.expect("qn"),
                (&bk.dq0, 0),
                (&gb.qn, 0),
                h * n,
                dh,
            );
            &bk.dq0
        } else {
            &bk.dq
        };
        // a group's key and value gradients, its heads summed in head order
        let (dkg, dvg_off) = (&bk.dkg, kv * hs);
        if per == 1 {
            r.copy(&bk.dkh, 0, dkg, 0, kv * hs);
            r.copy(&bk.dvh, 0, dkg, dvg_off, kv * hs);
        } else {
            for gi in 0..kv {
                r.copy(&bk.dkh, gi * per * hs, dkg, gi * hs, hs);
                r.copy(&bk.dvh, gi * per * hs, dkg, dvg_off + gi * hs, hs);
                for j in 1..per {
                    r.add_into((dkg, gi * hs), (&bk.dkh, (gi * per + j) * hs), hs);
                    r.add_into((dkg, dvg_off + gi * hs), (&bk.dvh, (gi * per + j) * hs), hs);
                }
            }
        }
        r.rope(
            (dkg, 0),
            self.gn.cos_buf(),
            self.gn.sin_buf(),
            kv * n,
            dh / 2,
            n,
            0,
            true,
        );
        let (dk, dk_off) = if cfg.qk_norm {
            r.fill((&bk.dk0, 0), 0.0, kv * hs);
            r.norm_back(
                (dkg, 0),
                (&t.kxh, 0),
                (&t.krs, 0),
                wb.kn.expect("kn"),
                (&bk.dk0, 0),
                (&gb.kn, 0),
                kv * n,
                dh,
            );
            (&bk.dk0, 0)
        } else {
            (dkg, 0)
        };
        r.copy_strided((dq, 0, hs, dh), (&bk.dqkv, 0, dh, w3), dh, n, h);
        r.copy_strided((dk, dk_off, hs, dh), (&bk.dqkv, aw, dh, w3), dh, n, kv);
        r.copy_strided((dkg, dvg_off, hs, dh), (&bk.dqkv, aw + kv * dh, dh, w3), dh, n, kv);
        if !b.bqkv.is_empty() {
            r.col_acc((&bk.dqkv, 0), None, (&gb.bqkv, 0), n, w3);
        }
        r.gemm(
            View::t(&t.a, d),
            View::rows(&bk.dqkv, w3),
            View::rows(&gb.wqkv, w3),
            d,
            n,
            w3,
            1,
            true,
            None,
        );
        r.gemm(
            View::rows(&bk.dqkv, w3),
            View::t(wb.wqkv, w3),
            View::rows(&bk.da, d),
            n,
            w3,
            d,
            1,
            false,
            None,
        );
        r.norm_back(
            (&bk.da, 0),
            (&t.xh1, 0),
            (&t.rs1, 0),
            wb.n1,
            (&bk.dx, 0),
            (&gb.n1, 0),
            n,
            d,
        );
    }
}

/// the embedding tables' gradients from d(loss)/d(embedded rows), token by token in
/// order, as the end of `Net::backward_gen`
fn embed_back(net: &Net, w: &Window, dx: &[f32], g: &mut Net) {
    let d = net.cfg.d_model;
    let gn = &net.cfg.gains;
    let rows_of = |i: u8| i as usize * d..(i as usize + 1) * d;
    for t in 0..w.len() {
        let r = &dx[t * d..(t + 1) * d];
        let tk = w.tokens[t] as usize;
        g.tok[tk * d..(tk + 1) * d].iter_mut().zip(r).for_each(|(a, b)| *a += b);
        if w.tokens[t] == NUM && !g.num.is_empty() {
            let v = w.values.get(t).copied().unwrap_or(0.0);
            g.num.iter_mut().zip(r).for_each(|(a, b)| *a += v * b);
        }
        let dl: Vec<f32> = r.iter().map(|v| gn.letter * v).collect();
        let dw: Vec<f32> = r.iter().map(|v| gn.word * v).collect();
        let add = |tab: &mut Vec<f32>, idx: u8, v: &[f32]| {
            tab[rows_of(idx)].iter_mut().zip(v).for_each(|(a, b)| *a += b);
        };
        if net.cfg.letters {
            add(&mut g.letter, w.letter[t], &dl);
        }
        if net.cfg.words {
            add(&mut g.word_ord, w.ordinal[t], &dw);
            add(&mut g.word_rev, w.reversed[t], &dw);
            add(&mut g.word_atb, w.atbash[t], &dw);
            add(&mut g.word_root, w.root[t], &dw);
        }
    }
}
