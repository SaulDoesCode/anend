//! The classifiers view (Ctrl+7): classifiers as conceptual selectors the player grows.
//! Each is made from lists (a class per list, an optional "not" list, classifiers it
//! builds on), trained on a thread of its own with a live progress bar, the loss as a
//! line, its metrics and confusion matrix and the growth it went through; steered by
//! the player's sliders (bias per class, temperature, threshold, channel gains) with the
//! bias it introduces measured live; searched by class, where every result takes a yes
//! or no, and "why" lists the classifiers that had a part in it, each with its own yes
//! or no, and end to end through the couplings. Its outputs go back to lists; the save is
//! indexed into its buckets (memoized, so colouring and near search read no network
//! twice); texts near a text by what it says of them or by its embedding; modules go out
//! and come in as `.anendcls` files.
//!
//! Everything heavy runs in `jobs::Work`; the view only draws what came back.

use bevy::feathers::controls::{ButtonVariant, FeathersTextInput, FeathersTextInputContainer};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{FocusedInput, InputFocus};
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEditChange};
use bevy::ui_widgets::{Activate, ScrollArea, SliderValue, ValueChange};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anend_core::classifiers::{Definition, Input, Judged, Summary, Tick, TrainArgs};
use anend_core::classify::{BiasReport, Flow, HeadKind, Steer};
use anend_core::memo;
use anend_core::memory::Ref;
use anend_core::store::Store;

use crate::cls_widgets::*;
use crate::icons::{Icon, Tip};
use crate::jobs::{Then, Work, pool};
use crate::theme::colors;
use crate::ui::{Db, Fonts, Route, Session, Status};

/// how many memories and notes a search, an index or a list reads at most
pub const SCAN: usize = 20_000;

// ------------------------------------------------------------------ state

#[derive(Component, Default, Clone)]
pub struct ClsView;
#[derive(Component, Default, Clone)]
pub struct ClsSide;
#[derive(Component, Default, Clone)]
pub struct ClsMain;
#[derive(Component, Default, Clone)]
pub struct ClsNameInput;

/// a training run as it goes
#[derive(Default)]
pub struct TrainLive {
    pub name: String,
    pub losses: Vec<f64>,
    pub last: Option<Tick>,
    pub eval: Option<(f64, f64, f64)>,
    pub started: Option<std::time::Instant>,
}

pub struct Training {
    pub live: Arc<Mutex<TrainLive>>,
    pub cancel: Arc<AtomicBool>,
}

pub struct Indexing {
    pub name: String,
    pub done: Arc<Mutex<(usize, usize)>>,
    pub cancel: Arc<AtomicBool>,
}

/// a text a classifier ranked
#[derive(Clone)]
pub struct Found {
    pub r: Ref,
    /// what the classifier reads
    pub reads: String,
    /// what's shown
    pub shown: String,
    pub prob: f32,
    pub top: usize,
    pub member: bool,
    pub valid: bool,
}

pub struct Searched {
    pub name: String,
    pub class: usize,
    pub results: Vec<Found>,
    pub scanned: usize,
    pub stack: Vec<String>,
}

/// one classifier's part in a result
#[derive(Clone)]
pub struct WhyRow {
    pub name: String,
    pub share: f64,
    pub classes: Vec<String>,
    pub probs: Vec<f32>,
    pub top: usize,
    pub judged: bool,
}

pub struct Buckets {
    pub name: String,
    pub indexed: usize,
    /// (class, or none for the uncertain; texts per confidence band)
    pub per: Vec<(Option<u32>, [usize; 10])>,
}

pub struct Near {
    pub source: String,
    pub results: Vec<(Ref, String, f32)>,
    pub indexed: usize,
}

/// the new classifier form
#[derive(Default, Clone)]
pub struct Form {
    pub name: String,
    pub classes: Vec<String>,
    pub not: Option<String>,
    pub inputs: Vec<String>,
    pub preset: String,
    pub multi: bool,
}

/// entities the parts of the view are drawn into
#[derive(Default)]
pub struct Boxes {
    pub live: Option<Entity>,
    pub bias: Option<Entity>,
    pub results: Option<Entity>,
    pub buckets: Option<Entity>,
    pub near: Option<Entity>,
}

#[derive(Resource)]
pub struct ClsState {
    pub redraw: bool,
    pub reload: bool,
    pub selected: Option<String>,
    pub defs: Vec<Definition>,
    /// lists to make classes of: (key label, name, members)
    pub lists: Vec<(String, String, usize)>,
    pub creating: bool,
    pub form: Form,
    pub importing: bool,
    pub imports: Vec<PathBuf>,
    pub confirm_delete: bool,
    // training
    pub epochs: f32,
    pub rounds: f32,
    pub fresh: bool,
    pub training: Option<Training>,
    pub last_run: Option<Result<Summary, String>>,
    pub live_dirty: bool,
    live_at: f64,
    // steering
    pub steer: Option<Steer>,
    pub bias: Option<BiasReport>,
    bias_busy: bool,
    bias_due: Option<f64>,
    save_due: Option<f64>,
    pub bias_dirty: bool,
    // search
    pub class: usize,
    pub query: String,
    pub searched: Option<Searched>,
    pub searching: bool,
    pub why: HashMap<String, Vec<WhyRow>>,
    pub results_dirty: bool,
    pub judging: bool,
    // buckets
    pub indexing: Option<Indexing>,
    pub buckets: Option<Buckets>,
    pub bucket_open: Option<(Option<u32>, Vec<(Ref, String, f32)>)>,
    pub buckets_dirty: bool,
    pub buckets_reload: bool,
    // near
    pub near: Option<Near>,
    pub near_embedding: bool,
    pub near_dirty: bool,
    // outputs
    pub out_class: usize,
    pub out_threshold: f32,
    pub boxes: Boxes,
    side: String,
}

impl Default for ClsState {
    fn default() -> Self {
        ClsState {
            redraw: true,
            reload: true,
            selected: None,
            defs: vec![],
            lists: vec![],
            creating: false,
            form: Form {
                preset: anend_core::classifiers::DEFAULT_PRESET.into(),
                ..Form::default()
            },
            importing: false,
            imports: vec![],
            confirm_delete: false,
            epochs: 14.0,
            rounds: 1.0,
            fresh: false,
            training: None,
            last_run: None,
            live_dirty: false,
            live_at: 0.0,
            steer: None,
            bias: None,
            bias_busy: false,
            bias_due: None,
            save_due: None,
            bias_dirty: false,
            class: 0,
            query: String::new(),
            searched: None,
            searching: false,
            why: HashMap::new(),
            results_dirty: false,
            judging: false,
            indexing: None,
            buckets: None,
            bucket_open: None,
            buckets_dirty: false,
            buckets_reload: false,
            near: None,
            near_embedding: true,
            near_dirty: false,
            out_class: 0,
            out_threshold: 0.5,
            boxes: Boxes::default(),
            side: String::new(),
        }
    }
}

impl ClsState {
    pub fn def(&self) -> Option<&Definition> {
        let s = self.selected.as_ref()?;
        self.defs.iter().find(|d| &d.name == s)
    }

    pub fn training_name(&self) -> Option<String> {
        self.training
            .as_ref()
            .map(|t| t.live.lock().unwrap_or_else(|e| e.into_inner()).name.clone())
    }

    /// opens a classifier, forgetting what was shown of the last one
    pub fn select(&mut self, name: Option<String>) {
        if self.selected != name {
            self.searched = None;
            self.why.clear();
            self.near = None;
            self.buckets = None;
            self.bucket_open = None;
            self.class = 0;
            self.out_class = 0;
            self.last_run = None;
        }
        self.selected = name;
        self.steer = None;
        self.bias = None;
        self.bias_due = Some(0.0);
        self.creating = false;
        self.importing = false;
        self.confirm_delete = false;
        self.redraw = true;
        self.buckets_reload = true;
    }
}

fn say(world: &mut World, msg: impl Into<String>) {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let mut status = world.resource_mut::<Status>();
    status.text = msg.into();
    status.until = now + 6.0;
}

fn say_ctx(status: &mut Status, time: &Time, msg: impl Into<String>) {
    status.text = msg.into();
    status.until = time.elapsed_secs_f64() + 6.0;
}

// ------------------------------------------------------------------ the scene

pub fn view() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            display: Display::None,
            flex_direction: FlexDirection::Row,
            column_gap: px(8),
        }
        ClsView
        Children [
            Node {
                width: px(250),
                flex_shrink: 0.0,
                min_height: px(0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                padding: UiRect::all(px(8)),
                border_radius: BorderRadius::all(px(4)),
                overflow: Overflow::scroll_y(),
            }
            BackgroundColor(colors::CARD)
            ScrollArea
            ClsSide
            --
            Node {
                flex_grow: 1.0,
                min_width: px(300),
                min_height: px(0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(8),
                overflow: Overflow::scroll_y(),
            }
            ScrollArea
            ClsMain
        ]
    }
}

// ------------------------------------------------------------------ jobs

fn reload(st: &mut ClsState, store: &Store) {
    st.defs = store.classifiers().unwrap_or_default();
    st.defs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    st.lists = anend_core::lists::stats(store)
        .map(|v| {
            v.into_iter()
                .filter(|l| !l.members.is_empty())
                .map(|l| (l.list.label(), l.name, l.members.len()))
                .collect()
        })
        .unwrap_or_default();
    if st.selected.as_ref().is_some_and(|s| !st.defs.iter().any(|d| &d.name == s)) {
        st.select(None);
    }
    if st.selected.is_none()
        && let Some(d) = st.defs.first()
    {
        let n = d.name.clone();
        st.select(Some(n));
    }
    if let Some(d) = st.def() {
        let k = d.class_names().len();
        st.class = st.class.min(k.saturating_sub(1));
        st.out_class = st.out_class.min(k.saturating_sub(1));
    }
}

/// texts to measure a steering's bias on: the classifier's own, or the save's
fn bias_texts(store: &Store, def: &Definition) -> Vec<String> {
    let mut t: Vec<String> = store
        .examples(def)
        .map(|v| v.into_iter().map(|e| e.text).collect())
        .unwrap_or_default();
    if t.is_empty() {
        t = store
            .candidates(300)
            .map(|v| v.into_iter().map(|c| c.1).collect())
            .unwrap_or_default();
    }
    let step = (t.len() / 200).max(1);
    t.into_iter().step_by(step).take(200).collect()
}

fn start_bias(st: &mut ClsState, work: &Work, store: Arc<Store>) {
    let Some(def) = st.def().cloned() else { return };
    if !def.trained() {
        return;
    }
    let steer = st.steer.clone().unwrap_or_else(|| def.steer.clone());
    st.bias_busy = true;
    work.spawn("bias", move || {
        let r: Result<BiasReport, String> = (|| {
            let mut stack = store.stack(&def.name)?;
            let node = stack.node;
            stack.graph.nodes[node].clf.steer = steer;
            let texts = bias_texts(&store, &def);
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            if refs.is_empty() {
                return Err("no texts to measure on".into());
            }
            Ok(stack.bias(&refs, &pool()))
        })();
        Box::new(move |w: &mut World| {
            let mut st = w.resource_mut::<ClsState>();
            st.bias_busy = false;
            if st.selected.as_deref() == Some(def.name.as_str()) {
                st.bias = r.ok();
                st.bias_dirty = true;
            }
        })
    });
}

pub fn start_search(st: &mut ClsState, work: &Work, store: Arc<Store>) {
    let Some(def) = st.def().cloned() else { return };
    if !def.trained() {
        return;
    }
    let class = st.class;
    let q = st.query.trim().to_string();
    st.searching = true;
    st.results_dirty = true;
    work.spawn("search", move || {
        let r: Result<Searched, String> = (|| {
            let stack = store.stack(&def.name)?;
            let mut cands = store.candidates(SCAN)?;
            if !q.is_empty() {
                let found: std::collections::HashSet<Ref> =
                    store.search_with_variations(&q)?.into_iter().map(|h| h.r).collect();
                cands.retain(|c| found.contains(&c.0));
            }
            let heads: std::collections::HashSet<String> = def.classes.iter().map(|c| c.list.clone()).collect();
            cands.retain(|c| !heads.contains(&c.0.label()));
            let items: Vec<(Ref, &str)> = cands.iter().map(|c| (c.0.clone(), c.1.as_str())).collect();
            let rows = store.checked_rows(&stack, &items, &pool())?;
            let mut order: Vec<usize> = (0..cands.len()).collect();
            let p = |i: usize| rows[i].1.probs.get(class).copied().unwrap_or(0.0);
            order.sort_by(|&a, &b| p(b).total_cmp(&p(a)).then(a.cmp(&b)));
            let results = order
                .into_iter()
                .take(40)
                .map(|i| {
                    let (row, chk) = &rows[i];
                    Found {
                        r: cands[i].0.clone(),
                        reads: cands[i].1.clone(),
                        shown: cands[i].2.clone(),
                        prob: p(i),
                        top: row.top,
                        member: row.members.contains(&class) && chk.valid,
                        valid: chk.valid,
                    }
                })
                .collect();
            Ok(Searched {
                name: def.name.clone(),
                class,
                results,
                scanned: cands.len(),
                stack: stack.names(),
            })
        })();
        Box::new(move |w: &mut World| {
            let msg = {
                let mut st = w.resource_mut::<ClsState>();
                st.searching = false;
                st.results_dirty = true;
                match r {
                    Ok(s) if st.selected.as_deref() == Some(s.name.as_str()) => {
                        st.why.clear();
                        st.searched = Some(s);
                        None
                    }
                    Ok(_) => None,
                    Err(e) => Some(e),
                }
            };
            if let Some(e) = msg {
                say(w, e);
            }
        })
    });
}

fn start_why(st: &mut ClsState, work: &Work, store: Arc<Store>, f: &Found) {
    let Some(name) = st.selected.clone() else { return };
    let class = st.class;
    let text = f.reads.clone();
    let key = f.r.label();
    work.spawn("why", move || {
        let r: Result<Vec<WhyRow>, String> = (|| {
            let stack = store.stack(&name)?;
            Ok(stack
                .contributions(&text, class)?
                .into_iter()
                .map(|(k, o)| WhyRow {
                    name: k.name.clone(),
                    share: k.share,
                    classes: stack.defs[k.node].class_names(),
                    probs: o.probs.clone(),
                    top: o.top,
                    judged: k.node == stack.node,
                })
                .collect())
        })();
        Box::new(move |w: &mut World| match r {
            Ok(rows) => {
                let mut st = w.resource_mut::<ClsState>();
                st.why.insert(key, rows);
                st.results_dirty = true;
            }
            Err(e) => say(w, e),
        })
    });
}

/// yes or no on a text: one classifier's own, or end to end through the stack
pub fn start_judge(work: &Work, store: Arc<Store>, name: String, f: (Ref, String), class: String, yes: bool, stack: bool) {
    work.spawn("judge", move || {
        let j = Judged {
            key: Some(f.0),
            text: f.1,
        };
        let r = if stack {
            store
                .feedback_stack(&name, &j, &class, yes, 1.0)
                .map(|(e, _, touched)| (e, touched))
        } else {
            store.feedback_classifier(&name, &j, &class, yes, 1.0).map(|e| (e, vec![]))
        };
        Box::new(move |w: &mut World| {
            let msg = match &r {
                Ok((e, touched)) => format!(
                    "{}: {} on “{}”, {} → {}{}",
                    name,
                    if yes { "yes" } else { "no" },
                    e.class,
                    pct(e.before as f64),
                    pct(e.after as f64),
                    if touched.is_empty() {
                        String::new()
                    } else {
                        format!(" (reached {})", touched.join(", "))
                    }
                ),
                Err(e) => e.clone(),
            };
            say(w, msg);
            classifiers_changed(w);
            let store = w.resource::<Db>().0.clone();
            w.resource_scope(|w, mut st: Mut<ClsState>| {
                st.judging = false;
                let work = w.resource::<Work>();
                if st.searched.is_some() {
                    start_search(&mut st, work, store.clone());
                }
            });
            w.resource_mut::<crate::index_panel::IndexPanel>().reread = true;
        })
    });
}

/// a classifier's weights or steering changed: what came from them is read again
pub fn classifiers_changed(w: &mut World) {
    {
        let mut st = w.resource_mut::<ClsState>();
        st.reload = true;
        st.redraw = true;
        st.bias = None;
        st.bias_due = Some(0.0);
        st.buckets_reload = true;
        st.bucket_open = None;
    }
    w.resource_mut::<crate::tint::Tints>().reset();
}

fn start_buckets(st: &mut ClsState, work: &Work, store: Arc<Store>) {
    let Some(def) = st.def().cloned() else { return };
    if !def.trained() {
        st.buckets = None;
        return;
    }
    work.spawn("buckets", move || {
        let r: Result<Buckets, String> = (|| {
            let stack = store.stack(&def.name)?;
            let counts = store.bucket_counts(&stack)?;
            let k = def.class_names().len() as u32;
            let mut per = Vec::new();
            for c in (0..k).map(Some).chain([None]) {
                let key = c.unwrap_or(memo::UNCERTAIN);
                let mut bands = [0usize; 10];
                for ((cc, b), n) in &counts {
                    if *cc == key && (*b as usize) < 10 {
                        bands[*b as usize] = *n;
                    }
                }
                per.push((c, bands));
            }
            Ok(Buckets {
                name: def.name.clone(),
                indexed: store.memo_count(&stack)?,
                per,
            })
        })();
        Box::new(move |w: &mut World| {
            let mut st = w.resource_mut::<ClsState>();
            if let Ok(b) = r
                && st.selected.as_deref() == Some(b.name.as_str())
            {
                st.buckets = Some(b);
                st.buckets_dirty = true;
            }
        })
    });
}

fn start_index(st: &mut ClsState, work: &Work, store: Arc<Store>) {
    let Some(name) = st.selected.clone() else { return };
    let done = Arc::new(Mutex::new((0usize, 0usize)));
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = work.stop_flag();
    st.indexing = Some(Indexing {
        name: name.clone(),
        done: done.clone(),
        cancel: cancel.clone(),
    });
    st.buckets_dirty = true;
    work.spawn("index", move || {
        let r = (|| {
            let stack = store.stack(&name)?;
            store.memo_index(&stack, None, SCAN, &pool(), |d, t| {
                *done.lock().unwrap_or_else(|e| e.into_inner()) = (d, t);
                if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
                    Flow::Stop
                } else {
                    Flow::Go
                }
            })
        })();
        Box::new(move |w: &mut World| {
            let msg = match &r {
                Ok(rep) if rep.stopped => format!("indexing stopped at {} of {}", rep.computed + rep.cached, rep.total),
                Ok(rep) => format!(
                    "indexed {} texts in {:.1} s ({} read, {} kept from before)",
                    rep.total, rep.seconds, rep.computed, rep.cached
                ),
                Err(e) => e.clone(),
            };
            say(w, msg);
            let store = w.resource::<Db>().0.clone();
            w.resource_scope(|w, mut st: Mut<ClsState>| {
                st.indexing = None;
                st.buckets_dirty = true;
                start_buckets(&mut st, w.resource::<Work>(), store);
            });
            w.resource_mut::<crate::tint::Tints>().reset();
        })
    });
}

fn start_bucket_open(st: &mut ClsState, work: &Work, store: Arc<Store>, class: Option<u32>) {
    let Some(name) = st.selected.clone() else { return };
    work.spawn("bucket", move || {
        let r: Result<Vec<(Ref, String, f32)>, String> = (|| {
            let stack = store.stack(&name)?;
            let c = class.unwrap_or(memo::UNCERTAIN);
            let (filed, _) = store.class_bucket(&stack, c, 0..=9)?;
            Ok(filed
                .into_iter()
                .take(60)
                .map(|f| {
                    let p = match class {
                        Some(c) => f.row.probs.get(c as usize).copied().unwrap_or(0.0),
                        None => f.row.confidence(),
                    };
                    (f.r, f.text, p)
                })
                .collect())
        })();
        Box::new(move |w: &mut World| match r {
            Ok(v) => {
                let mut st = w.resource_mut::<ClsState>();
                st.bucket_open = Some((class, v));
                st.buckets_dirty = true;
            }
            Err(e) => say(w, e),
        })
    });
}

fn start_near(st: &mut ClsState, work: &Work, store: Arc<Store>, key: Option<Ref>, text: String) {
    let Some(name) = st.selected.clone() else { return };
    let by = if st.near_embedding {
        memo::By::Embedding
    } else {
        memo::By::Outputs
    };
    st.near_dirty = true;
    work.spawn("near", move || {
        let r: Result<Near, String> = (|| {
            let stack = store.stack(&name)?;
            let me = match &key {
                Some(k) => store.memo_rows(&stack, &[(k.clone(), text.as_str())], &pool())?.remove(0),
                None => memo::read(&stack, store.memo_fresh(&stack)?, &text),
            };
            let results = store.memo_near(&stack, &me, by, key.as_ref(), 16)?;
            Ok(Near {
                source: text.clone(),
                results,
                indexed: store.memo_count(&stack)?,
            })
        })();
        Box::new(move |w: &mut World| match r {
            Ok(n) => {
                let mut st = w.resource_mut::<ClsState>();
                st.near = Some(n);
                st.near_dirty = true;
            }
            Err(e) => say(w, e),
        })
    });
}

fn start_training(st: &mut ClsState, work: &Work, store: Arc<Store>) {
    let Some(def) = st.def().cloned() else { return };
    if st.training.is_some() {
        return;
    }
    let args = TrainArgs {
        epochs: Some(st.epochs.round().max(1.0) as usize),
        rounds: Some(st.rounds.round().max(1.0) as usize),
        fresh: st.fresh,
        threads: Some(anend_core::classify::pool::available().saturating_sub(1).max(1)),
        ..TrainArgs::default()
    };
    let live = Arc::new(Mutex::new(TrainLive {
        name: def.name.clone(),
        started: Some(std::time::Instant::now()),
        ..TrainLive::default()
    }));
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = work.stop_flag();
    st.training = Some(Training {
        live: live.clone(),
        cancel: cancel.clone(),
    });
    st.last_run = None;
    st.live_dirty = true;
    let name = def.name.clone();
    work.spawn("train", move || {
        let r: Result<(Summary, bool), String> = (|| {
            let mut t = store.prepare(&name, &args)?;
            let s = t.run(|tick| {
                let mut l = live.lock().unwrap_or_else(|e| e.into_inner());
                if tick.loss.is_finite() {
                    l.losses.push(tick.loss);
                }
                if tick.eval.is_some() {
                    l.eval = tick.eval;
                }
                l.last = Some(tick.clone());
                if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
                    Flow::Stop
                } else {
                    Flow::Go
                }
            })?;
            if s.cancelled {
                return Ok((s, false));
            }
            store.commit(&t, &s)?;
            Ok((s, true))
        })();
        Box::new(move |w: &mut World| {
            let msg = match &r {
                Ok((_, false)) => format!("“{}”: training stopped, nothing kept", name),
                Ok((s, true)) => format!(
                    "“{}” trained: {} on {} texts, {:.1} s",
                    name,
                    s.metrics.as_ref().map(|m| pct(m.accuracy)).unwrap_or_else(|| "–".into()),
                    s.measured_on,
                    s.seconds
                ),
                Err(e) => format!("“{}”: {}", name, e),
            };
            say(w, msg);
            {
                let mut st = w.resource_mut::<ClsState>();
                st.training = None;
                st.last_run = Some(r.map(|(s, _)| s));
                st.live_dirty = true;
            }
            classifiers_changed(w);
            w.resource_mut::<crate::index_panel::IndexPanel>().reread = true;
        })
    });
}

fn import_paths(config: &crate::config::Config) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![config.saves_dir.clone()];
    if let Some(h) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let h = PathBuf::from(h);
        dirs.push(h.join("Downloads"));
        dirs.push(h);
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        for p in crate::vfs::list(&d) {
            if p.extension().is_some_and(|x| x == anend_core::clsmodule::EXTENSION) && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn start_import(work: &Work, store: Arc<Store>, path: PathBuf) {
    work.spawn("import", move || {
        let r: Result<anend_core::clsmodule::Imported, String> = (|| {
            if crate::vfs::meta(&path).is_some_and(|m| m.len as usize > anend_core::clsmodule::MAX_SIZE) {
                return Err("that file is too big for a module".into());
            }
            let b = crate::vfs::read(&path).map_err(|e| format!("couldn't read {}: {}", path.display(), e))?;
            let m = anend_core::clsmodule::read(&b, anend_core::clsmodule::MAX_SIZE)?;
            store.import_module(&m, &anend_core::clsmodule::ImportOpts::default())
        })();
        Box::new(move |w: &mut World| {
            match &r {
                Ok(i) => {
                    let lists: Vec<String> = i
                        .lists
                        .iter()
                        .map(|(c, _, new)| format!("{}{}", c, if *new { " (new list)" } else { "" }))
                        .collect();
                    say(
                        w,
                        format!(
                            "took in “{}”{}: classes {}",
                            i.name,
                            if i.reused.is_empty() {
                                String::new()
                            } else {
                                format!(", reusing {}", i.reused.join(", "))
                            },
                            lists.join(", ")
                        ),
                    );
                    let mut st = w.resource_mut::<ClsState>();
                    st.importing = false;
                    let n = i.name.clone();
                    st.select(Some(n));
                }
                Err(e) => say(w, format!("couldn't take it in: {}", e)),
            }
            classifiers_changed(w);
        })
    });
}

// ------------------------------------------------------------------ ticking

/// what runs every frame in the classifiers view: reload, progress, steering's debounce
pub fn tick_classifiers(
    mut st: ResMut<ClsState>,
    session: Res<Session>,
    db: Res<Db>,
    work: Res<Work>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    if st.training.is_some() || st.indexing.is_some() {
        if now - st.live_at > 0.25 {
            st.live_at = now;
            st.live_dirty = true;
            st.buckets_dirty |= st.indexing.is_some();
        }
    }
    if session.route != Route::Classifiers {
        return;
    }
    if std::mem::take(&mut st.reload) {
        reload(&mut st, &db.0);
        st.redraw = true;
    }
    if let Some(due) = st.save_due
        && now >= due
    {
        st.save_due = None;
        if let (Some(name), Some(steer)) = (st.selected.clone(), st.steer.clone()) {
            match db.0.steer_classifier(&name, steer) {
                Ok(d) => {
                    if let Some(x) = st.defs.iter_mut().find(|x| x.name == d.name) {
                        *x = d;
                    }
                    st.buckets_reload = true;
                }
                Err(e) => warn!("steering didn't keep: {}", e),
            }
        }
    }
    if let Some(due) = st.bias_due
        && now >= due
        && !st.bias_busy
    {
        st.bias_due = None;
        start_bias(&mut st, &work, db.0.clone());
    }
    if st.indexing.is_none() && std::mem::take(&mut st.buckets_reload) {
        start_buckets(&mut st, &work, db.0.clone());
    }
}

// ------------------------------------------------------------------ drawing

/// a classifier in the list: its name, and how it's doing at the right
fn name_row(name: String, status: String, variant: ButtonVariant) -> impl Scene {
    bsn! {
        @bevy::feathers::controls::FeathersButton {
            @caption: bsn_list! {
                @bevy::feathers::display::caption(name)
                Node { flex_shrink: 1.0, min_width: px(0) }
                --
                Node { flex_grow: 1.0 }
                --
                @bevy::feathers::display::caption(status)
            },
            @variant: variant,
        }
        Node {
            justify_content: JustifyContent::Start,
            flex_shrink: 0.0,
            column_gap: px(8),
        }
    }
}

fn kind_word(d: &Definition) -> &'static str {
    if d.head == HeadKind::Multi { "labels" } else { "one class" }
}

fn status_word(d: &Definition, training: Option<&str>) -> String {
    if training == Some(d.name.as_str()) {
        "training…".into()
    } else if let Some(m) = d.last_training().and_then(|e| e.metrics.as_ref()) {
        pct(m.accuracy)
    } else if d.trained() {
        "trained".into()
    } else {
        "untrained".into()
    }
}

pub fn rebuild_classifiers(
    mut st: ResMut<ClsState>,
    session: Res<Session>,
    config: Res<crate::config::Config>,
    side: Single<Entity, With<ClsSide>>,
    main: Single<Entity, With<ClsMain>>,
    bar_side: Single<Entity, With<crate::bar::BarSide>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if session.route != Route::Classifiers {
        st.side.clear();
        return;
    }
    let c = &mut commands;
    let f = &*fonts;

    // the bar's side
    let line = match st.def() {
        Some(d) if d.trained() => format!(
            "{} classifiers · Enter searches “{}” for {}",
            st.defs.len(),
            d.name,
            d.class_names().get(st.class).cloned().unwrap_or_default()
        ),
        Some(d) => format!("{} classifiers · train “{}” to search with it", st.defs.len(), d.name),
        None => format!("{} classifiers", st.defs.len()),
    };
    if st.side != line {
        st.side = line.clone();
        c.entity(*bar_side).despawn_related::<Children>();
        c.spawn((
            Text::new(line),
            TextFont {
                font: (&f.regular).into(),
                font_size: FontSize::Px(12.0),
                ..default()
            },
            TextColor(crate::bar::BAR_DIM),
            Pickable::IGNORE,
            ChildOf(*bar_side),
        ));
    }

    if std::mem::take(&mut st.redraw) {
        draw_side(&mut st, c, *side, f);
        draw_main(&mut st, c, *main, f, &config);
        st.live_dirty = true;
        st.bias_dirty = true;
        st.results_dirty = true;
        st.buckets_dirty = true;
        st.near_dirty = true;
    }
    if let Some(b) = st.boxes.live
        && std::mem::take(&mut st.live_dirty)
    {
        c.entity(b).despawn_related::<Children>();
        draw_live(&st, c, b, f);
    }
    if let Some(b) = st.boxes.bias
        && std::mem::take(&mut st.bias_dirty)
    {
        c.entity(b).despawn_related::<Children>();
        draw_bias(&st, c, b, f);
    }
    if let Some(b) = st.boxes.results
        && std::mem::take(&mut st.results_dirty)
    {
        c.entity(b).despawn_related::<Children>();
        draw_results(&st, c, b, f);
    }
    if let Some(b) = st.boxes.buckets
        && std::mem::take(&mut st.buckets_dirty)
    {
        c.entity(b).despawn_related::<Children>();
        draw_buckets(&st, c, b, f);
    }
    if let Some(b) = st.boxes.near
        && std::mem::take(&mut st.near_dirty)
    {
        c.entity(b).despawn_related::<Children>();
        draw_near(&st, c, b, f);
    }
}

fn draw_side(st: &mut ClsState, c: &mut Commands, side: Entity, f: &Fonts) {
    c.entity(side).despawn_related::<Children>();
    heading(c, side, Icon::Classifier, "classifiers", f);
    if st.defs.is_empty() {
        let t = dim(c, side, "none yet: a classifier learns lists, then tells any text which list it goes with", f);
        c.entity(t).insert(Node {
            margin: UiRect::vertical(px(4)),
            ..default()
        });
    }
    let training = st.training_name();
    for d in &st.defs {
        let on = st.selected.as_deref() == Some(d.name.as_str()) && !st.creating && !st.importing;
        let name = d.name.clone();
        c.spawn_scene(name_row(
            d.name.clone(),
            status_word(d, training.as_deref()),
            if on { ButtonVariant::Primary } else { ButtonVariant::Plain },
        ))
        .insert((
            ChildOf(side),
            Tip(format!(
                "{} · {} · {} classes{}",
                d.name,
                kind_word(d),
                d.class_names().len(),
                if d.inputs.is_empty() {
                    String::new()
                } else {
                    format!(" · builds on {}", d.inputs.iter().map(|i| i.from.as_str()).collect::<Vec<_>>().join(", "))
                }
            )),
        ))
        .observe(move |_a: On<Activate>, mut st: ResMut<ClsState>| {
            st.select(Some(name.clone()));
        });
    }
    let r = row(c, side, 6.0);
    c.entity(r).insert(Node {
        margin: UiRect::top(px(8)),
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        column_gap: px(6),
        row_gap: px(4),
        ..default()
    });
    icon_label(
        c,
        r,
        Icon::New,
        "new".into(),
        if st.creating { ButtonVariant::Primary } else { ButtonVariant::Normal },
        "make a classifier from lists",
    )
    .observe(|_a: On<Activate>, mut st: ResMut<ClsState>| {
        st.creating = !st.creating;
        st.importing = false;
        st.reload = true;
    });
    icon_label(
        c,
        r,
        Icon::Download,
        "take in".into(),
        if st.importing { ButtonVariant::Primary } else { ButtonVariant::Normal },
        "take in a classifier module (.anendcls) from Downloads or the saves folder",
    )
    .observe(
        |_a: On<Activate>, mut st: ResMut<ClsState>, config: Res<crate::config::Config>| {
            st.importing = !st.importing;
            st.creating = false;
            st.imports = import_paths(&config);
            st.redraw = true;
        },
    );
}

fn draw_main(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, config: &crate::config::Config) {
    c.entity(main).despawn_related::<Children>();
    st.boxes = Boxes::default();
    if st.creating {
        draw_form(st, c, main, f);
        return;
    }
    if st.importing {
        draw_import(st, c, main, f);
        return;
    }
    let Some(d) = st.def().cloned() else {
        let k = card(c, main);
        heading(c, k, Icon::Classifier, "classifiers", f);
        for line in [
            "A classifier is a selector you grow: it learns from lists (each list a class), then says of any memory or note which class it belongs to, and how sure it is.",
            "Make one with “new” on the left: pick two or more lists (the lists view makes them), train it, then steer it, tell it yes or no on what it finds, and write what it finds back to lists for the next one to learn from.",
        ] {
            label(c, k, line, f, 14.0, colors::TEXT);
        }
        return;
    };
    draw_head(st, c, main, f, &d, config);
    draw_train(st, c, main, f, &d);
    if d.trained() {
        draw_steer(st, c, main, f, &d);
        draw_search(st, c, main, f, &d);
        draw_buckets_card(st, c, main, f, &d);
        draw_near_card(st, c, main, f, &d);
        draw_outputs(st, c, main, f, &d);
    }
}

fn draw_head(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, d: &Definition, config: &crate::config::Config) {
    let k = card(c, main);
    let top = row(c, k, 10.0);
    crate::icons::spawn_icon(c, top, Icon::Classifier, 22.0, colors::TEXT);
    bold(c, top, &d.name, f, 24.0, colors::TEXT);
    dim(
        c,
        top,
        &format!("{} · {} · version {}", kind_word(d), d.preset, d.version),
        f,
    );
    grow(c, top);
    let colouring = config.tint_by.as_deref() == Some(d.name.as_str());
    let name = d.name.clone();
    icon_label(
        c,
        top,
        Icon::Palette,
        if colouring { "colouring views".into() } else { "colour views".into() },
        ButtonVariant::selected(colouring),
        "tint the memory view's and the space's texts by this classifier's top class",
    )
    .observe(
        move |_a: On<Activate>,
              mut config: ResMut<crate::config::Config>,
              mut st: ResMut<ClsState>,
              mut tints: ResMut<crate::tint::Tints>| {
            config.tint_by = if config.tint_by.as_deref() == Some(name.as_str()) {
                None
            } else {
                Some(name.clone())
            };
            config.save();
            tints.reset();
            st.redraw = true;
        },
    );
    if d.trained() {
        let name = d.name.clone();
        icon_button(c, top, Icon::Export, "save it as a module (.anendcls) in Downloads", ButtonVariant::Normal).observe(
            move |_a: On<Activate>, db: Res<Db>, mut status: ResMut<Status>, time: Res<Time>| {
                let o = anend_core::clsmodule::ExportOpts {
                    upstream: true,
                    ..Default::default()
                };
                let msg = match db.0.export_module(&name, &o).and_then(|b| {
                    crate::vfs::save_download(&format!("{}.{}", name, anend_core::clsmodule::EXTENSION), &b)
                }) {
                    Ok(to) => format!("saved “{}” to {}", name, to),
                    Err(e) => e,
                };
                say_ctx(&mut status, &time, msg);
            },
        );
    }
    let name = d.name.clone();
    let confirm = st.confirm_delete;
    let mut del = if confirm {
        button(c, top, "delete it?".into(), ButtonVariant::Primary, "click again to delete it, its weights and its feedback")
    } else {
        icon_button(c, top, Icon::Delete, "delete this classifier (asks first)", ButtonVariant::Normal)
    };
    del.observe(
        move |_a: On<Activate>,
              mut st: ResMut<ClsState>,
              db: Res<Db>,
              mut status: ResMut<Status>,
              time: Res<Time>,
              mut config: ResMut<crate::config::Config>| {
            if !st.confirm_delete {
                st.confirm_delete = true;
                st.redraw = true;
                return;
            }
            match db.0.delete_classifier(&name) {
                Ok(()) => {
                    say_ctx(&mut status, &time, format!("deleted “{}”", name));
                    if config.tint_by.as_deref() == Some(name.as_str()) {
                        config.tint_by = None;
                        config.save();
                    }
                    st.select(None);
                    st.reload = true;
                }
                Err(e) => {
                    st.confirm_delete = false;
                    st.redraw = true;
                    say_ctx(&mut status, &time, e);
                }
            }
        },
    );

    // its classes, with their colours and lists
    let classes = d.class_names();
    let r = row(c, k, 6.0);
    dim(c, r, "classes", f);
    for (i, n) in classes.iter().enumerate() {
        let ch = chip(c, r, class_color(i, classes.len()), n, f);
        let list = d.classes.get(i).map(|x| x.list.clone());
        let out = d.outputs.get(n).cloned();
        c.entity(ch).insert(Tip(match (list, out) {
            (Some(l), Some(o)) => format!("learns list {} · its outputs went to list {}", l, o),
            (Some(l), None) => format!("learns list {}", l),
            (None, _) => "the “not” list: texts that are none of the others".into(),
        }));
    }
    if let Some(n) = &d.not
        && d.head == HeadKind::Multi
    {
        dim(c, r, &format!("not: {}", n.name), f);
    }
    // what it builds on, each link's coupling
    if !d.inputs.is_empty() {
        let r = col(c, k, 4.0);
        dim(
            c,
            r,
            "builds on (their outputs are its inputs; coupling: how much end-to-end yes or no reaches them)",
            f,
        );
        for (idx, i) in d.inputs.iter().enumerate() {
            let line = row(c, r, 8.0);
            let from = i.from.clone();
            button(c, line, i.from.clone(), ButtonVariant::Plain, "open it").observe(
                move |_a: On<Activate>, mut st: ResMut<ClsState>| st.select(Some(from.clone())),
            );
            let name = d.name.clone();
            slider(c, line, 0.0, 2.0, i.coupling, 0.05, 200.0).observe(
                move |ch: On<ValueChange<f32>>, mut commands: Commands, db: Res<Db>, mut st: ResMut<ClsState>| {
                    commands.entity(ch.source).insert(SliderValue(ch.value));
                    if let Ok(mut def) = db.0.classifier(&name)
                        && let Some(inp) = def.inputs.get_mut(idx)
                    {
                        inp.coupling = ch.value;
                        if db.0.put_classifier(&mut def, None).is_ok()
                            && let Some(x) = st.defs.iter_mut().find(|x| x.name == def.name)
                        {
                            *x = def;
                        }
                    }
                },
            );
        }
    }
    // feedback given, and how it grew
    let growth: Vec<&anend_core::classifiers::Event> = d
        .history
        .iter()
        .rev()
        .filter(|e| e.kind != "train")
        .take(6)
        .collect();
    if !growth.is_empty() {
        let r = col(c, k, 2.0);
        dim(c, r, "lately", f);
        for e in growth {
            let line = row(c, r, 6.0);
            let icon = match e.kind.as_str() {
                "growth" => Icon::Sort,
                "enlist" => Icon::List,
                _ => Icon::Check,
            };
            crate::icons::spawn_icon(c, line, icon, 13.0, colors::TEXT_DIM);
            label(c, line, &format!("{} · {}", e.kind, e.note), f, 12.5, colors::TEXT);
        }
    }
}

fn draw_train(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, d: &Definition) {
    let k = card(c, main);
    let top = row(c, k, 10.0);
    heading(c, top, Icon::Play, "training", f);
    grow(c, top);
    let busy = st.training_name();
    if busy.as_deref() == Some(d.name.as_str()) {
        icon_label(c, top, Icon::Close, "stop".into(), ButtonVariant::Normal, "stop the training; nothing is kept").observe(
            |_a: On<Activate>, st: Res<ClsState>| {
                if let Some(t) = &st.training {
                    t.cancel.store(true, Ordering::Relaxed);
                }
            },
        );
    } else if busy.is_some() {
        dim(c, top, &format!("“{}” is training; one at a time", busy.unwrap_or_default()), f);
    } else {
        let fresh = st.fresh;
        button(
            c,
            top,
            if fresh { "✓ from scratch".into() } else { "from scratch".into() },
            ButtonVariant::selected(fresh),
            "start from new weights instead of what it learned",
        )
        .observe(|_a: On<Activate>, mut st: ResMut<ClsState>| {
            st.fresh = !st.fresh;
            st.redraw = true;
        });
        icon_label(
            c,
            top,
            Icon::Play,
            if d.trained() { "train more".into() } else { "train".into() },
            ButtonVariant::Primary,
            if d.trained() {
                "train it more, on top of what it learned, from its lists as they are now"
            } else {
                "train it on its lists"
            },
        )
        .observe(|_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>| {
            start_training(&mut st, &work, db.0.clone());
            st.redraw = true;
        });
    }
    let sliders = row(c, k, 18.0);
    slider_row(c, sliders, "epochs", "passes over its texts", f, (1.0, 80.0, st.epochs, 1.0)).observe(
        |ch: On<ValueChange<f32>>, mut commands: Commands, mut st: ResMut<ClsState>| {
            commands.entity(ch.source).insert(SliderValue(ch.value));
            st.epochs = ch.value;
        },
    );
    slider_row(
        c,
        sliders,
        "growth rounds",
        "above 1, it may widen, deepen or prune itself between rounds, keeping what it can do",
        f,
        (1.0, 8.0, st.rounds, 1.0),
    )
    .observe(|ch: On<ValueChange<f32>>, mut commands: Commands, mut st: ResMut<ClsState>| {
        commands.entity(ch.source).insert(SliderValue(ch.value));
        st.rounds = ch.value;
    });
    let live = col(c, k, 6.0);
    st.boxes.live = Some(live);
}

fn draw_live(st: &ClsState, c: &mut Commands, b: Entity, f: &Fonts) {
    let Some(d) = st.def() else { return };
    let classes = d.class_names();
    if let Some(t) = &st.training {
        let l = t.live.lock().unwrap_or_else(|e| e.into_inner());
        if l.name == d.name {
            let (frac, line) = match &l.last {
                Some(tk) => {
                    let total = (tk.steps * tk.rounds.max(1)).max(1);
                    let done = tk.round * tk.steps + tk.step;
                    (
                        done as f32 / total as f32,
                        format!(
                            "{}epoch {} · step {}/{} · loss {:.3} · {:.0} tokens/s{}",
                            if tk.rounds > 1 {
                                format!("round {}/{} · ", tk.round + 1, tk.rounds)
                            } else {
                                String::new()
                            },
                            tk.epoch + 1,
                            tk.step,
                            tk.steps,
                            tk.loss,
                            tk.tokens_per_sec,
                            l.started
                                .map(|s| format!(" · {:.0} s", s.elapsed().as_secs_f64()))
                                .unwrap_or_default()
                        ),
                    )
                }
                None => (0.0, "reading its lists…".into()),
            };
            meter(c, b, frac, 520.0, 10.0, colors::ACCENT);
            label(c, b, &line, f, 13.0, colors::TEXT);
            if let Some((loss, acc, f1)) = l.eval {
                dim(
                    c,
                    b,
                    &format!("held out: accuracy {}, macro-F1 {:.3}, loss {:.3}", pct(acc), f1, loss),
                    f,
                );
            }
            spark(c, b, &l.losses, 520.0, 44.0);
            return;
        }
    }
    if let Some(Err(e)) = &st.last_run {
        label(c, b, &format!("the last run failed: {}", e), f, 13.0, Color::srgb(0.7, 0.25, 0.2));
    }
    if let Some(Ok(s)) = &st.last_run {
        label(
            c,
            b,
            &format!(
                "{} steps on {} texts ({} to learn, {} held out) in {:.1} s{}{}",
                s.steps,
                s.examples,
                s.train_examples,
                s.val_examples,
                s.seconds,
                if s.stopped_early { ", stopped early" } else { "" },
                if s.cancelled { ", stopped" } else { "" }
            ),
            f,
            13.0,
            colors::TEXT,
        );
        for g in &s.growth {
            let r = row(c, b, 6.0);
            crate::icons::spawn_icon(c, r, Icon::Sort, 13.0, colors::ACCENT);
            label(c, r, &format!("grew: {}", g), f, 13.0, colors::TEXT);
        }
    }
    let last = d.last_training();
    match last.and_then(|e| e.metrics.as_ref()) {
        Some(m) => {
            let r = row(c, b, 14.0);
            figure(c, r, &pct(m.accuracy), "accuracy", f);
            figure(c, r, &format!("{:.3}", m.macro_f1), "macro-F1", f);
            figure(c, r, &format!("{:.3}", m.log_loss), "log-loss", f);
            figure(
                c,
                r,
                &m.n.to_string(),
                &format!("texts {}", last.map(|e| e.measured_on.as_str()).unwrap_or("")),
                f,
            );
            confusion(c, b, &m.confusion, &classes, f);
            if let Some(e) = last {
                dim(c, b, &format!("last run: {}, {:.1} s", e.note, e.seconds), f);
            }
        }
        None if !d.trained() => {
            dim(
                c,
                b,
                "not trained yet: it learns its lists’ texts, then tells any text which list it belongs with",
                f,
            );
        }
        None => {}
    }
}

fn draw_steer(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, d: &Definition) {
    let k = card(c, main);
    let top = row(c, k, 10.0);
    heading(c, top, Icon::Sliders, "steering", f);
    dim(c, top, "yours: training never touches it", f);
    grow(c, top);
    icon_button(c, top, Icon::Undo, "back to neutral", ButtonVariant::Normal).observe(
        |_a: On<Activate>, mut st: ResMut<ClsState>, time: Res<Time>| {
            if let Some(d) = st.def() {
                let k = d.class_names().len();
                let mut s = d.steer.clone();
                s.gains = Default::default();
                s.heads = vec![anend_core::classify::HeadSteer::neutral(k)];
                st.steer = Some(s);
                let now = time.elapsed_secs_f64();
                st.save_due = Some(now);
                st.bias_due = Some(now);
                st.redraw = true;
            }
        },
    );
    let steer = st.steer.clone().unwrap_or_else(|| d.steer.clone());
    if st.steer.is_none() {
        st.steer = Some(steer.clone());
    }
    let classes = d.class_names();
    let cols = row(c, k, 24.0);
    c.entity(cols).insert(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        align_items: AlignItems::Start,
        column_gap: px(24),
        row_gap: px(8),
        ..default()
    });
    let left = col(c, cols, 4.0);
    bold(c, left, "bias per class", f, 13.0, colors::TEXT_DIM);
    for (i, n) in classes.iter().enumerate() {
        let r = row(c, left, 6.0);
        swatch(c, r, class_color(i, classes.len()), 10.0);
        let v = steer.heads[0].bias.get(i).copied().unwrap_or(0.0);
        slider_row(c, r, n, &format!("added to {}’s logit: up favours it, down holds it back", n), f, (-8.0, 8.0, v, 0.1))
            .observe(move |ch: On<ValueChange<f32>>, mut commands: Commands, mut st: ResMut<ClsState>, time: Res<Time>| {
                commands.entity(ch.source).insert(SliderValue(ch.value));
                steer_set(&mut st, &time, |s| {
                    if let Some(b) = s.heads[0].bias.get_mut(i) {
                        *b = ch.value;
                    }
                });
            });
    }
    let right = col(c, cols, 4.0);
    bold(c, right, "reading", f, 13.0, colors::TEXT_DIM);
    let h = &steer.heads[0];
    slider_row(c, right, "temperature", "above 1 softens, below 1 sharpens", f, (0.2, 5.0, h.temperature, 0.05)).observe(
        |ch: On<ValueChange<f32>>, mut commands: Commands, mut st: ResMut<ClsState>, time: Res<Time>| {
            commands.entity(ch.source).insert(SliderValue(ch.value));
            steer_set(&mut st, &time, |s| s.heads[0].temperature = ch.value.max(0.05));
        },
    );
    slider_row(c, right, "threshold", "how sure it must be to count a text in", f, (0.0, 1.0, h.threshold, 0.01)).observe(
        |ch: On<ValueChange<f32>>, mut commands: Commands, mut st: ResMut<ClsState>, time: Res<Time>| {
            commands.entity(ch.source).insert(SliderValue(ch.value));
            steer_set(&mut st, &time, |s| s.heads[0].threshold = ch.value.clamp(0.0, 1.0));
        },
    );
    bold(c, right, "channel gains", f, 13.0, colors::TEXT_DIM);
    let g = steer.gains;
    let mut gains: Vec<(&str, f32, &str)> = vec![
        ("letter", g.letter, "the letters' values, added to each byte"),
        ("word", g.word, "each word's sums"),
        ("side", g.side, "the whole text's sums at the head"),
    ];
    if !d.inputs.is_empty() {
        gains.push(("extra", g.extra, "what the classifiers it builds on say"));
    }
    for (which, v, tip) in gains {
        slider_row(c, right, which, tip, f, (0.0, 2.0, v, 0.05)).observe(
            move |ch: On<ValueChange<f32>>, mut commands: Commands, mut st: ResMut<ClsState>, time: Res<Time>| {
                commands.entity(ch.source).insert(SliderValue(ch.value));
                let v = ch.value.max(0.0);
                steer_set(&mut st, &time, |s| match which {
                    "letter" => s.gains.letter = v,
                    "word" => s.gains.word = v,
                    "side" => s.gains.side = v,
                    _ => s.gains.extra = v,
                });
            },
        );
    }
    let b = col(c, k, 6.0);
    st.boxes.bias = Some(b);
}

/// changes the steering being tried: the bias is measured again soon, and it's kept a
/// little after the hand stops
fn steer_set(st: &mut ClsState, time: &Time, f: impl FnOnce(&mut Steer)) {
    let Some(d) = st.def().cloned() else { return };
    let mut s = st.steer.clone().unwrap_or(d.steer);
    f(&mut s);
    st.steer = Some(s);
    let now = time.elapsed_secs_f64();
    st.bias_due = Some(now + 0.15);
    st.save_due = Some(now + 0.6);
}

fn draw_bias(st: &ClsState, c: &mut Commands, b: Entity, f: &Fonts) {
    let Some(d) = st.def() else { return };
    let Some(r) = &st.bias else {
        dim(c, b, if st.bias_busy { "measuring the bias…" } else { "the bias shows once it’s measured" }, f);
        return;
    };
    let top = row(c, b, 8.0);
    bold(c, top, "the bias it adds", f, 13.0, colors::TEXT_DIM);
    if st.bias_busy {
        dim(c, top, "· measuring…", f);
    }
    let figs = row(c, b, 14.0);
    figure(c, figs, &pct(r.flipped), "decisions flipped", f);
    figure(c, figs, &format!("{:.3}", r.mean_kl), "mean KL (nats)", f);
    figure(c, figs, &format!("{:.3}", r.max_kl), "most KL", f);
    figure(c, figs, &format!("{:.3}", r.prior_skew), "class skew", f);
    let classes = d.class_names();
    for (i, n) in classes.iter().enumerate() {
        let v = r.shift.get(i).copied().unwrap_or(0.0);
        let line = row(c, b, 8.0);
        c.entity(line).insert(Tip(format!(
            "{}: {}{:.1} points on average, {} of the texts it learned",
            n,
            if v >= 0.0 { "+" } else { "" },
            v * 100.0,
            pct(r.prior.get(i).copied().unwrap_or(0.0))
        )));
        swatch(c, line, class_color(i, classes.len()), 10.0);
        let t = label(c, line, &clip(n, 16), f, 13.0, colors::TEXT);
        c.entity(t).insert(Node { width: px(120), ..default() });
        shift_bar(c, line, (v * 2.0) as f32, 120.0);
        label(c, line, &format!("{}{:.1}", if v >= 0.0 { "+" } else { "" }, v * 100.0), f, 12.0, colors::TEXT_DIM);
    }
    dim(c, b, &format!("over {} of its texts", r.texts), f);
}

fn draw_search(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, d: &Definition) {
    let k = card(c, main);
    let top = row(c, k, 8.0);
    heading(c, top, Icon::Search, "search by class", f);
    grow(c, top);
    let classes = d.class_names();
    for (i, n) in classes.iter().enumerate() {
        let on = st.class == i;
        let mut b = button(
            c,
            top,
            n.clone(),
            ButtonVariant::selected(on),
            &format!("the memories and notes most likely {}", n),
        );
        b.observe(move |_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>| {
            st.class = i;
            st.redraw = true;
            start_search(&mut st, &work, db.0.clone());
        });
    }
    icon_button(c, top, Icon::Search, "search (Enter in the bar: only texts with the words typed there)", ButtonVariant::Primary)
        .observe(|_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>| {
            start_search(&mut st, &work, db.0.clone());
        });
    let b = col(c, k, 4.0);
    st.boxes.results = Some(b);
}

fn draw_results(st: &ClsState, c: &mut Commands, b: Entity, f: &Fonts) {
    let Some(d) = st.def() else { return };
    let Some(s) = st.searched.as_ref().filter(|s| s.name == d.name) else {
        dim(
            c,
            b,
            if st.searching {
                "searching…"
            } else {
                "memories and notes, most likely first; tell it yes or no on each, and it learns. Words typed in the bar narrow the search."
            },
            f,
        );
        return;
    };
    let classes = d.class_names();
    let cname = classes.get(s.class).cloned().unwrap_or_default();
    let stacked = s.stack.len() > 1;
    if st.searching {
        dim(c, b, "searching again…", f);
    }
    if s.results.is_empty() {
        dim(c, b, "nothing to rank: the save has no memories or notes with those words", f);
    }
    for (idx, r) in s.results.iter().enumerate() {
        let line = row(c, b, 8.0);
        c.entity(line).insert(Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            align_items: AlignItems::Center,
            column_gap: px(8),
            padding: UiRect::axes(px(6), px(3)),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        });
        if idx % 2 == 0 {
            c.entity(line).insert(BackgroundColor(colors::PAD.with_alpha(0.35)));
        }
        let p = col(c, line, 2.0);
        c.entity(p).insert(Node {
            flex_direction: FlexDirection::Column,
            width: px(64),
            flex_shrink: 0.0,
            row_gap: px(2),
            ..default()
        });
        label(c, p, &pct(r.prob as f64), f, 12.5, colors::TEXT);
        meter(c, p, r.prob, 60.0, 4.0, class_color(s.class, classes.len()));
        let t = label(
            c,
            line,
            &format!("{}{}", if matches!(r.r, Ref::Note(_)) { "✎ " } else { "" }, clip(&r.shown, 120)),
            f,
            14.0,
            if r.valid { colors::TEXT } else { colors::TEXT_DIM },
        );
        c.entity(t).insert((
            Node {
                flex_grow: 1.0,
                flex_shrink: 1.0,
                min_width: px(0),
                ..default()
            },
            Pickable::default(),
            Tip(format!(
                "{} · says {}{}{}",
                r.r.label(),
                classes.get(r.top).cloned().unwrap_or_default(),
                if r.member { format!(" · counted in {}", cname) } else { String::new() },
                if r.valid { "" } else { " · its validators don't pass it" }
            )),
        ));
        if r.top != s.class {
            dim(c, line, &format!("({})", clip(&classes.get(r.top).cloned().unwrap_or_default(), 12)), f);
        }
        judge_buttons(c, line, &d.name, r, &cname, false);
        if stacked {
            let fr = r.clone();
            icon_button(c, line, Icon::Help, "why: which classifiers had a part in this", ButtonVariant::Normal).observe(
                move |_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>| {
                    let key = fr.r.label();
                    if st.why.remove(&key).is_some() {
                        st.results_dirty = true;
                    } else {
                        start_why(&mut st, &work, db.0.clone(), &fr);
                    }
                },
            );
        }
        let fr = r.clone();
        icon_button(c, line, Icon::Link, "texts near this one", ButtonVariant::Normal).observe(
            move |_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>| {
                start_near(&mut st, &work, db.0.clone(), Some(fr.r.clone()), fr.reads.clone());
            },
        );
        if let Some(why) = st.why.get(&r.r.label()) {
            let w = col(c, b, 3.0);
            c.entity(w).insert((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(3),
                    margin: UiRect::left(px(72)),
                    padding: UiRect::all(px(8)),
                    border: UiRect::left(px(3)),
                    ..default()
                },
                BorderColor::all(colors::ACCENT),
            ));
            dim(c, w, "each classifier’s part in this, and a yes or no on what it said:", f);
            for x in why {
                let wl = row(c, w, 8.0);
                let pp = col(c, wl, 2.0);
                c.entity(pp).insert(Node {
                    flex_direction: FlexDirection::Column,
                    width: px(56),
                    ..default()
                });
                label(c, pp, &pct(x.share), f, 12.0, colors::TEXT);
                meter(c, pp, x.share as f32, 50.0, 3.0, colors::ACCENT);
                bold(c, wl, &x.name, f, 13.0, colors::TEXT);
                let said = x.classes.get(x.top).cloned().unwrap_or_default();
                dim(
                    c,
                    wl,
                    &format!("says {} ({})", said, pct(x.probs.get(x.top).copied().unwrap_or(0.0) as f64)),
                    f,
                );
                grow(c, wl);
                // the judged classifier is told about the class searched; the others
                // about what they said
                let class = if x.judged { cname.clone() } else { said };
                judge_buttons(c, wl, &x.name, r, &class, false);
            }
            let end = row(c, w, 8.0);
            dim(c, end, "end to end, through the couplings:", f);
            judge_buttons(c, end, &d.name, r, &cname, true);
        }
    }
    if !s.results.is_empty() {
        dim(
            c,
            b,
            &format!("{} texts looked through · {}", s.scanned, s.stack.join(" → ")),
            f,
        );
    }
}

fn judge_buttons(c: &mut Commands, parent: Entity, name: &str, r: &Found, class: &str, stack: bool) {
    for yes in [true, false] {
        let (icon, tip) = if yes {
            (
                Icon::Check,
                if stack {
                    format!("yes, end to end: it is {}", class)
                } else {
                    format!("yes: {} is right, it is {}", name, class)
                },
            )
        } else {
            (
                Icon::Close,
                if stack {
                    format!("no, end to end: it isn't {}", class)
                } else {
                    format!("no: {} is wrong, it isn't {}", name, class)
                },
            )
        };
        let (name, class, key, text) = (name.to_string(), class.to_string(), r.r.clone(), r.reads.clone());
        icon_button(c, parent, icon, &tip, ButtonVariant::Normal).observe(
            move |_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>, mut status: ResMut<Status>, time: Res<Time>| {
                if st.judging {
                    say_ctx(&mut status, &time, "one yes or no at a time");
                    return;
                }
                if st.training.is_some() {
                    say_ctx(&mut status, &time, "wait for the training to finish");
                    return;
                }
                st.judging = true;
                say_ctx(&mut status, &time, format!("telling {} {}…", name, if yes { "yes" } else { "no" }));
                start_judge(&work, db.0.clone(), name.clone(), (key.clone(), text.clone()), class.clone(), yes, stack);
            },
        );
    }
}

fn draw_buckets_card(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, _d: &Definition) {
    let k = card(c, main);
    let top = row(c, k, 8.0);
    heading(c, top, Icon::Bucket, "buckets", f);
    dim(c, top, "every memory and note filed by class and how sure, kept until it changes", f);
    let b = col(c, k, 6.0);
    st.boxes.buckets = Some(b);
}

fn draw_buckets(st: &ClsState, c: &mut Commands, b: Entity, f: &Fonts) {
    let Some(d) = st.def() else { return };
    let top = row(c, b, 8.0);
    if let Some(ix) = st.indexing.as_ref().filter(|i| i.name == d.name) {
        let (done, total) = *ix.done.lock().unwrap_or_else(|e| e.into_inner());
        meter(c, top, if total > 0 { done as f32 / total as f32 } else { 0.0 }, 360.0, 10.0, colors::ACCENT);
        label(c, top, &format!("indexing {} of {}", done, total), f, 13.0, colors::TEXT);
        icon_button(c, top, Icon::Close, "stop indexing (what's done is kept)", ButtonVariant::Normal).observe(
            |_a: On<Activate>, st: Res<ClsState>| {
                if let Some(i) = &st.indexing {
                    i.cancel.store(true, Ordering::Relaxed);
                }
            },
        );
    } else {
        let indexed = st.buckets.as_ref().map(|x| x.indexed).unwrap_or(0);
        icon_label(
            c,
            top,
            Icon::Refresh,
            if indexed > 0 { "index again".into() } else { "index the save".into() },
            if indexed > 0 { ButtonVariant::Normal } else { ButtonVariant::Primary },
            "read every memory and note once and file it; what's filed and unchanged isn't read again",
        )
        .observe(|_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>, mut status: ResMut<Status>, time: Res<Time>| {
            if st.indexing.is_some() {
                say_ctx(&mut status, &time, "one index at a time");
                return;
            }
            start_index(&mut st, &work, db.0.clone());
        });
        dim(
            c,
            top,
            &if indexed > 0 {
                format!("{} texts filed", indexed)
            } else {
                "nothing filed for this version yet".into()
            },
            f,
        );
    }
    let Some(bk) = st.buckets.as_ref().filter(|x| x.name == d.name) else {
        return;
    };
    if bk.indexed == 0 {
        return;
    }
    let classes = d.class_names();
    let max = bk
        .per
        .iter()
        .map(|(_, bands)| bands.iter().sum::<usize>())
        .max()
        .unwrap_or(1)
        .max(1);
    for (cl, bands) in &bk.per {
        let n: usize = bands.iter().sum();
        let (name, color) = match cl {
            Some(i) => (
                classes.get(*i as usize).cloned().unwrap_or_default(),
                class_color(*i as usize, classes.len()),
            ),
            None => ("uncertain".to_string(), UNCLAIMED),
        };
        let line = row(c, b, 8.0);
        let open = st.bucket_open.as_ref().is_some_and(|(o, _)| o == cl);
        let cl2 = *cl;
        button(
            c,
            line,
            format!("{}  {}", clip(&name, 18), n),
            ButtonVariant::selected(open),
            &format!("the {} texts filed under {}, surest first", n, name),
        )
        .insert(Node {
            width: px(170),
            justify_content: JustifyContent::Start,
            ..default()
        })
        .observe(move |_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>| {
            if st.bucket_open.as_ref().is_some_and(|(o, _)| *o == cl2) {
                st.bucket_open = None;
                st.buckets_dirty = true;
            } else {
                start_bucket_open(&mut st, &work, db.0.clone(), cl2);
            }
        });
        meter(c, line, n as f32 / max as f32, 200.0, 8.0, color);
        // how sure: ten bands from 0 to 100 %
        let hist = c
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::End,
                    column_gap: px(1),
                    height: px(18),
                    ..default()
                },
                Tip(format!(
                    "{}: how sure, in tenths: {}",
                    name,
                    bands.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" · ")
                )),
                ChildOf(line),
            ))
            .id();
        let bmax = bands.iter().copied().max().unwrap_or(1).max(1) as f32;
        for &x in bands {
            c.spawn((
                Node {
                    width: px(6),
                    height: px(2.0 + 16.0 * x as f32 / bmax),
                    ..default()
                },
                BackgroundColor(color.with_alpha(if x > 0 { 0.9 } else { 0.2 })),
                Pickable::IGNORE,
                ChildOf(hist),
            ));
        }
        if open && let Some((_, hits)) = &st.bucket_open {
            let list = col(c, b, 2.0);
            c.entity(list).insert(Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                margin: UiRect::left(px(20)),
                ..default()
            });
            if hits.is_empty() {
                dim(c, list, "empty", f);
            }
            for (r, t, p) in hits {
                let l = row(c, list, 8.0);
                let pl = label(c, l, &pct(*p as f64), f, 12.0, colors::TEXT_DIM);
                c.entity(pl).insert(Node { width: px(52), ..default() });
                let tt = label(c, l, &clip(t, 110), f, 13.0, colors::TEXT);
                c.entity(tt).insert((Pickable::default(), Tip(r.label())));
                let (r2, t2) = (r.clone(), t.clone());
                icon_button(c, l, Icon::Link, "texts near this one", ButtonVariant::Plain).observe(
                    move |_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>| {
                        start_near(&mut st, &work, db.0.clone(), Some(r2.clone()), t2.clone());
                    },
                );
            }
        }
    }
}

fn draw_near_card(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, _d: &Definition) {
    let k = card(c, main);
    let top = row(c, k, 8.0);
    heading(c, top, Icon::Link, "near texts", f);
    grow(c, top);
    for (emb, word, tip) in [
        (true, "by embedding", "what it represents texts as"),
        (false, "by verdict", "what it says of them: its class probabilities"),
    ] {
        button(c, top, word.into(), ButtonVariant::selected(st.near_embedding == emb), tip).observe(
            move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                st.near_embedding = emb;
                st.redraw = true;
            },
        );
    }
    icon_label(c, top, Icon::Search, "near the bar's text".into(), ButtonVariant::Normal, "the filed texts nearest what's typed in the bar")
        .observe(|_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>, mut status: ResMut<Status>, time: Res<Time>| {
            let q = st.query.trim().to_string();
            if q.is_empty() {
                say_ctx(&mut status, &time, "type a text in the bar first");
                return;
            }
            start_near(&mut st, &work, db.0.clone(), None, q);
        });
    let b = col(c, k, 3.0);
    st.boxes.near = Some(b);
}

fn draw_near(st: &ClsState, c: &mut Commands, b: Entity, f: &Fonts) {
    let Some(n) = &st.near else {
        dim(
            c,
            b,
            "the link beside a result or a filed text finds the texts nearest it, among those filed in the buckets",
            f,
        );
        return;
    };
    let r = row(c, b, 6.0);
    dim(c, r, "near", f);
    bold(c, r, &clip(&n.source, 80), f, 13.0, colors::TEXT);
    if n.indexed == 0 {
        dim(c, b, "nothing is filed yet: index the save (buckets) to find neighbours", f);
    }
    for (rf, t, cos) in &n.results {
        let l = row(c, b, 8.0);
        let p = col(c, l, 2.0);
        c.entity(p).insert(Node {
            flex_direction: FlexDirection::Column,
            width: px(56),
            ..default()
        });
        label(c, p, &format!("{:.2}", cos), f, 12.0, colors::TEXT);
        meter(c, p, cos.max(0.0), 50.0, 3.0, colors::ACCENT);
        let tt = label(
            c,
            l,
            &format!("{}{}", if matches!(rf, Ref::Note(_)) { "✎ " } else { "" }, clip(t, 110)),
            f,
            13.0,
            colors::TEXT,
        );
        c.entity(tt).insert((Pickable::default(), Tip(rf.label())));
    }
}

fn draw_outputs(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, d: &Definition) {
    let k = card(c, main);
    let top = row(c, k, 8.0);
    heading(c, top, Icon::List, "outputs as a list", f);
    let classes = d.class_names();
    let r = row(c, k, 8.0);
    for (i, n) in classes.iter().enumerate() {
        button(c, r, n.clone(), ButtonVariant::selected(st.out_class == i), "the class to write").observe(
            move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                st.out_class = i;
                st.redraw = true;
            },
        );
    }
    slider_row(c, r, "at least", "how sure it must be of a text to write it", f, (0.0, 1.0, st.out_threshold, 0.05)).observe(
        |ch: On<ValueChange<f32>>, mut commands: Commands, mut st: ResMut<ClsState>| {
            commands.entity(ch.source).insert(SliderValue(ch.value));
            st.out_threshold = ch.value;
        },
    );
    let name = d.name.clone();
    let class = classes.get(st.out_class).cloned().unwrap_or_default();
    icon_label(c, r, Icon::List, "write the list".into(), ButtonVariant::Primary, "write the texts it counts in the class as a list (replacing what that list held)")
        .observe(move |_a: On<Activate>, st: Res<ClsState>, work: Res<Work>, db: Res<Db>, mut status: ResMut<Status>, time: Res<Time>| {
            let (name, class, thr) = (name.clone(), class.clone(), st.out_threshold);
            let store = db.0.clone();
            say_ctx(&mut status, &time, format!("writing “{}: {}”…", name, class));
            work.spawn("enlist", move || {
                let r = store.enlist_outputs(&name, &class, Some(thr), SCAN);
                Box::new(move |w: &mut World| {
                    match r {
                        Ok((head, n)) => say(w, format!("{} texts in the list “{}: {}” ({})", n, name, class, head.label())),
                        Err(e) => say(w, e),
                    }
                    w.resource_mut::<ClsState>().reload = true;
                    w.resource_mut::<crate::lists_ui::ListsState>().redraw = true;
                }) as Then
            });
        });
    dim(
        c,
        k,
        "the texts it counts in a class become a list another classifier can learn from",
        f,
    );
}

fn draw_form(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts) {
    let k = card(c, main);
    heading(c, k, Icon::New, "new classifier", f);
    let r = row(c, k, 8.0);
    dim(c, r, "name", f);
    c.spawn_scene(bsn! {
        @FeathersTextInputContainer
        Node { flex_grow: 0.0, min_width: px(260) }
        Children [
            @FeathersTextInput {
                @visible_width: 24f32,
            }
            ClsNameInput
        ]
    })
    .insert(ChildOf(r));
    if st.lists.is_empty() {
        label(
            c,
            k,
            "There are no lists with members yet. Make some in the lists view (Ctrl+6): each list becomes a class.",
            f,
            14.0,
            colors::TEXT,
        );
        return;
    }
    dim(c, k, "its classes: one per list (pick two or more, or one and a “not” list)", f);
    let r = row(c, k, 6.0);
    for (key, name, n) in &st.lists {
        let on = st.form.classes.contains(key);
        let key2 = key.clone();
        button(c, r, format!("{}{}  {}", if on { "✓ " } else { "" }, name, n), ButtonVariant::selected(on), &format!("list {} with {} members", key, n))
            .observe(move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                match st.form.classes.iter().position(|x| *x == key2) {
                    Some(i) => {
                        st.form.classes.remove(i);
                    }
                    None => {
                        st.form.classes.push(key2.clone());
                        if st.form.not.as_ref() == Some(&key2) {
                            st.form.not = None;
                        }
                    }
                }
                st.redraw = true;
            });
    }
    if st.lists.len() > st.form.classes.len() {
        dim(c, k, "a “not” list (optional): texts that are none of them", f);
    }
    let r = row(c, k, 6.0);
    for (key, name, _) in &st.lists {
        if st.form.classes.contains(key) {
            continue;
        }
        let on = st.form.not.as_ref() == Some(key);
        let key2 = key.clone();
        button(c, r, name.clone(), ButtonVariant::selected(on), "texts that are none of the classes").observe(
            move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                st.form.not = if st.form.not.as_ref() == Some(&key2) { None } else { Some(key2.clone()) };
                st.redraw = true;
            },
        );
    }
    let trained: Vec<String> = st.defs.iter().filter(|d| d.trained()).map(|d| d.name.clone()).collect();
    if !trained.is_empty() {
        dim(c, k, "builds on (their outputs become its inputs; trained ones only)", f);
        let r = row(c, k, 6.0);
        for n in trained {
            let on = st.form.inputs.contains(&n);
            let n2 = n.clone();
            button(c, r, n, ButtonVariant::selected(on), "").observe(move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                match st.form.inputs.iter().position(|x| *x == n2) {
                    Some(i) => {
                        st.form.inputs.remove(i);
                    }
                    None => st.form.inputs.push(n2.clone()),
                }
                st.redraw = true;
            });
        }
    }
    let r = row(c, k, 6.0);
    dim(c, r, "size", f);
    for p in anend_core::classifiers::PRESETS {
        let tip = match p {
            "tiny" => "smallest: quick to train, for a handful of short texts",
            "compact" => "89k weights: seconds for a few lists of short memories",
            "small" => "0.9M weights: minutes, for longer texts",
            _ => "5M weights: slow; for large lists",
        };
        button(c, r, p.into(), ButtonVariant::selected(st.form.preset == p), tip).observe(
            move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                st.form.preset = p.into();
                st.redraw = true;
            },
        );
    }
    dim(c, r, "· head", f);
    for (multi, word, tip) in [
        (false, "one class", "each text is in one class"),
        (true, "labels", "a text can carry any of the classes"),
    ] {
        button(c, r, word.into(), ButtonVariant::selected(st.form.multi == multi), tip).observe(
            move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                st.form.multi = multi;
                st.redraw = true;
            },
        );
    }
    let r = row(c, k, 8.0);
    icon_label(c, r, Icon::New, "make it".into(), ButtonVariant::Primary, "make the classifier; train it next")
        .observe(
            |_a: On<Activate>,
             mut st: ResMut<ClsState>,
             db: Res<Db>,
             names: Query<&EditableText, With<ClsNameInput>>,
             mut status: ResMut<Status>,
             time: Res<Time>| {
                let name = names
                    .iter()
                    .next()
                    .map(|e| e.value().to_string())
                    .unwrap_or_else(|| st.form.name.clone());
                let classes: Vec<Ref> = st.form.classes.iter().filter_map(|k| Ref::parse(k)).collect();
                let not = st.form.not.as_ref().and_then(|k| Ref::parse(k));
                let inputs: Vec<Input> = st
                    .form
                    .inputs
                    .iter()
                    .map(|n| Input {
                        from: n.clone(),
                        coupling: 1.0,
                    })
                    .collect();
                let head = if st.form.multi { HeadKind::Multi } else { HeadKind::Single };
                match db.0.create_classifier(&name, &classes, not.as_ref(), head, &st.form.preset, inputs) {
                    Ok(d) => {
                        say_ctx(&mut status, &time, format!("made “{}”: train it next", d.name));
                        st.form = Form {
                            preset: anend_core::classifiers::DEFAULT_PRESET.into(),
                            ..Form::default()
                        };
                        st.reload = true;
                        st.select(Some(d.name));
                    }
                    Err(e) => say_ctx(&mut status, &time, e),
                }
            },
        );
    button(c, r, "cancel".into(), ButtonVariant::Normal, "").observe(|_a: On<Activate>, mut st: ResMut<ClsState>| {
        st.creating = false;
        st.redraw = true;
    });
}

fn draw_import(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts) {
    let k = card(c, main);
    heading(c, k, Icon::Download, "take in a classifier", f);
    label(
        c,
        k,
        "A module (.anendcls) carries a trained classifier and what it builds on. Its classes become lists here (by name, or new ones). Files in Downloads, the saves folder and your home folder are listed; a path typed in the bar and Enter takes that file in.",
        f,
        13.5,
        colors::TEXT,
    );
    if st.imports.is_empty() {
        dim(c, k, "no .anendcls files found there", f);
    }
    for p in &st.imports {
        let r = row(c, k, 8.0);
        crate::icons::spawn_icon(c, r, Icon::Classifier, 15.0, colors::TEXT_DIM);
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        bold(c, r, &name, f, 14.0, colors::TEXT);
        let size = crate::vfs::meta(p).map(|m| format!("{:.1} KB", m.len as f64 / 1024.0)).unwrap_or_default();
        dim(c, r, &format!("{} · {}", size, p.parent().map(|d| d.display().to_string()).unwrap_or_default()), f);
        grow(c, r);
        let p2 = p.clone();
        icon_label(c, r, Icon::Download, "take in".into(), ButtonVariant::Primary, "take this module into the save").observe(
            move |_a: On<Activate>, work: Res<Work>, db: Res<Db>, mut status: ResMut<Status>, time: Res<Time>| {
                say_ctx(&mut status, &time, "taking it in…");
                start_import(&work, db.0.clone(), p2.clone());
            },
        );
    }
}

/// the name typed in the form outlives the form being drawn again
pub fn keep_form_name(
    mut st: ResMut<ClsState>,
    inputs: Query<(&mut EditableText, bevy::prelude::Ref<ClsNameInput>)>,
) {
    for (mut e, marker) in inputs {
        if marker.is_added() {
            if !st.form.name.is_empty() {
                let n = st.form.name.clone();
                e.editor_mut().set_text(&n);
            }
        } else if e.is_changed() {
            let v = e.value().to_string();
            if v != st.form.name {
                st.form.name = v;
            }
        }
    }
}

// ------------------------------------------------------------------ the bar

/// typing in the bar keeps the words a search narrows to
pub fn bar_changed(
    change: On<TextEditChange>,
    session: Res<Session>,
    inputs: Query<&EditableText, With<crate::bar::Bar>>,
    mut st: ResMut<ClsState>,
) {
    if session.route != Route::Classifiers {
        return;
    }
    if let Ok(input) = inputs.get(change.event_target()) {
        st.query = input.value().to_string();
    }
}

/// Enter searches the open classifier's class (or takes in a typed `.anendcls` path)
pub fn bar_key(
    key: On<FocusedInput<KeyboardInput>>,
    session: Res<Session>,
    mut st: ResMut<ClsState>,
    work: Res<Work>,
    db: Res<Db>,
    mut focus: ResMut<InputFocus>,
    mut status: ResMut<Status>,
    time: Res<Time>,
) {
    if !key.input.state.is_pressed() || session.route != Route::Classifiers {
        return;
    }
    match key.input.logical_key {
        Key::Escape => focus.clear(),
        Key::Enter => {
            let q = st.query.trim().to_string();
            if q.ends_with(&format!(".{}", anend_core::clsmodule::EXTENSION)) {
                say_ctx(&mut status, &time, "taking it in…");
                start_import(&work, db.0.clone(), PathBuf::from(q));
                return;
            }
            match st.def() {
                Some(d) if d.trained() => start_search(&mut st, &work, db.0.clone()),
                Some(d) => {
                    let n = d.name.clone();
                    say_ctx(&mut status, &time, format!("train “{}” first", n))
                }
                None => say_ctx(&mut status, &time, "make a classifier first"),
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ANEND_SEED=<file.redb> cargo test -p anend seed_demo -- --ignored`: a save with
    /// three lists of short texts and a few notes, to try the classifiers on
    #[test]
    #[ignore]
    fn seed_demo() {
        let Some(path) = std::env::var_os("ANEND_SEED") else { return };
        let store = Store::open(std::path::Path::new(&path)).unwrap();
        let lists: [(&str, &[&str]); 3] = [
            (
                "fruit",
                &[
                    "apple", "pear", "plum", "peach", "cherry", "banana", "mango", "grape", "lemon", "lime", "orange",
                    "apricot", "fig", "kiwi", "melon", "papaya", "quince", "raspberry", "blackberry", "nectarine",
                    "a ripe plum", "sweet peach jam", "a basket of apples", "fresh mango slices",
                ],
            ),
            (
                "stones",
                &[
                    "granite", "basalt", "slate", "flint", "marble", "quartz", "obsidian", "shale", "limestone",
                    "sandstone", "chalk", "gneiss", "pumice", "jade", "onyx", "agate", "jasper", "feldspar", "mica",
                    "a slab of granite", "polished marble floor", "flint arrowheads", "grey slate roof",
                ],
            ),
            (
                "animals",
                &[
                    "fox", "wolf", "badger", "otter", "heron", "sparrow", "hare", "stoat", "weasel", "owl", "raven",
                    "deer", "boar", "lynx", "beaver", "falcon", "kestrel", "hedgehog", "the red fox runs",
                    "a heron in the reeds", "wolves howl at night", "an otter swims",
                ],
            ),
        ];
        for (head, members) in lists {
            let (h, _) = store.saturate(head).unwrap();
            let h = Ref::Memory(h);
            for m in members {
                let (k, _) = store.saturate(m).unwrap();
                store.enlist(&h, &Ref::Memory(k)).unwrap();
            }
        }
        for t in ["cherry tart", "a quartz crystal", "the owl at dusk", "lemon and lime", "basalt columns"] {
            store.saturate(t).unwrap();
        }
        for (title, md, tags) in [
            ("Orchard walk", "Picked plums and pears; the apricots were not ripe yet.", vec!["garden"]),
            ("Quarry notes", "Granite and slate beds, a seam of quartz, some flint.", vec!["field"]),
            ("Night birds", "An owl called twice; a fox crossed the lane near the badger set.", vec!["field"]),
        ] {
            let n = crate::note::Note {
                title: title.into(),
                markdown: md.into(),
                tags: tags.into_iter().map(String::from).collect(),
                ..Default::default()
            };
            store.save(&n).unwrap();
        }
    }
}
