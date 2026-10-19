//! The space: the three.js room (app/src/space.js, over the vendored three.js in app/vendor)
//! hosted through Dioxus's JS interop. Rust searches and gathers what each text is
//! (`space.features`: its sums, length, kind, word types, lists, time; the colouring
//! classifier's class), space.js runs the free spread and draws, and they talk over one
//! eval channel.
//!
//! As the Bevy app's space had it: the terminal (the command bar: Enter, Ctrl+Enter,
//! Shift+Enter, Delete, ↑ ↓ history, `#123` `#r123` `#name`, the words Enlist List Delist
//! Unlist Count Backup Clear ClearAll Settle Maps `Save name` Help); texts dragged (then
//! pinned), pushed and pulled with the wheel, pinned (U), new (N, a double click on empty
//! room), edited (Enter), removed (Delete), copied (Ctrl+C), undone (Ctrl+Z); maps saved
//! (Shift+X) and exported (X); the physics panel (O) with pause (P) and settle (K); the
//! position card (I), help (H), and the details of the text under the pointer in a window
//! that moves (M), its close button showing on hover.

use std::collections::{BTreeMap, HashMap};

use dioxus::prelude::*;
use futures_util::{StreamExt, future::Either};
use serde_json::{Value, json};

use crate::backend::Hit;
use crate::gem::{self, Mode};
use crate::keys::Action;
use crate::shell::set_pref;
use crate::ui::*;
use crate::views::{TChip, use_keys};

const THREE: &str = include_str!("../../vendor/three.module.min.js");
const ENGINE: &str = include_str!("../space.js");

/// a physics slider: its key (space.js `PHYSICS`), label, range, step, default and group
pub struct Slider {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub def: f64,
    pub group: &'static str,
}

const fn sl(
    key: &'static str,
    label: &'static str,
    min: f64,
    max: f64,
    step: f64,
    def: f64,
    group: &'static str,
) -> Slider {
    Slider {
        key,
        label,
        min,
        max,
        step,
        def,
        group,
    }
}

/// the physics panel; a weight runs from −1 (texts alike that way push apart) to +1 (they
/// pull together)
pub const SLIDERS: [Slider; 24] = [
    sl(
        "w_ordinal",
        "ordinal sums",
        -1.0,
        1.0,
        0.05,
        0.6,
        "what pulls texts together",
    ),
    sl(
        "w_reversed",
        "reversed sums",
        -1.0,
        1.0,
        0.05,
        0.3,
        "what pulls texts together",
    ),
    sl(
        "w_bucket",
        "the same bucket",
        -1.0,
        1.0,
        0.05,
        0.8,
        "what pulls texts together",
    ),
    sl("w_length", "length", -1.0, 1.0, 0.05, 0.2, "what pulls texts together"),
    sl(
        "w_types",
        "kind and word types",
        -1.0,
        1.0,
        0.05,
        0.3,
        "what pulls texts together",
    ),
    sl(
        "w_lists",
        "shared lists",
        -1.0,
        1.0,
        0.05,
        0.7,
        "what pulls texts together",
    ),
    sl(
        "w_class",
        "the colouring classifier's class",
        -1.0,
        1.0,
        0.05,
        0.5,
        "what pulls texts together",
    ),
    sl(
        "w_time",
        "written near in time",
        -1.0,
        1.0,
        0.05,
        0.2,
        "what pulls texts together",
    ),
    sl(
        "w_letters",
        "the same letters",
        -1.0,
        1.0,
        0.05,
        0.4,
        "what pulls texts together",
    ),
    sl(
        "qscale",
        "how near sums count as alike",
        1.0,
        100.0,
        1.0,
        12.0,
        "what pulls texts together",
    ),
    sl("attract", "attraction", 0.0, 4.0, 0.05, 0.9, "motion"),
    sl("repel", "repulsion", 0.0, 60.0, 0.5, 9.0, "motion"),
    sl("link", "distance between alike texts", 0.5, 20.0, 0.1, 4.0, "motion"),
    sl("gravity", "gravity to the centre", 0.0, 1.0, 0.005, 0.06, "motion"),
    sl("pad", "gap between texts", 0.0, 2.0, 0.05, 0.3, "motion"),
    sl("damping", "damping", 0.0, 10.0, 0.1, 2.2, "motion"),
    sl("cool", "cooling", 0.001, 0.08, 0.001, 0.012, "motion"),
    sl("substeps", "speed (steps per frame)", 0.0, 8.0, 1.0, 2.0, "motion"),
    sl("depth", "depth by the measure", 0.0, 20.0, 0.25, 0.0, "depth"),
    sl("flat", "flatness", 0.0, 5.0, 0.05, 1.5, "depth"),
    sl("distance", "spread distance", 5.0, 200.0, 1.0, 26.0, "depth"),
    sl("size", "text size", 0.1, 3.0, 0.05, 0.5, "texts"),
    sl("wrap", "text wrap (characters)", 16.0, 160.0, 1.0, 60.0, "texts"),
    sl("minPx", "hide texts under px", 0.0, 8.0, 0.1, 1.2, "texts"),
];

pub const DEPTH_BY: [&str; 4] = ["none", "ordinal", "length", "time"];

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct SpaceEvent {
    kind: String,
    #[serde(default)]
    key: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    note: bool,
    #[serde(default)]
    free: bool,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    shift: bool,
    #[serde(default)]
    alt: bool,
    #[serde(default)]
    q: f64,
    #[serde(default)]
    hue: f64,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    data: Value,
}

fn boot_js() -> String {
    format!(
        r#"
const THREE_SRC = {three};
const ENGINE_SRC = {engine};
const url = (src) => URL.createObjectURL(new Blob([src], {{ type: 'text/javascript' }}));
const tu = url(THREE_SRC);
const eu = url(ENGINE_SRC.split("'__THREE__'").join(JSON.stringify(tu)));
let host = null;
for (let i = 0; i < 100 && !host; i++) {{ host = document.getElementById('space-host'); if (!host) await new Promise((r) => setTimeout(r, 20)); }}
try {{
  const mod = await import(eu);
  const sp = new mod.Space(host, (m) => dioxus.send(m));
  window.__anendSpace = sp;
  dioxus.send({{ kind: 'ready' }});
  while (true) {{
    const c = await dioxus.recv();
    if (c.cmd === 'place') sp.place(c.texts, c.again);
    else if (c.cmd === 'clear') sp.clear(!!c.all);
    else if (c.cmd === 'room') sp.setRoom(c.bg, c.ink);
    else if (c.cmd === 'start') sp.start();
    else if (c.cmd === 'stop') sp.stop();
    else if (c.cmd === 'mark') sp.mark(c.list, c.candidate);
    else if (c.cmd === 'focus') host.focus();
    else if (c.cmd === 'keys') sp.setKeys(c.map);
    else if (c.cmd === 'physics') sp.setPhysics(c.p);
    else if (c.cmd === 'pause') sp.paused = !!c.on;
    else if (c.cmd === 'settle') dioxus.send({{ kind: 'settled', data: sp.settle() }});
    else if (c.cmd === 'undo') dioxus.send({{ kind: 'undone', data: sp.undoLast() }});
    else if (c.cmd === 'pin') dioxus.send({{ kind: 'toast', text: sp.pinSelected() || 'select a text first' }});
    else if (c.cmd === 'remove') {{ const k = sp.removeSelected(); dioxus.send({{ kind: 'toast', text: k ? 'text removed · Ctrl+Z puts it back' : 'select a text first' }}); }}
    else if (c.cmd === 'free') dioxus.send({{ kind: 'made', key: sp.addFree(c.text), text: c.text || 'text', free: true }});
    else if (c.cmd === 'retext') sp.editFree(c.key, c.text);
    else if (c.cmd === 'rows') dioxus.send({{ kind: 'rows', data: {{ why: c.why, name: c.name || '', rows: sp.rows() }} }});
    else if (c.cmd === 'load') sp.loadRows(c.rows, c.feats || {{}});
    else if (c.cmd === 'colours') sp.colours(c.map || {{}});
    else if (c.cmd === 'blocks') {{
      sp.setBlocks(c.blocks, c.textures || {{}}, !!c.replace);
      if (c.added) sp.record({{ kind: 'blockadd', ids: c.added }});
      if (c.select != null) sp.pickBlock(c.select);
    }}
    else if (c.cmd === 'unblock') sp.unblock(c.ids || []);
    else if (c.cmd === 'blockspot') dioxus.send({{ kind: 'spot', data: {{ ...sp.blockSpot(), want: c.want }} }});
    else if (c.cmd === 'blockfields') sp.setBlockFields(c.id, c.t);
    else if (c.cmd === 'pickblock') sp.pickBlock(c.id);
  }}
}} catch (e) {{
  dioxus.send({{ kind: 'error', text: String(e && e.message || e) }});
}}
"#,
        three = serde_json::to_string(THREE).unwrap_or_default(),
        engine = serde_json::to_string(ENGINE).unwrap_or_default(),
    )
}

#[derive(Clone, Copy)]
struct Sp {
    tx: Signal<Option<futures_channel::mpsc::UnboundedSender<Value>>>,
    /// the texts in the room (memory texts and notes), by key
    texts: Signal<Vec<Hit>>,
    /// what the details window shows: the text under the pointer, else the selected one
    card: Signal<Option<SpaceEvent>>,
    selected: Signal<Option<SpaceEvent>>,
    pos: Signal<String>,
    /// ↑ ↓ in the bar: where in the history
    hist_at: Signal<Option<usize>>,
    paused: Signal<bool>,
    /// each text's bucket axis Q, for the shared-bucket underlines
    q_bucket: Signal<HashMap<String, u64>>,
    /// the selected block: `{id, kind, texture, x, y, z, w, h, d, about}`
    block: Signal<Option<Value>>,
    /// how many blocks and textures the save has
    block_use: Signal<(u64, u64)>,
}

impl Sp {
    fn send(&self, v: Value) {
        if let Some(tx) = self.tx.peek().as_ref() {
            let _ = tx.unbounded_send(v);
        }
    }
}

/// the physics' settings for space.js: the sliders (defaults and the person's changes)
fn physics(ui: Ui) -> Value {
    let p = ui.prefs.peek();
    let mut o = serde_json::Map::new();
    for s in &SLIDERS {
        o.insert(
            s.key.into(),
            json!(p.space.physics.get(s.key).copied().unwrap_or(s.def)),
        );
    }
    o.insert("depth_by".into(), json!(p.space.depth_by));
    o.insert("reversed".into(), json!(p.space_reversed as u8));
    Value::Object(o)
}

/// the letters of a text as counts a–z, for the "same letters" weight
fn letter_counts(t: &str) -> Vec<u32> {
    let mut v = vec![0u32; 26];
    for c in t.bytes() {
        let c = c | 32;
        if c.is_ascii_lowercase() {
            v[(c - b'a') as usize] += 1;
        }
    }
    v
}

/// lays hits out: in place of what's there (pinned and free texts stay), or (keep) added
/// to it; each with what it is, from `space.features` and the colouring classifier
fn show(ui: Ui, api: Api, sp: Sp, hits: Vec<Hit>) {
    let (keep, reversed, tint) = {
        let p = ui.prefs.peek();
        (p.space_keep, p.space_reversed, p.tint.clone())
    };
    let mut all = if keep { sp.texts.peek().clone() } else { vec![] };
    let again = keep && !all.is_empty();
    let mut fresh = Vec::new();
    for h in hits {
        if !all.iter().any(|x| x.key == h.key) {
            all.push(h.clone());
            fresh.push(h);
        }
    }
    let mut texts = sp.texts;
    if all.is_empty() {
        texts.set(vec![]);
        sp.send(json!({ "cmd": "clear", "all": false }));
        return;
    }
    dioxus::dioxus_core::spawn_forever(async move {
        let keys: Vec<String> = fresh.iter().map(|h| h.key.clone()).collect();
        let feats = api
            .call("space.features", json!({ "keys": keys }))
            .await
            .unwrap_or(Value::Null);
        let mut by: HashMap<String, Value> = HashMap::new();
        for f in feats["features"].as_array().into_iter().flatten() {
            by.insert(f["key"].as_str().unwrap_or("").to_string(), f.clone());
        }
        let mut tints: HashMap<String, (Value, Value, String)> = HashMap::new();
        if !tint.is_empty()
            && let Ok(t) = api.call("classify.tints", json!({ "name": tint, "keys": keys })).await
        {
            let palette = crate::views::classifiers::strs(&t["palette"]);
            for x in t["tints"].as_array().into_iter().flatten() {
                let c = x["class"].as_u64().map(|c| c as usize);
                let colour = c.and_then(|c| palette.get(c).cloned()).unwrap_or_default();
                tints.insert(
                    x["key"].as_str().unwrap_or("").to_string(),
                    (x["class"].clone(), x["strength"].clone(), colour),
                );
            }
        }
        // the bucket axis Q (r, or s when reversed) and how many of the room share it
        let mut qs = sp.q_bucket.peek().clone();
        if !again {
            qs.clear();
        }
        for h in &fresh {
            let (s, r) = gem::sums(&h.text);
            qs.insert(h.key.clone(), if reversed { s } else { r });
        }
        let mut count: HashMap<u64, usize> = HashMap::new();
        for q in qs.values() {
            *count.entry(*q).or_default() += 1;
        }
        let payload: Vec<Value> = fresh
            .iter()
            .map(|h| {
                let f = by.get(&h.key).cloned().unwrap_or(Value::Null);
                let (s, r) = gem::sums(&h.text);
                let q = if reversed { s } else { r };
                let mut v = json!({
                    "key": h.key, "text": h.text, "note": h.note,
                    "s": s, "r": r, "q": q, "hue": anend_core::spread::seed_hue(q as i64),
                    "shared": count.get(&q).copied().unwrap_or(0) >= 2,
                    // the gematria grouping underline, the memory view's colour for it
                    "under": gem::underline(&h.text, if reversed { Mode::Reversed } else { Mode::Ordinal }),
                    "chars": f["chars"], "words": f["words"], "kind": f["kind"], "types": f["types"],
                    "created": f["created"], "lists": f["lists"], "freq": letter_counts(&h.text),
                });
                if let Some((c, st, colour)) = tints.get(&h.key) {
                    v["cls"] = c.clone();
                    v["strength"] = st.clone();
                    if !colour.is_empty() {
                        v["colour"] = json!(colour);
                    }
                }
                v
            })
            .collect();
        let mut q_bucket = sp.q_bucket;
        q_bucket.set(qs);
        sp.send(json!({ "cmd": "place", "texts": payload, "again": again }));
        texts.set(all);
    });
}

fn topline(mut ui: Ui, title: String, mode: Mode, text: Option<&str>, n: usize) {
    let (s, r) = text.map(gem::sums).unwrap_or((0, 0));
    ui.topline.set(Some(TopLine {
        title,
        detail: format!("{} · {n} text{}", mode.name(), if n == 1 { "" } else { "s" }),
        s,
        r,
    }));
}

fn space_mode(ui: Ui) -> Mode {
    if ui.prefs.peek().space_reversed {
        Mode::Reversed
    } else {
        Mode::Ordinal
    }
}

async fn gsearch(ui: Ui, api: Api, sp: Sp, n: u64, mode: Mode, text: Option<String>) {
    set_pref(ui, |p| {
        p.space.address = format!("{}{n}", if mode == Mode::Reversed { "r" } else { "" })
    });
    match api.gsearch(mode, &n.to_string()).await {
        Ok(hits) => {
            let t = text.or_else(|| hits.first().map(|h| h.text.clone()));
            topline(ui, n.to_string(), mode, t.as_deref(), hits.len());
            if hits.is_empty() {
                ui.say(format!("nothing in {} {n} yet", mode.name()));
            }
            show(ui, api, sp, hits);
        }
        Err(e) => ui.fail(e),
    }
}

/// a value into the bar, and gsearch it (remem's `envale`)
fn envale(ui: Ui, api: Api, sp: Sp, n: u64, mode: Mode) {
    ui.set_bar(&n.to_string());
    dioxus::dioxus_core::spawn_forever(gsearch(ui, api, sp, n, mode, None));
}

/// the maps kept in the save: name → rows
async fn maps(api: &Api) -> BTreeMap<String, Value> {
    let raw = api.setting("space.maps").await.ok().flatten().unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or_default()
}

/// `#123` an ordinal bucket, `#r123` a reversed one, `#name` a map
async fn route(ui: Ui, api: Api, sp: Sp, addr: &str) {
    let addr = addr.trim();
    let (rev, digits) = match addr.strip_prefix('r') {
        Some(d) => (true, d),
        None => (false, addr),
    };
    if !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && let Ok(n) = digits.parse::<u64>()
    {
        if ui.prefs.peek().space_reversed != rev {
            set_pref(ui, |p| p.space_reversed = rev);
        }
        gsearch(ui, api, sp, n, if rev { Mode::Reversed } else { Mode::Ordinal }, None).await;
        return;
    }
    let all = maps(&api).await;
    let Some(rows) = all.get(addr).cloned() else {
        ui.fail(format!("there's no map named {addr}"));
        return;
    };
    // the memory texts of the map, with what they are
    let keys: Vec<String> = rows
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r[4]["key"].as_str().map(str::to_string))
        .collect();
    let feats = api
        .call("space.features", json!({ "keys": keys }))
        .await
        .unwrap_or(Value::Null);
    let reversed = ui.prefs.peek().space_reversed;
    let mut fm = serde_json::Map::new();
    for f in feats["features"].as_array().into_iter().flatten() {
        let k = f["key"].as_str().unwrap_or("").to_string();
        let mut f = f.clone();
        let (s, r) = (f["s"].as_u64().unwrap_or(0), f["r"].as_u64().unwrap_or(0));
        let q = if reversed { s } else { r };
        f["q"] = json!(q);
        f["hue"] = json!(anend_core::spread::seed_hue(q as i64));
        let text = rows
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r[4]["key"].as_str() == Some(k.as_str()))
            .and_then(|r| r[0].as_str())
            .unwrap_or("")
            .to_string();
        f["under"] = json!(gem::underline(
            &text,
            if reversed { Mode::Reversed } else { Mode::Ordinal }
        ));
        fm.insert(k, f);
    }
    let hits: Vec<Hit> = rows
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(Hit {
                key: r[4]["key"].as_str()?.to_string(),
                text: r[0].as_str().unwrap_or("").to_string(),
                note: r[4]["key"].as_str()?.starts_with("note:"),
            })
        })
        .collect();
    let mut texts = sp.texts;
    texts.set(hits);
    sp.send(json!({ "cmd": "load", "rows": rows, "feats": fm }));
    set_pref(ui, |p| p.space.address = addr.to_string());
    let mut tl = ui.topline;
    tl.set(Some(TopLine {
        title: format!("map {addr}"),
        detail: format!("{} texts", rows.as_array().map(|a| a.len()).unwrap_or(0)),
        s: 0,
        r: 0,
    }));
}

fn push_history(ui: Ui, sp: Sp, v: &str) {
    set_pref(ui, |p| {
        let h = &mut p.space.history;
        if h.last().map(String::as_str) != Some(v) {
            h.push(v.to_string());
        }
        let n = h.len();
        if n > 300 {
            h.drain(0..n - 300);
        }
    });
    let mut at = sp.hist_at;
    at.set(None);
}

fn need(ui: Ui, h: Option<Hit>, what: &str) -> Option<Hit> {
    if h.is_none() {
        ui.fail(format!(
            "no {what}: {} a text to set it",
            if what == "list" { "Shift+click" } else { "Ctrl+click" }
        ));
    }
    h
}

/// the space's words (the Bevy terminal's commands)
async fn command(ui: Ui, api: Api, sp: Sp, word: &str, rest: &str) {
    let picks = ui.picks.peek().clone();
    match word {
        "Enlist" => {
            if let (Some(l), Some(c)) = (need(ui, picks.list, "list"), need(ui, picks.candidate, "candidate")) {
                match api.enlist(&l.key, &c.key).await {
                    Ok(()) => {
                        ui.did(Did::Enlisted {
                            list: l.key.clone(),
                            member: c.key.clone(),
                        });
                        ui.say(format!("enlisted “{}” in “{}”", c.text, l.text));
                        ui.touched();
                    }
                    Err(e) => ui.fail(e),
                }
            }
        }
        "Delist" => {
            if let (Some(l), Some(c)) = (need(ui, picks.list, "list"), need(ui, picks.candidate, "candidate")) {
                match api.delist(&l.key, &c.key).await {
                    Ok(()) => {
                        ui.did(Did::Delisted {
                            list: l.key.clone(),
                            member: c.key.clone(),
                        });
                        ui.say(format!("delisted “{}” from “{}”", c.text, l.text));
                        ui.touched();
                    }
                    Err(e) => ui.fail(e),
                }
            }
        }
        "List" => {
            if let Some(l) = need(ui, picks.list, "list") {
                match api.lists().await {
                    Ok(all) => {
                        let members = all
                            .lists
                            .into_iter()
                            .find(|x| x.list == l.key)
                            .map(|x| x.members)
                            .unwrap_or_default();
                        let mut tl = ui.topline;
                        tl.set(Some(TopLine {
                            title: format!("list {}", l.text),
                            detail: format!("{} texts", members.len()),
                            s: 0,
                            r: 0,
                        }));
                        show(ui, api, sp, members);
                    }
                    Err(e) => ui.fail(e),
                }
            }
        }
        "Unlist" => {
            if let Some(l) = need(ui, picks.list, "list") {
                let members: Vec<String> = api
                    .lists()
                    .await
                    .ok()
                    .and_then(|a| a.lists.into_iter().find(|x| x.list == l.key))
                    .map(|x| x.members.into_iter().map(|m| m.key).collect())
                    .unwrap_or_default();
                match api.unlist(&l.key).await {
                    Ok(n) => {
                        ui.did(Did::Unlisted {
                            list: l.key.clone(),
                            members,
                        });
                        ui.say(format!("unlisted “{}” ({n} members) · Ctrl+Z brings it back", l.text));
                        ui.touched();
                    }
                    Err(e) => ui.fail(e),
                }
            }
        }
        "Count" => {
            let h = ui.hello.peek().clone();
            ui.toast(format!("{} texts · {} notes", h.counts.memories, h.counts.notes));
        }
        "Backup" => {
            let name = ui.hello.peek().save.clone();
            match api.backup(&name).await {
                Ok(b) if !b.paths.is_empty() => ui.say(format!("backed up: {}", b.paths.join(" and "))),
                Ok(_) => ui.say("backed up"),
                Err(e) => ui.fail(e),
            }
        }
        "Clear" | "ClearAll" => {
            let mut texts = sp.texts;
            texts.set(vec![]);
            sp.send(json!({ "cmd": "clear", "all": word == "ClearAll" }));
        }
        "Settle" => sp.send(json!({ "cmd": "settle" })),
        "Help" => set_pref(ui, |p| p.space.show_help = !p.space.show_help),
        "Maps" => {
            let all = maps(&api).await;
            if all.is_empty() {
                ui.say("no maps yet: Save name (or Shift+X) keeps the room as one");
            } else {
                ui.toast(format!(
                    "maps: {}  (#name opens one)",
                    all.keys().map(|k| format!("#{k}")).collect::<Vec<_>>().join("  ")
                ));
            }
        }
        "Save" => {
            let name = rest.trim();
            let name = if name.is_empty() {
                default_map_name(ui)
            } else {
                name.to_string()
            };
            sp.send(json!({ "cmd": "rows", "why": "save", "name": name }));
        }
        _ => {}
    }
}

fn default_map_name(ui: Ui) -> String {
    let a = ui.prefs.peek().space.address.clone();
    let rev = a.starts_with('r');
    let digits = a.trim_start_matches('r');
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        format!("spread-{}{digits}", if rev { "r" } else { "" })
    } else if a.is_empty() {
        "untitled".into()
    } else {
        a
    }
}

const COMMANDS: [&str; 14] = [
    "Enlist", "List", "Delist", "Unlist", "Count", "Backup", "Clear", "ClearAll", "Settle", "Maps", "Save", "Help",
    "Note", "Notes",
];

async fn enter(mut ui: Ui, api: Api, sp: Sp, raw: String, ctrl: bool, shift: bool) {
    let t = raw.trim().to_string();
    // an edit started from the right-click menu or Enter on a text
    let pending = ui.pending.peek().clone();
    if let Pending::Edit(h) = pending {
        ui.pending.set(Pending::None);
        if t.is_empty() {
            return;
        }
        if h.key.starts_with("t:") {
            sp.send(json!({ "cmd": "retext", "key": h.key, "text": t }));
        } else {
            match api.put_memory(&h.key, &t).await {
                Ok(()) => {
                    ui.did(Did::Edited {
                        key: h.key.clone(),
                        old: h.text.clone(),
                    });
                    sp.send(json!({ "cmd": "retext", "key": h.key, "text": t }));
                    ui.say(format!("{} now reads “{t}”", h.key));
                    ui.touched();
                }
                Err(e) => ui.fail(e),
            }
        }
        ui.set_bar("");
        return;
    }
    if t.is_empty() {
        return;
    }
    push_history(ui, sp, &t);
    let mut words = t.split_whitespace();
    let word = words.next().unwrap_or_default().to_string();
    let rest: Vec<&str> = words.collect();
    if COMMANDS.contains(&word.as_str()) && (rest.is_empty() || word == "Save") {
        if matches!(word.as_str(), "Note" | "Notes") {
            ui.say("weum's notes on memories aren't part of anend; lists are");
        } else {
            command(ui, api, sp, &word, &rest.join(" ")).await;
        }
        return;
    }
    if let Some(addr) = t.strip_prefix('#') {
        route(ui, api, sp, addr).await;
        return;
    }
    let (gs, sat) = {
        let p = ui.prefs.peek();
        (p.gsearch_text, p.saturate)
    };
    let mode = space_mode(ui);
    if shift {
        // recall by key
        match api.remember(&t).await {
            Ok(h) => {
                let mut tl = ui.topline;
                tl.set(Some(TopLine {
                    title: format!("key {}", h.key),
                    detail: String::new(),
                    s: 0,
                    r: 0,
                }));
                show(ui, api, sp, vec![h]);
            }
            Err(e) => ui.fail(e),
        }
        return;
    }
    if let Ok(n) = t.parse::<u64>() {
        gsearch(ui, api, sp, n, mode, None).await;
        return;
    }
    if !gs && !ctrl {
        match api.search(&t, true).await {
            Ok(hits) => {
                topline(ui, format!("search “{t}”"), mode, Some(&t), hits.len());
                if hits.is_empty() {
                    ui.say(format!("didn't find anything for {t}"));
                }
                show(ui, api, sp, hits);
            }
            Err(e) => ui.fail(e),
        }
        return;
    }
    if sat || ctrl {
        match api.saturate(&t).await {
            Ok((key, new)) => {
                if new {
                    ui.did(Did::Remembered {
                        key: key.clone(),
                        text: t.clone(),
                    });
                    ui.say(format!("remembered “{t}” under {key}"));
                    ui.touched();
                }
            }
            Err(e) => ui.fail(e),
        }
    }
    gsearch(ui, api, sp, gem::sum_in(&t, mode), mode, Some(t)).await;
}

/// the bar's Delete: forget the memory under the key typed
async fn forget_typed(ui: Ui, api: Api, sp: Sp, raw: String) {
    let key = raw.trim().to_string();
    match api.forget(&key).await {
        Ok(text) => {
            ui.did(Did::Forgot {
                key: key.clone(),
                text: text.clone(),
            });
            ui.say(format!("forgot {key}: “{text}” · Ctrl+Z puts it back"));
            let mut texts = sp.texts;
            texts.write().retain(|h| h.key != key);
            ui.touched();
        }
        Err(e) => ui.fail(e),
    }
}

fn copy_text(ui: Ui, text: &str) {
    let js = format!(
        "(async () => {{ const t = {}; try {{ await navigator.clipboard.writeText(t); }} catch {{ const a = document.createElement('textarea'); a.value = t; document.body.append(a); a.select(); document.execCommand('copy'); a.remove(); }} }})();",
        serde_json::to_string(text).unwrap_or_default()
    );
    document::eval(&js);
    ui.say("copied");
}

/// the room's texts to a file: a download on the web, a file in the data folder on the
/// desktop
fn export(ui: Ui, name: &str, rows: &Value) {
    let text = serde_json::to_string_pretty(rows).unwrap_or_default();
    let file = format!("{name}.json");
    let n = rows.as_array().map(|a| a.len()).unwrap_or(0);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = crate::prefs::data_dir().join("exports");
        let path = dir.join(&file);
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &text)) {
            Ok(()) => ui.say(format!("{n} texts to {}", path.display())),
            Err(e) => ui.fail(format!("couldn't write {}: {e}", path.display())),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::views::settings::download(&file, &text);
        ui.say(format!("{n} texts to {file}"));
    }
}

#[component]
pub fn Space() -> Element {
    let ui = ui();
    let api = api();
    let sp = Sp {
        tx: use_signal(|| None),
        texts: use_signal(Vec::new),
        card: use_signal(|| None),
        selected: use_signal(|| None),
        pos: use_signal(String::new),
        hist_at: use_signal(|| None),
        paused: use_signal(|| false),
        q_bucket: use_signal(HashMap::new),
        block: use_signal(|| None),
        block_use: use_signal(|| (0, 0)),
    };
    // the engine and its channel
    {
        let api = api.clone();
        use_hook(move || {
            let (tx, mut rx) = futures_channel::mpsc::unbounded::<Value>();
            let mut stx = sp.tx;
            stx.set(Some(tx));
            spawn(async move {
                let mut ev = document::eval(&boot_js());
                loop {
                    let got = {
                        let recv = ev.recv::<SpaceEvent>();
                        let next = rx.next();
                        futures_util::pin_mut!(recv, next);
                        match futures_util::future::select(recv, next).await {
                            Either::Left((r, _)) => Either::Left(r),
                            Either::Right((c, _)) => Either::Right(c),
                        }
                    };
                    match got {
                        Either::Left(Ok(e)) => on_event(ui, api.clone(), sp, e),
                        Either::Left(Err(_)) => break,
                        Either::Right(Some(c)) => {
                            let _ = ev.send(c);
                        }
                        Either::Right(None) => break,
                    }
                }
            });
        });
    }
    // the save's blocks, whenever another save opens
    {
        let api = api.clone();
        use_effect(move || {
            let save = (ui.hello)().save;
            let api = api.clone();
            dioxus::dioxus_core::spawn_forever(async move {
                let _ = save;
                load_blocks(ui, api, sp).await;
            });
        });
    }
    // running only while shown; the room's colour and ink from the settings
    use_effect(move || {
        let shown = (ui.view)() == View::Space;
        sp.send(json!({ "cmd": if shown { "start" } else { "stop" } }));
    });
    // the flying keys as the keymap binds them
    use_effect(move || {
        let map = crate::keys::Keymap::from_changes(&ui.prefs.read().keymap).flying();
        sp.send(json!({ "cmd": "keys", "map": map }));
    });
    // the physics, whenever a slider moves
    use_effect(move || {
        let p = ui.prefs.read();
        let _ = (&p.space.physics, &p.space.depth_by, p.space_reversed);
        drop(p);
        sp.send(json!({ "cmd": "physics", "p": physics(ui) }));
    });
    use_effect(move || {
        let bg = ui.prefs.read().space_bg.clone();
        sp.send(json!({ "cmd": "room", "bg": bg, "ink": crate::theme::ink_on(&bg) }));
    });
    use_effect(move || {
        let p = (ui.picks)();
        sp.send(json!({ "cmd": "mark", "list": p.list.map(|h| h.key), "candidate": p.candidate.map(|h| h.key) }));
    });
    {
        let api = api.clone();
        use_keys(View::Space, move |k: KeyPress| {
            if k.bar {
                match k.key.as_str() {
                    "Enter" => {
                        dioxus::dioxus_core::spawn_forever(enter(ui, api.clone(), sp, k.text.clone(), k.ctrl, k.shift));
                        return true;
                    }
                    "Delete" => {
                        dioxus::dioxus_core::spawn_forever(forget_typed(ui, api.clone(), sp, k.text.clone()));
                        return true;
                    }
                    "ArrowUp" | "ArrowDown" => {
                        let h = ui.prefs.peek().space.history.clone();
                        if h.is_empty() || k.text.contains('\n') {
                            return true;
                        }
                        let n = h.len();
                        let cur = sp.hist_at.peek().unwrap_or(n);
                        let i = if k.key == "ArrowUp" {
                            cur.saturating_sub(1)
                        } else {
                            (cur + 1).min(n)
                        };
                        let mut at = sp.hist_at;
                        at.set(Some(i));
                        ui.set_bar(h.get(i).map(String::as_str).unwrap_or(""));
                        return true;
                    }
                    _ => return false,
                }
            }
            if k.key == "Escape" {
                let mut s = sp.selected;
                s.set(None);
                return true;
            }
            let sel = sp.selected.peek().clone();
            match k.action {
                Some(Action::Help) => set_pref(ui, |p| p.space.show_help = !p.space.show_help),
                Some(Action::Hud) => set_pref(ui, |p| p.space.show_hud = !p.space.show_hud),
                Some(Action::Details) => set_pref(ui, |p| p.space.show_details = !p.space.show_details),
                Some(Action::Physics) => set_pref(ui, |p| p.space.show_physics = !p.space.show_physics),
                Some(Action::Reversed) => set_pref(ui, |p| p.space_reversed = !p.space_reversed),
                Some(Action::Pause) => {
                    let on = !*sp.paused.peek();
                    let mut p = sp.paused;
                    p.set(on);
                    sp.send(json!({ "cmd": "pause", "on": on }));
                    ui.say(if on { "physics paused" } else { "physics running" });
                }
                Some(Action::Settle) => sp.send(json!({ "cmd": "settle" })),
                Some(Action::Pin) => sp.send(json!({ "cmd": "pin" })),
                Some(Action::DeleteText) => match sp.block.peek().as_ref().and_then(|b| b["id"].as_u64()) {
                    Some(id) => delete_block(ui, api.clone(), sp, id),
                    None => sp.send(json!({ "cmd": "remove" })),
                },
                Some(Action::NewText) => {
                    let t = ui.bar.peek().trim().to_string();
                    sp.send(json!({ "cmd": "free", "text": if t.is_empty() { "text".to_string() } else { t } }));
                }
                Some(Action::EditText) => match sel {
                    Some(s) => start_edit(ui, &s),
                    None => ui.focus_bar(),
                },
                Some(Action::CopyText) => match sel {
                    Some(s) => copy_text(ui, &s.text),
                    None => ui.say("select a text first"),
                },
                Some(Action::Undo) => sp.send(json!({ "cmd": "undo" })),
                Some(Action::Export) => {
                    sp.send(json!({ "cmd": "rows", "why": "export", "name": default_map_name(ui) }))
                }
                Some(Action::SaveMap) => sp.send(json!({ "cmd": "rows", "why": "save", "name": default_map_name(ui) })),
                Some(Action::SideSum) | Some(Action::SideReversedSum) => {
                    let t = ui.bar.peek().clone();
                    let (s, r) = gem::sums(&t);
                    if k.action == Some(Action::SideSum) {
                        envale(ui, api.clone(), sp, s, Mode::Ordinal);
                    } else {
                        envale(ui, api.clone(), sp, r, Mode::Reversed);
                    }
                }
                // the engine flies and hops on the rest itself
                _ => {
                    return matches!(
                        k.key.as_str(),
                        "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | " "
                    );
                }
            }
            true
        });
    }
    // the last address, opened the first time the space is shown
    {
        let api = api.clone();
        use_hook(move || {
            let a = ui.prefs.peek().space.address.clone();
            if !a.is_empty() {
                dioxus::dioxus_core::spawn_forever(async move {
                    // the engine boots first
                    sleep_ms(500).await;
                    route(ui, api, sp, &a).await;
                });
            }
        });
    }
    let card = (sp.card)();
    let n = sp.texts.read().len();
    let p = ui.prefs.read().clone();
    let bg = p.space_bg.clone();
    let ink = crate::theme::ink_on(&bg);
    let api_g = api.clone();
    let api_b = api.clone();
    rsx! {
        div { class: "space", style: "position: absolute; inset: 0; --room-bg: {bg}; --room-ink: {ink};",
            div { id: "space-host", class: "space-host", tabindex: "0", "aria-label": "the space: right drag looks, W A S D fly, arrows hop between texts" }
            if p.space.show_hud {
                div { class: "poscard", "{sp.pos}" }
            }
            if n == 0 {
                div { class: "space-empty",
                    p { "The room is empty." }
                    p { class: "small", "Type a word, a number or a key in the bar and press Enter: its bucket spreads out here, pulled together by how alike the texts are. Hold the right mouse button to look around, W A S D to fly, the arrow keys hop from text to text; H for help." }
                }
            }
            if p.space.show_help {
                Help {}
            }
            if p.space.show_physics {
                PhysicsPanel {
                    paused: (sp.paused)(),
                    on_settle: move |_| sp.send(json!({ "cmd": "settle" })),
                    on_pause: move |on: bool| {
                        let mut ps = sp.paused;
                        ps.set(on);
                        sp.send(json!({ "cmd": "pause", "on": on }));
                    },
                }
            }
            if p.space.show_blocks {
                Blocks {
                    selected: (sp.selected)(),
                    block: (sp.block)(),
                    counts: (sp.block_use)(),
                    on_send: move |v: Value| sp.send(v),
                    on_duplicate: { let api = api_b.clone(); move |b: Value| duplicate_block(ui, api.clone(), sp, b) },
                    on_delete: { let api = api_b.clone(); move |id: u64| delete_block(ui, api.clone(), sp, id) },
                }
            }
            if p.space.show_details {
                if let Some(c) = card {
                    Details { ev: c, on_gsearch: move |(n, m): (u64, Mode)| envale(ui, api_g.clone(), sp, n, m) }
                }
            }
        }
    }
}

/// puts a text in the bar to edit: Enter puts it back (a memory under its key, a free text
/// in its place)
fn start_edit(mut ui: Ui, s: &SpaceEvent) {
    ui.set_bar(&s.text);
    ui.pending.set(Pending::Edit(Hit {
        key: s.key.clone(),
        text: s.text.clone(),
        note: s.note,
    }));
    ui.focus_bar();
    ui.say("edit it in the bar; Enter puts it back, Esc leaves it");
}

fn on_event(mut ui: Ui, api: Api, sp: Sp, e: SpaceEvent) {
    let hit = Hit {
        key: e.key.clone(),
        text: e.text.clone(),
        note: e.note,
    };
    let mut card = sp.card;
    let mut pos = sp.pos;
    let mut selected = sp.selected;
    match e.kind.as_str() {
        "info" => pos.set(e.text),
        "error" => ui.fail(format!("the space couldn't start: {}", e.text)),
        "toast" => ui.say(e.text),
        "hover" | "focus" => card.set(Some(e.clone())),
        "select" => selected.set((!e.key.is_empty()).then(|| e.clone())),
        "settled" => ui.say(format!("settled in {} steps", e.data)),
        "block" => {
            let mut b = sp.block;
            b.set((!e.data.is_null()).then(|| e.data.clone()));
        }
        "blockset" => {
            let mut b = sp.block;
            if b.peek().as_ref().map(|x| &x["id"]) == Some(&e.data["id"]) {
                let mut v = e.data.clone();
                if let Some(old) = b.peek().as_ref() {
                    v["about"] = old["about"].clone();
                }
                b.set(Some(v));
            }
            let args = e.data.clone();
            dioxus::dioxus_core::spawn_forever(async move {
                if let Err(err) = api.call("blocks.set", args).await {
                    ui.fail(err);
                }
            });
        }
        "spot" => {
            let want = e.data["want"].clone();
            let mut args = want.clone();
            for k in ["x", "y", "z"] {
                args[k] = e.data[k].clone();
            }
            dioxus::dioxus_core::spawn_forever(async move {
                match api.call("blocks.add", args).await {
                    Ok(r) => {
                        let id = r["ids"][0].as_u64();
                        let textures = dress(ui, &api, r["textures"].clone()).await;
                        sp.send(json!({ "cmd": "blocks", "blocks": r["blocks"], "textures": textures, "added": r["ids"], "select": id }));
                        count_blocks(api, sp).await;
                        ui.say("a block · drag its arrows to move it, the knobs on its faces to resize it · Ctrl+Z takes it back");
                    }
                    Err(err) => ui.fail(err),
                }
            });
        }
        // an undo took back blocks just added, or put back blocks just deleted
        "blockgone" => {
            let ids = e.data.clone();
            dioxus::dioxus_core::spawn_forever(async move {
                match api.call("blocks.delete", json!({ "ids": ids })).await {
                    Ok(_) => count_blocks(api, sp).await,
                    Err(err) => ui.fail(err),
                }
            });
        }
        "blockrestore" => {
            let blocks = e.data.clone();
            dioxus::dioxus_core::spawn_forever(async move {
                match api.call("blocks.restore", json!({ "blocks": blocks })).await {
                    Ok(_) => count_blocks(api, sp).await,
                    Err(err) => ui.fail(err),
                }
            });
        }
        "undone" => {
            if e.data == "nothing" {
                undo_last(ui, api);
            } else {
                ui.say("undone");
            }
        }
        "made" => {
            selected.set(Some(e.clone()));
            start_edit(ui, &e);
        }
        "newtext" => {
            let t = ui.bar.peek().trim().to_string();
            sp.send(json!({ "cmd": "free", "text": if t.is_empty() { "text".to_string() } else { t } }));
        }
        "dblclick" => {
            if e.free {
                start_edit(ui, &e);
            } else {
                ui.set_bar(&e.key);
            }
        }
        "rows" => {
            let rows = e.data["rows"].clone();
            let name = e.data["name"].as_str().unwrap_or("untitled").to_string();
            if e.data["why"] == "export" {
                export(ui, &name, &rows);
            } else {
                let ok = (1..=96).contains(&name.len())
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
                if !ok {
                    ui.fail("a map's name: 1 to 96 of letters, digits, _ and -");
                    return;
                }
                dioxus::dioxus_core::spawn_forever(async move {
                    let mut all = maps(&api).await;
                    let n = rows.as_array().map(|a| a.len()).unwrap_or(0);
                    all.insert(name.clone(), rows);
                    match api
                        .set_setting("space.maps", &serde_json::to_string(&all).unwrap_or_default())
                        .await
                    {
                        Ok(()) => {
                            set_pref(ui, |p| p.space.address = name.clone());
                            ui.say(format!("saved the map {name} · {n} texts · #{name} opens it"));
                        }
                        Err(e) => ui.fail(e),
                    }
                });
            }
        }
        "click" => {
            if e.free {
                ui.set_bar(&e.text);
                return;
            }
            if e.ctrl {
                ui.picks.write().candidate = Some(hit.clone());
                ui.say(format!("candidate: {}", hit.text));
            } else if e.shift {
                let mut p = ui.picks.write();
                p.candidate = None;
                p.list = Some(hit.clone());
                drop(p);
                ui.say(format!("list: {}", hit.text));
            } else {
                ui.set_bar(&hit.text);
                let mode = space_mode(ui);
                // its triangle in the top bar
                let (s, r) = gem::sums(&hit.text);
                ui.topline.set(Some(TopLine {
                    title: hit.text.clone(),
                    detail: format!("{} {}", mode.name(), gem::sum_in(&hit.text, mode)),
                    s,
                    r,
                }));
                card.set(Some(e.clone()));
            }
        }
        "menu" => {
            if e.free {
                start_edit(ui, &e);
            } else if e.ctrl {
                let m = gem::Measure::of(&hit.text);
                dioxus::dioxus_core::spawn_forever(gsearch(ui, api, sp, m.triangle(), space_mode(ui), Some(hit.text)));
            } else {
                ui.menu.set(Some(Menu { hit, x: e.x, y: e.y }));
            }
        }
        _ => {}
    }
}

// ------------------------------------------------------------------ blocks

/// what the engine needs to draw the textures: a text's grouping colour (its labels'
/// underline), a picture's thumbnail as a data URL
async fn dress(ui: Ui, api: &Api, textures: Value) -> Value {
    let mode = space_mode(ui);
    let mut out = textures;
    if let Some(o) = out.as_object_mut() {
        for (k, about) in o.iter_mut() {
            if let Some(t) = k.strip_prefix("text:").and(about["text"].as_str().map(str::to_string)) {
                about["colour"] = json!(gem::underline(&t, mode));
            } else if let Some(id) = k.strip_prefix("picture:")
                && about.get("gone").is_none()
                && let Ok(p) = api.call("pictures.get", json!({ "id": id, "thumb": 256 })).await
                && let Some(t) = p["thumb"].as_str()
            {
                about["src"] = json!(format!("data:image/png;base64,{t}"));
            }
        }
    }
    out
}

async fn count_blocks(api: Api, sp: Sp) {
    if let Ok(u) = api.call("blocks.stats", json!({})).await {
        let mut c = sp.block_use;
        c.set((u["blocks"].as_u64().unwrap_or(0), u["textures"].as_u64().unwrap_or(0)));
    }
}

/// the open save's blocks into the room, in place of what was there
async fn load_blocks(ui: Ui, api: Api, sp: Sp) {
    match api.call("blocks.list", json!({})).await {
        Ok(r) => {
            let textures = dress(ui, &api, r["textures"].clone()).await;
            sp.send(json!({ "cmd": "blocks", "blocks": r["blocks"], "textures": textures, "replace": true }));
            let mut b = sp.block;
            b.set(None);
            count_blocks(api, sp).await;
        }
        // a hub from before blocks: the room has none
        Err(_) => sp.send(json!({ "cmd": "blocks", "blocks": [], "replace": true })),
    }
}

fn delete_block(ui: Ui, api: Api, sp: Sp, id: u64) {
    dioxus::dioxus_core::spawn_forever(async move {
        match api.call("blocks.delete", json!({ "ids": [id] })).await {
            Ok(_) => {
                sp.send(json!({ "cmd": "unblock", "ids": [id] }));
                let mut b = sp.block;
                b.set(None);
                count_blocks(api, sp).await;
                ui.say("block deleted · Ctrl+Z puts it back");
            }
            Err(e) => ui.fail(e),
        }
    });
}

fn duplicate_block(ui: Ui, api: Api, sp: Sp, b: Value) {
    let id = b["id"].as_u64().unwrap_or(0);
    // side by side, along its width
    let by = b["w"].as_f64().unwrap_or(1.0) + 0.1;
    dioxus::dioxus_core::spawn_forever(async move {
        match api
            .call("blocks.duplicate", json!({ "ids": [id], "by": [by, 0, 0] }))
            .await
        {
            Ok(r) => {
                let textures = dress(ui, &api, r["textures"].clone()).await;
                sp.send(json!({ "cmd": "blocks", "blocks": r["blocks"], "textures": textures, "added": r["ids"], "select": r["ids"][0] }));
                count_blocks(api, sp).await;
                ui.say("a copy: the same texture, stored once");
            }
            Err(e) => ui.fail(e),
        }
    });
}

/// the blocks window: make blocks from the selected text, a picture or a colour; the
/// selected block's place and size; copy, delete
#[component]
fn Blocks(
    selected: Option<SpaceEvent>,
    block: Option<Value>,
    counts: (u64, u64),
    on_send: EventHandler<Value>,
    on_duplicate: EventHandler<Value>,
    on_delete: EventHandler<u64>,
) -> Element {
    let ui = ui();
    let api = api();
    let mut pictures: Signal<Vec<(String, String)>> = use_signal(Vec::new);
    let mut picture = use_signal(String::new);
    let mut colour = use_signal(|| "#b49a6c".to_string());
    use_hook(move || {
        let api = api.clone();
        spawn(async move {
            if let Ok(v) = api.call("pictures.list", json!({})).await {
                let all: Vec<(String, String)> = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|p| {
                        let id = p["id"].as_str().unwrap_or("").to_string();
                        let name = p["name"]
                            .as_str()
                            .filter(|n| !n.is_empty())
                            .unwrap_or(&id[..id.len().min(8)])
                            .to_string();
                        (id, name)
                    })
                    .collect();
                if let Some((id, _)) = all.first() {
                    picture.set(id.clone());
                }
                pictures.set(all);
            }
        });
    });
    let text = selected.clone().filter(|s| !s.free && !s.key.is_empty());
    let w = window_width();
    let field = move |b: Value, k: &'static str| {
        let v = b[k].as_f64().unwrap_or(0.0);
        let id = b["id"].as_u64().unwrap_or(0);
        rsx! {
            label { class: "bfield", title: if "whd".contains(k) { "a side, in units (the texture tiles once per unit)" } else { "its centre, in units" },
                span { "{k}" }
                input {
                    r#type: "number",
                    step: "0.25",
                    value: "{v:.2}",
                    onchange: move |e| {
                        if let Ok(x) = e.value().parse::<f64>() {
                            on_send.call(json!({ "cmd": "blockfields", "id": id, "t": { k: x } }));
                        }
                    },
                }
            }
        }
    };
    rsx! {
        Win {
            id: "blocks",
            at: ((w - 330.0).max(8.0), 420.0),
            on_close: move |_| set_pref(ui, |p| p.space.show_blocks = false),
            title: rsx! {
                span { class: "win-title",
                    Icon { name: "cube" }
                    "blocks"
                    span { class: "dim small", style: "font-weight: 400;", "{counts.0} in this save · {counts.1} textures" }
                }
            },
            div { class: "blocks-add",
                button {
                    class: "btn small",
                    disabled: text.is_none(),
                    title: "a block showing the selected text, where it is",
                    onclick: move |_| {
                        if let Some(t) = &text {
                            add_block_from(on_send, "text", &t.key)
                        }
                    },
                    Icon { name: "text" }
                    "the selected text"
                }
                if pictures.read().is_empty() {
                    span { class: "dim small", title: "add pictures in the models view (spec models, pictures)", "no pictures yet" }
                } else {
                    span { class: "row",
                        select {
                            "aria-label": "a picture",
                            value: "{picture}",
                            onchange: move |e| picture.set(e.value()),
                            for (id , name) in pictures.read().iter() {
                                option { key: "{id}", value: "{id}", selected: *id == picture(), "{name}" }
                            }
                        }
                        button { class: "btn small", onclick: move |_| add_block_from(on_send, "picture", &picture()), Icon { name: "image" } "picture" }
                    }
                }
                span { class: "row",
                    input { r#type: "color", "aria-label": "a colour", value: "{colour}", oninput: move |e| colour.set(e.value()) }
                    button { class: "btn small", onclick: move |_| add_block_from(on_send, "plain", &colour()), Icon { name: "palette" } "colour" }
                }
            }
            if let Some(b) = block {
                div { class: "blocks-sel",
                    div { class: "small",
                        b {
                            {
                                match b["kind"].as_str().unwrap_or("") {
                                    "text" => format!("“{}”", b["about"]["text"].as_str().unwrap_or("a text")),
                                    "picture" => format!("picture {}", b["about"]["name"].as_str().unwrap_or("")),
                                    _ => format!("colour {}", b["texture"].as_str().unwrap_or("")),
                                }
                            }
                        }
                        span { class: "dim", " · block {b[\"id\"]}" }
                    }
                    div { class: "bfields",
                        {field(b.clone(), "x")}
                        {field(b.clone(), "y")}
                        {field(b.clone(), "z")}
                        {field(b.clone(), "w")}
                        {field(b.clone(), "h")}
                        {field(b.clone(), "d")}
                    }
                    div { class: "row",
                        button { class: "btn small", title: "a copy beside it (the texture is stored once)", onclick: { let b = b.clone(); move |_| on_duplicate.call(b.clone()) }, Icon { name: "copy" } "copy" }
                        button { class: "btn small", title: "{ui.key_of(Action::DeleteText)}", onclick: { let id = b["id"].as_u64().unwrap_or(0); move |_| on_delete.call(id) }, Icon { name: "delete" } "delete" }
                    }
                }
            } else {
                p { class: "dim small", "Click a block to select it: its arrows move it, the knobs on its faces resize it (the texture tiles, it doesn't stretch), the ball moves it across the view. Shift snaps to quarter units." }
            }
        }
    }
}

fn add_block_from(on_send: EventHandler<Value>, kind: &str, texture: &str) {
    if texture.is_empty() {
        return;
    }
    on_send.call(json!({ "cmd": "blockspot", "want": { "kind": kind, "texture": texture } }));
}

/// a window over the room that moves by its title bar and remembers where it was left;
/// its close button shows when the pointer is over it
#[component]
fn Win(id: String, title: Element, at: (f64, f64), on_close: EventHandler<()>, children: Element) -> Element {
    let ui = ui();
    let saved = ui.prefs.read().space.wins.get(&id).copied();
    let mut place = use_signal(move || saved.unwrap_or(at));
    let mut grab: Signal<Option<(f64, f64)>> = use_signal(|| None);
    let (x, y) = place();
    let id2 = id.clone();
    rsx! {
        // kept inside the room however the window is resized: the place is clamped by the
        // layout itself, so a window left far right comes back when the room narrows
        div {
            class: "win",
            style: "left: clamp(0px, {x:.0}px, max(0px, calc(100% - var(--win-w)))); top: clamp(0px, {y:.0}px, max(0px, calc(100% - var(--over-bottom, 40px) - 44px)));",
            role: "dialog",
            div {
                class: "win-head",
                title: "drag to move it",
                onpointerdown: move |e| {
                    // from where it shows (clamped), not where it was left; no text selection
                    e.prevent_default();
                    let c = e.client_coordinates();
                    let o = e.element_coordinates();
                    place.set((c.x - o.x, c.y - o.y));
                    grab.set(Some((o.x, o.y)));
                },
                {title}
                button {
                    class: "plain ibtn close",
                    "aria-label": "close",
                    onclick: move |_| on_close.call(()),
                    onpointerdown: move |e| e.stop_propagation(),
                    Icon { name: "close" }
                }
            }
            div { class: "win-body", {children} }
        }
        if grab().is_some() {
            // while dragging, a sheet over everything follows the pointer
            div {
                class: "win-drag",
                onpointermove: move |e| {
                    if let Some((ox, oy)) = grab() {
                        let c = e.client_coordinates();
                        place.set(((c.x - ox).max(0.0), (c.y - oy).max(0.0)));
                    }
                },
                onpointerup: move |_| {
                    grab.set(None);
                    let id = id2.clone();
                    spawn(async move {
                        // kept clamped to the window as it is now
                        let mut ev = document::eval("dioxus.send([innerWidth, innerHeight]);");
                        let (w, h) = ev.recv::<(f64, f64)>().await.unwrap_or((1e6, 1e6));
                        let (x, y) = place();
                        let at = (x.clamp(0.0, (w - 120.0).max(0.0)), y.clamp(0.0, (h - 80.0).max(0.0)));
                        place.set(at);
                        set_pref(ui, move |p| {
                            p.space.wins.insert(id, at);
                        });
                    });
                },
            }
        }
    }
}

/// the numbers of a hovered or clicked text (the Bevy app's metrics card), each gsearching
/// itself; its key, bucket and whether it's pinned
#[component]
fn Details(ev: SpaceEvent, on_gsearch: EventHandler<(u64, Mode)>) -> Element {
    let ui = ui();
    let m = anend_core::spread::metrics(&ev.text);
    let rev = ui.prefs.read().space_reversed;
    let mode = if rev { Mode::Reversed } else { Mode::Ordinal };
    let n = move |v: i64, md: Option<Mode>| {
        let md = md.unwrap_or(mode);
        rsx! {
            span {
                class: "num",
                title: "gsearch {v} ({md.name()})",
                onclick: move |_| {
                    if v >= 0 {
                        on_gsearch.call((v as u64, md))
                    }
                },
                "{v}"
            }
        }
    };
    let rows: [[(&str, Element); 2]; 7] = [
        [("s", n(m.s, Some(Mode::Ordinal))), ("r", n(m.r, Some(Mode::Reversed)))],
        [("s+r", n(m.sr, None)), ("s·r", n(m.product, None))],
        [("√(s·r)", rsx! { "{m.root:.3}" }), ("⌊√(s·r)⌋", n(m.jump, None))],
        [("s−r", rsx! { "{m.diff}" }), ("s+r+|s−r|", n(m.wide, None))],
        [("s+r+(s−r)", n(m.lean, None)), ("|s−r|−(s−r)", n(m.gap, None))],
        [("v", n(m.v, None)), ("c", n(m.c, None))],
        [("w", rsx! { "{m.w}" }), ("letters", rsx! { "{m.letters}" })],
    ];
    let w = window_width();
    let key = ev.key.clone();
    let q = ev.q as u64;
    rsx! {
        Win {
            id: "details",
            at: ((w - 330.0).max(8.0), 52.0),
            on_close: move |_| set_pref(ui, |p| p.space.show_details = false),
            title: rsx! { span { class: "win-title", "{ev.text}" } },
            table { class: "metrics-table",
                for (i , row) in rows.into_iter().enumerate() {
                    tr { key: "{i}",
                        for (label , cell) in row {
                            th { key: "{label}", "{label}" }
                            td { {cell} }
                        }
                    }
                }
            }
            Triangle { s: m.s.max(0) as u64, r: m.r.max(0) as u64, height: 40.0 }
            div { class: "gsum", {gem::gsum_line(&ev.text)} }
            if !ev.free {
                div { class: "dim small row",
                    "key "
                    span { class: "num", title: "into the bar", onclick: move |_| ui.set_bar(&key), "{ev.key}" }
                    " · bucket "
                    span { class: "swatch", style: "display: inline-block; width: 10px; height: 10px; border-radius: 2px; background: hsl({ev.hue} 70% 45%);" }
                    span {
                        class: "num",
                        title: "gsearch it",
                        onclick: move |_| on_gsearch.call((q, if rev { Mode::Ordinal } else { Mode::Reversed })),
                        " {q}"
                    }
                    if ev.pinned { " · pinned" }
                }
            } else {
                div { class: "dim small",
                    "a free text: double click or Enter edits it"
                    if ev.pinned { " · pinned" }
                }
            }
            if m.s > 1 {
                crate::views::memory::PfLine { n: m.s as u64 }
            }
        }
    }
}

/// the window's width, for placing a window at the right the first time
fn window_width() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|v| v.as_f64())
            .unwrap_or(1200.0)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        1200.0
    }
}

/// the physics panel: every slider (double click its name: back to its default), what the
/// depth follows, reset all, settle, pause
#[component]
fn PhysicsPanel(paused: bool, on_settle: EventHandler<()>, on_pause: EventHandler<bool>) -> Element {
    let ui = ui();
    let p = ui.prefs.read().space.clone();
    let mut groups: Vec<(&str, Vec<&Slider>)> = Vec::new();
    for s in &SLIDERS {
        match groups.iter_mut().find(|(g, _)| *g == s.group) {
            Some((_, v)) => v.push(s),
            None => groups.push((s.group, vec![s])),
        }
    }
    rsx! {
        Win {
            id: "physics",
            at: (8.0, 80.0),
            on_close: move |_| set_pref(ui, |p| p.space.show_physics = false),
            title: rsx! {
                span { class: "win-title", Icon { name: "sliders" } " physics" }
            },
            div { class: "physics",
                for (g , sliders) in groups {
                    div { key: "{g}", class: "pgroup",
                        h4 { "{g}" }
                        for s in sliders {
                            PSlider { key: "{s.key}", which: s.key, value: p.physics.get(s.key).copied().unwrap_or(s.def) }
                        }
                        if g == "depth" {
                            label { class: "pslider",
                                span { class: "pname", "depth follows" }
                                select {
                                    value: "{p.depth_by}",
                                    onchange: move |e| set_pref(ui, |p| p.space.depth_by = e.value()),
                                    for d in DEPTH_BY {
                                        option { key: "{d}", value: "{d}", selected: p.depth_by == d, "{d}" }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "row",
                    button {
                        onclick: move |_| set_pref(ui, |p| {
                            p.space.physics.clear();
                            p.space.depth_by = "none".into();
                        }),
                        Icon { name: "undo" }
                        "reset all"
                    }
                    button { "data-tip": "settle now · {ui.key_of(Action::Settle)}", onclick: move |_| on_settle.call(()),
                        Icon { name: "check" }
                        "settle"
                    }
                    button {
                        class: if paused { "on" } else { "" },
                        "data-tip": "pause the physics · {ui.key_of(Action::Pause)}",
                        onclick: move |_| on_pause.call(!paused),
                        Icon { name: if paused { "play" } else { "pause" } }
                        if paused { "run" } else { "pause" }
                    }
                }
            }
        }
    }
}

#[component]
fn PSlider(which: &'static str, value: f64) -> Element {
    let ui = ui();
    let Some(s) = SLIDERS.iter().find(|s| s.key == which) else {
        return rsx! {};
    };
    let def = s.def;
    let decimals = (-s.step.log10()).ceil().max(0.0) as usize;
    rsx! {
        label { class: "pslider",
            span {
                class: "pname",
                title: "double click: back to {def}",
                ondoubleclick: move |_| set_pref(ui, move |p| {
                    p.space.physics.remove(which);
                }),
                "{s.label}"
            }
            input {
                r#type: "range",
                min: "{s.min}",
                max: "{s.max}",
                step: "{s.step}",
                value: "{value}",
                oninput: move |e| {
                    if let Ok(x) = e.value().parse::<f64>() {
                        set_pref(ui, move |p| {
                            if (x - def).abs() < 1e-9 {
                                p.space.physics.remove(which);
                            } else {
                                p.space.physics.insert(which.to_string(), x);
                            }
                        });
                    }
                },
            }
            span { class: "pval", {format!("{value:.decimals$}")} }
        }
    }
}

/// the keys as they're bound, and what the mouse does
#[component]
fn Help() -> Element {
    let ui = ui();
    let k = |a: Action| ui.key_of(a);
    let sections: Vec<(&str, Vec<(String, &str)>)> = vec![
        (
            "flying",
            vec![
                ("Right mouse".into(), "hold to look"),
                (
                    format!(
                        "{} {} {} {}",
                        k(Action::FlyForward),
                        k(Action::FlyLeft),
                        k(Action::FlyBack),
                        k(Action::FlyRight)
                    ),
                    "move",
                ),
                (format!("{} / {}", k(Action::FlyUp), k(Action::FlyDown)), "up / down"),
                (format!("{} / {}", k(Action::RollLeft), k(Action::RollRight)), "roll"),
                ("Shift".into(), "fast"),
                (
                    format!("{} {} or the wheel", k(Action::Slower), k(Action::Faster)),
                    "flight speed",
                ),
                (k(Action::Level), "level the horizon"),
                (k(Action::BackToStart), "back to start"),
                ("arrows".into(), "hop to the nearest text that way"),
            ],
        ),
        (
            "texts",
            vec![
                ("drag a text".into(), "move it; it stays pinned"),
                ("wheel on a text".into(), "push away / pull near"),
                ("click".into(), "its text into the bar, its numbers in the details"),
                ("double click".into(), "its key into the bar (a free text: edit it)"),
                (
                    format!("double click the room, {}", k(Action::NewText)),
                    "a new free text",
                ),
                (k(Action::EditText), "edit the selected text"),
                (k(Action::DeleteText), "remove it from the room"),
                (k(Action::CopyText), "copy it"),
                (k(Action::Pin), "pin / unpin it"),
                (k(Action::Undo), "undo (a move, a pin, a removal; else the last action)"),
                (
                    format!("{} / {}", k(Action::Export), k(Action::SaveMap)),
                    "export the texts / save the map",
                ),
            ],
        ),
        (
            "blocks",
            vec![
                (
                    "the cube chip".into(),
                    "the blocks window: a block of the selected text, a picture or a colour",
                ),
                ("click a block".into(), "select it; drag it across the view"),
                ("drag an arrow".into(), "move it along that axis"),
                (
                    "drag a face's knob".into(),
                    "resize it: the texture tiles, the far face stays",
                ),
                ("Shift while dragging".into(), "snap to quarter units"),
                (k(Action::DeleteText), "delete the selected block"),
                (k(Action::Undo), "undo a move, a resize, an add or a delete"),
            ],
        ),
        (
            "the bar",
            vec![
                ("Enter".into(), "a number: its bucket; a text: its sum (or its words)"),
                ("Ctrl+Enter".into(), "remember the text, gsearch its sum"),
                ("Shift+Enter".into(), "bring back a key, like 0,12"),
                ("Delete".into(), "forget the key typed"),
                ("↑ ↓".into(), "history"),
                ("#123  #r123  #name".into(), "a bucket, a reversed one, a saved map"),
                (
                    "words".into(),
                    "Enlist List Delist Unlist Count Backup Clear ClearAll Settle Maps Save name",
                ),
                ("Ctrl+click / Shift+click".into(), "the list's candidate / the list"),
                ("Ctrl+right click".into(), "gsearch ⌊√(s·r)⌋"),
                (
                    format!("{} {}", k(Action::SideSum), k(Action::SideReversedSum)),
                    "gsearch the bar's sums",
                ),
            ],
        ),
        (
            "panels",
            vec![
                (k(Action::Physics), "the physics"),
                (
                    format!("{} / {}", k(Action::Pause), k(Action::Settle)),
                    "pause / settle now",
                ),
                (k(Action::Details), "the details window"),
                (k(Action::Hud), "the position card"),
                (k(Action::Reversed), "reversed sums"),
                (k(Action::Help), "this help"),
            ],
        ),
    ];
    rsx! {
        div { class: "space-help",
            button {
                class: "plain ibtn close",
                "aria-label": "close",
                onclick: move |_| set_pref(ui, |p| p.space.show_help = false),
                Icon { name: "close" }
            }
            div { class: "help-cols",
                for (title , rows) in sections {
                    div { key: "{title}",
                        h4 { "{title}" }
                        table {
                            for (key , what) in rows {
                                tr { key: "{key}",
                                    td { kbd { "{key}" } }
                                    td { "{what}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// what the space adds to the command bar: the bar's sums, the list state, the chips
#[component]
pub fn SpaceStrip() -> Element {
    let ui = ui();
    let p = ui.prefs.read().clone();
    let measure = (ui.measure)();
    let picks = (ui.picks)();
    let list = match (&picks.list, &picks.candidate) {
        (Some(l), Some(c)) => format!("list {} · candidate {}", l.text, c.text),
        (Some(l), None) => format!("list {}", l.text),
        (None, Some(c)) => format!("candidate {}", c.text),
        _ => String::new(),
    };
    rsx! {
        if let Some(me) = measure {
            span { class: "sidecar", "aria-label": "the bar's sums",
                "("
                span {
                    class: if !p.space_reversed { "num on" } else { "num" },
                    title: "ordinal · {ui.key_of(Action::SideSum)} gsearches it",
                    onclick: move |_| set_pref(ui, |p| p.space_reversed = false),
                    "{me.ordinal}"
                }
                ", "
                span {
                    class: if p.space_reversed { "num on" } else { "num" },
                    title: "reversed · {ui.key_of(Action::SideReversedSum)} gsearches it",
                    onclick: move |_| set_pref(ui, |p| p.space_reversed = true),
                    "{me.reversed}"
                }
                ")"
            }
        }
        if !list.is_empty() {
            span { class: "sidenote", "{list}" }
        }
        span { class: "toggles",
            TChip {
                label: "gsearch: a text by its sum, not its words",
                icon: "search",
                on: p.gsearch_text,
                onclick: move |_| set_pref(ui, |p| p.gsearch_text = !p.gsearch_text),
            }
            TChip {
                label: format!("reversed: bucket by the reversed sum · {}", ui.key_of(Action::Reversed)),
                letter: "R",
                on: p.space_reversed,
                onclick: move |_| set_pref(ui, |p| p.space_reversed = !p.space_reversed),
            }
            TChip {
                label: "saturate: remember what's searched",
                icon: "bolt",
                on: p.saturate,
                onclick: move |_| set_pref(ui, |p| p.saturate = !p.saturate),
            }
            TChip {
                label: "keep: add new results to the spread",
                icon: "pin",
                on: p.space_keep,
                onclick: move |_| set_pref(ui, |p| p.space_keep = !p.space_keep),
            }
        }
        span { class: "toggles",
            TChip {
                label: format!("the physics · {}", ui.key_of(Action::Physics)),
                icon: "sliders",
                on: p.space.show_physics,
                onclick: move |_| set_pref(ui, |p| p.space.show_physics = !p.space.show_physics),
            }
            TChip {
                label: format!("the details window · {}", ui.key_of(Action::Details)),
                icon: "view",
                on: p.space.show_details,
                onclick: move |_| set_pref(ui, |p| p.space.show_details = !p.space.show_details),
            }
            TChip {
                label: "blocks: spatial saves, boxes showing texts and pictures",
                icon: "cube",
                on: p.space.show_blocks,
                onclick: move |_| set_pref(ui, |p| p.space.show_blocks = !p.space.show_blocks),
            }
            TChip {
                label: format!("help · {}", ui.key_of(Action::Help)),
                icon: "help",
                on: p.space.show_help,
                onclick: move |_| set_pref(ui, |p| p.space.show_help = !p.space.show_help),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliders_match_the_engine() {
        // every slider is one of space.js's PHYSICS keys, with the same default
        for s in &SLIDERS {
            let pat = format!("{}: {}", s.key, s.def);
            let alt = format!("{}: {}", s.key, s.def as i64);
            assert!(
                ENGINE.contains(&pat) || ENGINE.contains(&alt),
                "{} = {} isn't space.js's",
                s.key,
                s.def
            );
            assert!(s.min <= s.def && s.def <= s.max, "{}", s.key);
        }
        // a free spread: no spiral and no anchors left in the engine
        assert!(!ENGINE.contains("spiral(") && !ENGINE.contains("p.anchor"));
        assert_eq!(letter_counts("Ab b")[1], 2);
    }
}
