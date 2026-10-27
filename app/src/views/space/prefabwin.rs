//! The prefabs window: subassemblies (core's `graph::prefabs`, `ops::prefabs`). Pick blocks
//! in the room and group them into a prefab (their textures, turns, wirings, animations
//! and the links between them go with it); the menu shows the save's prefabs with a
//! picture of each, to place a copy ahead of you, rename or delete. A part of a copy,
//! selected, shows its copy here: move, turn and size the whole copy, take its own changes
//! back, make them the prefab's (every copy following, or the others kept as they were),
//! detach it into ordinary blocks, or remove it. A part moved, resized or retextured by
//! hand changes in its copy only.

use std::collections::HashMap;

use dioxus::prelude::*;
use serde_json::{Value, json};

use super::{Sp, Win, dress, load_blocks};
use crate::shell::set_pref;
use crate::ui::*;

/// the save's prefabs, and their pictures drawn by the room
/// whether an id is a prefab's copy (not one of its parts)
pub(super) fn is_copy(id: &str) -> bool {
    id.starts_with("prefab:") && !id.contains('/')
}

/// the selected copy moved or turned in the room (a drag, or an undo of one)
pub(super) fn copy_set(ui: Ui, api: Api, sp: Sp, data: Value) {
    dioxus::dioxus_core::spawn_forever(async move {
        match api.call("prefabs.set", data).await {
            Ok(r) => {
                show(ui, &api, sp, &r).await;
                load_prefabs(ui, &api, sp).await;
            }
            Err(e) => ui.fail(e),
        }
    });
}

/// a removed copy put back (an undo)
pub(super) fn copy_restore(ui: Ui, api: Api, sp: Sp, restore: Value) {
    dioxus::dioxus_core::spawn_forever(async move {
        match api.call("prefabs.place", restore).await {
            Ok(r) => {
                show(ui, &api, sp, &r).await;
                load_prefabs(ui, &api, sp).await;
                sp.send(json!({ "cmd": "pickcopy", "id": r["copy"]["id"] }));
                ui.say("the copy is back");
            }
            Err(e) => ui.fail(e),
        }
    });
}

/// the selected copy taken away, as a whole (Ctrl+Z puts it back)
pub(super) fn copy_remove(ui: Ui, api: Api, sp: Sp, id: String) {
    dioxus::dioxus_core::spawn_forever(async move {
        match api.call("prefabs.remove", json!({ "id": id })).await {
            Ok(r) => {
                sp.send(json!({ "cmd": "copygone", "ids": r["removed"], "restore": r["restore"] }));
                let mut b = sp.block;
                b.set(None);
                load_prefabs(ui, &api, sp).await;
                ui.say("the copy is gone · Ctrl+Z puts it back");
            }
            Err(e) => ui.fail(e),
        }
    });
}

/// another copy beside the selected one, with its own values
pub(super) fn copy_duplicate(ui: Ui, api: Api, sp: Sp, id: String) {
    dioxus::dioxus_core::spawn_forever(async move {
        match api.call("prefabs.duplicate", json!({ "id": id })).await {
            Ok(r) => {
                show(ui, &api, sp, &r).await;
                load_prefabs(ui, &api, sp).await;
                sp.send(json!({ "cmd": "pickcopy", "id": r["copy"]["id"] }));
                ui.say("a copy of the copy, beside it, with its own changes");
            }
            Err(e) => ui.fail(e),
        }
    });
}

pub(super) async fn load_prefabs(ui: Ui, api: &Api, sp: Sp) {
    let r = match api.call("prefabs.list", json!({})).await {
        Ok(r) => r,
        // a hub from before prefabs
        Err(_) => return,
    };
    let list = r["prefabs"].as_array().cloned().unwrap_or_default();
    let textures = dress(ui, api, r["textures"].clone()).await;
    let want: Vec<Value> = list
        .iter()
        .map(|p| json!({ "id": p["id"], "parts": p["parts"] }))
        .collect();
    sp.send(json!({ "cmd": "thumbs", "prefabs": want, "textures": textures }));
    let mut l = sp.prefabs;
    l.set(list);
    let copies = api
        .call("prefabs.copies", json!({}))
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    sp.send(json!({ "cmd": "copies", "list": copies }));
    let mut c = sp.copies;
    c.set(copies);
}

/// a new copy of a prefab where the room says (ahead of you, or at the selected text)
pub(super) fn place_at(ui: Ui, api: Api, sp: Sp, data: Value) {
    dioxus::dioxus_core::spawn_forever(async move {
        let args = json!({ "prefab": data["want"], "x": data["x"], "y": data["y"], "z": data["z"] });
        match api.call("prefabs.place", args).await {
            Ok(r) => {
                show(ui, &api, sp, &r).await;
                load_prefabs(ui, &api, sp).await;
                sp.send(json!({ "cmd": "pickcopy", "id": r["copy"]["id"] }));
                ui.say("a copy, selected as one: drag it to move it, its rings turn it · double-click a part to edit the parts");
            }
            Err(e) => ui.fail(e),
        }
    });
}

/// a copy just where the hand put it (the ghost), turned as it was
pub(super) fn place_exactly(ui: Ui, api: Api, sp: Sp, data: Value) {
    dioxus::dioxus_core::spawn_forever(async move {
        let args =
            json!({ "prefab": data["prefab"], "x": data["x"], "y": data["y"], "z": data["z"], "rot": data["rot"] });
        match api.call("prefabs.place", args).await {
            Ok(r) => {
                show(ui, &api, sp, &r).await;
                load_prefabs(ui, &api, sp).await;
            }
            Err(e) => ui.fail(e),
        }
    });
}

/// a prefab to place by hand: its ghost follows where you look
async fn ghost(ui: Ui, api: &Api, sp: Sp, p: &Value) {
    let r = match api.call("prefabs.list", json!({})).await {
        Ok(r) => r,
        Err(e) => return ui.fail(e),
    };
    let textures = dress(ui, api, r["textures"].clone()).await;
    sp.send(json!({ "cmd": "ghost", "prefab": p["id"], "parts": p["parts"], "textures": textures, "name": p["name"] }));
    ui.say(format!(
        "placing {}: it follows where you look · the wheel turns it · click places · Shift: off the grid · Esc stops",
        p["name"].as_str().unwrap_or("")
    ));
}

/// blocks an answer brings, into the room
async fn show(ui: Ui, api: &Api, sp: Sp, r: &Value) {
    if r["removed"].as_array().is_some_and(|a| !a.is_empty()) {
        sp.send(json!({ "cmd": "unblock", "ids": r["removed"] }));
    }
    if r["blocks"].as_array().is_some_and(|a| !a.is_empty()) {
        let textures = dress(ui, api, r["textures"].clone()).await;
        sp.send(json!({ "cmd": "blocks", "blocks": r["blocks"], "textures": textures }));
    }
}

fn short(id: &str) -> String {
    // a prefab copy's part: its address inside, and which copy
    if let Some((copy, inner)) = id.strip_prefix("prefab:").and_then(|r| r.split_once('/')) {
        let n = copy.rsplit_once('#').map_or(copy, |x| x.1);
        return format!("{} in copy {n}", short(inner));
    }
    let (k, n) = id.rsplit_once('#').unwrap_or((id, ""));
    let k = k.split_once(':').map_or(k, |(_, t)| t);
    let k = if k.chars().count() > 14 {
        format!("{}…", k.chars().take(12).collect::<String>())
    } else {
        k.to_string()
    };
    if n.is_empty() { k } else { format!("{k} #{n}") }
}

#[component]
pub(super) fn Prefabs(sp: Sp) -> Element {
    let ui = ui();
    let api = api();
    let mut grouping = use_signal(|| false);
    let mut group: Signal<Vec<String>> = use_signal(Vec::new);
    let mut name = use_signal(String::new);
    let mut renaming: Signal<Option<u64>> = use_signal(|| None);
    let mut rename_to = use_signal(String::new);
    let mut deleting: Signal<Option<u64>> = use_signal(|| None);
    let mut busy = use_signal(|| false);

    let api_l = api.clone();
    use_hook(move || {
        let api = api_l.clone();
        spawn(async move { load_prefabs(ui, &api, sp).await });
    });

    let sel = (sp.block)();
    let sel_id = sel.as_ref().and_then(|b| b["id"].as_str().map(str::to_string));
    // picking blocks to group: each one selected joins (a copy's parts can't, yet)
    use_effect(use_reactive!(|sel_id| {
        let Some(id) = sel_id else { return };
        if !*grouping.peek() || id.starts_with("prefab:") {
            return;
        }
        if !group.peek().contains(&id) {
            group.write().push(id);
        }
    }));
    let copy_id = sel.as_ref().and_then(|b| b["part_of"].as_str().map(str::to_string));
    let list = (sp.prefabs)();
    let thumbs: HashMap<u64, String> = (sp.thumbs)();
    let copy = copy_id.as_ref().and_then(|c| {
        let scene: u64 = c.strip_prefix("prefab:")?.split('#').next()?.parse().ok()?;
        list.iter()
            .find(|p| p["id"].as_u64() == Some(scene))
            .cloned()
            .map(|p| (c.clone(), p))
    });

    // an op on the selected copy; `reload`: the room reads every block again after
    let api_c = api.clone();
    let on_copy = use_callback(
        move |(op, args, reload, said): (&'static str, Value, bool, &'static str)| {
            let api = api_c.clone();
            busy.set(true);
            spawn(async move {
                match api.call(op, args).await {
                    Ok(r) => {
                        if reload {
                            load_blocks(ui, api.clone(), sp).await;
                        } else {
                            show(ui, &api, sp, &r).await;
                        }
                        load_prefabs(ui, &api, sp).await;
                        super::wirewin::load_wires(&api, sp).await;
                        if !said.is_empty() {
                            ui.say(said.to_string());
                        }
                    }
                    Err(e) => ui.fail(e),
                }
                busy.set(false);
            });
        },
    );
    let api_m = api.clone();
    let make = move |_| {
        let ids = group.peek().clone();
        let n = name.peek().trim().to_string();
        let api = api_m.clone();
        spawn(async move {
            match api.call("prefabs.make", json!({ "name": n, "ids": ids })).await {
                Ok(_) => {
                    group.set(Vec::new());
                    grouping.set(false);
                    name.set(String::new());
                    load_blocks(ui, api.clone(), sp).await;
                    load_prefabs(ui, &api, sp).await;
                    super::wirewin::load_wires(&api, sp).await;
                    ui.say("a prefab · its copy stands where the blocks were; place more from the menu".to_string());
                }
                Err(e) => ui.fail(e),
            }
        });
    };

    rsx! {
        Win {
            id: "prefabs",
            at: (8.0, 160.0),
            on_close: move |_| { grouping.set(false); set_pref(ui, |p| p.space.show_prefabs = false) },
            title: rsx! {
                span { class: "win-title",
                    Icon { name: "prefab" }
                    "prefabs"
                    span { class: "dim small", style: "font-weight: 400;", "grouped blocks, placed as copies" }
                }
            },
            if let Some((cid, p)) = copy.clone() {
                {
                    let c = cid.clone();
                    let own = (sp.copies)().iter().find(|x| x["id"].as_str() == Some(c.as_str())).and_then(|x| x["own"].as_u64()).unwrap_or(0);
                    let pname = p["name"].as_str().unwrap_or("").to_string();
                    let copies = p["copies"].as_u64().unwrap_or(1);
                    let mv = |by: [f64; 3]| { let c = c.clone(); move |_| on_copy.call(("prefabs.set", json!({ "id": c, "by": by }), false, "")) };
                    rsx! {
                        div { class: "wire-sec pf-copy",
                            span { class: "small", "the selected part's copy of " b { "{pname}" } }
                            div { class: "row wrap",
                                span { class: "small", "move" }
                                button { class: "btn small", title: "along −x", onclick: mv([-1.0, 0.0, 0.0]), "←" }
                                button { class: "btn small", title: "along +x", onclick: mv([1.0, 0.0, 0.0]), "→" }
                                button { class: "btn small", title: "along −z (away)", onclick: mv([0.0, 0.0, -1.0]), "↑" }
                                button { class: "btn small", title: "along +z (near)", onclick: mv([0.0, 0.0, 1.0]), "↓" }
                                button { class: "btn small", title: "up", onclick: mv([0.0, 1.0, 0.0]), "up" }
                                button { class: "btn small", title: "down", onclick: mv([0.0, -1.0, 0.0]), "down" }
                            }
                            div { class: "row wrap",
                                span { class: "small", "turn" }
                                button { class: "btn small", title: "a quarter turn to the left", onclick: { let c = c.clone(); move |_| on_copy.call(("prefabs.set", json!({ "id": c, "turn": 1 }), false, "")) }, "⟲" }
                                button { class: "btn small", title: "a quarter turn to the right", onclick: { let c = c.clone(); move |_| on_copy.call(("prefabs.set", json!({ "id": c, "turn": -1 }), false, "")) }, "⟳" }
                                span { class: "small", "size" }
                                {
                                    let sc = (sp.copies)().iter().find(|x| x["id"].as_str() == Some(c.as_str())).and_then(|x| x["scale"].as_f64()).unwrap_or(1.0);
                                    let (c1, c2) = (c.clone(), c.clone());
                                    rsx! {
                                        button { class: "btn small", onclick: move |_| on_copy.call(("prefabs.set", json!({ "id": c1, "scale": sc / 1.25 }), false, "")), "−" }
                                        span { class: "dim small", {format!("×{sc:.2}")} }
                                        button { class: "btn small", onclick: move |_| on_copy.call(("prefabs.set", json!({ "id": c2, "scale": sc * 1.25 }), false, "")), "+" }
                                    }
                                }
                            }
                            if own > 0 {
                                span { class: "small", {format!("its own changes to {own} part{} (moved, resized, retextured or rewired here)", if own == 1 { "" } else { "s" })} }
                                div { class: "row wrap",
                                    button { class: "btn small", title: "it shows the prefab as it is again",
                                        onclick: { let c = c.clone(); move |_| on_copy.call(("prefabs.reset", json!({ "id": c }), true, "back to the prefab")) }, "take them back" }
                                    button { class: "btn small", disabled: busy(), title: "the prefab becomes this copy; every copy shows it",
                                        onclick: { let c = c.clone(); move |_| on_copy.call(("prefabs.apply", json!({ "id": c, "others": true }), true, "the prefab changed; every copy follows")) }, "make them the prefab's" }
                                    if copies > 1 {
                                        button { class: "btn small", disabled: busy(), title: "the prefab becomes this copy; the other copies keep it as it was (a prefab of its own)",
                                            onclick: { let c = c.clone(); move |_| on_copy.call(("prefabs.apply", json!({ "id": c, "others": false }), true, "the prefab changed; the other copies kept it as it was")) }, "…but not the other copies" }
                                    }
                                }
                            }
                            div { class: "row wrap",
                                button { class: "btn small", title: "its parts become ordinary blocks; it stops following the prefab",
                                    onclick: { let c = c.clone(); move |_| on_copy.call(("prefabs.detach", json!({ "id": c }), true, "detached: ordinary blocks now")) }, "detach" }
                                button { class: "btn small",
                                    onclick: { let c = c.clone(); move |_| on_copy.call(("prefabs.remove", json!({ "id": c }), false, "copy removed")) }, "remove this copy" }
                            }
                        }
                    }
                }
            }
            div { class: "wire-sec",
                span { class: "small", "group blocks into a prefab" }
                div { class: "row wrap",
                    button { class: if grouping() { "btn small primary" } else { "btn small" },
                        onclick: move |_| {
                            let now = grouping();
                            grouping.set(!now);
                            // the selected block (if any) starts the group
                            if !now
                                && let Some(id) = (sp.block)().and_then(|b| b["id"].as_str().map(str::to_string)).filter(|i| !i.starts_with("prefab:"))
                                && !group.peek().contains(&id)
                            {
                                group.write().push(id);
                            }
                        },
                        if grouping() { "done picking" } else { "pick blocks in the room" }
                    }
                    if !group().is_empty() {
                        button { class: "btn small", onclick: move |_| group.set(Vec::new()), "clear" }
                    }
                }
                if grouping() {
                    p { class: "dim small", "Click the blocks to group; each one joins." }
                }
                div { class: "wire-targets",
                    for t in group() {
                        {
                            let t2 = t.clone();
                            rsx! {
                                span { key: "{t}", class: "chip small", {short(&t)}
                                    button { class: "plain", "aria-label": "leave it out", onclick: move |_| group.write().retain(|x| x != &t2), "×" }
                                }
                            }
                        }
                    }
                }
                if !group().is_empty() {
                    div { class: "row",
                        input { r#type: "text", placeholder: "its name", value: "{name}", "aria-label": "the prefab's name",
                            oninput: move |e: FormEvent| name.set(e.value()) }
                        button { class: "btn small primary", disabled: name().trim().is_empty(), onclick: make, "make it" }
                    }
                }
            }
            div { class: "wire-sec",
                span { class: "small", if list.is_empty() { "no prefabs yet" } else { "the menu: place a copy ahead of you" } }
                button { class: "btn small",
                    title: "a switch, AND, OR, XOR and NOT gates, a latch, a lamp and a door, run by s programs (notes tagged s); placed in a row at one height, each one's ray reaches the next",
                    onclick: {
                        let api = api.clone();
                        move |_| {
                            let api = api.clone();
                            spawn(async move {
                                match api.call("prefabs.starters", json!({})).await {
                                    Ok(r) => {
                                        load_prefabs(ui, &api, sp).await;
                                        let n = r["prefabs"].as_array().map_or(0, |a| a.len());
                                        ui.say(if n == 0 { "the starters are here already".to_string() } else { format!("{n} starter prefabs · place them in a row at one height, then play: click a switch") });
                                    }
                                    Err(e) => ui.fail(e),
                                }
                            });
                        }
                    },
                    "add the starters: switch, gates, lamp, door"
                }
                div { class: "pf-grid",
                    for p in list.iter() {
                        {
                            let id = p["id"].as_u64().unwrap_or(0);
                            let pname = p["name"].as_str().unwrap_or("").to_string();
                            let n = p["copies"].as_u64().unwrap_or(0);
                            let parts = p["parts"].as_array().map_or(0, |a| a.len());
                            let src = thumbs.get(&id).cloned();
                            let api_r = api.clone();
                            let api_d = api.clone();
                            rsx! {
                                div { key: "{id}", class: "pf-card",
                                    button { class: "pf-thumb", title: "place a copy of {pname} ahead of you",
                                        onclick: move |_| sp.send(json!({ "cmd": "prefabspot", "want": id })),
                                        if let Some(s) = src { img { src: "{s}", alt: "{pname}" } } else { Icon { name: "cube" } }
                                    }
                                    if renaming() == Some(id) {
                                        input { r#type: "text", value: "{rename_to}", "aria-label": "its new name",
                                            oninput: move |e: FormEvent| rename_to.set(e.value()),
                                            onkeydown: move |e: KeyboardEvent| {
                                                if e.key() == Key::Enter {
                                                    let api = api_r.clone();
                                                    let to = rename_to.peek().clone();
                                                    renaming.set(None);
                                                    spawn(async move {
                                                        match api.call("prefabs.rename", json!({ "prefab": id, "name": to })).await {
                                                            Ok(_) => load_prefabs(ui, &api, sp).await,
                                                            Err(e) => ui.fail(e),
                                                        }
                                                    });
                                                } else if e.key() == Key::Escape { renaming.set(None); }
                                            }
                                        }
                                    } else {
                                        b { class: "small", ondoubleclick: { let pn = pname.clone(); move |_| { rename_to.set(pn.clone()); renaming.set(Some(id)); } }, title: "double click to rename", "{pname}" }
                                    }
                                    span { class: "dim small", {format!("{parts} block{} · {n} cop{}", if parts == 1 { "" } else { "s" }, if n == 1 { "y" } else { "ies" })} }
                                    div { class: "row wrap",
                                        button { class: "btn small", onclick: move |_| sp.send(json!({ "cmd": "prefabspot", "want": id })), "place" }
                                        button { class: "btn small", title: "place copies by hand: the prefab follows where you look, flying or walking; the wheel turns it, a click places it, Esc stops",
                                            onclick: { let api = api.clone(); let p = p.clone(); move |_| { let api = api.clone(); let p = p.clone(); spawn(async move { ghost(ui, &api, sp, &p).await }); } },
                                            "by hand"
                                        }
                                        button { class: if deleting() == Some(id) { "btn small primary" } else { "btn small" },
                                            onclick: move |_| {
                                                if deleting() != Some(id) { deleting.set(Some(id)); return; }
                                                deleting.set(None);
                                                let api = api_d.clone();
                                                spawn(async move {
                                                    match api.call("prefabs.delete", json!({ "prefab": id })).await {
                                                        Ok(r) => {
                                                            sp.send(json!({ "cmd": "unblock", "ids": r["removed"] }));
                                                            load_prefabs(ui, &api, sp).await;
                                                            super::wirewin::load_wires(&api, sp).await;
                                                        }
                                                        Err(e) => ui.fail(e),
                                                    }
                                                });
                                            },
                                            if deleting() == Some(id) { if n > 0 { "and its copies?" } else { "sure?" } } else { "delete" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// a copy's menu (a right click on it): it's one group
#[component]
pub(super) fn CopyMenu(c: Value, x: f64, y: f64, sp: Sp) -> Element {
    let ui = ui();
    let api = api();
    let mut menu = sp.bmenu;
    let mut close = move || menu.set(None);
    let id = c["id"].as_str().unwrap_or("").to_string();
    let name = c["name"].as_str().unwrap_or("a prefab").to_string();
    let (a1, a2, a3, a4) = (api.clone(), api.clone(), api.clone(), api.clone());
    let (i1, i2, i3, i4, i5) = (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
    rsx! {
        div { class: "backdrop", onclick: move |_| close(), oncontextmenu: move |e| { e.prevent_default(); close() } }
        div { class: "menu smenu", role: "menu",
            style: "left: min({x}px, calc(100vw - 330px)); top: max(8px, min({y}px, calc(100vh - 300px)));",
            div { class: "title", "a copy of {name}" }
            div { class: "title dim small", "one group: it moves, turns, copies and goes as a whole" }
            button { role: "menuitem", onclick: move |_| { sp.send(json!({ "cmd": "opencopy", "id": i1 })); close(); },
                Icon { name: "edit" } "open it to edit its parts" kbd { "double-click" } }
            button { role: "menuitem", onclick: move |_| { copy_set(ui, a1.clone(), sp, json!({ "id": i2, "turn": 1 })); close(); },
                Icon { name: "refresh" } "a quarter turn about the upright" }
            button { role: "menuitem", onclick: move |_| { copy_duplicate(ui, a2.clone(), sp, i3.clone()); close(); },
                Icon { name: "copy" } "a copy beside it" }
            button { role: "menuitem", onclick: move |_| { set_pref(ui, |p| p.space.show_prefabs = true); close(); },
                Icon { name: "prefab" } "its prefab…" span { class: "dim small", " (the prefabs window: size, own changes)" } }
            button { role: "menuitem",
                onclick: move |_| {
                    let (api, id) = (a3.clone(), i4.clone());
                    spawn(async move {
                        match api.call("prefabs.detach", json!({ "id": id })).await {
                            Ok(r) => { show(ui, &api, sp, &r).await; load_prefabs(ui, &api, sp).await; ui.say("detached: ordinary blocks now"); }
                            Err(e) => ui.fail(e),
                        }
                    });
                    close();
                },
                Icon { name: "cube" } "detach it into blocks" }
            button { role: "menuitem", onclick: move |_| { copy_remove(ui, a4.clone(), sp, i5.clone()); close(); },
                Icon { name: "delete" } "remove it" kbd { "{ui.key_of(crate::keys::Action::DeleteText)}" } }
        }
    }
}
