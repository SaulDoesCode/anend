//! Markdown to HTML for reading notes: raw HTML in a note is shown as text, and links and
//! pictures only go to http(s), mailto or relative addresses.

use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, html};

fn safe_url(u: &str) -> bool {
    let t: String = u.chars().filter(|c| !c.is_control() && !c.is_whitespace()).collect();
    let lower = t.to_ascii_lowercase();
    match lower.split_once(':') {
        Some((scheme, _)) if !scheme.contains('/') => matches!(scheme, "http" | "https" | "mailto"),
        _ => true,
    }
}

pub fn to_html(text: &str) -> String {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events = Parser::new_ext(text, opts).map(|e| match e {
        Event::Html(h) | Event::InlineHtml(h) => Event::Text(h),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: if safe_url(&dest_url) {
                dest_url
            } else {
                CowStr::from("#")
            },
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: if safe_url(&dest_url) {
                dest_url
            } else {
                CowStr::from("")
            },
            title,
            id,
        }),
        e => e,
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe() {
        let h = to_html("# hi\n\n<script>alert(1)</script>\n\n[x](javascript:alert(1)) [y](https://a.b) **b**");
        assert!(h.contains("<h1>hi</h1>") && h.contains("<strong>b</strong>"));
        assert!(!h.contains("<script>") && h.contains("&lt;script&gt;"));
        assert!(!h.contains("javascript:") && h.contains("href=\"https://a.b\""));
        assert!(
            to_html("[x](JaVa%20script:alert(1)) [y](JAVASCRIPT:x)")
                .matches("href=\"#\"")
                .count()
                >= 1
        );
    }
}
