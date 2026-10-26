//! The bytes an entry, a past version and an edge are stored as; ids and weum keys; the
//! content hash. Pure functions, no database.

/// the record layout version
pub const VERSION: u8 = 1;

/// the body holds only text and refs that borrow (its rendered text never changes unless
/// the entry itself is edited); such entries are in the content hash index
pub const PLAIN: u8 = 1;
/// the body has refs (borrowed parts for a plain entry, transclusions or calls otherwise)
pub const HAS_REFS: u8 = 2;
/// flag bits 4–7: how many borrow hops rendering may take (a hint for borrowing)
pub const DEPTH_SHIFT: u8 = 4;

/// mime types every store knows; others are registered in the `graph mimes` table from
/// `FIRST_CUSTOM_MIME` on
pub mod mime {
    pub const TEXT: u16 = 0;
    pub const MARKDOWN: u16 = 1;
    pub const FN: u16 = 2;
    pub const TAG: u16 = 3;
    pub const LIST: u16 = 4;
    pub const MAP: u16 = 5;
    pub const CLASSIFIER: u16 = 6;
    pub const JSON: u16 = 7;
    /// a chain: a ref to its title, members as ordered `MEMBER` edges (`graph::chains`)
    pub const CHAIN: u16 = 8;
    /// an animation: its text the animation's JSON (`graph::anims`)
    pub const ANIMATION: u16 = 9;
    /// a block's wiring: its text the wiring's JSON (`graph::wiring`)
    pub const WIRING: u16 = 10;

    /// entries that are a block's data rather than texts: left out of search and counts
    pub fn hidden(m: u16) -> bool {
        m == ANIMATION || m == WIRING
    }

    pub const NAMES: [&str; 11] = [
        "text/plain",
        "text/markdown",
        "application/x-anend-fn",
        "application/x-anend-tag",
        "application/x-anend-list",
        "application/x-anend-map",
        "application/x-anend-classifier",
        "application/json",
        "application/x-anend-chain",
        "application/x-anend-animation",
        "application/x-anend-wiring",
    ];

    pub fn builtin(name: &str) -> Option<u16> {
        NAMES.iter().position(|n| *n == name).map(|i| i as u16)
    }
}

pub const FIRST_CUSTOM_MIME: u16 = 64;

// ---------------------------------------------------------------- varints (bytes)

pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

pub fn get_varint(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*at)?;
        *at += 1;
        v |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

// ---------------------------------------------------------------- entries

/// an entry as stored
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub flags: u8,
    /// milliseconds since the Unix epoch
    pub created: u64,
    pub edited: u64,
    pub mime: u16,
    /// the packed run (see `codec`)
    pub body: Vec<u8>,
}

impl Record {
    pub fn plain(&self) -> bool {
        self.flags & PLAIN != 0
    }

    pub fn has_refs(&self) -> bool {
        self.flags & HAS_REFS != 0
    }

    pub fn depth(&self) -> u8 {
        self.flags >> DEPTH_SHIFT
    }

    /// `version, flags, varint created, varint (edited - created), varint mime, body`
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.body.len() + 12);
        out.push(VERSION);
        out.push(self.flags);
        put_varint(&mut out, self.created);
        put_varint(&mut out, self.edited.saturating_sub(self.created));
        put_varint(&mut out, self.mime as u64);
        out.extend_from_slice(&self.body);
        out
    }

    pub fn decode(b: &[u8]) -> Option<Record> {
        if b.len() < 2 || b[0] != VERSION {
            return None;
        }
        let mut at = 2;
        let created = get_varint(b, &mut at)?;
        let edited = created.checked_add(get_varint(b, &mut at)?)?;
        let mime = u16::try_from(get_varint(b, &mut at)?).ok()?;
        Some(Record {
            flags: b[1],
            created,
            edited,
            mime,
            body: b[at..].to_vec(),
        })
    }
}

// ---------------------------------------------------------------- past versions

/// a past version: either whole, or the text of the version after it with a middle part
/// swapped (`prefix` bytes kept from the start, `suffix` from the end)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Past {
    Whole(Record),
    Delta {
        mime: u16,
        prefix: u64,
        suffix: u64,
        /// the packed middle text
        middle: Vec<u8>,
    },
}

impl Past {
    /// byte 0 the version, byte 1 the kind (0 whole: a record follows; 1 delta: varint
    /// mime, prefix and suffix, then the packed middle)
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Past::Whole(r) => [vec![VERSION, 0], r.encode()].concat(),
            Past::Delta {
                mime,
                prefix,
                suffix,
                middle,
            } => {
                let mut out = vec![VERSION, 1];
                put_varint(&mut out, *mime as u64);
                put_varint(&mut out, *prefix);
                put_varint(&mut out, *suffix);
                out.extend_from_slice(middle);
                out
            }
        }
    }

    pub fn decode(b: &[u8]) -> Option<Past> {
        if b.len() < 2 || b[0] != VERSION {
            return None;
        }
        match b[1] {
            0 => Record::decode(&b[2..]).map(Past::Whole),
            1 => {
                let mut at = 2;
                let mime = u16::try_from(get_varint(b, &mut at)?).ok()?;
                let prefix = get_varint(b, &mut at)?;
                let suffix = get_varint(b, &mut at)?;
                Some(Past::Delta {
                    mime,
                    prefix,
                    suffix,
                    middle: b[at..].to_vec(),
                })
            }
            _ => None,
        }
    }
}

/// how many bytes `old` shares with `new` at the start and the end (on char boundaries,
/// not overlapping)
pub fn common_ends(old: &str, new: &str) -> (usize, usize) {
    let mut p = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(p) || !new.is_char_boundary(p) {
        p -= 1;
    }
    let max_s = old.len().min(new.len()) - p;
    let mut s = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(max_s)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - s) || !new.is_char_boundary(new.len() - s) {
        s -= 1;
    }
    (p, s)
}

// ---------------------------------------------------------------- edges

/// a forward edge value: the target id (big endian) then the payload
pub fn edge_value(to: u64, payload: &[u8]) -> Vec<u8> {
    [&to.to_be_bytes()[..], payload].concat()
}

pub fn split_edge(v: &[u8]) -> Option<(u64, &[u8])> {
    let to = u64::from_be_bytes(v.get(..8)?.try_into().ok()?);
    Some((to, &v[8..]))
}

// ---------------------------------------------------------------- ids

/// where each weum key length starts among the ids
const SPANS: [u64; 4] = [0, 256, 256 + 65_536, 256 + 65_536 + 16_777_216];
/// the first id with no weum key
pub const PAST_WEUM: u64 = 256 + 65_536 + 16_777_216 + 4_294_967_296;

/// weum's key for an id: ids count weum's allocation order (`[0]..[255]`, `[0,0]..`,
/// three bytes, four bytes), so id `n + 1` is weum's `next_key` of id `n`. Ids past the
/// four byte keys are their 8 big endian bytes.
pub fn key_of(id: u64) -> Vec<u8> {
    for len in (1..=4).rev() {
        if id >= SPANS[len - 1] && id < PAST_WEUM {
            let v = id - SPANS[len - 1];
            return v.to_be_bytes()[8 - len..].to_vec();
        }
    }
    id.to_be_bytes().to_vec()
}

pub fn id_of(key: &[u8]) -> Option<u64> {
    match key.len() {
        1..=4 => {
            let mut b = [0u8; 8];
            b[8 - key.len()..].copy_from_slice(key);
            Some(SPANS[key.len() - 1] + u64::from_be_bytes(b))
        }
        8 => {
            let id = u64::from_be_bytes(key.try_into().ok()?);
            (id >= PAST_WEUM).then_some(id)
        }
        _ => None,
    }
}

/// how weum shows a key: its bytes, comma separated ("3,14")
pub fn label(id: u64) -> String {
    key_of(id).iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",")
}

pub fn parse_label(s: &str) -> Option<u64> {
    let bytes: Option<Vec<u8>> = s.trim().split(',').map(|p| p.trim().parse::<u8>().ok()).collect();
    id_of(&bytes?)
}

// ---------------------------------------------------------------- hashing

const BASE: u64 = 0x100_0000_01b3;

/// a polynomial hash over bytes, so any prefix or suffix hash comes from prefix hashes
pub fn poly(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0u64, |h, &b| h.wrapping_mul(BASE).wrapping_add(b as u64 + 1))
}

/// the poly hash of every prefix: `p[k]` hashes `bytes[..k]`
pub fn prefix_polys(bytes: &[u8]) -> Vec<u64> {
    let mut out = Vec::with_capacity(bytes.len() + 1);
    let mut h = 0u64;
    out.push(h);
    for &b in bytes {
        h = h.wrapping_mul(BASE).wrapping_add(b as u64 + 1);
        out.push(h);
    }
    out
}

pub fn pow(n: usize) -> u64 {
    let (mut r, mut b, mut e) = (1u64, BASE, n);
    while e > 0 {
        if e & 1 == 1 {
            r = r.wrapping_mul(b);
        }
        b = b.wrapping_mul(b);
        e >>= 1;
    }
    r
}

/// the poly hash of `bytes[a..b]` from the prefix hashes
pub fn slice_poly(prefix: &[u64], a: usize, b: usize) -> u64 {
    prefix[b].wrapping_sub(prefix[a].wrapping_mul(pow(b - a)))
}

/// the content hash the dedup index is keyed by, from a poly hash and a length
pub fn finish(poly: u64, len: usize) -> u64 {
    let mut x = poly ^ (len as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

pub fn content_hash(text: &str) -> u64 {
    finish(poly(text.as_bytes()), text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip() {
        let r = Record {
            flags: PLAIN | HAS_REFS | (3 << DEPTH_SHIFT),
            created: 1_791_000_000_000,
            edited: 1_791_000_000_500,
            mime: 300,
            body: vec![1, 2, 3],
        };
        let b = r.encode();
        assert_eq!(b.len(), 2 + 6 + 2 + 2 + 3);
        assert_eq!(Record::decode(&b), Some(r.clone()));
        assert_eq!(r.depth(), 3);
        assert!(Record::decode(&[2, 0, 0, 0, 0]).is_none());
        assert!(Record::decode(&[1]).is_none());
        let p = Past::Delta {
            mime: 1,
            prefix: 10,
            suffix: 200,
            middle: vec![9, 9],
        };
        assert_eq!(Past::decode(&p.encode()), Some(p));
        let w = Past::Whole(r);
        assert_eq!(Past::decode(&w.encode()), Some(w));
    }

    #[test]
    fn weum_keys() {
        assert_eq!(key_of(0), vec![0]);
        assert_eq!(key_of(255), vec![255]);
        assert_eq!(key_of(256), vec![0, 0]);
        assert_eq!(key_of(256 + 0x0102), vec![1, 2]);
        assert_eq!(key_of(256 + 65_536), vec![0, 0, 0]);
        assert_eq!(key_of(PAST_WEUM - 1), vec![255, 255, 255, 255]);
        assert_eq!(key_of(PAST_WEUM).len(), 8);
        for id in [
            0,
            1,
            255,
            256,
            257,
            65_791,
            65_792,
            70_000,
            16_843_007,
            16_843_008,
            PAST_WEUM - 1,
            PAST_WEUM,
            u64::MAX,
        ] {
            assert_eq!(id_of(&key_of(id)), Some(id), "{id}");
            assert_eq!(parse_label(&label(id)), Some(id));
        }
        assert_eq!(label(256 + 3 * 256 + 14), "3,14");
        assert_eq!(parse_label(" 3, 14 "), Some(256 + 3 * 256 + 14));
        assert_eq!(parse_label("hello"), None);
        assert_eq!(id_of(&[]), None);
        assert_eq!(id_of(&[0; 8]), None, "8 byte keys are only for ids past weum's");
        // id n + 1 is weum's next key after id n
        let next = |b: &[u8]| id_of(b).map(|i| key_of(i + 1));
        assert_eq!(next(&[255]), Some(vec![0, 0]));
        assert_eq!(next(&[0, 255]), Some(vec![1, 0]));
        assert_eq!(next(&[255, 255]), Some(vec![0, 0, 0]));
        assert_eq!(next(&[0, 0, 1, 255]), Some(vec![0, 0, 2, 0]));
    }

    #[test]
    fn hashes_compose() {
        let t = "the quick brown fox";
        let p = prefix_polys(t.as_bytes());
        assert_eq!(p[t.len()], poly(t.as_bytes()));
        assert_eq!(slice_poly(&p, 4, 9), poly(b"quick"));
        assert_eq!(slice_poly(&p, 10, t.len()), poly(b"brown fox"));
        assert_ne!(content_hash("a"), content_hash("a "));
        assert_ne!(content_hash(""), content_hash("\0"));
        assert_eq!(common_ends("hello world", "hello there world"), (6, 5));
        assert_eq!(common_ends("aaa", "aaaa"), (3, 0));
        assert_eq!(common_ends("ab", "ab"), (2, 0));
        assert_eq!(common_ends("xé", "yé"), (0, 2));
        assert_eq!(common_ends("éa", "èa"), (0, 1));
    }
}

// ---------------------------------------------------------------- blocks

/// the first byte of a long record's stub (never a record's version)
pub const LONG_STUB: u8 = 0xF5;

/// a stub for a record kept in `graph long`: its length in bytes and how many pieces
pub fn make_long_stub(total: usize, pieces: u32) -> Vec<u8> {
    let mut out = vec![LONG_STUB];
    put_varint(&mut out, total as u64);
    put_varint(&mut out, pieces as u64);
    out
}

/// a stub's (length, pieces), when a slot holds one
pub fn long_stub(b: &[u8]) -> Option<(usize, u32)> {
    if b.first() != Some(&LONG_STUB) {
        return None;
    }
    let mut at = 1;
    let total = get_varint(b, &mut at)? as usize;
    let pieces = u32::try_from(get_varint(b, &mut at)?).ok()?;
    Some((total, pieces))
}

/// entries are stored 32 to a row: block `id >> BLOCK_BITS`, slot `id & 31`
pub const BLOCK_BITS: u32 = 5;
pub const SLOTS: usize = 1 << BLOCK_BITS;

pub fn block_of(id: u64) -> u64 {
    id >> BLOCK_BITS
}

pub fn slot_of(id: u64) -> usize {
    (id as usize) & (SLOTS - 1)
}

/// a block row: a u32 (little endian) with a bit per occupied slot, a varint length per
/// occupied slot in slot order, then the records in slot order
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Block {
    pub slots: [Option<Vec<u8>>; SLOTS],
}

impl Block {
    pub fn decode(b: &[u8]) -> Block {
        let mut out = Block::default();
        if b.len() < 4 {
            return out;
        }
        let bits = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let mut at = 4;
        let mut lens = Vec::new();
        for i in 0..SLOTS {
            if bits >> i & 1 == 1 {
                let Some(l) = get_varint(b, &mut at) else { return out };
                lens.push((i, l as usize));
            }
        }
        for (i, l) in lens {
            let Some(rec) = b.get(at..at + l) else { return out };
            out.slots[i] = Some(rec.to_vec());
            at += l;
        }
        out
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bits = 0u32;
        let mut out = vec![0; 4];
        for (i, s) in self.slots.iter().enumerate() {
            if let Some(r) = s {
                bits |= 1 << i;
                put_varint(&mut out, r.len() as u64);
            }
        }
        out[..4].copy_from_slice(&bits.to_le_bytes());
        for r in self.slots.iter().flatten() {
            out.extend_from_slice(r);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }
}

/// one slot's record straight from a block row, without decoding the rest
pub fn block_slot(b: &[u8], slot: usize) -> Option<&[u8]> {
    if b.len() < 4 {
        return None;
    }
    let bits = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    if bits >> slot & 1 == 0 {
        return None;
    }
    let mut at = 4;
    let mut skip = 0usize;
    let mut len = 0usize;
    for i in 0..SLOTS {
        if bits >> i & 1 == 1 {
            let l = get_varint(b, &mut at)? as usize;
            if i < slot {
                skip += l;
            } else if i == slot {
                len = l;
            }
        }
    }
    b.get(at + skip..at + skip + len)
}

#[cfg(test)]
mod block_tests {
    use super::*;

    #[test]
    fn blocks() {
        let mut b = Block::default();
        b.slots[0] = Some(vec![1, 2, 3]);
        b.slots[7] = Some(vec![]);
        b.slots[31] = Some(vec![9; 200]);
        let e = b.encode();
        assert_eq!(e.len(), 4 + 1 + 1 + 2 + 3 + 200);
        assert_eq!(Block::decode(&e), b);
        assert_eq!(block_slot(&e, 0), Some(&[1u8, 2, 3][..]));
        assert_eq!(block_slot(&e, 7), Some(&[][..]));
        assert_eq!(block_slot(&e, 31).map(|r| r.len()), Some(200));
        assert_eq!(block_slot(&e, 8), None);
        assert_eq!(block_slot(&e[..10], 31), None);
        assert!(Block::default().is_empty());
        assert_eq!((block_of(70), slot_of(70)), (2, 6));
    }
}
