//! Notes: chronological (newest first) at the left, each with its date; long titles show
//! in full on hover. The editor at the right holds `# title` and markdown, its tags in a
//! box; Ctrl+S (or the save button) keeps it. The layout is a grid, so the editor takes
//! whatever room the window gives it. The bar narrows the list by words.

use dioxus::prelude::*;

use crate::backend::{Note, NoteQuery, NoteSummary};
use crate::keys::Action;
use crate::shell::set_pref;
use crate::ui::*;
use crate::views::use_keys;

fn when(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|t| t.with_timezone(&chrono::Local).format("%d %b %Y, %H:%M").to_string())
        .unwrap_or_default()
}

/// keeps the highlighting layer scrolled with the pad (once per page)
const PAD_JS: &str = r#"
// the colour layer's segments, kept by Rust (`highlight::Layer`): set them all, or
// replace a run of them; a last span keeps an empty last line's height
window.__anendHl = window.__anendHl || {
  mk(html) { const s = document.createElement('span'); s.innerHTML = html; return s; },
  tail(h, end) {
    let t = h.lastElementChild;
    if (!t || !t.classList.contains('hl-end')) { t = document.createElement('span'); t.className = 'hl-end'; h.append(t); }
    t.textContent = end ? ' ' : '';
  },
  set(htmls, end) {
    const h = document.getElementById('note-hl');
    if (!h) return;
    h.replaceChildren(...htmls.map((x) => this.mk(x)));
    this.tail(h, end);
  },
  patch(from, count, htmls, end) {
    const h = document.getElementById('note-hl');
    if (!h) return;
    const kids = h.children;
    const before = kids[from + count] || null;
    for (let i = 0; i < count && kids[from]; i++) kids[from].remove();
    for (const x of htmls) h.insertBefore(this.mk(x), before && before.isConnected ? before : h.lastElementChild);
    this.tail(h, end);
  },
};
if (!window.__anendPad) {
  window.__anendPad = true;
  document.addEventListener('scroll', (e) => {
    const t = e.target;
    if (t && t.id === 'note-pad') {
      const h = document.getElementById('note-hl');
      if (h) { h.scrollTop = t.scrollTop; h.scrollLeft = t.scrollLeft; }
    }
  }, true);
  // the pad's own undo: the desktop's web view has none for a text box, and a
  // replacement put in by a click must come back out with Ctrl+Z too. A pause in typing
  // ends a step; Ctrl+Z undoes, Ctrl+Shift+Z or Ctrl+Y redoes.
  const stacks = new WeakMap();
  const st = (p) => { let s = stacks.get(p); if (!s) { s = { undo: [], redo: [], last: 0 }; stacks.set(p, s); } return s; };
  window.__anendPadSnap = (p, force) => {
    const s = st(p);
    const now = Date.now();
    if (force || now - s.last > 800 || !s.undo.length) s.undo.push({ v: p.value, a: p.selectionStart, b: p.selectionEnd });
    if (s.undo.length > 500) s.undo.shift();
    s.last = force ? 0 : now;
    s.redo = [];
  };
  document.addEventListener('beforeinput', (e) => {
    const p = e.target;
    if (p && p.id === 'note-pad' && !window.__anendPadReplaying) window.__anendPadSnap(p, false);
  }, true);
  document.addEventListener('keydown', (e) => {
    const p = e.target;
    if (!p || p.id !== 'note-pad' || !(e.ctrlKey || e.metaKey) || e.altKey) return;
    const k = e.key.toLowerCase();
    const undo = k === 'z' && !e.shiftKey, redo = (k === 'z' && e.shiftKey) || k === 'y';
    if (!undo && !redo) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    const s = st(p);
    const snap = (undo ? s.undo : s.redo).pop();
    if (!snap) return;
    (undo ? s.redo : s.undo).push({ v: p.value, a: p.selectionStart, b: p.selectionEnd });
    p.value = snap.v;
    p.setSelectionRange(snap.a, snap.b);
    s.last = 0;
    window.__anendPadReplaying = true;
    p.dispatchEvent(new Event('input', { bubbles: true }));
    window.__anendPadReplaying = false;
  }, true);
}
"#;

/// typing in segments (the user's approach for long texts, docs/format.md "Long texts"):
/// each change to the pad goes to Rust as the range it replaced and what went in
/// (UTF-16 units), so a key sends a few characters rather than the whole text. A pad
/// whose last value isn't known (just opened) sends its whole text once.
const PAD_SYNC_JS: &str = r#"
window.__anendPadSend = (m) => dioxus.send(m);
if (!window.__anendPadSync) {
  window.__anendPadSync = true;
  document.addEventListener('input', (e) => {
    const p = e.target;
    if (!p || p.id !== 'note-pad' || !window.__anendPadSend) return;
    const v = p.value, prev = p.__prev;
    p.__prev = v;
    if (typeof prev !== 'string') { window.__anendPadSend({ full: v }); return; }
    const n = Math.min(prev.length, v.length);
    let a = 0;
    while (a < n && prev.charCodeAt(a) === v.charCodeAt(a)) a++;
    let b = 0;
    while (b < n - a && prev.charCodeAt(prev.length - 1 - b) === v.charCodeAt(v.length - 1 - b)) b++;
    window.__anendPadSend({ at: a, del: prev.length - a - b, ins: v.slice(a, v.length - b), len: v.length });
  }, true);
}
await new Promise(() => {});
"#;

/// a change sent by `PAD_SYNC_JS`, applied to the text; false when it doesn't fit (the
/// pad's whole text is asked for then)
fn apply_delta(t: &mut String, m: &serde_json::Value) -> Delta {
    if let Some(full) = m["full"].as_str() {
        *t = full.to_string();
        return Delta::Whole;
    }
    match splice(t, m) {
        Some(e) => Delta::Edit(e),
        None => Delta::Lost,
    }
}

/// the notes view's colour layer: its segments, and whether it shows spelling flags
#[derive(Default)]
struct PadLayer {
    layer: crate::highlight::Layer,
    flagged: bool,
}

/// what a pad change did to the text
enum Delta {
    /// the whole text came
    Whole,
    /// bytes `a..b` were replaced by `ins`
    Edit((usize, usize, String)),
    /// it didn't fit: the pad's whole text is asked for
    Lost,
}

fn splice(t: &mut String, m: &serde_json::Value) -> Option<(usize, usize, String)> {
    let (Some(at), Some(del), Some(ins), Some(len)) = (
        m["at"].as_u64(),
        m["del"].as_u64(),
        m["ins"].as_str(),
        m["len"].as_u64(),
    ) else {
        return None;
    };
    // UTF-16 offsets to byte offsets
    let byte_of = |t: &str, u: usize| -> Option<usize> {
        let mut units = 0;
        for (i, c) in t.char_indices() {
            if units == u {
                return Some(i);
            }
            units += c.len_utf16();
            if units > u {
                return None;
            }
        }
        (units == u).then_some(t.len())
    };
    let a = byte_of(t, at as usize)?;
    let b = byte_of(&t[a..], del as usize).map(|x| a + x)?;
    t.replace_range(a..b, ins);
    // the length check: a pad changed without an input event would drift
    (t.encode_utf16().count() == len as usize).then(|| (a, b, ins.to_string()))
}

/// puts a replacement in the pad as one undo step of its own (Ctrl+Z takes it back);
/// `from`..`to` in UTF-16 units
fn replace_in_pad(from: usize, to: usize, with: &str) {
    let js = format!(
        r#"(() => {{
  const p = document.getElementById('note-pad');
  if (!p) return;
  p.focus();
  if (window.__anendPadSnap) window.__anendPadSnap(p, true);
  p.setRangeText({with}, {from}, {to}, 'end');
  window.__anendPadReplaying = true;
  p.dispatchEvent(new Event('input', {{ bubbles: true }}));
  window.__anendPadReplaying = false;
}})();"#,
        with = serde_json::to_string(with).unwrap_or_default()
    );
    document::eval(&js);
}

/// a flagged word the pointer clicked, and where
#[derive(Clone, PartialEq)]
struct Pick {
    flag: serde_json::Value,
    x: f64,
    y: f64,
}

/// a note longer than this (bytes) is typed without the spelling check (which reads the
/// whole text), unless the save's `limits.grammar_check_bytes` says otherwise; the
/// colours stay on at any length, as the layer is coloured in segments
const BIG: usize = 200_000;

fn split_tags(t: &str) -> Vec<String> {
    t.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

#[component]
pub fn Notes() -> Element {
    let ui = ui();
    let api = api();
    let mut q = use_signal(String::new);
    let mut open: Signal<Option<Note>> = use_signal(|| None);
    let mut text = use_signal(String::new);
    let mut tags: Signal<Vec<String>> = use_signal(Vec::new);
    let mut dirty = use_signal(|| false);
    let mut armed = use_signal(|| false);
    let mut seq = use_signal(|| 0u64);
    let mut reading = use_signal(|| false);
    let mut judging = use_signal(|| false);
    let mut tag_gen = use_signal(|| 0u64);
    let mut tag_draft = use_signal(String::new);
    // the spelling check: the flags for the text they were made for
    let mut flags: Signal<(String, Vec<serde_json::Value>)> = use_signal(|| (String::new(), vec![]));
    let mut check_gen = use_signal(|| 0u64);
    let mut report: Signal<Option<serde_json::Value>> = use_signal(|| None);
    let mut pick: Signal<Option<Pick>> = use_signal(|| None);
    // how long a note the spelling check reads (the save's limit)
    let mut big_at = use_signal(|| BIG);
    {
        let api = api.clone();
        use_hook(move || {
            spawn(async move {
                if let Ok(v) = api.call("limits.list", serde_json::json!({})).await {
                    let n = v
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|l| l["name"] == "limits.grammar_check_bytes")
                        .and_then(|l| l["value"].as_u64());
                    if let Some(n) = n {
                        big_at.set(n as usize);
                    }
                }
            });
        });
    }
    // the colour layer, kept in typing segments (`highlight::Layer`); `layer_gen` asks for
    // it whole (a note opened, a text put in, the colours or the flags changed)
    let layer: std::rc::Rc<std::cell::RefCell<PadLayer>> = use_hook(Default::default);
    let mut layer_gen = use_signal(|| 0u64);
    use_hook(|| {
        document::eval(PAD_JS);
    });
    // the pad's changes, as deltas (`PAD_SYNC_JS`)
    let layer_d = layer.clone();
    use_hook(move || {
        let layer = layer_d.clone();
        spawn(async move {
            let mut ev = document::eval(PAD_SYNC_JS);
            while let Ok(m) = ev.recv::<serde_json::Value>().await {
                let d = text.with_mut(|t| apply_delta(t, &m));
                match d {
                    Delta::Lost => {
                        // out of step: the whole text once, then deltas again
                        document::eval(
                            "const p = document.getElementById('note-pad'); if (p) { p.__prev = undefined; p.dispatchEvent(new Event('input', { bubbles: true })); }",
                        );
                        continue;
                    }
                    Delta::Whole => layer_gen += 1,
                    Delta::Edit((a, b, ins)) => {
                        // only the segments around the edit are coloured again; a layer
                        // showing spelling flags (now stale) is made again whole
                        let colour = ui.prefs.peek().highlight;
                        let patched = {
                            let mut l = layer.borrow_mut();
                            if l.flagged {
                                None
                            } else {
                                l.layer.edit(a, b, &ins, colour)
                            }
                        };
                        match patched {
                            Some((from, went, html)) => {
                                let end = text.peek().ends_with('\n') || text.peek().is_empty();
                                document::eval(&format!(
                                    "window.__anendHl && window.__anendHl.patch({from}, {went}, {}, {end});",
                                    serde_json::to_string(&html).unwrap_or_default()
                                ));
                            }
                            None => layer_gen += 1,
                        }
                    }
                }
                dirty.set(true);
                pick.set(None);
            }
        });
    });
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
            note.tags = tags.peek().clone();
            let t = text.peek().clone();
            if anend_core::note::split(&t).0.trim().is_empty() {
                ui.fail("a note needs a '# title' line before it can be saved");
                return;
            }
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
    let mut new_note = move || {
        open.set(Some(Note::default()));
        text.set("# \n\n".into());
        seq += 1;
        tags.set(Vec::new());
        dirty.set(false);
        reading.set(false);
        document::eval(
            "setTimeout(() => { const p = document.getElementById('note-pad'); if (p) { p.focus(); p.setSelectionRange(2, 2); } }, 30);",
        );
    };
    let save_k = save.clone();
    use_keys(View::Notes, move |k: KeyPress| {
        if k.bar && k.key == "Enter" {
            let t = k.text.trim().to_string();
            // Enter again on the same words opens the first note found
            if t == *q.peek()
                && let Some(first) = list
                    .peek()
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .and_then(|v| v.first().cloned())
            {
                let mut o = ui.open_note;
                o.set(Some(first.key));
                document::eval("document.activeElement && document.activeElement.blur()");
                return true;
            }
            q.set(t);
            return true;
        }
        match k.action {
            Some(Action::Save) => save_k(),
            Some(Action::NewNote) => new_note(),
            Some(Action::ToggleRead) => {
                reading.set(!reading());
                seq += 1;
            }
            Some(Action::ToggleList) => set_pref(ui, |p| p.hide_list = !p.hide_list),
            Some(Action::NoteClassifiers) => judging.set(!judging()),
            _ => return false,
        }
        true
    });
    // leaving the view keeps a titled note's changes, as the Bevy app did
    {
        let save = save.clone();
        use_effect(move || {
            let v = (ui.view)();
            if v != View::Notes
                && *dirty.peek()
                && open.peek().is_some()
                && !anend_core::note::split(&text.peek()).0.trim().is_empty()
            {
                save();
            }
        });
    }
    // closing the window keeps a titled note's changes too (desktop)
    {
        let api = api.clone();
        use_effect(move || {
            if (ui.closing)() != 1 {
                return;
            }
            let t = text.peek().clone();
            if !*dirty.peek() || open.peek().is_none() || anend_core::note::split(&t).0.trim().is_empty() {
                return;
            }
            let mut c = ui.closing;
            c.set(2);
            let mut note = open.peek().clone().unwrap_or_default();
            note.tags = tags.peek().clone();
            let api = api.clone();
            dioxus::dioxus_core::spawn_forever(async move {
                let _ = api.save_note(&note, &t).await;
                #[cfg(not(target_arch = "wasm32"))]
                std::process::exit(0);
            });
        });
    }
    // a note another view asked to open (the reader's edit)
    {
        let api = api.clone();
        use_effect(move || {
            let Some(key) = (ui.open_note)() else { return };
            let mut asked = ui.open_note;
            asked.set(None);
            let api = api.clone();
            spawn(async move {
                match api.note(&key).await {
                    Ok(note) => {
                        text.set(
                            note.source
                                .clone()
                                .unwrap_or_else(|| anend_core::note::compose(&note.title, &note.markdown)),
                        );
                        tags.set(note.tags.clone());
                        open.set(Some(note));
                        reading.set(false);
                        seq += 1;
                        dirty.set(false);
                    }
                    Err(e) => ui.fail(e),
                }
            });
        });
    }
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
                format!("{n} with “{words}” · Enter again to narrow")
            },
        );
    });
    // checked a beat after typing stops
    {
        let api = api.clone();
        use_effect(move || {
            let on = ui.prefs.read().check_spelling;
            if !on || text.read().len() > big_at() {
                return;
            }
            let t = text();
            if t.trim().is_empty() {
                return;
            }
            let g = *check_gen.peek() + 1;
            check_gen.set(g);
            let key = open.peek().as_ref().map(|n| n.key.clone()).unwrap_or_default();
            let api = api.clone();
            spawn(async move {
                crate::ui::sleep_ms(700).await;
                if *check_gen.peek() != g {
                    return;
                }
                match api
                    .call("grammar.check", serde_json::json!({ "text": t, "exclude": key }))
                    .await
                {
                    Ok(r) => {
                        if *check_gen.peek() == g {
                            let fl = r["flags"].as_array().cloned().unwrap_or_default();
                            flags.set((t, fl));
                            report.set(Some(r["report"].clone()));
                        }
                    }
                    // a hub from before the check: say nothing, show nothing
                    Err(_) => flags.set((String::new(), vec![])),
                }
            });
        });
    }
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
        // only read mode shows it: no copy of a long text on every key
        _ if reading() => text(),
        _ => String::new(),
    };
    let shown_title = anend_core::note::split(&text.read()).0;
    let (hide_list, list_right, colour) = {
        let p = ui.prefs.read();
        (p.hide_list, p.list_right, p.highlight)
    };
    // with no note open the list is all there is: it shows whatever was chosen (the choice
    // to slide it away holds while a note is open)
    let hide_list = hide_list && open().is_some();
    let check = ui.prefs.read().check_spelling;
    let shown_flags: Vec<crate::highlight::Flag> = {
        let (t, fl) = &*flags.read();
        if check && *t == *text.read() {
            fl.iter()
                .map(|f| {
                    let sug: Vec<String> = f["suggestions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|s| match s["text"].as_str().unwrap_or("") {
                            "" => "take it out".to_string(),
                            x => x.to_string(),
                        })
                        .collect();
                    crate::highlight::Flag {
                        start: f["start"].as_u64().unwrap_or(0) as usize,
                        end: f["end"].as_u64().unwrap_or(0) as usize,
                        kind: if f["kind"] == "spelling" { "spell" } else { "grammar" },
                        title: format!("{} · click: {}", f["kind"].as_str().unwrap_or(""), sug.join(", ")),
                    }
                })
                .collect()
        } else {
            vec![]
        }
    };
    // the colour layer, kept in typing segments by `layer` (made whole here when asked;
    // each typed change patches only its segments, in the pad's delta loop). The spelling
    // check still pauses on long notes.
    let big = text.read().len() > big_at();
    let layered = colour || !shown_flags.is_empty();
    {
        let layer = layer.clone();
        let flags_now = shown_flags.clone();
        use_effect(use_reactive!(|layered, colour, flags_now| {
            let _ = layer_gen();
            let mut l = layer.borrow_mut();
            if !layered {
                l.layer = Default::default();
                l.flagged = false;
                document::eval("window.__anendHl && window.__anendHl.set([], false);");
                return;
            }
            let t = text.peek();
            let html = l.layer.full(&t, colour, &flags_now);
            l.flagged = !flags_now.is_empty();
            let end = t.ends_with('\n') || t.is_empty();
            document::eval(&format!(
                "window.__anendHl && window.__anendHl.set({}, {end});",
                serde_json::to_string(&html).unwrap_or_default()
            ));
        }));
    }
    let api_d = api.clone();
    let mut class = String::from("notes");
    if hide_list {
        class.push_str(" no-list");
    }
    if list_right {
        class.push_str(" list-right");
    }
    let unsaved = dirty();
    rsx! {
        div { class: "{class}",
            // slid away, the list waits off the edge and slides in while the pointer is
            // near that edge (or on the list), as the Bevy app's `hide_list`
            if hide_list {
                div { class: "list-edge", "aria-hidden": "true" }
            }
            div { class: "card notelist",
                button { class: "primary", "data-tip": "start a new note · {ui.key_of(Action::NewNote)}", onclick: move |_| new_note(),
                    Icon { name: "new" } "a new note" }
                ul { role: "listbox", "aria-label": "notes, newest first",
                    if open().is_some_and(|n| n.key.is_empty()) {
                        li { class: "on", role: "option", {if shown_title.trim().is_empty() { "untitled".to_string() } else { shown_title.clone() }} if unsaved { " •" } span { class: "when", "not saved yet" } }
                    }
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
                                                tags.set(note.tags.clone());
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
                            if n.key == cur_key { "{shown_title}" if unsaved { " •" } } else { "{n.title}" }
                            span { class: "when", "{when(&n.created)}" if !n.tags.is_empty() { {format!(" · {}", n.tags.join(", "))} } }
                        }
                    }
                }
            }
            div { class: "card editor",
                if let Some(note) = open() {
                    div { class: "head",
                        button { class: "plain", "data-tip": "slide the note list away / back · {ui.key_of(Action::ToggleList)}", "aria-label": "the note list",
                            onclick: move |_| set_pref(ui, |p| p.hide_list = !p.hide_list), Icon { name: "sidebar" } }
                        span { class: "title", title: "{shown_title}", "{shown_title}" if unsaved { span { class: "dim", " •" } } }
                        div { class: "tagchips",
                            for t in tags() {
                                button { key: "{t}", class: "chip", "data-tip": "take the tag off",
                                    onclick: { let t = t.clone(); move |_| { tags.write().retain(|x| *x != t); dirty.set(true); } },
                                    "#{t}" span { class: "x", "×" } }
                            }
                            for g in [tag_gen()] {
                                input { key: "{g}", class: "taginput", r#type: "text", placeholder: "a tag", "aria-label": "add a tag (Enter or a comma)",
                                    oninput: move |e| {
                                        let v = e.value();
                                        if v.contains(',') {
                                            for t in split_tags(&v) { if !tags.peek().contains(&t) { tags.write().push(t); } }
                                            dirty.set(true);
                                            tag_draft.set(String::new());
                                            tag_gen += 1;
                                        } else {
                                            tag_draft.set(v);
                                        }
                                    },
                                    onkeydown: move |e: KeyboardEvent| {
                                        if e.key() == Key::Enter {
                                            e.prevent_default();
                                            for t in split_tags(&tag_draft.peek()) { if !tags.peek().contains(&t) { tags.write().push(t); dirty.set(true); } }
                                            tag_draft.set(String::new());
                                            tag_gen += 1;
                                            document::eval("setTimeout(() => { const i = document.querySelector('.taginput'); if (i) i.focus(); }, 20);");
                                        }
                                    },
                                }
                            }
                        }
                        button {
                            class: if check { "plain on" } else { "plain" },
                            "data-tip": if check { "the spelling check is on: click an underlined word for what it may be" } else { "check spelling and word choice" },
                            "aria-label": "check spelling",
                            "aria-pressed": "{check}",
                            onclick: move |_| set_pref(ui, |p| p.check_spelling = !p.check_spelling),
                            Icon { name: "check" }
                        }
                        button {
                            class: if judging() { "plain on" } else { "plain" },
                            "data-tip": "the classifiers this note is measured on · {ui.key_of(Action::NoteClassifiers)}",
                            "aria-label": "the note's classifiers",
                            onclick: move |_| judging.set(!judging()),
                            Icon { name: "classifier" }
                        }
                        button {
                            class: "plain",
                            "data-tip": if reading() { format!("edit it · {}", ui.key_of(Action::ToggleRead)) } else { format!("read it · {}", ui.key_of(Action::ToggleRead)) },
                            "aria-label": if reading() { "edit" } else { "read" },
                            onclick: move |_| { reading.set(!reading()); seq += 1; },
                            Icon { name: if reading() { "edit" } else { "view" } }
                        }
                        button { class: if dirty() { "primary" } else { "" }, "data-tip": "save · {ui.key_of(Action::Save)}", "aria-label": "save",
                            onclick: { let s = save.clone(); move |_| s() }, Icon { name: "save" } }
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
                    if judging() {
                        if cur_key.is_empty() {
                            div { class: "judge dim small", "save the note first: then pick classifiers to measure it on" }
                        } else {
                            crate::views::judge::Judge { entry: cur_key.clone() }
                        }
                    }
                    if reading() {
                        div { class: "md rendered", dangerous_inner_html: crate::md::to_html(&read_text) }
                    }
                    if !reading() {
                        Replace { text, dirty, seq }
                    }
                    for pad in [format!("{}-{}", cur_key, seq())].into_iter().filter(|_| !reading()) {
                        div { key: "{pad}", class: if layered { "pad hl-on" } else { "pad" },
                            // the colours, painted under the textarea's transparent glyphs
                            // its segments are put in by `layer` (not by the render)
                            pre { id: "note-hl", class: "hl", "aria-hidden": "true",
                                onmounted: move |_| layer_gen += 1,
                            }
                            // not bound to the signal while typing (a round trip per key would
                            // drop letters typed fast); its text is put in when a note opens
                            textarea {
                                id: "note-pad",
                                "aria-label": "the note: # title, then markdown",
                                spellcheck: "false",
                                onmounted: move |_| {
                                    let js = format!(
                                        "const p = document.getElementById('note-pad'); if (p) {{ p.value = {}; p.__prev = p.value; }}",
                                        serde_json::to_string(&*text.peek()).unwrap_or_default()
                                    );
                                    document::eval(&js);
                                },
                                onclick: move |e: MouseEvent| {
                                    if !ui.prefs.peek().check_spelling || flags.peek().1.is_empty() {
                                        return;
                                    }
                                    let c = e.client_coordinates();
                                    spawn(async move {
                                        let mut ev = document::eval("const p = document.getElementById('note-pad'); dioxus.send(p ? p.selectionStart : -1);");
                                        let Ok(at) = ev.recv::<i64>().await else { return };
                                        let (t, fl) = flags.peek().clone();
                                        if t != *text.peek() || at < 0 {
                                            return;
                                        }
                                        let at = at as u64;
                                        let hit = fl.into_iter().find(|f| {
                                            f["from"].as_u64().unwrap_or(0) <= at && at <= f["to"].as_u64().unwrap_or(0)
                                        });
                                        pick.set(hit.map(|flag| Pick { flag, x: c.x, y: c.y }));
                                    });
                                },
                            }
                        }
                        if let Some(pk) = pick() {
                            SpellPick {
                                pick: pk,
                                on_close: move |_| pick.set(None),
                            }
                        }
                        if big {
                            div { class: "spell-note dim small", {format!("a long note ({} KB): the spelling check pauses above {} KB, so typing stays quick", text.read().len() / 1000, big_at() / 1000)} }
                        }
                        if check && !big {
                            if let Some(r) = report() {
                                div { class: "spell-note dim small",
                                    title: "measured on a fifth of its corpus held out, with errors put in the same way it was trained: real typing errors are harder",
                                    {format!(
                                        "spelling check: finds {:.0}% of the errors put into held-out text, {:.0}% of what it flags is one, the first suggestion is right {:.0}% of the time; {:.1} false alarms per 1000 clean words",
                                        r["recall"].as_f64().unwrap_or(0.0) * 100.0,
                                        r["precision"].as_f64().unwrap_or(0.0) * 100.0,
                                        r["top1"].as_f64().unwrap_or(0.0) * 100.0,
                                        r["false_alarms_per_1000"].as_f64().unwrap_or(0.0),
                                    )}
                                }
                            }
                        }
                    }
                } else {
                    div { class: "empty",
                        p { "Pick a note in the list, or make a new one ({ui.key_of(Action::NewNote)}). A note is a text whose first line is its # title." }
                    }
                }
            }
        }
    }
}

/// what a flagged word may be: each suggestion (a click puts it in; Ctrl+Z takes it back)
#[component]
fn SpellPick(pick: Pick, on_close: EventHandler<()>) -> Element {
    let f = pick.flag.clone();
    let from = f["from"].as_u64().unwrap_or(0) as usize;
    let to = f["to"].as_u64().unwrap_or(0) as usize;
    let kind = f["kind"].as_str().unwrap_or("").to_string();
    let p = f["p"].as_f64().unwrap_or(0.0) * 100.0;
    let sugs: Vec<(String, f64)> = f["suggestions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            (
                s["text"].as_str().unwrap_or("").to_string(),
                s["p"].as_f64().unwrap_or(0.0),
            )
        })
        .collect();
    let what = match kind.as_str() {
        "repeated" => "a word twice",
        "confused" => "another word may fit",
        _ => "not a word it knows",
    };
    rsx! {
        div { class: "spell-pick", style: "left: {pick.x:.0}px; top: {pick.y + 14.0:.0}px;", role: "menu",
            div { class: "dim small", "{what} · {p:.0}% sure" }
            if sugs.is_empty() {
                div { class: "dim small", "no suggestion" }
            }
            for (s , sp) in sugs {
                button {
                    key: "{s}",
                    class: "plain",
                    role: "menuitem",
                    onclick: {
                        let s = s.clone();
                        move |_| {
                            replace_in_pad(from, to, &s);
                            on_close.call(());
                        }
                    },
                    if s.is_empty() { i { "take it out" } } else { b { "{s}" } }
                    span { class: "dim small", " {sp * 100.0:.0}%" }
                }
            }
            button { class: "plain dim small", onclick: move |_| on_close.call(()), "leave it" }
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
            "const p = document.getElementById('note-pad'); if (p) {{ p.value = {}; p.__prev = undefined; p.dispatchEvent(new Event('input', {{ bubbles: true }})); p.focus(); }}",
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
