//! Rebindable keys. Every keyboard action has a binding (a key plus modifiers) that the
//! remap menu can change; bindings are kept in the app's settings file, so they stay the
//! same whichever save is open.
//!
//! Bindings with Ctrl or Alt work everywhere. Plain keys only act while no text box has
//! focus, the way weum's memory view ignored its single letter keys inside the term bar.
//! The space's keys only act in the space, and never while typing, the way remem left
//! its terminal alone; the note and memory views' keys only act outside the space.

use bevy::input::keyboard::KeyCode;
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Action {
    #[default]
    Save,
    NewNote,
    ToggleView,
    ToggleList,
    GotoEditor,
    GotoReader,
    GotoMemory,
    GotoStats,
    GotoSpace,
    GotoLists,
    GotoTags,
    GotoClassifiers,
    GotoFilters,
    OpenSettings,
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
    FlyForward,
    FlyBack,
    FlyLeft,
    FlyRight,
    FlyUp,
    FlyDown,
    RollLeft,
    RollRight,
    Level,
    BackToStart,
    Slower,
    Faster,
    NewText,
    EditText,
    SpaceTerm,
    DeleteText,
    CopyText,
    Undo,
    Reversed,
    Pin,
    Sliders,
    Pause,
    Settle,
    SideSum,
    SideReversedSum,
    Help,
    Hud,
    Export,
    SaveMap,
    SpaceKeys,
}

/// where an action's key works: everywhere, in the note and memory views, or in the space
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    Notes,
    Space,
}

impl Scope {
    /// whether two actions can be pressed in the same place, so can't share a key
    pub fn overlaps(self, other: Scope) -> bool {
        self == other || self == Scope::Global || other == Scope::Global
    }
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

const fn shift(key: KeyCode) -> Binding {
    Binding {
        key,
        ctrl: false,
        shift: true,
        alt: false,
    }
}

impl Action {
    pub const ALL: [Action; 57] = [
        Action::Save,
        Action::NewNote,
        Action::ToggleView,
        Action::ToggleList,
        Action::GotoEditor,
        Action::GotoReader,
        Action::GotoMemory,
        Action::GotoStats,
        Action::GotoSpace,
        Action::GotoLists,
        Action::GotoTags,
        Action::GotoClassifiers,
        Action::GotoFilters,
        Action::OpenSettings,
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
        Action::FlyForward,
        Action::FlyBack,
        Action::FlyLeft,
        Action::FlyRight,
        Action::FlyUp,
        Action::FlyDown,
        Action::RollLeft,
        Action::RollRight,
        Action::Level,
        Action::BackToStart,
        Action::Slower,
        Action::Faster,
        Action::NewText,
        Action::EditText,
        Action::SpaceTerm,
        Action::DeleteText,
        Action::CopyText,
        Action::Undo,
        Action::Reversed,
        Action::Pin,
        Action::Sliders,
        Action::Pause,
        Action::Settle,
        Action::SideSum,
        Action::SideReversedSum,
        Action::Help,
        Action::Hud,
        Action::Export,
        Action::SaveMap,
        Action::SpaceKeys,
    ];

    pub fn scope(self) -> Scope {
        use Action::*;
        match self {
            GotoEditor | GotoReader | GotoMemory | GotoStats | GotoSpace | GotoLists | GotoTags | GotoClassifiers
            | GotoFilters | OpenSettings => Scope::Global,
            Save | NewNote | ToggleView | ToggleList | FocusTerm | BucketOrdinal | BucketReversed | BucketAtbash
            | BucketFrequency | ToggleGsearch | ToggleSaturate | WhiteMode | SideOrdinal | SideReversed
            | SideAtbash | SideFrequency | OpenRemap => Scope::Notes,
            _ => Scope::Space,
        }
    }

    /// actions that act while their key is held rather than when it's pressed
    pub fn held(self) -> bool {
        use Action::*;
        matches!(
            self,
            FlyForward | FlyBack | FlyLeft | FlyRight | FlyUp | FlyDown | RollLeft | RollRight
        )
    }

    /// stable name used in the settings table
    pub fn id(self) -> &'static str {
        match self {
            Action::Save => "save",
            Action::NewNote => "new_note",
            Action::ToggleView => "toggle_view",
            Action::ToggleList => "toggle_list",
            Action::GotoEditor => "goto_editor",
            Action::GotoReader => "goto_reader",
            Action::GotoMemory => "goto_memory",
            Action::GotoStats => "goto_stats",
            Action::GotoSpace => "goto_space",
            Action::GotoLists => "goto_lists",
            Action::GotoTags => "goto_tags",
            Action::GotoClassifiers => "goto_classifiers",
            Action::GotoFilters => "goto_filters",
            Action::OpenSettings => "open_settings",
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
            Action::FlyForward => "fly_forward",
            Action::FlyBack => "fly_back",
            Action::FlyLeft => "fly_left",
            Action::FlyRight => "fly_right",
            Action::FlyUp => "fly_up",
            Action::FlyDown => "fly_down",
            Action::RollLeft => "roll_left",
            Action::RollRight => "roll_right",
            Action::Level => "level",
            Action::BackToStart => "back_to_start",
            Action::Slower => "slower",
            Action::Faster => "faster",
            Action::NewText => "new_text",
            Action::EditText => "edit_text",
            Action::SpaceTerm => "space_term",
            Action::DeleteText => "delete_text",
            Action::CopyText => "copy_text",
            Action::Undo => "undo",
            Action::Reversed => "reversed",
            Action::Pin => "pin",
            Action::Sliders => "sliders",
            Action::Pause => "pause",
            Action::Settle => "settle",
            Action::SideSum => "side_sum",
            Action::SideReversedSum => "side_reversed_sum",
            Action::Help => "help",
            Action::Hud => "hud",
            Action::Export => "export",
            Action::SaveMap => "save_map",
            Action::SpaceKeys => "space_keys",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Save => "save the note",
            Action::NewNote => "new note",
            Action::ToggleView => "editor: view / edit",
            Action::ToggleList => "slide the note list away / back",
            Action::GotoEditor => "go to the editor",
            Action::GotoReader => "go to the reader",
            Action::GotoMemory => "go to memory",
            Action::GotoStats => "go to stats",
            Action::GotoSpace => "go to the space",
            Action::GotoLists => "go to the lists",
            Action::GotoTags => "go to the tags",
            Action::GotoClassifiers => "go to the classifiers",
            Action::GotoFilters => "go to the filters",
            Action::OpenSettings => "open the settings (saves, fonts, zen)",
            Action::FocusTerm => "focus the command bar",
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
            Action::FlyForward => "space: fly forward (hold)",
            Action::FlyBack => "space: fly back (hold)",
            Action::FlyLeft => "space: fly left (hold)",
            Action::FlyRight => "space: fly right (hold)",
            Action::FlyUp => "space: fly up (hold)",
            Action::FlyDown => "space: fly down (hold)",
            Action::RollLeft => "space: roll left (hold)",
            Action::RollRight => "space: roll right (hold)",
            Action::Level => "space: level the horizon",
            Action::BackToStart => "space: back to start",
            Action::Slower => "space: slower flight",
            Action::Faster => "space: faster flight",
            Action::NewText => "space: new text",
            Action::EditText => "space: edit the selected text, or the terminal",
            Action::SpaceTerm => "space: terminal",
            Action::DeleteText => "space: remove the selected text",
            Action::CopyText => "space: copy the selected text",
            Action::Undo => "space: undo",
            Action::Reversed => "space: reversed on/off",
            Action::Pin => "space: pin / unpin the selected text",
            Action::Sliders => "space: sliders",
            Action::Pause => "space: pause physics",
            Action::Settle => "space: settle now",
            Action::SideSum => "space: gsearch the terminal's ordinal sum",
            Action::SideReversedSum => "space: gsearch the terminal's reversed sum",
            Action::Help => "space: help",
            Action::Hud => "space: position card",
            Action::Export => "space: export the texts to a file",
            Action::SaveMap => "space: save the map",
            Action::SpaceKeys => "space: open this menu",
        }
    }

    pub fn default_binding(self) -> Binding {
        match self {
            Action::Save => ctrl(KeyCode::KeyS),
            Action::NewNote => ctrl(KeyCode::KeyN),
            Action::ToggleView => ctrl(KeyCode::KeyE),
            Action::ToggleList => ctrl(KeyCode::KeyB),
            Action::GotoEditor => ctrl(KeyCode::Digit1),
            Action::GotoReader => ctrl(KeyCode::Digit2),
            Action::GotoMemory => ctrl(KeyCode::Digit3),
            Action::GotoStats => ctrl(KeyCode::Digit4),
            Action::GotoSpace => ctrl(KeyCode::Digit5),
            Action::GotoLists => ctrl(KeyCode::Digit6),
            Action::GotoClassifiers => ctrl(KeyCode::Digit7),
            Action::GotoTags => ctrl(KeyCode::Digit8),
            Action::GotoFilters => ctrl(KeyCode::Digit9),
            Action::OpenSettings => ctrl(KeyCode::Comma),
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
            Action::FlyForward => plain(KeyCode::KeyW),
            Action::FlyBack => plain(KeyCode::KeyS),
            Action::FlyLeft => plain(KeyCode::KeyA),
            Action::FlyRight => plain(KeyCode::KeyD),
            Action::FlyUp => plain(KeyCode::Space),
            Action::FlyDown => plain(KeyCode::KeyC),
            Action::RollLeft => plain(KeyCode::KeyQ),
            Action::RollRight => plain(KeyCode::KeyE),
            Action::Level => plain(KeyCode::KeyR),
            Action::BackToStart => plain(KeyCode::Home),
            Action::Slower => plain(KeyCode::BracketLeft),
            Action::Faster => plain(KeyCode::BracketRight),
            Action::NewText => plain(KeyCode::KeyN),
            Action::EditText => plain(KeyCode::Enter),
            Action::SpaceTerm => plain(KeyCode::Slash),
            Action::DeleteText => plain(KeyCode::Delete),
            Action::CopyText => ctrl(KeyCode::KeyC),
            Action::Undo => ctrl(KeyCode::KeyZ),
            Action::Reversed => plain(KeyCode::KeyF),
            Action::Pin => plain(KeyCode::KeyU),
            Action::Sliders => plain(KeyCode::KeyO),
            Action::Pause => plain(KeyCode::KeyP),
            Action::Settle => plain(KeyCode::KeyK),
            Action::SideSum => plain(KeyCode::ArrowLeft),
            Action::SideReversedSum => plain(KeyCode::ArrowRight),
            Action::Help => plain(KeyCode::KeyH),
            Action::Hud => plain(KeyCode::KeyI),
            Action::Export => plain(KeyCode::KeyX),
            Action::SaveMap => shift(KeyCode::KeyX),
            Action::SpaceKeys => ctrl(KeyCode::KeyK),
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

    /// binds `action`, handing its old binding to whichever action held the new one where
    /// both can be pressed
    pub fn rebind(&mut self, action: Action, binding: Binding) {
        let old = self.get(action);
        if let Some(other) = Action::ALL
            .iter()
            .copied()
            .find(|a| *a != action && a.scope().overlaps(action.scope()) && self.get(*a) == binding)
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
            // kept before the classifiers came: tags moved from Ctrl+7 to Ctrl+8, as in
            // the web app, unless the player had chosen another key for them
            if !map.contains_key(Action::GotoClassifiers.id())
                && map.get(Action::GotoTags.id()).map(String::as_str) == Some("Ctrl+7")
            {
                keymap.0.insert(Action::GotoTags, ctrl(KeyCode::Digit8));
            }
        }
        keymap
    }
}

/// writes changed bindings to the settings file
pub fn persist_keymap(keymap: Res<Keymap>, mut config: ResMut<crate::config::Config>) {
    if keymap.is_changed() && !keymap.is_added() {
        config.keymap = serde_json::from_slice(&keymap.to_json()).ok();
        config.save();
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
    session: Res<crate::ui::Session>,
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
    let in_space = session.route == crate::ui::Route::Space;
    for key in keys.get_just_pressed() {
        for action in Action::ALL {
            let active = match action.scope() {
                Scope::Global => true,
                Scope::Notes => !in_space,
                Scope::Space => in_space && !typing && !action.held(),
            };
            let b = keymap.get(action);
            if active
                && b.key == *key
                && b.ctrl == ctrl
                && b.alt == alt
                && b.shift == shift
                && (b.is_chord() || !typing)
            {
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
        // every default is nameable and distinct where both can be pressed
        for (i, a) in Action::ALL.iter().enumerate() {
            assert!(key_name(a.default_binding().key).is_some());
            for b in &Action::ALL[i + 1..] {
                assert!(
                    !(a.scope().overlaps(b.scope()) && a.default_binding() == b.default_binding()),
                    "{:?} and {:?} share {}",
                    a,
                    b,
                    a.default_binding()
                );
            }
        }
        // a keymap kept before the classifiers moves tags to Ctrl+8
        let old = br#"{"goto_tags":"Ctrl+7","save":"Ctrl+S"}"#;
        let m = Keymap::from_json(old);
        assert_eq!(m.get(Action::GotoTags).to_string(), "Ctrl+8");
        assert_eq!(m.get(Action::GotoClassifiers).to_string(), "Ctrl+7");
        let mine = br#"{"goto_tags":"Ctrl+T"}"#;
        assert_eq!(Keymap::from_json(mine).get(Action::GotoTags).to_string(), "Ctrl+T");
        // the space's keys don't push the memory view's out
        k.rebind(Action::Settle, plain(KeyCode::KeyG));
        assert_eq!(k.get(Action::ToggleGsearch), plain(KeyCode::KeyG));
    }
}
