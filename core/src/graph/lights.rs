//! The room's lights: what lights its blocks. A light is flat (ambient, from everywhere;
//! or directional, from one way), a point, or a cone (spot). Each has a colour and an
//! intensity; a point and a cone reach `range` units (fading to nothing there) from where
//! they are; a directional light and a cone point along `dir`; a cone opens `angle`
//! degrees from its axis with a `softness` at its edge.
//!
//! The light is baked: the space computes it onto the blocks' vertices (each face cut
//! into cells about a unit across) when a light or a block changes, not every frame, and
//! nothing about it is stored but the lights themselves (the bake follows from them and
//! the blocks, the same every time). No shadows: a block doesn't shade another.
//!
//! | table | key | holds |
//! |---|---|---|
//! | `graph lights` | light id (u64) | the light as JSON ([`Light`]) |

use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use super::store::{GResult, Graph, err};

pub const LIGHTS: TableDefinition<u64, &[u8]> = TableDefinition::new("graph lights");

/// lights a save holds at most
pub const MAX_LIGHTS: usize = 64;

pub fn ensure_tables(txn: &redb::WriteTransaction) -> GResult<()> {
    txn.open_table(LIGHTS).map_err(err)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LightKind {
    /// from everywhere at once
    #[default]
    Ambient,
    /// from one way, as the sun
    Directional,
    /// from a point, all round
    Point,
    /// from a point, in a cone
    Spot,
}

fn white() -> String {
    "#ffffff".into()
}
fn one() -> f32 {
    1.0
}
fn down() -> [f32; 3] {
    [0.0, -1.0, 0.0]
}
fn ten() -> f32 {
    10.0
}
fn thirty() -> f32 {
    30.0
}
fn soft() -> f32 {
    0.3
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Light {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub kind: LightKind,
    #[serde(default = "white")]
    pub colour: String,
    /// how bright, 0–10 (1 is a face lit square on at full strength)
    #[serde(default = "one")]
    pub intensity: f32,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub z: f32,
    /// the way it shines (a unit vector), for a directional light and a cone
    #[serde(default = "down")]
    pub dir: [f32; 3],
    /// how far a point or a cone reaches, in units
    #[serde(default = "ten")]
    pub range: f32,
    /// a cone's half angle, in degrees
    #[serde(default = "thirty")]
    pub angle: f32,
    /// how soft a cone's edge is, 0 (sharp) to 1
    #[serde(default = "soft")]
    pub softness: f32,
}

impl Default for Light {
    fn default() -> Light {
        Light {
            id: 0,
            kind: LightKind::Ambient,
            colour: white(),
            intensity: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            dir: down(),
            range: 10.0,
            angle: 30.0,
            softness: 0.3,
        }
    }
}

impl Light {
    /// within bounds: a colour, finite numbers, a unit direction
    pub fn checked(mut self) -> Result<Light, String> {
        if !super::textures::is_colour(&self.colour) {
            return Err(format!("{:?} isn't a colour (#rrggbb)", self.colour));
        }
        self.colour = self.colour.to_lowercase();
        let all = [
            self.intensity,
            self.x,
            self.y,
            self.z,
            self.range,
            self.angle,
            self.softness,
        ];
        if all.iter().chain(&self.dir).any(|v| !v.is_finite()) {
            return Err("a light's values are numbers".into());
        }
        let m = super::blocks::MAX_COORD;
        if [self.x, self.y, self.z].iter().any(|v| v.abs() > m) {
            return Err(format!("a light stays within {m} of the origin"));
        }
        self.intensity = self.intensity.clamp(0.0, 10.0);
        self.range = self.range.clamp(0.1, 10_000.0);
        self.angle = self.angle.clamp(1.0, 89.0);
        self.softness = self.softness.clamp(0.0, 1.0);
        let n = self.dir.iter().map(|v| v * v).sum::<f32>().sqrt();
        // a unit already stays as it is, so checking twice changes nothing
        self.dir = if n < 1e-6 {
            down()
        } else if (n - 1.0).abs() < 1e-5 {
            self.dir
        } else {
            self.dir.map(|v| v / n)
        };
        Ok(self)
    }
}

impl Graph {
    /// the lights, by id
    pub fn lights(&self) -> GResult<Vec<Light>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(LIGHTS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (k, v) = item.map_err(err)?;
            if let Ok(mut l) = serde_json::from_slice::<Light>(v.value()) {
                l.id = k.value();
                out.push(l);
            }
        }
        Ok(out)
    }

    /// adds a light (a new id) and answers it
    pub fn add_light(&self, l: Light) -> GResult<Light> {
        let mut l = l.checked()?;
        let txn = self.db().begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(LIGHTS).map_err(err)?;
            use redb::ReadableTableMetadata;
            if t.len().map_err(err)? as usize >= MAX_LIGHTS {
                return Err(format!("a save has at most {MAX_LIGHTS} lights"));
            }
            l.id = t.last().map_err(err)?.map_or(1, |(k, _)| k.value() + 1);
            t.insert(l.id, serde_json::to_vec(&l).map_err(err)?.as_slice())
                .map_err(err)?;
        }
        txn.commit().map_err(err)?;
        Ok(l)
    }

    /// changes a light that's there
    pub fn set_light(&self, l: Light) -> GResult<Light> {
        let l = l.checked()?;
        let txn = self.db().begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(LIGHTS).map_err(err)?;
            if t.get(l.id).map_err(err)?.is_none() {
                return Err(format!("no light {}", l.id));
            }
            t.insert(l.id, serde_json::to_vec(&l).map_err(err)?.as_slice())
                .map_err(err)?;
        }
        txn.commit().map_err(err)?;
        Ok(l)
    }

    /// takes a light out, answering it
    pub fn delete_light(&self, id: u64) -> GResult<Option<Light>> {
        let txn = self.db().begin_write().map_err(err)?;
        let gone = txn
            .open_table(LIGHTS)
            .map_err(err)?
            .remove(id)
            .map_err(err)?
            .and_then(|v| serde_json::from_slice::<Light>(v.value()).ok());
        txn.commit().map_err(err)?;
        Ok(gone.map(|mut l| {
            l.id = id;
            l
        }))
    }

    /// puts lights back with their ids (an undo, a restore)
    pub fn put_lights(&self, lights: &[Light]) -> GResult<usize> {
        let txn = self.db().begin_write().map_err(err)?;
        let mut n = 0;
        {
            let mut t = txn.open_table(LIGHTS).map_err(err)?;
            for l in lights {
                let Ok(l) = l.clone().checked() else { continue };
                if l.id == 0 || t.get(l.id).map_err(err)?.is_some() {
                    continue;
                }
                t.insert(l.id, serde_json::to_vec(&l).map_err(err)?.as_slice())
                    .map_err(err)?;
                n += 1;
            }
        }
        txn.commit().map_err(err)?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lights_are_kept_checked() {
        let g = Graph::in_memory().unwrap();
        let a = g
            .add_light(Light {
                kind: LightKind::Spot,
                colour: "#FFAA00".into(),
                intensity: 40.0,
                dir: [0.0, 0.0, -2.0],
                angle: 120.0,
                ..Light::default()
            })
            .unwrap();
        assert_eq!(
            (a.id, a.colour.as_str(), a.intensity, a.dir, a.angle),
            (1, "#ffaa00", 10.0, [0.0, 0.0, -1.0], 89.0)
        );
        let b = g.add_light(Light::default()).unwrap();
        assert_eq!(b.id, 2);
        assert!(
            g.add_light(Light {
                colour: "red".into(),
                ..Light::default()
            })
            .is_err()
        );
        let moved = g.set_light(Light { x: 3.0, ..a.clone() }).unwrap();
        assert_eq!(g.lights().unwrap(), vec![moved.clone(), b.clone()]);
        assert!(
            g.set_light(Light {
                id: 9,
                ..Light::default()
            })
            .is_err()
        );
        let gone = g.delete_light(1).unwrap().unwrap();
        assert_eq!(g.lights().unwrap(), vec![b]);
        assert_eq!(g.put_lights(&[gone]).unwrap(), 1);
        assert_eq!(g.lights().unwrap().len(), 2);
        // the next id is after the last
        assert_eq!(g.add_light(Light::default()).unwrap().id, 3);
    }
}
