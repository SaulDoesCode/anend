//! Lists: every list as a tile; a list's own page has its name at the top (editable), its
//! members as editable entries a page at a time, and its figures at the bottom. Deleting
//! a list takes two clicks. The bar finds lists by name, or (Enter, nothing found) makes
//! one: a new text heading an empty list waits for members from the memory view.

use dioxus::prelude::*;

use crate::backend::{Hit, ListInfo};
use crate::gem;
use crate::ui::*;
use crate::views::use_keys;

const PAGE: usize = 40;

#[component]
pub fn Lists() -> Element {
    let ui = ui();
    let api = api();
    let mut open: Signal<Option<String>> = use_signal(|| None);
    let api_r = api.clone();
    let lists = use_resource(move || {
        let api = api_r.clone();
        let _ = (ui.changed)();
        async move { api.lists().await }
    });
    {
        let api = api.clone();
        use_keys(View::Lists, move |k: KeyPress| {
            if k.bar && k.key == "Enter" {
                let name = k.text.trim().to_string();
                let found = lists.peek().as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| {
                    l.lists
                        .iter()
                        .find(|x| x.name.to_lowercase().contains(&name.to_lowercase()))
                        .cloned()
                });
                match found {
                    Some(l) if !name.is_empty() => open.set(Some(l.list)),
                    _ if !name.is_empty() => {
                        let api = api.clone();
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.saturate(&name).await {
                                Ok((key, _)) => {
                                    let mut p = ui.picks;
                                    p.write().list = Some(Hit {
                                        key,
                                        text: name.clone(),
                                        note: false,
                                    });
                                    ui.say(format!("“{name}” is the list now: Ctrl+click texts in the memory view and type Enlist, or Alt+click several and put them in it"));
                                }
                                Err(e) => ui.fail(e),
                            }
                        });
                    }
                    _ => {}
                }
                return true;
            }
            if !k.bar && k.key == "Escape" && open.peek().is_some() {
                open.set(None);
                return true;
            }
            false
        });
    }
    let q = ui.bar.read().trim().to_lowercase();
    let all: Vec<ListInfo> = match &*lists.read() {
        Some(Ok(l)) => l.lists.clone(),
        Some(Err(e)) => return rsx! { div { class: "page", div { class: "card bad", "{e}" } } },
        None => return rsx! { div { class: "page", div { class: "card dim", "…" } } },
    };
    if let Some(key) = open()
        && let Some(l) = all.iter().find(|l| l.list == key).cloned()
    {
        return rsx! { ListPage { list: l, back: move |_| open.set(None) } };
    }
    let shown: Vec<ListInfo> = all
        .into_iter()
        .filter(|l| q.is_empty() || l.name.to_lowercase().contains(&q))
        .collect();
    rsx! {
        div { class: "page",
            div { class: "row", h2 { "lists" } span { class: "dim", "{shown.len()} shown · type to find one, Enter on a new name starts a list" } }
            if shown.is_empty() {
                div { class: "card empty", "No lists yet. In the memory view, Shift+click a text to make it a list's head, Ctrl+click another and type Enlist; or Alt+click several and put them in a list." }
            }
            div { class: "list-grid",
                for l in shown {
                    button {
                        key: "{l.list}",
                        class: "card list-tile",
                        onclick: { let k = l.list.clone(); move |_| open.set(Some(k.clone())) },
                        h3 { "{l.name}" }
                        span { class: "dim small", "{l.count} members · ordinal mean {l.ordinal_mean:.1} · reversed mean {l.reversed_mean:.1}" }
                        span { class: "small", {l.members.iter().take(6).map(|m| m.text.clone()).collect::<Vec<_>>().join(" · ")} }
                    }
                }
            }
        }
    }
}

#[component]
fn ListPage(list: ListInfo, back: EventHandler<()>) -> Element {
    let ui = ui();
    let api = api();
    let mut page = use_signal(|| 0usize);
    let mut armed = use_signal(|| false);
    let mut name = use_signal(|| list.name.clone());
    let pages = list.members.len().div_ceil(PAGE).max(1);
    let p = page().min(pages - 1);
    let members: Vec<Hit> = list.members.iter().skip(p * PAGE).take(PAGE).cloned().collect();
    let key = list.list.clone();
    let k2 = key.clone();
    let api2 = api.clone();
    let api3 = api.clone();
    let old_name = list.name.clone();
    rsx! {
        div { class: "page list-page",
            div { class: "row",
                button { class: "plain tip-right", "data-tip": "all the lists · Esc", "aria-label": "back", onclick: move |_| back.call(()), Icon { name: "chevron-left" } }
                input {
                    class: "name",
                    r#type: "text",
                    "aria-label": "the list's name",
                    initial_value: "{name}",
                    style: "flex: 1; height: auto; background: none;",
                    oninput: move |e| name.set(e.value()),
                    onchange: move |_| {
                        let n = name();
                        if n.trim().is_empty() || n == old_name { return; }
                        let (api, key, old) = (api2.clone(), k2.clone(), old_name.clone());
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.put_memory(&key, &n).await {
                                Ok(()) => { ui.did(Did::Edited { key, old }); ui.say("renamed"); ui.touched(); }
                                Err(e) => ui.fail(e),
                            }
                        });
                    },
                }
                button {
                    class: if armed() { "armed" } else { "plain" },
                    "data-tip": "delete the list (its members stay); click twice",
                    "aria-label": "delete the list",
                    onclick: move |_| {
                        if !armed() { armed.set(true); return; }
                        let (api, key) = (api3.clone(), key.clone());
                        let members: Vec<String> = list.members.iter().map(|h| h.key.clone()).collect();
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.unlist(&key).await {
                                Ok(n) => { ui.did(Did::Unlisted { list: key, members }); ui.say(format!("the list is gone ({n} members); Ctrl+Z brings it back")); ui.touched(); }
                                Err(e) => ui.fail(e),
                            }
                        });
                        back.call(());
                    },
                    Icon { name: "delete" }
                    if armed() { "delete the list?" }
                }
            }
            div { class: "card members",
                if members.is_empty() { span { class: "dim", "no members yet" } }
                for (i , m) in members.into_iter().enumerate() {
                    Member { key: "{m.key}", list: list.list.clone(), hit: m, n: p * PAGE + i + 1 }
                }
            }
            if pages > 1 {
                div { class: "pager",
                    button { disabled: p == 0, onclick: move |_| page.set(p.saturating_sub(1)), Icon { name: "chevron-left" } }
                    span { "page {p + 1} of {pages}" }
                    button { disabled: p + 1 >= pages, onclick: move |_| page.set(p + 1), Icon { name: "chevron-right" } }
                }
            }
            Figures { list: list.clone() }
        }
    }
}

#[component]
fn Member(list: String, hit: Hit, n: usize) -> Element {
    let ui = ui();
    let api = api();
    let mut text = use_signal(|| hit.text.clone());
    let (api1, api2) = (api.clone(), api.clone());
    let h1 = hit.clone();
    let h2 = hit.clone();
    rsx! {
        div { class: "member",
            span { class: "key", title: "its key", "{n}. {hit.key}" }
            input {
                r#type: "text",
                "aria-label": "member {n}",
                initial_value: "{text}",
                readonly: hit.note,
                oninput: move |e| text.set(e.value()),
                onchange: move |_| {
                    let t = text();
                    if t == h1.text || t.trim().is_empty() { return; }
                    let (api, h) = (api1.clone(), h1.clone());
                    dioxus::dioxus_core::spawn_forever(async move {
                        match api.put_memory(&h.key, &t).await {
                            Ok(()) => { ui.did(Did::Edited { key: h.key.clone(), old: h.text.clone() }); ui.say(format!("{} now reads “{t}”", h.key)); }
                            Err(e) => ui.fail(e),
                        }
                    });
                },
            }
            button {
                class: "plain tip-left",
                "data-tip": "take it out of the list (it stays a text)",
                "aria-label": "take out of the list",
                onclick: move |_| {
                    let (api, h, l) = (api2.clone(), h2.clone(), list.clone());
                    dioxus::dioxus_core::spawn_forever(async move {
                        match api.delist(&l, &h.key).await {
                            Ok(()) => { ui.did(Did::Delisted { list: l, member: h.key }); ui.touched(); }
                            Err(e) => ui.fail(e),
                        }
                    });
                },
                Icon { name: "close" }
            }
        }
    }
}

#[component]
fn Figures(list: ListInfo) -> Element {
    let joined: String = list
        .members
        .iter()
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let (s, r) = gem::sums(&joined);
    let max = list.vector.iter().cloned().fold(0.0f64, f64::max).max(1e-9);
    rsx! {
        div { class: "card",
            h3 { "figures" }
            div { class: "figures",
                div { b { "{list.count}" } "members" }
                div { b { "{list.ordinal_sum}" } "ordinal, all of them" }
                div { b { "{list.reversed_sum}" } "reversed, all of them" }
                div { b { "{list.ordinal_mean:.2}" } "ordinal mean" }
                div { b { "{list.reversed_mean:.2}" } "reversed mean" }
                div { b { "{gem::byte_sum(&joined)}" } "bytes (1–256)" }
                div { b { "{((s as f64) * (r as f64)).sqrt().floor()}" } "⌊√(s·r)⌋ of them whole" }
            }
            if !list.vector.is_empty() {
                div { class: "letterbars", title: "the members' letters",
                    for (i , v) in list.vector.iter().enumerate() {
                        div { key: "{i}", class: "col",
                            span { class: "bar", style: "height: {(v / max * 34.0).max(1.0):.1}px;" }
                            span { class: "l", {((b'a' + i as u8) as char).to_string()} }
                        }
                    }
                }
            }
        }
    }
}
