//! Spatial saves: blocks placed in the space, each showing a texture (a text of the save,
//! one of its pictures, or a plain colour) tiled over its faces.
//!
//! | table | key | holds |
//! |---|---|---|
//! | `graph blocks` (multimap) | what the block shows: `text:<entry label>`, `picture:<hash>` or `plain:<#rrggbb>` | one 32 byte value per instance: its id and transform ([`Transform::to_bytes`]) |
//! | `graph block ids` | instance id | its key in `graph blocks` |
//!
//! A texture is stored once (the text is an entry, the picture is in `graph pictures`)
//! and every block showing it is one 32 byte value under the same key, so a thousand
//! copies of one text cost a thousand transforms and nothing more. The values sort by
//! the id (big endian first), so a key's instances come back in the order they were made.
//! Moving or scaling a block replaces its value; `graph block ids` finds the key from the
//! id without a scan. A region query reads every value once (no spatial index yet: the
//! values are small and a save's blocks number in the thousands, not millions).
//!
//! A block's size is in the space's units; its texture tiles at one tile per unit
//! ([`TILE`]), so making a block bigger shows more tiles instead of stretching one.

use redb::{MultimapTableDefinition, ReadableDatabase, ReadableMultimapTable, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use super::store::{GResult, Graph, err};

pub const BLOCKS: MultimapTableDefinition<&str, &[u8]> = MultimapTableDefinition::new("graph blocks");
pub const BLOCK_IDS: TableDefinition<u64, &str> = TableDefinition::new("graph block ids");

/// the side of one texture tile, in space units
pub const TILE: f32 = 1.0;
/// a side at least and at most
pub const MIN_SIDE: f32 = 0.05;
pub const MAX_SIDE: f32 = 10_000.0;
/// how far from the origin a block may be
pub const MAX_COORD: f32 = 1.0e6;

pub fn ensure_tables(txn: &redb::WriteTransaction) -> GResult<()> {
    txn.open_multimap_table(BLOCKS).map_err(err)?;
    txn.open_table(BLOCK_IDS).map_err(err)?;
    Ok(())
}

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

/// where a block is and how big: its centre and its sides
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
    pub h: f32,
    pub d: f32,
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
        }
    }
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
        Ok(Transform {
            w: side(self.w),
            h: side(self.h),
            d: side(self.d),
            ..self
        })
    }

    /// whether the block's box meets the box from `lo` to `hi`
    pub fn meets(&self, lo: [f32; 3], hi: [f32; 3]) -> bool {
        let c = [self.x, self.y, self.z];
        let half = [self.w / 2.0, self.h / 2.0, self.d / 2.0];
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
    pub id: u64,
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

impl Graph {
    fn next_block_id(t: &redb::Table<u64, &str>) -> GResult<u64> {
        Ok(t.last().map_err(err)?.map(|(k, _)| k.value() + 1).unwrap_or(1))
    }

    /// places a block; its id
    pub fn add_block(&self, kind: Kind, texture: &str, t: Transform) -> GResult<u64> {
        Ok(self.add_blocks(&[(kind, texture.to_string(), t)])?[0])
    }

    /// places several blocks in one write; their ids
    pub fn add_blocks(&self, blocks: &[(Kind, String, Transform)]) -> GResult<Vec<u64>> {
        let mut checked = Vec::with_capacity(blocks.len());
        for (kind, texture, t) in blocks {
            checked.push((key(*kind, texture.trim()), t.checked()?));
        }
        let txn = self.db().begin_write().map_err(err)?;
        let ids = {
            let mut ids_t = txn.open_table(BLOCK_IDS).map_err(err)?;
            let mut b = txn.open_multimap_table(BLOCKS).map_err(err)?;
            let first = Self::next_block_id(&ids_t)?;
            let mut out = Vec::with_capacity(checked.len());
            for (id, (k, t)) in (first..).zip(checked.iter()) {
                b.insert(k.as_str(), t.to_bytes(id).as_slice()).map_err(err)?;
                ids_t.insert(id, k.as_str()).map_err(err)?;
                out.push(id);
            }
            out
        };
        txn.commit().map_err(err)?;
        Ok(ids)
    }

    /// one block
    pub fn block(&self, id: u64) -> GResult<Option<Block>> {
        let txn = self.db().begin_read().map_err(err)?;
        let ids = txn.open_table(BLOCK_IDS).map_err(err)?;
        let Some(k) = ids.get(id).map_err(err)?.map(|v| v.value().to_string()) else {
            return Ok(None);
        };
        let b = txn.open_multimap_table(BLOCKS).map_err(err)?;
        for v in b.get(k.as_str()).map_err(err)? {
            let v = v.map_err(err)?;
            if let Some((i, t)) = Transform::from_bytes(v.value())
                && i == id
            {
                let (kind, texture) = split_key(&k).ok_or("a block's key doesn't read")?;
                return Ok(Some(Block {
                    id,
                    kind,
                    texture: texture.to_string(),
                    t,
                }));
            }
        }
        Ok(None)
    }

    /// moves or resizes blocks (each its whole new transform) in one write; the ones
    /// that weren't there are skipped, the rest come back as they are now
    pub fn set_blocks(&self, changes: &[(u64, Transform)]) -> GResult<Vec<Block>> {
        let mut checked = Vec::with_capacity(changes.len());
        for (id, t) in changes {
            checked.push((*id, t.checked()?));
        }
        let txn = self.db().begin_write().map_err(err)?;
        let mut out = Vec::new();
        {
            let ids = txn.open_table(BLOCK_IDS).map_err(err)?;
            let mut b = txn.open_multimap_table(BLOCKS).map_err(err)?;
            for (id, t) in checked {
                let Some(k) = ids.get(id).map_err(err)?.map(|v| v.value().to_string()) else {
                    continue;
                };
                let old: Option<Vec<u8>> = b
                    .get(k.as_str())
                    .map_err(err)?
                    .filter_map(|v| v.ok().map(|v| v.value().to_vec()))
                    .find(|v| v.len() == 32 && v[..8] == id.to_be_bytes());
                if let Some(old) = old {
                    b.remove(k.as_str(), old.as_slice()).map_err(err)?;
                }
                b.insert(k.as_str(), t.to_bytes(id).as_slice()).map_err(err)?;
                let (kind, texture) = split_key(&k).ok_or("a block's key doesn't read")?;
                out.push(Block {
                    id,
                    kind,
                    texture: texture.to_string(),
                    t,
                });
            }
        }
        txn.commit().map_err(err)?;
        Ok(out)
    }

    /// takes blocks away; the ones that were there, as they were (to put them back)
    pub fn delete_blocks(&self, ids: &[u64]) -> GResult<Vec<Block>> {
        let mut out = Vec::new();
        for id in ids {
            if let Some(b) = self.block(*id)? {
                out.push(b);
            }
        }
        let txn = self.db().begin_write().map_err(err)?;
        {
            let mut ids_t = txn.open_table(BLOCK_IDS).map_err(err)?;
            let mut t = txn.open_multimap_table(BLOCKS).map_err(err)?;
            for b in &out {
                let k = key(b.kind, &b.texture);
                t.remove(k.as_str(), b.t.to_bytes(b.id).as_slice()).map_err(err)?;
                ids_t.remove(b.id).map_err(err)?;
            }
        }
        txn.commit().map_err(err)?;
        Ok(out)
    }

    /// puts blocks back with their own ids (an undo, a restore); the ids already in use
    /// are skipped
    pub fn put_blocks(&self, blocks: &[Block]) -> GResult<usize> {
        let txn = self.db().begin_write().map_err(err)?;
        let mut n = 0;
        {
            let mut ids_t = txn.open_table(BLOCK_IDS).map_err(err)?;
            let mut t = txn.open_multimap_table(BLOCKS).map_err(err)?;
            for b in blocks {
                if b.id == 0 || ids_t.get(b.id).map_err(err)?.is_some() {
                    continue;
                }
                let tr = b.t.checked()?;
                let k = key(b.kind, &b.texture);
                t.insert(k.as_str(), tr.to_bytes(b.id).as_slice()).map_err(err)?;
                ids_t.insert(b.id, k.as_str()).map_err(err)?;
                n += 1;
            }
        }
        txn.commit().map_err(err)?;
        Ok(n)
    }

    /// copies of blocks, each moved by `offset`; the new ids
    pub fn duplicate_blocks(&self, ids: &[u64], offset: [f32; 3]) -> GResult<Vec<u64>> {
        let mut new = Vec::new();
        for id in ids {
            let b = self.block(*id)?.ok_or_else(|| format!("no block {id}"))?;
            let mut t = b.t;
            t.x += offset[0];
            t.y += offset[1];
            t.z += offset[2];
            new.push((b.kind, b.texture, t));
        }
        self.add_blocks(&new)
    }

    /// every block, or (with a region) those whose boxes meet it, by id
    pub fn blocks(&self, region: Option<([f32; 3], [f32; 3])>) -> GResult<Vec<Block>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_multimap_table(BLOCKS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (k, vs) = item.map_err(err)?;
            let Some((kind, texture)) = split_key(k.value()) else {
                continue;
            };
            for v in vs {
                let v = v.map_err(err)?;
                let Some((id, tr)) = Transform::from_bytes(v.value()) else {
                    continue;
                };
                if region.is_some_and(|(lo, hi)| !tr.meets(lo, hi)) {
                    continue;
                }
                out.push(Block {
                    id,
                    kind,
                    texture: texture.to_string(),
                    t: tr,
                });
            }
        }
        out.sort_by_key(|b| b.id);
        Ok(out)
    }

    /// the instances of one texture
    pub fn blocks_of(&self, kind: Kind, texture: &str) -> GResult<Vec<Block>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_multimap_table(BLOCKS).map_err(err)?;
        let mut out = Vec::new();
        for v in t.get(key(kind, texture).as_str()).map_err(err)? {
            let v = v.map_err(err)?;
            if let Some((id, tr)) = Transform::from_bytes(v.value()) {
                out.push(Block {
                    id,
                    kind,
                    texture: texture.to_string(),
                    t: tr,
                });
            }
        }
        Ok(out)
    }

    /// how many blocks and distinct textures
    pub fn block_use(&self) -> GResult<BlockUse> {
        use redb::ReadableTableMetadata;
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_multimap_table(BLOCKS).map_err(err)?;
        let mut textures = 0;
        for item in t.iter().map_err(err)? {
            item.map_err(err)?;
            textures += 1;
        }
        Ok(BlockUse {
            blocks: t.len().map_err(err)?,
            textures,
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

    #[test]
    fn transforms_round_trip_and_sort_by_id() {
        let a = Transform {
            x: 1.5,
            y: -2.0,
            z: 3.25,
            w: 4.0,
            h: 0.5,
            d: 2.0,
        };
        assert_eq!(Transform::from_bytes(&a.to_bytes(7)), Some((7, a)));
        assert!(t(0.0, 1.0).to_bytes(2) < t(-9.0, 1.0).to_bytes(300));
        assert!(Transform::from_bytes(&[0; 31]).is_none());
        assert_eq!(a.repeats(), [[4.0, 0.5], [2.0, 0.5], [4.0, 2.0]]);
        assert!(t(f32::NAN, 1.0).checked().is_err());
        assert_eq!(t(0.0, 0.0).checked().unwrap().w, MIN_SIDE);
    }

    #[test]
    fn repeated_textures_share_one_key() {
        let g = Graph::in_memory().unwrap();
        let ids = g
            .add_blocks(&[
                (Kind::Text, "1,2".into(), t(0.0, 1.0)),
                (Kind::Text, "1,2".into(), t(5.0, 2.0)),
                (Kind::Picture, "ab".into(), t(-5.0, 1.0)),
            ])
            .unwrap();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(g.block_use().unwrap(), BlockUse { blocks: 3, textures: 2 });
        assert_eq!(g.blocks_of(Kind::Text, "1,2").unwrap().len(), 2);

        // move and scale replace the value; the key stays
        let moved = g.set_blocks(&[(2, t(9.0, 3.0)), (99, t(0.0, 1.0))]).unwrap();
        assert_eq!(moved.len(), 1);
        assert_eq!(g.block(2).unwrap().unwrap().t, t(9.0, 3.0));
        assert_eq!(g.block_use().unwrap().blocks, 3);

        // a region
        let near: Vec<u64> = g
            .blocks(Some(([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0])))
            .unwrap()
            .iter()
            .map(|b| b.id)
            .collect();
        assert_eq!(near, vec![1]);

        // duplicate, delete, put back
        let copies = g.duplicate_blocks(&[1, 3], [0.0, 2.0, 0.0]).unwrap();
        assert_eq!(copies, vec![4, 5]);
        assert_eq!(g.block(5).unwrap().unwrap().t.y, 2.0);
        assert_eq!(g.blocks_of(Kind::Picture, "ab").unwrap().len(), 2);
        let gone = g.delete_blocks(&[1, 4]).unwrap();
        assert_eq!(gone.len(), 2);
        assert_eq!(g.blocks_of(Kind::Text, "1,2").unwrap().len(), 1);
        assert_eq!(g.put_blocks(&gone).unwrap(), 2);
        assert_eq!(g.put_blocks(&gone).unwrap(), 0);
        assert_eq!(g.blocks(None).unwrap().len(), 5);
        // the next id is past the highest
        assert_eq!(g.add_block(Kind::Plain, "#ff0000", t(0.0, 1.0)).unwrap(), 6);
    }
}
