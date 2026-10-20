//! A [`Net`]'s trunk resident on the device: the weights uploaded once, each window's
//! embedded tokens uploaded, every block run as a list of kernel dispatches in one
//! submission, and only what the CPU needs read back (the final normed rows, the
//! generative logits). The embedding lookup, pooling and the small classification
//! heads stay on the CPU (they are a sliver of the work); the decoder keeps its
//! key/value cache on the device.
//!
//! Each dispatch mirrors the CPU's operations in `Net::forward` / `Decoder::step` in the
//! same order, so on a device the probe finds exact the outputs are the CPU's bits
//! (`tests` check that, and a tolerance everywhere else).

use super::{Buf, Gpu, Precision, Run, View};
use crate::classify::kernels as k;
use crate::classify::model::{Gains, Net, Pooling};
use crate::classify::text::{self, Window};

struct GBlock {
    heads: usize,
    kv: usize,
    ff: usize,
    n1: Buf,
    wqkv: Buf,
    bqkv: Option<Buf>,
    qn: Option<Buf>,
    kn: Option<Buf>,
    wo: Buf,
    n2: Buf,
    wg: Buf,
    wu: Buf,
    wd: Buf,
}

/// a block's weights on the device
pub(crate) struct BlockBufs<'a> {
    pub n1: &'a Buf,
    pub wqkv: &'a Buf,
    pub qn: Option<&'a Buf>,
    pub kn: Option<&'a Buf>,
    pub wo: &'a Buf,
    pub n2: &'a Buf,
    pub wg: &'a Buf,
    pub wu: &'a Buf,
    pub wd: &'a Buf,
}

/// a net's trunk (blocks, final norm, generative head) on the device
pub struct GpuNet {
    pub g: &'static Gpu,
    pub prec: Precision,
    d: usize,
    dh: usize,
    eps: f32,
    causal: bool,
    qk_norm: bool,
    max_len: usize,
    gen_out: usize,
    blocks: Vec<GBlock>,
    nf: Buf,
    genh: Option<(Buf, Buf)>,
    cos: Buf,
    sin: Buf,
}

/// one block's activations for a window of n (the training trace keeps one per block)
pub(crate) struct Acts {
    pub a: Buf,
    pub xh1: Buf,
    pub rs1: Buf,
    pub qkv: Buf,
    /// queries [heads × n × dh] and keys, values [kv × n × dh], after norm and rotation
    pub q: Buf,
    pub k: Buf,
    pub v: Buf,
    /// before QK-norm, and its x̂ and rstd
    pub q0: Buf,
    pub k0: Buf,
    pub qxh: Buf,
    pub qrs: Buf,
    pub kxh: Buf,
    pub krs: Buf,
    /// attention probabilities [heads × n × n]
    pub p: Buf,
    pub ctx: Buf,
    pub o: Buf,
    pub hres: Buf,
    pub xh2: Buf,
    pub rs2: Buf,
    pub bn: Buf,
    pub gg: Buf,
    pub u: Buf,
    pub s: Buf,
    pub act: Buf,
    pub dn: Buf,
    pub m1: Option<Buf>,
    pub m2: Option<Buf>,
}

impl GpuNet {
    /// uploads a net; errors when a tensor is larger than the device takes
    pub fn new(g: &'static Gpu, net: &Net, prec: Precision) -> Result<GpuNet, String> {
        let cfg = &net.cfg;
        let max = g.max_floats();
        let biggest = net
            .blocks
            .iter()
            .flat_map(|b| [b.wqkv.len(), b.wg.len(), b.wd.len()])
            .chain([net.gen_w.len(), cfg.max_len * cfg.max_len * cfg.heads])
            .max()
            .unwrap_or(0);
        if biggest > max {
            return Err(format!(
                "a tensor of {biggest} floats is larger than {} takes ({max})",
                g.describe()
            ));
        }
        let opt = |v: &Vec<f32>| (!v.is_empty()).then(|| g.upload(v));
        let blocks = net
            .blocks
            .iter()
            .map(|b| GBlock {
                heads: b.heads,
                kv: cfg.kv_of(b.heads),
                ff: b.ff,
                n1: g.upload(&b.n1),
                wqkv: g.upload(&b.wqkv),
                bqkv: opt(&b.bqkv),
                qn: opt(&b.qn),
                kn: opt(&b.kn),
                wo: g.upload(&b.wo),
                n2: g.upload(&b.n2),
                wg: g.upload(&b.wg),
                wu: g.upload(&b.wu),
                wd: g.upload(&b.wd),
            })
            .collect();
        let (cos, sin) = net.rope.tables();
        Ok(GpuNet {
            g,
            prec,
            d: cfg.d_model,
            dh: cfg.head_dim,
            eps: cfg.norm_eps,
            causal: cfg.causal,
            qk_norm: cfg.qk_norm,
            max_len: cfg.max_len,
            gen_out: cfg.gen_out(),
            blocks,
            nf: g.upload(&net.nf),
            genh: net
                .generator
                .as_ref()
                .map(|_| (g.upload(&net.gen_w), g.upload(&net.gen_b))),
            cos: g.upload(cos),
            sin: g.upload(sin),
        })
    }

    /// writes a net's weights (same shapes) over the uploaded ones, as after an
    /// optimizer step
    pub fn refresh(&self, net: &Net) {
        let g = self.g;
        for (gb, b) in self.blocks.iter().zip(&net.blocks) {
            for (buf, v) in [
                (&gb.n1, &b.n1),
                (&gb.wqkv, &b.wqkv),
                (&gb.wo, &b.wo),
                (&gb.n2, &b.n2),
                (&gb.wg, &b.wg),
                (&gb.wu, &b.wu),
                (&gb.wd, &b.wd),
            ] {
                g.write(buf, 0, v);
            }
            for (buf, v) in [(&gb.bqkv, &b.bqkv), (&gb.qn, &b.qn), (&gb.kn, &b.kn)] {
                if let Some(buf) = buf {
                    g.write(buf, 0, v);
                }
            }
        }
        g.write(&self.nf, 0, &net.nf);
        if let Some((w, b)) = &self.genh {
            g.write(w, 0, &net.gen_w);
            g.write(b, 0, &net.gen_b);
        }
    }

    /// whether this net matches a CPU net's shapes (blocks, heads, widths)
    pub fn fits(&self, net: &Net) -> bool {
        self.blocks.len() == net.blocks.len()
            && self.d == net.cfg.d_model
            && self
                .blocks
                .iter()
                .zip(&net.blocks)
                .all(|(a, b)| a.heads == b.heads && a.ff == b.ff && a.wqkv.len == b.wqkv.len())
    }

    pub(crate) fn acts(&self, b: usize, n: usize, masks: bool) -> Acts {
        self.acts_for(b, n, n * n, masks)
    }

    /// activations for `n` rows in all whose windows' attention squares add up to `sq`
    /// (one window: `sq = n²`; packed windows: the sum of each one's)
    fn acts_for(&self, b: usize, n: usize, sq: usize, masks: bool) -> Acts {
        let g = self.g;
        let blk = &self.blocks[b];
        let (d, dh) = (self.d, self.dh);
        let (h, kv, ff) = (blk.heads, blk.kv, blk.ff);
        let w3 = (h + 2 * kv) * dh;
        let qk = |m: usize| if self.qk_norm { m } else { 0 };
        Acts {
            a: g.zeros(n * d),
            xh1: g.zeros(n * d),
            rs1: g.zeros(n),
            qkv: g.zeros(n * w3),
            q: g.zeros(h * n * dh),
            k: g.zeros(kv * n * dh),
            v: g.zeros(kv * n * dh),
            q0: g.zeros(qk(h * n * dh)),
            k0: g.zeros(qk(kv * n * dh)),
            qxh: g.zeros(qk(h * n * dh)),
            qrs: g.zeros(qk(h * n)),
            kxh: g.zeros(qk(kv * n * dh)),
            krs: g.zeros(qk(kv * n)),
            p: g.zeros(h * sq),
            ctx: g.zeros(n * h * dh),
            o: g.zeros(n * d),
            hres: g.zeros(n * d),
            xh2: g.zeros(n * d),
            rs2: g.zeros(n),
            bn: g.zeros(n * d),
            gg: g.zeros(n * ff),
            u: g.zeros(n * ff),
            s: g.zeros(n * ff),
            act: g.zeros(n * ff),
            dn: g.zeros(n * d),
            m1: masks.then(|| g.zeros(n * d)),
            m2: masks.then(|| g.zeros(n * d)),
        }
    }

    /// records block `l` over a window of n rows: x → out, its activations into `t`
    pub(crate) fn block_fwd(&self, r: &mut Run, l: usize, x: &Buf, out: &Buf, t: &Acts, n: usize) {
        self.block_fwd_packed(r, l, x, out, t, &[n]);
    }

    /// records block `l` over windows packed one after another (`lens` rows each): the
    /// row-wise work (norms, projections, the feed-forward) runs over all the rows at
    /// once, attention per window. Each output element is computed by the same chain as
    /// for its window alone, so packing changes no bits.
    pub(crate) fn block_fwd_packed(&self, r: &mut Run, l: usize, x: &Buf, out: &Buf, t: &Acts, lens: &[usize]) {
        let n: usize = lens.iter().sum();
        let b = &self.blocks[l];
        let (d, dh, eps) = (self.d, self.dh, self.eps);
        let (h, kv, ff) = (b.heads, b.kv, b.ff);
        let (aw, w3, per) = (h * dh, (h + 2 * kv) * dh, h / kv);
        let scale = 1.0 / (dh as f32).sqrt();
        r.norm_fwd((x, 0), &b.n1, (&t.a, 0), Some(((&t.xh1, 0), (&t.rs1, 0))), n, d, eps);
        r.gemm(
            View::rows(&t.a, d),
            View::rows(&b.wqkv, w3),
            View::rows(&t.qkv, w3),
            n,
            d,
            w3,
            1,
            false,
            None,
        );
        if let Some(bq) = &b.bqkv {
            r.add_rows(&t.qkv, bq, n * w3, w3);
        }
        let (qd, kd) = if self.qk_norm { (&t.q0, &t.k0) } else { (&t.q, &t.k) };
        // where each window starts: its first row, and its attention square
        let mut at = Vec::with_capacity(lens.len());
        let (mut s, mut sq) = (0, 0);
        for &m in lens {
            at.push((s, sq, m));
            s += m;
            sq += m * m;
        }
        for &(s, _, m) in &at {
            let (qo, ko) = (h * dh * s, kv * dh * s);
            r.copy_strided((&t.qkv, s * w3, dh, w3), (qd, qo, m * dh, dh), dh, m, h);
            r.copy_strided((&t.qkv, s * w3 + aw, dh, w3), (kd, ko, m * dh, dh), dh, m, kv);
            r.copy_strided(
                (&t.qkv, s * w3 + aw + kv * dh, dh, w3),
                (&t.v, ko, m * dh, dh),
                dh,
                m,
                kv,
            );
        }
        if self.qk_norm {
            let (qn, kn) = (b.qn.as_ref().expect("qn"), b.kn.as_ref().expect("kn"));
            r.norm_fwd(
                (&t.q0, 0),
                qn,
                (&t.q, 0),
                Some(((&t.qxh, 0), (&t.qrs, 0))),
                h * n,
                dh,
                eps,
            );
            r.norm_fwd(
                (&t.k0, 0),
                kn,
                (&t.k, 0),
                Some(((&t.kxh, 0), (&t.krs, 0))),
                kv * n,
                dh,
                eps,
            );
        }
        for &(s, sq, m) in &at {
            let (qo, ko, po) = (h * dh * s, kv * dh * s, h * sq);
            r.rope((&t.q, qo), &self.cos, &self.sin, h * m, dh / 2, m, 0, false);
            r.rope((&t.k, ko), &self.cos, &self.sin, kv * m, dh / 2, m, 0, false);
            let pv_a = View::of(&t.p, po, m, 1, m * m);
            let pv_b = View::of(&t.v, ko, dh, 1, m * dh).per(per);
            let pv_y = View::of(&t.ctx, s * aw, aw, 1, dh);
            if self.causal {
                r.scores(
                    (&t.q, qo, m * dh),
                    (&t.k, ko, m * dh),
                    (&t.p, po),
                    m,
                    m,
                    dh,
                    h,
                    0,
                    scale,
                    per,
                );
                r.softmax((&t.p, po), h * m, m, Some((m, 0)));
                r.gemm(pv_a, pv_b, pv_y, m, m, dh, h, false, Some(0));
            } else {
                let qa = View::of(&t.q, qo, dh, 1, m * dh);
                let kb = View::of(&t.k, ko, 1, dh, m * dh).per(per);
                r.gemm(qa, kb, View::of(&t.p, po, m, 1, m * m), m, dh, m, h, false, None);
                r.scale((&t.p, po), scale, h * m * m);
                r.softmax((&t.p, po), h * m, m, None);
                r.gemm(pv_a, pv_b, pv_y, m, m, dh, h, false, None);
            }
        }
        r.gemm(
            View::rows(&t.ctx, aw),
            View::rows(&b.wo, d),
            View::rows(&t.o, d),
            n,
            aw,
            d,
            1,
            false,
            None,
        );
        if let Some(m) = &t.m1 {
            r.mul_into(&t.o, m, n * d);
        }
        r.add(x, &t.o, &t.hres, n * d);
        r.norm_fwd(
            (&t.hres, 0),
            &b.n2,
            (&t.bn, 0),
            Some(((&t.xh2, 0), (&t.rs2, 0))),
            n,
            d,
            eps,
        );
        r.gemm(
            View::rows(&t.bn, d),
            View::rows(&b.wg, ff),
            View::rows(&t.gg, ff),
            n,
            d,
            ff,
            1,
            false,
            None,
        );
        r.gemm(
            View::rows(&t.bn, d),
            View::rows(&b.wu, ff),
            View::rows(&t.u, ff),
            n,
            d,
            ff,
            1,
            false,
            None,
        );
        r.swiglu(&t.gg, &t.u, &t.s, &t.act, n * ff);
        r.gemm(
            View::rows(&t.act, ff),
            View::rows(&b.wd, d),
            View::rows(&t.dn, d),
            n,
            ff,
            d,
            1,
            false,
            None,
        );
        if let Some(m) = &t.m2 {
            r.mul_into(&t.dn, m, n * d);
        }
        r.add(&t.hres, &t.dn, out, n * d);
    }

    /// the final norm (with its x̂, rstd) and, with a generative head, its logits
    pub(crate) fn top_fwd(
        &self,
        r: &mut Run,
        x: &Buf,
        xf: &Buf,
        nf: Option<(&Buf, &Buf)>,
        genh: Option<&Buf>,
        n: usize,
    ) {
        r.norm_fwd(
            (x, 0),
            &self.nf,
            (xf, 0),
            nf.map(|(a, b)| ((a, 0), (b, 0))),
            n,
            self.d,
            self.eps,
        );
        if let (Some(y), Some((w, b))) = (genh, &self.genh) {
            let go = self.gen_out;
            r.gemm(
                View::rows(xf, self.d),
                View::rows(w, go),
                View::rows(y, go),
                n,
                self.d,
                go,
                1,
                false,
                None,
            );
            r.add_rows(y, b, n * go, go);
        }
    }

    /// one window through the trunk: the final normed rows [n × d] and, when asked
    /// and the net has one, the generative logits [n × gen_out]
    pub fn forward(&self, net: &Net, w: &Window, gains: &Gains, want_gen: bool) -> (Vec<f32>, Option<Vec<f32>>) {
        let n = w.len();
        assert!(n <= self.max_len, "window longer than max_len");
        let g = self.g;
        let d = self.d;
        let x0 = g.upload(&net.embed(w, gains));
        let x1 = g.zeros(n * d);
        let mut r = g.run(self.prec);
        let mut acts: Option<Acts> = None;
        let (mut xa, mut xb) = (&x0, &x1);
        for l in 0..self.blocks.len() {
            // blocks of the same shape share one set of activation buffers
            let same = acts
                .as_ref()
                .is_some_and(|a| a.qkv.len == n * self.w3(l) && a.gg.len == n * self.blocks[l].ff);
            if !same {
                acts = Some(self.acts(l, n, false));
            }
            self.block_fwd(&mut r, l, xa, xb, acts.as_ref().unwrap(), n);
            std::mem::swap(&mut xa, &mut xb);
        }
        let xf = g.zeros(n * d);
        let genh = (want_gen && self.genh.is_some()).then(|| g.zeros(n * self.gen_out));
        self.top_fwd(&mut r, xa, &xf, None, genh.as_ref(), n);
        let t0 = r.read(&xf, 0, n * d);
        let t1 = genh.as_ref().map(|y| r.read(y, 0, y.len));
        let mut out = r.submit();
        let yv = t1.map(|i| std::mem::take(&mut out[i]));
        (std::mem::take(&mut out[t0]), yv)
    }

    pub(crate) fn gen_bufs(&self) -> Option<(&Buf, &Buf)> {
        self.genh.as_ref().map(|(w, b)| (w, b))
    }

    pub(crate) fn nf_buf(&self) -> &Buf {
        &self.nf
    }

    pub(crate) fn cos_buf(&self) -> &Buf {
        &self.cos
    }

    pub(crate) fn sin_buf(&self) -> &Buf {
        &self.sin
    }

    pub(crate) fn block_bufs(&self, l: usize) -> BlockBufs<'_> {
        let b = &self.blocks[l];
        BlockBufs {
            n1: &b.n1,
            wqkv: &b.wqkv,
            qn: b.qn.as_ref(),
            kn: b.kn.as_ref(),
            wo: &b.wo,
            n2: &b.n2,
            wg: &b.wg,
            wu: &b.wu,
            wd: &b.wd,
        }
    }

    fn w3(&self, l: usize) -> usize {
        let b = &self.blocks[l];
        (b.heads + 2 * b.kv) * self.dh
    }

    /// every head's logits for a window, computed on the CPU from the device's rows
    /// as `Net::forward` and `Net::logits` do
    pub fn head_logits(&self, net: &Net, w: &Window, extra: &[f32], gains: &Gains) -> Vec<Vec<f32>> {
        let (xf, _) = self.forward(net, w, gains, false);
        let z = head_input(net, w, &xf, extra, gains);
        (0..net.heads.len()).map(|h| head_logits(net, &z, h)).collect()
    }

    /// the mean of the heads' logits over every window of some data
    /// (`train::forward_text` on the device)
    pub fn text_logits(&self, net: &Net, data: &[u8], extra: &[f32], gains: &Gains) -> Vec<Vec<f32>> {
        self.text_runs(net, &[data], &[extra], gains).swap_remove(0).logits
    }

    /// whether `n` packed rows whose attention squares add up to `sq` fit one submission
    fn fits_pack(&self, n: usize, sq: usize) -> bool {
        let max = self.g.max_floats();
        let mut total = 3 * n * self.d;
        for b in &self.blocks {
            let w3 = (b.heads + 2 * b.kv) * self.dh;
            let widest = [b.heads * sq, n * w3, n * b.ff, n * self.d]
                .into_iter()
                .max()
                .unwrap_or(0);
            if widest > max {
                return false;
            }
            let qk = if self.qk_norm { 3 } else { 1 };
            let block = n * (9 * self.d + w3 + 2 + qk * (b.heads + 2 * b.kv) * self.dh + 4 * b.ff) + b.heads * sq;
            total = total.max(3 * n * self.d + block);
        }
        total <= PACK_FLOATS
    }

    /// many windows through the trunk, packed into as few submissions as fit
    /// ([`PACK_FLOATS`] of activations, at most [`PACK_WINDOWS`] windows each): every
    /// window's final normed rows [n × d], the same values `forward` gives one by one
    pub fn forward_many(&self, net: &Net, ws: &[&Window], gains: &Gains) -> Vec<Vec<f32>> {
        let mut out = Vec::with_capacity(ws.len());
        let mut i = 0;
        while i < ws.len() {
            let (mut n, mut sq, mut j) = (0, 0, i);
            while j < ws.len() && j - i < PACK_WINDOWS {
                let m = ws[j].len();
                if j > i && !self.fits_pack(n + m, sq + m * m) {
                    break;
                }
                n += m;
                sq += m * m;
                j += 1;
            }
            out.extend(self.forward_pack(net, &ws[i..j], gains, n, sq));
            i = j;
        }
        out
    }

    fn forward_pack(&self, net: &Net, ws: &[&Window], gains: &Gains, n: usize, sq: usize) -> Vec<Vec<f32>> {
        let g = self.g;
        let d = self.d;
        let lens: Vec<usize> = ws.iter().map(|w| w.len()).collect();
        let mut emb = Vec::with_capacity(n * d);
        for w in ws {
            assert!(w.len() <= self.max_len, "window longer than max_len");
            emb.extend(net.embed(w, gains));
        }
        let x0 = g.upload(&emb);
        let x1 = g.zeros(n * d);
        let mut r = g.run(self.prec);
        let mut acts: Option<Acts> = None;
        let (mut xa, mut xb) = (&x0, &x1);
        for l in 0..self.blocks.len() {
            let same = acts
                .as_ref()
                .is_some_and(|a| a.qkv.len == n * self.w3(l) && a.gg.len == n * self.blocks[l].ff);
            if !same {
                acts = Some(self.acts_for(l, n, sq, false));
            }
            self.block_fwd_packed(&mut r, l, xa, xb, acts.as_ref().unwrap(), &lens);
            std::mem::swap(&mut xa, &mut xb);
        }
        let xf = g.zeros(n * d);
        self.top_fwd(&mut r, xa, &xf, None, None, n);
        let t0 = r.read(&xf, 0, n * d);
        let all = r.submit().swap_remove(t0);
        let mut out = Vec::with_capacity(ws.len());
        let mut at = 0;
        for m in lens {
            out.push(all[at..at + m * d].to_vec());
            at += m * d;
        }
        out
    }

    /// many texts at once, every window of all of them packed through the trunk: per
    /// text, each head's logits and the pooled vector averaged over its windows in
    /// window order, as `Classifier::run_bytes` computes them on the CPU. `extras` is one
    /// per text.
    pub fn text_runs(&self, net: &Net, texts: &[&[u8]], extras: &[&[f32]], gains: &Gains) -> Vec<TextRun> {
        assert_eq!(texts.len(), extras.len(), "one extra input per text");
        let per: Vec<Vec<Window>> = texts.iter().map(|t| net.windows_of(t).0).collect();
        let flat: Vec<&Window> = per.iter().flatten().collect();
        let rows = self.forward_many(net, &flat, gains);
        let mut rows = rows.into_iter();
        per.iter()
            .zip(extras)
            .map(|(ws, extra)| {
                let mut logits: Vec<Vec<f32>> = net.heads.iter().map(|h| vec![0.0; h.width()]).collect();
                let mut pool_sum = vec![0f32; net.cfg.pooled()];
                for w in ws {
                    let xf = rows.next().expect("a window's rows");
                    let (_, pv) = pooled(net, &xf, w.len());
                    let z = head_input(net, w, &xf, extra, gains);
                    for (h, acc) in logits.iter_mut().enumerate() {
                        acc.iter_mut().zip(head_logits(net, &z, h)).for_each(|(a, b)| *a += b);
                    }
                    pool_sum.iter_mut().zip(&pv).for_each(|(a, b)| *a += b);
                }
                let nw = ws.len() as f32;
                logits.iter_mut().flatten().for_each(|v| *v /= nw);
                pool_sum.iter_mut().for_each(|v| *v /= nw);
                TextRun {
                    logits,
                    pooled: pool_sum,
                    windows: ws.len(),
                }
            })
            .collect()
    }
}

/// most activation floats one packed submission holds (128 MiB)
pub const PACK_FLOATS: usize = 32 << 20;
/// most windows one packed submission holds
pub const PACK_WINDOWS: usize = 256;

/// a text's run on the device: what `Classifier::run_bytes` gives on the CPU
#[derive(Clone, Debug)]
pub struct TextRun {
    /// per head, the mean of the windows' logits
    pub logits: Vec<Vec<f32>>,
    /// the mean of the windows' pooled vectors
    pub pooled: Vec<f32>,
    pub windows: usize,
}

/// pooled rows, the side vector and the extra inputs: the heads' input, as the tail of
/// `Net::forward` builds it (inference: no dropout)
pub(crate) fn pooled(net: &Net, xf: &[f32], n: usize) -> (Vec<f32>, Vec<f32>) {
    let d = net.cfg.d_model;
    let mean = || {
        let mut m = vec![0f32; d];
        for row in xf.chunks_exact(d) {
            m.iter_mut().zip(row).for_each(|(a, b)| *a += b);
        }
        m.iter_mut().for_each(|a| *a /= n as f32);
        m
    };
    let mut pool_p = Vec::new();
    let pooled = match net.cfg.pooling {
        Pooling::Cls => xf[..d].to_vec(),
        Pooling::Mean => mean(),
        Pooling::ClsMean => [&xf[..d], &mean()[..]].concat(),
        Pooling::Attention => {
            pool_p = xf.chunks_exact(d).map(|r| k::dot(r, &net.pool_q)).collect();
            k::softmax_rows(&mut pool_p, n);
            let mut m = vec![0f32; d];
            for (row, &p) in xf.chunks_exact(d).zip(&pool_p) {
                k::axpy(&mut m, p, row);
            }
            m
        }
    };
    (pool_p, pooled)
}

fn head_input(net: &Net, w: &Window, xf: &[f32], extra: &[f32], gains: &Gains) -> Vec<f32> {
    let (_, mut z) = pooled(net, xf, w.len());
    if net.cfg.side {
        z.extend(w.side.iter().map(|v| v * gains.side));
    }
    assert_eq!(extra.len(), net.cfg.extra, "extra inputs");
    z.extend(extra.iter().map(|v| v * gains.extra));
    z
}

fn head_logits(net: &Net, z: &[f32], h: usize) -> Vec<f32> {
    let hd = &net.heads[h];
    let mut y = k::matmul(k::rows(z, z.len()), &hd.w, 1, z.len(), hd.width(), None);
    y.iter_mut().zip(&hd.b).for_each(|(a, b)| *a += b);
    y
}

/// the embedding of the token a decoder reads at position `seq.len()`, with the feature
/// channels a causal window gives it there (as `Decoder::step` computes them)
fn step_embedding(net: &Net, seq: &[u32], token: u32, gains: &Gains) -> Vec<f32> {
    let cfg = &net.cfg;
    let pos = seq.len();
    let mut w1 = Window {
        tokens: vec![token],
        letter: vec![0],
        ordinal: vec![0],
        reversed: vec![0],
        atbash: vec![0],
        root: vec![0],
        ..Window::default()
    };
    if pos > 0 && (cfg.letters || cfg.words) {
        let bytes: Vec<u8> = seq.iter().map(|&t| t as u8).collect();
        let w = text::window_with(&bytes, true, false);
        w1.letter[0] = w.letter[pos];
        w1.ordinal[0] = w.ordinal[pos];
        w1.reversed[0] = w.reversed[pos];
        w1.atbash[0] = w.atbash[pos];
        w1.root[0] = w.root[pos];
    }
    net.embed(&w1, gains)
}

/// `model::Decoder` on the device: one token at a time with the keys and values of
/// every block cached on the device, the same restarts when the window fills. Each
/// step is one submission and one read of the logits.
pub struct GpuDecoder<'a> {
    gn: &'a GpuNet,
    net: &'a Net,
    gains: Gains,
    seq: Vec<u32>,
    /// per block: keys and values [kv × max_len × dh]
    kc: Vec<Buf>,
    vc: Vec<Buf>,
    logits: Vec<f32>,
    st: Step,
}

/// one step's buffers (a single row)
struct Step {
    x: Buf,
    x2: Buf,
    a: Buf,
    qkv: Buf,
    q0: Buf,
    q: Buf,
    k0: Buf,
    k: Buf,
    p: Buf,
    ctx: Buf,
    o: Buf,
    hres: Buf,
    bn: Buf,
    gg: Buf,
    u: Buf,
    s: Buf,
    act: Buf,
    dn: Buf,
    xf: Buf,
    y: Buf,
}

impl<'a> GpuDecoder<'a> {
    /// a decoder that has read the start token and then `prompt` (bytes, or through
    /// the net's tokenizer), as `Decoder::new`
    pub fn new(gn: &'a GpuNet, net: &'a Net, gains: Gains, prompt: &[u8]) -> GpuDecoder<'a> {
        let mut dec = GpuDecoder::empty(gn, net, gains);
        let (ts, _) = net.sequence(prompt, 0);
        for t in ts {
            dec.push_token(t);
        }
        dec
    }

    /// a decoder that has read the start token and these token ids
    pub fn from_tokens(gn: &'a GpuNet, net: &'a Net, gains: Gains, prompt: &[u32]) -> GpuDecoder<'a> {
        let mut dec = GpuDecoder::empty(gn, net, gains);
        for &t in prompt {
            dec.push_token(t);
        }
        dec
    }

    fn empty(gn: &'a GpuNet, net: &'a Net, gains: Gains) -> GpuDecoder<'a> {
        assert!(
            net.cfg.causal && net.generator.is_some(),
            "decoding needs a causal model with a generative head"
        );
        let g = gn.g;
        let (d, dh, ml) = (gn.d, gn.dh, gn.max_len);
        let hmax = gn.blocks.iter().map(|b| b.heads).max().unwrap_or(1);
        let kvmax = gn.blocks.iter().map(|b| b.kv).max().unwrap_or(1);
        let ffmax = gn.blocks.iter().map(|b| b.ff).max().unwrap_or(1);
        let w3max = (0..gn.blocks.len()).map(|l| gn.w3(l)).max().unwrap_or(1);
        let st = Step {
            x: g.zeros(d),
            x2: g.zeros(d),
            a: g.zeros(d),
            qkv: g.zeros(w3max),
            q0: g.zeros(hmax * dh),
            q: g.zeros(hmax * dh),
            k0: g.zeros(kvmax * dh),
            k: g.zeros(kvmax * dh),
            p: g.zeros(hmax * ml),
            ctx: g.zeros(hmax * dh),
            o: g.zeros(d),
            hres: g.zeros(d),
            bn: g.zeros(d),
            gg: g.zeros(ffmax),
            u: g.zeros(ffmax),
            s: g.zeros(ffmax),
            act: g.zeros(ffmax),
            dn: g.zeros(d),
            xf: g.zeros(d),
            y: g.zeros(gn.gen_out),
        };
        let mut dec = GpuDecoder {
            gn,
            net,
            gains,
            seq: Vec::new(),
            kc: gn.blocks.iter().map(|b| g.zeros(b.kv * ml * dh)).collect(),
            vc: gn.blocks.iter().map(|b| g.zeros(b.kv * ml * dh)).collect(),
            logits: Vec::new(),
            st,
        };
        dec.restart(&[]);
        dec
    }

    pub fn logits(&self) -> &[f32] {
        &self.logits
    }

    pub fn tokens(&self) -> &[u32] {
        &self.seq
    }

    pub fn window(&self) -> Vec<u8> {
        self.net.token_bytes(&self.seq)
    }

    fn restart(&mut self, keep: &[u32]) {
        // the cache rows past the position are never read, so nothing to clear
        self.seq.clear();
        self.step(self.net.start_token());
        for &t in keep {
            self.seq.push(t);
            self.step(t);
        }
    }

    pub fn push(&mut self, b: u8) {
        self.push_token(b as u32);
    }

    pub fn push_token(&mut self, t: u32) {
        let cfg = &self.net.cfg;
        if self.seq.len() + 1 >= cfg.max_len {
            let keep = self.seq[self.seq.len() - cfg.room() / 2..].to_vec();
            self.restart(&keep);
        }
        self.seq.push(t);
        self.step(t);
    }

    fn step(&mut self, token: u32) {
        let (gn, net, st) = (self.gn, self.net, &self.st);
        let cfg = &net.cfg;
        let (d, dh, eps, ml) = (gn.d, gn.dh, gn.eps, gn.max_len);
        let pos = self.seq.len();
        let x = step_embedding(net, &self.seq, token, &self.gains);
        let g = gn.g;
        g.write(&st.x, 0, &x);
        let mut r = g.run(gn.prec);
        let scale = 1.0 / (dh as f32).sqrt();
        let (mut xa, mut xb) = (&st.x, &st.x2);
        for (l, b) in gn.blocks.iter().enumerate() {
            let (h, kv, ff) = (b.heads, b.kv, b.ff);
            let (aw, w3, per) = (h * dh, gn.w3(l), h / kv);
            let (kc, vc) = (&self.kc[l], &self.vc[l]);
            r.norm_fwd((xa, 0), &b.n1, (&st.a, 0), None, 1, d, eps);
            r.gemm(
                View::rows(&st.a, d),
                View::rows(&b.wqkv, w3),
                View::rows(&st.qkv, w3),
                1,
                d,
                w3,
                1,
                false,
                None,
            );
            if let Some(bq) = &b.bqkv {
                r.add_rows(&st.qkv, bq, w3, w3);
            }
            let (qd, kd) = if cfg.qk_norm { (&st.q0, &st.k0) } else { (&st.q, &st.k) };
            r.copy_strided((&st.qkv, 0, dh, w3), (qd, 0, dh, dh), dh, 1, h);
            r.copy_strided((&st.qkv, aw, dh, w3), (kd, 0, dh, dh), dh, 1, kv);
            r.copy_strided((&st.qkv, aw + kv * dh, dh, w3), (vc, pos * dh, ml * dh, dh), dh, 1, kv);
            if cfg.qk_norm {
                let (qn, kn) = (b.qn.as_ref().expect("qn"), b.kn.as_ref().expect("kn"));
                r.norm_fwd((&st.q0, 0), qn, (&st.q, 0), None, h, dh, eps);
                r.norm_fwd((&st.k0, 0), kn, (&st.k, 0), None, kv, dh, eps);
            }
            r.rope((&st.q, 0), &gn.cos, &gn.sin, h, dh / 2, 1, pos, false);
            r.rope((&st.k, 0), &gn.cos, &gn.sin, kv, dh / 2, 1, pos, false);
            r.copy_strided((&st.k, 0, dh, dh), (kc, pos * dh, ml * dh, dh), dh, 1, kv);
            let w = pos + 1;
            r.scores(
                (&st.q, 0, dh),
                (kc, 0, ml * dh),
                (&st.p, 0),
                1,
                w,
                dh,
                h,
                pos,
                scale,
                per,
            );
            r.softmax((&st.p, 0), h, w, None);
            r.gemm(
                View::of(&st.p, 0, w, 1, w),
                View::of(vc, 0, dh, 1, ml * dh).per(per),
                View::of(&st.ctx, 0, aw, 1, dh),
                1,
                w,
                dh,
                h,
                false,
                None,
            );
            r.gemm(
                View::rows(&st.ctx, aw),
                View::rows(&b.wo, d),
                View::rows(&st.o, d),
                1,
                aw,
                d,
                1,
                false,
                None,
            );
            r.add(xa, &st.o, &st.hres, d);
            r.norm_fwd((&st.hres, 0), &b.n2, (&st.bn, 0), None, 1, d, eps);
            r.gemm(
                View::rows(&st.bn, d),
                View::rows(&b.wg, ff),
                View::rows(&st.gg, ff),
                1,
                d,
                ff,
                1,
                false,
                None,
            );
            r.gemm(
                View::rows(&st.bn, d),
                View::rows(&b.wu, ff),
                View::rows(&st.u, ff),
                1,
                d,
                ff,
                1,
                false,
                None,
            );
            r.swiglu(&st.gg, &st.u, &st.s, &st.act, ff);
            r.gemm(
                View::rows(&st.act, ff),
                View::rows(&b.wd, d),
                View::rows(&st.dn, d),
                1,
                ff,
                d,
                1,
                false,
                None,
            );
            r.add(&st.hres, &st.dn, xb, d);
            std::mem::swap(&mut xa, &mut xb);
        }
        gn.top_fwd(&mut r, xa, &st.xf, None, Some(&st.y), 1);
        let t = r.read(&st.y, 0, gn.gen_out);
        self.logits = r.submit().swap_remove(t);
    }
}
