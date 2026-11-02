//! Taxonomies (docs/architecture.md, part 1): the save's classifiers in a tree. Each node
//! names a classifier; its answer for a text picks the class, and the class may route the
//! text on to a further node, so "English or not?" can decide which classifier reads it
//! next. A taxonomy is an entry of the save (mime `application/x-anend-taxonomy`, its text
//! the [`Taxonomy`]'s JSON), kept out of search and counts like a block's wiring.
//!
//! Nothing about a text is stored: routing reads it afresh each time. What a taxonomy keeps
//! is how it was marked by hand: each node counts the routings someone marked right and
//! wrong there, and a mark is also the classifier's feedback (a yes for the class it gave,
//! or a no for it and a yes for the right one), so marking trains as it measures. Each
//! node shows that hand count beside its classifier's held-out accuracy.
//!
//! A node has one parent (a tree); a classifier can stand at several nodes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Res, Saves, s};
use crate::graph::query::{Page, Query};
use crate::graph::record::{self, mime};
use crate::graph::{Graph, Reader};

/// nodes a taxonomy has at most, and how deep it goes
const MAX_NODES: usize = 256;
const MAX_DEPTH: usize = 16;

/// one node: a classifier, where its classes lead, and its marks by hand
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub classifier: String,
    /// class → the node a text in it goes on to
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub routes: BTreeMap<String, Node>,
    /// routings marked right and wrong at this node
    #[serde(default)]
    pub right: u32,
    #[serde(default)]
    pub wrong: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Taxonomy {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    pub root: Node,
}

impl Node {
    fn count(&self) -> usize {
        1 + self.routes.values().map(Node::count).sum::<usize>()
    }

    fn depth(&self) -> usize {
        1 + self.routes.values().map(Node::depth).max().unwrap_or(0)
    }

    fn at(&self, path: &[String]) -> Option<&Node> {
        match path.split_first() {
            None => Some(self),
            Some((c, rest)) => self.routes.get(c)?.at(rest),
        }
    }

    fn at_mut(&mut self, path: &[String]) -> Option<&mut Node> {
        match path.split_first() {
            None => Some(self),
            Some((c, rest)) => self.routes.get_mut(c)?.at_mut(rest),
        }
    }

    /// marks from `old` where the node at the same place reads with the same classifier
    fn keep_marks(&mut self, old: Option<&Node>) {
        let (r, w) = match old {
            Some(o) if o.classifier == self.classifier => (o.right, o.wrong),
            _ => (0, 0),
        };
        self.right = r;
        self.wrong = w;
        for (c, n) in self.routes.iter_mut() {
            let o = old.and_then(|o| o.routes.get(c));
            n.keep_marks(o);
        }
    }

    /// every node, depth first, with its path
    fn walk<'a>(&'a self, path: &mut Vec<String>, out: &mut Vec<(Vec<String>, &'a Node)>) {
        out.push((path.clone(), self));
        for (c, n) in &self.routes {
            path.push(c.clone());
            n.walk(path, out);
            path.pop();
        }
    }
}

pub fn routed(op: &str) -> bool {
    op.starts_with("taxonomy.")
}

/// the save's entries of a mime whose text reads as `T`, by entry
pub(super) fn entries<T: serde::de::DeserializeOwned>(g: &Graph, m: u16) -> Result<Vec<(u64, T)>, String> {
    let v = g.view()?;
    let found = v.query(
        &Query {
            mime: Some(m),
            ..Query::default()
        },
        Page::all(),
    )?;
    let mut out = Vec::new();
    for id in found.ids {
        if let Ok(t) = serde_json::from_str::<T>(&v.render(id)?) {
            out.push((id, t));
        }
    }
    Ok(out)
}

/// writes an entry's JSON: a new entry, or the old one edited
pub(super) fn put<T: Serialize>(g: &Graph, id: Option<u64>, m: u16, t: &T) -> Result<u64, String> {
    let text = serde_json::to_string(t).map_err(|e| e.to_string())?;
    Ok(match id {
        Some(id) => {
            if g.render(id)? != text {
                g.edit_text(id, &text)?;
            }
            id
        }
        None => g.save_text(&text, m)?.0,
    })
}

/// the save's taxonomies, by name
fn all(g: &Graph) -> Result<Vec<(u64, Taxonomy)>, String> {
    let mut out: Vec<(u64, Taxonomy)> = entries(g, mime::TAXONOMY)?;
    out.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    Ok(out)
}

fn named(g: &Graph, name: &str) -> Result<(u64, Taxonomy), String> {
    all(g)?
        .into_iter()
        .find(|(_, t)| t.name == name)
        .ok_or_else(|| format!("there's no taxonomy called {name:?}"))
}

fn write(g: &Graph, id: Option<u64>, t: &Taxonomy) -> Result<u64, String> {
    put(g, id, mime::TAXONOMY, t)
}

/// a classifier as a taxonomy sees it: its classes, whether it's trained, its held-out accuracy
type Known = (Vec<String>, bool, Option<f64>);

/// the save's classifiers, by name
fn classifiers(ws: &Saves) -> Result<BTreeMap<String, Known>, String> {
    let list = ws.call("classify.list", &json!({}))?;
    Ok(list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?.to_string();
            let classes = c
                .get("classes")?
                .as_array()?
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            let trained = c.get("trained").and_then(Value::as_bool).unwrap_or(false);
            Some((name, (classes, trained, c.get("accuracy").and_then(Value::as_f64))))
        })
        .collect())
}

fn checked(ws: &Saves, t: &Taxonomy) -> Result<(), String> {
    if t.name.trim().is_empty() {
        return Err("a taxonomy needs a name".into());
    }
    if t.root.count() > MAX_NODES {
        return Err(format!("a taxonomy has at most {MAX_NODES} nodes"));
    }
    if t.root.depth() > MAX_DEPTH {
        return Err(format!("a taxonomy goes at most {MAX_DEPTH} classifiers deep"));
    }
    let cls = classifiers(ws)?;
    let mut nodes = Vec::new();
    t.root.walk(&mut Vec::new(), &mut nodes);
    for (path, n) in nodes {
        let at = if path.is_empty() {
            "at the top".to_string()
        } else {
            format!("after {}", path.join(" → "))
        };
        let Some((classes, _, _)) = cls.get(&n.classifier) else {
            return Err(format!("{at}: there's no classifier called {:?}", n.classifier));
        };
        if let Some(c) = n.routes.keys().find(|c| !classes.contains(c)) {
            return Err(format!("{at}: {:?} has no class {c:?}", n.classifier));
        }
    }
    Ok(())
}

/// a taxonomy with each node's place, classes, training and accuracies
fn details(ws: &Saves, id: u64, t: &Taxonomy) -> Res {
    let cls = classifiers(ws).unwrap_or_default();
    let mut nodes = Vec::new();
    t.root.walk(&mut Vec::new(), &mut nodes);
    let nodes: Vec<Value> = nodes
        .into_iter()
        .map(|(path, n)| {
            let (classes, trained, acc) = cls.get(&n.classifier).cloned().unwrap_or_default();
            let marked = n.right + n.wrong;
            json!({
                "path": path, "classifier": n.classifier, "classes": classes, "trained": trained,
                "held_out": acc, "right": n.right, "wrong": n.wrong,
                "by_hand": (marked > 0).then(|| n.right as f64 / marked as f64),
                "routes": n.routes.keys().collect::<Vec<_>>(),
                "missing": !cls.contains_key(&n.classifier),
            })
        })
        .collect();
    Ok(json!({ "key": record::label(id), "taxonomy": t, "nodes": nodes }))
}

/// the text to read: `{texts: [text]}` or `{keys: [key]}` for `classify.*`
fn reading(args: &Value) -> Result<Value, String> {
    let k = s(args, "key");
    if !k.is_empty() {
        return Ok(json!({ "keys": [k] }));
    }
    match args.get("text").and_then(Value::as_str) {
        Some(t) => Ok(json!({ "texts": [t] })),
        None => Err("give a text, or a key".into()),
    }
}

/// what the classifier at a node gives the text: (class, probability, trained)
fn predict(ws: &Saves, classifier: &str, what: &Value) -> Result<(Option<String>, f64, bool), String> {
    let mut a = what.clone();
    a["name"] = json!(classifier);
    let r = ws.call("classify.predict", &a)?;
    let first = &r["results"][0];
    let p = first["top"]
        .as_u64()
        .and_then(|t| first["probs"].get(t as usize))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    Ok((
        first["class"].as_str().map(String::from),
        p,
        r["trained"].as_bool().unwrap_or(false),
    ))
}

pub fn call(ws: &Saves, g: &Graph, op: &str, args: &Value) -> Res {
    match op {
        "taxonomy.list" => {
            let mut out = Vec::new();
            for (id, t) in all(g)? {
                let mut nodes = Vec::new();
                t.root.walk(&mut Vec::new(), &mut nodes);
                let (r, w) = nodes.iter().fold((0, 0), |(r, w), (_, n)| (r + n.right, w + n.wrong));
                out.push(json!({
                    "key": record::label(id), "name": t.name, "about": t.about, "nodes": nodes.len(),
                    "root": t.root.classifier, "right": r, "wrong": w,
                }));
            }
            Ok(json!(out))
        }
        "taxonomy.get" => {
            let (id, t) = named(g, s(args, "name"))?;
            details(ws, id, &t)
        }
        "taxonomy.save" => {
            let mut t: Taxonomy = serde_json::from_value(args.get("taxonomy").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("that isn't a taxonomy: {e}"))?;
            t.name = t.name.trim().to_string();
            checked(ws, &t)?;
            let was = match s(args, "was") {
                "" => t.name.clone(),
                w => w.to_string(),
            };
            let fresh = args.get("new").and_then(Value::as_bool).unwrap_or(false);
            let old = if fresh {
                None
            } else {
                all(g)?.into_iter().find(|(_, o)| o.name == was)
            };
            if (fresh || was != t.name) && all(g)?.iter().any(|(_, o)| o.name == t.name) {
                return Err(format!("there's a taxonomy called {:?} already", t.name));
            }
            t.root.keep_marks(old.as_ref().map(|(_, o)| &o.root));
            let id = write(g, old.map(|(id, _)| id), &t)?;
            details(ws, id, &t)
        }
        "taxonomy.delete" => {
            let (id, t) = named(g, s(args, "name"))?;
            g.delete(id)?;
            Ok(json!({ "removed": t.name }))
        }
        // a text down the tree: each node's classifier, its class and how sure it is,
        // until a class leads nowhere further (or a classifier isn't trained)
        "taxonomy.route" => {
            let (_, t) = named(g, s(args, "name"))?;
            let what = reading(args)?;
            let mut steps = Vec::new();
            let mut node = &t.root;
            let mut path: Vec<String> = Vec::new();
            let mut done = true;
            loop {
                let (class, p, trained) = predict(ws, &node.classifier, &what)?;
                let marked = node.right + node.wrong;
                steps.push(json!({
                    "path": path, "classifier": node.classifier, "class": class, "prob": p, "trained": trained,
                    "right": node.right, "wrong": node.wrong,
                    "by_hand": (marked > 0).then(|| node.right as f64 / marked as f64),
                }));
                if !trained {
                    done = false;
                    break;
                }
                match class.as_ref().and_then(|c| node.routes.get(c).map(|n| (c, n))) {
                    Some((c, next)) => {
                        path.push(c.clone());
                        node = next;
                    }
                    None => break,
                }
            }
            let leaf = steps.last().and_then(|s| s["class"].as_str()).map(String::from);
            Ok(json!({ "name": t.name, "steps": steps, "class": leaf, "reached": done }))
        }
        // marks the class a node gave a text: right (a yes for it), or wrong (a no for
        // it, and a yes for `class` when it's given); the node counts it
        "taxonomy.mark" => {
            let (id, mut t) = named(g, s(args, "name"))?;
            let path: Vec<String> = serde_json::from_value(args.get("path").cloned().unwrap_or(json!([])))
                .map_err(|_| "path is the classes leading to the node, as a list".to_string())?;
            let what = reading(args)?;
            let classifier = t
                .root
                .at(&path)
                .ok_or_else(|| format!("{:?} has no node after {}", t.name, path.join(" → ")))?
                .classifier
                .clone();
            let (gave, _, trained) = predict(ws, &classifier, &what)?;
            if !trained {
                return Err(format!("{classifier:?} isn't trained yet"));
            }
            let gave = gave.ok_or_else(|| format!("{classifier:?} gave no class"))?;
            let right = args.get("right").and_then(Value::as_bool).unwrap_or(false);
            let fb = |class: &str, yes: bool| -> Res {
                let mut a = what.clone();
                if let Some(k) = a.get("keys").and_then(|k| k.get(0)).cloned() {
                    a = json!({ "key": k });
                } else if let Some(x) = a.get("texts").and_then(|k| k.get(0)).cloned() {
                    a = json!({ "text": x });
                }
                a["name"] = json!(classifier);
                a["class"] = json!(class);
                a["yes"] = json!(yes);
                ws.call("classify.feedback", &a)
            };
            if right {
                fb(&gave, true)?;
            } else {
                fb(&gave, false)?;
                let class = s(args, "class");
                if !class.is_empty() && class != gave {
                    fb(class, true)?;
                }
            }
            let n = t.root.at_mut(&path).expect("found above");
            if right {
                n.right += 1;
            } else {
                n.wrong += 1;
            }
            write(g, Some(id), &t)?;
            details(ws, id, &t)
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}
