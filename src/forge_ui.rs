//! The forge menu, laid out like Oblivion's spellmaking altar: the saves you have on the
//! left, the ingredients you've chosen (each with its effect) in the middle, and the
//! result on the right: what it holds, a few of its memories with their sums, filters to
//! narrow it, a name, and the forge. Forging makes a new save; pouring puts the result
//! into the open one. See forge.rs for what the effects do.

use bevy::feathers::controls::{ButtonVariant, FeathersTextInput, FeathersTextInputContainer};
use bevy::picking::Pickable;
use bevy::picking::cursor::EntityCursor;
use bevy::picking::events::PointerClick;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEdit, TextEditChange};
use bevy::ui_widgets::{Activate, ScrollArea};
use bevy::window::SystemCursorIcon;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::{Config, clean_name, save_name};
use crate::forge::{self, Contents, Effect, Filter};
use crate::settings::{Settings, unique};
use crate::spread;
use crate::store::Store;
use crate::theme::colors;
use crate::ui::{Db, Fonts, Status, plain_scene_button, text};

/// how many of the result's memories show
const SHOWN: usize = 18;

#[derive(Resource, Default)]
pub struct ForgeState {
    pub open: bool,
    /// rebuild the lists and the result
    pub changed: bool,
    /// the saves in the folder, read when the forge opens
    saves: Vec<PathBuf>,
    ingredients: Vec<(PathBuf, Effect)>,
    filter: Filter,
    name: String,
    /// what each save holds, read once per opening (the open save is read each time)
    cache: HashMap<PathBuf, Contents>,
    result: Option<Contents>,
    note: String,
    /// pour the result into the open save, next frame (it needs the whole world)
    pour: bool,
    fill_inputs: bool,
}

impl ForgeState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.changed = true;
        self.cache.clear();
        self.note.clear();
        self.fill_inputs = true;
    }
}

#[derive(Component, Default, Clone)]
pub struct ForgeMenu;
#[derive(Component, Default, Clone)]
pub struct ForgeSaves;
#[derive(Component, Default, Clone)]
pub struct ForgeIngredients;
#[derive(Component, Default, Clone)]
pub struct ForgeResult;
#[derive(Component, Default, Clone)]
pub struct ForgeKinds;
#[derive(Component, Default, Clone)]
pub struct ForgeNote;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Field {
    #[default]
    Name,
    Words,
    OrdinalFrom,
    OrdinalTo,
    ReversedFrom,
    ReversedTo,
}

#[derive(Component, Default, Clone)]
pub struct ForgeField(pub Field);

fn number(s: &str) -> Option<i64> {
    s.trim().parse().ok()
}

fn edit_field(c: On<TextEditChange>, inputs: Query<(&ForgeField, &EditableText)>, mut f: ResMut<ForgeState>) {
    let Ok((field, t)) = inputs.get(c.event_target()) else {
        return;
    };
    let v = t.value().to_string();
    match field.0 {
        Field::Name => {
            f.name = v;
            return;
        }
        Field::Words => f.filter.words = v,
        Field::OrdinalFrom => f.filter.ordinal.0 = number(&v),
        Field::OrdinalTo => f.filter.ordinal.1 = number(&v),
        Field::ReversedFrom => f.filter.reversed.0 = number(&v),
        Field::ReversedTo => f.filter.reversed.1 = number(&v),
    }
    f.changed = true;
}

fn small_text(s: &'static str) -> impl Scene {
    bsn! {
        Text(s)
        TextFont {
            font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
            font_size: FontSize::Px(13.0),
        }
        TextColor(colors::TEXT_DIM)
    }
}

fn column_title(s: &'static str) -> impl Scene {
    bsn! {
        Text(s)
        TextFont {
            font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::BOLD),
            font_size: FontSize::Px(15.0),
        }
        TextColor(colors::TEXT)
    }
}

fn field_box(field: Field, width: f32) -> impl Scene {
    bsn! {
        @FeathersTextInputContainer
        Node { width: px(width), flex_grow: 0.0 }
        Children [
            @FeathersTextInput
            ForgeField(field)
            on(edit_field)
        ]
    }
}

/// a column of the altar
fn column() -> Node {
    Node {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        row_gap: px(6),
        padding: UiRect::all(px(10)),
        border_radius: BorderRadius::all(px(4)),
        min_height: px(0),
        ..default()
    }
}

pub fn menu() -> impl Scene {
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
        ForgeMenu
        Children [
            Node {
                width: px(1040),
                max_width: percent(96),
                height: percent(90),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
                padding: UiRect::all(px(14)),
                border_radius: BorderRadius::all(px(6)),
            }
            BackgroundColor(colors::CARD)
            Children [
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(10),
                }
                Children [
                    Text("the forge")
                    TextFont {
                        font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::BOLD),
                        font_size: FontSize::Px(22.0),
                    }
                    TextColor(colors::TEXT)
                    --
                    @small_text("saves in, a save out: + adds one, − takes it away, ∩ keeps only what they share")
                    --
                    Node { flex_grow: 1.0 }
                    --
                    @plain_scene_button("close".into(), ButtonVariant::Primary)
                    on(|_a: On<Activate>, mut f: ResMut<ForgeState>| f.toggle())
                ]
                --
                Node {
                    flex_grow: 1.0,
                    min_height: px(0),
                    display: Display::Grid,
                    grid_template_columns: vec![RepeatedGridTrack::fr(1, 1.0), RepeatedGridTrack::fr(1, 1.0), RepeatedGridTrack::fr(1, 1.3)],
                    column_gap: px(8),
                }
                Children [
                    // the saves to choose from
                    Node { ..column() }
                    BackgroundColor(colors::CODE_BG)
                    Children [
                        @column_title("your saves")
                        --
                        Node {
                            display: Display::Flex,
                            flex_direction: FlexDirection::Column,
                            row_gap: px(3),
                            overflow: Overflow::scroll_y(),
                            flex_grow: 1.0,
                            min_height: px(0),
                        }
                        ScrollArea
                        ForgeSaves
                    ]
                    --
                    // the ingredients, in order
                    Node { ..column() }
                    BackgroundColor(colors::CODE_BG)
                    Children [
                        @column_title("ingredients")
                        --
                        Node {
                            display: Display::Flex,
                            flex_direction: FlexDirection::Column,
                            row_gap: px(3),
                            overflow: Overflow::scroll_y(),
                            flex_grow: 1.0,
                            min_height: px(0),
                        }
                        ScrollArea
                        ForgeIngredients
                    ]
                    --
                    // the result
                    Node { ..column() }
                    BackgroundColor(colors::CODE_BG)
                    Children [
                        @column_title("the result")
                        --
                        Node {
                            display: Display::Flex,
                            flex_direction: FlexDirection::Column,
                            row_gap: px(3),
                            overflow: Overflow::scroll_y(),
                            flex_grow: 1.0,
                            min_height: px(0),
                        }
                        ScrollArea
                        ForgeResult
                        --
                        Node {
                            flex_direction: FlexDirection::Row,
                            flex_wrap: FlexWrap::Wrap,
                            align_items: AlignItems::Center,
                            column_gap: px(6),
                            row_gap: px(4),
                        }
                        Children [
                            @small_text("words")
                            --
                            @field_box(Field::Words, 180.0)
                            --
                            Node { width: px(8) }
                            --
                            @small_text("ordinal")
                            --
                            @field_box(Field::OrdinalFrom, 48.0)
                            --
                            @small_text("to")
                            --
                            @field_box(Field::OrdinalTo, 48.0)
                            --
                            @small_text("reversed")
                            --
                            @field_box(Field::ReversedFrom, 48.0)
                            --
                            @small_text("to")
                            --
                            @field_box(Field::ReversedTo, 48.0)
                        ]
                        --
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: px(6),
                        }
                        ForgeKinds
                        --
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: px(6),
                        }
                        Children [
                            @small_text("name")
                            --
                            @field_box(Field::Name, 170.0)
                            --
                            @plain_scene_button("forge".into(), ButtonVariant::Primary)
                            on(forge_new)
                            --
                            @plain_scene_button("pour into the open save".into(), ButtonVariant::Normal)
                            on(|_a: On<Activate>, mut f: ResMut<ForgeState>| {
                                f.pour = true;
                            })
                        ]
                        --
                        Text("")
                        TextFont {
                            font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
                            font_size: FontSize::Px(13.0),
                        }
                        TextColor(colors::LINK)
                        ForgeNote
                    ]
                ]
            ]
        ]
    }
}

fn same(a: &Path, b: &Path) -> bool {
    crate::vfs::same(a, b)
}

/// what a save holds: the open one from the open store, others read once
fn contents(f: &mut ForgeState, db: &Store, path: &Path) -> Result<Contents, String> {
    if same(path, db.path()) {
        return forge::read(db);
    }
    if let Some(c) = f.cache.get(path) {
        return Ok(c.clone());
    }
    let store = Store::open(path).map_err(|e| format!("couldn't open {}: {e}", save_name(path)))?;
    let c = forge::read(&store)?;
    f.cache.insert(path.to_path_buf(), c.clone());
    Ok(c)
}

/// the ingredients combined and filtered
fn brew(f: &mut ForgeState, db: &Store) -> Result<Contents, String> {
    let mut parts = Vec::new();
    for (path, effect) in f.ingredients.clone() {
        parts.push((effect, contents(f, db, &path)?));
    }
    let refs: Vec<(Effect, &Contents)> = parts.iter().map(|(e, c)| (*e, c)).collect();
    Ok(f.filter.apply(forge::combine(&refs)))
}

fn forge_new(
    _a: On<Activate>,
    mut f: ResMut<ForgeState>,
    db: Res<Db>,
    config: Res<Config>,
    mut settings: ResMut<Settings>,
    mut status: ResMut<Status>,
    time: Res<Time>,
) {
    let Some(result) = f.result.clone() else {
        f.note = "add an ingredient first".into();
        f.changed = true;
        return;
    };
    let name = clean_name(&f.name)
        .or_else(|| f.ingredients.first().map(|(p, _)| format!("{} forged", save_name(p))))
        .unwrap_or_else(|| "forged".into());
    let path = unique(&config.saves_dir, &name);
    let made = (|| -> Result<forge::Written, String> {
        let out = Store::open(&path)?;
        if let Some((first, _)) = f.ingredients.first() {
            if same(first, db.0.path()) {
                forge::copy_settings(&db.0, &out)?;
            } else if let Ok(src) = Store::open(first) {
                forge::copy_settings(&src, &out)?;
            }
        }
        forge::write_into(&out, &result)
    })();
    f.note = match made {
        Ok(w) => {
            settings.rescan = true;
            settings.changed = true;
            let msg = format!(
                "forged {}: {} memories, {} notes, {} list links; load it from the saves",
                save_name(&path),
                w.memories,
                w.notes,
                w.lists
            );
            status.text = msg.clone();
            status.until = time.elapsed_secs_f64() + 5.0;
            msg
        }
        Err(e) => {
            let _ = crate::vfs::remove(&path);
            format!("couldn't forge it: {e}")
        }
    };
    f.cache.remove(&path);
    f.changed = true;
}

/// pours the result into the open save, then reloads it
pub fn pour(world: &mut World) {
    if !std::mem::take(&mut world.resource_mut::<ForgeState>().pour) {
        return;
    }
    let Some(result) = world.resource::<ForgeState>().result.clone() else {
        let mut f = world.resource_mut::<ForgeState>();
        f.note = "add an ingredient first".into();
        f.changed = true;
        return;
    };
    let written = forge::write_into(&world.resource::<Db>().0, &result);
    let note = match written {
        Ok(w) => {
            crate::settings::reload_open(world);
            format!(
                "poured in {} memories, {} notes, {} list links (what it had already stayed as it was)",
                w.memories, w.notes, w.lists
            )
        }
        Err(e) => format!("couldn't pour it in: {e}"),
    };
    let mut f = world.resource_mut::<ForgeState>();
    f.note = note;
    f.changed = true;
    world.resource_mut::<Settings>().rescan = true;
    world.resource_mut::<Settings>().changed = true;
}

fn small(commands: &mut Commands, parent: Entity, s: &str, fonts: &Fonts, color: Color) -> Entity {
    let e = text(commands, parent, s, &fonts.regular, 13.0, color);
    commands.entity(e).insert(Pickable::IGNORE);
    e
}

fn row(commands: &mut Commands, parent: Entity) -> Entity {
    commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(6),
                padding: UiRect::axes(px(6), px(3)),
                border_radius: BorderRadius::all(px(3)),
                ..default()
            },
            BackgroundColor(colors::CARD),
            ChildOf(parent),
        ))
        .id()
}

fn button<'a>(commands: &'a mut Commands, parent: Entity, label: &str, variant: ButtonVariant) -> EntityCommands<'a> {
    let mut e = commands.spawn_scene(plain_scene_button(label.to_string(), variant));
    e.insert(ChildOf(parent));
    e
}

#[allow(clippy::too_many_arguments)]
pub fn rebuild_forge(
    mut f: ResMut<ForgeState>,
    config: Res<Config>,
    db: Res<Db>,
    fonts: Res<Fonts>,
    keys: Res<ButtonInput<KeyCode>>,
    mut menu: Single<&mut Node, With<ForgeMenu>>,
    parts: (
        Single<Entity, With<ForgeSaves>>,
        Single<Entity, With<ForgeIngredients>>,
        Single<Entity, With<ForgeResult>>,
        Single<Entity, With<ForgeKinds>>,
    ),
    mut note: Single<&mut Text, With<ForgeNote>>,
    mut inputs: Query<(&ForgeField, &mut EditableText)>,
    mut commands: Commands,
) {
    if f.open && keys.just_pressed(KeyCode::Escape) {
        f.toggle();
    }
    if !f.changed {
        return;
    }
    f.changed = false;
    menu.display = if f.open { Display::Flex } else { Display::None };
    if !f.open {
        return;
    }
    let (saves_box, ingredients_box, result_box, kinds_box) = (*parts.0, *parts.1, *parts.2, *parts.3);
    if std::mem::take(&mut f.fill_inputs) {
        f.saves = crate::vfs::list(&config.saves_dir)
            .into_iter()
            .filter(|p| p.extension().is_some_and(|x| x == "redb"))
            .collect();
        if !f.saves.iter().any(|p| same(p, db.0.path())) {
            f.saves.push(db.0.path().to_path_buf());
        }
        f.saves.sort_by_key(|p| save_name(p).to_lowercase());
        for (field, mut t) in &mut inputs {
            let v = match field.0 {
                Field::Name => f.name.clone(),
                Field::Words => f.filter.words.clone(),
                Field::OrdinalFrom => f.filter.ordinal.0.map(|n| n.to_string()).unwrap_or_default(),
                Field::OrdinalTo => f.filter.ordinal.1.map(|n| n.to_string()).unwrap_or_default(),
                Field::ReversedFrom => f.filter.reversed.0.map(|n| n.to_string()).unwrap_or_default(),
                Field::ReversedTo => f.filter.reversed.1.map(|n| n.to_string()).unwrap_or_default(),
            };
            t.editor_mut().set_text(&v);
            t.queue_edit(TextEdit::TextEnd(false));
        }
    }

    // the saves, each with its three effects
    commands.entity(saves_box).despawn_related::<Children>();
    for path in f.saves.clone() {
        let r = row(&mut commands, saves_box);
        let open = same(&path, db.0.path());
        let label = if open {
            format!("{} · open", save_name(&path))
        } else {
            save_name(&path)
        };
        let t = small(&mut commands, r, &label, &fonts, colors::TEXT);
        commands.entity(t).insert(Node {
            flex_grow: 1.0,
            ..default()
        });
        for effect in Effect::ALL {
            let p = path.clone();
            button(&mut commands, r, effect.glyph(), ButtonVariant::Normal).observe(
                move |_a: On<Activate>, mut f: ResMut<ForgeState>| {
                    // the first ingredient is what the others work on, so it's always added
                    let effect = if f.ingredients.is_empty() { Effect::Add } else { effect };
                    f.ingredients.push((p.clone(), effect));
                    f.changed = true;
                },
            );
        }
    }

    // the ingredients
    commands.entity(ingredients_box).despawn_related::<Children>();
    if f.ingredients.is_empty() {
        small(
            &mut commands,
            ingredients_box,
            "Choose saves on the left. The first is the base; each one after it adds to it, takes away from it, or keeps only what they share.",
            &fonts,
            colors::TEXT_DIM,
        );
    }
    for (i, (path, effect)) in f.ingredients.clone().into_iter().enumerate() {
        let r = row(&mut commands, ingredients_box);
        small(&mut commands, r, &format!("{}.", i + 1), &fonts, colors::TEXT_DIM);
        let variant = match effect {
            Effect::Add => ButtonVariant::Primary,
            _ => ButtonVariant::Normal,
        };
        button(
            &mut commands,
            r,
            &format!("{} {}", effect.glyph(), effect.name()),
            variant,
        )
        .observe(move |_a: On<Activate>, mut f: ResMut<ForgeState>| {
            if i == 0 {
                return;
            }
            let next = match f.ingredients[i].1 {
                Effect::Add => Effect::TakeAway,
                Effect::TakeAway => Effect::KeepShared,
                Effect::KeepShared => Effect::Add,
            };
            f.ingredients[i].1 = next;
            f.changed = true;
        });
        let t = small(&mut commands, r, &save_name(&path), &fonts, colors::TEXT);
        commands.entity(t).insert(Node {
            flex_grow: 1.0,
            ..default()
        });
        let x = commands
            .spawn((
                Text::new("×"),
                TextFont {
                    font: fonts.regular.clone().into(),
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(colors::TEXT_DIM),
                EntityCursor::System(SystemCursorIcon::Pointer),
                ChildOf(r),
            ))
            .id();
        commands
            .entity(x)
            .observe(move |_c: On<PointerClick>, mut f: ResMut<ForgeState>| {
                if i < f.ingredients.len() {
                    f.ingredients.remove(i);
                    if let Some(first) = f.ingredients.first_mut() {
                        first.1 = Effect::Add;
                    }
                    f.changed = true;
                }
            });
    }

    // which kinds of things go in
    commands.entity(kinds_box).despawn_related::<Children>();
    small(&mut commands, kinds_box, "keep", &fonts, colors::TEXT_DIM);
    for (label, on) in [
        ("memories", f.filter.memories),
        ("notes", f.filter.notes),
        ("lists", f.filter.lists),
    ] {
        let variant = if on {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Normal
        };
        button(&mut commands, kinds_box, label, variant).observe(move |_a: On<Activate>, mut f: ResMut<ForgeState>| {
            match label {
                "memories" => f.filter.memories = !f.filter.memories,
                "notes" => f.filter.notes = !f.filter.notes,
                _ => f.filter.lists = !f.filter.lists,
            }
            f.changed = true;
        });
    }
    small(
        &mut commands,
        kinds_box,
        "· words: all of them, -word for none of it",
        &fonts,
        colors::TEXT_DIM,
    );

    // the result
    commands.entity(result_box).despawn_related::<Children>();
    f.result = None;
    if f.ingredients.is_empty() {
        small(&mut commands, result_box, "Nothing yet.", &fonts, colors::TEXT_DIM);
    } else {
        match brew(&mut f, &db.0) {
            Ok(c) => {
                let stats = commands
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: px(18),
                            margin: UiRect::bottom(px(4)),
                            ..default()
                        },
                        ChildOf(result_box),
                    ))
                    .id();
                for (n, what) in [
                    (c.memories.len(), "memories"),
                    (c.notes.len(), "notes"),
                    (c.lists.len(), "list links"),
                ] {
                    let s = commands
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                ..default()
                            },
                            ChildOf(stats),
                        ))
                        .id();
                    text(&mut commands, s, &n.to_string(), &fonts.bold, 20.0, colors::TEXT);
                    small(&mut commands, s, what, &fonts, colors::TEXT_DIM);
                }
                for m in c.memories.iter().take(SHOWN) {
                    let r = row(&mut commands, result_box);
                    let t = small(&mut commands, r, m, &fonts, colors::TEXT);
                    commands.entity(t).insert(Node {
                        flex_grow: 1.0,
                        ..default()
                    });
                    let (s, rv) = spread::sums(m);
                    small(&mut commands, r, &format!("{s} · {rv}"), &fonts, colors::TEXT_DIM);
                }
                if c.memories.len() > SHOWN {
                    small(
                        &mut commands,
                        result_box,
                        &format!("and {} more", c.memories.len() - SHOWN),
                        &fonts,
                        colors::TEXT_DIM,
                    );
                }
                if !c.notes.is_empty() {
                    let titles: Vec<&str> = c.notes.iter().take(8).map(|n| n.display_title()).collect();
                    let more = if c.notes.len() > 8 {
                        format!(" and {} more", c.notes.len() - 8)
                    } else {
                        String::new()
                    };
                    small(
                        &mut commands,
                        result_box,
                        &format!("notes: {}{}", titles.join(", "), more),
                        &fonts,
                        colors::TEXT_DIM,
                    );
                }
                f.result = Some(c);
            }
            Err(e) => {
                small(&mut commands, result_box, &e, &fonts, colors::LINK);
            }
        }
    }
    if note.0 != f.note {
        note.0 = f.note.clone();
    }
}
