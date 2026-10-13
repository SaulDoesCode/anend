//! The "sand" look of the old anend admin panel, as a feathers theme.
//!
//! From admin.css: the page sits on `--primary-dark` (hsl 39 30% 44%), the nav bar is a
//! dark grey strip with "mood" coloured (hsl 39 30% 66%) buttons carrying near black
//! text, the editor is a white card and the writing pad is a light sand (hsl 39 19% 82%).

use bevy::color::{Alpha, Color, Luminance, Mix};
use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::ThemeProps;
use bevy::feathers::controls::ButtonVariant;
use bevy::feathers::tokens::semantic;

pub mod colors {
    use bevy::color::Color;

    /// --the-mood / --primary
    pub const MOOD: Color = Color::hsl(39.0, 0.30, 0.66);
    /// --primary-dark, the window background
    pub const MOOD_DARK: Color = Color::hsl(39.0, 0.30, 0.44);
    /// the writing pad
    pub const PAD: Color = Color::hsl(39.0, 0.19, 0.82);
    /// the nav bar
    pub const NAV: Color = Color::hsl(0.0, 0.0, 0.25);
    pub const CARD: Color = Color::WHITE;
    /// body text, hsl(0, 0%, 18%)
    pub const TEXT: Color = Color::hsl(0.0, 0.0, 0.18);
    pub const TEXT_DIM: Color = Color::hsl(0.0, 0.0, 0.42);
    /// the stronger sand used for selected things
    pub const ACCENT: Color = Color::hsl(39.0, 0.42, 0.56);
    pub const CODE_BG: Color = Color::hsl(39.0, 0.19, 0.90);
    pub const LINK: Color = Color::hsl(30.0, 0.55, 0.34);
}

use crate::palette::{Palette, Role};

/// the feathers theme, from the palette the settings pick
pub fn sand_theme(p: &Palette) -> ThemeProps {
    let mut theme = create_dark_theme();
    // the dark theme's per surface corrections don't apply to a light palette
    theme.semantic_overrides.clear();
    let near_black = Color::hsl(0.0, 0.0, 0.06);
    let (mood_dark, card, nav, pad, mood, text, text_dim, accent) = (
        p.get(Role::Window),
        p.get(Role::Cards),
        p.get(Role::Bars),
        p.get(Role::Pad),
        p.get(Role::Buttons),
        p.get(Role::Text),
        p.get(Role::Dim),
        p.get(Role::Accent),
    );
    // text on a button: dark or light, whichever stands out more from it
    let light = Color::hsl(0.0, 0.0, 0.94);
    let on = |c: Color| {
        let ratio = |a: Color, b: Color| {
            let (la, lb) = (crate::palette::brightness(a), crate::palette::brightness(b));
            (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
        };
        if ratio(near_black, c) >= ratio(light, c) { near_black } else { light }
    };
    // feathers puts every button's text in this one colour, plain and accent buttons alike
    let on_accent = on(mood.mix(&accent, 0.5));
    for (token, color) in [
        (semantic::SURFACE_WINDOW, mood_dark),
        (semantic::SURFACE_PANE_BODY, card),
        (semantic::SURFACE_PANE_HEADER, nav),
        (semantic::SURFACE_SUBPANE_BODY, pad),
        (semantic::SURFACE_SUBPANE_HEADER, mood),
        (semantic::SURFACE_GROUP, pad),
        (semantic::SURFACE_DIALOG, card),
        (semantic::SURFACE_MENU, card),
        (semantic::BORDER_DEFAULT, mood),
        (semantic::TEXT_DEFAULT, text),
        (semantic::TEXT_DIM, text_dim),
        (semantic::TEXT_DISABLED, text.with_alpha(0.4)),
        (semantic::TEXT_ON_ACCENT, on_accent),
        (semantic::FILL_ACCENT_DEFAULT, accent),
        (semantic::FILL_ACCENT_HOVER, accent.lighter(0.05)),
        (semantic::FILL_ACCENT_PRESSED, accent.darker(0.06)),
        (semantic::FILL_ACCENT_DISABLED, accent.with_alpha(0.5)),
        (semantic::FILL_SOLID_DEFAULT, mood),
        (semantic::FILL_SOLID_HOVER, mood.lighter(0.05)),
        (semantic::FILL_SOLID_PRESSED, mood.darker(0.06)),
        (semantic::FILL_SOLID_DISABLED, mood.with_alpha(0.5)),
        (semantic::FILL_ITEM_DEFAULT, Color::NONE),
        (semantic::FILL_ITEM_HOVER, pad),
        (semantic::FILL_ITEM_PRESSED, mood),
        (semantic::FILL_FIELD_DEFAULT, pad),
        (semantic::FILL_FIELD_HOVER, pad.lighter(0.03)),
        (semantic::FILL_FIELD_PRESSED, pad.darker(0.03)),
        (semantic::FOCUS_RING, mood_dark.with_alpha(0.7)),
    ] {
        theme.semantic_base.insert(token, color);
    }
    theme
}

/// Feathers gives every button the same text colour, the one that reads on a filled
/// button. A plain button has no fill until it's hovered, so on a dark card its text
/// would vanish: its text takes the card's text colour instead.
#[allow(clippy::type_complexity)]
pub fn plain_button_text(
    palette: bevy::prelude::Res<Palette>,
    all: bevy::prelude::Query<(bevy::prelude::Entity, &ButtonVariant, bevy::prelude::Has<bevy::ui::InteractionDisabled>)>,
    changed: bevy::prelude::Query<
        bevy::prelude::Entity,
        bevy::prelude::Or<(
            bevy::prelude::Changed<bevy::feathers::theme::InheritableThemeTextColor>,
            bevy::prelude::Changed<ButtonVariant>,
        )>,
    >,
    mut commands: bevy::prelude::Commands,
) {
    use bevy::prelude::DetectChanges;
    let text = p_text(&palette);
    let mut fix = |e, v: &ButtonVariant, disabled: bool| {
        if *v == ButtonVariant::Plain && !disabled {
            commands
                .entity(e)
                .try_insert(bevy::app::Propagate(bevy::prelude::TextColor(text)));
        }
    };
    if palette.is_changed() {
        for (e, v, d) in &all {
            fix(e, v, d);
        }
    } else {
        for e in &changed {
            if let Ok((e, v, d)) = all.get(e) {
                fix(e, v, d);
            }
        }
    }
}

fn p_text(p: &Palette) -> Color {
    p.get(Role::Text)
}
