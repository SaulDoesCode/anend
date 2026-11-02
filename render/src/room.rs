//! The room, in Rust (the renderer line: space.js ported, so the desktop runs it with no
//! JavaScript). The same protocol as space.js: Rust's commands come in as JSON (`command`),
//! what happens goes out as the same events (`take_events`), so the app's handlers, windows
//! and menus are unchanged. The pointer and keys come in through `pointer_*`, `wheel` and
//! `key_*`; `frame` moves the room on; `scene` says what to draw.
//!
//! The spread: every two texts repel, and attract as much as they're alike (a weighted sum
//! of what the physics panel weighs); a weak gravity keeps the cloud together, labels never
//! overlap, and the integration runs in fixed steps with damping and cooling.

use std::collections::{HashMap, HashSet};

use glam::{Quat, Vec3};
use serde_json::{Map, Value, json};

use crate::text;

/// the em the labels are measured at (the webview's labels were 32px)
pub const BASE_PX: f32 = 32.0;
const LOOK: f32 = 0.0026;
const ROLL: f32 = 1.4;
pub const STEP: f32 = 1.0 / 60.0;

const TYPES: [&str; 17] = [
    "word",
    "name",
    "number",
    "date",
    "time",
    "link",
    "email",
    "hashtag",
    "noun",
    "verb",
    "adjective",
    "adverb",
    "pronoun",
    "determiner",
    "preposition",
    "conjunction",
    "interjection",
];

fn default_keys() -> HashMap<String, String> {
    [
        ("fly_forward", "KeyW"),
        ("fly_back", "KeyS"),
        ("fly_left", "KeyA"),
        ("fly_right", "KeyD"),
        ("fly_up", "Space"),
        ("fly_down", "KeyC"),
        ("roll_left", "KeyQ"),
        ("roll_right", "KeyE"),
        ("level", "KeyR"),
        ("back_to_start", "Home"),
        ("slower", "BracketLeft"),
        ("faster", "BracketRight"),
        ("walk", "KeyV"),
        ("superflight", "KeyB"),
        ("grapple", "KeyG"),
        ("sneak", "KeyZ"),
    ]
    .into_iter()
    .map(|(a, k)| (a.to_string(), k.to_string()))
    .collect()
}

const FLYING: [&str; 9] = [
    "fly_forward",
    "fly_back",
    "fly_left",
    "fly_right",
    "fly_up",
    "fly_down",
    "roll_left",
    "roll_right",
    "sneak",
];

/// the physics' settings (the panel's sliders), as space.js's PHYSICS
#[derive(Clone, Debug, PartialEq)]
pub struct Physics {
    pub v: Map<String, Value>,
}

impl Physics {
    fn defaults() -> Map<String, Value> {
        match json!({
            "damping": 2.2, "gravity": 0.06, "repel": 9, "link": 4, "attract": 0.9, "pad": 0.3, "cool": 0.012, "substeps": 2,
            "size": 0.5, "wrap": 60, "minPx": 1.2, "depth": 0, "flat": 1.5, "qscale": 12, "distance": 26,
            "w_ordinal": 0.6, "w_reversed": 0.3, "w_bucket": 0.8, "w_length": 0.2, "w_types": 0.3, "w_lists": 0.7, "w_class": 0.5, "w_time": 0.2, "w_letters": 0.4,
            "depth_by": "none", "reversed": 0, "look": 1, "smooth": 14,
        }) {
            Value::Object(m) => m,
            _ => Map::new(),
        }
    }

    fn new(over: Option<&Value>) -> Physics {
        let mut v = Self::defaults();
        if let Some(Value::Object(o)) = over {
            for (k, x) in o {
                v.insert(k.clone(), x.clone());
            }
        }
        Physics { v }
    }

    pub fn f(&self, k: &str) -> f32 {
        self.v.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32
    }

    fn s(&self, k: &str) -> &str {
        self.v.get(k).and_then(Value::as_str).unwrap_or("")
    }
}

fn hash01(s: &str, salt: u32) -> f32 {
    let mut h: u32 = 2166136261 ^ salt;
    for c in s.encode_utf16() {
        h = (h ^ c as u32).wrapping_mul(16777619);
    }
    (h % 100000) as f32 / 100000.0
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut d, mut na, mut nb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        d += x * y;
        na += x * x;
        nb += y * y;
    }
    if na > 0.0 && nb > 0.0 {
        d / (na * nb).sqrt()
    } else {
        0.0
    }
}

fn fnum(f: &Map<String, Value>, k: &str) -> f32 {
    f.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

fn fstr<'a>(f: &'a Map<String, Value>, k: &str) -> &'a str {
    f.get(k).and_then(Value::as_str).unwrap_or("")
}

fn key_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

/// a CSS colour as sRGB, 0–1
pub fn css_rgb(css: &str) -> Option<[f32; 3]> {
    let c = css.trim().to_lowercase();
    if let Some(h) = c.strip_prefix('#') {
        let v: Vec<u8> = h.chars().filter_map(|x| x.to_digit(16).map(|d| d as u8)).collect();
        return match v.len() {
            3 | 4 => Some([
                v[0] as f32 * 17.0 / 255.0,
                v[1] as f32 * 17.0 / 255.0,
                v[2] as f32 * 17.0 / 255.0,
            ]),
            6 | 8 => Some([
                (v[0] * 16 + v[1]) as f32 / 255.0,
                (v[2] * 16 + v[3]) as f32 / 255.0,
                (v[4] * 16 + v[5]) as f32 / 255.0,
            ]),
            _ => None,
        };
    }
    let args = |name: &str| -> Option<Vec<f32>> {
        let inner = c
            .strip_prefix(name)?
            .trim_start_matches('a')
            .strip_prefix('(')?
            .strip_suffix(')')?;
        Some(
            inner
                .split([',', ' ', '/'])
                .filter(|x| !x.is_empty())
                .map(|x| x.trim_end_matches(['%', 'g', 'd', 'e']).parse().unwrap_or(0.0))
                .collect(),
        )
    };
    if let Some(a) = args("rgb") {
        return (a.len() >= 3).then(|| [a[0] / 255.0, a[1] / 255.0, a[2] / 255.0]);
    }
    if let Some(a) = args("hsl") {
        if a.len() < 3 {
            return None;
        }
        let (h, s, l) = (a[0].rem_euclid(360.0) / 360.0, a[1] / 100.0, a[2] / 100.0);
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        let f = |mut t: f32| {
            t = t.rem_euclid(1.0);
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        return Some([f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]);
    }
    Some(match c.as_str() {
        "white" => [1.0, 1.0, 1.0],
        "black" => [0.0, 0.0, 0.0],
        "red" => [1.0, 0.0, 0.0],
        "green" => [0.0, 0.5, 0.0],
        "blue" => [0.0, 0.0, 1.0],
        "gray" | "grey" => [0.5, 0.5, 0.5],
        "orange" => [1.0, 0.65, 0.0],
        "yellow" => [1.0, 1.0, 0.0],
        "purple" => [0.5, 0.0, 0.5],
        _ => return None,
    })
}

/// a text's placement in the room
#[derive(Clone, Debug)]
pub struct Node {
    pub key: String,
    /// what Rust sent about the text (key, text, note, sums, kind, lists, …)
    pub f: Map<String, Value>,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    vx: f32,
    vy: f32,
    vz: f32,
    pub hw: f32,
    pub hh: f32,
    pub pinned: bool,
    pub free: bool,
    /// a detached placement: the entry's version it keeps
    pub det: u64,
    /// an inheriting placement's own text
    pub own: Option<String>,
    /// its own font size (units an em), else the room's
    pub size: Option<f32>,
    fx_size: f32,
    /// a chain member's head (its node key) and place in the chain
    pub head: Option<String>,
    pub idx: i64,
    pub links: Vec<String>,
    zone: Option<String>,
    /// a colour from Rust (`colours`)
    colour: Option<String>,
    picked: bool,
    /// where it was drawn: its middle and half sizes in pixels, and its depth
    screen: Option<(f32, f32, f32, f32, f32)>,
}

impl Node {
    fn shown(&self) -> String {
        let t = match &self.own {
            Some(o) => o.clone(),
            None if self.det > 0 => fstr(&self.f, "text").to_string(),
            None => self
                .f
                .get("shown")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(fstr(&self.f, "text"))
                .to_string(),
        };
        let mut s = String::new();
        if self.det > 0 {
            s.push('◇');
        } else if self.own.is_some() {
            s.push('◆');
        }
        if self.f.get("note").and_then(Value::as_bool) == Some(true)
            && self.f.get("virt").and_then(Value::as_bool) != Some(true)
        {
            s.push_str("✎ ");
        }
        s.push_str(&t);
        if self.pinned {
            s.push('•');
        }
        s
    }

    fn entry(&self) -> String {
        key_text(self.f.get("key").unwrap_or(&Value::Null))
    }
}

/// a step Ctrl+Z takes back (the texts'; the rest are passed back to Rust as space.js did)
#[derive(Clone, Debug)]
pub enum Undo {
    Move {
        n: String,
        x: f32,
        y: f32,
        z: f32,
        pinned: bool,
    },
    Pin {
        n: String,
        was: bool,
    },
    Remove {
        n: Box<Node>,
    },
    Add {
        n: String,
    },
    Text {
        n: String,
        from: String,
    },
    Size {
        n: String,
        from: Option<f32>,
    },
    Own {
        n: String,
        from: Option<String>,
    },
    /// a step Rust undoes when told (its event and data)
    Event {
        kind: String,
        data: Value,
    },
    /// several made together, the last first
    Many(Vec<Undo>),
}

/// the camera easing onto a text (the arrows' hop)
#[derive(Clone, Copy, Debug)]
struct Ease {
    from: Vec3,
    to: Vec3,
    from_q: Quat,
    to_q: Quat,
    t: f32,
}

/// a text dragged with the pointer
#[derive(Clone, Debug)]
struct Drag {
    n: String,
    el: String,
    x0: f32,
    y0: f32,
    moved: bool,
    depth: f32,
    from: (f32, f32, f32, bool),
    off: Vec3,
}

/// the modifier keys held with a pointer or key event
#[derive(Clone, Copy, Debug, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

pub struct Room {
    pub nodes: HashMap<String, Node>,
    pub order: Vec<String>,
    sim: Option<Vec<f32>>,
    held: HashSet<String>,
    bind: HashMap<String, String>,
    pub p: Physics,
    pub speed: f32,
    pub at: Vec3,
    yaw: f32,
    pitch: f32,
    roll_a: f32,
    tyaw: f32,
    tpitch: f32,
    running: bool,
    looking: bool,
    pub paused: bool,
    alpha: f32,
    acc: f32,
    energy: f32,
    mean: (f32, f32, f32, f32, f32),
    ease: Option<Ease>,
    pub selected: Option<String>,
    hover: Option<String>,
    drag: Option<Drag>,
    undo: Vec<Undo>,
    batch: Option<Vec<Undo>>,
    free_n: u32,
    origin: Vec3,
    quat: Quat,
    pub room: String,
    pub ink: String,
    pub size: (f32, f32),
    fx: Map<String, Value>,
    events: Vec<Value>,
    last_info: String,
    info_t: f32,
    clock: f32,
    last_click: Option<(String, f32)>,
    last_empty: f32,
    rdown: Option<(f32, f32)>,
    pointer: (f32, f32),
    wheel_undo: Option<(String, f32)>,
    /// the blocks as Rust sent them (drawn; the block slice moves their editing over)
    pub blocks: Vec<(String, Value)>,
    /// whether anything changed since the last picture
    pub dirty: bool,
}

impl Default for Room {
    fn default() -> Self {
        Room::new()
    }
}

impl Room {
    pub fn new() -> Room {
        let mut r = Room {
            nodes: HashMap::new(),
            order: Vec::new(),
            sim: None,
            held: HashSet::new(),
            bind: default_keys(),
            p: Physics::new(None),
            speed: 6.0,
            at: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            roll_a: 0.0,
            tyaw: 0.0,
            tpitch: 0.0,
            running: false,
            looking: false,
            paused: false,
            alpha: 0.0,
            acc: 0.0,
            energy: 0.0,
            mean: (0.0, 1.0, 0.0, 0.0, 1.0),
            ease: None,
            selected: None,
            hover: None,
            drag: None,
            undo: Vec::new(),
            batch: None,
            free_n: 0,
            origin: Vec3::ZERO,
            quat: Quat::IDENTITY,
            room: "#e4d4be".into(),
            ink: "#222222".into(),
            size: (1.0, 1.0),
            fx: Map::new(),
            events: Vec::new(),
            last_info: String::new(),
            info_t: -1.0,
            clock: 0.0,
            last_click: None,
            last_empty: -10.0,
            rdown: None,
            pointer: (0.0, 0.0),
            wheel_undo: None,
            blocks: Vec::new(),
            dirty: true,
        };
        r.set_frame();
        r
    }

    // ------------------------------------------------------------------ the camera

    pub fn camera_rot(&self) -> Quat {
        Quat::from_euler(glam::EulerRot::YXZ, self.yaw, self.pitch, self.roll_a)
    }

    pub fn camera(&self) -> crate::Camera {
        crate::Camera {
            at: self.at,
            yaw: self.yaw,
            pitch: self.pitch,
            roll: self.roll_a,
            fov: 50f32.to_radians(),
        }
    }

    fn set_frame(&mut self) {
        self.quat = self.camera_rot();
        self.origin = self.at + self.quat * Vec3::new(0.0, 0.0, -self.p.f("distance"));
    }

    pub fn world(&self, n: &Node) -> Vec3 {
        self.quat * Vec3::new(n.x, n.y, n.z) + self.origin
    }

    fn local(&self, w: Vec3) -> Vec3 {
        self.quat.inverse() * (w - self.origin)
    }

    fn set_local(&mut self, key: &str, w: Vec3) {
        let v = self.local(w);
        if let Some(n) = self.nodes.get_mut(key) {
            (n.x, n.y, n.z) = (v.x, v.y, v.z);
        }
    }

    /// a point of the picture as a direction from the camera, and the point at a depth
    fn ray_dir(&self, x: f32, y: f32) -> Vec3 {
        let (w, h) = self.size;
        let t = (25f32).to_radians().tan();
        let nx = (x / w) * 2.0 - 1.0;
        let ny = 1.0 - (y / h) * 2.0;
        (self.camera_rot() * Vec3::new(nx * t * w / h, ny * t, -1.0)).normalize()
    }

    fn ray_at(&self, x: f32, y: f32, depth: f32) -> Vec3 {
        let d = self.ray_dir(x, y);
        let fwd = self.camera_rot() * Vec3::NEG_Z;
        self.at + d * (depth / d.dot(fwd).max(1e-4))
    }

    fn roll_deg(&self) -> f32 {
        self.roll_a.to_degrees()
    }

    pub fn home(&mut self) {
        self.at = Vec3::ZERO;
        (self.yaw, self.tyaw, self.pitch, self.tpitch, self.roll_a) = (0.0, 0.0, 0.0, 0.0, 0.0);
        self.ease = None;
        self.dirty = true;
    }

    pub fn level(&mut self) {
        self.roll_a = 0.0;
        self.dirty = true;
    }

    fn sync_turn(&mut self, q: Quat) {
        let (y, x, z) = q.to_euler(glam::EulerRot::YXZ);
        self.yaw = y;
        self.tyaw = y;
        self.pitch = x.clamp(-1.55, 1.55);
        self.tpitch = self.pitch;
        self.roll_a = z;
    }

    // ------------------------------------------------------------------ events

    fn ev(&self, kind: &str, n: Option<&str>, at: Option<(f32, f32, Mods)>, data: Value) -> Value {
        let n = n.and_then(|k| self.nodes.get(k));
        let same: Vec<&String> = match n {
            Some(n) if !n.free => self.placements(&n.entry()),
            _ => vec![],
        };
        let (x, y, m) = at.unwrap_or((0.0, 0.0, Mods::default()));
        json!({
            "kind": kind,
            "key": n.map(|n| if n.free { n.key.clone() } else { n.entry() }).unwrap_or_default(),
            "node": n.map(|n| n.key.clone()).unwrap_or_default(),
            "copies": same.len(),
            "primary": n.is_some_and(|n| !n.free && same.first().is_some_and(|k| **k == n.key)),
            "text": n.map(|n| n.own.clone().unwrap_or_else(|| fstr(&n.f, "text").to_string())).unwrap_or_default(),
            "note": n.is_some_and(|n| n.f.get("note").and_then(Value::as_bool) == Some(true)),
            "free": n.is_some_and(|n| n.free),
            "det": n.map(|n| n.det).unwrap_or(0),
            "chain": n.map(|n| match &n.head {
                Some(h) => self.nodes.get(h).map(|h| h.entry()).unwrap_or_default(),
                None if n.f.get("isChain").and_then(Value::as_bool) == Some(true) => n.entry(),
                None => String::new(),
            }).unwrap_or_default(),
            "idx": n.map(|n| if n.head.is_some() { n.idx } else { -1 }).unwrap_or(-1),
            "members": n.map(|n| n.links.len()).unwrap_or(0),
            "virt": n.is_some_and(|n| n.f.get("virt").and_then(Value::as_bool) == Some(true)),
            "head_node": n.and_then(|n| n.head.clone()).unwrap_or_default(),
            "font": n.and_then(|n| n.size).map(|s| (s * 50.0).round() as i64).unwrap_or(0),
            "font_default": (self.p.f("size") * 50.0).round() as i64,
            "own": n.and_then(|n| n.own.clone()),
            "entry_text": n.map(|n| fstr(&n.f, "text").to_string()).unwrap_or_default(),
            "x": x, "y": y, "ctrl": m.ctrl, "shift": m.shift, "alt": m.alt,
            "q": n.map(|n| fnum(&n.f, "q")).unwrap_or(0.0),
            "hue": n.map(|n| fnum(&n.f, "hue")).unwrap_or(0.0),
            "pinned": n.is_some_and(|n| n.pinned),
            "data": if data.is_null() { json!("") } else { data },
        })
    }

    fn send(&mut self, v: Value) {
        self.events.push(v);
    }

    fn send_ev(&mut self, kind: &str, n: Option<&str>, at: Option<(f32, f32, Mods)>, data: Value) {
        let e = self.ev(kind, n, at, data);
        self.send(e);
    }

    pub fn take_events(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.events)
    }

    fn toast(&mut self, text: &str) {
        let mut e = self.ev("toast", None, None, Value::Null);
        e["text"] = json!(text);
        self.send(e);
    }

    fn info(&mut self, now: bool) {
        if !now && self.clock - self.info_t < 0.12 {
            return;
        }
        self.info_t = self.clock;
        let state = if self.paused {
            "paused"
        } else if self.asleep() {
            "still"
        } else {
            "moving"
        };
        let text = format!(
            "{:.1} {:.1} {:.1}   speed {:.1}   roll {}°   {} texts   spread {state}",
            self.at.x,
            self.at.y,
            -self.at.z,
            self.speed,
            self.roll_deg().round() as i64,
            self.order.len()
        );
        if text == self.last_info {
            return;
        }
        self.last_info = text.clone();
        let mut e = self.ev("info", None, None, Value::Null);
        e["text"] = json!(text);
        self.send(e);
    }

    fn placements(&self, entry: &str) -> Vec<&String> {
        self.order
            .iter()
            .filter(|k| {
                self.nodes
                    .get(*k)
                    .is_some_and(|n| !n.free && n.head.is_none() && n.entry() == entry)
            })
            .collect()
    }

    // ------------------------------------------------------------------ texts

    fn measure(&mut self, key: &str) {
        let wrap = self.p.f("wrap");
        let size0 = self.p.f("size");
        let Some(n) = self.nodes.get_mut(key) else { return };
        let wrap_em = if n.free { 80.0 } else { wrap } * text::ch();
        let lay = text::layout(&n.shown(), wrap_em);
        let size = n.size.unwrap_or(size0) * n.fx_size;
        n.hw = lay.w / 2.0 * size;
        n.hh = lay.h / 2.0 * size;
    }

    fn make(
        &mut self,
        t: Map<String, Value>,
        free: bool,
        node_key: Option<String>,
        det: u64,
        own: Option<String>,
    ) -> String {
        let key = node_key.unwrap_or_else(|| key_text(t.get("key").unwrap_or(&Value::Null)));
        let chain = t.get("chain").cloned();
        let n = Node {
            key: key.clone(),
            f: t,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            hw: 1.0,
            hh: 0.4,
            pinned: false,
            free,
            det,
            own,
            size: None,
            fx_size: 1.0,
            head: None,
            idx: -1,
            links: vec![],
            zone: None,
            colour: None,
            picked: false,
            screen: None,
        };
        self.nodes.insert(key.clone(), n);
        self.order.push(key.clone());
        self.measure(&key);
        self.link(&key, chain.as_ref());
        key
    }

    fn link(&mut self, key: &str, members: Option<&Value>) {
        let old = self.nodes.get(key).map(|n| n.links.clone()).unwrap_or_default();
        for c in old {
            self.drop_node(&c);
        }
        if let Some(n) = self.nodes.get_mut(key) {
            n.links.clear();
        }
        let Some(Value::Array(ms)) = members else { return };
        let (hx, hy, hz) = self.nodes.get(key).map(|n| (n.x, n.y, n.z)).unwrap_or_default();
        for (i, m) in ms.iter().enumerate() {
            let mut f = Map::new();
            f.insert("key".into(), json!(key_text(&m["key"])));
            f.insert("text".into(), m["text"].clone());
            f.insert("note".into(), json!(m["note"] == true));
            f.insert("virt".into(), json!(m["virt"] == true));
            let ck = self.make(f, false, Some(format!("{key}>{i}>{}", key_text(&m["key"]))), 0, None);
            if let Some(c) = self.nodes.get_mut(&ck) {
                c.head = Some(key.to_string());
                c.idx = i as i64;
                (c.x, c.y, c.z) = (hx, hy - (i as f32 + 1.0) * 0.9, hz);
            }
            if let Some(n) = self.nodes.get_mut(key) {
                n.links.push(ck);
            }
        }
        self.sim = None;
    }

    fn drop_node(&mut self, key: &str) {
        let links = self.nodes.get(key).map(|n| n.links.clone()).unwrap_or_default();
        for c in links {
            if c != key {
                self.drop_node(&c);
            }
        }
        self.nodes.remove(key);
        self.order.retain(|k| k != key);
        if self.selected.as_deref() == Some(key) {
            self.selected = None;
        }
        if self.hover.as_deref() == Some(key) {
            self.hover = None;
        }
        self.sim = None;
        self.dirty = true;
    }

    fn seed(&mut self, key: &str) {
        let r0 = 2.0 + (self.order.len() as f32).sqrt() * 1.4;
        if let Some(n) = self.nodes.get_mut(key) {
            let a = hash01(key, 1) * std::f32::consts::TAU;
            let r = r0 * hash01(key, 2).sqrt();
            n.x = a.cos() * r;
            n.y = a.sin() * r;
            n.z = (hash01(key, 3) - 0.5) * 0.5;
        }
    }

    fn dress(&mut self, key: &str) {
        let e = self
            .nodes
            .get(key)
            .filter(|n| !n.free)
            .and_then(|n| self.fx.get(&n.entry()).cloned());
        if let Some(n) = self.nodes.get_mut(key) {
            let g = e.as_ref().and_then(|e| e.get("gather")).cloned().unwrap_or(Value::Null);
            n.f.insert("gather".into(), g);
            n.fx_size = e
                .as_ref()
                .and_then(|e| e.get("size"))
                .and_then(Value::as_f64)
                .unwrap_or(1.0) as f32;
        }
        self.measure(key);
    }

    pub fn place(&mut self, texts: &[Value], again: bool) -> usize {
        if !again {
            self.set_frame();
            let gone: Vec<String> = self
                .order
                .iter()
                .filter(|k| self.nodes.get(*k).is_some_and(|n| !n.pinned && !n.free))
                .cloned()
                .collect();
            for k in gone {
                self.drop_node(&k);
            }
        }
        let before = self.order.len();
        for t in texts {
            let Value::Object(t) = t else { continue };
            let key = key_text(t.get("key").unwrap_or(&Value::Null));
            if self.nodes.contains_key(&key) {
                let (kept, det, was) = {
                    let n = &self.nodes[&key];
                    let was: Vec<String> = n
                        .links
                        .iter()
                        .filter_map(|c| self.nodes.get(c))
                        .map(|c| format!("{}|{}", c.entry(), fstr(&c.f, "text")))
                        .collect();
                    (fstr(&n.f, "text").to_string(), n.det, was.join("\n"))
                };
                if let Some(n) = self.nodes.get_mut(&key) {
                    for (k, v) in t {
                        n.f.insert(k.clone(), v.clone());
                    }
                    if det > 0 {
                        n.f.insert("text".into(), json!(kept));
                    }
                }
                let now: Vec<String> = t
                    .get("chain")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|m| format!("{}|{}", key_text(&m["key"]), m["text"].as_str().unwrap_or("")))
                            .collect()
                    })
                    .unwrap_or_default();
                if was != now.join("\n") {
                    let chain = t.get("chain").cloned();
                    self.link(&key, chain.as_ref());
                }
                self.dress(&key);
                continue;
            }
            let k = self.make(t.clone(), false, None, 0, None);
            self.dress(&k);
            self.seed(&k);
        }
        self.sim = None;
        self.wake(1.0);
        self.info(true);
        self.order.len() - before
    }

    pub fn effects(&mut self, fx: Option<&Value>) {
        self.fx = fx.and_then(Value::as_object).cloned().unwrap_or_default();
        for k in self.order.clone() {
            self.dress(&k);
        }
        self.sim = None;
        self.wake(1.0);
    }

    pub fn clear(&mut self, all: bool) {
        let gone: Vec<String> = self
            .order
            .iter()
            .filter(|k| self.nodes.get(*k).is_some_and(|n| all || (!n.pinned && !n.free)))
            .cloned()
            .collect();
        for k in gone {
            self.drop_node(&k);
        }
        self.undo.clear();
        self.info(true);
    }

    fn mark(&mut self, list: Option<&str>, candidate: Option<&str>) {
        for n in self.nodes.values_mut() {
            n.picked = Some(n.key.as_str()) == list || Some(n.key.as_str()) == candidate;
        }
        self.dirty = true;
    }

    fn colours(&mut self, map: &Value) {
        for n in self.nodes.values_mut() {
            n.colour = map.get(&n.key).and_then(Value::as_str).map(String::from);
        }
        self.dirty = true;
    }

    fn copy_key(&self, entry: &str) -> String {
        let mut i = 2;
        while self.nodes.contains_key(&format!("{entry}~{i}")) {
            i += 1;
        }
        format!("{entry}~{i}")
    }

    fn set_text(&mut self, key: &str, text: &str) {
        if let Some(n) = self.nodes.get_mut(key) {
            n.f.insert("text".into(), json!(text));
            n.f.insert("shown".into(), Value::Null);
        }
        self.measure(key);
        self.wake(0.3);
    }

    fn set_chain(&mut self, key: &str, chain: Option<&Value>) {
        for k in self.order.clone() {
            let Some(n) = self.nodes.get(&k) else { continue };
            if n.head.is_some() || n.entry() != key {
                continue;
            }
            match chain {
                None | Some(Value::Null) => {
                    let links = n.links.clone();
                    if let Some(n) = self.nodes.get_mut(&k) {
                        let title =
                            n.f.get("titleKey")
                                .cloned()
                                .unwrap_or_else(|| n.f.get("key").cloned().unwrap_or(Value::Null));
                        n.f.insert("key".into(), title);
                        n.f.insert("isChain".into(), json!(false));
                        n.links.clear();
                    }
                    for c in links {
                        if let Some(c) = self.nodes.get_mut(&c) {
                            c.head = None;
                            c.pinned = true;
                        }
                    }
                }
                Some(ch) => {
                    if let Some(n) = self.nodes.get_mut(&k) {
                        n.f.insert("text".into(), ch["title"]["text"].clone());
                        n.f.insert("titleKey".into(), ch["title"]["key"].clone());
                        n.f.insert("isChain".into(), json!(true));
                    }
                    self.measure(&k);
                    let ms = ch["members"].clone();
                    self.link(&k, Some(&ms));
                }
            }
        }
        self.wake(0.5);
    }

    // ------------------------------------------------------------------ the spread

    pub fn wake(&mut self, a: f32) {
        self.dirty = true;
        self.alpha = self.alpha.max(a);
    }

    fn alike(&self, a: &Node, b: &Node) -> f32 {
        let p = &self.p;
        let (fa, fb) = (&a.f, &b.f);
        let mut sum = 0.0;
        let mut w = 0.0;
        let mut add = |weight: f32, m: Option<f32>| {
            if let Some(m) = m
                && weight != 0.0
                && m.is_finite()
            {
                sum += weight * m;
                w += weight.abs();
            }
        };
        let q = p.f("qscale").max(1e-3);
        add(
            p.f("w_ordinal"),
            Some((-(fnum(fa, "s") - fnum(fb, "s")).abs() / q).exp()),
        );
        add(
            p.f("w_reversed"),
            Some((-(fnum(fa, "r") - fnum(fb, "r")).abs() / q).exp()),
        );
        add(
            p.f("w_bucket"),
            Some(if fa.get("q") == fb.get("q") { 1.0 } else { 0.0 }),
        );
        let chars = |f: &Map<String, Value>| fnum(f, "chars").max(1.0);
        add(
            p.f("w_length"),
            Some((-(chars(fa).ln_1p() - chars(fb).ln_1p()).abs() * 1.5).exp()),
        );
        if let (Some(ta), Some(tb)) = (
            fa.get("types").and_then(Value::as_object),
            fb.get("types").and_then(Value::as_object),
        ) {
            let va: Vec<f32> = TYPES
                .iter()
                .map(|t| ta.get(*t).and_then(Value::as_f64).unwrap_or(0.0) as f32)
                .collect();
            let vb: Vec<f32> = TYPES
                .iter()
                .map(|t| tb.get(*t).and_then(Value::as_f64).unwrap_or(0.0) as f32)
                .collect();
            add(
                p.f("w_types"),
                Some(0.5 * cosine(&va, &vb) + 0.5 * if fa.get("kind") == fb.get("kind") { 1.0 } else { 0.0 }),
            );
        }
        let la: Vec<&Value> = fa
            .get("lists")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        let lb: Vec<&Value> = fb
            .get("lists")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        if !la.is_empty() || !lb.is_empty() {
            let shared = la.iter().filter(|x| lb.contains(x)).count() as f32;
            add(
                p.f("w_lists"),
                Some(shared / (la.len() as f32 + lb.len() as f32 - shared)),
            );
        } else {
            add(p.f("w_lists"), Some(0.0));
        }
        let cls = |f: &Map<String, Value>| f.get("cls").filter(|v| !v.is_null()).cloned();
        if let (Some(ca), Some(cb)) = (cls(fa), cls(fb)) {
            let st = |f: &Map<String, Value>| f.get("strength").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            add(p.f("w_class"), Some(if ca == cb { st(fa).min(st(fb)) } else { 0.0 }));
        }
        let ga = fa.get("gather").filter(|v| !v.is_null());
        let gb = fb.get("gather").filter(|v| !v.is_null());
        if ga.is_some() || gb.is_some() {
            add(2.0, Some(if ga == gb { 1.0 } else { 0.0 }));
        }
        let (ca, cb) = (fnum(fa, "created"), fnum(fb, "created"));
        if ca > 0.0 && cb > 0.0 {
            add(p.f("w_time"), Some((-(ca - cb).abs() / (7.0 * 864e5)).exp()));
        }
        if let (Some(qa), Some(qb)) = (
            fa.get("freq").and_then(Value::as_array),
            fb.get("freq").and_then(Value::as_array),
        ) {
            let va: Vec<f32> = qa.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            let vb: Vec<f32> = qb.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            add(p.f("w_letters"), Some(cosine(&va, &vb)));
        }
        if w > 0.0 { sum / w } else { 0.0 }
    }

    fn weights(&mut self) {
        let n = self.order.len();
        let mut sim = vec![0.0f32; n * n];
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (&self.nodes[&self.order[i]], &self.nodes[&self.order[j]]);
                let s = if a.free || b.free { 0.0 } else { self.alike(a, b) };
                sim[i * n + j] = s;
                sim[j * n + i] = s;
            }
        }
        self.sim = Some(sim);
    }

    fn depth_of(&self, n: &Node) -> f32 {
        let (ms, ss, ml, mt, tt) = self.mean;
        match self.p.s("depth_by") {
            "ordinal" => (fnum(&n.f, "s") - ms) / ss.max(1.0),
            "length" => fnum(&n.f, "chars").max(1.0).ln_1p() - ml,
            "time" => {
                let c = fnum(&n.f, "created");
                if c > 0.0 { (c - mt) / tt.max(864e5) } else { 0.0 }
            }
            _ => 0.0,
        }
    }

    fn follow(&mut self, h: f32) -> f32 {
        let k = (10.0 * h).min(1.0);
        let mut moved = 0.0;
        for key in self.order.clone() {
            let Some(n) = self.nodes.get(&key) else { continue };
            if n.links.is_empty() {
                continue;
            }
            let gap = 0.12;
            let left = n.x - n.hw;
            let mut y = n.y - n.hh - gap;
            let nz = n.z;
            for c in n.links.clone() {
                let Some(c) = self.nodes.get_mut(&c) else { continue };
                let (tx, ty) = (left + c.hw, y - c.hh);
                y -= c.hh * 2.0 + gap;
                let (dx, dy, dz) = (tx - c.x, ty - c.y, nz - c.z);
                c.x += dx * k;
                c.y += dy * k;
                c.z += dz * k;
                (c.vx, c.vy, c.vz) = (0.0, 0.0, 0.0);
                moved += dx * dx + dy * dy + dz * dz;
            }
        }
        moved
    }

    pub fn step(&mut self, h: f32) {
        let n = self.order.len();
        if n == 0 {
            return;
        }
        if self.sim.as_ref().is_none_or(|s| s.len() != n * n) {
            self.weights();
        }
        if self.p.s("depth_by") != "none" {
            let (mut s, mut l, mut t, mut k) = (0.0, 0.0, 0.0, 0.0);
            for key in &self.order {
                let f = &self.nodes[key].f;
                s += fnum(f, "s");
                l += fnum(f, "chars").max(1.0).ln_1p();
                if fnum(f, "created") > 0.0 {
                    t += fnum(f, "created");
                    k += 1.0;
                }
            }
            let nn = n as f32;
            let (ms, ml, mt) = (s / nn, l / nn, if k > 0.0 { t / k } else { 0.0 });
            let (mut ss, mut tt) = (0.0, 0.0);
            for key in &self.order {
                let f = &self.nodes[key].f;
                ss += (fnum(f, "s") - ms).powi(2);
                if fnum(f, "created") > 0.0 {
                    tt += (fnum(f, "created") - mt).powi(2);
                }
            }
            self.mean = (ms, (ss / nn).sqrt(), ml, mt, (tt / k.max(1.0)).sqrt());
        }
        let a = self.alpha;
        let (repel, attract, link, gravity, flat, depth, damping, pad) = (
            self.p.f("repel"),
            self.p.f("attract"),
            self.p.f("link"),
            self.p.f("gravity"),
            self.p.f("flat"),
            self.p.f("depth"),
            self.p.f("damping"),
            self.p.f("pad"),
        );
        let depth_none = self.p.s("depth_by") == "none";
        let sim = self.sim.clone().unwrap_or_default();
        let nodes: Vec<Node> = self.order.iter().map(|k| self.nodes[k].clone()).collect();
        let dragged = self.drag.as_ref().map(|d| d.n.clone());
        let mut fx = vec![0.0f32; n];
        let mut fy = vec![0.0f32; n];
        let mut fz = vec![0.0f32; n];
        for i in 0..n {
            let aa = &nodes[i];
            for j in i + 1..n {
                let bb = &nodes[j];
                if aa.head.is_some() || bb.head.is_some() || aa.zone != bb.zone {
                    continue;
                }
                let (mut dx, mut dy, dz) = (bb.x - aa.x, bb.y - aa.y, bb.z - aa.z);
                let mut d2 = dx * dx + dy * dy + dz * dz;
                if d2 < 1e-6 {
                    dx = (hash01(&format!("{}{}", aa.key, bb.key), 7) - 0.5) * 0.01;
                    dy = (hash01(&format!("{}{}", bb.key, aa.key), 9) - 0.5) * 0.01;
                    d2 = dx * dx + dy * dy + 1e-6;
                }
                let d = d2.sqrt();
                let s = sim[i * n + j];
                let mut f = -repel * (1.0 + (-s).max(0.0) * 2.0) / (d2 + 0.5);
                if s > 0.0 {
                    f += attract * s * (d - link * (1.15 - s));
                }
                let (ux, uy, uz) = (dx / d, dy / d, dz / d);
                fx[i] += f * ux;
                fy[i] += f * uy;
                fz[i] += f * uz;
                fx[j] -= f * ux;
                fy[j] -= f * uy;
                fz[j] -= f * uz;
            }
            fx[i] -= gravity * aa.x;
            fy[i] -= gravity * aa.y;
            let zt = if depth_none { 0.0 } else { self.depth_of(aa) * depth };
            fz[i] -= flat * (aa.z - zt);
        }
        let mut ke = 0.0;
        let drag_k = (-damping * h).exp();
        for (i, key) in self.order.clone().iter().enumerate() {
            let Some(aa) = self.nodes.get_mut(key) else { continue };
            if aa.pinned || aa.head.is_some() || dragged.as_deref() == Some(key.as_str()) {
                (aa.vx, aa.vy, aa.vz) = (0.0, 0.0, 0.0);
                continue;
            }
            aa.vx = (aa.vx + a * fx[i] * h) * drag_k;
            aa.vy = (aa.vy + a * fy[i] * h) * drag_k;
            aa.vz = (aa.vz + a * fz[i] * h) * drag_k;
            let v2 = aa.vx * aa.vx + aa.vy * aa.vy + aa.vz * aa.vz;
            if v2 > 900.0 {
                let k = 30.0 / v2.sqrt();
                aa.vx *= k;
                aa.vy *= k;
                aa.vz *= k;
            }
            aa.x += aa.vx * h;
            aa.y += aa.vy * h;
            aa.z += aa.vz * h;
            ke += v2;
        }
        ke += self.follow(h) * 100.0;
        // labels never overlap: rectangles apart in the facing plane, the shorter way
        let keys = self.order.clone();
        for _pass in 0..2 {
            for i in 0..n {
                for j in i + 1..n {
                    let (ka, kb) = (&keys[i], &keys[j]);
                    let (a0, b0) = (self.nodes[ka].clone(), self.nodes[kb].clone());
                    if (a0.z - b0.z).abs() > 1.5 || a0.zone != b0.zone {
                        continue;
                    }
                    let ox = a0.hw + b0.hw + pad - (b0.x - a0.x).abs();
                    let oy = a0.hh + b0.hh + pad * 0.5 - (b0.y - a0.y).abs();
                    if ox <= 0.0 || oy <= 0.0 {
                        continue;
                    }
                    if a0.head.as_deref() == Some(kb.as_str())
                        || (a0.head.is_some() && a0.head == b0.head)
                        || b0.head.as_deref() == Some(ka.as_str())
                    {
                        continue;
                    }
                    let fixed = |x: &Node| x.pinned || x.head.is_some() || dragged.as_deref() == Some(x.key.as_str());
                    let am = if fixed(&a0) { 0.0 } else { 1.0 };
                    let bm = if fixed(&b0) { 0.0 } else { 1.0 };
                    if am == 0.0 && bm == 0.0 {
                        continue;
                    }
                    let share = if am > 0.0 && bm > 0.0 { 0.5 } else { 1.0 };
                    if ox / (a0.hw + b0.hw) < oy / (a0.hh + b0.hh) {
                        let sgn = if b0.x >= a0.x { 1.0 } else { -1.0 };
                        if let Some(x) = self.nodes.get_mut(ka) {
                            x.x -= sgn * ox * share * am;
                        }
                        if let Some(x) = self.nodes.get_mut(kb) {
                            x.x += sgn * ox * share * bm;
                        }
                    } else {
                        let sgn = if b0.y >= a0.y { 1.0 } else { -1.0 };
                        if let Some(x) = self.nodes.get_mut(ka) {
                            x.y -= sgn * oy * share * am;
                        }
                        if let Some(x) = self.nodes.get_mut(kb) {
                            x.y += sgn * oy * share * bm;
                        }
                    }
                }
            }
        }
        self.alpha = (a - a * self.p.f("cool")).max(0.0);
        self.energy = ke / n as f32;
    }

    pub fn asleep(&self) -> bool {
        self.alpha < 0.01 && self.energy < 0.0005
    }

    pub fn settle(&mut self) -> u32 {
        self.wake(1.0);
        let t0 = std::time::Instant::now();
        let mut i = 0;
        while i < 6000 && t0.elapsed().as_millis() < 400 {
            self.step(STEP);
            i += 1;
            if self.asleep() {
                break;
            }
        }
        self.dirty = true;
        i
    }

    // ------------------------------------------------------------------ editing

    fn pick(&mut self, key: Option<String>) {
        self.selected = key.clone();
        self.dirty = true;
        self.send_ev("select", key.as_deref(), None, Value::Null);
    }

    pub fn record(&mut self, u: Undo) {
        if let Some(b) = &mut self.batch {
            b.push(u);
            return;
        }
        self.undo.push(u);
        if self.undo.len() > 300 {
            self.undo.remove(0);
        }
    }

    pub fn set_batch(&mut self, on: bool) {
        if on {
            self.batch.get_or_insert_with(Vec::new);
            return;
        }
        if let Some(mut b) = self.batch.take()
            && !b.is_empty()
        {
            let u = if b.len() == 1 { b.remove(0) } else { Undo::Many(b) };
            self.record(u);
        }
    }

    fn undo_name(u: &Undo) -> String {
        match u {
            Undo::Move { .. } => "move",
            Undo::Pin { .. } => "pin",
            Undo::Remove { .. } => "remove",
            Undo::Add { .. } => "add",
            Undo::Text { .. } => "text",
            Undo::Size { .. } => "size",
            Undo::Own { .. } => "own",
            Undo::Event { .. } => "event",
            Undo::Many(_) => "many",
        }
        .to_string()
    }

    pub fn undo_last(&mut self) -> String {
        let Some(u) = self.undo.pop() else {
            return "nothing".into();
        };
        let name = Self::undo_name(&u);
        self.apply_undo(u);
        self.wake(0.5);
        name
    }

    fn apply_undo(&mut self, u: Undo) {
        match u {
            Undo::Many(items) => {
                for x in items.into_iter().rev() {
                    self.apply_undo(x);
                }
            }
            Undo::Move { n, x, y, z, pinned } => {
                if let Some(n) = self.nodes.get_mut(&n) {
                    (n.x, n.y, n.z, n.pinned) = (x, y, z, pinned);
                }
            }
            Undo::Pin { n, was } => {
                if let Some(n) = self.nodes.get_mut(&n) {
                    n.pinned = was;
                }
            }
            Undo::Remove { n } => {
                let key = n.key.clone();
                self.nodes.insert(key.clone(), *n);
                self.order.push(key);
                self.sim = None;
            }
            Undo::Add { n } => self.drop_node(&n),
            Undo::Text { n, from } => self.set_text(&n, &from),
            Undo::Size { n, from } => {
                if let Some(x) = self.nodes.get_mut(&n) {
                    x.size = from;
                }
                self.measure(&n);
                self.send_ev("select", Some(&n), None, Value::Null);
            }
            Undo::Own { n, from } => {
                if let Some(x) = self.nodes.get_mut(&n) {
                    x.own = from;
                }
                self.measure(&n);
            }
            Undo::Event { kind, data } => self.send_ev(&kind, None, None, data),
        }
    }

    pub fn pin_selected(&mut self) -> Option<&'static str> {
        let k = self.selected.clone()?;
        let n = self.nodes.get(&k)?;
        let was = n.pinned;
        self.record(Undo::Pin { n: k.clone(), was });
        if let Some(n) = self.nodes.get_mut(&k) {
            n.pinned = !was;
        }
        self.measure(&k);
        self.wake(0.5);
        Some(if was { "unpinned" } else { "pinned" })
    }

    pub fn remove_node(&mut self, key: &str) -> Option<String> {
        let n = self.nodes.get(key)?.clone();
        self.record(Undo::Remove { n: Box::new(n) });
        self.drop_node(key);
        self.info(true);
        Some(key.to_string())
    }

    pub fn duplicate(&mut self, node: Option<&str>, det: u64) -> Option<String> {
        let k = node.map(String::from).or_else(|| self.selected.clone())?;
        let n = self.nodes.get(&k)?.clone();
        let (cdet, cown) = if n.free {
            (0, None)
        } else if det > 0 {
            (det, None)
        } else if n.det > 0 {
            (n.det, None)
        } else {
            (0, n.own.clone())
        };
        let ck = if n.free {
            self.free_n += 1;
            format!("t:{}{}", (self.clock * 1000.0) as u64, self.free_n)
        } else {
            self.copy_key(&n.entry())
        };
        let c = self.make(n.f.clone(), n.free, Some(ck.clone()), cdet, cown);
        if let Some(s) = n.size {
            if let Some(x) = self.nodes.get_mut(&c) {
                x.size = Some(s);
            }
            self.measure(&c);
        }
        let (chw, chh) = self.nodes.get(&c).map(|x| (x.hw, x.hh)).unwrap_or((1.0, 0.4));
        let (mut cx, mut cy, cz) = (n.x + n.hw * 2.0 + 0.6, n.y, n.z);
        let _ = &mut cx;
        for _ in 0..50 {
            let near = self.order.iter().any(|o| {
                *o != c
                    && self.nodes.get(o).is_some_and(|x| {
                        (x.x - cx).abs() < chw && (x.y - cy).abs() < chh * 2.0 && (x.z - cz).abs() < 0.5
                    })
            });
            if !near {
                break;
            }
            cy -= chh * 2.0 + 0.25;
        }
        if let Some(x) = self.nodes.get_mut(&c) {
            (x.x, x.y, x.z, x.pinned) = (cx, cy, cz, true);
        }
        self.record(Undo::Add { n: c.clone() });
        self.pick(Some(c.clone()));
        self.sim = None;
        self.info(true);
        Some(c)
    }

    pub fn add_free(&mut self, text: &str) -> String {
        self.free_n += 1;
        let key = format!("t:{}{}", (self.clock * 1000.0) as u64, self.free_n);
        let mut f = Map::new();
        f.insert("key".into(), json!(key));
        f.insert("text".into(), json!(if text.is_empty() { "text" } else { text }));
        f.insert("note".into(), json!(false));
        let k = self.make(f, true, None, 0, None);
        let ahead = self.at + self.camera_rot() * Vec3::new(0.0, 0.0, -6.0);
        self.set_local(&k, ahead);
        if let Some(n) = self.nodes.get_mut(&k) {
            n.pinned = true;
        }
        self.record(Undo::Add { n: k.clone() });
        self.pick(Some(k.clone()));
        k
    }

    pub fn edit_free(&mut self, key: &str, text: &str) {
        if let Some(n) = self.nodes.get(key)
            && n.free
        {
            let from = fstr(&n.f, "text").to_string();
            self.record(Undo::Text {
                n: key.to_string(),
                from,
            });
            self.set_text(key, text);
            return;
        }
        for k in self.order.clone() {
            if self
                .nodes
                .get(&k)
                .is_some_and(|x| !x.free && x.det == 0 && x.entry() == key)
            {
                self.set_text(&k, text);
            }
        }
    }

    pub fn set_size(&mut self, node: Option<&str>, by: i64, to: Option<i64>) -> Option<i64> {
        let k = node.map(String::from).or_else(|| self.selected.clone())?;
        let n = self.nodes.get(&k)?;
        let now = n.size.map(|s| (s * 50.0).round() as i64).unwrap_or(0);
        let base = if now > 0 {
            now
        } else {
            (self.p.f("size") * 50.0).round() as i64
        };
        let step = to.unwrap_or((base + by).clamp(1, 255));
        let from = n.size;
        self.record(Undo::Size { n: k.clone(), from });
        if let Some(n) = self.nodes.get_mut(&k) {
            n.size = (step > 0).then_some(step as f32 / 50.0);
        }
        self.measure(&k);
        self.sim = None;
        self.wake(0.3);
        self.send_ev("select", Some(&k), None, Value::Null);
        Some(step)
    }

    pub fn set_own(&mut self, node: &str, text: Option<&str>) -> Option<String> {
        let n = self.nodes.get(node)?;
        if n.free || n.det > 0 {
            return None;
        }
        let from = n.own.clone();
        self.record(Undo::Own {
            n: node.to_string(),
            from,
        });
        if let Some(n) = self.nodes.get_mut(node) {
            n.own = text.filter(|t| !t.is_empty()).map(String::from);
        }
        self.measure(node);
        self.wake(0.3);
        Some(node.to_string())
    }

    pub fn rows(&self) -> Vec<Value> {
        self.order
            .iter()
            .filter_map(|k| self.nodes.get(k))
            .filter(|n| n.head.is_none())
            .map(|n| {
                let w = self.world(n);
                let mut o = Map::new();
                if !n.free {
                    o.insert("key".into(), json!(n.entry()));
                }
                if let Some(s) = n.size {
                    o.insert("size".into(), json!(s));
                }
                if !n.free && n.det > 0 {
                    o.insert("detached".into(), json!(n.det));
                }
                if !n.free
                    && let Some(own) = &n.own
                {
                    o.insert("own".into(), json!(own));
                }
                let r = |v: f32| (v * 100.0).round() / 100.0;
                json!([
                    n.own.clone().unwrap_or_else(|| fstr(&n.f, "text").to_string()),
                    r(w.x),
                    r(w.y),
                    r(w.z),
                    o
                ])
            })
            .collect()
    }

    pub fn load_rows(&mut self, rows: &[Value], feats: &Value) {
        self.clear(true);
        self.set_frame();
        for row in rows {
            let Some(a) = row.as_array() else { continue };
            let text = a.first().and_then(Value::as_str).unwrap_or("").to_string();
            let w = Vec3::new(
                a.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32,
                a.get(2).and_then(Value::as_f64).unwrap_or(0.0) as f32,
                a.get(3).and_then(Value::as_f64).unwrap_or(0.0) as f32,
            );
            let o = a.get(4).cloned().unwrap_or(Value::Null);
            let has_key = o.get("key").is_some_and(|k| !k.is_null());
            self.free_n += 1;
            let key = if has_key {
                key_text(&o["key"])
            } else {
                format!("t:{}{}", (self.clock * 1000.0) as u64, self.free_n)
            };
            let mut f: Map<String, Value> = if has_key {
                feats.get(&key).and_then(Value::as_object).cloned().unwrap_or_default()
            } else {
                Map::new()
            };
            if f.is_empty() {
                f.insert("key".into(), json!(key));
                f.insert("note".into(), json!(false));
            }
            let own = if has_key {
                o.get("own").and_then(Value::as_str).map(String::from)
            } else {
                None
            };
            let det = if has_key {
                o.get("detached").and_then(Value::as_u64).unwrap_or(0)
            } else {
                0
            };
            if !(own.is_some() && f.get("text").is_some()) {
                f.insert("text".into(), json!(text));
            }
            let nk = if self.nodes.contains_key(&key) {
                Some(self.copy_key(&key))
            } else {
                None
            };
            let k = self.make(f, !has_key, nk, det, own);
            if let Some(s) = o.get("size").and_then(Value::as_f64) {
                if let Some(n) = self.nodes.get_mut(&k) {
                    n.size = Some(s as f32);
                }
                self.measure(&k);
            }
            self.set_local(&k, w);
            if let Some(n) = self.nodes.get_mut(&k) {
                n.pinned = true;
            }
        }
        self.sim = None;
        self.info(true);
    }

    // ------------------------------------------------------------------ commands

    /// a command from Rust, as space.js took them; what it answers goes out as events
    pub fn command(&mut self, c: &Value) {
        let cmd = c["cmd"].as_str().unwrap_or("");
        let s = |k: &str| c[k].as_str().unwrap_or("").to_string();
        match cmd {
            "place" => {
                let texts = c["texts"].as_array().cloned().unwrap_or_default();
                self.place(&texts, c["again"] == true);
            }
            "effects" => self.effects(c.get("fx")),
            "clear" => self.clear(c["all"] == true),
            "room" => {
                self.room = s("bg");
                self.ink = s("ink");
                self.dirty = true;
            }
            "start" => self.running = true,
            "stop" => {
                self.running = false;
                self.held.clear();
                self.looking = false;
            }
            "mark" => {
                let (l, cnd) = (
                    c["list"].as_str().map(String::from),
                    c["candidate"].as_str().map(String::from),
                );
                self.mark(l.as_deref(), cnd.as_deref());
            }
            "keys" => {
                self.bind = default_keys();
                if let Some(m) = c["map"].as_object() {
                    for (k, v) in m {
                        if let Some(v) = v.as_str() {
                            self.bind.insert(k.clone(), v.to_string());
                        }
                    }
                }
                self.held.clear();
            }
            "physics" => {
                let old = self.p.clone();
                self.p = Physics::new(c.get("p"));
                let changed = |k: &str| old.v.get(k) != self.p.v.get(k);
                if old
                    .v
                    .keys()
                    .any(|k| (k.starts_with("w_") || k == "qscale" || k == "reversed") && changed(k))
                {
                    self.sim = None;
                }
                if changed("wrap") || changed("size") {
                    for k in self.order.clone() {
                        self.measure(&k);
                    }
                }
                self.wake(0.6);
            }
            "pause" => self.paused = c["on"] == true,
            "settle" => {
                let n = self.settle();
                let mut e = self.ev("settled", None, None, Value::Null);
                e["data"] = json!(n);
                self.send(e);
            }
            "undo" => {
                let k = self.undo_last();
                let mut e = self.ev("undone", None, None, Value::Null);
                e["data"] = json!(k);
                self.send(e);
            }
            "pin" => {
                let t = self.pin_selected().unwrap_or("select a text first").to_string();
                self.toast(&t);
            }
            "remove" => {
                let k = self.selected.clone().and_then(|k| self.remove_node(&k));
                self.toast(if k.is_some() {
                    "text removed · Ctrl+Z puts it back"
                } else {
                    "select a text first"
                });
            }
            "free" => {
                let t = s("text");
                let key = self.add_free(&t);
                let mut e = self.ev("made", None, None, Value::Null);
                e["key"] = json!(key);
                e["text"] = json!(if t.is_empty() { "text".to_string() } else { t });
                e["free"] = json!(true);
                self.send(e);
            }
            "retext" => {
                let (k, t) = (s("key"), s("text"));
                self.edit_free(&k, &t);
            }
            "rows" => {
                let rows = self.rows();
                let mut e = self.ev("rows", None, None, Value::Null);
                e["data"] = json!({ "why": c["why"], "name": c["name"].as_str().unwrap_or(""), "rows": rows });
                self.send(e);
            }
            "load" => {
                let rows = c["rows"].as_array().cloned().unwrap_or_default();
                let feats = c.get("feats").cloned().unwrap_or(json!({}));
                self.load_rows(&rows, &feats);
            }
            "colours" => {
                let m = c.get("map").cloned().unwrap_or(json!({}));
                self.colours(&m);
            }
            "dup" => {
                let node = c["node"].as_str().map(String::from);
                let det = c["det"].as_u64().unwrap_or(0);
                let k = self.duplicate(node.as_deref(), det);
                self.toast(match (k.is_some(), det > 0) {
                    (false, _) => "select a text first",
                    (true, true) => "a detached copy: it keeps the text as it reads now · Ctrl+Z takes it away",
                    (true, false) => "a copy that inherits: it shows the entry as it changes · Ctrl+Z takes it away",
                });
            }
            "size" => {
                let node = c["node"].as_str().map(String::from);
                self.set_size(node.as_deref(), c["by"].as_i64().unwrap_or(0), c["to"].as_i64());
            }
            "chain" => {
                let k = key_text(&c["key"]);
                let ch = c.get("chain").cloned();
                self.set_chain(&k, ch.as_ref());
            }
            "chainify" => {
                let node = s("node");
                if let Some(n) = self.nodes.get_mut(&node) {
                    n.f.insert("key".into(), c["chain"]["key"].clone());
                }
                let k = key_text(&c["chain"]["key"]);
                let ch = c.get("chain").cloned();
                self.set_chain(&k, ch.as_ref());
            }
            "own" => {
                let node = s("node");
                let t = c["text"].as_str().map(String::from);
                let k = self.set_own(&node, t.as_deref());
                self.toast(match (k.is_some(), t.as_deref().is_some_and(|t| !t.is_empty())) {
                    (false, _) => "that placement can't have its own text",
                    (true, true) => "this placement has its own text now; the entry is unchanged · Ctrl+Z undoes it",
                    (true, false) => "it follows the entry again · Ctrl+Z undoes it",
                });
            }
            "removenode" => {
                let k = self.remove_node(&s("node"));
                if c["quiet"] != true {
                    self.toast(if k.is_some() {
                        "taken out of the room (the entry stays) · Ctrl+Z puts it back"
                    } else {
                        "gone already"
                    });
                }
            }
            "pinnode" => {
                let node = s("node");
                if self.nodes.contains_key(&node) {
                    self.pick(Some(node));
                    let t = self.pin_selected().unwrap_or("gone already").to_string();
                    self.toast(&t);
                } else {
                    self.toast("gone already");
                }
            }
            "batch" => self.set_batch(c["on"] == true),
            "blocks" => {
                if c["replace"] == true {
                    self.blocks.clear();
                }
                for b in c["blocks"].as_array().into_iter().flatten() {
                    let id = b["id"].as_str().unwrap_or("").to_string();
                    match self.blocks.iter_mut().find(|(k, _)| *k == id) {
                        Some(x) => x.1 = b.clone(),
                        None => self.blocks.push((id, b.clone())),
                    }
                }
                self.dirty = true;
            }
            "unblock" => {
                let ids: Vec<String> = c["ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                self.blocks.retain(|(k, _)| !ids.contains(k));
                self.dirty = true;
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ input

    pub fn resize(&mut self, w: f32, h: f32) {
        if (w, h) != self.size {
            self.size = (w.max(1.0), h.max(1.0));
            self.dirty = true;
        }
    }

    /// the text whose label is under a point of the picture (the nearest of those over it)
    fn label_at(&self, x: f32, y: f32) -> Option<String> {
        self.order
            .iter()
            .filter_map(|k| self.nodes.get(k))
            .filter_map(|n| {
                let (cx, cy, hw, hh, d) = n.screen?;
                ((x - cx).abs() <= hw && (y - cy).abs() <= hh).then_some((d, n.key.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, k)| k)
    }

    pub fn key_down(&mut self, code: &str, m: Mods) {
        if !self.running || m.ctrl || m.alt {
            return;
        }
        let action = self
            .bind
            .iter()
            .find(|(_, v)| v.as_str() == code)
            .map(|(k, _)| k.clone());
        if action.as_deref().is_some_and(|a| FLYING.contains(&a)) || code == "ShiftLeft" || code == "ShiftRight" {
            self.held.insert(code.to_string());
            if let Some(e) = self.ease.take() {
                self.sync_turn(e.to_q);
            }
            return;
        }
        if code.starts_with("Arrow") && !m.shift {
            self.hop(code);
            return;
        }
        match action.as_deref() {
            Some("level") => self.level(),
            Some("back_to_start") => self.home(),
            Some("slower") => self.speed = (self.speed / 1.25).max(0.5),
            Some("faster") => self.speed = (self.speed * 1.25).min(200.0),
            _ => {}
        }
    }

    pub fn key_up(&mut self, code: &str) {
        self.held.remove(code);
    }

    pub fn blur(&mut self) {
        self.held.clear();
    }

    fn holding(&self, action: &str) -> bool {
        self.bind.get(action).is_some_and(|k| self.held.contains(k))
    }

    fn hop(&mut self, dir: &str) {
        let want = match dir {
            "ArrowLeft" => (-1.0, 0.0),
            "ArrowRight" => (1.0, 0.0),
            "ArrowUp" => (0.0, 1.0),
            _ => (0.0, -1.0),
        };
        let inv = self.camera_rot().inverse();
        let mut best: Option<(f32, String)> = None;
        for k in &self.order {
            let n = &self.nodes[k];
            let local = inv * (self.world(n) - self.at);
            let dist = local.length();
            if dist < 0.5 {
                continue;
            }
            let (sx, sy) = (local.x / dist, local.y / dist);
            let along = sx * want.0 + sy * want.1;
            if along <= 0.02 {
                continue;
            }
            let across = (sx * want.1 - sy * want.0).abs();
            let score =
                (along.min(1.0).acos() + across * 2.0) * 10.0 + dist * 0.05 + if local.z > 0.0 { 50.0 } else { 0.0 };
            if best.as_ref().is_none_or(|b| score < b.0) {
                best = Some((score, k.clone()));
            }
        }
        let Some((_, k)) = best else { return };
        let target = self.world(&self.nodes[&k]);
        let fwd = (target - self.at).normalize_or_zero();
        let stand = (fstr(&self.nodes[&k].f, "text").chars().count() as f32 * 0.35).clamp(4.0, 14.0);
        let to = target - fwd * stand;
        let up = self.camera_rot() * Vec3::Y;
        let to_q = Quat::from_mat4(&glam::camera::rh::view::look_at_mat4(to, target, up).inverse());
        self.ease = Some(Ease {
            from: self.at,
            to,
            from_q: self.camera_rot(),
            to_q,
            t: 0.0,
        });
        self.pick(Some(k.clone()));
        self.send_ev("focus", Some(&k), None, Value::Null);
    }

    pub fn pointer_down(&mut self, x: f32, y: f32, button: u8, m: Mods) {
        self.pointer = (x, y);
        if button == 0
            && let Some(k) = self.label_at(x, y)
        {
            let t = self.nodes.get(&k).and_then(|n| n.head.clone()).unwrap_or(k.clone());
            let Some(tn) = self.nodes.get(&t) else { return };
            let w = self.world(tn);
            let fwd = self.camera_rot() * Vec3::NEG_Z;
            let wn = self.world(&self.nodes[&k]);
            let depth = (wn - self.at).dot(fwd);
            let hit = self.ray_at(x, y, depth);
            self.drag = Some(Drag {
                n: t.clone(),
                el: k,
                x0: x,
                y0: y,
                moved: false,
                depth,
                from: (tn.x, tn.y, tn.z, tn.pinned),
                off: w - hit,
            });
            let _ = m;
            return;
        }
        if button == 0 {
            // a click on empty room lets go of the selection; a double click makes a text
            if self.clock - self.last_empty < 0.45 {
                self.last_empty = -10.0;
                self.send_ev("newtext", None, Some((x, y, m)), Value::Null);
            } else {
                self.last_empty = self.clock;
            }
            if self.selected.is_some() {
                self.pick(None);
            }
            return;
        }
        if button == 2 {
            self.rdown = Some((x, y));
            self.looking = true;
            if let Some(e) = self.ease.take() {
                self.sync_turn(e.to_q);
            }
        }
    }

    pub fn pointer_move(&mut self, x: f32, y: f32, m: Mods) {
        let (dx, dy) = (x - self.pointer.0, y - self.pointer.1);
        self.pointer = (x, y);
        if let Some(d) = self.drag.clone() {
            if !d.moved && (x - d.x0).hypot(y - d.y0) < 4.0 {
                return;
            }
            let w = self.ray_at(x, y, d.depth) + d.off;
            self.set_local(&d.n, w);
            if let Some(n) = self.nodes.get_mut(&d.n) {
                n.pinned = true;
            }
            if let Some(dd) = &mut self.drag {
                dd.moved = true;
            }
            self.wake(0.3);
            return;
        }
        if self.looking {
            let k = LOOK * self.p.f("look").max(0.05);
            self.tyaw -= dx * k;
            self.tpitch = (self.tpitch - dy * k).clamp(-1.55, 1.55);
            return;
        }
        // hovering a text: its card
        let h = self.label_at(x, y);
        if h != self.hover {
            self.hover = h.clone();
            self.dirty = true;
            if let Some(k) = h {
                self.send_ev("hover", Some(&k), Some((x, y, m)), Value::Null);
            }
        }
    }

    pub fn pointer_up(&mut self, x: f32, y: f32, button: u8, m: Mods) {
        if let Some(d) = self.drag.take() {
            if d.moved {
                let (fx, fy, fz, fp) = d.from;
                self.record(Undo::Move {
                    n: d.n.clone(),
                    x: fx,
                    y: fy,
                    z: fz,
                    pinned: fp,
                });
                self.sim = None;
                self.send_ev("moved", Some(&d.n), Some((x, y, m)), Value::Null);
            } else {
                self.pick(Some(d.el.clone()));
                if self
                    .last_click
                    .as_ref()
                    .is_some_and(|(k, t)| *k == d.el && self.clock - t < 0.45)
                {
                    self.last_click = None;
                    self.send_ev("dblclick", Some(&d.el), Some((x, y, m)), Value::Null);
                } else {
                    self.last_click = Some((d.el.clone(), self.clock));
                    self.send_ev("click", Some(&d.el), Some((x, y, m)), Value::Null);
                }
            }
            return;
        }
        if button == 2 {
            self.looking = false;
            let r = self.rdown.take();
            // a right click that didn't look around, on a text: its menu
            if r.is_some_and(|(rx, ry)| (x - rx).hypot(y - ry) < 4.0)
                && let Some(k) = self.label_at(x, y)
            {
                self.send_ev("menu", Some(&k), Some((x, y, m)), Value::Null);
            }
        }
    }

    pub fn wheel(&mut self, x: f32, y: f32, dy: f32) {
        if let Some(k) = self.label_at(x, y) {
            // push away or pull near, along the line from the eye
            let f = (-dy * 0.0015).exp();
            let n = &self.nodes[&k];
            let w = self.world(n);
            let from = w - self.at;
            let len = (from.length() * f).clamp(0.3, 2000.0);
            if !self
                .wheel_undo
                .as_ref()
                .is_some_and(|(wk, t)| *wk == k && self.clock - t < 0.8)
            {
                let (nx, ny, nz, np) = (n.x, n.y, n.z, n.pinned);
                self.record(Undo::Move {
                    n: k.clone(),
                    x: nx,
                    y: ny,
                    z: nz,
                    pinned: np,
                });
            }
            self.wheel_undo = Some((k.clone(), self.clock));
            let to = self.at + from.normalize_or_zero() * len;
            self.set_local(&k, to);
            if let Some(n) = self.nodes.get_mut(&k) {
                n.pinned = true;
            }
            self.wake(0.3);
            return;
        }
        self.speed = (self.speed * if dy > 0.0 { 0.9 } else { 1.1 }).clamp(0.5, 200.0);
        self.info(true);
    }

    // ------------------------------------------------------------------ the frame

    /// moves the room on by `dt` seconds
    pub fn frame(&mut self, dt: f32) {
        let dt = dt.min(0.1);
        self.clock += dt;
        let rot = self.camera_rot();
        if !self.held.is_empty() {
            let fast = if self.held.contains("ShiftLeft") || self.held.contains("ShiftRight") {
                4.0
            } else {
                1.0
            };
            let v = self.speed * fast * dt;
            let mut mv = Vec3::ZERO;
            for (a, d) in [
                ("fly_forward", Vec3::NEG_Z),
                ("fly_back", Vec3::Z),
                ("fly_left", Vec3::NEG_X),
                ("fly_right", Vec3::X),
                ("fly_up", Vec3::Y),
                ("fly_down", Vec3::NEG_Y),
            ] {
                if self.holding(a) {
                    mv += rot * d;
                }
            }
            self.at += mv * v;
            if self.holding("roll_left") {
                self.roll_a += ROLL * dt;
            }
            if self.holding("roll_right") {
                self.roll_a -= ROLL * dt;
            }
            self.dirty = true;
        }
        if let Some(mut e) = self.ease {
            e.t = (e.t + dt / 0.6).min(1.0);
            let s = e.t * e.t * (3.0 - 2.0 * e.t);
            self.at = e.from.lerp(e.to, s);
            let q = e.from_q.slerp(e.to_q, s);
            self.ease = if e.t >= 1.0 { None } else { Some(e) };
            self.sync_turn(q);
            if self.ease.is_some() {
                let (y, x, z) = q.to_euler(glam::EulerRot::YXZ);
                (self.yaw, self.pitch, self.roll_a) = (y, x, z);
            }
            self.dirty = true;
        } else {
            let a = 1.0 - (-(self.p.f("smooth").max(1.0)) * dt).exp();
            let (dy, dp) = (self.tyaw - self.yaw, self.tpitch - self.pitch);
            if dy.abs() > 1e-5 || dp.abs() > 1e-5 {
                self.yaw += dy * a;
                self.pitch += dp * a;
                self.dirty = true;
            }
        }
        let moving = !self.paused && !self.order.is_empty() && !self.asleep();
        if moving {
            self.acc = (self.acc + dt * self.p.f("substeps").max(1.0)).min(0.25);
            while self.acc >= STEP {
                self.step(STEP);
                self.acc -= STEP;
            }
            self.dirty = true;
        }
        self.info(false);
    }

    // ------------------------------------------------------------------ what's drawn

    /// a CSS colour (`#rgb`, `#rrggbb`, `rgb(…)`, `hsl(…)`, a few names) as linear RGBA,
    /// premultiplied by `alpha`
    pub fn colour(css: &str, alpha: f32) -> [f32; 4] {
        let [r, g, b] = css_rgb(css).unwrap_or([0.1, 0.1, 0.1]);
        let lin = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        [lin(r) * alpha, lin(g) * alpha, lin(b) * alpha, alpha]
    }

    /// what to draw now; it also keeps where each label went, for the pointer
    pub fn scene(&mut self) -> crate::Scene {
        let sky = Self::colour(&self.room, 1.0);
        let mut scene = crate::Scene {
            sky: [sky[0], sky[1], sky[2]],
            ..Default::default()
        };
        // the blocks (their own slice moves textures, bends and editing over)
        for (_, b) in &self.blocks {
            let f = |k: &str, d: f32| b[k].as_f64().map(|x| x as f32).unwrap_or(d);
            let r: Vec<f32> = b["rot"]
                .as_array()
                .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect())
                .unwrap_or_default();
            let rot = if r.len() == 4 {
                Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize()
            } else {
                Quat::IDENTITY
            };
            let tex = b["texture"].as_str().unwrap_or("");
            let c = if tex.starts_with('#') {
                Self::colour(tex, 1.0)
            } else {
                [0.55, 0.5, 0.45, 1.0]
            };
            scene.blocks.push(crate::Block {
                at: Vec3::new(f("x", 0.0), f("y", 0.0), f("z", 0.0)),
                rot,
                size: Vec3::new(f("w", 1.0), f("h", 1.0), f("d", 1.0)),
                colour: [c[0], c[1], c[2]],
                sphere: b["shape"] == "sphere",
            });
        }
        // the labels, as the webview placed them: the farthest first
        let (w, h) = self.size;
        let focal = h / 2.0 / (25f32).to_radians().tan();
        let inv = self.camera_rot().inverse();
        let size0 = self.p.f("size");
        let min_px = self.p.f("minPx");
        let wrap = self.p.f("wrap");
        let mut drawn: Vec<(f32, crate::Label)> = Vec::new();
        let keys = self.order.clone();
        for k in keys {
            let Some(n) = self.nodes.get(&k) else { continue };
            let v = inv * (self.world(n) - self.at);
            let dist = -v.z;
            let mut screen = None;
            if dist >= 0.3 {
                let size = n.size.unwrap_or(size0) * n.fx_size;
                let px = size * focal / dist;
                let x = w / 2.0 + v.x / dist * focal;
                let y = h / 2.0 - v.y / dist * focal;
                if px >= min_px && x > -w && x < 2.0 * w && y > -h && y < 2.0 * h {
                    let em = px.min(220.0);
                    let alpha = (1.4 - dist / 220.0).clamp(0.15, 1.0);
                    let accent = "#e811a0";
                    let hot = self.hover.as_deref() == Some(k.as_str()) || self.selected.as_deref() == Some(k.as_str());
                    let fx = (!n.free).then(|| self.fx.get(&n.entry())).flatten();
                    let col = if hot {
                        accent.to_string()
                    } else {
                        fx.and_then(|e| e.get("colour"))
                            .and_then(Value::as_str)
                            .map(String::from)
                            .or_else(|| n.colour.clone())
                            .or_else(|| n.f.get("colour").and_then(Value::as_str).map(String::from))
                            .unwrap_or_else(|| self.ink.clone())
                    };
                    let under = fx
                        .and_then(|e| e.get("underline"))
                        .and_then(Value::as_str)
                        .or_else(|| n.f.get("under").and_then(Value::as_str))
                        .map(|c| Self::colour(c, alpha));
                    let wrap_em = if n.free { 80.0 } else { wrap } * text::ch();
                    let shown = n.shown();
                    let lay = text::layout(&shown, wrap_em);
                    screen = Some((x, y, lay.w * em / 2.0, lay.h * em / 2.0, dist));
                    drawn.push((
                        dist,
                        crate::Label {
                            x,
                            y,
                            px: em,
                            text: shown,
                            colour: Self::colour(&col, alpha),
                            under,
                            back: n.picked.then(|| Self::colour("#c9a227", 0.4 * alpha)),
                            wrap: wrap_em,
                        },
                    ));
                }
            }
            if let Some(n) = self.nodes.get_mut(&k) {
                n.screen = screen;
            }
        }
        drawn.sort_by(|a, b| b.0.total_cmp(&a.0));
        scene.labels = drawn.into_iter().map(|(_, l)| l).collect();
        self.dirty = false;
        scene
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(key: &str, t: &str, s: f64) -> Value {
        json!({ "key": key, "text": t, "note": false, "s": s, "r": 50.0, "q": 3, "chars": t.len() })
    }

    #[test]
    fn texts_spread_apart_and_settle() {
        let mut r = Room::new();
        r.resize(1200.0, 800.0);
        r.command(&json!({ "cmd": "start" }));
        r.command(&json!({ "cmd": "place", "texts": [text("0,1", "apple", 50.0), text("0,2", "pear", 51.0), text("0,3", "granite", 80.0)] }));
        assert_eq!(r.order.len(), 3);
        let n = r.settle();
        assert!(n > 0);
        // labels don't overlap
        let ns: Vec<&Node> = r.order.iter().map(|k| &r.nodes[k]).collect();
        for i in 0..3 {
            for j in i + 1..3 {
                let (a, b) = (ns[i], ns[j]);
                let apart =
                    (a.x - b.x).abs() >= a.hw + b.hw || (a.y - b.y).abs() >= a.hh + b.hh || (a.z - b.z).abs() > 1.5;
                assert!(apart, "{} and {} overlap", a.key, b.key);
            }
        }
        // they're in front of the camera, and drawn
        let s = r.scene();
        assert_eq!(s.labels.len(), 3, "{:?}", s.labels);
        // a click on a label selects it (the events say so)
        let l = &r.nodes["0,1"].screen.unwrap();
        r.pointer_down(l.0, l.1, 0, Mods::default());
        r.pointer_up(l.0, l.1, 0, Mods::default());
        let ev = r.take_events();
        assert!(ev.iter().any(|e| e["kind"] == "click" && e["key"] == "0,1"), "{ev:?}");
        assert_eq!(r.selected.as_deref(), Some("0,1"));
        // a drag moves it and pins it; Ctrl+Z puts it back
        let (x0, y0) = (r.nodes["0,1"].x, r.nodes["0,1"].y);
        r.pointer_down(l.0, l.1, 0, Mods::default());
        r.pointer_move(l.0 + 80.0, l.1, Mods::default());
        r.pointer_up(l.0 + 80.0, l.1, 0, Mods::default());
        assert!(r.nodes["0,1"].pinned && (r.nodes["0,1"].x - x0).abs() > 0.1);
        r.command(&json!({ "cmd": "undo" }));
        assert!(((r.nodes["0,1"].x - x0).abs() < 1e-4) && ((r.nodes["0,1"].y - y0).abs() < 1e-4));
        // flying: W moves the camera forward
        r.key_down("KeyW", Mods::default());
        r.frame(0.1);
        r.key_up("KeyW");
        assert!(r.at.z < -0.3, "{:?}", r.at);
        // colours as CSS writes them
        assert_eq!(css_rgb("#ff8000"), Some([1.0, 128.0 / 255.0, 0.0]));
        let h = css_rgb("hsl(120, 100%, 50%)").unwrap();
        assert!(h[1] > 0.99 && h[0] < 0.01);
        // a free text, its copy, rows out and back in
        r.command(&json!({ "cmd": "free", "text": "a note to self" }));
        r.command(&json!({ "cmd": "dup", "node": "0,2" }));
        assert!(r.nodes.contains_key("0,2~2"));
        let rows = r.rows();
        assert_eq!(rows.len(), 5);
        r.load_rows(&rows, &json!({}));
        assert_eq!(r.order.len(), 5);
    }
}
