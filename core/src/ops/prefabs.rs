//! Prefabs (`prefabs.*`, queued request #19; `graph::prefabs` keeps them): blocks grouped
//! into a subassembly, a menu of the save's prefabs to place copies from, and a copy's
//! own changes made the prefab's (every copy following, or the others kept as they were).
//!
//! Grouping keeps the links inside the group: a grouped block's sticky ray that names
//! another grouped block names it by its address inside the prefab, so each copy's ray
//! goes to its own part; a block outside the group whose ray named a grouped block names
//! that part of the new copy.

use serde_json::{Value, json};

use super::{Res, s};
use crate::graph::prefabs::{inner_ids, split_copy};
use crate::graph::record::mime;
use crate::graph::wiring::Wiring;
use crate::graph::{Graph, Reader};

pub fn routed(op: &str) -> bool {
    op.starts_with("prefabs.")
}

fn num(v: &Value, k: &str) -> Option<f32> {
    v.get(k)
        .and_then(Value::as_f64)
        .map(|x| x as f32)
        .filter(|x| x.is_finite())
}

fn rot(v: &Value) -> Option<[f32; 4]> {
    let a = v.get("rot")?.as_array()?;
    if a.len() != 4 {
        return None;
    }
    Some([
        a[0].as_f64()? as f32,
        a[1].as_f64()? as f32,
        a[2].as_f64()? as f32,
        a[3].as_f64()? as f32,
    ])
}

/// a wiring entry's wiring
fn wiring_at(g: &Graph, entry: u64) -> Option<Wiring> {
    let v = g.view().ok()?;
    let r = v.record(entry).ok()??;
    (r.mime == mime::WIRING).then(|| serde_json::from_str(&v.render(entry).ok()?).ok())?
}

/// a copy's parts as `blocks.list` has them, with the copy
fn copy_out(g: &Graph, id: &str) -> Res {
    let parts = g.copy_parts(id)?;
    let mut out = super::blocks::listed(g, &parts)?;
    out["copy"] = json!(g.copies()?.into_iter().find(|c| c.id == id));
    Ok(out)
}

// ---------------------------------------------------------------- starters (#27)

/// the s programs the starter prefabs run (notes tagged `s`), title → body. `inputs` and
/// `gate-out` are components the gates share: a gate is `inputs.`, a rule, `gate-out.`
pub const PROGRAMS: [(&str, &str); 10] = [
    (
        "switch",
        "{A switch: each click turns it on or off, tilts its paddle and tells what's ahead} //\n\
         _ <_ _~ s# 1 s~ - =~ s#\n\
         {keep ~s~} >\n\
         s~ 1 == {{turn -30 0 0} >} ~? //\n\
         s~ 0 == {{turn 30 0 0} >} ~? //\n\
         {cast} >",
    ),
    (
        "inputs",
        "{A gate's inputs: its state is its output, then each input's last value by who sent it, as who=value. It counts the inputs and how many are on, for the gate to decide} //\n\
         args 0 ev [l] args 1 who [l] args 3 v [l]\n\
         _ <_ _~ st /l\n\
         0 ones# 0 count# 0 w0@ st 0 w0 [l] w0~ was#\n\
         ins l-\n\
         {{~k~=~val~} ins l count; {~val~} 1 == ones;?} keep1$\n\
         {st n~ it [l] 0 k@ 0 val@ {~it~} kv '=' /l kv 0 k [l] kv 1 val [l] kv l- {~k~} {~who~} != keep1.?} step2$\n\
         {n~ 0 >> step2.?} step$\n\
         {{~who~=~v~} ins l count; {~v~} 1 == ones;?} add$\n\
         st #l =~ step &&&\n\
         {~ev~} hit = add.?\n\
         {~ev~} signal = add.?",
    ),
    (
        "gate-out",
        "{A gate's output: kept first in its state, shown by its colour, told ahead by its ray} //\n\
         ins line l/\n\
         {keep ~o~ ~line~} >\n\
         o~ 1 == {{colour #f5c518} >} ~? //\n\
         o~ 0 == {{colour ~base~} >} ~? //\n\
         {cast} >",
    ),
    (
        "and",
        "{An AND gate: on while two inputs are on} //\n\
         {#3d6fb6} base@\n\
         inputs.\n\
         0 o# ones~ 2 >>= {1 o#} ~? //\n\
         gate-out.",
    ),
    (
        "or",
        "{An OR gate: on while any input is on} //\n\
         {#3f9b4b} base@\n\
         inputs.\n\
         0 o# ones~ 0 >> {1 o#} ~? //\n\
         gate-out.",
    ),
    (
        "xor",
        "{An XOR gate: on while an odd number of inputs are on} //\n\
         {#8e44ad} base@\n\
         inputs.\n\
         ones~ 2 % =~ o#\n\
         gate-out.",
    ),
    (
        "not",
        "{A NOT gate: on while no input is on} //\n\
         {#c0392b} base@\n\
         inputs.\n\
         1 o# ones~ 0 >> {0 o#} ~? //\n\
         gate-out.",
    ),
    (
        "latch",
        "{A latch: it flips each time an input turns on, and holds} //\n\
         {#d35400} base@\n\
         inputs.\n\
         was~ o#\n\
         {~ev~} hit = {~v~} 1 & {1 was~ - =~ o#} ~? //\n\
         gate-out.",
    ),
    (
        "lamp",
        "{A lamp: lit while what reaches it is on} //\n\
         args 3 v [l]\n\
         {~v~} 1 = {{colour #ffd84a} >} ~? //\n\
         {~v~} 1 != {{colour #3a3a3a} >} ~? //\n\
         {keep ~v~} >",
    ),
    (
        "door",
        "{A door: it swings open while what reaches it is on, and shut when it's off} //\n\
         _ <_ _~ open# args 3 v [l]\n\
         {~v~} 1 = open~ 0 & {{turn 0 90 0} > {keep 1} >} ~? //\n\
         {~v~} 1 != open~ 1 & {{turn 0 -90 0} > {keep 0} >} ~? //",
    ),
];

/// makes the starter programs and prefabs the save lacks (by title and name): a switch,
/// AND, OR, XOR and NOT gates, a latch, a lamp and a door. Their parts sit so their
/// signal line is the copy's height: rays go out of each one's right side (+x) at y 0, so
/// copies placed in a row at one height wire themselves. What it made
fn starters(g: &Graph) -> Res {
    use crate::graph::blocks::{Kind, Shape, Transform};
    use crate::graph::wiring::{Event, Ray, WireRef};
    let removed = Default::default();
    let have = super::script::programs(g)?;
    let mut programs = Vec::new();
    for (title, body) in PROGRAMS {
        if !have.contains_key(title) {
            super::call(
                g,
                &removed,
                "notes.save",
                &json!({ "note": { "tags": [super::script::TAG] }, "text": format!("# {title}\n\n{body}") }),
            )?;
            programs.push(title);
        }
    }
    let wire = |program: &str, on: Event, ray: bool| -> Result<Option<WireRef>, String> {
        let w = Wiring {
            on: vec![on],
            program: program.into(),
            ray: ray.then(|| Ray {
                dir: [1.0, 0.0, 0.0],
                range: 20.0,
                ..Ray::default()
            }),
            ..Wiring::default()
        }
        .checked()?;
        let text = serde_json::to_string(&w).map_err(|e| e.to_string())?;
        Ok(Some(WireRef {
            entry: g.save_text(&text, mime::WIRING)?.0,
        }))
    };
    let t = |x: f32, y: f32, z: f32, w: f32, h: f32, d: f32| Transform {
        x,
        y,
        z,
        w,
        h,
        d,
        ..Transform::default()
    };
    let label =
        |text: &str| -> Result<String, String> { Ok(crate::graph::record::label(g.save_text(text, mime::TEXT)?.0)) };
    let mut made = Vec::new();
    let names: std::collections::HashSet<String> = g.prefabs()?.into_iter().map(|p| p.name).collect();
    let mut add = |name: &str, parts: Vec<(Kind, String, Transform)>| -> Result<(), String> {
        if !names.contains(name) {
            g.put_prefab(name, &parts)?;
            made.push(name.to_string());
        }
        Ok(())
    };
    let mut paddle = t(0.0, 0.0, 0.0, 0.25, 0.4, 0.6);
    paddle.wire = wire("switch", Event::Click, true)?;
    add(
        "switch",
        vec![
            (Kind::Plain, "#e8e8e8".into(), t(0.0, -0.35, 0.0, 1.2, 0.3, 1.2)),
            (Kind::Plain, "#fafafa".into(), paddle),
        ],
    )?;
    for (name, program, colour, text) in [
        ("AND gate", "and", "#3d6fb6", "AND"),
        ("OR gate", "or", "#3f9b4b", "OR"),
        ("XOR gate", "xor", "#8e44ad", "XOR"),
        ("NOT gate", "not", "#c0392b", "NOT"),
        ("latch", "latch", "#d35400", "LATCH"),
    ] {
        let mut body = t(0.0, 0.0, 0.0, 1.0, 1.0, 2.4);
        body.wire = wire(program, Event::Hit, true)?;
        add(
            name,
            vec![
                (Kind::Plain, colour.into(), body),
                (Kind::Text, label(text)?, t(0.0, 0.56, 0.0, 1.0, 0.12, 2.4)),
            ],
        )?;
    }
    let mut bulb = t(0.0, 0.0, 0.0, 0.8, 0.8, 0.8);
    bulb.shape = Shape::Sphere;
    bulb.wire = wire("lamp", Event::Hit, false)?;
    add(
        "lamp",
        vec![
            (Kind::Plain, "#3a3a3a".into(), bulb),
            (Kind::Plain, "#777777".into(), t(0.0, -0.6, 0.0, 0.2, 0.4, 0.2)),
        ],
    )?;
    let mut panel = t(0.0, 0.5, 0.0, 0.2, 2.0, 1.6);
    panel.wire = wire("door", Event::Hit, false)?;
    add("door", vec![(Kind::Plain, "#8b5a2b".into(), panel)])?;
    Ok(json!({ "programs": programs, "prefabs": made }))
}

pub fn call(g: &Graph, op: &str, args: &Value) -> Res {
    match op {
        // the starter programs and prefabs the save lacks (#27)
        "prefabs.starters" => starters(g),
        // the prefabs, each with its parts (for a thumbnail) and how many copies stand
        "prefabs.list" => {
            let copies = g.copies()?;
            let mut out = Vec::new();
            let mut all = Vec::new();
            for p in g.prefabs()? {
                let parts = g.prefab_parts(p.id)?;
                all.extend(parts.iter().cloned());
                out.push(json!({
                    "id": p.id, "name": p.name, "created": p.created, "edited": p.edited,
                    "copies": copies.iter().filter(|c| c.scene == p.id).count(),
                    "parts": parts,
                }));
            }
            let mut l = super::blocks::listed(g, &all)?;
            l["prefabs"] = json!(out);
            if let Some(o) = l.as_object_mut() {
                o.remove("blocks");
            }
            Ok(l)
        }
        "prefabs.copies" => Ok(json!(g.copies()?)),
        // `{name, ids}`: the blocks grouped; the prefab, its copy where they were, and the
        // addresses that went
        "prefabs.make" => {
            let ids: Vec<String> = args
                .get("ids")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            let name = s(args, "name").trim().to_string();
            if name.is_empty() {
                return Err("a prefab needs a name".into());
            }
            let mut bs = Vec::new();
            for id in &ids {
                bs.push(g.block(id)?.ok_or_else(|| format!("no block {id}"))?);
            }
            let inner = inner_ids(&bs.iter().map(|b| (b.kind, b.texture.clone())).collect::<Vec<_>>());
            let to_inner = |t: &String| ids.iter().position(|i| i == t).map(|k| inner[k].clone());
            // each grouped block's wiring, its links inside the group by inner address
            let mut wires = Vec::new();
            for b in &bs {
                let Some(r) = b.t.wire else {
                    wires.push(None);
                    continue;
                };
                match wiring_at(g, r.entry) {
                    Some(mut w) => {
                        if let Some(ray) = w.ray.as_mut()
                            && ray.targets.iter().any(|t| ids.contains(t))
                        {
                            ray.targets = ray
                                .targets
                                .iter()
                                .map(|t| to_inner(t).unwrap_or_else(|| t.clone()))
                                .collect();
                            let text = serde_json::to_string(&w).map_err(|e| e.to_string())?;
                            wires.push(Some(g.save_text(&text, mime::WIRING)?.0));
                        } else {
                            wires.push(Some(r.entry));
                        }
                    }
                    None => wires.push(None),
                }
            }
            let (scene, copy) = g.make_prefab(&name, &ids, &wires)?;
            // blocks outside the group that aimed at one of its blocks aim at that part now
            for b in g.blocks(None)? {
                let Some(r) = b.t.wire else { continue };
                let Some(mut w) = wiring_at(g, r.entry) else { continue };
                let Some(ray) = w.ray.as_mut() else { continue };
                if !ray.targets.iter().any(|t| ids.contains(t)) {
                    continue;
                }
                ray.targets = ray
                    .targets
                    .iter()
                    .map(|t| to_inner(t).map(|i| format!("{copy}/{i}")).unwrap_or_else(|| t.clone()))
                    .collect();
                super::wiring::set_wiring(g, &b, Some(w))?;
            }
            let mut out = copy_out(g, &copy)?;
            out["prefab"] = json!(scene);
            out["removed"] = json!(ids);
            Ok(out)
        }
        // `{prefab, x, y, z, rot?, scale?}`: a new copy; its parts
        "prefabs.place" => {
            let scene = args
                .get("prefab")
                .and_then(Value::as_u64)
                .ok_or("which prefab? (prefab)")?;
            let at = [
                num(args, "x").unwrap_or(0.0),
                num(args, "y").unwrap_or(0.0),
                num(args, "z").unwrap_or(0.0),
            ];
            let id = g.add_copy(
                scene,
                at,
                rot(args).unwrap_or(crate::graph::place::IDENTITY),
                num(args, "scale").unwrap_or(1.0),
            )?;
            copy_out(g, &id)
        }
        // `{id, x?, y?, z?, by?, rot?, turn?, scale?}`: a copy moved, turned (`turn`: a
        // quarter turn about the upright, so many times) or sized; its parts
        "prefabs.set" => {
            let id = s(args, "id");
            let c = g
                .copies()?
                .into_iter()
                .find(|c| c.id == id)
                .ok_or_else(|| format!("no copy {id}"))?;
            let mut at = [
                num(args, "x").unwrap_or(c.x),
                num(args, "y").unwrap_or(c.y),
                num(args, "z").unwrap_or(c.z),
            ];
            if let Some(by) = args.get("by").and_then(Value::as_array).filter(|a| a.len() == 3) {
                for (i, v) in by.iter().enumerate() {
                    at[i] += v.as_f64().unwrap_or(0.0) as f32;
                }
            }
            let mut q = rot(args).unwrap_or(c.rot);
            if let Some(n) = args.get("turn").and_then(Value::as_i64) {
                let a = (n as f32) * std::f32::consts::FRAC_PI_4;
                let t = [0.0, a.sin(), 0.0, a.cos()];
                let [ax, ay, az, aw] = t;
                let [bx, by, bz, bw] = q;
                q = [
                    aw * bx + ax * bw + ay * bz - az * by,
                    aw * by - ax * bz + ay * bw + az * bx,
                    aw * bz + ax * by - ay * bx + az * bw,
                    aw * bw - ax * bx - ay * by - az * bz,
                ];
            }
            g.set_copy(id, at, q, num(args, "scale").unwrap_or(c.scale))?;
            copy_out(g, id)
        }
        // `{id}`: the copy taken away; its parts' addresses
        "prefabs.remove" => {
            let id = s(args, "id");
            let parts: Vec<String> = g.copy_parts(id)?.into_iter().map(|b| b.id).collect();
            g.delete_copy(id)?;
            Ok(json!({ "removed": parts }))
        }
        // `{id}`: the copy shows the prefab as it is again
        "prefabs.reset" => {
            let id = s(args, "id");
            g.reset_copy(id)?;
            copy_out(g, id)
        }
        // `{id, others}`: the copy's own changes made the prefab's; `others: true` every
        // other copy follows, false they keep the prefab as it was (a prefab of its own)
        "prefabs.apply" => {
            let id = s(args, "id");
            let follow = args.get("others").and_then(Value::as_bool).unwrap_or(true);
            let kept = g.apply_copy(id, follow)?;
            // every copy's parts, since others may have changed
            let mut all = Vec::new();
            for c in g.copies()? {
                all.extend(g.copy_parts(&c.id)?);
            }
            let mut out = super::blocks::listed(g, &all)?;
            out["kept"] = json!(kept);
            out["copies"] = json!(g.copies()?);
            Ok(out)
        }
        // `{id}`: the copy as ordinary blocks; the new blocks, and the parts' addresses
        // that went
        "prefabs.detach" => {
            let id = s(args, "id");
            split_copy(id).ok_or_else(|| format!("{id:?} isn't a copy"))?;
            // the parts' own wirings (a counter's state) come along
            let mut own = std::collections::HashMap::new();
            for b in g.copy_parts(id)? {
                if let Some(w) = g
                    .part_wiring(&b.id)?
                    .and_then(|v| serde_json::from_value::<Wiring>(v).ok())
                {
                    own.insert(b.id, w);
                }
            }
            let made = g.detach_copy(id)?;
            let back: std::collections::HashMap<&String, &String> = made.iter().map(|(p, n)| (n, p)).collect();
            // links inside the copy follow its parts to the new blocks
            let map: std::collections::HashMap<String, String> = made.iter().cloned().collect();
            let inner_map: std::collections::HashMap<String, String> = made
                .iter()
                .filter_map(|(part, new)| part.split_once('/').map(|(_, i)| (i.to_string(), new.clone())))
                .collect();
            for (_, new) in &made {
                let Some(b) = g.block(new)? else { continue };
                let mine = back.get(new).and_then(|p| own.get(*p)).cloned();
                let had_own = mine.is_some();
                let Some(mut w) = mine.or_else(|| b.t.wire.and_then(|r| wiring_at(g, r.entry))) else {
                    continue;
                };
                let Some(ray) = w.ray.as_mut() else {
                    if had_own {
                        super::wiring::set_wiring(g, &b, Some(w))?;
                    }
                    continue;
                };
                if had_own
                    || ray
                        .targets
                        .iter()
                        .any(|t| inner_map.contains_key(t) || map.contains_key(t))
                {
                    ray.targets = ray
                        .targets
                        .iter()
                        .map(|t| {
                            inner_map
                                .get(t)
                                .or_else(|| map.get(t))
                                .cloned()
                                .unwrap_or_else(|| t.clone())
                        })
                        .collect();
                    super::wiring::set_wiring(g, &b, Some(w))?;
                }
            }
            let news: Vec<crate::graph::blocks::Block> =
                made.iter().filter_map(|(_, n)| g.block(n).ok().flatten()).collect();
            let mut out = super::blocks::listed(g, &news)?;
            out["ids"] = json!(made.iter().map(|m| &m.1).collect::<Vec<_>>());
            out["removed"] = json!(made.iter().map(|m| &m.0).collect::<Vec<_>>());
            Ok(out)
        }
        "prefabs.rename" => {
            let id = args
                .get("prefab")
                .and_then(Value::as_u64)
                .ok_or("which prefab? (prefab)")?;
            if !g.prefabs()?.iter().any(|p| p.id == id) {
                return Err(format!("no prefab {id}"));
            }
            g.rename_scene(id, s(args, "name"))?;
            Ok(json!({ "renamed": id }))
        }
        // `{prefab}`: it goes with its copies; the parts' addresses that went
        "prefabs.delete" => {
            let id = args
                .get("prefab")
                .and_then(Value::as_u64)
                .ok_or("which prefab? (prefab)")?;
            let mut parts = Vec::new();
            for c in g.copies()?.into_iter().filter(|c| c.scene == id) {
                parts.extend(g.copy_parts(&c.id)?.into_iter().map(|b| b.id));
            }
            let copies = g.delete_prefab(id)?;
            Ok(json!({ "copies": copies, "removed": parts }))
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping_keeps_links_inside_each_copy() {
        let g = Graph::in_memory().unwrap();
        let rm = Default::default();
        let c = |op: &str, a: Value| -> Value {
            if super::super::blocks::routed(op) {
                super::super::blocks::call(&g, op, &a, &super::super::classify::Limits::NONE).unwrap()
            } else if super::super::wiring::routed(op) {
                super::super::wiring::call(&g, op, &a, false).unwrap()
            } else if routed(op) {
                call(&g, op, &a).unwrap()
            } else {
                super::super::call(&g, &rm, op, &a).unwrap()
            }
        };
        c(
            "notes.save",
            json!({ "note": { "tags": ["s"] }, "text": "# lift\n\n{move 0 1 0} >" }),
        );
        let r = c(
            "blocks.add",
            json!({ "blocks": [
                { "kind": "plain", "texture": "#aa0000", "x": 0 },
                { "kind": "plain", "texture": "#00aa00", "x": 3 },
                { "kind": "plain", "texture": "#0000aa", "x": 30 },
            ] }),
        );
        let ids: Vec<String> = r["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        // a switch aims at its lamp; the lamp lifts when hit
        c(
            "wiring.set",
            json!({ "id": ids[0], "wiring": { "on": ["click"], "ray": { "targets": [ids[1]] } } }),
        );
        c(
            "wiring.set",
            json!({ "id": ids[1], "wiring": { "on": ["hit"], "program": "lift" } }),
        );
        // a block outside aims at the lamp too
        c(
            "wiring.set",
            json!({ "id": ids[2], "wiring": { "on": ["click"], "ray": { "targets": [ids[1]] } } }),
        );
        let m = c(
            "prefabs.make",
            json!({ "name": "switch and lamp", "ids": [ids[0], ids[1]] }),
        );
        let copy = m["copy"]["id"].as_str().unwrap().to_string();
        let scene = m["prefab"].as_u64().unwrap();
        assert_eq!(m["blocks"].as_array().unwrap().len(), 2);
        assert_eq!(m["blocks"][0]["part_of"], copy);
        // a second copy far away
        let two = c("prefabs.place", json!({ "prefab": scene, "x": 0, "y": 0, "z": 50 }));
        let two_id = two["copy"]["id"].as_str().unwrap().to_string();
        // clicking the second copy's switch lifts the second copy's lamp only
        let f = c(
            "wiring.fire",
            json!({ "id": format!("{two_id}/plain:#aa0000#1"), "event": "click" }),
        );
        assert_eq!(f["events"][1]["block"], format!("{two_id}/plain:#00aa00#1"), "{f}");
        let lamp = |copy: &str| {
            c("blocks.list", json!({}))["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["id"] == format!("{copy}/plain:#00aa00#1"))
                .unwrap()["y"]
                .as_f64()
                .unwrap()
        };
        assert_eq!(lamp(&two_id), 1.0);
        assert_eq!(lamp(&copy), 0.0);
        // the outside block aims at the first copy's lamp
        c("wiring.fire", json!({ "id": ids[2], "event": "click" }));
        assert_eq!(lamp(&copy), 1.0);
        let l = c("prefabs.list", json!({}));
        assert_eq!(l["prefabs"][0]["copies"], 2);
        // detaching the second copy keeps its switch aimed at its own lamp
        let d = c("prefabs.detach", json!({ "id": two_id }));
        let new: Vec<String> = d["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let f = c(
            "wiring.fire",
            json!({ "id": new.iter().find(|i| i.contains("aa0000")).unwrap(), "event": "click" }),
        );
        assert_eq!(f["events"].as_array().unwrap().len(), 2, "{f}");
        // a merge brings the prefab and its copy, the link inside still working
        let other = Graph::in_memory().unwrap();
        let w = super::super::merge::assets(&other, &g).unwrap();
        assert_eq!((w.prefabs, w.copies), (1, 1));
        let oc = other.copies().unwrap()[0].id.clone();
        let f = super::super::wiring::call(
            &other,
            "wiring.fire",
            &json!({ "id": format!("{oc}/plain:#aa0000#1"), "event": "click" }),
            false,
        )
        .unwrap();
        assert_eq!(f["events"].as_array().unwrap().len(), 2, "{f}");
        assert_eq!(c("prefabs.delete", json!({ "prefab": scene }))["copies"], 1);
    }

    #[test]
    fn switches_through_gates_light_a_lamp() {
        let g = Graph::in_memory().unwrap();
        let c = |op: &str, a: Value| -> Value {
            if super::super::wiring::routed(op) {
                super::super::wiring::call(&g, op, &a, false).unwrap()
            } else {
                call(&g, op, &a).unwrap()
            }
        };
        let made = c("prefabs.starters", json!({}));
        assert_eq!(made["prefabs"].as_array().unwrap().len(), 8, "{made}");
        assert_eq!(made["programs"].as_array().unwrap().len(), PROGRAMS.len());
        // a second time makes nothing
        assert_eq!(c("prefabs.starters", json!({}))["prefabs"], json!([]));
        let id = |name: &str| g.prefabs().unwrap().into_iter().find(|p| p.name == name).unwrap().id;
        let place = |name: &str, x: f32, z: f32| {
            c("prefabs.place", json!({ "prefab": id(name), "x": x, "y": 0, "z": z }))["copy"]["id"]
                .as_str()
                .unwrap()
                .to_string()
        };
        // two switches, side by side, into an AND gate, into a lamp
        let a = place("switch", 0.0, -0.8);
        let b = place("switch", 0.0, 0.8);
        let and = place("AND gate", 3.0, 0.0);
        let lamp = place("lamp", 6.0, 0.0);
        let paddle = |s: &str| format!("{s}/plain:#fafafa#1");
        let bulb = format!("{lamp}/plain:#3a3a3a#1");
        let lit = || g.part(&bulb).unwrap().unwrap().texture;
        let click = |s: &str| {
            let f = c("wiring.fire", json!({ "id": paddle(s), "event": "click" }));
            for e in f["events"].as_array().unwrap() {
                assert!(e.get("err").is_none(), "{f}");
            }
            f
        };
        let f = click(&a);
        assert_eq!(f["events"].as_array().unwrap().len(), 3, "{f}");
        assert_eq!(lit(), "#3a3a3a", "one switch on: the AND stays off");
        click(&b);
        assert_eq!(lit(), "#ffd84a", "both on: lit");
        click(&a);
        assert_eq!(lit(), "#3a3a3a", "one off again: dark");
        // the gate shows its output by its colour, and keeps its inputs
        let state = c("wiring.get", json!({ "id": format!("{and}/plain:#3d6fb6#1") }))["state"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(state.starts_with("0 "), "{state}");
        assert_eq!(state.matches('=').count(), 2, "{state}");
        // a NOT gate opens a door while nothing is on
        let not = place("NOT gate", 3.0, 10.0);
        let door = place("door", 6.0, 10.0);
        let s = place("switch", 0.0, 10.0);
        click(&s);
        let panel = |s: &str| g.part(&format!("{s}/plain:#8b5a2b#1")).unwrap().unwrap().t.rot;
        assert!(panel(&door)[3] > 0.99, "switch on: NOT off: the door stays shut");
        click(&s);
        assert!(panel(&door)[3] < 0.8, "switch off: NOT on: the door swings open");
        let _ = not;
        // OR, XOR and the latch, each between two switches and a lamp: the lamp after
        // a on, b on, a off
        for (k, (gate, want)) in [
            ("OR gate", ["#ffd84a", "#ffd84a", "#ffd84a"]),
            ("XOR gate", ["#ffd84a", "#3a3a3a", "#ffd84a"]),
            ("latch", ["#ffd84a", "#3a3a3a", "#3a3a3a"]),
        ]
        .into_iter()
        .enumerate()
        {
            let z = 20.0 + 10.0 * k as f32;
            let a = place("switch", 0.0, z - 0.8);
            let b = place("switch", 0.0, z + 0.8);
            place(gate, 3.0, z);
            let lamp = place("lamp", 6.0, z);
            let bulb = format!("{lamp}/plain:#3a3a3a#1");
            let lit = || g.part(&bulb).unwrap().unwrap().texture;
            let mut got = Vec::new();
            for s in [&a, &b, &a] {
                click(s);
                got.push(lit());
            }
            assert_eq!(got, want, "{gate}");
        }
    }
}
