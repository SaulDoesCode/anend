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
use super::model::{Config, GenSpec, HeadSpec, Net, Rope};
use super::{Classifier, Meta, Steer};

pub const MAGIC: &[u8; 8] = b"ANENDCLS";
/// 2 adds optional feature channels, NUM tokens, causal models and the generative head;
/// version 1 files read as version 2 files with the gematria channels on
pub const VERSION: u32 = 2;

/// the largest shapes a file may claim, so a damaged or hostile header can't make the
/// reader allocate more than the file itself could hold
pub const MAX_D_MODEL: usize = 2048;
pub const MAX_HEAD_DIM: usize = 512;
pub const MAX_HEADS: usize = 64;
pub const MAX_LAYERS: usize = 64;
pub const MAX_FF: usize = 16_384;
pub const MAX_LEN: usize = 8192;
pub const MAX_OUT_HEADS: usize = 16;
pub const MAX_CLASSES: usize = 1024;
pub const MAX_EXTRA: usize = 1 << 16;

/// the shape limits for models with their own vocabulary (imported pretrained models),
/// set far past any open checkpoint: they only stop a hostile header from asking for
/// absurd tables that the weights' size check can't see (the rotary table)
pub const BIG_D_MODEL: usize = 1 << 16;
pub const BIG_HEAD_DIM: usize = 1 << 12;
pub const BIG_HEADS: usize = 1 << 12;
pub const BIG_LAYERS: usize = 1 << 12;
pub const BIG_FF: usize = 1 << 20;
pub const BIG_LEN: usize = 1 << 21;
pub const BIG_VOCAB: usize = 1 << 24;

/// a model's tokenizer as saved: its `tokenizer.json` and the lexicon's tokens
#[derive(Serialize, Deserialize)]
struct SavedLexicon {
    tokenizer: String,
    start: u32,
    end: u32,
    #[serde(default)]
    stops: Vec<u32>,
}

#[derive(Serialize, Deserialize)]
struct Header {
    config: Config,
    /// (heads, ff) of each block, which growth and pruning change
    blocks: Vec<(usize, usize)>,
    heads: Vec<HeadSpec>,
    /// the generative head (version 2)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generator: Option<GenSpec>,
    steer: Steer,
    meta: Meta,
    /// the tokenizer of a model with its own vocabulary
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lexicon: Option<SavedLexicon>,
}

impl Classifier {
    pub fn to_bytes(&self) -> Vec<u8> {
        let header = Header {
            config: self.net.cfg.clone(),
            blocks: self.net.blocks.iter().map(|b| (b.heads, b.ff)).collect(),
            heads: self.net.heads.iter().map(|h| h.spec.clone()).collect(),
            generator: self.net.generator.clone(),
            steer: self.steer.clone(),
            meta: self.meta.clone(),
            lexicon: self.net.lexicon.as_ref().map(|l| SavedLexicon {
                tokenizer: l.tokenizer.source().to_string(),
                start: l.start,
                end: l.end,
                stops: l.stops.clone(),
            }),
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
        // the reader is careful, but a file is data: a panic anywhere in it is a bad file
        std::panic::catch_unwind(|| Classifier::read_bytes(b))
            .unwrap_or_else(|_| Err("not a classifier file this build reads".into()))
    }

    fn read_bytes(b: &[u8]) -> Result<Classifier, String> {
        let take = |at: usize, n: usize| b.get(at..at + n).ok_or_else(|| "file too short".to_string());
        if take(0, 8)? != MAGIC {
            return Err("not a classifier file".into());
        }
        let version = u32::from_le_bytes(take(8, 4)?.try_into().unwrap());
        if version != 1 && version != VERSION {
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
        if 16 + hl > body {
            return Err("file too short".into());
        }
        let h: Header = serde_json::from_slice(take(16, hl)?).map_err(|e| e.to_string())?;
        let at = 16 + hl;
        let n = u64::from_le_bytes(take(at, 8)?.try_into().unwrap());
        let expected = expected_weights(&h)?;
        if n != expected as u64 || (at + 8).checked_add(expected.checked_mul(4).ok_or("too big")?) != Some(body) {
            return Err("weights do not match the header".into());
        }
        let mut cfg = h.config.clone();
        cfg.layers = h.blocks.len();
        let mut net = Net::with_gen(cfg, &h.heads, h.generator.clone(), 0)?;
        // reshape the blocks to what was saved, then fill every tensor
        for (blk, &(heads, ff)) in net.blocks.iter_mut().zip(&h.blocks) {
            let (d, a) = (h.config.d_model, heads * h.config.head_dim);
            blk.heads = heads;
            blk.ff = ff;
            let w3 = blk.qkv_width(&h.config);
            blk.wqkv = vec![0.0; d * w3];
            if h.config.qkv_bias {
                blk.bqkv = vec![0.0; w3];
            }
            blk.wo = vec![0.0; a * d];
            blk.wg = vec![0.0; d * ff];
            blk.wu = vec![0.0; d * ff];
            blk.wd = vec![0.0; ff * d];
        }
        let mut at = at + 8;
        let n = n as usize;
        if n != net.weight_count() || at + 4 * n != body {
            return Err("weights do not match the header".into());
        }
        for (_, t) in net.params_mut() {
            for v in t.iter_mut() {
                *v = f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
                at += 4;
            }
        }
        net.rope = Rope::of(&net.cfg);
        if let Some(l) = h.lexicon {
            let tokenizer = super::bpe::Tokenizer::from_json(&l.tokenizer)?;
            let ok = |t: u32| (t as usize) < net.cfg.vocab_size;
            if !ok(l.start) || !ok(l.end) || !l.stops.iter().all(|&t| ok(t)) {
                return Err("the classifier file's start or end token is out of range".into());
            }
            net.lexicon = Some(std::sync::Arc::new(super::bpe::Lexicon {
                tokenizer,
                start: l.start,
                end: l.end,
                stops: l.stops,
            }));
        } else if net.cfg.vocab_size > 0 {
            return Err("the classifier file has a vocabulary but no tokenizer".into());
        }
        Ok(Classifier {
            net,
            steer: h.steer,
            meta: h.meta,
            opt: None,
        })
    }

    /// names a classifier by what it computes: its shapes, heads and every weight, but
    /// not its steering or history (the same weights steered two ways hash the same)
    pub fn weights_hash(&self) -> u64 {
        let blocks: Vec<(usize, usize)> = self.net.blocks.iter().map(|b| (b.heads, b.ff)).collect();
        let heads: Vec<&HeadSpec> = self.net.heads.iter().map(|h| &h.spec).collect();
        let shape = serde_json::to_vec(&(&self.net.cfg, &blocks, &heads)).expect("shapes serialise");
        let mut h = fnv(0x636c73, &shape);
        let mut buf = Vec::with_capacity(4096);
        for (_, t) in self.net.params() {
            for v in t {
                buf.extend_from_slice(&v.to_le_bytes());
                if buf.len() >= 4096 {
                    h = fnv(h, &buf);
                    buf.clear();
                }
            }
        }
        fnv(h, &buf)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        std::fs::write(path, self.to_bytes()).map_err(|e| e.to_string())
    }

    pub fn load(path: &std::path::Path) -> Result<Classifier, String> {
        Classifier::from_bytes(&std::fs::read(path).map_err(|e| e.to_string())?)
    }
}

/// how many weights a header's shapes hold, refusing shapes past the limits above
fn expected_weights(h: &Header) -> Result<usize, String> {
    use super::model::Pooling;
    use super::text::{LETTERS, ROOT_BUCKETS, SIDE, SUM_BUCKETS, VOCAB};
    let c = &h.config;
    let bad = |what: &str| Err(format!("the classifier file's {what} is out of range"));
    let big = c.vocab_size > 0;
    let pick = |small: usize, large: usize| if big { large } else { small };
    let (max_d, max_hd, max_heads) = (
        pick(MAX_D_MODEL, BIG_D_MODEL),
        pick(MAX_HEAD_DIM, BIG_HEAD_DIM),
        pick(MAX_HEADS, BIG_HEADS),
    );
    let (max_layers, max_ff, max_len) = (
        pick(MAX_LAYERS, BIG_LAYERS),
        pick(MAX_FF, BIG_FF),
        pick(MAX_LEN, BIG_LEN),
    );
    if c.d_model == 0 || c.d_model > max_d {
        return bad("width");
    }
    if c.head_dim == 0 || c.head_dim > max_hd || !c.head_dim.is_multiple_of(2) {
        return bad("head width");
    }
    if c.heads == 0 || c.heads > max_heads || c.d_ff == 0 || c.d_ff > max_ff {
        return bad("block shape");
    }
    if c.vocab_size > BIG_VOCAB || (big && h.lexicon.is_none()) {
        return bad("vocabulary");
    }
    if c.kv_heads > 0 && (!c.heads.is_multiple_of(c.kv_heads) || h.blocks.iter().any(|b| b.0 != c.heads)) {
        return bad("key/value heads");
    }
    if h.blocks.is_empty() || h.blocks.len() > max_layers {
        return bad("depth");
    }
    // windows overlap by at most three quarters: a stride of 1 on long windows would make
    // every long text cost thousands of times its length to read
    if c.max_len < 3 || c.max_len > max_len || c.stride == 0 || c.stride > max_len || c.stride * 4 < c.max_len - 2 {
        return bad("window");
    }
    if c.extra > MAX_EXTRA {
        return bad("extra input width");
    }
    if !(0.0..1.0).contains(&c.dropout)
        || !(c.rope_base.is_finite() && c.rope_base > 1.0)
        || !(c.norm_eps.is_finite() && c.norm_eps > 0.0)
    {
        return bad("settings");
    }
    let g = c.gains;
    if ![g.letter, g.word, g.side, g.extra].iter().all(|x| x.is_finite()) {
        return bad("gains");
    }
    if (h.heads.is_empty() && h.generator.is_none())
        || h.heads.len() > MAX_OUT_HEADS
        || h.heads
            .iter()
            .any(|x| x.classes.is_empty() || x.classes.len() > MAX_CLASSES)
    {
        return bad("heads");
    }
    // the steering is read for every head, its bias zipped with the logits
    if h.steer.heads.len() != h.heads.len()
        || h.steer.heads.iter().zip(&h.heads).any(|(s, x)| {
            s.bias.len() != x.classes.len()
                || !s.bias.iter().all(|b| b.is_finite())
                || !(s.temperature.is_finite() && s.temperature > 0.0)
                || !(0.0..=1.0).contains(&s.threshold)
        })
    {
        return bad("steering");
    }
    let d = c.d_model;
    let mut n: usize = if big { c.vocab_size } else { VOCAB + c.numbers as usize } * d;
    n += if c.letters { LETTERS * d } else { 0 }
        + if c.words {
            (3 * SUM_BUCKETS + ROOT_BUCKETS) * d
        } else {
            0
        };
    if c.numbers {
        n += d;
    }
    if h.generator.is_some() {
        if !c.causal {
            return bad("generative head (needs a causal model)");
        }
        n += (d + 1) * c.gen_out();
    }
    for &(heads, ff) in &h.blocks {
        if heads == 0 || heads > max_heads || ff == 0 || ff > max_ff {
            return bad("block shape");
        }
        let a = heads * c.head_dim;
        let w3 = (heads + 2 * c.kv_of(heads)) * c.head_dim;
        let qk = if c.qk_norm { 2 * c.head_dim } else { 0 };
        n += 2 * d + d * w3 + a * d + qk + 3 * d * ff + if c.qkv_bias { w3 } else { 0 };
    }
    n += d + if c.pooling == Pooling::Attention { d } else { 0 };
    let pooled = if c.pooling == Pooling::ClsMean { 2 * d } else { d };
    let z = pooled + if c.side { SIDE } else { 0 } + c.extra;
    for x in &h.heads {
        n += (z + 1) * x.classes.len();
    }
    Ok(n)
}
