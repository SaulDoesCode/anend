//! What kind of text an entry is, told by its pattern: a link, an email address, a
//! number, a date, a time, JSON, a hashtag, a word, a name, a phrase, a sentence, or a
//! longer text. Shown with an entry (`entries.info`); a wrong guess is fixed by hand and
//! kept in the save (`entries.set_kind`).

/// every kind, in the order they're offered
pub const KINDS: [&str; 12] = [
    "link", "email", "number", "date", "time", "json", "hashtag", "word", "name", "phrase", "sentence", "text",
];

fn is_date(t: &str) -> bool {
    // 2026-10-19, 19/10/2026, 10.19.26, 2026/10/19
    let parts: Vec<&str> = t.split(['-', '/', '.']).collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit()))
        && parts.iter().any(|p| p.len() == 4 || p.len() == 2)
        && t.chars()
            .filter(|c| !c.is_ascii_digit())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == 1
}

fn is_time(t: &str) -> bool {
    let t = t
        .strip_suffix("am")
        .or_else(|| t.strip_suffix("pm"))
        .map_or(t, str::trim_end);
    let parts: Vec<&str> = t.split(':').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| (1..=2).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
}

fn is_number(t: &str) -> bool {
    let t = t.strip_prefix(['-', '+']).unwrap_or(t);
    let digits = t.replace([',', '_', ' '], "");
    let mut dots = 0;
    !digits.is_empty()
        && digits.bytes().any(|b| b.is_ascii_digit())
        && digits.bytes().all(|b| {
            dots += (b == b'.') as usize;
            b.is_ascii_digit() || b == b'.'
        })
        && dots <= 1
}

fn is_email(t: &str) -> bool {
    let Some((user, host)) = t.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && !t.contains(char::is_whitespace)
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains('@')
}

fn is_link(t: &str) -> bool {
    let l = t.to_ascii_lowercase();
    !t.contains(char::is_whitespace)
        && (l.starts_with("http://")
            || l.starts_with("https://")
            || l.starts_with("www.")
            || l.starts_with("ftp://")
            || l.starts_with("mailto:"))
}

/// the kind a text looks like
pub fn detect(text: &str) -> &'static str {
    let t = text.trim();
    if t.is_empty() {
        return "text";
    }
    if is_link(t) {
        return "link";
    }
    if is_email(t) {
        return "email";
    }
    if is_date(t) {
        return "date";
    }
    if is_time(&t.to_ascii_lowercase()) {
        return "time";
    }
    if is_number(t) {
        return "number";
    }
    if ((t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']')))
        && serde_json::from_str::<serde_json::Value>(t).is_ok()
    {
        return "json";
    }
    if t.starts_with('#') && t.len() > 1 && !t.contains(char::is_whitespace) {
        return "hashtag";
    }
    if t.contains('\n') || t.chars().count() > 200 {
        return "text";
    }
    let words: Vec<&str> = t.split_whitespace().collect();
    if t.ends_with(['.', '!', '?']) && words.len() > 1 {
        return "sentence";
    }
    let capital = |w: &str| w.chars().next().is_some_and(char::is_uppercase);
    if words.len() <= 4 && words.iter().all(|w| capital(w)) && words.iter().any(|w| w.chars().count() > 1) {
        return "name";
    }
    if words.len() == 1 {
        return "word";
    }
    if words.len() <= 12 { "phrase" } else { "text" }
}

/// each word of a text and its kind (a word read on its own: `word`, `name` when it has a
/// capital, `number`, `date`, `time`, `link`, `email`, `hashtag`); punctuation around a
/// word is left out
pub fn word_types(text: &str) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for raw in text.split_whitespace().take(400) {
        let keep = is_link(raw) || is_email(raw);
        let w = if keep {
            raw.trim_end_matches(['.', ',', ';', ')', '!', '?'])
        } else {
            raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '#' && c != '-' && c != '+' && c != '\'')
                .trim_end_matches(['.', '-', '\''])
        };
        if w.is_empty() {
            continue;
        }
        let k = match detect(w) {
            "phrase" | "sentence" | "text" | "json" => "word",
            k => k,
        };
        out.push((w.to_string(), k));
    }
    out
}

/// a text's word types in a line, for the hover line: how many words of each type, with
/// a few of each type other than plain words
/// (`4 words: 2 word · 1 name (Paris) · 1 number (12)`)
pub fn describe_words(text: &str) -> String {
    let ws = word_types(text);
    if ws.is_empty() {
        return String::new();
    }
    let mut by: Vec<(&'static str, Vec<&str>)> = Vec::new();
    for (w, k) in &ws {
        match by.iter_mut().find(|(t, _)| t == k) {
            Some((_, v)) => v.push(w),
            None => by.push((k, vec![w])),
        }
    }
    by.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    let parts: Vec<String> = by
        .iter()
        .map(|(k, v)| {
            if *k == "word" {
                format!("{} {k}", v.len())
            } else {
                let mut shown: Vec<&str> = Vec::new();
                for w in v {
                    if !shown.contains(w) && shown.len() < 3 {
                        shown.push(w);
                    }
                }
                format!("{} {k} ({})", v.len(), shown.join(", "))
            }
        })
        .collect();
    format!(
        "{} word{}: {}",
        ws.len(),
        if ws.len() == 1 { "" } else { "s" },
        parts.join(" · ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_have_types() {
        let t = word_types("Met Ann in Paris at 10:30 on 2026-10-19, paid 12.50 (see https://x.org). #trip");
        let k = |w: &str| t.iter().find(|(x, _)| x == w).map(|x| x.1);
        assert_eq!(k("Met"), Some("name"));
        assert_eq!(k("in"), Some("word"));
        assert_eq!(k("10:30"), Some("time"));
        assert_eq!(k("2026-10-19"), Some("date"));
        assert_eq!(k("12.50"), Some("number"));
        assert_eq!(k("https://x.org"), Some("link"));
        assert_eq!(k("#trip"), Some("hashtag"));
        assert_eq!(
            describe_words("apples and 3 Pears"),
            "4 words: 2 word · 1 name (Pears) · 1 number (3)"
        );
        assert_eq!(describe_words("  "), "");
    }

    #[test]
    fn kinds_by_pattern() {
        for (t, k) in [
            ("https://example.org/a?b=1", "link"),
            ("www.anend.org", "link"),
            ("ann@example.org", "email"),
            ("2026-10-19", "date"),
            ("19/10/2026", "date"),
            ("12:30", "time"),
            ("9:05 pm", "time"),
            ("3.14", "number"),
            ("-1,000,000", "number"),
            ("{\"a\": 1}", "json"),
            ("[1, 2]", "json"),
            ("[not json", "phrase"),
            ("#gematria", "hashtag"),
            ("plum", "word"),
            ("Herman Melville", "name"),
            ("New York", "name"),
            ("ripe plums in june", "phrase"),
            ("The plum is ripe.", "sentence"),
            ("a\nb", "text"),
            ("1.2.3.4.5", "word"),
            ("", "text"),
        ] {
            assert_eq!(detect(t), k, "{t:?}");
            assert!(KINDS.contains(&k));
        }
    }
}
