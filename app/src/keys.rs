//! Rebindable keys, as the Bevy app had them (its keys.rs): every keyboard action has a
//! binding (a key and its modifiers) that the keys menu in the settings changes; the
//! changes are kept in the app's settings file (`Prefs::keymap`, only the ones that differ
//! from the defaults), so they stay whichever save is open, on the desktop and the web.
//!
//! Keys are named by where they sit (the `code` of a key event: `KeyW` is W on any layout),
//! like Bevy's `KeyCode`. Bindings with Ctrl or Alt work everywhere, even while typing;
//! plain keys only act while no text box has focus. Each action has a scope: global, or
//! one view; two actions only clash (and swap when one takes the other's key) when both
//! can be pressed in the same place.

use std::collections::BTreeMap;

use crate::ui::View;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    // everywhere
    GotoMemory,
    GotoSpace,
    GotoLists,
    GotoReader,
    GotoNotes,
    GotoTags,
    GotoStats,
    GotoModels,
    GotoFilters,
    OpenSettings,
    OpenKeys,
    FocusBar,
    Undo,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    // notes
    Save,
    NewNote,
    ToggleRead,
    ToggleList,
    NoteClassifiers,
    ToggleCheck,
    // memory
    BucketOrdinal,
    BucketReversed,
    BucketFrequency,
    ToggleGsearch,
    ToggleSaturate,
    WhiteMode,
    Letters,
    ClassifierSearch,
    WordTypes,
    SideOrdinal,
    SideReversed,
    SideFrequency,
    // space
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
    Walk,
    SuperFlight,
    Grapple,
    Sneak,
    NewText,
    EditText,
    DeleteText,
    CopyText,
    Reversed,
    Pin,
    Physics,
    Pause,
    Settle,
    SideSum,
    SideReversedSum,
    Help,
    Hud,
    Details,
    Export,
    SaveMap,
    // reader
    ReadPrev,
    ReadNext,
}

/// where an action's key works
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    In(View),
}

impl Scope {
    /// whether two actions can be pressed in the same place, so can't share a key
    pub fn overlaps(self, other: Scope) -> bool {
        self == other || self == Scope::Global || other == Scope::Global
    }

    pub fn name(self) -> &'static str {
        match self {
            Scope::Global => "everywhere",
            Scope::In(v) => v.name(),
        }
    }
}

use Action::*;

impl Action {
    pub const ALL: [Action; 68] = [
        GotoMemory,
        GotoSpace,
        GotoLists,
        GotoReader,
        GotoNotes,
        GotoTags,
        GotoStats,
        GotoModels,
        GotoFilters,
        OpenSettings,
        OpenKeys,
        FocusBar,
        Undo,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        Save,
        NewNote,
        ToggleRead,
        ToggleList,
        NoteClassifiers,
        ToggleCheck,
        BucketOrdinal,
        BucketReversed,
        BucketFrequency,
        ToggleGsearch,
        ToggleSaturate,
        WhiteMode,
        Letters,
        ClassifierSearch,
        WordTypes,
        SideOrdinal,
        SideReversed,
        SideFrequency,
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
        Walk,
        SuperFlight,
        Grapple,
        Sneak,
        NewText,
        EditText,
        DeleteText,
        CopyText,
        Reversed,
        Pin,
        Physics,
        Pause,
        Settle,
        SideSum,
        SideReversedSum,
        Help,
        Hud,
        Details,
        Export,
        SaveMap,
        ReadPrev,
        ReadNext,
    ];

    pub fn scope(self) -> Scope {
        match self {
            GotoMemory | GotoSpace | GotoLists | GotoReader | GotoNotes | GotoTags | GotoStats | GotoModels
            | GotoFilters | OpenSettings | OpenKeys | FocusBar | Undo | ZoomIn | ZoomOut | ZoomReset => Scope::Global,
            Save | NewNote | ToggleRead | ToggleList | NoteClassifiers | ToggleCheck => Scope::In(View::Notes),
            BucketOrdinal | BucketReversed | BucketFrequency | ToggleGsearch | ToggleSaturate | WhiteMode | Letters
            | ClassifierSearch | WordTypes | SideOrdinal | SideReversed | SideFrequency => Scope::In(View::Memory),
            ReadPrev | ReadNext => Scope::In(View::Reader),
            _ => Scope::In(View::Space),
        }
    }

    /// the space's flying keys act while held (space.js reads them), not on a press
    pub fn held(self) -> bool {
        matches!(
            self,
            FlyForward | FlyBack | FlyLeft | FlyRight | FlyUp | FlyDown | RollLeft | RollRight | Sneak
        )
    }

    /// the stable name kept in the settings file
    pub fn id(self) -> &'static str {
        match self {
            GotoMemory => "goto_memory",
            GotoSpace => "goto_space",
            GotoLists => "goto_lists",
            GotoReader => "goto_reader",
            GotoNotes => "goto_notes",
            GotoTags => "goto_tags",
            GotoStats => "goto_stats",
            GotoModels => "goto_models",
            GotoFilters => "goto_filters",
            OpenSettings => "open_settings",
            OpenKeys => "open_keys",
            FocusBar => "focus_bar",
            Undo => "undo",
            ZoomIn => "zoom_in",
            ZoomOut => "zoom_out",
            ZoomReset => "zoom_reset",
            Save => "save",
            NewNote => "new_note",
            ToggleRead => "toggle_read",
            ToggleList => "toggle_list",
            NoteClassifiers => "note_classifiers",
            ToggleCheck => "toggle_check",
            BucketOrdinal => "bucket_ordinal",
            BucketReversed => "bucket_reversed",
            BucketFrequency => "bucket_frequency",
            ToggleGsearch => "toggle_gsearch",
            ToggleSaturate => "toggle_saturate",
            WhiteMode => "white_mode",
            Letters => "letters",
            ClassifierSearch => "classifier_search",
            WordTypes => "word_types",
            SideOrdinal => "side_ordinal",
            SideReversed => "side_reversed",
            SideFrequency => "side_frequency",
            FlyForward => "fly_forward",
            FlyBack => "fly_back",
            FlyLeft => "fly_left",
            FlyRight => "fly_right",
            FlyUp => "fly_up",
            FlyDown => "fly_down",
            RollLeft => "roll_left",
            RollRight => "roll_right",
            Level => "level",
            BackToStart => "back_to_start",
            Slower => "slower",
            Faster => "faster",
            Walk => "walk",
            SuperFlight => "superflight",
            Grapple => "grapple",
            Sneak => "sneak",
            NewText => "new_text",
            EditText => "edit_text",
            DeleteText => "delete_text",
            CopyText => "copy_text",
            Reversed => "reversed",
            Pin => "pin",
            Physics => "physics",
            Pause => "pause",
            Settle => "settle",
            SideSum => "side_sum",
            SideReversedSum => "side_reversed_sum",
            Help => "help",
            Hud => "hud",
            Details => "details",
            Export => "export",
            SaveMap => "save_map",
            ReadPrev => "read_prev",
            ReadNext => "read_next",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GotoMemory => "go to memory",
            GotoSpace => "go to the space",
            GotoLists => "go to the lists",
            GotoReader => "go to the reader",
            GotoNotes => "go to the notes",
            GotoTags => "go to the tags",
            GotoStats => "go to stats",
            GotoModels => "go to the models (classifiers)",
            GotoFilters => "go to the filters",
            OpenSettings => "open the settings",
            OpenKeys => "open this keys menu",
            FocusBar => "focus the command bar",
            Undo => "take back the latest action",
            ZoomIn => "zoom in",
            ZoomOut => "zoom out",
            ZoomReset => "zoom back to 100%",
            Save => "save the note",
            NewNote => "new note",
            ToggleRead => "read / edit the note",
            ToggleList => "slide the note list away / back",
            NoteClassifiers => "the note's classifiers",
            ToggleCheck => "check spelling and grammar",
            BucketOrdinal => "ordinal bucket",
            BucketReversed => "reversed bucket (again: back to ordinal)",
            BucketFrequency => "frequency bucket (again: back to ordinal)",
            ToggleGsearch => "gsearch typed text on / off",
            ToggleSaturate => "saturate searches on / off",
            WhiteMode => "white mode",
            Letters => "the sidecar's letters",
            ClassifierSearch => "search with the colouring classifier",
            WordTypes => "label every text's word types",
            SideOrdinal => "gsearch the bar's ordinal sum",
            SideReversed => "gsearch the bar's reversed sum",
            SideFrequency => "gsearch the bar's letters",
            FlyForward => "fly forward (hold)",
            FlyBack => "fly back (hold)",
            FlyLeft => "fly left (hold)",
            FlyRight => "fly right (hold)",
            FlyUp => "fly up (hold)",
            FlyDown => "fly down (hold)",
            RollLeft => "roll left (hold)",
            RollRight => "roll right (hold)",
            Level => "level the horizon",
            BackToStart => "back to start",
            Slower => "slower flight",
            Faster => "faster flight",
            Walk => "walk / fly (first person: gravity, jump, crouch, lean, mantle)",
            SuperFlight => "superflight / fly (collects the texts flown through)",
            Grapple => "grapple: hook what's ahead / let go (the wheel reels)",
            Sneak => "sneak while walking (hold)",
            NewText => "new text ahead",
            EditText => "edit the selected text",
            DeleteText => "remove the selected text",
            CopyText => "copy the selected text",
            Reversed => "reversed on / off",
            Pin => "pin / unpin the selected text",
            Physics => "the physics panel",
            Pause => "pause the physics",
            Settle => "settle now",
            SideSum => "gsearch the bar's ordinal sum",
            SideReversedSum => "gsearch the bar's reversed sum",
            Help => "help",
            Hud => "the position card",
            Details => "the details window",
            Export => "export the texts to a file",
            SaveMap => "save the map",
            ReadPrev => "the one before",
            ReadNext => "the next one",
        }
    }

    pub fn default_binding(self) -> Binding {
        let b = |s: &str| Binding::parse(s).expect("a default binding reads");
        b(match self {
            GotoMemory => "Ctrl+1",
            GotoSpace => "Ctrl+2",
            GotoLists => "Ctrl+3",
            GotoReader => "Ctrl+4",
            GotoNotes => "Ctrl+5",
            GotoTags => "Ctrl+6",
            GotoStats => "Ctrl+7",
            GotoModels => "Ctrl+8",
            GotoFilters => "Ctrl+9",
            OpenSettings => "Ctrl+,",
            OpenKeys => "Ctrl+K",
            FocusBar => "/",
            Undo => "Ctrl+Z",
            ZoomIn => "Ctrl+=",
            ZoomOut => "Ctrl+-",
            ZoomReset => "Ctrl+0",
            Save => "Ctrl+S",
            NewNote => "Ctrl+N",
            ToggleRead => "Ctrl+E",
            ToggleList => "Ctrl+B",
            NoteClassifiers => "Ctrl+I",
            ToggleCheck => "Ctrl+J",
            BucketOrdinal => "O",
            BucketReversed => "R",
            BucketFrequency => "F",
            ToggleGsearch => "G",
            ToggleSaturate => "S",
            WhiteMode => "W",
            Letters => "L",
            ClassifierSearch => "C",
            WordTypes => "T",
            SideOrdinal => "Left",
            SideReversed => "Right",
            SideFrequency => "Down",
            FlyForward => "W",
            FlyBack => "S",
            FlyLeft => "A",
            FlyRight => "D",
            FlyUp => "Space",
            FlyDown => "C",
            RollLeft => "Q",
            RollRight => "E",
            Level => "R",
            BackToStart => "Home",
            Slower => "[",
            Faster => "]",
            Walk => "V",
            SuperFlight => "B",
            Grapple => "G",
            Sneak => "Z",
            NewText => "N",
            EditText => "Enter",
            DeleteText => "Delete",
            CopyText => "Ctrl+C",
            Reversed => "F",
            Pin => "U",
            Physics => "O",
            Pause => "P",
            Settle => "K",
            SideSum => "Shift+Left",
            SideReversedSum => "Shift+Right",
            Help => "H",
            Hud => "I",
            Details => "M",
            Export => "X",
            SaveMap => "Shift+X",
            ReadPrev => "Left",
            ReadNext => "Right",
        })
    }

    pub fn from_id(id: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.id() == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    /// the key, by its name in [`KEYS`]
    pub key: &'static str,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// the keys that can be bound: a key event's `code`, and the name shown and kept
const KEYS: &[(&str, &str)] = &[
    ("KeyA", "A"),
    ("KeyB", "B"),
    ("KeyC", "C"),
    ("KeyD", "D"),
    ("KeyE", "E"),
    ("KeyF", "F"),
    ("KeyG", "G"),
    ("KeyH", "H"),
    ("KeyI", "I"),
    ("KeyJ", "J"),
    ("KeyK", "K"),
    ("KeyL", "L"),
    ("KeyM", "M"),
    ("KeyN", "N"),
    ("KeyO", "O"),
    ("KeyP", "P"),
    ("KeyQ", "Q"),
    ("KeyR", "R"),
    ("KeyS", "S"),
    ("KeyT", "T"),
    ("KeyU", "U"),
    ("KeyV", "V"),
    ("KeyW", "W"),
    ("KeyX", "X"),
    ("KeyY", "Y"),
    ("KeyZ", "Z"),
    ("Digit0", "0"),
    ("Digit1", "1"),
    ("Digit2", "2"),
    ("Digit3", "3"),
    ("Digit4", "4"),
    ("Digit5", "5"),
    ("Digit6", "6"),
    ("Digit7", "7"),
    ("Digit8", "8"),
    ("Digit9", "9"),
    ("F1", "F1"),
    ("F2", "F2"),
    ("F3", "F3"),
    ("F4", "F4"),
    ("F5", "F5"),
    ("F6", "F6"),
    ("F7", "F7"),
    ("F8", "F8"),
    ("F9", "F9"),
    ("F10", "F10"),
    ("F11", "F11"),
    ("F12", "F12"),
    ("ArrowLeft", "Left"),
    ("ArrowRight", "Right"),
    ("ArrowUp", "Up"),
    ("ArrowDown", "Down"),
    ("Enter", "Enter"),
    ("NumpadEnter", "Enter"),
    ("Tab", "Tab"),
    ("Space", "Space"),
    ("Backspace", "Backspace"),
    ("Delete", "Delete"),
    ("Insert", "Insert"),
    ("Home", "Home"),
    ("End", "End"),
    ("PageUp", "PageUp"),
    ("PageDown", "PageDown"),
    ("Minus", "-"),
    ("NumpadSubtract", "-"),
    ("Equal", "="),
    ("NumpadAdd", "="),
    ("BracketLeft", "["),
    ("BracketRight", "]"),
    ("Backslash", "\\"),
    ("Semicolon", ";"),
    ("Quote", "'"),
    ("Comma", ","),
    ("Period", "."),
    ("Slash", "/"),
    ("Backquote", "`"),
];

/// a key's name from a key event's `code`
pub fn key_name(code: &str) -> Option<&'static str> {
    KEYS.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

/// a key event's `code` for a key's name (the first, where two codes share a name)
pub fn key_code(name: &str) -> Option<&'static str> {
    KEYS.iter().find(|(_, n)| *n == name).map(|(c, _)| *c)
}

fn name_of(s: &str) -> Option<&'static str> {
    KEYS.iter().find(|(_, n)| n.eq_ignore_ascii_case(s)).map(|(_, n)| *n)
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
        f.write_str(self.key)
    }
}

impl Binding {
    pub fn parse(s: &str) -> Option<Binding> {
        let mut b = Binding {
            key: "A",
            ctrl: false,
            shift: false,
            alt: false,
        };
        let mut rest = s.trim();
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
        b.key = name_of(rest)?;
        Some(b)
    }

    /// a key event as a binding
    pub fn of(code: &str, ctrl: bool, shift: bool, alt: bool) -> Option<Binding> {
        Some(Binding {
            key: key_name(code)?,
            ctrl,
            shift,
            alt,
        })
    }

    /// chords work while typing, plain keys don't
    pub fn is_chord(&self) -> bool {
        self.ctrl || self.alt
    }
}

/// every action's binding: the defaults with the person's changes over them
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap(BTreeMap<Action, Binding>);

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
    /// both can be pressed; returns that action
    pub fn rebind(&mut self, action: Action, binding: Binding) -> Option<Action> {
        let old = self.get(action);
        let other = Action::ALL
            .iter()
            .copied()
            .find(|a| *a != action && a.scope().overlaps(action.scope()) && self.get(*a) == binding);
        if let Some(o) = other {
            self.0.insert(o, old);
        }
        self.0.insert(action, binding);
        other
    }

    /// the person's changes, as kept in the settings file (action id → binding)
    pub fn changes(&self) -> BTreeMap<String, String> {
        Action::ALL
            .iter()
            .filter(|a| self.get(**a) != a.default_binding())
            .map(|a| (a.id().to_string(), self.get(*a).to_string()))
            .collect()
    }

    pub fn from_changes(map: &BTreeMap<String, String>) -> Keymap {
        let mut k = Keymap::default();
        for (id, b) in map {
            if let (Some(a), Some(b)) = (Action::from_id(id), Binding::parse(b)) {
                k.0.insert(a, b);
            }
        }
        k
    }

    /// the action a key press fires in `view`: one of that view's, else a global one
    pub fn action(&self, view: View, b: Binding, typing: bool) -> Option<Action> {
        if typing && !b.is_chord() {
            return None;
        }
        let find = |want: Scope| {
            Action::ALL
                .into_iter()
                .find(|a| a.scope() == want && !a.held() && self.get(*a) == b)
        };
        find(Scope::In(view)).or_else(|| find(Scope::Global))
    }

    /// the bindings the page's key listener should keep from the browser: chords anywhere,
    /// plain keys outside text boxes (as `ctrl,shift,alt,code` strings)
    pub fn taken(&self, view: View) -> Vec<String> {
        Action::ALL
            .into_iter()
            .filter(|a| a.scope() == Scope::Global || a.scope() == Scope::In(view))
            .map(|a| {
                let b = self.get(a);
                format!(
                    "{},{},{},{}",
                    b.ctrl as u8,
                    b.shift as u8,
                    b.alt as u8,
                    key_code(b.key).unwrap_or("")
                )
            })
            .collect()
    }

    /// the space's held keys for space.js: action id → key code
    pub fn flying(&self) -> BTreeMap<&'static str, &'static str> {
        Action::ALL
            .into_iter()
            .filter(|a| a.held() || matches!(a, Level | BackToStart | Slower | Faster | Walk | SuperFlight | Grapple))
            .map(|a| (a.id(), key_code(self.get(a).key).unwrap_or("")))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_roundtrip_and_swap() {
        let mut k = Keymap::default();
        assert_eq!(k.get(Save).to_string(), "Ctrl+S");
        assert_eq!(
            Binding::parse("Ctrl+Shift+Left"),
            Some(Binding {
                key: "Left",
                ctrl: true,
                shift: true,
                alt: false
            })
        );
        assert_eq!(Binding::parse("Nope"), None);
        // taking another action's key swaps them
        let swapped = k.rebind(BucketFrequency, Binding::parse("R").unwrap());
        assert_eq!(swapped, Some(BucketReversed));
        assert_eq!(k.get(BucketReversed).to_string(), "F");
        let back = Keymap::from_changes(&k.changes());
        assert_eq!(back, k);
        assert!(Keymap::default().changes().is_empty());
        // every default is nameable and distinct where both can be pressed
        for (i, a) in Action::ALL.iter().enumerate() {
            assert!(key_code(a.default_binding().key).is_some(), "{a:?}");
            for b in &Action::ALL[i + 1..] {
                assert!(
                    !(a.scope().overlaps(b.scope()) && a.default_binding() == b.default_binding()),
                    "{a:?} and {b:?} share {}",
                    a.default_binding()
                );
                assert_ne!(a.id(), b.id());
            }
        }
        // the space's keys don't push the memory view's out
        k.rebind(Settle, Binding::parse("G").unwrap());
        assert_eq!(k.get(ToggleGsearch).to_string(), "G");
    }

    #[test]
    fn presses_find_their_action() {
        let k = Keymap::default();
        let r = Binding::of("KeyR", false, false, false).unwrap();
        assert_eq!(k.action(View::Memory, r, false), Some(BucketReversed));
        assert_eq!(k.action(View::Space, r, false), Some(Level));
        assert_eq!(k.action(View::Memory, r, true), None, "plain keys wait while typing");
        let s = Binding::of("KeyS", true, false, false).unwrap();
        assert_eq!(k.action(View::Notes, s, true), Some(Save));
        assert_eq!(
            k.action(View::Memory, Binding::of("Digit2", true, false, false).unwrap(), true),
            Some(GotoSpace)
        );
        // held keys are space.js's
        assert_eq!(
            k.action(View::Space, Binding::of("KeyW", false, false, false).unwrap(), false),
            None
        );
        assert_eq!(k.flying()["fly_forward"], "KeyW");
    }
}
