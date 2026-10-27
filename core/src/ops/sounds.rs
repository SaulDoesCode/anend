//! The app's sound operations (`sounds.*`, listed in docs/app-api.md; queued request #17):
//! sound files kept in the save (`graph::sounds`), each known by the hash of its bytes,
//! with a name, its kind and (for a WAV) its length. Blocks play them as emitters
//! (`blocks.set` with `sound`), in the room while it plays.

use serde_json::{Value, json};

use super::classify::Limits;
use super::{Res, s};
use crate::classifiers::{base64, unbase64};
use crate::graph::Graph;
use crate::graph::assets::picture_id;
use crate::graph::sounds::{Sound, sniff};

/// a sound's bytes at most, on a shared hub and on one's own machine
pub const MAX_BYTES_SHARED: usize = 8 << 20;
pub const MAX_BYTES: usize = 256 << 20;
/// sounds a save holds at most on a shared hub (one's own machine: no limit)
pub const MAX_SOUNDS_SHARED: usize = 500;

pub fn routed(op: &str) -> bool {
    op.starts_with("sounds.")
}

/// a sound file's bytes into the save; its details and whether it's new
pub fn add(g: &Graph, bytes: &[u8], name: &str, limits: &Limits) -> Res {
    let max = if limits.shared { MAX_BYTES_SHARED } else { MAX_BYTES };
    if bytes.len() > max {
        return Err(format!("a sound is at most {} MB here", max >> 20));
    }
    let (mime, seconds) = sniff(bytes).ok_or("that isn't a sound file (WAV, OGG, MP3, FLAC, WebM or M4A)")?;
    let id = picture_id(bytes);
    if let Some(s) = g.sound(&id)? {
        return Ok(json!({ "sound": s, "new": false }));
    }
    if limits.shared && g.sounds()?.len() >= MAX_SOUNDS_SHARED {
        return Err(format!(
            "a save holds at most {MAX_SOUNDS_SHARED} sounds on a shared hub"
        ));
    }
    let s = Sound {
        id,
        mime: mime.into(),
        name: name.trim().to_string(),
        added: crate::graph::store::system_ms(),
        size: bytes.len() as u64,
        seconds,
    };
    let new = g.put_sound(&s, bytes)?;
    Ok(json!({ "sound": s, "new": new }))
}

pub fn call(g: &Graph, op: &str, args: &Value, limits: &Limits) -> Res {
    match op {
        "sounds.list" => Ok(json!(g.sounds()?)),
        // `{data (base64), name}`, or on one's own machine `{path}`
        "sounds.add" => {
            let (bytes, name) = match args.get("data").and_then(Value::as_str) {
                Some(d) => (
                    unbase64(d).ok_or("the sound's data isn't base64")?,
                    s(args, "name").to_string(),
                ),
                None => {
                    let path = s(args, "path").trim();
                    if path.is_empty() {
                        return Err("which sound? (data or path)".into());
                    }
                    if limits.shared {
                        return Err("a shared hub takes a sound's bytes, not a path".into());
                    }
                    let b = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    (b, name)
                }
            };
            add(g, &bytes, &name, limits)
        }
        // `{id}`: `{sound, data}` (base64), for the room to play
        "sounds.get" => {
            let id = s(args, "id");
            let snd = g.sound(id)?.ok_or_else(|| format!("no sound {id:?}"))?;
            let b = g.sound_bytes(id)?.ok_or_else(|| format!("no sound {id:?}"))?;
            Ok(json!({ "sound": snd, "data": base64(&b) }))
        }
        "sounds.name" => {
            g.name_sound(s(args, "id"), s(args, "name").trim())?;
            Ok(json!(g.sound(s(args, "id"))?))
        }
        "sounds.delete" => Ok(json!({ "removed": g.delete_sound(s(args, "id"))? })),
        _ => Err(format!("unknown op {op:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_through_the_api() {
        let g = Graph::in_memory().unwrap();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF\x24\x00\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00\x40\x1f\x00\x00\x80\x3e\x00\x00\x02\x00\x10\x00data\x00\x00\x00\x00");
        let c = |op: &str, a: Value| call(&g, op, &a, &Limits::NONE);
        assert!(c("sounds.add", json!({ "data": base64(b"not a sound"), "name": "x" })).is_err());
        let r = c("sounds.add", json!({ "data": base64(&wav), "name": "hush.wav" })).unwrap();
        assert_eq!(r["new"], true);
        assert_eq!(r["sound"]["mime"], "audio/wav");
        let id = r["sound"]["id"].as_str().unwrap().to_string();
        assert_eq!(
            c("sounds.add", json!({ "data": base64(&wav), "name": "again" })).unwrap()["new"],
            false
        );
        assert_eq!(c("sounds.list", json!({})).unwrap().as_array().unwrap().len(), 1);
        assert_eq!(
            unbase64(c("sounds.get", json!({ "id": id })).unwrap()["data"].as_str().unwrap()).unwrap(),
            wav
        );
        assert_eq!(
            c("sounds.name", json!({ "id": id, "name": "quiet" })).unwrap()["name"],
            "quiet"
        );
        // a block plays it
        let b = super::super::blocks::call(
            &g,
            "blocks.add",
            &json!({ "kind": "plain", "texture": "#808080", "sound": { "id": id, "volume": 9, "loop": true } }),
            &Limits::NONE,
        )
        .unwrap();
        assert_eq!(b["blocks"][0]["sound"]["volume"], 4.0);
        assert_eq!(b["blocks"][0]["sound"]["loop"], true);
        // a program starts it
        let rm = Default::default();
        super::super::call(
            &g,
            &rm,
            "notes.save",
            &json!({ "note": { "tags": ["s"] }, "text": "# beep\n\n{sound} >" }),
        )
        .unwrap();
        let bid = b["ids"][0].as_str().unwrap();
        super::super::wiring::call(
            &g,
            "wiring.set",
            &json!({ "id": bid, "wiring": { "on": ["click"], "program": "beep" } }),
            false,
        )
        .unwrap();
        let f = super::super::wiring::call(&g, "wiring.fire", &json!({ "id": bid, "event": "click" }), false).unwrap();
        assert_eq!(f["sounds"], json!([{ "block": bid, "play": true }]), "{f}");
        assert_eq!(c("sounds.delete", json!({ "id": id })).unwrap()["removed"], true);
    }
}
