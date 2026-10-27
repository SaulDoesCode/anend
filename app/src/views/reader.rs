//! The reader: notes and texts as cards, a page at a time; short texts flow in a tight
//! grid. Clicking one (or Enter on it) expands it to read big, one at a time, ← and →
//! stepping through; Esc goes back. The bar narrows by words.
//!
//! What it shows is a view (`views.run`, docs/architecture.md part 4): its own three
//! (everything, notes, texts) are built-in presets, and any s program that loads
//! `std/view.s.` is a view of its own beside them: where its texts come from, their order,
//! headings, a layout (cards, a tight grid or a list) and the pipes whose effects it shows.

use dioxus::prelude::*;

use serde_json::Value;

use crate::backend::Hit;
use crate::keys::Action;

use crate::ui::*;
use crate::views::use_keys;

#[component]
pub fn Reader() -> Element {
    let ui = ui();
    let api = api();
    let mut view = use_signal(|| "everything".to_string());
    let mut page = use_signal(|| 0usize);
    let mut words = use_signal(String::new);
    let mut expanded: Signal<Option<usize>> = use_signal(|| None);
    let mut tag: Signal<Option<String>> = use_signal(|| None);
    let api_r = api.clone();
    let found = use_resource(move || {
        let api = api_r.clone();
        let _ = (ui.changed)();
        let mut a = serde_json::json!({ "name": view(), "words": words() });
        if let Some(t) = tag() {
            a["tag"] = serde_json::json!(t);
        }
        async move { api.call("views.run", a).await }
    });
    let api_v = api.clone();
    let views = use_resource(move || {
        let api = api_v.clone();
        let _ = (ui.changed)();
        async move {
            api.call("views.list", serde_json::json!({}))
                .await
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
        }
    });
    let views = views.read().clone().unwrap_or_default();
    // each note's dates, for its card (as the Bevy reader showed them)
    let api_n = api.clone();
    let dates = use_resource(move || {
        let api = api_n.clone();
        let _ = (ui.changed)();
        async move {
            api.notes(&crate::backend::NoteQuery::default())
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|n| (n.key.clone(), (n.created.clone(), n.modified.clone(), n.edits)))
                .collect::<std::collections::HashMap<_, _>>()
        }
    });
    let dates = dates.read().clone().unwrap_or_default();
    let got: Value = match &*found.read() {
        Some(Ok(v)) => v.clone(),
        _ => Value::Null,
    };
    let fail = match &*found.read() {
        Some(Err(e)) => Some(e.clone()),
        _ => None,
    };
    let raw: Vec<Value> = got["hits"].as_array().cloned().unwrap_or_default();
    let all: Vec<Hit> = raw
        .iter()
        .filter_map(|h| serde_json::from_value(h.clone()).ok())
        .collect();
    let fx: Vec<Value> = raw.iter().map(|h| h["fx"].clone()).collect();
    // a group's heading goes before its first text
    let heads: std::collections::HashMap<usize, (String, u64)> = got["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|g| !g["name"].as_str().unwrap_or("").is_empty())
        .map(|g| {
            (
                g["from"].as_u64().unwrap_or(0) as usize,
                (
                    g["name"].as_str().unwrap_or("").to_string(),
                    g["count"].as_u64().unwrap_or(0),
                ),
            )
        })
        .collect();
    let layout = got["layout"].as_str().unwrap_or("auto").to_string();
    let title = got["title"].as_str().unwrap_or("").to_string();
    let builtin = views
        .iter()
        .any(|v| v["name"] == view().as_str() && v["builtin"] == true);
    let tiny = match layout.as_str() {
        "grid" => true,
        "cards" | "list" => false,
        _ => all.iter().filter(|h| h.text.chars().count() <= 40 && !h.note).count() * 2 > all.len(),
    };
    let listed = layout == "list";
    let per = if tiny || listed { 240 } else { 48 };
    let pages = all.len().div_ceil(per).max(1);
    let p = page().min(pages - 1);
    let n = all.len();
    let total = got["total"].as_u64().map(|t| t as usize).unwrap_or(n);
    use_keys(View::Reader, move |k: KeyPress| {
        if k.bar && k.key == "Enter" {
            words.set(k.text.trim().to_string());
            page.set(0);
            expanded.set(None);
            return true;
        }
        if k.bar {
            return false;
        }
        let step = match k.action {
            Some(Action::ReadNext) => 1,
            Some(Action::ReadPrev) => -1,
            _ => 0,
        };
        match (k.key.as_str(), step, expanded()) {
            ("Escape", _, Some(_)) => expanded.set(None),
            (_, 1, Some(i)) => expanded.set(Some((i + 1).min(n.saturating_sub(1)))),
            (_, -1, Some(i)) => expanded.set(Some(i.saturating_sub(1))),
            (_, 1, None) => page.set((p + 1).min(pages - 1)),
            (_, -1, None) => page.set(p.saturating_sub(1)),
            _ => return false,
        }
        true
    });
    if let Some(i) = expanded()
        && let Some(h) = all.get(i).cloned()
    {
        return rsx! {
            Expanded {
                hit: h,
                i,
                n,
                on_close: move |_| expanded.set(None),
                on_step: move |d: i64| expanded.set(Some((i as i64 + d).clamp(0, n as i64 - 1) as usize)),
                on_tag: move |t: String| {
                    tag.set(Some(t));
                    page.set(0);
                    expanded.set(None);
                },
            }
        };
    }
    let shown: Vec<(usize, Hit)> = all.into_iter().enumerate().skip(p * per).take(per).collect();
    rsx! {
        div { class: "page", style: "max-width: 1200px;",
            div { class: "row",
                h2 { "reader" }
                for v in views.iter() {
                    {
                        let name = v["name"].as_str().unwrap_or("").to_string();
                        let own = v["builtin"] != true;
                        let tip = if own { format!("your view: the s program “{name}”") } else { v["about"].as_str().unwrap_or("").to_string() };
                        rsx! {
                            button { key: "{name}", class: if view() == name { if own { "chip on own" } else { "chip on" } } else if own { "chip own" } else { "chip" },
                                "data-tip": "{tip}",
                                onclick: { let n = name.clone(); move |_| { view.set(n.clone()); page.set(0); expanded.set(None); } },
                                if own { Icon { name: "editor" } } "{name}" }
                        }
                    }
                }
                span { class: "dim small", "{total} found" if total > n { " (the first {n} here)" } if !words().is_empty() { " for “{words}”" } " · click one to read it big" }
                if !builtin && !title.is_empty() {
                    span { class: "small view-title", "{title}" }
                }
                if let Some(t) = tag() {
                    button { class: "chip on", "data-tip": "show them all again", onclick: move |_| { tag.set(None); page.set(0); }, "#{t}" span { class: "x", " ×" } }
                }
            }
            if let Some(e) = fail {
                div { class: "card bad", "{e}" }
            }
            div { class: if listed { "reader-list" } else if tiny { "reader-grid tiny" } else { "reader-grid" },
                for (i , h) in shown {
                    div { key: "{h.key}", style: "display: contents;",
                    if let Some((g, c)) = heads.get(&i) {
                        h3 { class: "reader-head", "{g}" span { class: "dim small", " {c}" } }
                    }
                    div {
                        class: if listed { "rrow" } else if tiny { "card rcard tiny" } else { "card rcard" },
                        style: fx_style(&fx[i]),
                        tabindex: "0",
                        role: "button",
                        title: "{h.text}",
                        onclick: move |_| expanded.set(Some(i)),
                        onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { expanded.set(Some(i)) },
                        if h.note {
                            h3 { "{h.text}" }
                            {
                                let d = dates.get(h.key.trim_start_matches("note:")).cloned();
                                rsx! {
                                    span { class: "dim small rdate",
                                        match d {
                                            Some((c, m, e)) => rsx! {
                                                "{short_date(&c)}"
                                                if e > 0 && !m.is_empty() && m != c { " · edited {relative(&m)}" }
                                            },
                                            None => rsx! { "a note" },
                                        }
                                    }
                                }
                            }
                        } else {
                            div { class: "body", "{h.text}" }
                        }
                        if let Some(l) = fx[i]["label"].as_str() {
                            span { class: "fx-label", "{l}" }
                        }
                    }
                    }
                }
            }
            if pages > 1 {
                div { class: "pager",
                    button { "aria-label": "the page before", disabled: p == 0, onclick: move |_| page.set(p.saturating_sub(1)), Icon { name: "chevron-left" } }
                    span { "page {p + 1} of {pages}" }
                    button { "aria-label": "the next page", disabled: p + 1 >= pages, onclick: move |_| page.set(p + 1), Icon { name: "chevron-right" } }
                }
            }
        }
    }
}

/// a pipe's effects on a card: its colour as the left edge, its glow, its size
fn fx_style(fx: &Value) -> String {
    let mut s = String::new();
    if let Some(c) = fx["colour"].as_str() {
        s.push_str(&format!("border-left: 5px solid {c};"));
    }
    if let Some(c) = fx["underline"].as_str() {
        s.push_str(&format!("text-decoration: underline; text-decoration-color: {c};"));
    }
    if let Some(c) = fx["glow"].as_str() {
        s.push_str(&format!("box-shadow: 0 0 10px {c};"));
    }
    if let Some(z) = fx["size"].as_f64() {
        s.push_str(&format!("font-size: {z:.2}em;"));
    }
    s
}

/// a card's date: `19 October 2026`
fn short_date(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|t| t.with_timezone(&chrono::Local).format("%-d %B %Y").to_string())
        .unwrap_or_default()
}

/// how long ago, in words
fn relative(iso: &str) -> String {
    let full = long_date(iso);
    full.rsplit_once('(')
        .map(|(_, r)| r.trim_end_matches(')').to_string())
        .unwrap_or(full)
}

/// a date for people: `19 Oct 2026, 08:46`, and how long ago
fn long_date(iso: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(iso) else {
        return String::new();
    };
    let local = t.with_timezone(&chrono::Local);
    let ago = chrono::Local::now().signed_duration_since(local);
    let rel = if ago.num_days() >= 365 {
        format!("{} years ago", ago.num_days() / 365)
    } else if ago.num_days() >= 30 {
        format!("{} months ago", ago.num_days() / 30)
    } else if ago.num_days() >= 1 {
        format!("{} days ago", ago.num_days())
    } else if ago.num_hours() >= 1 {
        format!("{} hours ago", ago.num_hours())
    } else if ago.num_minutes() >= 1 {
        format!("{} minutes ago", ago.num_minutes())
    } else {
        "just now".into()
    };
    format!("{} ({rel})", local.format("%A %-d %B %Y, %H:%M"))
}

#[component]
fn Expanded(
    hit: Hit,
    i: usize,
    n: usize,
    on_close: EventHandler<()>,
    on_step: EventHandler<i64>,
    on_tag: EventHandler<String>,
) -> Element {
    let mut ui = ui();
    let api = api();
    let key = hit.key.clone();
    let note = hit.note;
    let api_b = api.clone();
    let body = use_resource(move || {
        let api = api_b.clone();
        let key = key.clone();
        async move { if note { api.note(&key).await.ok() } else { None } }
    });
    // when it was written and last changed (any entry)
    let k2 = hit.key.clone();
    let info = use_resource(move || {
        let api = api.clone();
        let k = k2.clone();
        async move { api.call("entries.info", serde_json::json!({ "key": k })).await.ok() }
    });
    let mut judging = use_signal(|| true);
    let n_body = body.read().clone().flatten();
    let info = info.read().clone().flatten();
    let created = info
        .as_ref()
        .and_then(|v| v["created"].as_str())
        .unwrap_or("")
        .to_string();
    let edited = info
        .as_ref()
        .and_then(|v| v["edited"].as_str())
        .unwrap_or("")
        .to_string();
    let kind = info.as_ref().and_then(|v| v["kind"].as_str()).unwrap_or("").to_string();
    let edits = n_body.as_ref().map(|n| n.edits.len()).unwrap_or(0);
    let tags = n_body.as_ref().map(|n| n.tags.clone()).unwrap_or_default();
    let md = n_body.as_ref().map(|n| n.markdown.clone()).unwrap_or_default();
    let open_key = hit.key.clone();
    rsx! {
        div { class: "card expand",
            div { class: "row",
                button { class: "plain", "aria-label": "back", onclick: move |_| on_close.call(()), Icon { name: "chevron-left" } "back" }
                span { class: "grow" }
                if note {
                    button { class: "plain", "data-tip": "edit it in the notes view", "aria-label": "edit",
                        onclick: move |_| {
                            ui.open_note.set(Some(open_key.clone()));
                            crate::shell::go(ui, View::Notes);
                        },
                        Icon { name: "edit" } "edit" }
                }
                button { class: if judging() { "plain on" } else { "plain" }, "data-tip": "how it measures on your classifiers", "aria-label": "classifiers",
                    onclick: move |_| judging.set(!judging()), Icon { name: "classifier" } }
                button { class: "plain", "aria-label": "the one before", "data-tip": "the one before · {ui.key_of(Action::ReadPrev)}", disabled: i == 0, onclick: move |_| on_step.call(-1), Icon { name: "chevron-left" } }
                span { class: "dim", "{i + 1} of {n}" }
                button { class: "plain", "aria-label": "the next one", "data-tip": "the next one · {ui.key_of(Action::ReadNext)}", disabled: i + 1 >= n, onclick: move |_| on_step.call(1), Icon { name: "chevron-right" } }
            }
            if hit.note {
                h2 { "{hit.text}" }
            }
            div { class: "dates dim small",
                if !created.is_empty() { span { "written " b { "{long_date(&created)}" } } }
                if !edited.is_empty() && edited != created { span { " · last changed " b { "{long_date(&edited)}" } } }
                if edits > 0 { span { " · {edits} edit" if edits != 1 { "s" } } }
                if !kind.is_empty() { span { " · a {kind}" } }
            }
            if !tags.is_empty() {
                div { class: "row",
                    for t in tags {
                        button { key: "{t}", class: "chip", "data-tip": "the notes tagged #{t}", onclick: { let t = t.clone(); move |_| on_tag.call(t.clone()) }, "#{t}" }
                    }
                }
            }
            if hit.note {
                div { class: "md", dangerous_inner_html: crate::md::to_html(&md) }
            } else {
                div { class: "text", style: "font-size: 1.6em;", "{hit.text}" }
            }
            div { class: "dim small", "{hit.key} · {crate::gem::gsum_line(&hit.text)}" }
            if judging() {
                crate::views::judge::Judge { key: "{hit.key}", entry: hit.key.clone(), title: "how it measures on:" }
            }
        }
    }
}
