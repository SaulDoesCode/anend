//! Deterministic arithmetic: transcendental functions built only from + − × ÷, sqrt,
//! floor and bit manipulation, so every platform (and a JS port rounding each step with
//! `Math.fround`) gets the same bits; plus the seeded random numbers and hashes.
//!
//! Nothing here calls the platform's libm. Each function documents its operation order;
//! a port must keep it, one rounding per written operation, never fusing a multiply and
//! an add.

// ---------------------------------------------------------------- f32 functions

const LOG2E: f32 = std::f32::consts::LOG2_E;
// ln 2 in two parts (Cody & Waite): `k * LN2_HI` is exact for |k| < 2^15
const LN2_HI: f32 = 0.693_359_4;
const LN2_LO: f32 = -2.121_944_4e-4;

/// 2^k for k in -126..=127, from the exponent bits
#[inline(always)]
fn pow2(k: i32) -> f32 {
    f32::from_bits(((k + 127) as u32) << 23)
}

/// adding and taking away 1.5·2^23 rounds an f32 below 2^22 to the nearest integer (ties
/// to even), with plain arithmetic
const ROUND: f32 = 12_582_912.0;

/// e^x, within about 2 ulp; 0 below −87, ∞ above 88.72.
///
/// `c = clamp(x, −87, 88.72)`, `k = (c·log2e + 1.5·2^23) − 1.5·2^23`,
/// `r = (c − k·LN2_HI) − k·LN2_LO`, then the Taylor series of e^r to r^7 by Horner from
/// the highest term, times 2^(k>>1) and 2^(k − (k>>1)). Branch free, so loops over it
/// vectorise.
#[inline(always)]
pub fn exp(x: f32) -> f32 {
    let c = x.clamp(-87.0, 88.72);
    let k = (c * LOG2E + ROUND) - ROUND;
    let r = (c - k * LN2_HI) - k * LN2_LO;
    let p = 1.0
        + r * (1.0
            + r * (0.5
                + r * (1.0 / 6.0 + r * (1.0 / 24.0 + r * (1.0 / 120.0 + r * (1.0 / 720.0 + r * (1.0 / 5040.0)))))));
    let k = k as i32;
    let h = k >> 1;
    let v = p * pow2(h) * pow2(k - h);
    if x < -87.0 {
        0.0
    } else if x > 88.72 {
        f32::INFINITY
    } else {
        v
    }
}

/// e^x − 1, accurate near zero: Taylor to x^9 for |x| < 0.5, else `exp(x) − 1`
pub fn expm1(x: f32) -> f32 {
    if x.abs() < 0.5 {
        x * (1.0
            + x * (0.5
                + x * (1.0 / 6.0
                    + x * (1.0 / 24.0
                        + x * (1.0 / 120.0
                            + x * (1.0 / 720.0 + x * (1.0 / 5040.0 + x * (1.0 / 40320.0 + x * (1.0 / 362_880.0)))))))))
    } else {
        exp(x) - 1.0
    }
}

/// natural log: x = m·2^e with m in [√½, √2), then `ln m = 2·atanh(s)`, s = (m−1)/(m+1),
/// as `s·(2 + s²·(2/3 + s²·(2/5 + s²·(2/7 + s²·2/9))))`, and `e·LN2_HI + (p + e·LN2_LO)`
pub fn ln(x: f32) -> f32 {
    if x.is_nan() || x < 0.0 {
        return f32::NAN;
    }
    if x == 0.0 {
        return f32::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    let (mut x, mut e) = (x, 0i32);
    if x.to_bits() < 0x0080_0000 {
        x *= 8_388_608.0; // 2^23, out of the subnormals
        e = -23;
    }
    let bits = x.to_bits();
    e += (bits >> 23) as i32 - 127;
    let mut m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    if m > std::f32::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let p = s * (2.0 + s2 * (2.0 / 3.0 + s2 * (2.0 / 5.0 + s2 * (2.0 / 7.0 + s2 * (2.0 / 9.0)))));
    let e = e as f32;
    e * LN2_HI + (p + e * LN2_LO)
}

/// tanh as `e/(e + 2)` with e = expm1(2|x|), the sign put back; ±1 beyond |x| = 9
pub fn tanh(x: f32) -> f32 {
    let a = x.abs();
    if a > 9.0 {
        return 1f32.copysign(x);
    }
    let e = expm1(2.0 * a);
    (e / (e + 2.0)).copysign(x)
}

/// the logistic function: with e = e^−|x|, `1/(1 + e)` for x ≥ 0 and `e/(1 + e)` below
#[inline(always)]
pub fn sigmoid(x: f32) -> f32 {
    let e = exp(-x.abs());
    let d = 1.0 + e;
    if x >= 0.0 { 1.0 / d } else { e / d }
}

/// ln(1 + e^x), stably: `max(x, 0) + ln(1 + e^−|x|)`
pub fn softplus(x: f32) -> f32 {
    x.max(0.0) + ln(1.0 + exp(-x.abs()))
}

// ---------------------------------------------------------------- f64 functions

const LN2_HI64: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
const LN2_LO64: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);

/// e^x in f64: Cody–Waite reduction by ln 2 and Taylor to r^13
pub fn exp64(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x < -708.0 {
        return 0.0;
    }
    if x > 709.7 {
        return f64::INFINITY;
    }
    let k = (x * std::f64::consts::LOG2_E + 0.5).floor();
    let r = (x - k * LN2_HI64) - k * LN2_LO64;
    let mut p = 1.0 / 6_227_020_800.0; // 1/13!
    let mut f = 6_227_020_800.0;
    for n in (1..13).rev() {
        f /= (n + 1) as f64;
        p = 1.0 / f + r * p;
    }
    p = 1.0 + r * p;
    let k = k as i64;
    let h = k >> 1;
    let pow2 = |k: i64| f64::from_bits(((k + 1023) as u64) << 52);
    p * pow2(h) * pow2(k - h)
}

/// natural log in f64, the atanh series to s^21
pub fn ln64(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    let (mut x, mut e) = (x, 0i64);
    if x.to_bits() < 0x0010_0000_0000_0000 {
        x *= 4_503_599_627_370_496.0; // 2^52
        e = -52;
    }
    let bits = x.to_bits();
    e += (bits >> 52) as i64 - 1023;
    let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if m > std::f64::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut p = 2.0 / 21.0;
    for n in (0..10).rev() {
        p = 2.0 / (2 * n + 1) as f64 + s2 * p;
    }
    let e = e as f64;
    e * LN2_HI64 + (s * p + e * LN2_LO64)
}

// π/2 in three parts (fdlibm): `k * PIO2_1` is exact for |k| < 2^20
const PIO2_1: f64 = f64::from_bits(0x3ff9_21fb_5440_0000);
const PIO2_2: f64 = f64::from_bits(0x3dd0_b461_1a60_0000);
const PIO2_3: f64 = f64::from_bits(0x3ba3_198a_2e00_0000);

/// sine and cosine of r in [−π/4, π/4]: Taylor to r^17 and r^18
fn sincos_reduced(r: f64) -> (f64, f64) {
    let r2 = r * r;
    let (mut s, mut c) = (0.0, 0.0);
    // coefficients from the top: (−1)^n / (2n+1)! and (−1)^n / (2n)!
    let mut fs = [0f64; 9];
    let mut fc = [0f64; 10];
    let mut f = 1.0; // (2n)!, then (2n+1)!
    for n in 0..10 {
        let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
        fc[n] = sign / f;
        f *= (2 * n + 1) as f64;
        if n < 9 {
            fs[n] = sign / f;
        }
        f *= (2 * n + 2) as f64;
    }
    for &k in fs.iter().rev() {
        s = k + r2 * s;
    }
    for &k in fc.iter().rev() {
        c = k + r2 * c;
    }
    (r * s, c)
}

/// sine and cosine in f64 for |x| up to about 10^6: x = k·π/2 + r, then the quadrant
pub fn sincos64(x: f64) -> (f64, f64) {
    let k = (x * std::f64::consts::FRAC_2_PI + 0.5).floor();
    let r = ((x - k * PIO2_1) - k * PIO2_2) - k * PIO2_3;
    let (s, c) = sincos_reduced(r);
    match (k as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

pub fn cos64(x: f64) -> f64 {
    sincos64(x).1
}

// ---------------------------------------------------------------- random numbers

/// SplitMix64's finaliser: a bijective scramble of 64 bits
pub fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// a key from several parts, order sensitive: `h = mix(h ^ part)` from h = 0
pub fn key(parts: &[u64]) -> u64 {
    parts.iter().fold(0x6a09_e667_f3bc_c908, |h, &p| mix(h ^ p))
}

/// a uniform f32 in [0, 1) from the top 24 bits of a word (exact)
pub fn unit(bits: u64) -> f32 {
    (bits >> 40) as f32 * (1.0 / 16_777_216.0)
}

/// xoshiro256**, seeded from a key through SplitMix64
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut z = seed;
        let mut s = [0u64; 4];
        for x in &mut s {
            z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
            *x = mix(z.wrapping_sub(0x9e37_79b9_7f4a_7c15));
        }
        Rng { s }
    }

    pub fn next_u64(&mut self) -> u64 {
        let out = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        out
    }

    /// uniform in [0, 1)
    pub fn unit(&mut self) -> f32 {
        unit(self.next_u64())
    }

    /// uniform in [−a, a)
    pub fn symmetric(&mut self, a: f32) -> f32 {
        (self.unit() * 2.0 - 1.0) * a
    }

    /// uniform in 0..n (Lemire's multiply, high half)
    pub fn below(&mut self, n: usize) -> usize {
        (((self.next_u64() >> 32) * n as u64) >> 32) as usize
    }

    /// Fisher–Yates from the back
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

/// FNV-1a over bytes, with a seed folded in first
pub fn fnv(seed: u64, data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325 ^ mix(seed);
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ulps(a: f32, b: f32) -> u32 {
        (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs() as u32
    }

    #[test]
    fn transcendentals_track_std() {
        let mut x = -20.0f32;
        while x < 20.0 {
            assert!(ulps(exp(x), x.exp()) <= 3, "exp {x}: {} {}", exp(x), x.exp());
            let t = tanh(x);
            assert!(
                (t - x.tanh()).abs() <= 2e-7 * x.tanh().abs().max(1e-30) + 1e-7,
                "tanh {x}"
            );
            assert!((sigmoid(x) - 1.0 / (1.0 + (-x).exp())).abs() < 1e-6);
            x += 0.0137;
        }
        for &x in &[1e-30f32, 1e-6, 0.3, 0.7, 1.0, 1.5, 2.0, 10.0, 1e5, 3e38, 1e-40] {
            assert!(ulps(ln(x), x.ln()) <= 2, "ln {x}: {} {}", ln(x), x.ln());
            let y = x as f64;
            assert!((ln64(y) - y.ln()).abs() <= 4.0 * f64::EPSILON * y.ln().abs().max(1.0));
        }
        assert!((expm1(1e-5) - 1e-5f32.exp_m1()).abs() < 1e-12);
        assert_eq!(tanh(0.0), 0.0);
        for i in -500..500 {
            let x = i as f64 * 0.731;
            let (s, c) = sincos64(x);
            assert!((s - x.sin()).abs() < 1e-14 && (c - x.cos()).abs() < 1e-14, "sincos {x}");
            let e = i as f64 * 0.05;
            assert!((exp64(e) - e.exp()).abs() <= 4.0 * f64::EPSILON * e.exp());
        }
    }

    #[test]
    fn rng_is_stable() {
        let mut r = Rng::new(7);
        let a: Vec<u64> = (0..3).map(|_| r.next_u64()).collect();
        let mut r = Rng::new(7);
        assert_eq!(a, (0..3).map(|_| r.next_u64()).collect::<Vec<_>>());
        assert_ne!(key(&[1, 2]), key(&[2, 1]));
        assert!((0..1000).all(|_| r.below(10) < 10));
    }
}
