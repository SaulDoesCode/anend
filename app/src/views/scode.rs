//! The notes view as s's code editor (docs/s.md; queued requests #11, #12): what it knows of
//! a program's text. An outline of what the program defines (scripts, `name$`; words,
//! `value word ><`) and the other programs it calls (`name.`), the word at a place (for
//! go to definition), and the words to complete a prefix with.

use anend_core::s::lex::{Tok, lex_spans};

/// one thing in the outline: its name, what it is, and where (a byte offset in the code)
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    /// `script` (`name$`), `word` (`value word ><`) or `call` (`name.` of another program)
    pub kind: &'static str,
    pub at: usize,
}

/// a word without the suffix that makes it a form: `loop.` and `loop..` are `loop`
pub fn stem(w: &str) -> &str {
    let w = w.strip_suffix("..").unwrap_or(w);
    if w.chars().count() >= 2
        && let Some(last) = w.chars().last()
        && anend_core::s::machine::SUFFIXES.contains(&last)
    {
        return &w[..w.len() - last.len_utf8()];
    }
    w
}

/// what the program defines, in order, and the programs it calls (by `titles`)
pub fn outline(code: &str, titles: &[String]) -> Vec<Item> {
    let Ok(toks) = lex_spans(code) else { return vec![] };
    let mut out: Vec<Item> = Vec::new();
    for (i, (t, r)) in toks.iter().enumerate() {
        let Tok::Word(w) = t else { continue };
        if w == "><" {
            // the word defined is the token before
            let name = match i.checked_sub(1).map(|j| &toks[j].0) {
                Some(Tok::Word(x)) | Some(Tok::Quote(x)) => x.clone(),
                _ => continue,
            };
            out.push(Item {
                name,
                kind: "word",
                at: toks[i - 1].1.start,
            });
        } else if w.len() >= 2 && w.ends_with('$') && !w.ends_with("$$") {
            out.push(Item {
                name: w[..w.len() - 1].to_string(),
                kind: "script",
                at: r.start,
            });
        } else if (w.ends_with('.') || w.ends_with("..")) && w.len() >= 2 {
            let s = stem(w);
            if titles.iter().any(|t| t == s || format!("{t}.s") == s)
                && !out.iter().any(|x| x.kind == "call" && x.name == s)
            {
                out.push(Item {
                    name: s.to_string(),
                    kind: "call",
                    at: r.start,
                });
            }
        }
    }
    out
}

/// the word around a place (a byte offset): its start, its end and its text; words run
/// between whitespace and braces or quotes
pub fn word_at(text: &str, at: usize) -> Option<(usize, usize, &str)> {
    let at = at.min(text.len());
    if !text.is_char_boundary(at) {
        return None;
    }
    let edge = |c: char| c.is_whitespace() || matches!(c, '{' | '}' | '\'');
    let a = text[..at]
        .char_indices()
        .rev()
        .find(|(_, c)| edge(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let b = text[at..]
        .char_indices()
        .find(|(_, c)| edge(*c))
        .map_or(text.len(), |(i, _)| at + i);
    (a < b).then(|| (a, b, &text[a..b]))
}

/// the word being typed before a place: its text (a prefix, empty when none)
pub fn prefix_at(text: &str, at: usize) -> &str {
    match word_at(text, at) {
        Some((a, _, _)) if a < at && text.is_char_boundary(at) => &text[a..at],
        _ => "",
    }
}

/// the words a prefix may be (shortest first, then alphabetical), not itself, at most `n`
pub fn complete(prefix: &str, words: &[String], n: usize) -> Vec<String> {
    if prefix.is_empty() || prefix.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') {
        return vec![];
    }
    let mut c: Vec<String> = words
        .iter()
        .filter(|w| w.starts_with(prefix) && w.as_str() != prefix)
        .cloned()
        .collect();
    c.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
    c.dedup();
    c.truncate(n);
    c
}

/// a byte offset's line, counted from 0
pub fn line_of(text: &str, at: usize) -> usize {
    text.as_bytes()[..at.min(text.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
}

/// where a note's body starts in its whole text: after its `# title` line and the blank
/// space after it (as `note::split` takes the body, without trimming its end)
pub fn body_offset(text: &str) -> usize {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with('#') {
            let after = at + line.len();
            let rest = &text[after..];
            return after + (rest.len() - rest.trim_start().len());
        }
        at += line.len();
    }
    // no title: the body is the text, from its first non-blank character
    text.len() - text.trim_start().len()
}

/// the bar's `Run <program> [arguments]`: the save's s program of that title, run; what it
/// wrote shown, or its error. True when the bar's text was a `Run`
pub async fn run_from_bar(ui: crate::ui::Ui, api: &crate::ui::Api, text: &str) -> bool {
    let Some(rest) = text.trim().strip_prefix("Run ") else {
        return false;
    };
    let mut words = rest.split_whitespace();
    let Some(title) = words.next().map(|t| t.strip_suffix(".s").unwrap_or(t).to_string()) else {
        return false;
    };
    let args: Vec<String> = words.map(str::to_string).collect();
    let progs = api.call("s.programs", serde_json::json!({})).await.unwrap_or_default();
    let Some(key) = progs
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["title"] == title.as_str())
        .and_then(|p| p["key"].as_str().map(str::to_string))
    else {
        ui.fail(format!("the save has no s program titled “{title}” (a note tagged s)"));
        return true;
    };
    match api.call("s.run", serde_json::json!({ "key": key, "args": args })).await {
        Ok(r) => {
            let out = r["out"].as_str().unwrap_or("").trim_start_matches('\n').to_string();
            match r["err"].as_str() {
                Some(e) => ui.fail(format!("{title}: {e}")),
                None => ui.toast(if out.trim().is_empty() {
                    format!("{title} ran ({} steps)", r["steps"])
                } else {
                    out
                }),
            }
        }
        Err(e) => ui.fail(e),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_words_and_completion() {
        let code = "{n~ 0 >> step.?} loop$\n{1 +} 'inc' ><\nworld who@ greet.\nloop..";
        let o = outline(code, &["greet".to_string()]);
        let names: Vec<(&str, &str)> = o.iter().map(|i| (i.name.as_str(), i.kind)).collect();
        assert_eq!(names, [("loop", "script"), ("inc", "word"), ("greet", "call")]);
        assert_eq!(&code[o[0].at..o[0].at + 5], "loop$");
        assert_eq!(stem("loop.."), "loop");
        assert_eq!(stem("a~"), "a");
        assert_eq!(stem(">"), ">");
        let at = code.find("greet").unwrap() + 2;
        assert_eq!(word_at(code, at).unwrap().2, "greet.");
        assert_eq!(prefix_at(code, at), "gr");
        assert_eq!(word_at("{abc}", 2).unwrap().2, "abc");
        let words: Vec<String> = ["loop", "load", "lo", "l"].iter().map(|s| s.to_string()).collect();
        assert_eq!(complete("lo", &words, 5), ["load", "loop"]);
        assert!(complete("12", &words, 5).is_empty());
        assert_eq!(line_of(code, code.find("world").unwrap()), 2);
        assert_eq!(body_offset("# t\n\nbody"), 5);
        assert_eq!(body_offset("# t\n\n{a} >\n\n"), 5);
        assert_eq!(body_offset("  x"), 2);
    }
}
