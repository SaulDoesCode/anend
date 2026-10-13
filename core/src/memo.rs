//! Classifier buckets: each classifier's steered outputs on the save's memories and notes,
//! memoized, so searching by a class, filtering on classifiers, colouring texts by class
//! and finding near texts don't run the network again. A fifth bucket family beside the
//! ordinal, reversed, atbash and frequency ones: a text is filed under its classifier's
//! class and how sure it was.
//!
//! | table | maps |
//! |---|---|
//! | `classifier memo` | (classifier, ref) -> a `Row`: stamp, text hash, probabilities, top class, members, embedding |
//! | `classifier buckets` | (classifier, class·16 + band) -> refs |
//! | `classifier memo stamps` | classifier -> the stamp its rows were made under |
//! | `classifier indexing` | ref -> the classifiers chosen to index it (JSON names) |
//!
//! A **stamp** is a hash of everything the outputs depend on: the classifier's and its
//! upstream classifiers' weight versions and steering. Training, feedback, a steering
//! change, or a retrain upstream give a new stamp, and the first use under it drops the
//! old rows. A row also keeps a hash of the text it read, so an edited note (or a
//! forgotten memory's key given to a new text) is read again. Rows are filled lazily
//! (whatever a call needs and doesn't have) or ahead of time by `Store::memo_index`, in
//! batches on the engine's pool. Everything is deterministic: the same weights, steering
//! and text give the same bytes on any thread count.
//!
//! **Buckets:** a text is filed under each class it's a member of (the steered threshold:
//! for a one-class head its top class when sure enough), with its probability's band
//! (0 for 0–10 %, …, 9 for 90–100 %); a text in no class is filed under `UNCERTAIN` by
//! its top probability. `Store::class_bucket` reads a bucket back.

use redb::{MultimapTableDefinition, ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::classifiers::Stack;
use crate::classify::math::fnv;
use crate::classify::{Flow, Pool};
use crate::memory::Ref;
use crate::store::{Store, StoreResult};

pub const MEMO: TableDefinition<(&str, &[u8]), &[u8]> = TableDefinition::new("classifier memo");
pub const BUCKETS: MultimapTableDefinition<(&str, u32), &[u8]> = MultimapTableDefinition::new("classifier buckets");
pub const STAMPS: TableDefinition<&str, u64> = TableDefinition::new("classifier memo stamps");
pub const INDEXED: TableDefinition<&[u8], &[u8]> = TableDefinition::new("classifier indexing");

/// the class number texts in no class are filed under
pub const UNCERTAIN: u32 = 4095;
/// probability bands per class
pub const BANDS: u32 = 10;
/// bumped when what a row holds changes
const FORMAT: u64 = 1;
/// rows written per transaction while indexing
pub const BATCH: usize = 96;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn create_tables(txn: &WriteTransaction) -> StoreResult<()> {
    txn.open_table(MEMO).map_err(err)?;
    txn.open_multimap_table(BUCKETS).map_err(err)?;
    txn.open_table(STAMPS).map_err(err)?;
    txn.open_table(INDEXED).map_err(err)?;
    Ok(())
}

/// the band a probability falls in
pub fn band(p: f32) -> u32 {
    if p.is_nan() || p <= 0.0 {
        0
    } else {
        ((p * BANDS as f32) as u32).min(BANDS - 1)
    }
}

/// a bucket's key: class (or `UNCERTAIN`) and band
pub fn bucket_key(class: u32, band: u32) -> u32 {
    class * 16 + band.min(15)
}

/// a text's hash as rows keep it
pub fn text_hash(text: &str) -> u64 {
    fnv(0x6d656d6f, text.as_bytes())
}

/// one classifier's memoized reading of one text
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub stamp: u64,
    pub text_hash: u64,
    /// steered probabilities, one per class
    pub probs: Vec<f32>,
    pub top: usize,
    /// the classes it's in under the steered threshold
    pub members: Vec<usize>,
    /// the pooled vector (`Classifier::represent`)
    pub embedding: Vec<f32>,
}

impl Row {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(28 + 4 * (self.probs.len() + self.embedding.len()) + 2 * self.members.len());
        out.extend_from_slice(&self.stamp.to_le_bytes());
        out.extend_from_slice(&self.text_hash.to_le_bytes());
        out.extend_from_slice(&(self.probs.len() as u16).to_le_bytes());
        for p in &self.probs {
            out.extend_from_slice(&p.to_le_bytes());
        }
        out.extend_from_slice(&(self.top as u16).to_le_bytes());
        out.extend_from_slice(&(self.members.len() as u16).to_le_bytes());
        for m in &self.members {
            out.extend_from_slice(&(*m as u16).to_le_bytes());
        }
        out.extend_from_slice(&(self.embedding.len() as u32).to_le_bytes());
        for v in &self.embedding {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    pub fn decode(b: &[u8]) -> Option<Row> {
        let mut at: usize = 0;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(at..at.checked_add(n)?)?;
            at += n;
            Some(s)
        };
        let stamp = u64::from_le_bytes(take(8)?.try_into().ok()?);
        let text_hash = u64::from_le_bytes(take(8)?.try_into().ok()?);
        let k = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
        let probs = take(4 * k)?.chunks(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let top = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
        let nm = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
        let members = take(2 * nm)?
            .chunks(2)
            .map(|c| u16::from_le_bytes(c.try_into().unwrap()) as usize)
            .collect();
        let ne = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        let embedding = take(4usize.checked_mul(ne)?)?
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        (top < k.max(1)).then_some(Row {
            stamp,
            text_hash,
            probs,
            top,
            members,
            embedding,
        })
    }

    /// the buckets it's filed under
    pub fn keys(&self) -> Vec<u32> {
        if self.members.is_empty() {
            return vec![bucket_key(UNCERTAIN, band(self.probs.get(self.top).copied().unwrap_or(0.0)))];
        }
        self.members
            .iter()
            .map(|&c| bucket_key(c as u32, band(self.probs.get(c).copied().unwrap_or(0.0))))
            .collect()
    }

    /// the top class's probability
    pub fn confidence(&self) -> f32 {
        self.probs.get(self.top).copied().unwrap_or(0.0)
    }
}

/// what a stack's outputs depend on: every classifier's weight version and steering
pub fn stamp(stack: &Stack) -> u64 {
    let mut h = fnv(0x73746d70, &FORMAT.to_le_bytes());
    for d in &stack.defs {
        h = fnv(h, d.name.as_bytes());
        h = fnv(h, &[0]);
        h = fnv(h, &d.version.to_le_bytes());
        h = fnv(h, serde_json::to_string(&d.steer).unwrap_or_default().as_bytes());
    }
    h
}

/// one row from one pass of the stack
pub fn read(stack: &Stack, stamp: u64, text: &str) -> Row {
    let (mut outs, embedding) = stack.graph.read_node(stack.node, text);
    let o = outs.swap_remove(0);
    Row {
        stamp,
        text_hash: text_hash(text),
        probs: o.probs,
        top: o.top,
        members: o.members,
        embedding,
    }
}

/// rows for many texts at once, in parallel, in the texts' order
pub fn read_many(stack: &Stack, stamp: u64, texts: &[&str], pool: &Pool) -> Vec<Row> {
    let slots: Vec<std::sync::Mutex<Option<Row>>> = texts.iter().map(|_| Default::default()).collect();
    pool.run(texts.len(), &|i| *slots[i].lock().unwrap() = Some(read(stack, stamp, texts[i])));
    slots.into_iter().map(|s| s.into_inner().unwrap().unwrap()).collect()
}

/// how indexing went
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexReport {
    pub total: usize,
    /// rows read through the network now
    pub computed: usize,
    /// rows that were there already
    pub cached: usize,
    pub stopped: bool,
    pub seconds: f64,
}

/// a member of a bucket
#[derive(Clone, Debug, PartialEq)]
pub struct Filed {
    pub r: Ref,
    pub text: String,
    pub row: Row,
}

/// what near texts are found by
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// the classifier's (steered) probabilities: texts it reads the same way
    Outputs,
    /// its pooled vector: texts it represents alike
    Embedding,
}

impl Store {
    /// the stamp rows of `stack` are kept under, dropping rows from an older one
    pub fn memo_fresh(&self, stack: &Stack) -> StoreResult<u64> {
        let name = stack.def().name.as_str();
        let now = stamp(stack);
        let kept = {
            let txn = self.db.begin_read().map_err(err)?;
            let t = txn.open_table(STAMPS).map_err(err)?;
            t.get(name).map_err(err)?.map(|g| g.value())
        };
        if kept != Some(now) {
            let txn = self.db.begin_write().map_err(err)?;
            clear_in(&txn, name)?;
            txn.open_table(STAMPS).map_err(err)?.insert(name, now).map_err(err)?;
            txn.commit().map_err(err)?;
        }
        Ok(now)
    }

    /// drops a classifier's rows and buckets (it was deleted or replaced)
    pub fn memo_clear(&self, name: &str) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        clear_in(&txn, name)?;
        txn.open_table(STAMPS).map_err(err)?.remove(name).map_err(err)?;
        txn.commit().map_err(err)
    }

    /// the rows kept for these refs, current or not (None where there's none)
    pub fn memo_kept(&self, name: &str, refs: &[Ref]) -> StoreResult<Vec<Option<Row>>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(MEMO).map_err(err)?;
        refs.iter()
            .map(|r| {
                let b = r.bytes();
                Ok(t.get((name, b.as_slice())).map_err(err)?.and_then(|g| Row::decode(g.value())))
            })
            .collect()
    }

    /// rows for texts (each a ref and what the classifier reads of it): the kept ones
    /// when current, the rest read now on the pool and kept
    pub fn memo_rows(&self, stack: &Stack, items: &[(Ref, &str)], pool: &Pool) -> StoreResult<Vec<Row>> {
        let stamp = self.memo_fresh(stack)?;
        let name = stack.def().name.clone();
        let refs: Vec<Ref> = items.iter().map(|i| i.0.clone()).collect();
        let kept = self.memo_kept(&name, &refs)?;
        let mut out: Vec<Option<Row>> = kept
            .into_iter()
            .zip(items)
            .map(|(k, (_, t))| k.filter(|r| r.stamp == stamp && r.text_hash == text_hash(t)))
            .collect();
        let missing: Vec<usize> = (0..items.len()).filter(|&i| out[i].is_none()).collect();
        for chunk in missing.chunks(BATCH * pool.threads().max(1)) {
            let texts: Vec<&str> = chunk.iter().map(|&i| items[i].1).collect();
            let rows = read_many(stack, stamp, &texts, pool);
            let pairs: Vec<(Ref, Row)> = chunk.iter().map(|&i| items[i].0.clone()).zip(rows.iter().cloned()).collect();
            if !self.memo_write(&name, stamp, &pairs)? {
                return Err(format!("{name:?} changed while it was read; try again"));
            }
            for (&i, r) in chunk.iter().zip(rows) {
                out[i] = Some(r);
            }
        }
        Ok(out.into_iter().map(|r| r.expect("filled")).collect())
    }

    /// writes rows (and refiles their buckets) if the stamp is still `stamp`; false if
    /// the classifier moved on meanwhile
    fn memo_write(&self, name: &str, stamp: u64, rows: &[(Ref, Row)]) -> StoreResult<bool> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let st = txn.open_table(STAMPS).map_err(err)?;
            if st.get(name).map_err(err)?.map(|g| g.value()) != Some(stamp) {
                return Ok(false);
            }
            let mut t = txn.open_table(MEMO).map_err(err)?;
            let mut b = txn.open_multimap_table(BUCKETS).map_err(err)?;
            for (r, row) in rows {
                let key = r.bytes();
                let old = t
                    .insert((name, key.as_slice()), row.encode().as_slice())
                    .map_err(err)?
                    .and_then(|g| Row::decode(g.value()));
                if let Some(old) = old {
                    for k in old.keys() {
                        b.remove((name, k), key.as_slice()).map_err(err)?;
                    }
                }
                for k in row.keys() {
                    b.insert((name, k), key.as_slice()).map_err(err)?;
                }
            }
        }
        txn.commit().map_err(err)?;
        Ok(true)
    }

    /// fills a classifier's rows ahead of use: for `refs`, or every memory and note
    /// (`limit` at most); `cb(done, total)` sees each batch and may stop it
    pub fn memo_index(&self, stack: &Stack, refs: Option<&[Ref]>, limit: usize, pool: &Pool, mut cb: impl FnMut(usize, usize) -> Flow) -> StoreResult<IndexReport> {
        let started = std::time::Instant::now();
        let items: Vec<(Ref, String)> = match refs {
            Some(rs) => rs
                .iter()
                .filter_map(|r| match self.ref_text(r) {
                    Ok(Some(t)) if !t.trim().is_empty() => Some(Ok((r.clone(), t))),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                })
                .collect::<StoreResult<_>>()?,
            None => self.candidates(limit)?.into_iter().map(|(r, t, _)| (r, t)).collect(),
        };
        let stamp = self.memo_fresh(stack)?;
        let name = stack.def().name.clone();
        let mut rep = IndexReport {
            total: items.len(),
            ..IndexReport::default()
        };
        let refs: Vec<Ref> = items.iter().map(|i| i.0.clone()).collect();
        let kept = self.memo_kept(&name, &refs)?;
        let todo: Vec<usize> = (0..items.len())
            .filter(|&i| !kept[i].as_ref().is_some_and(|r| r.stamp == stamp && r.text_hash == text_hash(&items[i].1)))
            .collect();
        rep.cached = items.len() - todo.len();
        let mut done = rep.cached;
        if cb(done, rep.total) == Flow::Stop {
            rep.stopped = true;
        }
        for chunk in todo.chunks(BATCH * pool.threads().max(1)) {
            if rep.stopped {
                break;
            }
            let texts: Vec<&str> = chunk.iter().map(|&i| items[i].1.as_str()).collect();
            let rows = read_many(stack, stamp, &texts, pool);
            let pairs: Vec<(Ref, Row)> = chunk.iter().map(|&i| items[i].0.clone()).zip(rows).collect();
            if !self.memo_write(&name, stamp, &pairs)? {
                return Err(format!("{name:?} changed while it was indexed; index it again"));
            }
            rep.computed += chunk.len();
            done += chunk.len();
            if cb(done, rep.total) == Flow::Stop {
                rep.stopped = true;
            }
        }
        rep.seconds = started.elapsed().as_secs_f64();
        Ok(rep)
    }

    /// how many current rows a classifier has
    pub fn memo_count(&self, stack: &Stack) -> StoreResult<usize> {
        let stamp = stamp(stack);
        let name = stack.def().name.as_str();
        let txn = self.db.begin_read().map_err(err)?;
        let st = txn.open_table(STAMPS).map_err(err)?;
        if st.get(name).map_err(err)?.map(|g| g.value()) != Some(stamp) {
            return Ok(0);
        }
        let t = txn.open_table(MEMO).map_err(err)?;
        let mut n = 0;
        for item in t.range((name, &[][..])..).map_err(err)? {
            let (k, _) = item.map_err(err)?;
            if k.value().0 != name {
                break;
            }
            n += 1;
        }
        Ok(n)
    }

    /// every current row of a classifier, with its ref
    pub fn memo_all(&self, stack: &Stack) -> StoreResult<Vec<(Ref, Row)>> {
        let stamp = self.memo_fresh(stack)?;
        let name = stack.def().name.as_str();
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(MEMO).map_err(err)?;
        let mut out = Vec::new();
        for item in t.range((name, &[][..])..).map_err(err)? {
            let (k, v) = item.map_err(err)?;
            let (n, r) = k.value();
            if n != name {
                break;
            }
            if let Some(row) = Row::decode(v.value()).filter(|row| row.stamp == stamp) {
                out.push((Ref::from_bytes(r), row));
            }
        }
        Ok(out)
    }

    /// a bucket: the texts filed under `class` (an index, or `UNCERTAIN`) in the bands
    /// `bands`, surest first; texts that changed since they were read are left out (and
    /// counted as the second value)
    pub fn class_bucket(&self, stack: &Stack, class: u32, bands: std::ops::RangeInclusive<u32>) -> StoreResult<(Vec<Filed>, usize)> {
        let stamp = self.memo_fresh(stack)?;
        let name = stack.def().name.as_str();
        let mut refs: Vec<Vec<u8>> = Vec::new();
        {
            let txn = self.db.begin_read().map_err(err)?;
            let b = txn.open_multimap_table(BUCKETS).map_err(err)?;
            for band in bands {
                if band >= BANDS {
                    break;
                }
                for v in b.get((name, bucket_key(class, band))).map_err(err)? {
                    refs.push(v.map_err(err)?.value().to_vec());
                }
            }
        }
        let rs: Vec<Ref> = refs.iter().map(|b| Ref::from_bytes(b)).collect();
        let rows = self.memo_kept(name, &rs)?;
        let mut out = Vec::new();
        let mut stale = 0;
        for (r, row) in rs.into_iter().zip(rows) {
            let Some(row) = row.filter(|x| x.stamp == stamp) else {
                stale += 1;
                continue;
            };
            match self.ref_text(&r)? {
                Some(t) if text_hash(&t) == row.text_hash => out.push(Filed { r, text: t, row }),
                _ => stale += 1,
            }
        }
        let p = |f: &Filed| {
            if class == UNCERTAIN {
                f.row.confidence()
            } else {
                f.row.probs.get(class as usize).copied().unwrap_or(0.0)
            }
        };
        out.sort_by(|a, b| p(b).total_cmp(&p(a)).then_with(|| a.r.bytes().cmp(&b.r.bytes())));
        Ok((out, stale))
    }

    /// how many texts each bucket holds: (class or `UNCERTAIN`, band) -> count
    pub fn bucket_counts(&self, stack: &Stack) -> StoreResult<Vec<((u32, u32), usize)>> {
        let mut counts: HashMap<(u32, u32), usize> = HashMap::new();
        for (_, row) in self.memo_all(stack)? {
            for k in row.keys() {
                *counts.entry((k / 16, k % 16)).or_default() += 1;
            }
        }
        let mut v: Vec<_> = counts.into_iter().collect();
        v.sort();
        Ok(v)
    }

    /// the texts nearest a row by cosine, among the ones indexed (`except` left out);
    /// ties by key, so the order is fixed
    pub fn memo_near(&self, stack: &Stack, me: &Row, by: By, except: Option<&Ref>, limit: usize) -> StoreResult<Vec<(Ref, String, f32)>> {
        let all = self.memo_all(stack)?;
        let v = |r: &Row| -> Vec<f32> {
            match by {
                By::Outputs => r.probs.clone(),
                By::Embedding => r.embedding.clone(),
            }
        };
        let q = v(me);
        let mut scored: Vec<(f32, Ref, u64)> = all
            .into_iter()
            .filter(|(r, _)| Some(r) != except)
            .map(|(r, row)| (crate::classify::cosine(&q, &v(&row)), r, row.text_hash))
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.bytes().cmp(&b.1.bytes())));
        let mut out = Vec::new();
        for (c, r, h) in scored {
            if out.len() >= limit {
                break;
            }
            // a text that changed since is skipped (indexing again brings it back)
            if let Some(t) = self.ref_text(&r)?
                && text_hash(&t) == h
            {
                out.push((r, t, c));
            }
        }
        Ok(out)
    }

    // ---- which classifiers index a memory or note

    /// the classifiers chosen to index a memory or note
    pub fn indexed_on(&self, r: &Ref) -> StoreResult<Vec<String>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(INDEXED).map_err(err)?;
        let b = r.bytes();
        Ok(t.get(b.as_slice())
            .map_err(err)?
            .and_then(|g| serde_json::from_slice(g.value()).ok())
            .unwrap_or_default())
    }

    /// chooses the classifiers that index a memory or note (none: forgets the choice)
    pub fn set_indexed_on(&self, r: &Ref, names: &[String]) -> StoreResult<Vec<String>> {
        let mut v: Vec<String> = Vec::new();
        for n in names {
            self.classifier(n)?;
            if !v.contains(n) {
                v.push(n.clone());
            }
        }
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(INDEXED).map_err(err)?;
            let b = r.bytes();
            if v.is_empty() {
                t.remove(b.as_slice()).map_err(err)?;
            } else {
                t.insert(b.as_slice(), serde_json::to_vec(&v).map_err(err)?.as_slice()).map_err(err)?;
            }
        }
        txn.commit().map_err(err)?;
        Ok(v)
    }

    /// every memory and note with chosen classifiers
    pub fn all_indexed(&self) -> StoreResult<Vec<(Ref, Vec<String>)>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(INDEXED).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (k, v) = item.map_err(err)?;
            out.push((Ref::from_bytes(k.value()), serde_json::from_slice(v.value()).unwrap_or_default()));
        }
        Ok(out)
    }

    /// takes a deleted classifier out of every choice
    pub fn unindex_classifier(&self, name: &str) -> StoreResult<()> {
        for (r, names) in self.all_indexed()? {
            if names.iter().any(|n| n == name) {
                let keep: Vec<String> = names.into_iter().filter(|n| n != name).collect();
                self.set_indexed_on_unchecked(&r, &keep)?;
            }
        }
        Ok(())
    }

    fn set_indexed_on_unchecked(&self, r: &Ref, names: &[String]) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(INDEXED).map_err(err)?;
            let b = r.bytes();
            if names.is_empty() {
                t.remove(b.as_slice()).map_err(err)?;
            } else {
                t.insert(b.as_slice(), serde_json::to_vec(names).map_err(err)?.as_slice()).map_err(err)?;
            }
        }
        txn.commit().map_err(err)
    }
}

/// drops every row and bucket of one classifier inside a transaction
fn clear_in(txn: &WriteTransaction, name: &str) -> StoreResult<()> {
    let mut t = txn.open_table(MEMO).map_err(err)?;
    let mut keys: Vec<Vec<u8>> = Vec::new();
    for item in t.range((name, &[][..])..).map_err(err)? {
        let (k, _) = item.map_err(err)?;
        let (n, r) = k.value();
        if n != name {
            break;
        }
        keys.push(r.to_vec());
    }
    for k in &keys {
        t.remove((name, k.as_slice())).map_err(err)?;
    }
    let mut b = txn.open_multimap_table(BUCKETS).map_err(err)?;
    let mut bk: Vec<u32> = Vec::new();
    use redb::ReadableMultimapTable;
    for item in b.range((name, 0)..=(name, u32::MAX)).map_err(err)? {
        let (k, _) = item.map_err(err)?;
        bk.push(k.value().1);
    }
    for k in bk {
        b.remove_all((name, k)).map_err(err)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- colours

/// a class's colour: a fixed sand-friendly set for the first ten, then golden-angle hues
pub fn palette(k: usize) -> Vec<[u8; 3]> {
    const BASE: [[u8; 3]; 10] = [
        [0xc0, 0x5a, 0x3c], // terracotta
        [0x3f, 0x7f, 0x8c], // teal
        [0xc8, 0x9a, 0x2e], // ochre
        [0x6b, 0x5b, 0x95], // plum
        [0x5f, 0x8a, 0x4a], // olive
        [0xb0, 0x4f, 0x7a], // rose
        [0x2f, 0x5d, 0x9e], // indigo
        [0x9a, 0x6a, 0x3a], // umber
        [0x4f, 0x9e, 0x86], // jade
        [0x8e, 0x3b, 0x3b], // oxblood
    ];
    (0..k)
        .map(|i| {
            if i < BASE.len() {
                BASE[i]
            } else {
                hsl(((i as f64) * 137.507_764) % 360.0, 0.45, 0.45)
            }
        })
        .collect()
}

fn hsl(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [to(r), to(g), to(b)]
}

/// how a text is tinted by a classifier: its class (None when it's in none, shown
/// neutral) and how strongly (the class's probability, 0 to 1)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tint {
    pub class: Option<usize>,
    pub strength: f32,
}

pub fn tint(row: &Row) -> Tint {
    match row.members.iter().copied().max_by(|&a, &b| row.probs[a].total_cmp(&row.probs[b]).then(b.cmp(&a))) {
        Some(c) => Tint {
            class: Some(c),
            strength: row.probs[c],
        },
        None => Tint {
            class: None,
            strength: row.confidence(),
        },
    }
}

impl Store {
    /// the tints of some memories and notes by a classifier, filling what isn't memoized
    /// (a ref with no text gets None)
    pub fn tints(&self, stack: &Stack, refs: &[Ref], pool: &Pool) -> StoreResult<Vec<Option<Tint>>> {
        let mut items: Vec<(Ref, String)> = Vec::new();
        let mut at: Vec<Option<usize>> = Vec::new();
        for r in refs {
            match self.ref_text(r)? {
                Some(t) if !t.trim().is_empty() => {
                    at.push(Some(items.len()));
                    items.push((r.clone(), t));
                }
                _ => at.push(None),
            }
        }
        let its: Vec<(Ref, &str)> = items.iter().map(|(r, t)| (r.clone(), t.as_str())).collect();
        let rows = self.memo_rows(stack, &its, pool)?;
        Ok(at.into_iter().map(|i| i.map(|i| tint(&rows[i]))).collect())
    }
}
