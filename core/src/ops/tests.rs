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
