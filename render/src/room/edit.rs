//! Building in the room, as space.js did it: a block picked shows the gizmo (arrows move
//! it along an axis, the knobs on its faces size it, the rings turn it, the middle ball
//! drags it in the view's plane; Shift snaps); a placed prefab is one copy, picked, moved
//! and turned as a whole and opened with a double click to edit its parts; Shift+click
//! and a box drawn on empty room select many (blocks, copies, texts, lights), which the
//! gizmo moves and turns together; groups (Ctrl+G) are picked as one and open with a
//! double click. Every change goes to Rust as the same events, and Ctrl+Z takes it back.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec2, Vec3};
use serde_json::{Value, json};

use super::{Mods, Room, Undo};
use crate::prims::{self, Tris};
use crate::{Mesh, Pass, Rect};

/// one of many selected: a block (b), a prefab's copy (c), a text's placement (t) or a
/// light (l), by its id
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: char,
    pub id: Value,
}

impl Item {
    fn new(kind: char, id: impl Into<Value>) -> Item {
        Item { kind, id: id.into() }
    }

    fn key(&self) -> String {
        format!("{}:{}", self.kind, ids(&self.id))
    }

    fn sid(&self) -> String {
        ids(&self.id)
    }

    fn json(&self) -> Value {
        json!({ "kind": self.kind.to_string(), "id": self.id })
    }
}

/// an id as space.js wrote it into a key (a string as it is, a number as digits)
fn ids(v: &Value) -> String {
    v.as_str().map(String::from).unwrap_or_else(|| v.to_string())
}

/// what the gizmo handles: a block, a copy (its origin and turn, its parts' box), or many
/// (their middle)
#[derive(Clone, Debug)]
pub struct Target {
    group: bool,
    multi: bool,
    origin: Vec3,
    id: String,
    at: Vec3,
    rot: Quat,
    size: Vec3,
    scale: f32,
    name: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Plane,
    Move,
    Scale,
    Rotate,
}

#[derive(Clone, Debug)]
enum Which {
    Block(String),
    Copy(String),
    Multi,
}

/// one of many where it was when a drag began
#[derive(Clone, Debug)]
enum Start {
    Block {
        id: String,
        from: Value,
        p: Vec3,
        q: Quat,
    },
    Copy {
        id: String,
        origin: Vec3,
        rot: Quat,
        parts: Vec<(String, Vec3, Quat)>,
    },
    Text {
        key: String,
        p: Vec3,
        from: (f32, f32, f32, bool),
    },
    Light {
        id: Value,
        p: Vec3,
        from: Vec3,
    },
}

/// a gizmo drag
#[derive(Clone, Debug)]
struct BDrag {
    which: Which,
    mode: Mode,
    axis: usize,
    along: Option<Vec3>,
    x0: f32,
    y0: f32,
    from: Target,
    from_b: Value,
    cur: Target,
    depth: f32,
    moved: bool,
    sdir: Option<Vec2>,
    off: Vec3,
    centre: Vec2,
    a0: f32,
    toward: bool,
    parts: Vec<(String, Vec3, Quat)>,
    items: Vec<Start>,
}

/// a box being drawn on empty room
#[derive(Clone, Debug)]
struct BoxSel {
    x0: f32,
    y0: f32,
    x: f32,
    y: f32,
    add: bool,
    shown: bool,
    m: Mods,
}

/// a part of the gizmo: its triangles (in the gizmo's frame), colour, handle, and
/// whether it's drawn (the grips are fatter and unseen, easier to hit)
struct Part {
    tris: Tris,
    colour: [f32; 4],
    mode: Mode,
    axis: usize,
    drawn: bool,
    order: u8,
}

const AXIS_COLOURS: [&str; 3] = ["#d0453a", "#3f9b4b", "#3a6fd0"];

fn straight(css: &str, a: f32) -> [f32; 4] {
    let c = Room::colour(css, 1.0);
    [c[0], c[1], c[2], a]
}

fn axis(i: usize) -> Vec3 {
    [Vec3::X, Vec3::Y, Vec3::Z][i.min(2)]
}

fn gizmo_parts() -> Vec<Part> {
    let mut out = Vec::new();
    let place = |t: Tris, at: Vec3, q: Quat| -> Tris { t.into_iter().map(|p| q * p + at).collect() };
    for (i, col) in AXIS_COLOURS.iter().enumerate() {
        let a = axis(i);
        let q = prims::turn(Vec3::Y, a);
        let c = straight(col, 0.95);
        out.push(Part {
            tris: place(prims::cylinder(0.025, 0.025, 0.8, 8), a * 0.4, q),
            colour: c,
            mode: Mode::Move,
            axis: i,
            drawn: true,
            order: 1,
        });
        out.push(Part {
            tris: place(prims::cone(0.08, 0.22, 14), a * 0.9, q),
            colour: c,
            mode: Mode::Move,
            axis: i,
            drawn: true,
            order: 1,
        });
        out.push(Part {
            tris: place(prims::cylinder(0.1, 0.1, 1.05, 8), a * 0.52, q),
            colour: c,
            mode: Mode::Move,
            axis: i,
            drawn: false,
            order: 1,
        });
    }
    for (i, col) in AXIS_COLOURS.iter().enumerate() {
        let q = prims::turn(Vec3::Z, axis(i));
        let c = straight(col, 0.75);
        out.push(Part {
            tris: place(prims::torus(1.25, 0.018, 8, 72), Vec3::ZERO, q),
            colour: c,
            mode: Mode::Rotate,
            axis: i,
            drawn: true,
            order: 0,
        });
        out.push(Part {
            tris: place(prims::torus(1.25, 0.09, 6, 48), Vec3::ZERO, q),
            colour: c,
            mode: Mode::Rotate,
            axis: i,
            drawn: false,
            order: 0,
        });
    }
    out.push(Part {
        tris: prims::ball(0.1, 16, 12),
        colour: straight("#f0e6d0", 0.95),
        mode: Mode::Plane,
        axis: 0,
        drawn: true,
        order: 3,
    });
    out
}

/// the building state of the room
pub struct Edit {
    pub sel_block: Option<String>,
    pub sel_copy: Option<String>,
    copy_o: Option<Target>,
    pub open_copy: Option<String>,
    copy_info: HashMap<String, Value>,
    pub multi: Vec<Item>,
    multi_o: Option<Target>,
    groups: HashMap<String, Value>,
    parent: HashMap<String, String>,
    pub sel_group: Option<String>,
    open_groups: Vec<String>,
    bdrag: Option<BDrag>,
    box_sel: Option<BoxSel>,
    pub lights_shown: bool,
    light_picked: Option<Value>,
    parts: Vec<Part>,
    knob: Tris,
    last_up: Option<(f32, f32, f32)>,
}

impl Default for Edit {
    fn default() -> Self {
        Edit {
            sel_block: None,
            sel_copy: None,
            copy_o: None,
            open_copy: None,
            copy_info: HashMap::new(),
            multi: Vec::new(),
            multi_o: None,
            groups: HashMap::new(),
            parent: HashMap::new(),
            sel_group: None,
            open_groups: Vec::new(),
            bdrag: None,
            box_sel: None,
            lights_shown: false,
            light_picked: None,
            parts: gizmo_parts(),
            knob: prims::cuboid(Vec3::splat(0.16)),
            last_up: None,
        }
    }
}

/// a block's fields as space.js sent them back (`blockOut`)
pub fn block_out(b: &Value) -> Value {
    let mut o = json!({
        "id": b["id"], "kind": b["kind"], "texture": b["texture"],
        "x": b["x"], "y": b["y"], "z": b["z"], "w": b["w"], "h": b["h"], "d": b["d"],
        "rot": if b["rot"].is_array() { b["rot"].clone() } else { json!([0, 0, 0, 1]) },
        "bend": b.get("bend").cloned().unwrap_or(Value::Null),
        "faces": b.get("faces").cloned().unwrap_or(Value::Null),
        "shape": b["shape"].as_str().unwrap_or("box"),
        "zone": b.get("zone").cloned().unwrap_or(Value::Null),
        "anim": b.get("anim").cloned().unwrap_or(Value::Null),
    });
    for k in ["variant", "part_of", "sound"] {
        if let Some(v) = b.get(k).filter(|v| !v.is_null() && **v != false) {
            o[k] = v.clone();
        }
    }
    o
}

fn r4(v: f32) -> f64 {
    ((v as f64) * 1e4).round() / 1e4
}

fn quat_json(q: Quat) -> Value {
    json!([q.x, q.y, q.z, q.w])
}

fn quat_of(v: &Value) -> Quat {
    let r: Vec<f32> = v
        .as_array()
        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect())
        .unwrap_or_default();
    if r.len() == 4 {
        Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize()
    } else {
        Quat::IDENTITY
    }
}

fn num(v: &Value, k: &str, d: f32) -> f32 {
    v[k].as_f64().map(|x| x as f32).unwrap_or(d)
}

impl Room {
    // ------------------------------------------------------------------ the camera's picture

    /// a point of the room on the picture, in pixels (none behind the camera)
    pub(crate) fn to_px(&self, v: Vec3) -> Option<Vec2> {
        let (w, h) = self.size;
        let focal = h / 2.0 / (25f32).to_radians().tan();
        let c = self.camera_rot().inverse() * (v - self.at);
        let dist = -c.z;
        if dist <= 1e-4 {
            return None;
        }
        Some(Vec2::new(w / 2.0 + c.x / dist * focal, h / 2.0 - c.y / dist * focal))
    }

    /// the same, also for points behind (projected through, as three.js's `project`)
    fn to_px_any(&self, v: Vec3) -> Vec2 {
        let (w, h) = self.size;
        let focal = h / 2.0 / (25f32).to_radians().tan();
        let c = self.camera_rot().inverse() * (v - self.at);
        let dist = if c.z.abs() < 1e-4 { 1e-4 } else { -c.z };
        Vec2::new(w / 2.0 + c.x / dist * focal, h / 2.0 - c.y / dist * focal)
    }

    // ------------------------------------------------------------------ blocks

    fn about_of(&self, b: &Value) -> Value {
        let key = format!(
            "{}:{}",
            b["kind"].as_str().unwrap_or(""),
            b["texture"].as_str().unwrap_or("")
        );
        self.blocks.abouts.get(&key).cloned().unwrap_or(json!({}))
    }

    pub fn pick_block(&mut self, id: Option<String>) {
        // a part of a prefab's copy stands for its whole copy, unless that copy is open
        if let Some(i) = &id
            && let Some(c) = copy_of(i)
            && self.ed.open_copy.as_deref() != Some(c.as_str())
        {
            return self.pick_copy(Some(c), false);
        }
        if self.ed.sel_copy.is_some() {
            self.pick_copy(None, true);
        }
        self.ed.sel_block = id.clone();
        if id.is_some() && self.selected.is_some() {
            self.selected = None;
            self.send_ev("select", None, None, Value::Null);
        }
        let data = id
            .as_deref()
            .and_then(|i| self.blocks.get(i))
            .map(|b| {
                let mut o = block_out(b);
                o["about"] = self.about_of(b);
                o
            })
            .unwrap_or(Value::Null);
        self.send_ev("block", None, None, data);
        self.dirty = true;
    }

    /// blocks Rust sent (`blocks`); what was added is one undo step, and one may be picked
    pub(crate) fn set_blocks(&mut self, c: &Value) {
        let list = c["blocks"].as_array().cloned().unwrap_or_default();
        for b in &list {
            if let Some(id) = b["id"].as_str() {
                self.blocks.poses.remove(id);
            }
        }
        self.blocks.set(&list, c.get("textures"), c["replace"] == true);
        if c["replace"] == true && self.ed.sel_block.as_deref().is_some_and(|s| !self.blocks.has(s)) {
            self.ed.sel_block = None;
        }
        if self.ed.sel_copy.is_some() {
            self.ed.copy_o = self.ed.sel_copy.clone().and_then(|c| self.copy_target(&c));
        }
        if let Some(added) = c["added"].as_array().filter(|a| !a.is_empty()) {
            let ids = added.iter().filter_map(|x| x.as_str().map(String::from)).collect();
            self.record(Undo::BlockAdd { ids });
        }
        if let Some(s) = c.get("select").filter(|s| !s.is_null()) {
            self.pick_block(s.as_str().map(String::from));
        }
        self.dirty = true;
    }

    fn drop_block(&mut self, id: &str) {
        self.stop_sound(id);
        self.blocks.remove(&[id.to_string()]);
        self.blocks.poses.remove(id);
        if self.ed.sel_block.as_deref() == Some(id) {
            self.pick_block(None);
        }
        self.dirty = true;
    }

    /// blocks Rust deleted: out of the room, kept for an undo
    pub(crate) fn unblock(&mut self, ids: &[String]) {
        let items: Vec<(Value, Value)> = ids
            .iter()
            .filter_map(|id| self.blocks.get(id))
            .map(|b| (block_out(b), self.about_of(b)))
            .collect();
        if items.is_empty() {
            return;
        }
        self.record(Undo::Unblock { items });
        for id in ids {
            self.drop_block(id);
        }
    }

    /// a change from the block window's fields
    pub(crate) fn set_block_fields(&mut self, id: &str, t: &Value) {
        let Some(b) = self.blocks.get(id).cloned() else { return };
        self.record(Undo::Block {
            id: id.to_string(),
            t: b.clone(),
        });
        if let (Some(bb), Some(t)) = (self.blocks.get_mut(id), t.as_object()) {
            for (k, v) in t {
                bb[k] = v.clone();
            }
        }
        if let Some(b) = self.blocks.get(id).cloned() {
            self.send_ev("blockset", None, None, block_out(&b));
        }
        self.dirty = true;
    }

    pub(crate) fn restore_block(&mut self, id: &str, t: Value) {
        if let Some(b) = self.blocks.get_mut(id) {
            *b = t;
            self.blocks.poses.remove(id);
            let b = self.blocks.get(id).cloned().unwrap_or_default();
            self.send_ev("blockset", None, None, block_out(&b));
        }
        self.dirty = true;
    }

    pub(crate) fn restore_unblocked(&mut self, items: Vec<(Value, Value)>) {
        let mut textures = serde_json::Map::new();
        for (b, about) in &items {
            textures.insert(
                format!(
                    "{}:{}",
                    b["kind"].as_str().unwrap_or(""),
                    b["texture"].as_str().unwrap_or("")
                ),
                about.clone(),
            );
        }
        let list: Vec<Value> = items.into_iter().map(|(b, _)| b).collect();
        self.blocks.set(&list, Some(&Value::Object(textures)), false);
        self.send_ev("blockrestore", None, None, Value::Array(list));
        self.dirty = true;
    }

    /// where a new block goes: ahead of the camera, or at the selected text
    pub(crate) fn block_spot(&self) -> Value {
        let at = match self.selected.as_ref().and_then(|k| self.nodes.get(k)) {
            Some(n) => self.world(n),
            None => self.at + self.camera_rot() * Vec3::new(0.0, 0.0, -8.0),
        };
        json!({ "x": r2(at.x), "y": r2(at.y), "z": r2(at.z) })
    }

    /// where a new light goes: a little ahead of the camera, facing the way it looks
    pub(crate) fn light_spot(&self) -> Value {
        let q = self.camera_rot();
        let at = self.at + q * Vec3::new(0.0, 0.0, -4.0);
        let d = q * Vec3::NEG_Z;
        let r3 = |v: f32| ((v as f64) * 1e3).round() / 1e3;
        json!({ "x": r2(at.x), "y": r2(at.y), "z": r2(at.z), "dir": [r3(d.x), r3(d.y), r3(d.z)] })
    }

    // ------------------------------------------------------------------ copies as groups

    pub(crate) fn set_copies(&mut self, list: &[Value]) {
        self.ed.copy_info = list
            .iter()
            .filter_map(|c| Some((c["id"].as_str()?.to_string(), c.clone())))
            .collect();
        if self
            .ed
            .open_copy
            .as_ref()
            .is_some_and(|o| !self.ed.copy_info.contains_key(o))
        {
            self.ed.open_copy = None;
        }
        if self
            .ed
            .sel_copy
            .as_ref()
            .is_some_and(|o| !self.ed.copy_info.contains_key(o))
        {
            self.pick_copy(None, false);
        } else if let Some(c) = self.ed.sel_copy.clone() {
            self.ed.copy_o = self.copy_target(&c);
        }
        self.dirty = true;
    }

    fn parts_of(&self, cid: &str) -> Vec<String> {
        self.blocks
            .list
            .iter()
            .filter(|(k, _)| copy_of(k).as_deref() == Some(cid))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// the box round some blocks where they're drawn now
    fn box_of(&mut self, ids: &[String]) -> Option<(Vec3, Vec3)> {
        let mut out: Option<(Vec3, Vec3)> = None;
        for id in ids {
            if let Some((lo, hi)) = self.blocks.bounds(id) {
                out = Some(match out {
                    Some((a, b)) => (a.min(lo), b.max(hi)),
                    None => (lo, hi),
                });
            }
        }
        out
    }

    /// the copy as the gizmo handles it: its origin and turn, and its parts' box
    fn copy_target(&mut self, cid: &str) -> Option<Target> {
        let c = self.ed.copy_info.get(cid)?.clone();
        let origin = Vec3::new(num(&c, "x", 0.0), num(&c, "y", 0.0), num(&c, "z", 0.0));
        let parts = self.parts_of(cid);
        let (mid, size) = match self.box_of(&parts) {
            Some((lo, hi)) => ((lo + hi) / 2.0, hi - lo),
            None => (origin, Vec3::ONE),
        };
        Some(Target {
            group: true,
            multi: false,
            origin,
            id: cid.to_string(),
            at: mid,
            rot: quat_of(&c["rot"]),
            size,
            scale: num(&c, "scale", 1.0),
            name: c["name"].as_str().unwrap_or("").to_string(),
        })
    }

    pub(crate) fn pick_copy(&mut self, cid: Option<String>, quiet: bool) {
        self.ed.sel_copy = cid.clone();
        self.ed.copy_o = cid.as_deref().and_then(|c| self.copy_target(c));
        if cid.is_some() {
            self.ed.sel_block = None;
            if self.selected.is_some() {
                self.selected = None;
                self.send_ev("select", None, None, Value::Null);
            }
        }
        if !quiet {
            let data = match (&cid, &self.ed.copy_o) {
                (Some(c), Some(o)) => json!({
                    "id": c, "copy": true, "part_of": c, "name": o.name,
                    "x": o.at.x, "y": o.at.y, "z": o.at.z, "rot": quat_json(o.rot), "scale": o.scale,
                }),
                _ => Value::Null,
            };
            self.send_ev("block", None, None, data);
        }
        self.dirty = true;
    }

    /// a copy Rust removed: its parts out of the room, kept for an undo
    pub(crate) fn copy_gone(&mut self, ids: &[String], restore: Value) {
        self.record(Undo::Send {
            name: "copygone",
            kind: "copyrestore".into(),
            data: restore,
        });
        if let Some(o) = self.ed.open_copy.clone()
            && ids.iter().any(|i| copy_of(i).as_deref() == Some(o.as_str()))
        {
            self.ed.open_copy = None;
        }
        for id in ids {
            self.drop_block(id);
        }
        if self.ed.sel_copy.is_some() {
            self.pick_copy(None, false);
        }
    }

    pub(crate) fn open_copy_named(&mut self, cid: &str) -> bool {
        match self.parts_of(cid).first().cloned() {
            Some(first) => self.open_copy_at(&first),
            None => false,
        }
    }

    /// a double click on a part: its copy opens, the part selected
    fn open_copy_at(&mut self, id: &str) -> bool {
        let Some(cid) = copy_of(id) else { return false };
        self.ed.open_copy = Some(cid.clone());
        self.ed.sel_copy = None;
        self.ed.copy_o = None;
        self.pick_block(Some(id.to_string()));
        self.send_ev("copyopen", None, None, json!(cid));
        true
    }

    pub(crate) fn close_copy(&mut self) -> bool {
        let Some(cid) = self.ed.open_copy.take() else {
            return false;
        };
        self.ed.sel_block = None;
        self.pick_copy(Some(cid), false);
        true
    }

    // ------------------------------------------------------------------ the selection of many

    fn seed_multi(&mut self) {
        if let Some(c) = self.ed.sel_copy.clone() {
            self.add_item(Item::new('c', c));
        } else if let Some(b) = self.ed.sel_block.clone() {
            self.add_item(Item::new('b', b));
        }
        if let Some(t) = self.selected.clone() {
            self.add_item(Item::new('t', t));
        }
    }

    fn has_item(&self, key: &str) -> bool {
        self.ed.multi.iter().any(|x| x.key() == key)
    }

    fn add_item(&mut self, x: Item) {
        if !self.has_item(&x.key()) {
            self.ed.multi.push(x);
        }
    }

    fn drop_item(&mut self, key: &str) {
        self.ed.multi.retain(|x| x.key() != key);
    }

    pub(crate) fn toggle_multi(&mut self, kind: char, id: Value) {
        self.ed.sel_group = None;
        let x = Item { kind, id };
        // a thing in a group comes and goes with its whole group
        if matches!(kind, 'b' | 'c')
            && let Some(g) = self.outer(&x.key())
        {
            if self.ed.multi.is_empty() {
                self.seed_multi();
            }
            let ls = self.leaves(&g);
            let all = ls.iter().all(|y| self.has_item(&y.key()));
            for y in ls {
                if all {
                    self.drop_item(&y.key());
                } else {
                    self.add_item(y);
                }
            }
            self.after_multi(false);
            return;
        }
        if self.ed.multi.is_empty() {
            self.seed_multi();
        }
        let k = x.key();
        if self.has_item(&k) {
            self.drop_item(&k);
        } else {
            self.ed.multi.push(x);
        }
        self.after_multi(false);
    }

    fn set_multi(&mut self, items: Vec<Item>, add: bool) {
        if !add {
            self.ed.multi.clear();
        } else if self.ed.multi.is_empty() {
            self.seed_multi();
        }
        for x in items {
            self.add_item(x);
        }
        self.after_multi(false);
    }

    pub(crate) fn clear_multi(&mut self) -> bool {
        self.ed.sel_group = None;
        if self.ed.multi.is_empty() {
            return false;
        }
        self.ed.multi.clear();
        self.after_multi(false);
        true
    }

    fn light_of(&self, id: &Value) -> Option<&Value> {
        self.blocks.lights.iter().find(|l| l["id"] == *id)
    }

    fn item_there(&self, x: &Item) -> bool {
        match x.kind {
            'b' => self.blocks.has(&x.sid()),
            'c' => self.ed.copy_info.contains_key(&x.sid()),
            't' => self.nodes.contains_key(&x.sid()),
            _ => self.light_of(&x.id).is_some(),
        }
    }

    /// where an item is, in the room
    fn item_at(&mut self, x: &Item) -> Vec3 {
        match x.kind {
            'b' => self
                .blocks
                .model(&x.sid())
                .map(|m| m.w_axis.truncate())
                .unwrap_or_default(),
            'c' => {
                let parts = self.parts_of(&x.sid());
                self.box_of(&parts).map(|(a, b)| (a + b) / 2.0).unwrap_or_default()
            }
            't' => self.nodes.get(&x.sid()).map(|n| self.world(n)).unwrap_or_default(),
            _ => self
                .light_of(&x.id)
                .map(|l| Vec3::new(num(l, "x", 0.0), num(l, "y", 0.0), num(l, "z", 0.0)))
                .unwrap_or_default(),
        }
    }

    pub(crate) fn after_multi(&mut self, quiet: bool) {
        let gone: Vec<String> = self
            .ed
            .multi
            .iter()
            .filter(|x| !self.item_there(x))
            .map(Item::key)
            .collect();
        for k in gone {
            self.drop_item(&k);
        }
        if !self.ed.multi.is_empty() {
            // the one selected becomes one of many
            if self.ed.sel_block.is_some() || self.ed.sel_copy.is_some() {
                self.ed.sel_block = None;
                self.ed.sel_copy = None;
                self.ed.copy_o = None;
                self.send_ev("block", None, None, Value::Null);
            }
            if self.selected.is_some() {
                self.selected = None;
                self.send_ev("select", None, None, Value::Null);
            }
        }
        self.ed.multi_o = (!self.ed.multi.is_empty()).then(|| self.multi_target());
        if !quiet {
            let mut n = [0usize; 4];
            for x in &self.ed.multi {
                n[match x.kind {
                    'b' => 0,
                    'c' => 1,
                    't' => 2,
                    _ => 3,
                }] += 1;
            }
            let g = self.ed.sel_group.as_ref().and_then(|g| self.ed.groups.get(g));
            let data = json!({
                "count": self.ed.multi.len(), "blocks": n[0], "copies": n[1], "texts": n[2], "lights": n[3],
                "group": g.map(|g| g["id"].clone()), "name": g.map(|g| g["name"].as_str().unwrap_or("").to_string()),
            });
            self.send_ev("multi", None, None, data);
        }
        self.dirty = true;
    }

    /// the many as the gizmo handles them: their middle
    fn multi_target(&mut self) -> Target {
        let items = self.ed.multi.clone();
        let mut c = Vec3::ZERO;
        for x in &items {
            c += self.item_at(x);
        }
        c /= items.len().max(1) as f32;
        Target {
            group: true,
            multi: true,
            origin: c,
            id: "many".into(),
            at: c,
            rot: Quat::IDENTITY,
            size: Vec3::ONE,
            scale: 1.0,
            name: String::new(),
        }
    }

    fn pose_of(&self, id: &str) -> (Vec3, Quat) {
        let m = self.blocks.model(id).unwrap_or(Mat4::IDENTITY);
        let (_, q, p) = m.to_scale_rotation_translation();
        (p, q)
    }

    fn multi_start(&mut self) -> Vec<Start> {
        let mut out = Vec::new();
        for x in self.ed.multi.clone() {
            match x.kind {
                'b' => {
                    let id = x.sid();
                    let Some(b) = self.blocks.get(&id).cloned() else {
                        continue;
                    };
                    let (p, q) = self.pose_of(&id);
                    out.push(Start::Block { id, from: b, p, q });
                }
                'c' => {
                    let id = x.sid();
                    let Some(c) = self.ed.copy_info.get(&id).cloned() else {
                        continue;
                    };
                    let parts = self.parts_of(&id).into_iter().map(|p| {
                        let (a, q) = self.pose_of(&p);
                        (p, a, q)
                    });
                    out.push(Start::Copy {
                        origin: Vec3::new(num(&c, "x", 0.0), num(&c, "y", 0.0), num(&c, "z", 0.0)),
                        rot: quat_of(&c["rot"]),
                        parts: parts.collect(),
                        id,
                    });
                }
                't' => {
                    let key = x.sid();
                    let Some(n) = self.nodes.get(&key) else { continue };
                    out.push(Start::Text {
                        p: self.world(n),
                        from: (n.x, n.y, n.z, n.pinned),
                        key,
                    });
                }
                _ => {
                    let Some(l) = self.light_of(&x.id) else { continue };
                    let p = Vec3::new(num(l, "x", 0.0), num(l, "y", 0.0), num(l, "z", 0.0));
                    out.push(Start::Light {
                        id: x.id.clone(),
                        p,
                        from: p,
                    });
                }
            }
        }
        out
    }

    /// the many follow the gizmo: moved, and turned about their middle
    fn move_multi(&mut self, d: &BDrag) {
        let turn = d.cur.rot * d.from.rot.inverse();
        let (p0, p1) = (d.from.at, d.cur.at);
        let to = |p: Vec3| turn * (p - p0) + p1;
        for s in &d.items {
            match s {
                Start::Block { id, p, q, .. } => {
                    self.blocks
                        .poses
                        .insert(id.clone(), Mat4::from_rotation_translation(turn * *q, to(*p)));
                }
                Start::Copy { parts, .. } => {
                    for (id, p, q) in parts {
                        self.blocks
                            .poses
                            .insert(id.clone(), Mat4::from_rotation_translation(turn * *q, to(*p)));
                    }
                }
                Start::Text { key, p, .. } => {
                    if let Some(n) = self.nodes.get_mut(key) {
                        n.pinned = true;
                    }
                    self.set_local(key, to(*p));
                }
                Start::Light { id, p, .. } => {
                    let w = to(*p);
                    if let Some(l) = self.blocks.lights.iter_mut().find(|l| l["id"] == *id) {
                        l["x"] = json!(w.x);
                        l["y"] = json!(w.y);
                        l["z"] = json!(w.z);
                    }
                }
            }
        }
        self.blocks.rev += 1;
        self.wake(0.3);
    }

    /// a drag of many ended: each kept where it went, one step to undo
    fn end_multi(&mut self, d: &BDrag) {
        let turn = d.cur.rot * d.from.rot.inverse();
        let (p0, p1) = (d.from.at, d.cur.at);
        self.set_batch(true);
        for s in &d.items {
            match s {
                Start::Block { id, from, .. } => {
                    self.record(Undo::Block {
                        id: id.clone(),
                        t: from.clone(),
                    });
                    let (p, q) = self.pose_of(id);
                    self.blocks.poses.remove(id);
                    if let Some(b) = self.blocks.get_mut(id) {
                        b["x"] = json!(r4(p.x));
                        b["y"] = json!(r4(p.y));
                        b["z"] = json!(r4(p.z));
                        b["rot"] = quat_json(q);
                    }
                    let b = self.blocks.get(id).cloned().unwrap_or_default();
                    self.send_ev("blockset", None, None, block_out(&b));
                }
                Start::Copy { id, origin, rot, .. } => {
                    let o1 = turn * (*origin - p0) + p1;
                    let r = turn * *rot;
                    self.record(Undo::Send {
                        name: "copy",
                        kind: "copyset".into(),
                        data: json!({ "id": id, "x": origin.x, "y": origin.y, "z": origin.z, "rot": quat_json(*rot) }),
                    });
                    self.send_ev(
                        "copyset",
                        None,
                        None,
                        json!({ "id": id, "x": r4(o1.x), "y": r4(o1.y), "z": r4(o1.z), "rot": quat_json(r) }),
                    );
                }
                Start::Text { key, from, .. } => {
                    let (x, y, z, pinned) = *from;
                    self.record(Undo::Move {
                        n: key.clone(),
                        x,
                        y,
                        z,
                        pinned,
                    });
                    self.send_ev("moved", Some(key), None, Value::Null);
                }
                Start::Light { id, from, .. } => {
                    let Some(l) = self.light_of(id).cloned() else { continue };
                    self.record(Undo::Light {
                        id: id.clone(),
                        t: json!({ "x": from.x, "y": from.y, "z": from.z }),
                    });
                    self.send_ev("lightset", None, None, json!({ "id": id, "x": r4(num(&l, "x", 0.0)), "y": r4(num(&l, "y", 0.0)), "z": r4(num(&l, "z", 0.0)) }));
                }
            }
        }
        self.set_batch(false);
        self.after_multi(true);
    }

    /// Delete with many selected: texts out of the room and blocks deleted here, the
    /// rest (copies, lights) by Rust, which closes the undo step when it's done
    pub(crate) fn remove_multi(&mut self) -> Value {
        if self.ed.multi.is_empty() {
            return Value::Null;
        }
        let items = std::mem::take(&mut self.ed.multi);
        self.set_batch(true);
        let blocks: Vec<String> = items.iter().filter(|x| x.kind == 'b').map(Item::sid).collect();
        for x in items.iter().filter(|x| x.kind == 't') {
            self.remove_node(&x.sid());
        }
        if !blocks.is_empty() {
            self.unblock(&blocks);
            self.send_ev("blockgone", None, None, json!(blocks));
        }
        let lights: Vec<Value> = items
            .iter()
            .filter(|x| x.kind == 'l')
            .filter_map(|x| self.light_of(&x.id).cloned())
            .collect();
        for l in &lights {
            self.record(Undo::Send {
                name: "lightgone",
                kind: "lightrestore".into(),
                data: l.clone(),
            });
        }
        let copies: Vec<String> = items.iter().filter(|x| x.kind == 'c').map(Item::sid).collect();
        if copies.is_empty() {
            self.set_batch(false);
        }
        self.after_multi(false);
        json!({ "count": items.len(), "copies": copies, "lights": lights.iter().map(|l| l["id"].clone()).collect::<Vec<_>>() })
    }

    // ------------------------------------------------------------------ groups

    pub(crate) fn set_groups(&mut self, list: &[Value]) {
        self.ed.groups = list
            .iter()
            .filter_map(|g| Some((ids(g.get("id")?), g.clone())))
            .collect();
        self.ed.parent.clear();
        for (gid, g) in &self.ed.groups {
            for m in g["members"].as_array().into_iter().flatten() {
                let k = format!("{}:{}", m["kind"].as_str().unwrap_or(""), ids(&m["id"]));
                self.ed.parent.insert(k, gid.clone());
            }
        }
        let groups = &self.ed.groups;
        self.ed.open_groups.retain(|g| groups.contains_key(g));
        if self
            .ed
            .sel_group
            .as_ref()
            .is_some_and(|g| !self.ed.groups.contains_key(g))
        {
            self.ed.sel_group = None;
        }
    }

    /// the outermost closed group a thing is in (an open group lets its things be picked)
    fn outer(&self, key: &str) -> Option<String> {
        let mut g = self.ed.parent.get(key).cloned();
        let mut top = None;
        while let Some(x) = g {
            if self.ed.open_groups.contains(&x) {
                break;
            }
            g = self.ed.parent.get(&format!("g:{x}")).cloned();
            top = Some(x);
        }
        top
    }

    fn leaves(&self, gid: &str) -> Vec<Item> {
        let mut out = Vec::new();
        if let Some(g) = self.ed.groups.get(gid) {
            for m in g["members"].as_array().into_iter().flatten() {
                let kind = m["kind"].as_str().and_then(|k| k.chars().next()).unwrap_or('b');
                if kind == 'g' {
                    out.extend(self.leaves(&ids(&m["id"])));
                } else {
                    out.push(Item {
                        kind,
                        id: m["id"].clone(),
                    });
                }
            }
        }
        out
    }

    pub(crate) fn pick_group(&mut self, gid: &str, add: bool) {
        let ls = self.leaves(gid);
        self.set_multi(ls, add);
        self.ed.sel_group = (!add).then(|| gid.to_string());
        self.after_multi(false);
    }

    /// what Ctrl+G groups: the things selected, each as its outermost group
    pub(crate) fn group_members(&self) -> Value {
        let mut out: Vec<Value> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for x in &self.ed.multi {
            if !matches!(x.kind, 'b' | 'c') {
                continue;
            }
            let m = match self.outer(&x.key()) {
                Some(g) => {
                    json!({ "kind": "g", "id": self.ed.groups.get(&g).map(|g| g["id"].clone()).unwrap_or(json!(g)) })
                }
                None => x.json(),
            };
            let k = format!("{}:{}", m["kind"].as_str().unwrap_or(""), ids(&m["id"]));
            if seen.insert(k) {
                out.push(m);
            }
        }
        Value::Array(out)
    }

    pub(crate) fn sel_group_id(&self) -> Value {
        self.ed
            .sel_group
            .as_ref()
            .and_then(|g| self.ed.groups.get(g))
            .map(|g| g["id"].clone())
            .unwrap_or(Value::Null)
    }

    fn open_group_at(&mut self, key: &str) -> bool {
        let Some(g) = self.outer(key) else { return false };
        self.ed.open_groups.push(g.clone());
        self.clear_multi();
        let id = self.ed.groups.get(&g).map(|x| x["id"].clone()).unwrap_or(json!(g));
        self.send_ev("groupopen", None, None, id);
        true
    }

    // ------------------------------------------------------------------ lights

    pub(crate) fn show_lights(&mut self, on: bool, picked: Option<Value>) {
        self.ed.lights_shown = on;
        self.ed.light_picked = picked.filter(|p| !p.is_null());
        self.dirty = true;
    }

    /// the point and cone lights' markers: (id, where) for the pointer
    fn light_marks(&self) -> Vec<(Value, Vec3, f32)> {
        if !self.ed.lights_shown {
            return Vec::new();
        }
        self.blocks
            .lights
            .iter()
            .filter(|l| matches!(l["kind"].as_str(), Some("point" | "spot")))
            .map(|l| {
                let picked = self.ed.light_picked.as_ref() == Some(&l["id"]);
                (
                    l["id"].clone(),
                    Vec3::new(num(l, "x", 0.0), num(l, "y", 0.0), num(l, "z", 0.0)),
                    if picked { 0.22 } else { 0.15 },
                )
            })
            .collect()
    }

    // ------------------------------------------------------------------ the gizmo

    fn target(&self) -> Option<Target> {
        if !self.ed.multi.is_empty() {
            return self.ed.multi_o.clone();
        }
        if self.ed.sel_copy.is_some() {
            return self.ed.copy_o.clone();
        }
        let id = self.ed.sel_block.as_ref()?;
        let b = self.blocks.get(id)?;
        let (at, rot) = crate::blocks::place(b);
        Some(Target {
            group: false,
            multi: false,
            origin: at,
            id: id.clone(),
            at,
            rot,
            size: Vec3::new(num(b, "w", 1.0), num(b, "h", 1.0), num(b, "d", 1.0)),
            scale: 1.0,
            name: String::new(),
        })
    }

    /// the gizmo's place and size (the same size on screen wherever it is)
    fn gizmo_frame(&self, t: &Target) -> (Mat4, f32) {
        let s = (t.at.distance(self.at) * 0.11).max(0.05);
        (
            Mat4::from_scale_rotation_translation(Vec3::splat(s), Quat::IDENTITY, t.at),
            s,
        )
    }

    fn knob_model(&self, t: &Target, i: usize, s: f32) -> Mat4 {
        let half = t.size / 2.0;
        Mat4::from_scale_rotation_translation(Vec3::splat(s), t.rot, t.at + t.rot * axis(i) * half[i])
    }

    /// the gizmo handle a ray meets (the middle ball first, then the arrows and knobs,
    /// then the rings: a ring seen edge-on crosses the middle)
    fn gizmo_hit(&self, o: Vec3, d: Vec3) -> Option<(Mode, usize)> {
        let t = self.target()?;
        let (m, s) = self.gizmo_frame(&t);
        let mut hits: Vec<(u8, f32, Mode, usize)> = Vec::new();
        let rank = |m: Mode| match m {
            Mode::Plane => 0,
            Mode::Move => 1,
            Mode::Scale => 2,
            Mode::Rotate => 3,
        };
        for p in &self.ed.parts {
            if let Some(dist) = prims::ray_tris(o, d, &p.tris, m) {
                hits.push((rank(p.mode), dist, p.mode, p.axis));
            }
        }
        if !t.group {
            for i in 0..3 {
                if let Some(dist) = prims::ray_tris(o, d, &self.ed.knob, self.knob_model(&t, i, s)) {
                    hits.push((2, dist, Mode::Scale, i));
                }
            }
        }
        hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        hits.first().map(|h| (h.2, h.3))
    }

    /// the gizmo and the outlines, as meshes drawn over everything
    pub(crate) fn edit_meshes(&mut self, out: &mut Vec<Mesh>) {
        let base = u64::MAX - 1000;
        let lines = |id: u64, segs: Vec<Vec3>, c: [f32; 4]| Mesh {
            id,
            version: segs.len() as u64
                ^ segs.first().map(|v| v.x.to_bits() as u64).unwrap_or(0)
                ^ segs.last().map(|v| (v.z.to_bits() as u64) << 7).unwrap_or(0),
            verts: std::sync::Arc::new(
                segs.into_iter()
                    .map(|p| crate::Vert {
                        pos: p.to_array(),
                        uv: [0.0, 0.0],
                        colour: c,
                    })
                    .collect(),
            ),
            texture: None,
            pass: Pass::Over,
            model: Mat4::IDENTITY,
            cull: false,
            lines: true,
        };
        // the copy's outline (dashed round an open one)
        if let Some(cid) = self.ed.sel_copy.clone().or(self.ed.open_copy.clone()) {
            let parts = self.parts_of(&cid);
            if let Some((lo, hi)) = self.box_of(&parts) {
                let (lo, hi) = (lo - 0.06, hi + 0.06);
                let open = self.ed.sel_copy.is_none() && self.ed.open_copy.as_deref() == Some(cid.as_str());
                let mut segs: Vec<Vec3> = prims::edges(hi - lo).into_iter().map(|p| p + (lo + hi) / 2.0).collect();
                if open {
                    segs = prims::dashed(&segs, 0.2, 0.12);
                }
                out.push(lines(base + 1, segs, straight("#e0a03f", 1.0)));
            }
        }
        // the many's outlines (texts are ringed where they're drawn)
        let mut segs = Vec::new();
        for x in self.ed.multi.clone() {
            let b = match x.kind {
                't' => continue,
                'b' => self.blocks.bounds(&x.sid()),
                'c' => {
                    let parts = self.parts_of(&x.sid());
                    self.box_of(&parts)
                }
                _ => {
                    let c = self.item_at(&x);
                    Some((c - 0.25, c + 0.25))
                }
            };
            let Some((lo, hi)) = b else { continue };
            let (lo, hi) = (lo - 0.05, hi + 0.05);
            segs.extend(prims::edges(hi - lo).into_iter().map(|p| p + (lo + hi) / 2.0));
        }
        if !segs.is_empty() {
            out.push(lines(base + 2, segs, straight("#3f8cff", 1.0)));
        }
        // the lights' markers
        let q = self.camera_rot();
        for (i, l) in self.blocks.lights.clone().iter().enumerate() {
            if !self.ed.lights_shown || !matches!(l["kind"].as_str(), Some("point" | "spot")) {
                continue;
            }
            let picked = self.ed.light_picked.as_ref() == Some(&l["id"]);
            let at = Vec3::new(num(l, "x", 0.0), num(l, "y", 0.0), num(l, "z", 0.0));
            let col = l["colour"].as_str().unwrap_or("#ffffff");
            let id = base + 100 + i as u64 * 4;
            let mk = |id: u64, tris: Tris, c: [f32; 4], m: Mat4, pass: Pass| Mesh {
                id,
                version: (c[0].to_bits() as u64) ^ ((c[3].to_bits() as u64) << 3) ^ tris.len() as u64,
                verts: std::sync::Arc::new(prims::verts(&tris, Mat4::IDENTITY, c)),
                texture: None,
                pass,
                model: m,
                cull: pass != Pass::Glass,
                lines: false,
            };
            let r = if picked { 0.22 } else { 0.15 };
            out.push(mk(
                id,
                prims::ball(r, 16, 12),
                straight(col, if picked { 1.0 } else { 0.8 }),
                Mat4::from_translation(at),
                Pass::Over,
            ));
            let rr = if picked { 0.34 } else { 0.26 };
            out.push(mk(
                id + 1,
                prims::torus(rr, 0.02, 6, 32),
                straight(if picked { "#ffffff" } else { "#333333" }, 1.0),
                Mat4::from_rotation_translation(q, at),
                Pass::Over,
            ));
            if l["kind"] == "spot" {
                let dir: Vec<f32> = l["dir"]
                    .as_array()
                    .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
                    .unwrap_or_default();
                let d = if dir.len() == 3 {
                    Vec3::new(dir[0], dir[1], dir[2]).normalize_or(Vec3::NEG_Y)
                } else {
                    Vec3::NEG_Y
                };
                let len = num(l, "range", 10.0).min(2.0);
                let rad = num(l, "angle", 30.0).to_radians().tan() * len;
                // an open cone (no base): its sides only
                let tris: Tris = prims::cone(rad, len, 24)
                    .as_chunks::<12>()
                    .0
                    .iter()
                    .flat_map(|c| c[..6].to_vec())
                    .collect();
                let m = Mat4::from_rotation_translation(prims::turn(Vec3::NEG_Y, d), at + d * (len / 2.0));
                out.push(mk(id + 2, tris, straight(col, 0.08), m, Pass::Glass));
            }
        }
        // the gizmo
        let Some(t) = self.target() else { return };
        let (m, s) = self.gizmo_frame(&t);
        for (k, order) in [(0u64, 0u8), (1, 1)] {
            let mut verts = Vec::new();
            for p in self.ed.parts.iter().filter(|p| p.drawn && p.order == order) {
                verts.extend(prims::verts(&p.tris, Mat4::IDENTITY, p.colour));
            }
            out.push(Mesh {
                id: base + 10 + k,
                version: 1,
                verts: std::sync::Arc::new(verts),
                texture: None,
                pass: Pass::Over,
                model: m,
                cull: true,
                lines: false,
            });
        }
        if !t.group {
            for (i, col) in AXIS_COLOURS.iter().enumerate() {
                let v = prims::verts(&self.ed.knob, Mat4::IDENTITY, straight(col, 0.95));
                out.push(Mesh {
                    id: base + 20 + i as u64,
                    version: 1,
                    verts: std::sync::Arc::new(v),
                    texture: None,
                    pass: Pass::Over,
                    model: self.knob_model(&t, i, s),
                    cull: true,
                    lines: false,
                });
            }
        }
        let ball: Vec<crate::Vert> = self
            .ed
            .parts
            .iter()
            .filter(|p| p.drawn && p.order == 3)
            .flat_map(|p| prims::verts(&p.tris, Mat4::IDENTITY, p.colour))
            .collect();
        out.push(Mesh {
            id: base + 30,
            version: 1,
            verts: std::sync::Arc::new(ball),
            texture: None,
            pass: Pass::Over,
            model: m,
            cull: true,
            lines: false,
        });
    }

    /// the box being drawn to select, on the picture
    pub(crate) fn edit_rects(&self) -> Vec<Rect> {
        match &self.ed.box_sel {
            Some(b) if b.shown => {
                let blue = Room::colour("#3f8cff", 1.0);
                vec![Rect {
                    x: b.x0.min(b.x),
                    y: b.y0.min(b.y),
                    w: (b.x - b.x0).abs(),
                    h: (b.y - b.y0).abs(),
                    fill: [blue[0] * 0.12, blue[1] * 0.12, blue[2] * 0.12, 0.12],
                    border: blue,
                }]
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn text_ringed(&self, key: &str) -> bool {
        self.ed.multi.iter().any(|x| x.kind == 't' && x.sid() == key)
    }

    // ------------------------------------------------------------------ dragging

    fn start_block_drag(&mut self, x: f32, y: f32, handle: Option<(Mode, usize)>) -> bool {
        let Some(t) = self.target() else { return false };
        if t.group && handle.is_some_and(|h| h.0 == Mode::Scale) {
            return false;
        }
        // many move only by the gizmo
        if t.multi && handle.is_none() {
            return false;
        }
        let (mode, ax) = handle.unwrap_or((Mode::Plane, 0));
        let fwd = self.camera_rot() * Vec3::NEG_Z;
        let depth = (t.at - self.at).dot(fwd);
        let along = (mode == Mode::Scale).then(|| t.rot * axis(ax));
        let centre = self.to_px_any(t.at);
        let which = if t.multi {
            Which::Multi
        } else if t.group {
            Which::Copy(t.id.clone())
        } else {
            Which::Block(t.id.clone())
        };
        let sdir = (handle.is_some() && mode != Mode::Rotate).then(|| {
            let a = along.unwrap_or(axis(ax));
            self.to_px_any(t.at + a) - self.to_px_any(t.at)
        });
        let parts = match &which {
            Which::Copy(c) => self
                .parts_of(c)
                .into_iter()
                .map(|p| {
                    let (a, q) = self.pose_of(&p);
                    (p, a, q)
                })
                .collect(),
            _ => Vec::new(),
        };
        let items = if t.multi { self.multi_start() } else { Vec::new() };
        let from_b = match &which {
            Which::Block(id) => self.blocks.get(id).cloned().unwrap_or_default(),
            _ => Value::Null,
        };
        self.ed.bdrag = Some(BDrag {
            which,
            mode,
            axis: ax,
            along,
            x0: x,
            y0: y,
            from: t.clone(),
            from_b,
            cur: t.clone(),
            depth,
            moved: false,
            sdir,
            off: t.at - self.ray_at(x, y, depth),
            centre,
            a0: (y - centre.y).atan2(x - centre.x),
            toward: axis(ax).dot(self.at - t.at) > 0.0,
            parts,
            items,
        });
        true
    }

    fn drag_block(&mut self, x: f32, y: f32, m: Mods) {
        let Some(mut d) = self.ed.bdrag.take() else { return };
        if !d.moved && (x - d.x0).hypot(y - d.y0) < 3.0 {
            self.ed.bdrag = Some(d);
            return;
        }
        d.moved = true;
        let snap = |v: f32| if m.shift { (v * 4.0).round() / 4.0 } else { v };
        let f = d.from.clone();
        match d.mode {
            Mode::Plane => {
                let w = self.ray_at(x, y, d.depth) + d.off;
                d.cur.at = Vec3::new(snap(w.x), snap(w.y), snap(w.z));
            }
            Mode::Rotate => {
                let mut ang = (y - d.centre.y).atan2(x - d.centre.x) - d.a0;
                if d.toward {
                    ang = -ang;
                }
                // Shift: steps of 15°
                if m.shift {
                    let st = std::f32::consts::PI / 12.0;
                    ang = (ang / st).round() * st;
                }
                d.cur.rot = (Quat::from_axis_angle(axis(d.axis), ang) * f.rot).normalize();
            }
            _ => {
                let mv = Vec2::new(x - d.x0, y - d.y0);
                let sd = d.sdir.unwrap_or(Vec2::X);
                let units = mv.dot(sd) / sd.length_squared().max(1e-6);
                if d.mode == Mode::Move {
                    let mut at = f.at;
                    at[d.axis] = snap(f.at[d.axis] + units);
                    d.cur.at = at;
                } else {
                    let size = snap(f.size[d.axis] + units).max(0.05);
                    d.cur.size[d.axis] = size;
                    // the opposite face stays put, along the block's own axis
                    let shift = (size - f.size[d.axis]) / 2.0;
                    d.cur.at = f.at + d.along.unwrap_or(Vec3::ZERO) * shift;
                }
            }
        }
        match &d.which {
            Which::Multi => self.move_multi(&d),
            Which::Copy(_) => {
                let turn = d.cur.rot * d.from.rot.inverse();
                for (id, p, q) in &d.parts {
                    let rel = turn * (*p - d.from.at);
                    self.blocks
                        .poses
                        .insert(id.clone(), Mat4::from_rotation_translation(turn * *q, d.cur.at + rel));
                }
                self.blocks.rev += 1;
            }
            Which::Block(id) => {
                let c = d.cur.clone();
                if let Some(b) = self.blocks.get_mut(id) {
                    b["x"] = json!(c.at.x);
                    b["y"] = json!(c.at.y);
                    b["z"] = json!(c.at.z);
                    b["rot"] = quat_json(c.rot);
                    b["w"] = json!(c.size.x);
                    b["h"] = json!(c.size.y);
                    b["d"] = json!(c.size.z);
                }
            }
        }
        // the gizmo follows
        match &d.which {
            Which::Multi => self.ed.multi_o = Some(d.cur.clone()),
            Which::Copy(_) => self.ed.copy_o = Some(d.cur.clone()),
            Which::Block(_) => {}
        }
        self.ed.bdrag = Some(d);
        self.dirty = true;
    }

    fn end_block_drag(&mut self) {
        let Some(d) = self.ed.bdrag.take() else { return };
        if !d.moved {
            return;
        }
        match &d.which {
            Which::Multi => self.end_multi(&d),
            Which::Copy(cid) => {
                // the copy's origin, carried as the middle was
                let turn = d.cur.rot * d.from.rot.inverse();
                let o0 = d.from.origin;
                let o1 = turn * (o0 - d.from.at) + d.cur.at;
                self.record(Undo::Send {
                    name: "copy",
                    kind: "copyset".into(),
                    data: json!({ "id": cid, "x": o0.x, "y": o0.y, "z": o0.z, "rot": quat_json(d.from.rot) }),
                });
                self.send_ev(
                    "copyset",
                    None,
                    None,
                    json!({ "id": cid, "x": r4(o1.x), "y": r4(o1.y), "z": r4(o1.z), "rot": quat_json(d.cur.rot) }),
                );
            }
            Which::Block(id) => {
                self.record(Undo::Block {
                    id: id.clone(),
                    t: d.from_b.clone(),
                });
                let b = self.blocks.get(id).cloned().unwrap_or_default();
                self.send_ev("blockset", None, None, block_out(&b));
                self.pick_block(Some(id.clone()));
            }
        }
    }

    // ------------------------------------------------------------------ the pointer

    /// the block (itself, not the gizmo over it) under a point of the picture
    fn block_at(&mut self, x: f32, y: f32) -> Option<String> {
        let d = self.ray_dir(x, y);
        self.blocks.hit(self.at, d).map(|h| h.0)
    }

    /// a press the building takes (the gizmo, a block, a light's marker, a box drawn on
    /// empty room); false leaves it to the rest of the room
    pub(crate) fn edit_down(&mut self, x: f32, y: f32, m: Mods) -> bool {
        let (o, d) = (self.at, self.ray_dir(x, y));
        // a light's marker (while the lights show): picked, or with Shift one of many
        let mut best: Option<(f32, Value)> = None;
        for (id, at, r) in self.light_marks() {
            let t = (at - o).dot(d);
            if t > 0.0 && (o + d * t).distance(at) <= r && best.as_ref().is_none_or(|b| t < b.0) {
                best = Some((t, id));
            }
        }
        if let Some((_, id)) = best {
            if m.shift {
                self.toggle_multi('l', id);
            } else {
                self.clear_multi();
                self.send_ev("lightpick", None, None, id);
            }
            return true;
        }
        let handle = self.gizmo_hit(o, d);
        let block = if handle.is_none() { self.block_at(x, y) } else { None };
        // Shift+click on a block or a copy: one of many, in or out
        if m.shift
            && let Some(sid) = self.block_at(x, y)
        {
            match copy_of(&sid) {
                Some(c) if self.ed.open_copy.as_deref() != Some(c.as_str()) => self.toggle_multi('c', json!(c)),
                _ => self.toggle_multi('b', json!(sid)),
            }
            return true;
        }
        // a thing in a group: the whole group, dragged by any of it
        if let Some(bid) = &block {
            let key = match copy_of(bid) {
                Some(c) if self.ed.open_copy.as_deref() != Some(c.as_str()) => format!("c:{c}"),
                _ => format!("b:{bid}"),
            };
            if let Some(g) = self.outer(&key) {
                if self.ed.sel_group.as_deref() != Some(g.as_str()) {
                    self.pick_group(&g, false);
                }
                self.start_block_drag(x, y, Some((Mode::Plane, 0)));
                return true;
            }
        }
        if block.is_some() && !self.ed.multi.is_empty() {
            self.clear_multi();
        }
        if handle.is_some() || block.is_some() {
            // a click outside the open copy closes it
            if let (Some(oc), Some(b)) = (&self.ed.open_copy, &block)
                && copy_of(b).as_deref() != Some(oc.as_str())
            {
                self.ed.open_copy = None;
            }
            let cid = block.as_deref().and_then(copy_of);
            match (&cid, &block) {
                (Some(c), _) if self.ed.open_copy.as_deref() != Some(c.as_str()) => {
                    if self.ed.sel_copy.as_deref() != Some(c.as_str()) {
                        self.pick_copy(Some(c.clone()), false);
                    }
                }
                (_, Some(b)) if self.ed.sel_block.as_deref() != Some(b.as_str()) => self.pick_block(Some(b.clone())),
                _ => {}
            }
            self.start_block_drag(x, y, handle);
            return true;
        }
        // empty room: a drag draws a box to select what's in it; a click lets go
        self.ed.box_sel = Some(BoxSel {
            x0: x,
            y0: y,
            x,
            y,
            add: m.shift,
            shown: false,
            m,
        });
        true
    }

    pub(crate) fn edit_move(&mut self, x: f32, y: f32, m: Mods) -> bool {
        if self.ed.bdrag.is_some() {
            self.drag_block(x, y, m);
            return true;
        }
        if let Some(b) = &mut self.ed.box_sel {
            (b.x, b.y) = (x, y);
            if !b.shown && (x - b.x0).hypot(y - b.y0) >= 5.0 {
                b.shown = true;
            }
            self.dirty = true;
            return true;
        }
        false
    }

    pub(crate) fn edit_up(&mut self, x: f32, y: f32) -> bool {
        let dbl = self
            .ed
            .last_up
            .is_some_and(|(lx, ly, t)| self.clock - t < 0.45 && (x - lx).hypot(y - ly) < 6.0);
        self.ed.last_up = if dbl { None } else { Some((x, y, self.clock)) };
        if self.ed.bdrag.is_some() {
            self.end_block_drag();
            if dbl {
                self.edit_dblclick(x, y);
            }
            return true;
        }
        if let Some(b) = self.ed.box_sel.take() {
            self.end_box(b, x, y);
            if dbl {
                self.edit_dblclick(x, y);
            }
            self.dirty = true;
            return true;
        }
        false
    }

    /// a double click on a block: its group or its prefab's copy opens
    fn edit_dblclick(&mut self, x: f32, y: f32) {
        let Some(id) = self.block_at(x, y) else { return };
        let c = copy_of(&id);
        let key = match &c {
            Some(c) if self.ed.open_copy.as_deref() != Some(c.as_str()) => format!("c:{c}"),
            _ => format!("b:{id}"),
        };
        if self.outer(&key).is_some() && self.open_group_at(&key) {
            return;
        }
        if c.is_some_and(|c| self.ed.open_copy.as_deref() != Some(c.as_str())) {
            self.open_copy_at(&id);
        }
    }

    /// a right click (that didn't look round) on a block: its menu
    pub(crate) fn block_menu(&mut self, x: f32, y: f32, m: Mods) -> bool {
        let Some(id) = self.block_at(x, y) else { return false };
        self.pick_block(Some(id.clone()));
        self.send_ev("blockmenu", None, Some((x, y, m)), json!(id));
        true
    }

    fn end_box(&mut self, b: BoxSel, x: f32, y: f32) {
        if !b.shown {
            // a click on empty room: everything let go; a double click makes a text
            self.ed.open_copy = None;
            if self.ed.sel_copy.is_some() {
                self.pick_copy(None, false);
            }
            if self.ed.sel_block.is_some() {
                self.pick_block(None);
            }
            self.clear_multi();
            if self.clock - self.last_empty < 0.45 {
                self.last_empty = -10.0;
                self.send_ev("newtext", None, Some((b.x0, b.y0, b.m)), Value::Null);
            } else {
                self.last_empty = self.clock;
            }
            if self.selected.is_some() {
                self.pick(None);
            }
            return;
        }
        let (l, r) = (b.x0.min(x), b.x0.max(x));
        let (t, bt) = (b.y0.min(y), b.y0.max(y));
        let inside = |p: Option<Vec2>| p.is_some_and(|p| p.x >= l && p.x <= r && p.y >= t && p.y <= bt);
        let mut items = Vec::new();
        for k in &self.order {
            let Some(n) = self.nodes.get(k) else { continue };
            if n.head.is_some() {
                continue;
            }
            if let Some((sx, sy, ..)) = n.screen
                && inside(Some(Vec2::new(sx, sy)))
            {
                items.push(Item::new('t', k.clone()));
            }
        }
        let mut seen = std::collections::HashSet::new();
        let ids: Vec<String> = self.blocks.list.iter().map(|(k, _)| k.clone()).collect();
        for id in ids {
            match copy_of(&id) {
                Some(c) if self.ed.open_copy.as_deref() != Some(c.as_str()) => {
                    if !seen.insert(c.clone()) {
                        continue;
                    }
                    let it = Item::new('c', c);
                    let at = self.item_at(&it);
                    if inside(self.to_px(at)) {
                        items.push(it);
                    }
                }
                _ => {
                    let at = self.blocks.model(&id).map(|m| m.w_axis.truncate()).unwrap_or_default();
                    if inside(self.to_px(at)) {
                        items.push(Item::new('b', id));
                    }
                }
            }
        }
        for (id, at, _) in self.light_marks() {
            if inside(self.to_px(at)) {
                items.push(Item { kind: 'l', id });
            }
        }
        if !b.add && self.ed.sel_copy.is_some() {
            self.pick_copy(None, false);
        }
        if !b.add && self.ed.sel_block.is_some() {
            self.pick_block(None);
        }
        self.set_multi(items, b.add);
    }

    /// Esc: the open copy closes, the many are let go, the open group closes
    pub(crate) fn edit_escape(&mut self) -> bool {
        if self.ed.open_copy.is_some() {
            return self.close_copy();
        }
        if self.clear_multi() {
            return true;
        }
        if let Some(g) = self.ed.open_groups.pop() {
            self.pick_group(&g, false);
            return true;
        }
        false
    }

    /// building commands (space.js's protocol); false for one that isn't
    pub(crate) fn edit_command(&mut self, cmd: &str, c: &Value) -> bool {
        let strs = |v: &Value| -> Vec<String> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        };
        match cmd {
            "blocks" => self.set_blocks(c),
            "unblock" => self.unblock(&strs(&c["ids"])),
            "copies" => self.set_copies(c["list"].as_array().map(Vec::as_slice).unwrap_or(&[])),
            "groups" => {
                self.set_groups(c["list"].as_array().map(Vec::as_slice).unwrap_or(&[]));
                if let Some(made) = c.get("made").filter(|m| !m.is_null()) {
                    self.record(Undo::Send {
                        name: "grouped",
                        kind: "groupbreak".into(),
                        data: json!({ "id": made, "quiet": true }),
                    });
                    self.pick_group(&ids(made), false);
                }
                if let Some(broke) = c.get("broke").filter(|m| !m.is_null()) {
                    self.record(Undo::Send {
                        name: "ungrouped",
                        kind: "grouprestore".into(),
                        data: json!({ "members": broke["members"], "name": broke["name"].as_str().unwrap_or("") }),
                    });
                    self.after_multi(false);
                }
                if let Some(s) = c.get("select").filter(|m| !m.is_null()) {
                    self.pick_group(&ids(s), false);
                }
            }
            "group" => {
                let m = self.group_members();
                self.send(json!({ "kind": "groupmake", "data": { "members": m } }));
            }
            "ungroup" => {
                let id = self.sel_group_id();
                self.send(json!({ "kind": "groupbreak", "data": { "id": id } }));
            }
            "removemulti" => {
                let d = self.remove_multi();
                self.send(json!({ "kind": "removedmulti", "data": d }));
            }
            "clearmulti" => {
                self.clear_multi();
            }
            "copygone" => self.copy_gone(&strs(&c["ids"]), c["restore"].clone()),
            "opencopy" => {
                self.open_copy_named(c["id"].as_str().unwrap_or(""));
            }
            "closecopy" => {
                self.close_copy();
            }
            "pickcopy" => self.pick_copy(c["id"].as_str().map(String::from), false),
            "pickblock" => self.pick_block(c["id"].as_str().map(String::from)),
            "blockfields" => self.set_block_fields(c["id"].as_str().unwrap_or(""), &c["t"]),
            "showlights" => self.show_lights(c["on"] == true, c.get("picked").cloned()),
            "lightspot" => {
                let mut d = self.light_spot();
                d["want"] = c["want"].clone();
                self.send(json!({ "kind": "lightspot", "data": d }));
            }
            "blockspot" | "prefabspot" => {
                let mut d = self.block_spot();
                d["want"] = c["want"].clone();
                let kind = if cmd == "blockspot" { "spot" } else { "prefabspot" };
                self.send(json!({ "kind": kind, "data": d }));
            }
            _ => return false,
        }
        self.dirty = true;
        true
    }
}

fn r2(v: f32) -> f64 {
    ((v as f64) * 100.0).round() / 100.0
}

/// the copy a prefab's part belongs to (`prefab:<copy>/<part>`)
pub fn copy_of(id: &str) -> Option<String> {
    (id.starts_with("prefab:") && id.contains('/')).then(|| id[..id.find('/').unwrap_or(id.len())].to_string())
}
