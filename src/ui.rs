//! The interface: a nav bar (editor / reader / memory / stats / space plus a message box), the editor
//! (note list on the left, the markdown editor card on the right) and the reader, which
//! shows published notes the way the old site's front page showed writs.

use bevy::ecs::system::SystemParam;
use bevy::feathers::constants::fonts;
use bevy::feathers::controls::{
    ButtonVariant, FeathersButton, FeathersCheckbox, FeathersDialog, FeathersDialogBody, FeathersDialogFooter,
    FeathersDialogHeader, FeathersTextInput, FeathersTextInputContainer,
};
use bevy::feathers::display::caption;
use bevy::feathers::theme::ThemeBackgroundColor;
use bevy::feathers::tokens;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::input_focus::{FocusedInput, InputFocus};
use bevy::picking::Pickable;
use bevy::picking::cursor::EntityCursor;
use bevy::prelude::*;
use bevy::text::{
    EditableText, FontSourceTemplate, Strikethrough, TextBackgroundColor, TextCursorStyle, TextEdit, TextEditChange,
    Underline,
};
use bevy::ui::Checked;
use bevy::ui_widgets::{Activate, RequestClose, ScrollArea, TextInput, ValueChange};
use bevy::window::{SystemCursorIcon, WindowCloseRequested};

use crate::keys::{Action, Fired, Keymap, Remap};
use crate::markdown::{self, Block, Span};
use crate::memory_ui::MemoryView;
use crate::note::{self, Note};
use crate::space_ui::SpaceView;
use crate::store::{Store, sort_newest_first};
use crate::theme::colors;

// ------------------------------------------------------------------ state

#[derive(Resource)]
pub struct Db(pub Store);

/// every stored note, newest first
#[derive(Resource, Default)]
pub struct Notes(pub Vec<Note>);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Component)]
pub enum Route {
    #[default]
    Editor,
    Reader,
    Memory,
    Stats,
    Space,
}

#[derive(Resource, Default)]
pub struct Session {
    pub route: Route,
    /// the note being edited; an empty key means it hasn't been saved yet
    pub active: Note,
    pub view_mode: bool,
    /// the reader's tag filter
    pub reader_tag: Option<String>,
    /// notes matching the note list's search bar, when it has text
    pub note_filter: Option<std::collections::HashSet<String>>,
}

#[derive(Resource, Default)]
pub struct Status {
    pub text: String,
    pub until: f64,
}

/// which parts of the interface need rebuilding
#[derive(Resource)]
pub struct Redraw {
    pub route: bool,
    pub list: bool,
    pub editor_text: bool,
    pub meta: bool,
    pub preview: bool,
    pub reader: bool,
    pub stats: bool,
}

impl Default for Redraw {
    fn default() -> Self {
        Redraw {
            route: true,
            list: true,
            editor_text: true,
            meta: true,
            preview: true,
            reader: true,
            stats: true,
        }
    }
}

#[derive(Resource)]
pub struct Fonts {
    pub regular: Handle<Font>,
    pub bold: Handle<Font>,
    pub italic: Handle<Font>,
    pub bold_italic: Handle<Font>,
    pub mono: Handle<Font>,
}

impl FromWorld for Fonts {
    fn from_world(world: &mut World) -> Self {
        let assets = world.resource::<AssetServer>();
        Fonts {
            regular: assets.load(fonts::REGULAR),
            bold: assets.load(fonts::BOLD),
            italic: assets.load(fonts::ITALIC),
            bold_italic: assets.load(fonts::BOLD_ITALIC),
            mono: assets.load(fonts::MONO),
        }
    }
}

/// everything the note actions need
#[derive(SystemParam)]
pub struct Ctx<'w> {
    pub db: Res<'w, Db>,
    pub notes: ResMut<'w, Notes>,
    pub session: ResMut<'w, Session>,
    pub status: ResMut<'w, Status>,
    pub redraw: ResMut<'w, Redraw>,
    pub time: Res<'w, Time>,
}

/// a note as the editor would show it, so stored and edited versions compare fairly
fn normalized(n: &Note) -> (String, String, Vec<String>, bool) {
    let (title, markdown) = note::split(&n.editor_text());
    (title, markdown, n.tags.clone(), n.published)
}

impl Ctx<'_> {
    pub fn say(&mut self, msg: impl Into<String>) {
        self.status.text = msg.into();
        self.status.until = self.time.elapsed_secs_f64() + 5.0;
    }

    fn stored(&self, key: &str) -> Option<&Note> {
        self.notes.0.iter().find(|n| n.key == key)
    }

    pub fn is_dirty(&self) -> bool {
        let a = &self.session.active;
        match self.stored(&a.key) {
            Some(stored) => normalized(stored) != normalized(a),
            None => !a.title.is_empty() || !a.markdown.is_empty() || !a.tags.is_empty(),
        }
    }

    pub fn save(&mut self) -> bool {
        if self.session.active.title.trim().is_empty() {
            self.say("a note needs a '# title' line before it can be saved");
            return false;
        }
        match self.db.0.save(&self.session.active) {
            Ok(saved) => {
                match self.notes.0.iter_mut().find(|n| n.key == saved.key) {
                    Some(n) => *n = saved.clone(),
                    None => self.notes.0.push(saved.clone()),
                }
                sort_newest_first(&mut self.notes.0);
                self.session.active = saved;
                self.say(format!("saved \"{}\"", self.session.active.title));
                self.redraw.list = true;
                self.redraw.reader = true;
                self.redraw.stats = true;
                true
            }
            Err(e) => {
                self.say(format!("could not save: {}", e));
                false
            }
        }
    }

    /// saves pending changes before leaving the active note; false means stay
    fn leave_active(&mut self) -> bool {
        if !self.is_dirty() {
            return true;
        }
        if self.session.active.title.trim().is_empty() {
            self.say("give this note a '# title' first, or delete it");
            return false;
        }
        self.save()
    }

    fn show(&mut self, note: Note) {
        self.session.active = note;
        self.session.route = Route::Editor;
        self.redraw.route = true;
        self.redraw.list = true;
        self.redraw.editor_text = true;
        self.redraw.meta = true;
        self.redraw.preview = true;
    }

    pub fn open(&mut self, key: &str) {
        if key == self.session.active.key && self.session.route == Route::Editor {
            return;
        }
        if !self.leave_active() {
            return;
        }
        if let Some(n) = self.stored(key).cloned() {
            self.show(n);
        }
    }

    pub fn new_note(&mut self) {
        if self.leave_active() {
            self.session.view_mode = false;
            self.show(Note::default());
        }
    }

    pub fn delete_active(&mut self) {
        let key = self.session.active.key.clone();
        if !key.is_empty() {
            if let Err(e) = self.db.0.delete(&key) {
                self.say(format!("could not delete: {}", e));
                return;
            }
            self.notes.0.retain(|n| n.key != key);
            self.say(format!("deleted \"{}\"", self.session.active.display_title()));
        }
        self.show(Note::default());
        self.redraw.reader = true;
        self.redraw.stats = true;
    }

    pub fn toggle_publish(&mut self) {
        self.session.active.published = !self.session.active.published;
        if !self.save() {
            self.session.active.published = !self.session.active.published;
        }
        self.redraw.meta = true;
    }

    fn set_tags(&mut self, tags: Vec<String>) {
        self.session.active.tags = tags;
        self.redraw.meta = true;
        self.redraw.list = true;
    }
}

// ------------------------------------------------------------------ markers

#[derive(Component, Default, Clone)]
pub struct StatusText;
#[derive(Component, Default, Clone)]
pub struct NavTab(pub Route);
#[derive(Component, Default, Clone)]
pub struct EditorView;
#[derive(Component, Default, Clone)]
pub struct ReaderView;
#[derive(Component, Default, Clone)]
pub struct StatsView;
#[derive(Component, Default, Clone)]
pub struct NoteList;
#[derive(Component, Default, Clone)]
pub struct TagChips;
#[derive(Component, Default, Clone)]
pub struct TagInput;
#[derive(Component, Default, Clone)]
pub struct PublishBox;
#[derive(Component, Default, Clone)]
pub struct ViewToggle;
#[derive(Component, Default, Clone)]
pub struct EditorPad;
#[derive(Component, Default, Clone)]
pub struct Preview;
/// the top bar
#[derive(Component, Default, Clone)]
pub struct NavBar;
#[derive(Component, Default, Clone)]
pub struct RemapMenu;
#[derive(Component, Default, Clone)]
pub struct RemapList;
/// the window's root node; the space clears its background to show the 3D view
#[derive(Component, Default, Clone)]
pub struct AppRoot;

// ------------------------------------------------------------------ layout

pub fn scene() -> impl SceneList {
    bsn_list! {
        Camera2d
        IsDefaultUiCamera
        --
        @root()
    }
}

fn root() -> impl Scene {
    bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(6)),
            row_gap: px(6),
        }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        TabGroup
        AppRoot
        Pickable::IGNORE
        Children [
            @nav()
            --
            Node {
                flex_grow: 1.0,
                min_height: px(0),
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
            }
            Pickable::IGNORE
            Children [
                @editor_view()
                --
                @reader_view()
                --
                @crate::memory_ui::view()
                --
                @stats_view()
                --
                @crate::space_ui::view()
            ]
            --
            @crate::bar::bar()
            --
            @crate::settings::menu()
            --
            @remap_menu()
        ]
    }
}

fn nav_button(label: &'static str, route: Route) -> impl Scene {
    bsn! {
        @FeathersButton {
            @caption: bsn! { @caption(label) }
        }
        NavTab(route)
        on(|activate: On<Activate>, tabs: Query<&NavTab>, mut ctx: Ctx| {
            if let Ok(tab) = tabs.get(activate.event_target()) {
                // the reader and stats read stored notes, so save titled changes first;
                // an untitled draft just waits in the editor
                if tab.0 != Route::Editor && ctx.is_dirty() && !ctx.session.active.title.trim().is_empty() {
                    ctx.save();
                }
                ctx.session.route = tab.0;
                ctx.redraw.route = true;
            }
        })
    }
}

fn nav() -> impl Scene {
    bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(6),
            padding: UiRect::all(px(4)),
            border_radius: BorderRadius::all(px(4)),
        }
        BackgroundColor(colors::NAV)
        NavBar
        crate::zen::Chrome
        Children [
            @nav_button("editor", Route::Editor)
            --
            @nav_button("reader", Route::Reader)
            --
            @nav_button("memory", Route::Memory)
            --
            @nav_button("stats", Route::Stats)
            --
            @nav_button("space", Route::Space)
            --
            Node {
                flex_grow: 1.0,
                min_height: px(24),
                display: Display::Flex,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::End,
                padding: UiRect::axes(px(8), px(3)),
            }
            Children [
                Text("")
                StatusText
                TextFont {
                    font: FontSourceTemplate::Handle(fonts::REGULAR),
                    font_size: FontSize::Px(13.0),
                }
                TextColor({crate::bar::BAR_TEXT})
            ]
            --
            @FeathersButton {
                @caption: bsn! { @caption("settings") }
            }
            on(|_activate: On<Activate>, mut settings: ResMut<crate::settings::Settings>| settings.toggle())
        ]
    }
}

fn tool_button(label: &'static str) -> impl Scene {
    bsn! {
        @FeathersButton {
            @caption: bsn! { @caption(label) }
        }
    }
}

fn editor_view() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            column_gap: px(8),
        }
        EditorView
        Children [
            // the note list
            Node {
                width: px(240),
                min_height: px(0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(6)),
                border_radius: BorderRadius::all(px(4)),
            }
            BackgroundColor(colors::CARD)
            crate::zen::Chrome
            Children [
                @FeathersButton {
                    @caption: bsn! { @caption("+ new") },
                    @variant: ButtonVariant::Primary,
                }
                on(|_activate: On<Activate>, mut ctx: Ctx| ctx.new_note())
                --
                Node {
                    flex_grow: 1.0,
                    min_height: px(0),
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    overflow: Overflow::scroll_y(),
                }
                ScrollArea
                NoteList
            ]
            --
            // the editor card
            Node {
                flex_grow: 1.0,
                min_width: px(280),
                min_height: px(0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(8)),
                border_radius: BorderRadius::all(px(4)),
            }
            BackgroundColor(colors::CARD)
            Children [
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                }
                crate::zen::Chrome
                Children [
                    Node {
                        flex_grow: 1.0,
                        display: Display::Flex,
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        align_items: AlignItems::Center,
                        column_gap: px(4),
                        row_gap: px(4),
                    }
                    TagChips
                    --
                    @FeathersTextInputContainer
                    Node { flex_grow: 0.0 }
                    Children [
                        @FeathersTextInput {
                            @visible_width: 12f32,
                        }
                        TagInput
                        on(tag_input_key)
                    ]
                    --
                    @FeathersCheckbox {
                        @caption: bsn! { @caption("published") }
                    }
                    PublishBox
                    on(|_change: On<ValueChange<bool>>, mut ctx: Ctx| ctx.toggle_publish())
                    --
                    @tool_button("view")
                    ViewToggle
                    on(|_activate: On<Activate>, mut ctx: Ctx| {
                        ctx.session.view_mode = !ctx.session.view_mode;
                        ctx.redraw.meta = true;
                        ctx.redraw.preview = true;
                    })
                    --
                    @tool_button("save")
                    on(|_activate: On<Activate>, mut ctx: Ctx| { ctx.save(); })
                    --
                    @tool_button("delete")
                    on(confirm_delete)
                ]
                --
                // the writing pad
                Node {
                    flex_grow: 1.0,
                    min_height: px(0),
                    padding: UiRect::all(px(10)),
                    border_radius: BorderRadius::all(px(5)),
                }
                BackgroundColor(colors::PAD)
                TextInput
                EditableText {
                    allow_newlines: true,
                    visible_lines: {None},
                    cursor_width: 0.15,
                }
                TextLayout {
                    linebreak: LineBreak::WordOrCharacter,
                }
                TextFont {
                    font: FontSourceTemplate::Handle(fonts::MONO),
                    font_size: FontSize::Px(15.0),
                }
                TextColor(colors::TEXT)
                TextCursorStyle
                TabIndex(0)
                EntityCursor::System(SystemCursorIcon::Text)
                EditorPad
                on(pad_changed)
                --
                // the rendered view
                Node {
                    flex_grow: 1.0,
                    min_height: px(0),
                    display: Display::None,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(8),
                    padding: UiRect::all(px(10)),
                    overflow: Overflow::scroll_y(),
                }
                ScrollArea
                Preview
            ]
        ]
    }
}

fn reader_view() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            display: Display::None,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: px(10),
            overflow: Overflow::scroll_y(),
        }
        ScrollArea
        ReaderView
    }
}

fn stats_view() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            display: Display::None,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            overflow: Overflow::scroll_y(),
        }
        ScrollArea
        StatsView
    }
}

// ------------------------------------------------------------------ observers

fn pad_changed(change: On<TextEditChange>, pads: Query<&EditableText, With<EditorPad>>, mut ctx: Ctx) {
    let Ok(pad) = pads.get(change.event_target()) else {
        return;
    };
    let (title, markdown) = note::split(&pad.value().to_string());
    let active = &ctx.session.active;
    if active.title == title && active.markdown == markdown {
        return;
    }
    let title_changed = active.title != title;
    ctx.session.active.title = title;
    ctx.session.active.markdown = markdown;
    // the list shows the live title and an unsaved marker
    ctx.redraw.list |= title_changed || ctx.is_dirty();
    ctx.redraw.preview |= ctx.session.view_mode;
}

fn tag_input_key(
    key: On<FocusedInput<KeyboardInput>>,
    mut inputs: Query<&mut EditableText, With<TagInput>>,
    mut ctx: Ctx,
) {
    if !key.input.state.is_pressed() || !matches!(key.input.logical_key, Key::Enter | Key::Character(_)) {
        return;
    }
    let is_comma = matches!(&key.input.logical_key, Key::Character(c) if c.as_str() == ",");
    if key.input.logical_key != Key::Enter && !is_comma {
        return;
    }
    let Ok(mut input) = inputs.get_mut(key.focused_entity) else {
        return;
    };
    let typed = input.value().to_string();
    if let Some(tag) = note::normalize_tag(&typed) {
        let mut tags = ctx.session.active.tags.clone();
        if !tags.contains(&tag) {
            tags.push(tag);
            ctx.set_tags(tags);
        }
    }
    input.queue_edit(TextEdit::SelectAll);
    input.queue_edit(TextEdit::Backspace);
}

fn confirm_delete(activate: On<Activate>, mut commands: Commands, ctx: Ctx) {
    let title = ctx.session.active.display_title().to_string();
    let saved = !ctx.session.active.key.is_empty();
    let question = if saved {
        format!("Delete \"{}\"? This cannot be undone.", title)
    } else {
        "Discard this unsaved note?".to_string()
    };
    commands
        .entity(activate.event_target())
        .queue_spawn_related_scenes::<Children>(bsn_list! {
            @FeathersDialog {
                @width: px(340),
                @contents: bsn_list! {
                    @FeathersDialogHeader Children [ @caption("delete note") ]
                    --
                    @FeathersDialogBody Children [ @caption(question) ]
                    --
                    @FeathersDialogFooter Children [
                        @FeathersButton { @caption: bsn! { @caption("cancel") } }
                        on(|activate: On<Activate>, mut commands: Commands| {
                            commands.trigger(RequestClose { source: activate.event_target() });
                        })
                        --
                        @FeathersButton {
                            @caption: bsn! { @caption("delete") },
                            @variant: ButtonVariant::Primary,
                        }
                        on(|activate: On<Activate>, mut commands: Commands, mut ctx: Ctx| {
                            ctx.delete_active();
                            commands.trigger(RequestClose { source: activate.event_target() });
                        })
                    ]
                }
            }
            on(|close: On<RequestClose>, mut commands: Commands| {
                commands.entity(close.event_target()).despawn();
            })
        });
}

// ------------------------------------------------------------------ systems

/// the editor's and the app's own key actions (the memory view handles its own)
pub fn app_actions(
    mut fired: MessageReader<Fired>,
    mut ctx: Ctx,
    mut remap: ResMut<Remap>,
    mut settings: ResMut<crate::settings::Settings>,
    bar: Single<Entity, With<crate::bar::Bar>>,
    mut focus: ResMut<InputFocus>,
) {
    for Fired(action) in fired.read() {
        match action {
            Action::OpenSettings => settings.toggle(),
            Action::FocusTerm if ctx.session.route != Route::Memory => crate::bar::focus_bar(&mut focus, *bar),
            Action::Save => {
                ctx.save();
            }
            Action::NewNote => ctx.new_note(),
            Action::ToggleView if ctx.session.route == Route::Editor => {
                ctx.session.view_mode = !ctx.session.view_mode;
                ctx.redraw.meta = true;
                ctx.redraw.preview = true;
            }
            Action::GotoEditor | Action::GotoReader | Action::GotoMemory | Action::GotoStats | Action::GotoSpace => {
                let route = match action {
                    Action::GotoEditor => Route::Editor,
                    Action::GotoReader => Route::Reader,
                    Action::GotoMemory => Route::Memory,
                    Action::GotoSpace => Route::Space,
                    _ => Route::Stats,
                };
                if route != Route::Editor && ctx.is_dirty() && !ctx.session.active.title.trim().is_empty() {
                    ctx.save();
                }
                ctx.session.route = route;
                ctx.redraw.route = true;
            }
            Action::OpenRemap => {
                remap.open = !remap.open;
                remap.changed = true;
            }
            _ => {}
        }
    }
}

pub fn status_text(mut status: ResMut<Status>, time: Res<Time>, mut text: Single<&mut Text, With<StatusText>>) {
    if !status.text.is_empty() && time.elapsed_secs_f64() > status.until {
        status.text.clear();
    }
    if text.0 != status.text {
        text.0 = status.text.clone();
    }
}

/// saves the open note when the window is closed
pub fn on_close(mut closes: MessageReader<WindowCloseRequested>, mut ctx: Ctx, mut commands: Commands) {
    for close in closes.read() {
        if ctx.is_dirty() && !ctx.session.active.title.trim().is_empty() {
            ctx.save();
        }
        commands.entity(close.window).despawn();
    }
}

pub fn apply_route(
    mut redraw: ResMut<Redraw>,
    session: Res<Session>,
    mut views: Query<(
        &mut Node,
        Has<EditorView>,
        Has<ReaderView>,
        Has<MemoryView>,
        Has<StatsView>,
        Has<SpaceView>,
    )>,
    tabs: Query<(Entity, &NavTab)>,
    mut commands: Commands,
) {
    if !redraw.route {
        return;
    }
    redraw.route = false;
    for (mut node, editor, reader, memory, stats, space) in views.iter_mut() {
        let shown = match session.route {
            Route::Editor => editor,
            Route::Reader => reader,
            Route::Memory => memory,
            Route::Stats => stats,
            Route::Space => space,
        };
        if editor || reader || memory || stats || space {
            node.display = if shown { Display::Flex } else { Display::None };
        }
    }
    for (entity, tab) in tabs.iter() {
        commands
            .entity(entity)
            .insert(ButtonVariant::selected(tab.0 == session.route));
    }
}

pub(crate) fn plain_scene_button(label: String, variant: ButtonVariant) -> impl Scene {
    bsn! {
        @FeathersButton {
            @caption: bsn! { @caption(label) },
            @variant: variant,
        }
        Node {
            justify_content: JustifyContent::Start,
            flex_shrink: 0.0,
        }
    }
}

pub fn rebuild_list(list: Single<Entity, With<NoteList>>, mut ctx: Ctx, mut commands: Commands) {
    if !ctx.redraw.list {
        return;
    }
    ctx.redraw.list = false;
    commands.entity(*list).despawn_related::<Children>();
    let active = &ctx.session.active;
    let dirty = ctx.is_dirty();

    // a new, unsaved note gets a row of its own at the top
    if active.key.is_empty() {
        let label = format!("{}{}", active.display_title(), if dirty { " •" } else { "" });
        commands
            .spawn_scene(plain_scene_button(label, ButtonVariant::Primary))
            .insert(ChildOf(*list));
    }
    for n in ctx.notes.0.iter() {
        let is_active = n.key == active.key;
        if !is_active && ctx.session.note_filter.as_ref().is_some_and(|f| !f.contains(&n.key)) {
            continue;
        }
        let title = if is_active {
            active.display_title()
        } else {
            n.display_title()
        };
        let label = format!("{}{}", title, if is_active && dirty { " •" } else { "" });
        let variant = if is_active {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Plain
        };
        let key = n.key.clone();
        commands
            .spawn_scene(plain_scene_button(label, variant))
            .insert(ChildOf(*list))
            .observe(move |_activate: On<Activate>, mut ctx: Ctx| ctx.open(&key));
    }
}

pub fn load_editor_text(
    mut redraw: ResMut<Redraw>,
    session: Res<Session>,
    mut pad: Single<&mut EditableText, With<EditorPad>>,
) {
    if !redraw.editor_text {
        return;
    }
    redraw.editor_text = false;
    pad.editor_mut().set_text(&session.active.editor_text());
    pad.queue_edit(TextEdit::TextStart(false));
}

pub fn rebuild_meta(
    mut redraw: ResMut<Redraw>,
    session: Res<Session>,
    chips: Single<Entity, With<TagChips>>,
    publish: Single<(Entity, Has<Checked>), With<PublishBox>>,
    toggle: Single<&Children, With<ViewToggle>>,
    mut texts: Query<&mut Text>,
    mut commands: Commands,
) {
    if !redraw.meta {
        return;
    }
    redraw.meta = false;

    let (publish, checked) = *publish;
    if session.active.published != checked {
        if session.active.published {
            commands.entity(publish).insert(Checked);
        } else {
            commands.entity(publish).remove::<Checked>();
        }
    }
    for child in toggle.iter() {
        if let Ok(mut t) = texts.get_mut(child) {
            t.0 = if session.view_mode {
                "edit".into()
            } else {
                "view".into()
            };
        }
    }

    commands.entity(*chips).despawn_related::<Children>();
    for tag in session.active.tags.iter() {
        let t = tag.clone();
        commands
            .spawn_scene(plain_scene_button(format!("{}  ×", tag), ButtonVariant::Normal))
            .insert(ChildOf(*chips))
            .observe(move |_activate: On<Activate>, mut ctx: Ctx| {
                let tags = ctx.session.active.tags.iter().filter(|x| **x != t).cloned().collect();
                ctx.set_tags(tags);
            });
    }
}

pub fn rebuild_preview(
    mut redraw: ResMut<Redraw>,
    session: Res<Session>,
    preview: Single<(Entity, &mut Node), (With<Preview>, Without<EditorPad>)>,
    mut pad: Single<&mut Node, With<EditorPad>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
    mut focus: ResMut<InputFocus>,
) {
    if !redraw.preview {
        return;
    }
    redraw.preview = false;
    let (preview, mut preview_node) = preview.into_inner();
    pad.display = if session.view_mode {
        Display::None
    } else {
        Display::Flex
    };
    preview_node.display = if session.view_mode {
        Display::Flex
    } else {
        Display::None
    };
    commands.entity(preview).despawn_related::<Children>();
    if session.view_mode {
        focus.clear();
        spawn_markdown(
            &mut commands,
            preview,
            &note::compose(&session.active.title, &session.active.markdown),
            &fonts,
        );
    }
}

pub fn rebuild_reader(
    mut redraw: ResMut<Redraw>,
    reader: Single<Entity, With<ReaderView>>,
    notes: Res<Notes>,
    session: Res<Session>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if !redraw.reader {
        return;
    }
    redraw.reader = false;
    commands.entity(*reader).despawn_related::<Children>();

    let published: Vec<&Note> = notes
        .0
        .iter()
        .filter(|n| n.published)
        .filter(|n| session.reader_tag.as_ref().is_none_or(|t| n.tags.contains(t)))
        .filter(|n| session.note_filter.as_ref().is_none_or(|f| f.contains(&n.key)))
        .collect();

    if let Some(tag) = &session.reader_tag {
        let bar = commands
            .spawn((card_node(), BackgroundColor(colors::CARD), ChildOf(*reader)))
            .id();
        let row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                    ..default()
                },
                ChildOf(bar),
            ))
            .id();
        text(
            &mut commands,
            row,
            &format!("notes tagged \"{}\"", tag),
            &fonts.regular,
            15.0,
            colors::TEXT,
        );
        commands
            .spawn_scene(plain_scene_button("show all".into(), ButtonVariant::Normal))
            .insert(ChildOf(row))
            .observe(|_activate: On<Activate>, mut ctx: Ctx| {
                ctx.session.reader_tag = None;
                ctx.redraw.reader = true;
            });
    }

    if published.is_empty() {
        let card = commands
            .spawn((card_node(), BackgroundColor(colors::CARD), ChildOf(*reader)))
            .id();
        text(
            &mut commands,
            card,
            "Nothing published yet. Tick \"published\" on a note in the editor and it shows up here.",
            &fonts.italic,
            15.0,
            colors::TEXT_DIM,
        );
        return;
    }

    for n in published {
        let card = commands
            .spawn((card_node(), BackgroundColor(colors::CARD), ChildOf(*reader)))
            .id();

        let title_row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                    ..default()
                },
                ChildOf(card),
            ))
            .id();
        text(
            &mut commands,
            title_row,
            n.display_title(),
            &fonts.bold,
            24.0,
            colors::TEXT,
        );
        let key = n.key.clone();
        commands
            .spawn_scene(plain_scene_button("edit".into(), ButtonVariant::Plain))
            .insert(ChildOf(title_row))
            .observe(move |_activate: On<Activate>, mut ctx: Ctx| ctx.open(&key));

        let mut meta = n.created_at().map(|t| note::long_date(&t)).unwrap_or_default();
        if !n.edits.is_empty()
            && let Some(m) = n.modified_at()
        {
            meta.push_str(&format!("  ·  edited {}", note::relative(&m)));
        }
        text(&mut commands, card, &meta, &fonts.regular, 13.0, colors::TEXT_DIM);

        if !n.tags.is_empty() {
            let tags = commands
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: px(4),
                        row_gap: px(4),
                        ..default()
                    },
                    ChildOf(card),
                ))
                .id();
            for tag in &n.tags {
                let t = tag.clone();
                commands
                    .spawn_scene(plain_scene_button(format!("#{}", tag), ButtonVariant::Normal))
                    .insert(ChildOf(tags))
                    .observe(move |_activate: On<Activate>, mut ctx: Ctx| {
                        ctx.session.reader_tag = Some(t.clone());
                        ctx.redraw.reader = true;
                    });
            }
        }

        let body = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(8),
                    ..default()
                },
                ChildOf(card),
            ))
            .id();
        spawn_markdown(&mut commands, body, &n.markdown, &fonts);
    }
}

pub fn rebuild_stats(
    mut redraw: ResMut<Redraw>,
    stats: Single<Entity, With<StatsView>>,
    notes: Res<Notes>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if !redraw.stats {
        return;
    }
    redraw.stats = false;
    commands.entity(*stats).despawn_related::<Children>();
    let card = commands
        .spawn((card_node(), BackgroundColor(colors::CARD), ChildOf(*stats)))
        .id();

    let total = notes.0.len();
    let published = notes.0.iter().filter(|n| n.published).count();
    let words: usize = notes.0.iter().map(|n| n.markdown.split_whitespace().count()).sum();
    let edits: usize = notes.0.iter().map(|n| n.edits.len()).sum();
    let mut tags: Vec<(String, usize)> = Vec::new();
    for n in notes.0.iter() {
        for t in &n.tags {
            match tags.iter_mut().find(|(x, _)| x == t) {
                Some((_, c)) => *c += 1,
                None => tags.push((t.clone(), 1)),
            }
        }
    }
    tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    text(&mut commands, card, "stats", &fonts.bold, 24.0, colors::TEXT);
    for line in [
        format!(
            "{} {}, {} published, {} {}",
            total,
            plural(total, "note"),
            published,
            total - published,
            plural(total - published, "draft")
        ),
        format!("{} words written across {} saves", words, total + edits),
        match (
            notes.0.last().and_then(|n| n.created_at()),
            notes.0.first().and_then(|n| n.created_at()),
        ) {
            (Some(first), Some(last)) => format!(
                "first note {}, latest {}",
                note::long_date(&first),
                note::relative(&last)
            ),
            _ => "no notes yet, press \"+ new\" in the editor".into(),
        },
    ] {
        text(&mut commands, card, &line, &fonts.regular, 15.0, colors::TEXT);
    }
    if !tags.is_empty() {
        text(&mut commands, card, "tags", &fonts.bold, 18.0, colors::TEXT);
        let list = tags
            .iter()
            .map(|(t, c)| format!("#{} ({})", t, c))
            .collect::<Vec<_>>()
            .join("   ");
        text(&mut commands, card, &list, &fonts.regular, 15.0, colors::TEXT);
    }
}

// ------------------------------------------------------------------ markdown rendering

fn card_node() -> Node {
    Node {
        width: percent(100),
        max_width: px(820),
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Column,
        row_gap: px(8),
        padding: UiRect::all(px(16)),
        border_radius: BorderRadius::all(px(4)),
        ..default()
    }
}

pub(crate) fn text(
    commands: &mut Commands,
    parent: Entity,
    s: &str,
    font: &Handle<Font>,
    size: f32,
    color: Color,
) -> Entity {
    commands
        .spawn((
            Text::new(s),
            TextFont {
                font: font.into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
            ChildOf(parent),
        ))
        .id()
}

fn span_font<'a>(s: &Span, fonts: &'a Fonts, bold_base: bool) -> &'a Handle<Font> {
    if s.code {
        return &fonts.mono;
    }
    match (s.bold || bold_base, s.italic) {
        (true, true) => &fonts.bold_italic,
        (true, false) => &fonts.bold,
        (false, true) => &fonts.italic,
        (false, false) => &fonts.regular,
    }
}

/// a block of styled text runs
fn rich_text(
    commands: &mut Commands,
    parent: Entity,
    spans: &[Span],
    fonts: &Fonts,
    size: f32,
    bold: bool,
    color: Color,
) -> Entity {
    let root = commands
        .spawn((
            Text::default(),
            TextFont {
                font: (&fonts.regular).into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
            TextLayout {
                linebreak: LineBreak::WordBoundary,
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    for s in spans {
        let font_size = if s.code { size * 0.9 } else { size };
        let mut span = commands.spawn((
            TextSpan::new(s.text.clone()),
            TextFont {
                font: span_font(s, fonts, bold).into(),
                font_size: FontSize::Px(font_size),
                ..default()
            },
            TextColor(if s.link { colors::LINK } else { color }),
            ChildOf(root),
        ));
        if s.code {
            span.insert(TextBackgroundColor(colors::CODE_BG));
        }
        if s.strike {
            span.insert(Strikethrough);
        }
        if s.link {
            span.insert(Underline);
        }
    }
    root
}

pub fn spawn_markdown(commands: &mut Commands, parent: Entity, md: &str, fonts: &Fonts) {
    let mut table: Option<Entity> = None;
    for block in markdown::parse(md) {
        if !matches!(block, Block::TableRow { .. }) {
            table = None;
        }
        match block {
            Block::Heading { level, spans } => {
                let size = [28.0, 23.0, 19.0, 17.0, 15.0, 14.0][(level as usize - 1).min(5)];
                let e = rich_text(commands, parent, &spans, fonts, size, true, colors::TEXT);
                commands.entity(e).insert(Node {
                    margin: UiRect::top(px(if level <= 2 { 6.0 } else { 2.0 })),
                    ..default()
                });
            }
            Block::Paragraph { spans, quote } => {
                if quote {
                    let q = quote_box(commands, parent);
                    rich_text(commands, q, &spans, fonts, 15.0, false, colors::TEXT_DIM);
                } else {
                    rich_text(commands, parent, &spans, fonts, 15.0, false, colors::TEXT);
                }
            }
            Block::Item {
                depth,
                marker,
                spans,
                quote,
            } => {
                let container = if quote { quote_box(commands, parent) } else { parent };
                let row = commands
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: px(6),
                            padding: UiRect::left(px(18.0 * depth as f32 + 6.0)),
                            ..default()
                        },
                        ChildOf(container),
                    ))
                    .id();
                let m = text(commands, row, &marker, &fonts.bold, 15.0, colors::MOOD_DARK);
                commands.entity(m).insert(Node {
                    min_width: px(16),
                    ..default()
                });
                let t = rich_text(commands, row, &spans, fonts, 15.0, false, colors::TEXT);
                commands.entity(t).insert(Node {
                    flex_shrink: 1.0,
                    ..default()
                });
            }
            Block::Code(code) => {
                let b = commands
                    .spawn((
                        Node {
                            padding: UiRect::all(px(10)),
                            border_radius: BorderRadius::all(px(4)),
                            ..default()
                        },
                        BackgroundColor(colors::CODE_BG),
                        ChildOf(parent),
                    ))
                    .id();
                text(commands, b, &code, &fonts.mono, 14.0, colors::TEXT);
            }
            Block::Rule => {
                commands.spawn((
                    Node {
                        height: px(1),
                        width: percent(100),
                        ..default()
                    },
                    BackgroundColor(colors::MOOD),
                    ChildOf(parent),
                ));
            }
            Block::TableRow { cells, header } => {
                let t = *table.get_or_insert_with(|| {
                    commands
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                border: UiRect::all(px(1)),
                                ..default()
                            },
                            BorderColor::all(colors::MOOD),
                            ChildOf(parent),
                        ))
                        .id()
                });
                let row = commands
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            border: UiRect::bottom(px(1)),
                            ..default()
                        },
                        BorderColor::all(colors::MOOD),
                        ChildOf(t),
                    ))
                    .id();
                if header {
                    commands.entity(row).insert(BackgroundColor(colors::PAD));
                }
                for cell in cells {
                    let c = commands
                        .spawn((
                            Node {
                                flex_grow: 1.0,
                                flex_basis: px(0),
                                padding: UiRect::axes(px(8), px(4)),
                                ..default()
                            },
                            ChildOf(row),
                        ))
                        .id();
                    rich_text(commands, c, &cell, fonts, 14.0, header, colors::TEXT);
                }
            }
        }
    }
}

fn quote_box(commands: &mut Commands, parent: Entity) -> Entity {
    commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                border: UiRect::left(px(4)),
                padding: UiRect::left(px(10)),
                ..default()
            },
            BorderColor::all(colors::MOOD),
            ChildOf(parent),
        ))
        .id()
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { word.to_string() } else { format!("{}s", word) }
}

// ------------------------------------------------------------------ key remap menu

fn remap_menu() -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            display: Display::None,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        }
        BackgroundColor({Color::BLACK.with_alpha(0.55)})
        GlobalZIndex(10)
        RemapMenu
        Children [
            Node {
                width: px(560),
                max_height: percent(90),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(14)),
                border_radius: BorderRadius::all(px(4)),
                overflow: Overflow::scroll_y(),
            }
            BackgroundColor(colors::CARD)
            ScrollArea
            RemapList
        ]
    }
}

/// shows the remap menu and lists every action with its binding; clicking a binding waits
/// for the next key (Escape cancels), taking a key from another action swaps the two
pub fn rebuild_remap(
    mut remap: ResMut<Remap>,
    keymap: Res<Keymap>,
    mut menu: Single<&mut Node, With<RemapMenu>>,
    list: Single<Entity, With<RemapList>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if !remap.changed && !keymap.is_changed() {
        return;
    }
    remap.changed = false;
    menu.display = if remap.open { Display::Flex } else { Display::None };
    commands.entity(*list).despawn_related::<Children>();
    if !remap.open {
        return;
    }
    text(&mut commands, *list, "keys", &fonts.bold, 22.0, colors::TEXT);
    text(
        &mut commands,
        *list,
        "click a binding, then press the new key (with Ctrl / Alt / Shift if you like). Escape cancels. \
         Keys without Ctrl or Alt only act while you're not typing in a text box.",
        &fonts.regular,
        13.0,
        colors::TEXT_DIM,
    );
    for action in Action::ALL {
        let row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                    ..default()
                },
                ChildOf(*list),
            ))
            .id();
        let label = text(&mut commands, row, action.label(), &fonts.regular, 14.0, colors::TEXT);
        commands.entity(label).insert(Node {
            flex_grow: 1.0,
            ..default()
        });
        let (caption, variant) = if remap.listening == Some(action) {
            ("press a key…".to_string(), ButtonVariant::Primary)
        } else {
            (keymap.get(action).to_string(), ButtonVariant::Normal)
        };
        commands
            .spawn_scene(plain_scene_button(caption, variant))
            .insert(ChildOf(row))
            .observe(move |_activate: On<Activate>, mut remap: ResMut<Remap>| {
                remap.listening = Some(action);
                remap.changed = true;
            });
    }
    let buttons = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: px(8),
                margin: UiRect::top(px(6)),
                ..default()
            },
            ChildOf(*list),
        ))
        .id();
    commands
        .spawn_scene(plain_scene_button("reset to defaults".into(), ButtonVariant::Normal))
        .insert(ChildOf(buttons))
        .observe(
            |_activate: On<Activate>, mut keymap: ResMut<Keymap>, mut remap: ResMut<Remap>| {
                *keymap = Keymap::default();
                remap.changed = true;
            },
        );
    commands
        .spawn_scene(plain_scene_button("close".into(), ButtonVariant::Primary))
        .insert(ChildOf(buttons))
        .observe(|_activate: On<Activate>, mut remap: ResMut<Remap>| {
            remap.open = false;
            remap.listening = None;
            remap.changed = true;
        });
}
