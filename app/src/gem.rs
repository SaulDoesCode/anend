//! The numbers the views show, computed in the app (no round trip): weum's sums, its
//! `gsum(text, 1)` line, `pfR`, `seedCLR`, the underline groups, the triangle and the byte
//! sum. weum's api.js and memory.js (docs/weum-reference) are the reference.

use anend_core::{memory, spread, weum};

/// the gsearch modes (the memory view's buckets)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Ordinal,
    Reversed,
    Frequency,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Ordinal, Mode::Reversed, Mode::Frequency];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Ordinal => "ordinal",
            Mode::Reversed => "reversed",
            Mode::Frequency => "frequency",
        }
    }

    pub fn letter(self) -> &'static str {
        match self {
            Mode::Ordinal => "O",
            Mode::Reversed => "R",
            Mode::Frequency => "F",
        }
    }

    /// the sum whose shared values colour the underlines in this mode: ordinal by
    /// reversed groups, reversed by ordinal groups (weum), frequency by ordinal ones
    pub fn grouping(self) -> Mode {
        match self {
            Mode::Ordinal => Mode::Reversed,
            Mode::Reversed | Mode::Frequency => Mode::Ordinal,
        }
    }
}

/// a text's numbers
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Measure {
    pub ordinal: u64,
    pub reversed: u64,
    /// weum's letter table (`e1i1l1n1s1t1`)
    pub frequency: String,
    /// every byte counted as 1 to 256 (a separate measure, not a mode)
    pub bytes: u64,
}

impl Measure {
    pub fn of(text: &str) -> Measure {
        let m = memory::measure(text);
        let (s, r) = spread::sums(text);
        Measure {
            ordinal: s.max(0) as u64,
            reversed: r.max(0) as u64,
            frequency: m.frequency,
            bytes: byte_sum(text),
        }
    }

    /// weum's ⌊√(ordinal · reversed)⌋, the triangle's third side and Ctrl+right click's
    /// gsearch
    pub fn triangle(&self) -> u64 {
        ((self.ordinal as f64) * (self.reversed as f64)).sqrt().floor() as u64
    }
}

/// the sum of a text's bytes, each counted as its value plus one (1 to 256)
pub fn byte_sum(text: &str) -> u64 {
    text.bytes().map(|b| b as u64 + 1).sum()
}

/// weum's ordinal and reversed sums (`gsum(t, 3)`)
pub fn sums(text: &str) -> (u64, u64) {
    let (s, r) = spread::sums(text);
    (s.max(0) as u64, r.max(0) as u64)
}

/// the sum a text gsearches by in a mode
pub fn sum_in(text: &str, mode: Mode) -> u64 {
    match mode {
        Mode::Ordinal => sums(text).0,
        Mode::Reversed => sums(text).1,
        Mode::Frequency => sums(text).0,
    }
}

/// weum's letter table as the letters themselves: e1i1l1n1s1t1 → eilnst
pub fn letters(freq: &str) -> String {
    let mut out = String::new();
    let mut chars = freq.chars().peekable();
    while let Some(c) = chars.next() {
        let mut n = String::new();
        while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
            n.push(*d);
            chars.next();
        }
        let k: usize = n.parse().unwrap_or(1);
        for _ in 0..k.min(64) {
            out.push(c);
        }
    }
    out
}

fn vowel(c: char) -> u64 {
    match c {
        'a' => 1,
        'e' => 2,
        'o' => 3,
        'u' => 4,
        'i' => 5,
        'y' => 6,
        _ => 0,
    }
}

fn consonant(c: char) -> u64 {
    // weum's table, t and v both 16
    match c {
        'b' => 1,
        'c' => 2,
        'd' => 3,
        'f' => 4,
        'g' => 5,
        'h' => 6,
        'j' => 7,
        'k' => 8,
        'l' => 9,
        'm' => 10,
        'n' => 11,
        'p' => 12,
        'q' => 13,
        'r' => 14,
        's' => 15,
        't' | 'v' => 16,
        'w' => 17,
        'x' => 18,
        'z' => 19,
        _ => 0,
    }
}

/// weum's `gsum(text, 1)`, letter for letter:
/// `(s + r = t; * p; *sqrt(q); diff d, a b c) v vowels c consonants w words`
pub fn gsum_line(text: &str) -> String {
    let (sum, r) = sums(text);
    let (sum, r) = (sum as i64, r as i64);
    let (mut v, mut c) = (0, 0);
    for ch in text.chars().flat_map(char::to_lowercase) {
        v += vowel(ch);
        c += consonant(ch);
    }
    let words = text.split_whitespace().count().max(1);
    let a = (sum + r) + (sum - r).abs();
    let b = (sum + r) + (sum - r);
    format!(
        "({} + {} = {}; * {}; *sqrt({:.3}); diff {}, {} {} {}) v {} c {} w {}",
        sum,
        r,
        sum + r,
        sum * r,
        ((sum * r) as f64).sqrt(),
        sum - r,
        a,
        b,
        a - b,
        v,
        c,
        words
    )
}

/// weum's `pfR(s)` as pieces; numbers are clickable
pub fn pf_line(s: u64) -> Vec<weum::Seg> {
    if s < 2 { Vec::new() } else { weum::pf_reduction(s) }
}

/// weum's `seedCLR(m)`: a hue from the string of `m` (JavaScript's 32 bit string hash)
pub fn seed_hue(key: &str) -> u32 {
    let mut hash: i32 = 0;
    for c in key.encode_utf16() {
        hash = (c as i32).wrapping_add(hash.wrapping_shl(5).wrapping_sub(hash));
    }
    (hash % 360).unsigned_abs()
}

/// `hsl(hue, 70%, 45%)`, weum's underline colour for a group key
pub fn seed_clr(key: &str) -> String {
    format!("hsl({}, 70%, 45%)", seed_hue(key))
}

/// the underline colour weum's `baroeskwidjel` gives a text in a mode: the colour of its
/// grouping sum, when that sum's string is two or more characters long (weum checks the
/// key's length, so a lone text with a two digit sum is underlined too)
pub fn underline(text: &str, mode: Mode) -> Option<String> {
    let key = sum_in(text, mode.grouping()).to_string();
    (key.len() >= 2).then(|| seed_clr(&key))
}

/// the triangle with sides s, r and t = ⌊√(s·r)⌋ as an svg polygon fitted to a box:
/// `(width, height, points, real)`. When the three can't close, the flat shape they come
/// closest to (`real` false), drawn dashed.
pub fn triangle_points(s: u64, r: u64, height: f64, max_width: f64) -> Option<(f64, f64, String, bool)> {
    let t = ((s as f64) * (r as f64)).sqrt().floor();
    let (s, r) = (s as f64, r as f64);
    if !(s > 0.0 && r > 0.0 && t > 0.0) {
        return None;
    }
    let real = s < r + t && r < s + t && t < s + r;
    let x = ((s * s + t * t - r * r) / (2.0 * s)).clamp(-t, t);
    let y = if real { (t * t - x * x).max(0.0).sqrt() } else { 0.0 };
    let min_x = x.min(0.0);
    let max_x = x.max(s);
    let w = max_x - min_x;
    let pad = 2.0;
    let k = ((height - pad * 2.0) / y.max(1e-9)).min((max_width - pad * 2.0) / w);
    let ww = w * k + pad * 2.0;
    let hh = if real { y * k + pad * 2.0 } else { height };
    let base = if real { hh - pad } else { hh / 2.0 };
    let p = |px: f64, py: f64| format!("{:.1},{:.1}", (px - min_x) * k + pad, base - py * k);
    Some((ww, hh, format!("{} {} {}", p(0.0, 0.0), p(s, 0.0), p(x, y)), real))
}

/// the triangle's angles, or why it lies flat, for its tooltip
pub fn triangle_title(s: u64, r: u64) -> String {
    let t = ((s as f64) * (r as f64)).sqrt().floor();
    let (sf, rf) = (s as f64, r as f64);
    let real = sf < rf + t && rf < sf + t && t < sf + rf;
    let deg = |v: f64| v.clamp(-1.0, 1.0).acos().to_degrees();
    if real {
        format!(
            "s {s}, r {r}, t = ⌊√(s·r)⌋ {t}\nangles {:.1}° (facing r), {:.1}° (facing t), {:.1}° (facing s)",
            deg((sf * sf + t * t - rf * rf) / (2.0 * sf * t)),
            deg((sf * sf + rf * rf - t * t) / (2.0 * sf * rf)),
            deg((rf * rf + t * t - sf * sf) / (2.0 * rf * t)),
        )
    } else {
        let m = sf.max(rf).max(t);
        format!(
            "s {s}, r {r}, t = ⌊√(s·r)⌋ {t}\nnot a triangle: {m} is at least the other two sides together ({}), so it lies flat",
            sf + rf + t - m
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weum_numbers() {
        assert_eq!(sums("listen"), (79, 83));
        assert_eq!(Measure::of("listen").triangle(), 80);
        // the JavaScript string hash, as weum's seedCLR
        assert_eq!(
            seed_hue("83"),
            ("83"
                .encode_utf16()
                .fold(0i32, |h, c| (c as i32).wrapping_add((h << 5) - h))
                % 360)
                .unsigned_abs()
        );
        assert_eq!(seed_clr("345"), "hsl(276, 70%, 45%)");
        assert_eq!(letters("e1i1l1n1s1t1"), "eilnst");
        assert_eq!(byte_sum("ab"), 97 + 1 + 98 + 1);
    }

    #[test]
    fn gsum_like_weum() {
        // weum: gsum('listen', 1)
        assert_eq!(
            gsum_line("listen"),
            "(79 + 83 = 162; * 6557; *sqrt(80.975); diff -4, 166 158 8) v 7 c 51 w 1"
        );
    }

    #[test]
    fn underline_groups() {
        // ordinal mode groups by reversed: listen's reversed is 83
        assert_eq!(underline("listen", Mode::Ordinal), Some(seed_clr("83")));
        assert_eq!(underline("listen", Mode::Reversed), Some(seed_clr("79")));
        // a one digit grouping sum isn't underlined ("a": reversed 26, ordinal 1)
        assert_eq!(underline("a", Mode::Reversed), None);
        assert!(underline("a", Mode::Ordinal).is_some());
    }

    #[test]
    fn triangles() {
        let (_, _, pts, real) = triangle_points(87, 48, 22.0, 64.0).unwrap();
        assert!(real && pts.split(' ').count() == 3);
        assert!(!triangle_points(1, 26, 22.0, 64.0).unwrap().3);
        assert!(triangle_points(0, 0, 22.0, 64.0).is_none());
    }
}
