//! The space drawn natively on the Linux desktop, with no JavaScript (docs/wip/questions.md
//! 114): `anend-render` draws the room on a thread of its own, and a GTK drawing area
//! *under* the (transparent) webview paints each picture where the space's element is.
//! The HTML around and over the room (bars, windows, menus) stays on top; GTK puts the two
//! together in one window.
//!
//! macOS and Windows show the same room in the page instead (picture.rs); neither space.js nor three.js is in a desktop build
//! (docs/wip/questions.md 118).

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use gtk::prelude::*;

/// a picture ready to paint: BGRA (cairo's order), where it goes in the window
struct Frame {
    x: f64,
    y: f64,
    w: i32,
    h: i32,
    bgra: Vec<u8>,
}

/// what the drawing area paints: the window's colour, then the room's picture (while shown)
#[derive(Default)]
struct Paint {
    window: (f64, f64, f64),
    frame: Option<Frame>,
    shown: bool,
}

fn paint() -> &'static Arc<Mutex<Paint>> {
    static P: OnceLock<Arc<Mutex<Paint>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

thread_local! {
    static AREA: RefCell<Option<gtk::DrawingArea>> = const { RefCell::new(None) };
}

/// asks GTK (on its own thread) to paint again
fn redraw() {
    gtk::glib::MainContext::default().invoke(|| {
        AREA.with(|a| {
            if let Some(a) = a.borrow().as_ref() {
                a.queue_draw();
            }
        })
    });
}

/// puts the drawing area under the webview and makes the webview see-through; once
pub fn install(window: &dioxus::desktop::tao::window::Window, webview: &dioxus::desktop::wry::WebView) {
    use dioxus::desktop::tao::platform::unix::WindowExtUnix;
    if AREA.with(|a| a.borrow().is_some()) {
        return;
    }
    let Some(vbox) = window.default_vbox() else { return };
    let Some(web) = vbox.children().into_iter().next() else {
        return;
    };
    vbox.remove(&web);
    let overlay = gtk::Overlay::new();
    let area = gtk::DrawingArea::new();
    area.connect_draw(|_, cr| {
        let p = paint().lock().unwrap_or_else(|e| e.into_inner());
        let (r, g, b) = p.window;
        cr.set_source_rgb(r, g, b);
        let _ = cr.paint();
        if p.shown
            && let Some(f) = &p.frame
            && let Ok(s) =
                gtk::cairo::ImageSurface::create_for_data(f.bgra.clone(), gtk::cairo::Format::Rgb24, f.w, f.h, f.w * 4)
        {
            let _ = cr.set_source_surface(&s, f.x, f.y);
            let _ = cr.paint();
        }
        gtk::glib::Propagation::Stop
    });
    overlay.add(&area);
    overlay.add_overlay(&web);
    vbox.pack_start(&overlay, true, true, 0);
    overlay.show_all();
    // a webview at full opacity is put on screen by itself, over (and hiding) what's under
    // it; just under full, GTK draws it into the window with the room showing through
    web.set_opacity(0.99);
    let _ = webview.set_background_color((0, 0, 0, 0));
    AREA.with(|a| *a.borrow_mut() = Some(area));
}

/// the window's colour (behind the see-through parts of the page), `#rrggbb`
pub fn set_window_colour(hex: &str) {
    let h = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f64 / 255.0;
    paint().lock().unwrap_or_else(|e| e.into_inner()).window = (h(1), h(3), h(5));
    redraw();
}

/// whether the room's picture shows (the space is the view on screen)
pub fn show(on: bool) {
    paint().lock().unwrap_or_else(|e| e.into_inner()).shown = on;
    redraw();
}

/// one picture to draw: the room, from where, into which part of the window
pub struct Job {
    pub scene: anend_render::Scene,
    pub camera: anend_render::Camera,
    pub x: f64,
    pub y: f64,
    pub w: u32,
    pub h: u32,
}

fn worker() -> &'static Mutex<Sender<Job>> {
    static W: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    W.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("anend space".into())
            .spawn(move || draw_loop(rx))
            .expect("the space's thread");
        Mutex::new(tx)
    })
}

/// asks for a picture; only the newest asked for is drawn
pub fn draw(job: Job) {
    let _ = worker().lock().unwrap_or_else(|e| e.into_inner()).send(job);
}

fn draw_loop(rx: Receiver<Job>) {
    let mut renderer: Option<anend_render::Renderer> = None;
    while let Ok(mut job) = rx.recv() {
        while let Ok(newer) = rx.try_recv() {
            job = newer;
        }
        if job.w == 0 || job.h == 0 {
            continue;
        }
        let r = match &mut renderer {
            Some(r) => {
                r.resize(job.w, job.h);
                r
            }
            None => match anend_render::Renderer::new(job.w, job.h) {
                Ok(r) => renderer.insert(r),
                Err(e) => {
                    eprintln!("anend: the space can't be drawn natively: {e}");
                    return;
                }
            },
        };
        let mut px = r.render(&job.scene, &job.camera);
        // something hidden behind came into view: drawn again, so it shows now
        if r.wants_again {
            px = r.render(&job.scene, &job.camera);
        }
        for p in px.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
        }
        paint().lock().unwrap_or_else(|e| e.into_inner()).frame = Some(Frame {
            x: job.x,
            y: job.y,
            w: job.w as i32,
            h: job.h as i32,
            bgra: px,
        });
        redraw();
    }
}
