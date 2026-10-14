//! A save as plain JSON, keys and all: what the web app keeps in the browser when it runs
//! without a hub, and how a save moves between the browser and a hub's workspace.
//!
//! ```json
//! { "anend": 1, "name": "notes",
//!   "notes": [{ "_key": "1000000", "title": "...", "markdown": "...", "tags": [], "created": "...", "edits": [] }],
//!   "memories": [["listen", [0]], ["silent", [1]]],
//!   "lists": [["0", "1"], ["0", "note:1000000"]],
//!   "settings": { "space map: home": "7b22..." } }
//! ```
//!
//! Memories are weum's `[text, key bytes]`, list links are pairs of labels (`Ref::label`:
//! a memory's key bytes comma separated, or `note:<key>`), and settings are hex.
//!
//! `classifiers` is optional and left out when a save has none, so bundles from before
//! classifiers read as they did and the version stays 1: each is
//! `{definition, weights?, feedback?}` (`classifiers::Bundled`), the weights being the
//! `ANENDCLS` file in standard padded base64 (a third smaller than hex, and weights are
//! the one big thing in a bundle). `merge` (and the forge) leave classifiers out: their
//! classes are lists by key, and merged lists get fresh keys.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::forge::{self, Contents, Item};
use crate::memory::Ref;
use crate::note::Note;
use crate::store::{Store, StoreResult};

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Bundle {
    pub anend: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub memories: Vec<(String, Vec<u8>)>,
    #[serde(default)]
    pub lists: Vec<(String, String)>,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classifiers: Vec<crate::classifiers::Bundled>,
    /// saved filters (`filters::Preset`), left out when there are none
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<crate::filters::Preset>,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// everything in a save
pub fn export(store: &Store, name: &str) -> StoreResult<Bundle> {
    let mut settings = BTreeMap::new();
    for n in store.setting_names("")? {
        if let Some(v) = store.setting(&n)? {
            settings.insert(n, hex(&v));
        }
    }
    let mut memories = store.all_memories()?;
    memories.sort_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| a.1.cmp(&b.1)));
    Ok(Bundle {
        anend: VERSION,
        name: name.to_string(),
        notes: store.all()?,
        memories,
        lists: store.all_lists()?.iter().map(|(l, m)| (l.label(), m.label())).collect(),
        settings,
        classifiers: store.export_classifiers()?,
        filters: store.filter_presets()?,
    })
}

pub fn check(b: &Bundle) -> StoreResult<()> {
    if b.anend == 0 || b.anend > VERSION {
        return Err(format!(
            "that isn't an anend save bundle this version reads ({})",
            b.anend
        ));
    }
    Ok(())
}

/// fills an empty save with a bundle, keeping its keys
pub fn import(store: &Store, b: &Bundle) -> StoreResult<Written> {
    check(b)?;
    let memories = store.import_memories(b.memories.clone())?;
    let notes = store.import(b.notes.clone())?;
    let pairs: Vec<(Ref, Ref)> = b
        .lists
        .iter()
        .filter_map(|(l, m)| Some((Ref::parse(l)?, Ref::parse(m)?)))
        .collect();
    store.enlist_many(&pairs)?;
    for (n, v) in &b.settings {
        if let Some(v) = unhex(v) {
            store.set_setting(n, &v)?;
        }
    }
    store.import_classifiers(&b.classifiers)?;
    store.import_filters(&b.filters)?;
    Ok(Written {
        memories,
        notes,
        lists: pairs.len(),
    })
}

pub use forge::Written;

/// a bundle's contents by what they are, for merging into a save that has its own keys
pub fn contents(b: &Bundle) -> StoreResult<Contents> {
    check(b)?;
    let texts: HashMap<Vec<u8>, &str> = b.memories.iter().map(|(t, k)| (k.clone(), t.as_str())).collect();
    let notes: HashMap<&str, &Note> = b.notes.iter().map(|n| (n.key.as_str(), n)).collect();
    let item = |label: &str| match Ref::parse(label)? {
        Ref::Memory(k) => texts.get(&k).map(|t| Item::Memory(t.to_string())),
        Ref::Note(k) => notes.get(k.as_str()).map(|n| Item::Note(forge::note_id(n))),
    };
    let mut seen = std::collections::HashSet::new();
    Ok(Contents {
        memories: b
            .memories
            .iter()
            .map(|(t, _)| t.clone())
            .filter(|t| seen.insert(t.clone()))
            .collect(),
        notes: b.notes.clone(),
        lists: b.lists.iter().filter_map(|(l, m)| Some((item(l)?, item(m)?))).collect(),
    })
}

/// merges a bundle into a save: what's new gets fresh keys there
pub fn merge(store: &Store, b: &Bundle) -> StoreResult<Written> {
    forge::write_into(store, &contents(b)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("anend-bundle-{}-{}.redb", std::process::id(), name));
        let _ = std::fs::remove_file(&path);
        (Store::open(&path).unwrap(), path)
    }

    #[test]
    fn round_trip() {
        let (a, pa) = store("a");
        let l = a.saturate_new("listen").unwrap();
        let s = a.saturate_new("silent").unwrap();
        let n = a
            .save(&Note {
                title: "a note".into(),
                markdown: "body".into(),
                ..Default::default()
            })
            .unwrap();
        a.enlist(&Ref::Memory(l.clone()), &Ref::Memory(s)).unwrap();
        a.enlist(&Ref::Memory(l), &Ref::Note(n.key)).unwrap();
        a.set_setting("space map: home", b"{\"x\":1}").unwrap();
        let b = export(&a, "a").unwrap();
        let json = serde_json::to_string(&b).unwrap();
        let back: Bundle = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);

        let (c, pc) = store("c");
        let w = import(&c, &back).unwrap();
        assert_eq!((w.memories, w.notes, w.lists), (2, 1, 2));
        assert_eq!(export(&c, "a").unwrap(), b);

        // merging the same again adds nothing; into another save it all arrives
        assert_eq!(merge(&c, &b).unwrap().memories, 0);
        let (d, pd) = store("d");
        d.saturate_new("other").unwrap();
        let w = merge(&d, &b).unwrap();
        assert_eq!((w.memories, w.notes, w.lists), (2, 1, 2));
        assert!(import(&d, &Bundle::default()).is_err());
        for p in [pa, pc, pd] {
            let _ = std::fs::remove_file(p);
        }
    }
}
