//! From data to model input. Tokens are bytes (0..=255) plus four specials (CLS, SEP,
//! PAD, and NUM for a number whose value rides along); no vocabulary is learned or
//! stored. Any data is bytes: text, binary, pictures through `codec`, numbers as text or
//! as NUM tokens.
//!
//! Optional feature channels run next to each token. They are opt-in per model
//! (`Config::letters`, `Config::words`, `Config::side`), off for new models and on for
//! classifiers saved before they became optional: the letter's ordinal value, the sums
//! of the word the token sits in (ordinal, reversed and Atbash, each bucketed by modulo,
//! and their digit roots), and a whole-window side vector of text level measures (letter
//! frequencies, the sums, the prime factor chain, shape of the text). In a causal model
//! (one that generates) the word channels are the sums of the word so far, so no token
//! sees a byte after it.

use super::math::ln;
use crate::simd;
use crate::weum;

pub const CLS: u16 = 256;
pub const SEP: u16 = 257;
pub const PAD: u16 = 258;
/// a number: the token's embedding is its row plus value × a learned direction
pub const NUM: u16 = 259;
/// tokens without NUM (the table size of models without number inputs)
pub const VOCAB: usize = 259;

/// rows of the letter value table: 0 for "not a letter", then a = 1 .. z = 26
pub const LETTERS: usize = 27;
/// word sums land in 1 + sum mod 96; 0 is "not in a word"
pub const SUM_BUCKETS: usize = 97;
/// 1 + 10·root(ordinal) + root(reversed); 0 is "not in a word"
pub const ROOT_BUCKETS: usize = 101;
/// length of the side vector
pub const SIDE: usize = 46;

/// one window of a text, ready for the embedder
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Window {
    pub tokens: Vec<u16>,
    pub letter: Vec<u8>,
    pub ordinal: Vec<u8>,
    pub reversed: Vec<u8>,
    pub atbash: Vec<u8>,
    pub root: Vec<u8>,
    pub side: Vec<f32>,
    /// per token, the value of a NUM token (0 elsewhere); empty when there are none
    pub values: Vec<f32>,
}

impl Window {
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

/// the digit root: 0 for 0, else 1 + (n − 1) mod 9
pub fn digit_root(n: u64) -> u64 {
    if n == 0 { 0 } else { 1 + (n - 1) % 9 }
}

fn in_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b >= 0x80
}

/// `[CLS] bytes [SEP]` with every channel filled in; `bytes` should fit the window
pub fn window(bytes: &[u8]) -> Window {
    window_with(bytes, false, true)
}

/// `[CLS] bytes`, then `[SEP]` when `end`. `causal`: each token's word channels are the
/// sums of its word up to and including it (what a generator may see)
pub fn window_with(bytes: &[u8], causal: bool, end: bool) -> Window {
    let n = bytes.len() + 1 + end as usize;
    let mut w = Window {
        tokens: Vec::with_capacity(n),
        letter: vec![0; n],
        ordinal: vec![0; n],
        reversed: vec![0; n],
        atbash: vec![0; n],
        root: vec![0; n],
        side: side(bytes),
        values: Vec::new(),
    };
    w.tokens.push(CLS);
    w.tokens.extend(bytes.iter().map(|&b| b as u16));
    if end {
        w.tokens.push(SEP);
    }
    for (i, &b) in bytes.iter().enumerate() {
        let lc = b | 0x20;
        if lc.is_ascii_lowercase() {
            w.letter[i + 1] = lc - b'a' + 1;
        }
    }
    let mut i = 0;
    while i < bytes.len() {
        if !in_word(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && in_word(bytes[i]) {
            i += 1;
        }
        let bucket = |v: u64| (1 + v % 96) as u8;
        let mut fill = |t: usize, s: simd::Sums| {
            w.ordinal[t] = bucket(s.ordinal);
            w.reversed[t] = bucket(s.reversed);
            w.atbash[t] = bucket(s.atbash);
            w.root[t] = (1 + 10 * digit_root(s.ordinal) + digit_root(s.reversed)) as u8;
        };
        if causal {
            for t in start..i {
                fill(t + 1, simd::sums(&bytes[start..t + 1]));
            }
        } else {
            let s = simd::sums(&bytes[start..i]);
            for t in start + 1..i + 1 {
                fill(t, s);
            }
        }
    }
    w
}

impl Window {
    /// appends NUM tokens carrying `values` (no channels)
    pub fn push_numbers(&mut self, values: &[f32]) {
        if self.values.is_empty() {
            self.values = vec![0.0; self.tokens.len()];
        }
        for &v in values {
            self.tokens.push(NUM);
            for c in [
                &mut self.letter,
                &mut self.ordinal,
                &mut self.reversed,
                &mut self.atbash,
                &mut self.root,
            ] {
                c.push(0);
            }
            self.values.push(v);
        }
    }

    /// the value at token t (0 when the window has no numbers)
    pub fn value(&self, t: usize) -> f32 {
        self.values.get(t).copied().unwrap_or(0.0)
    }
}

/// where the windows of a `len` byte text start, for windows of `room` bytes advancing by
/// `stride`: 0, stride, 2·stride, …, and the last one flush with the end
pub fn starts(len: usize, room: usize, stride: usize) -> Vec<usize> {
    if len <= room {
        return vec![0];
    }
    let stride = stride.clamp(1, room);
    let last = len - room;
    let mut v: Vec<usize> = (0..last).step_by(stride).collect();
    v.push(last);
    v
}

/// every window of a text for a model whose sequences are `max_len` tokens long
pub fn windows(text: &str, max_len: usize, stride: usize) -> Vec<Window> {
    windows_of(text.as_bytes(), max_len, stride, false)
}

/// every window of some bytes; `causal` windows have causal word channels and end in
/// SEP only where the data ends
pub fn windows_of(b: &[u8], max_len: usize, stride: usize, causal: bool) -> Vec<Window> {
    let room = max_len.saturating_sub(2).max(1);
    starts(b.len(), room, stride)
        .into_iter()
        .map(|s| {
            let e = (s + room).min(b.len());
            window_with(&b[s..e], causal, !causal || e == b.len())
        })
        .collect()
}

/// the text level measures of a window, each scaled to about 0..1:
///
/// | index | measure |
/// |---|---|
/// | 0..26 | letter frequencies a..z as shares of the letters, ×10 |
/// | 26..29 | mean ordinal, reversed and Atbash value per letter, /26 |
/// | 29..32 | ln(1 + sum)/10 for the three sums |
/// | 32..35 | digit roots of the three sums, /9 |
/// | 35..39 | the ordinal sum's prime factors: count/8, largest/sum, chain length/32, root of the chain total/9 |
/// | 39..41 | ln(1 + bytes)/8, ln(1 + words)/8 |
/// | 41..46 | shares of letters, capitals, punctuation, digits, whitespace |
pub fn side(bytes: &[u8]) -> Vec<f32> {
    let mut v = Vec::with_capacity(SIDE);
    let f = simd::frequencies(bytes);
    let s = simd::sums(bytes);
    let letters = s.letters.max(1) as f32;
    v.extend(f.iter().map(|&c| c as f32 / letters * 10.0));
    for sum in [s.ordinal, s.reversed, s.atbash] {
        v.push(sum as f32 / letters / 26.0);
    }
    for sum in [s.ordinal, s.reversed, s.atbash] {
        v.push(ln(1.0 + sum as f32) / 10.0);
    }
    for sum in [s.ordinal, s.reversed, s.atbash] {
        v.push(digit_root(sum) as f32 / 9.0);
    }
    let pf = weum::prime_factors(s.ordinal);
    v.push(pf.len() as f32 / 8.0);
    v.push(pf.last().map_or(0.0, |&p| p as f32 / s.ordinal.max(1) as f32));
    let nums: Vec<u64> = weum::pf_reduction(s.ordinal)
        .into_iter()
        .filter_map(|g| match g {
            weum::Seg::Num(n) => Some(n),
            weum::Seg::Text(_) => None,
        })
        .collect();
    v.push(nums.len().saturating_sub(1) as f32 / 32.0);
    v.push(digit_root(nums.iter().fold(0u64, |a, &n| a.wrapping_add(n))) as f32 / 9.0);
    let words = simd::tokenize(bytes).len();
    v.push(ln(1.0 + bytes.len() as f32) / 8.0);
    v.push(ln(1.0 + words as f32) / 8.0);
    let total = bytes.len().max(1) as f32;
    let count = |p: fn(&u8) -> bool| bytes.iter().filter(|b| p(b)).count() as f32 / total;
    v.push(count(|b| b.is_ascii_alphabetic()));
    v.push(count(|b| b.is_ascii_uppercase()));
    v.push(count(|b| b.is_ascii_punctuation()));
    v.push(count(|b| b.is_ascii_digit()));
    v.push(count(|b| b.is_ascii_whitespace()));
    debug_assert_eq!(v.len(), SIDE);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels() {
        let w = window(b"Ab, c");
        assert_eq!(w.tokens, vec![CLS, 65, 98, 44, 32, 99, SEP]);
        assert_eq!(w.letter, vec![0, 1, 2, 0, 0, 3, 0]);
        // "Ab" sums to 3 ordinal, 51 reversed
        assert_eq!(w.ordinal[1], 4);
        assert_eq!(w.ordinal[2], 4);
        assert_eq!(w.reversed[1], 52);
        assert_eq!(w.ordinal[3], 0);
        assert_eq!(w.root[5], (1 + 10 * 3 + 6) as u8);
        assert_eq!(w.side.len(), SIDE);
        assert!(w.side.iter().all(|x| x.is_finite()));
        assert_eq!(window(b"").tokens, vec![CLS, SEP]);
    }

    #[test]
    fn causal_channels_and_numbers() {
        let c = window_with(b"Ab c", true, false);
        assert_eq!(c.tokens, vec![CLS, 65, 98, 32, 99]);
        // "A" so far sums to 1, "Ab" to 3
        assert_eq!((c.ordinal[1], c.ordinal[2]), (2, 4));
        let full = window(b"Ab c");
        assert_eq!(full.ordinal[1], 4);
        let mut n = window(b"x");
        n.push_numbers(&[1.5, -2.0]);
        assert_eq!(n.tokens, vec![CLS, 120, SEP, NUM, NUM]);
        assert_eq!(n.values, vec![0.0, 0.0, 0.0, 1.5, -2.0]);
        assert_eq!(n.root.len(), 5);
        let ws = windows_of(&[b'a'; 100], 32, 15, true);
        assert!(ws[..ws.len() - 1].iter().all(|w| *w.tokens.last().unwrap() != SEP));
        assert_eq!(*ws.last().unwrap().tokens.last().unwrap(), SEP);
    }

    #[test]
    fn window_starts() {
        assert_eq!(starts(10, 30, 15), vec![0]);
        assert_eq!(starts(100, 30, 15), vec![0, 15, 30, 45, 60, 70]);
        assert_eq!(starts(31, 30, 30), vec![0, 1]);
        let ws = windows(&"x".repeat(100), 32, 15);
        assert!(ws.iter().all(|w| w.len() <= 32));
        assert_eq!(ws.len(), starts(100, 30, 15).len());
    }
}
