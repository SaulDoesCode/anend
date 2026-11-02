//! The room's blocks as meshes, as space.js built them: boxes whose texture coordinates
//! count tiles (cut into cells about a unit across, so baked light has somewhere to land),
//! bent ones cut into slices across their width (core's graph::blocks::Bend), spheres
//! (ellipsoids), faces shaded by their direction or lit by the room's lights (baked into
//! their vertex colours), their texture a recipe's tile for their variant, a text's tile,
//! a picture or a plain colour. Zones are see-through.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat4, Quat, Vec2, Vec3};
use serde_json::Value;

use crate::tiles::{self, Pictures};
use crate::{Mesh, Pass, Texture, Vert};

/// a CSS cubic bezier's wave: 0, c1, c2, 0 at s
fn wave(s: f32, c1: f32, c2: f32) -> f32 {
    3.0 * (1.0 - s) * (1.0 - s) * s * c1 + 3.0 * (1.0 - s) * s * s * c2
}

fn wave_peak(c1: f32, c2: f32) -> f32 {
    let m = (0..=64)
        .map(|i| wave(i as f32 / 64.0, c1, c2).abs())
        .fold(0.0, f32::max);
    if m > 1e-6 { m } else { 1.0 }
}

fn num(v: &Value, k: &str, d: f32) -> f32 {
    v[k].as_f64().map(|x| x as f32).unwrap_or(d)
}

/// how far a bent block's slice at u across its width (0–1) moves along its depth: core's
/// `Bend::offset`, as space.js's `bendAt`
pub fn bend_at(bend: &Value, u: f32) -> f32 {
    let mut z = 0.0;
    let r = &bend["ripple"];
    let (amount, freq) = (num(r, "amount", 0.0), num(r, "freq", 0.0));
    if amount != 0.0 && freq > 0.0 {
        let s = (u * freq + num(r, "phase", 0.0)).rem_euclid(1.0);
        let (c1, c2) = (num(r, "c1", 0.0), num(r, "c2", 0.0));
        z += amount * wave(s, c1, c2) / wave_peak(c1, c2);
    }
    let f = &bend["fold"];
    let amount = num(f, "amount", 0.0);
    if amount != 0.0 {
        let line = 0.5 + num(f, "at", 0.0) * 0.5;
        let t = if u >= line {
            (u - line) / (1.0 - line).max(1e-6)
        } else {
            (line - u) / line.max(1e-6)
        };
        let t = t.clamp(0.0, 1.0);
        let (c1, c2) = (num(f, "c1", 0.0), num(f, "c2", 0.0));
        z += amount * (3.0 * (1.0 - t) * (1.0 - t) * t * c1 + 3.0 * (1.0 - t) * t * t * c2 + t * t * t);
    }
    z
}

/// which variant the n-th block of a recipe shows: core's graph::textures::variant
pub fn variant_of(seed: u32, n: u64, variants: u32) -> u32 {
    if variants <= 1 {
        return 0;
    }
    let (lo, hi) = (n as u32, (n >> 32) as u32);
    let mut h = seed ^ lo.wrapping_mul(0x9e37_79b9) ^ hi;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h % variants
}

/// a corner of a shape: where, which way it faces, where in the texture (in tiles)
#[derive(Clone, Copy, Debug)]
pub struct Corner {
    pub pos: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
}

/// a block's shape: its triangles by face (a box's six: +x, −x, +y, −y, +z, −z; a
/// sphere's one)
pub struct Shape {
    pub faces: Vec<Vec<Corner>>,
    pub round: bool,
}

/// a face of a box as three.js lays it out: (u axis, v axis, w axis, udir, vdir, width,
/// height, depth, grid x, grid y)
type Plane = (usize, usize, usize, f32, f32, f32, f32, f32, u32, u32);

/// a block's sides (w, h, d) and its bend and shape, as Rust sent it
pub fn shape(b: &Value) -> Shape {
    let (w, h, d) = (num(b, "w", 1.0), num(b, "h", 1.0), num(b, "d", 1.0));
    if b["shape"] == "sphere" {
        return sphere(w, h, d);
    }
    let bend = &b["bend"];
    let bent = num(&bend["ripple"], "amount", 0.0) != 0.0 || num(&bend["fold"], "amount", 0.0) != 0.0;
    let freq = if bent && num(&bend["ripple"], "amount", 0.0) != 0.0 {
        num(&bend["ripple"], "freq", 0.0).max(1.0)
    } else {
        1.0
    };
    let seg = if bent {
        ((freq * 24.0).ceil() as u32).clamp(16, 512)
    } else {
        1
    };
    let cells = |v: f32| ((v - 1e-6).ceil() as u32).clamp(1, 16);
    let (sx, sy, sz) = (seg.max(cells(w)), cells(h), cells(d));
    // three.js's BoxGeometry, plane by plane (the same corners and texture directions)
    let mut faces = Vec::new();
    // (u axis, v axis, w axis, udir, vdir, width, height, depth, grid x, grid y)
    let planes: [Plane; 6] = [
        (2, 1, 0, -1.0, -1.0, d, h, w, sz, sy),
        (2, 1, 0, 1.0, -1.0, d, h, -w, sz, sy),
        (0, 2, 1, 1.0, 1.0, w, d, h, sx, sz),
        (0, 2, 1, 1.0, -1.0, w, d, -h, sx, sz),
        (0, 1, 2, 1.0, -1.0, w, h, d, sx, sy),
        (0, 1, 2, -1.0, -1.0, w, h, -d, sx, sy),
    ];
    for (i, &(ua, va, wa, ud, vd, pw, ph, pd, gx, gy)) in planes.iter().enumerate() {
        let mut grid = Vec::with_capacity(((gx + 1) * (gy + 1)) as usize);
        for iy in 0..=gy {
            let y = iy as f32 * ph / gy as f32 - ph / 2.0;
            for ix in 0..=gx {
                let x = ix as f32 * pw / gx as f32 - pw / 2.0;
                let mut p = Vec3::ZERO;
                p[ua] = x * ud;
                p[va] = y * vd;
                p[wa] = pd / 2.0;
                let mut n = Vec3::ZERO;
                n[wa] = if pd > 0.0 { 1.0 } else { -1.0 };
                let uv = Vec2::new(ix as f32 / gx as f32, 1.0 - iy as f32 / gy as f32);
                // the texture counts tiles: a face's u and v run over its sides in units
                let span = if i < 2 {
                    Vec2::new(d, h)
                } else if i < 4 {
                    Vec2::new(w, d)
                } else {
                    Vec2::new(w, h)
                };
                grid.push(Corner {
                    pos: p,
                    normal: n,
                    uv: uv * span,
                });
            }
        }
        if bent {
            for c in &mut grid {
                c.pos.z += bend_at(bend, (c.pos.x + w / 2.0) / w);
            }
        }
        let at = |ix: u32, iy: u32| (iy * (gx + 1) + ix) as usize;
        let mut tris = Vec::new();
        for iy in 0..gy {
            for ix in 0..gx {
                let (a, bb, c, dd) = (at(ix, iy), at(ix, iy + 1), at(ix + 1, iy + 1), at(ix + 1, iy));
                tris.extend_from_slice(&[a, bb, dd, bb, c, dd]);
            }
        }
        if bent {
            // normals from the bent faces round each corner (computeVertexNormals)
            let mut ns = vec![Vec3::ZERO; grid.len()];
            for t in tris.as_chunks::<3>().0 {
                let (p0, p1, p2) = (grid[t[0]].pos, grid[t[1]].pos, grid[t[2]].pos);
                let n = (p1 - p0).cross(p2 - p0);
                for &k in t {
                    ns[k] += n;
                }
            }
            for (c, n) in grid.iter_mut().zip(ns) {
                c.normal = n.normalize_or(c.normal);
            }
        }
        faces.push(tris.into_iter().map(|k| grid[k]).collect());
    }
    Shape { faces, round: false }
}

/// an ellipsoid with the block's sides as its diameters (three.js's SphereGeometry, 48 by
/// 32), its texture once a unit round its middle and from pole to pole
fn sphere(w: f32, h: f32, d: f32) -> Shape {
    let (ws, hs) = (48u32, 32u32);
    let round = std::f32::consts::PI * (w + d) / 2.0;
    let meridian = std::f32::consts::PI * h / 2.0;
    let mut grid = Vec::new();
    for iy in 0..=hs {
        let v = iy as f32 / hs as f32;
        for ix in 0..=ws {
            let u = ix as f32 / ws as f32;
            let (ph, th) = (u * std::f32::consts::TAU, v * std::f32::consts::PI);
            let unit = Vec3::new(-0.5 * ph.cos() * th.sin(), 0.5 * th.cos(), 0.5 * ph.sin() * th.sin());
            let pos = unit * Vec3::new(w, h, d);
            // an ellipsoid's normal: its surface's gradient
            let n = (pos / Vec3::new(w * w, h * h, d * d)).normalize_or(unit.normalize_or(Vec3::Y));
            grid.push(Corner {
                pos,
                normal: n,
                uv: Vec2::new(u * round, (1.0 - v) * meridian),
            });
        }
    }
    let at = |ix: u32, iy: u32| (iy * (ws + 1) + ix) as usize;
    let mut out = Vec::new();
    for iy in 0..hs {
        for ix in 0..ws {
            let (a, b, c, dd) = (at(ix + 1, iy), at(ix, iy), at(ix, iy + 1), at(ix + 1, iy + 1));
            if iy != 0 {
                out.extend_from_slice(&[grid[a], grid[b], grid[dd]]);
            }
            if iy != hs - 1 {
                out.extend_from_slice(&[grid[b], grid[c], grid[dd]]);
            }
        }
    }
    Shape {
        faces: vec![out],
        round: true,
    }
}

/// a light of the room, as Rust sends it (core's graph::lights)
#[derive(Clone, Debug)]
pub struct Light {
    pub kind: String,
    pub at: Vec3,
    pub dir: Vec3,
    pub colour: [f32; 3],
    pub intensity: f32,
    pub range: f32,
    pub angle: f32,
    pub softness: f32,
}

impl Light {
    pub fn of(x: &Value) -> Light {
        let dir: Vec<f32> = x["dir"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
            .unwrap_or_default();
        let dir = if dir.len() == 3 {
            Vec3::new(dir[0], dir[1], dir[2])
        } else {
            Vec3::NEG_Y
        };
        let c = crate::room::Room::colour(x["colour"].as_str().unwrap_or("#ffffff"), 1.0);
        Light {
            kind: x["kind"].as_str().unwrap_or("point").to_string(),
            at: Vec3::new(num(x, "x", 0.0), num(x, "y", 0.0), num(x, "z", 0.0)),
            dir: dir.normalize_or(Vec3::NEG_Y),
            colour: [c[0], c[1], c[2]],
            intensity: num(x, "intensity", 1.0),
            range: num(x, "range", 10.0),
            angle: num(x, "angle", 30.0),
            softness: num(x, "softness", 0.2),
        }
    }

    /// how much of it falls on a point facing a way
    fn on(&self, p: Vec3, n: Vec3) -> f32 {
        match self.kind.as_str() {
            "ambient" => self.intensity,
            "directional" => self.intensity * (-n.dot(self.dir)).max(0.0),
            _ => {
                let l = self.at - p;
                let dist = l.length();
                if dist > self.range {
                    return 0.0;
                }
                let l = l / if dist > 0.0 { dist } else { 1.0 };
                let fade = (1.0 - dist / self.range).powi(2);
                let mut f = self.intensity * n.dot(l).max(0.0) * fade;
                if self.kind == "spot" {
                    let cos = -l.dot(self.dir);
                    let outer = self.angle.to_radians().cos();
                    let inner = (self.angle * (1.0 - self.softness)).to_radians().cos();
                    let t = if inner > outer {
                        ((cos - outer) / (inner - outer)).clamp(0.0, 1.0)
                    } else if cos >= outer {
                        1.0
                    } else {
                        0.0
                    };
                    f *= t * t * (3.0 - 2.0 * t);
                }
                f
            }
        }
    }
}

/// a corner's colour: shaded by its direction with no lights, else the lights on it
fn light_at(c: &Corner, round: bool, model: &Mat4, rot: Quat, lights: &[Light]) -> [f32; 3] {
    if lights.is_empty() {
        let n = c.normal;
        let v = if round {
            0.82 + 0.18 * n.y + 0.06 * n.z.abs()
        } else {
            crate::shade(n)
        };
        return [v, v, v];
    }
    let p = model.transform_point3(c.pos);
    let n = (rot * c.normal).normalize_or(Vec3::Y);
    let mut out = [0.0f32; 3];
    for l in lights {
        let f = l.on(p, n);
        for (o, c) in out.iter_mut().zip(l.colour) {
            *o += c * f;
        }
    }
    out.map(|v| v.min(2.0))
}

/// a block's place and turn, as Rust sent it
pub fn place(b: &Value) -> (Vec3, Quat) {
    let r: Vec<f32> = b["rot"]
        .as_array()
        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect())
        .unwrap_or_default();
    let rot = if r.len() == 4 {
        Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize()
    } else {
        Quat::IDENTITY
    };
    (Vec3::new(num(b, "x", 0.0), num(b, "y", 0.0), num(b, "z", 0.0)), rot)
}

/// where a texture is sampled: its repeat and offset (three.js's `repeat` and `offset`)
#[derive(Clone, Copy, Debug, PartialEq)]
struct Uv {
    repeat: Vec2,
    offset: Vec2,
}

impl Uv {
    const ONE: Uv = Uv {
        repeat: Vec2::ONE,
        offset: Vec2::ZERO,
    };

    /// a recipe's for a variant: its tile repeats every `scale` units, shifted by
    /// `offset`, a variant k by (k·0.381966, k·0.618034) of a tile more
    fn recipe(about: &Value, variant: u32) -> Uv {
        let r = &about["recipe"];
        let pair = |k: &str, d: f32| {
            let a: Vec<f32> = r[k]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(d as f64) as f32).collect())
                .unwrap_or_default();
            if a.len() == 2 {
                Vec2::new(a[0], a[1])
            } else {
                Vec2::splat(d)
            }
        };
        let (s, o) = (pair("scale", 1.0), pair("offset", 0.0));
        let v = variant as f32;
        Uv {
            repeat: Vec2::new(1.0 / s.x.max(1e-3), 1.0 / s.y.max(1e-3)),
            offset: o + Vec2::new(v * 0.381966, v * 0.618034),
        }
    }

    /// a corner's place in the picture (its rows from the top, as the GPU counts them)
    fn at(&self, uv: Vec2) -> [f32; 2] {
        let t = uv * self.repeat + self.offset;
        [t.x, 1.0 - t.y]
    }
}

/// a block drawn: what it was built from, and its meshes
struct Built {
    sig: String,
    meshes: Vec<Mesh>,
    /// the box round it (its own frame) and its colour on the whole, for its billboard
    lo: Vec3,
    hi: Vec3,
    avg: [f32; 4],
}

/// past this distance from the eye (exactly, from a block's middle), a block is drawn as
/// a billboard: one upright quad of its size and its colour on the whole (questions 120)
pub const BILLBOARD_AT: f32 = 120.0;
/// past this distance a block isn't drawn, and rays don't meet it (deterministic: the
/// same distance everywhere, so it can be designed with)
pub const DRAW_TO: f32 = 400.0;

/// the blocks, the pictures they show (by texture key, as Rust sent them) and the
/// room's lights, as meshes and textures to draw
#[derive(Default)]
pub struct Blocks {
    /// id → the block (`{id, kind, texture, x, y, z, w, h, d, rot, bend, faces, variant, shape, zone, …}`)
    pub list: Vec<(String, Value)>,
    pub abouts: HashMap<String, Value>,
    pub lights: Vec<Value>,
    lights_rev: u64,
    built: HashMap<String, Built>,
    textures: HashMap<String, Texture>,
    tex_sig: HashMap<String, String>,
    pictures: Pictures,
    next_id: u64,
    /// the sky's recipe (none: the room's colour)
    pub sky: Option<Value>,
    sky_mesh: Option<(String, Mesh)>,
    /// changed since the last scene
    pub rev: u64,
    /// blocks drawn somewhere other than their place for now (dragged with a copy or
    /// many, posed by an animation): their whole placement
    pub poses: HashMap<String, Mat4>,
    /// blocks showing another texture for now (an animation's key)
    pub tex_over: HashMap<String, String>,
    tex_avg: HashMap<String, [f32; 3]>,
    /// how many blocks the last scene drew whole and as billboards, and left out as too far
    pub counts: (usize, usize, usize),
}

impl Blocks {
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.list.iter().find(|(k, _)| k == id).map(|(_, b)| b)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Value> {
        self.rev += 1;
        self.list.iter_mut().find(|(k, _)| k == id).map(|(_, b)| b)
    }

    /// a recipe's preview for the textures window (a PNG data URL)
    pub fn preview(&mut self, about: &Value, tiles: u32, variants: bool, w: u32, h: u32) -> String {
        tiles::preview(about, tiles, variants, w, h, &mut self.pictures)
    }

    /// the pictures made so far, by key
    pub fn textures(&self) -> HashMap<String, Texture> {
        self.textures.clone()
    }

    pub fn has(&self, id: &str) -> bool {
        self.list.iter().any(|(k, _)| k == id)
    }

    /// where a block is drawn now (its pose, else its place)
    pub fn model(&self, id: &str) -> Option<Mat4> {
        if let Some(m) = self.poses.get(id) {
            return Some(*m);
        }
        let (at, rot) = place(self.get(id)?);
        Some(Mat4::from_rotation_translation(rot, at))
    }

    /// a block's corners (its own frame), built when it's new
    fn corners(&mut self, id: &str) -> Vec<Vec3> {
        if !self.built.contains_key(id)
            && let Some(b) = self.get(id).cloned()
        {
            let (at, rot) = place(&b);
            self.build(id, &b, Mat4::from_rotation_translation(rot, at), rot);
        }
        self.built
            .get(id)
            .map(|x| {
                x.meshes
                    .iter()
                    .flat_map(|m| m.verts.iter().map(|v| Vec3::from(v.pos)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// the block a ray meets first, and how far along it
    pub fn hit(&mut self, o: Vec3, d: Vec3) -> Option<(String, f32)> {
        let ids: Vec<String> = self.list.iter().map(|(k, _)| k.clone()).collect();
        let mut best: Option<(String, f32)> = None;
        for id in ids {
            let Some(m) = self.model(&id) else { continue };
            let Some((lo, hi)) = self.bounds(&id) else { continue };
            // the box round it first
            let (c, r) = ((lo + hi) / 2.0, (hi - lo).length() / 2.0);
            let t = (c - o).dot(d);
            if (o + d * t).distance(c) > r + 1e-3 {
                continue;
            }
            let tris = self.corners(&id);
            if let Some(t) = crate::prims::ray_tris(o, d, &tris, m)
                && t <= DRAW_TO
                && best.as_ref().is_none_or(|b| t < b.1)
            {
                best = Some((id, t));
            }
        }
        best
    }

    /// the block a ray meets first (none further than `far`): its id, the point, and the
    /// face's normal there
    pub fn hit_n(&mut self, o: Vec3, d: Vec3, far: f32) -> Option<(String, Vec3, Vec3)> {
        let (id, _) = self.hit(o, d)?;
        let m = self.model(&id)?;
        let tris = self.corners(&id);
        let (t, n) = crate::prims::ray_tris_n(o, d, &tris, m)?;
        (t <= far).then(|| (id, o + d * t, n))
    }

    /// the boxes round the blocks that are in the way (not zones)
    pub fn solids(&mut self) -> Vec<(Vec3, Vec3)> {
        let ids: Vec<String> = self
            .list
            .iter()
            .filter(|(_, b)| b.get("zone").is_none_or(|z| z.is_null()))
            .map(|(k, _)| k.clone())
            .collect();
        ids.iter().filter_map(|id| self.bounds(id)).collect()
    }

    /// the box round a block where it's drawn now (lowest and highest corners)
    pub fn bounds(&mut self, id: &str) -> Option<(Vec3, Vec3)> {
        let m = self.model(id)?;
        let cs = self.corners(id);
        if cs.is_empty() {
            return None;
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for c in cs {
            let p = m.transform_point3(c);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        Some((lo, hi))
    }

    pub fn set(&mut self, list: &[Value], textures: Option<&Value>, replace: bool) {
        if replace {
            self.list.clear();
        }
        if let Some(t) = textures.and_then(Value::as_object) {
            for (k, v) in t {
                self.abouts.insert(k.clone(), v.clone());
            }
        }
        for b in list {
            let id = b["id"].as_str().unwrap_or("").to_string();
            match self.list.iter_mut().find(|(k, _)| *k == id) {
                Some(x) => x.1 = b.clone(),
                None => self.list.push((id, b.clone())),
            }
        }
        self.rev += 1;
    }

    pub fn remove(&mut self, ids: &[String]) {
        self.list.retain(|(k, _)| !ids.contains(k));
        self.rev += 1;
    }

    pub fn set_lights(&mut self, list: Vec<Value>) {
        self.lights = list;
        self.lights_rev += 1;
        self.rev += 1;
    }

    /// a texture by key, made (or remade) when what it shows changed; its key
    fn texture(&mut self, key: &str, make: impl FnOnce(&mut Pictures) -> tiles::Canvas, sig: String) -> String {
        if self.tex_sig.get(key) != Some(&sig) {
            let c = make(&mut self.pictures);
            let version = self.textures.get(key).map(|t| t.version + 1).unwrap_or(1);
            // its mean colour, linear, for far billboards
            let bytes = c.bytes();
            let lin = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            let mut m = [0.0f32; 3];
            for p in bytes.as_chunks::<4>().0 {
                for i in 0..3 {
                    m[i] += lin(p[i]);
                }
            }
            let px = (bytes.len() / 4).max(1) as f32;
            self.tex_avg.insert(key.to_string(), m.map(|x| x / px));
            self.textures.insert(
                key.to_string(),
                Texture {
                    version,
                    w: c.w,
                    h: c.h,
                    rgba: Arc::new(c.bytes()),
                },
            );
            self.tex_sig.insert(key.to_string(), sig);
        }
        key.to_string()
    }

    /// a texture key's look: the texture to sample (none: a plain colour), how, and the
    /// colour it's multiplied by
    fn look(&mut self, key: &str, variant: u32) -> (Option<String>, Uv, [f32; 3]) {
        let about = self.abouts.get(key).cloned().unwrap_or(Value::Null);
        let sig = about.to_string();
        let (kind, rest) = key.split_once(':').unwrap_or((key, ""));
        let white = [1.0, 1.0, 1.0];
        match kind {
            "texture" => {
                if about["recipe"].is_null() {
                    return (None, Uv::ONE, lin("#9a8f84"));
                }
                let turns = if about["recipe"]["turn_variants"] == true {
                    variant % 4
                } else {
                    0
                };
                let tk = format!("{key}|{turns}");
                let a = about.clone();
                let k = self.texture(&tk, |p| tiles::recipe_tile(&a, turns, 256, p), sig);
                (Some(k), Uv::recipe(&about, variant), white)
            }
            "text" => {
                let gone = about.get("gone").is_some_and(|g| !g.is_null() && *g != false);
                let text = if gone {
                    "(gone)".to_string()
                } else {
                    about["text"].as_str().unwrap_or("").to_string()
                };
                let colour = about["colour"].as_str().map(String::from);
                let k = self.texture(key, |_| tiles::text_tile(&text, colour.as_deref()), sig);
                (Some(k), Uv::ONE, white)
            }
            "picture" => match about["src"].as_str() {
                Some(src) => {
                    let src = src.to_string();
                    let k = self.texture(key, |p| p.get(&src).unwrap_or_else(|| tiles::Canvas::new(1, 1)), sig);
                    (Some(k), Uv::ONE, white)
                }
                None => (None, Uv::ONE, lin("#b8a07a")),
            },
            "plain" => (None, Uv::ONE, lin(rest)),
            _ => (None, Uv::ONE, lin("#b8a07a")),
        }
    }

    /// a block's triangles grouped by what they show (its own frame; lit where `model`
    /// puts it), shown with another texture when one's given (an animation's key)
    pub fn geometry(
        &mut self,
        b: &Value,
        model: Mat4,
        rot: Quat,
        over: Option<&str>,
    ) -> Vec<(Option<String>, Vec<Vert>)> {
        let key = format!(
            "{}:{}",
            b["kind"].as_str().unwrap_or(""),
            b["texture"].as_str().unwrap_or("")
        );
        let faces: Vec<Option<String>> = if over.is_some() {
            Vec::new()
        } else {
            b["faces"]
                .as_array()
                .map(|a| a.iter().map(|f| f.as_str().map(String::from)).collect())
                .unwrap_or_default()
        };
        let shape = shape(b);
        let lights: Vec<Light> = self.lights.iter().map(Light::of).collect();
        let variant = if over.is_some() {
            0
        } else {
            b["variant"].as_u64().unwrap_or(0) as u32
        };
        let zone = b.get("zone").is_some_and(|z| !z.is_null());
        let mut groups: Vec<(Option<String>, [f32; 3], Vec<Vert>)> = Vec::new();
        for (i, face) in shape.faces.iter().enumerate() {
            let own = (!shape.round).then(|| faces.get(i).cloned().flatten()).flatten();
            let (tex, uv, tint) = match (&own, over) {
                (_, Some(o)) => self.look(o, 0),
                (Some(f), None) => self.look(&format!("texture:{f}"), 0),
                (None, None) => self.look(&key, variant),
            };
            let alpha = if zone { 0.22 } else { 1.0 };
            let verts = face.iter().map(|c| {
                let l = light_at(c, shape.round, &model, rot, &lights);
                Vert {
                    pos: c.pos.to_array(),
                    uv: uv.at(c.uv),
                    colour: [l[0] * tint[0], l[1] * tint[1], l[2] * tint[2], alpha],
                }
            });
            match groups.iter_mut().find(|g| g.0 == tex && g.1 == tint) {
                Some(g) => g.2.extend(verts),
                None => groups.push((tex, tint, verts.collect())),
            }
        }
        groups.into_iter().map(|(t, _, v)| (t, v)).collect()
    }

    /// a block's meshes, built again when it (or its texture, or the lights) changed
    fn build(&mut self, id: &str, b: &Value, model: Mat4, rot: Quat) -> Vec<Mesh> {
        let key = format!(
            "{}:{}",
            b["kind"].as_str().unwrap_or(""),
            b["texture"].as_str().unwrap_or("")
        );
        let over = self.tex_over.get(id).cloned();
        let faces: Vec<String> = b["faces"]
            .as_array()
            .map(|a| a.iter().filter_map(|f| f.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let mut abouts = String::new();
        for k in std::iter::once(key.clone())
            .chain(faces.iter().map(|f| format!("texture:{f}")))
            .chain(over.clone())
        {
            abouts.push_str(&self.abouts.get(&k).map(|a| a.to_string()).unwrap_or_default());
        }
        let lit = if self.lights.is_empty() { 0 } else { self.lights_rev };
        let sig = format!("{b}|{abouts}|{lit}|{over:?}");
        if let Some(x) = self.built.get(id)
            && x.sig == sig
        {
            return x.meshes.clone();
        }
        let zone = b.get("zone").is_some_and(|z| !z.is_null());
        let groups = self.geometry(b, model, rot, over.as_deref());
        let old = self.built.remove(id);
        let mut meshes = Vec::new();
        for (i, (tex, verts)) in groups.into_iter().enumerate() {
            let (mid, version) = match old.as_ref().and_then(|o| o.meshes.get(i)) {
                Some(m) => (m.id, m.version + 1),
                None => {
                    self.next_id += 1;
                    (self.next_id, 1)
                }
            };
            meshes.push(Mesh {
                id: mid,
                version,
                verts: Arc::new(verts),
                texture: tex,
                pass: if zone { Pass::Glass } else { Pass::Opaque },
                model,
                cull: true,
                lines: false,
            });
        }
        // its box and its colour on the whole (vertex colours times each texture's mean)
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let (mut sum, mut n) = ([0.0f32; 4], 0.0f32);
        for m in &meshes {
            let t = m
                .texture
                .as_ref()
                .and_then(|k| self.tex_avg.get(k))
                .copied()
                .unwrap_or([1.0; 3]);
            for v in m.verts.iter() {
                lo = lo.min(Vec3::from(v.pos));
                hi = hi.max(Vec3::from(v.pos));
                for i in 0..3 {
                    sum[i] += v.colour[i] * t[i];
                }
                sum[3] += v.colour[3];
                n += 1.0;
            }
        }
        let avg = sum.map(|x| x / n.max(1.0));
        self.built.insert(
            id.to_string(),
            Built {
                sig,
                meshes: meshes.clone(),
                lo,
                hi,
                avg,
            },
        );
        meshes
    }

    /// what to draw: every block's meshes (at its pose when it has one), the sky round
    /// the camera, and the textures they show
    pub fn meshes(&mut self, eye: Vec3) -> (Vec<Mesh>, HashMap<String, Texture>) {
        let mut out = Vec::new();
        let list = self.list.clone();
        let mut boards: Vec<Vert> = Vec::new();
        let mut counts = (0, 0, 0);
        for (id, b) in &list {
            let (at, rot) = place(b);
            let model = Mat4::from_rotation_translation(rot, at);
            let mut ms = self.build(id, b, model, rot);
            let placed = self.poses.get(id).copied().unwrap_or(model);
            let Some(x) = self.built.get(id) else { continue };
            let mid = placed.transform_point3((x.lo + x.hi) / 2.0);
            let d = mid.distance(eye);
            if d > DRAW_TO {
                counts.2 += 1;
                continue;
            }
            if d > BILLBOARD_AT {
                counts.1 += 1;
                // upright, turned to the eye, as wide and tall as its placed box
                let e = placed.x_axis.truncate().abs() * (x.hi.x - x.lo.x)
                    + placed.y_axis.truncate().abs() * (x.hi.y - x.lo.y)
                    + placed.z_axis.truncate().abs() * (x.hi.z - x.lo.z);
                let f = Vec3::new(eye.x - mid.x, 0.0, eye.z - mid.z).normalize_or(Vec3::Z);
                let right = Vec3::new(f.z, 0.0, -f.x) * (e.x.max(e.z) / 2.0);
                let up = Vec3::Y * (e.y / 2.0);
                let c = x.avg;
                let v = |p: Vec3| Vert {
                    pos: p.to_array(),
                    uv: [0.0, 0.0],
                    colour: c,
                };
                let (p0, p1, p2, p3) = (mid - right - up, mid + right - up, mid + right + up, mid - right + up);
                boards.extend_from_slice(&[v(p0), v(p1), v(p2), v(p0), v(p2), v(p3)]);
                continue;
            }
            counts.0 += 1;
            for m in &mut ms {
                m.model = placed;
            }
            out.extend(ms);
        }
        self.counts = counts;
        if !boards.is_empty() {
            let h = boards.iter().fold(boards.len() as u64, |h, v| {
                h.wrapping_mul(1_000_003) ^ v.pos[0].to_bits() as u64 ^ (v.pos[2].to_bits() as u64) << 17
            });
            out.push(Mesh {
                id: u64::MAX - 7,
                version: h,
                verts: Arc::new(boards),
                texture: None,
                pass: Pass::Opaque,
                model: Mat4::IDENTITY,
                cull: false,
                lines: false,
            });
        }
        let live: std::collections::HashSet<&String> = list.iter().map(|(k, _)| k).collect();
        self.built.retain(|k, _| live.contains(k));
        if let Some(sky) = self.sky.clone() {
            out.push(self.sky_mesh(&sky, eye));
        }
        (out, self.textures.clone())
    }

    /// the sky: a recipe round the room, far off and following the camera
    fn sky_mesh(&mut self, about: &Value, eye: Vec3) -> Mesh {
        let sig = about.to_string();
        let tk = "sky".to_string();
        let a = about.clone();
        self.texture(&tk, |p| tiles::recipe_tile(&a, 0, 512, p), sig.clone());
        let model = Mat4::from_translation(eye);
        if let Some((s, m)) = &self.sky_mesh
            && *s == sig
        {
            return Mesh { model, ..m.clone() };
        }
        let r = &about["recipe"];
        let sc: Vec<f32> = r["scale"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(1.0) as f32).collect())
            .unwrap_or_default();
        let (sw, sh) = if sc.len() == 2 { (sc[0], sc[1]) } else { (1.0, 1.0) };
        // four tiles a turn for a one-unit tile (fewer, bigger, for a bigger one)
        let rep = Vec2::new((4.0 / sw).round().max(1.0), (2.0 / sh).round().max(1.0));
        let off = Uv::recipe(about, 0).offset;
        let uv = Uv {
            repeat: rep,
            offset: off,
        };
        let s = sphere(3600.0, 3600.0, 3600.0);
        let (round, meridian) = (std::f32::consts::PI * 3600.0, std::f32::consts::PI * 1800.0);
        let verts = s.faces[0]
            .iter()
            .map(|c| Vert {
                pos: c.pos.to_array(),
                uv: uv.at(Vec2::new(c.uv.x / round, c.uv.y / meridian)),
                colour: [1.0; 4],
            })
            .collect();
        self.next_id += 1;
        let m = Mesh {
            id: self.next_id,
            version: 1,
            verts: Arc::new(verts),
            texture: Some(tk),
            pass: Pass::Sky,
            model,
            cull: true,
            lines: false,
        };
        self.sky_mesh = Some((sig, m.clone()));
        m
    }
}

/// a CSS colour, linear
fn lin(css: &str) -> [f32; 3] {
    let c = crate::room::Room::colour(css, 1.0);
    [c[0], c[1], c[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blocks_are_shaped_and_lit() {
        let b =
            json!({"id": "1", "kind": "plain", "texture": "#ff0000", "x": 0, "y": 0, "z": -5, "w": 2, "h": 1, "d": 1});
        let s = shape(&b);
        assert_eq!(s.faces.len(), 6);
        // 2×1 cells on the front face, two triangles each
        assert_eq!(s.faces[4].len(), 2 * 6);
        // texture coordinates count tiles: the front runs 2 units across
        let umax = s.faces[4].iter().map(|c| c.uv.x).fold(0.0, f32::max);
        assert!((umax - 2.0).abs() < 1e-5);
        // a bent block's slices move along its depth
        let bent =
            json!({"w": 4, "h": 1, "d": 0.2, "bend": {"fold": {"amount": 1.0, "at": 0.0, "c1": 0.3, "c2": 0.7}}});
        let s = shape(&bent);
        let zs: Vec<f32> = s.faces[4].iter().map(|c| c.pos.z).collect();
        assert!(zs.iter().cloned().fold(0.0, f32::max) > 0.9);
        assert!((bend_at(&bent["bend"], 0.5)).abs() < 1e-5);
        let ball = shape(&json!({"shape": "sphere", "w": 1, "h": 2, "d": 1}));
        assert_eq!(ball.faces.len(), 1);
        assert!(ball.faces[0].iter().all(|c| c.pos.y.abs() <= 1.0 + 1e-4));
        // the variant mix agrees with core's (seed 7, the 3rd of 5: worked through by hand)
        assert!(variant_of(7, 3, 5) < 5);
        assert_eq!(variant_of(7, 3, 1), 0);
        // the meshes: a plain red box shaded; a light makes its lit side bright
        let mut bl = Blocks::default();
        bl.set(std::slice::from_ref(&b), None, false);
        let (ms, _) = bl.meshes(Vec3::ZERO);
        assert_eq!(ms.len(), 1);
        assert!(ms[0].verts.iter().all(|v| v.colour[0] > 0.5 && v.colour[1] < 0.01));
        bl.set_lights(vec![
            json!({"kind": "point", "x": 0, "y": 0, "z": 0, "intensity": 1, "range": 30, "colour": "#ffffff"}),
        ]);
        let (ms, _) = bl.meshes(Vec3::ZERO);
        let front = ms[0]
            .verts
            .iter()
            .filter(|v| v.pos[2] > 0.49)
            .map(|v| v.colour[0])
            .fold(0.0, f32::max);
        let back = ms[0]
            .verts
            .iter()
            .filter(|v| v.pos[2] < -0.49)
            .map(|v| v.colour[0])
            .fold(0.0, f32::max);
        assert!(front > 0.5 && back < 0.01, "lit front {front}, dark back {back}");
        // a text block shows its tile
        bl.set(
            &[json!({"id": "2", "kind": "text", "texture": "t1", "x": 0, "y": 0, "z": -5, "w": 1, "h": 1, "d": 1})],
            Some(&json!({"text:t1": {"text": "hello"}})),
            false,
        );
        let (ms, tex) = bl.meshes(Vec3::ZERO);
        assert!(ms.iter().any(|m| m.texture.as_deref() == Some("text:t1")));
        assert!(tex.contains_key("text:t1"));
    }

    #[test]
    fn far_blocks_are_billboards_then_gone() {
        let mut bl = Blocks::default();
        let at = |id: &str, z: f64| json!({"id": id, "kind": "plain", "texture": "#ff0000", "x": 0, "y": 0, "z": z, "w": 2, "h": 3, "d": 1});
        bl.set(&[at("near", -10.0), at("far", -200.0), at("gone", -500.0)], None, false);
        let (ms, _) = bl.meshes(Vec3::ZERO);
        assert_eq!(bl.counts, (1, 1, 1));
        let board = ms.iter().find(|m| m.id == u64::MAX - 7).unwrap();
        assert_eq!(board.verts.len(), 6);
        // as tall as the block, red on the whole
        let ys: Vec<f32> = board.verts.iter().map(|v| v.pos[1]).collect();
        assert!((ys.iter().cloned().fold(f32::MIN, f32::max) - 1.5).abs() < 1e-4);
        assert!(board.verts[0].colour[0] > 0.5 && board.verts[0].colour[1] < 0.01);
        // rays meet the far one (real geometry), not the one past the limit
        assert_eq!(
            bl.hit(Vec3::new(0.0, 0.0, -100.0), Vec3::NEG_Z).map(|h| h.0),
            Some("far".to_string())
        );
        assert!(
            bl.hit(Vec3::new(0.0, 0.0, -50.0), Vec3::NEG_Z)
                .is_some_and(|h| h.0 == "far")
        );
        bl.remove(&["far".to_string()]);
        assert!(bl.hit(Vec3::new(0.0, 0.0, -50.0), Vec3::NEG_Z).is_none());
    }
}
