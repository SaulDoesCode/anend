//! The guide (docs/guide.md) as a page of the hub, at `/guide`, and its pictures at
//! `/guide/{name}.png`. Both are built into the binary, so the page is the same static
//! text for everyone. The markdown is turned into HTML here, every text escaped and any
//! raw HTML in it shown as text, links kept to the page's own headings and to https.

use askama::Template;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use rust_embed::RustEmbed;
use salvo::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use salvo::prelude::*;
use std::sync::LazyLock;

use crate::web::{Layout, layout, not_found, render};

const GUIDE: &str = include_str!("../../docs/guide.md");
/// where the guide's links to the rest of the repository go
const REPO: &str = "https://github.com/SaulDoesCode/anend/blob/main/";

#[derive(RustEmbed)]
#[folder = "../docs/guide/"]
struct Pictures;

static HTML: LazyLock<String> = LazyLock::new(|| to_html(GUIDE));

#[derive(Template)]
#[template(path = "guide.html")]
struct GuidePage {
    l: Layout,
    body: &'static str,
}

#[handler]
pub async fn page(depot: &mut Depot, res: &mut Response) {
    render(
        res,
        GuidePage {
            l: layout(depot, "guide"),
            body: HTML.as_str(),
        },
    );
}

#[handler]
pub async fn picture(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let name = req.param::<String>("name").unwrap_or_default();
    match Pictures::get(&name).filter(|_| picture_name(&name)) {
        Some(f) => {
            res.headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
            res.headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));
            res.write_body(f.data.into_owned()).ok();
        }
        None => not_found(res, depot),
    }
}

/// a picture's name: lowercase letters, digits and dashes, then .png
fn picture_name(name: &str) -> bool {
    name.strip_suffix(".png").is_some_and(|stem| {
        !stem.is_empty()
            && stem
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

fn escape(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

/// a heading's anchor, the way GitHub makes them, so the guide's links work in both places
pub fn slug(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '_' {
                Some(c.to_lowercase().next().unwrap_or(c))
            } else if c == ' ' || c == '-' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// where a link may go: the page's own headings, the repository's files, https
fn link(dest: &str) -> String {
    if dest.starts_with('#') || dest.starts_with("https://") || dest.starts_with("http://127.0.0.1") {
        dest.to_string()
    } else if let Some(file) = dest.strip_prefix("../") {
        format!("{REPO}{file}")
    } else {
        "#".to_string()
    }
}

fn level(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// markdown to HTML: headings with anchors, paragraphs, emphasis, code, lists, tables,
/// quotes, rules, links and the guide's own pictures. Anything else is left out, and raw
/// HTML shows as text.
pub fn to_html(md: &str) -> String {
    let mut out = String::with_capacity(md.len() * 2);
    // a heading being read: where it starts in `out`, its level, its text
    let mut heading: Option<(usize, u8, String)> = None;
    // an image being read: its source and alt text
    let mut image: Option<(String, String)> = None;
    let mut in_head = false;
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(md, opts) {
        if let Some((_, alt)) = image.as_mut() {
            match ev {
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    let (src, alt) = image.take().unwrap_or_default();
                    if let Some(name) = src.strip_prefix("guide/").filter(|n| picture_name(n)) {
                        out.push_str("<img src=\"/guide/");
                        out.push_str(name);
                        out.push_str("\" alt=\"");
                        escape(&mut out, &alt);
                        out.push_str("\" loading=\"lazy\">");
                    }
                }
                _ => {}
            }
            continue;
        }
        if let Some((_, _, text)) = heading.as_mut()
            && let Event::Text(t) | Event::Code(t) = &ev
        {
            text.push_str(t);
        }
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => out.push_str("<p>"),
                Tag::Heading { level: l, .. } => heading = Some((out.len(), level(l), String::new())),
                Tag::BlockQuote(_) => out.push_str("<blockquote>"),
                Tag::CodeBlock(kind) => {
                    out.push_str("<pre><code");
                    if let CodeBlockKind::Fenced(lang) = kind
                        && !lang.is_empty()
                    {
                        out.push_str(" class=\"lang-");
                        escape(&mut out, &lang);
                        out.push('"');
                    }
                    out.push('>');
                }
                Tag::List(Some(1)) => out.push_str("<ol>"),
                Tag::List(Some(n)) => out.push_str(&format!("<ol start=\"{n}\">")),
                Tag::List(None) => out.push_str("<ul>"),
                Tag::Item => out.push_str("<li>"),
                Tag::Table(_) => out.push_str("<div class=\"scroll\"><table>"),
                Tag::TableHead => {
                    in_head = true;
                    out.push_str("<thead><tr>");
                }
                Tag::TableRow => out.push_str("<tr>"),
                Tag::TableCell => out.push_str(if in_head { "<th>" } else { "<td>" }),
                Tag::Emphasis => out.push_str("<em>"),
                Tag::Strong => out.push_str("<strong>"),
                Tag::Strikethrough => out.push_str("<s>"),
                Tag::Link { dest_url, .. } => {
                    let to = link(&dest_url);
                    out.push_str("<a href=\"");
                    escape(&mut out, &to);
                    out.push('"');
                    if to.starts_with("http") {
                        out.push_str(" rel=\"noopener\"");
                    }
                    out.push('>');
                }
                Tag::Image { dest_url, .. } => image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => out.push_str("</p>\n"),
                TagEnd::Heading(_) => {
                    if let Some((at, n, text)) = heading.take() {
                        out.insert_str(at, &format!("<h{n} id=\"{}\">", slug(&text)));
                        out.push_str(&format!("</h{n}>\n"));
                    }
                }
                TagEnd::BlockQuote(_) => out.push_str("</blockquote>\n"),
                TagEnd::CodeBlock => out.push_str("</code></pre>\n"),
                TagEnd::List(true) => out.push_str("</ol>\n"),
                TagEnd::List(false) => out.push_str("</ul>\n"),
                TagEnd::Item => out.push_str("</li>\n"),
                TagEnd::Table => out.push_str("</tbody></table></div>\n"),
                TagEnd::TableHead => {
                    in_head = false;
                    out.push_str("</tr></thead><tbody>\n");
                }
                TagEnd::TableRow => out.push_str("</tr>\n"),
                TagEnd::TableCell => out.push_str(if in_head { "</th>" } else { "</td>" }),
                TagEnd::Emphasis => out.push_str("</em>"),
                TagEnd::Strong => out.push_str("</strong>"),
                TagEnd::Strikethrough => out.push_str("</s>"),
                TagEnd::Link => out.push_str("</a>"),
                _ => {}
            },
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => escape(&mut out, &t),
            Event::Code(t) => {
                out.push_str("<code>");
                escape(&mut out, &t);
                out.push_str("</code>");
            }
            Event::SoftBreak => out.push('\n'),
            Event::HardBreak => out.push_str("<br>\n"),
            Event::Rule => out.push_str("<hr>\n"),
            Event::TaskListMarker(done) => out.push_str(if done { "☑ " } else { "☐ " }),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_anchors() {
        let h = to_html(
            "# Teaching it: yes & no\n\n<script>x</script>\n\n*a* `<b>`\n\n[in](#teaching-it-yes--no) [out](javascript:alert(1)) [readme](../README.md)\n\n![x](guide/editor.png) ![y](../secret.png)\n",
        );
        assert!(
            h.contains("<h1 id=\"teaching-it-yes--no\">Teaching it: yes &amp; no</h1>"),
            "{h}"
        );
        assert!(!h.contains("<script>"), "{h}");
        assert!(h.contains("&lt;script&gt;"));
        assert!(h.contains("<code>&lt;b&gt;</code>"));
        assert!(h.contains("href=\"#\""), "{h}");
        assert!(h.contains(&format!("href=\"{REPO}README.md\"")));
        assert!(h.contains("<img src=\"/guide/editor.png\" alt=\"x\""));
        assert!(!h.contains("secret"));
    }

    /// every link to a heading finds it, and every picture is built in
    #[test]
    fn the_guide_holds_together() {
        let h = to_html(GUIDE);
        let mut rest = h.as_str();
        while let Some(i) = rest.find("href=\"#") {
            rest = &rest[i + 7..];
            let id = &rest[..rest.find('"').unwrap()];
            assert!(h.contains(&format!(" id=\"{id}\"")), "no heading for #{id}");
        }
        let mut rest = h.as_str();
        let mut pictures = 0;
        while let Some(i) = rest.find("<img src=\"/guide/") {
            rest = &rest[i + 17..];
            let name = &rest[..rest.find('"').unwrap()];
            assert!(Pictures::get(name).is_some(), "{name} isn't built in");
            pictures += 1;
        }
        assert_eq!(pictures, GUIDE.matches("](guide/").count());
        assert!(!picture_name("../x.png") && !picture_name(".png") && picture_name("learn.png"));
    }
}
