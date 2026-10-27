//! The app's scene operations (`scenes.*`, docs/app-api.md): the saved scenes of a save
//! (`graph::scenes`), read and written in the rows the space keeps,
//! `[text, x, y, z, {key?, size?}]`. The room (the space's own scene, with its blocks) is
//! listed but not among the saved ones.

use serde_json::{Map, Value, json};

use super::{Res, is_note, s};
use crate::graph::record;
use crate::graph::scenes::{Def, Kind, size_of};
use crate::graph::settle::{Body, settle, text_body};
use crate::graph::{Graph, Reader};

/// whether a scene is a prefab (`graph::prefabs`)
fn is_prefab(s: &crate::graph::scenes::Scene) -> bool {
    s.settings.get("prefab") == Some(&Value::Bool(true))
}

pub fn routed(op: &str) -> bool {
    op.starts_with("scenes.")
}

/// a scene's name: 1 to 96 characters
fn name_arg(args: &Value, k: &str) -> Result<String, String> {
    let n = s(args, k).trim().to_string();
    if n.is_empty() || n.chars().count() > 96 {
        return Err("a scene's name: 1 to 96 characters".into());
    }
    Ok(n)
}

/// a scene's text placements as the space's rows, `[text, x, y, z, {key, size?,
/// detached?, own?}]`: the inheriting ones in (entry, number) order showing their entry's
/// text now (or their own text, `own`), then the detached ones showing the text as it read
/// when they were detached (`detached`: that version's edited time, ms)
pub fn rows(g: &Graph, scene: u64) -> Result<Vec<Value>, String> {
    let v = g.view()?;
    let mut out = Vec::new();
    for detached in [false, true] {
        for p in g.placements(scene, detached)? {
            let (Kind::Text, Def::Entry(id)) = (p.kind, p.def) else {
                continue;
            };
            let text = match p.version {
                Some(at) => g.text_at(id, at),
                None => v.render(id),
            };
            let Ok(text) = text else { continue };
            let note = is_note(&v, id);
            let shown = if note { crate::note::split(&text).0 } else { text };
            let mut o = json!({ "key": if note { format!("note:{}", record::label(id)) } else { record::label(id) } });
            if let Some(sz) = size_of(p.place.font) {
                o["size"] = json!(sz);
            }
            if let Some(at) = p.version {
                o["detached"] = json!(at);
            }
            let own = p.place.own_text();
            if let Some(t) = &own {
                o["own"] = json!(t);
            }
            let [x, y, z] = p.place.pos;
            out.push(json!([own.unwrap_or(shown), x, y, z, o]));
        }
    }
    Ok(out)
}

pub fn call(g: &Graph, op: &str, args: &Value) -> Res {
    let room = g.room()?;
    match op {
        "scenes.list" => {
            let mut out = Vec::new();
            // prefabs are listed by `prefabs.list`
            for sc in g.scenes()?.into_iter().filter(|s| !is_prefab(s)) {
                let ps = [g.placements(sc.id, false)?, g.placements(sc.id, true)?].concat();
                let texts = ps.iter().filter(|p| p.kind == Kind::Text).count();
                let blocks = ps.iter().filter(|p| p.kind == Kind::Block).count();
                out.push(json!({
                    "id": sc.id, "name": sc.name, "parent": sc.parent, "room": Some(sc.id) == room,
                    "texts": texts, "blocks": blocks, "created": sc.created, "edited": sc.edited,
                }));
            }
            Ok(Value::Array(out))
        }
        // every saved scene by name, as rows (the scenes window, `#name`)
        "scenes.maps" => {
            let mut m = Map::new();
            for sc in g.scenes()?.into_iter().filter(|s| !is_prefab(s)) {
                if Some(sc.id) != room {
                    m.insert(sc.name.clone(), Value::Array(rows(g, sc.id)?));
                }
            }
            Ok(Value::Object(m))
        }
        "scenes.get" => {
            let n = name_arg(args, "name")?;
            let sc = g
                .scene_named(&n)?
                .ok_or_else(|| format!("there's no scene named {n}"))?;
            Ok(json!({ "id": sc.id, "name": sc.name, "rows": rows(g, sc.id)? }))
        }
        // the room's texts saved under a name, replacing the texts of a scene so named
        "scenes.save" => {
            let n = name_arg(args, "name")?;
            let given = args.get("rows").and_then(Value::as_array).cloned().unwrap_or_default();
            if given.len() > 100_000 {
                return Err("at most 100 000 texts in a scene at once".into());
            }
            let id = match g.scene_named(&n)? {
                Some(sc) if Some(sc.id) != room => sc.id,
                Some(_) => return Err("the room keeps its own name".into()),
                None => g.add_scene(&n, 0)?,
            };
            let texts = g.rows_to_texts(&given)?;
            let count: usize = texts.iter().map(|t| t.places.len()).sum();
            g.set_texts(id, texts)?;
            Ok(json!({ "id": id, "name": n, "texts": count }))
        }
        "scenes.rename" => {
            let (from, to) = (name_arg(args, "from")?, name_arg(args, "to")?);
            if g.scene_named(&to)?.is_some() {
                return Err(format!("there's a scene {to} already"));
            }
            let sc = g
                .scene_named(&from)?
                .ok_or_else(|| format!("there's no scene named {from}"))?;
            g.rename_scene(sc.id, &to)?;
            Ok(json!({ "id": sc.id, "name": to }))
        }
        // takes a saved scene away; answers its rows, which `scenes.save` puts back
        "scenes.delete" => {
            let n = name_arg(args, "name")?;
            let sc = g
                .scene_named(&n)?
                .ok_or_else(|| format!("there's no scene named {n}"))?;
            if Some(sc.id) == room {
                return Err("the room can't be deleted".into());
            }
            let rows = rows(g, sc.id)?;
            g.delete_scene(sc.id)?;
            Ok(json!({ "name": n, "rows": rows }))
        }
        // scene `from`'s placements copied into scene `into` (the room by default), settled
        // out of what `into` has and out of each other (answer 5); `from` stays as it was
        "scenes.combine" => {
            let from = name_arg(args, "from")?;
            let src = g
                .scene_named(&from)?
                .ok_or_else(|| format!("there's no scene named {from}"))?;
            let into = match s(args, "into").trim() {
                "" => room.ok_or("the room has no scene yet: save it first")?,
                n => {
                    g.scene_named(n)?
                        .ok_or_else(|| format!("there's no scene named {n}"))?
                        .id
                }
            };
            if into == src.id {
                return Err("a scene can't be combined with itself".into());
            }
            let (placed, settled) = combine(g, src.id, into)?;
            Ok(json!({ "from": from, "into": into, "placements": placed, "settled": settled }))
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}

/// a placement's box for settling: a text's from its length and size, a block's from
/// its sides, turn and bend
fn body_of(v: &impl Reader, p: &crate::graph::scenes::Placed) -> Body {
    match p.kind {
        Kind::Block => {
            let t = crate::graph::blocks::transform_of(&p.place);
            Body {
                at: p.place.pos,
                half: t.half_extent(),
            }
        }
        _ => {
            let chars = match p.def {
                Def::Entry(id) => v.render(id).map(|t| t.chars().count().min(200)).unwrap_or(10),
                _ => 10,
            };
            text_body(p.place.pos, chars, size_of(p.place.font).unwrap_or(0.5) as f32)
        }
    }
}

/// copies `from`'s placements into `into`, settled; (how many, how many moved)
pub fn combine(g: &Graph, from: u64, into: u64) -> Result<(usize, usize), String> {
    let v = g.view()?;
    let have = [g.placements(into, false)?, g.placements(into, true)?].concat();
    let mut came = [g.placements(from, false)?, g.placements(from, true)?].concat();
    let fixed: Vec<Body> = have.iter().map(|p| body_of(&v, p)).collect();
    let mut bodies: Vec<Body> = came.iter().map(|p| body_of(&v, p)).collect();
    drop(v);
    let settled = settle(&fixed, &mut bodies);
    for (p, b) in came.iter_mut().zip(&bodies) {
        p.place.pos = b.at;
        // new numbers in `into`
        p.place.n = 0;
    }
    // one write per (kind, definition, version)
    let mut groups: Vec<(Kind, Def, Option<u64>, Vec<crate::graph::scenes::Placement>)> = Vec::new();
    for p in &came {
        match groups
            .iter_mut()
            .find(|x| x.0 == p.kind && x.1 == p.def && x.2 == p.version)
        {
            Some(x) => x.3.push(p.place.clone()),
            None => groups.push((p.kind, p.def, p.version, vec![p.place.clone()])),
        }
    }
    for (kind, def, version, places) in groups {
        g.place(into, version.is_some(), kind, def, version, places)?;
    }
    Ok((came.len(), settled))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_through_the_api() {
        let g = Graph::in_memory().unwrap();
        let (id, _) = g.save_text("listen", record::mime::TEXT).unwrap();
        let key = record::label(id);
        let c = |op: &str, a: Value| call(&g, op, &a);
        assert_eq!(c("scenes.maps", json!({})).unwrap(), json!({}));
        let rows = json!([
            ["listen", 1.5, -2, 0.25, { "key": key }],
            ["listen", 4, 0, 0, { "key": key, "size": 0.8 }],
            ["a text only the room had", 0, 1, 0, {}],
        ]);
        let saved = c("scenes.save", json!({ "name": "first", "rows": rows })).unwrap();
        assert_eq!(saved["texts"], 3);
        let m = c("scenes.maps", json!({})).unwrap();
        let back = m["first"].as_array().unwrap();
        assert_eq!(back.len(), 3);
        // the keyless text is a memory entry now, and the copies keep their places and size
        assert!(back.iter().all(|r| r[4]["key"].is_string()));
        assert_eq!(back[0], json!(["listen", 1.5, -2.0, 0.25, { "key": key }]));
        assert_eq!(back[1][4]["size"], 0.8);
        assert_eq!(
            g.view()
                .unwrap()
                .find_text("a text only the room had", None)
                .unwrap()
                .len(),
            1
        );
        // saving again replaces; rename; delete and put back
        c(
            "scenes.save",
            json!({ "name": "first", "rows": [["listen", 0, 0, 0, { "key": key }]] }),
        )
        .unwrap();
        assert_eq!(
            c("scenes.get", json!({ "name": "first" })).unwrap()["rows"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        c("scenes.save", json!({ "name": "second", "rows": [] })).unwrap();
        assert!(c("scenes.rename", json!({ "from": "first", "to": "second" })).is_err());
        c("scenes.rename", json!({ "from": "first", "to": "third" })).unwrap();
        let gone = c("scenes.delete", json!({ "name": "third" })).unwrap();
        assert!(c("scenes.get", json!({ "name": "third" })).is_err());
        c("scenes.save", json!({ "name": "third", "rows": gone["rows"] })).unwrap();
        assert_eq!(
            c("scenes.get", json!({ "name": "third" })).unwrap()["rows"],
            gone["rows"]
        );
        let list = c("scenes.list", json!({})).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 2);
    }

    #[test]
    fn scenes_travel_with_backups_and_merges() {
        let g = Graph::in_memory().unwrap();
        let (id, _) = g.save_text("listen", record::mime::TEXT).unwrap();
        let key = record::label(id);
        let rows = json!([["listen", 1, 2, 3, { "key": key, "size": 0.6 }], ["listen", -1, 0, 0, { "key": key }]]);
        call(&g, "scenes.save", &json!({ "name": "here", "rows": rows })).unwrap();
        g.add_block(crate::graph::blocks::Kind::Plain, "#123456", Default::default())
            .unwrap();
        let (texts, other) = g.backup().unwrap();
        let back = Graph::in_memory().unwrap();
        back.restore(&texts, &other).unwrap();
        assert_eq!(
            call(&back, "scenes.maps", &json!({})).unwrap(),
            call(&g, "scenes.maps", &json!({})).unwrap()
        );
        assert_eq!(back.blocks(None).unwrap(), g.blocks(None).unwrap());
        assert_eq!(back.instance_rows().unwrap(), g.instance_rows().unwrap());

        // a merge brings the scene in whole, beside the target's own of that name
        let target = Graph::in_memory().unwrap();
        target.save_text("other", record::mime::TEXT).unwrap();
        call(&target, "scenes.save", &json!({ "name": "here", "rows": [] })).unwrap();
        let v = g.view().unwrap();
        target.write(|tx| crate::ops::merge::merge(tx, &v)).unwrap();
        let w = crate::ops::merge::assets(&target, &g).unwrap();
        assert_eq!(w.scenes, 1);
        let m = call(&target, "scenes.maps", &json!({})).unwrap();
        assert_eq!(m["here"], json!([]));
        assert_eq!(m["here 2"].as_array().unwrap().len(), 2);
        assert_eq!(m["here 2"][0][0], "listen");
    }

    #[test]
    fn copies_inherit_or_are_detached() {
        let g = Graph::in_memory().unwrap();
        let (id, _) = g.save_text("a ripe plum", record::mime::TEXT).unwrap();
        let key = record::label(id);
        let then = g.view().unwrap().record(id).unwrap().unwrap().edited;
        let c = |op: &str, a: Value| call(&g, op, &a);
        // the room's rows: the first placement, an inheriting copy (its node key
        // `<label>~2`), one with its own text, and one detached at the version as now
        let rows = json!([
            ["a ripe plum", 0, 0, 0, { "key": key }],
            ["a ripe plum", 1, 0, 0, { "key": format!("{key}~2") }],
            ["my plum", 2, 0, 0, { "key": format!("{key}~3"), "own": "my plum" }],
            ["a ripe plum", 3, 0, 0, { "key": format!("{key}~4"), "detached": then }],
        ]);
        assert_eq!(
            c("scenes.save", json!({ "name": "orchard", "rows": rows })).unwrap()["texts"],
            4
        );
        let sc = g.scene_named("orchard").unwrap().unwrap();
        assert_eq!(g.placements(sc.id, false).unwrap().len(), 3, "three inherit");
        assert_eq!(g.placements(sc.id, true).unwrap().len(), 1, "one is detached");
        // the entry is edited: the inheriting copies follow, the own text and the detached
        // copy don't
        std::thread::sleep(std::time::Duration::from_millis(3));
        g.write(|tx| tx.edit_text(id, "a dried prune")).unwrap();
        let back = c("scenes.get", json!({ "name": "orchard" })).unwrap()["rows"].clone();
        let shown: Vec<&str> = back
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[0].as_str().unwrap())
            .collect();
        assert_eq!(shown, ["a dried prune", "a dried prune", "my plum", "a ripe plum"]);
        assert_eq!(back[2][4]["own"], "my plum");
        assert_eq!(back[3][4]["detached"], then);
        assert!(back.as_array().unwrap().iter().all(|r| r[4]["key"] == key), "one entry");
        // saved again as loaded, nothing changes
        c("scenes.save", json!({ "name": "orchard", "rows": back })).unwrap();
        assert_eq!(c("scenes.get", json!({ "name": "orchard" })).unwrap()["rows"], back);
        // backups keep both tables
        let (texts, other) = g.backup().unwrap();
        let restored = Graph::in_memory().unwrap();
        restored.restore(&texts, &other).unwrap();
        assert_eq!(
            call(&restored, "scenes.get", &json!({ "name": "orchard" })).unwrap()["rows"],
            back
        );
        // a merge brings the detached copy in as the text it shows
        let target = Graph::in_memory().unwrap();
        let v = g.view().unwrap();
        target.write(|tx| crate::ops::merge::merge(tx, &v)).unwrap();
        crate::ops::merge::assets(&target, &g).unwrap();
        let m = call(&target, "scenes.get", &json!({ "name": "orchard" })).unwrap()["rows"].clone();
        let shown: Vec<&str> = m.as_array().unwrap().iter().map(|r| r[0].as_str().unwrap()).collect();
        assert_eq!(shown, ["a dried prune", "a dried prune", "my plum", "a ripe plum"]);
        assert!(m[3][4]["detached"].is_u64());
    }

    #[test]
    fn combined_scenes_settle_their_overlaps() {
        let g = Graph::in_memory().unwrap();
        let (id, _) = g.save_text("plum", record::mime::TEXT).unwrap();
        let key = record::label(id);
        let c = |op: &str, a: Value| call(&g, op, &a);
        c(
            "scenes.save",
            json!({ "name": "a", "rows": [["plum", 0, 0, 0, { "key": key }]] }),
        )
        .unwrap();
        c(
            "scenes.save",
            json!({ "name": "b", "rows": [["plum", 0, 0, 0, { "key": key }], ["far", 50, 0, 0, {}]] }),
        )
        .unwrap();
        let r = c("scenes.combine", json!({ "from": "b", "into": "a" })).unwrap();
        assert_eq!(r["placements"], 2);
        assert_eq!(r["settled"], 1, "the one on top of a's moved; the far one didn't");
        let rows = c("scenes.get", json!({ "name": "a" })).unwrap()["rows"].clone();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (rows[0][1].as_f64(), rows[0][2].as_f64()),
            (Some(0.0), Some(0.0)),
            "a's own stays put"
        );
        let moved = rows.iter().filter(|r| r[0] == "plum").nth(1).unwrap();
        assert!(moved[1].as_f64() != Some(0.0) || moved[2].as_f64() != Some(0.0));
        // b is unchanged; a scene can't combine with itself
        assert_eq!(
            c("scenes.get", json!({ "name": "b" })).unwrap()["rows"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(c("scenes.combine", json!({ "from": "a", "into": "a" })).is_err());
    }

    #[test]
    fn a_merge_settles_blocks_that_land_on_the_rooms_own() {
        use crate::graph::blocks::{Kind as BKind, Transform};
        let target = Graph::in_memory().unwrap();
        target.add_block(BKind::Plain, "#112233", Transform::default()).unwrap();
        let src = Graph::in_memory().unwrap();
        src.add_block(BKind::Plain, "#445566", Transform::default()).unwrap();
        src.add_block(
            BKind::Plain,
            "#445566",
            Transform {
                x: 40.0,
                ..Transform::default()
            },
        )
        .unwrap();
        let w = crate::ops::merge::assets(&target, &src).unwrap();
        assert_eq!((w.blocks, w.settled), (2, 1));
        let bs = target.blocks(None).unwrap();
        let mine = bs.iter().find(|b| b.texture == "#112233").unwrap();
        assert_eq!((mine.t.x, mine.t.y, mine.t.z), (0.0, 0.0, 0.0), "the room's own stays");
        let came = bs.iter().find(|b| b.texture == "#445566" && b.t.x < 20.0).unwrap();
        assert!(
            came.t.x.abs() >= 1.0 || came.t.y.abs() >= 1.0 || came.t.z.abs() >= 1.0,
            "{:?}",
            came.t
        );
    }

    #[test]
    fn old_saved_scenes_become_scenes_when_the_save_opens() {
        let dir = std::env::temp_dir().join(format!("anend-oldmaps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("old.redb");
        let old = json!({
            "corner": [["fox", 1, 2, 3, {}], ["fox", 4, 5, 6, { "size": 1.2 }]],
            "empty": [],
        })
        .to_string();
        {
            let g = Graph::open(&path).unwrap();
            g.write(|tx| tx.set_setting("space.maps", old.as_bytes())).unwrap();
        }
        let g = Graph::open(&path).unwrap();
        let m = call(&g, "scenes.maps", &json!({})).unwrap();
        assert_eq!(m["corner"].as_array().unwrap().len(), 2);
        assert_eq!(m["corner"][1][4]["size"], 1.2);
        assert_eq!(m["empty"], json!([]));
        // the keyless rows are one entry now (the same text), placed twice
        assert_eq!(m["corner"][0][4]["key"], m["corner"][1][4]["key"]);
        let v = g.view().unwrap();
        assert_eq!(v.setting("space.maps").unwrap(), None);
        assert_eq!(
            v.setting(crate::graph::scenes::MAPS_BEFORE).unwrap().unwrap(),
            old.as_bytes()
        );
        drop(v);
        drop(g);
        // nothing moves twice
        let g = Graph::open(&path).unwrap();
        assert_eq!(g.scenes().unwrap().len(), 2);
        drop(g);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
