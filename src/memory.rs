//! weum's memory, as tables in the notes database: memories are short texts under
//! compact byte keys, and both memories and notes are indexed into the search tables.
//!
//! | table | maps |
//! |---|---|
//! | `memories` | memory key -> text |
//! | `memory index` | token -> refs (tokens from the SIMD tokenizer) |
//! | `memg`, `memg reversed`, `memg atbash` | gematria sum -> refs |
//! | `memg frequency` | letter frequency key -> refs |
//! | `lists` | list ref -> member refs |
//! | `state`, `reusables` | the key allocator |
//!
//! A ref is either a memory key (1 to 4 bytes) or `note:` followed by a note's key, so the
//! two never collide. Memory keys come from weum's allocator: one byte keys first, then two,
//! three and four byte ones, reusing the keys of forgotten memories before minting new ones.

use redb::{
    MultimapTableDefinition, ReadableDatabase, ReadableMultimapTable, ReadableTable, TableDefinition, WriteTransaction,
};
use std::collections::{HashMap, HashSet};

use crate::note::Note;
use crate::simd;
use crate::store::{NOTES, Store, StoreResult};
use crate::weum;

pub const MEMORIES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("memories");
const STATE: TableDefinition<&str, &[u8]> = TableDefinition::new("state");
const TO_REUSE: MultimapTableDefinition<&str, &[u8]> = MultimapTableDefinition::new("reusables");
pub const TOKENS: MultimapTableDefinition<&[u8], &[u8]> = MultimapTableDefinition::new("memory index");
pub const ORDINAL: MultimapTableDefinition<u32, &[u8]> = MultimapTableDefinition::new("memg");
pub const REVERSED: MultimapTableDefinition<u32, &[u8]> = MultimapTableDefinition::new("memg reversed");
pub const ATBASH: MultimapTableDefinition<u32, &[u8]> = MultimapTableDefinition::new("memg atbash");
pub const FREQUENCY: MultimapTableDefinition<&[u8], &[u8]> = MultimapTableDefinition::new("memg frequency");
pub const LISTS: MultimapTableDefinition<&[u8], &[u8]> = MultimapTableDefinition::new("lists");
pub const SETTINGS: TableDefinition<&str, &[u8]> = TableDefinition::new("settings");

const ALLOCATED: &str = "allocated";
const NOTE_PREFIX: &[u8] = b"note:";

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ref {
    Memory(Vec<u8>),
    Note(String),
}

impl Ref {
    pub fn bytes(&self) -> Vec<u8> {
        match self {
            Ref::Memory(k) => k.clone(),
            Ref::Note(k) => [NOTE_PREFIX, k.as_bytes()].concat(),
        }
    }

    pub fn from_bytes(b: &[u8]) -> Ref {
        match b.strip_prefix(NOTE_PREFIX) {
            Some(k) if b.len() > 4 => Ref::Note(String::from_utf8_lossy(k).into_owned()),
            _ => Ref::Memory(b.to_vec()),
        }
    }

    /// how weum shows keys: the bytes comma separated ("0,12"); notes as `note:<key>`
    pub fn label(&self) -> String {
        match self {
            Ref::Memory(k) => k.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(","),
            Ref::Note(k) => format!("note:{}", k),
        }
    }

    /// reads a label typed into the term bar, like `JSON.parse('[' + term + ']')`
    pub fn parse(label: &str) -> Option<Ref> {
        let label = label.trim();
        if let Some(k) = label.strip_prefix("note:") {
            return (!k.trim().is_empty()).then(|| Ref::Note(k.trim().to_string()));
        }
        let bytes: Option<Vec<u8>> = label.split(',').map(|p| p.trim().parse::<u8>().ok()).collect();
        bytes.filter(|b| !b.is_empty() && b.len() <= 4).map(Ref::Memory)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub r: Ref,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Bucket {
    #[default]
    Ordinal,
    Reversed,
    Atbash,
    Frequency,
}

impl Bucket {
    pub const ALL: [Bucket; 4] = [Bucket::Ordinal, Bucket::Reversed, Bucket::Atbash, Bucket::Frequency];

    pub fn name(self) -> &'static str {
        match self {
            Bucket::Ordinal => "ordinal",
            Bucket::Reversed => "reversed",
            Bucket::Atbash => "atbash",
            Bucket::Frequency => "frequency",
        }
    }
}

/// what a text is filed under in each gsearch bucket
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measure {
    pub ordinal: u32,
    pub reversed: u32,
    pub atbash: u32,
    pub frequency: String,
}

pub fn measure(text: &str) -> Measure {
    let s = simd::sums(text.as_bytes());
    Measure {
        ordinal: s.ordinal.min(u32::MAX as u64) as u32,
        reversed: s.reversed.min(u32::MAX as u64) as u32,
        atbash: s.atbash.min(u32::MAX as u64) as u32,
        frequency: simd::frequency_key(text.as_bytes()),
    }
}

/// a gsearch key: a sum for the numeric buckets, a frequency key for the letter table
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BucketKey {
    Sum(u32),
    Frequency(String),
}

impl Measure {
    pub fn key(&self, bucket: Bucket) -> BucketKey {
        match bucket {
            Bucket::Ordinal => BucketKey::Sum(self.ordinal),
            Bucket::Reversed => BucketKey::Sum(self.reversed),
            Bucket::Atbash => BucketKey::Sum(self.atbash),
            Bucket::Frequency => BucketKey::Frequency(self.frequency.clone()),
        }
    }
}

/// the texts a note is indexed by: its title for gematria, title and body for tokens
pub fn note_texts(note: &Note) -> (String, String) {
    let mut plain = note.title.clone();
    for block in crate::markdown::parse(&note.markdown) {
        plain.push('\n');
        let spans: Vec<crate::markdown::Span> = match block {
            crate::markdown::Block::Heading { spans, .. }
            | crate::markdown::Block::Paragraph { spans, .. }
            | crate::markdown::Block::Item { spans, .. } => spans,
            crate::markdown::Block::Code(code) => {
                plain.push_str(&code);
                continue;
            }
            crate::markdown::Block::TableRow { cells, .. } => cells.into_iter().flatten().collect(),
            crate::markdown::Block::Rule => continue,
        };
        for s in spans {
            plain.push_str(&s.text);
        }
    }
    (note.title.clone(), plain)
}

/// adds (or with `add` false removes) a ref's entries in every search table
pub(crate) fn index(
    txn: &WriteTransaction,
    r: &Ref,
    gematria_text: &str,
    token_text: &str,
    add: bool,
) -> StoreResult<()> {
    let rb = r.bytes();
    let rb = rb.as_slice();
    {
        let mut tokens = txn.open_multimap_table(TOKENS).map_err(err)?;
        for tk in simd::unique_tokens(token_text.as_bytes()) {
            if add {
                tokens.insert(tk, rb).map_err(err)?;
            } else {
                tokens.remove(tk, rb).map_err(err)?;
            }
        }
    }
    if gematria_text.trim().is_empty() {
        return Ok(());
    }
    let m = measure(gematria_text);
    for (table, sum) in [(ORDINAL, m.ordinal), (REVERSED, m.reversed), (ATBASH, m.atbash)] {
        let mut t = txn.open_multimap_table(table).map_err(err)?;
        if add {
            t.insert(sum, rb).map_err(err)?;
        } else {
            t.remove(sum, rb).map_err(err)?;
        }
    }
    let mut f = txn.open_multimap_table(FREQUENCY).map_err(err)?;
    if add {
        f.insert(m.frequency.as_bytes(), rb).map_err(err)?;
    } else {
        f.remove(m.frequency.as_bytes(), rb).map_err(err)?;
    }
    Ok(())
}

pub(crate) fn index_note(txn: &WriteTransaction, note: &Note, add: bool) -> StoreResult<()> {
    let (gem, tokens) = note_texts(note);
    index(txn, &Ref::Note(note.key.clone()), &gem, &tokens, add)
}

pub(crate) fn create_tables(txn: &WriteTransaction) -> StoreResult<()> {
    txn.open_table(MEMORIES).map_err(err)?;
    txn.open_table(STATE).map_err(err)?;
    txn.open_table(SETTINGS).map_err(err)?;
    {
        let t = TO_REUSE;
        txn.open_multimap_table(t).map_err(err)?;
    }
    for t in [TOKENS, FREQUENCY, LISTS] {
        txn.open_multimap_table(t).map_err(err)?;
    }
    for t in [ORDINAL, REVERSED, ATBASH] {
        txn.open_multimap_table(t).map_err(err)?;
    }
    Ok(())
}

/// the key after `b` in weum's allocation order: 0..=255, then [0,0]..=[255,255], then
/// three bytes, then four
fn next_key(b: &[u8]) -> StoreResult<Vec<u8>> {
    Ok(match b.len() {
        1 => {
            if b[0] == u8::MAX {
                vec![0, 0]
            } else {
                vec![b[0] + 1]
            }
        }
        2 => {
            let at = u16::from_be_bytes([b[0], b[1]]);
            if at == u16::MAX {
                vec![0, 0, 0]
            } else {
                (at + 1).to_be_bytes().to_vec()
            }
        }
        3 => {
            let at = u32::from_be_bytes([0, b[0], b[1], b[2]]);
            if at == 0xFF_FFFF {
                vec![0, 0, 0, 0]
            } else {
                (at + 1).to_be_bytes()[1..].to_vec()
            }
        }
        4 => {
            let at = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            at.checked_add(1)
                .ok_or("every memory key is taken")?
                .to_be_bytes()
                .to_vec()
        }
        _ => return Err("weird sized allocated, not normal".into()),
    })
}

/// allocation order: shorter keys first, then by value
fn key_order(k: &[u8]) -> (usize, Vec<u8>) {
    (k.len(), k.to_vec())
}

impl Store {
    fn hit(&self, txn: &redb::ReadTransaction, r: Ref) -> StoreResult<Option<Hit>> {
        let text = match &r {
            Ref::Memory(k) => {
                let t = txn.open_table(MEMORIES).map_err(err)?;
                let found = t.get(k.as_slice()).map_err(err)?;
                found.map(|g| String::from_utf8_lossy(g.value()).into_owned())
            }
            Ref::Note(k) => {
                let t = txn.open_table(NOTES).map_err(err)?;
                let found = t.get(k.as_str()).map_err(err)?;
                found
                    .and_then(|g| serde_json::from_slice::<Note>(g.value()).ok())
                    .map(|n| n.display_title().to_string())
            }
        };
        Ok(text.map(|text| Hit { r, text }))
    }

    fn hits(&self, txn: &redb::ReadTransaction, refs: Vec<Vec<u8>>) -> StoreResult<Vec<Hit>> {
        let mut out = Vec::new();
        for r in refs {
            if let Some(h) = self.hit(txn, Ref::from_bytes(&r))? {
                out.push(h);
            }
        }
        Ok(out)
    }

    /// stores a text as a new memory under the next free key (weum's `saturate`)
    pub fn saturate_new(&self, text: &str) -> StoreResult<Vec<u8>> {
        let txn = self.db.begin_write().map_err(err)?;
        let key = {
            let mut reuse = txn.open_multimap_table(TO_REUSE).map_err(err)?;
            let mut state = txn.open_table(STATE).map_err(err)?;
            let reusable = reuse
                .get(ALLOCATED)
                .map_err(err)?
                .next()
                .transpose()
                .map_err(err)?
                .map(|g| g.value().to_vec());
            let key = match reusable {
                Some(k) => {
                    reuse.remove(ALLOCATED, k.as_slice()).map_err(err)?;
                    k
                }
                None => {
                    let last = state.get(ALLOCATED).map_err(err)?.map(|g| g.value().to_vec());
                    let k = match last {
                        Some(b) => next_key(&b)?,
                        None => vec![0],
                    };
                    state.insert(ALLOCATED, k.as_slice()).map_err(err)?;
                    k
                }
            };
            txn.open_table(MEMORIES)
                .map_err(err)?
                .insert(key.as_slice(), text.as_bytes())
                .map_err(err)?;
            key
        };
        index(&txn, &Ref::Memory(key.clone()), text, text, true)?;
        txn.commit().map_err(err)?;
        Ok(key)
    }

    /// the key of a memory with exactly this text, if there is one
    pub fn find_exact(&self, text: &str) -> StoreResult<Option<Vec<u8>>> {
        let hits = self.gsearch(Bucket::Ordinal, &BucketKey::Sum(measure(text).ordinal))?;
        Ok(hits.into_iter().find_map(|h| match h.r {
            Ref::Memory(k) if h.text == text => Some(k),
            _ => None,
        }))
    }

    /// saturates a text unless the very same text is already a memory; returns its key
    /// and whether it was new
    pub fn saturate(&self, text: &str) -> StoreResult<(Vec<u8>, bool)> {
        match self.find_exact(text)? {
            Some(k) => Ok((k, false)),
            None => Ok((self.saturate_new(text)?, true)),
        }
    }

    pub fn remember(&self, r: &Ref) -> StoreResult<Hit> {
        let txn = self.db.begin_read().map_err(err)?;
        self.hit(&txn, r.clone())?.ok_or_else(|| "didn't find that".to_string())
    }

    /// removes a memory, freeing its key for reuse; returns its text
    pub fn forget(&self, key: &[u8]) -> StoreResult<String> {
        let txn = self.db.begin_write().map_err(err)?;
        let old = txn
            .open_table(MEMORIES)
            .map_err(err)?
            .remove(key)
            .map_err(err)?
            .map(|g| g.value().to_vec());
        let Some(old) = old else {
            return Err("there was nothing with that name anyways".into());
        };
        let text = String::from_utf8_lossy(&old).into_owned();
        txn.open_multimap_table(TO_REUSE)
            .map_err(err)?
            .insert(ALLOCATED, key)
            .map_err(err)?;
        let r = Ref::Memory(key.to_vec());
        index(&txn, &r, &text, &text, false)?;
        {
            // drop it from every list, and drop the list it named
            let mut lists = txn.open_multimap_table(LISTS).map_err(err)?;
            let mut pairs = Vec::new();
            for item in lists.iter().map_err(err)? {
                let (k, vs) = item.map_err(err)?;
                for v in vs {
                    let v = v.map_err(err)?;
                    if v.value() == key || k.value() == key {
                        pairs.push((k.value().to_vec(), v.value().to_vec()));
                    }
                }
            }
            for (k, v) in pairs {
                lists.remove(k.as_slice(), v.as_slice()).map_err(err)?;
            }
        }
        txn.commit().map_err(err)?;
        Ok(text)
    }

    pub fn memory_count(&self) -> StoreResult<u64> {
        use redb::ReadableTableMetadata;
        let txn = self.db.begin_read().map_err(err)?;
        txn.open_table(MEMORIES).map_err(err)?.len().map_err(err)
    }

    /// weum's token search: refs found under at least three quarters of the query's tokens
    pub fn search(&self, query: &str) -> StoreResult<Vec<Hit>> {
        let tokens = simd::unique_tokens(query.as_bytes());
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let threshold = ((tokens.len() as f32 * 0.75).ceil() as usize).max(1);
        let txn = self.db.begin_read().map_err(err)?;
        let table = txn.open_multimap_table(TOKENS).map_err(err)?;
        let mut scores: HashMap<Vec<u8>, usize> = HashMap::new();
        let mut order: Vec<Vec<u8>> = Vec::new();
        for tk in tokens {
            for v in table.get(tk).map_err(err)? {
                let r = v.map_err(err)?.value().to_vec();
                let n = scores.entry(r.clone()).or_insert(0);
                if *n == 0 {
                    order.push(r);
                }
                *n += 1;
            }
        }
        let refs = order.into_iter().filter(|r| scores[r] >= threshold).collect();
        self.hits(&txn, refs)
    }

    /// the ordinary search of the memory view: every query weum's variation expansion
    /// produces, results merged in the order they turn up
    pub fn search_with_variations(&self, query: &str) -> StoreResult<Vec<Hit>> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for q in weum::search_variations(query.trim()) {
            for h in self.search(&q)? {
                if seen.insert(h.r.clone()) {
                    out.push(h);
                }
            }
        }
        Ok(out)
    }

    /// everything filed under a key in one of the gsearch buckets
    pub fn gsearch(&self, bucket: Bucket, key: &BucketKey) -> StoreResult<Vec<Hit>> {
        let txn = self.db.begin_read().map_err(err)?;
        let mut refs = Vec::new();
        match (bucket, key) {
            (Bucket::Frequency, BucketKey::Frequency(f)) => {
                let t = txn.open_multimap_table(FREQUENCY).map_err(err)?;
                for v in t.get(f.as_bytes()).map_err(err)? {
                    refs.push(v.map_err(err)?.value().to_vec());
                }
            }
            (b, BucketKey::Sum(s)) => {
                let table = match b {
                    Bucket::Reversed => REVERSED,
                    Bucket::Atbash => ATBASH,
                    _ => ORDINAL,
                };
                let t = txn.open_multimap_table(table).map_err(err)?;
                for v in t.get(*s).map_err(err)? {
                    refs.push(v.map_err(err)?.value().to_vec());
                }
            }
            _ => {}
        }
        self.hits(&txn, refs)
    }

    /// how many texts share each of these reversed sums
    pub fn reversed_counts(&self, sums: &[u32]) -> StoreResult<HashMap<u32, u64>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_multimap_table(REVERSED).map_err(err)?;
        let mut out = HashMap::new();
        for &s in sums {
            if let std::collections::hash_map::Entry::Vacant(e) = out.entry(s) {
                e.insert(t.get(s).map_err(err)?.len());
            }
        }
        Ok(out)
    }

    /// keys of the notes whose tokens start with every word of `query` (any case), for
    /// the note list's search bar; prefixes come from range scans of the token table
    pub fn note_filter(&self, query: &str) -> StoreResult<HashSet<String>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_multimap_table(TOKENS).map_err(err)?;
        let mut result: Option<HashSet<String>> = None;
        for word in query.split_whitespace() {
            let mut found = HashSet::new();
            let lower = word.to_lowercase();
            let forms: HashSet<String> = [word.to_string(), lower.clone(), weum_capitalize(&lower)]
                .into_iter()
                .collect();
            for form in forms {
                for item in t.range(form.as_bytes()..).map_err(err)? {
                    let (tk, refs) = item.map_err(err)?;
                    if !tk.value().starts_with(form.as_bytes()) {
                        break;
                    }
                    for r in refs {
                        if let Ref::Note(k) = Ref::from_bytes(r.map_err(err)?.value()) {
                            found.insert(k);
                        }
                    }
                }
            }
            result = Some(match result {
                None => found,
                Some(prev) => prev.intersection(&found).cloned().collect(),
            });
        }
        Ok(result.unwrap_or_default())
    }

    pub fn enlist(&self, list: &Ref, member: &Ref) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_multimap_table(LISTS)
            .map_err(err)?
            .insert(list.bytes().as_slice(), member.bytes().as_slice())
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    pub fn delist(&self, list: &Ref, member: &Ref) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_multimap_table(LISTS)
            .map_err(err)?
            .remove(list.bytes().as_slice(), member.bytes().as_slice())
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    pub fn list(&self, list: &Ref) -> StoreResult<Vec<Hit>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_multimap_table(LISTS).map_err(err)?;
        let mut refs = Vec::new();
        for v in t.get(list.bytes().as_slice()).map_err(err)? {
            refs.push(v.map_err(err)?.value().to_vec());
        }
        self.hits(&txn, refs)
    }

    /// removes a whole list, returning how many members it had
    pub fn unlist(&self, list: &Ref) -> StoreResult<u64> {
        let txn = self.db.begin_write().map_err(err)?;
        let n = txn
            .open_multimap_table(LISTS)
            .map_err(err)?
            .remove_all(list.bytes().as_slice())
            .map_err(err)?
            .len();
        txn.commit().map_err(err)?;
        Ok(n)
    }

    pub fn setting(&self, name: &str) -> StoreResult<Option<Vec<u8>>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(SETTINGS).map_err(err)?;
        let found = t.get(name).map_err(err)?;
        Ok(found.map(|g| g.value().to_vec()))
    }

    pub fn set_setting(&self, name: &str, value: &[u8]) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_table(SETTINGS)
            .map_err(err)?
            .insert(name, value)
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    /// puts a forgotten memory back under its old key (undoing a forget)
    pub fn restore_memory(&self, key: &[u8], text: &str) -> StoreResult<()> {
        if self.remember(&Ref::Memory(key.to_vec())).is_ok() {
            return Err(format!("{} is taken again", Ref::Memory(key.to_vec()).label()));
        }
        self.import_memories(vec![(text.to_string(), key.to_vec())])?;
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_multimap_table(TO_REUSE)
            .map_err(err)?
            .remove(ALLOCATED, key)
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    /// every memory as weum's backup has them, `[text, key]`
    pub fn all_memories(&self) -> StoreResult<Vec<(String, Vec<u8>)>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(MEMORIES).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (k, v) = item.map_err(err)?;
            out.push((String::from_utf8_lossy(v.value()).into_owned(), k.value().to_vec()));
        }
        Ok(out)
    }

    /// the names of the settings starting with `prefix`, without it
    pub fn setting_names(&self, prefix: &str) -> StoreResult<Vec<String>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(SETTINGS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.range(prefix..).map_err(err)? {
            let (k, _) = item.map_err(err)?;
            match k.value().strip_prefix(prefix) {
                Some(name) => out.push(name.to_string()),
                None => break,
            }
        }
        Ok(out)
    }

    /// imports weum's memory backup (`all-of-it.json`: `[[text, [key bytes]], ...]`),
    /// keeping the keys
    pub fn import_memories(&self, entries: Vec<(String, Vec<u8>)>) -> StoreResult<usize> {
        let txn = self.db.begin_write().map_err(err)?;
        let mut highest: Option<Vec<u8>> = None;
        let mut n = 0;
        for (text, key) in entries {
            if key.is_empty() || key.len() > 4 {
                continue;
            }
            let old = txn
                .open_table(MEMORIES)
                .map_err(err)?
                .insert(key.as_slice(), text.as_bytes())
                .map_err(err)?
                .map(|g| g.value().to_vec());
            let r = Ref::Memory(key.clone());
            if let Some(old) = old {
                let old = String::from_utf8_lossy(&old).into_owned();
                index(&txn, &r, &old, &old, false)?;
            }
            index(&txn, &r, &text, &text, true)?;
            if highest.as_ref().is_none_or(|h| key_order(&key) > key_order(h)) {
                highest = Some(key);
            }
            n += 1;
        }
        if let Some(h) = highest {
            let mut state = txn.open_table(STATE).map_err(err)?;
            let current = state.get(ALLOCATED).map_err(err)?.map(|g| g.value().to_vec());
            if current.as_ref().is_none_or(|c| key_order(&h) > key_order(c)) {
                state.insert(ALLOCATED, h.as_slice()).map_err(err)?;
            }
        }
        txn.commit().map_err(err)?;
        Ok(n)
    }
}

fn weum_capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "anend-mem-{}-{:?}.redb",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        (Store::open(&path).unwrap(), path)
    }

    #[test]
    fn allocator_follows_weum() {
        assert_eq!(next_key(&[0]).unwrap(), vec![1]);
        assert_eq!(next_key(&[255]).unwrap(), vec![0, 0]);
        assert_eq!(next_key(&[0, 255]).unwrap(), vec![1, 0]);
        assert_eq!(next_key(&[255, 255]).unwrap(), vec![0, 0, 0]);
        assert_eq!(next_key(&[0, 255, 255]).unwrap(), vec![1, 0, 0]);
        assert_eq!(next_key(&[255, 255, 255]).unwrap(), vec![0, 0, 0, 0]);
        assert_eq!(next_key(&[0, 0, 1, 255]).unwrap(), vec![0, 0, 2, 0]);
        assert!(next_key(&[255, 255, 255, 255]).is_err());

        let (s, path) = temp_store();
        assert_eq!(s.saturate_new("first").unwrap(), vec![0]);
        assert_eq!(s.saturate_new("second").unwrap(), vec![1]);
        assert_eq!(s.forget(&[0]).unwrap(), "first");
        // forgotten keys are reused before new ones are minted
        assert_eq!(s.saturate_new("third").unwrap(), vec![0]);
        assert_eq!(s.saturate_new("fourth").unwrap(), vec![2]);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn saturate_search_gsearch_forget() {
        let (s, path) = temp_store();
        let (k, new) = s.saturate("the quick brown fox").unwrap();
        assert!(new);
        assert_eq!(s.saturate("the quick brown fox").unwrap(), (k.clone(), false));
        s.saturate("brown bread").unwrap();
        s.saturate("listen").unwrap();
        s.saturate("silent").unwrap();

        let texts = |hits: Vec<Hit>| hits.into_iter().map(|h| h.text).collect::<Vec<_>>();
        assert_eq!(texts(s.search("quick fox").unwrap()), vec!["the quick brown fox"]);
        // 3 of 4 tokens must hit
        assert!(s.search("quick fox jumps over").unwrap().is_empty());
        assert_eq!(texts(s.search("brown").unwrap()).len(), 2);

        let m = measure("the quick brown fox");
        assert_eq!(m.ordinal, 211);
        for b in Bucket::ALL {
            assert_eq!(
                texts(s.gsearch(b, &m.key(b)).unwrap()),
                vec!["the quick brown fox"],
                "{:?}",
                b
            );
        }
        let mut anagrams = texts(
            s.gsearch(Bucket::Frequency, &measure("enlist").key(Bucket::Frequency))
                .unwrap(),
        );
        anagrams.sort();
        assert_eq!(anagrams, vec!["listen", "silent"]);

        let counts = s.reversed_counts(&[measure("listen").reversed, 9999]).unwrap();
        assert_eq!(counts[&measure("listen").reversed], 2);
        assert_eq!(counts[&9999], 0);

        s.forget(&k).unwrap();
        assert!(s.search("quick").unwrap().is_empty());
        assert!(s.gsearch(Bucket::Ordinal, &BucketKey::Sum(211)).unwrap().is_empty());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn variations_and_notes() {
        let (s, path) = temp_store();
        s.saturate("Rusty gears turn").unwrap();
        // "rust" finds "Rusty" through the variations (capitalised + "y")
        assert_eq!(s.search_with_variations("rust").unwrap()[0].text, "Rusty gears turn");

        let note = s
            .save(&Note {
                title: "Gardening plans".into(),
                markdown: "plant **tomatoes** soon".into(),
                ..Default::default()
            })
            .unwrap();
        let hits = s.search("tomatoes").unwrap();
        assert_eq!(
            hits,
            vec![Hit {
                r: Ref::Note(note.key.clone()),
                text: "Gardening plans".into()
            }]
        );
        assert!(s.note_filter("gard tom").unwrap().contains(&note.key));
        assert!(s.note_filter("gard potato").unwrap().is_empty());
        let m = measure("Gardening plans");
        assert_eq!(s.gsearch(Bucket::Reversed, &m.key(Bucket::Reversed)).unwrap().len(), 1);

        // editing re-indexes, deleting unindexes
        let mut edited = note.clone();
        edited.markdown = "plant potatoes".into();
        s.save(&edited).unwrap();
        assert!(s.search("tomatoes").unwrap().is_empty());
        assert_eq!(s.search("potatoes").unwrap().len(), 1);
        s.delete(&note.key).unwrap();
        assert!(s.search("potatoes").unwrap().is_empty());
        assert!(
            s.gsearch(Bucket::Reversed, &m.key(Bucket::Reversed))
                .unwrap()
                .is_empty()
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn lists_refs_and_import() {
        let (s, path) = temp_store();
        let a = Ref::Memory(s.saturate_new("list name").unwrap());
        let b = Ref::Memory(s.saturate_new("member").unwrap());
        s.enlist(&a, &b).unwrap();
        assert_eq!(s.list(&a).unwrap()[0].text, "member");
        s.delist(&a, &b).unwrap();
        assert!(s.list(&a).unwrap().is_empty());
        s.enlist(&a, &b).unwrap();
        assert_eq!(s.unlist(&a).unwrap(), 1);

        assert_eq!(Ref::parse("0, 12").unwrap(), Ref::Memory(vec![0, 12]));
        assert_eq!(Ref::parse("note:1000001").unwrap(), Ref::Note("1000001".into()));
        assert!(Ref::parse("hello").is_none());
        assert_eq!(Ref::from_bytes(&Ref::Note("7".into()).bytes()), Ref::Note("7".into()));
        assert_eq!(Ref::Memory(vec![3, 4]).label(), "3,4");

        let n = s
            .import_memories(vec![("old saying".into(), vec![0, 9]), ("another".into(), vec![5])])
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(s.remember(&Ref::Memory(vec![0, 9])).unwrap().text, "old saying");
        // new keys continue after the highest imported one
        assert_eq!(s.saturate_new("fresh").unwrap(), vec![0, 10]);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn restore_atbash_and_maps() {
        let (s, path) = temp_store();
        let k = s.saturate_new("שלום light").unwrap();
        // Latin a=26 .. z=1, Hebrew א=22 .. ת=1
        let atbash = 79 + 2 + 11 + 17;
        assert_eq!(measure("שלום light").atbash, atbash);
        assert_eq!(s.gsearch(Bucket::Atbash, &BucketKey::Sum(atbash)).unwrap().len(), 1);

        // a forget undone gets its key back, and the key isn't handed out again
        let text = s.forget(&k).unwrap();
        s.restore_memory(&k, &text).unwrap();
        assert_eq!(s.remember(&Ref::Memory(k.clone())).unwrap().text, "שלום light");
        assert_ne!(s.saturate_new("other").unwrap(), k);
        assert!(s.restore_memory(&k, "again").is_err());

        s.set_setting("map:one", b"[]").unwrap();
        s.set_setting("map:two", b"[]").unwrap();
        s.set_setting("mapz", b"x").unwrap();
        assert_eq!(s.setting_names("map:").unwrap(), vec!["one", "two"]);
        assert_eq!(s.all_memories().unwrap().len(), 2);
        std::fs::remove_file(path).ok();
    }
}
