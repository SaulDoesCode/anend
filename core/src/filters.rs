//! Filters over classifiers: include and exclude texts by what classifiers say of them,
//! nested with and / or / not, beside the word, sum, list, tag and kind conditions the
//! searches already have. They run over the memoized classifier buckets (`memo`), so a
//! filter run twice reads no network the second time. Filters are kept in the save as
//! presets (materialized views, their own list system), can refer to each other, and
//! their results can be written out as lists.
//!
//! **Validators:** a classifier can be attached as a validator of another. Its verdict
//! gates the other's outputs (a text the validator doesn't pass counts in none of the
//! other's classes) or weighs them (the other's probability times the validator's), in
//! filters and in search; `Store::disagreements` lists the texts where the two part ways,
//! to be told yes or no.
//!
//! | table | maps |
//! |---|---|
//! | `filters` | name -> `Preset` JSON |
//! | `classifier validators` | classifier -> `[Validator]` JSON |
//!
//! An expression, as JSON:
//!
//! ```json
//! { "op": "and", "of": [
//!     { "op": "class", "classifier": "kind", "class": "fruit", "min": 0.6 },
//!     { "op": "not", "of": { "op": "class", "classifier": "mood", "class": "sad" } },
//!     { "op": "or", "of": [ { "op": "words", "q": "plum" }, { "op": "sum", "bucket": "ordinal", "from": 50, "to": 90 } ] } ] }
//! ```

use redb::{ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::classifiers::Stack;
use crate::classify::Pool;
use crate::memo::Row;
use crate::memory::Ref;
use crate::store::{Store, StoreResult};

pub const FILTERS: TableDefinition<&str, &[u8]> = TableDefinition::new("filters");
pub const VALIDATORS: TableDefinition<&str, &[u8]> = TableDefinition::new("classifier validators");

/// how deep presets may refer to presets, and how many conditions an expression holds
pub const MAX_DEPTH: usize = 12;
pub const MAX_NODES: usize = 256;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn create_tables(txn: &WriteTransaction) -> StoreResult<()> {
    txn.open_table(FILTERS).map_err(err)?;
    txn.open_table(VALIDATORS).map_err(err)?;
    Ok(())
}

// ---------------------------------------------------------------- expressions

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Expr {
    /// every one of them
    And { of: Vec<Expr> },
    /// any one of them
    Or { of: Vec<Expr> },
    Not { of: Box<Expr> },
    /// a classifier's (validated) probability for a class, at least `min` (the
    /// classifier's steered threshold if left out) and at most `max`; `top`: the class
    /// must be its top one too
    Class {
        classifier: String,
        class: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f32>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        top: bool,
    },
    /// the words search (with weum's variations)
    Words { q: String },
    /// a gematria sum in a range: `bucket` ordinal, reversed or atbash
    Sum {
        bucket: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<i64>,
    },
    /// a member of a list (by its head's key)
    List { list: String },
    /// notes with these tags (all of them, or any)
    Tags {
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        any: bool,
    },
    /// memories or notes only
    Kind { kind: String },
    /// another saved filter
    Preset { name: String },
}

impl Expr {
    /// the classifiers it asks (presets left unexpanded)
    pub fn classifiers(&self, out: &mut Vec<String>) {
        match self {
            Expr::And { of } | Expr::Or { of } => of.iter().for_each(|e| e.classifiers(out)),
            Expr::Not { of } => of.classifiers(out),
            Expr::Class { classifier, .. } if !out.contains(classifier) => out.push(classifier.clone()),
            _ => {}
        }
    }

    fn count(&self) -> usize {
        match self {
            Expr::And { of } | Expr::Or { of } => 1 + of.iter().map(Expr::count).sum::<usize>(),
            Expr::Not { of } => 1 + of.count(),
            _ => 1,
        }
    }

    /// a short reading of it, for lists of presets
    pub fn describe(&self) -> String {
        let join = |of: &[Expr], w: &str| {
            let parts: Vec<String> = of.iter().map(|e| e.describe()).collect();
            format!("({})", parts.join(&format!(" {w} ")))
        };
        match self {
            Expr::And { of } => join(of, "and"),
            Expr::Or { of } => join(of, "or"),
            Expr::Not { of } => format!("not {}", of.describe()),
            Expr::Class { classifier, class, min, max, top } => {
                let mut s = format!("{classifier}: {class}");
                if *top {
                    s.push_str(" (top)");
                }
                if let Some(m) = min {
                    s.push_str(&format!(" ≥ {:.0}%", m * 100.0));
                }
                if let Some(m) = max {
                    s.push_str(&format!(" ≤ {:.0}%", m * 100.0));
                }
                s
            }
            Expr::Words { q } => format!("“{q}”"),
            Expr::Sum { bucket, from, to } => format!(
                "{bucket} {}–{}",
                from.map(|x| x.to_string()).unwrap_or_default(),
                to.map(|x| x.to_string()).unwrap_or_default()
            ),
            Expr::List { list } => format!("in list {list}"),
            Expr::Tags { tags, any } => format!(
                "{} #{}",
                if *any { "any of" } else { "tags" },
                tags.join(" #")
            ),
            Expr::Kind { kind } => kind.clone(),
            Expr::Preset { name } => format!("[{name}]"),
        }
    }
}

/// a saved filter
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub expr: Expr,
    #[serde(default)]
    pub description: String,
    pub created: String,
    pub modified: String,
    /// the list its results were last written to (a label)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    /// how many texts it found the last time it ran
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
}

pub fn clean_name(name: &str) -> StoreResult<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 80 || n.chars().any(char::is_control) {
        return Err("a filter's name is 1 to 80 letters".into());
    }
    Ok(n.to_string())
}

// ---------------------------------------------------------------- validators

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// a text the validator doesn't pass is in none of the classes
    #[default]
    Gate,
    /// the probabilities are multiplied by the validator's
    Weigh,
}

/// a classifier that checks another: it passes a text when its `class` is at least `min`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Validator {
    pub by: String,
    pub class: String,
    #[serde(default = "half")]
    pub min: f32,
    #[serde(default)]
    pub mode: Mode,
}

fn half() -> f32 {
    0.5
}

/// a classifier's verdict on a text with its validators applied
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checked {
    /// the probabilities after gating or weighing
    pub probs: Vec<f32>,
    /// every validator passed it
    pub valid: bool,
    /// each validator's probability for its class, in order
    pub checks: Vec<f32>,
}

/// applies validators' rows to a row (`checks[i]` is validator i's row, if read)
pub fn check(row: &Row, vals: &[(Validator, usize)], checks: &[&Row]) -> Checked {
    let mut probs = row.probs.clone();
    let mut valid = true;
    let mut seen = Vec::new();
    for ((v, c), r) in vals.iter().zip(checks) {
        let p = r.probs.get(*c).copied().unwrap_or(0.0);
        seen.push(p);
        match v.mode {
            Mode::Gate => {
                if p < v.min {
                    valid = false;
                    probs.iter_mut().for_each(|x| *x = 0.0);
                }
            }
            Mode::Weigh => {
                if p < v.min {
                    valid = false;
                }
                probs.iter_mut().for_each(|x| *x *= p);
            }
        }
    }
    Checked { probs, valid, checks: seen }
}

impl Store {
    pub fn validators(&self, name: &str) -> StoreResult<Vec<Validator>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(VALIDATORS).map_err(err)?;
        Ok(t.get(name)
            .map_err(err)?
            .and_then(|g| serde_json::from_slice(g.value()).ok())
            .unwrap_or_default())
    }

    /// sets the validators of a classifier (checked: they exist, aren't it, have the class)
    pub fn set_validators(&self, name: &str, vals: &[Validator]) -> StoreResult<Vec<Validator>> {
        self.classifier(name)?;
        if vals.len() > 8 {
            return Err("at most 8 validators".into());
        }
        for v in vals {
            if v.by == name {
                return Err("a classifier can't validate itself".into());
            }
            let d = self.classifier(&v.by)?;
            d.class_index(&v.class)?;
            if !(0.0..=1.0).contains(&v.min) {
                return Err("a validator's min is 0 to 1".into());
            }
            // the validator's own validators would make a loop when it is checked by them
            if self.validators(&v.by)?.iter().any(|x| x.by == name) {
                return Err(format!("{:?} is validated by {name:?} already", v.by));
            }
        }
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(VALIDATORS).map_err(err)?;
            if vals.is_empty() {
                t.remove(name).map_err(err)?;
            } else {
                t.insert(name, serde_json::to_vec(vals).map_err(err)?.as_slice()).map_err(err)?;
            }
        }
        txn.commit().map_err(err)?;
        Ok(vals.to_vec())
    }

    /// takes a deleted classifier out of every validator list (and its own)
    pub fn drop_validators_of(&self, name: &str) -> StoreResult<()> {
        let all: Vec<(String, Vec<Validator>)> = {
            let txn = self.db.begin_read().map_err(err)?;
            let t = txn.open_table(VALIDATORS).map_err(err)?;
            let mut v = Vec::new();
            for item in t.iter().map_err(err)? {
                let (k, val) = item.map_err(err)?;
                v.push((k.value().to_string(), serde_json::from_slice(val.value()).unwrap_or_default()));
            }
            v
        };
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(VALIDATORS).map_err(err)?;
            for (k, vals) in all {
                let keep: Vec<Validator> = vals.into_iter().filter(|v| v.by != name).collect();
                if k == name || keep.is_empty() {
                    t.remove(k.as_str()).map_err(err)?;
                } else {
                    t.insert(k.as_str(), serde_json::to_vec(&keep).map_err(err)?.as_slice()).map_err(err)?;
                }
            }
        }
        txn.commit().map_err(err)
    }

    /// a classifier's rows on texts with its validators applied
    pub fn checked_rows(&self, stack: &Stack, items: &[(Ref, &str)], pool: &Pool) -> StoreResult<Vec<(Row, Checked)>> {
        let rows = self.memo_rows(stack, items, pool)?;
        let vals = self.validators(&stack.def().name)?;
        let mut resolved: Vec<(Validator, usize)> = Vec::new();
        let mut vrows: Vec<Vec<Row>> = Vec::new();
        for v in vals {
            let vs = self.stack(&v.by)?;
            if !vs.def().trained() {
                continue;
            }
            let c = vs.def().class_index(&v.class)?;
            vrows.push(self.memo_rows(&vs, items, pool)?);
            resolved.push((v, c));
        }
        Ok(rows
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                let cs: Vec<&Row> = vrows.iter().map(|v| &v[i]).collect();
                let c = check(&r, &resolved, &cs);
                (r, c)
            })
            .collect())
    }

    /// texts where a classifier counts a text in a class but its validators don't pass
    /// it (or, `missed`, where only the validation keeps it out of none): (ref, shown, the
    /// classifier's class, its probability, each validator's probability)
    pub fn disagreements(&self, name: &str, limit: usize, scan: usize, pool: &Pool) -> StoreResult<Vec<Disagreement>> {
        let stack = self.stack(name)?;
        if !stack.def().trained() {
            return Err(format!("train {name:?} first"));
        }
        let vals = self.validators(name)?;
        if vals.is_empty() {
            return Ok(vec![]);
        }
        let cands = self.candidates(scan)?;
        let items: Vec<(Ref, &str)> = cands.iter().map(|(r, t, _)| (r.clone(), t.as_str())).collect();
        let checked = self.checked_rows(&stack, &items, pool)?;
        let classes = stack.def().class_names();
        let mut out: Vec<Disagreement> = cands
            .iter()
            .zip(checked)
            .filter(|(_, (r, c))| !r.members.is_empty() && !c.valid)
            .map(|((r, _, shown), (row, c))| Disagreement {
                key: r.label(),
                text: shown.clone(),
                class: classes.get(row.top).cloned().unwrap_or_default(),
                prob: row.confidence(),
                checks: vals.iter().map(|v| v.by.clone()).zip(c.checks).collect(),
            })
            .collect();
        out.sort_by(|a, b| b.prob.total_cmp(&a.prob).then_with(|| a.key.cmp(&b.key)));
        out.truncate(limit);
        Ok(out)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Disagreement {
    pub key: String,
    pub text: String,
    pub class: String,
    pub prob: f32,
    /// (validator, its probability for the class it checks)
    pub checks: Vec<(String, f32)>,
}

// ---------------------------------------------------------------- presets

impl Store {
    pub fn filter_presets(&self) -> StoreResult<Vec<Preset>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(FILTERS).map_err(err)?;
        let mut out = Vec::new();
        for item in t.iter().map_err(err)? {
            let (_, v) = item.map_err(err)?;
            out.push(serde_json::from_slice(v.value()).map_err(err)?);
        }
        Ok(out)
    }

    pub fn filter_preset(&self, name: &str) -> StoreResult<Preset> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(FILTERS).map_err(err)?;
        let found = t.get(name).map_err(err)?;
        let g = found.ok_or_else(|| format!("there's no filter called {name:?}"))?;
        serde_json::from_slice(g.value()).map_err(err)
    }

    fn put_preset(&self, p: &Preset) -> StoreResult<()> {
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_table(FILTERS)
            .map_err(err)?
            .insert(p.name.as_str(), serde_json::to_vec(p).map_err(err)?.as_slice())
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    /// saves a filter (new, or replacing one by that name), after checking it reads
    pub fn save_filter(&self, name: &str, expr: Expr, description: &str) -> StoreResult<Preset> {
        let name = clean_name(name)?;
        if description.chars().count() > 2000 {
            return Err("a description is at most 2000 characters".into());
        }
        self.check_expr(&expr, &mut vec![name.clone()])?;
        let now = crate::note::now();
        let old = self.filter_preset(&name).ok();
        let p = Preset {
            name: name.clone(),
            expr,
            description: description.trim().to_string(),
            created: old.as_ref().map(|o| o.created.clone()).unwrap_or_else(|| now.clone()),
            modified: now,
            list: old.as_ref().and_then(|o| o.list.clone()),
            count: None,
        };
        self.put_preset(&p)?;
        Ok(p)
    }

    pub fn delete_filter(&self, name: &str) -> StoreResult<()> {
        self.filter_preset(name)?;
        for p in self.filter_presets()? {
            if p.name != name && refers(&p.expr, name) {
                return Err(format!("the filter {:?} uses {name:?}; change it first", p.name));
            }
        }
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_table(FILTERS).map_err(err)?.remove(name).map_err(err)?;
        txn.commit().map_err(err)
    }

    /// that an expression reads: classifiers and classes exist, presets don't loop,
    /// numbers are in range, and it isn't too big
    pub fn check_expr(&self, e: &Expr, path: &mut Vec<String>) -> StoreResult<()> {
        self.check_inner(e, path, &mut HashMap::new()).map(|_| ())
    }

    /// `check_expr`, remembering how many levels of presets each checked preset holds, so
    /// a preset referred to many times is read once; answers this expression's levels
    fn check_inner(&self, e: &Expr, path: &mut Vec<String>, done: &mut HashMap<String, usize>) -> StoreResult<usize> {
        if e.count() > MAX_NODES {
            return Err(format!("a filter holds at most {MAX_NODES} conditions"));
        }
        if path.len() > MAX_DEPTH {
            return Err("filters refer to filters too deeply".into());
        }
        match e {
            Expr::And { of } | Expr::Or { of } => {
                // an empty "all of" holds for everything (a filter not yet narrowed); an
                // empty "any of" could only ever match nothing
                if of.is_empty() && matches!(e, Expr::Or { .. }) {
                    return Err("an \"any of\" needs something in it".into());
                }
                let mut h = 0;
                for x in of {
                    h = h.max(self.check_inner(x, path, done)?);
                }
                return Ok(h);
            }
            Expr::Not { of } => return self.check_inner(of, path, done),
            Expr::Class { classifier, class, min, max, .. } => {
                let d = self.classifier(classifier)?;
                d.class_index(class)?;
                for v in [min, max].into_iter().flatten() {
                    if !(0.0..=1.0).contains(v) {
                        return Err("probabilities are 0 to 1".into());
                    }
                }
            }
            Expr::Words { q } => {
                if q.trim().is_empty() || q.len() > 400 {
                    return Err("words: 1 to 400 characters".into());
                }
            }
            Expr::Sum { bucket, .. } => {
                if !["ordinal", "reversed", "atbash"].contains(&bucket.as_str()) {
                    return Err("a sum is ordinal, reversed or atbash".into());
                }
            }
            Expr::List { list } => {
                Ref::parse(list).ok_or_else(|| format!("{list:?} isn't a list's key"))?;
            }
            Expr::Tags { tags, .. } => {
                if tags.is_empty() || tags.len() > 32 {
                    return Err("1 to 32 tags".into());
                }
            }
            Expr::Kind { kind } => {
                if kind != "memories" && kind != "notes" {
                    return Err("a kind is memories or notes".into());
                }
            }
            Expr::Preset { name } => {
                if path.contains(name) {
                    return Err(format!("the filter {name:?} would include itself"));
                }
                let inner = match done.get(name) {
                    Some(&h) => h,
                    None => {
                        let p = self.filter_preset(name)?;
                        path.push(name.clone());
                        let h = self.check_inner(&p.expr, path, done);
                        path.pop();
                        let h = h?;
                        done.insert(name.clone(), h);
                        h
                    }
                };
                if path.len() + 1 + inner > MAX_DEPTH {
                    return Err("filters refer to filters too deeply".into());
                }
                return Ok(inner + 1);
            }
        }
        Ok(0)
    }
}

fn refers(e: &Expr, name: &str) -> bool {
    match e {
        Expr::And { of } | Expr::Or { of } => of.iter().any(|x| refers(x, name)),
        Expr::Not { of } => refers(of, name),
        Expr::Preset { name: n } => n == name,
        _ => false,
    }
}

// ---------------------------------------------------------------- running

/// what a filter found
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub key: String,
    pub text: String,
    pub note: bool,
    /// how strongly its classifier conditions hold (the weakest of them; 1 without any)
    pub score: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Results {
    pub scanned: usize,
    pub matched: usize,
    pub results: Vec<Found>,
    /// the classifiers it read, and those it skipped because they aren't trained
    pub classifiers: Vec<String>,
    pub untrained: Vec<String>,
}

/// a classifier read over a run's texts: its classes, threshold, and checked rows
type Read = (Vec<String>, f32, Vec<(Row, Checked)>);

/// the texts a run looks through, and what's worked out about them once
struct Universe<'a> {
    store: &'a Store,
    pool: &'a Pool,
    refs: Vec<Ref>,
    texts: Vec<String>,
    shown: Vec<String>,
    /// classifier -> (classes, threshold, checked rows) over the universe
    checked: HashMap<String, Option<Read>>,
    notes: Option<HashMap<String, Vec<String>>>,
    /// each preset's answer, worked out once however often it's referred to (a preset
    /// using another 256 times, twelve deep, would otherwise never finish)
    presets: HashMap<String, (Vec<bool>, Vec<f32>)>,
    /// the presets being worked out, to catch one that comes back to itself
    open: Vec<String>,
}

impl<'a> Universe<'a> {
    fn classifier(&mut self, name: &str) -> StoreResult<Option<&Read>> {
        if !self.checked.contains_key(name) {
            let stack = self.store.stack(name)?;
            let v = if stack.def().trained() {
                let items: Vec<(Ref, &str)> = self.refs.iter().cloned().zip(self.texts.iter().map(String::as_str)).collect();
                let rows = self.store.checked_rows(&stack, &items, self.pool)?;
                Some((stack.def().class_names(), stack.def().steer.heads[0].threshold, rows))
            } else {
                None
            };
            self.checked.insert(name.to_string(), v);
        }
        Ok(self.checked[name].as_ref())
    }

    /// which texts match, and the score of each (the weakest positive class condition)
    fn eval(&mut self, e: &Expr, depth: usize) -> StoreResult<(Vec<bool>, Vec<f32>)> {
        let n = self.refs.len();
        if depth > MAX_DEPTH {
            return Err("filters refer to filters too deeply".into());
        }
        Ok(match e {
            Expr::And { of } => {
                let mut m = vec![true; n];
                let mut s = vec![1.0f32; n];
                for x in of {
                    let (mx, sx) = self.eval(x, depth)?;
                    for i in 0..n {
                        m[i] &= mx[i];
                        s[i] = s[i].min(sx[i]);
                    }
                }
                (m, s)
            }
            Expr::Or { of } => {
                let mut m = vec![false; n];
                let mut s = vec![0.0f32; n];
                for x in of {
                    let (mx, sx) = self.eval(x, depth)?;
                    for i in 0..n {
                        if mx[i] {
                            m[i] = true;
                            s[i] = s[i].max(sx[i]);
                        }
                    }
                }
                (m, s)
            }
            Expr::Not { of } => {
                let (m, _) = self.eval(of, depth)?;
                (m.into_iter().map(|x| !x).collect(), vec![1.0; n])
            }
            Expr::Class { classifier, class, min, max, top } => {
                let Some((classes, threshold, rows)) = self.classifier(classifier)? else {
                    return Ok((vec![false; n], vec![0.0; n]));
                };
                let c = classes
                    .iter()
                    .position(|x| x == class)
                    .ok_or_else(|| format!("{classifier} has no class {class:?}"))?;
                let lo = min.unwrap_or(*threshold);
                let hi = max.unwrap_or(1.0);
                let mut m = vec![false; n];
                let mut s = vec![0.0; n];
                for (i, (row, ch)) in rows.iter().enumerate() {
                    let p = ch.probs.get(c).copied().unwrap_or(0.0);
                    m[i] = p >= lo && p <= hi && (!*top || row.top == c);
                    s[i] = p;
                }
                (m, s)
            }
            Expr::Words { q } => {
                let found: HashSet<Ref> = self.store.search_with_variations(q)?.into_iter().map(|h| h.r).collect();
                (self.refs.iter().map(|r| found.contains(r)).collect(), vec![1.0; n])
            }
            Expr::Sum { bucket, from, to } => {
                let m = self
                    .texts
                    .iter()
                    .map(|t| {
                        let ms = crate::memory::measure(t);
                        let v = match bucket.as_str() {
                            "reversed" => ms.reversed,
                            "atbash" => ms.atbash,
                            _ => ms.ordinal,
                        } as i64;
                        from.is_none_or(|f| v >= f) && to.is_none_or(|t| v <= t)
                    })
                    .collect();
                (m, vec![1.0; n])
            }
            Expr::List { list } => {
                let head = Ref::parse(list).ok_or_else(|| format!("{list:?} isn't a list's key"))?;
                let members: HashSet<Ref> = self.store.list_members(&head)?.into_iter().collect();
                (self.refs.iter().map(|r| members.contains(r)).collect(), vec![1.0; n])
            }
            Expr::Tags { tags, any } => {
                if self.notes.is_none() {
                    self.notes = Some(self.store.all()?.into_iter().map(|x| (x.key, x.tags)).collect());
                }
                let notes = self.notes.as_ref().expect("read");
                let want: Vec<String> = tags.iter().filter_map(|t| crate::note::normalize_tag(t)).collect();
                let m = self
                    .refs
                    .iter()
                    .map(|r| match r {
                        Ref::Note(k) => notes.get(k).is_some_and(|have| {
                            let has = |t: &String| have.iter().any(|h| crate::note::normalize_tag(h).as_ref() == Some(t));
                            if *any { want.iter().any(has) } else { want.iter().all(has) }
                        }),
                        Ref::Memory(_) => false,
                    })
                    .collect();
                (m, vec![1.0; n])
            }
            Expr::Kind { kind } => {
                let notes = kind == "notes";
                (self.refs.iter().map(|r| matches!(r, Ref::Note(_)) == notes).collect(), vec![1.0; n])
            }
            Expr::Preset { name } => {
                if let Some(done) = self.presets.get(name) {
                    return Ok(done.clone());
                }
                if self.open.contains(name) {
                    return Err(format!("the filter {name:?} comes back to itself"));
                }
                let p = self.store.filter_preset(name)?;
                self.open.push(name.clone());
                let r = self.eval(&p.expr, depth + 1);
                self.open.pop();
                let r = r?;
                self.presets.insert(name.clone(), r.clone());
                r
            }
        })
    }
}

impl Store {
    /// runs an expression over the save's memories and notes (up to `scan`), the
    /// strongest matches first
    pub fn run_filter(&self, e: &Expr, limit: usize, scan: usize, pool: &Pool) -> StoreResult<Results> {
        self.check_expr(e, &mut vec![])?;
        let cands = self.candidates(scan)?;
        let mut u = Universe {
            store: self,
            pool,
            refs: cands.iter().map(|c| c.0.clone()).collect(),
            texts: cands.iter().map(|c| c.1.clone()).collect(),
            shown: cands.iter().map(|c| c.2.clone()).collect(),
            checked: HashMap::new(),
            notes: None,
            presets: HashMap::new(),
            open: Vec::new(),
        };
        let (m, s) = u.eval(e, 0)?;
        let mut order: Vec<usize> = (0..u.refs.len()).filter(|&i| m[i]).collect();
        order.sort_by(|&a, &b| s[b].total_cmp(&s[a]).then(a.cmp(&b)));
        let matched = order.len();
        let mut names: Vec<String> = u.checked.keys().cloned().collect();
        names.sort();
        let untrained = names.iter().filter(|n| u.checked[*n].is_none()).cloned().collect();
        Ok(Results {
            scanned: u.refs.len(),
            matched,
            results: order
                .into_iter()
                .take(limit)
                .map(|i| Found {
                    key: u.refs[i].label(),
                    text: u.shown[i].clone(),
                    note: matches!(u.refs[i], Ref::Note(_)),
                    score: s[i],
                })
                .collect(),
            classifiers: names,
            untrained,
        })
    }

    /// runs a saved filter and writes what it finds as a list headed by the memory
    /// "filter: <name>" (replacing what that list held); returns the head and the count
    pub fn enlist_filter(&self, name: &str, scan: usize, pool: &Pool) -> StoreResult<(Ref, usize)> {
        let mut p = self.filter_preset(name)?;
        let (head, _) = self.saturate(&format!("filter: {}", p.name))?;
        let head = Ref::Memory(head);
        let r = self.run_filter(&p.expr, usize::MAX, scan, pool)?;
        let pairs: Vec<(Ref, Ref)> = r
            .results
            .iter()
            .filter_map(|f| Ref::parse(&f.key))
            .filter(|m| m != &head)
            .map(|m| (head.clone(), m))
            .collect();
        self.unlist(&head)?;
        self.enlist_many(&pairs)?;
        p.list = Some(head.label());
        p.count = Some(pairs.len());
        self.put_preset(&p)?;
        Ok((head, pairs.len()))
    }

    /// notes the last run's count on a preset
    pub fn note_filter_count(&self, name: &str, count: usize) -> StoreResult<()> {
        let mut p = self.filter_preset(name)?;
        p.count = Some(count);
        self.put_preset(&p)
    }

    /// every preset, for a bundle
    pub fn import_filters(&self, all: &[Preset]) -> StoreResult<usize> {
        for p in all {
            clean_name(&p.name)?;
            // a bundle is data from anywhere: only its size is checked here (what it reads
            // may come with it), the rest when it runs
            if p.expr.count() > MAX_NODES {
                return Err(format!("the filter {:?} holds more than {MAX_NODES} conditions", p.name));
            }
            self.put_preset(p)?;
        }
        Ok(all.len())
    }
}
