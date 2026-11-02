//! The native room's sounds on the Linux desktop (no Web Audio): sounds decoded once with
//! symphonia (wav, ogg, flac, mp3, mp4), mixed on cpal's output thread, each voice at the
//! gains the room says its two ears hear it with (`anend_render::room::Audio`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use anend_render::room::Audio;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// a sound decoded: stereo frames at the output's rate
type Pcm = Arc<Vec<[f32; 2]>>;

struct Voice {
    pcm: Pcm,
    at: usize,
    looped: bool,
    /// the gains it's heard with, and where they're easing to (no clicks)
    gains: [f32; 2],
    target: [f32; 2],
}

#[derive(Default)]
struct Mix {
    rate: u32,
    sounds: HashMap<String, Pcm>,
    voices: HashMap<String, Voice>,
    /// each voice's gains as last said (they can come before it starts)
    gains: HashMap<String, [f32; 2]>,
    /// voices waiting on a sound still decoding: voice → (sound, looped)
    pending: HashMap<String, (String, bool)>,
}

impl Mix {
    fn start(&mut self, voice: String, sound: &str, looped: bool) {
        let Some(pcm) = self.sounds.get(sound).cloned() else {
            self.pending.insert(voice, (sound.to_string(), looped));
            return;
        };
        let g = self
            .gains
            .get(&voice)
            .copied()
            .unwrap_or(if voice.is_empty() { [1.0, 1.0] } else { [0.0, 0.0] });
        self.voices.insert(
            voice,
            Voice {
                pcm,
                at: 0,
                looped,
                gains: g,
                target: g,
            },
        );
    }
}

fn mix() -> &'static Arc<Mutex<Mix>> {
    static M: OnceLock<Arc<Mutex<Mix>>> = OnceLock::new();
    M.get_or_init(|| {
        let m = Arc::new(Mutex::new(Mix {
            rate: 48_000,
            ..Default::default()
        }));
        start(m.clone());
        m
    })
}

/// the output stream, on a thread of its own (cpal's stream isn't Send); no device, no sound
fn start(m: Arc<Mutex<Mix>>) {
    std::thread::spawn(move || {
        let Some(dev) = cpal::default_host().default_output_device() else {
            return;
        };
        let Ok(cfg) = dev.default_output_config() else { return };
        let channels = cfg.channels() as usize;
        if let Ok(mut x) = m.lock() {
            x.rate = cfg.sample_rate().0;
        }
        let fill = move |out: &mut [f32]| {
            out.fill(0.0);
            let Ok(mut x) = m.lock() else { return };
            x.voices.retain(|_, v| {
                for frame in out.chunks_mut(channels.max(1)) {
                    if v.at >= v.pcm.len() {
                        if v.looped && !v.pcm.is_empty() {
                            v.at = 0;
                        } else {
                            return false;
                        }
                    }
                    for i in 0..2 {
                        v.gains[i] += (v.target[i] - v.gains[i]) * 0.002;
                    }
                    let s = v.pcm[v.at];
                    v.at += 1;
                    if channels == 1 {
                        frame[0] += (s[0] * v.gains[0] + s[1] * v.gains[1]) * 0.5;
                    } else {
                        frame[0] += s[0] * v.gains[0];
                        frame[1] += s[1] * v.gains[1];
                    }
                }
                true
            });
            for s in out.iter_mut() {
                *s = s.clamp(-1.0, 1.0);
            }
        };
        let err = |_e: cpal::StreamError| {};
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => {
                dev.build_output_stream(&cfg.config(), move |d: &mut [f32], _: &_| fill(d), err, None)
            }
            cpal::SampleFormat::I16 => {
                let mut buf = Vec::new();
                dev.build_output_stream(
                    &cfg.config(),
                    move |d: &mut [i16], _: &_| {
                        buf.resize(d.len(), 0.0);
                        fill(&mut buf);
                        for (o, s) in d.iter_mut().zip(&buf) {
                            *o = (s * i16::MAX as f32) as i16;
                        }
                    },
                    err,
                    None,
                )
            }
            _ => return,
        };
        let Ok(stream) = stream else { return };
        if stream.play().is_err() {
            return;
        }
        // the stream plays while it's kept: park here for the app's life
        loop {
            std::thread::park();
        }
    });
}

/// a sound's bytes decoded to stereo at a rate (resampled straight across), or none
fn decode(bytes: Vec<u8>, rate: u32) -> Option<Vec<[f32; 2]>> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let mut format = probed.format;
    let track = format.default_track()?.clone();
    let mut dec = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;
    let src_rate = track.codec_params.sample_rate.unwrap_or(rate);
    let mut frames: Vec<[f32; 2]> = Vec::new();
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track.id {
            continue;
        }
        let Ok(buf) = dec.decode(&packet) else { continue };
        let spec = *buf.spec();
        let ch = spec.channels.count().max(1);
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        for f in sb.samples().chunks(ch) {
            frames.push(if ch == 1 { [f[0], f[0]] } else { [f[0], f[1]] });
        }
    }
    if frames.is_empty() {
        return None;
    }
    if src_rate == rate {
        return Some(frames);
    }
    let step = src_rate as f64 / rate as f64;
    let n = (frames.len() as f64 / step) as usize;
    Some(
        (0..n)
            .map(|i| {
                let t = i as f64 * step;
                let (a, f) = (t as usize, (t.fract()) as f32);
                let (x, y) = (frames[a.min(frames.len() - 1)], frames[(a + 1).min(frames.len() - 1)]);
                [x[0] + (y[0] - x[0]) * f, x[1] + (y[1] - x[1]) * f]
            })
            .collect(),
    )
}

/// does what the room said
pub fn play(cmds: Vec<Audio>) {
    if cmds.is_empty() {
        return;
    }
    let m = mix();
    for c in cmds {
        match c {
            Audio::Data { id, bytes } => {
                let rate = m.lock().map(|x| x.rate).unwrap_or(48_000);
                // decoding takes a moment: off the room's thread, then what waited starts
                let m = m.clone();
                std::thread::spawn(move || {
                    let pcm = decode(bytes, rate).unwrap_or_default();
                    if let Ok(mut x) = m.lock() {
                        x.sounds.insert(id.clone(), Arc::new(pcm));
                        let waiting: Vec<(String, bool)> = x
                            .pending
                            .iter()
                            .filter(|(_, (s, _))| *s == id)
                            .map(|(v, (_, l))| (v.clone(), *l))
                            .collect();
                        for (v, l) in waiting {
                            x.pending.remove(&v);
                            x.start(v, &id, l);
                        }
                    }
                });
            }
            Audio::Play { voice, sound, looped } => {
                if let Ok(mut x) = m.lock() {
                    x.start(voice, &sound, looped);
                }
            }
            Audio::Stop { voice } => {
                if let Ok(mut x) = m.lock() {
                    x.voices.remove(&voice);
                    x.pending.remove(&voice);
                }
            }
            Audio::Gains { voice, left, right } => {
                if let Ok(mut x) = m.lock() {
                    if let Some(v) = x.voices.get_mut(&voice) {
                        v.target = [left, right];
                    }
                    x.gains.insert(voice, [left, right]);
                }
            }
        }
    }
}
