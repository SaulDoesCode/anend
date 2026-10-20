//! Posting lists: the derived indexes keep one row per (term, id chunk) whose value is the
//! set of ids under that term, rather than one multimap row per (term, id) pair, because
//! redb's per-row cost dwarfs a 2–3 byte id.
//!
//! **An id set** covers the ids of one chunk (`id >> CHUNK_BITS`), as their low 16 bits:
//! - byte 0 = 0: the offsets in ascending order as LEB128 varints of their differences
//!   (the first from 0);
//! - byte 0 = 1: a 65536-bit bitmap (8192 bytes), used when it's smaller (dense sets).
//!
//! **The content hash index** is keyed by the hash's top 14 bits; its value is a list of
//! `(the next 32 bits of the hash, little endian u32; the id, LEB128)` pairs sorted by
//! check then id. A probe returns every id whose check matches; callers compare texts.

use std::collections::{BTreeMap, HashMap};

use super::record::{get_varint, put_varint};

pub const CHUNK_BITS: u32 = 16;
pub const BITMAP: usize = 8192;
/// the content hash's bucket bits
pub const HASH_BUCKET_BITS: u32 = 14;

pub fn chunk_of(id: u64) -> u32 {
    (id >> CHUNK_BITS) as u32
}

fn low(id: u64) -> u16 {
    id as u16
}

/// the ids of a chunk's set, ascending
pub fn decode_ids(b: &[u8], chunk: u32) -> Vec<u64> {
    let base = (chunk as u64) << CHUNK_BITS;
    match b.first() {
        Some(1) if b.len() == 1 + BITMAP => {
            let mut out = Vec::new();
            for (i, byte) in b[1..].iter().enumerate() {
                let mut bits = *byte;
                while bits != 0 {
                    let j = bits.trailing_zeros() as u64;
                    out.push(base + i as u64 * 8 + j);
                    bits &= bits - 1;
                }
            }
            out
        }
        Some(0) => {
            let mut out = Vec::new();
            let mut at = 1;
            let mut v = 0u64;
            while at < b.len() {
                let Some(d) = get_varint(b, &mut at) else { break };
                v += d;
                if v > 0xFFFF {
                    break;
                }
                out.push(base + v);
            }
            out
        }
        _ => Vec::new(),
    }
}

/// encodes ascending, distinct ids of one chunk
pub fn encode_ids(ids: &[u64]) -> Vec<u8> {
    let mut list = vec![0u8];
    let mut prev = 0u64;
    for &id in ids {
        let o = low(id) as u64;
        put_varint(&mut list, o - prev);
        prev = o;
    }
    if list.len() <= 1 + BITMAP {
        return list;
    }
    let mut bm = vec![0u8; 1 + BITMAP];
    bm[0] = 1;
    for &id in ids {
        let o = low(id) as usize;
        bm[1 + o / 8] |= 1 << (o % 8);
    }
    bm
}

/// adds and removes ids (all of one chunk) in an encoded set; `None` when it ends empty
pub fn update_ids(old: Option<&[u8]>, chunk: u32, add: &[u64], remove: &[u64]) -> Option<Vec<u8>> {
    let mut ids = old.map(|b| decode_ids(b, chunk)).unwrap_or_default();
    if !remove.is_empty() {
        ids.retain(|i| !remove.contains(i));
    }
    for &a in add {
        if let Err(at) = ids.binary_search(&a) {
            ids.insert(at, a);
        }
    }
    (!ids.is_empty()).then(|| encode_ids(&ids))
}

pub fn hash_bucket(hash: u64) -> u64 {
    hash >> (64 - HASH_BUCKET_BITS)
}

pub fn hash_check(hash: u64) -> u32 {
    (hash >> (64 - HASH_BUCKET_BITS - 32)) as u32
}

pub fn decode_pairs(b: &[u8]) -> Vec<(u32, u64)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= b.len() {
        let check = u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        at += 4;
        let Some(id) = get_varint(b, &mut at) else { break };
        out.push((check, id));
    }
    out
}

pub fn encode_pairs(pairs: &[(u32, u64)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pairs.len() * 7);
    for &(c, id) in pairs {
        out.extend_from_slice(&c.to_le_bytes());
        put_varint(&mut out, id);
    }
    out
}

pub fn update_pairs(old: Option<&[u8]>, add: &[(u32, u64)], remove: &[(u32, u64)]) -> Option<Vec<u8>> {
    let mut pairs = old.map(decode_pairs).unwrap_or_default();
    pairs.retain(|p| !remove.contains(p));
    for &p in add {
        if let Err(at) = pairs.binary_search(&p) {
            pairs.insert(at, p);
        }
    }
    (!pairs.is_empty()).then(|| encode_pairs(&pairs))
}

/// the letter frequency term of a frequency key: its hash's top 14 bits, so a row holds
/// a few anagram classes and readers check the key (most texts are alone in their class,
/// and a row per class would be a row per entry)
pub fn freq_term(key: &str) -> Term {
    Term::Freq(super::record::content_hash(key) >> (64 - HASH_BUCKET_BITS))
}

/// what an entry is filed under in the derived indexes
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Term {
    /// a sum mode (`gematria::Mode as u8`) and value
    Sum(u8, i64),
    /// the bucket of a letter frequency key (see `freq_term`)
    Freq(u64),
    /// a word (a token from `simd::tokenize`)
    Word(Vec<u8>),
    /// the content hash of a plain entry's text
    Hash(u64),
    /// the creation time bucket (`created >> TIME_BUCKET_BITS` ms)
    Created(u64),
    /// an entry this one refers to or calls (what editing that entry must look after)
    Mention(u64),
}

/// how many milliseconds of creation time share a bucket (2^22 ms, about 70 minutes)
pub const TIME_BUCKET_BITS: u32 = 22;

/// the derived indexes held in memory (the `memory` index mode)
#[derive(Default)]
pub struct MemIndex {
    pub map: HashMap<Term, Vec<u64>>,
    /// words again, ordered, for prefix search
    pub words: BTreeMap<Vec<u8>, Vec<u64>>,
    /// creation buckets, ordered, for time ranges
    pub created: BTreeMap<u64, Vec<u64>>,
}

fn add_sorted(v: &mut Vec<u64>, id: u64) {
    if let Err(at) = v.binary_search(&id) {
        v.insert(at, id);
    }
}

fn remove_sorted(v: &mut Vec<u64>, id: u64) {
    if let Ok(at) = v.binary_search(&id) {
        v.remove(at);
    }
}

impl MemIndex {
    pub fn add(&mut self, t: &Term, id: u64) {
        match t {
            Term::Word(w) => add_sorted(self.words.entry(w.clone()).or_default(), id),
            Term::Created(b) => add_sorted(self.created.entry(*b).or_default(), id),
            _ => add_sorted(self.map.entry(t.clone()).or_default(), id),
        }
    }

    pub fn remove(&mut self, t: &Term, id: u64) {
        let gone = match t {
            Term::Word(w) => self.words.get_mut(w).map(|v| {
                remove_sorted(v, id);
                v.is_empty()
            }),
            Term::Created(b) => self.created.get_mut(b).map(|v| {
                remove_sorted(v, id);
                v.is_empty()
            }),
            _ => self.map.get_mut(t).map(|v| {
                remove_sorted(v, id);
                v.is_empty()
            }),
        };
        if gone == Some(true) {
            match t {
                Term::Word(w) => {
                    self.words.remove(w);
                }
                Term::Created(b) => {
                    self.created.remove(b);
                }
                _ => {
                    self.map.remove(t);
                }
            }
        }
    }

    pub fn get(&self, t: &Term) -> &[u64] {
        match t {
            Term::Word(w) => self.words.get(w),
            Term::Created(b) => self.created.get(b),
            _ => self.map.get(t),
        }
        .map(Vec::as_slice)
        .unwrap_or(&[])
    }

    /// bytes the index takes, roughly (ids and keys, not the maps' own overhead)
    pub fn approx_bytes(&self) -> usize {
        let ids: usize = self.map.values().map(|v| v.len()).sum::<usize>()
            + self.words.values().map(|v| v.len()).sum::<usize>()
            + self.created.values().map(|v| v.len()).sum::<usize>();
        ids * 8 + self.words.keys().map(|k| k.len() + 24).sum::<usize>() + self.map.len() * 40
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_sets() {
        let ids: Vec<u64> = vec![65536, 65537, 65600, 131071];
        let b = encode_ids(&ids);
        assert_eq!(b[0], 0);
        assert_eq!(decode_ids(&b, 1), ids);
        // dense sets become bitmaps, and stay exact
        let dense: Vec<u64> = (0..60_000).map(|i| i + (i / 7)).filter(|&i| i < 65536).collect();
        let b = encode_ids(&dense);
        assert_eq!(b.len(), 1 + BITMAP);
        assert_eq!(decode_ids(&b, 0), dense);
        let u = update_ids(Some(&b), 0, &[6], &[0, 1]).unwrap();
        let mut want: Vec<u64> = dense.iter().copied().filter(|&i| i > 1).collect();
        want.push(6);
        want.sort();
        want.dedup();
        assert_eq!(decode_ids(&u, 0), want);
        assert_eq!(update_ids(Some(&encode_ids(&[5])), 0, &[], &[5]), None);
        assert_eq!(
            decode_ids(&update_ids(None, 3, &[3 << 16 | 9, 3 << 16 | 2], &[]).unwrap(), 3),
            vec![3 << 16 | 2, 3 << 16 | 9]
        );
        // a small set is about a byte an id
        let b = encode_ids(&(100..200).collect::<Vec<_>>());
        assert_eq!(b.len(), 1 + 1 + 99);
    }

    #[test]
    fn hash_pairs_and_memory() {
        let p = update_pairs(None, &[(5, 300), (1, 2), (5, 1)], &[]).unwrap();
        assert_eq!(decode_pairs(&p), vec![(1, 2), (5, 1), (5, 300)]);
        assert_eq!(
            decode_pairs(&update_pairs(Some(&p), &[], &[(5, 1)]).unwrap()),
            vec![(1, 2), (5, 300)]
        );
        assert_eq!(hash_bucket(u64::MAX), (1 << HASH_BUCKET_BITS) - 1);
        let mut m = MemIndex::default();
        m.add(&Term::Word(b"x".to_vec()), 3);
        m.add(&Term::Word(b"x".to_vec()), 1);
        m.add(&Term::Sum(0, 5), 2);
        assert_eq!(m.get(&Term::Word(b"x".to_vec())), &[1, 3]);
        m.remove(&Term::Word(b"x".to_vec()), 1);
        m.remove(&Term::Word(b"x".to_vec()), 3);
        assert!(m.words.is_empty());
        assert_eq!(m.get(&Term::Sum(0, 5)), &[2]);
    }
}
