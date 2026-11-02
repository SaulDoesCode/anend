//! The settings' hub section (the Bevy app's hub_ui.rs and the JS client's hub pages):
//! connect to an anend hub (its address and an app token from your profile there), browse
//! the saves and classifiers people share, take one in, and share the open save or a
//! classifier. On the desktop a taken save lands in the saves folder and opens; on the web
//! the app's own hub takes it into your workspace (`POST /app/hub/{id}`).

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::hubnet;
use crate::shell::set_pref;
use crate::ui::*;
use crate::views::Fold;

#[derive(Clone, Copy, PartialEq)]
enum What {
    Saves,
    Classifiers,
}

/// the hub's address: the setting, else (on the web) the page's own
fn address(ui: Ui) -> String {
    let a = ui.prefs.peek().hub_url.trim().to_string();
    if !a.is_empty() {
        return a;
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        String::new()
    }
}

#[component]
pub fn Hub() -> Element {
    let ui = ui();
    let api = api();
    let p = ui.prefs.read().clone();
    let mut info: Signal<Option<String>> = use_signal(|| None);
    let mut what = use_signal(|| What::Saves);
    let mut q = use_signal(String::new);
    let mut sort = use_signal(|| "new");
    let mut page = use_signal(|| 0u32);
    let mut found: Signal<Option<Value>> = use_signal(|| None);
    let mut share_name = use_signal(String::new);
    let mut share_desc = use_signal(String::new);
    let mut share_tags = use_signal(String::new);
    let mut share_cls = use_signal(String::new);
    let connect = move |_| {
        let hub = address(ui);
        let tok = ui.prefs.peek().hub_token.clone();
        spawn(async move {
            let url = match hubnet::join(&hub, "/api/hub") {
                Ok(u) => u,
                Err(e) => return ui.fail(e),
            };
            match hubnet::get_json(url, None).await {
                Ok(v) => {
                    let mut line = format!(
                        "{} · {} saves · {} classifiers · {} people",
                        v["name"].as_str().unwrap_or("a hub"),
                        v["saves"],
                        v["classifiers"],
                        v["users"]
                    );
                    if !tok.trim().is_empty()
                        && let Ok(u) = hubnet::join(&hub, "/api/me")
                    {
                        match hubnet::get_json(u, Some(tok)).await {
                            Ok(me) => line.push_str(&format!(" · you're {}", me["username"].as_str().unwrap_or("?"))),
                            Err(e) => line.push_str(&format!(" · the token: {e}")),
                        }
                    }
                    info.set(Some(line));
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let browse = move || {
        let hub = address(ui);
        let path = match what() {
            What::Saves => "/api/saves",
            What::Classifiers => "/api/classifiers",
        };
        let url = hubnet::join(
            &hub,
            &format!(
                "{path}?q={}&sort={}&page={}",
                hubnet::encode(q().trim()),
                sort(),
                page()
            ),
        );
        spawn(async move {
            match url {
                Ok(u) => match hubnet::get_json(u, None).await {
                    Ok(v) => found.set(Some(v)),
                    Err(e) => ui.fail(e),
                },
                Err(e) => ui.fail(e),
            }
        });
    };
    let b1 = browse;
    let api_t = api.clone();
    let take = move |item: Value| {
        let api = api_t.clone();
        let hub = address(ui);
        let kind = what();
        spawn(async move {
            let id = item["id"].as_str().unwrap_or("").to_string();
            if !hubnet::id_ok(&id) {
                return ui.fail("the hub sent an odd id");
            }
            let name = item["name"].as_str().unwrap_or("hub").to_string();
            let r: Result<String, String> = async {
                match kind {
                    What::Saves => take_save(&api, &hub, &id, &name, &item).await,
                    What::Classifiers => {
                        #[cfg(target_arch = "wasm32")]
                        {
                            let u = hubnet::join(&hub, &format!("/app/classifiers/{id}/take"))?;
                            hubnet::post(u, None, Some(("application/json".into(), b"{}".to_vec()))).await?;
                            Ok(format!("took in the classifier {name}"))
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            let u = hubnet::join(&hub, &format!("/classifiers/{id}/download"))?;
                            let bytes = hubnet::get_bytes(u, 64 << 20).await?;
                            api.call(
                                "classify.import",
                                json!({ "data": crate::fonts::base64(&bytes), "name": name }),
                            )
                            .await?;
                            Ok(format!("took in the classifier {name}"))
                        }
                    }
                }
            }
            .await;
            match r {
                Ok(m) => {
                    ui.say(m);
                    ui.touched();
                    ui.refresh(api);
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let api_s = api.clone();
    let share_save = move |_| {
        let api = api_s.clone();
        let hub = address(ui);
        let tok = ui.prefs.peek().hub_token.clone();
        let (n, d, t) = (share_name(), share_desc(), share_tags());
        spawn(async move {
            let r: Result<Value, String> = async {
                let name = if n.trim().is_empty() {
                    ui.hello.peek().save.clone()
                } else {
                    n.trim().to_string()
                };
                let bytes = open_save_bytes(&api, &ui.hello.peek().save).await?;
                let (kind, body) = hubnet::multipart(
                    &[("name", &name), ("description", d.trim()), ("tags", t.trim())],
                    ("file", &format!("{name}.redb"), &bytes),
                );
                hubnet::post(hubnet::join(&hub, "/api/saves")?, Some(tok), Some((kind, body))).await
            }
            .await;
            match r {
                Ok(v) => ui.say(format!(
                    "shared: {} ({})",
                    v["url"].as_str().unwrap_or(""),
                    v["status"].as_str().unwrap_or("")
                )),
                Err(e) => ui.fail(e),
            }
        });
    };
    let api_c = api.clone();
    let share_classifier = move |_| {
        let api = api_c.clone();
        let hub = address(ui);
        let tok = ui.prefs.peek().hub_token.clone();
        let n = share_cls();
        spawn(async move {
            let r: Result<Value, String> = async {
                if n.trim().is_empty() {
                    return Err("pick a classifier to share".to_string());
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (&api, &tok);
                    hubnet::post(
                        hubnet::join(&hub, "/app/classifiers/share")?,
                        None,
                        Some(("application/json".into(), json!({ "name": n }).to_string().into_bytes())),
                    )
                    .await
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let v = api.call("classify.export", json!({ "name": n })).await?;
                    let bytes =
                        crate::fonts::unbase64(v["data"].as_str().unwrap_or("")).ok_or("the export didn't read")?;
                    let (kind, body) = hubnet::multipart(
                        &[("name", &n)],
                        ("file", v["file"].as_str().unwrap_or("c.anendcls"), &bytes),
                    );
                    hubnet::post(hubnet::join(&hub, "/api/classifiers")?, Some(tok), Some((kind, body))).await
                }
            }
            .await;
            match r {
                Ok(v) => ui.say(format!("shared {n}: {}", v["status"].as_str().unwrap_or("done"))),
                Err(e) => ui.fail(e),
            }
        });
    };
    let api_l = api.clone();
    let mine = use_resource(move || {
        let api = api_l.clone();
        async move {
            api.call("classify.list", json!({}))
                .await
                .ok()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|c| c["name"].as_str().map(str::to_string))
                .collect::<Vec<_>>()
        }
    });
    let items: Vec<Value> = found()
        .map(|v| {
            v[if what() == What::Saves { "saves" } else { "classifiers" }]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let more = found().is_some_and(|v| v["more"] == true);
    let take2 = take.clone();
    rsx! {
        Fold { name: "hub", title: "the hub",
            label { r#for: "hub", "its address" }
            div { class: "row",
                input { id: "hub", r#type: "text", style: "flex: 1;", placeholder: if cfg!(target_arch = "wasm32") { "this page's own hub" } else { "http://localhost:8080" }, initial_value: "{p.hub_url}",
                    oninput: move |e| set_pref(ui, |p| p.hub_url = e.value()) }
                button { onclick: connect, Icon { name: "hub" } "connect" }
            }
            label { r#for: "hubtok", "an app token" }
            div { class: "row",
                input { id: "hubtok", r#type: "password", style: "flex: 1;", placeholder: "from your profile on the hub (sharing needs one)", initial_value: "{p.hub_token}",
                    oninput: move |e| set_pref(ui, |p| p.hub_token = e.value()) }
            }
            if let Some(i) = info() { div { class: "wide dim small", "{i}" } }
            div { class: "wide row",
                span { class: "mtabs",
                    button { class: if what() == What::Saves { "mtab on" } else { "mtab" }, onclick: move |_| { what.set(What::Saves); found.set(None); }, "saves" }
                    button { class: if what() == What::Classifiers { "mtab on" } else { "mtab" }, onclick: move |_| { what.set(What::Classifiers); found.set(None); }, "classifiers" }
                }
                input { r#type: "text", style: "flex: 1;", placeholder: "names, tags, authors", oninput: move |e| q.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { page.set(0); b1() } }
                select { "aria-label": "order", onchange: move |e| sort.set(match e.value().as_str() { "stars" => "stars", "downloads" => "downloads", "featured" => "featured", _ => "new" }),
                    for s in ["new", "stars", "downloads", "featured"] { option { key: "{s}", value: "{s}", "{s}" } }
                }
                button { onclick: move |_| { page.set(0); browse() }, Icon { name: "search" } "browse" }
            }
            if found().is_some() {
                div { class: "wide hub-list",
                    if items.is_empty() { div { class: "dim", "nothing shared there yet (or nothing matches)" } }
                    for (i , it) in items.into_iter().enumerate() {
                        div { key: "{i}", class: "row hub-item",
                            div { class: "grow",
                                b { "{it[\"name\"].as_str().unwrap_or(\"\")}" }
                                span { class: "dim small", " by {it[\"author\"].as_str().unwrap_or(\"?\")}" }
                                div { class: "dim small",
                                    {it["description"].as_str().unwrap_or("").chars().take(160).collect::<String>()}
                                    if what() == What::Saves { " · {it[\"memories\"]} texts · {it[\"notes\"]} notes" } else { " · {it[\"classes\"].as_array().map(|a| a.iter().filter_map(|c| c.as_str()).collect::<Vec<_>>().join(\", \")).unwrap_or_default()}" }
                                    " · ★ {it[\"stars\"]} · ↓ {it[\"downloads\"]}"
                                }
                            }
                            button { onclick: { let t = take2.clone(); let it = it.clone(); move |_| t(it.clone()) }, Icon { name: "download" } "take it" }
                        }
                    }
                    div { class: "row",
                        button { disabled: page() == 0, onclick: move |_| { page -= 1; browse() }, Icon { name: "chevron-left" } }
                        span { class: "dim small", "page {page() + 1}" }
                        button { disabled: !more, onclick: move |_| { page += 1; browse() }, Icon { name: "chevron-right" } }
                    }
                }
            }
            label { "share this save" }
            div { class: "row",
                if cfg!(target_arch = "wasm32") {
                    span { class: "dim small", "from the desktop app (the web app's saves stay on this hub; export a bundle to move one)" }
                } else {
                    input { r#type: "text", placeholder: "its name there", oninput: move |e| share_name.set(e.value()) }
                    input { r#type: "text", placeholder: "a line about it", oninput: move |e| share_desc.set(e.value()) }
                    input { r#type: "text", placeholder: "tags, comma separated", oninput: move |e| share_tags.set(e.value()) }
                    button { onclick: share_save, Icon { name: "upload" } "share" }
                }
            }
            label { "share a classifier" }
            div { class: "row",
                select { "aria-label": "the classifier to share", onchange: move |e| share_cls.set(e.value()),
                    option { value: "", "a classifier…" }
                    for n in mine.read().clone().unwrap_or_default() { option { key: "{n}", value: "{n}", "{n}" } }
                }
                button { onclick: share_classifier, Icon { name: "upload" } "share" }
            }
        }
    }
}

/// a hub save into this app: the desktop downloads it into the saves folder and opens it;
/// the web asks its own hub to take it into the workspace
async fn take_save(api: &Api, hub: &str, id: &str, name: &str, item: &Value) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (api, item);
        let u = hubnet::join(hub, &format!("/app/hub/{id}"))?;
        hubnet::post(u, None, None).await?;
        Ok(format!("took {name} into your saves"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let link = item["download_url"]
            .as_str()
            .map(str::to_string)
            .unwrap_or(format!("/saves/{id}/download"));
        let bytes = hubnet::get_bytes(hubnet::join(hub, &link)?, hubnet::MAX_DOWNLOAD).await?;
        if !bytes.starts_with(b"redb\x1A\x0A\xA9\x0D\x0A") {
            return Err("the hub sent something that isn't a save".into());
        }
        let clean: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || " -_.".contains(c) {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('.')
            .trim()
            .chars()
            .take(80)
            .collect();
        let clean = if clean.is_empty() {
            "hub save".to_string()
        } else {
            clean
        };
        let dir = crate::prefs::data_dir().join("saves");
        let mut n = clean.clone();
        let mut i = 2;
        while dir.join(format!("{n}.redb")).exists() {
            n = format!("{clean} {i}");
            i += 1;
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{n}.redb")), &bytes).map_err(|e| format!("couldn't keep it: {e}"))?;
        api.open_save(&n).await.map_err(|e| {
            let _ = std::fs::remove_file(dir.join(format!("{n}.redb")));
            format!("the app can't open it ({e}); deleted it")
        })?;
        Ok(format!("took {name} and opened it"))
    }
}

/// the open save's file, to share
async fn open_save_bytes(api: &Api, name: &str) -> Result<Vec<u8>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = api;
        let path = crate::prefs::data_dir().join("saves").join(format!("{name}.redb"));
        std::fs::read(&path).map_err(|e| format!("couldn't read {}: {e}", path.display()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (api, name);
        Err("sharing a save goes from the desktop app".into())
    }
}
