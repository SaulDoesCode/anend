//! Settings: a dialog that closes on a click outside it (or Esc), its sections folding
//! (remembered), separated by rules. Looks, the memory view, the space, the saves (open,
//! new, copy, rename, delete, a backup, and importing weum's all-of-it.json step by step),
//! the keys, and the hub (folded; absent without the `hub` feature).

use dioxus::prelude::*;

use crate::backend::{Imported, SaveInfo};
use crate::keys::{Action, Binding, Scope};
use crate::shell::set_pref;
use crate::theme::{PALETTES, ROLES, colour};
use crate::ui::*;
use crate::views::Fold;

#[component]
pub fn Settings() -> Element {
    let mut ui = ui();
    let mut forge = use_signal(|| false);
    if forge() {
        return rsx! { crate::views::forge::Forge { on_close: move |_| forge.set(false) } };
    }
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
                    button { class: "plain", "data-tip": "the forge: a save from other saves", onclick: move |_| forge.set(true), Icon { name: "forge" } "forge" }
                    a { class: "btn plain", href: "https://github.com/SaulDoesCode/anend/blob/main/docs/guide.md", target: "_blank", rel: "noopener", "data-tip": "the guide (opens in a browser)", Icon { name: "help" } "guide" }
                    button { class: "plain ibtn close tip-left", "data-tip": "close · Esc", "aria-label": "close",
                        onclick: move |_| ui.settings.set(false), Icon { name: "close" } }
                }
                Looks {}
                hr {}
                MemorySettings {}
                hr {}
                Saves {}
                hr {}
                GpuSettings {}
                hr {}
                LimitSettings {}
                hr {}
                Keys {}
                if cfg!(feature = "hub") {
                    hr {}
                    crate::views::hubui::Hub {}
                }
            }
        }
    }
}

#[component]
fn Looks() -> Element {
    let ui = ui();
    let p = ui.prefs.read().clone();
    // the role whose colour the hue / saturation / lightness editor holds
    let mut editing: Signal<Option<&'static str>> = use_signal(|| None);
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
            label { r#for: "hsl-role", "edit a colour" }
            div { class: "hsl-edit",
                select { id: "hsl-role", value: editing().unwrap_or(""),
                    onchange: move |e| { let v = e.value(); editing.set(ROLES.iter().map(|r| r.0).find(|r| *r == v)); },
                    option { value: "", "pick a role…" }
                    for (role, _) in ROLES {
                        option { key: "{role}", value: "{role}", selected: editing() == Some(role), "{role}" }
                    }
                }
                if let Some(role) = editing() {
                    HslEditor { key: "{role}", role, hex: colour(&p.palette, &p.colours, role) }
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
            label { "the bars" }
            label { class: "row", input { r#type: "checkbox", checked: p.bars_swapped, onchange: move |e| set_pref(ui, |p| p.bars_swapped = e.checked()) } "the top bar at the bottom, and the command bar at the top" }
            label { r#for: "tbh", "sizes" }
            div { class: "row",
                "top bar" input { id: "tbh", r#type: "range", min: "24", max: "64", step: "1", value: "{p.top_bar}", oninput: move |e| set_pref(ui, |p| p.top_bar = e.value().parse().unwrap_or(32.0)) } span { "{p.top_bar}px" }
                "command bar" input { r#type: "range", min: "24", max: "64", step: "1", value: "{p.bottom_bar}", "aria-label": "the command bar's height", oninput: move |e| set_pref(ui, |p| p.bottom_bar = e.value().parse().unwrap_or(32.0)) } span { "{p.bottom_bar}px" }
                button { class: "plain", onclick: move |_| set_pref(ui, |p| { p.top_bar = 32.0; p.bottom_bar = 32.0; p.font_size = 14.0; p.zoom = 1.0; }), "default sizes" }
            }
            label { r#for: "font", "fonts" }
            div { class: "row",
                input { id: "font", r#type: "text", list: "fontlist", placeholder: "Fira Sans (built in)", initial_value: "{p.font}", "aria-label": "the interface's font", oninput: move |e| set_pref(ui, |p| p.font = e.value()) }
                input { r#type: "text", list: "fontlist", placeholder: "the memory view: system serif", initial_value: "{p.memory_family}", "aria-label": "the memory view's font", oninput: move |e| set_pref(ui, |p| p.memory_family = e.value()) }
                datalist { id: "fontlist",
                    for f in ["Fira Sans", "Inter", "Noto Sans", "DejaVu Sans", "Liberation Sans", "Ubuntu", "Cantarell", "Georgia", "Times New Roman", "Liberation Serif", "DejaVu Serif", "Noto Serif", "Fira Mono", "DejaVu Sans Mono"] { option { key: "{f}", value: "{f}" } }
                }
                span { class: "dim small", "a font's name as the system knows it; empty for the built-in" }
            }
            label { "the editor" }
            div { class: "row",
                label { class: "row", input { r#type: "checkbox", checked: p.highlight, onchange: move |e| set_pref(ui, |p| p.highlight = e.checked()) } "colour the markdown (headings, emphasis, code, links, tags…)" }
                label { class: "row", input { r#type: "checkbox", checked: p.list_right, onchange: move |e| set_pref(ui, |p| p.list_right = e.checked()) } "the note list on the right" }
            }
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
            label { r#for: "pline", "long texts, shown piece by piece" }
            div { class: "row",
                input { id: "pline", r#type: "number", min: "10", max: "400", step: "1", value: "{p.piece_line}", style: "width: 5em;",
                    onchange: move |e| set_pref(ui, |p| p.piece_line = e.value().parse::<usize>().unwrap_or(80).clamp(10, 400)) }
                span { "characters a line," }
                input { r#type: "number", "aria-label": "lines a piece", min: "1", max: "200", step: "1", value: "{p.piece_lines}", style: "width: 4em;",
                    onchange: move |e| set_pref(ui, |p| p.piece_lines = e.value().parse::<usize>().unwrap_or(6).clamp(1, 200)) }
                span { "lines a piece" }
                span { class: "dim small", "a longer text shows its first piece, and opens into the rest" }
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

/// the open save's checkpoints (redb savepoints): made by hand with a name, gone back to
/// (dropping what changed since, and the checkpoints after it), deleted
#[component]
fn Checkpoints(on_back: EventHandler<()>) -> Element {
    let ui = ui();
    let api = api();
    let mut rev = use_signal(|| 0u64);
    let mut made = use_signal(|| 0u64);
    let mut armed: Signal<Option<u64>> = use_signal(|| None);
    let api_r = api.clone();
    let list = use_resource(move || {
        let api = api_r.clone();
        let _ = (rev(), (ui.changed)());
        async move { api.call("saves.checkpoints", serde_json::json!({})).await }
    });
    let cps: Vec<serde_json::Value> = match &*list.read() {
        Some(Ok(v)) => v.as_array().cloned().unwrap_or_default(),
        _ => vec![],
    };
    let call = move |op: &'static str, args: serde_json::Value, said: String, back: bool| {
        let api = api.clone();
        dioxus::dioxus_core::spawn_forever(async move {
            match api.call(op, args).await {
                Ok(_) => {
                    ui.say(said);
                    rev += 1;
                    if back {
                        on_back.call(());
                    }
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let when = |ms: u64| {
        chrono::DateTime::from_timestamp_millis(ms as i64)
            .map(|t| t.with_timezone(&chrono::Local).format("%d %b %Y, %H:%M").to_string())
            .unwrap_or_default()
    };
    let call2 = call.clone();
    rsx! {
        div { class: "checkpoints",
            span { class: "dim small", "checkpoints of this save" }
            if cps.is_empty() {
                span { class: "dim small", " · none yet" }
            }
            for c in cps {
                div { key: "{c[\"id\"]}", class: "row checkpoint",
                    Icon { name: "save" }
                    b { {c["name"].as_str().unwrap_or("").to_string()} }
                    span { class: "dim small", {when(c["at"].as_u64().unwrap_or(0))} }
                    span { class: "grow" }
                    button {
                        class: if armed() == c["id"].as_u64() { "armed" } else { "plain" },
                        "data-tip": "go back to it: what changed since goes, and the checkpoints made after it (click twice)",
                        onclick: { let call = call.clone(); let id = c["id"].as_u64().unwrap_or(0); let n = c["name"].as_str().unwrap_or("").to_string(); move |_| {
                            if armed() != Some(id) { armed.set(Some(id)); return; }
                            armed.set(None);
                            call("saves.restore_checkpoint", serde_json::json!({ "id": id }), format!("back to the checkpoint {n}"), true);
                        } },
                        Icon { name: "undo" }
                        if armed() == c["id"].as_u64() { "go back?" } else { "go back" }
                    }
                    button { class: "plain", "data-tip": "delete the checkpoint (the save stays as it is)", "aria-label": "delete the checkpoint",
                        onclick: { let call = call.clone(); let id = c["id"].as_u64().unwrap_or(0); move |_| call("saves.delete_checkpoint", serde_json::json!({ "id": id }), "checkpoint deleted".into(), false) },
                        Icon { name: "delete" } }
                }
            }
            div { class: "row",
                // not bound while typing (a round trip per key drops fast letters); a new
                // one after each checkpoint
                // read when the button is pressed: re-rendering on every key dropped letters
                // in the desktop's web view
                for g in [made()] {
                    input { key: "{g}", id: "cp-name", r#type: "text", placeholder: "a name (or the time)", "aria-label": "the checkpoint's name" }
                }
                button { onclick: move |_| {
                        let call2 = call2.clone();
                        spawn(async move {
                            let mut ev = document::eval("const i = document.getElementById('cp-name'); dioxus.send(i ? i.value : '');");
                            let n: String = ev.recv().await.unwrap_or_default();
                            call2("saves.checkpoint", serde_json::json!({ "name": n }), "checkpoint made".into(), false);
                            made += 1;
                        });
                    },
                    Icon { name: "save" } "a checkpoint now" }
            }
        }
    }
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
    let mut renaming: Signal<Option<(String, String)>> = use_signal(|| None);
    let list: Vec<SaveInfo> = match &*saves.read() {
        Some(Ok(v)) => v.clone(),
        _ => vec![],
    };
    let on_disk = api.on_disk();
    let api_k = use_signal(|| api.clone());
    // the desktop opens the save it had open last (`prefs.save`), so its folder too
    let keep = move || {
        if on_disk {
            let path = api_k.peek().describe();
            set_pref(ui, |p| p.save = Some(path));
        }
    };
    let act = move |f: crate::backend::Fut<()>, msg: String| {
        dioxus::dioxus_core::spawn_forever(async move {
            match f.await {
                Ok(()) => {
                    ui.say(msg);
                    keep();
                    reload += 1;
                    ui.touched();
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let api_f = api.clone();
    let folder = use_resource(move || {
        let api = api_f.clone();
        let _ = reload();
        async move { api.call("saves.folder", serde_json::json!({})).await }
    });
    let folder_now = match &*folder.read() {
        Some(Ok(v)) if v["can"].as_bool() == Some(true) => v["folder"].as_str().map(str::to_string),
        _ => None,
    };
    let mut folder_text = use_signal(String::new);
    let mut folder_gen = use_signal(|| 0u64);
    let use_folder = {
        let api = api.clone();
        move || {
            let api = api.clone();
            let path = folder_text();
            dioxus::dioxus_core::spawn_forever(async move {
                match api.call("saves.folder", serde_json::json!({ "path": path })).await {
                    Ok(v) => {
                        ui.say(format!(
                            "saves from {}: opened {}",
                            v["folder"].as_str().unwrap_or(""),
                            v["save"].as_str().unwrap_or("")
                        ));
                        keep();
                        folder_text.set(String::new());
                        folder_gen += 1;
                        reload += 1;
                        ui.touched();
                        ui.refresh(api);
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    rsx! {
        Fold { name: "saves", title: "saves",
            if let Some(f) = folder_now {
                label { r#for: "savesfolder", "saves folder" }
                div { class: "row",
                    for g in [folder_gen()] {
                        input { key: "{g}", id: "savesfolder", r#type: "text", style: "flex: 1; min-width: 12em;",
                            placeholder: "another folder, or a .redb file",
                            oninput: move |e| folder_text.set(e.value()),
                            onkeydown: { let u = use_folder.clone(); move |e: KeyboardEvent| if e.key() == Key::Enter { u() } } }
                    }
                    button { disabled: folder_text().trim().is_empty(), onclick: { let u = use_folder.clone(); move |_| u() },
                        Icon { name: "folder" } "use it" }
                    span { class: "dim small", "now {f}. A folder lists its saves and opens the first (or makes a main); a .redb file opens that save, and its folder becomes the saves folder." }
                }
            }
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
                                    button { class: "plain", "data-tip": "rename it", "aria-label": "rename",
                                        onclick: { let n = s.name.clone(); move |_| renaming.set(Some((n.clone(), n.clone()))) },
                                        Icon { name: "rename" } }
                                    button { class: "plain", "data-tip": "export a bundle (.anend.json): keys, lists, notes, classifiers and all", "aria-label": "export a bundle",
                                        onclick: { let api = api.clone(); let n = s.name.clone(); move |_| {
                                            let (api, n) = (api.clone(), n.clone());
                                            dioxus::dioxus_core::spawn_forever(async move {
                                                match api.call("saves.export", serde_json::json!({ "name": n })).await {
                                                    Ok(b) => { download(&format!("{n}.anend.json"), &b.to_string()); ui.say(format!("{n}.anend.json: a download")); }
                                                    Err(e) => ui.fail(e),
                                                }
                                            });
                                        } },
                                        Icon { name: "export" } }
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
                            if s.open {
                                tr { key: "{s.name} checkpoints", class: "checkpoints-row",
                                    td { colspan: "4", Checkpoints { on_back: move |_| { reload += 1; ui.touched(); } } }
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
            if let Some((from, to)) = renaming() {
                label { "rename “{from}”" }
                div { class: "row",
                    input { r#type: "text", initial_value: "{to}", "aria-label": "its new name", oninput: move |e| { let f = from.clone(); renaming.set(Some((f, e.value()))) } }
                    button { class: "primary", onclick: { let api = api.clone(); move |_| {
                            if let Some((f, t)) = renaming() { act(api.rename_save(&f, &t), format!("renamed {f} to {t}")); }
                            renaming.set(None);
                        } }, "rename" }
                    button { class: "plain", onclick: move |_| renaming.set(None), "cancel" }
                }
            }
            label { "a bundle" }
            div { class: "row",
                label { class: "button btn", Icon { name: "upload" } "import a bundle (.anend.json) as a new save"
                    input { r#type: "file", accept: ".json,.anend.json", style: "display: none;",
                        onchange: { let api = api.clone(); move |e: FormEvent| {
                            let api = api.clone();
                            let files = e.files();
                            spawn(async move {
                                let Some(f) = files.into_iter().next() else { return };
                                let text = match f.read_string().await { Ok(t) => t, Err(e) => return ui.fail(e.to_string()) };
                                let Ok(b) = serde_json::from_str::<serde_json::Value>(&text) else { return ui.fail("that isn't a bundle (JSON)") };
                                match api.import_bundle(b, "", false).await {
                                    Ok(im) => { ui.say(format!("imported {} texts{}", im.memories, im.save.map(|s| format!(" into “{s}”")).unwrap_or_default())); reload += 1; ui.touched(); }
                                    Err(e) => ui.fail(e),
                                }
                            });
                        } },
                    }
                }
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

/// what a `reading` answer says, in words
fn reading_words(r: &serde_json::Value) -> String {
    let dev = match r["device"].as_str() {
        Some("gpu") => "the GPU",
        Some("gpu_fast") => "the GPU (fast)",
        Some("cpu") => "the CPU",
        _ => return String::new(),
    };
    let mut s = format!("now on {dev}");
    if let Some(g) = r["gpu"].as_str() {
        s += &format!(", {g}");
    }
    if r["device"] != "cpu" {
        s += if r["exact"] == true {
            ", the CPU's results bit for bit"
        } else {
            ", within a small difference of the CPU's results"
        };
    }
    if let Some(n) = r["note"].as_str() {
        s += &format!(": {n}");
    }
    s
}

/// the limits the open save sets for itself (`limits.*`), each a number with its default;
/// none caps how long a text can be
/// a colour as hue, saturation and lightness sliders and a hex box (the Bevy app's), each
/// change kept as the role's own colour
#[component]
fn HslEditor(role: &'static str, hex: String) -> Element {
    let ui = ui();
    let (h, s, l) = crate::theme::hsl_of(&hex);
    let set = move |h: f64, s: f64, l: f64| {
        let c = crate::theme::hex_of_hsl(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        set_pref(ui, move |p| {
            p.colours.insert(role.to_string(), c.clone());
        });
    };
    let mut draft = use_signal(|| hex.clone());
    rsx! {
        div { class: "hsl-sliders",
            span { class: "hsl-swatch", style: "background: {hex};" }
            label { span { "hue" }
                input { r#type: "range", min: "0", max: "359", step: "1", value: "{h.round()}", class: "hue",
                    oninput: move |e| set(e.value().parse().unwrap_or(h), s, l) }
                span { class: "num", "{h.round()}°" } }
            label { span { "saturation" }
                input { r#type: "range", min: "0", max: "100", step: "1", value: "{(s * 100.0).round()}",
                    oninput: move |e| set(h, e.value().parse::<f64>().unwrap_or(s * 100.0) / 100.0, l) }
                span { class: "num", "{(s * 100.0).round()}%" } }
            label { span { "lightness" }
                input { r#type: "range", min: "0", max: "100", step: "1", value: "{(l * 100.0).round()}",
                    oninput: move |e| set(h, s, e.value().parse::<f64>().unwrap_or(l * 100.0) / 100.0) }
                span { class: "num", "{(l * 100.0).round()}%" } }
            label { span { "hex" }
                input { r#type: "text", value: "{hex}", "aria-label": "{role} as hex", style: "width: 7em;",
                    oninput: move |e| draft.set(e.value()),
                    onchange: move |_| {
                        let v = draft().trim().to_string();
                        let v = if v.starts_with('#') { v } else { format!("#{v}") };
                        if v.len() == 7 && v[1..].chars().all(|c| c.is_ascii_hexdigit()) {
                            set_pref(ui, move |p| { p.colours.insert(role.to_string(), v.to_lowercase()); });
                        } else {
                            ui.fail("a colour in hex is like #a07850");
                        }
                    } } }
        }
    }
}

#[component]
fn LimitSettings() -> Element {
    let ui = ui();
    let api = api();
    let mut rows = use_signal(Vec::<serde_json::Value>::new);
    {
        let api = api.clone();
        use_hook(move || {
            spawn(async move {
                if let Ok(v) = api.call("limits.list", serde_json::json!({})).await {
                    rows.set(v.as_array().cloned().unwrap_or_default());
                }
            });
        });
    }
    rsx! {
        Fold { name: "limits", title: "limits of this save",
            div { class: "wide dim small", "Limits the save sets for itself, with their defaults. Empty puts a default back. None of them limits how long a text can be." }
            for r in rows() {
                {
                    let name = r["name"].as_str().unwrap_or("").to_string();
                    let label = name.trim_start_matches("limits.").trim_start_matches("graph.").replace('_', " ");
                    let id = format!("lim-{}", name.replace('.', "-"));
                    let api = api.clone();
                    let n2 = name.clone();
                    let value = r["value"].to_string();
                    rsx! {
                        label { key: "{name}", r#for: "{id}", "{label}" }
                        div { class: "row",
                            input { id: "{id}", r#type: "number", min: "0", step: "1", style: "width: 9em;",
                                value: "{value}",
                                onchange: move |e| {
                                    let (api, n) = (api.clone(), n2.clone());
                                    let v = e.value();
                                    spawn(async move {
                                        match api.call("limits.set", serde_json::json!({ "name": n, "value": v })).await {
                                            Ok(_) => {
                                                if let Ok(v) = api.call("limits.list", serde_json::json!({})).await {
                                                    rows.set(v.as_array().cloned().unwrap_or_default());
                                                }
                                                ui.say(format!("{n} set"));
                                            }
                                            Err(e) => ui.fail(e),
                                        }
                                    });
                                }
                            }
                            span { class: "dim small", {format!("{} (default {})", r["about"].as_str().unwrap_or(""), r["default"])} }
                        }
                    }
                }
            }
        }
    }
}

/// the GPU: where classifiers read texts (predict, search, indexing, buckets, tints),
/// and a report on this machine's GPU for the user to send back
#[component]
fn GpuSettings() -> Element {
    let ui = ui();
    let api = api();
    let mut state = use_signal(|| serde_json::Value::Null);
    let mut busy = use_signal(|| false);
    let mut written = use_signal(String::new);
    {
        let api = api.clone();
        use_hook(move || {
            spawn(async move {
                if let Ok(v) = api.call("classify.device", serde_json::json!({})).await {
                    state.set(v);
                }
            });
        });
    }
    let choice = ui.prefs.read().read_device.clone();
    let mut now = reading_words(&state.read()["reading"]);
    if choice == "auto" && state.read()["reading"]["device"] == "gpu" {
        // auto's answer depends on the model's size, measured on its first reading
        now = format!(
            "the GPU here, {}, gives the CPU's results bit for bit; auto reads on it with the model sizes it is faster for, measured on each size's first reading",
            state.read()["reading"]["gpu"].as_str().unwrap_or("")
        );
    }
    let speeds: Vec<String> = state.read()["speeds"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s["words"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let again = {
        let api = api.clone();
        move |_| {
            let api = api.clone();
            spawn(async move {
                match api
                    .call("classify.device", serde_json::json!({ "measure_again": true }))
                    .await
                {
                    Ok(r) => {
                        state.set(r);
                        ui.say("each model size is measured again the next time it reads");
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let set = {
        let api = api.clone();
        move |e: Event<FormData>| {
            let v = e.value();
            set_pref(ui, |p| p.read_device = v.clone());
            let api = api.clone();
            spawn(async move {
                match api.call("classify.device", serde_json::json!({ "device": v })).await {
                    Ok(r) => state.set(r),
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let report = move |_| {
        let api = api.clone();
        busy.set(true);
        ui.say("measuring the GPU and the CPU; this takes some minutes (about twenty on a software renderer)");
        spawn(async move {
            let r = api.call("gpu.report", serde_json::json!({})).await;
            busy.set(false);
            let r = match r {
                Ok(r) => r,
                Err(e) => return ui.fail(e),
            };
            let text = serde_json::to_string_pretty(&r).unwrap_or_default();
            let secs = (now_ms() / 1000.0) as u64;
            let file = format!("gpu-report-{secs}.json");
            #[cfg(not(target_arch = "wasm32"))]
            {
                let dir = crate::prefs::data_dir().join("exports");
                let path = dir.join(&file);
                match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &text)) {
                    Ok(()) => {
                        written.set(format!("written to {}", path.display()));
                        ui.say(format!("the GPU report is in {}", path.display()));
                    }
                    Err(e) => ui.fail(format!("couldn't write {}: {e}", path.display())),
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                download(&file, &text);
                written.set(format!("downloaded as {file}"));
                ui.say(format!("the GPU report downloaded as {file}"));
            }
        });
    };
    rsx! {
        Fold { name: "gpu", title: "the GPU",
            label { r#for: "readdev", "classifiers read texts on" }
            div { class: "row",
                select { id: "readdev", onchange: set,
                    option { value: "auto", selected: choice == "auto", "auto: the GPU when it gives the CPU's results and is faster" }
                    option { value: "cpu", selected: choice == "cpu", "the CPU" }
                    option { value: "gpu", selected: choice == "gpu", "the GPU" }
                    option { value: "gpu_fast", selected: choice == "gpu_fast", "the GPU, fast" }
                }
                span { class: "dim small", "{now}" }
            }
            if !speeds.is_empty() {
                label { "measured on this machine" }
                div { class: "wide small",
                    for w in speeds.iter() {
                        div { "{w}" }
                    }
                }
                div { class: "row",
                    button { onclick: again, Icon { name: "refresh" } "measure again" }
                    span { class: "dim small", "forgets these rates; each model size is measured again on its next reading" }
                }
            }
            div { class: "wide dim small",
                "Predicting, classifier search, indexing, buckets, colouring by a classifier and filters by one. Training picks its device on the classifier's page."
            }
            label { "a report on this machine's GPU" }
            div { class: "row",
                button { disabled: busy(), onclick: report, Icon { name: "stats" } if busy() { "measuring…" } else { "write a GPU report" } }
                span { class: "dim small", "the adapter and driver, its limits, whether every kernel gives the CPU's bits, and timings of a fixed training run and of reading up to 10 000 texts on the CPU and the GPU (two minutes at most on each); it takes some minutes, about twenty on a software renderer" }
            }
            if !written().is_empty() {
                div { class: "wide small", "{written} — paste the file back to report how the GPU does" }
            }
        }
    }
}

/// a file handed to the viewer (the web build's backups)
pub fn download(name: &str, text: &str) {
    let js = format!(
        "const a = document.createElement('a'); a.href = URL.createObjectURL(new Blob([{}], {{type: 'application/json'}})); a.download = {}; a.click(); setTimeout(() => URL.revokeObjectURL(a.href), 4000);",
        serde_json::to_string(text).unwrap_or_default(),
        serde_json::to_string(name).unwrap_or_default()
    );
    document::eval(&js);
}

/// the keys menu's answer: the key pressed while it waited for one
pub fn rebind(mut ui: Ui, a: Action, k: &KeyPress) {
    ui.listening.set(None);
    if k.key == "Escape" {
        ui.say("the key stays as it was");
        return;
    }
    let Some(b) = Binding::of(&k.code, k.ctrl, k.shift, k.alt) else {
        ui.fail(format!(
            "{} can't be bound",
            if k.code.is_empty() { &k.key } else { &k.code }
        ));
        return;
    };
    let mut map = ui.keymap();
    let other = map.rebind(a, b);
    set_pref(ui, |p| p.keymap = map.changes());
    match other {
        Some(o) => ui.say(format!(
            "{} is {b} now; {} takes {} instead",
            a.label(),
            o.label(),
            map.get(o)
        )),
        None => ui.say(format!("{} is {b} now", a.label())),
    }
}

/// every action and its key: click a key, then press the new one (Esc leaves it); a key
/// another action holds where both work is swapped with it; kept in the settings file
#[component]
fn Keys() -> Element {
    let mut ui = ui();
    let map = crate::keys::Keymap::from_changes(&ui.prefs.read().keymap);
    let listening = (ui.listening)();
    let changed = !ui.prefs.read().keymap.is_empty();
    let mut groups: Vec<(Scope, Vec<Action>)> = Vec::new();
    for a in Action::ALL {
        match groups.iter_mut().find(|(s, _)| *s == a.scope()) {
            Some((_, v)) => v.push(a),
            None => groups.push((a.scope(), vec![a])),
        }
    }
    let rows = [
        ("Ctrl+Z / Ctrl+Shift+Z in the bar", "undo and redo the bar's typing"),
        (
            "Enter in the bar",
            "the view's search; Ctrl+Enter saturates, Shift+Enter brings back a key",
        ),
        ("Delete in the bar", "forget the key typed"),
        ("↑ ↓ in the space's bar", "its history"),
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
            div { class: "wide dim small",
                "Click a key, then press the new one (with Ctrl, Alt or Shift if you like); Esc leaves it. A key another action holds is swapped with it. Keys without Ctrl or Alt only act while you're not typing in a text box."
            }
            div { class: "wide keymap",
                for (scope , acts) in groups {
                    div { key: "{scope.name()}", class: "keygroup",
                        h4 { "{scope.name()}" }
                        for a in acts {
                            div { key: "{a.id()}", class: "keyrow",
                                span { class: "what", "{a.label()}" }
                                button {
                                    class: if listening == Some(a) { "keycap on" } else if map.get(a) != a.default_binding() { "keycap changed" } else { "keycap" },
                                    "aria-label": "the key for {a.label()}",
                                    onclick: move |_| ui.listening.set(Some(a)),
                                    if listening == Some(a) { "press a key…" } else { "{map.get(a)}" }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "wide row",
                button { disabled: !changed, onclick: move |_| { set_pref(ui, |p| p.keymap.clear()); ui.say("every key is back to its default"); }, Icon { name: "undo" } "reset to defaults" }
            }
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
