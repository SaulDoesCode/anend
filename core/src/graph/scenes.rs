//! Scenes and their placements (docs/format.md, "One model for a definition and its
//! placed instances"): a save holds many scenes; a scene holds placements of texts
//! (billboards), blocks and prefabs (scenes placed inside scenes).
//!
//! | table | key | value |
//! |---|---|---|
//! | `graph scenes` | scene id | its record as JSON: name, parent, created, edited, settings |
//! | `graph scene instances` (multimap) | scene id | inheriting placements: per (kind, definition, chunk of 64 numbers) one value, [`Chunk`] |
//! | `graph scene detached` (multimap) | scene id | detached placements, the same layout with the entry's version after the definition |
//! | `graph placed` (multimap) | definition ref | the scenes placing it |
//!
//! The room (the space's own scene, holding its blocks) is the scene the setting
//! `scenes.room` names. `graph::blocks` keeps its block API over the room's placements,
//! and `ops::scenes` reads and writes the saved scenes.

use redb::{MultimapTableDefinition, ReadableDatabase, ReadableMultimapTable, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use super::codec::{BitReader, BitWriter};
use super::store::{GResult, Graph, err};

pub const SCENES: TableDefinition<u64, &[u8]> = TableDefinition::new("graph scenes");
pub const INSTANCES: MultimapTableDefinition<u64, &[u8]> = MultimapTableDefinition::new("graph scene instances");
pub const DETACHED: MultimapTableDefinition<u64, &[u8]> = MultimapTableDefinition::new("graph scene detached");
pub const PLACED: MultimapTableDefinition<&[u8], u64> = MultimapTableDefinition::new("graph placed");

/// placements a chunk holds at most (numbers `64c .. 64c + 63`)
pub const CHUNK: u64 = 64;

pub fn ensure_tables(txn: &redb::WriteTransaction) -> GResult<()> {
    txn.open_table(SCENES).map_err(err)?;
    txn.open_multimap_table(INSTANCES).map_err(err)?;
    txn.open_multimap_table(DETACHED).map_err(err)?;
    txn.open_multimap_table(PLACED).map_err(err)?;
    Ok(())
}

// ---------------------------------------------------------------- sized ids

/// one length byte (0–8), then that many big-endian bytes with no leading zero byte, so
/// comparing the bytes compares the numbers
pub fn put_sized(out: &mut Vec<u8>, v: u64) {
    let n = 8 - (v.leading_zeros() / 8) as usize;
    out.push(n as u8);
    out.extend_from_slice(&v.to_be_bytes()[8 - n..]);
}

pub fn get_sized(b: &[u8]) -> Option<(u64, usize)> {
    let n = *b.first()? as usize;
    if n > 8 || b.len() < 1 + n || (n > 0 && b[1] == 0) {
        return None;
    }
    let mut v = 0u64;
    for &x in &b[1..1 + n] {
        v = (v << 8) | x as u64;
    }
    Some((v, 1 + n))
}

// ---------------------------------------------------------------- definitions

/// what is placed; the byte order of [`Def::bytes`] is its sort order
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Def {
    /// an entry: a text, a chain, or a block's text texture
    Entry(u64),
    /// a picture, by its 128-bit content hash
    Picture([u8; 16]),
    /// a plain colour
    Colour([u8; 3]),
    /// a scene, placed as a prefab
    Scene(u64),
    /// a texture recipe (`graph::textures`), by its 128-bit content hash
    Texture([u8; 16]),
}

impl Def {
    pub fn write(&self, out: &mut Vec<u8>) {
        match self {
            Def::Entry(id) => {
                out.push(0);
                put_sized(out, *id);
            }
            Def::Picture(h) => {
                out.push(1);
                out.extend_from_slice(h);
            }
            Def::Colour(c) => {
                out.push(2);
                out.extend_from_slice(c);
            }
            Def::Scene(id) => {
                out.push(3);
                put_sized(out, *id);
            }
            Def::Texture(h) => {
                out.push(4);
                out.extend_from_slice(h);
            }
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        self.write(&mut v);
        v
    }

    pub fn read(b: &[u8]) -> Option<(Def, usize)> {
        let rest = b.get(1..)?;
        Some(match b.first()? {
            0 => {
                let (id, n) = get_sized(rest)?;
                (Def::Entry(id), 1 + n)
            }
            1 => (Def::Picture(rest.get(..16)?.try_into().ok()?), 17),
            2 => (Def::Colour(rest.get(..3)?.try_into().ok()?), 4),
            3 => {
                let (id, n) = get_sized(rest)?;
                (Def::Scene(id), 1 + n)
            }
            4 => (Def::Texture(rest.get(..16)?.try_into().ok()?), 17),
            _ => return None,
        })
    }
}

/// how a definition is placed
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Kind {
    /// a billboard: a position and a font size
    Text = 0,
    /// a block with a rotation, a scale, a colour and a shape
    Block = 1,
    /// a scene placed inside another, with a rotation and a scale
    Prefab = 2,
}

impl Kind {
    fn from_u8(b: u8) -> Option<Kind> {
        Some(match b {
            0 => Kind::Text,
            1 => Kind::Block,
            2 => Kind::Prefab,
            _ => return None,
        })
    }

    /// whether `def` can be placed this way
    pub fn takes(self, def: &Def) -> bool {
        match self {
            Kind::Text => matches!(def, Def::Entry(_)),
            Kind::Block => !matches!(def, Def::Scene(_)),
            Kind::Prefab => matches!(def, Def::Scene(_)),
        }
    }
}

// ---------------------------------------------------------------- placements

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scale {
    One,
    Uniform(f32),
    Axes([f32; 3]),
}

impl Scale {
    /// the same scale in its canonical form: one when every axis is 1, uniform when the
    /// three are the same bits
    pub fn canonical(self) -> Scale {
        let one = 1f32.to_bits();
        match self {
            Scale::Uniform(v) if v.to_bits() == one => Scale::One,
            Scale::Axes([a, b, c]) if a.to_bits() == b.to_bits() && b.to_bits() == c.to_bits() => {
                Scale::Uniform(a).canonical()
            }
            s => s,
        }
    }

    fn same(&self, o: &Scale) -> bool {
        match (self, o) {
            (Scale::One, Scale::One) => true,
            (Scale::Uniform(a), Scale::Uniform(b)) => a.to_bits() == b.to_bits(),
            (Scale::Axes(a), Scale::Axes(b)) => a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits()),
            _ => false,
        }
    }
}

/// one placement; the fields a kind doesn't have stay at their defaults
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// its number among its definition's placements of its kind in its scene, from 1
    pub n: u64,
    /// free coordinates, stored as the exact f32 bits
    pub pos: [f32; 3],
    /// a text's font size: 0 the default (a setting), 1–255 explicit
    pub font: u8,
    /// a block's or prefab's rotation, packed as `place::pack_rotation` (None: identity)
    pub rot: Option<u32>,
    pub scale: Scale,
    /// a block's RGBA8 colour
    pub colour: Option<u32>,
    /// 1 pinned, 2 hidden, 4 spread, 8–64 a block's shape (`place::Kind`), the rest kept
    pub flags: Option<u64>,
    /// an asset id, a prefab copy's overrides, later a program; kept as it is
    pub extra: Option<Vec<u8>>,
}

/// the flag saying a placement's extra holds its own values, which shadow its
/// definition's (an inheriting copy's overrides, as a JSON object: `text` for a text)
pub const OWN: u64 = 128;

impl Placement {
    /// an inheriting copy's own values (overrides), when it has any
    pub fn own(&self) -> Option<serde_json::Map<String, serde_json::Value>> {
        if self.flags.unwrap_or(0) & OWN == 0 {
            return None;
        }
        serde_json::from_slice(self.extra.as_deref()?).ok()
    }

    /// sets or clears its own values; an empty map clears them
    pub fn set_own(&mut self, own: Option<serde_json::Map<String, serde_json::Value>>) {
        let rest = self.flags.unwrap_or(0) & !OWN;
        match own.filter(|m| !m.is_empty()) {
            Some(m) => {
                self.flags = Some(rest | OWN);
                self.extra = Some(serde_json::to_vec(&m).expect("a JSON map"));
            }
            None => {
                if self.flags.unwrap_or(0) & OWN != 0 {
                    self.extra = None;
                }
                self.flags = Some(rest).filter(|f| *f != 0);
            }
        }
    }

    /// its own text, shadowing its entry's
    pub fn own_text(&self) -> Option<String> {
        self.own()?.get("text")?.as_str().map(str::to_string)
    }

    pub fn at(n: u64, pos: [f32; 3]) -> Placement {
        Placement {
            n,
            pos,
            font: 0,
            rot: None,
            scale: Scale::One,
            colour: None,
            flags: None,
            extra: None,
        }
    }

    /// the fields `kind` keeps, the rest at their defaults, the scale canonical
    pub fn for_kind(mut self, kind: Kind) -> Placement {
        self.scale = self.scale.canonical();
        match kind {
            Kind::Text => {
                self.rot = None;
                self.scale = Scale::One;
                self.colour = None;
            }
            Kind::Block => self.font = 0,
            Kind::Prefab => {
                self.font = 0;
                self.colour = None;
            }
        }
        self
    }
}

fn put_f32(w: &mut BitWriter, v: f32) {
    w.put(v.to_bits(), 32);
}

fn get_f32(r: &mut BitReader) -> GResult<f32> {
    Ok(f32::from_bits(r.get(32).map_err(err)?))
}

/// the bits a varint takes
fn varint_bits(v: u64) -> usize {
    5 * (64 - v.leading_zeros() as usize).div_ceil(4).max(1)
}

fn scale_bits(s: &Scale) -> usize {
    match s {
        Scale::One => 0,
        Scale::Uniform(_) => 32,
        Scale::Axes(_) => 96,
    }
}

/// one value of a scene's multimap: the prefix (kind, definition, the detached version,
/// the chunk number) and its placements in number order
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub kind: Kind,
    pub def: Def,
    /// a detached placement's entry version (the `edited` ms it was detached at)
    pub version: Option<u64>,
    pub c: u64,
    pub places: Vec<Placement>,
}

impl Chunk {
    /// the bytes every value of this (kind, definition, version, chunk) starts with
    pub fn prefix(kind: Kind, def: &Def, version: Option<u64>, c: u64) -> Vec<u8> {
        let mut out = vec![kind as u8];
        def.write(&mut out);
        if let Some(v) = version {
            put_sized(&mut out, v);
        }
        put_sized(&mut out, c);
        out
    }

    /// the canonical bytes: the prefix, then a 3-bit padding count and one record per
    /// placement (gap, position, flags, extra, then the kind's own fields)
    pub fn encode(&self) -> GResult<Vec<u8>> {
        let mut out = Chunk::prefix(self.kind, &self.def, self.version, self.c);
        let mut w = BitWriter::default();
        w.put(0, 3);
        let mut prev: Option<u64> = None;
        let places: Vec<Placement> = self.places.iter().cloned().map(|p| p.for_kind(self.kind)).collect();
        for (i, p) in places.iter().enumerate() {
            if p.n / CHUNK != self.c {
                return Err(format!("placement {} isn't in chunk {}", p.n, self.c));
            }
            let gap = match prev {
                None => p.n - CHUNK * self.c,
                Some(q) if p.n > q => p.n - q - 1,
                Some(_) => return Err("placements out of number order".into()),
            };
            prev = Some(p.n);
            w.varint(gap);
            for v in p.pos {
                put_f32(&mut w, v);
            }
            w.put(p.flags.is_some() as u32, 1);
            w.put(p.extra.is_some() as u32, 1);
            if let Some(f) = p.flags {
                w.varint(f);
            }
            if let Some(x) = &p.extra {
                w.varint(x.len() as u64);
                for b in x {
                    w.put(*b as u32, 8);
                }
            }
            match self.kind {
                Kind::Text => w.varint(p.font as u64),
                Kind::Block | Kind::Prefab => {
                    // the nearest earlier placement with the same bits, when that's smaller
                    let back = |same: &dyn Fn(&Placement) -> bool| {
                        (0..i).rev().find(|&j| same(&places[j])).map(|j| p.n - places[j].n)
                    };
                    match p.rot {
                        None => w.put(0, 2),
                        Some(r) => match back(&|q: &Placement| q.rot == Some(r)) {
                            Some(d) if varint_bits(d) < 32 => {
                                w.put(2, 2);
                                w.varint(d);
                            }
                            _ => {
                                w.put(1, 2);
                                w.put(r, 32);
                            }
                        },
                    }
                    let own = scale_bits(&p.scale);
                    match back(&|q: &Placement| q.scale.same(&p.scale)) {
                        Some(d) if own > 0 && varint_bits(d) < own => {
                            w.put(3, 2);
                            w.varint(d);
                        }
                        _ => match p.scale {
                            Scale::One => w.put(0, 2),
                            Scale::Uniform(v) => {
                                w.put(1, 2);
                                put_f32(&mut w, v);
                            }
                            Scale::Axes(a) => {
                                w.put(2, 2);
                                for v in a {
                                    put_f32(&mut w, v);
                                }
                            }
                        },
                    }
                    if self.kind == Kind::Block {
                        w.put(p.colour.is_some() as u32, 1);
                        if let Some(c) = p.colour {
                            w.put(c, 32);
                        }
                    }
                }
            }
        }
        out.extend(w.finish_padded());
        Ok(out)
    }

    /// reads a value back; `detached` says whether a version follows the definition
    pub fn decode(b: &[u8], detached: bool) -> GResult<Chunk> {
        let bad = || "a scene value that doesn't read".to_string();
        let kind = Kind::from_u8(*b.first().ok_or_else(bad)?).ok_or_else(bad)?;
        let (def, n) = Def::read(&b[1..]).ok_or_else(bad)?;
        let mut at = 1 + n;
        let version = if detached {
            let (v, n) = get_sized(&b[at..]).ok_or_else(bad)?;
            at += n;
            Some(v)
        } else {
            None
        };
        let (c, n) = get_sized(&b[at..]).ok_or_else(bad)?;
        at += n;
        let body = &b[at..];
        let mut places: Vec<Placement> = Vec::new();
        if !body.is_empty() {
            let pad = (body[0] >> 5) as usize;
            let mut r = BitReader::new(body, 3, body.len() * 8 - pad);
            let mut prev: Option<u64> = None;
            while r.left() > 0 {
                let gap = r.varint().map_err(err)?;
                let n = match prev {
                    None => CHUNK * c + gap,
                    Some(q) => q + 1 + gap,
                };
                prev = Some(n);
                let mut p = Placement::at(n, [get_f32(&mut r)?, get_f32(&mut r)?, get_f32(&mut r)?]);
                let has_flags = r.get(1).map_err(err)? == 1;
                let has_extra = r.get(1).map_err(err)? == 1;
                if has_flags {
                    p.flags = Some(r.varint().map_err(err)?);
                }
                if has_extra {
                    let len = r.varint().map_err(err)? as usize;
                    let mut x = Vec::with_capacity(len);
                    for _ in 0..len {
                        x.push(r.get(8).map_err(err)? as u8);
                    }
                    p.extra = Some(x);
                }
                let find = |d: u64, places: &Vec<Placement>| -> GResult<Placement> {
                    places
                        .iter()
                        .find(|q| q.n + d == n)
                        .cloned()
                        .ok_or_else(|| "a reference to a placement that isn't there".to_string())
                };
                match kind {
                    Kind::Text => p.font = u8::try_from(r.varint().map_err(err)?).map_err(|_| bad())?,
                    Kind::Block | Kind::Prefab => {
                        p.rot = match r.get(2).map_err(err)? {
                            0 => None,
                            1 => Some(r.get(32).map_err(err)?),
                            2 => find(r.varint().map_err(err)?, &places)?.rot,
                            _ => return Err(bad()),
                        };
                        p.scale = match r.get(2).map_err(err)? {
                            0 => Scale::One,
                            1 => Scale::Uniform(get_f32(&mut r)?),
                            2 => Scale::Axes([get_f32(&mut r)?, get_f32(&mut r)?, get_f32(&mut r)?]),
                            _ => find(r.varint().map_err(err)?, &places)?.scale,
                        };
                        if kind == Kind::Block && r.get(1).map_err(err)? == 1 {
                            p.colour = Some(r.get(32).map_err(err)?);
                        }
                    }
                }
                places.push(p);
            }
        }
        Ok(Chunk {
            kind,
            def,
            version,
            c,
            places,
        })
    }
}

// ---------------------------------------------------------------- scenes

/// a scene's record
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub id: u64,
    pub name: String,
    /// the scene it sits in (0: none)
    #[serde(default)]
    pub parent: u64,
    pub created: u64,
    pub edited: u64,
    #[serde(default)]
    pub settings: serde_json::Value,
}

/// a placement with where it is
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub kind: Kind,
    pub def: Def,
    pub version: Option<u64>,
    pub place: Placement,
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn table(detached: bool) -> MultimapTableDefinition<'static, u64, &'static [u8]> {
    if detached { DETACHED } else { INSTANCES }
}

/// the setting naming the room's scene (its id, in decimal)
pub const ROOM: &str = "scenes.room";

fn room_id(b: &[u8]) -> Option<u64> {
    std::str::from_utf8(b).ok()?.trim().parse().ok()
}

/// a scene's values in one table, decoded, with their bytes
fn values(
    t: &impl ReadableMultimapTable<u64, &'static [u8]>,
    scene: u64,
    detached: bool,
) -> GResult<Vec<(Vec<u8>, Chunk)>> {
    let mut out = Vec::new();
    for v in t.get(scene).map_err(err)? {
        let b = v.map_err(err)?.value().to_vec();
        let ch = Chunk::decode(&b, detached)?;
        out.push((b, ch));
    }
    Ok(out)
}

/// splits placements into chunks of 64 numbers and writes them
fn write_chunks(
    t: &mut redb::MultimapTable<u64, &[u8]>,
    scene: u64,
    kind: Kind,
    def: Def,
    version: Option<u64>,
    mut places: Vec<Placement>,
) -> GResult<()> {
    places.sort_by_key(|p| p.n);
    let mut i = 0;
    while i < places.len() {
        let c = places[i].n / CHUNK;
        let j = places[i..]
            .iter()
            .position(|p| p.n / CHUNK != c)
            .map_or(places.len(), |k| i + k);
        let ch = Chunk {
            kind,
            def,
            version,
            c,
            places: places[i..j].to_vec(),
        };
        t.insert(scene, ch.encode()?.as_slice()).map_err(err)?;
        i = j;
    }
    Ok(())
}

/// `graph placed` made to agree with what the scene holds now
fn sync_placed(txn: &redb::WriteTransaction, scene: u64) -> GResult<()> {
    let mut now: std::collections::BTreeSet<Vec<u8>> = Default::default();
    for d in [false, true] {
        let t = txn.open_multimap_table(table(d)).map_err(err)?;
        for (_, ch) in values(&t, scene, d)? {
            now.insert(ch.def.bytes());
        }
    }
    let mut placed = txn.open_multimap_table(PLACED).map_err(err)?;
    let mut stale = Vec::new();
    for item in placed.iter().map_err(err)? {
        let (k, vs) = item.map_err(err)?;
        for v in vs {
            if v.map_err(err)?.value() == scene && !now.contains(k.value()) {
                stale.push(k.value().to_vec());
            }
        }
    }
    for k in stale {
        placed.remove(k.as_slice(), scene).map_err(err)?;
    }
    for k in now {
        placed.insert(k.as_slice(), scene).map_err(err)?;
    }
    Ok(())
}

/// writes placements of one (kind, definition, version) into a scene inside a write:
/// each replaces the one with its number, or takes the next free number when its `n` is
/// 0; `drop` takes numbers out. Their numbers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_in(
    txn: &redb::WriteTransaction,
    scene: u64,
    detached: bool,
    kind: Kind,
    def: Def,
    version: Option<u64>,
    places: Vec<Placement>,
    drop: &[u64],
) -> GResult<Vec<u64>> {
    if !kind.takes(&def) {
        return Err(format!("a {kind:?} can't place {def:?}"));
    }
    if detached != version.is_some() {
        return Err("a detached placement has the version it was detached at, and only it".into());
    }
    if txn.open_table(SCENES).map_err(err)?.get(scene).map_err(err)?.is_none() {
        return Err(format!("no scene {scene}"));
    }
    let numbers = {
        let mut t = txn.open_multimap_table(table(detached)).map_err(err)?;
        let mut mine: Vec<Placement> = Vec::new();
        for (b, ch) in values(&t, scene, detached)? {
            if ch.kind == kind && ch.def == def && ch.version == version {
                mine.extend(ch.places);
                t.remove(scene, b.as_slice()).map_err(err)?;
            }
        }
        mine.retain(|p| !drop.contains(&p.n));
        let mut next = mine.iter().map(|p| p.n).max().unwrap_or(0) + 1;
        let mut numbers = Vec::new();
        for mut p in places {
            if p.n == 0 {
                p.n = next;
                next += 1;
            }
            numbers.push(p.n);
            mine.retain(|q| q.n != p.n);
            mine.push(p);
        }
        write_chunks(&mut t, scene, kind, def, version, mine)?;
        numbers
    };
    sync_placed(txn, scene)?;
    Ok(numbers)
}

/// a new scene inside a write; its id
pub(crate) fn add_scene_in(txn: &redb::WriteTransaction, name: &str, parent: u64) -> GResult<u64> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a scene needs a name".into());
    }
    let mut t = txn.open_table(SCENES).map_err(err)?;
    if parent != 0 && t.get(parent).map_err(err)?.is_none() {
        return Err(format!("no scene {parent}"));
    }
    let id = t.last().map_err(err)?.map(|(k, _)| k.value() + 1).unwrap_or(1);
    let now = now_ms();
    let s = Scene {
        id,
        name: name.to_string(),
        parent,
        created: now,
        edited: now,
        settings: serde_json::Value::Null,
    };
    t.insert(id, serde_json::to_vec(&s).map_err(err)?.as_slice())
        .map_err(err)?;
    Ok(id)
}

/// the room's scene inside a write, made when there's none
pub(crate) fn room_in(txn: &redb::WriteTransaction) -> GResult<u64> {
    let kept = {
        let s = txn.open_table(super::store::SETTINGS).map_err(err)?;
        s.get(ROOM).map_err(err)?.and_then(|v| room_id(v.value()))
    };
    if let Some(id) = kept
        && txn.open_table(SCENES).map_err(err)?.get(id).map_err(err)?.is_some()
    {
        return Ok(id);
    }
    let id = add_scene_in(txn, "room", 0)?;
    txn.open_table(super::store::SETTINGS)
        .map_err(err)?
        .insert(ROOM, id.to_string().as_bytes())
        .map_err(err)?;
    Ok(id)
}

impl Graph {
    /// a new scene; its id
    pub fn add_scene(&self, name: &str, parent: u64) -> GResult<u64> {
        let txn = self.db().begin_write().map_err(err)?;
        let id = add_scene_in(&txn, name, parent)?;
        txn.commit().map_err(err)?;
        Ok(id)
    }

    /// the room's scene (the space's own, holding its blocks), if it has been made
    pub fn room(&self) -> GResult<Option<u64>> {
        let txn = self.db().begin_read().map_err(err)?;
        let s = txn.open_table(super::store::SETTINGS).map_err(err)?;
        let kept = s.get(ROOM).map_err(err)?.and_then(|v| room_id(v.value()));
        match kept {
            Some(id) if txn.open_table(SCENES).map_err(err)?.get(id).map_err(err)?.is_some() => Ok(Some(id)),
            _ => Ok(None),
        }
    }

    pub fn scenes(&self) -> GResult<Vec<Scene>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(SCENES).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (_, v) = item.map_err(err)?;
            out.push(serde_json::from_slice(v.value()).map_err(err)?);
        }
        Ok(out)
    }

    pub fn scene(&self, id: u64) -> GResult<Option<Scene>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(SCENES).map_err(err)?;
        match t.get(id).map_err(err)? {
            Some(v) => Ok(Some(serde_json::from_slice(v.value()).map_err(err)?)),
            None => Ok(None),
        }
    }

    /// the first scene with this name (prefabs aside: `graph::prefabs`)
    pub fn scene_named(&self, name: &str) -> GResult<Option<Scene>> {
        Ok(self
            .scenes()?
            .into_iter()
            .find(|s| s.name == name && s.settings.get("prefab") != Some(&serde_json::Value::Bool(true))))
    }

    pub fn rename_scene(&self, id: u64, name: &str) -> GResult<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a scene needs a name".into());
        }
        let txn = self.db().begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(SCENES).map_err(err)?;
            let mut s: Scene = match t.get(id).map_err(err)? {
                Some(v) => serde_json::from_slice(v.value()).map_err(err)?,
                None => return Err(format!("no scene {id}")),
            };
            s.name = name.to_string();
            s.edited = now_ms();
            t.insert(id, serde_json::to_vec(&s).map_err(err)?.as_slice())
                .map_err(err)?;
        }
        txn.commit().map_err(err)
    }

    /// removes a scene with its placements; what it held (to put back)
    pub fn delete_scene(&self, id: u64) -> GResult<(Scene, Vec<Placed>)> {
        let s = self.scene(id)?.ok_or_else(|| format!("no scene {id}"))?;
        let held = [self.placements(id, false)?, self.placements(id, true)?].concat();
        let txn = self.db().begin_write().map_err(err)?;
        {
            for d in [false, true] {
                txn.open_multimap_table(table(d))
                    .map_err(err)?
                    .remove_all(id)
                    .map_err(err)?;
            }
            txn.open_table(SCENES).map_err(err)?.remove(id).map_err(err)?;
            sync_placed(&txn, id)?;
        }
        txn.commit().map_err(err)?;
        Ok((s, held))
    }

    /// every placement in a scene, inheriting or detached, in (kind, definition, number)
    /// order
    pub fn placements(&self, scene: u64, detached: bool) -> GResult<Vec<Placed>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_multimap_table(table(detached)).map_err(err)?;
        let mut out = Vec::new();
        for (_, ch) in values(&t, scene, detached)? {
            out.extend(ch.places.into_iter().map(|place| Placed {
                kind: ch.kind,
                def: ch.def,
                version: ch.version,
                place,
            }));
        }
        Ok(out)
    }

    /// writes placements into a scene: each replaces the one with its number (its
    /// `n`), or is added under the next free number when `n` is 0; their numbers
    pub fn place(
        &self,
        scene: u64,
        detached: bool,
        kind: Kind,
        def: Def,
        version: Option<u64>,
        places: Vec<Placement>,
    ) -> GResult<Vec<u64>> {
        let txn = self.db().begin_write().map_err(err)?;
        let numbers = place_in(&txn, scene, detached, kind, def, version, places, &[])?;
        txn.commit().map_err(err)?;
        Ok(numbers)
    }

    /// takes placements out by number; how many went
    pub fn unplace(
        &self,
        scene: u64,
        detached: bool,
        kind: Kind,
        def: Def,
        version: Option<u64>,
        numbers: &[u64],
    ) -> GResult<usize> {
        let before = self.placements(scene, detached)?.len();
        let txn = self.db().begin_write().map_err(err)?;
        place_in(&txn, scene, detached, kind, def, version, vec![], numbers)?;
        txn.commit().map_err(err)?;
        Ok(before - self.placements(scene, detached)?.len())
    }

    /// replaces every text placement of a scene, inheriting and detached, with these, in
    /// one write: per entry, the version it was detached at (None: inheriting) and its
    /// placements
    pub fn set_texts(&self, scene: u64, texts: Vec<SceneTexts>) -> GResult<()> {
        let txn = self.db().begin_write().map_err(err)?;
        {
            if txn.open_table(SCENES).map_err(err)?.get(scene).map_err(err)?.is_none() {
                return Err(format!("no scene {scene}"));
            }
            for d in [false, true] {
                let mut t = txn.open_multimap_table(table(d)).map_err(err)?;
                for (b, ch) in values(&t, scene, d)? {
                    if ch.kind == Kind::Text {
                        t.remove(scene, b.as_slice()).map_err(err)?;
                    }
                }
                for st in texts.iter().filter(|st| st.version.is_some() == d) {
                    write_chunks(
                        &mut t,
                        scene,
                        Kind::Text,
                        Def::Entry(st.id),
                        st.version,
                        st.places.clone(),
                    )?;
                }
            }
            let mut st = txn.open_table(SCENES).map_err(err)?;
            let mut s: Scene =
                serde_json::from_slice(st.get(scene).map_err(err)?.expect("checked").value()).map_err(err)?;
            s.edited = now_ms();
            st.insert(scene, serde_json::to_vec(&s).map_err(err)?.as_slice())
                .map_err(err)?;
        }
        sync_placed(&txn, scene)?;
        txn.commit().map_err(err)
    }

    /// the scenes placing a definition, and how many placements each holds
    pub fn placed_in(&self, def: Def) -> GResult<Vec<(u64, usize)>> {
        let scenes: Vec<u64> = {
            let txn = self.db().begin_read().map_err(err)?;
            let t = txn.open_multimap_table(PLACED).map_err(err)?;
            t.get(def.bytes().as_slice())
                .map_err(err)?
                .map(|v| v.map(|v| v.value()).map_err(err))
                .collect::<GResult<_>>()?
        };
        let mut out = Vec::new();
        for s in scenes {
            let n = [false, true]
                .iter()
                .map(|&d| {
                    self.placements(s, d)
                        .map(|ps| ps.iter().filter(|p| p.def == def).count())
                })
                .sum::<GResult<usize>>()?;
            out.push((s, n));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- migration

/// a space text's size (world units per em) as a placement's font size: steps of 1/50,
/// so the space's default 0.5 is 25 (docs/wip/questions.md, 1)
pub fn font_of(size: f64) -> u8 {
    (size * 50.0).round().clamp(1.0, 255.0) as u8
}

/// a font size back as a space text's size (None: the default)
pub fn size_of(font: u8) -> Option<f64> {
    (font != 0).then(|| font as f64 / 50.0)
}

/// where the old saved scenes' JSON is kept after it moved into scenes
pub const MAPS_BEFORE: &str = "space.maps (before scenes)";

impl Graph {
    /// a saved scene's rows as the space keeps them, `[text, x, y, z, {key?, size?,
    /// detached?, own?}]`, into a scene's text placements: a row with a key places that
    /// entry (a note's key is `note:<label>`; a room copy's `<label>~2` is the same
    /// entry), one without becomes a memory entry first (format.md answer 1). `detached`
    /// (the entry's edited time, ms) makes it a detached copy showing that version;
    /// `own` is an inheriting copy's own text, shadowing the entry's
    pub fn rows_to_texts(&self, rows: &[serde_json::Value]) -> GResult<Vec<SceneTexts>> {
        let mut out: Vec<SceneTexts> = Vec::new();
        let mut keyless = Vec::new();
        for r in rows {
            let Some(a) = r.as_array().filter(|a| a.len() >= 4) else {
                continue;
            };
            let pos = [a[1].as_f64(), a[2].as_f64(), a[3].as_f64()];
            let [Some(x), Some(y), Some(z)] = pos else { continue };
            let mut p = Placement::at(0, [x as f32, y as f32, z as f32]);
            let o = a.get(4).cloned().unwrap_or_default();
            if let Some(s) = o.get("size").and_then(serde_json::Value::as_f64) {
                p.font = font_of(s);
            }
            let key = o.get("key").and_then(serde_json::Value::as_str).unwrap_or("");
            let key = key.split('~').next().unwrap_or(key);
            let id = super::record::parse_label(key.strip_prefix("note:").unwrap_or(key));
            let version = o.get("detached").and_then(serde_json::Value::as_u64);
            if version.is_none()
                && let Some(t) = o.get("own").and_then(serde_json::Value::as_str)
            {
                let mut m = serde_json::Map::new();
                m.insert("text".into(), serde_json::json!(t));
                p.set_own(Some(m));
            }
            match id {
                Some(id) => push(&mut out, id, version, p),
                None => {
                    let text = a[0].as_str().unwrap_or("").trim().to_string();
                    if !text.is_empty() {
                        keyless.push((text, p));
                    }
                }
            }
        }
        if !keyless.is_empty() {
            let ids = self.write(|tx| {
                keyless
                    .iter()
                    .map(|(t, _)| tx.save_text(t, super::record::mime::TEXT).map(|x| x.0))
                    .collect::<GResult<Vec<u64>>>()
            })?;
            for (id, (_, p)) in ids.into_iter().zip(keyless) {
                push(&mut out, id, None, p);
            }
        }
        // each (entry, version)'s placements numbered in row order, from 1
        for st in out.iter_mut() {
            for (i, p) in st.places.iter_mut().enumerate() {
                p.n = i as u64 + 1;
            }
        }
        Ok(out)
    }

    /// an entry's text as it read at `version` (an edited time, ms): the newest version
    /// written at or before it, from `graph history`; the oldest kept when none is
    pub fn text_at(&self, id: u64, version: u64) -> GResult<String> {
        use super::Reader;
        let vs = self.view()?.versions(id)?;
        let found = vs.iter().find(|v| v.at <= version).or(vs.last());
        Ok(found.map(|v| v.text.clone()).unwrap_or_default())
    }

    /// saves made before scenes: the space's saved scenes (`space.maps`, name → rows) and
    /// the maps' `PLACE` edges become scenes; the old JSON is kept as [`MAPS_BEFORE`]
    pub(crate) fn migrate_maps(&self) -> GResult<usize> {
        let mut made = 0;
        // PLACE edges: each map entry with some becomes a scene named by its text
        let mut edges: std::collections::BTreeMap<u64, Vec<(u64, Vec<u8>)>> = Default::default();
        {
            let txn = self.db().begin_read().map_err(err)?;
            let t = txn.open_multimap_table(super::store::EDGES).map_err(err)?;
            for item in t.iter().map_err(err)? {
                let (k, vs) = item.map_err(err)?;
                let (from, r) = k.value();
                if r != super::store::rel::PLACE {
                    continue;
                }
                for v in vs {
                    let v = v.map_err(err)?.value().to_vec();
                    if v.len() >= 8 {
                        let to = u64::from_be_bytes(v[..8].try_into().expect("8 bytes"));
                        edges.entry(from).or_default().push((to, v[8..].to_vec()));
                    }
                }
            }
        }
        for (map, placed) in edges {
            let name = self
                .render(map)
                .unwrap_or_else(|_| format!("map {}", super::record::label(map)));
            let scene = self.add_scene(&name, 0)?;
            for (to, payload) in &placed {
                let Some(pl) = super::place::Place::unpack(payload) else {
                    continue;
                };
                let mut p = Placement::at(0, pl.pos);
                p.flags = Some(pl.flags as u64 | ((pl.kind as u64) << 3)).filter(|f| *f != 0);
                let kind = if pl.kind == super::place::Kind::Text {
                    Kind::Text
                } else {
                    Kind::Block
                };
                if kind == Kind::Text {
                    p.font = font_of(pl.scale[0] as f64);
                    // anything else a text's placement had is kept whole
                    let plain = super::place::Place::text(pl.pos, pl.scale[0], pl.color, pl.flags);
                    if plain.pack() != pl.pack() {
                        p.extra = Some(pl.pack().to_vec());
                    }
                } else {
                    p.rot = (pl.rot != super::place::IDENTITY).then(|| super::place::pack_rotation(pl.rot));
                    p.scale = Scale::Axes(pl.scale).canonical();
                    p.colour = Some(u32::from_be_bytes(pl.color));
                    p.extra = (pl.asset != 0).then(|| pl.asset.to_le_bytes().to_vec());
                }
                self.place(scene, false, kind, Def::Entry(*to), None, vec![p])?;
            }
            self.write(|tx| {
                for (to, payload) in &placed {
                    tx.unlink_exact(map, super::store::rel::PLACE, *to, payload)?;
                }
                Ok(())
            })?;
            made += 1;
        }
        // the space's saved scenes
        let raw = {
            let txn = self.db().begin_read().map_err(err)?;
            let t = txn.open_table(super::store::SETTINGS).map_err(err)?;
            t.get("space.maps").map_err(err)?.map(|v| v.value().to_vec())
        };
        if let Some(raw) = raw {
            let maps: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&raw).unwrap_or_default();
            for (name, rows) in &maps {
                let texts = self.rows_to_texts(rows.as_array().map(Vec::as_slice).unwrap_or(&[]))?;
                let scene = self.add_scene(name, 0)?;
                self.set_texts(scene, texts)?;
                made += 1;
            }
            self.write(|tx| {
                tx.set_setting(MAPS_BEFORE, &raw)?;
                tx.remove_setting("space.maps")
            })?;
        }
        Ok(made)
    }
}

/// one entry's text placements in a scene, for `set_texts`: inheriting (`version`
/// None) or detached at that version
#[derive(Clone, Debug, PartialEq)]
pub struct SceneTexts {
    pub id: u64,
    pub version: Option<u64>,
    pub places: Vec<Placement>,
}

pub(crate) fn push(out: &mut Vec<SceneTexts>, id: u64, version: Option<u64>, p: Placement) {
    match out.iter_mut().find(|st| st.id == id && st.version == version) {
        Some(st) => st.places.push(p),
        None => out.push(SceneTexts {
            id,
            version,
            places: vec![p],
        }),
    }
}

// ---------------------------------------------------------------- backups

/// an f32 in a backup: a number when finite (printed shortest, so it reads back exact),
/// else its bits as `"bits:7fc00000"`
fn f_json(v: f32) -> serde_json::Value {
    if v.is_finite() {
        serde_json::json!(v)
    } else {
        serde_json::json!(format!("bits:{:08x}", v.to_bits()))
    }
}

fn f_read(v: &serde_json::Value) -> Option<f32> {
    match v {
        serde_json::Value::String(s) => u32::from_str_radix(s.strip_prefix("bits:")?, 16)
            .ok()
            .map(f32::from_bits),
        _ => v.as_f64().map(|x| x as f32),
    }
}

/// a definition as a backup writes it
pub fn def_name(def: &Def, version: Option<u64>) -> String {
    let base = match def {
        Def::Entry(id) => format!("text:{}", super::record::label(*id)),
        Def::Picture(h) => format!("picture:{}", h.iter().map(|b| format!("{b:02x}")).collect::<String>()),
        Def::Colour([r, g, b]) => format!("plain:#{r:02x}{g:02x}{b:02x}"),
        Def::Scene(id) => format!("scene:{id}"),
        Def::Texture(h) => format!("texture:{}", h.iter().map(|b| format!("{b:02x}")).collect::<String>()),
    };
    match version {
        Some(v) => format!("{base}@{v}"),
        None => base,
    }
}

pub fn parse_def_name(s: &str) -> Option<(Def, Option<u64>)> {
    let (s, version) = match s.rsplit_once('@') {
        Some((a, v)) if !a.ends_with('#') => (a, Some(v.parse().ok()?)),
        _ => (s, None),
    };
    let (kind, rest) = s.split_once(':')?;
    let def = match kind {
        "scene" => Def::Scene(rest.parse().ok()?),
        "text" => super::blocks::def_of(super::blocks::Kind::Text, rest).ok()?,
        "picture" => super::blocks::def_of(super::blocks::Kind::Picture, rest).ok()?,
        "plain" => super::blocks::def_of(super::blocks::Kind::Plain, rest).ok()?,
        "texture" => super::blocks::def_of(super::blocks::Kind::Texture, rest).ok()?,
        _ => return None,
    };
    Some((def, version))
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Text => "text",
        Kind::Block => "block",
        Kind::Prefab => "prefab",
    }
}

impl Graph {
    /// every placement of every scene as backup rows: `[scene, "inherit" | "detached",
    /// kind, definition, n, {pos, font?, rot?, scale?, colour?, flags?, extra?}]`
    pub fn instance_rows(&self) -> GResult<Vec<serde_json::Value>> {
        use serde_json::json;
        let mut out = Vec::new();
        for s in self.scenes()? {
            for d in [false, true] {
                for p in self.placements(s.id, d)? {
                    let q = &p.place;
                    let mut o = json!({ "pos": q.pos.iter().map(|v| f_json(*v)).collect::<Vec<_>>() });
                    if q.font != 0 {
                        o["font"] = json!(q.font);
                    }
                    if let Some(r) = q.rot {
                        o["rot"] = json!(r);
                    }
                    match q.scale {
                        Scale::One => {}
                        Scale::Uniform(v) => o["scale"] = f_json(v),
                        Scale::Axes(a) => o["scale"] = json!(a.iter().map(|v| f_json(*v)).collect::<Vec<_>>()),
                    }
                    if let Some(c) = q.colour {
                        o["colour"] = json!(format!("#{c:08x}"));
                    }
                    if let Some(f) = q.flags {
                        o["flags"] = json!(f);
                    }
                    if let Some(x) = &q.extra {
                        o["extra"] = json!(crate::classifiers::base64(x));
                    }
                    out.push(json!([
                        s.id,
                        if d { "detached" } else { "inherit" },
                        kind_name(p.kind),
                        def_name(&p.def, p.version),
                        q.n,
                        o
                    ]));
                }
            }
        }
        Ok(out)
    }

    /// puts scenes and their placements back from a backup, keeping their ids; rows that
    /// don't read are skipped
    pub fn put_instance_rows(&self, scenes: &[Scene], rows: &[serde_json::Value]) -> GResult<usize> {
        let txn = self.db().begin_write().map_err(err)?;
        let mut n = 0;
        {
            {
                let mut t = txn.open_table(SCENES).map_err(err)?;
                for s in scenes {
                    t.insert(s.id, serde_json::to_vec(s).map_err(err)?.as_slice())
                        .map_err(err)?;
                }
            }
            for r in rows {
                let Some(a) = r.as_array().filter(|a| a.len() == 6) else {
                    continue;
                };
                let (Some(scene), Some(how), Some(kind), Some(def), Some(num)) = (
                    a[0].as_u64(),
                    a[1].as_str(),
                    a[2].as_str(),
                    a[3].as_str(),
                    a[4].as_u64(),
                ) else {
                    continue;
                };
                let kind = match kind {
                    "text" => Kind::Text,
                    "block" => Kind::Block,
                    "prefab" => Kind::Prefab,
                    _ => continue,
                };
                let Some((def, version)) = parse_def_name(def) else {
                    continue;
                };
                let o = &a[5];
                let pos: Vec<f32> = o["pos"].as_array().into_iter().flatten().filter_map(f_read).collect();
                let Ok(pos) = <[f32; 3]>::try_from(pos) else { continue };
                let mut p = Placement::at(num, pos);
                p.font = o["font"].as_u64().unwrap_or(0).min(255) as u8;
                p.rot = o["rot"].as_u64().map(|v| v as u32);
                p.scale = match &o["scale"] {
                    serde_json::Value::Array(v) => {
                        let v: Vec<f32> = v.iter().filter_map(f_read).collect();
                        <[f32; 3]>::try_from(v).map(Scale::Axes).unwrap_or(Scale::One)
                    }
                    serde_json::Value::Null => Scale::One,
                    v => f_read(v).map(Scale::Uniform).unwrap_or(Scale::One),
                };
                p.colour = o["colour"]
                    .as_str()
                    .and_then(|c| u32::from_str_radix(c.trim_start_matches('#'), 16).ok());
                p.flags = o["flags"].as_u64();
                p.extra = o["extra"].as_str().and_then(crate::classifiers::unbase64);
                if place_in(&txn, scene, how == "detached", kind, def, version, vec![p], &[]).is_ok() {
                    n += 1;
                }
            }
        }
        txn.commit().map_err(err)?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(n: u64, pos: [f32; 3]) -> Placement {
        Placement::at(n, pos)
    }

    #[test]
    fn sized_ids_sort_as_numbers() {
        let mut prev = Vec::new();
        for v in [0u64, 1, 255, 256, 1038, 65535, 65536, u64::MAX] {
            let mut b = Vec::new();
            put_sized(&mut b, v);
            assert_eq!(get_sized(&b), Some((v, b.len())));
            assert!(b > prev, "{v}");
            prev = b;
        }
        let mut b = Vec::new();
        put_sized(&mut b, 1038);
        assert_eq!(b, [2, 0x04, 0x0e], "format.md's example");
        assert_eq!(get_sized(&[2, 0, 5]), None, "a leading zero byte isn't canonical");
    }

    #[test]
    fn edge_values_round_trip_and_bytes_are_canonical() {
        let odd = [
            f32::NAN,
            -0.0,
            0.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MAX,
            f32::MIN_POSITIVE / 2.0,
            1.0e-40,
            65504.0,
            -3.25,
        ];
        let mut places = Vec::new();
        for (i, v) in odd.iter().enumerate() {
            let mut p = block(i as u64 * 3 + 1, [*v, -*v, i as f32]);
            p.rot = (i % 3 == 0).then_some(0x1234_5678 ^ (i as u32 % 2));
            p.scale = match i % 4 {
                0 => Scale::One,
                1 => Scale::Uniform(*v),
                2 => Scale::Axes([*v, 1.0, 2.0]),
                _ => Scale::Uniform(-3.25),
            };
            p.colour = (i % 2 == 0).then_some(0xff80_40ff);
            p.flags = (i % 5 == 0).then_some(1 | 8);
            p.extra = (i == 4).then(|| vec![0, 255, 7]);
            places.push(p.for_kind(Kind::Block));
        }
        for kind in [Kind::Block, Kind::Prefab, Kind::Text] {
            let def = if kind == Kind::Prefab {
                Def::Scene(9)
            } else {
                Def::Entry(1038)
            };
            let ch = Chunk {
                kind,
                def,
                version: None,
                c: 0,
                places: places.iter().cloned().map(|p| p.for_kind(kind)).collect(),
            };
            let bytes = ch.encode().unwrap();
            let back = Chunk::decode(&bytes, false).unwrap();
            assert_eq!(back.places.len(), ch.places.len());
            for (a, b) in back.places.iter().zip(&ch.places) {
                assert_eq!(a.n, b.n);
                for (x, y) in a.pos.iter().zip(b.pos) {
                    assert_eq!(x.to_bits(), y.to_bits(), "positions keep their bits");
                }
                assert!(a.scale.same(&b.scale), "{:?} {:?}", a.scale, b.scale);
                assert_eq!(
                    (a.rot, a.colour, a.flags, &a.extra, a.font),
                    (b.rot, b.colour, b.flags, &b.extra, b.font)
                );
            }
            assert_eq!(back.encode().unwrap(), bytes, "pack(unpack(v)) == v");
        }
    }

    #[test]
    fn defaults_cost_what_the_design_says() {
        // a default-sized text: 5 + 96 + 2 + 5 bits = 108 a placement (format.md)
        let places: Vec<Placement> = (0..64).map(|n| Placement::at(n, [n as f32 * 3.4, 1.0, -2.0])).collect();
        let ch = Chunk {
            kind: Kind::Text,
            def: Def::Entry(1038),
            version: None,
            c: 0,
            places,
        };
        let b = ch.encode().unwrap();
        let prefix = Chunk::prefix(Kind::Text, &Def::Entry(1038), None, 0).len();
        assert_eq!(b.len() - prefix, (3 + 64 * 108usize).div_ceil(8));
        // 10k copies: about 136 KB, as format.md works out
        let per_chunk = b.len() as f64;
        let total = per_chunk * (10_000.0 / 64.0);
        eprintln!("10k default text placements: {total:.0} bytes of values");
        assert!((130_000.0..140_000.0).contains(&total), "{total}");
    }

    #[test]
    fn repeated_rotations_and_scales_refer_back() {
        let mut a = block(1, [0.0; 3]);
        a.rot = Some(0xdead_beef);
        a.scale = Scale::Axes([2.0, 3.0, 4.0]);
        let mut b = block(2, [1.0, 0.0, 0.0]);
        b.rot = a.rot;
        b.scale = a.scale;
        let one = Chunk {
            kind: Kind::Block,
            def: Def::Colour([1, 2, 3]),
            version: None,
            c: 0,
            places: vec![a.clone()],
        };
        let two = Chunk {
            places: vec![a, b],
            ..one.clone()
        };
        let (l1, l2) = (one.encode().unwrap().len(), two.encode().unwrap().len());
        // the second costs its gap, position, flag bits and two short references
        assert!(l2 - l1 < 17, "{l1} then {l2}");
        assert_eq!(Chunk::decode(&two.encode().unwrap(), false).unwrap(), two);
    }

    #[test]
    fn scenes_hold_numbered_placements_and_know_where_a_text_is() {
        let dir = std::env::temp_dir().join(format!("anend-scenes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let g = Graph::open(&dir.join("s.redb")).unwrap();
        let room = g.add_scene("room", 0).unwrap();
        let inner = g.add_scene("corner", room).unwrap();
        assert!(g.add_scene("lost", 999).is_err());
        assert_eq!(g.scenes().unwrap()[1].parent, room);
        let text = Def::Entry(42);
        // seventy copies: two chunks
        let ns = g
            .place(
                room,
                false,
                Kind::Text,
                text,
                None,
                (0..70).map(|i| Placement::at(0, [i as f32, 0.0, 0.0])).collect(),
            )
            .unwrap();
        assert_eq!(ns.first(), Some(&1));
        assert_eq!(ns.last(), Some(&70));
        g.place(inner, false, Kind::Text, text, None, vec![Placement::at(0, [0.5; 3])])
            .unwrap();
        assert!(
            g.place(room, false, Kind::Prefab, text, None, vec![]).is_err(),
            "a text isn't a prefab"
        );
        assert_eq!(g.placed_in(text).unwrap(), vec![(room, 70), (inner, 1)]);
        // taking one out keeps the others' numbers; the next new one gets the next number
        assert_eq!(g.unplace(room, false, Kind::Text, text, None, &[3]).unwrap(), 1);
        let left: Vec<u64> = g.placements(room, false).unwrap().iter().map(|p| p.place.n).collect();
        assert!(!left.contains(&3) && left.len() == 69);
        assert_eq!(
            g.place(room, false, Kind::Text, text, None, vec![Placement::at(0, [9.0; 3])])
                .unwrap(),
            vec![71]
        );
        // moving one replaces it by its number
        let mut moved = Placement::at(5, [-1.0, -2.0, -3.0]);
        moved.font = 40;
        g.place(room, false, Kind::Text, text, None, vec![moved.clone()])
            .unwrap();
        let got = g.placements(room, false).unwrap();
        assert_eq!(got.iter().find(|p| p.place.n == 5).unwrap().place, moved);
        assert_eq!(got.len(), 70);
        // a detached copy carries its version, in the other table
        assert!(g.place(room, true, Kind::Text, text, None, vec![]).is_err());
        g.place(
            room,
            true,
            Kind::Text,
            text,
            Some(1_700_000_000_000),
            vec![Placement::at(0, [1.0; 3])],
        )
        .unwrap();
        assert_eq!(g.placements(room, true).unwrap()[0].version, Some(1_700_000_000_000));
        assert_eq!(g.placed_in(text).unwrap(), vec![(room, 71), (inner, 1)]);
        // the last one out of a scene takes the scene off the definition's list
        g.unplace(inner, false, Kind::Text, text, None, &[1]).unwrap();
        assert_eq!(g.placed_in(text).unwrap(), vec![(room, 71)]);
        drop(g);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
