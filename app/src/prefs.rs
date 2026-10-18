//! The app's own settings (this machine's, not the save's): colours, font size, zoom,
//! spacing, the memory view's toggles, which sections are collapsed. The desktop keeps
//! them in `app.json` beside the Bevy app's config (`ANEND_APP_CONFIG` overrides the
//! path); the web app in localStorage.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::gem::Mode;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// sand, night, paper or sea
    pub palette: String,
    /// colours changed from the palette's, by role
    pub colours: BTreeMap<String, String>,
    /// the interface's text size in pixels
    pub font_size: f32,
    /// the whole page's zoom (Ctrl+= / Ctrl+−, Ctrl+PageUp / PageDown)
    pub zoom: f32,
    /// the memory view's text size in pixels
    pub memory_font: f32,
    /// the gap between texts in the memory view, in pixels
    pub spacing: f32,
    /// the memory view's centre field's width, in pixels (0: the whole width)
    pub field_width: f32,
    pub mode: Mode,
    /// Enter on a text gsearches its sum (weum's gsch), not its words
    pub gsearch_text: bool,
    /// remember what's searched (weum's alsoSaturate)
    pub saturate: bool,
    /// the sidecar's frequency letters
    pub letters: bool,
    /// the classifier colouring the memory view's texts (empty: none)
    pub tint: String,
    /// Enter in the memory view searches with that classifier: a class's name finds the
    /// texts most likely in it, any other text the texts it reads most like
    pub cls_search: bool,
    /// the space's background colour
    pub space_bg: String,
    /// the space buckets by the reversed sum
    pub space_reversed: bool,
    /// the space adds new results to what's there
    pub space_keep: bool,
    /// fade the bars away while the mouse and keys rest
    pub zen: bool,
    /// seconds of stillness before zen fades
    pub zen_delay: f32,
    /// slide the top bar away until the pointer comes near it
    pub hide_bar: bool,
    /// sections folded shut, by name (settings, lists…)
    pub collapsed: BTreeSet<String>,
    pub view: String,
    /// the save the desktop opens (a path)
    pub save: Option<String>,
    /// the hub (when built with it)
    pub hub_url: String,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            palette: "sand".into(),
            colours: BTreeMap::new(),
            font_size: 14.0,
            zoom: 1.0,
            memory_font: 20.0,
            spacing: 4.0,
            field_width: 0.0,
            mode: Mode::Ordinal,
            gsearch_text: true,
            saturate: true,
            letters: true,
            tint: String::new(),
            cls_search: false,
            space_bg: "#e4d4be".into(),
            space_reversed: false,
            space_keep: false,
            zen: false,
            zen_delay: 2.5,
            hide_bar: false,
            collapsed: ["hub".to_string()].into_iter().collect(),
            view: "memory".into(),
            save: None,
            hub_url: String::new(),
        }
    }
}

impl Prefs {
    pub fn folded(&self, section: &str) -> bool {
        self.collapsed.contains(section)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("ANEND_APP_CONFIG") {
        return p.into();
    }
    config_dir().join("app.json")
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_dir(xdg: &str, fallback: &str) -> std::path::PathBuf {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(std::path::PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os(xdg)
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(fallback)))
    };
    base.unwrap_or_else(|| ".".into()).join("anend")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn config_dir() -> std::path::PathBuf {
    platform_dir("XDG_CONFIG_HOME", ".config")
}

/// where saves live unless the settings say otherwise (the Bevy app's folder)
#[cfg(not(target_arch = "wasm32"))]
pub fn data_dir() -> std::path::PathBuf {
    platform_dir("XDG_DATA_HOME", ".local/share")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Prefs {
    std::fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn store(p: &Prefs) {
    let path = path();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec_pretty(p) {
        let _ = std::fs::write(path, b);
    }
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(target_arch = "wasm32")]
pub fn load() -> Prefs {
    storage()
        .and_then(|s| s.get_item("anend.app").ok().flatten())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
pub fn store(p: &Prefs) {
    if let (Some(s), Ok(t)) = (storage(), serde_json::to_string(p)) {
        let _ = s.set_item("anend.app", &t);
    }
}
