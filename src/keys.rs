//! Rebindable keys. Every keyboard action has a binding (a key plus modifiers) that the
//! remap menu can change; bindings are kept in the database's settings table.
//!
//! Bindings with Ctrl or Alt work everywhere. Plain keys only act while no text box has
//! focus, the way weum's memory view ignored its single letter keys inside the term bar.

use bevy::input::keyboard::KeyCode;
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Save,
    NewNote,
    ToggleView,
    GotoEditor,
    GotoReader,
    GotoMemory,
    GotoStats,
    FocusTerm,
    BucketOrdinal,
    BucketReversed,
    BucketAtbash,
    BucketFrequency,
    ToggleGsearch,
    ToggleSaturate,
    WhiteMode,
    SideOrdinal,
    SideReversed,
    SideAtbash,
    SideFrequency,
    OpenRemap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub key: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

const fn plain(key: KeyCode) -> Binding {
    Binding {
        key,
        ctrl: false,
        shift: false,
        alt: false,
    }
}

const fn ctrl(key: KeyCode) -> Binding {
    Binding {
        key,
        ctrl: true,
        shift: false,
        alt: false,
    }
}

impl Action {
    pub const ALL: [Action; 20] = [
        Action::Save,
        Action::NewNote,
        Action::ToggleView,
        Action::GotoEditor,
        Action::GotoReader,
        Action::GotoMemory,
        Action::GotoStats,
        Action::FocusTerm,
        Action::BucketOrdinal,
        Action::BucketReversed,
        Action::BucketAtbash,
        Action::BucketFrequency,
        Action::ToggleGsearch,
        Action::ToggleSaturate,
        Action::WhiteMode,
        Action::SideOrdinal,
        Action::SideReversed,
        Action::SideAtbash,
        Action::SideFrequency,
        Action::OpenRemap,
    ];

    /// stable name used in the settings table
    pub fn id(self) -> &'static str {
        match self {
            Action::Save => "save",
            Action::NewNote => "new_note",
            Action::ToggleView => "toggle_view",
            Action::GotoEditor => "goto_editor",
            Action::GotoReader => "goto_reader",
            Action::GotoMemory => "goto_memory",
            Action::GotoStats => "goto_stats",
            Action::FocusTerm => "focus_term",
            Action::BucketOrdinal => "bucket_ordinal",
            Action::BucketReversed => "bucket_reversed",
            Action::BucketAtbash => "bucket_atbash",
            Action::BucketFrequency => "bucket_frequency",
            Action::ToggleGsearch => "toggle_gsearch",
            Action::ToggleSaturate => "toggle_saturate",
            Action::WhiteMode => "white_mode",
            Action::SideOrdinal => "side_ordinal",
            Action::SideReversed => "side_reversed",
            Action::SideAtbash => "side_atbash",
            Action::SideFrequency => "side_frequency",
            Action::OpenRemap => "open_remap",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Save => "save the note",
            Action::NewNote => "new note",
            Action::ToggleView => "editor: view / edit",
            Action::GotoEditor => "go to the editor",
            Action::GotoReader => "go to the reader",
            Action::GotoMemory => "go to memory",
            Action::GotoStats => "go to stats",
            Action::FocusTerm => "memory: focus the term bar",
            Action::BucketOrdinal => "memory: ordinal bucket",
            Action::BucketReversed => "memory: reversed bucket (again: back to ordinal)",
            Action::BucketAtbash => "memory: atbash bucket",
            Action::BucketFrequency => "memory: frequency bucket",
            Action::ToggleGsearch => "memory: gsearch typed text on/off",
            Action::ToggleSaturate => "memory: saturate searches on/off",
            Action::WhiteMode => "memory: white mode",
            Action::SideOrdinal => "memory: gsearch the term's ordinal",
            Action::SideReversed => "memory: gsearch the term's reversed",
            Action::SideAtbash => "memory: gsearch the term's atbash",
            Action::SideFrequency => "memory: gsearch the term's letters",
            Action::OpenRemap => "open this menu",
        }
    }

    pub fn default_binding(self) -> Binding {
        match self {
            Action::Save => ctrl(KeyCode::KeyS),
            Action::NewNote => ctrl(KeyCode::KeyN),
            Action::ToggleView => ctrl(KeyCode::KeyE),
            Action::GotoEditor => ctrl(KeyCode::Digit1),
            Action::GotoReader => ctrl(KeyCode::Digit2),
            Action::GotoMemory => ctrl(KeyCode::Digit3),
            Action::GotoStats => ctrl(KeyCode::Digit4),
            Action::FocusTerm => plain(KeyCode::Slash),
            Action::BucketOrdinal => plain(KeyCode::KeyO),
            Action::BucketReversed => plain(KeyCode::KeyR),
            Action::BucketAtbash => plain(KeyCode::KeyA),
            Action::BucketFrequency => plain(KeyCode::KeyF),
            Action::ToggleGsearch => plain(KeyCode::KeyG),
            Action::ToggleSaturate => plain(KeyCode::KeyS),
            Action::WhiteMode => plain(KeyCode::KeyW),
            Action::SideOrdinal => plain(KeyCode::ArrowLeft),
            Action::SideReversed => plain(KeyCode::ArrowRight),
            Action::SideAtbash => plain(KeyCode::ArrowUp),
            Action::SideFrequency => plain(KeyCode::ArrowDown),
            Action::OpenRemap => plain(KeyCode::KeyK),
        }
    }
}

/// keys that can be bound, with the names shown and stored for them
const KEYS: &[(KeyCode, &str)] = &[
    (KeyCode::KeyA, "A"),
    (KeyCode::KeyB, "B"),
    (KeyCode::KeyC, "C"),
    (KeyCode::KeyD, "D"),
    (KeyCode::KeyE, "E"),
    (KeyCode::KeyF, "F"),
    (KeyCode::KeyG, "G"),
    (KeyCode::KeyH, "H"),
    (KeyCode::KeyI, "I"),
    (KeyCode::KeyJ, "J"),
    (KeyCode::KeyK, "K"),
    (KeyCode::KeyL, "L"),
    (KeyCode::KeyM, "M"),
    (KeyCode::KeyN, "N"),
    (KeyCode::KeyO, "O"),
    (KeyCode::KeyP, "P"),
    (KeyCode::KeyQ, "Q"),
    (KeyCode::KeyR, "R"),
    (KeyCode::KeyS, "S"),
    (KeyCode::KeyT, "T"),
    (KeyCode::KeyU, "U"),
    (KeyCode::KeyV, "V"),
    (KeyCode::KeyW, "W"),
    (KeyCode::KeyX, "X"),
    (KeyCode::KeyY, "Y"),
    (KeyCode::KeyZ, "Z"),
    (KeyCode::Digit0, "0"),
    (KeyCode::Digit1, "1"),
    (KeyCode::Digit2, "2"),
    (KeyCode::Digit3, "3"),
    (KeyCode::Digit4, "4"),
    (KeyCode::Digit5, "5"),
    (KeyCode::Digit6, "6"),
    (KeyCode::Digit7, "7"),
    (KeyCode::Digit8, "8"),
    (KeyCode::Digit9, "9"),
    (KeyCode::F1, "F1"),
    (KeyCode::F2, "F2"),
    (KeyCode::F3, "F3"),
    (KeyCode::F4, "F4"),
    (KeyCode::F5, "F5"),
    (KeyCode::F6, "F6"),
    (KeyCode::F7, "F7"),
    (KeyCode::F8, "F8"),
    (KeyCode::F9, "F9"),
    (KeyCode::F10, "F10"),
    (KeyCode::F11, "F11"),
    (KeyCode::F12, "F12"),
    (KeyCode::ArrowLeft, "Left"),
    (KeyCode::ArrowRight, "Right"),
    (KeyCode::ArrowUp, "Up"),
    (KeyCode::ArrowDown, "Down"),
    (KeyCode::Enter, "Enter"),
    (KeyCode::Tab, "Tab"),
    (KeyCode::Space, "Space"),
    (KeyCode::Backspace, "Backspace"),
    (KeyCode::Delete, "Delete"),
    (KeyCode::Insert, "Insert"),
    (KeyCode::Home, "Home"),
    (KeyCode::End, "End"),
    (KeyCode::PageUp, "PageUp"),
    (KeyCode::PageDown, "PageDown"),
    (KeyCode::Minus, "-"),
    (KeyCode::Equal, "="),
    (KeyCode::BracketLeft, "["),
    (KeyCode::BracketRight, "]"),
    (KeyCode::Backslash, "\\"),
    (KeyCode::Semicolon, ";"),
    (KeyCode::Quote, "'"),
    (KeyCode::Comma, ","),
    (KeyCode::Period, "."),
    (KeyCode::Slash, "/"),
    (KeyCode::Backquote, "`"),
];

pub fn key_name(key: KeyCode) -> Option<&'static str> {
    KEYS.iter().find(|(k, _)| *k == key).map(|(_, n)| *n)
}

fn key_from_name(name: &str) -> Option<KeyCode> {
    KEYS.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(k, _)| *k)
}

impl std::fmt::Display for Binding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        f.write_str(key_name(self.key).unwrap_or("?"))
    }
}

impl Binding {
    pub fn parse(s: &str) -> Option<Binding> {
        let mut b = plain(KeyCode::KeyA);
        let mut rest = s;
        loop {
            if let Some(r) = rest.strip_prefix("Ctrl+") {
                b.ctrl = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("Alt+") {
                b.alt = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("Shift+") {
                b.shift = true;
                rest = r;
            } else {
                break;
            }
        }
        b.key = key_from_name(rest)?;
        Some(b)
    }

    /// chords work while typing, plain keys don't
    pub fn is_chord(&self) -> bool {
        self.ctrl || self.alt
    }
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Keymap(pub HashMap<Action, Binding>);

impl Default for Keymap {
    fn default() -> Self {
        Keymap(Action::ALL.iter().map(|a| (*a, a.default_binding())).collect())
    }
}

impl Keymap {
    pub fn get(&self, a: Action) -> Binding {
        self.0.get(&a).copied().unwrap_or_else(|| a.default_binding())
    }

    /// binds `action`, handing its old binding to whichever action held the new one
    pub fn rebind(&mut self, action: Action, binding: Binding) {
        let old = self.get(action);
        if let Some(other) = Action::ALL
            .iter()
            .copied()
            .find(|a| *a != action && self.get(*a) == binding)
        {
            self.0.insert(other, old);
        }
        self.0.insert(action, binding);
    }

    pub fn to_json(&self) -> Vec<u8> {
        let map: HashMap<&str, String> = Action::ALL.iter().map(|a| (a.id(), self.get(*a).to_string())).collect();
        serde_json::to_vec(&map).unwrap_or_default()
    }

    pub fn from_json(bytes: &[u8]) -> Keymap {
        let mut keymap = Keymap::default();
        if let Ok(map) = serde_json::from_slice::<HashMap<String, String>>(bytes) {
            for a in Action::ALL {
                if let Some(b) = map.get(a.id()).and_then(|s| Binding::parse(s)) {
                    keymap.0.insert(a, b);
                }
            }
        }
        keymap
    }
}

/// fired for every action whose binding was pressed
#[derive(Message, Clone, Copy, Debug)]
pub struct Fired(pub Action);

/// the remap menu: whether it shows and which action waits for a key
#[derive(Resource, Default)]
pub struct Remap {
    pub open: bool,
    pub listening: Option<Action>,
    pub changed: bool,
}

pub fn modifiers(keys: &ButtonInput<KeyCode>) -> (bool, bool, bool) {
    (
        keys.any_pressed([
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ]),
        keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]),
    )
}

/// turns key presses into actions, or into a new binding while the remap menu listens
pub fn read_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut keymap: ResMut<Keymap>,
    mut remap: ResMut<Remap>,
    focus: Res<InputFocus>,
    editables: Query<(), With<EditableText>>,
    db: Res<crate::ui::Db>,
    mut fired: MessageWriter<Fired>,
) {
    let (ctrl, shift, alt) = modifiers(&keys);
    if let Some(action) = remap.listening {
        for key in keys.get_just_pressed() {
            if *key == KeyCode::Escape {
                remap.listening = None;
                remap.changed = true;
                return;
            }
            if key_name(*key).is_some() {
                keymap.rebind(
                    action,
                    Binding {
                        key: *key,
                        ctrl,
                        shift,
                        alt,
                    },
                );
                let _ = db.0.set_setting("keymap", &keymap.to_json());
                remap.listening = None;
                remap.changed = true;
                return;
            }
        }
        return;
    }
    if remap.open && keys.just_pressed(KeyCode::Escape) {
        remap.open = false;
        remap.changed = true;
        return;
    }
    let typing = focus.get().is_some_and(|e| editables.contains(e));
    for key in keys.get_just_pressed() {
        for action in Action::ALL {
            let b = keymap.get(action);
            if b.key == *key && b.ctrl == ctrl && b.alt == alt && b.shift == shift && (b.is_chord() || !typing) {
                fired.write(Fired(action));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_roundtrip_and_swap() {
        let mut k = Keymap::default();
        assert_eq!(k.get(Action::Save).to_string(), "Ctrl+S");
        assert_eq!(
            Binding::parse("Ctrl+Shift+Left"),
            Some(Binding {
                key: KeyCode::ArrowLeft,
                ctrl: true,
                shift: true,
                alt: false
            })
        );
        assert_eq!(Binding::parse("Nope"), None);

        // taking another action's key swaps them
        k.rebind(Action::BucketAtbash, plain(KeyCode::KeyR));
        assert_eq!(k.get(Action::BucketAtbash), plain(KeyCode::KeyR));
        assert_eq!(k.get(Action::BucketReversed), plain(KeyCode::KeyA));

        let back = Keymap::from_json(&k.to_json());
        assert_eq!(back, k);
        assert_eq!(Keymap::from_json(b"garbage"), Keymap::default());
        // every default is distinct and nameable
        let defaults: Vec<Binding> = Action::ALL.iter().map(|a| a.default_binding()).collect();
        for (i, a) in defaults.iter().enumerate() {
            assert!(key_name(a.key).is_some());
            assert!(!defaults[i + 1..].contains(a), "{:?} bound twice", a);
        }
    }
}
