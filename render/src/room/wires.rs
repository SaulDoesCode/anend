//! Wires and zones, as space.js had them. Each sticky ray is drawn as a cable from its
//! block to each target, sagging, lit while a ray passes along it. In wiring mode (J)
//! every block shows a port and a drag from one block to another makes a wire (or takes
//! it away). While the room plays, wired blocks fire on clicks, on their timers and when
//! texts come into or leave a zone. A zone is a see-through block whose texts spread
//! under its own physics, kept inside it.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};
use serde_json::{Map, Value, json};

use super::{Physics, Room, Undo};
use crate::{Mesh, Pass, Vert, prims};

/// a zone in the spread's frame: its middle and turn, half sides, volume and physics
#[derive(Clone, Debug)]
pub(super) struct Frame {
    pub id: String,
    pub c: Vec3,
    pub q: Quat,
    pub qi: Quat,
    pub half: Vec3,
    pub vol: f32,
    pub p: Physics,
}

/// the wiring state of the room
#[derive(Default)]
pub struct Wires {
    /// block id → its wiring (`{id, links, on, every, …}`)
    list: Vec<(String, Value)>,
    pub live: bool,
    timers: HashMap<String, f32>,
    lit: HashMap<String, f32>,
    pub wiring: bool,
    drag: Option<(String, f32, Vec3)>,
    rays: Vec<(Vec<Vec3>, f32)>,
    /// the zones as they were last assigned (a change assigns them again)
    zones_sig: String,
    rev_seen: u64,
}

fn num(v: &Value, k: &str, d: f32) -> f32 {
    v[k].as_f64().map(|x| x as f32).unwrap_or(d)
}

fn v3(v: &Value) -> Vec3 {
    let a: Vec<f32> = v
        .as_array()
        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect())
        .unwrap_or_default();
    if a.len() == 3 {
        Vec3::new(a[0], a[1], a[2])
    } else {
        Vec3::ZERO
    }
}

fn straight(css: &str, a: f32) -> [f32; 4] {
    let c = Room::colour(css, 1.0);
    [c[0], c[1], c[2], a]
}

/// a tube along a curve's points (three.js's TubeGeometry: rings round each point)
fn tube(pts: &[Vec3], r: f32, radial: u32) -> Vec<Vec3> {
    let n = pts.len();
    if n < 2 {
        return Vec::new();
    }
    let mut rings: Vec<Vec<Vec3>> = Vec::with_capacity(n);
    let mut normal = {
        let t = (pts[1] - pts[0]).normalize_or(Vec3::X);
        let pick = if t.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
        t.cross(pick).normalize_or(Vec3::Z)
    };
    for i in 0..n {
        let t = (pts[(i + 1).min(n - 1)] - pts[i.saturating_sub(1)]).normalize_or(Vec3::X);
        // the frame carried along (parallel transport), so the tube doesn't twist
        normal = (normal - t * normal.dot(t)).normalize_or(normal);
        let bi = t.cross(normal);
        rings.push(
            (0..radial)
                .map(|j| {
                    let a = std::f32::consts::TAU * j as f32 / radial as f32;
                    pts[i] + (normal * a.cos() + bi * a.sin()) * r
                })
                .collect(),
        );
    }
    let mut out = Vec::new();
    for i in 0..n - 1 {
        for j in 0..radial as usize {
            let k = (j + 1) % radial as usize;
            let (a, b, c, d) = (rings[i][j], rings[i + 1][j], rings[i + 1][k], rings[i][k]);
            out.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }
    out
}

impl Room {
    // ------------------------------------------------------------------ zones

    /// the zones as frames in the spread's own space
    pub(super) fn zone_frames(&self) -> Vec<Frame> {
        let qi = self.quat.inverse();
        self.blocks
            .list
            .iter()
            .filter(|(_, b)| b.get("zone").is_some_and(|z| z.is_object()))
            .map(|(id, b)| {
                let (at, rot) = crate::blocks::place(b);
                let c = qi * (at - self.origin);
                let q = qi * rot;
                let (w, h, d) = (num(b, "w", 1.0), num(b, "h", 1.0), num(b, "d", 1.0));
                let mut over: Map<String, Value> = self.p.v.clone();
                for (k, v) in b["zone"].as_object().into_iter().flatten() {
                    if !v.is_null() {
                        over.insert(k.clone(), v.clone());
                    }
                }
                Frame {
                    id: id.clone(),
                    c,
                    q,
                    qi: q.inverse(),
                    half: Vec3::new(w, h, d) / 2.0,
                    vol: w * h * d,
                    p: Physics { v: over },
                }
            })
            .collect()
    }

    /// the smallest zone a point (in the spread's space) is in
    pub(super) fn zone_at(p: Vec3, frames: &[Frame]) -> Option<String> {
        let mut best: Option<&Frame> = None;
        for f in frames {
            let v = f.qi * (p - f.c);
            if v.x.abs() <= f.half.x
                && v.y.abs() <= f.half.y
                && v.z.abs() <= f.half.z
                && best.is_none_or(|b| f.vol < b.vol)
            {
                best = Some(f);
            }
        }
        best.map(|f| f.id.clone())
    }

    /// a text kept inside its zone's box (its label's half size in from the sides, at
    /// most half the box); whether it had to be moved
    pub(super) fn keep_in(&mut self, key: &str, f: &Frame) -> bool {
        let Some(n) = self.nodes.get_mut(key) else { return false };
        let v = f.qi * (Vec3::new(n.x, n.y, n.z) - f.c);
        let m = Vec3::new(
            (if n.hw > 0.0 { n.hw } else { 0.3 }).min(f.half.x * 0.45),
            (if n.hh > 0.0 { n.hh } else { 0.2 }).min(f.half.y * 0.45),
            0.3f32.min(f.half.z * 0.45),
        );
        let c = v.clamp(-f.half + m, f.half - m);
        if c == v {
            return false;
        }
        let w = f.q * c + f.c;
        (n.x, n.y, n.z) = (w.x, w.y, w.z);
        true
    }

    /// which zone each text is in (members of a zone that moved come along first); a
    /// change of zones assigns them again
    pub(super) fn assign_zones(&mut self, frames: &[Frame]) {
        let sig = Self::zones_sig(frames);
        if sig == self.wires.zones_sig {
            return;
        }
        self.wires.zones_sig = sig;
        let by: HashMap<&str, &Frame> = frames.iter().map(|f| (f.id.as_str(), f)).collect();
        let mut moved = false;
        for k in self.order.clone() {
            let Some(n) = self.nodes.get(&k) else { continue };
            let (zone, pinned) = (n.zone.clone(), n.pinned);
            if let Some(f) = zone.as_deref().and_then(|z| by.get(z))
                && !pinned
            {
                let f = (*f).clone();
                self.keep_in(&k, &f);
            }
            let Some(n) = self.nodes.get(&k) else { continue };
            let now = if frames.is_empty() {
                None
            } else {
                Self::zone_at(Vec3::new(n.x, n.y, n.z), frames)
            };
            if now != zone {
                moved = true;
                if let Some(n) = self.nodes.get_mut(&k) {
                    n.zone = now.clone();
                }
                if self.wires.live {
                    if let Some(w) = &zone {
                        self.wire_zone(w, &k, "leave");
                    }
                    if let Some(z) = &now {
                        self.wire_zone(z, &k, "enter");
                    }
                }
            }
        }
        if moved {
            self.sim = None;
        }
    }

    /// a zone moved, came or went: the spread wakes to look at its texts again
    pub(super) fn zone_check(&mut self) {
        if self.blocks.rev == self.wires.rev_seen {
            return;
        }
        self.wires.rev_seen = self.blocks.rev;
        let frames = self.zone_frames();
        if Self::zones_sig(&frames) != self.wires.zones_sig {
            self.wake(1.0);
        }
    }

    fn zones_sig(frames: &[Frame]) -> String {
        format!(
            "{:?}",
            frames
                .iter()
                .map(|f| (&f.id, f.c.to_array(), f.q.to_array(), f.half.to_array(), f.p.v.len()))
                .collect::<Vec<_>>()
        )
    }

    /// a text dropped: into a zone, or out of one
    pub(super) fn drop_into_zone(&mut self, key: &str) {
        let frames = self.zone_frames();
        let Some(n) = self.nodes.get(key) else { return };
        let was = n.zone.clone();
        let now = Self::zone_at(Vec3::new(n.x, n.y, n.z), &frames);
        if let Some(n) = self.nodes.get_mut(key) {
            n.zone = now.clone();
        }
        if was != now && self.wires.live {
            if let Some(w) = &was {
                self.wire_zone(w, key, "leave");
            }
            if let Some(z) = &now {
                self.wire_zone(z, key, "enter");
            }
        }
    }

    // ------------------------------------------------------------------ wires

    pub(super) fn set_wires(&mut self, list: &[Value], live: bool) {
        self.wires.list = list
            .iter()
            .filter_map(|w| Some((w["id"].as_str()?.to_string(), w.clone())))
            .collect();
        // timers keep their beat across a reload of the list (a cause reloads it)
        let was = self.wires.live;
        self.wires.live = live;
        if !(was && live) {
            self.wires.timers.clear();
        }
        let ids: Vec<String> = self.wires.list.iter().map(|(k, _)| k.clone()).collect();
        self.wires.timers.retain(|k, _| ids.contains(k));
        if live && !was {
            self.start_loops();
        }
        if !live && was {
            self.hush_all();
        }
        self.dirty = true;
    }

    fn wiring_of(&self, id: &str) -> Option<&Value> {
        self.wires.list.iter().find(|(k, _)| k == id).map(|(_, w)| w)
    }

    fn listens(w: &Value, event: &str) -> bool {
        w["on"].as_array().is_some_and(|a| a.iter().any(|e| e == event))
    }

    pub(super) fn set_wiring(&mut self, on: Option<bool>) {
        self.wires.wiring = on.unwrap_or(!self.wires.wiring);
        let w = self.wires.wiring;
        self.send_ev("wiremode", None, None, json!(w));
        self.dirty = true;
    }

    pub(super) fn wire_fire(&mut self, id: &str, event: &str, from: &str) {
        self.send_ev(
            "blockfire",
            None,
            None,
            json!({ "id": id, "event": event, "from": from }),
        );
    }

    /// the timers of wired blocks, on the room's clock (playing only)
    pub(super) fn wire_timers(&mut self) {
        if !self.wires.live {
            return;
        }
        let now = self.clock;
        let mut fire = Vec::new();
        for (id, w) in &self.wires.list {
            if !Self::listens(w, "timer") {
                continue;
            }
            let every = num(w, "every", 1.0).max(0.2);
            match self.wires.timers.get(id) {
                None => {
                    self.wires.timers.insert(id.clone(), now);
                }
                Some(t) if now - t >= every => {
                    self.wires.timers.insert(id.clone(), now);
                    fire.push(id.clone());
                }
                _ => {}
            }
        }
        for id in fire {
            self.wire_fire(&id, "timer", "");
        }
    }

    /// a text came into or left a zone: the zone's block is told, when it listens
    fn wire_zone(&mut self, zone: &str, text: &str, event: &str) {
        if self.wires.live && self.wiring_of(zone).is_some_and(|w| Self::listens(w, event)) {
            self.wire_fire(zone, event, text);
        }
    }

    /// the rays a cause cast: drawn for a moment, and the wires they went along lit
    pub(super) fn show_rays(&mut self, rays: &[Value]) {
        if rays.is_empty() {
            return;
        }
        let until = self.clock + 0.7;
        for r in rays {
            let from = v3(&r["from"]);
            for (id, w) in &self.wires.list {
                let Some(m) = self.blocks.model(id) else { continue };
                if m.w_axis.truncate().distance(from) > 0.6 {
                    continue;
                }
                let links = w["links"].as_array().cloned().unwrap_or_default();
                for t in r["hits"].as_array().into_iter().flatten() {
                    if links.contains(t) {
                        self.wires
                            .lit
                            .insert(format!("{id}>{}", t.as_str().unwrap_or("")), until);
                    }
                }
            }
        }
        let pts = rays.iter().flat_map(|r| [v3(&r["from"]), v3(&r["to"])]).collect();
        self.wires.rays.push((pts, self.clock));
        self.dirty = true;
    }

    /// whether rays are still fading or wires lit (the room is drawn again while they are)
    pub(super) fn wires_moving(&mut self) -> bool {
        let now = self.clock;
        self.wires.rays.retain(|(_, t0)| now - t0 < 0.8);
        let lit = self.wires.lit.len();
        self.wires.lit.retain(|_, until| *until > now);
        !self.wires.rays.is_empty() || lit != self.wires.lit.len() || !self.wires.lit.is_empty()
    }

    /// a press while wiring (a drag from a block makes a wire) or playing (a click fires
    /// a block listening for clicks); false leaves it to the rest
    pub(super) fn wire_down(&mut self, x: f32, y: f32) -> bool {
        if self.wires.wiring && !self.wires.live {
            let d = self.ray_dir(x, y);
            let Some((from, _)) = self.blocks.hit(self.at, d) else {
                return false;
            };
            let at = self
                .blocks
                .model(&from)
                .map(|m| m.w_axis.truncate())
                .unwrap_or_default();
            let depth = (at - self.at).dot(self.camera_rot() * Vec3::NEG_Z);
            self.wires.drag = Some((from, depth, at));
            return true;
        }
        if self.wires.live && !self.wires.list.is_empty() {
            let d = self.ray_dir(x, y);
            if let Some((id, _)) = self.blocks.hit(self.at, d)
                && self.wiring_of(&id).is_some_and(|w| Self::listens(w, "click"))
            {
                self.wire_fire(&id, "click", "");
                return true;
            }
        }
        false
    }

    pub(super) fn wire_move(&mut self, x: f32, y: f32) -> bool {
        let Some((from, depth, _)) = self.wires.drag.clone() else {
            return false;
        };
        let d = self.ray_dir(x, y);
        let to = match self.blocks.hit(self.at, d) {
            Some((id, _)) if id != from => self.blocks.model(&id).map(|m| m.w_axis.truncate()),
            _ => None,
        };
        let p = to.unwrap_or_else(|| self.ray_at(x, y, depth));
        self.wires.drag = Some((from, depth, p));
        self.dirty = true;
        true
    }

    pub(super) fn wire_up(&mut self, x: f32, y: f32) -> bool {
        let Some((from, ..)) = self.wires.drag.take() else {
            return false;
        };
        let d = self.ray_dir(x, y);
        if let Some((to, _)) = self.blocks.hit(self.at, d)
            && to != from
        {
            self.send_ev("wirelink", None, None, json!({ "from": from, "to": to }));
        }
        self.dirty = true;
        true
    }

    /// the wires' commands (space.js's protocol); false for one that isn't
    pub(super) fn wire_command(&mut self, cmd: &str, c: &Value) -> bool {
        match cmd {
            "wires" => self.set_wires(
                c["wires"].as_array().map(Vec::as_slice).unwrap_or(&[]),
                c["live"] == true,
            ),
            "wiremode" => self.set_wiring(if c["toggle"] == true {
                None
            } else {
                Some(c["on"] == true)
            }),
            "wired" => {
                let items: Vec<Undo> = c["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|x| Undo::Send {
                        name: "wired",
                        kind: "wireset".into(),
                        data: json!({ "id": x["id"], "wiring": x["before"] }),
                    })
                    .collect();
                if !items.is_empty() {
                    self.record(if items.len() == 1 {
                        items.into_iter().next().unwrap_or(Undo::Many(vec![]))
                    } else {
                        Undo::Many(items)
                    });
                }
            }
            "rays" => self.show_rays(c["rays"].as_array().map(Vec::as_slice).unwrap_or(&[])),
            _ => return false,
        }
        true
    }

    /// the wires, the ports while wiring, the wire being dragged and the rays, as meshes
    pub(super) fn wire_meshes(&mut self, out: &mut Vec<Mesh>) {
        let base = u64::MAX - 5000;
        let now = self.clock;
        let mut dim = Vec::new();
        let mut lit = Vec::new();
        for (id, w) in self.wires.list.clone() {
            let Some(a) = self.blocks.model(&id) else { continue };
            for t in w["links"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                let Some(b) = self.blocks.model(t) else { continue };
                let (p0, p1) = (a.w_axis.truncate(), b.w_axis.truncate());
                let mut mid = (p0 + p1) / 2.0;
                mid.y -= (p0.distance(p1) * 0.18).min(1.2);
                let pts: Vec<Vec3> = (0..=28)
                    .map(|i| {
                        let s = i as f32 / 28.0;
                        p0 * (1.0 - s) * (1.0 - s) + mid * 2.0 * (1.0 - s) * s + p1 * s * s
                    })
                    .collect();
                let on = self.wires.lit.get(&format!("{id}>{t}")).is_some_and(|u| *u > now);
                if on {
                    lit.extend(tube(&pts, 0.045, 6));
                } else {
                    dim.extend(tube(&pts, 0.028, 6));
                }
            }
        }
        let mesh = |id: u64, tris: Vec<Vec3>, c: [f32; 4], pass: Pass| Mesh {
            id,
            version: tris.len() as u64
                ^ tris
                    .first()
                    .map(|v| (v.x.to_bits() as u64) << 11 ^ v.y.to_bits() as u64)
                    .unwrap_or(0),
            verts: std::sync::Arc::new(prims::verts(&tris, Mat4::IDENTITY, c)),
            texture: None,
            pass,
            model: Mat4::IDENTITY,
            cull: true,
            lines: false,
        };
        if !dim.is_empty() {
            out.push(mesh(base, dim, straight("#5a4a32", 0.85), Pass::Glass));
        }
        if !lit.is_empty() {
            out.push(mesh(base + 1, lit, straight("#ffd23f", 1.0), Pass::Over));
        }
        // the ports, while wiring
        if self.wires.wiring {
            let ball = prims::ball(0.09, 12, 8);
            let mut tris = Vec::new();
            for id in self.blocks.list.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>() {
                let Some((lo, hi)) = self.blocks.bounds(&id) else {
                    continue;
                };
                let at = Vec3::new((lo.x + hi.x) / 2.0, hi.y + 0.12, (lo.z + hi.z) / 2.0);
                tris.extend(ball.iter().map(|p| *p + at));
            }
            if !tris.is_empty() {
                out.push(mesh(base + 2, tris, straight("#3f8cff", 0.9), Pass::Over));
            }
        }
        // the wire being dragged
        if let Some((from, _, to)) = &self.wires.drag
            && let Some(m) = self.blocks.model(from)
        {
            let segs = prims::dashed(&[m.w_axis.truncate(), *to], 0.15, 0.08);
            out.push(line_mesh(base + 3, segs, straight("#3f8cff", 1.0)));
        }
        // the rays, fading over 0.8 s
        for (i, (pts, t0)) in self.wires.rays.iter().enumerate() {
            let a = 0.95 * (1.0 - (now - t0) / 0.8);
            if a > 0.0 {
                let mut m = line_mesh(base + 10 + i as u64, pts.clone(), straight("#ffcc33", a));
                m.version = (a * 1000.0) as u64;
                out.push(m);
            }
        }
    }
}

fn line_mesh(id: u64, segs: Vec<Vec3>, c: [f32; 4]) -> Mesh {
    Mesh {
        id,
        version: segs.len() as u64
            ^ segs
                .last()
                .map(|v| (v.x.to_bits() as u64) << 5 ^ v.z.to_bits() as u64)
                .unwrap_or(0),
        verts: std::sync::Arc::new(
            segs.into_iter()
                .map(|p| Vert {
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
    }
}
