//! Settings: a dialog that closes on a click outside it (or Esc), its sections folding
//! (remembered), separated by rules. Looks, the memory view, the space, the saves (open,
//! new, copy, rename, delete, a backup, and importing weum's all-of-it.json step by step),
//! the keys, and the hub (folded; absent without the `hub` feature).

use dioxus::prelude::*;

use crate::backend::{Imported, SaveInfo};
use crate::shell::set_pref;
use crate::theme::{PALETTES, ROLES, colour};
use crate::ui::*;
use crate::views::Fold;

#[component]
pub fn Settings() -> Element {
    let mut ui = ui();
    rsx! {
        div {
            class: "overlay",
            onclick: move |_| ui.settings.set(false),
            div {
                class: "dialog",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "settings",
                onclick: move |e| e.stop_propagation(),
                div { class: "head",
                    h2 { "settings" }
                    span { class: "grow" }
                    button { class: "plain ibtn close tip-left", "data-tip": "close · Esc", "aria-label": "close",
                        onclick: move |_| ui.settings.set(false), Icon { name: "close" } }
                }
                Looks {}
                hr {}
                MemorySettings {}
                hr {}
                Saves {}
                hr {}
                Keys {}
                if cfg!(feature = "hub") {
                    hr {}
                    Hub {}
                }
            }
        }
    }
}

#[component]
fn Looks() -> Element {
    let ui = ui();
    let p = ui.prefs.read().clone();
    rsx! {
        Fold { name: "looks", title: "looks",
            label { "palette" }
            div { class: "row",
                for name in PALETTES {
                    button {
                        key: "{name}",
                        class: if p.palette == name { "on" } else { "" },
                        onclick: move |_| set_pref(ui, |p| { p.palette = name.to_string(); p.colours.clear(); }),
                        "{name}"
                    }
                }
            }
            label { "your colours" }
            div { class: "swatches",
                for (role , _) in ROLES {
                    label { key: "{role}",
                        input {
                            r#type: "color",
                            value: "{colour(&p.palette, &p.colours, role)}",
                            "aria-label": "{role}",
                            oninput: move |e| set_pref(ui, |p| { p.colours.insert(role.to_string(), e.value()); }),
                        }
                        "{role}"
                    }
                }
                if !p.colours.is_empty() {
                    button { class: "plain", onclick: move |_| set_pref(ui, |p| p.colours.clear()), "the palette's own" }
                }
            }
            label { r#for: "fs", "text size" }
            div { class: "row",
                input { id: "fs", r#type: "range", min: "11", max: "22", step: "0.5", value: "{p.font_size}",
                    oninput: move |e| set_pref(ui, |p| p.font_size = e.value().parse().unwrap_or(14.0)) }
                span { "{p.font_size}px" }
            }
            label { "zen" }
            div { class: "row",
                label { class: "row", input { r#type: "checkbox", checked: p.zen, onchange: move |e| set_pref(ui, |p| p.zen = e.checked()) } "fade the bars while you rest" }
                input { r#type: "range", min: "0.5", max: "10", step: "0.5", value: "{p.zen_delay}", "aria-label": "seconds before zen fades",
                    oninput: move |e| set_pref(ui, |p| p.zen_delay = e.value().parse().unwrap_or(2.5)) }
                span { "after {p.zen_delay}s" }
            }
            label { "the top bar" }
            label { class: "row", input { r#type: "checkbox", checked: p.hide_bar, onchange: move |e| set_pref(ui, |p| p.hide_bar = e.checked()) } "slide it away until the pointer comes near the top" }
            label { r#for: "zoom", "zoom" }
            div { class: "row",
                input { id: "zoom", r#type: "range", min: "0.5", max: "3", step: "0.05", value: "{p.zoom}",
                    oninput: move |e| set_pref(ui, |p| p.zoom = e.value().parse().unwrap_or(1.0)) }
                span { "{(p.zoom * 100.0).round()}%" }
                span { class: "dim small", "Ctrl+= / Ctrl+− or Ctrl+PageUp / PageDown; Ctrl+0 resets" }
            }
        }
    }
}

#[component]
fn MemorySettings() -> Element {
    let ui = ui();
    let p = ui.prefs.read().clone();
    rsx! {
        Fold { name: "memory", title: "the memory view and the space",
            label { r#for: "mfs", "texts' size" }
            div { class: "row",
                input { id: "mfs", r#type: "range", min: "12", max: "40", step: "1", value: "{p.memory_font}",
                    oninput: move |e| set_pref(ui, |p| p.memory_font = e.value().parse().unwrap_or(20.0)) }
                span { "{p.memory_font}px" }
            }
            label { r#for: "msp", "space between texts" }
            div { class: "row",
                input { id: "msp", r#type: "range", min: "0", max: "24", step: "1", value: "{p.spacing}",
                    oninput: move |e| set_pref(ui, |p| p.spacing = e.value().parse().unwrap_or(4.0)) }
                span { "{p.spacing}px" }
            }
            label { "the centre field" }
            div { class: "row",
                span { if p.field_width > 0.0 { "{p.field_width.round()}px wide" } else { "the whole width" } }
                button { class: "plain", onclick: move |_| set_pref(ui, |p| p.field_width = 0.0), "whole width" }
                span { class: "dim small", "or drag its edges" }
            }
            label { r#for: "sbg", "the space's background" }
            div { class: "row",
                input { id: "sbg", r#type: "color", value: "{p.space_bg}",
                    oninput: move |e| set_pref(ui, |p| p.space_bg = e.value()) }
                for (name , c) in [("sand", "#e4d4be"), ("night", "#14120f"), ("deep", "#0b1622"), ("paper", "#f4f1ea")] {
                    button { key: "{name}", class: "plain", onclick: move |_| set_pref(ui, |p| p.space_bg = c.to_string()), "{name}" }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
enum Step {
    Closed,
    /// the file read: its rows and a sample
    Read {
        target: String,
        data: String,
        rows: usize,
        sample: Vec<String>,
    },
    Importing,
    Done(Imported),
}

#[component]
fn Saves() -> Element {
    let ui = ui();
    let api = api();
    let mut reload = use_signal(|| 0);
    let api_r = api.clone();
    let saves = use_resource(move || {
        let api = api_r.clone();
        let _ = reload();
        let _ = (ui.changed)();
        async move { api.saves().await }
    });
    let mut name = use_signal(String::new);
    let mut made = use_signal(|| 0u64);
    let mut step = use_signal(|| Step::Closed);
    let mut armed = use_signal(|| None::<String>);
    let list: Vec<SaveInfo> = match &*saves.read() {
        Some(Ok(v)) => v.clone(),
        _ => vec![],
    };
    let act = move |f: crate::backend::Fut<()>, msg: String| {
        dioxus::dioxus_core::spawn_forever(async move {
            match f.await {
                Ok(()) => {
                    ui.say(msg);
                    reload += 1;
                    ui.touched();
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    rsx! {
        Fold { name: "saves", title: "saves",
            div { class: "wide",
                table { class: "saves",
                    thead { tr { th { "save" } th { "entries" } th { "size" } th { "" } } }
                    tbody {
                        for s in list {
                            tr { key: "{s.name}", class: if s.open { "open" } else { "" },
                                td { "{s.name}" if s.open { span { class: "dim", " · open" } } }
                                td { {s.memories.map(|m| m.to_string()).unwrap_or_default()} }
                                td { "{s.size / 1024} kB" }
                                td { class: "row",
                                    if !s.open {
                                        button { class: "plain", "data-tip": "open it", "aria-label": "open",
                                            onclick: { let api = api.clone(); let n = s.name.clone(); move |_| act(api.open_save(&n), format!("opened {n}")) },
                                            Icon { name: "open" } }
                                    }
                                    button { class: "plain", "data-tip": "a backup: its texts in one JSON, the rest in another", "aria-label": "backup",
                                        onclick: { let api = api.clone(); let n = s.name.clone(); move |_| {
                                            let api = api.clone(); let n = n.clone();
                                            dioxus::dioxus_core::spawn_forever(async move {
                                                match api.backup(&n).await {
                                                    Ok(b) if !b.paths.is_empty() => ui.say(format!("backed up: {}", b.paths.join(" and "))),
                                                    Ok(b) => {
                                                        let text = |v: &serde_json::Value| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                                                        download(&format!("{n} texts.json"), &text(&b.texts));
                                                        download(&format!("{n} other.json"), &text(&b.other));
                                                        ui.say("backed up: two downloads");
                                                    }
                                                    Err(e) => ui.fail(e),
                                                }
                                            });
                                        } },
                                        Icon { name: "download" } }
                                    button { class: "plain", "data-tip": "import weum's all-of-it.json into this save", "aria-label": "import",
                                        onclick: { let n = s.name.clone(); move |_| step.set(Step::Read { target: n.clone(), data: String::new(), rows: 0, sample: vec![] }) },
                                        Icon { name: "upload" } }
                                    button { class: "plain", "data-tip": "copy", "aria-label": "copy",
                                        onclick: { let api = api.clone(); let n = s.name.clone(); move |_| act(api.copy_save(&n, ""), format!("copied {n}")) },
                                        Icon { name: "copy" } }
                                    if !s.open {
                                        button {
                                            class: if armed() == Some(s.name.clone()) { "armed" } else { "plain" },
                                            "data-tip": "delete (click twice)",
                                            "aria-label": "delete",
                                            onclick: { let api = api.clone(); let n = s.name.clone(); move |_| {
                                                if armed() == Some(n.clone()) {
                                                    armed.set(None);
                                                    act(api.delete_save(&n), format!("deleted {n}"));
                                                } else {
                                                    armed.set(Some(n.clone()));
                                                }
                                            } },
                                            Icon { name: "delete" }
                                            if armed() == Some(s.name.clone()) { "delete?" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            label { r#for: "newsave", "a new save" }
            div { class: "row",
                for g in [made()] {
                    input { key: "{g}", id: "newsave", r#type: "text", placeholder: "its name", initial_value: "{name}", oninput: move |e| name.set(e.value()) }
                }
                button { onclick: { let api = api.clone(); move |_| { let n = name(); act(api.new_save(&n), format!("made and opened {n}")); name.set(String::new()); made += 1; } },
                    Icon { name: "new" } "make it" }
                button { class: "plain", onclick: move |_| step.set(Step::Read { target: String::new(), data: String::new(), rows: 0, sample: vec![] }),
                    Icon { name: "upload" } "a new save from all-of-it.json" }
            }
            if step() != Step::Closed {
                div { class: "wide import",
                    ImportSteps { step }
                }
            }
        }
    }
}

/// importing weum's all-of-it.json: choose the file, see what's in it, import, done
#[component]
fn ImportSteps(step: Signal<Step>) -> Element {
    let ui = ui();
    let api = api();
    let mut new_name = use_signal(|| "weum".to_string());
    let cur = step();
    let on_disk = api.on_disk();
    let mut path = use_signal(String::new);
    let target_p = match &cur {
        Step::Read { target, .. } => target.clone(),
        _ => String::new(),
    };
    let n = match &cur {
        Step::Read { rows: 0, .. } => 1,
        Step::Read { .. } => 2,
        Step::Importing => 3,
        _ => 4,
    };
    rsx! {
        div { class: "steps",
            for (i , label) in ["1 the file", "2 what's in it", "3 importing", "4 done"].into_iter().enumerate() {
                span { key: "{i}", class: if i + 1 == n { "on" } else { "" }, "{label}" }
            }
            span { class: "grow" }
            button { class: "plain ibtn", "aria-label": "close the import", onclick: move |_| step.set(Step::Closed), Icon { name: "close" } }
        }
        match cur {
            Step::Read { target, rows: 0, .. } => {
                let t2 = target.clone();
                rsx! {
                    div { class: "row",
                        "weum's all-of-it.json (or a backup's texts.json):"
                        input {
                            r#type: "file",
                            accept: ".json,application/json",
                            onchange: move |e: FormEvent| {
                                let target = target.clone();
                                async move {
                                    let Some(f) = e.files().into_iter().next() else { return };
                                    match f.read_string().await {
                                        Ok(data) => read_rows(ui, step, target, data),
                                        Err(e) => ui.fail(format!("couldn't read it: {e}")),
                                    }
                                }
                            },
                        }
                    }
                    if on_disk {
                        div { class: "row",
                            "or its path:"
                            input { r#type: "text", placeholder: "/home/you/all-of-it.json", style: "flex: 1;",
                                oninput: move |e| path.set(e.value()),
                                onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { read_path(ui, step, t2.clone(), path()) } }
                            button { onclick: move |_| read_path(ui, step, target_p.clone(), path()), "read it" }
                        }
                    }
                }
            }
            Step::Read { target, data, rows, sample } => rsx! {
                div { "{rows} rows. The first: " span { class: "dim", {sample.join(" · ")} } }
                div { class: "row",
                    if target.is_empty() {
                        "into a new save named"
                        input { r#type: "text", initial_value: "{new_name}", oninput: move |e| new_name.set(e.value()) }
                    } else {
                        "into “{target}”: keys already there are replaced (the old text kept as a version)"
                    }
                    button { class: "primary",
                        onclick: move |_| {
                            let api = api.clone();
                            let data = data.clone();
                            let into_new = if target.is_empty() { new_name() } else { String::new() };
                            let target = target.clone();
                            step.set(Step::Importing);
                            dioxus::dioxus_core::spawn_forever(async move {
                                // into a save that isn't open: open it first
                                if !target.is_empty()
                                    && target != ui.hello.peek().save
                                    && let Err(e) = api.open_save(&target).await
                                {
                                    ui.fail(e);
                                    step.set(Step::Closed);
                                    return;
                                }
                                match api.import_texts(&data, &into_new).await {
                                    Ok(im) => {
                                        step.set(Step::Done(im));
                                        ui.touched();
                                    }
                                    Err(e) => {
                                        ui.fail(e);
                                        step.set(Step::Closed);
                                    }
                                }
                            });
                        },
                        Icon { name: "upload" }
                        "import"
                    }
                }
            },
            Step::Importing => rsx! { div { "importing…" } },
            Step::Done(im) => rsx! {
                div { "imported {im.memories} texts" if let Some(s) = im.save { " into “{s}”" } "." }
            },
            Step::Closed => rsx! {},
        }
    }
}

/// a weum file's rows read: on to the step that shows what's in it
fn read_rows(ui: Ui, mut step: Signal<Step>, target: String, data: String) {
    match serde_json::from_str::<serde_json::Value>(&data) {
        Ok(serde_json::Value::Array(rows)) => {
            let sample = rows
                .iter()
                .take(8)
                .filter_map(|r| r.get(0).and_then(|t| t.as_str()).map(str::to_string))
                .collect();
            step.set(Step::Read {
                target,
                data,
                rows: rows.len(),
                sample,
            });
        }
        _ => ui.fail("that isn't a list of [text, [key bytes]] rows"),
    }
}

/// the desktop reads the file at a path typed in (when a file dialog isn't at hand)
fn read_path(ui: Ui, step: Signal<Step>, target: String, path: String) {
    #[cfg(not(target_arch = "wasm32"))]
    match std::fs::read_to_string(path.trim()) {
        Ok(data) => read_rows(ui, step, target, data),
        Err(e) => ui.fail(format!("couldn't read {}: {e}", path.trim())),
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (step, target, path);
        ui.fail("pick the file instead");
    }
}

/// a file handed to the viewer (the web build's backups)
fn download(name: &str, text: &str) {
    let js = format!(
        "const a = document.createElement('a'); a.href = URL.createObjectURL(new Blob([{}], {{type: 'application/json'}})); a.download = {}; a.click(); setTimeout(() => URL.revokeObjectURL(a.href), 4000);",
        serde_json::to_string(text).unwrap_or_default(),
        serde_json::to_string(name).unwrap_or_default()
    );
    document::eval(&js);
}

#[component]
fn Keys() -> Element {
    let rows = [
        ("Ctrl+1 … Ctrl+9", "the views"),
        ("Ctrl+,", "settings"),
        ("Ctrl+= / Ctrl+−, Ctrl+PageUp / PageDown, Ctrl+0", "zoom"),
        ("Ctrl+Z / Ctrl+Shift+Z in the bar", "undo and redo the bar's typing"),
        (
            "Ctrl+Z elsewhere",
            "take back the latest action (forget, edit, lists, …)",
        ),
        (
            "Enter",
            "gsearch (a number, or a text's sum) and the words; Enter outside the bar goes to it",
        ),
        ("Ctrl+Enter", "gsearch the text's sum, remember it and toast its key"),
        ("Shift+Enter", "bring back a key"),
        ("Delete in the bar", "forget the key typed"),
        ("O / R / F", "ordinal, reversed (R toggles), frequency"),
        ("G / S / L", "gsearch text, saturate, the sidecar's letters"),
        ("← / →", "gsearch the sidecar's first or last sum"),
        (
            "click / Ctrl / Shift / Alt + click",
            "into the bar / candidate / list / select",
        ),
        ("double click", "the key into the bar"),
        ("right click / Ctrl+right click", "the menu / gsearch ⌊√(s·r)⌋"),
        ("Esc", "close, cancel an edit or tag, leave the bar"),
    ];
    rsx! {
        Fold { name: "keys", title: "keys",
            div { class: "wide",
                table { class: "keys",
                    for (k , what) in rows {
                        tr { key: "{k}", td { kbd { "{k}" } } td { "{what}" } }
                    }
                }
            }
        }
    }
}

#[component]
fn Hub() -> Element {
    let ui = ui();
    let p = ui.prefs.read().clone();
    rsx! {
        Fold { name: "hub", title: "the hub",
            label { r#for: "hub", "its address" }
            input { id: "hub", r#type: "text", placeholder: "http://localhost:8080", initial_value: "{p.hub_url}",
                oninput: move |e| set_pref(ui, |p| p.hub_url = e.value()) }
            div { class: "wide dim small", "Sharing saves and models on the hub comes back here when the hub moves to the entry graph." }
        }
    }
}
