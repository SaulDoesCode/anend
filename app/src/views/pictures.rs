//! The pictures panel (docs/app-api.md, `pictures.*`): the save's pictures as
//! thumbnails, added by dropping files on the panel or picking them, put into lists
//! (by name, beside the texts of the list with that head) and taken out, deleted with
//! two clicks. In the lists view, for all the save's pictures (with a picker of the
//! picture lists) and on a list's page for that list's (what is added there goes into
//! it). Spec models train on them (`{"from": "pictures", "list": ..}`).

use dioxus::html::HasFileData;
use dioxus::prelude::*;
use serde_json::Value;

use crate::ui::*;

/// thumbnails' side in pixels
const THUMB: u32 = 128;

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

#[component]
pub fn Pictures(
    /// the list whose pictures these are (None: all the save's)
    list: Option<String>,
    /// list names to offer when putting a picture into a list
    names: Vec<String>,
) -> Element {
    let ui = ui();
    let api = api();
    let mut rev = use_signal(|| 0u64);
    // the picture list shown when this is the save's panel ("" for all)
    let mut showing = use_signal(String::new);
    let mut over = use_signal(|| false);
    let mut busy = use_signal(|| 0usize);
    let fixed = list.clone();
    let api_r = api.clone();
    let pics = use_resource(move || {
        let api = api_r.clone();
        let l = fixed.clone().or_else(|| Some(showing()).filter(|s| !s.is_empty()));
        let _ = ((ui.changed)(), rev());
        async move { api.pictures(l.as_deref(), THUMB).await }
    });
    let api_l = api.clone();
    let plists = use_resource(move || {
        let api = api_l.clone();
        let _ = ((ui.changed)(), rev());
        async move { api.picture_lists().await }
    });
    // files in: each read and sent; the panel's list gets them too
    let into = list.clone().or_else(|| Some(showing()).filter(|s| !s.is_empty()));
    let take = {
        let api = api.clone();
        let into = into.clone();
        move |files: Vec<dioxus::html::FileData>| {
            let api = api.clone();
            let into = into.clone();
            async move {
                let n = files.len();
                if n == 0 {
                    return;
                }
                busy += n;
                let (mut added, mut had) = (0, 0);
                for f in files {
                    let name = f.name();
                    match f.read_bytes().await {
                        Ok(b) => match api.add_picture(&b, &name, into.as_deref()).await {
                            Ok(v) if v.get("new").and_then(Value::as_bool) == Some(true) => added += 1,
                            Ok(_) => had += 1,
                            Err(e) => ui.fail(format!("{name}: {e}")),
                        },
                        Err(e) => ui.fail(format!("couldn't read {name}: {e}")),
                    }
                    busy -= 1;
                }
                match (added, had) {
                    (0, 0) => {}
                    (a, 0) => ui.say(format!("{a} picture{} added", if a == 1 { "" } else { "s" })),
                    (a, h) => ui.say(format!("{a} added, {h} the save had already")),
                }
                rev += 1;
            }
        }
    };
    let take_drop = take.clone();
    // on the desktop a file's path works too (where the system's file dialog doesn't)
    let on_disk = api.on_disk();
    let mut path = use_signal(String::new);
    // bumped to empty the path field (it isn't bound to the signal, so fast typing keeps
    // every letter)
    let mut path_gen = use_signal(|| 0u64);
    let add_path = {
        let api = api.clone();
        let into = into.clone();
        move || {
            let p = path.peek().trim().to_string();
            if p.is_empty() {
                return;
            }
            let (api, into) = (api.clone(), into.clone());
            spawn(async move {
                match api.add_picture_path(&p, into.as_deref()).await {
                    Ok(v) => {
                        ui.say(if v.get("new").and_then(Value::as_bool) == Some(true) {
                            "the picture is in"
                        } else {
                            "the save had that picture already"
                        });
                        path.set(String::new());
                        path_gen += 1;
                        rev += 1;
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let add_path2 = add_path.clone();
    let (shown, note): (Vec<Value>, Option<String>) = match &*pics.read() {
        Some(Ok(v)) => (v.clone(), None),
        Some(Err(e)) => (vec![], Some(e.clone())),
        None => (vec![], None),
    };
    let lists: Vec<(String, u64)> = match &*plists.read() {
        Some(Ok(v)) => v
            .iter()
            .map(|l| {
                (
                    str_of(l, "list").to_string(),
                    l.get("count").and_then(Value::as_u64).unwrap_or(0),
                )
            })
            .collect(),
        _ => vec![],
    };
    let mut offer: Vec<String> = names.clone();
    for (l, _) in &lists {
        if !offer.contains(l) {
            offer.push(l.clone());
        }
    }
    offer.sort_by_key(|n| n.to_lowercase());
    let dl = format!("pic-lists-{}", list.as_deref().unwrap_or("all").replace(' ', "-"));
    rsx! {
        div {
            class: if over() { "card pictures over" } else { "card pictures" },
            ondragover: move |e: DragEvent| { e.prevent_default(); over.set(true); },
            ondragleave: move |_| over.set(false),
            ondrop: move |e: DragEvent| {
                e.prevent_default();
                over.set(false);
                let files = e.files();
                let take = take_drop.clone();
                spawn(async move { take(files).await });
            },
            div { class: "row",
                h3 { Icon { name: "image" } "pictures" }
                span { class: "dim small",
                    match &list {
                        Some(l) => format!("{} in “{l}” · drop PNG or JPEG files here, or pick them", shown.len()),
                        None => format!("{} shown · drop PNG or JPEG files here, or pick them; spec models learn from them", shown.len()),
                    }
                }
                span { class: "grow" }
                if busy() > 0 { span { class: "dim small", "adding {busy()}…" } }
                label { class: "button", "data-tip": "PNG or JPEG files",
                    Icon { name: "upload" } "add pictures"
                    input { r#type: "file", accept: "image/png,image/jpeg", multiple: true, style: "display: none;",
                        "aria-label": "pictures to add",
                        onchange: move |e: FormEvent| {
                            let take = take.clone();
                            async move { take(e.files()).await }
                        },
                    }
                }
            }
            if on_disk {
                div { class: "row",
                    for g in [path_gen()] {
                        input { key: "{g}", r#type: "text", style: "flex: 1;", placeholder: "or a picture's path: /home/you/Pictures/cat.png",
                            "aria-label": "a picture's path",
                            oninput: move |e| path.set(e.value()),
                            onkeydown: { let add_path = add_path.clone(); move |e: KeyboardEvent| if e.key() == Key::Enter { add_path() } },
                        }
                    }
                    button { onclick: move |_| add_path2(), "add" }
                }
            }
            if list.is_none() && !lists.is_empty() {
                div { class: "row chips", role: "tablist", "aria-label": "picture lists",
                    button { class: if showing().is_empty() { "chip on" } else { "chip" }, onclick: move |_| showing.set(String::new()), "all" }
                    for (l , n) in lists.iter().cloned() {
                        button { key: "{l}", class: if showing() == l { "chip on" } else { "chip" },
                            onclick: { let l = l.clone(); move |_| showing.set(l.clone()) },
                            "{l} " span { class: "dim", "{n}" } }
                    }
                }
            }
            if let Some(n) = note { div { class: "bad small", "{n}" } }
            datalist { id: "{dl}",
                for n in offer.iter() { option { key: "{n}", value: "{n}" } }
            }
            if shown.is_empty() {
                div { class: "pic-drop dim", "no pictures yet: drop some here" }
            }
            div { class: "pic-grid",
                for p in shown.into_iter() {
                    Thumb { key: "{str_of(&p, \"id\")}", pic: p.clone(), datalist: dl.clone(), on_change: move |_| rev += 1 }
                }
            }
        }
    }
}

#[component]
fn Thumb(pic: Value, datalist: String, on_change: EventHandler<()>) -> Element {
    let ui = ui();
    let api = api();
    let mut armed = use_signal(|| false);
    let mut into = use_signal(String::new);
    let mut into_gen = use_signal(|| 0u64);
    let id = str_of(&pic, "id").to_string();
    let name = str_of(&pic, "name").to_string();
    let thumb = str_of(&pic, "thumb").to_string();
    let (w, h) = (
        pic.get("width").and_then(Value::as_u64).unwrap_or(0),
        pic.get("height").and_then(Value::as_u64).unwrap_or(0),
    );
    let kb = pic.get("size").and_then(Value::as_u64).unwrap_or(0).div_ceil(1024);
    let lists: Vec<String> = crate::views::classifiers::strs(pic.get("lists").unwrap_or(&Value::Null));
    let shown_name = if name.is_empty() {
        id[..8.min(id.len())].to_string()
    } else {
        name.clone()
    };
    let tag = {
        let (api, id) = (api.clone(), id.clone());
        move |list: String, remove: bool| {
            let (api, id) = (api.clone(), id.clone());
            spawn(async move {
                match api.tag_picture(&id, &list, remove).await {
                    Ok(_) => {
                        ui.say(if remove {
                            format!("out of “{list}”")
                        } else {
                            format!("in “{list}” now")
                        });
                        on_change.call(());
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let tag2 = tag.clone();
    let api_d = api.clone();
    let id_d = id.clone();
    rsx! {
        div { class: "pic", title: "{shown_name} · {w}×{h} · {kb} KB",
            img { src: "data:image/png;base64,{thumb}", alt: "{shown_name}" }
            div { class: "pic-name small", "{shown_name}" }
            div { class: "pic-lists",
                for l in lists.iter().cloned() {
                    span { key: "{l}", class: "chip small",
                        "{l}"
                        button { class: "plain", "aria-label": "take it out of {l}", "data-tip": "take it out of the list",
                            onclick: { let tag = tag.clone(); let l = l.clone(); move |_| tag(l.clone(), true) },
                            Icon { name: "close" } }
                    }
                }
            }
            div { class: "row pic-tools",
                for g in [into_gen()] {
                    input { key: "{g}", r#type: "text", list: "{datalist}", placeholder: "into a list…", "aria-label": "put it into a list",
                        oninput: move |e| into.set(e.value()),
                        onkeydown: { let tag2 = tag2.clone(); move |e: KeyboardEvent| {
                            if e.key() == Key::Enter {
                                let l = into().trim().to_string();
                                if !l.is_empty() { tag2(l, false); into.set(String::new()); into_gen += 1; }
                            }
                        } },
                    }
                }
                button {
                    class: if armed() { "armed" } else { "plain" },
                    "aria-label": "delete the picture",
                    "data-tip": "delete the picture; click twice",
                    onclick: move |_| {
                        if !armed() { armed.set(true); return; }
                        let (api, id) = (api_d.clone(), id_d.clone());
                        spawn(async move {
                            match api.delete_picture(&id).await {
                                Ok(()) => { ui.say("the picture is gone"); on_change.call(()); }
                                Err(e) => ui.fail(e),
                            }
                        });
                    },
                    Icon { name: "delete" }
                }
            }
        }
    }
}
