//! Moving through the room and what plays in it, as space.js did it: animations (core's
//! graph::anims) posing their blocks every frame; walking (V: gravity, blocks to stand on
//! and bump into, run, sneak, crouch, jump, lean, mantle), the grapple (G: hook what's
//! ahead, swing, reel with the wheel) and superflight (B: fly fast where you look,
//! collecting the texts flown through, in order, drawn as a trail); and placing a prefab
//! by hand (its see-through ghost where you look, snapped, turned with the wheel, red
//! where it would overlap). The body moves in fixed steps of 1/120 s.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{EulerRot, Mat4, Quat, Vec3};
use serde_json::{Value, json};

use super::Room;
use crate::{Mesh, Pass, Vert};

const FP_STEP: f32 = 1.0 / 120.0;

/// the walking body's sizes, speeds (units a second) and the floor
struct BodyK;
impl BodyK {
    const R: f32 = 0.3;
    const STAND: f32 = 1.7;
    const CROUCH: f32 = 1.05;
    const WALK: f32 = 3.2;
    const RUN: f32 = 6.0;
    const SNEAK: f32 = 1.4;
    const CRAWL: f32 = 1.6;
    const JUMP: f32 = 6.2;
    const GRAVITY: f32 = 20.0;
    const STEP: f32 = 0.35;
    const MANTLE: f32 = 1.3;
    const FLOOR: f32 = -9.0;
}

#[derive(Clone, Debug)]
struct Body {
    p: Vec3,
    v: Vec3,
    ground: bool,
    h: f32,
    lean: f32,
    acc: f32,
    ledge: Option<f32>,
    jump_held: bool,
}

#[derive(Clone, Debug)]
struct Flight {
    v: Vec3,
    acc: f32,
}

#[derive(Clone, Debug)]
struct Ghost {
    prefab: Value,
    parts: Vec<Value>,
    turn: u32,
    ok: bool,
    at: Option<Vec3>,
}

/// what moves and plays
pub struct Play {
    anims: HashMap<String, Value>,
    anim_t0: f32,
    anim_playing: bool,
    /// the animations window's: block → (animation, phase or none for playing, start)
    previews: HashMap<String, (Value, Option<f32>, f32)>,
    posed: Vec<String>,
    pub mode: String,
    body: Option<Body>,
    flight: Option<Flight>,
    rope: Option<(Vec3, f32)>,
    collected: Vec<(String, String, Vec3)>,
    ghost: Option<Ghost>,
    solids: (u64, Vec<(Vec3, Vec3)>),
    /// prefabs' small pictures to draw: id, what, from where
    pub thumbs: Vec<(Value, crate::Scene, crate::Camera)>,
}

impl Default for Play {
    fn default() -> Self {
        Play {
            anims: HashMap::new(),
            anim_t0: 0.0,
            anim_playing: true,
            previews: HashMap::new(),
            posed: Vec::new(),
            mode: "fly".into(),
            body: None,
            flight: None,
            rope: None,
            collected: Vec::new(),
            ghost: None,
            solids: (u64::MAX, Vec::new()),
            thumbs: Vec::new(),
        }
    }
}

fn arr<const N: usize>(v: &Value, d: [f32; N]) -> [f32; N] {
    let a: Vec<f32> = v
        .as_array()
        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect())
        .unwrap_or_default();
    if a.len() == N {
        let mut o = [0.0; N];
        o.copy_from_slice(&a);
        o
    } else {
        d
    }
}

/// a CSS cubic-bezier timing at x (0 to 1): core's graph::anims::bezier, by bisection
fn cubic_bezier(t: [f32; 4], x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let at = |a: f32, b: f32, s: f32| 3.0 * (1.0 - s) * (1.0 - s) * s * a + 3.0 * (1.0 - s) * s * s * b + s * s * s;
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if at(t[0], t[2], mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(t[1], t[3], (lo + hi) / 2.0)
}

/// where in an animation (0 to 1) `t` seconds after it started
pub fn anim_phase(a: &Value, t: f32) -> f32 {
    let x = t.max(0.0) / a["duration"].as_f64().map(|d| d as f32).unwrap_or(2.0).max(0.05);
    match a["repeat"].as_str() {
        Some("loop") => x % 1.0,
        Some("alternate") => {
            let f = x % 2.0;
            if f <= 1.0 { f } else { 2.0 - f }
        }
        _ => x.min(1.0),
    }
}

/// an animation's pose at a phase: its move, turn (degrees), scale and texture
pub fn anim_pose(a: &Value, p: f32) -> ([f32; 3], [f32; 3], [f32; 3], Option<String>) {
    let mut keys: Vec<&Value> = a["keys"].as_array().map(|k| k.iter().collect()).unwrap_or_default();
    let at = |k: &Value| k["at"].as_f64().unwrap_or(0.0) as f32;
    keys.sort_by(|x, y| at(x).total_cmp(&at(y)));
    let timing = arr(&a["timing"], [0.25, 0.1, 0.25, 1.0]);
    let field = |name: &str, rest: [f32; 3]| -> [f32; 3] {
        let set: Vec<&&Value> = keys.iter().filter(|k| k[name].is_array()).collect();
        let Some(f) = set.first() else { return rest };
        let fv = arr(&f[name], rest);
        if p <= at(f) {
            if at(f) <= 0.0 {
                return fv;
            }
            let u = cubic_bezier(timing, p / at(f));
            return [0, 1, 2].map(|i| rest[i] + (fv[i] - rest[i]) * u);
        }
        for w in set.windows(2) {
            let (k0, k1) = (w[0], w[1]);
            if p <= at(k1) {
                let t0 = if k0["timing"].is_array() {
                    arr(&k0["timing"], timing)
                } else {
                    timing
                };
                let u = cubic_bezier(t0, (p - at(k0)) / (at(k1) - at(k0)).max(1e-6));
                let (a0, a1) = (arr(&k0[name], rest), arr(&k1[name], rest));
                return [0, 1, 2].map(|i| a0[i] + (a1[i] - a0[i]) * u);
            }
        }
        arr(&set[set.len() - 1][name], rest)
    };
    let mut texture = None;
    for k in &keys {
        if at(k) <= p
            && let Some(t) = k["texture"].as_str()
        {
            texture = Some(t.to_string());
        }
    }
    (
        field("move", [0.0; 3]),
        field("turn", [0.0; 3]),
        field("scale", [1.0; 3]),
        texture,
    )
}

fn straight(css: &str, a: f32) -> [f32; 4] {
    let c = Room::colour(css, 1.0);
    [c[0], c[1], c[2], a]
}

fn boxes_meet(a: (Vec3, Vec3), b: (Vec3, Vec3)) -> bool {
    a.0.x <= b.1.x && a.1.x >= b.0.x && a.0.y <= b.1.y && a.1.y >= b.0.y && a.0.z <= b.1.z && a.1.z >= b.0.z
}

impl Room {
    // ------------------------------------------------------------------ animations

    pub(super) fn set_anims(&mut self, list: &[Value], textures: Option<&Value>) {
        if let Some(t) = textures.and_then(Value::as_object) {
            for (k, v) in t {
                self.blocks.abouts.insert(k.clone(), v.clone());
            }
        }
        self.play.anims = list
            .iter()
            .filter_map(|x| Some((x["key"].as_str()?.to_string(), x["anim"].clone())))
            .collect();
        self.play.anim_t0 = self.clock;
        self.unpose_all();
        self.dirty = true;
    }

    /// play or pause every animation (paused: each block back where it was put)
    pub(super) fn play_anims(&mut self, on: bool) {
        self.play.anim_playing = on;
        if on {
            self.play.anim_t0 = self.clock;
        } else {
            self.unpose_all();
        }
        self.dirty = true;
    }

    pub(super) fn preview_anim(&mut self, id: &str, anim: Option<Value>, p: Option<f32>) {
        match anim.filter(|a| !a.is_null()) {
            Some(a) => {
                self.play.previews.insert(id.to_string(), (a, p, self.clock));
            }
            None => {
                self.play.previews.remove(id);
                self.unpose(id);
            }
        }
        self.dirty = true;
    }

    fn unpose(&mut self, id: &str) {
        self.blocks.poses.remove(id);
        if self.blocks.tex_over.remove(id).is_some() {
            self.blocks.rev += 1;
        }
    }

    fn unpose_all(&mut self) {
        for id in std::mem::take(&mut self.play.posed) {
            self.unpose(&id);
        }
    }

    /// every playing block posed for this moment; whether any is
    pub(super) fn apply_anims(&mut self) -> bool {
        let now = self.clock;
        let mut posed = Vec::new();
        for (id, b) in self.blocks.list.clone() {
            let (a, p) = if let Some((a, p, t0)) = self.play.previews.get(&id) {
                (a.clone(), p.unwrap_or_else(|| anim_phase(a, now - t0)))
            } else if self.play.anim_playing
                && let Some(key) = b["anim"]["key"].as_str()
                && let Some(a) = self.play.anims.get(key)
            {
                let t = (now - self.play.anim_t0 - b["anim"]["delay"].as_f64().unwrap_or(0.0) as f32)
                    * b["anim"]["speed"].as_f64().unwrap_or(1.0) as f32;
                (a.clone(), anim_phase(a, t))
            } else {
                continue;
            };
            let (mv, turn, scale, texture) = anim_pose(&a, p);
            let (at, rot) = crate::blocks::place(&b);
            let q = rot
                * Quat::from_euler(
                    EulerRot::XYZ,
                    turn[0].to_radians(),
                    turn[1].to_radians(),
                    turn[2].to_radians(),
                );
            self.blocks.poses.insert(
                id.clone(),
                Mat4::from_scale_rotation_translation(Vec3::from(scale), q, at + Vec3::from(mv)),
            );
            if self.blocks.tex_over.get(&id) != texture.as_ref() {
                match texture {
                    Some(t) => self.blocks.tex_over.insert(id.clone(), t),
                    None => self.blocks.tex_over.remove(&id),
                };
            }
            posed.push(id);
        }
        // blocks no longer playing go back where they were put
        for id in std::mem::take(&mut self.play.posed) {
            if !posed.contains(&id) {
                self.unpose(&id);
            }
        }
        let any = !posed.is_empty();
        self.play.posed = posed;
        if any {
            self.blocks.rev += 1;
            self.dirty = true;
        }
        any
    }

    // ------------------------------------------------------------------ first person

    pub(super) fn set_mode(&mut self, mode: &str) {
        if self.play.mode == mode {
            return;
        }
        self.play.mode = mode.to_string();
        self.ease = None;
        match mode {
            "fly" => {
                self.drop_rope();
                self.play.body = None;
                self.play.flight = None;
            }
            "walk" => {
                self.play.flight = None;
                let c = self.at;
                self.play.body = Some(Body {
                    p: Vec3::new(c.x, c.y - (BodyK::STAND - 0.1), c.z),
                    v: Vec3::ZERO,
                    ground: false,
                    h: BodyK::STAND,
                    lean: 0.0,
                    acc: 0.0,
                    ledge: None,
                    jump_held: false,
                });
                self.roll_a = 0.0;
            }
            _ => {
                self.drop_rope();
                self.play.body = None;
                self.play.flight = Some(Flight {
                    v: Vec3::ZERO,
                    acc: 0.0,
                });
            }
        }
        self.send_ev("mode", None, None, json!(mode));
        self.dirty = true;
    }

    fn solids(&mut self) -> Vec<(Vec3, Vec3)> {
        if self.play.solids.0 != self.blocks.rev {
            self.play.solids = (self.blocks.rev, self.blocks.solids());
        }
        self.play.solids.1.clone()
    }

    fn body_box(p: Vec3, h: f32) -> (Vec3, Vec3) {
        (
            Vec3::new(p.x - BodyK::R, p.y, p.z - BodyK::R),
            Vec3::new(p.x + BodyK::R, p.y + h, p.z + BodyK::R),
        )
    }

    /// whether a body at feet `p`, `h` tall, would stand in a block
    fn blocked(solids: &[(Vec3, Vec3)], p: Vec3, h: f32) -> bool {
        let (lo, hi) = Self::body_box(p, h);
        let b = (lo + 0.001, hi - 0.001);
        solids.iter().any(|s| boxes_meet(b, *s))
    }

    /// the body (walking) or the flight moves on by `dt`; the camera follows
    pub(super) fn move_body(&mut self, dt: f32) {
        if self.play.mode == "super" {
            return self.superflight(dt);
        }
        let Some(mut b) = self.play.body.take() else { return };
        b.acc = (b.acc + dt).min(0.25);
        let solids = self.solids();
        while b.acc >= FP_STEP {
            self.walk_step(&mut b, &solids, FP_STEP);
            b.acc -= FP_STEP;
        }
        // the camera at the eyes, leaning
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        self.at = Vec3::new(b.p.x, b.p.y + b.h - 0.1, b.p.z) + right * b.lean * 0.4;
        self.roll_a = -b.lean * 0.12;
        self.play.body = Some(b);
        self.dirty = true;
    }

    fn walk_step(&mut self, b: &mut Body, solids: &[(Vec3, Vec3)], dt: f32) {
        let k = |r: &Room, a: &str| r.holding(a);
        // crouching (and staying down where there's no room to stand)
        let want = if k(self, "fly_down") {
            BodyK::CROUCH
        } else {
            BodyK::STAND
        };
        if !(want > b.h && Self::blocked(solids, b.p, want)) {
            b.h = want;
        }
        let lean = (if k(self, "roll_left") { -1.0 } else { 0.0 }) + (if k(self, "roll_right") { 1.0 } else { 0.0 });
        b.lean += (lean - b.lean) * (dt * 10.0).min(1.0);
        // where the keys push, on the ground's plane
        let fwd = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let mut wish = Vec3::ZERO;
        if k(self, "fly_forward") {
            wish += fwd;
        }
        if k(self, "fly_back") {
            wish -= fwd;
        }
        if k(self, "fly_left") {
            wish -= right;
        }
        if k(self, "fly_right") {
            wish += right;
        }
        let wish = wish.normalize_or_zero();
        let shift = self.held.contains("ShiftLeft") || self.held.contains("ShiftRight");
        let speed = if b.h < BodyK::STAND {
            BodyK::CRAWL
        } else if k(self, "sneak") {
            BodyK::SNEAK
        } else if shift {
            BodyK::RUN
        } else {
            BodyK::WALK
        };
        if self.play.rope.is_some() {
            // on the rope: a little push, gravity, the rope's pull
            b.v += wish * 6.0 * dt;
            b.v.y -= BodyK::GRAVITY * dt;
        } else if b.ground {
            b.v.x = wish.x * speed;
            b.v.z = wish.z * speed;
        } else {
            b.v.x += (wish.x * speed - b.v.x) * (dt * 2.0).min(1.0);
            b.v.z += (wish.z * speed - b.v.z) * (dt * 2.0).min(1.0);
            b.v.y -= BodyK::GRAVITY * dt;
        }
        if b.ground && self.play.rope.is_none() {
            b.v.y = 0.0;
        }
        // a jump; at a ledge in reach, a mantle onto it
        let up = k(self, "fly_up");
        if up && !b.jump_held {
            if self.play.rope.is_some() {
                self.drop_rope();
            } else if let Some(l) = b.ledge.filter(|l| l - b.p.y <= BodyK::MANTLE)
                && !Self::blocked(
                    solids,
                    Vec3::new(b.p.x + fwd.x * 0.35, l + 0.01, b.p.z + fwd.z * 0.35),
                    b.h,
                )
            {
                b.p = Vec3::new(b.p.x + fwd.x * 0.35, l + 0.01, b.p.z + fwd.z * 0.35);
                b.v = Vec3::ZERO;
            } else if b.ground {
                b.v.y = BodyK::JUMP;
            }
        }
        b.jump_held = up;
        // move one axis at a time, out of what's in the way
        b.ledge = None;
        b.ground = false;
        for axis in [0usize, 2, 1] {
            let d = b.v[axis] * dt;
            if d == 0.0 {
                continue;
            }
            b.p[axis] += d;
            for s in solids {
                if !boxes_meet(Self::body_box(b.p, b.h), *s) {
                    continue;
                }
                if axis == 1 {
                    if d < 0.0 {
                        b.p.y = s.1.y;
                        b.ground = true;
                    } else {
                        b.p.y = s.0.y - b.h;
                    }
                    b.v.y = 0.0;
                } else {
                    // a low step is climbed; a higher one is a ledge to mantle
                    let rise = s.1.y - b.p.y;
                    if rise > 0.0
                        && rise <= BodyK::STEP
                        && b.v.y <= 0.0
                        && !Self::blocked(solids, Vec3::new(b.p.x, s.1.y + 0.001, b.p.z), b.h)
                    {
                        b.p.y = s.1.y + 0.001;
                        continue;
                    }
                    if rise > 0.0 {
                        b.ledge = Some(b.ledge.map_or(s.1.y, |l| l.max(s.1.y)));
                    }
                    b.p[axis] = if d > 0.0 {
                        s.0[axis] - BodyK::R - 0.0005
                    } else {
                        s.1[axis] + BodyK::R + 0.0005
                    };
                    if self.play.rope.is_none() {
                        b.v[axis] = 0.0;
                    }
                }
            }
        }
        if b.p.y <= BodyK::FLOOR {
            b.p.y = BodyK::FLOOR;
            b.v.y = b.v.y.max(0.0);
            b.ground = true;
        }
        // standing on something (resting on its top)
        if !b.ground && b.v.y <= 0.0 {
            let below = Self::body_box(Vec3::new(b.p.x, b.p.y - 0.01, b.p.z), 0.009);
            if solids.iter().any(|s| boxes_meet(below, *s)) {
                b.ground = true;
            }
        }
        // the rope keeps the hand within its length of the hook
        if let Some((at, len)) = self.play.rope {
            let hand = Vec3::new(b.p.x, b.p.y + b.h - 0.3, b.p.z);
            let d = hand - at;
            let l = d.length();
            if l > len {
                let d = d / l;
                b.p += d * (len - l);
                let out = b.v.dot(d);
                if out > 0.0 {
                    b.v -= d * out;
                }
                b.ground = false;
            }
            b.v *= 1.0 - (dt * 0.15).min(1.0);
        }
    }

    /// the grapple: hook what's ahead (a block within 60, else a text near where you
    /// look), or let go
    pub(super) fn grapple(&mut self) {
        if self.play.rope.is_some() {
            return self.drop_rope();
        }
        if self.play.mode != "walk" {
            self.set_mode("walk");
        }
        let fwd = self.camera_rot() * Vec3::NEG_Z;
        let mut at = self.blocks.hit_n(self.at, fwd, 60.0).map(|h| h.1);
        if at.is_none() {
            let mut best = 0.05;
            for k in &self.order {
                let w = self.world(&self.nodes[k]);
                let d = w - self.at;
                let dist = d.length();
                if !(1.0..=60.0).contains(&dist) {
                    continue;
                }
                let off = 1.0 - (d / dist).dot(fwd);
                if off < best {
                    best = off;
                    at = Some(w);
                }
            }
        }
        let Some(at) = at else {
            self.toast("nothing to hook ahead (60 units at most)");
            return;
        };
        let Some(b) = &self.play.body else { return };
        let hand = Vec3::new(b.p.x, b.p.y + b.h - 0.3, b.p.z);
        self.play.rope = Some((at, hand.distance(at).max(1.2)));
        self.dirty = true;
    }

    fn drop_rope(&mut self) {
        if self.play.rope.take().is_some() {
            self.dirty = true;
        }
    }

    /// superflight: thrust where you look, Shift to boost; texts flown through collected
    fn superflight(&mut self, dt: f32) {
        let Some(mut f) = self.play.flight.take() else { return };
        f.acc = (f.acc + dt).min(0.25);
        while f.acc >= FP_STEP {
            let t = FP_STEP;
            let q = self.camera_rot();
            let (look, right) = (q * Vec3::NEG_Z, q * Vec3::X);
            let boost = if self.held.contains("ShiftLeft") || self.held.contains("ShiftRight") {
                3.0
            } else {
                1.0
            };
            let mut thrust = Vec3::ZERO;
            if self.holding("fly_forward") {
                thrust += look * 40.0 * boost;
            }
            if self.holding("fly_back") {
                thrust += f.v * -2.0;
            }
            if self.holding("fly_left") {
                thrust -= right * 20.0;
            }
            if self.holding("fly_right") {
                thrust += right * 20.0;
            }
            if self.holding("fly_up") {
                thrust.y += 20.0;
            }
            if self.holding("fly_down") {
                thrust.y -= 20.0;
            }
            f.v += thrust * t;
            f.v *= 1.0 - (1.2 * t).min(1.0);
            let max = 30.0 * boost;
            if f.v.length() > max {
                f.v = f.v.normalize() * max;
            }
            let from = self.at;
            self.at += f.v * t;
            self.collect(from, self.at);
            f.acc -= t;
        }
        self.play.flight = Some(f);
        self.dirty = true;
    }

    /// the texts within reach of the path from `a` to `b`, in the order met
    fn collect(&mut self, a: Vec3, b: Vec3) {
        let seg = b - a;
        let len2 = seg.length_squared();
        if len2 < 1e-9 {
            return;
        }
        let size0 = self.p.f("size");
        let mut met: Vec<(f32, String, String, Vec3)> = Vec::new();
        for k in &self.order {
            if self.play.collected.iter().any(|c| c.0 == *k) {
                continue;
            }
            let n = &self.nodes[k];
            let w = self.world(n);
            let t = ((w - a).dot(seg) / len2).clamp(0.0, 1.0);
            if (a + seg * t).distance(w) <= 0.6 + n.size.unwrap_or(size0) {
                met.push((
                    t,
                    k.clone(),
                    n.f.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                    w,
                ));
            }
        }
        if met.is_empty() {
            return;
        }
        met.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        for (_, k, t, w) in met {
            self.play.collected.push((k, t, w));
        }
        self.send_collected();
    }

    fn send_collected(&mut self) {
        let d: Vec<Value> = self
            .play
            .collected
            .iter()
            .map(|(k, t, _)| json!({ "key": k, "text": t }))
            .collect();
        self.send_ev("collected", None, None, Value::Array(d));
        self.dirty = true;
    }

    pub(super) fn clear_collected(&mut self) {
        self.play.collected.clear();
        self.send_collected();
    }

    /// what the info line adds for the mode
    pub(super) fn mode_info(&self) -> String {
        match self.play.mode.as_str() {
            "walk" if self.play.rope.is_some() => "   on the grapple".into(),
            "walk" => "   walking".into(),
            "super" => format!(
                "   superflight {:.0}",
                self.play.flight.as_ref().map(|f| f.v.length()).unwrap_or(0.0)
            ),
            _ => String::new(),
        }
    }

    // ------------------------------------------------------------------ placing prefabs

    pub(super) fn start_ghost(&mut self, prefab: Value, parts: &[Value], textures: Option<&Value>) {
        if let Some(t) = textures.and_then(Value::as_object) {
            for (k, v) in t {
                self.blocks.abouts.insert(k.clone(), v.clone());
            }
        }
        self.play.ghost = Some(Ghost {
            prefab,
            parts: parts.to_vec(),
            turn: 0,
            ok: true,
            at: None,
        });
        self.dirty = true;
    }

    pub(super) fn stop_ghost(&mut self) {
        if self.play.ghost.take().is_some() {
            self.dirty = true;
        }
    }

    pub(super) fn ghosting(&self) -> bool {
        self.play.ghost.is_some()
    }

    /// the ghost's parts' corners, turned by its quarter turns (its own frame)
    fn ghost_box(&mut self, g: &Ghost) -> Option<(Vec3, Vec3)> {
        let q = Quat::from_rotation_y(g.turn as f32 * std::f32::consts::FRAC_PI_2);
        let mut out: Option<(Vec3, Vec3)> = None;
        for b in &g.parts {
            let (at, rot) = crate::blocks::place(b);
            for c in crate::blocks::shape(b).faces.iter().flatten() {
                let p = q * (rot * c.pos + at);
                out = Some(match out {
                    Some((lo, hi)) => (lo.min(p), hi.max(p)),
                    None => (p, p),
                });
            }
        }
        out
    }

    /// the ghost where you look: on a block's face or the floor, snapped, turned; red if
    /// it would overlap
    pub(super) fn aim_ghost(&mut self) {
        let Some(mut g) = self.play.ghost.take() else { return };
        let (x, y) = self.pointer;
        let d = self.ray_dir(x, y);
        let o = self.at;
        let (at, n) = match self.blocks.hit_n(o, d, 80.0) {
            Some((_, p, n)) => (p, n),
            None => {
                // the floor, where the ray meets it (else 12 units ahead)
                let t = if d.y < -1e-3 { (BodyK::FLOOR - o.y) / d.y } else { -1.0 };
                (if t > 0.0 && t < 200.0 { o + d * t } else { o + d * 12.0 }, Vec3::Y)
            }
        };
        let Some((lo, hi)) = self.ghost_box(&g) else {
            self.play.ghost = Some(g);
            return;
        };
        // the ghost's box rests against the surface along its normal
        let ax = if n.x.abs() > n.y.abs() && n.x.abs() > n.z.abs() {
            0
        } else if n.y.abs() >= n.z.abs() {
            1
        } else {
            2
        };
        let sign = if n[ax] < 0.0 { -1.0 } else { 1.0 };
        let mut pos = at;
        pos[ax] += if sign > 0.0 { -lo[ax] } else { -hi[ax] };
        let free = self.held.contains("ShiftLeft") || self.held.contains("ShiftRight");
        if !free {
            for k in 0..3 {
                if k != ax {
                    pos[k] = (pos[k] / 0.5).round() * 0.5;
                }
            }
        }
        let placed = (lo + pos + 0.01, hi + pos - 0.01);
        g.ok = !self.solids().iter().any(|s| boxes_meet(placed, *s));
        g.at = Some(pos);
        self.play.ghost = Some(g);
        self.dirty = true;
    }

    /// the click: a copy where the ghost is, if it fits
    fn place_ghost(&mut self) {
        let Some(g) = &self.play.ghost else { return };
        let Some(at) = g.at else { return };
        if !g.ok {
            self.toast("it would overlap a block there (red); move it, or turn it with the wheel");
            return;
        }
        let q = Quat::from_rotation_y(g.turn as f32 * std::f32::consts::FRAC_PI_2);
        let r = |v: f32| ((v as f64) * 1e4).round() / 1e4;
        let data = json!({ "prefab": g.prefab, "x": r(at.x), "y": r(at.y), "z": r(at.z), "rot": [q.x, q.y, q.z, q.w] });
        self.send_ev("ghostplace", None, None, data);
    }

    /// a press while placing a prefab: a click places it, a right click stops
    pub(super) fn ghost_down(&mut self, button: u8) -> bool {
        if self.play.ghost.is_none() || (button != 0 && button != 2) {
            return false;
        }
        if button == 0 {
            self.place_ghost();
        } else {
            self.stop_ghost();
            self.send_ev("ghostdone", None, None, Value::Null);
        }
        true
    }

    /// the wheel while placing (a quarter turn) or on the grapple (the rope reeled)
    pub(super) fn play_wheel(&mut self, dy: f32) -> bool {
        if let Some(g) = &mut self.play.ghost {
            g.turn = (g.turn + if dy > 0.0 { 1 } else { 3 }) % 4;
            self.dirty = true;
            return true;
        }
        if let Some((_, len)) = &mut self.play.rope {
            *len = (*len + if dy > 0.0 { 0.8 } else { -0.8 }).clamp(1.2, 60.0);
            return true;
        }
        false
    }

    /// the play's keys: walking, superflight, the grapple, Esc while placing
    pub(super) fn play_key(&mut self, action: Option<&str>, code: &str) -> bool {
        if code == "Escape" && self.play.ghost.is_some() {
            self.stop_ghost();
            self.send_ev("ghostdone", None, None, Value::Null);
            return true;
        }
        match action {
            Some("walk") => {
                let m = if self.play.mode == "walk" { "fly" } else { "walk" };
                self.set_mode(m);
            }
            Some("superflight") => {
                let m = if self.play.mode == "super" { "fly" } else { "super" };
                self.set_mode(m);
            }
            Some("grapple") => self.grapple(),
            _ => return false,
        }
        true
    }

    /// each prefab as a small picture of its parts (the prefab menu's), seen from above
    /// and to the side, on a see-through ground: scenes for the desktop to draw
    pub(super) fn thumbs(&mut self, list: &[Value], textures: Option<&Value>) {
        if let Some(t) = textures.and_then(Value::as_object) {
            for (k, v) in t {
                self.blocks.abouts.insert(k.clone(), v.clone());
            }
        }
        for (i, p) in list.iter().enumerate() {
            let parts = p["parts"].as_array().cloned().unwrap_or_default();
            if parts.is_empty() {
                continue;
            }
            let mut meshes = Vec::new();
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for (j, b) in parts.iter().enumerate() {
                let (at, rot) = crate::blocks::place(b);
                let m = Mat4::from_rotation_translation(rot, at);
                for c in crate::blocks::shape(b).faces.iter().flatten() {
                    let w = m.transform_point3(c.pos);
                    lo = lo.min(w);
                    hi = hi.max(w);
                }
                for (k, (tex, verts)) in self.blocks.geometry(b, m, rot, None).into_iter().enumerate() {
                    meshes.push(Mesh {
                        id: (1 << 40) + (i as u64) * 4096 + (j as u64) * 8 + k as u64,
                        version: 1,
                        verts: Arc::new(verts),
                        texture: tex,
                        pass: Pass::Opaque,
                        model: m,
                        cull: true,
                        lines: false,
                    });
                }
            }
            let c = (lo + hi) / 2.0;
            let r = ((hi - lo).length() / 2.0).max(0.5);
            let fov = 35f32.to_radians();
            let eye = c + Vec3::new(0.9, 0.7, 1.4).normalize() * (r / (fov / 2.0).sin());
            let d = (c - eye).normalize();
            let camera = crate::Camera {
                at: eye,
                yaw: (-d.x).atan2(-d.z),
                pitch: d.y.asin(),
                roll: 0.0,
                fov,
            };
            let scene = crate::Scene {
                meshes,
                textures: self.blocks.textures(),
                clear: Some([0.0; 4]),
                ..Default::default()
            };
            self.play.thumbs.push((p["id"].clone(), scene, camera));
        }
    }

    /// the prefabs' pictures asked for, to draw
    pub fn take_thumbs(&mut self) -> Vec<(Value, crate::Scene, crate::Camera)> {
        std::mem::take(&mut self.play.thumbs)
    }

    /// the commands of what plays (space.js's protocol); false for one that isn't
    pub(super) fn play_command(&mut self, cmd: &str, c: &Value) -> bool {
        match cmd {
            "anims" => self.set_anims(
                c["anims"].as_array().map(Vec::as_slice).unwrap_or(&[]),
                c.get("textures"),
            ),
            "playanims" => self.play_anims(c["on"] == true),
            "animpreview" => self.preview_anim(
                c["id"].as_str().unwrap_or(""),
                c.get("anim").cloned(),
                c["p"].as_f64().map(|p| p as f32),
            ),
            "ghost" => self.start_ghost(
                c["prefab"].clone(),
                c["parts"].as_array().map(Vec::as_slice).unwrap_or(&[]),
                c.get("textures"),
            ),
            "noghost" => self.stop_ghost(),
            "clearcollected" => self.clear_collected(),
            "thumbs" => self.thumbs(
                c["prefabs"].as_array().map(Vec::as_slice).unwrap_or(&[]),
                c.get("textures"),
            ),
            "preview" => {
                let id = c["id"].as_str().unwrap_or("").to_string();
                let (dw, dh) = match id.as_str() {
                    "tx-tile" => (96, 96),
                    "tx-patch" => (200, 200),
                    x if x.starts_with("tx-saved-") => (56, 56),
                    _ => (240, 240),
                };
                let (w, h) = (
                    c["w"].as_f64().map(|v| v as u32).unwrap_or(dw),
                    c["h"].as_f64().map(|v| v as u32).unwrap_or(dh),
                );
                let tiles = c["tiles"].as_f64().unwrap_or(1.0).max(1.0) as u32;
                let src = self.blocks.preview(&c["about"], tiles, c["variants"] == true, w, h);
                self.send(json!({ "kind": "preview", "data": { "id": id, "src": src } }));
            }
            _ => return false,
        }
        true
    }

    /// the rope, the trail and the ghost, as meshes
    pub(super) fn play_meshes(&mut self, out: &mut Vec<Mesh>) {
        let base = u64::MAX - 8000;
        let line = |id: u64, pts: Vec<Vec3>, c: [f32; 4]| Mesh {
            id,
            version: pts.len() as u64
                ^ pts
                    .last()
                    .map(|v| (v.x.to_bits() as u64) << 9 ^ v.y.to_bits() as u64 ^ (v.z.to_bits() as u64) << 21)
                    .unwrap_or(0),
            verts: Arc::new(
                pts.into_iter()
                    .map(|p| Vert {
                        pos: p.to_array(),
                        uv: [0.0, 0.0],
                        colour: c,
                    })
                    .collect(),
            ),
            texture: None,
            pass: Pass::Opaque,
            model: Mat4::IDENTITY,
            cull: false,
            lines: true,
        };
        if let (Some(b), Some((at, _))) = (&self.play.body, self.play.rope) {
            let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
            let hand = Vec3::new(b.p.x, b.p.y + b.h - 0.35, b.p.z) + right * 0.25;
            out.push(line(base, vec![hand, at], straight("#5a4632", 1.0)));
        }
        if self.play.collected.len() >= 2 {
            let pts: Vec<Vec3> = self.play.collected.iter().map(|c| c.2).collect();
            let segs: Vec<Vec3> = pts.windows(2).flat_map(|w| [w[0], w[1]]).collect();
            out.push(line(base + 1, segs, straight("#ff8c1a", 1.0)));
        }
        let Some(g) = self.play.ghost.clone() else { return };
        let Some(at) = g.at else { return };
        let q = Quat::from_rotation_y(g.turn as f32 * std::f32::consts::FRAC_PI_2);
        let tint = straight(if g.ok { "#b8ffc0" } else { "#ff6a6a" }, 0.55);
        for (i, b) in g.parts.iter().enumerate() {
            let (p, rot) = crate::blocks::place(b);
            let m = Mat4::from_rotation_translation(rot, p);
            for (j, (tex, verts)) in self.blocks.geometry(b, m, rot, None).into_iter().enumerate() {
                let verts: Vec<Vert> = verts
                    .into_iter()
                    .map(|v| Vert {
                        colour: [
                            v.colour[0] * tint[0],
                            v.colour[1] * tint[1],
                            v.colour[2] * tint[2],
                            tint[3],
                        ],
                        ..v
                    })
                    .collect();
                out.push(Mesh {
                    id: base + 100 + (i * 8 + j) as u64,
                    version: g.ok as u64 + 2 * g.parts.len() as u64,
                    verts: Arc::new(verts),
                    texture: tex,
                    pass: Pass::Glass,
                    model: Mat4::from_rotation_translation(q, at) * m,
                    cull: true,
                    lines: false,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animations_pose_and_walking_lands() {
        let a = json!({"duration": 2.0, "repeat": "loop", "keys": [{"at": 0.0, "move": [0, 0, 0]}, {"at": 1.0, "move": [0, 4, 0]}]});
        assert!((anim_phase(&a, 3.0) - 0.5).abs() < 1e-5);
        let (mv, _, sc, _) = anim_pose(&a, 0.5);
        assert!(mv[1] > 0.5 && mv[1] < 3.5, "{mv:?}");
        assert_eq!(sc, [1.0, 1.0, 1.0]);
        let mut r = Room::new();
        r.command(&json!({"cmd": "blocks", "blocks": [
            {"id": "floor", "kind": "plain", "texture": "#888", "x": 0, "y": -1, "z": 0, "w": 10, "h": 1, "d": 10, "anim": null}
        ]}));
        r.command(&json!({"cmd": "start"}));
        r.key_down("KeyV", Default::default());
        assert_eq!(r.play.mode, "walk");
        for _ in 0..120 {
            r.frame(1.0 / 60.0);
        }
        // standing on the block's top (y −0.5), the eyes 1.6 above
        let b = r.play.body.as_ref().unwrap();
        assert!(b.ground && (b.p.y + 0.5).abs() < 0.01, "{:?}", b.p);
        assert!((r.at.y - 1.1).abs() < 0.02, "{}", r.at.y);
        // an animation moves a block where it's drawn, not where it was put
        r.command(&json!({"cmd": "blocks", "blocks": [
            {"id": "a", "kind": "plain", "texture": "#888", "x": 0, "y": 5, "z": 0, "w": 1, "h": 1, "d": 1, "anim": {"key": "up"}}
        ]}));
        r.command(&json!({"cmd": "anims", "anims": [{"key": "up", "anim": a}]}));
        r.frame(0.5);
        let m = r.blocks.model("a").unwrap();
        assert!(m.w_axis.y > 5.1, "{}", m.w_axis.y);
        assert_eq!(r.blocks.get("a").unwrap()["y"], 5);
    }
}
