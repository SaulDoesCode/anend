//! Files: one place the app reads, writes, lists and moves them, and opens saves.

use chrono::{DateTime, Local};
use std::io;
use std::path::{Path, PathBuf};

pub struct Meta {
    pub len: u64,
    pub modified: Option<DateTime<Local>>,
}

mod imp {
    use super::*;

    pub fn read(p: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(p)
    }

    pub fn write(p: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(p, bytes)
    }

    pub fn remove(p: &Path) -> io::Result<()> {
        std::fs::remove_file(p)
    }

    pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    pub fn copy(from: &Path, to: &Path) -> io::Result<u64> {
        std::fs::copy(from, to)
    }

    pub fn create_dir_all(p: &Path) -> io::Result<()> {
        std::fs::create_dir_all(p)
    }

    pub fn list(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default()
    }

    pub fn meta(p: &Path) -> Option<Meta> {
        let m = std::fs::metadata(p).ok()?;
        Some(Meta {
            len: m.len(),
            modified: m.modified().ok().map(DateTime::<Local>::from),
        })
    }

    pub fn same(a: &Path, b: &Path) -> bool {
        a == b || std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok()
    }
}

pub use imp::*;

/// opens (or makes) a redb database at a path, on a desktop or in the browser
pub fn database(path: &Path) -> Result<redb::Database, redb::DatabaseError> {
    redb::Database::create(path)
}

/// hands a file to the person: into Downloads on a desktop, through the browser's own
/// download in the page; returns where it went
pub fn save_download(name: &str, bytes: &[u8]) -> Result<String, String> {
    {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        let dir = match home {
            Some(h) if h.join("Downloads").is_dir() => h.join("Downloads"),
            Some(h) => h,
            None => PathBuf::from("."),
        };
        let path = dir.join(name);
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(path.display().to_string())
    }
}
