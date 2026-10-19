//! Note storage in an embedded redb database: one table of JSON documents keyed by the
//! note's `_key`, plus a counter for handing out new keys. The same database holds the
//! memories and search tables (see memory.rs); every note change updates those in the
//! same transaction. Every write is a durable transaction.

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::path::Path;

use crate::note::Note;

pub const NOTES: TableDefinition<&str, &[u8]> = TableDefinition::new("notes");
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");

pub type StoreResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// bumped when what gets indexed changes, so existing databases are re-indexed
const INDEX_VERSION: u64 = 1;

pub struct Store {
    pub db: Database,
    path: std::path::PathBuf,
    /// where the classifiers' texts and lists come from, when not these tables
    pub(crate) texts: Option<std::sync::Arc<dyn crate::texts::Texts>>,
}

impl Store {
    pub fn open(path: &Path) -> StoreResult<Store> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            crate::vfs::create_dir_all(dir).map_err(err)?;
        }
        let db = crate::vfs::database(path).map_err(err)?;
        let txn = db.begin_write().map_err(err)?;
        txn.open_table(NOTES).map_err(err)?;
        txn.open_table(META).map_err(err)?;
        crate::memory::create_tables(&txn)?;
        crate::classifiers::create_tables(&txn)?;
        crate::memo::create_tables(&txn)?;
        crate::filters::create_tables(&txn)?;
        crate::specs::create_tables(&txn)?;
        txn.commit().map_err(err)?;
        let store = Store {
            db,
            path: path.to_path_buf(),
            texts: None,
        };
        store.index_existing_notes()?;
        Ok(store)
    }

    /// a store whose classifiers read their texts and lists from `texts` (`texts.rs`)
    pub fn open_over(path: &Path, texts: std::sync::Arc<dyn crate::texts::Texts>) -> StoreResult<Store> {
        let mut s = Store::open(path)?;
        s.texts = Some(texts);
        Ok(s)
    }

    /// how many notes and memories a save holds, read without taking it over (it can't be
    /// the one that's open)
    pub fn summary(path: &Path) -> Option<(u64, u64)> {
        use redb::ReadableTableMetadata;
        // a save the app didn't close cleanly needs redb's repair, which only a writable
        // open does; it's not open anywhere else, so repair it the way loading it would
        let (ro, rw);
        let txn = match redb::ReadOnlyDatabase::open(path) {
            Ok(db) => {
                ro = db;
                ro.begin_read().ok()?
            }
            Err(_) => {
                rw = Database::open(path).ok()?;
                rw.begin_read().ok()?
            }
        };
        let notes = txn.open_table(NOTES).ok().and_then(|t| t.len().ok()).unwrap_or(0);
        let memories = txn
            .open_table(crate::memory::MEMORIES)
            .ok()
            .and_then(|t| t.len().ok())
            .unwrap_or(0);
        Some((notes, memories))
    }

    /// how many notes and memories this one holds
    pub fn counts(&self) -> (u64, u64) {
        use redb::ReadableTableMetadata;
        let Ok(txn) = self.db.begin_read() else { return (0, 0) };
        let notes = txn.open_table(NOTES).ok().and_then(|t| t.len().ok()).unwrap_or(0);
        let memories = txn
            .open_table(crate::memory::MEMORIES)
            .ok()
            .and_then(|t| t.len().ok())
            .unwrap_or(0);
        (notes, memories)
    }

    /// where the database lives
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// indexes notes written before the search tables existed (or by an older version)
    fn index_existing_notes(&self) -> StoreResult<()> {
        let current = {
            let txn = self.db.begin_read().map_err(err)?;
            let meta = txn.open_table(META).map_err(err)?;

            meta.get("index_version").map_err(err)?.map(|g| g.value())
        };
        if current == Some(INDEX_VERSION) {
            return Ok(());
        }
        let notes = self.all()?;
        let txn = self.db.begin_write().map_err(err)?;
        for note in &notes {
            crate::memory::index_note(&txn, note, true)?;
        }
        txn.open_table(META)
            .map_err(err)?
            .insert("index_version", INDEX_VERSION)
            .map_err(err)?;
        txn.commit().map_err(err)
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
            let previous = if note.key.is_empty() {
                None
            } else {
                let table = txn.open_table(NOTES).map_err(err)?;
                let found = table.get(note.key.as_str()).map_err(err)?;
                found.and_then(|g| serde_json::from_slice::<Note>(g.value()).ok())
            };
            if let Some(previous) = &previous {
                crate::memory::index_note(&txn, previous, false)?;
            }
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
        crate::memory::index_note(&txn, &note, true)?;
        txn.commit().map_err(err)?;
        Ok(note)
    }

    pub fn delete(&self, key: &str) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        let removed = {
            let mut table = txn.open_table(NOTES).map_err(err)?;
            let removed = table.remove(key).map_err(err)?.map(|g| g.value().to_vec());
            removed.ok_or_else(|| format!("there is no note {:?}", key))?
        };
        if let Ok(note) = serde_json::from_slice::<Note>(&removed) {
            crate::memory::index_note(&txn, &note, false)?;
        }
        txn.commit().map_err(err)?;
        Ok(())
    }

    /// imports notes (e.g. writs exported from the old server's database) as they are,
    /// keeping their keys and dates
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
                let old = table
                    .insert(note.key.as_str(), bytes.as_slice())
                    .map_err(err)?
                    .map(|g| g.value().to_vec());
                if let Some(old) = old.and_then(|o| serde_json::from_slice::<Note>(&o).ok()) {
                    crate::memory::index_note(&txn, &old, false)?;
                }
                crate::memory::index_note(&txn, &note, true)?;
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
