//! Pipes (docs/architecture.md, parts 2 and 3): texts flow from a filter to stacked
//! effects. A pipe is its filter (`filters::Expr`: words, sums, lists, tags, kinds and
//! what classifiers say, in groups; nothing given lets every text through), its
//! **effects** on the texts it lets through, shown wherever the pipe is turned on (a
//! colour, a colour per class of a classifier, an underline, a label, a size, a glow, and
//! in the space a pull that gathers them), and its **actions**, done on demand (put them
//! in a list, tag them, run a program on each).
//!
//! Nothing is stored about the texts: effects are worked out when a view asks for them
//! (`pipes.effects`), from the classifiers' memoized outputs, so a retrained classifier or
//! an edited text shows at once. Several pipes stack in the order they're turned on: a
//! later colour, underline or glow wins, sizes multiply, labels add up.
//!
//! A pipe is an entry of the save (mime `application/x-anend-pipe`, its text the [`Pipe`]'s
//! JSON), kept out of search and counts.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::taxonomy::{entries, put};
use super::{Res, Saves, s};
use crate::graph::Graph;
use crate::graph::record::{self, mime};

/// texts a pipe reads at most when a view asks for its effects, and when it acts
const MATCHED: u64 = 5000;
/// texts an action runs a program on at most
const RUNS: usize = 200;

/// what a pipe does to the texts it lets through, where it's turned on
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    /// the text in a colour (`#rrggbb`)
    Colour { colour: String },
    /// each text in its class's colour, as the classifier colours its classes
    ColourBy { classifier: String },
    /// an underline in a colour
    Underline { colour: String },
    /// a short label beside the text
    Label { text: String },
    /// the text larger or smaller (1: as it is)
    Size { scale: f32 },
    /// a glow in a colour
    Glow { colour: String },
    /// in the space, the texts pull together
    Gather,
}

/// what a pipe does once, when asked, to every text it lets through
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// into the list of that name (made when there's none)
    Enlist { list: String },
    /// tagged
    Tag { tag: String },
    /// an s program run on each, the text as its first argument
    Run { program: String },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pipe {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    /// the filter; null lets every text through
    #[serde(default)]
    pub from: Value,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub actions: Vec<Action>,
}

pub fn routed(op: &str) -> bool {
    op.starts_with("pipes.")
}

fn all(g: &Graph) -> Result<Vec<(u64, Pipe)>, String> {
    let mut out: Vec<(u64, Pipe)> = entries(g, mime::PIPE)?;
    out.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    Ok(out)
}

fn named(g: &Graph, name: &str) -> Result<(u64, Pipe), String> {
    all(g)?
        .into_iter()
        .find(|(_, p)| p.name == name)
        .ok_or_else(|| format!("there's no pipe called {name:?}"))
}

fn hex(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit())
}

/// the filter as it runs: an empty one lets everything through
fn expr(p: &Pipe) -> Value {
    if p.from.is_null() {
        json!({ "op": "and", "of": [] })
    } else {
        p.from.clone()
    }
}

fn checked(ws: &Saves, p: &Pipe) -> Result<(), String> {
    if p.name.trim().is_empty() {
        return Err("a pipe needs a name".into());
    }
    for e in &p.effects {
        match e {
            Effect::Colour { colour } | Effect::Underline { colour } | Effect::Glow { colour } if !hex(colour) => {
                return Err(format!("{colour:?} isn't a colour like #a0c4ff"));
            }
            Effect::Size { scale } if !(0.25..=4.0).contains(scale) => {
                return Err("a size goes from 0.25 to 4".into());
            }
            Effect::Label { text } if text.chars().count() > 40 => {
                return Err("a label is at most 40 characters".into());
            }
            Effect::ColourBy { classifier } => {
                ws.call("classify.get", &json!({ "name": classifier }))?;
            }
            _ => {}
        }
    }
    for a in &p.actions {
        match a {
            Action::Enlist { list } if list.trim().is_empty() => return Err("which list?".into()),
            Action::Tag { tag } if tag.trim().is_empty() => return Err("which tag?".into()),
            Action::Run { program } => {
                program_key(ws, program)?;
            }
            _ => {}
        }
    }
    // the filter reads, and asks only classifiers that are there
    ws.call("filters.run", &json!({ "expr": expr(p), "limit": 1 }))?;
    Ok(())
}

fn program_key(ws: &Saves, title: &str) -> Result<String, String> {
    let all = ws.call("s.programs", &json!({}))?;
    all.as_array()
        .into_iter()
        .flatten()
        .find(|x| x["title"] == title)
        .and_then(|x| x["key"].as_str().map(String::from))
        .ok_or_else(|| format!("there's no s program called {title:?}"))
}

/// what the pipe lets through: `(key, text)`, strongest first
fn matched(ws: &Saves, p: &Pipe, limit: u64) -> Result<(Vec<(String, String)>, Value), String> {
    let r = ws.call("filters.run", &json!({ "expr": expr(p), "limit": limit }))?;
    let found = r["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| {
            Some((
                h["key"].as_str()?.to_string(),
                h["text"].as_str().unwrap_or("").to_string(),
            ))
        })
        .collect();
    Ok((found, r))
}

/// each key's stacked effects from these pipes, in order
fn effects(ws: &Saves, pipes: &[Pipe], keys: &[String]) -> Result<Vec<Value>, String> {
    let want: HashSet<&str> = keys.iter().map(String::as_str).collect();
    let mut out: BTreeMap<String, serde_json::Map<String, Value>> = BTreeMap::new();
    for p in pipes {
        let (found, _) = matched(ws, p, MATCHED)?;
        let here: Vec<String> = found
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| want.contains(k.as_str()))
            .collect();
        if here.is_empty() {
            continue;
        }
        // a classifier's colour for each text it reads
        let mut by: HashMap<String, String> = HashMap::new();
        for e in &p.effects {
            if let Effect::ColourBy { classifier } = e {
                let t = ws.call("classify.tints", &json!({ "name": classifier, "keys": here }))?;
                let palette: Vec<String> = t["palette"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.as_str().map(String::from))
                    .collect();
                for x in t["tints"].as_array().into_iter().flatten() {
                    if let (Some(k), Some(c)) = (x["key"].as_str(), x["class"].as_u64())
                        && let Some(col) = palette.get(c as usize)
                    {
                        by.insert(k.to_string(), col.clone());
                    }
                }
            }
        }
        for k in here {
            let o = out.entry(k.clone()).or_default();
            if let Some(a) = o.entry("pipes").or_insert_with(|| json!([])).as_array_mut() {
                a.push(json!(p.name));
            }
            for e in &p.effects {
                match e {
                    Effect::Colour { colour } => {
                        o.insert("colour".into(), json!(colour));
                    }
                    Effect::ColourBy { .. } => {
                        if let Some(c) = by.get(&k) {
                            o.insert("colour".into(), json!(c));
                        }
                    }
                    Effect::Underline { colour } => {
                        o.insert("underline".into(), json!(colour));
                    }
                    Effect::Label { text } => {
                        let l = match o.get("label").and_then(Value::as_str) {
                            Some(was) => format!("{was} · {text}"),
                            None => text.clone(),
                        };
                        o.insert("label".into(), json!(l));
                    }
                    Effect::Size { scale } => {
                        let was = o.get("size").and_then(Value::as_f64).unwrap_or(1.0);
                        o.insert("size".into(), json!((was * *scale as f64).clamp(0.25, 4.0)));
                    }
                    Effect::Glow { colour } => {
                        o.insert("glow".into(), json!(colour));
                    }
                    Effect::Gather => {
                        o.insert("gather".into(), json!(p.name));
                    }
                }
            }
        }
    }
    Ok(out
        .into_iter()
        .map(|(k, mut o)| {
            o.insert("key".into(), json!(k));
            Value::Object(o)
        })
        .collect())
}

pub fn call(ws: &Saves, g: &Graph, op: &str, args: &Value) -> Res {
    match op {
        "pipes.list" => Ok(json!(
            all(g)?
                .into_iter()
                .map(|(id, p)| json!({ "key": record::label(id), "pipe": p }))
                .collect::<Vec<_>>()
        )),
        "pipes.save" => {
            let mut p: Pipe = serde_json::from_value(args.get("pipe").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("that isn't a pipe: {e}"))?;
            p.name = p.name.trim().to_string();
            checked(ws, &p)?;
            let fresh = args.get("new").and_then(Value::as_bool).unwrap_or(false);
            let was = match s(args, "was") {
                "" => p.name.clone(),
                w => w.to_string(),
            };
            let old = if fresh {
                None
            } else {
                all(g)?.into_iter().find(|(_, o)| o.name == was)
            };
            if (fresh || was != p.name) && all(g)?.iter().any(|(_, o)| o.name == p.name) {
                return Err(format!("there's a pipe called {:?} already", p.name));
            }
            let id = put(g, old.map(|(id, _)| id), mime::PIPE, &p)?;
            Ok(json!({ "key": record::label(id), "pipe": p }))
        }
        "pipes.delete" => {
            let (id, p) = named(g, s(args, "name"))?;
            g.delete(id)?;
            Ok(json!({ "removed": p.name }))
        }
        // what a pipe (kept, or given) lets through, and what it does to them
        "pipes.run" => {
            let p = match args.get("pipe") {
                Some(p) if !p.is_null() => {
                    serde_json::from_value(p.clone()).map_err(|e| format!("that isn't a pipe: {e}"))?
                }
                _ => named(g, s(args, "name"))?.1,
            };
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(200)
                .clamp(1, MATCHED);
            let (found, r) = matched(ws, &p, limit)?;
            let keys: Vec<String> = found.iter().map(|(k, _)| k.clone()).collect();
            let fx = effects(ws, std::slice::from_ref(&p), &keys)?;
            Ok(json!({
                "scanned": r["scanned"], "matched": r["matched"], "results": r["results"],
                "untrained": r["untrained"], "effects": fx,
            }))
        }
        // each key's stacked effects from the pipes turned on, in their order
        "pipes.effects" => {
            let names: Vec<String> = serde_json::from_value(args.get("names").cloned().unwrap_or(json!([])))
                .map_err(|_| "names is a list of pipes".to_string())?;
            let keys: Vec<String> = serde_json::from_value(args.get("keys").cloned().unwrap_or(json!([])))
                .map_err(|_| "keys is a list of keys".to_string())?;
            if keys.len() > MATCHED as usize {
                return Err(format!("at most {MATCHED} keys at a time"));
            }
            let kept = all(g)?;
            let mut pipes = Vec::new();
            let mut missing = Vec::new();
            for n in names {
                match kept.iter().find(|(_, p)| p.name == n) {
                    Some((_, p)) => pipes.push(p.clone()),
                    None => missing.push(n),
                }
            }
            Ok(json!({ "effects": effects(ws, &pipes, &keys)?, "missing": missing }))
        }
        // the pipe's actions, once, on every text it lets through
        "pipes.apply" => {
            let (_, p) = named(g, s(args, "name"))?;
            if p.actions.is_empty() {
                return Err(format!("{:?} has no actions", p.name));
            }
            let (found, _) = matched(ws, &p, MATCHED)?;
            let mut done = Vec::new();
            for a in &p.actions {
                match a {
                    Action::Enlist { list } => {
                        let head = ws.call("mem.saturate", &json!({ "text": list }))?;
                        let head = head["key"].as_str().unwrap_or("").to_string();
                        let mut n = 0;
                        for (k, _) in &found {
                            if *k != head {
                                ws.call("lists.enlist", &json!({ "list": head, "member": k }))?;
                                n += 1;
                            }
                        }
                        done.push(json!({ "kind": "enlist", "list": list, "key": head, "texts": n }));
                    }
                    Action::Tag { tag } => {
                        for (k, _) in &found {
                            ws.call("tags.add", &json!({ "key": k, "tag": tag }))?;
                        }
                        done.push(json!({ "kind": "tag", "tag": tag, "texts": found.len() }));
                    }
                    Action::Run { program } => {
                        let key = program_key(ws, program)?;
                        let mut outs = Vec::new();
                        let mut failed = 0;
                        for (_, t) in found.iter().take(RUNS) {
                            let r = ws.call("s.run", &json!({ "key": key, "args": [t] }))?;
                            if r["err"].is_string() {
                                failed += 1;
                            }
                            let o = r["out"].as_str().unwrap_or("").trim().to_string();
                            if !o.is_empty() {
                                outs.push(o);
                            }
                        }
                        done.push(json!({
                            "kind": "run", "program": program, "texts": found.len().min(RUNS),
                            "left": found.len().saturating_sub(RUNS), "failed": failed, "out": outs,
                        }));
                    }
                }
            }
            Ok(json!({ "name": p.name, "texts": found.len(), "done": done }))
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}
