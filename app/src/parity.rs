//! Guards for docs/parity.md: the settings keys and the views the app has. A key or a view
//! that goes away fails these tests until the expected list here and docs/parity.md are
//! changed together, saying why.

/// every key the app's settings file holds (`prefs::Prefs`); add new ones here, and never
/// take one out without a line in docs/parity.md
#[cfg(test)]
pub const SETTINGS: &[&str] = &[
    "palette",
    "colours",
    "font_size",
    "zoom",
    "memory_font",
    "spacing",
    "field_width",
    "mode",
    "gsearch_text",
    "saturate",
    "letters",
    "tint",
    "cls_search",
    "space_bg",
    "space_reversed",
    "space_keep",
    "zen",
    "zen_delay",
    "hide_bar",
    "collapsed",
    "view",
    "save",
    "hub_url",
    "filter_open",
    "filter",
    "keymap",
    "highlight",
    "hide_list",
    "list_right",
    "white",
    "word_types",
];

/// the views, in their tab order
#[cfg(test)]
pub const VIEWS: &[&str] = &[
    "memory", "space", "lists", "reader", "notes", "tags", "stats", "models", "filters",
];

/// the keymap's action ids (keys.rs): every one the Bevy app had that still applies, and
/// the ones added since
#[cfg(test)]
pub const KEY_ACTIONS: &[&str] = &[
    "goto_memory",
    "goto_space",
    "goto_lists",
    "goto_reader",
    "goto_notes",
    "goto_tags",
    "goto_stats",
    "goto_models",
    "goto_filters",
    "open_settings",
    "open_keys",
    "focus_bar",
    "undo",
    "zoom_in",
    "zoom_out",
    "zoom_reset",
    "save",
    "new_note",
    "toggle_read",
    "toggle_list",
    "note_classifiers",
    "toggle_check",
    "bucket_ordinal",
    "bucket_reversed",
    "bucket_frequency",
    "toggle_gsearch",
    "toggle_saturate",
    "white_mode",
    "letters",
    "classifier_search",
    "word_types",
    "side_ordinal",
    "side_reversed",
    "side_frequency",
    "fly_forward",
    "fly_back",
    "fly_left",
    "fly_right",
    "fly_up",
    "fly_down",
    "roll_left",
    "roll_right",
    "level",
    "back_to_start",
    "slower",
    "faster",
    "new_text",
    "edit_text",
    "delete_text",
    "copy_text",
    "reversed",
    "pin",
    "physics",
    "pause",
    "settle",
    "side_sum",
    "side_reversed_sum",
    "help",
    "hud",
    "details",
    "export",
    "save_map",
    "read_prev",
    "read_next",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_keys_stay() {
        let v = serde_json::to_value(crate::prefs::Prefs::default()).unwrap();
        let have: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        for k in SETTINGS {
            assert!(have.contains(k), "the setting {k} went away: say why in docs/parity.md");
        }
        for k in &have {
            assert!(SETTINGS.contains(k), "a new setting {k}: add it to parity::SETTINGS");
        }
    }

    #[test]
    fn key_actions_stay() {
        let have: Vec<&str> = crate::keys::Action::ALL.iter().map(|a| a.id()).collect();
        for k in KEY_ACTIONS {
            assert!(
                have.contains(k),
                "the key action {k} went away: say why in docs/parity.md"
            );
        }
        for k in &have {
            assert!(
                KEY_ACTIONS.contains(k),
                "a new key action {k}: add it to parity::KEY_ACTIONS"
            );
        }
    }

    #[test]
    fn views_stay() {
        let have: Vec<&str> = crate::ui::View::ALL.iter().map(|v| v.name()).collect();
        assert_eq!(have, VIEWS, "the views changed: say why in docs/parity.md");
    }
}
