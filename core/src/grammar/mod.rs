//! A proof of concept for the classifiers: finding spelling and word-choice errors in a
//! text, and suggesting what was meant.
//!
//! Two small trained models, both logistic regressions with deterministic training:
//!
//! - the **detector** reads each word in its context and gives the probability that it is
//!   an error: a word no lexicon knows (with what its letters look like and whether a
//!   known word is one edit away), a word repeated by mistake ("the the"), or a word of a
//!   confusable set ("their / there / they're", "its / it's", "then / than", ...) where
//!   another member fits its neighbours better;
//! - the **suggester** ranks replacements for a flagged word: known words one edit away
//!   (a letter dropped, added, swapped with its neighbour or replaced), the confusable
//!   set's other members, or taking a repeated word out; by the kind of edit, how common
//!   the candidate is and how well it fits between its neighbours.
//!
//! Both learn from deterministic corruptions of clean text: `corpus.txt` (the project's
//! own prose, from its docs and doc comments: 2870 sentences) has words dropped, doubled,
//! swapped, mistyped with a keyboard neighbour, repeated and confused, and the models
//! learn to find the corrupted words and to bring back the originals. A fifth of the
//! sentences are held out and corrupted the same way to measure them ([`Report`]):
//! precision and recall of the detector per word, how often the suggester's first (and
//! first three) suggestions hold the original, and how often clean text is flagged.
//!
//! The lexicon is `words.fc` (the MIT licensed `word-list` package, 269 870 lowercase
//! English words, front coded), the corpus's words, a list of contractions and, when
//! checking a save's text, every word the save holds, so one's own names and terms
//! aren't errors. Words with digits, or two capitals or more, are never flagged.
//!
//! What it is not: a grammar checker. It knows nothing of agreement or tense; its
//! word-choice sense is the neighbour counts of a 30 000 word corpus, so it is right
//! about common pairs ("more than", "it's a") and guesses elsewhere.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::Serialize;

const WORDS_FC: &str = include_str!("words.fc");
const CORPUS: &str = include_str!("corpus.txt");

const CONTRACTIONS: &[&str] = &[
    "it's",
    "don't",
    "doesn't",
    "isn't",
    "aren't",
    "can't",
    "won't",
    "i'm",
    "you're",
    "they're",
    "we're",
    "i've",
    "you've",
    "we've",
    "they've",
    "i'll",
    "you'll",
    "he'll",
    "she'll",
    "we'll",
    "they'll",
    "he's",
    "she's",
    "that's",
    "there's",
    "what's",
    "who's",
    "let's",
    "didn't",
    "wasn't",
    "weren't",
    "hasn't",
    "haven't",
    "hadn't",
    "couldn't",
    "wouldn't",
    "shouldn't",
    "mustn't",
    "i'd",
    "you'd",
    "he'd",
    "she'd",
    "we'd",
    "they'd",
    "here's",
    "where's",
    "how's",
    "it'll",
    "that'll",
    "a",
    "i",
];

/// words easily written for one another
const CONFUSABLE: &[&[&str]] = &[
    &["their", "there", "they're"],
    &["its", "it's"],
    &["your", "you're"],
    &["then", "than"],
    &["to", "too"],
    &["a", "an"],
    &["affect", "effect"],
    &["lose", "loose"],
    &["were", "where"],
    &["whose", "who's"],
    &["of", "off"],
    &["accept", "except"],
    &["quiet", "quite"],
    &["weather", "whether"],
    &["principal", "principle"],
];

/// what a repeated word may be without being a mistake
const MAY_REPEAT: &[&str] = &["that", "had", "is", "very", "bye", "no", "ha", "so"];

const KEYBOARD: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];

fn neighbours(c: char) -> Vec<char> {
    let mut out = Vec::new();
    for (r, row) in KEYBOARD.iter().enumerate() {
        if let Some(i) = row.find(c) {
            for (rr, other) in KEYBOARD.iter().enumerate() {
                if rr.abs_diff(r) > 1 {
                    continue;
                }
                for (j, d) in other.char_indices() {
                    let shift = if rr == r { 0 } else { 1 };
                    if d != c && j.abs_diff(i) <= shift.max(1) && (rr == r || j + 1 >= i && j <= i + 1) {
                        out.push(d);
                    }
                }
            }
        }
    }
    out
}

fn is_neighbour(a: char, b: char) -> bool {
    neighbours(a).contains(&b)
}

// ---------------------------------------------------------------- words in a text

/// a word in a text: its byte span
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// the words of a text: runs of letters, with an apostrophe inside (it's, don't)
pub fn words(text: &str) -> Vec<Span> {
    let b: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].1.is_alphabetic() {
            let start = b[i].0;
            let mut j = i;
            while j + 1 < b.len()
                && (b[j + 1].1.is_alphabetic()
                    || ((b[j + 1].1 == '\'' || b[j + 1].1 == '’') && j + 2 < b.len() && b[j + 2].1.is_alphabetic()))
            {
                j += 1;
            }
            let end = b.get(j + 1).map(|x| x.0).unwrap_or(text.len());
            out.push(Span { start, end });
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn lower(w: &str) -> String {
    w.to_lowercase().replace('’', "'")
}

/// a word that is never flagged: a name in capitals, an acronym, with digits
fn exempt(w: &str) -> bool {
    w.chars().filter(|c| c.is_uppercase()).count() >= 2 || w.chars().any(|c| c.is_ascii_digit()) || !w.is_ascii()
}

// ---------------------------------------------------------------- the lexicon and counts

struct Lexicon {
    words: HashSet<String>,
}

impl Lexicon {
    fn load() -> Lexicon {
        let mut words = HashSet::with_capacity(280_000);
        let mut prev = String::new();
        for line in WORDS_FC.lines() {
            let mut cs = line.chars();
            let k = cs.next().and_then(|c| c.to_digit(10)).unwrap_or(0) as usize;
            let mut w: String = prev.chars().take(k).collect();
            w.push_str(cs.as_str());
            words.insert(w.clone());
            prev = w;
        }
        for c in CONTRACTIONS {
            words.insert(c.to_string());
        }
        Lexicon { words }
    }
}

#[derive(Default)]
struct Counts {
    uni: HashMap<String, u32>,
    bi: HashMap<(String, String), u32>,
}

impl Counts {
    fn add(&mut self, sentence: &str) {
        let ws: Vec<String> = words(sentence)
            .iter()
            .map(|s| lower(&sentence[s.start..s.end]))
            .collect();
        for (i, w) in ws.iter().enumerate() {
            *self.uni.entry(w.clone()).or_default() += 1;
            let prev = if i == 0 { "<s>".to_string() } else { ws[i - 1].clone() };
            *self.bi.entry((prev, w.clone())).or_default() += 1;
        }
        if let Some(last) = ws.last() {
            *self.bi.entry((last.clone(), "</s>".into())).or_default() += 1;
        }
    }

    fn uni(&self, w: &str) -> f32 {
        (1.0 + *self.uni.get(w).unwrap_or(&0) as f32).ln()
    }

    fn bi(&self, a: &str, b: &str) -> f32 {
        (1.0 + *self.bi.get(&(a.to_string(), b.to_string())).unwrap_or(&0) as f32).ln()
    }

    /// how well `w` fits between `prev` and `next`
    fn fit(&self, prev: &str, w: &str, next: &str) -> f32 {
        self.bi(prev, w) + self.bi(w, next)
    }
}

// ---------------------------------------------------------------- a word in its context

struct Ctx<'a> {
    /// the word as written, and lower case
    raw: &'a str,
    w: String,
    prev: String,
    next: String,
    /// only spaces between it and the word before
    spaced: bool,
    first: bool,
}

fn contexts<'a>(text: &'a str, spans: &[Span]) -> Vec<Ctx<'a>> {
    let low: Vec<String> = spans.iter().map(|s| lower(&text[s.start..s.end])).collect();
    spans
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let gap = if i == 0 {
                &text[..s.start]
            } else {
                &text[spans[i - 1].end..s.start]
            };
            let first = i == 0 || gap.contains(['.', '!', '?', '\n']);
            Ctx {
                raw: &text[s.start..s.end],
                w: low[i].clone(),
                prev: if i == 0 || first {
                    "<s>".into()
                } else {
                    low[i - 1].clone()
                },
                next: low.get(i + 1).cloned().unwrap_or_else(|| "</s>".into()),
                spaced: i > 0 && !gap.is_empty() && gap.chars().all(|c| c == ' ' || c == '\t'),
                first,
            }
        })
        .collect()
}

fn confusable_set(w: &str) -> Option<&'static [&'static str]> {
    CONFUSABLE.iter().copied().find(|s| s.contains(&w))
}

/// every string one edit from `w`: (the string, the kind of edit)
fn edits1(w: &str) -> Vec<(String, Edit)> {
    let cs: Vec<char> = w.chars().collect();
    let letters = "abcdefghijklmnopqrstuvwxyz'";
    let mut out = Vec::new();
    for i in 0..cs.len() {
        // a letter that shouldn't be there: the original had it deleted... the other way
        // round: the candidate is `w` with letter i taken out (the writer added it)
        let mut d = cs.clone();
        d.remove(i);
        out.push((d.iter().collect(), Edit::Added));
        if i + 1 < cs.len() {
            let mut t = cs.clone();
            t.swap(i, i + 1);
            out.push((t.iter().collect(), Edit::Swapped));
        }
        for l in letters.chars() {
            if l != cs[i] {
                let mut r = cs.clone();
                r[i] = l;
                let kind = if is_neighbour(cs[i], l) {
                    Edit::Neighbour
                } else {
                    Edit::Replaced
                };
                out.push((r.iter().collect(), kind));
            }
        }
    }
    for i in 0..=cs.len() {
        for l in letters.chars() {
            let mut a = cs.clone();
            a.insert(i, l);
            out.push((a.iter().collect(), Edit::Dropped));
        }
    }
    out
}

/// what the writer did, as a candidate undoes it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit {
    /// the writer added a letter (the candidate takes it out)
    Added,
    /// the writer dropped a letter
    Dropped,
    Swapped,
    /// a key next to the right one
    Neighbour,
    Replaced,
    Confused,
    Repeated,
}

const EDITS: [Edit; 7] = [
    Edit::Added,
    Edit::Dropped,
    Edit::Swapped,
    Edit::Neighbour,
    Edit::Replaced,
    Edit::Confused,
    Edit::Repeated,
];

// ---------------------------------------------------------------- the two models

const DET_BITS: u32 = 16;

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

/// a sparse logistic regression over hashed features
struct Detector {
    w: Vec<f32>,
}

impl Detector {
    fn score(&self, feats: &[u32]) -> f32 {
        sigmoid(feats.iter().map(|&i| self.w[i as usize]).sum())
    }
}

/// the suggester's features for replacing `c.w` by `cand`
const SUG_N: usize = 9 + EDITS.len();

struct Suggester {
    w: [f32; SUG_N],
}

/// a deterministic generator (xorshift)
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, p: f64) -> bool {
        (self.next() % 1_000_000) as f64 / 1_000_000.0 < p
    }
}

/// how well the two models did on the held-out sentences
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Report {
    pub train_sentences: usize,
    pub test_sentences: usize,
    /// per word, at the default threshold
    pub precision: f32,
    pub recall: f32,
    pub f1: f32,
    pub errors: usize,
    pub flagged: usize,
    pub caught: usize,
    /// the share of caught errors whose first suggestion is the original, and whose first
    /// three hold it
    pub top1: f32,
    pub top3: f32,
    /// clean held-out text: words flagged per thousand
    pub false_alarms_per_1000: f32,
    pub threshold: f32,
    /// recall by the kind of error: (caught, errors) for misspellings, confused words
    /// and repeated words
    pub by_kind: Vec<(String, usize, usize)>,
}

/// the trained models
pub struct Model {
    lex: Lexicon,
    counts: Counts,
    det: Detector,
    sug: Suggester,
    pub report: Report,
}

/// a flagged word
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Flag {
    /// byte offsets in the text
    pub start: usize,
    pub end: usize,
    /// UTF-16 offsets (a browser's text field counts these)
    pub from: usize,
    pub to: usize,
    pub word: String,
    /// the detector's probability that it's an error
    pub p: f32,
    /// what it was, probably: (replacement, probability among the candidates); an empty
    /// replacement takes the word out (with the space before it)
    pub suggestions: Vec<Suggestion>,
    /// spelling, repeated or confused
    pub kind: &'static str,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Suggestion {
    pub text: String,
    pub p: f32,
}

pub const THRESHOLD: f32 = 0.5;

impl Model {
    fn known(&self, w: &str, user: &dyn Fn(&str) -> bool) -> bool {
        self.lex.words.contains(w)
            || self.counts.uni.contains_key(w)
            || w.strip_suffix("'s").is_some_and(|s| self.lex.words.contains(s))
            || user(w)
    }

    fn det_features(&self, c: &Ctx, user: &dyn Fn(&str) -> bool) -> Vec<u32> {
        let mask = (1u64 << DET_BITS) - 1;
        let mut f: Vec<String> = vec!["bias".into()];
        let known = self.known(&c.w, user);
        let len = c.w.chars().count().min(12);
        f.push(format!("k{}", known as u8));
        f.push(format!("len{}|k{}", len, known as u8));
        if !known {
            let near = edits1(&c.w).iter().filter(|(s, _)| self.known(s, user)).count();
            f.push(format!("near{}", near.min(4)));
            let padded: Vec<char> = format!("^{}$", c.w).chars().collect();
            for t in padded.windows(3) {
                f.push(format!("t:{}", t.iter().collect::<String>()));
            }
            if c.raw.chars().next().is_some_and(char::is_uppercase) && !c.first {
                f.push("name".into());
            }
        } else {
            f.push(format!("uf{}", (self.counts.uni(&c.w) * 2.0) as i32));
        }
        if c.spaced && c.w == c.prev && !MAY_REPEAT.contains(&c.w.as_str()) {
            f.push("rep".into());
        }
        if let Some(set) = confusable_set(&c.w) {
            let mine = self.counts.fit(&c.prev, &c.w, &c.next);
            let best = set
                .iter()
                .filter(|a| **a != c.w)
                .map(|a| self.counts.fit(&c.prev, a, &c.next))
                .fold(f32::MIN, f32::max);
            let d = ((best - mine) * 2.0).round().clamp(-4.0, 4.0) as i32;
            f.push(format!("cb{d}"));
            f.push(format!("cw:{}", c.w));
            f.push(format!("cw:{}|n:{}", c.w, c.next));
            f.push(format!("cw:{}|p:{}", c.w, c.prev));
            // a / an by the next word's first sound, roughly
            if (c.w == "a" || c.w == "an") && c.next != "</s>" {
                let vowel = c.next.starts_with(['a', 'e', 'i', 'o', 'u']);
                f.push(format!("aan:{}|{}", c.w, vowel));
            }
        }
        f.iter().map(|s| (fnv(s) & mask) as u32).collect()
    }

    fn candidates(&self, c: &Ctx, user: &dyn Fn(&str) -> bool) -> Vec<(String, Edit)> {
        let mut out: Vec<(String, Edit)> = Vec::new();
        if c.spaced && c.w == c.prev {
            out.push((String::new(), Edit::Repeated));
        }
        if let Some(set) = confusable_set(&c.w) {
            for a in set.iter().filter(|a| **a != c.w) {
                out.push((a.to_string(), Edit::Confused));
            }
        }
        let mut seen: HashSet<String> = out.iter().map(|(s, _)| s.clone()).collect();
        seen.insert(c.w.clone());
        for (s, e) in edits1(&c.w) {
            if !s.is_empty() && !s.starts_with('\'') && self.known(&s, user) && seen.insert(s.clone()) {
                out.push((s, e));
            }
        }
        out
    }

    fn sug_features(&self, c: &Ctx, cand: &str, e: Edit) -> [f32; SUG_N] {
        let mut f = [0.0; SUG_N];
        f[0] = 1.0;
        f[1] = self.counts.uni(cand);
        f[2] = self.counts.bi(&c.prev, cand);
        f[3] = self.counts.bi(cand, &c.next);
        f[4] = crate::kinds::closed_class(cand) as u8 as f32;
        f[5] = (cand.chars().next() == c.w.chars().next()) as u8 as f32;
        f[6] = cand.chars().count().abs_diff(c.w.chars().count()) as f32;
        f[7] = self.counts.uni.contains_key(cand) as u8 as f32;
        f[8] = (cand.chars().count() as f32).ln_1p();
        let k = EDITS.iter().position(|x| *x == e).unwrap_or(0);
        f[9 + k] = 1.0;
        f
    }

    fn sug_score(&self, f: &[f32; SUG_N]) -> f32 {
        f.iter().zip(self.sug.w.iter()).map(|(a, b)| a * b).sum()
    }

    fn suggest(&self, c: &Ctx, user: &dyn Fn(&str) -> bool, n: usize) -> Vec<Suggestion> {
        let cands = self.candidates(c, user);
        if cands.is_empty() {
            return Vec::new();
        }
        let scores: Vec<f32> = cands
            .iter()
            .map(|(s, e)| self.sug_score(&self.sug_features(c, s, *e)))
            .collect();
        let m = scores.iter().cloned().fold(f32::MIN, f32::max);
        let z: f32 = scores.iter().map(|s| (s - m).exp()).sum();
        let mut ranked: Vec<(usize, f32)> = scores.iter().enumerate().map(|(i, s)| (i, (s - m).exp() / z)).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked
            .into_iter()
            .take(n)
            .map(|(i, p)| Suggestion {
                text: recase(c.raw, &cands[i].0),
                p,
            })
            .collect()
    }

    /// the words of `text` the detector flags at `threshold`, with suggestions; `user`
    /// says whether a (lower case) word is one of the save's own
    pub fn check(&self, text: &str, user: &dyn Fn(&str) -> bool, threshold: f32) -> Vec<Flag> {
        let spans = words(text);
        let ctx = contexts(text, &spans);
        let mut out = Vec::new();
        for (s, c) in spans.iter().zip(&ctx) {
            if exempt(c.raw) {
                continue;
            }
            let p = self.det.score(&self.det_features(c, user));
            if p < threshold {
                continue;
            }
            let repeated = c.spaced && c.w == c.prev && !MAY_REPEAT.contains(&c.w.as_str());
            let kind = if repeated {
                "repeated"
            } else if !self.known(&c.w, user) {
                "spelling"
            } else {
                "confused"
            };
            let mut start = s.start;
            if repeated {
                // taking it out takes the space before it too
                start = text[..s.start].trim_end_matches([' ', '\t']).len();
            }
            out.push(Flag {
                start,
                end: s.end,
                from: text[..start].encode_utf16().count(),
                to: text[..s.end].encode_utf16().count(),
                word: c.raw.to_string(),
                p,
                suggestions: self.suggest(c, user, 3),
                kind,
            });
        }
        out
    }
}

/// a replacement written the way the word was: Capitalised or not
fn recase(raw: &str, s: &str) -> String {
    if raw.chars().next().is_some_and(char::is_uppercase) {
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().chain(c).collect(),
            None => String::new(),
        }
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------- corruptions

/// a sentence with errors put in: the new text and, for each word, what it was (None:
/// left alone; Some(""): a repeated word that should go)
pub fn corrupt(sentence: &str, seed: u64, lex: &dyn Fn(&str) -> bool) -> (String, Vec<Option<String>>) {
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let spans = words(sentence);
    let mut out = String::new();
    let mut labels = Vec::new();
    let mut at = 0;
    for s in &spans {
        out.push_str(&sentence[at..s.start]);
        let raw = &sentence[s.start..s.end];
        let w = lower(raw);
        at = s.end;
        if exempt(raw) {
            out.push_str(raw);
            labels.push(None);
            continue;
        }
        if let Some(set) = confusable_set(&w)
            && rng.chance(0.3)
        {
            let others: Vec<&&str> = set.iter().filter(|a| **a != w).collect();
            out.push_str(&recase(raw, others[rng.below(others.len())]));
            labels.push(Some(raw.to_string()));
            continue;
        }
        if w.chars().count() >= 3 && w.chars().all(|c| c.is_ascii_lowercase()) && rng.chance(0.12) {
            let mut done = None;
            for _ in 0..6 {
                let mut cs: Vec<char> = w.chars().collect();
                let i = rng.below(cs.len());
                match rng.below(5) {
                    0 if cs.len() >= 4 => {
                        cs.remove(i);
                    }
                    1 => cs.insert(i, cs[i]),
                    2 if i + 1 < cs.len() && cs[i] != cs[i + 1] => cs.swap(i, i + 1),
                    3 => {
                        let n = neighbours(cs[i]);
                        if n.is_empty() {
                            continue;
                        }
                        cs[i] = n[rng.below(n.len())];
                    }
                    4 => {
                        let n = neighbours(cs[i]);
                        if n.is_empty() {
                            continue;
                        }
                        cs.insert(i, n[rng.below(n.len())]);
                    }
                    _ => continue,
                }
                let s: String = cs.iter().collect();
                // a misspelling that is no word (real-word errors are the confusables')
                if s != w && !lex(&s) {
                    done = Some(s);
                    break;
                }
            }
            if let Some(s) = done {
                out.push_str(&recase(raw, &s));
                labels.push(Some(raw.to_string()));
                continue;
            }
        }
        out.push_str(raw);
        labels.push(None);
        if !MAY_REPEAT.contains(&w.as_str()) && rng.chance(0.015) {
            out.push(' ');
            out.push_str(raw);
            labels.push(Some(String::new()));
        }
    }
    out.push_str(&sentence[at..]);
    (out, labels)
}

// ---------------------------------------------------------------- training

/// the models, trained from the corpus the first time they're asked for
/// the clean sentences the checker learns from (the project's own prose)
pub fn corpus() -> &'static str {
    CORPUS
}

/// whether the bundled lexicon (`words.fc`) knows a lowercase word
pub fn is_word(w: &str) -> bool {
    static L: OnceLock<Lexicon> = OnceLock::new();
    L.get_or_init(Lexicon::load).words.contains(w)
}

pub fn model() -> &'static Model {
    static M: OnceLock<Model> = OnceLock::new();
    M.get_or_init(train)
}

fn train() -> Model {
    let lex = Lexicon::load();
    let sentences: Vec<&str> = CORPUS.lines().filter(|l| !l.trim().is_empty()).collect();
    type Numbered<'a> = Vec<(usize, &'a str)>;
    let (test, trainset): (Numbered, Numbered) = sentences.iter().copied().enumerate().partition(|(i, _)| i % 5 == 0);
    let mut counts = Counts::default();
    for (_, s) in &trainset {
        counts.add(s);
    }
    let mut m = Model {
        lex,
        counts,
        det: Detector {
            w: vec![0.0; 1 << DET_BITS],
        },
        sug: Suggester { w: [0.0; SUG_N] },
        report: Report::default(),
    };
    let none = |_: &str| false;
    // the corrupted training sentences, three ways each
    let mut det_ex: Vec<(Vec<u32>, bool)> = Vec::new();
    let mut sug_ex: Vec<([f32; SUG_N], bool)> = Vec::new();
    for (i, s) in &trainset {
        for k in 0..3u64 {
            let (text, labels) = corrupt(s, (*i as u64) * 7 + k, &|w| m.lex.words.contains(w));
            let spans = words(&text);
            let ctx = contexts(&text, &spans);
            for (c, label) in ctx.iter().zip(&labels) {
                if exempt(c.raw) {
                    continue;
                }
                det_ex.push((m.det_features(c, &none), label.is_some()));
                if let Some(orig) = label {
                    let orig = lower(orig);
                    let cands = m.candidates(c, &none);
                    if cands.iter().any(|(s, _)| *s == orig) {
                        for (cand, e) in &cands {
                            sug_ex.push((m.sug_features(c, cand, *e), *cand == orig));
                        }
                    }
                }
            }
        }
    }
    // the detector: SGD on the log loss, errors weighted up to balance them
    let pos = det_ex.iter().filter(|e| e.1).count().max(1);
    let neg = det_ex.len() - pos;
    let wpos = ((neg as f32 / pos as f32).sqrt()).max(1.0);
    let mut rng = Rng(0x5eed);
    let mut order: Vec<usize> = (0..det_ex.len()).collect();
    for epoch in 0..6 {
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i + 1));
        }
        let lr = 0.2 / (1.0 + epoch as f32);
        for &i in &order {
            let (f, y) = &det_ex[i];
            let p = m.det.score(f);
            let g = (p - *y as u8 as f32) * if *y { wpos } else { 1.0 };
            for &j in f {
                let w = &mut m.det.w[j as usize];
                *w -= lr * (g + 1e-5 * *w);
            }
        }
    }
    // the suggester: the same on its dense features
    let mut order: Vec<usize> = (0..sug_ex.len()).collect();
    for epoch in 0..8 {
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i + 1));
        }
        let lr = 0.05 / (1.0 + epoch as f32);
        for &i in &order {
            let (f, y) = &sug_ex[i];
            let p = sigmoid(m.sug_score(f));
            let g = p - *y as u8 as f32;
            for (w, x) in m.sug.w.iter_mut().zip(f.iter()) {
                *w -= lr * (g * x + 1e-4 * *w);
            }
        }
    }
    m.report = evaluate(
        &m,
        &test.iter().map(|(i, s)| (*i, *s)).collect::<Vec<_>>(),
        trainset.len(),
    );
    m
}

fn evaluate(m: &Model, test: &[(usize, &str)], train_n: usize) -> Report {
    let none = |_: &str| false;
    let (mut tp, mut fp, mut fneg, mut top1, mut top3) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut clean_words, mut clean_flags) = (0usize, 0usize);
    let mut kinds: Vec<(String, usize, usize)> = ["spelling", "confused", "repeated"]
        .iter()
        .map(|k| (k.to_string(), 0, 0))
        .collect();
    for (i, s) in test {
        let (text, labels) = corrupt(s, (*i as u64) * 7 + 1000, &|w| m.lex.words.contains(w));
        let spans = words(&text);
        let ctx = contexts(&text, &spans);
        for (c, label) in ctx.iter().zip(&labels) {
            if exempt(c.raw) {
                continue;
            }
            let flagged = m.det.score(&m.det_features(c, &none)) >= THRESHOLD;
            if let Some(orig) = label {
                let k = if orig.is_empty() {
                    2
                } else if confusable_set(&c.w).is_some_and(|set| set.contains(&lower(orig).as_str())) {
                    1
                } else {
                    0
                };
                kinds[k].2 += 1;
                kinds[k].1 += flagged as usize;
            }
            match (flagged, label) {
                (true, Some(orig)) => {
                    tp += 1;
                    let sug = m.suggest(c, &none, 3);
                    let orig = lower(orig);
                    if sug.first().is_some_and(|x| lower(&x.text) == orig) {
                        top1 += 1;
                    }
                    if sug.iter().any(|x| lower(&x.text) == orig) {
                        top3 += 1;
                    }
                }
                (true, None) => fp += 1,
                (false, Some(_)) => fneg += 1,
                (false, None) => {}
            }
        }
        clean_words += words(s).len();
        clean_flags += m.check(s, &none, THRESHOLD).len();
    }
    let precision = tp as f32 / (tp + fp).max(1) as f32;
    let recall = tp as f32 / (tp + fneg).max(1) as f32;
    Report {
        train_sentences: train_n,
        test_sentences: test.len(),
        precision,
        recall,
        f1: if precision + recall > 0.0 {
            2.0 * precision * recall / (precision + recall)
        } else {
            0.0
        },
        errors: tp + fneg,
        flagged: tp + fp,
        caught: tp,
        top1: top1 as f32 / tp.max(1) as f32,
        top3: top3 as f32 / tp.max(1) as f32,
        false_alarms_per_1000: 1000.0 * clean_flags as f32 / clean_words.max(1) as f32,
        threshold: THRESHOLD,
        by_kind: kinds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corruptions_are_deterministic_and_labelled() {
        let lex = |w: &str| ["the", "cat", "sat", "on", "mat", "their", "there"].contains(&w);
        let s = "The cat sat on the mat over there, and their hat too.";
        let a = corrupt(s, 3, &lex);
        assert_eq!(a, corrupt(s, 3, &lex));
        let n = words(&a.0).len();
        assert_eq!(a.1.len(), n);
        // over many seeds every kind of change shows up
        let all: Vec<_> = (0..200).map(|k| corrupt(s, k, &lex)).collect();
        assert!(all.iter().any(|(_, l)| l.iter().any(|x| x.as_deref() == Some(""))));
        assert!(
            all.iter()
                .any(|(t, _)| t.contains("there hat") || t.contains("they're hat"))
        );
        assert_eq!(words("it's a dog's life, isn't it"), {
            let t = "it's a dog's life, isn't it";
            let mut v = Vec::new();
            for w in ["it's", "a", "dog's", "life", "isn't", "it"] {
                let st = if w == "it" { t.rfind(w) } else { t.find(w) }.unwrap();
                v.push(Span {
                    start: st,
                    end: st + w.len(),
                });
            }
            v
        });
    }

    #[test]
    fn finds_and_fixes_errors_and_says_how_well() {
        let m = model();
        let r = &m.report;
        eprintln!("{r:?}");
        assert!(r.test_sentences > 400 && r.errors > 200, "{r:?}");
        // the measured numbers (STATUS.md has them); these are floors, not targets
        assert!(r.precision > 0.6, "{r:?}");
        assert!(r.recall > 0.6, "{r:?}");
        assert!(r.top3 > 0.6, "{r:?}");
        assert!(r.false_alarms_per_1000 < 40.0, "{r:?}");

        let none = |_: &str| false;
        let flags = m.check("I recieve the the letter and it was definately mine.", &none, THRESHOLD);
        let words: Vec<&str> = flags.iter().map(|f| f.word.as_str()).collect();
        assert!(words.contains(&"recieve") && words.contains(&"definately"), "{flags:?}");
        let rep = flags.iter().find(|f| f.kind == "repeated").expect("the repeated the");
        assert_eq!(rep.suggestions[0].text, "");
        let fix = flags.iter().find(|f| f.word == "recieve").unwrap();
        assert!(fix.suggestions.iter().any(|s| s.text == "receive"), "{fix:?}");
        // one's own words aren't errors
        let mine = |w: &str| w == "anendish";
        assert!(m.check("an anendish word", &mine, THRESHOLD).is_empty());
        // UTF-16 offsets for the browser
        let f = m.check("é recieve", &none, THRESHOLD);
        assert_eq!((f[0].from, f[0].to), (2, 9));
    }
}
