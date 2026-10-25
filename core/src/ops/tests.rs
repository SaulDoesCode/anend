use super::*;

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "anend-ops-{tag}-{}-{}",
        std::process::id(),
        crate::graph::store::system_ms()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

use std::path::PathBuf;

#[test]
fn saves_merge_forge() {
    let dir = temp("saves");
    let sv = Saves::new(&dir, "notes");
    let c = |op: &str, a: Value| sv.call(op, &a).unwrap();
    assert_eq!(c("hello", json!({}))["save"], "notes");
    let k = c("mem.saturate", json!({"text": "listen"}))["key"]
        .as_str()
        .unwrap()
        .to_string();
    let k2 = c("mem.saturate", json!({"text": "silent"}))["key"]
        .as_str()
        .unwrap()
        .to_string();
    c("lists.enlist", json!({"list": k, "member": k2}));
    let n = c(
        "notes.save",
        json!({"note": {"tags": ["a"]}, "text": "# Hello\n\nworld"}),
    );
    assert_eq!(n["tags"], json!(["a"]));
    assert!(sv.call("mem.gsearch", &json!({"bucket": "atbash", "key": 1})).is_err());
    let st = c("lists.stats", json!({}));
    assert!((st["dot"][0][0].as_f64().unwrap() - 1.0).abs() < 1e-9);
    // a copy holds the same
    c("saves.copy", json!({"name": "notes", "to": "two"}));
    c("saves.open", json!({"name": "two"}));
    assert_eq!(c("hello", json!({}))["counts"], json!({"notes": 1, "memories": 2}));
    c("mem.saturate", json!({"text": "truth"}));
    // merge the export of `two` into `notes`: only truth is new
    let b = c("saves.export", json!({}));
    c("saves.open", json!({"name": "notes"}));
    let m = c("saves.import", json!({"bundle": b, "merge": true}));
    assert_eq!((m["memories"].as_u64(), m["notes"].as_u64()), (Some(1), Some(0)), "{m}");
    // forge: notes take away a save holding only truth
    c("saves.new", json!({"name": "t"}));
    c("mem.saturate", json!({"text": "truth"}));
    let p = c(
        "forge.preview",
        json!({"ingredients": [{"save": "notes"}, {"save": "t", "effect": "take away"}]}),
    );
    assert_eq!(
        (p["memories"].as_u64(), p["notes"].as_u64(), p["lists"].as_u64()),
        (Some(2), Some(1), Some(1)),
        "{p}"
    );
    let f = c(
        "forge.make",
        json!({"name": "f", "ingredients": [{"save": "notes"}, {"save": "t", "effect": "take away"}]}),
    );
    assert_eq!(f["memories"], 2);
    c("saves.open", json!({"name": "f"}));
    assert_eq!(c("lists.stats", json!({}))["lists"][0]["count"], 1);
    assert_eq!(c("notes.tags", json!({})), json!([{"tag": "a", "count": 1}]));
    // rename the open one
    c("saves.rename", json!({"name": "f", "to": "g"}));
    assert_eq!(c("hello", json!({}))["save"], "g");
    let im = c("saves.import", json!({"bundle": b}));
    assert_eq!(im["save"], "two 2");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn kinds_detected_and_fixed() {
    let dir = temp("kinds");
    let sv = Saves::new(&dir, "notes");
    let c = |op: &str, a: Value| sv.call(op, &a);
    let k = c("mem.saturate", json!({"text": "https://anend.org"})).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let i = c("entries.info", json!({"key": k})).unwrap();
    assert_eq!(i["kind"], "link");
    assert_eq!(i["fixed"], false);
    assert_eq!(i["mime"], "text/plain");
    assert!(i["created"].as_str().unwrap().starts_with("20"));
    c("entries.set_kind", json!({"key": k, "kind": "phrase"})).unwrap();
    let i = c("entries.info", json!({"key": k})).unwrap();
    assert_eq!(
        (i["kind"].as_str(), i["detected"].as_str(), i["fixed"].as_bool()),
        (Some("phrase"), Some("link"), Some(true))
    );
    assert!(c("entries.set_kind", json!({"key": k, "kind": "potato"})).is_err());
    c("entries.set_kind", json!({"key": k, "kind": ""})).unwrap();
    assert_eq!(c("entries.info", json!({"key": k})).unwrap()["kind"], "link");
    assert!(c("entries.info", json!({"key": "9,9,9"})).is_err());
    drop(sv);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn space_features_name_lists_kinds_and_times() {
    let dir = temp("features");
    let sv = Saves::new(&dir, "notes");
    let c = |op: &str, a: Value| sv.call(op, &a).unwrap();
    let key = |t: &str| {
        c("mem.saturate", json!({ "text": t }))["key"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (fruit, plum, paris) = (key("fruit"), key("ripe plum 12"), key("Paris"));
    c("lists.enlist", json!({"list": fruit, "member": plum}));
    let f = c("space.features", json!({ "keys": [plum, paris, "9,9"] }));
    let all = f["features"].as_array().unwrap();
    assert_eq!(all.len(), 2, "an unknown key is left out: {f}");
    assert_eq!(all[0]["lists"], json!([fruit]));
    assert_eq!(all[0]["types"]["number"], 1);
    assert_eq!(all[0]["words"], 3);
    assert_eq!(all[1]["kind"], "name");
    assert!(all[1]["lists"].as_array().unwrap().is_empty());
    assert!(all[0]["created"].as_u64().unwrap() > 1_700_000_000_000);
    let (s, r) = crate::spread::sums("Paris");
    assert_eq!((all[1]["s"].as_i64(), all[1]["r"].as_i64()), (Some(s), Some(r)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn grammar_check_knows_the_saves_own_words() {
    let dir = temp("grammar");
    let sv = Saves::new(&dir, "notes");
    let c = |op: &str, a: Value| sv.call(op, &a).unwrap();
    let flagged = |a: Value| -> Vec<String> {
        c("grammar.check", a)["flags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["word"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(flagged(json!({ "text": "a zorbly widget" })).contains(&"zorbly".to_string()));
    // a word of the save's own is no error, unless only the note being checked has it
    let k = c("mem.saturate", json!({ "text": "zorbly" }))["key"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(flagged(json!({ "text": "a zorbly widget" })).is_empty());
    assert!(flagged(json!({ "text": "a zorbly widget", "exclude": k })).contains(&"zorbly".to_string()));
    let r = c("grammar.report", json!({}));
    assert!(r["precision"].as_f64().unwrap() > 0.5 && r["recall"].as_f64().unwrap() > 0.5);
    drop(sv);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn random_search_samples_texts_sharing_a_word() {
    let dir = temp("random");
    let sv = Saves::new(&dir, "notes");
    let c = |op: &str, a: Value| sv.call(op, &a).unwrap();
    for t in ["red fox", "red hen", "blue jay", "red sky", "grey owl"] {
        c("mem.saturate", json!({ "text": t }));
    }
    let texts = |a: Value| -> Vec<String> {
        c("mem.random", a)
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["text"].as_str().unwrap().to_string())
            .collect()
    };
    let reds = texts(json!({ "q": "Red", "seed": 7 }));
    assert_eq!(reds.len(), 3);
    assert!(reds.iter().all(|t| t.starts_with("red")));
    // the same seed, the same order; a limit takes the first of them
    assert_eq!(reds, texts(json!({ "q": "red", "seed": 7 })));
    assert_eq!(texts(json!({ "seed": 3, "limit": 2 })).len(), 2);
    assert_eq!(texts(json!({ "seed": 3 })).len(), 5);
    drop(sv);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn limits_are_settings_with_defaults() {
    let dir = temp("limits");
    let sv = Saves::new(&dir, "notes");
    let c = |op: &str, a: Value| sv.call(op, &a);
    let all = c("limits.list", json!({})).unwrap();
    let get = |all: &Value, n: &str| all.as_array().unwrap().iter().find(|l| l["name"] == n).unwrap()["value"].clone();
    assert_eq!(get(&all, "limits.texts_per_call"), 2000);
    assert_eq!(get(&all, "limits.render_output_bytes"), 0, "no cap by default");
    // a smaller spelling-check limit bites, and empty puts the default back
    c("limits.set", json!({"name": "limits.grammar_check_bytes", "value": 10})).unwrap();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let e = c("grammar.check", json!({"text": "a text longer than ten bytes"})).unwrap_err();
        assert!(e.contains("limits.grammar_check_bytes"), "{e}");
    }
    c("limits.set", json!({"name": "limits.grammar_check_bytes", "value": ""})).unwrap();
    assert_eq!(
        get(&c("limits.list", json!({})).unwrap(), "limits.grammar_check_bytes"),
        200_000
    );
    assert!(c("limits.set", json!({"name": "limits.nothing", "value": 1})).is_err());
    assert!(c("limits.set", json!({"name": "limits.texts_per_call", "value": "lots"})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
