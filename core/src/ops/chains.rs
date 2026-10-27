//! The app's chain operations (`chains.*`, docs/app-api.md): make, read, reorder, cut,
//! join and break chains (`graph::chains`). A chain is answered as `{key, title: {key,
//! text}, members: [{key, text}]}`.

use serde_json::{Value, json};

use super::{Res, id_arg, is_note};

use crate::graph::chains::{self, chain_of};
use crate::graph::record::label;
use crate::graph::{Graph, Reader};

pub fn routed(op: &str) -> bool {
    op.starts_with("chains.")
}

fn keys(args: &Value, k: &str) -> Result<Vec<u64>, String> {
    args.get(k)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|v| {
            let s = v.as_str().unwrap_or("");
            crate::graph::record::parse_label(s.trim().strip_prefix("note:").unwrap_or(s))
                .ok_or_else(|| format!("{s:?} isn't a key"))
        })
        .collect()
}

fn item(v: &impl Reader, id: u64) -> Value {
    let text = v.render(id).unwrap_or_default();
    let note = is_note(v, id);
    let text = if note { crate::note::split(&text).0 } else { text };
    json!({ "key": if note { format!("note:{}", label(id)) } else { label(id) }, "text": text })
}

/// a chain as the app reads it
pub fn chain_json(v: &impl Reader, id: u64) -> Result<Value, String> {
    let c = chain_of(v, id)?.ok_or_else(|| format!("{} isn't a chain", label(id)))?;
    Ok(json!({
        "key": label(id),
        "title": item(v, c.title),
        "members": c.members.iter().map(|&m| item(v, m)).collect::<Vec<_>>(),
    }))
}

pub fn call(g: &Graph, op: &str, args: &Value) -> Res {
    match op {
        "chains.make" => {
            let title = id_arg(args, "title")?;
            let members = keys(args, "members")?;
            let id = g.write(|tx| tx.make_chain(title, &members))?;
            chain_json(&g.view()?, id)
        }
        "chains.get" => chain_json(&g.view()?, id_arg(args, "key")?),
        // the members in a new order, or with some added or taken out
        "chains.set" => {
            let id = id_arg(args, "key")?;
            let members = keys(args, "members")?;
            g.write(|tx| tx.set_chain(id, &members))?;
            chain_json(&g.view()?, id)
        }
        // cut before member `at`: that member heads a new chain of the ones after it
        "chains.cut" => {
            let id = id_arg(args, "key")?;
            let at = args
                .get("at")
                .and_then(Value::as_u64)
                .ok_or("cut before which member? (at)")? as usize;
            let new = g.write(|tx| tx.cut_chain(id, at))?;
            let v = g.view()?;
            Ok(json!({ "chain": chain_json(&v, id)?, "new": new.map(|n| chain_json(&v, n)).transpose()? }))
        }
        "chains.join" => {
            let (a, b) = (id_arg(args, "key")?, id_arg(args, "with")?);
            g.write(|tx| tx.join_chains(a, b))?;
            chain_json(&g.view()?, a)
        }
        // the chain entry goes; its title and members stay as they are
        "chains.break" => {
            let id = id_arg(args, "key")?;
            let before = chain_json(&g.view()?, id)?;
            g.delete(id)?;
            Ok(before)
        }
        // the chains a text heads or is in
        "chains.of" => {
            let id = id_arg(args, "key")?;
            let v = g.view()?;
            let mut out: Vec<Value> = Vec::new();
            for c in chains::chains_with(&v, id)? {
                out.push(chain_json(&v, c)?);
            }
            Ok(Value::Array(out))
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::record::mime;

    #[test]
    fn chains_through_the_api() {
        let g = Graph::in_memory().unwrap();
        let k: Vec<String> = ["Title", "one", "two", "three"]
            .iter()
            .map(|t| label(g.save_text(t, mime::TEXT).unwrap().0))
            .collect();
        let c = |op: &str, a: Value| call(&g, op, &a);
        let ch = c("chains.make", json!({ "title": k[0], "members": [k[1], k[2], k[3]] })).unwrap();
        assert_eq!(ch["title"]["text"], "Title");
        assert_eq!(ch["members"].as_array().unwrap().len(), 3);
        let key = ch["key"].as_str().unwrap().to_string();
        let r = c("chains.set", json!({ "key": key, "members": [k[3], k[1]] })).unwrap();
        assert_eq!(r["members"][0]["text"], "three");
        let cut = c("chains.cut", json!({ "key": key, "at": 1 })).unwrap();
        assert_eq!(cut["new"]["title"]["text"], "one");
        assert_eq!(
            c("chains.of", json!({ "key": k[3] }))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        c("chains.join", json!({ "key": key, "with": cut["new"]["key"] })).unwrap();
        assert_eq!(
            c("chains.get", json!({ "key": key })).unwrap()["members"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        c("chains.break", json!({ "key": key })).unwrap();
        assert!(c("chains.get", json!({ "key": key })).is_err());
        assert_eq!(
            g.render(crate::graph::record::parse_label(&k[1]).unwrap()).unwrap(),
            "one"
        );
    }
}
