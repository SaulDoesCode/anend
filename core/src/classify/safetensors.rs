//! The safetensors weight format (what open models ship in), read and written with std
//! and `serde_json` only.
//!
//! A file is an 8-byte little-endian header length `N`, `N` bytes of JSON mapping each
//! tensor name to `{"dtype", "shape", "data_offsets": [begin, end]}` (offsets into the
//! data that follows the header; an optional `"__metadata__"` map of strings), then the
//! raw little-endian data. Big checkpoints are split into shards listed by
//! `model.safetensors.index.json` (`{"weight_map": {name: file}}`).
//!
//! Tensors are read on demand (seek + read), so a checkpoint never has to fit in memory
//! twice; `F32`, `F16`, `BF16` and `F64` convert to f32 (F16 and BF16 exactly, F64
//! rounded). Integer and quantised dtypes are refused by name.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value as Json;

/// the element types read (and written)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dtype {
    F32,
    F16,
    BF16,
    F64,
}

impl Dtype {
    pub fn parse(s: &str) -> Result<Dtype, String> {
        Ok(match s {
            "F32" => Dtype::F32,
            "F16" => Dtype::F16,
            "BF16" => Dtype::BF16,
            "F64" => Dtype::F64,
            other => {
                return Err(format!(
                    "dtype {other} is not supported (F32, F16, BF16 and F64 are; quantised checkpoints need converting first)"
                ));
            }
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Dtype::F32 => "F32",
            Dtype::F16 => "F16",
            Dtype::BF16 => "BF16",
            Dtype::F64 => "F64",
        }
    }

    pub fn size(self) -> usize {
        match self {
            Dtype::F32 => 4,
            Dtype::F16 | Dtype::BF16 => 2,
            Dtype::F64 => 8,
        }
    }
}

/// IEEE half to f32, exactly (subnormals, infinities and NaN included)
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = match (exp, man) {
        (0, 0) => sign,
        (0, m) => {
            // subnormal: normalise the mantissa
            let mut e = 127 - 15 + 1;
            let mut m = m;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3ff) << 13)
        }
        (0x1f, 0) => sign | 0x7f80_0000,
        (0x1f, m) => sign | 0x7fc0_0000 | (m << 13),
        (e, m) => sign | ((e + 127 - 15) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

/// f32 to IEEE half, rounding to nearest even (overflow to infinity)
pub fn f32_to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        // subnormal half: shift the full mantissa (with its hidden bit) into place
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rest = m & ((1 << shift) - 1);
        let mut v = m >> shift;
        if rest > half || (rest == half && v & 1 == 1) {
            v += 1;
        }
        return sign | v as u16;
    }
    let mut v = ((e as u32) << 10) | (man >> 13);
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && v & 1 == 1) {
        v += 1; // may carry into the exponent, which is the right answer (up to infinity)
    }
    sign | v as u16
}

/// bfloat16 to f32, exactly
pub fn bf16_to_f32(h: u16) -> f32 {
    f32::from_bits((h as u32) << 16)
}

/// f32 to bfloat16, rounding to nearest even
pub fn f32_to_bf16(x: f32) -> u16 {
    let b = x.to_bits();
    if x.is_nan() {
        return ((b >> 16) | 0x40) as u16;
    }
    let rest = b & 0xffff;
    let mut v = b >> 16;
    if rest > 0x8000 || (rest == 0x8000 && v & 1 == 1) {
        v += 1;
    }
    v as u16
}

/// little-endian raw data of `dtype` to f32
pub fn to_f32(dtype: Dtype, raw: &[u8]) -> Vec<f32> {
    match dtype {
        Dtype::F32 => raw.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect(),
        Dtype::F16 => raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| f16_to_f32(u16::from_le_bytes(*c)))
            .collect(),
        Dtype::BF16 => raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| bf16_to_f32(u16::from_le_bytes(*c)))
            .collect(),
        Dtype::F64 => raw
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes(*c) as f32)
            .collect(),
    }
}

fn from_f32(dtype: Dtype, x: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(x.len() * dtype.size());
    for &v in x {
        match dtype {
            Dtype::F32 => out.extend_from_slice(&v.to_le_bytes()),
            Dtype::F16 => out.extend_from_slice(&f32_to_f16(v).to_le_bytes()),
            Dtype::BF16 => out.extend_from_slice(&f32_to_bf16(v).to_le_bytes()),
            Dtype::F64 => out.extend_from_slice(&(v as f64).to_le_bytes()),
        }
    }
    out
}

/// one tensor's entry in a header
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub dtype: Dtype,
    pub shape: Vec<usize>,
    /// byte range in the file (header included)
    pub start: u64,
    pub end: u64,
}

impl Info {
    pub fn len(&self) -> usize {
        self.shape.iter().product()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// one safetensors file: its header read, tensors read on demand
#[derive(Debug)]
pub struct SafeTensors {
    pub path: PathBuf,
    pub tensors: BTreeMap<String, Info>,
    pub metadata: BTreeMap<String, String>,
    file: Mutex<File>,
}

/// tensors by name, and the metadata strings
type Header = (BTreeMap<String, Info>, BTreeMap<String, String>);

/// parses a header (the JSON after the length) for a file of `file_len` bytes
fn parse_header(json: &[u8], header_len: u64, file_len: u64) -> Result<Header, String> {
    let v: Json = serde_json::from_slice(json).map_err(|e| format!("safetensors header: {e}"))?;
    let obj = v.as_object().ok_or("safetensors header is not an object")?;
    let base = 8 + header_len;
    let mut tensors = BTreeMap::new();
    let mut metadata = BTreeMap::new();
    for (name, e) in obj {
        if name == "__metadata__" {
            if let Some(m) = e.as_object() {
                for (k, v) in m {
                    metadata.insert(k.clone(), v.as_str().unwrap_or_default().to_string());
                }
            }
            continue;
        }
        let bad = |what: &str| format!("safetensors tensor {name:?}: {what}");
        let dtype = Dtype::parse(e["dtype"].as_str().ok_or_else(|| bad("no dtype"))?).map_err(|m| bad(&m))?;
        let shape: Vec<usize> = e["shape"]
            .as_array()
            .ok_or_else(|| bad("no shape"))?
            .iter()
            .map(|d| {
                d.as_u64()
                    .map(|d| d as usize)
                    .ok_or_else(|| bad("shape is not whole numbers"))
            })
            .collect::<Result<_, _>>()?;
        let off = e["data_offsets"].as_array().ok_or_else(|| bad("no data_offsets"))?;
        let (b, en) = match (off.first().and_then(Json::as_u64), off.get(1).and_then(Json::as_u64)) {
            (Some(b), Some(en)) if off.len() == 2 && b <= en => (b, en),
            _ => return Err(bad("data_offsets is not [begin, end]")),
        };
        let count = shape
            .iter()
            .try_fold(1usize, |a, &d| a.checked_mul(d))
            .ok_or_else(|| bad("shape too large"))?;
        let bytes = count.checked_mul(dtype.size()).ok_or_else(|| bad("shape too large"))? as u64;
        if en - b != bytes {
            return Err(bad(&format!(
                "{} bytes of data for shape {shape:?} of {}",
                en - b,
                dtype.name()
            )));
        }
        let (start, end) = (
            base.checked_add(b).ok_or_else(|| bad("offset"))?,
            base.checked_add(en).ok_or_else(|| bad("offset"))?,
        );
        if end > file_len {
            return Err(bad("data past the end of the file"));
        }
        tensors.insert(
            name.clone(),
            Info {
                dtype,
                shape,
                start,
                end,
            },
        );
    }
    Ok((tensors, metadata))
}

impl SafeTensors {
    pub fn open(path: &Path) -> Result<SafeTensors, String> {
        let err = |e: std::io::Error| format!("{}: {e}", path.display());
        let mut f = File::open(path).map_err(err)?;
        let file_len = f.metadata().map_err(err)?.len();
        let mut n = [0u8; 8];
        f.read_exact(&mut n)
            .map_err(|_| format!("{}: not a safetensors file (too short)", path.display()))?;
        let hl = u64::from_le_bytes(n);
        if hl.checked_add(8).is_none_or(|e| e > file_len) {
            return Err(format!(
                "{}: not a safetensors file (header length {hl})",
                path.display()
            ));
        }
        let mut json = vec![0u8; hl as usize];
        f.read_exact(&mut json).map_err(err)?;
        let (tensors, metadata) = parse_header(&json, hl, file_len).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(SafeTensors {
            path: path.to_path_buf(),
            tensors,
            metadata,
            file: Mutex::new(f),
        })
    }

    /// a tensor's values as f32
    pub fn read(&self, name: &str) -> Result<Vec<f32>, String> {
        let info = self
            .tensors
            .get(name)
            .ok_or_else(|| format!("{}: no tensor {name:?}", self.path.display()))?;
        let mut raw = vec![0u8; (info.end - info.start) as usize];
        let mut f = self.file.lock().unwrap();
        f.seek(SeekFrom::Start(info.start))
            .and_then(|_| f.read_exact(&mut raw))
            .map_err(|e| format!("{}: {name}: {e}", self.path.display()))?;
        Ok(to_f32(info.dtype, &raw))
    }
}

/// a model's weights: one safetensors file or the shards of an index
#[derive(Debug)]
pub struct Checkpoint {
    files: Vec<SafeTensors>,
    /// tensor name → file
    index: BTreeMap<String, usize>,
}

impl Checkpoint {
    /// `model.safetensors.index.json` (sharded), else `model.safetensors`, else every
    /// `*.safetensors` file in `dir`
    pub fn open_dir(dir: &Path) -> Result<Checkpoint, String> {
        let idx = dir.join("model.safetensors.index.json");
        if idx.exists() {
            return Checkpoint::open_index(&idx);
        }
        let one = dir.join("model.safetensors");
        if one.exists() {
            return Checkpoint::from_files(&[one]);
        }
        let mut all: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "safetensors"))
            .collect();
        all.sort();
        if all.is_empty() {
            return Err(format!(
                "{}: no safetensors weights (model.safetensors or an index)",
                dir.display()
            ));
        }
        Checkpoint::from_files(&all)
    }

    /// a sharded checkpoint from its index file
    pub fn open_index(path: &Path) -> Result<Checkpoint, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Json = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let map = v["weight_map"]
            .as_object()
            .ok_or(format!("{}: no weight_map", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut names: Vec<String> = map.values().filter_map(|f| f.as_str().map(String::from)).collect();
        names.sort();
        names.dedup();
        for n in &names {
            if n.contains("..") || Path::new(n).is_absolute() {
                return Err(format!("{}: shard {n:?} is outside the model folder", path.display()));
            }
        }
        let files: Vec<PathBuf> = names.iter().map(|n| dir.join(n)).collect();
        let cp = Checkpoint::from_files(&files)?;
        // every tensor the index names must be in its shard
        for (t, f) in map {
            let f = f.as_str().unwrap_or_default();
            let i = names.iter().position(|n| n == f).expect("listed");
            if !cp.files[i].tensors.contains_key(t) {
                return Err(format!("{}: tensor {t:?} is not in shard {f}", path.display()));
            }
        }
        Ok(cp)
    }

    pub fn from_files(paths: &[PathBuf]) -> Result<Checkpoint, String> {
        let files: Vec<SafeTensors> = paths.iter().map(|p| SafeTensors::open(p)).collect::<Result<_, _>>()?;
        let mut index = BTreeMap::new();
        for (i, f) in files.iter().enumerate() {
            for name in f.tensors.keys() {
                if index.insert(name.clone(), i).is_some() {
                    return Err(format!("tensor {name:?} is in two shards"));
                }
            }
        }
        Ok(Checkpoint { files, index })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(String::as_str)
    }

    pub fn has(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn info(&self, name: &str) -> Option<&Info> {
        self.index.get(name).and_then(|&i| self.files[i].tensors.get(name))
    }

    /// a tensor as f32, checked against the shape the caller expects
    pub fn get(&self, name: &str, shape: &[usize]) -> Result<Vec<f32>, String> {
        let i = *self
            .index
            .get(name)
            .ok_or_else(|| format!("the checkpoint has no tensor {name:?}"))?;
        let info = &self.files[i].tensors[name];
        if info.shape != shape {
            return Err(format!(
                "tensor {name:?} has shape {:?}, the config implies {shape:?}",
                info.shape
            ));
        }
        self.files[i].read(name)
    }
}

/// writes a safetensors file (tensors in the order given, each as `dtype`)
pub fn write(path: &Path, tensors: &[(&str, Dtype, &[usize], &[f32])]) -> Result<(), String> {
    let mut header = serde_json::Map::new();
    let mut data = Vec::new();
    for (name, dtype, shape, values) in tensors {
        if shape.iter().product::<usize>() != values.len() {
            return Err(format!("tensor {name:?}: {} values for shape {shape:?}", values.len()));
        }
        let begin = data.len();
        data.extend(from_f32(*dtype, values));
        header.insert(
            name.to_string(),
            serde_json::json!({"dtype": dtype.name(), "shape": shape, "data_offsets": [begin, data.len()]}),
        );
    }
    let mut json = serde_json::to_vec(&Json::Object(header)).expect("json");
    // the format pads the header with spaces to a multiple of 8
    while !json.len().is_multiple_of(8) {
        json.push(b' ');
    }
    let mut f = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(&(json.len() as u64).to_le_bytes())
        .and_then(|_| f.write_all(&json))
        .and_then(|_| f.write_all(&data))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("anend-st-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn half_floats_convert_exactly() {
        for h in 0..=u16::MAX {
            let f = f16_to_f32(h);
            if f.is_nan() {
                assert!(f32_to_f16(f) & 0x7c00 == 0x7c00);
                continue;
            }
            assert_eq!(f32_to_f16(f), h, "half {h:#06x} → {f} back");
            assert_eq!(f32_to_bf16(bf16_to_f32(h)), h);
        }
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24));
        assert_eq!(f32_to_f16(65520.0), 0x7c00, "rounds up to infinity");
        assert_eq!(f32_to_f16(1.0 + 2f32.powi(-11)), 0x3c00, "ties to even");
        assert_eq!(bf16_to_f32(0x3f80), 1.0);
        assert_eq!(f32_to_bf16(1.0 + 2f32.powi(-8)), 0x3f80, "ties to even");
    }

    #[test]
    fn files_and_shards_read_back() {
        let d = dir("shards");
        let a: Vec<f32> = (0..6).map(|i| i as f32 * 0.5 - 1.0).collect();
        let b = vec![1.5f32, -0.25, 3.0, 0.0];
        write(
            &d.join("one.safetensors"),
            &[("a", Dtype::F32, &[2, 3], &a), ("b", Dtype::BF16, &[4], &b)],
        )
        .unwrap();
        write(
            &d.join("two.safetensors"),
            &[("c", Dtype::F16, &[2, 2], &b), ("d", Dtype::F64, &[1], &[7.0])],
        )
        .unwrap();
        std::fs::write(
            d.join("model.safetensors.index.json"),
            r#"{"metadata": {"total_size": 1}, "weight_map": {"a": "one.safetensors", "b": "one.safetensors",
               "c": "two.safetensors", "d": "two.safetensors"}}"#,
        )
        .unwrap();
        let cp = Checkpoint::open_dir(&d).unwrap();
        assert_eq!(cp.names().collect::<Vec<_>>(), ["a", "b", "c", "d"]);
        assert_eq!(cp.get("a", &[2, 3]).unwrap(), a);
        assert_eq!(cp.get("b", &[4]).unwrap(), b);
        assert_eq!(cp.get("c", &[2, 2]).unwrap(), b);
        assert_eq!(cp.get("d", &[1]).unwrap(), [7.0]);
        assert!(cp.get("a", &[3, 2]).unwrap_err().contains("shape [2, 3]"));
        assert!(cp.get("z", &[1]).unwrap_err().contains("no tensor"));
        // a single file without an index
        let d2 = dir("single");
        write(
            &d2.join("model.safetensors"),
            &[("x", Dtype::F16, &[3], &[1.0, 2.0, -0.5])],
        )
        .unwrap();
        assert_eq!(
            Checkpoint::open_dir(&d2).unwrap().get("x", &[3]).unwrap(),
            [1.0, 2.0, -0.5]
        );
    }

    #[test]
    fn bad_files_are_refused_in_words() {
        let d = dir("bad");
        let p = d.join("x.safetensors");
        std::fs::write(&p, [1, 2, 3]).unwrap();
        assert!(SafeTensors::open(&p).unwrap_err().contains("too short"));
        let mk = |json: &str, data: usize| {
            let mut v = (json.len() as u64).to_le_bytes().to_vec();
            v.extend_from_slice(json.as_bytes());
            v.extend(std::iter::repeat_n(0u8, data));
            std::fs::write(&p, v).unwrap();
            SafeTensors::open(&p).map(|_| ())
        };
        assert!(
            mk(r#"{"w": {"dtype": "I8", "shape": [2], "data_offsets": [0, 2]}}"#, 2)
                .unwrap_err()
                .contains("dtype I8 is not supported")
        );
        assert!(
            mk(r#"{"w": {"dtype": "F32", "shape": [2], "data_offsets": [0, 4]}}"#, 4)
                .unwrap_err()
                .contains("4 bytes of data for shape [2]")
        );
        assert!(
            mk(r#"{"w": {"dtype": "F32", "shape": [2], "data_offsets": [0, 8]}}"#, 4)
                .unwrap_err()
                .contains("past the end")
        );
        let mut v = u64::MAX.to_le_bytes().to_vec();
        v.extend([0; 8]);
        std::fs::write(&p, v).unwrap();
        assert!(SafeTensors::open(&p).unwrap_err().contains("header length"));
    }
}
