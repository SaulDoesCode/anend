//! The lists view: every list with its members, their sums added up and averaged, the
//! list's gematria read as one text, and its letters as a vector; and how alike every
//! two lists are (the cosine of their letter vectors), like embeddings side by side.
//! The command bar fills the chosen list: Enter adds what's typed (remembering it as a
//! memory when it's new); with no list chosen, it starts one headed by that text.

use bevy::feathers::controls::ButtonVariant;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::FocusedInput;
use bevy::picking::Pickable;
use bevy::picking::cursor::EntityCursor;
use bevy::picking::events::PointerClick;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEdit};
use bevy::ui_widgets::{Activate, ScrollArea};
use bevy::window::SystemCursorIcon;

use crate::memory::{self, Ref};
use crate::theme::colors;
use crate::ui::{Db, Fonts, Route, Session, Status, card_node, plain_scene_button, text};
use crate::{lists, spread, weum};

#[derive(Component, Default, Clone)]
pub struct ListsView;

#[derive(Resource, Default)]
pub struct ListsState {
    pub redraw: bool,
    /// the list the bar fills
    chosen: Option<Ref>,
    /// its head's text, while it has no members yet
    chosen_name: String,
}

pub fn view() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            display: Display::None,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: px(8),
            overflow: Overflow::scroll_y(),
        }
        ScrollArea
        ListsView
    }
}

/// Enter in the bar, in the lists view
pub fn bar_key(
    key: On<FocusedInput<KeyboardInput>>,
    session: Res<Session>,
    mut inputs: Query<&mut EditableText>,
    db: Res<Db>,
    mut st: ResMut<ListsState>,
    mut status: ResMut<Status>,
    time: Res<Time>,
) {
    if session.route != Route::Lists || !key.input.state.is_pressed() || key.input.logical_key != Key::Enter {
        return;
    }
    let Ok(mut input) = inputs.get_mut(key.event_target()) else {
        return;
    };
    let typed = input.value().to_string().trim().to_string();
    if typed.is_empty() {
        return;
    }
    let said = match db.0.saturate(&typed) {
        Ok((k, _)) => {
            let r = Ref::Memory(k);
            match st.chosen.clone() {
                Some(list) if list == r => "a list can't hold itself".to_string(),
                Some(list) => match db.0.enlist(&list, &r) {
                    Ok(()) => format!("added “{}” to {}", typed, st.chosen_name),
                    Err(e) => e,
                },
                None => {
                    st.chosen = Some(r);
                    st.chosen_name = typed.clone();
                    format!("new list “{}”: type its members", typed)
                }
            }
        }
        Err(e) => e,
    };
    input.editor_mut().set_text("");
    input.queue_edit(TextEdit::TextEnd(false));
    status.text = said;
    status.until = time.elapsed_secs_f64() + 4.0;
    st.redraw = true;
}

fn row(commands: &mut Commands, parent: Entity, gap: f32) -> Entity {
    commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Center,
                column_gap: px(gap),
                row_gap: px(4),
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}

fn small(commands: &mut Commands, parent: Entity, s: &str, fonts: &Fonts, color: Color) -> Entity {
    let e = text(commands, parent, s, &fonts.regular, 13.0, color);
    commands.entity(e).insert(Pickable::IGNORE);
    e
}

/// a number and what it is, in a column
fn figure(commands: &mut Commands, parent: Entity, value: String, what: &str, fonts: &Fonts) {
    let c = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    text(commands, c, &value, &fonts.bold, 18.0, colors::TEXT);
    small(commands, c, what, fonts, colors::TEXT_DIM);
}

pub fn rebuild_lists(
    mut st: ResMut<ListsState>,
    session: Res<Session>,
    view: Single<Entity, With<ListsView>>,
    side: Single<Entity, With<crate::bar::BarSide>>,
    db: Res<Db>,
    fonts: Res<Fonts>,
    mut last_route: Local<Option<Route>>,
    mut commands: Commands,
) {
    let entered = session.route == Route::Lists && *last_route != Some(Route::Lists);
    *last_route = Some(session.route);
    if !(entered || std::mem::take(&mut st.redraw)) || session.route != Route::Lists {
        return;
    }
    commands.entity(*view).despawn_related::<Children>();
    // the bar's side says what Enter does here
    commands.entity(*side).despawn_related::<Children>();
    let hint = match &st.chosen {
        Some(_) => format!("Enter adds to {}", st.chosen_name),
        None => "Enter starts a list".into(),
    };
    commands.spawn((
        Text::new(hint),
        TextFont {
            font: (&fonts.regular).into(),
            font_size: FontSize::Px(12.0),
            ..default()
        },
        TextColor(crate::bar::BAR_DIM),
        Pickable::IGNORE,
        ChildOf(*side),
    ));
    let all = match lists::stats(&db.0) {
        Ok(l) => l,
        Err(e) => {
            let c = commands
                .spawn((card_node(), BackgroundColor(colors::CARD), ChildOf(*view)))
                .id();
            small(&mut commands, c, &e, &fonts, colors::LINK);
            return;
        }
    };

    // the head: what the bar does now
    let head = commands
        .spawn((card_node(), BackgroundColor(colors::CARD), ChildOf(*view)))
        .id();
    text(&mut commands, head, "lists", &fonts.bold, 24.0, colors::TEXT);
    let line = match &st.chosen {
        Some(_) => format!(
            "Typing in the bar adds to “{}”. Choose another list below, or let go of it to start a new one.",
            st.chosen_name
        ),
        None => "Type in the bar to start a new list headed by that text; choose a list below to add to it.".into(),
    };
    small(&mut commands, head, &line, &fonts, colors::TEXT_DIM);
    if st.chosen.is_some() {
        let r = row(&mut commands, head, 6.0);
        commands
            .spawn_scene(plain_scene_button("let go of it".into(), ButtonVariant::Normal))
            .insert(ChildOf(r))
            .observe(|_a: On<Activate>, mut st: ResMut<ListsState>| {
                st.chosen = None;
                st.chosen_name.clear();
                st.redraw = true;
            });
    }
    if all.is_empty() {
        small(&mut commands, head, "No lists yet.", &fonts, colors::TEXT_DIM);
        return;
    }

    // how alike the lists are
    if all.len() > 1 {
        let card = commands
            .spawn((card_node(), BackgroundColor(colors::CARD), ChildOf(*view)))
            .id();
        text(&mut commands, card, "how alike", &fonts.bold, 16.0, colors::TEXT);
        small(
            &mut commands,
            card,
            "the cosine of each two lists' letter vectors: 1 is the same letters in the same proportions",
            &fonts,
            colors::TEXT_DIM,
        );
        let dots = lists::dots(&all);
        let grid = commands
            .spawn((
                Node {
                    display: Display::Grid,
                    grid_template_columns: std::iter::once(RepeatedGridTrack::px(1, 120.0))
                        .chain(std::iter::once(RepeatedGridTrack::px(all.len() as u16, 46.0)))
                        .collect(),
                    column_gap: px(2),
                    row_gap: px(2),
                    margin: UiRect::top(px(6)),
                    ..default()
                },
                ChildOf(card),
            ))
            .id();
        small(&mut commands, grid, "", &fonts, colors::TEXT_DIM);
        for l in &all {
            let name: String = l.name.chars().take(6).collect();
            small(&mut commands, grid, &name, &fonts, colors::TEXT_DIM);
        }
        for (i, l) in all.iter().enumerate() {
            let name: String = l.name.chars().take(16).collect();
            small(&mut commands, grid, &name, &fonts, colors::TEXT);
            for v in &dots[i] {
                let cell = commands
                    .spawn((
                        Node {
                            height: px(22),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(px(2)),
                            ..default()
                        },
                        BackgroundColor(Color::hsla(
                            spread::seed_hue((v * 360.0) as i64),
                            0.6,
                            0.55,
                            0.15 + 0.75 * *v as f32,
                        )),
                        ChildOf(grid),
                    ))
                    .id();
                small(&mut commands, cell, &format!("{v:.2}"), &fonts, colors::TEXT);
            }
        }
    }

    // each list
    for l in all {
        let chosen = st.chosen.as_ref() == Some(&l.list);
        let card = commands
            .spawn((card_node(), BackgroundColor(colors::CARD), ChildOf(*view)))
            .id();
        let top = row(&mut commands, card, 8.0);
        text(&mut commands, top, &l.name, &fonts.bold, 18.0, colors::TEXT);
        small(&mut commands, top, &l.list.label(), &fonts, colors::TEXT_DIM);
        let (list, name) = (l.list.clone(), l.name.clone());
        commands
            .spawn_scene(plain_scene_button(
                if chosen { "chosen".into() } else { "choose".into() },
                if chosen {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Normal
                },
            ))
            .insert(ChildOf(top))
            .observe(move |_a: On<Activate>, mut st: ResMut<ListsState>| {
                st.chosen = Some(list.clone());
                st.chosen_name = name.clone();
                st.redraw = true;
            });
        let list = l.list.clone();
        commands
            .spawn_scene(plain_scene_button("unlist".into(), ButtonVariant::Normal))
            .insert(ChildOf(top))
            .observe(
                move |_a: On<Activate>,
                      db: Res<Db>,
                      mut st: ResMut<ListsState>,
                      mut status: ResMut<Status>,
                      time: Res<Time>| {
                    status.text = match db.0.unlist(&list) {
                        Ok(n) => format!("unlisted {n} members"),
                        Err(e) => e,
                    };
                    status.until = time.elapsed_secs_f64() + 4.0;
                    if st.chosen.as_ref() == Some(&list) {
                        st.chosen = None;
                    }
                    st.redraw = true;
                },
            );

        // the list in numbers
        let figures = row(&mut commands, card, 18.0);
        figure(&mut commands, figures, l.members.len().to_string(), "members", &fonts);
        figure(&mut commands, figures, l.ordinal_sum.to_string(), "ordinal", &fonts);
        figure(
            &mut commands,
            figures,
            format!("{:.1}", l.ordinal_mean()),
            "ordinal mean",
            &fonts,
        );
        figure(&mut commands, figures, l.reversed_sum.to_string(), "reversed", &fonts);
        figure(
            &mut commands,
            figures,
            format!("{:.1}", l.reversed_mean()),
            "reversed mean",
            &fonts,
        );
        // the whole list read as one text
        let m = memory::measure(&l.joined);
        small(
            &mut commands,
            card,
            &format!(
                "as one: ordinal {} · reversed {} · atbash {} · letters {}",
                m.ordinal,
                m.reversed,
                m.atbash,
                m.frequency.chars().take(40).collect::<String>()
            ),
            &fonts,
            colors::TEXT,
        );
        small(
            &mut commands,
            card,
            &weum::gsum_detail(&l.joined),
            &fonts,
            colors::TEXT_DIM,
        );
        if m.ordinal > 1 {
            let chain = weum::segs_to_string(&weum::pf_reduction(m.ordinal as u64));
            small(&mut commands, card, &format!("({chain})"), &fonts, colors::TEXT_DIM);
        }

        // its letters as a vector
        let strip = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::End,
                    column_gap: px(2),
                    height: px(44),
                    margin: UiRect::vertical(px(4)),
                    ..default()
                },
                ChildOf(card),
            ))
            .id();
        for (i, v) in l.vector.iter().enumerate() {
            let col = commands
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::End,
                        width: px(14),
                        height: percent(100),
                        ..default()
                    },
                    ChildOf(strip),
                ))
                .id();
            commands.spawn((
                Node {
                    width: px(10),
                    height: px((v * 30.0).max(1.0) as f32),
                    border_radius: BorderRadius::top(px(2)),
                    ..default()
                },
                BackgroundColor(colors::ACCENT),
                ChildOf(col),
            ));
            let letter = ((b'a' + i as u8) as char).to_string();
            let t = text(&mut commands, col, &letter, &fonts.regular, 10.0, colors::TEXT_DIM);
            commands.entity(t).insert(Pickable::IGNORE);
        }

        // its members
        let chips = row(&mut commands, card, 4.0);
        for h in l.members {
            let (s, r) = spread::sums(&h.text);
            let chip = commands
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(5),
                        padding: UiRect::axes(px(7), px(2)),
                        border_radius: BorderRadius::all(px(3)),
                        ..default()
                    },
                    BackgroundColor(colors::PAD),
                    ChildOf(chips),
                ))
                .id();
            small(&mut commands, chip, &h.text, &fonts, colors::TEXT);
            small(&mut commands, chip, &format!("{s}·{r}"), &fonts, colors::TEXT_DIM);
            let (list, member) = (l.list.clone(), h.r.clone());
            let x = text(&mut commands, chip, "×", &fonts.regular, 14.0, colors::TEXT_DIM);
            commands
                .entity(x)
                .insert(EntityCursor::System(SystemCursorIcon::Pointer))
                .observe(move |_c: On<PointerClick>, db: Res<Db>, mut st: ResMut<ListsState>| {
                    let _ = db.0.delist(&list, &member);
                    st.redraw = true;
                });
        }
    }
}
