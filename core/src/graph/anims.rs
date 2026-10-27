//! Animations: keyframes for blocks, as CSS has them. An animation is an entry of the save
//! (mime `application/x-anend-animation`, its text the animation's JSON), so one animation
//! is shared by every block that plays it and travels like any entry; a block refers to
//! it from its placement ([`AnimRef`], with a delay and a speed of its own).
//!
//! An animation lasts `duration` seconds and plays once, in a loop, or back and forth
//! (`repeat`). Its keys are moments (`at`, 0 to 1 of the duration) with any of: a move (units,
//! added to the block's place), a turn (degrees about x, y and z, after the block's own),
//! a scale (times the block's sides) and a texture (from that moment on). Between two keys
//! each value eases along a timing curve, a CSS cubic-bezier (the key's own, or the
//! animation's); a value a key leaves out is carried between the keys that set it, and
//! is at rest (no move, no turn, scale 1) where none does. [`sample`] is the reference
//! the space follows (`space.js` `animPose`).

use serde::{Deserialize, Serialize};

/// how an animation repeats
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    /// plays once and stays at its end
    #[default]
    Once,
    /// starts again from the beginning
    Loop,
    /// plays forward, then back
    Alternate,
}

fn ease() -> [f32; 4] {
    [0.25, 0.1, 0.25, 1.0]
}
fn two() -> f32 {
    2.0
}

/// one moment of an animation
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// when, 0 (the start) to 1 (the end)
    pub at: f32,
    #[serde(default, rename = "move", skip_serializing_if = "Option::is_none")]
    pub moved: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<[f32; 3]>,
    /// a texture key (`plain:#rrggbb`, `texture:<id>`, ...) shown from this moment on
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    /// the timing from this key to the next, when not the animation's
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    #[serde(default)]
    pub name: String,
    /// seconds
    #[serde(default = "two")]
    pub duration: f32,
    #[serde(default)]
    pub repeat: Repeat,
    /// the timing between keys: a cubic-bezier's two control points (x1, y1, x2, y2)
    #[serde(default = "ease")]
    pub timing: [f32; 4],
    #[serde(default)]
    pub keys: Vec<Key>,
}

impl Default for Animation {
    fn default() -> Animation {
        Animation {
            name: String::new(),
            duration: 2.0,
            repeat: Repeat::Once,
            timing: ease(),
            keys: Vec::new(),
        }
    }
}

/// keys an animation has at most
pub const MAX_KEYS: usize = 64;

fn curve(t: [f32; 4]) -> Result<[f32; 4], String> {
    if t.iter().any(|v| !v.is_finite()) {
        return Err("a timing's values are numbers".into());
    }
    // x stays in 0..1 so the curve is a function of time; y may overshoot
    Ok([
        t[0].clamp(0.0, 1.0),
        t[1].clamp(-2.0, 3.0),
        t[2].clamp(0.0, 1.0),
        t[3].clamp(-2.0, 3.0),
    ])
}

fn finite3(v: Option<[f32; 3]>, lo: f32, hi: f32, what: &str) -> Result<Option<[f32; 3]>, String> {
    match v {
        Some(a) if a.iter().any(|x| !x.is_finite()) => Err(format!("a {what}'s values are numbers")),
        Some(a) => Ok(Some(a.map(|x| x.clamp(lo, hi)))),
        None => Ok(None),
    }
}

impl Animation {
    /// within bounds, its keys in time order (one key a moment: a later one at the same
    /// moment wins)
    pub fn checked(mut self) -> Result<Animation, String> {
        if !self.duration.is_finite() {
            return Err("a duration is a number".into());
        }
        self.name = self.name.trim().chars().take(200).collect();
        self.duration = self.duration.clamp(0.05, 3600.0);
        self.timing = curve(self.timing)?;
        if self.keys.len() > MAX_KEYS {
            return Err(format!("an animation has at most {MAX_KEYS} keys"));
        }
        let mut keys: Vec<Key> = Vec::with_capacity(self.keys.len());
        for k in self.keys {
            if !k.at.is_finite() {
                return Err("a key's moment is a number".into());
            }
            let k = Key {
                at: (k.at.clamp(0.0, 1.0) * 10_000.0).round() / 10_000.0,
                moved: finite3(k.moved, -1.0e5, 1.0e5, "move")?,
                turn: finite3(k.turn, -3600.0, 3600.0, "turn")?,
                scale: finite3(k.scale, 0.01, 100.0, "scale")?,
                texture: k.texture.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()),
                timing: k.timing.map(curve).transpose()?,
            };
            match keys.iter_mut().find(|x| x.at == k.at) {
                Some(x) => *x = k,
                None => keys.push(k),
            }
        }
        keys.sort_by(|a, b| a.at.total_cmp(&b.at));
        self.keys = keys;
        Ok(self)
    }

    /// where in the animation (0 to 1) the time `t` seconds after it started falls
    pub fn phase(&self, t: f32) -> f32 {
        let d = self.duration.max(0.05);
        let t = t.max(0.0) / d;
        match self.repeat {
            Repeat::Once => t.min(1.0),
            Repeat::Loop => t.fract(),
            Repeat::Alternate => {
                let f = t.rem_euclid(2.0);
                if f <= 1.0 { f } else { 2.0 - f }
            }
        }
    }
}

/// a cubic-bezier timing at x (time, 0 to 1): the curve's y there, as CSS computes it
pub fn bezier(t: [f32; 4], x: f32) -> f32 {
    let [x1, y1, x2, y2] = t;
    let at = |a: f32, b: f32, s: f32| 3.0 * (1.0 - s) * (1.0 - s) * s * a + 3.0 * (1.0 - s) * s * s * b + s * s * s;
    // the ends exactly
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // the curve's parameter whose x is x: bisection, deterministic and good to 1e-6
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if at(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(y1, y2, (lo + hi) / 2.0)
}

/// a block's pose at phase `p`: its move, turn (degrees), scale and texture (None: its own)
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub moved: [f32; 3],
    pub turn: [f32; 3],
    pub scale: [f32; 3],
    pub texture: Option<String>,
}

/// the pose at phase `p` (0 to 1)
pub fn sample(a: &Animation, p: f32) -> Pose {
    let field = |get: &dyn Fn(&Key) -> Option<[f32; 3]>, rest: [f32; 3]| -> [f32; 3] {
        let set: Vec<(&Key, [f32; 3])> = a.keys.iter().filter_map(|k| get(k).map(|v| (k, v))).collect();
        if set.is_empty() {
            return rest;
        }
        // before the first key that sets it: from rest at 0 (or its value, when at 0)
        let first = set[0];
        if p <= first.0.at {
            if first.0.at <= 0.0 {
                return first.1;
            }
            let u = bezier(a.timing, p / first.0.at);
            return [0, 1, 2].map(|i| rest[i] + (first.1[i] - rest[i]) * u);
        }
        for w in set.windows(2) {
            let (k0, v0) = w[0];
            let (k1, v1) = w[1];
            if p <= k1.at {
                let span = (k1.at - k0.at).max(1e-6);
                let u = bezier(k0.timing.unwrap_or(a.timing), (p - k0.at) / span);
                return [0, 1, 2].map(|i| v0[i] + (v1[i] - v0[i]) * u);
            }
        }
        set[set.len() - 1].1
    };
    let texture = a
        .keys
        .iter()
        .rfind(|k| k.at <= p && k.texture.is_some())
        .and_then(|k| k.texture.clone());
    Pose {
        moved: field(&|k| k.moved, [0.0; 3]),
        turn: field(&|k| k.turn, [0.0; 3]),
        scale: field(&|k| k.scale, [1.0; 3]),
        texture,
    }
}

/// a block's animation: the entry, how long after the room opens it starts, how fast
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimRef {
    pub entry: u64,
    pub delay: f32,
    pub speed: f32,
}

impl AnimRef {
    pub fn checked(self) -> AnimRef {
        let f = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        AnimRef {
            entry: self.entry,
            delay: f(self.delay, 0.0, 3600.0, 0.0),
            speed: f(self.speed, 0.01, 100.0, 1.0),
        }
    }

    /// `A`, version 1, the entry (u64), the delay and the speed (f32s), little endian
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&[b'A', 1]);
        out.extend_from_slice(&self.entry.to_le_bytes());
        out.extend_from_slice(&self.delay.to_le_bytes());
        out.extend_from_slice(&self.speed.to_le_bytes());
    }

    pub fn read(b: &[u8]) -> Option<(AnimRef, usize)> {
        if b.len() < 18 || b[0] != b'A' || b[1] != 1 {
            return None;
        }
        let f = |i: usize| f32::from_le_bytes(b[i..i + 4].try_into().unwrap_or([0; 4]));
        Some((
            AnimRef {
                entry: u64::from_le_bytes(b[2..10].try_into().ok()?),
                delay: f(10),
                speed: f(14),
            },
            18,
        ))
    }
}

impl Serialize for AnimRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(3))?;
        m.serialize_entry("key", &super::record::label(self.entry))?;
        m.serialize_entry("delay", &self.delay)?;
        m.serialize_entry("speed", &self.speed)?;
        m.end()
    }
}

impl<'de> Deserialize<'de> for AnimRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<AnimRef, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            key: String,
            #[serde(default)]
            delay: f32,
            #[serde(default = "one")]
            speed: f32,
        }
        fn one() -> f32 {
            1.0
        }
        let r = Raw::deserialize(d)?;
        let entry = super::record::parse_label(&r.key)
            .ok_or_else(|| serde::de::Error::custom(format!("{:?} isn't an animation's key", r.key)))?;
        Ok(AnimRef {
            entry,
            delay: r.delay,
            speed: r.speed,
        }
        .checked())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bob() -> Animation {
        Animation {
            name: " bob ".into(),
            duration: 4.0,
            repeat: Repeat::Alternate,
            timing: [0.0, 0.0, 1.0, 1.0],
            keys: vec![
                Key {
                    at: 1.0,
                    moved: Some([0.0, 2.0, 0.0]),
                    texture: Some("plain:#ff0000".into()),
                    ..Key::default()
                },
                Key {
                    at: 0.0,
                    moved: Some([0.0, 0.0, 0.0]),
                    ..Key::default()
                },
                Key {
                    at: 0.5,
                    turn: Some([0.0, 90.0, 0.0]),
                    ..Key::default()
                },
            ],
        }
        .checked()
        .unwrap()
    }

    #[test]
    fn keys_ease_between_moments() {
        let a = bob();
        assert_eq!(a.name, "bob");
        assert_eq!(a.keys.iter().map(|k| k.at).collect::<Vec<_>>(), vec![0.0, 0.5, 1.0]);
        // a linear timing: halfway up at half time
        let p = sample(&a, 0.5);
        assert!((p.moved[1] - 1.0).abs() < 1e-4, "{p:?}");
        // the turn rises from rest to 90 by the middle, then holds
        assert!((sample(&a, 0.25).turn[1] - 45.0).abs() < 1e-3);
        assert_eq!(sample(&a, 0.9).turn[1], 90.0);
        assert_eq!(sample(&a, 0.3).scale, [1.0; 3]);
        // the texture from its key on
        assert_eq!(sample(&a, 0.99).texture, None);
        assert_eq!(sample(&a, 1.0).texture.as_deref(), Some("plain:#ff0000"));
        // back and forth
        assert_eq!(a.phase(2.0), 0.5);
        assert_eq!(a.phase(6.0), 0.5);
        assert_eq!(a.phase(5.0), 0.75);
        let once = Animation {
            repeat: Repeat::Once,
            ..a.clone()
        };
        assert_eq!(once.phase(9.0), 1.0);
        let l = Animation {
            repeat: Repeat::Loop,
            ..a
        };
        assert_eq!(l.phase(5.0), 0.25);
    }

    #[test]
    fn css_timing() {
        // ease: slow start, fast middle; linear is the line
        assert!(bezier([0.25, 0.1, 0.25, 1.0], 0.1) < 0.1 + 0.1);
        assert!((bezier([0.0, 0.0, 1.0, 1.0], 0.3) - 0.3).abs() < 1e-4);
        assert!((bezier([0.42, 0.0, 0.58, 1.0], 0.5) - 0.5).abs() < 1e-4);
        assert_eq!(bezier([0.25, 0.1, 0.25, 1.0], 0.0), 0.0);
        assert_eq!(bezier([0.25, 0.1, 0.25, 1.0], 1.0), 1.0);
    }

    #[test]
    fn refs_are_packed() {
        let r = AnimRef {
            entry: 77,
            delay: 1.5,
            speed: 2.0,
        };
        let mut b = Vec::new();
        r.write(&mut b);
        assert_eq!(AnimRef::read(&b), Some((r, 18)));
        let j = serde_json::to_value(r).unwrap();
        let back: AnimRef = serde_json::from_value(j).unwrap();
        assert_eq!(back, r);
        assert!(
            Animation {
                duration: f32::NAN,
                ..Animation::default()
            }
            .checked()
            .is_err()
        );
    }
}
