//! The app's own settings, kept in a small JSON file next to (not inside) the saves, so
//! they stay when you swap one database for another: where the saves live, which one is
//! open, the fonts, zen mode, the top bar and the key bindings.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Resource, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    /// the folder new saves go in and the saves menu lists
    pub saves_dir: PathBuf,
    /// the save that's open
    pub current: Option<PathBuf>,
    /// the interface font's family; none: the built in Fira Sans
    pub font: Option<String>,
    /// the memory view's font; none: the system serif
    pub memory_font: Option<String>,
    /// fade the interface away while the mouse rests
    pub zen: bool,
    /// seconds of stillness before zen fades
    pub zen_delay: f32,
    /// slide the top bar away until the pointer comes near it
    pub hide_bar: bool,
    /// colours changed from the built in ones, by role name, as `#rrggbb`
    pub colors: std::collections::BTreeMap<String, String>,
    /// the whole interface's scale
    pub ui_scale: f32,
    /// the top bar's height in pixels; its buttons and text fit it
    pub top_bar: f32,
    /// the command bar's height in pixels
    pub bottom_bar: f32,
    /// the space's search line (the sums and the digit chain) in the top bar
    pub space_sums: bool,
    /// slide the note list away until the pointer comes to the left edge
    pub hide_list: bool,
    pub keymap: Option<serde_json::Value>,
    /// the anend hub saves are shared on, e.g. `http://localhost:8080`
    pub hub_url: String,
    /// an app token made on the hub's profile page
    pub hub_token: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            saves_dir: data_dir(),
            current: None,
            font: None,
            memory_font: None,
            zen: false,
            zen_delay: 2.5,
            hide_bar: false,
            colors: Default::default(),
            ui_scale: 1.0,
            top_bar: 32.0,
            bottom_bar: 30.0,
            space_sums: true,
            hide_list: false,
            keymap: None,
            hub_url: String::new(),
            hub_token: String::new(),
        }
    }
}

fn platform_dir(xdg: &str, fallback: &str) -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os(xdg)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join("anend")
}

/// where saves go unless the settings say otherwise
pub fn data_dir() -> PathBuf {
    platform_dir("XDG_DATA_HOME", ".local/share")
}

fn config_path() -> PathBuf {
    std::env::var_os("ANEND_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| platform_dir("XDG_CONFIG_HOME", ".config").join("config.json"))
}

impl Config {
    pub fn load() -> Config {
        std::fs::read(config_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, bytes);
        }
    }

    /// the save to open: the last one, or `notes.redb` in the saves folder
    pub fn current_save(&self) -> PathBuf {
        self.current
            .clone()
            .unwrap_or_else(|| self.saves_dir.join("notes.redb"))
    }
}

/// a save's file name without `.redb`
pub fn save_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// a name typed for a save, made safe for a file name
pub fn clean_name(name: &str) -> Option<String> {
    let n: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let n = n.trim_matches('.').trim().to_string();
    (!n.is_empty() && n.len() <= 96).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(save_name(Path::new("/a/b/my notes.redb")), "my notes");
        assert_eq!(clean_name(" journal/2026 ").as_deref(), Some("journal-2026"));
        assert_eq!(clean_name("..."), None);
        assert_eq!(clean_name(""), None);
        let c: Config = serde_json::from_str(r#"{"zen": true}"#).unwrap();
        assert!(c.zen && !c.hide_bar);
        assert_eq!(c.zen_delay, 2.5);
        assert!(c.hub_url.is_empty() && c.hub_token.is_empty());
    }
}
