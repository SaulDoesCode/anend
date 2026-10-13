//! The colours the settings can change. Each colour the app paints with is a role (the
//! window, the bars, the cards, the text...). Anything spawned with a role's built in
//! colour is tagged with that role when it appears, so the code that builds the interface
//! keeps using the constants in `theme::colors` and never needs to know; when a colour
//! changes, every tagged background, text and border is repainted (keeping its alpha,
//! which zen uses) and the feathers theme is rebuilt from the same palette.

use bevy::feathers::theme::{ThemeBackgroundColor, ThemeBorderColor, ThemedText, UiTheme};
use bevy::prelude::*;
use bevy::text::TextBackgroundColor;
use std::collections::BTreeMap;

use crate::config::Config;
use crate::theme::colors;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Window,
    Bars,
    BarText,
    BarDim,
    Cards,
    Pad,
    Buttons,
    Accent,
    Text,
    Dim,
    Link,
    Code,
    Room,
}

/// what a role paints
#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    Back,
    Ink,
}

impl Role {
    pub const ALL: [Role; 13] = [
        Role::Window,
        Role::Bars,
        Role::BarText,
        Role::BarDim,
        Role::Cards,
        Role::Pad,
        Role::Buttons,
        Role::Accent,
        Role::Text,
        Role::Dim,
        Role::Link,
        Role::Code,
        Role::Room,
    ];

    /// the name in the settings and the config file
    pub fn name(self) -> &'static str {
        match self {
            Role::Window => "window",
            Role::Bars => "bars",
            Role::BarText => "bar text",
            Role::BarDim => "bar labels",
            Role::Cards => "cards",
            Role::Pad => "writing pad",
            Role::Buttons => "buttons",
            Role::Accent => "accent",
            Role::Text => "text",
            Role::Dim => "dim text",
            Role::Link => "links",
            Role::Code => "code",
            Role::Room => "space room",
        }
    }

    pub fn default_color(self) -> Color {
        match self {
            Role::Window => colors::MOOD_DARK,
            Role::Bars => colors::NAV,
            Role::BarText => crate::bar::BAR_TEXT,
            Role::BarDim => crate::bar::BAR_DIM,
            Role::Cards => colors::CARD,
            Role::Pad => colors::PAD,
            Role::Buttons => colors::MOOD,
            Role::Accent => colors::ACCENT,
            Role::Text => colors::TEXT,
            Role::Dim => colors::TEXT_DIM,
            Role::Link => colors::LINK,
            Role::Code => colors::CODE_BG,
            Role::Room => crate::space::WORLD,
        }
    }

    fn paints(self) -> Use {
        match self {
            Role::BarText | Role::BarDim | Role::Text | Role::Dim | Role::Link => Use::Ink,
            _ => Use::Back,
        }
    }
}

/// a few palettes to start from
pub const PRESETS: [(&str, &[(Role, &str)]); 4] = [
    ("sand", &[]),
    (
        "night",
        &[
            (Role::Window, "#1d1b19"),
            (Role::Bars, "#2b2825"),
            (Role::BarText, "#e8e2d8"),
            (Role::BarDim, "#9a9184"),
            (Role::Cards, "#2f2c29"),
            (Role::Pad, "#3a3632"),
            (Role::Buttons, "#6b5d48"),
            (Role::Accent, "#a8875a"),
            (Role::Text, "#e6dfd4"),
            (Role::Dim, "#a39a8d"),
            (Role::Link, "#d9a066"),
            (Role::Code, "#26231f"),
            (Role::Room, "#26221e"),
        ],
    ),
    (
        "paper",
        &[
            (Role::Window, "#e9e6df"),
            (Role::Bars, "#d9d4ca"),
            (Role::BarText, "#2e2b27"),
            (Role::BarDim, "#77706a"),
            (Role::Cards, "#fbfaf7"),
            (Role::Pad, "#f2efe8"),
            (Role::Buttons, "#d6cfc2"),
            (Role::Accent, "#b9a688"),
            (Role::Text, "#2b2926"),
            (Role::Dim, "#7a746c"),
            (Role::Link, "#8a4f22"),
            (Role::Code, "#ece8df"),
            (Role::Room, "#f1ede4"),
        ],
    ),
    (
        "sea",
        &[
            (Role::Window, "#4d6e78"),
            (Role::Bars, "#2c3b40"),
            (Role::Cards, "#f6fafa"),
            (Role::Pad, "#d5e2e4"),
            (Role::Buttons, "#9dbcc2"),
            (Role::Accent, "#6f9ea7"),
            (Role::Link, "#2d6470"),
            (Role::Code, "#e3eced"),
            (Role::Room, "#d8e4e3"),
        ],
    ),
];

pub fn to_hex(c: Color) -> String {
    let s = c.to_srgba();
    format!(
        "#{:02x}{:02x}{:02x}",
        (s.red * 255.0).round() as u8,
        (s.green * 255.0).round() as u8,
        (s.blue * 255.0).round() as u8
    )
}

pub fn from_hex(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    let h = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        h.to_string()
    };
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(&h, 16).ok()?;
    Some(Color::srgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// the colours in use
#[derive(Resource, Clone, PartialEq)]
pub struct Palette(pub [Color; 13]);

impl Default for Palette {
    fn default() -> Self {
        Palette(Role::ALL.map(Role::default_color))
    }
}

impl Palette {
    pub fn from_config(map: &BTreeMap<String, String>) -> Palette {
        Palette(Role::ALL.map(|r| {
            map.get(r.name())
                .and_then(|h| from_hex(h))
                .unwrap_or_else(|| r.default_color())
        }))
    }

    pub fn get(&self, r: Role) -> Color {
        self.0[r as usize]
    }
}

/// a preset as the config's colour map
pub fn preset_map(i: usize) -> BTreeMap<String, String> {
    PRESETS[i]
        .1
        .iter()
        .map(|(r, h)| (r.name().to_string(), h.to_string()))
        .collect()
}

/// which roles an entity's colours came from
#[derive(Component, Default)]
pub struct Painted {
    bg: Option<Role>,
    text: Option<Role>,
    border: Option<Role>,
    span_bg: Option<Role>,
}

/// keeps an entity (and what's under it) out of the palette: the memory view has weum's
/// own colours
#[derive(Component, Default, Clone)]
pub struct OwnColors;

fn same_rgb(a: Color, b: Color) -> bool {
    let (a, b) = (a.to_srgba(), b.to_srgba());
    (a.red - b.red).abs() < 1e-3 && (a.green - b.green).abs() < 1e-3 && (a.blue - b.blue).abs() < 1e-3
}

fn role_of(c: Color, u: Use) -> Option<Role> {
    if c.alpha() == 0.0 {
        return None;
    }
    Role::ALL
        .into_iter()
        .find(|r| r.paints() == u && *r != Role::Room && same_rgb(c, r.default_color()))
}

/// relative luminance, 0 for black to 1 for white
pub fn brightness(c: Color) -> f32 {
    let l = c.to_linear();
    0.2126 * l.red + 0.7152 * l.green + 0.0722 * l.blue
}

fn with_alpha_of(new: Color, old: Color) -> Color {
    new.with_alpha(old.alpha())
}

#[allow(clippy::type_complexity)]
pub fn tag_paints(
    palette: Res<Palette>,
    mut fresh: Query<
        (
            Entity,
            Option<&mut BackgroundColor>,
            Option<&mut TextColor>,
            Option<&mut BorderColor>,
            Option<&mut TextBackgroundColor>,
            Has<ThemeBackgroundColor>,
            Has<ThemedText>,
            Has<ThemeBorderColor>,
        ),
        (
            Or<(
                Added<BackgroundColor>,
                Added<TextColor>,
                Added<BorderColor>,
                Added<TextBackgroundColor>,
            )>,
            Without<Painted>,
        ),
    >,
    parents: Query<&ChildOf>,
    own: Query<(), With<OwnColors>>,
    mut commands: Commands,
) {
    let default = Palette::default();
    let changed = *palette != default;
    for (e, bg, text, border, span_bg, themed_bg, themed_text, themed_border) in &mut fresh {
        let mut p = Painted::default();
        if let Some(bg) = bg.as_ref().filter(|_| !themed_bg) {
            p.bg = role_of(bg.0, Use::Back);
        }
        if let Some(t) = text.as_ref().filter(|_| !themed_text) {
            p.text = role_of(t.0, Use::Ink);
        }
        if let Some(b) = border.as_ref().filter(|_| !themed_border) {
            p.border = role_of(b.top, Use::Back);
        }
        if let Some(b) = span_bg.as_ref() {
            p.span_bg = role_of(b.0, Use::Back);
        }
        if p.bg.is_none() && p.text.is_none() && p.border.is_none() && p.span_bg.is_none() {
            continue;
        }
        if own.contains(e) || parents.iter_ancestors(e).any(|a| own.contains(a)) {
            continue;
        }
        if changed {
            paint(&p, &palette, bg, text, border, span_bg);
        }
        commands.entity(e).try_insert(p);
    }
}

fn paint(
    p: &Painted,
    palette: &Palette,
    bg: Option<Mut<BackgroundColor>>,
    text: Option<Mut<TextColor>>,
    border: Option<Mut<BorderColor>>,
    span_bg: Option<Mut<TextBackgroundColor>>,
) {
    if let (Some(r), Some(mut b)) = (p.span_bg, span_bg) {
        let c = with_alpha_of(palette.get(r), b.0);
        if b.0 != c {
            b.0 = c;
        }
    }
    if let (Some(r), Some(mut bg)) = (p.bg, bg) {
        let c = with_alpha_of(palette.get(r), bg.0);
        if bg.0 != c {
            bg.0 = c;
        }
    }
    if let (Some(r), Some(mut t)) = (p.text, text) {
        let c = with_alpha_of(palette.get(r), t.0);
        if t.0 != c {
            t.0 = c;
        }
    }
    if let (Some(r), Some(mut b)) = (p.border, border) {
        let c = palette.get(r);
        b.top = with_alpha_of(c, b.top);
        b.right = with_alpha_of(c, b.right);
        b.bottom = with_alpha_of(c, b.bottom);
        b.left = with_alpha_of(c, b.left);
    }
}

/// follows the config's colours: repaints what's tagged, the theme and the clear colours
#[allow(clippy::type_complexity)]
pub fn apply_palette(
    config: Res<Config>,
    mut palette: ResMut<Palette>,
    mut theme: ResMut<UiTheme>,
    mut clear: ResMut<ClearColor>,
    mut cams: Query<&mut Camera, With<crate::space::SpaceCam>>,
    mut painted: Query<(
        &Painted,
        Option<&mut BackgroundColor>,
        Option<&mut TextColor>,
        Option<&mut BorderColor>,
        Option<&mut TextBackgroundColor>,
    )>,
) {
    if !config.is_changed() {
        return;
    }
    let next = Palette::from_config(&config.colors);
    if *palette == next && !config.is_added() {
        return;
    }
    *palette = next;
    theme.0 = crate::theme::sand_theme(&palette);
    clear.0 = palette.get(Role::Window);
    for mut cam in &mut cams {
        cam.clear_color = ClearColorConfig::Custom(palette.get(Role::Room));
    }
    for (p, bg, text, border, span_bg) in &mut painted {
        paint(p, &palette, bg, text, border, span_bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        for r in Role::ALL {
            let c = r.default_color();
            let back = from_hex(&to_hex(c)).unwrap();
            assert!(same_rgb(c, back) || (to_hex(c) == to_hex(back)), "{:?}", r);
        }
        assert_eq!(to_hex(from_hex("#abc").unwrap()), "#aabbcc");
        assert!(from_hex("#12345").is_none());
        assert!(from_hex("zzzzzz").is_none());
    }

    #[test]
    fn roles_are_told_apart() {
        for r in Role::ALL {
            if r == Role::Room {
                continue;
            }
            assert_eq!(role_of(r.default_color(), r.paints()), Some(r));
        }
        let mut m = BTreeMap::new();
        m.insert("cards".to_string(), "#000000".to_string());
        m.insert("nonsense".to_string(), "#ffffff".to_string());
        let p = Palette::from_config(&m);
        assert_eq!(to_hex(p.get(Role::Cards)), "#000000");
        assert_eq!(p.get(Role::Text), colors::TEXT);
    }
}
