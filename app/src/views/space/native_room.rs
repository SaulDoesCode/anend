//! The space drawn natively (Linux desktop, `ANEND_NATIVE_SPACE=1`; crate::native): the
//! room's element stays empty and see-through, the room is drawn under it by
//! `anend-render`, and its pointer and keys come here through Dioxus's own handlers.
//!
//! What it does so far (the renderer line moves the room over in slices): the save's
//! blocks in their colours, boxes and spheres, and the wires between them, on the room's
//! colour; flying (W A S D, Space and C, Shift faster) and looking (the right button held).

use std::collections::HashSet;

use dioxus::prelude::*;
use serde_json::Value;

use crate::ui::*;

fn hex(c: &str) -> [f32; 3] {
    let h = |i: usize| u8::from_str_radix(c.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(128) as f32 / 255.0;
    // sRGB to linear: the renderer writes an sRGB picture
    let lin = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    if c.starts_with('#') && c.len() == 7 {
        [lin(h(1)), lin(h(3)), lin(h(5))]
    } else {
        [0.5, 0.5, 0.5]
    }
}

/// the room's blocks and wires as the renderer draws them
fn scene(blocks: &[Value], wires: &[Value], sky: &str) -> anend_render::Scene {
    use anend_render::glam::{Quat, Vec3};
    let mut out = anend_render::Scene {
        sky: hex(sky),
        ..Default::default()
    };
    let f = |v: &Value, k: &str, d: f32| v[k].as_f64().map(|x| x as f32).unwrap_or(d);
    let mut at = std::collections::HashMap::new();
    for b in blocks {
        let r = b["rot"]
            .as_array()
            .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect::<Vec<_>>())
            .unwrap_or_default();
        let rot = if r.len() == 4 {
            Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize()
        } else {
            Quat::IDENTITY
        };
        let pos = Vec3::new(f(b, "x", 0.0), f(b, "y", 0.0), f(b, "z", 0.0));
        let tex = b["texture"].as_str().unwrap_or("");
        out.blocks.push(anend_render::Block {
            at: pos,
            rot,
            size: Vec3::new(f(b, "w", 1.0), f(b, "h", 1.0), f(b, "d", 1.0)),
            colour: if tex.starts_with('#') {
                hex(tex)
            } else {
                [0.75, 0.72, 0.68]
            },
            sphere: b["shape"] == "sphere",
        });
        at.insert(b["id"].as_str().unwrap_or("").to_string(), pos);
    }
    for w in wires {
        let Some(p0) = at.get(w["id"].as_str().unwrap_or("")) else {
            continue;
        };
        for t in w["links"].as_array().into_iter().flatten() {
            let Some(p1) = at.get(t.as_str().unwrap_or("")) else {
                continue;
            };
            let mid = (*p0 + *p1) * 0.5 - Vec3::Y * (p0.distance(*p1) * 0.18).min(1.2);
            let points = (0..=20)
                .map(|i| {
                    let t = i as f32 / 20.0;
                    *p0 * (1.0 - t) * (1.0 - t) + mid * 2.0 * t * (1.0 - t) + *p1 * t * t
                })
                .collect();
            out.lines.push(anend_render::Line {
                points,
                colour: [0.35, 0.27, 0.15],
            });
        }
    }
    out
}

#[component]
pub fn NativeRoom() -> Element {
    let ui = ui();
    let api = api();
    // the window: the drawing area goes under the webview once
    use_hook(|| {
        let d = dioxus::desktop::window();
        crate::native::install(&d.window, &d.webview);
        crate::native::show(true);
    });
    use_drop(|| crate::native::show(false));
    use_effect(move || {
        let p = ui.prefs.read();
        crate::native::set_window_colour(&crate::theme::colour(&p.palette, &p.colours, "window"));
    });
    let blocks = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            let _ = (ui.changed)();
            async move {
                api.call("blocks.list", serde_json::json!({}))
                    .await
                    .ok()
                    .and_then(|r| r["blocks"].as_array().cloned())
                    .unwrap_or_default()
            }
        }
    });
    let wires = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            let _ = (ui.changed)();
            async move {
                api.call("wiring.list", serde_json::json!({}))
                    .await
                    .ok()
                    .and_then(|r| r.as_array().cloned())
                    .unwrap_or_default()
            }
        }
    });
    let mut rect = use_signal(|| (0.0f64, 0.0f64, 0.0f64, 0.0f64));
    let mut el: Signal<Option<std::rc::Rc<MountedData>>> = use_signal(|| None);
    let mut camera = use_signal(|| anend_render::Camera {
        at: anend_render::glam::Vec3::new(0.0, 0.5, 6.0),
        ..Default::default()
    });
    let mut keys: Signal<HashSet<String>> = use_signal(HashSet::new);
    let mut looking: Signal<Option<(f64, f64)>> = use_signal(|| None);
    // the frame loop: fly by the held keys, keep the element's place, ask for a picture
    use_future(move || async move {
        let mut last = std::time::Instant::now();
        let mut tick = 0u32;
        loop {
            crate::ui::sleep_ms(16).await;
            let dt = last.elapsed().as_secs_f32().min(0.1);
            last = std::time::Instant::now();
            tick += 1;
            if tick % 30 == 1
                && let Some(m) = el.peek().clone()
                && let Ok(r) = m.get_client_rect().await
            {
                rect.set((r.origin.x, r.origin.y, r.size.width, r.size.height));
            }
            let held = keys.peek().clone();
            if !held.is_empty() {
                use anend_render::glam::{Quat, Vec3};
                let mut c = *camera.peek();
                let rot = Quat::from_euler(anend_render::glam::EulerRot::YXZ, c.yaw, c.pitch, 0.0);
                let fast = if held.contains("Shift") { 4.0 } else { 1.0 };
                let v = 6.0 * fast * dt;
                let mut mv = Vec3::ZERO;
                for (k, d) in [("w", Vec3::NEG_Z), ("s", Vec3::Z), ("a", Vec3::NEG_X), ("d", Vec3::X)] {
                    if held.contains(k) {
                        mv += rot * d;
                    }
                }
                if held.contains(" ") {
                    mv += Vec3::Y;
                }
                if held.contains("c") {
                    mv -= Vec3::Y;
                }
                c.at += mv * v;
                camera.set(c);
            }
            let (x, y, w, h) = *rect.peek();
            if w < 2.0 || h < 2.0 {
                continue;
            }
            let sky = ui.prefs.peek().space_bg.clone();
            let s = scene(
                &blocks.peek().clone().unwrap_or_default(),
                &wires.peek().clone().unwrap_or_default(),
                &sky,
            );
            crate::native::draw(crate::native::Job {
                scene: s,
                camera: *camera.peek(),
                x,
                y,
                w: w as u32,
                h: h as u32,
            });
        }
    });
    rsx! {
        // the page is see-through where the room is: the window's colour is painted under it
        style { "html, body, #main, .app, .view.space {{ background: transparent !important; }}" }
        div { id: "space-host", class: "space-host native", tabindex: "0",
            "aria-label": "the space: right drag looks, W A S D fly",
            onmounted: move |e| el.set(Some(e.data())),
            onkeydown: move |e: KeyboardEvent| {
                let k = match e.key() { Key::Character(c) => c.to_lowercase(), Key::Shift => "Shift".into(), _ => return };
                keys.write().insert(k);
            },
            onkeyup: move |e: KeyboardEvent| {
                let k = match e.key() { Key::Character(c) => c.to_lowercase(), Key::Shift => "Shift".into(), _ => return };
                keys.write().remove(&k);
            },
            onblur: move |_| keys.write().clear(),
            onmousedown: move |e: MouseEvent| {
                if e.trigger_button() == Some(dioxus::html::input_data::MouseButton::Secondary) {
                    let p = e.client_coordinates();
                    looking.set(Some((p.x, p.y)));
                }
            },
            onmousemove: move |e: MouseEvent| {
                if let Some((x0, y0)) = looking() {
                    let p = e.client_coordinates();
                    let mut c = *camera.peek();
                    c.yaw -= ((p.x - x0) * 0.004) as f32;
                    c.pitch = (c.pitch - ((p.y - y0) * 0.004) as f32).clamp(-1.55, 1.55);
                    camera.set(c);
                    looking.set(Some((p.x, p.y)));
                }
            },
            onmouseup: move |_| looking.set(None),
            oncontextmenu: move |e| e.prevent_default(),
        }
    }
}
