//! The app's shell: the top bar (the mark, the views, the search line, the status, undo and
//! settings), the views, the dock (the strip of toggles in the memory view and the space,
//! the command bar), toasts, the right-click menu, and the keyboard.

use dioxus::prelude::*;

use crate::gem::Measure;
use crate::keys::{Action, Binding};
use crate::prefs::Prefs;
use crate::ui::*;
use crate::views;

const CSS: &str = include_str!("style.css");

/// the one key listener: keys the shell or the open view takes, sent to Rust; the browser
/// keeps the rest (typing, Tab, the browser's own)
const KEYS_JS: &str = r#"
const KEY = /^\d{1,3}(,\d{1,3}){0,5}$/;
// the bindings the keymap holds here (`ctrl,shift,alt,code`), set by Rust per view
window.__anendTaken = window.__anendTaken || new Set();
// the keys menu waits for a key: every key goes to Rust
window.__anendListen = false;
window.addEventListener('keydown', (e) => {
  // an input method composing (IME): its keys are its own, Enter included
  if (e.isComposing || e.keyCode === 229) return;
  const t = e.target || {};
  const bar = t.id === 'bar';
  const typing = t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable;
  const k = e.key, c = e.ctrlKey || e.metaKey;
  const send = () => dioxus.send({ key: k, code: e.code, ctrl: c, shift: e.shiftKey, alt: e.altKey, bar, typing, text: bar ? t.value : '' });
  if (window.__anendListen) {
    if (!['Control', 'Shift', 'Alt', 'Meta'].includes(k)) { e.preventDefault(); send(); }
    return;
  }
  const bound = window.__anendTaken.has(`${+c},${+e.shiftKey},${+e.altKey},${e.code}`);
  // a text box keeps its own editing chords
  const editing = typing && !bar && c && ['z', 'Z', 'y', 'x', 'c', 'v', 'a'].includes(k);
  let take = false, prevent = false;
  if (editing) return;
  if (c && (k === 'z' || k === 'Z' || k === 'y') && bar) { take = true; prevent = true; }
  else if (bound && (c || e.altKey)) { take = true; prevent = true; }
  else if (c && (k === 'PageUp' || k === 'PageDown' || k === '+')) { take = true; prevent = true; }
  else if (k === 'Escape') take = true;
  else if (bar && (k === 'Enter' || k === 'ArrowUp' || k === 'ArrowDown')) { take = true; prevent = true; }
  else if (bar && k === 'Delete' && KEY.test(t.value.trim()) && t.selectionStart === t.value.length) { take = true; prevent = true; }
  else if (!typing && !e.altKey && !c) { take = true; if (bound || k.startsWith('Arrow') || k === ' ') prevent = true; }
  if (!take) return;
  if (prevent) e.preventDefault();
  send();
});
// zen: the bars fade while the mouse and keys rest; the top bar slides in near the top
let zenTimer = 0;
const stir = (y) => {
  const app = document.querySelector('.app');
  if (!app) return;
  app.classList.remove('idle');
  clearTimeout(zenTimer);
  if (app.dataset.zen === '1') zenTimer = setTimeout(() => {
    if (!app.querySelector('.dock:focus-within, .overlay, .menu')) app.classList.add('idle');
  }, (parseFloat(app.dataset.zenDelay) || 2.5) * 1000);
  // shown from the window's top edge, kept while the pointer stays on the bar
  if (y !== undefined) app.classList.toggle('peek', y < (app.classList.contains('peek') ? 36 : 8));
};
window.addEventListener('mousemove', (e) => stir(e.clientY), { passive: true });
window.addEventListener('keydown', () => stir(), { passive: true });
window.addEventListener('mousedown', (e) => stir(e.clientY), { passive: true });
"#;

fn measure_of(t: &str) -> Option<Measure> {
    let t = t.trim();
    let key_like = !t.is_empty() && t.split(',').all(|p| p.trim().parse::<u8>().is_ok());
    (!t.is_empty() && !key_like && t.parse::<u64>().is_err()).then(|| Measure::of(t))
}

#[component]
pub fn App() -> Element {
    let api = use_context_provider(crate::make_api);
    let prefs = use_signal(crate::prefs::load);
    let ui = use_context_provider(|| Ui {
        view: Signal::new(View::parse(&prefs.peek().view)),
        prefs,
        status: Signal::new((String::new(), false, 0)),
        toasts: Signal::new(Vec::new()),
        bar: Signal::new(String::new()),
        measure: Signal::new(None),
        settings: Signal::new(false),
        hello: Signal::new(Default::default()),
        topline: Signal::new(None),
        picks: Signal::new(Default::default()),
        pending: Signal::new(Pending::None),
        menu: Signal::new(None),
        actions: Signal::new(Vec::new()),
        bar_history: Signal::new(Default::default()),
        keys: Signal::new(Vec::new()),
        changed: Signal::new(0),
        bar_texts: Signal::new(Default::default()),
        seq: Signal::new(0),
        side: Signal::new(Default::default()),
        listening: Signal::new(None),
        open_note: Signal::new(None),
        pick_tag: Signal::new(None),
        closing: Signal::new(0),
    });
    // closing the window (desktop): the notes view saves the open note first; the window
    // only hides when asked to close (main.rs), and the app ends here
    #[cfg(not(target_arch = "wasm32"))]
    {
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        dioxus::desktop::use_wry_event_handler(move |ev, _| {
            if let Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } = ev
            {
                let mut c = ui.closing;
                if *c.peek() != 0 {
                    return;
                }
                c.set(1);
                dioxus::dioxus_core::spawn_forever(async move {
                    // a view that has something to keep takes it (2) and ends the app when
                    // it's kept; give up waiting after a few seconds
                    crate::ui::sleep_ms(200).await;
                    if *ui.closing.peek() == 2 {
                        crate::ui::sleep_ms(5000).await;
                    }
                    std::process::exit(0);
                });
            }
        });
    }

    // where classifiers read texts, as chosen in the settings (a shared hub keeps its own)
    {
        let api = api.clone();
        use_hook(move || {
            let device = prefs.peek().read_device.clone();
            spawn(async move {
                let _ = api
                    .call("classify.device", serde_json::json!({ "device": device }))
                    .await;
            });
        });
    }

    // the save's name and counts, and again whenever something changed
    {
        let api = api.clone();
        use_effect(move || {
            let _ = (ui.changed)();
            ui.refresh(api.clone());
        });
    }

    // the bar's text measured a beat after typing stops, off the input path
    use_effect(move || {
        let t = ui.bar.read().clone();
        spawn(async move {
            sleep_ms(110).await;
            if *ui.bar.peek() == t {
                let mut m = ui.measure;
                m.set(measure_of(&t));
            }
        });
    });

    // the page's key listener keeps the keys bound here from the browser
    use_effect(move || {
        let view = (ui.view)();
        let keymap = crate::keys::Keymap::from_changes(&ui.prefs.read().keymap);
        let listen = ui.listening.read().is_some();
        let taken = serde_json::to_string(&keymap.taken(view)).unwrap_or_default();
        document::eval(&format!(
            "window.__anendTaken = new Set({taken}); window.__anendListen = {listen};"
        ));
    });

    // the keyboard
    {
        let api = api.clone();
        use_hook(move || {
            spawn(async move {
                let mut ev = document::eval(KEYS_JS);
                while let Ok(k) = ev.recv::<KeyPress>().await {
                    on_key(ui, api.clone(), k);
                }
            });
        });
    }

    let p = prefs.read();
    let vars = format!(
        "{}{}",
        crate::theme::css_vars(&p.palette, &p.colours),
        crate::theme::hl_vars(&p.palette, &p.colours)
    );
    let zoom = p.zoom;
    let fs = p.font_size;
    let (zen, zen_delay, hide_bar) = (p.zen, p.zen_delay, p.hide_bar);
    let view = (ui.view)();
    // zoom only when it isn't 1 (WebKit's css zoom upsets the space's 3d transforms)
    let zoomed = if (zoom - 1.0).abs() > 0.001 {
        format!("zoom: {zoom}; height: calc(100vh / {zoom});")
    } else {
        String::new()
    };
    // a family the person named goes before the built-in ones
    let fam = |f: &str, rest: &str| {
        let f = f.trim().replace(['"', ';', '{', '}'], "");
        if f.is_empty() {
            String::new()
        } else {
            format!(
                "--{rest}: \"{f}\", {};",
                if rest == "font" {
                    "\"Fira Sans\", system-ui, sans-serif"
                } else {
                    "\"Times New Roman\", serif"
                }
            )
        }
    };
    let style = format!(
        "{vars} --fs: {fs}px; --mfs: {}px; --mspace: {}px; --top: {}px; --barh: {}px; {}{}{zoomed}",
        p.memory_font,
        p.spacing,
        p.top_bar.clamp(20.0, 80.0),
        p.bottom_bar.clamp(20.0, 80.0),
        fam(&p.font, "font"),
        fam(&p.memory_family, "serif"),
    );
    let swapped = p.bars_swapped;
    drop(p);
    rsx! {
        style { {crate::fonts::css()} }
        style { {CSS} }
        div {
            class: "app v-{view.name()}",
            class: if zen { "zen" } else { "" },
            class: if hide_bar { "hide-bar" } else { "" },
            class: if swapped { "swapped" } else { "" },
            "data-zen": if zen { "1" } else { "0" },
            "data-zen-delay": "{zen_delay}",
            style: "{style}",
            TopBar {}
            div { class: "views",
                for v in View::ALL {
                    views::ViewHost { key: "{v.name()}", view: v }
                }
            }
            Dock {}
            Toasts {}
            ContextMenu {}
            if (ui.settings)() {
                views::settings::Settings {}
            }
        }
    }
}

fn set_zoom(mut ui: Ui, z: f32) {
    ui.prefs.write().zoom = (z * 20.0).round().clamp(10.0, 60.0) / 20.0;
    ui.save_prefs();
    let z = ui.prefs.peek().zoom;
    ui.say(format!("zoom {}%", (z * 100.0).round()));
}

fn on_key(mut ui: Ui, api: Api, mut k: KeyPress) {
    let view = *ui.view.peek();
    // the keys menu is waiting for a key
    if let Some(a) = *ui.listening.peek() {
        crate::views::settings::rebind(ui, a, &k);
        return;
    }
    let keymap = ui.keymap();
    let pressed = Binding::of(&k.code, k.ctrl, k.shift, k.alt);
    k.action = pressed.and_then(|b| keymap.action(view, b, k.typing || k.bar));
    let z = ui.prefs.peek().zoom;
    // the bar's own undo (Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z while in it)
    if k.ctrl && k.bar && matches!(k.key.as_str(), "z" | "Z" | "y") {
        ui.bar_undo(k.key != "z");
        return;
    }
    if k.ctrl && !k.alt {
        match k.key.as_str() {
            "PageUp" | "+" => return set_zoom(ui, z + 0.05),
            "PageDown" => return set_zoom(ui, z - 0.05),
            _ => {}
        }
    }
    let global = match k.action {
        Some(Action::GotoMemory) => Some(View::Memory),
        Some(Action::GotoSpace) => Some(View::Space),
        Some(Action::GotoLists) => Some(View::Lists),
        Some(Action::GotoReader) => Some(View::Reader),
        Some(Action::GotoNotes) => Some(View::Notes),
        Some(Action::GotoTags) => Some(View::Tags),
        Some(Action::GotoStats) => Some(View::Stats),
        Some(Action::GotoModels) => Some(View::Models),
        Some(Action::GotoFilters) => Some(View::Filters),
        _ => None,
    };
    if let Some(v) = global {
        go(ui, v);
        return;
    }
    match k.action {
        Some(Action::OpenSettings) => {
            let open = *ui.settings.peek();
            ui.settings.set(!open);
            return;
        }
        Some(Action::OpenKeys) => {
            crate::shell::set_pref(ui, |p| {
                p.collapsed.remove("keys");
            });
            ui.settings.set(true);
            return;
        }
        Some(Action::ZoomIn) => return set_zoom(ui, z + 0.05),
        Some(Action::ZoomOut) => return set_zoom(ui, z - 0.05),
        Some(Action::ZoomReset) => return set_zoom(ui, 1.0),
        // the space keeps its own undo (moves, pins, removals) and falls back on this
        Some(Action::Undo) if !k.typing && view != View::Space => return undo_last(ui, api),
        Some(Action::FocusBar) if !*ui.settings.peek() => {
            ui.focus_bar();
            return;
        }
        _ => {}
    }
    if k.key == "Escape" {
        if ui.menu.peek().is_some() {
            ui.menu.set(None);
        } else if *ui.settings.peek() {
            ui.settings.set(false);
        } else if *ui.pending.peek() != Pending::None {
            ui.pending.set(Pending::None);
            ui.say("never mind");
        } else if k.bar {
            document::eval("document.activeElement && document.activeElement.blur()");
        } else {
            let handler = ui.keys.peek().iter().find(|(v, _)| *v == view).map(|(_, h)| *h);
            if let Some(h) = handler {
                h.call(k);
            }
        }
        return;
    }
    if *ui.settings.peek() || ui.menu.peek().is_some() {
        return;
    }
    let handler = ui.keys.peek().iter().find(|(v, _)| *v == view).map(|(_, h)| *h);
    let taken = handler.map(|h| h.call(k.clone())).unwrap_or(false);
    if !taken && !k.bar && !k.typing && k.key == "Enter" {
        ui.focus_bar();
    }
}

pub fn go(mut ui: Ui, v: View) {
    let from = *ui.view.peek();
    if from == v {
        return;
    }
    // each view keeps its own bar text
    let now = ui.bar.peek().clone();
    ui.bar_texts.write().insert(from, now);
    let back = ui.bar_texts.peek().get(&v).cloned().unwrap_or_default();
    ui.set_bar(&back);
    ui.view.set(v);
    ui.prefs.write().view = v.name().to_string();
    ui.save_prefs();
    ui.menu.set(None);
}

#[component]
fn TopBar() -> Element {
    let mut ui = ui();
    let api = api();
    let view = (ui.view)();
    let (status, bad, _) = (ui.status)();
    let hello = (ui.hello)();
    let line = (ui.topline)();
    let show_line = matches!(view, View::Memory | View::Space);
    // the notes view counts its notes at the bar's end ("n of m notes" while narrowed),
    // so the top line leaves its count out there: one notes count per view
    let idle = if view == View::Notes {
        format!("{} · {} texts", hello.save, hello.counts.memories)
    } else {
        format!(
            "{} · {} texts · {} notes",
            hello.save, hello.counts.memories, hello.counts.notes
        )
    };
    let last = ui.actions.read().last().map(|a| a.describe());
    rsx! {
        header { class: "top", role: "banner",
            nav { class: "tabs", "aria-label": "views",
                for (i , v) in View::ALL.into_iter().enumerate() {
                    button {
                        key: "{v.name()}",
                        class: if v == view { "tab on" } else { "tab" },
                        class: if i == 0 { "tip-right" } else { "" },
                        "data-tip": "{v.name()} · Ctrl+{i + 1}",
                        "aria-label": "{v.name()}",
                        "aria-current": if v == view { "page" } else { "false" },
                        onclick: move |_| go(ui, v),
                        Icon { name: v.icon() }
                    }
                }
            }
            if show_line {
                if let Some(l) = line {
                    div { class: "searchline", role: "status",
                        b { "{l.title}" }
                        if !l.detail.is_empty() {
                            span { class: "detail", "{l.detail}" }
                        }
                        Triangle { s: l.s, r: l.r }
                    }
                }
            }
            span { class: "grow" }
            span {
                class: if bad { "status bad" } else { "status" },
                role: "status",
                "aria-live": "polite",
                if status.is_empty() { "{idle}" } else { "{status}" }
            }
            button {
                class: "side tip-left",
                "data-tip": match &last { Some(d) => format!("undo {d} · Ctrl+Z"), None => "nothing to undo".to_string() },
                "aria-label": "undo",
                disabled: last.is_none(),
                onclick: move |_| undo_last(ui, api.clone()),
                Icon { name: "undo" }
            }
            button {
                class: "side tip-left",
                "data-tip": "settings · Ctrl+,",
                "aria-label": "settings",
                onclick: move |_| ui.settings.set(!(ui.settings)()),
                Icon { name: "settings" }
            }
        }
    }
}

#[component]
fn Dock() -> Element {
    let ui = ui();
    let view = (ui.view)();
    let pending = (ui.pending)();
    // the bar names what Enter will do only while it does something else than search; it
    // never names the view (the tabs say which one is open)
    let badge = match &pending {
        Pending::Edit(h) => Some(format!("editing {}", h.key)),
        Pending::EditOwn { .. } => Some("this placement's own text".to_string()),
        Pending::Tag(hs) => Some(format!(
            "tag {} with…",
            if hs.len() == 1 {
                hs[0].text.clone()
            } else {
                format!("{} texts", hs.len())
            }
        )),
        Pending::None => None,
    };
    rsx! {
        footer { class: "dock",
            div { class: "cmdbar",
                if let Some(b) = badge {
                    label { class: "pending", r#for: "bar", "{b}" }
                }
                input {
                    id: "bar",
                    r#type: "text",
                    autocomplete: "off",
                    spellcheck: "false",
                    "aria-label": "command bar",
                    placeholder: view.placeholder(),
                    oninput: move |e| ui.typed(e.value()),
                }
                div { class: "side", role: "toolbar", "aria-label": "{view.name()} controls",
                    views::Side { view }
                }
            }
        }
    }
}

#[component]
fn Toasts() -> Element {
    let mut ui = ui();
    let toasts = (ui.toasts)();
    rsx! {
        div { class: "toasts", "aria-live": "polite",
            for t in toasts {
                div {
                    key: "{t.id}",
                    class: "toast",
                    title: "click: into the bar · double click: dismiss",
                    onclick: {
                        let text = t.text.clone();
                        move |_| ui.set_bar(&text)
                    },
                    ondoubleclick: move |_| ui.toasts.write().retain(|x| x.id != t.id),
                    "{t.text}"
                }
            }
        }
    }
}

#[component]
fn ContextMenu() -> Element {
    let mut ui = ui();
    let api = api();
    let Some(m) = (ui.menu)() else { return rsx! {} };
    let hit = m.hit.clone();
    let selected = ui.picks.read().selected.iter().any(|h| h.key == hit.key);
    let (x, y) = (m.x.min(10_000.0), m.y.min(10_000.0));
    let item = move |f: Box<dyn Fn()>| {
        move |_| {
            f();
            let mut menu = ui.menu;
            menu.set(None);
        }
    };
    let h1 = hit.clone();
    let h2 = hit.clone();
    let h3 = hit.clone();
    let h4 = hit.clone();
    let h5 = hit.clone();
    let h6 = hit.clone();
    let h7 = hit.clone();
    let api2 = api.clone();
    rsx! {
        div { class: "backdrop", onclick: move |_| ui.menu.set(None), oncontextmenu: move |e| { e.prevent_default(); ui.menu.set(None) } }
        div {
            class: "menu",
            role: "menu",
            style: "left: min({x}px, calc(100vw - 200px)); top: min({y}px, calc(100vh - 300px));",
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            tabindex: "-1",
            div { class: "title", "{hit.text}" }
            MenuKind { key: "{hit.key}", entry: hit.key.clone() }
            button { role: "menuitem", onclick: item(Box::new(move || {
                    ui.set_bar(&h1.text);
                    let mut p = ui.pending; p.set(Pending::Edit(h1.clone()));
                    ui.focus_bar();
                    ui.say("edit it in the bar; Enter puts it back under the same key, Esc leaves it");
                })), Icon { name: "edit" } "edit" kbd { "Enter keeps the key" } }
            button { role: "menuitem", onclick: item(Box::new(move || {
                    let sel = ui.picks.peek().selected.clone();
                    let targets = if sel.iter().any(|h| h.key == h2.key) { sel } else { vec![h2.clone()] };
                    let mut p = ui.pending; p.set(Pending::Tag(targets));
                    ui.set_bar("");
                    ui.focus_bar();
                    ui.say("type a tag and press Enter");
                })), Icon { name: "tag" } "tag…" }
            button { role: "menuitem", onclick: item(Box::new(move || {
                    let mut picks = ui.picks;
                    let mut p = picks.write();
                    if let Some(i) = p.selected.iter().position(|h| h.key == h3.key) { p.selected.remove(i); } else { p.selected.push(h3.clone()); }
                })), Icon { name: "check" } if selected { "unselect" } else { "select" } kbd { "Alt+click" } }
            button { role: "menuitem", onclick: item(Box::new(move || {
                    let mut p = ui.picks; p.write().list = Some(h4.clone());
                    ui.say(format!("list: {}", h4.text));
                })), Icon { name: "list" } "make it the list" kbd { "Shift+click" } }
            button { role: "menuitem", onclick: item(Box::new(move || {
                    let mut p = ui.picks; p.write().candidate = Some(h5.clone());
                    ui.say(format!("candidate: {}", h5.text));
                })), Icon { name: "pick" } "make it the candidate" kbd { "Ctrl+click" } }
            button { role: "menuitem", onclick: item(Box::new(move || {
                    ui.set_bar(&h6.key);
                })), Icon { name: "keys" } "its key into the bar" kbd { "double click" } }
            if !hit.note {
                button { role: "menuitem", onclick: item(Box::new(move || {
                        let h = h7.clone();
                        let api = api2.clone();
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.forget(&h.key).await {
                                Ok(text) => {
                                    ui.did(Did::Forgot { key: h.key.clone(), text: text.clone() });
                                    ui.say(format!("forgot “{text}” ({}) · Ctrl+Z takes it back", h.key));
                                    ui.touched();
                                }
                                Err(e) => ui.fail(e),
                            }
                        });
                    })), Icon { name: "delete" } "forget" kbd { "Delete" } }
            }
        }
    }
}

/// in the right-click menu: when the entry was written, and its kind (told by its
/// pattern; picked here when the guess is wrong, kept in the save)
#[component]
fn MenuKind(entry: String) -> Element {
    let ui = ui();
    let api = api();
    let key = entry;
    let k0 = key.clone();
    let api_i = api.clone();
    let info = use_resource(move || {
        let (api, k) = (api_i.clone(), k0.clone());
        async move { api.call("entries.info", serde_json::json!({ "key": k })).await }
    });
    let Some(Ok(v)) = info.read().clone() else {
        return rsx! {};
    };
    let kind = v["kind"].as_str().unwrap_or("text").to_string();
    let detected = v["detected"].as_str().unwrap_or("text").to_string();
    let fixed = v["fixed"] == true;
    let kinds: Vec<String> = v["kinds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|k| k.as_str().map(str::to_string))
        .collect();
    let written = v["created"].as_str().unwrap_or("").get(..10).unwrap_or("").to_string();
    rsx! {
        div { class: "kindrow small",
            label { "kind"
                select {
                    "aria-label": "its kind",
                    value: "{kind}",
                    onchange: move |e| {
                        let api = api.clone();
                        let k = key.clone();
                        let pick = e.value();
                        let det = detected.clone();
                        let mut menu = ui.menu;
                        menu.set(None);
                        dioxus::dioxus_core::spawn_forever(async move {
                            let set = if pick == det { String::new() } else { pick.clone() };
                            match api.call("entries.set_kind", serde_json::json!({ "key": k, "kind": set })).await {
                                Ok(_) => ui.say(format!("{k} is a {pick} now")),
                                Err(e) => ui.fail(e),
                            }
                        });
                    },
                    for k in kinds {
                        option { key: "{k}", value: "{k}", selected: k == kind, "{k}" }
                    }
                }
            }
            span { class: "dim", if fixed { "set by hand" } else { "by its pattern" } }
            if !written.is_empty() { span { class: "dim", "written {written}" } }
        }
    }
}

/// the prefs, changed and kept
pub fn set_pref(mut ui: Ui, f: impl FnOnce(&mut Prefs)) {
    f(&mut ui.prefs.write());
    ui.save_prefs();
}
