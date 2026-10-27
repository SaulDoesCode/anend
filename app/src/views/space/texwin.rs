//! The textures window: texture recipes (core's `graph::textures`) built step by step, as a
//! pipeline from a source to where it's used, each step with a preview drawn by the room
//! itself (`space.js` `preview`), so what the window shows is what blocks show.
//!
//! Source (a picture of the save, the selected text, or a colour) → crop (drag a box on the
//! source) → tile (its size in units, its offset, a quarter turn, a mirror, a tint) →
//! variants (how many, shuffled by a seed) → use (on the selected block, on one of its
//! faces, as the sky, on a new block). Recipes are kept by their content hash, so making
//! the same one twice keeps one; the saved ones are listed at the top to pick up again.

use dioxus::prelude::*;
use serde_json::{Value, json};

use super::{SpaceEvent, Win, add_block_from, dress};
use crate::gem::{self, Mode};
use crate::shell::set_pref;
use crate::ui::*;

/// a fresh recipe: a colour, one unit a tile
fn blank() -> Value {
    json!({
        "source": { "plain": "#b49a6c" },
        "crop": [0.0, 0.0, 1.0, 1.0],
        "scale": [1.0, 1.0],
        "offset": [0.0, 0.0],
        "turn": 0,
        "mirror": false,
        "tint": "#000000",
        "tint_amount": 0.0,
        "variants": 1,
        "seed": 0,
        "turn_variants": false,
    })
}

/// a recipe with every field there (one from the save may leave defaults out)
fn filled(r: &Value) -> Value {
    let mut out = blank();
    if let Some(o) = r.as_object() {
        for (k, v) in o {
            if !v.is_null() {
                out[k] = v.clone();
            }
        }
    }
    out
}

fn f(v: &Value, i: usize) -> f64 {
    v.get(i).and_then(Value::as_f64).unwrap_or(0.0)
}

/// a tile's side on a slider: 0–100 for 0.1 to 10 units, evenly in log
fn side_to_slider(v: f64) -> f64 {
    ((v.clamp(0.1, 10.0) / 0.1).ln() / 100f64.ln() * 100.0).round()
}
fn slider_to_side(s: f64) -> f64 {
    let v = 0.1 * 100f64.powf(s / 100.0);
    (v * 100.0).round() / 100.0
}

/// what a recipe's source is, in words
fn source_words(r: &Value, pictures: &[(String, String, u64, u64)], text: &str) -> String {
    let s = &r["source"];
    if let Some(c) = s["plain"].as_str() {
        format!("the colour {c}")
    } else if let Some(p) = s["picture"].as_str() {
        let name = pictures
            .iter()
            .find(|x| x.0 == p)
            .map(|x| x.1.clone())
            .unwrap_or_else(|| p[..8.min(p.len())].to_string());
        format!("the picture {name}")
    } else if s["text"].is_string() {
        format!("the text “{}”", text.chars().take(40).collect::<String>())
    } else {
        "nothing yet".into()
    }
}

#[component]
pub(super) fn Textures(
    selected: Option<SpaceEvent>,
    block: Option<Value>,
    on_send: EventHandler<Value>,
    on_changed: EventHandler<()>,
) -> Element {
    let ui = ui();
    let api = api();
    // the saved recipes, dressed for drawing
    let mut saved: Signal<Vec<Value>> = use_signal(Vec::new);
    let mut rev = use_signal(|| 0u64);
    // pictures: id, name, width, height
    let mut pictures: Signal<Vec<(String, String, u64, u64)>> = use_signal(Vec::new);
    // the recipe being made, its name, and what its source draws from (a picture's
    // thumbnail, a text)
    let mut recipe = use_signal(blank);
    let mut name = use_signal(String::new);
    let src_url = use_signal(String::new);
    let mut src_text = use_signal(String::new);
    let mut sky = use_signal(String::new);
    let mut armed: Signal<Option<String>> = use_signal(|| None);
    // a crop being dragged: where it started, as fractions of the source
    let mut crop_from: Signal<Option<(f64, f64)>> = use_signal(|| None);

    let api_l = api.clone();
    use_effect(move || {
        let _ = rev();
        let api = api_l.clone();
        spawn(async move {
            if let Ok(v) = api.call("textures.list", json!({})).await {
                let mut all = Vec::new();
                for t in v.as_array().into_iter().flatten() {
                    let id = t["id"].as_str().unwrap_or("").to_string();
                    let dressed = dress(ui, &api, json!({ format!("texture:{id}"): t })).await;
                    all.push(dressed[format!("texture:{id}")].clone());
                }
                saved.set(all);
            }
            if let Ok(v) = api.call("settings.get", json!({ "name": "space.sky" })).await {
                sky.set(v["value"].as_str().unwrap_or("").to_string());
            }
        });
    });
    let api_p = api.clone();
    use_hook(move || {
        let api = api_p.clone();
        spawn(async move {
            if let Ok(v) = api.call("pictures.list", json!({})).await {
                let all = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|p| {
                        let id = p["id"].as_str().unwrap_or("").to_string();
                        let name = p["name"]
                            .as_str()
                            .filter(|n| !n.is_empty())
                            .unwrap_or(&id[..id.len().min(8)])
                            .to_string();
                        (
                            id,
                            name,
                            p["width"].as_u64().unwrap_or(1),
                            p["height"].as_u64().unwrap_or(1),
                        )
                    })
                    .collect();
                pictures.set(all);
            }
        });
    });

    // what the room needs to draw the recipe being made
    let about_now = move || {
        let r = recipe();
        let mut a = json!({ "recipe": r });
        if r["source"]["picture"].is_string() {
            a["src"] = json!(src_url());
        }
        if r["source"]["text"].is_string() {
            let t = src_text();
            a["colour"] = json!(gem::underline(&t, Mode::Ordinal));
            a["text"] = json!(t);
        }
        a
    };
    // the previews: the whole source (to crop), one tile, a patch of two by two units
    // with the variants, and each saved recipe
    use_effect(move || {
        let a = about_now();
        let mut whole = a.clone();
        whole["recipe"] = json!({ "source": a["recipe"]["source"], "crop": [0, 0, 1, 1] });
        on_send.call(json!({ "cmd": "preview", "id": "tx-source", "about": whole, "tiles": 1 }));
        on_send.call(json!({ "cmd": "preview", "id": "tx-tile", "about": a, "tiles": 1 }));
        // one variant: two by two units of it; more: a tile of each, side by side
        let sw = f(&a["recipe"]["scale"], 0).max(0.05);
        let n = a["recipe"]["variants"].as_u64().unwrap_or(1);
        let across = if n > 1 {
            (n as f64).sqrt().ceil()
        } else {
            (2.0 / sw).round().clamp(1.0, 24.0)
        };
        on_send.call(json!({ "cmd": "preview", "id": "tx-patch", "about": a, "tiles": across, "variants": n > 1 }));
    });
    use_effect(move || {
        for t in saved() {
            let id = t["id"].as_str().unwrap_or("").to_string();
            on_send.call(json!({ "cmd": "preview", "id": format!("tx-saved-{id}"), "about": t, "tiles": 2 }));
        }
    });

    let set = move |k: &str, v: Value| {
        let mut recipe = recipe;
        let mut r = recipe.write();
        r[k] = v;
    };
    let set_at = move |k: &str, i: usize, v: f64| {
        let mut recipe = recipe;
        let mut r = recipe.write();
        r[k][i] = json!(v);
    };
    // a picture as the source: its thumbnail fetched for drawing
    let api_s = api.clone();
    let use_picture = move |id: String| {
        let api = api_s.clone();
        let (mut recipe, mut src_url) = (recipe, src_url);
        recipe.write()["source"] = json!({ "picture": id.clone() });
        recipe.write()["crop"] = json!([0.0, 0.0, 1.0, 1.0]);
        spawn(async move {
            if let Ok(p) = api.call("pictures.get", json!({ "id": id, "thumb": 512 })).await
                && let Some(t) = p["thumb"].as_str()
            {
                src_url.set(format!("data:image/png;base64,{t}"));
            }
        });
    };
    let api_k = api.clone();
    // keeps the recipe and answers its id
    let keep = move || {
        let api = api_k.clone();
        let r = recipe();
        let n = name();
        async move {
            match api.call("textures.save", json!({ "recipe": r, "name": n })).await {
                Ok(t) => {
                    rev += 1;
                    Some(t["id"].as_str().unwrap_or("").to_string())
                }
                Err(e) => {
                    ui.fail(e);
                    None
                }
            }
        }
    };

    let r = recipe();
    let src = &r["source"];
    let pics = pictures();
    let block_id = block.as_ref().and_then(|b| b["id"].as_str()).map(str::to_string);
    let text = selected.clone().filter(|s| !s.free && !s.key.is_empty());
    // the source's shape on screen, so the crop box is drawn over it as it is
    let (sw_px, sh_px) = match src["picture"].as_str().and_then(|p| pics.iter().find(|x| x.0 == p)) {
        Some((_, _, w, h)) => {
            let (w, h) = (*w as f64, *h as f64);
            if w >= h {
                (240.0, (240.0 * h / w).max(40.0))
            } else {
                ((240.0 * w / h).max(40.0), 240.0)
            }
        }
        None => (240.0, 240.0),
    };
    let crop = r["crop"].clone();
    let (cl, ct, cr, cb) = (f(&crop, 0), f(&crop, 1), f(&crop, 2), f(&crop, 3));
    let scale = r["scale"].clone();
    let (tw, th) = (f(&scale, 0), f(&scale, 1));
    let offset = r["offset"].clone();
    let turn = r["turn"].as_u64().unwrap_or(0);
    let variants = r["variants"].as_u64().unwrap_or(1);
    let tint = r["tint"].as_str().unwrap_or("#000000").to_string();
    let tint_amount = r["tint_amount"].as_f64().unwrap_or(0.0);
    let sky_now = sky();
    let words = source_words(&r, &pics, &src_text());
    let pic_shape = src["picture"]
        .as_str()
        .and_then(|p| pics.iter().find(|x| x.0 == p))
        .map(|x| (x.2 as f64, x.3 as f64));

    rsx! {
        Win {
            id: "textures",
            at: (8.0, 60.0),
            on_close: move |_| set_pref(ui, |p| p.space.show_textures = false),
            title: rsx! {
                span { class: "win-title",
                    Icon { name: "image" }
                    "textures"
                    span { class: "dim small", style: "font-weight: 400;", {match saved.read().len() { 0 => "no recipes yet".to_string(), 1 => "1 recipe".to_string(), n => format!("{n} recipes") }} }
                }
            },
            if !saved.read().is_empty() {
                div { class: "tx-saved",
                    for t in saved() {
                        {
                            let id = t["id"].as_str().unwrap_or("").to_string();
                            let label = t["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| id[..8].to_string());
                            let is_sky = sky_now == id;
                            rsx! {
                                div { key: "{id}", class: "tx-item", title: "{label}: open it to change or use it",
                                    canvas { id: "tx-saved-{id}", width: "56", height: "56",
                                        onclick: {
                                            let t = t.clone();
                                            let use_picture = use_picture.clone();
                                            move |_| {
                                                recipe.set(filled(&t["recipe"]));
                                                name.set(t["name"].as_str().unwrap_or("").to_string());
                                                src_text.set(t["text"].as_str().unwrap_or("").to_string());
                                                if let Some(p) = t["recipe"]["source"]["picture"].as_str() {
                                                    let crop = t["recipe"]["crop"].clone();
                                                    use_picture(p.to_string());
                                                    recipe.write()["crop"] = crop;
                                                }
                                            }
                                        },
                                    }
                                    span { class: "small", if is_sky { "☁ " } "{label}" }
                                    button {
                                        class: if armed() == Some(id.clone()) { "armed small" } else { "plain small" },
                                        title: "delete the recipe (blocks showing it draw grey)",
                                        onclick: {
                                            let id = id.clone();
                                            let api = api.clone();
                                            move |_| {
                                                if armed() == Some(id.clone()) {
                                                    armed.set(None);
                                                    let (api, id) = (api.clone(), id.clone());
                                                    spawn(async move {
                                                        match api.call("textures.delete", json!({ "id": id })).await {
                                                            Ok(_) => { rev += 1; ui.say("recipe deleted"); }
                                                            Err(e) => ui.fail(e),
                                                        }
                                                    });
                                                } else {
                                                    armed.set(Some(id.clone()));
                                                }
                                            }
                                        },
                                        Icon { name: "delete" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "tx-steps",
                // 1. the source
                div { class: "tx-step",
                    div { class: "tx-step-head", span { class: "tx-n", "1" } b { "source" } span { class: "dim small", "{words}" } }
                    div { class: "row wrap",
                        if !pics.is_empty() {
                            select {
                                "aria-label": "a picture",
                                onchange: { let use_picture = use_picture.clone(); move |e: FormEvent| { let v = e.value(); if !v.is_empty() { use_picture(v) } } },
                                option { value: "", selected: !src["picture"].is_string(), "a picture…" }
                                for (id , n , _ , _) in pics.iter() {
                                    option { key: "{id}", value: "{id}", selected: src["picture"].as_str() == Some(id.as_str()), "{n}" }
                                }
                            }
                        }
                        button { class: "btn small", disabled: text.is_none(), title: "the text selected in the room, drawn as a text block draws it",
                            onclick: {
                                let text = text.clone();
                                move |_| if let Some(t) = &text {
                                    recipe.write()["source"] = json!({ "text": t.key.clone() });
                                    recipe.write()["crop"] = json!([0.0, 0.0, 1.0, 1.0]);
                                    src_text.set(t.text.clone());
                                }
                            },
                            Icon { name: "text" } "the selected text" }
                        label { class: "row", title: "a plain colour",
                            input { r#type: "color", value: src["plain"].as_str().unwrap_or("#b49a6c"),
                                oninput: move |e| { recipe.write()["source"] = json!({ "plain": e.value() }); } }
                            span { class: "small", "colour" }
                        }
                    }
                }
                div { class: "tx-arrow", "↓" }
                // 2. the crop
                div { class: "tx-step",
                    div { class: "tx-step-head", span { class: "tx-n", "2" } b { "crop" }
                        span { class: "dim small",
                            if src["plain"].is_string() { "a colour has nothing to crop" } else { "drag a box over the part to use" }
                        }
                    }
                    div { class: "tx-crop", hidden: src["plain"].is_string(), style: "width: {sw_px}px; height: {sh_px}px;",
                        canvas { id: "tx-source", width: "{sw_px.round()}", height: "{sh_px.round()}" }
                        div { class: "tx-cropbox", style: "left: {cl * 100.0}%; top: {ct * 100.0}%; width: {(cr - cl) * 100.0}%; height: {(cb - ct) * 100.0}%;" }
                        div { class: "tx-crop-grip",
                            onpointerdown: move |e| {
                                if recipe.peek()["source"]["plain"].is_string() { return; }
                                e.prevent_default();
                                let o = e.element_coordinates();
                                let (x, y) = ((o.x / sw_px).clamp(0.0, 1.0), (o.y / sh_px).clamp(0.0, 1.0));
                                crop_from.set(Some((x, y)));
                                recipe.write()["crop"] = json!([x, y, (x + 0.01).min(1.0), (y + 0.01).min(1.0)]);
                            },
                            onpointermove: move |e| {
                                if let Some((x0, y0)) = crop_from() {
                                    let o = e.element_coordinates();
                                    let (x, y) = ((o.x / sw_px).clamp(0.0, 1.0), (o.y / sh_px).clamp(0.0, 1.0));
                                    let r4 = |v: f64| (v * 1000.0).round() / 1000.0;
                                    recipe.write()["crop"] = json!([r4(x0.min(x)), r4(y0.min(y)), r4(x0.max(x)), r4(y0.max(y))]);
                                }
                            },
                            onpointerup: move |_| crop_from.set(None),
                            onpointerleave: move |_| crop_from.set(None),
                        }
                    }
                    div { class: "row",
                        span { class: "dim small", {format!("{:.0}% × {:.0}% of it", (cr - cl) * 100.0, (cb - ct) * 100.0)} }
                        button { class: "plain small", onclick: move |_| set("crop", json!([0.0, 0.0, 1.0, 1.0])), "all of it" }
                    }
                }
                div { class: "tx-arrow", "↓" }
                // 3. the tile
                div { class: "tx-step",
                    div { class: "tx-step-head", span { class: "tx-n", "3" } b { "tile" } span { class: "dim small", {format!("{tw:.2} × {th:.2} units")} } }
                    div { class: "tx-tile-row",
                        canvas { id: "tx-tile", width: "96", height: "96" }
                        div { class: "tx-sliders",
                            label { span { "width" } input { r#type: "range", min: "0", max: "100", step: "1", value: "{side_to_slider(tw)}", oninput: move |e| set_at("scale", 0, slider_to_side(e.value().parse().unwrap_or(50.0))) } }
                            label { span { "height" } input { r#type: "range", min: "0", max: "100", step: "1", value: "{side_to_slider(th)}", oninput: move |e| set_at("scale", 1, slider_to_side(e.value().parse().unwrap_or(50.0))) } }
                            label { span { "across" } input { r#type: "range", min: "0", max: "1", step: "0.01", value: "{f(&offset, 0)}", oninput: move |e| set_at("offset", 0, e.value().parse().unwrap_or(0.0)) } }
                            label { span { "down" } input { r#type: "range", min: "0", max: "1", step: "0.01", value: "{f(&offset, 1)}", oninput: move |e| set_at("offset", 1, e.value().parse().unwrap_or(0.0)) } }
                        }
                    }
                    div { class: "row wrap",
                        for (q , label) in [(0u64, "0°"), (1, "90°"), (2, "180°"), (3, "270°")] {
                            button { key: "{q}", class: if turn == q { "btn small on" } else { "plain small" }, title: "turn the tile", onclick: move |_| set("turn", json!(q)), "{label}" }
                        }
                        label { class: "row small", input { r#type: "checkbox", checked: r["mirror"].as_bool().unwrap_or(false), oninput: move |e| set("mirror", json!(e.checked())) } "mirror" }
                    }
                    div { class: "row wrap",
                        if let Some((pw, ph)) = pic_shape {
                            button { class: "plain small", title: "the tile's height from its width, so the crop keeps its shape",
                                onclick: move |_| {
                                    let r = recipe.peek().clone();
                                    let (cw, ch) = ((f(&r["crop"], 2) - f(&r["crop"], 0)) * pw, (f(&r["crop"], 3) - f(&r["crop"], 1)) * ph);
                                    let w = f(&r["scale"], 0);
                                    if cw > 0.0 { set_at("scale", 1, ((w * ch / cw) * 100.0).round() / 100.0) }
                                },
                                "keep the crop's shape" }
                        }
                        span { class: "small", "tint" }
                        input { r#type: "color", value: "{tint}", "aria-label": "the tint", oninput: move |e| set("tint", json!(e.value())) }
                        input { r#type: "range", min: "0", max: "1", step: "0.01", value: "{tint_amount}", "aria-label": "how strong the tint is", oninput: move |e| set("tint_amount", json!(e.value().parse::<f64>().unwrap_or(0.0))) }
                        span { class: "dim small", {format!("{:.0}%", tint_amount * 100.0)} }
                    }
                }
                div { class: "tx-arrow", "↓" }
                // 4. the variants
                div { class: "tx-step",
                    div { class: "tx-step-head", span { class: "tx-n", "4" } b { "variants" }
                        span { class: "dim small", if variants <= 1 { "every block the same" } else { "{variants}, each block picks one" } }
                    }
                    div { class: "row wrap",
                        input { r#type: "range", min: "1", max: "16", step: "1", value: "{variants}", "aria-label": "how many variants", oninput: move |e| set("variants", json!(e.value().parse::<u64>().unwrap_or(1))) }
                        button { class: "plain small", disabled: variants <= 1, title: "another seed: the blocks pick again",
                            onclick: move |_| { let s = (js_random() * 4_294_967_295.0) as u64; set("seed", json!(s)) }, "shuffle" }
                        label { class: "row small", input { r#type: "checkbox", checked: r["turn_variants"].as_bool().unwrap_or(false), disabled: variants <= 1, oninput: move |e| set("turn_variants", json!(e.checked())) } "turned too" }
                    }
                    div { class: "dim small", if variants <= 1 { "two by two units, as a block shows it:" } else { "a tile of each variant (a block shows one all over):" } }
                    canvas { id: "tx-patch", class: "tx-patch", width: "200", height: "200" }
                }
                div { class: "tx-arrow", "↓" }
                // 5. where it's used
                div { class: "tx-step",
                    div { class: "tx-step-head", span { class: "tx-n", "5" } b { "use" }
                        input { r#type: "text", class: "tx-name", placeholder: "its name (optional)", value: "{name}", oninput: move |e| name.set(e.value()) }
                    }
                    div { class: "row wrap",
                        button { class: "btn small", title: "keep the recipe in the save",
                            onclick: { let keep = keep.clone(); move |_| { let k = keep(); spawn(async move { if k.await.is_some() { ui.say("recipe kept") } }); } },
                            Icon { name: "save" } "keep" }
                        button { class: "btn small", title: "a new block showing it, ahead of you",
                            onclick: { let keep = keep.clone(); move |_| { let k = keep(); spawn(async move { if let Some(id) = k.await { add_block_from(on_send, "texture", &id) } }); } },
                            Icon { name: "cube" } "a new block" }
                        button { class: "btn small", disabled: block_id.is_none(), title: "the selected block shows it (on every face that has no recipe of its own)",
                            onclick: {
                                let keep = keep.clone();
                                let api = api.clone();
                                let bid = block_id.clone();
                                move |_| {
                                    let (k, api, bid) = (keep(), api.clone(), bid.clone());
                                    spawn(async move {
                                        let (Some(id), Some(bid)) = (k.await, bid) else { return };
                                        match api.call("blocks.retexture", json!({ "id": bid, "kind": "texture", "texture": id })).await {
                                            Ok(r) => {
                                                let textures = dress(ui, &api, r["textures"].clone()).await;
                                                on_send.call(json!({ "cmd": "unblock", "ids": [bid] }));
                                                on_send.call(json!({ "cmd": "blocks", "blocks": r["blocks"], "textures": textures, "select": r["ids"][0] }));
                                                on_changed.call(());
                                                ui.say("the block shows the recipe");
                                            }
                                            Err(e) => ui.fail(e),
                                        }
                                    });
                                }
                            },
                            Icon { name: "image" } "on the selected block" }
                    }
                    if let Some(b) = block.clone() {
                        div { class: "row wrap tx-faces",
                            span { class: "small", "on one face:" }
                            for (i , face) in ["right", "left", "top", "bottom", "front", "back"].into_iter().enumerate() {
                                {
                                    let has = b["faces"].get(i).is_some_and(|v| !v.is_null());
                                    let keep = keep.clone();
                                    let api = api.clone();
                                    let b = b.clone();
                                    rsx! {
                                        button { key: "{face}", class: if has { "btn small on" } else { "plain small" },
                                            title: if has { "this face has a recipe of its own; click to put this one there" } else { "put the recipe on this face only" },
                                            onclick: move |_| {
                                                let (k, api, b) = (keep(), api.clone(), b.clone());
                                                spawn(async move {
                                                    let Some(id) = k.await else { return };
                                                    let mut faces = b["faces"].as_array().cloned().unwrap_or_else(|| vec![Value::Null; 6]);
                                                    faces.resize(6, Value::Null);
                                                    faces[i] = json!(id);
                                                    set_faces(ui, api, on_send, &b, Value::Array(faces)).await;
                                                });
                                            },
                                            "{face}" }
                                    }
                                }
                            }
                            if b["faces"].as_array().is_some_and(|a| a.iter().any(|v| !v.is_null())) {
                                button { class: "plain small", title: "every face shows the block's own texture again",
                                    onclick: { let api = api.clone(); let b = b.clone(); move |_| { let (api, b) = (api.clone(), b.clone()); spawn(async move { set_faces(ui, api, on_send, &b, Value::Null).await }); } },
                                    "clear the faces" }
                            }
                        }
                    }
                    div { class: "row wrap",
                        button { class: "btn small", title: "the room's sky: the recipe all round, far off",
                            onclick: {
                                let keep = keep.clone();
                                let api = api.clone();
                                move |_| {
                                    let (k, api) = (keep(), api.clone());
                                    spawn(async move {
                                        let Some(id) = k.await else { return };
                                        set_sky(ui, &api, on_send, &id).await;
                                        sky.set(id);
                                    });
                                }
                            },
                            "☁ as the sky" }
                        if !sky_now.is_empty() {
                            button { class: "plain small", title: "no sky: the room's colour again",
                                onclick: { let api = api.clone(); move |_| { let api = api.clone(); spawn(async move { set_sky(ui, &api, on_send, "").await; sky.set(String::new()); }); } },
                                "no sky" }
                        }
                    }
                }
            }
        }
    }
}

/// a random number in [0, 1) (the clock has no time in a browser's wasm; Math.random does)
#[cfg(target_arch = "wasm32")]
fn js_random() -> f64 {
    js_sys::Math::random()
}

/// a random number in [0, 1)
#[cfg(not(target_arch = "wasm32"))]
fn js_random() -> f64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(7);
    let mut x = t as u64 ^ 0x9e37_79b9_7f4a_7c15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// a block's faces set (null clears them), and the room told
async fn set_faces(ui: Ui, api: Api, on_send: EventHandler<Value>, b: &Value, faces: Value) {
    let id = b["id"].as_str().unwrap_or("").to_string();
    match api.call("blocks.set", json!({ "id": id, "faces": faces })).await {
        Ok(r) => {
            let textures = dress(ui, &api, r["textures"].clone()).await;
            on_send.call(json!({ "cmd": "blocks", "blocks": r["blocks"], "textures": textures, "select": id }));
            ui.say(if faces.is_null() {
                "the faces show the block's texture again"
            } else {
                "the face shows the recipe"
            });
        }
        Err(e) => ui.fail(e),
    }
}

/// the room's sky: a recipe's id in the save's settings (`space.sky`), "" for none; the
/// room is told
pub(super) async fn set_sky(ui: Ui, api: &Api, on_send: EventHandler<Value>, id: &str) {
    if let Err(e) = api
        .call("settings.set", json!({ "name": "space.sky", "value": id }))
        .await
    {
        return ui.fail(e);
    }
    send_sky(ui, api, on_send, id).await;
    ui.say(if id.is_empty() {
        "no sky"
    } else {
        "the sky shows the recipe"
    });
}

/// the room's sky drawn from the save's setting (on opening the room)
pub(super) async fn load_sky(ui: Ui, api: &Api, on_send: EventHandler<Value>) {
    let id = match api.call("settings.get", json!({ "name": "space.sky" })).await {
        Ok(v) => v["value"].as_str().unwrap_or("").to_string(),
        Err(_) => String::new(),
    };
    send_sky(ui, api, on_send, &id).await;
}

async fn send_sky(ui: Ui, api: &Api, on_send: EventHandler<Value>, id: &str) {
    if id.is_empty() {
        on_send.call(json!({ "cmd": "sky", "about": null }));
        return;
    }
    let Ok(all) = api.call("textures.list", json!({})).await else {
        return;
    };
    let Some(t) = all
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["id"].as_str() == Some(id))
        .cloned()
    else {
        on_send.call(json!({ "cmd": "sky", "about": null }));
        return;
    };
    let dressed = dress(ui, api, json!({ format!("texture:{id}"): t })).await;
    on_send.call(json!({ "cmd": "sky", "about": dressed[format!("texture:{id}")] }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_on_a_slider() {
        for v in [0.1, 0.5, 1.0, 2.0, 10.0] {
            assert!((slider_to_side(side_to_slider(v)) - v).abs() < v * 0.06, "{v}");
        }
        assert_eq!(slider_to_side(50.0), 1.0);
        assert_eq!(
            filled(&json!({ "source": { "plain": "#000000" }, "turn": 2 }))["scale"],
            json!([1.0, 1.0])
        );
    }
}
