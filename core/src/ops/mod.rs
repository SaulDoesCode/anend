//! The app API's operations (docs/app-api.md) over the entry graph (docs/format.md),
//! shared by the desktop (app/src/backend/local.rs) and the hub (hub/src/app.rs), so
//! both answer every op the same way.
//!
//! Keys are weum labels of entry ids (`record::label`, like `0,12`); a note is an entry of
//! mime `text/markdown` (`# title` and markdown), lists are member edges, tags tag edges,
//! notes on an entry note edges. [`call`] answers the ops on one save; [`Saves`] (not in
//! the browser build) keeps a folder of saves and answers `saves.*` and `forge.*` too.

#[cfg(not(target_arch = "wasm32"))]
pub mod blocks;
#[cfg(not(target_arch = "wasm32"))]
pub mod classify;
pub mod merge;
#[cfg(not(target_arch = "wasm32"))]
pub mod models;
#[cfg(not(target_arch = "wasm32"))]
pub mod pictures;
#[cfg(not(target_arch = "wasm32"))]
mod saves;
pub mod templates;
pub mod texts;

#[cfg(not(target_arch = "wasm32"))]
pub use saves::{Open, Saves, clean, graph_at};
pub use texts::GraphTexts;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::graph::gematria::{Measure, Mode};
use crate::graph::query::{Order, Page, Query};
use crate::graph::record::{self, mime};
use crate::graph::store::{self as gstore, rel};
use crate::graph::{Graph, Reader, View};
use crate::spread::{self, Params, Spread};
use crate::weum;

pub type Res = Result<Value, String>;

/// what forgetting took away, per entry, so putting the same text back (`mem.put`)
/// restores its edges too
pub type Removed = Mutex<HashMap<u64, gstore::Removed>>;

/// past this many lists the grid of how alike every two are (lists² numbers) is left out
pub const MAX_DOT_LISTS: usize = 1000;

// ------------------------------------------------------------------ helpers

pub fn s<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(Value::as_str).unwrap_or("")
}

/// a key argument: a weum label (`0,12`); `note:0,12`, as the old app named notes, too
pub fn id_arg(args: &Value, k: &str) -> Result<u64, String> {
    let l = s(args, k);
    let bare = l.trim().strip_prefix("note:").unwrap_or(l);
    record::parse_label(bare).ok_or_else(|| format!("{l:?} isn't a key (like 0,12)"))
}

pub fn iso(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

fn ms_of(iso: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|t| t.timestamp_millis().max(0) as u64)
}

fn gmode(name: &str) -> Result<Mode, String> {
    match name {
        "" | "ordinal" => Ok(Mode::Ordinal),
        "reversed" => Ok(Mode::Reversed),
        "atbash" => Err("atbash was removed (it groups texts as reversed does): ordinal, reversed or frequency".into()),
        b => Err(format!("a bucket is ordinal, reversed or frequency, not {b:?}")),
    }
}

fn strings(args: &Value, k: &str) -> Vec<String> {
    args.get(k)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn is_note<R: Reader>(v: &R, id: u64) -> bool {
    v.record(id).ok().flatten().is_some_and(|r| r.mime == mime::MARKDOWN)
}

/// a hit: `{key, text, note, ordinal, reversed}`; a note shows as its title
pub fn hits<R: Reader>(v: &R, ids: &[u64]) -> Value {
    let texts = v.render_many(ids);
    let mut out = Vec::with_capacity(ids.len());
    for (id, t) in ids.iter().zip(texts) {
        let Ok(text) = t else { continue };
        let note = is_note(v, *id);
        let shown = if note { crate::note::split(&text).0 } else { text };
        let m = Measure::of(&shown);
        out.push(json!({
            "key": record::label(*id), "text": shown, "note": note,
            "ordinal": m.ordinal, "reversed": m.reversed,
        }));
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

pub fn note_json<R: Reader>(v: &R, id: u64) -> Res {
    let r = v.entry(id)?.record;
    let text = v.render(id)?;
    let (title, markdown) = crate::note::split(&text);
    let edits: Vec<String> = v
        .versions(id)
        .unwrap_or_default()
        .iter()
        .skip(1)
        .rev()
        .map(|ver| iso(ver.at))
        .collect();
    let mut out = json!({
        "_key": record::label(id), "title": title, "markdown": markdown, "tags": tag_names(v, id),
        "created": iso(r.created), "edits": edits, "modified": iso(r.edited),
    });
    // a note with replacements: its text as written, `{{key}}` where an entry goes
    if !r.plain()
        && let Some(src) = templates::source_of(v, id)?
    {
        out["source"] = json!(src);
    }
    Ok(out)
}

fn search_ids(v: &View, q: &str) -> Result<Vec<u64>, String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for variant in weum::search_variations(q.trim()) {
        for id in v.search_words(&variant)? {
            if seen.insert(id) {
                out.push(id);
            }
        }
    }
    Ok(out)
}

/// everything worth knowing about a text's numbers: ordinal and reversed (weum's), the
/// letter frequency key, weum's gsum line, prime factor reductions, and the space's
/// triangle ⌊√(s·r)⌋
pub fn measure_json(text: &str) -> Value {
    let m = Measure::of(text);
    let (s, r) = (m.ordinal, m.reversed);
    let pf = |n: u64| weum::segs_to_string(&weum::pf_reduction(n));
    json!({
        "ordinal": s,
        "reversed": r,
        "frequency": m.frequency,
        "gsum": weum::gsum_detail(text),
        "pf_ordinal": if s > 1 { pf(s as u64) } else { String::new() },
        "pf_reversed": if r > 1 { pf(r as u64) } else { String::new() },
        "triangle": ((s.max(0) * r.max(0)) as f64).sqrt().floor() as i64,
        "s": s,
        "r": r,
    })
}

/// how many notes and memories (entries that are neither notes nor tags) a save holds
pub fn counts(v: &View) -> Result<(u64, u64), String> {
    let all = v.entry_count()?;
    let of = |m: u16| -> Result<u64, String> {
        let q = Query {
            mime: Some(m),
            ..Query::default()
        };
        Ok(v.query(&q, Page::first(0))?.total as u64)
    };
    let notes = of(mime::MARKDOWN)?;
    let tags = of(mime::TAG)?;
    Ok((notes, all.saturating_sub(notes + tags)))
}

fn tag_counts(v: &View) -> Result<Vec<(String, u64)>, String> {
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
    Ok(out)
}

/// what the space's free spread weighs texts by (`space.features`): for each key, its sums,
/// length, kind, word types, when it was written and the lists it is in
fn space_features(v: &View, args: &Value) -> Res {
    let keys = strings(args, "keys");
    if keys.len() > 2000 {
        return Err("at most 2000 texts at once".into());
    }
    // every list a text is in: the heads of the member edges pointing at it
    let mut lists: HashMap<u64, Vec<String>> = HashMap::new();
    for (from, r, to, _) in v.all_edges()? {
        if r == rel::MEMBER {
            lists.entry(to).or_default().push(record::label(from));
        }
    }
    let mut out = Vec::new();
    for k in &keys {
        let bare = k.trim().strip_prefix("note:").unwrap_or(k);
        let Some(id) = record::parse_label(bare) else { continue };
        let Some(rec) = v.record(id)? else { continue };
        let text = v.render(id)?;
        let m = Measure::of(&text);
        let mut types: BTreeMap<&str, u32> = BTreeMap::new();
        for (_, t) in crate::kinds::word_types(&text) {
            *types.entry(t).or_default() += 1;
        }
        out.push(json!({
            "key": k,
            "s": m.ordinal,
            "r": m.reversed,
            "chars": text.chars().count(),
            "words": text.split_whitespace().count(),
            "kind": crate::kinds::detect(&text),
            "types": types,
            "created": rec.created,
            "lists": lists.get(&id).cloned().unwrap_or_default(),
            "frequency": m.frequency,
        }));
    }
    Ok(json!({ "features": out }))
}

fn list_stats(v: &View) -> Res {
    let mut heads: Vec<u64> = Vec::new();
    let mut seen = HashSet::new();
    for (from, r, _, _) in v.all_edges()? {
        if r == rel::MEMBER && seen.insert(from) {
            heads.push(from);
        }
    }
    let mut lists = Vec::new();
    let mut vectors = Vec::new();
    for head in heads {
        let members = v.members(head)?;
        let hs = hits(v, &members);
        let texts: Vec<String> = hs
            .as_array()
            .into_iter()
            .flatten()
            .map(|h| h["text"].as_str().unwrap_or("").to_string())
            .collect();
        let (mut os, mut rs) = (0i64, 0i64);
        for t in &texts {
            let m = Measure::of(t);
            os += m.ordinal;
            rs += m.reversed;
        }
        let n = texts.len().max(1) as f64;
        let vector = crate::lists::letter_vector(texts.iter().map(String::as_str));
        lists.push(json!({
            "list": record::label(head), "name": v.render(head).unwrap_or_default(),
            "members": hs, "count": texts.len(), "ordinal_sum": os, "reversed_sum": rs,
            "ordinal_mean": os as f64 / n, "reversed_mean": rs as f64 / n,
            "whole": measure_json(&texts.join(" ")), "vector": vector.to_vec(),
        }));
        vectors.push(vector);
    }
    let dot: Vec<Vec<f64>> = if vectors.len() <= MAX_DOT_LISTS {
        vectors
            .iter()
            .map(|a| vectors.iter().map(|b| crate::lists::cosine(a, b)).collect())
            .collect()
    } else {
        Vec::new()
    };
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
        let text = s(it, "text");
        // about the size the app's texts are: half a unit per character, half a unit high
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

/// the ops that only read, or make room: they work whatever a quota says
pub const READ_ONLY: &[&str] = &[
    "hello",
    "saves.list",
    "saves.open",
    "saves.delete",
    "saves.rename",
    "saves.export",
    "saves.backup",
    "notes.list",
    "notes.tags",
    "notes.get",
    "notes.alternatives",
    "notes.delete",
    "notes.on",
    "tags.all",
    "tags.entries",
    "tags.remove",
    "mem.measure",
    "mem.gsearch",
    "mem.search",
    "mem.forget",
    "mem.remember",
    "mem.all",
    "mem.buckets",
    "lists.stats",
    "lists.delist",
    "lists.unlist",
    "space.spread",
    "space.features",
    "forge.preview",
    "entries.query",
    "settings.get",
    "entries.info",
    "stats",
];

/// the ops that make a save
pub const MAKE_A_SAVE: &[&str] = &[
    "saves.new",
    "saves.copy",
    "saves.import",
    "saves.import_texts",
    "forge.make",
];

// ------------------------------------------------------------------ the operations on one save

/// one op on the save `g` (everything but `saves.*`, `forge.*` and `hello`, which need
/// the folder; see [`Saves::call`])
pub fn call(g: &Graph, removed: &Removed, op: &str, args: &Value) -> Res {
    let forgot = |id: u64, r: gstore::Removed| {
        removed.lock().unwrap_or_else(|e| e.into_inner()).insert(id, r);
    };
    match op {
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
            if let (Some(a), Some(b)) = (ms_of(s(args, "since")), ms_of(s(args, "until"))) {
                q.created = Some((a, b));
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
        "notes.tags" | "tags.all" => Ok(json!(
            tag_counts(&g.view()?)?
                .into_iter()
                .map(|(tag, count)| json!({ "tag": tag, "count": count }))
                .collect::<Vec<_>>()
        )),
        "tags.entries" => {
            // every entry carrying all (or, with mode "or", any) of these tags
            let v = g.view()?;
            let any = s(args, "mode") == "or";
            let mut sets: Vec<Vec<u64>> = Vec::new();
            for t in strings(args, "tags") {
                let ids = match v
                    .find_text(t.trim().trim_start_matches('#').trim(), Some(mime::TAG))?
                    .first()
                {
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
        "notes.get" => {
            let mut n = note_json(&g.view()?, id_arg(args, "key")?)?;
            // saved filters' results where the note names them
            let whole = crate::note::compose(s(&n, "title"), s(&n, "markdown"));
            if whole.contains("{{filter:") {
                n["rendered"] = json!(templates::with_views(g, removed, s(&n, "markdown"))?);
            }
            Ok(n)
        }
        "notes.alternatives" => templates::alternatives(g, args),
        "notes.save" => {
            let note = args.get("note").cloned().unwrap_or(Value::Null);
            let text = match args.get("text").and_then(Value::as_str) {
                Some(t) => t.to_string(),
                None => crate::note::compose(s(&note, "title"), s(&note, "markdown")),
            };
            if crate::note::split(&text).0.trim().is_empty() {
                return Err("a note needs a # title".into());
            }
            let tags: Vec<String> = strings(&note, "tags")
                .into_iter()
                .map(|t| t.trim().trim_start_matches('#').trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            let key = s(&note, "_key").trim().to_string();
            // a note made on an entry (`on`) is linked to it
            let on = match s(args, "on") {
                "" => None,
                _ => Some(id_arg(args, "on")?),
            };
            let id = g.write(|tx| {
                let id = match record::parse_label(&key) {
                    Some(id) if !key.is_empty() && tx.exists(id)? => {
                        // `{{key}}` replacements are kept as live refs
                        match templates::run_of(tx, &text, Some(id))? {
                            Some(run) => {
                                if templates::source_of(tx, id)?.as_deref() != Some(text.as_str()) {
                                    tx.edit_run(id, &run)?;
                                }
                            }
                            None => {
                                let r = tx.record(id)?.ok_or("gone")?;
                                if !r.plain() || tx.render(id)? != text {
                                    tx.edit_text(id, &text)?;
                                }
                            }
                        }
                        id
                    }
                    _ => {
                        let id = tx.alloc_id()?;
                        let now = tx.now;
                        match templates::run_of(tx, &text, None)? {
                            Some(run) => tx.put_run_at(id, &run, mime::MARKDOWN, now, now)?,
                            None => tx.put_text_at(id, &text, mime::MARKDOWN, now, now)?,
                        }
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
                if let Some(on) = on
                    && on != id
                    && !tx.out_edges(id, rel::NOTE)?.iter().any(|(t, _)| *t == on)
                {
                    tx.link(id, rel::NOTE, on, &[])?;
                }
                Ok(id)
            })?;
            note_json(&g.view()?, id)
        }
        "notes.delete" => {
            let id = id_arg(args, "key")?;
            let r = g.delete(id)?;
            forgot(id, r);
            Ok(Value::Null)
        }
        "notes.on" => {
            let v = g.view()?;
            let ids = v.notes_on(id_arg(args, "key")?)?;
            Ok(hits(&v, &ids))
        }

        // ---- texts and sums
        "mem.measure" => Ok(measure_json(s(args, "text"))),
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
                let m = gmode(b)?;
                let value = match args.get("key") {
                    Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
                    Some(Value::String(t)) => match t.trim().parse::<i64>() {
                        Ok(n) => n,
                        Err(_) => Measure::of(t).get(m),
                    },
                    _ => return Err("gsearch needs a key: a number, or a text to measure".into()),
                };
                v.by_sum(m, value)?
            };
            Ok(json!({ "bucket": if b.is_empty() { "ordinal" } else { b }, "hits": hits(&v, &ids) }))
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
            if !g.view()?.exists(id)? {
                return Err("there was nothing with that name anyways".into());
            }
            let r = g.delete(id)?;
            let text = r.text.clone();
            forgot(id, r);
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
            let back = removed.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            g.write(|tx| {
                if tx.exists(id)? {
                    tx.edit_text(id, text)
                } else if let Some(r) = back.as_ref().filter(|r| r.text == text) {
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
            let counts = v.sum_counts(gmode(s(args, "bucket"))?)?;
            Ok(
                json!({ "buckets": counts.into_iter().map(|(sum, count)| json!({ "sum": sum, "count": count })).collect::<Vec<_>>() }),
            )
        }

        // ---- lists: member edges
        "lists.stats" => list_stats(&g.view()?),
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
        "space.spread" => spread_layout(args),
        "space.features" => space_features(&g.view()?, args),

        // ---- the whole save
        "stats" => {
            let v = g.view()?;
            let st = v.stats()?;
            let notes = v.query(
                &Query {
                    mime: Some(mime::MARKDOWN),
                    ..Query::default()
                },
                Page::all(),
            )?;
            let mut words = 0u64;
            for id in &notes.ids {
                words += v
                    .render(*id)
                    .map(|t| crate::note::split(&t).1.split_whitespace().count() as u64)
                    .unwrap_or(0);
            }
            let tags: Vec<(String, u64)> = tag_counts(&v)?;
            Ok(json!({
                "notes": st.notes, "memories": counts(&v)?.1, "words": words,
                "saves": st.notes + st.versions, "tags": tags,
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
                q.sum_range = Some((gmode(s(args, "bucket"))?, lo, hi));
            }
            if let (Some(a), Some(b)) = (ms_of(s(args, "since")), ms_of(s(args, "until"))) {
                q.created = Some((a, b));
            }
            if let Some(t) = args.get("tag").and_then(Value::as_str) {
                match v
                    .find_text(t.trim().trim_start_matches('#').trim(), Some(mime::TAG))?
                    .first()
                {
                    Some(&tid) => q.tag = Some(tid),
                    None => return Ok(json!({ "total": 0, "hits": [] })),
                }
            }
            match s(args, "kind") {
                "notes" => q.mime = Some(mime::MARKDOWN),
                "memories" => q.mime = Some(mime::TEXT),
                _ => {}
            }
            let limit = match args.get("limit").and_then(Value::as_u64) {
                Some(0) | None => 300,
                Some(n) => n.min(20_000) as usize,
            };
            let found = v.query(&q, Page::first(limit))?;
            Ok(json!({ "total": found.total, "hits": hits(&v, &found.ids) }))
        }

        // ---- settings in the save
        "entries.info" => {
            let id = id_arg(args, "key")?;
            let v = g.view()?;
            let rec = v.record(id)?.ok_or("didn't find that")?;
            let label = record::label(id);
            let text = v.render(id)?;
            let detected = crate::kinds::detect(&text);
            let fixed = v
                .setting(&format!("kind {label}"))?
                .map(|b| String::from_utf8_lossy(&b).into_owned());
            Ok(json!({
                "key": label,
                "mime": v.mime_name(rec.mime)?,
                "created": iso(rec.created),
                "edited": iso(rec.edited),
                "kind": fixed.as_deref().unwrap_or(detected),
                "detected": detected,
                "fixed": fixed.is_some(),
                "kinds": crate::kinds::KINDS,
            }))
        }
        "entries.set_kind" => {
            let id = id_arg(args, "key")?;
            let kind = s(args, "kind").trim();
            if !kind.is_empty() && !crate::kinds::KINDS.contains(&kind) {
                return Err(format!("a kind is one of {}", crate::kinds::KINDS.join(", ")));
            }
            let name = format!("kind {}", record::label(id));
            g.write(|tx| {
                if !tx.exists(id)? {
                    return Err("didn't find that".into());
                }
                // the detected kind again: nothing kept
                if kind.is_empty() {
                    tx.remove_setting(&name)
                } else {
                    tx.set_setting(&name, kind.as_bytes())
                }
            })?;
            Ok(Value::Null)
        }
        "settings.get" => {
            let v = g.view()?;
            Ok(json!({ "value": v.setting(s(args, "name"))?.map(|b| String::from_utf8_lossy(&b).into_owned()) }))
        }
        "settings.set" => {
            let (name, value) = (s(args, "name").to_string(), s(args, "value").to_string());
            if name.trim().is_empty() {
                return Err("which setting?".into());
            }
            g.write(|tx| tx.set_setting(&name, value.as_bytes()))?;
            Ok(Value::Null)
        }
        _ => Err(format!("there's no {op:?} here")),
    }
}

/// a save as a bundle: `{anend: 2, name, memories: texts.json's rows, other: other.json}`
pub fn bundle(name: &str, g: &Graph) -> Res {
    let (texts, other) = g.backup()?;
    let texts: Value = serde_json::from_str(&texts).map_err(|e| e.to_string())?;
    let other: Value = serde_json::from_str(&other).map_err(|e| e.to_string())?;
    Ok(json!({ "anend": 2, "name": name, "memories": texts, "other": other }))
}

/// what a bundle holds to restore: its texts rows (weum's or a backup's) and, from a
/// whole backup, everything else
pub fn bundle_parts(b: &Value) -> Result<(String, Option<String>), String> {
    let rows = b
        .get("memories")
        .or_else(|| b.get("texts"))
        .cloned()
        .unwrap_or(json!([]));
    if !rows.is_array() {
        return Err("that bundle's memories should be a list of [text, [key bytes], ...]".into());
    }
    let other = b.get("other").filter(|o| o.is_object()).map(Value::to_string);
    if b.get("anend").and_then(Value::as_u64) == Some(1)
        && b.get("notes")
            .is_some_and(|n| n.as_array().is_some_and(|a| !a.is_empty()))
    {
        return Err("that's an old bundle (notes as their own records); only its memories would come in. Export it again from a current app, or import its weum texts".into());
    }
    Ok((rows.to_string(), other))
}

#[cfg(test)]
mod tests;
