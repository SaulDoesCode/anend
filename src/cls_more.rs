//! More of the classifiers view: validators (classifiers that check another) with the
//! texts they disagree on, and classifiers shared on a hub (browse, take, share). The
//! hub and the long reads run on worker threads (`jobs::Work`).

use bevy::feathers::controls::ButtonVariant;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, SliderValue, ValueChange};
use std::sync::Arc;

use anend_core::classifiers::Definition;
use anend_core::clsmodule::{self, ImportOpts};
use anend_core::filters::{Disagreement, Mode, Validator};
use anend_core::memory::Ref;
use anend_core::store::Store;

use crate::classify_ui::{ClsState, SCAN, after_import, judge_pair, say};
use crate::cls_widgets::*;
use crate::hub::{self, HubClassifier, Sort};
use crate::icons::{Icon, Tip};
use crate::jobs::{Work, pool};
use crate::theme::colors;
use crate::ui::{Db, Fonts, Status};

/// a text a classifier and its validators part ways on, with what the classifier reads
#[derive(Clone)]
pub struct Disputed {
    pub d: Disagreement,
    pub r: Ref,
    pub reads: String,
}

/// the hub's shared classifiers as browsed
#[derive(Default)]
pub struct HubBrowse {
    pub open: bool,
    pub sort: Sort,
    pub page: u32,
    /// the words searched for
    pub q: String,
    pub found: Vec<HubClassifier>,
    pub more: bool,
    pub busy: bool,
    pub error: Option<String>,
    /// load (again) from the first page
    pub reload: bool,
    /// the id being taken in
    pub taking: Option<String>,
    pub sharing: bool,
}

// ------------------------------------------------------------------ validators

/// the open classifier's validators, kept after an edit (called from the tick)
pub fn keep_validators(st: &mut ClsState, store: &Store) -> Result<(), String> {
    let Some(name) = st.selected.clone() else { return Ok(()) };
    match store.set_validators(&name, &st.vals) {
        Ok(v) => {
            st.vals = v;
            Ok(())
        }
        Err(e) => {
            st.vals = store.validators(&name).unwrap_or_default();
            st.redraw = true;
            Err(e)
        }
    }
}

pub fn start_disagree(st: &mut ClsState, work: &Work, store: Arc<Store>) {
    let Some(name) = st.selected.clone() else { return };
    st.disagree_busy = true;
    st.redraw = true;
    work.spawn("disagreements", move || {
        let r: Result<Vec<Disputed>, String> = (|| {
            let found = store.disagreements(&name, 40, SCAN, &pool())?;
            if found.is_empty() {
                return Ok(vec![]);
            }
            // what the classifier reads of each, for a yes or no
            let reads: std::collections::HashMap<String, (Ref, String)> = store
                .candidates(SCAN)?
                .into_iter()
                .map(|(r, t, _)| (r.label(), (r, t)))
                .collect();
            Ok(found
                .into_iter()
                .filter_map(|d| {
                    let (r, t) = reads.get(&d.key)?.clone();
                    Some(Disputed { d, r, reads: t })
                })
                .collect())
        })();
        Box::new(move |w: &mut World| {
            let err = {
                let mut st = w.resource_mut::<ClsState>();
                st.disagree_busy = false;
                st.redraw = true;
                match r {
                    Ok(v) if st.selected.as_deref() == Some(name.as_str()) => {
                        st.disagree = Some(v);
                        None
                    }
                    Ok(_) => None,
                    Err(e) => Some(e),
                }
            };
            if let Some(e) = err {
                say(w, e);
            }
        })
    });
}

/// the validators editor: which classifiers check this one, on what class, how sure
pub fn draw_checks(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, d: &Definition) {
    let k = card(c, main);
    let top = row(c, k, 8.0);
    heading(c, top, Icon::Shield, "validators", f);
    dim(c, top, "other classifiers that check this one", f);
    let others: Vec<&Definition> = st.defs.iter().filter(|x| x.name != d.name && x.trained()).collect();
    if st.vals.is_empty() {
        dim(
            c,
            k,
            "none: it says what it says. Add one and a text counts in its classes only when the validator agrees.",
            f,
        );
    }
    for (i, v) in st.vals.clone().into_iter().enumerate() {
        let r = row(c, k, 8.0);
        c.entity(r).insert((
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Center,
                column_gap: px(8),
                row_gap: px(4),
                padding: UiRect::all(px(6)),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            BackgroundColor(colors::PAD.with_alpha(0.4)),
        ));
        dim(c, r, "checked by", f);
        bold(c, r, &v.by, f, 14.0, colors::TEXT);
        dim(c, r, "saying", f);
        let classes = st
            .defs
            .iter()
            .find(|x| x.name == v.by)
            .map(|x| x.class_names())
            .unwrap_or_default();
        if classes.is_empty() {
            dim(c, r, "(gone, or not trained: skipped)", f);
        }
        for (j, cl) in classes.iter().enumerate() {
            let cl2 = cl.clone();
            let b = button(
                c,
                r,
                cl.clone(),
                ButtonVariant::selected(*cl == v.class),
                "the class it must say",
            )
            .observe(move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                if let Some(x) = st.vals.get_mut(i) {
                    x.class = cl2.clone();
                }
                st.vals_save = true;
                st.redraw = true;
            })
            .id();
            let sw = swatch(c, b, class_color(j, classes.len()), 9.0);
            c.entity(sw).insert(Node {
                width: px(9),
                height: px(9),
                margin: UiRect::left(px(6)),
                ..default()
            });
        }
        dim(c, r, "at least", f);
        slider(c, r, 0.0, 1.0, v.min, 0.05, 140.0).observe(
            move |ch: On<ValueChange<f32>>, mut commands: Commands, mut st: ResMut<ClsState>| {
                commands.entity(ch.source).insert(SliderValue(ch.value));
                if let Some(x) = st.vals.get_mut(i) {
                    x.min = ch.value;
                }
                st.vals_save = true;
            },
        );
        for (mode, word, tip) in [
            (
                Mode::Gate,
                "gate",
                "gate: a text it doesn't pass is in none of the classes",
            ),
            (
                Mode::Weigh,
                "scale",
                "scale: the probabilities are multiplied by the validator's",
            ),
        ] {
            button(c, r, word.into(), ButtonVariant::selected(v.mode == mode), tip).observe(
                move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                    if let Some(x) = st.vals.get_mut(i) {
                        x.mode = mode;
                    }
                    st.vals_save = true;
                    st.redraw = true;
                },
            );
        }
        grow(c, r);
        icon_button(c, r, Icon::Close, "take this validator off", ButtonVariant::Plain).observe(
            move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                if i < st.vals.len() {
                    st.vals.remove(i);
                }
                st.vals_save = true;
                st.disagree = None;
                st.redraw = true;
            },
        );
    }
    let free: Vec<&&Definition> = others
        .iter()
        .filter(|x| !st.vals.iter().any(|v| v.by == x.name))
        .collect();
    if st.vals.len() < 8 {
        let r = row(c, k, 6.0);
        if others.is_empty() {
            dim(c, r, "train another classifier to check this one with", f);
        } else if !free.is_empty() {
            dim(c, r, "+ checked by", f);
        }
        for o in free {
            let (by, class) = (o.name.clone(), o.class_names().first().cloned().unwrap_or_default());
            button(
                c,
                r,
                o.name.clone(),
                ButtonVariant::Normal,
                "add it as a validator (then pick the class it must say)",
            )
            .observe(move |_a: On<Activate>, mut st: ResMut<ClsState>| {
                st.vals.push(Validator {
                    by: by.clone(),
                    class: class.clone(),
                    min: 0.5,
                    mode: Mode::Gate,
                });
                st.vals_save = true;
                st.redraw = true;
            });
        }
    }
    dim(
        c,
        k,
        &format!(
            "A text counts in “{}”'s classes only when each validator says its class at least that sure (gate), or its probabilities are scaled by the validator's (scale). Search, buckets' texts, filters and colouring use the checked verdicts. Changes keep by themselves.",
            d.name
        ),
        f,
    );
}

/// the texts it counts in a class that a validator doesn't pass, to tell either one
pub fn draw_disagree(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, d: &Definition) {
    let k = card(c, main);
    let top = row(c, k, 8.0);
    heading(c, top, Icon::Alert, "disagreements", f);
    grow(c, top);
    if st.vals.is_empty() {
        dim(
            c,
            k,
            "add a validator first: then this lists the texts the two don't agree on",
            f,
        );
        return;
    }
    icon_label(
        c,
        top,
        Icon::Alert,
        if st.disagree_busy {
            "looking…".into()
        } else {
            "find them".into()
        },
        ButtonVariant::Normal,
        "find the texts it counts in a class that a validator doesn't pass",
    )
    .observe(
        |_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, db: Res<Db>| {
            if !st.disagree_busy {
                start_disagree(&mut st, &work, db.0.clone());
            }
        },
    );
    let Some(list) = st.disagree.clone() else {
        dim(
            c,
            k,
            "the texts it puts in a class while a validator says no; tell either one it was right",
            f,
        );
        return;
    };
    if list.is_empty() {
        dim(c, k, "they agree on every text", f);
        return;
    }
    let classes = d.class_names();
    for (n, x) in list.iter().enumerate() {
        let item = col(c, k, 3.0);
        c.entity(item).insert((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(3),
                padding: UiRect::axes(px(6), px(4)),
                ..default()
            },
            BackgroundColor(if n % 2 == 0 {
                colors::PAD.with_alpha(0.35)
            } else {
                Color::NONE
            }),
        ));
        let line = row(c, item, 8.0);
        let p = col(c, line, 2.0);
        c.entity(p).insert(Node {
            flex_direction: FlexDirection::Column,
            width: px(56),
            ..default()
        });
        label(c, p, &pct(x.d.prob as f64), f, 12.0, colors::TEXT);
        meter(c, p, x.d.prob, 50.0, 3.0, colors::ACCENT);
        let t = label(c, line, &clip(&x.d.text, 110), f, 13.5, colors::TEXT);
        c.entity(t)
            .insert((bevy::picking::Pickable::default(), Tip(x.d.key.clone())));
        let ci = classes.iter().position(|y| *y == x.d.class).unwrap_or(0);
        chip(c, line, class_color(ci, classes.len()), &x.d.class, f);
        grow(c, line);
        dim(c, line, &format!("{}:", d.name), f);
        judge_pair(c, line, &d.name, x.r.clone(), x.reads.clone(), &x.d.class, false);
        let checks = row(c, item, 8.0);
        c.entity(checks).insert(Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            column_gap: px(8),
            row_gap: px(2),
            margin: UiRect::left(px(64)),
            ..default()
        });
        for (by, p) in &x.d.checks {
            let vclass = st.vals.iter().find(|v| &v.by == by).map(|v| v.class.clone());
            dim(c, checks, &format!("{}: {}", by, vclass.clone().unwrap_or_default()), f);
            bold(c, checks, &pct(*p as f64), f, 12.5, colors::TEXT);
            if let Some(vc) = vclass {
                judge_pair(c, checks, by, x.r.clone(), x.reads.clone(), &vc, false);
            }
        }
    }
}

// ------------------------------------------------------------------ the hub

pub fn start_hub(st: &mut ClsState, work: &Work, url: String) {
    let b = &mut st.hub;
    if url.trim().is_empty() {
        b.error = Some("connect to a hub first: its address in the settings' hub section".into());
        b.found.clear();
        b.more = false;
        st.redraw = true;
        return;
    }
    if b.busy {
        return;
    }
    b.busy = true;
    let (q, sort, page) = (b.q.clone(), b.sort, b.page);
    st.redraw = true;
    work.spawn("hub classifiers", move || {
        let r = bevy::tasks::block_on(hub::classifiers(&url, &q, sort, page));
        Box::new(move |w: &mut World| {
            let mut st = w.resource_mut::<ClsState>();
            let b = &mut st.hub;
            b.busy = false;
            match r {
                Ok(p) => {
                    if page == 0 {
                        b.found.clear();
                    }
                    b.found.extend(p.classifiers);
                    b.more = p.more;
                    b.error = None;
                }
                Err(e) => b.error = Some(e),
            }
            st.redraw = true;
        })
    });
}

fn start_take(st: &mut ClsState, work: &Work, store: Arc<Store>, url: String, id: String) {
    if st.hub.taking.is_some() {
        return;
    }
    st.hub.taking = Some(id.clone());
    st.redraw = true;
    work.spawn("take classifier", move || {
        let r = (|| {
            let bytes = bevy::tasks::block_on(hub::classifier_module(&url, &id))?;
            let m = clsmodule::read(&bytes, hub::MAX_MODULE as usize)?;
            // classes go to lists of the same name, or new ones
            store.import_module(&m, &ImportOpts::default())
        })();
        Box::new(move |w: &mut World| {
            {
                let mut st = w.resource_mut::<ClsState>();
                st.hub.taking = None;
                st.redraw = true;
                if r.is_ok() {
                    st.hub.open = false;
                }
            }
            after_import(w, r);
        })
    });
}

pub fn start_share(
    st: &mut ClsState,
    work: &Work,
    store: Arc<Store>,
    config: &crate::config::Config,
    name: String,
) -> Result<(), String> {
    if config.hub_url.trim().is_empty() || config.hub_token.trim().is_empty() {
        return Err("connect to a hub with an app token first (the settings' hub section)".into());
    }
    if st.hub.sharing {
        return Err("sharing one already".into());
    }
    st.hub.sharing = true;
    let (url, token) = (config.hub_url.clone(), config.hub_token.clone());
    work.spawn("share classifier", move || {
        let r = (|| {
            let o = clsmodule::ExportOpts {
                upstream: true,
                ..Default::default()
            };
            let bytes = store.export_module(&name, &o)?;
            bevy::tasks::block_on(hub::share_classifier(&url, &token, &name, "", "", &bytes))
        })();
        Box::new(move |w: &mut World| {
            w.resource_mut::<ClsState>().hub.sharing = false;
            let msg = match r {
                Ok(u) if u.status == "approved" => format!("shared “{}” on the hub", name),
                Ok(_) => format!("shared “{}” on the hub: it shows once an admin approves it", name),
                Err(e) => format!("couldn't share “{}”: {}", name, e),
            };
            say(w, msg);
        })
    });
    Ok(())
}

/// the hub's shared classifiers: search (the bar), sort, take one in
pub fn draw_hub(st: &mut ClsState, c: &mut Commands, main: Entity, f: &Fonts, config: &crate::config::Config) {
    let k = card(c, main);
    let top = row(c, k, 8.0);
    heading(c, top, Icon::Hub, "classifiers on the hub", f);
    if !config.hub_url.trim().is_empty() {
        dim(c, top, &config.hub_url, f);
    }
    grow(c, top);
    for s in Sort::ALL {
        button(
            c,
            top,
            s.name().into(),
            ButtonVariant::selected(st.hub.sort == s),
            "order them",
        )
        .observe(move |_a: On<Activate>, mut st: ResMut<ClsState>| {
            st.hub.sort = s;
            st.hub.page = 0;
            st.hub.reload = true;
        });
    }
    icon_button(c, top, Icon::Refresh, "ask the hub again", ButtonVariant::Normal).observe(
        |_a: On<Activate>, mut st: ResMut<ClsState>| {
            st.hub.page = 0;
            st.hub.reload = true;
        },
    );
    label(
        c,
        k,
        "Classifiers people trained and shared. Taking one in brings its weights (and what it builds on, when bundled) into this save; its classes go to your lists of the same name, or to new ones. Words typed in the bar and Enter search names, descriptions, authors, classes and #tags.",
        f,
        13.5,
        colors::TEXT,
    );
    if !st.hub.q.is_empty() {
        dim(c, k, &format!("matching “{}”", st.hub.q), f);
    }
    if let Some(e) = &st.hub.error {
        label(c, k, e, f, 13.5, colors::LINK);
    }
    if st.hub.busy && st.hub.found.is_empty() {
        dim(c, k, "asking the hub…", f);
    } else if st.hub.found.is_empty() && st.hub.error.is_none() {
        dim(c, k, "none shared there yet (or none match)", f);
    }
    for h in st.hub.found.clone() {
        let item = col(c, main, 4.0);
        c.entity(item).insert((
            Node {
                width: percent(100),
                max_width: px(900),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                padding: UiRect::all(px(12)),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            BackgroundColor(colors::CARD),
        ));
        let r = row(c, item, 8.0);
        crate::icons::spawn_icon(c, r, Icon::Classifier, 18.0, colors::TEXT);
        bold(c, r, &h.name, f, 16.0, colors::TEXT);
        if h.featured {
            crate::icons::spawn_icon(c, r, Icon::Star, 14.0, colors::ACCENT);
        }
        dim(
            c,
            r,
            &format!(
                "by {} · {} · {} · v{}",
                if h.author.is_empty() { "someone" } else { &h.author },
                if h.head == "multi" { "labels" } else { "one class" },
                h.preset,
                h.version.max(1)
            ),
            f,
        );
        grow(c, r);
        crate::icons::spawn_icon(c, r, Icon::Star, 13.0, colors::TEXT_DIM);
        dim(c, r, &h.stars.to_string(), f);
        crate::icons::spawn_icon(c, r, Icon::Download, 13.0, colors::TEXT_DIM);
        dim(c, r, &h.downloads.to_string(), f);
        let taking = st.hub.taking.as_deref() == Some(h.id.as_str());
        let id = h.id.clone();
        icon_label(
            c,
            r,
            Icon::Download,
            if taking { "taking…".into() } else { "take".into() },
            ButtonVariant::Primary,
            "take it into this save: its classes to your lists of the same name, or new ones",
        )
        .observe(
            move |_a: On<Activate>,
                  mut st: ResMut<ClsState>,
                  work: Res<Work>,
                  db: Res<Db>,
                  config: Res<crate::config::Config>,
                  mut status: ResMut<Status>,
                  time: Res<Time>| {
                status.text = "taking it in…".into();
                status.until = time.elapsed_secs_f64() + 6.0;
                start_take(&mut st, &work, db.0.clone(), config.hub_url.clone(), id.clone());
            },
        );
        if !h.description.is_empty() {
            label(c, item, &clip(&h.description, 300), f, 13.5, colors::TEXT);
        }
        let r = row(c, item, 6.0);
        for (i, cl) in h.classes.iter().enumerate() {
            chip(c, r, class_color(i, h.classes.len()), cl, f);
        }
        let mut facts = vec![];
        if let Some(a) = h.accuracy {
            facts.push(format!("accuracy {}", pct(a)));
        }
        if let Some(m) = h.macro_f1 {
            facts.push(format!("macro-F1 {}", pct(m)));
        }
        if h.examples > 0 {
            facts.push(format!("{} examples", h.examples));
        }
        facts.push(format!("{} weights", h.weights));
        facts.push(format!("{:.0} KB", h.size as f64 / 1024.0));
        for t in &h.tags {
            facts.push(format!("#{}", t));
        }
        dim(c, item, &facts.join(" · "), f);
        if !h.upstream.is_empty() {
            let parts: Vec<String> = h
                .upstream
                .iter()
                .map(|u| {
                    if u.bundled {
                        format!("{} (bundled)", u.name)
                    } else if st.defs.iter().any(|d| d.name == u.name) {
                        format!("{} (not bundled; this save has one by that name)", u.name)
                    } else {
                        format!("{} (not bundled, and not in this save: it can't be taken)", u.name)
                    }
                })
                .collect();
            let missing = h
                .upstream
                .iter()
                .any(|u| !u.bundled && !st.defs.iter().any(|d| d.name == u.name));
            let r = row(c, item, 6.0);
            if missing {
                crate::icons::spawn_icon(c, r, Icon::Alert, 13.0, colors::LINK);
            }
            dim(c, r, &format!("builds on {}", parts.join(", ")), f);
        }
    }
    if st.hub.more {
        let r = row(c, main, 8.0);
        button(
            c,
            r,
            if st.hub.busy { "…".into() } else { "more".into() },
            ButtonVariant::Normal,
            "the next page",
        )
        .observe(
            |_a: On<Activate>, mut st: ResMut<ClsState>, work: Res<Work>, config: Res<crate::config::Config>| {
                if !st.hub.busy {
                    st.hub.page += 1;
                    start_hub(&mut st, &work, config.hub_url.clone());
                }
            },
        );
    }
}
