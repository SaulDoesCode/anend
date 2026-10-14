//! The text codec: 5-bit tokens, bit-packed, most significant bit first.
//!
//! | token | meaning |
//! |---|---|
//! | 0 | space |
//! | 1–26 | `a`–`z` (`A`–`Z` while caps lock is on) |
//! | 27 `ESC` | an escape: 8 payload bits follow (see below) |
//! | 28 `UPPER` | the next letter takes the other case; `UPPER UPPER` toggles caps lock |
//! | 29 `REF` | an entry id follows as a varint: transclude that entry's rendered text |
//! | 30 `CALL` | a callee varint, an argument count varint, then each argument as a bit-length varint and that many bits of a nested run |
//! | 31 | newline |
//!
//! A **varint** is 5-bit chunks, least significant group first: bit 4 says another chunk
//! follows, bits 0–3 are data. Ids under 16 take one chunk, under 256 two, under 65536 four.
//!
//! The **escape payload** byte `b`:
//! - `0x00..=0x7F`: that ASCII byte (letters, space and newline have their own tokens);
//! - `0xC2..=0xF4`: a UTF-8 lead byte; its continuation bytes follow as 6 bits each (the
//!   `10` top bits are implied), and it becomes the run's *last lead*;
//! - `0x80..=0xBF`: `n = b - 0x7F` characters (1–64) that share the last lead byte, each as
//!   its continuation bits only (Hebrew, Cyrillic and most CJK runs pay 6 or 12 bits per
//!   character instead of 16 or 24);
//! - `0xF5`: a digit run: 5 bits holding `n - 2`, then `n` (2–33) digits of 4 bits each;
//! - anything else (`0xC0`, `0xC1`, `0xF6..=0xFF`) is reserved, and fails to decode.
//!
//! A **callee** under 32 is a builtin (see `interp`); an entry `id` is written `id + 32`.
//!
//! A **run** is a sequence of segments (text, refs and calls). Caps lock and the last lead
//! start off at the start of every run, so a call's arguments decode on their own.
//!
//! A **body** (what an entry stores) is a run with a 3-bit header holding how many padding
//! bits end its last byte; an empty body is zero bytes.

use std::sync::Arc;

pub const SPACE: u32 = 0;
pub const ESC: u32 = 27;
pub const UPPER: u32 = 28;
pub const REF: u32 = 29;
pub const CALL: u32 = 30;
pub const NEWLINE: u32 = 31;

const DIGITS: u8 = 0xF5;
/// how deep calls may nest inside a body (decoding recurses per level)
pub const MAX_NESTING: usize = 64;

/// what a call calls
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Callee {
    /// one of the interpreter's builtins (0–31)
    Builtin(u8),
    /// a function (or any) entry, its body evaluated with the arguments
    Entry(u64),
}

/// a nested run (a call's argument)
pub type Run = Arc<Vec<Seg>>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Seg {
    Text(String),
    Ref(u64),
    Call { f: Callee, args: Vec<Run> },
}

impl Seg {
    pub fn text(s: impl Into<String>) -> Seg {
        Seg::Text(s.into())
    }

    pub fn call(f: Callee, args: Vec<Vec<Seg>>) -> Seg {
        Seg::Call {
            f,
            args: args.into_iter().map(Arc::new).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// the bits ran out inside a token
    Truncated,
    /// a reserved escape, a bad case token, a run without a lead, a bad digit
    Invalid(&'static str),
    /// the bytes don't make UTF-8
    Utf8,
    /// calls nested deeper than `MAX_NESTING`
    TooDeep,
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CodecError::Truncated => write!(f, "the packed text ends inside a token"),
            CodecError::Invalid(why) => write!(f, "the packed text is invalid: {why}"),
            CodecError::Utf8 => write!(f, "the packed text doesn't decode to UTF-8"),
            CodecError::TooDeep => write!(f, "calls nest deeper than {MAX_NESTING}"),
        }
    }
}

// ---------------------------------------------------------------- bits

#[derive(Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    pub fn put(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (value >> i) & 1 == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }

    pub fn varint(&mut self, mut v: u64) {
        loop {
            let data = (v & 0xF) as u32;
            v >>= 4;
            if v == 0 {
                self.put(data, 5);
                return;
            }
            self.put(0x10 | data, 5);
        }
    }

    /// appends another writer's bits
    pub fn append(&mut self, other: &BitWriter) {
        let mut r = BitReader::new(&other.bytes, 0, other.bits);
        let mut left = other.bits;
        while left > 0 {
            let n = left.min(32) as u32;
            self.put(r.get(n).unwrap_or(0), n);
            left -= n as usize;
        }
    }

    pub fn bits(&self) -> usize {
        self.bits
    }
}

pub struct BitReader<'a> {
    bytes: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(bytes: &'a [u8], pos: usize, end: usize) -> BitReader<'a> {
        BitReader {
            bytes,
            pos,
            end: end.min(bytes.len() * 8),
        }
    }

    pub fn left(&self) -> usize {
        self.end.saturating_sub(self.pos)
    }

    pub fn get(&mut self, n: u32) -> Result<u32, CodecError> {
        if self.left() < n as usize {
            return Err(CodecError::Truncated);
        }
        let mut v = 0u32;
        let mut need = n as usize;
        while need > 0 {
            let byte = self.bytes[self.pos / 8];
            let off = self.pos % 8;
            let take = need.min(8 - off);
            let chunk = (byte >> (8 - off - take)) & ((1u16 << take) - 1) as u8;
            v = (v << take) | chunk as u32;
            self.pos += take;
            need -= take;
        }
        Ok(v)
    }

    pub fn varint(&mut self) -> Result<u64, CodecError> {
        let mut v = 0u64;
        for shift in (0..64).step_by(4) {
            let c = self.get(5)? as u64;
            v |= (c & 0xF) << shift;
            if c & 0x10 == 0 {
                return Ok(v);
            }
        }
        Err(CodecError::Invalid("a varint longer than 64 bits"))
    }
}

// ---------------------------------------------------------------- encoding

fn letter(c: char) -> Option<u32> {
    c.is_ascii_lowercase().then(|| c as u32 - 'a' as u32 + 1)
}

fn is_cased(c: char) -> bool {
    c.is_ascii_alphabetic()
}

/// how many continuation bytes a UTF-8 lead byte has
fn continuations(lead: u8) -> Option<usize> {
    match lead {
        0xC2..=0xDF => Some(1),
        0xE0..=0xEF => Some(2),
        0xF0..=0xF4 => Some(3),
        _ => None,
    }
}

struct RunState {
    caps: bool,
    lead: Option<u8>,
}

fn encode_text(w: &mut BitWriter, text: &str, st: &mut RunState) {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' {
            w.put(SPACE, 5);
        } else if c == '\n' {
            w.put(NEWLINE, 5);
        } else if is_cased(c) {
            let upper = c.is_ascii_uppercase();
            let v = letter(c.to_ascii_lowercase()).unwrap();
            if upper != st.caps {
                // switch the lock when the case run ahead is long enough to pay for it
                let ahead = chars[i..]
                    .iter()
                    .filter(|c| c.is_ascii_alphabetic())
                    .take_while(|c| c.is_ascii_uppercase() == upper)
                    .take(4)
                    .count();
                let switch = if upper { ahead >= 4 } else { ahead >= 2 };
                if switch {
                    w.put(UPPER, 5);
                    w.put(UPPER, 5);
                    st.caps = upper;
                } else {
                    w.put(UPPER, 5);
                }
            }
            w.put(v, 5);
        } else if c.is_ascii_digit() {
            let n = chars[i..].iter().take(33).take_while(|c| c.is_ascii_digit()).count();
            if n >= 3 {
                w.put(ESC, 5);
                w.put(DIGITS as u32, 8);
                w.put(n as u32 - 2, 5);
                for d in &chars[i..i + n] {
                    w.put(*d as u32 - '0' as u32, 4);
                }
                i += n;
                continue;
            }
            w.put(ESC, 5);
            w.put(c as u32, 8);
        } else if c.is_ascii() {
            w.put(ESC, 5);
            w.put(c as u32, 8);
        } else {
            let mut buf = [0u8; 4];
            let lead = c.encode_utf8(&mut buf).as_bytes()[0];
            if st.lead == Some(lead) {
                // a run of characters sharing the last lead byte
                let n = chars[i..]
                    .iter()
                    .take(64)
                    .take_while(|c| {
                        let mut b = [0u8; 4];
                        !c.is_ascii() && c.encode_utf8(&mut b).as_bytes()[0] == lead
                    })
                    .count();
                w.put(ESC, 5);
                w.put(0x7F + n as u32, 8);
                for ch in &chars[i..i + n] {
                    let mut b = [0u8; 4];
                    for cont in &ch.encode_utf8(&mut b).as_bytes()[1..] {
                        w.put((cont & 0x3F) as u32, 6);
                    }
                }
                i += n;
                continue;
            }
            let bytes = c.encode_utf8(&mut buf).as_bytes();
            w.put(ESC, 5);
            w.put(lead as u32, 8);
            for cont in &bytes[1..] {
                w.put((cont & 0x3F) as u32, 6);
            }
            st.lead = Some(lead);
        }
        i += 1;
    }
}

fn encode_run(w: &mut BitWriter, segs: &[Seg]) {
    let mut st = RunState {
        caps: false,
        lead: None,
    };
    for s in segs {
        match s {
            Seg::Text(t) => encode_text(w, t, &mut st),
            Seg::Ref(id) => {
                w.put(REF, 5);
                w.varint(*id);
            }
            Seg::Call { f, args } => {
                w.put(CALL, 5);
                w.varint(match f {
                    Callee::Builtin(b) => (*b & 31) as u64,
                    Callee::Entry(id) => id.saturating_add(32),
                });
                w.varint(args.len() as u64);
                for a in args {
                    let mut inner = BitWriter::default();
                    encode_run(&mut inner, a);
                    w.varint(inner.bits() as u64);
                    w.append(&inner);
                }
            }
        }
    }
}

/// packs a run into a body
pub fn pack(segs: &[Seg]) -> Vec<u8> {
    if segs.iter().all(|s| matches!(s, Seg::Text(t) if t.is_empty())) {
        return Vec::new();
    }
    let mut w = BitWriter::default();
    w.put(0, 3);
    encode_run(&mut w, segs);
    let pad = (8 - w.bits % 8) % 8;
    w.bytes[0] |= (pad as u8) << 5;
    w.bytes
}

pub fn pack_text(text: &str) -> Vec<u8> {
    if text.is_empty() {
        return Vec::new();
    }
    pack(&[Seg::Text(text.to_string())])
}

/// how many bits a text takes inside a run (no body header), for weighing borrowing
pub fn text_bits(text: &str) -> usize {
    let mut w = BitWriter::default();
    encode_text(
        &mut w,
        text,
        &mut RunState {
            caps: false,
            lead: None,
        },
    );
    w.bits()
}

/// how many bits a ref to this id takes
pub fn ref_bits(id: u64) -> usize {
    let mut w = BitWriter::default();
    w.varint(id);
    5 + w.bits()
}

// ---------------------------------------------------------------- decoding

fn push_text(out: &mut Vec<Seg>, bytes: &mut Vec<u8>) -> Result<(), CodecError> {
    if bytes.is_empty() {
        return Ok(());
    }
    let s = String::from_utf8(std::mem::take(bytes)).map_err(|_| CodecError::Utf8)?;
    match out.last_mut() {
        Some(Seg::Text(t)) => t.push_str(&s),
        _ => out.push(Seg::Text(s)),
    }
    Ok(())
}

fn decode_run(r: &mut BitReader, depth: usize) -> Result<Vec<Seg>, CodecError> {
    if depth > MAX_NESTING {
        return Err(CodecError::TooDeep);
    }
    let mut out = Vec::new();
    let mut text: Vec<u8> = Vec::new();
    let mut caps = false;
    let mut lead: Option<u8> = None;
    while r.left() >= 5 {
        let t = r.get(5)?;
        match t {
            SPACE => text.push(b' '),
            1..=26 => text.push(if caps { b'A' } else { b'a' } + t as u8 - 1),
            NEWLINE => text.push(b'\n'),
            UPPER => match r.get(5)? {
                UPPER => caps = !caps,
                l @ 1..=26 => text.push(if caps { b'a' } else { b'A' } + l as u8 - 1),
                _ => return Err(CodecError::Invalid("UPPER before a non-letter")),
            },
            ESC => {
                let b = r.get(8)? as u8;
                if b < 0x80 {
                    text.push(b);
                } else if let Some(k) = continuations(b) {
                    text.push(b);
                    for _ in 0..k {
                        text.push(0x80 | r.get(6)? as u8);
                    }
                    lead = Some(b);
                } else if (0x80..=0xBF).contains(&b) {
                    let l = lead.ok_or(CodecError::Invalid("a lead run with no lead before it"))?;
                    let k = continuations(l).unwrap_or(0);
                    for _ in 0..(b as usize - 0x7F) {
                        text.push(l);
                        for _ in 0..k {
                            text.push(0x80 | r.get(6)? as u8);
                        }
                    }
                } else if b == DIGITS {
                    let n = r.get(5)? + 2;
                    for _ in 0..n {
                        let d = r.get(4)?;
                        if d > 9 {
                            return Err(CodecError::Invalid("a digit above 9"));
                        }
                        text.push(b'0' + d as u8);
                    }
                } else {
                    return Err(CodecError::Invalid("a reserved escape"));
                }
            }
            REF => {
                push_text(&mut out, &mut text)?;
                out.push(Seg::Ref(r.varint()?));
            }
            CALL => {
                push_text(&mut out, &mut text)?;
                let c = r.varint()?;
                let f = if c < 32 {
                    Callee::Builtin(c as u8)
                } else {
                    Callee::Entry(c - 32)
                };
                let argc = r.varint()?;
                if argc > 1 << 16 {
                    return Err(CodecError::Invalid("too many arguments"));
                }
                let mut args = Vec::with_capacity(argc as usize);
                for _ in 0..argc {
                    let len = r.varint()? as usize;
                    if len > r.left() {
                        return Err(CodecError::Truncated);
                    }
                    let mut inner = BitReader::new(r.bytes, r.pos, r.pos + len);
                    args.push(Arc::new(decode_run(&mut inner, depth + 1)?));
                    if inner.left() != 0 {
                        return Err(CodecError::Invalid("an argument with stray bits"));
                    }
                    r.pos += len;
                }
                out.push(Seg::Call { f, args });
            }
            _ => unreachable!("5-bit token"),
        }
    }
    if r.left() != 0 {
        return Err(CodecError::Truncated);
    }
    push_text(&mut out, &mut text)?;
    Ok(out)
}

/// unpacks a body into its run
pub fn unpack(body: &[u8]) -> Result<Vec<Seg>, CodecError> {
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let pad = (body[0] >> 5) as usize;
    let end = body.len() * 8;
    if pad > end - 3 {
        return Err(CodecError::Truncated);
    }
    let mut r = BitReader::new(body, 3, end - pad);
    decode_run(&mut r, 0)
}

/// the text of a body with no refs or calls in it
pub fn unpack_text(body: &[u8]) -> Result<String, CodecError> {
    let mut out = String::new();
    for s in unpack(body)? {
        match s {
            Seg::Text(t) => out.push_str(&t),
            _ => return Err(CodecError::Invalid("a ref or call in a plain text")),
        }
    }
    Ok(out)
}

/// the run's text with refs and calls left out (for a source view or a quick check)
pub fn plain_parts(segs: &[Seg]) -> String {
    segs.iter()
        .filter_map(|s| match s {
            Seg::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

/// every entry a run refers to or calls, nested arguments included
pub fn mentions(segs: &[Seg], out: &mut Vec<u64>) {
    for s in segs {
        match s {
            Seg::Ref(id) => out.push(*id),
            Seg::Call { f, args } => {
                if let Callee::Entry(id) = f {
                    out.push(*id);
                }
                for a in args {
                    mentions(a, out);
                }
            }
            Seg::Text(_) => {}
        }
    }
}

/// merges adjacent texts and drops empty ones, so equal runs compare equal
pub fn normalize(segs: Vec<Seg>) -> Vec<Seg> {
    let mut out: Vec<Seg> = Vec::with_capacity(segs.len());
    for s in segs {
        match s {
            Seg::Text(t) if t.is_empty() => {}
            Seg::Text(t) => match out.last_mut() {
                Some(Seg::Text(prev)) => prev.push_str(&t),
                _ => out.push(Seg::Text(t)),
            },
            Seg::Call { f, args } => out.push(Seg::Call {
                f,
                args: args
                    .into_iter()
                    .map(|a| Arc::new(normalize(Arc::unwrap_or_clone(a))))
                    .collect(),
            }),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift, so failures reproduce
    pub struct Rng(u64);
    impl Rng {
        pub fn new(seed: u64) -> Rng {
            Rng(seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1)
        }
        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        pub fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    fn round(text: &str) {
        let packed = pack_text(text);
        assert_eq!(unpack_text(&packed).unwrap(), text, "{text:?}");
    }

    #[test]
    fn known_layouts() {
        assert!(pack_text("").is_empty());
        // "a": header 3 bits (pad 0b000 → pad = 8 - 8 = 0)... 3 + 5 = 8 bits, no padding
        assert_eq!(pack_text("a"), vec![0b000_00001]);
        // "ab": 3 + 10 = 13 bits, 3 padding bits
        assert_eq!(pack_text("ab"), vec![0b0110_0001, 0b0001_0000]);
        assert_eq!(pack_text("z "), vec![0b0111_1010, 0b0000_0000]);
        // lowercase prose is 5/8 of its UTF-8 size, plus the header
        assert_eq!(pack_text(&"abcdefgh".repeat(10)).len(), (3 + 400usize).div_ceil(8));
        assert_eq!(ref_bits(0), 10);
        assert_eq!(ref_bits(15), 10);
        assert_eq!(ref_bits(16), 15);
        assert_eq!(ref_bits(65535), 25);
    }

    #[test]
    fn round_trips() {
        for t in [
            "a",
            " ",
            "\n",
            "hello world",
            "Hello World",
            "HELLO WORLD and NASA",
            "McDONALD's",
            "aBcDeFgHiJ",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz",
            "x = 1234567890123456789012345678901234567890 y",
            "12 345 6789 0",
            "# Title\n\n- **bold** _it_ `code` [link](http://x.y/z?q=1&r=2)\n",
            "tabs\tand\r\nreturns",
            "שלום עולם",
            "Λόγος καὶ ἀλήθεια",
            "привет мир Привет",
            "日本語のテキスト、漢字とかな",
            "emoji 🙂🙃 mixed ok 🎉",
            "é è ê ë",
            "\u{0}\u{7f}\u{80}\u{7ff}\u{800}\u{ffff}\u{10000}\u{10ffff}",
        ] {
            round(t);
        }
    }

    #[test]
    fn fuzz_any_utf8() {
        let pools: [&[char]; 4] = [
            &['a', 'e', 't', ' ', 'z', 'q', 'n'],
            &['a', 'B', 'C', ' ', '\n', '1', '9', '.', '#', '*', 'Z'],
            &['ש', 'ל', 'ו', 'ם', ' ', 'λ', 'ό', 'я', '日', '🙂', 'a', '0'],
            &[
                '\u{0}',
                '\u{7f}',
                '\u{80}',
                '\u{7ff}',
                '\u{800}',
                '\u{ffff}',
                '\u{10000}',
                '\u{10ffff}',
                'é',
                'A',
            ],
        ];
        let mut rng = Rng::new(42);
        for case in 0..4000 {
            let len = rng.below(120) as usize;
            let text: String = if case % 5 == 4 {
                // any scalar value at all
                (0..len)
                    .filter_map(|_| char::from_u32(rng.below(0x11_0000) as u32))
                    .collect()
            } else {
                let pool = pools[case % 4];
                (0..len).map(|_| pool[rng.below(pool.len() as u64) as usize]).collect()
            };
            round(&text);
        }
    }

    fn random_run(rng: &mut Rng, depth: usize) -> Vec<Seg> {
        let mut out = Vec::new();
        for _ in 0..rng.below(5) {
            match rng.below(if depth > 3 { 2 } else { 3 }) {
                0 => {
                    let pool = ['a', 'Q', ' ', 'λ', '5', '\n', '日'];
                    out.push(Seg::Text(
                        (0..1 + rng.below(9)).map(|_| pool[rng.below(7) as usize]).collect(),
                    ))
                }
                1 => out.push(Seg::Ref(rng.next() >> rng.below(64))),
                _ => {
                    let f = if rng.below(2) == 0 {
                        Callee::Builtin(rng.below(32) as u8)
                    } else {
                        Callee::Entry(rng.below(1 << 40))
                    };
                    let args = (0..rng.below(4)).map(|_| random_run(rng, depth + 1)).collect();
                    out.push(Seg::call(f, args));
                }
            }
        }
        normalize(out)
    }

    #[test]
    fn fuzz_runs_with_refs_and_calls() {
        let mut rng = Rng::new(7);
        for _ in 0..3000 {
            let run = random_run(&mut rng, 0);
            let packed = pack(&run);
            assert_eq!(unpack(&packed).unwrap(), run);
        }
        // u64::MAX as a ref, the largest entry callee
        let run = vec![Seg::Ref(u64::MAX), Seg::call(Callee::Entry(u64::MAX - 32), vec![])];
        assert_eq!(unpack(&pack(&run)).unwrap(), run);
    }

    #[test]
    fn corrupt_bodies_fail_cleanly() {
        let mut rng = Rng::new(99);
        let mut failures = 0;
        for _ in 0..20_000 {
            let len = rng.below(24) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            // never a panic; errors are fine
            if unpack(&bytes).is_err() {
                failures += 1;
            }
        }
        assert!(failures > 0);
        // a reserved escape
        let mut w = BitWriter::default();
        w.put(0, 3);
        w.put(ESC, 5);
        w.put(0xFF, 8);
        let pad = ((8 - w.bits % 8) % 8) as u8;
        let mut b = w.bytes.clone();
        b[0] |= pad << 5;
        assert!(matches!(unpack(&b), Err(CodecError::Invalid(_))));
        // deep nesting is refused, not a stack overflow
        let mut run = vec![Seg::text("x")];
        for _ in 0..(MAX_NESTING + 2) {
            run = vec![Seg::call(Callee::Builtin(3), vec![run])];
        }
        assert_eq!(unpack(&pack(&run)), Err(CodecError::TooDeep));
    }

    #[test]
    fn compression_is_better_than_utf8() {
        let prose = "the quick brown fox jumps over the lazy dog and keeps running far away";
        let p = pack_text(prose).len() as f64 / prose.len() as f64;
        assert!(p < 0.66, "{p}");
        // 2-byte letters cost 6 bits after the first in a run, spaces 5
        let hebrew = "שלום עולם ומלואו שלום";
        let h = pack_text(hebrew).len() as f64 / hebrew.len() as f64;
        assert!(h < 0.65, "{h}");
    }
}
