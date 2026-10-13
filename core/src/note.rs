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
}
