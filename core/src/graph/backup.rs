//! weum's `all-of-it.json` import, and the two-file backup (like weum's `memory.backup`):
//! all the texts in one JSON, everything else in another.
//!
//! **Texts** (`texts.json`): `[[text, [key bytes], created, edited, mime?], ...]`, one row
//! per entry in id order; `text` is the rendered text, the key is weum's (see
//! `record::key_of`), the times are milliseconds since the Unix epoch, and `mime` is left
//! out for `text/plain`. The first two elements are weum's own `[text, key]` rows, so
//! weum's backup imports through the same reader (with import time for both times).
//!
//! **The rest** (`other.json`), an object:
//! - `format`: `"anend-graph"`, `version`: 1;
//! - `allocated`: the last allocated key (or null), `free`: the forgotten keys in reuse order;
//! - `mimes`: `{name: id}` for registered mime types;
//! - `sources`: `[[key, run], ...]` for entries that aren't plain text (templates,
//!   functions, transclusions), as a run: a JSON array of strings (text), `{"ref": key}`,
//!   `{"call": builtin name, "args": [run, ...]}` and `{"fn": key, "args": [run, ...]}`;
//! - `edges`: `[[from key, relation, to key, payload hex], ...]` (relation by name for the
//!   known ones, else its number);
//! - `history`: `[[key, at, mime, text], ...]`, past versions, newest first per entry;
//! - `settings`: `{name: text}`, or `{name: {"hex": ...}}` for one that isn't UTF-8;
//! - `specs`: the spec models' records (`crate::specs::Record` as JSON: name, the spec's
//!   source, created, modified, version, history), without their trained weights;
//! - `pictures`: `[{id, mime, width, height, name, added, size, data}, ...]`, `data` the
//!   file's bytes in base64; `picture_lists`: `[[list name, picture id], ...]`.
//! - `textures`: the texture recipes (`graph::textures::Saved`: id, name, recipe, added).
//! - `lights`: the room's lights (`graph::lights::Light`), restored with their ids.
//! - `blocks`: `[{id, kind, texture, x, y, z, w, h, d}, ...]` (`graph::blocks`), restored
//!   with their ids.

use serde_json::{Value, json};

use super::codec::{Callee, Seg};
use super::interp::builtin;
use super::record::{self as rec, Past, mime};
use super::store::{GResult, Graph, Reader, err, rel};

fn key_json(id: u64) -> Value {
    json!(rec::key_of(id))
}

fn key_from(v: &Value) -> Option<u64> {
    match v {
        Value::Array(a) => {
            let bytes: Option<Vec<u8>> = a
                .iter()
                .map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()))
                .collect();
            rec::id_of(&bytes?)
        }
        Value::String(s) => rec::parse_label(s),
        _ => None,
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

pub fn run_json(run: &[Seg]) -> Value {
    Value::Array(
        run.iter()
            .map(|s| match s {
                Seg::Text(t) => json!(t),
                Seg::Ref(id) => json!({ "ref": key_json(*id) }),
                Seg::Call { f, args } => {
                    let args: Vec<Value> = args.iter().map(|a| run_json(a)).collect();
                    match f {
                        Callee::Builtin(b) => json!({
                            "call": builtin::NAMES.get(*b as usize).map(|n| n.to_string()).unwrap_or_else(|| b.to_string()),
                            "args": args
                        }),
                        Callee::Entry(id) => json!({ "fn": key_json(*id), "args": args }),
                    }
                }
            })
            .collect(),
    )
}

pub fn run_from(v: &Value) -> GResult<Vec<Seg>> {
    let Value::Array(items) = v else {
        return Err("a run is an array".into());
    };
    let mut out = Vec::new();
    for it in items {
        match it {
            Value::String(t) => out.push(Seg::Text(t.clone())),
            Value::Object(o) => {
                let args = || -> GResult<Vec<Vec<Seg>>> {
                    o.get("args")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().map(run_from).collect())
                        .unwrap_or(Ok(Vec::new()))
                };
                if let Some(k) = o.get("ref") {
                    out.push(Seg::Ref(key_from(k).ok_or("a bad ref key")?));
                } else if let Some(n) = o.get("call") {
                    let n = n.as_str().ok_or("a builtin's name is text")?;
                    let b = builtin::by_name(n)
                        .or_else(|| n.parse().ok())
                        .ok_or_else(|| format!("no builtin {n}"))?;
                    out.push(Seg::call(Callee::Builtin(b), args()?));
                } else if let Some(k) = o.get("fn") {
                    out.push(Seg::call(Callee::Entry(key_from(k).ok_or("a bad fn key")?), args()?));
                } else {
                    return Err("a run part that isn't text, ref, call or fn".into());
                }
            }
            _ => return Err("a run part that isn't text or an object".into()),
        }
    }
    Ok(super::codec::normalize(out))
}

/// what an import did
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imported {
    pub entries: usize,
    /// rows that were replaced (their key was taken), keeping the old text as a version
    pub replaced: usize,
    /// rows skipped (not `[text, key, ...]`, or a bad key)
    pub skipped: usize,
    /// entries stored partly as refs to others
    pub borrowing: usize,
}

impl Graph {
    /// imports texts rows: weum's `all-of-it.json` (`[[text, [key bytes]], ...]`) or this
    /// store's own `texts.json`. Keys become ids (`record::id_of`); rows without times
    /// get the import time. One transaction; borrowing applies in key order, so a variant
    /// borrows from shorter texts imported before it.
    pub fn import_texts(&self, json: &str) -> GResult<Imported> {
        let rows: Value = serde_json::from_str(json).map_err(err)?;
        let Value::Array(rows) = rows else {
            return Err("expected an array of [text, key] rows".into());
        };
        let mut parsed = Vec::with_capacity(rows.len());
        let mut out = Imported::default();
        for row in &rows {
            let Some(a) = row.as_array() else {
                out.skipped += 1;
                continue;
            };
            match (a.first().and_then(Value::as_str), a.get(1).and_then(key_from)) {
                (Some(t), Some(id)) if id <= super::store::MAX_ID => parsed.push((
                    id,
                    t.to_string(),
                    a.get(2).and_then(Value::as_u64),
                    a.get(3).and_then(Value::as_u64),
                    a.get(4).and_then(Value::as_str).map(str::to_string),
                )),
                _ => out.skipped += 1,
            }
        }
        // shorter texts first within the order keys give, so variants find their sources
        parsed.sort_by_key(|r| r.0);
        self.write(|tx| {
            for (id, text, created, edited, m) in parsed {
                let now = tx.now;
                let m = match m {
                    Some(name) => tx.mime_id(&name)?,
                    None => mime::TEXT,
                };
                let created = created.unwrap_or(now);
                if tx.exists(id)? {
                    out.replaced += 1;
                }
                tx.put_text_at(id, &text, m, created, edited.unwrap_or(created))?;
                out.entries += 1;
                if tx.record(id)?.is_some_and(|r| r.has_refs()) {
                    out.borrowing += 1;
                }
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// the two backup files: the texts, and everything else
    pub fn backup(&self) -> GResult<(String, String)> {
        let v = self.view()?;
        let all = v.all()?;
        let ids: Vec<u64> = all.iter().map(|e| e.id).collect();
        let texts = v.render_many(&ids);
        let mut rows = Vec::with_capacity(all.len());
        let mut sources = Vec::new();
        for (e, t) in all.iter().zip(texts) {
            let t = t.unwrap_or_default();
            let mut row = vec![
                json!(t),
                key_json(e.id),
                json!(e.record.created),
                json!(e.record.edited),
            ];
            if e.record.mime != mime::TEXT {
                row.push(json!(v.mime_name(e.record.mime)?));
            }
            rows.push(Value::Array(row));
            if !e.record.plain() {
                sources.push(json!([key_json(e.id), run_json(&v.source(e.id)?)]));
            }
        }
        let edges: Vec<Value> = v
            .all_edges()?
            .into_iter()
            .map(|(f, r, t, p)| json!([key_json(f), rel::name(r), key_json(t), hex(&p)]))
            .collect();
        let mut history = Vec::new();
        for e in &all {
            if v.history_rows(e.id)?.is_empty() {
                continue;
            }
            for ver in v.versions(e.id)?.into_iter().skip(1) {
                history.push(json!([key_json(e.id), ver.at, v.mime_name(ver.mime)?, ver.text]));
            }
        }
        let mut settings = serde_json::Map::new();
        for (k, b) in v.all_settings()? {
            settings.insert(
                k,
                match String::from_utf8(b) {
                    Ok(s) => json!(s),
                    Err(e) => json!({ "hex": hex(e.as_bytes()) }),
                },
            );
        }
        let mimes: serde_json::Map<String, Value> = v.custom_mimes()?.into_iter().map(|(n, i)| (n, json!(i))).collect();
        let other = json!({
            "format": "anend-graph",
            "version": 1,
            "allocated": v.allocated()?.map(key_json),
            "free": v.free_ids()?.into_iter().map(key_json).collect::<Vec<_>>(),
            "mimes": mimes,
            "sources": sources,
            "edges": edges,
            "history": history,
            "settings": settings,
            "specs": self.spec_records()?.iter()
                .filter_map(|(_, b)| serde_json::from_slice::<Value>(b).ok())
                .collect::<Vec<_>>(),
            "pictures": self.pictures()?.into_iter().map(|p| {
                let data = self.picture_bytes(&p.id).ok().flatten().unwrap_or_default();
                let mut v = json!(p);
                v["data"] = json!(crate::classifiers::base64(&data));
                v
            }).collect::<Vec<_>>(),
            "picture_lists": self.picture_list_rows()?,
            "textures": self.textures()?,
            "lights": self.lights()?,
            "scenes": self.scenes()?,
            "instances": self.instance_rows()?,
        });
        Ok((
            serde_json::to_string(&Value::Array(rows)).map_err(err)?,
            serde_json::to_string(&other).map_err(err)?,
        ))
    }

    /// restores a backup into this store (meant for an empty one; entries under the same
    /// keys are replaced)
    pub fn restore(&self, texts: &str, other: &str) -> GResult<Imported> {
        let o: Value = serde_json::from_str(other).map_err(err)?;
        if o.get("format").and_then(Value::as_str) != Some("anend-graph") {
            return Err("that isn't an anend graph backup".into());
        }
        // mimes first, so the texts' mime names map to the same ids
        self.write(|tx| {
            if let Some(m) = o.get("mimes").and_then(Value::as_object) {
                let mut named: Vec<(&String, u64)> = m.iter().filter_map(|(k, v)| v.as_u64().map(|i| (k, i))).collect();
                named.sort_by_key(|x| x.1);
                for (name, _) in named {
                    tx.mime_id(name)?;
                }
            }
            Ok(())
        })?;
        let report = self.import_texts(texts)?;
        self.write(|tx| {
            for s in o.get("sources").and_then(Value::as_array).into_iter().flatten() {
                let (Some(id), Some(run)) = (s.get(0).and_then(key_from), s.get(1)) else {
                    continue;
                };
                let run = run_from(run)?;
                if tx.exists(id)? {
                    tx.set_run_quietly(id, &run)?;
                }
            }
            for e in o.get("edges").and_then(Value::as_array).into_iter().flatten() {
                let from = e.get(0).and_then(key_from);
                let r = e.get(1).and_then(|r| match r {
                    Value::String(s) => rel::parse(s),
                    Value::Number(n) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
                    _ => None,
                });
                let to = e.get(2).and_then(key_from);
                let p = e.get(3).and_then(Value::as_str).and_then(unhex).unwrap_or_default();
                if let (Some(f), Some(r), Some(t)) = (from, r, to)
                    && tx.exists(f)?
                    && tx.exists(t)?
                {
                    tx.link(f, r, t, &p)?;
                }
            }
            // past versions, newest first per entry: each is a delta against the one after
            let mut newer: Option<(u64, String)> = None;
            for h in o.get("history").and_then(Value::as_array).into_iter().flatten() {
                let (Some(id), Some(at), Some(m), Some(text)) = (
                    h.get(0).and_then(key_from),
                    h.get(1).and_then(Value::as_u64),
                    h.get(2).and_then(Value::as_str),
                    h.get(3).and_then(Value::as_str),
                ) else {
                    continue;
                };
                if !tx.exists(id)? {
                    continue;
                }
                let base = match &newer {
                    Some((nid, t)) if *nid == id => t.clone(),
                    _ => tx.render(id)?,
                };
                let m = tx.mime_id(m)?;
                let (p, s) = rec::common_ends(text, &base);
                let past = Past::Delta {
                    mime: m,
                    prefix: p as u64,
                    suffix: s as u64,
                    middle: super::codec::pack_text(&text[p..text.len() - s]),
                };
                tx.put_past(id, at, &past)?;
                newer = Some((id, text.to_string()));
            }
            if let Some(a) = o.get("allocated").and_then(key_from) {
                tx.set_allocated(a)?;
            }
            for f in o.get("free").and_then(Value::as_array).into_iter().flatten() {
                if let Some(id) = key_from(f) {
                    tx.put_free(id)?;
                }
            }
            if let Some(s) = o.get("settings").and_then(Value::as_object) {
                for (k, v) in s {
                    let bytes = match v {
                        Value::String(t) => t.as_bytes().to_vec(),
                        other => other
                            .get("hex")
                            .and_then(Value::as_str)
                            .and_then(unhex)
                            .unwrap_or_default(),
                    };
                    tx.set_setting(k, &bytes)?;
                }
            }
            Ok(())
        })?;
        self.restore_assets(&o)?;
        Ok(report)
    }

    /// a backup's spec model records and pictures (rows that don't read are skipped)
    pub fn restore_assets(&self, o: &Value) -> GResult<()> {
        for r in o.get("specs").and_then(Value::as_array).into_iter().flatten() {
            if let Some(name) = r.get("name").and_then(Value::as_str) {
                self.put_spec_record(None, name, r.to_string().as_bytes())?;
            }
        }
        for p in o.get("pictures").and_then(Value::as_array).into_iter().flatten() {
            let Some(data) = p
                .get("data")
                .and_then(Value::as_str)
                .and_then(crate::classifiers::unbase64)
            else {
                continue;
            };
            let Ok(mut pic) = serde_json::from_value::<super::assets::Picture>(p.clone()) else {
                continue;
            };
            pic.id = super::assets::picture_id(&data);
            pic.size = data.len() as u64;
            self.put_picture(&pic, &data)?;
        }
        for row in o.get("picture_lists").and_then(Value::as_array).into_iter().flatten() {
            if let (Some(l), Some(id)) = (row.get(0).and_then(Value::as_str), row.get(1).and_then(Value::as_str))
                && self.picture(id)?.is_some()
            {
                self.tag_picture(id, l, true)?;
            }
        }
        for t in o.get("textures").and_then(Value::as_array).into_iter().flatten() {
            if let Ok(t) = serde_json::from_value::<super::textures::Saved>(t.clone()) {
                self.put_texture(&t.recipe, &t.name, t.added)?;
            }
        }
        let lights: Vec<super::lights::Light> = o
            .get("lights")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|l| serde_json::from_value(l.clone()).ok())
            .collect();
        self.put_lights(&lights)?;
        if let Some(scenes) = o.get("scenes").and_then(Value::as_array) {
            let scenes: Vec<super::scenes::Scene> = scenes
                .iter()
                .filter_map(|s| serde_json::from_value(s.clone()).ok())
                .collect();
            let rows: Vec<Value> = o
                .get("instances")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            self.put_instance_rows(&scenes, &rows)?;
        }
        // backups made before scenes: their blocks go into the room, numbered afresh
        let blocks: Vec<super::blocks::Block> = o
            .get("blocks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|b| {
                let mut b = b.clone();
                if b.get("id").is_some_and(Value::is_number) {
                    b["id"] = json!("");
                }
                serde_json::from_value(b).ok()
            })
            .collect();
        self.put_blocks(&blocks)?;
        // a backup made before scenes: its saved scenes (a setting then) become scenes
        self.migrate_maps()?;
        Ok(())
    }
}
