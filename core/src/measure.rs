//! Byte sums: the gematria idea generalised from letters to any data. Every byte
//! counts, valued 1 to 256 (`b + 1`), so text in any script, binary files, pictures and
//! numbers all have a sum. A standalone measure, like the letter sums in `simd`; nothing
//! here feeds the transformers in `classify`, which are general purpose.
//!
//! | measure | value of byte `b` |
//! |---|---|
//! | `bytes` | `b + 1` (1 ..= 256) |
//! | `reversed` | `256 − b` (256 ..= 1), so `bytes + reversed = 257·len` |
//! | `digits` | the sum of the decimal digits of `b + 1` |

use serde::{Deserialize, Serialize};

/// the byte sums of some data
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteSums {
    /// Σ (b + 1)
    pub bytes: u64,
    /// Σ (256 − b)
    pub reversed: u64,
    /// Σ digit_sum(b + 1)
    pub digits: u64,
    /// how many bytes were summed
    pub len: u64,
}

impl ByteSums {
    /// the digit roots of `bytes`, `reversed` and `digits` (0 for 0, else 1 + (n − 1) mod 9)
    pub fn roots(&self) -> [u64; 3] {
        [digit_root(self.bytes), digit_root(self.reversed), digit_root(self.digits)]
    }

    /// the mean byte value, 1 ..= 256 (0 for no bytes)
    pub fn mean(&self) -> f64 {
        if self.len == 0 { 0.0 } else { self.bytes as f64 / self.len as f64 }
    }
}

/// the value of one byte: 1 ..= 256
pub fn value(b: u8) -> u64 {
    b as u64 + 1
}

/// the sum of n's decimal digits
pub fn digit_sum(mut n: u64) -> u64 {
    let mut s = 0;
    while n > 0 {
        s += n % 10;
        n /= 10;
    }
    s
}

/// 0 for 0, else 1 + (n − 1) mod 9
pub fn digit_root(n: u64) -> u64 {
    if n == 0 { 0 } else { 1 + (n - 1) % 9 }
}

const DIGITS: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut n = i + 1;
        let mut s = 0;
        while n > 0 {
            s += n % 10;
            n /= 10;
        }
        t[i] = s as u8;
        i += 1;
    }
    t
};

/// every byte sum of `data`
pub fn byte_sums(data: &[u8]) -> ByteSums {
    let (mut bytes, mut digits) = (0u64, 0u64);
    for &b in data {
        bytes += b as u64;
        digits += DIGITS[b as usize] as u64;
    }
    let len = data.len() as u64;
    bytes += len;
    ByteSums {
        bytes,
        reversed: 257 * len - bytes,
        digits,
        len,
    }
}

/// Σ (b + 1) alone
pub fn byte_sum(data: &[u8]) -> u64 {
    data.iter().map(|&b| b as u64).sum::<u64>() + data.len() as u64
}

/// the byte sum of each word (runs of bytes that are not ASCII whitespace or
/// punctuation; bytes ≥ 0x80 count as word bytes, so UTF-8 letters stay in their word)
pub fn word_sums(data: &[u8]) -> Vec<(std::ops::Range<usize>, u64)> {
    let in_word = |b: u8| !(b.is_ascii_whitespace() || b.is_ascii_punctuation() || b.is_ascii_control());
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if !in_word(data[i]) {
            i += 1;
            continue;
        }
        let s = i;
        while i < data.len() && in_word(data[i]) {
            i += 1;
        }
        out.push((s..i, byte_sum(&data[s..i])));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums() {
        let s = byte_sums(b"ab");
        // a = 97 → 98, b = 98 → 99
        assert_eq!(s.bytes, 197);
        assert_eq!(s.reversed, (256 - 97) + (256 - 98));
        assert_eq!(s.digits, (9 + 8) + (9 + 9));
        assert_eq!(s.len, 2);
        assert_eq!(s.bytes + s.reversed, 257 * 2);
        assert_eq!(byte_sum(b"ab"), 197);
        assert_eq!(byte_sums(&[0, 255]).bytes, 1 + 256);
        assert_eq!(byte_sums(&[]), ByteSums::default());
        assert_eq!(byte_sums(&[0, 255]).roots(), [digit_root(257), digit_root(257), digit_root(1 + 13)]);
        assert_eq!(digit_sum(256), 13);
        assert!((byte_sums(b"aa").mean() - 98.0).abs() < 1e-12);
    }

    #[test]
    fn any_bytes_and_words() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(byte_sum(&all), 256 * 257 / 2);
        assert_eq!(byte_sums(&all).reversed, 256 * 257 / 2);
        let w = word_sums("hi, é!".as_bytes());
        assert_eq!(w.len(), 2);
        assert_eq!(w[0], (0..2, 105 + 106));
        assert_eq!(w[1].0, 4..6);
        assert_eq!(w[1].1, (0xc3 + 1) + (0xa9 + 1));
    }
}
