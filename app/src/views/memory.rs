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
use crate::keys::Action;
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
    /// the text under the pointer, for its words' types under the hover line
    hover_text: Signal<String>,
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

/// anagrams: everything with the same letters as `t`
async fn letters_search(ui: Ui, api: Api, m: Mem, t: String) {
    let letters = gem::letters(&gem::Measure::of(&t).frequency);
    show_sum(ui, m, letters.clone(), None, None);
    match api.gsearch(Mode::Frequency, &t).await {
        Ok(hits) => {
            set_topline(ui, &letters, Mode::Frequency, Some(&t), hits.len());
            let mut r = m.results;
            r.set(hits);
        }
        Err(e) => ui.fail(e),
    }
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
        // a space placement's own text, left pending when the view changed: dropped
        Pending::EditOwn { .. } => ui.pending.set(Pending::None),
        Pending::None => {}
    }
    if list_command(ui, api.clone(), m, &t).await {
        return;
    }
    let (tint, cls_search) = {
        let p = ui.prefs.peek();
        (p.tint.clone(), p.cls_search)
    };
    if cls_search && !tint.is_empty() && !shift {
        classifier_search(ui, api, m, &tint, &t).await;
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
        letters_search(ui, api.clone(), m, t.clone()).await;
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

/// the classifier search mode: a class's name (and words after it) finds the texts most
/// likely in that class; any other text, the texts the classifier reads most like it
async fn classifier_search(ui: Ui, api: Api, m: Mem, name: &str, t: &str) {
    let classes: Vec<String> = match api.call("classify.get", serde_json::json!({ "name": name })).await {
        Ok(v) => v["classes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str().map(str::to_string))
            .collect(),
        Err(e) => return ui.fail(e),
    };
    let lower = t.to_lowercase();
    let class = classes.iter().find(|c| {
        let c = c.to_lowercase();
        lower == c || lower.starts_with(&format!("{c} "))
    });
    let mode = ui.mode();
    match class {
        Some(c) => {
            let words = t[c.len()..].trim().to_string();
            let title = if words.is_empty() {
                format!("{name}: {c}")
            } else {
                format!("{name}: {c} · {words}")
            };
            show_sum(ui, m, title.clone(), None, None);
            match api.model_search(name, c, &words).await {
                Ok(found) => {
                    // the texts it counts in the class; when there are none, the likeliest
                    let members = found.iter().filter(|s| s.member).count();
                    if members == 0 {
                        ui.say(format!("{name} counts nothing here as {c}; these are the likeliest"));
                    }
                    let hits: Vec<Hit> = found
                        .into_iter()
                        .filter(|s| s.member || members == 0)
                        .take(if members == 0 { 24 } else { usize::MAX })
                        .filter_map(|s| {
                            Some(Hit {
                                key: s.key?,
                                text: s.text,
                                note: s.note,
                            })
                        })
                        .collect();
                    set_topline(ui, &title, mode, hits.first().map(|h| h.text.as_str()), hits.len());
                    let mut r = m.results;
                    r.set(hits);
                }
                Err(e) => ui.fail(e),
            }
        }
        None => {
            let title = format!("like “{t}” · {name}");
            show_sum(ui, m, title.clone(), None, None);
            match api
                .call(
                    "classify.similar",
                    serde_json::json!({ "name": name, "text": t, "limit": 60 }),
                )
                .await
            {
                Ok(v) => {
                    let hits: Vec<Hit> = v["results"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|h| {
                            Some(Hit {
                                key: h["key"].as_str()?.to_string(),
                                text: h["text"].as_str()?.to_string(),
                                note: h["note"].as_bool().unwrap_or(false),
                            })
                        })
                        .collect();
                    set_topline(ui, &title, mode, Some(t), hits.len());
                    let mut r = m.results;
                    r.set(hits);
                }
                Err(e) => ui.fail(e),
            }
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
        hover_text: use_signal(String::new),
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
                if k.key == "Escape" {
                    let mut p = m.picking;
                    p.set(false);
                    ui.picks.write().selected.clear();
                    return true;
                }
                match k.action {
                    Some(Action::BucketReversed) => set_mode(
                        ui,
                        if mode == Mode::Reversed {
                            Mode::Ordinal
                        } else {
                            Mode::Reversed
                        },
                    ),
                    Some(Action::BucketOrdinal) => set_mode(ui, Mode::Ordinal),
                    Some(Action::BucketFrequency) => set_mode(
                        ui,
                        if mode == Mode::Frequency {
                            Mode::Ordinal
                        } else {
                            Mode::Frequency
                        },
                    ),
                    Some(Action::ToggleGsearch) => set_pref(ui, |p| p.gsearch_text = !p.gsearch_text),
                    Some(Action::ToggleSaturate) => set_pref(ui, |p| p.saturate = !p.saturate),
                    Some(Action::Letters) => set_pref(ui, |p| p.letters = !p.letters),
                    Some(Action::WhiteMode) => set_pref(ui, |p| p.white = !p.white),
                    Some(Action::WordTypes) => set_pref(ui, |p| p.word_types = !p.word_types),
                    Some(Action::ClassifierSearch) if !ui.prefs.peek().tint.is_empty() => {
                        set_pref(ui, |p| p.cls_search = !p.cls_search)
                    }
                    Some(Action::SideOrdinal) => match measure {
                        // the sidecar's first: the ordinal sum, gsearched in ordinal
                        Some(me) => {
                            set_mode(ui, Mode::Ordinal);
                            envale(ui, api.clone(), m, me.ordinal, Mode::Ordinal);
                        }
                        None => return false,
                    },
                    Some(Action::SideReversed) => match measure {
                        // the last: the reversed sum, gsearched in reversed
                        Some(me) => {
                            set_mode(ui, Mode::Reversed);
                            envale(ui, api.clone(), m, me.reversed, Mode::Reversed);
                        }
                        None => return false,
                    },
                    Some(Action::SideFrequency) => match measure {
                        // the letters: gsearched by frequency
                        Some(me) => {
                            set_mode(ui, Mode::Frequency);
                            let _ = me;
                            let t = ui.bar.peek().trim().to_string();
                            dioxus::dioxus_core::spawn_forever(letters_search(ui, api.clone(), m, t));
                        }
                        None => return false,
                    },
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
    let (word_types_on, white) = {
        let p = ui.prefs.read();
        (p.word_types, p.white)
    };
    rsx! {
        div { class: if white { "memory white" } else { "memory" }, style: "position: absolute; inset: 0;",
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
            if !(m.hover_text)().is_empty() && !word_types_on {
                div { class: "mem-types", "aria-label": "its words' types", TaggedText { text: (m.hover_text)(), labels: true } }
            }
        }
    }
}

/// each word type's short label
pub(crate) fn type_label(t: &str) -> &'static str {
    match t {
        "noun" => "n",
        "verb" => "v",
        "adjective" => "adj",
        "adverb" => "adv",
        "pronoun" => "pron",
        "determiner" => "det",
        "preposition" => "prep",
        "conjunction" => "conj",
        "interjection" => "intj",
        "name" => "name",
        "number" => "num",
        "date" => "date",
        "time" => "time",
        "link" => "link",
        "email" => "email",
        "hashtag" => "tag",
        _ => "",
    }
}

/// a text with each word labelled by its type (core's `kinds::tag`), in that type's colour;
/// the spaces and punctuation between words kept as they are
#[component]
pub(crate) fn TaggedText(
    text: String,
    #[props(default = true)] labels: bool,
    /// as ruby: the words stay inline text, so a text's own underline (the gematria
    /// colour of its reversed-sum group) runs under them untouched; the type is the label
    /// under each word, in its colour, never an underline
    #[props(default)]
    ruby: bool,
) -> Element {
    let tags = anend_core::kinds::tag(&text);
    let mut parts: Vec<(String, Option<&'static str>)> = Vec::new();
    let mut at = 0;
    for t in &tags {
        if t.start > at {
            parts.push((text[at..t.start].to_string(), None));
        }
        parts.push((text[t.start..t.end].to_string(), Some(t.ty)));
        at = t.end;
    }
    if at < text.len() {
        parts.push((text[at..].to_string(), None));
    }
    rsx! {
        span { class: "tagged",
            for (i , (piece , ty)) in parts.into_iter().enumerate() {
                match ty {
                    Some(ty) if ruby => rsx! {
                        ruby { key: "{i}", class: "rw t-{ty}", title: "{ty}",
                            "{piece}"
                            rt { "{type_label(ty)}" }
                        }
                    },
                    Some(ty) => rsx! {
                        span { key: "{i}", class: "tw t-{ty}", title: "{ty}",
                            span { class: "w", "{piece}" }
                            if labels { span { class: "l", "{type_label(ty)}" } }
                        }
                    },
                    None => rsx! { span { key: "{i}", "{piece}" } },
                }
            }
        }
    }
}

/// the word types shown, each in its colour, with how many of each
#[component]
fn TypesLegend(texts: Vec<String>) -> Element {
    let mut counts: std::collections::BTreeMap<usize, (&'static str, usize)> = Default::default();
    for t in &texts {
        for w in anend_core::kinds::tag(t) {
            let i = anend_core::kinds::WORD_TYPES
                .iter()
                .position(|x| *x == w.ty)
                .unwrap_or(99);
            counts.entry(i).or_insert((w.ty, 0)).1 += 1;
        }
    }
    rsx! {
        div { class: "types-legend", "aria-label": "word types",
            for (_ , (ty , n)) in counts {
                span { key: "{ty}", class: "tw t-{ty}", b { "{type_label(ty)}" } span { "{ty} {n}" } }
            }
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
    let api = api();
    let keys: Vec<String> = results.iter().map(|h| h.key.clone()).collect();
    let keys = use_memo(use_reactive!(|keys| keys));
    let tints = use_resource(move || {
        let api = api.clone();
        let name = ui.prefs.read().tint.clone();
        let keys = keys();
        let _ = (ui.changed)();
        async move {
            if name.is_empty() || keys.is_empty() {
                return None;
            }
            let keys: Vec<String> = keys.into_iter().take(5000).collect();
            api.call("classify.tints", serde_json::json!({ "name": name, "keys": keys }))
                .await
                .ok()
        }
    });
    let tinted = tints.read().clone().flatten();
    let (tint_of, legend) = tint_map(tinted.as_ref());
    // the search filter (the square at the strip's right): what it keeps of the results
    let filter = ui.prefs.read().filter.clone();
    let filter_open = ui.prefs.read().filter_open;
    let class_of = class_map(tinted.as_ref());
    let api_f = use_context::<Api>();
    let judged = use_resource(move || {
        let api = api_f.clone();
        let name = ui.prefs.read().tint.clone();
        let want = ui.prefs.read().filter.feedback || ui.prefs.read().filter_open;
        let _ = (ui.changed)();
        async move {
            if name.is_empty() || !want {
                return None;
            }
            let d = api
                .call("classify.get", serde_json::json!({ "name": name }))
                .await
                .ok()?;
            let bias = api
                .call(
                    "classify.steer",
                    serde_json::json!({ "name": name, "steer": {}, "save": false }),
                )
                .await
                .ok()
                .map(|v| v["bias"].clone());
            let keys: std::collections::HashSet<String> = d["feedback"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| f["key"].as_str().map(str::to_string))
                .collect();
            Some((
                keys,
                bias.unwrap_or_default(),
                d["feedback_count"].as_u64().unwrap_or(0),
            ))
        }
    });
    let judged = judged.read().clone().flatten();
    let all_results = results.clone();
    let results: Vec<Hit> = results
        .into_iter()
        .filter(|h| {
            keeps(
                &filter,
                h,
                class_of.get(&h.key).map(String::as_str),
                judged.as_ref().map(|j| &j.0),
            )
        })
        .collect();
    let hidden = all_results.len() - results.len();
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
            if !legend.is_empty() {
                div { class: "tint-legend", "aria-label": "colours",
                    for (c , colour , n) in legend.clone() {
                        span { key: "{c}", span { class: "dot", style: "background: {colour};" } "{c} {n}" }
                    }
                }
            }
            if hidden > 0 {
                div { class: "filter-note", "the filter hides {hidden} of {all_results.len()} · the square at the right of the strip changes it" }
            }
            if ui.prefs.read().word_types && !results.is_empty() {
                TypesLegend { texts: results.iter().map(|h| h.text.clone()).collect::<Vec<_>>() }
            }
            div { class: "arts", role: "list",
                if results.is_empty() && hidden == 0 {
                    span { class: "dim", style: "color: #bbb; padding: 6px;", "nothing filed here yet" }
                }
                for h in results {
                    Art {
                        key: "{h.key}",
                        tint: tint_of.get(&h.key).cloned(),
                        under: gem::underline(&h.text, mode),
                        is_list: picks.list.as_ref().is_some_and(|l| l.key == h.key),
                        is_cand: picks.candidate.as_ref().is_some_and(|l| l.key == h.key),
                        selected: picks.selected.iter().any(|s| s.key == h.key),
                        hit: h,
                    }
                }
            }
        }
        if filter_open {
            FilterPanel { shown: all_results.clone(), class_of: class_of.clone(), legend: legend.clone(), judged: judged.clone() }
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

/// each key's class name under the colouring classifier ("unsure" when in none)
fn class_map(v: Option<&serde_json::Value>) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let Some(v) = v else { return out };
    let names: Vec<String> = crate::views::classifiers::strs(&v["classes"]);
    for t in v["tints"].as_array().into_iter().flatten() {
        let Some(key) = t["key"].as_str() else { continue };
        let c = t["class"]
            .as_u64()
            .and_then(|c| names.get(c as usize).cloned())
            .unwrap_or_else(|| "unsure".into());
        out.insert(key.to_string(), c);
    }
    out
}

/// whether the search filter keeps a text
fn keeps(
    f: &crate::prefs::MemFilter,
    h: &Hit,
    class: Option<&str>,
    judged: Option<&std::collections::HashSet<String>>,
) -> bool {
    if (h.note && f.no_notes) || (!h.note && f.no_texts) {
        return false;
    }
    if !f.kinds.is_empty() && !f.kinds.contains(anend_core::kinds::detect(&h.text)) {
        return false;
    }
    if !f.words.is_empty() {
        let types = anend_core::kinds::word_types(&h.text);
        if !f.words.iter().all(|w| types.iter().any(|(_, k)| k == w)) {
            return false;
        }
    }
    if !f.classes.is_empty() && class.is_some_and(|c| !f.classes.contains(c)) {
        return false;
    }
    if f.feedback && judged.is_some_and(|j| !j.contains(&h.key)) {
        return false;
    }
    true
}

/// counts of a thing among the shown texts, most first
fn tally<'a>(items: impl Iterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut m: std::collections::BTreeMap<&str, usize> = Default::default();
    for i in items {
        *m.entry(i).or_default() += 1;
    }
    let mut v: Vec<(String, usize)> = m.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// the search filter panel: what the shown texts are (their kinds, their words' types,
/// their classes under the colouring classifier, which were given a verdict, the
/// classifier's bias), each a chip that keeps only those
#[component]
fn FilterPanel(
    shown: Vec<Hit>,
    class_of: std::collections::HashMap<String, String>,
    legend: Legend,
    judged: Option<(std::collections::HashSet<String>, serde_json::Value, u64)>,
) -> Element {
    let ui = ui();
    let f = ui.prefs.read().filter.clone();
    let tint = ui.prefs.read().tint.clone();
    let kinds = tally(shown.iter().map(|h| anend_core::kinds::detect(&h.text)));
    let words = tally(
        shown
            .iter()
            .flat_map(|h| anend_core::kinds::word_types(&h.text))
            .map(|(_, k)| k)
            .collect::<Vec<_>>()
            .into_iter(),
    );
    let notes = shown.iter().filter(|h| h.note).count();
    let classes: Vec<(String, usize)> = legend.iter().map(|(c, _, n)| (c.clone(), *n)).collect();
    let flip = move |set: fn(&mut crate::prefs::MemFilter) -> &mut std::collections::BTreeSet<String>, v: String| {
        set_pref(ui, move |p| {
            let s = set(&mut p.filter);
            if !s.remove(&v) {
                s.insert(v.clone());
            }
        })
    };
    let _ = &class_of;
    rsx! {
        div { class: "filter-panel", role: "dialog", "aria-label": "search filter",
            div { class: "row",
                b { "filter what's shown" }
                span { class: "grow" }
                if f.active() {
                    button { class: "plain", onclick: move |_| set_pref(ui, |p| p.filter = Default::default()), "clear" }
                }
                button { class: "plain", "aria-label": "close the filter", onclick: move |_| set_pref(ui, |p| p.filter_open = false), Icon { name: "close" } }
            }
            div { class: "fgroup",
                span { class: "dim small", "entries" }
                button { class: if !f.no_texts { "chip on" } else { "chip" }, onclick: move |_| set_pref(ui, |p| p.filter.no_texts = !p.filter.no_texts),
                    "texts " span { class: "dim", "{shown.len() - notes}" } }
                button { class: if !f.no_notes { "chip on" } else { "chip" }, onclick: move |_| set_pref(ui, |p| p.filter.no_notes = !p.filter.no_notes),
                    "notes " span { class: "dim", "{notes}" } }
            }
            div { class: "fgroup",
                span { class: "dim small", "kind" }
                for (k , n) in kinds {
                    button { key: "{k}", class: if f.kinds.contains(&k) { "chip on" } else { "chip" },
                        onclick: { let k = k.clone(); move |_| flip(|f| &mut f.kinds, k.clone()) },
                        "{k} " span { class: "dim", "{n}" } }
                }
            }
            div { class: "fgroup",
                span { class: "dim small", "has words of type" }
                for (k , n) in words {
                    button { key: "{k}", class: if f.words.contains(&k) { "chip on" } else { "chip" },
                        title: "{n} words of this type among the texts shown",
                        onclick: { let k = k.clone(); move |_| flip(|f| &mut f.words, k.clone()) },
                        "{k} " span { class: "dim", "{n}" } }
                }
            }
            if tint.is_empty() {
                div { class: "fgroup dim small", "pick a classifier in the strip (the palette) to filter by its classes, its verdicts and see its bias" }
            } else {
                div { class: "fgroup",
                    span { class: "dim small", "{tint}" }
                    for (c , n) in classes {
                        button { key: "{c}", class: if f.classes.contains(&c) { "chip on" } else { "chip" },
                            onclick: { let c = c.clone(); move |_| flip(|f| &mut f.classes, c.clone()) },
                            "{c} " span { class: "dim", "{n}" } }
                    }
                }
                div { class: "fgroup",
                    span { class: "dim small", "feedback" }
                    button { class: if f.feedback { "chip on" } else { "chip" },
                        title: "only texts this classifier was given a yes or no on (its latest 50)",
                        onclick: move |_| set_pref(ui, |p| p.filter.feedback = !p.filter.feedback),
                        "given a verdict "
                        span { class: "dim", {judged.as_ref().map(|j| format!("{} of {}", shown.iter().filter(|h| j.0.contains(&h.key)).count(), j.2)).unwrap_or_default()} }
                    }
                }
                if let Some((_, bias, _)) = &judged && bias.is_object() {
                    div { class: "fgroup small",
                        span { class: "dim", "bias" }
                        span { title: "mean KL of the steered from the trained outputs", {format!("KL {:.3}", bias["mean_kl"].as_f64().unwrap_or(0.0))} }
                        span { title: "share of texts whose top class the steering flips", {format!("flipped {:.0}%", 100.0 * bias["flipped"].as_f64().unwrap_or(0.0))} }
                        span { title: "how uneven the classes' shares are", {format!("imbalance {:.2}", bias["imbalance"].as_f64().unwrap_or(0.0))} }
                        span { title: "how far the predicted shares are from the training lists'", {format!("prior skew {:.2}", bias["prior_skew"].as_f64().unwrap_or(0.0))} }
                    }
                }
            }
        }
    }
}

/// weum's pfR line; its numbers go into the bar when clicked
#[component]
pub(crate) fn PfLine(n: u64) -> Element {
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
fn Art(hit: Hit, under: Option<String>, is_list: bool, is_cand: bool, selected: bool, tint: Option<String>) -> Element {
    let mut ui = ui();
    let api = api();
    let m = use_context::<Mem>();
    let title = format!("{} {}", hit.key, gem::gsum_line(&hit.text));
    // a long text shows piece by piece (lines at word boundaries, a few lines a piece)
    let (pw, pl) = {
        let p = ui.prefs.read();
        (p.piece_line, p.piece_lines)
    };
    let n_pieces = anend_core::pieces::count(&hit.text, pw, pl);
    let long = n_pieces > 1;
    let mut shown = use_signal(|| 1usize);
    let shown_n = shown().min(n_pieces);
    let shown_text: String = if long {
        anend_core::pieces::pieces(&hit.text, pw, pl)[..shown_n].concat()
    } else {
        String::new()
    };
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
    let under_style = match &under {
        Some(c) => format!("text-decoration: underline; text-decoration-color: {c};"),
        None => String::new(),
    };
    // a long text underlines its piece, not the controls under it
    let mut style = if long { String::new() } else { under_style.clone() };
    if let Some(t) = &tint {
        style.push_str(&format!(" border-left: 5px solid {t};"));
    }
    if long {
        style.push_str(&format!(" --piece-ch: {pw}ch;"));
        if shown_n > 1 {
            class.push_str(" open");
        }
    }
    let (h1, h2, h3, h4) = (hit.clone(), hit.clone(), hit.clone(), hit.clone());
    // the hover line: the title, the text's kind and its words' types (kinds.rs)
    let words = anend_core::kinds::describe_words(&hit.text);
    let t2 = if words.is_empty() {
        title.clone()
    } else {
        format!("{title} · {} · {words}", anend_core::kinds::detect(&hit.text))
    };
    let mut hover = m.hover;
    let mut hover_text = m.hover_text;
    // the words' types strip shows a long text's first piece, not all of it
    let ht = if long {
        anend_core::pieces::pieces(&hit.text, pw, pl)[0].to_string()
    } else {
        hit.text.clone()
    };
    let tagged = ui.prefs.read().word_types;
    // the tooltip has the word types on a line of their own
    let tip = if words.is_empty() {
        title.clone()
    } else {
        format!("{title}\n{} · {words}", anend_core::kinds::detect(&hit.text))
    };
    rsx! {
        div {
            class: "{class}",
            style: "{style}",
            role: "listitem",
            tabindex: "0",
            title: "{tip}",
            onmouseenter: move |_| {
                hover.set(t2.clone());
                hover_text.set(ht.clone());
            },
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
            onauxclick: {
                let h = hit.clone();
                move |e: PointerEvent| {
                    // the middle button opens a note in the editor
                    if e.trigger_button() == Some(dioxus::html::input_data::MouseButton::Auxiliary) && h.note {
                        let mut o = ui.open_note;
                        o.set(Some(h.key.clone()));
                        crate::shell::go(ui, View::Notes);
                    }
                }
            },
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
            if long {
                div { class: "piece", style: "{under_style}",
                    if tagged { TaggedText { text: shown_text.clone(), labels: true, ruby: true } } else { "{shown_text}" }
                }
                div { class: "pieces-more",
                    span { class: "dim", {format!("{} of {} pieces · {} KB", shown_n, n_pieces, hit.text.len().div_ceil(1000))} }
                    if shown_n < n_pieces {
                        button { class: "plain", title: "the next pieces",
                            onclick: move |e| { e.stop_propagation(); shown.set((shown_n + 4).min(n_pieces)); }, "more" }
                        button { class: "plain", title: "every piece (a long text takes a while to lay out)",
                            onclick: move |e| { e.stop_propagation(); shown.set(n_pieces); }, "all" }
                    }
                    if shown_n > 1 {
                        button { class: "plain", title: "back to the first piece",
                            onclick: move |e| { e.stop_propagation(); shown.set(1); }, "less" }
                    }
                }
            } else if tagged { TaggedText { text: hit.text.clone(), labels: true, ruby: true } } else { "{hit.text}" }
        }
    }
}

/// each class shown, its colour, and how many texts it has here
type Legend = Vec<(String, String, usize)>;

/// a classifier's tints as each key's colour (its class's, fainter when less sure; grey
/// when it's in none) and the legend: each class, its colour, how many texts it has here
fn tint_map(v: Option<&serde_json::Value>) -> (std::collections::HashMap<String, String>, Legend) {
    let mut map = std::collections::HashMap::new();
    let Some(v) = v else { return (map, vec![]) };
    let names: Vec<String> = v["classes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c.as_str().map(str::to_string))
        .collect();
    let palette: Vec<String> = v["palette"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c.as_str().map(str::to_string))
        .collect();
    let mut counts = vec![0usize; names.len() + 1];
    for t in v["tints"].as_array().into_iter().flatten() {
        let Some(key) = t["key"].as_str() else { continue };
        let strength = t["strength"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
        let colour = match t["class"].as_u64().map(|c| c as usize) {
            Some(c) if c < palette.len() => {
                counts[c] += 1;
                format!(
                    "color-mix(in srgb, {} {:.0}%, transparent)",
                    palette[c],
                    35.0 + 65.0 * strength
                )
            }
            _ => {
                counts[names.len()] += 1;
                "#6b6461".to_string()
            }
        };
        map.insert(key.to_string(), colour);
    }
    let mut legend: Vec<(String, String, usize)> = names
        .iter()
        .zip(&palette)
        .zip(&counts)
        .filter(|(_, k)| **k > 0)
        .map(|((n, c), k)| (n.clone(), c.clone(), *k))
        .collect();
    if counts[names.len()] > 0 {
        legend.push(("unsure".into(), "#6b6461".into(), counts[names.len()]));
    }
    (map, legend)
}

/// the classifier colouring the memory view, picked in the strip, and its search mode
#[component]
fn TintPick() -> Element {
    let ui = ui();
    let api = api();
    let p = ui.prefs.read().clone();
    let models = use_resource(move || {
        let api = api.clone();
        let _ = (ui.changed)();
        async move { api.models().await.unwrap_or_default() }
    });
    let trained: Vec<String> = models
        .read()
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.trained)
        .map(|m| m.name)
        .collect();
    if trained.is_empty() && p.tint.is_empty() {
        return rsx! {};
    }
    rsx! {
        span { class: "toggles tintpick",
            span { class: "icon-l", title: "colour the texts by a classifier's classes", Icon { name: "palette" } }
            select {
                "aria-label": "colour by a classifier",
                title: "colour the texts by a classifier's classes",
                onchange: move |e| { let v = e.value(); set_pref(ui, |p| { p.tint = v; if p.tint.is_empty() { p.cls_search = false; } }) },
                option { value: "", selected: p.tint.is_empty(), "no colours" }
                for n in trained {
                    option { key: "{n}", value: "{n}", selected: p.tint == n, "{n}" }
                }
            }
            if !p.tint.is_empty() {
                TChip { label: format!("search with {}: a class's name finds its texts, any other text the texts like it · C", p.tint), icon: "classifier", on: p.cls_search, onclick: move |_| set_pref(ui, |p| p.cls_search = !p.cls_search) }
            }
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
                                h.call(KeyPress { key: key.into(), code: String::new(), ctrl: false, shift: false, alt: false, bar: false, typing: false, text: String::new(), action: Some(if md == Mode::Ordinal { Action::SideOrdinal } else { Action::SideReversed }) });
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
            TChip { label: format!("word types: label every word (noun, verb, name, number…) · {}", ui.key_of(Action::WordTypes)), letter: "n·v", on: p.word_types, onclick: move |_| set_pref(ui, |p| p.word_types = !p.word_types) }
            TChip { label: format!("white: dark texts on white · {}", ui.key_of(Action::WhiteMode)), icon: "sun", on: p.white, onclick: move |_| set_pref(ui, |p| p.white = !p.white) }
        }
        TintPick {}
        span { class: "toggles filter-square",
            TChip {
                label: if p.filter.active() { "the search filter (on): kinds, word types, classes, verdicts, bias".to_string() } else { "the search filter: kinds, word types, classes, verdicts, bias".to_string() },
                icon: "filter",
                on: p.filter_open || p.filter.active(),
                onclick: move |_| set_pref(ui, |p| p.filter_open = !p.filter_open),
            }
        }
    }
}
