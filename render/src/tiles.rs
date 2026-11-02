//! The pictures blocks repeat, drawn in Rust as space.js drew them into canvases: a text's
//! tile (its words on paper, its grouping colour as a band at the foot), a recipe's tile
//! (core's graph::textures: a source cropped, mirrored, turned and tinted) and a picture's
//! thumbnail, decoded from the data URL Rust sends.

use std::collections::HashMap;

use ab_glyph::{Font, PxScale, ScaleFont};
use base64::Engine;
use serde_json::Value;

use crate::room::css_rgb;

/// a picture being drawn: straight RGBA in sRGB, 0–1, as a 2D canvas blends
#[derive(Clone, Debug)]
pub struct Canvas {
    pub w: u32,
    pub h: u32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        Canvas {
            w,
            h,
            px: vec![[0.0; 4]; (w * h) as usize],
        }
    }

    /// blends a colour over a pixel by a coverage (source-over)
    fn blend(&mut self, x: i32, y: i32, c: [f32; 4], cover: f32) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let a = (c[3] * cover).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let d = &mut self.px[(y as u32 * self.w + x as u32) as usize];
        let out_a = a + d[3] * (1.0 - a);
        for i in 0..3 {
            d[i] = if out_a > 0.0 {
                (c[i] * a + d[i] * d[3] * (1.0 - a)) / out_a
            } else {
                0.0
            };
        }
        d[3] = out_a;
    }

    pub fn fill(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 4]) {
        let (ax, ay) = (x0.max(0.0).floor() as i32, y0.max(0.0).floor() as i32);
        let (bx, by) = (x1.min(self.w as f32).ceil() as i32, y1.min(self.h as f32).ceil() as i32);
        for y in ay..by {
            for x in ax..bx {
                // the part of the pixel the rect covers (crisp at whole pixels)
                let cx = ((x + 1) as f32).min(x1) - (x as f32).max(x0);
                let cy = ((y + 1) as f32).min(y1) - (y as f32).max(y0);
                self.blend(x, y, c, cx.clamp(0.0, 1.0) * cy.clamp(0.0, 1.0));
            }
        }
    }

    /// a rect's outline `lw` wide, centred on its edges (as `strokeRect`)
    pub fn stroke(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, lw: f32, c: [f32; 4]) {
        let h = lw / 2.0;
        self.fill(x0 - h, y0 - h, x1 + h, y0 + h, c);
        self.fill(x0 - h, y1 - h, x1 + h, y1 + h, c);
        self.fill(x0 - h, y0 + h, x0 + h, y1 - h, c);
        self.fill(x1 - h, y0 + h, x1 + h, y1 - h, c);
    }

    /// a line of text, its middle at (x, y) (`textAlign: center`, `textBaseline: middle`)
    pub fn text(&mut self, s: &str, x: f32, y: f32, size: f32, c: [f32; 4]) {
        let f = crate::text::font();
        let sf = f.as_scaled(PxScale::from(size));
        let w = measure(s, size);
        let base = y + (sf.ascent() + sf.descent()) / 2.0;
        let mut pen = x - w / 2.0;
        for ch in s.chars() {
            let id = sf.glyph_id(ch);
            let g = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(pen, base));
            pen += sf.h_advance(id);
            if let Some(o) = f.outline_glyph(g) {
                let b = o.px_bounds();
                o.draw(|gx, gy, v| self.blend(b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, c, v));
            }
        }
    }

    pub fn get(&self, x: u32, y: u32) -> [f32; 4] {
        self.px[(y.min(self.h - 1) * self.w + x.min(self.w - 1)) as usize]
    }

    /// the picture as RGBA bytes (alpha straight)
    pub fn bytes(&self) -> Vec<u8> {
        self.px
            .iter()
            .flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            .collect()
    }
}

/// a text's width at so many pixels an em
pub fn measure(s: &str, size: f32) -> f32 {
    let sf = crate::text::font().as_scaled(PxScale::from(size));
    s.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum()
}

/// a CSS colour as straight sRGB RGBA (alpha from `rgba()`'s fourth number)
pub fn css(c: &str) -> [f32; 4] {
    let [r, g, b] = css_rgb(c).unwrap_or([0.5, 0.5, 0.5]);
    let a = c
        .trim()
        .strip_prefix("rgba(")
        .and_then(|r| r.trim_end_matches(')').split(',').nth(3))
        .and_then(|a| a.trim().parse::<f32>().ok())
        .unwrap_or(1.0);
    [r, g, b, a]
}

/// a text's tile, as space.js's `textCanvas`: its words as large as fit, on paper, with
/// its grouping colour as a band at the foot
pub fn text_tile(text: &str, colour: Option<&str>) -> Canvas {
    let mut c = Canvas::new(256, 256);
    c.fill(0.0, 0.0, 256.0, 256.0, css("#f3ead9"));
    c.fill(0.0, 244.0, 256.0, 256.0, css(colour.unwrap_or("#8a7350")));
    c.stroke(1.0, 1.0, 255.0, 255.0, 2.0, css("rgba(60, 45, 25, 0.25)"));
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut size = 64.0f32;
    let mut lines: Vec<String> = Vec::new();
    while size >= 12.0 {
        lines.clear();
        let mut line = String::new();
        for w in &words {
            let t = if line.is_empty() {
                w.to_string()
            } else {
                format!("{line} {w}")
            };
            if measure(&t, size) > 224.0 && !line.is_empty() {
                lines.push(std::mem::replace(&mut line, w.to_string()));
            } else {
                line = t;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        if lines.len() as f32 * size * 1.15 <= 220.0 && lines.iter().all(|l| measure(l, size) <= 232.0) {
            break;
        }
        size -= 4.0;
    }
    let size = size.max(12.0);
    let top = 122.0 - (lines.len().saturating_sub(1) as f32 * size * 1.15) / 2.0;
    for (i, l) in lines.iter().take(12).enumerate() {
        c.text(l, 128.0, top + i as f32 * size * 1.15, size, css("#2b2219"));
    }
    c
}

/// a picture decoded (a `data:` URL's PNG or JPEG), or none
pub fn decode(src: &str) -> Option<Canvas> {
    let b64 = src.split_once(";base64,")?.1;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut c = Canvas::new(w.max(1), h.max(1));
    for (i, p) in img.pixels().enumerate() {
        c.px[i] = [
            p[0] as f32 / 255.0,
            p[1] as f32 / 255.0,
            p[2] as f32 / 255.0,
            p[3] as f32 / 255.0,
        ];
    }
    Some(c)
}

/// a picture sampled between pixels (bilinear), in its own pixels
fn sample(img: &Canvas, x: f32, y: f32) -> [f32; 4] {
    let (x, y) = (x - 0.5, y - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |dx: f32, dy: f32| img.get((x0 + dx).max(0.0) as u32, (y0 + dy).max(0.0) as u32);
    let (a, b, c, d) = (at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0));
    let mut out = [0.0; 4];
    for i in 0..4 {
        out[i] = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
    }
    out
}

/// a recipe's tile (`size` square), as space.js's `recipeCanvas`: its source (a colour, a
/// picture's thumbnail, a text's tile) cropped, mirrored, turned a quarter at a time (and
/// `extra` quarters more), then tinted
pub fn recipe_tile(about: &Value, extra: u32, size: u32, pictures: &mut Pictures) -> Canvas {
    let r = &about["recipe"];
    let src = &r["source"];
    let n = size as f32;
    let source: Option<Canvas> = if src.get("text").is_some_and(|t| !t.is_null()) {
        let gone = about.get("text_gone").is_some_and(|g| !g.is_null() && *g != false);
        Some(text_tile(
            if gone {
                "(gone)"
            } else {
                about["text"].as_str().unwrap_or("")
            },
            about["colour"].as_str(),
        ))
    } else if src.get("picture").is_some_and(|p| !p.is_null()) {
        about["src"].as_str().and_then(|s| pictures.get(s))
    } else {
        None
    };
    let mut c = Canvas::new(size, size);
    let turn = ((r["turn"].as_u64().unwrap_or(0) as u32 + extra) % 4) as f32 * std::f32::consts::FRAC_PI_2;
    let mirror = r["mirror"] == true;
    let (cs, sn) = (turn.cos().round(), turn.sin().round());
    if let Some(plain) = src["plain"].as_str() {
        c.fill(0.0, 0.0, n, n, css(plain));
    } else if let Some(img) = &source {
        let crop: Vec<f32> = r["crop"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
            .unwrap_or_default();
        let [l, t, rr, b] = if crop.len() == 4 {
            [crop[0], crop[1], crop[2], crop[3]]
        } else {
            [0.0, 0.0, 1.0, 1.0]
        };
        let (iw, ih) = (img.w as f32, img.h as f32);
        let (sx, sy, sw, sh) = (l * iw, t * ih, ((rr - l) * iw).max(1.0), ((b - t) * ih).max(1.0));
        for y in 0..size {
            for x in 0..size {
                // where this pixel was before the turn and the mirror
                let (dx, dy) = (x as f32 + 0.5 - n / 2.0, y as f32 + 0.5 - n / 2.0);
                let (mut lx, ly) = (dx * cs + dy * sn, -dx * sn + dy * cs);
                if mirror {
                    lx = -lx;
                }
                let (u, v) = ((lx + n / 2.0) / n, (ly + n / 2.0) / n);
                let p = sample(img, sx + u * sw, sy + v * sh);
                c.px[(y * size + x) as usize] = p;
            }
        }
    } else {
        let gone = about.get("gone").is_some() || about.get("picture_gone").is_some();
        c.fill(0.0, 0.0, n, n, css(if gone { "#9a8f84" } else { "#b8a07a" }));
    }
    if let (Some(tint), Some(amount)) = (r["tint"].as_str(), r["tint_amount"].as_f64())
        && amount > 0.0
    {
        let mut t = css(tint);
        t[3] *= amount as f32;
        c.fill(0.0, 0.0, n, n, t);
    }
    c
}

/// a recipe's tiles as the textures window shows them (`w`×`h` pixels): `tiles` across,
/// each block variant in turn when `variants`, each moved by its offset as the room
/// samples it; a PNG data URL
pub fn preview(about: &Value, tiles: u32, variants: bool, w: u32, h: u32, pictures: &mut Pictures) -> String {
    let r = &about["recipe"];
    let n = tiles.max(1);
    let (w, h) = (w.clamp(1, 1024), h.clamp(1, 1024));
    let pair = |k: &str, d: f32| -> [f32; 2] {
        let a: Vec<f32> = r[k]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(d as f64) as f32).collect())
            .unwrap_or_default();
        if a.len() == 2 { [a[0], a[1]] } else { [d, d] }
    };
    let ([sw, sh], [ou, ov]) = (pair("scale", 1.0), pair("offset", 0.0));
    let nv = r["variants"].as_u64().unwrap_or(1).max(1) as u32;
    let turns = variants && r["turn_variants"] == true;
    let kinds = if variants { nv.max(if turns { 4 } else { 1 }) } else { 1 };
    let made: Vec<Canvas> = (0..kinds)
        .map(|k| recipe_tile(about, if r["turn_variants"] == true { k % 4 } else { 0 }, 128, pictures))
        .collect();
    // a tile keeps its shape (scale w by h)
    let cw = w as f32 / n as f32;
    let ch = cw * (sh / sw.max(1e-3));
    let mut c = Canvas::new(w, h);
    for py in 0..h {
        for px in 0..w {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            let (cx, cy) = ((x / cw).floor(), (y / ch).floor());
            let v = if variants {
                ((cy as u32) * n + cx as u32) % nv
            } else {
                0
            };
            let tile = made.get(v as usize).unwrap_or(&made[0]);
            let fu = (ou + v as f32 * 0.381966).rem_euclid(1.0);
            let fv = (ov + v as f32 * 0.618034).rem_euclid(1.0);
            let lx = (x - cx * cw + fu * cw).rem_euclid(cw) / cw;
            let ly = (y - cy * ch - fv * ch).rem_euclid(ch) / ch;
            c.px[(py * w + px) as usize] = sample(tile, lx * tile.w as f32, ly * tile.h as f32);
        }
    }
    png_url(&c)
}

/// a picture as a PNG data URL
pub fn png_url(c: &Canvas) -> String {
    rgba_url(c.w, c.h, c.bytes())
}

/// RGBA bytes (alpha straight) as a PNG data URL
pub fn rgba_url(w: u32, h: u32, rgba: Vec<u8>) -> String {
    let mut png = Vec::new();
    let img = image::RgbaImage::from_raw(w, h, rgba).unwrap_or_default();
    let _ = image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png);
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&png)
    )
}

/// pictures decoded once, by their data URL's hash
#[derive(Default)]
pub struct Pictures {
    seen: HashMap<u64, Option<Canvas>>,
}

impl Pictures {
    pub fn get(&mut self, src: &str) -> Option<Canvas> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        src.hash(&mut h);
        let k = h.finish();
        if self.seen.len() > 400 {
            self.seen.clear();
        }
        self.seen.entry(k).or_insert_with(|| decode(src)).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tiles_draw_their_sources() {
        let t = text_tile("a quiet lake", Some("#ff0000"));
        assert_eq!(t.bytes().len(), 256 * 256 * 4);
        // the band is the colour, the paper is paper, and some ink is dark
        assert!(t.get(128, 250)[0] > 0.9 && t.get(128, 250)[1] < 0.1);
        assert!(t.get(20, 20)[0] > 0.9);
        assert!((60..180).any(|y| (0..256).any(|x| t.get(x, y)[0] < 0.3)));
        let mut pics = Pictures::default();
        let plain = recipe_tile(
            &json!({"recipe": {"source": {"plain": "#0000ff"}, "tint": "#ff0000", "tint_amount": 0.5}}),
            0,
            32,
            &mut pics,
        );
        let p = plain.get(5, 5);
        assert!(p[0] > 0.4 && p[2] > 0.4 && p[1] < 0.1, "{p:?}");
        // a picture, turned a quarter: its left column goes to the top row
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(0, 1, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 0, 255, 255]));
        img.put_pixel(1, 1, image::Rgba([0, 0, 255, 255]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let src = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        let about = json!({"recipe": {"source": {"picture": "p1"}, "turn": 1}, "src": src});
        let tile = recipe_tile(&about, 0, 16, &mut pics);
        assert!(tile.get(8, 1)[0] > 0.9, "the top: {:?}", tile.get(8, 1));
        assert!(tile.get(8, 14)[2] > 0.9, "the foot: {:?}", tile.get(8, 14));
    }
}
