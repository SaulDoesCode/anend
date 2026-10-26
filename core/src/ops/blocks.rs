//! The app's block operations (`blocks.*`, listed in docs/app-api.md): spatial saves,
//! blocks placed in the space showing a text, a picture or a plain colour tiled over
//! their faces (`graph::blocks` keeps them: one key per texture, one 32 byte transform
//! per instance, so repeating a text is cheap), and the texture recipes blocks, faces and
//! the sky show (`textures.*`, `graph::textures`).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::classify::Limits;
use super::{Res, s};
use crate::graph::blocks::{Block, Faces, Kind, Transform, key};
use crate::graph::record;
use crate::graph::textures::{self, Recipe};
use crate::graph::{Graph, Reader};

/// blocks a save holds at most on a shared hub (one's own machine: no limit)
pub const MAX_BLOCKS_SHARED: u64 = 50_000;
/// blocks one call adds or changes at most
pub const MAX_PER_CALL: usize = 5000;

/// whether `op` is one of these
pub fn routed(op: &str) -> bool {
    op.starts_with("blocks.") || op.starts_with("textures.") || op.starts_with("lights.") || op.starts_with("anims.")
}

fn num(v: &Value, k: &str) -> Option<f32> {
    v.get(k).and_then(Value::as_f64).map(|x| x as f32)
}

/// a transform from `{x, y, z, w, h, d}`, the missing ones from `base`
fn transform(v: &Value, base: Transform) -> Transform {
    Transform {
        x: num(v, "x").unwrap_or(base.x),
        y: num(v, "y").unwrap_or(base.y),
        z: num(v, "z").unwrap_or(base.z),
        w: num(v, "w").unwrap_or(base.w),
        h: num(v, "h").unwrap_or(base.h),
        d: num(v, "d").unwrap_or(base.d),
        rot: v
            .get("rot")
            .and_then(Value::as_array)
            .filter(|a| a.len() == 4)
            .and_then(|a| {
                Some([
                    a[0].as_f64()? as f32,
                    a[1].as_f64()? as f32,
                    a[2].as_f64()? as f32,
                    a[3].as_f64()? as f32,
                ])
            })
            .unwrap_or(base.rot),
        // `bend: null` straightens it
        bend: match v.get("bend") {
            None => base.bend,
            Some(Value::Null) => None,
            Some(b) => serde_json::from_value(b.clone()).ok().or(base.bend),
        },
        // six recipe ids or nulls (right, left, top, bottom, front, back); `faces: null`
        // clears them
        faces: match v.get("faces") {
            None => base.faces,
            Some(Value::Null) => None,
            Some(f) => serde_json::from_value::<Faces>(f.clone()).ok().or(base.faces),
        },
        shape: v
            .get("shape")
            .and_then(|s| serde_json::from_value(s.clone()).ok())
            .unwrap_or(base.shape),
        // `zone: {repel?, attract?, link?, gravity?, damping?}` makes it a staging zone,
        // `zone: null` an ordinary block again
        zone: match v.get("zone") {
            None => base.zone,
            Some(Value::Null) => None,
            Some(z) => serde_json::from_value(z.clone()).ok().or(base.zone),
        },
        // `anim: {key, delay?, speed?}` plays an animation, `anim: null` stops it
        anim: match v.get("anim") {
            None => base.anim,
            Some(Value::Null) => None,
            Some(a) => serde_json::from_value(a.clone()).ok().or(base.anim),
        },
    }
}

fn vec3(v: &Value) -> Option<[f32; 3]> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32])
}

/// block ids: an `id` or `ids`, each a block's address (`text:3,14#2`)
fn ids(args: &Value) -> Result<Vec<String>, String> {
    let one = args.get("id").and_then(Value::as_str).map(str::to_string);
    let many: Vec<String> = args
        .get("ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let all: Vec<String> = one.into_iter().chain(many).collect();
    if all.is_empty() {
        return Err("which blocks? (id or ids)".into());
    }
    if all.len() > MAX_PER_CALL {
        return Err(format!("at most {MAX_PER_CALL} blocks at once"));
    }
    Ok(all)
}

/// what a block shows, checked against the save: a text that's there (by its label, a
/// note's `note:` prefix taken off), a picture that's there, or a `#rrggbb` colour
fn texture(g: &Graph, kind: Kind, t: &str) -> Result<String, String> {
    let t = t.trim();
    match kind {
        Kind::Text => {
            let label = t.strip_prefix("note:").unwrap_or(t);
            let id = record::parse_label(label).ok_or_else(|| format!("{t:?} isn't a text's key"))?;
            g.view()?.record(id)?.ok_or_else(|| format!("no text {t:?}"))?;
            Ok(record::label(id))
        }
        Kind::Picture => {
            g.picture(t)?.ok_or_else(|| format!("no picture {t:?}"))?;
            Ok(t.to_string())
        }
        Kind::Plain => {
            let ok = t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit());
            if ok {
                Ok(t.to_lowercase())
            } else {
                Err(format!("{t:?} isn't a colour (#rrggbb)"))
            }
        }
        Kind::Texture => {
            let id = t.to_lowercase();
            g.texture(&id)?.ok_or_else(|| format!("no texture {t:?}"))?;
            Ok(id)
        }
    }
}

/// what the space needs to draw a recipe: the recipe, its name, and its source's text
/// (a picture's thumbnail the app adds)
fn recipe_about(g: &Graph, v: &crate::graph::View, id: &str) -> Result<Value, String> {
    let Some(t) = g.texture(id)? else {
        return Ok(json!({ "gone": true }));
    };
    let mut about = json!({ "recipe": t.recipe, "name": t.name });
    match &t.recipe.source {
        textures::Source::Text(l) => match record::parse_label(l) {
            Some(e) if v.record(e)?.is_some() => about["text"] = json!(v.render(e)?),
            _ => about["text_gone"] = json!(true),
        },
        textures::Source::Picture(p) => {
            if g.picture(p)?.is_none() {
                about["picture_gone"] = json!(true);
            }
        }
        textures::Source::Plain(_) => {}
    }
    Ok(about)
}

/// blocks as JSON, with what their textures are: each text once (however many blocks
/// show it), each picture's size and name
fn listed(g: &Graph, blocks: &[Block]) -> Res {
    let mut textures: BTreeMap<String, Value> = BTreeMap::new();
    let v = g.view()?;
    for b in blocks {
        let k = key(b.kind, &b.texture);
        if textures.contains_key(&k) {
            continue;
        }
        if b.kind == Kind::Texture {
            textures.insert(k, recipe_about(g, &v, &b.texture)?);
            continue;
        }
        let about = match b.kind {
            Kind::Text => match record::parse_label(&b.texture) {
                Some(id) if v.record(id)?.is_some() => json!({ "text": v.render(id)? }),
                _ => json!({ "text": "", "gone": true }),
            },
            Kind::Picture => match g.picture(&b.texture)? {
                Some(p) => json!({ "name": p.name, "width": p.width, "height": p.height }),
                None => json!({ "gone": true }),
            },
            Kind::Plain => json!({ "colour": b.texture }),
            Kind::Texture => unreachable!("recipes are listed above"),
        };
        textures.insert(k, about);
    }
    // the recipes on single faces
    for b in blocks {
        for h in b.t.faces.iter().flat_map(|f| f.0.iter().flatten()) {
            let id: String = h.iter().map(|x| format!("{x:02x}")).collect();
            let k = key(Kind::Texture, &id);
            if let std::collections::btree_map::Entry::Vacant(e) = textures.entry(k) {
                e.insert(recipe_about(g, &v, &id)?);
            }
        }
    }
    // each block's variant of a recipe with variants (by its number among its texture's)
    let blocks: Vec<Value> = blocks
        .iter()
        .map(|b| {
            let mut o = json!(b);
            if b.kind == Kind::Texture
                && let Some(r) = textures[&key(b.kind, &b.texture)].get("recipe")
                && let Ok(r) = serde_json::from_value::<Recipe>(r.clone())
                && r.variants > 1
            {
                let n = crate::graph::blocks::split_id(&b.id).map_or(0, |(_, _, n)| n);
                o["variant"] = json!(textures::variant(r.seed, n, r.variants));
            }
            o
        })
        .collect();
    Ok(json!({ "blocks": blocks, "textures": textures }))
}

/// animations (`anims.*`, `graph::anims`): entries of their own mime, their text the JSON
fn anims_call(g: &Graph, op: &str, args: &Value) -> Res {
    use crate::graph::anims::Animation;
    use crate::graph::query::{Page, Query};
    use crate::graph::record::mime;
    let key = |args: &Value| -> Result<u64, String> {
        let k = s(args, "key");
        let id = record::parse_label(k).ok_or_else(|| format!("{k:?} isn't an animation's key"))?;
        match g.view()?.record(id)? {
            Some(r) if r.mime == mime::ANIMATION => Ok(id),
            _ => Err(format!("no animation {k}")),
        }
    };
    match op {
        "anims.list" => {
            let v = g.view()?;
            let found = v.query(
                &Query {
                    mime: Some(mime::ANIMATION),
                    ..Query::default()
                },
                Page::all(),
            )?;
            let mut out = Vec::new();
            for id in found.ids {
                if let Ok(a) = serde_json::from_str::<Animation>(&v.render(id)?) {
                    out.push(json!({ "key": record::label(id), "anim": a }));
                }
            }
            Ok(json!(out))
        }
        "anims.save" => {
            let a: Animation = serde_json::from_value(args.get("anim").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("that isn't an animation: {e}"))?;
            let a = a.checked()?;
            let text = serde_json::to_string(&a).map_err(|e| e.to_string())?;
            let id = if s(args, "key").is_empty() {
                g.save_text(&text, mime::ANIMATION)?.0
            } else {
                let id = key(args)?;
                if g.render(id)? != text {
                    g.edit_text(id, &text)?;
                }
                id
            };
            Ok(json!({ "key": record::label(id), "anim": a }))
        }
        "anims.delete" => {
            let id = key(args)?;
            g.delete(id)?;
            Ok(json!({ "removed": record::label(id) }))
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}

/// the room's lights (`lights.*`, `graph::lights`)
fn lights_call(g: &Graph, op: &str, args: &Value) -> Res {
    use crate::graph::lights::Light;
    let light = |v: &Value| serde_json::from_value::<Light>(v.clone()).map_err(|e| format!("that isn't a light: {e}"));
    match op {
        "lights.list" => Ok(json!(g.lights()?)),
        "lights.add" => Ok(json!(g.add_light(light(args.get("light").unwrap_or(args))?)?)),
        // the fields given replace the light's, the rest stay
        "lights.set" => {
            let id = args.get("id").and_then(Value::as_u64).ok_or("which light? (id)")?;
            let now = g
                .lights()?
                .into_iter()
                .find(|l| l.id == id)
                .ok_or_else(|| format!("no light {id}"))?;
            let mut v = json!(now);
            if let Some(o) = args.as_object() {
                for (k, x) in o {
                    v[k] = x.clone();
                }
            }
            v["id"] = json!(id);
            Ok(json!(g.set_light(light(&v)?)?))
        }
        "lights.delete" => {
            let id = args.get("id").and_then(Value::as_u64).ok_or("which light? (id)")?;
            Ok(json!({ "removed": g.delete_light(id)? }))
        }
        "lights.restore" => {
            let ls: Vec<Light> = serde_json::from_value(args.get("lights").cloned().unwrap_or(Value::Null))
                .map_err(|e| e.to_string())?;
            Ok(json!({ "restored": g.put_lights(&ls)? }))
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}

/// the recipes (`textures.*`)
fn textures_call(g: &Graph, op: &str, args: &Value) -> Res {
    match op {
        "textures.list" => {
            let v = g.view()?;
            let mut out = Vec::new();
            for t in g.textures()? {
                let mut about = recipe_about(g, &v, &t.id)?;
                about["id"] = json!(t.id);
                about["added"] = json!(t.added);
                out.push(about);
            }
            Ok(json!(out))
        }
        "textures.save" => {
            let r: Recipe = serde_json::from_value(args.get("recipe").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("that isn't a recipe: {e}"))?;
            match &r.source {
                textures::Source::Picture(p) if g.picture(p)?.is_none() => return Err(format!("no picture {p:?}")),
                textures::Source::Text(l) => {
                    texture(g, Kind::Text, l)?;
                }
                _ => {}
            }
            let t = g.put_texture(&r, s(args, "name"), crate::graph::store::system_ms())?;
            Ok(json!(t))
        }
        "textures.name" => {
            let id = s(args, "id");
            let t = g.texture(id)?.ok_or_else(|| format!("no texture {id:?}"))?;
            Ok(json!(g.put_texture(&t.recipe, s(args, "name"), t.added)?))
        }
        "textures.delete" => Ok(json!({ "removed": g.delete_texture(s(args, "id"))? })),
        _ => Err(format!("unknown op {op:?}")),
    }
}

pub fn call(g: &Graph, op: &str, args: &Value, limits: &Limits) -> Res {
    if op.starts_with("textures.") {
        return textures_call(g, op, args);
    }
    if op.starts_with("lights.") {
        return lights_call(g, op, args);
    }
    if op.starts_with("anims.") {
        return anims_call(g, op, args);
    }
    match op {
        "blocks.list" => {
            let region = match (args.get("lo").and_then(vec3), args.get("hi").and_then(vec3)) {
                (Some(lo), Some(hi)) => Some((lo, hi)),
                (None, None) => None,
                _ => return Err("a region is lo and hi, [x, y, z] each".into()),
            };
            let mut blocks = g.blocks(region)?;
            if let Some(k) = Kind::parse(s(args, "kind")) {
                blocks.retain(|b| b.kind == k);
            }
            if !s(args, "texture").is_empty() {
                blocks.retain(|b| b.texture == s(args, "texture"));
            }
            listed(g, &blocks)
        }
        "blocks.stats" => Ok(json!(g.block_use()?)),
        "blocks.add" => {
            let items: Vec<Value> = match args.get("blocks").and_then(Value::as_array) {
                Some(a) => a.clone(),
                None => vec![args.clone()],
            };
            if items.len() > MAX_PER_CALL {
                return Err(format!("at most {MAX_PER_CALL} blocks at once"));
            }
            if limits.shared && g.block_use()?.blocks + items.len() as u64 > MAX_BLOCKS_SHARED {
                return Err(format!(
                    "a save holds at most {MAX_BLOCKS_SHARED} blocks on a shared hub"
                ));
            }
            let mut new = Vec::with_capacity(items.len());
            for it in &items {
                let kind =
                    Kind::parse(s(it, "kind")).ok_or("which kind of block? (text, picture, plain or texture)")?;
                let tex = texture(g, kind, s(it, "texture"))?;
                new.push((kind, tex, transform(it, Transform::default())));
            }
            let ids = g.add_blocks(&new)?;
            let blocks: Vec<Block> = ids.iter().filter_map(|id| g.block(id).ok().flatten()).collect();
            let mut out = listed(g, &blocks)?;
            out["ids"] = json!(ids);
            Ok(out)
        }
        // move and scale: the fields given replace the block's, the rest stay
        "blocks.set" | "blocks.move" | "blocks.scale" => {
            let items: Vec<Value> = match args.get("changes").and_then(Value::as_array) {
                Some(a) => a.clone(),
                None => vec![args.clone()],
            };
            if items.len() > MAX_PER_CALL {
                return Err(format!("at most {MAX_PER_CALL} blocks at once"));
            }
            let mut changes = Vec::new();
            for it in &items {
                let id = it
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("which block? (id)")?
                    .to_string();
                let b = g.block(&id)?.ok_or_else(|| format!("no block {id}"))?;
                let mut t = transform(it, b.t);
                match op {
                    "blocks.move" => (t.w, t.h, t.d) = (b.t.w, b.t.h, b.t.d),
                    "blocks.scale" => (t.x, t.y, t.z) = (b.t.x, b.t.y, b.t.z),
                    _ => {}
                }
                if let Some(by) = it.get("by").and_then(vec3) {
                    t.x += by[0];
                    t.y += by[1];
                    t.z += by[2];
                }
                changes.push((id, t));
            }
            listed(g, &g.set_blocks(&changes)?)
        }
        "blocks.duplicate" => {
            let ids = ids(args)?;
            if limits.shared && g.block_use()?.blocks + ids.len() as u64 > MAX_BLOCKS_SHARED {
                return Err(format!(
                    "a save holds at most {MAX_BLOCKS_SHARED} blocks on a shared hub"
                ));
            }
            let by = args.get("by").and_then(vec3).unwrap_or([1.0, 0.0, 0.0]);
            let new = g.duplicate_blocks(&ids, by)?;
            let blocks: Vec<Block> = new.iter().filter_map(|id| g.block(id).ok().flatten()).collect();
            let mut out = listed(g, &blocks)?;
            out["ids"] = json!(new);
            Ok(out)
        }
        // shows another texture: the block comes back as the n-th of its new texture's
        // (a new id), where it was, as big and as turned
        "blocks.retexture" => {
            let id = s(args, "id");
            let b = g.block(id)?.ok_or_else(|| format!("no block {id}"))?;
            let kind =
                Kind::parse(s(args, "kind")).ok_or("which kind of texture? (text, picture, plain or texture)")?;
            let tex = texture(g, kind, s(args, "texture"))?;
            if kind == b.kind && tex == b.texture {
                let mut out = listed(g, &[b])?;
                out["ids"] = json!([id]);
                return Ok(out);
            }
            let removed = g.delete_blocks(&[id.to_string()])?;
            let ids = g.add_blocks(&[(kind, tex, b.t)])?;
            let blocks: Vec<Block> = ids.iter().filter_map(|i| g.block(i).ok().flatten()).collect();
            let mut out = listed(g, &blocks)?;
            out["ids"] = json!(ids);
            out["removed"] = json!(removed);
            Ok(out)
        }
        "blocks.delete" => {
            let gone = g.delete_blocks(&ids(args)?)?;
            Ok(json!({ "removed": gone }))
        }
        // puts back what `blocks.delete` answered (an undo), with the same ids
        "blocks.restore" => {
            let blocks: Vec<Block> = serde_json::from_value(args.get("blocks").cloned().unwrap_or(Value::Null))
                .map_err(|e| e.to_string())?;
            if blocks.len() > MAX_PER_CALL {
                return Err(format!("at most {MAX_PER_CALL} blocks at once"));
            }
            let mut ok = Vec::new();
            for b in blocks {
                if texture(g, b.kind, &b.texture).is_ok() {
                    ok.push(b);
                }
            }
            Ok(json!({ "restored": g.put_blocks(&ok)? }))
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_through_the_api() {
        let g = Graph::in_memory().unwrap();
        let (id, _) = g.save_text("listen", crate::graph::record::mime::TEXT).unwrap();
        let label = record::label(id);
        let c = |op: &str, a: Value| call(&g, op, &a, &Limits::NONE);
        assert!(c("blocks.add", json!({ "kind": "text", "texture": "9,9,9" })).is_err());
        assert!(c("blocks.add", json!({ "kind": "plain", "texture": "red" })).is_err());
        assert!(c("blocks.add", json!({ "kind": "picture", "texture": "00" })).is_err());
        let r = c(
            "blocks.add",
            json!({ "blocks": [
                { "kind": "text", "texture": label, "x": 0, "w": 3 },
                { "kind": "text", "texture": label, "x": 4 },
                { "kind": "plain", "texture": "#A0B0C0", "y": -2, "w": 20, "d": 20, "h": 0.2 },
            ] }),
        )
        .unwrap();
        assert_eq!(
            r["ids"],
            json!([format!("text:{label}#1"), format!("text:{label}#2"), "plain:#a0b0c0#1"])
        );
        let (b1, b2, b3) = (format!("text:{label}#1"), format!("text:{label}#2"), "plain:#a0b0c0#1");
        // the text is sent once for the two blocks showing it
        assert_eq!(r["textures"].as_object().unwrap().len(), 2);
        assert_eq!(r["textures"][format!("text:{label}")]["text"], "listen");
        assert_eq!(r["blocks"][2]["texture"], "#a0b0c0");

        // move keeps the size, scale keeps the place
        let m = c("blocks.move", json!({ "id": b1, "x": 5, "w": 99 })).unwrap();
        assert_eq!(
            (m["blocks"][0]["x"].as_f64(), m["blocks"][0]["w"].as_f64()),
            (Some(5.0), Some(3.0))
        );
        let s = c("blocks.scale", json!({ "id": b1, "x": 0, "w": 6 })).unwrap();
        assert_eq!(
            (s["blocks"][0]["x"].as_f64(), s["blocks"][0]["w"].as_f64()),
            (Some(5.0), Some(6.0))
        );
        let s = c("blocks.set", json!({ "changes": [{ "id": b2, "by": [0, 1, 0] }] })).unwrap();
        assert_eq!(s["blocks"][0]["y"].as_f64(), Some(1.0));

        let d = c("blocks.duplicate", json!({ "ids": [b1, b2], "by": [0, 0, 3] })).unwrap();
        assert_eq!(d["ids"], json!([format!("text:{label}#3"), format!("text:{label}#4")]));
        assert_eq!(
            c("blocks.stats", json!({})).unwrap(),
            json!({ "blocks": 5, "textures": 2 })
        );
        let near = c("blocks.list", json!({ "lo": [4, -1, -1], "hi": [6, 1, 1] })).unwrap();
        // block 1 (x 5, w 6) and block 2 (x 4) meet it; their copies are 3 further back
        assert_eq!(near["blocks"].as_array().unwrap().len(), 2);

        let gone = c("blocks.delete", json!({ "ids": [b1, b3] })).unwrap();
        assert_eq!(c("blocks.stats", json!({})).unwrap()["blocks"], 3);
        assert_eq!(
            c("blocks.restore", json!({ "blocks": gone["removed"] })).unwrap()["restored"],
            2
        );
        assert_eq!(
            c("blocks.list", json!({})).unwrap()["blocks"].as_array().unwrap().len(),
            5
        );
    }

    #[test]
    fn texture_recipes_on_blocks_and_faces() {
        let g = Graph::in_memory().unwrap();
        let (id, _) = g.save_text("brick", crate::graph::record::mime::TEXT).unwrap();
        let label = record::label(id);
        let c = |op: &str, a: Value| call(&g, op, &a, &Limits::NONE);
        assert!(c("textures.save", json!({ "recipe": { "source": { "text": "9,9,9" } } })).is_err());
        assert!(c("textures.save", json!({ "recipe": { "source": { "picture": "00" } } })).is_err());
        let r = c(
            "textures.save",
            json!({ "name": "wall", "recipe": { "source": { "text": label }, "scale": [2, 1], "variants": 4, "seed": 9 } }),
        )
        .unwrap();
        let rid = r["id"].as_str().unwrap().to_string();
        let plain = c(
            "textures.save",
            json!({ "recipe": { "source": { "plain": "#336699" } } }),
        )
        .unwrap();
        let pid = plain["id"].as_str().unwrap().to_string();
        let list = c("textures.list", json!({})).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 2);
        assert_eq!(list[0]["text"], "brick");
        assert_eq!(list[0]["name"], "wall");

        let a = c(
            "blocks.add",
            json!({ "blocks": [
            { "kind": "texture", "texture": rid, "w": 4 },
            { "kind": "texture", "texture": rid, "x": 5 },
            { "kind": "plain", "texture": "#000000", "x": 9 },
        ] }),
        )
        .unwrap();
        assert_eq!(a["ids"][0], format!("texture:{rid}#1"));
        // each block of a recipe with variants says which it shows
        let v0 = a["blocks"][0]["variant"].as_u64().unwrap() as u8;
        assert_eq!(v0, crate::graph::textures::variant(9, 1, 4));
        assert!(a["blocks"][2].get("variant").is_none());
        assert_eq!(
            a["textures"][format!("texture:{rid}")]["recipe"]["scale"],
            json!([2.0, 1.0])
        );

        // a face of the plain block shows the plain recipe; the recipe is listed with it
        let plain_block = a["ids"][2].as_str().unwrap().to_string();
        let f = c(
            "blocks.set",
            json!({ "id": plain_block, "faces": [null, null, pid, null, null, null] }),
        )
        .unwrap();
        assert_eq!(f["blocks"][0]["faces"], json!([null, null, pid, null, null, null]));
        assert!(f["textures"].get(format!("texture:{pid}")).is_some());
        // kept with its bend
        let f = c(
            "blocks.set",
            json!({ "id": plain_block, "bend": { "ripple": { "amount": 0.5 } } }),
        )
        .unwrap();
        assert_eq!(f["blocks"][0]["faces"][2], json!(pid));
        assert_eq!(f["blocks"][0]["bend"]["ripple"]["amount"], 0.5);
        let f = c("blocks.set", json!({ "id": plain_block, "faces": null })).unwrap();
        assert!(f["blocks"][0].get("faces").is_none());

        // retexturing gives the block its new texture's next number
        let t = c(
            "blocks.retexture",
            json!({ "id": plain_block, "kind": "texture", "texture": rid }),
        )
        .unwrap();
        assert_eq!(t["ids"][0], format!("texture:{rid}#3"));
        assert_eq!(t["blocks"][0]["x"], 9.0);
        assert_eq!(
            c("blocks.stats", json!({})).unwrap(),
            json!({ "blocks": 3, "textures": 1 })
        );

        // a backup brings the recipes back
        let (texts, other) = g.backup().unwrap();
        let h = Graph::in_memory().unwrap();
        h.restore(&texts, &other).unwrap();
        assert_eq!(h.textures().unwrap().len(), 2);
        assert_eq!(h.blocks(None).unwrap().len(), 3);
    }

    #[test]
    fn lights_through_the_api() {
        let g = Graph::in_memory().unwrap();
        let c = |op: &str, a: Value| call(&g, op, &a, &Limits::NONE);
        let a = c(
            "lights.add",
            json!({ "kind": "point", "colour": "#ffeedd", "y": 3, "range": 12 }),
        )
        .unwrap();
        assert_eq!((a["id"].as_u64(), a["kind"].as_str()), (Some(1), Some("point")));
        let s = c("lights.set", json!({ "id": 1, "intensity": 2.5 })).unwrap();
        assert_eq!(
            (s["intensity"].as_f64(), s["range"].as_f64(), s["y"].as_f64()),
            (Some(2.5), Some(12.0), Some(3.0))
        );
        c(
            "lights.add",
            json!({ "light": { "kind": "directional", "dir": [1, -1, 0] } }),
        )
        .unwrap();
        assert_eq!(c("lights.list", json!({})).unwrap().as_array().unwrap().len(), 2);
        // a backup and a merge carry them
        let (texts, other) = g.backup().unwrap();
        let h = Graph::in_memory().unwrap();
        h.restore(&texts, &other).unwrap();
        assert_eq!(h.lights().unwrap(), g.lights().unwrap());
        let v = g.view().unwrap();
        h.write(|tx| crate::ops::merge::merge(tx, &v)).unwrap();
        assert_eq!(crate::ops::merge::assets(&h, &g).unwrap().lights, 2);
        assert_eq!(h.lights().unwrap().len(), 4);
        let gone = c("lights.delete", json!({ "id": 1 })).unwrap();
        assert_eq!(c("lights.list", json!({})).unwrap().as_array().unwrap().len(), 1);
        assert_eq!(
            c("lights.restore", json!({ "lights": [gone["removed"]] })).unwrap()["restored"],
            1
        );
    }

    #[test]
    fn animations_are_entries_blocks_play() {
        let g = Graph::in_memory().unwrap();
        let c = |op: &str, a: Value| call(&g, op, &a, &Limits::NONE);
        let a = c(
            "anims.save",
            json!({ "anim": { "name": "bob", "duration": 3, "repeat": "loop",
            "keys": [{ "at": 1, "move": [0, 2, 0] }, { "at": 0, "move": [0, 0, 0] }] } }),
        )
        .unwrap();
        let key = a["key"].as_str().unwrap().to_string();
        assert_eq!(a["anim"]["keys"][0]["at"], 0.0);
        // not a text: search and the counts leave it out
        assert_eq!(super::super::counts(&g.view().unwrap()).unwrap(), (0, 0));
        let b = c(
            "blocks.add",
            json!({ "kind": "plain", "texture": "#ffffff", "anim": { "key": key, "delay": 0.5 } }),
        )
        .unwrap();
        assert_eq!(
            b["blocks"][0]["anim"],
            json!({ "key": key, "delay": 0.5, "speed": 1.0 })
        );
        // edited in place: the block still plays it
        let e = c(
            "anims.save",
            json!({ "key": key, "anim": { "name": "bob", "duration": 6, "keys": [] } }),
        )
        .unwrap();
        assert_eq!(e["key"], key);
        assert_eq!(c("anims.list", json!({})).unwrap()[0]["anim"]["duration"], 6.0);
        // a merge finds it in the target as an entry of its own
        let target = Graph::in_memory().unwrap();
        target.save_text("other", crate::graph::record::mime::TEXT).unwrap();
        let v = g.view().unwrap();
        target.write(|tx| crate::ops::merge::merge(tx, &v)).unwrap();
        crate::ops::merge::assets(&target, &g).unwrap();
        let tb = &target.blocks(None).unwrap()[0];
        let r = tb.t.anim.unwrap();
        assert!(target.render(r.entry).unwrap().contains("\"duration\":6"));
        let id = b["ids"][0].as_str().unwrap().to_string();
        let s = c("blocks.set", json!({ "id": id, "anim": null })).unwrap();
        assert!(s["blocks"][0].get("anim").is_none());
        c("anims.delete", json!({ "key": key })).unwrap();
        assert_eq!(c("anims.list", json!({})).unwrap(), json!([]));
    }

    #[test]
    fn zones_keep_their_physics() {
        let g = Graph::in_memory().unwrap();
        let c = |op: &str, a: Value| call(&g, op, &a, &Limits::NONE);
        let a = c(
            "blocks.add",
            json!({ "kind": "plain", "texture": "#88aa88", "w": 10, "h": 6, "d": 2, "zone": {} }),
        )
        .unwrap();
        let id = a["ids"][0].as_str().unwrap().to_string();
        assert_eq!(a["blocks"][0]["zone"], json!({}));
        let s = c(
            "blocks.set",
            json!({ "id": id, "zone": { "repel": 99, "gravity": 0.2 }, "bend": { "fold": { "amount": 1 } } }),
        )
        .unwrap();
        assert_eq!(
            s["blocks"][0]["zone"],
            json!({ "repel": 60.0, "gravity": 0.2_f32 as f64 })
        );
        // with its bend, read back from the placement
        let l = c("blocks.list", json!({})).unwrap();
        assert_eq!(l["blocks"][0]["zone"]["repel"], 60.0);
        assert_eq!(l["blocks"][0]["bend"]["fold"]["amount"], 1.0);
        let s = c("blocks.set", json!({ "id": id, "zone": null })).unwrap();
        assert!(s["blocks"][0].get("zone").is_none());
    }

    #[test]
    fn spheres_are_blocks_too() {
        let g = Graph::in_memory().unwrap();
        let c = |op: &str, a: Value| call(&g, op, &a, &Limits::NONE);
        let a = c(
            "blocks.add",
            json!({ "kind": "plain", "texture": "#ffcc00", "shape": "sphere", "w": 2, "h": 1 }),
        )
        .unwrap();
        let id = a["ids"][0].as_str().unwrap().to_string();
        assert_eq!(a["blocks"][0]["shape"], "sphere");
        // a sphere neither bends nor has faces
        let s = c(
            "blocks.set",
            json!({ "id": id, "bend": { "ripple": { "amount": 1 } }, "faces": [null, null, null, null, null, null] }),
        )
        .unwrap();
        assert!(s["blocks"][0].get("bend").is_none());
        assert_eq!(c("blocks.list", json!({})).unwrap()["blocks"][0]["shape"], "sphere");
        // back to a box: the shape isn't written for one
        let s = c("blocks.set", json!({ "id": id, "shape": "box" })).unwrap();
        assert!(s["blocks"][0].get("shape").is_none());
        let (texts, other) = g.backup().unwrap();
        c("blocks.set", json!({ "id": id, "shape": "sphere" })).unwrap();
        let (_, other2) = g.backup().unwrap();
        assert_ne!(other, other2);
        let h = Graph::in_memory().unwrap();
        h.restore(&texts, &other2).unwrap();
        assert_eq!(h.blocks(None).unwrap()[0].t.shape, crate::graph::blocks::Shape::Sphere);
    }

    #[test]
    fn blocks_travel_with_backups_and_merges() {
        let g = Graph::in_memory().unwrap();
        let (id, _) = g.save_text("listen", crate::graph::record::mime::TEXT).unwrap();
        g.add_block(Kind::Text, &record::label(id), Transform::default())
            .unwrap();
        g.add_block(
            Kind::Plain,
            "#112233",
            Transform {
                x: 2.0,
                ..Transform::default()
            },
        )
        .unwrap();
        let (texts, other) = g.backup().unwrap();
        let back = Graph::in_memory().unwrap();
        back.restore(&texts, &other).unwrap();
        assert_eq!(back.blocks(None).unwrap(), g.blocks(None).unwrap());

        // a merge finds the text in the target, whatever its key there
        let target = Graph::in_memory().unwrap();
        target.save_text("other", crate::graph::record::mime::TEXT).unwrap();
        target.save_text("more", crate::graph::record::mime::TEXT).unwrap();
        let v = g.view().unwrap();
        target.write(|tx| crate::ops::merge::merge(tx, &v)).unwrap();
        let w = crate::ops::merge::assets(&target, &g).unwrap();
        assert_eq!(w.blocks, 2);
        let tb = target.blocks(Some(([-0.1; 3], [0.1; 3]))).unwrap();
        assert_eq!(tb.len(), 1);
        let tid = record::parse_label(&tb[0].texture).unwrap();
        assert_eq!(target.render(tid).unwrap(), "listen");

        // a recipe from a text: in the target it names the target's key, so its id changes,
        // and the blocks and faces showing it follow
        let r = Recipe {
            source: textures::Source::Text(record::label(id)),
            ..Recipe::plain("#000000")
        };
        let saved = g.put_texture(&r, "wall", 1).unwrap();
        let mut faces = Faces::default();
        faces.0[4] = crate::graph::blocks::def_of(Kind::Texture, &saved.id)
            .ok()
            .and_then(|d| match d {
                crate::graph::scenes::Def::Texture(h) => Some(h),
                _ => None,
            });
        g.add_block(
            Kind::Texture,
            &saved.id,
            Transform {
                x: 40.0,
                faces: Some(faces),
                ..Transform::default()
            },
        )
        .unwrap();
        let target = Graph::in_memory().unwrap();
        target.save_text("other", crate::graph::record::mime::TEXT).unwrap();
        let v = g.view().unwrap();
        target.write(|tx| crate::ops::merge::merge(tx, &v)).unwrap();
        let w = crate::ops::merge::assets(&target, &g).unwrap();
        assert_eq!((w.textures, w.blocks), (1, 3));
        let t = &target.textures().unwrap()[0];
        assert_ne!(t.id, saved.id);
        let textures::Source::Text(l) = &t.recipe.source else {
            panic!("a text recipe")
        };
        assert_eq!(target.render(record::parse_label(l).unwrap()).unwrap(), "listen");
        let b = target
            .blocks(None)
            .unwrap()
            .into_iter()
            .find(|b| b.kind == Kind::Texture)
            .unwrap();
        assert_eq!(b.texture, t.id);
        let face: String = b.t.faces.unwrap().0[4]
            .unwrap()
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect();
        assert_eq!(face, t.id);
    }
}
