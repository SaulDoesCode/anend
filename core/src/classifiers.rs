//! Classifiers kept in a save: what each one is (its classes as lists, head, size,
//! steering, the classifiers it builds on, training options, history), its weights, and
//! the yes/no feedback players gave it. Training data is read from the save's lists when
//! training starts, so a classifier follows its lists as they grow.
//!
//! | table | maps |
//! |---|---|
//! | `classifiers` | name -> definition, JSON (`Definition`) |
//! | `classifier weights` | name -> the `ANENDCLS` file (`classify::io`) |
//! | `classifier feedback` | (name, n) -> one yes/no, JSON (`FeedbackEntry`) |
//!
//! The definition is the authority for steering: the weights file carries a copy, but
//! `load` puts the definition's steering on the loaded classifier, so steering never
//! needs the weights rewritten.
//!
//! Training is split so a caller can run it off its own thread without holding the
//! save: `Store::prepare` snapshots the examples and the classifiers (a `Training`),
//! `Training::run` trains with no store at all, and `Store::commit` writes the weights
//! and the history entry. The hub runs `run` on a thread of its own; the desktop app
//! can do the same.

use redb::{ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::classify::compose::{Contribution, Edge, Graph, StackReport};
use crate::classify::device::{self, Choice, Reading, Runner};
use crate::classify::grow::{self, Grower};
use crate::classify::metrics::Metrics;
use crate::classify::steer::Output;
use crate::classify::train::{self, Feedback, FeedbackOpts};
use crate::classify::{
    BiasReport, Classifier, Config, Example, Flow, HeadKind, HeadSpec, Pool, Progress, Steer, Target, TrainOpts,
};
use crate::memory::{LISTS, MEMORIES, Ref};
use crate::note::Note;
use crate::store::{NOTES, Store, StoreResult};

pub const DEFINITIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("classifiers");
pub const WEIGHTS: TableDefinition<&str, &[u8]> = TableDefinition::new("classifier weights");
pub const FEEDBACK: TableDefinition<(&str, u64), &[u8]> = TableDefinition::new("classifier feedback");
/// (name, n) -> what a yes/no on that classifier changed, to take it back (`Undo`)
pub const UNDO: TableDefinition<(&str, u64), &[u8]> = TableDefinition::new("classifier undo");

/// how many yes/no answers on a classifier can be taken back
pub const UNDO_KEPT: u64 = 20;

/// how many history events a definition keeps (oldest dropped first)
pub const HISTORY_KEPT: usize = 200;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn create_tables(txn: &WriteTransaction) -> StoreResult<()> {
    txn.open_table(DEFINITIONS).map_err(err)?;
    txn.open_table(WEIGHTS).map_err(err)?;
    txn.open_table(FEEDBACK).map_err(err)?;
    txn.open_table(UNDO).map_err(err)?;
    Ok(())
}

/// what one yes/no changed: the feedback it logged, and each classifier it wrote as it
/// was before (its definition's JSON and its weights in base64)
#[derive(Serialize, Deserialize)]
struct Undo {
    feedback: u64,
    was: Vec<(String, String, Option<String>)>,
}

// ---------------------------------------------------------------- what's stored

/// a class: a list, by its head's label, and the list's name when it was chosen
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClassDef {
    pub name: String,
    pub list: String,
}

/// an upstream classifier whose steered outputs this one takes as extra inputs
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Input {
    pub from: String,
    /// how much end-to-end feedback reaches it (0 leaves it alone)
    #[serde(default = "one")]
    pub coupling: f32,
}

fn one() -> f32 {
    1.0
}

/// something that happened to a classifier: a training run, growth, a list written
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub time: String,
    /// "train", "growth", "enlist", "feedback stack"
    pub kind: String,
    #[serde(default)]
    pub note: String,
    /// the head's metrics after training
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Metrics>,
    /// "held out" or "training" (when too few texts were left to hold some out)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub measured_on: String,
    #[serde(default)]
    pub examples: usize,
    #[serde(default)]
    pub steps: u64,
    #[serde(default)]
    pub seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    pub name: String,
    pub classes: Vec<ClassDef>,
    /// a list of texts that are none of the classes: an extra class for a single-label
    /// head, all labels off for a multi-label one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not: Option<ClassDef>,
    pub head: HeadKind,
    /// "tiny", "compact", "small" or "base" (see `preset`)
    pub preset: String,
    /// the gematria input channels (letter values, word sums, the side vector): opt-in,
    /// off for new classifiers; definitions saved before it was optional had them on
    #[serde(default = "yes")]
    pub gematria: bool,
    pub steer: Steer,
    #[serde(default)]
    pub inputs: Vec<Input>,
    pub train: TrainOpts,
    #[serde(default)]
    pub history: Vec<Event>,
    /// class name -> the list its outputs were last written to (a label)
    #[serde(default)]
    pub outputs: BTreeMap<String, String>,
    /// bumped by every write of the weights
    #[serde(default)]
    pub version: u64,
    pub seed: u64,
    pub created: String,
    pub modified: String,
}

/// one yes or no a player gave
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackEntry {
    pub classifier: String,
    /// the memory or note judged, when it was one (a label)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub text: String,
    pub class: String,
    pub yes: bool,
    pub weight: f32,
    pub time: String,
    /// end to end through the classifiers it builds on
    #[serde(default)]
    pub stack: bool,
    /// the class's unsteered probability before and after
    pub before: f32,
    pub after: f32,
}

fn yes() -> bool {
    true
}

/// the sizes on offer, smallest first
pub const PRESETS: [&str; 4] = ["tiny", "compact", "small", "base"];
pub const DEFAULT_PRESET: &str = "compact";

pub fn preset(name: &str) -> Option<Config> {
    Some(match name {
        "tiny" => Config::tiny(),
        // 89k weights (with three classes): seconds for a few lists of short memories
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
        _ => return None,
    })
}

/// training options that suit a preset
pub fn preset_opts(name: &str) -> TrainOpts {
    let base = TrainOpts {
        val_fraction: 0.15,
        patience: 4,
        ..TrainOpts::default()
    };
    match name {
        "tiny" => TrainOpts {
            epochs: 10,
            batch: 8,
            lr: 3e-3,
            ..base
        },
        "compact" => TrainOpts {
            epochs: 14,
            batch: 8,
            lr: 2e-3,
            ..base
        },
        _ => base,
    }
}

/// the largest batch a training run takes
pub const MAX_BATCH: usize = 4096;

/// steering a classifier of `k` classes may have: one head of `k` biases, all finite, a
/// temperature above 0, a threshold from 0 to 1 and gains of 0 or more
pub fn check_steer(steer: &crate::classify::Steer, k: usize) -> StoreResult<()> {
    if steer.heads.len() != 1 || steer.heads[0].bias.len() != k {
        return Err(format!("steering here has one head of {k} biases"));
    }
    let h = &steer.heads[0];
    let finite = |x: f32| x.is_finite();
    if !(h.temperature > 0.0 && h.temperature.is_finite())
        || !h.bias.iter().all(|&b| finite(b))
        || !(0.0..=1.0).contains(&h.threshold)
    {
        return Err("temperature above 0, biases finite, threshold from 0 to 1".into());
    }
    let g = &steer.gains;
    if ![g.letter, g.word, g.side, g.extra]
        .iter()
        .all(|&x| finite(x) && x >= 0.0)
    {
        return Err("gains are 0 or more".into());
    }
    Ok(())
}

pub fn clean_name(name: &str) -> StoreResult<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 80 || n.chars().any(char::is_control) {
        return Err("a classifier's name is 1 to 80 letters".into());
    }
    Ok(n.to_string())
}

impl Definition {
    /// a new, untrained definition; the class lists are checked by `Store::create_classifier`
    pub fn new(
        name: &str,
        classes: Vec<ClassDef>,
        not: Option<ClassDef>,
        head: HeadKind,
        preset_name: &str,
    ) -> StoreResult<Definition> {
        let name = clean_name(name)?;
        if preset(preset_name).is_none() {
            return Err(format!(
                "there's no preset {preset_name:?} (tiny, compact, small, base)"
            ));
        }
        let now = crate::note::now();
        let mut d = Definition {
            seed: crate::classify::math::fnv(0, name.as_bytes()),
            name,
            classes,
            not,
            head,
            preset: preset_name.into(),
            gematria: false,
            steer: Steer {
                gains: Default::default(),
                heads: vec![],
            },
            inputs: vec![],
            train: preset_opts(preset_name),
            history: vec![],
            outputs: BTreeMap::new(),
            version: 0,
            created: now.clone(),
            modified: now,
        };
        d.steer.heads = vec![crate::classify::HeadSteer::neutral(d.class_names().len())];
        match head {
            HeadKind::Regress => return Err("classifiers here have class or label heads".into()),
            HeadKind::Single if d.class_names().len() < 2 => {
                return Err("a classifier needs two lists (or one and a \"not\" list)".into());
            }
            _ if d.classes.is_empty() => return Err("a classifier needs at least one list".into()),
            _ => {}
        }
        Ok(d)
    }

    /// the head's class names: the lists, then the "not" list for a single-label head
    pub fn class_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.classes.iter().map(|c| c.name.clone()).collect();
        if self.head == HeadKind::Single
            && let Some(n) = &self.not
        {
            v.push(n.name.clone());
        }
        v
    }

    /// a class by name (or its index as text)
    pub fn class_index(&self, class: &str) -> StoreResult<usize> {
        let names = self.class_names();
        names
            .iter()
            .position(|n| n == class)
            .or_else(|| class.parse::<usize>().ok().filter(|&i| i < names.len()))
            .ok_or_else(|| format!("{} has no class {class:?}", self.name))
    }

    pub fn trained(&self) -> bool {
        self.version > 0
    }

    /// the last training event, if any
    pub fn last_training(&self) -> Option<&Event> {
        self.history.iter().rev().find(|e| e.kind == "train")
    }

    pub fn push(&mut self, e: Event) {
        self.history.push(e);
        let n = self.history.len();
        if n > HISTORY_KEPT {
            self.history.drain(..n - HISTORY_KEPT);
        }
    }

    /// a fresh, untrained classifier for this definition; `extra` is the width of its
    /// upstream inputs
    pub fn build(&self, extra: usize) -> StoreResult<Classifier> {
        let mut cfg = preset(&self.preset).ok_or("unknown preset")?.gematria(self.gematria);
        cfg.extra = extra;
        let names = self.class_names();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut clf = Classifier::new(cfg, &[HeadSpec::new(&self.name, self.head, &names)], self.seed)?;
        clf.meta.name = self.name.clone();
        clf.steer = self.steer.clone();
        Ok(clf)
    }
}

// ---------------------------------------------------------------- a classifier with its upstream

/// a classifier and everything it builds on, loaded, as a `Graph` (the judged one last)
pub struct Stack {
    pub graph: Graph,
    pub node: usize,
    /// definitions in graph order
    pub defs: Vec<Definition>,
    /// where its texts are read, decided on first use
    pub read: Read,
}

/// where a stack reads texts: the choice it was loaded with, and (on first use) the
/// device that gives and the nets uploaded to it
pub struct Read {
    pub choice: Choice,
    /// whether the GPU may be used here
    pub allowed: bool,
    /// the save's classifiers file, naming the uploaded nets
    pub origin: String,
    state: std::sync::OnceLock<(Reading, Vec<Option<std::sync::Arc<Runner>>>)>,
}

impl Read {
    pub fn new(choice: Choice, allowed: bool, origin: String) -> Read {
        Read {
            choice,
            allowed,
            origin,
            state: std::sync::OnceLock::new(),
        }
    }

    /// the CPU, as the reference
    pub fn cpu() -> Read {
        Read::new(Choice::Cpu, false, String::new())
    }
}

impl Stack {
    pub fn def(&self) -> &Definition {
        &self.defs[self.node]
    }

    pub fn clf(&self) -> &Classifier {
        &self.graph.nodes[self.node].clf
    }

    pub fn names(&self) -> Vec<String> {
        self.defs.iter().map(|d| d.name.clone()).collect()
    }

    /// where this stack reads texts, and the nets on the device for it (opened on first
    /// use; a net the device can't take sends the whole stack to the CPU, saying why)
    fn reader(&self) -> &(Reading, Vec<Option<std::sync::Arc<Runner>>>) {
        self.read.state.get_or_init(|| {
            let r = device::reading(self.read.choice, self.read.allowed);
            if !r.device.is_gpu() {
                return (r, vec![]);
            }
            let mut runners = Vec::new();
            for (d, n) in self.defs.iter().zip(&self.graph.nodes) {
                let key = format!("{}\u{0}{}\u{0}{}", self.read.origin, d.name, d.version);
                match device::runner(&key, r.device, &n.clf.net) {
                    Ok(x) => runners.push(x),
                    Err(e) => {
                        let note = format!("{:?} doesn't fit the GPU ({e}): read on the CPU", d.name);
                        return (Reading::cpu(r.choice, Some(note)), vec![]);
                    }
                }
            }
            (r, runners)
        })
    }

    /// where this stack reads texts
    pub fn reading(&self) -> &Reading {
        &self.reader().0
    }

    /// every node's steered outputs and the judged node's pooled vector for many texts,
    /// on the stack's device
    pub fn read_many(&self, texts: &[&str], pool: &Pool) -> Vec<(Vec<Output>, Vec<f32>)> {
        let runners: Vec<Option<&Runner>> = self.reader().1.iter().map(|r| r.as_deref()).collect();
        self.graph.read_node_many(self.node, texts, &runners, pool)
    }

    /// head 0's steered output for every text, on the stack's device (the pool on the CPU)
    pub fn outputs(&self, texts: &[&str], pool: &Pool) -> Vec<Output> {
        self.read_many(texts, pool)
            .into_iter()
            .map(|(mut o, _)| o.swap_remove(0))
            .collect()
    }

    /// the extra inputs of every text
    pub fn extras(&self, texts: &[&str], pool: &Pool) -> Vec<Vec<f32>> {
        if self.clf().net.cfg.extra == 0 {
            return vec![];
        }
        let slots: Vec<std::sync::Mutex<Vec<f32>>> = texts.iter().map(|_| Default::default()).collect();
        pool.run(texts.len(), &|i| {
            *slots[i].lock().unwrap() = self.graph.extra(self.node, texts[i])
        });
        slots.into_iter().map(|s| s.into_inner().unwrap()).collect()
    }

    pub fn bias(&self, texts: &[&str], pool: &Pool) -> BiasReport {
        let extras = self.extras(texts, pool);
        self.clf().bias(0, texts, &extras, pool)
    }

    /// each classifier's share in putting `text` in `class`, with what each says of it
    pub fn contributions(&self, text: &str, class: usize) -> StoreResult<Vec<(Contribution, Output)>> {
        let outs = self.graph.outputs(self.node, text);
        let cs = self.graph.contributions(self.node, 0, class, text)?;
        Ok(cs
            .into_iter()
            .map(|c| {
                let o = outs[c.node].as_ref().map(|o| o[0].clone()).unwrap_or_else(|| Output {
                    logits: vec![],
                    probs: vec![],
                    top: 0,
                    members: vec![],
                });
                (c, o)
            })
            .collect())
    }
}

// ---------------------------------------------------------------- training off the store

/// a training run's snapshot: needs no store until `Store::commit`
pub struct Training {
    pub stack: Stack,
    pub data: Vec<Example>,
    pub opts: TrainOpts,
    /// growth rounds (1: plain training)
    pub rounds: usize,
    /// the version of the weights it started from
    pub from_version: u64,
    /// why it runs on the CPU when a GPU was asked for
    pub device_note: Option<String>,
}

/// what a run did
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Summary {
    pub classes: Vec<String>,
    pub examples: usize,
    pub train_examples: usize,
    pub val_examples: usize,
    pub steps: u64,
    pub seconds: f64,
    pub tokens_per_sec: f64,
    pub stopped_early: bool,
    pub cancelled: bool,
    pub metrics: Option<Metrics>,
    pub measured_on: String,
    pub growth: Vec<String>,
}

/// a training run's progress as the caller sees it
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tick {
    pub round: usize,
    pub rounds: usize,
    pub epoch: usize,
    pub step: usize,
    pub steps: usize,
    pub loss: f64,
    pub lr: f32,
    pub tokens_per_sec: f64,
    /// set on evaluation steps: validation loss, accuracy and macro-F1
    pub eval: Option<(f64, f64, f64)>,
}

impl Training {
    /// trains the snapshot; `cb` sees every step and may stop it
    pub fn run(&mut self, mut cb: impl FnMut(&Tick) -> Flow) -> StoreResult<Summary> {
        let started = std::time::Instant::now();
        let (tr, va) = train::split(&self.data, self.opts.val_fraction, self.opts.seed);
        let (tr, va) = if tr.is_empty() { (va, vec![]) } else { (tr, va) };
        let node = self.stack.node;
        let rounds = self.rounds.max(1);
        let mut round = 0;
        let mut last = 0;
        let mut on_step = |p: &Progress| {
            if p.step < last {
                round += 1;
            }
            last = p.step;
            cb(&Tick {
                round,
                rounds,
                epoch: p.epoch,
                step: p.step,
                steps: p.steps,
                loss: p.loss,
                lr: p.lr,
                tokens_per_sec: p.tokens_per_sec,
                eval: p.eval.map(|e| {
                    let m = e.heads.first().cloned().flatten().unwrap_or_default();
                    (e.loss, m.accuracy, m.macro_f1)
                }),
            })
        };
        let clf = &mut self.stack.graph.nodes[node].clf;
        let steps_before = clf.meta.steps;
        let mut s = Summary {
            classes: self.stack.defs[node].class_names(),
            examples: self.data.len(),
            train_examples: tr.len(),
            val_examples: va.len(),
            ..Summary::default()
        };
        if rounds > 1 {
            let mut g = Grower::new(0.95);
            let out = grow::cycle(clf, &self.data, &tr, &va, &self.opts, &mut g, rounds, &mut on_step)?;
            for r in &out {
                s.cancelled |= r.train.cancelled;
                s.tokens_per_sec = r.train.tokens_per_sec;
                if !r.note.is_empty() {
                    s.growth.push(r.note.clone());
                }
            }
        } else {
            let rep = train::fit(clf, &self.data, &tr, &va, &self.opts, &mut on_step)?;
            s.cancelled = rep.cancelled;
            s.stopped_early = rep.stopped_early;
            s.tokens_per_sec = rep.tokens_per_sec;
        }
        let pool = self.opts.pool();
        let (idx, on) = if va.is_empty() {
            (&tr, "training")
        } else {
            (&va, "held out")
        };
        let ev = train::evaluate(clf, &self.data, idx, &pool);
        s.metrics = ev.heads.into_iter().next().flatten();
        s.measured_on = on.into();
        s.steps = clf.meta.steps - steps_before;
        s.seconds = started.elapsed().as_secs_f64();
        Ok(s)
    }
}

// ---------------------------------------------------------------- the store

/// options for `Store::prepare`
#[derive(Clone, Debug, Default)]
pub struct TrainArgs {
    pub epochs: Option<usize>,
    pub lr: Option<f32>,
    pub batch: Option<usize>,
    pub max_steps: Option<usize>,
    pub threads: Option<usize>,
    /// growth rounds, each `epochs` long, deciding after each to widen, deepen or prune
    pub rounds: Option<usize>,
    /// start from new weights instead of the trained ones
    pub fresh: bool,
    /// where to train: the CPU (the reference), the GPU exact or fast; a GPU this build
    /// or machine doesn't have falls back to the CPU, saying why
    pub device: crate::classify::device::Device,
}

/// what yes/no feedback is about: a text, and the memory or note it came from
#[derive(Clone, Debug)]
pub struct Judged {
    pub key: Option<Ref>,
    pub text: String,
}

impl Store {
    // ---- definitions and weights

    pub fn classifier_names(&self) -> StoreResult<Vec<String>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(DEFINITIONS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            out.push(item.map_err(err)?.0.value().to_string());
        }
        Ok(out)
    }

    pub fn classifiers(&self) -> StoreResult<Vec<Definition>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(DEFINITIONS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (_, v) = item.map_err(err)?;
            out.push(serde_json::from_slice(v.value()).map_err(err)?);
        }
        Ok(out)
    }

    pub fn classifier(&self, name: &str) -> StoreResult<Definition> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(DEFINITIONS).map_err(err)?;
        let found = t.get(name).map_err(err)?;
        let g = found.ok_or_else(|| format!("there's no classifier called {name:?}"))?;
        serde_json::from_slice(g.value()).map_err(err)
    }

    pub fn classifier_weights(&self, name: &str) -> StoreResult<Option<Vec<u8>>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(WEIGHTS).map_err(err)?;
        Ok(t.get(name).map_err(err)?.map(|g| g.value().to_vec()))
    }

    /// writes a definition, and with `weights` the weights too (bumping the version), in
    /// one transaction
    pub fn put_classifier(&self, def: &mut Definition, weights: Option<&Classifier>) -> StoreResult<()> {
        def.modified = crate::note::now();
        let bytes = weights.map(|c| {
            let mut c = c.clone();
            c.steer = def.steer.clone();
            c.to_bytes()
        });
        if bytes.is_some() {
            def.version += 1;
        }
        let txn = self.db.begin_write().map_err(err)?;
        {
            let json = serde_json::to_vec(&*def).map_err(err)?;
            txn.open_table(DEFINITIONS)
                .map_err(err)?
                .insert(def.name.as_str(), json.as_slice())
                .map_err(err)?;
            if let Some(b) = bytes {
                txn.open_table(WEIGHTS)
                    .map_err(err)?
                    .insert(def.name.as_str(), b.as_slice())
                    .map_err(err)?;
            }
        }
        txn.commit().map_err(err)
    }

    /// a new classifier over lists (by label): `classes` the lists, `not` an optional
    /// list of texts that are none of them, `inputs` classifiers it builds on; no
    /// gematria channels (see `create_classifier_with`)
    pub fn create_classifier(
        &self,
        name: &str,
        classes: &[Ref],
        not: Option<&Ref>,
        head: HeadKind,
        preset_name: &str,
        inputs: Vec<Input>,
    ) -> StoreResult<Definition> {
        self.create_classifier_with(name, classes, not, head, preset_name, inputs, false)
    }

    /// `create_classifier`, with the gematria input channels (letter values, word sums,
    /// the side vector) on when `gematria`
    #[allow(clippy::too_many_arguments)]
    pub fn create_classifier_with(
        &self,
        name: &str,
        classes: &[Ref],
        not: Option<&Ref>,
        head: HeadKind,
        preset_name: &str,
        inputs: Vec<Input>,
        gematria: bool,
    ) -> StoreResult<Definition> {
        let name = clean_name(name)?;
        if self.classifier(&name).is_ok() {
            return Err(format!("there's already a classifier called {name:?}"));
        }
        let class = |r: &Ref| -> StoreResult<ClassDef> {
            if self.list_members(r)?.is_empty() {
                return Err(format!("{} isn't a list with members", r.label()));
            }
            let name = self.remember(r).map(|h| h.text).unwrap_or_else(|_| r.label());
            Ok(ClassDef { name, list: r.label() })
        };
        let classes = classes.iter().map(class).collect::<StoreResult<Vec<_>>>()?;
        let not = not.map(class).transpose()?;
        let mut def = Definition::new(&name, classes, not, head, preset_name)?;
        def.gematria = gematria;
        for i in &inputs {
            if i.from == name {
                return Err("a classifier can't build on itself".into());
            }
            self.classifier(&i.from)?;
        }
        def.inputs = inputs;
        self.put_classifier(&mut def, None)?;
        Ok(def)
    }

    /// removes a classifier, its weights and its feedback; refused while another
    /// classifier builds on it
    pub fn delete_classifier(&self, name: &str) -> StoreResult<()> {
        self.classifier(name)?;
        if let Some(d) = self
            .classifiers()?
            .into_iter()
            .find(|d| d.inputs.iter().any(|i| i.from == name))
        {
            return Err(format!("{:?} builds on {name:?}; delete that first", d.name));
        }
        let txn = self.db.begin_write().map_err(err)?;
        {
            txn.open_table(DEFINITIONS).map_err(err)?.remove(name).map_err(err)?;
            txn.open_table(WEIGHTS).map_err(err)?.remove(name).map_err(err)?;
            let mut f = txn.open_table(FEEDBACK).map_err(err)?;
            f.retain_in((name, 0)..=(name, u64::MAX), |_, _| false).map_err(err)?;
            let mut u = txn.open_table(UNDO).map_err(err)?;
            u.retain_in((name, 0)..=(name, u64::MAX), |_, _| false).map_err(err)?;
        }
        txn.commit().map_err(err)?;
        self.memo_clear(name)?;
        self.unindex_classifier(name)?;
        self.drop_validators_of(name)
    }

    /// the classifier as stored (untrained weights when it has none yet), its steering
    /// from the definition
    pub fn load_classifier(&self, def: &Definition, extra: usize) -> StoreResult<Classifier> {
        let mut clf = match self.classifier_weights(&def.name)? {
            Some(b) => Classifier::from_bytes(&b)?,
            None => def.build(extra)?,
        };
        if def.steer.heads.len() == clf.net.heads.len()
            && def
                .steer
                .heads
                .iter()
                .zip(&clf.net.heads)
                .all(|(s, h)| s.bias.len() == h.width())
        {
            clf.steer = def.steer.clone();
        }
        Ok(clf)
    }

    /// a classifier and everything it builds on
    pub fn stack(&self, name: &str) -> StoreResult<Stack> {
        let mut order: Vec<Definition> = Vec::new();
        let mut seen: HashMap<String, bool> = HashMap::new(); // false: on the path
        fn visit(
            s: &Store,
            name: &str,
            seen: &mut HashMap<String, bool>,
            order: &mut Vec<Definition>,
        ) -> StoreResult<()> {
            match seen.get(name) {
                Some(true) => return Ok(()),
                Some(false) => return Err(format!("{name:?} builds on itself")),
                None => {}
            }
            seen.insert(name.to_string(), false);
            let d = s.classifier(name)?;
            for i in &d.inputs {
                visit(s, &i.from, seen, order)?;
            }
            seen.insert(name.to_string(), true);
            order.push(d);
            Ok(())
        }
        visit(self, name, &mut seen, &mut order)?;
        let mut graph = Graph::default();
        let index: HashMap<String, usize> = order.iter().enumerate().map(|(i, d)| (d.name.clone(), i)).collect();
        for d in &order {
            let edges: Vec<Edge> = d
                .inputs
                .iter()
                .map(|i| Edge {
                    from: index[&i.from],
                    head: 0,
                    coupling: i.coupling,
                })
                .collect();
            let extra: usize = edges.iter().map(|e| graph.nodes[e.from].clf.net.heads[0].width()).sum();
            let clf = self.load_classifier(d, extra)?;
            graph.add(&d.name, clf, edges)?;
        }
        Ok(Stack {
            node: order.len() - 1,
            graph,
            defs: order,
            read: Read::new(
                device::choice(),
                self.gpu_reads(),
                self.path().to_string_lossy().into_owned(),
            ),
        })
    }

    // ---- texts

    /// a list's members as stored
    pub fn list_members(&self, list: &Ref) -> StoreResult<Vec<Ref>> {
        if let Some(t) = self.texts() {
            return t.members(list);
        }
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_multimap_table(LISTS).map_err(err)?;
        let mut out = Vec::new();
        for v in t.get(list.bytes().as_slice()).map_err(err)? {
            out.push(Ref::from_bytes(v.map_err(err)?.value()));
        }
        Ok(out)
    }

    /// what a classifier reads of a memory (its text) or a note (title and markdown)
    pub fn ref_text(&self, r: &Ref) -> StoreResult<Option<String>> {
        if let Some(t) = self.texts() {
            return t.text(r);
        }
        let txn = self.db.begin_read().map_err(err)?;
        Ok(match r {
            Ref::Memory(k) => {
                let t = txn.open_table(MEMORIES).map_err(err)?;
                t.get(k.as_slice())
                    .map_err(err)?
                    .map(|g| String::from_utf8_lossy(g.value()).into_owned())
            }
            Ref::Note(k) => {
                let t = txn.open_table(NOTES).map_err(err)?;
                let found = t.get(k.as_str()).map_err(err)?;
                found
                    .and_then(|g| serde_json::from_slice::<Note>(g.value()).ok())
                    .map(|n| note_text(&n))
            }
        })
    }

    /// every memory and note a classifier can rank: (ref, what it reads, what's shown)
    pub fn candidates(&self, limit: usize) -> StoreResult<Vec<(Ref, String, String)>> {
        if let Some(t) = self.texts() {
            return t.candidates(limit);
        }
        let mut out: Vec<(Ref, String, String)> = self
            .all_memories()?
            .into_iter()
            .map(|(t, k)| (Ref::Memory(k), t.clone(), t))
            .collect();
        for n in self.all()? {
            out.push((Ref::Note(n.key.clone()), note_text(&n), n.display_title().to_string()));
        }
        out.retain(|c| !c.1.trim().is_empty());
        out.truncate(limit);
        Ok(out)
    }

    /// the training examples a definition's lists give now
    pub fn examples(&self, def: &Definition) -> StoreResult<Vec<Example>> {
        let k = def.classes.len();
        let mut lists: Vec<(Option<usize>, Ref)> = def
            .classes
            .iter()
            .enumerate()
            .map(|(i, c)| Ok((Some(i), Ref::parse(&c.list).ok_or("a class list's key doesn't read")?)))
            .collect::<StoreResult<_>>()?;
        if let Some(n) = &def.not {
            lists.push((None, Ref::parse(&n.list).ok_or("the not list's key doesn't read")?));
        }
        let mut texts: Vec<String> = Vec::new();
        let mut at: HashMap<String, usize> = HashMap::new();
        let mut labels: Vec<Vec<f32>> = Vec::new();
        let mut out = Vec::new();
        for (class, list) in &lists {
            for m in self.list_members(list)? {
                if &m == list {
                    continue;
                }
                let Some(t) = self.ref_text(&m)? else { continue };
                if t.trim().is_empty() {
                    continue;
                }
                match def.head {
                    HeadKind::Single => {
                        let c = class.unwrap_or(k);
                        out.push(Example::new(&t, 0, Target::Class(c)));
                    }
                    _ => {
                        let i = *at.entry(t.clone()).or_insert_with(|| {
                            texts.push(t.clone());
                            labels.push(vec![0.0; k]);
                            texts.len() - 1
                        });
                        if let Some(c) = class {
                            labels[i][*c] = 1.0;
                        }
                    }
                }
            }
        }
        if def.head != HeadKind::Single {
            out = texts
                .iter()
                .zip(labels)
                .map(|(t, l)| Example::new(t, 0, Target::Labels(l)))
                .collect();
        }
        Ok(out)
    }

    // ---- training

    /// snapshots what a training run needs
    pub fn prepare(&self, name: &str, a: &TrainArgs) -> StoreResult<Training> {
        let mut stack = self.stack(name)?;
        let def = stack.def().clone();
        for d in &stack.defs[..stack.node] {
            if !d.trained() {
                return Err(format!("{name:?} builds on {:?}, which isn't trained yet", d.name));
            }
        }
        if a.fresh || !def.trained() {
            let extra = stack.clf().net.cfg.extra;
            stack.graph.nodes[stack.node].clf = def.build(extra)?;
        }
        let mut data = self.examples(&def)?;
        if data.is_empty() {
            return Err("its lists have no texts to learn from".into());
        }
        let mut o = def.train.clone();
        o.epochs = a.epochs.unwrap_or(o.epochs).clamp(1, 1000);
        o.lr = a.lr.unwrap_or(o.lr);
        if !(o.lr > 0.0 && o.lr <= 1.0) {
            return Err("a learning rate is above 0 and at most 1".into());
        }
        o.batch = a.batch.unwrap_or(o.batch).clamp(1, MAX_BATCH);
        o.max_steps = a.max_steps.unwrap_or(o.max_steps);
        // more threads than the machine has only slows it (and each is a real thread)
        o.threads = a.threads.unwrap_or(o.threads).min(crate::classify::pool::available());
        o.seed = crate::classify::math::key(&[def.seed, def.version]);
        let device_note = match a.device.check() {
            Ok(()) => {
                o.device = a.device;
                None
            }
            Err(e) => {
                o.device = crate::classify::device::Device::Cpu;
                Some(format!("no GPU here ({e}): trained on the CPU"))
            }
        };
        if stack.node > 0 {
            let pool = o.pool();
            data = stack.graph.with_extras(stack.node, &data, &pool);
        }
        Ok(Training {
            stack,
            data,
            opts: o,
            rounds: a.rounds.unwrap_or(1).clamp(1, 50),
            from_version: def.version,
            device_note,
        })
    }

    /// writes a finished run's weights and its history (keeping steering and anything
    /// else changed meanwhile)
    pub fn commit(&self, t: &Training, s: &Summary) -> StoreResult<Definition> {
        let name = &t.stack.def().name;
        let mut def = self.classifier(name)?;
        if def.version != t.from_version {
            return Err(format!("{name:?} changed while it trained; train it again"));
        }
        let time = crate::note::now();
        for g in &s.growth {
            def.push(Event {
                time: time.clone(),
                kind: "growth".into(),
                note: g.clone(),
                ..Event::default()
            });
        }
        def.push(Event {
            time,
            kind: "train".into(),
            note: format!(
                "{} steps on {} texts{}",
                s.steps,
                s.examples,
                if s.stopped_early { ", stopped early" } else { "" }
            ),
            metrics: s.metrics.clone(),
            measured_on: s.measured_on.clone(),
            examples: s.examples,
            steps: s.steps,
            seconds: s.seconds,
        });
        self.put_classifier(&mut def, Some(t.stack.clf()))?;
        // the yes/no answers before it can't be taken back past new weights
        self.drop_undo(name)?;
        Ok(def)
    }

    /// prepare, run and commit in one go (for callers that may block)
    pub fn train_classifier(
        &self,
        name: &str,
        a: &TrainArgs,
        cb: impl FnMut(&Tick) -> Flow,
    ) -> StoreResult<(Definition, Summary)> {
        let mut t = self.prepare(name, a)?;
        let s = t.run(cb)?;
        if s.cancelled {
            return Err("cancelled".into());
        }
        Ok((self.commit(&t, &s)?, s))
    }

    // ---- steering

    /// replaces the steering (the head's bias, temperature, threshold, and the gains)
    pub fn steer_classifier(&self, name: &str, steer: Steer) -> StoreResult<Definition> {
        let mut def = self.classifier(name)?;
        let k = def.class_names().len();
        check_steer(&steer, k)?;
        def.steer = steer;
        self.put_classifier(&mut def, None)?;
        Ok(def)
    }

    // ---- feedback

    fn log_feedback(&self, e: &FeedbackEntry) -> StoreResult<u64> {
        let txn = self.db.begin_write().map_err(err)?;
        let n = {
            let mut t = txn.open_table(FEEDBACK).map_err(err)?;
            let name = e.classifier.as_str();
            let next = t
                .range((name, 0)..=(name, u64::MAX))
                .map_err(err)?
                .next_back()
                .transpose()
                .map_err(err)?
                .map_or(0, |(k, _)| k.value().1 + 1);
            let json = serde_json::to_vec(e).map_err(err)?;
            t.insert((name, next), json.as_slice()).map_err(err)?;
            next
        };
        txn.commit().map_err(err)?;
        Ok(n)
    }

    /// the classifiers `names` as they are now, for an [`Undo`]
    fn snapshot(&self, names: &[String]) -> StoreResult<Vec<(String, String, Option<String>)>> {
        let txn = self.db.begin_read().map_err(err)?;
        let d = txn.open_table(DEFINITIONS).map_err(err)?;
        let w = txn.open_table(WEIGHTS).map_err(err)?;
        let mut out = Vec::new();
        for n in names {
            let Some(def) = d.get(n.as_str()).map_err(err)? else {
                continue;
            };
            let def = String::from_utf8_lossy(def.value()).into_owned();
            let weights = w.get(n.as_str()).map_err(err)?.map(|g| base64(g.value()));
            out.push((n.clone(), def, weights));
        }
        Ok(out)
    }

    /// keeps what a yes/no on `name` changed, the oldest beyond [`UNDO_KEPT`] dropped
    fn keep_undo(&self, name: &str, u: &Undo) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(UNDO).map_err(err)?;
            let next = t
                .range((name, 0)..=(name, u64::MAX))
                .map_err(err)?
                .next_back()
                .transpose()
                .map_err(err)?
                .map_or(0, |(k, _)| k.value().1 + 1);
            let json = serde_json::to_vec(u).map_err(err)?;
            t.insert((name, next), json.as_slice()).map_err(err)?;
            if next >= UNDO_KEPT {
                let cut = next - UNDO_KEPT;
                t.retain_in((name, 0)..=(name, cut), |_, _| false).map_err(err)?;
            }
        }
        txn.commit().map_err(err)
    }

    /// forgets what could be taken back on `name`, and every yes/no through a stack
    /// that wrote it (its weights changed another way)
    pub fn drop_undo(&self, name: &str) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_table(UNDO)
            .map_err(err)?
            .retain(|_, v| serde_json::from_slice::<Undo>(v).is_ok_and(|u| u.was.iter().all(|w| w.0 != name)))
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    /// how many yes/no answers on `name` can be taken back
    pub fn undo_count(&self, name: &str) -> StoreResult<usize> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(UNDO).map_err(err)?;
        Ok(t.range((name, 0)..=(name, u64::MAX)).map_err(err)?.count())
    }

    /// takes back the latest yes/no on `name`: every classifier it wrote gets its
    /// definition and weights back as they were, and the feedback leaves the log
    /// (so training no longer replays it); the entry taken back
    pub fn unfeedback(&self, name: &str) -> StoreResult<FeedbackEntry> {
        self.classifier(name)?;
        let txn = self.db.begin_write().map_err(err)?;
        let entry = {
            let mut t = txn.open_table(UNDO).map_err(err)?;
            let last = t
                .range((name, 0)..=(name, u64::MAX))
                .map_err(err)?
                .next_back()
                .transpose()
                .map_err(err)?
                .map(|(k, v)| (k.value().1, v.value().to_vec()));
            let Some((n, bytes)) = last else {
                return Err(format!("there's no yes or no on {name:?} to take back"));
            };
            t.remove((name, n)).map_err(err)?;
            let u: Undo = serde_json::from_slice(&bytes).map_err(err)?;
            let mut d = txn.open_table(DEFINITIONS).map_err(err)?;
            let mut w = txn.open_table(WEIGHTS).map_err(err)?;
            for (n, def, weights) in &u.was {
                d.insert(n.as_str(), def.as_bytes()).map_err(err)?;
                match weights.as_deref().map(unbase64) {
                    Some(Some(b)) => {
                        w.insert(n.as_str(), b.as_slice()).map_err(err)?;
                    }
                    Some(None) => return Err("a kept copy of the weights doesn't read".into()),
                    None => {
                        w.remove(n.as_str()).map_err(err)?;
                    }
                }
            }
            let mut f = txn.open_table(FEEDBACK).map_err(err)?;
            let e = f.remove((name, u.feedback)).map_err(err)?.map(|g| g.value().to_vec());
            match e {
                Some(e) => serde_json::from_slice::<FeedbackEntry>(&e).map_err(err)?,
                None => return Err("that yes or no isn't in the log any more".into()),
            }
        };
        txn.commit().map_err(err)?;
        // the texts it filed under the classifier answer as before, so the memo stays
        Ok(entry)
    }

    /// a classifier's feedback log, oldest first
    pub fn classifier_feedback(&self, name: &str) -> StoreResult<Vec<FeedbackEntry>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(FEEDBACK).map_err(err)?;
        let mut out = Vec::new();
        for item in t.range((name, 0)..=(name, u64::MAX)).map_err(err)? {
            let (_, v) = item.map_err(err)?;
            out.push(serde_json::from_slice(v.value()).map_err(err)?);
        }
        Ok(out)
    }

    /// texts from the classifier's lists to replay beside feedback, so one yes or no
    /// doesn't drag the rest along
    fn replay(&self, def: &Definition, n: usize) -> StoreResult<Vec<Example>> {
        let all = self.examples(def)?;
        let step = (all.len() / n.max(1)).max(1);
        Ok(all.into_iter().step_by(step).take(n).collect())
    }

    /// yes or no on one classifier's own weights: a few small steps, kept in its
    /// feedback for replay, and logged
    pub fn feedback_classifier(
        &self,
        name: &str,
        j: &Judged,
        class: &str,
        yes: bool,
        weight: f32,
    ) -> StoreResult<FeedbackEntry> {
        let mut stack = self.stack(name)?;
        let mut def = stack.def().clone();
        if !def.trained() {
            return Err(format!("train {name:?} before telling it yes or no"));
        }
        let c = def.class_index(class)?;
        let node = stack.node;
        let extra = if node > 0 {
            stack.graph.extra(node, &j.text)
        } else {
            vec![]
        };
        let mut replay = self.replay(&def, 24)?;
        if node > 0 {
            let pool = Pool::new(crate::classify::pool::available());
            replay = stack.graph.with_extras(node, &replay, &pool);
        }
        let fb = Feedback {
            weight,
            extra,
            ..Feedback::new(&j.text, c, yes)
        };
        let clf = &mut stack.graph.nodes[node].clf;
        let rep = clf.feedback(
            std::slice::from_ref(&fb),
            &replay,
            &FeedbackOpts {
                steps: 6,
                lr: 1e-3,
                seed: def.seed,
                ..FeedbackOpts::default()
            },
        )?;
        let e = FeedbackEntry {
            classifier: def.name.clone(),
            key: j.key.as_ref().map(Ref::label),
            text: j.text.clone(),
            class: def.class_names()[c].clone(),
            yes,
            weight,
            time: crate::note::now(),
            stack: false,
            before: rep.before[0],
            after: rep.after[0],
        };
        let was = self.snapshot(std::slice::from_ref(&def.name))?;
        self.put_classifier(&mut def, Some(clf))?;
        let feedback = self.log_feedback(&e)?;
        self.keep_undo(&def.name, &Undo { feedback, was })?;
        Ok(e)
    }

    /// yes or no end to end: through the classifiers it builds on, each link scaled by
    /// its coupling; every classifier reached is written back
    pub fn feedback_stack(
        &self,
        name: &str,
        j: &Judged,
        class: &str,
        yes: bool,
        weight: f32,
    ) -> StoreResult<(FeedbackEntry, StackReport, Vec<String>)> {
        let mut stack = self.stack(name)?;
        let def = stack.def().clone();
        if stack.defs.iter().any(|d| !d.trained()) {
            return Err("train every classifier in the stack first".into());
        }
        let c = def.class_index(class)?;
        let fb = Feedback {
            weight,
            ..Feedback::new(&j.text, c, yes)
        };
        let rep = stack.graph.feedback(
            stack.node,
            &fb,
            &FeedbackOpts {
                steps: 6,
                lr: 1e-3,
                seed: def.seed,
                ..FeedbackOpts::default()
            },
        )?;
        let mut touched = Vec::new();
        let was = self.snapshot(&stack.names())?;
        for (i, d) in stack.defs.iter().enumerate() {
            if rep.grad_norms[i] > 0.0 || i == stack.node {
                let mut d = self.classifier(&d.name)?;
                if i == stack.node {
                    d.push(Event {
                        time: crate::note::now(),
                        kind: "feedback stack".into(),
                        note: format!("{} on {:?}", if yes { "yes" } else { "no" }, def.class_names()[c]),
                        ..Event::default()
                    });
                }
                self.put_classifier(&mut d, Some(&stack.graph.nodes[i].clf))?;
                touched.push(d.name.clone());
            }
        }
        let e = FeedbackEntry {
            classifier: def.name.clone(),
            key: j.key.as_ref().map(Ref::label),
            text: j.text.clone(),
            class: def.class_names()[c].clone(),
            yes,
            weight,
            time: crate::note::now(),
            stack: true,
            before: rep.before,
            after: rep.after,
        };
        let feedback = self.log_feedback(&e)?;
        let was = was.into_iter().filter(|w| touched.contains(&w.0)).collect();
        self.keep_undo(&def.name, &Undo { feedback, was })?;
        Ok((e, rep, touched))
    }

    // ---- outputs as lists

    /// writes a class's members (texts at or above `threshold`, the steered threshold
    /// if none) as a list headed by a memory "<classifier>: <class>", replacing what
    /// that list held; returns the head and the member count
    pub fn enlist_outputs(
        &self,
        name: &str,
        class: &str,
        threshold: Option<f32>,
        limit: usize,
    ) -> StoreResult<(Ref, usize)> {
        let stack = self.stack(name)?;
        let mut def = stack.def().clone();
        if !def.trained() {
            return Err(format!("train {name:?} first"));
        }
        let c = def.class_index(class)?;
        let cname = def.class_names()[c].clone();
        let threshold = threshold.unwrap_or(def.steer.heads[0].threshold);
        let (head, _) = self.saturate(&format!("{}: {}", def.name, cname))?;
        let head = Ref::Memory(head);
        let cands: Vec<(Ref, String, String)> = self
            .candidates(limit)?
            .into_iter()
            .filter(|(r, ..)| r != &head)
            .collect();
        let texts: Vec<&str> = cands.iter().map(|c| c.1.as_str()).collect();
        let pool = Pool::new(crate::classify::pool::available());
        let outs = stack.outputs(&texts, &pool);
        let pairs: Vec<(Ref, Ref)> = cands
            .iter()
            .zip(&outs)
            .filter(|(_, o)| o.probs[c] >= threshold)
            .map(|(cand, _)| (head.clone(), cand.0.clone()))
            .collect();
        self.unlist(&head)?;
        self.enlist_many(&pairs)?;
        def.outputs.insert(cname.clone(), head.label());
        def.push(Event {
            time: crate::note::now(),
            kind: "enlist".into(),
            note: format!("{} texts to {:?} at {threshold:.2}", pairs.len(), cname),
            ..Event::default()
        });
        self.put_classifier(&mut def, None)?;
        Ok((head, pairs.len()))
    }
}

/// a note as a classifier reads it
pub fn note_text(n: &Note) -> String {
    if n.markdown.trim().is_empty() {
        n.title.clone()
    } else {
        format!("{}\n\n{}", n.title, n.markdown)
    }
}

// ---------------------------------------------------------------- bundles

/// a classifier in a bundle: its definition, its weights (the `ANENDCLS` file in
/// standard base64, padded) and its feedback log
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bundled {
    pub definition: Definition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feedback: Vec<FeedbackEntry>,
    /// the classifiers that validate it (`filters::Validator`)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validators: Vec<crate::filters::Validator>,
}

impl Store {
    pub fn export_classifiers(&self) -> StoreResult<Vec<Bundled>> {
        self.classifiers()?
            .into_iter()
            .map(|d| {
                Ok(Bundled {
                    weights: self.classifier_weights(&d.name)?.map(|w| base64(&w)),
                    feedback: self.classifier_feedback(&d.name)?,
                    validators: self.validators(&d.name)?,
                    definition: d,
                })
            })
            .collect()
    }

    /// writes bundled classifiers as they are (their lists keep their keys on import)
    pub fn import_classifiers(&self, all: &[Bundled]) -> StoreResult<usize> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut defs = txn.open_table(DEFINITIONS).map_err(err)?;
            let mut ws = txn.open_table(WEIGHTS).map_err(err)?;
            let mut fb = txn.open_table(FEEDBACK).map_err(err)?;
            for b in all {
                let name = b.definition.name.as_str();
                if let Some(w) = &b.weights {
                    let bytes = unbase64(w).ok_or_else(|| format!("{name:?}: its weights aren't base64"))?;
                    Classifier::from_bytes(&bytes).map_err(|e| format!("{name:?}: {e}"))?;
                    ws.insert(name, bytes.as_slice()).map_err(err)?;
                }
                let json = serde_json::to_vec(&b.definition).map_err(err)?;
                defs.insert(name, json.as_slice()).map_err(err)?;
                for (i, e) in b.feedback.iter().enumerate() {
                    let json = serde_json::to_vec(e).map_err(err)?;
                    fb.insert((name, i as u64), json.as_slice()).map_err(err)?;
                }
            }
        }
        txn.commit().map_err(err)?;
        for b in all {
            self.memo_clear(&b.definition.name)?;
        }
        // validators after every classifier is in, since they name each other
        for b in all.iter().filter(|b| !b.validators.is_empty()) {
            self.set_validators(&b.definition.name, &b.validators)?;
        }
        Ok(all.len())
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// standard base64 with padding
pub fn base64(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for ch in s.bytes() {
        let v = B64.iter().position(|&x| x == ch)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// new definitions build without gematria; ones saved before the field existed
    /// keep the channels they were made with
    #[test]
    fn gematria_is_opt_in_and_old_definitions_keep_it() {
        let classes = vec![
            ClassDef {
                name: "a".into(),
                list: "a".into(),
            },
            ClassDef {
                name: "b".into(),
                list: "b".into(),
            },
        ];
        let d = Definition::new("x", classes, None, HeadKind::Single, DEFAULT_PRESET).unwrap();
        assert!(!d.gematria);
        let c = d.build(0).unwrap();
        assert!(!c.net.cfg.letters && !c.net.cfg.words && !c.net.cfg.side);
        let mut v = serde_json::to_value(&d).unwrap();
        v.as_object_mut().unwrap().remove("gematria");
        let old: Definition = serde_json::from_value(v).unwrap();
        assert!(old.gematria);
        let c = old.build(0).unwrap();
        assert!(c.net.cfg.letters && c.net.cfg.words && c.net.cfg.side);
    }

    pub fn temp_store(name: &str) -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("anend-clf-{}-{name}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        (Store::open(&path).unwrap(), path)
    }

    const FRUIT: [&str; 10] = [
        "apple", "banana", "cherry", "mango", "peach", "plum", "grape", "melon", "kiwi", "lemon",
    ];
    const ROCK: [&str; 10] = [
        "rock", "stone", "granite", "basalt", "flint", "slate", "quartz", "shale", "marble", "gneiss",
    ];

    /// two lists of short texts, and a note in the first
    pub fn lists(s: &Store) -> (Ref, Ref) {
        let k = |t: &str| Ref::Memory(s.saturate(t).unwrap().0);
        let (f, r) = (k("fruit"), k("rocks"));
        for i in 0..24 {
            let a = format!(
                "{} {} {}",
                FRUIT[i % 10],
                FRUIT[(i * 3 + 1) % 10],
                FRUIT[(i / 10 + 5) % 10]
            );
            let b = format!(
                "{} {} {}",
                ROCK[i % 10],
                ROCK[(i * 7 + 2) % 10],
                ROCK[(i / 10 + 5) % 10]
            );
            s.enlist(&f, &k(&a)).unwrap();
            s.enlist(&r, &k(&b)).unwrap();
        }
        let n = s
            .save(&Note {
                title: "orchard".into(),
                markdown: "plum and cherry".into(),
                ..Default::default()
            })
            .unwrap();
        s.enlist(&f, &Ref::Note(n.key)).unwrap();
        (f, r)
    }

    #[test]
    fn base64_round_trip() {
        for n in 0..10 {
            let b: Vec<u8> = (0..n as u8).map(|x| x.wrapping_mul(97)).collect();
            assert_eq!(unbase64(&base64(&b)).unwrap(), b);
        }
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert!(unbase64("**").is_none());
    }

    #[test]
    fn definitions_weights_feedback_and_bundles() {
        let (s, path) = temp_store("store");
        let (f, r) = lists(&s);
        assert!(
            s.create_classifier("x", std::slice::from_ref(&f), None, HeadKind::Single, "tiny", vec![])
                .is_err()
        );
        let def = s
            .create_classifier("kind", &[f.clone(), r.clone()], None, HeadKind::Single, "tiny", vec![])
            .unwrap();
        assert_eq!(def.class_names(), vec!["fruit", "rocks"]);
        assert!(
            s.create_classifier("kind", &[f.clone(), r.clone()], None, HeadKind::Single, "tiny", vec![])
                .is_err()
        );
        let ex = s.examples(&def).unwrap();
        assert_eq!(ex.len(), 49);
        assert!(ex.iter().any(|e| e.text == "orchard\n\nplum and cherry"));
        assert_eq!(s.classifier("kind").unwrap(), def);
        assert!(s.classifier_weights("kind").unwrap().is_none());

        // a short run, then everything back out
        let a = TrainArgs {
            epochs: Some(2),
            threads: Some(2),
            ..TrainArgs::default()
        };
        let (def, sum) = s.train_classifier("kind", &a, |_| Flow::Go).unwrap();
        assert_eq!(def.version, 1);
        assert!(sum.metrics.is_some() && sum.steps > 0);
        assert_eq!(def.last_training().unwrap().examples, 49);
        let w = s.classifier_weights("kind").unwrap().unwrap();
        let st = s.stack("kind").unwrap();
        assert_eq!(st.clf().to_bytes(), w);

        let mut steer = def.steer.clone();
        steer.heads[0].bias = vec![0.5, -0.5];
        let def = s.steer_classifier("kind", steer.clone()).unwrap();
        assert_eq!(s.stack("kind").unwrap().clf().steer, steer);
        assert_eq!(def.version, 1, "steering leaves the weights alone");

        let j = Judged {
            key: None,
            text: "apple granite".into(),
        };
        // a firm yes: the replayed list members outweigh one feedback of weight 1 on this
        // byte-only (no gematria) tiny model
        let e = s.feedback_classifier("kind", &j, "rocks", true, 2.0).unwrap();
        assert!(e.after > e.before, "{e:?}");
        s.feedback_classifier("kind", &j, "rocks", false, 1.0).unwrap();
        let log = s.classifier_feedback("kind").unwrap();
        assert_eq!(log.len(), 2);
        assert!(log[0].yes && !log[1].yes);
        assert_eq!(s.classifier("kind").unwrap().version, 3);

        // a bundle carries them, and old bundles without them still read
        let b = crate::bundle::export(&s, "store").unwrap();
        assert_eq!(b.classifiers.len(), 1);
        let json = serde_json::to_string(&b).unwrap();
        let back: crate::bundle::Bundle = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
        let (c, pc) = temp_store("copy");
        crate::bundle::import(&c, &back).unwrap();
        assert_eq!(c.classifier("kind").unwrap(), s.classifier("kind").unwrap());
        assert_eq!(
            c.classifier_weights("kind").unwrap(),
            s.classifier_weights("kind").unwrap()
        );
        assert_eq!(c.classifier_feedback("kind").unwrap(), log);
        let mut old: serde_json::Value = serde_json::to_value(&b).unwrap();
        old.as_object_mut().unwrap().remove("classifiers");
        let old: crate::bundle::Bundle = serde_json::from_value(old).unwrap();
        assert!(old.classifiers.is_empty());
        assert!(!serde_json::to_string(&old).unwrap().contains("classifiers"));

        s.delete_classifier("kind").unwrap();
        assert!(s.classifier("kind").is_err());
        assert!(s.classifier_feedback("kind").unwrap().is_empty());
        assert!(s.classifier_weights("kind").unwrap().is_none());
        drop((s, c));
        for p in [path, pc] {
            let _ = std::fs::remove_file(p);
        }
    }

    /// training arguments from a caller are kept in bounds, and steering is checked
    #[test]
    fn hostile_arguments() {
        let (s, path) = temp_store("hostile");
        let (f, r) = lists(&s);
        let def = s
            .create_classifier("kind", &[f, r], None, HeadKind::Single, "tiny", vec![])
            .unwrap();
        for lr in [-1.0, 0.0, 5.0] {
            let a = TrainArgs {
                lr: Some(lr),
                ..TrainArgs::default()
            };
            assert!(s.prepare("kind", &a).is_err(), "lr {lr}");
        }
        let a = TrainArgs {
            threads: Some(1 << 20),
            batch: Some(usize::MAX),
            epochs: Some(usize::MAX),
            rounds: Some(usize::MAX),
            ..TrainArgs::default()
        };
        let t = s.prepare("kind", &a).unwrap();
        assert!(t.opts.threads <= crate::classify::pool::available());
        assert_eq!((t.opts.batch, t.opts.epochs, t.rounds), (MAX_BATCH, 1000, 50));
        let mut bad = def.steer.clone();
        bad.heads[0].temperature = 0.0;
        assert!(check_steer(&bad, 2).is_err() && s.steer_classifier("kind", bad).is_err());
        let mut bad = def.steer.clone();
        bad.heads[0].bias = vec![0.0; 3];
        assert!(check_steer(&bad, 2).is_err());
        assert!(check_steer(&def.steer, 2).is_ok());
        drop(s);
        let _ = std::fs::remove_file(path);
    }

    /// presets referring to presets many times over run once each, and a loop brought in
    /// by a bundle is an error rather than a hang
    #[test]
    fn filter_presets_stay_cheap() {
        use crate::filters::{Expr, Preset};
        let (s, path) = temp_store("presets");
        lists(&s);
        let pool = Pool::new(1);
        s.save_filter("p0", Expr::Words { q: "plum".into() }, "").unwrap();
        for i in 1..=10 {
            let of = vec![
                Expr::Preset {
                    name: format!("p{}", i - 1)
                };
                200
            ];
            s.save_filter(&format!("p{i}"), Expr::And { of }, "").unwrap();
        }
        let t = std::time::Instant::now();
        let r = s
            .run_filter(&Expr::Preset { name: "p10".into() }, 10, 10_000, &pool)
            .unwrap();
        assert!(r.matched > 0);
        assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());

        let p = |name: &str, to: &str| Preset {
            name: name.into(),
            expr: Expr::And {
                of: vec![Expr::Preset { name: to.into() }; 200],
            },
            description: String::new(),
            created: String::new(),
            modified: String::new(),
            list: None,
            count: None,
        };
        s.import_filters(&[p("ping", "pong"), p("pong", "ping")]).unwrap();
        let e = s
            .run_filter(&Expr::Preset { name: "ping".into() }, 10, 10_000, &pool)
            .unwrap_err();
        assert!(e.contains("comes back") || e.contains("include itself"), "{e}");
        let big = Preset {
            expr: Expr::Or {
                of: vec![Expr::Words { q: "x".into() }; 300],
            },
            ..p("big", "p0")
        };
        assert!(s.import_filters(&[big]).is_err());
        drop(s);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stacks_and_outputs_as_lists() {
        let (s, path) = temp_store("stack");
        let (f, r) = lists(&s);
        s.create_classifier("kind", &[f.clone(), r.clone()], None, HeadKind::Single, "tiny", vec![])
            .unwrap();
        let input = Input {
            from: "kind".into(),
            coupling: 0.5,
        };
        s.create_classifier(
            "again",
            &[f.clone(), r.clone()],
            None,
            HeadKind::Multi,
            "tiny",
            vec![input],
        )
        .unwrap();
        let a = TrainArgs {
            epochs: Some(3),
            threads: Some(2),
            ..TrainArgs::default()
        };
        assert!(s.prepare("again", &a).is_err(), "upstream untrained");
        s.train_classifier("kind", &a, |_| Flow::Go).unwrap();
        s.train_classifier("again", &a, |_| Flow::Go).unwrap();
        let st = s.stack("again").unwrap();
        assert_eq!(st.names(), vec!["kind", "again"]);
        assert_eq!(st.clf().net.cfg.extra, 2);
        let cs = st.contributions("granite slate", 1).unwrap();
        assert_eq!(cs.len(), 2);
        assert!(s.delete_classifier("kind").is_err(), "again builds on it");

        let j = Judged {
            key: None,
            text: "granite plum".into(),
        };
        let (_, rep, touched) = s.feedback_stack("again", &j, "rocks", true, 1.0).unwrap();
        assert!(rep.after > rep.before);
        assert_eq!(touched, vec!["kind", "again"]);
        assert_eq!(s.classifier("kind").unwrap().version, 2);

        let (head, n) = s.enlist_outputs("kind", "rocks", Some(0.5), 10_000).unwrap();
        assert_eq!(s.remember(&head).unwrap().text, "kind: rocks");
        assert_eq!(s.list_members(&head).unwrap().len(), n);
        assert!(n > 0);
        assert_eq!(s.classifier("kind").unwrap().outputs["rocks"], head.label());
        drop(s);
        let _ = std::fs::remove_file(path);
    }

    /// `cargo test --release -p anend-core timing -- --ignored --nocapture`: the default
    /// preset on three lists of 50 short memories
    #[test]
    #[ignore]
    fn timing() {
        let (s, path) = temp_store("timing");
        let mut heads = vec![];
        for (n, ws) in [
            ("fruit", FRUIT),
            ("rocks", ROCK),
            (
                "more",
                [
                    "alpha", "beta", "gamma", "delta", "omega", "sigma", "theta", "kappa", "lambda", "zeta",
                ],
            ),
        ] {
            let h = Ref::Memory(s.saturate(n).unwrap().0);
            for i in 0..50 {
                let t = format!("{} {} {}", ws[i % 10], ws[(i * 7 + 3) % 10], ws[(i / 10 + 5) % 10]);
                s.enlist(&h, &Ref::Memory(s.saturate(&t).unwrap().0)).unwrap();
            }
            heads.push(h);
        }
        s.create_classifier("t", &heads, None, HeadKind::Single, DEFAULT_PRESET, vec![])
            .unwrap();
        let t0 = std::time::Instant::now();
        let (_, sum) = s.train_classifier("t", &TrainArgs::default(), |_| Flow::Go).unwrap();
        println!(
            "{} texts, {} steps, {:.2} s, {:.0} tokens/s, accuracy {:?}",
            sum.examples,
            sum.steps,
            t0.elapsed().as_secs_f64(),
            sum.tokens_per_sec,
            sum.metrics.map(|m| m.accuracy)
        );
        drop(s);
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod phase3_tests {
    use super::tests::{lists, temp_store};
    use super::*;
    use crate::filters::{Expr, Mode, Validator};
    use crate::memo::{By, UNCERTAIN};

    #[test]
    fn memo_filters_and_modules() {
        let (s, path) = temp_store("phase3");
        let (f, r) = lists(&s);
        s.create_classifier("kind", &[f.clone(), r.clone()], None, HeadKind::Single, "tiny", vec![])
            .unwrap();
        let a = TrainArgs {
            epochs: Some(4),
            threads: Some(2),
            ..TrainArgs::default()
        };
        s.train_classifier("kind", &a, |_| Flow::Go).unwrap();
        let pool = Pool::new(2);
        let st = s.stack("kind").unwrap();

        // indexing: deterministic, matches a direct read, buckets filled
        let rep = s.memo_index(&st, None, 10_000, &pool, |_, _| Flow::Go).unwrap();
        assert!(rep.computed > 0 && rep.cached == 0);
        let again = s.memo_index(&st, None, 10_000, &Pool::new(1), |_, _| Flow::Go).unwrap();
        assert_eq!(again.computed, 0, "memoized");
        let all = s.memo_all(&st).unwrap();
        assert_eq!(all.len(), rep.total);
        let (r0, row0) = &all[0];
        let t0 = s.ref_text(r0).unwrap().unwrap();
        assert_eq!(&crate::memo::read(&st, row0.stamp, &t0), row0);
        assert_eq!(row0.embedding, st.clf().represent(&t0));
        let mut filed = 0;
        for c in [0, 1, UNCERTAIN] {
            filed += s.class_bucket(&st, c, 0..=9).unwrap().0.len();
        }
        assert_eq!(filed, all.len());
        let near = s.memo_near(&st, row0, By::Embedding, Some(r0), 3).unwrap();
        assert_eq!(near.len(), 3);
        assert!(near.windows(2).all(|w| w[0].2 >= w[1].2));

        // a steering change invalidates
        let mut steer = st.def().steer.clone();
        steer.heads[0].bias = vec![1.0, -1.0];
        s.steer_classifier("kind", steer).unwrap();
        let st2 = s.stack("kind").unwrap();
        assert_ne!(crate::memo::stamp(&st), crate::memo::stamp(&st2));
        assert_eq!(s.memo_count(&st2).unwrap(), 0);
        assert!(s.memo_all(&st2).unwrap().is_empty());

        // filters: class, not, words; presets referring to presets; loops refused
        let fruit = Expr::Class {
            classifier: "kind".into(),
            class: "fruit".into(),
            min: Some(0.5),
            max: None,
            top: false,
        };
        let res = s.run_filter(&fruit, 1000, 10_000, &pool).unwrap();
        let not = Expr::Not {
            of: Box::new(fruit.clone()),
        };
        let res2 = s.run_filter(&not, 1000, 10_000, &pool).unwrap();
        assert_eq!(res.matched + res2.matched, res.scanned);
        let and = Expr::And {
            of: vec![fruit.clone(), Expr::Words { q: "plum".into() }],
        };
        let res3 = s.run_filter(&and, 1000, 10_000, &pool).unwrap();
        // an empty "all of" is everything; an empty "any of" is refused
        let every = s.run_filter(&Expr::And { of: vec![] }, 1000, 10_000, &pool).unwrap();
        assert_eq!(every.matched, every.scanned);
        assert!(s.run_filter(&Expr::Or { of: vec![] }, 1000, 10_000, &pool).is_err());
        assert!(res3.matched <= res.matched);
        assert!(res3.results.iter().all(|x| x.note || x.text.contains("plum")));
        s.save_filter("fruity", fruit.clone(), "").unwrap();
        s.save_filter("outer", Expr::Preset { name: "fruity".into() }, "")
            .unwrap();
        assert_eq!(
            s.run_filter(&Expr::Preset { name: "outer".into() }, 1000, 10_000, &pool)
                .unwrap()
                .matched,
            res.matched
        );
        assert!(
            s.save_filter("fruity", Expr::Preset { name: "outer".into() }, "")
                .is_err(),
            "a loop"
        );
        assert!(s.delete_filter("fruity").is_err(), "outer uses it");
        let (head, n) = s.enlist_filter("outer", 10_000, &pool).unwrap();
        assert_eq!(s.list_members(&head).unwrap().len(), n);

        // a validator gating everything out
        s.create_classifier("judge", &[f.clone(), r.clone()], None, HeadKind::Single, "tiny", vec![])
            .unwrap();
        s.train_classifier("judge", &a, |_| Flow::Go).unwrap();
        assert!(
            s.set_validators(
                "kind",
                &[Validator {
                    by: "kind".into(),
                    class: "fruit".into(),
                    min: 0.5,
                    mode: Mode::Gate
                }]
            )
            .is_err()
        );
        s.set_validators(
            "kind",
            &[Validator {
                by: "judge".into(),
                class: "fruit".into(),
                min: 1.1f32.min(1.0),
                mode: Mode::Gate,
            }],
        )
        .unwrap();
        let gated = s.run_filter(&fruit, 1000, 10_000, &pool).unwrap();
        assert!(gated.matched <= res.matched);
        assert!(!s.disagreements("kind", 100, 10_000, &pool).unwrap().is_empty() || gated.matched == 0);

        // modules: round trip into another save, corrupt input refused
        let bytes = s
            .export_module("kind", &crate::clsmodule::ExportOpts::default())
            .unwrap();
        let m = crate::clsmodule::read(&bytes, crate::clsmodule::MAX_SIZE).unwrap();
        assert_eq!(m.manifest.main().classes, vec!["fruit", "rocks"]);
        let (c, pc) = temp_store("phase3b");
        let got = c.import_module(&m, &crate::clsmodule::ImportOpts::default()).unwrap();
        assert_eq!(got.name, "kind");
        assert_eq!(got.lists.len(), 2);
        let mine = s.stack("kind").unwrap();
        let theirs = c.stack("kind").unwrap();
        assert_eq!(
            mine.graph.predict(mine.node, "granite plum"),
            theirs.graph.predict(theirs.node, "granite plum")
        );
        let again = c.import_module(&m, &crate::clsmodule::ImportOpts::default()).unwrap();
        assert_eq!(again.reused, vec!["kind"]);
        for i in [0, 9, 20, bytes.len() / 2, bytes.len() - 1] {
            let mut bad = bytes.clone();
            bad[i] ^= 0x41;
            assert!(
                crate::clsmodule::read(&bad, crate::clsmodule::MAX_SIZE).is_err(),
                "byte {i}"
            );
        }
        assert!(crate::clsmodule::read(&bytes[..bytes.len() - 9], crate::clsmodule::MAX_SIZE).is_err());
        assert!(crate::clsmodule::read(b"junk", crate::clsmodule::MAX_SIZE).is_err());
        assert!(crate::clsmodule::read(&bytes, 100).is_err());
        // a weights file whose header claims huge shapes is refused before allocating
        let w = s.classifier_weights("kind").unwrap().unwrap();
        let hl = u32::from_le_bytes(w[12..16].try_into().unwrap()) as usize;
        let h = String::from_utf8(w[16..16 + hl].to_vec())
            .unwrap()
            .replacen("\"d_model\":16", "\"d_model\":99999", 1);
        let mut forged = w[..12].to_vec();
        forged.extend_from_slice(&(h.len() as u32).to_le_bytes());
        forged.extend_from_slice(h.as_bytes());
        forged.extend_from_slice(&w[16 + hl..w.len() - 8]);
        let sum = crate::classify::math::fnv(0, &forged);
        forged.extend_from_slice(&sum.to_le_bytes());
        assert!(Classifier::from_bytes(&forged).is_err());

        s.delete_classifier("kind").unwrap();
        drop((s, c));
        for p in [path, pc] {
            let _ = std::fs::remove_file(p);
        }
    }
}
