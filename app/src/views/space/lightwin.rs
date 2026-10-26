//! The lights window: the room's lights (core's `graph::lights`), baked onto the blocks by
//! the room (`space.js` `bake`). Add an ambient light, a sun (directional), a point or a
//! cone ahead of you; pick one to set its colour, strength, reach, direction and cone with
//! sliders. Every change shows at once; it's kept when the slider is let go.

use dioxus::prelude::*;
use serde_json::{Value, json};

use super::{Sp, Win};
use crate::shell::set_pref;
use crate::ui::*;

/// the room's lights into the room (all of them, or baking only where `changed` reach)
pub(super) fn send_lights(sp: Sp, changed: Option<Vec<u64>>) {
    let lights = sp.lights.peek().clone();
    sp.send(json!({ "cmd": "lights", "lights": lights, "changed": changed }));
}

/// the save's lights, read again and sent to the room
pub(super) async fn load_lights(api: &Api, sp: Sp) {
    let all = match api.call("lights.list", json!({})).await {
        Ok(v) => v.as_array().cloned().unwrap_or_default(),
        // a hub from before lights: none
        Err(_) => Vec::new(),
    };
    let mut l = sp.lights;
    l.set(all);
    send_lights(sp, None);
}

/// a slider for a reach: 0–100 for 0.5 to 200 units, evenly in log
fn reach_to_slider(v: f64) -> f64 {
    ((v.clamp(0.5, 200.0) / 0.5).ln() / 400f64.ln() * 100.0).round()
}
fn slider_to_reach(s: f64) -> f64 {
    ((0.5 * 400f64.powf(s / 100.0)) * 10.0).round() / 10.0
}

/// a direction as yaw (degrees round the upright, 0 looking along −z) and pitch (degrees up)
fn yaw_pitch(d: &Value) -> (f64, f64) {
    let (x, y, z) = (
        d[0].as_f64().unwrap_or(0.0),
        d[1].as_f64().unwrap_or(-1.0),
        d[2].as_f64().unwrap_or(0.0),
    );
    let yaw = (-x).atan2(-z).to_degrees().rem_euclid(360.0);
    let pitch = y.clamp(-1.0, 1.0).asin().to_degrees();
    // no "-0°"
    (yaw.round().abs() % 360.0, pitch.round() + 0.0)
}
fn dir_of(yaw: f64, pitch: f64) -> Value {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    let r = |v: f64| (v * 1000.0).round() / 1000.0;
    json!([r(-y.sin() * p.cos()), r(p.sin()), r(-y.cos() * p.cos())])
}

fn kind_words(k: &str) -> &'static str {
    match k {
        "ambient" => "ambient",
        "directional" => "sun",
        "point" => "point",
        "spot" => "cone",
        _ => "light",
    }
}

#[component]
pub(super) fn Lights(sp: Sp) -> Element {
    let ui = ui();
    let api = api();
    let mut picked: Signal<Option<u64>> = use_signal(|| None);
    let mut armed = use_signal(|| false);
    // the markers show while the window is open, the picked one bigger
    use_effect(move || {
        let p = picked();
        sp.send(json!({ "cmd": "showlights", "on": true, "picked": p }));
    });
    // however the window closes
    use_drop(move || sp.send(json!({ "cmd": "showlights", "on": false })));
    let lights = (sp.lights)();
    let now = picked().and_then(|id| lights.iter().find(|l| l["id"].as_u64() == Some(id)).cloned());

    // a field of the picked light changed: shown at once, kept when `keep`
    let api_c = api.clone();
    let change = move |id: u64, k: &'static str, v: Value, keep: bool| {
        let mut ls = sp.lights;
        if let Some(l) = ls.write().iter_mut().find(|l| l["id"].as_u64() == Some(id)) {
            l[k] = v.clone();
        }
        send_lights(sp, Some(vec![id]));
        if keep {
            let api = api_c.clone();
            spawn(async move {
                if let Err(e) = api.call("lights.set", json!({ "id": id, k: v })).await {
                    ui.fail(e);
                }
            });
        }
    };
    let add = move |kind: &'static str| sp.send(json!({ "cmd": "lightspot", "want": { "kind": kind } }));

    rsx! {
        Win {
            id: "lights",
            at: (8.0, 120.0),
            on_close: move |_| set_pref(ui, |p| p.space.show_lights = false),
            title: rsx! {
                span { class: "win-title",
                    Icon { name: "sun" }
                    "lights"
                    span { class: "dim small", style: "font-weight: 400;",
                        {match lights.len() { 0 => "none: faces are shaded by their direction".to_string(), 1 => "1 light".to_string(), n => format!("{n} lights") }}
                    }
                }
            },
            div { class: "row wrap lt-add",
                span { class: "small", "add" }
                button { class: "btn small", title: "light from everywhere, so nothing is black", onclick: move |_| add("ambient"), "ambient" }
                button { class: "btn small", title: "light from one way, the way you look", onclick: move |_| add("directional"), "sun" }
                button { class: "btn small", title: "a light all round, a little ahead of you", onclick: move |_| add("point"), "point" }
                button { class: "btn small", title: "a cone of light ahead of you, shining the way you look", onclick: move |_| add("spot"), "cone" }
            }
            div { class: "lt-list",
                for l in lights.iter() {
                    {
                        let id = l["id"].as_u64().unwrap_or(0);
                        let colour = l["colour"].as_str().unwrap_or("#ffffff").to_string();
                        let k = kind_words(l["kind"].as_str().unwrap_or(""));
                        let i = l["intensity"].as_f64().unwrap_or(1.0);
                        rsx! {
                            button { key: "{id}", class: if picked() == Some(id) { "lt-row on" } else { "lt-row" },
                                onclick: move |_| { picked.set(Some(id)); armed.set(false); },
                                span { class: "lt-swatch", style: "background: {colour};" }
                                span { "{k}" }
                                span { class: "dim small", {format!("strength {i:.2}")} }
                            }
                        }
                    }
                }
            }
            if let Some(l) = now {
                {
                    let id = l["id"].as_u64().unwrap_or(0);
                    let kind = l["kind"].as_str().unwrap_or("ambient").to_string();
                    let placed = kind == "point" || kind == "spot";
                    let aimed = kind == "directional" || kind == "spot";
                    let (yaw, pitch) = yaw_pitch(&l["dir"]);
                    let range = l["range"].as_f64().unwrap_or(10.0);
                    let api = api.clone();
                    let change2 = change.clone();
                    let change3 = change.clone();
                    let change4 = change.clone();
                    let change5 = change.clone();
                    let change6 = change.clone();
                    let change7 = change.clone();
                    let change8 = change.clone();
                    rsx! {
                        div { class: "lt-edit",
                            div { class: "row",
                                b { {kind_words(&kind)} }
                                input { r#type: "color", value: l["colour"].as_str().unwrap_or("#ffffff"), "aria-label": "its colour",
                                    oninput: { let c = change.clone(); move |e: FormEvent| c(id, "colour", json!(e.value()), false) },
                                    onchange: { let c = change.clone(); move |e: FormEvent| c(id, "colour", json!(e.value()), true) } }
                            }
                            label { class: "lt-slider", span { "strength" }
                                input { r#type: "range", min: "0", max: "5", step: "0.05", value: "{l[\"intensity\"].as_f64().unwrap_or(1.0)}",
                                    oninput: move |e| change2(id, "intensity", json!(e.value().parse::<f64>().unwrap_or(1.0)), false),
                                    onchange: move |e| change3(id, "intensity", json!(e.value().parse::<f64>().unwrap_or(1.0)), true) }
                                span { class: "dim small", {format!("{:.2}", l["intensity"].as_f64().unwrap_or(1.0))} }
                            }
                            if placed {
                                label { class: "lt-slider", span { "reach" }
                                    input { r#type: "range", min: "0", max: "100", step: "1", value: "{reach_to_slider(range)}",
                                        oninput: move |e| change4(id, "range", json!(slider_to_reach(e.value().parse().unwrap_or(50.0))), false),
                                        onchange: move |e| change5(id, "range", json!(slider_to_reach(e.value().parse().unwrap_or(50.0))), true) }
                                    span { class: "dim small", {format!("{range:.1} units")} }
                                }
                            }
                            if aimed {
                                label { class: "lt-slider", span { "round" }
                                    input { r#type: "range", min: "0", max: "359", step: "1", value: "{yaw}",
                                        oninput: { let c = change6.clone(); move |e: FormEvent| c(id, "dir", dir_of(e.value().parse().unwrap_or(0.0), pitch), false) },
                                        onchange: { let c = change6.clone(); move |e: FormEvent| c(id, "dir", dir_of(e.value().parse().unwrap_or(0.0), pitch), true) } }
                                    span { class: "dim small", "{yaw}°" }
                                }
                                label { class: "lt-slider", span { "up/down" }
                                    input { r#type: "range", min: "-90", max: "90", step: "1", value: "{pitch}",
                                        oninput: { let c = change7.clone(); move |e: FormEvent| c(id, "dir", dir_of(yaw, e.value().parse().unwrap_or(-45.0)), false) },
                                        onchange: { let c = change7.clone(); move |e: FormEvent| c(id, "dir", dir_of(yaw, e.value().parse().unwrap_or(-45.0)), true) } }
                                    span { class: "dim small", "{pitch}°" }
                                }
                            }
                            if kind == "spot" {
                                label { class: "lt-slider", span { "cone" }
                                    input { r#type: "range", min: "1", max: "89", step: "1", value: "{l[\"angle\"].as_f64().unwrap_or(30.0)}",
                                        oninput: { let c = change8.clone(); move |e: FormEvent| c(id, "angle", json!(e.value().parse::<f64>().unwrap_or(30.0)), false) },
                                        onchange: { let c = change8.clone(); move |e: FormEvent| c(id, "angle", json!(e.value().parse::<f64>().unwrap_or(30.0)), true) } }
                                    span { class: "dim small", {format!("{:.0}°", l["angle"].as_f64().unwrap_or(30.0))} }
                                }
                                label { class: "lt-slider", span { "soft edge" }
                                    input { r#type: "range", min: "0", max: "1", step: "0.05", value: "{l[\"softness\"].as_f64().unwrap_or(0.3)}",
                                        oninput: { let c = change8.clone(); move |e: FormEvent| c(id, "softness", json!(e.value().parse::<f64>().unwrap_or(0.3)), false) },
                                        onchange: { let c = change8.clone(); move |e: FormEvent| c(id, "softness", json!(e.value().parse::<f64>().unwrap_or(0.3)), true) } }
                                }
                            }
                            div { class: "row wrap",
                                if placed || aimed {
                                    button { class: "btn small", title: if placed { "to a little ahead of you, shining the way you look" } else { "shining the way you look" },
                                        onclick: move |_| sp.send(json!({ "cmd": "lightspot", "want": { "move": id, "placed": placed } })),
                                        if placed { "move it here" } else { "the way I look" } }
                                }
                                button {
                                    class: if armed() { "armed small" } else { "plain small" },
                                    title: "delete the light (click twice)",
                                    onclick: move |_| {
                                        if !armed() {
                                            armed.set(true);
                                            return;
                                        }
                                        armed.set(false);
                                        picked.set(None);
                                        let api = api.clone();
                                        spawn(async move {
                                            match api.call("lights.delete", json!({ "id": id })).await {
                                                Ok(_) => {
                                                    let mut ls = sp.lights;
                                                    ls.write().retain(|l| l["id"].as_u64() != Some(id));
                                                    send_lights(sp, Some(vec![id]));
                                                    ui.say("light deleted");
                                                }
                                                Err(e) => ui.fail(e),
                                            }
                                        });
                                    },
                                    Icon { name: "delete" }
                                    if armed() { "delete?" }
                                }
                            }
                        }
                    }
                }
            } else if !lights.is_empty() {
                p { class: "dim small", "Pick a light to change it. Point and cone lights show as dots in the room while this window is open." }
            } else {
                p { class: "dim small", "Lights are baked onto the blocks: worked out when a light or a block changes, not every frame, and without shadows. Texts aren't lit." }
            }
        }
    }
}

/// what the room answered for a new light's place: add it, or move the one asked for
pub(super) fn light_spot(ui: Ui, api: Api, sp: Sp, data: Value) {
    spawn_forever(async move {
        let want = &data["want"];
        let dir = data["dir"].clone();
        let at = json!({ "x": data["x"], "y": data["y"], "z": data["z"] });
        if let Some(id) = want["move"].as_u64() {
            let mut args = json!({ "id": id, "dir": dir });
            if want["placed"].as_bool() == Some(true) {
                for k in ["x", "y", "z"] {
                    args[k] = at[k].clone();
                }
            }
            match api.call("lights.set", args).await {
                Ok(l) => {
                    let mut ls = sp.lights;
                    if let Some(x) = ls.write().iter_mut().find(|x| x["id"].as_u64() == Some(id)) {
                        *x = l;
                    }
                    send_lights(sp, Some(vec![id]));
                }
                Err(e) => ui.fail(e),
            }
            return;
        }
        let kind = want["kind"].as_str().unwrap_or("point");
        let mut light = json!({ "kind": kind, "x": at["x"], "y": at["y"], "z": at["z"], "dir": dir });
        match kind {
            "ambient" => light["intensity"] = json!(0.6),
            // a sun shines down whichever way one looks
            "directional" => {
                let d = &light["dir"];
                let (x, z) = (d[0].as_f64().unwrap_or(0.0), d[2].as_f64().unwrap_or(-1.0));
                light["dir"] = json!([x, -1.0, z]);
                light["intensity"] = json!(0.8);
            }
            "spot" => light["intensity"] = json!(2.0),
            _ => light["intensity"] = json!(1.5),
        }
        match api.call("lights.add", light).await {
            Ok(l) => {
                let id = l["id"].as_u64();
                let mut ls = sp.lights;
                ls.write().push(l);
                send_lights(sp, id.map(|i| vec![i]));
                ui.say("a light · pick it in the lights window to change it");
            }
            Err(e) => ui.fail(e),
        }
    });
}

fn spawn_forever(f: impl std::future::Future<Output = ()> + 'static) {
    dioxus::dioxus_core::spawn_forever(f);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_round_trip() {
        for (yaw, pitch) in [(0.0, -45.0), (90.0, 10.0), (200.0, -80.0), (359.0, 0.0)] {
            let (y, p) = yaw_pitch(&dir_of(yaw, pitch));
            assert!(
                (y - yaw).abs() <= 1.0 && (p - pitch).abs() <= 1.0,
                "{yaw} {pitch} → {y} {p}"
            );
        }
        assert!((slider_to_reach(reach_to_slider(10.0)) - 10.0).abs() < 0.7);
    }
}
