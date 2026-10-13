//! Zen mode and the hiding top bar, both off unless the settings turn them on.
//!
//! Zen: while the mouse rests (and no menu is open, and you're not typing in the command
//! bar) the chrome fades away: the top bar, the command bar, the note list and the
//! editor's toolbar, the space's panels, the memory view's hover line. Writing in the
//! editor doesn't bring it back; moving the mouse does. Bevy's UI has no opacity for a
//! whole subtree, so the fade scales the alpha of every background and text under each
//! piece of chrome, remembering what each one had.
//!
//! The top bar: slides up out of the window, and back down when the pointer comes to the
//! top edge.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::config::Config;
use crate::ui::{NavBar, Status};

/// a piece of the interface that zen fades
#[derive(Component, Default, Clone)]
pub struct Chrome;

/// the alphas an entity had before zen faded it
#[derive(Component)]
pub struct ZenBase {
    bg: Option<f32>,
    text: Option<f32>,
    border: Option<[f32; 4]>,
}

#[derive(Resource)]
pub struct Zen {
    idle: f32,
    /// how much of the chrome shows, 0..1
    pub fade: f32,
    faded: bool,
    last_cursor: Option<Vec2>,
    /// how far the top bar is down, 0..1
    slide: f32,
    /// how far the note list is out, 0..1
    list: f32,
}

impl Default for Zen {
    fn default() -> Self {
        Zen {
            idle: 0.0,
            fade: 1.0,
            faded: false,
            last_cursor: None,
            slide: 1.0,
            list: 1.0,
        }
    }
}

pub fn zen_fade(
    mut zen: ResMut<Zen>,
    config: Res<Config>,
    time: Res<Time>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    settings: Res<crate::settings::Settings>,
    remap: Res<crate::keys::Remap>,
    space: Res<crate::space::Space>,
    focus: Res<InputFocus>,
    bar: Single<Entity, With<crate::bar::Bar>>,
    roots: Query<Entity, With<Chrome>>,
    children: Query<&Children>,
    mut paints: Query<(
        Option<&mut BackgroundColor>,
        Option<&mut TextColor>,
        Option<&mut BorderColor>,
        Option<&ZenBase>,
    )>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let cursor = window.cursor_position();
    // looking around in the space moves the mouse too, but isn't reaching for the interface
    let moved = (!space.looking && motion.delta != Vec2::ZERO)
        || (!space.looking && cursor != zen.last_cursor && cursor.is_some())
        || scroll.delta != Vec2::ZERO
        || mouse.get_pressed().next().is_some();
    zen.last_cursor = cursor;
    if moved {
        zen.idle = 0.0;
    } else {
        zen.idle += dt;
    }
    let hold = !config.zen || settings.open || remap.open || focus.get() == Some(*bar);
    let target = if hold || zen.idle < config.zen_delay { 1.0 } else { 0.0 };
    let rate = if target > zen.fade { 1.0 / 0.15 } else { 1.0 / 0.8 };
    zen.fade = if target > zen.fade {
        (zen.fade + rate * dt).min(target)
    } else {
        (zen.fade - rate * dt).max(target)
    };

    if zen.fade >= 1.0 && !zen.faded {
        return;
    }
    let done = zen.fade >= 1.0;
    zen.faded = !done;
    let k = zen.fade;
    let mut stack: Vec<Entity> = roots.iter().collect();
    while let Some(e) = stack.pop() {
        if let Ok(c) = children.get(e) {
            stack.extend(c.iter());
        }
        let Ok((bg, text, border, base)) = paints.get_mut(e) else {
            continue;
        };
        let base = match base {
            Some(b) => (b.bg, b.text, b.border),
            None => {
                let b = (
                    bg.as_ref().map(|c| c.0.alpha()),
                    text.as_ref().map(|c| c.0.alpha()),
                    border
                        .as_ref()
                        .map(|c| [c.top.alpha(), c.right.alpha(), c.bottom.alpha(), c.left.alpha()]),
                );
                if !done {
                    commands.entity(e).insert(ZenBase {
                        bg: b.0,
                        text: b.1,
                        border: b.2,
                    });
                }
                b
            }
        };
        if let (Some(mut border), Some(a)) = (border, base.2)
            && (border.top.alpha() - a[0] * k).abs() > 1e-4
        {
            border.top.set_alpha(a[0] * k);
            border.right.set_alpha(a[1] * k);
            border.bottom.set_alpha(a[2] * k);
            border.left.set_alpha(a[3] * k);
        }
        if let (Some(mut bg), Some(a)) = (bg, base.0) {
            let want = a * k;
            if (bg.0.alpha() - want).abs() > 1e-4 {
                bg.0.set_alpha(want);
            }
        }
        if let (Some(mut text), Some(a)) = (text, base.1) {
            let want = a * k;
            if (text.0.alpha() - want).abs() > 1e-4 {
                text.0.set_alpha(want);
            }
        }
        if done {
            commands.entity(e).try_remove::<ZenBase>();
        }
    }
}

/// slides the top bar away and back
pub fn nav_autohide(
    mut zen: ResMut<Zen>,
    config: Res<Config>,
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    nav: Single<(&mut Node, &ComputedNode), With<NavBar>>,
    settings: Res<crate::settings::Settings>,
    remap: Res<crate::keys::Remap>,
    status: Res<Status>,
) {
    let (mut node, computed) = nav.into_inner();
    let h = computed.size().y * computed.inverse_scale_factor() + 6.0;
    // the bar's edge: the top, or the bottom when the bars are swapped
    let height = window.height();
    let from_edge = |y: f32| if config.bars_swapped { height - y } else { y };
    let near = window
        .cursor_position()
        .is_some_and(|c| from_edge(c.y) < 10.0 || (zen.slide > 0.5 && from_edge(c.y) < h + 6.0));
    let show = !config.hide_bar || near || settings.open || remap.open || !status.text.is_empty();
    let target = if show { 1.0 } else { 0.0 };
    let dt = time.delta_secs();
    let slide = zen.slide + (target - zen.slide) * (1.0 - (-dt * 14.0).exp());
    zen.slide = if (slide - target).abs() < 0.002 { target } else { slide };
    let off = px(-h * (1.0 - zen.slide));
    let (top, bottom) = if config.bars_swapped { (px(0), off) } else { (off, px(0)) };
    if node.margin.top != top || node.margin.bottom != bottom {
        node.margin.top = top;
        node.margin.bottom = bottom;
    }
}

/// slides the note list away to the left, and back while the pointer is at the left edge
#[allow(clippy::type_complexity)]
pub fn list_autohide(
    mut zen: ResMut<Zen>,
    config: Res<Config>,
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    list: Single<(&mut Node, &ComputedNode), With<crate::ui::NoteListPanel>>,
    toggle: Single<&Children, With<crate::ui::ListToggle>>,
    mut texts: Query<&mut Text>,
    settings: Res<crate::settings::Settings>,
) {
    let (mut node, computed) = list.into_inner();
    let w = computed.size().x * computed.inverse_scale_factor() + 8.0;
    // the list's edge of the window
    let width = window.width();
    let from_edge = |x: f32| if config.list_right { width - x } else { x };
    let near = window
        .cursor_position()
        .is_some_and(|c| from_edge(c.x) < 10.0 || (zen.list > 0.5 && from_edge(c.x) < w + 16.0));
    let show = !config.hide_list || (near && !settings.open);
    let target = if show { 1.0 } else { 0.0 };
    let dt = time.delta_secs();
    let slide = zen.list + (target - zen.list) * (1.0 - (-dt * 14.0).exp());
    zen.list = if (slide - target).abs() < 0.002 { target } else { slide };
    let off = px(-w * (1.0 - zen.list));
    let (left, right) = if config.list_right { (px(0), off) } else { (off, px(0)) };
    if node.margin.left != left || node.margin.right != right {
        node.margin.left = left;
        node.margin.right = right;
    }
    // the arrow points the way the list goes
    let glyph = match (config.hide_list, config.list_right) {
        (false, false) | (true, true) => "‹",
        _ => "›",
    };
    for c in toggle.iter() {
        if let Ok(mut t) = texts.get_mut(c)
            && t.0 != glyph
        {
            t.0 = glyph.into();
        }
    }
}

/// where things sit: the bars swapped or not, the note list left or right
pub fn apply_placement(
    config: Res<Config>,
    mut root: Single<&mut Node, (With<crate::ui::AppRoot>, Without<crate::ui::EditorView>)>,
    mut editor: Single<&mut Node, (With<crate::ui::EditorView>, Without<crate::ui::AppRoot>)>,
) {
    let column = if config.bars_swapped {
        FlexDirection::ColumnReverse
    } else {
        FlexDirection::Column
    };
    if root.flex_direction != column {
        root.flex_direction = column;
    }
    let row = if config.list_right { FlexDirection::RowReverse } else { FlexDirection::Row };
    if editor.flex_direction != row {
        editor.flex_direction = row;
    }
}
