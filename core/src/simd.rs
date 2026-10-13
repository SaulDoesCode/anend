//! SIMD text kernels, ported from weum: the whitespace tokenizer and the gematria dual
//! sum, plus the letter frequency table. Each kernel has an AVX2 path (picked at runtime)
//! and a scalar path, which is also the reference the tests hold the AVX2 path to.
//!
//! Letter values: a=1 .. z=26 (ordinal), a=26 .. z=1 (reversed, 27 - v) and Atbash: each
//! letter swapped for its mirror in its own alphabet, `n + 1 - v` for an alphabet of n
//! letters, counted over Latin (a..z), Greek (α..ω, 24 letters) and Hebrew (א..ת, 22
//! letters). For Latin letters that is the reversed value, so the SIMD dual sum covers
//! them and only text with Greek or Hebrew in it takes a scalar pass over its characters.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// the three gematria measures of a text and its letter count
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sums {
    pub ordinal: u64,
    pub reversed: u64,
    pub atbash: u64,
    pub letters: u64,
}

impl Sums {
    fn from_dual(ordinal: u64, reversed: u64) -> Sums {
        let letters = (ordinal + reversed) / 27;
        Sums {
            ordinal,
            reversed,
            atbash: reversed,
            letters,
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn has_avx2() -> bool {
    std::is_x86_feature_detected!("avx2")
}

// ---------------------------------------------------------------- gematria

/// ordinal, reversed and Atbash sums of the ASCII letters in `data`, case-insensitively
pub fn sums(data: &[u8]) -> Sums {
    #[cfg(target_arch = "x86_64")]
    if has_avx2() {
        // SAFETY: AVX2 support was just checked
        let (o, r) = unsafe { dual_sum_avx2(data) };
        return Sums::from_dual(o, r).with_other_alphabets(data);
    }
    sums_scalar(data)
}

pub const GREEK: &str = "αβγδεζηθικλμνξοπρστυφχψω";
pub const HEBREW: &str = "אבגדהוזחטיכלמנסעפצקרשת";

/// the Atbash sum of the Greek and Hebrew letters of a text: `n + 1 - v` in each alphabet,
/// after lower-casing the whole text (so a word-final Σ becomes ς, which isn't one of the
/// 24, and Hebrew final forms don't count either)
pub fn other_alphabets_atbash(text: &str) -> u64 {
    let lower = text.to_lowercase();
    let mut sum = 0;
    for c in lower.chars() {
        for alphabet in [GREEK, HEBREW] {
            if let Some(i) = alphabet.chars().position(|a| a == c) {
                sum += (alphabet.chars().count() - i) as u64;
            }
        }
    }
    sum
}

impl Sums {
    fn with_other_alphabets(mut self, data: &[u8]) -> Sums {
        if !data.is_ascii() {
            self.atbash += other_alphabets_atbash(&String::from_utf8_lossy(data));
        }
        self
    }
}

pub fn sums_scalar(data: &[u8]) -> Sums {
    sums_ascii_scalar(data).with_other_alphabets(data)
}

fn sums_ascii_scalar(data: &[u8]) -> Sums {
    let (mut o, mut r) = (0u64, 0u64);
    for &b in data {
        let lc = b | 0x20;
        if lc.is_ascii_lowercase() {
            let v = (lc - b'a' + 1) as u64;
            o += v;
            r += 27 - v;
        }
    }
    Sums::from_dual(o, r)
}

/// AVX2 fast path from weum: 32 bytes per block, 4 blocks unrolled, PSADBW accumulation.
///
/// Unlike weum's kernel, the letter test is `(b | 0x20) - 'a' <= 25` rather than
/// `(b | 0x20) - ('a' - 1) <= 26`: with the latter, '@' and '`' pass as letters of value
/// 0 and add 27 to the reversed sum.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn dual_sum_avx2(data: &[u8]) -> (u64, u64) {
    let lower = _mm256_set1_epi8(0x20);
    let a = _mm256_set1_epi8(b'a' as i8);
    let max = _mm256_set1_epi8(25);
    let one = _mm256_set1_epi8(1);
    let twentyseven = _mm256_set1_epi8(27);
    let zero = _mm256_setzero_si256();

    let mut s = [zero; 4];
    let mut r = [zero; 4];

    macro_rules! block {
        ($i:expr, $p:expr) => {{
            let v = unsafe { _mm256_loadu_si256($p as *const __m256i) };
            // 0 for 'a' .. 25 for 'z', anything else lands above 25
            let off = _mm256_sub_epi8(_mm256_or_si256(v, lower), a);
            let mask = _mm256_cmpeq_epi8(_mm256_min_epu8(off, max), off);
            let gval = _mm256_and_si256(_mm256_add_epi8(off, one), mask);
            let rval = _mm256_and_si256(_mm256_sub_epi8(twentyseven, gval), mask);
            s[$i] = _mm256_add_epi64(s[$i], _mm256_sad_epu8(gval, zero));
            r[$i] = _mm256_add_epi64(r[$i], _mm256_sad_epu8(rval, zero));
        }};
    }

    let len = data.len();
    let ptr = data.as_ptr();
    let mut i = 0;
    while i + 128 <= len {
        block!(0, ptr.add(i));
        block!(1, ptr.add(i + 32));
        block!(2, ptr.add(i + 64));
        block!(3, ptr.add(i + 96));
        i += 128;
    }
    while i + 32 <= len {
        block!(0, ptr.add(i));
        i += 32;
    }

    let reduce = |acc: [__m256i; 4]| -> u64 {
        let t = _mm256_add_epi64(_mm256_add_epi64(acc[0], acc[1]), _mm256_add_epi64(acc[2], acc[3]));
        let lanes = _mm_add_epi64(_mm256_castsi256_si128(t), _mm256_extracti128_si256(t, 1));
        (_mm_cvtsi128_si64(lanes) as u64).wrapping_add(_mm_extract_epi64(lanes, 1) as u64)
    };
    let tail = sums_scalar(&data[i..]);
    (reduce(s) + tail.ordinal, reduce(r) + tail.reversed)
}

// ---------------------------------------------------------------- frequency table

/// how often each letter a..z occurs, case-insensitively
pub fn frequencies(data: &[u8]) -> [u32; 26] {
    #[cfg(target_arch = "x86_64")]
    if has_avx2() {
        // SAFETY: AVX2 support was just checked
        return unsafe { frequencies_avx2(data) };
    }
    frequencies_scalar(data)
}

pub fn frequencies_scalar(data: &[u8]) -> [u32; 26] {
    let mut counts = [0u32; 26];
    for &b in data {
        let lc = b | 0x20;
        if lc.is_ascii_lowercase() {
            counts[(lc - b'a') as usize] += 1;
        }
    }
    counts
}

/// per 32 byte block: lowercase once, then one compare + popcount per letter
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn frequencies_avx2(data: &[u8]) -> [u32; 26] {
    let lower = _mm256_set1_epi8(0x20);
    let mut counts = [0u32; 26];
    let len = data.len();
    let mut i = 0;
    while i + 32 <= len {
        let v = unsafe { _mm256_loadu_si256(data.as_ptr().add(i) as *const __m256i) };
        let lc = _mm256_or_si256(v, lower);
        for (l, count) in counts.iter_mut().enumerate() {
            let hit = _mm256_cmpeq_epi8(lc, _mm256_set1_epi8((b'a' + l as u8) as i8));
            *count += (_mm256_movemask_epi8(hit) as u32).count_ones();
        }
        i += 32;
    }
    let tail = frequencies_scalar(&data[i..]);
    for (c, t) in counts.iter_mut().zip(tail) {
        *c += t;
    }
    counts
}

/// the frequency table as a compact key, letters in order with their counts:
/// "listen" -> "e1i1l1n1s1t1"; texts that are anagrams of each other share it
pub fn frequency_key(data: &[u8]) -> String {
    let mut key = String::new();
    for (l, &c) in frequencies(data).iter().enumerate() {
        if c > 0 {
            key.push((b'a' + l as u8) as char);
            key.push_str(&c.to_string());
        }
    }
    key
}

/// a readable form of a frequency key: the letters sorted ("eilnst"), or the key itself
/// when that would be long
pub fn frequency_letters(key: &str) -> String {
    let mut out = String::new();
    let mut chars = key.chars().peekable();
    while let Some(letter) = chars.next() {
        let mut digits = String::new();
        while let Some(d) = chars.peek().filter(|c| c.is_ascii_digit()) {
            digits.push(*d);
            chars.next();
        }
        let n: usize = digits.parse().unwrap_or(0);
        if out.len() + n > 24 {
            return key.to_string();
        }
        out.extend(std::iter::repeat_n(letter, n));
    }
    out
}

// ---------------------------------------------------------------- tokenizer

fn is_delimiter(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\t' | b'\r')
}

/// splits on spaces, newlines, tabs and carriage returns like weum's `tokenate`; tokens
/// are the raw bytes between delimiters, in order
pub fn tokenize(data: &[u8]) -> Vec<&[u8]> {
    #[cfg(target_arch = "x86_64")]
    if has_avx2() {
        // SAFETY: AVX2 support was just checked
        return unsafe { tokenize_avx2(data) };
    }
    tokenize_scalar(data)
}

pub fn tokenize_scalar(data: &[u8]) -> Vec<&[u8]> {
    data.split(|&b| is_delimiter(b)).filter(|t| !t.is_empty()).collect()
}

/// 32 bytes at a time: one delimiter bitmask per block, from which word starts and ends
/// fall out as bit transitions (carried across blocks)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn tokenize_avx2(data: &[u8]) -> Vec<&[u8]> {
    let space = _mm256_set1_epi8(b' ' as i8);
    let newline = _mm256_set1_epi8(b'\n' as i8);
    let tab = _mm256_set1_epi8(b'\t' as i8);
    let cr = _mm256_set1_epi8(b'\r' as i8);

    let mut tokens = Vec::new();
    let mut start = 0usize;
    let mut in_word = false;
    let len = data.len();
    let mut i = 0;
    while i + 32 <= len {
        let v = unsafe { _mm256_loadu_si256(data.as_ptr().add(i) as *const __m256i) };
        let delim = _mm256_or_si256(
            _mm256_or_si256(_mm256_cmpeq_epi8(v, space), _mm256_cmpeq_epi8(v, newline)),
            _mm256_or_si256(_mm256_cmpeq_epi8(v, tab), _mm256_cmpeq_epi8(v, cr)),
        );
        let word = !(_mm256_movemask_epi8(delim) as u32);
        // a bit per byte saying whether the byte before it was part of a word
        let before = (word << 1) | in_word as u32;
        let mut edges = (word & !before) | (!word & before);
        while edges != 0 {
            let bit = edges.trailing_zeros() as usize;
            if word >> bit & 1 == 1 {
                start = i + bit;
            } else {
                tokens.push(&data[start..i + bit]);
            }
            edges &= edges - 1;
        }
        in_word = word >> 31 == 1;
        i += 32;
    }
    for (j, &b) in data.iter().enumerate().skip(i) {
        match (is_delimiter(b), in_word) {
            (true, true) => {
                tokens.push(&data[start..j]);
                in_word = false;
            }
            (false, false) => {
                start = j;
                in_word = true;
            }
            _ => {}
        }
    }
    if in_word {
        tokens.push(&data[start..]);
    }
    tokens
}

/// distinct tokens in order of first appearance
pub fn unique_tokens(data: &[u8]) -> Vec<&[u8]> {
    let mut seen = std::collections::HashSet::new();
    tokenize(data).into_iter().filter(|t| seen.insert(*t)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a small deterministic generator, so failures reproduce
    fn noise(seed: u64, len: usize, alphabet: &[u8]) -> Vec<u8> {
        let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                alphabet[(x % alphabet.len() as u64) as usize]
            })
            .collect()
    }

    const WORDY: &[u8] = b"abcXYZ  \n\t\r@`[{-,.0189 \xc3\xa9\xff";

    #[test]
    fn known_values() {
        // values from weum's gsum
        assert_eq!((sums(b"hello").ordinal, sums(b"hello").reversed), (52, 83));
        assert_eq!(
            (sums(b"Hello World").ordinal, sums(b"Hello World").reversed),
            (124, 146)
        );
        assert_eq!(sums(b"The quick brown fox").ordinal, 211);
        assert_eq!(sums(b"The quick brown fox").reversed, 221);
        let abc = sums(b"abc xyz!");
        assert_eq!((abc.ordinal, abc.reversed, abc.letters), (81, 81, 6));
        // atbashSum: a=26 .. z=1 in Latin, ω=1 .. α=24 in Greek, ת=1 .. א=22 in Hebrew
        assert_eq!(sums(b"az").atbash, 27);
        assert_eq!(sums(b"hello").atbash, 83);
        assert_eq!(sums("αω".as_bytes()).atbash, 24 + 1);
        assert_eq!(sums("Λόγος".as_bytes()).atbash, (24 - 10) + (24 - 2) + (24 - 14));
        assert_eq!(sums("אבת".as_bytes()).atbash, 22 + 21 + 1);
        assert_eq!(sums("ab α".as_bytes()).atbash, 26 + 25 + 24);
        assert_eq!(sums("שלום".as_bytes()).atbash, (23 - 21) + (23 - 12) + (23 - 6));
        // '@' and '`' are not letters
        assert_eq!(sums(b"@`[{"), Sums::default());
    }

    #[test]
    fn avx2_matches_scalar() {
        for seed in 0..200u64 {
            let len = (seed as usize * 37) % 700;
            let data = noise(seed, len, WORDY);
            assert_eq!(sums(&data), sums_scalar(&data), "sums, seed {}", seed);
            assert_eq!(
                frequencies(&data),
                frequencies_scalar(&data),
                "frequencies, seed {}",
                seed
            );
            assert_eq!(tokenize(&data), tokenize_scalar(&data), "tokens, seed {}", seed);
        }
        // every byte value, at every alignment
        let all: Vec<u8> = (0..=255u8).cycle().take(1024).collect();
        for off in 0..64 {
            assert_eq!(sums(&all[off..]), sums_scalar(&all[off..]));
            assert_eq!(frequencies(&all[off..]), frequencies_scalar(&all[off..]));
        }
    }

    #[test]
    fn tokens_across_block_boundaries() {
        // words straddling the 32 byte blocks, runs of delimiters, leading/trailing ones
        let mut text = Vec::new();
        for w in 0..60 {
            text.extend(std::iter::repeat_n(b'x', w % 7 + 1));
            text.extend(std::iter::repeat_n(b' ', w % 3 + 1));
        }
        assert_eq!(tokenize(&text), tokenize_scalar(&text));
        let solid = vec![b'q'; 100];
        assert_eq!(tokenize(&solid), vec![&solid[..]]);
        assert!(tokenize(&[b' '; 70]).is_empty());
        assert_eq!(tokenize(b"  Rust: is\tfun\r\n"), vec![&b"Rust:"[..], b"is", b"fun"]);
        assert_eq!(unique_tokens(b"a b a c b"), vec![&b"a"[..], b"b", b"c"]);
    }

    #[test]
    fn frequency_keys() {
        assert_eq!(frequency_key(b"Listen"), "e1i1l1n1s1t1");
        assert_eq!(frequency_key(b"Silent!"), frequency_key(b"listen"));
        assert_eq!(frequency_letters("e1i1l1n1s1t1"), "eilnst");
        assert_eq!(frequency_letters("a30"), "a30");
        assert_eq!(frequency_key(b"123"), "");
    }
}
