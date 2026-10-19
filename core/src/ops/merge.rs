//! Saves into saves: merging one entry graph into another, and the forge (several saves
//! added, taken away or kept where they agree, then narrowed by a filter). An entry is
//! known by its mime and rendered text, so the same text in two saves is one entry in
//! the result (the target's own, when it has it); new entries get fresh ids and keep
//! their times. Lists come along as (head, member) pairs in their order, tags with
//! the entries carrying them, and notes-on, placements and other relations when both
//! ends came along. Structured entries (templates, functions) arrive as their rendered
//! text.

use std::collections::{HashMap, HashSet};

use crate::graph::Reader;
use crate::graph::gematria::Measure;
use crate::graph::record::mime;
use crate::graph::store::{GResult, Tx, View, rel};

/// an entry as the forge sees it: (mime, rendered text)
pub type Key = (u16, String);

#[derive(Clone, Debug)]
pub struct Item {
    /// which of the sources it came from
    pub save: usize,
    pub id: u64,
    pub mime: u16,
    pub text: String,
    pub created: u64,
    pub edited: u64,
}

impl Item {
    fn key(&self) -> Key {
        (self.mime, self.text.clone())
    }

    pub fn is_note(&self) -> bool {
        self.mime == mime::MARKDOWN
    }
}

/// what a save (or a combination of saves) holds
#[derive(Clone, Debug, Default)]
pub struct Contents {
    pub items: Vec<Item>,
    /// list pairs (head, member) in list order
    pub links: Vec<(Key, Key)>,
    /// where each key a link names can be read from
    ends: HashMap<Key, Item>,
}

impl Contents {
    pub fn memories(&self) -> usize {
        self.items.iter().filter(|i| !i.is_note()).count()
    }

    pub fn notes(&self) -> usize {
        self.items.iter().filter(|i| i.is_note()).count()
    }
}

/// everything in a save (tag entries come with what they tag, not on their own)
pub fn read(save: usize, v: &View) -> GResult<Contents> {
    let mut c = Contents::default();
    let mut by_id: HashMap<u64, Item> = HashMap::new();
    for e in v.all()? {
        if e.record.mime == mime::TAG {
            continue;
        }
        let Ok(text) = v.render(e.id) else { continue };
        let it = Item {
            save,
            id: e.id,
            mime: e.record.mime,
            text,
            created: e.record.created,
            edited: e.record.edited,
        };
        by_id.insert(e.id, it.clone());
        c.items.push(it);
    }
    let mut heads = Vec::new();
    let mut seen = HashSet::new();
    for (from, r, _, _) in v.all_edges()? {
        if r == rel::MEMBER && seen.insert(from) {
            heads.push(from);
        }
    }
    for h in heads {
        let Some(head) = by_id.get(&h) else { continue };
        for m in v.members(h)? {
            let Some(member) = by_id.get(&m) else { continue };
            c.ends.entry(head.key()).or_insert_with(|| head.clone());
            c.ends.entry(member.key()).or_insert_with(|| member.clone());
            c.links.push((head.key(), member.key()));
        }
    }
    Ok(c)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Add,
    TakeAway,
    KeepShared,
}

impl Effect {
    pub fn parse(s: &str) -> Effect {
        match s {
            "take away" | "take_away" | "-" => Effect::TakeAway,
            "keep shared" | "keep_shared" | "shared" => Effect::KeepShared,
            _ => Effect::Add,
        }
    }
}

/// the saves in order, each added, taken away or kept where it agrees with what's so far
pub fn combine(parts: Vec<(Effect, Contents)>) -> Contents {
    let mut out = Contents::default();
    for (effect, c) in parts {
        let keys: HashSet<Key> = c.items.iter().map(Item::key).collect();
        let links: HashSet<(Key, Key)> = c.links.iter().cloned().collect();
        match effect {
            Effect::Add => {
                let have: HashSet<Key> = out.items.iter().map(Item::key).collect();
                out.items
                    .extend(c.items.into_iter().filter(|i| !have.contains(&i.key())));
                let have: HashSet<(Key, Key)> = out.links.iter().cloned().collect();
                out.links.extend(c.links.into_iter().filter(|l| !have.contains(l)));
                for (k, it) in c.ends {
                    out.ends.entry(k).or_insert(it);
                }
            }
            Effect::TakeAway => {
                out.items.retain(|i| !keys.contains(&i.key()));
                out.links.retain(|l| !links.contains(l));
            }
            Effect::KeepShared => {
                out.items.retain(|i| keys.contains(&i.key()));
                out.links.retain(|l| links.contains(l));
            }
        }
    }
    out
}

/// what the forge's result is narrowed to
#[derive(Clone, Debug)]
pub struct Filter {
    /// words a text must contain; `-word` ones it mustn't
    pub words: String,
    /// inclusive ranges of a memory's ordinal and reversed sums
    pub ordinal: (Option<i64>, Option<i64>),
    pub reversed: (Option<i64>, Option<i64>),
    pub memories: bool,
    pub notes: bool,
    pub lists: bool,
}

impl Default for Filter {
    fn default() -> Filter {
        Filter {
            words: String::new(),
            ordinal: (None, None),
            reversed: (None, None),
            memories: true,
            notes: true,
            lists: true,
        }
    }
}

fn within(v: i64, (lo, hi): (Option<i64>, Option<i64>)) -> bool {
    lo.is_none_or(|l| v >= l) && hi.is_none_or(|h| v <= h)
}

impl Filter {
    fn words_match(&self, text: &str) -> bool {
        let text = text.to_lowercase();
        self.words.split_whitespace().all(|w| {
            let w = w.to_lowercase();
            match w.strip_prefix('-') {
                Some(not) if !not.is_empty() => !text.contains(not),
                _ => text.contains(&w),
            }
        })
    }

    fn sums_set(&self) -> bool {
        self.ordinal != (None, None) || self.reversed != (None, None)
    }

    pub fn apply(&self, mut c: Contents) -> Contents {
        c.items.retain(|i| {
            if i.is_note() {
                // a sum range is about memories; notes only answer to words
                self.notes && !self.sums_set() && self.words_match(&i.text)
            } else {
                let m = Measure::of(&i.text);
                self.memories
                    && self.words_match(&i.text)
                    && within(m.ordinal, self.ordinal)
                    && within(m.reversed, self.reversed)
            }
        });
        if !self.lists {
            c.links.clear();
        }
        c
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Written {
    /// new memories (entries that aren't notes) and notes
    pub memories: usize,
    pub notes: usize,
    /// lists that gained members
    pub lists: usize,
}

/// writes `c` into the save `tx` writes, reading entries' tags and relations from
/// `views` (indexed as the items' `save`)
pub fn write(tx: &mut Tx, views: &[&View], c: &Contents) -> GResult<Written> {
    let mut w = Written::default();
    let mut map: HashMap<Key, u64> = HashMap::new();
    // (source save, source id) → the target's id
    let mut ids: HashMap<(usize, u64), u64> = HashMap::new();
    let mut put = |tx: &mut Tx, it: &Item, w: &mut Written| -> GResult<u64> {
        if let Some(&id) = map.get(&it.key()) {
            return Ok(id);
        }
        let id = match tx.find_text(&it.text, Some(it.mime))?.first() {
            Some(&id) => id,
            None => {
                let id = tx.alloc_id()?;
                tx.put_text_at(id, &it.text, it.mime, it.created, it.edited)?;
                if it.is_note() {
                    w.notes += 1;
                } else {
                    w.memories += 1;
                }
                id
            }
        };
        map.insert(it.key(), id);
        Ok(id)
    };
    for it in &c.items {
        let id = put(tx, it, &mut w)?;
        ids.insert((it.save, it.id), id);
    }
    // lists, in order; their ends come along when the filter left them out
    let mut grew = HashSet::new();
    for (h, m) in &c.links {
        let (Some(hi), Some(mi)) = (c.ends.get(h), c.ends.get(m)) else {
            continue;
        };
        let head = put(tx, hi, &mut w)?;
        let member = put(tx, mi, &mut w)?;
        ids.entry((hi.save, hi.id)).or_insert(head);
        ids.entry((mi.save, mi.id)).or_insert(member);
        if !tx.members(head)?.contains(&member) {
            tx.add_member(head, member)?;
            grew.insert(head);
        }
    }
    w.lists = grew.len();
    // tags, and the relations both of whose ends came
    let pairs: Vec<((usize, u64), u64)> = ids.iter().map(|(k, v)| (*k, *v)).collect();
    for ((save, src), dst) in pairs {
        let Some(v) = views.get(save) else { continue };
        for t in v.tags_of(src)? {
            if let Ok(name) = v.render(t) {
                tx.tag(dst, &name)?;
            }
        }
        let (outs, _) = v.relations_of(src)?;
        for r in outs {
            if r == rel::MEMBER || r == rel::TAG {
                continue;
            }
            for (to, payload) in v.out_edges(src, r)? {
                let Some(&dto) = ids.get(&(save, to)) else { continue };
                let have = tx.out_edges(dst, r)?;
                if !have.iter().any(|(t, p)| *t == dto && *p == payload) {
                    tx.link(dst, r, dto, &payload)?;
                }
            }
        }
    }
    Ok(w)
}

/// one whole save into another
pub fn merge(tx: &mut Tx, src: &View) -> GResult<Written> {
    let c = read(0, src)?;
    write(tx, &[src], &c)
}

/// what [`assets`] brought in
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetsWritten {
    pub models: usize,
    pub pictures: usize,
}

/// a save's spec models and pictures into another (a merge's second half; the entries
/// go through [`merge`]). Pictures are known by their content hash, so one in both saves
/// stays one; their lists are joined. A spec model whose name the target has already:
/// the same spec (its JSON as written) is the same model and nothing changes; a
/// different one comes in beside it as `<name> 2` (or 3, ...), its spec renamed too, so
/// the target's own models (and the critics naming them) keep answering as before.
/// Spec models arrive untrained (their weights live in the source's models file).
pub fn assets(target: &crate::graph::Graph, src: &crate::graph::Graph) -> GResult<AssetsWritten> {
    let mut w = AssetsWritten::default();
    for (name, bytes) in src.spec_records()? {
        let r = crate::specs::Record::from_json(&bytes)?;
        let r = match target.spec_record(&name)? {
            None => r,
            Some(have) if crate::specs::Record::from_json(&have)?.source == r.source => continue,
            Some(_) => {
                let mut i = 2;
                while target.spec_record(&format!("{name} {i}"))?.is_some() {
                    i += 1;
                }
                r.renamed(&format!("{name} {i}"))?
            }
        };
        target.put_spec_record(None, &r.name, &r.to_json()?)?;
        w.models += 1;
    }
    for p in src.pictures()? {
        if let Some(b) = src.picture_bytes(&p.id)?
            && target.put_picture(&p, &b)?
        {
            w.pictures += 1;
        }
    }
    for (list, id) in src.picture_list_rows()? {
        if target.picture(&id)?.is_some() {
            target.tag_picture(&id, &list, true)?;
        }
    }
    Ok(w)
}
