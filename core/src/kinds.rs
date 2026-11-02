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

/// every word type [`tag`] gives, in the order they're shown
pub const WORD_TYPES: [&str; 16] = [
    "noun",
    "verb",
    "adjective",
    "adverb",
    "pronoun",
    "determiner",
    "preposition",
    "conjunction",
    "interjection",
    "name",
    "number",
    "date",
    "time",
    "link",
    "email",
    "hashtag",
];

const PRONOUNS: &[&str] = &[
    "i",
    "me",
    "my",
    "mine",
    "myself",
    "you",
    "your",
    "yours",
    "yourself",
    "yourselves",
    "he",
    "him",
    "his",
    "himself",
    "she",
    "her",
    "hers",
    "herself",
    "it",
    "its",
    "itself",
    "we",
    "us",
    "our",
    "ours",
    "ourselves",
    "they",
    "them",
    "their",
    "theirs",
    "themselves",
    "who",
    "whom",
    "whose",
    "what",
    "which",
    "someone",
    "somebody",
    "something",
    "anyone",
    "anybody",
    "anything",
    "everyone",
    "everybody",
    "everything",
    "nobody",
    "nothing",
    "none",
    "one",
    "oneself",
    "whoever",
    "whatever",
    "i'm",
    "you're",
    "we're",
    "they're",
    "it's",
    "he's",
    "she's",
    "i've",
    "you've",
    "we've",
    "they've",
    "i'll",
    "you'll",
    "we'll",
    "they'll",
    "i'd",
];
const DETERMINERS: &[&str] = &[
    "the", "a", "an", "this", "that", "these", "those", "each", "every", "either", "neither", "some", "any", "no",
    "all", "both", "few", "many", "much", "more", "most", "several", "such", "another", "other", "enough", "less",
    "least", "own",
];
const PREPOSITIONS: &[&str] = &[
    "in",
    "on",
    "at",
    "by",
    "for",
    "with",
    "about",
    "against",
    "between",
    "into",
    "through",
    "during",
    "before",
    "after",
    "above",
    "below",
    "to",
    "from",
    "up",
    "down",
    "of",
    "off",
    "over",
    "under",
    "again",
    "further",
    "near",
    "across",
    "along",
    "among",
    "around",
    "behind",
    "beside",
    "beyond",
    "inside",
    "outside",
    "onto",
    "toward",
    "towards",
    "upon",
    "within",
    "without",
    "via",
    "per",
    "like",
    "unlike",
    "despite",
    "since",
    "until",
    "till",
    "amid",
    "past",
    "beneath",
    "throughout",
    "out",
];
const CONJUNCTIONS: &[&str] = &[
    "and", "or", "but", "nor", "so", "yet", "because", "although", "though", "while", "whereas", "if", "unless",
    "than", "whether", "as", "once", "when", "whenever", "where", "wherever", "lest",
];
const INTERJECTIONS: &[&str] = &[
    "oh", "ah", "wow", "hey", "hello", "hi", "alas", "oops", "ouch", "hmm", "yes", "no", "yeah", "ok", "okay", "bye",
    "goodbye", "thanks", "please", "hooray", "ugh", "aha", "eh", "huh",
];
const AUXILIARIES: &[&str] = &[
    "is",
    "am",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "have",
    "has",
    "had",
    "having",
    "do",
    "does",
    "did",
    "will",
    "would",
    "shall",
    "should",
    "can",
    "could",
    "may",
    "might",
    "must",
    "ought",
    "isn't",
    "aren't",
    "wasn't",
    "weren't",
    "don't",
    "doesn't",
    "didn't",
    "won't",
    "wouldn't",
    "can't",
    "cannot",
    "couldn't",
    "shouldn't",
    "hasn't",
    "haven't",
    "hadn't",
];
const VERBS: &[&str] = &[
    "say",
    "says",
    "said",
    "go",
    "goes",
    "went",
    "gone",
    "get",
    "gets",
    "got",
    "make",
    "makes",
    "made",
    "know",
    "knows",
    "knew",
    "known",
    "think",
    "thinks",
    "thought",
    "take",
    "takes",
    "took",
    "taken",
    "see",
    "sees",
    "saw",
    "seen",
    "come",
    "comes",
    "came",
    "want",
    "wants",
    "look",
    "looks",
    "use",
    "uses",
    "find",
    "finds",
    "found",
    "give",
    "gives",
    "gave",
    "given",
    "tell",
    "tells",
    "told",
    "work",
    "works",
    "call",
    "calls",
    "try",
    "tries",
    "ask",
    "asks",
    "need",
    "needs",
    "feel",
    "feels",
    "felt",
    "become",
    "becomes",
    "became",
    "leave",
    "leaves",
    "left",
    "put",
    "puts",
    "mean",
    "means",
    "meant",
    "keep",
    "keeps",
    "kept",
    "let",
    "lets",
    "begin",
    "begins",
    "began",
    "begun",
    "seem",
    "seems",
    "help",
    "helps",
    "talk",
    "talks",
    "turn",
    "turns",
    "start",
    "starts",
    "show",
    "shows",
    "shown",
    "hear",
    "hears",
    "heard",
    "play",
    "plays",
    "run",
    "runs",
    "ran",
    "move",
    "moves",
    "live",
    "lives",
    "believe",
    "believes",
    "hold",
    "holds",
    "held",
    "bring",
    "brings",
    "brought",
    "happen",
    "happens",
    "write",
    "writes",
    "wrote",
    "written",
    "sit",
    "sits",
    "sat",
    "stand",
    "stands",
    "stood",
    "lose",
    "loses",
    "lost",
    "pay",
    "pays",
    "paid",
    "meet",
    "meets",
    "met",
    "learn",
    "learns",
    "change",
    "changes",
    "lead",
    "leads",
    "led",
    "understand",
    "understood",
    "watch",
    "watches",
    "follow",
    "follows",
    "stop",
    "stops",
    "create",
    "creates",
    "speak",
    "speaks",
    "spoke",
    "spoken",
    "read",
    "reads",
    "allow",
    "allows",
    "add",
    "adds",
    "spend",
    "spends",
    "spent",
    "grow",
    "grows",
    "grew",
    "grown",
    "open",
    "opens",
    "walk",
    "walks",
    "win",
    "wins",
    "won",
    "offer",
    "offers",
    "remember",
    "remembers",
    "love",
    "loves",
    "consider",
    "appear",
    "appears",
    "buy",
    "buys",
    "bought",
    "wait",
    "waits",
    "serve",
    "die",
    "dies",
    "send",
    "sends",
    "sent",
    "expect",
    "build",
    "builds",
    "built",
    "stay",
    "stays",
    "fall",
    "falls",
    "fell",
    "fallen",
    "cut",
    "cuts",
    "reach",
    "kill",
    "remain",
    "remains",
    "suggest",
    "raise",
    "pass",
    "sell",
    "sold",
    "require",
    "decide",
    "pull",
    "carry",
    "carries",
    "sing",
    "sings",
    "sang",
    "sung",
    "jump",
    "jumps",
    "swim",
    "swims",
    "swam",
    "fly",
    "flies",
    "flew",
    "flown",
    "listen",
    "listens",
    "eat",
    "eats",
    "ate",
    "eaten",
    "drink",
    "drinks",
    "drank",
    "sleep",
    "sleeps",
    "slept",
    "sits",
    "dance",
    "dances",
    "draw",
    "draws",
    "drew",
    "drawn",
    "drive",
    "drives",
    "drove",
    "driven",
    "fight",
    "fights",
    "fought",
    "laugh",
    "laughs",
    "cry",
    "cries",
    "smile",
    "smiles",
    "shine",
    "shines",
    "shone",
    "howl",
    "howls",
    "sits",
    "lie",
    "lies",
    "lay",
    "rise",
    "rises",
    "rose",
    "risen",
    "carry",
    "teach",
    "teaches",
    "taught",
    "forget",
    "forgets",
    "forgot",
    "forgotten",
    "think",
    "wish",
    "wishes",
    "hope",
    "hopes",
    "know",
    "am",
    "seek",
    "seeks",
    "sought",
    "throw",
    "throws",
    "threw",
    "thrown",
    "catch",
    "catches",
    "caught",
    "break",
    "breaks",
    "broke",
    "broken",
    "choose",
    "chose",
    "chosen",
    "wear",
    "wears",
    "wore",
    "worn",
    "hide",
    "hides",
    "hid",
    "hidden",
    "ride",
    "rides",
    "rode",
    "ridden",
    "shake",
    "shook",
    "steal",
    "stole",
    "stolen",
    "swing",
    "swung",
    "wake",
    "woke",
    "woken",
    "tear",
    "tore",
    "torn",
    "sink",
    "sank",
    "sunk",
    "ring",
    "rang",
    "rung",
    "blow",
    "blew",
    "blown",
    "bite",
    "bit",
    "bitten",
    "hang",
    "hung",
    "dig",
    "dug",
    "feed",
    "fed",
    "flee",
    "fled",
    "freeze",
    "froze",
    "frozen",
    "kneel",
    "knelt",
    "lend",
    "lent",
    "light",
    "lit",
    "sew",
    "shoot",
    "shot",
    "shut",
    "slide",
    "slid",
    "spin",
    "spun",
    "spit",
    "spat",
    "split",
    "spread",
    "stick",
    "stuck",
    "sting",
    "stung",
    "strike",
    "struck",
    "swear",
    "swore",
    "sworn",
    "sweep",
    "swept",
    "weep",
    "wept",
    "wind",
    "wound",
    "carries",
    "walks",
    "runs",
    "think",
];
const ADJECTIVES: &[&str] = &[
    "good",
    "new",
    "first",
    "last",
    "long",
    "great",
    "little",
    "old",
    "right",
    "big",
    "high",
    "different",
    "small",
    "large",
    "next",
    "early",
    "young",
    "important",
    "public",
    "bad",
    "same",
    "able",
    "red",
    "blue",
    "green",
    "yellow",
    "black",
    "white",
    "brown",
    "grey",
    "gray",
    "pink",
    "purple",
    "orange",
    "quick",
    "slow",
    "lazy",
    "ripe",
    "sweet",
    "sour",
    "bitter",
    "hot",
    "cold",
    "warm",
    "cool",
    "dark",
    "bright",
    "light",
    "heavy",
    "soft",
    "hard",
    "fresh",
    "true",
    "false",
    "real",
    "full",
    "empty",
    "free",
    "clear",
    "deep",
    "wide",
    "narrow",
    "short",
    "tall",
    "low",
    "late",
    "happy",
    "sad",
    "kind",
    "wild",
    "quiet",
    "loud",
    "rich",
    "poor",
    "strong",
    "weak",
    "fine",
    "nice",
    "pretty",
    "ugly",
    "fast",
    "easy",
    "simple",
    "whole",
    "ready",
    "sure",
    "certain",
    "strange",
    "silent",
    "open",
    "close",
    "dead",
    "alive",
    "dry",
    "wet",
    "clean",
    "dirty",
    "safe",
    "calm",
    "brave",
    "proud",
    "gentle",
    "fierce",
    "ancient",
    "modern",
    "golden",
    "silver",
    "polished",
    "grand",
    "tiny",
    "huge",
    "vast",
    "main",
    "only",
    "best",
    "better",
    "worse",
    "worst",
    "own",
    "lovely",
];
const ADVERBS: &[&str] = &[
    "not",
    "very",
    "too",
    "also",
    "just",
    "now",
    "then",
    "here",
    "there",
    "always",
    "never",
    "often",
    "sometimes",
    "soon",
    "still",
    "already",
    "almost",
    "even",
    "only",
    "again",
    "away",
    "back",
    "far",
    "well",
    "quite",
    "rather",
    "really",
    "perhaps",
    "maybe",
    "together",
    "today",
    "tomorrow",
    "yesterday",
    "tonight",
    "ever",
    "once",
    "twice",
    "how",
    "why",
    "everywhere",
    "somewhere",
    "anywhere",
    "nowhere",
    "forward",
    "backward",
    "abroad",
    "indeed",
    "instead",
    "otherwise",
    "thus",
    "therefore",
    "however",
    "meanwhile",
    "anyway",
    "else",
    "ago",
    "soon",
    "seldom",
    "yet",
];
const NUMBER_WORDS: &[&str] = &[
    "zero", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "twenty",
    "thirty", "forty", "fifty", "hundred", "thousand", "million", "billion", "dozen",
];

/// a word of a text as [`tag`] reads it: its bytes in the text and its type
#[derive(Debug, Clone, PartialEq)]
pub struct Tagged {
    pub word: String,
    pub start: usize,
    pub end: usize,
    pub ty: &'static str,
}

/// the type of a word from its form alone, when its form tells
fn by_form(w: &str) -> Option<&'static str> {
    let l = w.to_lowercase();
    let l = l.as_str();
    let ends = |s: &[&str]| s.iter().any(|x| l.len() > x.len() + 2 && l.ends_with(x));
    if ends(&["ly"])
        && !matches!(
            l,
            "only" | "family" | "fly" | "ally" | "belly" | "jelly" | "holy" | "ugly"
        )
    {
        return Some("adverb");
    }
    if ends(&[
        "tion", "sion", "ness", "ment", "ity", "ism", "ship", "hood", "ance", "ence", "dom", "ist", "ure", "age",
    ]) {
        return Some("noun");
    }
    if ends(&["ous", "ful", "ive", "able", "ible", "ical", "less", "ish"]) {
        return Some("adjective");
    }
    if ends(&["ize", "ise", "ify", "ate", "ing", "ed", "en"]) {
        return Some("verb");
    }
    None
}

/// every word of a text with its type: a link, email, date, time, number or hashtag by its
/// pattern; a name by its capital (not the first word of a sentence); the closed classes
/// (pronouns, determiners, prepositions, conjunctions, interjections, auxiliaries) by
/// their lists; the rest by a small lexicon, then by their endings, then by what comes
/// before them (after a determiner or adjective: a noun; after a pronoun, `to` or an
/// auxiliary: a verb), else a noun. A tagger of rules, not a model: good enough to show
/// what a text is made of.
/// whether a (lower case) word is one of the closed classes: pronouns, determiners,
/// prepositions, conjunctions, auxiliaries (the common little words)
pub fn closed_class(w: &str) -> bool {
    [PRONOUNS, DETERMINERS, PREPOSITIONS, CONJUNCTIONS, AUXILIARIES]
        .iter()
        .any(|l| l.contains(&w))
}

pub fn tag(text: &str) -> Vec<Tagged> {
    let mut out: Vec<Tagged> = Vec::new();
    let mut sentence_start = true;
    let mut pos = 0;
    for raw in text.split_whitespace().take(400) {
        let at = text[pos..].find(raw).map_or(pos, |i| pos + i);
        pos = at + raw.len();
        let keep = is_link(raw) || is_email(raw);
        let w = if keep {
            raw.trim_end_matches(['.', ',', ';', ')', '!', '?'])
        } else {
            raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '#' && c != '-' && c != '+' && c != '\'')
                .trim_end_matches(['.', '-', '\''])
        };
        let ends_sentence = raw.ends_with(['.', '!', '?']) && !keep;
        if w.is_empty() {
            sentence_start |= ends_sentence;
            continue;
        }
        let start = at + raw.find(w).unwrap_or(0);
        let shape = detect(w);
        let l = w.to_lowercase();
        let ls = l.as_str();
        let prev = out.last().map(|t| t.ty);
        let prev_word = out.last().map(|t| t.word.to_lowercase()).unwrap_or_default();
        let ty: &'static str = match shape {
            "link" | "email" | "date" | "time" | "number" | "hashtag" => shape,
            _ if NUMBER_WORDS.contains(&ls) => "number",
            _ if PRONOUNS.contains(&ls) && !(ls == "one" && prev == Some("determiner")) => "pronoun",
            _ if DETERMINERS.contains(&ls) => "determiner",
            _ if AUXILIARIES.contains(&ls) => "verb",
            _ if ls == "to" && matches!(out.last(), Some(t) if t.ty == "verb") => "preposition",
            _ if PREPOSITIONS.contains(&ls) => "preposition",
            _ if CONJUNCTIONS.contains(&ls) => "conjunction",
            _ if INTERJECTIONS.contains(&ls) && (sentence_start || raw.ends_with([',', '!'])) => "interjection",
            _ if w.chars().next().is_some_and(char::is_uppercase) && !sentence_start => "name",
            _ => {
                let after_det = matches!(prev, Some("determiner") | Some("adjective"));
                let after_subject = matches!(prev, Some("pronoun"))
                    || prev_word == "to"
                    || matches!(prev, Some("verb") if AUXILIARIES.contains(&prev_word.as_str()));
                if VERBS.contains(&ls) && !after_det {
                    "verb"
                } else if ADJECTIVES.contains(&ls) {
                    "adjective"
                } else if ADVERBS.contains(&ls) {
                    "adverb"
                } else if after_subject && by_form(ls) != Some("noun") && by_form(ls) != Some("adverb") {
                    "verb"
                } else if after_det {
                    match by_form(ls) {
                        Some("adjective") => "adjective",
                        _ => "noun",
                    }
                } else if VERBS.contains(&ls) {
                    "verb"
                } else {
                    by_form(ls).unwrap_or(
                        if w.chars().next().is_some_and(char::is_uppercase) && text.split_whitespace().count() == 1 {
                            "name"
                        } else {
                            "noun"
                        },
                    )
                }
            }
        };
        out.push(Tagged {
            word: w.to_string(),
            start,
            end: start + w.len(),
            ty,
        });
        sentence_start = ends_sentence;
    }
    out
}

/// whether a word is English: the bundled lexicon's (`grammar`), or in the browser, where
/// the lexicon isn't built in, the word lists the types are told by
#[cfg(not(target_arch = "wasm32"))]
fn in_lexicon(w: &str) -> bool {
    crate::grammar::is_word(w)
}

#[cfg(target_arch = "wasm32")]
fn in_lexicon(w: &str) -> bool {
    [
        PREPOSITIONS,
        CONJUNCTIONS,
        AUXILIARIES,
        VERBS,
        ADJECTIVES,
        ADVERBS,
        NUMBER_WORDS,
        INTERJECTIONS,
    ]
    .iter()
    .any(|l| l.contains(&w))
        || by_form(w).is_some()
}

/// whether a text reads as English: most of its words (60 %) are in the bundled English
/// lexicon (or are capitalised, as names are), and a text of four words or more has one of
/// English's commonest words. Word types by English rules are only given
/// to texts that do, so other languages aren't labelled as though they were English.
pub fn looks_english(text: &str) -> bool {
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphabetic() && c != '\'')
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty())
        .take(200)
        .collect();
    if words.is_empty() {
        return true;
    }
    let known = words
        .iter()
        .filter(|w| {
            let l = w.to_lowercase();
            in_lexicon(&l)
                || PRONOUNS.contains(&l.as_str())
                || DETERMINERS.contains(&l.as_str())
                || w.chars().next().is_some_and(char::is_uppercase)
        })
        .count();
    // a sentence of English (four words or more) has some of its commonest words
    const COMMON: [&str; 32] = [
        "the", "a", "an", "of", "to", "in", "is", "are", "was", "were", "and", "or", "it", "that", "for", "on", "with",
        "as", "at", "by", "i", "you", "he", "she", "we", "they", "this", "be", "have", "not", "my", "his",
    ];
    let common = words.len() < 4 || words.iter().any(|w| COMMON.contains(&w.to_lowercase().as_str()));
    common && known * 10 >= words.len() * 6
}

/// each word of a text and its type ([`tag`])
pub fn word_types(text: &str) -> Vec<(String, &'static str)> {
    tag(text).into_iter().map(|t| (t.word, t.ty)).collect()
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
            if *k == "noun" || *k == "verb" {
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
        assert_eq!(k("Met"), Some("verb"), "{t:?}");
        assert_eq!(k("Ann"), Some("name"));
        assert_eq!(k("in"), Some("preposition"));
        assert_eq!(k("10:30"), Some("time"));
        assert_eq!(k("2026-10-19"), Some("date"));
        assert_eq!(k("12.50"), Some("number"));
        assert_eq!(k("https://x.org"), Some("link"));
        assert_eq!(k("#trip"), Some("hashtag"));
        assert_eq!(describe_words("  "), "");
        for (text, want) in [
            (
                "The red fox runs through the forest at night.",
                vec![
                    "determiner",
                    "adjective",
                    "noun",
                    "verb",
                    "preposition",
                    "determiner",
                    "noun",
                    "preposition",
                    "noun",
                ],
            ),
            (
                "She walks quickly to the old market.",
                vec![
                    "pronoun",
                    "verb",
                    "adverb",
                    "preposition",
                    "determiner",
                    "adjective",
                    "noun",
                ],
            ),
            ("We were singing loudly", vec!["pronoun", "verb", "verb", "adverb"]),
            (
                "a basket of apples and three plums",
                vec![
                    "determiner",
                    "noun",
                    "preposition",
                    "noun",
                    "conjunction",
                    "number",
                    "noun",
                ],
            ),
        ] {
            let got: Vec<&str> = word_types(text).into_iter().map(|x| x.1).collect();
            assert_eq!(got, want, "{text}");
        }
        // every type is one of the list, and the byte ranges point at the words
        let text = "Oh, the happiness of Rome's quiet streets!";
        for t in tag(text) {
            assert!(WORD_TYPES.contains(&t.ty), "{t:?}");
            assert_eq!(&text[t.start..t.end], t.word);
        }
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
