//! The space's panels, remem's milk and coffee cards over the 3D view: the position card
//! (top left), help (top right), the toast, the metrics of the text under the pointer
//! (bottom right), the sliders (bottom left) and the terminal (bottom middle) with its
//! search line, sidecar and chips.

use bevy::feathers::constants::fonts;
use bevy::feathers::controls::{ButtonVariant, FeathersSlider};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{FocusCause, FocusedInput, InputFocus};
use bevy::picking::Pickable;
use bevy::picking::cursor::EntityCursor;
use bevy::picking::events::{PointerClick, PointerEnter, PointerLeave};
use bevy::prelude::*;
use bevy::text::{EditableText, FontSource, FontSourceTemplate, GenericFontFamily, TextEdit, TextEditChange};
use bevy::ui_widgets::{Activate, ScrollArea, SliderPrecision, SliderStep, SliderValue, ValueChange};
use bevy::window::SystemCursorIcon;

use crate::bar::{BAR_DIM, BAR_TEXT, BarMode, BarSide};
use crate::keys::{Action, Keymap, modifiers};
use crate::palette::{Palette, Role};
use crate::space::{ACCENT, Qline, SLIDERS, Sp, Space, SpaceText, bucket_color};
use crate::theme::colors;
use crate::ui::{Route, Session};

const INK: Color = colors::TEXT;
use crate::spread;
use crate::ui::{Fonts, plain_scene_button};
use crate::weum::{self, Seg};

/// the app's white cards, a little see-through over the room
const PANEL: Color = Color::srgba(1.0, 1.0, 1.0, 0.82);
const DIM: Color = colors::TEXT_DIM;

const HINT_EDIT: &str = "editing · Enter done · Shift+Enter new line";

#[derive(Component, Default, Clone)]
pub struct SpaceView;
#[derive(Component, Default, Clone)]
pub struct SpHud;
#[derive(Component, Default, Clone)]
pub struct SpHudText;
#[derive(Component, Default, Clone)]
pub struct SpHelp;
#[derive(Component, Default, Clone)]
pub struct SpStart;
#[derive(Component, Default, Clone)]
pub struct SpToast;
#[derive(Component, Default, Clone)]
pub struct SpToastText;
#[derive(Component, Default, Clone)]
pub struct SpMetrics;
#[derive(Component, Default, Clone)]
pub struct SpOpts;
#[derive(Component, Default, Clone)]
pub struct SpQline;
#[derive(Component, Default, Clone)]
pub struct SpTerm;

fn panel() -> Node {
    Node {
        position_type: PositionType::Absolute,
        display: Display::None,
        flex_direction: FlexDirection::Column,
        padding: UiRect::axes(px(10), px(6)),
        border_radius: BorderRadius::all(px(4)),
        ..default()
    }
}

pub fn view() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            display: Display::None,
        }
        SpaceView
        Pickable::IGNORE
        Children [
            // the position line
            Node {
                position_type: PositionType::Absolute,
                top: px(0),
                left: px(2),
                padding: UiRect::axes(px(8), px(3)),
                border_radius: BorderRadius::all(px(4)),
            }
            BackgroundColor(PANEL)
            Pickable::IGNORE
            SpHud
            crate::zen::Chrome
            Children [
                Text("")
                TextFont {
                    font: FontSourceTemplate::Handle(fonts::REGULAR),
                    font_size: FontSize::Px(12.0),
                }
                TextColor(DIM)
                Pickable::IGNORE
                SpHudText
            ]
            --
            // help
            Node {
                top: px(0),
                right: px(2),
                max_height: percent(100),
                overflow: Overflow::scroll_y(),
                ..panel()
            }
            BackgroundColor(PANEL)
            ScrollArea
            SpHelp
            crate::zen::Chrome
            --
            // the first time's hint
            Node {
                top: percent(12),
                left: percent(30),
                padding: UiRect::axes(px(14), px(6)),
                ..panel()
            }
            BackgroundColor(PANEL)
            Pickable::IGNORE
            SpStart
            Children [
                Text("Hold the right mouse button to look · H for help")
                TextFont {
                    font: FontSourceTemplate::Handle(fonts::REGULAR),
                    font_size: FontSize::Px(14.0),
                }
                TextColor(INK)
            ]
            --
            // the toast
            Node {
                position_type: PositionType::Absolute,
                top: px(0),
                left: px(0),
                right: px(0),
                display: Display::None,
                justify_content: JustifyContent::Center,
            }
            Pickable::IGNORE
            SpToast
            GlobalZIndex(3)
            Children [
                Node {
                    padding: UiRect::axes(px(10), px(4)),
                    border_radius: BorderRadius::all(px(4)),
                }
                BackgroundColor(PANEL)
                Pickable::IGNORE
                Children [
                    Text("")
                    TextFont {
                        font: FontSourceTemplate::Handle(fonts::REGULAR),
                        font_size: FontSize::Px(13.0),
                    }
                    TextColor(INK)
                    Pickable::IGNORE
                    SpToastText
                ]
            ]
            --
            // metrics of the text under the pointer
            Node {
                bottom: px(0),
                right: px(2),
                width: px(268),
                ..panel()
            }
            BackgroundColor(PANEL)
            SpMetrics
            crate::zen::Chrome
            --
            // the sliders
            Node {
                bottom: px(0),
                left: px(2),
                width: px(230),
                max_height: percent(100),
                row_gap: px(1),
                overflow: Overflow::scroll_y(),
                ..panel()
            }
            BackgroundColor(PANEL)
            ScrollArea
            SpOpts
            crate::zen::Chrome

        ]
    }
}

// ------------------------------------------------------------------ the terminal

pub(crate) fn term_key(
    key: On<FocusedInput<KeyboardInput>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut terms: Query<&mut EditableText, With<SpTerm>>,
    mut focus: ResMut<InputFocus>,
    session: Res<Session>,
    mut sp: Sp,
) {
    if !key.input.state.is_pressed() || session.route != Route::Space {
        return;
    }
    let Ok(mut term) = terms.get_mut(key.focused_entity) else {
        return;
    };
    let text = term.value().to_string();
    let (ctrl, shift, _) = modifiers(&keys);
    if sp.st.editing.is_some() {
        match key.input.logical_key {
            Key::Enter if shift => term.queue_edit(TextEdit::Insert("\n".into())),
            Key::Enter | Key::Escape => sp.close_editor(),
            _ => {}
        }
        return;
    }
    match key.input.logical_key {
        Key::Escape => focus.clear(),
        Key::Enter => sp.run(&text, ctrl, shift),
        Key::Delete => sp.forget_typed(&text),
        Key::ArrowUp => sp.history(true),
        Key::ArrowDown => sp.history(false),
        _ => {}
    }
}

pub(crate) fn term_changed(
    change: On<TextEditChange>,
    terms: Query<&EditableText, With<SpTerm>>,
    session: Res<Session>,
    mut sp: Sp,
) {
    if session.route != Route::Space {
        return;
    }
    if let Ok(term) = terms.get(change.event_target()) {
        sp.box_input(term.value().to_string());
    }
}

/// puts text into the box, focuses and blurs it, and marks it while editing a text
pub fn apply_term(
    mut st: ResMut<Space>,
    term: Single<(Entity, &mut EditableText), With<SpTerm>>,
    mode: Single<(&mut Text, &mut TextColor), With<BarMode>>,
    mut focus: ResMut<InputFocus>,
    session: Res<Session>,
) {
    if session.route != Route::Space {
        return;
    }
    let (entity, mut term) = term.into_inner();
    if let Some(t) = st.set_term.take() {
        term.editor_mut().set_text(&t);
        term.queue_edit(TextEdit::TextEnd(false));
        if std::mem::take(&mut st.select_term) {
            term.queue_edit(TextEdit::SelectAll);
        }
    }
    let lines = term.value().to_string().lines().count().clamp(1, 6) as f32;
    if term.visible_lines != Some(lines) {
        term.visible_lines = Some(lines);
    }
    if std::mem::take(&mut st.blur_term) && focus.get() == Some(entity) {
        focus.clear();
    }
    if std::mem::take(&mut st.focus_term) {
        focus.set(entity, FocusCause::Navigated);
    }
    if std::mem::take(&mut st.redraw.term_style) {
        // the bar has no border to mark editing with, so its name says it
        let (mut text, mut color) = mode.into_inner();
        let editing = st.editing.is_some();
        text.0 = if editing { "editing" } else { "space" }.into();
        color.0 = if editing { ACCENT } else { BAR_DIM };
        st.redraw.termrow = true;
    }
}

// ------------------------------------------------------------------ drawing helpers

/// Fira Sans, falling back on the system's sans for ⌊ ⌋ √ and the like
fn text_font(fonts: &Fonts, bold: bool, size: f32) -> TextFont {
    let own = if bold { &fonts.bold } else { &fonts.regular };
    TextFont {
        font: FontSource::from(vec![
            FontSource::Handle(own.clone()),
            FontSource::from(GenericFontFamily::SansSerif),
        ]),
        font_size: FontSize::Px(size),
        ..default()
    }
}

fn plain(commands: &mut Commands, parent: Entity, s: impl Into<String>, fonts: &Fonts, color: Color) -> Entity {
    commands
        .spawn((
            Text::new(s),
            text_font(fonts, false, 13.0),
            TextColor(color),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

fn bold_in(commands: &mut Commands, parent: Entity, s: impl Into<String>, fonts: &Fonts, ink: Color) -> Entity {
    commands
        .spawn((
            Text::new(s),
            text_font(fonts, true, 13.0),
            TextColor(ink),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

/// the palette colour a link goes back to when the pointer leaves
#[derive(Component)]
struct Ink(Role);

/// a clickable bit of text that turns accent under the pointer
fn link<'a>(
    commands: &'a mut Commands,
    parent: Entity,
    s: impl Into<String>,
    fonts: &Fonts,
    size: f32,
) -> EntityCommands<'a> {
    link_in(commands, parent, s, fonts, size, Role::Text)
}

fn link_in<'a>(
    commands: &'a mut Commands,
    parent: Entity,
    s: impl Into<String>,
    fonts: &Fonts,
    size: f32,
    ink: Role,
) -> EntityCommands<'a> {
    let mut e = commands.spawn((
        Text::new(s),
        text_font(fonts, false, size),
        TextColor(ink.default_color()),
        Ink(ink),
        EntityCursor::System(SystemCursorIcon::Pointer),
        ChildOf(parent),
    ));
    e.observe(|e: On<PointerEnter>, mut colors: Query<&mut TextColor>| {
        if let Ok(mut c) = colors.get_mut(e.event_target()) {
            c.0 = ACCENT.with_alpha(c.0.alpha());
        }
    })
    .observe(
        |e: On<PointerLeave>, mut colors: Query<(&mut TextColor, &Ink)>, palette: Res<Palette>| {
            if let Ok((mut c, ink)) = colors.get_mut(e.event_target()) {
                c.0 = palette.get(ink.0).with_alpha(c.0.alpha());
            }
        },
    );
    e
}

fn row(commands: &mut Commands, parent: Entity) -> Entity {
    commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Baseline,
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}

/// weum's prime factor reduction line, its numbers gsearching themselves
fn pf_line(commands: &mut Commands, parent: Entity, n: u64, rev: bool, fonts: &Fonts, ink: Role) {
    let r = row(commands, parent);
    commands.entity(r).insert(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        margin: UiRect::top(px(4)),
        ..default()
    });
    let small = |c: &mut Commands, s: String| {
        c.spawn((
            Text::new(s),
            text_font(fonts, false, 11.0),
            TextColor(ink.default_color()),
            Pickable::IGNORE,
            ChildOf(r),
        ));
    };
    small(commands, "(".into());
    for seg in weum::pf_reduction(n) {
        match seg {
            Seg::Text(t) => small(commands, t),
            Seg::Num(v) => {
                link_in(commands, r, v.to_string(), fonts, 11.0, ink)
                    .observe(move |_c: On<PointerClick>, mut sp: Sp| sp.envale(v.min(u32::MAX as u64) as u32, rev));
            }
        }
    }
    small(commands, ")".into());
}

// ------------------------------------------------------------------ panels

pub fn space_panels(
    mut st: ResMut<Space>,
    time: Res<Time>,
    cam: Single<&Transform, With<crate::space::SpaceCam>>,
    texts: Query<&SpaceText>,
    mut panels: ParamSet<(
        Single<&mut Node, With<SpHud>>,
        Single<&mut Node, With<SpHelp>>,
        Single<&mut Node, With<SpStart>>,
        Single<&mut Node, With<SpToast>>,
        Single<&mut Node, With<SpOpts>>,
    )>,
    mut hud: Single<&mut Text, (With<SpHudText>, Without<SpToastText>)>,
    mut toast: Single<&mut Text, (With<SpToastText>, Without<SpHudText>)>,
) {
    let shown = |on: bool| if on { Display::Flex } else { Display::None };
    if let Some((_, until)) = &st.toast
        && time.elapsed_secs_f64() > *until
    {
        st.toast = None;
        st.redraw.toast = true;
    }
    if std::mem::take(&mut st.redraw.toast) {
        toast.0 = st.toast.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        panels.p3().display = shown(st.toast.is_some());
    }
    if std::mem::take(&mut st.redraw.panels) {
        panels.p0().display = shown(st.show_hud);
        panels.p1().display = shown(st.show_help);
        panels.p2().display = shown(st.show_start);
        panels.p4().display = shown(st.show_opts);
    }
    if st.show_hud {
        let p = cam.translation;
        let roll = crate::space::roll_deg(cam.rotation)
            .map(|r| format!("{:.0}°", r))
            .unwrap_or_else(|| "—".into());
        let n = texts.iter().filter(|t| !t.removed).count();
        let state = if st.physics_paused {
            "paused"
        } else if st.spread.asleep() {
            "still"
        } else {
            "moving"
        };
        let s = format!(
            "{:.1}  {:.1}  {:.1}   speed {:.1}   roll {}   {} texts   spread {}",
            p.x, p.y, p.z, st.speed, roll, n, state
        );
        if hud.0 != s {
            hud.0 = s;
        }
    }
}

/// the help card, with the keys as they're bound now
pub fn rebuild_help(
    mut st: ResMut<Space>,
    keymap: Res<Keymap>,
    help: Single<Entity, With<SpHelp>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if !std::mem::take(&mut st.redraw.help) && !keymap.is_changed() {
        return;
    }
    let k = |a: Action| keymap.get(a).to_string();
    let sections: Vec<(&str, Vec<(String, &str)>)> = vec![
        (
            "flying",
            vec![
                ("Right mouse".into(), "hold to look"),
                (
                    format!(
                        "{} {} {} {}",
                        k(Action::FlyForward),
                        k(Action::FlyLeft),
                        k(Action::FlyBack),
                        k(Action::FlyRight)
                    ),
                    "move",
                ),
                (format!("{} / {}", k(Action::FlyUp), k(Action::FlyDown)), "up / down"),
                (
                    format!("{} / {}", k(Action::RollLeft), k(Action::RollRight)),
                    "roll left / right",
                ),
                ("Shift".into(), "fast"),
                (
                    format!("{} {}  or wheel", k(Action::Slower), k(Action::Faster)),
                    "flight speed (wheel over empty space)",
                ),
                (k(Action::Level), "level the horizon"),
                (k(Action::BackToStart), "back to start"),
            ],
        ),
        (
            "texts",
            vec![
                (
                    "Left drag on a text".into(),
                    "move it; a spread text dragged stays pinned",
                ),
                ("Wheel on a text".into(), "push away / pull near"),
                ("Click a text".into(), "its text into the terminal"),
                (
                    "Double click a text".into(),
                    "edit it; on a memory text: its key into the terminal",
                ),
                (k(Action::EditText), "edit the selected text"),
                (format!("Double click empty, {}", k(Action::NewText)), "new text"),
                (k(Action::DeleteText), "remove the selected text"),
                (k(Action::CopyText), "copy the selected text"),
                (k(Action::Undo), "undo, including a forget"),
                (
                    format!("{} / {}", k(Action::Export), k(Action::SaveMap)),
                    "export all texts to a file / save the map",
                ),
            ],
        ),
        (
            "memory",
            vec![
                (
                    format!("{} or {}", k(Action::SpaceTerm), k(Action::EditText)),
                    "terminal (Enter with nothing selected)",
                ),
                ("Enter in terminal".into(), "number: gsearch that bucket; text: search"),
                ("Ctrl+Enter".into(), "saturate the text, gsearch its sum"),
                ("Shift+Enter".into(), "recall by key, e.g. 3,14"),
                ("Delete in terminal".into(), "forget the key typed there"),
                ("↑ ↓ in terminal".into(), "history"),
                ("Ctrl+click".into(), "list candidate"),
                ("Shift+click".into(), "list"),
                ("Ctrl+right click".into(), "gsearch ⌊√(s·r)⌋"),
                (
                    format!("{} {}", k(Action::SideSum), k(Action::SideReversedSum)),
                    "gsearch the sidecar values",
                ),
                (k(Action::Reversed), "reversed"),
                (k(Action::Pin), "pin / unpin the selected spread text"),
                (
                    format!("{} / {} / {}", k(Action::Sliders), k(Action::Pause), k(Action::Settle)),
                    "sliders / pause physics / settle now",
                ),
                (
                    "#123  #r123  #name".into(),
                    "an ordinal / a reversed bucket, a saved map",
                ),
                (
                    "Words".into(),
                    "Enlist List Delist Unlist Count Backup Clear ClearAll Settle Maps Save name",
                ),
                (
                    format!("{} / {}", k(Action::Help), k(Action::Hud)),
                    "help / position card",
                ),
                (k(Action::SpaceKeys), "the keys menu"),
            ],
        ),
    ];
    commands.entity(*help).despawn_related::<Children>();
    for (title, rows) in sections {
        let t = commands
            .spawn((
                Text::new(title),
                text_font(&fonts, true, 13.0),
                TextColor(INK.with_alpha(0.65)),
                Node {
                    margin: UiRect::top(px(8)),
                    ..default()
                },
                ChildOf(*help),
            ))
            .id();
        let _ = t;
        for (key, what) in rows {
            let r = commands
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: px(16),
                        ..default()
                    },
                    ChildOf(*help),
                ))
                .id();
            commands.spawn((
                Text::new(key),
                text_font(&fonts, true, 13.0),
                TextColor(INK),
                Node {
                    width: px(150),
                    flex_shrink: 0.0,
                    ..default()
                },
                ChildOf(r),
            ));
            commands.spawn((
                Text::new(what),
                text_font(&fonts, false, 13.0),
                TextColor(INK),
                Node {
                    max_width: px(260),
                    ..default()
                },
                ChildOf(r),
            ));
        }
    }
}

/// the search line in the top bar: what was searched, how many texts and buckets, and the
/// sum's prime factor chain; only in the space, and only when the settings want it
pub fn rebuild_qline(
    mut st: ResMut<Space>,
    qline: Single<(Entity, &mut Node), With<SpQline>>,
    session: Res<Session>,
    config: Res<crate::config::Config>,
    fonts: Res<Fonts>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    let (q, mut node) = qline.into_inner();
    let empty = matches!(st.qline, Qline::None) && st.mem_index.is_empty();
    let display = if empty || session.route != Route::Space || !config.space_sums {
        Display::None
    } else {
        Display::Flex
    };
    if node.display != display {
        node.display = display;
    }
    if !std::mem::take(&mut st.redraw.qline) {
        return;
    }
    commands.entity(q).despawn_related::<Children>();
    let count = format!(" · {} texts · {} buckets", st.mem_index.len(), st.spread.order.len());
    match st.qline.clone() {
        Qline::None => {
            if !st.mem_index.is_empty() {
                plain(&mut commands, q, count.trim_start_matches(" · "), &fonts, BAR_TEXT);
            }
        }
        Qline::Gsearch { n, rev } => {
            bold_in(&mut commands, q, n.to_string(), &fonts, BAR_TEXT);
            plain(
                &mut commands,
                q,
                format!(" {}{}", if rev { "reversed" } else { "ordinal" }, count),
                &fonts,
                BAR_TEXT,
            );
            if n > 1 {
                let holder = commands
                    .spawn((
                        Node {
                            margin: UiRect::left(px(10)),
                            ..default()
                        },
                        ChildOf(q),
                    ))
                    .id();
                pf_line(&mut commands, holder, n as u64, rev, &fonts, Role::BarText);
            }
        }
        Qline::Key(k) | Qline::List(k) => {
            let word = if matches!(st.qline, Qline::Key(_)) {
                "key "
            } else {
                "list "
            };
            plain(&mut commands, q, word, &fonts, BAR_TEXT);
            bold_in(&mut commands, q, k, &fonts, BAR_TEXT);
            plain(&mut commands, q, count, &fonts, BAR_TEXT);
        }
        Qline::Search(v) => {
            plain(&mut commands, q, format!("search “{}”{}", v, count), &fonts, BAR_TEXT);
        }
        Qline::Maps(names) => {
            plain(
                &mut commands,
                q,
                if names.is_empty() {
                    "no maps yet: Save name"
                } else {
                    "maps"
                },
                &fonts,
                BAR_TEXT,
            );
            for name in names {
                plain(&mut commands, q, " · ", &fonts, BAR_TEXT);
                let n = name.clone();
                link_in(&mut commands, q, name, &fonts, 13.0, Role::BarText)
                    .observe(move |_c: On<PointerClick>, mut sp: Sp| sp.route(&n));
            }
        }
    }
    // the triangle the last Ctrl+right clicked text makes (or the terminal's)
    let text = if st.tri_text.is_empty() {
        st.term.clone()
    } else {
        st.tri_text.clone()
    };
    if text.chars().any(|c| c.is_ascii_alphabetic()) {
        let holder = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(5),
                    margin: UiRect::left(px(10)),
                    ..default()
                },
                ChildOf(q),
            ))
            .id();
        match spread::triangle(&text) {
            Some(t) => {
                commands.spawn((
                    ImageNode::new(images.add(triangle_image(&t))),
                    Node {
                        width: px(TRI_W as f32),
                        height: px(TRI_H as f32),
                        ..default()
                    },
                    Pickable::IGNORE,
                    ChildOf(holder),
                ));
                let [s, r, tt] = t.sides;
                let [a, b, c] = t.angles;
                plain(
                    &mut commands,
                    holder,
                    format!("{s} {r} {tt} · {a:.0}° {b:.0}° {c:.0}°"),
                    &fonts,
                    BAR_DIM,
                );
            }
            None => {
                let (s, r) = spread::sums(&text);
                let t = ((s.max(0) * r.max(0)) as f64).sqrt().floor() as i64;
                plain(
                    &mut commands,
                    holder,
                    format!("△ {s} {r} {t} don't close"),
                    &fonts,
                    BAR_DIM,
                );
            }
        }
    }
}

/// the space's side of the command bar: the terminal's sums, the chips, the list state
pub fn rebuild_termrow(
    mut st: ResMut<Space>,
    side: Single<Entity, With<BarSide>>,
    session: Res<Session>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if session.route != Route::Space || !std::mem::take(&mut st.redraw.termrow) {
        return;
    }
    let t = *side;
    commands.entity(t).despawn_related::<Children>();
    let bar_text = |commands: &mut Commands, parent: Entity, s: String, color: Color| {
        commands.spawn((
            Text::new(s),
            text_font(&fonts, false, 13.0),
            TextColor(color),
            Pickable::IGNORE,
            ChildOf(parent),
        ));
    };
    if st.editing.is_some() {
        bar_text(&mut commands, t, HINT_EDIT.into(), BAR_DIM);
        return;
    }
    let term = st.term.trim();
    if !term.is_empty() && term.parse::<u32>().is_err() {
        let sc = row(&mut commands, t);
        let (s, r) = spread::sums(&st.term);
        bar_text(&mut commands, sc, "(".into(), BAR_DIM);
        for (i, v) in [s, r].into_iter().enumerate() {
            if i == 1 {
                bar_text(&mut commands, sc, ", ".into(), BAR_DIM);
            }
            bar_link(&mut commands, sc, v.to_string(), &fonts).observe(move |_c: On<PointerClick>, mut sp: Sp| {
                sp.set_reversed(i == 1);
                sp.envale(v.clamp(0, u32::MAX as i64) as u32, i == 1);
            });
        }
        bar_text(&mut commands, sc, ")".into(), BAR_DIM);
    }
    let list = match (&st.list_id, &st.list_candidate) {
        (Some(l), Some(c)) => format!("list {} · candidate {}", l.label(), c.label()),
        (Some(l), None) => format!("list {}", l.label()),
        (None, Some(c)) => format!("candidate {}", c.label()),
        (None, None) => String::new(),
    };
    if !list.is_empty() {
        bar_text(&mut commands, t, list, BAR_DIM);
    }
    let chips = st.chips;
    let chip = |commands: &mut Commands, label: &str, on: bool, f: fn(&mut crate::space::Chips)| {
        commands
            .spawn((
                Text::new(label),
                text_font(&fonts, false, 13.0),
                TextColor(if on { BAR_TEXT } else { BAR_DIM }),
                EntityCursor::System(SystemCursorIcon::Pointer),
                ChildOf(t),
            ))
            .insert_if(bevy::text::Underline, || on)
            .observe(move |_c: On<PointerClick>, mut sp: Sp| {
                let mut c = sp.st.chips;
                f(&mut c);
                sp.set_chips(c);
            });
    };
    chip(&mut commands, "gsearch", chips.gsch, |c| c.gsch = !c.gsch);
    chip(&mut commands, "reversed", chips.rtb, |c| c.rtb = !c.rtb);
    chip(&mut commands, "saturate", chips.also_saturate, |c| {
        c.also_saturate = !c.also_saturate
    });
    chip(&mut commands, "keep", chips.keep, |c| c.keep = !c.keep);
}

/// a clickable number in the bar
fn bar_link<'a>(commands: &'a mut Commands, parent: Entity, s: String, fonts: &Fonts) -> EntityCommands<'a> {
    link_in(commands, parent, s, fonts, 13.0, Role::BarText)
}

/// remem's metrics card: the sums of the text under the pointer (or the selected one), its
/// key and bucket, each number gsearching itself
pub fn rebuild_metrics(
    mut st: ResMut<Space>,
    metrics: Single<(Entity, &mut Node), With<SpMetrics>>,
    texts: Query<&SpaceText>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if !std::mem::take(&mut st.redraw.metrics) {
        return;
    }
    let (m_e, mut node) = metrics.into_inner();
    commands.entity(m_e).despawn_related::<Children>();
    let Some(t) = st.shown_for.and_then(|e| texts.get(e).ok()).filter(|t| !t.removed) else {
        node.display = Display::None;
        return;
    };
    node.display = Display::Flex;
    let rtb = st.chips.rtb;
    let m = spread::metrics(t.own_text());
    // label, value, clickable as (reversed?) — None: not clickable, Some(None): the chip decides
    type Cell = (&'static str, String, Option<Option<bool>>);
    let n = |v: i64, rev: Option<bool>| (v.to_string(), Some(rev));
    let rows: [[Cell; 2]; 7] = [
        [
            ("s", n(m.s, Some(false)).0, Some(Some(false))),
            ("r", m.r.to_string(), Some(Some(true))),
        ],
        [
            ("s+r", m.sr.to_string(), Some(None)),
            ("s·r", m.product.to_string(), Some(None)),
        ],
        [
            ("√(s·r)", format!("{:.3}", m.root), None),
            ("⌊√(s·r)⌋", m.jump.to_string(), Some(None)),
        ],
        [
            ("s−r", m.diff.to_string(), None),
            ("s+r+|s−r|", m.wide.to_string(), Some(None)),
        ],
        [
            ("s+r+(s−r)", m.lean.to_string(), Some(None)),
            ("|s−r|−(s−r)", m.gap.to_string(), Some(None)),
        ],
        [("v", m.v.to_string(), Some(None)), ("c", m.c.to_string(), Some(None))],
        [("w", m.w.to_string(), None), ("letters", m.letters.to_string(), None)],
    ];
    for cells in rows {
        let r = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    ..default()
                },
                ChildOf(m_e),
            ))
            .id();
        for (i, (label, value, click)) in cells.into_iter().enumerate() {
            let w = if i == 0 { [64.0, 76.0] } else { [84.0, 50.0] };
            let l = plain(&mut commands, r, label, &fonts, INK.with_alpha(0.65));
            commands.entity(l).insert(Node {
                width: px(w[0]),
                ..default()
            });
            let cell = commands
                .spawn((
                    Node {
                        width: px(w[1]),
                        ..default()
                    },
                    ChildOf(r),
                ))
                .id();
            match (click, value.parse::<i64>()) {
                (Some(rev), Ok(v)) if v >= 0 => {
                    let rev = rev.unwrap_or(rtb);
                    link(&mut commands, cell, value, &fonts, 13.0)
                        .observe(move |_c: On<PointerClick>, mut sp: Sp| sp.envale(v.min(u32::MAX as i64) as u32, rev));
                }
                _ => {
                    plain(&mut commands, cell, value, &fonts, INK);
                }
            }
        }
    }
    if let Some(k) = t.mem.clone() {
        let mem = row(&mut commands, m_e);
        commands.entity(mem).insert(Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            margin: UiRect::top(px(6)),
            ..default()
        });
        plain(&mut commands, mem, "key ", &fonts, INK);
        let kk = k.clone();
        link(&mut commands, mem, k.clone(), &fonts, 13.0)
            .observe(move |_c: On<PointerClick>, mut sp: Sp| sp.set_term(kk.clone()));
        plain(&mut commands, mem, " · ", &fonts, INK);
        let node = st.spread.index(&k).map(|i| st.spread.nodes[i].clone());
        let bucket = node
            .as_ref()
            .filter(|_| !st.spread.dirty)
            .and_then(|n| st.spread.order.get(n.bucket));
        match bucket {
            Some(b) => {
                if b.n >= 2 {
                    commands.spawn((
                        Node {
                            width: px(10),
                            height: px(10),
                            margin: UiRect::right(px(4)),
                            border: UiRect::all(px(1)),
                            ..default()
                        },
                        BackgroundColor(bucket_color(b.q)),
                        BorderColor::all(colors::MOOD_DARK),
                        ChildOf(mem),
                    ));
                }
                plain(&mut commands, mem, "bucket ", &fonts, INK);
                let (q, rev) = (b.q, !rtb);
                link(&mut commands, mem, q.to_string(), &fonts, 13.0).observe(
                    move |_c: On<PointerClick>, mut sp: Sp| sp.envale(q.clamp(0, u32::MAX as i64) as u32, rev),
                );
                plain(&mut commands, mem, format!(" ×{}", b.n), &fonts, INK);
            }
            None => {
                plain(&mut commands, mem, "bucket —", &fonts, INK);
            }
        }
        if node.is_some_and(|n| n.pinned) {
            plain(&mut commands, mem, " · pinned", &fonts, INK);
        }
    }
    if m.s > 1 {
        pf_line(&mut commands, m_e, m.s as u64, rtb, &fonts, Role::Text);
    }
}

/// the sliders: double click a name to reset it
pub fn build_opts(
    mut st: ResMut<Space>,
    opts: Single<Entity, With<SpOpts>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if !std::mem::take(&mut st.redraw.opts) {
        return;
    }
    commands.entity(*opts).despawn_related::<Children>();
    for (i, s) in SLIDERS.iter().enumerate() {
        let key = s.key;
        let label = commands
            .spawn((
                Text::new(s.label),
                text_font(&fonts, false, 12.0),
                TextColor(INK),
                ChildOf(*opts),
            ))
            .id();
        commands.entity(label).observe(
            move |c: On<PointerClick>, mut sp: Sp, sliders: Query<(Entity, &SliderKey)>| {
                if c.count >= 2 {
                    let def = SLIDERS.iter().find(|s| s.key == key).map(|s| s.def).unwrap_or_default();
                    sp.set_slider(key, def);
                    for (e, k) in sliders.iter() {
                        if k.0 == key {
                            sp.commands.entity(e).insert(SliderValue(def as f32));
                        }
                    }
                }
            },
        );
        let decimals = (-s.step.log10()).ceil().max(0.0) as i32;
        let (min, max) = (s.min as f32, s.max as f32);
        let value = st.values[i] as f32;
        commands
            .spawn_scene(bsn! {
                @FeathersSlider {
                    @min: {min},
                    @max: {max},
                }
            })
            .insert((
                ChildOf(*opts),
                SliderValue(value),
                SliderStep(s.step as f32),
                SliderPrecision(decimals),
                SliderKey(key),
            ))
            .observe(move |c: On<ValueChange<f32>>, mut sp: Sp, mut commands: Commands| {
                commands.entity(c.source).insert(SliderValue(c.value));
                sp.set_slider(key, c.value as f64);
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
            ChildOf(*opts),
        ))
        .id();
    commands
        .spawn_scene(plain_scene_button("reset all".into(), ButtonVariant::Normal))
        .insert(ChildOf(buttons))
        .observe(|_a: On<Activate>, mut sp: Sp, sliders: Query<(Entity, &SliderKey)>| {
            for s in SLIDERS.iter() {
                sp.set_slider(s.key, s.def);
            }
            for (e, k) in sliders.iter() {
                if let Some(s) = SLIDERS.iter().find(|s| s.key == k.0) {
                    sp.commands.entity(e).insert(SliderValue(s.def as f32));
                }
            }
        });
    commands
        .spawn_scene(plain_scene_button("settle".into(), ButtonVariant::Normal))
        .insert(ChildOf(buttons))
        .observe(|_a: On<Activate>, mut sp: Sp| sp.settle());
}

#[derive(Component)]
pub struct SliderKey(pub &'static str);

const TRI_W: u32 = 40;
const TRI_H: u32 = 22;

/// a little picture of a text's triangle: filled faintly, edged in the bar's colour
fn triangle_image(t: &spread::Triangle) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let r = t.sides[1] as f64;
    let (ax, ay) = t.apex;
    let (min_x, max_x) = (0f64.min(ax), r.max(ax));
    let k = ((TRI_W as f64 - 3.0) / (max_x - min_x)).min((TRI_H as f64 - 3.0) / ay.max(1e-9));
    let ox = (TRI_W as f64 - (max_x - min_x) * k) / 2.0 - min_x * k;
    let oy = (TRI_H as f64 + ay * k) / 2.0;
    // corners in pixels, y down
    let p = [(ox, oy), (ox + r * k, oy), (ox + ax * k, oy - ay * k)];
    let edge = |(x0, y0): (f64, f64), (x1, y1): (f64, f64), x: f64, y: f64| {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = (dx * dx + dy * dy).sqrt().max(1e-9);
        let u = (((x - x0) * dx + (y - y0) * dy) / (len * len)).clamp(0.0, 1.0);
        ((x - x0 - u * dx).powi(2) + (y - y0 - u * dy).powi(2)).sqrt()
    };
    let side = |a: (f64, f64), b: (f64, f64), x: f64, y: f64| (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
    let mut data = vec![0u8; (TRI_W * TRI_H * 4) as usize];
    for py in 0..TRI_H {
        for px_ in 0..TRI_W {
            let (x, y) = (px_ as f64 + 0.5, py as f64 + 0.5);
            let d = edge(p[0], p[1], x, y)
                .min(edge(p[1], p[2], x, y))
                .min(edge(p[2], p[0], x, y));
            let (s0, s1, s2) = (side(p[0], p[1], x, y), side(p[1], p[2], x, y), side(p[2], p[0], x, y));
            let inside = (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0) || (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0);
            let line = (1.4 - d).clamp(0.0, 1.0);
            let fill = if inside { 0.22 } else { 0.0 };
            let a = line.max(fill);
            let i = ((py * TRI_W + px_) * 4) as usize;
            data[i..i + 4].copy_from_slice(&[235, 235, 235, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: TRI_W,
            height: TRI_H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}
