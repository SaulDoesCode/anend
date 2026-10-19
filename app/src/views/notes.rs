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
    let mut seq = use_signal(|| 0u64);
    let mut reading = use_signal(|| false);
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
    // the bar's right end: how many notes there are, or match
    use_effect(move || {
        let n = list
            .read()
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|v| v.len())
            .unwrap_or(0);
        let words = q();
        ui.side_note(
            View::Notes,
            if words.is_empty() {
                format!("{n} note{}", if n == 1 { "" } else { "s" })
            } else {
                format!("{n} with “{words}”")
            },
        );
    });
    let notes: Vec<NoteSummary> = match &*list.read() {
        Some(Ok(v)) => v.clone(),
        _ => vec![],
    };
    let cur_key = open().map(|n| n.key).unwrap_or_default();
    // read mode shows the note as saved (its replacements and views put in) until it's
    // edited
    let read_text = match open() {
        Some(n) if !dirty() && !n.key.is_empty() => {
            anend_core::note::compose(&n.title, n.rendered.as_deref().unwrap_or(&n.markdown))
        }
        _ => text(),
    };
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
                            seq += 1;
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
                                                    // with replacements, the note as written
                                                    text.set(note.source.clone().unwrap_or_else(|| anend_core::note::compose(&note.title, &note.markdown)));
                                                    tags.set(note.tags.join(", "));
                                                    open.set(Some(note));
                                                    seq += 1;
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
    for g in [format!("{}-{}", cur_key, seq())] {
                                input { key: "{g}", r#type: "text", placeholder: "tags, comma separated", initial_value: "{tags}", "aria-label": "tags",
                                    oninput: move |e| { tags.set(e.value()); dirty.set(true); } }
    }
                            }
                            button {
                            class: "plain",
                            "data-tip": if reading() { "edit it" } else { "read it" },
                            "aria-label": if reading() { "edit" } else { "read" },
                            onclick: move |_| { reading.set(!reading()); seq += 1; },
                            Icon { name: if reading() { "edit" } else { "view" } }
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
                        // not bound to the signal while typing (a round trip per key would drop
                        // letters typed fast); its text is put in when a note opens
                        if reading() {
                        div { class: "md rendered", dangerous_inner_html: crate::md::to_html(&read_text) }
                    }
                    if !reading() {
                        Replace { text, dirty, seq }
                    }
                    for pad in [format!("{}-{}", cur_key, seq())].into_iter().filter(|_| !reading()) {
                            textarea {
                                key: "{pad}",
                                id: "note-pad",
                                "aria-label": "the note: # title, then markdown",
                                spellcheck: "true",
                                onmounted: move |_| {
                                    let js = format!(
                                        "const p = document.getElementById('note-pad'); if (p) {{ p.value = {}; }}",
                                        serde_json::to_string(&*text.peek()).unwrap_or_default()
                                    );
                                    document::eval(&js);
                                },
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
                        }
                    } else {
                        div { class: "empty", "Pick a note, or make a new one. A note is a text whose first line is its # title." }
                    }
                }
            }
        }
}

/// the text before the selection, the word, the text after, and the word's alternatives
type Around = (String, String, String, Vec<serde_json::Value>);

/// the editor's replacements: `{{key}}` puts an entry's text in (live: it shows that
/// entry as it is now), `{{filter:name}}` a saved filter's results; a word's synonyms and
/// registers come from the lists it is in, those keeping the text's sum first
#[component]
fn Replace(text: Signal<String>, dirty: Signal<bool>, seq: Signal<u64>) -> Element {
    let ui = ui();
    let api = api();
    // (the text before the selection, the word, the text after) and its alternatives
    let mut alts: Signal<Option<Around>> = use_signal(|| None);
    let mut find = use_signal(String::new);
    let mut find_gen = use_signal(|| 0u64);
    // puts a new text into the pad and the signal
    let mut put = move |t: String| {
        text.set(t.clone());
        dirty.set(true);
        let js = format!(
            "const p = document.getElementById('note-pad'); if (p) {{ p.value = {}; p.focus(); }}",
            serde_json::to_string(&t).unwrap_or_default()
        );
        document::eval(&js);
    };
    // the pad's text around its selection (the word under the caret when nothing is selected)
    const AROUND: &str = r#"
        const p = document.getElementById('note-pad');
        if (!p) { dioxus.send(null); } else {
          let a = p.selectionStart, b = p.selectionEnd, v = p.value;
          if (a === b && WORD) {
            while (a > 0 && /[\p{L}\p{N}'-]/u.test(v[a - 1])) a--;
            while (b < v.length && /[\p{L}\p{N}'-]/u.test(v[b])) b++;
          }
          dioxus.send([v.slice(0, a), v.slice(a, b), v.slice(b)]);
        }"#;
    let api_a = api.clone();
    let synonyms = move |_| {
        let api = api_a.clone();
        spawn(async move {
            let mut ev = document::eval(&AROUND.replace("WORD", "true"));
            let Ok(Some((before, word, after))) = ev.recv::<Option<(String, String, String)>>().await else {
                return;
            };
            if word.trim().is_empty() {
                ui.say("select a word (or put the caret in one) first");
                return;
            }
            match api.alternatives(word.trim()).await {
                Ok(v) => {
                    let list: Vec<serde_json::Value> = v["alternatives"].as_array().cloned().unwrap_or_default();
                    if list.is_empty() {
                        ui.say(format!(
                            "“{}” is in no list: make one of words that may stand for each other",
                            word.trim()
                        ));
                        alts.set(None);
                    } else {
                        alts.set(Some((before, word, after, list)));
                    }
                }
                Err(e) => ui.fail(e),
            }
        });
    };
    let api_f = api.clone();
    let insert = move || {
        let api = api_f.clone();
        let want = find.peek().trim().to_string();
        if want.is_empty() {
            return;
        }
        spawn(async move {
            // a key as it is, a filter by its name, else the first text with those words
            let piece = if want.starts_with("filter:") || anend_core::graph::record::parse_label(&want).is_some() {
                format!("{{{{{want}}}}}")
            } else {
                match api.search(&want, false).await {
                    Ok(hits) if !hits.is_empty() => format!("{{{{{}}}}}", hits[0].key.trim_start_matches("note:")),
                    Ok(_) => {
                        ui.fail(format!("no text with “{want}”"));
                        return;
                    }
                    Err(e) => {
                        ui.fail(e);
                        return;
                    }
                }
            };
            let mut ev = document::eval(&AROUND.replace("WORD", "false"));
            let Ok(Some((before, sel, after))) = ev.recv::<Option<(String, String, String)>>().await else {
                return;
            };
            let _ = sel;
            put(format!("{before}{piece}{after}"));
            find.set(String::new());
            find_gen += 1;
            ui.say(format!(
                "{piece} goes in where the caret was: it shows that entry's text when read"
            ));
        });
    };
    let insert2 = insert.clone();
    let _ = seq;
    rsx! {
        div { class: "replace row",
            button { class: "plain", "data-tip": "a word's synonyms and registers: the other words of the lists it is in", onclick: synonyms,
                Icon { name: "swap" } "alternatives" }
            if let Some((before, word, after, list)) = alts() {
                select {
                    "aria-label": "replace “{word}” with",
                    onchange: move |e| {
                        let v = e.value();
                        if !v.is_empty() {
                            put(format!("{before}{v}{after}"));
                            ui.say(format!("“{}” → “{v}”", word.trim()));
                        }
                        alts.set(None);
                    },
                    option { value: "", {format!("“{}” (o {} r {}) →", word.trim(), crate::gem::sums(word.trim()).0, crate::gem::sums(word.trim()).1)} }
                    for a in list.iter() {
                        option { key: "{a[\"key\"]}", value: "{a[\"text\"].as_str().unwrap_or(\"\")}",
                            {format!("{}{} · {} · o {} r {}",
                                if a["same_ordinal"].as_bool() == Some(true) { "= " } else { "" },
                                a["text"].as_str().unwrap_or(""),
                                a["list"].as_str().unwrap_or(""),
                                a["ordinal"], a["reversed"])}
                        }
                    }
                }
            }
            span { class: "grow" }
            for g in [find_gen()] {
                input { key: "{g}", r#type: "text", placeholder: "put in {{{{key}}}}: a key, words, or filter:name",
                    "aria-label": "an entry to put in where the caret is",
                    oninput: move |e| find.set(e.value()),
                    onkeydown: { let insert = insert.clone(); move |e: KeyboardEvent| if e.key() == Key::Enter { insert() } },
                }
            }
            button { onclick: move |_| insert2(), "put in" }
        }
    }
}
