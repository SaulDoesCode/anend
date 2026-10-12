//! The "sand" look of the old anend admin panel, as a feathers theme.
//!
//! From admin.css: the page sits on `--primary-dark` (hsl 39 30% 44%), the nav bar is a
//! dark grey strip with "mood" coloured (hsl 39 30% 66%) buttons carrying near black
//! text, the editor is a white card and the writing pad is a light sand (hsl 39 19% 82%).

use bevy::color::{Alpha, Color, Luminance};
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

use colors::*;

pub fn sand_theme() -> ThemeProps {
    let mut theme = create_dark_theme();
    // the dark theme's per surface corrections don't apply to a light palette
    theme.semantic_overrides.clear();
    let near_black = Color::hsl(0.0, 0.0, 0.06);
    for (token, color) in [
        (semantic::SURFACE_WINDOW, MOOD_DARK),
        (semantic::SURFACE_PANE_BODY, CARD),
        (semantic::SURFACE_PANE_HEADER, NAV),
        (semantic::SURFACE_SUBPANE_BODY, PAD),
        (semantic::SURFACE_SUBPANE_HEADER, MOOD),
        (semantic::SURFACE_GROUP, PAD),
        (semantic::SURFACE_DIALOG, CARD),
        (semantic::SURFACE_MENU, CARD),
        (semantic::BORDER_DEFAULT, MOOD),
        (semantic::TEXT_DEFAULT, TEXT),
        (semantic::TEXT_DIM, TEXT_DIM),
        (semantic::TEXT_DISABLED, TEXT.with_alpha(0.4)),
        (semantic::TEXT_ON_ACCENT, near_black),
        (semantic::FILL_ACCENT_DEFAULT, ACCENT),
        (semantic::FILL_ACCENT_HOVER, ACCENT.lighter(0.05)),
        (semantic::FILL_ACCENT_PRESSED, ACCENT.darker(0.06)),
        (semantic::FILL_ACCENT_DISABLED, ACCENT.with_alpha(0.5)),
        (semantic::FILL_SOLID_DEFAULT, MOOD),
        (semantic::FILL_SOLID_HOVER, MOOD.lighter(0.05)),
        (semantic::FILL_SOLID_PRESSED, MOOD.darker(0.06)),
        (semantic::FILL_SOLID_DISABLED, MOOD.with_alpha(0.5)),
        (semantic::FILL_ITEM_DEFAULT, Color::NONE),
        (semantic::FILL_ITEM_HOVER, PAD),
        (semantic::FILL_ITEM_PRESSED, MOOD),
        (semantic::FILL_FIELD_DEFAULT, PAD),
        (semantic::FILL_FIELD_HOVER, PAD.lighter(0.03)),
        (semantic::FILL_FIELD_PRESSED, PAD.darker(0.03)),
        (semantic::FOCUS_RING, MOOD_DARK.with_alpha(0.7)),
    ] {
        theme.semantic_base.insert(token, color);
    }
    theme
}
