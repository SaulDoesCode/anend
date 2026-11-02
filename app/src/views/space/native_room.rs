//! The room in Rust: space.js's place is taken by `anend_render::room::Room`, which takes the
//! same commands and sends the same events. It's drawn by `anend-render`: on the Linux
//! desktop under the see-through webview (crate::native), on macOS and Windows into the page
//! as frames (crate::picture), and on the web straight into a canvas with WebGPU. The pointer
//! and keys come to the room through Dioxus's own handlers. No JavaScript is involved.

use std::cell::RefCell;
use std::rc::Rc;

use anend_render::room::{Mods, Room};
use dioxus::prelude::*;
use serde_json::Value;

use super::SpaceEvent;

thread_local! {
    static ROOM: Rc<RefCell<Room>> = Rc::new(RefCell::new(Room::new()));
    static HOST: RefCell<Option<Rc<MountedData>>> = const { RefCell::new(None) };
    static RECT: RefCell<(f64, f64, f64, f64)> = const { RefCell::new((0.0, 0.0, 0.0, 0.0)) };
    /// whether the space is the view on screen
    static SHOWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// the newest frame shown in the page (the picture path), and the signal its img reads
    static FRAME: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static FRAME_SIG: RefCell<Option<Signal<u64>>> = const { RefCell::new(None) };
}

fn room() -> Rc<RefCell<Room>> {
    ROOM.with(|r| r.clone())
}

/// what differs between the desktop and the web: where the picture goes, the clock, sounds
/// and the prefab pictures
#[cfg(feature = "desktop")]
mod gpu {
    use super::*;

    pub fn install() {
        // on Linux the picture goes under the page (GTK); elsewhere, into it (an img)
        #[cfg(target_os = "linux")]
        if !crate::picture::enabled() {
            let d = dioxus::desktop::window();
            crate::native::install(&d.window, &d.webview);
        }
    }

    pub fn now() -> f64 {
        thread_local! { static T0: std::time::Instant = std::time::Instant::now(); }
        T0.with(|t| t.elapsed().as_secs_f64())
    }

    pub fn thumbs(jobs: Vec<(Value, anend_render::Scene, anend_render::Camera)>, tx: &std::sync::mpsc::Sender<Value>) {
        crate::picture::thumbs(jobs, tx.clone());
    }

    pub fn audio(cmds: Vec<anend_render::room::Audio>) {
        crate::native_audio::play(cmds);
    }

    /// whether the picture is an img in the page
    pub fn picture() -> bool {
        crate::picture::enabled()
    }

    pub fn draw(scene: anend_render::Scene, camera: anend_render::Camera, x: f64, y: f64, w: u32, h: u32) {
        let _ = (x, y);
        if crate::picture::enabled() {
            crate::picture::draw(crate::picture::Job { scene, camera, w, h });
        } else {
            #[cfg(target_os = "linux")]
            crate::native::draw(crate::native::Job {
                scene,
                camera,
                x,
                y,
                w,
                h,
            });
        }
    }

    pub fn shown(on: bool) {
        #[cfg(target_os = "linux")]
        if !crate::picture::enabled() {
            crate::native::show(on);
        }
        let _ = on;
    }

    pub fn window_colour(hex: &str) {
        #[cfg(target_os = "linux")]
        crate::native::set_window_colour(hex);
        let _ = hex;
    }

    pub fn frame_no() -> u64 {
        crate::picture::frame_no()
    }

    pub fn frame_url(n: u64) -> String {
        crate::picture::frame_url(n)
    }
}

#[cfg(not(feature = "desktop"))]
mod gpu {
    use super::*;
    use std::cell::Cell;
    use wasm_bindgen::JsCast;

    thread_local! {
        static RENDERER: RefCell<Option<anend_render::Renderer>> = const { RefCell::new(None) };
        static STARTED: Cell<bool> = const { Cell::new(false) };
    }

    pub fn install() {}

    pub fn now() -> f64 {
        js_sys::Date::now() / 1000.0
    }

    /// the prefab pictures need a picture read back, which WebGPU only gives later: not yet
    pub fn thumbs(_: Vec<(Value, anend_render::Scene, anend_render::Camera)>, _: &std::sync::mpsc::Sender<Value>) {}

    /// sounds on the web aren't mixed yet
    pub fn audio(_: Vec<anend_render::room::Audio>) {}

    pub fn picture() -> bool {
        false
    }

    fn canvas() -> Option<web_sys::HtmlCanvasElement> {
        web_sys::window()?
            .document()?
            .get_element_by_id("space-canvas")?
            .dyn_into()
            .ok()
    }

    /// draws into the canvas (made the first time, once the canvas is on the page)
    pub fn draw(scene: anend_render::Scene, camera: anend_render::Camera, _x: f64, _y: f64, w: u32, h: u32) {
        let Some(c) = canvas() else { return };
        if c.width() != w || c.height() != h {
            c.set_width(w);
            c.set_height(h);
        }
        let ready = RENDERER.with(|r| r.borrow().is_some());
        if !ready {
            if !STARTED.with(|s| s.replace(true)) {
                spawn(async move {
                    match anend_render::Renderer::for_canvas(c).await {
                        Ok(r) => {
                            web_sys::console::log_1(
                                &format!("anend: the room draws with WebGPU ({})", r.adapter_name).into(),
                            );
                            RENDERER.with(|x| *x.borrow_mut() = Some(r));
                        }
                        Err(e) => {
                            web_sys::console::warn_1(&format!("anend: the room can't draw with WebGPU: {e}").into())
                        }
                    }
                });
            }
            return;
        }
        RENDERER.with(|r| {
            if let Some(r) = r.borrow_mut().as_mut() {
                r.resize(w, h);
                if let Err(e) = r.present(&scene, &camera) {
                    web_sys::console::warn_1(&format!("anend: {e}").into());
                }
            }
        });
    }

    pub fn shown(_: bool) {}

    pub fn window_colour(_: &str) {}

    pub fn frame_no() -> u64 {
        0
    }

    pub fn frame_url(_: u64) -> String {
        String::new()
    }
}

/// the room's loop: the commands Rust sends, a step of the room, its events, its picture
pub fn run(mut rx: futures_channel::mpsc::UnboundedReceiver<Value>, on_event: impl Fn(SpaceEvent) + 'static) {
    gpu::install();
    spawn(async move {
        let mut last = gpu::now();
        let mut tick = 0u32;
        let (thumbs_tx, thumbs_rx) = std::sync::mpsc::channel::<Value>();
        loop {
            crate::ui::sleep_ms(15).await;
            let r = room();
            // the prefabs' pictures, drawn off this thread
            let jobs = r.borrow_mut().take_thumbs();
            gpu::thumbs(jobs, &thumbs_tx);
            while let Ok(data) = thumbs_rx.try_recv() {
                if let Ok(e) =
                    serde_json::from_value::<SpaceEvent>(serde_json::json!({ "kind": "thumbs", "data": data }))
                {
                    on_event(e);
                }
            }
            loop {
                match rx.try_recv() {
                    Ok(c) => {
                        // the room's element takes the keys
                        if c["cmd"] == "focus"
                            && let Some(m) = HOST.with(|h| h.borrow().clone())
                        {
                            spawn(async move {
                                let _ = m.set_focus(true).await;
                            });
                        }
                        r.borrow_mut().command(&c)
                    }
                    Err(futures_channel::mpsc::TryRecvError::Closed) => return,
                    Err(_) => break,
                }
            }
            let now = gpu::now();
            let dt = (now - last) as f32;
            last = now;
            r.borrow_mut().frame(dt);
            let audio = r.borrow_mut().take_audio();
            gpu::audio(audio);
            let evs = r.borrow_mut().take_events();
            for e in evs {
                if let Ok(e) = serde_json::from_value::<SpaceEvent>(e) {
                    on_event(e);
                }
            }
            // where the room's element is (it moves with the window's size)
            tick += 1;
            if tick % 20 == 1
                && let Some(m) = HOST.with(|h| h.borrow().clone())
                && let Ok(rc) = m.get_client_rect().await
            {
                RECT.with(|x| *x.borrow_mut() = (rc.origin.x, rc.origin.y, rc.size.width, rc.size.height));
            }
            let (x, y, w, h) = RECT.with(|x| *x.borrow());
            if w < 2.0 || h < 2.0 {
                continue;
            }
            let mut rm = r.borrow_mut();
            rm.resize(w as f32, h as f32);
            if SHOWN.with(|s| s.get()) && (rm.dirty || tick.is_multiple_of(30)) {
                let scene = rm.scene();
                let camera = rm.camera();
                drop(rm);
                gpu::draw(scene, camera, x, y, w as u32, h as u32);
            }
            // a new frame in the page: the img asks for it
            if gpu::picture() {
                let n = gpu::frame_no();
                if FRAME.with(|f| f.get()) != n {
                    FRAME.with(|f| f.set(n));
                    if let Some(mut sig) = FRAME_SIG.with(|f| *f.borrow()) {
                        sig.set(n);
                    }
                }
            }
        }
    });
}

fn mods(m: dioxus::html::Modifiers) -> Mods {
    Mods {
        shift: m.shift(),
        ctrl: m.ctrl() || m.meta(),
        alt: m.alt(),
    }
}

fn button(e: &MouseEvent) -> u8 {
    use dioxus::html::input_data::MouseButton;
    match e.trigger_button() {
        Some(MouseButton::Secondary) => 2,
        Some(MouseButton::Auxiliary) => 1,
        _ => 0,
    }
}

/// the room's element: empty and see-through, its pointer and keys for the room
#[component]
pub fn NativeHost() -> Element {
    let ui = crate::ui::ui();
    // the room shows while the space is the view on screen (views are kept once built)
    let open = (ui.view)() == crate::ui::View::Space;
    let picture = gpu::picture();
    let frame = use_signal(|| 0u64);
    use_hook(move || FRAME_SIG.with(|f| *f.borrow_mut() = Some(frame)));
    use_effect(move || {
        let on = (ui.view)() == crate::ui::View::Space;
        SHOWN.with(|s| s.set(on));
        gpu::shown(on);
    });
    use_drop(move || {
        SHOWN.with(|s| s.set(false));
        gpu::shown(false);
    });
    use_effect(move || {
        let p = ui.prefs.read();
        gpu::window_colour(&crate::theme::colour(&p.palette, &p.colours, "window"));
    });
    let at = |e: &MouseEvent| {
        let p = e.element_coordinates();
        (p.x as f32, p.y as f32)
    };
    rsx! {
        // the page is see-through where the room is: the window's colour is painted under it
        if open && !picture && cfg!(feature = "desktop") {
            style { "html, body, #main, .app, .view.space, .space {{ background: transparent !important; }}" }
        }
        div { id: "space-host", class: "space-host native", tabindex: "0",
            "aria-label": "the space: right drag looks, W A S D fly, arrows hop between texts",
            onmounted: move |e| HOST.with(|h| *h.borrow_mut() = Some(e.data())),
            onkeydown: move |e: KeyboardEvent| {
                let code = e.code().to_string();
                if code.starts_with("Arrow") || code == "Space" {
                    e.prevent_default();
                }
                room().borrow_mut().key_down(&code, mods(e.modifiers()));
            },
            onkeyup: move |e: KeyboardEvent| room().borrow_mut().key_up(&e.code().to_string()),
            onblur: move |_| room().borrow_mut().blur(),
            onmousedown: move |e: MouseEvent| {
                let (x, y) = at(&e);
                room().borrow_mut().pointer_down(x, y, button(&e), mods(e.modifiers()));
            },
            onmousemove: move |e: MouseEvent| {
                let (x, y) = at(&e);
                room().borrow_mut().pointer_move(x, y, mods(e.modifiers()));
            },
            onmouseup: move |e: MouseEvent| {
                let (x, y) = at(&e);
                room().borrow_mut().pointer_up(x, y, button(&e), mods(e.modifiers()));
            },
            onwheel: move |e: WheelEvent| {
                e.prevent_default();
                let d = e.delta().strip_units();
                let (x, y) = { let p = e.element_coordinates(); (p.x as f32, p.y as f32) };
                room().borrow_mut().wheel(x, y, d.y as f32);
            },
            oncontextmenu: move |e| e.prevent_default(),
            if picture {
                img { class: "space-frame", src: "{gpu::frame_url(frame())}", alt: "", draggable: "false" }
            }
            if !cfg!(feature = "desktop") {
                canvas { id: "space-canvas", class: "space-frame" }
            }
        }
    }
}
