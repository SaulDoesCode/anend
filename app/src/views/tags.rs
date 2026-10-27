//! Tags: every tag (on any entry, texts and notes alike) with its count; click picks one,
//! Ctrl or Shift+click adds or drops it; all or any decides how several combine. The
//! entries carrying them show beside. The bar narrows the tags; Enter picks the first.

use dioxus::prelude::*;

use crate::ui::*;
use crate::views::use_keys;

#[component]
pub fn Tags() -> Element {
    let ui = ui();
    let api = api();
    let mut picked: Signal<Vec<String>> = use_signal(Vec::new);
    let mut any = use_signal(|| false);
    // a tag another view asked for (the stats' tags)
    use_effect(move || {
        if let Some(t) = (ui.pick_tag)() {
            let mut pt = ui.pick_tag;
            pt.set(None);
            picked.set(vec![t]);
        }
    });
    let api_t = api.clone();
    let tags = use_resource(move || {
        let api = api_t.clone();
        let _ = (ui.changed)();
        async move { api.all_tags().await.unwrap_or_default() }
    });
    let api_e = api.clone();
    let found = use_resource(move || {
        let api = api_e.clone();
        let p = picked();
        let a = any();
        let _ = (ui.changed)();
        async move {
            if p.is_empty() {
                Ok(vec![])
            } else {
                api.tagged(&p, a).await
            }
        }
    });
    let q = ui.bar.read().trim().to_lowercase();
    let all: Vec<(String, u64)> = tags.read().clone().unwrap_or_default();
    let shown: Vec<(String, u64)> = all
        .into_iter()
        .filter(|(t, _)| q.is_empty() || t.to_lowercase().contains(&q))
        .collect();
    let first = shown.first().map(|t| t.0.clone());
    use_keys(View::Tags, move |k: KeyPress| {
        if k.bar && k.key == "Enter" {
            if let Some(t) = first.clone() {
                let mut p = picked.write();
                if !p.contains(&t) {
                    p.push(t);
                }
                drop(p);
                ui.set_bar("");
            }
            return true;
        }
        false
    });
    let hits = match &*found.read() {
        Some(Ok(h)) => h.clone(),
        _ => vec![],
    };
    rsx! {
        div { class: "page",
            div { class: "row",
                h2 { "tags" }
                button { class: if !any() { "chip on" } else { "chip" }, onclick: move |_| any.set(false), "data-tip": "entries with every picked tag", "all" }
                button { class: if any() { "chip on" } else { "chip" }, onclick: move |_| any.set(true), "data-tip": "entries with any picked tag", "any" }
                if !picked().is_empty() {
                    button { class: "plain", onclick: move |_| picked.set(vec![]), Icon { name: "close" } "clear" }
                }
            }
            div { class: "card taglist", role: "group", "aria-label": "tags",
                if shown.is_empty() {
                    span { class: "dim", "No tags yet: right-click a text in the memory view and pick “tag…”, or give a note tags." }
                }
                for (t , n) in shown {
                    button {
                        key: "{t}",
                        class: if picked().contains(&t) { "chip on" } else { "chip" },
                        "aria-pressed": "{picked().contains(&t)}",
                        title: "click: just this · Ctrl/Shift+click: add or drop",
                        onclick: move |e: MouseEvent| {
                            let m = e.modifiers();
                            let mut p = picked.write();
                            if m.ctrl() || m.shift() || m.meta() {
                                if let Some(i) = p.iter().position(|x| *x == t) { p.remove(i); } else { p.push(t.clone()); }
                            } else {
                                *p = vec![t.clone()];
                            }
                        },
                        "{t} "
                        span { class: "dim", "{n}" }
                    }
                }
            }
            if !picked().is_empty() {
                div { class: "card",
                    h3 { {format!("{} carrying {}", hits.len(), picked().join(if any() { " or " } else { " and " }))} }
                    div { class: "results",
                        for h in hits {
                            div { key: "{h.key}", class: "result",
                                span { class: "p", "{h.key}" }
                                span { class: "t", title: if h.note { "open it in the editor" } else { "into the bar" },
                                    onclick: { let h = h.clone(); move |_| if h.note { let mut o = ui.open_note; o.set(Some(h.key.clone())); crate::shell::go(ui, View::Notes); } else { ui.set_bar(&h.text) } },
                                    if h.note { "✎ " } "{h.text}" }
                                button { class: "plain", "data-tip": "into the bar", "aria-label": "into the bar",
                                    onclick: { let t = h.text.clone(); move |_| ui.set_bar(&t) }, Icon { name: "text" } }
                            }
                        }
                    }
                }
            }
        }
    }
}
