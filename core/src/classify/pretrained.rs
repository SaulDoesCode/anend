//! Pretrained open models in the engine: a HuggingFace model folder (`config.json`,
//! `tokenizer.json`, safetensors weights, optionally `generation_config.json`) becomes a
//! `Classifier` whose trunk and generative head are the checkpoint's, reading the
//! checkpoint's own vocabulary (`bpe`). From there it is an ordinary engine model: it
//! runs, generates through `generate` (critics, constraints, guidance, reinforcement),
//! fine-tunes through `train`, takes extra classifier heads on its trunk, and saves as
//! a classifier file (weights in f32 plus the tokenizer).
//!
//! Supported: the Llama family as `transformers` writes it, `model_type` `llama`
//! (Llama 1–3, TinyLlama, SmolLM/SmolLM2, OpenLLaMA, …), `mistral`, `qwen2` (q/k/v
//! biases) and `qwen3` (QK-norm). That is RMSNorm, SwiGLU (`silu`), rotary positions
//! (`rope_theta`, `rope_scaling` `linear` or `llama3`), grouped key/value heads, an
//! explicit `head_dim`, tied or separate input and output embeddings. Anything else in
//! the config that would change the computation is refused with the field named.
//!
//! Numbers: the checkpoint's weights become f32 (F16/BF16 exactly), and the engine
//! computes in f32 with its own deterministic kernels, so outputs match the reference
//! implementation to rounding (not bit for bit), and match themselves bit for bit on
//! any machine.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value as Json;

use super::bpe::{Lexicon, Tokenizer};
use super::kernels::transpose;
use super::model::{Config, Gains, GenSpec, HeadSpec, Net, Pooling, RopeScaling};
use super::safetensors::Checkpoint;
use super::{Classifier, Meta, Steer};

/// the parts of `config.json` the engine uses, checked
#[derive(Clone, Debug, PartialEq)]
pub struct Arch {
    pub model_type: String,
    pub hidden: usize,
    pub layers: usize,
    pub heads: usize,
    pub kv_heads: usize,
    pub head_dim: usize,
    pub ff: usize,
    pub vocab: usize,
    pub context: usize,
    pub eps: f32,
    pub theta: f32,
    pub scaling: Option<RopeScaling>,
    pub tied: bool,
    pub bos: Option<u32>,
    pub eos: Vec<u32>,
}

/// how a pretrained model is brought in
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// tokens per window; default the model's context length (`max_position_embeddings`,
    /// or its sliding window if shorter)
    pub max_len: Option<usize>,
    /// classifier heads to put on the trunk (new, untrained), besides the generator
    pub heads: Vec<HeadSpec>,
    /// the generative head's name
    pub generator: String,
    /// dropout when fine-tuning
    pub dropout: f32,
    /// seeds the new heads
    pub seed: u64,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            max_len: None,
            heads: Vec::new(),
            generator: "next".into(),
            dropout: 0.0,
            seed: 0,
        }
    }
}

fn need_usize(v: &Json, f: &str) -> Result<usize, String> {
    v[f].as_u64()
        .map(|x| x as usize)
        .ok_or(format!("config.json: {f} is missing or not a whole number"))
}

fn ids_of(v: &Json) -> Vec<u32> {
    match v {
        Json::Number(n) => n.as_u64().map(|x| vec![x as u32]).unwrap_or_default(),
        Json::Array(a) => a.iter().filter_map(|x| x.as_u64().map(|x| x as u32)).collect(),
        _ => Vec::new(),
    }
}

impl Arch {
    /// reads and checks a `config.json` (with `generation_config.json`'s end tokens,
    /// when given)
    pub fn from_json(text: &str, generation: Option<&str>) -> Result<Arch, String> {
        let v: Json = serde_json::from_str(text).map_err(|e| format!("config.json: {e}"))?;
        let model_type = v["model_type"].as_str().unwrap_or("").to_string();
        if !["llama", "mistral", "qwen2", "qwen3"].contains(&model_type.as_str()) {
            return Err(format!(
                "config.json: model_type {model_type:?} is not supported (llama, mistral, qwen2, qwen3)"
            ));
        }
        let unsupported = |f: &str, why: &str| Err(format!("config.json: {f} {why} is not supported"));
        let act = v["hidden_act"]
            .as_str()
            .or(v["hidden_activation"].as_str())
            .unwrap_or("silu");
        if act != "silu" {
            return unsupported("hidden_act", &format!("{act:?} (silu is)"));
        }
        if v["attention_bias"].as_bool() == Some(true) {
            return unsupported("attention_bias", "true (the output projection's bias)");
        }
        if v["mlp_bias"].as_bool() == Some(true) {
            return unsupported("mlp_bias", "true");
        }
        if v["pretraining_tp"].as_u64().is_some_and(|t| t > 1) {
            return unsupported("pretraining_tp", "above 1");
        }
        if v["num_experts"].as_u64().is_some_and(|n| n > 0) || v["num_local_experts"].as_u64().is_some_and(|n| n > 0) {
            return unsupported("num_experts", "(mixture of experts)");
        }
        let hidden = need_usize(&v, "hidden_size")?;
        let heads = need_usize(&v, "num_attention_heads")?;
        let kv_heads = v["num_key_value_heads"].as_u64().map_or(heads, |x| x as usize);
        let head_dim = match v["head_dim"].as_u64() {
            Some(h) => h as usize,
            None if heads > 0 && hidden.is_multiple_of(heads) => hidden / heads,
            None => return Err("config.json: hidden_size is not a multiple of num_attention_heads".into()),
        };
        if heads == 0 || kv_heads == 0 || !heads.is_multiple_of(kv_heads) {
            return Err(format!(
                "config.json: num_attention_heads {heads} is not a multiple of num_key_value_heads {kv_heads}"
            ));
        }
        if head_dim == 0 || !head_dim.is_multiple_of(2) {
            return Err(format!("config.json: head_dim {head_dim} must be even"));
        }
        if v["partial_rotary_factor"].as_f64().is_some_and(|f| f != 1.0) {
            return unsupported("partial_rotary_factor", "other than 1");
        }
        let mut context = v["max_position_embeddings"].as_u64().map_or(4096, |x| x as usize);
        let sliding = match model_type.as_str() {
            "qwen2" | "qwen3" => v["use_sliding_window"].as_bool() == Some(true),
            _ => true,
        };
        if sliding && let Some(w) = v["sliding_window"].as_u64() {
            // windows no longer than the sliding window see exactly what it lets through
            context = context.min(w as usize);
        }
        let scaling = match &v["rope_scaling"] {
            Json::Null => None,
            s => {
                let kind = s["rope_type"].as_str().or(s["type"].as_str()).unwrap_or("");
                let f = |name: &str| {
                    s[name]
                        .as_f64()
                        .ok_or(format!("config.json: rope_scaling.{name} is missing"))
                };
                match kind {
                    "default" => None,
                    "linear" => Some(RopeScaling::Linear { factor: f("factor")? }),
                    "llama3" => Some(RopeScaling::Llama3 {
                        factor: f("factor")?,
                        low_freq_factor: f("low_freq_factor")?,
                        high_freq_factor: f("high_freq_factor")?,
                        original_max_position_embeddings: f("original_max_position_embeddings")?,
                    }),
                    other => return unsupported("rope_scaling", &format!("type {other:?} (linear and llama3 are)")),
                }
            }
        };
        let mut eos = ids_of(&v["eos_token_id"]);
        if let Some(g) = generation {
            let g: Json = serde_json::from_str(g).map_err(|e| format!("generation_config.json: {e}"))?;
            for t in ids_of(&g["eos_token_id"]) {
                if !eos.contains(&t) {
                    eos.push(t);
                }
            }
        }
        Ok(Arch {
            hidden,
            layers: need_usize(&v, "num_hidden_layers")?,
            heads,
            kv_heads,
            head_dim,
            ff: need_usize(&v, "intermediate_size")?,
            vocab: need_usize(&v, "vocab_size")?,
            context,
            eps: v["rms_norm_eps"].as_f64().unwrap_or(1e-6) as f32,
            theta: v["rope_theta"].as_f64().unwrap_or(10000.0) as f32,
            scaling,
            tied: v["tie_word_embeddings"].as_bool().unwrap_or(false),
            bos: v["bos_token_id"].as_u64().map(|x| x as u32),
            eos,
            model_type,
        })
    }

    /// the engine configuration (biases and QK-norm as the checkpoint has them)
    pub fn config(&self, o: &Options, qkv_bias: bool, qk_norm: bool) -> Result<Config, String> {
        let max_len = o.max_len.unwrap_or(self.context).max(3);
        let cfg = Config {
            d_model: self.hidden,
            head_dim: self.head_dim,
            heads: self.heads,
            layers: self.layers,
            d_ff: self.ff,
            max_len,
            stride: ((max_len - 2) / 2).max(1),
            dropout: o.dropout,
            pooling: Pooling::Mean,
            qk_norm,
            rope_base: self.theta,
            norm_eps: self.eps,
            side: false,
            extra: 0,
            gains: Gains::default(),
            zero_out: false,
            letters: false,
            words: false,
            causal: true,
            numbers: false,
            kv_heads: if self.kv_heads == self.heads { 0 } else { self.kv_heads },
            vocab_size: self.vocab,
            qkv_bias,
            rope_scaling: self.scaling.clone(),
        };
        cfg.validate()
            .map_err(|e| format!("pretrained {}: {e}", self.model_type))?;
        Ok(cfg)
    }
}

/// a model folder's files
pub struct Folder {
    pub dir: PathBuf,
    pub arch: Arch,
    pub tokenizer: Tokenizer,
    pub weights: Checkpoint,
}

impl Folder {
    pub fn open(dir: &Path) -> Result<Folder, String> {
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name)).map_err(|e| format!("{}: {e}", dir.join(name).display()))
        };
        let config = read("config.json")?;
        let generation = read("generation_config.json").ok();
        let arch = Arch::from_json(&config, generation.as_deref()).map_err(|e| format!("{}: {e}", dir.display()))?;
        let tokenizer = Tokenizer::load(&dir.join("tokenizer.json"))?;
        let weights = Checkpoint::open_dir(dir)?;
        Ok(Folder {
            dir: dir.to_path_buf(),
            arch,
            tokenizer,
            weights,
        })
    }

    /// the lexicon: BOS (or the first end token) to start, the end tokens to stop
    pub fn lexicon(&self) -> Result<Lexicon, String> {
        let a = &self.arch;
        if self.tokenizer.size() > a.vocab {
            return Err(format!(
                "tokenizer.json has {} tokens, more than the model's vocab_size {}",
                self.tokenizer.size(),
                a.vocab
            ));
        }
        let end = *a.eos.first().ok_or("config.json: eos_token_id is missing")?;
        let start = a.bos.unwrap_or(end);
        if [start, end].iter().chain(&a.eos).any(|&t| t as usize >= a.vocab) {
            return Err("config.json: bos/eos token ids are past vocab_size".into());
        }
        Ok(Lexicon {
            tokenizer: self.tokenizer.clone(),
            start,
            end,
            stops: a.eos[1..].to_vec(),
        })
    }

    /// the engine model with the checkpoint's weights
    pub fn model(&self, o: &Options) -> Result<Classifier, String> {
        let a = &self.arch;
        let w = &self.weights;
        let qkv_bias = w.has("model.layers.0.self_attn.q_proj.bias");
        let qk_norm = w.has("model.layers.0.self_attn.q_norm.weight");
        if w.has("model.layers.0.self_attn.o_proj.bias") {
            return Err("the checkpoint has an output projection bias (not supported)".into());
        }
        let cfg = a.config(o, qkv_bias, qk_norm)?;
        let mut net = Net::with_gen(cfg, &o.heads, Some(GenSpec::new(&o.generator)), o.seed)?;
        net.lexicon = Some(Arc::new(self.lexicon()?));
        let (d, dh, v) = (a.hidden, a.head_dim, a.vocab);
        let (qa, kva) = (a.heads * dh, a.kv_heads * dh);
        net.tok = w.get("model.embed_tokens.weight", &[v, d])?;
        // [V × d] → the head's [d × V]
        let head = if w.has("lm_head.weight") {
            w.get("lm_head.weight", &[v, d])?
        } else if a.tied {
            net.tok.clone()
        } else {
            return Err("the checkpoint has no lm_head.weight and tie_word_embeddings is false".into());
        };
        net.gen_w = transpose(&head, v, d);
        drop(head);
        net.gen_b = vec![0.0; v];
        net.nf = w.get("model.norm.weight", &[d])?;
        for (l, b) in net.blocks.iter_mut().enumerate() {
            let p = |n: &str| format!("model.layers.{l}.{n}");
            b.n1 = w.get(&p("input_layernorm.weight"), &[d])?;
            b.n2 = w.get(&p("post_attention_layernorm.weight"), &[d])?;
            // q, k, v [out × d] side by side as columns of [d × (q + 2·kv)]
            let wq = w.get(&p("self_attn.q_proj.weight"), &[qa, d])?;
            let wk = w.get(&p("self_attn.k_proj.weight"), &[kva, d])?;
            let wv = w.get(&p("self_attn.v_proj.weight"), &[kva, d])?;
            let mut rows = wq;
            rows.extend(wk);
            rows.extend(wv);
            b.wqkv = transpose(&rows, qa + 2 * kva, d);
            drop(rows);
            if qkv_bias {
                let mut bias = w.get(&p("self_attn.q_proj.bias"), &[qa])?;
                bias.extend(w.get(&p("self_attn.k_proj.bias"), &[kva])?);
                bias.extend(w.get(&p("self_attn.v_proj.bias"), &[kva])?);
                b.bqkv = bias;
            }
            if qk_norm {
                b.qn = w.get(&p("self_attn.q_norm.weight"), &[dh])?;
                b.kn = w.get(&p("self_attn.k_norm.weight"), &[dh])?;
            }
            b.wo = transpose(&w.get(&p("self_attn.o_proj.weight"), &[d, qa])?, d, qa);
            b.wg = transpose(&w.get(&p("mlp.gate_proj.weight"), &[a.ff, d])?, a.ff, d);
            b.wu = transpose(&w.get(&p("mlp.up_proj.weight"), &[a.ff, d])?, a.ff, d);
            b.wd = transpose(&w.get(&p("mlp.down_proj.weight"), &[d, a.ff])?, d, a.ff);
        }
        let steer = Steer::neutral(&net.heads);
        let name = self
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| a.model_type.clone());
        Ok(Classifier {
            meta: Meta {
                name,
                seed: o.seed,
                class_counts: net.heads.iter().map(|h| vec![0.0; h.width()]).collect(),
                log: vec![format!(
                    "imported pretrained {} from {}",
                    a.model_type,
                    self.dir.display()
                )],
                ..Meta::default()
            },
            net,
            steer,
            opt: None,
        })
    }
}

/// a pretrained model folder as an engine model
pub fn load(dir: &Path, o: &Options) -> Result<Classifier, String> {
    Folder::open(dir)?.model(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::bpe::tests::{byte_level_json, sentencepiece_json};
    use crate::classify::generate::{self, Constraints, Critic, GenOpts, Sampling, Want};
    use crate::classify::math::Rng;
    use crate::classify::model::{Decoder, Mode};
    use crate::classify::safetensors::{Dtype, write};
    use crate::classify::train::{self, Example, TrainOpts};
    use crate::classify::{Flow, HeadSpec};
    use std::collections::BTreeMap;

    /// a synthetic checkpoint's shapes (tests only: tiny so the suite stays fast; the
    /// loader itself has no size limits)
    #[derive(Clone)]
    struct Shape {
        model_type: &'static str,
        d: usize,
        layers: usize,
        heads: usize,
        kv: usize,
        dh: usize,
        ff: usize,
        vocab: usize,
        tied: bool,
        bias: bool,
        qk_norm: bool,
        dtype: Dtype,
        shards: bool,
        sentencepiece: bool,
        extra: serde_json::Value,
    }

    impl Shape {
        fn llama() -> Shape {
            Shape {
                model_type: "llama",
                d: 24,
                layers: 2,
                heads: 4,
                kv: 2,
                dh: 8,
                ff: 40,
                vocab: 288,
                tied: true,
                bias: false,
                qk_norm: false,
                dtype: Dtype::F32,
                shards: false,
                sentencepiece: false,
                extra: serde_json::json!({}),
            }
        }
    }

    type Tensors = BTreeMap<String, (Vec<usize>, Vec<f32>)>;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("anend-pre-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// writes config.json, tokenizer.json and random weights; returns the weights
    fn synth(dir: &Path, s: &Shape, seed: u64) -> Tensors {
        let mut r = Rng::new(seed);
        let mut t: Tensors = BTreeMap::new();
        let mut put = |name: String, shape: Vec<usize>, scale: f32, base: f32| {
            let n = shape.iter().product();
            let v: Vec<f32> = (0..n).map(|_| base + r.symmetric(scale)).collect();
            t.insert(name, (shape, v));
        };
        let (d, qa, kva) = (s.d, s.heads * s.dh, s.kv * s.dh);
        put("model.embed_tokens.weight".into(), vec![s.vocab, d], 1.0, 0.0);
        if !s.tied {
            put("lm_head.weight".into(), vec![s.vocab, d], 0.3, 0.0);
        }
        put("model.norm.weight".into(), vec![d], 0.2, 1.0);
        for l in 0..s.layers {
            let p = |n: &str| format!("model.layers.{l}.{n}");
            put(p("input_layernorm.weight"), vec![d], 0.2, 1.0);
            put(p("post_attention_layernorm.weight"), vec![d], 0.2, 1.0);
            put(p("self_attn.q_proj.weight"), vec![qa, d], 0.4, 0.0);
            put(p("self_attn.k_proj.weight"), vec![kva, d], 0.4, 0.0);
            put(p("self_attn.v_proj.weight"), vec![kva, d], 0.4, 0.0);
            put(p("self_attn.o_proj.weight"), vec![d, qa], 0.3, 0.0);
            if s.bias {
                put(p("self_attn.q_proj.bias"), vec![qa], 0.3, 0.0);
                put(p("self_attn.k_proj.bias"), vec![kva], 0.3, 0.0);
                put(p("self_attn.v_proj.bias"), vec![kva], 0.3, 0.0);
            }
            if s.qk_norm {
                put(p("self_attn.q_norm.weight"), vec![s.dh], 0.2, 1.0);
                put(p("self_attn.k_norm.weight"), vec![s.dh], 0.2, 1.0);
            }
            put(p("mlp.gate_proj.weight"), vec![s.ff, d], 0.3, 0.0);
            put(p("mlp.up_proj.weight"), vec![s.ff, d], 0.3, 0.0);
            put(p("mlp.down_proj.weight"), vec![d, s.ff], 0.3, 0.0);
        }
        // what the reader will see after the dtype round trip
        for (_, v) in t.values_mut() {
            *v = crate::classify::safetensors::to_f32(
                s.dtype,
                &match s.dtype {
                    Dtype::F32 => v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>(),
                    Dtype::BF16 => v
                        .iter()
                        .flat_map(|x| crate::classify::safetensors::f32_to_bf16(*x).to_le_bytes())
                        .collect(),
                    Dtype::F16 => v
                        .iter()
                        .flat_map(|x| crate::classify::safetensors::f32_to_f16(*x).to_le_bytes())
                        .collect(),
                    Dtype::F64 => unreachable!(),
                },
            );
        }
        let list: Vec<(&str, Dtype, &[usize], &[f32])> = t
            .iter()
            .map(|(n, (sh, v))| (n.as_str(), s.dtype, &sh[..], &v[..]))
            .collect();
        if s.shards {
            let (a, b) = list.split_at(list.len() / 2);
            write(&dir.join("model-00001-of-00002.safetensors"), a).unwrap();
            write(&dir.join("model-00002-of-00002.safetensors"), b).unwrap();
            let mut map = serde_json::Map::new();
            for (n, ..) in a {
                map.insert(n.to_string(), "model-00001-of-00002.safetensors".into());
            }
            for (n, ..) in b {
                map.insert(n.to_string(), "model-00002-of-00002.safetensors".into());
            }
            std::fs::write(
                dir.join("model.safetensors.index.json"),
                serde_json::json!({"metadata": {}, "weight_map": map}).to_string(),
            )
            .unwrap();
        } else {
            write(&dir.join("model.safetensors"), &list).unwrap();
        }
        let tok = if s.sentencepiece {
            sentencepiece_json()
        } else {
            byte_level_json()
        };
        let tk = Tokenizer::from_json(&tok).unwrap();
        let (bos, eos) = if s.sentencepiece {
            (Some(1), 2)
        } else {
            (None, tk.id("<|endoftext|>").unwrap())
        };
        std::fs::write(dir.join("tokenizer.json"), tok).unwrap();
        let mut cfg = serde_json::json!({
            "architectures": ["X"], "model_type": s.model_type, "hidden_size": d,
            "intermediate_size": s.ff, "num_hidden_layers": s.layers,
            "num_attention_heads": s.heads, "num_key_value_heads": s.kv, "head_dim": s.dh,
            "max_position_embeddings": 64, "rms_norm_eps": 1e-5, "rope_theta": 10000.0,
            "tie_word_embeddings": s.tied, "vocab_size": s.vocab, "hidden_act": "silu",
            "eos_token_id": eos, "torch_dtype": "bfloat16"
        });
        if let Some(b) = bos {
            cfg["bos_token_id"] = b.into();
        }
        for (k, v) in s.extra.as_object().unwrap() {
            cfg[k] = v.clone();
        }
        std::fs::write(dir.join("config.json"), cfg.to_string()).unwrap();
        t
    }

    /// the reference computation, written straight from the transformers code (f64,
    /// weights as [out × in]); logits at every position of `ids`
    fn reference(t: &Tensors, s: &Shape, theta: f64, ids: &[u32]) -> Vec<Vec<f64>> {
        let g = |n: &str| -> Vec<f64> { t[n].1.iter().map(|&x| x as f64).collect() };
        let lin = |w: &[f64], x: &[f64], out: usize| -> Vec<f64> {
            let i = x.len();
            (0..out).map(|o| (0..i).map(|j| w[o * i + j] * x[j]).sum()).collect()
        };
        let norm = |x: &[f64], w: &[f64]| -> Vec<f64> {
            let ms = x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64;
            let r = 1.0 / (ms + 1e-5).sqrt();
            x.iter().zip(w).map(|(a, b)| a * r * b).collect()
        };
        let rope = |x: &mut [f64], pos: usize| {
            let h = s.dh / 2;
            for i in 0..h {
                let f = theta.powf(-((2 * i) as f64) / s.dh as f64);
                let (sn, c) = (pos as f64 * f).sin_cos();
                let (a, b) = (x[i], x[i + h]);
                x[i] = a * c - b * sn;
                x[i + h] = b * c + a * sn;
            }
        };
        let (d, dh) = (s.d, s.dh);
        let emb = g("model.embed_tokens.weight");
        let mut xs: Vec<Vec<f64>> = ids
            .iter()
            .map(|&i| emb[i as usize * d..(i as usize + 1) * d].to_vec())
            .collect();
        for l in 0..s.layers {
            let p = |n: &str| format!("model.layers.{l}.{n}");
            let (wq, wk, wv, wo) = (
                g(&p("self_attn.q_proj.weight")),
                g(&p("self_attn.k_proj.weight")),
                g(&p("self_attn.v_proj.weight")),
                g(&p("self_attn.o_proj.weight")),
            );
            let mut qs = Vec::new();
            let mut ks = Vec::new();
            let mut vs = Vec::new();
            for (pos, x) in xs.iter().enumerate() {
                let h = norm(x, &g(&p("input_layernorm.weight")));
                let mut q = lin(&wq, &h, s.heads * dh);
                let mut k = lin(&wk, &h, s.kv * dh);
                let mut v = lin(&wv, &h, s.kv * dh);
                if s.bias {
                    for (a, b) in q.iter_mut().zip(g(&p("self_attn.q_proj.bias"))) {
                        *a += b;
                    }
                    for (a, b) in k.iter_mut().zip(g(&p("self_attn.k_proj.bias"))) {
                        *a += b;
                    }
                    for (a, b) in v.iter_mut().zip(g(&p("self_attn.v_proj.bias"))) {
                        *a += b;
                    }
                }
                for (hh, chunk) in q.chunks_mut(dh).enumerate() {
                    if s.qk_norm {
                        let n = norm(chunk, &g(&p("self_attn.q_norm.weight")));
                        chunk.copy_from_slice(&n);
                    }
                    rope(chunk, pos);
                    let _ = hh;
                }
                for chunk in k.chunks_mut(dh) {
                    if s.qk_norm {
                        let n = norm(chunk, &g(&p("self_attn.k_norm.weight")));
                        chunk.copy_from_slice(&n);
                    }
                    rope(chunk, pos);
                }
                qs.push(q);
                ks.push(k);
                vs.push(v);
            }
            let per = s.heads / s.kv;
            for t in 0..xs.len() {
                let mut ctx = vec![0f64; s.heads * dh];
                for h in 0..s.heads {
                    let g_ = h / per;
                    let q = &qs[t][h * dh..(h + 1) * dh];
                    let sc: Vec<f64> = (0..=t)
                        .map(|u| {
                            q.iter()
                                .zip(&ks[u][g_ * dh..(g_ + 1) * dh])
                                .map(|(a, b)| a * b)
                                .sum::<f64>()
                                / (dh as f64).sqrt()
                        })
                        .collect();
                    let m = sc.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    let e: Vec<f64> = sc.iter().map(|v| (v - m).exp()).collect();
                    let z: f64 = e.iter().sum();
                    for (u, eu) in e.iter().enumerate() {
                        for i in 0..dh {
                            ctx[h * dh + i] += eu / z * vs[u][g_ * dh + i];
                        }
                    }
                }
                let o = lin(&wo, &ctx, d);
                let x = &mut xs[t];
                x.iter_mut().zip(&o).for_each(|(a, b)| *a += b);
                let h2 = norm(x, &g(&p("post_attention_layernorm.weight")));
                let gt = lin(&g(&p("mlp.gate_proj.weight")), &h2, s.ff);
                let up = lin(&g(&p("mlp.up_proj.weight")), &h2, s.ff);
                let act: Vec<f64> = gt.iter().zip(&up).map(|(a, b)| a / (1.0 + (-a).exp()) * b).collect();
                let dn = lin(&g(&p("mlp.down_proj.weight")), &act, d);
                x.iter_mut().zip(&dn).for_each(|(a, b)| *a += b);
            }
        }
        let head = if s.tied {
            g("model.embed_tokens.weight")
        } else {
            g("lm_head.weight")
        };
        xs.iter()
            .map(|x| lin(&head, &norm(x, &g("model.norm.weight")), s.vocab))
            .collect()
    }

    fn compare(s: &Shape, name: &str) -> Classifier {
        let dd = dir(name);
        let t = synth(&dd, s, 7);
        let clf = load(&dd, &Options::default()).unwrap();
        let net = &clf.net;
        let ids: Vec<u32> = std::iter::once(net.start_token())
            .chain(net.lexicon.as_ref().unwrap().tokenizer.encode("hello world, the 12"))
            .collect();
        let want = reference(&t, s, 10000.0, &ids);
        let w = net.window_tokens(&ids[1..], false);
        assert_eq!(w.tokens, ids);
        let tr = net.forward(&w, &[], &net.cfg.gains, Mode::EVAL, None);
        let got = net.gen_logits(&tr);
        let v = s.vocab;
        for (p, row) in want.iter().enumerate() {
            for (i, &x) in row.iter().enumerate() {
                let y = got[p * v + i] as f64;
                assert!(
                    (x - y).abs() < 2e-3 * (1.0 + x.abs()),
                    "{name} position {p} token {i}: reference {x} engine {y}"
                );
            }
        }
        // the cached decoder gives the same bits as the full pass
        let mut dec = Decoder::from_tokens(net, net.cfg.gains, &[]);
        assert_eq!(dec.logits(), &got[..v]);
        for (p, &id) in ids[1..].iter().enumerate() {
            dec.push_token(id);
            assert_eq!(
                dec.logits(),
                &got[(p + 1) * v..(p + 2) * v],
                "{name} decoder at {}",
                p + 1
            );
        }
        clf
    }

    #[test]
    fn llama_family_checkpoints_match_the_reference() {
        compare(&Shape::llama(), "llama");
        // Qwen2: q/k/v biases, separate output embedding, bfloat16, two shards
        compare(
            &Shape {
                model_type: "qwen2",
                bias: true,
                tied: false,
                dtype: Dtype::BF16,
                shards: true,
                ..Shape::llama()
            },
            "qwen2",
        );
        // Qwen3: QK-norm, head_dim apart from hidden/heads, half floats
        compare(
            &Shape {
                model_type: "qwen3",
                qk_norm: true,
                dh: 10,
                heads: 6,
                kv: 3,
                dtype: Dtype::F16,
                ..Shape::llama()
            },
            "qwen3",
        );
        // Mistral / TinyLlama shaped: SentencePiece tokenizer with BOS, plain heads
        compare(
            &Shape {
                model_type: "mistral",
                heads: 4,
                kv: 4,
                tied: false,
                sentencepiece: true,
                ..Shape::llama()
            },
            "mistral",
        );
    }

    #[test]
    fn rope_scaling_and_errors_in_words() {
        let ok = |extra: serde_json::Value| {
            let mut c = serde_json::json!({"model_type": "llama", "hidden_size": 16, "intermediate_size": 8,
                "num_hidden_layers": 1, "num_attention_heads": 2, "vocab_size": 300, "eos_token_id": [1, 2]});
            for (k, v) in extra.as_object().unwrap() {
                c[k] = v.clone();
            }
            Arch::from_json(&c.to_string(), Some(r#"{"eos_token_id": [2, 3]}"#))
        };
        let a = ok(serde_json::json!({})).unwrap();
        assert_eq!((a.kv_heads, a.head_dim, a.eos.clone()), (2, 8, vec![1, 2, 3]));
        let a = ok(
            serde_json::json!({"rope_scaling": {"rope_type": "llama3", "factor": 8.0, "low_freq_factor": 1.0,
            "high_freq_factor": 4.0, "original_max_position_embeddings": 8192}}),
        )
        .unwrap();
        assert!(matches!(a.scaling, Some(RopeScaling::Llama3 { .. })));
        let cfg = a.config(&Options::default(), false, false).unwrap();
        let plain = crate::classify::model::Rope::new(cfg.max_len, 8, cfg.rope_base);
        assert_ne!(crate::classify::model::Rope::of(&cfg), plain);
        for (extra, says) in [
            (serde_json::json!({"model_type": "gemma"}), "model_type \"gemma\""),
            (serde_json::json!({"hidden_act": "gelu"}), "hidden_act \"gelu\""),
            (
                serde_json::json!({"rope_scaling": {"type": "yarn", "factor": 4.0}}),
                "rope_scaling type \"yarn\"",
            ),
            (serde_json::json!({"attention_bias": true}), "attention_bias"),
            (serde_json::json!({"num_key_value_heads": 3}), "num_key_value_heads 3"),
            (serde_json::json!({"hidden_size": null}), "hidden_size"),
        ] {
            let e = ok(extra.clone()).unwrap_err();
            assert!(e.contains(says), "{extra}: {e}");
        }
        // a missing or misshapen tensor names itself
        let dd = dir("broken");
        let mut t = synth(&dd, &Shape::llama(), 1);
        t.remove("model.layers.1.mlp.up_proj.weight");
        let list: Vec<(&str, Dtype, &[usize], &[f32])> = t
            .iter()
            .map(|(n, (sh, v))| (n.as_str(), Dtype::F32, &sh[..], &v[..]))
            .collect();
        write(&dd.join("model.safetensors"), &list).unwrap();
        let e = load(&dd, &Options::default()).unwrap_err();
        assert!(e.contains("model.layers.1.mlp.up_proj.weight"), "{e}");
        let mut c: Json = serde_json::from_str(&std::fs::read_to_string(dd.join("config.json")).unwrap()).unwrap();
        c["intermediate_size"] = 41.into();
        std::fs::write(dd.join("config.json"), c.to_string()).unwrap();
        let e = load(&dd, &Options::default()).unwrap_err();
        assert!(e.contains("mlp.gate_proj.weight") && e.contains("[40, 24]"), "{e}");
    }

    /// generation with critics, fine-tuning, saving: the engine's paths on a pretrained model
    #[test]
    fn pretrained_models_generate_tune_and_save() {
        let dd = dir("paths");
        synth(&dd, &Shape::llama(), 3);
        let o = Options {
            heads: vec![HeadSpec::single("kind", &["a", "b"])],
            max_len: Some(24),
            ..Options::default()
        };
        let mut g = load(&dd, &o).unwrap();
        assert_eq!(g.net.head_names(), ["kind", "next"]);
        // a byte-model critic judges what the pretrained model writes
        let critic = Classifier::new(
            crate::classify::Config::tiny().gematria(false),
            &[HeadSpec::single("d", &["no", "yes"])],
            2,
        )
        .unwrap();
        let critics = [Critic::new(&critic, 0, Want::Class(1), 1.0)];
        let opts = GenOpts {
            prompt: b"hello".to_vec(),
            sampling: Sampling {
                temperature: 1.0,
                max_len: 16,
                seed: 5,
                ..Sampling::default()
            },
            candidates: 4,
            threads: 2,
            guidance: Some(generate::Guidance::default()),
            ..GenOpts::default()
        };
        let a = generate::draw(&g, &critics, &Constraints::default(), &opts).unwrap();
        let b = generate::draw(
            &g,
            &critics,
            &Constraints::default(),
            &GenOpts {
                threads: 1,
                ..opts.clone()
            },
        )
        .unwrap();
        assert_eq!(a, b);
        assert!(a.iter().all(|c| c.bytes.len() <= 16 && c.scores.len() == 1));
        // fine-tuning on a text makes it likelier; the classifier head trains alongside
        let text = "the world, hello the world";
        let before = train::nll(&g, text.as_bytes(), 0);
        assert!(before.1 > 3, "{before:?}");
        let data: Vec<Example> = (0..6)
            .map(|i| {
                if i % 2 == 0 {
                    Example::generate(text)
                } else {
                    Example::class("hello", 1)
                }
            })
            .collect();
        let to = TrainOpts {
            epochs: 4,
            batch: 3,
            lr: 3e-3,
            val_fraction: 0.0,
            threads: 2,
            ..TrainOpts::default()
        };
        train::train(&mut g, &data, &to, |_| Flow::Go).unwrap();
        let after = train::nll(&g, text.as_bytes(), 0);
        assert!(after.0 < before.0, "{before:?} → {after:?}");
        // the file keeps the vocabulary, the tokenizer and every weight
        let back = Classifier::from_bytes(&g.to_bytes()).unwrap();
        assert_eq!(back.net, g.net);
        assert_eq!(train::nll(&back, text.as_bytes(), 0), after);
        let (s1, _) = generate::sample(&g, b"hello", &Sampling::greedy(8), 0);
        assert_eq!(generate::sample(&back, b"hello", &Sampling::greedy(8), 0).0, s1);
    }

    /// the declarative spec: `"from": {"pretrained": dir}`, trained, run, generating
    /// with a critic model; checks name what a pretrained spec can't set
    #[test]
    fn a_pretrained_model_from_a_spec() {
        use crate::classify::spec::{self, MemSource, Model, ModelSpec, Value};
        let dd = dir("spec");
        synth(&dd, &Shape::llama(), 4);
        let json = r#"{
          "name": "writer",
          "from": {"pretrained": "DIR"},
          "input": {"type": "text"},
          "trunk": {"max_len": 32},
          "heads": [{"name": "next", "kind": "generate"},
                    {"name": "tone", "kind": "classify", "classes": ["calm", "loud"]}],
          "data": [{"from": "list", "list": "texts", "head": "next"},
                   {"from": "list", "list": "loud", "head": "tone", "class": "loud"},
                   {"from": "list", "list": "calm", "head": "tone", "class": "calm"}],
          "critics": [{"model": "judge", "head": "kind", "want": {"class": "yes"}}],
          "sampling": {"temperature": 1.0, "max_len": 12, "seed": 2},
          "train": {"epochs": 2, "batch": 2, "lr": 0.002, "val_fraction": 0.0, "threads": 2}
        }"#
        .replace("DIR", &dd.display().to_string().replace('\\', "/"));
        let sp = ModelSpec::from_json(&json).unwrap();
        let src = MemSource::default()
            .with_texts("texts", &["hello the world", "the world, hello"])
            .with_texts("loud", &["HELLO!", "WORLD!"])
            .with_texts("calm", &["hello", "world"]);
        let (m, rep) = spec::train(None, &sp, &src, None, |_| Flow::Go).unwrap();
        assert!(rep.steps > 0);
        let a = m.run(&Value::Text("hello".into())).unwrap();
        assert!(a.contains_key("tone") && a.contains_key("next"), "{a:?}");
        let judge = ModelSpec::from_json(
            r#"{"name": "judge", "input": {"type": "text"}, "trunk": {"preset": "test"},
                "heads": [{"name": "kind", "kind": "classify", "classes": ["no", "yes"]}]}"#,
        )
        .unwrap();
        let jm = spec::build(&judge, None).unwrap();
        let models: BTreeMap<String, &Model> = [("judge".to_string(), &jm)].into();
        let c = spec::generate(&m, &models, &Value::Text("the".into()), 3, None).unwrap();
        assert!(!c.is_empty() && c.iter().all(|c| c.bytes.len() <= 12));
        // saved and read back: the weights travel in the file, not the folder
        let back = Model::from_bytes(&m.to_bytes()).unwrap();
        assert_eq!(back.run(&Value::Text("hello".into())).unwrap(), a);
        for (field, says) in [
            (r#""trunk": {"preset": "small"}"#, "sizes come from its checkpoint"),
            (r#""input": {"type": "text", "features": ["letters"]}"#, "plain text"),
        ] {
            let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
            let f: serde_json::Value = serde_json::from_str(&format!("{{{field}}}")).unwrap();
            for (k, x) in f.as_object().unwrap() {
                v[k] = x.clone();
            }
            let e = ModelSpec::from_json(&v.to_string()).unwrap_err();
            assert!(e.contains(says), "{e}");
        }
    }
}
