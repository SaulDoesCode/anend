//! Chains (docs/format.md "The model", answer 4): an entry of its own, mime
//! `application/x-anend-chain`, that is a title plus an ordered run of member entries, such
//! as the paragraphs of one piece. Its body is a ref to its title entry, so it renders,
//! searches, lists and opens as its title does (and follows the title's edits); its
//! members are `MEMBER` edges with order keys, as a list's are. The same text can be a
//! member of several chains. Notes are not chains; a note placed in a scene is shown as
//! a chain of two when it is shown, with no chain entry made.

use super::codec::Seg;
use super::record::{label, mime};
use super::store::{GResult, Reader, Tx, rel};

/// a chain as read: its entry, its title entry and its members in order
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    pub id: u64,
    pub title: u64,
    pub members: Vec<u64>,
}

/// the chain an entry is, if it is one
pub fn chain_of(r: &impl Reader, id: u64) -> GResult<Option<Chain>> {
    let Some(rec) = r.record(id)? else { return Ok(None) };
    if rec.mime != mime::CHAIN {
        return Ok(None);
    }
    let title = match r.source(id)?.as_slice() {
        [Seg::Ref(t)] => *t,
        _ => return Err(format!("chain {} isn't a ref to its title", label(id))),
    };
    Ok(Some(Chain {
        id,
        title,
        members: r.members(id)?,
    }))
}

/// the chains an entry is a member of
pub fn chains_with(r: &impl Reader, member: u64) -> GResult<Vec<u64>> {
    let mut out = Vec::new();
    for c in r.in_edges(member, rel::MEMBER)? {
        if r.record(c)?.is_some_and(|x| x.mime == mime::CHAIN) {
            out.push(c);
        }
    }
    Ok(out)
}

fn check(tx: &Tx, title: u64, members: &[u64]) -> GResult<Vec<u64>> {
    if !tx.exists(title)? {
        return Err(format!("there's no entry {}", label(title)));
    }
    let mut out: Vec<u64> = Vec::new();
    for &m in members {
        if m == title || out.contains(&m) {
            continue;
        }
        if !tx.exists(m)? {
            return Err(format!("there's no entry {}", label(m)));
        }
        out.push(m);
    }
    Ok(out)
}

impl Tx<'_> {
    /// a new chain: `title` and then `members` in order (the title and repeats left out of
    /// the members); its id
    pub fn make_chain(&mut self, title: u64, members: &[u64]) -> GResult<u64> {
        let members = check(self, title, members)?;
        let id = self.save_run(&[Seg::Ref(title)], mime::CHAIN)?;
        self.set_members(id, &members)?;
        Ok(id)
    }

    /// a chain's members replaced by these, in this order (make, reorder, add, take out)
    pub fn set_chain(&mut self, chain: u64, members: &[u64]) -> GResult<()> {
        let c = chain_of(self, chain)?.ok_or_else(|| format!("{} isn't a chain", label(chain)))?;
        let members = check(self, c.title, members)?;
        self.set_members(chain, &members)
    }

    fn set_members(&mut self, chain: u64, members: &[u64]) -> GResult<()> {
        for m in self.members(chain)? {
            self.unlink(chain, rel::MEMBER, m)?;
        }
        for (i, m) in members.iter().enumerate() {
            self.link(chain, rel::MEMBER, *m, &(i as u64).to_be_bytes())?;
        }
        Ok(())
    }

    /// cuts a chain before member `at`: the members before it stay; that member heads a new
    /// chain of the ones after it (its id; None when `at` is past the end)
    pub fn cut_chain(&mut self, chain: u64, at: usize) -> GResult<Option<u64>> {
        let c = chain_of(self, chain)?.ok_or_else(|| format!("{} isn't a chain", label(chain)))?;
        if at >= c.members.len() {
            return Ok(None);
        }
        let (keep, rest) = c.members.split_at(at);
        self.set_members(chain, keep)?;
        Ok(Some(self.make_chain(rest[0], &rest[1..])?))
    }

    /// two chains joined: `b`'s title and members after `a`'s members; `b` goes
    pub fn join_chains(&mut self, a: u64, b: u64) -> GResult<()> {
        let ca = chain_of(self, a)?.ok_or_else(|| format!("{} isn't a chain", label(a)))?;
        let cb = chain_of(self, b)?.ok_or_else(|| format!("{} isn't a chain", label(b)))?;
        let mut all = ca.members.clone();
        all.push(cb.title);
        all.extend(cb.members);
        self.delete(b)?;
        self.set_chain(a, &all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;

    #[test]
    fn chains_are_entries_with_a_title_and_ordered_members() {
        let g = Graph::in_memory().unwrap();
        let ids: Vec<u64> = ["The heron", "It waits.", "It strikes.", "It eats.", "It flies off."]
            .iter()
            .map(|t| g.save_text(t, mime::TEXT).unwrap().0)
            .collect();
        let c = g
            .write(|tx| tx.make_chain(ids[0], &[ids[1], ids[2], ids[1], ids[0], ids[3]]))
            .unwrap();
        let v = g.view().unwrap();
        let ch = chain_of(&v, c).unwrap().unwrap();
        assert_eq!(ch.title, ids[0]);
        assert_eq!(
            ch.members,
            vec![ids[1], ids[2], ids[3]],
            "repeats and the title left out"
        );
        // it reads and searches as its title
        assert_eq!(v.render(c).unwrap(), "The heron");
        assert!(chain_of(&v, ids[1]).unwrap().is_none());
        assert_eq!(chains_with(&v, ids[2]).unwrap(), vec![c]);
        drop(v);
        // the title edited: the chain follows
        g.write(|tx| tx.edit_text(ids[0], "The grey heron")).unwrap();
        assert_eq!(g.render(c).unwrap(), "The grey heron");
        // reorder, cut, join
        g.write(|tx| tx.set_chain(c, &[ids[3], ids[1], ids[2], ids[4]]))
            .unwrap();
        let rest = g.write(|tx| tx.cut_chain(c, 2)).unwrap().unwrap();
        let v = g.view().unwrap();
        assert_eq!(chain_of(&v, c).unwrap().unwrap().members, vec![ids[3], ids[1]]);
        let r = chain_of(&v, rest).unwrap().unwrap();
        assert_eq!((r.title, r.members.clone()), (ids[2], vec![ids[4]]));
        drop(v);
        assert_eq!(g.write(|tx| tx.cut_chain(c, 9)).unwrap(), None);
        g.write(|tx| tx.join_chains(c, rest)).unwrap();
        let v = g.view().unwrap();
        assert_eq!(
            chain_of(&v, c).unwrap().unwrap().members,
            vec![ids[3], ids[1], ids[2], ids[4]]
        );
        assert!(!v.exists(rest).unwrap());
        // the same text in two chains
        drop(v);
        let c2 = g.write(|tx| tx.make_chain(ids[4], &[ids[1]])).unwrap();
        let mut both = chains_with(&g.view().unwrap(), ids[1]).unwrap();
        both.sort();
        assert_eq!(both, {
            let mut x = vec![c, c2];
            x.sort();
            x
        });
        // a chain survives a backup
        let (t, o) = g.backup().unwrap();
        let back = Graph::in_memory().unwrap();
        back.restore(&t, &o).unwrap();
        let bv = back.view().unwrap();
        let chains: Vec<Chain> = bv
            .ids()
            .unwrap()
            .into_iter()
            .filter_map(|i| chain_of(&bv, i).unwrap())
            .collect();
        assert_eq!(chains.len(), 2);
        assert!(chains.iter().any(|ch| ch.members.len() == 4));
    }
}
