// The engine's kernels as WGSL compute shaders (native through wgpu; the same source is
// valid WebGPU). Each mirrors a function of `kernels.rs` / `model.rs` operation for
// operation, so that on a device that rounds every + − × ÷ √ once (IEEE, no fused
// multiply-add) the results are the CPU's bits:
//
// - products: each output element is one chain in ascending contraction index, from 0
//   (or from y for accumulating products); threads split outputs, never a chain;
// - vector reductions: 8 lanes, element i to lane i mod 8 in order, folded as
//   ((l0+l4)+(l1+l5)) + ((l2+l6)+(l3+l7));
// - exp and the logistic function are `math::exp` / `math::sigmoid`, never built-ins.
//
// The prelude Rust puts before this file defines `mul`, `div` and `sqrtf` and the
// constants (as hex literals, so no decimal rounding happens in the shader compiler).
// Exact prelude: `mul` launders each product through an integer XOR with a uniform
// zero (`p.u3.w`), so the compiler cannot fuse it into the following add, and `div` /
// `sqrtf` are correctly rounded in integer arithmetic. Fast prelude: plain operators.
//
// Bindings: 0 the parameters (dynamic offset), 1..6 storage buffers of f32; a kernel
// uses the ones it names, the rest are bound to a dummy.

struct P {
    u0: vec4<u32>,
    u1: vec4<u32>,
    u2: vec4<u32>,
    u3: vec4<u32>,
    u4: vec4<u32>,
    f0: vec4<f32>,
}

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> b1: array<f32>;
@group(0) @binding(2) var<storage, read_write> b2: array<f32>;
@group(0) @binding(3) var<storage, read_write> b3: array<f32>;
@group(0) @binding(4) var<storage, read_write> b4: array<f32>;
@group(0) @binding(5) var<storage, read_write> b5: array<f32>;
@group(0) @binding(6) var<storage, read_write> b6: array<f32>;

// ---------------------------------------------------------------- exact arithmetic

/// a / b correctly rounded (to nearest, ties to even) for normal operands and results,
/// by long division of the significands; anything else goes to the hardware divide
fn div_sw(a: f32, b: f32) -> f32 {
    let ua = bitcast<u32>(a);
    let ub = bitcast<u32>(b);
    let ea = (ua >> 23u) & 0xffu;
    let eb = (ub >> 23u) & 0xffu;
    if (ea == 0u || ea == 255u || eb == 0u || eb == 255u) {
        return a / b;
    }
    let sign = (ua ^ ub) & 0x80000000u;
    let ma = (ua & 0x7fffffu) | 0x800000u;
    let mb = (ub & 0x7fffffu) | 0x800000u;
    var e = i32(ea) - i32(eb) + 127;
    var rem = ma;
    if (rem < mb) {
        rem = rem << 1u;
        e = e - 1;
    }
    var q = 0u;
    for (var i = 0u; i < 25u; i = i + 1u) {
        q = q << 1u;
        if (rem >= mb) {
            rem = rem - mb;
            q = q | 1u;
        }
        rem = rem << 1u;
    }
    var m = q >> 1u;
    if ((q & 1u) == 1u && (rem != 0u || (m & 1u) == 1u)) {
        m = m + 1u;
    }
    if (m == 0x1000000u) {
        m = m >> 1u;
        e = e + 1;
    }
    if (e <= 0 || e >= 255) {
        return a / b;
    }
    return bitcast<f32>(sign | (u32(e) << 23u) | (m & 0x7fffffu));
}

/// √x correctly rounded for positive normal x, digit by digit; anything else goes to
/// the hardware square root
fn sqrt_sw(x: f32) -> f32 {
    let u = bitcast<u32>(x);
    let ex = (u >> 23u) & 0xffu;
    if (ex == 0u || ex == 255u || (u >> 31u) == 1u) {
        return sqrt(x);
    }
    var m = (u & 0x7fffffu) | 0x800000u;
    var e = i32(ex) - 127;
    if ((e & 1) != 0) {
        m = m << 1u;
        e = e - 1;
    }
    // r = ⌊√(m·2^25)⌋, 25 bits, from the 50-bit radicand two bits at a time
    var r = 0u;
    var rem = 0u;
    for (var i = 0u; i < 25u; i = i + 1u) {
        let j = 49u - 2u * i;
        var two = 0u;
        if (j >= 25u) {
            two = ((m >> (j - 25u)) & 1u) << 1u;
        }
        if (j - 1u >= 25u) {
            two = two | ((m >> (j - 26u)) & 1u);
        }
        rem = (rem << 2u) | two;
        let trial = (r << 2u) | 1u;
        if (rem >= trial) {
            rem = rem - trial;
            r = (r << 1u) | 1u;
        } else {
            r = r << 1u;
        }
    }
    var mm = r >> 1u;
    if ((r & 1u) == 1u && (rem != 0u || (mm & 1u) == 1u)) {
        mm = mm + 1u;
    }
    var be = e / 2 + 127;
    if (mm == 0x1000000u) {
        mm = mm >> 1u;
        be = be + 1;
    }
    return bitcast<f32>((u32(be) << 23u) | (mm & 0x7fffffu));
}

// ---------------------------------------------------------------- math.rs

/// x, opaque to the optimiser (an XOR with the uniform zero): stops it simplifying
/// `(x + C) − C` to x, which the rounding trick in `expf` depends on in either precision
fn keep(x: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(x) ^ p.u3.w);
}

fn pow2(k: i32) -> f32 {
    return bitcast<f32>(u32(k + 127) << 23u);
}

/// `math::exp`, operation for operation
fn expf(x: f32) -> f32 {
    let c = clamp(x, EXP_LO, EXP_HI);
    let k = keep(mul(c, LOG2E) + ROUND) - ROUND;
    let r = (c - mul(k, LN2_HI)) - mul(k, LN2_LO);
    let q = 1.0 + mul(r, 1.0 + mul(r, 0.5 + mul(r, C6 + mul(r, C24 + mul(r, C120 + mul(r, C720 + mul(r, C5040)))))));
    let ki = i32(k);
    let h = ki >> 1u;
    let v = q * pow2(h) * pow2(ki - h);
    if (x < EXP_LO) {
        return 0.0;
    }
    if (x > EXP_HI) {
        return bitcast<f32>(0x7f800000u);
    }
    return v;
}

/// `math::sigmoid`
fn sigm(x: f32) -> f32 {
    let e = expf(-abs(x));
    let d = 1.0 + e;
    if (x >= 0.0) {
        return div(1.0, d);
    }
    return div(e, d);
}

fn fold8(l: array<f32, 8>) -> f32 {
    return ((l[0] + l[4]) + (l[1] + l[5])) + ((l[2] + l[6]) + (l[3] + l[7]));
}

fn flat(wid: vec3<u32>, nw: vec3<u32>, li: u32, size: u32) -> u32 {
    return (wid.x + wid.y * nw.x) * size + li;
}

fn neg_inf() -> f32 {
    return bitcast<f32>(0xff800000u);
}

// ---------------------------------------------------------------- products

var<workgroup> ta: array<f32, 1024>;
var<workgroup> tb: array<f32, 1024>;

// y[z][r][c] (+)= Σ_p A[z][r][p]·B[z][p][c], each element one chain in ascending p.
// u0 (n, k, m, flags: 1 accumulate, 2 causal: p ≤ t0 + r only)
// u1 (a_off, a_rs, a_ps, a_zs)  u2 (b_off, b_ps, b_cs, b_zs)  u3 (y_off, y_rs, y_zs, ·)
// u4 (t0, batch, per, ·): B's batch index is z / per (grouped keys and values); b1 A, b2 B, b3 y. Tiles of 64×64 outputs, 4×4 per thread, p in
// steps of 16 through workgroup memory (which keeps every chain in order).
@compute @workgroup_size(16, 16)
fn gemm(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let n = p.u0.x;
    let k = p.u0.y;
    let m = p.u0.z;
    let acc_in = (p.u0.w & 1u) == 1u;
    let causal = (p.u0.w & 2u) == 2u;
    let t0 = p.u4.x;
    let z = wid.z;
    let ab = p.u1.x + z * p.u1.w;
    let bb = p.u2.x + (z / max(p.u4.z, 1u)) * p.u2.w;
    let yb = p.u3.x + z * p.u3.z;
    let r0 = wid.y * 64u;
    let c0 = wid.x * 64u;
    var acc: array<array<f32, 4>, 4>;
    for (var i = 0u; i < 4u; i = i + 1u) {
        for (var j = 0u; j < 4u; j = j + 1u) {
            let r = r0 + lid.y + 16u * i;
            let c = c0 + lid.x + 16u * j;
            var v = 0.0;
            if (acc_in && r < n && c < m) {
                v = b3[yb + r * p.u3.y + c];
            }
            acc[i][j] = v;
        }
    }
    for (var p0 = 0u; p0 < k; p0 = p0 + 16u) {
        for (var l = 0u; l < 4u; l = l + 1u) {
            let e = li + l * 256u;
            let rr = e / 16u;
            let pp = e % 16u;
            var va = 0.0;
            if (r0 + rr < n && p0 + pp < k) {
                va = b1[ab + (r0 + rr) * p.u1.y + (p0 + pp) * p.u1.z];
            }
            ta[e] = va;
            let pb = e / 64u;
            let cc = e % 64u;
            var vb = 0.0;
            if (p0 + pb < k && c0 + cc < m) {
                vb = b2[bb + (p0 + pb) * p.u2.y + (c0 + cc) * p.u2.z];
            }
            tb[e] = vb;
        }
        workgroupBarrier();
        let pend = min(16u, k - p0);
        for (var pp = 0u; pp < pend; pp = pp + 1u) {
            var bv: array<f32, 4>;
            for (var j = 0u; j < 4u; j = j + 1u) {
                bv[j] = tb[pp * 64u + lid.x + 16u * j];
            }
            for (var i = 0u; i < 4u; i = i + 1u) {
                let av = ta[(lid.y + 16u * i) * 16u + pp];
                if (!causal || p0 + pp <= t0 + r0 + lid.y + 16u * i) {
                    for (var j = 0u; j < 4u; j = j + 1u) {
                        acc[i][j] = acc[i][j] + mul(av, bv[j]);
                    }
                }
            }
        }
        workgroupBarrier();
    }
    for (var i = 0u; i < 4u; i = i + 1u) {
        for (var j = 0u; j < 4u; j = j + 1u) {
            let r = r0 + lid.y + 16u * i;
            let c = c0 + lid.x + 16u * j;
            if (r < n && c < m) {
                b3[yb + r * p.u3.y + c] = acc[i][j];
            }
        }
    }
}

// the same product, one thread per output (few rows: decoding, the heads)
@compute @workgroup_size(64)
fn gemm_small(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let n = p.u0.x;
    let k = p.u0.y;
    let m = p.u0.z;
    let i = flat(wid, nw, li, 64u);
    let zc = p.u4.y;
    if (i >= zc * n * m) {
        return;
    }
    let z = i / (n * m);
    let r = (i / m) % n;
    let c = i % m;
    let ab = p.u1.x + z * p.u1.w + r * p.u1.y;
    let bb = p.u2.x + (z / max(p.u4.z, 1u)) * p.u2.w + c * p.u2.z;
    let yi = p.u3.x + z * p.u3.z + r * p.u3.y + c;
    var v = 0.0;
    if ((p.u0.w & 1u) == 1u) {
        v = b3[yi];
    }
    var kk = k;
    if ((p.u0.w & 2u) == 2u) {
        kk = min(k, p.u4.x + r + 1u);
    }
    for (var q = 0u; q < kk; q = q + 1u) {
        v = v + mul(b1[ab + q * p.u1.z], b2[bb + q * p.u2.y]);
    }
    b3[yi] = v;
}

// causal attention scores: S[h][t][s] = dot8(q[h][t], k[h][s])·scale for s ≤ t0 + t,
// else 0; head h reads key head h / per. u0 (n_q, width, dh, heads) u1 (q_off, q_hs, k_off, k_hs)
// u2 (s_off, t0, per, ·)
// f0.x scale; b1 q, b2 k, b3 S
@compute @workgroup_size(64)
fn scores(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let nq = p.u0.x;
    let w = p.u0.y;
    let dh = p.u0.z;
    let i = flat(wid, nw, li, 64u);
    if (i >= p.u0.w * nq * w) {
        return;
    }
    let h = i / (nq * w);
    let t = (i / w) % nq;
    let s = i % w;
    var v = 0.0;
    if (s <= p.u2.y + t) {
        let qb = p.u1.x + h * p.u1.y + t * dh;
        let kb = p.u1.z + (h / max(p.u2.z, 1u)) * p.u1.w + s * dh;
        var l: array<f32, 8>;
        for (var j = 0u; j < 8u; j = j + 1u) {
            l[j] = 0.0;
        }
        for (var c = 0u; c < dh; c = c + 1u) {
            l[c % 8u] = l[c % 8u] + mul(b1[qb + c], b2[kb + c]);
        }
        v = fold8(l) * p.f0.x;
    }
    b3[p.u2.x + i] = v;
}

// ---------------------------------------------------------------- rows of 8 lanes

var<workgroup> red: array<f32, 64>;

fn fold_red(li: u32) -> f32 {
    let r = (li / 8u) * 8u;
    return ((red[r] + red[r + 4u]) + (red[r + 1u] + red[r + 5u])) + ((red[r + 2u] + red[r + 6u]) + (red[r + 3u] + red[r + 7u]));
}

// RMSNorm forward, 8 rows per workgroup, a lane per thread.
// u0 (rows, d, x_off, y_off) u1 (trace, rstd_off, xhat_off, ·) f0.x eps;
// b1 x, b2 g, b3 y, b4 x̂, b5 rstd
@compute @workgroup_size(64)
fn norm_fwd(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let rows = p.u0.x;
    let d = p.u0.y;
    let row = (wid.x + wid.y * nw.x) * 8u + li / 8u;
    let lane = li % 8u;
    let ok = row < rows;
    let xb = p.u0.z + row * d;
    var a = 0.0;
    if (ok) {
        for (var i = lane; i < d; i = i + 8u) {
            let v = b1[xb + i];
            a = a + mul(v, v);
        }
    }
    red[li] = a;
    workgroupBarrier();
    let s = fold_red(li);
    if (ok) {
        let rs = div(1.0, sqrtf(div(s, f32(d)) + p.f0.x));
        let tr = p.u1.x == 1u;
        if (tr && lane == 0u) {
            b5[p.u1.y + row] = rs;
        }
        for (var i = lane; i < d; i = i + 8u) {
            let h = mul(b1[xb + i], rs);
            if (tr) {
                b4[p.u1.z + row * d + i] = h;
            }
            b3[p.u0.w + row * d + i] = mul(h, b2[i]);
        }
    }
}

// RMSNorm backward into dx: dx̂ = dy·g, c = dot(dx̂, x̂)/d, dx += rstd·(dx̂ − x̂·c).
// u0 (rows, d, dy_off, xhat_off) u1 (rstd_off, dx_off, ·, ·);
// b1 dy, b2 x̂, b3 g, b4 rstd, b5 dx
@compute @workgroup_size(64)
fn norm_back(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let rows = p.u0.x;
    let d = p.u0.y;
    let row = (wid.x + wid.y * nw.x) * 8u + li / 8u;
    let lane = li % 8u;
    let ok = row < rows;
    let yb = p.u0.z + row * d;
    let hb = p.u0.w + row * d;
    var a = 0.0;
    if (ok) {
        for (var i = lane; i < d; i = i + 8u) {
            a = a + mul(mul(b1[yb + i], b3[i]), b2[hb + i]);
        }
    }
    red[li] = a;
    workgroupBarrier();
    let c = div(fold_red(li), f32(d));
    if (ok) {
        let s = b4[p.u1.x + row];
        let xb = p.u1.y + row * d;
        for (var i = lane; i < d; i = i + 8u) {
            let dh = mul(b1[yb + i], b3[i]);
            b5[xb + i] = b5[xb + i] + mul(s, dh - mul(b2[hb + i], c));
        }
    }
}

// dg[i] += Σ_r dy[r][i]·x̂[r][i] in row order (a column per thread); with flag 1 in
// u1.x, dg[i] += Σ_r dy[r][i] (a bias gradient). u0 (rows, d, dy_off, xhat_off)
// u1 (plain, dg_off, ·, ·); b1 dy, b2 x̂, b3 dg
@compute @workgroup_size(64)
fn col_acc(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let rows = p.u0.x;
    let d = p.u0.y;
    let i = flat(wid, nw, li, 64u);
    if (i >= d) {
        return;
    }
    var v = b3[p.u1.y + i];
    if (p.u1.x == 1u) {
        for (var r = 0u; r < rows; r = r + 1u) {
            v = v + b1[p.u0.z + r * d + i];
        }
    } else {
        for (var r = 0u; r < rows; r = r + 1u) {
            v = v + mul(b1[p.u0.z + r * d + i], b2[p.u0.w + r * d + i]);
        }
    }
    b3[p.u1.y + i] = v;
}

// softmax of rows in place: e = exp(x − max), x = e / Σe. A row's length is the width,
// or for causal rows (u1.x = 1) min(t0 + (row mod n_q) + 1, width); the rest is left.
// u0 (rows, width, off, ·) u1 (causal, n_q, t0, ·); b1 x
@compute @workgroup_size(64)
fn softmax(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let rows = p.u0.x;
    let w = p.u0.y;
    let row = (wid.x + wid.y * nw.x) * 8u + li / 8u;
    let lane = li % 8u;
    let ok = row < rows;
    var len = w;
    if (p.u1.x == 1u) {
        len = min(p.u1.z + row % p.u1.y + 1u, w);
    }
    let base = p.u0.z + row * w;
    var mx = neg_inf();
    if (ok) {
        for (var i = lane; i < len; i = i + 8u) {
            mx = max(mx, b1[base + i]);
        }
    }
    red[li] = mx;
    workgroupBarrier();
    let r8 = (li / 8u) * 8u;
    var m = red[r8];
    for (var j = 1u; j < 8u; j = j + 1u) {
        m = max(m, red[r8 + j]);
    }
    workgroupBarrier();
    var a = 0.0;
    if (ok) {
        for (var i = lane; i < len; i = i + 8u) {
            let e = expf(b1[base + i] - m);
            b1[base + i] = e;
            a = a + e;
        }
    }
    red[li] = a;
    workgroupBarrier();
    let s = fold_red(li);
    if (ok) {
        for (var i = lane; i < len; i = i + 8u) {
            b1[base + i] = div(b1[base + i], s);
        }
    }
}

// softmax backward per row over the full width: dp = p·(dp − dot(p, dp)).
// u0 (rows, width, p_off, dp_off); b1 p, b2 dp
@compute @workgroup_size(64)
fn softmax_back(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let rows = p.u0.x;
    let w = p.u0.y;
    let row = (wid.x + wid.y * nw.x) * 8u + li / 8u;
    let lane = li % 8u;
    let ok = row < rows;
    let pb = p.u0.z + row * w;
    let db = p.u0.w + row * w;
    var a = 0.0;
    if (ok) {
        for (var i = lane; i < w; i = i + 8u) {
            a = a + mul(b1[pb + i], b2[db + i]);
        }
    }
    red[li] = a;
    workgroupBarrier();
    let c = fold_red(li);
    if (ok) {
        for (var i = lane; i < w; i = i + 8u) {
            b2[db + i] = mul(b1[pb + i], b2[db + i] - c);
        }
    }
}

// out[r] = dot8(a[r], b[r]) over rows of length len. u0 (rows, len, ·, ·); b1 a, b2 b, b3 out
@compute @workgroup_size(64)
fn dot_rows(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let rows = p.u0.x;
    let len = p.u0.y;
    let r = flat(wid, nw, li, 64u);
    if (r >= rows) {
        return;
    }
    var l: array<f32, 8>;
    for (var j = 0u; j < 8u; j = j + 1u) {
        l[j] = 0.0;
    }
    for (var c = 0u; c < len; c = c + 1u) {
        l[c % 8u] = l[c % 8u] + mul(b1[r * len + c], b2[r * len + c]);
    }
    b3[r] = fold8(l);
}

// ---------------------------------------------------------------- elementwise

// u0 (count, op, a_off, b_off) u1 (y_off, ·, ·, ·) f0.x a scalar;
// ops: 0 y = a + b, 1 a += b, 2 a *= b, 3 a *= f, 4 a = f, 5 a = exp(a),
// 6 y = sigmoid(a), 7 y = a / b, 8 y = √a, 9 a[i] += b[i mod width] (u1.y = width)
@compute @workgroup_size(64)
fn ew(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let i = flat(wid, nw, li, 64u);
    if (i >= p.u0.x) {
        return;
    }
    let ai = p.u0.z + i;
    let bi = p.u0.w + i;
    let yi = p.u1.x + i;
    switch p.u0.y {
        case 0u: { b3[yi] = b1[ai] + b2[bi]; }
        case 1u: { b1[ai] = b1[ai] + b2[bi]; }
        case 2u: { b1[ai] = b1[ai] * b2[bi]; }
        case 3u: { b1[ai] = b1[ai] * p.f0.x; }
        case 4u: { b1[ai] = p.f0.x; }
        case 5u: { b1[ai] = expf(b1[ai]); }
        case 6u: { b3[yi] = sigm(b1[ai]); }
        case 7u: { b3[yi] = div(b1[ai], b2[bi]); }
        case 8u: { b3[yi] = sqrtf(b1[ai]); }
        case 9u: { b1[ai] = b1[ai] + b2[p.u0.w + i % p.u1.y]; }
        default: {}
    }
}

// SwiGLU forward: s = σ(g), act = (g·s)·u. u0 (count, ·, ·, ·); b1 g, b2 u, b3 s, b4 act
@compute @workgroup_size(64)
fn swiglu(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let i = flat(wid, nw, li, 64u);
    if (i >= p.u0.x) {
        return;
    }
    let g = b1[i];
    let s = sigm(g);
    b3[i] = s;
    b4[i] = (g * s) * b2[i];
}

// SwiGLU backward: du = dact·(g·s), dg = (dact·u)·(s·(1 + g·(1 − s))).
// u0 (count, ·, ·, ·); b1 dact, b2 g, b3 s, b4 u, b5 dg, b6 du
@compute @workgroup_size(64)
fn swiglu_back(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let i = flat(wid, nw, li, 64u);
    if (i >= p.u0.x) {
        return;
    }
    let da = b1[i];
    let g = b2[i];
    let s = b3[i];
    b6[i] = da * (g * s);
    b5[i] = (da * b4[i]) * (s * (1.0 + mul(g, 1.0 - s)));
}

// rotary positions on rows of 2·half: (a, b) → (a·c − b·s, a·s + b·c), backward the
// transpose. Row r is position t0 + (r mod per). u0 (rows, half, off, per) u1 (t0, back, ·, ·);
// b1 x, b2 cos, b3 sin ([positions × half])
@compute @workgroup_size(64)
fn rope(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let half = p.u0.y;
    let i = flat(wid, nw, li, 64u);
    if (i >= p.u0.x * half) {
        return;
    }
    let r = i / half;
    let j = i % half;
    let t = p.u1.x + r % p.u0.w;
    let xb = p.u0.z + r * 2u * half;
    let c = b2[t * half + j];
    let s = b3[t * half + j];
    let a = b1[xb + j];
    let b = b1[xb + j + half];
    if (p.u1.y == 0u) {
        b1[xb + j] = mul(a, c) - mul(b, s);
        b1[xb + j + half] = mul(a, s) + mul(b, c);
    } else {
        b1[xb + j] = mul(a, c) + mul(b, s);
        b1[xb + j + half] = mul(b, c) - mul(a, s);
    }
}

// strided copy: dst[d_off + z·d_zs + r·d_rs + c] = src[s_off + z·s_zs + r·s_rs + c].
// u0 (width, rows, zs, ·) u1 (s_off, s_zs, s_rs, ·) u2 (d_off, d_zs, d_rs, ·); b1 src, b2 dst
@compute @workgroup_size(64)
fn copy(@builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let w = p.u0.x;
    let rows = p.u0.y;
    let i = flat(wid, nw, li, 64u);
    if (i >= w * rows * p.u0.z) {
        return;
    }
    let c = i % w;
    let r = (i / w) % rows;
    let z = i / (w * rows);
    b2[p.u2.x + z * p.u2.y + r * p.u2.z + c] = b1[p.u1.x + z * p.u1.y + r * p.u1.z + c];
}
