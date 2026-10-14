//! The query model the lenses read through: filters by words, sums, letters, tags, time,
//! mime and relations, intersected, then ordered and paged; and the figures for a stats
//! view.

use std::collections::{BTreeMap, HashMap};

use super::gematria::{Measure, Mode};
use super::postings::{TIME_BUCKET_BITS, Term};
use super::record::{self as rec, mime};
use super::store::{GResult, Reader, View, rel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub offset: usize,
    pub limit: usize,
}

impl Page {
    pub fn first(limit: usize) -> Page {
        Page { offset: 0, limit }
    }

    pub fn all() -> Page {
        Page {
            offset: 0,
            limit: usize::MAX,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// the entries `id` has edges to (a list's members, what a note is on, an entry's tags,
    /// what a map places)
    From,
    /// the entries with edges to `id` (notes on it, lists holding it, entries with a tag)
    To,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// ascending ids (weum's key order)
    #[default]
    Id,
    /// oldest first
    Created,
    /// newest first
    CreatedDesc,
    /// most recently edited first
    EditedDesc,
    /// the relation's own order (a list's member order, notes oldest first), when the
    /// query has a relation; else by id
    Relation,
}

/// what to find; every filter given must hold
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// weum's token search: entries with at least three quarters of the query's words
    pub words: Option<String>,
    /// entries filed under a sum
    pub sum: Option<(Mode, i64)>,
    /// entries with a sum in an inclusive range
    pub sum_range: Option<(Mode, i64, i64)>,
    /// anagrams of a text (the same letter frequencies)
    pub letters_of: Option<String>,
    /// entries carrying a tag entry
    pub tag: Option<u64>,
    /// created between these times (ms, inclusive)
    pub created: Option<(u64, u64)>,
    /// last edited between these times (ms, inclusive)
    pub edited: Option<(u64, u64)>,
    pub mime: Option<u16>,
    /// entries related to an entry
    pub related: Option<(u16, u64, Dir)>,
    pub order: Order,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// the page of ids
    pub ids: Vec<u64>,
    /// how many matched in all
    pub total: usize,
}

fn intersect(a: Vec<u64>, b: &[u64]) -> Vec<u64> {
    let set: std::collections::HashSet<u64> = b.iter().copied().collect();
    a.into_iter().filter(|x| set.contains(x)).collect()
}

impl View<'_> {
    /// weum's search: ids found under at least three quarters of the query's tokens, in
    /// the order they turn up
    pub fn search_words(&self, query: &str) -> GResult<Vec<u64>> {
        let tokens = crate::simd::unique_tokens(query.as_bytes());
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let threshold = ((tokens.len() as f32 * 0.75).ceil() as usize).max(1);
        let mut scores: HashMap<u64, usize> = HashMap::new();
        let mut order = Vec::new();
        for tk in tokens {
            for id in self.postings(&Term::Word(tk.to_vec()))? {
                let n = scores.entry(id).or_insert(0);
                if *n == 0 {
                    order.push(id);
                }
                *n += 1;
            }
        }
        Ok(order.into_iter().filter(|id| scores[id] >= threshold).collect())
    }

    /// entries filed under a sum
    pub fn by_sum(&self, mode: Mode, value: i64) -> GResult<Vec<u64>> {
        self.postings(&Term::Sum(mode as u8, value))
    }

    /// anagrams of a text; candidates by the frequency key's hash, checked
    pub fn by_letters(&self, text: &str) -> GResult<Vec<u64>> {
        let key = crate::simd::frequency_key(text.as_bytes());
        if key.is_empty() {
            return Ok(Vec::new());
        }
        let ids = self.postings(&Term::Freq(rec::content_hash(&key)))?;
        let texts = self.render_many(&ids);
        Ok(ids
            .into_iter()
            .zip(texts)
            .filter(|(_, t)| {
                t.as_ref()
                    .is_ok_and(|t| crate::simd::frequency_key(t.as_bytes()) == key)
            })
            .map(|(id, _)| id)
            .collect())
    }

    /// entries created in a time range (ms, inclusive), oldest first
    pub fn created_between(&self, from: u64, to: u64) -> GResult<Vec<u64>> {
        let mut out = Vec::new();
        for id in self.created_buckets(from >> TIME_BUCKET_BITS, to >> TIME_BUCKET_BITS)? {
            if let Some(r) = self.record(id)?
                && r.created >= from
                && r.created <= to
            {
                out.push((r.created, id));
            }
        }
        out.sort();
        out.dedup();
        Ok(out.into_iter().map(|(_, id)| id).collect())
    }

    fn related(&self, r: u16, id: u64, dir: Dir) -> GResult<Vec<u64>> {
        Ok(match (r, dir) {
            (rel::MEMBER, Dir::From) => self.members(id)?,
            (rel::NOTE, Dir::To) => self.notes_on(id)?,
            (_, Dir::From) => {
                let mut v: Vec<u64> = self.out_edges(id, r)?.into_iter().map(|(t, _)| t).collect();
                v.dedup();
                v
            }
            (_, Dir::To) => self.in_edges(id, r)?,
        })
    }

    pub fn query(&self, q: &Query, page: Page) -> GResult<Found> {
        let mut sets: Vec<Vec<u64>> = Vec::new();
        let mut rel_order: Option<Vec<u64>> = None;
        if let Some((r, id, dir)) = q.related {
            let v = self.related(r, id, dir)?;
            rel_order = Some(v.clone());
            sets.push(v);
        }
        if let Some(w) = &q.words {
            sets.push(self.search_words(w)?);
        }
        if let Some((m, v)) = q.sum {
            sets.push(self.by_sum(m, v)?);
        }
        if let Some((m, lo, hi)) = q.sum_range {
            let mut v = Vec::new();
            for (value, _) in self.sum_counts(m)? {
                if value >= lo && value <= hi {
                    v.extend(self.by_sum(m, value)?);
                }
            }
            sets.push(v);
        }
        if let Some(t) = &q.letters_of {
            sets.push(self.by_letters(t)?);
        }
        if let Some(t) = q.tag {
            sets.push(self.in_edges(t, rel::TAG)?);
        }
        if let Some((a, b)) = q.created {
            sets.push(self.created_between(a, b)?);
        }
        let mut ids = match sets.len() {
            0 => self.ids()?,
            _ => {
                // the first set keeps its order (relation order, search order)
                let mut it = sets.into_iter();
                let first = it.next().unwrap();
                it.fold(first, |acc, s| intersect(acc, &s))
            }
        };
        let mut seen = std::collections::HashSet::new();
        ids.retain(|id| seen.insert(*id));
        let needs_records = q.mime.is_some()
            || q.edited.is_some()
            || matches!(q.order, Order::Created | Order::CreatedDesc | Order::EditedDesc);
        let mut rows: Vec<(u64, u64, u64)> = Vec::with_capacity(ids.len());
        for id in ids {
            if needs_records {
                let Some(r) = self.record(id)? else { continue };
                if q.mime.is_some_and(|m| m != r.mime) {
                    continue;
                }
                if q.edited.is_some_and(|(a, b)| r.edited < a || r.edited > b) {
                    continue;
                }
                rows.push((id, r.created, r.edited));
            } else {
                rows.push((id, 0, 0));
            }
        }
        match q.order {
            Order::Id => rows.sort_by_key(|r| r.0),
            Order::Created => rows.sort_by_key(|r| (r.1, r.0)),
            Order::CreatedDesc => rows.sort_by_key(|r| std::cmp::Reverse((r.1, r.0))),
            Order::EditedDesc => rows.sort_by_key(|r| std::cmp::Reverse((r.2, r.0))),
            Order::Relation => {
                if rel_order.is_none() {
                    rows.sort_by_key(|r| r.0);
                }
            }
        }
        let total = rows.len();
        let ids = rows
            .into_iter()
            .skip(page.offset)
            .take(page.limit)
            .map(|r| r.0)
            .collect();
        Ok(Found { ids, total })
    }

    /// the figures of a stats view
    pub fn stats(&self) -> GResult<Stats> {
        let all = self.all()?;
        let mut by_mime: BTreeMap<u16, u64> = BTreeMap::new();
        let mut oldest = u64::MAX;
        let mut newest = 0;
        let mut body_bytes = 0u64;
        let mut borrowing = 0u64;
        for e in &all {
            *by_mime.entry(e.record.mime).or_default() += 1;
            oldest = oldest.min(e.record.created);
            newest = newest.max(e.record.created);
            body_bytes += e.record.body.len() as u64;
            if e.record.plain() && e.record.has_refs() {
                borrowing += 1;
            }
        }
        let mut edges: BTreeMap<u16, u64> = BTreeMap::new();
        for (_, r, _, _) in self.all_edges()? {
            *edges.entry(r).or_default() += 1;
        }
        let mut mimes = Vec::new();
        for (m, n) in by_mime {
            mimes.push((self.mime_name(m)?, n));
        }
        Ok(Stats {
            entries: all.len() as u64,
            by_mime: mimes,
            edges: edges.into_iter().map(|(r, n)| (rel::name(r), n)).collect(),
            versions: self.history_count()?,
            oldest: (oldest != u64::MAX).then_some(oldest),
            newest: (!all.is_empty()).then_some(newest),
            body_bytes,
            borrowing,
            notes: all.iter().filter(|e| e.record.mime == mime::MARKDOWN).count() as u64,
        })
    }

    /// what a text measures (for a sidecar)
    pub fn measure(text: &str) -> Measure {
        Measure::of(text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    pub entries: u64,
    pub by_mime: Vec<(String, u64)>,
    pub edges: Vec<(String, u64)>,
    pub versions: u64,
    pub oldest: Option<u64>,
    pub newest: Option<u64>,
    /// packed body bytes of every entry
    pub body_bytes: u64,
    /// entries stored partly as refs to others
    pub borrowing: u64,
    pub notes: u64,
}
