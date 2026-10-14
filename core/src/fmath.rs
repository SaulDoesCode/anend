//! The floating point the spread needs, the same everywhere. `f64::sin`, `cos` and
//! `hypot` go to the platform's libm (glibc picks a different sin/cos on CPUs with and
//! without FMA, Windows has its own), and the web app's offline engine must lay texts out
//! exactly as the hub does. So: hypot is glibc's own kernel (e_hypot.c), sin and cos are
//! correctly rounded in double-double. `hub/web/local/fmath.js` is the same code.

// ---------------------------------------------------------------- hypot (glibc 2.35+)

const SCALE: f64 = 2.409919865102884e-181; // 2^-600
const LARGE_VAL: f64 = 6.703903964971299e153; // 2^511
const TINY_VAL: f64 = 1.4916681462400413e-154; // 2^-511
const EPS: f64 = 5.551115123125783e-17; // 2^-54

fn kernel(ax: f64, ay: f64) -> f64 {
    let mut h = (ax * ax + ay * ay).sqrt();
    let (t1, t2);
    if h <= 2.0 * ay {
        let delta = h - ay;
        t1 = ax * (2.0 * delta - ax);
        t2 = (delta - 2.0 * (ax - ay)) * delta;
    } else {
        let delta = h - ax;
        t1 = 2.0 * delta * (ax - 2.0 * ay);
        t2 = (4.0 * delta - ay) * ay + delta * delta;
    }
    h -= (t1 + t2) / (2.0 * h);
    h
}

pub fn hypot(x: f64, y: f64) -> f64 {
    if !x.is_finite() || !y.is_finite() {
        return if x.is_infinite() || y.is_infinite() {
            f64::INFINITY
        } else {
            f64::NAN
        };
    }
    let (x, y) = (x.abs(), y.abs());
    let (ax, ay) = if x < y { (y, x) } else { (x, y) };
    if ax > LARGE_VAL {
        if ay <= ax * EPS {
            return ax + ay;
        }
        return kernel(ax * SCALE, ay * SCALE) / SCALE;
    }
    if ay < TINY_VAL {
        if ax >= ay / EPS {
            return ax + ay;
        }
        return kernel(ax / SCALE, ay / SCALE) * SCALE;
    }
    if ay <= ax * EPS {
        return ax + ay;
    }
    kernel(ax, ay)
}

// ---------------------------------------------------------------- double-double

type Dd = (f64, f64);
const SPLIT: f64 = 134217729.0; // 2^27 + 1

fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    (s, a - (s - bb) + (b - bb))
}

fn quick_two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    (s, b - (s - a))
}

fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    let t = SPLIT * a;
    let ah = t - (t - a);
    let al = a - ah;
    let t = SPLIT * b;
    let bh = t - (t - b);
    let bl = b - bh;
    (p, ah * bh - p + ah * bl + al * bh + al * bl)
}

fn add(a: Dd, b: Dd) -> Dd {
    let (s, e) = two_sum(a.0, b.0);
    let (t, f) = two_sum(a.1, b.1);
    let (s, e) = quick_two_sum(s, e + t);
    quick_two_sum(s, e + f)
}

fn mul(a: Dd, b: Dd) -> Dd {
    let (p, e) = two_prod(a.0, b.0);
    quick_two_sum(p, e + (a.0 * b.1 + a.1 * b.0))
}

fn mul_d(a: Dd, d: f64) -> Dd {
    let (p, e) = two_prod(a.0, d);
    quick_two_sum(p, e + a.1 * d)
}

fn div_d(a: Dd, d: f64) -> Dd {
    let q1 = a.0 / d;
    let r = add(a, mul_d((q1, 0.0), -d));
    quick_two_sum(q1, r.0 / d)
}

/// 1/n! for n up to 30
fn inv_fact() -> &'static [Dd; 31] {
    static T: std::sync::OnceLock<[Dd; 31]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [(1.0, 0.0); 31];
        for i in 1..=30 {
            t[i] = div_d(t[i - 1], i as f64);
        }
        t
    })
}

const PIO2: [f64; 3] = [
    std::f64::consts::FRAC_PI_2,
    6.123233995736766e-17,
    -1.4973849048591698e-33,
];

/// x − n·π/2 for |x| up to a few thousand (the spread's angles are far smaller)
fn reduce(x: f64) -> (i64, Dd) {
    // JavaScript's Math.round: halves go up
    let n = (x / PIO2[0] + 0.5).floor();
    let mut r = (x, 0.0);
    for p in PIO2 {
        r = add(r, mul_d((p, 0.0), -n));
    }
    ((n as i64).rem_euclid(4), r)
}

fn series(r: Dd, odd: bool) -> f64 {
    let f = inv_fact();
    let r2 = mul(r, r);
    let mut t = if odd { r } else { (1.0, 0.0) };
    let mut s = t;
    let mut i = if odd { 3 } else { 2 };
    let mut minus = true;
    while i <= 30 {
        t = mul(t, r2);
        let term = mul(t, f[i]);
        s = add(s, if minus { (-term.0, -term.1) } else { term });
        if term.0.abs() < 1e-36 {
            break;
        }
        i += 2;
        minus = !minus;
    }
    s.0 + s.1
}

pub fn sin(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x.abs() < 1.4901161193847656e-8 || x.abs() > 1048576.0 {
        // tiny: sin x = x; huge: never in a spread
        return if x.abs() < 1.0 { x } else { x.sin() };
    }
    let (q, r) = reduce(x);
    let v = series(r, q % 2 == 0);
    if q >= 2 { -v } else { v }
}

pub fn cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x.abs() < 7.450580596923828e-9 {
        return 1.0;
    }
    if x.abs() > 1048576.0 {
        return x.cos();
    }
    let (q, r) = reduce(x);
    let v = series(r, q % 2 == 1);
    if q == 1 || q == 2 { -v } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_to_libm() {
        // correctly rounded, so at most an ulp from any decent libm, and exact on easy points
        let mut x = -50.0f64;
        while x < 50.0 {
            for (a, b) in [(sin(x), x.sin()), (cos(x), x.cos())] {
                let ulps = (a.to_bits() as i64 - b.to_bits() as i64).abs();
                assert!(ulps <= 1 || (a - b).abs() < 1e-300, "{x}: {a} vs {b}");
            }
            assert_eq!(hypot(x, 3.25), x.hypot(3.25));
            x += 0.0137;
        }
        assert_eq!(sin(0.0), 0.0);
        assert_eq!(cos(0.0), 1.0);
        assert_eq!(hypot(3.0, 4.0), 5.0);
    }
}
