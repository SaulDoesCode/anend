//! s's library (docs/s.md §12): `std/core.s`, `std/line.s`, `std/text.s`, `std/gem.s`,
//! `std/sh.s`, `std/logic.s` and `std/elec.s`, loaded with `std/name.s.` on an empty
//! stack. Each module's words are natives, defined at the generation where it loads, so
//! code written before the load doesn't see them and a later definition can wrap or
//! replace any of them. A module loads what it needs first and loads once per run.
//!
//! In anend the `sh` module works over the run's own files (as §10's browser runtime
//! does): `sh` runs the built-in shell, `env` answers 404, and `sleep` returns at once.

use super::machine::{NativeFn, Res, Rt, Stop, err};
use super::val::V;

type Natives = &'static [(&'static str, NativeFn)];

/// a module: what it loads first, and its words
fn module(name: &str) -> Option<(&'static [&'static str], Natives)> {
    Some(match name {
        "core" => (&[], CORE),
        "line" => (&["core"], LINE),
        "text" => (&["line"], TEXT),
        "gem" => (&["text"], GEM),
        "sh" => (&["text"], SH),
        "logic" => (&["core"], LOGIC),
        "elec" => (&["line"], ELEC),
        "anend" => (&[], ANEND),
        _ => return None,
    })
}

/// loads `std/name.s` (or registers `std:name`); false when `name` isn't a module
pub fn load(rt: &mut Rt, call: &str) -> Result<bool, Stop> {
    let name = match call
        .strip_prefix("std/")
        .and_then(|n| n.strip_suffix(".s"))
        .or_else(|| call.strip_prefix("std:"))
    {
        Some(n) => n,
        None => return Ok(false),
    };
    let Some((deps, words)) = module(name) else {
        return Ok(false);
    };
    if rt.loaded.contains(name) {
        return Ok(true);
    }
    rt.loaded.insert(name.to_string());
    // the caller's line waits while it loads
    let saved = std::mem::take(&mut rt.stack);
    for d in deps {
        load(rt, &format!("std:{d}"))?;
    }
    rt.define_natives(words);
    rt.stack = saved;
    Ok(true)
}

// ------------------------------------------------------------ helpers

fn need(rt: &Rt, n: usize, who: &str) -> Res {
    if rt.stack.len() < n {
        return err(format!("{who} needs {n} value{}", if n == 1 { "" } else { "s" }));
    }
    Ok(())
}

fn num1(rt: &mut Rt, who: &str, f: impl Fn(f64) -> f64) -> Res {
    let a = rt.pop_num(who)?;
    rt.push_num(f(a));
    Ok(())
}

fn num2(rt: &mut Rt, who: &str, f: impl Fn(f64, f64) -> f64) -> Res {
    need(rt, 2, who)?;
    let b = rt.pop_num(who)?;
    let a = rt.pop_num(who)?;
    rt.push_num(f(a, b));
    Ok(())
}

fn cmp2(rt: &mut Rt, who: &str, f: impl Fn(f64, f64) -> bool) -> Res {
    need(rt, 2, who)?;
    let b = rt.pop_num(who)?;
    let a = rt.pop_num(who)?;
    rt.ow = f(a, b);
    Ok(())
}

fn text1(rt: &mut Rt, who: &str, f: impl Fn(&str) -> String) -> Res {
    let a = rt.pop_text(who)?;
    rt.push_text(f(&a));
    Ok(())
}

fn is_num(t: &str) -> bool {
    let t = t.trim();
    !t.is_empty() && (t.parse::<f64>().is_ok() || matches!(t, "inf" | "-inf" | "NaN"))
}

fn int(x: f64) -> i64 {
    x as i64
}

/// runs `q` on a copy of `top`, apart from the line: its ow
fn test(rt: &mut Rt, top: V, q: &str) -> Result<bool, Stop> {
    let line = std::mem::replace(&mut rt.stack, vec![top]);
    let r = rt.run_quote(q);
    rt.stack = line;
    r?;
    Ok(rt.ow)
}

/// runs `q` on one value apart from the line: what it leaves
fn apply(rt: &mut Rt, x: V, q: &str) -> Result<Vec<V>, Stop> {
    let line = std::mem::replace(&mut rt.stack, vec![x]);
    let r = rt.run_quote(q);
    let out = std::mem::replace(&mut rt.stack, line);
    r?;
    Ok(out)
}

fn line(rt: &mut Rt) -> Vec<V> {
    std::mem::take(&mut rt.stack)
}

// ------------------------------------------------------------ std/core.s

const CORE: Natives = &[
    ("drop", |rt| rt.pop("drop").map(|_| ())),
    ("dup", |rt| {
        let a = rt.pop("dup")?;
        rt.push(a.clone());
        rt.push(a);
        Ok(())
    }),
    ("swap", |rt| {
        need(rt, 2, "swap")?;
        let n = rt.stack.len();
        rt.stack.swap(n - 1, n - 2);
        Ok(())
    }),
    ("over", |rt| {
        need(rt, 2, "over")?;
        let a = rt.stack[rt.stack.len() - 2].clone();
        rt.push(a);
        Ok(())
    }),
    ("rot", |rt| {
        need(rt, 3, "rot")?;
        let n = rt.stack.len();
        let a = rt.stack.remove(n - 3);
        rt.push(a);
        Ok(())
    }),
    ("nip", |rt| {
        need(rt, 2, "nip")?;
        let n = rt.stack.len();
        rt.stack.remove(n - 2);
        Ok(())
    }),
    ("tuck", |rt| {
        need(rt, 2, "tuck")?;
        let b = rt.stack[rt.stack.len() - 1].clone();
        let n = rt.stack.len();
        rt.stack.insert(n - 2, b);
        Ok(())
    }),
    ("2dup", |rt| {
        need(rt, 2, "2dup")?;
        let n = rt.stack.len();
        let (a, b) = (rt.stack[n - 2].clone(), rt.stack[n - 1].clone());
        rt.push(a);
        rt.push(b);
        Ok(())
    }),
    ("2drop", |rt| {
        need(rt, 2, "2drop")?;
        let n = rt.stack.len();
        rt.stack.truncate(n - 2);
        Ok(())
    }),
    ("pick", |rt| {
        let k = rt.pop_num("pick")?.max(0.0) as usize;
        need(rt, k + 1, "pick")?;
        let v = rt.stack[rt.stack.len() - 1 - k].clone();
        rt.push(v);
        Ok(())
    }),
    ("clear", |rt| {
        rt.stack.clear();
        Ok(())
    }),
    ("depth", |rt| {
        rt.push_num(rt.stack.len() as f64);
        Ok(())
    }),
    ("add", |rt| num2(rt, "add", |a, b| a + b)),
    ("sub", |rt| num2(rt, "sub", |a, b| a - b)),
    ("mul", |rt| num2(rt, "mul", |a, b| a * b)),
    ("div", |rt| num2(rt, "div", |a, b| a / b)),
    ("mod", |rt| num2(rt, "mod", |a, b| a - b * (a / b).floor())),
    ("pow", |rt| num2(rt, "pow", f64::powf)),
    ("idiv", |rt| num2(rt, "idiv", |a, b| (a / b).floor())),
    ("neg", |rt| num1(rt, "neg", |a| -a)),
    ("inc", |rt| num1(rt, "inc", |a| a + 1.0)),
    ("dec", |rt| num1(rt, "dec", |a| a - 1.0)),
    ("abs", |rt| {
        let a = rt.pop_num("abs")?;
        rt.ow = a < 0.0;
        rt.push_num(a.abs());
        Ok(())
    }),
    ("min", |rt| {
        need(rt, 2, "min")?;
        let b = rt.pop_num("min")?;
        let a = rt.pop_num("min")?;
        rt.ow = a < b;
        rt.push_num(a.min(b));
        Ok(())
    }),
    ("max", |rt| {
        need(rt, 2, "max")?;
        let b = rt.pop_num("max")?;
        let a = rt.pop_num("max")?;
        rt.ow = a > b;
        rt.push_num(a.max(b));
        Ok(())
    }),
    ("clamp", |rt| {
        need(rt, 3, "clamp")?;
        let hi = rt.pop_num("clamp")?;
        let lo = rt.pop_num("clamp")?;
        let x = rt.pop_num("clamp")?;
        rt.push_num(x.max(lo).min(hi));
        Ok(())
    }),
    ("floor", |rt| num1(rt, "floor", f64::floor)),
    ("ceil", |rt| num1(rt, "ceil", f64::ceil)),
    ("round", |rt| num1(rt, "round", f64::round)),
    ("trunc", |rt| num1(rt, "trunc", f64::trunc)),
    ("sign", |rt| {
        num1(rt, "sign", |a| {
            if a > 0.0 {
                1.0
            } else if a < 0.0 {
                -1.0
            } else {
                0.0
            }
        })
    }),
    ("sqrt", |rt| num1(rt, "sqrt", f64::sqrt)),
    ("hypot", |rt| num2(rt, "hypot", f64::hypot)),
    ("exp", |rt| num1(rt, "exp", f64::exp)),
    ("ln", |rt| num1(rt, "ln", f64::ln)),
    ("log", |rt| num1(rt, "log", f64::log10)),
    ("log2", |rt| num1(rt, "log2", f64::log2)),
    ("sin", |rt| num1(rt, "sin", f64::sin)),
    ("cos", |rt| num1(rt, "cos", f64::cos)),
    ("tan", |rt| num1(rt, "tan", f64::tan)),
    ("asin", |rt| num1(rt, "asin", f64::asin)),
    ("acos", |rt| num1(rt, "acos", f64::acos)),
    ("atan", |rt| num1(rt, "atan", f64::atan)),
    ("atan2", |rt| num2(rt, "atan2", f64::atan2)),
    ("pi", |rt| {
        rt.push_num(std::f64::consts::PI);
        Ok(())
    }),
    ("tau", |rt| {
        rt.push_num(std::f64::consts::TAU);
        Ok(())
    }),
    ("rand", |rt| {
        let x = rt.prng.float();
        rt.push_num(x);
        Ok(())
    }),
    ("randint", |rt| {
        need(rt, 2, "randint")?;
        let hi = rt.pop_num("randint")?.floor();
        let lo = rt.pop_num("randint")?.ceil();
        let span = (hi - lo + 1.0).max(1.0);
        let x = lo + (rt.prng.float() * span).floor();
        rt.push_num(x);
        Ok(())
    }),
    ("band", |rt| num2(rt, "band", |a, b| (int(a) & int(b)) as f64)),
    ("bor", |rt| num2(rt, "bor", |a, b| (int(a) | int(b)) as f64)),
    ("bxor", |rt| num2(rt, "bxor", |a, b| (int(a) ^ int(b)) as f64)),
    ("bnot", |rt| num1(rt, "bnot", |a| (!int(a)) as f64)),
    ("shl", |rt| {
        num2(rt, "shl", |a, b| int(a).checked_shl(int(b) as u32).unwrap_or(0) as f64)
    }),
    ("shr", |rt| {
        num2(rt, "shr", |a, b| int(a).checked_shr(int(b) as u32).unwrap_or(0) as f64)
    }),
    ("lt", |rt| cmp2(rt, "lt", |a, b| a < b)),
    ("gt", |rt| cmp2(rt, "gt", |a, b| a > b)),
    ("le", |rt| cmp2(rt, "le", |a, b| a <= b)),
    ("ge", |rt| cmp2(rt, "ge", |a, b| a >= b)),
    ("eq", |rt| cmp2(rt, "eq", |a, b| a == b)),
    ("ne", |rt| cmp2(rt, "ne", |a, b| a != b)),
    ("same", |rt| same(rt, true)),
    ("differ", |rt| same(rt, false)),
    ("between", |rt| {
        need(rt, 3, "between")?;
        let hi = rt.pop_num("between")?;
        let lo = rt.pop_num("between")?;
        let x = rt.pop_num("between")?;
        rt.ow = lo <= x && x <= hi;
        Ok(())
    }),
    ("even", |rt| {
        let a = rt.pop_num("even")?;
        rt.ow = a.rem_euclid(2.0) == 0.0;
        Ok(())
    }),
    ("odd", |rt| {
        let a = rt.pop_num("odd")?;
        rt.ow = a.rem_euclid(2.0) == 1.0;
        Ok(())
    }),
    ("isnum", |rt| {
        let a = rt.pop_text("isnum")?;
        rt.ow = is_num(&a);
        Ok(())
    }),
    ("not", |rt| {
        rt.ow = !rt.ow;
        Ok(())
    }),
    ("yes", |rt| {
        rt.ow = true;
        Ok(())
    }),
    ("no", |rt| {
        rt.ow = false;
        Ok(())
    }),
    ("call", |rt| {
        let q = rt.pop_text("call")?;
        rt.run_quote(&q)
    }),
    ("if", |rt| {
        need(rt, 2, "if")?;
        let f = rt.pop_text("if")?;
        let t = rt.pop_text("if")?;
        rt.run_quote(if rt.ow { &t } else { &f })
    }),
    ("when", |rt| {
        let q = rt.pop_text("when")?;
        if rt.ow { rt.run_quote(&q) } else { Ok(()) }
    }),
    ("unless", |rt| {
        let q = rt.pop_text("unless")?;
        if rt.ow { Ok(()) } else { rt.run_quote(&q) }
    }),
    ("test", |rt| {
        let q = rt.pop_text("test")?;
        let top = rt.stack.last().cloned().unwrap_or_else(|| V::str(""));
        let ow = test(rt, top, &q)?;
        rt.ow = ow;
        Ok(())
    }),
    ("and", |rt| {
        need(rt, 2, "and")?;
        let q = rt.pop_text("and")?;
        let p = rt.pop_text("and")?;
        rt.run_quote(&p)?;
        if rt.ow {
            rt.run_quote(&q)?;
        }
        Ok(())
    }),
    ("or", |rt| {
        need(rt, 2, "or")?;
        let q = rt.pop_text("or")?;
        let p = rt.pop_text("or")?;
        rt.run_quote(&p)?;
        if !rt.ow {
            rt.run_quote(&q)?;
        }
        Ok(())
    }),
    ("dip", |rt| {
        need(rt, 2, "dip")?;
        let q = rt.pop_text("dip")?;
        let x = rt.pop("dip")?;
        rt.run_quote(&q)?;
        rt.push(x);
        Ok(())
    }),
    ("keep", |rt| {
        need(rt, 2, "keep")?;
        let q = rt.pop_text("keep")?;
        let x = rt.stack.last().cloned().expect("a value");
        rt.run_quote(&q)?;
        rt.push(x);
        Ok(())
    }),
    ("bi", |rt| {
        need(rt, 3, "bi")?;
        let q = rt.pop_text("bi")?;
        let p = rt.pop_text("bi")?;
        let x = rt.pop("bi")?;
        rt.push(x.clone());
        rt.run_quote(&p)?;
        rt.push(x);
        rt.run_quote(&q)
    }),
    ("times", |rt| {
        need(rt, 2, "times")?;
        let n = rt.pop_num("times")?.max(0.0) as u64;
        let q = rt.pop_text("times")?;
        for _ in 0..n {
            rt.run_quote(&q)?;
        }
        Ok(())
    }),
    ("while", |rt| {
        need(rt, 2, "while")?;
        let b = rt.pop_text("while")?;
        let c = rt.pop_text("while")?;
        loop {
            let top = rt.stack.last().cloned().unwrap_or_else(|| V::str(""));
            if !test(rt, top, &c)? {
                break;
            }
            rt.run_quote(&b)?;
        }
        Ok(())
    }),
    ("until", |rt| {
        need(rt, 2, "until")?;
        let c = rt.pop_text("until")?;
        let b = rt.pop_text("until")?;
        loop {
            rt.run_quote(&b)?;
            let top = rt.stack.last().cloned().unwrap_or_else(|| V::str(""));
            if test(rt, top, &c)? {
                break;
            }
        }
        Ok(())
    }),
];

/// same or different text, numbers compared as numbers
fn same(rt: &mut Rt, want: bool) -> Res {
    need(rt, 2, "same")?;
    let b = rt.pop("same")?;
    let a = rt.pop("same")?;
    let (ta, tb) = (a.text(), b.text());
    let eq = if is_num(&ta) && is_num(&tb) {
        a.num() == b.num()
    } else {
        ta == tb
    };
    rt.ow = eq == want;
    Ok(())
}

// ------------------------------------------------------------ std/line.s

fn sorted(mut xs: Vec<V>, desc: bool) -> Vec<V> {
    xs.sort_by(|a, b| {
        let o = a.num().total_cmp(&b.num());
        if desc { o.reverse() } else { o }
    });
    xs
}

fn fold(rt: &mut Rt, who: &str, right: bool, running: bool) -> Res {
    let q = rt.pop_text(who)?;
    let mut xs = line(rt);
    if right {
        xs.reverse();
    }
    let mut it = xs.into_iter();
    let Some(mut acc) = it.next() else { return Ok(()) };
    let mut steps = vec![acc.clone()];
    for x in it {
        rt.stack = if right { vec![x, acc] } else { vec![acc, x] };
        rt.run_quote(&q)?;
        acc = rt.stack.pop().unwrap_or_else(|| V::str(""));
        steps.push(acc.clone());
    }
    rt.stack = if running { steps } else { vec![acc] };
    Ok(())
}

const LINE: Natives = &[
    ("gather", |rt| {
        let name = rt.pop_text("gather")?;
        let xs = line(rt);
        *rt.list_mut(&name) = xs;
        Ok(())
    }),
    ("spill", |rt| {
        let name = rt.pop_text("spill")?;
        let xs = rt.list(&name).cloned().unwrap_or_default();
        rt.stack.extend(xs);
        Ok(())
    }),
    ("show", |rt| {
        rt.out.push('\n');
        let s: Vec<String> = rt.stack.iter().map(|v| v.text().to_string()).collect();
        for v in s {
            rt.out.push(' ');
            rt.out.push_str(&v);
        }
        Ok(())
    }),
    ("count", |rt| {
        let n = rt.stack.len();
        rt.stack = vec![V::N(n as f64)];
        Ok(())
    }),
    ("map", |rt| {
        let q = rt.pop_text("map")?;
        let mut out = Vec::new();
        for x in line(rt) {
            out.extend(apply(rt, x, &q)?);
        }
        rt.stack = out;
        Ok(())
    }),
    ("filter", |rt| {
        let q = rt.pop_text("filter")?;
        let mut out = Vec::new();
        for x in line(rt) {
            if test(rt, x.clone(), &q)? {
                out.push(x);
            }
        }
        rt.stack = out;
        Ok(())
    }),
    ("fold", |rt| fold(rt, "fold", false, false)),
    ("foldr", |rt| fold(rt, "foldr", true, false)),
    ("scan", |rt| fold(rt, "scan", false, true)),
    ("rev", |rt| {
        rt.stack.reverse();
        Ok(())
    }),
    ("sort", |rt| {
        let xs = line(rt);
        rt.stack = sorted(xs, false);
        Ok(())
    }),
    ("sortdesc", |rt| {
        let xs = line(rt);
        rt.stack = sorted(xs, true);
        Ok(())
    }),
    ("dedup", |rt| {
        let mut seen = std::collections::HashSet::new();
        let xs = line(rt);
        rt.stack = xs.into_iter().filter(|v| seen.insert(v.text())).collect();
        Ok(())
    }),
    ("sum", |rt| {
        let s: f64 = line(rt).iter().map(V::num).sum();
        rt.push_num(s);
        Ok(())
    }),
    ("product", |rt| {
        let s: f64 = line(rt).iter().map(V::num).product();
        rt.push_num(s);
        Ok(())
    }),
    ("mean", |rt| {
        let xs = line(rt);
        let m = if xs.is_empty() {
            0.0
        } else {
            xs.iter().map(V::num).sum::<f64>() / xs.len() as f64
        };
        rt.push_num(m);
        Ok(())
    }),
    ("most", |rt| {
        let xs = line(rt);
        if let Some(m) = xs.iter().map(V::num).reduce(f64::max) {
            rt.push_num(m);
        }
        Ok(())
    }),
    ("least", |rt| {
        let xs = line(rt);
        if let Some(m) = xs.iter().map(V::num).reduce(f64::min) {
            rt.push_num(m);
        }
        Ok(())
    }),
    ("iota", |rt| {
        let n = rt.pop_num("iota")?.max(0.0) as u64;
        for i in 1..=n {
            rt.push_num(i as f64);
        }
        Ok(())
    }),
    ("til", |rt| {
        let n = rt.pop_num("til")?.max(0.0) as u64;
        for i in 0..n {
            rt.push_num(i as f64);
        }
        Ok(())
    }),
    ("range", |rt| {
        need(rt, 2, "range")?;
        let b = rt.pop_num("range")?;
        let a = rt.pop_num("range")?;
        let step = if b >= a { 1.0 } else { -1.0 };
        let mut x = a;
        while (step > 0.0 && x <= b) || (step < 0.0 && x >= b) {
            rt.push_num(x);
            x += step;
        }
        Ok(())
    }),
    ("take", |rt| {
        let n = rt.pop_num("take")?.max(0.0) as usize;
        rt.stack.truncate(n);
        Ok(())
    }),
    ("skip", |rt| {
        let n = (rt.pop_num("skip")?.max(0.0) as usize).min(rt.stack.len());
        rt.stack.drain(..n);
        Ok(())
    }),
    ("first", |rt| {
        let xs = line(rt);
        rt.push(xs.into_iter().next().unwrap_or_else(|| V::str("404")));
        Ok(())
    }),
    ("last", |rt| {
        let xs = line(rt);
        rt.push(xs.into_iter().last().unwrap_or_else(|| V::str("404")));
        Ok(())
    }),
    ("at", |rt| {
        let i = rt.pop_num("at")? as i64;
        let xs = line(rt);
        let i = if i < 0 { xs.len() as i64 + i } else { i };
        rt.push(if i >= 0 && (i as usize) < xs.len() {
            xs[i as usize].clone()
        } else {
            V::str("404")
        });
        Ok(())
    }),
    ("index", |rt| {
        let v = rt.pop("index")?;
        let xs = line(rt);
        let i = xs.iter().position(|x| x.same(&v)).map_or(-1.0, |i| i as f64);
        rt.push_num(i);
        Ok(())
    }),
    ("find", |rt| {
        let q = rt.pop_text("find")?;
        let xs = line(rt);
        for x in xs {
            if test(rt, x.clone(), &q)? {
                rt.push(x);
                rt.ow = true;
                return Ok(());
            }
        }
        rt.push_text("404");
        rt.ow = false;
        Ok(())
    }),
    ("any", |rt| {
        let q = rt.pop_text("any")?;
        let xs = rt.stack.clone();
        let mut ow = false;
        for x in xs {
            if test(rt, x, &q)? {
                ow = true;
                break;
            }
        }
        rt.ow = ow;
        Ok(())
    }),
    ("all", |rt| {
        let q = rt.pop_text("all")?;
        let xs = rt.stack.clone();
        let mut ow = true;
        for x in xs {
            if !test(rt, x, &q)? {
                ow = false;
                break;
            }
        }
        rt.ow = ow;
        Ok(())
    }),
];

// ------------------------------------------------------------ std/text.s

fn join(xs: Vec<V>, sep: &str) -> String {
    xs.iter().map(|v| v.text().to_string()).collect::<Vec<_>>().join(sep)
}

fn fill(fmt: &str, xs: &[V]) -> String {
    let mut out = String::new();
    let mut it = xs.iter();
    for c in fmt.chars() {
        if c == '%' {
            match it.next() {
                Some(v) => out.push_str(&v.text()),
                None => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn pad(rt: &mut Rt, left: bool) -> Res {
    need(rt, 2, "pad")?;
    let w = rt.pop_num("pad")?.max(0.0) as usize;
    let t = rt.pop_text("pad")?;
    let n = t.chars().count();
    let sp = " ".repeat(w.saturating_sub(n));
    rt.push_text(if left { format!("{sp}{t}") } else { format!("{t}{sp}") });
    Ok(())
}

fn parts(rt: &mut Rt, who: &str, f: impl Fn(&str) -> Vec<String>) -> Res {
    let t = rt.pop_text(who)?;
    for p in f(&t) {
        rt.push_text(p);
    }
    Ok(())
}

fn record_key(rt: &mut Rt, who: &str) -> Result<(String, String), Stop> {
    need(rt, 2, who)?;
    let k = rt.pop_text(who)?;
    let n = rt.pop_text(who)?;
    Ok((n.to_string(), k.to_string()))
}

const TEXT: Natives = &[
    ("cat", |rt| {
        need(rt, 2, "cat")?;
        let b = rt.pop_text("cat")?;
        let a = rt.pop_text("cat")?;
        rt.push_text(format!("{a}{b}"));
        Ok(())
    }),
    ("concat", |rt| {
        let xs = line(rt);
        rt.push_text(join(xs, ""));
        Ok(())
    }),
    ("glue", |rt| {
        let xs = line(rt);
        rt.push_text(join(xs, " "));
        Ok(())
    }),
    ("joinwith", |rt| {
        let sep = rt.pop_text("joinwith")?;
        let xs = line(rt);
        rt.push_text(join(xs, &sep));
        Ok(())
    }),
    ("unlines", |rt| {
        let xs = line(rt);
        rt.push_text(join(xs, "\n"));
        Ok(())
    }),
    ("quote", |rt| text1(rt, "quote", |t| format!("{{{t}}}"))),
    ("words", |rt| {
        parts(rt, "words", |t| t.split_whitespace().map(str::to_string).collect())
    }),
    ("chars", |rt| {
        parts(rt, "chars", |t| t.chars().map(|c| c.to_string()).collect())
    }),
    ("lines", |rt| {
        parts(rt, "lines", |t| {
            let t = t.strip_suffix('\n').unwrap_or(t);
            if t.is_empty() {
                vec![]
            } else {
                t.split('\n').map(str::to_string).collect()
            }
        })
    }),
    ("split", |rt| {
        need(rt, 2, "split")?;
        let sep = rt.pop_text("split")?;
        let t = rt.pop_text("split")?;
        let ps: Vec<String> = if sep.is_empty() {
            t.chars().map(|c| c.to_string()).collect()
        } else {
            t.split(&*sep).map(str::to_string).collect()
        };
        for p in ps {
            rt.push_text(p);
        }
        Ok(())
    }),
    ("len", |rt| {
        let t = rt.pop_text("len")?;
        rt.push_num(t.chars().count() as f64);
        Ok(())
    }),
    ("reversed", |rt| text1(rt, "reversed", |t| t.chars().rev().collect())),
    ("repeat", |rt| {
        need(rt, 2, "repeat")?;
        let n = rt.pop_num("repeat")?.max(0.0) as usize;
        let t = rt.pop_text("repeat")?;
        if t.len().saturating_mul(n) > rt.max_out {
            return err("repeat: too long");
        }
        rt.push_text(t.repeat(n));
        Ok(())
    }),
    ("squish", |rt| {
        text1(rt, "squish", |t| t.split_whitespace().collect::<Vec<_>>().join(" "))
    }),
    ("trim", |rt| text1(rt, "trim", |t| t.trim().to_string())),
    ("ltrim", |rt| text1(rt, "ltrim", |t| t.trim_start().to_string())),
    ("rtrim", |rt| text1(rt, "rtrim", |t| t.trim_end().to_string())),
    ("has", |rt| has(rt, |t, p| t.contains(p))),
    ("starts", |rt| has(rt, |t, p| t.starts_with(p))),
    ("ends", |rt| has(rt, |t, p| t.ends_with(p))),
    ("subst", |rt| subst(rt, false)),
    ("subst1", |rt| subst(rt, true)),
    ("occurs", |rt| {
        need(rt, 2, "occurs")?;
        let p = rt.pop_text("occurs")?;
        let t = rt.pop_text("occurs")?;
        let n = if p.is_empty() { 0 } else { t.matches(&*p).count() };
        rt.push_num(n as f64);
        Ok(())
    }),
    ("pos", |rt| {
        need(rt, 2, "pos")?;
        let p = rt.pop_text("pos")?;
        let t = rt.pop_text("pos")?;
        let i = t.find(&*p).map_or(-1.0, |b| t[..b].chars().count() as f64);
        rt.push_num(i);
        Ok(())
    }),
    ("slice", |rt| {
        need(rt, 3, "slice")?;
        let n = rt.pop_num("slice")?.max(0.0) as usize;
        let from = rt.pop_num("slice")? as i64;
        let t = rt.pop_text("slice")?;
        let cs: Vec<char> = t.chars().collect();
        let from = if from < 0 {
            (cs.len() as i64 + from).max(0) as usize
        } else {
            (from as usize).min(cs.len())
        };
        rt.push_text(cs[from..(from + n).min(cs.len())].iter().collect::<String>());
        Ok(())
    }),
    ("padl", |rt| pad(rt, true)),
    ("padr", |rt| pad(rt, false)),
    ("upper", |rt| text1(rt, "upper", |t| t.to_ascii_uppercase())),
    ("lower", |rt| text1(rt, "lower", |t| t.to_ascii_lowercase())),
    ("cap", |rt| {
        text1(rt, "cap", |t| {
            let mut cs = t.chars();
            match cs.next() {
                Some(c) => c.to_ascii_uppercase().to_string() + cs.as_str(),
                None => String::new(),
            }
        })
    }),
    ("title", |rt| {
        text1(rt, "title", |t| {
            let mut out = String::new();
            let mut start = true;
            for c in t.chars() {
                out.push(if start { c.to_ascii_uppercase() } else { c });
                start = c.is_whitespace();
            }
            out
        })
    }),
    ("codeof", |rt| {
        let t = rt.pop_text("codeof")?;
        rt.push_num(t.chars().next().map_or(0.0, |c| c as u32 as f64));
        Ok(())
    }),
    ("charof", |rt| {
        let n = rt.pop_num("charof")?;
        rt.push_text(char::from_u32(n.max(0.0) as u32).map(String::from).unwrap_or_default());
        Ok(())
    }),
    ("fixed", |rt| {
        need(rt, 2, "fixed")?;
        let n = rt.pop_num("fixed")?.clamp(0.0, 20.0) as usize;
        let x = rt.pop_num("fixed")?;
        rt.push_text(format!("{x:.n$}"));
        Ok(())
    }),
    ("hex", |rt| {
        let x = rt.pop_num("hex")? as i64;
        rt.push_text(if x < 0 { format!("-{:x}", -x) } else { format!("{x:x}") });
        Ok(())
    }),
    ("bin", |rt| {
        let x = rt.pop_num("bin")? as i64;
        rt.push_text(if x < 0 { format!("-{:b}", -x) } else { format!("{x:b}") });
        Ok(())
    }),
    ("unhex", |rt| radix(rt, 16, "0x")),
    ("unbin", |rt| radix(rt, 2, "0b")),
    ("format", |rt| {
        let f = rt.pop_text("format")?;
        let xs = line(rt);
        rt.push_text(fill(&f, &xs));
        Ok(())
    }),
    ("printf", |rt| {
        let f = rt.pop_text("printf")?;
        let xs = line(rt);
        let s = fill(&f, &xs);
        rt.out.push_str(&s);
        Ok(())
    }),
    ("say", |rt| {
        let xs = line(rt);
        let s = join(xs, " ");
        rt.out.push_str(&s);
        rt.out.push('\n');
        Ok(())
    }),
    ("get", |rt| {
        let (n, k) = record_key(rt, "get")?;
        let v = rt.lookup(&format!("{n}/{k}"));
        rt.push(v);
        Ok(())
    }),
    ("set", |rt| {
        need(rt, 3, "set")?;
        let v = rt.pop("set")?;
        let (n, k) = record_key(rt, "set")?;
        let keys = format!("{n}/keys");
        if !rt.list(&keys).is_some_and(|l| l.iter().any(|x| *x.text() == *k)) {
            rt.list_mut(&keys).push(V::str(&k));
        }
        rt.mset(&format!("{n}/{k}"), v);
        Ok(())
    }),
    ("keys", |rt| {
        let n = rt.pop_text("keys")?;
        let ks = rt.list(&format!("{n}/keys")).cloned().unwrap_or_default();
        rt.stack.extend(ks);
        Ok(())
    }),
    ("del", |rt| {
        let (n, k) = record_key(rt, "del")?;
        let var = format!("{n}/{k}");
        rt.scope(&var).mem.remove(var.as_str());
        rt.scope(&var).numbers.remove(var.as_str());
        rt.list_mut(&format!("{n}/keys")).retain(|x| *x.text() != *k);
        Ok(())
    }),
    ("haskey", |rt| {
        let (n, k) = record_key(rt, "haskey")?;
        let var = format!("{n}/{k}");
        let s = rt.scope_ref(&var);
        rt.ow = s.mem.contains_key(var.as_str()) || s.numbers.contains_key(var.as_str());
        Ok(())
    }),
];

fn has(rt: &mut Rt, f: impl Fn(&str, &str) -> bool) -> Res {
    need(rt, 2, "has")?;
    let p = rt.pop_text("has")?;
    let t = rt.pop_text("has")?;
    rt.ow = f(&t, &p);
    Ok(())
}

fn subst(rt: &mut Rt, once: bool) -> Res {
    need(rt, 3, "subst")?;
    let new = rt.pop_text("subst")?;
    let old = rt.pop_text("subst")?;
    let t = rt.pop_text("subst")?;
    let r = if old.is_empty() {
        t.to_string()
    } else if once {
        t.replacen(&*old, &new, 1)
    } else {
        t.replace(&*old, &new)
    };
    rt.push_text(r);
    Ok(())
}

fn radix(rt: &mut Rt, base: u32, prefix: &str) -> Res {
    let t = rt.pop_text("unhex")?;
    let t = t.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t),
    };
    let t = t.strip_prefix(prefix).unwrap_or(t);
    let v = i64::from_str_radix(t, base).unwrap_or(0) as f64;
    rt.push_num(if neg { -v } else { v });
    Ok(())
}

// ------------------------------------------------------------ std/gem.s

fn letters(t: &str) -> impl Iterator<Item = u32> + '_ {
    t.chars().map(|c| {
        if c.is_ascii_alphabetic() {
            c.to_ascii_lowercase() as u32 - 'a' as u32 + 1
        } else {
            0
        }
    })
}

fn ord(t: &str) -> f64 {
    letters(t).sum::<u32>() as f64
}
fn red(t: &str) -> f64 {
    letters(t)
        .map(|l| if l == 0 { 0 } else { (l - 1) % 9 + 1 })
        .sum::<u32>() as f64
}
fn rord(t: &str) -> f64 {
    letters(t).map(|l| if l == 0 { 0 } else { 27 - l }).sum::<u32>() as f64
}

const GEM: Natives = &[
    ("ord", |rt| {
        let t = rt.pop_text("ord")?;
        rt.push_num(ord(&t));
        Ok(())
    }),
    ("red", |rt| {
        let t = rt.pop_text("red")?;
        rt.push_num(red(&t));
        Ok(())
    }),
    ("rord", |rt| {
        let t = rt.pop_text("rord")?;
        rt.push_num(rord(&t));
        Ok(())
    }),
    ("gem", |rt| {
        let t = rt.pop_text("gem")?;
        let (o, r, ro) = (ord(&t), red(&t), rord(&t));
        rt.push_text(t.to_string());
        rt.push_num(o);
        rt.push_num(r);
        rt.push_num(ro);
        Ok(())
    }),
    ("ords", |rt| {
        let t = rt.pop_text("ords")?;
        for w in t.split_whitespace() {
            rt.push_num(ord(w));
        }
        Ok(())
    }),
    ("letters", |rt| {
        let t = rt.pop_text("letters")?;
        for l in letters(&t).filter(|&l| l > 0).collect::<Vec<_>>() {
            rt.push_num(l as f64);
        }
        Ok(())
    }),
    ("droot", |rt| {
        num1(rt, "droot", |n| {
            let n = n.abs().trunc();
            if n == 0.0 { 0.0 } else { 1.0 + (n - 1.0).rem_euclid(9.0) }
        })
    }),
];

// ------------------------------------------------------------ std/sh.s

fn lines_of(t: &str) -> Vec<String> {
    let t = t.strip_suffix('\n').unwrap_or(t);
    if t.is_empty() {
        vec![]
    } else {
        t.split('\n').map(str::to_string).collect()
    }
}

fn keep_lines(rt: &mut Rt, who: &str, f: impl Fn(&str, &str) -> bool) -> Res {
    let p = rt.pop_text(who)?;
    let xs = line(rt);
    rt.stack = xs.into_iter().filter(|x| f(&x.text(), &p)).collect();
    Ok(())
}

fn write_lines(rt: &mut Rt, who: &str, append: bool) -> Res {
    let path = rt.pop_text(who)?;
    let xs = line(rt);
    let text: String = xs.iter().map(|x| format!("{}\n", x.text())).collect();
    let f = rt.files.entry(path.to_string()).or_default();
    if !append {
        f.clear();
    }
    f.push_str(&text);
    Ok(())
}

const SH: Natives = &[
    ("cat", |rt| {
        let p = rt.pop_text("cat")?;
        let c = rt
            .files
            .get(&*p)
            .cloned()
            .ok_or_else(|| Stop::Err(format!("cat: there's no file {p}")))?;
        for l in lines_of(&c) {
            rt.push_text(l);
        }
        Ok(())
    }),
    ("sh", |rt| {
        let c = rt.pop_text("sh")?;
        let (out, code) = rt.shell(&c);
        rt.global.mem.insert("r".into(), V::from(out.clone()));
        rt.global.numbers.insert("rc".into(), code as f64);
        for l in lines_of(&out) {
            rt.push_text(l);
        }
        Ok(())
    }),
    ("grep", |rt| keep_lines(rt, "grep", |l, p| l.contains(p))),
    ("grepv", |rt| keep_lines(rt, "grepv", |l, p| !l.contains(p))),
    ("sed", |rt| {
        need(rt, 2, "sed")?;
        let new = rt.pop_text("sed")?;
        let old = rt.pop_text("sed")?;
        let xs = line(rt);
        rt.stack = xs
            .into_iter()
            .map(|x| {
                V::from(if old.is_empty() {
                    x.text().to_string()
                } else {
                    x.text().replace(&*old, &new)
                })
            })
            .collect();
        Ok(())
    }),
    ("head", |rt| {
        let n = rt.pop_num("head")?.max(0.0) as usize;
        rt.stack.truncate(n);
        Ok(())
    }),
    ("tail", |rt| {
        let n = rt.pop_num("tail")?.max(0.0) as usize;
        let k = rt.stack.len().saturating_sub(n);
        rt.stack.drain(..k);
        Ok(())
    }),
    ("wc", |rt| {
        let n = rt.stack.len();
        rt.stack = vec![V::N(n as f64)];
        Ok(())
    }),
    ("uniq", |rt| {
        let xs = line(rt);
        let mut out: Vec<V> = Vec::new();
        for x in xs {
            if out.last().is_none_or(|l| !l.same(&x)) {
                out.push(x);
            }
        }
        rt.stack = out;
        Ok(())
    }),
    ("nl", |rt| {
        let xs = line(rt);
        rt.stack = xs
            .into_iter()
            .enumerate()
            .map(|(i, x)| V::from(format!("{}  {}", i + 1, x.text())))
            .collect();
        Ok(())
    }),
    ("shout", |rt| {
        let xs = line(rt);
        rt.stack = xs.into_iter().map(|x| V::from(x.text().to_ascii_uppercase())).collect();
        Ok(())
    }),
    ("put", |rt| {
        let xs = line(rt);
        for x in xs {
            rt.out.push_str(&x.text());
            rt.out.push('\n');
        }
        Ok(())
    }),
    (">file", |rt| write_lines(rt, ">file", false)),
    (">>file", |rt| write_lines(rt, ">>file", true)),
    ("ls", |rt| {
        let d = rt.pop_text("ls")?;
        let prefix = if d.is_empty() || &*d == "." {
            String::new()
        } else {
            format!("{}/", d.trim_end_matches('/'))
        };
        let names: Vec<String> = rt
            .files
            .keys()
            .filter_map(|k| {
                k.strip_prefix(&prefix)
                    .map(|r| r.split('/').next().unwrap_or(r).to_string())
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for n in names {
            rt.push_text(n);
        }
        Ok(())
    }),
    ("exists", |rt| {
        let p = rt.pop_text("exists")?;
        let dir = format!("{}/", p.trim_end_matches('/'));
        rt.ow = rt.files.contains_key(&*p) || rt.files.keys().any(|k| k.starts_with(&dir));
        Ok(())
    }),
    ("mkdir", |rt| {
        // folders are implied by the paths of the run's files
        rt.pop_text("mkdir").map(|_| ())
    }),
    ("env", |rt| {
        rt.pop_text("env")?;
        rt.push_text("404");
        Ok(())
    }),
    ("now", |rt| {
        // chrono reads the browser's clock on the web, where the system time panics
        rt.push_num(chrono::Utc::now().timestamp_millis() as f64);
        Ok(())
    }),
    ("sleep", |rt| rt.pop_num("sleep").map(|_| ())),
];

// ------------------------------------------------------------ std/logic.s

fn bit(x: f64) -> f64 {
    if x != 0.0 { 1.0 } else { 0.0 }
}

fn gate2(rt: &mut Rt, who: &str, f: fn(bool, bool) -> bool) -> Res {
    num2(rt, who, |a, b| if f(a != 0.0, b != 0.0) { 1.0 } else { 0.0 })
}

const LOGIC: Natives = &[
    ("bit", |rt| num1(rt, "bit", bit)),
    ("buf", |rt| num1(rt, "buf", |a| a)),
    ("not", |rt| num1(rt, "not", |a| 1.0 - bit(a))),
    ("and", |rt| gate2(rt, "and", |a, b| a && b)),
    ("or", |rt| gate2(rt, "or", |a, b| a || b)),
    ("xor", |rt| gate2(rt, "xor", |a, b| a != b)),
    ("nand", |rt| gate2(rt, "nand", |a, b| !(a && b))),
    ("nor", |rt| gate2(rt, "nor", |a, b| !(a || b))),
    ("xnor", |rt| gate2(rt, "xnor", |a, b| a == b)),
    ("hadd", |rt| {
        need(rt, 2, "hadd")?;
        let b = bit(rt.pop_num("hadd")?);
        let a = bit(rt.pop_num("hadd")?);
        rt.push_num(bit((a as u8 ^ b as u8) as f64));
        rt.push_num(a * b);
        Ok(())
    }),
    ("fadd", |rt| {
        need(rt, 3, "fadd")?;
        let c = bit(rt.pop_num("fadd")?) as u8;
        let b = bit(rt.pop_num("fadd")?) as u8;
        let a = bit(rt.pop_num("fadd")?) as u8;
        let s = a + b + c;
        rt.push_num((s & 1) as f64);
        rt.push_num((s >> 1) as f64);
        Ok(())
    }),
    ("mux", |rt| {
        need(rt, 3, "mux")?;
        let s = rt.pop_num("mux")?;
        let b = rt.pop("mux")?;
        let a = rt.pop("mux")?;
        rt.push(if s == 0.0 { a } else { b });
        Ok(())
    }),
    ("majority", |rt| {
        need(rt, 3, "majority")?;
        let c = bit(rt.pop_num("majority")?);
        let b = bit(rt.pop_num("majority")?);
        let a = bit(rt.pop_num("majority")?);
        rt.push_num(if a + b + c >= 2.0 { 1.0 } else { 0.0 });
        Ok(())
    }),
    ("bits", |rt| {
        need(rt, 2, "bits")?;
        let w = rt.pop_num("bits")?.clamp(0.0, 64.0) as u32;
        let n = rt.pop_num("bits")? as i64 as u64;
        for i in (0..w).rev() {
            rt.push_num(((n >> i) & 1) as f64);
        }
        Ok(())
    }),
    ("unbits", |rt| {
        let xs = line(rt);
        let n = xs.iter().fold(0u64, |acc, b| (acc << 1) | (b.num() != 0.0) as u64);
        rt.push_num(n as f64);
        Ok(())
    }),
    ("popcount", |rt| {
        num1(rt, "popcount", |a| (a as i64 as u64).count_ones() as f64)
    }),
];

// ------------------------------------------------------------ std/elec.s

const E12: [f64; 12] = [1.0, 1.2, 1.5, 1.8, 2.2, 2.7, 3.3, 3.9, 4.7, 5.6, 6.8, 8.2];

fn band(c: &str) -> Option<f64> {
    Some(match c.to_ascii_lowercase().as_str() {
        "black" => 0.0,
        "brown" => 1.0,
        "red" => 2.0,
        "orange" => 3.0,
        "yellow" => 4.0,
        "green" => 5.0,
        "blue" => 6.0,
        "violet" | "purple" => 7.0,
        "grey" | "gray" => 8.0,
        "white" => 9.0,
        _ => return None,
    })
}

const ELEC: Natives = &[
    ("ohm", |rt| num2(rt, "ohm", |i, r| i * r)),
    ("amps", |rt| num2(rt, "amps", |v, r| v / r)),
    ("watts", |rt| num2(rt, "watts", |v, i| v * i)),
    ("series", |rt| {
        let s: f64 = line(rt).iter().map(V::num).sum();
        rt.push_num(s);
        Ok(())
    }),
    ("parallel", |rt| {
        let s: f64 = line(rt).iter().map(|r| 1.0 / r.num()).sum();
        rt.push_num(1.0 / s);
        Ok(())
    }),
    ("divider", |rt| {
        need(rt, 3, "divider")?;
        let r2 = rt.pop_num("divider")?;
        let r1 = rt.pop_num("divider")?;
        let v = rt.pop_num("divider")?;
        rt.push_num(v * r2 / (r1 + r2));
        Ok(())
    }),
    ("timeconst", |rt| num2(rt, "timeconst", |r, c| r * c)),
    ("charge", |rt| rc(rt, true)),
    ("discharge", |rt| rc(rt, false)),
    ("ledr", |rt| {
        need(rt, 3, "ledr")?;
        let i = rt.pop_num("ledr")?;
        let vf = rt.pop_num("ledr")?;
        let vs = rt.pop_num("ledr")?;
        rt.push_num((vs - vf) / i);
        Ok(())
    }),
    ("xc", |rt| num2(rt, "xc", |f, c| 1.0 / (std::f64::consts::TAU * f * c))),
    ("xl", |rt| num2(rt, "xl", |f, l| std::f64::consts::TAU * f * l)),
    ("resonance", |rt| {
        num2(rt, "resonance", |l, c| 1.0 / (std::f64::consts::TAU * (l * c).sqrt()))
    }),
    ("db", |rt| num1(rt, "db", |r| 20.0 * r.log10())),
    ("dbp", |rt| num1(rt, "dbp", |r| 10.0 * r.log10())),
    ("bands", |rt| {
        let xs = line(rt);
        let Some((mult, digits)) = xs.split_last() else {
            return err("bands needs colours");
        };
        let mut v = 0.0;
        for d in digits {
            v = v * 10.0 + band(&d.text()).ok_or_else(|| Stop::Err(format!("{} isn't a band colour", d.text())))?;
        }
        let m = match mult.text().to_ascii_lowercase().as_str() {
            "gold" => 0.1,
            "silver" => 0.01,
            c => 10f64.powf(band(c).ok_or_else(|| Stop::Err(format!("{c} isn't a band colour")))?),
        };
        rt.push_num(v * m);
        Ok(())
    }),
    ("e12", |rt| {
        num1(rt, "e12", |r| {
            if r <= 0.0 {
                return 0.0;
            }
            let dec = 10f64.powf(r.log10().floor());
            let cands = E12.iter().map(|m| m * dec).chain([10.0 * dec]);
            cands
                .min_by(|a, b| (a.ln() - r.ln()).abs().total_cmp(&(b.ln() - r.ln()).abs()))
                .unwrap_or(r)
        })
    }),
];

fn rc(rt: &mut Rt, up: bool) -> Res {
    need(rt, 4, "charge")?;
    let t = rt.pop_num("charge")?;
    let c = rt.pop_num("charge")?;
    let r = rt.pop_num("charge")?;
    let v = rt.pop_num("charge")?;
    let e = (-t / (r * c)).exp();
    rt.push_num(if up { v * (1.0 - e) } else { v * e });
    Ok(())
}

// ------------------------------------------------------------ std/anend.s
//
// anend's own module (not in s 1.0): the project's functions, headless. `op` calls any
// of the app API's ops (docs/app-api.md) by name with JSON arguments, on the save the run
// belongs to, with the hub's limits on a shared hub; `json`, `#json` and `quote` read its
// answers and build its arguments. A run without a host (s on its own) fails on `op`.

/// a JSON value at a dotted path (`0.title`, `members.2`): the value, if it's there
fn json_at<'v>(v: &'v serde_json::Value, path: &str) -> Option<&'v serde_json::Value> {
    path.split('.').filter(|p| !p.is_empty()).try_fold(v, |v, p| match v {
        serde_json::Value::Array(a) => p.parse::<usize>().ok().and_then(|i| a.get(i)),
        serde_json::Value::Object(o) => o.get(p),
        _ => None,
    })
}

/// a JSON value as s text: a string as itself, anything else as its JSON
fn json_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

const ANEND: Natives = &[
    // name [args (JSON) = {}] [dest = r]: mem[dest] = the answer (JSON); ow: it answered
    // (else mem[err] says why)
    ("op", |rt| {
        let vals = line(rt);
        let Some(name) = vals.first().map(|v| v.text().to_string()) else {
            return err("op needs an op's name");
        };
        let args = vals.get(1).map(|v| v.text().to_string()).unwrap_or_else(|| "{}".into());
        let dest = vals.get(2).map(|v| v.text().to_string()).unwrap_or_else(|| "r".into());
        let Some(host) = rt.host else {
            return err("op: this run has no anend to ask (it runs in the notes view, the bar or `anend run`)");
        };
        let args: serde_json::Value = match serde_json::from_str(if args.trim().is_empty() { "{}" } else { &args }) {
            Ok(a) => a,
            Err(e) => return err(format!("op {name}: its arguments aren't JSON: {e}")),
        };
        match host(&name, &args) {
            Ok(v) => {
                rt.mset(&dest, V::from(v.to_string()));
                rt.ow = true;
            }
            Err(e) => {
                rt.global.mem.insert("err".into(), V::from(e));
                rt.ow = false;
            }
        }
        Ok(())
    }),
    // json path [dest = j]: mem[dest] = the value there (a string as itself); ow: found
    ("json", |rt| {
        let vals = line(rt);
        need_vals(&vals, 2, "json")?;
        let dest = vals.get(2).map(|v| v.text().to_string()).unwrap_or_else(|| "j".into());
        let found = serde_json::from_str::<serde_json::Value>(&vals[0].text())
            .ok()
            .and_then(|v| json_at(&v, &vals[1].text()).map(json_text));
        rt.ow = found.is_some();
        rt.mset(&dest, V::from(found.unwrap_or_else(|| "404".into())));
        Ok(())
    }),
    // json [path] [dest = =]: numbers[dest] = how many items (or keys, or characters) there
    ("#json", |rt| {
        let vals = line(rt);
        need_vals(&vals, 1, "#json")?;
        let path = vals.get(1).map(|v| v.text().to_string()).unwrap_or_default();
        let dest = vals.get(2).map(|v| v.text().to_string()).unwrap_or_else(|| "=".into());
        let n = serde_json::from_str::<serde_json::Value>(&vals[0].text())
            .ok()
            .and_then(|v| {
                json_at(&v, &path).map(|x| match x {
                    serde_json::Value::Array(a) => a.len(),
                    serde_json::Value::Object(o) => o.len(),
                    serde_json::Value::String(s) => s.chars().count(),
                    _ => 0,
                })
            })
            .unwrap_or(0);
        rt.nset(&dest, n as f64);
        Ok(())
    }),
    // key value key value ...: mem[o] = a JSON object of them (a value that reads as JSON,
    // a number, an object, a list, true, false or null, goes in as it is; the rest as text)
    ("obj", |rt| {
        let vals = line(rt);
        if !vals.len().is_multiple_of(2) {
            return err("obj needs keys and values in pairs");
        }
        let mut o = serde_json::Map::new();
        for kv in vals.chunks(2) {
            let v = kv[1].text().to_string();
            let j = serde_json::from_str::<serde_json::Value>(&v)
                .ok()
                .filter(|j| !j.is_string())
                .unwrap_or(serde_json::Value::String(v));
            o.insert(kv[0].text().to_string(), j);
        }
        rt.mset("o", V::from(serde_json::Value::Object(o).to_string()));
        Ok(())
    }),
    // text [dest = q]: mem[dest] = the text as a JSON string (for building arguments)
    ("quote", |rt| {
        let vals = line(rt);
        need_vals(&vals, 1, "quote")?;
        let dest = vals.get(1).map(|v| v.text().to_string()).unwrap_or_else(|| "q".into());
        rt.mset(
            &dest,
            V::from(serde_json::Value::String(vals[0].text().to_string()).to_string()),
        );
        Ok(())
    }),
];

fn need_vals(vals: &[V], n: usize, who: &str) -> Res {
    if vals.len() < n {
        return err(format!("{who} needs {n} value{}", if n == 1 { "" } else { "s" }));
    }
    Ok(())
}

/// every library word, by module (for completion and highlighting)
pub fn words() -> Vec<(&'static str, Vec<&'static str>)> {
    ["core", "line", "text", "gem", "sh", "logic", "elec", "anend"]
        .iter()
        .filter_map(|m| module(m).map(|(_, w)| (*m, w.iter().map(|(n, _)| *n).collect())))
        .collect()
}
