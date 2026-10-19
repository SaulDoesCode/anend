//! The app's block operations (`blocks.*`, listed in docs/app-api.md): spatial saves,
//! blocks placed in the space showing a text, a picture or a plain colour tiled over
//! their faces (`graph::blocks` keeps them: one key per texture, one 32 byte transform
//! per instance, so repeating a text is cheap).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::classify::Limits;
use super::{Res, s};
use crate::graph::blocks::{Block, Kind, Transform, key};
use crate::graph::record;
use crate::graph::{Graph, Reader};

/// blocks a save holds at most on a shared hub (one's own machine: no limit)
pub const MAX_BLOCKS_SHARED: u64 = 50_000;
/// blocks one call adds or changes at most
pub const MAX_PER_CALL: usize = 5000;

/// whether `op` is one of these
pub fn routed(op: &str) -> bool {
    op.starts_with("blocks.")
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
    }
}

fn vec3(v: &Value) -> Option<[f32; 3]> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32])
}

fn ids(args: &Value) -> Result<Vec<u64>, String> {
    let one = args.get("id").and_then(Value::as_u64);
    let many: Vec<u64> = args
        .get("ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect();
    let all: Vec<u64> = one.into_iter().chain(many).collect();
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
    }
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
        };
        textures.insert(k, about);
    }
    Ok(json!({ "blocks": blocks, "textures": textures }))
}

pub fn call(g: &Graph, op: &str, args: &Value, limits: &Limits) -> Res {
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
                let kind = Kind::parse(s(it, "kind")).ok_or("which kind of block? (text, picture or plain)")?;
                let tex = texture(g, kind, s(it, "texture"))?;
                new.push((kind, tex, transform(it, Transform::default())));
            }
            let ids = g.add_blocks(&new)?;
            let blocks: Vec<Block> = ids.iter().filter_map(|id| g.block(*id).ok().flatten()).collect();
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
                let id = it.get("id").and_then(Value::as_u64).ok_or("which block? (id)")?;
                let b = g.block(id)?.ok_or_else(|| format!("no block {id}"))?;
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
            let blocks: Vec<Block> = new.iter().filter_map(|id| g.block(*id).ok().flatten()).collect();
            let mut out = listed(g, &blocks)?;
            out["ids"] = json!(new);
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
        assert_eq!(r["ids"], json!([1, 2, 3]));
        // the text is sent once for the two blocks showing it
        assert_eq!(r["textures"].as_object().unwrap().len(), 2);
        assert_eq!(r["textures"][format!("text:{label}")]["text"], "listen");
        assert_eq!(r["blocks"][2]["texture"], "#a0b0c0");

        // move keeps the size, scale keeps the place
        let m = c("blocks.move", json!({ "id": 1, "x": 5, "w": 99 })).unwrap();
        assert_eq!(
            (m["blocks"][0]["x"].as_f64(), m["blocks"][0]["w"].as_f64()),
            (Some(5.0), Some(3.0))
        );
        let s = c("blocks.scale", json!({ "id": 1, "x": 0, "w": 6 })).unwrap();
        assert_eq!(
            (s["blocks"][0]["x"].as_f64(), s["blocks"][0]["w"].as_f64()),
            (Some(5.0), Some(6.0))
        );
        let s = c("blocks.set", json!({ "changes": [{ "id": 2, "by": [0, 1, 0] }] })).unwrap();
        assert_eq!(s["blocks"][0]["y"].as_f64(), Some(1.0));

        let d = c("blocks.duplicate", json!({ "ids": [1, 2], "by": [0, 0, 3] })).unwrap();
        assert_eq!(d["ids"], json!([4, 5]));
        assert_eq!(
            c("blocks.stats", json!({})).unwrap(),
            json!({ "blocks": 5, "textures": 2 })
        );
        let near = c("blocks.list", json!({ "lo": [4, -1, -1], "hi": [6, 1, 1] })).unwrap();
        // block 1 (x 5, w 6) and block 2 (x 4) meet it; their copies are 3 further back
        assert_eq!(near["blocks"].as_array().unwrap().len(), 2);

        let gone = c("blocks.delete", json!({ "ids": [1, 3] })).unwrap();
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
    }
}
