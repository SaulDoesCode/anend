//! The desktop's backend: the entry graph (core/src/graph, docs/format.md) in this
//! process. Every call runs on a worker thread and answers through a channel, so the
//! interface never waits on the disk.
//!
//! The operations are the app API's (docs/app-api.md) answered from the graph: keys are
//! weum labels of entry ids (`record::label`), a note is an entry of mime `text/markdown`
//! (`# title` and markdown), lists are `member` edges, tags `tag` edges. Added here (the
//! hub gets them when it moves to the graph): `mem.put`, `mem.buckets`, `tags.add`,
//! `tags.remove`, `tags.all`, `tags.entries`, `notes.on`, `entries.query`,
//! `saves.backup`, `saves.import_texts`, `settings.get`, `settings.set`. Models wait for
//! the model spec (docs/models.md).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anend_core::graph::gematria::{Measure as GMeasure, Mode as GMode};
use anend_core::graph::query::{Order, Page, Query};
use anend_core::graph::record::{self, mime};
use anend_core::graph::store::{Removed, rel};
use anend_core::graph::{Graph, Reader};
use anend_core::spread::{self, Params, Spread};
use serde_json::{Value, json};

use super::{Backend, Fut};

type Res = Result<Value, String>;

struct Open {
    name: String,
    path: PathBuf,
    graph: Arc<Graph>,
}

/// the saves folder and the open save
#[derive(Clone)]
pub struct Local {
    dir: PathBuf,
    open: Arc<Mutex<Open>>,
    /// what forgetting took away, so undo puts back the edges too
    removed: Arc<Mutex<HashMap<u64, Removed>>>,
}

impl Local {
    /// opens `save` (a path), or `main.redb` in `dir`
    pub fn open(dir: &Path, save: Option<&Path>) -> Result<Local, String> {
        let path = save.map(Path::to_path_buf).unwrap_or_else(|| dir.join("main.redb"));
        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dir.to_path_buf());
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let graph = Graph::open(&path)?;
        Ok(Local {
            dir,
            open: Arc::new(Mutex::new(Open {
                name: save_name(&path),
                path,
                graph: Arc::new(graph),
            })),
            removed: Default::default(),
        })
    }

    fn current(&self) -> (String, Arc<Graph>) {
        let o = self.open.lock().unwrap_or_else(|e| e.into_inner());
        (o.name.clone(), o.graph.clone())
    }

    fn path(&self, name: &str) -> Result<PathBuf, String> {
        Ok(self.dir.join(format!("{}.redb", clean(name)?)))
    }

    fn switch(&self, name: &str) -> Result<(), String> {
        let path = self.path(name)?;
        let mut o = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if o.path == path {
            return Ok(());
        }
        let graph = Graph::open(&path)?;
        *o = Open {
            name: save_name(&path),
            path,
            graph: Arc::new(graph),
        };
        self.removed.lock().unwrap_or_else(|e| e.into_inner()).clear();
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

    pub fn save_path(&self) -> PathBuf {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).path.clone()
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
        self.save_path().display().to_string()
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

fn id_arg(args: &Value, k: &str) -> Result<u64, String> {
    let l = s(args, k);
    record::parse_label(l).ok_or_else(|| format!("{l:?} isn't a key (like 0,12)"))
}

fn iso(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| t.with_timezone(&chrono::Local).to_rfc3339())
        .unwrap_or_default()
}

fn ms_of(iso: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|t| t.timestamp_millis().max(0) as u64)
}

fn gmode(name: &str) -> GMode {
    match name {
        "reversed" => GMode::Reversed,
        _ => GMode::Ordinal,
    }
}

fn split_note(text: &str) -> (String, String) {
    anend_core::note::split(text)
}

fn hits<R: Reader>(v: &R, ids: &[u64]) -> Value {
    let texts = v.render_many(ids);
    let mut out = Vec::with_capacity(ids.len());
    for (id, t) in ids.iter().zip(texts) {
        let Ok(text) = t else { continue };
        let note = v.record(*id).ok().flatten().is_some_and(|r| r.mime == mime::MARKDOWN);
        let shown = if note { split_note(&text).0 } else { text };
        out.push(json!({ "key": record::label(*id), "text": shown, "note": note }));
    }
    Value::Array(out)
}

fn tag_names<R: Reader>(v: &R, id: u64) -> Vec<String> {
    v.tags_of(id)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|t| v.render(t).ok())
        .collect()
}

fn note_json<R: Reader>(v: &R, id: u64) -> Result<Value, String> {
    let r = v.entry(id)?.record;
    let text = v.render(id)?;
    let (title, markdown) = split_note(&text);
    let edits: Vec<String> = v
        .versions(id)
        .unwrap_or_default()
        .iter()
        .skip(1)
        .rev()
        .map(|ver| iso(ver.at))
        .collect();
    Ok(json!({
        "_key": record::label(id), "title": title, "markdown": markdown, "tags": tag_names(v, id),
        "created": iso(r.created), "edits": edits, "modified": iso(r.edited),
    }))
}

fn search_ids(v: &anend_core::graph::View, q: &str) -> Result<Vec<u64>, String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for variant in anend_core::weum::search_variations(q.trim()) {
        for id in v.search_words(&variant)? {
            if seen.insert(id) {
                out.push(id);
            }
        }
    }
    Ok(out)
}

fn strings(args: &Value, k: &str) -> Vec<String> {
    args.get(k)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

// ------------------------------------------------------------------ the operations

impl Local {
    fn dispatch(&self, op: &str, args: &Value) -> Res {
        if op.starts_with("saves.") {
            return self.saves_call(op, args);
        }
        let (open, g) = self.current();
        let g: &Graph = &g;
        match op {
            "hello" => {
                let v = g.view()?;
                let all = v.entry_count()?;
                let notes_q = Query {
                    mime: Some(mime::MARKDOWN),
                    ..Query::default()
                };
                let notes = v.query(&notes_q, Page::first(0))?.total as u64;
                Ok(
                    json!({ "save": open, "saves": self.names(), "counts": { "notes": notes, "memories": all - notes } }),
                )
            }

            // ---- notes: markdown entries
            "notes.list" => {
                let v = g.view()?;
                let mut q = Query {
                    mime: Some(mime::MARKDOWN),
                    order: Order::CreatedDesc,
                    ..Query::default()
                };
                if !s(args, "q").trim().is_empty() {
                    q.words = Some(s(args, "q").to_string());
                }
                let tags = strings(args, "tags");
                let any = s(args, "mode") == "or";
                let mut out = Vec::new();
                for id in v.query(&q, Page::all())?.ids {
                    let n = note_json(&v, id)?;
                    let have: Vec<&str> = n["tags"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    let ok = tags.is_empty()
                        || if any {
                            tags.iter().any(|t| have.contains(&t.as_str()))
                        } else {
                            tags.iter().all(|t| have.contains(&t.as_str()))
                        };
                    if !ok {
                        continue;
                    }
                    let md = n["markdown"].as_str().unwrap_or("");
                    out.push(json!({
                        "key": n["_key"], "title": n["title"], "tags": n["tags"], "created": n["created"],
                        "modified": n["modified"], "edits": n["edits"].as_array().map(Vec::len).unwrap_or(0),
                        "preview": md.chars().take(160).collect::<String>(), "words": md.split_whitespace().count(),
                    }));
                }
                Ok(Value::Array(out))
            }
            "notes.tags" | "tags.all" => {
                let v = g.view()?;
                let mut counts: BTreeMap<u64, u64> = BTreeMap::new();
                for (_, r, to, _) in v.all_edges()? {
                    if r == rel::TAG {
                        *counts.entry(to).or_default() += 1;
                    }
                }
                let mut out: Vec<(String, u64)> = counts
                    .into_iter()
                    .filter_map(|(t, n)| v.render(t).ok().map(|name| (name, n)))
                    .collect();
                out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                Ok(json!(
                    out.into_iter()
                        .map(|(tag, count)| json!({ "tag": tag, "count": count }))
                        .collect::<Vec<_>>()
                ))
            }
            "tags.entries" => {
                // every entry carrying all (or, with mode "or", any) of these tags
                let v = g.view()?;
                let any = s(args, "mode") == "or";
                let mut sets: Vec<Vec<u64>> = Vec::new();
                for t in strings(args, "tags") {
                    let ids = match v.find_text(&t, Some(mime::TAG))?.first() {
                        Some(&tid) => v.in_edges(tid, rel::TAG)?,
                        None => vec![],
                    };
                    sets.push(ids);
                }
                let mut ids: Vec<u64> = if any {
                    let mut all: Vec<u64> = sets.concat();
                    all.sort();
                    all.dedup();
                    all
                } else {
                    let mut it = sets.into_iter();
                    let first = it.next().unwrap_or_default();
                    it.fold(first, |acc, s| acc.into_iter().filter(|x| s.contains(x)).collect())
                };
                ids.truncate(2000);
                Ok(hits(&v, &ids))
            }
            "tags.add" | "tags.remove" => {
                let id = id_arg(args, "key")?;
                let name = s(args, "tag").trim().trim_start_matches('#').trim().to_string();
                if name.is_empty() {
                    return Err("which tag?".into());
                }
                g.write(|tx| {
                    if op == "tags.add" {
                        tx.tag(id, &name)?;
                    } else if let Some(&t) = tx.find_text(&name, Some(mime::TAG))?.first() {
                        tx.unlink(id, rel::TAG, t)?;
                    }
                    Ok(())
                })?;
                Ok(Value::Null)
            }
            "notes.get" => note_json(&g.view()?, id_arg(args, "key")?),
            "notes.save" => {
                let note = args.get("note").cloned().unwrap_or(Value::Null);
                let text = match args.get("text").and_then(Value::as_str) {
                    Some(t) => t.to_string(),
                    None => anend_core::note::compose(s(&note, "title"), s(&note, "markdown")),
                };
                if split_note(&text).0.trim().is_empty() {
                    return Err("a note needs a # title".into());
                }
                let tags = strings(&note, "tags");
                let key = s(&note, "_key").to_string();
                let id = g.write(|tx| {
                    let id = match record::parse_label(&key) {
                        Some(id) if !key.is_empty() && tx.exists(id)? => {
                            tx.edit_text(id, &text)?;
                            id
                        }
                        _ => {
                            let id = tx.alloc_id()?;
                            let now = tx.now;
                            tx.put_text_at(id, &text, mime::MARKDOWN, now, now)?;
                            id
                        }
                    };
                    let have: Vec<u64> = tx.tags_of(id)?;
                    let mut want = Vec::new();
                    for t in &tags {
                        want.push(tx.tag(id, t)?);
                    }
                    for t in have {
                        if !want.contains(&t) {
                            tx.unlink(id, rel::TAG, t)?;
                        }
                    }
                    Ok(id)
                })?;
                note_json(&g.view()?, id)
            }
            "notes.delete" => {
                let id = id_arg(args, "key")?;
                let r = g.delete(id)?;
                self.removed.lock().unwrap_or_else(|e| e.into_inner()).insert(id, r);
                Ok(Value::Null)
            }
            "notes.on" => {
                let v = g.view()?;
                let ids = v.notes_on(id_arg(args, "key")?)?;
                Ok(hits(&v, &ids))
            }

            // ---- texts and sums
            "mem.gsearch" => {
                let b = s(args, "bucket");
                let v = g.view()?;
                let ids = if b == "frequency" {
                    let t = match args.get("key") {
                        Some(Value::String(t)) => t.clone(),
                        Some(other) => other.to_string(),
                        None => String::new(),
                    };
                    v.by_letters(&t)?
                } else {
                    let m = gmode(b);
                    let value = match args.get("key") {
                        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
                        Some(Value::String(t)) => match t.trim().parse::<i64>() {
                            Ok(n) => n,
                            Err(_) => GMeasure::of(t).get(m),
                        },
                        _ => return Err("gsearch needs a key: a number, or a text to measure".into()),
                    };
                    v.by_sum(m, value)?
                };
                Ok(json!({ "bucket": b, "hits": hits(&v, &ids) }))
            }
            "mem.search" => {
                let v = g.view()?;
                let q = s(args, "q");
                let ids = if args.get("variations").and_then(Value::as_bool).unwrap_or(true) {
                    search_ids(&v, q)?
                } else {
                    v.search_words(q)?
                };
                Ok(hits(&v, &ids))
            }
            "mem.saturate" => {
                let text = s(args, "text").trim();
                if text.is_empty() {
                    return Err("nothing to remember".into());
                }
                let (id, new) = g.save_text(text, mime::TEXT)?;
                Ok(json!({ "key": record::label(id), "new": new }))
            }
            "mem.forget" => {
                let id = id_arg(args, "key")?;
                let r = g.delete(id)?;
                let text = r.text.clone();
                self.removed.lock().unwrap_or_else(|e| e.into_inner()).insert(id, r);
                Ok(json!({ "text": text }))
            }
            "mem.remember" => {
                let v = g.view()?;
                let id = id_arg(args, "key")?;
                hits(&v, &[id])
                    .as_array()
                    .and_then(|a| a.first().cloned())
                    .ok_or_else(|| "didn't find that".to_string())
            }
            "mem.put" => {
                let id = id_arg(args, "key")?;
                let text = s(args, "text");
                if text.trim().is_empty() {
                    return Err("an empty text: forget it instead".into());
                }
                let removed = self.removed.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                g.write(|tx| {
                    if tx.exists(id)? {
                        tx.edit_text(id, text)
                    } else if let Some(r) = removed.as_ref().filter(|r| r.text == text) {
                        tx.undelete(r)
                    } else {
                        let now = tx.now;
                        tx.put_text_at(id, text, mime::TEXT, now, now)
                    }
                })?;
                Ok(Value::Null)
            }
            "mem.all" => {
                let v = g.view()?;
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(5000) as usize;
                let ids = v.query(&Query::default(), Page::first(limit))?.ids;
                Ok(hits(&v, &ids))
            }
            "mem.buckets" => {
                let v = g.view()?;
                let counts = v.sum_counts(gmode(s(args, "bucket")))?;
                Ok(
                    json!({ "buckets": counts.into_iter().map(|(sum, count)| json!({ "sum": sum, "count": count })).collect::<Vec<_>>() }),
                )
            }

            // ---- lists: member edges
            "lists.stats" => {
                let v = g.view()?;
                let mut heads: Vec<u64> = Vec::new();
                let mut seen = HashSet::new();
                for (from, r, _, _) in v.all_edges()? {
                    if r == rel::MEMBER && seen.insert(from) {
                        heads.push(from);
                    }
                }
                let mut lists = Vec::new();
                for head in heads {
                    let members = v.members(head)?;
                    let hs = hits(&v, &members);
                    let texts: Vec<String> = hs
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|h| h["text"].as_str().unwrap_or("").to_string())
                        .collect();
                    let (mut os, mut rs) = (0i64, 0i64);
                    for t in &texts {
                        let (o, r) = spread::sums(t);
                        os += o;
                        rs += r;
                    }
                    let n = texts.len().max(1) as f64;
                    let vector = anend_core::lists::letter_vector(texts.iter().map(String::as_str));
                    lists.push(json!({
                        "list": record::label(head), "name": v.render(head).unwrap_or_default(),
                        "members": hs, "count": texts.len(), "ordinal_sum": os, "reversed_sum": rs,
                        "ordinal_mean": os as f64 / n, "reversed_mean": rs as f64 / n, "vector": vector.to_vec(),
                    }));
                }
                Ok(json!({ "lists": lists, "dot": [] }))
            }
            "lists.enlist" => {
                let (l, m) = (id_arg(args, "list")?, id_arg(args, "member")?);
                g.write(|tx| tx.add_member(l, m))?;
                Ok(Value::Null)
            }
            "lists.delist" => {
                let (l, m) = (id_arg(args, "list")?, id_arg(args, "member")?);
                g.write(|tx| tx.unlink(l, rel::MEMBER, m).map(|_| ()))?;
                Ok(Value::Null)
            }
            "lists.unlist" => {
                let l = id_arg(args, "list")?;
                let n = g.write(|tx| {
                    let members = tx.members(l)?;
                    for m in &members {
                        tx.unlink(l, rel::MEMBER, *m)?;
                    }
                    Ok(members.len())
                })?;
                Ok(json!({ "removed": n }))
            }

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
                let v = g.view()?;
                let st = v.stats()?;
                let notes_q = Query {
                    mime: Some(mime::MARKDOWN),
                    ..Query::default()
                };
                let notes = v.query(&notes_q, Page::all())?;
                let mut words = 0u64;
                for id in &notes.ids {
                    words += v.render(*id).map(|t| t.split_whitespace().count() as u64).unwrap_or(0);
                }
                Ok(json!({
                    "notes": st.notes, "memories": st.entries - st.notes, "words": words, "saves": st.versions,
                    "entries": st.entries, "by_mime": st.by_mime, "edges": st.edges, "versions": st.versions,
                    "body_bytes": st.body_bytes, "borrowing": st.borrowing,
                    "first": st.oldest.map(iso), "latest": st.newest.map(iso),
                }))
            }
            "entries.query" => {
                // the query model: words, a sum range, a time range, a tag, a kind
                let v = g.view()?;
                let mut q = Query::default();
                if !s(args, "words").trim().is_empty() {
                    q.words = Some(s(args, "words").to_string());
                }
                if let (Some(lo), Some(hi)) = (
                    args.get("from").and_then(Value::as_i64),
                    args.get("to").and_then(Value::as_i64),
                ) {
                    q.sum_range = Some((gmode(s(args, "bucket")), lo, hi));
                }
                if let (Some(a), Some(b)) = (ms_of(s(args, "since")), ms_of(s(args, "until"))) {
                    q.created = Some((a, b));
                }
                if let Some(t) = args.get("tag").and_then(Value::as_str)
                    && let Some(&tid) = v.find_text(t, Some(mime::TAG))?.first()
                {
                    q.tag = Some(tid);
                }
                match s(args, "kind") {
                    "notes" => q.mime = Some(mime::MARKDOWN),
                    "memories" => q.mime = Some(mime::TEXT),
                    _ => {}
                }
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(300) as usize;
                let found = v.query(&q, Page::first(limit))?;
                Ok(json!({ "total": found.total, "hits": hits(&v, &found.ids) }))
            }

            // ---- settings in the save
            "settings.get" => {
                let v = g.view()?;
                Ok(json!({ "value": v.setting(s(args, "name"))?.map(|b| String::from_utf8_lossy(&b).into_owned()) }))
            }
            "settings.set" => {
                let (name, value) = (s(args, "name").to_string(), s(args, "value").to_string());
                g.write(|tx| tx.set_setting(&name, value.as_bytes()))?;
                Ok(Value::Null)
            }
            o if o.starts_with("classify.") || o.starts_with("filters.") => {
                Err("models come with the model spec (docs/models.md); not in this build yet".into())
            }
            _ => Err(format!("there's no {op:?} here")),
        }
    }

    fn saves_call(&self, op: &str, args: &Value) -> Res {
        let (open, g) = self.current();
        let target = |name: &str| -> Result<(String, Arc<Graph>), String> {
            if name.is_empty() || name.eq_ignore_ascii_case(&open) {
                Ok((open.clone(), g.clone()))
            } else {
                Ok((name.to_string(), Arc::new(Graph::open(&self.path(name)?)?)))
            }
        };
        match op {
            "saves.list" => Ok(Value::Array(
                self.names()
                    .iter()
                    .map(|n| {
                        let path = self.path(n).unwrap_or_default();
                        let meta = std::fs::metadata(&path).ok();
                        let is_open = n.eq_ignore_ascii_case(&open);
                        let entries = if is_open {
                            g.view().and_then(|v| v.entry_count()).ok()
                        } else {
                            None
                        };
                        json!({
                            "name": n, "open": is_open, "notes": Value::Null, "memories": entries,
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
                let (_, src) = target(from)?;
                let (texts, other) = src.backup()?;
                Graph::open(&self.path(&to)?)?.restore(&texts, &other)?;
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
            "saves.backup" => {
                // weum's backup: the texts in one file, everything else in another, in a
                // `backups` folder beside the saves
                let (name, graph) = target(s(args, "name"))?;
                let (texts, other) = graph.backup()?;
                let dir = self.dir.join("backups");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let stamp = chrono::Local::now().format("%Y-%m-%d %H%M%S");
                let tp = dir.join(format!("{name} {stamp} texts.json"));
                let opth = dir.join(format!("{name} {stamp} other.json"));
                std::fs::write(&tp, texts).map_err(|e| e.to_string())?;
                std::fs::write(&opth, other).map_err(|e| e.to_string())?;
                Ok(json!({ "paths": [tp.display().to_string(), opth.display().to_string()] }))
            }
            "saves.import_texts" => {
                // weum's all-of-it.json (or a backup's texts.json) into a save: `name` a
                // new save, or the open one when empty
                let data = s(args, "data");
                let new = s(args, "name").trim();
                let (name, graph) = if new.is_empty() {
                    (open.clone(), g.clone())
                } else {
                    let n = clean(new)?;
                    if self.path(&n)?.exists() {
                        return Err("there's already a save called that".into());
                    }
                    let gr = Arc::new(Graph::open(&self.path(&n)?)?);
                    (n, gr)
                };
                let im = graph.import_texts(data)?;
                Ok(json!({ "save": name, "memories": im.entries, "replaced": im.replaced,
                           "skipped": im.skipped, "borrowing": im.borrowing }))
            }
            "saves.import" => {
                // a bundle's texts (weum rows) into the open save or a new one
                let rows = args
                    .get("bundle")
                    .and_then(|b| b.get("memories"))
                    .cloned()
                    .unwrap_or(json!([]));
                let merge = args.get("merge").and_then(Value::as_bool).unwrap_or(false);
                let name = if merge {
                    String::new()
                } else {
                    s(args, "name").to_string()
                };
                self.saves_call("saves.import_texts", &json!({ "data": rows.to_string(), "name": name }))
            }
            "saves.export" => {
                let (name, graph) = target(s(args, "name"))?;
                let (texts, other) = graph.backup()?;
                let texts: Value = serde_json::from_str(&texts).map_err(|e| e.to_string())?;
                let other: Value = serde_json::from_str(&other).map_err(|e| e.to_string())?;
                Ok(json!({ "anend": 2, "name": name, "memories": texts, "other": other }))
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
        (Local::open(&dir, None).unwrap(), dir)
    }

    fn wait<T: 'static>(mut f: Fut<T>) -> Result<T, String> {
        loop {
            match futures_util::FutureExt::now_or_never(&mut f) {
                Some(r) => return r,
                None => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        }
    }

    #[test]
    fn texts_lists_tags_notes_backup() {
        let (b, dir) = temp();
        let (k, new) = wait(b.saturate("listen")).unwrap();
        assert!(new && k == "0");
        wait(b.saturate("silent")).unwrap();
        assert_eq!(wait(b.saturate("listen")).unwrap(), ("0".to_string(), false));
        assert_eq!(wait(b.gsearch(Mode::Ordinal, "79")).unwrap().len(), 2);
        assert_eq!(wait(b.gsearch(Mode::Frequency, "tinsel")).unwrap().len(), 2);
        wait(b.put_memory("0", "enlist")).unwrap();
        assert_eq!(wait(b.remember("0")).unwrap().text, "enlist");
        let counts = wait(b.bucket_counts(Mode::Ordinal)).unwrap();
        assert_eq!(counts, vec![super::super::BucketCount { sum: 79, count: 2 }]);
        // forget and put back: the list keeps its member
        wait(b.enlist("0", "1")).unwrap();
        assert_eq!(wait(b.lists()).unwrap().lists[0].count, 1);
        assert_eq!(wait(b.forget("1")).unwrap(), "silent");
        wait(b.put_memory("1", "silent")).unwrap();
        assert_eq!(wait(b.lists()).unwrap().lists[0].count, 1);
        // tags on texts
        wait(b.tag("0", "anagram")).unwrap();
        assert_eq!(wait(b.tagged(&["anagram".into()], false)).unwrap()[0].text, "enlist");
        // notes
        let n = wait(b.save_note(&Default::default(), "# hello\n\nworld")).unwrap();
        assert_eq!(n.title, "hello");
        assert_eq!(wait(b.notes(&Default::default())).unwrap().len(), 1);
        let h = wait(b.hello()).unwrap();
        assert_eq!(h.counts.notes, 1);
        let backup = wait(b.call("saves.backup", json!({}))).unwrap();
        assert_eq!(backup["paths"].as_array().unwrap().len(), 2);
        let im = wait(b.import_texts(r#"[["truth", [9]]]"#, "")).unwrap();
        assert_eq!(im.memories, 1);
        assert_eq!(wait(b.remember("9")).unwrap().text, "truth");
        std::fs::remove_dir_all(dir).ok();
    }
}
