//! Running s programs (`s.*`, docs/s.md): a program is a note tagged `s` (its body is the
//! code; the notes view edits it right-aligned), or code given directly. A run's files are
//! the save's s programs, each under its title (and its title with `.s`), so `name.` runs
//! another program; what a run writes to files stays in that run. On a shared hub a run
//! takes at most 2 million steps, on one's own machine 50 million.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::classify::Limits;
use super::{Res, note_json, s};
use crate::graph::query::{Order, Page, Query};
use crate::graph::record::{self, mime};
use crate::graph::{Graph, Reader};

pub fn routed(op: &str) -> bool {
    op.starts_with("s.")
}

/// the tag that makes a note an s program
pub const TAG: &str = "s";

/// the save's s programs: title → code
pub fn programs(g: &Graph) -> Result<BTreeMap<String, (String, String)>, String> {
    let v = g.view()?;
    let mut out = BTreeMap::new();
    let q = Query {
        mime: Some(mime::MARKDOWN),
        order: Order::CreatedDesc,
        ..Query::default()
    };
    for id in v.query(&q, Page::all())?.ids {
        let n = note_json(&v, id)?;
        if n["tags"].as_array().is_some_and(|t| t.iter().any(|x| x == TAG)) {
            let title = n["title"].as_str().unwrap_or("").trim().to_string();
            if !title.is_empty() {
                out.insert(
                    title,
                    (record::label(id), n["markdown"].as_str().unwrap_or("").to_string()),
                );
            }
        }
    }
    Ok(out)
}

pub fn call(g: &Graph, op: &str, args: &Value, limits: &Limits) -> Res {
    match op {
        "s.programs" => {
            let p = programs(g)?;
            Ok(json!(
                p.iter()
                    .map(|(t, (k, _))| json!({ "title": t, "key": k }))
                    .collect::<Vec<_>>()
            ))
        }
        "s.run" => {
            let progs = programs(g)?;
            let code = match args.get("code").and_then(Value::as_str) {
                Some(c) => c.to_string(),
                None => {
                    let key = s(args, "key");
                    let id = record::parse_label(key.strip_prefix("note:").unwrap_or(key))
                        .ok_or("which program? (code or key)")?;
                    let text = g.view()?.render(id)?;
                    crate::note::split(&text).1.to_string()
                }
            };
            let mut files = BTreeMap::new();
            for (title, (_, body)) in progs {
                files.insert(format!("{title}.s"), body.clone());
                files.insert(title, body);
            }
            let argv: Vec<String> = args
                .get("args")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let max = if limits.shared { 2_000_000 } else { 50_000_000 };
            let o = crate::s::run(&code, s(args, "stdin"), files, &argv, max);
            Ok(json!({ "out": o.out, "err": o.err, "code": o.code, "steps": o.steps }))
        }
        "s.words" => {
            // what the editor colours: the commands, the suffixes, the library's words
            Ok(json!({
                "commands": crate::s::machine::COMMANDS,
                "suffixes": crate::s::machine::SUFFIXES.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
                "library": crate::s::lib::words().into_iter().map(|(m, w)| json!({ "module": m, "words": w })).collect::<Vec<_>>(),
            }))
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_notes_tagged_s() {
        let g = Graph::in_memory().unwrap();
        let rm = Default::default();
        let c = |op: &str, a: Value| super::super::call(&g, &rm, op, &a).unwrap();
        c(
            "notes.save",
            json!({ "note": { "tags": ["s"] }, "text": "# greet\n\n{hello ~who~} >" }),
        );
        c(
            "notes.save",
            json!({ "note": { "tags": [] }, "text": "# not code\n\nplain" }),
        );
        let main = c(
            "notes.save",
            json!({ "note": { "tags": ["s"] }, "text": "# main\n\nworld who@ greet." }),
        );
        let l = Limits::NONE;
        let p = call(&g, "s.programs", &json!({}), &l).unwrap();
        assert_eq!(p.as_array().unwrap().len(), 2);
        let r = call(&g, "s.run", &json!({ "key": main["_key"] }), &l).unwrap();
        assert_eq!(r["out"], "\n hello world");
        assert_eq!(r["err"], Value::Null);
        let r = call(
            &g,
            "s.run",
            &json!({ "code": "_ <_ _~ > args 0 a [l] a~ >", "stdin": "in", "args": ["x"] }),
            &l,
        )
        .unwrap();
        assert_eq!(r["out"], "\n in\n x");
        let r = call(
            &g,
            "s.run",
            &json!({ "code": "{l.} l$ l.." }),
            &Limits {
                shared: true,
                ..Limits::NONE
            },
        )
        .unwrap();
        assert!(r["err"].as_str().unwrap().contains("2000000 steps"));
        assert!(
            call(&g, "s.words", &json!({}), &l).unwrap()["commands"]
                .as_array()
                .unwrap()
                .len()
                > 50
        );
    }
}
