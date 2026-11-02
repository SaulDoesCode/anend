//! Taxonomies: the save's classifiers in a tree (docs/architecture.md, part 1;
//! `taxonomy.*` in docs/app-api.md). The models view's third tab.
//!
//! The left column lists them. A taxonomy's page edits its tree (each node a classifier,
//! each of its classes either the end or the next classifier), shows each node's
//! held-out accuracy and its count marked by hand, and has a try card: a text routed down
//! the tree, each step marked right or wrong (wrong asks which class it was), which counts
//! at that node and tells its classifier.

use std::collections::BTreeMap;

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::ui::*;
use crate::views::classifiers::pct;

fn path_of(v: &Value) -> Vec<String> {
    crate::views::classifiers::strs(v)
}

/// the node at `path` in a taxonomy's JSON tree
fn node_mut<'a>(root: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    match path.split_first() {
        None => Some(root),
        Some((c, rest)) => node_mut(root.get_mut("routes")?.get_mut(c)?, rest),
    }
}

/// held out and by hand, for a node's line
fn scores(n: &Value) -> String {
    let mut s = Vec::new();
    match n["held_out"].as_f64() {
        Some(a) => s.push(format!("held out {}", pct(a))),
        None if n["trained"] == false => s.push("not trained yet".into()),
        None => {}
    }
    let (r, w) = (n["right"].as_u64().unwrap_or(0), n["wrong"].as_u64().unwrap_or(0));
    if r + w > 0 {
        s.push(format!("by hand {} ({r} of {})", pct(r as f64 / (r + w) as f64), r + w));
    } else {
        s.push("not marked yet".into());
    }
    s.join(" · ")
}

#[component]
pub fn Taxonomies(bar: Signal<(u64, String)>) -> Element {
    let ui = ui();
    let api = api();
    let mut chosen: Signal<Option<String>> = use_signal(|| None);
    let mut making = use_signal(|| false);
    let mut rev = use_signal(|| 0u64);
    let api_l = api.clone();
    let list = use_resource(move || {
        let api = api_l.clone();
        let _ = ((ui.changed)(), rev());
        async move { api.call("taxonomy.list", json!({})).await }
    });
    let api_m = api.clone();
    let models = use_resource(move || {
        let api = api_m.clone();
        let _ = ((ui.changed)(), rev());
        async move { api.models().await }
    });
    let (items, note): (Vec<Value>, Option<String>) = match &*list.read() {
        Some(Ok(v)) => (v.as_array().cloned().unwrap_or_default(), None),
        Some(Err(e)) => (vec![], Some(e.clone())),
        None => (vec![], None),
    };
    let classifiers: BTreeMap<String, Vec<String>> = match &*models.read() {
        Some(Ok(l)) => l.iter().map(|m| (m.name.clone(), m.classes.clone())).collect(),
        _ => BTreeMap::new(),
    };
    let open = chosen().or_else(|| {
        (!making())
            .then(|| items.first().and_then(|t| t["name"].as_str().map(String::from)))
            .flatten()
    });
    rsx! {
        if let Some(n) = note {
            div { class: "card bad", "{n}" }
        }
        div { class: "cls-layout",
            div { class: "card cls-list", role: "listbox", "aria-label": "taxonomies",
                button { class: if making() { "cls-item on" } else { "cls-item" }, onclick: move |_| making.set(!making()),
                    b { Icon { name: "new" } " new" } }
                if items.is_empty() {
                    span { class: "dim small", "none yet" }
                }
                for t in items.iter().cloned() {
                    {
                        let name = t["name"].as_str().unwrap_or("").to_string();
                        let on = open.as_deref() == Some(name.as_str()) && !making();
                        let (r, w) = (t["right"].as_u64().unwrap_or(0), t["wrong"].as_u64().unwrap_or(0));
                        rsx! {
                            button { key: "{name}", role: "option", "aria-selected": "{on}",
                                class: if on { "cls-item on" } else { "cls-item" },
                                onclick: { let n = name.clone(); move |_| { chosen.set(Some(n.clone())); making.set(false); } },
                                b { "{name}" }
                                span { class: "small", {format!("{} · {} node{}", t["root"].as_str().unwrap_or(""), t["nodes"], if t["nodes"] == 1 { "" } else { "s" })} }
                                span { class: "small state",
                                    if r + w > 0 { {format!("{} right by hand", pct(r as f64 / (r + w) as f64))} } else { "not marked yet" }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "cls-main",
                if making() {
                    NewTaxonomy { classifiers: classifiers.clone(), on_made: move |name: String| { making.set(false); chosen.set(Some(name)); rev += 1; } }
                } else if let Some(name) = open {
                    TaxonomyPage { key: "{name}", name: name.clone(), classifiers: classifiers.clone(), bar,
                        on_saved: move |name: String| { chosen.set(Some(name)); rev += 1; },
                        on_gone: move |_| { chosen.set(None); rev += 1; } }
                } else {
                    div { class: "card empty",
                        p { "A taxonomy puts classifiers in a tree. The first one reads a text; each of its classes can end there or send the text on to a further classifier, so a question like \"English or not?\" decides which classifier reads the text next." }
                        p { "Mark each step of a routing right or wrong as you try texts: each node counts its marks beside its classifier's held-out accuracy, and every mark is that classifier's feedback too. Nothing is stored about the texts themselves." }
                        if classifiers.is_empty() {
                            p { "Make a classifier first (the classifiers tab)." }
                        } else {
                            p { button { onclick: move |_| making.set(true), Icon { name: "new" } "new taxonomy" } }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn NewTaxonomy(classifiers: BTreeMap<String, Vec<String>>, on_made: EventHandler<String>) -> Element {
    let ui = ui();
    let api = api();
    let mut name = use_signal(String::new);
    let first = classifiers.keys().next().cloned().unwrap_or_default();
    let mut root = use_signal(move || first);
    rsx! {
        div { class: "card cls-new",
            h3 { Icon { name: "new" } "a new taxonomy" }
            div { class: "form",
                label { r#for: "tax-name", "name" }
                input { r#type: "text", id: "tax-name", value: "{name}", placeholder: "languages, kinds of thing…", oninput: move |e| name.set(e.value()) }
                label { r#for: "tax-root", "first classifier" }
                select { id: "tax-root", value: "{root}", onchange: move |e| root.set(e.value()),
                    for c in classifiers.keys() {
                        option { key: "{c}", value: "{c}", selected: *c == root(), "{c}" }
                    }
                }
            }
            div { class: "row",
                button { disabled: name().trim().is_empty() || root().is_empty(),
                    onclick: move |_| {
                        let api = api.clone();
                        let t = json!({ "name": name().trim(), "root": { "classifier": root() } });
                        dioxus::dioxus_core::spawn_forever(async move {
                            match api.call("taxonomy.save", json!({ "taxonomy": t, "new": true })).await {
                                Ok(v) => {
                                    ui.say("made: now send its classes on to further classifiers");
                                    on_made.call(v["taxonomy"]["name"].as_str().unwrap_or("").to_string());
                                }
                                Err(e) => ui.fail(e),
                            }
                        });
                    },
                    Icon { name: "save" } "make it" }
            }
        }
    }
}

#[component]
fn TaxonomyPage(
    name: String,
    classifiers: BTreeMap<String, Vec<String>>,
    bar: Signal<(u64, String)>,
    on_saved: EventHandler<String>,
    on_gone: EventHandler<()>,
) -> Element {
    let ui = ui();
    let api = api();
    let mut rev = use_signal(|| 0u64);
    let api_g = api.clone();
    let n_g = name.clone();
    let got = use_resource(move || {
        let (api, n) = (api_g.clone(), n_g.clone());
        let _ = rev();
        async move { api.call("taxonomy.get", json!({ "name": n })).await }
    });
    // the tree as edited, and as saved
    let mut draft: Signal<Option<Value>> = use_signal(|| None);
    let mut saved: Signal<Option<Value>> = use_signal(|| None);
    use_effect(move || {
        if let Some(Ok(v)) = &*got.read() {
            draft.set(Some(v["taxonomy"].clone()));
            saved.set(Some(v["taxonomy"].clone()));
        }
    });
    let mut sure = use_signal(|| false);
    let nodes: Vec<Value> = match &*got.read() {
        Some(Ok(v)) => v["nodes"].as_array().cloned().unwrap_or_default(),
        Some(Err(e)) => return rsx! { div { class: "card bad", "{e}" } },
        None => return rsx! { div { class: "card dim", "…" } },
    };
    let Some(t) = draft() else {
        return rsx! { div { class: "card dim", "…" } };
    };
    let stats: BTreeMap<Vec<String>, Value> = nodes.iter().map(|n| (path_of(&n["path"]), n.clone())).collect();
    let dirty = draft() != saved();
    let save = {
        let api = api.clone();
        let was = name.clone();
        move |_| {
            let (api, was) = (api.clone(), was.clone());
            let Some(t) = draft() else { return };
            dioxus::dioxus_core::spawn_forever(async move {
                match api.call("taxonomy.save", json!({ "taxonomy": t, "was": was })).await {
                    Ok(v) => {
                        ui.say("saved");
                        let n = v["taxonomy"]["name"].as_str().unwrap_or("").to_string();
                        if n == was {
                            rev += 1;
                        }
                        on_saved.call(n);
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let delete = {
        let api = api.clone();
        let n = name.clone();
        move |_| {
            if !sure() {
                sure.set(true);
                return;
            }
            let (api, n) = (api.clone(), n.clone());
            dioxus::dioxus_core::spawn_forever(async move {
                match api.call("taxonomy.delete", json!({ "name": n })).await {
                    Ok(_) => {
                        ui.say(format!("deleted the taxonomy {n} (its classifiers stay)"));
                        on_gone.call(());
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    rsx! {
        div { class: "cls-page",
            div { class: "card cls-head",
                div { class: "row",
                    input { r#type: "text", class: "tax-title", "aria-label": "the taxonomy's name", value: "{t[\"name\"].as_str().unwrap_or(\"\")}",
                        oninput: move |e| { if let Some(d) = draft.write().as_mut() { d["name"] = json!(e.value()); } } }
                    span { class: "grow" }
                    button { disabled: !dirty, class: if dirty { "on" } else { "" }, "data-tip": "save the tree (marks stay where the same classifier stands)", onclick: save,
                        Icon { name: "save" } "save" }
                    if dirty {
                        button { onclick: move |_| draft.set(saved()), Icon { name: "undo" } "as saved" }
                    }
                    button { class: if sure() { "danger" } else { "" }, onclick: delete,
                        Icon { name: "delete" } if sure() { "sure? delete" } else { "delete" } }
                }
                input { r#type: "text", class: "tax-about", placeholder: "what it sorts (optional)", "aria-label": "what it sorts",
                    value: "{t[\"about\"].as_str().unwrap_or(\"\")}",
                    oninput: move |e| { if let Some(d) = draft.write().as_mut() { d["about"] = json!(e.value()); } } }
            }
            div { class: "card",
                h3 { Icon { name: "classifier" } "the tree" }
                p { class: "dim small", "Each classifier's classes end the routing, or send the text on to the next classifier." }
                {node_view(Vec::new(), &t["root"], &classifiers, &stats, draft)}
            }
            TryRoute { name: name.clone(), classifiers: classifiers.clone(), dirty, bar, on_marked: move |_| rev += 1 }
        }
    }
}

/// a node of the tree being edited, with its classes and where each goes
fn node_view(
    path: Vec<String>,
    node: &Value,
    classifiers: &BTreeMap<String, Vec<String>>,
    stats: &BTreeMap<Vec<String>, Value>,
    mut draft: Signal<Option<Value>>,
) -> Element {
    let cls = node["classifier"].as_str().unwrap_or("").to_string();
    let classes = classifiers.get(&cls).cloned().unwrap_or_default();
    let score = stats.get(&path).map(scores).unwrap_or_else(|| "not saved yet".into());
    let missing = !classifiers.contains_key(&cls);
    let p_set = path.clone();
    rsx! {
        div { class: "tax-node",
            div { class: "row",
                select { "aria-label": "the classifier here", value: "{cls}",
                    onchange: move |e| {
                        let v = e.value();
                        if let Some(d) = draft.write().as_mut()
                            && let Some(n) = node_mut(&mut d["root"], &p_set)
                        {
                            n["classifier"] = json!(v);
                            if let Some(r) = n.get_mut("routes").and_then(Value::as_object_mut) {
                                r.clear();
                            }
                        }
                    },
                    if missing {
                        option { value: "{cls}", selected: true, "{cls} (missing)" }
                    }
                    for c in classifiers.keys() {
                        option { key: "{c}", value: "{c}", selected: *c == cls, "{c}" }
                    }
                }
                span { class: "dim small", "{score}" }
            }
            for class in classes.iter().cloned() {
                {
                    let next = node["routes"].get(&class).cloned();
                    let goes = next.as_ref().and_then(|n| n["classifier"].as_str()).unwrap_or("").to_string();
                    let mut sub = path.clone();
                    sub.push(class.clone());
                    let p_route = path.clone();
                    let c_route = class.clone();
                    rsx! {
                        div { key: "{class}", class: "tax-route",
                            div { class: "row",
                                span { class: "tax-class", "{class}" }
                                span { class: "dim", "→" }
                                select { "aria-label": "where {class} goes", value: "{goes}",
                                    onchange: move |e| {
                                        let v = e.value();
                                        if let Some(d) = draft.write().as_mut()
                                            && let Some(n) = node_mut(&mut d["root"], &p_route)
                                        {
                                            if v.is_empty() {
                                                if let Some(r) = n.get_mut("routes").and_then(Value::as_object_mut) {
                                                    r.remove(&c_route);
                                                }
                                            } else {
                                                if !n["routes"].is_object() {
                                                    n["routes"] = json!({});
                                                }
                                                n["routes"][&c_route] = json!({ "classifier": v });
                                            }
                                        }
                                    },
                                    option { value: "", selected: goes.is_empty(), "ends here" }
                                    for c in classifiers.keys() {
                                        option { key: "{c}", value: "{c}", selected: *c == goes, "{c}" }
                                    }
                                }
                            }
                            if let Some(n) = next {
                                div { class: "tax-sub", {node_view(sub, &n, classifiers, stats, draft)} }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// a text routed down the saved tree, each step marked right or wrong
#[component]
fn TryRoute(
    name: String,
    classifiers: BTreeMap<String, Vec<String>>,
    dirty: bool,
    bar: Signal<(u64, String)>,
    on_marked: EventHandler<()>,
) -> Element {
    let ui = ui();
    let api = api();
    let mut text = use_signal(String::new);
    let mut got: Signal<Option<Value>> = use_signal(|| None);
    // the step being told which class it was
    let mut asking: Signal<Option<usize>> = use_signal(|| None);
    let route = {
        let (api, name) = (api.clone(), name.clone());
        move || {
            let (api, name) = (api.clone(), name.clone());
            let t = text.peek().clone();
            if t.trim().is_empty() {
                return;
            }
            dioxus::dioxus_core::spawn_forever(async move {
                match api.call("taxonomy.route", json!({ "name": name, "text": t })).await {
                    Ok(v) => {
                        asking.set(None);
                        got.set(Some(v));
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let mark = {
        let (api, name) = (api.clone(), name.clone());
        let route = route.clone();
        move |path: Value, right: bool, class: Option<String>| {
            let (api, name, route) = (api.clone(), name.clone(), route.clone());
            let mut a = json!({ "name": name, "text": text(), "path": path, "right": right });
            if let Some(c) = &class {
                a["class"] = json!(c);
            }
            dioxus::dioxus_core::spawn_forever(async move {
                match api.call("taxonomy.mark", a).await {
                    Ok(_) => {
                        ui.say(match (right, class) {
                            (true, _) => {
                                "marked right: the node counts it, and its classifier was told yes".to_string()
                            }
                            (false, Some(c)) => format!("marked wrong: it was {c}; its classifier was told so"),
                            (false, None) => "marked wrong: its classifier was told no".to_string(),
                        });
                        ui.touched();
                        on_marked.call(());
                        route();
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    // Enter in the bar routes its text (the first time it changes after this card opened)
    let seen = use_hook(|| bar.peek().0);
    {
        let route = route.clone();
        use_effect(move || {
            let (n, t) = bar();
            if n != seen && !t.is_empty() {
                text.set(t);
                route();
            }
        });
    }
    let steps: Vec<Value> = got().and_then(|v| v["steps"].as_array().cloned()).unwrap_or_default();
    let reached = got().is_some_and(|v| v["reached"] == true);
    rsx! {
        div { class: "card",
            h3 { Icon { name: "search" } "try a text" }
            if dirty {
                p { class: "dim small", "Routing uses the tree as saved." }
            }
            div { class: "row",
                input { r#type: "text", class: "grow", placeholder: "a text to route", "aria-label": "a text to route", value: "{text}",
                    oninput: move |e| text.set(e.value()),
                    onkeydown: { let route = route.clone(); move |e: KeyboardEvent| if e.key() == Key::Enter { route() } } }
                button { onclick: { let route = route.clone(); move |_| route() }, Icon { name: "play" } "route it" }
            }
            for (i , st) in steps.iter().cloned().enumerate() {
                {
                    let cls = st["classifier"].as_str().unwrap_or("").to_string();
                    let class = st["class"].as_str().unwrap_or("—").to_string();
                    let trained = st["trained"] == true;
                    let others: Vec<String> = classifiers.get(&cls).cloned().unwrap_or_default().into_iter().filter(|c| *c != class).collect();
                    let path = st["path"].clone();
                    let (m1, m2, m3) = (mark.clone(), mark.clone(), mark.clone());
                    let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());
                    rsx! {
                        div { key: "{i}", class: "result tax-step",
                            span { class: "num", "{i + 1}" }
                            span { class: "t", b { "{cls}" } " → " if trained { "{class}" } else { span { class: "dim", "not trained yet" } } }
                            span { class: "p", if trained { {pct(st["prob"].as_f64().unwrap_or(0.0))} } }
                            span { class: "dim small", {scores(&st)} }
                            if trained {
                                button { class: "plain", "aria-label": "right: it is {class}", "data-tip": "right", onclick: move |_| m1(p1.clone(), true, None), Icon { name: "check" } }
                                button { class: "plain", "aria-label": "wrong: it isn't {class}", "data-tip": "wrong: which was it?",
                                    onclick: move |_| asking.set(if asking() == Some(i) { None } else { Some(i) }), Icon { name: "close" } }
                            }
                        }
                        if asking() == Some(i) {
                            div { class: "row tax-which",
                                span { class: "dim small", "it was:" }
                                for o in others.iter().cloned() {
                                    button { key: "{o}", class: "chip", onclick: { let (m, p, o) = (m2.clone(), p2.clone(), o.clone()); move |_| m(p.clone(), false, Some(o.clone())) }, "{o}" }
                                }
                                button { class: "plain", onclick: move |_| m3(p3.clone(), false, None), "none of these" }
                            }
                        }
                    }
                }
            }
            if let Some(v) = got() {
                p { class: "small",
                    if reached { {format!("ends in {}", v["class"].as_str().unwrap_or("—"))} } else { "stopped: a classifier on the way isn't trained" }
                }
            }
        }
    }
}
