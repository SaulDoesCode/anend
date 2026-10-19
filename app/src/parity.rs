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
];

/// the views, in their tab order
#[cfg(test)]
pub const VIEWS: &[&str] = &[
    "memory", "space", "lists", "reader", "notes", "tags", "stats", "models", "filters",
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
    fn views_stay() {
        let have: Vec<&str> = crate::ui::View::ALL.iter().map(|v| v.name()).collect();
        assert_eq!(have, VIEWS, "the views changed: say why in docs/parity.md");
    }
}
