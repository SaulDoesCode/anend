//! The app's classifier operations (`classify.*`, listed in docs/app-api.md), over the
//! open save's classifiers (`anend_core::classifiers`). Training runs on a thread of its
//! own: `classify.train` snapshots the examples, answers at once, and the thread writes
//! the weights when it's done; `classify.job` reports how it goes and `classify.cancel`
//! stops it (a cancelled run writes nothing).

use anend_core::classifiers::{self as clfs, Input, Judged, Stack, Summary, Tick, TrainArgs};
use anend_core::classify::steer::Output;
use anend_core::classify::{Flow, Gains, HeadKind, Pool};
use anend_core::memory::Ref;
use anend_core::store::Store;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::app::Res;

/// how many memories and notes a search, a list or a neighbour search looks through
const SCAN: usize = 20_000;

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool::new(anend_core::classify::pool::available()))
}

// ---------------------------------------------------------------- jobs

#[derive(Clone, Debug, Default, Serialize)]
pub struct Eval {
    pub step: usize,
    pub round: usize,
    pub loss: f64,
    pub accuracy: f64,
    pub macro_f1: f64,
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
}

pub struct Job {
    cancel: AtomicBool,
    state: Mutex<JobState>,
    started: std::time::Instant,
}

impl Job {
    fn snapshot(&self) -> JobState {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if s.state == "running" {
            s.seconds = self.started.elapsed().as_secs_f64();
        }
        s
    }

    fn running(&self) -> bool {
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
fn jobs() -> &'static Mutex<HashMap<String, Arc<Job>>> {
    static JOBS: OnceLock<Mutex<HashMap<String, Arc<Job>>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

fn job_key(store: &Store, name: &str) -> String {
    format!("{}\u{0}{name}", store.path().display())
}

fn job(store: &Store, name: &str) -> Option<Arc<Job>> {
    jobs().lock().unwrap_or_else(|e| e.into_inner()).get(&job_key(store, name)).cloned()
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

fn start(store: Arc<Store>, name: &str, a: &TrainArgs, wait: bool) -> Res {
    let mut t = store.prepare(name, a)?;
    let job = Arc::new(Job {
        cancel: AtomicBool::new(false),
        state: Mutex::new(JobState {
            name: name.to_string(),
            state: "running".into(),
            rounds: t.rounds,
            examples: t.data.len(),
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
    let mut work = move || {
        let r = t.run(|tick| {
            j.tick(tick);
            if j.cancel.load(Ordering::Relaxed) { Flow::Stop } else { Flow::Go }
        });
        let r = r.and_then(|s| {
            if s.cancelled {
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
    } else {
        std::thread::Builder::new()
            .name(format!("train {name}"))
            .spawn(work)
            .map_err(|e| e.to_string())?;
    }
    Ok(json!(job.snapshot()))
}

// ---------------------------------------------------------------- json

fn s<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(Value::as_str).unwrap_or("")
}

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
        .and_then(|b| anend_core::classify::Classifier::from_bytes(&b).ok());
    let o = v.as_object_mut().expect("an object");
    o.insert("steer".into(), json!(d.steer));
    o.insert("history".into(), json!(d.history));
    o.insert("train".into(), json!(d.train));
    o.insert("created".into(), json!(d.created));
    o.insert("feedback_count".into(), json!(fb.len()));
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
    o.insert("job".into(), job(store, &d.name).map_or(Value::Null, |j| json!(j.snapshot())));
    Ok(v)
}

/// steering from the args over the current one: `bias` as a list or {class: value},
/// `temperature`, `threshold`, `gains` {letter, word, side, extra}
fn steer_from(def: &clfs::Definition, a: &Value) -> Result<anend_core::classify::Steer, String> {
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
        st.heads[0] = anend_core::classify::HeadSteer::neutral(names.len());
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
            o => serde_json::from_value(o.clone()).map_err(|e| format!("an input is a name or {{from, coupling}}: {e}")),
        })
        .collect()
}

// ---------------------------------------------------------------- the calls

pub fn call(store: Arc<Store>, op: &str, args: &Value) -> Res {
    let st: &Store = &store;
    match op {
        "classify.list" => Ok(Value::Array(
            st.classifiers()?.iter().map(|d| summary(st, d)).collect(),
        )),
        "classify.create" => {
            let lists = args.get("classes").and_then(Value::as_array).cloned().unwrap_or_default();
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
            let def = st.create_classifier(name_arg(args)?, &classes, not.as_ref(), head, preset, inputs_arg(args)?)?;
            details(st, &def)
        }
        "classify.get" => details(st, &st.classifier(name_arg(args)?)?),
        "classify.delete" => {
            let name = name_arg(args)?;
            not_training(st, &[name.to_string()])?;
            st.delete_classifier(name)?;
            jobs().lock().unwrap_or_else(|e| e.into_inner()).remove(&job_key(st, name));
            Ok(Value::Null)
        }
        "classify.couple" => {
            let name = name_arg(args)?;
            let mut def = st.classifier(name)?;
            let from = s(args, "from");
            let c = args.get("coupling").and_then(Value::as_f64).ok_or("coupling is a number")? as f32;
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
            start(store.clone(), name, &train_args(args), wait)
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
            Ok(json!({ "classes": classes, "trained": stack.def().trained(), "results": results }))
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
                        "key": r.label(), "text": shown, "note": matches!(r, Ref::Note(_)),
                        "prob": o.probs[c], "top": o.top, "top_class": classes.get(o.top),
                        "member": o.members.contains(&c),
                    })
                })
                .collect();
            Ok(json!({
                "classes": classes, "class": classes[c], "stack": stack.names(),
                "trained": def.trained(), "scanned": cands.len(), "results": results,
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
            let cos: Vec<f32> = vs.iter().map(|v| anend_core::classify::cosine(&me, v)).collect();
            let mut order: Vec<usize> = (0..cands.len()).collect();
            order.sort_by(|&a, &b| cos[b].total_cmp(&cos[a]).then(a.cmp(&b)));
            let results: Vec<Value> = order
                .into_iter()
                .take(limit)
                .map(|i| {
                    let (r, _, shown) = &cands[i];
                    json!({ "key": r.label(), "text": shown, "note": matches!(r, Ref::Note(_)), "cosine": cos[i] })
                })
                .collect();
            Ok(json!({ "text": j.text, "results": results }))
        }

        // ---- the player's knobs and verdicts
        "classify.steer" => {
            let name = name_arg(args)?;
            let def = st.classifier(name)?;
            let steer = steer_from(&def, args.get("steer").unwrap_or(&Value::Null))?;
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
        "classify.enlist" => {
            let name = name_arg(args)?;
            let def = st.classifier(name)?;
            let c = class_arg(args, &def)?;
            let threshold = args.get("threshold").and_then(Value::as_f64).map(|x| x as f32);
            let (head, n) = st.enlist_outputs(name, &def.class_names()[c], threshold, SCAN)?;
            let text = st.remember(&head).map(|h| h.text).unwrap_or_default();
            Ok(json!({ "list": head.label(), "name": text, "members": n }))
        }
        _ => Err(format!("there's no {op:?} here")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Workspace, call as app_call};

    #[test]
    fn classifier_calls() {
        let dir = std::env::temp_dir().join(format!("anend-clfops-{}", rand::random::<u64>()));
        let ws = Workspace::new(dir.clone());
        let c = |op: &str, a: Value| app_call(&ws, op, &a);
        let key = |t: &str| c("mem.saturate", json!({ "text": t })).unwrap()["key"].as_str().unwrap().to_string();
        let (fruit, rocks) = (key("fruit"), key("rocks"));
        let f = ["apple", "banana", "cherry", "mango", "peach", "plum", "grape", "melon"];
        let r = ["rock", "stone", "granite", "basalt", "flint", "slate", "quartz", "shale"];
        for i in 0..20 {
            let a = key(&format!("{} {} {}", f[i % 8], f[(i * 3 + 1) % 8], f[(i / 8 + 4) % 8]));
            let b = key(&format!("{} {} {}", r[i % 8], r[(i * 5 + 2) % 8], r[(i / 8 + 4) % 8]));
            c("lists.enlist", json!({ "list": fruit, "member": a })).unwrap();
            c("lists.enlist", json!({ "list": rocks, "member": b })).unwrap();
        }
        let made = c("classify.create", json!({ "name": "kind", "classes": [fruit, rocks], "preset": "tiny" })).unwrap();
        assert_eq!(made["classes"], json!(["fruit", "rocks"]));
        assert_eq!(made["trained"], false);
        assert!(c("classify.create", json!({ "name": "kind", "classes": [fruit, rocks] })).is_err());
        assert!(c("classify.create", json!({ "name": "odd", "classes": [fruit], "preset": "huge" })).is_err());

        // in the background, then waited for
        let j = c("classify.train", json!({ "name": "kind", "epochs": 6, "threads": 2 })).unwrap();
        assert_eq!(j["state"], "running");
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

        let p = c("classify.predict", json!({ "name": "kind", "texts": ["granite slate flint", "plum peach apple"] })).unwrap();
        assert_eq!(p["results"][0]["class"], "rocks", "{p}");
        assert_eq!(p["results"][1]["class"], "fruit", "{p}");

        let found = c("classify.search", json!({ "name": "kind", "class": "rocks", "limit": 5 })).unwrap();
        let res = found["results"].as_array().unwrap();
        assert_eq!(res.len(), 5);
        assert!(res.iter().all(|x| x["top_class"] == "rocks"), "{found}");
        let probs: Vec<f64> = res.iter().map(|x| x["prob"].as_f64().unwrap()).collect();
        assert!(probs.windows(2).all(|w| w[0] >= w[1]));
        let q = c("classify.search", json!({ "name": "kind", "class": 0, "q": "plum" })).unwrap();
        assert!(q["results"].as_array().unwrap().iter().all(|x| x["text"].as_str().unwrap().contains("plum")));

        // yes raises the class for that text, no lowers it
        let t = "melon basalt grape";
        let prob = || c("classify.predict", json!({ "name": "kind", "texts": [t] })).unwrap()["results"][0]["probs"][1].as_f64().unwrap();
        let p0 = prob();
        let fb = c("classify.feedback", json!({ "name": "kind", "text": t, "class": "rocks", "yes": true })).unwrap();
        let p1 = prob();
        assert!(p1 > p0, "{p0} {p1} {fb}");
        c("classify.feedback", json!({ "name": "kind", "text": t, "class": "rocks", "yes": false, "weight": 2.0 })).unwrap();
        assert!(prob() < p1);
        let got = c("classify.get", json!({ "name": "kind" })).unwrap();
        assert_eq!(got["feedback_count"], 2);
        assert_eq!(got["version"], 3);

        // steering: a bias towards fruit moves things, and the report says how much
        let s0 = c("classify.steer", json!({ "name": "kind", "steer": {}, "save": false })).unwrap();
        assert_eq!(s0["bias"]["flipped"], 0.0);
        let s1 = c("classify.steer", json!({ "name": "kind", "steer": { "bias": { "fruit": 4.0 }, "temperature": 2.0 } })).unwrap();
        assert!(s1["bias"]["mean_kl"].as_f64().unwrap() > 0.0, "{s1}");
        assert!(s1["bias"]["flipped"].as_f64().unwrap() > 0.0, "{s1}");
        assert_eq!(c("classify.get", json!({ "name": "kind" })).unwrap()["steer"]["heads"][0]["temperature"], 2.0);
        c("classify.steer", json!({ "name": "kind", "steer": { "reset": true } })).unwrap();

        // outputs become a list another classifier can train on
        let e = c("classify.enlist", json!({ "name": "kind", "class": "rocks", "threshold": 0.5 })).unwrap();
        assert_eq!(e["name"], "kind: rocks");
        assert!(e["members"].as_u64().unwrap() >= 15, "{e}");
        let stats = c("lists.stats", json!({})).unwrap();
        assert!(stats["lists"].as_array().unwrap().iter().any(|l| l["name"] == "kind: rocks"));
        c("classify.create", json!({ "name": "on top", "classes": [e["list"], fruit], "preset": "tiny", "inputs": ["kind"] })).unwrap();
        c("classify.train", json!({ "name": "on top", "epochs": 2, "threads": 2, "wait": true })).unwrap();
        let k = c("classify.contributions", json!({ "name": "on top", "text": "slate plum" })).unwrap();
        assert_eq!(k["results"].as_array().unwrap().len(), 2);
        let s = c("classify.feedback_stack", json!({ "name": "on top", "text": "slate plum", "class": 1, "yes": true })).unwrap();
        assert_eq!(s["touched"], json!(["kind", "on top"]));
        c("classify.couple", json!({ "name": "on top", "from": "kind", "coupling": 0.0 })).unwrap();

        let sim = c("classify.similar", json!({ "name": "kind", "text": "granite", "limit": 3 })).unwrap();
        assert_eq!(sim["results"].as_array().unwrap().len(), 3);
        let v = c("classify.represent", json!({ "name": "kind", "text": "granite" })).unwrap();
        assert_eq!(v["vector"].as_array().unwrap().len(), 32);

        assert!(c("classify.delete", json!({ "name": "kind" })).is_err(), "on top builds on it");
        c("classify.delete", json!({ "name": "on top" })).unwrap();
        c("classify.delete", json!({ "name": "kind" })).unwrap();
        assert!(c("classify.list", json!({})).unwrap().as_array().unwrap().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
