//! The note model. A note is what the anend server called a "writ": a markdown document
//! with a title, tags, a creation date and an edit history.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Note {
    #[serde(rename = "_key", default)]
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub markdown: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// RFC3339 creation time
    #[serde(default)]
    pub created: String,
    /// RFC3339 times of every save after the first
    #[serde(default)]
    pub edits: Vec<String>,
}

impl Note {
    /// the text shown in the editor: a `# title` line followed by the body
    pub fn editor_text(&self) -> String {
        compose(&self.title, &self.markdown)
    }

    pub fn created_at(&self) -> Option<DateTime<Local>> {
        parse_time(&self.created)
    }

    pub fn modified_at(&self) -> Option<DateTime<Local>> {
        self.edits
            .last()
            .and_then(|e| parse_time(e))
            .or_else(|| self.created_at())
    }

    pub fn display_title(&self) -> &str {
        if self.title.is_empty() { "untitled" } else { &self.title }
    }
}

pub fn now() -> String {
    Local::now().to_rfc3339()
}

pub fn parse_time(s: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Local))
}

/// the reader's date format, dayjs `DD MMMM YYYY | HH:mm`
pub fn long_date(t: &DateTime<Local>) -> String {
    t.format("%d %B %Y | %H:%M").to_string()
}

/// a rough "3 days ago", like dayjs' fromNow
pub fn relative(t: &DateTime<Local>) -> String {
    let secs = (Local::now() - *t).num_seconds();
    let (n, unit) = match secs {
        s if s < 45 => return "a few seconds ago".into(),
        s if s < 90 => return "a minute ago".into(),
        s if s < 45 * 60 => ((s + 30) / 60, "minutes"),
        s if s < 90 * 60 => return "an hour ago".into(),
        s if s < 22 * 3600 => ((s + 1800) / 3600, "hours"),
        s if s < 36 * 3600 => return "a day ago".into(),
        s if s < 26 * 86400 => ((s + 43200) / 86400, "days"),
        s if s < 46 * 86400 => return "a month ago".into(),
        s if s < 320 * 86400 => ((s + 15 * 86400) / (30 * 86400), "months"),
        s if s < 548 * 86400 => return "a year ago".into(),
        s => ((s + 182 * 86400) / (365 * 86400), "years"),
    };
    format!("{} {} ago", n, unit)
}

/// joins a title and a body into editor text
pub fn compose(title: &str, markdown: &str) -> String {
    match (title.is_empty(), markdown.is_empty()) {
        (true, _) => markdown.to_string(),
        (false, true) => format!("# {}\n\n", title),
        (false, false) => format!("# {}\n\n{}", title, markdown),
    }
}

/// splits editor text into its title (the first `# heading` line) and the remaining
/// markdown, the way the old markdown-editor component did
pub fn split(text: &str) -> (String, String) {
    let mut title = String::new();
    let mut title_line = None;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with('#') {
            title = t.trim_start_matches('#').trim().to_string();
            title_line = Some(i);
            break;
        }
    }
    let body: Vec<&str> = text
        .lines()
        .enumerate()
        .filter(|(i, _)| Some(*i) != title_line)
        .map(|(_, l)| l)
        .collect();
    (title, body.join("\n").trim().to_string())
}

/// cleans up a tag typed into the tag input
pub fn normalize_tag(tag: &str) -> Option<String> {
    let t = tag.trim().trim_matches(',').trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// every tag with how many notes carry it, the most used first (then by name)
pub fn tag_counts(notes: &[Note]) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for n in notes {
        // a tag listed twice on one note counts once
        let mut seen: Vec<&str> = Vec::new();
        for t in &n.tags {
            if !seen.contains(&t.as_str()) {
                seen.push(t);
                *counts.entry(t).or_default() += 1;
            }
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().map(|(t, c)| (t.to_string(), c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())).then(a.0.cmp(&b.0)));
    v
}

/// whether a note carries all (or, with `all` false, any) of `tags`; no tags match nothing
pub fn has_tags(note: &Note, tags: &[String], all: bool) -> bool {
    if tags.is_empty() {
        return false;
    }
    if all {
        tags.iter().all(|t| note.tags.contains(t))
    } else {
        tags.iter().any(|t| note.tags.contains(t))
    }
}

/// the start of a note's body as plain text on one line, for previews: markdown's marks
/// dropped, at most `max` characters, ending in an ellipsis when cut
pub fn preview(markdown: &str, max: usize) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in markdown.lines() {
        let l = line.trim();
        if l.starts_with("```") || l.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if l.is_empty() || (!in_code && l.chars().all(|c| matches!(c, '-' | '|' | ':' | ' ' | '*' | '_' | '='))) {
            continue;
        }
        let l = if in_code {
            l
        } else {
            l.trim_start_matches(['#', '>', ' '])
                .trim_start_matches("- ")
                .trim_start_matches("* ")
                .trim_start_matches("+ ")
        };
        let cleaned: String = if in_code {
            l.to_string()
        } else {
            // links keep their text: [text](url) -> text
            let mut s = String::new();
            let mut chars = l.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '*' | '_' | '`' | '[' => {}
                    '|' => s.push(' '),
                    ']' if chars.peek() == Some(&'(') => {
                        for c in chars.by_ref() {
                            if c == ')' {
                                break;
                            }
                        }
                    }
                    ']' => {}
                    c => s.push(c),
                }
            }
            s.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        if cleaned.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&cleaned);
        if out.chars().count() > max {
            break;
        }
    }
    if out.chars().count() > max {
        let cut: String = out.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_compose_roundtrip() {
        let (t, b) = split("# Hello there\n\nsome *text*\n\n## and more");
        assert_eq!(t, "Hello there");
        assert_eq!(b, "some *text*\n\n## and more");
        assert_eq!(split(&compose(&t, &b)), (t, b));
    }

    #[test]
    fn split_without_title() {
        assert_eq!(split("just text"), (String::new(), "just text".into()));
        assert_eq!(split("intro\n# Title\nbody"), ("Title".into(), "intro\nbody".into()));
        assert_eq!(split("#Tight"), ("Tight".into(), String::new()));
    }

    #[test]
    fn compose_variants() {
        assert_eq!(compose("", "x"), "x");
        assert_eq!(compose("T", ""), "# T\n\n");
    }

    #[test]
    fn old_writ_documents_load() {
        let writ = r#"{"_key":"1000001","title":"Old","markdown":"hi","tags":["a"],
            "created":"2018-11-16T20:04:05.12+02:00","public":true,"views":3,"author":"saul"}"#;
        let n: Note = serde_json::from_str(writ).unwrap();
        assert_eq!(n.created_at().unwrap().timestamp(), 1542391445);
        assert_eq!(n.display_title(), "Old");
    }

    #[test]
    fn tags() {
        assert_eq!(normalize_tag("  rust, "), Some("rust".into()));
        assert_eq!(normalize_tag(" , "), None);
    }

    #[test]
    fn tag_counts_and_matching() {
        let n = |tags: &[&str]| Note {
            tags: tags.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        };
        let notes = vec![n(&["rust", "meta"]), n(&["rust"]), n(&["Art", "rust", "rust"]), n(&[])];
        assert_eq!(
            tag_counts(&notes),
            vec![("rust".into(), 3), ("Art".into(), 1), ("meta".into(), 1)]
        );
        let sel = vec!["rust".to_string(), "meta".to_string()];
        assert_eq!(notes.iter().filter(|x| has_tags(x, &sel, true)).count(), 1);
        assert_eq!(notes.iter().filter(|x| has_tags(x, &sel, false)).count(), 3);
        assert!(!has_tags(&notes[0], &[], false));
    }

    #[test]
    fn previews() {
        assert_eq!(
            preview("## Head\n\nSome **bold** and a [link](http://x.y).\n\n- item\n\n|a|b|\n|---|---|", 200),
            "Head Some bold and a link. item a b"
        );
        assert_eq!(preview("abcdefghij", 5), "abcd…");
        assert_eq!(preview("", 5), "");
    }
}
