//! Wiring: what a block does when something happens to it (queued requests #15 and #28).
//! A block's wiring says which events it listens for (a click, a ray's hit, a text coming
//! into or leaving its zone, a timer, a named signal), which s program it runs then (a
//! note tagged `s`, by its title), and where its ray goes: in a fixed direction from its
//! centre, hitting what it crosses, or straight to chosen target blocks (sticky). A fixed
//! ray can start loose and become sticky on its first hit. Which of the blocks a ray
//! crosses it registers is its mode: the first (the default), the last, all of them in
//! order, or the nth; it can be limited to some kinds of block.
//!
//! A wiring is an entry of the save (mime `application/x-anend-wiring`, its text the
//! wiring's JSON), referred to from the block's placement ([`WireRef`], record `W`), so
//! copies of a block share it until one of them is changed. [`cast`] is the ray test,
//! against the blocks as they are kept (their boxes and spheres, turned; not their
//! bends, nor where an animation shows them).

use serde::{Deserialize, Serialize};

use super::blocks::{Block, Kind, Shape};

/// what a block can listen for
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Event {
    Click,
    Hit,
    Enter,
    Leave,
    Timer,
    Signal,
}

impl Event {
    pub fn name(self) -> &'static str {
        match self {
            Event::Click => "click",
            Event::Hit => "hit",
            Event::Enter => "enter",
            Event::Leave => "leave",
            Event::Timer => "timer",
            Event::Signal => "signal",
        }
    }

    pub fn parse(s: &str) -> Option<Event> {
        [
            Event::Click,
            Event::Hit,
            Event::Enter,
            Event::Leave,
            Event::Timer,
            Event::Signal,
        ]
        .into_iter()
        .find(|e| e.name() == s.trim())
    }
}

/// which of the blocks a ray crosses it registers
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    First,
    Last,
    All,
    Nth,
}

fn forward() -> [f32; 3] {
    [0.0, 0.0, 1.0]
}
fn reach() -> f32 {
    50.0
}
fn one() -> u32 {
    1
}

/// where a block's ray goes and what it registers
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ray {
    /// the direction in the block's own frame (out of its front, +z, by default)
    #[serde(default = "forward")]
    pub dir: [f32; 3],
    /// sticky: these blocks are hit whatever is between (their addresses); empty, the
    /// ray is fixed
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    /// a fixed ray that becomes sticky on what it first registers
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stick: bool,
    #[serde(default)]
    pub mode: Mode,
    /// for the nth mode, counted from 1
    #[serde(default = "one")]
    pub n: u32,
    /// how far it reaches
    #[serde(default = "reach")]
    pub range: f32,
    /// the kinds of block it sees (all when empty)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sees: Vec<Kind>,
}

impl Default for Ray {
    fn default() -> Ray {
        Ray {
            dir: forward(),
            targets: Vec::new(),
            stick: false,
            mode: Mode::First,
            n: 1,
            range: reach(),
            sees: Vec::new(),
        }
    }
}

/// a block's wiring
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Wiring {
    /// the events it listens for
    #[serde(default)]
    pub on: Vec<Event>,
    /// the timer's period in seconds (when it listens for the timer)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<f32>,
    /// the signal it listens for, by name
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signal: String,
    /// the s program it runs, by the title of its note (none: it only casts its ray)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub program: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ray: Option<Ray>,
    /// what the program keeps between runs: its stdin, set by its `keep` lines
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
}

/// the shortest timer period, in seconds
pub const MIN_EVERY: f32 = 0.2;

impl Wiring {
    /// within bounds, events once each and in order
    pub fn checked(mut self) -> Result<Wiring, String> {
        self.on.sort();
        self.on.dedup();
        self.every = self.every.filter(|e| e.is_finite()).map(|e| e.clamp(MIN_EVERY, 3600.0));
        if self.on.contains(&Event::Timer) && self.every.is_none() {
            self.every = Some(1.0);
        }
        self.signal = self.signal.trim().to_string();
        self.program = self.program.trim().to_string();
        if self.state.len() > 4096 {
            return Err("a wiring's state is at most 4096 bytes".into());
        }
        if let Some(r) = self.ray.as_mut() {
            let len = (r.dir[0] * r.dir[0] + r.dir[1] * r.dir[1] + r.dir[2] * r.dir[2]).sqrt();
            if !len.is_finite() || len < 1e-6 {
                return Err("a ray's direction can't be nothing".into());
            }
            r.dir = r.dir.map(|x| x / len);
            r.range = if r.range.is_finite() {
                r.range.clamp(0.1, 10_000.0)
            } else {
                reach()
            };
            r.n = r.n.clamp(1, 1000);
            r.targets.dedup();
            r.targets.truncate(64);
            r.sees.sort_by_key(|k| k.name());
            r.sees.dedup();
        }
        Ok(self)
    }
}

/// a block's reference to its wiring entry
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireRef {
    pub entry: u64,
}

impl WireRef {
    /// `W`, version 1, the entry (u64, little endian)
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&[b'W', 1]);
        out.extend_from_slice(&self.entry.to_le_bytes());
    }

    pub fn read(b: &[u8]) -> Option<(WireRef, usize)> {
        if b.len() < 10 || b[0] != b'W' || b[1] != 1 {
            return None;
        }
        Some((
            WireRef {
                entry: u64::from_le_bytes(b[2..10].try_into().ok()?),
            },
            10,
        ))
    }
}

impl Serialize for WireRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::record::label(self.entry))
    }
}

impl<'de> Deserialize<'de> for WireRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<WireRef, D::Error> {
        let s = String::deserialize(d)?;
        super::record::parse_label(&s)
            .map(|entry| WireRef { entry })
            .ok_or_else(|| serde::de::Error::custom(format!("{s:?} isn't a wiring's key")))
    }
}

/// `v` turned by the unit quaternion `q` (x, y, z, w)
pub fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    // t = 2 (q.xyz × v); v' = v + w t + q.xyz × t
    let t = [
        2.0 * (y * v[2] - z * v[1]),
        2.0 * (z * v[0] - x * v[2]),
        2.0 * (x * v[1] - y * v[0]),
    ];
    [
        v[0] + w * t[0] + (y * t[2] - z * t[1]),
        v[1] + w * t[1] + (z * t[0] - x * t[2]),
        v[2] + w * t[2] + (x * t[1] - y * t[0]),
    ]
}

fn conj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// where along the ray (`o` + t `d`, `d` a unit vector) it first meets the block, if it
/// does: 0 when it starts inside
pub fn meet(b: &Block, o: [f32; 3], d: [f32; 3]) -> Option<f32> {
    let t = &b.t;
    let qi = conj(t.rot);
    let rel = [o[0] - t.x, o[1] - t.y, o[2] - t.z];
    let p = rotate(qi, rel);
    let v = rotate(qi, d);
    let half = [t.w / 2.0, t.h / 2.0, t.d / 2.0];
    match t.shape {
        Shape::Sphere => {
            // the ellipsoid as a unit sphere
            let p = [0, 1, 2].map(|i| p[i] / half[i].max(1e-6));
            let v = [0, 1, 2].map(|i| v[i] / half[i].max(1e-6));
            let a = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
            let bq = 2.0 * (p[0] * v[0] + p[1] * v[1] + p[2] * v[2]);
            let c = p[0] * p[0] + p[1] * p[1] + p[2] * p[2] - 1.0;
            if c <= 0.0 {
                return Some(0.0);
            }
            let disc = bq * bq - 4.0 * a * c;
            if disc < 0.0 || a <= 0.0 {
                return None;
            }
            let t0 = (-bq - disc.sqrt()) / (2.0 * a);
            (t0 >= 0.0).then_some(t0)
        }
        _ => {
            let (mut lo, mut hi) = (0.0f32, f32::INFINITY);
            for i in 0..3 {
                if v[i].abs() < 1e-9 {
                    if p[i].abs() > half[i] {
                        return None;
                    }
                    continue;
                }
                let a = (-half[i] - p[i]) / v[i];
                let c = (half[i] - p[i]) / v[i];
                lo = lo.max(a.min(c));
                hi = hi.min(a.max(c));
                if lo > hi {
                    return None;
                }
            }
            Some(lo)
        }
    }
}

/// the blocks a fixed ray from `from` registers, nearest first, with how far along each is
/// (the block itself and those it doesn't see are passed through)
pub fn cast(from: &Block, ray: &Ray, blocks: &[Block]) -> Vec<(String, f32)> {
    let o = [from.t.x, from.t.y, from.t.z];
    let d = rotate(from.t.rot, ray.dir);
    let mut met: Vec<(String, f32)> = blocks
        .iter()
        .filter(|b| b.id != from.id && (ray.sees.is_empty() || ray.sees.contains(&b.kind)))
        .filter_map(|b| meet(b, o, d).filter(|t| *t <= ray.range).map(|t| (b.id.clone(), t)))
        .collect();
    met.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    match ray.mode {
        Mode::First => met.truncate(1),
        Mode::Last => {
            let last = met.pop();
            met = last.into_iter().collect();
        }
        Mode::All => {}
        Mode::Nth => {
            let n = ray.n.max(1) as usize - 1;
            met = met.into_iter().nth(n).into_iter().collect();
        }
    }
    met
}

/// where a ray from `from` ends: at its last registered block, or its full range
pub fn ray_end(from: &Block, ray: &Ray, hits: &[(String, f32)]) -> [f32; 3] {
    let d = rotate(from.t.rot, ray.dir);
    let t = hits.last().map_or(ray.range, |h| h.1);
    [from.t.x + d[0] * t, from.t.y + d[1] * t, from.t.z + d[2] * t]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::blocks::Transform;

    fn block(id: &str, x: f32, z: f32) -> Block {
        Block {
            id: id.into(),
            kind: Kind::Plain,
            texture: "#888888".into(),
            t: Transform {
                x,
                z,
                ..Transform::default()
            },
        }
    }

    #[test]
    fn rays_register_by_mode() {
        let from = block("plain:#888888#1", 0.0, 0.0);
        let row = vec![
            from.clone(),
            block("plain:#888888#2", 0.0, 3.0),
            block("plain:#888888#3", 0.0, 6.0),
            block("plain:#888888#4", 0.0, 9.0),
            block("plain:#888888#5", 5.0, 3.0),
        ];
        let ids = |r: &Ray| cast(&from, r, &row).into_iter().map(|h| h.0).collect::<Vec<_>>();
        let mut r = Ray::default();
        assert_eq!(ids(&r), ["plain:#888888#2"]);
        assert!((cast(&from, &r, &row)[0].1 - 2.5).abs() < 1e-5);
        r.mode = Mode::Last;
        assert_eq!(ids(&r), ["plain:#888888#4"]);
        r.mode = Mode::All;
        assert_eq!(ids(&r), ["plain:#888888#2", "plain:#888888#3", "plain:#888888#4"]);
        r.mode = Mode::Nth;
        r.n = 2;
        assert_eq!(ids(&r), ["plain:#888888#3"]);
        r.range = 4.0;
        assert!(ids(&r).is_empty());
        // a quarter turn about y points +z at +x
        let mut turned = from.clone();
        let s = std::f32::consts::FRAC_1_SQRT_2;
        turned.t.rot = [0.0, s, 0.0, s];
        let r = Ray::default();
        let h = cast(&turned, &r, &row);
        assert!(h.is_empty(), "nothing at +x on the row's line: {h:?}");
        // between the row and the fifth: the ray goes +x, away from the row
        (turned.t.x, turned.t.z) = (2.0, 3.0);
        let h = cast(&turned, &r, &row);
        assert_eq!(h[0].0, "plain:#888888#5");
        assert!((h[0].1 - 2.5).abs() < 1e-5);
        // the kinds it sees
        let r = Ray {
            sees: vec![Kind::Text],
            ..Ray::default()
        };
        assert!(ids(&r).is_empty());
    }

    #[test]
    fn spheres_are_round() {
        let from = block("plain:#888888#1", 0.0, 0.0);
        let mut ball = block("plain:#888888#2", 0.45, 3.0);
        ball.t.shape = Shape::Sphere;
        // a box there would be met; the ball's edge is at x 0.45 - 0.5, so the axis grazes it
        assert!(meet(&ball, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]).is_some());
        ball.t.x = 0.6;
        assert!(meet(&ball, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]).is_none());
        let mut cube = ball.clone();
        cube.t.shape = Shape::Box;
        cube.t.x = 0.45;
        let r = Ray::default();
        assert_eq!(cast(&from, &r, &[cube]).len(), 1);
    }

    #[test]
    fn wiring_is_checked() {
        let w: Wiring =
            serde_json::from_str(r#"{"on":["timer","click","click"],"ray":{"dir":[0,0,2],"mode":"nth","n":0}}"#)
                .unwrap();
        let w = w.checked().unwrap();
        assert_eq!(w.on, [Event::Click, Event::Timer]);
        assert_eq!(w.every, Some(1.0));
        let r = w.ray.unwrap();
        assert_eq!((r.dir, r.n, r.range), ([0.0, 0.0, 1.0], 1, 50.0));
        let bad: Wiring = serde_json::from_str(r#"{"ray":{"dir":[0,0,0]}}"#).unwrap();
        assert!(bad.checked().is_err());
        let mut b = Vec::new();
        WireRef { entry: 77 }.write(&mut b);
        assert_eq!(WireRef::read(&b), Some((WireRef { entry: 77 }, 10)));
    }
}
