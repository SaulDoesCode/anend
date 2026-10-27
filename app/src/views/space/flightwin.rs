//! What superflight collected (queued request #9): the texts flown through, in the order
//! met, shown while there are any. They can become one new entry (their words joined), a
//! chain (the first its title, the rest its members), a list (the first its head), or the
//! bar's text; or be cleared. Free texts (not entries of the save) join the new entry
//! and the bar, but a chain or a list takes entries only.

use dioxus::prelude::*;
use serde_json::json;

use super::{Sp, Win};
use crate::ui::*;

#[component]
pub(super) fn Collected(sp: Sp) -> Element {
    let ui = ui();
    let api = api();
    let got = (sp.collected)();
    let texts: Vec<String> = got
        .iter()
        .map(|c| c["text"].as_str().unwrap_or("").to_string())
        .collect();
    let joined = texts.join(" ");
    // entries only (a free text's key isn't an entry's)
    let keys: Vec<String> = got
        .iter()
        .filter_map(|c| c["key"].as_str())
        .filter(|k| anend_core::graph::record::parse_label(k).is_some())
        .map(str::to_string)
        .collect();
    let clear = move || sp.send(json!({ "cmd": "clearcollected" }));
    let make = {
        let api = api.clone();
        move |what: &'static str| {
            let api = api.clone();
            let (joined, keys) = (joined.clone(), keys.clone());
            spawn(async move {
                let r: Result<String, String> = match what {
                    "entry" => api
                        .call("mem.saturate", json!({ "text": joined }))
                        .await
                        .map(|v| format!("an entry: {}", v["key"].as_str().unwrap_or(""))),
                    "chain" if keys.len() >= 2 => api
                        .call("chains.make", json!({ "title": keys[0], "members": keys[1..] }))
                        .await
                        .map(|v| format!("a chain under “{}”", v["title"]["text"].as_str().unwrap_or(""))),
                    "list" if keys.len() >= 2 => {
                        let mut out = Ok(String::new());
                        for m in &keys[1..] {
                            if let Err(e) = api.call("lists.enlist", json!({ "list": keys[0], "member": m })).await {
                                out = Err(e);
                                break;
                            }
                        }
                        out.map(|_| format!("a list of {} under its first", keys.len() - 1))
                    }
                    _ => Err("a chain or a list needs two texts of the save at least".into()),
                };
                match r {
                    Ok(said) => {
                        ui.say(said);
                        sp.send(json!({ "cmd": "clearcollected" }));
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let (m1, m2, m3) = (make.clone(), make.clone(), make.clone());
    let shown = 24;
    rsx! {
        Win {
            id: "collected",
            at: (8.0, 200.0),
            on_close: move |_| clear(),
            title: rsx! {
                span { class: "win-title",
                    Icon { name: "bolt" }
                    "flown through"
                    span { class: "dim small", style: "font-weight: 400;", {format!("{} text{}, in order", got.len(), if got.len() == 1 { "" } else { "s" })} }
                }
            },
            div { class: "wire-targets",
                for (i, t) in texts.iter().take(shown).enumerate() {
                    span { key: "{i}", class: "chip small", "{t}" }
                }
                if texts.len() > shown {
                    span { class: "dim small", {format!("…and {} more", texts.len() - shown)} }
                }
            }
            div { class: "row wrap",
                button { class: "btn small primary", title: "one new entry: their words joined", onclick: move |_| m1("entry"), "an entry" }
                button { class: "btn small", title: "the first its title, the rest its members", onclick: move |_| m2("chain"), "a chain" }
                button { class: "btn small", title: "the first its head, the rest its members", onclick: move |_| m3("list"), "a list" }
                button { class: "btn small", onclick: { let j = texts.join(" "); move |_| ui.set_bar(&j) }, "into the bar" }
                button { class: "btn small", onclick: move |_| clear(), "clear" }
            }
        }
    }
}
