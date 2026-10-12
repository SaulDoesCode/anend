//! Turns markdown into simple layout blocks of styled text runs, which the UI then
//! renders with Bevy text spans (there is no HTML to lean on in a native app).

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: bool,
}

impl Span {
    fn same_style(&self, other: &Span) -> bool {
        (self.bold, self.italic, self.code, self.strike, self.link)
            == (other.bold, other.italic, other.code, other.strike, other.link)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph {
        spans: Vec<Span>,
        quote: bool,
    },
    /// a list item, `marker` is "•", "3." or a task box
    Item {
        depth: usize,
        marker: String,
        spans: Vec<Span>,
        quote: bool,
    },
    Code(String),
    Rule,
    TableRow {
        cells: Vec<Vec<Span>>,
        header: bool,
    },
}

#[derive(Default)]
struct Style {
    bold: usize,
    italic: usize,
    strike: usize,
    link: usize,
}

enum Container {
    Paragraph,
    Heading(u8),
    Item { depth: usize, marker: String },
    Cell,
}

/// lists: `None` for bullets, `Some(n)` for the next number
type ListStack = Vec<Option<u64>>;

pub fn parse(markdown: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut blocks = Vec::new();
    let mut style = Style::default();
    let mut lists: ListStack = Vec::new();
    let mut quote_depth = 0usize;
    let mut current: Option<(Container, Vec<Span>)> = None;
    let mut code: Option<String> = None;
    let mut row: Vec<Vec<Span>> = Vec::new();
    let mut in_head = false;

    fn flush(blocks: &mut Vec<Block>, current: &mut Option<(Container, Vec<Span>)>, quote: bool) {
        let Some((container, spans)) = current.take() else {
            return;
        };
        match container {
            Container::Paragraph if !spans.is_empty() => blocks.push(Block::Paragraph { spans, quote }),
            Container::Heading(level) => blocks.push(Block::Heading { level, spans }),
            Container::Item { depth, marker } => blocks.push(Block::Item {
                depth,
                marker,
                spans,
                quote,
            }),
            _ => {}
        }
    }

    let push_text = |current: &mut Option<(Container, Vec<Span>)>, style: &Style, text: &str, code: bool| {
        let (_, spans) = current.get_or_insert_with(|| (Container::Paragraph, Vec::new()));
        let span = Span {
            text: text.to_string(),
            bold: style.bold > 0,
            italic: style.italic > 0,
            code,
            strike: style.strike > 0,
            link: style.link > 0,
        };
        // merge runs with identical styling
        if let Some(last) = spans.last_mut()
            && last.same_style(&span)
        {
            last.text.push_str(&span.text);
            return;
        }
        spans.push(span);
    };

    for event in Parser::new_ext(markdown, opts) {
        let quote = quote_depth > 0;
        match event {
            Event::Start(Tag::Paragraph) => {
                // a paragraph inside a list item continues that item
                if !matches!(current, Some((Container::Item { .. }, _))) {
                    flush(&mut blocks, &mut current, quote);
                    current = Some((Container::Paragraph, Vec::new()));
                } else if let Some((_, spans)) = &current
                    && !spans.is_empty()
                {
                    flush(&mut blocks, &mut current, quote);
                    current = Some((Container::Paragraph, Vec::new()));
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if matches!(current, Some((Container::Paragraph, _))) {
                    flush(&mut blocks, &mut current, quote);
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut blocks, &mut current, quote);
                let level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                current = Some((Container::Heading(level), Vec::new()));
            }
            Event::End(TagEnd::Heading(_)) => flush(&mut blocks, &mut current, quote),
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut blocks, &mut current, quote);
                quote_depth += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut blocks, &mut current, quote);
                quote_depth = quote_depth.saturating_sub(1);
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut blocks, &mut current, quote);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut blocks, &mut current, quote);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut current, quote);
                let depth = lists.len().saturating_sub(1);
                let marker = match lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{}.", n);
                        *n += 1;
                        m
                    }
                    _ => "•".to_string(),
                };
                current = Some((Container::Item { depth, marker }, Vec::new()));
            }
            Event::End(TagEnd::Item) => flush(&mut blocks, &mut current, quote),
            Event::TaskListMarker(done) => {
                if let Some((Container::Item { marker, .. }, _)) = &mut current {
                    *marker = if done { "☑".into() } else { "☐".into() };
                }
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut blocks, &mut current, quote);
                code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(c) = code.take() {
                    blocks.push(Block::Code(c.trim_end_matches('\n').to_string()));
                }
            }
            Event::Start(Tag::Table(_)) => flush(&mut blocks, &mut current, quote),
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => {
                blocks.push(Block::TableRow {
                    cells: std::mem::take(&mut row),
                    header: true,
                });
                in_head = false;
            }
            Event::Start(Tag::TableRow) => row.clear(),
            Event::End(TagEnd::TableRow) => {
                if !in_head {
                    blocks.push(Block::TableRow {
                        cells: std::mem::take(&mut row),
                        header: false,
                    });
                }
            }
            Event::Start(Tag::TableCell) => current = Some((Container::Cell, Vec::new())),
            Event::End(TagEnd::TableCell) => {
                if let Some((_, spans)) = current.take() {
                    row.push(spans);
                }
            }
            Event::Start(Tag::Strong) => style.bold += 1,
            Event::End(TagEnd::Strong) => style.bold = style.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => style.italic += 1,
            Event::End(TagEnd::Emphasis) => style.italic = style.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => style.strike += 1,
            Event::End(TagEnd::Strikethrough) => style.strike = style.strike.saturating_sub(1),
            Event::Start(Tag::Link { .. }) => style.link += 1,
            Event::End(TagEnd::Link) => style.link = style.link.saturating_sub(1),
            Event::Start(Tag::Image { .. }) => push_text(&mut current, &style, "[image: ", false),
            Event::End(TagEnd::Image) => push_text(&mut current, &style, "]", false),
            Event::Text(t) => match &mut code {
                Some(c) => c.push_str(&t),
                None => push_text(&mut current, &style, &t, false),
            },
            Event::Code(t) => push_text(&mut current, &style, &t, true),
            Event::Html(t) | Event::InlineHtml(t) => match &mut code {
                Some(c) => c.push_str(&t),
                None => push_text(&mut current, &style, t.trim_end_matches('\n'), true),
            },
            Event::SoftBreak => push_text(&mut current, &style, " ", false),
            Event::HardBreak => push_text(&mut current, &style, "\n", false),
            Event::Rule => {
                flush(&mut blocks, &mut current, quote);
                blocks.push(Block::Rule);
            }
            _ => {}
        }
    }
    flush(&mut blocks, &mut current, quote_depth > 0);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn headings_paragraphs_and_styles() {
        let blocks = parse("# Title\n\nsome **bold** and *it* `code` [link](x)\nnext line\n\n---\n");
        assert_eq!(blocks.len(), 3);
        match &blocks[0] {
            Block::Heading { level: 1, spans } => assert_eq!(plain(spans), "Title"),
            other => panic!("{:?}", other),
        }
        match &blocks[1] {
            Block::Paragraph { spans, quote: false } => {
                assert_eq!(plain(spans), "some bold and it code link next line");
                assert!(spans.iter().any(|s| s.bold && s.text == "bold"));
                assert!(spans.iter().any(|s| s.italic && s.text == "it"));
                assert!(spans.iter().any(|s| s.code && s.text == "code"));
                assert!(spans.iter().any(|s| s.link && s.text == "link"));
            }
            other => panic!("{:?}", other),
        }
        assert_eq!(blocks[2], Block::Rule);
    }

    #[test]
    fn lists_tasks_quotes_code_tables() {
        let md = "- a\n- [x] done\n  1. one\n  2. two\n\n> quoted\n\n```\nfn x() {}\n```\n\n| h1 | h2 |\n|---|---|\n| c1 | c2 |\n";
        let blocks = parse(md);
        let items: Vec<(usize, String, String)> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Item {
                    depth, marker, spans, ..
                } => Some((*depth, marker.clone(), plain(spans))),
                _ => None,
            })
            .collect();
        assert_eq!(
            items,
            vec![
                (0, "•".into(), "a".into()),
                (0, "☑".into(), "done".into()),
                (1, "1.".into(), "one".into()),
                (1, "2.".into(), "two".into()),
            ]
        );
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Paragraph { quote: true, spans } if plain(spans) == "quoted"))
        );
        assert!(blocks.contains(&Block::Code("fn x() {}".into())));
        let rows: Vec<&Block> = blocks.iter().filter(|b| matches!(b, Block::TableRow { .. })).collect();
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0], Block::TableRow { header: true, cells } if cells.len() == 2));
    }
}
