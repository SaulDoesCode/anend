//! Store tests: entries, versions, borrowing, edges, indexes in both modes, backups.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::codec::{Callee, Seg};
use super::gematria::Measure;
use super::gematria::Mode;
use super::interp::build::*;
use super::place::Place;
use super::query::{Dir, Order, Page, Query};
use super::record::{self as rec, mime};
use super::store::{IndexMode, Reader, rel};
use super::*;

/// a store in memory with a clock the test moves
fn store() -> (Graph, Arc<AtomicU64>) {
    let t = Arc::new(AtomicU64::new(1_000_000));
    let c = t.clone();
    let g = Graph::in_memory().unwrap().with_clock(move || c.load(Ordering::SeqCst));
    (g, t)
}

fn texts(g: &Graph, ids: &[u64]) -> Vec<String> {
    let v = g.view().unwrap();
    ids.iter().map(|&id| v.render(id).unwrap()).collect()
}

#[test]
fn save_dedup_render_and_ids() {
    let (g, t) = store();
    let (a, new) = g.save_text("the quick brown fox", mime::TEXT).unwrap();
    assert!(new);
    assert_eq!(a, 0, "the first id is weum's key [0]");
    assert_eq!(g.save_text("the quick brown fox", mime::TEXT).unwrap(), (a, false));
    // the same text of another mime is another entry
    let (n, new) = g.save_text("the quick brown fox", mime::MARKDOWN).unwrap();
    assert!(new && n != a);
    assert_eq!(g.render(a).unwrap(), "the quick brown fox");
    let e = g.entry(a).unwrap();
    assert_eq!((e.record.created, e.record.edited), (1_000_000, 1_000_000));
    t.store(2_000_000, Ordering::SeqCst);
    // forgotten ids are reused before new ones, in weum's key byte order
    let ids: Vec<u64> = (0..300)
        .map(|i| g.save_text(&format!("text {i}"), mime::TEXT).unwrap().0)
        .collect();
    assert_eq!(ids[0], 2);
    assert_eq!(rec::label(ids[299]), "0,45");
    g.delete(ids[299]).unwrap(); // [0,45]
    g.delete(ids[10]).unwrap(); // [12]
    // [0,45] sorts before [12] as bytes, like weum's reusables
    assert_eq!(g.save_text("again", mime::TEXT).unwrap().0, ids[299]);
    assert_eq!(g.save_text("again 2", mime::TEXT).unwrap().0, ids[10]);
    assert_eq!(rec::label(g.save_text("again 3", mime::TEXT).unwrap().0), "0,46");
    assert!(g.render(999_999).is_err());
}

#[test]
fn edits_keep_versions() {
    let (g, t) = store();
    let (id, _) = g.save_text("hello world", mime::TEXT).unwrap();
    t.store(2_000_000, Ordering::SeqCst);
    g.edit_text(id, "hello there world").unwrap();
    t.store(3_000_000, Ordering::SeqCst);
    g.edit_text(id, "Hello there, World!").unwrap();
    // no change, no version
    g.edit_text(id, "Hello there, World!").unwrap();
    let v = g.view().unwrap().versions(id).unwrap();
    let got: Vec<(u64, &str)> = v.iter().map(|v| (v.at, v.text.as_str())).collect();
    assert_eq!(
        got,
        vec![
            (3_000_000, "Hello there, World!"),
            (2_000_000, "hello there world"),
            (1_000_000, "hello world"),
        ]
    );
    let e = g.entry(id).unwrap();
    assert_eq!((e.record.created, e.record.edited), (1_000_000, 3_000_000));
    // the old words are gone from the index, the new ones are in
    let v = g.view().unwrap();
    assert!(v.search_words("hello").unwrap().is_empty());
    assert_eq!(v.search_words("Hello").unwrap(), vec![id]);
    // the past versions are deltas: a few bytes each
    let rows = v.history_rows(id).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].1.len() < 8, "{rows:?}");
}

#[test]
fn borrowing_keeps_rendered_text() {
    let (g, _) = store();
    let (base, _) = g.save_text("the quick brown fox jumps over", mime::TEXT).unwrap();
    let (tail, _) = g
        .save_text("and then it ran far away from everyone", mime::TEXT)
        .unwrap();
    let whole = "the quick brown fox jumps over the lazy dog and then it ran far away from everyone";
    let (v1, _) = g.save_text(whole, mime::TEXT).unwrap();
    let e = g.entry(v1).unwrap().record;
    assert!(e.has_refs() && e.plain(), "{e:?}");
    assert_eq!(
        g.view().unwrap().source(v1).unwrap(),
        vec![Seg::Ref(base), Seg::text(" the lazy dog "), Seg::Ref(tail)]
    );
    assert_eq!(g.render(v1).unwrap(), whole);
    assert!(e.body.len() < codec::pack_text(whole).len() / 2);
    // saving it again finds it (the hash is over the rendered text)
    assert_eq!(g.save_text(whole, mime::TEXT).unwrap(), (v1, false));
    // a variant of the variant borrows from it
    let longer = format!("{whole} and never came back");
    let (v2, _) = g.save_text(&longer, mime::TEXT).unwrap();
    assert_eq!(g.view().unwrap().source(v2).unwrap()[0], Seg::Ref(v1));
    assert_eq!(g.entry(v2).unwrap().record.depth(), 2);

    // editing a source doesn't change what borrows from it
    g.edit_text(base, "something else entirely now").unwrap();
    assert_eq!(g.render(v1).unwrap(), whole);
    assert_eq!(g.render(v2).unwrap(), longer);
    // nor does deleting one
    g.delete(tail).unwrap();
    assert_eq!(g.render(v1).unwrap(), whole);
    assert_eq!(g.render(v2).unwrap(), longer);
    g.delete(v1).unwrap();
    assert_eq!(g.render(v2).unwrap(), longer);
    // and the indexes still find them by their whole text
    let v = g.view().unwrap();
    assert_eq!(v.search_words("lazy never").unwrap(), vec![v2]);
    assert_eq!(v.find_text(&longer, None).unwrap(), vec![v2]);
}

#[test]
fn no_borrow_cycles() {
    let (g, _) = store();
    let (a, _) = g.save_text("alpha beta", mime::TEXT).unwrap();
    let (b, _) = g.save_text("alpha beta gamma delta epsilon", mime::TEXT).unwrap();
    assert!(g.entry(b).unwrap().record.has_refs());
    // a now becomes a longer text that starts with b's text: borrowing b would loop
    g.edit_text(a, "alpha beta gamma delta epsilon zeta eta theta").unwrap();
    assert_eq!(g.render(a).unwrap(), "alpha beta gamma delta epsilon zeta eta theta");
    assert_eq!(g.render(b).unwrap(), "alpha beta gamma delta epsilon");
}

#[test]
fn templates_and_functions_in_the_store() {
    let (g, _) = store();
    let (name, _) = g.save_text("world", mime::TEXT).unwrap();
    let greet = g
        .write(|tx| tx.save_run(&cat(vec![t("hello "), r(name), t("!")]), mime::TEXT))
        .unwrap();
    assert_eq!(g.render(greet).unwrap(), "hello world!");
    // transclusion is live: edits show through
    g.edit_text(name, "there").unwrap();
    assert_eq!(g.render(greet).unwrap(), "hello there!");
    // a recursive function entry
    let fact = g.write(|tx| tx.save_run(&[], mime::FN)).unwrap();
    let body = b(
        "if",
        vec![
            b("eq", vec![arg(0), t("0")]),
            t("1"),
            b("mul", vec![arg(0), f(fact, vec![b("sub", vec![arg(0), t("1")])])]),
        ],
    );
    g.write(|tx| tx.edit_run(fact, &body)).unwrap();
    assert_eq!(g.view().unwrap().call(fact, &["12"]).unwrap(), "479001600");
    let user = g
        .write(|tx| tx.save_run(&cat(vec![t("6! = "), f(fact, vec![t("6")])]), mime::TEXT))
        .unwrap();
    assert_eq!(g.render(user).unwrap(), "6! = 720");
    // a ref to a deleted entry is an error, and the id isn't reused while referred to
    g.delete(name).unwrap();
    assert!(g.render(greet).is_err());
    let (fresh, _) = g.save_text("fresh", mime::TEXT).unwrap();
    assert_ne!(fresh, name);
    let _ = Callee::Builtin(0);
}

#[test]
fn notes_lists_tags_places() {
    let (g, t) = store();
    let (x, _) = g.save_text("a thing", mime::TEXT).unwrap();
    let n1 = g.write(|tx| tx.note(x, "# first\nnoticed it")).unwrap();
    t.store(1_000_500, Ordering::SeqCst);
    let n2 = g.write(|tx| tx.note(x, "second thoughts")).unwrap();
    let v = g.view().unwrap();
    assert_eq!(v.notes_on(x).unwrap(), vec![n1, n2]);
    assert_eq!(v.noted_on(n2).unwrap(), vec![x]);
    assert_eq!(v.entry(n1).unwrap().record.mime, mime::MARKDOWN);
    drop(v);

    let (list, _) = g.save_text("my list", mime::TEXT).unwrap();
    let (m1, _) = g.save_text("one", mime::TEXT).unwrap();
    let (m2, _) = g.save_text("two", mime::TEXT).unwrap();
    g.write(|tx| {
        tx.add_member(list, m2)?;
        tx.add_member(list, m1)?;
        tx.add_member(list, m2)?;
        tx.add_member(list, x)
    })
    .unwrap();
    assert_eq!(g.view().unwrap().members(list).unwrap(), vec![m2, m1, x]);
    g.write(|tx| tx.order_members(list, &[x, m1])).unwrap();
    assert_eq!(g.view().unwrap().members(list).unwrap(), vec![x, m1, m2]);

    let red = g.write(|tx| tx.tag(x, "red")).unwrap();
    g.write(|tx| tx.tag(m1, "red")).unwrap();
    assert_eq!(g.write(|tx| tx.tag(m1, "red")).unwrap(), red);
    assert_eq!(g.entry(red).unwrap().record.mime, mime::TAG);
    let mut tagged = g.view().unwrap().in_edges(red, rel::TAG).unwrap();
    tagged.sort();
    assert_eq!(tagged, vec![x, m1]);

    let (map, _) = g.save_text("home", mime::MAP).unwrap();
    let p1 = Place::text([1.0, 2.0, 3.0], 0.5, [1, 2, 3, 255], 0);
    let p2 = Place::text([4.0, 5.0, 6.0], 0.5, [1, 2, 3, 255], 0);
    g.write(|tx| {
        tx.place(map, x, &p1)?;
        tx.place(map, x, &p2)?;
        tx.place(map, x, &p1)
    })
    .unwrap();
    let placed = g.view().unwrap().placed_in(map).unwrap();
    assert_eq!(placed.len(), 2);
    assert!(placed.iter().all(|(id, _)| *id == x));
    // removing one placement keeps the reverse edge while the other stands
    g.write(|tx| tx.unlink_exact(map, rel::PLACE, x, &p1.pack())).unwrap();
    assert_eq!(g.view().unwrap().in_edges(x, rel::PLACE).unwrap(), vec![map]);
    g.write(|tx| tx.unlink_exact(map, rel::PLACE, x, &p2.pack())).unwrap();
    assert!(g.view().unwrap().in_edges(x, rel::PLACE).unwrap().is_empty());

    // deleting x drops its edges both ways, and undelete puts them back
    let removed = g.delete(x).unwrap();
    let v = g.view().unwrap();
    assert!(v.notes_on(x).unwrap().is_empty());
    assert_eq!(v.members(list).unwrap(), vec![m1, m2]);
    assert!(v.noted_on(n1).unwrap().is_empty());
    drop(v);
    g.write(|tx| tx.undelete(&removed)).unwrap();
    let v = g.view().unwrap();
    assert_eq!(v.render(x).unwrap(), "a thing");
    assert_eq!(v.notes_on(x).unwrap(), vec![n1, n2]);
    assert!(v.members(list).unwrap().contains(&x));
    assert!(v.tags_of(x).unwrap().contains(&red));
}

fn fill(g: &Graph, t: &AtomicU64) -> Vec<u64> {
    let words = [
        "listen",
        "silent",
        "the cat sat",
        "the cat sat on the mat",
        "Gardening plans",
        "tinsel",
    ];
    let mut ids = Vec::new();
    for (i, w) in words.iter().enumerate() {
        t.store(1_000_000 + i as u64 * 10_000_000, Ordering::SeqCst);
        ids.push(g.save_text(w, mime::TEXT).unwrap().0);
    }
    ids
}

fn check_queries(g: &Graph, ids: &[u64]) {
    let v = g.view().unwrap();
    let m = Measure::of("listen");
    for mode in Mode::ALL {
        let mut found = v.by_sum(mode, m.get(mode)).unwrap();
        found.sort();
        assert!(found.contains(&ids[0]) && found.contains(&ids[1]), "{mode:?}");
    }
    let mut anagrams = v.by_letters("enlist").unwrap();
    anagrams.sort();
    assert_eq!(anagrams, vec![ids[0], ids[1], ids[5]]);
    assert_eq!(v.search_words("cat sat").unwrap(), vec![ids[2], ids[3]]);
    // three quarters of the words must be there
    assert!(v.search_words("cat sat dog bird").unwrap().is_empty());
    assert_eq!(v.created_between(10_000_000, 25_000_000).unwrap(), vec![ids[1], ids[2]]);
    let q = Query {
        words: Some("the".into()),
        created: Some((0, 35_000_000)),
        order: Order::CreatedDesc,
        ..Default::default()
    };
    let found = v.query(&q, Page::first(1)).unwrap();
    assert_eq!((found.ids.clone(), found.total), (vec![ids[3]], 2));
    let all = v.query(&Query::default(), Page { offset: 2, limit: 2 }).unwrap();
    assert_eq!((all.ids, all.total), (vec![ids[2], ids[3]], 6));
    let words = v.words_with_prefix(b"si", 10).unwrap();
    assert_eq!(words, vec![(b"silent".to_vec(), vec![ids[1]])]);
    let counts = v.sum_counts(Mode::Ordinal).unwrap();
    assert_eq!(counts.iter().map(|c| c.1).sum::<u64>(), 6);
    let stats = v.stats().unwrap();
    assert_eq!(stats.entries, 6);
}

#[test]
fn queries_on_disk_and_in_memory() {
    let (mut g, t) = store();
    let ids = fill(&g, &t);
    assert_eq!(g.index_mode(), IndexMode::Disk);
    check_queries(&g, &ids);
    g.set_index_mode(IndexMode::Memory).unwrap();
    check_queries(&g, &ids);
    // writes keep the memory index current
    let (n, _) = g.save_text("enlist", mime::TEXT).unwrap();
    assert!(g.view().unwrap().by_letters("listen").unwrap().contains(&n));
    g.delete(n).unwrap();
    assert!(!g.view().unwrap().by_letters("listen").unwrap().contains(&n));
    // the disk tables are emptied in memory mode
    let uses = g.table_use().unwrap();
    assert_eq!(uses.iter().find(|u| u.name == "words").unwrap().rows, 0);
    g.set_index_mode(IndexMode::Disk).unwrap();
    check_queries(&g, &ids);
    g.reindex().unwrap();
    check_queries(&g, &ids);
}

#[test]
fn relation_queries() {
    let (g, _) = store();
    let (list, _) = g.save_text("list", mime::TEXT).unwrap();
    let ms: Vec<u64> = ["c", "a", "b"]
        .iter()
        .map(|s| g.save_text(s, mime::TEXT).unwrap().0)
        .collect();
    g.write(|tx| {
        for m in &ms {
            tx.add_member(list, *m)?;
        }
        Ok(())
    })
    .unwrap();
    let v = g.view().unwrap();
    let q = Query {
        related: Some((rel::MEMBER, list, Dir::From)),
        order: Order::Relation,
        ..Default::default()
    };
    assert_eq!(v.query(&q, Page::all()).unwrap().ids, ms);
    let q = Query {
        related: Some((rel::MEMBER, ms[1], Dir::To)),
        ..Default::default()
    };
    assert_eq!(v.query(&q, Page::all()).unwrap().ids, vec![list]);
}

#[test]
fn weum_import_and_backup_round_trip() {
    let (g, t) = store();
    let weum =
        r#"[["hello world",[0]],["hello world and more",[1]],["שלום",[0,12]],["Mixed Case",[3,14]],["bad"],"x"]"#;
    let rep = g.import_texts(weum).unwrap();
    assert_eq!((rep.entries, rep.skipped), (4, 2));
    assert_eq!(rep.borrowing, 1);
    let id = rec::parse_label("3,14").unwrap();
    assert_eq!(g.render(id).unwrap(), "Mixed Case");
    assert_eq!(g.entry(id).unwrap().record.created, 1_000_000);
    // new ids continue after the highest imported key
    assert_eq!(rec::label(g.save_text("next", mime::TEXT).unwrap().0), "3,15");

    // more of everything, then a backup
    t.store(5_000_000, Ordering::SeqCst);
    let n = g.write(|tx| tx.note(0, "a note on it")).unwrap();
    g.write(|tx| tx.add_member(1, 0)).unwrap();
    g.edit_text(0, "hello world, edited").unwrap();
    let tmpl = g
        .write(|tx| tx.save_run(&cat(vec![t_("<"), r(0), t_(">")]), mime::TEXT))
        .unwrap();
    let custom = g.write(|tx| tx.mime_id("text/x-custom")).unwrap();
    g.write(|tx| tx.set_mime(n, custom)).unwrap();
    g.write(|tx| tx.set_setting("theme", b"dark")).unwrap();
    g.write(|tx| tx.set_setting("blob", &[0xff, 0])).unwrap();
    let gone = g.save_text("to forget", mime::TEXT).unwrap().0;
    g.delete(gone).unwrap();
    let (texts, other) = g.backup().unwrap();
    assert!(
        texts.starts_with("[[\"hello world, edited\",[0],1000000,5000000]"),
        "{texts}"
    );

    let (h, _) = store();
    h.restore(&texts, &other).unwrap();
    let a = g.view().unwrap();
    let b = h.view().unwrap();
    let ea = a.all().unwrap();
    let eb = b.all().unwrap();
    assert_eq!(ea.len(), eb.len());
    for (x, y) in ea.iter().zip(&eb) {
        assert_eq!(x.id, y.id);
        assert_eq!(
            (x.record.created, x.record.edited, x.record.mime),
            (y.record.created, y.record.edited, y.record.mime)
        );
        assert_eq!(a.render(x.id).unwrap(), b.render(y.id).unwrap());
        assert_eq!(a.versions(x.id).unwrap(), b.versions(y.id).unwrap());
    }
    assert_eq!(b.source(tmpl).unwrap(), a.source(tmpl).unwrap());
    assert_eq!(a.all_edges().unwrap(), b.all_edges().unwrap());
    assert_eq!(a.all_settings().unwrap(), b.all_settings().unwrap());
    assert_eq!(a.free_ids().unwrap(), b.free_ids().unwrap());
    assert_eq!(a.allocated().unwrap(), b.allocated().unwrap());
    // and a second backup is the same as the first
    assert_eq!(h.backup().unwrap(), (texts, other));
}

fn t_(s: &str) -> Vec<Seg> {
    t(s)
}

#[test]
fn file_store_and_compaction() {
    let path = std::env::temp_dir().join(format!("anend-graph-{}.redb", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let mut g = Graph::open(&path).unwrap();
        for i in 0..200 {
            g.save_text(&format!("text number {i} with a few words"), mime::TEXT)
                .unwrap();
        }
        for i in 0..100 {
            g.delete(i).unwrap();
        }
        g.compact().unwrap();
    }
    let g = Graph::open(&path).unwrap();
    assert_eq!(g.view().unwrap().entry_count().unwrap(), 100);
    assert_eq!(texts(&g, &[150]), vec!["text number 150 with a few words"]);
    let uses = g.table_use().unwrap();
    assert!(uses.iter().find(|u| u.name == "entries").unwrap().rows <= 7);
    drop(g);
    std::fs::remove_file(path).ok();
}

#[test]
fn long_records_live_in_pieces_and_read_back_whole() {
    use redb::{ReadableDatabase, ReadableTableMetadata};
    let (g, clock) = store();
    // small pieces, so a few KB is "long"
    g.write(|tx| tx.set_setting("graph.piece_bytes", b"1024")).unwrap();
    let pieces = |g: &Graph| {
        let txn = g.db().begin_read().unwrap();
        txn.open_table(super::store::LONG).unwrap().len().unwrap()
    };
    let (near, _) = g.save_text("a short neighbour", mime::TEXT).unwrap();
    let long: String = (0..400).map(|i| format!("line {i} of a long text\n")).collect();
    let (id, _) = g.save_text(&long, mime::TEXT).unwrap();
    assert!(long.len() > 8 * 1024);
    let first = pieces(&g);
    assert!(first >= 4, "{first} pieces");
    assert_eq!(g.render(id).unwrap(), long);
    assert_eq!(g.render(near).unwrap(), "a short neighbour");
    assert_eq!(g.view().unwrap().find_text(&long, None).unwrap(), vec![id]);
    // edited short: the pieces go; long again: they come back; history has every version
    clock.fetch_add(10, Ordering::SeqCst);
    g.write(|tx| tx.edit_text(id, "now short")).unwrap();
    assert_eq!(pieces(&g), 0);
    clock.fetch_add(10, Ordering::SeqCst);
    let longer = format!("{long}{long}");
    g.write(|tx| tx.edit_text(id, &longer)).unwrap();
    assert!(pieces(&g) > first);
    let vs: Vec<String> = g
        .view()
        .unwrap()
        .versions(id)
        .unwrap()
        .into_iter()
        .map(|v| v.text)
        .collect();
    assert_eq!(vs, vec![longer.clone(), "now short".to_string(), long.clone()]);
    // every entry, backups and restores see the whole text
    assert!(g.view().unwrap().all().unwrap().iter().any(|e| e.id == id));
    let (t, o) = g.backup().unwrap();
    let back = Graph::in_memory().unwrap();
    back.restore(&t, &o).unwrap();
    let bid = back.view().unwrap().find_text(&longer, None).unwrap()[0];
    assert_eq!(back.render(bid).unwrap(), longer);
    // forgetting it takes its pieces
    g.delete(id).unwrap();
    assert_eq!(pieces(&g), 0);
    assert_eq!(g.render(near).unwrap(), "a short neighbour");
}
