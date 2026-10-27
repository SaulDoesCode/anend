//! A small backtracking regular expression matcher, enough for the pre-tokenizer
//! patterns tokenizers ship (GPT-2, Llama 3, Qwen2, StarCoder and their relatives):
//! alternation, groups (`(…)`, `(?:…)`, `(?i:…)`), lookahead (`(?=…)`, `(?!…)`),
//! greedy quantifiers (`? * + {n} {n,} {n,m}`), `.`, classes (`[…]`, `[^…]`, ranges)
//! and the escapes `\s \S \d \D \w \W \r \n \t \p{…} \P{…}`.
//!
//! Matching is leftmost-first with backtracking, as in Oniguruma and `fancy-regex`
//! (which the tokenizers library uses), so alternatives are tried in order. Unicode
//! properties use std's tables: `\p{L}` is `char::is_alphabetic` (which also counts
//! the combining marks of some scripts as letters), `\p{N}` is `char::is_numeric`,
//! `\p{Lu}`/`\p{Ll}` are upper/lower case letters and `\p{Lt}`/`\p{Lm}`/`\p{Lo}` the
//! other letters. Anything else (`\p{M}`, anchors, back references, lazy quantifiers)
//! is refused with the construct named.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prop {
    Letter,
    Upper,
    Lower,
    OtherLetter,
    Number,
    Digit,
    Space,
    Word,
}

impl Prop {
    fn has(self, c: char) -> bool {
        match self {
            Prop::Letter => c.is_alphabetic(),
            Prop::Upper => c.is_alphabetic() && c.is_uppercase(),
            Prop::Lower => c.is_alphabetic() && c.is_lowercase(),
            Prop::OtherLetter => c.is_alphabetic() && !c.is_uppercase() && !c.is_lowercase(),
            Prop::Number => c.is_numeric(),
            Prop::Digit => c.is_ascii_digit(),
            Prop::Space => c.is_whitespace(),
            Prop::Word => c.is_alphanumeric() || c == '_',
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Item {
    Range(char, char),
    Prop(Prop, bool),
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Char(char, bool),
    Class(Vec<Item>, bool, bool),
    Any,
    Group(Vec<Vec<Node>>),
    Look(Vec<Vec<Node>>, bool),
    Repeat(Box<Node>, usize, usize),
}

/// a compiled pattern
#[derive(Clone, Debug, PartialEq)]
pub struct Regex {
    source: String,
    alt: Vec<Vec<Node>>,
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
    src: &'a str,
}

impl Parser<'_> {
    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("pattern {:?}: {what} (at {})", self.src, self.i))
    }

    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn alt(&mut self, ci: bool) -> Result<Vec<Vec<Node>>, String> {
        let mut alts = vec![self.seq(ci)?];
        while self.eat('|') {
            alts.push(self.seq(ci)?);
        }
        Ok(alts)
    }

    fn seq(&mut self, ci: bool) -> Result<Vec<Node>, String> {
        let mut v = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom(ci)?;
            let atom = self.quantified(atom)?;
            v.push(atom);
        }
        Ok(v)
    }

    fn number(&mut self) -> Option<usize> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        (self.i > start).then(|| self.s[start..self.i].iter().collect::<String>().parse().ok())?
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, String> {
        let (min, max) = match self.peek() {
            Some('?') => (0, 1),
            Some('*') => (0, usize::MAX),
            Some('+') => (1, usize::MAX),
            Some('{') => {
                let save = self.i;
                self.i += 1;
                let Some(a) = self.number() else {
                    self.i = save;
                    return Ok(atom);
                };
                let b = if self.eat(',') {
                    self.number().unwrap_or(usize::MAX)
                } else {
                    a
                };
                if !self.eat('}') || b < a {
                    return self.err("a malformed {n,m}");
                }
                self.i -= 1;
                (a, b)
            }
            _ => return Ok(atom),
        };
        self.i += 1;
        if matches!(self.peek(), Some('?' | '+')) {
            return self.err("lazy or possessive quantifiers are not supported");
        }
        if matches!(atom, Node::Look(..)) {
            return self.err("a quantified lookahead");
        }
        Ok(Node::Repeat(Box::new(atom), min, max))
    }

    fn atom(&mut self, ci: bool) -> Result<Node, String> {
        let c = self.peek().expect("caller checked");
        self.i += 1;
        Ok(match c {
            '(' => {
                let node = if self.eat('?') {
                    match self.peek() {
                        Some(':') => {
                            self.i += 1;
                            Node::Group(self.alt(ci)?)
                        }
                        Some('i') if self.s.get(self.i + 1) == Some(&':') => {
                            self.i += 2;
                            Node::Group(self.alt(true)?)
                        }
                        Some('=') => {
                            self.i += 1;
                            Node::Look(self.alt(ci)?, false)
                        }
                        Some('!') => {
                            self.i += 1;
                            Node::Look(self.alt(ci)?, true)
                        }
                        _ => return self.err("this group kind is not supported"),
                    }
                } else {
                    Node::Group(self.alt(ci)?)
                };
                if !self.eat(')') {
                    return self.err("an unclosed group");
                }
                node
            }
            '[' => self.class(ci)?,
            '.' => Node::Any,
            '\\' => self.escape(ci, false)?,
            '^' | '$' => return self.err("anchors are not supported"),
            '*' | '+' | '?' => return self.err("a quantifier with nothing before it"),
            c => Node::Char(c, ci),
        })
    }

    fn prop(&mut self) -> Result<Prop, String> {
        let name: String = if self.eat('{') {
            let start = self.i;
            while self.peek().is_some_and(|c| c != '}') {
                self.i += 1;
            }
            let n = self.s[start..self.i].iter().collect();
            if !self.eat('}') {
                return self.err("an unclosed \\p{");
            }
            n
        } else {
            match self.peek() {
                Some(c) => {
                    self.i += 1;
                    c.to_string()
                }
                None => return self.err("\\p at the end"),
            }
        };
        Ok(match name.as_str() {
            "L" | "Letter" => Prop::Letter,
            "Lu" => Prop::Upper,
            "Ll" => Prop::Lower,
            "Lt" | "Lm" | "Lo" => Prop::OtherLetter,
            "N" | "Number" => Prop::Number,
            "Nd" => Prop::Digit,
            other => return self.err(&format!("unicode property {other} is not supported")),
        })
    }

    /// an escape; in a class it must be a single item
    fn escape(&mut self, ci: bool, in_class: bool) -> Result<Node, String> {
        let Some(c) = self.peek() else {
            return self.err("a backslash at the end");
        };
        self.i += 1;
        let prop = |p: Prop, neg: bool| Node::Class(vec![Item::Prop(p, false)], neg, false);
        Ok(match c {
            's' => prop(Prop::Space, false),
            'S' => prop(Prop::Space, true),
            'd' => prop(Prop::Digit, false),
            'D' => prop(Prop::Digit, true),
            'w' => prop(Prop::Word, false),
            'W' => prop(Prop::Word, true),
            'p' => prop(self.prop()?, false),
            'P' => prop(self.prop()?, true),
            'r' => Node::Char('\r', ci),
            'n' => Node::Char('\n', ci),
            't' => Node::Char('\t', ci),
            'f' => Node::Char('\x0c', ci),
            'v' => Node::Char('\x0b', ci),
            '0' => Node::Char('\0', ci),
            c if c.is_ascii_alphanumeric() => {
                return self.err(&format!(
                    "escape \\{c} is not supported{}",
                    if in_class { " in a class" } else { "" }
                ));
            }
            c => Node::Char(c, ci),
        })
    }

    fn class(&mut self, ci: bool) -> Result<Node, String> {
        let neg = self.eat('^');
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let Some(c) = self.peek() else {
                return self.err("an unclosed class");
            };
            if c == ']' && !first {
                self.i += 1;
                break;
            }
            first = false;
            self.i += 1;
            let lo = if c == '\\' {
                match self.escape(false, true)? {
                    Node::Char(ch, _) => ch,
                    Node::Class(it, n, _) => {
                        // \s, \p{L}, \P{N} … inside a class
                        for x in it {
                            if let Item::Prop(p, _) = x {
                                items.push(Item::Prop(p, n));
                            }
                        }
                        continue;
                    }
                    _ => unreachable!(),
                }
            } else if c == '[' && self.peek() == Some(':') {
                return self.err("POSIX classes are not supported");
            } else {
                c
            };
            if self.peek() == Some('-') && self.s.get(self.i + 1).is_some_and(|&n| n != ']') {
                self.i += 1;
                let hc = self.peek().expect("checked");
                self.i += 1;
                let hi = if hc == '\\' {
                    match self.escape(false, true)? {
                        Node::Char(ch, _) => ch,
                        _ => return self.err("a range ending in a class"),
                    }
                } else {
                    hc
                };
                if hi < lo {
                    return self.err("a backwards range");
                }
                items.push(Item::Range(lo, hi));
            } else {
                items.push(Item::Range(lo, lo));
            }
        }
        Ok(Node::Class(items, neg, ci))
    }
}

fn fold(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

fn class_has(items: &[Item], neg: bool, ci: bool, c: char) -> bool {
    let test = |c: char| {
        items.iter().any(|it| match *it {
            Item::Range(a, b) => (a..=b).contains(&c),
            Item::Prop(p, n) => p.has(c) != n,
        })
    };
    let hit = test(c) || (ci && (test(fold(c)) || c.to_uppercase().any(test)));
    hit != neg
}

impl Node {
    /// matches one character (for the iterative repeat)
    fn single(&self, c: char) -> Option<bool> {
        match self {
            Node::Char(x, ci) => Some(*x == c || (*ci && fold(*x) == fold(c))),
            Node::Class(items, neg, ci) => Some(class_has(items, *neg, *ci, c)),
            Node::Any => Some(c != '\n'),
            _ => None,
        }
    }
}

type Cont<'a> = &'a mut dyn FnMut(usize) -> bool;

impl Regex {
    pub fn new(src: &str) -> Result<Regex, String> {
        let chars: Vec<char> = src.chars().collect();
        let mut p = Parser { s: &chars, i: 0, src };
        let alt = p.alt(false)?;
        if p.i != chars.len() {
            return p.err("an unmatched )");
        }
        Ok(Regex {
            source: src.to_string(),
            alt,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    fn alt(&self, alt: &[Vec<Node>], s: &[char], pos: usize, k: Cont) -> bool {
        alt.iter().any(|seq| self.seq(seq, s, pos, k))
    }

    fn seq(&self, seq: &[Node], s: &[char], pos: usize, k: Cont) -> bool {
        match seq.split_first() {
            None => k(pos),
            Some((n, rest)) => self.node(n, s, pos, &mut |p| self.seq(rest, s, p, k)),
        }
    }

    fn node(&self, n: &Node, s: &[char], pos: usize, k: Cont) -> bool {
        match n {
            Node::Group(alt) => self.alt(alt, s, pos, k),
            Node::Look(alt, neg) => {
                let found = self.alt(alt, s, pos, &mut |_| true);
                found != *neg && k(pos)
            }
            Node::Repeat(inner, min, max) => {
                if inner.single('a').is_some() {
                    let mut count = 0;
                    while count < *max && pos + count < s.len() && inner.single(s[pos + count]) == Some(true) {
                        count += 1;
                    }
                    (*min..=count).rev().any(|c| k(pos + c))
                } else {
                    self.repeat(inner, *min, *max, 0, s, pos, k)
                }
            }
            single => pos < s.len() && single.single(s[pos]) == Some(true) && k(pos + 1),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(&self, n: &Node, min: usize, max: usize, done: usize, s: &[char], pos: usize, k: Cont) -> bool {
        if done < max
            && self.node(n, s, pos, &mut |p| {
                // an empty iteration can't make progress; stop repeating there
                p != pos && self.repeat(n, min, max, done + 1, s, p, k)
            })
        {
            return true;
        }
        done >= min && k(pos)
    }

    /// the end of the match starting at `pos`, if any
    fn match_at(&self, s: &[char], pos: usize) -> Option<usize> {
        let mut end = None;
        self.alt(&self.alt, s, pos, &mut |p| {
            end = Some(p);
            true
        });
        end
    }

    /// every non-overlapping match, leftmost first, as char ranges
    pub fn find_iter(&self, s: &[char]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos < s.len() {
            match self.match_at(s, pos) {
                Some(e) if e > pos => {
                    out.push((pos, e));
                    pos = e;
                }
                _ => pos += 1,
            }
        }
        out
    }

    /// the matches of a string as substrings
    pub fn find_all<'a>(&self, text: &'a str) -> Vec<&'a str> {
        let chars: Vec<char> = text.chars().collect();
        let offs: Vec<usize> = text.char_indices().map(|(i, _)| i).chain([text.len()]).collect();
        self.find_iter(&chars)
            .into_iter()
            .map(|(a, b)| &text[offs[a]..offs[b]])
            .collect()
    }
}

/// the GPT-2 pre-tokenizer pattern (`ByteLevel` with `use_regex`)
pub const GPT2: &str = r"'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+(?!\S)|\s+";

#[cfg(test)]
mod tests {
    use super::*;

    const LLAMA3: &str = r"(?i:'s|'t|'re|'ve|'m|'ll|'d)|[^\r\n\p{L}\p{N}]?\p{L}+|\p{N}{1,3}| ?[^\s\p{L}\p{N}]+[\r\n]*|\s*[\r\n]+|\s+(?!\S)|\s+";

    #[test]
    fn gpt2_pattern_splits_like_the_reference() {
        let r = Regex::new(GPT2).unwrap();
        assert_eq!(
            r.find_all("Hello world, it's 2024!  Ok"),
            ["Hello", " world", ",", " it", "'s", " 2024", "!", " ", " Ok"]
        );
        assert_eq!(r.find_all("a   b\n\nc"), ["a", "  ", " b", "\n", "\n", "c"]);
        assert_eq!(r.find_all("héllo wörld"), ["héllo", " wörld"]);
        assert_eq!(r.find_all("tail   "), ["tail", "   "]);
    }

    #[test]
    fn llama3_pattern_splits_like_the_reference() {
        let r = Regex::new(LLAMA3).unwrap();
        assert_eq!(
            r.find_all("I'LL pay 12345 now"),
            ["I", "'LL", " pay", " ", "123", "45", " now"]
        );
        assert_eq!(
            r.find_all("x = (a+b);\n\ny"),
            ["x", " =", " (", "a", "+b", ");\n\n", "y"]
        );
        assert_eq!(r.find_all("  \n hi"), ["  \n", " hi"]);
        assert_eq!(r.find_all("$foo"), ["$foo"]);
    }

    #[test]
    fn constructs() {
        let r = Regex::new(r"a{2,3}|[x-z]+|(?:ab)+c|\d").unwrap();
        assert_eq!(r.find_all("aaaa xyz ababc 7"), ["aaa", "xyz", "ababc", "7"]);
        let r = Regex::new(r"foo(?=bar)|[^a-c\s]").unwrap();
        assert_eq!(r.find_all("foobar foo"), ["foo", "r", "f", "o", "o"]);
        for (bad, says) in [
            (r"\p{M}", "property M"),
            ("(a", "unclosed group"),
            ("a)", "unmatched"),
            ("a+?", "lazy"),
            ("^a", "anchors"),
            (r"\1", "escape \\1"),
        ] {
            let e = Regex::new(bad).unwrap_err();
            assert!(e.contains(says), "{bad}: {e}");
        }
    }
}
