//! The "sand" look of the old anend admin panel, as a feathers theme.
//!
//! From admin.css: the page sits on `--primary-dark` (hsl 39 30% 44%), the nav bar is a
//! dark grey strip with "mood" coloured (hsl 39 30% 66%) buttons carrying near black
//! text, the editor is a white card and the writing pad is a light sand (hsl 39 19% 82%).

use bevy::color::{Alpha, Color, Luminance, Mix};
use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::ThemeProps;
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
    // text on a button: dark on a light one, light on a dark one
    let on = |c: Color| {
        if crate::palette::brightness(c) > 0.2 {
            near_black
        } else {
            Color::hsl(0.0, 0.0, 0.94)
        }
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
