//! Notes: chronological (newest first) at the left, each with its date; long titles show
//! in full on hover. The editor at the right holds `# title` and markdown, its tags in a
//! box; Ctrl+S (or the save button) keeps it. The layout is a grid, so the editor takes
//! whatever room the window gives it. The bar narrows the list by words.

use dioxus::prelude::*;

use crate::backend::{Note, NoteQuery, NoteSummary};
use crate::ui::*;
use crate::views::use_keys;

fn when(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|t| t.with_timezone(&chrono::Local).format("%d %b %Y, %H:%M").to_string())
        .unwrap_or_default()
}

#[component]
pub fn Notes() -> Element {
    let ui = ui();
    let api = api();
    let mut q = use_signal(String::new);
    let mut open: Signal<Option<Note>> = use_signal(|| None);
    let mut text = use_signal(String::new);
    let mut tags = use_signal(String::new);
    let mut dirty = use_signal(|| false);
    let mut armed = use_signal(|| false);
    let api_r = api.clone();
    let list = use_resource(move || {
        let api = api_r.clone();
        let _ = (ui.changed)();
        let query = NoteQuery {
            q: q(),
            ..Default::default()
        };
        async move { api.notes(&query).await }
    });
    let save = {
        let api = api.clone();
        move || {
            let api = api.clone();
            let mut note = open.peek().clone().unwrap_or_default();
            note.tags = tags
                .peek()
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            let t = text.peek().clone();
            dioxus::dioxus_core::spawn_forever(async move {
                match api.save_note(&note, &t).await {
                    Ok(n) => {
                        open.set(Some(n));
                        dirty.set(false);
                        ui.say("saved");
                        ui.touched();
                    }
                    Err(e) => ui.fail(e),
                }
            });
        }
    };
    let save_k = save.clone();
    use_keys(View::Notes, move |k: KeyPress| {
        if k.bar && k.key == "Enter" {
            q.set(k.text.trim().to_string());
            return true;
        }
        false
    });
    let notes: Vec<NoteSummary> = match &*list.read() {
        Some(Ok(v)) => v.clone(),
        _ => vec![],
    };
    let cur_key = open().map(|n| n.key).unwrap_or_default();
    let shown_title = anend_core::note::split(&text()).0;
    let api_d = api.clone();
    rsx! {
        div { class: "notes", style: "position: absolute; inset: 0;",
            div { class: "card notelist",
                button {
                    class: "primary",
                    onclick: move |_| {
                        open.set(Some(Note::default()));
                        text.set("# \n\n".into());
                        tags.set(String::new());
                        dirty.set(false);
                    },
                    Icon { name: "new" }
                    "a new note"
                }
                ul { role: "listbox", "aria-label": "notes, newest first",
                    if notes.is_empty() {
                        li { class: "dim", "no notes" if !q().is_empty() { " with “{q}”" } }
                    }
                    for n in notes {
                        li {
                            key: "{n.key}",
                            class: if n.key == cur_key { "on" } else { "" },
                            title: "{n.title}",
                            role: "option",
                            tabindex: "0",
                            onclick: {
                                let api = api.clone();
                                let key = n.key.clone();
                                move |_| {
                                    let api = api.clone();
                                    let key = key.clone();
                                    dioxus::dioxus_core::spawn_forever(async move {
                                        match api.note(&key).await {
                                            Ok(note) => {
                                                text.set(anend_core::note::compose(&note.title, &note.markdown));
                                                tags.set(note.tags.join(", "));
                                                open.set(Some(note));
                                                dirty.set(false);
                                                armed.set(false);
                                            }
                                            Err(e) => ui.fail(e),
                                        }
                                    });
                                }
                            },
                            "{n.title}"
                            span { class: "when", "{when(&n.created)}" if !n.tags.is_empty() { {format!(" · {}", n.tags.join(", "))} } }
                        }
                    }
                }
            }
            div { class: "card editor",
                if let Some(note) = open() {
                    div { class: "head",
                        span { class: "title", title: "{shown_title}", "{shown_title}" }
                        label { class: "row", Icon { name: "tag" }
                            input { r#type: "text", placeholder: "tags, comma separated", value: "{tags}", "aria-label": "tags",
                                oninput: move |e| { tags.set(e.value()); dirty.set(true); } }
                        }
                        button { class: if dirty() { "primary" } else { "" }, "data-tip": "save · Ctrl+S", "aria-label": "save",
                            onclick: { let s = save_k.clone(); move |_| s() }, Icon { name: "save" } }
                        if !note.key.is_empty() {
                            button {
                                class: if armed() { "armed" } else { "plain" },
                                "data-tip": "delete (click twice)",
                                "aria-label": "delete",
                                onclick: move |_| {
                                    if !armed() { armed.set(true); return; }
                                    let api = api_d.clone();
                                    let key = note.key.clone();
                                    dioxus::dioxus_core::spawn_forever(async move {
                                        match api.delete_note(&key).await {
                                            Ok(()) => { open.set(None); ui.say("deleted"); ui.touched(); }
                                            Err(e) => ui.fail(e),
                                        }
                                    });
                                },
                                Icon { name: "delete" }
                                if armed() { "delete?" }
                            }
                        }
                    }
                    if !note.created.is_empty() {
                        span { class: "dim small", "written {when(&note.created)}" if !note.edits.is_empty() { " · {note.edits.len()} edits" } }
                    }
                    textarea {
                        "aria-label": "the note: # title, then markdown",
                        spellcheck: "true",
                        value: "{text}",
                        oninput: move |e| { text.set(e.value()); dirty.set(true); },
                        onkeydown: {
                            let s = save.clone();
                            move |e: KeyboardEvent| {
                                if e.modifiers().ctrl() && e.key() == Key::Character("s".into()) {
                                    e.prevent_default();
                                    s();
                                }
                            }
                        },
                    }
                } else {
                    div { class: "empty", "Pick a note, or make a new one. A note is a text whose first line is its # title." }
                }
            }
        }
    }
}
