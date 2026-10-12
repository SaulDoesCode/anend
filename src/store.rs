//! Note storage in an embedded redb database: one table of JSON documents keyed by the
//! note's `_key`, plus a counter for handing out new keys. Every write is a durable
//! transaction.

use redb::{Database, ReadableTable, TableDefinition};
use std::path::Path;

use crate::note::Note;

const NOTES: TableDefinition<&str, &[u8]> = TableDefinition::new("notes");
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");

pub type StoreResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub struct Store {
    db: Database,
}

impl Store {
    pub fn open(path: &Path) -> StoreResult<Store> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        let db = Database::create(path).map_err(err)?;
        let txn = db.begin_write().map_err(err)?;
        txn.open_table(NOTES).map_err(err)?;
        txn.open_table(META).map_err(err)?;
        txn.commit().map_err(err)?;
        Ok(Store { db })
    }

    /// every note, newest first
    pub fn all(&self) -> StoreResult<Vec<Note>> {
        let txn = self.db.begin_read().map_err(err)?;
        let table = txn.open_table(NOTES).map_err(err)?;
        let mut notes = Vec::new();
        for item in table.iter().map_err(err)? {
            let (_, v) = item.map_err(err)?;
            notes.push(serde_json::from_slice::<Note>(v.value()).map_err(err)?);
        }
        sort_newest_first(&mut notes);
        Ok(notes)
    }

    /// stores a note, giving new notes a key and creation time and recording an edit
    /// time for existing ones; returns the stored version
    pub fn save(&self, note: &Note) -> StoreResult<Note> {
        let mut note = note.clone();
        let txn = self.db.begin_write().map_err(err)?;
        {
            if note.key.is_empty() {
                let mut meta = txn.open_table(META).map_err(err)?;
                let next = meta
                    .get("next_key")
                    .map_err(err)?
                    .map(|g| g.value())
                    .unwrap_or(1_000_000);
                meta.insert("next_key", next + 1).map_err(err)?;
                note.key = next.to_string();
                note.created = crate::note::now();
            } else {
                if note.created.is_empty() {
                    note.created = crate::note::now();
                }
                note.edits.push(crate::note::now());
            }
            let mut table = txn.open_table(NOTES).map_err(err)?;
            let bytes = serde_json::to_vec(&note).map_err(err)?;
            table.insert(note.key.as_str(), bytes.as_slice()).map_err(err)?;
        }
        txn.commit().map_err(err)?;
        Ok(note)
    }

    pub fn delete(&self, key: &str) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut table = txn.open_table(NOTES).map_err(err)?;
            if table.remove(key).map_err(err)?.is_none() {
                return Err(format!("there is no note {:?}", key));
            }
        }
        txn.commit().map_err(err)?;
        Ok(())
    }

    /// imports notes (e.g. writs exported from the old server's database) as they are,
    /// keeping their keys, dates and published state
    pub fn import(&self, notes: Vec<Note>) -> StoreResult<usize> {
        let txn = self.db.begin_write().map_err(err)?;
        let n = notes.len();
        {
            let mut meta = txn.open_table(META).map_err(err)?;
            let mut next = meta
                .get("next_key")
                .map_err(err)?
                .map(|g| g.value())
                .unwrap_or(1_000_000);
            let mut table = txn.open_table(NOTES).map_err(err)?;
            for mut note in notes {
                if note.key.is_empty() {
                    note.key = next.to_string();
                    next += 1;
                } else if let Ok(k) = note.key.parse::<u64>() {
                    next = next.max(k + 1);
                }
                if note.created.is_empty() {
                    note.created = crate::note::now();
                }
                let bytes = serde_json::to_vec(&note).map_err(err)?;
                table.insert(note.key.as_str(), bytes.as_slice()).map_err(err)?;
            }
            meta.insert("next_key", next).map_err(err)?;
        }
        txn.commit().map_err(err)?;
        Ok(n)
    }
}

/// newest first by creation time, like the old `SORT writ.created DESC`
pub fn sort_newest_first(notes: &mut [Note]) {
    notes.sort_by_key(|n| std::cmp::Reverse(n.created_at()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "anend-test-{}-{:?}.redb",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        (Store::open(&path).unwrap(), path)
    }

    #[test]
    fn save_update_delete() {
        let (s, path) = temp_store();
        let n = s
            .save(&Note {
                title: "a".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(n.key, "1000000");
        assert!(n.created_at().is_some() && n.edits.is_empty());

        let mut edited = n.clone();
        edited.markdown = "body".into();
        let edited = s.save(&edited).unwrap();
        assert_eq!(edited.key, n.key);
        assert_eq!(edited.edits.len(), 1);

        let second = s
            .save(&Note {
                title: "b".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(second.key, "1000001");
        let all = s.all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].markdown, "", "newest first");
        assert_eq!(all[1].markdown, "body");

        s.delete(&n.key).unwrap();
        assert!(s.delete(&n.key).is_err());
        assert_eq!(s.all().unwrap().len(), 1);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn import_keeps_keys_and_moves_counter() {
        let (s, path) = temp_store();
        let old = Note {
            key: "2000000".into(),
            title: "old".into(),
            created: "2018-11-16T20:04:05+02:00".into(),
            ..Default::default()
        };
        assert_eq!(s.import(vec![old]).unwrap(), 1);
        let fresh = s
            .save(&Note {
                title: "new".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(fresh.key, "2000001");
        assert_eq!(s.all().unwrap()[1].created, "2018-11-16T20:04:05+02:00");
        std::fs::remove_file(path).ok();
    }
}
