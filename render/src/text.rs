//! Texts in the room: Fira Sans (the app's own face, OFL), laid out in lines that wrap at a
//! width, and a glyph atlas the labels are drawn from. Sizes are in ems: a label is as
//! many world units tall per em as its size says, as the webview's labels were.

use std::collections::HashMap;
use std::sync::OnceLock;

use ab_glyph::{Font, FontRef, Glyph, PxScale, ScaleFont};

static FIRA: &[u8] = include_bytes!("../fonts/FiraSans-Regular.ttf");

pub fn font() -> &'static FontRef<'static> {
    static F: OnceLock<FontRef<'static>> = OnceLock::new();
    F.get_or_init(|| FontRef::try_from_slice(FIRA).expect("the bundled face reads"))
}

/// the CSS line height the labels had
pub const LINE: f32 = 1.15;
/// the labels' side padding (3px at 32px), in ems
pub const PAD: f32 = 3.0 / 32.0;

/// a glyph placed: which, and where its pen starts (ems, y down from the top, baseline)
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub ch: char,
    pub x: f32,
    pub y: f32,
}

/// a text laid out: its glyphs, and its width and height in ems (padding included)
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Layout {
    pub glyphs: Vec<Placed>,
    pub w: f32,
    pub h: f32,
}

fn advance(c: char) -> f32 {
    let f = font().as_scaled(PxScale::from(1.0));
    f.h_advance(f.glyph_id(c))
}

/// the width of `0` in ems (what `ch` means)
pub fn ch() -> f32 {
    advance('0')
}

/// lays a text out in lines no wider than `wrap` ems (breaking at spaces, or inside a word
/// longer than a line), keeping its own line breaks
pub fn layout(text: &str, wrap: f32) -> Layout {
    let f = font().as_scaled(PxScale::from(1.0));
    let ascent = f.ascent();
    let mut lines: Vec<Vec<(char, f32)>> = Vec::new();
    for para in text.split('\n') {
        let mut line: Vec<(char, f32)> = Vec::new();
        let mut x = 0.0f32;
        let mut word_start = 0usize;
        for c in para.chars() {
            let a = advance(c);
            if x + a > wrap && !line.is_empty() && c != ' ' {
                // break at the last space, or here when the word is a line of its own
                let cut = if word_start > 0 { word_start } else { line.len() };
                let rest: Vec<(char, f32)> = line.split_off(cut);
                let rest: Vec<(char, f32)> = rest.into_iter().skip_while(|(c, _)| *c == ' ').collect();
                while line.last().is_some_and(|(c, _)| *c == ' ') {
                    line.pop();
                }
                lines.push(std::mem::take(&mut line));
                x = 0.0;
                for (c2, _) in rest {
                    let a2 = advance(c2);
                    line.push((c2, x));
                    x += a2;
                }
                word_start = 0;
            }
            if c == ' ' {
                word_start = line.len() + 1;
            }
            line.push((c, x));
            x += a;
        }
        lines.push(line);
    }
    let mut out = Layout::default();
    let mut w = 0.0f32;
    for (i, line) in lines.iter().enumerate() {
        let y = i as f32 * LINE + (LINE - 1.0) / 2.0 + ascent;
        let end = line.last().map(|(c, x)| x + advance(*c)).unwrap_or(0.0);
        w = w.max(end);
        for (c, x) in line {
            if *c != ' ' {
                out.glyphs.push(Placed { ch: *c, x: x + PAD, y });
            }
        }
    }
    out.w = w + 2.0 * PAD;
    out.h = lines.len().max(1) as f32 * LINE;
    out
}

/// where a glyph is in the atlas, and how it sits on its pen position (ems)
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub uv: [f32; 4],
    /// the glyph's box from the pen: left, top, right, bottom, in ems
    pub at: [f32; 4],
}

/// glyphs drawn once at `PX` pixels an em into one coverage texture, as they're needed
pub struct Atlas {
    pub size: u32,
    pub pixels: Vec<u8>,
    cells: HashMap<char, Option<Cell>>,
    x: u32,
    y: u32,
    row: u32,
    /// pixels changed since the texture was last written
    pub dirty: bool,
}

pub const PX: f32 = 64.0;

impl Atlas {
    pub fn new(size: u32) -> Atlas {
        Atlas {
            size,
            pixels: vec![0; (size * size) as usize],
            cells: HashMap::new(),
            x: 1,
            y: 1,
            row: 0,
            dirty: true,
        }
    }

    /// a glyph's cell (drawn now if it's new); None for one with nothing to draw
    pub fn cell(&mut self, c: char) -> Option<Cell> {
        if let Some(cell) = self.cells.get(&c) {
            return *cell;
        }
        let f = font();
        let g: Glyph = f
            .glyph_id(c)
            .with_scale_and_position(PxScale::from(PX), ab_glyph::point(0.0, 0.0));
        let cell = f.outline_glyph(g).and_then(|o| {
            let b = o.px_bounds();
            let (w, h) = (b.width().ceil() as u32 + 2, b.height().ceil() as u32 + 2);
            if self.x + w + 1 >= self.size {
                self.x = 1;
                self.y += self.row + 1;
                self.row = 0;
            }
            if self.y + h + 1 >= self.size {
                return None;
            }
            let (ox, oy) = (self.x + 1, self.y + 1);
            o.draw(|x, y, v| {
                let i = ((oy + y) * self.size + ox + x) as usize;
                if let Some(p) = self.pixels.get_mut(i) {
                    *p = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            });
            let s = self.size as f32;
            let cell = Cell {
                uv: [
                    ox as f32 / s,
                    oy as f32 / s,
                    (ox as f32 + b.width()) / s,
                    (oy as f32 + b.height()) / s,
                ],
                at: [b.min.x / PX, b.min.y / PX, b.max.x / PX, b.max.y / PX],
            };
            self.x += w + 1;
            self.row = self.row.max(h);
            self.dirty = true;
            Some(cell)
        });
        self.cells.insert(c, cell);
        cell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_wrap_at_their_width() {
        let one = layout("a quiet lake", 30.0);
        assert!(one.w > 3.0 && one.w < 8.0, "{}", one.w);
        assert_eq!(one.h, LINE);
        let two = layout("a quiet lake under a grey sky", 5.0);
        assert!(two.h >= 2.0 * LINE, "{}", two.h);
        assert!(two.w <= 5.0 + 2.0 * PAD + 0.01, "{}", two.w);
        let nl = layout("one\ntwo", 30.0);
        assert_eq!(nl.h, 2.0 * LINE);
        let mut a = Atlas::new(512);
        assert!(a.cell('a').is_some());
        assert!(a.cell(' ').is_none());
        assert!(a.pixels.iter().any(|p| *p > 200));
    }
}
