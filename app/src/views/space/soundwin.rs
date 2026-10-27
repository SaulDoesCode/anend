//! The sounds window: the save's sounds (core's `graph::sounds`, `ops::sounds`) and the
//! selected block's emitter. Add sound files (WAV, OGG, MP3, FLAC, WebM or M4A), hear one
//! with ▶, rename (a double click) or delete one. The selected block plays a sound of
//! the save: how loud, how far it carries (it fades to nothing there), and whether it
//! loops. While the room plays (the wiring window's ▶ play), looping sounds play from the
//! start and the others when a program prints `sound`; each is heard from where it is.

use dioxus::prelude::*;
use serde_json::{Value, json};

use super::{Sp, Win, dress};
use crate::shell::set_pref;
use crate::ui::*;

#[component]
pub(super) fn Sounds(sp: Sp) -> Element {
    let ui = ui();
    let api = api();
    let mut list: Signal<Vec<Value>> = use_signal(Vec::new);
    let mut rev = use_signal(|| 0u64);
    let mut busy = use_signal(|| 0usize);
    let mut renaming: Signal<Option<String>> = use_signal(|| None);
    let mut rename_to = use_signal(String::new);
    let mut deleting: Signal<Option<String>> = use_signal(|| None);

    let api_l = api.clone();
    use_effect(move || {
        let _ = rev();
        let api = api_l.clone();
        spawn(async move {
            if let Ok(v) = api.call("sounds.list", json!({})).await {
                list.set(v.as_array().cloned().unwrap_or_default());
            }
        });
    });

    let take = {
        let api = api.clone();
        move |files: Vec<dioxus::html::FileData>| {
            let api = api.clone();
            async move {
                for f in files {
                    busy += 1;
                    let name = f.name();
                    match f.read_bytes().await {
                        Ok(b) => match api
                            .call(
                                "sounds.add",
                                json!({ "data": anend_core::classifiers::base64(&b), "name": name }),
                            )
                            .await
                        {
                            Ok(v) => ui.say(if v["new"] == true {
                                format!("{name} is in")
                            } else {
                                format!("the save had {name} already")
                            }),
                            Err(e) => ui.fail(format!("{name}: {e}")),
                        },
                        Err(e) => ui.fail(format!("couldn't read {name}: {e}")),
                    }
                    busy -= 1;
                }
                rev += 1;
            }
        }
    };

    let block = (sp.block)();
    let bid = block.as_ref().and_then(|b| b["id"].as_str().map(str::to_string));
    let em = block.as_ref().map(|b| b["sound"].clone()).unwrap_or(Value::Null);
    // the selected block's emitter changed: kept, and the room shows it
    let api_s = api.clone();
    let set_sound = move |v: Value| {
        let Some(id) = bid.clone() else { return };
        let api = api_s.clone();
        spawn(async move {
            match api.call("blocks.set", json!({ "id": id, "sound": v })).await {
                Ok(r) => {
                    let textures = dress(ui, &api, r["textures"].clone()).await;
                    sp.send(json!({ "cmd": "blocks", "blocks": r["blocks"], "textures": textures }));
                    if let Some(b) = r["blocks"].as_array().and_then(|a| a.first()) {
                        let mut s = sp.block;
                        let mut v = b.clone();
                        if let Some(old) = s.peek().as_ref() {
                            v["about"] = old["about"].clone();
                        }
                        s.set(Some(v));
                    }
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let sounds = list();
    let field = |k: &str, d: f64| em.get(k).and_then(Value::as_f64).unwrap_or(d);
    let (vol, range, looped) = (field("volume", 1.0), field("range", 20.0), em["loop"] == true);
    let cur = em["id"].as_str().map(str::to_string);

    rsx! {
        Win {
            id: "sounds",
            at: (8.0, 180.0),
            on_close: move |_| set_pref(ui, |p| p.space.show_sounds = false),
            title: rsx! {
                span { class: "win-title",
                    Icon { name: "sound" }
                    "sounds"
                    span { class: "dim small", style: "font-weight: 400;", "emitters in the room" }
                }
            },
            if let Some(id) = block.as_ref().and_then(|b| b["id"].as_str()) {
                div { class: "wire-sec",
                    span { class: "small", "the selected block " b { {id.rsplit_once('/').map_or(id, |x| x.1).to_string()} } " plays" }
                    select { "aria-label": "the sound it plays",
                        onchange: {
                            let set_sound = set_sound.clone();
                            move |e: FormEvent| {
                                let v = e.value();
                                set_sound(if v.is_empty() { Value::Null } else { json!({ "id": v, "volume": vol, "range": range, "loop": looped }) });
                            }
                        },
                        option { value: "", selected: cur.is_none(), "no sound" }
                        for s in sounds.iter() {
                            {
                                let sid = s["id"].as_str().unwrap_or("").to_string();
                                let name = s["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&sid[..8.min(sid.len())]).to_string();
                                rsx! { option { key: "{sid}", value: "{sid}", selected: cur.as_deref() == Some(sid.as_str()), "{name}" } }
                            }
                        }
                    }
                    if let Some(c) = cur.clone() {
                        label { class: "lt-slider", span { "volume" }
                            input { r#type: "range", min: "0", max: "200", value: "{(vol * 100.0).round()}",
                                onchange: { let set_sound = set_sound.clone(); let c = c.clone(); move |e: FormEvent| { let v = e.value().parse::<f64>().unwrap_or(100.0) / 100.0; set_sound(json!({ "id": c, "volume": v, "range": range, "loop": looped })); } }
                            }
                            span { class: "dim small", {format!("{:.0}%", vol * 100.0)} }
                        }
                        label { class: "lt-slider", span { "reach" }
                            input { r#type: "range", min: "1", max: "200", value: "{range.round()}",
                                onchange: { let set_sound = set_sound.clone(); let c = c.clone(); move |e: FormEvent| { let v = e.value().parse::<f64>().unwrap_or(20.0); set_sound(json!({ "id": c, "volume": vol, "range": v, "loop": looped })); } }
                            }
                            span { class: "dim small", {format!("{range:.0}")} }
                        }
                        label { class: "check small",
                            input { r#type: "checkbox", checked: looped,
                                onchange: { let set_sound = set_sound.clone(); let c = c.clone(); move |e: FormEvent| set_sound(json!({ "id": c, "volume": vol, "range": range, "loop": e.checked() })) }
                            }
                            "loops (plays from the start while the room plays)"
                        }
                        p { class: "dim small", "Not looping, it plays when a program prints sound (and stops at hush), while the room plays." }
                    }
                }
            } else {
                p { class: "dim small", "Select a block to make it a sound emitter." }
            }
            div { class: "wire-sec",
                div { class: "row",
                    span { class: "small", if sounds.is_empty() { "no sounds yet" } else { "the save's sounds" } }
                    span { class: "grow" }
                    if busy() > 0 { span { class: "dim small", "adding…" } }
                    label { class: "button btn small", "data-tip": "WAV, OGG, MP3, FLAC, WebM or M4A files",
                        Icon { name: "upload" } "add sounds"
                        input { r#type: "file", accept: "audio/*,.wav,.ogg,.mp3,.flac,.webm,.m4a", multiple: true, style: "display: none;",
                            "aria-label": "sounds to add",
                            onchange: move |e: FormEvent| { let take = take.clone(); async move { take(e.files()).await } },
                        }
                    }
                }
                for s in sounds.iter() {
                    {
                        let sid = s["id"].as_str().unwrap_or("").to_string();
                        let name = s["name"].as_str().unwrap_or("").to_string();
                        let secs = s["seconds"].as_f64().map(|x| format!("{x:.1} s · ")).unwrap_or_default();
                        let kb = s["size"].as_u64().unwrap_or(0).div_ceil(1024);
                        let mime = s["mime"].as_str().unwrap_or("").trim_start_matches("audio/").to_string();
                        let (s1, s2, s3, s4) = (sid.clone(), sid.clone(), sid.clone(), sid.clone());
                        let (api_r, api_d) = (api.clone(), api.clone());
                        rsx! {
                            div { key: "{sid}", class: "row snd-row",
                                button { class: "btn small", title: "hear it", "aria-label": "hear {name}", onclick: move |_| sp.send(json!({ "cmd": "soundpreview", "id": s1 })), "▶" }
                                if renaming().as_deref() == Some(sid.as_str()) {
                                    input { r#type: "text", value: "{rename_to}", "aria-label": "its new name",
                                        oninput: move |e: FormEvent| rename_to.set(e.value()),
                                        onkeydown: move |e: KeyboardEvent| {
                                            if e.key() == Key::Enter {
                                                let (api, id, to) = (api_r.clone(), s2.clone(), rename_to.peek().clone());
                                                renaming.set(None);
                                                spawn(async move {
                                                    match api.call("sounds.name", json!({ "id": id, "name": to })).await {
                                                        Ok(_) => rev += 1,
                                                        Err(e) => ui.fail(e),
                                                    }
                                                });
                                            } else if e.key() == Key::Escape { renaming.set(None); }
                                        }
                                    }
                                } else {
                                    span { class: "grow snd-name", title: "double click to rename",
                                        ondoubleclick: { let n = name.clone(); move |_| { rename_to.set(n.clone()); renaming.set(Some(s3.clone())); } },
                                        b { class: "small", if name.is_empty() { "(no name)" } else { "{name}" } }
                                        span { class: "dim small", " {secs}{mime} · {kb} KB" }
                                    }
                                }
                                button { class: if deleting().as_deref() == Some(sid.as_str()) { "btn small primary" } else { "btn small" },
                                    onclick: move |_| {
                                        if deleting().as_deref() != Some(s4.as_str()) { deleting.set(Some(s4.clone())); return; }
                                        deleting.set(None);
                                        let (api, id) = (api_d.clone(), s4.clone());
                                        spawn(async move {
                                            match api.call("sounds.delete", json!({ "id": id })).await {
                                                Ok(_) => rev += 1,
                                                Err(e) => ui.fail(e),
                                            }
                                        });
                                    },
                                    if deleting().as_deref() == Some(sid.as_str()) { "sure?" } else { "delete" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
