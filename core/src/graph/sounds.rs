//! Sounds (queued request #17): sound files kept in the save, as pictures are, each known by
//! the hash of its bytes, and played by blocks that are sound emitters.
//!
//! | table | key | holds |
//! |---|---|---|
//! | `graph sounds` | sound id (content hash, 32 hex digits) | its details as JSON ([`Sound`]) |
//! | `graph sound bytes` | sound id | the file's bytes, as added |
//!
//! A block's emitter ([`Emitter`], record `S` in its placement's extra) names a sound, how
//! loud it is, how far it carries (it fades to nothing at that distance) and whether it
//! loops. The room plays emitters while it's playing: looping ones from the start, the
//! others when a program says (`sound`, `hush`). The sound is played by the page (Web
//! Audio), placed where the block is and heard from where you are.

use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use super::store::{GResult, Graph, err};

pub const SOUNDS: TableDefinition<&str, &[u8]> = TableDefinition::new("graph sounds");
pub const SOUND_BYTES: TableDefinition<&str, &[u8]> = TableDefinition::new("graph sound bytes");

pub fn ensure_tables(txn: &redb::WriteTransaction) -> GResult<()> {
    txn.open_table(SOUNDS).map_err(err)?;
    txn.open_table(SOUND_BYTES).map_err(err)?;
    Ok(())
}

/// a sound's details (its bytes are kept apart)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sound {
    /// the content hash of its bytes (`assets::picture_id`'s hash)
    pub id: String,
    /// `audio/wav`, `audio/ogg`, `audio/mpeg`, `audio/flac`, `audio/webm` or `audio/mp4`
    pub mime: String,
    /// what it was called when added (a file name), or what it was named since
    #[serde(default)]
    pub name: String,
    /// when it was added, milliseconds since the Unix epoch
    pub added: u64,
    /// its bytes' length
    pub size: u64,
    /// how long it lasts, in seconds, when the file says (a WAV's header)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f32>,
}

/// what a sound file is, by its first bytes: its mime, and (for a WAV) how long it lasts
pub fn sniff(b: &[u8]) -> Option<(&'static str, Option<f32>)> {
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE" {
        return Some(("audio/wav", wav_seconds(b)));
    }
    if b.starts_with(b"OggS") {
        return Some(("audio/ogg", None));
    }
    if b.starts_with(b"fLaC") {
        return Some(("audio/flac", None));
    }
    if b.starts_with(b"ID3") || (b.len() >= 2 && b[0] == 0xFF && b[1] & 0xE0 == 0xE0) {
        return Some(("audio/mpeg", None));
    }
    if b.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(("audio/webm", None));
    }
    if b.len() >= 8 && &b[4..8] == b"ftyp" {
        return Some(("audio/mp4", None));
    }
    None
}

/// a WAV's length: its data chunk's bytes over its byte rate
fn wav_seconds(b: &[u8]) -> Option<f32> {
    let u32_at = |i: usize| -> Option<u32> { Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?)) };
    let mut at = 12;
    let mut rate = None;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32_at(at + 4)? as usize;
        if id == b"fmt " {
            rate = u32_at(at + 16);
        } else if id == b"data" {
            let r = rate.filter(|r| *r > 0)?;
            return Some(len as f32 / r as f32);
        }
        at += 8 + len + (len & 1);
    }
    None
}

/// a block's sound emitter
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emitter {
    /// the sound's id (its content hash)
    pub id: [u8; 16],
    /// 0 (silent) to 1 (as recorded), up to 4
    pub volume: f32,
    /// how far it carries: it fades to nothing at this distance
    pub range: f32,
    /// whether it loops (and starts when the room starts playing)
    pub looped: bool,
}

impl Emitter {
    pub fn id_hex(&self) -> String {
        self.id.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn checked(self) -> Emitter {
        let f = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        Emitter {
            id: self.id,
            volume: f(self.volume, 0.0, 4.0, 1.0),
            range: f(self.range, 0.5, 1000.0, 20.0),
            looped: self.looped,
        }
    }

    /// `S`, version 1, the id (16 bytes), the volume and the range (f32s), a byte of flags
    /// (1: loops)
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&[b'S', 1]);
        out.extend_from_slice(&self.id);
        out.extend_from_slice(&self.volume.to_le_bytes());
        out.extend_from_slice(&self.range.to_le_bytes());
        out.push(u8::from(self.looped));
    }

    pub fn read(b: &[u8]) -> Option<(Emitter, usize)> {
        if b.len() < 27 || b[0] != b'S' || b[1] != 1 {
            return None;
        }
        let f = |i: usize| f32::from_le_bytes(b[i..i + 4].try_into().unwrap_or([0; 4]));
        Some((
            Emitter {
                id: b[2..18].try_into().ok()?,
                volume: f(18),
                range: f(22),
                looped: b[26] & 1 != 0,
            },
            27,
        ))
    }
}

#[derive(Serialize, Deserialize)]
struct EmitterJson {
    id: String,
    #[serde(default = "one")]
    volume: f32,
    #[serde(default = "twenty")]
    range: f32,
    #[serde(default, rename = "loop")]
    looped: bool,
}

fn one() -> f32 {
    1.0
}
fn twenty() -> f32 {
    20.0
}

impl Serialize for Emitter {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        EmitterJson {
            id: self.id_hex(),
            volume: self.volume,
            range: self.range,
            looped: self.looped,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Emitter {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Emitter, D::Error> {
        let j = EmitterJson::deserialize(d)?;
        let h = j.id.trim();
        let mut id = [0u8; 16];
        if h.len() != 32 {
            return Err(serde::de::Error::custom(format!("{h:?} isn't a sound's id")));
        }
        for (i, slot) in id.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&h[2 * i..2 * i + 2], 16)
                .map_err(|_| serde::de::Error::custom(format!("{h:?} isn't a sound's id")))?;
        }
        Ok(Emitter {
            id,
            volume: j.volume,
            range: j.range,
            looped: j.looped,
        }
        .checked())
    }
}

impl Graph {
    /// every sound's details, oldest first
    pub fn sounds(&self) -> GResult<Vec<Sound>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(SOUNDS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (_, v) = item.map_err(err)?;
            out.push(serde_json::from_slice::<Sound>(v.value()).map_err(err)?);
        }
        out.sort_by(|a, b| a.added.cmp(&b.added).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    pub fn sound(&self, id: &str) -> GResult<Option<Sound>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(SOUNDS).map_err(err)?;
        match t.get(id).map_err(err)? {
            Some(v) => Ok(Some(serde_json::from_slice(v.value()).map_err(err)?)),
            None => Ok(None),
        }
    }

    pub fn sound_bytes(&self, id: &str) -> GResult<Option<Vec<u8>>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(SOUND_BYTES).map_err(err)?;
        Ok(t.get(id).map_err(err)?.map(|v| v.value().to_vec()))
    }

    /// stores a sound (its `id` must be its bytes' hash); false when it was there already
    pub fn put_sound(&self, s: &Sound, bytes: &[u8]) -> GResult<bool> {
        if s.id != super::assets::picture_id(bytes) {
            return Err("a sound's id is its bytes' hash".into());
        }
        let txn = self.db().begin_write().map_err(err)?;
        let new = {
            let mut t = txn.open_table(SOUNDS).map_err(err)?;
            if t.get(s.id.as_str()).map_err(err)?.is_some() {
                false
            } else {
                t.insert(s.id.as_str(), serde_json::to_vec(s).map_err(err)?.as_slice())
                    .map_err(err)?;
                txn.open_table(SOUND_BYTES)
                    .map_err(err)?
                    .insert(s.id.as_str(), bytes)
                    .map_err(err)?;
                true
            }
        };
        txn.commit().map_err(err)?;
        Ok(new)
    }

    pub fn name_sound(&self, id: &str, name: &str) -> GResult<()> {
        let mut s = self.sound(id)?.ok_or_else(|| format!("no sound {id:?}"))?;
        s.name = name.to_string();
        let txn = self.db().begin_write().map_err(err)?;
        txn.open_table(SOUNDS)
            .map_err(err)?
            .insert(id, serde_json::to_vec(&s).map_err(err)?.as_slice())
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    /// takes a sound out of the save (blocks naming it play nothing); whether it was there
    pub fn delete_sound(&self, id: &str) -> GResult<bool> {
        let txn = self.db().begin_write().map_err(err)?;
        let found = {
            let found = txn.open_table(SOUNDS).map_err(err)?.remove(id).map_err(err)?.is_some();
            txn.open_table(SOUND_BYTES).map_err(err)?.remove(id).map_err(err)?;
            found
        };
        txn.commit().map_err(err)?;
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a WAV of `n` silent 16-bit mono samples at 8000 Hz
    pub fn wav(n: usize) -> Vec<u8> {
        let data = n * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&((36 + data) as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&8000u32.to_le_bytes());
        b.extend_from_slice(&16000u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data as u32).to_le_bytes());
        b.resize(44 + data, 0);
        b
    }

    #[test]
    fn sounds_keep_and_sniff() {
        let b = wav(4000);
        assert_eq!(sniff(&b), Some(("audio/wav", Some(0.5))));
        assert_eq!(sniff(b"OggS...."), Some(("audio/ogg", None)));
        assert_eq!(sniff(b"ID3\x04"), Some(("audio/mpeg", None)));
        assert_eq!(sniff(b"hello"), None);
        let g = Graph::in_memory().unwrap();
        let s = Sound {
            id: super::super::assets::picture_id(&b),
            mime: "audio/wav".into(),
            name: "hum".into(),
            added: 1,
            size: b.len() as u64,
            seconds: Some(0.5),
        };
        assert!(g.put_sound(&s, &b).unwrap());
        assert!(!g.put_sound(&s, &b).unwrap());
        assert_eq!(g.sounds().unwrap(), std::slice::from_ref(&s));
        assert_eq!(g.sound_bytes(&s.id).unwrap().unwrap(), b);
        g.name_sound(&s.id, "drone").unwrap();
        assert_eq!(g.sound(&s.id).unwrap().unwrap().name, "drone");
        assert!(g.delete_sound(&s.id).unwrap());
        assert!(g.sounds().unwrap().is_empty());
        let e = Emitter {
            id: [7; 16],
            volume: 0.5,
            range: 12.0,
            looped: true,
        };
        let mut out = Vec::new();
        e.write(&mut out);
        assert_eq!(Emitter::read(&out), Some((e, 27)));
        let j = serde_json::to_value(e).unwrap();
        assert_eq!(j["loop"], true);
        assert_eq!(serde_json::from_value::<Emitter>(j).unwrap(), e);
    }
}
