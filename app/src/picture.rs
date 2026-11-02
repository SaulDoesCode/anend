//! The native room's picture shown *in* the page, for desktops without GTK under the
//! webview (macOS, Windows; on Linux with `ANEND_SPACE_PICTURE=1`, to check it): each frame
//! `anend-render` draws is kept as an uncompressed bitmap, served to the page through the
//! `anendframe` custom protocol, and shown by an `img` in the space's element. No script:
//! the `img`'s address changes with each new frame (docs/wip/questions.md 118).
//! The prefab pictures (`thumbs`) are drawn here too, on every desktop.

use std::borrow::Cow;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use dioxus::desktop::wry::http::{Request, Response};

/// whether the room's picture goes into the page (not under it)
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| !cfg!(target_os = "linux") || std::env::var("ANEND_SPACE_PICTURE").is_ok_and(|v| v == "1"))
}

/// the newest frame: its number and its bitmap
fn frame() -> &'static Mutex<(u64, Arc<Vec<u8>>)> {
    static F: OnceLock<Mutex<(u64, Arc<Vec<u8>>)>> = OnceLock::new();
    F.get_or_init(|| Mutex::new((0, Arc::new(bmp(1, 1, &[0, 0, 0, 0])))))
}

/// the newest frame's number (the page asks for it by number, so each new one loads)
pub fn frame_no() -> u64 {
    frame().lock().map(|f| f.0).unwrap_or(0)
}

/// where the page fetches a frame (WebView2 serves custom protocols over http)
pub fn frame_url(n: u64) -> String {
    if cfg!(target_os = "windows") {
        format!("http://anendframe.localhost/{n}")
    } else {
        format!("anendframe://localhost/{n}")
    }
}

/// the custom protocol: the newest frame, whatever number was asked for
pub fn protocol(_: dioxus::desktop::wry::WebViewId, _req: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let data = frame().lock().map(|f| f.1.clone()).unwrap_or_default();
    Response::builder()
        .header("Content-Type", "image/bmp")
        .header("Cache-Control", "no-store")
        .body(Cow::<'static, [u8]>::Owned(data.as_ref().clone()))
        .unwrap_or_else(|_| Response::new(Cow::Owned(Vec::new())))
}

/// RGBA rows (from the top) as a 32-bit BMP, top-down
fn bmp(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let size = (w * h * 4) as usize;
    let mut out = Vec::with_capacity(54 + size);
    let le32 = |v: u32| v.to_le_bytes();
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&le32(54 + size as u32));
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&le32(54));
    out.extend_from_slice(&le32(40));
    out.extend_from_slice(&le32(w));
    out.extend_from_slice(&(-(h as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&le32(size as u32));
    out.extend_from_slice(&[0; 16]);
    for p in rgba.as_chunks::<4>().0 {
        out.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    out
}

/// one picture to draw: the room, from where, how big
pub struct Job {
    pub scene: anend_render::Scene,
    pub camera: anend_render::Camera,
    pub w: u32,
    pub h: u32,
}

fn worker() -> &'static Mutex<Sender<Job>> {
    static W: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    W.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("anend space picture".into())
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
        if r.wants_again {
            px = r.render(&job.scene, &job.camera);
        }
        let b = Arc::new(bmp(job.w, job.h, &px));
        if let Ok(mut f) = frame().lock() {
            *f = (f.0 + 1, b);
        }
    }
}

/// the prefabs' small pictures (112 pixels square, see-through round their parts), drawn
/// on a renderer of their own: id → a PNG data URL, sent back when done
pub fn thumbs(
    jobs: Vec<(serde_json::Value, anend_render::Scene, anend_render::Camera)>,
    done: Sender<serde_json::Value>,
) {
    if jobs.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        const SIZE: u32 = 112;
        let Ok(mut r) = anend_render::Renderer::new(SIZE, SIZE) else {
            return;
        };
        let mut out = serde_json::Map::new();
        for (id, scene, camera) in jobs {
            let mut px = r.render(&scene, &camera);
            // the picture's edges are blended with nothing: their colour back to full
            for p in px.as_chunks_mut::<4>().0 {
                let a = p[3] as f32 / 255.0;
                if a > 0.0 && a < 1.0 {
                    for c in &mut p[..3] {
                        *c = ((*c as f32 / a).min(255.0)) as u8;
                    }
                }
            }
            let key = id.as_str().map(String::from).unwrap_or_else(|| id.to_string());
            out.insert(
                key,
                serde_json::Value::String(anend_render::tiles::rgba_url(SIZE, SIZE, px)),
            );
        }
        let _ = done.send(serde_json::Value::Object(out));
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn frames_are_bitmaps() {
        let b = super::bmp(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]);
        assert_eq!(&b[..2], b"BM");
        assert_eq!(b.len(), 54 + 8);
        // BGRA: the first pixel red
        assert_eq!(&b[54..58], &[0, 0, 255, 255]);
    }
}
