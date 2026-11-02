//! What a save holds besides its entries: its spec models' definitions and its pictures,
//! each in tables of the save's own database, so they travel with the file (copies,
//! renames, the hub's share and take) and in its backups and bundles.
//!
//! | table | key | holds |
//! |---|---|---|
//! | `graph spec models` | model name | the record as JSON (`crate::specs::Record`: the spec's JSON as written, created, modified, version, trainings) |
//! | `graph pictures` | picture id (content hash, 32 hex digits) | its details as JSON ([`Picture`]) |
//! | `graph picture bytes` | picture id | the file's bytes (PNG or JPEG, as added) |
//! | `graph picture lists` | list name → picture ids | which lists a picture is in |
//!
//! A picture's list is named, like a list of texts (the text of its head): the pictures
//! of list "cats" sit beside the texts of the list whose head is "cats". The trained
//! weights of spec models are large and stay in the save's models file (`crate::specs`).

use redb::{MultimapTableDefinition, ReadableDatabase, ReadableMultimapTable, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use super::store::{GResult, Graph, err};

pub const SPECS: TableDefinition<&str, &[u8]> = TableDefinition::new("graph spec models");
pub const PICTURES: TableDefinition<&str, &[u8]> = TableDefinition::new("graph pictures");
pub const PICTURE_BYTES: TableDefinition<&str, &[u8]> = TableDefinition::new("graph picture bytes");
pub const PICTURE_LISTS: MultimapTableDefinition<&str, &str> = MultimapTableDefinition::new("graph picture lists");

pub fn ensure_tables(txn: &redb::WriteTransaction) -> GResult<()> {
    txn.open_table(SPECS).map_err(err)?;
    txn.open_table(PICTURES).map_err(err)?;
    txn.open_table(PICTURE_BYTES).map_err(err)?;
    txn.open_multimap_table(PICTURE_LISTS).map_err(err)?;
    Ok(())
}

/// a picture's details (its bytes are kept apart)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picture {
    /// the content hash of its bytes ([`picture_id`])
    pub id: String,
    /// `image/png` or `image/jpeg`
    pub mime: String,
    pub width: u32,
    pub height: u32,
    /// what it was called when added (a file name), or what it was named since
    #[serde(default)]
    pub name: String,
    /// when it was added, milliseconds since the Unix epoch
    pub added: u64,
    /// its bytes' length
    pub size: u64,
}

/// a picture's id: FNV-1a over its bytes, 128 bits as 32 hex digits
pub fn picture_id(bytes: &[u8]) -> String {
    let mut h: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for b in bytes {
        h ^= *b as u128;
        h = h.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    format!("{h:032x}")
}

impl Graph {
    // ------------------------------------------------------------ spec models

    /// every spec model record, by name (JSON)
    pub fn spec_records(&self) -> GResult<Vec<(String, Vec<u8>)>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(SPECS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (k, v) = item.map_err(err)?;
            out.push((k.value().to_string(), v.value().to_vec()));
        }
        Ok(out)
    }

    pub fn spec_record(&self, name: &str) -> GResult<Option<Vec<u8>>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(SPECS).map_err(err)?;
        Ok(t.get(name).map_err(err)?.map(|v| v.value().to_vec()))
    }

    /// writes a record under `name`, taking it from `old` when it's renamed
    pub fn put_spec_record(&self, old: Option<&str>, name: &str, json: &[u8]) -> GResult<()> {
        let txn = self.db().begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(SPECS).map_err(err)?;
            if let Some(o) = old.filter(|o| *o != name) {
                t.remove(o).map_err(err)?;
            }
            t.insert(name, json).map_err(err)?;
        }
        txn.commit().map_err(err)
    }

    /// whether there was one
    pub fn delete_spec_record(&self, name: &str) -> GResult<bool> {
        let txn = self.db().begin_write().map_err(err)?;
        let found = txn.open_table(SPECS).map_err(err)?.remove(name).map_err(err)?.is_some();
        txn.commit().map_err(err)?;
        Ok(found)
    }

    // ------------------------------------------------------------ pictures

    /// every picture's details, oldest first
    pub fn pictures(&self) -> GResult<Vec<Picture>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(PICTURES).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (_, v) = item.map_err(err)?;
            out.push(serde_json::from_slice::<Picture>(v.value()).map_err(err)?);
        }
        out.sort_by(|a, b| a.added.cmp(&b.added).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    pub fn picture_count(&self) -> GResult<u64> {
        use redb::ReadableTableMetadata;
        let txn = self.db().begin_read().map_err(err)?;
        txn.open_table(PICTURES).map_err(err)?.len().map_err(err)
    }

    pub fn picture(&self, id: &str) -> GResult<Option<Picture>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(PICTURES).map_err(err)?;
        match t.get(id).map_err(err)? {
            Some(v) => Ok(Some(serde_json::from_slice(v.value()).map_err(err)?)),
            None => Ok(None),
        }
    }

    pub fn picture_bytes(&self, id: &str) -> GResult<Option<Vec<u8>>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_table(PICTURE_BYTES).map_err(err)?;
        Ok(t.get(id).map_err(err)?.map(|v| v.value().to_vec()))
    }

    /// stores a picture (its `id` must be its bytes' [`picture_id`]); false when it was
    /// there already (its details are left as they were)
    pub fn put_picture(&self, p: &Picture, bytes: &[u8]) -> GResult<bool> {
        if p.id != picture_id(bytes) {
            return Err("a picture's id is its bytes' hash".into());
        }
        let txn = self.db().begin_write().map_err(err)?;
        let new = {
            let mut t = txn.open_table(PICTURES).map_err(err)?;
            if t.get(p.id.as_str()).map_err(err)?.is_some() {
                false
            } else {
                t.insert(p.id.as_str(), serde_json::to_vec(p).map_err(err)?.as_slice())
                    .map_err(err)?;
                txn.open_table(PICTURE_BYTES)
                    .map_err(err)?
                    .insert(p.id.as_str(), bytes)
                    .map_err(err)?;
                true
            }
        };
        txn.commit().map_err(err)?;
        Ok(new)
    }

    /// renames a picture
    pub fn name_picture(&self, id: &str, name: &str) -> GResult<()> {
        let mut p = self.picture(id)?.ok_or_else(|| format!("no picture {id:?}"))?;
        p.name = name.to_string();
        let txn = self.db().begin_write().map_err(err)?;
        txn.open_table(PICTURES)
            .map_err(err)?
            .insert(id, serde_json::to_vec(&p).map_err(err)?.as_slice())
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    /// takes a picture out of the save and its lists; whether it was there
    pub fn delete_picture(&self, id: &str) -> GResult<bool> {
        let lists = self.lists_of_picture(id)?;
        let txn = self.db().begin_write().map_err(err)?;
        let found = {
            let found = txn
                .open_table(PICTURES)
                .map_err(err)?
                .remove(id)
                .map_err(err)?
                .is_some();
            txn.open_table(PICTURE_BYTES).map_err(err)?.remove(id).map_err(err)?;
            let mut l = txn.open_multimap_table(PICTURE_LISTS).map_err(err)?;
            for name in &lists {
                l.remove(name.as_str(), id).map_err(err)?;
            }
            found
        };
        txn.commit().map_err(err)?;
        Ok(found)
    }

    /// every (list, picture id) row
    pub fn picture_list_rows(&self) -> GResult<Vec<(String, String)>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_multimap_table(PICTURE_LISTS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (k, vs) = item.map_err(err)?;
            for v in vs {
                out.push((k.value().to_string(), v.map_err(err)?.value().to_string()));
            }
        }
        Ok(out)
    }

    /// the ids of a list's pictures
    pub fn list_pictures(&self, list: &str) -> GResult<Vec<String>> {
        let txn = self.db().begin_read().map_err(err)?;
        let t = txn.open_multimap_table(PICTURE_LISTS).map_err(err)?;
        let mut out = Vec::new();
        for v in t.get(list).map_err(err)? {
            out.push(v.map_err(err)?.value().to_string());
        }
        Ok(out)
    }

    /// the lists a picture is in
    pub fn lists_of_picture(&self, id: &str) -> GResult<Vec<String>> {
        Ok(self
            .picture_list_rows()?
            .into_iter()
            .filter(|(_, p)| p == id)
            .map(|(l, _)| l)
            .collect())
    }

    /// puts a picture into a list (or, with `on` false, takes it out); whether that
    /// changed anything
    pub fn tag_picture(&self, id: &str, list: &str, on: bool) -> GResult<bool> {
        let list = list.trim();
        if list.is_empty() {
            return Err("which list? (its name)".into());
        }
        if on && self.picture(id)?.is_none() {
            return Err(format!("no picture {id:?}"));
        }
        let txn = self.db().begin_write().map_err(err)?;
        let changed = {
            let mut t = txn.open_multimap_table(PICTURE_LISTS).map_err(err)?;
            if on {
                !t.insert(list, id).map_err(err)?
            } else {
                t.remove(list, id).map_err(err)?
            }
        };
        txn.commit().map_err(err)?;
        Ok(changed)
    }
}
