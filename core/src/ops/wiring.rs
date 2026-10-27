//! Blocks that cast rays and run programs (`wiring.*`, queued requests #15 and #28;
//! `graph::wiring` keeps what a block listens for, its program and its ray).
//!
//! `wiring.fire` is one cause and everything it sets off, in order: the block that the
//! event reaches runs its s program (when it listens for that event), with the event, who
//! caused it, its own address and the cause's value as arguments (a hit's or a signal's
//! value is the first word of the sender's kept state: a switch's 1 or 0, a gate's
//! output), and its kept state as stdin. The lines the
//! program prints that start with one of these words act on the block; the rest are its
//! output:
//!
//! - `move dx dy dz`, `place x y z`, `turn ax ay az` (degrees about the room's x, y, z),
//!   `size w h d`: where it is, how turned, how big;
//! - `colour #rrggbb`, `texture <key>` (`plain:#rrggbb`, `texture:<id>`, `text:<key>`): what
//!   it shows (it comes back under a new address, as a retextured block does);
//! - `play <animation's name>`, `stop`;
//! - `signal <name>`: every block listening for that signal gets it;
//! - `cast`: it casts its ray now;
//! - `keep <text>`: the state its next run reads;
//! - `say <text>`: a line shown in the room;
//! - `sound`, `hush`: its sound (a sound emitter's) starts or stops, while the room plays.
//!
//! A block without a program casts its ray whenever an event it listens for reaches it.
//! Each block a ray registers gets a `hit` from the caster. One cause sets off at most
//! [`MAX_EVENTS`] events, and their programs share one budget of steps (2 million on a
//! shared hub, 50 million on one's own machine), so a loop of blocks hitting each other
//! stops. The same cause in the same save always does the same things.

use std::collections::{BTreeMap, VecDeque};

use serde_json::{Value, json};

use super::{Res, s};
use crate::graph::blocks::{Block, Transform};
use crate::graph::record::{self, mime};
use crate::graph::wiring::{self, Event, WireRef, Wiring};
use crate::graph::{Graph, Reader};

/// the events one cause sets off at most
pub const MAX_EVENTS: usize = 200;

pub fn routed(op: &str) -> bool {
    op.starts_with("wiring.")
}

/// a block's wiring, when it has one that reads
pub(super) fn wiring_of(g: &Graph, b: &Block) -> Result<Option<Wiring>, String> {
    // a prefab copy's part: its copy's own wiring, else the prefab's
    if let Some(own) = g.part_wiring(&b.id)? {
        return Ok(serde_json::from_value(own).ok());
    }
    let Some(r) = b.t.wire else { return Ok(None) };
    let v = g.view()?;
    match v.record(r.entry)? {
        Some(rec) if rec.mime == mime::WIRING => Ok(serde_json::from_str(&v.render(r.entry)?).ok()),
        _ => Ok(None),
    }
}

/// gives the block this wiring (none takes it away); the block as it is now. An entry
/// another block shares isn't changed: this block gets an entry of its own
pub(super) fn set_wiring(g: &Graph, b: &Block, w: Option<Wiring>) -> Result<Block, String> {
    // a part's wiring changes in its copy only
    if crate::graph::prefabs::is_part(&b.id) {
        let v = match w {
            Some(w) => Some(serde_json::to_value(w.checked()?).map_err(|e| e.to_string())?),
            None => Some(serde_json::json!({ "on": [] })),
        };
        g.set_part_wiring(&b.id, v)?;
        return g.block(&b.id)?.ok_or_else(|| "the part went".to_string());
    }
    let mut t = b.t;
    match w {
        None => t.wire = None,
        Some(w) => {
            let text = serde_json::to_string(&w.checked()?).map_err(|e| e.to_string())?;
            let shared = |entry: u64| -> Result<bool, String> {
                // the room's other blocks, and the prefabs' parts
                let mut others = g.blocks(None)?;
                for p in g.prefabs()? {
                    others.extend(g.prefab_parts(p.id)?.into_iter().map(|mut x| {
                        x.id = format!("prefab {}: {}", p.id, x.id);
                        x
                    }));
                }
                Ok(others
                    .iter()
                    .any(|o| o.id != b.id && o.t.wire.is_some_and(|r| r.entry == entry)))
            };
            let entry = match b.t.wire {
                Some(r) if !shared(r.entry)? && g.view()?.record(r.entry)?.is_some_and(|x| x.mime == mime::WIRING) => {
                    if g.render(r.entry)? != text {
                        g.edit_text(r.entry, &text)?;
                    }
                    r.entry
                }
                _ => g.save_text(&text, mime::WIRING)?.0,
            };
            t.wire = Some(WireRef { entry });
        }
    }
    Ok(g.set_blocks(&[(b.id.clone(), t)])?.swap_remove(0))
}

fn nums(words: &[&str]) -> Option<[f32; 3]> {
    if words.len() < 4 {
        return None;
    }
    let p = |w: &str| w.parse::<f32>().ok().filter(|x| x.is_finite());
    Some([p(words[1])?, p(words[2])?, p(words[3])?])
}

/// the quaternion (x, y, z, w) of a turn in degrees about x, then y, then z
fn euler(deg: [f32; 3]) -> [f32; 4] {
    let h = deg.map(|d| d.to_radians() / 2.0);
    let (sx, cx) = h[0].sin_cos();
    let (sy, cy) = h[1].sin_cos();
    let (sz, cz) = h[2].sin_cos();
    // three.js's 'XYZ' order
    [
        sx * cy * cz + cx * sy * sz,
        cx * sy * cz - sx * cy * sz,
        cx * cy * sz + sx * sy * cz,
        cx * cy * cz - sx * sy * sz,
    ]
}

fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    let q = [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ];
    let n = q.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    q.map(|x| x / n)
}

/// what one cause did
#[derive(Default)]
struct Report {
    events: Vec<Value>,
    rays: Vec<Value>,
    says: Vec<String>,
    /// emitters to start (`true`) or stop: `{block, play}`
    sounds: Vec<Value>,
    /// blocks changed, by their address now
    changed: Vec<String>,
    /// addresses that went (a block retextured comes back under a new one)
    removed: Vec<String>,
    cut: bool,
}

/// what a block tells the blocks it hits or signals: the first word of its kept state
/// (a switch's 1 or 0, a gate's output)
fn value_of(w: &Wiring) -> String {
    w.state.split_whitespace().next().unwrap_or("").to_string()
}

/// the blocks listening for a signal by this name, in address order
fn listeners(g: &Graph, name: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for b in g.world_blocks()? {
        if let Some(w) = wiring_of(g, &b)?
            && w.on.contains(&Event::Signal)
            && w.signal == name
        {
            out.push(b.id);
        }
    }
    out.sort();
    Ok(out)
}

/// a program's line acting on its block: what changed, and whether it was an action
#[allow(clippy::too_many_arguments)]
fn act(
    g: &Graph,
    line: &str,
    id: &mut String,
    w: &mut Wiring,
    kept: &mut bool,
    cast: &mut bool,
    q: &mut VecDeque<(String, Event, String, String)>,
    r: &mut Report,
) -> Result<bool, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(&first) = words.first() else { return Ok(false) };
    let rest = line.trim_start()[first.len()..].trim();
    let Some(b) = g.block(id)? else { return Ok(false) };
    let mut t: Transform = b.t;
    match first {
        "move" | "place" | "turn" | "size" => {
            let Some(v) = nums(&words) else { return Ok(false) };
            match first {
                "move" => (t.x, t.y, t.z) = (t.x + v[0], t.y + v[1], t.z + v[2]),
                "place" => (t.x, t.y, t.z) = (v[0], v[1], v[2]),
                "turn" => t.rot = qmul(euler(v), t.rot),
                _ => (t.w, t.h, t.d) = (v[0].clamp(0.01, 1e4), v[1].clamp(0.01, 1e4), v[2].clamp(0.01, 1e4)),
            }
        }
        "colour" | "color" | "texture" => {
            let key = if first == "texture" {
                rest.to_string()
            } else {
                format!("plain:{rest}")
            };
            let Some((kind, tex)) = crate::graph::blocks::split_key(&key) else {
                return Ok(false);
            };
            let tex = super::blocks::texture(g, kind, tex)?;
            // a part shows it in its copy only, under the same address
            if crate::graph::prefabs::is_part(id) {
                g.retexture_part(id, kind, &tex)?;
                if !r.changed.contains(id) {
                    r.changed.push(id.clone());
                }
                return Ok(true);
            }
            if kind != b.kind || tex != b.texture {
                g.delete_blocks(std::slice::from_ref(id))?;
                let new = g.add_blocks(&[(kind, tex, b.t)])?.swap_remove(0);
                r.removed.push(std::mem::replace(id, new));
                r.changed.retain(|c| !r.removed.contains(c));
                r.changed.push(id.clone());
            }
            return Ok(true);
        }
        "play" => {
            let v = g.view()?;
            let found = v.query(
                &crate::graph::query::Query {
                    mime: Some(mime::ANIMATION),
                    ..Default::default()
                },
                crate::graph::query::Page::all(),
            )?;
            let mut hit = None;
            for e in found.ids {
                if let Ok(a) = serde_json::from_str::<crate::graph::anims::Animation>(&v.render(e)?)
                    && a.name == rest
                {
                    hit = Some(e);
                    break;
                }
            }
            let Some(entry) = hit else {
                return Err(format!("no animation named {rest:?}"));
            };
            t.anim = Some(crate::graph::anims::AnimRef {
                entry,
                delay: 0.0,
                speed: 1.0,
            });
        }
        "stop" if words.len() == 1 => t.anim = None,
        "signal" if !rest.is_empty() => {
            for to in listeners(g, rest)? {
                q.push_back((to, Event::Signal, id.clone(), value_of(w)));
            }
            return Ok(true);
        }
        "cast" if words.len() == 1 => {
            *cast = true;
            return Ok(true);
        }
        "keep" => {
            w.state = rest.to_string();
            *kept = true;
            return Ok(true);
        }
        "say" => {
            r.says.push(rest.to_string());
            return Ok(true);
        }
        "sound" | "hush" if words.len() == 1 => {
            if b.t.sound.is_none() {
                return Err("it has no sound (the sounds window gives it one)".into());
            }
            r.sounds.push(json!({ "block": id.clone(), "play": first == "sound" }));
            return Ok(true);
        }
        _ => return Ok(false),
    }
    g.set_blocks(&[(id.clone(), t)])?;
    if !r.changed.contains(id) {
        r.changed.push(id.clone());
    }
    Ok(true)
}

/// one cause and what it sets off
fn fire(
    g: &Graph,
    id: &str,
    event: Event,
    from: &str,
    value: &str,
    shared: bool,
    host: Option<crate::s::machine::Host<'_>>,
) -> Result<Report, String> {
    let progs = super::script::programs(g)?;
    let mut files = BTreeMap::new();
    for (title, (_, body)) in &progs {
        files.insert(format!("{title}.s"), body.clone());
        files.insert(title.clone(), body.clone());
    }
    let mut budget: u64 = if shared { 2_000_000 } else { 50_000_000 };
    let mut r = Report::default();
    let mut q: VecDeque<(String, Event, String, String)> = VecDeque::new();
    q.push_back((id.to_string(), event, from.to_string(), value.to_string()));
    let mut n = 0;
    while let Some((mut id, ev, from, value)) = q.pop_front() {
        let Some(b) = g.block(&id)? else { continue };
        let Some(mut w) = wiring_of(g, &b)? else { continue };
        if !w.on.contains(&ev) {
            continue;
        }
        if n >= MAX_EVENTS {
            r.cut = true;
            break;
        }
        n += 1;
        let mut log = json!({ "block": id, "event": ev.name(), "from": from });
        let mut cast = w.program.is_empty();
        let mut kept = false;
        if !w.program.is_empty() {
            match progs.get(&w.program) {
                None => log["err"] = json!(format!("no s program titled {:?}", w.program)),
                Some((_, code)) => {
                    let args = [ev.name().to_string(), from.clone(), id.clone(), value.clone()];
                    let o = crate::s::run(code, &w.state, files.clone(), &args, budget.max(1), g.now(), host);
                    budget = budget.saturating_sub(o.steps);
                    let mut out = Vec::new();
                    for line in o.out.lines() {
                        match act(g, line, &mut id, &mut w, &mut kept, &mut cast, &mut q, &mut r) {
                            Ok(true) => {}
                            Ok(false) => {
                                if !line.trim().is_empty() {
                                    out.push(line.trim().to_string());
                                }
                            }
                            Err(e) => {
                                log["err"] = json!(e);
                                break;
                            }
                        }
                    }
                    log["out"] = json!(out.join("\n"));
                    if let Some(e) = o.err {
                        log["err"] = json!(e);
                    }
                    log["steps"] = json!(o.steps);
                    if budget == 0 {
                        r.cut = true;
                    }
                }
            }
        }
        log["block"] = json!(id);
        if cast && let Some(mut ray) = w.ray.clone() {
            let b = g.block(&id)?.ok_or("the block went")?;
            let from_at = [b.t.x, b.t.y, b.t.z];
            let hits: Vec<String> = if ray.targets.is_empty() {
                let all = g.world_blocks()?;
                let hits = wiring::cast(&b, &ray, &all);
                r.rays.push(json!({ "from": from_at, "to": wiring::ray_end(&b, &ray, &hits), "hits": hits.iter().map(|h| &h.0).collect::<Vec<_>>() }));
                let ids: Vec<String> = hits.into_iter().map(|h| h.0).collect();
                if ray.stick && !ids.is_empty() {
                    ray.targets = ids.clone();
                    ray.stick = false;
                    w.ray = Some(ray);
                    kept = true;
                }
                ids
            } else {
                let mut ids = Vec::new();
                // a part's target inside its prefab is that part of its own copy
                let copy = crate::graph::prefabs::split_part(&id).map(|(c, _)| c.to_string());
                for t in &ray.targets {
                    let inside = copy.as_ref().map(|c| format!("{c}/{t}"));
                    let found = match &inside {
                        Some(i) => g.block(i)?.map(|b| (i.clone(), b)),
                        None => None,
                    };
                    let found = match found {
                        Some(f) => Some(f),
                        None => g.block(t)?.map(|b| (t.clone(), b)),
                    };
                    if let Some((t, tb)) = found {
                        r.rays
                            .push(json!({ "from": from_at, "to": [tb.t.x, tb.t.y, tb.t.z], "hits": [t] }));
                        ids.push(t);
                    }
                }
                ids
            };
            for h in hits {
                q.push_back((h, Event::Hit, id.clone(), value_of(&w)));
            }
        }
        if kept {
            let b = g.block(&id)?.ok_or("the block went")?;
            set_wiring(g, &b, Some(w))?;
        }
        r.events.push(log);
    }
    if !q.is_empty() && n >= MAX_EVENTS {
        r.cut = true;
    }
    Ok(r)
}

pub fn call(g: &Graph, op: &str, args: &Value, shared: bool) -> Res {
    call_hosted(g, op, args, shared, None)
}

/// `call`, with a host whose ops a block's program reaches through `std:anend`'s `op`
pub fn call_hosted(g: &Graph, op: &str, args: &Value, shared: bool, host: Option<crate::s::machine::Host<'_>>) -> Res {
    let block = |args: &Value| -> Result<Block, String> {
        let id = s(args, "id");
        g.block(id)?.ok_or_else(|| format!("no block {id}"))
    };
    match op {
        // the wired blocks: `[{id, key, wiring}]`
        "wiring.list" => {
            let mut out = Vec::new();
            for b in g.world_blocks()? {
                if let Some(w) = wiring_of(g, &b)? {
                    out.push(json!({ "id": b.id, "key": b.t.wire.map(|r| record::label(r.entry)), "wiring": w }));
                }
            }
            Ok(json!(out))
        }
        "wiring.get" => {
            let b = block(args)?;
            Ok(json!(wiring_of(g, &b)?))
        }
        // `wiring: {...}` wires the block, `wiring: null` unwires it
        "wiring.set" => {
            let b = block(args)?;
            let w = match args.get("wiring") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    Some(serde_json::from_value::<Wiring>(v.clone()).map_err(|e| format!("that isn't a wiring: {e}"))?)
                }
            };
            let b = set_wiring(g, &b, w)?;
            let mut out = super::blocks::listed(g, std::slice::from_ref(&b))?;
            out["wiring"] = json!(wiring_of(g, &b)?);
            Ok(out)
        }
        // `{id, event, from?}`: what it set off, and the blocks it changed
        "wiring.fire" => {
            let id = s(args, "id");
            let ev =
                Event::parse(s(args, "event")).ok_or("which event? (click, hit, enter, leave, timer or signal)")?;
            let r = fire(g, id, ev, s(args, "from"), s(args, "value"), shared, host)?;
            let now: Vec<Block> = r.changed.iter().filter_map(|i| g.block(i).ok().flatten()).collect();
            let mut out = super::blocks::listed(g, &now)?;
            out["events"] = json!(r.events);
            out["rays"] = json!(r.rays);
            out["says"] = json!(r.says);
            out["sounds"] = json!(r.sounds);
            out["removed"] = json!(r.removed);
            out["cut"] = json!(r.cut);
            Ok(out)
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dominos_fall_in_order() {
        let g = Graph::in_memory().unwrap();
        let rm = Default::default();
        let c = |op: &str, a: Value| {
            if super::super::blocks::routed(op) {
                super::super::blocks::call(&g, op, &a, &super::super::classify::Limits::NONE).unwrap()
            } else if routed(op) {
                call(&g, op, &a, false).unwrap()
            } else {
                super::super::call(&g, &rm, op, &a).unwrap()
            }
        };
        // a program that moves its block up and says who hit it
        c(
            "notes.save",
            json!({ "note": { "tags": ["s"] }, "text": "# lift\n\n{move 0 1 0} > args 1 who [l] {hit by ~who~} >" }),
        );
        // a counter that keeps its count
        c(
            "notes.save",
            json!({ "note": { "tags": ["s"] }, "text": "# count\n\n_ <_ _~ 1 + =~ n# {keep ~n~} > {cast} >" }),
        );
        let r = c(
            "blocks.add",
            json!({ "blocks": [
                { "kind": "plain", "texture": "#111111", "x": 0 },
                { "kind": "plain", "texture": "#222222", "z": 3 },
                { "kind": "plain", "texture": "#333333", "z": 6 },
            ] }),
        );
        let ids: Vec<String> = r["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        // the first casts on a click (no program), the last mode: through the second to the third
        c(
            "wiring.set",
            json!({ "id": ids[0], "wiring": { "on": ["click"], "ray": { "mode": "last" } } }),
        );
        c(
            "wiring.set",
            json!({ "id": ids[1], "wiring": { "on": ["hit"], "program": "lift" } }),
        );
        c(
            "wiring.set",
            json!({ "id": ids[2], "wiring": { "on": ["hit"], "program": "lift" } }),
        );
        let f = c("wiring.fire", json!({ "id": ids[0], "event": "click" }));
        let ev = f["events"].as_array().unwrap();
        assert_eq!(ev.len(), 2, "{f}");
        assert_eq!(ev[1]["block"], ids[2]);
        assert_eq!(ev[1]["out"], format!("hit by {}", ids[0]));
        assert_eq!(f["blocks"][0]["y"], 1.0);
        assert_eq!(f["rays"][0]["hits"], json!([ids[2]]));
        // the second wasn't hit (the ray registers the last)
        assert_eq!(c("blocks.list", json!({ "texture": "#222222" }))["blocks"][0]["y"], 0.0);

        // a counter keeps its state between runs, and a sticky ray goes to its targets
        c(
            "wiring.set",
            json!({ "id": ids[1], "wiring": { "on": ["click"], "program": "count", "ray": { "targets": [ids[0]] } } }),
        );
        c(
            "wiring.set",
            json!({ "id": ids[0], "wiring": { "on": ["hit"], "program": "lift" } }),
        );
        c("wiring.fire", json!({ "id": ids[1], "event": "click" }));
        let f = c("wiring.fire", json!({ "id": ids[1], "event": "click" }));
        assert_eq!(c("wiring.get", json!({ "id": ids[1] }))["state"], "2");
        assert_eq!(f["events"][1]["block"], ids[0]);
        // the third's wiring is its own: the second's change didn't touch it
        assert_eq!(c("wiring.get", json!({ "id": ids[2] }))["program"], "lift");

        // two blocks that hit each other stop after MAX_EVENTS
        c(
            "wiring.set",
            json!({ "id": ids[0], "wiring": { "on": ["click", "hit"], "ray": { "targets": [ids[1]] } } }),
        );
        c(
            "wiring.set",
            json!({ "id": ids[1], "wiring": { "on": ["hit"], "ray": { "targets": [ids[0]] } } }),
        );
        let f = c("wiring.fire", json!({ "id": ids[0], "event": "click" }));
        assert_eq!(f["cut"], true);
        assert_eq!(f["events"].as_array().unwrap().len(), MAX_EVENTS);
        assert_eq!(c("wiring.list", json!({})).as_array().unwrap().len(), 3);
    }
}
