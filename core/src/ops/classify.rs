//! The app's classifier operations (`classify.*`, listed in docs/app-api.md), over the
//! open save's classifiers (`crate::classifiers`). Training runs on a thread of its
//! own: `classify.train` snapshots the examples, answers at once, and the thread writes
//! the weights when it's done; `classify.job` reports how it goes and `classify.cancel`
//! stops it (a cancelled run writes nothing).

use crate::classifiers::{self as clfs, Input, Judged, Stack, Summary, Tick, TrainArgs};
use crate::classify::steer::Output;
use crate::classify::{Flow, Gains, HeadKind, Pool};
use crate::memory::Ref;
use crate::store::Store;
use crate::{clsmodule, filters, memo};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use super::{GraphTexts, Res, Saves};
use crate::graph::Graph;

/// how many memories and notes a search, a list or a neighbour search looks through
const SCAN: usize = 20_000;

pub(super) fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool::new(crate::classify::pool::available()))
}

// ---------------------------------------------------------------- jobs

#[derive(Clone, Debug, Default, Serialize)]
pub struct Eval {
    pub step: usize,
    pub round: usize,
    pub loss: f64,
    pub accuracy: f64,
    pub macro_f1: f64,
    /// a generate head's held-out bits per byte (spec models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bits_per_byte: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct JobState {
    pub name: String,
    /// running, done, failed, cancelled
    pub state: String,
    pub round: usize,
    pub rounds: usize,
    pub epoch: usize,
    pub step: usize,
    pub steps: usize,
    pub loss: f64,
    pub lr: f32,
    pub tokens_per_sec: f64,
    /// (step, loss), thinned to about 200 points
    pub losses: Vec<(usize, f64)>,
    pub evals: Vec<Eval>,
    pub seconds: f64,
    pub examples: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
    /// a spec model's summary when its training is done (`ops::models`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<Value>,
    /// where it runs: cpu, gpu or gpu_fast
    pub device: String,
    /// why it runs on the CPU when the GPU was asked for
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_note: Option<String>,
}

pub(crate) fn device_name(d: crate::classify::device::Device) -> &'static str {
    use crate::classify::device::Device;
    match d {
        Device::Cpu => "cpu",
        Device::Gpu => "gpu",
        Device::GpuFast => "gpu_fast",
    }
}

pub struct Job {
    pub(super) cancel: AtomicBool,
    pub(super) state: Mutex<JobState>,
    pub(super) started: std::time::Instant,
}

impl Job {
    pub(super) fn snapshot(&self) -> JobState {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if s.state == "running" {
            s.seconds = self.started.elapsed().as_secs_f64();
        }
        s
    }

    pub(super) fn running(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).state == "running"
    }

    fn tick(&self, t: &Tick) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.round = t.round;
        s.rounds = t.rounds;
        s.epoch = t.epoch;
        s.step = t.step;
        s.steps = t.steps;
        s.loss = t.loss;
        s.lr = t.lr;
        s.tokens_per_sec = t.tokens_per_sec;
        if let Some((loss, accuracy, macro_f1)) = t.eval {
            s.evals.push(Eval {
                step: t.step,
                round: t.round,
                loss,
                accuracy,
                macro_f1,
                bits_per_byte: None,
            });
        } else {
            s.losses.push((t.step, t.loss));
            if s.losses.len() > 400 {
                // keep every other point
                s.losses = s.losses.iter().step_by(2).copied().collect();
            }
        }
    }
}

/// jobs by save file and classifier
pub(super) fn jobs() -> &'static Mutex<HashMap<String, Arc<Job>>> {
    static JOBS: OnceLock<Mutex<HashMap<String, Arc<Job>>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

pub(super) fn job_key(store: &Store, name: &str) -> String {
    format!("{}\u{0}{name}", store.path().display())
}

fn job(store: &Store, name: &str) -> Option<Arc<Job>> {
    jobs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&job_key(store, name))
        .cloned()
}

fn training(store: &Store, name: &str) -> bool {
    job(store, name).is_some_and(|j| j.running())
}

fn not_training(store: &Store, names: &[String]) -> Result<(), String> {
    match names.iter().find(|n| training(store, n)) {
        Some(n) => Err(format!("{n:?} is training; wait for it or cancel it")),
        None => Ok(()),
    }
}

/// how much a machine shared by many (a hub, not `--local`) lets each save's folder run:
/// at most `per_folder` trainings and indexings at once, `total` in all, and a training
/// stops after `train`; [`Limits::NONE`] for a machine of one's own (the desktop)
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub shared: bool,
    pub per_folder: usize,
    pub total: usize,
    pub train: Option<std::time::Duration>,
    /// whether jobs may run on the GPU (a shared hub: only with `--gpu`)
    pub gpu: bool,
}

/// the shared hub's: each workspace runs at most this many trainings and indexings
pub const WORKSPACE_JOBS: usize = 2;
pub const TRAIN_LIMIT: std::time::Duration = std::time::Duration::from_secs(2 * 3600);

impl Limits {
    pub const NONE: Limits = Limits {
        shared: false,
        per_folder: usize::MAX,
        total: usize::MAX,
        train: None,
        gpu: true,
    };

    /// a hub's for its members
    pub fn shared() -> Limits {
        Limits {
            shared: true,
            per_folder: WORKSPACE_JOBS,
            total: crate::classify::pool::available().max(4),
            train: Some(TRAIN_LIMIT),
            gpu: false,
        }
    }
}

/// the workspace folder a job key's save is in
fn key_folder(key: &str) -> &str {
    let path = key.split('\u{0}').next().unwrap_or("");
    path.rsplit_once(['/', '\\']).map_or("", |(dir, _)| dir)
}

/// whether another job may start for `store`'s folder
pub(super) fn room_for_a_job(store: &Store, limits: &Limits) -> Result<(), String> {
    if !limits.shared {
        return Ok(());
    }
    let mine = job_key(store, "");
    let mine = key_folder(&mine);
    let mut running: Vec<String> = jobs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(_, j)| j.running())
        .map(|(k, _)| k.clone())
        .collect();
    running.extend(
        index_jobs()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, j)| index_snapshot(j)["state"] == "running")
            .map(|(k, _)| k.clone()),
    );
    if running.iter().filter(|k| key_folder(k) == mine).count() >= limits.per_folder {
        return Err(format!(
            "{} trainings or indexings are running in your saves already; wait for one to end",
            limits.per_folder
        ));
    }
    if running.len() >= limits.total {
        return Err("the hub is busy training for others; try again in a while".into());
    }
    Ok(())
}

fn start(store: Arc<Store>, name: &str, a: &TrainArgs, wait: bool, limits: &Limits) -> Res {
    room_for_a_job(&store, limits)?;
    if a.device.is_gpu() && !limits.gpu {
        return Err("this hub trains on its CPU (its admin can allow the GPU with --gpu)".into());
    }
    let mut t = store.prepare(name, a)?;
    let job = Arc::new(Job {
        cancel: AtomicBool::new(false),
        state: Mutex::new(JobState {
            name: name.to_string(),
            state: "running".into(),
            rounds: t.rounds,
            examples: t.data.len(),
            device: device_name(t.opts.device).into(),
            device_note: t.device_note.clone(),
            ..JobState::default()
        }),
        started: std::time::Instant::now(),
    });
    {
        let mut all = jobs().lock().unwrap_or_else(|e| e.into_inner());
        let key = job_key(&store, name);
        if all.get(&key).is_some_and(|j| j.running()) {
            return Err(format!("{name:?} is training already"));
        }
        all.insert(key, job.clone());
    }
    let j = job.clone();
    let limit = limits.train;
    let mut work = move || {
        let mut too_long = false;
        let r = t.run(|tick| {
            j.tick(tick);
            too_long = limit.is_some_and(|l| j.started.elapsed() > l);
            if too_long || j.cancel.load(Ordering::Relaxed) {
                Flow::Stop
            } else {
                Flow::Go
            }
        });
        let r = r.and_then(|s| {
            if too_long {
                Err(format!(
                    "the hub stops a training after {} hours; try fewer epochs or rounds, or a smaller preset",
                    limit.map_or(0, |l| l.as_secs() / 3600)
                ))
            } else if s.cancelled {
                Ok((s, "cancelled"))
            } else {
                store.commit(&t, &s).map(|_| (s, "done"))
            }
        });
        let mut st = j.state.lock().unwrap_or_else(|e| e.into_inner());
        st.seconds = j.started.elapsed().as_secs_f64();
        match r {
            Ok((s, state)) => {
                st.state = state.into();
                st.summary = Some(s);
            }
            Err(e) => {
                st.state = "failed".into();
                st.error = Some(e);
            }
        }
    };
    if wait {
        work();
    } else if let Err(e) = std::thread::Builder::new().name("train".into()).spawn(work) {
        // the job mustn't stay "running" with nothing behind it
        let mut st = job.state.lock().unwrap_or_else(|e| e.into_inner());
        st.state = "failed".into();
        st.error = Some(format!("couldn't start the training: {e}"));
    }
    Ok(json!(job.snapshot()))
}

// ---------------------------------------------------------------- json

use super::s;

fn name_arg(args: &Value) -> Result<&str, String> {
    match s(args, "name").trim() {
        "" => Err("which classifier? (name)".into()),
        n => Ok(n),
    }
}

fn parse_ref(label: &str) -> Result<Ref, String> {
    Ref::parse(label).ok_or_else(|| format!("{label:?} isn't a key (like 0,12 or note:1000001)"))
}

/// a class given by name or index
fn class_arg(args: &Value, def: &clfs::Definition) -> Result<usize, String> {
    match args.get("class") {
        Some(Value::Number(n)) => {
            let i = n.as_u64().unwrap_or(u64::MAX) as usize;
            (i < def.class_names().len())
                .then_some(i)
                .ok_or_else(|| format!("{} has no class {i}", def.name))
        }
        Some(Value::String(c)) => def.class_index(c),
        _ => Err("which class?".into()),
    }
}

/// the text a call is about: `key` (a memory or note) or `text`
fn judged(store: &Store, args: &Value) -> Result<Judged, String> {
    let key = s(args, "key").trim();
    if !key.is_empty() {
        let r = parse_ref(key)?;
        let text = store.ref_text(&r)?.ok_or("didn't find that")?;
        return Ok(Judged { key: Some(r), text });
    }
    let text = s(args, "text");
    if text.trim().is_empty() {
        return Err("a text or a key, please".into());
    }
    Ok(Judged {
        key: None,
        text: text.to_string(),
    })
}

fn summary(store: &Store, d: &clfs::Definition) -> Value {
    let last = d.last_training();
    let m = last.and_then(|e| e.metrics.as_ref());
    json!({
        "name": d.name,
        "classes": d.class_names(),
        "lists": d.classes.iter().map(|c| &c.list).collect::<Vec<_>>(),
        "not": d.not,
        "head": head_name(d.head),
        "preset": d.preset,
        "gematria": d.gematria,
        "starter": d.starter,
        "inputs": d.inputs,
        "trained": d.trained(),
        "version": d.version,
        "accuracy": m.map(|m| m.accuracy),
        "macro_f1": m.map(|m| m.macro_f1),
        "examples": last.map(|e| e.examples),
        "outputs": d.outputs,
        "modified": d.modified,
        "training": training(store, &d.name),
    })
}

fn head_name(h: HeadKind) -> &'static str {
    match h {
        HeadKind::Single => "single",
        HeadKind::Multi => "multi",
        HeadKind::Regress => "regress",
    }
}

fn output_json(classes: &[String], o: &Output) -> Value {
    json!({
        "probs": o.probs,
        "top": o.top,
        "class": classes.get(o.top),
        "members": o.members.iter().filter_map(|&c| classes.get(c)).collect::<Vec<_>>(),
    })
}

fn details(store: &Store, d: &clfs::Definition) -> Res {
    let mut v = summary(store, d);
    let fb = store.classifier_feedback(&d.name)?;
    let net = store
        .classifier_weights(&d.name)?
        .and_then(|b| crate::classify::Classifier::from_bytes(&b).ok());
    let o = v.as_object_mut().expect("an object");
    o.insert("steer".into(), json!(d.steer));
    o.insert("history".into(), json!(d.history));
    o.insert("train".into(), json!(d.train));
    o.insert("created".into(), json!(d.created));
    o.insert("feedback_count".into(), json!(fb.len()));
    o.insert("undo".into(), json!(store.undo_count(&d.name)?));
    let recent: Vec<_> = fb.iter().rev().take(50).collect();
    o.insert("feedback".into(), json!(recent));
    o.insert(
        "size".into(),
        match net {
            Some(c) => json!({
                "weights": c.net.weight_count(),
                "d_model": c.net.cfg.d_model,
                "layers": c.net.blocks.len(),
                "blocks": c.net.blocks.iter().map(|b| [b.heads, b.ff]).collect::<Vec<_>>(),
                "max_len": c.net.cfg.max_len,
                "steps": c.meta.steps,
                "growth": c.meta.log,
            }),
            None => Value::Null,
        },
    );
    o.insert(
        "job".into(),
        job(store, &d.name).map_or(Value::Null, |j| json!(j.snapshot())),
    );
    Ok(v)
}

/// steering from the args over the current one: `bias` as a list or {class: value},
/// `temperature`, `threshold`, `gains` {letter, word, side, extra}
fn steer_from(def: &clfs::Definition, a: &Value) -> Result<crate::classify::Steer, String> {
    let mut st = def.steer.clone();
    let h = &mut st.heads[0];
    let names = def.class_names();
    match a.get("bias") {
        Some(Value::Array(v)) => {
            if v.len() != names.len() {
                return Err(format!("{} biases, please", names.len()));
            }
            h.bias = v.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
        }
        Some(Value::Object(m)) => {
            for (k, x) in m {
                let i = def.class_index(k)?;
                h.bias[i] = x.as_f64().ok_or("a bias is a number")? as f32;
            }
        }
        _ => {}
    }
    let num = |k: &str| a.get(k).and_then(Value::as_f64).map(|x| x as f32);
    if let Some(t) = num("temperature") {
        h.temperature = t;
    }
    if let Some(t) = num("threshold") {
        h.threshold = t;
    }
    if let Some(g) = a.get("gains") {
        let get = |k: &str, d: f32| g.get(k).and_then(Value::as_f64).map_or(d, |x| x as f32);
        let old = st.gains;
        st.gains = Gains {
            letter: get("letter", old.letter),
            word: get("word", old.word),
            side: get("side", old.side),
            extra: get("extra", old.extra),
        };
    }
    if a.get("reset").and_then(Value::as_bool) == Some(true) {
        st.gains = Gains::default();
        st.heads[0] = crate::classify::HeadSteer::neutral(names.len());
    }
    Ok(st)
}

/// texts to measure the bias on: the classifier's own training texts, or the save's
fn bias_texts(store: &Store, def: &clfs::Definition) -> Result<Vec<String>, String> {
    let mut t: Vec<String> = store.examples(def)?.into_iter().map(|e| e.text).collect();
    if t.is_empty() {
        t = store.candidates(300)?.into_iter().map(|c| c.1).collect();
    }
    let step = (t.len() / 300).max(1);
    Ok(t.into_iter().step_by(step).take(300).collect())
}

fn bias_json(stack: &Stack, texts: &[String]) -> Value {
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    if refs.is_empty() {
        return Value::Null;
    }
    json!(stack.bias(&refs, pool()))
}

fn train_args(args: &Value) -> TrainArgs {
    let u = |k: &str| args.get(k).and_then(Value::as_u64).map(|x| x as usize);
    TrainArgs {
        epochs: u("epochs"),
        lr: args.get("lr").and_then(Value::as_f64).map(|x| x as f32),
        batch: u("batch"),
        max_steps: u("max_steps"),
        threads: u("threads"),
        rounds: u("rounds"),
        fresh: args.get("fresh").and_then(Value::as_bool).unwrap_or(false),
        device: device_arg(args),
    }
}

/// `"device"`: cpu (the default), gpu or gpu_fast
pub(crate) fn device_arg(args: &Value) -> crate::classify::device::Device {
    use crate::classify::device::Device;
    match s(args, "device").trim() {
        "gpu" => Device::Gpu,
        "gpu_fast" | "gpu fast" => Device::GpuFast,
        _ => Device::Cpu,
    }
}

fn inputs_arg(args: &Value) -> Result<Vec<Input>, String> {
    let Some(v) = args.get("inputs").and_then(Value::as_array) else {
        return Ok(vec![]);
    };
    v.iter()
        .map(|i| match i {
            Value::String(n) => Ok(Input {
                from: n.clone(),
                coupling: 1.0,
            }),
            o => {
                serde_json::from_value(o.clone()).map_err(|e| format!("an input is a name or {{from, coupling}}: {e}"))
            }
        })
        .collect()
}

// ---------------------------------------------------------------- the calls

/// `classify.device`: where classifiers read texts (predict, search, indexing, buckets,
/// tints), for this process; `{device}` sets it (auto, cpu, gpu, gpu_fast). `speeds` are
/// the rates auto measured on this machine, per model size; `{measure_again: true}`
/// forgets them, so each size is measured again on its next use
fn device_call(args: &Value, limits: &Limits) -> Res {
    use crate::classify::device::{self, Choice};
    if let Some(d) = args.get("device").and_then(Value::as_str) {
        if limits.shared {
            return Err("a shared hub decides where it reads texts".into());
        }
        let c = Choice::parse(d).ok_or_else(|| format!("{d:?} isn't auto, cpu, gpu or gpu_fast"))?;
        device::set_choice(c);
    }
    #[cfg(feature = "gpu")]
    let machine = crate::classify::speed::machine().ok();
    #[cfg(not(feature = "gpu"))]
    let machine: Option<String> = None;
    if args.get("measure_again").and_then(Value::as_bool) == Some(true) {
        if limits.shared {
            return Err("a shared hub decides where it reads texts".into());
        }
        if let Some(m) = &machine {
            crate::classify::speed::forget(m);
        }
    }
    let c = device::choice();
    let speeds: Vec<Value> = machine
        .as_deref()
        .map(crate::classify::speed::known)
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let mut v = json!(s);
            v["words"] = json!(s.words());
            v
        })
        .collect();
    Ok(json!({
        "device": c.name(),
        "reading": device::reading(c, limits.gpu),
        "choices": ["auto", "cpu", "gpu", "gpu_fast"],
        "speeds": speeds,
    }))
}

/// a starter's summary
#[cfg(not(target_arch = "wasm32"))]
fn starter_json(s: &crate::starters::Starter, books: &Path) -> Value {
    let mut v = json!(s);
    if s.source == crate::starters::Source::Fetched {
        v["fetched"] = json!(crate::starters::fetched(books));
        v["urls"] = json!(
            crate::starters::BOOKS
                .iter()
                .map(|(n, _)| crate::starters::book_url(*n))
                .collect::<Vec<_>>()
        );
    }
    v
}

/// the starter classifiers (`crate::starters`): `classify.starters` lists them,
/// `classify.starter_fetch {id}` fetches a fetched one's texts, `classify.starter {id,
/// name?, preset?, train?: true, epochs?, wait?}` writes its training texts as lists and
/// makes (and trains) a classifier on them, and `classify.starter_score {name}` scores a
/// classifier made from one on the starter's held-out texts
#[cfg(not(target_arch = "wasm32"))]
fn starter_call(saves: &Saves, op: &str, args: &Value, limits: &Limits) -> Res {
    use crate::starters::{self, Split};
    let books = saves.dir().join("starters");
    match op {
        "classify.starters" => Ok(Value::Array(
            starters::STARTERS.iter().map(|s| starter_json(s, &books)).collect(),
        )),
        "classify.starter_fetch" => {
            let id = s(args, "id");
            let sp = starters::get(id).ok_or_else(|| format!("no starter {id:?}"))?;
            if sp.source != starters::Source::Fetched {
                return Err(format!("{:?} has nothing to fetch", sp.title));
            }
            let got = starters::fetch(&books, &starters::http_get)?;
            let mut v = starter_json(sp, &books);
            v["got"] = json!(got);
            Ok(v)
        }
        "classify.starter" => {
            let id = s(args, "id");
            let sp = starters::get(id).ok_or_else(|| format!("no starter {id:?}"))?;
            let (_, models) = saves.models()?;
            let (_, g) = saves.graph("")?;
            let name = match s(args, "name").trim() {
                "" => sp.title.to_string(),
                n => n.to_string(),
            };
            if models.classifier(&name).is_ok() {
                return Err(format!("there's already a classifier called {name:?}"));
            }
            let per = args
                .get("per_class")
                .and_then(Value::as_u64)
                .map_or(starters::TRAIN_PER_CLASS, |n| (n as usize).clamp(10, 5000));
            let ex = starters::examples_with(id, Split::Train, Some(&books), per)?;
            // a list per class, headed "<title>: <class>", holding its training texts
            let heads = g.write(|tx| {
                let mut heads = Vec::new();
                for class in sp.classes {
                    heads.push(
                        tx.save_text(&format!("{}: {class}", sp.title), crate::ops::mime::TEXT)?
                            .0,
                    );
                }
                for (text, c) in &ex {
                    let (m, _) = tx.save_text(text, crate::ops::mime::TEXT)?;
                    tx.add_member(heads[*c], m)?;
                }
                Ok(heads)
            })?;
            let refs = heads
                .iter()
                .map(|h| parse_ref(&crate::graph::record::label(*h)))
                .collect::<Result<Vec<_>, _>>()?;
            let preset = match s(args, "preset") {
                "" => clfs::DEFAULT_PRESET,
                p => p,
            };
            models.create_classifier_with(&name, &refs, None, HeadKind::Single, preset, vec![], sp.gematria)?;
            let mut def = models.classifier(&name)?;
            def.starter = Some(id.to_string());
            // longer than a hand-made classifier's default: early stopping on a few dozen
            // held-back texts otherwise ends it within a second (core/examples/starters.rs)
            def.train.epochs = def.train.epochs.max(40);
            def.train.patience = def.train.patience.max(10);
            models.put_classifier(&mut def, None)?;
            let job = if args.get("train").and_then(Value::as_bool).unwrap_or(true) {
                let wait = args.get("wait").and_then(Value::as_bool).unwrap_or(false);
                start(models.clone(), &name, &train_args(args), wait, limits)?
            } else {
                Value::Null
            };
            Ok(json!({ "classifier": details(&models, &def)?, "job": job, "texts": ex.len() }))
        }
        "classify.starter_score" => {
            let (_, models) = saves.models()?;
            models.allow_gpu_reads(limits.gpu);
            let stack = models.stack(name_arg(args)?)?;
            let def = stack.def();
            let id = def
                .starter
                .clone()
                .ok_or_else(|| format!("{:?} wasn't made from a starter", def.name))?;
            let sp = starters::get(&id).ok_or_else(|| format!("no starter {id:?}"))?;
            if !def.trained() {
                return Err(format!("train {:?} first", def.name));
            }
            let held = starters::examples(&id, Split::Held, Some(&books))?;
            let texts: Vec<&str> = held.iter().map(|(t, _)| t.as_str()).collect();
            let started = std::time::Instant::now();
            let outs = stack.outputs(&texts, pool());
            let seconds = started.elapsed().as_secs_f64();
            let mut per = vec![(0usize, 0usize); sp.classes.len()];
            for ((_, c), o) in held.iter().zip(&outs) {
                per[*c].0 += 1;
                if o.top == *c {
                    per[*c].1 += 1;
                }
            }
            let right: usize = per.iter().map(|p| p.1).sum();
            Ok(json!({
                "name": def.name, "starter": id, "title": sp.title,
                "texts": held.len(), "right": right,
                "accuracy": right as f64 / held.len().max(1) as f64,
                "classes": sp.classes.iter().zip(&per).map(|(c, (n, r))| json!({ "class": c, "texts": n, "right": r })).collect::<Vec<_>>(),
                "seconds": seconds,
                "device": stack.reading(),
            }))
        }
        _ => Err(format!("no call {op:?}")),
    }
}

/// `gpu.report`: the machine's GPU, the exactness probe and timings next to the CPU
/// (`classify::report`); `{texts, steps, sizes}` shrink it. Not on a shared hub: it
/// keeps the machine busy for a while.
pub fn gpu_report(args: &Value, limits: &Limits) -> Res {
    if limits.shared {
        return Err("the GPU report is for a machine of your own (the desktop, or a --local hub)".into());
    }
    let mut o = crate::classify::report::Opts::default();
    if let Some(n) = args.get("texts").and_then(Value::as_u64) {
        o.texts = (n as usize).clamp(1, 100_000);
    }
    if let Some(n) = args.get("steps").and_then(Value::as_u64) {
        o.steps = (n as usize).clamp(1, 1000);
    }
    if let Some(n) = args.get("budget").and_then(Value::as_f64) {
        o.budget = n.clamp(0.0, 3600.0);
    }
    if let Some(v) = args.get("sizes").and_then(Value::as_array) {
        o.sizes = v.iter().filter_map(Value::as_str).map(String::from).collect();
    }
    Ok(crate::classify::report::report(&o))
}

pub fn call(store: Arc<Store>, op: &str, args: &Value, limits: &Limits) -> Res {
    let st: &Store = &store;
    st.allow_gpu_reads(limits.gpu);
    match op {
        "classify.device" => device_call(args, limits),
        "classify.list" => Ok(Value::Array(st.classifiers()?.iter().map(|d| summary(st, d)).collect())),
        "classify.create" => {
            let lists = args
                .get("classes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let classes = lists
                .iter()
                .map(|l| parse_ref(l.as_str().unwrap_or("")))
                .collect::<Result<Vec<_>, _>>()?;
            let not = match s(args, "not").trim() {
                "" => None,
                l => Some(parse_ref(l)?),
            };
            let head = match s(args, "head") {
                "" | "single" => HeadKind::Single,
                "multi" => HeadKind::Multi,
                h => return Err(format!("a head is single or multi, not {h:?}")),
            };
            let preset = match s(args, "preset") {
                "" => clfs::DEFAULT_PRESET,
                p => p,
            };
            let gematria = args.get("gematria").and_then(Value::as_bool).unwrap_or(false);
            let def = st.create_classifier_with(
                name_arg(args)?,
                &classes,
                not.as_ref(),
                head,
                preset,
                inputs_arg(args)?,
                gematria,
            )?;
            details(st, &def)
        }
        "classify.get" => details(st, &st.classifier(name_arg(args)?)?),
        // how it trains from now on: the share of examples held out to stop early by
        // (0 to 0.5), and how many evaluations without a better one it waits
        "classify.set_train" => {
            let name = name_arg(args)?;
            not_training(st, &[name.to_string()])?;
            let mut def = st.classifier(name)?;
            if let Some(v) = args.get("val_fraction").and_then(Value::as_f64) {
                if !(0.0..=0.5).contains(&v) {
                    return Err("the share held out goes from 0 to 0.5".into());
                }
                def.train.val_fraction = v as f32;
            }
            if let Some(p) = args.get("patience").and_then(Value::as_u64) {
                def.train.patience = (p as usize).clamp(1, 10_000);
            }
            st.put_classifier(&mut def, None)?;
            details(st, &def)
        }
        "classify.delete" => {
            let name = name_arg(args)?;
            not_training(st, &[name.to_string()])?;
            st.delete_classifier(name)?;
            jobs()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&job_key(st, name));
            Ok(Value::Null)
        }
        "classify.couple" => {
            let name = name_arg(args)?;
            let mut def = st.classifier(name)?;
            let from = s(args, "from");
            let c = args
                .get("coupling")
                .and_then(Value::as_f64)
                .ok_or("coupling is a number")? as f32;
            if !(0.0..=10.0).contains(&c) {
                return Err("coupling from 0 to 10".into());
            }
            let i = def
                .inputs
                .iter_mut()
                .find(|i| i.from == from)
                .ok_or_else(|| format!("{name:?} doesn't build on {from:?}"))?;
            i.coupling = c;
            st.put_classifier(&mut def, None)?;
            Ok(summary(st, &def))
        }

        // ---- training
        "classify.train" => {
            let name = name_arg(args)?;
            let wait = args.get("wait").and_then(Value::as_bool).unwrap_or(false);
            start(store.clone(), name, &train_args(args), wait, limits)
        }
        "classify.job" => {
            let name = name_arg(args)?;
            Ok(match job(st, name) {
                Some(j) => json!(j.snapshot()),
                None => json!({ "name": name, "state": "idle" }),
            })
        }
        "classify.cancel" => {
            let name = name_arg(args)?;
            let j = job(st, name).filter(|j| j.running()).ok_or("it isn't training")?;
            j.cancel.store(true, Ordering::Relaxed);
            Ok(json!(j.snapshot()))
        }

        // ---- reading texts
        "classify.predict" => {
            let stack = st.stack(name_arg(args)?)?;
            let classes = stack.def().class_names();
            let mut items: Vec<(Option<Ref>, String)> = Vec::new();
            for k in args.get("keys").and_then(Value::as_array).into_iter().flatten() {
                let r = parse_ref(k.as_str().unwrap_or(""))?;
                let t = st.ref_text(&r)?.ok_or_else(|| format!("didn't find {}", r.label()))?;
                items.push((Some(r), t));
            }
            for t in args.get("texts").and_then(Value::as_array).into_iter().flatten() {
                items.push((None, t.as_str().unwrap_or("").to_string()));
            }
            if items.len() > SCAN {
                return Err(format!("at most {SCAN} texts at once"));
            }
            let texts: Vec<&str> = items.iter().map(|i| i.1.as_str()).collect();
            let outs = stack.outputs(&texts, pool());
            let results: Vec<Value> = items
                .iter()
                .zip(&outs)
                .map(|((r, t), o)| {
                    let mut v = output_json(&classes, o);
                    v["text"] = json!(t);
                    v["key"] = json!(r.as_ref().map(Ref::label));
                    v
                })
                .collect();
            Ok(
                json!({ "classes": classes, "trained": stack.def().trained(), "results": results, "device": stack.reading() }),
            )
        }
        "classify.search" => {
            let stack = st.stack(name_arg(args)?)?;
            let def = stack.def();
            let c = class_arg(args, def)?;
            let classes = def.class_names();
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 1000) as usize;
            let mut cands = st.candidates(SCAN)?;
            let q = s(args, "q").trim();
            if !q.is_empty() {
                let found: HashSet<Ref> = st.search_with_variations(q)?.into_iter().map(|h| h.r).collect();
                cands.retain(|c| found.contains(&c.0));
            }
            // a list's head isn't one of its texts
            let heads: HashSet<String> = def.classes.iter().map(|c| c.list.clone()).collect();
            cands.retain(|c| !heads.contains(&c.0.label()));
            let texts: Vec<&str> = cands.iter().map(|c| c.1.as_str()).collect();
            let outs = stack.outputs(&texts, pool());
            let mut order: Vec<usize> = (0..cands.len()).collect();
            order.sort_by(|&a, &b| outs[b].probs[c].total_cmp(&outs[a].probs[c]).then(a.cmp(&b)));
            let results: Vec<Value> = order
                .into_iter()
                .take(limit)
                .map(|i| {
                    let (r, _, shown) = &cands[i];
                    let o = &outs[i];
                    json!({
                        "key": r.label(), "text": shown, "note": st.is_note(r),
                        "prob": o.probs[c], "top": o.top, "top_class": classes.get(o.top),
                        "member": o.members.contains(&c),
                    })
                })
                .collect();
            Ok(json!({
                "classes": classes, "class": classes[c], "stack": stack.names(),
                "trained": def.trained(), "scanned": cands.len(), "results": results,
                "device": stack.reading(),
            }))
        }
        "classify.contributions" => {
            let stack = st.stack(name_arg(args)?)?;
            let j = judged(st, args)?;
            let def = stack.def();
            let c = match args.get("class") {
                Some(_) => class_arg(args, def)?,
                None => stack.graph.predict(stack.node, &j.text)[0].top,
            };
            let rows: Vec<Value> = stack
                .contributions(&j.text, c)?
                .into_iter()
                .map(|(k, o)| {
                    let names = stack.defs[k.node].class_names();
                    json!({
                        "name": k.name, "share": k.share, "input_share": k.input_share, "grad_norm": k.grad_norm,
                        "classes": names, "probs": o.probs, "top": o.top, "top_class": names.get(o.top),
                        "judged": k.node == stack.node,
                    })
                })
                .collect();
            Ok(json!({ "class": def.class_names()[c], "text": j.text, "results": rows }))
        }
        "classify.represent" => {
            let stack = st.stack(name_arg(args)?)?;
            let j = judged(st, args)?;
            Ok(json!({ "vector": stack.clf().represent(&j.text) }))
        }
        "classify.similar" => {
            let stack = st.stack(name_arg(args)?)?;
            let j = judged(st, args)?;
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 500) as usize;
            let clf = stack.clf();
            let me = clf.represent(&j.text);
            let cands: Vec<_> = st
                .candidates(SCAN)?
                .into_iter()
                .filter(|c| Some(&c.0) != j.key.as_ref())
                .collect();
            let texts: Vec<&str> = cands.iter().map(|c| c.1.as_str()).collect();
            let vs = clf.represent_many(&texts, pool());
            let cos: Vec<f32> = vs.iter().map(|v| crate::classify::cosine(&me, v)).collect();
            let mut order: Vec<usize> = (0..cands.len()).collect();
            order.sort_by(|&a, &b| cos[b].total_cmp(&cos[a]).then(a.cmp(&b)));
            let results: Vec<Value> = order
                .into_iter()
                .take(limit)
                .map(|i| {
                    let (r, _, shown) = &cands[i];
                    json!({ "key": r.label(), "text": shown, "note": st.is_note(r), "cosine": cos[i] })
                })
                .collect();
            Ok(json!({ "text": j.text, "results": results }))
        }

        // ---- the player's knobs and verdicts
        "classify.steer" => {
            let name = name_arg(args)?;
            let def = st.classifier(name)?;
            let steer = steer_from(&def, args.get("steer").unwrap_or(&Value::Null))?;
            clfs::check_steer(&steer, def.class_names().len())?;
            let save = args.get("save").and_then(Value::as_bool).unwrap_or(true);
            let def = if save {
                st.steer_classifier(name, steer)?
            } else {
                let mut d = def;
                d.steer = steer;
                d
            };
            let mut stack = st.stack(name)?;
            let node = stack.node;
            stack.graph.nodes[node].clf.steer = def.steer.clone();
            let texts = bias_texts(st, &def)?;
            Ok(json!({
                "steer": def.steer, "classes": def.class_names(), "saved": save,
                "bias": if def.trained() { bias_json(&stack, &texts) } else { Value::Null },
            }))
        }
        "classify.feedback" | "classify.feedback_stack" => {
            let name = name_arg(args)?;
            let def = st.classifier(name)?;
            let c = class_arg(args, &def)?;
            let class = def.class_names()[c].clone();
            let j = judged(st, args)?;
            let yes = args.get("yes").and_then(Value::as_bool).ok_or("yes: true or false")?;
            let weight = args.get("weight").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            if !(weight > 0.0 && weight <= 10.0) {
                return Err("weight from above 0 to 10".into());
            }
            if op == "classify.feedback" {
                not_training(st, &[name.to_string()])?;
                let e = st.feedback_classifier(name, &j, &class, yes, weight)?;
                Ok(json!({ "entry": e }))
            } else {
                not_training(st, &st.stack(name)?.names())?;
                let (e, rep, touched) = st.feedback_stack(name, &j, &class, yes, weight)?;
                Ok(json!({ "entry": e, "report": rep, "touched": touched }))
            }
        }
        "classify.unfeedback" => {
            let name = name_arg(args)?;
            not_training(st, &st.stack(name)?.names())?;
            let e = st.unfeedback(name)?;
            Ok(json!({ "entry": e, "left": st.undo_count(name)? }))
        }
        "classify.enlist" => {
            let name = name_arg(args)?;
            let def = st.classifier(name)?;
            let c = class_arg(args, &def)?;
            let threshold = args.get("threshold").and_then(Value::as_f64).map(|x| x as f32);
            let (head, n) = st.enlist_outputs(name, &def.class_names()[c], threshold, SCAN)?;
            let text = st.remember(&head).map(|h| h.text).unwrap_or_default();
            Ok(json!({ "list": head.label(), "name": text, "members": n }))
        }
        _ => phase3(store.clone(), op, args, limits),
    }
}

// ---------------------------------------------------------------- buckets, filters, modules

/// indexing jobs by save file and classifier
fn index_jobs() -> &'static Mutex<HashMap<String, Arc<IndexJob>>> {
    static JOBS: OnceLock<Mutex<HashMap<String, Arc<IndexJob>>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

#[derive(Default)]
pub struct IndexJob {
    cancel: AtomicBool,
    state: Mutex<Value>,
}

fn index_snapshot(j: &IndexJob) -> Value {
    j.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn bands_arg(args: &Value) -> std::ops::RangeInclusive<u32> {
    match args.get("band").and_then(Value::as_u64) {
        Some(b) => (b as u32)..=(b as u32),
        None => {
            let lo = args
                .get("min")
                .and_then(Value::as_f64)
                .map_or(0, |p| memo::band(p as f32));
            lo..=9
        }
    }
}

fn hex_colour(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn row_json(classes: &[String], r: &memo::Row) -> Value {
    json!({
        "probs": r.probs, "top": r.top, "class": classes.get(r.top),
        "members": r.members.iter().filter_map(|&c| classes.get(c)).collect::<Vec<_>>(),
        "confidence": r.confidence(),
    })
}

fn phase3(store: Arc<Store>, op: &str, args: &Value, limits: &Limits) -> Res {
    let st: &Store = &store;
    match op {
        "classify.index" => {
            let name = name_arg(args)?.to_string();
            let stack = st.stack(&name)?;
            if !stack.def().trained() {
                return Err(format!("train {name:?} first"));
            }
            let refs: Option<Vec<Ref>> = match args.get("keys").and_then(Value::as_array) {
                Some(v) => Some(
                    v.iter()
                        .map(|k| parse_ref(k.as_str().unwrap_or("")))
                        .collect::<Result<_, _>>()?,
                ),
                None => None,
            };
            room_for_a_job(st, limits)?;
            let key = job_key(st, &name);
            let job = Arc::new(IndexJob {
                cancel: AtomicBool::new(false),
                state: Mutex::new(json!({ "name": name, "state": "running", "done": 0, "total": 0 })),
            });
            {
                let mut all = index_jobs().lock().unwrap_or_else(|e| e.into_inner());
                if all.get(&key).is_some_and(|j| index_snapshot(j)["state"] == "running") {
                    return Err(format!("{name:?} is being indexed already"));
                }
                all.insert(key, job.clone());
            }
            let j = job.clone();
            let store = store.clone();
            let wait = args.get("wait").and_then(Value::as_bool).unwrap_or(false);
            let work = move || {
                let r = store.memo_index(&stack, refs.as_deref(), SCAN, pool(), |done, total| {
                    let mut s = j.state.lock().unwrap_or_else(|e| e.into_inner());
                    s["done"] = json!(done);
                    s["total"] = json!(total);
                    if j.cancel.load(Ordering::Relaxed) {
                        Flow::Stop
                    } else {
                        Flow::Go
                    }
                });
                let mut s = j.state.lock().unwrap_or_else(|e| e.into_inner());
                match r {
                    Ok(rep) => {
                        s["state"] = json!(if rep.stopped { "cancelled" } else { "done" });
                        s["report"] = json!(rep);
                        s["device"] = json!(stack.reading());
                    }
                    Err(e) => {
                        s["state"] = json!("failed");
                        s["error"] = json!(e);
                    }
                }
            };
            if wait {
                work();
            } else {
                if let Err(e) = std::thread::Builder::new().name("index".into()).spawn(work) {
                    let mut s = job.state.lock().unwrap_or_else(|e| e.into_inner());
                    s["state"] = json!("failed");
                    s["error"] = json!(e.to_string());
                }
            }
            Ok(index_snapshot(&job))
        }
        "classify.index_job" => {
            let name = name_arg(args)?;
            let j = index_jobs()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&job_key(st, name))
                .cloned();
            Ok(match j {
                Some(j) => index_snapshot(&j),
                None => json!({ "name": name, "state": "idle" }),
            })
        }
        "classify.index_cancel" => {
            let name = name_arg(args)?;
            let j = index_jobs()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&job_key(st, name))
                .cloned()
                .ok_or("it isn't being indexed")?;
            j.cancel.store(true, Ordering::Relaxed);
            Ok(index_snapshot(&j))
        }
        "classify.buckets" => {
            let stack = st.stack(name_arg(args)?)?;
            let classes = stack.def().class_names();
            let counts = st.bucket_counts(&stack)?;
            let mut per: Vec<Value> = Vec::new();
            for (i, c) in classes
                .iter()
                .enumerate()
                .map(|(i, c)| (i as u32, c.clone()))
                .chain([(memo::UNCERTAIN, "uncertain".to_string())])
            {
                let bands: Vec<usize> = (0..memo::BANDS)
                    .map(|b| counts.iter().find(|x| x.0 == (i, b)).map_or(0, |x| x.1))
                    .collect();
                per.push(json!({ "class": c, "index": if i == memo::UNCERTAIN { Value::Null } else { json!(i) }, "bands": bands, "count": bands.iter().sum::<usize>() }));
            }
            Ok(json!({
                "name": stack.def().name, "classes": classes, "indexed": st.memo_count(&stack)?,
                "palette": memo::palette(classes.len()).into_iter().map(hex_colour).collect::<Vec<_>>(),
                "buckets": per,
            }))
        }
        "classify.bucket" => {
            let stack = st.stack(name_arg(args)?)?;
            let def = stack.def();
            let classes = def.class_names();
            let c = if args.get("class").is_some() {
                match args.get("class") {
                    Some(Value::String(u)) if u == "uncertain" => memo::UNCERTAIN,
                    _ => class_arg(args, def)? as u32,
                }
            } else {
                let j = judged(st, args)?;
                let r = memo::read(&stack, memo::stamp(&stack), &j.text);
                if r.members.is_empty() {
                    memo::UNCERTAIN
                } else {
                    r.top as u32
                }
            };
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(500).clamp(1, 5000) as usize;
            let (filed, stale) = st.class_bucket(&stack, c, bands_arg(args))?;
            let hits: Vec<Value> = filed
                .iter()
                .take(limit)
                .map(|f| {
                    let (o, r) = crate::spread::sums(&f.text);
                    json!({ "key": f.r.label(), "text": f.text, "note": st.is_note(&f.r), "ordinal": o, "reversed": r,
                        "prob": if c == memo::UNCERTAIN { f.row.confidence() } else { f.row.probs[c as usize] } })
                })
                .collect();
            Ok(json!({
                "class": if c == memo::UNCERTAIN { "uncertain".to_string() } else { classes[c as usize].clone() },
                "count": filed.len(), "stale": stale, "indexed": st.memo_count(&stack)?, "hits": hits,
            }))
        }
        "classify.near" => {
            let stack = st.stack(name_arg(args)?)?;
            let j = judged(st, args)?;
            let by = if s(args, "by") == "outputs" {
                memo::By::Outputs
            } else {
                memo::By::Embedding
            };
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 500) as usize;
            let me = match &j.key {
                Some(k) => st.memo_rows(&stack, &[(k.clone(), j.text.as_str())], pool())?.remove(0),
                None => memo::read(&stack, st.memo_fresh(&stack)?, &j.text),
            };
            let found = st.memo_near(&stack, &me, by, j.key.as_ref(), limit)?;
            Ok(json!({
                "text": j.text, "indexed": st.memo_count(&stack)?, "row": row_json(&stack.def().class_names(), &me),
                "results": found.iter().map(|(r, t, c)| json!({ "key": r.label(), "text": t, "note": st.is_note(r), "cosine": c })).collect::<Vec<_>>(),
            }))
        }
        "classify.tints" => {
            let stack = st.stack(name_arg(args)?)?;
            if !stack.def().trained() {
                return Err("train it first".into());
            }
            let keys = args.get("keys").and_then(Value::as_array).cloned().unwrap_or_default();
            if keys.len() > 5000 {
                return Err("at most 5000 keys".into());
            }
            let refs: Vec<Ref> = keys
                .iter()
                .map(|k| parse_ref(k.as_str().unwrap_or("")))
                .collect::<Result<_, _>>()?;
            let tints = st.tints(&stack, &refs, pool())?;
            let classes = stack.def().class_names();
            Ok(json!({
                "classes": classes,
                "palette": memo::palette(classes.len()).into_iter().map(hex_colour).collect::<Vec<_>>(),
                "tints": refs.iter().zip(tints).map(|(r, t)| json!({ "key": r.label(), "class": t.as_ref().and_then(|t| t.class), "strength": t.map(|t| t.strength) })).collect::<Vec<_>>(),
                "device": stack.reading(),
            }))
        }
        "classify.indexed" => {
            let r = parse_ref(s(args, "key"))?;
            let text = st.ref_text(&r)?.ok_or("didn't find that")?;
            let chosen = st.indexed_on(&r)?;
            let mut verdicts = Vec::new();
            for n in &chosen {
                let Ok(stack) = st.stack(n) else { continue };
                let classes = stack.def().class_names();
                if !stack.def().trained() {
                    verdicts.push(json!({ "name": n, "classes": classes, "trained": false }));
                    continue;
                }
                let (row, chk) = st
                    .checked_rows(&stack, &[(r.clone(), text.as_str())], pool())?
                    .remove(0);
                let mut v = row_json(&classes, &row);
                v["name"] = json!(n);
                v["classes"] = json!(classes);
                v["trained"] = json!(true);
                v["valid"] = json!(chk.valid);
                v["checked"] = json!(chk.probs);
                v["validators"] = json!(
                    st.validators(n)?
                        .iter()
                        .map(|x| &x.by)
                        .zip(chk.checks)
                        .collect::<Vec<_>>()
                );
                v["palette"] = json!(
                    memo::palette(classes.len())
                        .into_iter()
                        .map(hex_colour)
                        .collect::<Vec<_>>()
                );
                verdicts.push(v);
            }
            let all: Vec<Value> = st
                .classifiers()?
                .iter()
                .map(|d| json!({ "name": d.name, "trained": d.trained(), "chosen": chosen.contains(&d.name) }))
                .collect();
            Ok(json!({ "key": r.label(), "chosen": chosen, "verdicts": verdicts, "classifiers": all }))
        }
        "classify.set_index" => {
            let r = parse_ref(s(args, "key"))?;
            st.ref_text(&r)?.ok_or("didn't find that")?;
            let names: Vec<String> = args
                .get("names")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect();
            Ok(json!({ "chosen": st.set_indexed_on(&r, &names)? }))
        }
        "classify.validators" => {
            let name = name_arg(args)?;
            if let Some(v) = args.get("set") {
                let vals: Vec<filters::Validator> = serde_json::from_value(v.clone())
                    .map_err(|e| format!("validators are [{{by, class, min, mode}}]: {e}"))?;
                st.set_validators(name, &vals)?;
            }
            Ok(json!({ "validators": st.validators(name)? }))
        }
        "classify.disagreements" => {
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(40).clamp(1, 500) as usize;
            Ok(json!({ "results": st.disagreements(name_arg(args)?, limit, SCAN, pool())? }))
        }
        "classify.export" => {
            let name = name_arg(args)?;
            let o = clsmodule::ExportOpts {
                description: s(args, "description").to_string(),
                author: s(args, "author").to_string(),
                upstream: args.get("upstream").and_then(Value::as_bool).unwrap_or(true),
            };
            let b = st.export_module(name, &o)?;
            Ok(json!({ "file": format!("{name}.{}", clsmodule::EXTENSION), "size": b.len(), "data": clfs::base64(&b) }))
        }
        "classify.import" => {
            let b = clfs::unbase64(s(args, "data")).ok_or("data is the module in base64")?;
            let m = clsmodule::read(&b, MODULE_MAX)?;
            import_opts_and_take(st, &m, args)
        }
        "classify.inspect" => {
            let b = clfs::unbase64(s(args, "data")).ok_or("data is the module in base64")?;
            Ok(json!(clsmodule::summary(&clsmodule::read(&b, MODULE_MAX)?)))
        }

        // ---- filters
        "filters.list" => Ok(Value::Array(
            st.filter_presets()?
                .iter()
                .map(|p| {
                    let mut v = json!(p);
                    v["reads"] = json!(p.expr.describe());
                    v
                })
                .collect(),
        )),
        "filters.save" => {
            let e: filters::Expr = serde_json::from_value(args.get("expr").cloned().unwrap_or_default())
                .map_err(|e| format!("that filter doesn't read: {e}"))?;
            Ok(json!(st.save_filter(s(args, "name"), e, s(args, "description"))?))
        }
        "filters.delete" => {
            st.delete_filter(s(args, "name"))?;
            Ok(Value::Null)
        }
        "filters.run" => {
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(200).clamp(1, 5000) as usize;
            let (e, preset) = match args.get("expr") {
                Some(e) if !e.is_null() => (
                    serde_json::from_value(e.clone()).map_err(|e| format!("that filter doesn't read: {e}"))?,
                    None,
                ),
                _ => {
                    let p = st.filter_preset(s(args, "name"))?;
                    (p.expr, Some(p.name))
                }
            };
            let r = st.run_filter(&e, limit, SCAN, pool())?;
            if let Some(n) = preset {
                st.note_filter_count(&n, r.matched)?;
            }
            Ok(json!(r))
        }
        "filters.enlist" => {
            let (head, n) = st.enlist_filter(s(args, "name"), SCAN, pool())?;
            Ok(
                json!({ "list": head.label(), "name": st.remember(&head).map(|h| h.text).unwrap_or_default(), "members": n }),
            )
        }
        _ => Err(format!("there's no {op:?} here")),
    }
}

/// the largest module taken through the app's calls
pub const MODULE_MAX: usize = 64 << 20;

/// takes a read module into the save with the call's `name` and `lists` ({class: key})
pub fn import_opts_and_take(st: &Store, m: &clsmodule::Module, args: &Value) -> Res {
    let lists = args
        .get("lists")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let o = clsmodule::ImportOpts {
        name: Some(s(args, "as").trim()).filter(|n| !n.is_empty()).map(str::to_string),
        lists,
    };
    Ok(json!(st.import_module(m, &o)?))
}

// ---------------------------------------------------------------- a save's models

/// the extension of a save's classifiers file, `<save>.models` beside `<save>.redb`
pub const MODELS: &str = "models";

/// every classifiers store open in this process, by file (redb holds a file once)
fn open_models() -> &'static Mutex<HashMap<PathBuf, Weak<Store>>> {
    static M: OnceLock<Mutex<HashMap<PathBuf, Weak<Store>>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// a save's classifiers (`path`, its `.models` file), reading its graph's texts; shared
/// with whoever has them open already (a training thread, say)
pub fn models_at(path: &Path, graph: Arc<Graph>) -> Result<Arc<Store>, String> {
    // the last holder may still be closing the file: its handle is gone from the map but
    // redb hasn't let go of the lock yet, so wait a moment
    for _ in 0..200 {
        let mut all = open_models().lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|_, w| w.strong_count() > 0);
        if let Some(s) = all.get(path).and_then(Weak::upgrade) {
            return Ok(s);
        }
        match Store::open_over(path, Arc::new(GraphTexts(graph.clone()))) {
            Ok(s) => {
                // spec model records kept here by older saves move into the save
                crate::specs::migrate(&graph, &s)?;
                let s = Arc::new(s);
                all.insert(path.to_path_buf(), Arc::downgrade(&s));
                return Ok(s);
            }
            Err(e) if e.contains("already open") => {
                drop(all);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(e) => return Err(e),
        }
    }
    Err("the classifiers are busy closing; try again".into())
}

impl Saves {
    /// a save's name and classifiers (the open save's when `name` is empty)
    pub fn models_of(&self, name: &str) -> Result<(String, Arc<Store>), String> {
        let (n, g) = self.graph(name)?;
        let path = self.companion(&n, MODELS)?;
        Ok((n, models_at(&path, g)?))
    }

    /// the open save's name and classifiers
    pub fn models(&self) -> Result<(String, Arc<Store>), String> {
        self.models_of("")
    }
}

/// whether a [`Saves`] with models answers `op` here
pub fn routed(op: &str) -> bool {
    op.starts_with("classify.")
        || op.starts_with("filters.")
        || op == "gpu.report"
        || op == "saves.export"
        || op == "saves.import"
}

/// the ops that need a save's models: `classify.*` and `filters.*` on the open save's,
/// and bundles (`saves.export`, `saves.import`) carrying the classifiers
pub fn saves_call(saves: &Saves, op: &str, args: &Value, limits: &Limits) -> Res {
    if op == "gpu.report" {
        return gpu_report(args, limits);
    }
    // the browser build reaches the starters through the hub
    #[cfg(not(target_arch = "wasm32"))]
    if op.starts_with("classify.starter") {
        return starter_call(saves, op, args, limits);
    }
    if op.starts_with("classify.") || op.starts_with("filters.") {
        let (_, models) = saves.models()?;
        return call(models, op, args, limits);
    }
    match op {
        "saves.export" => {
            let mut b = saves.call_plain(op, args)?;
            let (_, models) = saves.models_of(s(args, "name"))?;
            let (_, g) = saves.graph(s(args, "name"))?;
            let cls = models.export_classifiers()?;
            if !cls.is_empty() {
                b["classifiers"] = serde_json::to_value(cls).map_err(|e| e.to_string())?;
            }
            let specs = crate::specs::export(&g, &models)?;
            if !specs.is_empty() {
                b["models"] = serde_json::to_value(specs).map_err(|e| e.to_string())?;
            }
            Ok(b)
        }
        "saves.import" => {
            let cls: Vec<clfs::Bundled> = match args.get("bundle").and_then(|b| b.get("classifiers")) {
                Some(c) => serde_json::from_value(c.clone())
                    .map_err(|e| format!("that bundle's classifiers don't read: {e}"))?,
                None => Vec::new(),
            };
            let specs: Vec<crate::specs::BundledSpec> = match args.get("bundle").and_then(|b| b.get("models")) {
                Some(c) => serde_json::from_value(c.clone())
                    .map_err(|e| format!("that bundle's spec models don't read: {e}"))?,
                None => Vec::new(),
            };
            let mut got = saves.call_plain(op, args)?;
            // merged: the classifiers stay out (their lists would point elsewhere); the
            // spec models came in with the save's tables (`merge::assets`), untrained
            if let Some(name) = got.get("save").and_then(Value::as_str).map(str::to_string)
                && !(cls.is_empty() && specs.is_empty())
            {
                let (_, models) = saves.models_of(&name)?;
                if !cls.is_empty() {
                    got["classifiers"] = json!(models.import_classifiers(&cls)?);
                }
                if !specs.is_empty() {
                    let (_, g) = saves.graph(&name)?;
                    got["models"] = json!(crate::specs::import(&g, &models, &specs)?);
                }
            }
            Ok(got)
        }
        _ => saves.call_plain(op, args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "anend-clfops-{tag}-{}-{}",
            std::process::id(),
            crate::graph::store::system_ms()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn job_folders() {
        assert_eq!(
            key_folder("/data/users/ann/saves/notes.redb\u{0}kind"),
            "/data/users/ann/saves"
        );
        assert_eq!(
            key_folder("/data/users/ann/saves/a/b.redb\u{0}x/y"),
            "/data/users/ann/saves/a"
        );
        assert_eq!(key_folder("plain"), "");
    }

    /// a GPU asked for where there's none trains on the CPU and says so; a shared hub
    /// refuses the GPU unless its admin allowed it
    #[test]
    fn devices_fall_back_and_hubs_decide() {
        assert_eq!(
            device_arg(&json!({ "device": "gpu_fast" })),
            crate::classify::device::Device::GpuFast
        );
        assert_eq!(device_arg(&json!({})), crate::classify::device::Device::Cpu);
        let dir = temp("devices");
        let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
        let c = |op: &str, a: Value| ws.call(op, &a);
        let key = |t: &str| {
            c("mem.saturate", json!({ "text": t })).unwrap()["key"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let (a, b) = (key("fruit"), key("rocks"));
        for t in ["apple pie", "plum jam", "ripe pear"] {
            c("lists.enlist", json!({ "list": a, "member": key(t) })).unwrap();
        }
        for t in ["granite", "slate roof", "flint"] {
            c("lists.enlist", json!({ "list": b, "member": key(t) })).unwrap();
        }
        c(
            "classify.create",
            json!({ "name": "k", "classes": [a, b], "preset": "tiny" }),
        )
        .unwrap();
        let j = c(
            "classify.train",
            json!({ "name": "k", "epochs": 1, "device": "gpu", "wait": true }),
        )
        .unwrap();
        if cfg!(feature = "gpu") && crate::classify::device::Device::Gpu.check().is_ok() {
            assert_eq!(j["device"], "gpu", "{j}");
        } else {
            assert_eq!(j["device"], "cpu", "{j}");
            assert!(j["device_note"].as_str().unwrap().contains("no GPU"), "{j}");
        }
        assert_eq!(j["state"], "done", "{j}");
        let shared = Saves::new(&dir, "notes").with_models(Limits::shared());
        let e = shared
            .call("classify.train", &json!({ "name": "k", "device": "gpu" }))
            .unwrap_err();
        assert!(e.contains("--gpu"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn starters_make_lists_train_and_score_on_held_out_texts() {
        let dir = temp("starters");
        let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
        let c = |op: &str, a: Value| ws.call(op, &a);
        let all = c("classify.starters", json!({})).unwrap();
        let ids: Vec<&str> = all
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert!(
            ids.contains(&"things") && ids.contains(&"authors") && ids.contains(&"languages"),
            "{all}"
        );
        assert!(
            all.as_array().unwrap().iter().all(|s| s["licence"].is_string()),
            "every one says its terms"
        );
        let langs = all.as_array().unwrap().iter().find(|s| s["id"] == "languages").unwrap();
        assert_eq!(langs["fetched"], false);
        assert!(
            c("classify.starter", json!({ "id": "languages" }))
                .unwrap_err()
                .contains("fetch")
        );
        assert!(c("classify.starter_fetch", json!({ "id": "things" })).is_err());

        let made = c(
            "classify.starter",
            json!({ "id": "things", "preset": "tiny", "per_class": 80, "epochs": 8, "threads": 2, "wait": true }),
        )
        .unwrap();
        assert_eq!(made["texts"], 240, "{made}");
        assert_eq!(made["classifier"]["train"]["patience"], 10, "{made}");
        assert_eq!(made["job"]["state"], "done", "{made}");
        assert_eq!(made["classifier"]["starter"], "things");
        assert_eq!(
            made["classifier"]["classes"],
            json!([
                "fruit, stones and animals: fruit",
                "fruit, stones and animals: stones",
                "fruit, stones and animals: animals"
            ])
        );
        // the lists hold the training texts
        let lists = c("lists.stats", json!({})).unwrap().to_string();
        assert!(lists.contains("fruit, stones and animals: stones"), "{lists}");
        assert!(
            c("classify.starter", json!({ "id": "things" }))
                .unwrap_err()
                .contains("already")
        );

        let sc = c("classify.starter_score", json!({ "name": "fruit, stones and animals" })).unwrap();
        assert_eq!(sc["texts"], 300, "{sc}");
        let acc = sc["accuracy"].as_f64().unwrap();
        assert!(acc > 0.5, "well above a third by chance: {sc}");
        let per: u64 = sc["classes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["right"].as_u64().unwrap())
            .sum();
        assert_eq!(per, sc["right"].as_u64().unwrap());
        assert!(c("classify.starter_score", json!({ "name": "nope" })).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn classifier_calls() {
        let dir = temp("calls");
        let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
        let c = |op: &str, a: Value| ws.call(op, &a);
        let key = |t: &str| {
            c("mem.saturate", json!({ "text": t })).unwrap()["key"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let (fruit, rocks) = (key("fruit"), key("rocks"));
        let f = ["apple", "banana", "cherry", "mango", "peach", "plum", "grape", "melon"];
        let r = [
            "rock", "stone", "granite", "basalt", "flint", "slate", "quartz", "shale",
        ];
        for i in 0..20 {
            let a = key(&format!("{} {} {}", f[i % 8], f[(i * 3 + 1) % 8], f[(i / 8 + 4) % 8]));
            let b = key(&format!("{} {} {}", r[i % 8], r[(i * 5 + 2) % 8], r[(i / 8 + 4) % 8]));
            c("lists.enlist", json!({ "list": fruit, "member": a })).unwrap();
            c("lists.enlist", json!({ "list": rocks, "member": b })).unwrap();
        }
        let made = c(
            "classify.create",
            json!({ "name": "kind", "classes": [fruit, rocks], "preset": "tiny" }),
        )
        .unwrap();
        assert_eq!(made["classes"], json!(["fruit", "rocks"]));
        assert_eq!(made["trained"], false);
        assert!(c("classify.create", json!({ "name": "kind", "classes": [fruit, rocks] })).is_err());
        assert!(
            c(
                "classify.create",
                json!({ "name": "odd", "classes": [fruit], "preset": "huge" })
            )
            .is_err()
        );

        // in the background, then waited for
        let j = c("classify.train", json!({ "name": "kind", "epochs": 6, "threads": 2 })).unwrap();
        assert_eq!(j["state"], "running");
        assert_eq!(j["device"], "cpu", "the CPU unless asked: {j}");
        let mut done = Value::Null;
        for _ in 0..600 {
            done = c("classify.job", json!({ "name": "kind" })).unwrap();
            if done["state"] != "running" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(done["state"], "done", "{done}");
        assert!(done["summary"]["metrics"]["accuracy"].as_f64().unwrap() > 0.5, "{done}");
        let list = c("classify.list", json!({})).unwrap();
        assert_eq!(list[0]["trained"], true);
        assert!(c("classify.cancel", json!({ "name": "kind" })).is_err());

        let p = c(
            "classify.predict",
            json!({ "name": "kind", "texts": ["granite slate flint", "plum peach apple"] }),
        )
        .unwrap();
        assert_eq!(p["results"][0]["class"], "rocks", "{p}");
        assert_eq!(p["results"][1]["class"], "fruit", "{p}");
        assert!(p["device"]["device"].is_string(), "where it read: {p}");

        // where texts are read: the choice and what it gives here; the CPU is always on
        // offer, and a shared hub doesn't let it be changed
        let d = c("classify.device", json!({ "device": "cpu" })).unwrap();
        assert_eq!(d["device"], "cpu", "{d}");
        assert_eq!(d["reading"]["device"], "cpu", "{d}");
        assert!(c("classify.device", json!({ "device": "abacus" })).is_err());
        let shared = Saves::new(&dir, "notes").with_models(Limits::shared());
        assert!(shared.call("classify.device", &json!({ "device": "gpu" })).is_err());
        assert!(shared.call("gpu.report", &json!({})).is_err());

        // the same answers read on the GPU (every text's windows packed), where there is one
        {
            use crate::classifiers::Read;
            use crate::classify::device::{Choice, Device};
            let (_, st) = ws.models().unwrap();
            let mut on_cpu = st.stack("kind").unwrap();
            on_cpu.read = Read::cpu();
            let mut on_gpu = st.stack("kind").unwrap();
            on_gpu.read = Read::new(Choice::Gpu, true, "test".into());
            let texts: Vec<String> = (0..30)
                .map(|i| format!("{} {} {}", f[i % 8], r[(i * 3) % 8], "and more words ".repeat(i % 7)))
                .collect();
            let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
            let a = on_cpu.outputs(&texts, pool());
            let b = on_gpu.outputs(&texts, pool());
            let reading = on_gpu.reading();
            if Device::Gpu.check().is_ok() {
                assert_eq!(reading.device, Device::Gpu, "{reading:?}");
            } else {
                assert_eq!(reading.device, Device::Cpu);
                assert!(reading.note.is_some(), "it says why: {reading:?}");
            }
            for (x, y) in a.iter().zip(&b) {
                if reading.exact {
                    assert_eq!(x.probs, y.probs);
                } else {
                    for (p, q) in x.probs.iter().zip(&y.probs) {
                        assert!((p - q).abs() < 1e-3, "{p} vs {q}");
                    }
                }
            }
        }

        let found = c(
            "classify.search",
            json!({ "name": "kind", "class": "rocks", "limit": 5 }),
        )
        .unwrap();
        let res = found["results"].as_array().unwrap();
        assert_eq!(res.len(), 5);
        assert!(res.iter().all(|x| x["top_class"] == "rocks"), "{found}");
        let probs: Vec<f64> = res.iter().map(|x| x["prob"].as_f64().unwrap()).collect();
        assert!(probs.windows(2).all(|w| w[0] >= w[1]));
        let q = c("classify.search", json!({ "name": "kind", "class": 0, "q": "plum" })).unwrap();
        assert!(
            q["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|x| x["text"].as_str().unwrap().contains("plum"))
        );

        // yes raises the class for that text, no lowers it
        let t = "melon basalt grape";
        let prob = || {
            c("classify.predict", json!({ "name": "kind", "texts": [t] })).unwrap()["results"][0]["probs"][1]
                .as_f64()
                .unwrap()
        };
        let p0 = prob();
        let fb = c(
            "classify.feedback",
            json!({ "name": "kind", "text": t, "class": "rocks", "yes": true }),
        )
        .unwrap();
        let p1 = prob();
        assert!(p1 > p0, "{p0} {p1} {fb}");
        c(
            "classify.feedback",
            json!({ "name": "kind", "text": t, "class": "rocks", "yes": false, "weight": 2.0 }),
        )
        .unwrap();
        let p2 = prob();
        assert!(p2 < p1);
        let got = c("classify.get", json!({ "name": "kind" })).unwrap();
        assert_eq!(got["feedback_count"], 2);
        assert_eq!(got["version"], 3);
        assert_eq!(got["undo"], 2);
        // taken back, one at a time, to exactly what it said before
        let u = c("classify.unfeedback", json!({ "name": "kind" })).unwrap();
        assert_eq!(
            (u["entry"]["yes"].as_bool(), u["left"].as_u64()),
            (Some(false), Some(1))
        );
        assert_eq!(prob(), p1);
        c("classify.unfeedback", json!({ "name": "kind" })).unwrap();
        assert_eq!(prob(), p0);
        assert!(c("classify.unfeedback", json!({ "name": "kind" })).is_err());
        let got = c("classify.get", json!({ "name": "kind" })).unwrap();
        assert_eq!(
            (got["feedback_count"].as_u64(), got["version"].as_u64()),
            (Some(0), Some(1))
        );
        c(
            "classify.feedback",
            json!({ "name": "kind", "text": t, "class": "rocks", "yes": true }),
        )
        .unwrap();
        c(
            "classify.feedback",
            json!({ "name": "kind", "text": t, "class": "rocks", "yes": false, "weight": 2.0 }),
        )
        .unwrap();
        assert_eq!(prob(), p2);

        // steering: a bias towards fruit moves things, and the report says how much
        let s0 = c("classify.steer", json!({ "name": "kind", "steer": {}, "save": false })).unwrap();
        assert_eq!(s0["bias"]["flipped"], 0.0);
        let s1 = c(
            "classify.steer",
            json!({ "name": "kind", "steer": { "bias": { "fruit": 4.0 }, "temperature": 2.0 } }),
        )
        .unwrap();
        assert!(s1["bias"]["mean_kl"].as_f64().unwrap() > 0.0, "{s1}");
        assert!(s1["bias"]["flipped"].as_f64().unwrap() > 0.0, "{s1}");
        assert_eq!(
            c("classify.get", json!({ "name": "kind" })).unwrap()["steer"]["heads"][0]["temperature"],
            2.0
        );
        c("classify.steer", json!({ "name": "kind", "steer": { "reset": true } })).unwrap();

        // outputs become a list another classifier can train on
        let e = c(
            "classify.enlist",
            json!({ "name": "kind", "class": "rocks", "threshold": 0.5 }),
        )
        .unwrap();
        assert_eq!(e["name"], "kind: rocks");
        assert!(e["members"].as_u64().unwrap() >= 15, "{e}");
        let stats = c("lists.stats", json!({})).unwrap();
        assert!(
            stats["lists"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["name"] == "kind: rocks")
        );
        c(
            "classify.create",
            json!({ "name": "on top", "classes": [e["list"], fruit], "preset": "tiny", "inputs": ["kind"] }),
        )
        .unwrap();
        c(
            "classify.train",
            json!({ "name": "on top", "epochs": 2, "threads": 2, "wait": true }),
        )
        .unwrap();
        let k = c(
            "classify.contributions",
            json!({ "name": "on top", "text": "slate plum" }),
        )
        .unwrap();
        assert_eq!(k["results"].as_array().unwrap().len(), 2);
        let s = c(
            "classify.feedback_stack",
            json!({ "name": "on top", "text": "slate plum", "class": 1, "yes": true }),
        )
        .unwrap();
        assert_eq!(s["touched"], json!(["kind", "on top"]));
        c(
            "classify.couple",
            json!({ "name": "on top", "from": "kind", "coupling": 0.0 }),
        )
        .unwrap();

        let sim = c(
            "classify.similar",
            json!({ "name": "kind", "text": "granite", "limit": 3 }),
        )
        .unwrap();
        assert_eq!(sim["results"].as_array().unwrap().len(), 3);
        let v = c("classify.represent", json!({ "name": "kind", "text": "granite" })).unwrap();
        assert_eq!(v["vector"].as_array().unwrap().len(), 32);

        // phase 3: buckets, near, tints, per-note indexing, validators, filters, modules
        let ix = c("classify.index", json!({ "name": "kind", "wait": true })).unwrap();
        assert_eq!(ix["state"], "done", "{ix}");
        // in the background too: it finishes, and another can start after
        c("classify.index", json!({ "name": "kind" })).unwrap();
        let mut state = json!(null);
        for _ in 0..600 {
            state = c("classify.index_job", json!({ "name": "kind" })).unwrap();
            if state["state"] != "running" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(state["state"], "done", "{state}");
        let bk = c("classify.buckets", json!({ "name": "kind" })).unwrap();
        let filed: u64 = bk["buckets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["count"].as_u64().unwrap())
            .sum();
        assert_eq!(filed, bk["indexed"].as_u64().unwrap());
        let rocks_bucket = c(
            "classify.bucket",
            json!({ "name": "kind", "text": "granite slate flint" }),
        )
        .unwrap();
        assert_eq!(rocks_bucket["class"], "rocks", "{rocks_bucket}");
        assert!(!rocks_bucket["hits"].as_array().unwrap().is_empty());
        let near = c(
            "classify.near",
            json!({ "name": "kind", "text": "granite", "by": "outputs", "limit": 4 }),
        )
        .unwrap();
        assert_eq!(near["results"].as_array().unwrap().len(), 4);
        let k0 = rocks_bucket["hits"][0]["key"].as_str().unwrap().to_string();
        let t = c("classify.tints", json!({ "name": "kind", "keys": [k0] })).unwrap();
        assert_eq!(t["palette"].as_array().unwrap().len(), 2);
        c("classify.set_index", json!({ "key": k0, "names": ["kind"] })).unwrap();
        let ind = c("classify.indexed", json!({ "key": k0 })).unwrap();
        assert_eq!(ind["verdicts"][0]["name"], "kind", "{ind}");
        c(
            "classify.validators",
            json!({ "name": "on top", "set": [{ "by": "kind", "class": "rocks", "min": 0.5 }] }),
        )
        .unwrap();
        c("classify.disagreements", json!({ "name": "on top" })).unwrap();
        let expr = json!({ "op": "and", "of": [{ "op": "class", "classifier": "kind", "class": "rocks" }, { "op": "not", "of": { "op": "words", "q": "granite" } }] });
        c("filters.save", json!({ "name": "stony", "expr": expr })).unwrap();
        let run = c("filters.run", json!({ "name": "stony" })).unwrap();
        assert!(run["matched"].as_u64().unwrap() > 0, "{run}");
        assert_eq!(c("filters.list", json!({})).unwrap()[0]["count"], run["matched"]);
        let en = c("filters.enlist", json!({ "name": "stony" })).unwrap();
        assert_eq!(en["members"], run["matched"]);
        assert!(
            c(
                "filters.save",
                json!({ "name": "bad", "expr": { "op": "class", "classifier": "nope", "class": "x" } })
            )
            .is_err()
        );
        c("filters.delete", json!({ "name": "stony" })).unwrap();
        let ex = c("classify.export", json!({ "name": "on top" })).unwrap();
        let ins = c("classify.inspect", json!({ "data": ex["data"] })).unwrap();
        assert_eq!(ins["upstream"][0][0], "kind");
        let im = c("classify.import", json!({ "data": ex["data"], "as": "copy" })).unwrap();
        assert_eq!(im["name"], "copy", "{im}");
        assert_eq!(im["reused"], json!(["kind"]));
        assert!(c("classify.import", json!({ "data": "QUJD" })).is_err());
        c("classify.delete", json!({ "name": "copy" })).unwrap();

        assert!(
            c("classify.delete", json!({ "name": "kind" })).is_err(),
            "on top builds on it"
        );
        c("classify.delete", json!({ "name": "on top" })).unwrap();
        c("classify.delete", json!({ "name": "kind" })).unwrap();
        assert!(c("classify.list", json!({})).unwrap().as_array().unwrap().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
