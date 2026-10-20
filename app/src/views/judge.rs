//! What classifiers say of one entry (the Bevy app's index panel, index_panel.rs): pick
//! which classifiers the entry is indexed on (kept in the save, `classify.set_index`), and
//! see how it measures on each: every class's probability as a bar in that classifier's
//! colours, the class it's put in, and a yes or a no that teaches it. Used by the notes
//! editor (the classifier button, Ctrl+I) and the reader's opened item.

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::ui::*;

/// `key` is an entry's key (`0,12` or `note:…`; a bare note key is taken as a note's)
#[component]
pub fn Judge(entry: String, #[props(default)] title: String) -> Element {
    let ui = ui();
    let api = api();
    let key = if entry.contains(',') || entry.starts_with("note:") {
        entry
    } else {
        format!("note:{entry}")
    };
    let mut again = use_signal(|| 0u64);
    let k0 = key.clone();
    let api0 = api.clone();
    let info = use_resource(move || {
        let (api, k) = (api0.clone(), k0.clone());
        let _ = (again(), (ui.changed)());
        async move { api.call("classify.indexed", json!({ "key": k })).await }
    });
    let v = match &*info.read() {
        Some(Ok(v)) => v.clone(),
        Some(Err(e)) => return rsx! { div { class: "judge dim small", "{e}" } },
        None => return rsx! { div { class: "judge dim small", "reading what the classifiers say…" } },
    };
    let all: Vec<(String, bool, bool)> = v["classifiers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            (
                c["name"].as_str().unwrap_or("").to_string(),
                c["trained"] == true,
                c["chosen"] == true,
            )
        })
        .collect();
    let chosen: Vec<String> = all.iter().filter(|c| c.2).map(|c| c.0.clone()).collect();
    let verdicts: Vec<Value> = v["verdicts"].as_array().cloned().unwrap_or_default();
    rsx! {
        div { class: "judge",
            div { class: "row judge-pick",
                Icon { name: "classifier" }
                if title.is_empty() { span { class: "dim small", "classifiers to measure it on:" } } else { span { class: "dim small", "{title}" } }
                if all.is_empty() {
                    span { class: "dim small", "none yet: make one in the models view (Ctrl+8)" }
                }
                for (name , trained , on) in all.clone() {
                    button {
                        key: "{name}",
                        class: if on { "chip on" } else { "chip" },
                        "aria-pressed": "{on}",
                        "data-tip": if trained { "measure it on {name}" } else { "{name} isn't trained yet" },
                        onclick: {
                            let (api, key, chosen, name) = (api.clone(), key.clone(), chosen.clone(), name.clone());
                            move |_| {
                                let mut names = chosen.clone();
                                if on { names.retain(|n| *n != name); } else { names.push(name.clone()); }
                                let (api, key) = (api.clone(), key.clone());
                                spawn(async move {
                                    match api.call("classify.set_index", json!({ "key": key, "names": names })).await {
                                        Ok(_) => again += 1,
                                        Err(e) => ui.fail(e),
                                    }
                                });
                            }
                        },
                        "{name}"
                    }
                }
            }
            for v in verdicts {
                Verdict { key: "{v[\"name\"]}", entry: key.clone(), v, on_change: move |_| again += 1 }
            }
        }
    }
}

#[component]
fn Verdict(entry: String, v: Value, on_change: EventHandler<()>) -> Element {
    let ui = ui();
    let api = api();
    let name = v["name"].as_str().unwrap_or("").to_string();
    let classes = crate::views::classifiers::strs(&v["classes"]);
    if v["trained"] != true {
        return rsx! {
            div { class: "verdict dim small", b { "{name}" } " isn't trained yet: train it in the models view" }
        };
    }
    let probs = crate::views::classifiers::nums(&v["probs"]);
    let palette = crate::views::classifiers::strs(&v["palette"]);
    let top = v["top"].as_u64().unwrap_or(0) as usize;
    let members = crate::views::classifiers::strs(&v["members"]);
    let confidence = v["confidence"].as_f64().unwrap_or(0.0);
    let valid = v["valid"] != false;
    let top_class = classes.get(top).cloned().unwrap_or_default();
    let teach = move |yes: bool| {
        let (api, name, entry, class) = (api.clone(), name.clone(), entry.clone(), top_class.clone());
        move |_| {
            let (api, name, entry, class) = (api.clone(), name.clone(), entry.clone(), class.clone());
            spawn(async move {
                match api.model_feedback(&name, Some(&entry), "", &class, yes).await {
                    Ok(_) => {
                        ui.did(Did::Feedback { model: name.clone() });
                        ui.say(format!(
                            "{name} learns that {} “{class}”; Ctrl+Z takes it back",
                            if yes { "it is" } else { "it isn't" }
                        ));
                        on_change.call(());
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let (yes, no) = (teach(true), teach(false));
    rsx! {
        div { class: "verdict",
            div { class: "row",
                b { "{v[\"name\"].as_str().unwrap_or(\"\")}" }
                span { class: "chip cls-class", style: "background: {palette.get(top).cloned().unwrap_or_default()};", "{classes.get(top).cloned().unwrap_or_default()}" }
                span { class: "dim small", {format!("{:.0}% sure", confidence * 100.0)} }
                if members.len() > 1 { span { class: "dim small", "also {members.iter().filter(|m| **m != classes[top.min(classes.len().saturating_sub(1))]).cloned().collect::<Vec<_>>().join(\", \")}" } }
                if !valid { span { class: "bad small", "its validators disagree" } }
                span { class: "grow" }
                button { class: "plain", "data-tip": "yes, it is that", "aria-label": "yes", onclick: yes, Icon { name: "check" } }
                button { class: "plain", "data-tip": "no, it isn't that", "aria-label": "no", onclick: no, Icon { name: "close" } }
            }
            div { class: "probs",
                for (i , c) in classes.iter().enumerate() {
                    div { key: "{c}", class: if i == top { "prob best" } else { "prob" },
                        span { class: "c", "{c}" }
                        span { class: "probbar wide", span { style: "width: {(probs.get(i).copied().unwrap_or(0.0) * 100.0).clamp(0.0, 100.0):.1}%; background: {palette.get(i).cloned().unwrap_or_default()};" } }
                        span { class: "p", {format!("{:.1}%", probs.get(i).copied().unwrap_or(0.0) * 100.0)} }
                    }
                }
            }
        }
    }
}
