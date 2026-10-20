//! Starter classifiers whose correctness a machine can check.
//!
//! Each starter is a set of labelled texts where the label is known for certain:
//!
//! - **made by rules** (`Source::Rules`): the texts are generated, and the rule that made
//!   each one gives its class (even or odd numbers, the parity of a gematria sum, English
//!   or Afrikaans words, a clean or a corrupted sentence, a sentence or a fragment of one,
//!   fruit, stones or animals). Where a rule can be read back from the text alone,
//!   [`check`] does so, and the tests hold every generated text to it;
//! - **bundled public-domain texts** (`Source::Bundled`): the class is where the text
//!   came from (which author);
//! - **public-domain texts fetched when asked** (`Source::Fetched`): Project Gutenberg
//!   books, the class their language, checked against the book's own `Language:` line.
//!
//! Training texts and held-out texts never overlap: generated ones use separate seeds,
//! sentences from the corpus are split by their line number, bundled and fetched texts
//! by their place in the work. [`examples`] gives either split; the held-out score is a
//! trained classifier's accuracy on the held-out split (`classify.starter_score`).

use std::path::{Path, PathBuf};

use crate::graph::gematria::{Measure, Mode};

/// where a starter's texts come from
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Rules,
    Bundled,
    Fetched,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Starter {
    pub id: &'static str,
    pub title: &'static str,
    pub about: &'static str,
    pub classes: &'static [&'static str],
    pub source: Source,
    /// where the texts come from and under what terms
    pub licence: &'static str,
    /// whether its classifier gets the gematria input channels
    pub gematria: bool,
}

const RULES_LICENCE: &str = "made by this project's rules (core/src/starters.rs, MPL-2.0); the word lists are the project's own, and the English sentences come from core/src/grammar/corpus.txt, the project's own prose";

pub const STARTERS: &[Starter] = &[
    Starter {
        id: "even-odd",
        title: "even or odd",
        about: "numbers written in digits, by whether they are even",
        classes: &["even", "odd"],
        source: Source::Rules,
        licence: RULES_LICENCE,
        gematria: false,
    },
    Starter {
        id: "gematria-parity",
        title: "even or odd sum",
        about: "a few English words, by whether their ordinal gematria sum is even",
        classes: &["even sum", "odd sum"],
        source: Source::Rules,
        licence: RULES_LICENCE,
        gematria: true,
    },
    Starter {
        id: "english",
        title: "English or not",
        about: "runs of common words, English or Afrikaans",
        classes: &["English", "not English"],
        source: Source::Rules,
        licence: RULES_LICENCE,
        gematria: false,
    },
    Starter {
        id: "spelling",
        title: "spelled right or not",
        about: "sentences as written, or with words dropped, doubled, swapped, mistyped or confused (the spelling checker's corruptions)",
        classes: &["spelled right", "corrupted"],
        source: Source::Rules,
        licence: RULES_LICENCE,
        gematria: false,
    },
    Starter {
        id: "sentence",
        title: "sentence or fragment",
        about: "whole sentences, or their first few words cut off mid-way",
        classes: &["sentence", "fragment"],
        source: Source::Rules,
        licence: RULES_LICENCE,
        gematria: false,
    },
    Starter {
        id: "things",
        title: "fruit, stones and animals",
        about: "short phrases naming fruit, stones or animals",
        classes: &["fruit", "stones", "animals"],
        source: Source::Rules,
        licence: RULES_LICENCE,
        gematria: false,
    },
    Starter {
        id: "authors",
        title: "who wrote it",
        about: "passages of 300 to 600 characters by six authors",
        classes: &["Austen", "Chesterton", "Melville", "Milton", "Shakespeare", "Whitman"],
        source: Source::Bundled,
        licence: "public domain: Project Gutenberg texts as distributed with NLTK's gutenberg corpus, the Project Gutenberg headers and licence removed (core/corpus/authors/README.md)",
        gematria: false,
    },
    Starter {
        id: "languages",
        title: "which language",
        about: "paragraphs from four Project Gutenberg books in English, German, French and Dutch, fetched when asked",
        classes: &["English", "German", "French", "Dutch"],
        source: Source::Fetched,
        licence: "public domain in the USA: Project Gutenberg ebooks 1342 (Austen, Pride and Prejudice), 22367 (Kafka, Die Verwandlung), 17489 (Hugo, Les Misérables I), 11024 (Multatuli, Max Havelaar), fetched from gutenberg.org when asked, with the Project Gutenberg header, footer and licence taken off. Project Gutenberg asks readers outside the USA to check their own country's copyright law",
        gematria: false,
    },
];

pub fn get(id: &str) -> Option<&'static Starter> {
    STARTERS.iter().find(|s| s.id == id)
}

/// which texts
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    Train,
    Held,
}

/// generated texts per class: to train on, and held out
pub const TRAIN_PER_CLASS: usize = 80;
pub const HELD_PER_CLASS: usize = 100;

// ---------------------------------------------------------------- word lists

const ENGLISH: &[&str] = &[
    "the", "of", "and", "to", "a", "that", "it", "with", "as", "for", "his", "they", "be", "at", "one", "have", "this",
    "from", "or", "had", "by", "word", "but", "what", "some", "we", "can", "out", "other", "were", "all", "there",
    "when", "up", "use", "your", "how", "said", "an", "each", "she", "which", "do", "their", "time", "if", "will",
    "way", "about", "many", "then", "them", "write", "would", "like", "so", "these", "her", "long", "make", "thing",
    "see", "him", "two", "look", "more", "day", "could", "go", "come", "did", "number", "sound", "no", "most",
    "people", "over", "know", "water", "than", "call", "first", "who", "may", "down", "side", "been", "now", "find",
    "any", "new", "work", "part", "take", "get", "place", "made", "live", "where", "after", "back", "little", "only",
    "round", "man", "year", "came", "show", "every", "good", "me", "give", "our", "under", "name", "very", "through",
    "just", "form", "sentence", "great", "think", "say", "help", "low", "line", "differ", "turn", "cause", "much",
    "mean", "before", "move", "right", "boy", "old", "too", "same", "tell", "does", "set", "three", "want", "air",
    "well", "also", "play", "small", "end", "put", "home", "read", "hand", "port", "large", "spell", "add", "even",
    "land", "here", "must", "big", "high", "such", "follow", "act", "why", "ask", "men", "change", "went", "light",
    "kind", "off", "need", "house", "picture", "try", "us", "again", "animal", "point", "mother", "world", "near",
    "build", "self", "earth", "father", "head", "stand", "own", "page", "should", "country", "found", "answer",
    "school", "grow", "study", "still", "learn", "plant",
];

const AFRIKAANS: &[&str] = &[
    "die",
    "en",
    "nie",
    "het",
    "van",
    "ek",
    "jy",
    "wat",
    "hy",
    "sy",
    "ons",
    "hulle",
    "met",
    "vir",
    "op",
    "te",
    "om",
    "dit",
    "maar",
    "ook",
    "sal",
    "kan",
    "baie",
    "goed",
    "huis",
    "kind",
    "kinders",
    "boom",
    "hond",
    "kat",
    "mens",
    "mense",
    "dag",
    "nag",
    "tyd",
    "jaar",
    "werk",
    "skool",
    "kos",
    "eet",
    "drink",
    "loop",
    "sien",
    "gaan",
    "kom",
    "maak",
    "weet",
    "dink",
    "groot",
    "klein",
    "mooi",
    "nuut",
    "oud",
    "lekker",
    "vandag",
    "more",
    "gister",
    "altyd",
    "nooit",
    "miskien",
    "waar",
    "hoekom",
    "wanneer",
    "hier",
    "daar",
    "buite",
    "binne",
    "saam",
    "alleen",
    "vriend",
    "vriende",
    "moeder",
    "vader",
    "suster",
    "broer",
    "ouma",
    "oupa",
    "straat",
    "stad",
    "dorp",
    "plaas",
    "berg",
    "rivier",
    "see",
    "strand",
    "son",
    "maan",
    "wolk",
    "reën",
    "wind",
    "koud",
    "warm",
    "nat",
    "droog",
    "brood",
    "melk",
    "kaas",
    "vleis",
    "vis",
    "appel",
    "boek",
    "brief",
    "woord",
    "taal",
    "lied",
    "speel",
    "lees",
    "skryf",
    "praat",
    "luister",
    "slaap",
    "word",
    "wil",
    "moet",
    "mag",
    "gee",
    "neem",
    "vat",
    "bring",
    "soek",
    "vind",
    "verloor",
    "help",
    "vra",
    "antwoord",
    "dankie",
    "asseblief",
    "seker",
    "regtig",
    "gelukkig",
    "hartseer",
    "honger",
    "moeg",
    "vinnig",
    "stadig",
    "ver",
    "naby",
    "voor",
    "agter",
    "onder",
    "bo",
    "tussen",
    "sonder",
    "omdat",
    "terwyl",
    "nadat",
    "voordat",
    "alles",
    "niks",
    "iemand",
    "niemand",
    "elke",
    "party",
    "ander",
    "eerste",
    "laaste",
    "twee",
    "drie",
    "vier",
    "vyf",
    "ses",
    "sewe",
    "agt",
    "nege",
    "tien",
    "honderd",
];

const FRUIT: &[&str] = &[
    "apple",
    "pear",
    "plum",
    "peach",
    "apricot",
    "cherry",
    "grape",
    "mango",
    "papaya",
    "banana",
    "orange",
    "lemon",
    "lime",
    "grapefruit",
    "fig",
    "date",
    "quince",
    "melon",
    "watermelon",
    "strawberry",
    "raspberry",
    "blackberry",
    "blueberry",
    "gooseberry",
    "currant",
    "kiwi",
    "pineapple",
    "guava",
    "lychee",
    "pomegranate",
    "persimmon",
    "nectarine",
    "tangerine",
    "cranberry",
    "elderberry",
    "mulberry",
    "passion fruit",
    "starfruit",
    "durian",
];

const STONES: &[&str] = &[
    "granite",
    "basalt",
    "marble",
    "slate",
    "flint",
    "quartz",
    "shale",
    "sandstone",
    "limestone",
    "chalk",
    "obsidian",
    "pumice",
    "jasper",
    "agate",
    "onyx",
    "jade",
    "opal",
    "garnet",
    "ruby",
    "sapphire",
    "emerald",
    "topaz",
    "amethyst",
    "feldspar",
    "mica",
    "gneiss",
    "schist",
    "dolomite",
    "gypsum",
    "pyrite",
    "malachite",
    "turquoise",
    "lapis",
    "chert",
    "diorite",
    "gabbro",
    "andesite",
    "travertine",
    "soapstone",
];

const ANIMALS: &[&str] = &[
    "fox", "wolf", "bear", "badger", "otter", "beaver", "hare", "rabbit", "deer", "elk", "moose", "horse", "donkey",
    "goat", "sheep", "cow", "pig", "cat", "dog", "mouse", "rat", "squirrel", "hedgehog", "lion", "tiger", "leopard",
    "cheetah", "zebra", "giraffe", "elephant", "rhino", "hippo", "camel", "llama", "kangaroo", "koala", "owl", "heron",
    "sparrow", "eagle", "falcon", "crow", "duck", "goose", "swan", "salmon", "trout",
];

const ADJECTIVES: &[&str] = &[
    "a small",
    "a large",
    "the old",
    "a fine",
    "two",
    "three",
    "some",
    "a heavy",
    "the grey",
    "a dark",
    "a bright",
    "my",
    "your",
    "the last",
    "a single",
    "many",
    "a few",
    "the first",
    "one",
    "an odd",
];

// ---------------------------------------------------------------- generating

/// a deterministic stream of numbers from a seed (xorshift64*)
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0xd1b5_4a32_d192_ed03 | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

/// the words of one list that the other doesn't have ("die" is English and Afrikaans)
fn only<'a>(xs: &[&'a str], not: &[&str]) -> Vec<&'a str> {
    xs.iter().copied().filter(|w| !not.contains(w)).collect()
}

fn ordinal(text: &str) -> i64 {
    Measure::of(text).get(Mode::Ordinal)
}

/// the corpus's sentences, numbered by line
fn sentences() -> Vec<(usize, &'static str)> {
    crate::grammar::corpus()
        .lines()
        .enumerate()
        .filter(|(_, l)| l.split_whitespace().count() >= 6)
        .collect()
}

/// a corpus sentence for this split: every fifth line is held out
fn sentence(split: Split, i: usize) -> &'static str {
    let all = sentences();
    let mine: Vec<&str> = all
        .iter()
        .filter(|(n, _)| (n % 5 == 0) == (split == Split::Held))
        .map(|(_, s)| *s)
        .collect();
    mine[i % mine.len()]
}

/// one generated text of `class` (the i-th of its split)
fn generate(id: &str, split: Split, class: usize, i: usize) -> String {
    let seed = (i as u64) + if split == Split::Held { 1 << 32 } else { 0 } + ((class as u64) << 40);
    let mut r = Rng::new(seed ^ crate::classify::math::key(&[id.len() as u64, id.as_bytes()[0] as u64]));
    match id {
        "even-odd" => {
            let digits = 1 + r.below(7);
            let mut s: String = (0..digits - 1)
                .map(|k| (b'0' + if k == 0 { 1 + r.below(9) } else { r.below(10) } as u8) as char)
                .collect();
            let last = 2 * r.below(5) + class;
            s.push((b'0' + last as u8) as char);
            s
        }
        "gematria-parity" => loop {
            let n = 1 + r.below(3);
            let s = (0..n).map(|_| r.pick(ENGLISH)).collect::<Vec<_>>().join(" ");
            if (ordinal(&s).rem_euclid(2)) as usize == class {
                break s;
            }
        },
        "english" => {
            let words = if class == 0 {
                only(ENGLISH, AFRIKAANS)
            } else {
                only(AFRIKAANS, ENGLISH)
            };
            let n = 4 + r.below(6);
            (0..n).map(|_| r.pick(&words)).collect::<Vec<_>>().join(" ")
        }
        "spelling" => {
            let s = sentence(split, i);
            if class == 0 {
                return s.to_string();
            }
            // corrupted until it differs (a corruption can come out as the same text)
            (0..)
                .map(|k| crate::grammar::corrupt(s, seed.wrapping_add(k), &crate::grammar::is_word).0)
                .find(|c| c != s)
                .expect("some corruption changes it")
        }
        "sentence" => {
            let s = sentence(split, i);
            if class == 0 {
                return s.to_string();
            }
            let words: Vec<&str> = s.split_whitespace().collect();
            let k = 2 + r.below(words.len() - 3);
            let cut = words[..k].join(" ");
            cut.trim_end_matches(|c: char| c.is_ascii_punctuation()).to_string()
        }
        "things" => {
            let list = [FRUIT, STONES, ANIMALS][class];
            let n = 1 + r.below(3);
            let names: Vec<String> = (0..n)
                .map(|_| {
                    let w = r.pick(list);
                    if r.below(2) == 0 {
                        format!("{} {w}", r.pick(ADJECTIVES))
                    } else {
                        w.to_string()
                    }
                })
                .collect();
            match names.len() {
                1 => names[0].clone(),
                _ => format!("{} and {}", names[..names.len() - 1].join(", "), names[names.len() - 1]),
            }
        }
        _ => unreachable!("a rule starter"),
    }
}

/// a text's class read back from the text by the rule alone, where the rule allows it
/// (None for starters whose class only the making of the text knows)
pub fn check(id: &str, text: &str) -> Option<usize> {
    match id {
        "even-odd" => {
            let d = text.trim().bytes().last()?;
            d.is_ascii_digit().then(|| ((d - b'0') % 2) as usize)
        }
        "gematria-parity" => Some(ordinal(text).rem_euclid(2) as usize),
        "english" => {
            let words: Vec<&str> = text.split_whitespace().collect();
            let en = only(ENGLISH, AFRIKAANS);
            let af = only(AFRIKAANS, ENGLISH);
            if words.iter().all(|w| en.contains(w)) {
                Some(0)
            } else if words.iter().all(|w| af.contains(w)) {
                Some(1)
            } else {
                None
            }
        }
        "things" => {
            let lists = [FRUIT, STONES, ANIMALS];
            let found: Vec<usize> = (0..3)
                .filter(|&c| {
                    lists[c]
                        .iter()
                        .any(|w| text.split([' ', ',']).any(|t| t == *w) || (w.contains(' ') && text.contains(w)))
                })
                .collect();
            (found.len() == 1).then(|| found[0])
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- bundled: the authors

const AUTHORS: [&str; 6] = [
    include_str!("../corpus/authors/austen.txt"),
    include_str!("../corpus/authors/chesterton.txt"),
    include_str!("../corpus/authors/melville.txt"),
    include_str!("../corpus/authors/milton.txt"),
    include_str!("../corpus/authors/shakespeare.txt"),
    include_str!("../corpus/authors/whitman.txt"),
];

/// passages of 300 to 600 characters, cut at spaces, from a stretch of text
fn passages(text: &str, n: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while out.len() < n && i < words.len() {
        let mut s = String::new();
        while i < words.len() && s.len() < 300 + 60 * (out.len() % 6) {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(words[i]);
            i += 1;
        }
        if s.len() >= 300 {
            out.push(s);
        }
    }
    out
}

/// the authors' passages: regions 0–7 of each author's ten to train on, 8–9 held out
fn authors(split: Split) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for (class, text) in AUTHORS.iter().enumerate() {
        for (k, region) in text.split("* * *").enumerate() {
            if (k >= 8) == (split == Split::Held) {
                out.extend(passages(region, 8).into_iter().map(|p| (p, class)));
            }
        }
    }
    out
}

// ---------------------------------------------------------------- fetched: Gutenberg books

/// the language starter's books: (Gutenberg ebook number, the language it says it is in)
pub const BOOKS: [(u32, &str); 4] = [
    (1342, "English"),
    (22367, "German"),
    (17489, "French"),
    (11024, "Dutch"),
];

pub fn book_url(n: u32) -> String {
    format!("https://www.gutenberg.org/cache/epub/{n}/pg{n}.txt")
}

fn book_path(dir: &Path, n: u32) -> PathBuf {
    dir.join(format!("gutenberg-{n}.txt"))
}

/// the book's text without Project Gutenberg's header, footer and licence, after
/// checking it says it is in `language`
pub fn book_body(raw: &str, language: &str) -> Result<String, String> {
    let said = raw
        .lines()
        .take(80)
        .find_map(|l| l.trim().strip_prefix("Language:"))
        .map(str::trim)
        .ok_or("the file has no Language: line, so it isn't a Project Gutenberg book")?;
    if said != language {
        return Err(format!("the book says it is in {said}, not {language}"));
    }
    let start = raw
        .find("*** START OF")
        .and_then(|i| raw[i..].find('\n').map(|j| i + j + 1))
        .ok_or("no *** START OF line")?;
    let end = raw[start..].find("*** END OF").map_or(raw.len(), |j| start + j);
    Ok(raw[start..end].to_string())
}

/// paragraphs of a book: every fifth held out
fn book_paragraphs(body: &str, split: Split, n: usize) -> Vec<String> {
    let normal = body.replace("\r\n", "\n");
    normal
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| (200..=1200).contains(&p.len()))
        .enumerate()
        .filter(|(k, _)| (k % 5 == 0) == (split == Split::Held))
        .map(|(_, p)| p)
        .take(n)
        .collect()
}

/// whether the books are in `dir`
pub fn fetched(dir: &Path) -> bool {
    BOOKS.iter().all(|(n, _)| book_path(dir, *n).exists())
}

/// fetches the books that aren't in `dir` yet with `get` (a URL to its bytes), checking
/// each is in the language it should be before keeping it
pub fn fetch(dir: &Path, get: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> Result<usize, String> {
    let mut got = 0;
    for (n, lang) in BOOKS {
        let path = book_path(dir, n);
        if path.exists() {
            continue;
        }
        let bytes = get(&book_url(n))?;
        let raw = String::from_utf8_lossy(&bytes);
        book_body(&raw, lang).map_err(|e| format!("ebook {n}: {e}"))?;
        crate::vfs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(&path, bytes.as_slice()).map_err(|e| format!("couldn't keep ebook {n}: {e}"))?;
        got += 1;
    }
    Ok(got)
}

/// a starter's labelled texts for a split; `dir` holds fetched books
pub fn examples(id: &str, split: Split, dir: Option<&Path>) -> Result<Vec<(String, usize)>, String> {
    let s = get(id).ok_or_else(|| format!("no starter {id:?}"))?;
    Ok(match s.source {
        Source::Rules => {
            let train: Vec<(String, usize)> = (0..TRAIN_PER_CLASS)
                .flat_map(|i| (0..s.classes.len()).map(move |c| (generate(id, Split::Train, c, i), c)))
                .collect();
            if split == Split::Train {
                return Ok(train);
            }
            // held-out texts are new ones: a short generated text (a one-digit number, a
            // single word) can come out the same as a training text, and is skipped
            let mut seen: std::collections::HashSet<String> = train.into_iter().map(|(t, _)| t).collect();
            let mut per: Vec<Vec<String>> = vec![Vec::new(); s.classes.len()];
            for (c, got) in per.iter_mut().enumerate() {
                let mut i = 0;
                while got.len() < HELD_PER_CLASS {
                    if i > 100 * HELD_PER_CLASS {
                        return Err(format!("{id}: not enough new texts to hold out"));
                    }
                    let t = generate(id, Split::Held, c, i);
                    i += 1;
                    if seen.insert(t.clone()) {
                        got.push(t);
                    }
                }
            }
            (0..HELD_PER_CLASS)
                .flat_map(|i| per.iter().enumerate().map(move |(c, v)| (v[i].clone(), c)))
                .collect()
        }
        Source::Bundled => authors(split),
        Source::Fetched => {
            let dir = dir.ok_or("fetched texts need a folder")?;
            if !fetched(dir) {
                return Err("fetch its books first".into());
            }
            let per = if split == Split::Held { 30 } else { 80 };
            let mut out = Vec::new();
            for (class, (n, lang)) in BOOKS.iter().enumerate() {
                let raw = std::fs::read(book_path(dir, *n)).map_err(|e| e.to_string())?;
                let body = book_body(&String::from_utf8_lossy(&raw), lang)?;
                out.extend(book_paragraphs(&body, split, per).into_iter().map(|p| (p, class)));
            }
            out
        }
    })
}

/// a URL's bytes over HTTPS (the desktop and the hub; the cargo feature `fetch`)
#[cfg(feature = "fetch")]
pub fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let mut r = ureq::get(url)
        .call()
        .map_err(|e| format!("couldn't fetch {url}: {e}"))?;
    r.body_mut()
        .with_config()
        .limit(64 << 20)
        .read_to_vec()
        .map_err(|e| format!("couldn't read {url}: {e}"))
}

#[cfg(not(feature = "fetch"))]
pub fn http_get(url: &str) -> Result<Vec<u8>, String> {
    Err(format!("this build can't fetch {url} (the desktop and the hub can)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_starters_make_what_their_rules_say() {
        for s in STARTERS.iter().filter(|s| s.source == Source::Rules) {
            let train = examples(s.id, Split::Train, None).unwrap();
            let held = examples(s.id, Split::Held, None).unwrap();
            assert_eq!(train.len(), TRAIN_PER_CLASS * s.classes.len(), "{}", s.id);
            assert_eq!(held.len(), HELD_PER_CLASS * s.classes.len(), "{}", s.id);
            for (t, c) in train.iter().chain(&held) {
                assert!(!t.trim().is_empty(), "{}", s.id);
                if let Some(k) = check(s.id, t) {
                    assert_eq!(k, *c, "{}: {t:?}", s.id);
                } else {
                    assert!(
                        ["spelling", "sentence"].contains(&s.id),
                        "{}: the rule can't read {t:?}",
                        s.id
                    );
                }
            }
            // the same every time
            assert_eq!(train, examples(s.id, Split::Train, None).unwrap(), "{}", s.id);
            // nothing held out was trained on
            let seen: std::collections::HashSet<&str> = train.iter().map(|(t, _)| t.as_str()).collect();
            assert!(held.iter().all(|(t, _)| !seen.contains(t.as_str())), "{}", s.id);
        }
    }

    #[test]
    fn spelling_and_fragments_differ_from_their_sentences() {
        for i in 0..50 {
            assert_ne!(
                generate("spelling", Split::Train, 0, i),
                generate("spelling", Split::Train, 1, i)
            );
            let whole = generate("sentence", Split::Train, 0, i);
            let cut = generate("sentence", Split::Train, 1, i);
            let first = cut.split_whitespace().next().unwrap();
            assert!(
                cut.len() < whole.len() && whole.starts_with(first),
                "{cut:?} of {whole:?}"
            );
        }
    }

    #[test]
    fn authors_are_held_out_by_region() {
        let train = authors(Split::Train);
        let held = authors(Split::Held);
        for c in 0..6 {
            assert!(train.iter().filter(|e| e.1 == c).count() >= 40, "author {c}");
            assert!(held.iter().filter(|e| e.1 == c).count() >= 8, "author {c}");
        }
        assert!(held.iter().all(|h| !train.contains(h)));
    }

    #[test]
    fn books_are_checked_and_stripped() {
        let raw = "The Project Gutenberg eBook of A Book\n\nLanguage: German\n\n*** START OF THE PROJECT GUTENBERG EBOOK A BOOK ***\n\nErster Absatz.\n\n*** END OF THE PROJECT GUTENBERG EBOOK A BOOK ***\nlicence text";
        assert_eq!(book_body(raw, "German").unwrap().trim(), "Erster Absatz.");
        assert!(book_body(raw, "French").unwrap_err().contains("German"));
        assert!(book_body("no header", "German").is_err());
        // fetching keeps only what passes the check
        let dir = std::env::temp_dir().join(format!("anend-books-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let wrong = |_: &str| Ok::<_, String>(raw.as_bytes().to_vec());
        assert!(fetch(&dir, &wrong).is_err());
        assert!(!fetched(&dir));
        let para = |lang: &str| {
            let p: String = (0..40).map(|i| format!("{lang} word{i} ")).collect();
            format!(
                "Language: {lang}\n*** START OF IT ***\n{}\n*** END OF IT ***\n",
                [p.as_str(); 12].join("\n\n")
            )
        };
        let right = |url: &str| {
            let (_, lang) = BOOKS.iter().find(|(n, _)| url == book_url(*n)).unwrap();
            Ok::<_, String>(para(lang).into_bytes())
        };
        assert_eq!(fetch(&dir, &right).unwrap(), 4);
        assert!(fetched(&dir));
        let held = examples("languages", Split::Held, Some(&dir)).unwrap();
        let train = examples("languages", Split::Train, Some(&dir)).unwrap();
        assert_eq!(held.iter().filter(|e| e.1 == 2).count(), 3);
        assert_eq!(train.iter().filter(|e| e.1 == 3).count(), 9);
        assert!(train.iter().all(|(t, c)| t.starts_with(BOOKS[*c].1)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
