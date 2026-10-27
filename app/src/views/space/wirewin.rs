//! The wiring window: blocks that cast rays and run programs (core's `graph::wiring`,
//! `ops::wiring`). For the selected block: the events it listens for (a click, a ray's
//! hit, a text coming into or leaving its zone, a timer, a named signal), the s program
//! it runs then (a note tagged `s`), and its ray: none, fixed (a direction out of the
//! block, registering the first, last, nth or all of what it crosses, of the kinds it
//! sees, and optionally sticking to what it first hits) or sticky (to target blocks,
//! picked in the room). "play" makes the room live: a click on a wired block fires it
//! (instead of selecting it), timers run, zones report texts coming and going. "try it"
//! fires the block once. The window lists what the last cause set off.

use dioxus::prelude::*;
use serde_json::{Value, json};

use super::{Sp, Win, dress};
use crate::shell::set_pref;
use crate::ui::*;

const EVENTS: [(&str, &str); 6] = [
    ("click", "a click (while playing)"),
    ("hit", "another block's ray"),
    ("enter", "a text coming into its zone"),
    ("leave", "a text leaving its zone"),
    ("timer", "a timer"),
    ("signal", "a signal by name"),
];

const DIRS: [(&str, [f64; 3]); 6] = [
    ("front", [0.0, 0.0, 1.0]),
    ("back", [0.0, 0.0, -1.0]),
    ("right", [1.0, 0.0, 0.0]),
    ("left", [-1.0, 0.0, 0.0]),
    ("up", [0.0, 1.0, 0.0]),
    ("down", [0.0, -1.0, 0.0]),
];

/// the wired blocks, into the room (what each listens for), and whether it's live
pub(super) async fn load_wires(api: &Api, sp: Sp) {
    let list = match api.call("wiring.list", json!({})).await {
        Ok(v) => v.as_array().cloned().unwrap_or_default(),
        // a hub from before wiring
        Err(_) => Vec::new(),
    };
    let wires: Vec<Value> = list
        .iter()
        .map(|w| json!({ "id": w["id"], "on": w["wiring"]["on"], "every": w["wiring"]["every"] }))
        .collect();
    let live = *sp.live.peek();
    sp.send(json!({ "cmd": "wires", "wires": wires, "live": live }));
}

/// fires a block: the room shows what changed, its rays and what was said
pub(super) fn fire(ui: Ui, api: Api, sp: Sp, id: String, event: &'static str, from: String) {
    dioxus::dioxus_core::spawn_forever(async move {
        match api
            .call("wiring.fire", json!({ "id": id, "event": event, "from": from }))
            .await
        {
            Ok(r) => {
                if r["removed"].as_array().is_some_and(|a| !a.is_empty()) {
                    sp.send(json!({ "cmd": "unblock", "ids": r["removed"] }));
                }
                if r["blocks"].as_array().is_some_and(|a| !a.is_empty()) {
                    let textures = dress(ui, &api, r["textures"].clone()).await;
                    sp.send(json!({ "cmd": "blocks", "blocks": r["blocks"], "textures": textures }));
                }
                sp.send(json!({ "cmd": "rays", "rays": r["rays"] }));
                if r["sounds"].as_array().is_some_and(|a| !a.is_empty()) {
                    sp.send(json!({ "cmd": "sounds", "sounds": r["sounds"] }));
                }
                for s in r["says"].as_array().into_iter().flatten() {
                    ui.say(s.as_str().unwrap_or("").to_string());
                }
                let errs: Vec<String> = r["events"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| {
                        e["err"]
                            .as_str()
                            .map(|x| format!("{}: {x}", e["block"].as_str().unwrap_or("")))
                    })
                    .collect();
                if let Some(e) = errs.first() {
                    ui.fail(e.clone());
                }
                if r["cut"] == true {
                    ui.say("stopped: one cause sets off at most 200 events, in one budget of steps".to_string());
                }
                let mut f = sp.fired;
                f.set(r["events"].as_array().cloned().unwrap_or_default());
                if r["removed"].as_array().is_some_and(|a| !a.is_empty())
                    || r["events"]
                        .as_array()
                        .is_some_and(|e| e.iter().any(|x| x["block"] != x["from"]))
                {
                    load_wires(&api, sp).await;
                }
            }
            Err(e) => ui.fail(e),
        }
    });
}

fn has(w: &Value, ev: &str) -> bool {
    w["on"].as_array().is_some_and(|a| a.iter().any(|x| x == ev))
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
pub(super) fn Wires(sp: Sp) -> Element {
    let ui = ui();
    let api = api();
    // the block being wired: the selected one, held while targets are picked
    let mut wired: Signal<Option<String>> = use_signal(|| None);
    let mut picking = use_signal(|| false);
    let mut w: Signal<Value> = use_signal(|| Value::Null);
    let mut dirty = use_signal(|| false);
    let mut programs: Signal<Vec<String>> = use_signal(Vec::new);
    let sel = (sp.block)().and_then(|b| b["id"].as_str().map(str::to_string));

    // the programs to choose from
    let api_p = api.clone();
    use_hook(move || {
        let api = api_p.clone();
        spawn(async move {
            if let Ok(v) = api.call("s.programs", json!({})).await {
                programs.set(
                    v.as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p["title"].as_str().map(str::to_string))
                        .collect(),
                );
            }
        });
    });

    // the selection: picked as a target, or the block to wire
    let api_s = api.clone();
    use_effect(use_reactive!(|sel| {
        let Some(id) = sel else { return };
        if *picking.peek() {
            if wired.peek().as_deref() != Some(id.as_str()) {
                let mut v = w.peek().clone();
                let ts = v["ray"]["targets"].as_array().cloned().unwrap_or_default();
                if !ts.iter().any(|t| t == &json!(id)) {
                    let mut ts = ts;
                    ts.push(json!(id));
                    v["ray"]["targets"] = json!(ts);
                    w.set(v);
                    dirty.set(true);
                }
            }
            return;
        }
        if wired.peek().as_deref() == Some(id.as_str()) {
            return;
        }
        wired.set(Some(id.clone()));
        dirty.set(false);
        let api = api_s.clone();
        spawn(async move {
            match api.call("wiring.get", json!({ "id": id })).await {
                Ok(v) => w.set(v),
                Err(e) => ui.fail(e),
            }
        });
    }));

    let api_k = api.clone();
    let keep = move |v: Value| {
        let Some(id) = wired.peek().clone() else { return };
        let api = api_k.clone();
        spawn(async move {
            match api.call("wiring.set", json!({ "id": id, "wiring": v })).await {
                Ok(r) => {
                    w.set(r["wiring"].clone());
                    dirty.set(false);
                    load_wires(&api, sp).await;
                    ui.say(
                        if r["wiring"].is_null() {
                            "unwired"
                        } else {
                            "wired · \"try it\" fires it once; \"play\" makes the room live"
                        }
                        .to_string(),
                    );
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let keep2 = keep.clone();

    let cur = w();
    let on = !cur.is_null();
    let ray = cur["ray"].clone();
    let aim = if ray.is_null() {
        "none"
    } else if ray["targets"].as_array().is_some_and(|a| !a.is_empty()) || *picking.read() {
        "sticky"
    } else {
        "fixed"
    };
    let mode = ray["mode"].as_str().unwrap_or("first").to_string();
    let live = (sp.live)();
    let fired = (sp.fired)();
    let mut set = move |f: &dyn Fn(&mut Value)| {
        let mut v = w.peek().clone();
        if v.is_null() {
            v = json!({ "on": [] });
        }
        f(&mut v);
        w.set(v);
        dirty.set(true);
    };

    rsx! {
        Win {
            id: "wiring",
            at: (8.0, 140.0),
            on_close: move |_| set_pref(ui, |p| p.space.show_wiring = false),
            title: rsx! {
                span { class: "win-title",
                    Icon { name: "bolt" }
                    "wiring"
                    span { class: "dim small", style: "font-weight: 400;", "rays and programs" }
                }
            },
            div { class: "row wrap",
                button { class: if live { "btn small primary" } else { "btn small" },
                    "aria-pressed": "{live}",
                    title: "play: a click on a wired block fires it, timers run, zones tell of texts coming and going",
                    onclick: {
                        let api = api.clone();
                        move |_| {
                            let mut l = sp.live;
                            l.set(!live);
                            let api = api.clone();
                            spawn(async move { load_wires(&api, sp).await });
                        }
                    },
                    if live { "■ stop playing" } else { "▶ play" }
                }
                if let Some(id) = wired() {
                    span { class: "small", "wiring " b { {short(&id)} } }
                }
            }
            if wired().is_none() {
                p { class: "dim small", "Select a block to wire it." }
            } else {
                div { class: "wire-sec",
                    span { class: "small", "it listens for" }
                    div { class: "row wrap",
                        for (ev, what) in EVENTS {
                            label { key: "{ev}", class: "check small", title: "{what}",
                                input { r#type: "checkbox", checked: has(&cur, ev),
                                    onchange: move |e: FormEvent| {
                                        let yes = e.checked();
                                        set(&|v: &mut Value| {
                                            let mut a: Vec<Value> = v["on"].as_array().cloned().unwrap_or_default();
                                            a.retain(|x| x != ev);
                                            if yes { a.push(json!(ev)); }
                                            v["on"] = json!(a);
                                        });
                                    }
                                }
                                "{ev}"
                            }
                        }
                    }
                    if has(&cur, "timer") {
                        label { class: "lt-slider", span { "every (s)" }
                            input { r#type: "number", min: "0.2", step: "0.1", value: "{cur[\"every\"].as_f64().unwrap_or(1.0)}",
                                oninput: move |e: FormEvent| { let x = e.value().parse::<f64>().unwrap_or(1.0); set(&|v: &mut Value| v["every"] = json!(x)); }
                            }
                        }
                    }
                    if has(&cur, "signal") {
                        label { class: "lt-slider", span { "signal" }
                            input { r#type: "text", placeholder: "its name", value: "{cur[\"signal\"].as_str().unwrap_or(\"\")}",
                                oninput: move |e: FormEvent| { let x = e.value(); set(&|v: &mut Value| v["signal"] = json!(x.clone())); }
                            }
                        }
                    }
                }
                div { class: "wire-sec",
                    label { class: "lt-slider", span { "program" }
                        select {
                            "aria-label": "the s program it runs",
                            onchange: move |e: FormEvent| { let x = e.value(); set(&|v: &mut Value| v["program"] = json!(x.clone())); },
                            option { value: "", selected: cur["program"].as_str().unwrap_or("").is_empty(), "none: it casts its ray" }
                            for p in programs() {
                                option { key: "{p}", value: "{p}", selected: cur["program"].as_str() == Some(p.as_str()), "{p}" }
                            }
                        }
                    }
                    if programs().is_empty() {
                        p { class: "dim small", "Programs are notes tagged s (the notes view's s button)." }
                    }
                }
                div { class: "wire-sec",
                    span { class: "small", "its ray" }
                    div { class: "row wrap",
                        for a in ["none", "fixed", "sticky"] {
                            button { key: "{a}", class: if aim == a { "btn small on" } else { "btn small" },
                                onclick: move |_| {
                                    picking.set(a == "sticky");
                                    set(&|v: &mut Value| match a {
                                        "none" => { if let Some(o) = v.as_object_mut() { o.remove("ray"); } }
                                        "fixed" => { let mut r = v["ray"].clone(); if r.is_null() { r = json!({}); } r["targets"] = json!([]); v["ray"] = r; }
                                        _ => { if v["ray"].is_null() { v["ray"] = json!({ "targets": [] }); } }
                                    });
                                },
                                "{a}"
                            }
                        }
                    }
                    if aim == "fixed" {
                        div { class: "row wrap",
                            span { class: "small", "out of its" }
                            for (name, d) in DIRS {
                                {
                                    let now: Vec<f64> = ray["dir"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or(vec![0.0, 0.0, 1.0]);
                                    let on = now.len() == 3 && (0..3).all(|i| (now[i] - d[i]).abs() < 1e-3);
                                    rsx! {
                                        button { key: "{name}", class: if on { "btn small on" } else { "btn small" },
                                            onclick: move |_| set(&|v: &mut Value| v["ray"]["dir"] = json!(d)),
                                            "{name}"
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "row wrap",
                            span { class: "small", "registers" }
                            for m in ["first", "last", "all", "nth"] {
                                button { key: "{m}", class: if mode == m { "btn small on" } else { "btn small" },
                                    onclick: move |_| set(&|v: &mut Value| v["ray"]["mode"] = json!(m)),
                                    "{m}"
                                }
                            }
                            if mode == "nth" {
                                input { r#type: "number", min: "1", style: "width: 4em;", value: "{ray[\"n\"].as_u64().unwrap_or(1)}",
                                    "aria-label": "which one it registers",
                                    oninput: move |e: FormEvent| { let n = e.value().parse::<u64>().unwrap_or(1).max(1); set(&|v: &mut Value| v["ray"]["n"] = json!(n)); }
                                }
                            }
                        }
                        label { class: "lt-slider", span { "reach" }
                            input { r#type: "range", min: "1", max: "200", value: "{ray[\"range\"].as_f64().unwrap_or(50.0)}",
                                oninput: move |e: FormEvent| { let x = e.value().parse::<f64>().unwrap_or(50.0); set(&|v: &mut Value| v["ray"]["range"] = json!(x)); }
                            }
                            span { class: "dim small", {format!("{:.0}", ray["range"].as_f64().unwrap_or(50.0))} }
                        }
                        div { class: "row wrap",
                            span { class: "small", "sees" }
                            for k in ["text", "picture", "plain", "texture"] {
                                {
                                    let sees: Vec<Value> = ray["sees"].as_array().cloned().unwrap_or_default();
                                    let on = sees.is_empty() || sees.iter().any(|x| x == k);
                                    rsx! {
                                        label { key: "{k}", class: "check small",
                                            input { r#type: "checkbox", checked: on,
                                                onchange: move |e: FormEvent| {
                                                    let yes = e.checked();
                                                    set(&|v: &mut Value| {
                                                        let mut s: Vec<Value> = v["ray"]["sees"].as_array().cloned().unwrap_or_default();
                                                        if s.is_empty() { s = ["text", "picture", "plain", "texture"].iter().map(|x| json!(x)).collect(); }
                                                        s.retain(|x| x != k);
                                                        if yes { s.push(json!(k)); }
                                                        if s.len() == 4 { s.clear(); }
                                                        v["ray"]["sees"] = json!(s);
                                                    });
                                                }
                                            }
                                            "{k}"
                                        }
                                    }
                                }
                            }
                        }
                        label { class: "check small",
                            input { r#type: "checkbox", checked: ray["stick"] == true,
                                onchange: move |e: FormEvent| { let yes = e.checked(); set(&|v: &mut Value| v["ray"]["stick"] = json!(yes)); }
                            }
                            "become sticky on its first hit"
                        }
                    }
                    if aim == "sticky" {
                        div { class: "wire-targets",
                            for t in ray["targets"].as_array().cloned().unwrap_or_default() {
                                {
                                    let t = t.as_str().unwrap_or("").to_string();
                                    let t2 = t.clone();
                                    rsx! {
                                        span { key: "{t}", class: "chip small", {short(&t)}
                                            button { class: "plain", "aria-label": "not this target",
                                                onclick: move |_| { let t = t2.clone(); set(&|v: &mut Value| { if let Some(a) = v["ray"]["targets"].as_array_mut() { a.retain(|x| x != &json!(t)); } }); },
                                                "×"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        button { class: if picking() { "btn small primary" } else { "btn small" },
                            onclick: move |_| {
                                let now = picking();
                                picking.set(!now);
                                if now && let Some(id) = wired.peek().clone() {
                                    // back to the wired block
                                    sp.send(json!({ "cmd": "pickblock", "id": id }));
                                }
                            },
                            if picking() { "done picking" } else { "pick targets in the room" }
                        }
                        if picking() {
                            p { class: "dim small", "Click the blocks it should hit; each becomes a target." }
                        }
                    }
                }
                div { class: "row wrap",
                    button { class: if dirty() { "btn small primary" } else { "btn small" }, disabled: !dirty(),
                        onclick: move |_| { picking.set(false); keep(w.peek().clone()); },
                        "keep"
                    }
                    button { class: "btn small", disabled: !on,
                        title: "fire it once, as a click would",
                        onclick: {
                            let api = api.clone();
                            move |_| {
                                if let Some(id) = wired.peek().clone() {
                                    fire(ui, api.clone(), sp, id.clone(), "click", String::new());
                                }
                            }
                        },
                        "try it"
                    }
                    button { class: "btn small", disabled: !on,
                        onclick: move |_| keep2(Value::Null),
                        "unwire"
                    }
                }
                if !fired.is_empty() {
                    div { class: "wire-log",
                        span { class: "small", "what the last cause set off" }
                        for (i, e) in fired.iter().enumerate() {
                            div { key: "{i}", class: "small",
                                b { {short(e["block"].as_str().unwrap_or(""))} }
                                " · {e[\"event\"].as_str().unwrap_or(\"\")}"
                                if let Some(o) = e["out"].as_str().filter(|o| !o.is_empty()) {
                                    pre { class: "s-out", "{o}" }
                                }
                                if let Some(x) = e["err"].as_str() {
                                    div { class: "s-err", "{x}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
