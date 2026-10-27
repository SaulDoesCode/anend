//! Pipes in the filters view (docs/architecture.md 2, 3; `pipes.*` in docs/app-api.md):
//! the builder's filter, with the effects it has on the texts it lets through and the
//! actions it can do to them, kept by name. Kept pipes are turned on and off for the
//! memory view and the space (stacked in the order they were turned on), edited in the
//! builder, made to act, and deleted.

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::shell::set_pref;
use crate::ui::*;

/// the effects the editor offers
const EFFECTS: [(&str, &str); 7] = [
    ("colour", "a colour"),
    ("colour_by", "a classifier's colours"),
    ("underline", "an underline"),
    ("label", "a label"),
    ("size", "a size"),
    ("glow", "a glow"),
    ("gather", "gather in the space"),
];

const ACTIONS: [(&str, &str); 3] = [
    ("enlist", "put them in a list"),
    ("tag", "tag them"),
    ("run", "run a program on each"),
];

fn blank(kind: &str) -> Value {
    match kind {
        "colour" => json!({ "kind": "colour", "colour": "#e07a3f" }),
        "colour_by" => json!({ "kind": "colour_by", "classifier": "" }),
        "underline" => json!({ "kind": "underline", "colour": "#3f7ae0" }),
        "label" => json!({ "kind": "label", "text": "" }),
        "size" => json!({ "kind": "size", "scale": 1.5 }),
        "glow" => json!({ "kind": "glow", "colour": "#ffd166" }),
        "gather" => json!({ "kind": "gather" }),
        "enlist" => json!({ "kind": "enlist", "list": "" }),
        "tag" => json!({ "kind": "tag", "tag": "" }),
        _ => json!({ "kind": "run", "program": "" }),
    }
}

/// a pipe's effects or actions in one line each, for the kept list
pub fn describe(v: &Value) -> String {
    let mut out = Vec::new();
    for e in v["effects"].as_array().into_iter().flatten() {
        out.push(match e["kind"].as_str().unwrap_or("") {
            "colour" => format!("colour {}", e["colour"].as_str().unwrap_or("")),
            "colour_by" => format!("{}'s colours", e["classifier"].as_str().unwrap_or("")),
            "underline" => format!("underline {}", e["colour"].as_str().unwrap_or("")),
            "label" => format!("label “{}”", e["text"].as_str().unwrap_or("")),
            "size" => format!("size ×{}", e["scale"]),
            "glow" => format!("glow {}", e["colour"].as_str().unwrap_or("")),
            "gather" => "gathers".to_string(),
            k => k.to_string(),
        });
    }
    for a in v["actions"].as_array().into_iter().flatten() {
        out.push(match a["kind"].as_str().unwrap_or("") {
            "enlist" => format!("→ list “{}”", a["list"].as_str().unwrap_or("")),
            "tag" => format!("→ tag {}", a["tag"].as_str().unwrap_or("")),
            "run" => format!("→ runs {}", a["program"].as_str().unwrap_or("")),
            k => k.to_string(),
        });
    }
    if out.is_empty() {
        "no effects yet".into()
    } else {
        out.join(" · ")
    }
}

/// the effects and actions being edited, under the builder's filter
#[component]
pub fn Parts(effects: Signal<Vec<Value>>, actions: Signal<Vec<Value>>, classifiers: Vec<String>) -> Element {
    let api = api();
    let programs = use_resource(move || {
        let api = api.clone();
        async move {
            api.call("s.programs", json!({}))
                .await
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|p| p["title"].as_str().map(String::from))
                .collect::<Vec<_>>()
        }
    });
    let programs = programs.read().clone().unwrap_or_default();
    let mut effects = effects;
    let mut actions = actions;
    rsx! {
        div { class: "pipe-parts",
            div { class: "row",
                b { "effects" }
                span { class: "dim small", "what the texts it lets through look like, where the pipe is on" }
                span { class: "grow" }
                select { key: "{effects().len()}", "aria-label": "add an effect", value: "",
                    onchange: move |e| { let k = e.value(); if !k.is_empty() { effects.write().push(blank(&k)); } },
                    option { value: "", selected: true, "add an effect…" }
                    for (k , l) in EFFECTS { option { key: "{k}", value: "{k}", "{l}" } }
                }
            }
            for (i , e) in effects().into_iter().enumerate() {
                div { key: "{i}-{e[\"kind\"]}", class: "cond row",
                    match e["kind"].as_str().unwrap_or("") {
                        k @ ("colour" | "underline" | "glow") => rsx! {
                            span { {match k { "colour" => "colour", "underline" => "underline", _ => "glow" }} }
                            input { r#type: "color", "aria-label": "its colour", value: "{e[\"colour\"].as_str().unwrap_or(\"#000000\")}",
                                oninput: move |ev| { if let Some(x) = effects.write().get_mut(i) { x["colour"] = json!(ev.value()); } } }
                        },
                        "colour_by" => rsx! {
                            span { "each in its class's colour, by" }
                            select { "aria-label": "the classifier",
                                onchange: move |ev| { if let Some(x) = effects.write().get_mut(i) { x["classifier"] = json!(ev.value()); } },
                                option { value: "", "a classifier…" }
                                for c in classifiers.iter() { option { key: "{c}", value: "{c}", selected: e["classifier"] == c.as_str(), "{c}" } }
                            }
                        },
                        "label" => rsx! {
                            span { "label" }
                            input { r#type: "text", maxlength: "40", placeholder: "a word or two", initial_value: "{e[\"text\"].as_str().unwrap_or(\"\")}",
                                oninput: move |ev| { if let Some(x) = effects.write().get_mut(i) { x["text"] = json!(ev.value()); } } }
                        },
                        "size" => rsx! {
                            span { "size ×" }
                            input { r#type: "number", min: "0.25", max: "4", step: "0.25", style: "width: 70px;", initial_value: "{e[\"scale\"]}",
                                oninput: move |ev| { if let (Some(x), Ok(v)) = (effects.write().get_mut(i), ev.value().parse::<f64>()) { x["scale"] = json!(v); } } }
                        },
                        "gather" => rsx! { span { "in the space, they pull together" } },
                        k => rsx! { span { class: "dim", "{k}" } },
                    }
                    span { class: "grow" }
                    button { class: "plain ibtn", "aria-label": "remove it", onclick: move |_| { let mut w = effects.write(); if i < w.len() { w.remove(i); } }, Icon { name: "close" } }
                }
            }
            div { class: "row",
                b { "actions" }
                span { class: "dim small", "done once, when you ask, to every text it lets through" }
                span { class: "grow" }
                select { key: "{actions().len()}", "aria-label": "add an action", value: "",
                    onchange: move |e| { let k = e.value(); if !k.is_empty() { actions.write().push(blank(&k)); } },
                    option { value: "", selected: true, "add an action…" }
                    for (k , l) in ACTIONS { option { key: "{k}", value: "{k}", "{l}" } }
                }
            }
            for (i , a) in actions().into_iter().enumerate() {
                div { key: "{i}-{a[\"kind\"]}", class: "cond row",
                    match a["kind"].as_str().unwrap_or("") {
                        "enlist" => rsx! {
                            Icon { name: "list" } span { "into the list" }
                            input { r#type: "text", placeholder: "its name", initial_value: "{a[\"list\"].as_str().unwrap_or(\"\")}",
                                oninput: move |ev| { if let Some(x) = actions.write().get_mut(i) { x["list"] = json!(ev.value()); } } }
                        },
                        "tag" => rsx! {
                            Icon { name: "tag" } span { "tagged" }
                            input { r#type: "text", placeholder: "a tag", initial_value: "{a[\"tag\"].as_str().unwrap_or(\"\")}",
                                oninput: move |ev| { if let Some(x) = actions.write().get_mut(i) { x["tag"] = json!(ev.value()); } } }
                        },
                        _ => rsx! {
                            Icon { name: "play" } span { "run the program" }
                            select { "aria-label": "the program",
                                onchange: move |ev| { if let Some(x) = actions.write().get_mut(i) { x["program"] = json!(ev.value()); } },
                                option { value: "", "a program…" }
                                for p in programs.iter() { option { key: "{p}", value: "{p}", selected: a["program"] == p.as_str(), "{p}" } }
                            }
                            span { class: "dim small", "on each, the text as its first argument" }
                        },
                    }
                    span { class: "grow" }
                    button { class: "plain ibtn", "aria-label": "remove it", onclick: move |_| { let mut w = actions.write(); if i < w.len() { w.remove(i); } }, Icon { name: "close" } }
                }
            }
        }
    }
}

/// the kept pipes: on in the views or not, into the builder, acting, deleting
#[component]
pub fn KeptPipes(rev: Signal<u64>, on_open: EventHandler<Value>) -> Element {
    let ui = ui();
    let api = api();
    let api_l = api.clone();
    let list = use_resource(move || {
        let api = api_l.clone();
        let _ = ((ui.changed)(), rev());
        async move { api.call("pipes.list", json!({})).await }
    });
    let pipes: Vec<Value> = match &*list.read() {
        Some(Ok(v)) => v.as_array().cloned().unwrap_or_default(),
        _ => vec![],
    };
    if pipes.is_empty() {
        return rsx! {};
    }
    let on = ui.prefs.read().pipes.clone();
    rsx! {
        div { class: "card",
            h3 { Icon { name: "filter" } "pipes" span { class: "dim small", " on: their effects show in the memory view and the space, stacked in the order turned on" } }
            for p in pipes {
                {
                    let pipe = p["pipe"].clone();
                    let name = pipe["name"].as_str().unwrap_or("").to_string();
                    let lit = on.iter().position(|n| *n == name);
                    rsx! {
                        PipeRow { key: "{name}", name: name.clone(), pipe: pipe.clone(), lit, on_open }
                    }
                }
            }
        }
    }
}

#[component]
fn PipeRow(name: String, pipe: Value, lit: Option<usize>, on_open: EventHandler<Value>) -> Element {
    let ui = ui();
    let api = api();
    let mut armed = use_signal(|| false);
    let has_actions = pipe["actions"].as_array().is_some_and(|a| !a.is_empty());
    let (n1, n2, n3) = (name.clone(), name.clone(), name.clone());
    let (api1, api2) = (api.clone(), api.clone());
    rsx! {
        div { class: "row kept",
            label { class: "row small", "data-tip": "show its effects in the memory view and the space",
                input { r#type: "checkbox", checked: lit.is_some(), "aria-label": "{name} on in the views",
                    onchange: move |e| {
                        let (n, on) = (n1.clone(), e.checked());
                        set_pref(ui, |p| { p.pipes.retain(|x| *x != n); if on { p.pipes.push(n); } });
                    } }
                if let Some(i) = lit { span { class: "pipe-n", "{i + 1}" } }
            }
            b { "{name}" }
            span { class: "dim small grow", {describe(&pipe)} }
            button { class: "plain", "data-tip": "into the builder", onclick: move |_| on_open.call(pipe.clone()), Icon { name: "edit" } }
            if has_actions {
                button { class: "plain", "data-tip": "do its actions now, on every text it lets through",
                    onclick: move |_| {
                        let (api, n) = (api1.clone(), n2.clone());
                        spawn(async move {
                            match api.call("pipes.apply", json!({ "name": n })).await {
                                Ok(v) => {
                                    let parts: Vec<String> = v["done"].as_array().into_iter().flatten().map(|d| match d["kind"].as_str() {
                                        Some("enlist") => format!("{} into “{}”", d["texts"], d["list"].as_str().unwrap_or("")),
                                        Some("tag") => format!("{} tagged {}", d["texts"], d["tag"].as_str().unwrap_or("")),
                                        _ => format!("{} ran {}{}", d["program"].as_str().unwrap_or(""), d["texts"],
                                            if d["failed"].as_u64().unwrap_or(0) > 0 { format!(" ({} failed)", d["failed"]) } else { String::new() }),
                                    }).collect();
                                    ui.say(format!("{}: {}", n, parts.join(", ")));
                                    ui.touched();
                                }
                                Err(e) => ui.fail(e),
                            }
                        });
                    },
                    Icon { name: "play" } "act" }
            }
            button { class: if armed() { "armed" } else { "plain" }, "data-tip": "delete it (click twice)",
                onclick: move |_| {
                    if !armed() { armed.set(true); return; }
                    let (api, n) = (api2.clone(), n3.clone());
                    spawn(async move {
                        match api.call("pipes.delete", json!({ "name": n.clone() })).await {
                            Ok(_) => { set_pref(ui, |p| p.pipes.retain(|x| *x != n)); ui.touched(); }
                            Err(e) => ui.fail(e),
                        }
                    });
                },
                Icon { name: "delete" } }
        }
    }
}
