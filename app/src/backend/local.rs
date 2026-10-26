//! The desktop's backend: the entry graph (core/src/graph, docs/format.md) in this
//! process. Every call runs on a worker thread and answers through a channel, so the
//! interface never waits on the disk.
//!
//! The operations are the app API's (docs/app-api.md), answered by `anend_core::ops`, the
//! same code the hub answers them with: keys are weum labels of entry ids, a note is an
//! entry of mime `text/markdown` (`# title` and markdown), lists are `member` edges, tags
//! `tag` edges. A save's classifiers live in `<save>.models` beside it
//! (`anend_core::ops::classify`), trained on threads of their own as on the hub.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anend_core::ops::Saves;
use anend_core::ops::classify::Limits;
use serde_json::Value;

use super::{Backend, Fut};

type Res = Result<Value, String>;

/// the saves folder and the open save
#[derive(Clone)]
pub struct Local {
    saves: Arc<Saves>,
}

impl Local {
    /// opens `save` (a path), or `main.redb` in `dir`
    pub fn open(dir: &Path, save: Option<&Path>) -> Result<Local, String> {
        let path = save.map(Path::to_path_buf).unwrap_or_else(|| dir.join("main.redb"));
        Ok(Local {
            saves: Arc::new(Saves::at(&path)?.with_models(Limits::NONE).with_folders()),
        })
    }

    pub fn save_path(&self) -> PathBuf {
        self.saves.current_path().unwrap_or_default()
    }

    fn dispatch(&self, op: &str, args: &Value) -> Res {
        self.saves.call(op, args)
    }
}

impl Backend for Local {
    fn call(&self, op: &str, args: Value) -> Fut<Value> {
        let me = self.clone();
        let op = op.to_string();
        let (tx, rx) = futures_channel::oneshot::channel();
        let spawned = std::thread::Builder::new().name(format!("anend {op}")).spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| me.dispatch(&op, &args)))
                .unwrap_or_else(|_| Err(format!("{op} failed unexpectedly")));
            let _ = tx.send(r);
        });
        Box::pin(async move {
            spawned.map_err(|e| format!("couldn't start a worker: {e}"))?;
            rx.await.map_err(|_| "the worker stopped".to_string())?
        })
    }

    fn describe(&self) -> String {
        self.save_path().display().to_string()
    }

    fn on_disk(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gem::Mode;
    use serde_json::json;

    fn temp() -> (Local, PathBuf) {
        temp_named("main")
    }

    fn temp_named(tag: &str) -> (Local, PathBuf) {
        let dir = std::env::temp_dir().join(format!("anend-app-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Local::open(&dir, None).unwrap(), dir)
    }

    fn wait<T: 'static>(mut f: Fut<T>) -> Result<T, String> {
        loop {
            match futures_util::FutureExt::now_or_never(&mut f) {
                Some(r) => return r,
                None => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        }
    }

    #[test]
    fn texts_lists_tags_notes_backup() {
        let (b, dir) = temp();
        let (k, new) = wait(b.saturate("listen")).unwrap();
        assert!(new && k == "0");
        wait(b.saturate("silent")).unwrap();
        assert_eq!(wait(b.saturate("listen")).unwrap(), ("0".to_string(), false));
        assert_eq!(wait(b.gsearch(Mode::Ordinal, "79")).unwrap().len(), 2);
        assert_eq!(wait(b.gsearch(Mode::Frequency, "tinsel")).unwrap().len(), 2);
        wait(b.put_memory("0", "enlist")).unwrap();
        assert_eq!(wait(b.remember("0")).unwrap().text, "enlist");
        let counts = wait(b.bucket_counts(Mode::Ordinal)).unwrap();
        assert_eq!(counts, vec![super::super::BucketCount { sum: 79, count: 2 }]);
        // forget and put back: the list keeps its member
        wait(b.enlist("0", "1")).unwrap();
        assert_eq!(wait(b.lists()).unwrap().lists[0].count, 1);
        assert_eq!(wait(b.forget("1")).unwrap(), "silent");
        wait(b.put_memory("1", "silent")).unwrap();
        assert_eq!(wait(b.lists()).unwrap().lists[0].count, 1);
        // tags on texts
        wait(b.tag("0", "anagram")).unwrap();
        assert_eq!(wait(b.tagged(&["anagram".into()], false)).unwrap()[0].text, "enlist");
        // notes
        let n = wait(b.save_note(&Default::default(), "# hello\n\nworld")).unwrap();
        assert_eq!(n.title, "hello");
        assert_eq!(wait(b.notes(&Default::default())).unwrap().len(), 1);
        let h = wait(b.hello()).unwrap();
        assert_eq!(h.counts.notes, 1);
        let backup = wait(b.call("saves.backup", json!({}))).unwrap();
        assert_eq!(backup["paths"].as_array().unwrap().len(), 2);
        let im = wait(b.import_texts(r#"[["truth", [9]]]"#, "")).unwrap();
        assert_eq!(im.memories, 1);
        assert_eq!(wait(b.remember("9")).unwrap().text, "truth");
        std::fs::remove_dir_all(dir).ok();
    }

    /// classifiers on the desktop: made from lists, trained in the background, asked,
    /// told yes or no, and the answer taken back
    #[test]
    fn classifiers() {
        let (b, dir) = temp_named("classify");
        let f = ["apple", "banana", "cherry", "mango", "peach", "plum", "grape", "melon"];
        let r = [
            "rock", "stone", "granite", "basalt", "flint", "slate", "quartz", "shale",
        ];
        let (fruit, _) = wait(b.saturate("fruit")).unwrap();
        let (rocks, _) = wait(b.saturate("rocks")).unwrap();
        for i in 0..20 {
            let (a, _) =
                wait(b.saturate(&format!("{} {} {}", f[i % 8], f[(i * 3 + 1) % 8], f[(i / 8 + 4) % 8]))).unwrap();
            let (c, _) =
                wait(b.saturate(&format!("{} {} {}", r[i % 8], r[(i * 5 + 2) % 8], r[(i / 8 + 4) % 8]))).unwrap();
            wait(b.enlist(&fruit, &a)).unwrap();
            wait(b.enlist(&rocks, &c)).unwrap();
        }
        wait(b.create_model("kind", &[fruit.clone(), rocks.clone()], "tiny")).unwrap();
        let list = wait(b.models()).unwrap();
        assert_eq!((list[0].name.as_str(), list[0].trained), ("kind", false));
        assert_eq!(list[0].classes, ["fruit", "rocks"]);
        let j = wait(b.train_model("kind", 6)).unwrap();
        assert_eq!(j.state, "running");
        let mut done = j;
        for _ in 0..1200 {
            done = wait(b.model_job("kind")).unwrap();
            if done.state != "running" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(done.state, "done", "{done:?}");
        assert!(wait(b.models()).unwrap()[0].trained);
        let p = wait(b.run_model("kind", "granite slate flint")).unwrap();
        assert_eq!(p["results"][0]["class"], "rocks", "{p}");
        let t = "melon basalt grape";
        let prob = || {
            wait(b.run_model("kind", t)).unwrap()["results"][0]["probs"][1]
                .as_f64()
                .unwrap()
        };
        let p0 = prob();
        wait(b.model_feedback("kind", None, t, "rocks", true)).unwrap();
        let p1 = prob();
        assert!(p1 > p0, "{p0} {p1}");
        wait(b.undo_model_feedback("kind")).unwrap();
        assert_eq!(prob(), p0);
        // the classifiers go with the save: copied, then deleted with it
        wait(b.copy_save("main", "other")).unwrap();
        assert!(dir.join("other.models").exists());
        wait(b.delete_save("other")).unwrap();
        assert!(!dir.join("other.models").exists());
        wait(b.delete_model("kind")).unwrap();
        assert!(wait(b.models()).unwrap().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
