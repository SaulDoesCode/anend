//! Looking inside an uploaded save: it has to be a redb file the app made (it has the
//! app's `notes` and `memories` tables), and its counts, some memories with their sums and
//! the note titles go on its page. The hub opens its own copy of the upload, and a file
//! that makes redb fail (or panic) is turned down rather than taking the hub with it.

use redb::{Database, ReadOnlyDatabase, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::path::Path;

use crate::store::Sample;

/// the app's tables (src/store.rs and src/memory.rs in the app)
const NOTES: TableDefinition<&str, &[u8]> = TableDefinition::new("notes");
const MEMORIES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("memories");

/// how many memories and titles a save's page shows
const SAMPLE: usize = 24;
const TITLES: usize = 40;

/// (the web app only checks a file is a save; the hub keeps what it found)
#[cfg_attr(not(feature = "hub"), allow(dead_code))]
#[derive(Debug, Default)]
pub struct Inside {
    pub notes: u64,
    pub memories: u64,
    pub sample: Vec<Sample>,
    pub titles: Vec<String>,
}

/// the ordinal and reversed sums of a text's English letters, as the app's space has them
/// (a=1..z=26, and 27 minus that)
pub fn sums(t: &str) -> (u64, u64) {
    let (mut s, mut n) = (0u64, 0u64);
    for c in t.bytes() {
        let c = c | 32;
        if c.is_ascii_lowercase() {
            s += (c - 96) as u64;
            n += 1;
        }
    }
    (s, 27 * n - s)
}

pub fn inspect(path: &Path) -> Result<Inside, String> {
    let path = path.to_path_buf();
    match std::panic::catch_unwind(move || read(&path)) {
        Ok(r) => r,
        Err(_) => Err("that file isn't a save the app can read".into()),
    }
}

fn read(path: &Path) -> Result<Inside, String> {
    let bad = || "that file isn't an anend save (a .redb file from the app)".to_string();
    // a save copied while the app has it open (the usual case) needs redb's repair, which a
    // read only open won't do; the hub only ever reads its own copy, so it may write to it
    // both kept alive while the transaction reads
    let (ro, rw);
    let txn = match ReadOnlyDatabase::open(path) {
        Ok(db) => {
            ro = db;
            ro.begin_read().map_err(|e| e.to_string())?
        }
        Err(_) => {
            rw = Database::open(path).map_err(|_| bad())?;
            rw.begin_read().map_err(|e| e.to_string())?
        }
    };
    let notes = txn.open_table(NOTES).map_err(|_| bad())?;
    let memories = txn.open_table(MEMORIES).map_err(|_| bad())?;
    let mut inside = Inside {
        notes: notes.len().map_err(|e| e.to_string())?,
        memories: memories.len().map_err(|e| e.to_string())?,
        ..Default::default()
    };
    // spread the sample over the whole save rather than its first keys
    let step = (inside.memories as usize / SAMPLE).max(1);
    for (i, row) in memories.iter().map_err(|e| e.to_string())?.enumerate() {
        if inside.sample.len() >= SAMPLE {
            break;
        }
        if i % step != 0 {
            continue;
        }
        let (_, v) = row.map_err(|e| e.to_string())?;
        let text: String = String::from_utf8_lossy(v.value()).chars().take(120).collect();
        let (ordinal, reversed) = sums(&text);
        inside.sample.push(Sample {
            text,
            ordinal,
            reversed,
        });
    }
    for row in notes.iter().map_err(|e| e.to_string())?.take(TITLES) {
        let (_, v) = row.map_err(|e| e.to_string())?;
        let title = serde_json::from_slice::<serde_json::Value>(v.value())
            .ok()
            .and_then(|n| n.get("title").and_then(|t| t.as_str()).map(str::to_string))
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "untitled".into());
        inside.titles.push(title.chars().take(100).collect());
    }
    Ok(inside)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// a save like the app's, with a few notes and memories
    pub fn make_save(path: &Path, memories: &[&str]) {
        let db = Database::create(path).unwrap();
        let txn = db.begin_write().unwrap();
        {
            let mut n = txn.open_table(NOTES).unwrap();
            n.insert(
                "1000001",
                br#"{"_key":"1000001","title":"Hello","markdown":"x"}"#.as_slice(),
            )
            .unwrap();
            let mut m = txn.open_table(MEMORIES).unwrap();
            for (i, t) in memories.iter().enumerate() {
                m.insert([0u8, i as u8].as_slice(), t.as_bytes()).unwrap();
            }
        }
        txn.commit().unwrap();
    }

    #[test]
    fn reads_a_save() {
        let dir = std::env::temp_dir().join(format!("anend-inspect-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.redb");
        make_save(&p, &["truth", "listen", "silent"]);
        let i = inspect(&p).unwrap();
        assert_eq!((i.notes, i.memories), (1, 3));
        assert_eq!(i.titles, vec!["Hello"]);
        assert_eq!(i.sample[0].text, "truth");
        assert_eq!((i.sample[0].ordinal, i.sample[0].reversed), (87, 48));

        let junk = dir.join("junk.redb");
        std::fs::write(&junk, b"not a database at all, just words").unwrap();
        assert!(inspect(&junk).is_err());
        // a redb file without the app's tables
        let other = dir.join("other.redb");
        Database::create(&other).unwrap();
        assert!(inspect(&other).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reads_a_save_copied_while_open() {
        let dir = std::env::temp_dir().join(format!("anend-inspect-open-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("open.redb");
        let db = Database::create(&p).unwrap();
        let txn = db.begin_write().unwrap();
        txn.open_table(NOTES).unwrap();
        txn.open_table(MEMORIES)
            .unwrap()
            .insert([1u8].as_slice(), b"listen".as_slice())
            .unwrap();
        txn.commit().unwrap();
        // the app still has it open, the way it does when it shares its save
        let copy = dir.join("copy.redb");
        std::fs::copy(&p, &copy).unwrap();
        assert!(
            ReadOnlyDatabase::open(&copy).is_err(),
            "a copy of an open save needs repair"
        );
        let i = inspect(&copy).unwrap();
        assert_eq!(i.memories, 1);
        drop(db);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sums_match_the_app() {
        assert_eq!(sums("truth"), (87, 48));
        assert_eq!(sums("A-z!"), (27, 27));
        assert_eq!(sums(""), (0, 0));
    }
}
