//! Filters (the models view is `classifiers.rs`).
//!
//! **Filters** run the query model (`entries.query`): words, a sum range, a time range, a
//! tag and a kind, intersected. Filters are kept in the save's settings by name.

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::backend::{EntryQuery, Hit};
use crate::ui::*;
use crate::views::use_keys;

#[component]
pub fn Filters() -> Element {
    let ui = ui();
    let api = api();
    let mut q = use_signal(EntryQuery::default);
    let mut since = use_signal(String::new);
    let mut until = use_signal(String::new);
    let mut name = use_signal(String::new);
    let mut form = use_signal(|| 0u64);
    let mut found: Signal<Option<(u64, Vec<Hit>)>> = use_signal(|| None);
    let api_s = api.clone();
    let saved = use_resource(move || {
        let api = api_s.clone();
        let _ = (ui.changed)();
        async move {
            let raw = api.setting("filters").await.ok().flatten().unwrap_or_default();
            serde_json::from_str::<Vec<(String, Value)>>(&raw).unwrap_or_default()
        }
    });
    let run = {
        let api = api.clone();
        move || {
            let api = api.clone();
            let mut query = q.peek().clone();
            let day = |d: &str, end: bool| {
                (!d.is_empty()).then(|| format!("{d}T{}", if end { "23:59:59Z" } else { "00:00:00Z" }))
            };
            query.since =
                day(&since.peek(), false).or_else(|| (!until.peek().is_empty()).then(|| "1970-01-01T00:00:00Z".into()));
            query.until =
                day(&until.peek(), true).or_else(|| query.since.as_ref().map(|_| "9999-12-31T23:59:59Z".into()));
            query.limit = 300;
            dioxus::dioxus_core::spawn_forever(async move {
                match api.query(&query).await {
                    Ok(r) => found.set(Some(r)),
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    {
        let run = run.clone();
        use_keys(View::Filters, move |k: KeyPress| {
            if !(k.bar && k.key == "Enter") {
                return false;
            }
            // a sum (79) or a range (70-90), else words
            let t = k.text.trim().to_string();
            let range: Option<(i64, i64)> = match t.split_once('-') {
                Some((a, b)) => a.trim().parse().ok().zip(b.trim().parse().ok()),
                None => t.parse().ok().map(|n| (n, n)),
            };
            let mut w = q.write();
            match range {
                Some((a, b)) => {
                    w.from = Some(a.min(b));
                    w.to = Some(a.max(b));
                    if w.bucket.is_empty() {
                        w.bucket = "ordinal".into();
                    }
                }
                None => w.words = t,
            }
            drop(w);
            form += 1;
            run();
            true
        });
    }
    let cur = q();
    let saved_list = saved.read().clone().unwrap_or_default();
    let api_save = api.clone();
    let run2 = run.clone();
    rsx! {
        div { class: "page",
            div { class: "row", h2 { "filters" } span { class: "dim small", "find texts by words, sums, time, tags and kind; below, by what classifiers say, in groups" } }
            // remade when a kept filter or the bar fills it (its inputs aren't bound while typing)
            for g in [form()] {
            div { key: "{g}", class: "card",
                div { class: "cond", Icon { name: "search" } "words"
                    input { r#type: "text", initial_value: "{cur.words}", placeholder: "any", oninput: move |e| q.write().words = e.value() } }
                div { class: "cond", Icon { name: "hash" } "sum"
                    select { value: "{cur.bucket}", onchange: move |e| q.write().bucket = e.value(),
                        option { value: "", "any" } option { value: "ordinal", "ordinal" } option { value: "reversed", "reversed" } }
                    "from" input { r#type: "number", initial_value: cur.from.map(|v| v.to_string()).unwrap_or_default(), oninput: move |e| q.write().from = e.value().parse().ok() }
                    "to" input { r#type: "number", initial_value: cur.to.map(|v| v.to_string()).unwrap_or_default(), oninput: move |e| q.write().to = e.value().parse().ok() } }
                div { class: "cond", Icon { name: "sort" } "written between"
                    input { r#type: "date", initial_value: "{since}", oninput: move |e| since.set(e.value()) } "and"
                    input { r#type: "date", initial_value: "{until}", oninput: move |e| until.set(e.value()) } }
                div { class: "cond", Icon { name: "tag" } "tag"
                    input { r#type: "text", initial_value: cur.tag.clone().unwrap_or_default(), placeholder: "any", oninput: move |e| { let v = e.value(); q.write().tag = (!v.is_empty()).then_some(v); } } }
                div { class: "cond", Icon { name: "filter" } "kind"
                    for (k , label) in [("", "everything"), ("memories", "texts"), ("notes", "notes")] {
                        button { key: "{k}", class: if cur.kind == k { "chip on" } else { "chip" }, onclick: move |_| q.write().kind = k.to_string(), "{label}" }
                    } }
                div { class: "row",
                    button { class: "primary", onclick: { let run2 = run2.clone(); move |_| run2() }, Icon { name: "play" } "run" }
                    input { r#type: "text", placeholder: "a name to keep it", initial_value: "{name}", oninput: move |e| name.set(e.value()) }
                    button { disabled: name().trim().is_empty(), onclick: { let api_save = api_save.clone(); move |_| {
                            let api = api_save.clone();
                            let mut all = saved.peek().clone().unwrap_or_default();
                            let n = name.peek().trim().to_string();
                            all.retain(|(x, _)| *x != n);
                            all.push((n.clone(), json!({ "q": serde_json::to_value(&*q.peek()).unwrap_or_default(), "since": since.peek().clone(), "until": until.peek().clone() })));
                            dioxus::dioxus_core::spawn_forever(async move {
                                match api.set_setting("filters", &serde_json::to_string(&all).unwrap_or_default()).await {
                                    Ok(()) => { ui.say(format!("kept “{n}”")); ui.touched(); }
                                    Err(e) => ui.fail(e),
                                }
                            });
                        } }, Icon { name: "save" } "keep" }
                }
            }
            }
            if !saved_list.is_empty() {
                div { class: "row",
                    span { class: "dim", "kept:" }
                    for (n , v) in saved_list {
                        button { key: "{n}", class: "chip", onclick: {
                                let run = run.clone();
                                move |_| {
                                    if let Ok(query) = serde_json::from_value::<EntryQuery>(v["q"].clone()) { q.set(query); }
                                    form += 1;
                                    since.set(v["since"].as_str().unwrap_or("").to_string());
                                    until.set(v["until"].as_str().unwrap_or("").to_string());
                                    name.set(n.clone());
                                    run();
                                }
                            }, "{n}" }
                    }
                }
            }
            crate::views::flt::Builder {}
            if let Some((total, hits)) = found() {
                div { class: "card",
                    h3 { "{total} found" if total as usize > hits.len() { " (the first {hits.len()})" } }
                    div { class: "results",
                        for h in hits {
                            div { key: "{h.key}", class: "result",
                                span { class: "p", "{h.key}" }
                                span { class: "t", title: "{h.text}", if h.note { "✎ " } "{h.text}" }
                                button { class: "plain", "aria-label": "into the bar", "data-tip": "into the bar",
                                    onclick: { let t = h.text.clone(); move |_| ui.set_bar(&t) }, Icon { name: "text" } }
                            }
                        }
                    }
                }
            }
        }
    }
}
