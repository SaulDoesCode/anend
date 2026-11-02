//! The room in Rust on the Linux desktop (`ANEND_NATIVE_SPACE=1`; crate::native): space.js's
//! place is taken by `anend_render::room::Room`, which takes the same commands and sends the
//! same events, and is drawn by `anend-render` under the see-through webview. Its element
//! stays empty; the pointer and keys come to the room through Dioxus's own handlers. No
//! JavaScript is involved.

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
}

fn room() -> Rc<RefCell<Room>> {
    ROOM.with(|r| r.clone())
}

/// the room's loop: the commands Rust sends, a step of the room, its events, its picture
pub fn run(mut rx: futures_channel::mpsc::UnboundedReceiver<Value>, on_event: impl Fn(SpaceEvent) + 'static) {
    {
        let d = dioxus::desktop::window();
        crate::native::install(&d.window, &d.webview);
    }
    spawn(async move {
        let mut last = std::time::Instant::now();
        let mut tick = 0u32;
        let (thumbs_tx, thumbs_rx) = std::sync::mpsc::channel::<Value>();
        loop {
            crate::ui::sleep_ms(15).await;
            let r = room();
            // the prefabs' pictures, drawn off this thread
            let jobs = r.borrow_mut().take_thumbs();
            crate::native::thumbs(jobs, thumbs_tx.clone());
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
            let dt = last.elapsed().as_secs_f32();
            last = std::time::Instant::now();
            r.borrow_mut().frame(dt);
            let audio = r.borrow_mut().take_audio();
            crate::native_audio::play(audio);
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
            if rm.dirty || tick.is_multiple_of(30) {
                let scene = rm.scene();
                let camera = rm.camera();
                drop(rm);
                crate::native::draw(crate::native::Job {
                    scene,
                    camera,
                    x,
                    y,
                    w: w as u32,
                    h: h as u32,
                });
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
    use_hook(|| crate::native::show(true));
    use_drop(|| crate::native::show(false));
    let ui = crate::ui::ui();
    use_effect(move || {
        let p = ui.prefs.read();
        crate::native::set_window_colour(&crate::theme::colour(&p.palette, &p.colours, "window"));
    });
    let at = |e: &MouseEvent| {
        let p = e.element_coordinates();
        (p.x as f32, p.y as f32)
    };
    rsx! {
        // the page is see-through where the room is: the window's colour is painted under it
        style { "html, body, #main, .app, .view.space, .space {{ background: transparent !important; }}" }
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
        }
    }
}
