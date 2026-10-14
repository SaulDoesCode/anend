//! The note's classifiers: a strip under the editor's toolbar (the classifier button)
//! where the open note picks which classifiers it's indexed on, and shows what each says
//! of it, a class chip in that classifier's colours, with a yes or no that teaches it.

use bevy::feathers::controls::ButtonVariant;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use anend_core::memory::Ref;

use crate::cls_widgets::*;
use crate::icons::{Icon, Tip};
use crate::jobs::{Work, pool};
use crate::theme::colors;
use crate::ui::{Db, Fonts, Route, Session, Status};

/// the strip's box in the editor
#[derive(Component, Default, Clone)]
pub struct IndexPanelBox;

/// what a chosen classifier says of the note
#[derive(Clone)]
pub struct Verdict {
    pub name: String,
    pub classes: Vec<String>,
    pub probs: Vec<f32>,
    pub top: usize,
    pub members: Vec<usize>,
    pub valid: bool,
    pub trained: bool,
}

#[derive(Resource, Default)]
pub struct IndexPanel {
    pub open: bool,
    pub dirty: bool,
    /// read the verdicts again (after a yes or no, or training)
    pub reread: bool,
    key: String,
    chosen: Vec<String>,
    all: Vec<(String, bool)>,
    verdicts: Vec<Verdict>,
    reading: bool,
    epoch: u64,
}

fn read_verdicts(panel: &mut IndexPanel, work: &Work, db: &Db, key: String) {
    let names = panel.chosen.clone();
    if names.is_empty() || key.is_empty() {
        panel.verdicts.clear();
        return;
    }
    panel.reading = true;
    panel.epoch += 1;
    let epoch = panel.epoch;
    let store = db.0.clone();
    work.spawn("verdicts", move || {
        let r = Ref::Note(key);
        let text = store.ref_text(&r).ok().flatten().unwrap_or_default();
        let mut out = Vec::new();
        for n in &names {
            let Ok(stack) = store.stack(n) else { continue };
            let classes = stack.def().class_names();
            if !stack.def().trained() || text.trim().is_empty() {
                out.push(Verdict {
                    name: n.clone(),
                    classes,
                    probs: vec![],
                    top: 0,
                    members: vec![],
                    valid: true,
                    trained: false,
                });
                continue;
            }
            if let Ok(mut v) = store.checked_rows(&stack, &[(r.clone(), text.as_str())], &pool()) {
                let (row, chk) = v.remove(0);
                out.push(Verdict {
                    name: n.clone(),
                    classes,
                    probs: chk.probs,
                    top: row.top,
                    members: row.members,
                    valid: chk.valid,
                    trained: true,
                });
            }
        }
        Box::new(move |w: &mut World| {
            let mut p = w.resource_mut::<IndexPanel>();
            if p.epoch == epoch {
                p.verdicts = out;
                p.reading = false;
                p.dirty = true;
            }
        })
    });
}

/// the strip, opened and filled for the note being edited
pub fn rebuild_index_panel(
    mut panel: ResMut<IndexPanel>,
    session: Res<Session>,
    db: Res<Db>,
    work: Res<Work>,
    boxes: Single<(Entity, &mut Node), With<IndexPanelBox>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    let (b, mut node) = boxes.into_inner();
    let want = if panel.open && session.route == Route::Editor && !session.view_mode {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != want {
        node.display = want;
        panel.dirty = true;
    }
    if want == Display::None {
        return;
    }
    let key = session.active.key.clone();
    if key != panel.key {
        // another note: read its choice and what its classifiers say
        panel.key = key.clone();
        panel.chosen = if key.is_empty() {
            vec![]
        } else {
            db.0.indexed_on(&Ref::Note(key.clone())).unwrap_or_default()
        };
        panel.all =
            db.0.classifiers()
                .map(|v| v.into_iter().map(|d| (d.name.clone(), d.trained())).collect())
                .unwrap_or_default();
        panel.verdicts.clear();
        read_verdicts(&mut panel, &work, &db, key.clone());
        panel.dirty = true;
    }
    if std::mem::take(&mut panel.reread) {
        read_verdicts(&mut panel, &work, &db, key.clone());
    }
    if !std::mem::take(&mut panel.dirty) {
        return;
    }
    let c = &mut commands;
    let f = &*fonts;
    c.entity(b).despawn_related::<Children>();
    let top = row(c, b, 6.0);
    crate::icons::spawn_icon(c, top, Icon::Classifier, 15.0, colors::TEXT_DIM);
    bold(c, top, "indexed on", f, 13.0, colors::TEXT);
    if key.is_empty() {
        dim(
            c,
            top,
            "save the note first, then pick the classifiers it's indexed on",
            f,
        );
        return;
    }
    if panel.all.is_empty() {
        dim(
            c,
            top,
            "no classifiers yet: make one in the classifiers view (Ctrl+7)",
            f,
        );
        return;
    }
    for (n, trained) in panel.all.clone() {
        let on = panel.chosen.contains(&n);
        let n2 = n.clone();
        button(
            c,
            top,
            format!("{}{}", if on { "✓ " } else { "" }, n),
            ButtonVariant::selected(on),
            &if trained {
                format!("index this note on {}", n)
            } else {
                format!("{} isn't trained yet", n)
            },
        )
        .observe(
            move |_a: On<Activate>,
                  mut panel: ResMut<IndexPanel>,
                  db: Res<Db>,
                  work: Res<Work>,
                  session: Res<Session>,
                  mut status: ResMut<Status>,
                  time: Res<Time>| {
                let mut chosen = panel.chosen.clone();
                match chosen.iter().position(|x| *x == n2) {
                    Some(i) => {
                        chosen.remove(i);
                    }
                    None => chosen.push(n2.clone()),
                }
                let key = session.active.key.clone();
                match db.0.set_indexed_on(&Ref::Note(key.clone()), &chosen) {
                    Ok(v) => {
                        panel.chosen = v;
                        read_verdicts(&mut panel, &work, &db, key);
                        panel.dirty = true;
                    }
                    Err(e) => {
                        status.text = e;
                        status.until = time.elapsed_secs_f64() + 5.0;
                    }
                }
            },
        );
    }
    grow(c, top);
    icon_button(c, top, Icon::Close, "close the strip", ButtonVariant::Plain).observe(
        |_a: On<Activate>, mut panel: ResMut<IndexPanel>| {
            panel.open = false;
        },
    );

    if panel.reading && panel.verdicts.is_empty() && !panel.chosen.is_empty() {
        dim(c, b, "reading what they say…", f);
    }
    for v in panel.verdicts.clone() {
        let line = row(c, b, 8.0);
        bold(c, line, &v.name, f, 13.0, colors::TEXT);
        if !v.trained {
            dim(c, line, "not trained yet (or the note is empty)", f);
            continue;
        }
        let k = v.classes.len();
        // the surest class first, then any other it counts the note in
        let mut shown: Vec<usize> = vec![v.top];
        for &m in &v.members {
            if !shown.contains(&m) {
                shown.push(m);
            }
        }
        for i in shown {
            let p = v.probs.get(i).copied().unwrap_or(0.0);
            let ch = chip(
                c,
                line,
                class_color(i, k),
                &format!("{} {}", v.classes.get(i).cloned().unwrap_or_default(), pct(p as f64)),
                f,
            );
            let member = v.members.contains(&i);
            c.entity(ch).insert(Tip(format!(
                "{} says {} ({}){}",
                v.name,
                v.classes.get(i).cloned().unwrap_or_default(),
                pct(p as f64),
                if member {
                    ", and counts the note in"
                } else {
                    ", not sure enough to count it in"
                }
            )));
            if !member {
                c.entity(ch).insert(BackgroundColor(colors::PAD.with_alpha(0.4)));
            }
        }
        if !v.valid {
            dim(c, line, "· its validators don't pass it", f);
        }
        grow(c, line);
        let cls = v.classes.get(v.top).cloned().unwrap_or_default();
        for yes in [true, false] {
            let (name, cls, key) = (v.name.clone(), cls.clone(), key.clone());
            icon_button(
                c,
                line,
                if yes { Icon::Check } else { Icon::Close },
                &if yes {
                    format!("yes: it is {}", cls)
                } else {
                    format!("no: it isn't {}", cls)
                },
                ButtonVariant::Normal,
            )
            .observe(
                move |_a: On<Activate>,
                      db: Res<Db>,
                      work: Res<Work>,
                      mut status: ResMut<Status>,
                      time: Res<Time>,
                      mut panel: ResMut<IndexPanel>| {
                    let r = Ref::Note(key.clone());
                    let text = db.0.ref_text(&r).ok().flatten().unwrap_or_default();
                    status.text = format!("telling {} {}…", name, if yes { "yes" } else { "no" });
                    status.until = time.elapsed_secs_f64() + 5.0;
                    panel.reading = true;
                    crate::classify_ui::start_judge(
                        &work,
                        db.0.clone(),
                        name.clone(),
                        (r, text),
                        cls.clone(),
                        yes,
                        false,
                    );
                },
            );
        }
    }
}
