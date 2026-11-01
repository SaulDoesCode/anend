//! Groups (the user's 27 Oct direction): things in the room grouped by hand (Ctrl+G) to be
//! selected, moved, turned, copied and made into a prefab as one. A group's members are
//! blocks (`b`), prefab copies (`c`) and other groups (`g`), so groups nest; a thing is
//! directly in one group at most. A group is an entry of the save (mime
//! `application/x-anend-group`, its text the [`Group`]'s JSON), out of search and counts;
//! its id is `group:<key>`. Members that are gone (deleted blocks, removed copies) are left
//! out when the groups are read, and a group left with fewer than two goes.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::taxonomy::{entries, put};
use super::{Res, s};
use crate::graph::Graph;
use crate::graph::record::{self, mime};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Member {
    /// b (a block), c (a prefab's copy) or g (a group)
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Group {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub members: Vec<Member>,
}

pub fn routed(op: &str) -> bool {
    op.starts_with("groups.")
}

fn gid(id: u64) -> String {
    format!("group:{}", record::label(id))
}

fn entry_of(id: &str) -> Result<u64, String> {
    id.strip_prefix("group:")
        .and_then(record::parse_label)
        .ok_or_else(|| format!("{id:?} isn't a group"))
}

/// the room's groups as they stand: gone members out, groups of fewer than two gone
fn live(g: &Graph) -> Result<Vec<(String, Group)>, String> {
    let blocks: HashSet<String> = g.blocks(None)?.into_iter().map(|b| b.id).collect();
    let copies: HashSet<String> = g.copies()?.into_iter().map(|c| c.id).collect();
    let mut all: HashMap<String, Group> = entries::<Group>(g, mime::GROUP)?
        .into_iter()
        .map(|(id, gr)| (gid(id), gr))
        .collect();
    // members gone, then groups emptied, until nothing changes
    loop {
        let ids: HashSet<String> = all.keys().cloned().collect();
        let mut changed = false;
        for gr in all.values_mut() {
            let before = gr.members.len();
            gr.members.retain(|m| match m.kind.as_str() {
                "b" => blocks.contains(&m.id),
                "c" => copies.contains(&m.id),
                "g" => ids.contains(&m.id),
                _ => false,
            });
            changed |= gr.members.len() != before;
        }
        let small: Vec<String> = all
            .iter()
            .filter(|(_, gr)| gr.members.len() < 2)
            .map(|(k, _)| k.clone())
            .collect();
        changed |= !small.is_empty();
        for k in small {
            all.remove(&k);
        }
        if !changed {
            break;
        }
    }
    let mut out: Vec<(String, Group)> = all.into_iter().collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn listed(g: &Graph) -> Res {
    Ok(json!(
        live(g)?
            .into_iter()
            .map(|(id, gr)| json!({ "id": id, "name": gr.name, "members": gr.members }))
            .collect::<Vec<_>>()
    ))
}

pub fn call(g: &Graph, op: &str, args: &Value) -> Res {
    match op {
        "groups.list" => listed(g),
        // `{members: [{kind, id}], name?}`: a new group of them; the groups
        "groups.make" => {
            let members: Vec<Member> = serde_json::from_value(args.get("members").cloned().unwrap_or(json!([])))
                .map_err(|e| format!("members are {{kind, id}}: {e}"))?;
            let mut seen = HashSet::new();
            let members: Vec<Member> = members.into_iter().filter(|m| seen.insert(m.clone())).collect();
            if members.len() < 2 {
                return Err("a group needs two things or more".into());
            }
            let now = live(g)?;
            let blocks: HashSet<String> = g.blocks(None)?.into_iter().map(|b| b.id).collect();
            let copies: HashSet<String> = g.copies()?.into_iter().map(|c| c.id).collect();
            for m in &members {
                let there = match m.kind.as_str() {
                    "b" => blocks.contains(&m.id) && !m.id.starts_with("prefab:"),
                    "c" => copies.contains(&m.id),
                    "g" => now.iter().any(|(k, _)| *k == m.id),
                    _ => false,
                };
                if !there {
                    return Err(format!(
                        "{} {:?} can't be grouped (it's not in the room, or it's a copy's part)",
                        m.kind, m.id
                    ));
                }
                if let Some((k, _)) = now.iter().find(|(_, gr)| gr.members.contains(m)) {
                    return Err(format!(
                        "{:?} is in the group {k} already: select that group to group it",
                        m.id
                    ));
                }
            }
            let id = put(
                g,
                None,
                mime::GROUP,
                &Group {
                    name: s(args, "name").trim().to_string(),
                    members,
                },
            )?;
            let mut out = json!({ "id": gid(id) });
            out["groups"] = listed(g)?;
            Ok(out)
        }
        // `{id}`: the group undone; its members stay (a nested group stays a group)
        "groups.ungroup" => {
            let id = s(args, "id");
            let e = entry_of(id)?;
            let gr = live(g)?
                .into_iter()
                .find(|(k, _)| k == id)
                .ok_or_else(|| format!("there's no group {id}"))?
                .1;
            g.delete(e)?;
            Ok(json!({ "members": gr.members, "groups": listed(g)? }))
        }
        // `{id, name}`
        "groups.rename" => {
            let id = s(args, "id");
            let e = entry_of(id)?;
            let mut gr = live(g)?
                .into_iter()
                .find(|(k, _)| k == id)
                .ok_or_else(|| format!("there's no group {id}"))?
                .1;
            gr.name = s(args, "name").trim().to_string();
            put(g, Some(e), mime::GROUP, &gr)?;
            listed(g)
        }
        // `{members: [{kind, id}]}`: groups made again as they were (an undo of ungroup)
        "groups.restore" => {
            let members: Vec<Member> =
                serde_json::from_value(args.get("members").cloned().unwrap_or(json!([]))).map_err(|e| e.to_string())?;
            let id = put(
                g,
                None,
                mime::GROUP,
                &Group {
                    name: s(args, "name").to_string(),
                    members,
                },
            )?;
            let mut out = json!({ "id": gid(id) });
            out["groups"] = listed(g)?;
            Ok(out)
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_nest_and_drop_what_goes() {
        let g = Graph::in_memory().unwrap();
        let lim = super::super::classify::Limits::NONE;
        let add = |x: f32| {
            super::super::blocks::call(
                &g,
                "blocks.add",
                &json!({ "blocks": [{ "kind": "plain", "texture": "#aa0000", "x": x }] }),
                &lim,
            )
            .unwrap()["ids"][0]
                .as_str()
                .unwrap()
                .to_string()
        };
        let (a, b, c) = (add(0.0), add(2.0), add(4.0));
        let m = |k: &str, id: &str| json!({ "kind": k, "id": id });
        assert!(
            call(&g, "groups.make", &json!({ "members": [m("b", &a)] }))
                .unwrap_err()
                .contains("two")
        );
        let one = call(&g, "groups.make", &json!({ "members": [m("b", &a), m("b", &b)] })).unwrap();
        let one_id = one["id"].as_str().unwrap().to_string();
        // a block can't be in two groups; the group can be in another
        assert!(
            call(&g, "groups.make", &json!({ "members": [m("b", &a), m("b", &c)] }))
                .unwrap_err()
                .contains("already")
        );
        let two = call(
            &g,
            "groups.make",
            &json!({ "members": [m("g", &one_id), m("b", &c)], "name": "both" }),
        )
        .unwrap();
        assert_eq!(two["groups"].as_array().unwrap().len(), 2);
        // gone members drop out; a group of one goes, and the group holding it shrinks
        super::super::blocks::call(&g, "blocks.delete", &json!({ "ids": [b] }), &lim).unwrap();
        let l = call(&g, "groups.list", &json!({})).unwrap();
        assert_eq!(l.as_array().unwrap().len(), 0, "{l}");
        // ungroup keeps the members
        let x = call(&g, "groups.make", &json!({ "members": [m("b", &a), m("b", &c)] })).unwrap();
        let u = call(&g, "groups.ungroup", &json!({ "id": x["id"] })).unwrap();
        assert_eq!(u["members"].as_array().unwrap().len(), 2);
        assert_eq!(u["groups"], json!([]));
        let r = call(&g, "groups.restore", &json!({ "members": u["members"] })).unwrap();
        assert_eq!(r["groups"].as_array().unwrap().len(), 1);
    }
}
