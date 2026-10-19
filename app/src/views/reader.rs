//! The reader: notes and texts as cards, a page at a time; short texts flow in a tight
//! grid. Clicking one (or Enter on it) expands it to read big, one at a time, ← and →
//! stepping through; Esc goes back. The bar narrows by words.

use dioxus::prelude::*;

use crate::backend::{EntryQuery, Hit};
use crate::keys::Action;
use crate::ui::*;
use crate::views::use_keys;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Notes,
    Texts,
    All,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Notes => "notes",
            Kind::Texts => "texts",
            Kind::All => "everything",
        }
    }
    fn query(self) -> &'static str {
        match self {
            Kind::Notes => "notes",
            Kind::Texts => "memories",
            Kind::All => "",
        }
    }
}

#[component]
pub fn Reader() -> Element {
    let ui = ui();
    let api = api();
    let mut kind = use_signal(|| Kind::All);
    let mut page = use_signal(|| 0usize);
    let mut words = use_signal(String::new);
    let mut expanded: Signal<Option<usize>> = use_signal(|| None);
    let api_r = api.clone();
    let found = use_resource(move || {
        let api = api_r.clone();
        let _ = (ui.changed)();
        let q = EntryQuery {
            words: words(),
            kind: kind().query().to_string(),
            limit: 20_000,
            ..Default::default()
        };
        async move { api.query(&q).await }
    });
    let all: Vec<Hit> = match &*found.read() {
        Some(Ok((_, h))) => h.clone(),
        _ => vec![],
    };
    let tiny = all.iter().filter(|h| h.text.chars().count() <= 40 && !h.note).count() * 2 > all.len();
    let per = if tiny { 240 } else { 48 };
    let pages = all.len().div_ceil(per).max(1);
    let p = page().min(pages - 1);
    let n = all.len();
    let total = match &*found.read() {
        Some(Ok((t, _))) => *t as usize,
        _ => n,
    };
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
        return rsx! { Expanded { hit: h, i, n, on_close: move |_| expanded.set(None), on_step: move |d: i64| expanded.set(Some((i as i64 + d).clamp(0, n as i64 - 1) as usize)) } };
    }
    let shown: Vec<(usize, Hit)> = all.into_iter().enumerate().skip(p * per).take(per).collect();
    rsx! {
        div { class: "page", style: "max-width: 1200px;",
            div { class: "row",
                h2 { "reader" }
                for k in [Kind::All, Kind::Notes, Kind::Texts] {
                    button { key: "{k.name()}", class: if kind() == k { "chip on" } else { "chip" }, onclick: move |_| { kind.set(k); page.set(0); }, "{k.name()}" }
                }
                span { class: "dim small", "{total} found" if total > n { " (the first {n} here)" } if !words().is_empty() { " for “{words}”" } " · click one to read it big" }
            }
            div { class: if tiny { "reader-grid tiny" } else { "reader-grid" },
                for (i , h) in shown {
                    div {
                        key: "{h.key}",
                        class: if tiny { "card rcard tiny" } else { "card rcard" },
                        tabindex: "0",
                        role: "button",
                        title: "{h.text}",
                        onclick: move |_| expanded.set(Some(i)),
                        onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { expanded.set(Some(i)) },
                        if h.note {
                            h3 { "{h.text}" }
                            span { class: "dim small", "a note · {h.key}" }
                        } else {
                            div { class: "body", "{h.text}" }
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

#[component]
fn Expanded(hit: Hit, i: usize, n: usize, on_close: EventHandler<()>, on_step: EventHandler<i64>) -> Element {
    let api = api();
    let key = hit.key.clone();
    let note = hit.note;
    let body = use_resource(move || {
        let api = api.clone();
        let key = key.clone();
        async move {
            if note {
                api.note(&key).await.ok().map(|n| n.markdown)
            } else {
                None
            }
        }
    });
    rsx! {
        div { class: "card expand",
            div { class: "row",
                button { class: "plain", "aria-label": "back", onclick: move |_| on_close.call(()), Icon { name: "chevron-left" } "back" }
                span { class: "grow" }
                button { class: "plain", "aria-label": "the one before", disabled: i == 0, onclick: move |_| on_step.call(-1), Icon { name: "chevron-left" } }
                span { class: "dim", "{i + 1} of {n}" }
                button { class: "plain", "aria-label": "the next one", disabled: i + 1 >= n, onclick: move |_| on_step.call(1), Icon { name: "chevron-right" } }
            }
            if hit.note {
                h2 { "{hit.text}" }
                div { class: "md", dangerous_inner_html: crate::md::to_html(&body.read().clone().flatten().unwrap_or_default()) }
            } else {
                div { class: "text", style: "font-size: 1.6em;", "{hit.text}" }
            }
            div { class: "dim small", "{hit.key} · {crate::gem::gsum_line(&hit.text)}" }
        }
    }
}
