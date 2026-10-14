//! The measures an entry is filed under: ordinal and reversed sums (from `simd`), the
//! user's `alphavert` atbash measure, and the letter frequency key.
//!
//! **Atbash, as weum's `alphavert` defines it** (the JS this ports):
//!
//! ```js
//! var alphavert = Object.assign(text => {
//!   const idx = ch => { if (ch === " ") return 27; const c = ch.toUpperCase().charCodeAt(0); return (c >= 65 && c <= 90) ? c - 64 : -1 }, out = []
//!   for (const ch of text) { const i = idx(ch); if (i < 0) continue; out.push([i, 27 - i]) }
//!   return out
//! },{ sum(pairs) { let t = 0, b = 0, h = [], i = [], j = []
//!   for (const [f,l] of pairs) { t += f; b += l; h.push(f); i.push(f-l); j.push(l) }
//!   return [`${b}+${t}=${b+t}`, h, i, j, ((b-pairs.length)*(t-pairs.length)), b-t] } })
//! ```
//!
//! Each code point is upper-cased on its own and its *first* resulting character is
//! looked at (so `ß` → `SS` counts as S = 19, `ﬁ` as F, `ı` as I); `A`–`Z` are 1–26, a
//! plain space (only U+0020) is 27, everything else is skipped. Each kept character is the
//! pair `[f, l] = [i, 27 - i]` (a space is `[27, 0]`).
//!
//! Note that `b + t = 27·n` always, `b` equals the reversed sum (spaces add 0), and `t` is the
//! ordinal sum plus 27 per space; the numbers that are new are the product
//! `(b − n)(t − n)` and the difference `b − t`. The **atbash bucket** a text is filed
//! under is the product.

use crate::simd;

/// the pairs `[i, 27 - i]` of the characters `alphavert` keeps
pub fn alphavert(text: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for ch in text.chars() {
        let i = if ch == ' ' {
            27
        } else {
            match ch.to_uppercase().next() {
                Some(c @ 'A'..='Z') => c as u32 - 64,
                _ => continue,
            }
        };
        out.push((i, 27 - i));
    }
    out
}

/// `alphavert.sum(pairs)`, field for field
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Atbash {
    /// the sum of the `l` values (`27 - i`)
    pub b: i64,
    /// the sum of the `f` values (`i`)
    pub t: i64,
    /// how many characters were kept
    pub n: i64,
    /// the `f` values
    pub h: Vec<i64>,
    /// `f - l` per character
    pub i: Vec<i64>,
    /// the `l` values
    pub j: Vec<i64>,
}

impl Atbash {
    pub fn of(text: &str) -> Atbash {
        Atbash::from_pairs(&alphavert(text))
    }

    pub fn from_pairs(pairs: &[(u32, u32)]) -> Atbash {
        let mut a = Atbash {
            n: pairs.len() as i64,
            ..Default::default()
        };
        for &(f, l) in pairs {
            let (f, l) = (f as i64, l as i64);
            a.t += f;
            a.b += l;
            a.h.push(f);
            a.i.push(f - l);
            a.j.push(l);
        }
        a
    }

    /// `${b}+${t}=${b+t}`
    pub fn label(&self) -> String {
        format!("{}+{}={}", self.b, self.t, self.b + self.t)
    }

    /// `(b - n) * (t - n)`: the atbash bucket
    pub fn product(&self) -> i64 {
        (self.b - self.n) * (self.t - self.n)
    }

    /// `b - t`
    pub fn difference(&self) -> i64 {
        self.b - self.t
    }

    /// the same six values `alphavert.sum` returns, as JSON
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!([self.label(), self.h, self.i, self.j, self.product(), self.difference()])
    }
}

/// which number a text is filed under
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mode {
    /// a = 1 .. z = 26, other characters 0 (`simd::sums`)
    Ordinal = 0,
    /// a = 26 .. z = 1 (`simd::sums`)
    Reversed = 1,
    /// `alphavert`'s product `(b - n)(t - n)`
    Atbash = 2,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Ordinal, Mode::Reversed, Mode::Atbash];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Ordinal => "ordinal",
            Mode::Reversed => "reversed",
            Mode::Atbash => "atbash",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.name() == s.trim())
    }

    pub fn from_u8(b: u8) -> Option<Mode> {
        Mode::ALL.get(b as usize).copied()
    }
}

/// everything a text is filed under
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measure {
    pub ordinal: i64,
    pub reversed: i64,
    pub atbash: i64,
    pub frequency: String,
}

impl Measure {
    pub fn of(text: &str) -> Measure {
        let s = simd::sums(text.as_bytes());
        Measure {
            ordinal: s.ordinal as i64,
            reversed: s.reversed as i64,
            atbash: Atbash::of(text).product(),
            frequency: simd::frequency_key(text.as_bytes()),
        }
    }

    pub fn get(&self, mode: Mode) -> i64 {
        match mode {
            Mode::Ordinal => self.ordinal,
            Mode::Reversed => self.reversed,
            Mode::Atbash => self.atbash,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worked_examples() {
        let a = Atbash::of("a");
        assert_eq!((a.b, a.t, a.n), (26, 1, 1));
        assert_eq!(a.label(), "26+1=27");
        assert_eq!((a.h.clone(), a.i.clone(), a.j.clone()), (vec![1], vec![-25], vec![26]));
        assert_eq!((a.product(), a.difference()), (0, 25));

        // h e l l o: f = 8 5 12 12 15, l = 19 22 15 15 12
        let h = Atbash::of("hello");
        assert_eq!(h.label(), "83+52=135");
        assert_eq!(h.h, vec![8, 5, 12, 12, 15]);
        assert_eq!(h.i, vec![-11, -17, -3, -3, 3]);
        assert_eq!(h.j, vec![19, 22, 15, 15, 12]);
        assert_eq!(h.product(), (83 - 5) * (52 - 5));
        assert_eq!(h.difference(), 31);

        // a space is [27, 0]; case doesn't matter; punctuation and digits are skipped
        let s = Atbash::of("Hi there!");
        assert_eq!(s.h, vec![8, 9, 27, 20, 8, 5, 18, 5]);
        assert_eq!(s.label(), "116+100=216");
        assert_eq!(s.product(), 108 * 92);
        assert_eq!(s.difference(), 16);
        assert_eq!(
            s.to_json(),
            serde_json::json!([
                "116+100=216",
                [8, 9, 27, 20, 8, 5, 18, 5],
                [-11, -9, 27, 13, -11, -17, 9, -17],
                [19, 18, 0, 7, 19, 22, 9, 22],
                9936,
                16
            ])
        );

        // a lone space: (0 - 1) * (27 - 1)
        assert_eq!(Atbash::of(" ").product(), -26);
        assert_eq!(Atbash::of("").label(), "0+0=0");
        // only U+0020 is a space; tabs and newlines are skipped
        assert_eq!(Atbash::of("a\tb\nc").n, 3);
        // the first character of the upper-cased code point: ß → SS → S, é → É (skipped)
        assert_eq!(alphavert("ß"), vec![(19, 8)]);
        assert_eq!(alphavert("é"), vec![]);
        assert_eq!(alphavert("ı"), vec![(9, 18)]);
        assert_eq!(alphavert("ﬁ"), vec![(6, 21)]);
        assert_eq!(alphavert("1-2"), vec![]);
    }

    #[test]
    fn differs_from_reversed() {
        // simd's atbash for Latin text is the reversed sum; this one isn't
        let m = Measure::of("hello world");
        let s = simd::sums(b"hello world");
        assert_eq!(s.atbash, s.reversed);
        assert_ne!(m.atbash, m.reversed);
        // b is the reversed sum, t the ordinal sum plus 27 per space
        let a = Atbash::of("hello world");
        assert_eq!(a.b, s.reversed as i64);
        assert_eq!(a.t, s.ordinal as i64 + 27);
        assert_eq!(a.b + a.t, 27 * a.n);
        assert_eq!(Mode::parse("atbash"), Some(Mode::Atbash));
        assert_eq!(Mode::from_u8(1), Some(Mode::Reversed));
    }
}
