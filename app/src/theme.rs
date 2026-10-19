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

fn hsl_of(hex: &str) -> (f64, f64, f64) {
    let (r, g, b) = rgb(hex);
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hex_of_hsl(h: f64, s: f64, l: f64) -> String {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f64| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", to(r), to(g), to(b))
}

/// relative luminance (WCAG)
fn luminance(hex: &str) -> f64 {
    let (r, g, b) = rgb(hex);
    let lin = |v: f64| {
        let v = v / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `c`, darkened or lightened (away from `bg`) until it stands out from it by `ratio`
pub fn readable(c: &str, bg: &str, ratio: f64) -> String {
    let darker = luminance(bg) > 0.18;
    let (h, s, mut l) = hsl_of(c);
    let mut out = hex_of_hsl(h, s, l);
    for _ in 0..40 {
        if contrast(&out, bg) >= ratio {
            break;
        }
        l = if darker {
            (l - 0.025).max(0.0)
        } else {
            (l + 0.025).min(1.0)
        };
        out = hex_of_hsl(h, s, l);
    }
    out
}

fn mix(a: &str, b: &str, t: f64) -> String {
    let (a, b) = (rgb(a), rgb(b));
    let m = |x: f64, y: f64| (x + (y - x) * t).round() as u8;
    format!("#{:02x}{:02x}{:02x}", m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// the editor's highlighting colours (highlight.rs's classes), as the Bevy app made them:
/// from the palette, each readable on the writing pad, the hues the palette doesn't name
/// turned from its link colour so they suit any preset
pub fn hl_vars(name: &str, own: &BTreeMap<String, String>) -> String {
    let pad = colour(name, own, "writing pad");
    let text = colour(name, own, "text");
    let dim = colour(name, own, "dim text");
    let link = colour(name, own, "links");
    let accent = colour(name, own, "accent");
    let (lh, ls, ll) = hsl_of(&link);
    let turn = |deg: f64, sat: f64| hex_of_hsl(lh + deg, (ls * sat).clamp(0.25, 0.75), ll);
    let heading = readable(&link, &pad, 5.0);
    let mark = readable(&mix(&dim, &pad, 0.2), &pad, 2.2);
    let vars = [
        ("--hl-mark", mark.clone()),
        ("--hl-heading", heading.clone()),
        ("--hl-emph", readable(&mix(&text, &heading, 0.4), &pad, 5.0)),
        ("--hl-strike", readable(&dim, &pad, 2.6)),
        ("--hl-code", readable(&turn(140.0, 0.8), &pad, 4.2)),
        ("--hl-link", readable(&turn(200.0, 1.0), &pad, 4.5)),
        ("--hl-url", readable(&turn(190.0, 0.6), &pad, 3.0)),
        ("--hl-list", readable(&accent, &pad, 3.0)),
        ("--hl-quote", readable(&dim, &pad, 3.4)),
        ("--hl-tag", readable(&turn(-40.0, 1.0), &pad, 4.0)),
        ("--hl-rule", mark),
    ];
    vars.iter().map(|(k, v)| format!("{k}: {v};")).collect()
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
        // the sand window is hsl(39 30% 44%), the Bevy app's MOOD_DARK
        assert_eq!(hex_of_hsl(39.0, 0.30, 0.44), "#927a4f");
        let (h, s, l) = hsl_of("#927a4f");
        assert!((h - 39.0).abs() < 1.0 && (s - 0.30).abs() < 0.01 && (l - 0.44).abs() < 0.01);
        // every highlighting colour reads on the pad
        for p in PALETTES {
            let pad = colour(p, &BTreeMap::new(), "writing pad");
            for part in hl_vars(p, &BTreeMap::new()).split(';').filter(|x| !x.is_empty()) {
                let c = part.split(": ").nth(1).unwrap();
                assert!(contrast(c, &pad) >= 2.2, "{p} {part}");
            }
        }
    }
}
