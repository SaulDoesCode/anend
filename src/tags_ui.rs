//! The tags view: every tag with how many notes carry it, and the notes carrying the
//! chosen ones. Typing in the command bar narrows the tags (Enter adds the first);
//! clicking a tag shows its notes, Ctrl- or Shift-clicking adds it to (or takes it from)
//! the choice, and the notes shown carry all of the chosen tags, or any of them. A note
//! opens in the editor when clicked.

use bevy::feathers::controls::{ButtonVariant, FeathersButton};
use bevy::feathers::display::caption;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{FocusedInput, InputFocus};
use bevy::picking::Pickable;
use bevy::picking::cursor::EntityCursor;
use bevy::picking::events::PointerClick;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEdit, TextEditChange};
use bevy::ui_widgets::{Activate, ScrollArea};
use bevy::window::SystemCursorIcon;

use crate::bar::{BAR_DIM, BarSide};
use crate::icons::{Icon, Tip, icon_label_button, icon_scene_button, spawn_icon};
use crate::note;
use crate::theme::colors;
use crate::ui::{Ctx, Fonts, Notes, Route, Session, plain_scene_button, text};

#[derive(Component, Default, Clone)]
pub struct TagsView;
#[derive(Component, Default, Clone)]
pub struct TagList;
#[derive(Component, Default, Clone)]
pub struct TagHead;
#[derive(Component, Default, Clone)]
pub struct TagResults;

#[derive(Resource)]
pub struct TagsState {
    /// what the bar narrows the tags to
    pub query: String,
    /// the chosen tags, in the order they were chosen
    pub chosen: Vec<String>,
    /// notes must carry every chosen tag (or else any of them)
    pub all: bool,
    pub redraw: bool,
    side: String,
}

impl Default for TagsState {
    fn default() -> Self {
        TagsState {
            query: String::new(),
            chosen: Vec::new(),
            all: true,
            redraw: true,
            side: String::new(),
        }
    }
}

impl TagsState {
    /// picks a tag: alone, or added to / taken from the choice
    pub fn pick(&mut self, tag: &str, add: bool) {
        if add {
            match self.chosen.iter().position(|t| t == tag) {
                Some(i) => {
                    self.chosen.remove(i);
                }
                None => self.chosen.push(tag.to_string()),
            }
        } else if self.chosen.len() == 1 && self.chosen[0] == tag {
            self.chosen.clear();
        } else {
            self.chosen = vec![tag.to_string()];
        }
        self.redraw = true;
    }

    fn matches(&self, tag: &str) -> bool {
        let q = self.query.trim().trim_start_matches('#').to_lowercase();
        q.is_empty() || q.split_whitespace().all(|w| tag.to_lowercase().contains(w))
    }
}

pub fn view() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            display: Display::None,
            flex_direction: FlexDirection::Row,
            column_gap: px(8),
        }
        TagsView
        Children [
            // every tag
            Node {
                width: px(260),
                flex_shrink: 0.0,
                min_height: px(0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                padding: UiRect::all(px(6)),
                border_radius: BorderRadius::all(px(4)),
                overflow: Overflow::scroll_y(),
            }
            BackgroundColor(colors::CARD)
            ScrollArea
            TagList
            --
            // what's chosen, and the notes it finds
            Node {
                flex_grow: 1.0,
                min_width: px(280),
                min_height: px(0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
            }
            Children [
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                    row_gap: px(4),
                    padding: UiRect::all(px(8)),
                    border_radius: BorderRadius::all(px(4)),
                    flex_shrink: 0.0,
                }
                BackgroundColor(colors::CARD)
                TagHead
                --
                Node {
                    flex_grow: 1.0,
                    min_height: px(0),
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: px(8),
                    overflow: Overflow::scroll_y(),
                }
                ScrollArea
                TagResults
            ]
        ]
    }
}

/// a tag in the list: its name, and its count at the right
fn tag_row(name: String, count: String, variant: ButtonVariant) -> impl Scene {
    bsn! {
        @FeathersButton {
            @caption: bsn_list! {
                @caption(name)
                Node { flex_shrink: 1.0, min_width: px(0) }
                --
                Node { flex_grow: 1.0 }
                --
                @caption(count)
            },
            @variant: variant,
        }
        Node {
            justify_content: JustifyContent::Start,
            flex_shrink: 0.0,
            column_gap: px(8),
        }
    }
}

fn adding(keys: &ButtonInput<KeyCode>) -> bool {
    let (ctrl, shift, _) = crate::keys::modifiers(keys);
    ctrl || shift
}

/// what the bar says in the tags view, and the list, the choice and the notes
pub fn rebuild_tags(
    mut st: ResMut<TagsState>,
    notes: Res<Notes>,
    session: Res<Session>,
    list: Single<Entity, With<TagList>>,
    head: Single<Entity, With<TagHead>>,
    results: Single<Entity, With<TagResults>>,
    side: Single<Entity, With<BarSide>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if session.route != Route::Tags {
        st.side.clear();
        return;
    }
    if !st.redraw && !notes.is_changed() && !session.is_changed() {
        return;
    }
    st.redraw = false;
    let counts = note::tag_counts(&notes.0);
    // a tag no note carries any more can't stay chosen
    st.chosen.retain(|t| counts.iter().any(|(c, _)| c == t));
    let shown: Vec<&(String, usize)> = counts.iter().filter(|(t, _)| st.matches(t)).collect();
    let found: Vec<&note::Note> = notes
        .0
        .iter()
        .filter(|n| note::has_tags(n, &st.chosen, st.all))
        .collect();

    // the bar's side
    let line = if st.query.trim().is_empty() {
        format!("{} tags · {} notes found", counts.len(), found.len())
    } else {
        format!(
            "{} of {} tags · Enter adds the first · {} notes found",
            shown.len(),
            counts.len(),
            found.len()
        )
    };
    if st.side != line {
        st.side = line.clone();
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

    // the tags
    commands.entity(*list).despawn_related::<Children>();
    if counts.is_empty() {
        text(
            &mut commands,
            *list,
            "No tags yet. Add one to a note in the editor (the box with the tag beside it).",
            &fonts.italic,
            13.0,
            colors::TEXT_DIM,
        );
    } else if shown.is_empty() {
        text(
            &mut commands,
            *list,
            &format!("no tag has \"{}\"", st.query.trim()),
            &fonts.italic,
            13.0,
            colors::TEXT_DIM,
        );
    }
    for (tag, count) in shown {
        let chosen = st.chosen.contains(tag);
        let t = tag.clone();
        commands
            .spawn_scene(tag_row(
                format!("#{}", tag),
                count.to_string(),
                if chosen { ButtonVariant::Primary } else { ButtonVariant::Plain },
            ))
            .insert((
                ChildOf(*list),
                Tip(format!(
                    "{} note{} · click to see them, Ctrl- or Shift-click to add to the choice",
                    count,
                    if *count == 1 { "" } else { "s" }
                )),
            ))
            .observe(
                move |_a: On<Activate>, keys: Res<ButtonInput<KeyCode>>, mut st: ResMut<TagsState>| {
                    st.pick(&t, adding(&keys));
                },
            );
    }

    // the choice
    commands.entity(*head).despawn_related::<Children>();
    spawn_icon(&mut commands, *head, Icon::Tags, 16.0, colors::TEXT_DIM);
    if st.chosen.is_empty() {
        text(
            &mut commands,
            *head,
            "pick a tag on the left, or type in the bar to find one",
            &fonts.regular,
            14.0,
            colors::TEXT_DIM,
        );
    }
    for tag in st.chosen.clone() {
        let t = tag.clone();
        commands
            .spawn_scene(plain_scene_button(format!("#{}  ×", tag), ButtonVariant::Primary))
            .insert((ChildOf(*head), Tip(format!("stop choosing #{}", tag))))
            .observe(move |_a: On<Activate>, mut st: ResMut<TagsState>| st.pick(&t, true));
    }
    commands.spawn((Node { flex_grow: 1.0, ..default() }, ChildOf(*head)));
    if st.chosen.len() > 1 {
        for (all, icon, label, tip) in [
            (true, Icon::And, "all", "notes with every chosen tag"),
            (false, Icon::Or, "any", "notes with any chosen tag"),
        ] {
            commands
                .spawn_scene(icon_label_button(icon, label.into(), ButtonVariant::selected(st.all == all)))
                .insert((ChildOf(*head), Tip(tip.into())))
                .observe(move |_a: On<Activate>, mut st: ResMut<TagsState>| {
                    st.all = all;
                    st.redraw = true;
                });
        }
    }
    if !st.chosen.is_empty() {
        commands
            .spawn_scene(icon_scene_button(Icon::Close, "clear the choice".into(), ButtonVariant::Normal))
            .insert(ChildOf(*head))
            .observe(|_a: On<Activate>, mut st: ResMut<TagsState>| {
                st.chosen.clear();
                st.redraw = true;
            });
    }

    // the notes
    commands.entity(*results).despawn_related::<Children>();
    if st.chosen.is_empty() {
        return;
    }
    if found.is_empty() {
        let card = commands
            .spawn((crate::ui::card_node(), BackgroundColor(colors::CARD), ChildOf(*results)))
            .id();
        text(
            &mut commands,
            card,
            "No note carries every chosen tag. Try \"any\".",
            &fonts.italic,
            15.0,
            colors::TEXT_DIM,
        );
        return;
    }
    for n in found {
        let key = n.key.clone();
        let card = commands
            .spawn((
                Node {
                    row_gap: px(4),
                    padding: UiRect::axes(px(16), px(12)),
                    ..crate::ui::card_node()
                },
                BackgroundColor(colors::CARD),
                EntityCursor::System(SystemCursorIcon::Pointer),
                Tip("open in the editor".into()),
                ChildOf(*results),
            ))
            .observe(move |c: On<PointerClick>, mut ctx: Ctx| {
                if c.button == PointerButton::Primary {
                    ctx.open(&key);
                }
            })
            .id();
        let t = text(&mut commands, card, n.display_title(), &fonts.bold, 18.0, colors::TEXT);
        commands.entity(t).insert(Pickable::IGNORE);
        let row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                    row_gap: px(2),
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(card),
            ))
            .id();
        if let Some(d) = n.created_at() {
            let t = text(&mut commands, row, &note::long_date(&d), &fonts.regular, 12.0, colors::TEXT_DIM);
            commands.entity(t).insert(Pickable::IGNORE);
        }
        for tag in &n.tags {
            let chosen = st.chosen.contains(tag);
            let t = text(
                &mut commands,
                row,
                &format!("#{}", tag),
                if chosen { &fonts.bold } else { &fonts.regular },
                12.0,
                if chosen { colors::LINK } else { colors::TEXT_DIM },
            );
            commands.entity(t).insert(Pickable::IGNORE);
        }
        let p = note::preview(&n.markdown, 240);
        if !p.is_empty() {
            let t = text(&mut commands, card, &p, &fonts.regular, 14.0, colors::TEXT);
            commands.entity(t).insert(Pickable::IGNORE);
        }
    }
}

/// typing in the bar narrows the tags
pub fn bar_changed(
    change: On<TextEditChange>,
    session: Res<Session>,
    inputs: Query<&EditableText, With<crate::bar::Bar>>,
    mut st: ResMut<TagsState>,
) {
    if session.route != Route::Tags {
        return;
    }
    if let Ok(input) = inputs.get(change.event_target()) {
        let q = input.value().to_string();
        if q != st.query {
            st.query = q;
            st.redraw = true;
        }
    }
}

/// Enter adds the first tag found to the choice and empties the bar, so typing tag after
/// tag chooses them all; Escape leaves the bar
pub fn bar_key(
    key: On<FocusedInput<KeyboardInput>>,
    session: Res<Session>,
    notes: Res<Notes>,
    mut inputs: Query<&mut EditableText>,
    mut st: ResMut<TagsState>,
    mut focus: ResMut<InputFocus>,
) {
    if !key.input.state.is_pressed() || session.route != Route::Tags {
        return;
    }
    match key.input.logical_key {
        Key::Escape => focus.clear(),
        Key::Enter => {
            let first = note::tag_counts(&notes.0)
                .into_iter()
                .map(|(t, _)| t)
                .find(|t| st.matches(t) && !st.chosen.contains(t));
            if st.query.trim().is_empty() {
                return;
            }
            if let Some(t) = first {
                st.pick(&t, true);
                st.query.clear();
                if let Ok(mut input) = inputs.get_mut(key.focused_entity) {
                    input.queue_edit(TextEdit::SelectAll);
                    input.queue_edit(TextEdit::Backspace);
                }
            }
        }
        _ => {}
    }
}
