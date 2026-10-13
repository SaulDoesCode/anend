//! Lists as a whole: each list's members, their sums added up and averaged, the list's
//! gematria read as one text, and its letters as a 26-long vector, so lists can be
//! compared like embeddings (the cosine of every two lists' vectors).

use std::collections::HashSet;

use crate::memory::{Hit, Ref};
use crate::spread;
use crate::store::{Store, StoreResult};

#[derive(Clone, Debug)]
pub struct ListStat {
    pub list: Ref,
    /// the list head's own text
    pub name: String,
    pub members: Vec<Hit>,
    pub ordinal_sum: i64,
    pub reversed_sum: i64,
    /// all the members' texts as one, for measuring the list whole
    pub joined: String,
    /// letter counts over all members, normalised to length 1
    pub vector: [f64; 26],
}

impl ListStat {
    pub fn ordinal_mean(&self) -> f64 {
        self.ordinal_sum as f64 / self.members.len().max(1) as f64
    }

    pub fn reversed_mean(&self) -> f64 {
        self.reversed_sum as f64 / self.members.len().max(1) as f64
    }
}

/// texts' letters as a unit vector
pub fn letter_vector<'a>(texts: impl IntoIterator<Item = &'a str>) -> [f64; 26] {
    let mut v = [0f64; 26];
    for t in texts {
        for (i, c) in crate::simd::frequencies(t.as_bytes()).iter().enumerate() {
            v[i] += *c as f64;
        }
    }
    let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

pub fn cosine(a: &[f64; 26], b: &[f64; 26]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// every list, in the order their heads are stored
pub fn stats(store: &Store) -> StoreResult<Vec<ListStat>> {
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut out = Vec::new();
    for (l, _) in store.all_lists()? {
        if !seen.insert(l.bytes()) {
            continue;
        }
        let members = store.list(&l)?;
        let name = store.remember(&l).map(|h| h.text).unwrap_or_else(|_| l.label());
        let (mut so, mut sr) = (0i64, 0i64);
        for m in &members {
            let (o, r) = spread::sums(&m.text);
            so += o;
            sr += r;
        }
        let joined = members.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join(" ");
        let vector = letter_vector(members.iter().map(|m| m.text.as_str()));
        out.push(ListStat {
            list: l,
            name,
            members,
            ordinal_sum: so,
            reversed_sum: sr,
            joined,
            vector,
        });
    }
    Ok(out)
}

/// the cosine of every two lists' letter vectors, rounded to thousandths
pub fn dots(lists: &[ListStat]) -> Vec<Vec<f64>> {
    lists
        .iter()
        .map(|a| {
            lists
                .iter()
                .map(|b| (cosine(&a.vector, &b.vector) * 1000.0).round() / 1000.0)
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_as_vectors() {
        let path = std::env::temp_dir().join(format!("anend-lists-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let s = Store::open(&path).unwrap();
        let k = |t: &str| Ref::Memory(s.saturate(t).unwrap().0);
        let (a, b) = (k("anagrams"), k("numbers"));
        for t in ["listen", "silent"] {
            s.enlist(&a, &k(t)).unwrap();
        }
        for t in ["seven", "eleven"] {
            s.enlist(&b, &k(t)).unwrap();
        }
        let st = stats(&s).unwrap();
        assert_eq!(st.len(), 2);
        let ana = st.iter().find(|l| l.name == "anagrams").unwrap();
        assert_eq!((ana.members.len(), ana.ordinal_sum, ana.reversed_sum), (2, 158, 166));
        assert_eq!(ana.ordinal_mean(), 79.0);
        let d = dots(&st);
        assert_eq!(d[0][0], 1.0);
        assert!(d[0][1] < 1.0 && d[0][1] > 0.0);
        drop(s);
        let _ = std::fs::remove_file(path);
    }
}
