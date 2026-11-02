//! The forge, as the Bevy app laid it out (forge_ui.rs, after Oblivion's spellmaking
//! altar): your saves on the left, the ingredients chosen in the middle (each with its
//! effect: add, take away, keep shared), and the result on the right: what it holds, a few
//! of its texts with their sums, filters to narrow it (words, ordinal and reversed ranges,
//! texts, notes, lists), a name, and the forge. Forging makes a new save; pouring puts the
//! result into the open one (`forge.preview`, `forge.make`, `forge.pour`).

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::ui::*;

const EFFECTS: [(&str, &str); 3] = [("add", "+"), ("take away", "−"), ("keep shared", "∩")];

#[component]
pub fn Forge(on_close: EventHandler<()>) -> Element {
    let ui = ui();
    let api = api();
    let mut parts: Signal<Vec<(String, String)>> = use_signal(Vec::new);
    let mut words = use_signal(String::new);
    let mut o_from = use_signal(String::new);
    let mut o_to = use_signal(String::new);
    let mut r_from = use_signal(String::new);
    let mut r_to = use_signal(String::new);
    let mut keep = use_signal(|| [true, true, true]);
    let mut name = use_signal(String::new);
    let api_s = api.clone();
    let saves = use_resource(move || {
        let api = api_s.clone();
        let _ = (ui.changed)();
        async move { api.saves().await.unwrap_or_default() }
    });
    let args = move || {
        let n = |s: Signal<String>| s.peek().trim().parse::<i64>().ok();
        let k = *keep.peek();
        json!({
            "ingredients": parts.peek().iter().map(|(s, e)| json!({ "save": s, "effect": e })).collect::<Vec<_>>(),
            "filter": { "words": words.peek().clone(), "ordinal_from": n(o_from), "ordinal_to": n(o_to),
                        "reversed_from": n(r_from), "reversed_to": n(r_to), "memories": k[0], "notes": k[1], "lists": k[2] },
        })
    };
    let api_p = api.clone();
    let preview = use_resource(move || {
        let api = api_p.clone();
        let _ = (parts(), words(), o_from(), o_to(), r_from(), r_to(), keep());
        let a = args();
        async move {
            if a["ingredients"].as_array().is_none_or(|x| x.is_empty()) {
                return None;
            }
            Some(api.call("forge.preview", a).await)
        }
    });
    let api_m = api.clone();
    let make = move |_| {
        let api = api_m.clone();
        let mut a = args();
        let n = name().trim().to_string();
        a["name"] = json!(n);
        spawn(async move {
            match api.call("forge.make", a).await {
                Ok(v) => {
                    ui.say(format!(
                        "forged {}: {} texts, {} notes, {} lists",
                        v["save"].as_str().unwrap_or(""),
                        v["memories"],
                        v["notes"],
                        v["lists"]
                    ));
                    ui.touched();
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let api_o = api.clone();
    let pour = move |_| {
        let api = api_o.clone();
        let a = args();
        spawn(async move {
            match api.call("forge.pour", a).await {
                Ok(v) => {
                    ui.say(format!(
                        "poured in {} texts, {} notes, {} lists",
                        v["memories"], v["notes"], v["lists"]
                    ));
                    ui.touched();
                    ui.refresh(api);
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let all = saves.read().clone().unwrap_or_default();
    let pv: Option<Result<Value, String>> = preview.read().clone().flatten();
    rsx! {
        div { class: "overlay", onclick: move |_| on_close.call(()),
            div { class: "dialog forge", role: "dialog", "aria-label": "the forge", onclick: move |e| e.stop_propagation(),
                div { class: "head",
                    h2 { Icon { name: "forge" } " the forge" }
                    span { class: "dim small", "saves in, a save out: + adds one, − takes it away, ∩ keeps only what they share" }
                    span { class: "grow" }
                    button { class: "plain ibtn", "aria-label": "close", onclick: move |_| on_close.call(()), Icon { name: "close" } }
                }
                div { class: "forge-cols",
                    div { class: "fcol",
                        h3 { "your saves" }
                        for s in all {
                            div { key: "{s.name}", class: "row fsave",
                                span { class: "grow", "{s.name}" if s.open { span { class: "dim small", " (open)" } } }
                                for (e , sym) in EFFECTS {
                                    button { key: "{e}", class: "plain", "data-tip": "{e}",
                                        onclick: { let n = s.name.clone(); move |_| parts.write().push((n.clone(), e.to_string())) },
                                        "{sym}" }
                                }
                            }
                        }
                    }
                    div { class: "fcol",
                        h3 { "ingredients" }
                        if parts().is_empty() { div { class: "dim small", "pick saves on the left with + − ∩" } }
                        for (i , (s , e)) in parts().into_iter().enumerate() {
                            div { key: "{i}", class: "row",
                                b { {EFFECTS.iter().find(|x| x.0 == e).map(|x| x.1).unwrap_or("+")} }
                                span { class: "grow", "{s}" span { class: "dim small", " · {e}" } }
                                button { class: "plain ibtn", "aria-label": "take it out", onclick: move |_| { parts.write().remove(i); }, Icon { name: "close" } }
                            }
                        }
                    }
                    div { class: "fcol",
                        h3 { "the result" }
                        match pv {
                            Some(Ok(v)) => rsx! {
                                div { "{v[\"memories\"]} texts · {v[\"notes\"]} notes · {v[\"lists\"]} list links" }
                                div { class: "fsample",
                                    for (i , t) in v["sample"].as_array().cloned().unwrap_or_default().into_iter().enumerate() {
                                        div { key: "{i}", class: "small", "{t[\"text\"].as_str().unwrap_or(\"\")} " span { class: "dim", "{t[\"ordinal\"]}·{t[\"reversed\"]}" } }
                                    }
                                    for (i , t) in v["titles"].as_array().cloned().unwrap_or_default().into_iter().enumerate() {
                                        div { key: "n{i}", class: "small", "✎ {t.as_str().unwrap_or(\"\")}" }
                                    }
                                }
                            },
                            Some(Err(e)) => rsx! { div { class: "bad small", "{e}" } },
                            None => rsx! { div { class: "dim small", "nothing yet" } },
                        }
                        div { class: "form",
                            label { "words" } input { r#type: "text", placeholder: "word -notword", oninput: move |e| words.set(e.value()) }
                            label { "ordinal" } div { class: "row", input { r#type: "number", style: "width: 80px;", oninput: move |e| o_from.set(e.value()) } "to" input { r#type: "number", style: "width: 80px;", oninput: move |e| o_to.set(e.value()) } }
                            label { "reversed" } div { class: "row", input { r#type: "number", style: "width: 80px;", oninput: move |e| r_from.set(e.value()) } "to" input { r#type: "number", style: "width: 80px;", oninput: move |e| r_to.set(e.value()) } }
                            label { "keep" }
                            div { class: "row",
                                for (i , what) in ["texts", "notes", "lists"].into_iter().enumerate() {
                                    label { key: "{what}", class: "row small", input { r#type: "checkbox", checked: keep()[i], onchange: move |e| keep.write()[i] = e.checked() } "{what}" }
                                }
                            }
                            label { "name" } input { r#type: "text", placeholder: "the new save's name", oninput: move |e| name.set(e.value()) }
                        }
                        div { class: "row",
                            button { class: "primary", disabled: parts().is_empty() || name().trim().is_empty(), onclick: make, Icon { name: "forge" } "forge" }
                            button { disabled: parts().is_empty(), "data-tip": "put the result into the open save", onclick: pour, Icon { name: "merge" } "pour into the open save" }
                        }
                    }
                }
            }
        }
    }
}
