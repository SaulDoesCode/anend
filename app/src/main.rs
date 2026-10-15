//! anend's app with Dioxus: the desktop app (the system webview) and, from the same code,
//! the web app (WebAssembly). See docs/next.md, "The Dioxus app".

mod backend;
mod gem;
mod icons;
mod prefs;
mod shell;
mod theme;
mod ui;
mod views;

use std::rc::Rc;

use ui::Api;

#[cfg(not(target_arch = "wasm32"))]
static LOCAL: std::sync::OnceLock<backend::local::Local> = std::sync::OnceLock::new();

/// the backend this build talks to
pub fn make_api() -> Api {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Api(Rc::new(
            LOCAL.get().expect("the save is opened before the window").clone(),
        ))
    }
    #[cfg(target_arch = "wasm32")]
    {
        Api(Rc::new(backend::hub::Hub::here()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
    // `anend-app [--save path.redb]`; else the last save, else main.redb in the saves folder
    let args: Vec<String> = std::env::args().collect();
    let arg_save = args
        .iter()
        .position(|a| a == "--save" || a == "--db")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);
    let prefs = prefs::load();
    let save = arg_save.or_else(|| prefs.save.clone().map(Into::into));
    let dir = prefs::data_dir().join("saves");
    let local = match backend::local::Local::open(&dir, save.as_deref()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("anend: couldn't open the save: {e}");
            std::process::exit(1);
        }
    };
    // `--import all-of-it.json`: weum's texts into the save first
    if let Some(file) = args.iter().position(|a| a == "--import").and_then(|i| args.get(i + 1)) {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let mut f = backend::Backend::import_texts(&local, &text, "");
        // the worker answers on its own thread; wait for it
        let r = loop {
            match futures_util::FutureExt::now_or_never(&mut f) {
                Some(r) => break r,
                None => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        };
        match r {
            Ok(im) => eprintln!("anend: imported {} texts", im.memories),
            Err(e) => eprintln!("anend: the import failed: {e}"),
        }
    }
    let _ = LOCAL.set(local);
    let window = WindowBuilder::new()
        .with_title("anend")
        .with_inner_size(LogicalSize::new(1200.0, 820.0));
    let cfg = Config::new()
        .with_window(window)
        .with_menu(None)
        .with_disable_context_menu(true)
        .with_background_color((0x40, 0x40, 0x40, 0xff));
    dioxus::LaunchBuilder::desktop().with_cfg(cfg).launch(shell::App);
}

#[cfg(target_arch = "wasm32")]
fn main() {
    dioxus::LaunchBuilder::web().launch(shell::App);
}
