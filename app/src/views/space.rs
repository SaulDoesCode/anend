//! The space: the three.js room (app/src/space.js, over the vendored three.js in app/vendor)
//! hosted through Dioxus's JS interop. Rust searches and lays texts out (`space.spread`),
//! JS flies and draws; they talk over one eval channel.
//!
//! Fixes over the web app's: clicking a text puts its triangle in the top bar (centered
//! there); the room's colour is a setting with the texts' ink picked for contrast; the
//! numbers card on the right is dismissable (×, Esc) and plain HTML, so its text is crisp;
//! the arrow keys ease the camera onto the nearest text that way; texts shrink near.

use dioxus::prelude::*;
use futures_util::{StreamExt, future::Either};
use serde_json::{Value, json};

use crate::backend::Hit;
use crate::gem::{self, Mode};
use crate::keys::Action;
use crate::shell::set_pref;
use crate::ui::*;
use crate::views::{TChip, use_keys};

const THREE: &str = include_str!("../../vendor/three.module.min.js");
const ENGINE: &str = include_str!("../space.js");

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct SpaceEvent {
    kind: String,
    key: String,
    text: String,
    note: bool,
    x: f64,
    y: f64,
    ctrl: bool,
    shift: bool,
    alt: bool,
    q: f64,
    hue: f64,
}

fn boot_js() -> String {
    format!(
        r#"
const THREE_SRC = {three};
const ENGINE_SRC = {engine};
const url = (src) => URL.createObjectURL(new Blob([src], {{ type: 'text/javascript' }}));
const tu = url(THREE_SRC);
const eu = url(ENGINE_SRC.split("'__THREE__'").join(JSON.stringify(tu)));
let host = null;
for (let i = 0; i < 100 && !host; i++) {{ host = document.getElementById('space-host'); if (!host) await new Promise((r) => setTimeout(r, 20)); }}
try {{
  const mod = await import(eu);
  const sp = new mod.Space(host, (m) => dioxus.send(m));
  window.__anendSpace = sp;
  dioxus.send({{ kind: 'ready', key: '', text: '', note: false, x: 0, y: 0, ctrl: false, shift: false, alt: false, q: 0, hue: 0 }});
  while (true) {{
    const c = await dioxus.recv();
    if (c.cmd === 'place') sp.place(c.texts, c.spread, c.again);
    else if (c.cmd === 'clear') sp.clear();
    else if (c.cmd === 'room') sp.setRoom(c.bg, c.ink);
    else if (c.cmd === 'start') sp.start();
    else if (c.cmd === 'stop') sp.stop();
    else if (c.cmd === 'mark') sp.mark(c.list, c.candidate);
    else if (c.cmd === 'focus') host.focus();
    else if (c.cmd === 'keys') sp.setKeys(c.map);
  }}
}} catch (e) {{
  dioxus.send({{ kind: 'error', key: '', text: String(e && e.message || e), note: false, x: 0, y: 0, ctrl: false, shift: false, alt: false, q: 0, hue: 0 }});
}}
"#,
        three = serde_json::to_string(THREE).unwrap_or_default(),
        engine = serde_json::to_string(ENGINE).unwrap_or_default(),
    )
}

#[derive(Clone, Copy)]
struct Sp {
    tx: Signal<Option<futures_channel::mpsc::UnboundedSender<Value>>>,
    texts: Signal<Vec<Hit>>,
    card: Signal<Option<SpaceEvent>>,
    pos: Signal<String>,
}

impl Sp {
    fn send(&self, v: Value) {
        if let Some(tx) = self.tx.peek().as_ref() {
            let _ = tx.unbounded_send(v);
        }
    }
}

/// lays hits out: in place of what's there, or (keep) added to it
fn show(ui: Ui, api: Api, sp: Sp, hits: Vec<Hit>) {
    let (keep, reversed) = {
        let p = ui.prefs.peek();
        (p.space_keep, p.space_reversed)
    };
    let mut all = if keep { sp.texts.peek().clone() } else { vec![] };
    let again = keep && !all.is_empty();
    for h in hits {
        if !all.iter().any(|x| x.key == h.key) {
            all.push(h);
        }
    }
    let mut texts = sp.texts;
    if all.is_empty() {
        texts.set(vec![]);
        sp.send(json!({ "cmd": "clear" }));
        return;
    }
    dioxus::dioxus_core::spawn_forever(async move {
        let pairs: Vec<(String, String)> = all.iter().map(|h| (h.key.clone(), h.text.clone())).collect();
        match api.spread(&pairs, reversed).await {
            Ok(s) => {
                sp.send(json!({ "cmd": "place", "texts": all, "spread": s, "again": again }));
                texts.set(all);
            }
            Err(e) => ui.fail(e),
        }
    });
}

fn topline(mut ui: Ui, title: String, mode: Mode, text: Option<&str>, n: usize) {
    let (s, r) = text.map(gem::sums).unwrap_or((0, 0));
    ui.topline.set(Some(TopLine {
        title,
        detail: format!("{} · {n} text{}", mode.name(), if n == 1 { "" } else { "s" }),
        s,
        r,
    }));
}

async fn gsearch(ui: Ui, api: Api, sp: Sp, n: u64, mode: Mode, text: Option<String>) {
    match api.gsearch(mode, &n.to_string()).await {
        Ok(hits) => {
            let t = text.or_else(|| hits.first().map(|h| h.text.clone()));
            topline(ui, n.to_string(), mode, t.as_deref(), hits.len());
            if hits.is_empty() {
                ui.say(format!("nothing in {} {n} yet", mode.name()));
            }
            show(ui, api, sp, hits);
        }
        Err(e) => ui.fail(e),
    }
}

async fn enter(ui: Ui, api: Api, sp: Sp, raw: String, ctrl: bool, shift: bool) {
    let t = raw.trim().to_string();
    if t.is_empty() {
        return;
    }
    let (gs, sat, rev) = {
        let p = ui.prefs.peek();
        (p.gsearch_text, p.saturate, p.space_reversed)
    };
    let mode = if rev { Mode::Reversed } else { Mode::Ordinal };
    if t == "Clear" {
        show(ui, api, sp, vec![]);
        return;
    }
    if shift {
        match api.remember(&t).await {
            Ok(h) => {
                let mut texts = sp.texts.peek().clone();
                texts.push(h);
                show(ui, api, sp, texts);
            }
            Err(e) => ui.fail(e),
        }
        return;
    }
    if let Ok(n) = t.trim_start_matches('#').parse::<u64>() {
        gsearch(ui, api, sp, n, mode, None).await;
        return;
    }
    if !gs && !ctrl {
        match api.search(&t, true).await {
            Ok(hits) => {
                topline(ui, t.clone(), mode, Some(&t), hits.len());
                show(ui, api, sp, hits);
            }
            Err(e) => ui.fail(e),
        }
        return;
    }
    if sat || ctrl {
        match api.saturate(&t).await {
            Ok((key, new)) => {
                if new {
                    ui.did(Did::Remembered {
                        key: key.clone(),
                        text: t.clone(),
                    });
                    ui.say(format!("remembered “{t}” under {key}"));
                    ui.touched();
                }
            }
            Err(e) => ui.fail(e),
        }
    }
    gsearch(ui, api, sp, gem::sum_in(&t, mode), mode, Some(t)).await;
}

#[component]
pub fn Space() -> Element {
    let ui = ui();
    let api = api();
    let sp = Sp {
        tx: use_signal(|| None),
        texts: use_signal(Vec::new),
        card: use_signal(|| None),
        pos: use_signal(String::new),
    };
    let mut help = use_signal(|| false);
    // the engine and its channel
    {
        let api = api.clone();
        use_hook(move || {
            let (tx, mut rx) = futures_channel::mpsc::unbounded::<Value>();
            let mut stx = sp.tx;
            stx.set(Some(tx));
            spawn(async move {
                let mut ev = document::eval(&boot_js());
                loop {
                    let got = {
                        let recv = ev.recv::<SpaceEvent>();
                        let next = rx.next();
                        futures_util::pin_mut!(recv, next);
                        match futures_util::future::select(recv, next).await {
                            Either::Left((r, _)) => Either::Left(r),
                            Either::Right((c, _)) => Either::Right(c),
                        }
                    };
                    match got {
                        Either::Left(Ok(e)) => on_event(ui, api.clone(), sp, e),
                        Either::Left(Err(_)) => break,
                        Either::Right(Some(c)) => {
                            let _ = ev.send(c);
                        }
                        Either::Right(None) => break,
                    }
                }
            });
        });
    }
    // running only while shown; the room's colour and ink from the settings
    use_effect(move || {
        let shown = (ui.view)() == View::Space;
        sp.send(json!({ "cmd": if shown { "start" } else { "stop" } }));
    });
    // the flying keys as the keymap binds them
    use_effect(move || {
        let map = crate::keys::Keymap::from_changes(&ui.prefs.read().keymap).flying();
        sp.send(json!({ "cmd": "keys", "map": map }));
    });
    use_effect(move || {
        let bg = ui.prefs.read().space_bg.clone();
        sp.send(json!({ "cmd": "room", "bg": bg, "ink": crate::theme::ink_on(&bg) }));
    });
    use_effect(move || {
        let p = (ui.picks)();
        sp.send(json!({ "cmd": "mark", "list": p.list.map(|h| h.key), "candidate": p.candidate.map(|h| h.key) }));
    });
    {
        let api = api.clone();
        use_keys(View::Space, move |k: KeyPress| {
            if k.bar {
                if k.key == "Enter" {
                    dioxus::dioxus_core::spawn_forever(enter(ui, api.clone(), sp, k.text.clone(), k.ctrl, k.shift));
                    return true;
                }
                return false;
            }
            if k.key == "Escape" {
                let mut c = sp.card;
                c.set(None);
                return true;
            }
            match k.action {
                Some(Action::Help) => help.set(!help()),
                Some(Action::Reversed) => set_pref(ui, |p| p.space_reversed = !p.space_reversed),
                // the engine flies and hops on the rest itself
                _ => {
                    return matches!(
                        k.key.as_str(),
                        "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | " "
                    );
                }
            }
            true
        });
    }
    let card = (sp.card)();
    let n = sp.texts.read().len();
    let bg = ui.prefs.read().space_bg.clone();
    let ink = crate::theme::ink_on(&bg);
    rsx! {
        div { class: "space", style: "position: absolute; inset: 0; --room-bg: {bg}; --room-ink: {ink};",
            div { id: "space-host", class: "space-host", tabindex: "0", "aria-label": "the space: right drag looks, W A S D fly, arrows hop between texts" }
            div { class: "poscard", "{sp.pos}" }
            if n == 0 {
                div { class: "space-empty",
                    p { "The room is empty." }
                    p { class: "small", "Type a word, a number or a key in the bar and press Enter: its bucket spreads out here. Right-drag to look around, W A S D to fly, the arrow keys hop from text to text." }
                }
            }
            button { class: "plain tip-left", style: "position: absolute; right: 6px; bottom: 6px; z-index: 2;", "data-tip": "how to fly · H", "aria-label": "help",
                onclick: move |_| help.set(!help()), Icon { name: "help" } }
            if help() {
                div { class: "space-help",
                    b { "flying " } "right mouse drag looks · W A S D move · Space / C up and down · Q / E roll · Shift fast · [ ] or the wheel speed · R level · Home back · arrows hop to the nearest text that way"
                    br {}
                    b { "texts " } "hover for its numbers · click: into the bar and its triangle up top · Ctrl+click candidate · Shift+click the list · right click the menu"
                }
            }
            if let Some(c) = card {
                Card { ev: c, card: sp.card }
            }
        }
    }
}

fn on_event(mut ui: Ui, api: Api, sp: Sp, e: SpaceEvent) {
    let hit = Hit {
        key: e.key.clone(),
        text: e.text.clone(),
        note: e.note,
    };
    let mut card = sp.card;
    let mut pos = sp.pos;
    match e.kind.as_str() {
        "info" => pos.set(e.text),
        "error" => ui.fail(format!("the space couldn't start: {}", e.text)),
        "hover" | "focus" => card.set(Some(e.clone())),
        "click" => {
            if e.ctrl {
                ui.picks.write().candidate = Some(hit.clone());
                ui.say(format!("candidate: {}", hit.text));
            } else if e.shift {
                ui.picks.write().list = Some(hit.clone());
                ui.say(format!("list: {}", hit.text));
            } else {
                ui.set_bar(&hit.text);
                let mode = if ui.prefs.peek().space_reversed {
                    Mode::Reversed
                } else {
                    Mode::Ordinal
                };
                // its triangle in the top bar
                let (s, r) = gem::sums(&hit.text);
                ui.topline.set(Some(TopLine {
                    title: hit.text.clone(),
                    detail: format!("{} {}", mode.name(), gem::sum_in(&hit.text, mode)),
                    s,
                    r,
                }));
                card.set(Some(e.clone()));
            }
        }
        "menu" => {
            if e.ctrl {
                let m = gem::Measure::of(&hit.text);
                let mode = if ui.prefs.peek().space_reversed {
                    Mode::Reversed
                } else {
                    Mode::Ordinal
                };
                dioxus::dioxus_core::spawn_forever(gsearch(ui, api, sp, m.triangle(), mode, Some(hit.text)));
            } else {
                ui.menu.set(Some(Menu { hit, x: e.x, y: e.y }));
            }
        }
        _ => {}
    }
}

/// the numbers of a hovered or clicked text: plain HTML on the right, dismissable
#[component]
fn Card(ev: SpaceEvent, card: Signal<Option<SpaceEvent>>) -> Element {
    let ui = ui();
    let m = gem::Measure::of(&ev.text);
    let (s, r) = (m.ordinal, m.reversed);
    let mut close = card;
    let n = move |v: u64| {
        rsx! {
            span { class: "num", title: "into the bar", onclick: move |_| ui.set_bar(&v.to_string()), "{v}" }
        }
    };
    rsx! {
        aside { class: "metrics", role: "dialog", "aria-label": "the text's numbers",
            button { class: "plain ibtn close", "aria-label": "close", "data-tip": "close · Esc", onclick: move |_| close.set(None), Icon { name: "close" } }
            div { class: "title", "{ev.text}" }
            table {
                tr { th { "s" } td { {n(s)} } th { "r" } td { {n(r)} } }
                tr { th { "s+r" } td { {n(s + r)} } th { "|s−r|" } td { {n(s.abs_diff(r))} } }
                tr { th { "s·r" } td { {n(s * r)} } th { "⌊√s·r⌋" } td { {n(m.triangle())} } }
                tr { th { "bytes" } td { {n(m.bytes)} } th { "letters" } td { {gem::letters(&m.frequency)} } }
            }
            Triangle { s, r, height: 40.0 }
            div { class: "gsum", {gem::gsum_line(&ev.text)} }
            div { class: "dim small",
                "{ev.key} · ring "
                span { class: "swatch", style: "display: inline-block; width: 10px; height: 10px; border-radius: 2px; background: hsl({ev.hue} 70% 45%);" }
                " {ev.q}"
            }
        }
    }
}

/// the strip over the bar in the space
#[component]
pub fn SpaceStrip() -> Element {
    let ui = ui();
    let p = ui.prefs.read().clone();
    rsx! {
        span { class: "toggles",
            TChip { label: "gsearch: a text by its sum, not its words", icon: "search", on: p.gsearch_text, onclick: move |_| set_pref(ui, |p| p.gsearch_text = !p.gsearch_text) }
            TChip { label: "reversed: bucket by the reversed sum · F", letter: "R", on: p.space_reversed, onclick: move |_| set_pref(ui, |p| p.space_reversed = !p.space_reversed) }
            TChip { label: "saturate: remember what's searched", icon: "bolt", on: p.saturate, onclick: move |_| set_pref(ui, |p| p.saturate = !p.saturate) }
            TChip { label: "keep: add new results to the spread", icon: "pin", on: p.space_keep, onclick: move |_| set_pref(ui, |p| p.space_keep = !p.space_keep) }
        }
    }
}
