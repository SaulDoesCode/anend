//! Stats for the whole app: entries, texts and notes, versions, edges by relation, kinds,
//! how much is stored as borrowings, the sums' spread, lists and tags.

use dioxus::prelude::*;

use crate::gem::Mode;
use crate::ui::*;

#[component]
pub fn Stats() -> Element {
    let ui = ui();
    let api = api();
    let api1 = api.clone();
    let raw = use_resource(move || {
        let api = api1.clone();
        let _ = (ui.changed)();
        async move { api.call("stats", serde_json::json!({})).await }
    });
    let api2 = api.clone();
    let more = use_resource(move || {
        let api = api2.clone();
        let _ = (ui.changed)();
        async move {
            let lists = api.lists().await.map(|l| l.lists).unwrap_or_default();
            let tags = api.all_tags().await.unwrap_or_default();
            let buckets = api.bucket_counts(Mode::Ordinal).await.unwrap_or_default();
            (lists, tags, buckets)
        }
    });
    let v = match &*raw.read() {
        Some(Ok(v)) => v.clone(),
        Some(Err(e)) => return rsx! { div { class: "page", div { class: "card bad", "{e}" } } },
        None => return rsx! { div { class: "page", div { class: "card dim", "…" } } },
    };
    let n = |k: &str| v[k].as_u64().unwrap_or(0);
    let date = |k: &str| {
        v[k].as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.format("%d %b %Y").to_string())
            .unwrap_or_else(|| "–".into())
    };
    let pairs = |k: &str| -> Vec<(String, u64)> {
        v[k].as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_u64()?)))
            .collect()
    };
    let (lists, tags, buckets) = more.read().clone().unwrap_or_default();
    let biggest = buckets.iter().max_by_key(|b| b.count).cloned();
    let edges = pairs("edges");
    let kinds = pairs("by_mime");
    let emax = edges.iter().map(|e| e.1).max().unwrap_or(1).max(1);
    let tmax = tags.iter().map(|e| e.1).max().unwrap_or(1).max(1);
    rsx! {
        div { class: "page",
            div { class: "row", h2 { "stats" } span { class: "dim small", "the whole save" } }
            div { class: "card stats-grid",
                div { class: "stat", b { {n("entries").to_string()} } span { "entries" } }
                div { class: "stat", b { {n("memories").to_string()} } span { "texts" } }
                div { class: "stat", b { {n("notes").to_string()} } span { "notes" } }
                div { class: "stat", b { {n("words").to_string()} } span { "words in notes" } }
                div { class: "stat", b { {n("versions").to_string()} } span { "past versions" } }
                div { class: "stat", b { "{lists.len()}" } span { "lists" } }
                div { class: "stat", b { "{tags.len()}" } span { "tags" } }
                div { class: "stat", b { "{buckets.len()}" } span { "ordinal sums in use" } }
                div { class: "stat", b { {n("borrowing").to_string()} } span { "stored as borrowings" } }
                div { class: "stat", b { {format!("{} kB", n("body_bytes") / 1024)} } span { "packed text" } }
                div { class: "stat", b { {date("first")} } span { "the first" } }
                div { class: "stat", b { {date("latest")} } span { "the latest" } }
            }
            if let Some(b) = biggest {
                div { class: "card", "The fullest ordinal sum is " b { "{b.sum}" } ", holding {b.count} texts." }
            }
            div { class: "card",
                h3 { "relations" }
                div { class: "bars",
                    for (name , c) in edges {
                        div { key: "{name}", class: "hbar", span { "{name}" } span { class: "track", span { class: "fill", style: "width: {c as f64 / emax as f64 * 100.0:.1}%;" } } span { class: "n", "{c}" } }
                    }
                }
                h3 { style: "margin-top: 10px;", "kinds" }
                div { class: "bars",
                    for (name , c) in kinds {
                        div { key: "{name}", class: "hbar", span { "{name}" } span { class: "track" } span { class: "n", "{c}" } }
                    }
                }
            }
            if !tags.is_empty() {
                div { class: "card",
                    h3 { "tags" }
                    div { class: "bars",
                        for (t , c) in tags.into_iter().take(30) {
                            div { key: "{t}", class: "hbar", span { "{t}" } span { class: "track", span { class: "fill", style: "width: {c as f64 / tmax as f64 * 100.0:.1}%;" } } span { class: "n", "{c}" } }
                        }
                    }
                }
            }
        }
    }
}
