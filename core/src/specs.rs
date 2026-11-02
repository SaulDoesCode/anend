//! A save's spec models (docs/models.md): models made from a declarative spec. Each
//! one's record (the spec as written, created and modified times, its history of
//! trainings) lives in the save's own database (table `graph spec models`,
//! `graph::assets`), so it travels with the file, its backups and bundles, and merges.
//! The trained model (`classify::spec::Model::to_bytes`: weights, spec and codebook; a
//! few MB for the engine's own trunks, hundreds for a pretrained one) stays in the save's
//! models file (`<save>.models`, beside its classifiers) in `spec model weights`, keyed
//! by the model's name, with `spec model weights fit` saying which spec shape they were
//! trained for: weights whose shape no longer matches their record are stale and unused.
//! Saves made before this kept their records in the models file's `spec models`;
//! [`migrate`] moves them into the save when its models file opens. The ops over them
//! are `ops::models`.

use redb::{ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction};
use serde::{Deserialize, Serialize};

use crate::classify::metrics::Metrics;
use crate::classify::spec::ModelSpec;
use crate::graph::Graph;
use crate::store::{Store, StoreResult};

/// the records' old home in the models file (read once, by [`migrate`])
pub const SPECS: TableDefinition<&str, &[u8]> = TableDefinition::new("spec models");
pub const SPEC_WEIGHTS: TableDefinition<&str, &[u8]> = TableDefinition::new("spec model weights");
/// name → the [`shape_of`] the weights were trained for
pub const SPEC_FIT: TableDefinition<&str, &str> = TableDefinition::new("spec model weights fit");

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn create_tables(txn: &WriteTransaction) -> StoreResult<()> {
    txn.open_table(SPECS).map_err(err)?;
    txn.open_table(SPEC_WEIGHTS).map_err(err)?;
    txn.open_table(SPEC_FIT).map_err(err)?;
    Ok(())
}

/// one training, as the history keeps it
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Trained {
    pub time: u64,
    pub steps: usize,
    pub epochs: usize,
    pub examples: usize,
    pub val_examples: usize,
    pub seconds: f64,
    pub tokens_per_sec: f64,
    pub stopped_early: bool,
    pub fresh: bool,
    /// the kept weights' validation loss
    pub loss: Option<f64>,
    /// the generate head's held-out bits per byte
    pub bits_per_byte: Option<f64>,
    /// per head with validation examples: its name and metrics
    #[serde(default)]
    pub heads: Vec<(String, Metrics)>,
    /// how many weights the trained model has
    #[serde(default)]
    pub weights: usize,
}

/// a spec model as stored
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub name: String,
    /// the spec's JSON as written (checked when it was stored)
    pub source: String,
    pub created: u64,
    pub modified: u64,
    /// bumped by every change (an edit, a training), so a training that started before
    /// an edit doesn't write over it
    pub version: u64,
    #[serde(default)]
    pub history: Vec<Trained>,
}

impl Record {
    pub fn spec(&self) -> StoreResult<ModelSpec> {
        ModelSpec::parse(&self.source, false).map_err(|e| e.to_string())
    }

    /// whether it has been trained (its weights may still be missing: see
    /// [`has_weights`])
    pub fn trained(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn to_json(&self) -> StoreResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(err)
    }

    pub fn from_json(b: &[u8]) -> StoreResult<Record> {
        serde_json::from_slice(b).map_err(err)
    }

    /// the same record under another name (its spec's `name` too)
    pub fn renamed(&self, name: &str) -> StoreResult<Record> {
        let mut v: serde_json::Value = serde_json::from_str(&self.source).map_err(err)?;
        v["name"] = serde_json::json!(name);
        Ok(Record {
            name: name.to_string(),
            source: serde_json::to_string_pretty(&v).map_err(err)?,
            ..self.clone()
        })
    }
}

/// what an edit does to the trained weights
pub enum Weights {
    Keep,
    Drop,
    Set(Vec<u8>),
}

/// a spec model in a bundle: its record and the trained model in base64
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BundledSpec {
    pub record: Record,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<String>,
}

/// the parts of a spec that shape the weights: when they change, the trained weights
/// no longer fit and are dropped; the rest (data, critics, sampling, constraints,
/// training options, device) can change under trained weights
pub fn shape_of(s: &ModelSpec) -> String {
    serde_json::to_string(&(&s.input, &s.trunk, &s.heads, &s.from, s.seed)).unwrap_or_default()
}

// ---------------------------------------------------------------- records, in the save

pub fn records(g: &Graph) -> StoreResult<Vec<Record>> {
    g.spec_records()?.iter().map(|(_, b)| Record::from_json(b)).collect()
}

pub fn record(g: &Graph, name: &str) -> StoreResult<Option<Record>> {
    g.spec_record(name)?.map(|b| Record::from_json(&b)).transpose()
}

// ---------------------------------------------------------------- weights, in the models file

impl Store {
    /// the trained model of `name`, whatever spec it was trained for
    pub fn spec_weights(&self, name: &str) -> StoreResult<Option<Vec<u8>>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(SPEC_WEIGHTS).map_err(err)?;
        Ok(t.get(name).map_err(err)?.map(|v| v.value().to_vec()))
    }

    /// the shape the weights of `name` were trained for
    pub fn spec_fit(&self, name: &str) -> StoreResult<Option<String>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(SPEC_FIT).map_err(err)?;
        Ok(t.get(name).map_err(err)?.map(|v| v.value().to_string()))
    }

    /// moves, drops or sets the weights of a model (renamed from `old`), `shape` the
    /// shape of the spec they fit
    pub fn put_spec_weights(&self, old: Option<&str>, name: &str, shape: &str, w: Weights) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut wt = txn.open_table(SPEC_WEIGHTS).map_err(err)?;
            let mut ft = txn.open_table(SPEC_FIT).map_err(err)?;
            let mut kept = None;
            if let Some(o) = old.filter(|o| *o != name) {
                kept = wt.remove(o).map_err(err)?.map(|v| v.value().to_vec());
                ft.remove(o).map_err(err)?;
            }
            match w {
                Weights::Keep => {
                    if let Some(b) = kept {
                        wt.insert(name, b.as_slice()).map_err(err)?;
                        ft.insert(name, shape).map_err(err)?;
                    }
                }
                Weights::Drop => {
                    wt.remove(name).map_err(err)?;
                    ft.remove(name).map_err(err)?;
                }
                Weights::Set(b) => {
                    wt.insert(name, b.as_slice()).map_err(err)?;
                    ft.insert(name, shape).map_err(err)?;
                }
            }
        }
        txn.commit().map_err(err)
    }
}

/// whether `r` has weights trained for its spec's current shape
pub fn has_weights(st: &Store, r: &Record) -> bool {
    match (st.spec_fit(&r.name), r.spec()) {
        (Ok(Some(fit)), Ok(spec)) => fit == shape_of(&spec),
        _ => false,
    }
}

/// the weights of `r` when they fit its spec (stale ones are left unused)
pub fn weights_of(st: &Store, r: &Record) -> StoreResult<Option<Vec<u8>>> {
    if has_weights(st, r) {
        st.spec_weights(&r.name)
    } else {
        Ok(None)
    }
}

/// writes a record into the save (under `old` when it is renamed from it) and its
/// weights into the models file
pub fn put(g: &Graph, st: &Store, old: Option<&str>, r: &Record, w: Weights) -> StoreResult<()> {
    let shape = shape_of(&r.spec()?);
    if matches!(w, Weights::Drop) {
        st.put_spec_weights(old, &r.name, &shape, Weights::Drop)?;
        g.put_spec_record(old, &r.name, &r.to_json()?)?;
        return Ok(());
    }
    g.put_spec_record(old, &r.name, &r.to_json()?)?;
    st.put_spec_weights(old, &r.name, &shape, w)
}

pub fn delete(g: &Graph, st: &Store, name: &str) -> StoreResult<()> {
    st.put_spec_weights(None, name, "", Weights::Drop)?;
    if !g.delete_spec_record(name)? {
        return Err(format!("no model {name:?}"));
    }
    Ok(())
}

/// every spec model with its fitting weights, for a bundle
pub fn export(g: &Graph, st: &Store) -> StoreResult<Vec<BundledSpec>> {
    records(g)?
        .into_iter()
        .map(|r| {
            Ok(BundledSpec {
                weights: weights_of(st, &r)?.map(|w| crate::classifiers::base64(&w)),
                record: r,
            })
        })
        .collect()
}

/// puts a bundle's spec models in (replacing ones of the same name); how many
pub fn import(g: &Graph, st: &Store, all: &[BundledSpec]) -> StoreResult<usize> {
    for b in all {
        b.record.spec()?;
        let w = match &b.weights {
            Some(s) => Weights::Set(
                crate::classifiers::unbase64(s)
                    .ok_or_else(|| format!("{:?}: its weights aren't base64", b.record.name))?,
            ),
            None => Weights::Drop,
        };
        put(g, st, None, &b.record, w)?;
    }
    Ok(all.len())
}

/// moves the records a models file still holds (saves from before records lived in the
/// save) into the save, losslessly: each record goes in as it was, its weights stay and
/// are marked as fitting it. A record the save has already under that name is kept when
/// its version is as new or newer; the older one then goes in as "<name> (moved)".
/// How many moved.
pub fn migrate(g: &Graph, st: &Store) -> StoreResult<usize> {
    let old: Vec<(String, Vec<u8>)> = {
        let txn = st.db.begin_read().map_err(err)?;
        let t = txn.open_table(SPECS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (k, v) = item.map_err(err)?;
            out.push((k.value().to_string(), v.value().to_vec()));
        }
        out
    };
    if old.is_empty() {
        return Ok(0);
    }
    let mut moved = 0;
    for (name, bytes) in &old {
        let r = Record::from_json(bytes)?;
        let shape = r.spec().map(|s| shape_of(&s)).unwrap_or_default();
        match record(g, name)? {
            Some(have) if have == r => {}
            Some(have) if have.version >= r.version => {
                // the save's own is newer: the older one is kept beside it, untrained
                let mut n = format!("{name} (moved)");
                let mut i = 2;
                while g.spec_record(&n)?.is_some() {
                    n = format!("{name} (moved {i})");
                    i += 1;
                }
                let r2 = r.renamed(&n).unwrap_or(Record {
                    name: n.clone(),
                    ..r.clone()
                });
                g.put_spec_record(None, &n, &r2.to_json()?)?;
                // its weights (from before fits were kept, so they are its) go with it
                if st.spec_fit(name)?.is_none() && st.spec_weights(name)?.is_some() {
                    st.put_spec_weights(Some(name), &n, &shape, Weights::Keep)?;
                }
            }
            _ => {
                g.put_spec_record(None, name, bytes)?;
                if st.spec_weights(name)?.is_some() && st.spec_fit(name)?.is_none() {
                    let txn = st.db.begin_write().map_err(err)?;
                    txn.open_table(SPEC_FIT)
                        .map_err(err)?
                        .insert(name.as_str(), shape.as_str())
                        .map_err(err)?;
                    txn.commit().map_err(err)?;
                }
            }
        }
        moved += 1;
    }
    // only once they're all in the save do they leave the models file
    let txn = st.db.begin_write().map_err(err)?;
    {
        let mut t = txn.open_table(SPECS).map_err(err)?;
        for (name, _) in &old {
            t.remove(name.as_str()).map_err(err)?;
        }
    }
    txn.commit().map_err(err)?;
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, seed: u64) -> String {
        serde_json::json!({
            "name": name, "input": {"type": "text"}, "trunk": {"preset": "test"}, "seed": seed,
            "heads": [{"name": "next", "kind": "generate"}]
        })
        .to_string()
    }

    fn rec(name: &str, version: u64, seed: u64) -> Record {
        Record {
            name: name.into(),
            source: spec(name, seed),
            created: 5,
            modified: 6 + version,
            version,
            history: vec![Trained {
                steps: 3,
                ..Trained::default()
            }],
        }
    }

    /// a models file from before: records and weights both in it
    fn old_file(st: &Store, rs: &[Record]) {
        let txn = st.db.begin_write().unwrap();
        {
            let mut t = txn.open_table(SPECS).unwrap();
            let mut w = txn.open_table(SPEC_WEIGHTS).unwrap();
            for r in rs {
                t.insert(r.name.as_str(), r.to_json().unwrap().as_slice()).unwrap();
                w.insert(r.name.as_str(), format!("weights of {}", r.name).as_bytes())
                    .unwrap();
            }
        }
        txn.commit().unwrap();
    }

    #[test]
    fn records_move_into_the_save_losslessly() {
        let dir = std::env::temp_dir().join(format!("anend-specs-mig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Graph::open(&dir.join("s.redb")).unwrap();
        let st = Store::open(&dir.join("s.models")).unwrap();
        let (a, b) = (rec("a", 3, 1), rec("b", 1, 1));
        old_file(&st, &[a.clone(), b.clone()]);
        // the save has a newer "b" already
        let newer = rec("b", 4, 2);
        g.put_spec_record(None, "b", &newer.to_json().unwrap()).unwrap();
        assert_eq!(migrate(&g, &st).unwrap(), 2);
        assert_eq!(record(&g, "a").unwrap().unwrap(), a);
        assert_eq!(record(&g, "b").unwrap().unwrap(), newer);
        let moved = record(&g, "b (moved)").unwrap().unwrap();
        assert_eq!((moved.version, moved.history.clone()), (b.version, b.history.clone()));
        assert!(moved.source.contains("\"b (moved)\""));
        assert!(has_weights(&st, &moved) && !has_weights(&st, &newer));
        assert_eq!(weights_of(&st, &moved).unwrap().unwrap(), b"weights of b");
        // a's weights fit it still; the models file keeps no records
        assert!(has_weights(&st, &a));
        assert_eq!(weights_of(&st, &a).unwrap().unwrap(), b"weights of a");
        assert_eq!(migrate(&g, &st).unwrap(), 0);
        // a shape change makes the weights stale
        let mut a2 = a.clone();
        a2.source = spec("a", 9);
        g.put_spec_record(None, "a", &a2.to_json().unwrap()).unwrap();
        assert!(!has_weights(&st, &a2));
        assert!(weights_of(&st, &a2).unwrap().is_none());
        drop((g, st));
        let _ = std::fs::remove_dir_all(dir);
    }
}
