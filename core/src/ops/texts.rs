//! The entry graph as a classifier's texts (`crate::texts::Texts`): every entry is a
//! `Ref::Memory` of its weum key (`record::key_of`), notes included; a note reads as its
//! title and markdown and shows as its title. Lists are member edges, tags tag edges.

use std::collections::HashSet;
use std::sync::Arc;

use crate::graph::query::{Page, Query};
use crate::graph::record::{self, mime};
use crate::graph::store::rel;
use crate::graph::{Graph, Reader};
use crate::memory::Ref;
use crate::store::StoreResult;

pub struct GraphTexts(pub Arc<Graph>);

/// an entry's ref, the way the apps' keys name it
pub fn ref_of(id: u64) -> Ref {
    Ref::Memory(record::key_of(id))
}

/// the entry a ref names (`note:<label>` reads as the label)
pub fn id_of(r: &Ref) -> Option<u64> {
    match r {
        Ref::Memory(k) => record::id_of(k),
        Ref::Note(l) => record::parse_label(l),
    }
}

fn need(r: &Ref) -> StoreResult<u64> {
    id_of(r).ok_or_else(|| format!("{} isn't a key", r.label()))
}

/// a note as a classifier reads it: its title, a blank line, its markdown
fn note_reads(text: &str) -> String {
    let (title, md) = crate::note::split(text);
    if md.trim().is_empty() { title } else { format!("{title}\n\n{md}") }
}

impl crate::texts::Texts for GraphTexts {
    fn members(&self, list: &Ref) -> StoreResult<Vec<Ref>> {
        let Some(id) = id_of(list) else { return Ok(vec![]) };
        Ok(self.0.view()?.members(id)?.into_iter().map(ref_of).collect())
    }

    fn text(&self, r: &Ref) -> StoreResult<Option<String>> {
        let Some(id) = id_of(r) else { return Ok(None) };
        let v = self.0.view()?;
        let Some(rec) = v.record(id)? else { return Ok(None) };
        let t = v.render(id)?;
        Ok(Some(if rec.mime == mime::MARKDOWN { note_reads(&t) } else { t }))
    }

    fn shown(&self, r: &Ref) -> StoreResult<Option<String>> {
        let Some(id) = id_of(r) else { return Ok(None) };
        let v = self.0.view()?;
        let Some(rec) = v.record(id)? else { return Ok(None) };
        let t = v.render(id)?;
        Ok(Some(if rec.mime == mime::MARKDOWN { crate::note::split(&t).0 } else { t }))
    }

    fn candidates(&self, limit: usize) -> StoreResult<Vec<(Ref, String, String)>> {
        let v = self.0.view()?;
        let mut out = Vec::new();
        for id in v.query(&Query::default(), Page::all())?.ids {
            if out.len() >= limit {
                break;
            }
            let Some(rec) = v.record(id)? else { continue };
            if rec.mime != mime::TEXT && rec.mime != mime::MARKDOWN {
                continue;
            }
            let Ok(t) = v.render(id) else { continue };
            if t.trim().is_empty() {
                continue;
            }
            if rec.mime == mime::MARKDOWN {
                out.push((ref_of(id), note_reads(&t), crate::note::split(&t).0));
            } else {
                out.push((ref_of(id), t.clone(), t));
            }
        }
        Ok(out)
    }

    fn search(&self, q: &str) -> StoreResult<Vec<Ref>> {
        let v = self.0.view()?;
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for variant in crate::weum::search_variations(q.trim()) {
            for id in v.search_words(&variant)? {
                if seen.insert(id) {
                    out.push(ref_of(id));
                }
            }
        }
        Ok(out)
    }

    fn saturate(&self, text: &str) -> StoreResult<(Ref, bool)> {
        let (id, new) = self.0.save_text(text, mime::TEXT)?;
        Ok((ref_of(id), new))
    }

    fn unlist(&self, list: &Ref) -> StoreResult<u64> {
        let Some(l) = id_of(list) else { return Ok(0) };
        self.0.write(|tx| {
            let members = tx.members(l)?;
            for m in &members {
                tx.unlink(l, rel::MEMBER, *m)?;
            }
            Ok(members.len() as u64)
        })
    }

    fn enlist_many(&self, pairs: &[(Ref, Ref)]) -> StoreResult<()> {
        let pairs = pairs
            .iter()
            .map(|(l, m)| Ok((need(l)?, need(m)?)))
            .collect::<StoreResult<Vec<_>>>()?;
        self.0.write(|tx| {
            for (l, m) in &pairs {
                if tx.exists(*l)? && tx.exists(*m)? {
                    tx.add_member(*l, *m)?;
                }
            }
            Ok(())
        })
    }

    fn is_note(&self, r: &Ref) -> bool {
        let Some(id) = id_of(r) else { return false };
        self.0
            .view()
            .ok()
            .and_then(|v| v.record(id).ok().flatten())
            .is_some_and(|rec| rec.mime == mime::MARKDOWN)
    }

    fn tags(&self, r: &Ref) -> StoreResult<Vec<String>> {
        let Some(id) = id_of(r) else { return Ok(vec![]) };
        let v = self.0.view()?;
        Ok(v.tags_of(id)?.into_iter().filter_map(|t| v.render(t).ok()).collect())
    }
}
