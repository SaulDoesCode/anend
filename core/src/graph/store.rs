//! The store: entries in blocks, edges, past versions, and the derived indexes (posting
//! lists on disk, or rebuilt into memory at open). See the module doc for the tables.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::RwLock;

use redb::{
    Database, MultimapTable, MultimapTableDefinition, ReadOnlyMultimapTable, ReadOnlyTable, ReadableDatabase,
    ReadableMultimapTable, ReadableTable, ReadableTableMetadata, Table, TableDefinition, WriteTransaction,
};

use super::codec::{self, Seg};
use super::gematria::{Measure, Mode};
use super::interp::{Interp, Limits, Source};
use super::postings::{self as post, MemIndex, Term};
use super::record::{self as rec, Block, Past, Record, mime};

pub type GResult<T> = Result<T, String>;

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub const ENTRIES: TableDefinition<u64, &[u8]> = TableDefinition::new("graph entries");
pub const EDGES: MultimapTableDefinition<(u64, u16), &[u8]> = MultimapTableDefinition::new("graph edges");
pub const EDGES_IN: MultimapTableDefinition<(u64, u16), u64> = MultimapTableDefinition::new("graph edges in");
pub const HISTORY: TableDefinition<(u64, u64), &[u8]> = TableDefinition::new("graph history");
pub const SUMS: TableDefinition<(u8, i64, u32), &[u8]> = TableDefinition::new("graph sums");
pub const FREQ: TableDefinition<(u64, u32), &[u8]> = TableDefinition::new("graph frequency");
pub const WORDS: TableDefinition<(&[u8], u32), &[u8]> = TableDefinition::new("graph words");
pub const HASHES: TableDefinition<u64, &[u8]> = TableDefinition::new("graph hashes");
pub const CREATED: TableDefinition<(u64, u32), &[u8]> = TableDefinition::new("graph created");
pub const MENTIONS: TableDefinition<(u64, u32), &[u8]> = TableDefinition::new("graph mentions");
pub const FREE: TableDefinition<u64, ()> = TableDefinition::new("graph free ids");
pub const META: TableDefinition<&str, &[u8]> = TableDefinition::new("graph meta");
pub const SETTINGS: TableDefinition<&str, &[u8]> = TableDefinition::new("graph settings");
pub const MIMES: TableDefinition<&str, u16> = TableDefinition::new("graph mimes");
/// records too long for their row (docs/format.md "Long texts"): (id, piece) → a piece
/// of the record's bytes; the row keeps a stub (`record::long_stub`)
pub const LONG: TableDefinition<(u64, u32), &[u8]> = TableDefinition::new("graph long");

/// the piece size for long records, in bytes, unless the save's setting
/// `graph.piece_bytes` says otherwise
pub const PIECE_BYTES: usize = 64 << 10;

/// the limits a save sets for itself (docs/format.md "Long texts", answer 9): each a
/// setting of the save with a default, as (name, default, what it limits). None caps how
/// long a text can be.
pub const LIMITS: &[(&str, u64, &str)] = &[
    (
        "limits.render_output_bytes",
        0,
        "bytes a rendered text with refs or calls may grow to (0: no cap; a program that runs forever is stopped by its step budget)",
    ),
    (
        "graph.piece_bytes",
        PIECE_BYTES as u64,
        "a text stored longer than this goes into pieces of this size (at least 1024)",
    ),
    (
        "limits.borrow_min_bytes",
        12,
        "texts shorter than this aren't tried for borrowing from others (stored whole)",
    ),
    (
        "limits.borrow_max_bytes",
        1 << 20,
        "texts longer than this aren't tried for borrowing (stored whole)",
    ),
    (
        "limits.texts_per_call",
        2000,
        "texts one call reads, spreads or lists at once",
    ),
    (
        "limits.grammar_check_bytes",
        200_000,
        "bytes the spelling check reads in one call",
    ),
];

/// a slot's bytes as a record's: a long record's stub is replaced by its pieces
fn resolve<T: ReadableTable<(u64, u32), &'static [u8]>>(long: &T, id: u64, slot: Vec<u8>) -> GResult<Vec<u8>> {
    let Some((total, pieces)) = rec::long_stub(&slot) else {
        return Ok(slot);
    };
    let mut out = Vec::with_capacity(total);
    for i in 0..pieces {
        let p = long
            .get((id, i))
            .map_err(err)?
            .ok_or_else(|| format!("entry {} is missing piece {i} of {pieces}", rec::label(id)))?;
        out.extend_from_slice(p.value());
    }
    if out.len() != total {
        return Err(format!("entry {}'s pieces don't add up", rec::label(id)));
    }
    Ok(out)
}

/// the names of every table, for stats
pub const TABLE_NAMES: [&str; 23] = [
    "graph entries",
    "graph long",
    "graph edges",
    "graph edges in",
    "graph history",
    "graph sums",
    "graph frequency",
    "graph words",
    "graph hashes",
    "graph created",
    "graph mentions",
    "graph free ids",
    "graph meta",
    "graph settings",
    "graph mimes",
    "graph spec models",
    "graph pictures",
    "graph picture bytes",
    "graph picture lists",
    "graph textures",
    "graph lights",
    "graph blocks",
    "graph block ids",
];

/// the store's layout version, in `graph meta` under "format"
pub const FORMAT: u64 = 1;

/// the relations every store knows; 256 and up are free for others
pub mod rel {
    /// a note entry (from) annotates an entry (to); weum's `notes`/`noted on` pair
    pub const NOTE: u16 = 1;
    /// a list (from, an entry whose text is its name) holds a member (to); the payload is
    /// a u64 big endian order key
    pub const MEMBER: u16 = 2;
    /// an entry (from) carries a tag (to, an entry of mime tag whose text is its name)
    pub const TAG: u16 = 3;
    /// a map (from) places an entry (to); the payload is a packed `place::Place`
    pub const PLACE: u16 = 4;
    /// a classifier (from) is checked by another (to)
    pub const VALIDATES: u16 = 5;

    pub const NAMES: [(u16, &str); 5] = [
        (NOTE, "note"),
        (MEMBER, "member"),
        (TAG, "tag"),
        (PLACE, "place"),
        (VALIDATES, "validates"),
    ];

    pub fn name(r: u16) -> String {
        NAMES
            .iter()
            .find(|(n, _)| *n == r)
            .map(|(_, s)| s.to_string())
            .unwrap_or_else(|| r.to_string())
    }

    pub fn parse(s: &str) -> Option<u16> {
        NAMES
            .iter()
            .find(|(_, n)| *n == s)
            .map(|(r, _)| *r)
            .or_else(|| s.parse().ok())
    }
}

/// ids are kept under 2^48 (posting chunks are `id >> 16` as a u32)
pub const MAX_ID: u64 = (1 << 48) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMode {
    /// posting lists in redb tables
    Disk,
    /// only entries, edges and history on disk; the indexes are rebuilt into memory at open
    Memory,
}

#[derive(Debug, Clone, Copy)]
pub struct BorrowOpts {
    pub enabled: bool,
    /// bytes a borrowed form must save to be used (a borrow also costs a mentions row)
    pub min_saving: usize,
    /// how many borrow hops a source may already have
    pub max_depth: u8,
    /// hash probes per side (prefixes, suffixes) per save
    pub max_probes: usize,
}

impl Default for BorrowOpts {
    fn default() -> BorrowOpts {
        BorrowOpts {
            enabled: true,
            min_saving: 4,
            max_depth: 4,
            max_probes: 128,
        }
    }
}

pub struct Graph {
    db: Database,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    mode: IndexMode,
    mem: RwLock<MemIndex>,
    pub borrow: BorrowOpts,
    pub limits: Limits,
}

/// milliseconds since the Unix epoch from the system clock (0 where there's none, as in a
/// browser without a clock passed in)
pub fn system_ms() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// an edge as (from, relation, to, payload)
pub type EdgeRow = (u64, u16, u64, Vec<u8>);

type HashDelta = (Vec<(u32, u64)>, Vec<(u32, u64)>);

/// a stored entry with its id
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub record: Record,
}

/// what deleting an entry took away, enough to put it back
#[derive(Debug, Clone, PartialEq)]
pub struct Removed {
    pub id: u64,
    pub text: String,
    pub record: Record,
    pub edges_out: Vec<(u16, u64, Vec<u8>)>,
    pub edges_in: Vec<(u16, u64)>,
}

/// one version of an entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// when this version was written (its edited time)
    pub at: u64,
    pub mime: u16,
    pub text: String,
}

fn free_key(id: u64) -> u64 {
    let k = rec::key_of(id);
    if k.len() <= 4 {
        let mut b = [0u8; 4];
        b[..k.len()].copy_from_slice(&k);
        ((u32::from_be_bytes(b) as u64) << 8) | k.len() as u64
    } else {
        (1 << 41) + id
    }
}

fn from_free_key(k: u64) -> Option<u64> {
    if k >= 1 << 41 {
        return Some(k - (1 << 41));
    }
    let len = (k & 0xFF) as usize;
    let b = ((k >> 8) as u32).to_be_bytes();
    rec::id_of(b.get(..len)?)
}

// ---------------------------------------------------------------- posting reads

macro_rules! ids_in {
    ($t:expr, $lo:expr, $hi:expr, $chunk:expr) => {{
        let mut out: Vec<u64> = Vec::new();
        for item in $t.range($lo..=$hi).map_err(err)? {
            let (k, v) = item.map_err(err)?;
            let c = ($chunk)(k.value());
            out.extend(post::decode_ids(v.value(), c));
        }
        Ok(out)
    }};
}

/// the tables a reader reads postings from
struct PostingTables<S, F, W, C, M, H> {
    sums: S,
    freq: F,
    words: W,
    created: C,
    mentions: M,
    hashes: H,
}

impl<S, F, W, C, M, H> PostingTables<S, F, W, C, M, H>
where
    S: ReadableTable<(u8, i64, u32), &'static [u8]>,
    F: ReadableTable<(u64, u32), &'static [u8]>,
    W: ReadableTable<(&'static [u8], u32), &'static [u8]>,
    C: ReadableTable<(u64, u32), &'static [u8]>,
    M: ReadableTable<(u64, u32), &'static [u8]>,
    H: ReadableTable<u64, &'static [u8]>,
{
    fn get(&self, t: &Term) -> GResult<Vec<u64>> {
        match t {
            Term::Sum(m, v) => ids_in!(self.sums, (*m, *v, 0), (*m, *v, u32::MAX), |k: (u8, i64, u32)| k.2),
            Term::Freq(h) => ids_in!(self.freq, (*h, 0), (*h, u32::MAX), |k: (u64, u32)| k.1),
            Term::Word(w) => ids_in!(self.words, (w.as_slice(), 0), (w.as_slice(), u32::MAX), |k: (
                &[u8],
                u32
            )| {
                k.1
            }),
            Term::Created(b) => ids_in!(self.created, (*b, 0), (*b, u32::MAX), |k: (u64, u32)| k.1),
            Term::Mention(x) => ids_in!(self.mentions, (*x, 0), (*x, u32::MAX), |k: (u64, u32)| k.1),
            Term::Hash(h) => {
                let check = post::hash_check(*h);
                let row = self.hashes.get(post::hash_bucket(*h)).map_err(err)?;
                Ok(row
                    .map(|v| post::decode_pairs(v.value()))
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(c, _)| *c == check)
                    .map(|(_, id)| id)
                    .collect())
            }
        }
    }

    /// words starting with `prefix`, with their ids
    fn words_with_prefix(&self, prefix: &[u8], limit: usize) -> GResult<Vec<(Vec<u8>, Vec<u64>)>> {
        let mut out: Vec<(Vec<u8>, Vec<u64>)> = Vec::new();
        for item in self.words.range((prefix, 0u32)..).map_err(err)? {
            let (k, v) = item.map_err(err)?;
            let (w, chunk) = k.value();
            if !w.starts_with(prefix) {
                break;
            }
            let ids = post::decode_ids(v.value(), chunk);
            match out.last_mut() {
                Some((lw, lids)) if lw.as_slice() == w => lids.extend(ids),
                _ => {
                    if out.len() == limit {
                        break;
                    }
                    out.push((w.to_vec(), ids))
                }
            }
        }
        Ok(out)
    }

    /// (bucket, ids) for creation buckets in a range
    fn created_range(&self, lo: u64, hi: u64) -> GResult<Vec<u64>> {
        ids_in!(self.created, (lo, 0), (hi, u32::MAX), |k: (u64, u32)| k.1)
    }

    /// every (value, how many) of a sum mode
    fn sum_counts(&self, mode: u8) -> GResult<Vec<(i64, u64)>> {
        let mut out: Vec<(i64, u64)> = Vec::new();
        for item in self
            .sums
            .range((mode, i64::MIN, 0)..=(mode, i64::MAX, u32::MAX))
            .map_err(err)?
        {
            let (k, v) = item.map_err(err)?;
            let (_, value, chunk) = k.value();
            let n = post::decode_ids(v.value(), chunk).len() as u64;
            match out.last_mut() {
                Some((lv, ln)) if *lv == value => *ln += n,
                _ => out.push((value, n)),
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- reading

/// what both a read view and a write transaction can read; most of the API is here
pub trait Reader {
    fn graph(&self) -> &Graph;
    /// the record bytes of an entry
    fn raw(&self, id: u64) -> GResult<Option<Vec<u8>>>;
    /// the ids under a term
    fn postings(&self, t: &Term) -> GResult<Vec<u64>>;
    /// edges from an entry in a relation: (to, payload), in target order
    fn out_edges(&self, from: u64, r: u16) -> GResult<Vec<(u64, Vec<u8>)>>;
    /// the entries with an edge in a relation to this one
    fn in_edges(&self, to: u64, r: u16) -> GResult<Vec<u64>>;
    /// every relation an entry has edges in, out and in
    fn relations_of(&self, id: u64) -> GResult<(Vec<u16>, Vec<u16>)>;
    fn history_rows(&self, id: u64) -> GResult<Vec<(u64, Vec<u8>)>>;

    fn record(&self, id: u64) -> GResult<Option<Record>> {
        Ok(self.raw(id)?.and_then(|b| Record::decode(&b)))
    }

    fn exists(&self, id: u64) -> GResult<bool> {
        Ok(self.raw(id)?.is_some())
    }

    fn entry(&self, id: u64) -> GResult<Entry> {
        let record = self
            .record(id)?
            .ok_or_else(|| format!("there is no entry {}", rec::label(id)))?;
        Ok(Entry { id, record })
    }

    /// an entry's run as stored (refs and calls unresolved)
    fn source(&self, id: u64) -> GResult<Vec<Seg>> {
        let r = self.entry(id)?.record;
        codec::unpack(&r.body).map_err(err)
    }

    /// an entry's rendered text
    fn render(&self, id: u64) -> GResult<String>
    where
        Self: Sized,
    {
        let r = self.entry(id)?.record;
        if !r.has_refs() {
            return codec::unpack_text(&r.body).map_err(err);
        }
        let src = Src(self);
        Interp::with_limits(&src, self.interp_limits()).render(id).map_err(err)
    }

    /// several entries' rendered texts, sharing one interpreter (and its caches)
    fn render_many(&self, ids: &[u64]) -> Vec<GResult<String>>
    where
        Self: Sized,
    {
        let src = Src(self);
        let mut i = Interp::with_limits(&src, self.interp_limits());
        ids.iter()
            .map(|&id| {
                let r = self.entry(id)?.record;
                if !r.has_refs() {
                    return codec::unpack_text(&r.body).map_err(err);
                }
                i.render(id).map_err(err)
            })
            .collect()
    }

    /// calls a function entry with arguments
    fn call(&self, f: u64, args: &[&str]) -> GResult<String>
    where
        Self: Sized,
    {
        let src = Src(self);
        Interp::with_limits(&src, self.interp_limits())
            .call(f, args)
            .map_err(err)
    }

    /// the entries whose text is exactly this (and of this mime, when given)
    fn find_text(&self, text: &str, mime: Option<u16>) -> GResult<Vec<u64>>
    where
        Self: Sized,
    {
        let mut out = Vec::new();
        for id in self.postings(&Term::Hash(rec::content_hash(text)))? {
            let Some(r) = self.record(id)? else { continue };
            if r.plain() && mime.is_none_or(|m| m == r.mime) && self.render(id)? == text {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// every version of an entry, newest (the current one) first
    fn versions(&self, id: u64) -> GResult<Vec<Version>>
    where
        Self: Sized,
    {
        let current = self.entry(id)?.record;
        let mut text = self.render(id)?;
        let mut out = vec![Version {
            at: current.edited,
            mime: current.mime,
            text: text.clone(),
        }];
        let mut rows = self.history_rows(id)?;
        rows.reverse();
        for (at, bytes) in rows {
            let past = Past::decode(&bytes).ok_or("a past version that doesn't decode")?;
            let (mime, t) = match past {
                Past::Whole(r) => {
                    let t = if r.has_refs() {
                        let src = Src(self);
                        let run = codec::unpack(&r.body).map_err(err)?;
                        Interp::with_limits(&src, self.interp_limits())
                            .eval(&run)
                            .map_err(err)?
                    } else {
                        codec::unpack_text(&r.body).map_err(err)?
                    };
                    (r.mime, t)
                }
                Past::Delta {
                    mime,
                    prefix,
                    suffix,
                    middle,
                } => {
                    let (p, s) = (prefix as usize, suffix as usize);
                    if p + s > text.len() || !text.is_char_boundary(p) || !text.is_char_boundary(text.len() - s) {
                        return Err("a past version that doesn't fit the next one".into());
                    }
                    let mid = codec::unpack_text(&middle).map_err(err)?;
                    (mime, format!("{}{}{}", &text[..p], mid, &text[text.len() - s..]))
                }
            };
            text = t.clone();
            out.push(Version { at, mime, text: t });
        }
        Ok(out)
    }

    /// whether rendering `from` reads `target` (following refs and calls a few levels)
    fn reaches(&self, from: u64, target: u64, depth: u8) -> GResult<bool> {
        if from == target {
            return Ok(true);
        }
        if depth == 0 {
            return Ok(true); // too deep to tell: treat as reaching, so nothing borrows it
        }
        let Some(r) = self.record(from)? else { return Ok(false) };
        if !r.has_refs() {
            return Ok(false);
        }
        let mut m = Vec::new();
        codec::mentions(&codec::unpack(&r.body).map_err(err)?, &mut m);
        for x in m {
            if self.reaches(x, target, depth - 1)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// the members of a list, in order
    fn members(&self, list: u64) -> GResult<Vec<u64>> {
        let mut m: Vec<(u64, u64)> = self
            .out_edges(list, rel::MEMBER)?
            .into_iter()
            .map(|(to, p)| {
                (
                    p.get(..8)
                        .map(|b| u64::from_be_bytes(b.try_into().unwrap()))
                        .unwrap_or(0),
                    to,
                )
            })
            .collect();
        m.sort();
        Ok(m.into_iter().map(|(_, id)| id).collect())
    }

    /// the notes on an entry, oldest first (chronological, by the notes' creation)
    fn notes_on(&self, id: u64) -> GResult<Vec<u64>> {
        let mut notes = Vec::new();
        for n in self.in_edges(id, rel::NOTE)? {
            notes.push((self.record(n)?.map(|r| r.created).unwrap_or(0), n));
        }
        notes.sort();
        Ok(notes.into_iter().map(|(_, n)| n).collect())
    }

    /// the entries a note is noted on
    fn noted_on(&self, note: u64) -> GResult<Vec<u64>> {
        Ok(self.out_edges(note, rel::NOTE)?.into_iter().map(|(to, _)| to).collect())
    }

    fn tags_of(&self, id: u64) -> GResult<Vec<u64>> {
        Ok(self.out_edges(id, rel::TAG)?.into_iter().map(|(to, _)| to).collect())
    }

    /// what a map places, and where
    fn placed_in(&self, map: u64) -> GResult<Vec<(u64, super::place::Place)>> {
        Ok(self
            .out_edges(map, rel::PLACE)?
            .into_iter()
            .filter_map(|(to, p)| super::place::Place::unpack(&p).map(|pl| (to, pl)))
            .collect())
    }

    fn setting(&self, name: &str) -> GResult<Option<Vec<u8>>>;

    /// the interpreter's limits, with the save's cap on a rendered text's size
    fn interp_limits(&self) -> Limits {
        let mut l = self.graph().limits;
        l.output = match self.limit("limits.render_output_bytes") {
            0 => usize::MAX,
            n => n as usize,
        };
        l
    }

    /// a limit's value: the save's setting, else its default in [`LIMITS`]
    fn limit(&self, name: &str) -> u64 {
        self.setting(name)
            .ok()
            .flatten()
            .and_then(|v| std::str::from_utf8(&v).ok()?.trim().parse().ok())
            .or_else(|| LIMITS.iter().find(|l| l.0 == name).map(|l| l.1))
            .unwrap_or(0)
    }
    fn mime_name(&self, id: u16) -> GResult<String>;
}

/// any reader as the interpreter's source
pub struct Src<'a, R: ?Sized>(pub &'a R);

impl<R: Reader + ?Sized> Source for Src<'_, R> {
    fn body(&self, id: u64) -> Result<Option<Vec<Seg>>, String> {
        match self.0.record(id)? {
            None => Ok(None),
            Some(r) => codec::unpack(&r.body).map(Some).map_err(err),
        }
    }
}

type RoPostings = PostingTables<
    ReadOnlyTable<(u8, i64, u32), &'static [u8]>,
    ReadOnlyTable<(u64, u32), &'static [u8]>,
    ReadOnlyTable<(&'static [u8], u32), &'static [u8]>,
    ReadOnlyTable<(u64, u32), &'static [u8]>,
    ReadOnlyTable<(u64, u32), &'static [u8]>,
    ReadOnlyTable<u64, &'static [u8]>,
>;

/// a consistent read of the store
pub struct View<'g> {
    g: &'g Graph,
    entries: ReadOnlyTable<u64, &'static [u8]>,
    long: ReadOnlyTable<(u64, u32), &'static [u8]>,
    edges: ReadOnlyMultimapTable<(u64, u16), &'static [u8]>,
    edges_in: ReadOnlyMultimapTable<(u64, u16), u64>,
    history: ReadOnlyTable<(u64, u64), &'static [u8]>,
    settings: ReadOnlyTable<&'static str, &'static [u8]>,
    mimes: ReadOnlyTable<&'static str, u16>,
    meta: ReadOnlyTable<&'static str, &'static [u8]>,
    free: ReadOnlyTable<u64, ()>,
    postings: RoPostings,
    /// the last few block rows read
    cache: RefCell<HashMap<u64, Option<Vec<u8>>>>,
}

fn read_out_edges<T: ReadableMultimapTable<(u64, u16), &'static [u8]>>(
    t: &T,
    from: u64,
    r: u16,
) -> GResult<Vec<(u64, Vec<u8>)>> {
    let mut out = Vec::new();
    for v in t.get((from, r)).map_err(err)? {
        let v = v.map_err(err)?;
        if let Some((to, p)) = rec::split_edge(v.value()) {
            out.push((to, p.to_vec()));
        }
    }
    Ok(out)
}

fn read_in_edges<T: ReadableMultimapTable<(u64, u16), u64>>(t: &T, to: u64, r: u16) -> GResult<Vec<u64>> {
    let mut out = Vec::new();
    for v in t.get((to, r)).map_err(err)? {
        out.push(v.map_err(err)?.value());
    }
    Ok(out)
}

fn read_relations<A, B>(out_t: &A, in_t: &B, id: u64) -> GResult<(Vec<u16>, Vec<u16>)>
where
    A: ReadableMultimapTable<(u64, u16), &'static [u8]>,
    B: ReadableMultimapTable<(u64, u16), u64>,
{
    let mut outs = Vec::new();
    for item in out_t.range((id, 0)..=(id, u16::MAX)).map_err(err)? {
        let (k, _) = item.map_err(err)?;
        outs.push(k.value().1);
    }
    let mut ins = Vec::new();
    for item in in_t.range((id, 0)..=(id, u16::MAX)).map_err(err)? {
        let (k, _) = item.map_err(err)?;
        ins.push(k.value().1);
    }
    Ok((outs, ins))
}

fn read_history<T: ReadableTable<(u64, u64), &'static [u8]>>(t: &T, id: u64) -> GResult<Vec<(u64, Vec<u8>)>> {
    let mut out = Vec::new();
    for item in t.range((id, 0)..=(id, u64::MAX)).map_err(err)? {
        let (k, v) = item.map_err(err)?;
        out.push((k.value().1, v.value().to_vec()));
    }
    Ok(out)
}

fn mime_name_in<T: ReadableTable<&'static str, u16>>(t: &T, id: u16) -> GResult<String> {
    if let Some(n) = mime::NAMES.get(id as usize) {
        return Ok(n.to_string());
    }
    for item in t.iter().map_err(err)? {
        let (k, v) = item.map_err(err)?;
        if v.value() == id {
            return Ok(k.value().to_string());
        }
    }
    Ok(format!("application/x-anend-{id}"))
}

impl Reader for View<'_> {
    fn graph(&self) -> &Graph {
        self.g
    }

    fn raw(&self, id: u64) -> GResult<Option<Vec<u8>>> {
        let b = rec::block_of(id);
        let mut cache = self.cache.borrow_mut();
        if !cache.contains_key(&b) {
            if cache.len() > 256 {
                cache.clear();
            }
            let row = self.entries.get(b).map_err(err)?.map(|g| g.value().to_vec());
            cache.insert(b, row);
        }
        let slot = cache[&b]
            .as_deref()
            .and_then(|row| rec::block_slot(row, rec::slot_of(id)))
            .map(<[u8]>::to_vec);
        drop(cache);
        slot.map(|s| resolve(&self.long, id, s)).transpose()
    }

    fn postings(&self, t: &Term) -> GResult<Vec<u64>> {
        match self.g.mode {
            IndexMode::Disk => self.postings.get(t),
            IndexMode::Memory => Ok(self.g.mem.read().map_err(err)?.get(t).to_vec()),
        }
    }

    fn out_edges(&self, from: u64, r: u16) -> GResult<Vec<(u64, Vec<u8>)>> {
        read_out_edges(&self.edges, from, r)
    }

    fn in_edges(&self, to: u64, r: u16) -> GResult<Vec<u64>> {
        read_in_edges(&self.edges_in, to, r)
    }

    fn relations_of(&self, id: u64) -> GResult<(Vec<u16>, Vec<u16>)> {
        read_relations(&self.edges, &self.edges_in, id)
    }

    fn history_rows(&self, id: u64) -> GResult<Vec<(u64, Vec<u8>)>> {
        read_history(&self.history, id)
    }

    fn setting(&self, name: &str) -> GResult<Option<Vec<u8>>> {
        Ok(self.settings.get(name).map_err(err)?.map(|g| g.value().to_vec()))
    }

    fn mime_name(&self, id: u16) -> GResult<String> {
        mime_name_in(&self.mimes, id)
    }
}

impl View<'_> {
    /// words beginning with a prefix (any case is the caller's business), up to `limit`
    pub fn words_with_prefix(&self, prefix: &[u8], limit: usize) -> GResult<Vec<(Vec<u8>, Vec<u64>)>> {
        match self.g.mode {
            IndexMode::Disk => self.postings.words_with_prefix(prefix, limit),
            IndexMode::Memory => {
                let m = self.g.mem.read().map_err(err)?;
                Ok(m.words
                    .range(prefix.to_vec()..)
                    .take_while(|(w, _)| w.starts_with(prefix))
                    .take(limit)
                    .map(|(w, ids)| (w.clone(), ids.clone()))
                    .collect())
            }
        }
    }

    /// ids created in a bucket range (callers filter exact times)
    pub fn created_buckets(&self, lo: u64, hi: u64) -> GResult<Vec<u64>> {
        match self.g.mode {
            IndexMode::Disk => self.postings.created_range(lo, hi),
            IndexMode::Memory => {
                let m = self.g.mem.read().map_err(err)?;
                Ok(m.created.range(lo..=hi).flat_map(|(_, v)| v.iter().copied()).collect())
            }
        }
    }

    /// every value of a sum mode with how many entries have it, ascending
    pub fn sum_counts(&self, mode: Mode) -> GResult<Vec<(i64, u64)>> {
        match self.g.mode {
            IndexMode::Disk => self.postings.sum_counts(mode as u8),
            IndexMode::Memory => {
                let m = self.g.mem.read().map_err(err)?;
                let mut out: Vec<(i64, u64)> = m
                    .map
                    .iter()
                    .filter_map(|(t, ids)| match t {
                        Term::Sum(md, v) if *md == mode as u8 => Some((*v, ids.len() as u64)),
                        _ => None,
                    })
                    .collect();
                out.sort();
                Ok(out)
            }
        }
    }

    /// every entry id, ascending
    pub fn ids(&self) -> GResult<Vec<u64>> {
        let mut out = Vec::new();
        for item in self.entries.iter().map_err(err)? {
            let (k, v) = item.map_err(err)?;
            let b = Block::decode(v.value());
            for (i, s) in b.slots.iter().enumerate() {
                if s.is_some() {
                    out.push((k.value() << rec::BLOCK_BITS) + i as u64);
                }
            }
        }
        Ok(out)
    }

    /// every entry with its record, ascending by id
    pub fn all(&self) -> GResult<Vec<Entry>> {
        let mut out = Vec::new();
        for item in self.entries.iter().map_err(err)? {
            let (k, v) = item.map_err(err)?;
            let b = Block::decode(v.value());
            for (i, s) in b.slots.into_iter().enumerate() {
                let id = (k.value() << rec::BLOCK_BITS) + i as u64;
                let Some(s) = s else { continue };
                if let Some(r) = Record::decode(&resolve(&self.long, id, s)?) {
                    out.push(Entry { id, record: r });
                }
            }
        }
        Ok(out)
    }

    /// every edge, as (from, relation, to, payload)
    pub fn all_edges(&self) -> GResult<Vec<EdgeRow>> {
        let mut out = Vec::new();
        for item in self.edges.iter().map_err(err)? {
            let (k, vs) = item.map_err(err)?;
            let (from, r) = k.value();
            for v in vs {
                let v = v.map_err(err)?;
                if let Some((to, p)) = rec::split_edge(v.value()) {
                    out.push((from, r, to, p.to_vec()));
                }
            }
        }
        Ok(out)
    }

    /// every past version row, as (id, time, encoded `Past`)
    pub fn all_history(&self) -> GResult<Vec<(u64, u64, Vec<u8>)>> {
        let mut out = Vec::new();
        for item in self.history.iter().map_err(err)? {
            let (k, v) = item.map_err(err)?;
            let (id, at) = k.value();
            out.push((id, at, v.value().to_vec()));
        }
        Ok(out)
    }

    pub fn all_settings(&self) -> GResult<Vec<(String, Vec<u8>)>> {
        let mut out = Vec::new();
        for item in self.settings.iter().map_err(err)? {
            let (k, v) = item.map_err(err)?;
            out.push((k.value().to_string(), v.value().to_vec()));
        }
        Ok(out)
    }

    pub fn custom_mimes(&self) -> GResult<Vec<(String, u16)>> {
        let mut out = Vec::new();
        for item in self.mimes.iter().map_err(err)? {
            let (k, v) = item.map_err(err)?;
            out.push((k.value().to_string(), v.value()));
        }
        Ok(out)
    }

    /// the last id handed out (or imported)
    pub fn allocated(&self) -> GResult<Option<u64>> {
        Ok(self
            .meta
            .get("allocated")
            .map_err(err)?
            .and_then(|g| g.value().try_into().ok().map(u64::from_be_bytes)))
    }

    /// the forgotten ids waiting to be reused, in the order they'll be handed out
    pub fn free_ids(&self) -> GResult<Vec<u64>> {
        let mut out = Vec::new();
        for item in self.free.iter().map_err(err)? {
            if let Some(id) = from_free_key(item.map_err(err)?.0.value()) {
                out.push(id);
            }
        }
        Ok(out)
    }

    pub fn entry_count(&self) -> GResult<u64> {
        Ok(self.ids()?.len() as u64)
    }

    pub fn edge_count(&self) -> GResult<u64> {
        self.edges.len().map_err(err)
    }

    pub fn history_count(&self) -> GResult<u64> {
        self.history.len().map_err(err)
    }
}

// ---------------------------------------------------------------- writing

type RwPostings<'t> = PostingTables<
    Table<'t, (u8, i64, u32), &'static [u8]>,
    Table<'t, (u64, u32), &'static [u8]>,
    Table<'t, (&'static [u8], u32), &'static [u8]>,
    Table<'t, (u64, u32), &'static [u8]>,
    Table<'t, (u64, u32), &'static [u8]>,
    Table<'t, u64, &'static [u8]>,
>;

/// index changes made in a transaction, not yet written
#[derive(Default)]
pub struct Pending {
    map: BTreeMap<Term, (BTreeSet<u64>, BTreeSet<u64>)>,
}

impl Pending {
    fn add(&mut self, t: Term, id: u64) {
        let e = self.map.entry(t).or_default();
        e.1.remove(&id);
        e.0.insert(id);
    }

    fn remove(&mut self, t: Term, id: u64) {
        let e = self.map.entry(t).or_default();
        e.0.remove(&id);
        e.1.insert(id);
    }

    fn overlay(&self, t: &Term, mut base: Vec<u64>) -> Vec<u64> {
        if let Some((add, rem)) = self.map.get(t) {
            base.retain(|i| !rem.contains(i));
            for a in add {
                if !base.contains(a) {
                    base.push(*a);
                }
            }
            base.sort_unstable();
        }
        base
    }
}

/// a write transaction; everything a write can do is here (and everything a read can,
/// through `Reader`)
pub struct Tx<'t> {
    g: &'t Graph,
    pub now: u64,
    entries: Table<'t, u64, &'static [u8]>,
    long: Table<'t, (u64, u32), &'static [u8]>,
    edges: MultimapTable<'t, (u64, u16), &'static [u8]>,
    edges_in: MultimapTable<'t, (u64, u16), u64>,
    history: Table<'t, (u64, u64), &'static [u8]>,
    free: Table<'t, u64, ()>,
    meta: Table<'t, &'static str, &'static [u8]>,
    settings: Table<'t, &'static str, &'static [u8]>,
    mimes: Table<'t, &'static str, u16>,
    postings: RwPostings<'t>,
    blocks: RefCell<HashMap<u64, Block>>,
    pending: Pending,
    /// borrowing on or off for this transaction (imports turn it off and on)
    pub borrow: BorrowOpts,
}

impl Reader for Tx<'_> {
    fn graph(&self) -> &Graph {
        self.g
    }

    fn raw(&self, id: u64) -> GResult<Option<Vec<u8>>> {
        let b = rec::block_of(id);
        let slot = match self.blocks.borrow().get(&b) {
            Some(block) => block.slots[rec::slot_of(id)].clone(),
            None => {
                let row = self.entries.get(b).map_err(err)?;
                row.and_then(|g| rec::block_slot(g.value(), rec::slot_of(id)).map(<[u8]>::to_vec))
            }
        };
        slot.map(|s| resolve(&self.long, id, s)).transpose()
    }

    fn postings(&self, t: &Term) -> GResult<Vec<u64>> {
        let base = match self.g.mode {
            IndexMode::Disk => self.postings.get(t)?,
            IndexMode::Memory => self.g.mem.read().map_err(err)?.get(t).to_vec(),
        };
        Ok(self.pending.overlay(t, base))
    }

    fn out_edges(&self, from: u64, r: u16) -> GResult<Vec<(u64, Vec<u8>)>> {
        read_out_edges(&self.edges, from, r)
    }

    fn in_edges(&self, to: u64, r: u16) -> GResult<Vec<u64>> {
        read_in_edges(&self.edges_in, to, r)
    }

    fn relations_of(&self, id: u64) -> GResult<(Vec<u16>, Vec<u16>)> {
        read_relations(&self.edges, &self.edges_in, id)
    }

    fn history_rows(&self, id: u64) -> GResult<Vec<(u64, Vec<u8>)>> {
        read_history(&self.history, id)
    }

    fn setting(&self, name: &str) -> GResult<Option<Vec<u8>>> {
        Ok(self.settings.get(name).map_err(err)?.map(|g| g.value().to_vec()))
    }

    fn mime_name(&self, id: u16) -> GResult<String> {
        mime_name_in(&self.mimes, id)
    }
}

/// what an entry is filed under
fn terms_of(r: &Record, rendered: &str) -> GResult<Vec<Term>> {
    let mut out = vec![Term::Created(r.created >> post::TIME_BUCKET_BITS)];
    if r.plain() {
        out.push(Term::Hash(rec::content_hash(rendered)));
    }
    if !rendered.trim().is_empty() {
        let m = Measure::of(rendered);
        for mode in Mode::ALL {
            out.push(Term::Sum(mode as u8, m.get(mode)));
        }
        if !m.frequency.is_empty() {
            out.push(post::freq_term(&m.frequency));
        }
    }
    for tk in crate::simd::unique_tokens(rendered.as_bytes()).into_iter().take(4096) {
        if tk.len() <= 64 {
            out.push(Term::Word(tk.to_vec()));
        }
    }
    if r.has_refs() {
        let mut m = Vec::new();
        codec::mentions(&codec::unpack(&r.body).map_err(err)?, &mut m);
        m.sort_unstable();
        m.dedup();
        out.extend(m.into_iter().map(Term::Mention));
    }
    Ok(out)
}

/// how a text is to be stored
struct Plan {
    body: Vec<u8>,
    flags: u8,
}

impl<'t> Tx<'t> {
    fn open(g: &'t Graph, txn: &'t WriteTransaction) -> GResult<Tx<'t>> {
        Ok(Tx {
            g,
            now: (g.clock)(),
            entries: txn.open_table(ENTRIES).map_err(err)?,
            long: txn.open_table(LONG).map_err(err)?,
            edges: txn.open_multimap_table(EDGES).map_err(err)?,
            edges_in: txn.open_multimap_table(EDGES_IN).map_err(err)?,
            history: txn.open_table(HISTORY).map_err(err)?,
            free: txn.open_table(FREE).map_err(err)?,
            meta: txn.open_table(META).map_err(err)?,
            settings: txn.open_table(SETTINGS).map_err(err)?,
            mimes: txn.open_table(MIMES).map_err(err)?,
            postings: PostingTables {
                sums: txn.open_table(SUMS).map_err(err)?,
                freq: txn.open_table(FREQ).map_err(err)?,
                words: txn.open_table(WORDS).map_err(err)?,
                created: txn.open_table(CREATED).map_err(err)?,
                mentions: txn.open_table(MENTIONS).map_err(err)?,
                hashes: txn.open_table(HASHES).map_err(err)?,
            },
            blocks: RefCell::new(HashMap::new()),
            pending: Pending::default(),
            borrow: g.borrow,
        })
    }

    // ------------------------------------------------------------ low level

    fn block_mut<R>(&self, id: u64, f: impl FnOnce(&mut Option<Vec<u8>>) -> R) -> GResult<R> {
        let b = rec::block_of(id);
        let mut blocks = self.blocks.borrow_mut();
        if let std::collections::hash_map::Entry::Vacant(e) = blocks.entry(b) {
            let row = self.entries.get(b).map_err(err)?.map(|g| Block::decode(g.value()));
            e.insert(row.unwrap_or_default());
        }
        Ok(f(&mut blocks.get_mut(&b).unwrap().slots[rec::slot_of(id)]))
    }

    /// a record into its row's slot; one longer than the piece size goes into `graph long`
    /// in pieces, its slot keeping a stub (so its row's neighbours stay small to read and
    /// write, and no single stored value has to hold the whole text)
    fn write_record(&mut self, id: u64, r: Option<&Record>) -> GResult<()> {
        let bytes = r.map(Record::encode);
        let piece = (self.limit("graph.piece_bytes") as usize).max(1024);
        let old = self.block_mut(id, |slot| slot.clone())?;
        if let Some((_, n)) = old.as_deref().and_then(rec::long_stub) {
            for i in 0..n {
                self.long.remove((id, i)).map_err(err)?;
            }
        }
        let slot = match bytes {
            Some(b) if b.len() > piece => {
                let n = b.len().div_ceil(piece) as u32;
                for (i, c) in b.chunks(piece).enumerate() {
                    self.long.insert((id, i as u32), c).map_err(err)?;
                }
                Some(rec::make_long_stub(b.len(), n))
            }
            b => b,
        };
        self.block_mut(id, |s| *s = slot)
    }

    fn index(&mut self, id: u64, terms: Vec<Term>, add: bool) {
        for t in terms {
            if add {
                self.pending.add(t, id);
            } else {
                self.pending.remove(t, id);
            }
        }
    }

    fn reindex(&mut self, id: u64, old: Vec<Term>, new: Vec<Term>) {
        let olds: HashSet<&Term> = old.iter().collect();
        let news: HashSet<&Term> = new.iter().collect();
        let gone: Vec<Term> = old.iter().filter(|t| !news.contains(t)).cloned().collect();
        let came: Vec<Term> = new.iter().filter(|t| !olds.contains(t)).cloned().collect();
        self.index(id, gone, false);
        self.index(id, came, true);
    }

    fn meta_u64(&self, name: &str) -> GResult<Option<u64>> {
        Ok(self
            .meta
            .get(name)
            .map_err(err)?
            .and_then(|g| g.value().try_into().ok().map(u64::from_be_bytes)))
    }

    fn set_meta_u64(&mut self, name: &str, v: u64) -> GResult<()> {
        self.meta.insert(name, v.to_be_bytes().as_slice()).map_err(err)?;
        Ok(())
    }

    /// the last id handed out (or imported), if any
    pub fn allocated(&self) -> GResult<Option<u64>> {
        self.meta_u64("allocated")
    }

    /// a free id: a forgotten one first (the first in weum's key byte order, as weum's
    /// `reusables` hands them out), else the one after the last allocated
    pub fn alloc_id(&mut self) -> GResult<u64> {
        loop {
            let reused = self.free.pop_first().map_err(err)?.map(|(k, _)| k.value());
            let id = match reused {
                Some(k) => from_free_key(k).ok_or("a bad free id")?,
                None => {
                    let id = match self.allocated()? {
                        Some(a) => a + 1,
                        None => 0,
                    };
                    if id > MAX_ID {
                        return Err("every id is taken".into());
                    }
                    self.set_meta_u64("allocated", id)?;
                    id
                }
            };
            if !self.exists(id)? {
                return Ok(id);
            }
        }
    }

    fn note_allocated(&mut self, id: u64) -> GResult<()> {
        if self.allocated()?.is_none_or(|a| id > a) {
            self.set_meta_u64("allocated", id)?;
        }
        self.free.remove(free_key(id)).map_err(err)?;
        Ok(())
    }

    /// the id of a mime type, registering it when it's new
    pub fn mime_id(&mut self, name: &str) -> GResult<u16> {
        if let Some(m) = mime::builtin(name) {
            return Ok(m);
        }
        if let Some(m) = self.mimes.get(name).map_err(err)? {
            return Ok(m.value());
        }
        let mut next = rec::FIRST_CUSTOM_MIME;
        for item in self.mimes.iter().map_err(err)? {
            next = next.max(item.map_err(err)?.1.value() + 1);
        }
        self.mimes.insert(name, next).map_err(err)?;
        Ok(next)
    }

    pub fn set_setting(&mut self, name: &str, value: &[u8]) -> GResult<()> {
        self.settings.insert(name, value).map_err(err)?;
        Ok(())
    }

    pub fn remove_setting(&mut self, name: &str) -> GResult<()> {
        self.settings.remove(name).map_err(err)?;
        Ok(())
    }

    // ------------------------------------------------------------ borrowing

    /// the plain text of a borrow candidate, if it can be borrowed from
    fn borrowable(&self, id: u64, exclude: u64) -> GResult<Option<(String, u8)>> {
        if id == exclude {
            return Ok(None);
        }
        let Some(r) = self.record(id)? else { return Ok(None) };
        if !r.plain() || r.depth() >= self.borrow.max_depth {
            return Ok(None);
        }
        if r.has_refs() && exclude != u64::MAX && self.reaches(id, exclude, self.borrow.max_depth + 1)? {
            return Ok(None);
        }
        Ok(Some((self.render(id)?, r.depth())))
    }

    fn probe(&self, text: &str, polys: &[u64], a: usize, b: usize, exclude: u64) -> GResult<Option<(u64, u8)>> {
        let h = rec::finish(rec::slice_poly(polys, a, b), b - a);
        for id in self.postings(&Term::Hash(h))? {
            if let Some((t, d)) = self.borrowable(id, exclude)?
                && t == text[a..b]
            {
                return Ok(Some((id, d)));
            }
        }
        Ok(None)
    }

    /// stores `text` as refs to entries it starts or ends with plus the rest, when that's
    /// smaller; never changes what it renders to
    fn plan_text(&self, text: &str, exclude: u64) -> GResult<Plan> {
        let plain = Plan {
            body: codec::pack_text(text),
            flags: rec::PLAIN,
        };
        let o = self.borrow;
        let (lo, hi) = (
            self.limit("limits.borrow_min_bytes") as usize,
            self.limit("limits.borrow_max_bytes") as usize,
        );
        if !o.enabled || text.len() < lo || text.len() > hi {
            return Ok(plain);
        }
        let bytes = text.as_bytes();
        let ws = |b: u8| b == b' ' || b == b'\n';
        let polys = rec::prefix_polys(bytes);
        let n = bytes.len();
        // prefixes end before a space, longest first
        let mut prefix: Option<(usize, u64, u8)> = None;
        let ends: Vec<usize> = (1..n).rev().filter(|&p| ws(bytes[p])).take(o.max_probes).collect();
        for p in ends {
            if let Some((id, d)) = self.probe(text, &polys, 0, p, exclude)? {
                prefix = Some((p, id, d));
                break;
            }
        }
        // suffixes start after a space, longest first, after the prefix
        let from = prefix.map(|p| p.0).unwrap_or(0);
        let mut suffix: Option<(usize, u64, u8)> = None;
        let starts: Vec<usize> = (from + 1..n).filter(|&s| ws(bytes[s - 1])).take(o.max_probes).collect();
        for s in starts {
            if let Some((id, d)) = self.probe(text, &polys, s, n, exclude)? {
                suffix = Some((s, id, d));
                break;
            }
        }
        if prefix.is_none() && suffix.is_none() {
            return Ok(plain);
        }
        let build = |pre: Option<(usize, u64, u8)>, suf: Option<(usize, u64, u8)>| -> Plan {
            let a = pre.map(|p| p.0).unwrap_or(0);
            let b = suf.map(|s| s.0).unwrap_or(n);
            let mut segs = Vec::new();
            if let Some((_, id, _)) = pre {
                segs.push(Seg::Ref(id));
            }
            if a < b {
                segs.push(Seg::Text(text[a..b].to_string()));
            }
            if let Some((_, id, _)) = suf {
                segs.push(Seg::Ref(id));
            }
            let depth = 1 + pre.map(|p| p.2).max(suf.map(|s| s.2)).unwrap_or(0);
            Plan {
                body: codec::pack(&segs),
                flags: rec::PLAIN | rec::HAS_REFS | (depth.min(15) << rec::DEPTH_SHIFT),
            }
        };
        let mut best = plain;
        for (pre, suf) in [(prefix, suffix), (prefix, None), (None, suffix)] {
            if pre.is_none() && suf.is_none() {
                continue;
            }
            let p = build(pre, suf);
            // each source costs a row in the mentions index too
            let rows = pre.is_some() as usize + suf.is_some() as usize;
            if p.body.len() + rows * o.min_saving < best.body.len() {
                best = p;
            }
        }
        Ok(best)
    }

    // ------------------------------------------------------------ entries

    fn check_id(id: u64) -> GResult<()> {
        if id > MAX_ID {
            return Err(format!("id {id} is past the largest ({MAX_ID})"));
        }
        Ok(())
    }

    fn put_new(&mut self, id: u64, r: Record, rendered: &str) -> GResult<()> {
        let terms = terms_of(&r, rendered)?;
        self.write_record(id, Some(&r))?;
        self.index(id, terms, true);
        Ok(())
    }

    /// saves a text as a new entry (or returns the entry that already has exactly this
    /// text and mime); returns its id and whether it's new
    pub fn save_text(&mut self, text: &str, m: u16) -> GResult<(u64, bool)> {
        if let Some(&id) = self.find_text(text, Some(m))?.first() {
            return Ok((id, false));
        }
        let id = self.alloc_id()?;
        let plan = self.plan_text(text, u64::MAX)?;
        let r = Record {
            flags: plan.flags,
            created: self.now,
            edited: self.now,
            mime: m,
            body: plan.body,
        };
        self.put_new(id, r, text)?;
        Ok((id, true))
    }

    /// saves a text under a chosen id with chosen times (imports, restores, undo); an
    /// entry already there is replaced as an edit would (its versions kept)
    pub fn put_text_at(&mut self, id: u64, text: &str, m: u16, created: u64, edited: u64) -> GResult<()> {
        Self::check_id(id)?;
        if self.exists(id)? {
            self.edit_inner(id, Content::Text(text), Some(m), Some(edited))?;
            return Ok(());
        }
        let plan = self.plan_text(text, id)?;
        let r = Record {
            flags: plan.flags,
            created,
            edited: edited.max(created),
            mime: m,
            body: plan.body,
        };
        self.put_new(id, r, text)?;
        self.note_allocated(id)
    }

    /// saves a structured run (transclusions, calls: a template or function) as a new
    /// entry; its refs stay live
    pub fn save_run(&mut self, run: &[Seg], m: u16) -> GResult<u64> {
        let id = self.alloc_id()?;
        self.put_run_at(id, run, m, self.now, self.now)?;
        Ok(id)
    }

    pub fn put_run_at(&mut self, id: u64, run: &[Seg], m: u16, created: u64, edited: u64) -> GResult<()> {
        Self::check_id(id)?;
        if self.exists(id)? {
            return self.edit_inner(id, Content::Run(run), Some(m), Some(edited));
        }
        let r = Record {
            flags: if run.iter().any(|s| !matches!(s, Seg::Text(_))) {
                rec::HAS_REFS
            } else {
                0
            },
            created,
            edited: edited.max(created),
            mime: m,
            body: codec::pack(run),
        };
        self.write_record(id, Some(&r))?;
        let rendered = self.render(id).unwrap_or_default();
        let terms = terms_of(&r, &rendered)?;
        self.index(id, terms, true);
        self.note_allocated(id)
    }

    /// changes an entry's text, keeping the old version
    pub fn edit_text(&mut self, id: u64, text: &str) -> GResult<()> {
        self.edit_inner(id, Content::Text(text), None, None)
    }

    pub fn edit_run(&mut self, id: u64, run: &[Seg]) -> GResult<()> {
        self.edit_inner(id, Content::Run(run), None, None)
    }

    pub fn set_mime(&mut self, id: u64, m: u16) -> GResult<()> {
        let mut r = self.entry(id)?.record;
        r.mime = m;
        self.write_record(id, Some(&r))
    }

    /// the plain mentioners of an entry with their texts as they render now
    fn borrowers(&self, id: u64) -> GResult<Vec<(u64, String)>> {
        let mut out = Vec::new();
        for m in self.postings(&Term::Mention(id))? {
            if m == id {
                continue;
            }
            if let Some(r) = self.record(m)?
                && r.plain()
            {
                out.push((m, self.render(m)?));
            }
        }
        Ok(out)
    }

    /// the structured mentioners of an entry (templates, notes with replacements) with
    /// their texts as they render now
    fn transcluders(&self, id: u64) -> GResult<Vec<(u64, Record, String)>> {
        let mut out = Vec::new();
        for m in self.postings(&Term::Mention(id))? {
            if m == id {
                continue;
            }
            if let Some(r) = self.record(m)?
                && !r.plain()
                && let Ok(t) = self.render(m)
            {
                out.push((m, r, t));
            }
        }
        Ok(out)
    }

    /// indexes the transcluders of a changed entry by what they render now
    fn refresh(&mut self, before: Vec<(u64, Record, String)>) -> GResult<()> {
        for (m, r, old_text) in before {
            let Ok(new_text) = self.render(m) else { continue };
            if new_text == old_text {
                continue;
            }
            let old_terms = terms_of(&r, &old_text)?;
            let new_terms = terms_of(&r, &new_text)?;
            self.reindex(m, old_terms, new_terms);
        }
        Ok(())
    }

    /// stores the borrowers' texts again after a source changed, so they render the same
    fn rehome(&mut self, borrowers: Vec<(u64, String)>) -> GResult<()> {
        for (m, text) in borrowers {
            let Some(old) = self.record(m)? else { continue };
            let plan = self.plan_text(&text, m)?;
            let new = Record {
                flags: plan.flags,
                body: plan.body,
                ..old.clone()
            };
            let old_terms: Vec<Term> = terms_of(&old, &text)?
                .into_iter()
                .filter(|t| matches!(t, Term::Mention(_)))
                .collect();
            let new_terms: Vec<Term> = terms_of(&new, &text)?
                .into_iter()
                .filter(|t| matches!(t, Term::Mention(_)))
                .collect();
            self.write_record(m, Some(&new))?;
            self.reindex(m, old_terms, new_terms);
        }
        Ok(())
    }

    fn edit_inner(&mut self, id: u64, content: Content, m: Option<u16>, at: Option<u64>) -> GResult<()> {
        let old = self.entry(id)?.record;
        let old_text = self.render(id)?;
        let at = at.unwrap_or(self.now).max(old.created);
        let borrowers = self.borrowers(id)?;
        let transcluders = self.transcluders(id)?;
        let new = match content {
            Content::Text(t) => {
                if old.plain() && t == old_text && m.is_none_or(|m| m == old.mime) {
                    return Ok(()); // nothing changed
                }
                let plan = self.plan_text(t, id)?;
                Record {
                    flags: plan.flags,
                    created: old.created,
                    edited: at,
                    mime: m.unwrap_or(old.mime),
                    body: plan.body,
                }
            }
            Content::Run(run) => Record {
                flags: if run.iter().any(|s| !matches!(s, Seg::Text(_))) {
                    rec::HAS_REFS
                } else {
                    0
                },
                created: old.created,
                edited: at,
                mime: m.unwrap_or(old.mime),
                body: codec::pack(run),
            },
        };
        // the version being replaced
        let past = if old.plain() && new.plain() {
            let new_text = match content {
                Content::Text(t) => t,
                Content::Run(_) => unreachable!("runs aren't plain"),
            };
            let (p, s) = rec::common_ends(&old_text, new_text);
            Past::Delta {
                mime: old.mime,
                prefix: p as u64,
                suffix: s as u64,
                middle: codec::pack_text(&old_text[p..old_text.len() - s]),
            }
        } else if old.plain() {
            Past::Whole(Record {
                flags: rec::PLAIN,
                body: codec::pack_text(&old_text),
                ..old.clone()
            })
        } else {
            Past::Whole(old.clone())
        };
        let mut key = old.edited;
        while self.history.get((id, key)).map_err(err)?.is_some() {
            key += 1;
        }
        self.history.insert((id, key), past.encode().as_slice()).map_err(err)?;

        let old_terms = terms_of(&old, &old_text)?;
        self.write_record(id, Some(&new))?;
        let new_text = self.render(id).unwrap_or_default();
        let new_terms = terms_of(&new, &new_text)?;
        self.reindex(id, old_terms, new_terms);
        self.rehome(borrowers)?;
        self.refresh(transcluders)
    }

    /// removes an entry, its edges and its versions; its id is reused later unless a
    /// structured entry still refers to it
    pub fn delete(&mut self, id: u64) -> GResult<Removed> {
        let old = self.entry(id)?.record;
        let text = self.render(id).unwrap_or_default();
        let borrowers = self.borrowers(id)?;
        let terms = terms_of(&old, &text)?;
        self.write_record(id, None)?;
        self.index(id, terms, false);
        self.rehome(borrowers)?;
        // edges both ways
        let (outs, ins) = self.relations_of(id)?;
        let mut edges_out = Vec::new();
        for r in outs {
            for (to, p) in self.out_edges(id, r)? {
                edges_out.push((r, to, p));
            }
        }
        let mut edges_in = Vec::new();
        for r in ins {
            for from in self.in_edges(id, r)? {
                edges_in.push((r, from));
            }
        }
        for (r, to, _) in &edges_out {
            self.unlink(id, *r, *to)?;
        }
        for (r, from) in &edges_in {
            self.unlink(*from, *r, id)?;
        }
        // versions
        let rows: Vec<u64> = self.history_rows(id)?.into_iter().map(|(at, _)| at).collect();
        for at in rows {
            self.history.remove((id, at)).map_err(err)?;
        }
        if self.postings(&Term::Mention(id))?.is_empty() {
            self.free.insert(free_key(id), ()).map_err(err)?;
        }
        Ok(Removed {
            id,
            text,
            record: old,
            edges_out,
            edges_in,
        })
    }

    /// puts a deleted entry back as it was, under its id
    pub fn undelete(&mut self, r: &Removed) -> GResult<()> {
        if self.exists(r.id)? {
            return Err(format!("{} is taken again", rec::label(r.id)));
        }
        let run = codec::unpack(&r.record.body).map_err(err)?;
        if r.record.plain() {
            self.put_text_at(r.id, &r.text, r.record.mime, r.record.created, r.record.edited)?;
        } else {
            self.put_run_at(r.id, &run, r.record.mime, r.record.created, r.record.edited)?;
        }
        for (rl, to, p) in &r.edges_out {
            if self.exists(*to)? {
                self.link(r.id, *rl, *to, p)?;
            }
        }
        for (rl, from) in &r.edges_in {
            if self.exists(*from)? {
                let payload = Vec::new();
                self.link(*from, *rl, r.id, &payload)?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ edges

    pub fn link(&mut self, from: u64, r: u16, to: u64, payload: &[u8]) -> GResult<()> {
        for id in [from, to] {
            if !self.exists(id)? {
                return Err(format!("there is no entry {}", rec::label(id)));
            }
        }
        self.edges
            .insert((from, r), rec::edge_value(to, payload).as_slice())
            .map_err(err)?;
        self.edges_in.insert((to, r), from).map_err(err)?;
        Ok(())
    }

    /// removes every edge from `from` to `to` in a relation; returns how many
    pub fn unlink(&mut self, from: u64, r: u16, to: u64) -> GResult<usize> {
        let values: Vec<Vec<u8>> = self
            .out_edges(from, r)?
            .into_iter()
            .filter(|(t, _)| *t == to)
            .map(|(t, p)| rec::edge_value(t, &p))
            .collect();
        for v in &values {
            self.edges.remove((from, r), v.as_slice()).map_err(err)?;
        }
        self.edges_in.remove((to, r), from).map_err(err)?;
        Ok(values.len())
    }

    /// removes one edge with this payload; the reverse goes when no other edge from
    /// `from` to `to` in the relation is left
    pub fn unlink_exact(&mut self, from: u64, r: u16, to: u64, payload: &[u8]) -> GResult<bool> {
        let gone = self
            .edges
            .remove((from, r), rec::edge_value(to, payload).as_slice())
            .map_err(err)?;
        if !self.out_edges(from, r)?.iter().any(|(t, _)| *t == to) {
            self.edges_in.remove((to, r), from).map_err(err)?;
        }
        Ok(gone)
    }

    /// a note on an entry: saves the note's text (markdown) and links it
    pub fn note(&mut self, on: u64, text: &str) -> GResult<u64> {
        let (n, _) = self.save_text(text, mime::MARKDOWN)?;
        self.link(n, rel::NOTE, on, &[])?;
        Ok(n)
    }

    /// adds a member at the end of a list (nothing when it's there already)
    pub fn add_member(&mut self, list: u64, member: u64) -> GResult<()> {
        let edges = self.out_edges(list, rel::MEMBER)?;
        if edges.iter().any(|(t, _)| *t == member) {
            return Ok(());
        }
        let next = edges
            .iter()
            .filter_map(|(_, p)| p.get(..8).map(|b| u64::from_be_bytes(b.try_into().unwrap())))
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        self.link(list, rel::MEMBER, member, &next.to_be_bytes())
    }

    /// puts a list's members in this order (members not named keep their place after)
    pub fn order_members(&mut self, list: u64, order: &[u64]) -> GResult<()> {
        let mut all = self.members(list)?;
        all.retain(|m| !order.contains(m));
        let ordered: Vec<u64> = order.iter().copied().chain(all).collect();
        for m in &ordered {
            self.unlink(list, rel::MEMBER, *m)?;
        }
        for (i, m) in ordered.iter().enumerate() {
            if self.exists(*m)? {
                self.link(list, rel::MEMBER, *m, &(i as u64).to_be_bytes())?;
            }
        }
        Ok(())
    }

    /// the tag entry of a name (made when it's new)
    pub fn tag_entry(&mut self, name: &str) -> GResult<u64> {
        Ok(self.save_text(name.trim(), mime::TAG)?.0)
    }

    pub fn tag(&mut self, id: u64, name: &str) -> GResult<u64> {
        let t = self.tag_entry(name)?;
        if !self.tags_of(id)?.contains(&t) {
            self.link(id, rel::TAG, t, &[])?;
        }
        Ok(t)
    }

    pub fn place(&mut self, map: u64, id: u64, p: &super::place::Place) -> GResult<()> {
        self.link(map, rel::PLACE, id, &p.pack())
    }

    /// gives an entry a structured body without keeping a version or touching its times
    /// (restores); entries borrowing from it are stored again
    pub fn set_run_quietly(&mut self, id: u64, run: &[Seg]) -> GResult<()> {
        let old = self.entry(id)?.record;
        let old_text = self.render(id)?;
        let borrowers = self.borrowers(id)?;
        let new = Record {
            flags: if run.iter().any(|s| !matches!(s, Seg::Text(_))) {
                rec::HAS_REFS
            } else {
                0
            },
            body: codec::pack(run),
            ..old.clone()
        };
        let old_terms = terms_of(&old, &old_text)?;
        self.write_record(id, Some(&new))?;
        let text = self.render(id).unwrap_or_default();
        let new_terms = terms_of(&new, &text)?;
        self.reindex(id, old_terms, new_terms);
        self.rehome(borrowers)
    }

    /// writes a past version row (restores)
    pub fn put_past(&mut self, id: u64, at: u64, past: &Past) -> GResult<()> {
        self.history.insert((id, at), past.encode().as_slice()).map_err(err)?;
        Ok(())
    }

    /// marks an id forgotten (restores), so it's handed out again
    pub fn put_free(&mut self, id: u64) -> GResult<()> {
        if !self.exists(id)? {
            self.free.insert(free_key(id), ()).map_err(err)?;
        }
        Ok(())
    }

    /// sets the last allocated id (restores)
    pub fn set_allocated(&mut self, id: u64) -> GResult<()> {
        self.set_meta_u64("allocated", id)
    }

    /// the forgotten ids waiting to be reused, in the order they'll be handed out
    pub fn free_ids(&self) -> GResult<Vec<u64>> {
        let mut out = Vec::new();
        for item in self.free.iter().map_err(err)? {
            if let Some(id) = from_free_key(item.map_err(err)?.0.value()) {
                out.push(id);
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------ flushing

    fn flush(mut self) -> GResult<Pending> {
        // blocks
        let blocks = std::mem::take(&mut *self.blocks.borrow_mut());
        for (b, block) in blocks {
            if block.is_empty() {
                self.entries.remove(b).map_err(err)?;
            } else {
                self.entries.insert(b, block.encode().as_slice()).map_err(err)?;
            }
        }
        let pending = std::mem::take(&mut self.pending);
        if self.g.mode == IndexMode::Memory {
            return Ok(pending);
        }
        // postings, a row rewrite per (term, chunk)
        let mut hashes: BTreeMap<u64, HashDelta> = BTreeMap::new();
        for (t, (add, rem)) in &pending.map {
            if let Term::Hash(h) = t {
                let e = hashes.entry(post::hash_bucket(*h)).or_default();
                e.0.extend(add.iter().map(|&id| (post::hash_check(*h), id)));
                e.1.extend(rem.iter().map(|&id| (post::hash_check(*h), id)));
                continue;
            }
            let mut by_chunk: BTreeMap<u32, (Vec<u64>, Vec<u64>)> = BTreeMap::new();
            for &a in add {
                by_chunk.entry(post::chunk_of(a)).or_default().0.push(a);
            }
            for &r in rem {
                by_chunk.entry(post::chunk_of(r)).or_default().1.push(r);
            }
            for (c, (a, r)) in by_chunk {
                macro_rules! rw {
                    ($table:expr, $key:expr) => {{
                        let old = $table.get($key).map_err(err)?.map(|g| g.value().to_vec());
                        match post::update_ids(old.as_deref(), c, &a, &r) {
                            Some(v) => {
                                $table.insert($key, v.as_slice()).map_err(err)?;
                            }
                            None => {
                                $table.remove($key).map_err(err)?;
                            }
                        }
                    }};
                }
                match t {
                    Term::Sum(m, v) => rw!(self.postings.sums, (*m, *v, c)),
                    Term::Freq(h) => rw!(self.postings.freq, (*h, c)),
                    Term::Word(w) => rw!(self.postings.words, (w.as_slice(), c)),
                    Term::Created(b) => rw!(self.postings.created, (*b, c)),
                    Term::Mention(x) => rw!(self.postings.mentions, (*x, c)),
                    Term::Hash(_) => unreachable!(),
                }
            }
        }
        for (bucket, (a, r)) in hashes {
            let old = self
                .postings
                .hashes
                .get(bucket)
                .map_err(err)?
                .map(|g| g.value().to_vec());
            match post::update_pairs(old.as_deref(), &a, &r) {
                Some(v) => {
                    self.postings.hashes.insert(bucket, v.as_slice()).map_err(err)?;
                }
                None => {
                    self.postings.hashes.remove(bucket).map_err(err)?;
                }
            }
        }
        Ok(Pending::default())
    }
}

#[derive(Clone, Copy)]
enum Content<'a> {
    Text(&'a str),
    Run(&'a [Seg]),
}

// ---------------------------------------------------------------- the graph

fn ensure_tables(txn: &WriteTransaction) -> GResult<()> {
    txn.open_table(ENTRIES).map_err(err)?;
    txn.open_table(LONG).map_err(err)?;
    txn.open_multimap_table(EDGES).map_err(err)?;
    txn.open_multimap_table(EDGES_IN).map_err(err)?;
    txn.open_table(HISTORY).map_err(err)?;
    txn.open_table(SUMS).map_err(err)?;
    txn.open_table(FREQ).map_err(err)?;
    txn.open_table(WORDS).map_err(err)?;
    txn.open_table(HASHES).map_err(err)?;
    txn.open_table(CREATED).map_err(err)?;
    txn.open_table(MENTIONS).map_err(err)?;
    txn.open_table(FREE).map_err(err)?;
    txn.open_table(META).map_err(err)?;
    txn.open_table(SETTINGS).map_err(err)?;
    txn.open_table(MIMES).map_err(err)?;
    super::assets::ensure_tables(txn)?;
    super::sounds::ensure_tables(txn)?;
    super::textures::ensure_tables(txn)?;
    super::lights::ensure_tables(txn)?;
    super::scenes::ensure_tables(txn)?;
    Ok(())
}

/// storage use of one table
#[derive(Debug, Clone, Default)]
pub struct TableUse {
    pub name: String,
    pub rows: u64,
    /// key and value bytes
    pub stored: u64,
    /// branch keys and other metadata
    pub metadata: u64,
    pub fragmented: u64,
    pub leaf_pages: u64,
    pub branch_pages: u64,
}

impl TableUse {
    /// bytes of pages the table holds
    pub fn page_bytes(&self) -> u64 {
        (self.leaf_pages + self.branch_pages) * 4096
    }

    /// how full its leaf pages are with keys and values
    pub fn fill(&self) -> f64 {
        if self.leaf_pages == 0 {
            0.0
        } else {
            self.stored as f64 / (self.leaf_pages * 4096) as f64
        }
    }
}

impl Graph {
    /// a store over an open redb database (a file, memory, or any `StorageBackend`)
    pub fn from_db(db: Database) -> GResult<Graph> {
        let txn = db.begin_write().map_err(err)?;
        ensure_tables(&txn)?;
        // saves made before scenes: their blocks move into the room
        super::blocks::migrate(&txn)?;
        let mode = {
            let mut meta = txn.open_table(META).map_err(err)?;
            if meta.get("format").map_err(err)?.is_none() {
                meta.insert("format", FORMAT.to_be_bytes().as_slice()).map_err(err)?;
            }
            let m = meta.get("index").map_err(err)?.map(|g| g.value().to_vec());
            if m.as_deref() == Some(b"memory") {
                IndexMode::Memory
            } else {
                IndexMode::Disk
            }
        };
        txn.commit().map_err(err)?;
        let g = Graph {
            db,
            clock: Box::new(system_ms),
            mode,
            mem: RwLock::new(MemIndex::default()),
            borrow: BorrowOpts::default(),
            limits: Limits {
                fuel: 1_000_000,
                ..Limits::default()
            },
        };
        if mode == IndexMode::Memory {
            g.rebuild_memory()?;
        }
        // saves made before scenes: their saved scenes and maps' placements move into scenes
        g.migrate_maps()?;
        Ok(g)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(path: &std::path::Path) -> GResult<Graph> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        Graph::from_db(Database::create(path).map_err(err)?)
    }

    /// a store held in memory (tests, the browser until a persistent backend is plugged in)
    pub fn in_memory() -> GResult<Graph> {
        let db = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .map_err(err)?;
        Graph::from_db(db)
    }

    /// uses another clock (milliseconds since the Unix epoch): the browser's, or a test's
    pub fn with_clock(mut self, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Graph {
        self.clock = Box::new(clock);
        self
    }

    pub fn now(&self) -> u64 {
        (self.clock)()
    }

    pub fn index_mode(&self) -> IndexMode {
        self.mode
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    /// a consistent read
    pub fn view(&self) -> GResult<View<'_>> {
        let txn = self.db.begin_read().map_err(err)?;
        Ok(View {
            g: self,
            entries: txn.open_table(ENTRIES).map_err(err)?,
            long: txn.open_table(LONG).map_err(err)?,
            edges: txn.open_multimap_table(EDGES).map_err(err)?,
            edges_in: txn.open_multimap_table(EDGES_IN).map_err(err)?,
            history: txn.open_table(HISTORY).map_err(err)?,
            settings: txn.open_table(SETTINGS).map_err(err)?,
            mimes: txn.open_table(MIMES).map_err(err)?,
            meta: txn.open_table(META).map_err(err)?,
            free: txn.open_table(FREE).map_err(err)?,
            postings: PostingTables {
                sums: txn.open_table(SUMS).map_err(err)?,
                freq: txn.open_table(FREQ).map_err(err)?,
                words: txn.open_table(WORDS).map_err(err)?,
                created: txn.open_table(CREATED).map_err(err)?,
                mentions: txn.open_table(MENTIONS).map_err(err)?,
                hashes: txn.open_table(HASHES).map_err(err)?,
            },
            cache: RefCell::new(HashMap::new()),
        })
    }

    /// runs `f` in one durable write transaction
    pub fn write<T>(&self, f: impl FnOnce(&mut Tx) -> GResult<T>) -> GResult<T> {
        self.write_with(redb::Durability::Immediate, f)
    }

    /// a write that's durable only once a later durable one commits (bulk work)
    pub fn write_fast<T>(&self, f: impl FnOnce(&mut Tx) -> GResult<T>) -> GResult<T> {
        self.write_with(redb::Durability::None, f)
    }

    fn write_with<T>(&self, d: redb::Durability, f: impl FnOnce(&mut Tx) -> GResult<T>) -> GResult<T> {
        let mut txn = self.db.begin_write().map_err(err)?;
        txn.set_durability(d).map_err(err)?;
        let (out, pending) = {
            let mut tx = Tx::open(self, &txn)?;
            let out = f(&mut tx)?;
            (out, tx.flush()?)
        };
        txn.commit().map_err(err)?;
        if self.mode == IndexMode::Memory {
            let mut m = self.mem.write().map_err(err)?;
            for (t, (add, rem)) in pending.map {
                for id in rem {
                    m.remove(&t, id);
                }
                for id in add {
                    m.add(&t, id);
                }
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------ shortcuts

    pub fn save_text(&self, text: &str, m: u16) -> GResult<(u64, bool)> {
        self.write(|tx| tx.save_text(text, m))
    }

    pub fn edit_text(&self, id: u64, text: &str) -> GResult<()> {
        self.write(|tx| tx.edit_text(id, text))
    }

    pub fn delete(&self, id: u64) -> GResult<Removed> {
        self.write(|tx| tx.delete(id))
    }

    pub fn render(&self, id: u64) -> GResult<String> {
        self.view()?.render(id)
    }

    pub fn entry(&self, id: u64) -> GResult<Entry> {
        self.view()?.entry(id)
    }

    // ------------------------------------------------------------ indexes

    /// the terms of every entry, from entries alone
    fn all_terms(&self) -> GResult<Vec<(u64, Vec<Term>)>> {
        let v = self.view()?;
        let entries = v.all()?;
        let ids: Vec<u64> = entries.iter().map(|e| e.id).collect();
        let texts = v.render_many(&ids);
        let mut out = Vec::with_capacity(entries.len());
        for (e, t) in entries.iter().zip(texts) {
            out.push((e.id, terms_of(&e.record, &t.unwrap_or_default())?));
        }
        Ok(out)
    }

    fn rebuild_memory(&self) -> GResult<()> {
        let mut m = MemIndex::default();
        for (id, terms) in self.all_terms()? {
            for t in terms {
                m.add(&t, id);
            }
        }
        *self.mem.write().map_err(err)? = m;
        Ok(())
    }

    fn clear_disk_indexes(&self, txn: &WriteTransaction) -> GResult<()> {
        txn.delete_table(SUMS).map_err(err)?;
        txn.delete_table(FREQ).map_err(err)?;
        txn.delete_table(WORDS).map_err(err)?;
        txn.delete_table(HASHES).map_err(err)?;
        txn.delete_table(CREATED).map_err(err)?;
        txn.delete_table(MENTIONS).map_err(err)?;
        ensure_tables(txn)
    }

    /// rebuilds every derived index from the entries (after a format change, or to check)
    pub fn reindex(&self) -> GResult<()> {
        let all = self.all_terms()?;
        match self.mode {
            IndexMode::Memory => self.rebuild_memory(),
            IndexMode::Disk => {
                let txn = self.db.begin_write().map_err(err)?;
                self.clear_disk_indexes(&txn)?;
                {
                    let mut tx = Tx::open(self, &txn)?;
                    for (id, terms) in all {
                        tx.index(id, terms, true);
                    }
                    tx.flush()?;
                }
                txn.commit().map_err(err)
            }
        }
    }

    /// keeps the derived indexes on disk or only in memory (rebuilt at every open)
    pub fn set_index_mode(&mut self, mode: IndexMode) -> GResult<()> {
        if mode == self.mode {
            return Ok(());
        }
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_table(META)
            .map_err(err)?
            .insert(
                "index",
                match mode {
                    IndexMode::Disk => b"disk".as_slice(),
                    IndexMode::Memory => b"memory".as_slice(),
                },
            )
            .map_err(err)?;
        self.clear_disk_indexes(&txn)?;
        txn.commit().map_err(err)?;
        self.mode = mode;
        match mode {
            IndexMode::Memory => self.rebuild_memory(),
            IndexMode::Disk => {
                *self.mem.write().map_err(err)? = MemIndex::default();
                self.reindex()
            }
        }
    }

    /// gives back space freed by deletes and rewrites (after imports, or on demand)
    pub fn compact(&mut self) -> GResult<bool> {
        self.db.compact().map_err(err)
    }

    /// storage use per table
    pub fn table_use(&self) -> GResult<Vec<TableUse>> {
        let txn = self.db.begin_read().map_err(err)?;
        let mut out = Vec::new();
        let push = |out: &mut Vec<TableUse>, name: &str, s: redb::TableStats, rows: u64| {
            out.push(TableUse {
                name: name.to_string(),
                rows,
                stored: s.stored_bytes(),
                metadata: s.metadata_bytes(),
                fragmented: s.fragmented_bytes(),
                leaf_pages: s.leaf_pages(),
                branch_pages: s.branch_pages(),
            })
        };
        macro_rules! plain {
            ($def:expr, $name:expr) => {{
                let t = txn.open_table($def).map_err(err)?;
                push(&mut out, $name, t.stats().map_err(err)?, t.len().map_err(err)?);
            }};
        }
        macro_rules! multi {
            ($def:expr, $name:expr) => {{
                let t = txn.open_multimap_table($def).map_err(err)?;
                push(&mut out, $name, t.stats().map_err(err)?, t.len().map_err(err)?);
            }};
        }
        plain!(ENTRIES, "entries");
        plain!(LONG, "long");
        multi!(EDGES, "edges");
        multi!(EDGES_IN, "edges in");
        plain!(HISTORY, "history");
        plain!(SUMS, "sums");
        plain!(FREQ, "frequency");
        plain!(WORDS, "words");
        plain!(HASHES, "hashes");
        plain!(CREATED, "created");
        plain!(MENTIONS, "mentions");
        plain!(FREE, "free ids");
        plain!(META, "meta");
        plain!(SETTINGS, "settings");
        plain!(MIMES, "mimes");
        plain!(super::assets::SPECS, "spec models");
        plain!(super::assets::PICTURES, "pictures");
        plain!(super::assets::PICTURE_BYTES, "picture bytes");
        multi!(super::assets::PICTURE_LISTS, "picture lists");
        plain!(super::sounds::SOUNDS, "sounds");
        plain!(super::sounds::SOUND_BYTES, "sound bytes");
        plain!(super::textures::TEXTURES, "textures");
        plain!(super::lights::LIGHTS, "lights");
        plain!(super::scenes::SCENES, "scenes");
        multi!(super::scenes::INSTANCES, "scene instances");
        multi!(super::scenes::DETACHED, "scene detached");
        multi!(super::scenes::PLACED, "placed");
        Ok(out)
    }

    /// bytes the in-memory index takes, roughly (0 in disk mode)
    pub fn memory_index_bytes(&self) -> usize {
        self.mem.read().map(|m| m.approx_bytes()).unwrap_or(0)
    }
}
