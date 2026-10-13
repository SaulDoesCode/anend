//! The app, run by the hub: each member's saves live on the server in a workspace of
//! their own (with `--local`, one workspace and no login, for a daemon on your own
//! machine), and the web app works on them through one dispatcher, reached over a
//! WebSocket (`/app/ws`) or plain posts (`/app/api/{op}`). Every call is
//! `{"id", "op", "args"}` and every answer `{"id", "ok": true, "data"}` or
//! `{"id", "ok": false, "error"}`; docs/app-api.md lists the operations.

use anend_core::bundle;
use anend_core::forge::{self, Effect, Filter};
use anend_core::memory::{self, Bucket, BucketKey, Hit, Ref};
use anend_core::note::Note;
use anend_core::spread::{self, Params, Spread};
use anend_core::store::Store;
use anend_core::weum;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::store::User;

pub type Res = Result<Value, String>;

/// whose saves a call works on
pub fn owner(user: Option<&User>) -> Result<String, String> {
    if *LOCAL.get().unwrap_or(&false) {
        return Ok("local".into());
    }
    match user {
        Some(u) if crate::hub::Hub::active(u) => Ok(u.name.to_lowercase()),
        Some(_) => Err("your account is waiting for an admin".into()),
        None => Err("log in to use the app here".into()),
    }
}

/// one workspace for everyone, no login (a daemon on your own machine)
pub static LOCAL: OnceLock<bool> = OnceLock::new();

pub struct Workspace {
    pub dir: PathBuf,
    open: Mutex<Option<(String, Arc<Store>)>>,
}

pub struct Workspaces {
    root: PathBuf,
    all: Mutex<HashMap<String, Arc<Workspace>>>,
}

pub static WORKSPACES: OnceLock<Workspaces> = OnceLock::new();

pub fn workspaces() -> &'static Workspaces {
    WORKSPACES.get().expect("workspaces start with the hub")
}

impl Workspaces {
    pub fn new(root: PathBuf) -> Workspaces {
        Workspaces {
            root,
            all: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, owner: &str) -> Arc<Workspace> {
        let mut all = self.all.lock().unwrap_or_else(|e| e.into_inner());
        all.entry(owner.to_string())
            .or_insert_with(|| {
                Arc::new(Workspace {
                    dir: self.root.join(owner).join("saves"),
                    open: Mutex::new(None),
                })
            })
            .clone()
    }
}

/// a save's name made safe for a file name
pub fn clean(name: &str) -> Result<String, String> {
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
    if n.is_empty() || n.chars().count() > 80 {
        Err("a save's name is 1 to 80 letters".into())
    } else {
        Ok(n)
    }
}

impl Workspace {
    pub fn path(&self, name: &str) -> Result<PathBuf, String> {
        Ok(self.dir.join(format!("{}.redb", clean(name)?)))
    }

    fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "redb"))
                    .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|n| n.to_lowercase());
        out
    }

    /// the open save, opening the last one (or making `notes`) the first time
    pub fn store(&self) -> Result<(String, Arc<Store>), String> {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((n, s)) = open.as_ref() {
            return Ok((n.clone(), s.clone()));
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let last = std::fs::read_to_string(self.dir.join(".open")).ok();
        let name = last
            .map(|s| s.trim().to_string())
            .filter(|n| self.path(n).is_ok_and(|p| p.exists()))
            .or_else(|| self.names().into_iter().next())
            .unwrap_or_else(|| "notes".into());
        let store = Arc::new(Store::open(&self.path(&name)?)?);
        *open = Some((name.clone(), store.clone()));
        Ok((name, store))
    }

    fn switch(&self, name: &str) -> Result<(String, Arc<Store>), String> {
        let path = self.path(name)?;
        let name = clean(name)?;
        {
            let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
            // let go of the old one first (redb holds its file)
            *open = None;
            let store = Arc::new(Store::open(&path)?);
            *open = Some((name.clone(), store));
        }
        let _ = std::fs::write(self.dir.join(".open"), &name);
        self.store()
    }

    fn close_if_open(&self, name: &str) {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if open.as_ref().is_some_and(|(n, _)| n.eq_ignore_ascii_case(name)) {
            *open = None;
        }
    }
}

// ------------------------------------------------------------------ json helpers

fn s<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(Value::as_str).unwrap_or("")
}

fn hit_json(h: &Hit) -> Value {
    let (o, r) = spread::sums(&h.text);
    json!({
        "key": h.r.label(),
        "text": h.text,
        "note": matches!(h.r, Ref::Note(_)),
        "ordinal": o,
        "reversed": r,
    })
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

/// everything worth knowing about a text's numbers
pub fn measure_json(text: &str) -> Value {
    let m = memory::measure(text);
    let (s, r) = spread::sums(text);
    let pf = |n: u64| weum::segs_to_string(&weum::pf_reduction(n));
    json!({
        "ordinal": m.ordinal,
        "reversed": m.reversed,
        "atbash": m.atbash,
        "frequency": m.frequency,
        "gsum": weum::gsum_detail(text),
        "pf_ordinal": if s > 1 { pf(s as u64) } else { String::new() },
        "pf_reversed": if r > 1 { pf(r as u64) } else { String::new() },
        // the space's Ctrl+right click: ⌊√(s·r)⌋, and the triangle s, r and it make
        "triangle": ((s.max(0) * r.max(0)) as f64).sqrt().floor() as i64,
        "s": s,
        "r": r,
    })
}

fn note_summary(n: &Note) -> Value {
    let preview: String = n.markdown.chars().take(160).collect();
    json!({
        "key": n.key,
        "title": n.display_title(),
        "tags": n.tags,
        "created": n.created,
        "modified": n.edits.last().unwrap_or(&n.created),
        "edits": n.edits.len(),
        "preview": preview,
        "words": n.markdown.split_whitespace().count(),
    })
}

fn list_stats(store: &Store) -> Res {
    let lists = anend_core::lists::stats(store)?;
    let dot = anend_core::lists::dots(&lists);
    let lists: Vec<Value> = lists
        .iter()
        .map(|l| {
            json!({
                "list": l.list.label(),
                "name": l.name,
                "members": hits(&l.members),
                "count": l.members.len(),
                "ordinal_sum": l.ordinal_sum,
                "reversed_sum": l.reversed_sum,
                "ordinal_mean": l.ordinal_mean(),
                "reversed_mean": l.reversed_mean(),
                "whole": measure_json(&l.joined),
                "vector": l.vector.to_vec(),
            })
        })
        .collect();
    Ok(json!({ "lists": lists, "dot": dot }))
}

fn spread_layout(args: &Value) -> Res {
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
        let id = s(it, "id");
        let text = s(it, "text");
        // about the size the app's texts are: half a unit per character, half a unit high
        let hw = (text.chars().count() as f64 * 0.27 + 0.1).min(12.0);
        sp.add(Spread::node(id, text, hw, 0.35));
    }
    sp.rekey();
    sp.wake(1.0);
    sp.settle(4000, 400);
    let out: Vec<Value> = sp
        .nodes
        .iter()
        .map(|n| {
            json!({
                "id": n.id, "x": n.x, "y": n.y, "z": n.z, "s": n.s, "r": n.r, "q": n.q,
                "hue": spread::seed_hue(n.q),
            })
        })
        .collect();
    Ok(json!({ "d": p.d, "nodes": out, "buckets": sp.order.len() }))
}

fn forge_parts(ws: &Workspace, args: &Value) -> Result<forge::Contents, String> {
    let (open_name, open_store) = ws.store()?;
    let mut parts = Vec::new();
    for ing in args
        .get("ingredients")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let name = s(&ing, "save");
        let effect = match s(&ing, "effect") {
            "take away" | "take_away" | "-" => Effect::TakeAway,
            "keep shared" | "keep_shared" | "shared" => Effect::KeepShared,
            _ => Effect::Add,
        };
        let c = if name.eq_ignore_ascii_case(&open_name) {
            forge::read(&open_store)?
        } else {
            let st = Store::open(&ws.path(name)?)?;
            forge::read(&st)?
        };
        parts.push((effect, c));
    }
    let refs: Vec<(Effect, &forge::Contents)> = parts.iter().map(|(e, c)| (*e, c)).collect();
    let f = args.get("filter").cloned().unwrap_or(Value::Null);
    let num = |k: &str| f.get(k).and_then(Value::as_i64);
    let flag = |k: &str| f.get(k).and_then(Value::as_bool).unwrap_or(true);
    let filter = Filter {
        words: s(&f, "words").to_string(),
        ordinal: (num("ordinal_from"), num("ordinal_to")),
        reversed: (num("reversed_from"), num("reversed_to")),
        memories: flag("memories"),
        notes: flag("notes"),
        lists: flag("lists"),
    };
    Ok(filter.apply(forge::combine(&refs)))
}

fn save_info(ws: &Workspace, name: &str, open: &str) -> Value {
    let path = ws.path(name).unwrap_or_default();
    let meta = std::fs::metadata(&path).ok();
    let counts = if name.eq_ignore_ascii_case(open) {
        ws.store().ok().map(|(_, s)| s.counts())
    } else {
        Store::summary(&path)
    };
    json!({
        "name": name,
        "open": name.eq_ignore_ascii_case(open),
        "notes": counts.map(|c| c.0),
        "memories": counts.map(|c| c.1),
        "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
        "modified": meta.and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
    })
}

/// the save operations, which open and close saves, so they mustn't hold one
fn saves_call(ws: &Workspace, open: &str, op: &str, args: &Value) -> Res {
    match op {
        "saves.list" => Ok(Value::Array(
            ws.names().iter().map(|n| save_info(ws, n, open)).collect(),
        )),
        "saves.open" => {
            let name = s(args, "name");
            if !ws.path(name)?.exists() {
                return Err("there's no save by that name".into());
            }
            let (n, _) = ws.switch(name)?;
            Ok(json!({ "save": n }))
        }
        "saves.new" => {
            let name = clean(s(args, "name"))?;
            if ws.path(&name)?.exists() {
                return Err("there's already a save called that".into());
            }
            let (n, _) = ws.switch(&name)?;
            Ok(json!({ "save": n }))
        }
        "saves.copy" => {
            let from = s(args, "name");
            let to = clean(s(args, "to")).or_else(|_| clean(&format!("{from} copy")))?;
            if ws.path(&to)?.exists() {
                return Err("there's already a save called that".into());
            }
            std::fs::copy(ws.path(from)?, ws.path(&to)?).map_err(|e| e.to_string())?;
            Ok(json!({ "save": to }))
        }
        "saves.rename" => {
            let from = s(args, "name").to_string();
            let to = clean(s(args, "to"))?;
            if ws.path(&to)?.exists() {
                return Err("there's already a save called that".into());
            }
            let was_open = from.eq_ignore_ascii_case(open);
            ws.close_if_open(&from);
            std::fs::rename(ws.path(&from)?, ws.path(&to)?).map_err(|e| e.to_string())?;
            if was_open {
                ws.switch(&to)?;
            }
            Ok(json!({ "save": to }))
        }
        "saves.delete" => {
            let name = s(args, "name");
            if name.eq_ignore_ascii_case(open) {
                return Err("open another save before deleting this one".into());
            }
            std::fs::remove_file(ws.path(name)?).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        "saves.export" => {
            let name = match s(args, "name") {
                "" => open.to_string(),
                n => n.to_string(),
            };
            let b = if name.eq_ignore_ascii_case(open) {
                bundle::export(&ws.store()?.1, open)?
            } else {
                let path = ws.path(&name)?;
                if !path.exists() {
                    return Err("there's no save by that name".into());
                }
                bundle::export(&Store::open(&path)?, &name)?
            };
            serde_json::to_value(b).map_err(|e| e.to_string())
        }
        "saves.import" => {
            let b: bundle::Bundle =
                serde_json::from_value(args.get("bundle").cloned().unwrap_or_default()).map_err(|e| format!("that bundle doesn't read: {e}"))?;
            bundle::check(&b)?;
            if args.get("merge").and_then(Value::as_bool).unwrap_or(false) {
                let w = bundle::merge(&ws.store()?.1, &b)?;
                return Ok(json!({ "merged": true, "memories": w.memories, "notes": w.notes, "lists": w.lists }));
            }
            let want = match s(args, "name") {
                "" if b.name.is_empty() => "imported",
                "" => b.name.as_str(),
                n => n,
            };
            let mut n = clean(want)?;
            let mut i = 2;
            while ws.path(&n)?.exists() {
                n = clean(&format!("{want} {i}"))?;
                i += 1;
            }
            std::fs::create_dir_all(&ws.dir).map_err(|e| e.to_string())?;
            let w = bundle::import(&Store::open(&ws.path(&n)?)?, &b)?;
            Ok(json!({ "save": n, "memories": w.memories, "notes": w.notes, "lists": w.lists }))
        }

        _ => Err(format!("there's no {op:?} here")),
    }
}

/// one operation on a workspace; runs off the async threads (redb blocks)
pub fn call(ws: &Workspace, op: &str, args: &Value) -> Res {
    if op.starts_with("saves.") {
        let open = ws.store()?.0;
        return saves_call(ws, &open, op, args);
    }
    let (open, store) = ws.store()?;
    let store: &Store = &store;
    match op {
        "hello" => Ok(json!({
            "save": open,
            "saves": ws.names(),
            "counts": { "notes": store.counts().0, "memories": store.counts().1 },
        })),

        // ---- notes
        "notes.list" => {
            let keep: Option<HashSet<String>> = match s(args, "q").trim() {
                "" => None,
                q => Some(store.note_filter(q)?),
            };
            let notes: Vec<Value> = store
                .all()?
                .iter()
                .filter(|n| keep.as_ref().is_none_or(|k| k.contains(&n.key)))
                .map(note_summary)
                .collect();
            Ok(Value::Array(notes))
        }
        "notes.get" => {
            let key = s(args, "key");
            let n = store.all()?.into_iter().find(|n| n.key == key).ok_or("no such note")?;
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
            // keep what the editor didn't send
            if !note.key.is_empty()
                && let Some(old) = store.all()?.into_iter().find(|n| n.key == note.key)
            {
                note.created = old.created;
                note.edits = old.edits;
            }
            serde_json::to_value(store.save(&note)?).map_err(|e| e.to_string())
        }
        "notes.delete" => {
            store.delete(s(args, "key"))?;
            Ok(Value::Null)
        }

        // ---- memories
        "mem.measure" => Ok(measure_json(s(args, "text"))),
        "mem.gsearch" => {
            let b = bucket(s(args, "bucket"));
            let key = match (b, args.get("key")) {
                // a word's letters, sorted, whatever order they're typed in
                (Bucket::Frequency, Some(Value::String(t))) => memory::measure(t).key(b),
                (_, Some(Value::Number(n))) => BucketKey::Sum(n.as_u64().unwrap_or(0) as u32),
                (_, Some(Value::String(t))) => match t.trim().parse::<u32>() {
                    Ok(n) if b != Bucket::Frequency => BucketKey::Sum(n),
                    _ => memory::measure(t).key(b),
                },
                _ => return Err("gsearch needs a key: a number, or a text to measure".into()),
            };
            Ok(json!({ "bucket": b.name(), "hits": hits(&store.gsearch(b, &key)?) }))
        }
        "mem.search" => {
            let q = s(args, "q");
            let found = if args.get("variations").and_then(Value::as_bool).unwrap_or(true) {
                store.search_with_variations(q)?
            } else {
                store.search(q)?
            };
            Ok(hits(&found))
        }
        "mem.saturate" => {
            let text = s(args, "text").trim();
            if text.is_empty() {
                return Err("nothing to remember".into());
            }
            let (key, new) = store.saturate(text)?;
            Ok(json!({ "key": Ref::Memory(key).label(), "new": new }))
        }
        "mem.forget" => {
            let r = parse_ref(s(args, "key"))?;
            let Ref::Memory(k) = r else {
                return Err("notes are deleted from the editor".into());
            };
            Ok(json!({ "text": store.forget(&k)? }))
        }
        "mem.remember" => Ok(hit_json(&store.remember(&parse_ref(s(args, "key"))?)?)),
        "mem.all" => {
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(5000) as usize;
            let all: Vec<Value> = store
                .all_memories()?
                .into_iter()
                .take(limit)
                .map(|(t, k)| {
                    hit_json(&Hit {
                        r: Ref::Memory(k),
                        text: t,
                    })
                })
                .collect();
            Ok(Value::Array(all))
        }

        // ---- lists
        "lists.stats" => list_stats(store),
        "lists.enlist" => {
            store.enlist(&parse_ref(s(args, "list"))?, &parse_ref(s(args, "member"))?)?;
            Ok(Value::Null)
        }
        "lists.delist" => {
            store.delist(&parse_ref(s(args, "list"))?, &parse_ref(s(args, "member"))?)?;
            Ok(Value::Null)
        }
        "lists.unlist" => Ok(json!({ "removed": store.unlist(&parse_ref(s(args, "list"))?)? })),

        // ---- the space
        "space.spread" => spread_layout(args),

        // ---- the forge
        "forge.preview" => {
            let c = forge_parts(ws, args)?;
            let sample: Vec<Value> = c
                .memories
                .iter()
                .take(24)
                .map(|t| {
                    let (o, r) = spread::sums(t);
                    json!({ "text": t, "ordinal": o, "reversed": r })
                })
                .collect();
            Ok(json!({
                "memories": c.memories.len(), "notes": c.notes.len(), "lists": c.lists.len(),
                "sample": sample,
                "titles": c.notes.iter().take(12).map(|n| n.display_title().to_string()).collect::<Vec<_>>(),
            }))
        }
        "forge.make" => {
            let name = clean(s(args, "name"))?;
            let path = ws.path(&name)?;
            if path.exists() {
                return Err("there's already a save called that".into());
            }
            let c = forge_parts(ws, args)?;
            let out = Store::open(&path)?;
            let w = forge::write_into(&out, &c)?;
            Ok(json!({ "save": name, "memories": w.memories, "notes": w.notes, "lists": w.lists }))
        }
        "forge.pour" => {
            let c = forge_parts(ws, args)?;
            let w = forge::write_into(store, &c)?;
            Ok(json!({ "memories": w.memories, "notes": w.notes, "lists": w.lists }))
        }

        // ---- stats
        "stats" => {
            let notes = store.all()?;
            let words: usize = notes.iter().map(|n| n.markdown.split_whitespace().count()).sum();
            let edits: usize = notes.iter().map(|n| n.edits.len()).sum();
            let mut tags: HashMap<String, usize> = HashMap::new();
            for n in &notes {
                for t in &n.tags {
                    *tags.entry(t.clone()).or_default() += 1;
                }
            }
            let mut tags: Vec<(String, usize)> = tags.into_iter().collect();
            tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            Ok(json!({
                "notes": notes.len(), "words": words, "saves": notes.len() + edits,
                "memories": store.memory_count()?, "tags": tags,
                "first": notes.last().map(|n| n.created.clone()),
                "latest": notes.first().map(|n| n.created.clone()),
            }))
        }
        _ => Err(format!("there's no {op:?} here")),
    }
}

/// a hub save into the workspace (`merge`: into the open save instead of beside it)
pub fn take_hub_save(ws: &Workspace, hub_file: &Path, name: &str, merge: bool) -> Res {
    if merge {
        let (_, store) = ws.store()?;
        let src = Store::open(hub_file)?;
        let w = forge::write_into(&store, &forge::read(&src)?)?;
        return Ok(json!({ "merged": true, "memories": w.memories, "notes": w.notes, "lists": w.lists }));
    }
    let mut n = clean(name)?;
    let mut i = 2;
    while ws.path(&n)?.exists() {
        n = clean(&format!("{name} {i}"))?;
        i += 1;
    }
    std::fs::create_dir_all(&ws.dir).map_err(|e| e.to_string())?;
    std::fs::copy(hub_file, ws.path(&n)?).map_err(|e| e.to_string())?;
    Ok(json!({ "save": n }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_calls() {
        let dir = std::env::temp_dir().join(format!("anend-app-{}", rand::random::<u64>()));
        let ws = Workspace {
            dir: dir.clone(),
            open: Mutex::new(None),
        };
        let v = call(&ws, "hello", &json!({})).unwrap();
        assert_eq!(v["save"], "notes");
        call(&ws, "mem.saturate", &json!({"text": "listen"})).unwrap();
        call(&ws, "mem.saturate", &json!({"text": "silent"})).unwrap();
        let again = call(&ws, "mem.saturate", &json!({"text": "listen"})).unwrap();
        assert_eq!(again["new"], false);
        let g = call(&ws, "mem.gsearch", &json!({"bucket": "ordinal", "key": 79})).unwrap();
        assert_eq!(g["hits"].as_array().unwrap().len(), 2);
        let g = call(&ws, "mem.gsearch", &json!({"bucket": "frequency", "key": "enlist"})).unwrap();
        assert_eq!(g["hits"].as_array().unwrap().len(), 2);
        let m = call(&ws, "mem.measure", &json!({"text": "truth"})).unwrap();
        assert_eq!(
            (m["s"].as_i64(), m["r"].as_i64(), m["triangle"].as_i64()),
            (Some(87), Some(48), Some(64))
        );

        let n = call(
            &ws,
            "notes.save",
            &json!({"note": {}, "text": "# Hello\n\nworld listen"}),
        )
        .unwrap();
        let key = n["_key"].as_str().unwrap().to_string();
        assert_eq!(
            call(&ws, "notes.list", &json!({"q": "wor"}))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(call(&ws, "notes.save", &json!({"note": {}, "text": "no title"})).is_err());

        call(&ws, "lists.enlist", &json!({"list": "0", "member": "1"})).unwrap();
        call(&ws, "lists.enlist", &json!({"list": "1", "member": "0"})).unwrap();
        let st = call(&ws, "lists.stats", &json!({})).unwrap();
        assert_eq!(st["lists"].as_array().unwrap().len(), 2);
        // listen and silent are anagrams: their letter vectors point the same way
        assert_eq!(st["dot"][0][1], 1.0);

        let sp = call(&ws, "space.spread", &json!({"texts": [{"id": "a", "text": "listen"}, {"id": "b", "text": "silent"}, {"id": "c", "text": "truth"}]})).unwrap();
        assert_eq!(sp["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(sp["buckets"], 2);

        call(&ws, "saves.new", &json!({"name": "second"})).unwrap();
        call(&ws, "mem.saturate", &json!({"text": "silent"})).unwrap();
        let p = call(
            &ws,
            "forge.preview",
            &json!({"ingredients": [{"save": "notes"}, {"save": "second", "effect": "take away"}]}),
        )
        .unwrap();
        assert_eq!(p["memories"], 1);
        call(
            &ws,
            "forge.make",
            &json!({"name": "subset", "ingredients": [{"save": "notes"}, {"save": "second", "effect": "take away"}]}),
        )
        .unwrap();
        assert_eq!(
            call(&ws, "saves.list", &json!({})).unwrap().as_array().unwrap().len(),
            3
        );
        assert!(
            call(&ws, "saves.delete", &json!({"name": "second"})).is_err(),
            "the open one stays"
        );
        call(&ws, "saves.open", &json!({"name": "notes"})).unwrap();
        let b = call(&ws, "saves.export", &json!({})).unwrap();
        assert_eq!(b["anend"], 1);
        let got = call(&ws, "saves.import", &json!({"bundle": b})).unwrap();
        assert_eq!(got["save"], "notes 2");
        assert_eq!(call(&ws, "saves.export", &json!({"name": "notes 2"})).unwrap()["memories"], b["memories"]);
        assert_eq!(call(&ws, "saves.import", &json!({"bundle": b, "merge": true})).unwrap()["memories"], 0);
        call(&ws, "saves.delete", &json!({"name": "notes 2"})).unwrap();
        // a bundle it can't read leaves nothing behind
        assert!(call(&ws, "saves.import", &json!({"bundle": {"anend": 2}, "name": "bad"})).is_err());
        assert!(!ws.path("bad").unwrap().exists());
        call(&ws, "notes.delete", &json!({"key": key})).unwrap();
        call(&ws, "saves.rename", &json!({"name": "notes", "to": "renamed"})).unwrap();
        assert_eq!(call(&ws, "hello", &json!({})).unwrap()["save"], "renamed");
        assert!(call(&ws, "nonsense", &json!({})).is_err());
        assert!(clean("../x").is_ok_and(|n| !n.contains('/')));
        std::fs::remove_dir_all(dir).ok();
    }
}
