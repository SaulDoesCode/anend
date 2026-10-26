//! Placements: the payload of a `PLACE` edge (from a map entry to the entry placed in it),
//! a transform bit-packed into a small fixed value. The same entry can stand in several
//! places and several maps; identical placements collapse into one edge.
//!
//! **The packed value** (`PACKED` = 34 bytes, little endian, version 2):
//!
//! | bytes | field |
//! |---|---|
//! | 0 | version, 2 (an unknown version doesn't unpack) |
//! | 1 | kind: 0 text, 1 cube, 2 long block, 3 brick, 4 model (glTF; reserved), 5 sphere |
//! | 2 | flags: 1 pinned, 2 hidden, 4 spread (laid out by its sums when not pinned); the rest kept as they are |
//! | 3 | reserved, 0 |
//! | 4–15 | position, 3 × f32 |
//! | 16–19 | rotation, smallest three: bits 30–31 the index of the largest component of the unit quaternion (x y z w), made positive and left out; the other three in order, 10 bits each (bits 20–29, 10–19, 0–9), each `round(v·√2·511) + 511` |
//! | 20–25 | scale, 3 × f16 (a text uses x as its em size) |
//! | 26–29 | colour, RGBA8 |
//! | 30–33 | asset, u32: 0 none (reserved for models) |
//!
//! The map isn't in the value: it's the edge's `from`.
//!
//! **Packing is canonical**, so the same placement always gives the same bytes and
//! `pack(unpack(b)) == b` for anything `pack` made: −0 is +0, a NaN position is 0 (a NaN
//! scale is 1), infinities clamp to ±f32::MAX, scales clamp to ±65504 and round to nearest
//! even, and a rotation is normalised first (a zero or broken one is the identity).
//!
//! The fix over the first version (docs/wip): smallest-three quantisation alone isn't
//! idempotent. When two components are nearly equal, the decoded largest can come out
//! smaller than another decoded component, so re-packing picks another index (or rounds a
//! component one step off after normalising). `pack_rotation` therefore iterates
//! `quantise ∘ dequantise` until it reaches a value that maps to itself, and if the
//! sequence ends in a cycle instead, takes the smallest value of the cycle. Both ends of a
//! round trip land on the same cycle, so they agree, and the chosen value maps to itself.

/// the layout version
pub const VERSION: u8 = 2;
pub const PACKED: usize = 34;

pub const PINNED: u8 = 1;
pub const HIDDEN: u8 = 2;
pub const SPREAD: u8 = 4;

pub const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Kind {
    #[default]
    Text = 0,
    Cube = 1,
    LongBlock = 2,
    Brick = 3,
    /// a glTF model (reserved)
    Model = 4,
    /// a sphere (an ellipsoid when its sides differ)
    Sphere = 5,
}

impl Kind {
    pub fn from_u8(b: u8) -> Option<Kind> {
        Some(match b {
            0 => Kind::Text,
            1 => Kind::Cube,
            2 => Kind::LongBlock,
            3 => Kind::Brick,
            4 => Kind::Model,
            5 => Kind::Sphere,
            _ => return None,
        })
    }
}

/// one placement of an entry in a map
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub kind: Kind,
    pub flags: u8,
    pub pos: [f32; 3],
    /// a quaternion, x y z w
    pub rot: [f32; 4],
    pub scale: [f32; 3],
    pub color: [u8; 4],
    pub asset: u32,
}

impl Place {
    /// a text of em size `size`, facing the camera
    pub fn text(pos: [f32; 3], size: f32, color: [u8; 4], flags: u8) -> Place {
        Place {
            kind: Kind::Text,
            flags,
            pos,
            rot: IDENTITY,
            scale: [size; 3],
            color,
            asset: 0,
        }
    }

    pub fn pinned(&self) -> bool {
        self.flags & PINNED != 0
    }

    pub fn hidden(&self) -> bool {
        self.flags & HIDDEN != 0
    }

    pub fn spread(&self) -> bool {
        self.flags & SPREAD != 0
    }

    pub fn pack(&self) -> [u8; PACKED] {
        let mut b = [0u8; PACKED];
        b[0] = VERSION;
        b[1] = self.kind as u8;
        b[2] = self.flags;
        for (i, v) in self.pos.iter().enumerate() {
            b[4 + i * 4..8 + i * 4].copy_from_slice(&canon_f32(*v).to_le_bytes());
        }
        b[16..20].copy_from_slice(&pack_rotation(self.rot).to_le_bytes());
        for (i, v) in self.scale.iter().enumerate() {
            // a scale that rounds to -0 is +0, like one that was -0 to begin with
            let h = match f32_to_f16(canon_scale(*v)) {
                0x8000 => 0,
                h => h,
            };
            b[20 + i * 2..22 + i * 2].copy_from_slice(&h.to_le_bytes());
        }
        b[26..30].copy_from_slice(&self.color);
        b[30..34].copy_from_slice(&self.asset.to_le_bytes());
        b
    }

    /// `None` for a value of another size, version or kind
    pub fn unpack(b: &[u8]) -> Option<Place> {
        if b.len() != PACKED || b[0] != VERSION {
            return None;
        }
        let f32_at = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let f16_at = |i: usize| f16_to_f32(u16::from_le_bytes([b[i], b[i + 1]]));
        Some(Place {
            kind: Kind::from_u8(b[1])?,
            flags: b[2],
            pos: [f32_at(4), f32_at(8), f32_at(12)],
            rot: unpack_rotation(u32::from_le_bytes([b[16], b[17], b[18], b[19]])),
            scale: [f16_at(20), f16_at(22), f16_at(24)],
            color: [b[26], b[27], b[28], b[29]],
            asset: u32::from_le_bytes([b[30], b[31], b[32], b[33]]),
        })
    }
}

fn canon_f32(v: f32) -> f32 {
    if v.is_nan() || v == 0.0 {
        0.0
    } else {
        v.clamp(f32::MIN, f32::MAX)
    }
}

fn canon_scale(v: f32) -> f32 {
    if v.is_nan() {
        1.0
    } else if v == 0.0 {
        0.0
    } else {
        v.clamp(-65504.0, 65504.0)
    }
}

// ---------------------------------------------------------------- f16

/// IEEE half precision, rounded to nearest even
pub fn f32_to_f16(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let man = x & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        let shift = (14 - e) as u32;
        if shift > 24 {
            return sign;
        }
        let full = man | 0x80_0000;
        let half = 1u32 << (shift - 1);
        let rest = full & ((1u32 << shift) - 1);
        let mut m = full >> shift;
        if rest > half || (rest == half && m & 1 == 1) {
            m += 1;
        }
        return sign | m as u16;
    }
    let mut h = ((e as u32) << 10) | (man >> 13);
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && h & 1 == 1) {
        h += 1;
    }
    sign | h as u16
}

pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let e = ((h >> 10) & 0x1f) as u32;
    let m = (h & 0x3ff) as u32;
    let bits = match (e, m) {
        (0, 0) => sign,
        (0, m) => {
            let v = m as f32 * (1.0 / 16_777_216.0);
            return if sign != 0 { -v } else { v };
        }
        (0x1f, 0) => sign | 0x7f80_0000,
        (0x1f, m) => sign | 0x7fc0_0000 | (m << 13),
        (e, m) => sign | ((e + 112) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

// ---------------------------------------------------------------- rotation

const STEPS: f32 = 511.0;

/// one smallest-three quantisation, without the fixed point search
fn quantise(q: [f32; 4]) -> u32 {
    let len = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    let q = if len.is_finite() && len > 1e-6 {
        q.map(|v| v / len)
    } else {
        IDENTITY
    };
    let mut big = 0;
    for i in 1..4 {
        if q[i].abs() > q[big].abs() {
            big = i;
        }
    }
    let q = if q[big] < 0.0 { q.map(|v| -v) } else { q };
    let mut out = (big as u32) << 30;
    let mut shift = 20;
    for (i, v) in q.iter().enumerate() {
        if i == big {
            continue;
        }
        let s = (v * std::f32::consts::SQRT_2 * STEPS).round().clamp(-STEPS, STEPS) as i32 + 511;
        out |= (s as u32) << shift;
        shift -= 10;
    }
    out
}

/// a unit quaternion (x y z w) in 32 bits, canonically: the result `r` always has
/// `pack_rotation(unpack_rotation(r)) == r`
pub fn pack_rotation(q: [f32; 4]) -> u32 {
    let mut seen: Vec<u32> = vec![quantise(q)];
    loop {
        let last = *seen.last().unwrap();
        let next = quantise(unpack_rotation(last));
        if next == last {
            return last;
        }
        if let Some(at) = seen.iter().position(|&s| s == next) {
            return *seen[at..].iter().min().unwrap();
        }
        seen.push(next);
        if seen.len() > 64 {
            // never seen in practice; still a value both ends agree on
            return *seen.iter().min().unwrap();
        }
    }
}

pub fn unpack_rotation(r: u32) -> [f32; 4] {
    let big = (r >> 30) as usize;
    let mut q = [0f32; 4];
    let mut shift = 20;
    let mut sum = 0.0;
    for (i, slot) in q.iter_mut().enumerate() {
        if i == big {
            continue;
        }
        let s = ((r >> shift) & 0x3ff).min(1022) as i32 - 511;
        *slot = s as f32 / STEPS * std::f32::consts::FRAC_1_SQRT_2;
        sum += *slot * *slot;
        shift -= 10;
    }
    q[big] = (1.0 - sum).max(0.0).sqrt();
    q
}

// ---------------------------------------------------------------- colours

/// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`
pub fn parse_hex_color(s: &str) -> Option<[u8; 4]> {
    let h = s.trim().trim_start_matches('#');
    if !h.is_ascii() {
        return None;
    }
    let nib = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = h.as_bytes();
    let mut out = [255u8; 4];
    match b.len() {
        3 | 4 => {
            for (i, c) in b.iter().enumerate() {
                out[i] = nib(*c)? * 17;
            }
        }
        6 | 8 => {
            for i in 0..b.len() / 2 {
                out[i] = nib(b[i * 2])? * 16 + nib(b[i * 2 + 1])?;
            }
        }
        _ => return None,
    }
    Some(out)
}

/// `#rrggbb`, with `aa` when it isn't opaque
pub fn hex_color(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 32) as u32
        }
        fn unit(&mut self) -> f32 {
            self.next() as f32 / u32::MAX as f32 * 2.0 - 1.0
        }
    }

    const INK: [u8; 4] = [0x3b, 0x2a, 0x1c, 255];

    fn sample() -> Place {
        Place {
            kind: Kind::Brick,
            flags: PINNED | SPREAD | 0x80,
            pos: [1.25, -3.5e7, 0.1],
            rot: [0.1, -0.7, 0.2, 0.68],
            scale: [0.5, 2.0, -1.5],
            color: [1, 2, 3, 4],
            asset: 0xdead_beef,
        }
    }

    #[test]
    fn layout_is_fixed() {
        let b = sample().pack();
        assert_eq!(b.len(), PACKED);
        assert_eq!((b[0], b[1], b[2], b[3]), (2, 3, 0x85, 0));
        assert_eq!(&b[4..8], &1.25f32.to_le_bytes());
        assert_eq!(&b[20..22], &0x3800u16.to_le_bytes()); // 0.5 in f16
        assert_eq!(&b[24..26], &0xbe00u16.to_le_bytes()); // -1.5
        assert_eq!(&b[26..30], &[1, 2, 3, 4]);
        assert_eq!(&b[30..34], &0xdead_beefu32.to_le_bytes());
        let id = pack_rotation(IDENTITY);
        assert_eq!(id, (3 << 30) | (511 << 20) | (511 << 10) | 511);
        assert_eq!(unpack_rotation(id), IDENTITY);
    }

    #[test]
    fn round_trip_and_idempotence() {
        let p = sample();
        let q = Place::unpack(&p.pack()).unwrap();
        assert_eq!(
            (q.kind, q.flags, q.pos, q.color, q.asset),
            (p.kind, p.flags, p.pos, p.color, p.asset)
        );
        assert_eq!(q.scale, [0.5, 2.0, -1.5]);
        assert_eq!(q.pack(), p.pack());
        let mut b = p.pack();
        assert!(Place::unpack(&b[..33]).is_none());
        b[0] = 1;
        assert!(Place::unpack(&b).is_none());
        let mut b = p.pack();
        b[1] = 9;
        assert!(Place::unpack(&b).is_none());

        let mut g = Lcg(7);
        for _ in 0..20_000 {
            let p = Place {
                kind: Kind::from_u8((g.next() % 5) as u8).unwrap(),
                flags: g.next() as u8,
                pos: [g.unit() * 1e4, g.unit(), g.unit() * 1e-3],
                rot: [g.unit(), g.unit(), g.unit(), g.unit()],
                scale: [g.unit() * 100.0, g.unit(), g.unit() * 1e-3],
                color: g.next().to_le_bytes(),
                asset: g.next(),
            };
            let b = p.pack();
            let q = Place::unpack(&b).unwrap();
            assert_eq!(q.pack(), b, "{p:?}");
            assert_eq!(q.pos, p.pos);
            for i in 0..3 {
                let tol = (p.scale[i].abs() * (1.0 / 2048.0)).max(1.0 / 33_554_432.0);
                assert!((q.scale[i] - p.scale[i]).abs() <= tol);
            }
            let len = p.rot.iter().map(|v| v * v).sum::<f32>().sqrt();
            if len > 1e-6 {
                let dot: f32 = p.rot.iter().zip(q.rot).map(|(a, b)| a / len * b).sum();
                let angle = 2.0 * dot.abs().min(1.0).acos();
                assert!(angle.to_degrees() < 0.3, "{angle} for {:?}", p.rot);
            }
        }
    }

    #[test]
    fn near_ties_repack_identically() {
        // the cases the first version got wrong: two or more components nearly equal
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let mut g = Lcg(11);
        let mut cases = vec![
            [0.0, 0.0, h, h],
            [h, 0.0, 0.0, h],
            [0.5, 0.5, 0.5, 0.5],
            [-0.5, 0.5, -0.5, 0.5],
        ];
        for _ in 0..50_000 {
            let e = g.unit() * 2e-3;
            let a = g.unit();
            let c = g.unit() * 0.3;
            let mut q = [c, a, a + e, -c * 0.5];
            let k = (g.next() % 4) as usize;
            q.swap(k, 3);
            cases.push(q);
            let s = 0.5 + g.unit() * 1e-3;
            cases.push([s, -s, s + e, 0.5]);
        }
        for q in cases {
            let r = pack_rotation(q);
            assert_eq!(pack_rotation(unpack_rotation(r)), r, "{q:?}");
            // and again from the decoded value, any number of times
            let again = pack_rotation(unpack_rotation(pack_rotation(unpack_rotation(r))));
            assert_eq!(again, r);
        }
        // every 32-bit pattern a corrupt value might hold re-packs to a fixed point
        for _ in 0..50_000 {
            let r = g.next();
            let once = pack_rotation(unpack_rotation(r));
            assert_eq!(pack_rotation(unpack_rotation(once)), once);
        }
    }

    #[test]
    fn canonical_values() {
        let base = Place::text([0.0; 3], 0.5, INK, 0);
        let with = |f: &dyn Fn(&mut Place)| {
            let mut p = base;
            f(&mut p);
            p.pack()
        };
        assert_eq!(with(&|p| p.pos = [-0.0, -0.0, -0.0]), base.pack());
        assert_eq!(with(&|p| p.pos[0] = f32::NAN), base.pack());
        let inf = Place::unpack(&with(&|p| p.pos = [f32::INFINITY, f32::NEG_INFINITY, 1.0])).unwrap();
        assert_eq!(inf.pos, [f32::MAX, f32::MIN, 1.0]);
        assert_eq!(with(&|p| p.scale[0] = -0.0), with(&|p| p.scale[0] = 0.0));
        assert_eq!(
            Place::unpack(&with(&|p| p.scale = [f32::NAN; 3])).unwrap().scale,
            [1.0; 3]
        );
        let r = [0.3, -0.5, 0.1, -0.8];
        assert_eq!(pack_rotation(r), pack_rotation(r.map(|v: f32| -v)));
        assert_eq!(pack_rotation(r), pack_rotation(r.map(|v: f32| v * 9.0)));
        assert_eq!(pack_rotation([0.0; 4]), pack_rotation(IDENTITY));
        assert_eq!(pack_rotation([f32::NAN, 0.0, 0.0, 1.0]), pack_rotation(IDENTITY));
        for big in 0..4 {
            let mut q = [0.1f32, -0.2, 0.15, 0.05];
            q[big] = -0.9;
            let b = pack_rotation(q);
            assert_eq!((b >> 30) as usize, big);
            assert!(unpack_rotation(b)[big] > 0.0);
        }
    }

    #[test]
    fn f16_every_value() {
        for h in 0..=u16::MAX {
            let v = f16_to_f32(h);
            if v.is_nan() {
                continue;
            }
            assert_eq!(f32_to_f16(v), h, "{h:#06x} {v}");
        }
        assert_eq!(f32_to_f16(1.0 + 1.0 / 2048.0), 0x3c00);
        assert_eq!(f32_to_f16(1.0 + 3.0 / 2048.0), 0x3c02);
    }

    #[test]
    fn colours() {
        assert_eq!(parse_hex_color("#3b2a1c"), Some(INK));
        assert_eq!(parse_hex_color("fff"), Some([255; 4]));
        assert_eq!(parse_hex_color("#10203040"), Some([0x10, 0x20, 0x30, 0x40]));
        assert_eq!(parse_hex_color("#12"), None);
        assert_eq!(hex_color([1, 2, 3, 4]), "#01020304");
    }
}
