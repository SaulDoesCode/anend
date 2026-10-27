//! s, the user's scripting language (docs/s.md): a postfix language whose acting word
//! stands at the right. Programs are entries of the save (mime `text/x-s`), stored in the
//! 5-bit format like any text (the format's s mode makes their symbols cheap, see
//! `graph::codec`), and run here: `lex` makes tokens, `machine` runs them (tail calls in
//! constant space), `lib` is the library (`std/*.s`).

pub mod lex;
pub mod lib;
pub mod machine;
pub mod val;

use std::collections::BTreeMap;

pub use machine::{Outcome, Rt};

/// runs a program: its source, what stdin holds, its files, its arguments, at most how
/// many steps it may take, the seed of its random numbers (the caller's clock: the
/// browser has no system time to read here), and its host for `std:anend` (none: `op`
/// fails)
pub fn run(
    src: &str,
    stdin: &str,
    files: BTreeMap<String, String>,
    args: &[String],
    max_steps: u64,
    seed: u64,
    host: Option<machine::Host<'_>>,
) -> Outcome {
    let mut rt = Rt::new(stdin, files, seed);
    rt.max_steps = max_steps;
    rt.host = host;
    rt.run_program(src, args)
}

/// a run with its inline results: the output each of the program's own tokens wrote
/// (while it or a call it made ran), by the token's place in `src` (byte offsets)
pub fn run_traced(
    src: &str,
    stdin: &str,
    files: BTreeMap<String, String>,
    args: &[String],
    max_steps: u64,
    seed: u64,
    host: Option<machine::Host<'_>>,
) -> (Outcome, Vec<(std::ops::Range<usize>, String)>) {
    let mut rt = Rt::new(stdin, files, seed);
    rt.max_steps = max_steps;
    rt.host = host;
    rt.trace = Some(machine::Trace::default());
    let o = rt.run_program(src, args);
    let spans = lex::lex_spans(src).unwrap_or_default();
    let writes = rt
        .trace
        .take()
        .map(|t| t.writes)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(i, w)| spans.get(i).map(|s| (s.1.clone(), w)))
        .collect();
    (o, writes)
}

/// a run stopped before its `steps`-th step, as a debugger shows it
#[derive(Clone, Debug, serde::Serialize)]
pub struct Snapshot {
    /// it ended before getting there (then `err` says how, if it failed)
    pub done: bool,
    pub steps: u64,
    pub out: String,
    pub err: Option<String>,
    /// where it stopped: in the program's own code (`main`), else the running unit (a
    /// script's body, a file, a definition) as text; the next token's place in it
    pub main: bool,
    pub unit: String,
    pub at: Option<std::ops::Range<usize>>,
    /// how many activations are running
    pub depth: usize,
    pub stack: Vec<String>,
    pub ow: bool,
    pub mem: BTreeMap<String, String>,
    pub numbers: BTreeMap<String, String>,
    pub lists: BTreeMap<String, Vec<String>>,
    pub locals: BTreeMap<String, String>,
}

/// runs `src` up to (not into) its `steps`-th step: what the debugger shows. The same seed
/// gives the same run, so stepping is running again one step further
pub fn debug(
    src: &str,
    stdin: &str,
    files: BTreeMap<String, String>,
    args: &[String],
    steps: u64,
    seed: u64,
    host: Option<machine::Host<'_>>,
) -> Snapshot {
    let mut rt = Rt::new(stdin, files, seed);
    rt.host = host;
    rt.trace = Some(machine::Trace::default());
    rt.pause_at = Some(steps);
    rt.max_steps = steps.saturating_add(1);
    let o = rt.run_program(src, args);
    let done = o.code != -1;
    let (unit, main, at, depth) = match (done, rt.position()) {
        (false, Some((u, main, pc, depth))) => {
            let at = lex::lex_spans(&u).ok().and_then(|s| s.get(pc).map(|t| t.1.clone()));
            (if main { String::new() } else { u.to_string() }, main, at, depth)
        }
        _ => (String::new(), false, None, 0),
    };
    let text = |m: &std::collections::HashMap<std::rc::Rc<str>, val::V>| -> BTreeMap<String, String> {
        m.iter().map(|(k, v)| (k.to_string(), v.text().to_string())).collect()
    };
    let nums = |m: &std::collections::HashMap<std::rc::Rc<str>, f64>| -> BTreeMap<String, String> {
        m.iter().map(|(k, v)| (k.to_string(), val::num_text(*v))).collect()
    };
    let mut locals = BTreeMap::new();
    if let Some(l) = rt.locals() {
        locals.extend(text(&l.mem));
        locals.extend(nums(&l.numbers));
    }
    Snapshot {
        done,
        steps: rt.steps,
        out: o.out,
        err: o.err,
        main,
        unit,
        at,
        depth,
        stack: rt.stack.iter().map(|v| v.text().to_string()).collect(),
        ow: rt.ow,
        mem: text(&rt.global.mem),
        numbers: nums(&rt.global.numbers),
        lists: rt
            .global
            .lists
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|x| x.text().to_string()).collect()))
            .collect(),
        locals,
    }
}

#[cfg(test)]
mod tests;
