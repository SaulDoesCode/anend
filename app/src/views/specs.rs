//! Spec models: models made from a declarative spec (docs/models.md, `models.*` in
//! docs/app-api.md), the second tab of the models view. The same view on the desktop and
//! the web, through the backend.
//!
//! The left column lists them; "new" starts one from a template (a classifier, a tagger,
//! a generator) filled in with the save's lists. A model's page has its spec in a JSON
//! editor (checked as you type, a broken rule shown with where it is; Ctrl+Enter saves),
//! its training (epochs, progress, cancel, the metrics), a run card (the command bar's
//! text through every head as you type) and, for a generator, a write card: candidates
//! judged by its critics, each critic's score shown (Enter in the bar writes after the
//! bar's text).

use dioxus::prelude::*;
use serde_json::{Value, json};

use super::classifiers::{Metrics, Progress, hue, pct, strs};
use crate::backend::ListInfo;
use crate::ui::*;

/// what the models view shares with this tab: the open model, whether "new" is open, and
/// a count the bar's Enter bumps (the write card writes after the bar's text)
#[derive(Clone, Copy, PartialEq)]
pub struct SpecState {
    pub chosen: Signal<Option<String>>,
    pub new: Signal<bool>,
    pub enter: Signal<u64>,
}

impl SpecState {
    pub fn new() -> SpecState {
        SpecState {
            chosen: Signal::new(None),
            new: Signal::new(false),
            enter: Signal::new(0),
        }
    }

    /// Escape: closes "new", else the open model
    pub fn escape(mut self) {
        if *self.new.peek() {
            self.new.set(false);
        } else {
            self.chosen.set(None);
        }
    }
}

#[component]
pub fn SpecModels(state: SpecState) -> Element {
    let ui = ui();
    let api = api();
    let SpecState {
        mut chosen, mut new, ..
    } = state;
    let mut rev = use_signal(|| 0u64);
    let api_r = api.clone();
    let models = use_resource(move || {
        let api = api_r.clone();
        let _ = ((ui.changed)(), rev());
        async move { api.spec_models().await }
    });
    let (list, note): (Vec<Value>, Option<String>) = match &*models.read() {
        Some(Ok(v)) => (v.clone(), None),
        Some(Err(e)) => (vec![], Some(e.clone())),
        None => (vec![], None),
    };
    let open = chosen().or_else(|| {
        (!new())
            .then(|| list.first().and_then(|m| m["name"].as_str().map(str::to_string)))
            .flatten()
    });
    let on_made = move |name: String| {
        new.set(false);
        chosen.set(Some(name));
        rev += 1;
    };
    rsx! {
        if let Some(n) = note {
            div { class: "card bad", "{n}" }
        }
        div { class: "cls-layout",
            div { class: "card cls-list", role: "listbox", "aria-label": "spec models",
                button { class: if new() { "cls-item on" } else { "cls-item" }, onclick: move |_| new.set(!new()),
                    b { Icon { name: "new" } " new" } span { class: "small", "from a template" } }
                if list.is_empty() {
                    span { class: "dim small", "none yet" }
                }
                for m in list.iter().cloned() {
                    {
                        let name = m["name"].as_str().unwrap_or("").to_string();
                        let on = open.as_deref() == Some(name.as_str()) && !new();
                        let heads: Vec<String> = m["heads"].as_array().into_iter().flatten()
                            .map(|h| format!("{} ({})", h["name"].as_str().unwrap_or(""), h["kind"].as_str().unwrap_or(""))).collect();
                        let state = if m["training"] == true { "training…".to_string() }
                            else if m["trained"] == true { trained_line(&m) }
                            else { "not trained yet".to_string() };
                        rsx! {
                            button { key: "{name}", role: "option", "aria-selected": "{on}",
                                class: if on { "cls-item on" } else { "cls-item" },
                                onclick: { let n = name.clone(); move |_| { chosen.set(Some(n.clone())); new.set(false); } },
                                b { "{name}" }
                                span { class: "small", {heads.join(" · ")} }
                                span { class: "small state", "{state}" }
                            }
                        }
                    }
                }
            }
            div { class: "cls-main",
                if new() {
                    NewSpec { on_made }
                } else {
                    match open {
                        Some(name) => rsx! { SpecPage { key: "{name}", name: name.clone(), enter: state.enter,
                            on_gone: move |_| { chosen.set(None); rev += 1; },
                            on_renamed: move |n: String| { chosen.set(Some(n)); rev += 1; } } },
                        None => rsx! {
                            div { class: "card empty",
                                p { "A spec model is a model described as plain data: what it reads, how big it is, what comes out (classes, labels, numbers, or text it writes), what it learns from in this save, and for a writer, which other models judge what it writes." }
                                p { "Press " b { "new" } " to start from a template, then edit its spec, train it, and try it from the command bar." }
                            }
                        },
                    }
                }
            }
        }
    }
}

fn trained_line(m: &Value) -> String {
    let mut s = "trained".to_string();
    if let Some(a) = m["accuracy"].as_f64() {
        s += &format!(" · {}", pct(a));
    }
    if let Some(b) = m["bits_per_byte"].as_f64() {
        s += &format!(" · {b:.2} bits/byte");
    }
    s
}

// ------------------------------------------------------------------ templates

#[derive(Clone, Copy, PartialEq)]
enum Template {
    Classifier,
    Tagger,
    Generator,
    Pictures,
}

impl Template {
    const ALL: [Template; 4] = [
        Template::Classifier,
        Template::Tagger,
        Template::Generator,
        Template::Pictures,
    ];

    fn name(self) -> &'static str {
        match self {
            Template::Classifier => "classifier",
            Template::Tagger => "tagger",
            Template::Generator => "generator",
            Template::Pictures => "pictures",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Template::Classifier => "one class of several for a text: each list a class",
            Template::Tagger => "any of several labels for a text, each from a list",
            Template::Generator => "writes texts like a list's, judged by another model",
            Template::Pictures => "one class of several for a picture: each picture list a class",
        }
    }
}

/// a judge a generator can use: a model's name, its head and a class of it
pub type Judge = (String, String, String);

/// a template's spec, filled in with the save's lists (by name), its picture lists and a
/// judge if there is one
fn template(t: Template, lists: &[ListInfo], judge: Option<&Judge>, plists: &[String]) -> String {
    let names: Vec<String> = lists.iter().filter(|l| l.count > 0).map(|l| l.name.clone()).collect();
    let pick = |n: usize| -> Vec<String> {
        let mut v: Vec<String> = names.iter().take(n).cloned().collect();
        for i in v.len()..n.min(2).max(v.len()) {
            v.push(format!("list {}", i + 1));
        }
        v
    };
    // written by hand so the fields keep the order docs/models.md gives them
    let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let classes = |c: &[String]| c.iter().map(|x| q(x)).collect::<Vec<_>>().join(", ");
    let data = |head: &str, c: &[String]| {
        c.iter()
            .map(|x| {
                format!(
                    "    {{\"from\": \"list\", \"list\": {}, \"head\": {}, \"class\": {}}}",
                    q(x),
                    q(head),
                    q(x)
                )
            })
            .collect::<Vec<_>>()
            .join(",\n")
    };
    match t {
        Template::Pictures => {
            let mut c: Vec<String> = plists.iter().take(3).cloned().collect();
            for i in c.len()..2 {
                c.push(format!("pictures {}", i + 1));
            }
            let d = c
                .iter()
                .map(|x| {
                    format!(
                        "    {{\"from\": \"pictures\", \"list\": {}, \"head\": \"what\", \"class\": {}}}",
                        q(x),
                        q(x)
                    )
                })
                .collect::<Vec<_>>()
                .join(",\n");
            format!(
                "{{\n  \"name\": \"what it shows\",\n  \"input\": {{\"type\": \"image\", \"width\": 32, \"height\": 32, \"patch\": 8, \"codes\": 64}},\n  \"trunk\": {{\"preset\": \"compact\"}},\n  \"heads\": [\n    {{\"name\": \"what\", \"kind\": \"classify\", \"classes\": [{cl}]}}\n  ],\n  \"data\": [\n{d}\n  ],\n  \"train\": {{\"epochs\": 30, \"batch\": 8}}\n}}\n",
                cl = classes(&c),
            )
        }
        Template::Classifier | Template::Tagger => {
            let c = pick(3);
            let (name, kind) = if t == Template::Classifier {
                ("kind", "classify")
            } else {
                ("tags", "multi")
            };
            format!(
                "{{\n  \"name\": {n},\n  \"input\": {{\"type\": \"text\"}},\n  \"trunk\": {{\"preset\": \"compact\"}},\n  \"heads\": [\n    {{\"name\": {n}, \"kind\": \"{kind}\", \"classes\": [{cl}]}}\n  ],\n  \"data\": [\n{d}\n  ],\n  \"train\": {{\"epochs\": 20}}\n}}\n",
                n = q(name),
                cl = classes(&c),
                d = data(name, &c),
            )
        }
        Template::Generator => {
            let l = pick(1).into_iter().next().unwrap_or_else(|| "list 1".into());
            let critics = match judge {
                Some((m, h, c)) => format!(
                    "  \"critics\": [\n    {{\"model\": {}, \"head\": {}, \"want\": {{\"class\": {}}}}}\n  ],\n",
                    q(m),
                    q(h),
                    q(c)
                ),
                None => String::new(),
            };
            format!(
                "{{\n  \"name\": \"writer\",\n  \"input\": {{\"type\": \"text\"}},\n  \"trunk\": {{\"preset\": \"compact\", \"max_len\": 96}},\n  \"heads\": [\n    {{\"name\": \"next\", \"kind\": \"generate\"}}\n  ],\n  \"data\": [\n    {{\"from\": \"list\", \"list\": {l}, \"head\": \"next\"}}\n  ],\n{critics}  \"constraints\": {{\"max_len\": 120}},\n  \"sampling\": {{\"temperature\": 0.7, \"top_p\": 0.9, \"max_len\": 60, \"seed\": 1}},\n  \"train\": {{\"epochs\": 80, \"batch\": 8, \"lr\": 0.003}}\n}}\n",
                l = q(&l),
            )
        }
    }
}

#[component]
fn NewSpec(on_made: EventHandler<String>) -> Element {
    let ui = ui();
    let api = api();
    let mut which = use_signal(|| Template::Classifier);
    // the spec as edited (None: the template as it is)
    let mut text: Signal<Option<String>> = use_signal(|| None);
    let mut form = use_signal(|| 0u64);
    let mut busy = use_signal(|| false);
    let api_l = api.clone();
    let ctx = use_resource(move || {
        let api = api_l.clone();
        async move {
            let lists = api.lists().await.map(|l| l.lists).unwrap_or_default();
            // a judge for a writer: a trained classifier, else a spec model's classify head
            let mut judge: Option<Judge> = None;
            if let Ok(cls) = api.models().await
                && let Some(c) = cls.iter().find(|c| c.trained && !c.classes.is_empty())
            {
                judge = Some((c.name.clone(), c.name.clone(), c.classes[0].clone()));
            }
            if judge.is_none()
                && let Ok(ms) = api.spec_models().await
            {
                judge = ms.iter().find_map(|m| {
                    let h = m["heads"].as_array()?.iter().find(|h| h["kind"] == "classify")?;
                    Some((
                        m["name"].as_str()?.to_string(),
                        h["name"].as_str()?.to_string(),
                        h["classes"][0].as_str()?.to_string(),
                    ))
                });
            }
            let plists: Vec<String> = api
                .picture_lists()
                .await
                .unwrap_or_default()
                .iter()
                .filter_map(|l| l["list"].as_str().map(str::to_string))
                .collect();
            (lists, judge, plists)
        }
    });
    let Some((lists, judge, plists)) = ctx.read().clone() else {
        return rsx! { div { class: "card dim", "…" } };
    };
    let shown = text
        .peek()
        .clone()
        .unwrap_or_else(|| template(*which.peek(), &lists, judge.as_ref(), &plists));
    let create = {
        let api = api.clone();
        let (lists, judge, plists) = (lists.clone(), judge.clone(), plists.clone());
        move || {
            let api = api.clone();
            let t = text
                .peek()
                .clone()
                .unwrap_or_else(|| template(*which.peek(), &lists, judge.as_ref(), &plists));
            busy.set(true);
            dioxus::dioxus_core::spawn_forever(async move {
                match api.create_spec(&t).await {
                    Ok(v) => {
                        let n = v["name"].as_str().unwrap_or("").to_string();
                        ui.say(format!("made {n}; train it next"));
                        on_made.call(n);
                    }
                    Err(e) => ui.fail(e),
                }
                busy.set(false);
            });
        }
    };
    let create2 = create.clone();
    rsx! {
        div { class: "card cls-new",
            h3 { Icon { name: "new" } "a new spec model" }
            div { class: "row",
                for t in Template::ALL {
                    button { key: "{t.name()}", class: if which() == t { "chip on" } else { "chip" }, "aria-pressed": "{which() == t}",
                        "data-tip": "{t.about()}",
                        onclick: move |_| {
                            which.set(t);
                            text.set(None);
                            form += 1;
                        },
                        "{t.name()}" }
                }
                span { class: "dim small", {which().about()} }
            }
            for g in [form()] {
                SpecEditor { key: "{g}", text: shown.clone(), on_change: move |t: String| text.set(Some(t)), on_save: { let mut c = create2.clone(); move |_| c() } }
            }
            div { class: "row",
                span { class: "dim small", "lists are named by their names; docs/models.md has every field" }
                span { class: "grow" }
                button { class: "primary", disabled: busy(), onclick: { let mut c = create.clone(); move |_| c() }, Icon { name: "check" } "make it" }
            }
        }
    }
}

// ------------------------------------------------------------------ the editor

/// a spec's JSON in a monospace textarea, checked a beat after typing stops: a broken
/// rule shows under it with where in the spec it is; Ctrl+Enter saves
#[component]
fn SpecEditor(text: String, on_change: EventHandler<String>, on_save: EventHandler<()>) -> Element {
    let api = api();
    let mut cur = use_signal(|| text.clone());
    let rows = text.lines().count().clamp(8, 40);
    let check = use_resource(move || {
        let api = api.clone();
        let t = cur();
        async move {
            sleep_ms(300).await;
            api.check_spec(&t).await
        }
    });
    let verdict = check.read().clone();
    rsx! {
        div { class: "spec-editor",
            textarea {
                class: "spec-text",
                rows: "{rows}",
                spellcheck: "false",
                "aria-label": "the model's spec (JSON)",
                initial_value: "{text}",
                oninput: move |e| { let v = e.value(); cur.set(v.clone()); on_change.call(v); },
                onkeydown: move |e: KeyboardEvent| {
                    if e.key() == Key::Enter && (e.modifiers().ctrl() || e.modifiers().meta()) {
                        e.prevent_default();
                        on_save.call(());
                    }
                },
            }
            match verdict {
                Some(Ok(v)) if v["ok"] == true => rsx! { div { class: "spec-ok small", Icon { name: "check" } "the spec reads" } },
                Some(Ok(v)) => rsx! {
                    div { class: "spec-err small", role: "alert",
                        Icon { name: "alert" }
                        if let Some(p) = v["path"].as_str().filter(|p| !p.is_empty()) { code { "{p}" } }
                        span { {v["message"].as_str().unwrap_or("").to_string()} }
                    }
                },
                Some(Err(e)) => rsx! { div { class: "spec-err small", "{e}" } },
                None => rsx! {},
            }
        }
    }
}

// ------------------------------------------------------------------ one model

#[component]
fn SpecPage(name: String, enter: Signal<u64>, on_gone: EventHandler<()>, on_renamed: EventHandler<String>) -> Element {
    let ui = ui();
    let api = api();
    let mut rev = use_signal(|| 0u64);
    let mut armed = use_signal(|| false);
    let mut job: Signal<Option<Value>> = use_signal(|| None);
    let mut watching = use_signal(|| true);
    // the spec as edited (None: as stored)
    let mut text: Signal<Option<String>> = use_signal(|| None);
    let mut form = use_signal(|| 0u64);
    let n0 = name.clone();
    let api_d = api.clone();
    let details = use_resource(move || {
        let (api, n) = (api_d.clone(), n0.clone());
        let _ = ((ui.changed)(), rev());
        async move { api.spec_model(&n).await }
    });
    {
        let (api, n) = (api.clone(), name.clone());
        use_future(move || {
            let (api, n) = (api.clone(), n.clone());
            async move {
                let mut was_running = false;
                loop {
                    if *watching.peek() {
                        match api.spec_job(&n).await {
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
                                        rev += 1;
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
    let source = d["source"].as_str().unwrap_or("").to_string();
    let trained = d["trained"] == true;
    let running = job().is_some_and(|j| j["state"] == "running");
    let generates = d["generates"] == true;
    let heads: Vec<Value> = d["heads"].as_array().cloned().unwrap_or_default();
    let edited = text().is_some_and(|t| t != source);
    let shown = text.peek().clone().unwrap_or_else(|| source.clone());
    let save = {
        let (api, n, source) = (api.clone(), name.clone(), source.clone());
        move || {
            let (api, n) = (api.clone(), n.clone());
            let t = text.peek().clone().unwrap_or_else(|| source.clone());
            dioxus::dioxus_core::spawn_forever(async move {
                match api.update_spec(&n, &t).await {
                    Ok(v) => {
                        let new = v["name"].as_str().unwrap_or("").to_string();
                        let kept = if v["weights_kept"] == true {
                            "; its training stays"
                        } else if v["trained"] == false && trained {
                            "; its shape changed, so it trains anew"
                        } else {
                            ""
                        };
                        ui.say(format!("saved {new}{kept}"));
                        text.set(None);
                        ui.touched();
                        if new != n {
                            on_renamed.call(new);
                        } else {
                            rev += 1;
                        }
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let save2 = save.clone();
    let api_del = api.clone();
    let n_del = name.clone();
    rsx! {
        div { class: "cls-page",
            div { class: "card cls-head",
                div { class: "row",
                    h2 { "{name}" }
                    for (i , h) in heads.iter().enumerate() {
                        span { key: "{i}", class: "chip cls-class", style: "--hue: {hue(i, heads.len())};",
                            title: strs(&h["classes"]).join(" · "),
                            {format!("{} · {}", h["name"].as_str().unwrap_or(""), h["kind"].as_str().unwrap_or(""))} }
                    }
                    span { class: "grow" }
                    button {
                        class: if armed() { "armed" } else { "plain" },
                        "data-tip": "delete it (its lists stay); click twice",
                        "aria-label": "delete the model",
                        disabled: running,
                        onclick: move |_| {
                            if !armed() { armed.set(true); return; }
                            let (api, n) = (api_del.clone(), n_del.clone());
                            dioxus::dioxus_core::spawn_forever(async move {
                                match api.delete_spec(&n).await {
                                    Ok(()) => { ui.say(format!("deleted {n}")); ui.touched(); on_gone.call(()); }
                                    Err(e) => ui.fail(e),
                                }
                            });
                        },
                        Icon { name: "delete" }
                        if armed() { "delete it?" }
                    }
                }
                div { class: "dim small",
                    {format!("reads {} · version {}{}{}{}",
                        d["input"].as_str().unwrap_or("text"),
                        d["version"],
                        d["pretrained"].as_str().map(|p| format!(" · from {p}")).unwrap_or_default(),
                        if d["critics"].as_array().is_some_and(|c| !c.is_empty()) { format!(" · judged by {}", strs(&d["critics"]).join(", ")) } else { String::new() },
                        if trained { format!(" · {} steps trained", d["steps"]) } else { " · not trained yet".into() },
                    )}
                }
            }
            div { class: "card",
                div { class: "row",
                    h3 { Icon { name: "editor" } "spec" }
                    span { class: "dim small", if edited { "edited · Ctrl+Enter saves" } else { "as stored" } }
                    span { class: "grow" }
                    button { disabled: !edited, onclick: move |_| { text.set(None); form += 1; }, Icon { name: "undo" } "as stored" }
                    button { class: "primary", disabled: !edited || running, onclick: { let c = save.clone(); move |_| c() }, Icon { name: "save" } "save" }
                }
                for g in [form()] {
                    SpecEditor { key: "{g}", text: shown.clone(), on_change: move |t: String| text.set(Some(t)), on_save: { let c = save2.clone(); move |_| c() } }
                }
            }
            SpecTrain { name: name.clone(), details: d.clone(), job: job(), on_start: move |j: Value| { job.set(Some(j)); watching.set(true); } }
            SpecRun { name: name.clone(), trained }
            if generates {
                SpecWrite { name: name.clone(), enter }
            }
        }
    }
}

// ------------------------------------------------------------------ training

#[component]
fn SpecTrain(name: String, details: Value, job: Option<Value>, on_start: EventHandler<Value>) -> Element {
    let ui = ui();
    let api = api();
    let mut epochs: Signal<Option<u32>> = use_signal(|| None);
    let mut fresh = use_signal(|| false);
    let state = job
        .as_ref()
        .and_then(|j| j["state"].as_str())
        .unwrap_or("idle")
        .to_string();
    let running = state == "running";
    let trained = details["trained"] == true;
    let last = details["history"].as_array().and_then(|h| h.last()).cloned();
    let (api_t, n_t, api_c, n_c) = (api.clone(), name.clone(), api.clone(), name.clone());
    let start = move |_| {
        let (api, n) = (api_t.clone(), n_t.clone());
        let (e, f) = (*epochs.peek(), *fresh.peek());
        dioxus::dioxus_core::spawn_forever(async move {
            match api.train_spec(&n, e, f).await {
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
            if let Err(e) = api.cancel_spec(&n).await {
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
                    label { class: "row small", title: "the spec's train.epochs when left empty", "epochs"
                        input { r#type: "number", min: "1", max: "1000", style: "width: 64px;", placeholder: "spec",
                            oninput: move |e| epochs.set(e.value().parse::<u32>().ok().map(|v| v.clamp(1, 1000))) }
                    }
                    if trained {
                        label { class: "row small", title: "from new weights instead of going on from these",
                            input { r#type: "checkbox", checked: fresh(), onchange: move |e| fresh.set(e.checked()) } "from scratch" }
                    }
                    button { class: "primary", onclick: start, Icon { name: "play" } if trained { "train more" } else { "train" } }
                } else {
                    button { onclick: cancel, Icon { name: "pause" } "cancel" }
                }
            }
            if let Some(j) = job.as_ref().filter(|_| running) {
                Progress { job: j.clone() }
            } else {
                if state == "failed" {
                    div { class: "bad small", {job.as_ref().and_then(|j| j["error"].as_str()).unwrap_or("it failed").to_string()} }
                }
                if state == "cancelled" {
                    div { class: "dim small", "the last run was cancelled; nothing was written" }
                }
                if let Some(t) = last {
                    TrainedFigures { t, heads: details["heads"].as_array().cloned().unwrap_or_default() }
                } else {
                    div { class: "dim small", "It learns from what its data queries find in this save when training starts (a list's members, a tag's entries)." }
                }
            }
        }
    }
}

#[component]
fn TrainedFigures(t: Value, heads: Vec<Value>) -> Element {
    // the spec's heads, for their class names
    let decls = heads;
    let heads: Vec<Value> = t["heads"].as_array().cloned().unwrap_or_default();
    rsx! {
        div { class: "figures",
            if let Some(l) = t["loss"].as_f64() { div { b { {format!("{l:.3}")} } "loss" } }
            if let Some(b) = t["bits_per_byte"].as_f64() { div { b { {format!("{b:.2}")} } "bits per byte (held out)" } }
            div { b { "{t[\"examples\"]}" } {format!("examples · {} held out", t["val_examples"])} }
            div { b { {format!("{:.0} s", t["seconds"].as_f64().unwrap_or(0.0))} } "{t[\"steps\"]} steps" }
            div { b { {format!("{:.0}", t["tokens_per_sec"].as_f64().unwrap_or(0.0))} } "tokens/s" }
            if t["weights"].as_u64().is_some_and(|w| w > 0) { div { b { {weights(t["weights"].as_u64().unwrap_or(0))} } "weights" } }
        }
        if heads.is_empty() && t["val_examples"] == 0 {
            div { class: "dim small", "no examples were held out, so there are no metrics (train.val_fraction in the spec)" }
        }
        for h in heads {
            {
                let name = h[0].as_str().unwrap_or("").to_string();
                let m = h[1].clone();
                let classes: Vec<String> = decls.iter().find(|d| d["name"] == name.as_str())
                    .map(|d| strs(&d["classes"])).unwrap_or_default();
                let ev = json!({ "metrics": m, "examples": t["examples"], "seconds": t["seconds"], "steps": t["steps"] });
                rsx! {
                    div { key: "{name}", class: "spec-head",
                        h4 { "head {name}" }
                        Metrics { event: ev, classes }
                    }
                }
            }
        }
    }
}

fn weights(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} M", n as f64 / 1e6)
    } else if n >= 1000 {
        format!("{:.0} k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

// ------------------------------------------------------------------ running it

/// every head's answer on the command bar's text, as you type
#[component]
fn SpecRun(name: String, trained: bool) -> Element {
    let ui = ui();
    let api = api();
    let n0 = name.clone();
    let out = use_resource(move || {
        let (api, n) = (api.clone(), n0.clone());
        let t = ui.bar.read().trim().to_string();
        let shown = (ui.view)() == View::Models;
        let _ = (ui.changed)();
        async move {
            if t.is_empty() || !shown {
                return None;
            }
            sleep_ms(200).await;
            Some((t.clone(), api.run_spec(&n, &t).await))
        }
    });
    let (text, v) = match &*out.read() {
        Some(Some((t, Ok(v)))) => (t.clone(), v.clone()),
        Some(Some((_, Err(e)))) => return rsx! { div { class: "card bad", "{e}" } },
        _ => {
            return rsx! {
                div { class: "card",
                    h3 { Icon { name: "text" } "run it" }
                    div { class: "dim", "Type a text in the command bar below: every head's answer shows here as you type." }
                }
            };
        }
    };
    let outputs: Vec<(String, Value)> = v["outputs"]
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    rsx! {
        div { class: "card",
            div { class: "row",
                h3 { Icon { name: "text" } "run it" }
                span { class: "dim small t", "“{text}”" }
                if !trained { span { class: "dim small", "untrained: its starting weights answer" } }
            }
            for (head , a) in outputs {
                div { key: "{head}", class: "spec-head",
                    h4 { "{head}" }
                    {answer(&a)}
                }
            }
        }
    }
}

fn answer(a: &Value) -> Element {
    match a["kind"].as_str() {
        Some("classes") => {
            let probs: Vec<(String, f64)> = a["probs"]
                .as_object()
                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_f64().unwrap_or(0.0))).collect())
                .unwrap_or_default();
            let top = a["top"].as_str().unwrap_or("").to_string();
            let members = strs(&a["members"]);
            let n = probs.len();
            rsx! {
                div { class: "results",
                    for (i , (c , p)) in probs.into_iter().enumerate() {
                        div { key: "{c}", class: if c == top { "result best" } else { "result" },
                            span { class: "t", style: "--hue: {hue(i, n)};", span { class: "swatch" } "{c}"
                                if members.contains(&c) { span { class: "dim small", " · in it" } } }
                            span { class: "probbar wide", span { style: "width: {p * 100.0:.1}%; background: hsl({hue(i, n)} 45% 55%);" } }
                            span { class: "p", {pct(p)} }
                        }
                    }
                }
            }
        }
        Some("values") => rsx! {
            div { class: "figures",
                for (k , v) in a["values"].as_object().cloned().unwrap_or_default() {
                    div { key: "{k}", b { {format!("{:.3}", v.as_f64().unwrap_or(0.0))} } "{k}" }
                }
            }
        },
        _ => {
            if let Some(png) = a["image"]["png"].as_str() {
                rsx! { img { class: "spec-img", src: "data:image/png;base64,{png}", alt: "what it drew" } }
            } else {
                let t = a["text"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| a["numbers"].to_string());
                if t.is_empty() {
                    rsx! { div { class: "dim small", "nothing: it ended at once" } }
                } else {
                    rsx! { div { class: "spec-out", "{t}" } }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ writing

#[component]
fn SpecWrite(name: String, enter: Signal<u64>) -> Element {
    let ui = ui();
    let api = api();
    let mut prompt = use_signal(String::new);
    let mut candidates = use_signal(|| 8u32);
    let mut length: Signal<Option<u32>> = use_signal(|| None);
    let mut temperature: Signal<Option<f64>> = use_signal(|| None);
    let mut seed = use_signal(|| 1u64);
    let mut guided = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut got: Signal<Option<Value>> = use_signal(|| None);
    let mut form = use_signal(|| 0u64);
    let write = {
        let (api, n) = (api.clone(), name.clone());
        move || {
            let (api, n) = (api.clone(), n.clone());
            let mut opts = json!({ "candidates": *candidates.peek(), "seed": *seed.peek(), "guided": *guided.peek() });
            if let Some(l) = *length.peek() {
                opts["max_len"] = json!(l);
            }
            if let Some(t) = *temperature.peek() {
                opts["temperature"] = json!(t);
            }
            let p = prompt.peek().clone();
            busy.set(true);
            dioxus::dioxus_core::spawn_forever(async move {
                match api.generate_spec(&n, &p, opts).await {
                    Ok(v) => got.set(Some(v)),
                    Err(e) => ui.fail(e),
                }
                busy.set(false);
            });
        }
    };
    // Enter in the bar: write after the bar's text
    {
        let mut write = write.clone();
        let mut seen = use_signal(|| *enter.peek());
        use_effect(move || {
            let n = enter();
            if n != *seen.peek() {
                seen.set(n);
                prompt.set(ui.bar.peek().trim().to_string());
                form += 1;
                write();
            }
        });
    }
    let write2 = write.clone();
    let res = got();
    let critics: Vec<String> = res
        .as_ref()
        .map(|r| {
            r["critics"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| {
                    let want = &c["want"];
                    let w = want
                        .as_object()
                        .and_then(|o| o.iter().next())
                        .map(|(k, v)| {
                            format!(
                                "{k} {}",
                                v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())
                            )
                        })
                        .unwrap_or_default();
                    format!("{} ({w})", c["model"].as_str().unwrap_or(""))
                })
                .collect()
        })
        .unwrap_or_default();
    rsx! {
        div { class: "card",
            div { class: "row",
                h3 { Icon { name: "edit" } "write" }
                span { class: "dim small", "candidates judged by its critics; the same seed writes the same ones · Enter in the bar writes after its text" }
            }
            for g in [form()] {
                div { key: "{g}", class: "row wrap",
                    input { r#type: "text", style: "flex: 1; min-width: 160px;", placeholder: "a start to go on from (empty: from nothing)",
                        "aria-label": "the start", initial_value: "{prompt}", oninput: move |e| prompt.set(e.value()) }
                    label { class: "row small", "candidates"
                        input { r#type: "number", min: "1", max: "32", style: "width: 56px;", initial_value: "{candidates}",
                            oninput: move |e| if let Ok(v) = e.value().parse::<u32>() { candidates.set(v.clamp(1, 32)) } } }
                    label { class: "row small", "length"
                        input { r#type: "number", min: "1", max: "2000", style: "width: 64px;", placeholder: "spec",
                            oninput: move |e| length.set(e.value().parse::<u32>().ok().map(|v| v.clamp(1, 2000))) } }
                    label { class: "row small", "temperature"
                        input { r#type: "number", min: "0", max: "3", step: "0.1", style: "width: 60px;", placeholder: "spec",
                            oninput: move |e| temperature.set(e.value().parse::<f64>().ok().map(|v| v.clamp(0.0, 3.0))) } }
                    label { class: "row small", "seed"
                        input { r#type: "number", min: "0", style: "width: 64px;", initial_value: "{seed}",
                            oninput: move |e| if let Ok(v) = e.value().parse::<u64>() { seed.set(v) } } }
                    label { class: "row small", title: "steer each byte toward what the critics want (slower)",
                        input { r#type: "checkbox", checked: guided(), onchange: move |e| guided.set(e.checked()) } "guided" }
                    button { class: "primary", disabled: busy(), onclick: { let mut c = write2.clone(); move |_| c() }, Icon { name: "play" } if busy() { "writing…" } else { "write" } }
                }
            }
            if let Some(r) = res {
                div { class: "dim small",
                    {format!("{} candidates in {:.1} s, seed {}", r["candidates"].as_array().map_or(0, |c| c.len()), r["seconds"].as_f64().unwrap_or(0.0), r["seed"])}
                    if !critics.is_empty() { {format!(" · judged by {}", critics.join(", "))} } else { " · no critics in its spec" }
                }
                div { class: "results cands",
                    for c in r["candidates"].as_array().cloned().unwrap_or_default() {
                        Candidate { key: "{c[\"index\"]}", c: c.clone(), critics: critics.clone() }
                    }
                }
            }
        }
    }
}

#[component]
fn Candidate(c: Value, critics: Vec<String>) -> Element {
    let ui = ui();
    let text = c["text"].as_str().unwrap_or("").to_string();
    let kept = c["kept"] == true;
    // the score is a weighted mean log; its exp is a geometric mean probability
    let score = c["score"].as_f64().unwrap_or(0.0).exp();
    let scores: Vec<f64> = c["scores"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    let broken = strs(&c["broken"]);
    let t = text.clone();
    rsx! {
        div { class: if kept { "cand" } else { "cand broken" },
            div { class: "row",
                if !critics.is_empty() {
                    span { class: "probbar", title: "the critics' score", span { style: "width: {score * 100.0:.1}%;" } }
                    span { class: "p small", {pct(score)} }
                }
                span { class: "cand-text", "{text}" }
                button { class: "plain", "aria-label": "into the bar", "data-tip": "into the bar", onclick: move |_| ui.set_bar(&t), Icon { name: "text" } }
            }
            div { class: "row small dim",
                for (i , s) in scores.iter().enumerate() {
                    span { key: "{i}", class: "chip", {format!("{} {}", critics.get(i).map(|c| c.split(' ').next().unwrap_or("")).unwrap_or(""), pct(*s))} }
                }
                span { {format!("log p {:.1}", c["logp"].as_f64().unwrap_or(0.0))} }
                if !broken.is_empty() { span { class: "bad", {format!("left out: {}", broken.join("; "))} } }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_is_a_spec() {
        let lists: Vec<ListInfo> = ["fruit", "rocks \"and\" stones"]
            .iter()
            .map(|n| ListInfo {
                name: n.to_string(),
                count: 3,
                ..Default::default()
            })
            .collect();
        let judge = ("kind".to_string(), "kind".to_string(), "fruit".to_string());
        for t in Template::ALL {
            for (l, j) in [(&lists[..], Some(&judge)), (&[][..], None)] {
                let s = template(t, l, j, &["cats".to_string(), "dogs".to_string()]);
                let spec = anend_core::classify::spec::ModelSpec::parse(&s, true)
                    .unwrap_or_else(|e| panic!("{}: {e:?}\n{s}", t.name()));
                assert!(s.find("\"name\"") < s.find("\"data\""), "{s}");
                assert_eq!(spec.critics.len(), usize::from(t == Template::Generator && j.is_some()));
            }
        }
    }
}
