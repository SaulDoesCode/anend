//! Colouring by a classifier: with one chosen (the "colour views" button in the
//! classifiers view, kept in the settings file, off by default), the memory view's texts
//! carry a bar and the space's texts take the colour of the class it puts them in, a
//! legend in the corner names the colours. The verdicts come from the classifier's
//! memoized buckets (`Store::tints`), read on a worker for the texts on screen only.

use bevy::feathers::controls::ButtonVariant;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;
use std::collections::{HashMap, HashSet};

use anend_core::memory::Ref;

use crate::cls_widgets::{UNCLAIMED, class_color, clip};
use crate::config::Config;
use crate::icons::{Icon, Tip};
use crate::jobs::{Work, pool};
use crate::memory_ui::MemoryState;
use crate::space::SpaceText;
use crate::theme::colors;
use crate::ui::{Db, Fonts, Route, Session};

/// a text's verdict: its class (none: no class claims it) and how sure
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tinted {
    pub class: Option<usize>,
    pub strength: f32,
}

#[derive(Resource, Default)]
pub struct Tints {
    /// the classifier colouring the views, once it's known to be usable
    pub name: Option<String>,
    pub classes: Vec<String>,
    /// key label -> its verdict (none: there's no text to read)
    pub map: HashMap<String, Option<Tinted>>,
    busy: bool,
    epoch: u64,
    /// verdicts arrived: the views redraw
    pub changed: bool,
    legend_for: Option<(Option<String>, Route, usize)>,
}

impl Tints {
    /// forgets every verdict (the classifier changed, or another was chosen)
    pub fn reset(&mut self) {
        self.map.clear();
        self.epoch += 1;
        self.busy = false;
        self.changed = true;
        self.name = None;
        self.legend_for = None;
    }

    /// a text's colour, and how strongly it holds it (0..1)
    pub fn color(&self, label: &str) -> Option<(Color, f32)> {
        self.name.as_ref()?;
        let t = (*self.map.get(label)?)?;
        let c = match t.class {
            Some(i) => class_color(i, self.classes.len()),
            None => UNCLAIMED,
        };
        Some((c, t.strength.clamp(0.0, 1.0)))
    }
}

/// the legend's box
#[derive(Component)]
pub struct Legend;

pub fn spawn_legend(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: px(14),
            bottom: px(48),
            display: Display::None,
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            padding: UiRect::all(px(10)),
            border_radius: BorderRadius::all(px(6)),
            max_width: px(300),
            ..default()
        },
        BackgroundColor(colors::CARD.with_alpha(0.94)),
        GlobalZIndex(20),
        Legend,
    ));
}

/// asks for the verdicts of the texts on screen that have none yet
pub fn request_tints(
    mut tints: ResMut<Tints>,
    config: Res<Config>,
    session: Res<Session>,
    mem: Res<MemoryState>,
    texts: Query<&SpaceText>,
    db: Res<Db>,
    work: Res<Work>,
) {
    let want = config.tint_by.clone();
    if want.is_none() {
        if tints.name.is_some() {
            tints.reset();
        }
        return;
    }
    if !matches!(session.route, Route::Memory | Route::Space) || tints.busy {
        return;
    }
    let name = want.unwrap_or_default();
    if tints.name.as_deref() != Some(name.as_str()) {
        tints.reset();
        match db.0.classifier(&name) {
            Ok(d) if d.trained() => {
                tints.classes = d.class_names();
                tints.name = Some(name.clone());
            }
            _ => return,
        }
    }
    let mut refs: Vec<Ref> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut need = |r: Ref, refs: &mut Vec<Ref>| {
        let l = r.label();
        if !tints.map.contains_key(&l) && seen.insert(l) {
            refs.push(r);
        }
    };
    match session.route {
        Route::Memory => {
            for h in &mem.results {
                need(h.r.clone(), &mut refs);
            }
        }
        _ => {
            for t in &texts {
                if let Some(r) = t.mem.as_deref().and_then(Ref::parse) {
                    need(r, &mut refs);
                }
            }
        }
    }
    if refs.is_empty() {
        return;
    }
    refs.truncate(1500);
    tints.busy = true;
    let epoch = tints.epoch;
    let store = db.0.clone();
    work.spawn("tints", move || {
        let r = (|| {
            let stack = store.stack(&name)?;
            store.tints(&stack, &refs, &pool())
        })();
        Box::new(move |w: &mut World| {
            let mut tints = w.resource_mut::<Tints>();
            if tints.epoch != epoch {
                return;
            }
            tints.busy = false;
            match r {
                Ok(v) => {
                    for (r, t) in refs.iter().zip(v) {
                        tints.map.insert(
                            r.label(),
                            t.map(|t| Tinted {
                                class: t.class,
                                strength: t.strength,
                            }),
                        );
                    }
                }
                Err(e) => {
                    // nothing more is asked of it until it changes
                    warn!("colouring by {}: {}", name, e);
                    for r in &refs {
                        tints.map.insert(r.label(), None);
                    }
                }
            }
            tints.changed = true;
        })
    });
}

/// verdicts that arrived reach the views: the memory view redraws, the space's texts
/// take their colours
pub fn apply_tints(
    mut tints: ResMut<Tints>,
    session: Res<Session>,
    mut mem: ResMut<MemoryState>,
    mut texts: Query<&mut SpaceText>,
) {
    if std::mem::take(&mut tints.changed) && session.route == Route::Memory {
        mem.redraw_results = true;
    }
    if session.route != Route::Space {
        return;
    }
    for mut t in &mut texts {
        let want = t
            .mem
            .as_deref()
            .and_then(|l| tints.color(l))
            .map(|(c, s)| mix(crate::space::INK, c, 0.45 + 0.55 * s));
        if t.class_color != want {
            t.class_color = want;
        }
    }
}

/// `a` moved towards `b` by `k`
pub fn mix(a: Color, b: Color, k: f32) -> Color {
    let (a, b) = (a.to_srgba(), b.to_srgba());
    Color::srgb(
        a.red + (b.red - a.red) * k,
        a.green + (b.green - a.green) * k,
        a.blue + (b.blue - a.blue) * k,
    )
}

/// the legend: which colour is which class, shown over the memory view and the space
pub fn rebuild_legend(
    mut tints: ResMut<Tints>,
    session: Res<Session>,
    legend: Single<(Entity, &mut Node), With<Legend>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    let (e, mut node) = legend.into_inner();
    let shown = tints.name.is_some() && matches!(session.route, Route::Memory | Route::Space);
    let want = if shown { Display::Flex } else { Display::None };
    if node.display != want {
        node.display = want;
    }
    // the space's help sits at the right, so its legend goes left
    let (left, right) = if session.route == Route::Space {
        (px(14), Val::Auto)
    } else {
        (Val::Auto, px(14))
    };
    if node.left != left || node.right != right {
        node.left = left;
        node.right = right;
    }
    let key = (tints.name.clone(), session.route, tints.classes.len());
    if !shown || tints.legend_for.as_ref() == Some(&key) {
        return;
    }
    tints.legend_for = Some(key);
    commands.entity(e).despawn_related::<Children>();
    let c = &mut commands;
    let head = c
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(6),
                ..default()
            },
            ChildOf(e),
        ))
        .id();
    crate::icons::spawn_icon(c, head, Icon::Classifier, 14.0, colors::TEXT_DIM);
    crate::cls_widgets::bold(c, head, &clip(tints.name.as_deref().unwrap_or(""), 24), &fonts, 13.0, colors::TEXT);
    crate::cls_widgets::grow(c, head);
    crate::cls_widgets::icon_button(c, head, Icon::Close, "stop colouring by it", ButtonVariant::Plain).observe(
        |_a: On<Activate>, mut config: ResMut<Config>, mut tints: ResMut<Tints>, mut st: ResMut<crate::classify_ui::ClsState>| {
            config.tint_by = None;
            config.save();
            tints.reset();
            st.redraw = true;
        },
    );
    let k = tints.classes.len();
    for (i, n) in tints.classes.iter().enumerate().map(|(i, n)| (Some(i), n.as_str())).chain([(None, "no class")]) {
        let r = c
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(e),
            ))
            .id();
        crate::cls_widgets::swatch(c, r, i.map(|i| class_color(i, k)).unwrap_or(UNCLAIMED), 11.0);
        crate::cls_widgets::label(c, r, &clip(n, 30), &fonts, 12.5, colors::TEXT);
    }
    let t = crate::cls_widgets::dim(c, e, "fainter: less sure", &fonts);
    c.entity(t).insert(Tip("a text's colour is its surest class; how strongly it shows is how sure".into()));
}
