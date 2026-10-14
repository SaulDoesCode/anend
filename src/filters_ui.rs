//! The filters view (Ctrl+9): include and exclude texts by what classifiers say of them,
//! by words, lists, tags and kind, nested in "all of" and "any of" groups; run over the
//! memoized classifier buckets, kept as presets (which can include each other), and
//! written out as lists.

use bevy::feathers::controls::{ButtonVariant, FeathersTextInput, FeathersTextInputContainer};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{FocusedInput, InputFocus};
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEditChange};
use bevy::ui_widgets::{Activate, ScrollArea, SliderValue, ValueChange};

use anend_core::filters::{Expr, Preset, Results};

use crate::classify_ui::SCAN;
use crate::cls_widgets::*;
use crate::icons::{Icon, Tip};
use crate::jobs::{Work, pool};
use crate::theme::colors;
use crate::ui::{Db, Fonts, Notes, Route, Session, Status};

#[derive(Component, Default, Clone)]
pub struct FiltersView;
#[derive(Component, Default, Clone)]
pub struct FltSide;
#[derive(Component, Default, Clone)]
pub struct FltMain;
#[derive(Component, Default, Clone)]
pub struct FltNameInput;

#[derive(Resource)]
pub struct FiltersState {
    pub redraw: bool,
    pub reload: bool,
    pub presets: Vec<Preset>,
    /// the filter being built
    pub expr: Expr,
    /// the group new conditions go into
    pub target: Vec<usize>,
    pub name: String,
    /// classifiers (name, classes, trained) and lists (key, name) to pick from
    pub classifiers: Vec<(String, Vec<String>, bool)>,
    pub lists: Vec<(String, String)>,
    pub pick: Option<String>,
    pub query: String,
    pub results: Option<Results>,
    pub running: bool,
    side: String,
}

impl Default for FiltersState {
    fn default() -> Self {
        FiltersState {
            redraw: true,
            reload: true,
            presets: vec![],
            expr: Expr::And { of: vec![] },
            target: vec![],
            name: String::new(),
            classifiers: vec![],
            lists: vec![],
            pick: None,
            query: String::new(),
            results: None,
            running: false,
            side: String::new(),
        }
    }
}

// ------------------------------------------------------------------ the tree by paths

/// a group's members: `path` indexes through groups, looking through "not"
fn group_mut<'a>(e: &'a mut Expr, path: &[usize]) -> Option<&'a mut Vec<Expr>> {
    match e {
        Expr::Not { of } => group_mut(of, path),
        Expr::And { of } | Expr::Or { of } => match path.split_first() {
            None => Some(of),
            Some((i, rest)) => group_mut(of.get_mut(*i)?, rest),
        },
        _ => None,
    }
}

fn node_mut<'a>(e: &'a mut Expr, path: &[usize]) -> Option<&'a mut Expr> {
    match path.split_last() {
        None => Some(e),
        Some((i, parent)) => group_mut(e, parent)?.get_mut(*i),
    }
}

fn add(st: &mut FiltersState, x: Expr) {
    let target = st.target.clone();
    match group_mut(&mut st.expr, &target) {
        Some(g) => g.push(x),
        None => {
            st.target.clear();
            if let Some(g) = group_mut(&mut st.expr, &[]) {
                g.push(x);
            }
        }
    }
    st.redraw = true;
}

fn remove(st: &mut FiltersState, path: &[usize]) {
    if let Some((i, parent)) = path.split_last()
        && let Some(g) = group_mut(&mut st.expr, parent)
        && *i < g.len()
    {
        g.remove(*i);
        if st.target.starts_with(path) {
            st.target = parent.to_vec();
        }
    }
    st.redraw = true;
}

fn toggle_not(st: &mut FiltersState, path: &[usize]) {
    if let Some(n) = node_mut(&mut st.expr, path) {
        let old = std::mem::replace(n, Expr::And { of: vec![] });
        *n = match old {
            Expr::Not { of } => *of,
            x => Expr::Not { of: Box::new(x) },
        };
    }
    st.redraw = true;
}

fn toggle_mode(st: &mut FiltersState, path: &[usize]) {
    if let Some(mut n) = node_mut(&mut st.expr, path) {
        if let Expr::Not { of } = n {
            n = of;
        }
        let old = std::mem::replace(n, Expr::And { of: vec![] });
        *n = match old {
            Expr::And { of } => Expr::Or { of },
            Expr::Or { of } => Expr::And { of },
            x => x,
        };
    }
    st.redraw = true;
}

// ------------------------------------------------------------------ scene

pub fn view() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            display: Display::None,
            flex_direction: FlexDirection::Row,
            column_gap: px(8),
        }
        FiltersView
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
            FltSide
            --
            Node {
                flex_grow: 1.0,
                min_width: px(300),
                min_height: px(0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
            }
            Children [
                Node {
                    flex_grow: 1.0,
                    min_height: px(0),
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: px(8),
                    overflow: Overflow::scroll_y(),
                }
                ScrollArea
                FltMain
            ]
        ]
    }
}

// ------------------------------------------------------------------ running

fn run(st: &mut FiltersState, work: &Work, db: &Db) {
    let e = st.expr.clone();
    st.running = true;
    st.redraw = true;
    let store = db.0.clone();
    work.spawn("filter", move || {
        let r = store.run_filter(&e, 200, SCAN, &pool());
        Box::new(move |w: &mut World| {
            let err = {
                let mut st = w.resource_mut::<FiltersState>();
                st.running = false;
                st.redraw = true;
                match r {
                    Ok(r) => {
                        st.results = Some(r);
                        None
                    }
                    Err(e) => Some(e),
                }
            };
            if let Some(e) = err {
                let now = w.resource::<Time>().elapsed_secs_f64();
                let mut s = w.resource_mut::<Status>();
                s.text = e;
                s.until = now + 6.0;
            }
        })
    });
}

fn say(status: &mut Status, time: &Time, msg: impl Into<String>) {
    status.text = msg.into();
    status.until = time.elapsed_secs_f64() + 6.0;
}

// ------------------------------------------------------------------ drawing

pub fn rebuild_filters(
    mut st: ResMut<FiltersState>,
    session: Res<Session>,
    db: Res<Db>,
    notes: Res<Notes>,
    side: Single<Entity, With<FltSide>>,
    main: Single<Entity, With<FltMain>>,
    bar_side: Single<Entity, With<crate::bar::BarSide>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if session.route != Route::Filters {
        st.side.clear();
        return;
    }
    if std::mem::take(&mut st.reload) {
        st.presets = db.0.filter_presets().unwrap_or_default();
        st.classifiers = db
            .0
            .classifiers()
            .map(|v| v.into_iter().map(|d| (d.name.clone(), d.class_names(), d.trained())).collect())
            .unwrap_or_default();
        st.lists = anend_core::lists::stats(&db.0)
            .map(|v| {
                v.into_iter()
                    .filter(|l| !l.members.is_empty())
                    .map(|l| (l.list.label(), l.name))
                    .collect()
            })
            .unwrap_or_default();
        st.redraw = true;
    }
    let c = &mut commands;
    let f = &*fonts;
    let line = format!(
        "{} filters · words typed here go in with “+ words” · Enter runs",
        st.presets.len()
    );
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
    if !std::mem::take(&mut st.redraw) {
        return;
    }

    // the presets
    c.entity(*side).despawn_related::<Children>();
    heading(c, *side, Icon::Filter, "filters", f);
    if st.presets.is_empty() {
        dim(c, *side, "none kept yet: build one, then keep it under a name", f);
    }
    for p in st.presets.clone() {
        let k = col(c, *side, 2.0);
        c.entity(k).insert((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                padding: UiRect::all(px(6)),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            BackgroundColor(if st.name == p.name { colors::PAD } else { colors::CARD }),
        ));
        let r = row(c, k, 4.0);
        let pc = p.clone();
        button(c, r, p.name.clone(), ButtonVariant::Plain, "open it to change or run").observe(
            move |_a: On<Activate>, mut st: ResMut<FiltersState>, work: Res<Work>, db: Res<Db>| {
                st.expr = pc.expr.clone();
                st.name = pc.name.clone();
                st.target.clear();
                run(&mut st, &work, &db);
            },
        );
        grow(c, r);
        let n = p.name.clone();
        icon_button(c, r, Icon::List, "write what it finds as a list “filter: name”", ButtonVariant::Plain).observe(
            move |_a: On<Activate>, work: Res<Work>, db: Res<Db>, mut status: ResMut<Status>, time: Res<Time>| {
                let (n, store) = (n.clone(), db.0.clone());
                say(&mut status, &time, format!("writing “filter: {}”…", n));
                work.spawn("enlist filter", move || {
                    let r = store.enlist_filter(&n, SCAN, &pool());
                    Box::new(move |w: &mut World| {
                        let msg = match r {
                            Ok((h, k)) => format!("{} texts in the list “filter: {}” ({})", k, n, h.label()),
                            Err(e) => e,
                        };
                        let now = w.resource::<Time>().elapsed_secs_f64();
                        let mut s = w.resource_mut::<Status>();
                        s.text = msg;
                        s.until = now + 6.0;
                        w.resource_mut::<FiltersState>().reload = true;
                        w.resource_mut::<crate::lists_ui::ListsState>().redraw = true;
                    })
                });
            },
        );
        let n = p.name.clone();
        icon_button(c, r, Icon::Delete, "delete this filter", ButtonVariant::Plain).observe(
            move |_a: On<Activate>, mut st: ResMut<FiltersState>, db: Res<Db>, mut status: ResMut<Status>, time: Res<Time>| {
                match db.0.delete_filter(&n) {
                    Ok(()) => say(&mut status, &time, format!("deleted the filter “{}”", n)),
                    Err(e) => say(&mut status, &time, e),
                }
                st.reload = true;
            },
        );
        let t = dim(c, k, &clip(&p.expr.describe(), 90), f);
        c.entity(t).insert(Node { max_width: px(220), ..default() });
        if let Some(n) = p.count {
            dim(c, k, &format!("{} found last time{}", n, if p.list.is_some() { " · a list" } else { "" }), f);
        }
    }

    // the builder
    c.entity(*main).despawn_related::<Children>();
    let k = card(c, *main);
    let top = row(c, k, 8.0);
    heading(c, top, Icon::Filter, "build a filter", f);
    grow(c, top);
    icon_button(c, top, Icon::Undo, "start over", ButtonVariant::Normal).observe(
        |_a: On<Activate>, mut st: ResMut<FiltersState>| {
            st.expr = Expr::And { of: vec![] };
            st.target.clear();
            st.name.clear();
            st.results = None;
            st.redraw = true;
        },
    );
    let expr = st.expr.clone();
    let target = st.target.clone();
    draw_node(c, k, &expr, &mut vec![], &target, f, &st);

    // adding
    let k = card(c, *main);
    let top = row(c, k, 8.0);
    heading(c, top, Icon::New, "add a condition", f);
    dim(c, top, if st.target.is_empty() { "to the top group" } else { "to the marked group" }, f);
    let r = row(c, k, 6.0);
    dim(c, r, "classifier", f);
    if st.classifiers.is_empty() {
        dim(c, r, "none yet (the classifiers view, Ctrl+7)", f);
    }
    for (n, _, trained) in st.classifiers.clone() {
        let on = st.pick.as_ref() == Some(&n);
        let n2 = n.clone();
        button(c, r, n.clone(), ButtonVariant::selected(on), if trained { "then pick its class" } else { "not trained yet" }).observe(
            move |_a: On<Activate>, mut st: ResMut<FiltersState>| {
                st.pick = if st.pick.as_ref() == Some(&n2) { None } else { Some(n2.clone()) };
                st.redraw = true;
            },
        );
    }
    if let Some((n, classes, _)) = st.classifiers.iter().find(|x| Some(&x.0) == st.pick.as_ref()).cloned() {
        let r = row(c, k, 6.0);
        dim(c, r, &format!("{}: in class", n), f);
        for (i, cl) in classes.iter().enumerate() {
            let (n2, cl2) = (n.clone(), cl.clone());
            let ch = button(c, r, cl.clone(), ButtonVariant::Normal, "texts it puts in this class (at its threshold; change it below)")
                .observe(move |_a: On<Activate>, mut st: ResMut<FiltersState>| {
                    add(
                        &mut st,
                        Expr::Class {
                            classifier: n2.clone(),
                            class: cl2.clone(),
                            min: None,
                            max: None,
                            top: false,
                        },
                    );
                })
                .id();
            let sw = swatch(c, ch, class_color(i, classes.len()), 9.0);
            c.entity(sw).insert(Node {
                width: px(9),
                height: px(9),
                margin: UiRect::left(px(6)),
                ..default()
            });
        }
    }
    let r = row(c, k, 6.0);
    let q = st.query.trim().to_string();
    button(
        c,
        r,
        if q.is_empty() { "+ words (type them in the bar)".into() } else { format!("+ words “{}”", clip(&q, 24)) },
        ButtonVariant::Normal,
        "texts with these words (and weum's variations)",
    )
    .observe(|_a: On<Activate>, mut st: ResMut<FiltersState>, mut status: ResMut<Status>, time: Res<Time>| {
        let q = st.query.trim().to_string();
        if q.is_empty() {
            say(&mut status, &time, "type the words in the bar first");
            return;
        }
        add(&mut st, Expr::Words { q });
    });
    for (kind, word) in [("memories", "+ memories only"), ("notes", "+ notes only")] {
        button(c, r, word.into(), ButtonVariant::Normal, "").observe(move |_a: On<Activate>, mut st: ResMut<FiltersState>| {
            add(&mut st, Expr::Kind { kind: kind.into() });
        });
    }
    button(c, r, "+ group: any of".into(), ButtonVariant::Normal, "a group whose members are alternatives; new conditions go into it").observe(
        |_a: On<Activate>, mut st: ResMut<FiltersState>| {
            let target = st.target.clone();
            let len = group_mut(&mut st.expr, &target).map(|g| g.len()).unwrap_or(0);
            add(&mut st, Expr::Or { of: vec![] });
            let mut t = target;
            t.push(len);
            st.target = t;
        },
    );
    if !st.lists.is_empty() {
        let r = row(c, k, 6.0);
        dim(c, r, "in list", f);
        for (key, name) in st.lists.clone() {
            button(c, r, name, ButtonVariant::Normal, &format!("members of list {}", key)).observe(
                move |_a: On<Activate>, mut st: ResMut<FiltersState>| add(&mut st, Expr::List { list: key.clone() }),
            );
        }
    }
    let tags = crate::note::tag_counts(&notes.0);
    if !tags.is_empty() {
        let r = row(c, k, 6.0);
        dim(c, r, "notes tagged", f);
        for (t, _) in tags.into_iter().take(30) {
            button(c, r, format!("#{}", t), ButtonVariant::Normal, "").observe(move |_a: On<Activate>, mut st: ResMut<FiltersState>| {
                add(
                    &mut st,
                    Expr::Tags {
                        tags: vec![t.clone()],
                        any: false,
                    },
                )
            });
        }
    }
    let others: Vec<String> = st.presets.iter().map(|p| p.name.clone()).filter(|n| *n != st.name).collect();
    if !others.is_empty() {
        let r = row(c, k, 6.0);
        dim(c, r, "another filter", f);
        for n in others {
            button(c, r, format!("[{}]", n), ButtonVariant::Normal, "what that filter finds").observe(
                move |_a: On<Activate>, mut st: ResMut<FiltersState>| add(&mut st, Expr::Preset { name: n.clone() }),
            );
        }
    }

    // run and keep
    let k = card(c, *main);
    let r = row(c, k, 8.0);
    icon_label(c, r, Icon::Play, "run".into(), ButtonVariant::Primary, "run it over the memories and notes (Enter in the bar)").observe(
        |_a: On<Activate>, mut st: ResMut<FiltersState>, work: Res<Work>, db: Res<Db>| run(&mut st, &work, &db),
    );
    dim(c, r, "keep as", f);
    c.spawn_scene(bsn! {
        @FeathersTextInputContainer
        Node { flex_grow: 0.0, min_width: px(200) }
        Children [
            @FeathersTextInput {
                @visible_width: 18f32,
            }
            FltNameInput
        ]
    })
    .insert(ChildOf(r));
    icon_label(c, r, Icon::Save, "keep".into(), ButtonVariant::Normal, "keep it under this name (replacing one of the same name)").observe(
        |_a: On<Activate>, mut st: ResMut<FiltersState>, db: Res<Db>, mut status: ResMut<Status>, time: Res<Time>| {
            let name = st.name.trim().to_string();
            match db.0.save_filter(&name, st.expr.clone(), "") {
                Ok(p) => {
                    say(&mut status, &time, format!("kept the filter “{}”", p.name));
                    if let Some(n) = st.results.as_ref().map(|r| r.matched) {
                        let _ = db.0.note_filter_count(&p.name, n);
                    }
                    st.reload = true;
                }
                Err(e) => say(&mut status, &time, e),
            }
        },
    );
    if st.running {
        dim(c, k, "running…", f);
    }
    match &st.results {
        None => {
            dim(c, k, "what it finds shows here, the strongest first", f);
        }
        Some(res) => {
            dim(
                c,
                k,
                &format!(
                    "{} of {} texts{}{}",
                    res.matched,
                    res.scanned,
                    if res.classifiers.is_empty() { String::new() } else { format!(" · read {}", res.classifiers.join(", ")) },
                    if res.untrained.is_empty() { String::new() } else { format!(" · skipped untrained {}", res.untrained.join(", ")) }
                ),
                f,
            );
            for (i, x) in res.results.iter().take(80).enumerate() {
                let line = row(c, k, 8.0);
                if i % 2 == 0 {
                    c.entity(line).insert(BackgroundColor(colors::PAD.with_alpha(0.35)));
                }
                let p = col(c, line, 2.0);
                c.entity(p).insert(Node {
                    flex_direction: FlexDirection::Column,
                    width: px(56),
                    ..default()
                });
                label(c, p, &pct(x.score as f64), f, 12.0, colors::TEXT);
                meter(c, p, x.score, 50.0, 3.0, colors::ACCENT);
                let t = label(c, line, &format!("{}{}", if x.note { "✎ " } else { "" }, clip(&x.text, 120)), f, 13.5, colors::TEXT);
                c.entity(t).insert((Pickable::default(), Tip(x.key.clone())));
            }
        }
    }
}

/// a condition or group, with its controls, and a group's members under it
fn draw_node(c: &mut Commands, parent: Entity, e: &Expr, path: &mut Vec<usize>, target: &[usize], f: &Fonts, st: &FiltersState) {
    let (inner, excluded) = match e {
        Expr::Not { of } => (&**of, true),
        x => (x, false),
    };
    let line = row(c, parent, 8.0);
    c.entity(line).insert(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        align_items: AlignItems::Center,
        column_gap: px(8),
        row_gap: px(4),
        margin: UiRect::left(px(22.0 * path.len() as f32)),
        ..default()
    });
    let p = path.clone();
    if !path.is_empty() {
        button(
            c,
            line,
            if excluded { "✗ exclude".into() } else { "✓ include".into() },
            ButtonVariant::selected(!excluded),
            "include the texts it finds, or leave them out",
        )
        .observe(move |_a: On<Activate>, mut st: ResMut<FiltersState>| toggle_not(&mut st, &p));
    }
    match inner {
        Expr::And { of } | Expr::Or { of } => {
            let all = matches!(inner, Expr::And { .. });
            let p = path.clone();
            button(
                c,
                line,
                if all { "all of".into() } else { "any of".into() },
                ButtonVariant::Normal,
                "every member must hold (all of), or one is enough (any of)",
            )
            .observe(move |_a: On<Activate>, mut st: ResMut<FiltersState>| toggle_mode(&mut st, &p));
            let marked = target == path.as_slice();
            let p = path.clone();
            button(
                c,
                line,
                if marked { "● adding here".into() } else { "add here".into() },
                ButtonVariant::selected(marked),
                "new conditions go into this group",
            )
            .observe(move |_a: On<Activate>, mut st: ResMut<FiltersState>| {
                st.target = p.clone();
                st.redraw = true;
            });
            if of.is_empty() {
                dim(c, line, if all { "(nothing yet: holds for every text)" } else { "(empty: add an alternative)" }, f);
            }
            if !path.is_empty() {
                let p = path.clone();
                icon_button(c, line, Icon::Close, "remove this group", ButtonVariant::Plain)
                    .observe(move |_a: On<Activate>, mut st: ResMut<FiltersState>| remove(&mut st, &p));
            }
            for (i, x) in of.iter().enumerate() {
                path.push(i);
                draw_node(c, parent, x, path, target, f, st);
                path.pop();
            }
        }
        Expr::Class { classifier, class, min, .. } => {
            let classes = st
                .classifiers
                .iter()
                .find(|x| &x.0 == classifier)
                .map(|x| x.1.clone())
                .unwrap_or_default();
            let i = classes.iter().position(|x| x == class).unwrap_or(0);
            chip(c, line, class_color(i, classes.len().max(1)), &format!("{}: {}", classifier, class), f);
            dim(c, line, "at least", f);
            let p = path.clone();
            slider(c, line, 0.0, 1.0, min.unwrap_or(0.5), 0.05, 160.0).observe(
                move |ch: On<ValueChange<f32>>, mut commands: Commands, mut st: ResMut<FiltersState>| {
                    commands.entity(ch.source).insert(SliderValue(ch.value));
                    if let Some(mut n) = node_mut(&mut st.expr, &p) {
                        if let Expr::Not { of } = n {
                            n = of;
                        }
                        if let Expr::Class { min, .. } = n {
                            *min = Some(ch.value);
                        }
                    }
                },
            );
            if min.is_none() {
                dim(c, line, "(its threshold)", f);
            }
            let p = path.clone();
            icon_button(c, line, Icon::Close, "remove", ButtonVariant::Plain)
                .observe(move |_a: On<Activate>, mut st: ResMut<FiltersState>| remove(&mut st, &p));
        }
        x => {
            label(c, line, &x.describe(), f, 13.5, colors::TEXT);
            let p = path.clone();
            icon_button(c, line, Icon::Close, "remove", ButtonVariant::Plain)
                .observe(move |_a: On<Activate>, mut st: ResMut<FiltersState>| remove(&mut st, &p));
        }
    }
}

/// the name typed to keep a filter under outlives a redraw
pub fn keep_name(mut st: ResMut<FiltersState>, inputs: Query<(&mut EditableText, bevy::prelude::Ref<FltNameInput>)>) {
    for (mut e, marker) in inputs {
        if marker.is_added() {
            if !st.name.is_empty() {
                let n = st.name.clone();
                e.editor_mut().set_text(&n);
            }
        } else if e.is_changed() {
            let v = e.value().to_string();
            if v != st.name {
                st.name = v;
            }
        }
    }
}

pub fn bar_changed(
    change: On<TextEditChange>,
    session: Res<Session>,
    inputs: Query<&EditableText, With<crate::bar::Bar>>,
    mut st: ResMut<FiltersState>,
) {
    if session.route != Route::Filters {
        return;
    }
    if let Ok(input) = inputs.get(change.event_target()) {
        let q = input.value().to_string();
        if q.trim() != st.query.trim() {
            st.redraw = true;
        }
        st.query = q;
    }
}

pub fn bar_key(
    key: On<FocusedInput<KeyboardInput>>,
    session: Res<Session>,
    mut st: ResMut<FiltersState>,
    work: Res<Work>,
    db: Res<Db>,
    mut focus: ResMut<InputFocus>,
) {
    if !key.input.state.is_pressed() || session.route != Route::Filters {
        return;
    }
    match key.input.logical_key {
        Key::Escape => focus.clear(),
        Key::Enter => run(&mut st, &work, &db),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_edit_the_tree() {
        let mut st = FiltersState::default();
        add(&mut st, Expr::Words { q: "plum".into() });
        add(&mut st, Expr::Or { of: vec![] });
        st.target = vec![1];
        add(&mut st, Expr::Kind { kind: "notes".into() });
        toggle_not(&mut st, &[1]);
        // through the "not", the group still takes members
        st.target = vec![1];
        add(&mut st, Expr::Kind { kind: "memories".into() });
        toggle_mode(&mut st, &[1]);
        assert_eq!(st.expr.describe(), "(“plum” and not (notes and memories))");
        remove(&mut st, &[1, 0]);
        toggle_not(&mut st, &[1]);
        assert_eq!(st.expr.describe(), "(“plum” and (memories))");
        remove(&mut st, &[1]);
        assert!(st.target.is_empty());
        assert_eq!(st.expr.describe(), "(“plum”)");
    }
}
