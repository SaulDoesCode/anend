//! A BPE tokenizer read from a HuggingFace `tokenizer.json`, for pretrained models
//! (`pretrained`). The engine's own models read bytes and keep doing so; this is only
//! the vocabulary a pretrained checkpoint was trained with.
//!
//! Supported, which covers the Llama family's tokenizers (GPT-2 style byte-level BPE as
//! in Llama 3, Qwen2/3, SmolLM, Mistral Nemo; SentencePiece style BPE with byte
//! fallback as in Llama 2, TinyLlama, Mistral 7B):
//!
//! - `model`: `BPE` with `vocab`, `merges` (as `"a b"` strings or `["a", "b"]` pairs),
//!   `byte_fallback`, `unk_token`, `ignore_merges`;
//! - `added_tokens` (special or not; `lstrip`/`rstrip`), matched in the raw text first;
//! - `normalizer`: `Sequence`, `Prepend`, `Replace` (a string pattern), `Lowercase`,
//!   `Strip`, and `NFC` (taken as already composed: text in decomposed form may split
//!   differently than the reference implementation);
//! - `pre_tokenizer`: `Sequence`, `ByteLevel` (with or without its GPT-2 regex and
//!   prefix space), `Split` (regex or string; isolated, removed, merged with the
//!   previous or the next), `Digits`, `Metaspace`;
//! - `decoder`: `ByteLevel`, `Metaspace`, or a `Sequence` of `Replace`, `ByteFallback`,
//!   `Fuse` and `Strip`.
//!
//! Anything else (WordPiece or Unigram models, other normalizers, BPE dropout, word
//! prefixes and suffixes) is refused with its name. Regexes go through `pattern`.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use serde_json::Value as Json;

use super::pattern::{self, Regex};

#[derive(Clone, Debug, PartialEq)]
enum Normalizer {
    Prepend(String),
    Replace(String, String),
    Lowercase,
    Strip(bool, bool),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Behavior {
    Isolated,
    Removed,
    MergedWithPrevious,
    MergedWithNext,
}

#[derive(Clone, Debug, PartialEq)]
enum Pre {
    ByteLevel {
        prefix_space: bool,
        regex: Option<Regex>,
    },
    Split {
        regex: Regex,
        behavior: Behavior,
        invert: bool,
    },
    Digits {
        individual: bool,
    },
    Metaspace {
        replacement: char,
        prepend: Prepend,
        split: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Prepend {
    Always,
    First,
    Never,
}

#[derive(Clone, Debug, PartialEq)]
struct Added {
    id: u32,
    content: String,
    special: bool,
    lstrip: bool,
    rstrip: bool,
}

/// a tokenizer: text to token ids and back
#[derive(Clone, Debug, PartialEq)]
pub struct Tokenizer {
    /// the `tokenizer.json` it was read from (saved with models, read again on load)
    source: String,
    /// id → its piece as written in the vocabulary
    pieces: Vec<String>,
    ids: HashMap<String, u32>,
    /// (left, right) → (rank, merged)
    merges: HashMap<(u32, u32), (u32, u32)>,
    added: Vec<Added>,
    /// id → index in `added`
    added_of: HashMap<u32, usize>,
    normalizers: Vec<Normalizer>,
    pre: Vec<Pre>,
    byte_level: bool,
    byte_fallback: bool,
    /// `<0xAB>` tokens
    byte_ids: Vec<Option<u32>>,
    unk: Option<u32>,
    ignore_merges: bool,
    /// decoding: the metaspace character read as a space, and leading spaces stripped
    space_char: Option<char>,
    strip_start: usize,
}

/// GPT-2's byte ↔ printable character table: bytes that print map to themselves,
/// the rest to 256, 257, … in order
pub fn byte_chars() -> [char; 256] {
    let mut out = ['\0'; 256];
    let mut n = 0;
    for b in 0..256u32 {
        let keep = (33..=126).contains(&b) || (161..=172).contains(&b) || (174..=255).contains(&b);
        out[b as usize] = if keep {
            char::from_u32(b).unwrap()
        } else {
            n += 1;
            char::from_u32(255 + n).unwrap()
        };
    }
    out
}

fn str_of(v: &Json, what: &str) -> Result<String, String> {
    v.as_str()
        .map(String::from)
        .ok_or(format!("tokenizer.json: {what} is not a string"))
}

fn kind(v: &Json) -> &str {
    v["type"].as_str().unwrap_or("")
}

fn pattern_of(v: &Json, what: &str) -> Result<Regex, String> {
    if let Some(s) = v["String"].as_str() {
        let escaped: String = s
            .chars()
            .flat_map(|c| {
                let special = "\\.+*?()|[]{}^$".contains(c);
                special.then_some('\\').into_iter().chain([c])
            })
            .collect();
        Regex::new(&escaped)
    } else if let Some(s) = v["Regex"].as_str() {
        Regex::new(s).map_err(|e| format!("tokenizer.json: {what}: {e}"))
    } else {
        Err(format!("tokenizer.json: {what}: a pattern is a String or a Regex"))
    }
}

fn normalizers(v: &Json, out: &mut Vec<Normalizer>) -> Result<(), String> {
    match kind(v) {
        "" if v.is_null() => {}
        "Sequence" => {
            for n in v["normalizers"]
                .as_array()
                .ok_or("tokenizer.json: normalizer Sequence without a list")?
            {
                normalizers(n, out)?;
            }
        }
        "Prepend" => out.push(Normalizer::Prepend(str_of(&v["prepend"], "Prepend.prepend")?)),
        "Replace" => {
            let p = v["pattern"]["String"]
                .as_str()
                .ok_or("tokenizer.json: normalizer Replace with a regex pattern is not supported")?;
            out.push(Normalizer::Replace(p.into(), str_of(&v["content"], "Replace.content")?));
        }
        "Lowercase" => out.push(Normalizer::Lowercase),
        "Strip" => out.push(Normalizer::Strip(
            v["strip_left"].as_bool().unwrap_or(true),
            v["strip_right"].as_bool().unwrap_or(true),
        )),
        // composed text is the common case; see the module doc
        "NFC" => {}
        other => return Err(format!("tokenizer.json: normalizer {other:?} is not supported")),
    }
    Ok(())
}

fn prepend_of(v: &Json) -> Prepend {
    match v["prepend_scheme"].as_str() {
        Some("always") => Prepend::Always,
        Some("first") => Prepend::First,
        Some("never") => Prepend::Never,
        _ => {
            if v["add_prefix_space"].as_bool().unwrap_or(true) {
                Prepend::Always
            } else {
                Prepend::Never
            }
        }
    }
}

fn metaspace_char(v: &Json) -> Result<char, String> {
    let s = str_of(&v["replacement"], "Metaspace.replacement")?;
    let mut c = s.chars();
    match (c.next(), c.next()) {
        (Some(x), None) => Ok(x),
        _ => Err("tokenizer.json: Metaspace.replacement must be one character".into()),
    }
}

fn pre_tokenizers(v: &Json, out: &mut Vec<Pre>) -> Result<(), String> {
    match kind(v) {
        "" if v.is_null() => {}
        "Sequence" => {
            for p in v["pretokenizers"]
                .as_array()
                .ok_or("tokenizer.json: pre_tokenizer Sequence without a list")?
            {
                pre_tokenizers(p, out)?;
            }
        }
        "ByteLevel" => out.push(Pre::ByteLevel {
            prefix_space: v["add_prefix_space"].as_bool().unwrap_or(false),
            regex: if v["use_regex"].as_bool().unwrap_or(true) {
                Some(Regex::new(pattern::GPT2)?)
            } else {
                None
            },
        }),
        "Split" => {
            let behavior = match v["behavior"].as_str().unwrap_or("Isolated") {
                "Isolated" => Behavior::Isolated,
                "Removed" => Behavior::Removed,
                "MergedWithPrevious" => Behavior::MergedWithPrevious,
                "MergedWithNext" => Behavior::MergedWithNext,
                other => return Err(format!("tokenizer.json: Split behavior {other:?} is not supported")),
            };
            out.push(Pre::Split {
                regex: pattern_of(&v["pattern"], "Split.pattern")?,
                behavior,
                invert: v["invert"].as_bool().unwrap_or(false),
            });
        }
        "Digits" => out.push(Pre::Digits {
            individual: v["individual_digits"].as_bool().unwrap_or(false),
        }),
        "Metaspace" => out.push(Pre::Metaspace {
            replacement: metaspace_char(v)?,
            prepend: prepend_of(v),
            split: v["split"].as_bool().unwrap_or(true),
        }),
        other => return Err(format!("tokenizer.json: pre_tokenizer {other:?} is not supported")),
    }
    Ok(())
}

/// splits `s` at the matches of a pattern, by `behavior`
fn split_by(s: &str, ranges: &[(usize, usize)], behavior: Behavior, invert: bool) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    // (piece, is_match) covering the whole string
    let mut parts: Vec<(String, bool)> = Vec::new();
    let mut at = 0;
    for &(a, b) in ranges {
        if a > at {
            parts.push((chars[at..a].iter().collect(), false));
        }
        parts.push((chars[a..b].iter().collect(), true));
        at = b;
    }
    if at < chars.len() {
        parts.push((chars[at..].iter().collect(), false));
    }
    if invert {
        parts.iter_mut().for_each(|p| p.1 = !p.1);
    }
    let mut out: Vec<String> = Vec::new();
    match behavior {
        Behavior::Isolated => out = parts.into_iter().map(|p| p.0).collect(),
        Behavior::Removed => out = parts.into_iter().filter(|p| !p.1).map(|p| p.0).collect(),
        Behavior::MergedWithPrevious => {
            for (p, m) in parts {
                match out.last_mut() {
                    Some(last) if m => last.push_str(&p),
                    _ => out.push(p),
                }
            }
        }
        Behavior::MergedWithNext => {
            let mut pending = String::new();
            for (p, m) in parts {
                if m {
                    if !pending.is_empty() {
                        out.push(std::mem::take(&mut pending));
                    }
                    pending = p;
                } else {
                    pending.push_str(&p);
                    out.push(std::mem::take(&mut pending));
                }
            }
            if !pending.is_empty() {
                out.push(pending);
            }
        }
    }
    out.retain(|p| !p.is_empty());
    out
}

impl Tokenizer {
    /// reads a `tokenizer.json` file
    pub fn load(path: &std::path::Path) -> Result<Tokenizer, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Tokenizer::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_json(text: &str) -> Result<Tokenizer, String> {
        let v: Json = serde_json::from_str(text).map_err(|e| format!("tokenizer.json: {e}"))?;
        let m = &v["model"];
        match kind(m) {
            "BPE" => {}
            "" if m["merges"].is_array() => {}
            other => {
                return Err(format!(
                    "tokenizer.json: model type {other:?} is not supported (BPE only)"
                ));
            }
        }
        if m["dropout"].as_f64().is_some_and(|d| d > 0.0) {
            return Err("tokenizer.json: BPE dropout is not supported (the engine is deterministic)".into());
        }
        for f in ["continuing_subword_prefix", "end_of_word_suffix"] {
            if m[f].as_str().is_some_and(|s| !s.is_empty()) {
                return Err(format!("tokenizer.json: model.{f} is not supported"));
            }
        }
        let vocab = m["vocab"]
            .as_object()
            .ok_or("tokenizer.json: model.vocab is not an object")?;
        let mut ids: HashMap<String, u32> = HashMap::with_capacity(vocab.len());
        let mut size = 0usize;
        for (p, id) in vocab {
            let id = id.as_u64().ok_or(format!("tokenizer.json: vocab id of {p:?}"))?;
            let id = u32::try_from(id).map_err(|_| format!("tokenizer.json: vocab id {id} too large"))?;
            ids.insert(p.clone(), id);
            size = size.max(id as usize + 1);
        }
        let mut added = Vec::new();
        for a in v["added_tokens"].as_array().into_iter().flatten() {
            let id = a["id"].as_u64().ok_or("tokenizer.json: an added token without an id")? as u32;
            size = size.max(id as usize + 1);
            added.push(Added {
                id,
                content: str_of(&a["content"], "added token content")?,
                special: a["special"].as_bool().unwrap_or(false),
                lstrip: a["lstrip"].as_bool().unwrap_or(false),
                rstrip: a["rstrip"].as_bool().unwrap_or(false),
            });
        }
        let mut pieces = vec![String::new(); size];
        for (p, &id) in &ids {
            pieces[id as usize] = p.clone();
        }
        for a in &added {
            pieces[a.id as usize] = a.content.clone();
            ids.entry(a.content.clone()).or_insert(a.id);
        }
        // longest first, so a token that starts another one doesn't hide it
        added.sort_by(|a, b| b.content.len().cmp(&a.content.len()).then(a.id.cmp(&b.id)));
        let added_of = added.iter().enumerate().map(|(i, a)| (a.id, i)).collect();
        let mut merges = HashMap::new();
        for (rank, mg) in m["merges"]
            .as_array()
            .ok_or("tokenizer.json: model.merges is not a list")?
            .iter()
            .enumerate()
        {
            let (a, b) = if let Some(s) = mg.as_str() {
                let (a, b) = s
                    .split_once(' ')
                    .ok_or(format!("tokenizer.json: merge {s:?} is not two pieces"))?;
                (a.to_string(), b.to_string())
            } else {
                match mg.as_array().map(|x| x.as_slice()) {
                    Some([a, b]) => (str_of(a, "merge")?, str_of(b, "merge")?),
                    _ => return Err(format!("tokenizer.json: merge {rank} is not two pieces")),
                }
            };
            let id = |p: &str| {
                ids.get(p).copied().ok_or(format!(
                    "tokenizer.json: merge {rank} names {p:?}, not in the vocabulary"
                ))
            };
            let (ia, ib) = (id(&a)?, id(&b)?);
            let merged = id(&format!("{a}{b}"))?;
            merges.entry((ia, ib)).or_insert((rank as u32, merged));
        }
        let mut norm = Vec::new();
        normalizers(&v["normalizer"], &mut norm)?;
        let mut pre = Vec::new();
        pre_tokenizers(&v["pre_tokenizer"], &mut pre)?;
        let byte_fallback = m["byte_fallback"].as_bool().unwrap_or(false);
        let mut byte_ids = vec![None; 256];
        for (b, slot) in byte_ids.iter_mut().enumerate() {
            *slot = ids.get(&format!("<0x{b:02X}>")).copied();
        }
        let unk = match m["unk_token"].as_str() {
            Some(u) => Some(
                *ids.get(u)
                    .ok_or(format!("tokenizer.json: unk_token {u:?} is not in the vocabulary"))?,
            ),
            None => None,
        };
        // the decoder decides how pieces read back
        let d = &v["decoder"];
        let (mut byte_level, mut space_char, mut strip_start) = (false, None, 0);
        let mut decoder = |d: &Json| -> Result<(), String> {
            match kind(d) {
                "ByteLevel" => byte_level = true,
                "Metaspace" => {
                    space_char = Some(metaspace_char(d)?);
                    strip_start = (prepend_of(d) != Prepend::Never) as usize;
                }
                "Replace" => {
                    let p = d["pattern"]["String"]
                        .as_str()
                        .ok_or("tokenizer.json: decoder Replace needs a string")?;
                    let mut c = p.chars();
                    match (c.next(), c.next(), d["content"].as_str()) {
                        (Some(x), None, Some(" ")) => space_char = Some(x),
                        _ => return Err(format!("tokenizer.json: decoder Replace {p:?} is not supported")),
                    }
                }
                "Strip" => {
                    if d["content"].as_str() != Some(" ") || d["stop"].as_u64().unwrap_or(0) != 0 {
                        return Err("tokenizer.json: decoder Strip other than leading spaces is not supported".into());
                    }
                    strip_start = d["start"].as_u64().unwrap_or(0) as usize;
                }
                "ByteFallback" | "Fuse" => {}
                other => return Err(format!("tokenizer.json: decoder {other:?} is not supported")),
            }
            Ok(())
        };
        match kind(d) {
            "" if d.is_null() => {}
            "Sequence" => {
                for x in d["decoders"]
                    .as_array()
                    .ok_or("tokenizer.json: decoder Sequence without a list")?
                {
                    decoder(x)?;
                }
            }
            _ => decoder(d)?,
        }
        byte_level |= pre.iter().any(|p| matches!(p, Pre::ByteLevel { .. }));
        Ok(Tokenizer {
            source: text.to_string(),
            pieces,
            ids,
            merges,
            added,
            added_of,
            normalizers: norm,
            pre,
            byte_level,
            byte_fallback,
            byte_ids,
            unk,
            ignore_merges: m["ignore_merges"].as_bool().unwrap_or(false),
            space_char,
            strip_start,
        })
    }

    /// the `tokenizer.json` text this was read from
    pub fn source(&self) -> &str {
        &self.source
    }

    /// ids 0 .. size (holes decode to nothing)
    pub fn size(&self) -> usize {
        self.pieces.len()
    }

    /// the id of a piece or added token, as written in the vocabulary
    pub fn id(&self, piece: &str) -> Option<u32> {
        self.ids.get(piece).copied()
    }

    pub fn piece(&self, id: u32) -> Option<&str> {
        self.pieces.get(id as usize).map(String::as_str)
    }

    pub fn is_special(&self, id: u32) -> bool {
        self.added_of.get(&id).is_some_and(|&i| self.added[i].special)
    }

    /// text to ids; added tokens written in the text are recognised (as the reference
    /// implementation does). Nothing is added at the start or end: models add their
    /// own start token (`pretrained::Lexicon`).
    pub fn encode(&self, text: &str) -> Vec<u32> {
        self.encode_with(text, true)
    }

    /// `encode`, optionally reading added tokens' text as plain text
    pub fn encode_with(&self, text: &str, added: bool) -> Vec<u32> {
        let mut out = Vec::new();
        let mut first = true;
        for seg in self.segments(text, added) {
            match seg {
                Seg::Token(id) => out.push(id),
                Seg::Text(t) => {
                    self.encode_text(t, first, &mut out);
                }
            }
            first = false;
        }
        out
    }

    /// splits at added tokens, applying their lstrip/rstrip
    fn segments<'a>(&self, text: &'a str, on: bool) -> Vec<Seg<'a>> {
        if !on || self.added.is_empty() {
            return vec![Seg::Text(text)];
        }
        let mut segs = Vec::new();
        let (mut start, mut i) = (0, 0);
        let b = text.as_bytes();
        while i < b.len() {
            let hit = self
                .added
                .iter()
                .find(|a| !a.content.is_empty() && b[i..].starts_with(a.content.as_bytes()));
            match hit {
                Some(a) => {
                    let mut before = &text[start..i];
                    if a.lstrip {
                        before = before.trim_end();
                    }
                    if !before.is_empty() {
                        segs.push(Seg::Text(before));
                    }
                    segs.push(Seg::Token(a.id));
                    i += a.content.len();
                    if a.rstrip {
                        while i < b.len() && text[i..].starts_with(char::is_whitespace) {
                            i += text[i..].chars().next().unwrap().len_utf8();
                        }
                    }
                    start = i;
                }
                None => i += 1,
            }
        }
        if start < b.len() {
            segs.push(Seg::Text(&text[start..]));
        }
        segs
    }

    fn encode_text(&self, text: &str, first: bool, out: &mut Vec<u32>) {
        let mut s = text.to_string();
        for n in &self.normalizers {
            s = match n {
                Normalizer::Prepend(p) if !s.is_empty() => format!("{p}{s}"),
                Normalizer::Prepend(_) => s,
                Normalizer::Replace(a, b) => s.replace(a.as_str(), b),
                Normalizer::Lowercase => s.to_lowercase(),
                Normalizer::Strip(l, r) => {
                    let t = if *l { s.trim_start() } else { &s };
                    (if *r { t.trim_end() } else { t }).to_string()
                }
            };
        }
        let mut pieces = vec![s];
        for p in &self.pre {
            let mut next = Vec::new();
            for (k, piece) in pieces.into_iter().enumerate() {
                match p {
                    Pre::ByteLevel { prefix_space, regex } => {
                        let piece = if *prefix_space && k == 0 && !piece.starts_with(' ') {
                            format!(" {piece}")
                        } else {
                            piece
                        };
                        match regex {
                            Some(r) => next.extend(r.find_all(&piece).into_iter().map(String::from)),
                            None => next.push(piece),
                        }
                    }
                    Pre::Split {
                        regex,
                        behavior,
                        invert,
                    } => {
                        let chars: Vec<char> = piece.chars().collect();
                        next.extend(split_by(&piece, &regex.find_iter(&chars), *behavior, *invert));
                    }
                    Pre::Digits { individual } => {
                        let mut cur = String::new();
                        let mut digit = false;
                        for c in piece.chars() {
                            let d = c.is_ascii_digit();
                            if !cur.is_empty() && (d != digit || (d && *individual)) {
                                next.push(std::mem::take(&mut cur));
                            }
                            digit = d;
                            cur.push(c);
                        }
                        if !cur.is_empty() {
                            next.push(cur);
                        }
                    }
                    Pre::Metaspace {
                        replacement,
                        prepend,
                        split,
                    } => {
                        let mut t: String = piece.chars().map(|c| if c == ' ' { *replacement } else { c }).collect();
                        let add = match prepend {
                            Prepend::Always => true,
                            Prepend::First => first && k == 0,
                            Prepend::Never => false,
                        };
                        if add && !t.starts_with(*replacement) {
                            t.insert(0, *replacement);
                        }
                        if *split {
                            let mut cur = String::new();
                            for c in t.chars() {
                                if c == *replacement && !cur.is_empty() {
                                    next.push(std::mem::take(&mut cur));
                                }
                                cur.push(c);
                            }
                            if !cur.is_empty() {
                                next.push(cur);
                            }
                        } else {
                            next.push(t);
                        }
                    }
                }
            }
            pieces = next;
        }
        let map = byte_chars();
        for piece in pieces {
            if piece.is_empty() {
                continue;
            }
            let word = if self.byte_level {
                piece.bytes().map(|b| map[b as usize]).collect()
            } else {
                piece
            };
            self.bpe(&word, out);
        }
    }

    /// one pre-token through the merges (lowest rank first, then leftmost)
    fn bpe(&self, word: &str, out: &mut Vec<u32>) {
        if self.ignore_merges
            && let Some(&id) = self.ids.get(word)
        {
            out.push(id);
            return;
        }
        let mut syms: Vec<u32> = Vec::with_capacity(word.len());
        let mut buf = [0u8; 4];
        for c in word.chars() {
            if let Some(&id) = self.ids.get(c.encode_utf8(&mut buf) as &str) {
                syms.push(id);
            } else if self.byte_fallback
                && c.encode_utf8(&mut buf)
                    .bytes()
                    .all(|b| self.byte_ids[b as usize].is_some())
            {
                syms.extend(
                    c.encode_utf8(&mut buf)
                        .bytes()
                        .map(|b| self.byte_ids[b as usize].unwrap()),
                );
            } else if let Some(u) = self.unk
                && syms.last() != Some(&u)
            {
                syms.push(u);
            }
        }
        if syms.len() < 2 {
            out.extend(syms);
            return;
        }
        // a linked list of symbols and a heap of candidate merges
        let n = syms.len();
        let mut prev: Vec<usize> = (0..n).map(|i| i.wrapping_sub(1)).collect();
        let mut next: Vec<usize> = (1..=n).collect();
        let mut alive = vec![true; n];
        let mut heap = BinaryHeap::new();
        let push = |heap: &mut BinaryHeap<Reverse<(u32, usize, u32)>>, syms: &[u32], i: usize, j: usize| {
            if let Some(&(rank, m)) = self.merges.get(&(syms[i], syms[j])) {
                heap.push(Reverse((rank, i, m)));
            }
        };
        for i in 0..n - 1 {
            push(&mut heap, &syms, i, i + 1);
        }
        while let Some(Reverse((_, i, m))) = heap.pop() {
            let j = next[i];
            if !alive[i] || j >= n || self.merges.get(&(syms[i], syms[j])).map(|x| x.1) != Some(m) {
                continue;
            }
            syms[i] = m;
            alive[j] = false;
            next[i] = next[j];
            if next[j] < n {
                prev[next[j]] = i;
            }
            if prev[i] < n {
                push(&mut heap, &syms, prev[i], i);
            }
            if next[i] < n {
                push(&mut heap, &syms, i, next[i]);
            }
        }
        out.extend((0..n).filter(|&i| alive[i]).map(|i| syms[i]));
    }

    /// the bytes one token stands for (no leading-space stripping)
    pub fn token_bytes(&self, id: u32) -> Vec<u8> {
        if let Some(&i) = self.added_of.get(&id) {
            return self.added[i].content.as_bytes().to_vec();
        }
        let Some(p) = self.pieces.get(id as usize) else {
            return Vec::new();
        };
        if self.byte_level {
            let map = byte_chars();
            let mut out = Vec::with_capacity(p.len());
            let mut buf = [0u8; 4];
            for c in p.chars() {
                match map.iter().position(|&m| m == c) {
                    Some(b) => out.push(b as u8),
                    None => out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes()),
                }
            }
            return out;
        }
        if self.byte_fallback
            && let Some(b) = p.strip_prefix("<0x").and_then(|r| r.strip_suffix('>'))
            && b.len() == 2
            && let Ok(v) = u8::from_str_radix(b, 16)
        {
            return vec![v];
        }
        match self.space_char {
            Some(c) => p.replace(c, " ").into_bytes(),
            None => p.as_bytes().to_vec(),
        }
    }

    /// ids back to bytes (special tokens left out when `skip_special`); the leading
    /// space a SentencePiece-style tokenizer adds is taken off again
    pub fn decode(&self, ids: &[u32], skip_special: bool) -> Vec<u8> {
        let mut out = Vec::new();
        for &id in ids {
            if skip_special && self.is_special(id) {
                continue;
            }
            out.extend(self.token_bytes(id));
        }
        let strip = out.iter().take(self.strip_start).take_while(|&&b| b == b' ').count();
        out.drain(..strip);
        out
    }

    /// `decode` as text (invalid UTF-8 replaced)
    pub fn decode_text(&self, ids: &[u32]) -> String {
        String::from_utf8_lossy(&self.decode(ids, true)).into_owned()
    }
}

/// a tokenizer with the tokens a model starts sequences with and ends them with
#[derive(Clone, Debug, PartialEq)]
pub struct Lexicon {
    pub tokenizer: Tokenizer,
    /// put before every window (BOS, or the end token for models without one)
    pub start: u32,
    /// the end of a text: what training windows end in and the generative head's END
    pub end: u32,
    /// tokens that also end generation (chat models have several)
    pub stops: Vec<u32>,
}

impl Lexicon {
    /// bytes to ids, `data[..from]` and `data[from..]` encoded apart (a prompt and its
    /// continuation), with the id index where the second part starts
    pub fn encode_split(&self, data: &[u8], from: usize) -> (Vec<u32>, usize) {
        let from = from.min(data.len());
        let mut ids = if from > 0 {
            self.tokenizer.encode(&String::from_utf8_lossy(&data[..from]))
        } else {
            Vec::new()
        };
        let at = ids.len();
        if from < data.len() {
            ids.extend(self.tokenizer.encode(&String::from_utf8_lossy(&data[from..])));
        }
        (ids, at)
    }

    /// whether a token ends generation
    pub fn stops(&self, id: u32) -> bool {
        id == self.end || self.stops.contains(&id)
    }
}

enum Seg<'a> {
    Text(&'a str),
    Token(u32),
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// a tiny GPT-2 style byte-level tokenizer (Llama 3 / Qwen2 shaped): every byte is
    /// a token, a few merges, two special tokens
    pub fn byte_level_json() -> String {
        let map = byte_chars();
        let mut vocab = serde_json::Map::new();
        for (b, c) in map.iter().enumerate() {
            vocab.insert(c.to_string(), b.into());
        }
        let merges = [
            "Ġ t", "h e", "Ġt he", "l l", "ll o", "Ġ w", "o r", "Ġw or", "l d", "Ġwor ld", "1 2",
        ];
        let mut next = 256;
        for m in merges {
            let (a, b) = m.split_once(' ').unwrap();
            vocab.insert(format!("{a}{b}"), next.into());
            next += 1;
        }
        serde_json::json!({
            "version": "1.0",
            "added_tokens": [
                {"id": next, "content": "<|endoftext|>", "special": true, "lstrip": false, "rstrip": false, "normalized": false, "single_word": false},
                {"id": next + 1, "content": "<|im_start|>", "special": true, "lstrip": false, "rstrip": false, "normalized": false, "single_word": false}
            ],
            "normalizer": {"type": "NFC"},
            "pre_tokenizer": {"type": "Sequence", "pretokenizers": [
                {"type": "Split", "pattern": {"Regex": "(?i:'s|'t|'re|'ve|'m|'ll|'d)|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+"},
                 "behavior": "Isolated", "invert": false},
                {"type": "ByteLevel", "add_prefix_space": false, "trim_offsets": false, "use_regex": false}
            ]},
            "post_processor": {"type": "ByteLevel"},
            "decoder": {"type": "ByteLevel", "add_prefix_space": true, "trim_offsets": true, "use_regex": true},
            "model": {"type": "BPE", "dropout": null, "unk_token": null, "byte_fallback": false,
                      "ignore_merges": false, "vocab": vocab,
                      "merges": merges.iter().map(|m| { let (a, b) = m.split_once(' ').unwrap(); serde_json::json!([a, b]) }).collect::<Vec<_>>()}
        })
        .to_string()
    }

    /// a tiny SentencePiece style tokenizer (Llama 2 / TinyLlama shaped): ▁ for spaces,
    /// byte fallback, merges written as strings
    pub fn sentencepiece_json() -> String {
        let mut vocab = serde_json::Map::new();
        vocab.insert("<unk>".into(), 0.into());
        vocab.insert("<s>".into(), 1.into());
        vocab.insert("</s>".into(), 2.into());
        for b in 0..256 {
            vocab.insert(format!("<0x{b:02X}>"), (3 + b).into());
        }
        let mut next = 259;
        for p in ["▁", "h", "e", "l", "o", "w", "r", "d", "a", "b"] {
            vocab.insert(p.into(), next.into());
            next += 1;
        }
        let merges = [
            "h e",
            "l l",
            "▁ he",
            "▁he ll",
            "▁hell o",
            "▁ w",
            "o r",
            "▁w or",
            "▁wor l",
            "▁worl d",
        ];
        for m in merges {
            let (a, b) = m.split_once(' ').unwrap();
            vocab.insert(format!("{a}{b}"), next.into());
            next += 1;
        }
        serde_json::json!({
            "added_tokens": [
                {"id": 0, "content": "<unk>", "special": true},
                {"id": 1, "content": "<s>", "special": true},
                {"id": 2, "content": "</s>", "special": true}
            ],
            "normalizer": {"type": "Sequence", "normalizers": [
                {"type": "Prepend", "prepend": "▁"},
                {"type": "Replace", "pattern": {"String": " "}, "content": "▁"}
            ]},
            "pre_tokenizer": null,
            "decoder": {"type": "Sequence", "decoders": [
                {"type": "Replace", "pattern": {"String": "▁"}, "content": " "},
                {"type": "ByteFallback"}, {"type": "Fuse"},
                {"type": "Strip", "content": " ", "start": 1, "stop": 0}
            ]},
            "model": {"type": "BPE", "unk_token": "<unk>", "byte_fallback": true, "fuse_unk": true,
                      "vocab": vocab, "merges": merges}
        })
        .to_string()
    }

    fn pieces(t: &Tokenizer, ids: &[u32]) -> Vec<String> {
        ids.iter().map(|&i| t.piece(i).unwrap().to_string()).collect()
    }

    #[test]
    fn byte_level_encodes_and_decodes() {
        let t = Tokenizer::from_json(&byte_level_json()).unwrap();
        let ids = t.encode("the world, hello 12!");
        // pre-tokens "the", " world", ",", " hello", " ", "1", "2", "!" (digits one by one)
        assert_eq!(
            pieces(&t, &ids),
            ["t", "he", "Ġworld", ",", "Ġ", "he", "llo", "Ġ", "1", "2", "!"]
        );
        assert_eq!(t.decode(&ids, false), b"the world, hello 12!");
        // a space merge: " the"
        assert_eq!(pieces(&t, &t.encode(" the")), ["Ġthe"]);
        // special tokens in the text, bytes outside ASCII
        let ids = t.encode("<|im_start|>héllo<|endoftext|>");
        assert_eq!(ids.first(), t.id("<|im_start|>").as_ref());
        assert_eq!(ids.last(), t.id("<|endoftext|>").as_ref());
        assert_eq!(t.decode(&ids, false), "<|im_start|>héllo<|endoftext|>".as_bytes());
        assert_eq!(t.decode(&ids, true), "héllo".as_bytes());
        assert!(t.is_special(*ids.last().unwrap()));
        // read as text, the special token is just bytes
        assert!(
            !t.encode_with("<|endoftext|>", false)
                .contains(&t.id("<|endoftext|>").unwrap())
        );
        // every byte string round trips
        let all: Vec<u8> = (0..=255).collect();
        let s = String::from_utf8_lossy(&all).into_owned();
        assert_eq!(t.decode(&t.encode(&s), false), s.as_bytes());
    }

    #[test]
    fn sentencepiece_style_encodes_and_decodes() {
        let t = Tokenizer::from_json(&sentencepiece_json()).unwrap();
        let ids = t.encode("hello world");
        assert_eq!(pieces(&t, &ids), ["▁hello", "▁world"]);
        assert_eq!(t.decode(&ids, true), b"hello world");
        // unknown characters fall back to their bytes
        let ids = t.encode("hé");
        assert_eq!(pieces(&t, &ids), ["▁", "h", "<0xC3>", "<0xA9>"]);
        assert_eq!(t.decode(&ids, true), "hé".as_bytes());
        // a generated token after a prompt keeps its space
        assert_eq!(t.token_bytes(t.id("▁world").unwrap()), b" world");
        assert_eq!(t.decode(&t.encode("<s>ab"), true), b"ab");
    }

    #[test]
    fn unsupported_parts_are_named() {
        let mut v: Json = serde_json::from_str(&byte_level_json()).unwrap();
        v["model"]["type"] = "WordPiece".into();
        assert!(
            Tokenizer::from_json(&v.to_string())
                .unwrap_err()
                .contains("model type \"WordPiece\"")
        );
        let mut v: Json = serde_json::from_str(&byte_level_json()).unwrap();
        v["normalizer"] = serde_json::json!({"type": "NFKC"});
        assert!(
            Tokenizer::from_json(&v.to_string())
                .unwrap_err()
                .contains("normalizer \"NFKC\"")
        );
        let mut v: Json = serde_json::from_str(&byte_level_json()).unwrap();
        v["pre_tokenizer"] = serde_json::json!({"type": "BertPreTokenizer"});
        assert!(
            Tokenizer::from_json(&v.to_string())
                .unwrap_err()
                .contains("BertPreTokenizer")
        );
        let mut v: Json = serde_json::from_str(&byte_level_json()).unwrap();
        v["model"]["merges"][0] = serde_json::json!(["Ġ", "zz"]);
        assert!(Tokenizer::from_json(&v.to_string()).unwrap_err().contains("\"zz\""));
    }
}
