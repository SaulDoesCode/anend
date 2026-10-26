//! Spatial saves: blocks placed in the space, each showing a texture (a text of the save,
//! one of its pictures, or a plain colour) tiled over its faces.
//!
//! Blocks are placements of kind `Block` in the room's scene (`graph::scenes`): the
//! texture is the definition (an entry, a picture by hash, a plain colour) and each
//! block a placement under it, its position and its sides (w, h, d) as the scale. A
//! block's id is its address, `<kind>:<texture>#<n>` ([`block_id`]): the n-th block
//! showing that texture. A thousand copies of one text cost about 15 bytes each (format.md).
//! Saves made before scenes kept blocks in `graph blocks` and `graph block ids`;
//! [`migrate`] moves them into the room when the save opens. A region query reads the
//! room's placements once (no spatial index yet).
//!
//! A block's size is in the space's units; its texture tiles at one tile per unit
//! ([`TILE`]), so making a block bigger shows more tiles instead of stretching one.

use redb::{MultimapTableDefinition, MultimapTableHandle, ReadableMultimapTable, TableDefinition};
use serde::{Deserialize, Serialize};

use super::record;
use super::scenes::{self, Def, Kind as SKind, Placed, Placement, Scale};
use super::store::{GResult, Graph, err};

/// the tables blocks were kept in before scenes (read once, by [`migrate`])
pub const OLD_BLOCKS: MultimapTableDefinition<&str, &[u8]> = MultimapTableDefinition::new("graph blocks");
pub const OLD_BLOCK_IDS: TableDefinition<u64, &str> = TableDefinition::new("graph block ids");

/// the side of one texture tile, in space units
pub const TILE: f32 = 1.0;
/// a side at least and at most
pub const MIN_SIDE: f32 = 0.05;
pub const MAX_SIDE: f32 = 10_000.0;
/// how far from the origin a block may be
pub const MAX_COORD: f32 = 1.0e6;

/// what a block shows
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// a text of the save, by its entry label
    Text,
    /// a picture of the save, by its content hash
    Picture,
    /// a plain colour, `#rrggbb`
    Plain,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Picture => "picture",
            Kind::Plain => "plain",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s.trim() {
            "text" => Some(Kind::Text),
            "picture" => Some(Kind::Picture),
            "plain" => Some(Kind::Plain),
            _ => None,
        }
    }
}

/// the key a texture is kept under
pub fn key(kind: Kind, texture: &str) -> String {
    format!("{}:{}", kind.name(), texture)
}

/// a key's kind and texture
pub fn split_key(k: &str) -> Option<(Kind, &str)> {
    let (a, b) = k.split_once(':')?;
    Some((Kind::parse(a)?, b))
}

/// where a block is, how big and how turned: its centre, its sides, its rotation (a unit
/// quaternion x, y, z, w; the identity costs nothing in the save) and its bend
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
    pub h: f32,
    pub d: f32,
    #[serde(default = "identity", skip_serializing_if = "is_identity")]
    pub rot: [f32; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bend: Option<Bend>,
}

fn identity() -> [f32; 4] {
    super::place::IDENTITY
}

fn is_identity(q: &[f32; 4]) -> bool {
    *q == super::place::IDENTITY
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
            h: 1.0,
            d: 1.0,
            rot: super::place::IDENTITY,
            bend: None,
        }
    }
}

/// how a block bends, in its own frame (x across its width, z through its depth): two
/// shapes added together, each moving the block's slices along z by an amount that
/// depends on where across the width they are. Both profiles are cubic bezier curves in
/// one dimension, with their ends fixed and two control values (c1, c2) shaping them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bend {
    pub ripple: Ripple,
    pub fold: Fold,
}

/// a curtain ripple: a wave across the width, `freq` periods of it, shifted by `phase`
/// (a fraction of a period); one period's profile is the bezier from 0 through c1, c2
/// back to 0, scaled so its peak is `amount` units
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ripple {
    pub amount: f32,
    pub freq: f32,
    pub phase: f32,
    pub c1: f32,
    pub c2: f32,
}

/// a fold across the middle: about the vertical line at `at` (-1 the left edge, 1 the
/// right), each side bent back by the bezier from 0 at the line through c1, c2 to 1 at the
/// far edge, times `amount` units
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fold {
    pub amount: f32,
    pub at: f32,
    pub c1: f32,
    pub c2: f32,
}

impl Default for Ripple {
    fn default() -> Self {
        Ripple {
            amount: 0.0,
            freq: 2.0,
            phase: 0.0,
            c1: 1.0,
            c2: -1.0,
        }
    }
}

impl Default for Fold {
    fn default() -> Self {
        Fold {
            amount: 0.0,
            at: 0.0,
            c1: 0.0,
            c2: 1.0,
        }
    }
}

/// a bezier's control values, an amount and a fold line kept within bounds
const BEND_MAX: f32 = 50.0;

impl Bend {
    pub fn straight(&self) -> bool {
        self.ripple.amount == 0.0 && self.fold.amount == 0.0
    }

    fn floats(&self) -> [f32; 9] {
        let (r, f) = (self.ripple, self.fold);
        [r.amount, r.freq, r.phase, r.c1, r.c2, f.amount, f.at, f.c1, f.c2]
    }

    /// the bytes a placement keeps: `B`, version 1, nine little endian f32s
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![b'B', 1];
        for v in self.floats() {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Bend> {
        if b.len() != 2 + 36 || b[0] != b'B' || b[1] != 1 {
            return None;
        }
        let f = |i: usize| f32::from_le_bytes(b[2 + i * 4..6 + i * 4].try_into().unwrap_or([0; 4]));
        Some(Bend {
            ripple: Ripple {
                amount: f(0),
                freq: f(1),
                phase: f(2),
                c1: f(3),
                c2: f(4),
            },
            fold: Fold {
                amount: f(5),
                at: f(6),
                c1: f(7),
                c2: f(8),
            },
        })
    }

    /// finite and within bounds (None when straight)
    pub fn checked(self) -> Result<Option<Bend>, String> {
        if self.floats().iter().any(|v| !v.is_finite()) {
            return Err("a bend's values are numbers".into());
        }
        let c = |v: f32, lo: f32, hi: f32| v.clamp(lo, hi);
        let b = Bend {
            ripple: Ripple {
                amount: c(self.ripple.amount, -BEND_MAX, BEND_MAX),
                freq: c(self.ripple.freq, 0.0, 64.0),
                phase: self.ripple.phase.rem_euclid(1.0),
                c1: c(self.ripple.c1, -3.0, 3.0),
                c2: c(self.ripple.c2, -3.0, 3.0),
            },
            fold: Fold {
                amount: c(self.fold.amount, -BEND_MAX, BEND_MAX),
                at: c(self.fold.at, -1.0, 1.0),
                c1: c(self.fold.c1, -3.0, 3.0),
                c2: c(self.fold.c2, -3.0, 3.0),
            },
        };
        Ok((!b.straight()).then_some(b))
    }

    /// how far it moves the block's slice at `u` across its width (0 the left edge, 1 the
    /// right) along z; the space draws the same (`space.js` `bendAt`)
    pub fn offset(&self, u: f32) -> f32 {
        let r = self.ripple;
        let mut z = 0.0;
        if r.amount != 0.0 && r.freq > 0.0 {
            let s = (u * r.freq + r.phase).rem_euclid(1.0);
            z += r.amount * wave(s, r.c1, r.c2) / wave_peak(r.c1, r.c2);
        }
        let f = self.fold;
        if f.amount != 0.0 {
            let line = 0.5 + f.at * 0.5;
            let t = if u >= line {
                (u - line) / (1.0 - line).max(1e-6)
            } else {
                (line - u) / line.max(1e-6)
            };
            z += f.amount * rise(t.clamp(0.0, 1.0), f.c1, f.c2);
        }
        z
    }

    /// the most it moves any slice either way (for regions)
    pub fn reach(&self) -> f32 {
        self.ripple.amount.abs() + self.fold.amount.abs()
    }
}

/// a period's profile: the cubic bezier 0, c1, c2, 0 at s
fn wave(s: f32, c1: f32, c2: f32) -> f32 {
    3.0 * (1.0 - s) * (1.0 - s) * s * c1 + 3.0 * (1.0 - s) * s * s * c2
}

/// the largest |wave| over a period, so `amount` is the peak (1 when it's flat)
fn wave_peak(c1: f32, c2: f32) -> f32 {
    let m = (0..=64)
        .map(|i| wave(i as f32 / 64.0, c1, c2).abs())
        .fold(0.0, f32::max);
    if m > 1e-6 { m } else { 1.0 }
}

/// a fold's profile: the cubic bezier 0, c1, c2, 1 at t
fn rise(t: f32, c1: f32, c2: f32) -> f32 {
    3.0 * (1.0 - t) * (1.0 - t) * t * c1 + 3.0 * (1.0 - t) * t * t * c2 + t * t * t
}

impl Transform {
    /// the id (big endian, so values sort by it) and six little endian f32s
    pub fn to_bytes(&self, id: u64) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&id.to_be_bytes());
        for (i, v) in [self.x, self.y, self.z, self.w, self.h, self.d].iter().enumerate() {
            b[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<(u64, Transform)> {
        if b.len() != 32 {
            return None;
        }
        let f = |i: usize| f32::from_le_bytes(b[8 + i * 4..12 + i * 4].try_into().unwrap_or([0; 4]));
        Some((
            u64::from_be_bytes(b[..8].try_into().ok()?),
            Transform {
                x: f(0),
                y: f(1),
                z: f(2),
                w: f(3),
                h: f(4),
                d: f(5),
                ..Transform::default()
            },
        ))
    }

    /// finite, inside the world, sides within bounds
    pub fn checked(self) -> Result<Transform, String> {
        let all = [self.x, self.y, self.z, self.w, self.h, self.d];
        if all.iter().any(|v| !v.is_finite()) {
            return Err("a block's place and size are numbers".into());
        }
        if [self.x, self.y, self.z].iter().any(|v| v.abs() > MAX_COORD) {
            return Err(format!("a block stays within {MAX_COORD} of the origin"));
        }
        let side = |v: f32| v.clamp(MIN_SIDE, MAX_SIDE);
        // a unit quaternion (a zero or broken one is no turn)
        let q = self.rot;
        let n = q.iter().map(|v| v * v).sum::<f32>().sqrt();
        let rot = if !n.is_finite() || n < 1e-6 {
            super::place::IDENTITY
        } else {
            let u = q.map(|v| v / n);
            // the same turn with w ≥ 0, and the identity exactly when it's no turn
            let u = if u[3] < 0.0 { u.map(|v| -v) } else { u };
            if (u[3] - 1.0).abs() < 1e-7 {
                super::place::IDENTITY
            } else {
                u
            }
        };
        let bend = match self.bend {
            Some(b) => b.checked()?,
            None => None,
        };
        Ok(Transform {
            w: side(self.w),
            h: side(self.h),
            d: side(self.d),
            rot,
            bend,
            ..self
        })
    }

    /// half the sides of the box around it as it's turned and bent (for regions)
    pub fn half_extent(&self) -> [f32; 3] {
        let reach = self.bend.map_or(0.0, |b| b.reach());
        let half = [self.w / 2.0, self.h / 2.0, self.d / 2.0 + reach];
        let [x, y, z, w] = self.rot;
        // the rotation matrix's columns, by absolute value
        let m = [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - z * w),
                2.0 * (x * z + y * w),
            ],
            [
                2.0 * (x * y + z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - x * w),
            ],
            [
                2.0 * (x * z - y * w),
                2.0 * (y * z + x * w),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ];
        [0, 1, 2].map(|r| (0..3).map(|c| m[r][c].abs() * half[c]).sum())
    }

    /// whether the block's box meets the box from `lo` to `hi`
    pub fn meets(&self, lo: [f32; 3], hi: [f32; 3]) -> bool {
        let c = [self.x, self.y, self.z];
        let half = self.half_extent();
        (0..3).all(|i| c[i] + half[i] >= lo[i] && c[i] - half[i] <= hi[i])
    }

    /// how many times the texture repeats across each face: (w, h) for the front and
    /// back, (d, h) for the sides, (w, d) for the top and bottom
    pub fn repeats(&self) -> [[f32; 2]; 3] {
        [
            [self.w / TILE, self.h / TILE],
            [self.d / TILE, self.h / TILE],
            [self.w / TILE, self.d / TILE],
        ]
    }
}

/// one block
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// its address in the room ([`block_id`])
    pub id: String,
    pub kind: Kind,
    pub texture: String,
    #[serde(flatten)]
    pub t: Transform,
}

/// how the blocks table is used: instances and distinct textures
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BlockUse {
    pub blocks: u64,
    pub textures: u64,
}

/// a block's address in the room: `<kind>:<texture>#<n>`, its texture's key and its
/// number among that texture's blocks
pub fn block_id(kind: Kind, texture: &str, n: u64) -> String {
    format!("{}#{n}", key(kind, texture))
}

/// an address's texture and number
pub fn split_id(id: &str) -> Option<(Kind, String, u64)> {
    let (k, n) = id.rsplit_once('#')?;
    let (kind, texture) = split_key(k)?;
    Some((kind, texture.to_string(), n.parse().ok()?))
}

/// what a texture is as a scene definition
pub fn def_of(kind: Kind, texture: &str) -> GResult<Def> {
    let t = texture.trim();
    match kind {
        Kind::Text => record::parse_label(t.strip_prefix("note:").unwrap_or(t))
            .map(Def::Entry)
            .ok_or_else(|| format!("{t:?} isn't a text's key")),
        Kind::Picture => {
            let b = (0..16)
                .map(|i| t.get(2 * i..2 * i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()))
                .collect::<Option<Vec<u8>>>()
                .filter(|_| t.len() == 32)
                .ok_or_else(|| format!("{t:?} isn't a picture's hash (32 hex digits)"))?;
            Ok(Def::Picture(b.try_into().expect("16 bytes")))
        }
        Kind::Plain => {
            let ok = t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit());
            if !ok {
                return Err(format!("{t:?} isn't a colour (#rrggbb)"));
            }
            let v = u32::from_str_radix(&t[1..], 16).map_err(err)?;
            Ok(Def::Colour([(v >> 16) as u8, (v >> 8) as u8, v as u8]))
        }
    }
}

/// a definition back as a texture
pub fn texture_of(def: &Def) -> Option<(Kind, String)> {
    Some(match def {
        Def::Entry(id) => (Kind::Text, record::label(*id)),
        Def::Picture(h) => (Kind::Picture, h.iter().map(|b| format!("{b:02x}")).collect()),
        Def::Colour([r, g, b]) => (Kind::Plain, format!("#{r:02x}{g:02x}{b:02x}")),
        Def::Scene(_) => return None,
    })
}

/// the shape flag every block has for now (`place::Kind::Cube`, bits 3–6)
const CUBE: u64 = (super::place::Kind::Cube as u64) << 3;

fn placement(t: &Transform, n: u64) -> Placement {
    let mut p = Placement::at(n, [t.x, t.y, t.z]);
    p.scale = Scale::Axes([t.w, t.h, t.d]).canonical();
    p.rot = (t.rot != super::place::IDENTITY).then(|| super::place::pack_rotation(t.rot));
    p.extra = t.bend.filter(|b| !b.straight()).map(|b| b.to_bytes());
    p.flags = Some(CUBE);
    p
}

fn transform(p: &Placement) -> Transform {
    let [w, h, d] = match p.scale {
        Scale::One => [1.0; 3],
        Scale::Uniform(v) => [v; 3],
        Scale::Axes(a) => a,
    };
    Transform {
        x: p.pos[0],
        y: p.pos[1],
        z: p.pos[2],
        w,
        h,
        d,
        rot: p
            .rot
            .map(super::place::unpack_rotation)
            .unwrap_or(super::place::IDENTITY),
        bend: p.extra.as_deref().and_then(Bend::from_bytes),
    }
}

fn block_of(p: &Placed) -> Option<Block> {
    if p.kind != SKind::Block {
        return None;
    }
    let (kind, texture) = texture_of(&p.def)?;
    Some(Block {
        id: block_id(kind, &texture, p.place.n),
        kind,
        texture,
        t: transform(&p.place),
    })
}

/// the old tables' blocks moved into the room, inside the write that opens the save;
/// how many moved. Each texture's blocks keep their order (they sorted by id) as their
/// numbers 1, 2, ...; a texture that doesn't read as a definition is kept as the old
/// tables had it and the tables stay
pub(crate) fn migrate(txn: &redb::WriteTransaction) -> GResult<usize> {
    let had = txn
        .list_multimap_tables()
        .map_err(err)?
        .any(|h| h.name() == OLD_BLOCKS.name());
    if !had {
        return Ok(0);
    }
    let mut groups: Vec<(Def, Vec<Placement>)> = Vec::new();
    {
        let t = txn.open_multimap_table(OLD_BLOCKS).map_err(err)?;
        for item in t.iter().map_err(err)? {
            let (k, vs) = item.map_err(err)?;
            let (kind, texture) = split_key(k.value()).ok_or("an old block's key doesn't read")?;
            let def = def_of(kind, texture)?;
            let mut ps = Vec::new();
            for v in vs {
                let (_, tr) = Transform::from_bytes(v.map_err(err)?.value()).ok_or("an old block doesn't read")?;
                ps.push(placement(&tr, 0));
            }
            groups.push((def, ps));
        }
    }
    let n: usize = groups.iter().map(|g| g.1.len()).sum();
    if n > 0 {
        let room = scenes::room_in(txn)?;
        for (def, ps) in groups {
            scenes::place_in(txn, room, false, SKind::Block, def, None, ps, &[])?;
        }
    }
    txn.delete_multimap_table(OLD_BLOCKS).map_err(err)?;
    txn.delete_table(OLD_BLOCK_IDS).map_err(err)?;
    Ok(n)
}

impl Graph {
    /// the room's placements (none before the room is made)
    fn room_placements(&self) -> GResult<Vec<Placed>> {
        match self.room()? {
            Some(r) => self.placements(r, false),
            None => Ok(vec![]),
        }
    }

    /// places a block in the room; its id
    pub fn add_block(&self, kind: Kind, texture: &str, t: Transform) -> GResult<String> {
        Ok(self.add_blocks(&[(kind, texture.to_string(), t)])?.swap_remove(0))
    }

    /// places several blocks in the room in one write; their ids
    pub fn add_blocks(&self, blocks: &[(Kind, String, Transform)]) -> GResult<Vec<String>> {
        let mut checked = Vec::with_capacity(blocks.len());
        for (kind, texture, t) in blocks {
            checked.push((*kind, texture.trim().to_string(), def_of(*kind, texture)?, t.checked()?));
        }
        if checked.is_empty() {
            return Ok(vec![]);
        }
        let txn = self.db().begin_write().map_err(err)?;
        let mut out = Vec::with_capacity(checked.len());
        {
            let room = scenes::room_in(&txn)?;
            for (kind, texture, def, t) in checked {
                let n = scenes::place_in(&txn, room, false, SKind::Block, def, None, vec![placement(&t, 0)], &[])?[0];
                out.push(block_id(kind, &texture_of(&def).map(|x| x.1).unwrap_or(texture), n));
            }
        }
        txn.commit().map_err(err)?;
        Ok(out)
    }

    /// one block
    pub fn block(&self, id: &str) -> GResult<Option<Block>> {
        let Some((kind, texture, n)) = split_id(id) else {
            return Ok(None);
        };
        let def = def_of(kind, &texture)?;
        Ok(self
            .room_placements()?
            .iter()
            .find(|p| p.kind == SKind::Block && p.def == def && p.place.n == n)
            .and_then(block_of))
    }

    /// moves or resizes blocks (each its whole new transform) in one write; the ones
    /// that weren't there are skipped, the rest come back as they are now
    pub fn set_blocks(&self, changes: &[(String, Transform)]) -> GResult<Vec<Block>> {
        let mut out = Vec::new();
        let mut todo = Vec::new();
        for (id, t) in changes {
            let t = t.checked()?;
            if let Some(mut b) = self.block(id)? {
                b.t = t;
                todo.push(b);
            }
        }
        let txn = self.db().begin_write().map_err(err)?;
        {
            let room = scenes::room_in(&txn)?;
            for b in todo {
                let (_, _, n) = split_id(&b.id).expect("read above");
                let mut p = placement(&b.t, n);
                p.n = n;
                scenes::place_in(
                    &txn,
                    room,
                    false,
                    SKind::Block,
                    def_of(b.kind, &b.texture)?,
                    None,
                    vec![p],
                    &[],
                )?;
                out.push(b);
            }
        }
        txn.commit().map_err(err)?;
        Ok(out)
    }

    /// takes blocks away; the ones that were there, as they were (to put them back)
    pub fn delete_blocks(&self, ids: &[String]) -> GResult<Vec<Block>> {
        let mut out = Vec::new();
        for id in ids {
            if let Some(b) = self.block(id)? {
                out.push(b);
            }
        }
        let txn = self.db().begin_write().map_err(err)?;
        {
            let room = scenes::room_in(&txn)?;
            for b in &out {
                let (_, _, n) = split_id(&b.id).expect("read above");
                scenes::place_in(
                    &txn,
                    room,
                    false,
                    SKind::Block,
                    def_of(b.kind, &b.texture)?,
                    None,
                    vec![],
                    &[n],
                )?;
            }
        }
        txn.commit().map_err(err)?;
        Ok(out)
    }

    /// puts blocks back with their own ids (an undo); the ids already in use are skipped,
    /// and a block without a number (`#0`, or an old backup's) gets the next free one
    pub fn put_blocks(&self, blocks: &[Block]) -> GResult<usize> {
        let mut todo = Vec::new();
        for b in blocks {
            let n = split_id(&b.id).map(|x| x.2).unwrap_or(0);
            if n != 0 && self.block(&b.id)?.is_some() {
                continue;
            }
            todo.push((def_of(b.kind, &b.texture)?, b.t.checked()?, n));
        }
        if todo.is_empty() {
            return Ok(0);
        }
        let txn = self.db().begin_write().map_err(err)?;
        {
            let room = scenes::room_in(&txn)?;
            for (def, t, n) in &todo {
                scenes::place_in(&txn, room, false, SKind::Block, *def, None, vec![placement(t, *n)], &[])?;
            }
        }
        txn.commit().map_err(err)?;
        Ok(todo.len())
    }

    /// copies of blocks, each moved by `offset`; the new ids
    pub fn duplicate_blocks(&self, ids: &[String], offset: [f32; 3]) -> GResult<Vec<String>> {
        let mut new = Vec::new();
        for id in ids {
            let b = self.block(id)?.ok_or_else(|| format!("no block {id}"))?;
            let mut t = b.t;
            t.x += offset[0];
            t.y += offset[1];
            t.z += offset[2];
            new.push((b.kind, b.texture, t));
        }
        self.add_blocks(&new)
    }

    /// every block in the room, or (with a region) those whose boxes meet it
    pub fn blocks(&self, region: Option<([f32; 3], [f32; 3])>) -> GResult<Vec<Block>> {
        Ok(self
            .room_placements()?
            .iter()
            .filter_map(block_of)
            .filter(|b| region.is_none_or(|(lo, hi)| b.t.meets(lo, hi)))
            .collect())
    }

    /// the blocks showing one texture
    pub fn blocks_of(&self, kind: Kind, texture: &str) -> GResult<Vec<Block>> {
        let def = def_of(kind, texture)?;
        Ok(self
            .room_placements()?
            .iter()
            .filter(|p| p.def == def)
            .filter_map(block_of)
            .collect())
    }

    /// how many blocks and distinct textures
    pub fn block_use(&self) -> GResult<BlockUse> {
        let bs = self.blocks(None)?;
        let textures: std::collections::BTreeSet<String> = bs.iter().map(|b| key(b.kind, &b.texture)).collect();
        Ok(BlockUse {
            blocks: bs.len() as u64,
            textures: textures.len() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(x: f32, w: f32) -> Transform {
        Transform {
            x,
            w,
            ..Transform::default()
        }
    }

    use redb::ReadableDatabase;

    const PIC: &str = "000102030405060708090a0b0c0d0e0f";

    #[test]
    fn transforms_round_trip_and_sort_by_id() {
        let a = Transform {
            x: 1.5,
            y: -2.0,
            z: 3.25,
            w: 4.0,
            h: 0.5,
            d: 2.0,
            ..Transform::default()
        };
        assert_eq!(Transform::from_bytes(&a.to_bytes(7)), Some((7, a)));
        assert!(Transform::from_bytes(&[0; 31]).is_none());
        assert_eq!(a.repeats(), [[4.0, 0.5], [2.0, 0.5], [4.0, 2.0]]);
        assert!(t(f32::NAN, 1.0).checked().is_err());
        assert_eq!(t(0.0, 0.0).checked().unwrap().w, MIN_SIDE);
        assert_eq!(transform(&placement(&a, 3)), a);
        assert_eq!(split_id("text:1,2#7"), Some((Kind::Text, "1,2".into(), 7)));
        assert_eq!(split_id("plain:#a0b0c0#2"), Some((Kind::Plain, "#a0b0c0".into(), 2)));
        assert!(def_of(Kind::Picture, "ab").is_err());
    }

    #[test]
    fn blocks_are_room_placements_numbered_per_texture() {
        let g = Graph::in_memory().unwrap();
        let ids = g
            .add_blocks(&[
                (Kind::Text, "1,2".into(), t(0.0, 1.0)),
                (Kind::Text, "1,2".into(), t(5.0, 2.0)),
                (Kind::Picture, PIC.into(), t(-5.0, 1.0)),
            ])
            .unwrap();
        assert_eq!(ids, ["text:1,2#1", "text:1,2#2", &format!("picture:{PIC}#1")]);
        assert_eq!(g.block_use().unwrap(), BlockUse { blocks: 3, textures: 2 });
        assert_eq!(g.blocks_of(Kind::Text, "1,2").unwrap().len(), 2);

        // moving one replaces it under its number
        let moved = g
            .set_blocks(&[("text:1,2#2".into(), t(9.0, 3.0)), ("text:1,2#99".into(), t(0.0, 1.0))])
            .unwrap();
        assert_eq!(moved.len(), 1);
        assert_eq!(g.block("text:1,2#2").unwrap().unwrap().t, t(9.0, 3.0));
        assert_eq!(g.block_use().unwrap().blocks, 3);

        let near: Vec<String> = g
            .blocks(Some(([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0])))
            .unwrap()
            .into_iter()
            .map(|b| b.id)
            .collect();
        assert_eq!(near, ["text:1,2#1"]);

        // duplicate, delete, put back with the same ids
        let pic = format!("picture:{PIC}#1");
        let copies = g
            .duplicate_blocks(&["text:1,2#1".into(), pic.clone()], [0.0, 2.0, 0.0])
            .unwrap();
        assert_eq!(copies, ["text:1,2#3", &format!("picture:{PIC}#2")]);
        assert_eq!(g.block(&copies[1]).unwrap().unwrap().t.y, 2.0);
        let gone = g.delete_blocks(&["text:1,2#1".into(), "text:1,2#3".into()]).unwrap();
        assert_eq!(gone.len(), 2);
        assert_eq!(g.blocks_of(Kind::Text, "1,2").unwrap().len(), 1);
        assert_eq!(g.put_blocks(&gone).unwrap(), 2);
        assert_eq!(g.put_blocks(&gone).unwrap(), 0, "their ids are in use again");
        assert_eq!(g.blocks(None).unwrap().len(), 5);
        assert_eq!(g.block("text:1,2#1").unwrap().unwrap().t, t(0.0, 1.0));
        assert_eq!(
            g.add_block(Kind::Plain, "#FF0000", t(0.0, 1.0)).unwrap(),
            "plain:#ff0000#1"
        );
    }

    #[test]
    fn old_saves_move_their_blocks_into_the_room() {
        let db = redb::Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .unwrap();
        // the tables as saves had them before scenes
        let txn = db.begin_write().unwrap();
        {
            let mut b = txn.open_multimap_table(OLD_BLOCKS).unwrap();
            let mut ids = txn.open_table(OLD_BLOCK_IDS).unwrap();
            for (id, k, tr) in [
                (1u64, "text:1,2", t(0.0, 1.0)),
                (2, "plain:#112233", t(1.0, 2.0)),
                (3, "text:1,2", t(5.0, 3.0)),
            ] {
                b.insert(k, tr.to_bytes(id).as_slice()).unwrap();
                ids.insert(id, k).unwrap();
            }
        }
        txn.commit().unwrap();
        let g = Graph::from_db(db).unwrap();
        let bs = g.blocks(None).unwrap();
        assert_eq!(bs.len(), 3);
        // a texture's blocks keep their order as their numbers
        assert_eq!(g.block("text:1,2#1").unwrap().unwrap().t, t(0.0, 1.0));
        assert_eq!(g.block("text:1,2#2").unwrap().unwrap().t, t(5.0, 3.0));
        assert_eq!(g.block("plain:#112233#1").unwrap().unwrap().t, t(1.0, 2.0));
        // the old tables are gone, so opening again moves nothing twice
        let txn = g.db().begin_read().unwrap();
        assert!(!txn.list_multimap_tables().unwrap().any(|h| h.name() == "graph blocks"));
    }
}

#[cfg(test)]
mod turn_and_bend_tests {
    use super::*;

    #[test]
    fn rotation_and_bend_are_kept_and_cost_nothing_when_absent() {
        let g = Graph::in_memory().unwrap();
        let plain = g.add_block(Kind::Plain, "#123456", Transform::default()).unwrap();
        let s = (0.3f32).sin();
        let turned = Transform {
            x: 1.0,
            w: 3.0,
            rot: [0.0, s, 0.0, (0.3f32).cos()],
            bend: Some(Bend {
                ripple: Ripple {
                    amount: 0.4,
                    freq: 3.0,
                    phase: 1.25,
                    ..Ripple::default()
                },
                fold: Fold {
                    amount: -0.5,
                    at: 0.2,
                    ..Fold::default()
                },
            }),
            ..Transform::default()
        };
        let id = g.add_block(Kind::Plain, "#123456", turned).unwrap();
        let b = g.block(&id).unwrap().unwrap();
        // the rotation comes back as packed (smallest three, 32 bits), the bend exactly
        assert!(
            (b.t.rot[1] - s).abs() < 1e-3 && b.t.rot[0].abs() < 1e-3,
            "{:?}",
            b.t.rot
        );
        let bend = b.t.bend.unwrap();
        assert_eq!(bend.ripple.phase, 0.25, "phase wraps");
        assert_eq!(bend.fold.at, 0.2);
        // stored again, the same bytes (canonical)
        assert_eq!(placement(&b.t, 2), placement(&transform(&placement(&b.t, 2)), 2));
        // a plain block has no rotation and no extra in its placement
        let p = g.block(&plain).unwrap().unwrap();
        let pl = placement(&p.t, 1);
        assert!(pl.rot.is_none() && pl.extra.is_none());
        assert_eq!(serde_json::to_value(p.t).unwrap().get("rot"), None);
        // straightened, the bend goes; a turned block's region grows with its turn
        let straight = g
            .set_blocks(&[(
                id.clone(),
                Transform {
                    bend: Some(Bend::default()),
                    ..b.t
                },
            )])
            .unwrap();
        assert_eq!(straight[0].t.bend, None);
        let tall = Transform {
            w: 4.0,
            h: 0.1,
            d: 0.1,
            rot: [
                0.0,
                0.0,
                (std::f32::consts::FRAC_PI_4).sin(),
                (std::f32::consts::FRAC_PI_4).cos(),
            ],
            ..Transform::default()
        };
        assert!(tall.half_extent()[1] > 1.4, "turned 90° about z, its width stands up");
        // the bend's profile: the fold is 0 at its line and `amount` at the edges
        let f = Bend {
            fold: Fold {
                amount: 2.0,
                ..Fold::default()
            },
            ..Bend::default()
        };
        assert_eq!(f.offset(0.5), 0.0);
        assert!((f.offset(0.0) - 2.0).abs() < 1e-6 && (f.offset(1.0) - 2.0).abs() < 1e-6);
        let r = Bend {
            ripple: Ripple {
                amount: 1.0,
                ..Ripple::default()
            },
            ..Bend::default()
        };
        let peak = (0..=200).map(|i| r.offset(i as f32 / 200.0).abs()).fold(0.0, f32::max);
        assert!((peak - 1.0).abs() < 0.02, "{peak}");
    }
}
