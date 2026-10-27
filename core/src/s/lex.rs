//! s's tokens (docs/s.md §2): brace literals (nesting, with `~name~` interpolation at
//! depth 1), quote literals (raw), and words. Characters right before a `{` or `'` are a
//! prefix of that literal; characters right after its end start a new token.

/// a piece of a brace literal: text, or a name whose value goes in when it's evaluated
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    Var(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// `{...}`: its parts (a prefix written before the brace is the first text)
    Brace(Vec<Part>),
    /// `'...'`, raw (with its prefix)
    Quote(String),
    Word(String),
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\n' | '\t' | '\r')
}

/// a brace literal's inside (between the outer braces) as parts: `~name~` at depth 1 is a
/// variable, `~~` a `~`, anything else literal; text at depth ≥ 2 is raw
fn brace_parts(prefix: &str, inner: &[char]) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut text = String::from(prefix);
    let mut depth = 0usize;
    let mut i = 0;
    while i < inner.len() {
        let c = inner[i];
        match c {
            '{' => {
                depth += 1;
                text.push(c);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                text.push(c);
            }
            '~' if depth == 0 => {
                if inner.get(i + 1) == Some(&'~') {
                    text.push('~');
                    i += 2;
                    continue;
                }
                // a name: non-empty, no whitespace, braces or tildes, closed by `~`
                let mut j = i + 1;
                while j < inner.len() && !is_space(inner[j]) && !matches!(inner[j], '{' | '}' | '~') {
                    j += 1;
                }
                if j > i + 1 && inner.get(j) == Some(&'~') {
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text)));
                    }
                    parts.push(Part::Var(inner[i + 1..j].iter().collect()));
                    i = j + 1;
                    continue;
                }
                text.push('~');
            }
            _ => text.push(c),
        }
        i += 1;
    }
    if !text.is_empty() || parts.is_empty() {
        parts.push(Part::Text(text));
    }
    parts
}

/// a unit's tokens, or why it doesn't parse
pub fn lex(src: &str) -> Result<Vec<Tok>, String> {
    Ok(lex_spans(src)?.into_iter().map(|(t, _)| t).collect())
}

/// a unit's tokens with where each is in `src` (byte offsets, a literal's prefix included):
/// for the code editor's outline, inline results and stepping
pub fn lex_spans(src: &str) -> Result<Vec<(Tok, std::ops::Range<usize>)>, String> {
    let cs: Vec<char> = src.chars().collect();
    // a char's index → its byte offset (and the end)
    let mut byte = Vec::with_capacity(cs.len() + 1);
    let mut b = 0;
    for c in &cs {
        byte.push(b);
        b += c.len_utf8();
    }
    byte.push(b);
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        if is_space(cs[i]) {
            i += 1;
            continue;
        }
        // a word, up to whitespace or the start of a literal
        let start = i;
        while i < cs.len() && !is_space(cs[i]) && cs[i] != '{' && cs[i] != '\'' {
            i += 1;
        }
        let word: String = cs[start..i].iter().collect();
        if i < cs.len() && (cs[i] == '{' || cs[i] == '\'') {
            // the word is the literal's prefix
            if cs[i] == '\'' {
                let close = cs[i + 1..].iter().position(|&c| c == '\'').ok_or("an unclosed quote")?;
                let body: String = cs[i + 1..i + 1 + close].iter().collect();
                out.push((Tok::Quote(format!("{word}{body}")), byte[start]..byte[i + close + 2]));
                i += close + 2;
            } else {
                let mut depth = 0usize;
                let mut j = i;
                loop {
                    if j >= cs.len() {
                        return Err("an unclosed brace".into());
                    }
                    match cs[j] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                out.push((Tok::Brace(brace_parts(&word, &cs[i + 1..j])), byte[start]..byte[j + 1]));
                i = j + 1;
            }
            continue;
        }
        out.push((Tok::Word(word), byte[start]..byte[i]));
    }
    Ok(out)
}

/// a brace literal's text as written (its parts with variables shown as `~name~`), for
/// code that wants the literal as source
pub fn brace_source(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => t.clone(),
            Part::Var(v) => format!("~{v}~"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Tok {
        Tok::Word(s.into())
    }

    #[test]
    fn tokens() {
        assert_eq!(lex("a  b\tc\n").unwrap(), vec![w("a"), w("b"), w("c")]);
        assert_eq!(
            lex("pre{fix}x").unwrap(),
            vec![Tok::Brace(vec![Part::Text("prefix".into())]), w("x")]
        );
        assert_eq!(
            lex("{a {b ~c~} ~d~ ~~ ~ e}").unwrap(),
            vec![Tok::Brace(vec![
                Part::Text("a {b ~c~} ".into()),
                Part::Var("d".into()),
                Part::Text(" ~ ~ e".into()),
            ])]
        );
        assert_eq!(lex("'a {b' c").unwrap(), vec![Tok::Quote("a {b".into()), w("c")]);
        assert_eq!(lex("{}").unwrap(), vec![Tok::Brace(vec![Part::Text(String::new())])]);
        assert!(lex("{a").is_err());
        assert!(lex("'a").is_err());
        assert_eq!(lex("x'y'z").unwrap(), vec![Tok::Quote("xy".into()), w("z")]);
        let sp: Vec<_> = lex_spans("é {a b} c'd'").unwrap().into_iter().map(|t| t.1).collect();
        assert_eq!(sp, [0..2, 3..8, 9..13]);
    }
}
