//! Views as programs (docs/architecture.md, part 4): a view is where its texts come from
//! (a pipe, or a search by words, kind and tag), how they're ordered and grouped, the
//! pipes whose effects it shows, and a layout. The reader shows any view.
//!
//! The reader's own three (everything, notes, texts) are built-in presets of this, and a
//! view of one's own is an s program that loads `std/view.s.` and says what the view is
//! (`{cards} layout`, `{fruity} from`, `{newest} order`, `{things} group`, …); the
//! reader's bar words come in as its first argument. So a new kind of view needs no new
//! code in the app, only a program.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{Res, Saves, s};
use crate::graph::record;
use crate::graph::{Graph, Reader};

/// texts a view holds at most
const MOST: u64 = 20_000;

/// the built-in presets: the reader's own views
const PRESETS: [(&str, &str, &str); 3] = [
    ("everything", "everything", "every note and text, newest first"),
    ("notes", "notes", "the notes, newest first"),
    ("texts", "texts", "the texts, newest first"),
];

pub fn routed(op: &str) -> bool {
    op.starts_with("views.")
}

/// whether a program says what a view is
fn is_view(code: &str) -> bool {
    code.contains("std/view.s.") || code.contains("std:view")
}

/// a view as it's described: field → value, and the pipes for its effects
type Described = (BTreeMap<String, String>, Vec<String>);

fn describe(ws: &Saves, g: &Graph, name: &str, words: &str) -> Result<Described, String> {
    if let Some((_, kind, about)) = PRESETS.iter().find(|p| p.0 == name) {
        let mut f = BTreeMap::new();
        f.insert("kind".into(), kind.to_string());
        f.insert("layout".into(), "auto".into());
        f.insert("order".into(), "found".into());
        f.insert("title".into(), about.to_string());
        f.insert("words".into(), words.to_string());
        return Ok((f, vec![]));
    }
    let progs = super::script::programs(g)?;
    let (_, code) = progs
        .get(name)
        .filter(|(_, c)| is_view(c))
        .ok_or_else(|| format!("there's no view called {name:?}"))?;
    let mut files = BTreeMap::new();
    for (title, (_, body)) in &progs {
        files.insert(format!("{title}.s"), body.clone());
        files.insert(title.clone(), body.clone());
    }
    let host = |op: &str, a: &Value| ws.hosted(op, a);
    let (o, mut f, fx) = crate::s::run_view(code, files, &[words.to_string()], 2_000_000, Some(&host));
    if let Some(e) = o.err {
        return Err(format!("the view {name:?} stopped: {e}"));
    }
    // the bar's words narrow any view
    if !words.trim().is_empty() {
        let w = match f.get("words") {
            Some(had) if !had.trim().is_empty() => format!("{had} {words}"),
            _ => words.to_string(),
        };
        f.insert("words".into(), w);
    }
    f.entry("title".into()).or_insert_with(|| name.to_string());
    Ok((f, fx))
}

/// a hit with what the view needs to order and group it
struct Item {
    hit: Value,
    created: u64,
}

fn items_of(g: &Graph, hits: &[Value]) -> Result<Vec<Item>, String> {
    let v = g.view()?;
    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        let created = record::parse_label(h["key"].as_str().unwrap_or(""))
            .and_then(|id| v.record(id).ok().flatten())
            .map(|r| r.created)
            .unwrap_or(0);
        out.push(Item {
            hit: h.clone(),
            created,
        });
    }
    Ok(out)
}

fn run(ws: &Saves, g: &Graph, name: &str, words: &str, tag: Option<&str>) -> Res {
    let (f, fx) = describe(ws, g, name, words)?;
    let get = |k: &str| f.get(k).map(String::as_str).unwrap_or("");
    let limit = get("limit")
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .unwrap_or(MOST)
        .min(MOST);
    let kind = match get("kind") {
        "notes" => "notes",
        "texts" => "memories",
        _ => "",
    };
    let tag = tag
        .map(str::to_string)
        .or_else(|| Some(get("tag").to_string()).filter(|t| !t.is_empty()));
    // where its texts come from
    let (total, hits): (u64, Vec<Value>) = if !get("from").is_empty() {
        let r = ws.call("pipes.run", &json!({ "name": get("from"), "limit": limit.min(5000) }))?;
        let w: Vec<String> = get("words").split_whitespace().map(str::to_lowercase).collect();
        let hits: Vec<Value> = r["results"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|h| match kind {
                "notes" => h["note"] == true,
                "memories" => h["note"] != true,
                _ => true,
            })
            .filter(|h| {
                let t = h["text"].as_str().unwrap_or("").to_lowercase();
                w.iter().all(|x| t.contains(x.as_str()))
            })
            .cloned()
            .collect();
        (hits.len() as u64, hits)
    } else {
        let mut q = json!({ "words": get("words"), "kind": kind, "limit": limit });
        if let Some(t) = &tag {
            q["tag"] = json!(t);
        }
        let r = ws.call("entries.query", &q)?;
        (
            r["total"].as_u64().unwrap_or(0),
            r["hits"].as_array().cloned().unwrap_or_default(),
        )
    };
    let mut items = items_of(g, &hits)?;
    let text = |i: &Item| i.hit["text"].as_str().unwrap_or("").to_string();
    match get("order") {
        "newest" => items.sort_by_key(|i| std::cmp::Reverse(i.created)),
        "oldest" => items.sort_by_key(|i| i.created),
        "ordinal" => items.sort_by_key(|i| i.hit["ordinal"].as_i64().unwrap_or(0)),
        "reversed" => items.sort_by_key(|i| i.hit["reversed"].as_i64().unwrap_or(0)),
        "alphabetical" => items.sort_by_key(|i| text(i).to_lowercase()),
        "length" => items.sort_by_key(|i| text(i).chars().count()),
        _ => {}
    }
    // headings
    let group = get("group");
    let mut heads: Vec<String> = match group {
        "" => vec![String::new(); items.len()],
        "kind" => items
            .iter()
            .map(|i| {
                if i.hit["note"] == true {
                    "notes".to_string()
                } else {
                    "texts".to_string()
                }
            })
            .collect(),
        "ordinal" | "reversed" => items
            .iter()
            .map(|i| format!("{group} {}", i.hit[group].as_i64().unwrap_or(0)))
            .collect(),
        "length" => items
            .iter()
            .map(|i| match text(i).chars().count() {
                0..=12 => "a word or two".to_string(),
                13..=60 => "a line".to_string(),
                61..=400 => "a paragraph".to_string(),
                _ => "long".to_string(),
            })
            .collect(),
        classifier => {
            let keys: Vec<Value> = items.iter().take(5000).map(|i| i.hit["key"].clone()).collect();
            let t = ws.call("classify.tints", &json!({ "name": classifier, "keys": keys }))?;
            let classes: Vec<String> = t["classes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| c.as_str().unwrap_or("").to_string())
                .collect();
            let by: BTreeMap<String, String> = t["tints"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| {
                    let c = x["class"]
                        .as_u64()
                        .and_then(|c| classes.get(c as usize).cloned())
                        .unwrap_or_else(|| "uncertain".into());
                    (x["key"].as_str().unwrap_or("").to_string(), c)
                })
                .collect();
            items
                .iter()
                .map(|i| {
                    by.get(i.hit["key"].as_str().unwrap_or(""))
                        .cloned()
                        .unwrap_or_else(|| "uncertain".into())
                })
                .collect()
        }
    };
    // a group is together, in the order its first text came
    if !group.is_empty() {
        let mut first: BTreeMap<String, usize> = BTreeMap::new();
        for (i, h) in heads.iter().enumerate() {
            first.entry(h.clone()).or_insert(i);
        }
        let mut idx: Vec<usize> = (0..items.len()).collect();
        idx.sort_by_key(|&i| (first[&heads[i]], i));
        let mut it: Vec<Option<Item>> = items.into_iter().map(Some).collect();
        items = idx.iter().map(|&i| it[i].take().expect("each once")).collect();
        heads = idx.iter().map(|&i| heads[i].clone()).collect();
    }
    let mut groups: Vec<Value> = Vec::new();
    for (i, h) in heads.iter().enumerate() {
        if i == 0 || heads[i - 1] != *h {
            groups.push(json!({ "name": h, "from": i, "count": 0 }));
        }
        if let Some(g) = groups.last_mut() {
            g["count"] = json!(g["count"].as_u64().unwrap_or(0) + 1);
        }
    }
    // the effects of its pipes (and of the pipe it comes from)
    let mut names = fx;
    if !get("from").is_empty() && !names.iter().any(|n| n == get("from")) {
        names.insert(0, get("from").to_string());
    }
    let mut effects = serde_json::Map::new();
    if !names.is_empty() && !items.is_empty() {
        let keys: Vec<Value> = items.iter().take(5000).map(|i| i.hit["key"].clone()).collect();
        let r = ws.call("pipes.effects", &json!({ "names": names, "keys": keys }))?;
        for e in r["effects"].as_array().into_iter().flatten() {
            effects.insert(e["key"].as_str().unwrap_or("").to_string(), e.clone());
        }
    }
    let hits: Vec<Value> = items
        .into_iter()
        .map(|i| {
            let mut h = i.hit;
            if let Some(e) = effects.get(h["key"].as_str().unwrap_or("")) {
                h["fx"] = e.clone();
            }
            h
        })
        .collect();
    Ok(json!({
        "name": name, "title": get("title"), "layout": if get("layout").is_empty() { "auto" } else { get("layout") },
        "total": total, "hits": hits, "groups": groups, "described": f,
    }))
}

pub fn call(ws: &Saves, g: &Graph, op: &str, args: &Value) -> Res {
    match op {
        // the presets and the save's view programs
        "views.list" => {
            let mut out: Vec<Value> = PRESETS
                .iter()
                .map(|(n, _, about)| json!({ "name": n, "builtin": true, "about": about }))
                .collect();
            for (title, (key, code)) in super::script::programs(g)? {
                if is_view(&code) {
                    out.push(json!({ "name": title, "builtin": false, "key": key }));
                }
            }
            Ok(json!(out))
        }
        "views.run" => run(
            ws,
            g,
            s(args, "name"),
            s(args, "words"),
            args.get("tag").and_then(Value::as_str),
        ),
        _ => Err(format!("unknown op {op:?}")),
    }
}
