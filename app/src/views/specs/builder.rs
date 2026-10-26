//! The spec builder: every part of a model spec (docs/models.md) as a control, so a model
//! is made without typing JSON. It edits the same JSON the JSON view shows (each change is
//! written back to it, and a change in the JSON view shows here), so the two stay in step.
//!
//! The parts: the model (name, version, seed, where it runs, a pretrained start), what it
//! reads (text with its feature channels, bytes, pictures with their size and patches,
//! named numbers with their width), the trunk (a preset and any size of its own), the heads
//! (each with its kind, classes and weight), the data (the save's lists, tags, list pairs,
//! picture lists, or examples written in, picked from what the save has), and for a
//! generator its critics (other models and what they should say), its constraints and how
//! it samples; then training. A diagram shows how the parts connect. A broken rule shows
//! on the control it concerns (and at the top, with where it is).
//!
//! [`COVERED`] lists every field the spec has, as a path; a test in this file walks a spec
//! that uses every part and checks each field it finds is here and has a control.

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::ui::*;

/// every field of the spec that has a control, as a path (`[]` for any index)
#[cfg_attr(not(test), allow(dead_code))]
pub const COVERED: &[&str] = &[
    "version",
    "name",
    "seed",
    "device",
    "from.pretrained",
    "input.type",
    "input.features",
    "input.width",
    "input.height",
    "input.patch",
    "input.codes",
    "input.names",
    "input.codec.digits",
    "input.codec.decimals",
    "input.codec.signed",
    "trunk.preset",
    "trunk.d_model",
    "trunk.layers",
    "trunk.heads",
    "trunk.head_dim",
    "trunk.d_ff",
    "trunk.max_len",
    "trunk.causal",
    "trunk.pooling",
    "trunk.dropout",
    "heads[].name",
    "heads[].kind",
    "heads[].classes",
    "heads[].weight",
    "data[].from",
    "data[].list",
    "data[].tag",
    "data[].input",
    "data[].output",
    "data[].head",
    "data[].class",
    "data[].value",
    "data[].items[].input.text",
    "data[].items[].input.bytes",
    "data[].items[].input.numbers",
    "data[].items[].input.image.width",
    "data[].items[].input.image.height",
    "data[].items[].input.image.rgb",
    "data[].items[].head",
    "data[].items[].class",
    "data[].items[].value",
    "critics[].model",
    "critics[].head",
    "critics[].weight",
    "critics[].want.class",
    "critics[].want.not",
    "critics[].want.near.value",
    "critics[].want.near.target",
    "critics[].want.near.scale",
    "critics[].want.range.value",
    "critics[].want.range.lo",
    "critics[].want.range.hi",
    "critics[].want.range.scale",
    "constraints.min_len",
    "constraints.max_len",
    "constraints.gates[].critic",
    "constraints.gates[].class",
    "constraints.gates[].member",
    "constraints.sums[].measure",
    "constraints.sums[].lo",
    "constraints.sums[].hi",
    "constraints.sums[].modulo",
    "constraints.contains",
    "constraints.excludes",
    "sampling.temperature",
    "sampling.top_k",
    "sampling.top_p",
    "sampling.max_len",
    "sampling.stop",
    "sampling.seed",
    "train.epochs",
    "train.batch",
    "train.lr",
    "train.val_fraction",
    "train.patience",
    "train.max_steps",
    "train.threads",
    "train.budget_seconds",
];

// ------------------------------------------------------------------ paths

enum Seg {
    Key(String),
    Idx(usize),
}

/// `a.b[2].c` as its steps
fn parts(path: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    for piece in path.split('.') {
        let (key, rest) = piece.split_once('[').map_or((piece, ""), |(k, r)| (k, r));
        if !key.is_empty() {
            out.push(Seg::Key(key.to_string()));
        }
        for i in rest.split('[') {
            if let Ok(n) = i.trim_end_matches(']').parse() {
                out.push(Seg::Idx(n));
            }
        }
    }
    out
}

/// the value at a path (null when it isn't there)
fn at<'a>(v: &'a Value, path: &str) -> &'a Value {
    let mut cur = v;
    for s in parts(path) {
        cur = match s {
            Seg::Key(k) => &cur[k.as_str()],
            Seg::Idx(i) => &cur[i],
        };
    }
    cur
}

/// sets the value at a path, making what's missing on the way; null takes a key out
fn put(v: &mut Value, path: &str, val: Value) {
    let ps = parts(path);
    let mut cur = v;
    for (n, s) in ps.iter().enumerate() {
        let last = n + 1 == ps.len();
        match s {
            Seg::Key(k) => {
                if !cur.is_object() {
                    *cur = json!({});
                }
                let o = cur.as_object_mut().expect("an object");
                if last {
                    if val.is_null() {
                        o.remove(k);
                    } else {
                        o.insert(k.clone(), val);
                    }
                    return;
                }
                cur = o.entry(k.clone()).or_insert(Value::Null);
            }
            Seg::Idx(i) => {
                if !cur.is_array() {
                    *cur = json!([]);
                }
                let a = cur.as_array_mut().expect("an array");
                while a.len() <= *i {
                    a.push(Value::Null);
                }
                if last {
                    a[*i] = val;
                    return;
                }
                cur = &mut a[*i];
            }
        }
    }
}

/// takes item `i` out of the array at a path
fn remove_at(v: &mut Value, path: &str, i: usize) {
    if let Some(a) = at_mut(v, path).and_then(Value::as_array_mut)
        && i < a.len()
    {
        a.remove(i);
    }
}

fn at_mut<'a>(v: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    let mut cur = v;
    for s in parts(path) {
        cur = match s {
            Seg::Key(k) => cur.get_mut(k.as_str())?,
            Seg::Idx(i) => cur.get_mut(i)?,
        };
    }
    Some(cur)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// a change to the spec: set a path, take one out (null), or edit it whole
#[derive(Clone, PartialEq)]
enum Change {
    Set(String, Value),
    Remove(String, usize),
    Push(String, Value),
}

/// the broken rule, when it's on this path
fn err_here(err: &Option<(String, String)>, path: &str) -> Option<String> {
    err.as_ref().filter(|(p, _)| p == path).map(|(_, m)| m.clone())
}

// ------------------------------------------------------------------ the save's names

/// a model's head: its name, kind and classes
type HeadInfo = (String, String, Vec<String>);

#[derive(Clone, PartialEq, Default)]
struct Names {
    lists: Vec<String>,
    tags: Vec<String>,
    picture_lists: Vec<String>,
    /// pictures: id and name
    pictures: Vec<(String, String)>,
    /// other spec models: name and heads (name, kind, classes)
    models: Vec<(String, Vec<HeadInfo>)>,
    /// pretrained folders on a shared hub; any folder when `any_folder`
    folders: Vec<String>,
    any_folder: bool,
}

// ------------------------------------------------------------------ the controls

/// a labelled control, with its help and, when it's broken, the rule
fn row(label: &str, help: &str, err: Option<String>, body: Element) -> Element {
    rsx! {
        div { class: if err.is_some() { "sb-row bad" } else { "sb-row" },
            span { class: "sb-label", "{label}" }
            div { class: "sb-ctl",
                {body}
                if !help.is_empty() { span { class: "sb-help", "{help}" } }
                if let Some(e) = err.clone() { span { class: "sb-err", role: "alert", Icon { name: "alert" } "{e}" } }
            }
        }
    }
}

/// a whole number (empty: left out, the default)
fn num(v: &Value, path: String, min: f64, max: f64, step: f64, placeholder: &str, ch: EventHandler<Change>) -> Element {
    let cur = at(v, &path);
    let shown = cur.as_f64().map(|x| x.to_string()).unwrap_or_default();
    let int = step >= 1.0;
    rsx! {
        input { r#type: "number", class: "sb-num", min: "{min}", max: "{max}", step: "{step}", placeholder: "{placeholder}", value: "{shown}",
            "aria-label": "{path}",
            onchange: move |e| {
                let t = e.value();
                let val = match t.trim().parse::<f64>() {
                    Ok(x) if int => json!(x.clamp(min, max).round() as i64),
                    Ok(x) => json!(x.clamp(min, max)),
                    Err(_) => Value::Null,
                };
                ch.call(Change::Set(path.clone(), val));
            } }
    }
}

/// a slider with its value beside it, and "default" to leave it out when `optional`
fn slide(
    v: &Value,
    path: String,
    (min, max, step, def): (f64, f64, f64, f64),
    optional: bool,
    ch: EventHandler<Change>,
) -> Element {
    let cur = at(v, &path).as_f64();
    let shown = cur.unwrap_or(def);
    let p2 = path.clone();
    rsx! {
        span { class: "sb-slide",
            input { r#type: "range", min: "{min}", max: "{max}", step: "{step}", value: "{shown}", "aria-label": "{path}",
                oninput: move |e| ch.call(Change::Set(path.clone(), json!(e.value().parse::<f64>().unwrap_or(def)))) }
            span { class: "sb-val", if cur.is_some() || !optional { {fmt(shown)} } else { "default" } }
            if optional && cur.is_some() {
                button { class: "plain small", title: "leave it to the default", onclick: move |_| ch.call(Change::Set(p2.clone(), Value::Null)), "default" }
            }
        }
    }
}

fn fmt(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e9 {
        format!("{x:.0}")
    } else {
        format!("{x:.3}").trim_end_matches('0').to_string()
    }
}

/// one of `options` (value, words), kept as text; `none` (when given) leaves it out
fn pick(
    v: &Value,
    path: String,
    options: Vec<(String, String)>,
    none: Option<&str>,
    ch: EventHandler<Change>,
) -> Element {
    choose(v, path, options, none, false, ch)
}

/// the same, its values numbers or true / false
fn pick_json(
    v: &Value,
    path: String,
    options: Vec<(String, String)>,
    none: Option<&str>,
    ch: EventHandler<Change>,
) -> Element {
    choose(v, path, options, none, true, ch)
}

fn choose(
    v: &Value,
    path: String,
    options: Vec<(String, String)>,
    none: Option<&str>,
    typed: bool,
    ch: EventHandler<Change>,
) -> Element {
    let cur = at(v, &path);
    let now = match cur {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let known = options.iter().any(|(o, _)| *o == now);
    rsx! {
        select { class: "sb-pick", "aria-label": "{path}",
            onchange: move |e| {
                let t = e.value();
                let val = if t.is_empty() {
                    Value::Null
                } else if typed {
                    serde_json::from_str::<Value>(&t).unwrap_or(Value::String(t))
                } else {
                    Value::String(t)
                };
                ch.call(Change::Set(path.clone(), val));
            },
            if let Some(n) = none { option { value: "", selected: now.is_empty(), "{n}" } }
            if !known && !now.is_empty() { option { value: "{now}", selected: true, "{now}" } }
            for (o , w) in options.iter() {
                option { key: "{o}", value: "{o}", selected: *o == now, "{w}" }
            }
        }
    }
}

/// a line of text (empty: left out when `optional`)
fn line(v: &Value, path: String, placeholder: &str, optional: bool, ch: EventHandler<Change>) -> Element {
    let shown = at(v, &path).as_str().unwrap_or("").to_string();
    rsx! {
        input { r#type: "text", class: "sb-text", placeholder: "{placeholder}", value: "{shown}", "aria-label": "{path}",
            onchange: move |e| {
                let t = e.value();
                ch.call(Change::Set(path.clone(), if t.is_empty() && optional { Value::Null } else { json!(t) }));
            } }
    }
}

/// on or off
fn tick(v: &Value, path: String, words: &str, ch: EventHandler<Change>) -> Element {
    let on = at(v, &path).as_bool().unwrap_or(false);
    rsx! {
        label { class: "sb-tick",
            input { r#type: "checkbox", checked: on, "aria-label": "{path}", oninput: move |e| ch.call(Change::Set(path.clone(), json!(e.checked()))) }
            "{words}"
        }
    }
}

/// a list of words as chips: × takes one out, Enter adds what's typed
fn chips(v: &Value, path: String, placeholder: &str, ch: EventHandler<Change>) -> Element {
    let all = strings(at(v, &path));
    let p_add = path.clone();
    rsx! {
        span { class: "sb-chips",
            for (i , w) in all.iter().enumerate() {
                span { key: "{i}-{w}", class: "chip",
                    "{w}"
                    button { class: "plain small", "aria-label": "take {w} out", onclick: { let p = path.clone(); move |_| ch.call(Change::Remove(p.clone(), i)) }, "×" }
                }
            }
            input { r#type: "text", class: "sb-chip-in", placeholder: "{placeholder}", "aria-label": "{path}: add one",
                onkeydown: move |e: KeyboardEvent| {
                    if e.key() == Key::Enter {
                        e.prevent_default();
                    }
                },
                onchange: move |e| {
                    let t = e.value().trim().to_string();
                    if !t.is_empty() {
                        ch.call(Change::Push(p_add.clone(), json!(t)));
                    }
                } }
        }
    }
}

/// byte strings (constraints' contains / excludes) as chips of text
fn byte_chips(v: &Value, path: String, placeholder: &str, ch: EventHandler<Change>) -> Element {
    let all: Vec<String> = at(v, &path)
        .as_array()
        .map(|a| {
            a.iter()
                .map(|b| {
                    let bytes: Vec<u8> = b
                        .as_array()
                        .map(|x| x.iter().filter_map(|n| n.as_u64().map(|n| n as u8)).collect())
                        .unwrap_or_default();
                    String::from_utf8_lossy(&bytes).into_owned()
                })
                .collect()
        })
        .unwrap_or_default();
    let p_add = path.clone();
    rsx! {
        span { class: "sb-chips",
            for (i , w) in all.iter().enumerate() {
                span { key: "{i}-{w}", class: "chip",
                    "“{w}”"
                    button { class: "plain small", "aria-label": "take it out", onclick: { let p = path.clone(); move |_| ch.call(Change::Remove(p.clone(), i)) }, "×" }
                }
            }
            input { r#type: "text", class: "sb-chip-in", placeholder: "{placeholder}", "aria-label": "{path}: add one",
                onchange: move |e| {
                    let t = e.value();
                    if !t.is_empty() {
                        ch.call(Change::Push(p_add.clone(), json!(t.as_bytes())));
                    }
                } }
        }
    }
}

/// one number per name (a regress head's values)
fn values(v: &Value, path: String, names: &[String], ch: EventHandler<Change>) -> Element {
    let cur: Vec<f64> = at(v, &path)
        .as_array()
        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect())
        .unwrap_or_default();
    let n = names.len().max(1);
    rsx! {
        span { class: "sb-values",
            for i in 0..n {
                label { key: "{i}", class: "sb-value",
                    span { class: "dim small", {names.get(i).cloned().unwrap_or_else(|| "value".into())} }
                    input { r#type: "number", class: "sb-num", step: "any", value: "{cur.get(i).copied().unwrap_or(0.0)}", "aria-label": "{path}[{i}]",
                        onchange: {
                            let path = path.clone();
                            let cur = cur.clone();
                            move |e: FormEvent| {
                                let mut all = cur.clone();
                                all.resize(n, 0.0);
                                all[i] = e.value().parse().unwrap_or(0.0);
                                ch.call(Change::Set(path.clone(), json!(all)));
                            }
                        } }
                }
            }
        }
    }
}

fn opts(xs: &[&str]) -> Vec<(String, String)> {
    xs.iter().map(|x| (x.to_string(), x.replace('_', " "))).collect()
}

fn named(xs: &[String]) -> Vec<(String, String)> {
    xs.iter().map(|x| (x.clone(), x.clone())).collect()
}

/// what an inline picture example's input holds, written whole from a picture of the save
/// (`pictures.get` with `rgb`)
#[allow(dead_code)]
const IMAGE_FIELDS: [&str; 3] = [
    "data[].items[].input.image.width",
    "data[].items[].input.image.height",
    "data[].items[].input.image.rgb",
];

// ------------------------------------------------------------------ the builder

#[component]
pub fn SpecBuilder(text: String, on_change: EventHandler<String>, err: Option<(String, String)>) -> Element {
    let api = api();
    let spec = use_memo(use_reactive!(|text| serde_json::from_str::<Value>(&text).ok()));
    // the save's lists, tags, pictures, other models and pretrained folders, for the pickers
    let names = use_resource(move || {
        let api = api.clone();
        async move {
            let mut n = Names::default();
            if let Ok(l) = api.lists().await {
                n.lists = l.lists.into_iter().map(|x| x.name).collect();
            }
            if let Ok(t) = api.call("tags.all", json!({})).await {
                n.tags = t
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x["tag"].as_str().map(str::to_string))
                    .collect();
            }
            if let Ok(p) = api.picture_lists().await {
                n.picture_lists = p
                    .iter()
                    .filter_map(|x| x["list"].as_str().map(str::to_string))
                    .collect();
            }
            if let Ok(p) = api.call("pictures.list", json!({})).await {
                n.pictures = p
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|x| {
                        let id = x["id"].as_str().unwrap_or("").to_string();
                        let name = x["name"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| id.chars().take(8).collect());
                        (id, name)
                    })
                    .collect();
            }
            if let Ok(ms) = api.spec_models().await {
                n.models = ms
                    .iter()
                    .map(|m| {
                        let heads = m["heads"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|h| {
                                (
                                    h["name"].as_str().unwrap_or("").to_string(),
                                    h["kind"].as_str().unwrap_or("").to_string(),
                                    strings(&h["classes"]),
                                )
                            })
                            .collect();
                        (m["name"].as_str().unwrap_or("").to_string(), heads)
                    })
                    .collect();
            }
            if let Ok(p) = api.call("models.pretrained", json!({})).await {
                n.any_folder = p["any"].as_bool().unwrap_or(false);
                n.folders = strings(&p["folders"]);
            }
            n
        }
    });
    let ch: EventHandler<Change> = use_callback(move |c: Change| {
        let Some(mut v) = spec.peek().clone() else { return };
        match c {
            Change::Set(p, val) => put(&mut v, &p, val),
            Change::Remove(p, i) => remove_at(&mut v, &p, i),
            Change::Push(p, val) => {
                let mut a = at(&v, &p).as_array().cloned().unwrap_or_default();
                a.push(val);
                put(&mut v, &p, Value::Array(a));
            }
        }
        on_change.call(serde_json::to_string_pretty(&v).unwrap_or_default());
    });
    let Some(v) = spec() else {
        return rsx! { div { class: "card sb", p { class: "spec-err small", Icon { name: "alert" } "The JSON doesn't read; fix it in the JSON view, and the builder comes back." } } };
    };
    let n = names.read().clone().unwrap_or_default();
    let heads: Vec<Value> = v["heads"].as_array().cloned().unwrap_or_default();
    let head_names: Vec<String> = heads
        .iter()
        .filter_map(|h| h["name"].as_str().map(str::to_string))
        .collect();
    let generates = heads.iter().any(|h| h["kind"] == "generate");
    let itype = v["input"]["type"].as_str().unwrap_or("text").to_string();
    let e = |p: &str| err_here(&err, p);

    rsx! {
        div { class: "sb",
            if let Some((p, m)) = err.clone() {
                div { class: "spec-err small sb-top-err", role: "alert", Icon { name: "alert" } code { "{p}" } span { "{m}" } }
            }
            Diagram { v: v.clone() }

            // the model
            div { class: "sb-sec",
                h4 { "the model" }
                {row("name", "other models' critics name it by this", e("name"), line(&v, "name".into(), "its name", false, ch))}
                {row("version", "the spec format's version", e("version"), pick_json(&v, "version".into(), vec![("1".into(), "1".into())], Some("1 (the default)"), ch))}
                {row("seed", "the weights' and the data split's seed", e("seed"), num(&v, "seed".into(), 0.0, 4294967295.0, 1.0, "0", ch))}
                {row("runs on", "where training and writing run", e("device"), pick(&v, "device".into(), vec![("gpu".into(), "the GPU, exact".into()), ("gpu_fast".into(), "the GPU, fast".into())], Some("the CPU (the default)"), ch))}
                {row("starts from", "a pretrained model folder (its trunk and vocabulary), or new weights", e("from.pretrained") .or(e("from")),
                    if n.any_folder {
                        line(&v, "from.pretrained".into(), "a model folder on this machine (empty: new weights)", true, ch)
                    } else {
                        pick(&v, "from.pretrained".into(), named(&n.folders), Some("new weights"), ch)
                    })}
            }

            // what it reads
            div { class: "sb-sec",
                h4 { "what it reads" }
                {row("input", "", e("input.type").or(e("input")), rsx! {
                    select { class: "sb-pick", "aria-label": "input.type",
                        onchange: move |ev| {
                            let t = ev.value();
                            let fresh = match t.as_str() {
                                "image" => json!({ "type": "image", "width": 32, "height": 32, "patch": 8, "codes": 128 }),
                                "numbers" => json!({ "type": "numbers", "names": ["a"], "codec": { "digits": 3, "decimals": 1, "signed": false } }),
                                other => json!({ "type": other }),
                            };
                            ch.call(Change::Set("input".into(), fresh));
                        },
                        for (o , w) in [("text", "text"), ("bytes", "bytes"), ("image", "pictures"), ("numbers", "numbers")] {
                            option { key: "{o}", value: "{o}", selected: itype == o, "{w}" }
                        }
                    }
                })}
                if itype == "text" {
                    {row("channels", "gematria and shape features beside the bytes (all off by default)", e("input.features"), rsx! {
                        span { class: "sb-ticks",
                            for (f , w) in [("letters", "letter values"), ("word_sums", "word sums"), ("side", "whole-text measures")] {
                                {
                                    let has = strings(&v["input"]["features"]).contains(&f.to_string());
                                    let all = strings(&v["input"]["features"]);
                                    rsx! {
                                        label { key: "{f}", class: "sb-tick",
                                            input { r#type: "checkbox", checked: has, "aria-label": "input.features {f}",
                                                oninput: move |ev| {
                                                    let mut a = all.clone();
                                                    a.retain(|x| x != f);
                                                    if ev.checked() { a.push(f.to_string()); }
                                                    ch.call(Change::Set("input.features".into(), json!(a)));
                                                } }
                                            "{w}"
                                        }
                                    }
                                }
                            }
                        }
                    })}
                }
                if itype == "image" {
                    {row("size", "pixels; each a multiple of the patch", e("input.width").or(e("input.height")), rsx! {
                        span { class: "sb-pair", {num(&v, "input.width".into(), 8.0, 1024.0, 8.0, "32", ch)} "×" {num(&v, "input.height".into(), 8.0, 1024.0, 8.0, "32", ch)} }
                    })}
                    {row("patch", "a patch's side in pixels: one token each", e("input.patch"), num(&v, "input.patch".into(), 1.0, 64.0, 1.0, "8", ch))}
                    {row("codes", "how many learned patch prototypes (at most 255)", e("input.codes"), slide(&v, "input.codes".into(), (2.0, 255.0, 1.0, 128.0), true, ch))}
                }
                if itype == "numbers" {
                    {row("names", "one per number the model reads", e("input.names"), chips(&v, "input.names".into(), "a name, Enter", ch))}
                    {row("width", "digits before and after the point", e("input.codec.digits").or(e("input.codec.decimals")), rsx! {
                        span { class: "sb-pair", {num(&v, "input.codec.digits".into(), 1.0, 12.0, 1.0, "3", ch)} "." {num(&v, "input.codec.decimals".into(), 0.0, 8.0, 1.0, "1", ch)} }
                    })}
                    {row("signed", "", e("input.codec.signed"), tick(&v, "input.codec.signed".into(), "numbers can be below zero", ch))}
                }
            }

            // the trunk
            div { class: "sb-sec",
                h4 { "the trunk" }
                {row("preset", "the sizes; any set below override it", e("trunk.preset"), pick(&v, "trunk.preset".into(), opts(&["compact", "small", "base", "auto", "test"]), Some("the default"), ch))}
                details { class: "sb-more",
                    summary { "sizes of its own" }
                    {row("width", "d_model", e("trunk.d_model"), num(&v, "trunk.d_model".into(), 8.0, 4096.0, 8.0, "from the preset", ch))}
                    {row("layers", "", e("trunk.layers"), num(&v, "trunk.layers".into(), 1.0, 48.0, 1.0, "from the preset", ch))}
                    {row("attention heads", "", e("trunk.heads"), num(&v, "trunk.heads".into(), 1.0, 64.0, 1.0, "from the preset", ch))}
                    {row("head size", "head_dim", e("trunk.head_dim"), num(&v, "trunk.head_dim".into(), 4.0, 512.0, 4.0, "from the preset", ch))}
                    {row("feed-forward", "d_ff", e("trunk.d_ff"), num(&v, "trunk.d_ff".into(), 8.0, 16384.0, 8.0, "from the preset", ch))}
                    {row("window", "tokens per window, CLS and SEP included", e("trunk.max_len"), num(&v, "trunk.max_len".into(), 8.0, 8192.0, 1.0, "from the preset", ch))}
                    {row("causal", "on by default when there's a generate head", e("trunk.causal"), pick_json(&v, "trunk.causal".into(), vec![("true".into(), "causal".into()), ("false".into(), "not causal".into())], Some("the default"), ch))}
                    {row("pooling", "how the tokens become one vector for the heads", e("trunk.pooling"), pick(&v, "trunk.pooling".into(), vec![("Cls".into(), "CLS token".into()), ("Mean".into(), "mean".into()), ("ClsMean".into(), "CLS and mean".into()), ("Attention".into(), "attention".into())], Some("the default"), ch))}
                    {row("dropout", "", e("trunk.dropout"), slide(&v, "trunk.dropout".into(), (0.0, 0.5, 0.01, 0.1), true, ch))}
                }
            }

            // the heads
            div { class: "sb-sec",
                h4 { "the heads" span { class: "dim small", " what comes out" } }
                for (i , h) in heads.iter().enumerate() {
                    {
                        let kind = h["kind"].as_str().unwrap_or("classify").to_string();
                        rsx! {
                            div { key: "{i}", class: "sb-item",
                                div { class: "sb-item-head",
                                    span { class: "sb-n", "{i + 1}" }
                                    {line(&v, format!("heads[{i}].name"), "its name", false, ch)}
                                    {pick(&v, format!("heads[{i}].kind"), vec![("classify".into(), "one class of several".into()), ("multi".into(), "any of several labels".into()), ("regress".into(), "numbers".into()), ("generate".into(), "writes (the next byte)".into())], None, ch)}
                                    button { class: "plain small", title: "take this head out", onclick: move |_| ch.call(Change::Remove("heads".into(), i)), Icon { name: "delete" } }
                                }
                                if let Some(m) = e(&format!("heads[{i}].name")).or(e(&format!("heads[{i}].kind"))).or(e(&format!("heads[{i}]"))) { span { class: "sb-err", Icon { name: "alert" } "{m}" } }
                                if kind != "generate" {
                                    {row(if kind == "regress" { "values" } else { "classes" }, if kind == "regress" { "a name per number it gives" } else { "" }, e(&format!("heads[{i}].classes")), chips(&v, format!("heads[{i}].classes"), "add one, Enter", ch))}
                                }
                                {row("weight", "its share of the loss when heads train together", e(&format!("heads[{i}].weight")), slide(&v, format!("heads[{i}].weight"), (0.0, 5.0, 0.05, 1.0), true, ch))}
                            }
                        }
                    }
                }
                button { class: "btn small", onclick: move |_| ch.call(Change::Push("heads".into(), json!({ "name": format!("head {}", heads.len() + 1), "kind": "classify", "classes": ["yes", "no"] }))),
                    Icon { name: "new" } "a head" }
            }

            // the data
            DataSection { v: v.clone(), names: n.clone(), head_names: head_names.clone(), itype: itype.clone(), err: err.clone(), ch }

            // a generator's critics, constraints and sampling
            if generates || v["critics"].as_array().is_some_and(|a| !a.is_empty()) {
                CriticSection { v: v.clone(), names: n.clone(), err: err.clone(), ch }
            }
            if generates {
                div { class: "sb-sec",
                    h4 { "what it writes" span { class: "dim small", " a candidate breaking these is dropped" } }
                    {row("length", "bytes, at least and at most (empty: no limit)", e("constraints.min_len").or(e("constraints.max_len")), rsx! {
                        span { class: "sb-pair", {num(&v, "constraints.min_len".into(), 0.0, 100000.0, 1.0, "0", ch)} "to" {num(&v, "constraints.max_len".into(), 0.0, 100000.0, 1.0, "no limit", ch)} }
                    })}
                    {row("must contain", "each of these", e("constraints.contains"), byte_chips(&v, "constraints.contains".into(), "a text, Enter", ch))}
                    {row("must not contain", "any of these", e("constraints.excludes"), byte_chips(&v, "constraints.excludes".into(), "a text, Enter", ch))}
                    for (g , _) in v["constraints"]["sums"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                        div { key: "sum{g}", class: "sb-item",
                            div { class: "sb-item-head",
                                span { class: "small", "a sum:" }
                                {pick(&v, format!("constraints.sums[{g}].measure"), vec![("bytes".into(), "bytes (Σ byte + 1)".into()), ("bytes_reversed".into(), "bytes reversed".into()), ("ordinal".into(), "ordinal".into()), ("reversed".into(), "reversed".into()), ("length".into(), "length".into())], None, ch)}
                                {num(&v, format!("constraints.sums[{g}].lo"), 0.0, 1e12, 1.0, "from", ch)}
                                "to"
                                {num(&v, format!("constraints.sums[{g}].hi"), 0.0, 1e12, 1.0, "to", ch)}
                                "mod"
                                {num(&v, format!("constraints.sums[{g}].modulo"), 0.0, 1e12, 1.0, "none", ch)}
                                button { class: "plain small", onclick: move |_| ch.call(Change::Remove("constraints.sums".into(), g)), Icon { name: "delete" } }
                            }
                            if let Some(m) = e(&format!("constraints.sums[{g}]")) { span { class: "sb-err", Icon { name: "alert" } "{m}" } }
                        }
                    }
                    button { class: "btn small", onclick: move |_| ch.call(Change::Push("constraints.sums".into(), json!({ "measure": "ordinal", "lo": 0, "hi": 1000 }))), Icon { name: "new" } "a sum it must fall in" }
                    GateRows { v: v.clone(), names: n.clone(), err: err.clone(), ch }
                    h4 { class: "sb-sub", "sampling" }
                    {row("temperature", "0 always picks the likeliest byte", e("sampling.temperature"), slide(&v, "sampling.temperature".into(), (0.0, 2.0, 0.05, 0.8), true, ch))}
                    {row("top k", "keep only the k likeliest bytes (0: all)", e("sampling.top_k"), num(&v, "sampling.top_k".into(), 0.0, 256.0, 1.0, "0", ch))}
                    {row("top p", "keep the fewest likeliest bytes reaching p", e("sampling.top_p"), slide(&v, "sampling.top_p".into(), (0.0, 1.0, 0.01, 0.95), true, ch))}
                    {row("at most", "bytes written", e("sampling.max_len"), num(&v, "sampling.max_len".into(), 1.0, 100000.0, 1.0, "200", ch))}
                    {row("stops at", "a character that ends what it writes", e("sampling.stop"), rsx! {
                        input { r#type: "text", class: "sb-text sb-short", maxlength: "1", placeholder: "none", "aria-label": "sampling.stop",
                            value: v["sampling"]["stop"].as_u64().map(|b| (b as u8 as char).to_string()).unwrap_or_default(),
                            onchange: move |ev| {
                                let t = ev.value();
                                ch.call(Change::Set("sampling.stop".into(), t.bytes().next().map_or(Value::Null, |b| json!(b))));
                            } }
                    })}
                    {row("seed", "", e("sampling.seed"), num(&v, "sampling.seed".into(), 0.0, 4294967295.0, 1.0, "0", ch))}
                }
            }

            // training
            div { class: "sb-sec",
                h4 { "training" }
                {row("epochs", "", e("train.epochs"), num(&v, "train.epochs".into(), 1.0, 10000.0, 1.0, "the default", ch))}
                {row("batch", "", e("train.batch"), num(&v, "train.batch".into(), 1.0, 4096.0, 1.0, "the default", ch))}
                {row("learning rate", "", e("train.lr"), num(&v, "train.lr".into(), 0.000001, 1.0, 0.0001, "the default", ch))}
                {row("held out", "the share of the data kept for checking", e("train.val_fraction"), slide(&v, "train.val_fraction".into(), (0.0, 0.5, 0.01, 0.1), true, ch))}
                {row("patience", "epochs without improving before it stops", e("train.patience"), num(&v, "train.patience".into(), 0.0, 1000.0, 1.0, "the default", ch))}
                {row("steps at most", "", e("train.max_steps"), num(&v, "train.max_steps".into(), 1.0, 1e9, 1.0, "no limit", ch))}
                {row("threads", "", e("train.threads"), num(&v, "train.threads".into(), 1.0, 256.0, 1.0, "the default", ch))}
                {row("time budget", "seconds an \"auto\" trunk is sized for", e("train.budget_seconds"), num(&v, "train.budget_seconds".into(), 1.0, 1e6, 1.0, "none", ch))}
            }
        }
    }
}

/// the data rows: where each head's examples come from
#[component]
fn DataSection(
    v: Value,
    names: Names,
    head_names: Vec<String>,
    itype: String,
    err: Option<(String, String)>,
    ch: EventHandler<Change>,
) -> Element {
    let api = api();
    let u = ui();
    let hn_add = head_names.clone();
    let data: Vec<Value> = v["data"].as_array().cloned().unwrap_or_default();
    let e = |p: &str| err_here(&err, p);
    let classes_of = |head: &str| -> (String, Vec<String>) {
        v["heads"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|h| h["name"] == head)
            .map(|h| (h["kind"].as_str().unwrap_or("").to_string(), strings(&h["classes"])))
            .unwrap_or_default()
    };
    // a head and its class or values, as the head's kind needs
    let target = |prefix: String, d: &Value| -> Element {
        let head = d["head"].as_str().unwrap_or("").to_string();
        let (kind, classes) = classes_of(&head);
        rsx! {
            {pick(&v, format!("{prefix}.head"), named(&head_names), Some("which head?"), ch)}
            if kind == "classify" || kind == "multi" {
                {pick(&v, format!("{prefix}.class"), named(&classes), Some("which class?"), ch)}
            }
            if kind == "regress" {
                {values(&v, format!("{prefix}.value"), &classes, ch)}
            }
        }
    };
    rsx! {
        div { class: "sb-sec",
            h4 { "the data" span { class: "dim small", " what each head learns from" } }
            for (i , d) in data.iter().enumerate() {
                {
                    let from = d["from"].as_str().unwrap_or("list").to_string();
                    let head = d["head"].clone();
                    rsx! {
                        div { key: "{i}", class: "sb-item",
                            div { class: "sb-item-head",
                                span { class: "sb-n", "{i + 1}" }
                                select { class: "sb-pick", "aria-label": "data[{i}].from",
                                    onchange: move |ev| {
                                        let f = ev.value();
                                        let mut fresh = json!({ "from": f });
                                        if f != "inline" && !head.is_null() { fresh["head"] = head.clone(); }
                                        if f == "inline" { fresh["items"] = json!([]); }
                                        ch.call(Change::Set(format!("data[{i}]"), fresh));
                                    },
                                    for (o , w) in [("list", "a list's members"), ("tagged", "texts with a tag"), ("pairs", "pairs of two lists"), ("pictures", "pictures"), ("inline", "examples written here")] {
                                        option { key: "{o}", value: "{o}", selected: from == o, "{w}" }
                                    }
                                }
                                button { class: "plain small", title: "take this out", onclick: move |_| ch.call(Change::Remove("data".into(), i)), Icon { name: "delete" } }
                            }
                            if let Some(m) = e(&format!("data[{i}]")).or(e(&format!("data[{i}].from"))) { span { class: "sb-err", Icon { name: "alert" } "{m}" } }
                            match from.as_str() {
                                "list" => rsx! {
                                    {row("list", "", e(&format!("data[{i}].list")), pick(&v, format!("data[{i}].list"), named(&names.lists), Some("which list?"), ch))}
                                    {row("for", "the head, and what its members are", e(&format!("data[{i}].head")).or(e(&format!("data[{i}].class"))).or(e(&format!("data[{i}].value"))), target(format!("data[{i}]"), d))}
                                },
                                "tagged" => rsx! {
                                    {row("tag", "", e(&format!("data[{i}].tag")), pick(&v, format!("data[{i}].tag"), named(&names.tags), Some("which tag?"), ch))}
                                    {row("for", "", e(&format!("data[{i}].head")).or(e(&format!("data[{i}].class"))).or(e(&format!("data[{i}].value"))), target(format!("data[{i}]"), d))}
                                },
                                "pairs" => rsx! {
                                    {row("in", "what it reads", e(&format!("data[{i}].input")), pick(&v, format!("data[{i}].input"), named(&names.lists), Some("which list?"), ch))}
                                    {row("out", "what it learns to write after it, member by member", e(&format!("data[{i}].output")), pick(&v, format!("data[{i}].output"), named(&names.lists), Some("which list?"), ch))}
                                    {row("head", "a generate head", e(&format!("data[{i}].head")), pick(&v, format!("data[{i}].head"), named(&head_names), Some("which head?"), ch))}
                                },
                                "pictures" => rsx! {
                                    {row("pictures", "each scaled to the input's size", e(&format!("data[{i}].list")), pick(&v, format!("data[{i}].list"), named(&names.picture_lists), Some("all of the save's pictures"), ch))}
                                    {row("for", "", e(&format!("data[{i}].head")).or(e(&format!("data[{i}].class"))).or(e(&format!("data[{i}].value"))), target(format!("data[{i}]"), d))}
                                },
                                _ => rsx! {
                                    for (j , item) in d["items"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                                        div { key: "{j}", class: "sb-inline",
                                            span { class: "dim small", "{j + 1}" }
                                            match itype.as_str() {
                                                "numbers" => rsx! {
                                                    {values(&v, format!("data[{i}].items[{j}].input.numbers"), &strings(&v["input"]["names"]), ch)}
                                                },
                                                "image" => {
                                                    let w = item["input"]["image"]["width"].as_u64().unwrap_or(0);
                                                    let h = item["input"]["image"]["height"].as_u64().unwrap_or(0);
                                                    let api = api.clone();
                                                    let (iw, ih) = (v["input"]["width"].as_u64().unwrap_or(32), v["input"]["height"].as_u64().unwrap_or(32));
                                                    rsx! {
                                                        span { class: "small", if w > 0 { "a picture, {w} × {h}" } else { "no picture yet" } }
                                                        select { class: "sb-pick", "aria-label": "data[{i}].items[{j}].input.image",
                                                            onchange: move |ev| {
                                                                let id = ev.value();
                                                                if id.is_empty() { return; }
                                                                let api = api.clone();
                                                                spawn(async move {
                                                                    // the picture as the item's input: IMAGE_FIELDS
                                                                    match api.call("pictures.get", json!({ "id": id, "rgb": { "width": iw, "height": ih } })).await {
                                                                        Ok(p) => ch.call(Change::Set(format!("data[{i}].items[{j}].input"), json!({ "image": p["image"] }))),
                                                                        Err(err) => u.fail(err),
                                                                    }
                                                                });
                                                            },
                                                            option { value: "", "from a picture…" }
                                                            for (id , nm) in names.pictures.iter() { option { key: "{id}", value: "{id}", "{nm}" } }
                                                        }
                                                    }
                                                }
                                                "bytes" => rsx! {
                                                    input { r#type: "text", class: "sb-text", placeholder: "its bytes, as text", "aria-label": "data[{i}].items[{j}].input.bytes",
                                                        value: String::from_utf8_lossy(&item["input"]["bytes"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64().map(|b| b as u8)).collect::<Vec<u8>>()).unwrap_or_default()).into_owned(),
                                                        onchange: move |ev| ch.call(Change::Set(format!("data[{i}].items[{j}].input"), json!({ "bytes": ev.value().as_bytes() }))) }
                                                },
                                                _ => rsx! {
                                                    input { r#type: "text", class: "sb-text", placeholder: "its text", "aria-label": "data[{i}].items[{j}].input.text",
                                                        value: item["input"]["text"].as_str().unwrap_or("").to_string(),
                                                        onchange: move |ev| ch.call(Change::Set(format!("data[{i}].items[{j}].input"), json!({ "text": ev.value() }))) }
                                                },
                                            }
                                            {target(format!("data[{i}].items[{j}]"), item)}
                                            if let Some(m) = e(&format!("data[{i}].items[{j}].head")).or(e(&format!("data[{i}].items[{j}].class"))).or(e(&format!("data[{i}].items[{j}].value"))).or(e(&format!("data[{i}].items[{j}].input"))) {
                                                span { class: "sb-err", Icon { name: "alert" } "{m}" }
                                            }
                                            button { class: "plain small", onclick: move |_| ch.call(Change::Remove(format!("data[{i}].items"), j)), "×" }
                                        }
                                    }
                                    button { class: "plain small", onclick: {
                                            let (itype, head_names) = (itype.clone(), head_names.clone());
                                            move |_| {
                                            let input = match itype.as_str() {
                                                "numbers" => json!({ "numbers": [] }),
                                                "bytes" => json!({ "bytes": [] }),
                                                "image" => json!({ "image": { "width": 0, "height": 0, "rgb": [] } }),
                                                _ => json!({ "text": "" }),
                                            };
                                            ch.call(Change::Push(format!("data[{i}].items"), json!({ "input": input, "head": head_names.first().cloned().unwrap_or_default() })));
                                        } },
                                        Icon { name: "new" } "an example" }
                                },
                            }
                        }
                    }
                }
            }
            button { class: "btn small", onclick: move |_| ch.call(Change::Push("data".into(), json!({ "from": "list", "head": hn_add.first().cloned().unwrap_or_default() }))), Icon { name: "new" } "data" }
        }
    }
}

/// a generator's critics: another model, its head, and what it should say
#[component]
fn CriticSection(v: Value, names: Names, err: Option<(String, String)>, ch: EventHandler<Change>) -> Element {
    let critics: Vec<Value> = v["critics"].as_array().cloned().unwrap_or_default();
    let e = |p: &str| err_here(&err, p);
    let me = v["name"].as_str().unwrap_or("").to_string();
    let models: Vec<String> = names.models.iter().map(|m| m.0.clone()).filter(|m| *m != me).collect();
    rsx! {
        div { class: "sb-sec",
            h4 { "critics" span { class: "dim small", " other models judging what it writes" } }
            for (k , c) in critics.iter().enumerate() {
                {
                    let model = c["model"].as_str().unwrap_or("").to_string();
                    let heads = names.models.iter().find(|m| m.0 == model).map(|m| m.1.clone()).unwrap_or_default();
                    let head = c["head"].as_str().unwrap_or("").to_string();
                    let (hkind, classes) = heads.iter().find(|h| h.0 == head).map(|h| (h.1.clone(), h.2.clone())).unwrap_or_default();
                    let want_kind = c["want"].as_object().and_then(|o| o.keys().next().cloned()).unwrap_or_else(|| "class".into());
                    let wk = want_kind.clone();
                    rsx! {
                        div { key: "{k}", class: "sb-item",
                            div { class: "sb-item-head",
                                span { class: "sb-n", "{k + 1}" }
                                {pick(&v, format!("critics[{k}].model"), named(&models), Some("which model?"), ch)}
                                {pick(&v, format!("critics[{k}].head"), heads.iter().map(|h| (h.0.clone(), format!("{} ({})", h.0, h.1))).collect(), Some("which head?"), ch)}
                                button { class: "plain small", onclick: move |_| ch.call(Change::Remove("critics".into(), k)), Icon { name: "delete" } }
                            }
                            if let Some(m) = e(&format!("critics[{k}]")).or(e(&format!("critics[{k}].model"))).or(e(&format!("critics[{k}].head"))) { span { class: "sb-err", Icon { name: "alert" } "{m}" } }
                            {row("wants", "", e(&format!("critics[{k}].want")), rsx! {
                                select { class: "sb-pick", "aria-label": "critics[{k}].want",
                                    onchange: move |ev| {
                                        let w = match ev.value().as_str() {
                                            "not" => json!({ "not": "" }),
                                            "near" => json!({ "near": { "value": "", "target": 0.0, "scale": 1.0 } }),
                                            "range" => json!({ "range": { "value": "", "lo": 0.0, "hi": 1.0, "scale": 1.0 } }),
                                            _ => json!({ "class": "" }),
                                        };
                                        ch.call(Change::Set(format!("critics[{k}].want"), w));
                                    },
                                    for (o , w) in [("class", "a class"), ("not", "not a class"), ("near", "a value near"), ("range", "a value between")] {
                                        option { key: "{o}", value: "{o}", selected: wk == o, "{w}" }
                                    }
                                }
                                match want_kind.as_str() {
                                    "not" => pick(&v, format!("critics[{k}].want.not"), named(&classes), Some("which class?"), ch),
                                    "near" => rsx! {
                                        {pick(&v, format!("critics[{k}].want.near.value"), named(&classes), Some("which value?"), ch)}
                                        "near" {num(&v, format!("critics[{k}].want.near.target"), -1e9, 1e9, 0.01, "target", ch)}
                                        "within" {num(&v, format!("critics[{k}].want.near.scale"), 0.0001, 1e9, 0.01, "scale", ch)}
                                    },
                                    "range" => rsx! {
                                        {pick(&v, format!("critics[{k}].want.range.value"), named(&classes), Some("which value?"), ch)}
                                        {num(&v, format!("critics[{k}].want.range.lo"), -1e9, 1e9, 0.01, "from", ch)}
                                        "to" {num(&v, format!("critics[{k}].want.range.hi"), -1e9, 1e9, 0.01, "to", ch)}
                                        "softness" {num(&v, format!("critics[{k}].want.range.scale"), 0.0001, 1e9, 0.01, "scale", ch)}
                                    },
                                    _ => pick(&v, format!("critics[{k}].want.class"), named(&classes), Some("which class?"), ch),
                                }
                                if !hkind.is_empty() { span { class: "dim small", "({hkind})" } }
                            })}
                            {row("weight", "its share of the score", e(&format!("critics[{k}].weight")), slide(&v, format!("critics[{k}].weight"), (0.0, 5.0, 0.05, 1.0), true, ch))}
                        }
                    }
                }
            }
            button { class: "btn small", disabled: models.is_empty(), title: if models.is_empty() { "make another model first: a critic is another model" } else { "another model judging what this one writes" },
                onclick: move |_| ch.call(Change::Push("critics".into(), json!({ "model": "", "head": "", "want": { "class": "" } }))),
                Icon { name: "new" } "a critic" }
        }
    }
}

/// gates: a critic's class that must (or must not) be among what it says
#[component]
fn GateRows(v: Value, names: Names, err: Option<(String, String)>, ch: EventHandler<Change>) -> Element {
    let critics: Vec<Value> = v["critics"].as_array().cloned().unwrap_or_default();
    let gates: Vec<Value> = v["constraints"]["gates"].as_array().cloned().unwrap_or_default();
    let e = |p: &str| err_here(&err, p);
    let classes_of = |ci: usize| -> Vec<String> {
        let Some(c) = critics.get(ci) else { return vec![] };
        names
            .models
            .iter()
            .find(|m| m.0 == c["model"].as_str().unwrap_or(""))
            .and_then(|m| m.1.iter().find(|h| h.0 == c["head"].as_str().unwrap_or("")))
            .map(|h| h.2.clone())
            .unwrap_or_default()
    };
    rsx! {
        for (g , gate) in gates.iter().enumerate() {
            {
                let ci = gate["critic"].as_u64().unwrap_or(0) as usize;
                let classes = classes_of(ci);
                rsx! {
                    div { key: "gate{g}", class: "sb-item",
                        div { class: "sb-item-head",
                            span { class: "small", "critic" }
                            {pick_json(&v, format!("constraints.gates[{g}].critic"), critics.iter().enumerate().map(|(i, c)| (i.to_string(), format!("{} {}", i + 1, c["model"].as_str().unwrap_or("")))).collect(), None, ch)}
                            {pick_json(&v, format!("constraints.gates[{g}].member"), vec![("true".into(), "must say".into()), ("false".into(), "must not say".into())], None, ch)}
                            {pick_json(&v, format!("constraints.gates[{g}].class"), classes.iter().enumerate().map(|(i, c)| (i.to_string(), c.clone())).collect(), None, ch)}
                            button { class: "plain small", onclick: move |_| ch.call(Change::Remove("constraints.gates".into(), g)), Icon { name: "delete" } }
                        }
                        if let Some(m) = e(&format!("constraints.gates[{g}]")) { span { class: "sb-err", Icon { name: "alert" } "{m}" } }
                    }
                }
            }
        }
        button { class: "btn small", disabled: critics.is_empty(), title: "a critic's class that must, or must not, be among what it says",
            onclick: move |_| ch.call(Change::Push("constraints.gates".into(), json!({ "critic": 0, "class": 0, "member": true }))),
            Icon { name: "new" } "a gate" }
    }
}

/// how the parts connect: the data into the heads, the input through the trunk to the
/// heads, the critics judging a generate head
#[component]
fn Diagram(v: Value) -> Element {
    let heads: Vec<(String, String)> = v["heads"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| {
            (
                h["name"].as_str().unwrap_or("").to_string(),
                h["kind"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let data: Vec<(String, String)> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let what = match d["from"].as_str().unwrap_or("") {
                "list" => format!("list {}", d["list"].as_str().unwrap_or("?")),
                "tagged" => format!("tag {}", d["tag"].as_str().unwrap_or("?")),
                "pairs" => format!(
                    "{} → {}",
                    d["input"].as_str().unwrap_or("?"),
                    d["output"].as_str().unwrap_or("?")
                ),
                "pictures" => d["list"]
                    .as_str()
                    .map_or("all pictures".to_string(), |l| format!("pictures {l}")),
                _ => format!("{} examples", d["items"].as_array().map_or(0, |a| a.len())),
            };
            (what, d["head"].as_str().unwrap_or("").to_string())
        })
        .collect();
    let critics: Vec<String> = v["critics"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| c["model"].as_str().unwrap_or("?").to_string())
        .collect();
    let input = match v["input"]["type"].as_str().unwrap_or("text") {
        "image" => format!("pictures {}×{}", v["input"]["width"], v["input"]["height"]),
        "numbers" => format!("{} numbers", v["input"]["names"].as_array().map_or(0, |a| a.len())),
        t => t.to_string(),
    };
    let trunk = format!(
        "trunk · {}{}",
        v["trunk"]["preset"].as_str().unwrap_or("default"),
        v["from"]["pretrained"].as_str().map(|_| " · pretrained").unwrap_or("")
    );
    let rows = heads.len().max(data.len()).max(critics.len()).max(1);
    let h = 26.0 * rows as f64 + 20.0;
    let y = |i: usize, n: usize| -> f64 { 10.0 + (h - 20.0) * (i as f64 + 0.5) / n.max(1) as f64 };
    let short = |s: &str| -> String {
        if s.chars().count() > 18 {
            format!("{}…", s.chars().take(17).collect::<String>())
        } else {
            s.to_string()
        }
    };
    let head_y = |name: &str| heads.iter().position(|h| h.0 == name).map(|i| y(i, heads.len()));
    let gen_y = heads.iter().position(|h| h.1 == "generate").map(|i| y(i, heads.len()));
    rsx! {
        svg { class: "sb-diagram", view_box: "0 0 640 {h}", width: "100%", height: "{h}", role: "img", "aria-label": "how the spec's parts connect",
            // the data feeding the heads they teach
            for (i , (what , head)) in data.iter().enumerate() {
                g { key: "d{i}",
                    rect { x: "4", y: "{y(i, data.len()) - 11.0}", width: "120", height: "22", rx: "5", class: "sb-d-data" }
                    text { x: "64", y: "{y(i, data.len()) + 4.0}", "text-anchor": "middle", {short(what)} }
                    if let Some(hy) = head_y(head) {
                        path { d: "M124 {y(i, data.len())} C 300 {y(i, data.len()) - 60.0}, 380 {hy + 40.0}, 460 {hy}", class: "sb-d-teach" }
                    }
                }
            }
            // the input and the trunk in the middle
            rect { x: "150", y: "{h / 2.0 - 13.0}", width: "110", height: "26", rx: "5", class: "sb-d-in" }
            text { x: "205", y: "{h / 2.0 + 4.0}", "text-anchor": "middle", {short(&input)} }
            line { x1: "260", y1: "{h / 2.0}", x2: "290", y2: "{h / 2.0}", class: "sb-d-line" }
            rect { x: "290", y: "{h / 2.0 - 13.0}", width: "130", height: "26", rx: "5", class: "sb-d-trunk" }
            text { x: "355", y: "{h / 2.0 + 4.0}", "text-anchor": "middle", {short(&trunk)} }
            for (i , (name , kind)) in heads.iter().enumerate() {
                g { key: "h{i}",
                    line { x1: "420", y1: "{h / 2.0}", x2: "460", y2: "{y(i, heads.len())}", class: "sb-d-line" }
                    rect { x: "460", y: "{y(i, heads.len()) - 11.0}", width: "100", height: "22", rx: "5", class: "sb-d-head" }
                    text { x: "510", y: "{y(i, heads.len()) + 4.0}", "text-anchor": "middle", {short(&format!("{name} · {kind}"))} }
                }
            }
            // the critics judging the generate head
            if let Some(gy) = gen_y {
                for (i , c) in critics.iter().enumerate() {
                    g { key: "c{i}",
                        rect { x: "580", y: "{y(i, critics.len()) - 11.0}", width: "56", height: "22", rx: "5", class: "sb-d-critic" }
                        text { x: "608", y: "{y(i, critics.len()) + 4.0}", "text-anchor": "middle", {short(c).chars().take(8).collect::<String>()} }
                        line { x1: "580", y1: "{y(i, critics.len())}", x2: "560", y2: "{gy}", class: "sb-d-judge" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// every path in a value, arrays as `[]` (scalars and arrays of scalars are leaves)
    fn walk(v: &Value, at: &str, out: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                for (k, x) in o {
                    let p = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
                    walk(x, &p, out);
                }
            }
            // an empty list says nothing about what its items hold
            Value::Array(a) if a.is_empty() => {}
            Value::Array(a) if a.iter().any(Value::is_object) => {
                for x in a {
                    walk(x, &format!("{at}[]"), out);
                }
            }
            _ => out.push(at.to_string()),
        }
    }

    /// specs using every part of the format, read by the core (so every field is real)
    fn every_part() -> Vec<Value> {
        use anend_core::classify::spec::ModelSpec;
        let full = r#"{
            "version": 1, "name": "all", "seed": 3, "device": "gpu_fast",
            "from": {"pretrained": "folder"},
            "input": {"type": "text", "features": ["letters", "word_sums", "side"]},
            "trunk": {"preset": "small", "d_model": 64, "layers": 2, "heads": 4, "head_dim": 16, "d_ff": 128,
                      "max_len": 64, "causal": true, "pooling": "Mean", "dropout": 0.1},
            "heads": [{"name": "c", "kind": "classify", "classes": ["a", "b"], "weight": 1.0},
                      {"name": "r", "kind": "regress", "classes": ["x"], "weight": 0.5},
                      {"name": "g", "kind": "generate"}],
            "data": [
                {"from": "list", "list": "l", "head": "c", "class": "a", "value": [1.0]},
                {"from": "tagged", "tag": "t", "head": "r", "value": [1.0]},
                {"from": "pairs", "input": "i", "output": "o", "head": "g"},
                {"from": "pictures", "list": "p", "head": "c", "class": "b"},
                {"from": "inline", "items": [
                    {"input": {"text": "x"}, "head": "c", "class": "a", "value": [1.0]},
                    {"input": {"bytes": [1, 2]}, "head": "c", "class": "b"},
                    {"input": {"numbers": [1.0]}, "head": "r", "value": [2.0]},
                    {"input": {"image": {"width": 1, "height": 1, "rgb": [0, 0, 0]}}, "head": "c", "class": "a"}
                ]}
            ],
            "critics": [
                {"model": "m", "head": "h", "want": {"class": "a"}, "weight": 1.0},
                {"model": "m", "head": "h", "want": {"not": "a"}},
                {"model": "m", "head": "h", "want": {"near": {"value": "v", "target": 1.0, "scale": 1.0}}},
                {"model": "m", "head": "h", "want": {"range": {"value": "v", "lo": 0.0, "hi": 1.0, "scale": 1.0}}}
            ],
            "constraints": {"min_len": 1, "max_len": 9, "gates": [{"critic": 0, "class": 0, "member": true}],
                            "sums": [{"measure": "ordinal", "lo": 0, "hi": 9, "modulo": 0}],
                            "contains": [[97]], "excludes": [[98]]},
            "sampling": {"temperature": 0.5, "top_k": 2, "top_p": 0.9, "max_len": 10, "stop": 10, "seed": 1},
            "train": {"epochs": 1, "batch": 2, "lr": 0.001, "val_fraction": 0.1, "patience": 2, "max_steps": 9,
                      "threads": 1, "budget_seconds": 5.0}
        }"#;
        let image = r#"{"name": "i", "input": {"type": "image", "width": 16, "height": 16, "patch": 8, "codes": 16},
                        "heads": [{"name": "c", "kind": "classify", "classes": ["a", "b"]}]}"#;
        let numbers = r#"{"name": "n", "input": {"type": "numbers", "names": ["a"], "codec": {"digits": 2, "decimals": 1, "signed": true}},
                          "heads": [{"name": "y", "kind": "regress", "classes": ["s"]}]}"#;
        let bytes = r#"{"name": "b", "input": {"type": "bytes"}, "heads": [{"name": "g", "kind": "generate"}]}"#;
        [full, image, numbers, bytes]
            .iter()
            .map(|s| {
                let spec: ModelSpec = serde_json::from_str(s).unwrap_or_else(|e| panic!("{e}: {s}"));
                serde_json::to_value(spec).unwrap()
            })
            .collect()
    }

    #[test]
    fn every_field_of_the_spec_has_a_control() {
        let mut found = Vec::new();
        for v in every_part() {
            walk(&v, "", &mut found);
        }
        found.sort();
        found.dedup();
        // `from: {pretrained}` and the value enums are objects of one key; the walk finds
        // them as their inner paths
        let missing: Vec<&String> = found.iter().filter(|p| !COVERED.contains(&p.as_str())).collect();
        assert!(missing.is_empty(), "fields without a control: {missing:?}");
        // and each path in COVERED is one the builder writes (indices as `[{i}]` there)
        let src = include_str!("builder.rs");
        let body = &src[src
            .find("// ------------------------------------------------------------------ paths")
            .expect("the code")..src.find("#[cfg(test)]").expect("the tests")];
        let norm = ["{i}", "{j}", "{k}", "{g}"]
            .iter()
            .fold(body.to_string(), |s, x| s.replace(&format!("[{x}]"), "[]"));
        for p in COVERED {
            assert!(norm.contains(&format!("\"{p}")), "no control writes {p}");
        }
    }

    #[test]
    fn paths_set_and_read() {
        let mut v = json!({});
        put(&mut v, "heads[1].classes", json!(["a"]));
        put(&mut v, "trunk.d_model", json!(64));
        assert_eq!(at(&v, "heads[1].classes"), &json!(["a"]));
        assert_eq!(v["heads"][0], Value::Null);
        put(&mut v, "trunk.d_model", Value::Null);
        assert_eq!(v["trunk"], json!({}));
        remove_at(&mut v, "heads", 0);
        assert_eq!(v["heads"], json!([{ "classes": ["a"] }]));
    }
}
