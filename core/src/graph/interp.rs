//! The interpreter: rendering a body means decoding its run, transcluding the entries it
//! refers to (`REF`) and evaluating its calls (`CALL`).
//!
//! Everything is text. A function is any entry (by convention of mime
//! `application/x-anend-fn`): its body is a template evaluated with the call's arguments,
//! which it reads with the builtin `arg(n)`. Arguments are evaluated before the call
//! (call by value), except for `if`, which evaluates only the branch it takes. With
//! recursion and `if` the language is Turing complete.
//!
//! **No native recursion.** Evaluation is a loop over an explicit continuation stack, so a
//! Rust stack overflow can't happen. A call in **tail position** replaces the frame that
//! makes it instead of pushing a new one, so tail recursion runs in constant stack,
//! bounded only by fuel. Tail positions:
//! - the last segment of a run (a function body, an `if` branch, an argument, an entry's
//!   body): the text before it has already been emitted into the frame's output, and the
//!   call's output continues it (`out = prefix ++ call`, which concatenation makes sound);
//! - the branch `if` takes, when the `if` is itself in tail position;
//! - the last argument of `concat`, when the `concat` is in tail position (the earlier
//!   arguments are evaluated and emitted first).
//!
//! Anything else (an argument of a call other than those, a call followed by more
//! segments) is a non-tail call and pushes frames, bounded by `Limits::depth`.
//!
//! **Limits**, each an error rather than a panic: fuel (one unit per step, plus the size
//! of builtin work), the continuation stack depth, the size of any one output, and how
//! long a text a builtin may be asked to make. `REF` resolution is cycle-safe (an entry
//! that transcludes itself, directly or not, is an error); calls may recurse freely.
//! Non-tail calls of entries are memoised by (entry, arguments): evaluation is pure, so
//! naive recursive Fibonacci runs in linear time.
//!
//! **Builtins** (the callee codes under 32; numbers are decimal `i64` text, the empty text
//! is 0, and truth is non-empty text with `"1"` for true and `""` for false):
//!
//! | code | name | does |
//! |---|---|---|
//! | 0 | `arg(n)` | the call's `n`th argument, from 0 (`""` when there's none) |
//! | 1 | `if(c, a, b)` | `a` when `c` is non-empty, else `b` (only that one is evaluated) |
//! | 2 | `eq(a, b, …)` | whether all are equal |
//! | 3 | `concat(…)` | the arguments joined |
//! | 4 | `len(a)` | how many characters |
//! | 5 | `sum(a, mode)` | `a`'s ordinal (the default), `reversed` or `atbash` sum |
//! | 6 | `slice(a, from, to)` | characters `from..to`; negative counts from the end, `to` defaults to the end |
//! | 7 | `lookup(id)` | the rendered text of the entry with that (decimal) id |
//! | 8–12 | `add`, `sub`, `mul`, `div`, `mod` | arithmetic over all the arguments, left to right; overflow and division by zero are errors |
//! | 13 | `lt(a, b)` | whether `a < b`, numerically |
//! | 14 | `not(a)` | whether `a` is empty |
//! | 15 | `argc()` | how many arguments the call has |
//!
//! Codes 16–31 are reserved (calling one is an error).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use super::codec::{Callee, CodecError, Run, Seg};
use super::gematria::{Measure, Mode};

pub mod builtin {
    pub const ARG: u8 = 0;
    pub const IF: u8 = 1;
    pub const EQ: u8 = 2;
    pub const CONCAT: u8 = 3;
    pub const LEN: u8 = 4;
    pub const SUM: u8 = 5;
    pub const SLICE: u8 = 6;
    pub const LOOKUP: u8 = 7;
    pub const ADD: u8 = 8;
    pub const SUB: u8 = 9;
    pub const MUL: u8 = 10;
    pub const DIV: u8 = 11;
    pub const MOD: u8 = 12;
    pub const LT: u8 = 13;
    pub const NOT: u8 = 14;
    pub const ARGC: u8 = 15;

    pub const NAMES: [&str; 16] = [
        "arg", "if", "eq", "concat", "len", "sum", "slice", "lookup", "add", "sub", "mul", "div", "mod", "lt", "not",
        "argc",
    ];

    pub fn by_name(name: &str) -> Option<u8> {
        NAMES.iter().position(|n| *n == name).map(|i| i as u8)
    }
}

/// where the interpreter reads entries' bodies from
pub trait Source {
    /// an entry's body as a run, `None` when there's no such entry
    fn body(&self, id: u64) -> Result<Option<Vec<Seg>>, String>;
}

impl Source for HashMap<u64, Vec<Seg>> {
    fn body(&self, id: u64) -> Result<Option<Vec<Seg>>, String> {
        Ok(self.get(&id).cloned())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// steps an evaluation may take
    pub fuel: u64,
    /// continuation frames that may be waiting at once (non-tail nesting)
    pub depth: usize,
    /// bytes any one output (or argument) may grow to
    pub output: usize,
    /// memoised calls kept
    pub memo: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            fuel: 10_000_000,
            depth: 10_000,
            output: 4 << 20,
            memo: 4096,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// ran out of fuel
    Fuel,
    /// non-tail calls nested deeper than the limit
    Depth,
    /// an output grew past the limit
    Output,
    /// an entry transcludes itself
    Cycle(u64),
    /// a ref or call names an entry that isn't there
    Missing(u64),
    /// a builtin was given something it can't use
    Builtin(&'static str, String),
    Decode(CodecError),
    Source(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            EvalError::Fuel => write!(f, "ran out of fuel"),
            EvalError::Depth => write!(f, "calls nest too deeply"),
            EvalError::Output => write!(f, "the output grew too large"),
            EvalError::Cycle(id) => write!(f, "entry {id} transcludes itself"),
            EvalError::Missing(id) => write!(f, "there is no entry {id}"),
            EvalError::Builtin(name, why) => write!(f, "{name}: {why}"),
            EvalError::Decode(e) => write!(f, "{e}"),
            EvalError::Source(e) => write!(f, "{e}"),
        }
    }
}

type Env = Rc<[String]>;

/// how many calls one memo frame remembers (a tail loop adds one per turn)
const MEMO_KEYS: usize = 16;

enum Cont {
    /// a run being evaluated into `out`
    Run {
        run: Run,
        pos: usize,
        env: Env,
        out: String,
    },
    /// collecting the arguments of the call at `site[at]`
    Args {
        site: Run,
        at: usize,
        vals: Vec<String>,
        env: Env,
        tail: bool,
    },
    /// waiting for the condition of the `if` at `site[at]`
    If { site: Run, at: usize, env: Env, tail: bool },
    /// a call's value, to remember under these keys
    Memo { keys: Vec<(u64, Vec<String>)> },
    /// an entry's rendered text, to cache and unmark
    RefDone { id: u64 },
}

/// what evaluating took
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub steps: u64,
    pub max_stack: usize,
    pub memo_hits: u64,
}

pub struct Interp<'a, S: Source + ?Sized> {
    src: &'a S,
    pub limits: Limits,
    pub stats: Stats,
    fuel: u64,
    bodies: HashMap<u64, Run>,
    rendered: HashMap<u64, Arc<str>>,
    memo: HashMap<(u64, Vec<String>), String>,
    active: HashSet<u64>,
}

fn num(name: &'static str, s: &str) -> Result<i64, EvalError> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(0);
    }
    t.parse::<i64>()
        .map_err(|_| EvalError::Builtin(name, format!("{t:?} isn't a number")))
}

fn truth(b: bool) -> String {
    if b { "1".into() } else { String::new() }
}

impl<'a, S: Source + ?Sized> Interp<'a, S> {
    pub fn new(src: &'a S) -> Interp<'a, S> {
        Interp::with_limits(src, Limits::default())
    }

    pub fn with_limits(src: &'a S, limits: Limits) -> Interp<'a, S> {
        Interp {
            src,
            limits,
            stats: Stats::default(),
            fuel: limits.fuel,
            bodies: HashMap::new(),
            rendered: HashMap::new(),
            memo: HashMap::new(),
            active: HashSet::new(),
        }
    }

    /// the rendered text of an entry
    pub fn render(&mut self, id: u64) -> Result<String, EvalError> {
        self.eval(&[Seg::Ref(id)])
    }

    /// evaluates a run with no arguments
    pub fn eval(&mut self, run: &[Seg]) -> Result<String, EvalError> {
        self.start(Arc::new(run.to_vec()), Rc::from(Vec::new()))
    }

    /// calls an entry with arguments
    pub fn call(&mut self, f: u64, args: &[&str]) -> Result<String, EvalError> {
        let run = vec![Seg::call(
            Callee::Entry(f),
            args.iter().map(|a| vec![Seg::text(*a)]).collect(),
        )];
        self.eval(&run)
    }

    fn body(&mut self, id: u64) -> Result<Run, EvalError> {
        if let Some(b) = self.bodies.get(&id) {
            return Ok(b.clone());
        }
        let b = Arc::new(
            self.src
                .body(id)
                .map_err(EvalError::Source)?
                .ok_or(EvalError::Missing(id))?,
        );
        if self.bodies.len() < 1 << 16 {
            self.bodies.insert(id, b.clone());
        }
        Ok(b)
    }

    fn burn(&mut self, n: u64) -> Result<(), EvalError> {
        self.stats.steps += n;
        self.fuel = self.fuel.checked_sub(n).ok_or(EvalError::Fuel)?;
        Ok(())
    }

    fn grow(&self, out: &mut String, s: &str) -> Result<(), EvalError> {
        if out.len() + s.len() > self.limits.output {
            return Err(EvalError::Output);
        }
        out.push_str(s);
        Ok(())
    }

    fn push(&mut self, stack: &mut Vec<Cont>, c: Cont) -> Result<(), EvalError> {
        stack.push(c);
        if stack.len() > self.limits.depth {
            return Err(EvalError::Depth);
        }
        self.stats.max_stack = self.stats.max_stack.max(stack.len());
        Ok(())
    }

    /// runs `run` (in `env`) in the place of the frame on top of the stack, keeping its
    /// output: a tail call
    fn replace_top(&mut self, stack: &mut [Cont], new: Run, new_env: Env) {
        if let Some(Cont::Run { run, pos, env, .. }) = stack.last_mut() {
            *run = new;
            *pos = 0;
            *env = new_env;
        }
    }

    fn enter(&mut self, stack: &mut Vec<Cont>, run: Run, env: Env, tail: bool) -> Result<(), EvalError> {
        if tail {
            self.replace_top(stack, run, env);
            Ok(())
        } else {
            self.push(
                stack,
                Cont::Run {
                    run,
                    pos: 0,
                    env,
                    out: String::new(),
                },
            )
        }
    }

    fn begin_ref(&mut self, stack: &mut Vec<Cont>, id: u64) -> Result<Option<String>, EvalError> {
        if let Some(t) = self.rendered.get(&id) {
            return Ok(Some(t.to_string()));
        }
        if !self.active.insert(id) {
            return Err(EvalError::Cycle(id));
        }
        let body = self.body(id)?;
        self.push(stack, Cont::RefDone { id })?;
        self.push(
            stack,
            Cont::Run {
                run: body,
                pos: 0,
                env: Rc::from(Vec::new()),
                out: String::new(),
            },
        )?;
        Ok(None)
    }

    fn start(&mut self, run: Run, env: Env) -> Result<String, EvalError> {
        self.active.clear();
        self.fuel = self.limits.fuel;
        let mut stack: Vec<Cont> = Vec::new();
        self.push(
            &mut stack,
            Cont::Run {
                run,
                pos: 0,
                env,
                out: String::new(),
            },
        )?;
        let mut ret: Option<String> = None;
        loop {
            self.burn(1)?;
            if let Some(v) = ret.take() {
                let Some(top) = stack.pop() else { return Ok(v) };
                match top {
                    Cont::Run { run, pos, env, mut out } => {
                        self.grow(&mut out, &v)?;
                        stack.push(Cont::Run { run, pos, env, out });
                    }
                    Cont::Args {
                        site,
                        at,
                        mut vals,
                        env,
                        tail,
                    } => {
                        vals.push(v);
                        let Seg::Call { f, args } = &site[at] else {
                            unreachable!()
                        };
                        let f = *f;
                        let next = vals.len();
                        if next == args.len() {
                            ret = self.dispatch(&mut stack, f, vals, env, tail)?;
                        } else if tail && f == Callee::Builtin(builtin::CONCAT) && next == args.len() - 1 {
                            // the last piece of a concat in tail position continues the frame
                            let last = args[next].clone();
                            if let Some(Cont::Run { out, .. }) = stack.last_mut() {
                                for v in &vals {
                                    if out.len() + v.len() > self.limits.output {
                                        return Err(EvalError::Output);
                                    }
                                    out.push_str(v);
                                }
                            }
                            self.replace_top(&mut stack, last, env);
                        } else {
                            let arg = args[next].clone();
                            self.push(
                                &mut stack,
                                Cont::Args {
                                    site,
                                    at,
                                    vals,
                                    env: env.clone(),
                                    tail,
                                },
                            )?;
                            self.push(
                                &mut stack,
                                Cont::Run {
                                    run: arg,
                                    pos: 0,
                                    env,
                                    out: String::new(),
                                },
                            )?;
                        }
                    }
                    Cont::If { site, at, env, tail } => {
                        let Seg::Call { args, .. } = &site[at] else {
                            unreachable!()
                        };
                        let branch = if v.is_empty() { 2 } else { 1 };
                        match args.get(branch) {
                            Some(b) => {
                                let b = b.clone();
                                self.enter(&mut stack, b, env, tail)?
                            }
                            None => ret = Some(String::new()),
                        }
                    }
                    Cont::Memo { keys } => {
                        for key in keys {
                            if self.memo.len() < self.limits.memo && v.len() <= 1 << 16 {
                                self.memo.insert(key, v.clone());
                            }
                        }
                        ret = Some(v);
                    }
                    Cont::RefDone { id } => {
                        self.active.remove(&id);
                        if self.rendered.len() < 1 << 14 {
                            self.rendered.insert(id, Arc::from(v.as_str()));
                        }
                        ret = Some(v);
                    }
                }
                continue;
            }
            let Some(Cont::Run { run, pos, env, mut out }) = stack.pop() else {
                unreachable!("only a run steps")
            };
            if pos >= run.len() {
                ret = Some(out);
                continue;
            }
            let at = pos;
            let tail = at + 1 == run.len();
            match &run[at] {
                Seg::Text(t) => {
                    self.grow(&mut out, t)?;
                    stack.push(Cont::Run {
                        run,
                        pos: pos + 1,
                        env,
                        out,
                    });
                }
                Seg::Ref(id) => {
                    let id = *id;
                    stack.push(Cont::Run {
                        run,
                        pos: pos + 1,
                        env,
                        out,
                    });
                    ret = self.begin_ref(&mut stack, id)?;
                }
                Seg::Call { f, args } => {
                    let f = *f;
                    let first = args.first().cloned();
                    let site = run.clone();
                    stack.push(Cont::Run {
                        run,
                        pos: pos + 1,
                        env: env.clone(),
                        out,
                    });
                    match first {
                        None => ret = self.dispatch(&mut stack, f, Vec::new(), env, tail)?,
                        Some(first) => {
                            let tail_concat = tail && f == Callee::Builtin(builtin::CONCAT);
                            if tail_concat && site_args(&site, at) == 1 {
                                // concat(x) in tail position is x in tail position
                                self.replace_top(&mut stack, first, env);
                                continue;
                            }
                            if f == Callee::Builtin(builtin::IF) {
                                self.push(
                                    &mut stack,
                                    Cont::If {
                                        site,
                                        at,
                                        env: env.clone(),
                                        tail,
                                    },
                                )?;
                            } else {
                                self.push(
                                    &mut stack,
                                    Cont::Args {
                                        site,
                                        at,
                                        vals: Vec::new(),
                                        env: env.clone(),
                                        tail,
                                    },
                                )?;
                            }
                            self.push(
                                &mut stack,
                                Cont::Run {
                                    run: first,
                                    pos: 0,
                                    env,
                                    out: String::new(),
                                },
                            )?;
                        }
                    }
                }
            }
        }
    }

    /// a call with its arguments evaluated: a value now, or frames pushed (`None`)
    fn dispatch(
        &mut self,
        stack: &mut Vec<Cont>,
        f: Callee,
        vals: Vec<String>,
        env: Env,
        tail: bool,
    ) -> Result<Option<String>, EvalError> {
        let id = match f {
            Callee::Entry(id) => id,
            Callee::Builtin(b) => return self.builtin(stack, b, vals, env),
        };
        let key = (id, vals);
        if let Some(v) = self.memo.get(&key) {
            self.stats.memo_hits += 1;
            return Ok(Some(v.clone()));
        }
        let body = self.body(id)?;
        let env: Env = Rc::from(key.1.clone());
        if tail {
            // a tail call from a frame that has emitted nothing has that frame's value:
            // remember it under the memo just below (up to a few keys, so loops stay
            // flat), or put one there
            let n = stack.len();
            let fresh = matches!(stack.last(), Some(Cont::Run { out, .. }) if out.is_empty());
            if fresh
                && n >= 2
                && let Cont::Memo { keys } = &mut stack[n - 2]
            {
                if keys.len() < MEMO_KEYS {
                    keys.push(key);
                }
                self.replace_top(stack, body, env);
                return Ok(None);
            }
            if fresh {
                stack.pop();
                self.push(stack, Cont::Memo { keys: vec![key] })?;
                self.push(
                    stack,
                    Cont::Run {
                        run: body,
                        pos: 0,
                        env,
                        out: String::new(),
                    },
                )?;
            } else {
                self.replace_top(stack, body, env);
            }
        } else {
            self.push(stack, Cont::Memo { keys: vec![key] })?;
            self.push(
                stack,
                Cont::Run {
                    run: body,
                    pos: 0,
                    env,
                    out: String::new(),
                },
            )?;
        }
        Ok(None)
    }

    fn builtin(
        &mut self,
        stack: &mut Vec<Cont>,
        b: u8,
        vals: Vec<String>,
        env: Env,
    ) -> Result<Option<String>, EvalError> {
        use builtin::*;
        let size: usize = vals.iter().map(|v| v.len()).sum();
        self.burn(1 + size as u64 / 64)?;
        let name = NAMES.get(b as usize).copied().unwrap_or("reserved");
        let arg = |i: usize| vals.get(i).map(String::as_str).unwrap_or("");
        let v = match b {
            ARG => {
                let n = num(name, arg(0))?;
                usize::try_from(n)
                    .ok()
                    .and_then(|n| env.get(n).cloned())
                    .unwrap_or_default()
            }
            ARGC => env.len().to_string(),
            // reached only when `if` has no arguments
            IF => String::new(),
            EQ => truth(vals.windows(2).all(|w| w[0] == w[1])),
            NOT => truth(arg(0).is_empty()),
            CONCAT => {
                if size > self.limits.output {
                    return Err(EvalError::Output);
                }
                vals.concat()
            }
            LEN => arg(0).chars().count().to_string(),
            SUM => {
                let mode = if arg(1).is_empty() {
                    Mode::Ordinal
                } else {
                    Mode::parse(arg(1)).ok_or_else(|| EvalError::Builtin(name, format!("no sum mode {:?}", arg(1))))?
                };
                Measure::of(arg(0)).get(mode).to_string()
            }
            SLICE => {
                let chars: Vec<char> = arg(0).chars().collect();
                let n = chars.len() as i64;
                let at = |v: i64| (if v < 0 { n + v } else { v }).clamp(0, n) as usize;
                let from = at(num(name, arg(1))?);
                let to = if vals.len() > 2 {
                    at(num(name, arg(2))?)
                } else {
                    n as usize
                };
                chars[from..to.max(from)].iter().collect()
            }
            LOOKUP => {
                let id = arg(0)
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| EvalError::Builtin(name, format!("{:?} isn't an id", arg(0))))?;
                return self.begin_ref(stack, id);
            }
            ADD | SUB | MUL | DIV | MOD => {
                let mut acc = num(name, arg(0))?;
                for v in vals.iter().skip(1) {
                    let x = num(name, v)?;
                    let r = match b {
                        ADD => acc.checked_add(x),
                        SUB => acc.checked_sub(x),
                        MUL => acc.checked_mul(x),
                        DIV if x == 0 => return Err(EvalError::Builtin(name, "division by zero".into())),
                        DIV => acc.checked_div(x),
                        _ if x == 0 => return Err(EvalError::Builtin(name, "division by zero".into())),
                        _ => acc.checked_rem(x),
                    };
                    acc = r.ok_or_else(|| EvalError::Builtin(name, "overflow".into()))?;
                }
                acc.to_string()
            }
            LT => truth(num(name, arg(0))? < num(name, arg(1))?),
            _ => return Err(EvalError::Builtin(name, format!("builtin {b} is reserved"))),
        };
        Ok(Some(v))
    }
}

fn site_args(site: &Run, at: usize) -> usize {
    match &site[at] {
        Seg::Call { args, .. } => args.len(),
        _ => 0,
    }
}

/// a small way to write runs by hand (tests, examples, the backup's readable form)
pub mod build {
    use super::*;

    pub fn t(s: &str) -> Vec<Seg> {
        vec![Seg::text(s)]
    }

    pub fn r(id: u64) -> Vec<Seg> {
        vec![Seg::Ref(id)]
    }

    pub fn b(name: &str, args: Vec<Vec<Seg>>) -> Vec<Seg> {
        let code = builtin::by_name(name).unwrap_or_else(|| panic!("no builtin {name}"));
        vec![Seg::call(Callee::Builtin(code), args)]
    }

    pub fn f(id: u64, args: Vec<Vec<Seg>>) -> Vec<Seg> {
        vec![Seg::call(Callee::Entry(id), args)]
    }

    pub fn arg(n: usize) -> Vec<Seg> {
        b("arg", vec![t(&n.to_string())])
    }

    pub fn cat(parts: Vec<Vec<Seg>>) -> Vec<Seg> {
        parts.into_iter().flatten().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    fn src(entries: Vec<(u64, Vec<Seg>)>) -> HashMap<u64, Vec<Seg>> {
        entries.into_iter().collect()
    }

    #[test]
    fn refs_and_builtins() {
        let s = src(vec![
            (1, t("hello")),
            (2, cat(vec![r(1), t(" world")])),
            (3, cat(vec![t("<"), r(2), t(">")])),
            (4, b("len", vec![r(3)])),
            (5, b("slice", vec![r(2), t("-5")])),
            (6, b("sum", vec![t("hello")])),
            (7, b("sum", vec![t("hello"), t("atbash")])),
            (8, b("lookup", vec![t("1")])),
            (9, b("concat", vec![t("a"), t("b"), t("c")])),
            (10, b("if", vec![t(""), t("yes"), t("no")])),
            (11, b("div", vec![t("7"), t("0")])),
            (12, b("add", vec![t("9223372036854775807"), t("1")])),
            (13, b("mul", vec![t("x"), t("2")])),
            (14, b("eq", vec![t("a"), t("a"), t("a")])),
            (15, b("sub", vec![t("10"), t("3"), t("2")])),
            (16, b("mod", vec![t("-7"), t("3")])),
            (17, vec![Seg::call(Callee::Builtin(20), vec![])]),
            (18, b("if", vec![t("x"), t("only")])),
            (19, b("if", vec![t(""), t("only")])),
        ]);
        let mut i = Interp::new(&s);
        assert_eq!(i.render(2).unwrap(), "hello world");
        assert_eq!(i.render(3).unwrap(), "<hello world>");
        assert_eq!(i.render(4).unwrap(), "13");
        assert_eq!(i.render(5).unwrap(), "world");
        assert_eq!(i.render(6).unwrap(), "52");
        assert_eq!(i.render(7).unwrap(), ((83 - 5) * (52 - 5)).to_string());
        assert_eq!(i.render(8).unwrap(), "hello");
        assert_eq!(i.render(9).unwrap(), "abc");
        assert_eq!(i.render(10).unwrap(), "no");
        assert!(matches!(i.render(11), Err(EvalError::Builtin("div", _))));
        assert!(matches!(i.render(12), Err(EvalError::Builtin("add", _))));
        assert!(matches!(i.render(13), Err(EvalError::Builtin("mul", _))));
        assert_eq!(i.render(14).unwrap(), "1");
        assert_eq!(i.render(15).unwrap(), "5");
        assert_eq!(i.render(16).unwrap(), "-1");
        assert!(matches!(i.render(17), Err(EvalError::Builtin(..))));
        assert_eq!(i.render(18).unwrap(), "only");
        assert_eq!(i.render(19).unwrap(), "");
        assert_eq!(i.render(99), Err(EvalError::Missing(99)));
    }

    #[test]
    fn ref_cycles_are_errors() {
        let s = src(vec![
            (1, cat(vec![t("a"), r(2)])),
            (2, cat(vec![t("b"), r(1)])),
            (3, r(3)),
            (4, b("lookup", vec![t("4")])),
            // the same entry twice, not nested, is fine
            (5, cat(vec![r(6), r(6)])),
            (6, t("x")),
        ]);
        let mut i = Interp::new(&s);
        assert_eq!(i.render(1), Err(EvalError::Cycle(1)));
        assert_eq!(i.render(3), Err(EvalError::Cycle(3)));
        assert_eq!(i.render(4), Err(EvalError::Cycle(4)));
        assert_eq!(i.render(5).unwrap(), "xx");
    }

    /// fact(n) = if(eq(n, 0), 1, mul(n, fact(sub(n, 1))))
    fn fact() -> Vec<Seg> {
        b(
            "if",
            vec![
                b("eq", vec![arg(0), t("0")]),
                t("1"),
                b("mul", vec![arg(0), f(100, vec![b("sub", vec![arg(0), t("1")])])]),
            ],
        )
    }

    /// fib(n) = if(lt(n, 2), n, add(fib(n - 1), fib(n - 2)))
    fn fib() -> Vec<Seg> {
        b(
            "if",
            vec![
                b("lt", vec![arg(0), t("2")]),
                arg(0),
                b(
                    "add",
                    vec![
                        f(101, vec![b("sub", vec![arg(0), t("1")])]),
                        f(101, vec![b("sub", vec![arg(0), t("2")])]),
                    ],
                ),
            ],
        )
    }

    #[test]
    fn recursion_factorial_fibonacci() {
        let s = src(vec![(100, fact()), (101, fib())]);
        let mut i = Interp::new(&s);
        assert_eq!(i.call(100, &["10"]).unwrap(), "3628800");
        assert_eq!(i.call(100, &["20"]).unwrap(), "2432902008176640000");
        assert!(matches!(i.call(100, &["21"]), Err(EvalError::Builtin("mul", _))));
        // naive Fibonacci, linear thanks to the memo
        assert_eq!(i.call(101, &["80"]).unwrap(), "23416728348467685");
        assert!(i.stats.memo_hits > 70);
        // the same through a REF to an entry that calls it
        let mut s2 = s.clone();
        s2.insert(5, cat(vec![t("fib(30) = "), f(101, vec![t("30")])]));
        assert_eq!(Interp::new(&s2).render(5).unwrap(), "fib(30) = 832040");
    }

    /// count(n, limit) = if(eq(n, limit), n, count(add(n, 1), limit)): a tail call
    fn counter() -> Vec<Seg> {
        b(
            "if",
            vec![
                b("eq", vec![arg(0), arg(1)]),
                arg(0),
                f(200, vec![b("add", vec![arg(0), t("1")]), arg(1)]),
            ],
        )
    }

    #[test]
    fn counter_to_a_thousand() {
        let s = src(vec![(200, counter())]);
        let mut i = Interp::new(&s);
        assert_eq!(i.call(200, &["0", "1000"]).unwrap(), "1000");
    }

    #[test]
    fn tail_recursion_runs_in_constant_stack() {
        let s = src(vec![(200, counter())]);
        let mut i = Interp::with_limits(
            &s,
            Limits {
                fuel: 100_000_000,
                ..Limits::default()
            },
        );
        assert_eq!(i.call(200, &["0", "1000000"]).unwrap(), "1000000");
        assert!(i.stats.max_stack <= 8, "{:?}", i.stats);
        // nor is the memo filled by a tail loop
        assert!(i.memo.len() <= MEMO_KEYS);
        // too little fuel is an error, not a hang
        let mut i = Interp::with_limits(
            &s,
            Limits {
                fuel: 10_000,
                ..Limits::default()
            },
        );
        assert_eq!(i.call(200, &["0", "1000000"]), Err(EvalError::Fuel));
    }

    #[test]
    fn mutual_tail_recursion() {
        // even(n) = if(eq(n, 0), 1, odd(n - 1)); odd(n) = if(eq(n, 0), "", even(n - 1))
        let s = src(vec![
            (
                300,
                b(
                    "if",
                    vec![
                        b("eq", vec![arg(0), t("0")]),
                        t("1"),
                        f(301, vec![b("sub", vec![arg(0), t("1")])]),
                    ],
                ),
            ),
            (
                301,
                b(
                    "if",
                    vec![
                        b("eq", vec![arg(0), t("0")]),
                        t(""),
                        f(300, vec![b("sub", vec![arg(0), t("1")])]),
                    ],
                ),
            ),
        ]);
        let mut i = Interp::with_limits(
            &s,
            Limits {
                fuel: 100_000_000,
                ..Limits::default()
            },
        );
        assert_eq!(i.call(300, &["200001"]).unwrap(), "");
        assert_eq!(i.call(301, &["200001"]).unwrap(), "1");
        assert!(i.stats.max_stack <= 8);
    }

    #[test]
    fn tail_calls_after_text_and_in_concat() {
        // rep(n) = if(eq(n, 0), "", concat("x", rep(n - 1)))
        // bang(n) = "!" rep(n): text before a tail call
        let s = src(vec![
            (
                400,
                b(
                    "if",
                    vec![
                        b("eq", vec![arg(0), t("0")]),
                        t(""),
                        b("concat", vec![t("x"), f(400, vec![b("sub", vec![arg(0), t("1")])])]),
                    ],
                ),
            ),
            (401, cat(vec![t("!"), f(400, vec![arg(0)])])),
            (
                402,
                b(
                    "if",
                    vec![
                        b("eq", vec![arg(0), t("0")]),
                        t(""),
                        cat(vec![t("y"), f(402, vec![b("sub", vec![arg(0), t("1")])])]),
                    ],
                ),
            ),
        ]);
        let mut i = Interp::new(&s);
        let out = i.call(400, &["100000"]).unwrap();
        assert_eq!(out.len(), 100_000);
        assert!(i.stats.max_stack <= 8, "{:?}", i.stats);
        assert_eq!(i.call(401, &["3"]).unwrap(), "!xxx");
        let out = i.call(402, &["50000"]).unwrap();
        assert_eq!(out, "y".repeat(50_000));
        assert!(i.stats.max_stack <= 8);
        // the output limit stops a runaway
        let mut i = Interp::with_limits(
            &s,
            Limits {
                output: 1000,
                ..Limits::default()
            },
        );
        assert_eq!(i.call(400, &["5000"]), Err(EvalError::Output));
    }

    #[test]
    fn non_tail_recursion_hits_the_depth_limit() {
        // down(n) = if(eq(n, 0), 0, add(1, down(n - 1))): the call is an argument
        let s = src(vec![(
            500,
            b(
                "if",
                vec![
                    b("eq", vec![arg(0), t("0")]),
                    t("0"),
                    b("add", vec![t("1"), f(500, vec![b("sub", vec![arg(0), t("1")])])]),
                ],
            ),
        )]);
        let mut i = Interp::new(&s);
        assert_eq!(i.call(500, &["1000"]).unwrap(), "1000");
        let mut i = Interp::new(&s);
        assert_eq!(i.call(500, &["1000000"]), Err(EvalError::Depth));
        // a body calling itself forever through a non-tail position
        let s = src(vec![(501, cat(vec![f(501, vec![]), t("x")]))]);
        assert_eq!(Interp::new(&s).render(501), Err(EvalError::Depth));
        // ... and through a tail position: fuel
        let s = src(vec![(502, f(502, vec![]))]);
        let few = Limits {
            fuel: 100_000,
            ..Limits::default()
        };
        assert_eq!(Interp::with_limits(&s, few).render(502), Err(EvalError::Fuel));
    }
}
