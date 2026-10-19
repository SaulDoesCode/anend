//! The app's spec model operations (`models.*`, listed in docs/app-api.md): models made
//! from a declarative spec (docs/models.md, `classify::spec`), kept in the save's models
//! file (`crate::specs`) beside its classifiers. Training runs as a background job with
//! the classifiers' job machinery and limits (`super::classify`): `models.train`
//! snapshots the data the spec's queries find in the save, answers at once, and the
//! thread writes the model when it's done; `models.job` reports how it goes and
//! `models.cancel` stops it (a cancelled run writes nothing). `models.run` answers with
//! every head's output, `models.generate` draws candidates judged by the spec's critics
//! (other spec models, or the save's classifiers, by name).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Value, json};

use super::classify::{self as cl, Eval, Job, JobState, Limits};
use super::texts::{GraphTexts, ref_of};
use super::{Res, Saves, s};
use crate::classifiers::Stack;
use crate::classify::Flow;
use crate::classify::codec::png;
use crate::classify::device::Device;
use crate::classify::generate::Sampling;
use crate::classify::model::Config;
use crate::classify::spec::{
    self, Answer, Critique, DataQuery, HeadDecl, InputSpec, MemSource, Model, ModelSpec, Origin, OutKind, SpecError,
    Value as In,
};
use crate::graph::record::{self, mime};
use crate::graph::store::rel;
use crate::graph::{Graph, Reader};
use crate::specs::{Record, Trained, Weights, shape_of};
use crate::store::Store;
use crate::texts::Texts;

/// candidates a generate call draws at most, on a shared hub and on one's own machine
const MAX_CANDIDATES_SHARED: usize = 32;
const MAX_CANDIDATES: usize = 256;
/// bytes a generate or run call writes at most on a shared hub
const MAX_WRITE_SHARED: usize = 2000;
/// weights a spec model may have on a shared hub (an estimate from its trunk)
const MAX_WEIGHTS_SHARED: usize = 40_000_000;
/// models kept loaded for run and generate
const CACHED: usize = 8;

// ---------------------------------------------------------------- pretrained folders

fn allowed() -> &'static Mutex<Vec<PathBuf>> {
    static A: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    A.get_or_init(Default::default)
}

/// the folders a shared hub loads pretrained models from (its `--pretrained`): a spec's
/// `from.pretrained` names a folder under one of them (relative to it, or absolute
/// inside it); without any, a shared hub loads none. A machine of one's own (no limits)
/// loads any folder.
pub fn allow_pretrained(dirs: Vec<PathBuf>) {
    *allowed().lock().unwrap_or_else(|e| e.into_inner()) = dirs;
}

/// where a spec's pretrained folder is, under the limits
fn resolve_pretrained(dir: &str, limits: &Limits) -> Result<PathBuf, String> {
    let p = Path::new(dir);
    if !limits.shared {
        return Ok(p.to_path_buf());
    }
    let roots = allowed().lock().unwrap_or_else(|e| e.into_inner()).clone();
    if roots.is_empty() {
        return Err("this hub loads no pretrained models (its admin can allow folders with --pretrained)".into());
    }
    for root in &roots {
        let Ok(r) = root.canonicalize() else { continue };
        let c = if p.is_absolute() { p.to_path_buf() } else { r.join(p) };
        if let Ok(c) = c.canonicalize()
            && c.starts_with(&r)
        {
            return Ok(c);
        }
    }
    Err(format!(
        "pretrained folder {dir:?} isn't one this hub has (models.pretrained lists them)"
    ))
}

/// the model folders a shared hub offers (each holding a config.json)
fn pretrained_folders() -> Vec<String> {
    let mut out = Vec::new();
    for root in allowed().lock().unwrap_or_else(|e| e.into_inner()).iter() {
        if root.join("config.json").is_file() {
            out.push(".".to_string());
        }
        for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
            if e.path().join("config.json").is_file() {
                out.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------- checks

fn spec_err(e: SpecError) -> String {
    if e.path.is_empty() {
        e.to_string()
    } else {
        format!("model spec {:?} at {}: {}", e.model, e.path, e.message)
    }
}

fn at(spec: &ModelSpec, path: &str, message: String) -> SpecError {
    SpecError {
        model: spec.name.clone(),
        path: path.into(),
        message,
    }
}

/// about how many weights a trunk of `c` has
fn weights_of(c: &Config) -> usize {
    let kv = if c.kv_heads > 0 { c.kv_heads } else { c.heads };
    let attn = c.d_model * c.head_dim * (2 * c.heads + 2 * kv);
    let ff = 3 * c.d_model * c.d_ff;
    c.layers.saturating_mul(attn + ff + 2 * c.d_model) + 260 * c.d_model
}

/// a spec under the limits: on a shared hub its pretrained folder is one the hub
/// allows, its trunk not too big, and it runs on the CPU
fn within(spec: &ModelSpec, limits: &Limits) -> Result<(), SpecError> {
    if let Some(Origin::Pretrained(dir)) = &spec.from {
        resolve_pretrained(dir, limits).map_err(|e| at(spec, "from.pretrained", e))?;
    }
    if !limits.shared {
        return Ok(());
    }
    if spec.device != Device::Cpu {
        return Err(at(spec, "device", "this hub trains and writes on its CPU".into()));
    }
    if spec.from.is_none() {
        let c = spec.config(None, 0).map_err(|e| at(spec, "trunk", e))?;
        let n = weights_of(&c);
        if n > MAX_WEIGHTS_SHARED {
            return Err(at(
                spec,
                "trunk",
                format!(
                    "about {} M weights; this hub trains at most {} M",
                    n / 1_000_000,
                    MAX_WEIGHTS_SHARED / 1_000_000
                ),
            ));
        }
    }
    Ok(())
}

/// the spec of `args.spec` (a JSON object, or its text), checked: its source text too
fn spec_arg(args: &Value) -> Result<(String, ModelSpec), SpecError> {
    let text = match args.get("spec") {
        Some(Value::String(t)) => t.clone(),
        Some(v @ Value::Object(_)) => serde_json::to_string_pretty(v).unwrap_or_default(),
        _ => {
            return Err(SpecError {
                model: String::new(),
                path: String::new(),
                message: "a spec, please (its JSON)".into(),
            });
        }
    };
    let spec = ModelSpec::parse(&text, true)?;
    Ok((text, spec))
}

/// a spec fit to store: checked, under the limits, its name free of the save's
/// classifiers (critics find models by name)
fn checked(store: &Store, args: &Value, limits: &Limits) -> Result<(String, ModelSpec), SpecError> {
    let (text, spec) = spec_arg(args)?;
    within(&spec, limits)?;
    let name = spec.name.trim();
    if name != spec.name {
        return Err(at(&spec, "name", "a name without spaces at its ends, please".into()));
    }
    if store.classifier(name).is_ok() {
        return Err(at(
            &spec,
            "name",
            format!("{name:?} is a classifier's name; critics find models by name, so pick another"),
        ));
    }
    Ok((text, spec))
}

// ---------------------------------------------------------------- jobs

fn key_name(name: &str) -> String {
    format!("\u{0}model\u{0}{name}")
}

fn job(store: &Store, name: &str) -> Option<Arc<Job>> {
    cl::jobs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&cl::job_key(store, &key_name(name)))
        .cloned()
}

fn training(store: &Store, name: &str) -> bool {
    job(store, name).is_some_and(|j| j.running())
}

fn not_training(store: &Store, name: &str) -> Result<(), String> {
    if training(store, name) {
        Err(format!("{name:?} is training; wait for it or cancel it"))
    } else {
        Ok(())
    }
}

fn tick(j: &Job, p: &crate::classify::Progress) {
    let mut s = j.state.lock().unwrap_or_else(|e| e.into_inner());
    s.epoch = p.epoch;
    s.step = p.step;
    s.steps = p.steps;
    s.loss = p.loss;
    s.lr = p.lr;
    s.tokens_per_sec = p.tokens_per_sec;
    match p.eval {
        Some(e) => s.evals.push(Eval {
            step: p.step,
            round: 1,
            loss: e.loss,
            accuracy: e.accuracy(),
            macro_f1: e.heads.iter().flatten().next().map_or(0.0, |m| m.macro_f1),
            bits_per_byte: e.bits_per_byte,
        }),
        None => {
            s.losses.push((p.step, p.loss));
            if s.losses.len() > 400 {
                s.losses = s.losses.iter().step_by(2).copied().collect();
            }
        }
    }
}

// ---------------------------------------------------------------- the save as a source

/// a list's entries: `name` is a key (`0,12`) or the text of the list's head
fn list_ids(g: &Graph, name: &str) -> Result<Vec<u64>, String> {
    let v = g.view()?;
    if let Some(id) = record::parse_label(name.trim())
        && v.record(id)?.is_some()
    {
        return v.members(id);
    }
    for id in v.find_text(name, None)? {
        let m = v.members(id)?;
        if !m.is_empty() {
            return Ok(m);
        }
    }
    Err(format!("the save has no list {name:?}"))
}

fn tagged_ids(g: &Graph, tag: &str) -> Result<Vec<u64>, String> {
    let v = g.view()?;
    Ok(
        match v
            .find_text(tag.trim().trim_start_matches('#'), Some(mime::TAG))?
            .first()
        {
            Some(&t) => v.in_edges(t, rel::TAG)?,
            None => vec![],
        },
    )
}

/// an entry's text as a value of the spec's input
fn value_of(input: &InputSpec, text: String) -> Result<In, String> {
    match input {
        InputSpec::Text { .. } | InputSpec::Bytes => Ok(In::Text(text)),
        InputSpec::Numbers { names, .. } => {
            let x: Vec<f32> = text
                .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    p.parse::<f32>()
                        .map_err(|_| format!("{text:?} isn't {} numbers", names.len()))
                })
                .collect::<Result<_, _>>()?;
            if x.len() != names.len() {
                return Err(format!("{text:?} isn't {} numbers", names.len()));
            }
            Ok(In::Numbers(x))
        }
        InputSpec::Image { .. } => {
            Err("a picture model's data comes inline in its spec for now (the save holds texts)".into())
        }
    }
}

/// what the spec's data queries find in the save, read now (a training's snapshot)
fn snapshot(g: &Arc<Graph>, spec: &ModelSpec) -> Result<MemSource, String> {
    let texts = GraphTexts(g.clone());
    let read = |ids: Vec<u64>| -> Result<Vec<In>, String> {
        let mut out = Vec::new();
        for id in ids {
            if let Some(t) = texts.text(&ref_of(id))?
                && !t.trim().is_empty()
            {
                out.push(value_of(&spec.input, t)?);
            }
        }
        Ok(out)
    };
    let mut src = MemSource::default();
    for (i, q) in spec.data.iter().enumerate() {
        let at = |e: String| format!("model spec {:?} at data[{i}]: {e}", spec.name);
        match q {
            DataQuery::List { list, .. } => {
                let v = read(list_ids(g, list).map_err(at)?).map_err(at)?;
                src.lists.insert(list.clone(), v);
            }
            DataQuery::Tagged { tag, .. } => {
                let v = read(tagged_ids(g, tag).map_err(at)?).map_err(at)?;
                src.tags.insert(tag.clone(), v);
            }
            DataQuery::Pairs { input, output, .. } => {
                for l in [input, output] {
                    let v = read(list_ids(g, l).map_err(at)?).map_err(at)?;
                    src.lists.insert(l.clone(), v);
                }
            }
            DataQuery::Inline { .. } => {}
        }
    }
    Ok(src)
}

// ---------------------------------------------------------------- loading

/// loaded models by save file and name, with the record version they were loaded at
type Loaded = Mutex<HashMap<String, (u64, Arc<Model>)>>;

fn cache() -> &'static Loaded {
    static C: OnceLock<Loaded> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn forget(store: &Store, name: &str) {
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&cl::job_key(store, &key_name(name)));
}

/// the spec with its pretrained folder where it is under the limits
fn resolved(spec: &ModelSpec, limits: &Limits) -> Result<ModelSpec, String> {
    let mut s = spec.clone();
    if let Some(Origin::Pretrained(dir)) = &spec.from {
        s.from = Some(Origin::Pretrained(
            resolve_pretrained(dir, limits)?.to_string_lossy().into_owned(),
        ));
    }
    Ok(s)
}

fn record(store: &Store, name: &str) -> Result<Record, String> {
    store.spec_model(name)?.ok_or_else(|| format!("no model {name:?}"))
}

/// the model as stored (its current spec over its trained weights), or a fresh one
/// from the spec (a pretrained one as its checkpoint has it)
fn load(store: &Store, r: &Record, limits: &Limits) -> Result<Arc<Model>, String> {
    let key = cl::job_key(store, &key_name(&r.name));
    if let Some((v, m)) = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key)
        && *v == r.version
    {
        return Ok(m.clone());
    }
    let spec = r.spec()?;
    let mut m = match store.spec_weights(&r.name)? {
        Some(b) => Model::from_bytes(&b)?,
        None => {
            within(&spec, limits).map_err(spec_err)?;
            spec::build(&resolved(&spec, limits)?, None)?
        }
    };
    m.spec = spec;
    let m = Arc::new(m);
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if c.len() >= CACHED {
        c.clear();
    }
    c.insert(key, (r.version, m.clone()));
    Ok(m)
}

// ---------------------------------------------------------------- json

fn name_arg(args: &Value) -> Result<&str, String> {
    match s(args, "name").trim() {
        "" => Err("which model? (name)".into()),
        n => Ok(n),
    }
}

fn kind_name(k: OutKind) -> &'static str {
    match k {
        OutKind::Classify => "classify",
        OutKind::Multi => "multi",
        OutKind::Regress => "regress",
        OutKind::Generate => "generate",
    }
}

fn input_name(i: &InputSpec) -> &'static str {
    match i {
        InputSpec::Text { .. } => "text",
        InputSpec::Bytes => "bytes",
        InputSpec::Image { .. } => "image",
        InputSpec::Numbers { .. } => "numbers",
    }
}

fn summary(store: &Store, r: &Record) -> Value {
    let spec = r.spec().ok();
    let last = r.history.last();
    json!({
        "name": r.name,
        "trained": r.trained(),
        "version": r.version,
        "created": r.created,
        "modified": r.modified,
        "input": spec.as_ref().map(|s| input_name(&s.input)),
        "heads": spec.as_ref().map(|s| s.heads.iter().map(|h| json!({
            "name": h.name, "kind": kind_name(h.kind), "classes": h.classes,
        })).collect::<Vec<_>>()),
        "generates": spec.as_ref().is_some_and(|s| s.heads.iter().any(|h| h.kind == OutKind::Generate)),
        "critics": spec.as_ref().map(|s| s.critics.iter().map(|c| c.model.clone()).collect::<Vec<_>>()),
        "pretrained": spec.as_ref().and_then(|s| s.from.as_ref().map(|Origin::Pretrained(d)| d.clone())),
        "device": spec.as_ref().map(|s| s.device),
        "loss": last.and_then(|t| t.loss),
        "accuracy": last.and_then(|t| t.heads.first().map(|h| h.1.accuracy)),
        "macro_f1": last.and_then(|t| t.heads.first().map(|h| h.1.macro_f1)),
        "bits_per_byte": last.and_then(|t| t.bits_per_byte),
        "steps": r.history.iter().map(|t| t.steps).sum::<usize>(),
        "training": training(store, &r.name),
    })
}

fn details(store: &Store, r: &Record) -> Res {
    let mut v = summary(store, r);
    let o = v.as_object_mut().expect("an object");
    o.insert("source".into(), json!(r.source));
    o.insert("history".into(), json!(r.history));
    o.insert(
        "job".into(),
        job(store, &r.name).map_or(Value::Null, |j| json!(j.snapshot())),
    );
    Ok(v)
}

fn now() -> u64 {
    crate::graph::store::system_ms()
}

/// a generated or given value as JSON: text, bytes (and their text), numbers, a picture
/// as a PNG in base64
fn value_json(v: &In) -> Value {
    match v {
        In::Text(t) => json!({ "text": t }),
        In::Bytes(b) => json!({ "bytes": b, "text": String::from_utf8_lossy(b) }),
        In::Numbers(x) => json!({ "numbers": x }),
        In::Image(im) => json!({
            "image": {"width": im.width, "height": im.height, "png": crate::classifiers::base64(&png(im))},
        }),
    }
}

fn answer_json(a: &Answer) -> Value {
    match a {
        Answer::Classes { probs, top, members } => json!({
            "kind": "classes", "probs": probs, "top": top, "members": members,
        }),
        Answer::Values(v) => json!({ "kind": "values", "values": v }),
        Answer::Generated(v) => {
            let mut j = value_json(v);
            j["kind"] = json!("generated");
            j
        }
    }
}

/// the input of a run or generate call: `input` (a value: {text}, {numbers}, {bytes}) or
/// `text` (or `prompt`)
fn input_arg(args: &Value, key: &str, empty: bool) -> Result<In, String> {
    if let Some(v) = args.get("input").filter(|v| !v.is_null()) {
        return serde_json::from_value(v.clone())
            .map_err(|e| format!("an input is {{\"text\": ..}}, {{\"numbers\": [..]}} or {{\"bytes\": [..]}}: {e}"));
    }
    let t = s(args, key);
    if t.is_empty() && !empty {
        return Err("a text, please".into());
    }
    Ok(if t.is_empty() {
        In::Bytes(Vec::new())
    } else {
        In::Text(t.to_string())
    })
}

/// the spec's sampling with the call's `temperature`, `top_k`, `top_p`, `max_len`,
/// `seed`, under the limits
fn sampling_arg(base: &Sampling, args: &Value, limits: &Limits) -> Result<Sampling, String> {
    let mut sm = base.clone();
    let f = |k: &str| args.get(k).and_then(Value::as_f64);
    if let Some(t) = f("temperature") {
        sm.temperature = t as f32;
    }
    if let Some(t) = f("top_p") {
        sm.top_p = t as f32;
    }
    if let Some(k) = args.get("top_k").and_then(Value::as_u64) {
        sm.top_k = k as usize;
    }
    if let Some(n) = args.get("max_len").and_then(Value::as_u64) {
        sm.max_len = n as usize;
    }
    if let Some(n) = args.get("seed").and_then(Value::as_u64) {
        sm.seed = n;
    }
    if limits.shared && sm.max_len > MAX_WRITE_SHARED {
        return Err(format!("this hub writes at most {MAX_WRITE_SHARED} bytes at a time"));
    }
    sm.check()?;
    Ok(sm)
}

// ---------------------------------------------------------------- training

fn start(store: Arc<Store>, g: &Arc<Graph>, args: &Value, limits: &Limits) -> Res {
    let name = name_arg(args)?;
    let r = record(&store, name)?;
    let mut sp = r.spec()?;
    within(&sp, limits).map_err(spec_err)?;
    let u = |k: &str| args.get(k).and_then(Value::as_u64).map(|x| x as usize);
    if let Some(e) = u("epochs") {
        sp.train.epochs = Some(e.clamp(1, 10_000));
    }
    if let Some(m) = u("max_steps") {
        sp.train.max_steps = Some(m);
    }
    if limits.shared {
        let cores = crate::classify::pool::available();
        sp.train.threads = Some(sp.train.threads.unwrap_or(cores).clamp(1, cores));
    }
    let fresh = args.get("fresh").and_then(Value::as_bool).unwrap_or(false);
    let wait = args.get("wait").and_then(Value::as_bool).unwrap_or(false);
    cl::room_for_a_job(&store, limits)?;
    let src = snapshot(g, &sp)?;
    let base = match store.spec_weights(name)? {
        Some(b) if !fresh => {
            let mut m = Model::from_bytes(&b)?;
            m.spec = sp.clone();
            Some(m)
        }
        _ => None,
    };
    let fresh = base.is_none();
    let run_spec = resolved(&sp, limits)?;
    // about how many examples: the queries' finds (pairs count once)
    let found = src.lists.values().chain(src.tags.values()).map(Vec::len).sum::<usize>()
        + sp.data
            .iter()
            .map(|q| match q {
                DataQuery::Inline { items } => items.len(),
                _ => 0,
            })
            .sum::<usize>();
    let job = Arc::new(Job {
        cancel: AtomicBool::new(false),
        state: Mutex::new(JobState {
            name: name.to_string(),
            state: "running".into(),
            rounds: 1,
            round: 1,
            examples: found,
            ..JobState::default()
        }),
        started: std::time::Instant::now(),
    });
    {
        let mut all = cl::jobs().lock().unwrap_or_else(|e| e.into_inner());
        let key = cl::job_key(&store, &key_name(name));
        if all.get(&key).is_some_and(|j| j.running()) {
            return Err(format!("{name:?} is training already"));
        }
        all.insert(key, job.clone());
    }
    let j = job.clone();
    let limit = limits.train;
    let name = name.to_string();
    let version = r.version;
    let work = move || {
        let mut too_long = false;
        let res = spec::train(base, &run_spec, &src, None, |p| {
            tick(&j, p);
            too_long = limit.is_some_and(|l| j.started.elapsed() > l);
            if too_long || j.cancel.load(Ordering::Relaxed) {
                Flow::Stop
            } else {
                Flow::Go
            }
        });
        let res = res.and_then(|(mut m, rep)| {
            j.state.lock().unwrap_or_else(|e| e.into_inner()).examples = rep.train_examples + rep.val_examples;
            if too_long {
                return Err(format!(
                    "the hub stops a training after {} hours; try fewer epochs or a smaller trunk",
                    limit.map_or(0, |l| l.as_secs() / 3600)
                ));
            }
            if rep.cancelled {
                return Ok((Value::Null, "cancelled"));
            }
            let mut r = record(&store, &name)?;
            if r.version != version {
                return Err("the spec changed while it trained; train it again".into());
            }
            m.spec = sp;
            let heads = m
                .spec
                .heads
                .iter()
                .filter(|h| h.kind != OutKind::Generate)
                .map(|h| h.name.clone());
            let metrics = rep.eval.as_ref();
            r.history.push(Trained {
                time: now(),
                steps: rep.steps,
                epochs: rep.epochs,
                examples: rep.train_examples,
                val_examples: rep.val_examples,
                seconds: rep.seconds,
                tokens_per_sec: rep.tokens_per_sec,
                stopped_early: rep.stopped_early,
                fresh,
                loss: rep.best_loss.or(metrics.map(|e| e.loss)),
                bits_per_byte: metrics.and_then(|e| e.bits_per_byte),
                heads: metrics.map_or(vec![], |e| {
                    heads
                        .zip(&e.heads)
                        .filter_map(|(n, m)| m.clone().map(|m| (n, m)))
                        .collect()
                }),
                weights: m.clf.net.weight_count(),
            });
            r.version += 1;
            r.modified = now();
            store.put_spec_model(None, &r, Weights::Set(m.to_bytes()))?;
            forget(&store, &name);
            Ok((summary(&store, &r), "done"))
        });
        let mut st = j.state.lock().unwrap_or_else(|e| e.into_inner());
        st.seconds = j.started.elapsed().as_secs_f64();
        match res {
            Ok((s, state)) => {
                st.state = state.into();
                if !s.is_null() {
                    st.model = Some(s);
                }
            }
            Err(e) => {
                st.state = "failed".into();
                st.error = Some(e);
            }
        }
    };
    if wait {
        work();
    } else if let Err(e) = std::thread::Builder::new().name("train model".into()).spawn(work) {
        let mut st = job.state.lock().unwrap_or_else(|e| e.into_inner());
        st.state = "failed".into();
        st.error = Some(format!("couldn't start the training: {e}"));
    }
    Ok(json!(job.snapshot()))
}

// ---------------------------------------------------------------- generating

/// what a critic is: a spec model, or a classifier of the save (its head is its name)
enum Critic {
    Model(Arc<Model>),
    Classifier(Stack, Vec<HeadDecl>),
}

fn critics_of(store: &Store, spec: &ModelSpec, limits: &Limits) -> Result<BTreeMap<String, Critic>, String> {
    let mut out = BTreeMap::new();
    for c in &spec.critics {
        if out.contains_key(&c.model) {
            continue;
        }
        let got = if c.model == spec.name {
            return Err(format!("{:?} can't be its own critic", spec.name));
        } else if let Some(r) = store.spec_model(&c.model)? {
            Critic::Model(load(store, &r, limits)?)
        } else if let Ok(d) = store.classifier(&c.model) {
            if !d.trained() {
                return Err(format!("critic {:?} isn't trained yet", c.model));
            }
            let kind = match d.head {
                crate::classify::HeadKind::Multi => OutKind::Multi,
                _ => OutKind::Classify,
            };
            let head = HeadDecl {
                name: d.name.clone(),
                kind,
                classes: d.class_names(),
                weight: 1.0,
            };
            Critic::Classifier(store.stack(&c.model)?, vec![head])
        } else {
            return Err(format!(
                "critic {:?}: the save has no model or classifier of that name",
                c.model
            ));
        };
        out.insert(c.model.clone(), got);
    }
    Ok(out)
}

fn generate(store: &Store, args: &Value, limits: &Limits) -> Res {
    let r = record(store, name_arg(args)?)?;
    let m = load(store, &r, limits)?;
    if !m.spec.heads.iter().any(|h| h.kind == OutKind::Generate) {
        return Err(format!("{:?} has no generate head", r.name));
    }
    let sampling = sampling_arg(&m.spec.sampling, args, limits)?;
    let most = if limits.shared {
        MAX_CANDIDATES_SHARED
    } else {
        MAX_CANDIDATES
    };
    let n = args.get("candidates").and_then(Value::as_u64).unwrap_or(8) as usize;
    if n == 0 || n > most {
        return Err(format!("1 to {most} candidates, please"));
    }
    let guidance = args
        .get("guided")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then(crate::classify::generate::Guidance::default);
    let prompt = input_arg(args, "prompt", true)?;
    let held = critics_of(store, &m.spec, limits)?;
    let critiques: BTreeMap<String, Critique> = held
        .iter()
        .map(|(n, c)| {
            let k = match c {
                Critic::Model(m) => m.critique(),
                Critic::Classifier(st, heads) => Critique {
                    judge: st,
                    heads: heads.clone(),
                },
            };
            (n.clone(), k)
        })
        .collect();
    let started = std::time::Instant::now();
    let cands = spec::generate_by(&m, &critiques, &prompt, &sampling, n, guidance)?;
    let critics: Vec<Value> = m
        .spec
        .critics
        .iter()
        .map(|c| json!({ "model": c.model, "head": c.head, "want": c.want, "weight": c.weight }))
        .collect();
    Ok(json!({
        "name": r.name,
        "trained": r.trained(),
        "seed": sampling.seed,
        "critics": critics,
        "seconds": started.elapsed().as_secs_f64(),
        "candidates": cands.iter().map(|c| {
            let mut v = value_json(&m.decode(&c.bytes));
            let o = v.as_object_mut().expect("an object");
            o.insert("index".into(), json!(c.index));
            o.insert("score".into(), json!(c.score));
            o.insert("scores".into(), json!(c.scores));
            o.insert("logp".into(), json!(c.logp));
            o.insert("kept".into(), json!(c.passed()));
            o.insert("broken".into(), json!(c.broken));
            v
        }).collect::<Vec<_>>(),
    }))
}

// ---------------------------------------------------------------- the calls

/// whether `op` is one of these
pub fn routed(op: &str) -> bool {
    op.starts_with("models.")
}

/// `models.*` on the open save of `saves`
pub fn saves_call(saves: &Saves, op: &str, args: &Value, limits: &Limits) -> Res {
    let (_, g) = saves.current()?;
    let (_, store) = saves.models()?;
    call(store, &g, op, args, limits)
}

pub fn call(store: Arc<Store>, g: &Arc<Graph>, op: &str, args: &Value, limits: &Limits) -> Res {
    let st: &Store = &store;
    match op {
        "models.list" => Ok(Value::Array(st.spec_models()?.iter().map(|r| summary(st, r)).collect())),
        "models.get" => details(st, &record(st, name_arg(args)?)?),
        "models.check" => Ok(match checked(st, args, limits) {
            Ok((_, spec)) => json!({ "ok": true, "name": spec.name }),
            Err(e) => json!({ "ok": false, "path": e.path, "message": e.message }),
        }),
        "models.pretrained" => Ok(json!({
            "any": !limits.shared,
            "folders": if limits.shared { pretrained_folders() } else { vec![] },
        })),
        "models.create" => {
            let (text, spec) = checked(st, args, limits).map_err(spec_err)?;
            if st.spec_model(&spec.name)?.is_some() {
                return Err(spec_err(at(
                    &spec,
                    "name",
                    format!("there's a model {:?} already", spec.name),
                )));
            }
            let t = now();
            let r = Record {
                name: spec.name.clone(),
                source: text,
                created: t,
                modified: t,
                version: 1,
                history: vec![],
            };
            st.put_spec_model(None, &r, Weights::Drop)?;
            details(st, &r)
        }
        "models.update" => {
            let name = name_arg(args)?;
            not_training(st, name)?;
            let old = record(st, name)?;
            let (text, spec) = checked(st, args, limits).map_err(spec_err)?;
            if spec.name != name && st.spec_model(&spec.name)?.is_some() {
                return Err(spec_err(at(
                    &spec,
                    "name",
                    format!("there's a model {:?} already", spec.name),
                )));
            }
            if spec.name != name {
                not_training(st, &spec.name)?;
            }
            let keeps = old.spec().is_ok_and(|o| shape_of(&o) == shape_of(&spec));
            let r = Record {
                name: spec.name.clone(),
                source: text,
                modified: now(),
                version: old.version + 1,
                history: if keeps { old.history.clone() } else { vec![] },
                ..old
            };
            st.put_spec_model(Some(name), &r, if keeps { Weights::Keep } else { Weights::Drop })?;
            forget(st, name);
            forget(st, &r.name);
            let mut v = details(st, &r)?;
            v["weights_kept"] = json!(keeps && r.trained());
            Ok(v)
        }
        "models.delete" => {
            let name = name_arg(args)?;
            not_training(st, name)?;
            st.delete_spec_model(name)?;
            forget(st, name);
            cl::jobs()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&cl::job_key(st, &key_name(name)));
            Ok(Value::Null)
        }
        "models.train" => start(store.clone(), g, args, limits),
        "models.job" => {
            let name = name_arg(args)?;
            Ok(match job(st, name) {
                Some(j) => json!(j.snapshot()),
                None => json!({ "name": name, "state": "idle" }),
            })
        }
        "models.cancel" => {
            let name = name_arg(args)?;
            let j = job(st, name).ok_or_else(|| format!("{name:?} isn't training"))?;
            j.cancel.store(true, Ordering::Relaxed);
            Ok(json!(j.snapshot()))
        }
        "models.run" => {
            let r = record(st, name_arg(args)?)?;
            let m = load(st, &r, limits)?;
            let input = input_arg(args, "text", false)?;
            let sampling = sampling_arg(&m.spec.sampling, args, limits)?;
            let started = std::time::Instant::now();
            let out = m.run_sampled(&input, None, &sampling)?;
            Ok(json!({
                "name": r.name,
                "trained": r.trained(),
                "seconds": started.elapsed().as_secs_f64(),
                "outputs": out.iter().map(|(k, a)| (k.clone(), answer_json(a))).collect::<serde_json::Map<_, _>>(),
            }))
        }
        "models.generate" => generate(st, args, limits),
        _ => Err(format!("unknown op {op:?}")),
    }
}

#[cfg(test)]
mod tests;
