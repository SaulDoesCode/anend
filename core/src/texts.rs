//! Where a classifier's texts come from. The classifiers keep their own tables in a
//! [`Store`](crate::store::Store) (definitions, weights, feedback, memo rows, filters), but
//! the texts and lists they read can live elsewhere: a store given a [`Texts`] reads list
//! members, texts, candidates and word searches through it, and writes the lists it makes
//! (a classifier's outputs, a filter's matches, a module's classes) back through it. The
//! entry graph's (`ops::GraphTexts`) is the one the apps use; without one, a store reads
//! its own memories, notes and lists as before.

use crate::memory::{Hit, Ref};
use crate::store::StoreResult;

pub trait Texts: Send + Sync {
    /// a list's members, in order
    fn members(&self, list: &Ref) -> StoreResult<Vec<Ref>>;
    /// what a classifier reads of an entry (a note: its title and markdown)
    fn text(&self, r: &Ref) -> StoreResult<Option<String>>;
    /// how an entry is shown (a note: its title)
    fn shown(&self, r: &Ref) -> StoreResult<Option<String>>;
    /// every text a classifier can rank, up to `limit`: (ref, what it reads, what's shown)
    fn candidates(&self, limit: usize) -> StoreResult<Vec<(Ref, String, String)>>;
    /// weum's word search with its variations, in the order found
    fn search(&self, q: &str) -> StoreResult<Vec<Ref>>;
    /// the entry with exactly this text (made when there's none); whether it's new
    fn saturate(&self, text: &str) -> StoreResult<(Ref, bool)>;
    /// empties a list; returns how many members it had
    fn unlist(&self, list: &Ref) -> StoreResult<u64>;
    /// adds members to lists
    fn enlist_many(&self, pairs: &[(Ref, Ref)]) -> StoreResult<()>;
    /// whether an entry is a note
    fn is_note(&self, r: &Ref) -> bool;
    /// an entry's tags
    fn tags(&self, r: &Ref) -> StoreResult<Vec<String>>;
}

impl crate::store::Store {
    /// the texts' source, when it isn't this store's own tables
    pub fn texts(&self) -> Option<&dyn Texts> {
        self.texts.as_deref()
    }

    /// whether a ref is a note (by its kind, or the source's say)
    pub fn is_note(&self, r: &Ref) -> bool {
        match self.texts() {
            Some(t) => t.is_note(r),
            None => matches!(r, Ref::Note(_)),
        }
    }

    pub(crate) fn hit_from(&self, t: &dyn Texts, r: &Ref) -> StoreResult<Hit> {
        let text = t.shown(r)?.ok_or("didn't find that")?;
        Ok(Hit { r: r.clone(), text })
    }
}
