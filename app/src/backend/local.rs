//! The desktop's backend: anend-core's `Store` on this machine. Every call runs on a
//! worker thread and answers through a channel, so the interface never waits on the disk.
//! The operations are the hub's (hub/src/app.rs, hub/src/classify.rs), answered the same
//! way, plus a few the hub doesn't have yet: `mem.put`, `mem.buckets`, `settings.get`,
//! `settings.set`, `classify.unfeedback`.
//!
//! When the entry graph store lands, `dispatch` is the one place that changes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anend_core::classifiers::{self as clfs, Judged, TrainArgs};
use anend_core::classify::{Flow, HeadKind, Pool};
use anend_core::memory::{self, Bucket, BucketKey, Hit, Ref};
use anend_core::note::Note;
use anend_core::spread::{self, Params, Spread};
use anend_core::store::Store;
use anend_core::{bundle, filters};
use redb::{ReadableDatabase, ReadableMultimapTable};
use serde_json::{Value, json};

use super::{Backend, Fut};

type Res = Result<Value, String>;

/// how many memories and notes a model search or a filter looks through
const SCAN: usize = 20_000;

struct Open {
    name: String,
    store: Arc<Store>,
}

/// the saves folder and the open save
#[derive(Clone)]
pub struct Local {
    dir: PathBuf,
    open: Arc<Mutex<Open>>,
    jobs: Arc<Mutex<HashMap<String, Arc<JobCell>>>>,
}

#[derive(Default)]
struct JobCell {
    cancel: AtomicBool,
    state: Mutex<Value>,
}

impl Local {
    /// opens `save` (a path), or `notes.redb` in `dir`
    pub fn open(dir: &Path, save: Option<&Path>) -> Result<Local, String> {
        let path = save.map(Path::to_path_buf).unwrap_or_else(|| dir.join("notes.redb"));
        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dir.to_path_buf());
        let store = Store::open(&path)?;
        Ok(Local {
            dir,
            open: Arc::new(Mutex::new(Open {
                name: save_name(&path),
                store: Arc::new(store),
            })),
            jobs: Default::default(),
        })
    }

    fn current(&self) -> (String, Arc<Store>) {
        let o = self.open.lock().unwrap_or_else(|e| e.into_inner());
        (o.name.clone(), o.store.clone())
    }

    fn path(&self, name: &str) -> Result<PathBuf, String> {
        Ok(self.dir.join(format!("{}.redb", clean(name)?)))
    }

    fn switch(&self, name: &str) -> Result<(), String> {
        let path = self.path(name)?;
        let store = Store::open(&path)?;
        let mut o = self.open.lock().unwrap_or_else(|e| e.into_inner());
        *o = Open {
            name: save_name(&path),
            store: Arc::new(store),
        };
        Ok(())
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "redb"))
            .map(|p| save_name(&p))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    /// where the open save is, for the status line
    pub fn save_path(&self) -> PathBuf {
        self.current().1.path().to_path_buf()
    }
}

impl Backend for Local {
    fn call(&self, op: &str, args: Value) -> Fut<Value> {
        let me = self.clone();
        let op = op.to_string();
        let (tx, rx) = futures_channel::oneshot::channel();
        let spawned = std::thread::Builder::new().name(format!("anend {op}")).spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| me.dispatch(&op, &args)))
                .unwrap_or_else(|_| Err(format!("{op} failed unexpectedly")));
            let _ = tx.send(r);
        });
        Box::pin(async move {
            spawned.map_err(|e| format!("couldn't start a worker: {e}"))?;
            rx.await.map_err(|_| "the worker stopped".to_string())?
        })
    }

    fn describe(&self) -> String {
        format!("{}", self.save_path().display())
    }

    fn on_disk(&self) -> bool {
        true
    }
}

// ------------------------------------------------------------------ helpers

fn save_name(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn clean(name: &str) -> Result<String, String> {
    let n: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let n = n.trim_matches('.').trim().to_string();
    if n.is_empty() || n.len() > 96 {
        Err("a save's name: letters, numbers, spaces, - _ and ., at most 96".into())
    } else {
        Ok(n)
    }
}

fn s<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(Value::as_str).unwrap_or("")
}

fn hit_json(h: &Hit) -> Value {
    let (o, r) = spread::sums(&h.text);
    json!({ "key": h.r.label(), "text": h.text, "note": matches!(h.r, Ref::Note(_)), "ordinal": o, "reversed": r })
}

fn hits(v: &[Hit]) -> Value {
    Value::Array(v.iter().map(hit_json).collect())
}

fn parse_ref(label: &str) -> Result<Ref, String> {
    Ref::parse(label).ok_or_else(|| format!("{label:?} isn't a key (like 0,12 or note:1000001)"))
}

fn bucket(name: &str) -> Bucket {
    match name {
        "reversed" => Bucket::Reversed,
        "atbash" => Bucket::Atbash,
        "frequency" => Bucket::Frequency,
        _ => Bucket::Ordinal,
    }
}

fn note_summary(n: &Note) -> Value {
    let preview: String = n.markdown.chars().take(160).collect();
    json!({
        "key": n.key, "title": n.display_title(), "tags": n.tags, "created": n.created,
        "modified": n.edits.last().unwrap_or(&n.created), "edits": n.edits.len(),
        "preview": preview, "words": n.markdown.split_whitespace().count(),
    })
}

fn tag_counts(notes: &[Note]) -> Vec<(String, usize)> {
    let mut tags: HashMap<String, usize> = HashMap::new();
    for n in notes {
        for t in &n.tags {
            *tags.entry(t.clone()).or_default() += 1;
        }
    }
    let mut v: Vec<(String, usize)> = tags.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool::new(anend_core::classify::pool::available()))
}

// ------------------------------------------------------------------ the operations

impl Local {
    fn dispatch(&self, op: &str, args: &Value) -> Res {
        if op.starts_with("saves.") {
            return self.saves_call(op, args);
        }
        let (open, store) = self.current();
        if op.starts_with("classify.") || op.starts_with("filters.") {
            return self.models_call(&store, op, args);
        }
        let st: &Store = &store;
        match op {
            "hello" => {
                let (notes, memories) = st.counts();
                Ok(json!({ "save": open, "saves": self.names(), "counts": { "notes": notes, "memories": memories } }))
            }

            // ---- notes
            "notes.list" => {
                let keep: Option<HashSet<String>> = match s(args, "q").trim() {
                    "" => None,
                    q => Some(st.note_filter(q)?),
                };
                let tags: Vec<&str> = args
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                let any = s(args, "mode") == "or";
                let tagged = |n: &Note| {
                    tags.is_empty()
                        || if any {
                            tags.iter().any(|t| n.tags.iter().any(|x| x == t))
                        } else {
                            tags.iter().all(|t| n.tags.iter().any(|x| x == t))
                        }
                };
                Ok(Value::Array(
                    st.all()?
                        .iter()
                        .filter(|n| keep.as_ref().is_none_or(|k| k.contains(&n.key)) && tagged(n))
                        .map(note_summary)
                        .collect(),
                ))
            }
            "notes.tags" => Ok(json!(
                tag_counts(&st.all()?)
                    .into_iter()
                    .map(|(tag, count)| json!({ "tag": tag, "count": count }))
                    .collect::<Vec<_>>()
            )),
            "notes.get" => {
                let key = s(args, "key");
                let n = st.all()?.into_iter().find(|n| n.key == key).ok_or("no such note")?;
                serde_json::to_value(n).map_err(|e| e.to_string())
            }
            "notes.save" => {
                let mut note: Note = serde_json::from_value(args.get("note").cloned().unwrap_or(Value::Null))
                    .map_err(|e| format!("that isn't a note: {e}"))?;
                if let Some(text) = args.get("text").and_then(Value::as_str) {
                    let (t, m) = anend_core::note::split(text);
                    note.title = t;
                    note.markdown = m;
                }
                if note.title.trim().is_empty() {
                    return Err("a note needs a # title".into());
                }
                if !note.key.is_empty()
                    && let Some(old) = st.all()?.into_iter().find(|n| n.key == note.key)
                {
                    note.created = old.created;
                    note.edits = old.edits;
                }
                serde_json::to_value(st.save(&note)?).map_err(|e| e.to_string())
            }
            "notes.delete" => {
                st.delete(s(args, "key"))?;
                Ok(Value::Null)
            }

            // ---- memories
            "mem.gsearch" => {
                let b = bucket(s(args, "bucket"));
                let key = match (b, args.get("key")) {
                    (Bucket::Frequency, Some(Value::String(t))) => memory::measure(t).key(b),
                    (_, Some(Value::Number(n))) => BucketKey::Sum(n.as_u64().unwrap_or(0) as u32),
                    (_, Some(Value::String(t))) => match t.trim().parse::<u32>() {
                        Ok(n) if b != Bucket::Frequency => BucketKey::Sum(n),
                        _ => memory::measure(t).key(b),
                    },
                    _ => return Err("gsearch needs a key: a number, or a text to measure".into()),
                };
                Ok(json!({ "bucket": b.name(), "hits": hits(&st.gsearch(b, &key)?) }))
            }
            "mem.search" => {
                let q = s(args, "q");
                let found = if args.get("variations").and_then(Value::as_bool).unwrap_or(true) {
                    st.search_with_variations(q)?
                } else {
                    st.search(q)?
                };
                Ok(hits(&found))
            }
            "mem.saturate" => {
                let text = s(args, "text").trim();
                if text.is_empty() {
                    return Err("nothing to remember".into());
                }
                let (key, new) = st.saturate(text)?;
                Ok(json!({ "key": Ref::Memory(key).label(), "new": new }))
            }
            "mem.forget" => {
                let Ref::Memory(k) = parse_ref(s(args, "key"))? else {
                    return Err("notes are deleted from the editor".into());
                };
                Ok(json!({ "text": st.forget(&k)? }))
            }
            "mem.remember" => Ok(hit_json(&st.remember(&parse_ref(s(args, "key"))?)?)),
            "mem.put" => {
                let Ref::Memory(k) = parse_ref(s(args, "key"))? else {
                    return Err("a note's text is edited in the editor".into());
                };
                let text = s(args, "text");
                if text.trim().is_empty() {
                    return Err("an empty text: forget it instead".into());
                }
                // replaces (and re-indexes) the text under the same key, or puts a forgotten
                // one back
                if st.remember(&Ref::Memory(k.clone())).is_ok() {
                    st.import_memories(vec![(text.to_string(), k)])?;
                } else {
                    st.restore_memory(&k, text)?;
                }
                Ok(Value::Null)
            }
            "mem.all" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(5000) as usize;
                Ok(Value::Array(
                    st.all_memories()?
                        .into_iter()
                        .take(limit)
                        .map(|(t, k)| {
                            hit_json(&Hit {
                                r: Ref::Memory(k),
                                text: t,
                            })
                        })
                        .collect(),
                ))
            }
            "mem.buckets" => {
                let table = match bucket(s(args, "bucket")) {
                    Bucket::Reversed => memory::REVERSED,
                    Bucket::Atbash | Bucket::Frequency => memory::ATBASH,
                    Bucket::Ordinal => memory::ORDINAL,
                };
                let txn = st.db.begin_read().map_err(|e| e.to_string())?;
                let t = txn.open_multimap_table(table).map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for item in t.iter().map_err(|e| e.to_string())? {
                    let (k, vs) = item.map_err(|e| e.to_string())?;
                    out.push(json!({ "sum": k.value(), "count": vs.len() }));
                }
                Ok(json!({ "buckets": out }))
            }

            // ---- lists
            "lists.stats" => {
                let lists = anend_core::lists::stats(st)?;
                let dot = if lists.len() <= 1000 {
                    anend_core::lists::dots(&lists)
                } else {
                    Vec::new()
                };
                let lists: Vec<Value> = lists
                    .iter()
                    .map(|l| {
                        json!({
                            "list": l.list.label(), "name": l.name, "members": hits(&l.members),
                            "count": l.members.len(), "ordinal_sum": l.ordinal_sum, "reversed_sum": l.reversed_sum,
                            "ordinal_mean": l.ordinal_mean(), "reversed_mean": l.reversed_mean(),
                            "vector": l.vector.to_vec(),
                        })
                    })
                    .collect();
                Ok(json!({ "lists": lists, "dot": dot }))
            }
            "lists.enlist" => {
                st.enlist(&parse_ref(s(args, "list"))?, &parse_ref(s(args, "member"))?)?;
                Ok(Value::Null)
            }
            "lists.delist" => {
                st.delist(&parse_ref(s(args, "list"))?, &parse_ref(s(args, "member"))?)?;
                Ok(Value::Null)
            }
            "lists.unlist" => Ok(json!({ "removed": st.unlist(&parse_ref(s(args, "list"))?)? })),

            // ---- the space
            "space.spread" => {
                let p = Params {
                    reversed: args.get("reversed").and_then(Value::as_bool).unwrap_or(false),
                    ..Params::default()
                };
                let mut sp = Spread::new(p);
                let items = args.get("texts").and_then(Value::as_array).cloned().unwrap_or_default();
                if items.len() > 2000 {
                    return Err("at most 2000 texts in a spread".into());
                }
                for it in &items {
                    let text = s(it, "text");
                    let hw = (text.chars().count() as f64 * 0.27 + 0.1).min(12.0);
                    sp.add(Spread::node(s(it, "id"), text, hw, 0.35));
                }
                sp.rekey();
                sp.wake(1.0);
                sp.settle(4000, 400);
                let nodes: Vec<Value> = sp
                    .nodes
                    .iter()
                    .map(|n| {
                        json!({ "id": n.id, "x": n.x, "y": n.y, "z": n.z, "s": n.s, "r": n.r, "q": n.q,
                                "hue": spread::seed_hue(n.q) })
                    })
                    .collect();
                Ok(json!({ "d": sp.p.d, "nodes": nodes, "buckets": sp.order.len() }))
            }

            // ---- the whole app
            "stats" => {
                let notes = st.all()?;
                let words: usize = notes.iter().map(|n| n.markdown.split_whitespace().count()).sum();
                let edits: usize = notes.iter().map(|n| n.edits.len()).sum();
                Ok(json!({
                    "notes": notes.len(), "words": words, "saves": notes.len() + edits,
                    "memories": st.memory_count()?, "tags": tag_counts(&notes),
                    "first": notes.last().map(|n| n.created.clone()),
                    "latest": notes.first().map(|n| n.created.clone()),
                }))
            }

            // ---- settings in the save (hex, as bundles keep them)
            "settings.get" => Ok(json!({
                "value": st.setting(s(args, "name"))?.map(|v| String::from_utf8_lossy(&v).into_owned())
            })),
            "settings.set" => {
                st.set_setting(s(args, "name"), s(args, "value").as_bytes())?;
                Ok(Value::Null)
            }
            _ => Err(format!("there's no {op:?} here")),
        }
    }

    fn saves_call(&self, op: &str, args: &Value) -> Res {
        let (open, store) = self.current();
        match op {
            "saves.list" => Ok(Value::Array(
                self.names()
                    .iter()
                    .map(|n| {
                        let path = self.path(n).unwrap_or_default();
                        let meta = std::fs::metadata(&path).ok();
                        let is_open = n.eq_ignore_ascii_case(&open);
                        let counts = if is_open {
                            Some(store.counts())
                        } else {
                            Store::summary(&path)
                        };
                        json!({
                            "name": n, "open": is_open,
                            "notes": counts.map(|c| c.0), "memories": counts.map(|c| c.1),
                            "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
                            "modified": meta.and_then(|m| m.modified().ok())
                                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
                        })
                    })
                    .collect(),
            )),
            "saves.open" => {
                let name = s(args, "name");
                if !self.path(name)?.exists() {
                    return Err("there's no save by that name".into());
                }
                self.switch(name)?;
                Ok(json!({ "save": name }))
            }
            "saves.new" => {
                let name = clean(s(args, "name"))?;
                if self.path(&name)?.exists() {
                    return Err("there's already a save called that".into());
                }
                self.switch(&name)?;
                Ok(json!({ "save": name }))
            }
            "saves.copy" => {
                let from = s(args, "name");
                let to = clean(s(args, "to")).or_else(|_| clean(&format!("{from} copy")))?;
                if self.path(&to)?.exists() {
                    return Err("there's already a save called that".into());
                }
                std::fs::copy(self.path(from)?, self.path(&to)?).map_err(|e| e.to_string())?;
                Ok(json!({ "save": to }))
            }
            "saves.rename" => {
                let from = s(args, "name").to_string();
                let to = clean(s(args, "to"))?;
                if self.path(&to)?.exists() {
                    return Err("there's already a save called that".into());
                }
                if from.eq_ignore_ascii_case(&open) {
                    return Err("open another save before renaming this one".into());
                }
                std::fs::rename(self.path(&from)?, self.path(&to)?).map_err(|e| e.to_string())?;
                Ok(json!({ "save": to }))
            }
            "saves.delete" => {
                let name = s(args, "name");
                if name.eq_ignore_ascii_case(&open) {
                    return Err("open another save before deleting this one".into());
                }
                std::fs::remove_file(self.path(name)?).map_err(|e| e.to_string())?;
                Ok(Value::Null)
            }
            "saves.export" => {
                let name = match s(args, "name") {
                    "" => open.clone(),
                    n => n.to_string(),
                };
                let b = if name.eq_ignore_ascii_case(&open) {
                    bundle::export(&store, &open)?
                } else {
                    bundle::export(&Store::open(&self.path(&name)?)?, &name)?
                };
                serde_json::to_value(b).map_err(|e| e.to_string())
            }
            "saves.backup" => {
                // weum's backup: the texts in one file, everything else in another, in a
                // `backups` folder beside the saves
                let name = match s(args, "name") {
                    "" => open.clone(),
                    n => n.to_string(),
                };
                let b = if name.eq_ignore_ascii_case(&open) {
                    bundle::export(&store, &open)?
                } else {
                    bundle::export(&Store::open(&self.path(&name)?)?, &name)?
                };
                let v = serde_json::to_value(b).map_err(|e| e.to_string())?;
                let mut out = super::Backup::from_bundle(&v);
                let dir = self.dir.join("backups");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let stamp = chrono::Local::now().format("%Y-%m-%d %H%M%S");
                let texts = dir.join(format!("{name} {stamp} texts.json"));
                let other = dir.join(format!("{name} {stamp} other.json"));
                std::fs::write(&texts, serde_json::to_vec(&out.texts).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                std::fs::write(
                    &other,
                    serde_json::to_vec_pretty(&out.other).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                out.paths = vec![texts.display().to_string(), other.display().to_string()];
                out.texts = Value::Null;
                out.other = Value::Null;
                serde_json::to_value(out).map_err(|e| e.to_string())
            }
            "saves.import" => {
                let b: bundle::Bundle = serde_json::from_value(args.get("bundle").cloned().unwrap_or_default())
                    .map_err(|e| format!("that bundle doesn't read: {e}"))?;
                bundle::check(&b)?;
                if args.get("merge").and_then(Value::as_bool).unwrap_or(false) {
                    let w = bundle::merge(&store, &b)?;
                    return Ok(json!({ "memories": w.memories, "notes": w.notes, "lists": w.lists }));
                }
                let want = match s(args, "name") {
                    "" if b.name.is_empty() => "imported",
                    "" => b.name.as_str(),
                    n => n,
                };
                let mut n = clean(want)?;
                let mut i = 2;
                while self.path(&n)?.exists() {
                    n = clean(&format!("{want} {i}"))?;
                    i += 1;
                }
                std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
                let w = bundle::import(&Store::open(&self.path(&n)?)?, &b)?;
                Ok(json!({ "save": n, "memories": w.memories, "notes": w.notes, "lists": w.lists }))
            }
            _ => Err(format!("there's no {op:?} here")),
        }
    }

    // ---- models (classifiers) and filters: what the models view uses today
    fn models_call(&self, store: &Arc<Store>, op: &str, args: &Value) -> Res {
        let st: &Store = store;
        let name = s(args, "name").trim();
        let job_key = format!("{}\u{0}{name}", st.path().display());
        match op {
            "classify.list" => {
                let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                Ok(Value::Array(
                    st.classifiers()?
                        .iter()
                        .map(|d| {
                            let last = d.last_training();
                            let m = last.and_then(|e| e.metrics.as_ref());
                            let key = format!("{}\u{0}{}", st.path().display(), d.name);
                            json!({
                                "name": d.name, "classes": d.class_names(),
                                "lists": d.classes.iter().map(|c| &c.list).collect::<Vec<_>>(),
                                "head": match d.head { HeadKind::Single => "single", HeadKind::Multi => "multi", HeadKind::Regress => "regress" },
                                "preset": d.preset, "trained": d.trained(),
                                "accuracy": m.map(|m| m.accuracy), "examples": last.map(|e| e.examples),
                                "training": jobs.get(&key).is_some_and(|j| j.state.lock().map(|s| s["state"] == "running").unwrap_or(false)),
                            })
                        })
                        .collect(),
                ))
            }
            "classify.create" => {
                let classes = args
                    .get("classes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|l| parse_ref(l.as_str().unwrap_or("")))
                    .collect::<Result<Vec<_>, _>>()?;
                let preset = match s(args, "preset") {
                    "" => clfs::DEFAULT_PRESET,
                    p => p,
                };
                st.create_classifier(name, &classes, None, HeadKind::Single, preset, vec![])?;
                Ok(Value::Null)
            }
            "classify.delete" => {
                st.delete_classifier(name)?;
                Ok(Value::Null)
            }
            "classify.train" => {
                let a = TrainArgs {
                    epochs: args.get("epochs").and_then(Value::as_u64).map(|x| x as usize),
                    ..TrainArgs::default()
                };
                let mut t = st.prepare(name, &a)?;
                let cell = Arc::new(JobCell::default());
                *cell.state.lock().unwrap_or_else(|e| e.into_inner()) = json!({ "name": name, "state": "running" });
                self.jobs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(job_key, cell.clone());
                let store = store.clone();
                let started = std::time::Instant::now();
                let job_name = name.to_string();
                std::thread::spawn(move || {
                    let c = cell.clone();
                    let r = t.run(|tick| {
                        *c.state.lock().unwrap_or_else(|e| e.into_inner()) = json!({
                            "name": job_name, "state": "running", "epoch": tick.epoch, "step": tick.step,
                            "steps": tick.steps, "loss": tick.loss, "seconds": started.elapsed().as_secs_f64(),
                        });
                        if c.cancel.load(Ordering::Relaxed) {
                            Flow::Stop
                        } else {
                            Flow::Go
                        }
                    });
                    let end = match r.and_then(|s| {
                        if s.cancelled {
                            Ok("cancelled")
                        } else {
                            store.commit(&t, &s).map(|_| "done")
                        }
                    }) {
                        Ok(state) => {
                            json!({ "name": job_name, "state": state, "seconds": started.elapsed().as_secs_f64() })
                        }
                        Err(e) => json!({ "name": job_name, "state": "failed", "error": e }),
                    };
                    *cell.state.lock().unwrap_or_else(|e| e.into_inner()) = end;
                });
                Ok(json!({ "name": name, "state": "running" }))
            }
            "classify.job" => {
                let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                Ok(jobs
                    .get(&job_key)
                    .map(|j| j.state.lock().unwrap_or_else(|e| e.into_inner()).clone())
                    .unwrap_or_else(|| json!({ "name": name, "state": "idle" })))
            }
            "classify.cancel" => {
                let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                let j = jobs.get(&job_key).ok_or("it isn't training")?;
                j.cancel.store(true, Ordering::Relaxed);
                Ok(Value::Null)
            }
            "classify.predict" => {
                let stack = st.stack(name)?;
                let classes = stack.def().class_names();
                let texts: Vec<String> = args
                    .get("texts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|t| t.as_str().unwrap_or("").to_string())
                    .collect();
                let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
                let outs = stack.outputs(&refs, pool());
                let results: Vec<Value> = texts
                    .iter()
                    .zip(&outs)
                    .map(|(t, o)| json!({ "text": t, "probs": o.probs, "top": o.top, "members": o.members }))
                    .collect();
                Ok(json!({ "classes": classes, "trained": stack.def().trained(), "results": results }))
            }
            "classify.search" => {
                let stack = st.stack(name)?;
                let def = stack.def();
                let classes = def.class_names();
                let c = def.class_index(s(args, "class"))?;
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 1000) as usize;
                let mut cands = st.candidates(SCAN)?;
                let q = s(args, "q").trim();
                if !q.is_empty() {
                    let found: HashSet<Ref> = st.search_with_variations(q)?.into_iter().map(|h| h.r).collect();
                    cands.retain(|c| found.contains(&c.0));
                }
                let heads: HashSet<String> = def.classes.iter().map(|c| c.list.clone()).collect();
                cands.retain(|c| !heads.contains(&c.0.label()));
                let texts: Vec<&str> = cands.iter().map(|c| c.1.as_str()).collect();
                let outs = stack.outputs(&texts, pool());
                let mut order: Vec<usize> = (0..cands.len()).collect();
                order.sort_by(|&a, &b| outs[b].probs[c].total_cmp(&outs[a].probs[c]).then(a.cmp(&b)));
                let results: Vec<Value> = order
                    .into_iter()
                    .take(limit)
                    .map(|i| {
                        let (r, _, shown) = &cands[i];
                        let o = &outs[i];
                        json!({ "key": r.label(), "text": shown, "note": matches!(r, Ref::Note(_)),
                                "prob": o.probs[c], "top_class": classes.get(o.top), "member": o.members.contains(&c) })
                    })
                    .collect();
                Ok(json!({ "results": results }))
            }
            "classify.feedback" => {
                let def = st.classifier(name)?;
                let class = def.class_names()[def.class_index(s(args, "class"))?].clone();
                let key = s(args, "key").trim();
                let j = if key.is_empty() {
                    Judged {
                        key: None,
                        text: s(args, "text").to_string(),
                    }
                } else {
                    let r = parse_ref(key)?;
                    let text = st.ref_text(&r)?.ok_or("didn't find that")?;
                    Judged { key: Some(r), text }
                };
                let yes = args.get("yes").and_then(Value::as_bool).ok_or("yes: true or false")?;
                let e = st.feedback_classifier(name, &j, &class, yes, 1.0)?;
                Ok(json!({ "entry": e }))
            }
            "classify.enlist" => {
                let def = st.classifier(name)?;
                let c = def.class_index(s(args, "class"))?;
                let (head, n) = st.enlist_outputs(name, &def.class_names()[c], None, SCAN)?;
                let text = st.remember(&head).map(|h| h.text).unwrap_or_default();
                Ok(json!({ "list": head.label(), "name": text, "members": n }))
            }
            "filters.list" => Ok(Value::Array(
                st.filter_presets()?
                    .iter()
                    .map(|p| {
                        let mut v = json!(p);
                        v["reads"] = json!(p.expr.describe());
                        v
                    })
                    .collect(),
            )),
            "filters.save" => {
                let e: filters::Expr = serde_json::from_value(args.get("expr").cloned().unwrap_or_default())
                    .map_err(|e| format!("that filter doesn't read: {e}"))?;
                Ok(json!(st.save_filter(name, e, s(args, "description"))?))
            }
            "filters.delete" => {
                st.delete_filter(name)?;
                Ok(Value::Null)
            }
            "filters.run" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(200).clamp(1, 5000) as usize;
                let e: filters::Expr = match args.get("expr") {
                    Some(e) if !e.is_null() => {
                        serde_json::from_value(e.clone()).map_err(|e| format!("that filter doesn't read: {e}"))?
                    }
                    _ => st.filter_preset(name)?.expr,
                };
                Ok(json!(st.run_filter(&e, limit, SCAN, pool())?))
            }
            "filters.enlist" => {
                let (head, n) = st.enlist_filter(name, SCAN, pool())?;
                Ok(
                    json!({ "list": head.label(), "name": st.remember(&head).map(|h| h.text).unwrap_or_default(), "members": n }),
                )
            }
            _ => Err(format!("there's no {op:?} here")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gem::Mode;

    fn temp() -> (Local, PathBuf) {
        let dir = std::env::temp_dir().join(format!("anend-app-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Local::open(&dir, None).unwrap(), dir)
    }

    fn wait<T: 'static>(f: Fut<T>) -> Result<T, String> {
        let mut f = f;
        loop {
            match futures_util::FutureExt::now_or_never(&mut f) {
                Some(r) => return r,
                None => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        }
    }

    #[test]
    fn memories_lists_and_backup() {
        let (b, dir) = temp();
        let (k, new) = wait(b.saturate("listen")).unwrap();
        assert!(new && k == "0");
        wait(b.saturate("silent")).unwrap();
        let found = wait(b.gsearch(Mode::Ordinal, "79")).unwrap();
        assert_eq!(found.len(), 2);
        wait(b.put_memory("0", "enlist")).unwrap();
        assert_eq!(wait(b.remember("0")).unwrap().text, "enlist");
        let counts = wait(b.bucket_counts(Mode::Ordinal)).unwrap();
        assert_eq!(counts, vec![super::super::BucketCount { sum: 79, count: 2 }]);
        wait(b.enlist("0", "1")).unwrap();
        assert_eq!(wait(b.lists()).unwrap().lists[0].count, 1);
        let backup = wait(b.call("saves.backup", json!({}))).unwrap();
        assert_eq!(backup["paths"].as_array().unwrap().len(), 2);
        let (bundle, n) = super::super::weum_bundle("w", &json!([["truth", [9]]])).unwrap();
        assert_eq!(n, 1);
        let im = wait(b.import_bundle(bundle, "", true)).unwrap();
        assert_eq!(im.memories, 1);
        std::fs::remove_dir_all(dir).ok();
    }
}
