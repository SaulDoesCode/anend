//! The space's panels, remem's milk and coffee cards over the 3D view: the position card
//! (top left), help (top right), the toast, the metrics of the text under the pointer
//! (bottom right), the sliders (bottom left) and the terminal (bottom middle) with its
//! search line, sidecar and chips.

use bevy::feathers::constants::fonts;
use bevy::feathers::controls::{ButtonVariant, FeathersSlider};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::input_focus::{FocusCause, FocusedInput, InputFocus};
use bevy::picking::Pickable;
use bevy::picking::cursor::EntityCursor;
use bevy::picking::events::{PointerClick, PointerEnter, PointerLeave};
use bevy::prelude::*;
use bevy::text::{
    EditableText, FontSource, FontSourceTemplate, GenericFontFamily, TextCursorStyle, TextEdit, TextEditChange,
};
use bevy::ui_widgets::{Activate, ScrollArea, SliderPrecision, SliderStep, SliderValue, TextInput, ValueChange};
use bevy::window::SystemCursorIcon;

use crate::keys::{Action, Keymap, modifiers};
use crate::space::{ACCENT, INK, Qline, SLIDERS, Sp, Space, SpaceText, bucket_color};
use crate::spread;
use crate::ui::{Fonts, plain_scene_button};
use crate::weum::{self, Seg};

const MILK: Color = Color::srgb(0xfb as f32 / 255.0, 0xf6 as f32 / 255.0, 0xee as f32 / 255.0);
const COFFEE: Color = Color::srgb(0xa9 as f32 / 255.0, 0x83 as f32 / 255.0, 0x5d as f32 / 255.0);
const DARK: Color = Color::srgb(0x7c as f32 / 255.0, 0x5c as f32 / 255.0, 0x3d as f32 / 255.0);
const TERM_BG: Color = Color::srgb(0xff as f32 / 255.0, 0xf9 as f32 / 255.0, 0xf1 as f32 / 255.0);

const HINT_TERM: &str = "Enter search · Ctrl+Enter save · Shift+Enter key · Esc fly";
const HINT_EDIT: &str = "Enter done · Shift+Enter new line · Esc done · clearing the text deletes it";

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
#[derive(Component, Default, Clone)]
pub struct SpTermrow;

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
            // the position card
            Node {
                position_type: PositionType::Absolute,
                top: px(4),
                left: px(4),
                padding: UiRect::axes(px(12), px(8)),
                border: UiRect::all(px(1)),
            }
            BackgroundColor(MILK)
            BorderColor::all(COFFEE)
            SpHud
            Children [
                Text("")
                TextFont {
                    font: FontSourceTemplate::Handle(fonts::MONO),
                    font_size: FontSize::Px(13.0),
                }
                TextColor(INK)
                SpHudText
            ]
            --
            // help
            Node {
                position_type: PositionType::Absolute,
                top: px(4),
                right: px(4),
                max_height: percent(96),
                display: Display::None,
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(px(12), px(8)),
                border: UiRect::all(px(1)),
                overflow: Overflow::scroll_y(),
            }
            BackgroundColor(MILK)
            BorderColor::all(COFFEE)
            ScrollArea
            SpHelp
            --
            // the first time's hint
            Node {
                position_type: PositionType::Absolute,
                top: percent(12),
                left: percent(22),
                display: Display::None,
                padding: UiRect::axes(px(16), px(6)),
                border: UiRect::all(px(1)),
            }
            BackgroundColor(MILK)
            BorderColor::all(COFFEE)
            Pickable::IGNORE
            SpStart
            Children [
                Text("Hold the right mouse button to look · H for help")
                TextFont {
                    font: FontSourceTemplate::Handle(fonts::REGULAR),
                    font_size: FontSize::Px(15.0),
                }
                TextColor(INK)
            ]
            --
            // the toast
            Node {
                position_type: PositionType::Absolute,
                top: px(4),
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
                    padding: UiRect::axes(px(12), px(8)),
                    border: UiRect::all(px(1)),
                }
                BackgroundColor(MILK)
                BorderColor::all(COFFEE)
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
                position_type: PositionType::Absolute,
                bottom: px(14),
                right: px(4),
                width: px(300),
                display: Display::None,
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(px(12), px(8)),
                border: UiRect::all(px(1)),
            }
            BackgroundColor(MILK)
            BorderColor::all(COFFEE)
            SpMetrics
            --
            // the sliders
            Node {
                position_type: PositionType::Absolute,
                bottom: px(14),
                left: px(4),
                width: px(250),
                max_height: percent(70),
                display: Display::None,
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                padding: UiRect::axes(px(12), px(8)),
                border: UiRect::all(px(1)),
                overflow: Overflow::scroll_y(),
            }
            BackgroundColor(MILK)
            BorderColor::all(COFFEE)
            ScrollArea
            SpOpts
            --
            // the terminal
            Node {
                position_type: PositionType::Absolute,
                bottom: px(14),
                left: px(0),
                right: px(0),
                display: Display::Flex,
                justify_content: JustifyContent::Center,
            }
            Pickable::IGNORE
            GlobalZIndex(2)
            Children [
                Node {
                    width: px(720),
                    max_width: percent(56),
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(6),
                    padding: UiRect::axes(px(12), px(8)),
                    border: UiRect::all(px(1)),
                }
                BackgroundColor(MILK)
                BorderColor::all(COFFEE)
                Children [
                    Node {
                        display: Display::Flex,
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        align_items: AlignItems::Baseline,
                        max_height: px(90),
                        overflow: Overflow::scroll_y(),
                    }
                    ScrollArea
                    SpQline
                    --
                    Node {
                        padding: UiRect::axes(px(10), px(6)),
                        border: UiRect::all(px(1)),
                        min_height: px(24),
                    }
                    BorderColor::all(COFFEE)
                    BackgroundColor(TERM_BG)
                    TextInput
                    EditableText
                    TextLayout {
                        linebreak: LineBreak::WordBoundary,
                    }
                    TextFont {
                        font: FontSourceTemplate::Handle(fonts::REGULAR),
                        font_size: FontSize::Px(15.0),
                    }
                    TextColor(INK)
                    TextCursorStyle {
                        color: INK,
                    }
                    TabIndex(0)
                    EntityCursor::System(SystemCursorIcon::Text)
                    SpTerm
                    on(term_key)
                    on(term_changed)
                    --
                    Node {
                        display: Display::Flex,
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        align_items: AlignItems::Center,
                        column_gap: px(14),
                        row_gap: px(4),
                    }
                    SpTermrow
                ]
            ]
        ]
    }
}

// ------------------------------------------------------------------ the terminal

fn term_key(
    key: On<FocusedInput<KeyboardInput>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut terms: Query<&mut EditableText, With<SpTerm>>,
    mut focus: ResMut<InputFocus>,
    mut sp: Sp,
) {
    if !key.input.state.is_pressed() {
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

fn term_changed(change: On<TextEditChange>, terms: Query<&EditableText, With<SpTerm>>, mut sp: Sp) {
    if let Ok(term) = terms.get(change.event_target()) {
        sp.box_input(term.value().to_string());
    }
}

/// puts text into the box, focuses and blurs it, and marks it while editing a text
pub fn apply_term(
    mut st: ResMut<Space>,
    term: Single<(Entity, &mut EditableText, &mut BorderColor), With<SpTerm>>,
    mut focus: ResMut<InputFocus>,
) {
    let (entity, mut term, mut border) = term.into_inner();
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
        *border = BorderColor::all(if st.editing.is_some() { ACCENT } else { COFFEE });
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

fn bold(commands: &mut Commands, parent: Entity, s: impl Into<String>, fonts: &Fonts) -> Entity {
    commands
        .spawn((
            Text::new(s),
            text_font(fonts, true, 13.0),
            TextColor(INK),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

/// a clickable bit of text that turns accent under the pointer
fn link<'a>(
    commands: &'a mut Commands,
    parent: Entity,
    s: impl Into<String>,
    fonts: &Fonts,
    size: f32,
) -> EntityCommands<'a> {
    let mut e = commands.spawn((
        Text::new(s),
        text_font(fonts, false, size),
        TextColor(INK),
        EntityCursor::System(SystemCursorIcon::Pointer),
        ChildOf(parent),
    ));
    e.observe(|e: On<PointerEnter>, mut colors: Query<&mut TextColor>| {
        if let Ok(mut c) = colors.get_mut(e.event_target()) {
            c.0 = ACCENT;
        }
    })
    .observe(|e: On<PointerLeave>, mut colors: Query<&mut TextColor>| {
        if let Ok(mut c) = colors.get_mut(e.event_target()) {
            c.0 = INK;
        }
    });
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
fn pf_line(commands: &mut Commands, parent: Entity, n: u64, rev: bool, fonts: &Fonts) {
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
            TextColor(INK),
            Pickable::IGNORE,
            ChildOf(r),
        ));
    };
    small(commands, "(".into());
    for seg in weum::pf_reduction(n) {
        match seg {
            Seg::Text(t) => small(commands, t),
            Seg::Num(v) => {
                link(commands, r, v.to_string(), fonts, 11.0)
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
            "x {:.2}\ny {:.2}\nz {:.2}\nspeed {:.1}\nroll {}\ntexts {}\nspread {} {}",
            p.x,
            p.y,
            p.z,
            st.speed,
            roll,
            n,
            st.spread.nodes.len(),
            state
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

/// the line over the terminal: what was searched, and how many texts and buckets
pub fn rebuild_qline(
    mut st: ResMut<Space>,
    qline: Single<Entity, With<SpQline>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if !std::mem::take(&mut st.redraw.qline) {
        return;
    }
    let q = *qline;
    commands.entity(q).despawn_related::<Children>();
    let count = format!(" · {} texts · {} buckets", st.mem_index.len(), st.spread.order.len());
    match st.qline.clone() {
        Qline::None => {
            if !st.mem_index.is_empty() {
                plain(&mut commands, q, count.trim_start_matches(" · "), &fonts, INK);
            }
        }
        Qline::Gsearch { n, rev } => {
            bold(&mut commands, q, n.to_string(), &fonts);
            plain(
                &mut commands,
                q,
                format!(" {}{}", if rev { "reversed" } else { "ordinal" }, count),
                &fonts,
                INK,
            );
            if n > 1 {
                let holder = commands
                    .spawn((
                        Node {
                            width: percent(100),
                            ..default()
                        },
                        ChildOf(q),
                    ))
                    .id();
                pf_line(&mut commands, holder, n as u64, rev, &fonts);
            }
        }
        Qline::Key(k) | Qline::List(k) => {
            let word = if matches!(st.qline, Qline::Key(_)) {
                "key "
            } else {
                "list "
            };
            plain(&mut commands, q, word, &fonts, INK);
            bold(&mut commands, q, k, &fonts);
            plain(&mut commands, q, count, &fonts, INK);
        }
        Qline::Search(v) => {
            plain(&mut commands, q, format!("search “{}”{}", v, count), &fonts, INK);
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
                INK,
            );
            for name in names {
                plain(&mut commands, q, " · ", &fonts, INK);
                let n = name.clone();
                link(&mut commands, q, name, &fonts, 13.0)
                    .observe(move |_c: On<PointerClick>, mut sp: Sp| sp.route(&n));
            }
        }
    }
}

/// the sidecar of the terminal's sums, the chips, the list state and the hint
pub fn rebuild_termrow(
    mut st: ResMut<Space>,
    termrow: Single<Entity, With<SpTermrow>>,
    fonts: Res<Fonts>,
    mut commands: Commands,
) {
    if !std::mem::take(&mut st.redraw.termrow) {
        return;
    }
    let t = *termrow;
    commands.entity(t).despawn_related::<Children>();
    let side = row(&mut commands, t);
    let (s, r) = spread::sums(&st.term);
    plain(&mut commands, side, "(", &fonts, INK);
    for (i, v) in [s, r].into_iter().enumerate() {
        if i == 1 {
            plain(&mut commands, side, ", ", &fonts, INK);
        }
        link(&mut commands, side, v.to_string(), &fonts, 13.0).observe(move |_c: On<PointerClick>, mut sp: Sp| {
            sp.set_reversed(i == 1);
            sp.envale(v.clamp(0, u32::MAX as i64) as u32, i == 1);
        });
    }
    plain(&mut commands, side, ")", &fonts, INK);

    let chips = st.chips;
    let chip = |commands: &mut Commands, label: &str, on: bool, f: fn(&mut crate::space::Chips)| {
        let c = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(4),
                    ..default()
                },
                EntityCursor::System(SystemCursorIcon::Pointer),
                ChildOf(t),
            ))
            .observe(move |_c: On<PointerClick>, mut sp: Sp| {
                let mut c = sp.st.chips;
                f(&mut c);
                sp.set_chips(c);
            })
            .id();
        commands.spawn((
            Node {
                width: px(11),
                height: px(11),
                border: UiRect::all(px(1)),
                ..default()
            },
            BackgroundColor(if on { DARK } else { TERM_BG }),
            BorderColor::all(DARK),
            Pickable::IGNORE,
            ChildOf(c),
        ));
        plain(commands, c, label, &fonts, INK);
    };
    chip(&mut commands, "gsearch", chips.gsch, |c| c.gsch = !c.gsch);
    chip(&mut commands, "reversed", chips.rtb, |c| c.rtb = !c.rtb);
    chip(&mut commands, "saturate", chips.also_saturate, |c| {
        c.also_saturate = !c.also_saturate
    });
    chip(&mut commands, "keep", chips.keep, |c| c.keep = !c.keep);

    let list = match (&st.list_id, &st.list_candidate) {
        (Some(l), Some(c)) => format!("list {} · candidate {}", l.label(), c.label()),
        (Some(l), None) => format!("list {}", l.label()),
        (None, Some(c)) => format!("candidate {}", c.label()),
        (None, None) => String::new(),
    };
    if !list.is_empty() {
        plain(&mut commands, t, list, &fonts, INK);
    }
    let hint = commands
        .spawn((
            Text::new(if st.editing.is_some() { HINT_EDIT } else { HINT_TERM }),
            TextFont {
                font: (&fonts.regular).into(),
                font_size: FontSize::Px(11.0),
                ..default()
            },
            TextColor(INK.with_alpha(0.75)),
            Node {
                margin: UiRect::left(Val::Auto),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(t),
        ))
        .id();
    let _ = hint;
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
                        BorderColor::all(COFFEE),
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
        pf_line(&mut commands, m_e, m.s as u64, rtb, &fonts);
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
