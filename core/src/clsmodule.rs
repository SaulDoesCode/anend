//! A baked classifier on its own: a `.anendcls` module, to keep, share on a hub and
//! take into any save. It holds the classifier's weights and what it is (class names,
//! head, preset, steering, training metrics and history), and the classifiers it builds
//! on, each either bundled or referenced by its weights' hash (`Classifier::weights_hash`)
//! for a save that has it already. Classes are names, not lists: taking a module in maps
//! each class to a list by name (a new list, or the one headed by a memory of that text),
//! or to lists the caller picks.
//!
//! | bytes | holds |
//! |---|---|
//! | 8 | magic `ANENDCLM` |
//! | 4 | format version, u32 little-endian |
//! | 4 | manifest length m, u32 LE (at most 1 MiB) |
//! | m | manifest, JSON (`Manifest`): its parts upstream first, the classifier itself last |
//! | Σ | each bundled part's `ANENDCLS` file, in order, its length in the manifest |
//! | 8 | FNV-1a of everything before, u64 LE |
//!
//! Reading is strict, for files from anyone: sizes are checked before anything is
//! allocated, every part must parse and match its definition (classes, head, inputs'
//! widths, hash), names and numbers must be sane, and a panic anywhere is a bad file.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::classifiers::{ClassDef, Definition, Event, Input, clean_name, preset};
use crate::classify::math::fnv;
use crate::classify::metrics::Metrics;
use crate::classify::{Classifier, HeadKind, Steer, TrainOpts};
use crate::memory::Ref;
use crate::store::{Store, StoreResult};

pub const MAGIC: &[u8; 8] = b"ANENDCLM";
pub const VERSION: u32 = 1;
pub const EXTENSION: &str = "anendcls";
pub const MAX_MANIFEST: usize = 1 << 20;
pub const MAX_PARTS: usize = 16;
/// the largest module read by default (a hub may take less)
pub const MAX_SIZE: usize = 256 << 20;

/// a classifier's definition as a module carries it: classes by name, not by list
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    pub classes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not: Option<String>,
    pub head: HeadKind,
    pub preset: String,
    pub steer: Steer,
    #[serde(default)]
    pub inputs: Vec<Input>,
    pub train: TrainOpts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Metrics>,
    #[serde(default)]
    pub measured_on: String,
    #[serde(default)]
    pub examples: usize,
    /// training, growth and other events, the latest 50
    #[serde(default)]
    pub history: Vec<Event>,
    pub seed: u64,
    /// the weights' hash, hex
    pub hash: String,
    /// bytes of its `ANENDCLS` file in this module; 0 when it is only referenced
    pub size: u64,
    pub weights: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub anendcls: u32,
    /// the classifier itself (the last part)
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub created: String,
    pub parts: Vec<Part>,
}

impl Manifest {
    pub fn main(&self) -> &Part {
        self.parts.last().expect("a module has a part")
    }
}

/// a module read and checked
#[derive(Clone, Debug)]
pub struct Module {
    pub manifest: Manifest,
    /// each part's weights file, when bundled
    pub weights: Vec<Option<Vec<u8>>>,
}

pub fn hex(x: u64) -> String {
    format!("{x:016x}")
}

fn part_of(def: &Definition, clf: &Classifier, size: u64) -> Part {
    let last = def.last_training();
    let n = def.history.len();
    Part {
        name: def.name.clone(),
        classes: def.classes.iter().map(|c| c.name.clone()).collect(),
        not: def.not.as_ref().map(|c| c.name.clone()),
        head: def.head,
        preset: def.preset.clone(),
        steer: def.steer.clone(),
        inputs: def.inputs.clone(),
        train: def.train.clone(),
        metrics: last.and_then(|e| e.metrics.clone()),
        measured_on: last.map(|e| e.measured_on.clone()).unwrap_or_default(),
        examples: last.map(|e| e.examples).unwrap_or(0),
        history: def.history[n.saturating_sub(50)..].to_vec(),
        seed: def.seed,
        hash: hex(clf.weights_hash()),
        size,
        weights: clf.net.weight_count() as u64,
    }
}

/// options for `Store::export_module`
#[derive(Clone, Debug, Default)]
pub struct ExportOpts {
    pub description: String,
    pub author: String,
    /// carry the classifiers it builds on (else they're referenced by hash)
    pub upstream: bool,
}

impl Store {
    /// a trained classifier (and what it builds on) as a module file
    pub fn export_module(&self, name: &str, o: &ExportOpts) -> StoreResult<Vec<u8>> {
        let stack = self.stack(name)?;
        if let Some(d) = stack.defs.iter().find(|d| !d.trained()) {
            return Err(format!("{:?} isn't trained yet", d.name));
        }
        if o.description.chars().count() > 4000 {
            return Err("a description is at most 4000 characters".into());
        }
        let mut parts = Vec::new();
        let mut files = Vec::new();
        for (i, d) in stack.defs.iter().enumerate() {
            let mut clf = stack.graph.nodes[i].clf.clone();
            clf.steer = d.steer.clone();
            clf.meta.feedback.clear();
            let bundled = i == stack.node || o.upstream;
            let bytes = clf.to_bytes();
            parts.push(part_of(d, &clf, if bundled { bytes.len() as u64 } else { 0 }));
            if bundled {
                files.push(bytes);
            }
        }
        let m = Manifest {
            anendcls: VERSION,
            name: name.to_string(),
            description: o.description.trim().to_string(),
            author: o.author.trim().chars().take(80).collect(),
            created: crate::note::now(),
            parts,
        };
        let json = serde_json::to_vec(&m).map_err(|e| e.to_string())?;
        if json.len() > MAX_MANIFEST {
            return Err("that classifier's description is too big for a module".into());
        }
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&json);
        for f in files {
            out.extend_from_slice(&f);
        }
        let sum = fnv(0, &out);
        out.extend_from_slice(&sum.to_le_bytes());
        Ok(out)
    }
}

/// reads a module, refusing anything off (`max` bytes at most)
pub fn read(b: &[u8], max: usize) -> StoreResult<Module> {
    std::panic::catch_unwind(|| read_inner(b, max))
        .unwrap_or_else(|_| Err("that file isn't a classifier module this build reads".into()))
}

fn read_inner(b: &[u8], max: usize) -> StoreResult<Module> {
    let bad = |what: &str| -> StoreResult<Module> { Err(format!("that classifier module {what}")) };
    if b.len() > max {
        return bad(&format!("is over {} MB", max >> 20));
    }
    if b.len() < 24 || &b[..8] != MAGIC {
        return bad("isn't one (a .anendcls file from anend)");
    }
    let version = u32::from_le_bytes(b[8..12].try_into().unwrap());
    if version != VERSION {
        return bad(&format!("is format {version}; this build reads {VERSION}"));
    }
    let body = b.len() - 8;
    if fnv(0, &b[..body]) != u64::from_le_bytes(b[body..].try_into().unwrap()) {
        return bad("is damaged (its checksum doesn't match)");
    }
    let ml = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
    if ml > MAX_MANIFEST || 16 + ml > body {
        return bad("has a manifest of the wrong size");
    }
    let m: Manifest = match serde_json::from_slice(&b[16..16 + ml]) {
        Ok(m) => m,
        Err(e) => return bad(&format!("has a manifest that doesn't read ({e})")),
    };
    check_manifest(&m)?;
    let mut at = 16 + ml;
    let mut weights = Vec::new();
    let mut widths: HashMap<&str, usize> = HashMap::new();
    for p in &m.parts {
        let file = if p.size > 0 {
            let n = usize::try_from(p.size).map_err(|_| "a part is too big".to_string())?;
            let end = at
                .checked_add(n)
                .filter(|&e| e <= body)
                .ok_or("a part runs past the end of the file")?;
            let f = b[at..end].to_vec();
            at = end;
            let clf = Classifier::from_bytes(&f).map_err(|e| format!("{:?}: {e}", p.name))?;
            check_part(p, &clf, &widths)?;
            Some(f)
        } else {
            None
        };
        widths.insert(p.name.as_str(), class_count(p));
        weights.push(file);
    }
    if at != body {
        return bad("has bytes left over after its parts");
    }
    if weights.last().is_none_or(Option::is_none) {
        return bad("doesn't carry its own classifier's weights");
    }
    Ok(Module { manifest: m, weights })
}

fn class_count(p: &Part) -> usize {
    p.classes.len() + usize::from(p.head == HeadKind::Single && p.not.is_some())
}

fn class_names(p: &Part) -> Vec<String> {
    let mut v = p.classes.clone();
    if p.head == HeadKind::Single
        && let Some(n) = &p.not
    {
        v.push(n.clone());
    }
    v
}

fn check_text(t: &str, what: &str, max: usize) -> StoreResult<()> {
    if t.trim().is_empty() || t.chars().count() > max || t.chars().any(char::is_control) {
        return Err(format!("{what} is 1 to {max} letters"));
    }
    Ok(())
}

fn check_manifest(m: &Manifest) -> StoreResult<()> {
    if m.anendcls != VERSION {
        return Err("that module's manifest is another version".into());
    }
    if m.parts.is_empty() || m.parts.len() > MAX_PARTS {
        return Err(format!("a module holds 1 to {MAX_PARTS} classifiers"));
    }
    if m.description.chars().count() > 4000 || m.author.chars().count() > 80 {
        return Err("that module's description is too long".into());
    }
    if m.main().name != m.name {
        return Err("that module's last part isn't its classifier".into());
    }
    let mut seen: Vec<&str> = Vec::new();
    for p in &m.parts {
        clean_name(&p.name)?;
        if seen.contains(&p.name.as_str()) {
            return Err(format!("the module has {:?} twice", p.name));
        }
        if preset(&p.preset).is_none() {
            return Err(format!("{:?} has an unknown preset", p.name));
        }
        if p.head == HeadKind::Regress {
            return Err("classifiers here have class or label heads".into());
        }
        if p.classes.is_empty() || p.classes.len() > crate::classify::io::MAX_CLASSES {
            return Err(format!("{:?} has no classes, or too many", p.name));
        }
        for c in p.classes.iter().chain(p.not.iter()) {
            check_text(c, "a class name", 200)?;
        }
        let k = class_count(p);
        if p.head == HeadKind::Single && k < 2 {
            return Err(format!("{:?} needs two classes", p.name));
        }
        let h = p.steer.heads.first().ok_or("steering for one head")?;
        if p.steer.heads.len() != 1
            || h.bias.len() != k
            || !h.bias.iter().all(|x| x.is_finite())
            || !(h.temperature.is_finite() && h.temperature > 0.0)
            || !(0.0..=1.0).contains(&h.threshold)
        {
            return Err(format!("{:?} has steering that doesn't fit it", p.name));
        }
        let g = p.steer.gains;
        if ![g.letter, g.word, g.side, g.extra]
            .iter()
            .all(|&x| x.is_finite() && x >= 0.0)
        {
            return Err(format!("{:?} has gains out of range", p.name));
        }
        for i in &p.inputs {
            if !seen.contains(&i.from.as_str()) {
                return Err(format!(
                    "{:?} builds on {:?}, which isn't before it in the module",
                    p.name, i.from
                ));
            }
            if !(i.coupling.is_finite() && (0.0..=10.0).contains(&i.coupling)) {
                return Err("a coupling is 0 to 10".into());
            }
        }
        if p.hash.len() != 16 || u64::from_str_radix(&p.hash, 16).is_err() {
            return Err(format!("{:?} has no weights hash", p.name));
        }
        if p.history.len() > 200 {
            return Err("too much history".into());
        }
        seen.push(&p.name);
    }
    Ok(())
}

/// a bundled part's weights against its definition
fn check_part(p: &Part, clf: &Classifier, widths: &HashMap<&str, usize>) -> StoreResult<()> {
    let names = class_names(p);
    if clf.net.heads.len() != 1 || clf.net.heads[0].spec.classes != names || clf.net.heads[0].spec.kind != p.head {
        return Err(format!("{:?}: its weights' classes don't match its definition", p.name));
    }
    let extra: usize = p
        .inputs
        .iter()
        .map(|i| widths.get(i.from.as_str()).copied().unwrap_or(0))
        .sum();
    if clf.net.cfg.extra != extra {
        return Err(format!("{:?}: its inputs don't fit its weights", p.name));
    }
    if hex(clf.weights_hash()) != p.hash {
        return Err(format!("{:?}: its weights don't match their hash", p.name));
    }
    if clf.net.weight_count() as u64 != p.weights {
        return Err(format!("{:?}: its weight count doesn't match", p.name));
    }
    // replayed feedback examples must fit the net (a hand-made file could hold others)
    if clf
        .meta
        .feedback
        .iter()
        .any(|e| e.head != 0 || !(e.extra.is_empty() || e.extra.len() == extra))
    {
        return Err(format!("{:?}: its feedback doesn't fit it", p.name));
    }
    Ok(())
}

/// what taking a module in did
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    /// the classifier's name in the save
    pub name: String,
    /// classifiers made (upstream ones too), and ones the save had already (same weights)
    pub created: Vec<String>,
    pub reused: Vec<String>,
    /// each class of the classifier and the list it went to (key, and whether it's new)
    pub lists: Vec<(String, String, bool)>,
}

/// options for `Store::import_module`
#[derive(Clone, Debug, Default)]
pub struct ImportOpts {
    /// a name for the classifier itself (its own name if left out, numbered when taken)
    pub name: Option<String>,
    /// class name -> list key, for the classes to go to lists the caller picks
    pub lists: BTreeMap<String, String>,
}

impl Store {
    fn free_name(&self, want: &str) -> StoreResult<String> {
        let want = clean_name(want)?;
        if self.classifier(&want).is_err() {
            return Ok(want);
        }
        for i in 2..10_000 {
            let n = clean_name(&format!("{} {i}", want.chars().take(74).collect::<String>()))?;
            if self.classifier(&n).is_err() {
                return Ok(n);
            }
        }
        Err("no free name".into())
    }

    /// the save's classifiers by weights hash
    pub fn classifier_hashes(&self) -> StoreResult<HashMap<String, String>> {
        let mut out = HashMap::new();
        for d in self.classifiers()? {
            if let Some(b) = self.classifier_weights(&d.name)?
                && let Ok(c) = Classifier::from_bytes(&b)
            {
                out.entry(hex(c.weights_hash())).or_insert(d.name);
            }
        }
        Ok(out)
    }

    /// takes a module into the save: its classifiers (reusing any the save has with the
    /// same weights) and lists for its classes
    pub fn import_module(&self, m: &Module, o: &ImportOpts) -> StoreResult<Imported> {
        let have = self.classifier_hashes()?;
        let mut renamed: HashMap<String, String> = HashMap::new();
        let mut out = Imported::default();
        let last = m.manifest.parts.len() - 1;
        // everything is checked before anything is written
        for (p, w) in m.manifest.parts.iter().zip(&m.weights) {
            if w.is_none() && !have.contains_key(&p.hash) {
                return Err(format!(
                    "it builds on {:?}, which isn't in this module or this save",
                    p.name
                ));
            }
        }
        for (k, v) in &o.lists {
            if !class_names(m.manifest.main()).contains(k) {
                return Err(format!("the module has no class {k:?}"));
            }
            Ref::parse(v).ok_or_else(|| format!("{v:?} isn't a list's key"))?;
        }
        for (i, (p, w)) in m.manifest.parts.iter().zip(&m.weights).enumerate() {
            if let Some(n) = have.get(&p.hash)
                && (i != last || o.name.is_none())
            {
                renamed.insert(p.name.clone(), n.clone());
                out.reused.push(n.clone());
                if i == last {
                    out.name = n.clone();
                }
                continue;
            }
            let Some(bytes) = w else {
                return Err(format!("{:?} isn't bundled", p.name));
            };
            let mut clf = Classifier::from_bytes(bytes)?;
            clf.steer = p.steer.clone();
            let want = if i == last {
                o.name.clone().unwrap_or_else(|| p.name.clone())
            } else {
                p.name.clone()
            };
            let name = self.free_name(&want)?;
            let class = |c: &str| -> StoreResult<(ClassDef, bool)> {
                let picked = if i == last { o.lists.get(c) } else { None };
                let (key, new) = match picked {
                    Some(k) => (Ref::parse(k).ok_or("a list's key")?, false),
                    None => {
                        let (k, new) = self.saturate(c)?;
                        (Ref::Memory(k), new)
                    }
                };
                Ok((
                    ClassDef {
                        name: c.to_string(),
                        list: key.label(),
                    },
                    new,
                ))
            };
            let mut classes = Vec::new();
            for c in &p.classes {
                let (cd, new) = class(c)?;
                if i == last {
                    out.lists.push((c.clone(), cd.list.clone(), new));
                }
                classes.push(cd);
            }
            let not = match &p.not {
                Some(c) => {
                    let (cd, new) = class(c)?;
                    if i == last {
                        out.lists.push((c.clone(), cd.list.clone(), new));
                    }
                    Some(cd)
                }
                None => None,
            };
            let now = crate::note::now();
            let mut history = p.history.clone();
            history.push(Event {
                time: now.clone(),
                kind: "import".into(),
                note: format!(
                    "from the module {:?}{}",
                    m.manifest.name,
                    if m.manifest.author.is_empty() {
                        String::new()
                    } else {
                        format!(" by {}", m.manifest.author)
                    }
                ),
                ..Event::default()
            });
            let mut def = Definition {
                name: name.clone(),
                classes,
                not,
                head: p.head,
                preset: p.preset.clone(),
                // as the weights were trained (retraining keeps the channels)
                gematria: clf.net.cfg.letters || clf.net.cfg.words || clf.net.cfg.side,
                steer: p.steer.clone(),
                inputs: p
                    .inputs
                    .iter()
                    .map(|x| Input {
                        from: renamed.get(&x.from).cloned().unwrap_or_else(|| x.from.clone()),
                        coupling: x.coupling,
                    })
                    .collect(),
                train: p.train.clone(),
                history,
                outputs: BTreeMap::new(),
                version: 0,
                seed: p.seed,
                created: now.clone(),
                modified: now,
            };
            self.memo_clear(&name)?;
            self.put_classifier(&mut def, Some(&clf))?;
            renamed.insert(p.name.clone(), name.clone());
            out.created.push(name.clone());
            if i == last {
                out.name = name;
            }
        }
        if out.lists.is_empty() {
            // reused as it was: its lists are the save's own already
            let d = self.classifier(&out.name)?;
            out.lists = d
                .classes
                .iter()
                .chain(d.not.iter())
                .map(|c| (c.name.clone(), c.list.clone(), false))
                .collect();
        }
        Ok(out)
    }
}

/// a short summary of a module, for pages and listings
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub name: String,
    pub description: String,
    pub author: String,
    pub classes: Vec<String>,
    pub head: String,
    pub preset: String,
    pub weights: u64,
    pub accuracy: Option<f64>,
    pub macro_f1: Option<f64>,
    pub examples: usize,
    pub measured_on: String,
    /// the classifiers it builds on, and whether each is bundled
    pub upstream: Vec<(String, bool)>,
}

pub fn summary(m: &Module) -> Summary {
    let p = m.manifest.main();
    Summary {
        name: m.manifest.name.clone(),
        description: m.manifest.description.clone(),
        author: m.manifest.author.clone(),
        classes: class_names(p),
        head: match p.head {
            HeadKind::Single => "single",
            HeadKind::Multi => "multi",
            HeadKind::Regress => "regress",
        }
        .into(),
        preset: p.preset.clone(),
        weights: m
            .manifest
            .parts
            .iter()
            .zip(&m.weights)
            .filter(|(_, w)| w.is_some())
            .map(|(p, _)| p.weights)
            .sum(),
        accuracy: p.metrics.as_ref().map(|x| x.accuracy),
        macro_f1: p.metrics.as_ref().map(|x| x.macro_f1),
        examples: p.examples,
        measured_on: p.measured_on.clone(),
        upstream: m.manifest.parts[..m.manifest.parts.len() - 1]
            .iter()
            .zip(&m.weights)
            .map(|(p, w)| (p.name.clone(), w.is_some()))
            .collect(),
    }
}
