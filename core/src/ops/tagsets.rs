//! Tag sets (queued requests #13 and #10): word types of the user's own. A tag set names
//! its types (each with a colour and a line about it) and learns them from words marked by
//! hand: each type keeps its words as a list (`<set>: <type>`), and the set's classifier
//! (`tags: <set>`) learns from those lists like any other, so it reads words it was never
//! shown. A set can say which texts it's for (a classifier's class, such as a language
//! starter's "not English"), so word types for other languages come from their own set.
//!
//! A **checker** set (`check: true`) has the two types "right" and "wrong": marking words
//! of a constructed language right or wrong trains it to flag the wrong ones, and each
//! flagged word gets suggestions, the nearest words marked right.
//!
//! The built-in English types (`kinds::tag`, noun, verb, …) are the set "english": rules,
//! not trained, used for texts no set of one's own claims, and only when the text reads as
//! English (`kinds::looks_english`), so other languages aren't given English types.
//!
//! A tag set is an entry of the save (mime `application/x-anend-tagset`, its text the
//! [`TagSet`]'s JSON), out of search and counts.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::taxonomy::{entries, put};
use super::{Res, Saves, s};
use crate::graph::Graph;
use crate::graph::record::{self, mime};

/// words a text is tagged on at most
const WORDS: usize = 400;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Type {
    pub name: String,
    /// `#rrggbb`
    pub colour: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
}

/// which texts a set is for: those a classifier puts in a class, or that a taxonomy (a
/// language taxonomy, say) routes to a class
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct For {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub classifier: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub taxonomy: String,
    pub class: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TagSet {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    pub types: Vec<Type>,
    /// a checker: types right and wrong, wrong words flagged with suggestions
    #[serde(default)]
    pub check: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub for_texts: Option<For>,
    /// below this probability a word gets no type
    #[serde(default = "half")]
    pub threshold: f64,
}

fn half() -> f64 {
    0.5
}

pub fn routed(op: &str) -> bool {
    op.starts_with("tagsets.")
}

fn all(g: &Graph) -> Result<Vec<(u64, TagSet)>, String> {
    let mut out: Vec<(u64, TagSet)> = entries(g, mime::TAGSET)?;
    out.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    Ok(out)
}

fn named(g: &Graph, name: &str) -> Result<(u64, TagSet), String> {
    all(g)?
        .into_iter()
        .find(|(_, t)| t.name == name)
        .ok_or_else(|| format!("there's no tag set called {name:?}"))
}

fn classifier_of(set: &str) -> String {
    format!("tags: {set}")
}

fn list_name(set: &str, ty: &str) -> String {
    format!("{set}: {ty}")
}

/// the type a class of the set's classifier stands for
fn type_of<'a>(set: &str, class: &'a str) -> &'a str {
    class.strip_prefix(&format!("{set}: ")).unwrap_or(class)
}

fn checked(ws: &Saves, t: &mut TagSet) -> Result<(), String> {
    t.name = t.name.trim().to_string();
    if t.name.is_empty() || t.name == "english" {
        return Err("a tag set needs a name of its own (english is the built-in one)".into());
    }
    if t.check {
        // a checker's types are fixed: right and wrong
        let colour = |n: &str| {
            t.types
                .iter()
                .find(|x| x.name == n)
                .map(|x| x.colour.clone())
                .unwrap_or_else(|| {
                    if n == "right" {
                        "#3fae6a".into()
                    } else {
                        "#d6453d".into()
                    }
                })
        };
        t.types = vec![
            Type {
                name: "right".into(),
                colour: colour("right"),
                about: "a word of the language".into(),
            },
            Type {
                name: "wrong".into(),
                colour: colour("wrong"),
                about: "not a word of it, or misspelled".into(),
            },
        ];
    }
    if t.types.len() < 2 {
        return Err("a tag set needs two types or more".into());
    }
    let mut seen = std::collections::HashSet::new();
    for ty in &t.types {
        let n = ty.name.trim();
        if n.is_empty() || n.contains(':') {
            return Err(format!("{n:?} can't be a type's name (no colons)"));
        }
        if !seen.insert(n.to_string()) {
            return Err(format!("the type {n:?} is there twice"));
        }
        let c = &ty.colour;
        if !(c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit())) {
            return Err(format!("{c:?} isn't a colour like #a0c4ff"));
        }
    }
    if !(0.0..=1.0).contains(&t.threshold) {
        return Err("the threshold goes from 0 to 1".into());
    }
    if let Some(f) = &t.for_texts {
        let (what, classes): (&str, Vec<String>) = if !f.taxonomy.is_empty() {
            let d = ws.call("taxonomy.get", &json!({ "name": f.taxonomy }))?;
            let cs = d["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|n| n["classes"].as_array().cloned().unwrap_or_default())
                .filter_map(|c| c.as_str().map(String::from))
                .collect();
            (&f.taxonomy, cs)
        } else if !f.classifier.is_empty() {
            let d = ws.call("classify.get", &json!({ "name": f.classifier }))?;
            let cs = d["classes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.as_str().map(String::from))
                .collect();
            (&f.classifier, cs)
        } else {
            return Err("which texts is it for? (a classifier or a taxonomy, and a class)".into());
        };
        if !classes.contains(&f.class) {
            return Err(format!("{what:?} has no class {:?}", f.class));
        }
    }
    Ok(())
}

/// the key of a type's list of examples, made when it's not there
fn list_key(ws: &Saves, set: &str, ty: &str) -> Result<String, String> {
    let r = ws.call("mem.saturate", &json!({ "text": list_name(set, ty) }))?;
    Ok(r["key"].as_str().unwrap_or("").to_string())
}

/// a type and its marked words, `(key, word)`
type Marked = (String, Vec<(String, String)>);

/// each type's marked words
fn examples(ws: &Saves, t: &TagSet) -> Result<Vec<Marked>, String> {
    let stats = ws.call("lists.stats", &json!({}))?;
    Ok(t.types
        .iter()
        .map(|ty| {
            let want = list_name(&t.name, &ty.name);
            let words = stats["lists"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|l| l["name"] == want.as_str())
                .map(|l| {
                    l["members"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|m| {
                            (
                                m["key"].as_str().unwrap_or("").to_string(),
                                m["text"].as_str().unwrap_or("").to_string(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            (ty.name.clone(), words)
        })
        .collect())
}

/// edit distance, for a checker's suggestions
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for j in 0..b.len() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != b[j])).min(row[j] + 1).min(row[j + 1] + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// the set that reads this text: the one named, else the first of one's own whose
/// classifier puts the text in its class, else the built-in English one when the text
/// reads as English, else none
fn choose(ws: &Saves, g: &Graph, name: &str, text: &str) -> Result<Option<TagSet>, String> {
    if name == "english" {
        return Ok(None);
    }
    if !name.is_empty() {
        return Ok(Some(named(g, name)?.1));
    }
    for (_, t) in all(g)? {
        let Some(f) = &t.for_texts else { continue };
        let fits = if !f.taxonomy.is_empty() {
            ws.call("taxonomy.route", &json!({ "name": f.taxonomy, "text": text }))
                .is_ok_and(|r| r["reached"] == true && r["class"] == f.class.as_str())
        } else {
            ws.call("classify.predict", &json!({ "name": f.classifier, "texts": [text] }))
                .is_ok_and(|r| r["trained"] == true && r["results"][0]["class"] == f.class.as_str())
        };
        if fits {
            return Ok(Some(t));
        }
    }
    Ok(None)
}

/// the words of a text, with where each is
fn words(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        let part = c.is_alphanumeric() || c == '\'' || c == '-';
        match (part, start) {
            (true, None) => start = Some(i),
            (false, Some(s0)) => {
                out.push((s0, i, text[s0..i].to_string()));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s0) = start {
        out.push((s0, text.len(), text[s0..].to_string()));
    }
    out.truncate(WORDS);
    out
}

fn tag(ws: &Saves, g: &Graph, name: &str, text: &str) -> Res {
    let Some(t) = choose(ws, g, name, text)? else {
        // the built-in English types, only on English
        let english = crate::kinds::looks_english(text);
        let tagged: Vec<Value> = if english || name == "english" {
            crate::kinds::tag(text)
                .into_iter()
                .map(|w| json!({ "word": w.word, "start": w.start, "end": w.end, "type": w.ty }))
                .collect()
        } else {
            vec![]
        };
        return Ok(json!({ "set": "english", "english": english, "words": tagged }));
    };
    let ws_ = words(text);
    let cls = classifier_of(&t.name);
    let colour = |ty: &str| {
        t.types
            .iter()
            .find(|x| x.name == ty)
            .map(|x| x.colour.clone())
            .unwrap_or_default()
    };
    let r = if ws_.is_empty() {
        Value::Null
    } else {
        let texts: Vec<&str> = ws_.iter().map(|w| w.2.as_str()).collect();
        ws.call("classify.predict", &json!({ "name": cls, "texts": texts }))
            .unwrap_or(Value::Null)
    };
    // not trained yet: the words, without types, to be marked
    if r["trained"] != true {
        let out: Vec<Value> = ws_
            .iter()
            .map(|(a, b, w)| json!({ "word": w, "start": a, "end": b, "type": null, "prob": 0.0 }))
            .collect();
        return Ok(json!({ "set": t.name, "check": t.check, "trained": false, "words": out }));
    }
    let classes: Vec<String> = r["classes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| c.as_str().unwrap_or("").to_string())
        .collect();
    let right: Vec<String> = if t.check {
        examples(ws, &t)?
            .into_iter()
            .find(|(n, _)| n == "right")
            .map(|(_, w)| w.into_iter().map(|x| x.1).collect())
            .unwrap_or_default()
    } else {
        vec![]
    };
    let mut out = Vec::new();
    for (i, (a, b, w)) in ws_.iter().enumerate() {
        let res = &r["results"][i];
        let top = res["top"].as_u64().unwrap_or(0) as usize;
        let p = res["probs"][top].as_f64().unwrap_or(0.0);
        let ty = classes.get(top).map(|c| type_of(&t.name, c).to_string());
        let shown = ty.filter(|_| p >= t.threshold);
        let mut o = json!({ "word": w, "start": a, "end": b, "type": shown, "prob": p });
        if let Some(ty) = o["type"].as_str() {
            o["colour"] = json!(colour(ty));
        }
        // a checker's wrong words: the nearest words marked right
        if t.check && o["type"] == "wrong" {
            let lw = w.to_lowercase();
            let mut near: Vec<(usize, &String)> = right.iter().map(|r| (distance(&lw, &r.to_lowercase()), r)).collect();
            near.sort();
            o["suggest"] = json!(
                near.iter()
                    .take(3)
                    .filter(|(d, _)| *d <= 3)
                    .map(|x| x.1)
                    .collect::<Vec<_>>()
            );
        }
        out.push(o);
    }
    Ok(json!({ "set": t.name, "check": t.check, "trained": true, "words": out }))
}

/// how well it reads: per type, the precision and recall on words held out of training
/// (from its last training), and how many words each type has marked
fn score(ws: &Saves, t: &TagSet) -> Res {
    let ex = examples(ws, t)?;
    let marked: Vec<Value> = ex
        .iter()
        .map(
            |(n, w)| json!({ "type": n, "words": w.len(), "some": w.iter().take(8).map(|x| &x.1).collect::<Vec<_>>() }),
        )
        .collect();
    let d = ws.call("classify.get", &json!({ "name": classifier_of(&t.name) })).ok();
    let mut per = Vec::new();
    let mut accuracy = Value::Null;
    let mut measured = Value::Null;
    if let Some(d) = &d
        && let Some(ev) = d["history"].as_array().and_then(|h| {
            h.iter()
                .rev()
                .find(|e| e["kind"] == "train" && e["metrics"].is_object())
        })
    {
        let classes: Vec<String> = d["classes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| c.as_str().unwrap_or("").to_string())
            .collect();
        accuracy = ev["metrics"]["accuracy"].clone();
        measured = ev["measured_on"].clone();
        for (i, c) in ev["metrics"]["classes"].as_array().into_iter().flatten().enumerate() {
            per.push(json!({
                "type": classes.get(i).map(|x| type_of(&t.name, x)).unwrap_or(""),
                "precision": c["precision"], "recall": c["recall"], "support": c["support"],
            }));
        }
    }
    Ok(json!({
        "marked": marked, "trained": d.as_ref().is_some_and(|d| d["trained"] == true),
        "training": d.as_ref().is_some_and(|d| d["training"] == true),
        "accuracy": accuracy, "measured_on": measured, "types": per,
    }))
}

pub fn call(ws: &Saves, g: &Graph, op: &str, args: &Value) -> Res {
    match op {
        "tagsets.list" => {
            let mut out = vec![json!({
                "name": "english", "builtin": true,
                "about": "English word types by rule, for texts that read as English",
                "types": crate::kinds::WORD_TYPES,
            })];
            for (id, t) in all(g)? {
                out.push(json!({ "key": record::label(id), "builtin": false, "set": t }));
            }
            Ok(json!(out))
        }
        "tagsets.save" => {
            let mut t: TagSet = serde_json::from_value(args.get("set").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("that isn't a tag set: {e}"))?;
            checked(ws, &mut t)?;
            let fresh = args.get("new").and_then(Value::as_bool).unwrap_or(false);
            let was = match s(args, "was") {
                "" => t.name.clone(),
                w => w.to_string(),
            };
            let old = if fresh {
                None
            } else {
                all(g)?.into_iter().find(|(_, o)| o.name == was)
            };
            if (fresh || was != t.name) && all(g)?.iter().any(|(_, o)| o.name == t.name) {
                return Err(format!("there's a tag set called {:?} already", t.name));
            }
            if old.as_ref().is_some_and(|(_, o)| o.name != t.name) {
                return Err("a tag set keeps its name (its words' lists are named after it)".into());
            }
            let id = put(g, old.map(|(id, _)| id), mime::TAGSET, &t)?;
            Ok(json!({ "key": record::label(id), "set": t }))
        }
        "tagsets.delete" => {
            let (id, t) = named(g, s(args, "name"))?;
            g.delete(id)?;
            let _ = ws.call("classify.delete", &json!({ "name": classifier_of(&t.name) }));
            Ok(json!({ "removed": t.name }))
        }
        // a word is of a type: into that type's list (out of the set's other types'),
        // and, when the set's classifier is trained, its feedback too
        "tagsets.mark" => {
            let (_, t) = named(g, s(args, "name"))?;
            let ty = s(args, "type");
            if !t.types.iter().any(|x| x.name == ty) {
                return Err(format!("{:?} has no type {ty:?}", t.name));
            }
            let word = s(args, "word").trim();
            if word.is_empty() || word.split_whitespace().count() > 1 {
                return Err("mark one word at a time".into());
            }
            let member = ws.call("mem.saturate", &json!({ "text": word }))?["key"]
                .as_str()
                .unwrap_or("")
                .to_string();
            for other in &t.types {
                let list = list_key(ws, &t.name, &other.name)?;
                let op = if other.name == ty {
                    "lists.enlist"
                } else {
                    "lists.delist"
                };
                ws.call(op, &json!({ "list": list, "member": member }))?;
            }
            let cls = classifier_of(&t.name);
            let mut taught = false;
            if ws
                .call("classify.get", &json!({ "name": cls }))
                .is_ok_and(|d| d["trained"] == true && d["training"] != true)
            {
                ws.call(
                    "classify.feedback",
                    &json!({ "name": cls, "text": word, "class": list_name(&t.name, ty), "yes": true }),
                )?;
                taught = true;
            }
            Ok(json!({ "word": word, "type": ty, "taught": taught, "score": score(ws, &t)? }))
        }
        // (re)trains the set's classifier on its marked words, made when there's none
        "tagsets.train" => {
            let (_, t) = named(g, s(args, "name"))?;
            let ex = examples(ws, &t)?;
            if let Some((n, _)) = ex.iter().find(|(_, w)| w.len() < 2) {
                return Err(format!("mark two words or more as {n} first"));
            }
            let cls = classifier_of(&t.name);
            if ws.call("classify.get", &json!({ "name": cls })).is_err() {
                let keys: Vec<String> = t
                    .types
                    .iter()
                    .map(|ty| list_key(ws, &t.name, &ty.name))
                    .collect::<Result<_, _>>()?;
                ws.call(
                    "classify.create",
                    &json!({ "name": cls, "classes": keys, "preset": "tiny" }),
                )?;
            }
            // few marked words: all of them to learn from (none held out to stop early by,
            // which would end it at once); with forty or more, a fifth held out to measure
            let total: usize = ex.iter().map(|(_, w)| w.len()).sum();
            let held = if total >= 40 { 0.2 } else { 0.0 };
            ws.call(
                "classify.set_train",
                &json!({ "name": cls, "val_fraction": held, "patience": 1000 }),
            )?;
            // marked words are few: many short epochs at a brisk rate, from scratch each time
            let mut a = json!({
                "name": cls, "epochs": args.get("epochs").cloned().unwrap_or(json!(200)),
                "lr": args.get("lr").cloned().unwrap_or(json!(0.01)), "fresh": true,
            });
            if let Some(w) = args.get("wait") {
                a["wait"] = w.clone();
            }
            ws.call("classify.train", &a)
        }
        "tagsets.tag" => tag(ws, g, s(args, "name"), s(args, "text")),
        "tagsets.score" => {
            let (_, t) = named(g, s(args, "name"))?;
            score(ws, &t)
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}
