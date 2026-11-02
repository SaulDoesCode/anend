//! The animations window: keyframe animations for blocks (core's `graph::anims`), made and
//! previewed on the selected block. An animation is an entry of the save, so one is shared
//! by every block that plays it.
//!
//! The window lists the save's animations; one is opened to edit: its name, length and how
//! it repeats, its timing (a cubic-bezier curve whose two control points are dragged, with
//! CSS's named curves one click away), and a timeline of keys. The scrubber shows any
//! moment on the selected block; "a key here" adds a key there with the pose it has; a
//! picked key's move, turn and scale are sliders, its texture a pick, and it can have a
//! timing of its own to the next key. "play" previews it on the block; "keep" saves it;
//! the selected block can then play it, with a delay and a speed of its own.

use dioxus::prelude::*;
use serde_json::{Value, json};

use super::{Sp, Win, dress};
use crate::shell::set_pref;
use crate::ui::*;

/// the save's animations, and the textures their keys show, into the room
pub(super) async fn load_anims(ui: Ui, api: &Api, sp: Sp) -> Vec<Value> {
    let list: Vec<Value> = match api.call("anims.list", json!({})).await {
        Ok(v) => v.as_array().cloned().unwrap_or_default(),
        // a hub from before animations
        Err(_) => Vec::new(),
    };
    let textures = textures_of(api, &list).await;
    let textures = dress(ui, api, textures).await;
    sp.send(json!({ "cmd": "anims", "anims": list, "textures": textures }));
    list
}

/// what the keys' textures are, for the room to draw them
async fn textures_of(api: &Api, list: &[Value]) -> Value {
    let mut out = serde_json::Map::new();
    let recipes = api.call("textures.list", json!({})).await.ok();
    for a in list {
        for k in a["anim"]["keys"].as_array().into_iter().flatten() {
            let Some(t) = k["texture"].as_str() else { continue };
            if out.contains_key(t) {
                continue;
            }
            let about = if let Some(c) = t.strip_prefix("plain:") {
                json!({ "colour": c })
            } else if let Some(id) = t.strip_prefix("texture:") {
                recipes
                    .as_ref()
                    .and_then(|r| r.as_array())
                    .and_then(|r| r.iter().find(|x| x["id"].as_str() == Some(id)).cloned())
                    .unwrap_or(json!({ "gone": true }))
            } else {
                json!({})
            };
            out.insert(t.to_string(), about);
        }
    }
    Value::Object(out)
}

fn blank() -> Value {
    json!({ "name": "", "duration": 2.0, "repeat": "loop", "timing": [0.25, 0.1, 0.25, 1.0],
            "keys": [{ "at": 0.0, "move": [0.0, 0.0, 0.0] }, { "at": 1.0, "move": [0.0, 1.0, 0.0] }] })
}

/// CSS's named timings
const NAMED: [(&str, [f64; 4]); 5] = [
    ("linear", [0.0, 0.0, 1.0, 1.0]),
    ("ease", [0.25, 0.1, 0.25, 1.0]),
    ("ease-in", [0.42, 0.0, 1.0, 1.0]),
    ("ease-out", [0.0, 0.0, 0.58, 1.0]),
    ("ease-in-out", [0.42, 0.0, 0.58, 1.0]),
];

fn four(v: &Value) -> [f64; 4] {
    let a: Vec<f64> = v
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    if a.len() == 4 {
        [a[0], a[1], a[2], a[3]]
    } else {
        [0.25, 0.1, 0.25, 1.0]
    }
}

/// two timings the same, as far as f32 storage keeps them
fn same(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
}

fn three(v: &Value, d: f64) -> [f64; 3] {
    let a: Vec<f64> = v
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    if a.len() == 3 { [a[0], a[1], a[2]] } else { [d; 3] }
}

/// a cubic-bezier timing drawn in a square: drag either control point
#[component]
fn Curve(t: [f64; 4], on_change: EventHandler<[f64; 4]>) -> Element {
    let mut dragging: Signal<Option<u8>> = use_signal(|| None);
    let (w, h) = (150.0f64, 150.0f64);
    let px = |x: f64| 15.0 + x * (w - 30.0);
    // y from -0.5 to 1.5 (overshoot shows)
    let py = |y: f64| h - 15.0 - (y + 0.5) / 2.0 * (h - 30.0) - 0.0;
    let mut d = String::new();
    for i in 0..=40 {
        let s = i as f64 / 40.0;
        let b = |a: f64, c: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * a + 3.0 * (1.0 - s) * s * s * c + s * s * s;
        d += &format!(
            "{}{:.1},{:.1} ",
            if i == 0 { "M" } else { "L" },
            px(b(t[0], t[2])),
            py(b(t[1], t[3]))
        );
    }
    let to = move |x: f64, y: f64| -> (f64, f64) {
        let fx = ((x - 15.0) / (w - 30.0)).clamp(0.0, 1.0);
        let fy = ((h - 15.0 - y) / (h - 30.0) * 2.0 - 0.5).clamp(-0.5, 1.5);
        ((fx * 100.0).round() / 100.0, (fy * 100.0).round() / 100.0)
    };
    rsx! {
        svg { class: "bend-curve anim-curve", width: "{w}", height: "{h}", view_box: "0 0 {w} {h}",
            rect { x: "{px(0.0)}", y: "{py(1.0)}", width: "{px(1.0) - px(0.0)}", height: "{py(0.0) - py(1.0)}", class: "anim-box" }
            line { x1: "{px(0.0)}", y1: "{py(0.0)}", x2: "{px(t[0])}", y2: "{py(t[1])}", class: "handle" }
            line { x1: "{px(1.0)}", y1: "{py(1.0)}", x2: "{px(t[2])}", y2: "{py(t[3])}", class: "handle" }
            path { d: "{d}", class: "curve" }
            circle { cx: "{px(t[0])}", cy: "{py(t[1])}", r: "7", class: "ctl", onmousedown: move |e| { e.prevent_default(); dragging.set(Some(1)) } }
            circle { cx: "{px(t[2])}", cy: "{py(t[3])}", r: "7", class: "ctl", onmousedown: move |e| { e.prevent_default(); dragging.set(Some(2)) } }
            if dragging().is_some() {
                rect { x: "0", y: "0", width: "{w}", height: "{h}", class: "catch",
                    onmousemove: move |e| {
                        let c = e.element_coordinates();
                        let (x, y) = to(c.x, c.y);
                        let mut n = t;
                        if dragging() == Some(1) { n[0] = x; n[1] = y; } else { n[2] = x; n[3] = y; }
                        on_change.call(n);
                    },
                    onmouseup: move |_| dragging.set(None),
                    onmouseleave: move |_| dragging.set(None),
                }
            }
        }
    }
}

/// three sliders for a key's vector, and whether the key sets it at all
fn vec_row(
    label: &str,
    v: &Value,
    (lo, hi, step, rest): (f64, f64, f64, f64),
    on: EventHandler<Option<[f64; 3]>>,
) -> Element {
    let set = !v.is_null();
    let cur = three(v, rest);
    rsx! {
        div { class: "anim-vec",
            label { class: "sb-tick", title: "whether this key sets it (else it's carried between the keys that do)",
                input { r#type: "checkbox", checked: set, oninput: move |e| on.call(if e.checked() { Some(cur) } else { None }) }
                "{label}"
            }
            if set {
                for (i , axis) in ["x", "y", "z"].into_iter().enumerate() {
                    label { key: "{axis}", class: "lt-slider",
                        span { "{axis}" }
                        input { r#type: "range", min: "{lo}", max: "{hi}", step: "{step}", value: "{cur[i]}",
                            oninput: move |e| { let mut n = cur; n[i] = e.value().parse().unwrap_or(rest); on.call(Some(n)) } }
                        span { class: "dim small", {format!("{:.2}", cur[i])} }
                    }
                }
            }
        }
    }
}

#[component]
pub(super) fn Anims(sp: Sp, block: Option<Value>, on_changed: EventHandler<()>) -> Element {
    let ui = ui();
    let api = api();
    let mut list: Signal<Vec<Value>> = use_signal(Vec::new);
    // the animation being edited: its key (None: new) and itself
    let mut open: Signal<Option<(Option<String>, Value)>> = use_signal(|| None);
    let mut picked: Signal<Option<usize>> = use_signal(|| None);
    let mut scrub = use_signal(|| 0.0f64);
    let mut playing = use_signal(|| false);
    let mut recipes: Signal<Vec<(String, String)>> = use_signal(Vec::new);
    let api_l = api.clone();
    use_hook(move || {
        let api = api_l.clone();
        spawn(async move {
            list.set(load_anims(ui, &api, sp).await);
            if let Ok(r) = api.call("textures.list", json!({})).await {
                recipes.set(
                    r.as_array()
                        .into_iter()
                        .flatten()
                        .map(|t| {
                            let id = t["id"].as_str().unwrap_or("").to_string();
                            let name = t["name"]
                                .as_str()
                                .filter(|n| !n.is_empty())
                                .map(str::to_string)
                                .unwrap_or_else(|| id.chars().take(8).collect());
                            (format!("texture:{id}"), name)
                        })
                        .collect(),
                );
            }
        });
    });
    let bid = block.as_ref().and_then(|b| b["id"].as_str()).map(str::to_string);
    // the open animation shown on the selected block: at the scrubber, or playing
    let bid_e = bid.clone();
    use_effect(move || {
        let (o, p, play) = (open(), scrub(), playing());
        if let Some(id) = bid_e.clone() {
            match o {
                Some((_, a)) => sp.send(json!({ "cmd": "animpreview", "id": id, "anim": a, "p": if play { Value::Null } else { json!(p) } })),
                None => sp.send(json!({ "cmd": "animpreview", "id": id, "anim": null })),
            }
        }
    });
    let bid_d = bid.clone();
    use_drop(move || {
        if let Some(id) = bid_d.clone() {
            sp.send(json!({ "cmd": "animpreview", "id": id, "anim": null }));
        }
    });
    let edit = move |f: &dyn Fn(&mut Value)| {
        let mut o = open;
        if let Some((_, a)) = o.write().as_mut() {
            f(a);
        }
    };
    let api_k = api.clone();
    let keep = move || {
        let api = api_k.clone();
        let Some((key, a)) = open.peek().clone() else { return };
        spawn(async move {
            match api.call("anims.save", json!({ "key": key, "anim": a })).await {
                Ok(v) => {
                    let k = v["key"].as_str().unwrap_or("").to_string();
                    open.set(Some((Some(k), v["anim"].clone())));
                    list.set(load_anims(ui, &api, sp).await);
                    ui.say("animation kept");
                }
                Err(e) => ui.fail(e),
            }
        });
    };

    let o = open();
    let keys: Vec<Value> = o
        .as_ref()
        .map(|(_, a)| a["keys"].as_array().cloned().unwrap_or_default())
        .unwrap_or_default();
    let pk = picked().filter(|&i| i < keys.len());
    let plays = block
        .as_ref()
        .and_then(|b| b["anim"]["key"].as_str())
        .map(str::to_string);
    rsx! {
        Win {
            id: "anims",
            at: (8.0, 80.0),
            on_close: move |_| set_pref(ui, |p| p.space.show_anims = false),
            title: rsx! {
                span { class: "win-title",
                    Icon { name: "play" }
                    "animations"
                    span { class: "dim small", style: "font-weight: 400;", {match list.read().len() { 0 => "none yet".to_string(), 1 => "1 in this save".to_string(), n => format!("{n} in this save") }} }
                }
            },
            div { class: "row wrap anim-list",
                for a in list() {
                    {
                        let key = a["key"].as_str().unwrap_or("").to_string();
                        let name = a["anim"]["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| format!("animation {key}"));
                        let on = o.as_ref().and_then(|x| x.0.clone()) == Some(key.clone());
                        rsx! {
                            button { key: "{key}", class: if on { "btn small on" } else { "plain small" },
                                title: if plays.as_deref() == Some(key.as_str()) { "the selected block plays this" } else { "open it" },
                                onclick: move |_| { open.set(Some((Some(key.clone()), a["anim"].clone()))); picked.set(None); scrub.set(0.0); },
                                if plays.as_deref() == Some(a["key"].as_str().unwrap_or("")) { "▶ " } "{name}" }
                        }
                    }
                }
                button { class: "btn small", onclick: move |_| { open.set(Some((None, blank()))); picked.set(Some(1)); scrub.set(1.0); }, Icon { name: "new" } "new" }
            }
            if let Some((key, a)) = o.clone() {
                div { class: "anim-edit",
                    div { class: "row",
                        input { r#type: "text", class: "sb-text", placeholder: "its name", value: a["name"].as_str().unwrap_or("").to_string(),
                            oninput: move |e| edit(&|a| a["name"] = json!(e.value())) }
                        select { class: "sb-pick", "aria-label": "how it repeats",
                            onchange: move |e| edit(&|a| a["repeat"] = json!(e.value())),
                            for (r , w) in [("once", "once"), ("loop", "in a loop"), ("alternate", "back and forth")] {
                                option { key: "{r}", value: "{r}", selected: a["repeat"].as_str().unwrap_or("once") == r, "{w}" }
                            }
                        }
                    }
                    label { class: "lt-slider", span { "length" }
                        input { r#type: "range", min: "0.1", max: "20", step: "0.1", value: "{a[\"duration\"].as_f64().unwrap_or(2.0)}",
                            oninput: move |e| edit(&|a| a["duration"] = json!(e.value().parse::<f64>().unwrap_or(2.0))) }
                        span { class: "dim small", {format!("{:.1} s", a["duration"].as_f64().unwrap_or(2.0))} }
                    }
                    div { class: "anim-timing",
                        Curve { t: four(&a["timing"]), on_change: move |t: [f64; 4]| edit(&|a| a["timing"] = json!(t)) }
                        div { class: "anim-named",
                            span { class: "dim small", "timing between keys" }
                            for (n , t) in NAMED {
                                button { key: "{n}", class: if same(four(&a["timing"]), t) { "btn small on" } else { "plain small" }, onclick: move |_| edit(&|a| a["timing"] = json!(t)), "{n}" }
                            }
                        }
                    }
                    // the timeline: the keys as dots; the scrubber shows a moment on the block
                    div { class: "anim-track",
                        for (i , at) in keys.iter().map(|k| k["at"].as_f64().unwrap_or(0.0)).enumerate() {
                            button { key: "{i}", class: if pk == Some(i) { "anim-key on" } else { "anim-key" },
                                style: "left: {at * 100.0}%;",
                                title: "the key at {at:.2}",
                                onclick: move |_| { picked.set(Some(i)); scrub.set(at); playing.set(false); },
                                "◆" }
                        }
                        input { r#type: "range", class: "anim-scrub", min: "0", max: "1", step: "0.005", value: "{scrub()}", "aria-label": "a moment",
                            oninput: move |e| { scrub.set(e.value().parse().unwrap_or(0.0)); playing.set(false); } }
                    }
                    div { class: "row wrap",
                        span { class: "dim small", {format!("{:.0}%", scrub() * 100.0)} }
                        button { class: "btn small", title: "a key at this moment, with the pose it has here",
                            onclick: move |_| {
                                let p = (scrub() * 1000.0).round() / 1000.0;
                                edit(&|a| {
                                    let mut ks = a["keys"].as_array().cloned().unwrap_or_default();
                                    ks.retain(|k| k["at"].as_f64() != Some(p));
                                    ks.push(json!({ "at": p }));
                                    ks.sort_by(|x, y| x["at"].as_f64().unwrap_or(0.0).total_cmp(&y["at"].as_f64().unwrap_or(0.0)));
                                    a["keys"] = json!(ks);
                                });
                                let i = open.peek().as_ref().and_then(|(_, a)| a["keys"].as_array().and_then(|k| k.iter().position(|k| k["at"].as_f64() == Some(p))));
                                picked.set(i);
                            },
                            Icon { name: "new" } "a key here" }
                        button { class: "btn small", disabled: bid.is_none(), title: if bid.is_none() { "select a block to see it on" } else { "play it on the selected block" },
                            onclick: move |_| playing.set(!playing()), if playing() { "pause" } else { "▶ play" } }
                        span { class: "grow" }
                        button { class: "btn small primary", onclick: { let k = keep.clone(); move |_| k() }, Icon { name: "save" } "keep" }
                    }
                    if let Some(i) = pk {
                        {
                            let k = keys[i].clone();
                            let path_set = move |field: &'static str, v: Option<[f64; 3]>| edit(&|a| {
                                match v {
                                    Some(x) => a["keys"][i][field] = json!(x),
                                    None => { if let Some(o) = a["keys"][i].as_object_mut() { o.remove(field); } }
                                }
                            });
                            let tex = k["texture"].as_str().unwrap_or("").to_string();
                            rsx! {
                                div { class: "anim-keyed",
                                    div { class: "row",
                                        b { "key {i + 1}" }
                                        label { class: "lt-slider", span { "at" }
                                            input { r#type: "range", min: "0", max: "1", step: "0.005", value: "{k[\"at\"].as_f64().unwrap_or(0.0)}",
                                                oninput: move |e| { let p: f64 = e.value().parse().unwrap_or(0.0); scrub.set(p); edit(&|a| a["keys"][i]["at"] = json!(p)) } }
                                        }
                                        button { class: "plain small", title: "take this key out",
                                            onclick: move |_| { edit(&|a| { if let Some(ks) = a["keys"].as_array_mut() && i < ks.len() { ks.remove(i); } }); picked.set(None); },
                                            Icon { name: "delete" } }
                                    }
                                    {vec_row("move (units)", &k["move"], (-10.0, 10.0, 0.05, 0.0), EventHandler::new(move |v| path_set("move", v)))}
                                    {vec_row("turn (degrees)", &k["turn"], (-360.0, 360.0, 1.0, 0.0), EventHandler::new(move |v| path_set("turn", v)))}
                                    {vec_row("scale (times)", &k["scale"], (0.1, 4.0, 0.05, 1.0), EventHandler::new(move |v| path_set("scale", v)))}
                                    div { class: "row wrap",
                                        span { class: "small", "texture from here" }
                                        select { class: "sb-pick", "aria-label": "the key's texture",
                                            onchange: move |e| { let t = e.value(); edit(&|a| { if t.is_empty() { if let Some(o) = a["keys"][i].as_object_mut() { o.remove("texture"); } } else { a["keys"][i]["texture"] = json!(t) } }) },
                                            option { value: "", selected: tex.is_empty(), "the block's own" }
                                            if tex.starts_with("plain:") { option { value: "{tex}", selected: true, "{tex}" } }
                                            for (id , n) in recipes() { option { key: "{id}", value: "{id}", selected: tex == id, "the recipe {n}" } }
                                        }
                                        input { r#type: "color", "aria-label": "a colour from this key", value: tex.strip_prefix("plain:").unwrap_or("#c0a070").to_string(),
                                            onchange: move |e| edit(&|a| a["keys"][i]["texture"] = json!(format!("plain:{}", e.value()))) }
                                    }
                                    div { class: "row wrap",
                                        label { class: "sb-tick", title: "a timing of its own from this key to the next",
                                            input { r#type: "checkbox", checked: !k["timing"].is_null(),
                                                oninput: move |e| { let on = e.checked(); edit(&|a| { if on { let t = a["timing"].clone(); a["keys"][i]["timing"] = t; } else if let Some(o) = a["keys"][i].as_object_mut() { o.remove("timing"); } }) } }
                                            "its own timing to the next key"
                                        }
                                    }
                                    if !k["timing"].is_null() {
                                        Curve { t: four(&k["timing"]), on_change: move |t: [f64; 4]| edit(&|a| a["keys"][i]["timing"] = json!(t)) }
                                    }
                                }
                            }
                        }
                    }
                    if let (Some(b), Some(k)) = (block.clone(), key.clone()) {
                        {
                            let id = b["id"].as_str().unwrap_or("").to_string();
                            let delay = b["anim"]["delay"].as_f64().unwrap_or(0.0);
                            let speed = b["anim"]["speed"].as_f64().unwrap_or(1.0);
                            let mine = plays.as_deref() == Some(k.as_str());
                            let api = api.clone();
                            let set_anim = move |v: Value| {
                                let (api, id) = (api.clone(), id.clone());
                                spawn(async move {
                                    match api.call("blocks.set", json!({ "id": id, "anim": v })).await {
                                        Ok(r) => {
                                            let textures = dress(ui, &api, r["textures"].clone()).await;
                                            sp.send(json!({ "cmd": "blocks", "blocks": r["blocks"], "textures": textures, "select": id }));
                                            on_changed.call(());
                                        }
                                        Err(e) => ui.fail(e),
                                    }
                                });
                            };
                            let (s1, s2, s3, s4) = (set_anim.clone(), set_anim.clone(), set_anim.clone(), set_anim.clone());
                            let (k1, k2, k3) = (k.clone(), k.clone(), k.clone());
                            rsx! {
                                div { class: "anim-use",
                                    if mine {
                                        span { class: "small", "the selected block plays it" }
                                        label { class: "lt-slider", span { "after" }
                                            input { r#type: "range", min: "0", max: "10", step: "0.1", value: "{delay}",
                                                onchange: move |e| s1(json!({ "key": k1, "delay": e.value().parse::<f64>().unwrap_or(0.0), "speed": speed })) }
                                            span { class: "dim small", {format!("{delay:.1} s")} }
                                        }
                                        label { class: "lt-slider", span { "speed" }
                                            input { r#type: "range", min: "0.1", max: "4", step: "0.05", value: "{speed}",
                                                onchange: move |e| s2(json!({ "key": k2, "delay": delay, "speed": e.value().parse::<f64>().unwrap_or(1.0) })) }
                                            span { class: "dim small", {format!("{speed:.2}×")} }
                                        }
                                        button { class: "plain small", onclick: move |_| s3(Value::Null), "stop it" }
                                    } else {
                                        button { class: "btn small", title: "the selected block plays this animation (kept in the save)",
                                            onclick: move |_| s4(json!({ "key": k3, "delay": 0.0, "speed": 1.0 })), Icon { name: "play" } "the selected block plays it" }
                                    }
                                }
                            }
                        }
                    } else if key.is_none() {
                        p { class: "dim small", "Keep it first; then a block can play it." }
                    } else {
                        p { class: "dim small", "Select a block to see it there and have it play it." }
                    }
                    if let Some(k) = key.clone() {
                        button { class: "plain small", title: "delete the animation (blocks playing it stop)",
                            onclick: { let api = api.clone(); move |_| {
                                let (api, k) = (api.clone(), k.clone());
                                spawn(async move {
                                    match api.call("anims.delete", json!({ "key": k })).await {
                                        Ok(_) => { open.set(None); list.set(load_anims(ui, &api, sp).await); ui.say("animation deleted"); }
                                        Err(e) => ui.fail(e),
                                    }
                                });
                            } },
                            Icon { name: "delete" } "delete it" }
                    }
                }
            } else {
                p { class: "dim small", "Keyframes for blocks, as CSS has them: a move, a turn, a scale and a texture at moments, eased along a timing curve. Open one, or make a new one; select a block to see it there." }
            }
        }
    }
}
