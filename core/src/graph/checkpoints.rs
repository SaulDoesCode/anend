//! Checkpoints: redb's persistent savepoints of a save's whole database, each with a name
//! and a time, to go back to. The names live in `graph meta` under `checkpoints` (JSON:
//! id → {name, at}); a savepoint holds the state from before its own name was written, so
//! going back writes the names of the checkpoints still there again. Going back to one
//! drops every checkpoint made after it (redb invalidates them) and rebuilds the in-memory
//! index.

use std::collections::BTreeMap;

use redb::{ReadableDatabase, ReadableTable};
use serde::{Deserialize, Serialize};

use super::store::{GResult, Graph, META, err};

const KEY: &str = "checkpoints";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: u64,
    pub name: String,
    /// when it was made, milliseconds since the Unix epoch
    pub at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Label {
    name: String,
    at: u64,
}

fn labels_in(meta: &impl ReadableTable<&'static str, &'static [u8]>) -> GResult<BTreeMap<u64, Label>> {
    Ok(match meta.get(KEY).map_err(err)? {
        Some(v) => serde_json::from_slice(v.value()).unwrap_or_default(),
        None => BTreeMap::new(),
    })
}

impl Graph {
    /// a checkpoint of the save as it is now; its id
    pub fn checkpoint(&self, name: &str) -> GResult<u64> {
        let txn = self.db().begin_write().map_err(err)?;
        let id = txn.persistent_savepoint().map_err(err)?;
        {
            let mut meta = txn.open_table(META).map_err(err)?;
            let mut labels = labels_in(&meta)?;
            labels.insert(
                id,
                Label {
                    name: name.trim().chars().take(120).collect(),
                    at: self.now(),
                },
            );
            meta.insert(KEY, serde_json::to_vec(&labels).map_err(err)?.as_slice())
                .map_err(err)?;
        }
        txn.commit().map_err(err)?;
        Ok(id)
    }

    /// the checkpoints there are, oldest first
    pub fn checkpoints(&self) -> GResult<Vec<Checkpoint>> {
        let txn = self.db().begin_write().map_err(err)?;
        let ids: Vec<u64> = txn.list_persistent_savepoints().map_err(err)?.collect();
        let labels = labels_in(&txn.open_table(META).map_err(err)?)?;
        txn.abort().map_err(err)?;
        let mut out: Vec<Checkpoint> = ids
            .into_iter()
            .map(|id| {
                let l = labels.get(&id).cloned().unwrap_or_default();
                Checkpoint {
                    id,
                    name: l.name,
                    at: l.at,
                }
            })
            .collect();
        out.sort_by_key(|c| c.id);
        Ok(out)
    }

    /// the save back as it was at a checkpoint (those made after it go)
    pub fn restore_checkpoint(&self, id: u64) -> GResult<()> {
        let labels = {
            let txn = self.db().begin_read().map_err(err)?;
            labels_in(&txn.open_table(META).map_err(err)?)?
        };
        let mut txn = self.db().begin_write().map_err(err)?;
        let sp = txn
            .get_persistent_savepoint(id)
            .map_err(|_| format!("no checkpoint {id}"))?;
        txn.restore_savepoint(&sp).map_err(err)?;
        let left: Vec<u64> = txn.list_persistent_savepoints().map_err(err)?.collect();
        {
            let mut meta = txn.open_table(META).map_err(err)?;
            let keep: BTreeMap<u64, Label> = labels.into_iter().filter(|(k, _)| left.contains(k)).collect();
            meta.insert(KEY, serde_json::to_vec(&keep).map_err(err)?.as_slice())
                .map_err(err)?;
        }
        txn.commit().map_err(err)?;
        self.reindex()
    }

    /// whether there was one
    pub fn delete_checkpoint(&self, id: u64) -> GResult<bool> {
        let txn = self.db().begin_write().map_err(err)?;
        let found = txn.delete_persistent_savepoint(id).map_err(err)?;
        {
            let mut meta = txn.open_table(META).map_err(err)?;
            let mut labels = labels_in(&meta)?;
            labels.remove(&id);
            meta.insert(KEY, serde_json::to_vec(&labels).map_err(err)?.as_slice())
                .map_err(err)?;
        }
        txn.commit().map_err(err)?;
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::record::mime;

    #[test]
    fn back_to_a_checkpoint() {
        let path = std::env::temp_dir().join(format!("anend-cp-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let g = Graph::open(&path).unwrap();
        g.save_text("listen", mime::TEXT).unwrap();
        let a = g.checkpoint("one text").unwrap();
        g.save_text("silent", mime::TEXT).unwrap();
        let b = g.checkpoint("two texts").unwrap();
        g.save_text("enlist", mime::TEXT).unwrap();
        let cps = g.checkpoints().unwrap();
        assert_eq!(
            cps.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["one text", "two texts"]
        );
        assert_eq!(g.entry_count_now(), 3);

        g.restore_checkpoint(b).unwrap();
        assert_eq!(g.entry_count_now(), 2);
        // its own name and the older one's are still there
        assert_eq!(g.checkpoints().unwrap().len(), 2);
        g.restore_checkpoint(a).unwrap();
        assert_eq!(g.entry_count_now(), 1);
        let left = g.checkpoints().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].name, "one text");
        // the index follows: search finds what's there
        assert!(g.view().unwrap().search_words("silent").unwrap().is_empty());
        assert!(g.delete_checkpoint(a).unwrap());
        assert!(g.checkpoints().unwrap().is_empty());
        assert!(g.restore_checkpoint(a).is_err());
        drop(g);
        let _ = std::fs::remove_file(&path);
    }

    impl Graph {
        fn entry_count_now(&self) -> u64 {
            self.view().unwrap().entry_count().unwrap()
        }
    }
}
