//! The forge: saves as ingredients, combined into a new save the way Oblivion's altar
//! combines effects into a spell. Each ingredient has an effect:
//!
//! - **add**: its memories, notes and lists join the result
//! - **take away**: whatever it shares with the result leaves it (a subset)
//! - **keep shared**: only what the result shares with it stays
//!
//! applied in order, then the filters (words, sum ranges, which kinds of things) narrow
//! it. Saves are compared by content, not by key: a memory is its text, a note its title
//! and body, a list link the two things it joins. Writing gives everything fresh keys in
//! the target, so the same machinery merges a save into the open one.

use std::collections::{HashMap, HashSet};

use crate::memory::Ref;
use crate::note::Note;
use crate::spread;
use crate::store::{Store, StoreResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Effect {
    #[default]
    Add,
    TakeAway,
    KeepShared,
}

impl Effect {
    pub const ALL: [Effect; 3] = [Effect::Add, Effect::TakeAway, Effect::KeepShared];

    pub fn name(self) -> &'static str {
        match self {
            Effect::Add => "add",
            Effect::TakeAway => "take away",
            Effect::KeepShared => "keep shared",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Effect::Add => "+",
            Effect::TakeAway => "−",
            Effect::KeepShared => "∩",
        }
    }
}

/// a thing in a save, by what it is rather than where it's kept
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Memory(String),
    Note(String),
}

pub(crate) fn note_id(n: &Note) -> String {
    format!("{}\u{0}{}", n.title, n.markdown)
}

#[derive(Clone, Debug, Default)]
pub struct Contents {
    /// memory texts, each once, in the order they were kept
    pub memories: Vec<String>,
    pub notes: Vec<Note>,
    /// (list, member)
    pub lists: Vec<(Item, Item)>,
}

impl Contents {
    fn memory_set(&self) -> HashSet<&str> {
        self.memories.iter().map(String::as_str).collect()
    }

    fn note_set(&self) -> HashSet<String> {
        self.notes.iter().map(note_id).collect()
    }

    fn has(&self, i: &Item, mems: &HashSet<&str>, notes: &HashSet<String>) -> bool {
        match i {
            Item::Memory(t) => mems.contains(t.as_str()),
            Item::Note(n) => notes.contains(n),
        }
    }

    /// drops list links whose ends are gone
    fn tidy(&mut self) {
        let mems: HashSet<String> = self.memories.iter().cloned().collect();
        let notes = self.note_set();
        let mems: HashSet<&str> = mems.iter().map(String::as_str).collect();
        let lists = std::mem::take(&mut self.lists);
        let mut seen = HashSet::new();
        self.lists = lists
            .into_iter()
            .filter(|(l, m)| self.has(l, &mems, &notes) && self.has(m, &mems, &notes))
            .filter(|p| seen.insert(p.clone()))
            .collect();
    }
}

/// everything in a save
pub fn read(store: &Store) -> StoreResult<Contents> {
    let mut by_key: HashMap<Vec<u8>, String> = HashMap::new();
    let mut memories = Vec::new();
    let mut seen = HashSet::new();
    for (text, key) in store.all_memories()? {
        by_key.insert(key, text.clone());
        if seen.insert(text.clone()) {
            memories.push(text);
        }
    }
    let notes = store.all()?;
    let note_ids: HashMap<String, String> = notes.iter().map(|n| (n.key.clone(), note_id(n))).collect();
    let mut unique_notes = Vec::new();
    let mut seen_notes = HashSet::new();
    for n in notes {
        if seen_notes.insert(note_id(&n)) {
            unique_notes.push(n);
        }
    }
    let item = |r: &Ref| match r {
        Ref::Memory(k) => by_key.get(k).cloned().map(Item::Memory),
        Ref::Note(k) => note_ids.get(k).cloned().map(Item::Note),
    };
    let lists = store
        .all_lists()?
        .iter()
        .filter_map(|(l, m)| Some((item(l)?, item(m)?)))
        .collect();
    let mut c = Contents {
        memories,
        notes: unique_notes,
        lists,
    };
    c.tidy();
    Ok(c)
}

/// applies the ingredients in order to an empty result
pub fn combine(parts: &[(Effect, &Contents)]) -> Contents {
    let mut out = Contents::default();
    for (effect, c) in parts {
        let mems = c.memory_set();
        let notes = c.note_set();
        let links: HashSet<&(Item, Item)> = c.lists.iter().collect();
        match effect {
            Effect::Add => {
                let have: HashSet<String> = out.memories.iter().cloned().collect();
                out.memories
                    .extend(c.memories.iter().filter(|t| !have.contains(*t)).cloned());
                let have = out.note_set();
                out.notes
                    .extend(c.notes.iter().filter(|n| !have.contains(&note_id(n))).cloned());
                out.lists.extend(c.lists.iter().cloned());
            }
            Effect::TakeAway => {
                out.memories.retain(|t| !mems.contains(t.as_str()));
                out.notes.retain(|n| !notes.contains(&note_id(n)));
                out.lists.retain(|p| !links.contains(p));
            }
            Effect::KeepShared => {
                out.memories.retain(|t| mems.contains(t.as_str()));
                out.notes.retain(|n| notes.contains(&note_id(n)));
                out.lists.retain(|p| links.contains(p));
            }
        }
        out.tidy();
    }
    out
}

/// what the result is narrowed to
#[derive(Clone, Debug)]
pub struct Filter {
    /// words a memory or note must contain; `-word` ones it mustn't
    pub words: String,
    /// inclusive ranges of a memory's ordinal and reversed sums
    pub ordinal: (Option<i64>, Option<i64>),
    pub reversed: (Option<i64>, Option<i64>),
    pub memories: bool,
    pub notes: bool,
    pub lists: bool,
}

impl Default for Filter {
    fn default() -> Self {
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
    lo.is_none_or(|lo| v >= lo) && hi.is_none_or(|hi| v <= hi)
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

    /// whether the sum ranges narrow anything
    fn sums_set(&self) -> bool {
        self.ordinal != (None, None) || self.reversed != (None, None)
    }

    pub fn apply(&self, mut c: Contents) -> Contents {
        if !self.memories {
            c.memories.clear();
        }
        if !self.notes {
            c.notes.clear();
        }
        if !self.lists {
            c.lists.clear();
        }
        c.memories.retain(|t| {
            let (s, r) = spread::sums(t);
            self.words_match(t) && within(s, self.ordinal) && within(r, self.reversed)
        });
        // a sum range is about memories; notes only answer to words
        c.notes
            .retain(|n| self.words_match(&format!("{} {}", n.title, n.markdown)) && !self.sums_set());
        c.tidy();
        c
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Written {
    pub memories: usize,
    pub notes: usize,
    pub lists: usize,
}

/// puts contents into a save, skipping what it already has; everything gets fresh keys
pub fn write_into(target: &Store, c: &Contents) -> StoreResult<Written> {
    // memories
    let mut keys: HashMap<String, Vec<u8>> = HashMap::new();
    for (text, key) in target.all_memories()? {
        keys.entry(text).or_insert(key);
    }
    let fresh: Vec<String> = c.memories.iter().filter(|t| !keys.contains_key(*t)).cloned().collect();
    for (t, k) in fresh.iter().zip(target.saturate_many(&fresh)?) {
        keys.insert(t.clone(), k);
    }

    // notes: new keys after the highest one there
    let existing = target.all()?;
    let mut note_keys: HashMap<String, String> = existing.iter().map(|n| (note_id(n), n.key.clone())).collect();
    let mut next = existing
        .iter()
        .filter_map(|n| n.key.parse::<u64>().ok())
        .max()
        .map_or(1_000_000, |k| k + 1)
        .max(1_000_000);
    let mut new_notes = Vec::new();
    for n in &c.notes {
        let id = note_id(n);
        if note_keys.contains_key(&id) {
            continue;
        }
        let mut n = n.clone();
        n.key = next.to_string();
        next += 1;
        note_keys.insert(id, n.key.clone());
        new_notes.push(n);
    }
    let notes = new_notes.len();
    target.import(new_notes)?;

    // lists, between the keys things have now
    let to_ref = |i: &Item| match i {
        Item::Memory(t) => keys.get(t).map(|k| Ref::Memory(k.clone())),
        Item::Note(id) => note_keys.get(id).map(|k| Ref::Note(k.clone())),
    };
    let have: HashSet<(Ref, Ref)> = target.all_lists()?.into_iter().collect();
    let pairs: Vec<(Ref, Ref)> = c
        .lists
        .iter()
        .filter_map(|(l, m)| Some((to_ref(l)?, to_ref(m)?)))
        .filter(|p| !have.contains(p))
        .collect();
    target.enlist_many(&pairs)?;
    Ok(Written {
        memories: fresh.len(),
        notes,
        lists: pairs.len(),
    })
}

/// copies the space's maps and settings from one save to another (a forged save takes
/// its first ingredient's)
pub fn copy_settings(from: &Store, to: &Store) -> StoreResult<()> {
    for name in from.setting_names("")? {
        if let Some(v) = from.setting(&name)? {
            to.set_setting(&name, &v)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("anend-forge-{}-{}.redb", std::process::id(), name));
        let _ = std::fs::remove_file(&path);
        (Store::open(&path).unwrap(), path)
    }

    fn note(title: &str) -> Note {
        Note {
            title: title.into(),
            markdown: format!("about {title}"),
            ..Default::default()
        }
    }

    fn fill(s: &Store, mems: &[&str], notes: &[&str]) {
        for m in mems {
            s.saturate_new(m).unwrap();
        }
        for n in notes {
            s.save(&note(n)).unwrap();
        }
    }

    #[test]
    fn effects_and_filters() {
        let (a, pa) = store("a");
        let (b, pb) = store("b");
        fill(&a, &["listen", "silent", "truth", "truth"], &["one", "two"]);
        fill(&b, &["silent", "enlist", "stone"], &["two", "three"]);
        // a list in a: listen → truth
        let key = |s: &Store, t: &str| s.find_exact(t).unwrap().map(Ref::Memory).unwrap();
        a.enlist(&key(&a, "listen"), &key(&a, "truth")).unwrap();
        a.enlist(&key(&a, "silent"), &key(&a, "truth")).unwrap();

        let ca = read(&a).unwrap();
        let cb = read(&b).unwrap();
        assert_eq!(ca.memories, ["listen", "silent", "truth"]);
        assert_eq!(ca.lists.len(), 2);

        let union = combine(&[(Effect::Add, &ca), (Effect::Add, &cb)]);
        assert_eq!(union.memories, ["listen", "silent", "truth", "enlist", "stone"]);
        assert_eq!(union.notes.len(), 3);

        let subset = combine(&[(Effect::Add, &ca), (Effect::TakeAway, &cb)]);
        assert_eq!(subset.memories, ["listen", "truth"]);
        assert_eq!(
            subset.notes.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(),
            ["one"]
        );
        // silent left, so its list link did too
        assert_eq!(
            subset.lists,
            [(Item::Memory("listen".into()), Item::Memory("truth".into()))]
        );

        let shared = combine(&[(Effect::Add, &ca), (Effect::KeepShared, &cb)]);
        assert_eq!(shared.memories, ["silent"]);
        assert_eq!(shared.notes.len(), 1);
        assert!(shared.lists.is_empty());

        let f = Filter {
            words: "-tru".into(),
            ..Default::default()
        };
        assert_eq!(f.apply(union.clone()).memories, ["listen", "silent", "enlist", "stone"]);
        let f = Filter {
            ordinal: (Some(79), Some(79)),
            ..Default::default()
        };
        assert_eq!(f.apply(union.clone()).memories, ["listen", "silent", "enlist"]);
        assert!(f.apply(union.clone()).notes.is_empty());
        let f = Filter {
            notes: false,
            ..Default::default()
        };
        assert!(f.apply(union.clone()).notes.is_empty());

        // forging the subset into a fresh save
        let (out, po) = store("out");
        let w = write_into(&out, &subset).unwrap();
        assert_eq!(
            w,
            Written {
                memories: 2,
                notes: 1,
                lists: 1
            }
        );
        let back = read(&out).unwrap();
        assert_eq!(back.memories, ["listen", "truth"]);
        assert_eq!(back.lists, subset.lists);
        assert!(
            !out.gsearch(crate::memory::Bucket::Ordinal, &crate::memory::BucketKey::Sum(87))
                .unwrap()
                .is_empty()
        );

        // merging b into a: only what's new goes in, and again adds nothing
        let w = write_into(&a, &cb).unwrap();
        assert_eq!(
            w,
            Written {
                memories: 2,
                notes: 1,
                lists: 0
            }
        );
        assert_eq!(write_into(&a, &cb).unwrap(), Written::default());
        assert_eq!(read(&a).unwrap().memories.len(), 5);
        let keys: HashSet<String> = a.all().unwrap().into_iter().map(|n| n.key).collect();
        assert_eq!(keys.len(), 3, "merged notes get keys of their own");

        drop((a, b, out));
        for p in [pa, pb, po] {
            let _ = std::fs::remove_file(p);
        }
    }
}
