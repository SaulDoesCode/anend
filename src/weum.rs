//! Ports of the helpers weum's memory view (memory.js / api.js) is built on: the prime
//! factor reduction line (`pfR`), the gematria detail line (`gsum(text, 1)`) and the search
//! variation expansion (`tryfulSearchVariations`).

use std::collections::VecDeque;

use crate::simd;

// ---------------------------------------------------------------- pfR

pub fn prime_factors(mut n: u64) -> Vec<u64> {
    let mut found = Vec::new();
    let mut d = 2;
    while n > 1 {
        while n.is_multiple_of(d) {
            found.push(d);
            n /= d;
        }
        d += 1;
        if d * d > n && n > 1 {
            found.push(n);
            break;
        }
    }
    found
}

pub fn digit_sum(n: u64) -> u64 {
    n.to_string().bytes().map(|b| (b - b'0') as u64).sum()
}

fn pf_sum(n: u64) -> u64 {
    prime_factors(n).iter().sum()
}

/// a piece of the reduction line; numbers are clickable in the view
#[derive(Debug, Clone, PartialEq)]
pub enum Seg {
    Text(String),
    Num(u64),
}

/// weum's `pfR(s)`: the prime factors and their sum, times the digit sum, then the chain
/// of (prime factor sum × digit sum) steps until it repeats, its total and digit sums
pub fn pf_reduction(s: u64) -> Vec<Seg> {
    const LIMIT: usize = 32;
    let pf = prime_factors(s);
    let pfr: u64 = pf.iter().sum();
    let ds = digit_sum(s);
    let dp = ds * pfr;
    let mut chain: Vec<u64> = Vec::new();
    for _ in 0..=LIMIT {
        let xn = match chain.last() {
            Some(&last) => pf_sum(last) * digit_sum(last),
            None => pf_sum(dp) * ds,
        };
        if chain.contains(&xn) {
            break;
        }
        chain.push(xn);
    }
    let total = chain.iter().sum::<u64>() + pfr;
    let fds = digit_sum(total);

    let mut segs = Vec::new();
    match pf.len() {
        0 => {}
        1 => segs.push(Seg::Text(format!("{} ", pf[0]))),
        _ => segs.push(Seg::Text(format!(
            "{} = {} -> ",
            pf.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" + "),
            pfr
        ))),
    }
    segs.push(Seg::Text(format!("* {} = ", ds)));
    segs.push(Seg::Num(dp));
    for t in &chain {
        segs.push(Seg::Text("+".into()));
        segs.push(Seg::Num(*t));
    }
    segs.push(Seg::Text(format!(" = {} {} | {}", total, fds, ds + fds)));
    segs
}

#[cfg(test)]
pub fn segs_to_string(segs: &[Seg]) -> String {
    segs.iter()
        .map(|s| match s {
            Seg::Text(t) => t.clone(),
            Seg::Num(n) => n.to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------- gsum detail

fn vowel(c: u8) -> u64 {
    match c {
        b'a' => 1,
        b'e' => 2,
        b'o' => 3,
        b'u' => 4,
        b'i' => 5,
        b'y' => 6,
        _ => 0,
    }
}

fn consonant(c: u8) -> u64 {
    // weum's table, t and v both 16 included
    match c {
        b'b' => 1,
        b'c' => 2,
        b'd' => 3,
        b'f' => 4,
        b'g' => 5,
        b'h' => 6,
        b'j' => 7,
        b'k' => 8,
        b'l' => 9,
        b'm' => 10,
        b'n' => 11,
        b'p' => 12,
        b'q' => 13,
        b'r' => 14,
        b's' => 15,
        b't' | b'v' => 16,
        b'w' => 17,
        b'x' => 18,
        b'z' => 19,
        _ => 0,
    }
}

/// weum's `gsum(text, 1)` line, with the Atbash measure appended:
/// `(sum + r = total; diff d, ...) v vowels c consonants w words a atbash`
pub fn gsum_detail(text: &str) -> String {
    let s = simd::sums(text.as_bytes());
    let (sum, r) = (s.ordinal as i64, s.reversed as i64);
    let (mut v, mut c) = (0, 0);
    for b in text.bytes() {
        let l = b.to_ascii_lowercase();
        v += vowel(l);
        c += consonant(l);
    }
    let words = text.split_whitespace().count().max(1);
    let a = (sum + r) + (sum - r).abs();
    let b = (sum + r) + (sum - r);
    format!(
        "({} + {} = {}; diff {}, {} {} {}) v {} c {} w {} a {}",
        sum,
        r,
        sum + r,
        sum - r,
        a,
        b,
        a - b,
        v,
        c,
        words,
        s.atbash
    )
}

// ---------------------------------------------------------------- search variations

const DEFAULT_VARIATIONS: [&str; 14] = [
    "man", "ton", "able", "y", "ly", "ings", "ing", "ed", "s,", ".", "?", ":", ",", "s",
];

/// the JS array `variations` together with the flags memory.js hangs off it
#[derive(Clone, Default)]
struct Variations {
    list: Vec<String>,
    lc: bool,
    sless: bool,
    erer: bool,
    encs: bool,
    mic: bool,
    ll: bool,
}

enum Task {
    Search(String),
    Continue { query: String, vars: Variations, max: i32 },
}

fn ends(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

fn drop_last(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars[..chars.len().saturating_sub(n)].iter().collect()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// one call of `tryfulSearchVariations`; searches go to `out`, timers to `tasks`
fn tsv(
    mut query: String,
    mut vars: Variations,
    f: bool,
    max: i32,
    caseful: bool,
    tasks: &mut VecDeque<Task>,
    out: &mut Vec<String>,
) {
    let _ = out;
    if max <= 0 || query.is_empty() {
        return;
    }
    let exact = query.len() >= 2 && query.starts_with('"') && query.ends_with('"');
    if exact {
        query = query[1..query.len() - 1].trim().to_string();
        if query.is_empty() {
            return;
        }
    }
    let last = query.chars().last().unwrap();
    if vars.list.is_empty() {
        if !vars.lc {
            vars.lc = true;
            if query != query.to_lowercase() {
                query = query.to_lowercase();
            }
        }
        let push = |vars: &mut Variations, items: &[&str]| vars.list.extend(items.iter().map(|s| s.to_string()));
        match last {
            'y' => {
                if ends(&query, "iety") {
                    query = drop_last(&query, 4);
                    push(&mut vars, &["iation", "iations", "ieties"]);
                } else if ends(&query, "ey") {
                    query = drop_last(&query, 2);
                    push(&mut vars, &["ies", "ied"]);
                } else {
                    query = drop_last(&query, 1);
                    push(&mut vars, &["ies", "s"]);
                }
            }
            's' => {
                if ends(&query, "ngs") {
                    query = drop_last(&query, 3);
                    push(&mut vars, &["ngies"]);
                } else if !vars.sless {
                    query = drop_last(&query, 1);
                    vars.sless = true;
                } else {
                    return;
                }
            }
            'r' | 'd' => {
                if !vars.erer {
                    vars.erer = true;
                    let first = if !ends(&query, "er") { "er" } else { "s" };
                    push(&mut vars, &[first, "ences"]);
                    vars.encs = true;
                } else if !vars.encs {
                    vars.encs = true;
                    if !ends(&query, "ences") {
                        push(&mut vars, &["ences"]);
                    }
                } else {
                    return;
                }
            }
            'm' | 'n' => {
                if !vars.mic {
                    push(&mut vars, &["ic"]);
                    vars.mic = true;
                } else {
                    return;
                }
            }
            'e' => {
                if ends(&query, "ime") {
                    query = drop_last(&query, 1);
                    push(&mut vars, &["ing"]);
                } else if ends(&query, "ane") {
                    query = drop_last(&query, 1);
                    push(&mut vars, &["ity"]);
                } else {
                    return;
                }
            }
            't' => {
                if ends(&query, "ent") {
                    query = drop_last(&query, 1);
                    push(&mut vars, &["cy", "ial", "ce"]);
                } else {
                    return;
                }
            }
            'l' if !vars.ll => {
                vars.ll = true;
                let chars: Vec<char> = query.chars().collect();
                if chars.len() >= 2 && chars[chars.len() - 2] == 'l' {
                    push(&mut vars, &["er", "ity"]);
                }
            }
            _ => return,
        }
    }
    if f {
        tasks.push_back(Task::Search(query.trim().to_string()));
    }
    // the clone only keeps the lowercase flag, like `[...variations]` with `_lc` set
    let clone = Variations {
        list: vars.list.clone(),
        lc: true,
        ..Default::default()
    };
    if !caseful {
        let fancy = capitalize(&query);
        if query != fancy {
            tsv(fancy, clone, true, max - 1, false, tasks, out);
        }
    }
    if !exact {
        tasks.push_back(Task::Continue { query, vars, max });
    }
}

/// every query weum's ordinary search sends for `query`, in the order it sends them
pub fn search_variations(query: &str) -> Vec<String> {
    let mut tasks = VecDeque::new();
    let mut out = Vec::new();
    let vars = Variations {
        list: DEFAULT_VARIATIONS.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    tsv(query.to_string(), vars, true, 50, false, &mut tasks, &mut out);
    let mut guard = 0;
    while let Some(task) = tasks.pop_front() {
        guard += 1;
        if guard > 100_000 {
            break;
        }
        match task {
            Task::Search(q) => out.push(q),
            Task::Continue { query, mut vars, max } => {
                let suffix = vars.list.pop().unwrap_or_default();
                out.push(format!("{}{}", query.trim(), suffix));
                tsv(query, vars, false, max - 1, true, &mut tasks, &mut out);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn reference() -> Value {
        serde_json::from_str(include_str!("weum_reference.json")).unwrap()
    }

    #[test]
    fn variations_match_weum() {
        let r = reference();
        for (query, expected) in r["variations"].as_object().unwrap() {
            let expected: Vec<String> = expected
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            assert_eq!(search_variations(query), expected, "variations of {:?}", query);
        }
    }

    #[test]
    fn pf_reduction_matches_weum() {
        let r = reference();
        for (n, expected) in r["pfR"].as_object().unwrap() {
            let got = segs_to_string(&pf_reduction(n.parse().unwrap()));
            assert_eq!(got, expected.as_str().unwrap(), "pfR({})", n);
        }
    }

    #[test]
    fn gsum_detail_matches_weum() {
        let r = reference();
        for (text, expected) in r["gsum"].as_object().unwrap() {
            let line = expected[2].as_str().unwrap();
            let got = gsum_detail(text);
            assert!(got.starts_with(line), "gsum({:?}, 1): {:?} vs {:?}", text, got, line);
        }
    }
}
