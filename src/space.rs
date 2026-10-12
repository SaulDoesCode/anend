//! The space: remem (kuis), weum's spatial memory, as a view of the app. You fly through a
//! warm room where memory texts float as camera-facing billboards drawn with
//! bevy_slugtext. A gsearch pulls in its bucket's memories, and [`crate::spread`] lays them
//! out by their sums, in front of where you stood when you searched.
//!
//! Flying: hold the right mouse button to look, WASD to move, Space / C up and down, Q / E
//! roll, Shift fast, `[` `]` or the wheel over empty space change the speed, R levels the
//! horizon, Home goes back to the start.
//!
//! Texts: drag one to move it (a spread text dragged stays pinned), wheel on it to push it
//! away or pull it near (while dragging: its depth), click puts its text in the terminal,
//! double click edits it (a memory text: puts its key in the terminal), double click on
//! empty space or N makes a new text, Enter edits the selected one, Delete removes it,
//! Ctrl+C copies it, Ctrl+Z undoes (a forget too). Ctrl+click picks a list candidate,
//! Shift+click the list, Ctrl+right click gsearches ⌊√(s·r)⌋.
//!
//! The terminal: Enter on a number gsearches that bucket, on text gsearches its sum (or
//! with gsearch off, searches its words); Ctrl+Enter saturates the text and gsearches its
//! sum, Shift+Enter recalls a key, Delete forgets the key typed, ↑ ↓ walk the history,
//! `#123` `#r123` `#name` go to a bucket or a saved map, and the words Enlist List Delist
//! Unlist Count Backup Clear ClearAll Settle Maps `Save name` are commands.
//!
//! Every key can be remapped in the keys menu. weum's esearch, speech and image memories
//! aren't part of it.

use bevy::ecs::system::SystemParam;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::input_focus::InputFocus;
use bevy::picking::cursor::{EntityCursor, OverrideCursor};
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::render::view::Tonemapping;
use bevy::text::EditableText;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow, SystemCursorIcon};
use bevy_slugtext::prelude::{TextMaterial, TextMesh};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::keys::{Action, Fired, Keymap, modifiers};
use crate::memory::{Bucket, BucketKey, Hit, Ref};
use crate::spread::{self, Node as SNode, Params, Spread};
use crate::ui::{Db, Fonts, Route, Session};

// ------------------------------------------------------------------ constants

pub const START_POS: Vec3 = Vec3::new(0.0, 1.6, 8.0);
pub const START_LOOK: Vec3 = Vec3::new(0.0, 1.6, -6.0);
/// world units per second
const SPEED: f32 = 6.0;
/// Shift multiplier
const BOOST: f32 = 4.0;
/// radians per second for Q / E
const ROLL_RATE: f32 = 1.4;
/// radians per pixel of mouse movement
const LOOK: f32 = 0.0022;
pub const FOV_DEG: f32 = 70.0;
const NEAR: f32 = 0.05;
const FAR: f32 = 5000.0;
pub const DEFAULT_SIZE: f32 = 0.5;
/// line height in em
const LINE: f32 = 1.3;
/// em above the baseline that is treated as the visual middle of a line
const CENTER: f32 = 0.36;
const UNDO_MAX: usize = 300;
const HISTORY_MAX: usize = 300;

pub const WORLD: Color = Color::srgb(0xe4 as f32 / 255.0, 0xd4 as f32 / 255.0, 0xbe as f32 / 255.0);
pub const INK: Color = Color::srgb(0x3b as f32 / 255.0, 0x2a as f32 / 255.0, 0x1c as f32 / 255.0);
pub const ACCENT: Color = Color::srgb(0xe8 as f32 / 255.0, 0x11 as f32 / 255.0, 0xa0 as f32 / 255.0);
const INK_HEX: &str = "#3b2a1c";

/// `[x, y, z, width, height, depth]`: solid boxes that hide whatever is behind them
const SOLIDS: [[f32; 6]; 1] = [[0.0, 111.5, -16.0, 40.0, 40.0, 0.25]];
/// +x, -x, +y, -y, +z, -z
const FACE_COLORS: [&str; 6] = ["#c8a984", "#b99a76", "#dcc3a2", "#a98a68", "#c3a47f", "#b49571"];

pub struct SliderDef {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub def: f64,
}

const fn sl(key: &'static str, label: &'static str, min: f64, max: f64, step: f64, def: f64) -> SliderDef {
    SliderDef {
        key,
        label,
        min,
        max,
        step,
        def,
    }
}

pub const SLIDERS: [SliderDef; 19] = [
    sl("damping", "damping", 0.0, 0.5, 0.005, 0.12),
    sl("gravity", "gravity in a bucket", 0.0, 6.0, 0.05, 1.2),
    sl("repel", "repulsion", 0.0, 40.0, 0.5, 8.0),
    sl("range", "repulsion range", 0.0, 12.0, 0.1, 3.0),
    sl("atbash", "atbash asymmetry", 0.0, 8.0, 0.05, 1.0),
    sl("pad", "gap between texts", 0.0, 2.0, 0.05, 0.3),
    sl("groupAttract", "bucket attraction", 0.0, 2.0, 0.01, 0.3),
    sl("groupRepel", "bucket repulsion", 0.0, 400.0, 1.0, 60.0),
    sl("groupAlign", "bucket velocity", 0.0, 2.0, 0.01, 0.3),
    sl("qscale", "bucket affinity width", 1.0, 100.0, 1.0, 12.0),
    sl("anchor", "pull to sum spiral", 0.0, 2.0, 0.01, 0.25),
    sl("ring", "spiral spacing", 1.0, 40.0, 0.5, 10.0),
    sl("depth", "depth per unit of sum", 0.0, 2.0, 0.01, 0.2),
    sl("D", "spread distance", 5.0, 200.0, 1.0, 30.0),
    sl("cool", "cooling", 0.002, 0.1, 0.001, 0.0228),
    sl("substeps", "steps per frame", 0.0, 8.0, 1.0, 2.0),
    sl("memSize", "memory text size", 0.1, 3.0, 0.05, 0.5),
    sl("wrap", "memory text wrap", 16.0, 160.0, 1.0, 60.0),
    sl("minPx", "hide texts under px", 0.0, 8.0, 0.1, 1.2),
];

fn slider_index(key: &str) -> usize {
    SLIDERS.iter().position(|s| s.key == key).expect("known slider")
}

/// writes a slider into the spread's parameters; false when it isn't one of them
fn set_param(p: &mut Params, key: &str, v: f64) -> bool {
    match key {
        "damping" => p.damping = v,
        "gravity" => p.gravity = v,
        "repel" => p.repel = v,
        "range" => p.range = v,
        "atbash" => p.atbash = v,
        "pad" => p.pad = v,
        "groupAttract" => p.group_attract = v,
        "groupRepel" => p.group_repel = v,
        "groupAlign" => p.group_align = v,
        "qscale" => p.qscale = v,
        "anchor" => p.anchor = v,
        "ring" => p.ring = v,
        "depth" => p.depth = v,
        "D" => p.d = v,
        "cool" => p.cool = v,
        _ => return false,
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chips {
    /// gsearch what isn't a number
    pub gsch: bool,
    /// reversed ordinal gematria
    pub rtb: bool,
    /// searches also saturate
    #[serde(rename = "alsoSaturate")]
    pub also_saturate: bool,
    /// new results join the current spread
    #[serde(rename = "accumulate")]
    pub keep: bool,
}

impl Default for Chips {
    fn default() -> Self {
        Chips {
            gsch: true,
            rtb: false,
            also_saturate: true,
            keep: false,
        }
    }
}

/// remem's `wraptxt`: the matches of
/// ``(?=.{1,limit}($|\s))\S.{0,limit-1}(?=\s|$)`` joined by newlines, or the text itself
/// when nothing matches
pub fn wraptxt(txt: &str, limit: usize) -> String {
    let c: Vec<char> = txt.chars().collect();
    let line_break = |ch: char| matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    let limit = limit.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if !c[i].is_whitespace() {
            // the longest run of up to limit characters that ends before a space or the end
            let mut best = None;
            let mut m = 0;
            loop {
                let end = i + 1 + m;
                if end == c.len() || c[end].is_whitespace() {
                    best = Some(m);
                }
                if m + 1 >= limit || end >= c.len() || line_break(c[end]) {
                    break;
                }
                m += 1;
            }
            if let Some(m) = best {
                out.push(c[i..i + 1 + m].iter().collect());
                i += 1 + m;
                continue;
            }
        }
        i += 1;
    }
    if out.is_empty() {
        txt.to_string()
    } else {
        out.join("\n")
    }
}

// ------------------------------------------------------------------ the font

/// the font the texts are drawn with, and its bytes for measuring lines
#[derive(Resource)]
pub struct SlugFont {
    pub handle: Handle<Font>,
    data: Option<Vec<u8>>,
}

impl FromWorld for SlugFont {
    fn from_world(world: &mut World) -> Self {
        SlugFont {
            handle: world.resource::<Fonts>().regular.clone(),
            data: None,
        }
    }
}

/// a text's lines with their widths, in em
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub lines: Vec<(String, f32)>,
    pub half_w: f32,
    pub half_h: f32,
}

impl SlugFont {
    /// forget the measured font, so it's read again (the font was swapped)
    pub fn reset(&mut self) {
        self.data = None;
    }

    /// widths the way bevy_slugtext lays a line out: advances of the characters the font
    /// has a glyph for
    pub fn layout(&self, text: &str) -> Layout {
        let face = self.data.as_ref().and_then(|d| ttf_parser::Face::parse(d, 0).ok());
        let mut lines = Vec::new();
        let mut max_w = 0f32;
        for line in text.split('\n') {
            let w = match &face {
                Some(face) => {
                    let upm = face.units_per_em().max(1) as f32;
                    line.chars()
                        .filter_map(|ch| face.glyph_index(ch))
                        .map(|g| face.glyph_hor_advance(g).map(|a| a as f32).unwrap_or(upm) / upm)
                        .sum()
                }
                None => line.chars().count() as f32 * 0.55,
            };
            max_w = max_w.max(w);
            lines.push((line.to_string(), w));
        }
        let n = lines.len() as f32;
        Layout {
            lines,
            half_w: max_w / 2.0 + 0.1,
            half_h: n * LINE / 2.0,
        }
    }
}

// ------------------------------------------------------------------ components

#[derive(Component)]
pub struct SpaceCam;

#[derive(Component)]
pub struct Solid;

/// a text in the space; memory texts carry their key and a node in the spread
#[derive(Component)]
pub struct SpaceText {
    /// what's shown (wrapped)
    pub text: String,
    /// a memory's own text
    pub raw: String,
    /// world height of one em
    pub size: f32,
    pub color: Color,
    /// the colour as a map stores it
    pub color_hex: String,
    pub layout: Layout,
    /// the memory key, "3,14" or "note:…"
    pub mem: Option<String>,
    /// a spread text waits for the spread to place it
    pub placed: bool,
    pub removed: bool,
    /// the spread node of a removed memory text, for undo
    parked: Option<SNode>,
    /// rebuild the lines
    pub relayout: bool,
    pub underline: Option<Color>,
    pub underline_dirty: bool,
    pub tint: Color,
}

impl SpaceText {
    pub fn own_text(&self) -> &str {
        if self.mem.is_some() { &self.raw } else { &self.text }
    }
}

/// one line of a text, drawn by bevy_slugtext
#[derive(Component)]
pub struct TextLine;

#[derive(Component)]
pub struct UnderlineBar;

// ------------------------------------------------------------------ state

pub struct Drag {
    pub e: Entity,
    pub depth: f32,
    pub ox: f32,
    pub oy: f32,
    pub from: Vec3,
    pub was_pinned: Option<bool>,
}

pub struct Press {
    pub pos: Vec2,
    pub e: Option<Entity>,
    pub at: f64,
}

pub enum Undo {
    /// put a removed text back
    Restore(Entity),
    /// take a new text away
    Remove(Entity),
    Move {
        e: Entity,
        from: Vec3,
        was_pinned: Option<bool>,
    },
    Text {
        e: Entity,
        from: String,
    },
    Pin {
        e: Entity,
        was: bool,
    },
    /// saturate a forgotten memory again under its key
    Resaturate {
        key: Vec<u8>,
        text: String,
    },
}

/// the plane the spread is laid out in: where the eye was at the last search
#[derive(Clone, Copy)]
pub struct Frame {
    pub eye: Vec3,
    pub r: Vec3,
    pub u: Vec3,
    pub f: Vec3,
    pub set: bool,
}

/// what the line over the terminal says
#[derive(Clone, Debug, Default)]
pub enum Qline {
    #[default]
    None,
    Gsearch {
        n: u32,
        rev: bool,
    },
    Key(String),
    List(String),
    Search(String),
    Maps(Vec<String>),
}

#[derive(Default)]
pub struct Redraw {
    pub qline: bool,
    pub termrow: bool,
    pub metrics: bool,
    pub toast: bool,
    pub panels: bool,
    pub opts: bool,
    pub help: bool,
    pub term_style: bool,
}

#[derive(Resource)]
pub struct Space {
    pub spread: Spread,
    pub mem_index: HashMap<String, Entity>,
    pub values: [f64; SLIDERS.len()],
    pub chips: Chips,
    pub frame: Frame,
    // flight
    pub speed: f32,
    pub roll_vel: f32,
    pub leveling: bool,
    pub vel: Vec3,
    pub looking: bool,
    // pointer
    pub cursor: Option<Vec2>,
    pub over_ui: bool,
    pub drag: Option<Drag>,
    pub press: Option<Press>,
    pub last_click: Option<(Vec2, f64)>,
    pub hover: Option<Entity>,
    pub selected: Option<Entity>,
    pub shown_for: Option<Entity>,
    // the terminal
    pub term: String,
    pub set_term: Option<String>,
    pub select_term: bool,
    pub editing: Option<(Entity, String)>,
    pub hist: Vec<String>,
    pub h_idx: usize,
    pub list_id: Option<Ref>,
    pub list_candidate: Option<Ref>,
    pub qline: Qline,
    pub focus_term: bool,
    pub blur_term: bool,
    // the rest
    pub physics_paused: bool,
    pub undo: Vec<Undo>,
    pub move_undo: Option<(Entity, f64)>,
    pub toast: Option<(String, f64)>,
    pub show_help: bool,
    pub show_hud: bool,
    pub show_opts: bool,
    pub show_start: bool,
    pub address: String,
    pub loaded: bool,
    /// window size in logical pixels, and its height in physical ones
    pub view: Vec2,
    pub view_h: f32,
    pub redraw: Redraw,
}

impl Default for Space {
    fn default() -> Self {
        Space {
            spread: Spread::new(Params::default()),
            mem_index: HashMap::new(),
            values: std::array::from_fn(|i| SLIDERS[i].def),
            chips: Chips::default(),
            frame: Frame {
                eye: Vec3::ZERO,
                r: Vec3::X,
                u: Vec3::Y,
                f: Vec3::NEG_Z,
                set: false,
            },
            speed: SPEED,
            roll_vel: 0.0,
            leveling: false,
            vel: Vec3::ZERO,
            looking: false,
            cursor: None,
            over_ui: false,
            drag: None,
            press: None,
            last_click: None,
            hover: None,
            selected: None,
            shown_for: None,
            term: String::new(),
            set_term: None,
            select_term: false,
            editing: None,
            hist: Vec::new(),
            h_idx: 0,
            list_id: None,
            list_candidate: None,
            qline: Qline::None,
            focus_term: false,
            blur_term: false,
            physics_paused: false,
            undo: Vec::new(),
            move_undo: None,
            toast: None,
            show_help: false,
            show_hud: true,
            show_opts: false,
            show_start: false,
            address: String::new(),
            loaded: false,
            view: Vec2::new(1200.0, 800.0),
            view_h: 800.0,
            redraw: Redraw::default(),
        }
    }
}

impl Space {
    pub fn get(&self, key: &str) -> f64 {
        self.values[slider_index(key)]
    }
}

/// options for a new text
#[derive(Default)]
pub struct LabelOpts {
    pub size: Option<f32>,
    pub color: Option<String>,
    pub mem: Option<String>,
    /// a memory text with a place of its own (from a map) is pinned there
    pub pinned: bool,
}

/// `[text, x, y, z, {size, color, key}]`
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Row(pub String, pub f32, pub f32, pub f32, #[serde(default)] pub RowOpts);

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct RowOpts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "key_string", default)]
    pub key: Option<String>,
}

/// map keys were written as "3,14", but older maps may hold a number
fn key_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.map(|v| match v {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    }))
}

fn parse_color(hex: &str) -> Color {
    Srgba::hex(hex.trim_start_matches('#')).map(Color::from).unwrap_or(INK)
}

/// the underline of a bucket: remem's `seedCLR`
pub fn bucket_color(q: i64) -> Color {
    Color::hsl(spread::seed_hue(q), 0.7, 0.45)
}

// ------------------------------------------------------------------ geometry helpers

/// the camera's right, up and forward
pub fn basis(tf: &Transform) -> (Vec3, Vec3, Vec3) {
    (tf.rotation * Vec3::X, tf.rotation * Vec3::Y, tf.rotation * Vec3::NEG_Z)
}

/// the ray through a window position, like three's `Raycaster.setFromCamera`
pub fn ray(tf: &Transform, cursor: Vec2, size: Vec2) -> (Vec3, Vec3) {
    let (r, u, f) = basis(tf);
    let ndc = Vec2::new(cursor.x / size.x * 2.0 - 1.0, -(cursor.y / size.y) * 2.0 + 1.0);
    let tan = (FOV_DEG.to_radians() / 2.0).tan();
    let aspect = size.x / size.y.max(1.0);
    let dir = (f + r * ndc.x * tan * aspect + u * ndc.y * tan).normalize();
    (tf.translation, dir)
}

/// distance along a ray to a solid box, if it hits one
fn hit_solids(o: Vec3, dir: Vec3) -> f32 {
    let mut best = f32::INFINITY;
    for [x, y, z, w, h, d] in SOLIDS {
        let lo = Vec3::new(x - w / 2.0, y - h / 2.0, z - d / 2.0);
        let hi = Vec3::new(x + w / 2.0, y + h / 2.0, z + d / 2.0);
        let inv = dir.recip();
        let t0 = (lo - o) * inv;
        let t1 = (hi - o) * inv;
        let tmin = t0.min(t1).max_element();
        let tmax = t0.max(t1).min_element();
        if tmax >= tmin.max(0.0) && tmin.max(0.0) < best {
            best = tmin.max(0.0);
        }
    }
    best
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// where exports go: Downloads when there is one
fn export_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    match home {
        Some(h) if h.join("Downloads").is_dir() => h.join("Downloads"),
        Some(h) => h,
        None => PathBuf::from("."),
    }
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

// ------------------------------------------------------------------ the space's actions

pub type TextItem = (Entity, &'static mut SpaceText, &'static mut Transform);

/// everything the space's actions need
#[derive(SystemParam)]
pub struct Sp<'w, 's> {
    pub commands: Commands<'w, 's>,
    pub st: ResMut<'w, Space>,
    pub db: Res<'w, Db>,
    pub font: Res<'w, SlugFont>,
    pub time: Res<'w, Time>,
    pub texts: Query<'w, 's, TextItem, Without<SpaceCam>>,
    pub cam: Single<'w, 's, &'static mut Transform, With<SpaceCam>>,
}

impl Sp<'_, '_> {
    pub fn toast(&mut self, msg: impl Into<String>) {
        let until = self.time.elapsed_secs_f64() + 2.4;
        self.st.toast = Some((msg.into(), until));
        self.st.redraw.toast = true;
    }

    fn save_setting<T: Serialize + ?Sized>(&self, name: &str, value: &T) {
        if let Ok(bytes) = serde_json::to_vec(value) {
            let _ = self.db.0.set_setting(name, &bytes);
        }
    }

    pub fn text(&self, e: Entity) -> Option<&SpaceText> {
        self.texts.get(e).ok().map(|(_, t, _)| t)
    }

    pub fn pos(&self, e: Entity) -> Option<Vec3> {
        self.texts.get(e).ok().map(|(_, _, tf)| tf.translation)
    }

    /// puts text in the terminal's box (or the editor's, while editing)
    fn set_box(&mut self, s: impl Into<String>) {
        self.st.set_term = Some(s.into());
    }

    /// the terminal's text
    pub fn set_term(&mut self, s: impl Into<String>) {
        let s = s.into();
        if self.st.editing.is_none() {
            self.set_box(s.clone());
        }
        self.st.term = s;
        self.st.redraw.termrow = true;
    }

    pub fn set_frame(&mut self) {
        let tf = **self.cam;
        let (r, u, f) = basis(&tf);
        self.st.frame = Frame {
            eye: tf.translation,
            r,
            u,
            f,
            set: true,
        };
    }

    fn node_to_world(frame: &Frame, d: f64, n: &SNode) -> Vec3 {
        let s = (d + n.z) / d;
        frame.eye + frame.f * (d + n.z) as f32 + frame.r * (n.x * s) as f32 + frame.u * (n.y * s) as f32
    }

    fn world_to_node(frame: &Frame, d: f64, n: &mut SNode, pos: Vec3) {
        let w = pos - frame.eye;
        let depth = (w.dot(frame.f) as f64).max(0.1);
        n.z = depth - d;
        n.x = w.dot(frame.r) as f64 * d / depth;
        n.y = w.dot(frame.u) as f64 * d / depth;
    }

    pub fn node_index(&self, e: Entity) -> Option<usize> {
        let t = self.text(e)?;
        if t.removed {
            return None;
        }
        self.st.spread.index(t.mem.as_ref()?)
    }

    pub fn pinned(&self, e: Entity) -> Option<bool> {
        self.node_index(e).map(|i| self.st.spread.nodes[i].pinned)
    }

    pub fn set_pinned(&mut self, e: Entity, on: bool) {
        if let Some(i) = self.node_index(e) {
            self.st.spread.nodes[i].pinned = on;
        }
    }

    /// remem's `syncPinned`: pinned texts tell the spread where they are
    fn sync_pinned(&mut self) {
        let (frame, d) = (self.st.frame, self.st.spread.p.d);
        let st = &mut *self.st;
        for n in st.spread.nodes.iter_mut().filter(|n| n.pinned) {
            if let Some(&e) = st.mem_index.get(&n.id)
                && let Ok((_, _, tf)) = self.texts.get(e)
            {
                Self::world_to_node(&frame, d, n, tf.translation);
            }
        }
    }

    /// lays a text out again after its text or size changed, and refits its node
    pub fn relayout(&mut self, e: Entity) {
        let Ok((_, mut t, _)) = self.texts.get_mut(e) else {
            return;
        };
        t.layout = self.font.layout(&t.text);
        t.relayout = true;
        let (hw, hh) = ((t.layout.half_w * t.size) as f64, (t.layout.half_h * t.size) as f64);
        if let Some(i) = self.node_index(e) {
            let n = &mut self.st.spread.nodes[i];
            n.hw = hw;
            n.hh = hh;
            self.st.spread.wake(0.3);
        }
    }

    /// remem's `makeLabel`
    pub fn make_label(&mut self, raw: &str, pos: Vec3, o: LabelOpts) -> Entity {
        let is_mem = o.mem.is_some();
        let size = o.size.unwrap_or(if is_mem {
            self.st.get("memSize") as f32
        } else {
            DEFAULT_SIZE
        });
        let color_hex = o.color.clone().unwrap_or_else(|| INK_HEX.to_string());
        let color = parse_color(&color_hex);
        let text = wraptxt(raw, if is_mem { self.st.get("wrap") as usize } else { 80 });
        let layout = self.font.layout(&text);
        let (hw, hh) = ((layout.half_w * size) as f64, (layout.half_h * size) as f64);
        let label = SpaceText {
            text,
            raw: raw.to_string(),
            size,
            color,
            color_hex,
            layout,
            mem: o.mem.clone(),
            placed: !is_mem || o.pinned,
            removed: false,
            parked: None,
            relayout: true,
            underline: None,
            underline_dirty: true,
            tint: color,
        };
        let e = self
            .commands
            .spawn((label, Transform::from_translation(pos), Visibility::Hidden))
            .id();
        if let Some(k) = o.mem {
            let mut n = Spread::node(&k, raw, hw, hh);
            if !self.st.frame.set {
                self.set_frame();
            }
            if o.pinned {
                n.pinned = true;
                n.placed = true;
                let (frame, d) = (self.st.frame, self.st.spread.p.d);
                Self::world_to_node(&frame, d, &mut n, pos);
            }
            self.st.mem_index.insert(k, e);
            self.st.spread.add(n);
        }
        self.st.redraw.qline = true;
        e
    }

    /// remem's `removeLabel`; the text stays around so undo can bring it back
    pub fn remove_label(&mut self, e: Entity) {
        let st = &mut *self.st;
        for s in [&mut st.selected, &mut st.hover] {
            if *s == Some(e) {
                *s = None;
            }
        }
        let node = self.node_index(e).map(|i| self.st.spread.nodes[i].clone());
        let Ok((_, mut t, _)) = self.texts.get_mut(e) else {
            return;
        };
        if t.removed {
            return;
        }
        t.removed = true;
        if let Some(k) = t.mem.clone() {
            t.parked = node;
            if self.st.mem_index.get(&k) == Some(&e) {
                self.st.mem_index.remove(&k);
            }
            self.st.spread.remove(&k);
        }
        self.st.redraw.qline = true;
    }

    /// remem's `addLabel` for a removed text
    fn add_label_back(&mut self, e: Entity) {
        let Ok((_, mut t, _)) = self.texts.get_mut(e) else {
            return;
        };
        if !t.removed {
            return;
        }
        t.removed = false;
        if let Some(k) = t.mem.clone() {
            let node = t.parked.take();
            self.st.mem_index.insert(k, e);
            if let Some(n) = node {
                self.st.spread.add(n);
            }
        }
        self.st.redraw.qline = true;
    }

    fn despawn_label(&mut self, e: Entity) {
        if let Some(i) = self.node_index(e) {
            let k = self.st.spread.nodes[i].id.clone();
            self.st.spread.remove(&k);
        }
        self.st.mem_index.retain(|_, v| *v != e);
        let st = &mut *self.st;
        for s in [&mut st.selected, &mut st.hover, &mut st.shown_for] {
            if *s == Some(e) {
                *s = None;
            }
        }
        if self.st.editing.as_ref().is_some_and(|(x, _)| *x == e) {
            self.st.editing = None;
            self.st.redraw.term_style = true;
        }
        if self.st.drag.as_ref().is_some_and(|d| d.e == e) {
            self.st.drag = None;
        }
        self.commands.entity(e).despawn();
    }

    /// remem's `clearSpread`: the spread's texts go, the pinned ones too when `all`
    pub fn clear_spread(&mut self, all: bool) {
        let doomed: Vec<Entity> = self
            .st
            .mem_index
            .values()
            .copied()
            .filter(|e| all || !self.pinned(*e).unwrap_or(false))
            .collect();
        for e in doomed {
            self.despawn_label(e);
        }
        self.st.spread.dirty = true;
        self.st.redraw.qline = true;
    }

    /// remem's `clrTxts`: replaces everything with a map's texts
    pub fn clr_txts(&mut self, rows: Vec<Row>) {
        let all: Vec<Entity> = self.texts.iter().map(|(e, _, _)| e).collect();
        for e in all {
            self.despawn_label(e);
        }
        self.st.spread.clear();
        self.st.mem_index.clear();
        self.st.undo.clear();
        for Row(text, x, y, z, o) in rows {
            let pinned = o.key.is_some();
            self.make_label(
                &text,
                Vec3::new(x, y, z),
                LabelOpts {
                    size: o.size,
                    color: o.color,
                    mem: o.key,
                    pinned,
                },
            );
        }
        self.st.redraw.qline = true;
    }

    pub fn record(&mut self, u: Undo) {
        self.st.undo.push(u);
        if self.st.undo.len() > UNDO_MAX {
            self.st.undo.remove(0);
        }
    }

    pub fn undo(&mut self) {
        let Some(u) = self.st.undo.pop() else {
            self.toast("nothing to undo");
            return;
        };
        match u {
            Undo::Restore(e) => self.add_label_back(e),
            Undo::Remove(e) => self.remove_label(e),
            Undo::Move { e, from, was_pinned } => {
                if let Ok((_, _, mut tf)) = self.texts.get_mut(e) {
                    tf.translation = from;
                }
                if let Some(was) = was_pinned {
                    self.set_pinned(e, was);
                    self.node_from_label_at(e, from);
                    self.st.spread.wake(1.0);
                }
            }
            Undo::Text { e, from } => {
                let mem = match self.texts.get_mut(e) {
                    Ok((_, mut t, _)) => {
                        t.text = from.clone();
                        if t.mem.is_some() {
                            t.raw = from.clone();
                        }
                        t.mem.is_some()
                    }
                    Err(_) => false,
                };
                if mem && let Some(i) = self.node_index(e) {
                    self.st.spread.set_text(i, &from);
                }
                self.relayout(e);
            }
            Undo::Pin { e, was } => {
                self.set_pinned(e, was);
                self.st.spread.wake(1.0);
            }
            Undo::Resaturate { key, text } => {
                let label = Ref::Memory(key.clone()).label();
                match self.db.0.restore_memory(&key, &text) {
                    Ok(()) => self.toast(format!("restored {}", label)),
                    Err(e) => self.toast(e),
                }
                self.st.move_undo = None;
                return;
            }
        }
        self.st.move_undo = None;
        self.toast("undone");
    }

    fn node_from_label_at(&mut self, e: Entity, pos: Vec3) {
        if let Some(i) = self.node_index(e) {
            let (frame, d) = (self.st.frame, self.st.spread.p.d);
            Self::world_to_node(&frame, d, &mut self.st.spread.nodes[i], pos);
        }
    }

    // -------------------------------------------------------------- picking and dragging

    /// shown: placed and big enough on screen
    pub fn visible(st: &Space, cam: &Transform, t: &SpaceText, tf: &Transform) -> bool {
        if t.removed || !t.placed {
            return false;
        }
        let (_, _, f) = basis(cam);
        let depth = (tf.translation - cam.translation).dot(f);
        if depth <= NEAR {
            return false;
        }
        let px_per_world = st.view_h * 0.5 / ((FOV_DEG.to_radians() / 2.0).tan() * depth);
        t.size * px_per_world >= st.get("minPx") as f32
    }

    /// remem's `pick`: rectangles in the camera plane, padded by a few pixels, behind no
    /// solid
    pub fn pick(&self, cursor: Vec2) -> Option<Entity> {
        let size = self.st.view;
        let tf = **self.cam;
        let (o, dir) = ray(&tf, cursor, size);
        let (r, u, f) = basis(&tf);
        let df = dir.dot(f);
        let solid_t = hit_solids(o, dir);
        let per_px = 2.0 * (FOV_DEG.to_radians() / 2.0).tan() / size.y.max(1.0);
        let mut best: Option<(Entity, f32)> = None;
        for (e, t, ltf) in self.texts.iter() {
            if !Self::visible(&self.st, &tf, t, ltf) {
                continue;
            }
            let d = ltf.translation - o;
            let depth = d.dot(f);
            if depth < 0.1 || df < 1e-4 {
                continue;
            }
            let tt = depth / df;
            if tt >= solid_t || best.is_some_and(|(_, bt)| tt >= bt) {
                continue;
            }
            let h = o + dir * tt - ltf.translation;
            let pad = 6.0 * per_px * depth + 0.1 * t.size;
            if h.dot(r).abs() <= t.layout.half_w * t.size + pad && h.dot(u).abs() <= t.layout.half_h * t.size + pad {
                best = Some((e, tt));
            }
        }
        best.map(|(e, _)| e)
    }

    pub fn begin_drag(&mut self, e: Entity, cursor: Vec2) {
        let Some(pos) = self.pos(e) else { return };
        let tf = **self.cam;
        let (o, dir) = ray(&tf, cursor, self.st.view);
        let (r, u, f) = basis(&tf);
        let depth = (pos - o).dot(f);
        let t = depth / dir.dot(f).max(1e-4);
        let d = pos - (o + dir * t);
        let was_pinned = self.pinned(e);
        self.st.drag = Some(Drag {
            e,
            depth,
            ox: d.dot(r),
            oy: d.dot(u),
            from: pos,
            was_pinned,
        });
        self.set_pinned(e, true);
    }

    pub fn update_drag(&mut self) {
        let (Some(drag), Some(cursor)) = (self.st.drag.as_ref(), self.st.cursor) else {
            return;
        };
        let tf = **self.cam;
        let (o, dir) = ray(&tf, cursor, self.st.view);
        let (r, u, f) = basis(&tf);
        let t = drag.depth / dir.dot(f).max(1e-4);
        let h = o + dir * t + r * drag.ox + u * drag.oy;
        let e = drag.e;
        if let Ok((_, _, mut ltf)) = self.texts.get_mut(e) {
            ltf.translation = h;
        }
        if self.node_index(e).is_some() {
            self.st.spread.wake(0.3);
        }
    }

    /// ends a drag, returning whether the text moved
    pub fn end_drag(&mut self) -> bool {
        let Some(Drag {
            e, from, was_pinned, ..
        }) = self.st.drag.take()
        else {
            return false;
        };
        let moved = self.pos(e).is_some_and(|p| p.distance_squared(from) > 1e-8);
        if moved {
            self.record(Undo::Move { e, from, was_pinned });
        } else if let Some(was) = was_pinned {
            self.set_pinned(e, was);
        }
        moved
    }

    /// wheel on a text: push it away or pull it near
    pub fn push_pull(&mut self, e: Entity, k: f32) {
        let Some(pos) = self.pos(e) else { return };
        let eye = self.cam.translation;
        let d = pos - eye;
        let len = d.length();
        let nl = (len * k).clamp(0.3, 2000.0);
        let now = self.time.elapsed_secs_f64();
        if !self.st.move_undo.is_some_and(|(x, t)| x == e && now - t <= 0.8) {
            let was_pinned = self.pinned(e);
            self.record(Undo::Move {
                e,
                from: pos,
                was_pinned,
            });
        }
        self.st.move_undo = Some((e, now));
        if self.node_index(e).is_some() {
            self.set_pinned(e, true);
            self.st.spread.wake(0.3);
        }
        if let Ok((_, _, mut tf)) = self.texts.get_mut(e) {
            tf.translation = eye + d * (nl / len.max(1e-6));
        }
    }

    // -------------------------------------------------------------- the box: terminal and editor

    pub fn open_editor(&mut self, e: Entity) {
        let Some(text) = self.text(e).map(|t| t.text.clone()) else {
            return;
        };
        self.open_editor_with(e, text);
    }

    fn open_editor_with(&mut self, e: Entity, text: String) {
        if self.st.editing.is_some() {
            self.close_editor();
        }
        self.st.editing = Some((e, text.clone()));
        self.st.selected = Some(e);
        self.set_box(text);
        self.st.select_term = true;
        self.st.focus_term = true;
        self.st.redraw.term_style = true;
    }

    pub fn close_editor(&mut self) {
        let Some((e, from)) = self.st.editing.take() else {
            return;
        };
        self.st.redraw.term_style = true;
        let term = self.st.term.clone();
        self.set_box(term);
        self.st.redraw.termrow = true;
        self.st.blur_term = true;
        let Some((text, mem)) = self.text(e).map(|t| (t.text.clone(), t.mem.is_some())) else {
            return;
        };
        if text.trim().is_empty() {
            if let Ok((_, mut t, _)) = self.texts.get_mut(e) {
                t.text = from;
            }
            self.relayout(e);
            self.remove_label(e);
            self.record(Undo::Restore(e));
            self.toast("text deleted");
        } else if text != from {
            if mem {
                if let Ok((_, mut t, _)) = self.texts.get_mut(e) {
                    t.raw = text.clone();
                }
                if let Some(i) = self.node_index(e) {
                    self.st.spread.set_text(i, &text);
                }
            }
            self.record(Undo::Text { e, from });
        }
    }

    /// the box changed: the edited text follows it, or the terminal's sidecar does
    pub fn box_input(&mut self, value: String) {
        if let Some((e, _)) = self.st.editing.clone() {
            if let Ok((_, mut t, _)) = self.texts.get_mut(e) {
                if t.text == value {
                    return;
                }
                t.text = value;
            }
            self.relayout(e);
            self.st.redraw.metrics = true;
        } else if self.st.term != value {
            self.st.term = value;
            self.st.redraw.termrow = true;
        }
    }

    fn push_history(&mut self, v: &str) {
        if self.st.hist.last().map(|s| s.as_str()) != Some(v) {
            self.st.hist.push(v.to_string());
        }
        let n = self.st.hist.len();
        if n > HISTORY_MAX {
            self.st.hist.drain(0..n - HISTORY_MAX);
        }
        self.st.h_idx = self.st.hist.len();
        self.save_setting("space.history", &self.st.hist);
    }

    /// ↑ / ↓ in the terminal
    pub fn history(&mut self, up: bool) {
        if self.st.term.contains('\n') || self.st.hist.is_empty() {
            return;
        }
        let n = self.st.hist.len() as isize;
        let i = (self.st.h_idx as isize + if up { -1 } else { 1 }).clamp(0, n) as usize;
        self.st.h_idx = i;
        let v = self.st.hist.get(i).cloned().unwrap_or_default();
        self.set_term(v);
    }

    // -------------------------------------------------------------- the memory base

    /// remem's `begin`: a new search replaces the spread unless "keep" is on, and lays it
    /// out from where you are
    pub fn begin(&mut self, head: Qline) {
        if !self.st.chips.keep || !self.st.frame.set {
            if !self.st.chips.keep {
                self.clear_spread(false);
            }
            self.set_frame();
        }
        self.st.qline = head;
        self.st.redraw.qline = true;
    }

    pub fn add_mem(&mut self, r: &Ref, text: &str) -> Entity {
        let k = r.label();
        if let Some(&e) = self.st.mem_index.get(&k) {
            return e;
        }
        self.make_label(
            text,
            Vec3::ZERO,
            LabelOpts {
                mem: Some(k),
                ..default()
            },
        )
    }

    fn add_hits(&mut self, hits: Vec<Hit>) {
        for h in hits {
            self.add_mem(&h.r, &h.text);
        }
    }

    fn set_address(&mut self, a: String) {
        if self.st.address != a {
            self.st.address = a;
            self.save_setting("space.address", &self.st.address);
        }
    }

    pub fn spread_g(&mut self, n: u32, rev: bool) {
        self.begin(Qline::Gsearch { n, rev });
        let bucket = if rev { Bucket::Reversed } else { Bucket::Ordinal };
        match self.db.0.gsearch(bucket, &BucketKey::Sum(n)) {
            Ok(hits) => self.add_hits(hits),
            Err(e) => self.toast(e),
        }
    }

    pub fn go(&mut self, n: u32, rev: bool) {
        self.set_address(format!("{}{}", if rev { "r" } else { "" }, n));
        self.spread_g(n, rev);
    }

    /// remem's `envale`: a value into the terminal, and gsearch it
    pub fn envale(&mut self, v: u32, rev: bool) {
        self.set_term(v.to_string());
        self.go(v, rev);
    }

    pub fn set_reversed(&mut self, on: bool) {
        self.st.chips.rtb = on;
        self.st.spread.p.reversed = on;
        self.st.spread.dirty = true;
        self.save_setting("space.chips", &self.st.chips);
        self.st.redraw.termrow = true;
    }

    pub fn set_chips(&mut self, chips: Chips) {
        if chips.rtb != self.st.chips.rtb {
            self.set_reversed(chips.rtb);
        }
        self.st.chips = chips;
        self.save_setting("space.chips", &self.st.chips);
        self.st.redraw.termrow = true;
    }

    fn sum_of(&self, text: &str) -> u32 {
        let (s, r) = spread::sums(text);
        (if self.st.chips.rtb { r } else { s }).clamp(0, u32::MAX as i64) as u32
    }

    fn saturate_and_add(&mut self, v: &str) {
        match self.db.0.saturate(v) {
            Ok((k, _)) => {
                let r = Ref::Memory(k);
                self.toast(format!("saturated {}", r.label()));
                self.add_mem(&r, v);
            }
            Err(e) => self.toast(e),
        }
    }

    /// the terminal's Enter
    pub fn run(&mut self, raw: &str, ctrl: bool, shift: bool) {
        let v = raw.trim().to_string();
        if v.is_empty() {
            return;
        }
        self.push_history(raw);
        let mut words = v.split_whitespace();
        let word = words.next().unwrap_or_default().to_string();
        let rest: Vec<&str> = words.collect();
        if COMMANDS.contains(&word.as_str()) && (rest.is_empty() || word == "Save") {
            self.command(&word, &rest.join(" "));
            return;
        }
        if let Some(addr) = v.strip_prefix('#') {
            self.route(addr.trim());
            return;
        }
        let rev = self.st.chips.rtb;
        if shift {
            match Ref::parse(&v) {
                Some(r) => {
                    self.begin(Qline::Key(r.label()));
                    match self.db.0.remember(&r) {
                        Ok(h) => {
                            self.add_mem(&h.r, &h.text);
                        }
                        Err(e) => self.toast(e),
                    }
                }
                None => self.toast("that's not a key"),
            }
            return;
        }
        if let Ok(n) = v.parse::<u32>() {
            self.go(n, rev);
            return;
        }
        let sum = self.sum_of(&v);
        if ctrl {
            self.go(sum, rev);
            self.saturate_and_add(&v);
            return;
        }
        if self.st.chips.gsch {
            self.go(sum, rev);
            match self.db.0.find_exact(&v) {
                Ok(Some(k)) => {
                    self.add_mem(&Ref::Memory(k), &v);
                }
                Ok(None) if self.st.chips.also_saturate => self.saturate_and_add(&v),
                Ok(None) => {}
                Err(e) => self.toast(e),
            }
            return;
        }
        self.begin(Qline::Search(v.clone()));
        match self.db.0.search_with_variations(&v) {
            Ok(hits) => {
                let none = hits.is_empty();
                self.add_hits(hits);
                if self.st.chips.also_saturate && matches!(self.db.0.find_exact(&v), Ok(None)) {
                    self.saturate_and_add(&v);
                } else if none {
                    self.toast(format!("didn't find anything for {}", v));
                }
            }
            Err(e) => self.toast(e),
        }
    }

    /// the terminal's Delete: forget the memory under the key typed
    pub fn forget_typed(&mut self, raw: &str) {
        match Ref::parse(raw) {
            Some(Ref::Memory(k)) => match self.db.0.forget(&k) {
                Ok(text) => {
                    let kstr = Ref::Memory(k.clone()).label();
                    let short: String = text.chars().take(60).collect();
                    self.toast(format!(
                        "forgot {}: {} · Ctrl+Z outside the terminal restores it",
                        kstr, short
                    ));
                    if let Some(&e) = self.st.mem_index.get(&kstr) {
                        self.remove_label(e);
                        self.record(Undo::Restore(e));
                    }
                    self.record(Undo::Resaturate { key: k, text });
                }
                Err(e) => self.toast(e),
            },
            Some(Ref::Note(_)) => self.toast("notes are deleted from the editor"),
            None => {}
        }
    }

    fn need_list(&mut self) -> Option<Ref> {
        if self.st.list_id.is_none() {
            self.toast("no list: Shift+click a memory text to set it");
        }
        self.st.list_id.clone()
    }

    fn need_cand(&mut self) -> Option<Ref> {
        if self.st.list_candidate.is_none() {
            self.toast("no candidate: Ctrl+click a memory text to set it");
        }
        self.st.list_candidate.clone()
    }

    fn command(&mut self, word: &str, rest: &str) {
        match word {
            "Enlist" => {
                if let (Some(l), Some(c)) = (self.need_list(), self.need_cand()) {
                    match self.db.0.enlist(&l, &c) {
                        Ok(()) => self.toast(format!("enlisted {} in {}", c.label(), l.label())),
                        Err(e) => self.toast(e),
                    }
                }
            }
            "List" => {
                if let Some(l) = self.need_list() {
                    self.begin(Qline::List(l.label()));
                    match self.db.0.list(&l) {
                        Ok(hits) => self.add_hits(hits),
                        Err(e) => self.toast(e),
                    }
                }
            }
            "Delist" => {
                if let (Some(l), Some(c)) = (self.need_list(), self.need_cand()) {
                    match self.db.0.delist(&l, &c) {
                        Ok(()) => self.toast(format!("delisted {} from {}", c.label(), l.label())),
                        Err(e) => self.toast(e),
                    }
                }
            }
            "Unlist" => {
                if let Some(l) = self.need_list() {
                    match self.db.0.unlist(&l) {
                        Ok(n) => self.toast(format!("unlisted {} ({} members)", l.label(), n)),
                        Err(e) => self.toast(e),
                    }
                }
            }
            "Note" | "Notes" | "Unnote" => self.toast("weum's notes on memories aren't part of anend; lists are"),
            "Count" => match self.db.0.memory_count() {
                Ok(n) => self.toast(format!("{} memories", n)),
                Err(e) => self.toast(e),
            },
            "Backup" => {
                let out = self.db.0.all_memories().and_then(|all| {
                    let path = export_dir().join("all-of-it.json");
                    std::fs::write(&path, serde_json::to_vec(&all).unwrap_or_default())
                        .map(|_| (all.len(), path))
                        .map_err(|e| e.to_string())
                });
                match out {
                    Ok((n, path)) => self.toast(format!("{} memories backed up to {}", n, path.display())),
                    Err(e) => self.toast(e),
                }
            }
            "Clear" => self.clear_spread(false),
            "ClearAll" => self.clear_spread(true),
            "Settle" => self.settle(),
            "Maps" => self.list_maps(),
            "Save" => self.save_map(rest),
            _ => {}
        }
    }

    pub fn settle(&mut self) {
        if self.st.spread.dirty {
            self.st.spread.rekey();
            self.paint();
        }
        self.sync_pinned();
        self.st.spread.wake(1.0);
        let i = self.st.spread.settle(usize::MAX, 400);
        self.toast(format!("settled in {} steps", i));
    }

    /// remem's `paintBuckets`: a memory text sharing its bucket is underlined in its colour
    pub fn paint(&mut self) {
        let st = &*self.st;
        let mut colors: Vec<(Entity, Option<Color>)> = Vec::new();
        for n in &st.spread.nodes {
            if let (Some(&e), Some(b)) = (st.mem_index.get(&n.id), st.spread.order.get(n.bucket)) {
                colors.push((e, (b.n >= 2).then(|| bucket_color(b.q))));
            }
        }
        for (e, c) in colors {
            if let Ok((_, mut t, _)) = self.texts.get_mut(e)
                && t.underline != c
            {
                t.underline = c;
                t.underline_dirty = true;
            }
        }
        self.st.redraw.qline = true;
        self.st.redraw.metrics = true;
    }

    // -------------------------------------------------------------- maps

    fn rows(&self) -> Vec<Row> {
        self.texts
            .iter()
            .filter(|(_, t, _)| !t.removed)
            .map(|(_, t, tf)| {
                let p = tf.translation;
                Row(
                    t.own_text().to_string(),
                    round2(p.x),
                    round2(p.y),
                    round2(p.z),
                    RowOpts {
                        size: (t.size != DEFAULT_SIZE).then_some(t.size),
                        color: (t.color_hex != INK_HEX).then(|| t.color_hex.clone()),
                        key: t.mem.clone(),
                    },
                )
            })
            .collect()
    }

    /// `spread-r123` for a bucket, the map's name for a map
    fn hash_name(&self) -> Option<String> {
        let a = self.st.address.trim();
        if a.is_empty() {
            None
        } else if let Some((n, rev)) = parse_bucket(a) {
            Some(format!("spread-{}{}", if rev { "r" } else { "" }, n))
        } else {
            Some(a.to_string())
        }
    }

    /// X: every text to a JSON file
    pub fn export(&mut self) {
        let rows = self.rows();
        let name = self.hash_name().unwrap_or_else(|| format!("{}-textuality", now_ms()));
        let path = export_dir().join(format!("{}.json", name));
        match serde_json::to_vec_pretty(&rows)
            .map_err(|e| e.to_string())
            .and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()))
        {
            Ok(()) => self.toast(format!("{} texts to {}", rows.len(), path.display())),
            Err(e) => self.toast(e),
        }
    }

    /// Shift+X, `Save name`: the map into the database
    pub fn save_map(&mut self, name: &str) {
        let name = if name.trim().is_empty() {
            self.hash_name().unwrap_or_else(|| "untitled".into())
        } else {
            name.trim().to_string()
        };
        let ok =
            (1..=96).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !ok {
            self.toast("map name: 1 to 96 of letters, digits, _ and -");
            return;
        }
        let rows = self.rows();
        self.save_setting(&format!("map:{}", name), &rows);
        self.set_address(name.clone());
        self.toast(format!("saved {} · {} texts", name, rows.len()));
    }

    pub fn list_maps(&mut self) {
        match self.db.0.setting_names("map:") {
            Ok(names) => {
                self.st.qline = Qline::Maps(names);
                self.st.redraw.qline = true;
            }
            Err(e) => self.toast(e),
        }
    }

    /// `#123` an ordinal bucket, `#r123` a reversed one, `#name` a map
    pub fn route(&mut self, addr: &str) {
        if let Some((n, rev)) = parse_bucket(addr) {
            if self.st.chips.rtb != rev {
                self.set_reversed(rev);
            }
            self.set_address(addr.to_string());
            self.spread_g(n, rev);
            return;
        }
        let rows = self
            .db
            .0
            .setting(&format!("map:{}", addr))
            .ok()
            .flatten()
            .and_then(|b| serde_json::from_slice::<Vec<Row>>(&b).ok());
        match rows {
            Some(rows) => {
                if !self.st.frame.set {
                    self.set_frame();
                }
                self.clr_txts(rows);
                self.set_address(addr.to_string());
                self.st.qline = Qline::None;
            }
            None => self.toast(format!("there's no map named {}", addr)),
        }
    }

    // -------------------------------------------------------------- clicks

    pub fn click_label(&mut self, e: Entity, ctrl: bool, shift: bool) {
        let Some((mem, own)) = self.text(e).map(|t| (t.mem.clone(), t.own_text().to_string())) else {
            return;
        };
        if let Some(k) = &mem {
            if ctrl {
                self.st.list_candidate = Ref::parse(k);
                self.st.redraw.termrow = true;
                self.toast(format!("candidate {}", k));
                return;
            }
            if shift {
                self.st.list_candidate = None;
                self.st.list_id = Ref::parse(k);
                self.st.redraw.termrow = true;
                self.toast(format!("list {}", k));
                return;
            }
        }
        if self.st.editing.is_some() {
            return;
        }
        self.set_term(own);
    }

    pub fn double_click(&mut self, cursor: Vec2) {
        match self.pick(cursor) {
            Some(e) => match self.text(e).and_then(|t| t.mem.clone()) {
                Some(k) => self.set_term(k),
                None => self.open_editor(e),
            },
            None => {
                let (_, dir) = ray(&self.cam, cursor, self.st.view);
                self.spawn(dir, 6.0);
            }
        }
    }

    pub fn spawn(&mut self, dir: Vec3, dist: f32) {
        let p = self.cam.translation + dir * dist;
        let e = self.make_label("text", p, LabelOpts::default());
        self.record(Undo::Remove(e));
        self.open_editor_with(e, "text".into());
    }

    pub fn spawn_ahead(&mut self) {
        let (_, _, f) = basis(&self.cam);
        self.spawn(f, 6.0);
    }

    /// Ctrl+right click: gsearch ⌊√(s·r)⌋
    pub fn jump(&mut self, e: Entity) {
        let Some(text) = self.text(e).map(|t| t.own_text().to_string()) else {
            return;
        };
        let (s, r) = spread::sums(&text);
        let rev = self.st.chips.rtb;
        self.envale(((s * r) as f64).sqrt().floor() as u32, rev);
    }

    pub fn back_to_start(&mut self) {
        **self.cam = Transform::from_translation(START_POS).looking_at(START_LOOK, Vec3::Y);
        self.st.vel = Vec3::ZERO;
        self.st.roll_vel = 0.0;
        self.st.leveling = false;
    }

    /// a slider moved
    pub fn set_slider(&mut self, key: &str, v: f64) {
        let i = slider_index(key);
        self.st.values[i] = v;
        if set_param(&mut self.st.spread.p, key, v) {
            if key == "ring" || key == "depth" {
                self.st.spread.dirty = true;
            }
            self.st.spread.wake(1.0);
        }
        let mems: Vec<Entity> = self.st.mem_index.values().copied().collect();
        if key == "memSize" {
            for e in &mems {
                if let Ok((_, mut t, _)) = self.texts.get_mut(*e) {
                    t.size = v as f32;
                }
                self.relayout(*e);
            }
        }
        if key == "wrap" {
            for e in &mems {
                if let Ok((_, mut t, _)) = self.texts.get_mut(*e) {
                    t.text = wraptxt(&t.raw, v as usize);
                }
                self.relayout(*e);
            }
        }
        let map: HashMap<&str, f64> = SLIDERS.iter().zip(self.st.values).map(|(s, v)| (s.key, v)).collect();
        self.save_setting("space.sliders", &map);
    }

    /// the first time the space opens: settings, history, and the last address
    fn load(&mut self) {
        self.st.loaded = true;
        let get = |name: &str| self.db.0.setting(name).ok().flatten();
        if let Some(map) = get("space.sliders").and_then(|b| serde_json::from_slice::<HashMap<String, f64>>(&b).ok()) {
            for (i, s) in SLIDERS.iter().enumerate() {
                if let Some(v) = map.get(s.key) {
                    self.st.values[i] = *v;
                }
            }
        }
        if let Some(c) = get("space.chips").and_then(|b| serde_json::from_slice::<Chips>(&b).ok()) {
            self.st.chips = c;
        }
        if let Some(h) = get("space.history").and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok()) {
            self.st.hist = h;
        }
        self.st.h_idx = self.st.hist.len();
        let first = get("space.seen").is_none();
        if first {
            let _ = self.db.0.set_setting("space.seen", b"1");
        }
        self.st.show_help = first;
        self.st.show_start = first;
        let values = self.st.values;
        for (s, v) in SLIDERS.iter().zip(values) {
            set_param(&mut self.st.spread.p, s.key, v);
        }
        self.st.spread.p.reversed = self.st.chips.rtb;
        self.st.redraw = Redraw {
            qline: true,
            termrow: true,
            metrics: true,
            toast: true,
            panels: true,
            opts: true,
            help: true,
            term_style: true,
        };
        if let Some(a) = get("space.address").and_then(|b| serde_json::from_slice::<String>(&b).ok())
            && !a.is_empty()
        {
            self.route(&a);
        }
    }
}

pub const COMMANDS: [&str; 15] = [
    "Enlist", "List", "Delist", "Unlist", "Note", "Notes", "Unnote", "Count", "Backup", "Clear", "ClearAll", "Settle",
    "Maps", "Save", "Help",
];

/// `123` → (123, ordinal), `r123` → (123, reversed)
pub fn parse_bucket(a: &str) -> Option<(u32, bool)> {
    let (rev, digits) = match a.strip_prefix('r') {
        Some(d) => (true, d),
        None => (false, a),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().map(|n| (n, rev))
}

// ------------------------------------------------------------------ systems

/// the 3D camera and the solids; the camera only renders in the space
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            is_active: false,
            clear_color: ClearColorConfig::Custom(WORLD),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: FOV_DEG.to_radians(),
            near: NEAR,
            far: FAR,
            ..default()
        }),
        Tonemapping::None,
        Transform::from_translation(START_POS).looking_at(START_LOOK, Vec3::Y),
        SpaceCam,
    ));
    use std::f32::consts::FRAC_PI_2;
    for [x, y, z, w, h, d] in SOLIDS {
        // +x, -x, +y, -y, +z, -z
        let faces = [
            (Vec3::new(w / 2.0, 0.0, 0.0), Quat::from_rotation_y(FRAC_PI_2), (d, h)),
            (Vec3::new(-w / 2.0, 0.0, 0.0), Quat::from_rotation_y(-FRAC_PI_2), (d, h)),
            (Vec3::new(0.0, h / 2.0, 0.0), Quat::from_rotation_x(-FRAC_PI_2), (w, d)),
            (Vec3::new(0.0, -h / 2.0, 0.0), Quat::from_rotation_x(FRAC_PI_2), (w, d)),
            (Vec3::new(0.0, 0.0, d / 2.0), Quat::IDENTITY, (w, h)),
            (
                Vec3::new(0.0, 0.0, -d / 2.0),
                Quat::from_rotation_y(std::f32::consts::PI),
                (w, h),
            ),
        ];
        for ((offset, rot, (a, b)), color) in faces.into_iter().zip(FACE_COLORS) {
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(a, b))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: parse_color(color),
                    unlit: true,
                    ..default()
                })),
                Transform::from_translation(Vec3::new(x, y, z) + offset).with_rotation(rot),
                Solid,
            ));
        }
    }
}

/// reads the font's bytes once it has loaded, and lays every text out with them
pub fn load_font(
    mut font: ResMut<SlugFont>,
    fonts: Res<Assets<Font>>,
    mut texts: Query<&mut SpaceText>,
    mut st: ResMut<Space>,
) {
    if font.data.is_some() {
        return;
    }
    let Some(f) = fonts.get(&font.handle) else { return };
    font.data = Some(f.data.data().to_vec());
    for mut t in texts.iter_mut() {
        t.layout = font.layout(&t.text);
        t.relayout = true;
        let (hw, hh) = ((t.layout.half_w * t.size) as f64, (t.layout.half_h * t.size) as f64);
        if let Some(k) = t.mem.clone()
            && let Some(i) = st.spread.index(&k)
        {
            st.spread.nodes[i].hw = hw;
            st.spread.nodes[i].hh = hh;
        }
    }
    st.spread.wake(0.3);
}

/// the space opens and closes: the 3D camera, the UI camera's clear, the window's root
pub fn apply_space_route(
    session: Res<Session>,
    mut last: Local<Option<Route>>,
    mut cams: Query<(&mut Camera, Has<SpaceCam>)>,
    root: Single<Entity, With<crate::ui::AppRoot>>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut override_cursor: ResMut<OverrideCursor>,
    mut st: ResMut<Space>,
    mut commands: Commands,
) {
    if *last == Some(session.route) {
        return;
    }
    *last = Some(session.route);
    let on = session.route == Route::Space;
    for (mut cam, space) in cams.iter_mut() {
        if space {
            cam.is_active = on;
        } else {
            cam.clear_color = if on {
                ClearColorConfig::None
            } else {
                ClearColorConfig::Default
            };
        }
    }
    if on {
        commands
            .entity(*root)
            .remove::<bevy::feathers::theme::ThemeBackgroundColor>()
            .insert(BackgroundColor(Color::NONE));
        st.redraw.panels = true;
    } else {
        commands
            .entity(*root)
            .insert(bevy::feathers::theme::ThemeBackgroundColor(
                bevy::feathers::tokens::WINDOW_BG,
            ));
        st.looking = false;
        st.drag = None;
        st.press = None;
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
        override_cursor.0 = None;
    }
}

/// loads the space's settings and last address the first time it opens
pub fn space_enter(session: Res<Session>, mut sp: Sp) {
    if session.route == Route::Space && !sp.st.loaded {
        sp.load();
    }
}

/// keys, buttons and the wheel
pub fn space_input(
    mut sp: Sp,
    session: Res<Session>,
    mut fired: MessageReader<Fired>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    hover_map: Res<HoverMap>,
    window: Single<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    windows: Query<(), With<Window>>,
    focus: Res<InputFocus>,
    editables: Query<(), With<EditableText>>,
    mut clipboard: ResMut<bevy::clipboard::Clipboard>,
    mut remap: ResMut<crate::keys::Remap>,
) {
    if session.route != Route::Space {
        fired.clear();
        return;
    }
    let (window, mut cursor_opts) = window.into_inner();
    sp.st.view = Vec2::new(window.width(), window.height());
    sp.st.view_h = window.physical_height() as f32;
    let cursor = window.cursor_position();
    if cursor.is_some() || !sp.st.looking {
        sp.st.cursor = cursor;
    }
    // the window itself counts as hit when nothing else is under the pointer
    sp.st.over_ui = hover_map
        .get(&PointerId::Mouse)
        .is_some_and(|m| m.keys().any(|e| !windows.contains(*e)));
    let over_ui = sp.st.over_ui;
    let typing = focus.get().is_some_and(|e| editables.contains(e));
    let (ctrl, shift, _) = modifiers(&keys);

    if sp.st.show_start && (keys.get_just_pressed().next().is_some() || mouse.get_just_pressed().next().is_some()) {
        sp.st.show_start = false;
        sp.st.redraw.panels = true;
    }

    for Fired(action) in fired.read() {
        let st = &mut sp.st;
        match action {
            Action::Level => st.leveling = true,
            Action::BackToStart => sp.back_to_start(),
            Action::Help => {
                st.show_help = !st.show_help;
                st.redraw.panels = true;
            }
            Action::Hud => {
                st.show_hud = !st.show_hud;
                st.redraw.panels = true;
            }
            Action::Sliders => {
                st.show_opts = !st.show_opts;
                st.redraw.panels = true;
            }
            Action::Export => sp.export(),
            Action::SaveMap => sp.save_map(""),
            Action::NewText => sp.spawn_ahead(),
            Action::Slower => st.speed = (st.speed / 1.25).max(0.2),
            Action::Faster => st.speed = (st.speed * 1.25).min(400.0),
            Action::EditText => match st.selected {
                Some(e) => sp.open_editor(e),
                None => st.focus_term = true,
            },
            Action::SpaceTerm => st.focus_term = true,
            Action::Reversed => {
                let on = !st.chips.rtb;
                sp.set_reversed(on);
                sp.toast(if on { "reversed" } else { "ordinal" });
            }
            Action::Pause => {
                st.physics_paused = !st.physics_paused;
                let msg = if st.physics_paused {
                    "physics paused"
                } else {
                    "physics running"
                };
                sp.toast(msg);
            }
            Action::Settle => sp.settle(),
            Action::Pin => {
                if let Some(e) = st.selected
                    && let Some(was) = sp.pinned(e)
                {
                    sp.set_pinned(e, !was);
                    sp.record(Undo::Pin { e, was });
                    sp.st.spread.wake(1.0);
                    sp.toast(if was { "unpinned" } else { "pinned" });
                }
            }
            Action::SideSum | Action::SideReversedSum => {
                let (s, r) = spread::sums(&st.term);
                let v = if *action == Action::SideSum { s } else { r };
                sp.envale(v.clamp(0, u32::MAX as i64) as u32, false);
            }
            Action::DeleteText => delete_selected(&mut sp),
            Action::CopyText => {
                if let Some(text) = st.selected.and_then(|e| sp.text(e)).map(|t| t.own_text().to_string()) {
                    let ok = clipboard.set_text(text).is_ok();
                    sp.toast(if ok { "copied" } else { "copy failed" });
                }
            }
            Action::Undo => sp.undo(),
            Action::SpaceKeys => {
                remap.open = !remap.open;
                remap.changed = true;
            }
            _ => {}
        }
    }
    if !typing && !remap.open {
        if keys.just_pressed(KeyCode::Escape) {
            sp.st.selected = None;
        }
        if keys.just_pressed(KeyCode::Backspace) && !ctrl {
            delete_selected(&mut sp);
        }
    }

    // the right button looks (Ctrl: gsearch ⌊√(s·r)⌋ of the text under it)
    if mouse.just_pressed(MouseButton::Right) && !over_ui {
        sp.close_editor();
        sp.st.blur_term = true;
        let picked = if ctrl { cursor.and_then(|c| sp.pick(c)) } else { None };
        match picked {
            Some(e) => sp.jump(e),
            None => {
                sp.st.looking = true;
                cursor_opts.grab_mode = CursorGrabMode::Locked;
                cursor_opts.visible = false;
            }
        }
    }
    if mouse.just_released(MouseButton::Right) && sp.st.looking {
        sp.st.looking = false;
        cursor_opts.grab_mode = CursorGrabMode::None;
        cursor_opts.visible = true;
    }

    if mouse.just_pressed(MouseButton::Left)
        && !over_ui
        && !sp.st.looking
        && let Some(c) = cursor
    {
        sp.close_editor();
        sp.st.blur_term = true;
        let picked = sp.pick(c);
        sp.st.selected = picked;
        let at = sp.time.elapsed_secs_f64();
        sp.st.press = Some(Press { pos: c, e: picked, at });
        if let Some(e) = picked {
            sp.begin_drag(e, c);
        }
    }
    if mouse.just_released(MouseButton::Left) {
        let pr = sp.st.press.take();
        let moved = sp.end_drag();
        if let (Some(pr), Some(c)) = (pr, cursor)
            && c.distance(pr.pos) < 5.0
        {
            if let Some(e) = pr.e
                && !moved
            {
                sp.click_label(e, ctrl, shift);
            }
            let now = sp.time.elapsed_secs_f64();
            let double = sp
                .st
                .last_click
                .is_some_and(|(p, t)| now - t < 0.45 && p.distance(c) < 5.0);
            if double {
                sp.st.last_click = None;
                if !moved {
                    sp.double_click(c);
                }
            } else {
                sp.st.last_click = Some((c, pr.at.max(now)));
            }
        }
    }

    // the wheel: a dragged text's depth, a text's distance, or the flight speed
    if !over_ui && scroll.delta.y != 0.0 {
        let dy = match scroll.unit {
            MouseScrollUnit::Line => -scroll.delta.y * 100.0,
            MouseScrollUnit::Pixel => -scroll.delta.y,
        };
        let k = (-dy * 0.0015).exp();
        if let Some(d) = sp.st.drag.as_mut() {
            d.depth = (d.depth * k).clamp(0.3, 2000.0);
            d.ox *= k;
            d.oy *= k;
        } else if let Some(e) = sp.st.hover {
            sp.push_pull(e, k);
        } else {
            sp.st.speed = (sp.st.speed * k).clamp(0.2, 400.0);
        }
    }
}

fn delete_selected(sp: &mut Sp) {
    if let Some(e) = sp.st.selected {
        sp.remove_label(e);
        sp.record(Undo::Restore(e));
        sp.toast("text deleted");
    }
}

fn horizon(q: Quat) -> Option<(Vec3, Vec3, Vec3)> {
    let f = q * Vec3::NEG_Z;
    let r = f.cross(Vec3::Y);
    if r.length_squared() < 1e-8 {
        return None;
    }
    let r = r.normalize();
    Some((f, r, r.cross(f)))
}

/// the rotation looking the same way with the horizon level
fn level_quat(q: Quat) -> Option<Quat> {
    let (f, r, u) = horizon(q)?;
    Some(Quat::from_mat3(&Mat3::from_cols(r, u, -f)))
}

pub fn roll_deg(q: Quat) -> Option<f32> {
    let (_, r, u) = horizon(q)?;
    let c = q * Vec3::Y;
    Some(c.dot(r).atan2(c.dot(u)).to_degrees())
}

/// flying: looking, rolling, levelling and moving
pub fn space_fly(
    mut st: ResMut<Space>,
    mut cam: Single<&mut Transform, With<SpaceCam>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keymap: Res<Keymap>,
    remap: Res<crate::keys::Remap>,
    motion: Res<AccumulatedMouseMotion>,
    time: Res<Time>,
    focus: Res<InputFocus>,
    editables: Query<(), With<EditableText>>,
    session: Res<Session>,
) {
    if session.route != Route::Space {
        return;
    }
    let dt = time.delta_secs().min(0.05);
    let typing = focus.get().is_some_and(|e| editables.contains(e));
    let held = |a: Action| -> f32 {
        if !typing && remap.listening.is_none() && keys.pressed(keymap.get(a).key) {
            1.0
        } else {
            0.0
        }
    };
    let mut q = cam.rotation;

    if st.looking {
        let d = motion.delta;
        if d.x.abs() <= 250.0 && d.y.abs() <= 250.0 && d != Vec2::ZERO {
            q *= Quat::from_rotation_y(-d.x * LOOK);
            q *= Quat::from_rotation_x(-d.y * LOOK);
        }
    }

    let roll_in = held(Action::RollLeft) - held(Action::RollRight);
    st.roll_vel += (roll_in * ROLL_RATE - st.roll_vel) * (1.0 - (-dt * 10.0).exp());
    if st.roll_vel.abs() > 1e-4 {
        q *= Quat::from_rotation_z(st.roll_vel * dt);
    }
    if roll_in != 0.0 {
        st.leveling = false;
    }
    if st.leveling {
        match level_quat(q) {
            Some(target) => {
                q = q.slerp(target, 1.0 - (-dt * 7.0).exp());
                if q.angle_between(target) < 0.002 {
                    q = target;
                    st.leveling = false;
                }
            }
            None => st.leveling = false,
        }
    }
    q = q.normalize();
    cam.rotation = q;

    let both = mouse.pressed(MouseButton::Right) && mouse.pressed(MouseButton::Middle);
    let mut v = Vec3::new(
        held(Action::FlyRight) - held(Action::FlyLeft),
        held(Action::FlyUp) - held(Action::FlyDown),
        held(Action::FlyBack) - (held(Action::FlyForward).max(if both { 1.0 } else { 0.0 })),
    );
    if v != Vec3::ZERO {
        let boost = if !typing && keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            BOOST
        } else {
            1.0
        };
        v = q * v.normalize() * st.speed * boost;
    }
    let vel = st.vel.lerp(v, 1.0 - (-dt * 9.0).exp());
    st.vel = vel;
    cam.translation += vel * dt;
}

/// the spread moves its texts; hover, tint and the cursor follow the pointer
pub fn space_physics(mut sp: Sp, session: Res<Session>, mut override_cursor: ResMut<OverrideCursor>) {
    if session.route != Route::Space {
        return;
    }
    if sp.st.drag.is_some() {
        sp.update_drag();
    }
    if !sp.st.spread.nodes.is_empty() {
        if sp.st.spread.dirty {
            sp.st.spread.rekey();
            sp.paint();
        }
        sp.sync_pinned();
        if !sp.st.physics_paused {
            let steps = sp.st.get("substeps") as usize;
            for _ in 0..steps {
                if sp.st.spread.asleep() {
                    break;
                }
                sp.st.spread.step(1.0 / 60.0);
            }
        }
        let (frame, d) = (sp.st.frame, sp.st.spread.p.d);
        let st = &*sp.st;
        let moves: Vec<(Entity, Vec3)> = st
            .spread
            .nodes
            .iter()
            .filter(|n| !n.pinned)
            .filter_map(|n| st.mem_index.get(&n.id).map(|&e| (e, Sp::node_to_world(&frame, d, n))))
            .collect();
        for (e, p) in moves {
            if let Ok((_, mut t, mut tf)) = sp.texts.get_mut(e) {
                tf.translation = p;
                t.placed = true;
            }
        }
    }

    let hover = if let Some(d) = &sp.st.drag {
        Some(d.e)
    } else if sp.st.looking {
        None
    } else if let (Some(c), false) = (sp.st.cursor, sp.st.over_ui) {
        sp.pick(c)
    } else {
        sp.st.hover
    };
    sp.st.hover = hover;
    let selected = sp.st.selected;
    for (e, mut t, _) in sp.texts.iter_mut() {
        let tint = if Some(e) == hover || Some(e) == selected {
            ACCENT
        } else {
            t.color
        };
        if t.tint != tint {
            t.tint = tint;
        }
    }
    let mt = hover.or(selected);
    if mt != sp.st.shown_for && !sp.st.over_ui {
        sp.st.shown_for = mt;
        sp.st.redraw.metrics = true;
    }
    let want = if sp.st.looking || sp.st.over_ui {
        None
    } else if sp.st.drag.is_some() {
        Some(EntityCursor::System(SystemCursorIcon::Grabbing))
    } else if hover.is_some() {
        Some(EntityCursor::System(SystemCursorIcon::Grab))
    } else {
        None
    };
    if override_cursor.0 != want {
        override_cursor.0 = want;
    }
}

#[derive(Default)]
pub struct VisualCache {
    quad: Option<Handle<Mesh>>,
    materials: HashMap<[u8; 4], Handle<StandardMaterial>>,
}

impl VisualCache {
    fn material(&mut self, materials: &mut Assets<StandardMaterial>, c: Color) -> Handle<StandardMaterial> {
        let key = c.to_srgba().to_u8_array();
        self.materials
            .entry(key)
            .or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: c,
                    unlit: true,
                    ..default()
                })
            })
            .clone()
    }
}

/// billboards: texts face the camera, hide when too small, and are built line by line
pub fn space_visuals(
    mut commands: Commands,
    st: Res<Space>,
    font: Res<SlugFont>,
    cam: Single<&Transform, With<SpaceCam>>,
    mut texts: Query<
        (
            Entity,
            &mut SpaceText,
            &mut Transform,
            &mut Visibility,
            Option<&Children>,
        ),
        Without<SpaceCam>,
    >,
    lines: Query<(&MeshMaterial3d<TextMaterial>, &ChildOf), With<TextLine>>,
    mut bars: Query<(&mut Visibility, &mut MeshMaterial3d<StandardMaterial>), (With<UnderlineBar>, Without<SpaceText>)>,
    mut text_materials: ResMut<Assets<TextMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut cache: Local<VisualCache>,
    session: Res<Session>,
) {
    if session.route != Route::Space {
        return;
    }
    let cam = **cam;
    let quad = cache
        .quad
        .get_or_insert_with(|| meshes.add(Rectangle::new(1.0, 1.0)))
        .clone();
    for (e, mut t, mut tf, mut vis, children) in texts.iter_mut() {
        tf.rotation = cam.rotation;
        let shown = Sp::visible(&st, &cam, &t, &tf);
        let want = if shown { Visibility::Visible } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if t.relayout && font.data.is_some() {
            t.relayout = false;
            t.underline_dirty = false;
            commands.entity(e).despawn_related::<Children>();
            let n = t.layout.lines.len() as f32;
            let top = n * LINE / 2.0;
            let size = t.size;
            let bar = t.underline.map(|c| cache.material(&mut materials, c));
            for (i, (line, w)) in t.layout.lines.iter().enumerate() {
                let base = top - (i as f32 + 0.5) * LINE - CENTER;
                if !line.trim().is_empty() {
                    commands.spawn((
                        TextMesh {
                            text: line.clone(),
                            font: font.handle.clone(),
                            color: t.tint,
                            bg_color: Color::NONE,
                            size,
                        },
                        Transform::from_xyz(-w / 2.0 * size, base * size, 0.0),
                        TextLine,
                        ChildOf(e),
                    ));
                }
                if *w > 0.0 {
                    commands.spawn((
                        Mesh3d(quad.clone()),
                        MeshMaterial3d(bar.clone().unwrap_or_default()),
                        Transform::from_xyz(0.0, (base - 0.135) * size, 0.0).with_scale(Vec3::new(
                            w * size,
                            0.07 * size,
                            1.0,
                        )),
                        if bar.is_some() {
                            Visibility::Inherited
                        } else {
                            Visibility::Hidden
                        },
                        UnderlineBar,
                        ChildOf(e),
                    ));
                }
            }
        } else if t.underline_dirty
            && let Some(children) = children
        {
            t.underline_dirty = false;
            let bar = t.underline.map(|c| cache.material(&mut materials, c));
            for c in children.iter() {
                if let Ok((mut v, mut m)) = bars.get_mut(c) {
                    match &bar {
                        Some(h) => {
                            *v = Visibility::Inherited;
                            m.0 = h.clone();
                        }
                        None => *v = Visibility::Hidden,
                    }
                }
            }
        }
    }
    // hover and selection tint the lines through their materials, which keeps the meshes
    for (mat, parent) in lines.iter() {
        let Ok((_, t, _, _, _)) = texts.get(parent.parent()) else {
            continue;
        };
        let want = t.tint.to_linear();
        if text_materials.get(&mat.0).is_some_and(|m| m.color != want)
            && let Some(mut m) = text_materials.get_mut(&mat.0)
        {
            m.color = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_like_remem() {
        assert_eq!(wraptxt("the quick brown fox", 9), "the quick\nbrown fox");
        assert_eq!(wraptxt("short", 60), "short");
        assert_eq!(wraptxt("", 60), "");
        // remem drops the head of a word longer than the limit
        assert_eq!(wraptxt("abcdefgh", 5), "defgh");
        assert_eq!(wraptxt("one\ntwo", 60), "one\ntwo");
        assert_eq!(wraptxt("   ", 60), "   ");
    }

    #[test]
    fn addresses() {
        assert_eq!(parse_bucket("123"), Some((123, false)));
        assert_eq!(parse_bucket("r7"), Some((7, true)));
        assert_eq!(parse_bucket("rome"), None);
        assert_eq!(parse_bucket("r"), None);
        assert_eq!(parse_bucket(""), None);
    }

    #[test]
    fn rows_roundtrip() {
        let rows: Vec<Row> = serde_json::from_str(
            r#"[["hi",1,2,3,{}],["dog",0.5,0,-1,{"key":"3,14","size":0.8}],["x",0,0,0,{"key":5}]]"#,
        )
        .unwrap();
        assert_eq!(rows[1].4.key.as_deref(), Some("3,14"));
        assert_eq!(rows[2].4.key.as_deref(), Some("5"));
        let back = serde_json::to_string(&rows[0]).unwrap();
        assert_eq!(back, r#"["hi",1.0,2.0,3.0,{}]"#);
    }

    #[test]
    fn levels_and_rolls() {
        let q = Quat::from_rotation_y(0.7) * Quat::from_rotation_z(0.4);
        assert!(
            (roll_deg(q).unwrap() - (-0.4f32).to_degrees()).abs() < 0.01
                || (roll_deg(q).unwrap() - 0.4f32.to_degrees()).abs() < 0.01
        );
        let l = level_quat(q).unwrap();
        assert!(roll_deg(l).unwrap().abs() < 1e-3);
        assert!(((l * Vec3::NEG_Z) - (q * Vec3::NEG_Z)).length() < 1e-5);
    }
}
