//! A save's spec models (docs/models.md): models made from a declarative spec, kept in
//! the save's models file (`<save>.models`, beside its classifiers) so they travel with
//! it (copies, renames, deletes and bundles). Table `spec models` holds each one's
//! record (the spec as written, its history of trainings), `spec model weights` the
//! trained model (`classify::spec::Model::to_bytes`: weights, spec and codebook).
//! The ops over them are `ops::models`.

use redb::{ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction};
use serde::{Deserialize, Serialize};

use crate::classify::metrics::Metrics;
use crate::classify::spec::ModelSpec;
use crate::store::{Store, StoreResult};

pub const SPECS: TableDefinition<&str, &[u8]> = TableDefinition::new("spec models");
pub const SPEC_WEIGHTS: TableDefinition<&str, &[u8]> = TableDefinition::new("spec model weights");

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn create_tables(txn: &WriteTransaction) -> StoreResult<()> {
    txn.open_table(SPECS).map_err(err)?;
    txn.open_table(SPEC_WEIGHTS).map_err(err)?;
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

    pub fn trained(&self) -> bool {
        !self.history.is_empty()
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

impl Store {
    pub fn spec_models(&self) -> StoreResult<Vec<Record>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(SPECS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (_, v) = item.map_err(err)?;
            out.push(serde_json::from_slice::<Record>(v.value()).map_err(err)?);
        }
        Ok(out)
    }

    pub fn spec_model(&self, name: &str) -> StoreResult<Option<Record>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(SPECS).map_err(err)?;
        match t.get(name).map_err(err)? {
            Some(v) => Ok(Some(serde_json::from_slice(v.value()).map_err(err)?)),
            None => Ok(None),
        }
    }

    pub fn spec_weights(&self, name: &str) -> StoreResult<Option<Vec<u8>>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(SPEC_WEIGHTS).map_err(err)?;
        Ok(t.get(name).map_err(err)?.map(|v| v.value().to_vec()))
    }

    /// writes a record (under `old` when it is renamed from it) and its weights
    pub fn put_spec_model(&self, old: Option<&str>, r: &Record, w: Weights) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(SPECS).map_err(err)?;
            let mut wt = txn.open_table(SPEC_WEIGHTS).map_err(err)?;
            let mut kept = None;
            if let Some(o) = old.filter(|o| *o != r.name) {
                t.remove(o).map_err(err)?;
                kept = wt.remove(o).map_err(err)?.map(|v| v.value().to_vec());
            }
            t.insert(r.name.as_str(), serde_json::to_vec(r).map_err(err)?.as_slice())
                .map_err(err)?;
            match w {
                Weights::Keep => {
                    if let Some(b) = kept {
                        wt.insert(r.name.as_str(), b.as_slice()).map_err(err)?;
                    }
                }
                Weights::Drop => {
                    wt.remove(r.name.as_str()).map_err(err)?;
                }
                Weights::Set(b) => {
                    wt.insert(r.name.as_str(), b.as_slice()).map_err(err)?;
                }
            }
        }
        txn.commit().map_err(err)
    }

    pub fn delete_spec_model(&self, name: &str) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        let found = {
            let mut t = txn.open_table(SPECS).map_err(err)?;
            let found = t.remove(name).map_err(err)?.is_some();
            txn.open_table(SPEC_WEIGHTS).map_err(err)?.remove(name).map_err(err)?;
            found
        };
        if !found {
            return Err(format!("no model {name:?}"));
        }
        txn.commit().map_err(err)
    }

    pub fn export_spec_models(&self) -> StoreResult<Vec<BundledSpec>> {
        self.spec_models()?
            .into_iter()
            .map(|r| {
                Ok(BundledSpec {
                    weights: self.spec_weights(&r.name)?.map(|w| crate::classifiers::base64(&w)),
                    record: r,
                })
            })
            .collect()
    }

    /// puts a bundle's spec models in (replacing ones of the same name); how many
    pub fn import_spec_models(&self, all: &[BundledSpec]) -> StoreResult<usize> {
        for b in all {
            b.record.spec()?;
            let w = match &b.weights {
                Some(s) => Weights::Set(
                    crate::classifiers::unbase64(s)
                        .ok_or_else(|| format!("{:?}: its weights aren't base64", b.record.name))?,
                ),
                None => Weights::Drop,
            };
            self.put_spec_model(None, &b.record, w)?;
        }
        Ok(all.len())
    }
}
