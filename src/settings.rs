//! The settings menu: the saves (each database is a save you can load, start, copy,
//! rename or delete, like a game's), where they're kept, a hub to share them on (see
//! hub_ui.rs), the fonts, zen mode and the top bar. Opened from the top bar or with Ctrl+, ; Escape closes it.

use bevy::feathers::controls::{
    ButtonVariant, ColorChannel, ColorSlider, FeathersColorSlider, FeathersTextInput, FeathersTextInputContainer,
    SliderBaseColor,
};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::FocusedInput;
use bevy::picking::Pickable;
use bevy::picking::cursor::EntityCursor;
use bevy::picking::events::PointerClick;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEdit, TextEditChange};
use bevy::ui_widgets::{Activate, ScrollArea, SliderValue, ValueChange};
use bevy::window::SystemCursorIcon;
use std::path::{Path, PathBuf};

use crate::config::{Config, clean_name, save_name};
use crate::memory_ui::MemoryState;
use crate::palette::{self, Palette, Role};
use crate::space::{Space, SpaceText};
use crate::store::Store;
use crate::theme::colors;
use crate::ui::{Ctx, Db, Fonts, Notes, Redraw, Session, Status, plain_scene_button, text};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum FontTarget {
    #[default]
    Interface,
    Memory,
}

pub struct SaveInfo {
    pub path: PathBuf,
    pub counts: Option<(u64, u64)>,
    pub modified: Option<chrono::DateTime<chrono::Local>>,
    pub size: u64,
    pub current: bool,
}

#[derive(Resource, Default)]
pub struct Settings {
    pub open: bool,
    /// rebuild the menu's lists
    pub changed: bool,
    /// read the saves folder again
    pub rescan: bool,
    pub saves: Vec<SaveInfo>,
    pub new_name: String,
    pub folder: String,
    pub renaming: Option<PathBuf>,
    pub rename_to: String,
    pub confirm_delete: Option<PathBuf>,
    pub font_target: FontTarget,
    pub font_filter: String,
    pub families: Option<Vec<String>>,
    pub message: String,
    /// the colour being edited, an index into `Role::ALL`
    pub color_role: usize,
    /// its colour as the sliders see it (a hex rounds the hue away on greys)
    pub hsl: Hsla,
    /// put the colour into the sliders and the hex box
    pub sync_colors: bool,
    /// a slider is moving: leave the sliders' values alone
    sliding: bool,
    /// put the saves folder in its box
    fill_folder: bool,
}

impl Settings {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.changed = true;
        self.rescan = true;
        self.fill_folder = true;
        self.renaming = None;
        self.confirm_delete = None;
        self.message.clear();
        self.sync_colors = true;
    }
}

#[derive(Component, Default, Clone)]
pub struct SettingsMenu;
#[derive(Component, Default, Clone)]
pub struct SavesList;
#[derive(Component, Default, Clone)]
pub struct NewSaveInput;
#[derive(Component, Default, Clone)]
pub struct FolderInput;
#[derive(Component, Default, Clone)]
pub struct FontTabs;
#[derive(Component, Default, Clone)]
pub struct FontFilterInput;
#[derive(Component, Default, Clone)]
pub struct FontList;
#[derive(Component, Default, Clone)]
pub struct Toggles;
#[derive(Component, Default, Clone)]
pub struct SettingsMessage;
#[derive(Component, Default, Clone)]
pub struct RenameInput;
#[derive(Component, Default, Clone)]
pub struct ColorPresets;
#[derive(Component, Default, Clone)]
pub struct ColorRoles;
#[derive(Component, Default, Clone)]
pub struct HexInput;
#[derive(Component, Default, Clone)]
pub struct SizeRows;
/// a colour swatch: always shows its colour, whatever the palette does
#[derive(Component)]
#[require(BackgroundColor)]
pub struct Swatch(pub Color);

pub fn paint_swatches(mut q: Query<(&Swatch, &mut BackgroundColor)>) {
    for (s, mut bg) in &mut q {
        if bg.0 != s.0 {
            bg.0 = s.0;
        }
    }
}

/// one of the colour editor's sliders, with its label
fn color_slider(channel: ColorChannel, label: &'static str) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(8),
        }
        Children [
            Text(label)
            TextFont {
                font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
                font_size: FontSize::Px(13.0),
            }
            TextColor(colors::TEXT_DIM)
            Node { width: px(70) }
            --
            @FeathersColorSlider {
                @value: 0.5,
                @channel: channel,
            }
            on(slide_color)
        ]
    }
}

/// a colour slider moved: the role's colour follows
fn slide_color(
    change: On<ValueChange<f32>>,
    mut s: ResMut<Settings>,
    mut config: ResMut<Config>,
    sliders: Query<&ColorSlider>,
    mut commands: Commands,
) {
    let Ok(slider) = sliders.get(change.event_target()) else {
        return;
    };
    let v = change.value;
    match slider.channel {
        ColorChannel::HslHue => s.hsl.hue = v,
        ColorChannel::HslSaturation => s.hsl.saturation = v,
        ColorChannel::HslLightness => s.hsl.lightness = v,
        _ => return,
    }
    commands.entity(change.event_target()).insert(SliderValue(v));
    let role = Role::ALL[s.color_role];
    config
        .colors
        .insert(role.name().to_string(), palette::to_hex(Color::from(s.hsl)));
    config.save();
    s.sync_colors = true;
    s.sliding = true;
}

/// the chosen role takes a colour (from the hex box)
fn set_role_color(s: &mut Settings, config: &mut Config, c: Color) {
    let role = Role::ALL[s.color_role];
    config.colors.insert(role.name().to_string(), palette::to_hex(c));
    config.save();
    s.hsl = c.into();
    s.sync_colors = true;
    s.changed = true;
}

/// puts the chosen colour in the sliders and the hex box
pub fn sync_color_editor(
    mut s: ResMut<Settings>,
    config: Res<Config>,
    mut sliders: Query<(Entity, &ColorSlider, &mut SliderBaseColor)>,
    mut hex: Single<&mut EditableText, With<HexInput>>,
    mut commands: Commands,
) {
    if !std::mem::take(&mut s.sync_colors) {
        return;
    }
    let sliding = std::mem::take(&mut s.sliding);
    if !sliding {
        let role = Role::ALL[s.color_role];
        s.hsl = Palette::from_config(&config.colors).get(role).into();
    }
    let c = Color::from(s.hsl);
    for (e, slider, mut base) in &mut sliders {
        base.0 = c;
        if !sliding {
            let v = match slider.channel {
                ColorChannel::HslHue => s.hsl.hue,
                ColorChannel::HslSaturation => s.hsl.saturation,
                ColorChannel::HslLightness => s.hsl.lightness,
                _ => continue,
            };
            commands.entity(e).insert(SliderValue(v));
        }
    }
    let h = palette::to_hex(c);
    if hex.value().to_string() != h {
        hex.editor_mut().set_text(&h);
        hex.queue_edit(TextEdit::TextEnd(false));
    }
}

pub(crate) fn heading(label: &'static str) -> impl Scene {
    bsn! {
        Text(label)
        TextFont {
            font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::BOLD),
            font_size: FontSize::Px(15.0),
        }
        TextColor(colors::TEXT)
        Node {
            margin: UiRect::top(px(10)),
        }
    }
}

fn row() -> Node {
    Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: px(8),
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
        BackgroundColor({Color::BLACK.with_alpha(0.45)})
        GlobalZIndex(9)
        SettingsMenu
        Children [
            Node {
                width: px(640),
                max_width: percent(94),
                max_height: percent(92),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(16)),
                border_radius: BorderRadius::all(px(6)),
                overflow: Overflow::scroll_y(),
            }
            BackgroundColor(colors::CARD)
            ScrollArea
            Children [
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                }
                Children [
                    Text("settings")
                    TextFont {
                        font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::BOLD),
                        font_size: FontSize::Px(22.0),
                    }
                    TextColor(colors::TEXT)
                    Node { flex_grow: 1.0 }
                    --
                    @crate::ui::plain_scene_button("forge".into(), ButtonVariant::Normal)
                    on(|_a: On<Activate>, mut forge: ResMut<crate::forge_ui::ForgeState>, mut s: ResMut<Settings>| {
                        s.toggle();
                        forge.toggle();
                    })
                    --
                    @crate::ui::plain_scene_button("keys".into(), ButtonVariant::Normal)
                    on(|_a: On<Activate>, mut remap: ResMut<crate::keys::Remap>| {
                        remap.open = true;
                        remap.changed = true;
                    })
                    --
                    @crate::ui::plain_scene_button("close".into(), ButtonVariant::Primary)
                    on(|_a: On<Activate>, mut s: ResMut<Settings>| s.toggle())
                ]
                --
                Text("")
                TextFont {
                    font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
                    font_size: FontSize::Px(13.0),
                }
                TextColor(colors::LINK)
                SettingsMessage
                --
                @heading("saves")
                --
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(4),
                }
                SavesList
                --
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                    margin: UiRect::top(px(4)),
                }
                Children [
                    @FeathersTextInputContainer
                    Node { flex_grow: 1.0 }
                    Children [
                        @FeathersTextInput
                        NewSaveInput
                        on(|c: On<TextEditChange>, inputs: Query<&EditableText>, mut s: ResMut<Settings>| {
                            if let Ok(t) = inputs.get(c.event_target()) {
                                s.new_name = t.value().to_string();
                            }
                        })
                        on(|k: On<FocusedInput<KeyboardInput>>, mut commands: Commands, mut ctx: Ctx, s: Res<Settings>, config: Res<Config>| {
                            if k.input.state.is_pressed() && k.input.logical_key == Key::Enter {
                                new_save(&mut commands, &mut ctx, &s, &config);
                            }
                        })
                    ]
                    --
                    @crate::ui::plain_scene_button("new save".into(), ButtonVariant::Primary)
                    on(|_a: On<Activate>, mut commands: Commands, mut ctx: Ctx, s: Res<Settings>, config: Res<Config>| {
                        new_save(&mut commands, &mut ctx, &s, &config);
                    })
                ]
                --
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                }
                Children [
                    Text("folder")
                    TextFont {
                        font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
                        font_size: FontSize::Px(13.0),
                    }
                    TextColor(colors::TEXT_DIM)
                    --
                    @FeathersTextInputContainer
                    Node { flex_grow: 1.0 }
                    Children [
                        @FeathersTextInput
                        FolderInput
                        on(|c: On<TextEditChange>, inputs: Query<&EditableText>, mut s: ResMut<Settings>| {
                            if let Ok(t) = inputs.get(c.event_target()) {
                                s.folder = t.value().to_string();
                            }
                        })
                        on(|k: On<FocusedInput<KeyboardInput>>, mut commands: Commands, mut ctx: Ctx, mut s: ResMut<Settings>, mut config: ResMut<Config>| {
                            if k.input.state.is_pressed() && k.input.logical_key == Key::Enter {
                                use_folder(&mut commands, &mut ctx, &mut s, &mut config);
                            }
                        })
                    ]
                    --
                    @crate::ui::plain_scene_button("use".into(), ButtonVariant::Normal)
                    on(|_a: On<Activate>, mut commands: Commands, mut ctx: Ctx, mut s: ResMut<Settings>, mut config: ResMut<Config>| {
                        use_folder(&mut commands, &mut ctx, &mut s, &mut config);
                    })
                ]
                --
                Text("a folder lists its saves; a path ending in .redb opens (or starts) that save")
                TextFont {
                    font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
                    font_size: FontSize::Px(12.0),
                }
                TextColor(colors::TEXT_DIM)
                --
                @heading("hub")
                --
                @crate::hub_ui::section()
                --
                @heading("fonts")
                --
                Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: px(8),
                    row_gap: px(4),
                }
                FontTabs
                --
                @FeathersTextInputContainer
                Children [
                    @FeathersTextInput
                    FontFilterInput
                    on(|c: On<TextEditChange>, inputs: Query<&EditableText>, mut s: ResMut<Settings>| {
                        if let Ok(t) = inputs.get(c.event_target()) {
                            s.font_filter = t.value().to_string();
                            s.changed = true;
                        }
                    })
                ]
                --
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    max_height: px(190),
                    flex_shrink: 0.0,
                    overflow: Overflow::scroll_y(),
                }
                ScrollArea
                FontList
                --
                @heading("colours")
                --
                Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                    row_gap: px(4),
                }
                ColorPresets
                --
                Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: px(4),
                    row_gap: px(4),
                }
                ColorRoles
                --
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(6),
                    padding: UiRect::axes(px(2), px(4)),
                }
                Children [
                    @color_slider(ColorChannel::HslHue, "hue")
                    --
                    @color_slider(ColorChannel::HslSaturation, "saturation")
                    --
                    @color_slider(ColorChannel::HslLightness, "lightness")
                    --
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(8),
                    }
                    Children [
                        Text("hex")
                        TextFont {
                            font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
                            font_size: FontSize::Px(13.0),
                        }
                        TextColor(colors::TEXT_DIM)
                        Node { width: px(70) }
                        --
                        @FeathersTextInputContainer
                        Node { width: px(110) }
                        Children [
                            @FeathersTextInput
                            HexInput
                            on(|k: On<FocusedInput<KeyboardInput>>, inputs: Query<&EditableText>, mut s: ResMut<Settings>, mut config: ResMut<Config>| {
                                if k.input.state.is_pressed() && k.input.logical_key == Key::Enter
                                    && let Ok(t) = inputs.get(k.event_target())
                                {
                                    match palette::from_hex(t.value().to_string().as_str()) {
                                        Some(c) => set_role_color(&mut s, &mut config, c),
                                        None => {
                                            s.message = "a colour is #rrggbb (or #rgb)".into();
                                            s.changed = true;
                                        }
                                    }
                                }
                            })
                        ]
                        --
                        @crate::ui::plain_scene_button("reset".into(), ButtonVariant::Normal)
                        on(|_a: On<Activate>, mut s: ResMut<Settings>, mut config: ResMut<Config>| {
                            let role = Role::ALL[s.color_role];
                            config.colors.remove(role.name());
                            config.save();
                            s.hsl = role.default_color().into();
                            s.sync_colors = true;
                            s.changed = true;
                        })
                    ]
                ]
                --
                @heading("sizes")
                --
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(4),
                }
                SizeRows
                --
                @heading("view")
                --
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(6),
                }
                Toggles
            ]
        ]
    }
}

// ------------------------------------------------------------------ saves

fn scan(config: &Config, db: &Store) -> Vec<SaveInfo> {
    let current = db.path().to_path_buf();
    let mut paths: Vec<PathBuf> = crate::vfs::list(&config.saves_dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "redb"))
        .collect();
    if !paths.iter().any(|p| same(p, &current)) {
        paths.push(current.clone());
    }
    let mut saves: Vec<SaveInfo> = paths
        .into_iter()
        .map(|path| {
            let meta = crate::vfs::meta(&path);
            let is_current = same(&path, &current);
            SaveInfo {
                counts: if is_current {
                    Some(db.counts())
                } else {
                    Store::summary(&path)
                },
                modified: meta.as_ref().and_then(|m| m.modified),
                size: meta.map(|m| m.len).unwrap_or(0),
                current: is_current,
                path,
            }
        })
        .collect();
    // the open one first, then the most recently played
    saves.sort_by(|a, b| b.current.cmp(&a.current).then(b.modified.cmp(&a.modified)));
    saves
}

fn same(a: &Path, b: &Path) -> bool {
    crate::vfs::same(a, b)
}

pub(crate) fn human_size(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1u64 << 20) as f64)
    } else {
        format!("{} KB", n.div_ceil(1024))
    }
}

fn say(world: &mut World, msg: impl Into<String>) {
    let msg = msg.into();
    let now = world.resource::<Time>().elapsed_secs_f64();
    let mut status = world.resource_mut::<Status>();
    status.text = msg.clone();
    status.until = now + 4.0;
    let mut s = world.resource_mut::<Settings>();
    s.message = msg;
    s.changed = true;
    s.rescan = true;
}

/// saves the open note if it can be, before the database changes under it
pub(crate) fn keep_note(ctx: &mut Ctx) {
    if ctx.is_dirty() && !ctx.session.active.title.trim().is_empty() {
        ctx.save();
    }
}

/// closes the open save and opens another; everything that came from the old one goes
/// reads the open save again from scratch (after something was poured into it)
pub(crate) fn reload_open(world: &mut World) {
    if let Some(Db(store)) = world.remove_resource::<Db>() {
        install(world, store);
    }
}

pub fn switch_save(world: &mut World, path: PathBuf) {
    if same(world.resource::<Db>().0.path(), &path) {
        return;
    }
    let store = match Store::open(&path) {
        Ok(s) => s,
        Err(e) => return say(world, format!("couldn't open {}: {}", path.display(), e)),
    };
    install(world, store);
    say(world, format!("loaded {}", save_name(&path)));
}

/// puts a store in place of the open one and resets what was read from the old one
fn install(world: &mut World, store: Store) {
    let path = store.path().to_path_buf();
    let notes = store.all().unwrap_or_default();
    world.insert_resource(Db(store));
    {
        let mut session = world.resource_mut::<Session>();
        session.active = notes.first().cloned().unwrap_or_default();
        session.view_mode = false;
        session.reader_tag = None;
        session.note_filter = None;
    }
    world.insert_resource(Notes(notes));
    world.insert_resource(Redraw::default());
    world.insert_resource(MemoryState::default());
    let texts: Vec<Entity> = world.query_filtered::<Entity, With<SpaceText>>().iter(world).collect();
    for e in texts {
        world.despawn(e);
    }
    world.insert_resource(Space::default());
    world.resource_mut::<crate::bar::BarState>().clear();
    let mut config = world.resource_mut::<Config>();
    config.current = Some(path);
    config.save();
}

fn new_save(commands: &mut Commands, ctx: &mut Ctx, s: &Settings, config: &Config) {
    let Some(name) = clean_name(&s.new_name) else {
        ctx.say("give the new save a name");
        return;
    };
    let path = config.saves_dir.join(format!("{}.redb", name));
    if path.exists() {
        ctx.say(format!("there's already a save called {}", name));
        return;
    }
    keep_note(ctx);
    commands.queue(move |w: &mut World| switch_save(w, path));
}

fn use_folder(commands: &mut Commands, ctx: &mut Ctx, s: &mut Settings, config: &mut Config) {
    let raw = s.folder.trim();
    if raw.is_empty() {
        return;
    }
    let path = PathBuf::from(shellexpand(raw));
    if path.extension().is_some_and(|x| x == "redb") {
        keep_note(ctx);
        commands.queue(move |w: &mut World| switch_save(w, path));
        return;
    }
    if let Err(e) = crate::vfs::create_dir_all(&path) {
        ctx.say(format!("can't use {}: {}", path.display(), e));
        return;
    }
    config.saves_dir = path;
    config.save();
    s.rescan = true;
    s.changed = true;
}

/// `~/` at the start of a path is the home folder
fn shellexpand(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{}/{}", home, rest),
        _ => p.to_string(),
    }
}

/// `name.redb` in `dir`, or `name 2.redb` and on if that's taken
pub(crate) fn unique(dir: &Path, name: &str) -> PathBuf {
    let mut p = dir.join(format!("{}.redb", name));
    let mut i = 2;
    while crate::vfs::meta(&p).is_some() {
        p = dir.join(format!("{} {}.redb", name, i));
        i += 1;
    }
    p
}

fn duplicate(world: &mut World, path: PathBuf) {
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let to = unique(&dir, &format!("{} copy", save_name(&path)));
    match crate::vfs::copy(&path, &to) {
        Ok(_) => say(world, format!("copied {} to {}", save_name(&path), save_name(&to))),
        Err(e) => say(world, format!("couldn't copy {}: {}", save_name(&path), e)),
    }
}

/// renames a save; the open one is closed for the move and opened again
fn rename(world: &mut World, path: PathBuf, name: String) {
    let Some(name) = clean_name(&name) else {
        return say(world, "a save needs a name");
    };
    let to = path.with_file_name(format!("{}.redb", name));
    if crate::vfs::meta(&to).is_some() {
        return say(world, format!("there's already a save called {}", name));
    }
    let current = same(world.resource::<Db>().0.path(), &path);
    if current {
        // redb holds the file while it's open
        let db = world.remove_resource::<Db>();
        drop(db);
    }
    let moved = crate::vfs::rename(&path, &to);
    if current {
        let path = if moved.is_ok() { &to } else { &path };
        if let Err(e) = reopen(world, path) {
            return say(world, e);
        }
    }
    world.resource_mut::<Settings>().renaming = None;
    match moved {
        Ok(()) => say(world, format!("renamed to {}", name)),
        Err(e) => say(world, format!("couldn't rename: {}", e)),
    }
}

/// opens a closed save again; with nothing to fall back on, starts an empty save so the
/// app keeps working
fn reopen(world: &mut World, path: &Path) -> Result<(), String> {
    match Store::open(path) {
        Ok(store) => {
            world.insert_resource(Db(store));
            let mut config = world.resource_mut::<Config>();
            config.current = Some(path.to_path_buf());
            config.save();
            Ok(())
        }
        Err(e) => {
            let fresh = unique(&world.resource::<Config>().saves_dir.clone(), "recovered");
            if let Ok(store) = Store::open(&fresh) {
                install(world, store);
            }
            Err(format!("couldn't reopen the save: {}", e))
        }
    }
}

/// the open save's name and bytes; it's closed for the read, since redb holds the file
/// (and marks it unfinished) while it's open, then opened again
pub(crate) fn read_open_save(world: &mut World, max: u64) -> Result<(String, Vec<u8>), String> {
    let path = world.resource::<Db>().0.path().to_path_buf();
    if crate::vfs::meta(&path).is_some_and(|m| m.len > max) {
        return Err(format!("this save is over {}", human_size(max)));
    }
    drop(world.remove_resource::<Db>());
    let bytes = crate::vfs::read(&path).map_err(|e| format!("couldn't read the save: {}", e));
    reopen(world, &path)?;
    bytes.map(|b| (save_name(&path), b))
}

fn delete(world: &mut World, path: PathBuf) {
    if same(world.resource::<Db>().0.path(), &path) {
        return say(world, "load another save before deleting this one");
    }
    world.resource_mut::<Settings>().confirm_delete = None;
    match crate::vfs::remove(&path) {
        Ok(()) => say(world, format!("deleted {}", save_name(&path))),
        Err(e) => say(world, format!("couldn't delete {}: {}", save_name(&path), e)),
    }
}

// ------------------------------------------------------------------ building the menu

fn small(commands: &mut Commands, parent: Entity, s: &str, fonts: &Fonts, color: Color) -> Entity {
    text(commands, parent, s, &fonts.regular, 12.0, color)
}

fn button(
    commands: &mut Commands,
    parent: Entity,
    label: &str,
    variant: ButtonVariant,
    f: impl FnOnce(&mut EntityCommands),
) {
    let mut e = commands.spawn_scene(plain_scene_button(label.to_string(), variant));
    e.insert(ChildOf(parent));
    f(&mut e);
}

/// a toggle: a little box and its label
fn toggle(commands: &mut Commands, parent: Entity, label: &str, on: bool, fonts: &Fonts) -> Entity {
    let r = commands
        .spawn((row(), EntityCursor::System(SystemCursorIcon::Pointer), ChildOf(parent)))
        .id();
    commands.spawn((
        Node {
            width: px(14),
            height: px(14),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(3)),
            ..default()
        },
        BackgroundColor(if on { colors::ACCENT } else { colors::CARD }),
        BorderColor::all(colors::MOOD_DARK),
        Pickable::IGNORE,
        ChildOf(r),
    ));
    let t = text(commands, r, label, &fonts.regular, 14.0, colors::TEXT);
    commands.entity(t).insert(Pickable::IGNORE);
    r
}

pub fn rebuild_settings(
    mut s: ResMut<Settings>,
    config: Res<Config>,
    db: Res<Db>,
    keys: Res<ButtonInput<KeyCode>>,
    remap: Res<crate::keys::Remap>,
    fonts: Res<Fonts>,
    mut menu: Single<&mut Node, With<SettingsMenu>>,
    lists: (
        Single<Entity, With<SavesList>>,
        Single<Entity, With<FontTabs>>,
        Single<Entity, With<FontList>>,
        Single<Entity, With<Toggles>>,
        Single<Entity, With<ColorPresets>>,
        Single<Entity, With<ColorRoles>>,
        Single<Entity, With<SizeRows>>,
    ),
    mut message: Single<&mut Text, With<SettingsMessage>>,
    mut folder: Single<&mut EditableText, With<FolderInput>>,
    mut commands: Commands,
) {
    if s.open && !remap.open && keys.just_pressed(KeyCode::Escape) {
        s.toggle();
    }
    if !s.changed {
        return;
    }
    s.changed = false;
    menu.display = if s.open { Display::Flex } else { Display::None };
    if !s.open {
        return;
    }
    let (saves_list, tabs, font_list, toggles) = (*lists.0, *lists.1, *lists.2, *lists.3);
    let (presets, roles, sizes) = (*lists.4, *lists.5, *lists.6);
    if message.0 != s.message {
        message.0 = s.message.clone();
    }
    if std::mem::take(&mut s.fill_folder) {
        let f = config.saves_dir.display().to_string();
        folder.editor_mut().set_text(&f);
        folder.queue_edit(TextEdit::TextEnd(false));
        s.folder = f;
    }
    if std::mem::take(&mut s.rescan) {
        s.saves = scan(&config, &db.0);
    }
    if s.families.is_none() {
        s.families = Some(crate::typeface::system_families());
    }

    // the saves
    commands.entity(saves_list).despawn_related::<Children>();
    let mut rename_to = None;
    for save in &s.saves {
        let card = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                    padding: UiRect::axes(px(10), px(6)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(if save.current { colors::PAD } else { colors::CODE_BG }),
                ChildOf(saves_list),
            ))
            .id();
        let info = commands
            .spawn((
                Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                ChildOf(card),
            ))
            .id();
        let path = save.path.clone();
        if s.renaming.as_ref() == Some(&save.path) {
            let input_row = commands.spawn((row(), ChildOf(info))).id();
            let container = commands
                .spawn_scene(bsn! { @FeathersTextInputContainer Node { flex_grow: 1.0 } })
                .insert(ChildOf(input_row))
                .id();
            let name = save_name(&save.path);
            let input = commands
                .spawn_scene(bsn! { @FeathersTextInput })
                .insert((ChildOf(container), RenameInput, EditableText::new(&name)))
                .id();
            commands.queue(move |w: &mut World| {
                w.resource_mut::<bevy::input_focus::InputFocus>()
                    .set(input, bevy::input_focus::FocusCause::Navigated);
            });
            commands
                .entity(input)
                .observe(
                    |c: On<TextEditChange>, inputs: Query<&EditableText>, mut s: ResMut<Settings>| {
                        if let Ok(t) = inputs.get(c.event_target()) {
                            s.rename_to = t.value().to_string();
                        }
                    },
                )
                .observe({
                    let path = path.clone();
                    move |k: On<FocusedInput<KeyboardInput>>, s: Res<Settings>, mut commands: Commands| {
                        if k.input.state.is_pressed() && k.input.logical_key == Key::Enter {
                            let (p, n) = (path.clone(), s.rename_to.clone());
                            commands.queue(move |w: &mut World| rename(w, p, n));
                        }
                    }
                });
            rename_to = Some(name);
        } else {
            let title = format!(
                "{}{}",
                save_name(&save.path),
                if save.current { "  · open" } else { "" }
            );
            text(&mut commands, info, &title, &fonts.bold, 15.0, colors::TEXT);
            let counts = match save.counts {
                Some((n, m)) => format!(
                    "{} note{} · {} memor{}",
                    n,
                    if n == 1 { "" } else { "s" },
                    m,
                    if m == 1 { "y" } else { "ies" }
                ),
                None => "can't read it".into(),
            };
            let when = save
                .modified
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default();
            small(
                &mut commands,
                info,
                &format!("{} · {} · {}", counts, when, human_size(save.size)),
                &fonts,
                colors::TEXT_DIM,
            );
        }
        if s.confirm_delete.as_ref() == Some(&save.path) {
            small(&mut commands, card, "delete for good?", &fonts, colors::LINK);
            let p = path.clone();
            button(&mut commands, card, "delete", ButtonVariant::Primary, |e| {
                e.observe(move |_a: On<Activate>, mut commands: Commands| {
                    let p = p.clone();
                    commands.queue(move |w: &mut World| delete(w, p));
                });
            });
            button(&mut commands, card, "keep", ButtonVariant::Normal, |e| {
                e.observe(|_a: On<Activate>, mut s: ResMut<Settings>| {
                    s.confirm_delete = None;
                    s.changed = true;
                });
            });
            continue;
        }
        if s.renaming.as_ref() == Some(&save.path) {
            let p = path.clone();
            button(&mut commands, card, "rename", ButtonVariant::Primary, |e| {
                e.observe(move |_a: On<Activate>, s: Res<Settings>, mut commands: Commands| {
                    let (p, n) = (p.clone(), s.rename_to.clone());
                    commands.queue(move |w: &mut World| rename(w, p, n));
                });
            });
            button(&mut commands, card, "cancel", ButtonVariant::Normal, |e| {
                e.observe(|_a: On<Activate>, mut s: ResMut<Settings>| {
                    s.renaming = None;
                    s.changed = true;
                });
            });
            continue;
        }
        if !save.current {
            let p = path.clone();
            button(&mut commands, card, "load", ButtonVariant::Primary, |e| {
                e.observe(move |_a: On<Activate>, mut ctx: Ctx, mut commands: Commands| {
                    keep_note(&mut ctx);
                    let p = p.clone();
                    commands.queue(move |w: &mut World| switch_save(w, p));
                });
            });
        }
        let p = path.clone();
        button(&mut commands, card, "copy", ButtonVariant::Normal, |e| {
            e.observe(move |_a: On<Activate>, mut commands: Commands| {
                let p = p.clone();
                commands.queue(move |w: &mut World| duplicate(w, p));
            });
        });
        let p = path.clone();
        button(&mut commands, card, "rename", ButtonVariant::Normal, |e| {
            e.observe(move |_a: On<Activate>, mut s: ResMut<Settings>| {
                s.renaming = Some(p.clone());
                s.confirm_delete = None;
                s.changed = true;
            });
        });
        if !save.current {
            let p = path.clone();
            button(&mut commands, card, "delete", ButtonVariant::Normal, |e| {
                e.observe(move |_a: On<Activate>, mut s: ResMut<Settings>| {
                    s.confirm_delete = Some(p.clone());
                    s.renaming = None;
                    s.changed = true;
                });
            });
        }
    }

    if let Some(n) = rename_to {
        s.rename_to = n;
    }

    // fonts: which one the list sets, then the families
    commands.entity(tabs).despawn_related::<Children>();
    let target = s.font_target;
    for (t, label, value, builtin) in [
        (FontTarget::Interface, "interface", &config.font, "Fira Sans"),
        (FontTarget::Memory, "memory view", &config.memory_font, "system serif"),
    ] {
        let caption = format!("{}: {}", label, value.as_deref().unwrap_or(builtin));
        let variant = if t == target {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Normal
        };
        button(&mut commands, tabs, &caption, variant, |e| {
            e.observe(move |_a: On<Activate>, mut s: ResMut<Settings>| {
                s.font_target = t;
                s.changed = true;
            });
        });
    }
    commands.entity(font_list).despawn_related::<Children>();
    let chosen = match target {
        FontTarget::Interface => config.font.clone(),
        FontTarget::Memory => config.memory_font.clone(),
    };
    let filter = s.font_filter.trim().to_lowercase();
    let builtin = match target {
        FontTarget::Interface => "built in (Fira Sans)",
        FontTarget::Memory => "system serif (Times New Roman)",
    };
    let mut items: Vec<(Option<String>, String)> = vec![(None, builtin.to_string())];
    items.extend(
        s.families
            .iter()
            .flatten()
            .filter(|f| filter.is_empty() || f.to_lowercase().contains(&filter))
            .take(80)
            .map(|f| (Some(f.clone()), f.clone())),
    );
    for (value, label) in items {
        let selected = value == chosen;
        let e = commands
            .spawn((
                Node {
                    padding: UiRect::axes(px(8), px(3)),
                    border_radius: BorderRadius::all(px(3)),
                    ..default()
                },
                BackgroundColor(if selected { colors::PAD } else { Color::NONE }),
                EntityCursor::System(SystemCursorIcon::Pointer),
                ChildOf(font_list),
            ))
            .observe(
                move |_c: On<PointerClick>, mut config: ResMut<Config>, mut s: ResMut<Settings>| {
                    match s.font_target {
                        FontTarget::Interface => config.font = value.clone(),
                        FontTarget::Memory => config.memory_font = value.clone(),
                    }
                    config.save();
                    s.changed = true;
                },
            )
            .id();
        let t = text(&mut commands, e, &label, &fonts.regular, 14.0, colors::TEXT);
        commands.entity(t).insert(Pickable::IGNORE);
    }

    // the colours
    commands.entity(presets).despawn_related::<Children>();
    small(&mut commands, presets, "start from", &fonts, colors::TEXT_DIM);
    for (i, (name, _)) in palette::PRESETS.iter().enumerate() {
        let current = config.colors == palette::preset_map(i);
        let variant = if current {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Normal
        };
        button(&mut commands, presets, name, variant, |e| {
            e.observe(
                move |_a: On<Activate>, mut config: ResMut<Config>, mut s: ResMut<Settings>| {
                    config.colors = palette::preset_map(i);
                    config.save();
                    s.sync_colors = true;
                    s.changed = true;
                },
            );
        });
    }
    commands.entity(roles).despawn_related::<Children>();
    let pal = Palette::from_config(&config.colors);
    for (i, role) in Role::ALL.into_iter().enumerate() {
        let chosen = i == s.color_role;
        let chip = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(5),
                    padding: UiRect::axes(px(6), px(3)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(3)),
                    ..default()
                },
                BackgroundColor(if chosen { colors::PAD } else { Color::NONE }),
                BorderColor::all(if chosen { colors::MOOD_DARK } else { Color::NONE }),
                EntityCursor::System(SystemCursorIcon::Pointer),
                ChildOf(roles),
            ))
            .observe(move |_c: On<PointerClick>, mut s: ResMut<Settings>| {
                s.color_role = i;
                s.sync_colors = true;
                s.changed = true;
            })
            .id();
        commands.spawn((
            Node {
                width: px(14),
                height: px(14),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(3)),
                ..default()
            },
            // painted after the palette tags things, so a swatch keeps the colour it shows
            Swatch(pal.get(role)),
            BorderColor::all(Color::BLACK.with_alpha(0.3)),
            Pickable::IGNORE,
            ChildOf(chip),
        ));
        let t = text(&mut commands, chip, role.name(), &fonts.regular, 13.0, colors::TEXT);
        commands.entity(t).insert(Pickable::IGNORE);
    }

    // the sizes
    commands.entity(sizes).despawn_related::<Children>();
    type Get = fn(&Config) -> f32;
    type Set = fn(&mut Config, f32);
    let size_rows: [(&str, Get, Set, f32, f32, f32, bool); 3] = [
        ("interface", |c| c.ui_scale, |c, v| c.ui_scale = v, 0.1, 0.6, 2.0, true),
        ("top bar", |c| c.top_bar, |c, v| c.top_bar = v, 2.0, 18.0, 96.0, false),
        (
            "command bar",
            |c| c.bottom_bar,
            |c, v| c.bottom_bar = v,
            2.0,
            18.0,
            96.0,
            false,
        ),
    ];
    for (label, get, set, step, lo, hi, percent) in size_rows {
        let r = commands.spawn((row(), ChildOf(sizes))).id();
        let l = text(&mut commands, r, label, &fonts.regular, 14.0, colors::TEXT);
        commands.entity(l).insert(Node {
            width: px(110),
            ..default()
        });
        let v = get(&config);
        for (sign, glyph) in [(-1.0f32, "−"), (1.0, "+")] {
            if sign > 0.0 {
                let shown = if percent {
                    format!("{:.0}%", v * 100.0)
                } else {
                    format!("{:.0} px", v)
                };
                let t = text(&mut commands, r, &shown, &fonts.regular, 14.0, colors::TEXT);
                commands.entity(t).insert(Node {
                    width: px(56),
                    justify_content: JustifyContent::Center,
                    ..default()
                });
            }
            button(&mut commands, r, glyph, ButtonVariant::Normal, |e| {
                e.observe(
                    move |_a: On<Activate>, mut config: ResMut<Config>, mut s: ResMut<Settings>| {
                        let v = ((get(&config) + sign * step) / step).round() * step;
                        set(&mut config, v.clamp(lo, hi));
                        config.save();
                        s.changed = true;
                    },
                );
            });
        }
    }
    let r = commands.spawn((row(), ChildOf(sizes))).id();
    small(
        &mut commands,
        r,
        "a bar's text and buttons are sized to fit its height",
        &fonts,
        colors::TEXT_DIM,
    );
    button(&mut commands, r, "default sizes", ButtonVariant::Normal, |e| {
        e.observe(
            |_a: On<Activate>, mut config: ResMut<Config>, mut s: ResMut<Settings>| {
                let d = Config::default();
                config.ui_scale = d.ui_scale;
                config.top_bar = d.top_bar;
                config.bottom_bar = d.bottom_bar;
                config.save();
                s.changed = true;
            },
        );
    });

    // the view toggles
    commands.entity(toggles).despawn_related::<Children>();
    let zen = toggle(
        &mut commands,
        toggles,
        "zen: fade the bars and panels away while the mouse rests",
        config.zen,
        &fonts,
    );
    commands.entity(zen).observe(
        |_c: On<PointerClick>, mut config: ResMut<Config>, mut s: ResMut<Settings>| {
            config.zen = !config.zen;
            config.save();
            s.changed = true;
        },
    );
    if config.zen {
        let delays = commands
            .spawn((
                Node {
                    margin: UiRect::left(px(22)),
                    ..row()
                },
                ChildOf(toggles),
            ))
            .id();
        small(&mut commands, delays, "after", &fonts, colors::TEXT_DIM);
        for d in [1.0f32, 2.5, 5.0, 10.0] {
            let variant = if (config.zen_delay - d).abs() < 0.01 {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Normal
            };
            button(&mut commands, delays, &format!("{}s", d), variant, |e| {
                e.observe(
                    move |_a: On<Activate>, mut config: ResMut<Config>, mut s: ResMut<Settings>| {
                        config.zen_delay = d;
                        config.save();
                        s.changed = true;
                    },
                );
            });
        }
    }
    let bar = toggle(
        &mut commands,
        toggles,
        "hide the top bar until the pointer comes near the top",
        config.hide_bar,
        &fonts,
    );
    commands.entity(bar).observe(
        |_c: On<PointerClick>, mut config: ResMut<Config>, mut s: ResMut<Settings>| {
            config.hide_bar = !config.hide_bar;
            config.save();
            s.changed = true;
        },
    );
    let list = toggle(
        &mut commands,
        toggles,
        "slide the note list away until the pointer comes to the left edge",
        config.hide_list,
        &fonts,
    );
    commands.entity(list).observe(
        |_c: On<PointerClick>, mut config: ResMut<Config>, mut s: ResMut<Settings>| {
            config.hide_list = !config.hide_list;
            config.save();
            s.changed = true;
        },
    );
    let swapped = toggle(
        &mut commands,
        toggles,
        "the top bar at the bottom, and the command bar at the top",
        config.bars_swapped,
        &fonts,
    );
    commands.entity(swapped).observe(
        |_c: On<PointerClick>, mut config: ResMut<Config>, mut s: ResMut<Settings>| {
            config.bars_swapped = !config.bars_swapped;
            config.save();
            s.changed = true;
        },
    );
    let right = toggle(
        &mut commands,
        toggles,
        "the note list on the right",
        config.list_right,
        &fonts,
    );
    commands.entity(right).observe(
        |_c: On<PointerClick>, mut config: ResMut<Config>, mut s: ResMut<Settings>| {
            config.list_right = !config.list_right;
            config.save();
            s.changed = true;
        },
    );
    let sums = toggle(
        &mut commands,
        toggles,
        "in the space, show the search line (the sums and their chain) in the top bar",
        config.space_sums,
        &fonts,
    );
    commands.entity(sums).observe(
        |_c: On<PointerClick>, mut config: ResMut<Config>, mut s: ResMut<Settings>, mut space: ResMut<Space>| {
            config.space_sums = !config.space_sums;
            config.save();
            space.redraw.qline = true;
            s.changed = true;
        },
    );
}
