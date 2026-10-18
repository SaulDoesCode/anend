//! The memory view: weum's memory page (docs/weum-reference/memory.js) on a black field.
//! The results header (the sum and its pfR line) and the texts sit in a centre field
//! whose width is dragged at its edges and stays centered; the sidecar and the toggles
//! are on the strip over the bar.
//!
//! weum's behaviour, kept: the hover title is the key and `gsum(text, 1)`; click puts a
//! text in the bar (its sums in the sidecar), Ctrl+click makes it the list candidate,
//! Shift+click the list, double click puts its key in the bar, Ctrl+right click gsearches
//! ⌊√(ordinal·reversed)⌋; underlines by the opposite sum's group with seedCLR; toasts; `r`
//! toggles reversed; ← → gsearch the sidecar's first and last; Enter, Ctrl+Enter,
//! Shift+Enter and Delete as weum; Enlist, List, Delist, Unlist. Added: Alt+click selects
//! (then a list is picked visually, or a tag typed), the right-click menu, the left-edge
//! strip of sum buckets, the triangle, spacing, and the bar's undo.

use dioxus::prelude::*;

use crate::backend::{BucketCount, Hit};
use crate::gem::{self, Mode};
use crate::shell::set_pref;
use crate::ui::*;
use crate::views::{TChip, use_keys};

#[derive(Clone, PartialEq, Default)]
struct Header {
    /// the sum (or what was searched)
    title: String,
    /// the number pfR runs on
    n: Option<u64>,
}

#[derive(Clone, Copy)]
struct Mem {
    results: Signal<Vec<Hit>>,
    header: Signal<Option<Header>>,
    hover: Signal<String>,
    picking: Signal<bool>,
}

/// weum's `envale(v)`: the header, then everything filed under `n`
fn envale(ui: Ui, api: Api, m: Mem, n: u64, mode: Mode) {
    let mode = if mode == Mode::Frequency { Mode::Ordinal } else { mode };
    show_sum(ui, m, n.to_string(), Some(n), None);
    dioxus::dioxus_core::spawn_forever(async move {
        match api.gsearch(mode, &n.to_string()).await {
            Ok(hits) => {
                set_topline(
                    ui,
                    &n.to_string(),
                    mode,
                    hits.first().map(|h| h.text.as_str()),
                    hits.len(),
                );
                let mut r = m.results;
                r.set(hits);
            }
            Err(e) => ui.fail(e),
        }
    });
}

fn show_sum(_ui: Ui, mut m: Mem, title: String, n: Option<u64>, keep: Option<Vec<Hit>>) {
    m.header.set(Some(Header { title, n }));
    m.results.set(keep.unwrap_or_default());
}

/// the search line in the top bar: what was searched, and the triangle of `text`
fn set_topline(mut ui: Ui, title: &str, mode: Mode, text: Option<&str>, count: usize) {
    let (s, r) = text.map(gem::sums).unwrap_or((0, 0));
    ui.topline.set(Some(TopLine {
        title: title.to_string(),
        detail: format!("{} · {count} text{}", mode.name(), if count == 1 { "" } else { "s" }),
        s,
        r,
    }));
}

fn add_hit(mut m: Mem, h: Hit) {
    let mut r = m.results.write();
    if !r.iter().any(|x| x.key == h.key) {
        r.push(h);
    }
}

/// the list commands typed in the bar
async fn list_command(ui: Ui, api: Api, m: Mem, word: &str) -> bool {
    if !["Enlist", "List", "Delist", "Unlist"].contains(&word) {
        return false;
    }
    let p = ui.picks.peek().clone();
    let Some(list) = p.list else {
        ui.fail("Shift+click a text to pick the list first");
        return true;
    };
    let r = match word {
        "Enlist" | "Delist" => {
            let Some(c) = p.candidate else {
                ui.fail("Ctrl+click a text to pick the candidate first");
                return true;
            };
            if word == "Enlist" {
                api.enlist(&list.key, &c.key).await.map(|_| {
                    ui.did(Did::Enlisted {
                        list: list.key.clone(),
                        member: c.key.clone(),
                    });
                    format!("“{}” is in the list “{}”", c.text, list.text)
                })
            } else {
                api.delist(&list.key, &c.key).await.map(|_| {
                    ui.did(Did::Delisted {
                        list: list.key.clone(),
                        member: c.key.clone(),
                    });
                    format!("“{}” left the list “{}”", c.text, list.text)
                })
            }
        }
        "Unlist" => {
            let members: Vec<String> = match api.lists().await {
                Ok(l) => l
                    .lists
                    .into_iter()
                    .find(|x| x.list == list.key)
                    .map(|x| x.members.into_iter().map(|h| h.key).collect())
                    .unwrap_or_default(),
                Err(_) => vec![],
            };
            api.unlist(&list.key).await.map(|n| {
                ui.did(Did::Unlisted {
                    list: list.key.clone(),
                    members,
                });
                let mut picks = ui.picks;
                picks.write().list = None;
                format!("the list “{}” is gone ({n} members)", list.text)
            })
        }
        _ => Ok(String::new()),
    };
    match r {
        Ok(msg) => {
            if !msg.is_empty() {
                ui.say(msg);
            }
            ui.touched();
            // List (and after a change): the list and its members
            if let Ok(l) = api.lists().await {
                let members = l
                    .lists
                    .into_iter()
                    .find(|x| x.list == list.key)
                    .map(|x| x.members)
                    .unwrap_or_default();
                show_sum(
                    ui,
                    m,
                    list.text.clone(),
                    None,
                    Some([vec![list.clone()], members].concat()),
                );
            }
        }
        Err(e) => ui.fail(e),
    }
    true
}

/// Enter in the bar (weum's term onkeydown)
async fn enter(mut ui: Ui, api: Api, m: Mem, raw: String, ctrl: bool, shift: bool) {
    let t = raw.trim().to_string();
    if t.is_empty() {
        return;
    }
    let mode = ui.mode();
    let (gsearch_text, saturate) = {
        let p = ui.prefs.peek();
        (p.gsearch_text, p.saturate)
    };
    // the right-click menu's edit and tag
    let pending = ui.pending.peek().clone();
    match pending {
        Pending::Edit(h) => {
            ui.pending.set(Pending::None);
            match api.put_memory(&h.key, &t).await {
                Ok(()) => {
                    ui.did(Did::Edited {
                        key: h.key.clone(),
                        old: h.text.clone(),
                    });
                    let mut r = m.results;
                    for x in r.write().iter_mut().filter(|x| x.key == h.key) {
                        x.text = t.clone();
                    }
                    ui.say(format!("{} now reads “{t}” · Ctrl+Z takes it back", h.key));
                    ui.touched();
                }
                Err(e) => ui.fail(e),
            }
            return;
        }
        Pending::Tag(hs) => {
            ui.pending.set(Pending::None);
            let mut done = 0;
            for h in &hs {
                match api.tag(&h.key, &t).await {
                    Ok(()) => done += 1,
                    Err(e) => ui.fail(e),
                }
            }
            ui.say(format!("tagged {done} with “{t}”"));
            ui.set_bar("");
            ui.touched();
            return;
        }
        Pending::None => {}
    }
    if list_command(ui, api.clone(), m, &t).await {
        return;
    }
    let number: Option<u64> = t.parse().ok();
    if shift {
        // remember by key
        match api.remember(&t).await {
            Ok(h) => show_sum(ui, m, t.clone(), None, Some(vec![h])),
            Err(e) => ui.fail(e),
        }
        return;
    }
    if let Some(n) = number {
        envale(ui, api, m, n, mode);
        return;
    }
    if mode == Mode::Frequency && !ctrl {
        // anagrams: the same letters
        let letters = gem::letters(&gem::Measure::of(&t).frequency);
        show_sum(ui, m, letters.clone(), None, None);
        match api.gsearch(Mode::Frequency, &t).await {
            Ok(hits) => {
                set_topline(ui, &letters, mode, Some(&t), hits.len());
                let mut r = m.results;
                r.set(hits);
            }
            Err(e) => ui.fail(e),
        }
        if saturate {
            saturate_and_show(ui, api, m, &t, false).await;
        }
        return;
    }
    let s = gem::sum_in(&t, mode);
    if ctrl {
        // weum: gsearch the text's sum, saturate it, toast its key
        envale(ui, api.clone(), m, s, mode);
        saturate_and_show(ui, api, m, &t, true).await;
        return;
    }
    if gsearch_text {
        envale(ui, api.clone(), m, s, mode);
        set_topline(ui, &s.to_string(), mode, Some(&t), 0);
        // and the words: the text itself, when it's there
        match api.search(&t, true).await {
            Ok(found) => {
                if let Some(h) = found.into_iter().find(|h| h.text == t) {
                    add_hit(m, h);
                } else if saturate {
                    saturate_and_show(ui, api, m, &t, true).await;
                }
            }
            Err(e) => ui.fail(e),
        }
    } else {
        // weum's tryfulSearchVariations: the words, with variations
        show_sum(ui, m, t.clone(), None, None);
        match api.search(&t, true).await {
            Ok(found) => {
                let present = found.iter().any(|h| h.text == t);
                set_topline(ui, &t, mode, Some(&t), found.len());
                let mut r = m.results;
                r.set(found);
                if !present && saturate {
                    saturate_and_show(ui, api, m, &t, true).await;
                }
            }
            Err(e) => ui.fail(e),
        }
    }
}

async fn saturate_and_show(ui: Ui, api: Api, m: Mem, t: &str, toast: bool) {
    match api.saturate(t).await {
        Ok((key, new)) => {
            if new {
                ui.did(Did::Remembered {
                    key: key.clone(),
                    text: t.to_string(),
                });
                ui.touched();
            }
            if toast || new {
                ui.toast(key.clone());
            }
            add_hit(
                m,
                Hit {
                    key,
                    text: t.to_string(),
                    note: false,
                },
            );
        }
        Err(e) => ui.fail(e),
    }
}

async fn forget(ui: Ui, api: Api, m: Mem, key: String) {
    match api.forget(&key).await {
        Ok(text) => {
            ui.did(Did::Forgot {
                key: key.clone(),
                text: text.clone(),
            });
            ui.say(format!("forgot “{text}” ({key}) · Ctrl+Z takes it back"));
            let mut r = m.results;
            r.write().retain(|h| h.key != key);
            ui.set_bar("");
            ui.touched();
        }
        Err(e) => ui.fail(e),
    }
}

fn set_mode(ui: Ui, mode: Mode) {
    set_pref(ui, |p| p.mode = mode);
}

#[component]
pub fn Memory() -> Element {
    let mut ui = ui();
    let api = api();
    let m = Mem {
        results: use_signal(Vec::new),
        header: use_signal(|| None),
        hover: use_signal(String::new),
        picking: use_signal(|| false),
    };
    use_context_provider(|| m);

    // the keys (weum's window.onkeydown, and the bar's)
    {
        let api = api.clone();
        use_keys(View::Memory, move |k: KeyPress| {
            if k.bar {
                match k.key.as_str() {
                    "Enter" => {
                        dioxus::dioxus_core::spawn_forever(enter(ui, api.clone(), m, k.text.clone(), k.ctrl, k.shift));
                        true
                    }
                    "Delete" => {
                        dioxus::dioxus_core::spawn_forever(forget(ui, api.clone(), m, k.text.trim().to_string()));
                        true
                    }
                    _ => false,
                }
            } else {
                let measure = ui.measure.peek().clone();
                let mode = ui.mode();
                match k.key.as_str() {
                    "r" | "R" => set_mode(
                        ui,
                        if mode == Mode::Reversed {
                            Mode::Ordinal
                        } else {
                            Mode::Reversed
                        },
                    ),
                    "o" | "O" => set_mode(ui, Mode::Ordinal),
                    "f" | "F" => set_mode(
                        ui,
                        if mode == Mode::Frequency {
                            Mode::Ordinal
                        } else {
                            Mode::Frequency
                        },
                    ),
                    "g" | "G" => set_pref(ui, |p| p.gsearch_text = !p.gsearch_text),
                    "s" | "S" => set_pref(ui, |p| p.saturate = !p.saturate),
                    "l" | "L" => set_pref(ui, |p| p.letters = !p.letters),
                    "ArrowLeft" => match measure {
                        // the sidecar's first: the ordinal sum, gsearched in ordinal
                        Some(me) => {
                            set_mode(ui, Mode::Ordinal);
                            envale(ui, api.clone(), m, me.ordinal, Mode::Ordinal);
                        }
                        None => return false,
                    },
                    "ArrowRight" => match measure {
                        // the last: the reversed sum, gsearched in reversed
                        Some(me) => {
                            set_mode(ui, Mode::Reversed);
                            envale(ui, api.clone(), m, me.reversed, Mode::Reversed);
                        }
                        None => return false,
                    },
                    "Escape" => {
                        let mut p = m.picking;
                        p.set(false);
                        ui.picks.write().selected.clear();
                    }
                    _ => return false,
                }
                true
            }
        });
    }

    let results = (m.results)();
    let header = (m.header)();
    let mode = ui.prefs.read().mode;
    let width = ui.prefs.read().field_width;
    let picks = (ui.picks)();
    let field_style = if width > 0.0 {
        format!("max-width: {width}px;")
    } else {
        String::new()
    };
    let nsel = picks.selected.len();
    rsx! {
        div { class: "memory", style: "position: absolute; inset: 0;",
            div { class: "edge" }
            Buckets {}
            div { class: "mem-main",
                if header.is_none() && results.is_empty() {
                    div { class: "mem-intro",
                        p { "Type a word below and press Enter: it is remembered, and everything sharing its sum comes up." }
                        p { "A number gsearches it; Shift+Enter brings back a key (like 0,12). Shift+click a text to head a list, Ctrl+click a candidate, Alt+click to select." }
                    }
                } else {
                    div { class: "spacer" }
                    Field { style: field_style, header, results, mode, picks: picks.clone() }
                    div { class: "spacer" }
                }
            }
            if nsel > 0 {
                SelBar { n: nsel }
            }
            div { class: "mem-hover", "{m.hover}" }
        }
    }
}

/// the lists, and the picked list (Shift+click, or a new one) even while it's empty
fn with_picked(mut lists: Vec<crate::backend::ListInfo>, picked: Option<Hit>) -> Vec<crate::backend::ListInfo> {
    if let Some(h) = picked
        && !lists.iter().any(|l| l.list == h.key)
    {
        lists.insert(
            0,
            crate::backend::ListInfo {
                list: h.key,
                name: h.text,
                ..Default::default()
            },
        );
    }
    lists
}

#[component]
fn SelBar(n: usize) -> Element {
    let mut ui = ui();
    let api = api();
    let m = use_context::<Mem>();
    let mut picking = m.picking;
    let api_r = api.clone();
    let lists = use_resource(move || {
        let api = api_r.clone();
        let open = picking();
        async move { if open { api.lists().await.ok() } else { None } }
    });
    rsx! {
        div { class: "selbar", role: "toolbar",
            "{n} selected"
            button { onclick: move |_| picking.set(!picking()), Icon { name: "list" } "put in a list…" }
            button {
                onclick: move |_| {
                    let sel = ui.picks.peek().selected.clone();
                    ui.pending.set(Pending::Tag(sel));
                    ui.set_bar("");
                    ui.focus_bar();
                },
                Icon { name: "tag" }
                "tag…"
            }
            button { onclick: move |_| ui.picks.write().selected.clear(), Icon { name: "close" } "clear" }
        }
        if picking() {
            div { class: "listpick",
                match &*lists.read() {
                    Some(Some(l)) if !l.lists.is_empty() || ui.picks.read().list.is_some() => rsx! {
                        for li in with_picked(l.lists.clone(), ui.picks.read().list.clone()) {
                            button {
                                key: "{li.list}",
                                title: "{li.count} members · {li.list}",
                                onclick: {
                                    let api = api.clone();
                                    let li = li.clone();
                                    move |_| {
                                        let api = api.clone();
                                        let li = li.clone();
                                        dioxus::dioxus_core::spawn_forever(async move {
                                            let sel = ui.picks.peek().selected.clone();
                                            for h in &sel {
                                                match api.enlist(&li.list, &h.key).await {
                                                    Ok(()) => ui.did(Did::Enlisted { list: li.list.clone(), member: h.key.clone() }),
                                                    Err(e) => ui.fail(e),
                                                }
                                            }
                                            ui.say(format!("{} into the list “{}”", sel.len(), li.name));
                                            ui.picks.write().selected.clear();
                                            let mut p = m.picking;
                                            p.set(false);
                                            ui.touched();
                                        });
                                    }
                                },
                                "{li.name} ({li.count})"
                            }
                        }
                    },
                    Some(Some(_)) => rsx! { span { "no lists yet: Shift+click a text to make one its head" } },
                    _ => rsx! { span { "…" } },
                }
            }
        }
    }
}

#[component]
fn Field(style: String, header: Option<Header>, results: Vec<Hit>, mode: Mode, picks: Picks) -> Element {
    let ui = ui();
    let mut drag: Signal<Option<(f64, f64, f64)>> = use_signal(|| None);
    let mut rect: Signal<Option<std::rc::Rc<MountedData>>> = use_signal(|| None);
    let start = move |e: MouseEvent, side: f64| {
        e.prevent_default();
        let x = e.client_coordinates().x;
        dioxus::dioxus_core::spawn_forever(async move {
            let w = match rect.peek().clone() {
                Some(r) => r.get_client_rect().await.map(|r| r.width()).unwrap_or(800.0),
                None => 800.0,
            };
            drag.set(Some((x, w, side)));
        });
    };
    rsx! {
        div {
            class: "field",
            style: "{style}",
            onmounted: move |e| rect.set(Some(e.data())),
            div { class: "grip l", title: "drag: the field's width (it stays centered); double click: the whole width",
                onmousedown: move |e| start(e, -1.0),
                ondoubleclick: move |_| set_pref(ui, |p| p.field_width = 0.0) }
            div { class: "grip r", title: "drag: the field's width (it stays centered); double click: the whole width",
                onmousedown: move |e| start(e, 1.0),
                ondoubleclick: move |_| set_pref(ui, |p| p.field_width = 0.0) }
            if let Some(h) = header {
                div { class: "results-head",
                    h3 { "{h.title}" }
                    if let Some(n) = h.n {
                        PfLine { n }
                    }
                }
            }
            div { class: "arts", role: "list",
                if results.is_empty() {
                    span { class: "dim", style: "color: #bbb; padding: 6px;", "nothing filed here yet" }
                }
                for h in results {
                    Art {
                        key: "{h.key}",
                        under: gem::underline(&h.text, mode),
                        is_list: picks.list.as_ref().is_some_and(|l| l.key == h.key),
                        is_cand: picks.candidate.as_ref().is_some_and(|l| l.key == h.key),
                        selected: picks.selected.iter().any(|s| s.key == h.key),
                        hit: h,
                    }
                }
            }
        }
        if let Some((x0, w0, side)) = drag() {
            div {
                style: "position: fixed; inset: 0; z-index: 90; cursor: ew-resize;",
                onmousemove: move |e| {
                    let x = e.client_coordinates().x;
                    let w = (w0 + 2.0 * side * (x - x0)).max(240.0);
                    set_pref(ui, |p| p.field_width = w as f32);
                },
                onmouseup: move |_| drag.set(None),
            }
        }
    }
}

/// weum's pfR line; its numbers go into the bar when clicked
#[component]
fn PfLine(n: u64) -> Element {
    let ui = ui();
    let segs = gem::pf_line(n);
    rsx! {
        div { class: "pf",
            for (i , s) in segs.into_iter().enumerate() {
                match s {
                    anend_core::weum::Seg::Text(t) => rsx! { span { key: "{i}", "{t}" } },
                    anend_core::weum::Seg::Num(v) => rsx! {
                        i { key: "{i}", title: "into the bar", onclick: move |_| ui.set_bar(&v.to_string()), "{v}" }
                    },
                }
            }
        }
    }
}

#[component]
fn Art(hit: Hit, under: Option<String>, is_list: bool, is_cand: bool, selected: bool) -> Element {
    let mut ui = ui();
    let api = api();
    let m = use_context::<Mem>();
    let title = format!("{} {}", hit.key, gem::gsum_line(&hit.text));
    let long = hit.text.chars().count() > 220;
    let mut class = String::from("art");
    for (on, c) in [
        (is_list, " is-list"),
        (is_cand, " is-candidate"),
        (selected, " selected"),
        (hit.note, " note"),
        (long, " long"),
    ] {
        if on {
            class.push_str(c);
        }
    }
    let style = match &under {
        Some(c) => format!("text-decoration: underline; text-decoration-color: {c};"),
        None => String::new(),
    };
    let (h1, h2, h3, h4) = (hit.clone(), hit.clone(), hit.clone(), hit.clone());
    let t2 = title.clone();
    let mut hover = m.hover;
    rsx! {
        div {
            class: "{class}",
            style: "{style}",
            role: "listitem",
            tabindex: "0",
            title: "{title}",
            onmouseenter: move |_| hover.set(t2.clone()),
            onclick: move |e: MouseEvent| {
                let md = e.modifiers();
                if md.ctrl() || md.meta() {
                    ui.picks.write().candidate = Some(h1.clone());
                    ui.say(format!("candidate: {}", h1.text));
                } else if md.shift() {
                    let mut p = ui.picks.write();
                    p.candidate = None;
                    p.list = Some(h1.clone());
                    drop(p);
                    ui.say(format!("list: {}", h1.text));
                } else if md.alt() {
                    let mut p = ui.picks.write();
                    if let Some(i) = p.selected.iter().position(|x| x.key == h1.key) {
                        p.selected.remove(i);
                    } else {
                        p.selected.push(h1.clone());
                    }
                } else {
                    ui.set_bar(&h1.text);
                    ui.focus_bar();
                }
            },
            ondoubleclick: move |_| ui.set_bar(&h2.key),
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Enter {
                    ui.set_bar(&h4.text);
                    ui.focus_bar();
                }
            },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                if e.modifiers().ctrl() {
                    // weum: gsearch ⌊√(s·r)⌋ in the current mode
                    let me = gem::Measure::of(&h3.text);
                    envale(ui, api.clone(), m, me.triangle(), ui.mode());
                } else {
                    let p = e.client_coordinates();
                    ui.menu.set(Some(Menu { hit: h3.clone(), x: p.x, y: p.y }));
                }
            },
            "{hit.text}"
        }
    }
}

/// the left edge: a circle per sum bucket, sized by how many texts it holds
#[component]
fn Buckets() -> Element {
    let ui = ui();
    let api = api();
    let m = use_context::<Mem>();
    let mode = ui.prefs.read().mode;
    let counts = use_resource(move || {
        let api = api.clone();
        let _ = (ui.changed)();
        let mode = ui.prefs.read().mode;
        async move {
            api.bucket_counts(if mode == Mode::Frequency { Mode::Ordinal } else { mode })
                .await
                .unwrap_or_default()
        }
    });
    let all: Vec<BucketCount> = counts.read().clone().unwrap_or_default();
    let max = all.iter().map(|b| b.count).max().unwrap_or(1).max(1) as f64;
    let shown_mode = if mode == Mode::Frequency { Mode::Ordinal } else { mode };
    let on = (m.header)().and_then(|h| h.n);
    let api2 = use_context::<Api>();
    rsx! {
        nav { class: "buckets", "aria-label": "sum buckets",
            div { class: "line" }
            for b in all.into_iter().take(4000) {
                div {
                    key: "{b.sum}",
                    class: if on == Some(b.sum as u64) { "b on" } else { "b" },
                    style: "--r: {2.0 + 6.0 * (b.count as f64 / max).sqrt():.1}px;",
                    title: format!("{} {} · {} text{}", shown_mode.name(), b.sum, b.count, if b.count == 1 { "" } else { "s" }),
                    onclick: {
                        let api = api2.clone();
                        move |_| envale(ui, api.clone(), m, b.sum.max(0) as u64, shown_mode)
                    },
                    span { class: "dot" }
                    "{b.sum}"
                }
            }
        }
    }
}

/// the strip over the bar: the sidecar (the bar's sums) and the toggles
#[component]
pub fn MemoryStrip() -> Element {
    let ui = ui();
    let measure = (ui.measure)();
    let p = ui.prefs.read().clone();
    rsx! {
        if let Some(me) = measure {
            span { class: "sidecar", "aria-label": "the bar's sums",
                "("
                for (i , (v , md)) in [(me.ordinal, Mode::Ordinal), (me.reversed, Mode::Reversed)].into_iter().enumerate() {
                    if i > 0 { ", " }
                    span {
                        class: if p.mode == md { "num on" } else { "num" },
                        title: format!("gsearch {} {v} ({})", md.name(), if i == 0 { "←" } else { "→" }),
                        onclick: move |_| {
                            // as ← and → do
                            let key = if md == Mode::Ordinal { "ArrowLeft" } else { "ArrowRight" };
                            let h = ui.keys.peek().iter().find(|(w, _)| *w == View::Memory).map(|(_, h)| *h);
                            if let Some(h) = h {
                                h.call(KeyPress { key: key.into(), code: String::new(), ctrl: false, shift: false, alt: false, bar: false, text: String::new() });
                            }
                        },
                        "{v}"
                    }
                }
                ")"
                span { class: "extra", title: "the byte sum (bytes 1–256)", "· b {me.bytes}" }
                if p.letters {
                    span { class: "letters", title: "the letters (frequency); L hides them", "{gem::letters(&me.frequency)}" }
                }
            }
        }
        span { class: "toggles", role: "group", "aria-label": "mode",
            for md in Mode::ALL {
                TChip {
                    key: "{md.name()}",
                    label: format!("{} · {}", md.name(), match md { Mode::Ordinal => "O", Mode::Reversed => "R toggles", Mode::Frequency => "F" }),
                    letter: md.letter(),
                    on: p.mode == md,
                    onclick: move |_| set_mode(ui, md),
                }
            }
        }
        span { class: "toggles",
            TChip { label: "gsearch text: by its sum, not its words · G", icon: "search", on: p.gsearch_text, onclick: move |_| set_pref(ui, |p| p.gsearch_text = !p.gsearch_text) }
            TChip { label: "saturate: remember what's searched · S", icon: "bolt", on: p.saturate, onclick: move |_| set_pref(ui, |p| p.saturate = !p.saturate) }
            TChip { label: "letters in the sidecar · L", letter: "abc", on: p.letters, onclick: move |_| set_pref(ui, |p| p.letters = !p.letters) }
        }
    }
}
