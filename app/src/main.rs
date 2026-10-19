//! anend's app with Dioxus: the desktop app (the system webview) and, from the same code,
//! the web app (WebAssembly). See docs/next.md, "The Dioxus app".

mod backend;
mod fonts;
mod gem;
mod highlight;
mod hubnet;
mod icons;
mod keys;
mod md;
mod parity;
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
    // WebKitGTK's DMA-BUF renderer needs GBM buffers the driver may refuse ("Failed to
    // create GBM buffer ... Invalid argument", NVIDIA's driver and some Wayland setups),
    // which leaves the window white. Turn it off unless the person chose otherwise
    // (`WEBKIT_DISABLE_DMABUF_RENDERER=0` keeps it).
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: first thing in main, before any thread is started.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
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
    // the icon goes on the window itself: `Config::with_icon` sets it on the config's own
    // window, which `with_window` then replaces (so it never showed in the title bar)
    let window = WindowBuilder::new()
        .with_title("anend")
        .with_window_icon(window_icon())
        .with_inner_size(LogicalSize::new(1200.0, 820.0));
    #[cfg(target_os = "linux")]
    desktop_entry();
    let cfg = Config::new()
        .with_window(window)
        .with_menu(None)
        .with_disable_context_menu(true)
        // the sand window's colour, so the first frame isn't a different shade
        .with_background_color((0x92, 0x7a, 0x4f, 0xff));
    dioxus::LaunchBuilder::desktop().with_cfg(cfg).launch(shell::App);
}

/// the window's icon, from assets/icon.png
#[cfg(not(target_arch = "wasm32"))]
fn window_icon() -> Option<dioxus::desktop::tao::window::Icon> {
    let dec = png::Decoder::new(std::io::Cursor::new(include_bytes!("../../assets/icon.png")));
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => return None,
    };
    dioxus::desktop::tao::window::Icon::from_rgba(rgba, info.width, info.height).ok()
}

/// Wayland desktops (GNOME, KDE) draw a window's icon from the desktop entry matching its
/// app id, not from the window: a user-level entry and icon are put in place once (never
/// overwriting one the person changed)
#[cfg(target_os = "linux")]
fn desktop_entry() {
    let Some(data) = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
    else {
        return;
    };
    let icon = data.join("icons/hicolor/256x256/apps/anend.png");
    if !icon.exists() {
        let _ = std::fs::create_dir_all(icon.parent().unwrap_or(&data));
        let _ = std::fs::write(&icon, include_bytes!("../../assets/icon.png"));
    }
    let entry = data.join("applications/anend-app.desktop");
    if !entry.exists()
        && let Ok(exe) = std::env::current_exe()
    {
        let _ = std::fs::create_dir_all(entry.parent().unwrap_or(&data));
        let _ = std::fs::write(
            &entry,
            format!(
                "[Desktop Entry]\nType=Application\nName=anend\nComment=notes, memory and the space\nExec=\"{}\" %F\nIcon=anend\nStartupWMClass=anend-app\nCategories=Office;Utility;\n",
                exe.display()
            ),
        );
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    dioxus::LaunchBuilder::web().launch(shell::App);
}
