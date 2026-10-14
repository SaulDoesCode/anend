//! The colours: every one has a role (as in the web and Bevy apps) and a CSS variable.
//! Sand is the old admin panel's; night, paper and sea start from it.

use std::collections::BTreeMap;

pub const ROLES: [(&str, &str); 13] = [
    ("window", "--window"),
    ("bars", "--bars"),
    ("bar text", "--bar-text"),
    ("bar labels", "--bar-dim"),
    ("cards", "--cards"),
    ("writing pad", "--pad"),
    ("buttons", "--buttons"),
    ("accent", "--accent"),
    ("text", "--text"),
    ("dim text", "--dim"),
    ("links", "--link"),
    ("code", "--code"),
    ("space room", "--room"),
];

const SAND: [&str; 13] = [
    "#927a4f", "#404040", "#ebebeb", "#9e9e9e", "#ffffff", "#dbd4c8", "#c4b28f", "#c4a265", "#2e2e2e", "#6b6b6b",
    "#86592a", "#e9e5de", "#e4d4be",
];
const NIGHT: [&str; 13] = [
    "#1d1b19", "#2b2825", "#e8e2d8", "#9a9184", "#2f2c29", "#3a3632", "#6b5d48", "#a8875a", "#e6dfd4", "#a39a8d",
    "#d9a066", "#26231f", "#26221e",
];
const PAPER: [&str; 13] = [
    "#e9e6df", "#d9d4ca", "#2e2b27", "#77706a", "#fbfaf7", "#f2efe8", "#d6cfc2", "#b9a688", "#2b2926", "#7a746c",
    "#8a4f22", "#ece8df", "#f1ede4",
];
const SEA: [&str; 13] = [
    "#4d6e78", "#2c3b40", "#ebebeb", "#9e9e9e", "#f6fafa", "#d5e2e4", "#9dbcc2", "#6f9ea7", "#2e2e2e", "#6b6b6b",
    "#2d6470", "#e3eced", "#d8e4e3",
];

pub const PALETTES: [&str; 4] = ["sand", "night", "paper", "sea"];

pub fn palette(name: &str) -> &'static [&'static str; 13] {
    match name {
        "night" => &NIGHT,
        "paper" => &PAPER,
        "sea" => &SEA,
        _ => &SAND,
    }
}

/// a role's colour: the custom one, else the palette's
pub fn colour(name: &str, own: &BTreeMap<String, String>, role: &str) -> String {
    if let Some(c) = own.get(role) {
        return c.clone();
    }
    ROLES
        .iter()
        .position(|(r, _)| *r == role)
        .map(|i| palette(name)[i].to_string())
        .unwrap_or_default()
}

pub fn rgb(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    let n = u32::from_str_radix(h.get(..6).unwrap_or("000000"), 16).unwrap_or(0);
    (((n >> 16) & 255) as f64, ((n >> 8) & 255) as f64, (n & 255) as f64)
}

/// whether dark ink reads on it
pub fn light(hex: &str) -> bool {
    let (r, g, b) = rgb(hex);
    0.2126 * r + 0.7152 * g + 0.0722 * b > 140.0
}

/// the CSS variables for a palette and its custom colours
pub fn css_vars(name: &str, own: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    for (role, var) in ROLES {
        out.push_str(&format!("{var}: {};", colour(name, own, role)));
    }
    let buttons = colour(name, own, "buttons");
    out.push_str(&format!(
        "--on-button: {};",
        if light(&buttons) { "#1f1f1f" } else { "#f1ebe2" }
    ));
    let cards = colour(name, own, "cards");
    out.push_str(&format!(
        "color-scheme: {};",
        if light(&cards) { "light" } else { "dark" }
    ));
    out
}

/// readable ink for texts on a background (the space's room): dark on light, light on dark
pub fn ink_on(bg: &str) -> &'static str {
    if light(bg) { "hsl(0 0% 12%)" } else { "hsl(35 30% 92%)" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes() {
        let own = BTreeMap::from([("accent".to_string(), "#ff0000".to_string())]);
        assert_eq!(colour("night", &own, "accent"), "#ff0000");
        assert_eq!(colour("night", &own, "window"), "#1d1b19");
        assert!(css_vars("sand", &BTreeMap::new()).contains("--window: #927a4f"));
        assert!(light("#ffffff") && !light("#1d1b19"));
        assert_eq!(ink_on("#000000"), "hsl(35 30% 92%)");
    }
}
