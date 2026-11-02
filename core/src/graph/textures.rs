//! Texture recipes: how a block's face (or the sky) is textured, as steps from a source.
//!
//! A recipe names a **source** (a picture of the save, a text of the save drawn as a
//! tile, or a plain colour), then the **steps**: a crop of the source, the tile's size in
//! space units, its offset, a quarter turn, a mirror, a tint, and how many **variants**
//! the blocks showing it pick from. A recipe is kept by its content hash (FNV-1a 128 over
//! its normalized JSON, as pictures are), so equal recipes are one row, shared by every
//! block and face that shows it; blocks and faces refer to the hash. Nothing is rendered
//! into the save: the space makes the tile from the recipe each time, the same way on the
//! desktop and the web.
//!
//! | table | key | holds |
//! |---|---|---|
//! | `graph textures` | recipe id (32 hex digits) | [`Saved`] as JSON: the recipe, its name, when it was made |
//!
//! Variants: a block picks variant [`variant`]`(seed, n, variants)` by its number `n`
//! among its texture's blocks, a deterministic hash; variant k shifts the tile by
//! (k·0.381966, k·0.618034) of a tile and turns it k quarter turns (when `turn_variants`),
//! so a wall of blocks showing one recipe doesn't repeat visibly.

use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use super::assets::picture_id;
use super::store::{GResult, Graph, err};

pub const TEXTURES: TableDefinition<&str, &[u8]> = TableDefinition::new("graph textures");

pub fn ensure_tables(txn: &redb::WriteTransaction) -> GResult<()> {
    txn.open_table(TEXTURES).map_err(err)?;
    Ok(())
}

/// where a recipe's pixels come from
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// a picture of the save, by its id (32 hex digits); generated pictures are pictures
    Picture(String),
    /// a text of the save, by its entry label, drawn as a tile
    Text(String),
    /// a plain colour, `#rrggbb`
    Plain(String),
}

fn full() -> [f32; 4] {
    [0.0, 0.0, 1.0, 1.0]
}
fn one() -> [f32; 2] {
    [1.0, 1.0]
}
fn single() -> u8 {
    1
}

/// a texture recipe: a source and the steps from it to a tile
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub source: Source,
    /// the part of the source used: left, top, right, bottom as fractions of it
    #[serde(default = "full")]
    pub crop: [f32; 4],
    /// one tile's width and height in space units
    #[serde(default = "one")]
    pub scale: [f32; 2],
    /// the tile shifted by this fraction of itself across and down
    #[serde(default)]
    pub offset: [f32; 2],
    /// quarter turns of the tile, 0–3
    #[serde(default)]
    pub turn: u8,
    /// the tile mirrored left to right
    #[serde(default)]
    pub mirror: bool,
    /// a colour laid over it, `#rrggbb`, and how strongly (0–1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<String>,
    #[serde(default)]
    pub tint_amount: f32,
    /// how many variants the blocks showing it pick from (1: none), and the seed they're
    /// picked by
    #[serde(default = "single")]
    pub variants: u8,
    #[serde(default)]
    pub seed: u32,
    /// whether variants turn the tile too (not for a picture with an up)
    #[serde(default)]
    pub turn_variants: bool,
}

/// a saved recipe: its id, what it's called and when it was made
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub recipe: Recipe,
    #[serde(default)]
    pub added: u64,
}

pub fn is_colour(t: &str) -> bool {
    t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// a recipe id: 32 hex digits
pub fn is_id(t: &str) -> bool {
    t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit())
}

/// a value rounded to 1/10000, so the same choices hash the same
fn r4(v: f32) -> f32 {
    (v * 10_000.0).round() / 10_000.0
}

impl Recipe {
    pub fn plain(colour: &str) -> Recipe {
        Recipe {
            source: Source::Plain(colour.to_string()),
            crop: full(),
            scale: one(),
            offset: [0.0; 2],
            turn: 0,
            mirror: false,
            tint: None,
            tint_amount: 0.0,
            variants: 1,
            seed: 0,
            turn_variants: false,
        }
    }

    /// within bounds, rounded, and with what doesn't matter left at its default, so equal
    /// choices make one id
    pub fn normalized(&self) -> Result<Recipe, String> {
        let vals = self
            .crop
            .iter()
            .chain(&self.scale)
            .chain(&self.offset)
            .chain([&self.tint_amount]);
        if vals.into_iter().any(|v| !v.is_finite()) {
            return Err("a recipe's values are numbers".into());
        }
        let source = match &self.source {
            Source::Picture(id) if is_id(id) => Source::Picture(id.to_lowercase()),
            Source::Picture(id) => return Err(format!("{id:?} isn't a picture's id (32 hex digits)")),
            Source::Text(l) if super::record::parse_label(l).is_some() => Source::Text(l.trim().to_string()),
            Source::Text(l) => return Err(format!("{l:?} isn't a text's key")),
            Source::Plain(c) if is_colour(c) => Source::Plain(c.to_lowercase()),
            Source::Plain(c) => return Err(format!("{c:?} isn't a colour (#rrggbb)")),
        };
        let c = self.crop.map(|v| r4(v.clamp(0.0, 1.0)));
        let (mut l, mut t, mut rr, mut b) = (c[0].min(c[2]), c[1].min(c[3]), c[0].max(c[2]), c[1].max(c[3]));
        // at least a hundredth of the source each way
        if rr - l < 0.01 {
            rr = (l + 0.01).min(1.0);
            l = rr - 0.01;
        }
        if b - t < 0.01 {
            b = (t + 0.01).min(1.0);
            t = b - 0.01;
        }
        let plain = matches!(source, Source::Plain(_));
        let tint = self.tint.as_deref().filter(|c| is_colour(c)).map(str::to_lowercase);
        let tint_amount = if tint.is_some() {
            r4(self.tint_amount.clamp(0.0, 1.0))
        } else {
            0.0
        };
        let (tint, tint_amount) = if tint_amount == 0.0 {
            (None, 0.0)
        } else {
            (tint, tint_amount)
        };
        let variants = self.variants.clamp(1, 16);
        Ok(Recipe {
            crop: if plain { full() } else { [r4(l), r4(t), r4(rr), r4(b)] },
            scale: self.scale.map(|v| r4(v.clamp(0.05, 100.0))),
            offset: self.offset.map(|v| r4(v.rem_euclid(1.0)) % 1.0),
            turn: self.turn % 4,
            mirror: self.mirror && !plain,
            tint,
            tint_amount,
            variants,
            seed: if variants > 1 { self.seed } else { 0 },
            turn_variants: variants > 1 && self.turn_variants,
            source,
        })
    }

    /// its id: the hash of its normalized JSON
    pub fn id(&self) -> Result<String, String> {
        let n = self.normalized()?;
        Ok(picture_id(&serde_json::to_vec(&n).map_err(err)?))
    }
}

/// which variant (0 to variants − 1) the n-th block showing a recipe shows; the space
/// computes the same (`space.js` `variantOf`)
pub fn variant(seed: u32, n: u64, variants: u8) -> u8 {
    if variants <= 1 {
        return 0;
    }
    // a 32-bit mix (Math.imul-friendly, so the page gets the same)
    let mut h = seed ^ (n as u32).wrapping_mul(0x9e37_79b9) ^ ((n >> 32) as u32);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    (h % variants as u32) as u8
}

impl Graph {
    /// every saved recipe, oldest first
    pub fn textures(&self) -> GResult<Vec<Saved>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(TEXTURES).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (_, v) = item.map_err(err)?;
            if let Ok(s) = serde_json::from_slice::<Saved>(v.value()) {
                out.push(s);
            }
        }
        out.sort_by(|a, b| a.added.cmp(&b.added).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    pub fn texture(&self, id: &str) -> GResult<Option<Saved>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(TEXTURES).map_err(err)?;
        match t.get(id).map_err(err)? {
            Some(v) => Ok(serde_json::from_slice(v.value()).ok()),
            None => Ok(None),
        }
    }

    /// keeps a recipe (normalized) and answers its id; one that's there already keeps its
    /// row, renamed when `name` isn't empty
    pub fn put_texture(&self, recipe: &Recipe, name: &str, added: u64) -> GResult<Saved> {
        let recipe = recipe.normalized()?;
        let id = recipe.id()?;
        let saved = match self.texture(&id)? {
            Some(mut s) => {
                if name.trim().is_empty() {
                    return Ok(s);
                }
                s.name = name.trim().to_string();
                s
            }
            None => Saved {
                id: id.clone(),
                name: name.trim().to_string(),
                recipe,
                added,
            },
        };
        let txn = self.db().begin_write().map_err(err)?;
        txn.open_table(TEXTURES)
            .map_err(err)?
            .insert(id.as_str(), serde_json::to_vec(&saved).map_err(err)?.as_slice())
            .map_err(err)?;
        txn.commit().map_err(err)?;
        Ok(saved)
    }

    /// takes a recipe out (blocks and faces still showing it draw as missing)
    pub fn delete_texture(&self, id: &str) -> GResult<bool> {
        let txn = self.db().begin_write().map_err(err)?;
        let had = txn
            .open_table(TEXTURES)
            .map_err(err)?
            .remove(id)
            .map_err(err)?
            .is_some();
        txn.commit().map_err(err)?;
        Ok(had)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic() -> Recipe {
        Recipe {
            source: Source::Picture("0123456789abcdef0123456789ABCDEF".into()),
            crop: [0.5, 0.5, 0.1, 0.2],
            scale: [2.0, 0.0],
            offset: [1.25, -0.25],
            turn: 6,
            mirror: true,
            tint: Some("#FF0000".into()),
            tint_amount: 0.30001,
            variants: 1,
            seed: 99,
            turn_variants: true,
        }
    }

    #[test]
    fn normalized_recipes_share_an_id() {
        let n = pic().normalized().unwrap();
        assert_eq!(n.crop, [0.1, 0.2, 0.5, 0.5]);
        assert_eq!(n.scale, [2.0, 0.05]);
        assert_eq!(n.offset, [0.25, 0.75]);
        assert_eq!(n.turn, 2);
        assert_eq!((n.seed, n.turn_variants), (0, false));
        assert_eq!(n.tint.as_deref(), Some("#ff0000"));
        assert_eq!(n.tint_amount, 0.3);
        assert_eq!(pic().id().unwrap(), n.id().unwrap());
        // a plain colour has no crop or mirror; no tint amount means no tint
        let mut p = Recipe::plain("#AABBCC");
        p.crop = [0.2, 0.2, 0.3, 0.3];
        p.mirror = true;
        p.tint = Some("#000000".into());
        assert_eq!(p.id().unwrap(), Recipe::plain("#aabbcc").id().unwrap());
        assert!(Recipe::plain("red").normalized().is_err());
        let mut bad = pic();
        bad.scale[0] = f32::NAN;
        assert!(bad.normalized().is_err());
    }

    #[test]
    fn variants_are_spread_and_repeatable() {
        let picks: Vec<u8> = (0..400).map(|n| variant(7, n, 4)).collect();
        assert_eq!(picks, (0..400).map(|n| variant(7, n, 4)).collect::<Vec<_>>());
        for k in 0..4 {
            let c = picks.iter().filter(|&&p| p == k).count();
            assert!((60..140).contains(&c), "variant {k}: {c}");
        }
        assert!((0..50).all(|n| variant(7, n, 1) == 0));
        // the page's copy of the mix gives these (space.js variantOf)
        assert_eq!(
            [variant(1, 1, 16), variant(1, 2, 16), variant(12345, 77, 16)],
            [3, 0, 1]
        );
    }

    #[test]
    fn textures_are_kept_by_their_recipe() {
        let dir = std::env::temp_dir().join(format!("anend-tex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Graph::open(&dir.join("t.redb")).unwrap();
        let a = g.put_texture(&pic(), "bricks", 1).unwrap();
        let b = g.put_texture(&pic().normalized().unwrap(), "", 2).unwrap();
        assert_eq!(a, b);
        assert_eq!(g.textures().unwrap().len(), 1);
        let c = g.put_texture(&Recipe::plain("#112233"), "", 3).unwrap();
        assert_ne!(c.id, a.id);
        assert!(g.delete_texture(&a.id).unwrap());
        assert_eq!(g.textures().unwrap(), vec![c]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
