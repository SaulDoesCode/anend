//! The sizes the settings pick: the whole interface's scale, and the heights of the top
//! bar and the command bar, the way a desktop panel is so many pixels high. The text in
//! a bar is sized to fit it, and its buttons grow and shrink with it.

use bevy::feathers::font_styles::InheritableFont;
use bevy::prelude::*;

use crate::bar::BarRow;
use crate::config::Config;
use crate::ui::NavBar;

/// the bar heights everything was laid out for
pub const TOP: f32 = 32.0;
pub const BOTTOM: f32 = 30.0;

/// a text's size before the bars changed it
#[derive(Component)]
pub struct BaseFont(f32);

/// the height of the top bar's buttons
fn button_height(top: f32) -> f32 {
    (top - 8.0).max(14.0)
}

#[allow(clippy::type_complexity)]
pub fn apply_sizes(
    config: Res<Config>,
    mut scale: ResMut<UiScale>,
    parents: Query<&ChildOf>,
    nav: Single<(Entity, &mut Node), (With<NavBar>, Without<BarRow>)>,
    bar: Single<(Entity, &mut Node), (With<BarRow>, Without<NavBar>)>,
    children: Query<&Children>,
    mut nodes: Query<&mut Node, (With<bevy::ui_widgets::Button>, Without<NavBar>, Without<BarRow>)>,
    inheritable: Query<&InheritableFont>,
    mut texts: ParamSet<(
        Query<Entity, Added<TextFont>>,
        Query<(&mut TextFont, Option<&BaseFont>)>,
    )>,
    mut applied: Local<Option<[f32; 3]>>,
    mut commands: Commands,
) {
    let (nav, mut nav_node) = nav.into_inner();
    let (bar, mut bar_node) = bar.into_inner();
    let new_text = texts
        .p0()
        .iter()
        .any(|e| parents.iter_ancestors(e).any(|a| a == nav || a == bar));
    let want = [config.ui_scale, config.top_bar, config.bottom_bar];
    if *applied == Some(want) && !new_text {
        return;
    }
    *applied = Some(want);
    let s = config.ui_scale.clamp(0.5, 3.0);
    if (scale.0 - s).abs() > 1e-4 {
        scale.0 = s;
    }
    let top = config.top_bar.clamp(18.0, 96.0);
    let bottom = config.bottom_bar.clamp(18.0, 96.0);
    let btn = button_height(top);
    let pad = ((top - btn) / 2.0).max(0.0);
    let height = px(top);
    if nav_node.height != height {
        nav_node.height = height;
        nav_node.padding = UiRect::axes(px(4), px(pad));
    }
    let min = px(bottom);
    if bar_node.min_height != min {
        bar_node.min_height = min;
    }

    // text grows slower than its bar, so a tall bar gets breathing room, like a panel's
    let fit = |r: f32| r.powf(0.6);
    for (root, k, buttons) in [
        (nav, fit(btn / button_height(TOP)), true),
        (bar, fit(bottom / BOTTOM), false),
    ] {
        let mut stack = vec![root];
        while let Some(e) = stack.pop() {
            if let Ok(c) = children.get(e) {
                stack.extend(c.iter());
            }
            if buttons && let Ok(mut n) = nodes.get_mut(e) {
                let h = px(btn);
                if n.height != h {
                    n.height = h;
                }
                if let Ok(f) = inheritable.get(e) {
                    let size = FontSize::Px((14.0 * k).round());
                    if f.font_size != size {
                        commands.entity(e).insert(InheritableFont {
                            font: f.font.clone(),
                            font_size: size,
                            weight: f.weight,
                        });
                    }
                }
            }
            if let Ok((mut font, base)) = texts.p1().get_mut(e) {
                let base = match base {
                    Some(b) => b.0,
                    None => {
                        let FontSize::Px(b) = font.font_size else {
                            continue;
                        };
                        commands.entity(e).insert(BaseFont(b));
                        b
                    }
                };
                let size = FontSize::Px((base * k).round().max(8.0));
                if font.font_size != size {
                    font.font_size = size;
                }
            }
        }
    }
}
