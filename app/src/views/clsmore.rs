//! More of the classifiers view, as the Bevy app had it (classify_ui.rs, cls_more.rs): the
//! page's tabs (learn, use, buckets, checks); the buckets (every text filed by class and
//! sureness in tenths, a column or a class opening its texts, index again); texts near a
//! text by meaning or by verdict; "why" (each classifier's part in a verdict); the checks
//! (validators: other classifiers that must say a class for a verdict to stand, and the
//! texts they disagree on); the growth it went through.

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::ui::*;
use crate::views::classifiers::{nums, strs};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Learn,
    Use,
    Buckets,
    Checks,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Learn, Tab::Use, Tab::Buckets, Tab::Checks];
    pub fn name(self) -> &'static str {
        match self {
            Tab::Learn => "learn",
            Tab::Use => "use",
            Tab::Buckets => "buckets",
            Tab::Checks => "checks",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Tab::Learn => "sliders",
            Tab::Use => "search",
            Tab::Buckets => "bucket",
            Tab::Checks => "shield",
        }
    }
    pub fn tip(self) -> &'static str {
        match self {
            Tab::Learn => "training, steering and what it was told",
            Tab::Use => "try it, why it says so, search by class, texts near a text",
            Tab::Buckets => "every text filed by class and sureness, and near texts",
            Tab::Checks => "other classifiers that check this one, and where they disagree",
        }
    }
}

#[component]
pub fn Tabs(tab: Signal<Tab>) -> Element {
    let mut tab = tab;
    rsx! {
        div { class: "card cls-tabs", role: "tablist",
            for t in Tab::ALL {
                button {
                    key: "{t.name()}",
                    role: "tab",
                    class: if tab() == t { "plain on" } else { "plain" },
                    "aria-selected": "{tab() == t}",
                    "data-tip": "{t.tip()}",
                    onclick: move |_| tab.set(t),
                    Icon { name: t.icon() }
                    "{t.name()}"
                }
            }
        }
    }
}

/// the growth and training it went through, newest first
#[component]
pub fn Growth(details: Value) -> Element {
    let hist: Vec<Value> = details["history"].as_array().cloned().unwrap_or_default();
    if hist.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "card",
            h3 { Icon { name: "stats" } "growth" span { class: "dim small", " what it went through, newest first" } }
            table { class: "growth",
                thead { tr { th { "when" } th { "what" } th { "accuracy" } th { "F1" } th { "on" } th { "steps" } th { "" } } }
                for (i , e) in hist.iter().rev().take(30).enumerate() {
                    tr { key: "{i}",
                        td { class: "dim", {e["time"].as_str().unwrap_or("").get(..16).unwrap_or("").replace('T', " ")} }
                        td { "{e[\"kind\"].as_str().unwrap_or(\"\")}" }
                        td { {e["metrics"]["accuracy"].as_f64().map(|a| format!("{:.1}%", a * 100.0)).unwrap_or_default()} }
                        td { {e["metrics"]["macro_f1"].as_f64().map(|a| format!("{a:.3}")).unwrap_or_default()} }
                        td { class: "dim", {format!("{} {}", e["examples"].as_u64().unwrap_or(0), e["measured_on"].as_str().unwrap_or(""))} }
                        td { "{e[\"steps\"].as_u64().unwrap_or(0)}" }
                        td { class: "dim small", "{e[\"note\"].as_str().unwrap_or(\"\")}" }
                    }
                }
            }
        }
    }
}

/// each classifier's part in what this one says of a text (`classify.contributions`)
#[component]
pub fn Why(name: String) -> Element {
    let ui = ui();
    let api = api();
    let mut text = use_signal(String::new);
    let mut got: Signal<Option<Value>> = use_signal(|| None);
    let ask = move |_| {
        let (api, n, t) = (api.clone(), name.clone(), text());
        if t.trim().is_empty() {
            ui.say("a text to ask about, please");
            return;
        }
        spawn(async move {
            match api
                .call("classify.contributions", json!({ "name": n, "text": t }))
                .await
            {
                Ok(v) => got.set(Some(v)),
                Err(e) => ui.fail(e),
            }
        });
    };
    let v = got();
    rsx! {
        div { class: "card",
            h3 { Icon { name: "help" } "why" span { class: "dim small", " each classifier's part in a verdict, end to end through the couplings" } }
            div { class: "row",
                input { r#type: "text", style: "flex: 1;", placeholder: "a text", "aria-label": "a text to ask why about", oninput: move |e| text.set(e.value()) }
                button { onclick: ask, "why?" }
            }
            if let Some(v) = v {
                div { class: "dim small", "it says “{v[\"class\"].as_str().unwrap_or(\"\")}”: who had a part in it" }
                div { class: "results",
                    for (i , r) in v["results"].as_array().cloned().unwrap_or_default().into_iter().enumerate() {
                        div { key: "{i}", class: "result",
                            span { class: "p", {format!("{:.0}%", r["share"].as_f64().unwrap_or(0.0) * 100.0)} }
                            span { class: "probbar", span { style: "width: {(r[\"share\"].as_f64().unwrap_or(0.0) * 100.0).clamp(0.0, 100.0):.0}%;" } }
                            span { class: "t", b { "{r[\"name\"].as_str().unwrap_or(\"\")}" } if r["judged"] == true { " (this one)" } " says {r[\"top_class\"].as_str().unwrap_or(\"\")}" }
                            span { class: "dim small", {format!("inputs {:.0}%", r["input_share"].as_f64().unwrap_or(0.0) * 100.0)} }
                        }
                    }
                }
            }
        }
    }
}

/// the texts filed nearest a text, by what the classifier represents them as (meaning) or
/// by how it judges them (verdict)
#[component]
pub fn Near(name: String) -> Element {
    let ui = ui();
    let api = api();
    let mut text = use_signal(String::new);
    let mut by = use_signal(|| "embedding");
    let mut got: Signal<Option<Value>> = use_signal(|| None);
    let ask = move |_| {
        let (api, n, t, b) = (api.clone(), name.clone(), text(), by());
        if t.trim().is_empty() {
            return;
        }
        // a key as it is, else a text
        let args = if anend_core::graph::record::parse_label(t.trim().trim_start_matches("note:")).is_some() {
            json!({ "name": n, "key": t.trim(), "by": b, "limit": 30 })
        } else {
            json!({ "name": n, "text": t, "by": b, "limit": 30 })
        };
        spawn(async move {
            match api.call("classify.near", args).await {
                Ok(v) => got.set(Some(v)),
                Err(e) => ui.fail(e),
            }
        });
    };
    rsx! {
        div { class: "card",
            h3 { Icon { name: "near" } "near texts" span { class: "dim small", " the filed texts closest to a text: by meaning (what it represents it as) or by verdict (how it judges it)" } }
            div { class: "row",
                span { class: "mtabs",
                    button { class: if by() == "embedding" { "mtab on" } else { "mtab" }, onclick: move |_| by.set("embedding"), "meaning" }
                    button { class: if by() == "outputs" { "mtab on" } else { "mtab" }, onclick: move |_| by.set("outputs"), "verdict" }
                }
                input { r#type: "text", style: "flex: 1;", placeholder: "a text, or a key like 0,12", "aria-label": "a text to find near texts of", oninput: move |e| text.set(e.value()) }
                button { onclick: ask, Icon { name: "near" } "find them" }
            }
            if let Some(v) = got() {
                div { class: "dim small", "{v[\"indexed\"]} texts filed" }
                div { class: "results",
                    for (i , r) in v["results"].as_array().cloned().unwrap_or_default().into_iter().enumerate() {
                        div { key: "{i}", class: "result",
                            span { class: "p", {format!("{:.3}", r["cosine"].as_f64().unwrap_or(0.0))} }
                            span { class: "t", title: "{r[\"key\"].as_str().unwrap_or(\"\")}", onclick: { let t = r["text"].as_str().unwrap_or("").to_string(); move |_| ui.set_bar(&t) },
                                if r["note"] == true { "✎ " } "{r[\"text\"].as_str().unwrap_or(\"\")}" }
                        }
                    }
                }
            }
        }
    }
}

/// every text filed by class and sureness, in tenths; a cell opens its texts
#[component]
pub fn Buckets(name: String) -> Element {
    let ui = ui();
    let api = api();
    let mut again = use_signal(|| 0u64);
    let mut open: Signal<Option<(String, Option<u32>)>> = use_signal(|| None);
    let mut job: Signal<Option<Value>> = use_signal(|| None);
    let (n0, api0) = (name.clone(), api.clone());
    let b = use_resource(move || {
        let (api, n) = (api0.clone(), n0.clone());
        let _ = (again(), (ui.changed)());
        async move { api.call("classify.buckets", json!({ "name": n })).await }
    });
    let (n1, api1) = (name.clone(), api.clone());
    let texts = use_resource(move || {
        let (api, n) = (api1.clone(), n1.clone());
        let o = open();
        async move {
            let (c, band) = o?;
            let mut args = json!({ "name": n, "class": c, "limit": 200 });
            if let Some(b) = band {
                args["band"] = json!(b);
            }
            api.call("classify.bucket", args).await.ok()
        }
    });
    let (n2, api2) = (name.clone(), api.clone());
    let index = move |_| {
        let (api, n) = (api2.clone(), n2.clone());
        spawn(async move {
            match api.call("classify.index", json!({ "name": n })).await {
                Ok(j) => {
                    job.set(Some(j));
                    loop {
                        sleep_ms(400).await;
                        match api.call("classify.index_job", json!({ "name": n })).await {
                            Ok(j) => {
                                let running = j["state"] == "running";
                                job.set(Some(j));
                                if !running {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    again += 1;
                    ui.say(format!("{n}: the save is filed again"));
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let v = match &*b.read() {
        Some(Ok(v)) => v.clone(),
        Some(Err(e)) => return rsx! { div { class: "card dim", "{e}" } },
        None => return rsx! { div { class: "card dim", "…" } },
    };
    let palette = strs(&v["palette"]);
    let buckets: Vec<Value> = v["buckets"].as_array().cloned().unwrap_or_default();
    let max = buckets.iter().flat_map(|b| nums(&b["bands"])).fold(1.0f64, f64::max);
    let total: u64 = buckets.iter().map(|b| b["count"].as_u64().unwrap_or(0)).sum();
    let running = job().is_some_and(|j| j["state"] == "running");
    rsx! {
        div { class: "card",
            div { class: "row",
                h3 { Icon { name: "bucket" } "buckets" }
                span { class: "dim small", "{total} of {v[\"indexed\"]} texts filed" }
                span { class: "grow" }
                if running {
                    span { class: "dim small", {format!("filing {} of {}", job().map(|j| j["done"].clone()).unwrap_or_default(), job().map(|j| j["total"].clone()).unwrap_or_default())} }
                }
                button { disabled: running, "data-tip": "file every text again (after training or new texts)", onclick: index, Icon { name: "bolt" } "index again" }
            }
            div { class: "bgrid",
                span { class: "dim small", "class" }
                span {}
                span { class: "dim small bscale", span { "0%" } span { "sureness →" } span { "100%" } }
                for (i , bk) in buckets.iter().enumerate() {
                    {
                        let cname = bk["class"].as_str().unwrap_or("").to_string();
                        let colour = if bk["index"].is_null() { "#8a8580".to_string() } else { palette.get(i).cloned().unwrap_or_default() };
                        let bands = nums(&bk["bands"]);
                        let c2 = cname.clone();
                        rsx! {
                            button { key: "c{i}", class: "plain bclass", onclick: move |_| open.set(Some((c2.clone(), None))),
                                span { class: "dot", style: "background: {colour};" } "{cname}" }
                            span { class: "dim num", "{bk[\"count\"]}" }
                            div { class: "bands",
                                for (j , n) in bands.iter().enumerate() {
                                    {
                                        let c3 = cname.clone();
                                        let h = (n / max * 100.0).clamp(0.0, 100.0);
                                        rsx! {
                                            button { key: "{j}", class: "band", title: "{cname}: {n} texts {j}0–{j + 1}0% sure",
                                                onclick: move |_| open.set(Some((c3.clone(), Some(j as u32)))),
                                                span { style: "height: {h:.0}%; background: {colour};" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "dim small", "each class's texts by how sure it is, in tenths; “uncertain” holds the texts it puts in no class. Click a class or a cell for its texts. What's filed is kept until training, feedback or steering changes it." }
            if let Some((c, band)) = open() {
                div { class: "bucket-texts",
                    div { class: "row",
                        b { "{c}" }
                        if let Some(bd) = band { span { class: "dim small", " {bd}0–{bd + 1}0% sure" } }
                        span { class: "grow" }
                        button { class: "plain ibtn", "aria-label": "close", onclick: move |_| open.set(None), Icon { name: "close" } }
                    }
                    match texts.read().clone().flatten() {
                        Some(t) => rsx! {
                            if t["stale"].as_u64().unwrap_or(0) > 0 { div { class: "dim small", "{t[\"stale\"]} filed before the last change: index again" } }
                            div { class: "results",
                                for (i , h) in t["hits"].as_array().cloned().unwrap_or_default().into_iter().enumerate() {
                                    div { key: "{i}", class: "result",
                                        span { class: "p", {format!("{:.0}%", h["prob"].as_f64().unwrap_or(0.0) * 100.0)} }
                                        span { class: "t", onclick: { let t = h["text"].as_str().unwrap_or("").to_string(); move |_| ui.set_bar(&t) },
                                            if h["note"] == true { "✎ " } "{h[\"text\"].as_str().unwrap_or(\"\")}" }
                                        span { class: "dim small", "{h[\"ordinal\"]}·{h[\"reversed\"]}" }
                                    }
                                }
                            }
                        },
                        None => rsx! { div { class: "dim", "…" } },
                    }
                }
            }
        }
        Near { name: name.clone() }
    }
}

/// the validators (classifiers that check this one) and the texts they disagree on
#[component]
pub fn Checks(name: String, all: Vec<String>) -> Element {
    let ui = ui();
    let api = api();
    let mut again = use_signal(|| 0u64);
    let (n0, api0) = (name.clone(), api.clone());
    let vals = use_resource(move || {
        let (api, n) = (api0.clone(), n0.clone());
        let _ = again();
        async move { api.call("classify.validators", json!({ "name": n })).await }
    });
    let (n1, api1) = (name.clone(), api.clone());
    let dis = use_resource(move || {
        let (api, n) = (api1.clone(), n1.clone());
        let _ = again();
        async move {
            api.call("classify.disagreements", json!({ "name": n, "limit": 40 }))
                .await
        }
    });
    let mut by = use_signal(String::new);
    let mut class = use_signal(String::new);
    let mut min = use_signal(|| 0.5f64);
    let mut mode = use_signal(|| "gate".to_string());
    // the classes of the classifier picked as a validator
    let api_c = api.clone();
    let by_classes = use_resource(move || {
        let api = api_c.clone();
        let b = by();
        async move {
            if b.is_empty() {
                return vec![];
            }
            api.call("classify.get", json!({ "name": b }))
                .await
                .map(|v| strs(&v["classes"]))
                .unwrap_or_default()
        }
    });
    let current: Vec<Value> = match &*vals.read() {
        Some(Ok(v)) => v["validators"].as_array().cloned().unwrap_or_default(),
        _ => vec![],
    };
    let set = {
        let (api, n) = (api.clone(), name.clone());
        move |list: Vec<Value>| {
            let (api, n) = (api.clone(), n.clone());
            spawn(async move {
                match api.call("classify.validators", json!({ "name": n, "set": list })).await {
                    Ok(_) => {
                        again += 1;
                        ui.touched();
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let set2 = set.clone();
    let cur2 = current.clone();
    let others: Vec<String> = all.into_iter().filter(|x| *x != name).collect();
    rsx! {
        div { class: "card",
            h3 { Icon { name: "shield" } "validators" span { class: "dim small", " other classifiers that check this one: a verdict stands when its validator says the class it must" } }
            if current.is_empty() { div { class: "dim", "none yet" } }
            for (i , v) in current.iter().enumerate() {
                div { key: "{i}", class: "row",
                    b { "{v[\"by\"].as_str().unwrap_or(\"\")}" }
                    span { "must say “{v[\"class\"].as_str().unwrap_or(\"\")}” at least {v[\"min\"].as_f64().unwrap_or(0.5) * 100.0:.0}% ({v[\"mode\"].as_str().unwrap_or(\"gate\")})" }
                    button { class: "plain", "data-tip": "take this validator off",
                        onclick: { let s = set.clone(); let cur = current.clone(); move |_| { let mut l = cur.clone(); l.remove(i); s(l); } },
                        Icon { name: "close" } }
                }
            }
            if others.is_empty() {
                div { class: "dim small", "train another classifier to check this one with" }
            } else {
                div { class: "row",
                    select { "aria-label": "the validator", onchange: move |e| { by.set(e.value()); class.set(String::new()); },
                        option { value: "", "a classifier…" }
                        for o in others { option { key: "{o}", value: "{o}", "{o}" } }
                    }
                    select { "aria-label": "the class it must say", onchange: move |e| class.set(e.value()),
                        option { value: "", "the class it must say…" }
                        for c in by_classes.read().clone().unwrap_or_default() { option { key: "{c}", value: "{c}", "{c}" } }
                    }
                    label { class: "row small", "at least" input { r#type: "number", min: "0", max: "1", step: "0.05", value: "{min}", style: "width: 70px;", oninput: move |e| if let Ok(x) = e.value().parse() { min.set(x) } } }
                    select { "aria-label": "how it checks", value: "{mode}", onchange: move |e| mode.set(e.value()),
                        option { value: "gate", "gate: no class unless it passes" }
                        option { value: "weigh", "weigh: multiply by its sureness" }
                    }
                    button { disabled: by().is_empty() || class().is_empty(),
                        onclick: move |_| { let mut l = cur2.clone(); l.push(json!({ "by": by(), "class": class(), "min": min(), "mode": mode() })); set2(l); },
                        Icon { name: "new" } "add it" }
                }
            }
        }
        div { class: "card",
            h3 { Icon { name: "alert" } "disagreements" span { class: "dim small", " texts it puts in a class its validators don't let stand" } }
            match &*dis.read() {
                Some(Ok(d)) => {
                    let rs = d["results"].as_array().cloned().unwrap_or_default();
                    rsx! {
                        if rs.is_empty() { div { class: "dim", "they agree on every text" } }
                        div { class: "results",
                            for (i , r) in rs.into_iter().enumerate() {
                                div { key: "{i}", class: "result",
                                    span { class: "p", {format!("{:.0}%", r["prob"].as_f64().unwrap_or(0.0) * 100.0)} }
                                    span { class: "t", onclick: { let t = r["text"].as_str().unwrap_or("").to_string(); move |_| ui.set_bar(&t) }, "{r[\"text\"].as_str().unwrap_or(\"\")}" }
                                    span { class: "dim small", "“{r[\"class\"].as_str().unwrap_or(\"\")}”, checked by " {r["checks"].as_array().cloned().unwrap_or_default().iter().map(|c| format!("{} {:.0}%", c[0].as_str().unwrap_or(""), c[1].as_f64().unwrap_or(0.0) * 100.0)).collect::<Vec<_>>().join(", ")} }
                                }
                            }
                        }
                    }
                }
                Some(Err(e)) => rsx! { div { class: "dim", "{e}" } },
                None => rsx! { div { class: "dim", "…" } },
            }
        }
    }
}
