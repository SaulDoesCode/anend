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

#[cfg(test)]
mod tests {
    use super::*;

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
