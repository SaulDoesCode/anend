//! The filters view's builder, as the Bevy app's filters_ui.rs had it: include and exclude
//! texts by what classifiers say of them, by words, sums, lists, tags and kind, nested in
//! "all of" and "any of" groups (each condition can be turned round: "not"); run over the
//! classifiers' memoized buckets (`filters.run`), kept as presets that can include each
//! other (`filters.save`), and written out as lists (`filters.enlist`).
//!
//! The expression is core's `filters::Expr` as JSON, edited in place by paths into it.

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::backend::FilterRun;
use crate::ui::*;
use crate::views::classifiers::strs;

/// what the builder offers to add
const ADD: [(&str, &str); 8] = [
    ("class", "a classifier says"),
    ("words", "words"),
    ("sum", "a sum"),
    ("list", "in a list"),
    ("tags", "tags"),
    ("kind", "texts or notes"),
    ("preset", "a kept filter"),
    ("group", "a group"),
];

fn blank(kind: &str) -> Value {
    match kind {
        "class" => json!({ "op": "class", "classifier": "", "class": "" }),
        "words" => json!({ "op": "words", "q": "" }),
        "sum" => json!({ "op": "sum", "bucket": "ordinal" }),
        "list" => json!({ "op": "list", "list": "" }),
        "tags" => json!({ "op": "tags", "tags": [] }),
        "kind" => json!({ "op": "kind", "kind": "memories" }),
        "preset" => json!({ "op": "preset", "name": "" }),
        _ => json!({ "op": "or", "of": [] }),
    }
}

/// the node at a path (indices into `of`)
fn at<'a>(root: &'a mut Value, path: &[usize]) -> Option<&'a mut Value> {
    let mut v = root;
    for &i in path {
        v = v.get_mut("of")?.get_mut(i)?;
    }
    Some(v)
}

/// a condition without its "not"
fn inner(v: &Value) -> (&Value, bool) {
    if v["op"] == "not" { (&v["of"], true) } else { (v, false) }
}

#[derive(Clone, PartialEq)]
struct Ctx {
    classifiers: Vec<(String, Vec<String>)>,
    lists: Vec<(String, String)>,
    presets: Vec<String>,
}

#[component]
pub fn Builder() -> Element {
    let ui = ui();
    let api = api();
    let mut expr = use_signal(|| json!({ "op": "and", "of": [] }));
    let mut name = use_signal(String::new);
    // a pipe's effects and actions on what the filter lets through
    let mut effects: Signal<Vec<Value>> = use_signal(Vec::new);
    let mut actions: Signal<Vec<Value>> = use_signal(Vec::new);
    // the kept pipe being edited, and the kept pipes' revision
    let mut editing: Signal<Option<String>> = use_signal(|| None);
    let mut pipes_rev = use_signal(|| 0u64);
    let mut found: Signal<Option<FilterRun>> = use_signal(|| None);
    let api_c = api.clone();
    let ctx = use_resource(move || {
        let api = api_c.clone();
        let _ = (ui.changed)();
        async move {
            let mut classifiers = Vec::new();
            for c in api
                .call("classify.list", json!({}))
                .await
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
            {
                let n = c["name"].as_str().unwrap_or("").to_string();
                let classes = api
                    .call("classify.get", json!({ "name": n }))
                    .await
                    .map(|d| strs(&d["classes"]))
                    .unwrap_or_default();
                classifiers.push((n, classes));
            }
            let lists = api
                .lists()
                .await
                .map(|l| l.lists.into_iter().map(|x| (x.list, x.name)).collect())
                .unwrap_or_default();
            let presets = api.filters().await.unwrap_or_default();
            (
                Ctx {
                    classifiers,
                    lists,
                    presets: presets.iter().map(|p| p.name.clone()).collect(),
                },
                presets,
            )
        }
    });
    let Some((cx, presets)) = ctx.read().clone() else {
        return rsx! { div { class: "card dim", "…" } };
    };
    use_context_provider(|| cx.clone());
    let api_r = api.clone();
    let run = move |_| {
        let api = api_r.clone();
        let e = prune(&expr());
        if e.is_null() {
            ui.say("add a condition first");
            return;
        }
        spawn(async move {
            match api.run_filter(&e).await {
                Ok(r) => found.set(Some(r)),
                Err(e) => ui.fail(e),
            }
        });
    };
    let api_s = api.clone();
    let save = move |_| {
        let (api, n, e) = (api_s.clone(), name().trim().to_string(), prune(&expr()));
        spawn(async move {
            match api.save_filter(&n, &e, "").await {
                Ok(()) => {
                    ui.say(format!(
                        "kept the filter “{n}”: it can be part of other filters, and written as a list"
                    ));
                    ui.touched();
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let api_p = api.clone();
    let keep_pipe = move |_| {
        let api = api_p.clone();
        let n = name().trim().to_string();
        let e = prune(&expr());
        let pipe = json!({ "name": n, "from": e, "effects": effects(), "actions": actions() });
        let was = editing();
        spawn(async move {
            let mut args = json!({ "pipe": pipe });
            match &was {
                Some(w) => args["was"] = json!(w),
                None => args["new"] = json!(true),
            }
            match api.call("pipes.save", args).await {
                Ok(_) => {
                    ui.say(format!(
                        "kept the pipe “{n}”: turn it on below to see its effects in the memory view and the space"
                    ));
                    if let Some(w) = was.filter(|w| *w != n) {
                        crate::shell::set_pref(ui, |p| {
                            for x in p.pipes.iter_mut() {
                                if *x == w {
                                    *x = n.clone();
                                }
                            }
                        });
                    }
                    editing.set(Some(n));
                    pipes_rev += 1;
                    ui.touched();
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let classifier_names: Vec<String> = cx.classifiers.iter().map(|c| c.0.clone()).collect();
    rsx! {
        div { class: "card",
            h3 { Icon { name: "classifier" } "by what classifiers say, and pipes" span { class: "dim small", " conditions in groups: all of them, or any; each can be turned round. With effects, it's a pipe." } }
            Group { expr, path: vec![], depth: 0 }
            crate::views::pipes::Parts { effects, actions, classifiers: classifier_names }
            div { class: "row",
                button { class: "primary", onclick: run, Icon { name: "play" } "run" }
                input { r#type: "text", placeholder: "a name to keep it", value: "{name}", oninput: move |e| name.set(e.value()) }
                button { disabled: name().trim().is_empty(), "data-tip": "keep the filter, to use in other filters", onclick: save, Icon { name: "save" } "keep the filter" }
                button { disabled: name().trim().is_empty(), "data-tip": "keep the filter with its effects and actions",
                    onclick: keep_pipe, Icon { name: "save" } if editing().is_some() { "save the pipe" } else { "keep as a pipe" } }
                if let Some(w) = editing() {
                    span { class: "dim small", "editing the pipe “{w}”" }
                    button { class: "plain", onclick: move |_| { editing.set(None); effects.set(vec![]); actions.set(vec![]); expr.set(json!({ "op": "and", "of": [] })); name.set(String::new()); },
                        Icon { name: "close" } "new" }
                }
            }
            if let Some(r) = found() {
                div { class: "dim small", "{r.matched} of {r.scanned} texts" }
                div { class: "results",
                    for (i , h) in r.results.iter().cloned().enumerate() {
                        div { key: "{i}", class: "result",
                            span { class: "p", {if h.prob > 0.0 { format!("{:.0}%", h.prob * 100.0) } else { String::new() }} }
                            span { class: "t", title: "{h.text}", onclick: { let t = h.text.clone(); move |_| ui.set_bar(&t) }, if h.note { "✎ " } "{h.text}" }
                        }
                    }
                }
            }
        }
        crate::views::pipes::KeptPipes { rev: pipes_rev, on_open: move |p: Value| {
            let from = p["from"].clone();
            expr.set(if from.is_null() { json!({ "op": "and", "of": [] }) } else if from["op"] == "and" || from["op"] == "or" { from } else { json!({ "op": "and", "of": [from] }) });
            effects.set(p["effects"].as_array().cloned().unwrap_or_default());
            actions.set(p["actions"].as_array().cloned().unwrap_or_default());
            let n = p["name"].as_str().unwrap_or("").to_string();
            name.set(n.clone());
            editing.set(Some(n));
        } }
        if !presets.is_empty() {
            div { class: "card",
                h3 { Icon { name: "save" } "kept filters" }
                for p in presets {
                    Kept { key: "{p.name}", name: p.name.clone(), reads: p.reads.clone(), count: p.count, expr: p.expr.clone(), on_open: { let mut expr = expr; move |e: Value| { expr.set(e); } } }
                }
            }
        }
    }
}

#[component]
fn Kept(name: String, reads: String, count: Option<u64>, expr: Value, on_open: EventHandler<Value>) -> Element {
    let ui = ui();
    let api = api();
    let mut armed = use_signal(|| false);
    let (api1, api2, n1, n2) = (api.clone(), api.clone(), name.clone(), name.clone());
    rsx! {
        div { class: "row kept",
            b { "{name}" }
            span { class: "dim small grow", "{reads}" if let Some(c) = count { " · {c} found last time" } }
            button { class: "plain", "data-tip": "into the builder", onclick: move |_| on_open.call(expr.clone()), Icon { name: "edit" } }
            button { class: "plain", "data-tip": "write its texts as a list",
                onclick: move |_| {
                    let (api, n) = (api1.clone(), n1.clone());
                    spawn(async move {
                        match api.enlist_filter(&n).await {
                            Ok(v) => { ui.say(format!("the list “{}” holds its {} texts", v["name"].as_str().unwrap_or(""), v["members"])); ui.touched(); }
                            Err(e) => ui.fail(e),
                        }
                    });
                },
                Icon { name: "list" } "as a list" }
            button { class: if armed() { "armed" } else { "plain" }, "data-tip": "delete it (click twice)",
                onclick: move |_| {
                    if !armed() { armed.set(true); return; }
                    let (api, n) = (api2.clone(), n2.clone());
                    spawn(async move { match api.delete_filter(&n).await { Ok(()) => ui.touched(), Err(e) => ui.fail(e) } });
                },
                Icon { name: "delete" } }
        }
    }
}

#[component]
fn Group(expr: Signal<Value>, path: Vec<usize>, depth: usize) -> Element {
    let mut expr = expr;
    let node = {
        let mut root = expr();
        at(&mut root, &path).cloned().unwrap_or(Value::Null)
    };
    let (g, not) = inner(&node);
    let op = g["op"].as_str().unwrap_or("and").to_string();
    let items: Vec<Value> = g["of"].as_array().cloned().unwrap_or_default();
    let p1 = path.clone();
    let p2 = path.clone();
    let p3 = path.clone();
    rsx! {
        div { class: if depth == 0 { "fgroup root" } else { "fgroup" },
            div { class: "row",
                span { class: "mtabs",
                    button { class: if op == "and" { "mtab on" } else { "mtab" }, onclick: { let p = p1.clone(); move |_| set_op(expr, &p, "and") }, "all of" }
                    button { class: if op == "or" { "mtab on" } else { "mtab" }, onclick: { let p = p1.clone(); move |_| set_op(expr, &p, "or") }, "any of" }
                }
                if not { span { class: "bad small", "not" } }
                span { class: "grow" }
                select {
                    key: "{items.len()}",
                    "aria-label": "add a condition",
                    value: "",
                    onchange: move |e| {
                        let k = e.value();
                        if k.is_empty() { return; }
                        let mut root = expr();
                        if let Some(n) = at(&mut root, &p2) {
                            let g = if n["op"] == "not" { &mut n["of"] } else { n };
                            if let Some(a) = g["of"].as_array_mut() { a.push(blank(&k)); }
                        }
                        expr.set(root);
                    },
                    option { value: "", "add a condition…" }
                    for (k , label) in ADD { option { key: "{k}", value: "{k}", "{label}" } }
                }
                if depth > 0 {
                    Turn { expr, path: p3.clone() }
                }
            }
            if items.is_empty() { div { class: "dim small", "no conditions yet: every text passes" } }
            for (i , it) in items.iter().enumerate() {
                {
                    let mut p = path.clone();
                    p.push(i);
                    let (c, _) = inner(it);
                    let group = c["op"] == "and" || c["op"] == "or";
                    rsx! {
                        if group {
                            Group { key: "{i}", expr, path: p, depth: depth + 1 }
                        } else {
                            Cond { key: "{i}", expr, path: p }
                        }
                    }
                }
            }
        }
    }
}

/// the expression without empty groups (a group with nothing in it lets everything by)
fn prune(v: &Value) -> Value {
    match v["op"].as_str() {
        Some("and") | Some("or") => {
            let of: Vec<Value> = v["of"]
                .as_array()
                .into_iter()
                .flatten()
                .map(prune)
                .filter(|x| !x.is_null())
                .collect();
            if of.is_empty() {
                Value::Null
            } else {
                json!({ "op": v["op"], "of": of })
            }
        }
        Some("not") => {
            let i = prune(&v["of"]);
            if i.is_null() {
                Value::Null
            } else {
                json!({ "op": "not", "of": i })
            }
        }
        _ => v.clone(),
    }
}

fn set_op(mut expr: Signal<Value>, path: &[usize], op: &str) {
    let mut root = expr();
    if let Some(n) = at(&mut root, path) {
        let g = if n["op"] == "not" { &mut n["of"] } else { n };
        g["op"] = json!(op);
    }
    expr.set(root);
}

/// "not" and remove, for a condition or a nested group
#[component]
fn Turn(expr: Signal<Value>, path: Vec<usize>) -> Element {
    let mut expr = expr;
    let p1 = path.clone();
    let p2 = path.clone();
    let not = {
        let mut root = expr();
        at(&mut root, &path).is_some_and(|n| n["op"] == "not")
    };
    rsx! {
        button { class: if not { "chip on" } else { "chip" }, "data-tip": "turn it round: the texts it doesn't let through",
            onclick: move |_| {
                let mut root = expr();
                if let Some(n) = at(&mut root, &p1) {
                    *n = if n["op"] == "not" { n["of"].clone() } else { json!({ "op": "not", "of": n.clone() }) };
                }
                expr.set(root);
            },
            "not" }
        button { class: "plain ibtn", "aria-label": "remove it",
            onclick: move |_| {
                let mut root = expr();
                if let Some((last, parent)) = p2.split_last()
                    && let Some(pn) = at(&mut root, parent)
                {
                    let g = if pn["op"] == "not" { &mut pn["of"] } else { pn };
                    if let Some(a) = g["of"].as_array_mut() && *last < a.len() { a.remove(*last); }
                }
                expr.set(root);
            },
            Icon { name: "close" } }
    }
}

#[component]
fn Cond(expr: Signal<Value>, path: Vec<usize>) -> Element {
    let cx = use_context::<Ctx>();
    let mut expr = expr;
    let node = {
        let mut root = expr();
        at(&mut root, &path).cloned().unwrap_or(Value::Null)
    };
    let (c, _) = inner(&node);
    let c = c.clone();
    let p = path.clone();
    // writes a field of the condition (inside its "not", if any)
    let set = move |k: &'static str, v: Value| {
        let mut root = expr();
        if let Some(n) = at(&mut root, &p) {
            let c = if n["op"] == "not" { &mut n["of"] } else { n };
            if v.is_null() {
                if let Some(o) = c.as_object_mut() {
                    o.remove(k);
                }
            } else {
                c[k] = v;
            }
        }
        expr.set(root);
    };
    let s = |k: &str| c[k].as_str().unwrap_or("").to_string();
    let op = s("op");
    let classes: Vec<String> = cx
        .classifiers
        .iter()
        .find(|(n, _)| *n == s("classifier"))
        .map(|x| x.1.clone())
        .unwrap_or_default();
    rsx! {
        div { class: "cond row",
            match op.as_str() {
                "class" => rsx! {
                    Icon { name: "classifier" }
                    select { "aria-label": "the classifier", onchange: { let mut set = set.clone(); move |e: FormEvent| { set("classifier", json!(e.value())); set("class", json!("")); } },
                        option { value: "", "a classifier…" }
                        for (n , _) in cx.classifiers.iter() { option { key: "{n}", value: "{n}", selected: *n == s("classifier"), "{n}" } }
                    }
                    "says"
                    select { "aria-label": "the class", onchange: { let mut set = set.clone(); move |e: FormEvent| set("class", json!(e.value())) },
                        option { value: "", "a class…" }
                        for k in classes { option { key: "{k}", value: "{k}", selected: k == s("class"), "{k}" } }
                    }
                    "at least"
                    input { r#type: "number", min: "0", max: "1", step: "0.05", style: "width: 70px;", placeholder: "its own",
                        initial_value: c["min"].as_f64().map(|x| x.to_string()).unwrap_or_default(),
                        oninput: { let mut set = set.clone(); move |e: FormEvent| set("min", e.value().parse::<f64>().map(|x| json!(x)).unwrap_or(Value::Null)) } }
                    label { class: "row small", input { r#type: "checkbox", checked: c["top"] == true, onchange: { let mut set = set.clone(); move |e: FormEvent| set("top", if e.checked() { json!(true) } else { Value::Null }) } } "its top class" }
                },
                "words" => rsx! {
                    Icon { name: "search" } "words"
                    input { r#type: "text", initial_value: "{s(\"q\")}", oninput: { let mut set = set.clone(); move |e: FormEvent| set("q", json!(e.value())) } }
                },
                "sum" => rsx! {
                    Icon { name: "hash" }
                    select { "aria-label": "which sum", onchange: { let mut set = set.clone(); move |e: FormEvent| set("bucket", json!(e.value())) },
                        for b in ["ordinal", "reversed"] { option { key: "{b}", value: "{b}", selected: b == s("bucket"), "{b}" } }
                    }
                    "from" input { r#type: "number", style: "width: 80px;", initial_value: c["from"].as_i64().map(|x| x.to_string()).unwrap_or_default(), oninput: { let mut set = set.clone(); move |e: FormEvent| set("from", e.value().parse::<i64>().map(|x| json!(x)).unwrap_or(Value::Null)) } }
                    "to" input { r#type: "number", style: "width: 80px;", initial_value: c["to"].as_i64().map(|x| x.to_string()).unwrap_or_default(), oninput: { let mut set = set.clone(); move |e: FormEvent| set("to", e.value().parse::<i64>().map(|x| json!(x)).unwrap_or(Value::Null)) } }
                },
                "list" => rsx! {
                    Icon { name: "list" } "in"
                    select { "aria-label": "the list", onchange: { let mut set = set.clone(); move |e: FormEvent| set("list", json!(e.value())) },
                        option { value: "", "a list…" }
                        for (k , n) in cx.lists.iter() { option { key: "{k}", value: "{k}", selected: *k == s("list"), "{n}" } }
                    }
                },
                "tags" => rsx! {
                    Icon { name: "tag" } "tagged"
                    input { r#type: "text", placeholder: "tags, comma separated",
                        initial_value: c["tags"].as_array().map(|a| a.iter().filter_map(|t| t.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
                        oninput: { let mut set = set.clone(); move |e: FormEvent| set("tags", json!(e.value().split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect::<Vec<_>>())) } }
                    label { class: "row small", input { r#type: "checkbox", checked: c["any"] == true, onchange: { let mut set = set.clone(); move |e: FormEvent| set("any", if e.checked() { json!(true) } else { Value::Null }) } } "any of them" }
                },
                "kind" => rsx! {
                    Icon { name: "filter" }
                    select { "aria-label": "texts or notes", onchange: { let mut set = set.clone(); move |e: FormEvent| set("kind", json!(e.value())) },
                        option { value: "memories", selected: s("kind") == "memories", "texts only" }
                        option { value: "notes", selected: s("kind") == "notes", "notes only" }
                    }
                },
                "preset" => rsx! {
                    Icon { name: "save" } "the kept filter"
                    select { "aria-label": "a kept filter", onchange: { let mut set = set.clone(); move |e: FormEvent| set("name", json!(e.value())) },
                        option { value: "", "which…" }
                        for n in cx.presets.iter() { option { key: "{n}", value: "{n}", selected: *n == s("name"), "{n}" } }
                    }
                },
                _ => rsx! { span { class: "dim", "{op}" } },
            }
            span { class: "grow" }
            Turn { expr, path: path.clone() }
        }
    }
}
