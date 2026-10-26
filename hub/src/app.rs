//! The app, run by the hub: each member's saves live on the server in a workspace of
//! their own (with `--local`, one workspace and no login, for a daemon on your own
//! machine), and the web app works on them through one dispatcher, reached over a
//! WebSocket (`/app/ws`) or plain posts (`/app/api/{op}`). Every call is
//! `{"id", "op", "args"}` and every answer `{"id", "ok": true, "data"}` or
//! `{"id", "ok": false, "error"}`; docs/app-api.md lists the operations.
//!
//! A save is an entry graph (`<name>.redb`), answered by `anend_core::ops`, the same code
//! as the desktop. Its classifiers keep their own tables beside it (`<name>.models`, an
//! `anend_core::store::Store` reading the graph's texts and lists through
//! `ops::GraphTexts`), renamed, copied, deleted and exported with it.

use anend_core::graph::Graph;
use anend_core::ops::classify::Limits;
use anend_core::ops::{self, Saves};
use anend_core::store::Store;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::store::User;

pub type Res = Result<Value, String>;

pub use anend_core::ops::classify::MODELS;

/// whose saves a call works on: on a `--local` server (`local`) the one workspace;
/// otherwise the signed-in, active member's own
pub fn owner(user: Option<&User>, local: bool) -> Result<String, String> {
    if local {
        return Ok("local".into());
    }
    match user {
        Some(u) if crate::hub::Hub::active(u) => Ok(u.name.to_lowercase()),
        Some(_) => Err("your account is waiting for an admin".into()),
        None => Err("log in to use the app here".into()),
    }
}

pub struct Workspace {
    pub dir: PathBuf,
    pub saves: Saves,
}

pub struct Workspaces {
    root: PathBuf,
    all: Mutex<HashMap<String, Arc<Workspace>>>,
}

pub static WORKSPACES: OnceLock<Workspaces> = OnceLock::new();

pub fn workspaces() -> &'static Workspaces {
    WORKSPACES.get().expect("workspaces start with the hub")
}

impl Workspaces {
    pub fn new(root: PathBuf) -> Workspaces {
        Workspaces {
            root,
            all: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, owner: &str) -> Arc<Workspace> {
        let mut all = self.all.lock().unwrap_or_else(|e| e.into_inner());
        all.entry(owner.to_string())
            .or_insert_with(|| Arc::new(Workspace::new(self.root.join(owner).join("saves"))))
            .clone()
    }
}

/// how much disk a member's saves may fill on a shared hub, in bytes (`--quota`; unset or
/// 0: no limit, and a `--local` hub has none)
pub static QUOTA: OnceLock<u64> = OnceLock::new();
/// members' trainings may run on the GPU (`--gpu`; a `--local` hub always may)
pub static GPU: OnceLock<bool> = OnceLock::new();
/// how many saves a member may keep on a shared hub
pub const MAX_SAVES: usize = 200;

/// calls that only read, or make room: they work whatever the quota says
const CLASSIFY_READ_ONLY: &[&str] = &[
    "classify.list",
    "classify.get",
    "classify.delete",
    "classify.job",
    "classify.cancel",
    "classify.predict",
    "classify.search",
    "classify.contributions",
    "classify.represent",
    "classify.similar",
    "classify.index_job",
    "classify.index_cancel",
    "classify.buckets",
    "classify.bucket",
    "classify.near",
    "classify.tints",
    "classify.indexed",
    "classify.disagreements",
    "classify.export",
    "classify.inspect",
    "filters.list",
    "filters.delete",
    "filters.run",
    "models.list",
    "models.get",
    "models.check",
    "models.pretrained",
    "models.delete",
    "models.job",
    "models.cancel",
    "models.run",
    "models.generate",
    "pictures.list",
    "pictures.lists",
    "pictures.get",
    "pictures.delete",
    "blocks.list",
    "blocks.stats",
    "blocks.delete",
    "textures.list",
    "textures.delete",
];

fn makes_no_room(op: &str) -> bool {
    ops::READ_ONLY.contains(&op) || CLASSIFY_READ_ONLY.contains(&op)
}

/// a size for people
fn mb(b: u64) -> String {
    crate::hub::human_size(b)
}

/// a save's name made safe for a file name
pub fn clean(name: &str) -> Result<String, String> {
    ops::clean(name)
}

impl Workspace {
    pub fn new(dir: PathBuf) -> Workspace {
        Workspace {
            saves: Saves::new(&dir, "notes")
                .remembering()
                .backups_answered()
                .with_models(if crate::mode::local() {
                    Limits::NONE
                } else {
                    Limits {
                        gpu: GPU.get().copied().unwrap_or(false),
                        ..Limits::shared()
                    }
                }),
            dir,
        }
    }

    pub fn path(&self, name: &str) -> Result<PathBuf, String> {
        self.saves.path(name)
    }

    /// the open save's name and graph
    pub fn graph(&self) -> Result<(String, Arc<Graph>), String> {
        self.saves.current()
    }

    /// a save's classifiers (the open save's when `name` is empty)
    pub fn models_of(&self, name: &str) -> Result<(String, Arc<Store>), String> {
        self.saves.models_of(name)
    }

    /// the open save's name and its classifiers
    pub fn models(&self) -> Result<(String, Arc<Store>), String> {
        self.models_of("")
    }

    /// whether the workspace may grow (and, with `new_save`, hold one more save): on a
    /// shared hub its saves stay under the quota and [`MAX_SAVES`]
    pub fn room(&self, new_save: bool) -> Result<(), String> {
        if crate::mode::local() {
            return Ok(());
        }
        self.room_under(QUOTA.get().copied().unwrap_or(0), new_save)
    }

    fn room_under(&self, quota: u64, new_save: bool) -> Result<(), String> {
        if quota == 0 {
            return Ok(());
        }
        let mut used = 0u64;
        let mut saves = 0usize;
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
            if ext == "redb" || ext == MODELS {
                used += e.metadata().map(|m| m.len()).unwrap_or(0);
                saves += (ext == "redb") as usize;
            }
        }
        if used >= quota {
            return Err(format!(
                "your saves fill {} of the {} this hub keeps for each member: delete or shrink some to make room",
                mb(used),
                mb(quota)
            ));
        }
        if new_save && saves >= MAX_SAVES {
            return Err(format!(
                "that's {MAX_SAVES} saves, as many as the hub keeps for each member: delete one first"
            ));
        }
        Ok(())
    }
}

/// one operation on a workspace; runs off the async threads (redb blocks)
pub fn call(ws: &Workspace, op: &str, args: &Value) -> Res {
    if !makes_no_room(op) {
        let merging = op == "saves.import" && args.get("merge").and_then(Value::as_bool) == Some(true);
        let into_open = op == "saves.import_texts" && ops::s(args, "name").trim().is_empty();
        ws.room(ops::MAKE_A_SAVE.contains(&op) && !merging && !into_open)?;
    }
    ws.saves.call(op, args)
}

/// a hub save (an entry graph file) into the workspace (`merge`: into the open save
/// instead of beside it)
pub fn take_hub_save(ws: &Workspace, hub_file: &Path, name: &str, merge: bool) -> Res {
    ws.room(!merge)?;
    if !Graph::is_save(hub_file) {
        return Err("that isn't an anend save (an entry graph .redb file from the app)".into());
    }
    if merge {
        let (_, g) = ws.graph()?;
        let src = ops::graph_at(hub_file)?;
        let v = src.view()?;
        let w = g.write(|tx| ops::merge::merge(tx, &v))?;
        drop(v);
        let a = ops::merge::assets(&g, &src)?;
        return Ok(
            json!({ "merged": true, "memories": w.memories, "notes": w.notes, "lists": w.lists,
                          "models": a.models, "pictures": a.pictures }),
        );
    }
    let n = ws.saves.free_name(name)?;
    std::fs::create_dir_all(&ws.dir).map_err(|e| e.to_string())?;
    std::fs::copy(hub_file, ws.path(&n)?).map_err(|e| e.to_string())?;
    Ok(json!({ "save": n }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("anend-{tag}-{}", rand::random::<u64>()))
    }

    /// every op of the app API on the graph, through a workspace
    #[test]
    fn workspace_calls() {
        let dir = temp("app");
        let ws = Workspace::new(dir.clone());
        let c = |op: &str, a: Value| call(&ws, op, &a).unwrap_or_else(|e| panic!("{op}: {e}"));
        let v = c("hello", json!({}));
        assert_eq!(v["save"], "notes");
        let listen = c("mem.saturate", json!({"text": "listen"}))["key"]
            .as_str()
            .unwrap()
            .to_string();
        let silent = c("mem.saturate", json!({"text": "silent"}))["key"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!((listen.as_str(), silent.as_str()), ("0", "1"));
        assert_eq!(c("mem.saturate", json!({"text": "listen"}))["new"], false);
        let g = c("mem.gsearch", json!({"bucket": "ordinal", "key": 79}));
        assert_eq!(g["hits"].as_array().unwrap().len(), 2);
        assert_eq!(g["hits"][0]["ordinal"], 79);
        let g = c("mem.gsearch", json!({"bucket": "frequency", "key": "enlist"}));
        assert_eq!(g["hits"].as_array().unwrap().len(), 2);
        assert!(
            call(&ws, "mem.gsearch", &json!({"bucket": "atbash", "key": 1}))
                .unwrap_err()
                .contains("removed")
        );
        let m = c("mem.measure", json!({"text": "truth"}));
        assert_eq!(
            (m["s"].as_i64(), m["r"].as_i64(), m["triangle"].as_i64()),
            (Some(87), Some(48), Some(64))
        );
        assert!(m.get("atbash").is_none());
        assert_eq!(c("mem.search", json!({"q": "listen"})).as_array().unwrap().len(), 1);
        assert_eq!(c("mem.all", json!({})).as_array().unwrap().len(), 2);
        assert_eq!(
            c("mem.buckets", json!({"bucket": "ordinal"}))["buckets"],
            json!([{"sum": 79, "count": 2}])
        );
        // forget and put back: lists keep the member
        c("lists.enlist", json!({"list": listen, "member": silent}));
        assert_eq!(c("mem.forget", json!({"key": silent}))["text"], "silent");
        assert!(call(&ws, "mem.remember", &json!({"key": silent})).is_err());
        c("mem.put", json!({"key": silent, "text": "silent"}));
        assert_eq!(c("mem.remember", json!({"key": silent}))["text"], "silent");
        assert_eq!(c("lists.stats", json!({}))["lists"][0]["count"], 1);

        // notes: entries with a # title
        let n = c("notes.save", json!({"note": {}, "text": "# Hello\n\nworld listen"}));
        let key = n["_key"].as_str().unwrap().to_string();
        assert_eq!(c("notes.list", json!({"q": "world"})).as_array().unwrap().len(), 1);
        assert!(call(&ws, "notes.save", &json!({"note": {}, "text": "no title"})).is_err());
        let n2 = c(
            "notes.save",
            json!({"note": {"_key": key, "tags": ["x"]}, "text": "# Hello\n\nworld again"}),
        );
        assert_eq!(n2["_key"], key.as_str());
        assert_eq!(n2["edits"].as_array().unwrap().len(), 1);
        assert_eq!(c("notes.get", json!({"key": key}))["markdown"], "world again");
        // a note on an entry
        c("notes.save", json!({"note": {}, "text": "# on listen", "on": listen}));
        assert_eq!(c("notes.on", json!({"key": listen}))[0]["text"], "on listen");

        // tags: counts, and notes by all or any of them
        c("notes.save", json!({"note": {"tags": ["a", "b"]}, "text": "# ab"}));
        c("notes.save", json!({"note": {"tags": ["a"]}, "text": "# a"}));
        c("notes.save", json!({"note": {"tags": ["c"]}, "text": "# c"}));
        assert_eq!(
            c("notes.tags", json!({})),
            json!([{"tag": "a", "count": 2}, {"tag": "b", "count": 1}, {"tag": "c", "count": 1}, {"tag": "x", "count": 1}])
        );
        let titles = |args: Value| -> Vec<String> {
            c("notes.list", args)
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["title"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(titles(json!({"tags": ["a", "b"]})), ["ab"]);
        assert_eq!(titles(json!({"tags": ["b", "c"], "mode": "or"})).len(), 2);
        assert_eq!(titles(json!({"tags": ["a"], "q": "zzz"})).len(), 0);
        assert_eq!(titles(json!({"tags": []})).len(), 5);
        c("tags.add", json!({"key": listen, "tag": "#word"}));
        assert_eq!(c("tags.entries", json!({"tags": ["word"]}))[0]["text"], "listen");
        c("tags.remove", json!({"key": listen, "tag": "word"}));
        assert_eq!(c("tags.entries", json!({"tags": ["word"]})), json!([]));
        assert_eq!(c("entries.query", json!({"kind": "notes"}))["total"], 5);
        assert_eq!(
            c("entries.query", json!({"bucket": "ordinal", "from": 70, "to": 80}))["total"],
            2
        );
        c("settings.set", json!({"name": "space", "value": "{}"}));
        assert_eq!(c("settings.get", json!({"name": "space"}))["value"], "{}");
        let st = c("stats", json!({}));
        assert_eq!((st["notes"].as_u64(), st["memories"].as_u64()), (Some(5), Some(2)));

        c("lists.enlist", json!({"list": silent, "member": listen}));
        let st = c("lists.stats", json!({}));
        assert_eq!(st["lists"].as_array().unwrap().len(), 2);
        // listen and silent are anagrams: their letter vectors point the same way
        assert!((st["dot"][0][1].as_f64().unwrap() - 1.0).abs() < 1e-9);
        c("lists.delist", json!({"list": silent, "member": listen}));
        assert_eq!(c("lists.unlist", json!({"list": listen}))["removed"], 1);

        let sp = c(
            "space.spread",
            json!({"texts": [{"id": "a", "text": "listen"}, {"id": "b", "text": "silent"}, {"id": "c", "text": "truth"}]}),
        );
        assert_eq!(sp["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(sp["buckets"], 2);

        // saves and the forge
        c("saves.new", json!({"name": "second"}));
        c("mem.saturate", json!({"text": "silent"}));
        let p = c(
            "forge.preview",
            json!({"ingredients": [{"save": "notes"}, {"save": "second", "effect": "take away"}]}),
        );
        assert_eq!((p["memories"].as_u64(), p["notes"].as_u64()), (Some(1), Some(5)), "{p}");
        let many: Vec<Value> = (0..=32).map(|_| json!({"save": "notes"})).collect();
        assert!(
            call(&ws, "forge.preview", &json!({"ingredients": many}))
                .unwrap_err()
                .contains("at most")
        );
        c(
            "forge.make",
            json!({"name": "subset", "ingredients": [{"save": "notes"}, {"save": "second", "effect": "take away"}]}),
        );
        assert_eq!(c("saves.list", json!({})).as_array().unwrap().len(), 3);
        c("forge.pour", json!({"ingredients": [{"save": "subset"}]}));
        assert_eq!(c("hello", json!({}))["counts"]["notes"], 5);
        assert!(
            call(&ws, "saves.delete", &json!({"name": "second"})).is_err(),
            "the open one stays"
        );
        c("saves.open", json!({"name": "notes"}));
        c("saves.copy", json!({"name": "notes", "to": "copied"}));
        let b = c("saves.export", json!({}));
        assert_eq!(b["anend"], 2);
        let got = c("saves.import", json!({"bundle": b}));
        assert_eq!(got["save"], "notes 2");
        assert_eq!(c("saves.export", json!({"name": "notes 2"}))["memories"], b["memories"]);
        assert_eq!(c("saves.export", json!({"name": "copied"}))["memories"], b["memories"]);
        assert_eq!(c("saves.import", json!({"bundle": b, "merge": true}))["memories"], 0);
        c("saves.delete", json!({"name": "notes 2"}));
        // a bundle it can't read leaves nothing behind
        assert!(
            call(
                &ws,
                "saves.import",
                &json!({"bundle": {"anend": 2, "memories": 3}, "name": "bad"})
            )
            .is_err()
        );
        assert!(!ws.path("bad").unwrap().exists());
        // weum's all-of-it.json and a backup's two documents
        let im = c(
            "saves.import_texts",
            json!({"data": "[[\"truth\", [9]]]", "name": "weum"}),
        );
        assert_eq!(im["memories"], 1);
        let bk = c("saves.backup", json!({}));
        assert!(bk["texts"].is_array() && bk["other"].is_object());
        let back = c(
            "saves.import_texts",
            json!({"data": bk["texts"].to_string(), "other": bk["other"], "name": "restored"}),
        );
        assert_eq!(back["notes"], 5);
        c("notes.delete", json!({"key": key}));
        c("saves.rename", json!({"name": "notes", "to": "renamed"}));
        assert_eq!(c("hello", json!({}))["save"], "renamed");
        assert!(call(&ws, "nonsense", &json!({})).is_err());
        assert!(clean("../x").is_ok_and(|n| !n.contains('/')));
        std::fs::remove_dir_all(dir).ok();
    }

    /// a member's saves stay under the quota: past it, only reading and deleting work
    #[test]
    fn quota() {
        let dir = temp("quota");
        let ws = Workspace::new(dir.clone());
        call(&ws, "mem.saturate", &json!({"text": "listen"})).unwrap();
        let used: u64 = std::fs::metadata(ws.path("notes").unwrap()).unwrap().len();
        assert!(ws.room_under(0, true).is_ok(), "0 is no limit");
        assert!(ws.room_under(used + 1, true).is_ok());
        let full = ws.room_under(used, false).unwrap_err();
        assert!(full.contains("delete or shrink"), "{full}");
        // the calls that need room are the ones not listed as making none
        for op in [
            "notes.save",
            "mem.saturate",
            "mem.put",
            "tags.add",
            "saves.new",
            "saves.import",
            "saves.import_texts",
            "forge.pour",
            "classify.train",
            "filters.save",
            "models.create",
            "models.update",
            "models.train",
            "pictures.add",
            "pictures.tag",
        ] {
            assert!(!makes_no_room(op), "{op}");
        }
        for op in ops::READ_ONLY {
            assert!(!ops::MAKE_A_SAVE.contains(op), "{op}");
        }
        for i in 0..MAX_SAVES {
            std::fs::write(dir.join(format!("s{i}.redb")), b"").unwrap();
        }
        assert!(ws.room_under(u64::MAX, false).is_ok());
        assert!(ws.room_under(u64::MAX, true).unwrap_err().contains("delete one first"));
        drop(ws);
        std::fs::remove_dir_all(dir).ok();
    }
}
