//! The state every view shares (signals in a context), and the small pieces they all use:
//! the status line, toasts, the command bar, picks (the list, its candidate and a
//! selection), the right-click menu, and undo.

use std::rc::Rc;

use dioxus::prelude::*;

use crate::backend::{Backend, Hello, Hit};
use crate::gem::{Measure, Mode};
use crate::prefs::Prefs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    Memory,
    Space,
    Lists,
    Reader,
    Notes,
    Tags,
    Stats,
    Models,
    Filters,
}

impl View {
    pub const ALL: [View; 9] = [
        View::Memory,
        View::Space,
        View::Lists,
        View::Reader,
        View::Notes,
        View::Tags,
        View::Stats,
        View::Models,
        View::Filters,
    ];

    pub fn name(self) -> &'static str {
        match self {
            View::Memory => "memory",
            View::Space => "space",
            View::Lists => "lists",
            View::Reader => "reader",
            View::Notes => "notes",
            View::Tags => "tags",
            View::Stats => "stats",
            View::Models => "models",
            View::Filters => "filters",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            View::Memory => "memory",
            View::Space => "space",
            View::Lists => "list",
            View::Reader => "reader",
            View::Notes => "editor",
            View::Tags => "tags",
            View::Stats => "stats",
            View::Models => "classifier",
            View::Filters => "filter",
        }
    }

    pub fn parse(s: &str) -> View {
        View::ALL.into_iter().find(|v| v.name() == s).unwrap_or(View::Memory)
    }

    /// what the command bar asks for in this view
    pub fn placeholder(self) -> &'static str {
        match self {
            View::Memory => "a number, a text, or a key",
            View::Space => "a number, a text, or a key (Esc to fly)",
            View::Lists => "a text for the chosen list, a list's name, or a new list's (Enter)",
            View::Reader | View::Notes => "words to find notes by",
            View::Tags => "a tag (Enter picks the first match)",
            View::Stats => "",
            View::Models => "a text to try the model on",
            View::Filters => "words, or a sum like 79 or 70-90",
        }
    }
}

/// a key press the shell hands to the open view: from the bar (`bar`), or anywhere else
/// that isn't typing
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct KeyPress {
    pub key: String,
    pub code: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub bar: bool,
    /// a text box (the bar or another) has the focus
    #[serde(default)]
    pub typing: bool,
    pub text: String,
    /// the action the keymap gives this press here (set by the shell)
    #[serde(skip)]
    pub action: Option<crate::keys::Action>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
}

/// the search line in the top bar: what was searched and its triangle
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TopLine {
    pub title: String,
    pub detail: String,
    pub s: u64,
    pub r: u64,
}

/// the list (Shift+click), its candidate (Ctrl+click) and a selection of texts
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Picks {
    pub list: Option<Hit>,
    pub candidate: Option<Hit>,
    pub selected: Vec<Hit>,
}

/// what the next Enter in the bar does instead of a search
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Pending {
    #[default]
    None,
    /// the bar holds the text; Enter puts it back under the same key
    Edit(Hit),
    /// the bar holds a placement's text in the space; Enter makes it that placement's
    /// own text (the entry and its other placements stay as they are)
    EditOwn { node: String, text: String },
    /// the next text typed tags these
    Tag(Vec<Hit>),
}

/// the right-click menu on a text
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub hit: Hit,
    pub x: f64,
    pub y: f64,
}

/// a recent action that can be taken back
#[derive(Debug, Clone, PartialEq)]
pub enum Did {
    Forgot { key: String, text: String },
    Remembered { key: String, text: String },
    Edited { key: String, old: String },
    Enlisted { list: String, member: String },
    Delisted { list: String, member: String },
    Unlisted { list: String, members: Vec<String> },
    Feedback { model: String },
}

impl Did {
    pub fn describe(&self) -> String {
        match self {
            Did::Forgot { text, .. } => format!("forgetting “{text}”"),
            Did::Remembered { text, .. } => format!("remembering “{text}”"),
            Did::Edited { old, .. } => format!("the edit of “{old}”"),
            Did::Enlisted { .. } => "adding to a list".into(),
            Did::Delisted { .. } => "taking out of a list".into(),
            Did::Unlisted { .. } => "deleting a list".into(),
            Did::Feedback { model } => format!("the verdict given to {model}"),
        }
    }
}

/// the bar's own undo: states it was in, coalesced while typing
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarHistory {
    pub back: Vec<String>,
    pub forth: Vec<String>,
    pub last_push: f64,
}

#[derive(Clone)]
pub struct Api(pub Rc<dyn Backend>);

impl PartialEq for Api {
    fn eq(&self, other: &Api) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl std::ops::Deref for Api {
    type Target = dyn Backend + 'static;
    fn deref(&self) -> &(dyn Backend + 'static) {
        &*self.0
    }
}

#[derive(Clone, Copy)]
pub struct Ui {
    pub view: Signal<View>,
    pub prefs: Signal<Prefs>,
    pub status: Signal<(String, bool, u64)>,
    pub toasts: Signal<Vec<Toast>>,
    /// the command bar's text, mirrored on every input (the input itself isn't bound to it,
    /// so typing never waits on a render)
    pub bar: Signal<String>,
    /// the bar's text measured, a beat after typing stops
    pub measure: Signal<Option<Measure>>,
    pub settings: Signal<bool>,
    pub hello: Signal<Hello>,
    pub topline: Signal<Option<TopLine>>,
    pub picks: Signal<Picks>,
    pub pending: Signal<Pending>,
    pub menu: Signal<Option<Menu>>,
    pub actions: Signal<Vec<Did>>,
    pub bar_history: Signal<BarHistory>,
    /// each view's key handler
    pub keys: Signal<Vec<(View, Callback<KeyPress, bool>)>>,
    /// bumped when the save's data changed (views reload)
    pub changed: Signal<u64>,
    /// what the bar held in each view, put back when you return to it
    pub bar_texts: Signal<std::collections::HashMap<View, String>>,
    pub seq: Signal<u64>,
    /// what each view says at the command bar's right end (its counts, what Enter does)
    pub side: Signal<std::collections::HashMap<View, String>>,
    /// the keys menu waits for a key for this action
    pub listening: Signal<Option<crate::keys::Action>>,
    /// a note another view asked the notes view to open
    pub open_note: Signal<Option<String>>,
    /// a tag another view asked the tags view to show
    pub pick_tag: Signal<Option<String>>,
    /// the window is closing (desktop): 1 asked, 2 a view is saving and closes it after
    pub closing: Signal<u8>,
}

pub fn ui() -> Ui {
    use_context::<Ui>()
}

pub fn api() -> Api {
    use_context::<Api>()
}

pub fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0)
    }
}

pub async fn sleep_ms(ms: u64) {
    #[cfg(target_arch = "wasm32")]
    {
        let p = js_sys::Promise::new(&mut |ok, _| {
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&ok, ms as i32);
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(p).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
}

impl Ui {
    /// the keymap: the defaults with the person's changes
    pub fn keymap(self) -> crate::keys::Keymap {
        crate::keys::Keymap::from_changes(&self.prefs.peek().keymap)
    }

    /// what the key bound to an action is called, for tooltips
    pub fn key_of(self, a: crate::keys::Action) -> String {
        self.keymap().get(a).to_string()
    }

    /// what the view says at the command bar's right end
    pub fn side_note(mut self, view: View, text: impl Into<String>) {
        let t = text.into();
        if self.side.peek().get(&view) != Some(&t) {
            self.side.write().insert(view, t);
        }
    }

    fn next(mut self) -> u64 {
        let n = *self.seq.peek() + 1;
        self.seq.set(n);
        n
    }

    /// a message in the top bar; errors stay a while longer
    pub fn say(mut self, msg: impl Into<String>) {
        let n = self.next();
        self.status.set((msg.into(), false, n));
        dioxus::dioxus_core::spawn_forever(async move {
            sleep_ms(4000).await;
            if self.status.peek().2 == n {
                self.status.set((String::new(), false, n));
            }
        });
    }

    pub fn fail(mut self, msg: impl Into<String>) {
        let n = self.next();
        self.status.set((msg.into(), true, n));
        dioxus::dioxus_core::spawn_forever(async move {
            sleep_ms(8000).await;
            if self.status.peek().2 == n {
                self.status.set((String::new(), false, n));
            }
        });
    }

    /// weum's toast: 15 seconds, click puts it in the bar, double click dismisses
    pub fn toast(mut self, text: impl Into<String>) {
        let id = self.next();
        self.toasts.write().push(Toast { id, text: text.into() });
        dioxus::dioxus_core::spawn_forever(async move {
            sleep_ms(15_000).await;
            self.toasts.write().retain(|t| t.id != id);
        });
    }

    /// puts a text in the command bar (and its undo history)
    pub fn set_bar(mut self, text: &str) {
        let old = self.bar.peek().clone();
        if old != text {
            let mut h = self.bar_history.write();
            h.back.push(old);
            h.forth.clear();
            h.last_push = now_ms();
        }
        self.bar.set(text.to_string());
        let js = format!(
            "const b = document.getElementById('bar'); if (b) b.value = {};",
            serde_json::to_string(text).unwrap_or_default()
        );
        document::eval(&js);
    }

    pub fn focus_bar(self) {
        document::eval("const b = document.getElementById('bar'); if (b) { b.focus(); }");
    }

    /// the bar's input, as typed: coalesced into undo states
    pub fn typed(mut self, text: String) {
        let old = self.bar.peek().clone();
        let t = now_ms();
        {
            let mut h = self.bar_history.write();
            // a new undo state after a pause or at a word's end
            if t - h.last_push > 700.0 || old.ends_with(' ') && !text.ends_with(' ') || h.back.is_empty() {
                h.back.push(old);
                if h.back.len() > 200 {
                    h.back.remove(0);
                }
                h.forth.clear();
            }
            h.last_push = t;
        }
        self.bar.set(text);
    }

    pub fn bar_undo(mut self, redo: bool) {
        let cur = self.bar.peek().clone();
        let to = {
            let mut h = self.bar_history.write();
            let t = if redo { h.forth.pop() } else { h.back.pop() };
            if t.is_some() {
                if redo {
                    h.back.push(cur);
                } else {
                    h.forth.push(cur);
                }
                h.last_push = 0.0;
            }
            t
        };
        if let Some(t) = to {
            self.bar.set(t.clone());
            document::eval(&format!(
                "const b = document.getElementById('bar'); if (b) b.value = {};",
                serde_json::to_string(&t).unwrap_or_default()
            ));
        }
    }

    pub fn did(mut self, a: Did) {
        let mut all = self.actions.write();
        all.push(a);
        if all.len() > 50 {
            all.remove(0);
        }
    }

    pub fn touched(mut self) {
        let n = *self.changed.peek() + 1;
        self.changed.set(n);
    }

    pub fn mode(self) -> Mode {
        self.prefs.read().mode
    }

    pub fn save_prefs(self) {
        crate::prefs::store(&self.prefs.peek());
    }

    /// counts and the save's name again
    pub fn refresh(mut self, api: Api) {
        dioxus::dioxus_core::spawn_forever(async move {
            match api.hello().await {
                Ok(h) => self.hello.set(h),
                Err(e) => self.fail(e),
            }
        });
    }
}

/// takes back the latest action
pub fn undo_last(ui: Ui, api: Api) {
    let mut actions = ui.actions;
    let Some(a) = actions.write().pop() else {
        ui.say("nothing to undo");
        return;
    };
    dioxus::dioxus_core::spawn_forever(async move {
        let r = match &a {
            Did::Forgot { key, text } => api.put_memory(key, text).await,
            Did::Remembered { key, .. } => api.forget(key).await.map(|_| ()),
            Did::Edited { key, old } => api.put_memory(key, old).await,
            Did::Enlisted { list, member } => api.delist(list, member).await,
            Did::Delisted { list, member } => api.enlist(list, member).await,
            Did::Unlisted { list, members } => {
                let mut r = Ok(());
                for m in members {
                    r = r.and(api.enlist(list, m).await);
                }
                r
            }
            Did::Feedback { model } => api.undo_model_feedback(model).await,
        };
        match r {
            Ok(()) => {
                ui.say(format!("took back {}", a.describe()));
                ui.touched();
                ui.refresh(api);
            }
            Err(e) => ui.fail(format!("couldn't take back {}: {e}", a.describe())),
        }
    });
}

/// an icon from the shared set
#[component]
pub fn Icon(name: String, #[props(default)] class: String) -> Element {
    rsx! {
        span { class: "icon {class}", "aria-hidden": "true", dangerous_inner_html: crate::icons::svg(&name) }
    }
}

/// the triangle with sides s, r and ⌊√(s·r)⌋, with its angles as the tooltip
#[component]
pub fn Triangle(s: u64, r: u64, #[props(default = 22.0)] height: f64) -> Element {
    let Some((w, h, pts, real)) = crate::gem::triangle_points(s, r, height, height * 3.0) else {
        return rsx! {};
    };
    let title = crate::gem::triangle_title(s, r);
    rsx! {
        svg {
            class: if real { "triangle" } else { "triangle flat" },
            width: "{w:.1}",
            height: "{h:.1}",
            view_box: "0 0 {w:.1} {h:.1}",
            role: "img",
            "aria-label": "{title}",
            title { "{title}" }
            polygon { points: "{pts}", stroke_dasharray: if real { "none" } else { "2 2" } }
        }
    }
}
