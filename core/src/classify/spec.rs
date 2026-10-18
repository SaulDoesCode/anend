//! The declarative model spec: the primary way to make a model. A `ModelSpec` is plain
//! data (JSON here; later the scripting language and the UI edit the same thing) that
//! says what goes in, how big the trunk is, which outputs come out, what it learns from
//! (queries over the save, through a `Source`), and, for generators, which other models
//! judge what it writes. `build` makes the model, `train` resolves the data and trains
//! every head together, `run` answers with named outputs, `generate` draws candidates
//! judged by the spec's critics. An existing classifier is one instance of a spec
//! (`Model::from_classifier`). The format is documented in `docs/models.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::bench::{self, MachineProfile, Task};
use super::codec::{Codebook, Image, NumberCodec};
use super::device::Device;
use super::generate::{self, Candidate, Constraints, Critic, GenOpts, Judge, Sampling, Want};
use super::model::{Config, GenSpec, HeadKind, HeadSpec, Pooling};
use super::steer::Output;
use super::train::{self, Example, Target, TrainOpts, TrainReport};
use super::{Classifier, Flow, Progress};

/// the newest spec version this build reads
pub const SPEC_VERSION: u32 = 1;
/// separates the input from the output of a pair in a sequence-to-sequence generator
pub const PAIR: u8 = 0x1e;

fn v1() -> u32 {
    1
}

fn one() -> f32 {
    1.0
}

/// an optional input feature channel (all off unless listed)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    /// a token's letter value, a = 1 .. z = 26
    Letters,
    /// the ordinal, reversed and Atbash sums of the word a token is in, and digit roots
    WordSums,
    /// whole-text measures (letter frequencies, sums, shape) at the heads
    Side,
}

/// what a model reads
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputSpec {
    /// UTF-8 text, as bytes, with any feature channels
    Text {
        #[serde(default)]
        features: Vec<Feature>,
    },
    /// any bytes
    Bytes,
    /// pictures cut into `patch`-pixel squares, each one of `codes` learned patch
    /// prototypes (one token per patch)
    Image {
        width: usize,
        height: usize,
        #[serde(default = "patch8")]
        patch: usize,
        #[serde(default = "codes")]
        codes: usize,
    },
    /// a vector of named numbers, written as fixed-width text for the trunk and also
    /// given to the heads as they are
    Numbers { names: Vec<String>, codec: NumberCodec },
}

fn patch8() -> usize {
    8
}

fn codes() -> usize {
    128
}

/// the trunk; anything left out comes from the preset (or, with `preset: "auto"` and a
/// machine profile, from the benchmark)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrunkSpec {
    /// "test" (tests and CI only), "compact", "small", "base", or "auto"
    pub preset: Option<String>,
    pub d_model: Option<usize>,
    pub layers: Option<usize>,
    pub heads: Option<usize>,
    pub head_dim: Option<usize>,
    pub d_ff: Option<usize>,
    /// tokens per window, CLS and SEP included
    pub max_len: Option<usize>,
    /// default: on when there is a generate head
    pub causal: Option<bool>,
    pub pooling: Option<Pooling>,
    pub dropout: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutKind {
    /// one class of several (softmax)
    Classify,
    /// any of several labels (a sigmoid each)
    Multi,
    /// numbers (squared error)
    Regress,
    /// the next byte (writes bytes, text, pictures, numbers)
    Generate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadDecl {
    pub name: String,
    pub kind: OutKind,
    /// class, label or value names (none for generate)
    #[serde(default)]
    pub classes: Vec<String>,
    /// how much this head's loss counts when heads train together
    #[serde(default = "one")]
    pub weight: f32,
}

/// a value of the model's input type
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Value {
    Text(String),
    Bytes(Vec<u8>),
    Image(Image),
    Numbers(Vec<f32>),
}

/// where training data comes from: queries over a `Source` (the save), or inline
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum DataQuery {
    /// the members of list `list` as class `class` of head `head` (for a generate
    /// head: texts to learn to write; for a regress head: `value`)
    List {
        list: String,
        head: String,
        #[serde(default)]
        class: Option<String>,
        #[serde(default)]
        value: Option<Vec<f32>>,
    },
    /// entries tagged `tag`, likewise
    Tagged {
        tag: String,
        head: String,
        #[serde(default)]
        class: Option<String>,
        #[serde(default)]
        value: Option<Vec<f32>>,
    },
    /// pairs (input from list `input`, output from list `output`) for a generate head:
    /// it learns to write the output after the input
    Pairs {
        input: String,
        output: String,
        head: String,
    },
    /// examples written into the spec
    Inline { items: Vec<InlineItem> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InlineItem {
    pub input: Value,
    pub head: String,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub value: Option<Vec<f32>>,
}

/// what a critic wants of its head
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WantDecl {
    Class(String),
    Not(String),
    Near {
        value: String,
        target: f32,
        scale: f32,
    },
    Range {
        value: String,
        lo: f32,
        hi: f32,
        scale: f32,
    },
}

/// another model judging what a generator writes
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CriticSpec {
    /// the model's name, resolved when generating
    pub model: String,
    pub head: String,
    pub want: WantDecl,
    #[serde(default = "one")]
    pub weight: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrainSpec {
    pub epochs: Option<usize>,
    pub batch: Option<usize>,
    pub lr: Option<f32>,
    pub val_fraction: Option<f32>,
    pub patience: Option<usize>,
    pub max_steps: Option<usize>,
    pub threads: Option<usize>,
    /// seconds of training to size an "auto" trunk for
    pub budget_seconds: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelSpec {
    #[serde(default = "v1")]
    pub version: u32,
    pub name: String,
    pub input: InputSpec,
    #[serde(default)]
    pub trunk: TrunkSpec,
    pub heads: Vec<HeadDecl>,
    #[serde(default)]
    pub data: Vec<DataQuery>,
    #[serde(default)]
    pub critics: Vec<CriticSpec>,
    #[serde(default)]
    pub constraints: Constraints,
    #[serde(default)]
    pub sampling: Sampling,
    #[serde(default)]
    pub train: TrainSpec,
    #[serde(default)]
    pub seed: u64,
    /// where the trunk comes from: a pretrained model folder instead of new weights
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Origin>,
    /// where training and generation run: "cpu" (the default and the reference),
    /// "gpu" (exact precision) or "gpu_fast"; a GPU needs the `gpu` build feature
    #[serde(default, skip_serializing_if = "is_cpu")]
    pub device: Device,
}

fn is_cpu(d: &Device) -> bool {
    *d == Device::Cpu
}

/// a model's starting point other than fresh weights
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// a HuggingFace model folder (config.json, tokenizer.json, safetensors), see
    /// `pretrained`: its trunk, vocabulary and language-model head (the spec's generate
    /// head); the spec's classify/multi/regress heads go on its trunk, new
    Pretrained(String),
}

/// the data behind queries: the save (the entry store implements this), or memory
pub trait Source {
    fn list(&self, name: &str) -> Result<Vec<Value>, String>;
    fn tagged(&self, tag: &str) -> Result<Vec<Value>, String>;
    /// by default the two lists side by side, in order
    fn pairs(&self, input: &str, output: &str) -> Result<Vec<(Value, Value)>, String> {
        Ok(self.list(input)?.into_iter().zip(self.list(output)?).collect())
    }
}

/// lists and tags in memory
#[derive(Clone, Debug, Default)]
pub struct MemSource {
    pub lists: BTreeMap<String, Vec<Value>>,
    pub tags: BTreeMap<String, Vec<Value>>,
}

impl MemSource {
    pub fn with_list(mut self, name: &str, items: Vec<Value>) -> MemSource {
        self.lists.insert(name.into(), items);
        self
    }

    pub fn with_texts(self, name: &str, texts: &[&str]) -> MemSource {
        self.with_list(name, texts.iter().map(|t| Value::Text(t.to_string())).collect())
    }
}

impl Source for MemSource {
    fn list(&self, name: &str) -> Result<Vec<Value>, String> {
        self.lists.get(name).cloned().ok_or(format!("no list {name:?}"))
    }

    fn tagged(&self, tag: &str) -> Result<Vec<Value>, String> {
        Ok(self.tags.get(tag).cloned().unwrap_or_default())
    }
}

/// one resolved data item: the value, its head, its class or values
type Item = (Value, String, Option<String>, Option<Vec<f32>>);

/// a head by name
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Pooled(usize),
    Gen,
}

impl ModelSpec {
    /// parses JSON and checks it
    pub fn from_json(s: &str) -> Result<ModelSpec, String> {
        let spec: ModelSpec = serde_json::from_str(s).map_err(|e| format!("model spec: {e}"))?;
        spec.check()?;
        Ok(spec)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("specs serialise")
    }

    pub fn slot(&self, head: &str) -> Result<Slot, String> {
        let mut pooled = 0;
        for h in &self.heads {
            if h.name == head {
                return Ok(if h.kind == OutKind::Generate {
                    Slot::Gen
                } else {
                    Slot::Pooled(pooled)
                });
            }
            if h.kind != OutKind::Generate {
                pooled += 1;
            }
        }
        Err(format!("{}: no head {head:?}", self.name))
    }

    fn decl(&self, head: &str) -> Result<&HeadDecl, String> {
        self.heads
            .iter()
            .find(|h| h.name == head)
            .ok_or(format!("{}: no head {head:?}", self.name))
    }

    fn generates(&self) -> bool {
        self.heads.iter().any(|h| h.kind == OutKind::Generate)
    }

    /// every rule a spec must keep, with the first broken one in words
    pub fn check(&self) -> Result<(), String> {
        let at = |m: String| Err(format!("model spec {:?}: {m}", self.name));
        if self.version == 0 || self.version > SPEC_VERSION {
            return at(format!(
                "version {} (this build reads 1 to {SPEC_VERSION})",
                self.version
            ));
        }
        if self.name.trim().is_empty() {
            return at("a model needs a name".into());
        }
        if self.heads.is_empty() {
            return at("a model needs at least one head".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for h in &self.heads {
            if !names.insert(&h.name) {
                return at(format!("two heads are called {:?}", h.name));
            }
            if !(h.weight >= 0.0 && h.weight.is_finite()) {
                return at(format!("head {:?}: weight must be finite and not negative", h.name));
            }
            match h.kind {
                OutKind::Generate if !h.classes.is_empty() => {
                    return at(format!("head {:?}: a generate head has no classes", h.name));
                }
                OutKind::Classify if h.classes.len() < 2 => {
                    return at(format!("head {:?}: classify needs at least two classes", h.name));
                }
                OutKind::Multi | OutKind::Regress if h.classes.is_empty() => {
                    return at(format!("head {:?}: needs at least one label or value name", h.name));
                }
                _ => {}
            }
        }
        if self.heads.iter().filter(|h| h.kind == OutKind::Generate).count() > 1 {
            return at("at most one generate head".into());
        }
        if self.generates() && self.trunk.causal == Some(false) {
            return at("a generate head needs a causal trunk".into());
        }
        if let InputSpec::Image {
            width,
            height,
            patch,
            codes,
        } = self.input
        {
            if patch == 0 || width == 0 || height == 0 || width % patch != 0 || height % patch != 0 {
                return at(format!(
                    "image {width}×{height} must be a positive multiple of the {patch} pixel patch"
                ));
            }
            if codes == 0 || codes > super::codec::MAX_CODES {
                return at(format!(
                    "an image codebook holds 1 to {} codes",
                    super::codec::MAX_CODES
                ));
            }
        }
        if let InputSpec::Numbers { names, .. } = &self.input
            && names.is_empty()
        {
            return at("a numbers input names at least one number".into());
        }
        for q in &self.data {
            let check_target = |head: &str, class: &Option<String>, value: &Option<Vec<f32>>| -> Result<(), String> {
                let h = self.decl(head)?;
                match h.kind {
                    OutKind::Classify | OutKind::Multi => match class {
                        Some(c) if h.classes.contains(c) => Ok(()),
                        Some(c) => Err(format!("head {head:?} has no class {c:?}")),
                        None => Err(format!("data for head {head:?} names a class")),
                    },
                    OutKind::Regress => match value {
                        Some(v) if v.len() == h.classes.len() => Ok(()),
                        _ => Err(format!("data for head {head:?} gives {} values", h.classes.len())),
                    },
                    OutKind::Generate => Ok(()),
                }
            };
            let r = match q {
                DataQuery::List { head, class, value, .. } | DataQuery::Tagged { head, class, value, .. } => {
                    check_target(head, class, value)
                }
                DataQuery::Pairs { head, .. } => match self.decl(head)?.kind {
                    OutKind::Generate => Ok(()),
                    _ => Err(format!("pairs train a generate head; {head:?} is not one")),
                },
                DataQuery::Inline { items } => items.iter().try_for_each(|i| check_target(&i.head, &i.class, &i.value)),
            };
            if let Err(e) = r {
                return at(e);
            }
        }
        if !self.critics.is_empty() && !self.generates() {
            return at("critics judge what a generate head writes; this model has none".into());
        }
        for c in &self.critics {
            if !(c.weight >= 0.0 && c.weight.is_finite()) {
                return at(format!("critic {:?}: weight must be finite and not negative", c.model));
            }
        }
        self.sampling.check().or_else(at)?;
        if let Some(Origin::Pretrained(dir)) = &self.from {
            if !matches!(self.input, InputSpec::Text { ref features } if features.is_empty())
                && self.input != InputSpec::Bytes
            {
                return at(format!(
                    "a pretrained model ({dir}) reads plain text, without feature channels"
                ));
            }
            if !self.generates() {
                return at("a pretrained model's language-model head is its generate head; declare one".into());
            }
            let t = &self.trunk;
            if t.preset.is_some()
                || t.d_model.is_some()
                || t.layers.is_some()
                || t.heads.is_some()
                || t.head_dim.is_some()
                || t.d_ff.is_some()
                || t.causal == Some(false)
                || t.pooling.is_some()
            {
                return at(
                    "a pretrained trunk's sizes come from its checkpoint (only max_len and dropout apply)".into(),
                );
            }
        }
        Ok(())
    }

    /// the engine configuration this spec asks for
    pub fn config(&self, profile: Option<&MachineProfile>, examples: usize) -> Result<Config, String> {
        let t = &self.trunk;
        let mut cfg = match t.preset.as_deref().unwrap_or("small") {
            "test" | "tiny" => Config::tiny(),
            "compact" => Config {
                d_model: 48,
                head_dim: 24,
                heads: 2,
                layers: 2,
                d_ff: 128,
                max_len: 96,
                stride: 47,
                dropout: 0.05,
                ..Config::tiny()
            },
            "small" => Config::small(),
            "base" => Config::base(),
            "auto" => match profile {
                Some(p) => bench::suggest(
                    p,
                    &Task {
                        examples: examples.max(1),
                        epochs: self.train.epochs.unwrap_or(20),
                        max_len: t.max_len.unwrap_or(self.natural_len()),
                        seconds: self.train.budget_seconds.unwrap_or(300.0),
                    },
                ),
                None => Config::small(),
            },
            other => {
                return Err(format!(
                    "unknown trunk preset {other:?} (test, compact, small, base, auto)"
                ));
            }
        };
        cfg = cfg.gematria(false);
        if let InputSpec::Text { features } = &self.input {
            cfg.letters = features.contains(&Feature::Letters);
            cfg.words = features.contains(&Feature::WordSums);
            cfg.side = features.contains(&Feature::Side);
        }
        if let Some(v) = t.d_model {
            cfg.d_model = v;
        }
        if let Some(v) = t.layers {
            cfg.layers = v;
        }
        if let Some(v) = t.heads {
            cfg.heads = v;
        }
        if let Some(v) = t.head_dim {
            cfg.head_dim = v;
        }
        if let Some(v) = t.d_ff {
            cfg.d_ff = v;
        }
        cfg.max_len = t.max_len.unwrap_or(match self.input {
            InputSpec::Image { .. } | InputSpec::Numbers { .. } => self.natural_len(),
            _ => cfg.max_len,
        });
        cfg.stride = (cfg.max_len.saturating_sub(2) / 2).max(1);
        cfg.causal = t.causal.unwrap_or(self.generates());
        cfg.pooling = t
            .pooling
            .unwrap_or(if cfg.causal { Pooling::Mean } else { cfg.pooling });
        if let Some(v) = t.dropout {
            cfg.dropout = v;
        }
        cfg.extra = match &self.input {
            InputSpec::Numbers { names, .. } => names.len(),
            _ => 0,
        };
        cfg.validate().map_err(|e| format!("model spec {:?}: {e}", self.name))?;
        Ok(cfg)
    }

    /// the window an input needs whole (pictures and number vectors), else 128
    fn natural_len(&self) -> usize {
        match &self.input {
            InputSpec::Image {
                width, height, patch, ..
            } => (width / patch + 1) * (height / patch) + 2,
            InputSpec::Numbers { names, codec } => names.len() * (codec.width() + 1) + 2,
            _ => 128,
        }
    }

    fn train_opts(&self) -> TrainOpts {
        let t = &self.train;
        let d = TrainOpts::default();
        TrainOpts {
            epochs: t.epochs.unwrap_or(d.epochs),
            batch: t.batch.unwrap_or(d.batch),
            lr: t.lr.unwrap_or(d.lr),
            val_fraction: t.val_fraction.unwrap_or(0.1),
            patience: t.patience.unwrap_or(d.patience),
            max_steps: t.max_steps.unwrap_or(0),
            threads: t.threads.unwrap_or(0),
            seed: self.seed,
            device: self.device,
            ..d
        }
    }
}

/// a model made from a spec: the engine's network, and what it learned besides weights
#[derive(Clone, Debug)]
pub struct Model {
    pub spec: ModelSpec,
    pub clf: Classifier,
    /// the picture codebook (image inputs; learned by the first `train`)
    pub codebook: Option<Codebook>,
}

/// one head's answer
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    /// class (or label) probabilities by name, the top one, the members (steered)
    Classes {
        probs: BTreeMap<String, f32>,
        top: String,
        members: Vec<String>,
    },
    Values(BTreeMap<String, f32>),
    /// what the generate head wrote after the input, with the spec's sampling
    Generated(Value),
}

#[derive(Serialize, Deserialize)]
struct Saved {
    spec: ModelSpec,
    codebook: Option<Codebook>,
}

/// a fresh model for a spec; `profile` sizes an "auto" trunk
pub fn build(spec: &ModelSpec, profile: Option<&MachineProfile>) -> Result<Model, String> {
    build_for(spec, profile, 0)
}

fn build_for(spec: &ModelSpec, profile: Option<&MachineProfile>, examples: usize) -> Result<Model, String> {
    spec.check()?;
    let heads: Vec<HeadSpec> = spec
        .heads
        .iter()
        .filter(|h| h.kind != OutKind::Generate)
        .map(|h| {
            let kind = match h.kind {
                OutKind::Multi => HeadKind::Multi,
                OutKind::Regress => HeadKind::Regress,
                _ => HeadKind::Single,
            };
            let c: Vec<&str> = h.classes.iter().map(String::as_str).collect();
            HeadSpec::new(&h.name, kind, &c).weighted(h.weight)
        })
        .collect();
    let generator = spec
        .heads
        .iter()
        .find(|h| h.kind == OutKind::Generate)
        .map(|h| GenSpec {
            name: h.name.clone(),
            weight: h.weight,
        });
    let mut clf = match &spec.from {
        Some(Origin::Pretrained(dir)) => {
            let o = super::pretrained::Options {
                max_len: spec.trunk.max_len,
                heads,
                generator: generator.map_or("next".into(), |g| g.name),
                dropout: spec.trunk.dropout.unwrap_or(0.0),
                seed: spec.seed,
            };
            super::pretrained::load(std::path::Path::new(dir), &o)
        }
        None => Classifier::with_gen(spec.config(profile, examples)?, &heads, generator, spec.seed),
    }
    .map_err(|e| format!("model spec {:?}: {e}", spec.name))?;
    clf.meta.name = spec.name.clone();
    Ok(Model {
        spec: spec.clone(),
        clf,
        codebook: None,
    })
}

impl Model {
    /// an existing classifier as an instance of a spec (sizes and features from its
    /// configuration, heads from its heads)
    pub fn from_classifier(clf: Classifier) -> Model {
        let c = &clf.net.cfg;
        let mut features = Vec::new();
        if c.letters {
            features.push(Feature::Letters);
        }
        if c.words {
            features.push(Feature::WordSums);
        }
        if c.side {
            features.push(Feature::Side);
        }
        let mut heads: Vec<HeadDecl> = clf
            .net
            .heads
            .iter()
            .map(|h| HeadDecl {
                name: h.spec.name.clone(),
                kind: match h.spec.kind {
                    HeadKind::Single => OutKind::Classify,
                    HeadKind::Multi => OutKind::Multi,
                    HeadKind::Regress => OutKind::Regress,
                },
                classes: h.spec.classes.clone(),
                weight: h.spec.weight,
            })
            .collect();
        if let Some(g) = &clf.net.generator {
            heads.push(HeadDecl {
                name: g.name.clone(),
                kind: OutKind::Generate,
                classes: Vec::new(),
                weight: g.weight,
            });
        }
        let spec = ModelSpec {
            version: SPEC_VERSION,
            name: if clf.meta.name.is_empty() {
                "classifier".into()
            } else {
                clf.meta.name.clone()
            },
            input: InputSpec::Text { features },
            trunk: TrunkSpec {
                preset: Some("small".into()),
                d_model: Some(c.d_model),
                layers: Some(clf.net.blocks.len()),
                heads: Some(c.heads),
                head_dim: Some(c.head_dim),
                d_ff: Some(c.d_ff),
                max_len: Some(c.max_len),
                causal: Some(c.causal),
                pooling: Some(c.pooling),
                dropout: Some(c.dropout),
            },
            heads,
            data: Vec::new(),
            critics: Vec::new(),
            constraints: Constraints::default(),
            sampling: Sampling::default(),
            train: TrainSpec::default(),
            seed: clf.meta.seed,
            from: None,
            device: Device::Cpu,
        };
        Model {
            spec,
            clf,
            codebook: None,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut clf = self.clf.clone();
        clf.meta.spec = Some(
            serde_json::to_value(Saved {
                spec: self.spec.clone(),
                codebook: self.codebook.clone(),
            })
            .expect("specs serialise"),
        );
        clf.to_bytes()
    }

    /// a model file, or any classifier file (as `from_classifier`)
    pub fn from_bytes(b: &[u8]) -> Result<Model, String> {
        let clf = Classifier::from_bytes(b)?;
        match clf.meta.spec.clone() {
            Some(v) => {
                let s: Saved = serde_json::from_value(v).map_err(|e| format!("model spec in file: {e}"))?;
                Ok(Model {
                    spec: s.spec,
                    clf,
                    codebook: s.codebook,
                })
            }
            None => Ok(Model::from_classifier(clf)),
        }
    }

    /// an input value as the bytes the trunk reads, and the heads' number inputs
    pub fn encode(&self, v: &Value) -> Result<(Vec<u8>, Vec<f32>), String> {
        match (&self.spec.input, v) {
            (InputSpec::Text { .. } | InputSpec::Bytes, Value::Text(t)) => Ok((t.as_bytes().to_vec(), vec![])),
            (InputSpec::Bytes | InputSpec::Text { .. }, Value::Bytes(b)) => Ok((b.clone(), vec![])),
            (InputSpec::Image { width, height, .. }, Value::Image(im)) => {
                if im.width != *width || im.height != *height {
                    return Err(format!(
                        "a {}×{} picture for a {width}×{height} model",
                        im.width, im.height
                    ));
                }
                let cb = self
                    .codebook
                    .as_ref()
                    .ok_or("the picture codebook is learned by training first")?;
                Ok((cb.encode(im), vec![]))
            }
            (InputSpec::Numbers { names, codec }, Value::Numbers(x)) => {
                if x.len() != names.len() {
                    return Err(format!("{} numbers for {} names", x.len(), names.len()));
                }
                let mut b = Vec::new();
                for (i, &n) in x.iter().enumerate() {
                    if i > 0 {
                        b.push(b' ');
                    }
                    b.extend(codec.encode(n as f64));
                }
                Ok((b, x.clone()))
            }
            (i, v) => Err(format!("a {} value for a {} input", kind_of(v), input_kind(i))),
        }
    }

    /// generated bytes back as a value of the input's type
    pub fn decode(&self, b: &[u8]) -> Value {
        match &self.spec.input {
            InputSpec::Text { .. } => Value::Text(String::from_utf8_lossy(b).into_owned()),
            InputSpec::Bytes => Value::Bytes(b.to_vec()),
            InputSpec::Image { width, height, .. } => match &self.codebook {
                Some(cb) => Value::Image(cb.decode(b, *width, *height)),
                None => Value::Bytes(b.to_vec()),
            },
            InputSpec::Numbers { codec, .. } => Value::Numbers(
                b.split(|&c| c == b' ')
                    .map(|p| codec.decode(p).map_or(f32::NAN, |x| x as f32))
                    .collect(),
            ),
        }
    }

    /// every head's answer on an input, by head name
    pub fn run(&self, input: &Value) -> Result<BTreeMap<String, Answer>, String> {
        let (bytes, extra) = self.encode(input)?;
        let extra = if extra.is_empty() {
            vec![0.0; self.clf.net.cfg.extra]
        } else {
            extra
        };
        let outs = if self.clf.net.heads.is_empty() {
            Vec::new()
        } else {
            self.clf.predict_bytes(&bytes, &extra, None)
        };
        let mut m = BTreeMap::new();
        for (h, o) in self.clf.net.heads.iter().zip(&outs) {
            let names = &h.spec.classes;
            let a = match h.spec.kind {
                HeadKind::Regress => Answer::Values(names.iter().cloned().zip(o.probs.iter().copied()).collect()),
                _ => Answer::Classes {
                    probs: names.iter().cloned().zip(o.probs.iter().copied()).collect(),
                    top: names[o.top].clone(),
                    members: o.members.iter().map(|&i| names[i].clone()).collect(),
                },
            };
            m.insert(h.spec.name.clone(), a);
        }
        if let Some(g) = &self.clf.net.generator {
            let prompt = self.prompt(&bytes);
            let (out, _) = generate::sample(&self.clf, &prompt, &self.spec.sampling, 0);
            m.insert(g.name.clone(), Answer::Generated(self.decode(&out)));
        }
        Ok(m)
    }

    /// what a generator continues from: a sequence-to-sequence model's input and PAIR
    fn prompt(&self, input: &[u8]) -> Vec<u8> {
        let pairs = self.spec.data.iter().any(|q| matches!(q, DataQuery::Pairs { .. }));
        if pairs {
            [input, &[PAIR]].concat()
        } else {
            input.to_vec()
        }
    }

    /// the spec's data queries as training examples
    pub fn examples(&mut self, src: &dyn Source) -> Result<Vec<Example>, String> {
        let spec = self.spec.clone();
        let mut items: Vec<Item> = Vec::new();
        let mut pairs: Vec<(Value, Value, String)> = Vec::new();
        for q in &spec.data {
            match q {
                DataQuery::List {
                    list,
                    head,
                    class,
                    value,
                } => items.extend(
                    src.list(list)?
                        .into_iter()
                        .map(|v| (v, head.clone(), class.clone(), value.clone())),
                ),
                DataQuery::Tagged {
                    tag,
                    head,
                    class,
                    value,
                } => items.extend(
                    src.tagged(tag)?
                        .into_iter()
                        .map(|v| (v, head.clone(), class.clone(), value.clone())),
                ),
                DataQuery::Pairs { input, output, head } => {
                    pairs.extend(src.pairs(input, output)?.into_iter().map(|(a, b)| (a, b, head.clone())))
                }
                DataQuery::Inline { items: its } => items.extend(
                    its.iter()
                        .map(|i| (i.input.clone(), i.head.clone(), i.class.clone(), i.value.clone())),
                ),
            }
        }
        if let InputSpec::Image { patch, codes, .. } = spec.input
            && self.codebook.is_none()
        {
            let ims: Vec<Image> = items
                .iter()
                .map(|i| &i.0)
                .chain(pairs.iter().flat_map(|p| [&p.0, &p.1]))
                .filter_map(|v| match v {
                    Value::Image(im) => Some(im.clone()),
                    _ => None,
                })
                .collect();
            self.codebook = Some(Codebook::learn(&ims, patch, codes, 8, spec.seed)?);
        }
        let mut out = Vec::new();
        for (v, head, class, value) in items {
            let (bytes, extra) = self.encode(&v)?;
            let d = spec.decl(&head)?;
            let target = match (d.kind, spec.slot(&head)?) {
                (OutKind::Generate, _) => Target::Next { from: 0 },
                (OutKind::Classify, _) => Target::Class(index(d, class.as_deref())?),
                (OutKind::Multi, _) => Target::Label(index(d, class.as_deref())?, true),
                (OutKind::Regress, _) => Target::Values(value.unwrap_or_default()),
            };
            let h = match spec.slot(&head)? {
                Slot::Pooled(i) => i,
                Slot::Gen => 0,
            };
            out.push(Example::bytes(&bytes, h, target).with_extra(extra));
        }
        for (a, b, _) in pairs {
            let (ia, _) = self.encode(&a)?;
            let (ib, _) = self.encode(&b)?;
            let prompt = self.prompt(&ia);
            out.push(Example::bytes(
                &[&prompt[..], &ib[..]].concat(),
                0,
                Target::Next { from: prompt.len() },
            ));
        }
        Ok(out)
    }
}

fn index(d: &HeadDecl, class: Option<&str>) -> Result<usize, String> {
    let c = class.ok_or(format!("data for head {:?} names a class", d.name))?;
    d.classes
        .iter()
        .position(|x| x == c)
        .ok_or(format!("head {:?} has no class {c:?}", d.name))
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Text(_) => "text",
        Value::Bytes(_) => "bytes",
        Value::Image(_) => "picture",
        Value::Numbers(_) => "numbers",
    }
}

fn input_kind(i: &InputSpec) -> &'static str {
    match i {
        InputSpec::Text { .. } => "text",
        InputSpec::Bytes => "bytes",
        InputSpec::Image { .. } => "picture",
        InputSpec::Numbers { .. } => "numbers",
    }
}

/// resolves the spec's data and trains every head together (a fresh model when `model`
/// is None; an "auto" trunk is sized for the data found)
pub fn train(
    model: Option<Model>,
    spec: &ModelSpec,
    src: &dyn Source,
    profile: Option<&MachineProfile>,
    cb: impl FnMut(&Progress) -> Flow,
) -> Result<(Model, TrainReport), String> {
    let mut m = match model {
        Some(m) => m,
        None => build(spec, profile)?,
    };
    let data = m.examples(src)?;
    if data.is_empty() {
        return Err(format!("model spec {:?}: the data queries found nothing", spec.name));
    }
    if spec.trunk.preset.as_deref() == Some("auto") && m.clf.meta.steps == 0 {
        let cb_saved = m.codebook.take();
        m = build_for(spec, profile, data.len())?;
        m.codebook = cb_saved;
    }
    let rep = train::train(&mut m.clf, &data, &spec.train_opts(), cb)?;
    Ok((m, rep))
}

impl Judge for Model {
    fn judge(&self, data: &[u8]) -> Vec<Output> {
        self.clf.judge(data)
    }
}

/// the spec's critics, resolved by model name, judge `opts.candidates` candidates;
/// the kept ones come best first
pub fn generate(
    model: &Model,
    models: &BTreeMap<String, &Model>,
    prompt: &Value,
    candidates: usize,
    guidance: Option<generate::Guidance>,
) -> Result<Vec<Candidate>, String> {
    let mut critics = Vec::new();
    for c in &model.spec.critics {
        let m = models
            .get(&c.model)
            .ok_or(format!("critic {:?} is not loaded", c.model))?;
        let slot = m.spec.slot(&c.head)?;
        let Slot::Pooled(h) = slot else {
            return Err(format!("critic {:?}: {:?} is a generate head", c.model, c.head));
        };
        let d = m.spec.decl(&c.head)?;
        let pos = |n: &str| {
            d.classes
                .iter()
                .position(|x| x == n)
                .ok_or(format!("critic {:?}: no class {n:?}", c.model))
        };
        let want = match &c.want {
            WantDecl::Class(n) => Want::Class(pos(n)?),
            WantDecl::Not(n) => Want::Not(pos(n)?),
            WantDecl::Near { value, target, scale } => Want::Near {
                index: pos(value)?,
                value: *target,
                scale: *scale,
            },
            WantDecl::Range { value, lo, hi, scale } => Want::Range {
                index: pos(value)?,
                lo: *lo,
                hi: *hi,
                scale: *scale,
            },
        };
        critics.push(Critic::new(*m as &dyn Judge, h, want, c.weight));
    }
    let (bytes, _) = match prompt {
        Value::Bytes(b) if b.is_empty() => (Vec::new(), Vec::new()),
        v => model.encode(v)?,
    };
    let opts = GenOpts {
        prompt: model.prompt(&bytes),
        sampling: model.spec.sampling.clone(),
        candidates,
        guidance,
        device: model.spec.device,
        ..GenOpts::default()
    };
    generate::generate(&model.clf, &critics, &model.spec.constraints, &opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn author_spec() -> ModelSpec {
        ModelSpec::from_json(
            r#"{
              "name": "kind",
              "input": {"type": "text"},
              "trunk": {"preset": "test"},
              "heads": [{"name": "kind", "kind": "classify", "classes": ["fruit", "rock"]}],
              "data": [
                {"from": "list", "list": "fruits", "head": "kind", "class": "fruit"},
                {"from": "list", "list": "rocks", "head": "kind", "class": "rock"}
              ],
              "train": {"epochs": 12, "batch": 8, "lr": 0.003, "val_fraction": 0.0, "threads": 2}
            }"#,
        )
        .unwrap()
    }

    fn source() -> MemSource {
        MemSource::default()
            .with_texts(
                "fruits",
                &[
                    "apple plum",
                    "peach melon",
                    "grape apple",
                    "mango plum",
                    "cherry peach",
                    "melon grape",
                ],
            )
            .with_texts(
                "rocks",
                &[
                    "granite slate",
                    "flint shale",
                    "quartz basalt",
                    "stone rock",
                    "slate flint",
                    "basalt stone",
                ],
            )
    }

    #[test]
    fn a_classifier_from_a_spec() {
        let spec = author_spec();
        let (m, rep) = train(None, &spec, &source(), None, |_| Flow::Go).unwrap();
        assert!(rep.steps > 0);
        let a = m.run(&Value::Text("slate granite flint".into())).unwrap();
        match &a["kind"] {
            Answer::Classes { top, .. } => assert_eq!(top, "rock"),
            x => panic!("{x:?}"),
        }
        let back = Model::from_bytes(&m.to_bytes()).unwrap();
        assert_eq!(back.spec, m.spec);
        assert_eq!(
            back.run(&Value::Text("apple".into())).unwrap(),
            m.run(&Value::Text("apple".into())).unwrap()
        );
        // an old-style classifier converts
        let c = Model::from_classifier(m.clf.clone());
        assert_eq!(c.spec.heads, m.spec.heads);
        assert!(c.spec.check().is_ok());
        assert!(build(&c.spec, None).is_ok());
    }

    #[test]
    fn specs_are_checked_with_clear_errors() {
        let bad = |j: &str| ModelSpec::from_json(j).unwrap_err();
        assert!(bad(r#"{"name":"x","input":{"type":"text"},"heads":[]}"#).contains("at least one head"));
        assert!(
            bad(r#"{"name":"x","version":9,"input":{"type":"text"},"heads":[{"name":"a","kind":"generate"}]}"#)
                .contains("version 9")
        );
        assert!(
            bad(r#"{"name":"x","input":{"type":"text"},"heads":[{"name":"a","kind":"classify","classes":["p"]}]}"#)
                .contains("two classes")
        );
        assert!(bad(
            r#"{"name":"x","input":{"type":"image","width":30,"height":32},"heads":[{"name":"a","kind":"generate"}]}"#
        )
        .contains("multiple"));
        assert!(
            bad(
                r#"{"name":"x","input":{"type":"text"},"heads":[{"name":"a","kind":"classify","classes":["p","q"]}],
              "data":[{"from":"list","list":"l","head":"a","class":"z"}]}"#
            )
            .contains("no class \"z\"")
        );
        assert!(bad(r#"{"name":"x","input":{"type":"text"},"heads":[{"name":"a","kind":"generate"}],"trunk":{"causal":false}}"#)
            .contains("causal"));
        assert!(
            bad(
                r#"{"name":"x","input":{"type":"text"},"heads":[{"name":"a","kind":"regress","classes":["v"]}],
              "critics":[{"model":"m","head":"h","want":{"class":"c"}}]}"#
            )
            .contains("has none")
        );
        assert!(
            build(&ModelSpec::from_json(r#"{"name":"x","input":{"type":"text"},"trunk":{"preset":"huge"},"heads":[{"name":"a","kind":"generate"}]}"#).unwrap(), None)
                .unwrap_err()
                .contains("unknown trunk preset")
        );
        // sizes left out: not the test preset
        let s =
            ModelSpec::from_json(r#"{"name":"x","input":{"type":"text"},"heads":[{"name":"a","kind":"generate"}]}"#)
                .unwrap();
        let c = s.config(None, 0).unwrap();
        assert!(c.causal && c.d_model >= Config::small().d_model && !c.letters);
    }

    #[test]
    fn a_steered_generator_and_a_number_regressor_from_specs() {
        let (critic, _) = train(None, &author_spec(), &source(), None, |_| Flow::Go).unwrap();
        let gspec = ModelSpec::from_json(
            r#"{
              "name": "writer",
              "input": {"type": "text"},
              "trunk": {"preset": "test", "max_len": 32},
              "heads": [{"name": "next", "kind": "generate"}],
              "data": [
                {"from": "list", "list": "fruits", "head": "next"},
                {"from": "list", "list": "rocks", "head": "next"}
              ],
              "critics": [{"model": "kind", "head": "kind", "want": {"class": "rock"}}],
              "sampling": {"temperature": 1.0, "max_len": 16, "seed": 3},
              "train": {"epochs": 10, "batch": 4, "lr": 0.01, "val_fraction": 0.0, "threads": 2}
            }"#,
        )
        .unwrap();
        let (g, _) = train(None, &gspec, &source(), None, |_| Flow::Go).unwrap();
        let models: BTreeMap<String, &Model> = [("kind".to_string(), &critic)].into();
        let c = generate(&g, &models, &Value::Bytes(vec![]), 6, None).unwrap();
        assert_eq!(c, generate(&g, &models, &Value::Bytes(vec![]), 6, None).unwrap());
        assert!(c.windows(2).all(|w| w[0].score >= w[1].score));
        assert!(matches!(
            g.run(&Value::Text("gra".into())).unwrap()["next"],
            Answer::Generated(Value::Text(_))
        ));
        assert!(
            generate(&g, &BTreeMap::new(), &Value::Bytes(vec![]), 2, None)
                .unwrap_err()
                .contains("not loaded")
        );

        // numbers: y = a + b, read as fixed-width text plus the numbers themselves
        let nspec = ModelSpec::from_json(
            r#"{
              "name": "sum",
              "input": {"type": "numbers", "names": ["a", "b"], "codec": {"digits": 1, "decimals": 1, "signed": false}},
              "trunk": {"preset": "test"},
              "heads": [{"name": "y", "kind": "regress", "classes": ["sum"]}],
              "train": {"epochs": 60, "batch": 8, "lr": 0.01, "val_fraction": 0.0, "threads": 1}
            }"#,
        )
        .unwrap();
        let items: Vec<InlineItem> = (0..24)
            .map(|i| {
                let (a, b) = ((i % 5) as f32 * 0.5, (i % 7) as f32 * 0.3);
                InlineItem {
                    input: Value::Numbers(vec![a, b]),
                    head: "y".into(),
                    class: None,
                    value: Some(vec![a + b]),
                }
            })
            .collect();
        let mut nspec = nspec;
        nspec.data.push(DataQuery::Inline { items });
        let (n, _) = train(None, &nspec, &MemSource::default(), None, |_| Flow::Go).unwrap();
        let Answer::Values(v) = &n.run(&Value::Numbers(vec![1.0, 0.6])).unwrap()["y"] else {
            panic!()
        };
        assert!((v["sum"] - 1.6).abs() < 0.4, "{v:?}");
    }

    #[test]
    fn a_picture_generator_from_a_spec() {
        let ims: Vec<Value> = (0..8)
            .map(|i| Value::Image(super::super::codec::toy(16, i % 4, i as u64)))
            .collect();
        let spec = ModelSpec::from_json(
            r#"{
              "name": "pictures",
              "input": {"type": "image", "width": 16, "height": 16, "patch": 4, "codes": 16},
              "trunk": {"preset": "test"},
              "heads": [{"name": "draw", "kind": "generate"}],
              "data": [{"from": "list", "list": "toys", "head": "draw"}],
              "sampling": {"temperature": 0.8, "max_len": 40, "seed": 1},
              "train": {"epochs": 6, "batch": 4, "lr": 0.01, "val_fraction": 0.0, "threads": 1}
            }"#,
        )
        .unwrap();
        assert_eq!(spec.config(None, 0).unwrap().max_len, 5 * 4 + 2);
        let (m, _) = train(None, &spec, &MemSource::default().with_list("toys", ims), None, |_| {
            Flow::Go
        })
        .unwrap();
        let Answer::Generated(Value::Image(im)) =
            &m.run(&Value::Image(super::super::codec::toy(16, 0, 99))).unwrap()["draw"]
        else {
            panic!()
        };
        assert_eq!((im.width, im.height), (16, 16));
    }
}
