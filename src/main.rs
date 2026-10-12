//! anend notes: a desktop markdown note app with the look and workflow of the old anend
//! blog admin panel, built on Bevy's feathers widgets. Notes live in a local redb file.

#![allow(clippy::type_complexity)] // bevy queries are naturally verbose

mod keys;
mod markdown;
mod memory;
mod memory_ui;
mod note;
mod simd;
mod store;
mod theme;
mod ui;
mod weum;

use bevy::feathers::FeathersPlugins;
use bevy::feathers::theme::UiTheme;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{ExitCondition, WindowResolution};
use std::path::PathBuf;

use crate::note::Note;
use crate::store::Store;
use crate::ui::*;

/// where notes are kept unless `--db` says otherwise
fn default_db_path() -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("anend")
        .join("notes.redb")
}

struct Args {
    db: PathBuf,
    import: Vec<PathBuf>,
    import_weum: Vec<PathBuf>,
    screenshot: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut args = Args {
        db: default_db_path(),
        import: Vec::new(),
        import_weum: Vec::new(),
        screenshot: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().unwrap_or_else(|| fail(&format!("{} needs a value", name)));
        match arg.as_str() {
            "--db" => args.db = PathBuf::from(value("--db")),
            "--import" => args.import.push(PathBuf::from(value("--import"))),
            "--import-weum" => args.import_weum.push(PathBuf::from(value("--import-weum"))),
            "--screenshot" => args.screenshot = Some(PathBuf::from(value("--screenshot"))),
            "-h" | "--help" => {
                println!(
                    "anend: markdown notes\n\n\
                     usage: anend [--db <file>] [--import <file.json>]... [--import-weum <all-of-it.json>]...\n\n  \
                     --db <file>       where the notes are stored (default {})\n  \
                     --import <file>   import notes, e.g. writs exported from the old anend server\n                    \
                     (a JSON array or one JSON document per line), then exit\n  \
                     --import-weum <file>  import weum memories from its backup (all-of-it.json), then exit\n\n\
                     ctrl+s save, ctrl+n new note, ctrl+e view/edit, ctrl+1..4 views; press k (or the keys button) to remap",
                    default_db_path().display()
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
    let store = Store::open(&args.db).unwrap_or_else(|e| fail(&format!("cannot open {}: {}", args.db.display(), e)));

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
    let keymap = store
        .setting("keymap")
        .ok()
        .flatten()
        .map(|b| keys::Keymap::from_json(&b))
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
    ))
    .insert_resource(UiTheme(theme::sand_theme()))
    .insert_resource(ClearColor(theme::colors::MOOD_DARK))
    .insert_resource(Db(store))
    // start on the most recent note, like picking the top writ in the old editor
    .insert_resource(Session {
        active: notes.first().cloned().unwrap_or_default(),
        ..default()
    })
    .insert_resource(Notes(notes))
    .init_resource::<Status>()
    .init_resource::<Redraw>()
    .init_resource::<Fonts>()
    .init_resource::<memory_ui::SerifFont>()
    .init_resource::<memory_ui::MemoryState>()
    .init_resource::<keys::Remap>()
    .insert_resource(keymap)
    .add_message::<keys::Fired>()
    .add_systems(Startup, ui::scene.spawn())
    .add_systems(
        Update,
        (
            keys::read_keys,
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
    );
    if let Some(path) = args.screenshot {
        app.insert_resource(ScreenshotPath(path))
            .add_systems(Update, screenshot);
    }
    app.run();
}
