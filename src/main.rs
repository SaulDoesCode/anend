//! anend notes: a desktop markdown note app with the look and workflow of the old anend
//! blog admin panel, built on Bevy's feathers widgets. Notes live in a local redb file.

#![allow(clippy::type_complexity, clippy::too_many_arguments)] // bevy queries and systems are naturally verbose

mod bar;
mod config;
mod keys;
mod markdown;
mod memory;
mod memory_ui;
mod note;
mod palette;
mod settings;
mod simd;
mod sizing;
mod space;
mod space_ui;
mod spread;
mod store;
mod theme;
mod typeface;
mod ui;
mod weum;
mod zen;

use bevy::feathers::FeathersPlugins;
use bevy::feathers::theme::UiTheme;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{ExitCondition, WindowResolution};
use std::path::PathBuf;

use crate::note::Note;
use crate::store::Store;
use crate::ui::*;

struct Args {
    db: Option<PathBuf>,
    import: Vec<PathBuf>,
    import_weum: Vec<PathBuf>,
    screenshot: Option<PathBuf>,
    space: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        db: None,
        import: Vec::new(),
        import_weum: Vec::new(),
        screenshot: None,
        space: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().unwrap_or_else(|| fail(&format!("{} needs a value", name)));
        match arg.as_str() {
            "--db" => args.db = Some(PathBuf::from(value("--db"))),
            "--import" => args.import.push(PathBuf::from(value("--import"))),
            "--import-weum" => args.import_weum.push(PathBuf::from(value("--import-weum"))),
            "--screenshot" => args.screenshot = Some(PathBuf::from(value("--screenshot"))),
            "--space" => args.space = true,
            "-h" | "--help" => {
                println!(
                    "anend: markdown notes\n\n\
                     usage: anend [--db <file>] [--import <file.json>]... [--import-weum <all-of-it.json>]...\n\n  \
                     --db <file>       the save to open (default: the last one, now {})\n  \
                     --import <file>   import notes, e.g. writs exported from the old anend server\n                    \
                     (a JSON array or one JSON document per line), then exit\n  \
                     --import-weum <file>  import weum memories from its backup (all-of-it.json), then exit\n  \
                     --space           open in the space\n\n\
                     ctrl+s save, ctrl+n new note, ctrl+e view/edit, ctrl+1..5 views, ctrl+, settings (saves, fonts, zen, keys)",
                    config::Config::load().current_save().display()
                );
                std::process::exit(0);
            }
            other => fail(&format!("unknown argument {:?}, see --help", other)),
        }
    }
    args
}

fn fail(msg: &str) -> ! {
    eprintln!("anend: {}", msg);
    std::process::exit(2);
}

fn read_notes(path: &PathBuf) -> Vec<Note> {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| fail(&format!("cannot read {}: {}", path.display(), e)));
    match serde_json::from_str::<Vec<Note>>(&raw) {
        Ok(notes) => notes,
        Err(_) => raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                serde_json::from_str(l).unwrap_or_else(|e| fail(&format!("bad note in {}: {}", path.display(), e)))
            })
            .collect(),
    }
}

#[derive(Resource)]
struct ScreenshotPath(PathBuf);

/// `--screenshot <file.png>`: renders a few frames, captures the window and quits
fn screenshot(
    mut frames: Local<u32>,
    path: Res<ScreenshotPath>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    *frames += 1;
    if *frames == 30 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.0.clone()));
    }
    if *frames == 45 {
        exit.write(AppExit::Success);
    }
}

fn main() {
    let args = parse_args();
    let mut config = config::Config::load();
    let db_path = args.db.clone().unwrap_or_else(|| config.current_save());
    let store = Store::open(&db_path).unwrap_or_else(|e| fail(&format!("cannot open {}: {}", db_path.display(), e)));
    if args.db.is_none() && config.current.is_none() {
        config.current = Some(db_path.clone());
        config.save();
    }

    if !args.import.is_empty() || !args.import_weum.is_empty() {
        for path in &args.import {
            let n = store.import(read_notes(path)).unwrap_or_else(|e| fail(&e));
            println!("imported {} notes from {}", n, path.display());
        }
        for path in &args.import_weum {
            let raw = std::fs::read_to_string(path)
                .unwrap_or_else(|e| fail(&format!("cannot read {}: {}", path.display(), e)));
            let entries: Vec<(String, Vec<u8>)> = serde_json::from_str(&raw)
                .unwrap_or_else(|e| fail(&format!("{} isn't a weum backup: {}", path.display(), e)));
            let n = store.import_memories(entries).unwrap_or_else(|e| fail(&e));
            println!("imported {} memories from {}", n, path.display());
        }
        return;
    }
    let keymap = config
        .keymap
        .as_ref()
        .map(|v| keys::Keymap::from_json(v.to_string().as_bytes()))
        .unwrap_or_default();

    let notes = store
        .all()
        .unwrap_or_else(|e| fail(&format!("cannot read notes: {}", e)));

    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "anend".into(),
                resolution: WindowResolution::new(1200, 800),
                ..default()
            }),
            // closing saves the open note first, see ui::on_close
            close_when_requested: false,
            exit_condition: ExitCondition::OnPrimaryClosed,
            ..default()
        }),
        FeathersPlugins,
        bevy_slugtext::SlugTextPlugin,
    ))
    .insert_resource(UiTheme(theme::sand_theme(&palette::Palette::default())))
    .insert_resource(ClearColor(theme::colors::MOOD_DARK))
    .init_resource::<palette::Palette>()
    .insert_resource(Db(store))
    // start on the most recent note, like picking the top writ in the old editor
    .insert_resource(Session {
        active: notes.first().cloned().unwrap_or_default(),
        route: if args.space { Route::Space } else { Route::Editor },
        ..default()
    })
    .insert_resource(Notes(notes))
    .init_resource::<Status>()
    .init_resource::<Redraw>()
    .init_resource::<Fonts>()
    .init_resource::<memory_ui::SerifFont>()
    .init_resource::<memory_ui::MemoryState>()
    .init_resource::<keys::Remap>()
    .init_resource::<space::SlugFont>()
    .init_resource::<space::Space>()
    .init_resource::<bar::BarState>()
    .init_resource::<settings::Settings>()
    .init_resource::<typeface::Typeface>()
    .init_resource::<zen::Zen>()
    .insert_resource(keymap)
    .insert_resource(config)
    .add_message::<keys::Fired>()
    .add_systems(Startup, (ui::scene.spawn(), space::setup))
    .add_systems(
        Update,
        (
            (keys::read_keys, keys::persist_keymap).chain(),
            (app_actions, memory_ui::memory_actions),
            on_close,
            (
                apply_route,
                load_editor_text,
                rebuild_list,
                rebuild_meta,
                rebuild_preview,
                rebuild_reader,
                rebuild_stats,
                rebuild_remap,
            ),
            bar::bar_route,
            (
                memory_ui::focus_on_open,
                memory_ui::apply_term,
                memory_ui::expire_toasts,
                memory_ui::rebuild_results,
                memory_ui::rebuild_chrome,
                memory_ui::rebuild_toasts,
            ),
            status_text,
        )
            .chain(),
    )
    .add_systems(
        Update,
        (
            space::apply_space_route,
            space::load_font,
            space::space_enter,
            space::space_input,
            space::space_fly,
            space::space_physics,
            space::space_visuals,
            space_ui::apply_term,
            space_ui::space_panels,
            space_ui::rebuild_help,
            space_ui::rebuild_qline,
            space_ui::rebuild_termrow,
            space_ui::rebuild_metrics,
            space_ui::build_opts,
        )
            .chain()
            .after(bar::bar_route),
    )
    .add_systems(
        Update,
        (
            bar::notes_side,
            settings::rebuild_settings,
            typeface::apply_typeface,
            typeface::apply_memory_font,
            palette::tag_paints,
            palette::apply_palette,
            settings::paint_swatches,
            settings::sync_color_editor,
            sizing::apply_sizes,
            zen::zen_fade,
            zen::nav_autohide,
            zen::list_autohide,
        )
            .chain()
            .after(space_ui::build_opts),
    );
    if let Some(path) = args.screenshot {
        app.insert_resource(ScreenshotPath(path))
            .add_systems(Update, screenshot);
    }
    app.run();
}
