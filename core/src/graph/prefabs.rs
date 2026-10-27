//! Prefabs (queued request #19; docs/format.md, "One model for a definition and its
//! placed instances"): blocks grouped into a subassembly, kept as a scene of their own
//! (its setting `prefab` true), and placed in the room as copies: one prefab placement
//! each (kind 2, the scene as its definition), a place, a turn and a size.
//!
//! A copy's parts are the prefab's blocks, turned, sized and moved with the copy. A part
//! is addressed as `prefab:<scene>#<n>/<its address in the prefab>`, so a link inside the
//! group (a sticky ray's target) names the inner address and works in every copy. Copies
//! inherit, as prototypes do in JavaScript: changing the prefab changes every copy, and a
//! copy's own changes to a part (its place, turn, size, texture or wiring state) are kept
//! in the copy's placement as overrides that shadow the prefab's values for it alone.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::blocks::{self, Block, Kind, Transform};
use super::place::{IDENTITY, pack_rotation, unpack_rotation};
use super::scenes::{self, Def, Kind as SKind, Placed, Placement, SCENES, Scale, Scene};
use super::store::{GResult, Graph, err};
use super::wiring::rotate;

/// a placed copy of a prefab
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Copy {
    /// `prefab:<scene>#<n>`
    pub id: String,
    pub scene: u64,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rot: [f32; 4],
    pub scale: f32,
    /// how many of its parts have their own values
    pub own: usize,
}

/// a copy's address
pub fn copy_id(scene: u64, n: u64) -> String {
    format!("prefab:{scene}#{n}")
}

/// a copy's scene and number
pub fn split_copy(id: &str) -> Option<(u64, u64)> {
    let rest = id.strip_prefix("prefab:")?;
    let (s, n) = rest.split_once('#')?;
    Some((s.parse().ok()?, n.parse().ok()?))
}

/// a part's copy and inner address
pub fn split_part(id: &str) -> Option<(&str, &str)> {
    if !id.starts_with("prefab:") {
        return None;
    }
    id.split_once('/')
}

pub fn is_part(id: &str) -> bool {
    split_part(id).is_some()
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

fn conj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// a part's transform in the room from its transform in the prefab
fn to_world(c: &Copy, t: &Transform) -> Transform {
    let p = rotate(c.rot, [t.x * c.scale, t.y * c.scale, t.z * c.scale]);
    Transform {
        x: c.x + p[0],
        y: c.y + p[1],
        z: c.z + p[2],
        w: t.w * c.scale,
        h: t.h * c.scale,
        d: t.d * c.scale,
        rot: qmul(c.rot, t.rot),
        ..*t
    }
}

/// a part's transform in the prefab from its transform in the room
fn to_local(c: &Copy, t: &Transform) -> Transform {
    let qi = conj(c.rot);
    let s = c.scale.max(1e-6);
    let p = rotate(qi, [t.x - c.x, t.y - c.y, t.z - c.z]);
    Transform {
        x: p[0] / s,
        y: p[1] / s,
        z: p[2] / s,
        w: t.w / s,
        h: t.h / s,
        d: t.d / s,
        rot: qmul(qi, t.rot),
        ..*t
    }
}

/// each block's address in a new prefab holding them, in this order: its number among
/// its texture's blocks there, counted from 1
pub fn inner_ids(parts: &[(Kind, String)]) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, u64> = Default::default();
    parts
        .iter()
        .map(|(k, t)| {
            let n = seen.entry(blocks::key(*k, t)).or_insert(0);
            *n += 1;
            blocks::block_id(*k, t, *n)
        })
        .collect()
}

fn copy_of(p: &Placed, names: &std::collections::HashMap<u64, String>) -> Option<Copy> {
    let (SKind::Prefab, Def::Scene(scene)) = (p.kind, p.def) else {
        return None;
    };
    let q = &p.place;
    Some(Copy {
        id: copy_id(scene, q.n),
        scene,
        name: names.get(&scene).cloned().unwrap_or_default(),
        x: q.pos[0],
        y: q.pos[1],
        z: q.pos[2],
        rot: q.rot.map(unpack_rotation).unwrap_or(IDENTITY),
        scale: match q.scale {
            Scale::One => 1.0,
            Scale::Uniform(v) => v,
            Scale::Axes(a) => (a[0] + a[1] + a[2]) / 3.0,
        },
        own: q
            .own()
            .and_then(|m| m.get("parts").and_then(Value::as_object).map(|p| p.len()))
            .unwrap_or(0),
    })
}

/// a copy's overrides: inner address → its own values
fn overrides(p: &Placement) -> Map<String, Value> {
    p.own()
        .and_then(|m| m.get("parts").and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

/// a part with a copy's own values applied
fn shadowed(mut b: Block, own: Option<&Value>) -> Block {
    let Some(o) = own else { return b };
    if let Some(t) = o
        .get("t")
        .and_then(|t| serde_json::from_value::<Transform>(t.clone()).ok())
    {
        let wire = b.t.wire;
        b.t = t;
        b.t.wire = wire;
    }
    if let (Some(k), Some(tex)) = (
        o.get("kind")
            .and_then(|k| serde_json::from_value::<Kind>(k.clone()).ok()),
        o.get("texture").and_then(Value::as_str),
    ) {
        b.kind = k;
        b.texture = tex.to_string();
    }
    b
}

impl Graph {
    /// the save's prefabs (scenes whose setting `prefab` is true)
    pub fn prefabs(&self) -> GResult<Vec<Scene>> {
        Ok(self
            .scenes()?
            .into_iter()
            .filter(|s| s.settings.get("prefab") == Some(&Value::Bool(true)))
            .collect())
    }

    /// a prefab's blocks, as placed in it (relative to its origin)
    pub fn prefab_parts(&self, scene: u64) -> GResult<Vec<Block>> {
        Ok(self
            .placements(scene, false)?
            .iter()
            .filter_map(blocks::block_of)
            .collect())
    }

    fn room_prefab_placements(&self) -> GResult<Vec<Placed>> {
        Ok(match self.room()? {
            Some(r) => self
                .placements(r, false)?
                .into_iter()
                .filter(|p| p.kind == SKind::Prefab)
                .collect(),
            None => vec![],
        })
    }

    fn names(&self) -> GResult<std::collections::HashMap<u64, String>> {
        Ok(self.prefabs()?.into_iter().map(|s| (s.id, s.name)).collect())
    }

    /// the copies placed in the room
    pub fn copies(&self) -> GResult<Vec<Copy>> {
        let names = self.names()?;
        Ok(self
            .room_prefab_placements()?
            .iter()
            .filter_map(|p| copy_of(p, &names))
            .collect())
    }

    fn copy_placement(&self, id: &str) -> GResult<(Copy, Placed)> {
        let (scene, n) = split_copy(id).ok_or_else(|| format!("{id:?} isn't a prefab's copy"))?;
        let names = self.names()?;
        let p = self
            .room_prefab_placements()?
            .into_iter()
            .find(|p| p.def == Def::Scene(scene) && p.place.n == n)
            .ok_or_else(|| format!("no copy {id}"))?;
        let c = copy_of(&p, &names).ok_or("not a copy")?;
        Ok((c, p))
    }

    /// a copy's parts in the room, each under its part address
    pub fn copy_parts(&self, id: &str) -> GResult<Vec<Block>> {
        let (c, p) = self.copy_placement(id)?;
        let own = overrides(&p.place);
        Ok(self
            .prefab_parts(c.scene)?
            .into_iter()
            .map(|b| {
                let inner = b.id.clone();
                let mut b = shadowed(b, own.get(&inner));
                b.t = to_world(&c, &b.t);
                b.id = format!("{}/{inner}", c.id);
                b
            })
            .collect())
    }

    /// every copy's parts in the room
    pub fn parts(&self) -> GResult<Vec<Block>> {
        let mut out = Vec::new();
        for c in self.copies()? {
            out.extend(self.copy_parts(&c.id)?);
        }
        Ok(out)
    }

    /// the room's blocks and every copy's parts
    pub fn world_blocks(&self) -> GResult<Vec<Block>> {
        let mut all = self.blocks(None)?;
        all.extend(self.parts()?);
        Ok(all)
    }

    /// one part, as it is in the room
    pub fn part(&self, id: &str) -> GResult<Option<Block>> {
        let Some((copy, _)) = split_part(id) else {
            return Ok(None);
        };
        if self.copy_placement(copy).is_err() {
            return Ok(None);
        }
        Ok(self.copy_parts(copy)?.into_iter().find(|b| b.id == id))
    }

    /// a copy's own values for one part: what's given replaces them (`Null` drops one)
    fn set_own(&self, copy: &str, inner: &str, f: impl FnOnce(&mut Map<String, Value>)) -> GResult<()> {
        let (_, mut p) = self.copy_placement(copy)?;
        let mut own = p.place.own().unwrap_or_default();
        let mut parts = own.get("parts").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut mine = parts.get(inner).and_then(Value::as_object).cloned().unwrap_or_default();
        f(&mut mine);
        mine.retain(|_, v| !v.is_null());
        if mine.is_empty() {
            parts.remove(inner);
        } else {
            parts.insert(inner.to_string(), Value::Object(mine));
        }
        if parts.is_empty() {
            own.remove("parts");
        } else {
            own.insert("parts".into(), Value::Object(parts));
        }
        p.place.set_own(Some(own));
        let room = self.room()?.ok_or("no room")?;
        self.place(room, false, SKind::Prefab, p.def, None, vec![p.place])?;
        Ok(())
    }

    /// moves, turns or resizes a part in its copy only (an override); the part as it is now
    pub fn set_part(&self, id: &str, t: Transform) -> GResult<Option<Block>> {
        let Some((copy, inner)) = split_part(id) else {
            return Ok(None);
        };
        let Ok((c, _)) = self.copy_placement(copy) else {
            return Ok(None);
        };
        let mut local = to_local(&c, &t.checked()?);
        local.wire = None;
        let v = serde_json::to_value(local).map_err(err)?;
        self.set_own(copy, inner, |m| {
            m.insert("t".into(), v);
        })?;
        self.part(id)
    }

    /// shows another texture on a part, in its copy only
    pub fn retexture_part(&self, id: &str, kind: Kind, texture: &str) -> GResult<Option<Block>> {
        let Some((copy, inner)) = split_part(id) else {
            return Ok(None);
        };
        blocks::def_of(kind, texture)?;
        self.set_own(copy, inner, |m| {
            m.insert("kind".into(), json!(kind));
            m.insert("texture".into(), json!(texture));
        })?;
        self.part(id)
    }

    /// a part's wiring as its copy has it (its own, when the copy changed it)
    pub fn part_wiring(&self, id: &str) -> GResult<Option<Value>> {
        let Some((copy, inner)) = split_part(id) else {
            return Ok(None);
        };
        let (_, p) = self.copy_placement(copy)?;
        Ok(overrides(&p.place).get(inner).and_then(|o| o.get("wiring")).cloned())
    }

    /// keeps a part's wiring (its state, a ray that stuck) in its copy only
    pub fn set_part_wiring(&self, id: &str, w: Option<Value>) -> GResult<()> {
        let Some((copy, inner)) = split_part(id) else {
            return Err(format!("{id:?} isn't a part"));
        };
        self.set_own(copy, inner, |m| {
            m.insert("wiring".into(), w.unwrap_or(Value::Null));
        })
    }

    /// groups room blocks into a new prefab named `name` and puts one copy where they
    /// were: the prefab's id and the copy's. `wires` gives each block's wiring entry as
    /// the prefab should keep it (links inside the group already naming inner addresses)
    pub fn make_prefab(&self, name: &str, ids: &[String], wires: &[Option<u64>]) -> GResult<(u64, String)> {
        let mut bs = Vec::new();
        for id in ids {
            if is_part(id) || split_copy(id).is_some() {
                return Err("a prefab can't hold another prefab's parts yet".into());
            }
            bs.push(self.block(id)?.ok_or_else(|| format!("no block {id}"))?);
        }
        if bs.is_empty() {
            return Err("which blocks? (select some first)".into());
        }
        // the origin: the middle of their boxes
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for b in &bs {
            let h = b.t.half_extent();
            let c = [b.t.x, b.t.y, b.t.z];
            for i in 0..3 {
                lo[i] = lo[i].min(c[i] - h[i]);
                hi[i] = hi[i].max(c[i] + h[i]);
            }
        }
        let o = [0, 1, 2].map(|i| (lo[i] + hi[i]) / 2.0);
        let txn = self.db().begin_write().map_err(err)?;
        let copy = {
            let scene = scenes::add_scene_in(&txn, name, 0)?;
            mark(&txn, scene)?;
            let room = scenes::room_in(&txn)?;
            for (i, b) in bs.iter().enumerate() {
                let mut t = b.t;
                (t.x, t.y, t.z) = (t.x - o[0], t.y - o[1], t.z - o[2]);
                t.wire = wires
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|entry| super::wiring::WireRef { entry });
                let def = blocks::def_of(b.kind, &b.texture)?;
                scenes::place_in(
                    &txn,
                    scene,
                    false,
                    SKind::Block,
                    def,
                    None,
                    vec![blocks::placement(&t, 0)],
                    &[],
                )?;
                let (_, _, n) = blocks::split_id(&b.id).ok_or("an address")?;
                scenes::place_in(&txn, room, false, SKind::Block, def, None, vec![], &[n])?;
            }
            let n = scenes::place_in(
                &txn,
                room,
                false,
                SKind::Prefab,
                Def::Scene(scene),
                None,
                vec![Placement::at(0, o).for_kind(SKind::Prefab)],
                &[],
            )?[0];
            (scene, copy_id(scene, n))
        };
        txn.commit().map_err(err)?;
        Ok(copy)
    }

    /// a new prefab from blocks given relative to its origin (a merge brings them); its id
    pub fn put_prefab(&self, name: &str, parts: &[(Kind, String, Transform)]) -> GResult<u64> {
        let txn = self.db().begin_write().map_err(err)?;
        let scene = scenes::add_scene_in(&txn, name, 0)?;
        mark(&txn, scene)?;
        for (k, tex, t) in parts {
            let def = blocks::def_of(*k, tex)?;
            scenes::place_in(
                &txn,
                scene,
                false,
                SKind::Block,
                def,
                None,
                vec![blocks::placement(&t.checked()?, 0)],
                &[],
            )?;
        }
        txn.commit().map_err(err)?;
        Ok(scene)
    }

    /// places a copy of a prefab in the room; its id
    pub fn add_copy(&self, scene: u64, at: [f32; 3], rot: [f32; 4], scale: f32) -> GResult<String> {
        if !self.prefabs()?.iter().any(|s| s.id == scene) {
            return Err(format!("no prefab {scene}"));
        }
        let room = match self.room()? {
            Some(r) => r,
            None => {
                let txn = self.db().begin_write().map_err(err)?;
                let r = scenes::room_in(&txn)?;
                txn.commit().map_err(err)?;
                r
            }
        };
        let n = self.place(
            room,
            false,
            SKind::Prefab,
            Def::Scene(scene),
            None,
            vec![copy_placement(0, at, rot, scale, None)?],
        )?[0];
        Ok(copy_id(scene, n))
    }

    /// a new copy with its own values (`own`, as a copy keeps them): a copy of a copy, or a
    /// removed copy put back
    pub fn add_copy_with(
        &self,
        scene: u64,
        at: [f32; 3],
        rot: [f32; 4],
        scale: f32,
        own: Option<Map<String, Value>>,
    ) -> GResult<String> {
        let id = self.add_copy(scene, at, rot, scale)?;
        if own.as_ref().is_some_and(|m| !m.is_empty()) {
            let (_, mut p) = self.copy_placement(&id)?;
            p.place.set_own(own);
            let room = self.room()?.ok_or("no room")?;
            self.place(room, false, SKind::Prefab, p.def, None, vec![p.place])?;
        }
        Ok(id)
    }

    /// a copy's own values (overrides of its parts), as it keeps them
    pub fn copy_own(&self, id: &str) -> GResult<Option<Map<String, Value>>> {
        Ok(self.copy_placement(id)?.1.place.own())
    }

    /// moves, turns or sizes a copy (its own values stay)
    pub fn set_copy(&self, id: &str, at: [f32; 3], rot: [f32; 4], scale: f32) -> GResult<Copy> {
        let (_, p) = self.copy_placement(id)?;
        let own = p.place.own();
        let room = self.room()?.ok_or("no room")?;
        self.place(
            room,
            false,
            SKind::Prefab,
            p.def,
            None,
            vec![copy_placement(p.place.n, at, rot, scale, own)?],
        )?;
        Ok(self.copy_placement(id)?.0)
    }

    /// takes a copy away; its placement (to put back)
    pub fn delete_copy(&self, id: &str) -> GResult<Placed> {
        let (_, p) = self.copy_placement(id)?;
        let room = self.room()?.ok_or("no room")?;
        self.unplace(room, false, SKind::Prefab, p.def, None, &[p.place.n])?;
        Ok(p)
    }

    /// puts a copy back as it was (an undo of [`Graph::delete_copy`])
    pub fn put_copy(&self, p: Placed) -> GResult<()> {
        let room = self.room()?.ok_or("no room")?;
        self.place(room, false, SKind::Prefab, p.def, None, vec![p.place])?;
        Ok(())
    }

    /// drops a copy's own values: it shows the prefab as it is again
    pub fn reset_copy(&self, id: &str) -> GResult<()> {
        let (_, mut p) = self.copy_placement(id)?;
        p.place.set_own(None);
        let room = self.room()?.ok_or("no room")?;
        self.place(room, false, SKind::Prefab, p.def, None, vec![p.place])?;
        Ok(())
    }

    /// makes a copy's own values the prefab's. With `others_follow`, every other copy
    /// shows the change (their own values still shadow it); without, the other copies
    /// keep the prefab as it was, as a prefab of its own (`<name> (before)`). The prefab
    /// the other copies now show, when it's another
    pub fn apply_copy(&self, id: &str, others_follow: bool) -> GResult<Option<u64>> {
        let (c, p) = self.copy_placement(id)?;
        let own = overrides(&p.place);
        let parts = self.prefab_parts(c.scene)?;
        let mut kept = None;
        if !others_follow && self.copies()?.iter().any(|o| o.scene == c.scene && o.id != c.id) {
            let name = format!("{} (before)", c.name);
            let old: Vec<(Kind, String, Transform)> = parts.iter().map(|b| (b.kind, b.texture.clone(), b.t)).collect();
            let s = self.put_prefab(&name, &old)?;
            let room = self.room()?.ok_or("no room")?;
            for o in self.room_prefab_placements()? {
                if o.def == Def::Scene(c.scene) && o.place.n != p.place.n {
                    self.unplace(room, false, SKind::Prefab, o.def, None, &[o.place.n])?;
                    let mut q = o.place.clone();
                    q.n = 0;
                    self.place(room, false, SKind::Prefab, Def::Scene(s), None, vec![q])?;
                }
            }
            kept = Some(s);
        }
        // the parts as this copy shows them, written into the prefab
        let txn = self.db().begin_write().map_err(err)?;
        for b in parts {
            let Some(o) = own.get(&b.id) else { continue };
            let (_, _, n) = blocks::split_id(&b.id).ok_or("an address")?;
            let def = blocks::def_of(b.kind, &b.texture)?;
            let mut nb = shadowed(b.clone(), Some(o));
            // a wiring of its own becomes the prefab's (an entry, as wirings are)
            if let Some(w) = o.get("wiring") {
                let text = serde_json::to_string(w).map_err(err)?;
                nb.t.wire = Some(super::wiring::WireRef {
                    entry: self.save_text(&text, super::record::mime::WIRING)?.0,
                });
            }
            scenes::place_in(&txn, c.scene, false, SKind::Block, def, None, vec![], &[n])?;
            let mut pl = blocks::placement(&nb.t, n);
            pl.n = if nb.kind == b.kind && nb.texture == b.texture {
                n
            } else {
                0
            };
            scenes::place_in(
                &txn,
                c.scene,
                false,
                SKind::Block,
                blocks::def_of(nb.kind, &nb.texture)?,
                None,
                vec![pl],
                &[],
            )?;
        }
        txn.commit().map_err(err)?;
        self.reset_copy(id)?;
        Ok(kept)
    }

    /// turns a copy into ordinary blocks where its parts are (it stops following the
    /// prefab); the new blocks' ids, each with the part it was
    pub fn detach_copy(&self, id: &str) -> GResult<Vec<(String, String)>> {
        let parts = self.copy_parts(id)?;
        let made = self.add_blocks(
            &parts
                .iter()
                .map(|b| (b.kind, b.texture.clone(), b.t))
                .collect::<Vec<_>>(),
        )?;
        self.delete_copy(id)?;
        Ok(parts.into_iter().map(|b| b.id).zip(made).collect())
    }

    /// removes a prefab with its copies in the room; how many copies went
    pub fn delete_prefab(&self, scene: u64) -> GResult<usize> {
        if !self.prefabs()?.iter().any(|s| s.id == scene) {
            return Err(format!("no prefab {scene}"));
        }
        let gone: Vec<String> = self
            .copies()?
            .into_iter()
            .filter(|c| c.scene == scene)
            .map(|c| c.id)
            .collect();
        for c in &gone {
            self.delete_copy(c)?;
        }
        self.delete_scene(scene)?;
        Ok(gone.len())
    }
}

/// marks a scene as a prefab, inside a write
fn mark(txn: &redb::WriteTransaction, scene: u64) -> GResult<()> {
    use redb::ReadableTable;
    let mut t = txn.open_table(SCENES).map_err(err)?;
    let mut s: Scene = match t.get(scene).map_err(err)? {
        Some(v) => serde_json::from_slice(v.value()).map_err(err)?,
        None => return Err(format!("no scene {scene}")),
    };
    s.settings = json!({ "prefab": true });
    t.insert(scene, serde_json::to_vec(&s).map_err(err)?.as_slice())
        .map_err(err)?;
    Ok(())
}

fn copy_placement(
    n: u64,
    at: [f32; 3],
    rot: [f32; 4],
    scale: f32,
    own: Option<Map<String, Value>>,
) -> GResult<Placement> {
    if !at.iter().all(|v| v.is_finite()) || !scale.is_finite() || scale <= 0.0 {
        return Err("a copy's place and size are numbers (its size above 0)".into());
    }
    let mut p = Placement::at(n, at).for_kind(SKind::Prefab);
    let len = rot.iter().map(|x| x * x).sum::<f32>().sqrt();
    if len.is_finite() && len > 1e-6 && rot != IDENTITY {
        p.rot = Some(pack_rotation(rot.map(|x| x / len)));
    }
    p.scale = Scale::Uniform(scale.clamp(0.01, 100.0)).canonical();
    p.set_own(own);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, z: f32) -> Transform {
        Transform {
            x,
            z,
            ..Transform::default()
        }
    }

    #[test]
    fn prefabs_group_copy_and_inherit() {
        let g = Graph::in_memory().unwrap();
        let ids = g
            .add_blocks(&[
                (Kind::Plain, "#ff0000".into(), at(0.0, 0.0)),
                (Kind::Plain, "#00ff00".into(), at(2.0, 0.0)),
                (Kind::Plain, "#0000ff".into(), at(9.0, 9.0)),
            ])
            .unwrap();
        let (scene, copy) = g.make_prefab("pair", &ids[..2], &[None, None]).unwrap();
        // the two went into the prefab; one copy stands where they were
        assert_eq!(g.blocks(None).unwrap().len(), 1);
        assert_eq!(g.prefabs().unwrap()[0].name, "pair");
        let parts = g.copy_parts(&copy).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].id, format!("{copy}/plain:#00ff00#1"));
        let x: Vec<f32> = g.prefab_parts(scene).unwrap().iter().map(|b| b.t.x).collect();
        assert_eq!(x, [1.0, -1.0]);
        assert_eq!(g.world_blocks().unwrap().len(), 3);

        // a second copy, a quarter turn about y, twice the size, at 20
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let two = g.add_copy(scene, [20.0, 0.0, 0.0], [0.0, s, 0.0, s], 2.0).unwrap();
        let red = g.part(&format!("{two}/plain:#ff0000#1")).unwrap().unwrap();
        // -1 along x, turned a quarter about y, is +2 along z at twice the size
        assert!(
            (red.t.x - 20.0).abs() < 1e-3 && (red.t.z - 2.0).abs() < 1e-3,
            "{:?}",
            red.t
        );
        assert!((red.t.w - 2.0).abs() < 1e-6);

        // moving a part in one copy is that copy's own
        let mut t = red.t;
        t.y = 5.0;
        g.set_part(&red.id, t).unwrap();
        assert!((g.part(&red.id).unwrap().unwrap().t.y - 5.0).abs() < 1e-4);
        let first_red = g.part(&format!("{copy}/plain:#ff0000#1")).unwrap().unwrap();
        assert_eq!(first_red.t.y, 0.0);
        g.retexture_part(&red.id, Kind::Plain, "#123456").unwrap();
        assert_eq!(g.part(&red.id).unwrap().unwrap().texture, "#123456");

        // applying it to the prefab while keeping the first copy as it was
        let kept = g.apply_copy(&two, false).unwrap().unwrap();
        assert_eq!(g.prefabs().unwrap().len(), 2);
        assert!(g.copies().unwrap().iter().any(|c| c.scene == kept));
        let now: Vec<String> = g
            .prefab_parts(scene)
            .unwrap()
            .iter()
            .map(|b| b.texture.clone())
            .collect();
        assert!(now.contains(&"#123456".to_string()));
        assert_eq!(g.copies().unwrap().iter().find(|c| c.scene == scene).unwrap().own, 0);

        // detaching makes ordinary blocks
        let made = g.detach_copy(&two).unwrap();
        assert_eq!(made.len(), 2);
        assert_eq!(g.blocks(None).unwrap().len(), 3);
        assert_eq!(g.delete_prefab(kept).unwrap(), 1);
        assert!(g.copies().unwrap().is_empty());
        assert_eq!(
            inner_ids(&[(Kind::Plain, "#aaaaaa".into()), (Kind::Plain, "#aaaaaa".into())])[1],
            "plain:#aaaaaa#2"
        );
    }
}
