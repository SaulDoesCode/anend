//! Auto's choice by measured speed. A GPU that gives the CPU's bits is only worth using
//! where it is also faster, and that depends on the machine and the model's size: small
//! models spend most of a GPU's time on overhead (submissions, waiting, reading back),
//! which a fast CPU doesn't have. So on first use of a model size, auto reads a fixed set
//! of texts on both, keeps the two rates, and reads where the rate is higher. The rates
//! are kept per machine (the adapter and driver, the CPU's kernels and threads, and this
//! build's version, since a new build may be faster) and per model size, in memory and in
//! a small JSON file in the user's cache folder, so the measuring happens once.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::model::Net;

/// the two rates measured for one machine and model size
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Speed {
    /// the machine (see `machine`)
    pub machine: String,
    /// the model's size, in words (see `size_of`)
    pub size: String,
    pub gpu_texts_per_sec: f64,
    pub cpu_texts_per_sec: f64,
    /// texts each device read for the rate
    pub texts: usize,
}

impl Speed {
    pub fn gpu_faster(&self) -> bool {
        self.gpu_texts_per_sec > self.cpu_texts_per_sec
    }

    /// the two rates and the choice, in words
    pub fn words(&self) -> String {
        format!(
            "with a model of {}, the GPU reads {} texts a second and the CPU {}, so {} reads",
            self.size,
            rate(self.gpu_texts_per_sec),
            rate(self.cpu_texts_per_sec),
            if self.gpu_faster() { "the GPU" } else { "the CPU" }
        )
    }
}

fn rate(r: f64) -> String {
    if r >= 100.0 {
        format!("{r:.0}")
    } else if r >= 10.0 {
        format!("{r:.1}")
    } else {
        format!("{r:.2}")
    }
}

/// a model's size in words: its width, blocks, window and weights (what its speed on
/// each device depends on; the values of the weights don't matter)
pub fn size_of(net: &Net) -> String {
    let weights: usize = net.params().iter().map(|(_, p)| p.len()).sum();
    let w = if weights >= 1_000_000 {
        format!("{:.1}M", weights as f64 / 1e6)
    } else {
        format!("{}k", weights.div_ceil(1000))
    };
    format!(
        "width {}, {} blocks, windows of {}, {w} weights{}",
        net.cfg.d_model,
        net.blocks.len(),
        net.cfg.max_len,
        if net.cfg.extra > 0 {
            format!(", {} inputs from others", net.cfg.extra)
        } else {
            String::new()
        }
    )
}

#[derive(Default)]
struct Kept {
    loaded: bool,
    all: BTreeMap<(String, String), Speed>,
}

static KEPT: Mutex<Kept> = Mutex::new(Kept {
    loaded: false,
    all: BTreeMap::new(),
});

/// where the rates are kept on disk: `ANEND_SPEED_FILE`, else the user's cache folder;
/// none in tests and in the browser (memory only)
static FILE: Mutex<Option<Option<std::path::PathBuf>>> = Mutex::new(None);

/// sets where the rates are kept (None: memory only); for tests and embedders
pub fn set_file(path: Option<std::path::PathBuf>) {
    *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    let mut k = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    k.loaded = false;
    k.all.clear();
}

fn file() -> Option<std::path::PathBuf> {
    if let Some(f) = FILE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return f;
    }
    if cfg!(test) || cfg!(target_arch = "wasm32") {
        return None;
    }
    if let Some(f) = std::env::var_os("ANEND_SPEED_FILE") {
        return Some(f.into());
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| home.map(|h| h.join(".cache")))
    };
    base.map(|b| b.join("anend").join("gpu-speed.json"))
}

fn with_kept<T>(f: impl FnOnce(&mut BTreeMap<(String, String), Speed>) -> T) -> T {
    let mut k = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    if !k.loaded {
        k.loaded = true;
        if let Some(p) = file()
            && let Ok(b) = std::fs::read(&p)
            && let Ok(v) = serde_json::from_slice::<Vec<Speed>>(&b)
        {
            for s in v {
                k.all.insert((s.machine.clone(), s.size.clone()), s);
            }
        }
    }
    f(&mut k.all)
}

fn store(all: &BTreeMap<(String, String), Speed>) {
    let Some(p) = file() else { return };
    let mut v: Vec<&Speed> = all.values().collect();
    v.sort_by(|a, b| (&a.machine, &a.size).cmp(&(&b.machine, &b.size)));
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(t) = serde_json::to_string_pretty(&v) {
        let _ = std::fs::write(&p, t);
    }
}

/// the rates kept for `machine`
pub fn known(machine: &str) -> Vec<Speed> {
    with_kept(|all| {
        let mut v: Vec<Speed> = all.values().filter(|s| s.machine == machine).cloned().collect();
        v.sort_by(|a, b| a.size.cmp(&b.size));
        v
    })
}

/// forgets the rates kept for `machine`, so each size is measured again on next use
pub fn forget(machine: &str) {
    with_kept(|all| {
        all.retain(|(m, _), _| m != machine);
        store(all);
    })
}

/// the kept rates for this machine and size, or `measure`'s, kept for next time
pub fn get_or_measure(
    machine: &str,
    size: &str,
    measure: impl FnOnce() -> Result<(f64, f64, usize), String>,
) -> Result<Speed, String> {
    let key = (machine.to_string(), size.to_string());
    if let Some(s) = with_kept(|all| all.get(&key).cloned()) {
        return Ok(s);
    }
    let (gpu, cpu, texts) = measure()?;
    let s = Speed {
        machine: machine.to_string(),
        size: size.to_string(),
        gpu_texts_per_sec: gpu,
        cpu_texts_per_sec: cpu,
        texts,
    };
    with_kept(|all| {
        all.insert(key, s.clone());
        store(all);
    });
    Ok(s)
}

/// the texts the measuring reads: deterministic, 20 to about 400 bytes
pub fn texts(n: usize) -> Vec<String> {
    const W: [&str; 12] = [
        "river", "stone", "copper", "gear", "heron", "valve", "moss", "lamp", "the", "of", "and", "light",
    ];
    (0..n)
        .map(|i| {
            let mut h = (i as u64 + 77).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
            let len = 20 + (h % 380) as usize;
            let mut s = String::new();
            while s.len() < len {
                h ^= h << 13;
                h ^= h >> 7;
                h ^= h << 17;
                s.push_str(W[(h % 12) as usize]);
                s.push(if h.is_multiple_of(9) { '.' } else { ' ' });
            }
            s
        })
        .collect()
}

#[cfg(feature = "gpu")]
pub use on_gpu::*;

#[cfg(feature = "gpu")]
mod on_gpu {
    use std::time::Instant;

    use super::super::device::{Device, Runner};
    use super::super::{Classifier, Pool, pool};
    use super::*;

    /// the GPU reading path's revision: raised when a change makes it faster or slower,
    /// so rates kept from before are measured again
    pub const READING_REVISION: u32 = 2;

    /// this machine, as the rates are kept by: the adapter and driver, the CPU's kernels
    /// and threads, this build's version and the reading path's revision
    pub fn machine() -> Result<String, String> {
        let g = super::super::gpu::shared()?;
        let i = &g.info;
        Ok(format!(
            "{} ({:?}, {} {}); CPU {} × {}; anend {} reading r{READING_REVISION}{}",
            i.name,
            i.backend,
            i.driver,
            i.driver_info,
            super::super::kernels::simd_name(),
            pool::available(),
            env!("CARGO_PKG_VERSION"),
            if cfg!(debug_assertions) { " debug" } else { "" }
        ))
    }

    /// texts the CPU reads for its rate; the GPU reads `FIRST` first, and the rest only
    /// when that rate is within a factor of 2 of the CPU's (a clearly slower GPU stops
    /// early)
    pub const TEXTS: usize = 192;
    pub const FIRST: usize = 32;

    /// reads the fixed texts with `clf` on the CPU and on `device`: (GPU rate, CPU rate,
    /// texts), each a rate in texts a second, after a warm-up on each
    pub fn measure(clf: &Classifier, device: Device) -> Result<(f64, f64, usize), String> {
        let all = texts(TEXTS);
        let refs: Vec<&str> = all.iter().map(String::as_str).collect();
        let p = Pool::new(pool::available());
        let extras: Vec<Vec<f32>> = if clf.net.cfg.extra > 0 {
            refs.iter().map(|_| vec![0.0; clf.net.cfg.extra]).collect()
        } else {
            vec![]
        };
        let warm_x: Vec<Vec<f32>> = extras.iter().take(4).cloned().collect();
        clf.predict_many(&refs[..4], &warm_x, &p);
        let t = Instant::now();
        clf.predict_many(&refs, &extras, &p);
        let cpu = TEXTS as f64 / t.elapsed().as_secs_f64().max(1e-9);

        let r = Runner::open(device, &clf.net)?.ok_or("no GPU")?;
        let zero = vec![0f32; clf.net.cfg.extra];
        let gains = clf.gains();
        let read = |ts: &[&str]| {
            let bytes: Vec<&[u8]> = ts.iter().map(|t| t.as_bytes()).collect();
            let ex: Vec<&[f32]> = ts.iter().map(|_| zero.as_slice()).collect();
            let t = Instant::now();
            r.text_runs(&clf.net, &bytes, &ex, &gains);
            t.elapsed().as_secs_f64().max(1e-9)
        };
        // the pipelines compile on the first call
        read(&refs[..4]);
        let first = FIRST as f64 / read(&refs[..FIRST]);
        if first * 2.0 < cpu {
            return Ok((first, cpu, FIRST));
        }
        let gpu = TEXTS as f64 / read(&refs);
        Ok((gpu, cpu, TEXTS))
    }

    /// the rates for `clf`'s size on this machine, measured on first use
    pub fn of(clf: &Classifier, device: Device) -> Result<Speed, String> {
        get_or_measure(&machine()?, &size_of(&clf.net), || measure(clf, device))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_are_kept_and_say_why() {
        set_file(None);
        let mut calls = 0;
        let a = get_or_measure("test machine", "tiny", || {
            calls += 1;
            Ok((50.0, 400.0, 32))
        })
        .unwrap();
        assert!(!a.gpu_faster());
        assert!(
            a.words().contains("the GPU reads 50.0 texts a second and the CPU 400"),
            "{}",
            a.words()
        );
        assert!(a.words().ends_with("so the CPU reads"));
        let b = get_or_measure("test machine", "tiny", || {
            calls += 1;
            Ok((1.0, 1.0, 1))
        })
        .unwrap();
        assert_eq!(a, b, "measured once");
        assert_eq!(calls, 1);
        assert_eq!(known("test machine").len(), 1);
        forget("test machine");
        assert!(known("test machine").is_empty());

        // kept in the file (one test: the kept rates are the process's)
        let dir = std::env::temp_dir().join(format!("anend-speed-{}", std::process::id()));
        let f = dir.join("gpu-speed.json");
        set_file(Some(f.clone()));
        get_or_measure("file machine", "big", || Ok((900.0, 300.0, 192))).unwrap();
        // as if a new process: the kept map is emptied and read back from the file
        set_file(Some(f.clone()));
        let k = known("file machine");
        assert_eq!(k.len(), 1);
        assert!(k[0].gpu_faster());
        set_file(None);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(all(test, feature = "gpu"))]
mod gpu_tests {
    use super::*;
    use crate::classify::device::Device;
    use crate::classify::{Classifier, Config, HeadSpec};

    #[test]
    fn measuring_gives_both_rates() {
        let Some(_) = crate::classify::gpu::tests::device() else {
            return;
        };
        let clf = Classifier::new(Config::tiny(), &[HeadSpec::single("s", &["a", "b"])], 5).unwrap();
        let (gpu, cpu, n) = measure(&clf, Device::Gpu).unwrap();
        assert!(gpu > 0.0 && cpu > 0.0);
        assert!(n == FIRST || n == TEXTS);
        assert!(machine().unwrap().contains("anend"));
        assert!(size_of(&clf.net).contains("blocks"));
    }
}
