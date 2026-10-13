//! The classifier file: a versioned binary format.
//!
//! | bytes | holds |
//! |---|---|
//! | 8 | magic `ANENDCLS` |
//! | 4 | format version, u32 little-endian |
//! | 4 | header length h, u32 LE |
//! | h | header, JSON: config, block shapes, heads, steer, meta |
//! | 8 | weight count n, u64 LE |
//! | 4·n | the weights, f32 LE, tensors in `Net::params` order |
//! | 8 | FNV-1a of everything before, u64 LE |

use serde::{Deserialize, Serialize};

use super::math::fnv;
use super::model::{Config, HeadSpec, Net, Rope};
use super::{Classifier, Meta, Steer};

pub const MAGIC: &[u8; 8] = b"ANENDCLS";
pub const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Header {
    config: Config,
    /// (heads, ff) of each block, which growth and pruning change
    blocks: Vec<(usize, usize)>,
    heads: Vec<HeadSpec>,
    steer: Steer,
    meta: Meta,
}

impl Classifier {
    pub fn to_bytes(&self) -> Vec<u8> {
        let header = Header {
            config: self.net.cfg.clone(),
            blocks: self.net.blocks.iter().map(|b| (b.heads, b.ff)).collect(),
            heads: self.net.heads.iter().map(|h| h.spec.clone()).collect(),
            steer: self.steer.clone(),
            meta: self.meta.clone(),
        };
        let json = serde_json::to_vec(&header).expect("header serialises");
        let n = self.net.weight_count();
        let mut out = Vec::with_capacity(24 + json.len() + 4 * n + 8);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&json);
        out.extend_from_slice(&(n as u64).to_le_bytes());
        for (_, t) in self.net.params() {
            for v in t {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        let sum = fnv(0, &out);
        out.extend_from_slice(&sum.to_le_bytes());
        out
    }

    pub fn from_bytes(b: &[u8]) -> Result<Classifier, String> {
        let take = |at: usize, n: usize| b.get(at..at + n).ok_or_else(|| "file too short".to_string());
        if take(0, 8)? != MAGIC {
            return Err("not a classifier file".into());
        }
        let version = u32::from_le_bytes(take(8, 4)?.try_into().unwrap());
        if version != VERSION {
            return Err(format!(
                "classifier format version {version}, this build reads {VERSION}"
            ));
        }
        let body = b.len().checked_sub(8).ok_or("file too short")?;
        let sum = u64::from_le_bytes(take(body, 8)?.try_into().unwrap());
        if fnv(0, &b[..body]) != sum {
            return Err("classifier file is damaged (checksum)".into());
        }
        let hl = u32::from_le_bytes(take(12, 4)?.try_into().unwrap()) as usize;
        let h: Header = serde_json::from_slice(take(16, hl)?).map_err(|e| e.to_string())?;
        let mut cfg = h.config.clone();
        cfg.layers = h.blocks.len();
        let mut net = Net::new(cfg, &h.heads, 0)?;
        // reshape the blocks to what was saved, then fill every tensor
        for (blk, &(heads, ff)) in net.blocks.iter_mut().zip(&h.blocks) {
            let (d, a) = (h.config.d_model, heads * h.config.head_dim);
            blk.heads = heads;
            blk.ff = ff;
            blk.wqkv = vec![0.0; d * 3 * a];
            blk.wo = vec![0.0; a * d];
            blk.wg = vec![0.0; d * ff];
            blk.wu = vec![0.0; d * ff];
            blk.wd = vec![0.0; ff * d];
        }
        let mut at = 16 + hl;
        let n = u64::from_le_bytes(take(at, 8)?.try_into().unwrap()) as usize;
        at += 8;
        if n != net.weight_count() || at + 4 * n != body {
            return Err("weights do not match the header".into());
        }
        for (_, t) in net.params_mut() {
            for v in t.iter_mut() {
                *v = f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
                at += 4;
            }
        }
        net.rope = Rope::new(net.cfg.max_len, net.cfg.head_dim, net.cfg.rope_base);
        Ok(Classifier {
            net,
            steer: h.steer,
            meta: h.meta,
            opt: None,
        })
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        std::fs::write(path, self.to_bytes()).map_err(|e| e.to_string())
    }

    pub fn load(path: &std::path::Path) -> Result<Classifier, String> {
        Classifier::from_bytes(&std::fs::read(path).map_err(|e| e.to_string())?)
    }
}
