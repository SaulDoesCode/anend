//! Classifiers: models trained on the save's lists, one class per list (docs/app-api.md,
//! `classify.*`). The same view on the desktop and the web, through the backend.
//!
//! The left column lists them; a classifier's page has its training (epochs, progress,
//! cancel, the metrics), a try card (the command bar's text, each class's probability,
//! yes/no, texts like it), its steering (bias, temperature, threshold), a search by class
//! (yes/no on each, the class as a list), and its recent verdicts. Every yes/no can be
//! taken back with Ctrl+Z. "new" makes one from lists; "import" takes a classifier file
//! (`.anendcls`), and each page exports its own.

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::backend::{ListInfo, ModelSummary, Scored};
use crate::ui::*;
use crate::views::clsmore::Tab;
use crate::views::use_keys;

/// what the open classifier's page is showing
#[derive(Clone, Copy, PartialEq)]
enum Side {
    None,
    New,
    Import,
    Starters,
}

pub(crate) fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect()
}

pub(crate) fn nums(v: &Value) -> Vec<f64> {
    v.as_array().into_iter().flatten().filter_map(Value::as_f64).collect()
}

pub(crate) fn pct(p: f64) -> String {
    format!("{:.0}%", p * 100.0)
}

#[component]
pub fn Models() -> Element {
    let ui = ui();
    let api = api();
    let mut chosen: Signal<Option<String>> = use_signal(|| None);
    let mut side = use_signal(|| Side::None);
    let mut rev = use_signal(|| 0u64);
    // the classifiers or the spec models
    let mut specs_tab = use_signal(|| ui.prefs.peek().folded("models: specs"));
    let specs = use_hook(crate::views::specs::SpecState::new);
    let api_r = api.clone();
    let models = use_resource(move || {
        let api = api_r.clone();
        let _ = ((ui.changed)(), rev());
        async move { api.models().await }
    });
    use_keys(View::Models, move |k: KeyPress| {
        if *specs_tab.peek() {
            if k.bar && k.key == "Enter" {
                let mut e = specs.enter;
                e += 1;
                return true;
            }
            if !k.bar && k.key == "Escape" {
                specs.escape();
                return true;
            }
            return false;
        }
        if k.bar && k.key == "Enter" {
            // the try card runs as you type
            if chosen.peek().is_none()
                && models
                    .peek()
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .is_none_or(|l| l.is_empty())
            {
                ui.fail("make a classifier first (new)");
            }
            return true;
        }
        if !k.bar && k.key == "Escape" {
            if *side.peek() != Side::None {
                side.set(Side::None);
            } else {
                chosen.set(None);
            }
            return true;
        }
        false
    });
    let (list, note): (Vec<ModelSummary>, Option<String>) = match &*models.read() {
        Some(Ok(v)) => (v.clone(), None),
        Some(Err(e)) => (vec![], Some(e.clone())),
        None => (vec![], None),
    };
    // the first one is open until another is picked
    let open = chosen().or_else(|| {
        (side() == Side::None)
            .then(|| list.first().map(|m| m.name.clone()))
            .flatten()
    });
    let on_made = move |name: String| {
        side.set(Side::None);
        chosen.set(Some(name));
        rev += 1;
    };
    rsx! {
        div { class: "page wide",
            div { class: "row",
                div { class: "mtabs", role: "tablist", "aria-label": "kinds of model",
                    for (on , label) in [(false, "classifiers"), (true, "spec models")] {
                        button { key: "{label}", role: "tab", "aria-selected": "{specs_tab() == on}",
                            class: if specs_tab() == on { "mtab on" } else { "mtab" },
                            onclick: move |_| {
                                specs_tab.set(on);
                                crate::shell::set_pref(ui, |p| {
                                    if on { p.collapsed.insert("models: specs".into()); } else { p.collapsed.remove("models: specs"); }
                                });
                            },
                            "{label}" }
                    }
                }
                if specs_tab() {
                    span { class: "dim small", "described as data: what they read, their heads, what they learn from" }
                } else {
                    span { class: "dim small", "learn classes from example lists, then read any text" }
                }
                span { class: "grow" }
                if !specs_tab() {
                button { class: if side() == Side::New { "on" } else { "" }, onclick: move |_| side.set(if side() == Side::New { Side::None } else { Side::New }),
                    Icon { name: "new" } "new" }
                button { class: if side() == Side::Starters { "on" } else { "" }, "data-tip": "ready-made classifiers whose answers a rule can check",
                    onclick: move |_| side.set(if side() == Side::Starters { Side::None } else { Side::Starters }),
                    Icon { name: "star" } "starters" }
                button { class: if side() == Side::Import { "on" } else { "" }, "data-tip": "a classifier file (.anendcls)",
                    onclick: move |_| side.set(if side() == Side::Import { Side::None } else { Side::Import }),
                    Icon { name: "upload" } "import" }
                }
            }
            if specs_tab() {
                crate::views::specs::SpecModels { state: specs }
            } else {
            if let Some(n) = note {
                div { class: "card bad", "{n}" }
            }
            div { class: "cls-layout",
                div { class: "card cls-list", role: "listbox", "aria-label": "classifiers",
                    if list.is_empty() {
                        span { class: "dim small", "none yet" }
                    }
                    for m in list.iter().cloned() {
                        button {
                            key: "{m.name}",
                            role: "option",
                            "aria-selected": "{open.as_deref() == Some(m.name.as_str())}",
                            class: if open.as_deref() == Some(m.name.as_str()) && side() == Side::None { "cls-item on" } else { "cls-item" },
                            onclick: { let n = m.name.clone(); move |_| { chosen.set(Some(n.clone())); side.set(Side::None); } },
                            b { "{m.name}" }
                            span { class: "small",
                                {m.classes.join(" · ")}
                            }
                            span { class: "small state",
                                if m.training { "training…" }
                                else if m.trained { {format!("trained{}", m.accuracy.map(|a| format!(" · {}", pct(a))).unwrap_or_default())} }
                                else { "not trained yet" }
                            }
                        }
                    }
                }
                div { class: "cls-main",
                    match side() {
                        Side::New => rsx! { NewClassifier { on_made } },
                        Side::Import => rsx! { ImportClassifier { on_made } },
                        Side::Starters => rsx! { Starters { on_made } },
                        Side::None => match open {
                            Some(name) => rsx! { ClassifierPage { key: "{name}", name: name.clone(), on_gone: move |_| { chosen.set(None); rev += 1; } } },
                            None => rsx! {
                                div { class: "card empty",
                                    p { "A classifier learns from examples, given as lists (each list a class, its members the examples), and then reads any text, in the save or newly typed: it orders classifier search, colours the memory and the space, and answers in the reader and the notes. Classifiers are one use of the project's transformer engine, which also runs spec models (taggers, generators, critics)." }
                                    p { "Make two or more lists (in the memory view: Shift+click a text to make it a list's head, then Alt+click texts and put them in it), then press " b { "new" } "." }
                                    p { "Or begin with a starter: ready-made texts whose classes a rule knows for certain, so how well a classifier learned them is measured on texts it never saw. "
                                        button { onclick: move |_| side.set(Side::Starters), Icon { name: "star" } "starters" } }
                                }
                            },
                        },
                    }
                }
            }
            }
        }
    }
}

// ------------------------------------------------------------------ making one

#[component]
fn NewClassifier(on_made: EventHandler<String>) -> Element {
    let ui = ui();
    let api = api();
    let mut name = use_signal(String::new);
    let mut classes: Signal<Vec<String>> = use_signal(Vec::new);
    let mut not = use_signal(String::new);
    let mut preset = use_signal(|| "compact".to_string());
    let mut multi = use_signal(|| false);
    let mut gematria = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut q = use_signal(String::new);
    let api_l = api.clone();
    let lists = use_resource(move || {
        let api = api_l.clone();
        async move { api.lists().await }
    });
    let all: Vec<ListInfo> = match &*lists.read() {
        Some(Ok(l)) => l.lists.clone(),
        Some(Err(e)) => return rsx! { div { class: "card bad", "{e}" } },
        None => return rsx! { div { class: "card dim", "…" } },
    };
    let usable: Vec<ListInfo> = all.iter().filter(|l| l.count > 0).cloned().collect();
    let ql = q().to_lowercase();
    let shown: Vec<ListInfo> = usable
        .iter()
        .filter(|l| ql.is_empty() || l.name.to_lowercase().contains(&ql) || classes.read().contains(&l.list))
        .cloned()
        .collect();
    let picked = classes();
    let ready = !name().trim().is_empty() && (picked.len() >= 2 || (picked.len() == 1 && !not().is_empty()));
    let create = move |_| {
        let api = api.clone();
        let args = json!({
            "name": name.peek().trim(),
            "classes": *classes.peek(),
            "not": *not.peek(),
            "preset": *preset.peek(),
            "head": if *multi.peek() { "multi" } else { "single" },
            "gematria": *gematria.peek(),
        });
        busy.set(true);
        dioxus::dioxus_core::spawn_forever(async move {
            match api.call("classify.create", args).await {
                Ok(v) => {
                    let n = v["name"].as_str().unwrap_or("").to_string();
                    ui.say(format!("made {n}; train it next"));
                    on_made.call(n);
                }
                Err(e) => ui.fail(e),
            }
            busy.set(false);
        });
    };
    rsx! {
        div { class: "card cls-new",
            h3 { Icon { name: "new" } "a new classifier" }
            if usable.len() < 2 {
                p { class: "dim", "It needs at least two lists with members (or one list and a list of texts that are not in it). This save has {usable.len()}." }
            }
            div { class: "form",
                label { r#for: "cls-name", "name" }
                input { id: "cls-name", r#type: "text", placeholder: "kind, author, mood…", oninput: move |e| name.set(e.value()) }
                label { "classes" }
                div { class: "col",
                    if usable.len() > 12 {
                        input { r#type: "text", placeholder: "find a list", oninput: move |e| q.set(e.value()) }
                    }
                    div { class: "taglist",
                        for l in shown {
                            button {
                                key: "{l.list}",
                                class: if picked.contains(&l.list) { "chip on" } else { "chip" },
                                "aria-pressed": "{picked.contains(&l.list)}",
                                title: l.members.iter().take(5).map(|m| m.text.clone()).collect::<Vec<_>>().join(" · "),
                                onclick: { let k = l.list.clone(); move |_| {
                                    let mut c = classes.write();
                                    if let Some(i) = c.iter().position(|x| *x == k) { c.remove(i); } else { c.push(k.clone()); }
                                } },
                                "{l.name}" span { class: "dim", " {l.count}" }
                            }
                        }
                    }
                    span { class: "dim small", "each list picked is a class; its members are the examples" }
                }
                label { r#for: "cls-not", "none of them" }
                select { id: "cls-not", onchange: move |e| not.set(e.value()),
                    option { value: "", "no such list" }
                    for l in usable.iter().filter(|l| !picked.contains(&l.list)) {
                        option { key: "{l.list}", value: "{l.list}", "{l.name} ({l.count})" }
                    }
                }
                label { r#for: "cls-preset", "size" }
                select { id: "cls-preset", value: "{preset}", onchange: move |e| preset.set(e.value()),
                    option { value: "tiny", "tiny: trains in seconds" }
                    option { value: "compact", "compact: the default" }
                    option { value: "small", "small: more to learn, slower" }
                    option { value: "base", "base: the largest" }
                }
                label { "options" }
                div { class: "col",
                    label { class: "row", input { r#type: "checkbox", checked: multi(), onchange: move |e| multi.set(e.checked()) } "a text can be in several classes at once" }
                    label { class: "row", input { r#type: "checkbox", checked: gematria(), onchange: move |e| gematria.set(e.checked()) } "read the letters' values and word sums too (gematria inputs)" }
                }
            }
            div { class: "row",
                span { class: "grow" }
                button { class: "primary", disabled: !ready || busy(), onclick: create, Icon { name: "check" } "make it" }
            }
        }
    }
}

#[component]
fn ImportClassifier(on_made: EventHandler<String>) -> Element {
    let ui = ui();
    let api = api();
    let mut path = use_signal(String::new);
    let mut as_name = use_signal(String::new);
    let on_disk = api.on_disk();
    let take = {
        let api = api.clone();
        move |bytes: Vec<u8>| {
            let api = api.clone();
            let n = as_name.peek().trim().to_string();
            dioxus::dioxus_core::spawn_forever(async move {
                let data = anend_core::classifiers::base64(&bytes);
                match api.call("classify.import", json!({ "data": data, "as": n })).await {
                    Ok(v) => {
                        let name = v["name"].as_str().unwrap_or("").to_string();
                        let lists = v["lists"].as_array().map_or(0, |l| l.len());
                        ui.say(format!("took in {name} ({lists} lists for its classes)"));
                        ui.touched();
                        on_made.call(name);
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let take2 = take.clone();
    rsx! {
        div { class: "card",
            h3 { Icon { name: "upload" } "a classifier file" }
            p { class: "dim small", "A .anendcls file exported from anend (here, another save, or the hub). Each class goes to the list of its name, made when there's none." }
            div { class: "form",
                label { r#for: "imp-as", "as" }
                input { id: "imp-as", r#type: "text", placeholder: "the name it came with", oninput: move |e| as_name.set(e.value()) }
                label { "the file" }
                input { r#type: "file", accept: ".anendcls",
                    onchange: move |e: FormEvent| {
                        let take = take.clone();
                        async move {
                            let Some(f) = e.files().into_iter().next() else { return };
                            match f.read_bytes().await {
                                Ok(b) => take(b.to_vec()),
                                Err(e) => ui.fail(format!("couldn't read it: {e}")),
                            }
                        }
                    },
                }
                if on_disk {
                    label { r#for: "imp-path", "or its path" }
                    div { class: "row",
                        input { id: "imp-path", r#type: "text", style: "flex: 1;", placeholder: "/home/you/Downloads/kind.anendcls", oninput: move |e| path.set(e.value()) }
                        button { onclick: move |_| {
                                #[cfg(not(target_arch = "wasm32"))]
                                match std::fs::read(path().trim()) {
                                    Ok(b) => take2(b),
                                    Err(e) => ui.fail(format!("couldn't read {}: {e}", path().trim())),
                                }
                                #[cfg(target_arch = "wasm32")]
                                { let _ = &take2; }
                            }, "take it in" }
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ one classifier

#[component]
fn ClassifierPage(name: String, on_gone: EventHandler<()>) -> Element {
    let ui = ui();
    let api = api();
    let mut rev = use_signal(|| 0u64);
    let mut armed = use_signal(|| false);
    let mut job: Signal<Option<Value>> = use_signal(|| None);
    let mut watching = use_signal(|| true);
    let tab = use_signal(|| Tab::Learn);
    let api_all = api.clone();
    let all = use_resource(move || {
        let api = api_all.clone();
        let _ = (ui.changed)();
        async move { api.call("classify.list", json!({})).await.ok() }
    });
    let all_names: Vec<String> = all
        .read()
        .clone()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c["name"].as_str().map(str::to_string))
        .collect();
    let n0 = name.clone();
    let api_d = api.clone();
    let details = use_resource(move || {
        let (api, n) = (api_d.clone(), n0.clone());
        let _ = ((ui.changed)(), rev());
        async move { api.call("classify.get", json!({ "name": n })).await }
    });
    // the training's progress, while there is one
    {
        let (api, n) = (api.clone(), name.clone());
        use_future(move || {
            let (api, n) = (api.clone(), n.clone());
            async move {
                let mut was_running = false;
                loop {
                    if *watching.peek() {
                        match api.call("classify.job", json!({ "name": n })).await {
                            Ok(j) => {
                                let running = j["state"] == "running";
                                if !running {
                                    watching.set(false);
                                    if was_running {
                                        match j["state"].as_str() {
                                            Some("done") => ui.say(format!("{n} is trained")),
                                            Some("failed") => ui.fail(format!(
                                                "{n}: {}",
                                                j["error"].as_str().unwrap_or("training failed")
                                            )),
                                            Some(s) => ui.say(format!("{n}: {s}")),
                                            None => {}
                                        }
                                        ui.touched();
                                    }
                                }
                                was_running = running;
                                job.set(Some(j));
                            }
                            Err(_) => watching.set(false),
                        }
                    }
                    sleep_ms(if was_running { 350 } else { 600 }).await;
                }
            }
        });
    }
    let d = match &*details.read() {
        Some(Ok(v)) => v.clone(),
        Some(Err(e)) => return rsx! { div { class: "card bad", "{e}" } },
        None => return rsx! { div { class: "card dim", "…" } },
    };
    let classes = strs(&d["classes"]);
    let trained = d["trained"].as_bool().unwrap_or(false);
    let running = job().is_some_and(|j| j["state"] == "running");
    let api_x = api.clone();
    let api_del = api.clone();
    let n_x = name.clone();
    let n_del = name.clone();
    let size = &d["size"];
    let inputs: Vec<String> = d["inputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["from"].as_str().map(str::to_string))
        .collect();
    rsx! {
        div { class: "cls-page",
            div { class: "card cls-head",
                div { class: "row",
                    h2 { "{name}" }
                    for (i , c) in classes.iter().enumerate() {
                        span { key: "{c}", class: "chip cls-class", style: "--hue: {hue(i, classes.len())};", "{c}" }
                    }
                    if let Some(n) = d["not"]["name"].as_str() {
                        span { class: "chip", title: "texts that are none of the classes", Icon { name: "not" } "{n}" }
                    }
                    span { class: "grow" }
                    button { class: "plain", "data-tip": "export it as a file (.anendcls)", "aria-label": "export",
                        onclick: move |_| export(ui, api_x.clone(), n_x.clone()),
                        Icon { name: "download" } }
                    button {
                        class: if armed() { "armed" } else { "plain" },
                        "data-tip": "delete it (its lists stay); click twice",
                        "aria-label": "delete the classifier",
                        disabled: running,
                        onclick: move |_| {
                            if !armed() { armed.set(true); return; }
                            let (api, n) = (api_del.clone(), n_del.clone());
                            dioxus::dioxus_core::spawn_forever(async move {
                                match api.delete_model(&n).await {
                                    Ok(()) => { ui.say(format!("deleted {n}")); on_gone.call(()); }
                                    Err(e) => ui.fail(e),
                                }
                            });
                        },
                        Icon { name: "delete" }
                        if armed() { "delete it?" }
                    }
                }
                div { class: "dim small",
                    {format!(
                        "{} · {} head{}{} · version {}",
                        d["preset"].as_str().unwrap_or(""),
                        d["head"].as_str().unwrap_or(""),
                        if d["gematria"].as_bool() == Some(true) { " · gematria inputs" } else { "" },
                        if size.is_object() { format!(" · {} weights, {} wide, {} blocks", size["weights"], size["d_model"], size["layers"]) } else { String::new() },
                        d["version"],
                    )}
                    if !inputs.is_empty() {
                        {format!(" · builds on {}", inputs.join(", "))}
                    }
                    if let Some(s) = d["starter"].as_str() {
                        {format!(" · from the starter {s:?}")}
                    }
                }
            }
            crate::views::clsmore::Tabs { tab }
            match tab() {
                Tab::Learn => rsx! {
                    Train { name: name.clone(), details: d.clone(), job: job(), on_start: move |j: Value| { job.set(Some(j)); watching.set(true); } }
                    if trained && d["starter"].is_string() {
                        HeldOut { name: name.clone(), version: d["version"].as_u64().unwrap_or(0) }
                    }
                    if trained {
                        Steer { name: name.clone(), details: d.clone() }
                    }
                    crate::views::clsmore::Growth { details: d.clone() }
                    Verdicts { name: name.clone(), details: d.clone(), on_change: move |_| rev += 1 }
                },
                _ if !trained => rsx! {
                    div { class: "card dim", if running { "It's training: the learn tab shows how it goes." } else { "Train it first (the learn tab): then try it on texts, search with it, file the save into its buckets and check it with other classifiers." } }
                },
                Tab::Use => rsx! {
                    Try { name: name.clone(), classes: classes.clone() }
                    crate::views::clsmore::Why { name: name.clone() }
                    Search { name: name.clone(), classes: classes.clone(), outputs: d["outputs"].clone() }
                },
                Tab::Buckets => rsx! { crate::views::clsmore::Buckets { name: name.clone() } },
                Tab::Checks => rsx! { crate::views::clsmore::Checks { name: name.clone(), all: all_names.clone() } },
            }
        }
    }
}

// ------------------------------------------------------------------ starters

/// the starters: each one's classes, where its texts come from and on what terms; making
/// one writes its training texts as lists and trains a classifier on them
#[component]
fn Starters(on_made: EventHandler<String>) -> Element {
    let ui = ui();
    let api = api();
    let mut rev = use_signal(|| 0u64);
    let mut busy: Signal<Option<String>> = use_signal(|| None);
    let api_l = api.clone();
    let all = use_resource(move || {
        let api = api_l.clone();
        let _ = rev();
        async move { api.call("classify.starters", json!({})).await }
    });
    let list: Vec<Value> = match &*all.read() {
        Some(Ok(v)) => v.as_array().cloned().unwrap_or_default(),
        Some(Err(e)) => return rsx! { div { class: "card bad", "{e}" } },
        None => return rsx! { div { class: "card dim", "…" } },
    };
    rsx! {
        div { class: "card starters",
            h3 { "starters" }
            p { class: "dim small",
                "Texts whose classes are known for certain: made by a rule, or taken from public-domain books. Making one writes its training texts into this save as lists, one per class (400 texts a class for the rule-made ones), and trains a classifier on them; its page then scores it on texts held out from training, which it never saw."
            }
            for s in list {
                {
                    let id = s["id"].as_str().unwrap_or("").to_string();
                    let classes = strs(&s["classes"]);
                    let source = match s["source"].as_str() {
                        Some("rules") => "made by rules",
                        Some("bundled") => "public domain, comes with the app",
                        _ => "public domain, fetched when asked",
                    };
                    let needs_fetch = s["fetched"] == false;
                    let this_busy = busy().as_deref() == Some(id.as_str());
                    let (api_m, id_m) = (api.clone(), id.clone());
                    let make = move |_| {
                        let (api, id) = (api_m.clone(), id_m.clone());
                        busy.set(Some(id.clone()));
                        spawn(async move {
                            let r = api.call("classify.starter", json!({ "id": id })).await;
                            busy.set(None);
                            match r {
                                Ok(v) => {
                                    let n = v["classifier"]["name"].as_str().unwrap_or("").to_string();
                                    ui.say(format!("{n}: {} texts in its lists; training", v["texts"]));
                                    ui.touched();
                                    on_made.call(n);
                                }
                                Err(e) => ui.fail(e),
                            }
                        });
                    };
                    let (api_f, id_f) = (api.clone(), id.clone());
                    let fetch = move |_| {
                        let (api, id) = (api_f.clone(), id_f.clone());
                        busy.set(Some(id.clone()));
                        spawn(async move {
                            let r = api.call("classify.starter_fetch", json!({ "id": id })).await;
                            busy.set(None);
                            match r {
                                Ok(v) => { ui.say(format!("fetched {} books", v["got"])); rev += 1; }
                                Err(e) => ui.fail(e),
                            }
                        });
                    };
                    rsx! {
                        div { key: "{id}", class: "starter",
                            div { class: "row",
                                b { {s["title"].as_str().unwrap_or("")} }
                                for (i , c) in classes.iter().enumerate() {
                                    span { key: "{c}", class: "chip cls-class", style: "--hue: {hue(i, classes.len())};", "{c}" }
                                }
                                span { class: "grow" }
                                if needs_fetch {
                                    button { disabled: busy().is_some(), onclick: fetch, Icon { name: "download" } if this_busy { "fetching…" } else { "fetch its books" } }
                                } else {
                                    button { disabled: busy().is_some(), onclick: make, Icon { name: "new" } if this_busy { "making…" } else { "make and train it" } }
                                }
                            }
                            div { class: "small", {s["about"].as_str().unwrap_or("")} }
                            div { class: "dim small", "{source}: " {s["licence"].as_str().unwrap_or("")} }
                        }
                    }
                }
            }
        }
    }
}

/// a starter classifier's score on the starter's held-out texts, read again when its
/// weights change
#[component]
fn HeldOut(name: String, version: u64) -> Element {
    let api = api();
    let mut again = use_signal(|| 0u64);
    let n0 = name.clone();
    let score = use_resource(move || {
        let (api, n) = (api.clone(), n0.clone());
        let _ = (again(), version);
        async move { api.call("classify.starter_score", json!({ "name": n })).await }
    });
    let body = match &*score.read() {
        Some(Ok(v)) => {
            let per: Vec<String> = v["classes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| format!("{} {}/{}", c["class"].as_str().unwrap_or(""), c["right"], c["texts"]))
                .collect();
            let dev = match v["device"]["device"].as_str() {
                Some("cpu") | None => "the CPU".to_string(),
                Some(_) => format!("the GPU ({})", v["device"]["gpu"].as_str().unwrap_or("")),
            };
            rsx! {
                div { class: "row",
                    b { class: "big", {format!("{:.1}%", 100.0 * v["accuracy"].as_f64().unwrap_or(0.0))} }
                    span { {format!("right on {} of {} held-out texts it never trained on", v["right"], v["texts"])} }
                }
                div { class: "dim small", {per.join(" · ")} }
                div { class: "dim small", {format!("read on {dev} in {:.2} s", v["seconds"].as_f64().unwrap_or(0.0))} }
            }
        }
        Some(Err(e)) => rsx! { div { class: "bad small", "{e}" } },
        None => rsx! { div { class: "dim small", "scoring on the held-out texts…" } },
    };
    rsx! {
        div { class: "card held-out",
            div { class: "row",
                h3 { "held-out score" }
                span { class: "grow" }
                button { class: "plain", "data-tip": "score it again", onclick: move |_| again += 1, Icon { name: "refresh" } }
            }
            {body}
        }
    }
}

/// a class's hue, spread round the wheel (the memo palette's order)
pub(crate) fn hue(i: usize, n: usize) -> f64 {
    (i as f64 * 360.0 / n.max(1) as f64 + 24.0) % 360.0
}

/// the classifier as a file: written to Downloads on the desktop, a download on the web
fn export(ui: Ui, api: Api, name: String) {
    dioxus::dioxus_core::spawn_forever(async move {
        let v = match api.call("classify.export", json!({ "name": name })).await {
            Ok(v) => v,
            Err(e) => return ui.fail(e),
        };
        let file = v["file"].as_str().unwrap_or("classifier.anendcls").to_string();
        let data = v["data"].as_str().unwrap_or("").to_string();
        if api.on_disk() {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let Some(bytes) = anend_core::classifiers::unbase64(&data) else {
                    return ui.fail("the export didn't read");
                };
                let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
                let downloads = home
                    .map(|h| std::path::PathBuf::from(h).join("Downloads"))
                    .filter(|d| d.is_dir());
                let dir = downloads.unwrap_or_else(|| {
                    std::path::PathBuf::from(api.describe())
                        .parent()
                        .map(|p| p.join("exports"))
                        .unwrap_or_default()
                });
                let _ = std::fs::create_dir_all(&dir);
                let path = dir.join(&file);
                match std::fs::write(&path, bytes) {
                    Ok(()) => ui.say(format!("exported to {}", path.display())),
                    Err(e) => ui.fail(format!("couldn't write {}: {e}", path.display())),
                }
            }
        } else {
            let js = format!(
                "const b = Uint8Array.from(atob({}), c => c.charCodeAt(0)); const a = document.createElement('a'); a.href = URL.createObjectURL(new Blob([b], {{type: 'application/octet-stream'}})); a.download = {}; a.click(); setTimeout(() => URL.revokeObjectURL(a.href), 4000);",
                serde_json::to_string(&data).unwrap_or_default(),
                serde_json::to_string(&file).unwrap_or_default()
            );
            document::eval(&js);
            ui.say(format!("exported {file}"));
        }
    });
}

// ------------------------------------------------------------------ training

#[component]
fn Train(name: String, details: Value, job: Option<Value>, on_start: EventHandler<Value>) -> Element {
    let ui = ui();
    let api = api();
    let mut epochs = use_signal(|| 8u32);
    let mut fresh = use_signal(|| false);
    let device = use_signal(|| "cpu".to_string());
    let state = job
        .as_ref()
        .and_then(|j| j["state"].as_str())
        .unwrap_or("idle")
        .to_string();
    let running = state == "running";
    let trained = details["trained"].as_bool().unwrap_or(false);
    let last = details["history"]
        .as_array()
        .and_then(|h| h.iter().rev().find(|e| e["kind"] == "train"))
        .cloned()
        .unwrap_or(Value::Null);
    let (api_t, n_t, api_c, n_c) = (api.clone(), name.clone(), api.clone(), name.clone());
    let start = move |_| {
        let (api, n) = (api_t.clone(), n_t.clone());
        let args = json!({ "name": n, "epochs": *epochs.peek(), "fresh": *fresh.peek(), "device": *device.peek() });
        dioxus::dioxus_core::spawn_forever(async move {
            match api.call("classify.train", args).await {
                Ok(j) => {
                    ui.say(format!("training {n}…"));
                    on_start.call(j);
                    ui.touched();
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let cancel = move |_| {
        let (api, n) = (api_c.clone(), n_c.clone());
        dioxus::dioxus_core::spawn_forever(async move {
            if let Err(e) = api.cancel_training(&n).await {
                ui.fail(e);
            }
        });
    };
    rsx! {
        div { class: "card",
            div { class: "row",
                h3 { Icon { name: "play" } "training" }
                span { class: "grow" }
                if !running {
                    label { class: "row small", "epochs"
                        input { r#type: "number", min: "1", max: "200", style: "width: 64px;", initial_value: "{epochs}",
                            oninput: move |e| if let Ok(v) = e.value().parse::<u32>() { epochs.set(v.clamp(1, 200)) } }
                    }
                    if trained {
                        label { class: "row small", title: "from new weights instead of going on from these",
                            input { r#type: "checkbox", checked: fresh(), onchange: move |e| fresh.set(e.checked()) } "from scratch" }
                    }
                    DevicePick { device }
                    button { class: "primary", onclick: start, Icon { name: "play" } if trained { "train more" } else { "train" } }
                } else {
                    button { onclick: cancel, Icon { name: "pause" } "cancel" }
                }
            }
            if let Some(j) = job.as_ref().filter(|_| running) {
                Progress { job: j.clone() }
            } else {
                if state == "failed" {
                    div { class: "bad small", {j_err(&job)} }
                }
                if let Some(j) = job.as_ref().filter(|j| j["state"] == "done") {
                    div { class: "dim small", "the last run: " {format!("{:.1} s {}", j["seconds"].as_f64().unwrap_or(0.0), device_words(j["device"].as_str().unwrap_or("")))}
                        if let Some(n) = j["device_note"].as_str() { span { class: "bad", " · {n}" } } }
                }
                if state == "cancelled" {
                    div { class: "dim small", "the last run was cancelled; nothing was written" }
                }
                if last.is_object() {
                    Metrics { event: last.clone(), classes: strs(&details["classes"]) }
                } else if !trained {
                    div { class: "dim small", "It learns from its lists' members as they are when training starts. A tiny classifier trains in seconds; compact in a minute or so." }
                }
            }
        }
    }
}

fn j_err(job: &Option<Value>) -> String {
    job.as_ref()
        .and_then(|j| j["error"].as_str())
        .unwrap_or("it failed")
        .to_string()
}

/// where a training runs: the CPU (the reference, bit for bit the same everywhere), the GPU
/// in exact precision, or the GPU fast (plain operators, within a hair of the CPU)
#[component]
pub(crate) fn DevicePick(device: Signal<String>) -> Element {
    let mut device = device;
    rsx! {
        label { class: "row small", title: "where it trains: without a GPU (or on a hub that keeps members on its CPU) it falls back to the CPU and says so",
            "on"
            select { "aria-label": "where it trains", value: "{device}", onchange: move |e| device.set(e.value()),
                option { value: "cpu", selected: device() == "cpu", "the CPU" }
                option { value: "gpu", selected: device() == "gpu", "the GPU" }
                option { value: "gpu_fast", selected: device() == "gpu_fast", "the GPU, fast" }
            }
        }
    }
}

/// a job's device in words
pub(crate) fn device_words(d: &str) -> &'static str {
    match d {
        "gpu" => "on the GPU",
        "gpu_fast" => "on the GPU (fast)",
        "cpu" => "on the CPU",
        _ => "",
    }
}

#[component]
pub(crate) fn Progress(job: Value) -> Element {
    let steps = job["steps"].as_f64().unwrap_or(0.0).max(1.0);
    let step = job["step"].as_f64().unwrap_or(0.0);
    let rounds = job["rounds"].as_u64().unwrap_or(1);
    let done = (step / steps).clamp(0.0, 1.0);
    let losses: Vec<(f64, f64)> = job["losses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
        .collect();
    let line = spark(&losses, 240.0, 34.0);
    rsx! {
        div { class: "progress", role: "progressbar", "aria-valuenow": "{(done * 100.0).round()}", "aria-valuemin": "0", "aria-valuemax": "100",
            span { style: "width: {done * 100.0:.1}%;" }
        }
        div { class: "row small",
            span { {format!("{:.0}%", done * 100.0)} }
            span { class: "dim", {format!("epoch {} · step {} of {}", job["epoch"].as_u64().unwrap_or(0) + 1, step as u64, steps as u64)} }
            if rounds > 1 { span { class: "dim", {format!("round {} of {rounds}", job["round"].as_u64().unwrap_or(0) + 1)} } }
            span { class: "dim", {format!("loss {:.3}", job["loss"].as_f64().unwrap_or(0.0))} }
            span { class: "dim", {format!("{:.0} tokens/s · {:.0} s · {} texts", job["tokens_per_sec"].as_f64().unwrap_or(0.0), job["seconds"].as_f64().unwrap_or(0.0), job["examples"])} }
            span { class: "device", {device_words(job["device"].as_str().unwrap_or(""))} }
        }
        if let Some(n) = job["device_note"].as_str() {
            div { class: "small bad", "{n}" }
        }
        if !line.is_empty() {
            svg { class: "spark", width: "240", height: "34", view_box: "0 0 240 34", role: "img", "aria-label": "the loss as it trains",
                polyline { points: "{line}" }
            }
        }
    }
}

/// points of a line through (x, y) pairs fitted to w × h
pub(crate) fn spark(p: &[(f64, f64)], w: f64, h: f64) -> String {
    if p.len() < 2 {
        return String::new();
    }
    let (x0, x1) = (p[0].0, p[p.len() - 1].0.max(p[0].0 + 1.0));
    let lo = p.iter().map(|q| q.1).fold(f64::INFINITY, f64::min);
    let hi = p.iter().map(|q| q.1).fold(f64::NEG_INFINITY, f64::max).max(lo + 1e-9);
    p.iter()
        .map(|(x, y)| {
            format!(
                "{:.1},{:.1}",
                (x - x0) / (x1 - x0) * w,
                h - 2.0 - (y - lo) / (hi - lo) * (h - 4.0)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[component]
pub(crate) fn Metrics(event: Value, classes: Vec<String>) -> Element {
    let m = &event["metrics"];
    let on = event["measured_on"].as_str().unwrap_or("");
    let per = m["classes"].as_array().cloned().unwrap_or_default();
    let confusion: Vec<Vec<u64>> = m["confusion"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| r.as_array().into_iter().flatten().filter_map(Value::as_u64).collect())
        .collect();
    rsx! {
        div { class: "figures",
            div { b { {pct(m["accuracy"].as_f64().unwrap_or(0.0))} } "accuracy" }
            div { b { {format!("{:.2}", m["macro_f1"].as_f64().unwrap_or(0.0))} } "macro F1" }
            div { b { {format!("{:.3}", m["log_loss"].as_f64().unwrap_or(0.0))} } "log loss" }
            div { b { "{event[\"examples\"]}" } "texts" }
            div { b { {format!("{:.0} s", event["seconds"].as_f64().unwrap_or(0.0))} } "{event[\"steps\"]} steps" }
        }
        div { class: "dim small", {format!("measured on {} texts {}", m["n"], if on.is_empty() { "held out".to_string() } else { on.to_string() })} " · " {event["note"].as_str().unwrap_or("").to_string()} }
        if !per.is_empty() {
            table { class: "confusion",
                thead { tr { th { "" } th { "F1" } th { "texts" } for c in classes.iter() { th { key: "{c}", title: "said {c}", "→ {c}" } } } }
                tbody {
                    for (i , c) in classes.iter().enumerate() {
                        tr { key: "{c}",
                            th { "{c}" }
                            td { {format!("{:.2}", per.get(i).and_then(|s| s["f1"].as_f64()).unwrap_or(0.0))} }
                            td { {per.get(i).map(|s| s["support"].to_string()).unwrap_or_default()} }
                            for (j , n) in confusion.get(i).cloned().unwrap_or_default().into_iter().enumerate() {
                                td { key: "{j}", class: if i == j { "hit" } else if n > 0 { "miss" } else { "" }, "{n}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ trying it

#[component]
fn Try(name: String, classes: Vec<String>) -> Element {
    let ui = ui();
    let api = api();
    let mut similar: Signal<Option<Vec<Value>>> = use_signal(|| None);
    let n0 = name.clone();
    let api_p = api.clone();
    let out = use_resource(move || {
        let (api, n) = (api_p.clone(), n0.clone());
        let t = ui.bar.read().trim().to_string();
        let shown = (ui.view)() == View::Models;
        let _ = (ui.changed)();
        async move {
            if t.is_empty() || !shown {
                return None;
            }
            // a beat after typing stops
            sleep_ms(180).await;
            Some(api.run_model(&n, &t).await)
        }
    });
    let v = match &*out.read() {
        Some(Some(Ok(v))) => v.clone(),
        Some(Some(Err(e))) => return rsx! { div { class: "card bad", "{e}" } },
        _ => {
            return rsx! {
                div { class: "card",
                    h3 { Icon { name: "text" } "try it" }
                    div { class: "dim", "Type a text in the command bar below: each class's probability shows here as you type." }
                }
            };
        }
    };
    let r = v["results"][0].clone();
    let text = r["text"].as_str().unwrap_or("").to_string();
    let probs = nums(&r["probs"]);
    let members = strs(&r["members"]);
    let top = r["class"].as_str().unwrap_or("").to_string();
    let api_s = api.clone();
    let (n_s, t_s) = (name.clone(), text.clone());
    rsx! {
        div { class: "card",
            div { class: "row",
                h3 { Icon { name: "text" } "try it" }
                span { class: "dim small t", "“{text}”" }
                span { class: "grow" }
                button { class: "plain", "data-tip": "the texts it reads most like this one", "aria-label": "texts like this",
                    onclick: move |_| {
                        let (api, n, t) = (api_s.clone(), n_s.clone(), t_s.clone());
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.call("classify.similar", json!({ "name": n, "text": t, "limit": 12 })).await {
                                Ok(v) => similar.set(Some(v["results"].as_array().cloned().unwrap_or_default())),
                                Err(e) => ui.fail(e),
                            }
                        });
                    },
                    Icon { name: "near" } "texts like this" }
            }
            div { class: "results",
                for (i , c) in classes.iter().cloned().enumerate() {
                    ClassRow { key: "{c}", name: name.clone(), class: c.clone(), prob: probs.get(i).copied().unwrap_or(0.0),
                        top: c == top, member: members.contains(&c), hue: hue(i, classes.len()), key_: None, text: text.clone() }
                }
            }
            if let Some(found) = similar() {
                div { class: "row small", span { class: "dim", "most like it:" } button { class: "plain", "aria-label": "close", onclick: move |_| similar.set(None), Icon { name: "close" } } }
                div { class: "results",
                    for h in found {
                        div { key: "{h[\"key\"]}", class: "result",
                            span { class: "p", {format!("{:.2}", h["cosine"].as_f64().unwrap_or(0.0))} }
                            span { class: "t", title: "{h[\"text\"].as_str().unwrap_or(\"\")}", {h["text"].as_str().unwrap_or("").to_string()} }
                            button { class: "plain", "aria-label": "into the bar", "data-tip": "into the bar",
                                onclick: { let t = h["text"].as_str().unwrap_or("").to_string(); move |_| ui.set_bar(&t) }, Icon { name: "text" } }
                        }
                    }
                }
            }
        }
    }
}

/// a class's probability for a text, with yes and no (Ctrl+Z takes either back)
#[component]
fn ClassRow(
    name: String,
    class: String,
    prob: f64,
    top: bool,
    member: bool,
    hue: f64,
    key_: Option<String>,
    text: String,
) -> Element {
    let ui = ui();
    let api = api();
    rsx! {
        div { class: if top { "result best" } else { "result" },
            span { class: "t", style: "--hue: {hue};", span { class: "swatch" } "{class}" if member { span { class: "dim small", " · in it" } } }
            span { class: "probbar wide", span { style: "width: {prob * 100.0:.1}%; background: hsl({hue} 45% 55%);" } }
            span { class: "p", {pct(prob)} }
            for yes in [true, false] {
                button { class: "plain", "aria-label": if yes { "yes, it is {class}" } else { "no, it isn't {class}" }, "data-tip": if yes { "yes, it is" } else { "no, it isn't" },
                    onclick: { let (api, n, t, c, k) = (api.clone(), name.clone(), text.clone(), class.clone(), key_.clone()); move |_| {
                        let (api, n, t, c, k) = (api.clone(), n.clone(), t.clone(), c.clone(), k.clone());
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.model_feedback(&n, k.as_deref(), &t, &c, yes).await {
                                Ok(v) => {
                                    let e = &v["entry"];
                                    ui.did(Did::Feedback { model: n });
                                    ui.say(format!("told it {}: {c} {} → {} · Ctrl+Z takes it back", if yes { "yes" } else { "no" },
                                        pct(e["before"].as_f64().unwrap_or(0.0)), pct(e["after"].as_f64().unwrap_or(0.0))));
                                    ui.touched();
                                }
                                Err(e) => ui.fail(e),
                            }
                        });
                    } },
                    Icon { name: if yes { "check" } else { "close" } } }
            }
        }
    }
}

// ------------------------------------------------------------------ steering

#[component]
fn Steer(name: String, details: Value) -> Element {
    let ui = ui();
    let api = api();
    let classes = strs(&details["classes"]);
    let h = &details["steer"]["heads"][0];
    let mut bias = use_signal(|| nums(&h["bias"]));
    let mut temperature = use_signal(|| h["temperature"].as_f64().unwrap_or(1.0));
    let mut threshold = use_signal(|| h["threshold"].as_f64().unwrap_or(0.5));
    let g0 = &details["steer"]["gains"];
    let mut gains = use_signal(|| ["letter", "word", "side", "extra"].map(|k| g0[k].as_f64().unwrap_or(1.0)));
    let mut report: Signal<Option<Value>> = use_signal(|| None);
    let mut form = use_signal(|| 0u64);
    let apply = {
        let (api, n) = (api.clone(), name.clone());
        move |reset: bool| {
            let (api, n) = (api.clone(), n.clone());
            let steer = if reset {
                json!({ "reset": true })
            } else {
                {
                    let g = *gains.peek();
                    json!({ "bias": *bias.peek(), "temperature": *temperature.peek(), "threshold": *threshold.peek(),
                            "gains": { "letter": g[0], "word": g[1], "side": g[2], "extra": g[3] } })
                }
            };
            dioxus::dioxus_core::spawn_forever(async move {
                match api.call("classify.steer", json!({ "name": n, "steer": steer })).await {
                    Ok(v) => {
                        let h = &v["steer"]["heads"][0];
                        if reset {
                            bias.set(nums(&h["bias"]));
                            temperature.set(h["temperature"].as_f64().unwrap_or(1.0));
                            threshold.set(h["threshold"].as_f64().unwrap_or(0.5));
                            let g = &v["steer"]["gains"];
                            gains.set(["letter", "word", "side", "extra"].map(|k| g[k].as_f64().unwrap_or(1.0)));
                            form += 1;
                        }
                        report.set(Some(v["bias"].clone()));
                        ui.touched();
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let (a1, a2, a3, a4) = (apply.clone(), apply.clone(), apply.clone(), apply.clone());
    let b = bias();
    rsx! {
        div { class: "card",
            div { class: "row",
                h3 { Icon { name: "sliders" } "steering" }
                span { class: "dim small", "your hand on its answers; training never changes these" }
                span { class: "grow" }
                button { class: "plain", onclick: move |_| a4(true), Icon { name: "refresh" } "reset" }
            }
            for g in [form()] {
                div { key: "{g}", class: "form",
                    for (i , c) in classes.iter().enumerate() {
                        label { "towards {c}" }
                        div { class: "row",
                            input { r#type: "range", min: "-4", max: "4", step: "0.1", style: "flex: 1;", "aria-label": "bias towards {c}",
                                initial_value: "{b.get(i).copied().unwrap_or(0.0)}",
                                oninput: move |e| if let (Ok(v), Some(x)) = (e.value().parse::<f64>(), bias.write().get_mut(i)) { *x = v; },
                                onchange: { let a = a1.clone(); move |_| a(false) } }
                            span { class: "num", {format!("{:+.1}", b.get(i).copied().unwrap_or(0.0))} }
                        }
                    }
                    label { "temperature" }
                    div { class: "row",
                        input { r#type: "range", min: "0.2", max: "4", step: "0.05", style: "flex: 1;", "aria-label": "temperature",
                            initial_value: "{temperature}", oninput: move |e| if let Ok(v) = e.value().parse() { temperature.set(v) },
                            onchange: { let a = a2.clone(); move |_| a(false) } }
                        span { class: "num", {format!("{:.2}", temperature())} }
                    }
                    label { "counts as in at" }
                    div { class: "row",
                        input { r#type: "range", min: "0", max: "1", step: "0.01", style: "flex: 1;", "aria-label": "threshold",
                            initial_value: "{threshold}", oninput: move |e| if let Ok(v) = e.value().parse() { threshold.set(v) },
                            onchange: { let a = a3.clone(); move |_| a(false) } }
                        span { class: "num", {pct(threshold())} }
                    }
                    label { class: "wide-label", "channel gains" }
                    div { class: "dim small", "how much each kind of input counts: the letters, the words' sums, the text's own measures, other classifiers" }
                    for (i , what) in ["letters", "word sums", "the text's measures", "other classifiers"].into_iter().enumerate() {
                        label { key: "gl{i}", "{what}" }
                        div { class: "row",
                            input { r#type: "range", min: "0", max: "3", step: "0.05", style: "flex: 1;", "aria-label": "the gain of {what}",
                                initial_value: "{gains()[i]}", oninput: move |e| if let Ok(v) = e.value().parse::<f64>() { gains.write()[i] = v; },
                                onchange: { let a = a1.clone(); move |_| a(false) } }
                            span { class: "num", {format!("{:.2}", gains()[i])} }
                        }
                    }
                }
            }
            if let Some(r) = report().filter(|r| r.is_object()) {
                div { class: "dim small",
                    {format!("on {} texts: {} change their answer, mean shift {:.3} nats (largest {:.3})",
                        r["texts"], pct(r["flipped"].as_f64().unwrap_or(0.0)), r["mean_kl"].as_f64().unwrap_or(0.0), r["max_kl"].as_f64().unwrap_or(0.0))}
                }
            }
        }
    }
}

// ------------------------------------------------------------------ searching with it

#[component]
fn Search(name: String, classes: Vec<String>, outputs: Value) -> Element {
    let ui = ui();
    let api = api();
    let mut class = use_signal(|| 0usize);
    let mut words = use_signal(String::new);
    let mut found: Signal<Option<Vec<Scored>>> = use_signal(|| None);
    let mut busy = use_signal(|| false);
    let run = {
        let (api, n, classes) = (api.clone(), name.clone(), classes.clone());
        move || {
            let (api, n) = (api.clone(), n.clone());
            let c = classes.get(*class.peek()).cloned().unwrap_or_default();
            let q = words.peek().trim().to_string();
            busy.set(true);
            dioxus::dioxus_core::spawn_forever(async move {
                match api.model_search(&n, &c, &q).await {
                    Ok(r) => found.set(Some(r)),
                    Err(e) => ui.fail(e),
                }
                busy.set(false);
            });
        }
    };
    let (r1, r2) = (run.clone(), run.clone());
    let api_e = api.clone();
    let n_e = name.clone();
    let cur = classes.get(class()).cloned().unwrap_or_default();
    let as_list = outputs[cur.as_str()].as_str().map(str::to_string);
    let c_e = cur.clone();
    rsx! {
        div { class: "card",
            div { class: "row",
                h3 { Icon { name: "search" } "search by class" }
                for (i , c) in classes.iter().enumerate() {
                    button { key: "{c}", class: if class() == i { "chip on" } else { "chip" }, onclick: { let mut r = r1.clone(); move |_| { class.set(i); r(); } }, "{c}" }
                }
                input { r#type: "text", placeholder: "and these words (optional)", style: "flex: 1; min-width: 120px;",
                    oninput: move |e| words.set(e.value()),
                    onkeydown: { let mut r = r2.clone(); move |e: KeyboardEvent| if e.key() == Key::Enter { r() } } }
                button { class: "primary", disabled: busy(), onclick: { let mut r = run.clone(); move |_| r() }, Icon { name: "search" } "find" }
                button { "data-tip": if as_list.is_some() { "write the list again from its answers now" } else { "its answers for this class as a list, for lists and other classifiers" },
                    onclick: move |_| {
                        let (api, n, c) = (api_e.clone(), n_e.clone(), c_e.clone());
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.model_enlist(&n, &c).await {
                                Ok(v) => { ui.say(format!("“{}”: {} texts", v["name"].as_str().unwrap_or(""), v["members"])); ui.touched(); }
                                Err(e) => ui.fail(e),
                            }
                        });
                    },
                    Icon { name: "list" } if as_list.is_some() { "list again" } else { "as a list" } }
            }
            if let Some(hits) = found() {
                if hits.is_empty() {
                    div { class: "dim", "nothing found" }
                }
                div { class: "results",
                    for h in hits {
                        SearchRow { key: "{h.key.clone().unwrap_or_default()}", name: name.clone(), class: cur.clone(), hit: h }
                    }
                }
            }
        }
    }
}

#[component]
fn SearchRow(name: String, class: String, hit: Scored) -> Element {
    let ui = ui();
    let api = api();
    let t = hit.text.clone();
    rsx! {
        div { class: "result",
            span { class: "p", {pct(hit.prob)} }
            span { class: "probbar", span { style: "width: {hit.prob * 100.0:.1}%;" } }
            span { class: "t", title: "{hit.text}", if hit.note { "✎ " } "{hit.text}" }
            if hit.top_class.as_deref().is_some_and(|c| c != class) {
                span { class: "dim small", "reads as {hit.top_class.clone().unwrap_or_default()}" }
            }
            for yes in [true, false] {
                button { class: "plain", "aria-label": if yes { "yes" } else { "no" }, "data-tip": if yes { "yes, it is {class}" } else { "no, it isn't {class}" },
                    onclick: { let (api, n, c, k, t) = (api.clone(), name.clone(), class.clone(), hit.key.clone(), hit.text.clone()); move |_| {
                        let (api, n, c, k, t) = (api.clone(), n.clone(), c.clone(), k.clone(), t.clone());
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.model_feedback(&n, k.as_deref(), &t, &c, yes).await {
                                Ok(_) => { ui.did(Did::Feedback { model: n }); ui.say("told it; Ctrl+Z takes it back"); ui.touched(); }
                                Err(e) => ui.fail(e),
                            }
                        });
                    } },
                    Icon { name: if yes { "check" } else { "close" } } }
            }
            button { class: "plain", "aria-label": "into the bar", "data-tip": "into the bar", onclick: move |_| ui.set_bar(&t), Icon { name: "text" } }
        }
    }
}

// ------------------------------------------------------------------ verdicts given

#[component]
fn Verdicts(name: String, details: Value, on_change: EventHandler<()>) -> Element {
    let ui = ui();
    let api = api();
    let fb = details["feedback"].as_array().cloned().unwrap_or_default();
    let count = details["feedback_count"].as_u64().unwrap_or(0);
    let undo = details["undo"].as_u64().unwrap_or(0);
    if count == 0 {
        return rsx! {};
    }
    let n = name.clone();
    rsx! {
        div { class: "card",
            div { class: "row",
                h3 { Icon { name: "check" } "yes and no given" }
                span { class: "dim small", "{count} in all; kept and replayed when it trains" }
                span { class: "grow" }
                button { class: "plain", disabled: undo == 0, "data-tip": if undo == 0 { "nothing to take back since it last trained".to_string() } else { format!("take back the latest ({undo} can be)") },
                    onclick: move |_| {
                        let (api, n) = (api.clone(), n.clone());
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.undo_model_feedback(&n).await {
                                Ok(()) => { ui.say("took it back"); ui.touched(); on_change.call(()); }
                                Err(e) => ui.fail(e),
                            }
                        });
                    },
                    Icon { name: "undo" } "take back" }
            }
            div { class: "results",
                for (i , e) in fb.into_iter().take(8).enumerate() {
                    div { key: "{i}", class: "result",
                        span { class: "p", Icon { name: if e["yes"] == true { "check" } else { "close" } } }
                        span { class: "small", style: "flex: none;", {e["class"].as_str().unwrap_or("").to_string()} }
                        span { class: "t dim", {e["text"].as_str().unwrap_or("").to_string()} }
                        span { class: "p", {format!("{} → {}", pct(e["before"].as_f64().unwrap_or(0.0)), pct(e["after"].as_f64().unwrap_or(0.0)))} }
                    }
                }
            }
        }
    }
}
