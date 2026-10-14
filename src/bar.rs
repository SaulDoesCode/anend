//! The command bar: one line at the bottom of the window that every view shares, so
//! there's a single place to type. In the editor, reader and stats it finds notes (Enter
//! opens the first); in the memory view it's weum's term bar; in the space it's the
//! terminal. Each view keeps its own text, so switching views doesn't lose what you typed.
//! The bar is the input, its text centered, the view's name small over its left end. What
//! the view adds sits at its right end (the note count, a hint), except in the memory view
//! and the space, where the sums and toggles go in a thin strip over the bar so nothing
//! takes width from the input there.

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
/// white a hair off the cards' white, so the palette leaves it white (weum's underline,
/// a lit chip)
pub const WHITE: Color = Color::srgb(0.995, 0.995, 0.995);

/// the input
#[derive(Component, Default, Clone)]
pub struct Bar;
/// the bar's row
#[derive(Component, Default, Clone)]
pub struct BarRow;
/// the bar and the strip over it
#[derive(Component, Default, Clone)]
pub struct BarDock;
/// room at each end of the input, so its centered text clears the view's name
pub const MODE_ROOM: f32 = 84.0;
/// the view's name at the left
#[derive(Component, Default, Clone)]
pub struct BarMode;
/// what the view adds to the bar: its counts, sums and toggles
#[derive(Component, Default, Clone)]
pub struct BarSide;
/// the strip over the bar, which holds the [`BarSide`] in the memory view and the space
#[derive(Component, Default, Clone)]
pub struct BarStrip;

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
            flex_direction: FlexDirection::Column,
            row_gap: px(2),
            border_radius: BorderRadius::all(px(4)),
        }
        BackgroundColor(Color::NONE)
        BarDock
        crate::zen::Chrome
        Children [
            // the strip: the memory view's and the space's sums and toggles
            Node {
                display: Display::None,
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(px(8), px(1)),
            }
            BarStrip
            --
            // the bar: the input spans it, its text centered; the view's name sits over
            // its left end without taking width from it
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                min_height: px(30),
                border_radius: BorderRadius::all(px(4)),
            }
            BackgroundColor(colors::NAV)
            BarRow
            Children [
                Node {
                    flex_grow: 1.0,
                    min_width: px(80),
                    padding: UiRect::axes(px(MODE_ROOM), px(4)),
                    border: UiRect::bottom(px(0)),
                }
                BorderColor::all(WHITE)
                TextInput
                EditableText
                TextLayout {
                    linebreak: LineBreak::WordBoundary,
                    justify: Justify::Center,
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
                    position_type: PositionType::Absolute,
                    left: px(10),
                    top: px(0),
                    bottom: px(0),
                    display: Display::Flex,
                    align_items: AlignItems::Center,
                }
                Pickable::IGNORE
                Children [
                    Text("notes")
                    TextFont {
                        font: FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
                        font_size: FontSize::Px(12.0),
                    }
                    TextColor(BAR_DIM)
                    Pickable::IGNORE
                    BarMode
                ]
                --
                // what the view adds: here at the bar's right end in the note views and
                // the lists and the like, moved up into the strip in the memory view and
                // the space
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    flex_shrink: 1.0,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::End,
                    column_gap: px(10),
                    row_gap: px(2),
                    padding: UiRect::right(px(10)),
                    border_radius: BorderRadius::all(px(4)),
                }
                BackgroundColor(Color::NONE)
                BarSide
            ]
        ]
    }
}

/// the views whose additions sit in the strip over the bar rather than in it
pub fn strip_route(r: Route) -> bool {
    matches!(r, Route::Memory | Route::Space)
}

/// the strip shows only in the memory view and the space, while it has something in it
pub fn strip_visibility(
    session: Res<Session>,
    mut strip: Single<&mut Node, With<BarStrip>>,
    side: Single<Option<&Children>, With<BarSide>>,
) {
    let want = if strip_route(session.route) && side.is_some_and(|c| !c.is_empty()) {
        Display::Flex
    } else {
        Display::None
    };
    if strip.display != want {
        strip.display = want;
    }
}

/// the memory view's bar is weum's: black, white text over a white underline, the strip
/// on the same black, joined to the view's black field; elsewhere it's the bars' colour
#[allow(clippy::type_complexity)]
pub fn bar_look(
    session: Res<Session>,
    palette: Res<crate::palette::Palette>,
    config: Res<crate::config::Config>,
    dock: Single<(Entity, &mut BackgroundColor, &mut Node), (With<BarDock>, Without<BarRow>, Without<Bar>)>,
    row: Single<(Entity, &mut BackgroundColor), (With<BarRow>, Without<BarDock>)>,
    input: Single<&mut Node, (With<Bar>, Without<BarDock>)>,
    side: Single<
        (Entity, &ChildOf, &mut BackgroundColor, &mut Node),
        (With<BarSide>, Without<BarDock>, Without<BarRow>, Without<Bar>),
    >,
    strip: Single<Entity, With<BarStrip>>,
    mut last: Local<Option<Route>>,
    mut commands: Commands,
) {
    let mem = session.route == Route::Memory;
    // colours are set when the view or the palette changes, never in between, so zen's
    // fade (which scales their alpha) is left alone; a colour set here is whole and zen
    // measures it again
    let recolor = *last != Some(session.route) || palette.is_changed() || config.is_changed();
    *last = Some(session.route);
    let up = strip_route(session.route);
    // the view's additions go up into the strip, or stay at the bar's right end
    let (side, parent, mut side_bg, mut side_node) = side.into_inner();
    let home = if up { *strip } else { row.0 };
    if parent.parent() != home {
        commands.entity(side).insert(ChildOf(home));
    }
    // over the space's room the strip is a small pill in the bars' colour, so it reads
    let pill = session.route == Route::Space;
    let want = if pill {
        palette.get(crate::palette::Role::Bars)
    } else {
        Color::NONE
    };
    if recolor {
        side_bg.0 = want;
        commands.entity(side).try_remove::<crate::zen::ZenBase>();
    }
    let pad = if pill {
        UiRect::axes(px(8), px(2))
    } else {
        UiRect::right(px(10))
    };
    if side_node.padding != pad {
        side_node.padding = pad;
    }
    let (dock_e, mut dock_bg, mut dock_node) = dock.into_inner();
    let (row_e, mut row_bg) = row.into_inner();
    let mut input_node = input.into_inner();
    let mut set = |bg: &mut BackgroundColor, e: Entity, want: Color| {
        if recolor {
            bg.0 = want;
            commands.entity(e).try_remove::<crate::zen::ZenBase>();
        }
    };
    set(&mut dock_bg, dock_e, if mem { Color::BLACK } else { Color::NONE });
    set(
        &mut row_bg,
        row_e,
        if mem {
            Color::BLACK
        } else {
            palette.get(crate::palette::Role::Bars)
        },
    );
    // the root's gap between the view and the bar closes over the memory view's black
    // (and its rounded corners)
    let gap = if mem { px(-10) } else { px(0) };
    let (top, bottom) = if config.bars_swapped {
        (px(0), gap)
    } else {
        (gap, px(0))
    };
    if dock_node.margin.top != top || dock_node.margin.bottom != bottom {
        dock_node.margin.top = top;
        dock_node.margin.bottom = bottom;
        let pad = if mem { px(10) } else { px(0) };
        dock_node.padding.top = if config.bars_swapped { px(0) } else { pad };
        dock_node.padding.bottom = if config.bars_swapped { pad } else { px(0) };
    }
    // centered over the whole bar when nothing shares its row; else just clear of the end
    let right = if up { input_node.padding.left } else { px(8) };
    if input_node.padding.right != right {
        input_node.padding.right = right;
    }
    // weum's underline: always white, only there in the memory view
    let line = if mem { px(1) } else { px(0) };
    if input_node.border.bottom != line {
        input_node.border.bottom = line;
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
