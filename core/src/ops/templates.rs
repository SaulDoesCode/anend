//! Replacements in the editor (docs/app-api.md, "Replacements"): a note may say `{{0,12}}`
//! where entry 0,12's text belongs. It is stored as a live reference (`codec::Seg::Ref`,
//! the graph's transclusion), so the note shows that entry's text as it is now, and an
//! entry it refers to may refer to others in turn (recursively; a cycle is an error).
//! `{{filter:name}}` refers to a saved filter (the filters view's, kept in the save's
//! `filters` setting): a materialized view, its results put in as a list when the note
//! is read. Sums: a note is indexed by its rendered text, so putting `{{key}}` where that
//! entry's text was leaves the note's sums as they were (sum invariance), and when a
//! referenced entry changes, the notes referring to it are indexed again.
//!
//! Synonym and register lists: any list of the save is a set of words that may stand for
//! each other; `notes.alternatives` answers, for a word, the other members of the lists
//! it is in, each with its sums and whether it keeps the text's sum.

use serde_json::{Value, json};

use super::{Removed, Res, s};
use crate::graph::codec::Seg;
use crate::graph::gematria::Measure;
use crate::graph::record::{self, mime};
use crate::graph::store::rel;
use crate::graph::{Graph, Reader};

/// a text with `{{key}}` replacements as a run of text and refs; None when it has none
/// (the keys must be entries other than `me`); `{{filter:..}}` and anything else between
/// braces stays text
pub fn run_of<R: Reader>(r: &R, text: &str, me: Option<u64>) -> Result<Option<Vec<Seg>>, String> {
    let mut out: Vec<Seg> = Vec::new();
    let mut rest = text;
    let mut any = false;
    let mut lit = String::new();
    while let Some(a) = rest.find("{{") {
        let Some(b) = rest[a + 2..].find("}}") else { break };
        let inner = rest[a + 2..a + 2 + b].trim();
        lit.push_str(&rest[..a]);
        match record::parse_label(inner) {
            Some(id) if Some(id) != me && r.record(id)?.is_some() => {
                if !lit.is_empty() {
                    out.push(Seg::Text(std::mem::take(&mut lit)));
                }
                out.push(Seg::Ref(id));
                any = true;
            }
            _ => lit.push_str(&rest[a..a + 2 + b + 2]),
        }
        rest = &rest[a + 2 + b + 2..];
    }
    lit.push_str(rest);
    if !lit.is_empty() {
        out.push(Seg::Text(lit));
    }
    Ok(any.then_some(out))
}

/// an entry's text as written: its refs as `{{key}}`; None when it holds calls (a
/// function or template made elsewhere)
pub fn source_of<R: Reader>(r: &R, id: u64) -> Result<Option<String>, String> {
    let mut out = String::new();
    for seg in r.source(id)? {
        match seg {
            Seg::Text(t) => out.push_str(&t),
            Seg::Ref(x) => out.push_str(&format!("{{{{{}}}}}", record::label(x))),
            Seg::Call { .. } => return Ok(None),
        }
    }
    Ok(Some(out))
}

/// a saved filter's results (the filters view's, by name) as a markdown list
fn filter_view(g: &Graph, removed: &Removed, name: &str) -> Result<String, String> {
    let v = g.view()?;
    let raw = v.setting("filters")?.unwrap_or_default();
    let all: Vec<(String, Value)> = serde_json::from_slice(&raw).unwrap_or_default();
    let Some((_, q)) = all.iter().find(|(n, _)| n.trim().eq_ignore_ascii_case(name.trim())) else {
        return Ok(format!("(no saved filter {name:?})"));
    };
    let mut q = q.clone();
    q["limit"] = json!(50);
    let got = super::call(g, removed, "entries.query", &q)?;
    let lines: Vec<String> = got["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| h["text"].as_str().map(|t| format!("- {}", t.replace('\n', " "))))
        .collect();
    Ok(if lines.is_empty() {
        "(nothing)".into()
    } else {
        lines.join("\n")
    })
}

/// a rendered text with its `{{filter:name}}` views put in
pub fn with_views(g: &Graph, removed: &Removed, text: &str) -> Result<String, String> {
    if !text.contains("{{filter:") {
        return Ok(text.to_string());
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(a) = rest.find("{{filter:") {
        let Some(b) = rest[a..].find("}}") else { break };
        out.push_str(&rest[..a]);
        let name = &rest[a + "{{filter:".len()..a + b];
        let list = filter_view(g, removed, name)?;
        // a list wants a line of its own
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&list);
        out.push('\n');
        rest = &rest[a + b + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// the other members of the lists a word is in, with their sums: `{word, ordinal,
/// reversed, alternatives: [{text, key, list, ordinal, reversed, same_ordinal,
/// same_reversed}]}`, those keeping the ordinal sum first
pub fn alternatives(g: &Graph, args: &Value) -> Res {
    let word = s(args, "word").trim();
    if word.is_empty() {
        return Err("which word?".into());
    }
    let v = g.view()?;
    let m = Measure::of(word);
    let mut ids = v.find_text(word, Some(mime::TEXT))?;
    if ids.is_empty() && word.chars().any(char::is_uppercase) {
        ids = v.find_text(&word.to_lowercase(), Some(mime::TEXT))?;
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        for list in v.in_edges(id, rel::MEMBER)? {
            let name = v.render(list).unwrap_or_default();
            // the list's head (its name) stands for the others too
            for other in std::iter::once(list).chain(v.members(list)?) {
                if other == id || !seen.insert(other) {
                    continue;
                }
                let Ok(t) = v.render(other) else { continue };
                if t.trim().is_empty() || t.eq_ignore_ascii_case(word) {
                    continue;
                }
                let o = Measure::of(&t);
                out.push(json!({
                    "text": t, "key": record::label(other), "list": name,
                    "ordinal": o.ordinal, "reversed": o.reversed,
                    "same_ordinal": o.ordinal == m.ordinal, "same_reversed": o.reversed == m.reversed,
                }));
            }
        }
    }
    out.sort_by_key(|a| {
        (
            !a["same_ordinal"].as_bool().unwrap_or(false),
            !a["same_reversed"].as_bool().unwrap_or(false),
        )
    });
    Ok(json!({ "word": word, "ordinal": m.ordinal, "reversed": m.reversed, "alternatives": out }))
}

#[cfg(test)]
mod tests {
    use super::super::{Removed, call};
    use serde_json::{Value, json};

    use crate::graph::gematria::Measure;
    use crate::graph::{Graph, Reader};

    #[test]
    fn replacements_in_notes() {
        let g = Graph::in_memory().unwrap();
        let removed = Removed::default();
        let c = |op: &str, a: Value| call(&g, &removed, op, &a).unwrap_or_else(|e| panic!("{op}: {e}"));
        let key = |t: &str| {
            c("mem.saturate", json!({ "text": t }))["key"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let city = key("Paris in spring");
        // an entry that refers to another: recursive
        let trip = c(
            "notes.save",
            json!({ "note": {}, "text": format!("# trip\n\nto {{{{{city}}}}} soon") }),
        );
        let trip_key = trip["_key"].as_str().unwrap().to_string();
        assert_eq!(trip["markdown"], "to Paris in spring soon");
        assert_eq!(trip["source"], format!("# trip\n\nto {{{{{city}}}}} soon"));
        let plan = c(
            "notes.save",
            json!({ "note": {}, "text": format!("# plan\n\nthe {{{{{trip_key}}}}} and {{{{nothing}}}}") }),
        );
        assert_eq!(
            plan["markdown"],
            "the # trip\n\nto Paris in spring soon and {{nothing}}"
        );

        // sum invariance: the note with a ref has the sums of the same note written out
        let written = c(
            "notes.save",
            json!({ "note": {}, "text": "# trip2\n\nto Paris in spring soon" }),
        );
        let by_ref = c(
            "notes.save",
            json!({ "note": {}, "text": format!("# trip2\n\nto {{{{{city}}}}} soon") }),
        );
        let v = g.view().unwrap();
        let id = |k: &Value| crate::graph::record::parse_label(k.as_str().unwrap()).unwrap();
        let (a, b) = (
            v.render(id(&written["_key"])).unwrap(),
            v.render(id(&by_ref["_key"])).unwrap(),
        );
        assert_eq!(Measure::of(&a), Measure::of(&b));
        drop(v);
        // and they're found by the sum they share
        let m = Measure::of(&a);
        let found = c(
            "entries.query",
            json!({ "bucket": "ordinal", "from": m.ordinal, "to": m.ordinal }),
        );
        let keys: Vec<&str> = found["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["key"].as_str().unwrap())
            .collect();
        assert!(
            keys.contains(&written["_key"].as_str().unwrap()) && keys.contains(&by_ref["_key"].as_str().unwrap()),
            "{found}"
        );

        // when the referenced entry changes, the note shows it and is found by its words
        c("mem.put", json!({ "key": city, "text": "Rome in autumn" }));
        assert_eq!(
            c("notes.get", json!({ "key": trip_key }))["markdown"],
            "to Rome in autumn soon"
        );
        let found = c("entries.query", json!({ "words": "autumn", "kind": "notes" }));
        assert!(found["total"].as_u64().unwrap() >= 1, "{found}");

        // saving the source again keeps the ref (not the text it shows)
        let again = c(
            "notes.save",
            json!({ "note": {"_key": trip_key}, "text": trip["source"] }),
        );
        assert_eq!(again["source"], trip["source"]);
        // a note can't refer to itself
        let me = c(
            "notes.save",
            json!({ "note": {"_key": trip_key}, "text": format!("# trip\n\n{{{{{trip_key}}}}}") }),
        );
        assert!(
            me.get("source").is_none()
                || me["source"] == me["markdown"].as_str().map(|m| format!("# trip\n\n{m}")).unwrap()
        );

        // a filter view
        c(
            "settings.set",
            json!({ "name": "filters", "value": json!([["springs", {"words": "autumn", "kind": "memories"}]]).to_string() }),
        );
        let fv = c(
            "notes.save",
            json!({ "note": {}, "text": "# views\n\nthese: {{filter:springs}} done" }),
        );
        let got = c("notes.get", json!({ "key": fv["_key"] }));
        assert!(got["rendered"].as_str().unwrap().contains("- Rome in autumn"), "{got}");
        assert!(got["markdown"].as_str().unwrap().contains("{{filter:springs}}"));

        // alternatives from lists, the ones keeping the sum first
        let syn = key("big");
        for w in ["large", "huge", "vast"] {
            let k = key(w);
            c("lists.enlist", json!({ "list": syn, "member": k }));
        }
        let alt = c("notes.alternatives", json!({ "word": "large" }));
        let texts: Vec<&str> = alt["alternatives"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["text"].as_str().unwrap())
            .collect();
        assert!(
            texts.contains(&"huge") && texts.contains(&"vast") && texts.contains(&"big"),
            "{alt}"
        );
        assert!(!texts.contains(&"large"));
    }
}
