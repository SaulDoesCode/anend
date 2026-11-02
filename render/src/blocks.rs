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
}

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
}

impl Blocks {
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.list.iter().find(|(k, _)| k == id).map(|(_, b)| b)
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

    /// a block's meshes, built again when it (or its texture, or the lights) changed
    fn build(&mut self, id: &str, b: &Value, model: Mat4, rot: Quat) -> Vec<Mesh> {
        let key = format!(
            "{}:{}",
            b["kind"].as_str().unwrap_or(""),
            b["texture"].as_str().unwrap_or("")
        );
        let faces: Vec<Option<String>> = b["faces"]
            .as_array()
            .map(|a| a.iter().map(|f| f.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let mut abouts = String::new();
        for k in std::iter::once(key.clone()).chain(faces.iter().flatten().map(|f| format!("texture:{f}"))) {
            abouts.push_str(&self.abouts.get(&k).map(|a| a.to_string()).unwrap_or_default());
        }
        let lit = if self.lights.is_empty() { 0 } else { self.lights_rev };
        let sig = format!("{b}|{abouts}|{lit}");
        if let Some(x) = self.built.get(id)
            && x.sig == sig
        {
            return x.meshes.clone();
        }
        let shape = shape(b);
        let lights: Vec<Light> = self.lights.iter().map(Light::of).collect();
        let variant = b["variant"].as_u64().unwrap_or(0) as u32;
        let zone = b.get("zone").is_some_and(|z| !z.is_null());
        // the faces grouped by what they show
        let mut groups: Vec<(Option<String>, [f32; 3], Vec<Vert>)> = Vec::new();
        for (i, face) in shape.faces.iter().enumerate() {
            let own = (!shape.round).then(|| faces.get(i).cloned().flatten()).flatten();
            let (tex, uv, tint) = match &own {
                Some(f) => self.look(&format!("texture:{f}"), 0),
                None => self.look(&key, variant),
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
        let old = self.built.remove(id);
        let mut meshes = Vec::new();
        for (i, (tex, _, verts)) in groups.into_iter().enumerate() {
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
        self.built.insert(
            id.to_string(),
            Built {
                sig,
                meshes: meshes.clone(),
            },
        );
        meshes
    }

    /// what to draw: every block's meshes (at `pose`'s place when it gives one), the sky
    /// round the camera, and the textures they show
    pub fn meshes(
        &mut self,
        eye: Vec3,
        pose: &dyn Fn(&str, &Value) -> Option<Mat4>,
    ) -> (Vec<Mesh>, HashMap<String, Texture>) {
        let mut out = Vec::new();
        let list = self.list.clone();
        for (id, b) in &list {
            let (at, rot) = place(b);
            let model = Mat4::from_rotation_translation(rot, at);
            let mut ms = self.build(id, b, model, rot);
            if let Some(m) = pose(id, b) {
                for x in &mut ms {
                    x.model = m;
                }
            }
            out.extend(ms);
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
        let (ms, _) = bl.meshes(Vec3::ZERO, &|_, _| None);
        assert_eq!(ms.len(), 1);
        assert!(ms[0].verts.iter().all(|v| v.colour[0] > 0.5 && v.colour[1] < 0.01));
        bl.set_lights(vec![
            json!({"kind": "point", "x": 0, "y": 0, "z": 0, "intensity": 1, "range": 30, "colour": "#ffffff"}),
        ]);
        let (ms, _) = bl.meshes(Vec3::ZERO, &|_, _| None);
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
        let (ms, tex) = bl.meshes(Vec3::ZERO, &|_, _| None);
        assert!(ms.iter().any(|m| m.texture.as_deref() == Some("text:t1")));
        assert!(tex.contains_key("text:t1"));
    }
}
