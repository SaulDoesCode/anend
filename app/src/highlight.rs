//! Markdown highlighting in the note editor, ported from the Bevy app's highlight.rs: a
//! class for every byte of the text, line by line ([`classify`]), and the text as HTML
//! spans of those classes ([`to_html`]) for the layer painted under the editor's textarea,
//! whose own glyphs are transparent (so editing, the caret, selection and input methods
//! stay the textarea's). The colours come from the palette (theme.rs `hl_vars`), each
//! darkened or lightened until it reads on the writing pad. Spelling and grammar marks
//! (`Mark`s) are underlined over the same layer.

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
    if (1..=6).contains(&hashes) && indent < 4 && trimmed[hashes..].chars().next().is_none_or(|c| c == ' ' || c == '\t')
    {
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
                    let link_class = if base == Class::Heading {
                        Class::Heading
                    } else {
                        Class::Link
                    };
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
                && line[i + 1..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_') =>
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
                let opener = open
                    .iter()
                    .rposition(|(oc, orun, _)| *oc == c && (*orun == run || run >= 2 && *orun >= 2));
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
                        if matches!(*x, Class::Plain | Class::Quote | Class::Heading)
                            || (class == Class::Strong && *x == Class::Emph)
                        {
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

impl Class {
    /// the CSS class of a span of this class
    pub fn css(self) -> &'static str {
        match self {
            Class::Plain => "",
            Class::Mark => "h-m",
            Class::Heading => "h-h",
            Class::Strong => "h-s",
            Class::Emph => "h-e",
            Class::Strike => "h-x",
            Class::Code => "h-c",
            Class::Link => "h-l",
            Class::Url => "h-u",
            Class::List => "h-li",
            Class::Quote => "h-q",
            Class::Table => "h-t",
            Class::Tag => "h-g",
            Class::Rule => "h-r",
        }
    }
}

/// a stretch of the text the checker flagged: bytes `start..end`, what it is, and a title
#[derive(Clone, Debug, PartialEq)]
pub struct Flag {
    pub start: usize,
    pub end: usize,
    pub kind: &'static str,
    pub title: String,
}

fn escape(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// the text as HTML: runs of one class in a span each (plain text bare), flagged
/// stretches inside a `mark`; a trailing newline gets a space after it so the layer is as
/// tall as the textarea's content
pub fn to_html(text: &str, colour: bool, flags: &[Flag]) -> String {
    let classes = if colour {
        classify(text)
    } else {
        vec![Class::Plain; text.len()]
    };
    let mut out = String::with_capacity(text.len() * 2);
    // the boundaries: where the class changes or a flag starts or ends
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for i in 1..text.len() {
        if classes[i] != classes[i - 1] {
            cuts.push(i);
        }
    }
    for f in flags {
        cuts.push(f.start.min(text.len()));
        cuts.push(f.end.min(text.len()));
    }
    cuts.retain(|&c| text.is_char_boundary(c));
    cuts.sort_unstable();
    cuts.dedup();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a == b {
            continue;
        }
        let piece = &text[a..b];
        let flag = flags.iter().find(|f| f.start <= a && b <= f.end);
        if let Some(f) = flag {
            out.push_str("<mark class=\"flag ");
            out.push_str(f.kind);
            out.push_str("\" title=\"");
            escape(&f.title, &mut out);
            out.push_str("\">");
        }
        let css = classes[a].css();
        if css.is_empty() {
            escape(piece, &mut out);
        } else {
            out.push_str("<span class=\"");
            out.push_str(css);
            out.push_str("\">");
            escape(piece, &mut out);
            out.push_str("</span>");
        }
        if flag.is_some() {
            out.push_str("</mark>");
        }
    }
    if text.ends_with('\n') || text.is_empty() {
        out.push(' ');
    }
    out
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
    fn html() {
        assert_eq!(
            to_html("# T", true, &[]),
            "<span class=\"h-m\">#</span><span class=\"h-h\"> T</span>"
        );
        assert_eq!(to_html("a<b\n", false, &[]), "a&lt;b\n ");
        let f = Flag {
            start: 2,
            end: 5,
            kind: "spell",
            title: String::new(),
        };
        assert_eq!(
            to_html("a teh b", false, &[f]),
            "a <mark class=\"flag spell\" title=\"\">teh</mark> b"
        );
        // a flag never cuts a character in two
        let f = Flag {
            start: 1,
            end: 2,
            kind: "spell",
            title: String::new(),
        };
        assert!(to_html("é", false, &[f]).contains('é'));
    }

    #[test]
    fn every_byte_gets_a_class() {
        for t in [
            "",
            "é **ü** `ß`",
            "[unclosed",
            "**",
            "` `",
            "# ",
            "> ",
            "|",
            "\n\n",
            "- [x] done",
        ] {
            assert_eq!(classify(t).len(), t.len());
        }
    }
}
