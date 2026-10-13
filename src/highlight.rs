//! Markdown highlighting in the editor.
//!
//! Bevy's editable text paints all of its glyphs in one colour, so the writing pad's own
//! glyphs are made transparent and a second text node lying exactly over it draws the
//! same glyphs in colour. Nothing is laid out twice: after the pad's layout runs, its
//! glyphs (positions, atlas cells) are copied into the overlay's layout, each pointing at
//! the overlay's text span whose colour fits the markdown around it. Editing, the cursor,
//! the selection, IME and scrolling all stay the pad's own; the overlay only paints. If
//! the glyphs can't be matched to the text (or highlighting is off) the pad shows its
//! own text again.
//!
//! [`classify`] is the markdown side: a class for every byte of the text, line by line,
//! so a long note costs one pass when it changes and nothing while it doesn't.

use bevy::color::{Alpha, Luminance, Mix};
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::text::{ComputedTextBlock, EditableText, EditableTextGeneration, TextCursorStyle, TextLayoutInfo};

use crate::palette::{Palette, Role};
use crate::ui::EditorPad;

/// what a stretch of markdown is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Class {
    Plain,
    /// syntax: the `#`s, `*`s, brackets and backticks around things
    Mark,
    Heading,
    Strong,
    Emph,
    Strike,
    Code,
    Link,
    Url,
    /// a list item's bullet or number
    List,
    /// a quote's text (its `>` is a [`Class::Quote`] too)
    Quote,
    /// a table's pipes and its alignment row
    Table,
    Tag,
    Rule,
    /// text still being composed by an input method
    Compose,
}

impl Class {
    pub const ALL: [Class; 15] = [
        Class::Plain,
        Class::Mark,
        Class::Heading,
        Class::Strong,
        Class::Emph,
        Class::Strike,
        Class::Code,
        Class::Link,
        Class::Url,
        Class::List,
        Class::Quote,
        Class::Table,
        Class::Tag,
        Class::Rule,
        Class::Compose,
    ];
}

/// a class for every byte of `text`
pub fn classify(text: &str) -> Vec<Class> {
    let mut out = vec![Class::Plain; text.len()];
    let mut fence: Option<(u8, usize)> = None;
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.len();
        let body = line.trim_end_matches(['\n', '\r']);
        classify_line(body, start, &mut out, &mut fence);
        start = end;
    }
    out
}

fn fill(out: &mut [Class], range: std::ops::Range<usize>, c: Class) {
    let end = range.end.min(out.len());
    if range.start < end {
        out[range.start..end].fill(c);
    }
}

/// a fence's char and length, if the line opens or closes one
fn fence_of(line: &str) -> Option<(u8, usize)> {
    let t = line.trim_start();
    if line.len() - t.len() > 3 {
        return None;
    }
    let ch = *t.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let n = t.bytes().take_while(|b| *b == ch).count();
    (n >= 3).then_some((ch, n))
}

fn classify_line(line: &str, at: usize, out: &mut [Class], fence: &mut Option<(u8, usize)>) {
    let span = at..at + line.len();
    // fenced code
    if let Some((ch, n)) = *fence {
        fill(out, span.clone(), Class::Code);
        if let Some((c2, n2)) = fence_of(line)
            && c2 == ch
            && n2 >= n
            && line.trim().bytes().all(|b| b == ch)
        {
            fill(out, span, Class::Mark);
            *fence = None;
        }
        return;
    }
    if let Some(f) = fence_of(line)
        && !(f.0 == b'`' && line.trim_start()[f.1..].contains('`'))
    {
        *fence = Some(f);
        fill(out, span, Class::Mark);
        return;
    }
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    // indented code (4 spaces or a tab, not inside a list item's text)
    if (line.starts_with("    ") || line.starts_with('\t')) && !trimmed.is_empty() && !is_list(trimmed) {
        fill(out, span, Class::Code);
        return;
    }
    // headings
    let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && indent < 4 && trimmed[hashes..].chars().next().is_none_or(|c| c == ' ' || c == '\t') {
        let mark_end = at + indent + hashes;
        fill(out, span.clone(), Class::Heading);
        fill(out, at..mark_end, Class::Mark);
        inline(line, at, indent + hashes, out, Class::Heading);
        // a closing run of #s is syntax too
        let t = line.trim_end();
        let closing = t.bytes().rev().take_while(|b| *b == b'#').count();
        if closing > 0 && closing < t.len() - indent - hashes && t.as_bytes()[t.len() - closing - 1] == b' ' {
            fill(out, at + t.len() - closing..at + t.len(), Class::Mark);
        }
        return;
    }
    // rules
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() >= 3
        && indent < 4
        && let Some(first) = compact.chars().next()
        && matches!(first, '-' | '*' | '_')
        && compact.chars().all(|c| c == first)
    {
        fill(out, span, Class::Rule);
        return;
    }
    // quotes: each > is a mark, the rest quoted text (which can still hold a list or inline marks)
    let mut i = indent;
    let mut base = Class::Plain;
    while line[i..].starts_with('>') {
        fill(out, at + i..at + i + 1, Class::Quote);
        base = Class::Quote;
        i += 1;
        while line[i..].starts_with(' ') {
            i += 1;
        }
    }
    if base == Class::Quote {
        fill(out, at + i..span.end, Class::Quote);
    }
    // tables: pipes, and the alignment row
    let rest = &line[i..];
    if rest.trim_start().starts_with('|') {
        let is_align = rest.trim().bytes().all(|b| matches!(b, b'|' | b'-' | b':' | b' '));
        if is_align {
            fill(out, at + i..span.end, Class::Table);
            return;
        }
        inline(line, at, i, out, base);
        for (j, b) in rest.bytes().enumerate() {
            if b == b'|' && out[at + i + j] != Class::Code {
                fill(out, at + i + j..at + i + j + 1, Class::Table);
            }
        }
        return;
    }
    // list items: the bullet or number, and a task box
    let lead = rest.len() - rest.trim_start().len();
    let item = &rest[lead..];
    if let Some(m) = list_marker(item) {
        let s = at + i + lead;
        fill(out, s..s + m, Class::List);
        let mut k = i + lead + m;
        let after = &line[k..];
        let t = after.trim_start();
        if t.starts_with("[ ]") || t.starts_with("[x]") || t.starts_with("[X]") {
            let o = k + after.len() - t.len();
            fill(out, at + o..at + o + 3, Class::List);
            k = o + 3;
        }
        inline(line, at, k, out, base);
        return;
    }
    inline(line, at, i, out, base);
}

fn is_list(t: &str) -> bool {
    list_marker(t).is_some()
}

/// the length of a list marker at the start of `t` (with its space), if there is one
fn list_marker(t: &str) -> Option<usize> {
    let b = t.as_bytes();
    if b.len() >= 2 && matches!(b[0], b'-' | b'*' | b'+') && (b[1] == b' ' || b[1] == b'\t') {
        return Some(1);
    }
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if (1..=9).contains(&digits)
        && b.len() > digits + 1
        && matches!(b[digits], b'.' | b')')
        && (b[digits + 1] == b' ' || b[digits + 1] == b'\t')
    {
        return Some(digits + 1);
    }
    None
}

/// inline marks from byte `from` of the line on: code spans, links, emphasis, tags
fn inline(line: &str, at: usize, from: usize, out: &mut [Class], base: Class) {
    let b = line.as_bytes();
    let n = b.len();
    let mut i = from;
    // open emphasis runs: (char, run length, the position after the run)
    let mut open: Vec<(u8, usize, usize)> = Vec::new();
    while i < n {
        let c = b[i];
        match c {
            b'\\' if i + 1 < n && b[i + 1].is_ascii_punctuation() => {
                fill(out, at + i..at + i + 1, Class::Mark);
                i += 2;
            }
            b'`' => {
                let run = b[i..].iter().take_while(|x| **x == b'`').count();
                // the matching run of the same length closes it
                let mut j = i + run;
                let mut close = None;
                while j < n {
                    if b[j] == b'`' {
                        let r = b[j..].iter().take_while(|x| **x == b'`').count();
                        if r == run {
                            close = Some(j);
                            break;
                        }
                        j += r;
                    } else {
                        j += 1;
                    }
                }
                match close {
                    Some(j) => {
                        fill(out, at + i..at + j + run, Class::Code);
                        fill(out, at + i..at + i + run, Class::Mark);
                        fill(out, at + j..at + j + run, Class::Mark);
                        i = j + run;
                    }
                    None => i += run,
                }
            }
            b'[' => match link_end(line, i) {
                Some((text_end, url_end)) => {
                    let link_class = if base == Class::Heading { Class::Heading } else { Class::Link };
                    fill(out, at + i..at + i + 1, Class::Mark);
                    fill(out, at + i + 1..at + text_end, link_class);
                    inline(&line[..text_end], at, i + 1, out, link_class);
                    fill(out, at + text_end..at + text_end + 2, Class::Mark);
                    fill(out, at + text_end + 2..at + url_end, Class::Url);
                    fill(out, at + url_end..at + url_end + 1, Class::Mark);
                    i = url_end + 1;
                }
                None => i += 1,
            },
            b'<' if line[i + 1..].starts_with("http://") || line[i + 1..].starts_with("https://") => {
                match line[i..].find('>') {
                    Some(k) => {
                        fill(out, at + i..at + i + k + 1, Class::Url);
                        i += k + 1;
                    }
                    None => i += 1,
                }
            }
            b'h' if (line[i..].starts_with("http://") || line[i..].starts_with("https://"))
                && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) =>
            {
                let len = line[i..]
                    .find(|c: char| c.is_whitespace() || c == ')' || c == '>')
                    .unwrap_or(n - i);
                fill(out, at + i..at + i + len, Class::Url);
                i += len;
            }
            b'#' if (i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t' || b[i - 1] == b'(')
                && line[i + 1..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_') =>
            {
                let len = 1 + line[i + 1..]
                    .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '.')))
                    .unwrap_or(n - i - 1);
                // a trailing dot ends a sentence, not the tag
                let len = len - usize::from(line[i..i + len].ends_with('.'));
                fill(out, at + i..at + i + len, Class::Tag);
                i += len;
            }
            b'*' | b'_' | b'~' => {
                let run = b[i..].iter().take_while(|x| **x == c).count();
                let prev = if i == 0 { b' ' } else { b[i - 1] };
                let next = if i + run < n { b[i + run] } else { b' ' };
                // _ inside a word (snake_case) isn't emphasis
                let intraword = c == b'_' && prev.is_ascii_alphanumeric() && next.is_ascii_alphanumeric();
                if c == b'~' && run != 2 || intraword {
                    i += run;
                    continue;
                }
                let can_close = !prev.is_ascii_whitespace();
                let can_open = !next.is_ascii_whitespace();
                let opener = open.iter().rposition(|(oc, orun, _)| *oc == c && (*orun == run || run >= 2 && *orun >= 2));
                if can_close && let Some(k) = opener {
                    let (_, orun, after) = open[k];
                    let used = orun.min(run);
                    let class = match (c, used) {
                        (b'~', _) => Class::Strike,
                        (_, 1) => Class::Emph,
                        _ => Class::Strong,
                    };
                    for x in &mut out[at + after..at + i] {
                        // inner marks and code stay what they are
                        if matches!(*x, Class::Plain | Class::Quote | Class::Heading) || (class == Class::Strong && *x == Class::Emph) {
                            *x = class;
                        }
                    }
                    fill(out, at + after - used..at + after, Class::Mark);
                    fill(out, at + i..at + i + used, Class::Mark);
                    open.truncate(k);
                    i += run;
                } else {
                    if can_open {
                        open.push((c, run, i + run));
                    }
                    i += run;
                }
            }
            _ => {
                // step over a whole character
                i += line[i..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
}

/// for a `[` at `i`: where its `](` is and where the `)` closing the url is
fn link_end(line: &str, i: usize) -> Option<(usize, usize)> {
    let b = line.as_bytes();
    let mut depth = 0;
    let mut j = i;
    while j < b.len() {
        match b[j] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= b.len() || b.get(j + 1) != Some(&b'(') {
        return None;
    }
    let close = line[j + 2..].find(')')?;
    Some((j, j + 2 + close))
}

// ------------------------------------------------------------------ the overlay

/// the colours of the classes, from the palette: chosen to read on the writing pad
pub fn class_colors(p: &Palette) -> [Color; 15] {
    let pad = p.get(Role::Pad);
    let text = p.get(Role::Text);
    let dim = p.get(Role::Dim);
    let link = p.get(Role::Link);
    // hues the palette doesn't name, turned from its link colour so they suit any preset
    let turn = |deg: f32, sat: f32| {
        let h = Hsla::from(link);
        Color::from(Hsla::new((h.hue + deg).rem_euclid(360.0), (h.saturation * sat).clamp(0.25, 0.75), h.lightness, 1.0))
    };
    let heading = readable(link, pad, 5.0);
    let code = readable(turn(140.0, 0.8), pad, 4.2);
    let url = readable(turn(190.0, 0.6), pad, 3.0);
    let accent = readable(p.get(Role::Accent), pad, 3.0);
    let mark = readable(dim.mix(&pad, 0.2), pad, 2.2);
    Class::ALL.map(|c| match c {
        Class::Plain | Class::Strong => text,
        Class::Mark => mark,
        Class::Heading => heading,
        Class::Emph => readable(text.mix(&heading, 0.4), pad, 5.0),
        Class::Strike => readable(dim, pad, 2.6),
        Class::Code => code,
        Class::Link => readable(turn(200.0, 1.0), pad, 4.5),
        Class::Url => url,
        Class::List | Class::Table => accent,
        Class::Quote => readable(dim, pad, 3.4),
        Class::Tag => readable(turn(-40.0, 1.0), pad, 4.0),
        Class::Rule => mark,
        Class::Compose => heading,
    })
}

fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (crate::palette::brightness(a), crate::palette::brightness(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `c`, darkened or lightened (away from `bg`) until it stands out from it by `ratio`
pub fn readable(c: Color, bg: Color, ratio: f32) -> Color {
    let darker = crate::palette::brightness(bg) > 0.18;
    let mut c = c.with_alpha(1.0);
    for _ in 0..40 {
        if contrast(c, bg) >= ratio {
            break;
        }
        c = if darker { c.darker(0.025) } else { c.lighter(0.025) };
    }
    c
}

/// the text node over the pad that paints its glyphs
#[derive(Component, Default, Clone)]
pub struct PadOverlay;

/// one of the overlay's spans, the colour of a class
#[derive(Component, Clone, Copy)]
pub struct ClassSpan(pub Class);

/// fills the overlay with a span for every class
pub fn setup_overlay(
    overlay: Single<Entity, (With<PadOverlay>, Without<Children>)>,
    palette: Res<Palette>,
    mut commands: Commands,
) {
    let colors = class_colors(&palette);
    for c in Class::ALL {
        commands.spawn((
            TextSpan::new(" "),
            TextColor(colors[c as usize]),
            ClassSpan(c),
            ChildOf(*overlay),
        ));
    }
}

/// what was worked out for the pad's current text
#[derive(Default)]
pub struct HighlightCache {
    /// each glyph's class
    glyphs: Vec<Class>,
    /// the glyphs couldn't be matched to the text
    failed: bool,
}

/// copies the pad's glyphs into the overlay, coloured by class
#[allow(clippy::type_complexity)]
pub fn paint_overlay(
    pad: Single<
        (
            Ref<EditableText>,
            Ref<EditableTextGeneration>,
            Ref<TextLayoutInfo>,
            &mut TextColor,
            &mut TextCursorStyle,
        ),
        (With<EditorPad>, Without<PadOverlay>),
    >,
    overlay: Single<
        (Mut<TextLayoutInfo>, &ComputedTextBlock, &mut Visibility),
        (With<PadOverlay>, Without<EditorPad>),
    >,
    spans: Query<&ClassSpan>,
    mut span_colors: Query<(&ClassSpan, &mut TextColor), Without<EditorPad>>,
    palette: Res<Palette>,
    config: Res<crate::config::Config>,
    mut cache: Local<HighlightCache>,
) {
    let (editable, generation, pad_info, mut pad_color, mut cursor) = pad.into_inner();
    let (mut info, block, mut visibility) = overlay.into_inner();

    if palette.is_changed() {
        // the cursor and the selection follow the palette too
        let accent = palette.get(Role::Accent);
        cursor.color = palette.get(Role::Text);
        cursor.selection_color = accent.with_alpha(0.45);
        cursor.unfocused_selection_color = accent.with_alpha(0.25);
        let colors = class_colors(&palette);
        for (s, mut c) in &mut span_colors {
            c.0 = colors[s.0 as usize];
        }
    }
    let ink = palette.get(Role::Text);

    // the text's classes, once per layout
    if generation.is_changed() || cache.glyphs.is_empty() {
        let Some(layout) = editable.editor().try_layout() else {
            return;
        };
        let text = editable.editor().raw_text();
        let mut classes = classify(text);
        if let Some(r) = editable.editor().raw_compose() {
            fill(&mut classes, r.clone(), Class::Compose);
        }
        cache.glyphs.clear();
        for line in layout.lines() {
            for run in line.runs() {
                for cluster in run.visual_clusters() {
                    let class = classes.get(cluster.text_range().start).copied().unwrap_or(Class::Plain);
                    for _ in cluster.glyphs() {
                        cache.glyphs.push(class);
                    }
                }
            }
        }
        cache.failed = false;
    }

    let mismatch = cache.glyphs.len() != pad_info.glyphs.len();
    let on = config.highlight && !mismatch;
    // the pad paints its own text when the overlay can't
    let want = if on { ink.with_alpha(0.0) } else { ink };
    if pad_color.0 != want {
        pad_color.0 = want;
    }
    let vis = if on { Visibility::Inherited } else { Visibility::Hidden };
    if *visibility != vis {
        *visibility = vis;
    }
    if !on {
        if mismatch && !cache.failed {
            cache.failed = true;
            warn!(
                "highlighting is off for this text: {} glyphs laid out, {} matched",
                pad_info.glyphs.len(),
                cache.glyphs.len()
            );
        }
        return;
    }
    if !(pad_info.is_changed() || editable.is_changed() || info.is_changed() || palette.is_changed() || config.is_changed()) {
        return;
    }
    // each class's section in the overlay's block
    let mut section = [0u32; 15];
    for (i, e) in block.entities().iter().enumerate() {
        if let Ok(s) = spans.get(e.entity) {
            section[s.0 as usize] = i as u32;
        }
    }
    let offset = editable.viewport.offset;
    let info = info.as_mut();
    info.scale_factor = pad_info.scale_factor;
    info.size = pad_info.size;
    info.glyphs.clear();
    // only what can be seen: a long note scrolls by without drawing all of itself
    let (top, bottom) = (-64.0, editable.viewport.size.y + 64.0);
    info.glyphs.extend(
        pad_info
            .glyphs
            .iter()
            .zip(&cache.glyphs)
            .filter(|(g, _)| {
                let y = g.position.y - offset.y;
                y > top && y < bottom
            })
            .map(|(g, c)| {
                let mut g = g.clone();
                g.position -= offset;
                g.section_index = section[*c as usize];
                g
            }),
    );
    info.run_geometry.clear();
    info.selection_rects.clear();
    info.cursor = None;
}

/// the overlay's node: over the pad's content box, clipping what scrolls out of it
pub fn overlay_scene() -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            padding: UiRect::all(px(10)),
            overflow: Overflow::clip(),
            overflow_clip_margin: OverflowClipMargin::content_box(),
        }
        Pickable::IGNORE
        crate::palette::OwnColors
        Children [
            Text("")
            Node { width: percent(100), height: percent(100) }
            Pickable::IGNORE
            PadOverlay
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Class::*;

    /// the classes of `text` as a string of letters, one per byte
    fn show(text: &str) -> String {
        classify(text)
            .iter()
            .map(|c| match c {
                Plain => '.',
                Mark => 'm',
                Heading => 'H',
                Strong => 'S',
                Emph => 'E',
                Strike => 'x',
                Code => 'c',
                Link => 'L',
                Url => 'u',
                List => 'l',
                Quote => 'q',
                Table => 't',
                Tag => 'g',
                Rule => 'r',
                Compose => 'C',
            })
            .collect()
    }

    #[test]
    fn headings_and_emphasis() {
        assert_eq!(show("# Title"), "mHHHHHH");
        assert_eq!(show("#tag here"), "gggg.....");
        assert_eq!(show("a **b** c"), "..mmSmm..");
        assert_eq!(show("*i* _j_"), "mEm.mEm");
        assert_eq!(show("snake_case_name"), "...............");
        assert_eq!(show("~~no~~"), "mmxxmm");
        assert_eq!(show("2 * 3 * 4"), ".........");
    }

    #[test]
    fn code_links_and_tags() {
        assert_eq!(show("a `b*c*` d"), "..mccccm..");
        assert_eq!(show("[t](u)"), "mLmmum");
        assert_eq!(show("see #rust."), "....ggggg.");
        assert_eq!(show("x https://a.b y"), "..uuuuuuuuuuu..");
        assert_eq!(show("```\nlet *a*\n```\nz"), "mmm.ccccccc.mmm..");
    }

    #[test]
    fn blocks() {
        assert_eq!(show("- item"), "l.....");
        assert_eq!(show("12. x"), "lll..");
        assert_eq!(show("> q *e*"), "q.qqmEm");
        assert_eq!(show("|a|b|\n|-|-|"), "t.t.t.ttttt");
        assert_eq!(show("---"), "rrr");
        assert_eq!(show("    code"), "cccccccc");
    }

    #[test]
    fn every_byte_gets_a_class() {
        for t in ["", "é **ü** `ß`", "[unclosed", "**", "` `", "# ", "> ", "|", "\n\n", "- [x] done"] {
            assert_eq!(classify(t).len(), t.len());
        }
    }
}
