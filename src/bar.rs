//! The command bar: one line at the bottom of the window that every view shares, so
//! there's a single place to type. In the editor, reader and stats it finds notes (Enter
//! opens the first); in the memory view it's weum's term bar; in the space it's the
//! terminal. Each view keeps its own text, so switching views doesn't lose what you typed.
//! The right side shows what the view needs: the note count, the memory sidecar and
//! toggles, or the space's sums and chips.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::input_focus::{FocusCause, FocusedInput, InputFocus};
use bevy::picking::Pickable;
use bevy::picking::cursor::EntityCursor;
use bevy::prelude::*;
use bevy::text::{EditableText, FontSourceTemplate, TextCursorStyle, TextEdit, TextEditChange};
use bevy::ui_widgets::TextInput;
use bevy::window::SystemCursorIcon;

use crate::memory_ui::{MemTerm, MemoryState, SerifFont};
use crate::space::Space;
use crate::space_ui::SpTerm;
use crate::theme::colors;
use crate::ui::{Ctx, Fonts, Route, Session};

pub const BAR_TEXT: Color = Color::hsl(0.0, 0.0, 0.92);
pub const BAR_DIM: Color = Color::hsl(0.0, 0.0, 0.62);

/// the input
#[derive(Component, Default, Clone)]
pub struct Bar;
/// the strip it sits in
#[derive(Component, Default, Clone)]
pub struct BarRow;
/// the view's name at the left
#[derive(Component, Default, Clone)]
pub struct BarMode;
/// the view's controls at the right
#[derive(Component, Default, Clone)]
pub struct BarSide;

#[derive(Resource, Default)]
pub struct BarState {
    /// what's typed in the note views
    pub notes: String,
    route: Option<Route>,
    /// the save changed: every view's text is gone
    reset: bool,
    notes_side: String,
}

impl BarState {
    pub fn clear(&mut self) {
        self.notes.clear();
        self.reset = true;
        self.route = None;
        self.notes_side.clear();
    }
}

fn notes_route(r: Route) -> bool {
    !matches!(
        r,
        Route::Memory | Route::Space | Route::Lists | Route::Tags | Route::Classifiers | Route::Filters
    )
}

pub fn bar() -> impl Scene {
    bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(10),
            padding: UiRect::axes(px(10), px(2)),
            min_height: px(30),
            border_radius: BorderRadius::all(px(4)),
        }
        BackgroundColor(colors::NAV)
        BarRow
        crate::zen::Chrome
        Children [
            Text("notes")
            TextFont {
                font: FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
                font_size: FontSize::Px(12.0),
            }
            TextColor(BAR_DIM)
            Pickable::IGNORE
            BarMode
            --
            Node {
                flex_grow: 1.0,
                min_width: px(80),
                padding: UiRect::axes(px(2), px(4)),
            }
            TextInput
            EditableText
            TextLayout {
                linebreak: LineBreak::WordBoundary,
            }
            TextFont {
                font: FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
                font_size: FontSize::Px(15.0),
            }
            TextColor(BAR_TEXT)
            TextCursorStyle {
                color: BAR_TEXT,
            }
            TabIndex(0)
            EntityCursor::System(SystemCursorIcon::Text)
            Bar
            MemTerm
            SpTerm
            on(notes_key)
            on(notes_changed)
            on(crate::memory_ui::term_key)
            on(crate::memory_ui::term_changed)
            on(crate::space_ui::term_key)
            on(crate::space_ui::term_changed)
            on(crate::lists_ui::bar_key)
            on(crate::tags_ui::bar_key)
            on(crate::tags_ui::bar_changed)
            on(crate::classify_ui::bar_key)
            on(crate::classify_ui::bar_changed)
            on(crate::filters_ui::bar_key)
            on(crate::filters_ui::bar_changed)
            --
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::End,
                column_gap: px(12),
                row_gap: px(2),
            }
            BarSide
        ]
    }
}

/// Enter opens the first note found, Escape leaves the bar
fn notes_key(key: On<FocusedInput<KeyboardInput>>, mut ctx: Ctx, mut focus: ResMut<InputFocus>) {
    if !key.input.state.is_pressed() || !notes_route(ctx.session.route) {
        return;
    }
    match key.input.logical_key {
        Key::Escape => focus.clear(),
        Key::Enter => {
            let first = ctx
                .notes
                .0
                .iter()
                .find(|n| ctx.session.note_filter.as_ref().is_none_or(|f| f.contains(&n.key)))
                .map(|n| n.key.clone());
            match first {
                Some(k) => {
                    ctx.open(&k);
                    focus.clear();
                }
                None => ctx.say("no note matches"),
            }
        }
        _ => {}
    }
}

/// typing filters the note list (and the reader) to notes with words starting so
fn notes_changed(
    change: On<TextEditChange>,
    inputs: Query<&EditableText, With<Bar>>,
    mut ctx: Ctx,
    mut bar: ResMut<BarState>,
) {
    if !notes_route(ctx.session.route) {
        return;
    }
    let Ok(input) = inputs.get(change.event_target()) else {
        return;
    };
    let query = input.value().to_string();
    bar.notes = query.clone();
    let filter = if query.trim().is_empty() {
        None
    } else {
        ctx.db.0.note_filter(&query).ok()
    };
    if filter != ctx.session.note_filter {
        ctx.session.note_filter = filter;
        ctx.redraw.list = true;
        ctx.redraw.reader = true;
    }
}

/// a view change gives the bar that view's text, name, font and controls
pub fn bar_route(
    session: Res<Session>,
    mut bar: ResMut<BarState>,
    input: Single<(&mut EditableText, &mut TextFont), With<Bar>>,
    mut mode: Single<&mut Text, With<BarMode>>,
    mut mem: ResMut<MemoryState>,
    mut space: ResMut<Space>,
    mut tags: ResMut<crate::tags_ui::TagsState>,
    cls: Res<crate::classify_ui::ClsState>,
    flt: Res<crate::filters_ui::FiltersState>,
    serif: Res<SerifFont>,
    fonts: Res<Fonts>,
    mut focus: ResMut<InputFocus>,
    bar_entity: Single<Entity, With<Bar>>,
) {
    let route = session.route;
    let (mut input, mut font) = input.into_inner();
    if std::mem::take(&mut bar.reset) {
        input.editor_mut().set_text("");
    }
    // the memory view's font can change while it's open
    if route == Route::Memory && serif.is_changed() && font.font != serif.0 {
        font.font = serif.0.clone();
    }
    if bar.route == Some(route) {
        return;
    }
    let text = match route {
        Route::Memory => mem.term.clone(),
        Route::Space => match &space.editing {
            Some(_) => {
                // a text left half edited stays as it is; the terminal comes back
                space.editing = None;
                space.redraw.term_style = true;
                space.term.clone()
            }
            None => space.term.clone(),
        },
        Route::Lists => String::new(),
        Route::Tags => tags.query.clone(),
        Route::Classifiers => cls.query.clone(),
        Route::Filters => flt.query.clone(),
        _ => bar.notes.clone(),
    };
    input.editor_mut().set_text(&text);
    input.queue_edit(TextEdit::TextEnd(false));
    font.font = if route == Route::Memory {
        serif.0.clone()
    } else {
        (&fonts.regular).into()
    };
    mode.0 = match route {
        Route::Memory => "memory",
        Route::Space => "space",
        Route::Lists => "lists",
        Route::Tags => "tags",
        Route::Classifiers => "classifiers",
        Route::Filters => "filters",
        _ => "notes",
    }
    .into();
    // the space starts out flying, not typing
    if route == Route::Space && focus.get() == Some(*bar_entity) {
        focus.clear();
    }
    bar.route = Some(route);
    bar.notes_side.clear();
    mem.redraw_chrome = true;
    space.redraw.termrow = true;
    tags.redraw = true;
}

/// the note views' side of the bar: how many notes there are, or match
pub fn notes_side(
    session: Res<Session>,
    notes: Res<crate::ui::Notes>,
    mut bar: ResMut<BarState>,
    side: Single<Entity, With<BarSide>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if !notes_route(session.route) {
        return;
    }
    let total = notes.0.len();
    let line = match &session.note_filter {
        Some(f) => {
            let n = notes.0.iter().filter(|n| f.contains(&n.key)).count();
            format!("{} of {} notes · Enter opens the first", n, total)
        }
        None => format!("{} note{}", total, if total == 1 { "" } else { "s" }),
    };
    if bar.notes_side == line {
        return;
    }
    bar.notes_side = line.clone();
    commands.entity(*side).despawn_related::<Children>();
    commands.spawn((
        Text::new(line),
        TextFont {
            font: (&fonts.regular).into(),
            font_size: FontSize::Px(12.0),
            ..default()
        },
        TextColor(BAR_DIM),
        Pickable::IGNORE,
        ChildOf(*side),
    ));
}

/// focuses the bar
pub fn focus_bar(focus: &mut InputFocus, bar: Entity) {
    focus.set(bar, FocusCause::Navigated);
}
