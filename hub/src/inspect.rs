//! Looking inside an uploaded save: it has to be an entry graph the app made (a redb
//! file with the graph's tables and format, docs/format.md), and its counts, some
//! memories with their sums and the note titles go on its page. The hub opens its own
//! copy of the upload, and a file that makes redb fail (or panic) is turned down rather
//! than taking the hub with it.

use anend_core::graph::query::{Page, Query};
use anend_core::graph::record::mime;
use anend_core::graph::{Graph, Reader};
use std::path::Path;

use crate::store::Sample;

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
    let bad = || "that file isn't an anend save (an entry graph .redb file from the app)".to_string();
    // a save copied while the app has it open (the usual case) needs redb's repair, which a
    // read only open won't do; the hub only ever reads its own copy, so it may write to it
    if !Graph::is_save(path) {
        return Err(bad());
    }
    let db = redb::Database::open(path).map_err(|_| bad())?;
    let g = Graph::from_db(db)?;
    let v = g.view()?;
    let (notes, memories) = anend_core::ops::counts(&v)?;
    let mut inside = Inside {
        notes,
        memories,
        ..Default::default()
    };
    let texts = v.query(
        &Query {
            mime: Some(mime::TEXT),
            ..Query::default()
        },
        Page::all(),
    )?;
    // spread the sample over the whole save rather than its first keys
    let step = (texts.ids.len() / SAMPLE).max(1);
    for id in texts.ids.iter().step_by(step).take(SAMPLE) {
        let text: String = v.render(*id).unwrap_or_default().chars().take(120).collect();
        let (ordinal, reversed) = sums(&text);
        inside.sample.push(Sample {
            text,
            ordinal,
            reversed,
        });
    }
    let titled = v.query(
        &Query {
            mime: Some(mime::MARKDOWN),
            ..Query::default()
        },
        Page::first(TITLES),
    )?;
    for id in titled.ids {
        let title = anend_core::note::split(&v.render(id).unwrap_or_default()).0;
        let title = if title.trim().is_empty() {
            "untitled".to_string()
        } else {
            title
        };
        inside.titles.push(title.chars().take(100).collect());
    }
    Ok(inside)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// a save like the app's: a note "Hello" and these memories
    pub fn make_save(path: &Path, memories: &[&str]) {
        let g = Graph::open(path).unwrap();
        g.write(|tx| {
            for t in memories {
                tx.save_text(t, mime::TEXT)?;
            }
            tx.save_text("# Hello\n\nx", mime::MARKDOWN)?;
            Ok(())
        })
        .unwrap();
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
        // a redb file without the graph's tables (an old save, say)
        let other = dir.join("other.redb");
        redb::Database::create(&other).unwrap();
        assert!(inspect(&other).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reads_a_save_copied_while_open() {
        let dir = std::env::temp_dir().join(format!("anend-inspect-open-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("open.redb");
        let g = Graph::open(&p).unwrap();
        g.save_text("listen", mime::TEXT).unwrap();
        // the app still has it open, the way it does when it shares its save
        let copy = dir.join("copy.redb");
        std::fs::copy(&p, &copy).unwrap();
        let i = inspect(&copy).unwrap();
        assert_eq!(i.memories, 1);
        drop(g);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sums_match_the_app() {
        assert_eq!(sums("truth"), (87, 48));
        assert_eq!(sums("A-z!"), (27, 27));
        assert_eq!(sums(""), (0, 0));
    }
}
