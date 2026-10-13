//! The settings menu's hub section: connect to an anend hub with an app token, browse
//! and download the saves people share there, and share the open one. The calls run on
//! threads of their own (see hub.rs) and their answers come back through a channel that
//! `poll_hub` reads each frame, so the app never waits on the network.

use bevy::feathers::controls::{ButtonVariant, FeathersTextInput, FeathersTextInputContainer};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::FocusedInput;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEdit, TextEditChange};
use bevy::ui_widgets::{Activate, ScrollArea};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::config::{Config, save_name};
use crate::hub::{self, HubInfo, HubSave, Me, SavesPage, Sort, Uploaded};
use crate::settings::{Settings, human_size, keep_note};
use crate::theme::colors;
use crate::ui::{Ctx, Db, Fonts, Status, plain_scene_button, text};

/// what came back from a call
enum Reply {
    Connected(Result<(HubInfo, Result<Me, String>), String>),
    Saves {
        generation: u64,
        page: u32,
        result: Result<SavesPage, String>,
    },
    Downloaded(Result<PathBuf, String>),
    Uploaded(Result<Uploaded, String>),
}

#[derive(Resource)]
pub struct HubState {
    /// rebuild the section's lists
    pub changed: bool,
    was_open: bool,
    url: String,
    token: String,
    query: String,
    sort: Sort,
    page: u32,
    saves: Vec<HubSave>,
    more: bool,
    browsed: bool,
    /// a newer search makes older answers stale
    generation: u64,
    connection: String,
    browse_note: String,
    share_note: String,
    share_name: String,
    share_description: String,
    share_tags: String,
    connecting: bool,
    browsing: bool,
    downloading: Option<String>,
    uploading: bool,
    tx: Sender<Reply>,
    rx: Mutex<Receiver<Reply>>,
}

impl Default for HubState {
    fn default() -> Self {
        let (tx, rx) = channel();
        HubState {
            changed: false,
            was_open: false,
            url: String::new(),
            token: String::new(),
            query: String::new(),
            sort: Sort::New,
            page: 0,
            saves: Vec::new(),
            more: false,
            browsed: false,
            generation: 0,
            connection: String::new(),
            browse_note: String::new(),
            share_note: String::new(),
            share_name: String::new(),
            share_description: String::new(),
            share_tags: String::new(),
            connecting: false,
            browsing: false,
            downloading: None,
            uploading: false,
            tx,
            rx: Mutex::new(rx),
        }
    }
}

impl HubState {
    /// runs a call on its own thread; its answer turns up in `poll_hub`
    fn run(&self, f: impl FnOnce() -> Reply + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
    }

    fn connect(&mut self, config: &mut Config) {
        if self.connecting {
            return;
        }
        let url = self.url.trim().to_string();
        if let Err(e) = hub::base(&url) {
            self.connection = e;
            self.changed = true;
            return;
        }
        config.hub_url = url.clone();
        config.hub_token = self.token.trim().to_string();
        config.save();
        self.connecting = true;
        self.changed = true;
        let token = config.hub_token.clone();
        self.run(move || {
            Reply::Connected(hub::hub_info(&url).map(|info| {
                let me = if token.is_empty() {
                    Err("add an app token to share saves".into())
                } else {
                    hub::me(&url, &token)
                };
                (info, me)
            }))
        });
        self.browse(0);
    }

    /// fetches a page of saves; page 0 starts the list again
    fn browse(&mut self, page: u32) {
        let url = self.url.trim().to_string();
        if let Err(e) = hub::base(&url) {
            self.browse_note = e;
            self.changed = true;
            return;
        }
        self.generation += 1;
        self.browsing = true;
        self.changed = true;
        let (generation, q, sort) = (self.generation, self.query.clone(), self.sort);
        self.run(move || Reply::Saves {
            generation,
            page,
            result: hub::saves(&url, &q, sort, page),
        });
    }

    fn download(&mut self, save: HubSave, dir: PathBuf) {
        if self.downloading.is_some() {
            return;
        }
        let url = self.url.trim().to_string();
        self.downloading = Some(save.id.clone());
        self.browse_note.clear();
        self.changed = true;
        self.run(move || Reply::Downloaded(hub::download(&url, &save, &dir)));
    }
}

/// which of the section's text boxes this is
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Field {
    #[default]
    Url,
    Token,
    Search,
    Name,
    Description,
    Tags,
}

#[derive(Component, Default, Clone)]
pub struct HubField(pub Field);
/// a line saying how a part of the section is doing
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Line {
    #[default]
    Connection,
    Browse,
    Share,
}

#[derive(Component, Default, Clone)]
pub struct HubLine(pub Line);
#[derive(Component, Default, Clone)]
pub struct HubSorts;
#[derive(Component, Default, Clone)]
pub struct HubList;
#[derive(Component, Default, Clone)]
pub struct HubMore;

fn edit_field(c: On<TextEditChange>, inputs: Query<(&HubField, &EditableText)>, mut hub: ResMut<HubState>) {
    let Ok((field, t)) = inputs.get(c.event_target()) else {
        return;
    };
    let v = t.value().to_string();
    match field.0 {
        Field::Url => hub.url = v,
        Field::Token => hub.token = v,
        Field::Search => hub.query = v,
        Field::Name => hub.share_name = v,
        Field::Description => hub.share_description = v,
        Field::Tags => hub.share_tags = v,
    }
}

fn enter_field(
    k: On<FocusedInput<KeyboardInput>>,
    inputs: Query<&HubField>,
    mut hub: ResMut<HubState>,
    mut config: ResMut<Config>,
) {
    if !k.input.state.is_pressed() || k.input.logical_key != Key::Enter {
        return;
    }
    match inputs.get(k.event_target()).map(|f| f.0) {
        Ok(Field::Url | Field::Token) => hub.connect(&mut config),
        Ok(Field::Search) => hub.browse(0),
        _ => {}
    }
}

fn label(s: &'static str) -> impl Scene {
    bsn! {
        Text(s)
        TextFont {
            font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
            font_size: FontSize::Px(13.0),
        }
        TextColor(colors::TEXT_DIM)
        Node { width: px(80), flex_shrink: 0.0 }
    }
}

fn note() -> impl Scene {
    bsn! {
        Text("")
        TextFont {
            font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
            font_size: FontSize::Px(13.0),
        }
        TextColor(colors::LINK)
    }
}

fn subheading(s: &'static str) -> impl Scene {
    bsn! {
        Text(s)
        TextFont {
            font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::BOLD),
            font_size: FontSize::Px(13.0),
        }
        TextColor(colors::TEXT_DIM)
        Node { margin: UiRect::top(px(6)) }
    }
}

/// a labelled text box
fn input(caption: &'static str, field: Field) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(8),
        }
        Children [
            @label(caption)
            --
            @FeathersTextInputContainer
            Node { flex_grow: 1.0 }
            Children [
                @FeathersTextInput
                HubField(field)
                on(edit_field)
                on(enter_field)
            ]
        ]
    }
}

/// the section's fixed parts; `rebuild_hub` fills the rest
pub fn section() -> impl Scene {
    bsn! {
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
        }
        Children [
            @input("address", Field::Url)
            --
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(8),
            }
            Children [
                @label("app token")
                --
                @FeathersTextInputContainer
                Node { flex_grow: 1.0 }
                Children [
                    @FeathersTextInput
                    HubField(Field::Token)
                    on(edit_field)
                    on(enter_field)
                ]
                --
                @plain_scene_button("connect".into(), ButtonVariant::Primary)
                on(|_a: On<Activate>, mut hub: ResMut<HubState>, mut config: ResMut<Config>| {
                    hub.connect(&mut config);
                })
            ]
            --
            @note()
            HubLine(Line::Connection)
            --
            @subheading("browse")
            --
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(8),
            }
            Children [
                @FeathersTextInputContainer
                Node { flex_grow: 1.0 }
                Children [
                    @FeathersTextInput
                    HubField(Field::Search)
                    on(edit_field)
                    on(enter_field)
                ]
                --
                @plain_scene_button("search".into(), ButtonVariant::Normal)
                on(|_a: On<Activate>, mut hub: ResMut<HubState>| hub.browse(0))
            ]
            --
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Center,
                column_gap: px(6),
                row_gap: px(4),
            }
            HubSorts
            --
            @note()
            HubLine(Line::Browse)
            --
            // the saves scroll in a box of their own, so the menu below stays near
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
                max_height: px(400),
                flex_shrink: 0.0,
                overflow: Overflow::scroll_y(),
            }
            ScrollArea
            Children [
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: px(8),
                    row_gap: px(8),
                }
                HubList
                --
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                }
                HubMore
            ]
            --
            @subheading("share this save")
            --
            @input("name", Field::Name)
            --
            @input("description", Field::Description)
            --
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(8),
            }
            Children [
                @label("tags")
                --
                @FeathersTextInputContainer
                Node { flex_grow: 1.0 }
                Children [
                    @FeathersTextInput
                    HubField(Field::Tags)
                    on(edit_field)
                ]
                --
                @plain_scene_button("upload".into(), ButtonVariant::Primary)
                on(upload)
            ]
            --
            @note()
            HubLine(Line::Share)
            --
            Text("tags are comma separated; a hub's admin looks a save over before others see it")
            TextFont {
                font: bevy::text::FontSourceTemplate::Handle(bevy::feathers::constants::fonts::REGULAR),
                font_size: FontSize::Px(12.0),
            }
            TextColor(colors::TEXT_DIM)
        ]
    }
}

/// shares the open save: the open note is saved first, then the save is read and sent
fn upload(_a: On<Activate>, mut ctx: Ctx, mut hub: ResMut<HubState>, config: Res<Config>, mut commands: Commands) {
    if hub.uploading {
        return;
    }
    let (url, token) = (config.hub_url.clone(), config.hub_token.clone());
    if url.trim().is_empty() || token.trim().is_empty() {
        hub.share_note = "connect with an address and an app token first".into();
        hub.changed = true;
        return;
    }
    keep_note(&mut ctx);
    hub.uploading = true;
    hub.share_note.clear();
    hub.changed = true;
    commands.queue(move |w: &mut World| {
        let read = crate::settings::read_open_save(w, hub::MAX_DOWNLOAD);
        let mut hub = w.resource_mut::<HubState>();
        let (open_name, bytes) = match read {
            Ok(r) => r,
            Err(e) => {
                hub.uploading = false;
                hub.share_note = e;
                hub.changed = true;
                return;
            }
        };
        let name = if hub.share_name.trim().is_empty() {
            open_name
        } else {
            hub.share_name.clone()
        };
        let (description, tags) = (hub.share_description.clone(), hub.share_tags.clone());
        hub.run(move || Reply::Uploaded(hub::upload(&url, &token, &name, &description, &tags, &bytes)));
    });
}

/// takes in what the calls sent back
pub fn poll_hub(mut hub: ResMut<HubState>, mut s: ResMut<Settings>, mut status: ResMut<Status>, time: Res<Time>) {
    let replies: Vec<Reply> = match hub.rx.lock() {
        Ok(rx) => rx.try_iter().collect(),
        Err(_) => return,
    };
    for reply in replies {
        hub.changed = true;
        match reply {
            Reply::Connected(r) => {
                hub.connecting = false;
                hub.connection = match r {
                    Ok((info, Ok(me))) => format!(
                        "connected to {} as {} · level {} {} · {} points",
                        clip(&info.name, 60),
                        clip(&me.username, 40),
                        me.level,
                        clip(&me.title, 40),
                        me.points
                    ),
                    Ok((info, Err(e))) => format!("connected to {} · {}", clip(&info.name, 60), e),
                    Err(e) => e,
                };
            }
            Reply::Saves {
                generation,
                page,
                result,
            } => {
                if generation != hub.generation {
                    continue;
                }
                hub.browsing = false;
                hub.browsed = true;
                match result {
                    Ok(p) => {
                        if page == 0 {
                            hub.saves.clear();
                            hub.browse_note.clear();
                        }
                        hub.saves.extend(p.saves);
                        hub.more = p.more;
                        hub.page = page;
                    }
                    Err(e) => hub.browse_note = e,
                }
            }
            Reply::Downloaded(r) => {
                hub.downloading = None;
                let msg = match r {
                    Ok(path) => {
                        s.rescan = true;
                        s.changed = true;
                        format!("downloaded {}; load it from saves above", save_name(&path))
                    }
                    Err(e) => e,
                };
                status.text = msg.clone();
                status.until = time.elapsed_secs_f64() + 5.0;
                hub.browse_note = msg;
            }
            Reply::Uploaded(r) => {
                hub.uploading = false;
                hub.share_note = match r {
                    Ok(u) if u.status == "approved" => "shared: it's on the hub now".into(),
                    Ok(_) => "sent: it shows on the hub once an admin approves it".into(),
                    Err(e) => e,
                };
            }
        }
    }
}

/// untrusted text, kept short and on one line
fn clip(s: &str, n: usize) -> String {
    let one: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > n {
        format!("{}…", one.chars().take(n - 1).collect::<String>())
    } else {
        one
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", n, if n == 1 { one } else { many })
}

fn button(commands: &mut Commands, parent: Entity, label: &str, variant: ButtonVariant) -> Entity {
    commands
        .spawn_scene(plain_scene_button(label.to_string(), variant))
        .insert(ChildOf(parent))
        .id()
}

pub fn rebuild_hub(
    s: Res<Settings>,
    mut hub: ResMut<HubState>,
    mut config: ResMut<Config>,
    db: Res<Db>,
    fonts: Res<Fonts>,
    mut inputs: Query<(&HubField, &mut EditableText)>,
    mut lines: Query<(&mut Text, &mut Node, &HubLine)>,
    parts: (
        Single<Entity, With<HubSorts>>,
        Single<Entity, With<HubList>>,
        Single<Entity, With<HubMore>>,
    ),
    mut commands: Commands,
) {
    if !s.open {
        if hub.was_open {
            hub.was_open = false;
        }
        return;
    }
    if !hub.was_open {
        // the menu just opened: fill the boxes from the settings and the open save
        hub.was_open = true;
        hub.url = config.hub_url.clone();
        hub.token = config.hub_token.clone();
        hub.share_name = save_name(db.0.path());
        for (field, mut t) in &mut inputs {
            let v = match field.0 {
                Field::Url => &hub.url,
                Field::Token => &hub.token,
                Field::Name => &hub.share_name,
                _ => continue,
            };
            t.editor_mut().set_text(v);
            t.queue_edit(TextEdit::TextEnd(false));
        }
        hub.changed = true;
        // a hub set up before: see what's there the first time the menu opens
        if !hub.browsed && hub.connection.is_empty() && !hub.url.trim().is_empty() {
            hub.connect(&mut config);
        }
    }
    if !std::mem::take(&mut hub.changed) {
        return;
    }
    let (sorts, list, more) = (*parts.0, *parts.1, *parts.2);

    for (mut t, mut node, line) in &mut lines {
        let v = match line.0 {
            Line::Connection if hub.connecting => "…".to_string(),
            Line::Connection => hub.connection.clone(),
            Line::Browse => hub.browse_note.clone(),
            Line::Share if hub.uploading => "…".to_string(),
            Line::Share => hub.share_note.clone(),
        };
        // an empty line takes no room
        let display = if v.is_empty() { Display::None } else { Display::Flex };
        if node.display != display {
            node.display = display;
        }
        if t.0 != v {
            t.0 = v;
        }
    }

    // the sort buttons
    commands.entity(sorts).despawn_related::<Children>();
    text(&mut commands, sorts, "sort by", &fonts.regular, 12.0, colors::TEXT_DIM);
    for sort in Sort::ALL {
        let variant = if sort == hub.sort {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Normal
        };
        let b = button(&mut commands, sorts, sort.name(), variant);
        commands
            .entity(b)
            .observe(move |_a: On<Activate>, mut hub: ResMut<HubState>| {
                hub.sort = sort;
                hub.browse(0);
            });
    }

    // the saves, two to a row
    commands.entity(list).despawn_related::<Children>();
    for save in &hub.saves {
        let card = commands
            .spawn((
                Node {
                    width: percent(49.3),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    padding: UiRect::axes(px(10), px(8)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(colors::CODE_BG),
                ChildOf(list),
            ))
            .id();
        let title = format!(
            "{}{}",
            clip(&save.name, 40),
            if save.featured { "  · featured" } else { "" }
        );
        text(&mut commands, card, &title, &fonts.bold, 15.0, colors::TEXT);
        let by = format!("by {} · {}", clip(&save.author, 30), human_size(save.size));
        text(&mut commands, card, &by, &fonts.regular, 12.0, colors::TEXT_DIM);
        if !save.description.trim().is_empty() {
            let d = clip(&save.description, 110);
            text(&mut commands, card, &d, &fonts.regular, 13.0, colors::TEXT);
        }
        let counts = format!(
            "{} · {}",
            plural(save.notes, "note", "notes"),
            plural(save.memories, "memory", "memories")
        );
        text(&mut commands, card, &counts, &fonts.regular, 12.0, colors::TEXT_DIM);
        let tags: Vec<String> = save.tags.iter().take(8).map(|t| format!("#{}", clip(t, 24))).collect();
        if !tags.is_empty() {
            text(&mut commands, card, &tags.join(" "), &fonts.regular, 12.0, colors::LINK);
        }
        let foot = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                    // the buttons line up along a row of cards
                    margin: UiRect::top(Val::Auto),
                    padding: UiRect::top(px(4)),
                    ..default()
                },
                ChildOf(card),
            ))
            .id();
        let busy = hub.downloading.as_ref() == Some(&save.id);
        let b = button(
            &mut commands,
            foot,
            if busy { "…" } else { "download" },
            ButtonVariant::Normal,
        );
        let stats = format!(
            "{} · {}",
            plural(save.stars, "star", "stars"),
            plural(save.downloads, "download", "downloads")
        );
        text(&mut commands, foot, &stats, &fonts.regular, 12.0, colors::TEXT_DIM);
        let save = save.clone();
        commands.entity(b).observe(
            move |_a: On<Activate>, mut hub: ResMut<HubState>, config: Res<Config>| {
                hub.download(save.clone(), config.saves_dir.clone());
            },
        );
    }

    // paging
    commands.entity(more).despawn_related::<Children>();
    if hub.browsing {
        text(&mut commands, more, "…", &fonts.regular, 13.0, colors::TEXT_DIM);
    } else if hub.more {
        let b = button(&mut commands, more, "more", ButtonVariant::Normal);
        commands
            .entity(b)
            .observe(|_a: On<Activate>, mut hub: ResMut<HubState>| {
                let next = hub.page + 1;
                hub.browse(next);
            });
    } else if !hub.browsed && hub.browse_note.is_empty() {
        let t = "connect to a hub to see the saves shared there";
        text(&mut commands, more, t, &fonts.regular, 12.0, colors::TEXT_DIM);
    } else if hub.browsed && hub.saves.is_empty() && hub.browse_note.is_empty() {
        text(
            &mut commands,
            more,
            "no saves found",
            &fonts.regular,
            13.0,
            colors::TEXT_DIM,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping() {
        assert_eq!(clip("a\nb\t c", 10), "a b c");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(plural(1, "note", "notes"), "1 note");
    }
}
