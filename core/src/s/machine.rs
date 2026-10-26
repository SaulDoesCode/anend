//! The s machine (docs/s.md §3–§7, §11): the stack, `ow`, the namespaces, definitions
//! with generations, and its own stack of activations, so calls, definitions expanding and
//! units run never recurse in Rust. A call that is a unit's last operation replaces the
//! running activation (a tail call), so recursion in tail position runs in constant space.
//!
//! In anend a program runs as the spec's browser runtime does (§10, §14): files live in the
//! run's own file table (the host fills it), `<` runs a small built-in shell over them,
//! workers run in step with the program (`>t` with no pending result is an error), `tcp`
//! always fails, and output and input are the run's own texts. The host bounds how many
//! steps a run may take.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::Rc;

use super::lex::{Part, Tok, brace_source, lex};
use super::val::{V, num_text};

pub type NativeFn = fn(&mut Rt) -> Res;

/// why a run stopped early
#[derive(Clone, Debug, PartialEq)]
pub enum Stop {
    /// an error: its message
    Err(String),
    /// `<--`: the exit code
    Exit(i32),
}

pub type Res = Result<(), Stop>;

pub fn err<T>(m: impl Into<String>) -> Result<T, Stop> {
    Err(Stop::Err(m.into()))
}

#[derive(Clone)]
enum Code {
    Unit(Rc<Vec<Tok>>),
    /// a value whose text doesn't parse: pushed as data
    Data(Rc<str>),
    Native(NativeFn),
}

#[derive(Clone)]
struct Def {
    generation: u64,
    code: Code,
}

#[derive(Clone)]
pub struct Script {
    src: Rc<str>,
    bound: u64,
}

#[derive(Default, Clone)]
pub struct Scope {
    pub mem: HashMap<Rc<str>, V>,
    pub numbers: HashMap<Rc<str>, f64>,
    pub lists: HashMap<Rc<str>, Vec<V>>,
    pub scripts: HashMap<Rc<str>, Script>,
}

enum Kind {
    Plain,
    /// a repeated call: how many more runs after this one
    Repeat(u64),
    /// `&&&`: the next n and the count
    Each {
        name: Rc<str>,
        next: u64,
        count: u64,
    },
}

struct Act {
    unit: Rc<Vec<Tok>>,
    pc: usize,
    bound: Option<u64>,
    locals: Scope,
    kind: Kind,
}

struct Handle {
    path: String,
    pos: usize,
    eof: bool,
    append: bool,
}

struct Worker {
    rt: Box<Rt>,
    results: VecDeque<String>,
}

/// what a run gave
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Outcome {
    pub out: String,
    /// the error, when it ended on one (written `s: message`)
    pub err: Option<String>,
    pub code: i32,
    pub steps: u64,
}

pub const COMMANDS: &[&str] = &[
    ">", ">_", "<_", "<", "w", "+w", "mv", "r", "r@", "f", "+f", "@f", "f_", "_f", "?eof", "f-", "tcp", "+", "-", "*",
    "/", "^", "%", "n", "#prng", "`prng", "=", "!=", "&", ";", "==", "<<", ">>", "<<=", ">>=", "!!", "l", "-l", "[l]",
    "l=", "l-", "#l", "/l", "l/", "#s", "?)", "&&&", "><", "//", "<-", "<--", "(", ")", "~", "##", "t", "<t", ">t",
    "-t", "***",
];

/// the characters that make a word a suffix form
pub const SUFFIXES: &[char] = &['?', '!', '~', '$', '`', '#', ';', ':', '.', '@'];

pub fn is_command(w: &str) -> bool {
    COMMANDS.contains(&w)
}

/// xoshiro256**, seeded through splitmix64
#[derive(Clone)]
pub struct Prng {
    s: [u64; 4],
}

impl Prng {
    pub fn new(seed: u64, seed2: u64) -> Prng {
        let mut x = seed ^ seed2.rotate_left(32) ^ seed2.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut next = || {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        Prng {
            s: [next(), next(), next(), next()],
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let r = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        r
    }

    pub fn float(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub struct Rt {
    pub stack: Vec<V>,
    pub ow: bool,
    pub global: Scope,
    defs: HashMap<String, Vec<Def>>,
    /// the next definition's generation
    pub generation: u64,
    saved: Vec<Vec<V>>,
    acts: Vec<Act>,
    units: HashMap<Rc<str>, Rc<Vec<Tok>>>,
    /// the run's files: path → contents
    pub files: BTreeMap<String, String>,
    handles: HashMap<String, Handle>,
    workers: Vec<Option<Worker>>,
    pub out: String,
    stdin: VecDeque<String>,
    pub prng: Prng,
    pub steps: u64,
    pub max_steps: u64,
    pub max_out: usize,
    pub loaded: HashSet<String>,
    /// the most activations held at once (tail calls keep it flat)
    pub max_depth: usize,
}

fn whole(v: f64) -> Option<u64> {
    (v >= 0.0 && v.fract() == 0.0 && v < 1.8e19).then_some(v as u64)
}

impl Rt {
    pub fn new(stdin: &str, files: BTreeMap<String, String>, seed: u64) -> Rt {
        Rt {
            stack: Vec::new(),
            ow: false,
            global: Scope::default(),
            defs: HashMap::new(),
            generation: 1,
            saved: Vec::new(),
            acts: Vec::new(),
            units: HashMap::new(),
            files,
            handles: HashMap::new(),
            workers: Vec::new(),
            out: String::new(),
            stdin: stdin.lines().map(str::to_string).collect(),
            prng: Prng::new(seed, 0),
            steps: 0,
            max_steps: 20_000_000,
            max_out: 16 << 20,
            loaded: HashSet::new(),
            max_depth: 0,
        }
    }

    /// runs a program with its command-line arguments
    pub fn run_program(&mut self, src: &str, args: &[String]) -> Outcome {
        let list: Vec<V> = args.iter().map(|a| V::str(a)).collect();
        self.global.numbers.insert("argc".into(), args.len() as f64);
        self.global.lists.insert("args".into(), list);
        let r = self.unit(src).and_then(|u| {
            self.push_act(u, None, Kind::Plain);
            self.drive(0)
        });
        let (err, code) = match r {
            Ok(()) => (None, 0),
            Err(Stop::Exit(c)) => (None, c),
            Err(Stop::Err(m)) => (Some(m), 1),
        };
        Outcome {
            out: self.out.clone(),
            err,
            code,
            steps: self.steps,
        }
    }

    // ------------------------------------------------------------ units and activations

    fn unit(&mut self, src: &str) -> Result<Rc<Vec<Tok>>, Stop> {
        if let Some(u) = self.units.get(src) {
            return Ok(u.clone());
        }
        let toks = lex(src).map_err(|e| Stop::Err(format!("{e} in a unit")))?;
        let u = Rc::new(toks);
        if self.units.len() < 10_000 {
            self.units.insert(src.into(), u.clone());
        }
        Ok(u)
    }

    fn push_act(&mut self, unit: Rc<Vec<Tok>>, bound: Option<u64>, kind: Kind) {
        self.acts.push(Act {
            unit,
            pc: 0,
            bound,
            locals: Scope::default(),
            kind,
        });
    }

    /// runs `src` as a unit on the line, to its end (for natives applying a quotation)
    pub fn run_quote(&mut self, src: &str) -> Res {
        let u = self.unit(src)?;
        let floor = self.acts.len();
        self.push_act(u, None, Kind::Plain);
        self.drive(floor)
    }

    /// starts `src` as a unit (it runs when the native returns)
    pub fn run_src(&mut self, src: &str) -> Res {
        let u = self.unit(src)?;
        self.push_act(u, None, Kind::Plain);
        Ok(())
    }

    fn bound(&self) -> Option<u64> {
        self.acts.last().and_then(|a| a.bound)
    }

    /// runs activations until there are only `floor` left
    fn drive(&mut self, floor: usize) -> Res {
        while self.acts.len() > floor {
            self.max_depth = self.max_depth.max(self.acts.len());
            let top = self.acts.len() - 1;
            let a = &mut self.acts[top];
            if a.pc >= a.unit.len() {
                self.finish(top)?;
                continue;
            }
            let unit = a.unit.clone();
            let pc = a.pc;
            a.pc += 1;
            let last = pc + 1 == unit.len();
            self.steps += 1;
            if self.steps > self.max_steps {
                return err(format!("the run took more than {} steps", self.max_steps));
            }
            match &unit[pc] {
                Tok::Quote(t) => self.stack.push(V::str(t)),
                Tok::Brace(parts) => {
                    let mut s = String::new();
                    for p in parts {
                        match p {
                            Part::Text(t) => s.push_str(t),
                            Part::Var(n) => s.push_str(&self.lookup(n).text()),
                        }
                    }
                    self.stack.push(s.into());
                }
                Tok::Word(w) => self.word(w, last)?,
            }
            if self.out.len() > self.max_out {
                return err("the output grew past its limit");
            }
        }
        Ok(())
    }

    /// an activation ran to its end
    fn finish(&mut self, i: usize) -> Res {
        match &mut self.acts[i].kind {
            Kind::Plain => {
                self.acts.pop();
            }
            Kind::Repeat(left) => {
                self.stack.clear();
                if *left == 0 {
                    self.acts.pop();
                } else {
                    *left -= 1;
                    let a = &mut self.acts[i];
                    a.pc = 0;
                    a.locals = Scope::default();
                }
            }
            Kind::Each { name, next, count } => {
                if *next >= *count {
                    self.acts.pop();
                    return Ok(());
                }
                let n = *next;
                *next += 1;
                let name = name.clone();
                self.stack.clear();
                self.global.numbers.insert("n".into(), n as f64);
                self.call(&name, true, false)?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ names

    fn is_local(name: &str) -> bool {
        name.starts_with(':') && name.len() > 1
    }

    pub fn scope(&mut self, name: &str) -> &mut Scope {
        if Self::is_local(name)
            && let Some(a) = self.acts.last_mut()
        {
            return &mut a.locals;
        }
        &mut self.global
    }

    pub fn scope_ref(&self, name: &str) -> &Scope {
        if Self::is_local(name)
            && let Some(a) = self.acts.last()
        {
            return &a.locals;
        }
        &self.global
    }

    /// §4.3: mem, else numbers, else a random u64 for `?`, else 404
    pub fn lookup(&mut self, x: &str) -> V {
        let s = self.scope_ref(x);
        if let Some(v) = s.mem.get(x) {
            return v.clone();
        }
        if let Some(n) = s.numbers.get(x) {
            return V::N(*n);
        }
        if x == "?" {
            return V::from(self.prng.next_u64().to_string());
        }
        V::str("404")
    }

    pub fn mset(&mut self, name: &str, v: V) {
        self.scope(name).mem.insert(name.into(), v);
    }

    pub fn nset(&mut self, name: &str, x: f64) {
        self.scope(name).numbers.insert(name.into(), x);
    }

    pub fn nget(&self, name: &str) -> Option<f64> {
        self.scope_ref(name).numbers.get(name).copied()
    }

    pub fn list_mut(&mut self, name: &str) -> &mut Vec<V> {
        self.scope(name).lists.entry(name.into()).or_default()
    }

    pub fn list(&self, name: &str) -> Option<&Vec<V>> {
        self.scope_ref(name).lists.get(name)
    }

    // ------------------------------------------------------------ definitions

    fn visible(&self, w: &str) -> Option<Def> {
        let b = self.bound();
        self.defs
            .get(w)?
            .iter()
            .rev()
            .find(|d| b.is_none_or(|b| d.generation < b))
            .cloned()
    }

    /// `value word ><`
    pub fn define(&mut self, word: &str, value: &str) {
        let code = match lex(value) {
            Ok(t) => Code::Unit(Rc::new(t)),
            Err(_) => Code::Data(value.into()),
        };
        let generation = self.generation;
        self.generation += 1;
        self.defs
            .entry(word.to_string())
            .or_default()
            .push(Def { generation, code });
    }

    /// natives defined together at one generation (a library module loading)
    pub fn define_natives(&mut self, natives: &[(&str, NativeFn)]) {
        let generation = self.generation;
        self.generation += 1;
        for (name, f) in natives {
            self.defs.entry(name.to_string()).or_default().push(Def {
                generation,
                code: Code::Native(*f),
            });
        }
    }

    fn run_def(&mut self, d: Def) -> Res {
        match d.code {
            Code::Unit(u) => {
                self.push_act(u, Some(d.generation), Kind::Plain);
                Ok(())
            }
            Code::Data(t) => {
                self.stack.push(V::T(t));
                Ok(())
            }
            Code::Native(f) => f(self),
        }
    }

    // ------------------------------------------------------------ words

    fn word(&mut self, w: &str, last: bool) -> Res {
        if is_command(w) {
            if let Some(d) = self.visible(w) {
                return self.run_def(d);
            }
            return self.command(w, last);
        }
        let n = w.chars().count();
        if n >= 2 && w.starts_with('\\') {
            let rest = &w[1..];
            if is_command(rest) {
                return self.command(rest, last);
            }
            if !rest.ends_with(SUFFIXES) || rest.chars().count() < 2 {
                self.stack.push(V::str(rest));
                return Ok(());
            }
            // a `\` before a suffix form changes nothing: it's the suffix form
        }
        // ⟨word⟩, maybe with a suffix after it
        if w.starts_with('⟨')
            && let Some(close) = w.find('⟩')
        {
            let inner = &w['⟨'.len_utf8()..close];
            let after = &w[close + '⟩'.len_utf8()..];
            let mut cs = after.chars();
            match (cs.next(), cs.next()) {
                (None, _) => return self.literal(inner),
                (Some(c), None) if SUFFIXES.contains(&c) => return self.suffix(inner, c, last),
                _ => {}
            }
        }
        if n >= 2 {
            let c = w.chars().last().expect("a character");
            if SUFFIXES.contains(&c) {
                let stem = &w[..w.len() - c.len_utf8()];
                return self.suffix(stem, c, last);
            }
        }
        self.literal(w)
    }

    fn literal(&mut self, w: &str) -> Res {
        if let Some(d) = self.visible(w) {
            return self.run_def(d);
        }
        self.stack.push(V::str(w));
        Ok(())
    }

    /// §4.3: a one-word value runs as that word; anything else is pushed
    pub fn dispatch(&mut self, v: V, last: bool) -> Res {
        let t = v.text();
        if !t.is_empty() && !t.chars().any(char::is_whitespace) {
            return self.word(&t, last);
        }
        self.stack.push(v);
        Ok(())
    }

    fn joined(&mut self) -> V {
        if self.stack.len() == 1 {
            return self.stack.pop().expect("one value");
        }
        let s: Vec<String> = self.stack.drain(..).map(|v| v.text().to_string()).collect();
        V::from(s.join(" "))
    }

    fn suffix(&mut self, x: &str, c: char, last: bool) -> Res {
        match c {
            '?' => {
                if self.ow {
                    return self.word(x, last);
                }
            }
            '!' => {
                if !self.ow {
                    return self.word(x, last);
                }
            }
            '~' => {
                let v = self.lookup(x);
                return self.dispatch(v, last);
            }
            '@' => {
                let v = self
                    .stack
                    .pop()
                    .ok_or_else(|| Stop::Err(format!("{x}@ needs a value")))?;
                self.mset(x, v);
            }
            '`' => {
                let v = self.joined();
                self.mset(x, v);
            }
            '#' => {
                let v = self.stack.first().map_or(0.0, V::num);
                self.stack.clear();
                self.nset(x, v);
            }
            ';' | ':' => {
                let by = self.stack.first().map_or(1.0, V::num);
                self.stack.clear();
                let now = self.nget(x).unwrap_or(0.0);
                self.nset(x, if c == ';' { now + by } else { now - by });
            }
            '$' => {
                let s: Vec<String> = self.stack.drain(..).map(|v| v.text().to_string()).collect();
                let src: Rc<str> = s.join(" ").replace('|', "~").into();
                let bound = self.generation;
                self.scope(x).scripts.insert(x.into(), Script { src, bound });
            }
            '.' => {
                // `x..`: once, never a repeat
                if x.chars().count() >= 2 && x.ends_with('.') {
                    return self.call(&x[..x.len() - 1], true, last);
                }
                return self.call(x, false, last);
            }
            _ => unreachable!("a suffix"),
        }
        Ok(())
    }

    // ------------------------------------------------------------ calls

    /// §6: a script, else a file, else a library module
    pub fn call(&mut self, name: &str, once: bool, last: bool) -> Res {
        let script = self.scope_ref(name).scripts.get(name).cloned();
        let (unit, bound) = if let Some(s) = script {
            (self.unit(&s.src)?, Some(s.bound))
        } else if let Some(src) = self.files.get(name).cloned() {
            (self.unit(&src)?, None)
        } else if super::lib::load(self, name)? {
            return Ok(());
        } else {
            return err(format!("there's no script or file {name}"));
        };
        let mut kind = Kind::Plain;
        if !once && self.stack.len() == 1 {
            let c = self.stack.pop().expect("one value");
            let count = whole(c.num())
                .filter(|_| !c.text().trim().is_empty())
                .ok_or_else(|| Stop::Err(format!("{} isn't a repeat count", c.text())))?;
            self.stack.clear();
            if count == 0 {
                return Ok(());
            }
            kind = Kind::Repeat(count - 1);
        }
        // a call that is the unit's last operation replaces it
        if last
            && let Some(a) = self.acts.last_mut()
            && matches!(a.kind, Kind::Plain)
        {
            a.unit = unit;
            a.pc = 0;
            a.bound = bound;
            a.locals = Scope::default();
            a.kind = kind;
            return Ok(());
        }
        self.push_act(unit, bound, kind);
        Ok(())
    }

    // ------------------------------------------------------------ commands

    fn args(&mut self, who: &str, min: usize) -> Result<Vec<V>, Stop> {
        if self.stack.len() < min {
            return err(format!("{who} needs {min} value{}", if min == 1 { "" } else { "s" }));
        }
        Ok(std::mem::take(&mut self.stack))
    }

    /// a failure: with an error-code unit, mem[err] and that unit; else an error
    fn fail(&mut self, code: Option<&V>, msg: String) -> Res {
        match code {
            Some(c) => {
                self.global.mem.insert("err".into(), msg.into());
                self.stack.clear();
                let src = c.text();
                self.run_src(&src)
            }
            None => err(msg),
        }
    }

    fn file(&self, path: &str) -> Option<&String> {
        self.files.get(path)
    }

    fn command(&mut self, w: &str, last: bool) -> Res {
        let _ = last;
        match w {
            ">" => {
                let a = std::mem::take(&mut self.stack);
                self.out.push('\n');
                for v in a {
                    self.out.push(' ');
                    self.out.push_str(&v.text());
                }
            }
            ">_" => {
                for v in std::mem::take(&mut self.stack) {
                    self.out.push_str(&v.text());
                }
            }
            "<_" => {
                let a = std::mem::take(&mut self.stack);
                let dest = a.first().map_or_else(|| "_".into(), V::text);
                match self.stdin.pop_front() {
                    Some(l) => {
                        self.mset(&dest, l.into());
                        self.global.mem.insert("eof".into(), V::N(0.0));
                    }
                    None => {
                        self.mset(&dest, V::str(""));
                        self.global.mem.insert("eof".into(), V::N(1.0));
                    }
                }
            }
            "<" => {
                let a = self.args("<", 1)?;
                let (out, rc) = self.shell(&a[0].text());
                self.global.mem.insert("r".into(), out.into());
                self.global.numbers.insert("rc".into(), rc as f64);
            }
            "w" | "+w" => {
                let a = self.args(w, 2)?;
                let (p, t) = (a[0].text().to_string(), a[1].text());
                let f = self.files.entry(p).or_default();
                if w == "w" {
                    f.clear();
                }
                f.push_str(&t);
            }
            "mv" => {
                let a = self.args("mv", 2)?;
                let from = a[0].text().to_string();
                let c = self
                    .files
                    .remove(&from)
                    .ok_or_else(|| Stop::Err(format!("mv: there's no file {from}")))?;
                self.files.insert(a[1].text().to_string(), c);
            }
            "r" => {
                let a = self.args("r", 1)?;
                let p = a[0].text().to_string();
                let c = self
                    .file(&p)
                    .cloned()
                    .ok_or_else(|| Stop::Err(format!("r: there's no file {p}")))?;
                let dest = a.get(1).map_or_else(|| p.clone().into(), V::text);
                self.mset(&dest, c.into());
            }
            "r@" => {
                let a = self.args("r@", 4)?;
                let p = a[0].text().to_string();
                let c = self
                    .file(&p)
                    .cloned()
                    .ok_or_else(|| Stop::Err(format!("r@: there's no file {p}")))?;
                let (off, len) = (a[1].num().max(0.0) as usize, a[2].num().max(0.0) as usize);
                let b = c.as_bytes();
                let s = &b[off.min(b.len())..(off + len).min(b.len())];
                self.mset(&a[3].text(), String::from_utf8_lossy(s).into_owned().into());
            }
            "f" => {
                let a = self.args("f", 3)?;
                let (p, name, mode) = (
                    a[0].text().to_string(),
                    a[1].text().to_string(),
                    a[2].text().replace('b', ""),
                );
                let exists = self.files.contains_key(&p);
                match mode.as_str() {
                    "r" | "r+" if !exists => return self.fail(a.get(3), format!("f: there's no file {p}")),
                    "w" | "w+" => {
                        self.files.insert(p.clone(), String::new());
                    }
                    "a" | "a+" => {
                        self.files.entry(p.clone()).or_default();
                    }
                    "r" | "r+" => {}
                    m => return self.fail(a.get(3), format!("f: {m} isn't a mode")),
                }
                let append = mode.starts_with('a');
                self.handles.insert(
                    name,
                    Handle {
                        path: p,
                        pos: 0,
                        eof: false,
                        append,
                    },
                );
            }
            "+f" => {
                let a = self.args("+f", 3)?;
                let name = a[0].text().to_string();
                let Some(h) = self.handles.get(&name) else {
                    return self.fail(a.get(3), format!("+f: no handle {name}"));
                };
                let path = h.path.clone();
                let text = self.lookup(&a[2].text()).text();
                let off = a[1].num().max(0.0) as usize;
                let f = self.files.entry(path).or_default();
                let mut b = std::mem::take(f).into_bytes();
                if b.len() < off {
                    b.resize(off, b' ');
                }
                let end = (off + text.len()).min(b.len());
                b.splice(off..end, text.bytes());
                *f = String::from_utf8_lossy(&b).into_owned();
            }
            "@f" => {
                let a = self.args("@f", 4)?;
                let name = a[0].text().to_string();
                let Some(h) = self.handles.get(&name) else {
                    return err(format!("@f: no handle {name}"));
                };
                let c = self.files.get(&h.path).cloned().unwrap_or_default();
                let (off, len) = (a[1].num().max(0.0) as usize, a[2].num().max(0.0) as usize);
                let b = c.as_bytes();
                let s = &b[off.min(b.len())..(off + len).min(b.len())];
                let short = s.len() < len;
                let v = String::from_utf8_lossy(s).into_owned();
                if let Some(h) = self.handles.get_mut(&name) {
                    h.eof = short;
                }
                self.mset(&a[3].text(), v.into());
            }
            "f_" => {
                let a = self.args("f_", 2)?;
                let name = a[0].text().to_string();
                let Some(h) = self.handles.get(&name) else {
                    return self.fail(a.get(2), format!("f_: no handle {name}"));
                };
                let (path, pos, append) = (h.path.clone(), h.pos, h.append);
                let line = format!("{}\n", a[1].text());
                let f = self.files.entry(path).or_default();
                let at = if append { f.len() } else { pos.min(f.len()) };
                let mut b = std::mem::take(f).into_bytes();
                let end = (at + line.len()).min(b.len());
                b.splice(at..end, line.bytes());
                *f = String::from_utf8_lossy(&b).into_owned();
                if let Some(h) = self.handles.get_mut(&name) {
                    h.pos = at + line.len();
                }
            }
            "_f" => {
                let a = self.args("_f", 2)?;
                let name = a[0].text().to_string();
                let Some(h) = self.handles.get(&name) else {
                    return err(format!("_f: no handle {name}"));
                };
                let c = self.files.get(&h.path).cloned().unwrap_or_default();
                let pos = h.pos.min(c.len());
                let (line, next, eof) = if pos >= c.len() {
                    (String::new(), pos, true)
                } else {
                    let rest = &c[pos..];
                    match rest.find('\n') {
                        Some(i) => (rest[..i].trim_end_matches('\r').to_string(), pos + i + 1, false),
                        None => (rest.to_string(), c.len(), false),
                    }
                };
                if let Some(h) = self.handles.get_mut(&name) {
                    h.pos = next;
                    h.eof = eof;
                }
                self.mset(&a[1].text(), line.into());
            }
            "?eof" => {
                let a = self.args("?eof", 1)?;
                let e = self.handles.get(&*a[0].text()).is_none_or(|h| h.eof);
                self.global.mem.insert("eof".into(), V::N(if e { 1.0 } else { 0.0 }));
            }
            "f-" => {
                let a = self.args("f-", 1)?;
                self.handles.remove(&*a[0].text());
            }
            "tcp" => {
                let a = self.args("tcp", 3)?;
                return self.fail(a.get(3), "tcp isn't available here".into());
            }
            "+" | "-" | "*" | "/" | "^" | "%" => {
                let a = self.args(w, 2)?;
                let (x, y) = (a[0].num(), a[1].num());
                let r = match w {
                    "+" => x + y,
                    "-" => x - y,
                    "*" => x * y,
                    "/" => x / y,
                    "^" => x.powf(y),
                    _ => x.rem_euclid(y),
                };
                let dest = a.get(2).map_or_else(|| "=".into(), V::text);
                self.nset(&dest, r);
            }
            "n" => {
                let a = self.args("n", 2)?;
                let x = a[0].num();
                self.nset(&a[1].text(), x);
            }
            "#prng" | "`prng" => {
                let a = std::mem::take(&mut self.stack);
                if let Some(s) = a.first() {
                    let s2 = a.get(1).map_or(0, |v| v.num() as i64 as u64);
                    self.prng = Prng::new(s.num() as i64 as u64, s2);
                }
                let dest = a.get(2).map_or_else(|| "rn".into(), V::text);
                if w == "#prng" {
                    let x = self.prng.float();
                    self.nset(&dest, x);
                } else {
                    const L: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
                    let s: String = (0..32)
                        .map(|_| L[(self.prng.next_u64() % 52) as usize] as char)
                        .collect();
                    self.mset(&dest, s.into());
                }
            }
            "=" | "!=" | "&" | ";" | "==" | "<<" | ">>" | "<<=" | ">>=" => {
                let a = self.args(w, 2)?;
                let same = a[0].same(&a[1]);
                let (x, y) = (a[0].num(), a[1].num());
                self.ow = match w {
                    "=" => same,
                    "!=" => !same,
                    "&" => self.ow && same,
                    ";" => self.ow && !same,
                    "==" => x == y,
                    "<<" => x < y,
                    ">>" => x > y,
                    "<<=" => x <= y,
                    _ => x >= y,
                };
            }
            "!!" => {
                self.stack.clear();
                self.ow = !self.ow;
            }
            "l" => {
                let a = self.args("l", 2)?;
                self.list_mut(&a[1].text()).push(a[0].clone());
            }
            "-l" => {
                let a = self.args("-l", 1)?;
                let v = self.list_mut(&a[0].text()).pop().unwrap_or_else(|| V::str("404"));
                self.global.mem.insert("$".into(), v);
            }
            "[l]" => {
                let a = self.args("[l]", 3)?;
                let name = a[0].text();
                if let Some(l) = self.list(&name) {
                    let i = a[1].num() as i64;
                    let i = if i < 0 { l.len() as i64 + i } else { i };
                    if i >= 0 && (i as usize) < l.len() {
                        let v = l[i as usize].clone();
                        self.mset(&a[2].text(), v);
                    }
                }
            }
            "l=" => {
                let a = self.args("l=", 3)?;
                let l = self.list_mut(&a[1].text());
                let i = a[2].num() as i64;
                let i = if i < 0 { l.len() as i64 + i } else { i };
                if i >= 0 && (i as usize) < l.len() {
                    l[i as usize] = a[0].clone();
                }
            }
            "l-" => {
                let a = self.args("l-", 1)?;
                let n = a[0].text();
                self.scope(&n).lists.remove(&n);
            }
            "#l" => {
                let a = self.args("#l", 1)?;
                let n = self.list(&a[0].text()).map_or(0, Vec::len);
                let dest = a.get(1).map_or_else(|| "=".into(), V::text);
                self.nset(&dest, n as f64);
            }
            "/l" => {
                let a = self.args("/l", 2)?;
                let t = a[0].text();
                let parts: Vec<V> = match a.get(2).map(V::text) {
                    None => t.split_whitespace().map(V::str).collect(),
                    Some(s) if s.is_empty() => t.chars().map(|c| V::from(c.to_string())).collect(),
                    Some(s) => t.split(&*s).map(V::str).collect(),
                };
                *self.list_mut(&a[1].text()) = parts;
            }
            "l/" => {
                let a = self.args("l/", 2)?;
                let sep = a.get(2).map_or_else(|| " ".into(), V::text);
                let s: Vec<String> = self
                    .list(&a[0].text())
                    .map(|l| l.iter().map(|v| v.text().to_string()).collect())
                    .unwrap_or_default();
                self.mset(&a[1].text(), s.join(&sep).into());
            }
            "#s" => {
                let a = self.args("#s", 1)?;
                let n = a[0].text().chars().count();
                let dest = a.get(1).map_or_else(|| "=".into(), V::text);
                self.nset(&dest, n as f64);
            }
            "?)" => {
                let a = self.args("?)", 2)?;
                let table = lex(&a[0].text()).map_err(|e| Stop::Err(format!("?): {e}")))?;
                let words: Vec<String> = table
                    .iter()
                    .map(|t| match t {
                        Tok::Word(w) | Tok::Quote(w) => w.clone(),
                        Tok::Brace(p) => brace_source(p),
                    })
                    .collect();
                let key = a[1].text();
                let hit = words
                    .chunks(2)
                    .find(|p| p.len() == 2 && *p[0] == *key)
                    .map(|p| p[1].clone());
                let code = hit.or_else(|| a.get(2).map(|d| d.text().to_string()));
                if let Some(c) = code {
                    return self.run_src(&c);
                }
            }
            "&&&" => {
                let a = self.args("&&&", 2)?;
                let count = whole(a[0].num()).ok_or_else(|| Stop::Err(format!("{} isn't a count", a[0].text())))?;
                self.push_act(
                    Rc::new(Vec::new()),
                    self.bound(),
                    Kind::Each {
                        name: a[1].text(),
                        next: 0,
                        count,
                    },
                );
            }
            "><" => {
                let a = self.args("><", 2)?;
                self.define(&a[1].text(), &a[0].text());
            }
            "//" => self.stack.clear(),
            "<-" => {
                if let Some(a) = self.acts.last_mut() {
                    a.pc = a.unit.len();
                }
            }
            "<--" => {
                let code = self.stack.first().map_or(0.0, V::num) as i32;
                return Err(Stop::Exit(code));
            }
            "(" => {
                let k = self.stack.pop().ok_or_else(|| Stop::Err("( needs a count".into()))?;
                let k = (k.num().max(0.0) as usize).min(self.stack.len());
                let keep = self.stack.split_off(self.stack.len() - k);
                let rest = std::mem::replace(&mut self.stack, keep);
                self.saved.push(rest);
            }
            ")" => {
                let mut under = self.saved.pop().ok_or_else(|| Stop::Err(") with no frame".into()))?;
                under.append(&mut self.stack);
                self.stack = under;
            }
            "~" => {
                let c = self.stack.pop().ok_or_else(|| Stop::Err("~ needs code".into()))?;
                return self.run_src(&c.text());
            }
            "##" => self.stack.push(V::N(self.stack.len() as f64)),
            "t" => {
                let a = std::mem::take(&mut self.stack);
                for p in a {
                    let path = p.text().to_string();
                    let src = self
                        .files
                        .get(&path)
                        .cloned()
                        .ok_or_else(|| Stop::Err(format!("t: there's no file {path}")))?;
                    let mut rt = Box::new(Rt::new("", self.files.clone(), self.prng.next_u64()));
                    rt.max_steps = self.max_steps.saturating_sub(self.steps);
                    let o = rt.run_program(&src, &[]);
                    self.out.push_str(&o.out);
                    self.steps += o.steps;
                    if let Some(e) = o.err {
                        return err(e);
                    }
                    self.workers.push(Some(Worker {
                        rt,
                        results: VecDeque::new(),
                    }));
                    self.global
                        .numbers
                        .insert("threads".into(), (self.workers.len() - 1) as f64);
                }
            }
            "<t" | ">t" | "-t" => {
                let a = self.args(w, if w == "<t" { 2 } else { 1 })?;
                let i = a[if w == "<t" { 1 } else { 0 }].num();
                let ok = i >= 0.0 && i.fract() == 0.0 && (i as usize) < self.workers.len();
                if !ok {
                    self.global.mem.insert("err".into(), V::str("Invalid thread index"));
                    return Ok(());
                }
                let i = i as usize;
                if self.workers[i].is_none() {
                    self.global.mem.insert("err".into(), V::str("thread closed"));
                    return Ok(());
                }
                match w {
                    "-t" => self.workers[i] = None,
                    "<t" => {
                        let code = a[0].text();
                        if &*code == "__stop__" {
                            self.workers[i] = None;
                            return Ok(());
                        }
                        let wk = self.workers[i].as_mut().expect("a worker");
                        wk.rt.out.clear();
                        let r = wk.rt.run_quote(&code);
                        let out = std::mem::take(&mut wk.rt.out);
                        if let Some(v) = wk.rt.global.mem.remove("out") {
                            wk.results.push_back(v.text().to_string());
                        }
                        self.out.push_str(&out);
                        if let Err(Stop::Err(e)) = r {
                            return err(e);
                        }
                    }
                    _ => {
                        let wk = self.workers[i].as_mut().expect("a worker");
                        let Some(res) = wk.results.pop_front() else {
                            return err(format!(">t: worker {i} has no result waiting"));
                        };
                        return self.run_src(&res);
                    }
                }
            }
            "***" => {
                let a = std::mem::take(&mut self.stack);
                let mut outs = Vec::new();
                for p in a {
                    let path = p.text().to_string();
                    let src = self
                        .files
                        .get(&path)
                        .cloned()
                        .ok_or_else(|| Stop::Err(format!("***: there's no file {path}")))?;
                    let mut rt = Rt::new("", self.files.clone(), self.prng.next_u64());
                    rt.max_steps = self.max_steps.saturating_sub(self.steps);
                    let o = rt.run_program(&src, &[]);
                    self.out.push_str(&o.out);
                    self.steps += o.steps;
                    if let Some(e) = o.err {
                        return err(e);
                    }
                    outs.push(rt.global.mem.get("out").map(|v| v.text().to_string()));
                }
                // each one's out, in order, as units
                for o in outs.into_iter().rev().flatten() {
                    self.run_src(&o)?;
                }
            }
            _ => return err(format!("{w} isn't a command")),
        }
        Ok(())
    }

    /// `<`: the built-in shell (echo, cat, ls, rm, cp, mv, pwd, true, false, exit) over the
    /// run's files: its output and exit code
    pub fn shell(&mut self, cmd: &str) -> (String, i32) {
        let words: Vec<&str> = cmd.split_whitespace().collect();
        let Some((&prog, args)) = words.split_first() else {
            return (String::new(), 0);
        };
        match prog {
            "echo" => (format!("{}\n", args.join(" ")), 0),
            "cat" => {
                let mut out = String::new();
                for a in args {
                    match self.files.get(*a) {
                        Some(c) => out.push_str(c),
                        None => return (out, 1),
                    }
                }
                (out, 0)
            }
            "ls" => {
                let names: Vec<&String> = self.files.keys().collect();
                (names.iter().map(|n| format!("{n}\n")).collect(), 0)
            }
            "rm" => {
                let mut rc = 0;
                for a in args {
                    if self.files.remove(*a).is_none() {
                        rc = 1;
                    }
                }
                (String::new(), rc)
            }
            "cp" | "mv" if args.len() == 2 => match self.files.get(args[0]).cloned() {
                Some(c) => {
                    if prog == "mv" {
                        self.files.remove(args[0]);
                    }
                    self.files.insert(args[1].to_string(), c);
                    (String::new(), 0)
                }
                None => (String::new(), 1),
            },
            "pwd" => ("/\n".into(), 0),
            "true" => (String::new(), 0),
            "false" => (String::new(), 1),
            "exit" => (String::new(), args.first().and_then(|a| a.parse().ok()).unwrap_or(0)),
            _ => (String::new(), 127),
        }
    }

    // ------------------------------------------------------------ for natives

    pub fn pop(&mut self, who: &str) -> Result<V, Stop> {
        self.stack
            .pop()
            .ok_or_else(|| Stop::Err(format!("{who} needs a value")))
    }

    pub fn pop_num(&mut self, who: &str) -> Result<f64, Stop> {
        Ok(self.pop(who)?.num())
    }

    pub fn pop_text(&mut self, who: &str) -> Result<Rc<str>, Stop> {
        Ok(self.pop(who)?.text())
    }

    pub fn push_num(&mut self, x: f64) {
        self.stack.push(V::N(x));
    }

    pub fn push_text(&mut self, s: impl Into<String>) {
        self.stack.push(V::from(s.into()));
    }

    pub fn push(&mut self, v: V) {
        self.stack.push(v);
    }
}

/// a value written as code that pushes it
pub fn quoted(t: &str) -> String {
    format!("'{t}'")
}

/// a number's text (for natives)
pub fn fmt(x: f64) -> String {
    num_text(x)
}
