//! Lists, as the Bevy app had them (lists_ui.rs): every list as a card with its figures,
//! the list read as one text, its letters as a vector and its members as chips with their
//! sums (× takes one out); how alike every two lists are (the cosine of their letter
//! vectors); a chosen list the bar fills (Enter adds what's typed, remembering it when
//! new), or, with none chosen, Enter chooses a list by its name or starts one. A list's
//! own page (its name clicked) renames it, edits its members a page at a time, and holds
//! its pictures. Unlisting takes two clicks, and Ctrl+Z brings a list back.

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
    // the list the bar fills (the Bevy app's "chosen"): Enter adds what's typed to it
    let mut chosen: Signal<Option<Hit>> = use_signal(|| None);
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
                let typed = k.text.trim().to_string();
                if typed.is_empty() {
                    return true;
                }
                let api = api.clone();
                let list = chosen.peek().clone();
                let exact = lists
                    .peek()
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .and_then(|l| l.lists.iter().find(|x| x.name.eq_ignore_ascii_case(&typed)).cloned());
                dioxus::dioxus_core::spawn_forever(async move {
                    match (list, exact) {
                        // a list is chosen: the text joins it (remembered when it's new)
                        (Some(l), _) => match api.saturate(&typed).await {
                            Ok((key, _)) if key == l.key => ui.fail("a list can't hold itself"),
                            Ok((key, _)) => match api.enlist(&l.key, &key).await {
                                Ok(()) => {
                                    ui.did(Did::Enlisted {
                                        list: l.key.clone(),
                                        member: key,
                                    });
                                    ui.say(format!("added “{typed}” to {}", l.text));
                                    ui.set_bar("");
                                    ui.touched();
                                }
                                Err(e) => ui.fail(e),
                            },
                            Err(e) => ui.fail(e),
                        },
                        // a list by that name: choose it
                        (None, Some(l)) => {
                            chosen.set(Some(Hit {
                                key: l.list.clone(),
                                text: l.name.clone(),
                                note: false,
                            }));
                            ui.set_bar("");
                            ui.say(format!("Enter adds to “{}” now", l.name));
                        }
                        // else a new list headed by the text, chosen
                        (None, None) => match api.saturate(&typed).await {
                            Ok((key, _)) => {
                                let h = Hit {
                                    key,
                                    text: typed.clone(),
                                    note: false,
                                };
                                chosen.set(Some(h.clone()));
                                let mut picks = ui.picks;
                                picks.write().list = Some(h);
                                ui.set_bar("");
                                ui.say(format!("a new list “{typed}”: type its members, Enter after each"));
                                ui.touched();
                            }
                            Err(e) => ui.fail(e),
                        },
                    }
                });
                return true;
            }
            if !k.bar && k.key == "Escape" {
                if open.peek().is_some() {
                    open.set(None);
                } else {
                    chosen.set(None);
                }
                return true;
            }
            false
        });
    }
    use_effect(move || {
        let note = match chosen() {
            Some(l) => format!("Enter adds to “{}”", l.text),
            None => "Enter chooses a list by its name, or starts one".to_string(),
        };
        ui.side_note(View::Lists, note);
    });
    let q = ui.bar.read().trim().to_lowercase();
    let (all, dot): (Vec<ListInfo>, Vec<Vec<f64>>) = match &*lists.read() {
        Some(Ok(l)) => (l.lists.clone(), l.dot.clone()),
        Some(Err(e)) => return rsx! { div { class: "page", div { class: "card bad", "{e}" } } },
        None => return rsx! { div { class: "page", div { class: "card dim", "…" } } },
    };
    let names: Vec<String> = all.iter().map(|l| l.name.clone()).collect();
    if let Some(key) = open()
        && let Some(l) = all.iter().find(|l| l.list == key).cloned()
    {
        return rsx! { ListPage { list: l, back: move |_| open.set(None) } };
    }
    let finding = chosen().is_none() && !q.is_empty();
    let shown: Vec<ListInfo> = all
        .iter()
        .filter(|l| !finding || l.name.to_lowercase().contains(&q))
        .cloned()
        .collect();
    let cur = chosen();
    rsx! {
        div { class: "page lists-page",
            div { class: "card lists-head",
                div { class: "row",
                    h2 { "lists" }
                    span { class: "dim small", "{all.len()} lists" if finding { " · {shown.len()} named like “{q}”" } }
                }
                match &cur {
                    Some(l) => rsx! {
                        div { class: "row",
                            span { "Typing in the bar adds to " b { "“{l.text}”" } ". Choose another list below, or let go of it to start a new one." }
                            button { onclick: move |_| chosen.set(None), Icon { name: "close" } "let go of it" }
                        }
                    },
                    None => rsx! {
                        div { class: "dim", "Type in the bar to start a new list headed by that text (or a list's name to choose it); choose a list below to add to it. In the memory view, Shift+click a text to head a list and Ctrl+click another, then Enlist." }
                    },
                }
            }
            if all.len() > 1 {
                Alike { all: all.clone(), dot: dot.clone() }
            }
            if shown.is_empty() && all.is_empty() {
                div { class: "card empty", "No lists yet." }
            }
            for l in shown {
                ListCard {
                    key: "{l.list}",
                    is_chosen: cur.as_ref().is_some_and(|c| c.key == l.list),
                    list: l.clone(),
                    on_choose: move |h: Hit| chosen.set(Some(h)),
                    on_open: move |k: String| open.set(Some(k)),
                }
            }
            crate::views::pictures::Pictures { list: None, names: names.clone() }
        }
    }
}

/// how alike the lists are: the cosine of each two lists' letter vectors (1 is the same
/// letters in the same proportions), as the Bevy app's grid
#[component]
fn Alike(all: Vec<ListInfo>, dot: Vec<Vec<f64>>) -> Element {
    if dot.len() != all.len() {
        return rsx! {};
    }
    let n = all.len();
    rsx! {
        div { class: "card",
            h3 { "how alike" }
            div { class: "dim small", "the cosine of each two lists' letter vectors: 1 is the same letters in the same proportions" }
            div { class: "alike", style: "grid-template-columns: 130px repeat({n}, minmax(38px, 52px));",
                span {}
                for l in all.iter() {
                    span { key: "h{l.list}", class: "alike-col", title: "{l.name}", {l.name.chars().take(7).collect::<String>()} }
                }
                for (i , l) in all.iter().enumerate() {
                    span { key: "r{l.list}", class: "alike-row", title: "{l.name}", {l.name.chars().take(18).collect::<String>()} }
                    for (j , v) in dot[i].iter().enumerate() {
                        span {
                            key: "{i}-{j}",
                            class: "alike-cell",
                            title: "{all[i].name} · {all[j].name}: {v:.3}",
                            style: "background: hsl({anend_core::spread::seed_hue((v * 360.0) as i64)} 60% 55% / {0.15 + 0.75 * v.clamp(0.0, 1.0):.2});",
                            {format!("{v:.2}")}
                        }
                    }
                }
            }
        }
    }
}

/// a list as the Bevy app showed it: its name and key, choose and unlist, its figures, the
/// list read as one text, its letters as bars, and its members as chips with their sums
#[component]
fn ListCard(list: ListInfo, is_chosen: bool, on_choose: EventHandler<Hit>, on_open: EventHandler<String>) -> Element {
    let ui = ui();
    let api = api();
    let mut armed = use_signal(|| false);
    let joined: String = list
        .members
        .iter()
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let m = gem::Measure::of(&joined);
    let max = list.vector.iter().cloned().fold(0.0f64, f64::max).max(1e-9);
    let (k1, k2, n1) = (list.list.clone(), list.list.clone(), list.name.clone());
    let api_u = api.clone();
    let members_keys: Vec<String> = list.members.iter().map(|h| h.key.clone()).collect();
    rsx! {
        div { class: if is_chosen { "card listcard chosen" } else { "card listcard" },
            div { class: "row",
                button { class: "plain lname", "data-tip": "its page: rename, edit members, pictures", onclick: move |_| on_open.call(k1.clone()), h3 { "{list.name}" } }
                span { class: "dim small mono", "{list.list}" }
                span { class: "grow" }
                button { class: if is_chosen { "primary" } else { "" }, "data-tip": "the bar's Enter adds to it",
                    onclick: move |_| on_choose.call(Hit { key: k2.clone(), text: n1.clone(), note: false }),
                    if is_chosen { "chosen" } else { "choose" } }
                button {
                    class: if armed() { "armed" } else { "plain" },
                    "data-tip": "unlist: the list goes, its members stay (click twice)",
                    onclick: move |_| {
                        if !armed() { armed.set(true); return; }
                        let (api, key, members) = (api_u.clone(), list.list.clone(), members_keys.clone());
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.unlist(&key).await {
                                Ok(n) => { ui.did(Did::Unlisted { list: key, members }); ui.say(format!("unlisted {n} members · Ctrl+Z brings it back")); ui.touched(); }
                                Err(e) => ui.fail(e),
                            }
                        });
                    },
                    Icon { name: "delete" }
                    if armed() { "unlist?" } else { "unlist" }
                }
            }
            div { class: "figures-row",
                div { b { "{list.count}" } span { "members" } }
                div { b { "{list.ordinal_sum}" } span { "ordinal" } }
                div { b { {format!("{:.1}", list.ordinal_mean)} } span { "ordinal mean" } }
                div { b { "{list.reversed_sum}" } span { "reversed" } }
                div { b { {format!("{:.1}", list.reversed_mean)} } span { "reversed mean" } }
            }
            div { class: "small", "as one: ordinal {m.ordinal} · reversed {m.reversed} · letters " span { class: "mono", {gem::letters(&m.frequency).chars().take(40).collect::<String>()} } }
            div { class: "dim small mono", {gem::gsum_line(&joined)} }
            if m.ordinal > 1 {
                crate::views::memory::PfLine { n: m.ordinal }
            }
            if !list.vector.is_empty() {
                div { class: "letterbars", title: "its letters as a vector",
                    for (i , v) in list.vector.iter().enumerate() {
                        div { key: "{i}", class: "col",
                            span { class: "bar", style: "height: {(v / max * 34.0).max(1.0):.1}px;" }
                            span { class: "l", {((b'a' + i as u8) as char).to_string()} }
                        }
                    }
                }
            }
            div { class: "memberchips",
                for h in list.members.iter().cloned() {
                    MemberChip { key: "{h.key}", list: list.list.clone(), hit: h }
                }
            }
        }
    }
}

#[component]
fn MemberChip(list: String, hit: Hit) -> Element {
    let ui = ui();
    let api = api();
    let (s, r) = gem::sums(&hit.text);
    rsx! {
        span { class: "mchip", title: "{hit.key}",
            span { class: "mt", onclick: { let t = hit.text.clone(); move |_| ui.set_bar(&t) }, "{hit.text}" }
            span { class: "ms", "{s}·{r}" }
            button { class: "plain mx", "aria-label": "take it out of the list", "data-tip": "take it out (it stays a text)",
                onclick: move |_| {
                    let (api, l, k) = (api.clone(), list.clone(), hit.key.clone());
                    dioxus::dioxus_core::spawn_forever(async move {
                        match api.delist(&l, &k).await {
                            Ok(()) => { ui.did(Did::Delisted { list: l, member: k }); ui.touched(); }
                            Err(e) => ui.fail(e),
                        }
                    });
                },
                "×"
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
            crate::views::pictures::Pictures { list: Some(list.name.clone()), names: vec![] }
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
