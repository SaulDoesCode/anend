//! Everything the views read and change goes through a [`Backend`]. A backend answers one
//! call: an operation of the app API (docs/app-api.md) with JSON arguments. The typed
//! methods below are what the views use; they build on that call and can be overridden.
//!
//! The backends:
//! - `local::Local` (the desktop): anend-core's `Store` on this machine, every call on a
//!   worker thread so the interface never waits on the disk.
//! - `hub::Hub` (the web build): the hub's app API over a WebSocket (`/app/ws`).
//!
//! **Switching to the entry graph** (core/src/graph, being built): write a backend whose
//! `call` answers the same operations from the graph store (or make `Local` dispatch to it),
//! and pick it in `main.rs`. The views don't change. **The core in WebAssembly** slots in
//! the same way: `Local`'s dispatcher compiles to wasm32 (anend-core does), over a store on
//! OPFS/IndexedDB, picked in `main.rs` when the page has no hub.

#[cfg(not(target_arch = "wasm32"))]
pub mod local;

#[cfg(target_arch = "wasm32")]
pub mod hub;

use futures_util::future::LocalBoxFuture;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::gem::Mode;

pub type Fut<T> = LocalBoxFuture<'static, Result<T, String>>;

fn typed<T: DeserializeOwned + 'static>(f: Fut<Value>) -> Fut<T> {
    Box::pin(async move { serde_json::from_value(f.await?).map_err(|e| format!("an answer that doesn't read: {e}")) })
}

fn field<T: DeserializeOwned + 'static>(f: Fut<Value>, name: &'static str) -> Fut<T> {
    Box::pin(async move {
        let mut v = f.await?;
        serde_json::from_value(v.get_mut(name).map(Value::take).unwrap_or(Value::Null))
            .map_err(|e| format!("an answer that doesn't read: {e}"))
    })
}

fn unit(f: Fut<Value>) -> Fut<()> {
    Box::pin(async move { f.await.map(|_| ()) })
}

// ------------------------------------------------------------------ what calls answer

/// a memory or a note as the views show it; `key` is weum's `0,12` or `note:<key>`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Hit {
    pub key: String,
    pub text: String,
    #[serde(default)]
    pub note: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Counts {
    #[serde(default)]
    pub notes: u64,
    #[serde(default)]
    pub memories: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Hello {
    pub save: String,
    #[serde(default)]
    pub saves: Vec<String>,
    #[serde(default)]
    pub counts: Counts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SaveInfo {
    pub name: String,
    #[serde(default)]
    pub open: bool,
    pub notes: Option<u64>,
    pub memories: Option<u64>,
    #[serde(default)]
    pub size: u64,
    pub modified: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NoteSummary {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub modified: String,
    #[serde(default)]
    pub edits: usize,
    #[serde(default)]
    pub preview: String,
    #[serde(default)]
    pub words: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Note {
    #[serde(rename = "_key", default)]
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub markdown: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub edits: Vec<String>,
}

/// what to find in the query model (`entries.query`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EntryQuery {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub words: String,
    /// ordinal or reversed, with `from`/`to`
    pub bucket: String,
    pub from: Option<i64>,
    pub to: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// notes, memories, or anything
    pub kind: String,
    pub limit: usize,
}

/// which notes: words, tags (all, or any), and a time range (RFC 3339, created)
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NoteQuery {
    pub q: String,
    pub tags: Vec<String>,
    pub any: bool,
    pub since: Option<String>,
    pub until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ListInfo {
    pub list: String,
    pub name: String,
    #[serde(default)]
    pub members: Vec<Hit>,
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub ordinal_sum: i64,
    #[serde(default)]
    pub reversed_sum: i64,
    #[serde(default)]
    pub ordinal_mean: f64,
    #[serde(default)]
    pub reversed_mean: f64,
    #[serde(default)]
    pub vector: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Lists {
    pub lists: Vec<ListInfo>,
    #[serde(default)]
    pub dot: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SpreadNode {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    #[serde(default)]
    pub s: i64,
    #[serde(default)]
    pub r: i64,
    #[serde(default)]
    pub q: i64,
    #[serde(default)]
    pub hue: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Spread {
    pub d: f64,
    pub nodes: Vec<SpreadNode>,
    #[serde(default)]
    pub buckets: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Stats {
    #[serde(default)]
    pub notes: u64,
    #[serde(default)]
    pub words: u64,
    #[serde(default)]
    pub saves: u64,
    #[serde(default)]
    pub memories: u64,
    #[serde(default)]
    pub tags: Vec<(String, u64)>,
    pub first: Option<String>,
    pub latest: Option<String>,
}

/// one sum and how many texts it holds, in a mode
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct BucketCount {
    pub sum: i64,
    pub count: u64,
}

/// a model (today: a classifier) as the models view lists it; the rest of its details
/// stay JSON until the declarative model spec lands (docs/models.md)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelSummary {
    pub name: String,
    #[serde(default)]
    pub classes: Vec<String>,
    #[serde(default)]
    pub lists: Vec<String>,
    #[serde(default)]
    pub head: String,
    #[serde(default)]
    pub preset: String,
    #[serde(default)]
    pub trained: bool,
    pub accuracy: Option<f64>,
    pub examples: Option<u64>,
    #[serde(default)]
    pub training: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Job {
    #[serde(default)]
    pub name: String,
    pub state: String,
    #[serde(default)]
    pub epoch: usize,
    #[serde(default)]
    pub step: usize,
    #[serde(default)]
    pub steps: usize,
    #[serde(default)]
    pub loss: f64,
    #[serde(default)]
    pub seconds: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Scored {
    pub key: Option<String>,
    pub text: String,
    #[serde(default)]
    pub note: bool,
    #[serde(default)]
    pub prob: f64,
    #[serde(default)]
    pub score: f64,
    pub top_class: Option<String>,
    #[serde(default)]
    pub member: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FilterPreset {
    pub name: String,
    pub expr: Value,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub reads: String,
    pub count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FilterRun {
    #[serde(default)]
    pub scanned: u64,
    #[serde(default)]
    pub matched: u64,
    #[serde(default)]
    pub results: Vec<Scored>,
}

/// what a backup wrote (the desktop: two files) or holds (the web: two downloads)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Backup {
    /// where the files went, when the backend wrote them
    #[serde(default)]
    pub paths: Vec<String>,
    /// every text, weum's `[[text, [key bytes]]]`
    #[serde(default)]
    pub texts: Value,
    /// everything else: notes, lists, settings, models
    #[serde(default)]
    pub other: Value,
}

/// what an import put in
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Imported {
    pub save: Option<String>,
    #[serde(default)]
    pub memories: u64,
    #[serde(default)]
    pub notes: u64,
    #[serde(default)]
    pub lists: u64,
}

// ------------------------------------------------------------------ the backend

/// the app's data, wherever it lives
pub trait Backend {
    /// one operation of the app API (docs/app-api.md); every typed call goes through it
    fn call(&self, op: &str, args: Value) -> Fut<Value>;

    /// where the data is, for the status line ("local", "hub at …")
    fn describe(&self) -> String;

    /// whether the saves are files on this machine (backups go to a folder, imports read
    /// paths); on the web they're downloads and uploads
    fn on_disk(&self) -> bool {
        false
    }

    // ---- the save
    fn hello(&self) -> Fut<Hello> {
        typed(self.call("hello", json!({})))
    }
    fn saves(&self) -> Fut<Vec<SaveInfo>> {
        typed(self.call("saves.list", json!({})))
    }
    fn open_save(&self, name: &str) -> Fut<()> {
        unit(self.call("saves.open", json!({ "name": name })))
    }
    fn new_save(&self, name: &str) -> Fut<()> {
        unit(self.call("saves.new", json!({ "name": name })))
    }
    fn copy_save(&self, name: &str, to: &str) -> Fut<()> {
        unit(self.call("saves.copy", json!({ "name": name, "to": to })))
    }
    fn rename_save(&self, name: &str, to: &str) -> Fut<()> {
        unit(self.call("saves.rename", json!({ "name": name, "to": to })))
    }
    fn delete_save(&self, name: &str) -> Fut<()> {
        unit(self.call("saves.delete", json!({ "name": name })))
    }
    /// a bundle into a save: `merge` into the open one, or a new save named `name`
    fn import_bundle(&self, bundle: Value, name: &str, merge: bool) -> Fut<Imported> {
        typed(self.call(
            "saves.import",
            json!({ "bundle": bundle, "name": name, "merge": merge }),
        ))
    }
    /// weum's `await memory.backup`: the texts in one JSON, everything else in another;
    /// on disk the backend writes the two files (their paths come back), a hub hands the
    /// two documents back to be downloaded
    fn backup(&self, name: &str) -> Fut<Backup> {
        typed(self.call("saves.backup", json!({ "name": name })))
    }
    /// weum's `all-of-it.json` (or a backup's texts.json) into the open save, or a new one
    /// named `new_save`
    fn import_texts(&self, json_text: &str, new_save: &str) -> Fut<Imported> {
        typed(self.call("saves.import_texts", json!({ "data": json_text, "name": new_save })))
    }

    // ---- texts (memories) and sums
    fn gsearch(&self, mode: Mode, key: &str) -> Fut<Vec<Hit>> {
        field(
            self.call("mem.gsearch", json!({ "bucket": mode.name(), "key": key })),
            "hits",
        )
    }
    fn search(&self, q: &str, variations: bool) -> Fut<Vec<Hit>> {
        typed(self.call("mem.search", json!({ "q": q, "variations": variations })))
    }
    /// remembers a text unless it's there already; its key and whether it's new
    fn saturate(&self, text: &str) -> Fut<(String, bool)> {
        let f = self.call("mem.saturate", json!({ "text": text }));
        Box::pin(async move {
            let v = f.await?;
            Ok((
                v["key"].as_str().unwrap_or_default().to_string(),
                v["new"].as_bool().unwrap_or(false),
            ))
        })
    }
    /// forgets a memory; its text
    fn forget(&self, key: &str) -> Fut<String> {
        field(self.call("mem.forget", json!({ "key": key })), "text")
    }
    fn remember(&self, key: &str) -> Fut<Hit> {
        typed(self.call("mem.remember", json!({ "key": key })))
    }
    /// a memory's text replaced, keeping its key (or a forgotten one put back)
    fn put_memory(&self, key: &str, text: &str) -> Fut<()> {
        unit(self.call("mem.put", json!({ "key": key, "text": text })))
    }
    fn all_memories(&self, limit: usize) -> Fut<Vec<Hit>> {
        typed(self.call("mem.all", json!({ "limit": limit })))
    }
    /// every sum in a mode and how many texts it holds, lowest first
    fn bucket_counts(&self, mode: Mode) -> Fut<Vec<BucketCount>> {
        field(self.call("mem.buckets", json!({ "bucket": mode.name() })), "buckets")
    }

    // ---- lists (and tags on texts, which are lists headed by the tag)
    fn lists(&self) -> Fut<Lists> {
        typed(self.call("lists.stats", json!({})))
    }
    fn enlist(&self, list: &str, member: &str) -> Fut<()> {
        unit(self.call("lists.enlist", json!({ "list": list, "member": member })))
    }
    fn delist(&self, list: &str, member: &str) -> Fut<()> {
        unit(self.call("lists.delist", json!({ "list": list, "member": member })))
    }
    fn unlist(&self, list: &str) -> Fut<u64> {
        field(self.call("lists.unlist", json!({ "list": list })), "removed")
    }

    // ---- tags on any entry
    fn tag(&self, key: &str, tag: &str) -> Fut<()> {
        unit(self.call("tags.add", json!({ "key": key, "tag": tag })))
    }
    fn untag(&self, key: &str, tag: &str) -> Fut<()> {
        unit(self.call("tags.remove", json!({ "key": key, "tag": tag })))
    }
    /// every tag and how many entries carry it, the most used first
    fn all_tags(&self) -> Fut<Vec<(String, u64)>> {
        let f = self.call("tags.all", json!({}));
        Box::pin(async move {
            let v = f.await?;
            Ok(v.as_array()
                .into_iter()
                .flatten()
                .map(|t| {
                    (
                        t["tag"].as_str().unwrap_or("").to_string(),
                        t["count"].as_u64().unwrap_or(0),
                    )
                })
                .collect())
        })
    }
    /// the entries carrying all (or any) of these tags
    fn tagged(&self, tags: &[String], any: bool) -> Fut<Vec<Hit>> {
        typed(self.call(
            "tags.entries",
            json!({ "tags": tags, "mode": if any { "or" } else { "and" } }),
        ))
    }
    /// weum's notes on an entry, oldest first
    fn notes_on(&self, key: &str) -> Fut<Vec<Hit>> {
        typed(self.call("notes.on", json!({ "key": key })))
    }
    /// the query model: words, a sum range, a creation time range, a tag, a kind
    fn query(&self, q: &EntryQuery) -> Fut<(u64, Vec<Hit>)> {
        let f = self.call("entries.query", serde_json::to_value(q).unwrap_or_default());
        Box::pin(async move {
            let mut v = f.await?;
            let hits = serde_json::from_value(v["hits"].take()).map_err(|e| e.to_string())?;
            Ok((v["total"].as_u64().unwrap_or(0), hits))
        })
    }

    // ---- notes
    fn notes(&self, q: &NoteQuery) -> Fut<Vec<NoteSummary>> {
        let f = self.call(
            "notes.list",
            json!({ "q": q.q, "tags": q.tags, "mode": if q.any { "or" } else { "and" } }),
        );
        let (since, until) = (q.since.clone(), q.until.clone());
        Box::pin(async move {
            let mut all: Vec<NoteSummary> =
                serde_json::from_value(f.await?).map_err(|e| format!("an answer that doesn't read: {e}"))?;
            // the time range, here so every backend has it
            all.retain(|n| {
                since.as_deref().is_none_or(|s| n.created.as_str() >= s)
                    && until.as_deref().is_none_or(|u| n.created.as_str() <= u)
            });
            Ok(all)
        })
    }
    fn note_tags(&self) -> Fut<Vec<(String, u64)>> {
        let f = self.call("notes.tags", json!({}));
        Box::pin(async move {
            let v = f.await?;
            Ok(v.as_array()
                .into_iter()
                .flatten()
                .map(|t| {
                    (
                        t["tag"].as_str().unwrap_or("").to_string(),
                        t["count"].as_u64().unwrap_or(0),
                    )
                })
                .collect())
        })
    }
    fn note(&self, key: &str) -> Fut<Note> {
        typed(self.call("notes.get", json!({ "key": key })))
    }
    /// stores a note from the editor's text (`# title` and markdown)
    fn save_note(&self, note: &Note, text: &str) -> Fut<Note> {
        typed(self.call("notes.save", json!({ "note": note, "text": text })))
    }
    fn delete_note(&self, key: &str) -> Fut<()> {
        unit(self.call("notes.delete", json!({ "key": key })))
    }

    // ---- the space
    fn spread(&self, texts: &[(String, String)], reversed: bool) -> Fut<Spread> {
        let texts: Vec<Value> = texts.iter().map(|(id, t)| json!({ "id": id, "text": t })).collect();
        typed(self.call("space.spread", json!({ "texts": texts, "reversed": reversed })))
    }

    // ---- the whole app
    fn stats(&self) -> Fut<Stats> {
        typed(self.call("stats", json!({})))
    }

    // ---- models (classifiers today), behind this seam until the model spec lands
    fn models(&self) -> Fut<Vec<ModelSummary>> {
        typed(self.call("classify.list", json!({})))
    }
    fn create_model(&self, name: &str, classes: &[String], preset: &str) -> Fut<()> {
        unit(self.call(
            "classify.create",
            json!({ "name": name, "classes": classes, "preset": preset }),
        ))
    }
    fn delete_model(&self, name: &str) -> Fut<()> {
        unit(self.call("classify.delete", json!({ "name": name })))
    }
    fn train_model(&self, name: &str, epochs: u32) -> Fut<Job> {
        typed(self.call("classify.train", json!({ "name": name, "epochs": epochs })))
    }
    fn model_job(&self, name: &str) -> Fut<Job> {
        typed(self.call("classify.job", json!({ "name": name })))
    }
    fn cancel_training(&self, name: &str) -> Fut<()> {
        unit(self.call("classify.cancel", json!({ "name": name })))
    }
    /// a model's outputs for a text: class names and probabilities
    fn run_model(&self, name: &str, text: &str) -> Fut<Value> {
        self.call("classify.predict", json!({ "name": name, "texts": [text] }))
    }
    /// the texts most likely in a class
    fn model_search(&self, name: &str, class: &str, q: &str) -> Fut<Vec<Scored>> {
        field(
            self.call(
                "classify.search",
                json!({ "name": name, "class": class, "q": q, "limit": 60 }),
            ),
            "results",
        )
    }
    /// yes or no on a text for a class
    fn model_feedback(&self, name: &str, key: Option<&str>, text: &str, class: &str, yes: bool) -> Fut<Value> {
        let mut args = json!({ "name": name, "class": class, "yes": yes });
        match key {
            Some(k) => args["key"] = json!(k),
            None => args["text"] = json!(text),
        }
        self.call("classify.feedback", args)
    }
    /// takes back the latest feedback on a model (the undo of yes/no/why)
    fn undo_model_feedback(&self, name: &str) -> Fut<()> {
        unit(self.call("classify.unfeedback", json!({ "name": name })))
    }
    fn model_enlist(&self, name: &str, class: &str) -> Fut<Value> {
        self.call("classify.enlist", json!({ "name": name, "class": class }))
    }

    // ---- filters
    fn filters(&self) -> Fut<Vec<FilterPreset>> {
        typed(self.call("filters.list", json!({})))
    }
    fn save_filter(&self, name: &str, expr: &Value, description: &str) -> Fut<()> {
        unit(self.call(
            "filters.save",
            json!({ "name": name, "expr": expr, "description": description }),
        ))
    }
    fn delete_filter(&self, name: &str) -> Fut<()> {
        unit(self.call("filters.delete", json!({ "name": name })))
    }
    fn run_filter(&self, expr: &Value) -> Fut<FilterRun> {
        typed(self.call("filters.run", json!({ "expr": expr, "limit": 300 })))
    }
    fn enlist_filter(&self, name: &str) -> Fut<Value> {
        self.call("filters.enlist", json!({ "name": name }))
    }

    // ---- settings kept in the save
    fn setting(&self, name: &str) -> Fut<Option<String>> {
        field(self.call("settings.get", json!({ "name": name })), "value")
    }
    fn set_setting(&self, name: &str, value: &str) -> Fut<()> {
        unit(self.call("settings.set", json!({ "name": name, "value": value })))
    }
}
