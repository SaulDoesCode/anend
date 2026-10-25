//! A long text cut for showing, piece by piece (docs/format.md "Long texts", answer 7):
//! lines of at most `width` characters broken at word boundaries (a word longer than a
//! line is broken), and pieces of at most `lines` such lines. The pieces are slices of
//! the text, cut only between lines, so joined they are the text exactly; nothing is
//! dropped or changed. Both numbers are settings (80 and 6 by default). These visual
//! pieces have nothing to do with how a text is stored.

/// the defaults: characters a line, lines a piece
pub const LINE: usize = 80;
pub const LINES: usize = 6;

/// where each visual line of `text` starts (byte offsets), at most `width` characters a
/// line, breaking after the spaces between words, at newlines, and inside a word only
/// when it is longer than a line
pub fn line_starts(text: &str, width: usize) -> Vec<usize> {
    let width = width.max(1);
    let mut starts = vec![0];
    let mut line_start = 0;
    let mut chars_in_line = 0;
    // the last place a line may break (after a space), and the characters up to it
    let mut last_break: Option<(usize, usize)> = None;
    for (i, c) in text.char_indices() {
        if c == '\n' {
            let next = i + 1;
            if next < text.len() {
                starts.push(next);
            }
            line_start = next;
            chars_in_line = 0;
            last_break = None;
            continue;
        }
        if chars_in_line >= width && !c.is_whitespace() {
            // full: break after the last space, else inside the word
            let at = match last_break {
                Some((at, _)) if at > line_start => at,
                _ => i,
            };
            starts.push(at);
            chars_in_line = text[at..i].chars().count();
            line_start = at;
            last_break = None;
        }
        chars_in_line += 1;
        if c == ' ' || c == '\t' {
            last_break = Some((i + c.len_utf8(), chars_in_line));
        }
    }
    starts
}

/// the text in pieces of at most `lines` visual lines of at most `width` characters;
/// joined, they are the text
pub fn pieces(text: &str, width: usize, lines: usize) -> Vec<&str> {
    let starts = line_starts(text, width);
    let lines = lines.max(1);
    let mut out = Vec::new();
    let mut i = 0;
    while i < starts.len() {
        let a = starts[i];
        let b = starts.get(i + lines).copied().unwrap_or(text.len());
        out.push(&text[a..b]);
        i += lines;
    }
    if out.is_empty() {
        out.push(text);
    }
    out
}

/// how many pieces a text has, without making them
pub fn count(text: &str, width: usize, lines: usize) -> usize {
    line_starts(text, width).len().div_ceil(lines.max(1)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(text: &str, width: usize) -> Vec<&str> {
        let s = line_starts(text, width);
        s.iter()
            .enumerate()
            .map(|(i, &a)| &text[a..s.get(i + 1).copied().unwrap_or(text.len())])
            .collect()
    }

    #[test]
    fn lines_break_at_words_and_never_run_long() {
        let t = "the quick brown fox jumps over the lazy dog";
        assert_eq!(
            lines_of(t, 10),
            ["the quick ", "brown fox ", "jumps over ", "the lazy ", "dog"]
        );
        // a word longer than a line is broken
        assert_eq!(lines_of("abcdefghijkl mn", 5), ["abcde", "fghij", "kl mn"]);
        // newlines are kept as breaks; an empty text is one empty line
        assert_eq!(lines_of("a\n\nb", 80), ["a\n", "\n", "b"]);
        assert_eq!(line_starts("", 80), [0]);
        for (t, w) in [(t, 10), ("ẞeta αβγ δεζ ηθι κλμ", 4), ("x  y   z", 2)] {
            for l in lines_of(t, w) {
                assert!(l.trim_end().chars().count() <= w, "{l:?} in {w}");
            }
        }
    }

    #[test]
    fn pieces_join_to_the_text() {
        let long: String = (0..400).map(|i| format!("word{i} ")).collect::<Vec<_>>().join("") + "\nend";
        for (w, l) in [(80, 6), (10, 1), (1, 1), (200, 50)] {
            let ps = pieces(&long, w, l);
            assert_eq!(ps.concat(), long);
            assert_eq!(ps.len(), count(&long, w, l));
            for p in &ps {
                assert!(line_starts(p, w).len() <= l);
            }
        }
        assert_eq!(pieces("short", 80, 6), ["short"]);
        assert_eq!(pieces("", 80, 6), [""]);
    }
}
