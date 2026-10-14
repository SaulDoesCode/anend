//! Small building blocks the classifier views share: rows, labels, buttons, meters,
//! sliders, the loss line, the confusion grid, and the class colours.

use bevy::ecs::system::EntityCommands;
use bevy::feathers::controls::{ButtonVariant, FeathersSlider};
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::ui_widgets::{SliderPrecision, SliderStep, SliderValue};

use crate::icons::{Icon, Tip, icon_scene_button};
use crate::theme::colors;
use crate::ui::{Fonts, plain_scene_button};

/// the colour of class `i` of `k`, from the save's shared palette (the web app's too)
pub fn class_color(i: usize, k: usize) -> Color {
    let p = anend_core::memo::palette(k.max(i + 1));
    let [r, g, b] = p[i];
    Color::srgb_u8(r, g, b)
}

/// the colour of texts no class claims
pub const UNCLAIMED: Color = Color::srgb(0.55, 0.55, 0.55);

pub fn pct(x: f64) -> String {
    if x.is_finite() {
        format!("{:.1}%", x * 100.0)
    } else {
        "–".into()
    }
}

pub fn row(c: &mut Commands, parent: Entity, gap: f32) -> Entity {
    c.spawn((
        Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            column_gap: px(gap),
            row_gap: px(4),
            ..default()
        },
        ChildOf(parent),
    ))
    .id()
}

pub fn col(c: &mut Commands, parent: Entity, gap: f32) -> Entity {
    c.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(gap),
            ..default()
        },
        ChildOf(parent),
    ))
    .id()
}

/// a flexible gap in a row
pub fn grow(c: &mut Commands, parent: Entity) {
    c.spawn((
        Node {
            flex_grow: 1.0,
            ..default()
        },
        ChildOf(parent),
    ));
}

pub fn label(c: &mut Commands, parent: Entity, s: &str, fonts: &Fonts, size: f32, color: Color) -> Entity {
    let e = crate::ui::text(c, parent, s, &fonts.regular, size, color);
    c.entity(e).insert(Pickable::IGNORE);
    e
}

pub fn bold(c: &mut Commands, parent: Entity, s: &str, fonts: &Fonts, size: f32, color: Color) -> Entity {
    let e = crate::ui::text(c, parent, s, &fonts.bold, size, color);
    c.entity(e).insert(Pickable::IGNORE);
    e
}

pub fn dim(c: &mut Commands, parent: Entity, s: &str, fonts: &Fonts) -> Entity {
    label(c, parent, s, fonts, 12.5, colors::TEXT_DIM)
}

/// a card's heading: an icon and a word
pub fn heading(c: &mut Commands, parent: Entity, icon: Icon, s: &str, fonts: &Fonts) -> Entity {
    let r = row(c, parent, 8.0);
    crate::icons::spawn_icon(c, r, icon, 17.0, colors::TEXT_DIM);
    bold(c, r, s, fonts, 17.0, colors::TEXT);
    r
}

pub fn card(c: &mut Commands, parent: Entity) -> Entity {
    c.spawn((
        Node {
            row_gap: px(8),
            padding: UiRect::all(px(14)),
            ..crate::ui::card_node()
        },
        BackgroundColor(colors::CARD),
        ChildOf(parent),
    ))
    .id()
}

pub fn button<'a>(
    c: &'a mut Commands,
    parent: Entity,
    s: String,
    variant: ButtonVariant,
    tip: &str,
) -> EntityCommands<'a> {
    let mut e = c.spawn_scene(plain_scene_button(s, variant));
    e.insert(ChildOf(parent));
    if !tip.is_empty() {
        e.insert(Tip(tip.to_string()));
    }
    e
}

pub fn icon_button<'a>(
    c: &'a mut Commands,
    parent: Entity,
    icon: Icon,
    tip: &str,
    variant: ButtonVariant,
) -> EntityCommands<'a> {
    let mut e = c.spawn_scene(icon_scene_button(icon, tip.to_string(), variant));
    e.insert(ChildOf(parent));
    e
}

pub fn icon_label<'a>(
    c: &'a mut Commands,
    parent: Entity,
    icon: Icon,
    s: String,
    variant: ButtonVariant,
    tip: &str,
) -> EntityCommands<'a> {
    let mut e = c.spawn_scene(crate::icons::icon_label_button(icon, s, variant));
    e.insert(ChildOf(parent));
    if !tip.is_empty() {
        e.insert(Tip(tip.to_string()));
    }
    e
}

pub fn slider<'a>(
    c: &'a mut Commands,
    parent: Entity,
    min: f32,
    max: f32,
    value: f32,
    step: f32,
    width: f32,
) -> EntityCommands<'a> {
    let decimals = (-step.log10()).ceil().max(0.0) as i32;
    // the slider fills a box of the width wanted (its own node keeps its height)
    let holder = c
        .spawn((
            Node {
                width: px(width),
                flex_shrink: 0.0,
                display: Display::Flex,
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    let mut e = c.spawn_scene(bsn! {
        @FeathersSlider {
            @min: {min},
            @max: {max},
        }
    });
    e.insert((
        ChildOf(holder),
        SliderValue(value.clamp(min, max)),
        SliderStep(step),
        SliderPrecision(decimals),
    ));
    e
}

/// a labelled slider row: name at the left, the slider beside it
pub fn slider_row<'a>(
    c: &'a mut Commands,
    parent: Entity,
    name: &str,
    tip: &str,
    fonts: &Fonts,
    (min, max, value, step): (f32, f32, f32, f32),
) -> EntityCommands<'a> {
    let r = row(c, parent, 8.0);
    c.entity(r).insert(Tip(tip.to_string()));
    let l = label(c, r, name, fonts, 13.0, colors::TEXT);
    c.entity(l).insert(Node {
        width: px(110),
        overflow: Overflow::clip(),
        ..default()
    });
    slider(c, r, min, max, value, step, 220.0)
}

/// a track with a filled part, `frac` of it
pub fn meter(c: &mut Commands, parent: Entity, frac: f32, width: f32, height: f32, fill: Color) -> Entity {
    let track = c
        .spawn((
            Node {
                width: px(width),
                height: px(height),
                border_radius: BorderRadius::all(px(height / 2.0)),
                overflow: Overflow::clip(),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(colors::PAD),
            ChildOf(parent),
        ))
        .id();
    c.spawn((
        Node {
            width: percent(frac.clamp(0.0, 1.0) * 100.0),
            height: percent(100),
            ..default()
        },
        BackgroundColor(fill),
        ChildOf(track),
    ));
    track
}

/// a small square of colour
pub fn swatch(c: &mut Commands, parent: Entity, color: Color, size: f32) -> Entity {
    c.spawn((
        Node {
            width: px(size),
            height: px(size),
            border_radius: BorderRadius::all(px(3)),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(color),
        Pickable::IGNORE,
        ChildOf(parent),
    ))
    .id()
}

/// a chip: a coloured mark and a word
pub fn chip(c: &mut Commands, parent: Entity, color: Color, s: &str, fonts: &Fonts) -> Entity {
    let r = c
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(5),
                padding: UiRect::axes(px(8), px(3)),
                border_radius: BorderRadius::all(px(10)),
                ..default()
            },
            BackgroundColor(colors::PAD),
            ChildOf(parent),
        ))
        .id();
    swatch(c, r, color, 10.0);
    label(c, r, s, fonts, 13.0, colors::TEXT);
    r
}

/// the loss as a row of thin bars, newest at the right
pub fn spark(c: &mut Commands, parent: Entity, points: &[f64], width: f32, height: f32) {
    if points.len() < 2 {
        return;
    }
    let n = (width / 4.0) as usize;
    let step = points.len().div_ceil(n).max(1);
    let shown: Vec<f64> = points
        .chunks(step)
        .map(|ch| ch.iter().sum::<f64>() / ch.len() as f64)
        .collect();
    let (lo, hi) = shown
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &x| (a.min(x), b.max(x)));
    let span = (hi - lo).max(1e-9);
    let r = c
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::End,
                column_gap: px(1),
                height: px(height),
                width: px(width),
                ..default()
            },
            Tip(format!(
                "training loss: {:.3} → {:.3}",
                points[0],
                points[points.len() - 1]
            )),
            ChildOf(parent),
        ))
        .id();
    let bar_w = ((width / shown.len() as f32) - 1.0).clamp(1.0, 6.0);
    for v in shown {
        let h = 2.0 + ((v - lo) / span) as f32 * (height - 2.0);
        c.spawn((
            Node {
                width: px(bar_w),
                height: px(h),
                border_radius: BorderRadius::top(px(1)),
                ..default()
            },
            BackgroundColor(colors::ACCENT),
            Pickable::IGNORE,
            ChildOf(r),
        ));
    }
}

/// the confusion matrix, truth down and prediction across
pub fn confusion(c: &mut Commands, parent: Entity, m: &[Vec<u32>], classes: &[String], fonts: &Fonts) {
    if m.is_empty() {
        return;
    }
    let max = m.iter().flatten().copied().max().unwrap_or(1).max(1) as f32;
    let grid = col(c, parent, 2.0);
    dim(c, grid, "truth ↓ · predicted →", fonts);
    let short = |s: &str| -> String {
        let t: String = s.chars().take(9).collect();
        if s.chars().count() > 9 { format!("{}…", t) } else { t }
    };
    let head = row(c, grid, 2.0);
    c.entity(head).insert(Node {
        flex_direction: FlexDirection::Row,
        column_gap: px(2),
        ..default()
    });
    let corner = label(c, head, "", fonts, 11.0, colors::TEXT_DIM);
    c.entity(corner).insert(Node {
        width: px(90),
        ..default()
    });
    for (j, name) in classes.iter().enumerate() {
        let cell = c
            .spawn((
                Node {
                    width: px(58),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Tip(name.clone()),
                ChildOf(head),
            ))
            .id();
        swatch(c, cell, class_color(j, classes.len()), 8.0);
        let t = label(c, cell, &short(name), fonts, 11.0, colors::TEXT_DIM);
        c.entity(t).insert(Node {
            margin: UiRect::left(px(3)),
            ..default()
        });
    }
    for (i, r) in m.iter().enumerate() {
        let line = c
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: px(2),
                    align_items: AlignItems::Center,
                    ..default()
                },
                ChildOf(grid),
            ))
            .id();
        let name = classes.get(i).cloned().unwrap_or_default();
        let t = label(c, line, &short(&name), fonts, 12.0, colors::TEXT);
        c.entity(t).insert((
            Node {
                width: px(90),
                ..default()
            },
            Tip(name.clone()),
        ));
        for (j, &v) in r.iter().enumerate() {
            let k = v as f32 / max;
            let fill = if i == j {
                colors::ACCENT.with_alpha(0.15 + 0.85 * k)
            } else if v > 0 {
                Color::srgb(0.75, 0.35, 0.3).with_alpha(0.15 + 0.7 * k)
            } else {
                colors::PAD.with_alpha(0.5)
            };
            let cell = c
                .spawn((
                    Node {
                        width: px(58),
                        height: px(24),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(px(3)),
                        ..default()
                    },
                    BackgroundColor(fill),
                    Tip(format!(
                        "{} → {}: {}",
                        name,
                        classes.get(j).cloned().unwrap_or_default(),
                        v
                    )),
                    ChildOf(line),
                ))
                .id();
            label(c, cell, &v.to_string(), fonts, 12.0, colors::TEXT);
        }
    }
}

/// a stat: a big number over a small word
pub fn figure(c: &mut Commands, parent: Entity, value: &str, what: &str, fonts: &Fonts) {
    let k = col(c, parent, 0.0);
    c.entity(k).insert(Node {
        flex_direction: FlexDirection::Column,
        min_width: px(84),
        ..default()
    });
    bold(c, k, value, fonts, 19.0, colors::TEXT);
    dim(c, k, what, fonts);
}

/// a bar from the middle: left for negative, right for positive, `v` in -1..1
pub fn shift_bar(c: &mut Commands, parent: Entity, v: f32, half: f32) {
    let r = c
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                width: px(half * 2.0 + 1.0),
                height: px(10),
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    for side in [-1.0f32, 1.0] {
        let h = c
            .spawn((
                Node {
                    width: px(half),
                    height: percent(100),
                    flex_direction: FlexDirection::Row,
                    justify_content: if side < 0.0 {
                        JustifyContent::End
                    } else {
                        JustifyContent::Start
                    },
                    ..default()
                },
                BackgroundColor(colors::PAD),
                ChildOf(r),
            ))
            .id();
        if v * side > 0.0 {
            c.spawn((
                Node {
                    width: px((v.abs() * half).min(half)),
                    height: percent(100),
                    ..default()
                },
                BackgroundColor(if side < 0.0 {
                    Color::srgb(0.70, 0.36, 0.30)
                } else {
                    Color::srgb(0.30, 0.55, 0.42)
                }),
                ChildOf(h),
            ));
        }
        if side < 0.0 {
            c.spawn((
                Node {
                    width: px(1),
                    height: percent(100),
                    ..default()
                },
                BackgroundColor(colors::TEXT_DIM),
                ChildOf(r),
            ));
        }
    }
}

/// a text cut to `n` characters
pub fn clip(s: &str, n: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > n {
        format!("{}…", one.chars().take(n).collect::<String>())
    } else {
        one
    }
}
