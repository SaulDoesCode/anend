//! Sounds in the room, as space.js placed them (core's graph::sounds): a block can be an
//! emitter (a sound of the save, a volume, how far it carries, whether it loops). While
//! the room plays, looping emitters sound from the start and the others when a program
//! prints `sound`; each is heard from the camera, panned by where its block is and
//! fading to nothing at its range. The room only says what to play and how loud each
//! ear hears it (`take_audio`); the desktop's mixer plays it. A sound's bytes are asked
//! of Rust the first time they're needed (`soundwant`).

use std::collections::{HashMap, HashSet};

use base64::Engine;
use glam::Vec3;
use serde_json::{Value, json};

use super::Room;

/// what the mixer is told
#[derive(Clone, Debug, PartialEq)]
pub enum Audio {
    /// a sound's bytes (wav, ogg, flac, mp3, mp4), to decode once
    Data {
        id: String,
        bytes: Vec<u8>,
    },
    /// a voice starts a sound from its beginning
    Play {
        voice: String,
        sound: String,
        looped: bool,
    },
    Stop {
        voice: String,
    },
    /// how loud each ear hears a voice now
    Gains {
        voice: String,
        left: f32,
        right: f32,
    },
}

/// a block sounding
#[derive(Clone, Debug)]
struct Voice {
    volume: f32,
    range: f32,
    gains: (f32, f32),
}

#[derive(Default)]
pub struct Sounds {
    have: HashSet<String>,
    asked: HashSet<String>,
    /// sound id → the voices to start when it arrives (block id, or "" for the preview)
    waiting: HashMap<String, Vec<String>>,
    voices: HashMap<String, Voice>,
    out: Vec<Audio>,
}

impl Room {
    pub fn take_audio(&mut self) -> Vec<Audio> {
        std::mem::take(&mut self.sounds.out)
    }

    /// a sound's bytes from Rust: kept, then what waited on it starts
    pub(super) fn sound_data(&mut self, id: &str, data: &str) {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data.trim()) else {
            self.toast("that sound doesn't play here: its bytes didn't read");
            return;
        };
        self.sounds.out.push(Audio::Data {
            id: id.to_string(),
            bytes,
        });
        self.sounds.have.insert(id.to_string());
        for v in self.sounds.waiting.remove(id).unwrap_or_default() {
            if v.is_empty() {
                self.sounds.out.push(Audio::Play {
                    voice: String::new(),
                    sound: id.to_string(),
                    looped: false,
                });
            } else {
                self.play_sound(&v);
            }
        }
    }

    /// runs a voice once the sound is here (asking Rust for it the first time)
    fn want_sound(&mut self, sound: &str, voice: &str) -> bool {
        if self.sounds.have.contains(sound) {
            return true;
        }
        self.sounds
            .waiting
            .entry(sound.to_string())
            .or_default()
            .push(voice.to_string());
        if self.sounds.asked.insert(sound.to_string()) {
            self.send_ev("soundwant", None, None, json!(sound));
        }
        false
    }

    /// a block's sound starts (again, from the beginning)
    pub(super) fn play_sound(&mut self, block: &str) {
        let Some(s) = self
            .blocks
            .get(block)
            .map(|b| b["sound"].clone())
            .filter(|s| s.is_object())
        else {
            return;
        };
        self.stop_sound(block);
        let Some(id) = s["id"].as_str().map(String::from) else {
            return;
        };
        if !self.want_sound(&id, block) {
            return;
        }
        let volume = s["volume"].as_f64().map(|v| v as f32).unwrap_or(1.0);
        let range = s["range"].as_f64().map(|v| v as f32).unwrap_or(20.0).max(1.01);
        self.sounds.voices.insert(
            block.to_string(),
            Voice {
                volume,
                range,
                gains: (-1.0, -1.0),
            },
        );
        self.place_voices();
        self.sounds.out.push(Audio::Play {
            voice: block.to_string(),
            sound: id,
            looped: s["loop"] == true,
        });
    }

    pub(super) fn stop_sound(&mut self, block: &str) {
        if self.sounds.voices.remove(block).is_some() {
            self.sounds.out.push(Audio::Stop {
                voice: block.to_string(),
            });
        }
    }

    pub(super) fn start_loops(&mut self) {
        let ids: Vec<String> = self
            .blocks
            .list
            .iter()
            .filter(|(_, b)| b["sound"]["loop"] == true)
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            self.play_sound(&id);
        }
    }

    pub(super) fn hush_all(&mut self) {
        for id in self.sounds.voices.keys().cloned().collect::<Vec<_>>() {
            self.stop_sound(&id);
        }
    }

    /// a sound heard as it is, not placed (the sounds window's ▶)
    pub(super) fn preview_sound(&mut self, id: &str) {
        self.sounds.out.push(Audio::Stop { voice: String::new() });
        if self.want_sound(id, "") {
            self.sounds.out.push(Audio::Play {
                voice: String::new(),
                sound: id.to_string(),
                looped: false,
            });
        }
    }

    /// the listener is the camera; each voice is where its block is: an equal-power pan
    /// by how far to the side it is, and a linear fade from one unit to its range
    pub(super) fn place_voices(&mut self) {
        if self.sounds.voices.is_empty() {
            return;
        }
        let right = self.camera_rot() * Vec3::X;
        let at = self.at;
        let mut changed = Vec::new();
        for (id, v) in &mut self.sounds.voices {
            let Some(m) = self.blocks.model(id) else { continue };
            let to = m.w_axis.truncate() - at;
            let d = to.length();
            let fade = (1.0 - (d.clamp(1.0, v.range) - 1.0) / (v.range - 1.0)).clamp(0.0, 1.0);
            let side = if d > 1e-4 { to.dot(right) / d } else { 0.0 };
            let x = (side + 1.0) / 2.0 * std::f32::consts::FRAC_PI_2;
            let g = (x.cos() * v.volume * fade, x.sin() * v.volume * fade);
            if (g.0 - v.gains.0).abs() > 0.004 || (g.1 - v.gains.1).abs() > 0.004 {
                v.gains = g;
                changed.push(Audio::Gains {
                    voice: id.clone(),
                    left: g.0,
                    right: g.1,
                });
            }
        }
        self.sounds.out.extend(changed);
    }

    /// the sounds' commands (space.js's protocol); false for one that isn't
    pub(super) fn sound_command(&mut self, cmd: &str, c: &Value) -> bool {
        match cmd {
            "sounddata" => self.sound_data(c["id"].as_str().unwrap_or(""), c["data"].as_str().unwrap_or("")),
            "sounds" => {
                for s in c["sounds"].as_array().into_iter().flatten() {
                    let b = s["block"].as_str().unwrap_or("").to_string();
                    if s["play"] == true {
                        self.play_sound(&b);
                    } else {
                        self.stop_sound(&b);
                    }
                }
            }
            "soundpreview" => self.preview_sound(c["id"].as_str().unwrap_or("")),
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_asked_for_then_placed() {
        let mut r = Room::new();
        r.command(&json!({"cmd": "blocks", "blocks": [
            {"id": "b", "kind": "plain", "texture": "#fff", "x": 5, "y": 0, "z": 0, "w": 1, "h": 1, "d": 1,
             "sound": {"id": "s1", "volume": 1.0, "range": 20, "loop": true}}
        ]}));
        r.take_events();
        r.command(&json!({"cmd": "sounds", "sounds": [{"block": "b", "play": true}]}));
        // not here yet: asked for once
        let evs = r.take_events();
        assert!(evs.iter().any(|e| e["kind"] == "soundwant" && e["data"] == "s1"));
        assert!(r.take_audio().is_empty());
        r.command(&json!({"cmd": "sounddata", "id": "s1", "data": "UklGRg=="}));
        let a = r.take_audio();
        assert!(matches!(&a[0], Audio::Data { id, .. } if id == "s1"));
        assert!(
            a.iter()
                .any(|x| matches!(x, Audio::Play { voice, looped: true, .. } if voice == "b"))
        );
        // to the right of the camera: the right ear hears more
        let g = a
            .iter()
            .find_map(|x| match x {
                Audio::Gains { left, right, .. } => Some((*left, *right)),
                _ => None,
            })
            .unwrap();
        assert!(g.1 > g.0, "{g:?}");
        r.command(&json!({"cmd": "sounds", "sounds": [{"block": "b", "play": false}]}));
        assert!(
            r.take_audio()
                .iter()
                .any(|x| matches!(x, Audio::Stop { voice } if voice == "b"))
        );
    }
}
