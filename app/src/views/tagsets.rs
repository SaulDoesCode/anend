//! Tag sets (queued requests #13, #10; `tagsets.*` in docs/app-api.md): word types of
//! one's own, the models view's fourth tab. A set's page shows its types and their
//! colours, how many words each has marked, how well it reads words it didn't train on
//! (precision and recall per type), and a try card: a text's words labelled by the set,
//! where clicking a word picks its right type (which marks it, and teaches the set when it
//! is trained). A checker set marks words right or wrong and offers the nearest right
//! words for a wrong one. The built-in English set is shown read-only.

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::ui::*;
use crate::views::classifiers::pct;

const PALETTE: [&str; 8] = [
    "#e07a3f", "#3f7ae0", "#3fae6a", "#b05cc8", "#d6453d", "#c9a227", "#2bb3b1", "#8a6d5a",
];

#[component]
pub fn TagSets() -> Element {
    let ui = ui();
    let api = api();
    let mut chosen: Signal<Option<String>> = use_signal(|| None);
    let mut making = use_signal(|| false);
    let mut rev = use_signal(|| 0u64);
    let list = use_resource(move || {
        let api = api.clone();
        let _ = ((ui.changed)(), rev());
        async move { api.call("tagsets.list", json!({})).await }
    });
    let (items, note): (Vec<Value>, Option<String>) = match &*list.read() {
        Some(Ok(v)) => (v.as_array().cloned().unwrap_or_default(), None),
        Some(Err(e)) => (vec![], Some(e.clone())),
        None => (vec![], None),
    };
    let open = chosen().unwrap_or_else(|| "english".to_string());
    let current = items
        .iter()
        .find(|t| t["builtin"] == true && open == "english" || t["set"]["name"] == open.as_str())
        .cloned();
    rsx! {
        if let Some(n) = note {
            div { class: "card bad", "{n}" }
        }
        div { class: "cls-layout",
            div { class: "card cls-list", role: "listbox", "aria-label": "tag sets",
                button { class: if making() { "cls-item on" } else { "cls-item" }, onclick: move |_| making.set(!making()),
                    b { Icon { name: "new" } " new" } }
                for t in items.iter().cloned() {
                    {
                        let builtin = t["builtin"] == true;
                        let name = if builtin { "english".to_string() } else { t["set"]["name"].as_str().unwrap_or("").to_string() };
                        let on = !making() && open == name;
                        let sub = if builtin {
                            "by rule, on English".to_string()
                        } else if t["set"]["check"] == true {
                            "a checker: right or wrong".to_string()
                        } else {
                            t["set"]["types"].as_array().into_iter().flatten().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(" · ")
                        };
                        rsx! {
                            button { key: "{name}", role: "option", "aria-selected": "{on}", class: if on { "cls-item on" } else { "cls-item" },
                                onclick: { let n = name.clone(); move |_| { chosen.set(Some(n.clone())); making.set(false); } },
                                b { "{name}" }
                                span { class: "small", "{sub}" }
                            }
                        }
                    }
                }
            }
            div { class: "cls-main",
                if making() {
                    SetEditor { set: Value::Null, on_saved: move |n: String| { making.set(false); chosen.set(Some(n)); rev += 1; } }
                } else if let Some(t) = current {
                    if t["builtin"] == true {
                        English { types: crate::views::classifiers::strs(&t["types"]) }
                    } else {
                        SetPage { key: "{open}", set: t["set"].clone(), on_changed: move |_| rev += 1, on_gone: move |_| { chosen.set(None); rev += 1; } }
                    }
                }
            }
        }
    }
}

#[component]
fn English(types: Vec<String>) -> Element {
    rsx! {
        div { class: "card",
            h3 { "english" span { class: "dim small", " built in" } }
            p { "Word types by rule, for texts that read as English (most of their words are English ones). The memory view's word types (T) use it. Texts in other languages get no English types; make a tag set for them, and say which texts it's for." }
            div { class: "row wrap",
                for t in types { span { key: "{t}", class: "tw t-{t}", b { "{t}" } } }
            }
        }
    }
}

/// a new set, or a set's types, threshold and which texts it's for
#[component]
fn SetEditor(set: Value, on_saved: EventHandler<String>) -> Element {
    let ui = ui();
    let api = api();
    let fresh = set.is_null();
    let mut draft = use_signal(|| {
        if set.is_null() {
            json!({ "name": "", "types": [
                { "name": "", "colour": PALETTE[0] }, { "name": "", "colour": PALETTE[1] }
            ], "check": false, "threshold": 0.5 })
        } else {
            set.clone()
        }
    });
    let api_c = api.clone();
    let models = use_resource(move || {
        let api = api_c.clone();
        async move { api.models().await.unwrap_or_default() }
    });
    let models = models.read().clone().unwrap_or_default();
    let d = draft();
    let check = d["check"] == true;
    let types: Vec<Value> = d["types"].as_array().cloned().unwrap_or_default();
    let ft_cls = d["for_texts"]["classifier"].as_str().unwrap_or("").to_string();
    let ft_class = d["for_texts"]["class"].as_str().unwrap_or("").to_string();
    let classes: Vec<String> = models
        .iter()
        .find(|m| m.name == ft_cls)
        .map(|m| m.classes.clone())
        .unwrap_or_default();
    let was = set["name"].as_str().unwrap_or("").to_string();
    rsx! {
        div { class: "card cls-new",
            h3 { Icon { name: "new" } if fresh { "a new tag set" } else { "its types" } }
            div { class: "form",
                if fresh {
                    label { r#for: "ts-name", "name" }
                    input { r#type: "text", id: "ts-name", placeholder: "elvish, sacred words…", value: "{d[\"name\"].as_str().unwrap_or(\"\")}",
                        oninput: move |e| draft.write()["name"] = json!(e.value()) }
                    label { "kind" }
                    div { class: "row",
                        button { class: if !check { "chip on" } else { "chip" }, onclick: move |_| draft.write()["check"] = json!(false), "word types" }
                        button { class: if check { "chip on" } else { "chip" }, "data-tip": "for a language of one's own: words marked right or wrong, wrong ones flagged with suggestions",
                            onclick: move |_| draft.write()["check"] = json!(true), "a checker" }
                    }
                }
                if !check {
                    label { "types" }
                    div { class: "col",
                        for (i , t) in types.iter().cloned().enumerate() {
                            div { key: "{i}", class: "row",
                                input { r#type: "color", "aria-label": "its colour", value: "{t[\"colour\"].as_str().unwrap_or(\"#888888\")}",
                                    oninput: move |e| draft.write()["types"][i]["colour"] = json!(e.value()) }
                                input { r#type: "text", "aria-label": "the type's name", placeholder: "a type", value: "{t[\"name\"].as_str().unwrap_or(\"\")}",
                                    oninput: move |e| draft.write()["types"][i]["name"] = json!(e.value()) }
                                input { r#type: "text", class: "grow", "aria-label": "what it is", placeholder: "what it is (optional)", value: "{t[\"about\"].as_str().unwrap_or(\"\")}",
                                    oninput: move |e| draft.write()["types"][i]["about"] = json!(e.value()) }
                                if types.len() > 2 {
                                    button { class: "plain ibtn", "aria-label": "remove it", onclick: move |_| { if let Some(a) = draft.write()["types"].as_array_mut() { a.remove(i); } }, Icon { name: "close" } }
                                }
                            }
                        }
                        button { class: "plain", onclick: move |_| {
                                let n = draft.peek()["types"].as_array().map(|a| a.len()).unwrap_or(0);
                                if let Some(a) = draft.write()["types"].as_array_mut() { a.push(json!({ "name": "", "colour": PALETTE[n % PALETTE.len()] })); }
                            }, Icon { name: "new" } "a type" }
                    }
                }
                label { "for texts" }
                div { class: "row wrap",
                    select { "aria-label": "the classifier that says which texts", value: "{ft_cls}",
                        onchange: move |e| { let v = e.value(); if v.is_empty() { if let Some(o) = draft.write().as_object_mut() { o.remove("for_texts"); } } else { draft.write()["for_texts"] = json!({ "classifier": v, "class": "" }); } },
                        option { value: "", selected: ft_cls.is_empty(), "any text I pick it for" }
                        for m in models.iter() { option { key: "{m.name}", value: "{m.name}", selected: m.name == ft_cls, "{m.name} says…" } }
                    }
                    if !ft_cls.is_empty() {
                        select { "aria-label": "the class", value: "{ft_class}",
                            onchange: move |e| draft.write()["for_texts"]["class"] = json!(e.value()),
                            option { value: "", "which class…" }
                            for c in classes { option { key: "{c}", value: "{c}", selected: c == ft_class, "{c}" } }
                        }
                    }
                }
                label { r#for: "ts-th", "show a type from" }
                div { class: "row",
                    input { r#type: "range", id: "ts-th", min: "0", max: "1", step: "0.05", value: "{d[\"threshold\"].as_f64().unwrap_or(0.5)}",
                        oninput: move |e| draft.write()["threshold"] = json!(e.value().parse::<f64>().unwrap_or(0.5)) }
                    span { class: "dim small", {format!("{} sure", pct(d["threshold"].as_f64().unwrap_or(0.5)))} }
                }
            }
            div { class: "row",
                button { class: "primary",
                    onclick: move |_| {
                        let api = api.clone();
                        let mut set = draft();
                        if let Some(a) = set["types"].as_array_mut() {
                            a.retain(|t| !t["name"].as_str().unwrap_or("").trim().is_empty());
                        }
                        let mut args = json!({ "set": set });
                        if fresh { args["new"] = json!(true); } else { args["was"] = json!(was.clone()); }
                        spawn(async move {
                            match api.call("tagsets.save", args).await {
                                Ok(v) => {
                                    ui.say("saved: mark some words of each type in the try card, then train it");
                                    on_saved.call(v["set"]["name"].as_str().unwrap_or("").to_string());
                                }
                                Err(e) => ui.fail(e),
                            }
                        });
                    },
                    Icon { name: "save" } if fresh { "make it" } else { "save" } }
            }
        }
    }
}

#[component]
fn SetPage(set: Value, on_changed: EventHandler<()>, on_gone: EventHandler<()>) -> Element {
    let ui = ui();
    let api = api();
    let name = set["name"].as_str().unwrap_or("").to_string();
    let mut rev = use_signal(|| 0u64);
    let mut editing = use_signal(|| false);
    let mut sure = use_signal(|| false);
    let (api_s, n_s) = (api.clone(), name.clone());
    let score = use_resource(move || {
        let (api, n) = (api_s.clone(), n_s.clone());
        let _ = ((ui.changed)(), rev());
        async move { api.call("tagsets.score", json!({ "name": n })).await }
    });
    let sc = score.read().clone().and_then(|r| r.ok()).unwrap_or(Value::Null);
    let training = sc["training"] == true;
    // while it trains, look again now and then
    use_future(move || async move {
        loop {
            crate::ui::sleep_ms(1500).await;
            if score
                .peek()
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .is_some_and(|v| v["training"] == true)
            {
                rev += 1;
            }
        }
    });
    let colour = |ty: &str| {
        set["types"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| t["name"] == ty)
            .and_then(|t| t["colour"].as_str())
            .unwrap_or("#888")
            .to_string()
    };
    let check = set["check"] == true;
    let (api_t, n_t) = (api.clone(), name.clone());
    let train = move |_| {
        let (api, n) = (api_t.clone(), n_t.clone());
        spawn(async move {
            match api.call("tagsets.train", json!({ "name": n })).await {
                Ok(_) => {
                    ui.say("training: it reads its marked words, and how well it reads unseen ones shows here");
                    rev += 1;
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let (api_d, n_d) = (api.clone(), name.clone());
    rsx! {
        div { class: "cls-page",
            div { class: "card cls-head",
                div { class: "row",
                    h2 { "{name}" }
                    for t in set["types"].as_array().cloned().unwrap_or_default() {
                        span { key: "{t[\"name\"]}", class: "chip ts-type", style: "--c: {t[\"colour\"].as_str().unwrap_or(\"#888\")};", title: "{t[\"about\"].as_str().unwrap_or(\"\")}", "{t[\"name\"].as_str().unwrap_or(\"\")}" }
                    }
                    span { class: "grow" }
                    button { class: if editing() { "on" } else { "" }, onclick: move |_| editing.set(!editing()), Icon { name: "edit" } "types" }
                    button { class: if sure() { "danger" } else { "" },
                        onclick: move |_| {
                            if !sure() { sure.set(true); return; }
                            let (api, n) = (api_d.clone(), n_d.clone());
                            spawn(async move {
                                match api.call("tagsets.delete", json!({ "name": n })).await {
                                    Ok(_) => { ui.say("deleted the set and its classifier (the marked words' lists stay)"); on_gone.call(()); }
                                    Err(e) => ui.fail(e),
                                }
                            });
                        },
                        Icon { name: "delete" } if sure() { "sure? delete" } else { "delete" } }
                }
                if let Some(f) = set["for_texts"].as_object() {
                    span { class: "dim small", {format!("for texts {} puts in {}", f.get("classifier").or(f.get("taxonomy")).and_then(|x| x.as_str()).unwrap_or(""), f.get("class").and_then(|x| x.as_str()).unwrap_or(""))} }
                } else {
                    span { class: "dim small", "for any text it's picked for" }
                }
            }
            if editing() {
                SetEditor { set: set.clone(), on_saved: move |_| { editing.set(false); on_changed.call(()); rev += 1; } }
            }
            div { class: "card",
                h3 { Icon { name: "classifier" } "how it reads"
                    span { class: "grow" }
                    button { class: "primary", disabled: training, onclick: train, Icon { name: "play" } if training { "training…" } else if sc["trained"] == true { "train again" } else { "train it" } }
                }
                table { class: "confusion",
                    thead { tr { th { "type" } th { "words marked" } th { "precision" } th { "recall" } } }
                    tbody {
                        for m in sc["marked"].as_array().cloned().unwrap_or_default() {
                            {
                                let ty = m["type"].as_str().unwrap_or("").to_string();
                                let st = sc["types"].as_array().into_iter().flatten().find(|x| x["type"] == ty.as_str()).cloned().unwrap_or(Value::Null);
                                let some: Vec<String> = crate::views::classifiers::strs(&m["some"]);
                                rsx! {
                                    tr { key: "{ty}",
                                        th { span { class: "swatch-dot", style: "background: {colour(&ty)};" } " {ty}" }
                                        td { title: "{some.join(\", \")}", "{m[\"words\"]}" }
                                        td { {st["precision"].as_f64().map(pct).unwrap_or_else(|| "—".into())} }
                                        td { {st["recall"].as_f64().map(pct).unwrap_or_else(|| "—".into())} }
                                    }
                                }
                            }
                        }
                    }
                }
                p { class: "dim small",
                    if let Some(a) = sc["accuracy"].as_f64() {
                        if sc["measured_on"] == "held out" {
                            {format!("{} right on marked words it didn't train on", pct(a))}
                        } else {
                            {format!("{} right on the words it trained on: with forty or more marked, a fifth are held out to measure it on words it hasn't seen", pct(a))}
                        }
                    }
                    else { "Mark two words or more of each type in the try card, then train it." }
                }
            }
            TryTags { name: name.clone(), set: set.clone(), check, on_marked: move |_| rev += 1 }
        }
    }
}

/// a text's words as the set labels them; clicking a word picks its right type
#[component]
fn TryTags(name: String, set: Value, check: bool, on_marked: EventHandler<()>) -> Element {
    let ui = ui();
    let api = api();
    let mut text = use_signal(String::new);
    let mut got: Signal<Option<Value>> = use_signal(|| None);
    let mut picking: Signal<Option<usize>> = use_signal(|| None);
    let tag = {
        let (api, name) = (api.clone(), name.clone());
        move || {
            let (api, name) = (api.clone(), name.clone());
            let t = text.peek().clone();
            if t.trim().is_empty() {
                return;
            }
            spawn(async move {
                match api.call("tagsets.tag", json!({ "name": name, "text": t })).await {
                    Ok(v) => {
                        picking.set(None);
                        got.set(Some(v));
                    }
                    Err(e) => {
                        got.set(None);
                        ui.fail(e)
                    }
                }
            });
        }
    };
    let types: Vec<(String, String)> = set["types"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            (
                t["name"].as_str().unwrap_or("").to_string(),
                t["colour"].as_str().unwrap_or("#888").to_string(),
            )
        })
        .collect();
    let words: Vec<Value> = got().and_then(|v| v["words"].as_array().cloned()).unwrap_or_default();
    rsx! {
        div { class: "card",
            h3 { Icon { name: "text" } "try a text" span { class: "dim small", " click a word to say which type it is" } }
            div { class: "row",
                input { r#type: "text", class: "grow", "aria-label": "a text to tag", placeholder: "a text in the set's language", value: "{text}",
                    oninput: move |e| text.set(e.value()),
                    onkeydown: { let tag = tag.clone(); move |e: KeyboardEvent| if e.key() == Key::Enter { tag() } } }
                button { onclick: { let tag = tag.clone(); move |_| tag() }, Icon { name: "play" } "tag it" }
            }
            div { class: "ts-words",
                for (i , w) in words.iter().cloned().enumerate() {
                    {
                        let word = w["word"].as_str().unwrap_or("").to_string();
                        let ty = w["type"].as_str().map(String::from);
                        let c = w["colour"].as_str().unwrap_or("transparent").to_string();
                        let suggest = crate::views::classifiers::strs(&w["suggest"]);
                        rsx! {
                            span { key: "{i}", class: if check && ty.as_deref() == Some("wrong") { "ts-word flagged" } else { "ts-word" },
                                button { class: "plain", style: "--c: {c};", "aria-label": "{word}: say which type",
                                    title: {format!("{} · {}", ty.clone().unwrap_or_else(|| "no type".into()), pct(w["prob"].as_f64().unwrap_or(0.0)))},
                                    onclick: move |_| picking.set(if picking() == Some(i) { None } else { Some(i) }),
                                    span { class: "w", "{word}" }
                                    span { class: "l", {ty.clone().unwrap_or_else(|| "?".into())} }
                                }
                                if !suggest.is_empty() {
                                    span { class: "ts-suggest",
                                        for s in suggest {
                                            button { key: "{s}", class: "chip", "data-tip": "use it",
                                                onclick: { let (s, word, tag) = (s.clone(), word.clone(), tag.clone()); move |_| { let t = text.peek().replacen(&word, &s, 1); text.set(t); tag(); } },
                                                "{s}" }
                                        }
                                    }
                                }
                                if picking() == Some(i) {
                                    span { class: "ts-pick",
                                        for (tn , tc) in types.iter().cloned() {
                                            button { key: "{tn}", class: "chip ts-type", style: "--c: {tc};",
                                                onclick: {
                                                    let (api, name, word, tag, tn) = (api.clone(), name.clone(), word.clone(), tag.clone(), tn.clone());
                                                    move |_| {
                                                        let (api, name, word, tag, tn) = (api.clone(), name.clone(), word.clone(), tag.clone(), tn.clone());
                                                        spawn(async move {
                                                            match api.call("tagsets.mark", json!({ "name": name, "word": word, "type": tn })).await {
                                                                Ok(v) => {
                                                                    ui.say(if v["taught"] == true { format!("“{word}” is {tn}: marked, and the set was told") } else { format!("“{word}” is {tn}: marked (train it to learn from the marks)") });
                                                                    on_marked.call(());
                                                                    tag();
                                                                }
                                                                Err(e) => ui.fail(e),
                                                            }
                                                        });
                                                    }
                                                },
                                                "{tn}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if got().is_some() && words.is_empty() {
                p { class: "dim small", "no words" }
            }
        }
    }
}
