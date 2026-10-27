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
    /// the pipes whose effects the memory view and the space show, stacked in this order
    pub pipes: Vec<String>,
    /// how the space's bar searches: normal, classifier or random
    pub space_search: String,
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
    /// the memory view's search filter panel is open
    pub filter_open: bool,
    /// what the memory view's search filter keeps
    pub filter: MemFilter,
    /// the keys changed from their defaults: action id → binding (keys.rs)
    pub keymap: BTreeMap<String, String>,
    /// colour the markdown in the note editor
    pub highlight: bool,
    /// underline likely spelling and word-choice errors in the note editor
    pub check_spelling: bool,
    /// the note list slid away
    pub hide_list: bool,
    /// the note list at the editor's right
    pub list_right: bool,
    /// the memory view's white mode: dark texts on white
    pub white: bool,
    /// the memory view labels every text's words with their types
    pub word_types: bool,
    /// the space's panels, physics, history and last address
    pub space: SpaceUi,
    /// the interface's font family (empty: the built-in Fira Sans)
    pub font: String,
    /// the memory view's font family (empty: the system serif)
    pub memory_family: String,
    /// the top bar's and the command bar's heights in pixels
    pub top_bar: f32,
    pub bottom_bar: f32,
    /// the top bar at the bottom, the command bar at the top
    pub bars_swapped: bool,
    /// an app token from the hub's profile page (sharing needs one)
    pub hub_token: String,
    /// where classifiers read texts: auto, cpu, gpu or gpu_fast (`classify.device`)
    pub read_device: String,
    /// a long text is shown piece by piece (`anend_core::pieces`): characters a line and
    /// lines a piece
    pub piece_line: usize,
    pub piece_lines: usize,
}

/// the space's own settings
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpaceUi {
    /// the physics panel's sliders changed from their defaults (views/space.rs `SLIDERS`)
    pub physics: BTreeMap<String, f64>,
    /// what the depth follows: none, ordinal, length or time
    pub depth_by: String,
    /// what was typed in the space's bar (↑ ↓), newest last
    pub history: Vec<String>,
    /// the last address (`123`, `r123` or a map's name), opened again next time
    pub address: String,
    pub show_physics: bool,
    pub show_hud: bool,
    pub show_help: bool,
    pub show_details: bool,
    /// the blocks window (spatial saves)
    pub show_blocks: bool,
    /// the saved scenes (maps) window
    pub show_scenes: bool,
    /// the textures window (texture recipes)
    pub show_textures: bool,
    /// the lights window
    pub show_lights: bool,
    /// the animations window
    pub show_anims: bool,
    /// the wiring window (blocks that cast rays and run programs)
    pub show_wiring: bool,
    /// the prefabs window (subassemblies and their menu)
    pub show_prefabs: bool,
    /// the sounds window (the save's sounds and the selected block's emitter)
    pub show_sounds: bool,
    /// where each movable window was left (`details`, `physics`): its left and top
    pub wins: BTreeMap<String, (f64, f64)>,
}

impl Default for SpaceUi {
    fn default() -> SpaceUi {
        SpaceUi {
            physics: BTreeMap::new(),
            depth_by: "none".into(),
            history: Vec::new(),
            address: String::new(),
            show_physics: false,
            show_hud: true,
            show_help: false,
            show_details: true,
            show_blocks: false,
            show_scenes: false,
            show_textures: false,
            show_lights: false,
            show_anims: false,
            show_wiring: false,
            show_prefabs: false,
            show_sounds: false,
            wins: BTreeMap::new(),
        }
    }
}

/// the memory view's search filter: what of the shown texts it keeps (every part empty or
/// off keeps everything)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemFilter {
    /// leave out notes / texts that aren't notes
    pub no_notes: bool,
    pub no_texts: bool,
    /// kinds of text (core's kinds::detect) to keep; empty: all
    pub kinds: BTreeSet<String>,
    /// word types a text must have (each of them at least once)
    pub words: BTreeSet<String>,
    /// classes of the colouring classifier to keep ("unsure" for none); empty: all
    pub classes: BTreeSet<String>,
    /// only texts the colouring classifier was given a yes or no on
    pub feedback: bool,
}

impl MemFilter {
    pub fn active(&self) -> bool {
        *self != MemFilter::default()
    }
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
            pipes: Vec::new(),
            space_search: "normal".into(),
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
            filter_open: false,
            filter: MemFilter::default(),
            keymap: BTreeMap::new(),
            highlight: true,
            check_spelling: true,
            hide_list: false,
            list_right: false,
            white: false,
            word_types: false,
            space: SpaceUi::default(),
            font: String::new(),
            memory_family: String::new(),
            top_bar: 32.0,
            bottom_bar: 32.0,
            bars_swapped: false,
            hub_token: String::new(),
            read_device: "auto".into(),
            piece_line: anend_core::pieces::LINE,
            piece_lines: anend_core::pieces::LINES,
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
