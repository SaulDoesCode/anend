//! The entry graph store against the old store, on a deterministic corpus like weum's:
//! about 100k short lowercase texts with many prefix and suffix variants, and 2k longer
//! mixed-case markdown notes. Reports bytes on disk (whole files and per table, against
//! the raw packed bytes), compression on sample corpora, insert/search/render latency, the
//! effect of borrowing, and how long rebuilding the indexes into memory takes.
//!
//! `cargo run --release -p anend-core --example graph_bench [texts] [notes]`

use std::path::{Path, PathBuf};
use std::time::Instant;

use anend_core::graph::codec;
use anend_core::graph::gematria::Mode;
use anend_core::graph::record::mime;
use anend_core::graph::store::{IndexMode, Reader};
use anend_core::graph::{Graph, query::Page, query::Query};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn vocabulary(r: &mut Rng) -> Vec<String> {
    let on = [
        "", "b", "c", "d", "f", "g", "h", "l", "m", "n", "p", "r", "s", "t", "th", "st", "w", "sh", "k",
    ];
    let nu = ["a", "e", "i", "o", "u", "ea", "ou", "ai", "y"];
    let co = ["", "n", "r", "s", "t", "l", "nd", "ng", "st", "ck", "m"];
    let mut words: Vec<String> = [
        "the", "and", "of", "to", "a", "in", "is", "it", "that", "was", "for", "on", "with", "as", "be",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    while words.len() < 3000 {
        let syl = 1 + r.below(3);
        let w: String = (0..syl)
            .map(|_| {
                format!(
                    "{}{}{}",
                    on[r.below(on.len())],
                    nu[r.below(nu.len())],
                    co[r.below(co.len())]
                )
            })
            .collect();
        words.push(w);
    }
    words
}

/// a word, common ones more often (roughly Zipf)
fn word<'a>(r: &mut Rng, v: &'a [String]) -> &'a str {
    let x = r.below(1000) as f64 / 1000.0;
    &v[((x * x * x) * v.len() as f64) as usize % v.len()]
}

/// `base` plus up to `spread - 1` words
fn phrase(r: &mut Rng, v: &[String], base: usize, spread: usize) -> Vec<String> {
    let n = base + r.below(spread);
    (0..n).map(|_| word(r, v).to_string()).collect()
}

fn corpus(n_texts: usize, n_notes: usize) -> (Vec<String>, Vec<String>) {
    let mut r = Rng(0x5eed_1234_abcd_0001);
    let v = vocabulary(&mut r);
    let mut bases: Vec<Vec<String>> = Vec::new();
    let mut texts = Vec::with_capacity(n_texts);
    while texts.len() < n_texts {
        let make_variant = !bases.is_empty() && r.below(100) < 55;
        let words = if make_variant {
            let b = bases[r.below(bases.len())].clone();
            match r.below(4) {
                // a suffix added
                0 | 1 => [b, phrase(&mut r, &v, 1, 4)].concat(),
                // a prefix added
                2 => [phrase(&mut r, &v, 1, 3), b].concat(),
                // a suffix missing
                _ => b[..(b.len() / 2).max(1)].to_vec(),
            }
        } else {
            phrase(&mut r, &v, 2, 8)
        };
        if words.len() >= 3 {
            bases.push(words.clone());
        }
        texts.push(words.join(" "));
    }
    let mut notes = Vec::with_capacity(n_notes);
    for i in 0..n_notes {
        let mut t = String::new();
        let title: Vec<String> = phrase(&mut r, &v, 2, 4)
            .into_iter()
            .map(|w| {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect())
                    .unwrap_or_default()
            })
            .collect();
        t.push_str(&format!("# {}\n\n", title.join(" ")));
        for p in 0..1 + r.below(4) {
            let mut words = phrase(&mut r, &v, 30, 120);
            words[0] = {
                let mut c = words[0].chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect())
                    .unwrap_or_default()
            };
            let k = r.below(words.len());
            words[k] = format!("**{}**", words[k]);
            t.push_str(&words.join(" "));
            t.push_str(&format!(
                ". Noted {} on day {}.\n\n",
                1900 + (i * 7 + p) % 120,
                r.below(365)
            ));
            if r.below(3) == 0 {
                for _ in 0..2 + r.below(4) {
                    t.push_str(&format!("- {}\n", phrase(&mut r, &v, 2, 5).join(" ")));
                }
                t.push('\n');
            }
        }
        notes.push(t);
    }
    (texts, notes)
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn mb(b: u64) -> String {
    format!("{:.2} MB", b as f64 / 1e6)
}

fn scratch(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("anend-graph-bench-{}-{name}.redb", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn compression() {
    println!("\n## Compression (packed / UTF-8)\n");
    let samples = [
        ("lowercase prose", "the quick brown fox jumps over the lazy dog and keeps on running far away from home into the night where nobody can see it any more".to_string()),
        ("mixed case markdown", "# Gardening Plans\n\nPlant **Tomatoes** in March, 2026-03-14.\n- Water them daily\n- NASA says it's fine\n\nSee [the guide](http://example.org/guide) for more.".to_string()),
        ("hebrew", "בראשית ברא אלהים את השמים ואת הארץ והארץ היתה תהו ובהו וחשך על פני תהום".to_string()),
        ("greek", "Ἐν ἀρχῇ ἦν ὁ λόγος, καὶ ὁ λόγος ἦν πρὸς τὸν θεόν, καὶ θεὸς ἦν ὁ λόγος.".to_string()),
        ("russian", "В начале было Слово, и Слово было у Бога, и Слово было Бог.".to_string()),
        ("japanese", "初めに言があった。言は神と共にあった。言は神であった。".to_string()),
        ("digits", "call 5551234567 at 0900 on 2026-10-14, ref 31415926535".to_string()),
    ];
    println!("| corpus | UTF-8 bytes | packed bytes | ratio |\n|---|---|---|---|");
    for (name, t) in samples {
        let p = codec::pack_text(&t).len();
        println!("| {name} | {} | {p} | {:.3} |", t.len(), p as f64 / t.len() as f64);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n_texts: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(100_000);
    let n_notes: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(2_000);
    let (texts, notes) = corpus(n_texts, n_notes);
    let utf8: usize = texts.iter().chain(&notes).map(|t| t.len()).sum();
    let packed: usize = texts.iter().chain(&notes).map(|t| codec::pack_text(t).len()).sum();
    println!("# graph_bench: {n_texts} texts, {n_notes} notes\n");
    println!(
        "UTF-8 text: {}; packed 5-bit (no borrowing): {} ({:.3})",
        mb(utf8 as u64),
        mb(packed as u64),
        packed as f64 / utf8 as f64
    );
    compression();

    // ---- the old store
    let old_path = scratch("old");
    let t0 = Instant::now();
    {
        let mut s = anend_core::store::Store::open(&old_path).unwrap();
        s.saturate_many(&texts).unwrap();
        let ns: Vec<anend_core::note::Note> = notes
            .iter()
            .map(|n| {
                let (title, body) = n.split_once("\n\n").unwrap_or((n, ""));
                anend_core::note::Note {
                    title: title.trim_start_matches("# ").to_string(),
                    markdown: body.to_string(),
                    ..Default::default()
                }
            })
            .collect();
        s.import(ns).unwrap();
        s.db.compact().unwrap();
    }
    let old_time = t0.elapsed();
    let old_size = size(&old_path);

    // ---- the new store, one import transaction, with and without borrowing
    let rows: Vec<serde_json::Value> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| serde_json::json!([t, anend_core::graph::record::key_of(i as u64)]))
        .collect();
    let json = serde_json::to_string(&rows).unwrap();
    let mut results = Vec::new();
    for (label, borrow, mode) in [
        ("disk indexes, borrowing", true, IndexMode::Disk),
        ("disk indexes, no borrowing", false, IndexMode::Disk),
        ("memory indexes, borrowing", true, IndexMode::Memory),
    ] {
        let path = scratch(label.replace([' ', ','], "-").as_str());
        let mut g = Graph::open(&path).unwrap();
        g.set_index_mode(mode).unwrap();
        g.borrow.enabled = borrow;
        let t = Instant::now();
        let rep = g.import_texts(&json).unwrap();
        let import = t.elapsed();
        let t = Instant::now();
        g.write(|tx| {
            for n in &notes {
                tx.save_text(n, mime::MARKDOWN)?;
            }
            Ok(())
        })
        .unwrap();
        let notes_time = t.elapsed();
        let before = size(&path);
        g.compact().unwrap();
        let after = size(&path);
        let stats = g.view().unwrap().stats().unwrap();
        results.push((
            label,
            path.clone(),
            rep.borrowing,
            import,
            notes_time,
            before,
            after,
            stats.body_bytes,
        ));
        if label == "disk indexes, borrowing" {
            println!("\n## Tables ({label}, after compaction)\n");
            println!(
                "| table | rows | stored | metadata | fragmented | pages | fill | bytes/entry |\n|---|---|---|---|---|---|---|---|"
            );
            let n = (n_texts + n_notes) as f64;
            for u in g.table_use().unwrap() {
                if u.rows == 0 {
                    continue;
                }
                println!(
                    "| {} | {} | {} | {} | {} | {} | {:.0}% | {:.1} |",
                    u.name,
                    u.rows,
                    u.stored,
                    u.metadata,
                    u.fragmented,
                    u.leaf_pages + u.branch_pages,
                    u.fill() * 100.0,
                    u.page_bytes() as f64 / n
                );
            }
            println!("\nraw packed bodies (with borrowing): {}", mb(stats.body_bytes));
        }
        drop(g);
    }

    println!("\n## Bytes on disk\n");
    println!(
        "| store | file (compacted) | before compaction | bodies | entries borrowing | import | notes |\n|---|---|---|---|---|---|---|"
    );
    println!(
        "| old store (memories + notes) | {} | | | | {:.2?} (both) | |",
        mb(old_size),
        old_time
    );
    for (label, _, borrowing, import, nt, before, after, bodies) in &results {
        println!(
            "| new: {label} | {} | {} | {} | {borrowing} | {import:.2?} | {nt:.2?} |",
            mb(*after),
            mb(*before),
            mb(*bodies)
        );
    }

    // ---- latencies on the first new store
    let path = &results[0].1;
    let g = Graph::open(path).unwrap();
    println!("\n## Latency (disk indexes, release)\n");
    let t = Instant::now();
    let n = 200;
    for i in 0..n {
        g.save_text(
            &format!("{} and one more time {i}", texts[i * 37 % texts.len()]),
            mime::TEXT,
        )
        .unwrap();
    }
    println!(
        "insert, one durable transaction each: {:.1?} per text",
        t.elapsed() / n as u32
    );
    let t = Instant::now();
    g.write(|tx| {
        for i in 0..1000 {
            tx.save_text(&format!("batch {i} {}", texts[i * 13 % texts.len()]), mime::TEXT)?;
        }
        Ok(())
    })
    .unwrap();
    println!("insert, 1000 in one transaction: {:.1?} per text", t.elapsed() / 1000);
    let v = g.view().unwrap();
    let queries: Vec<&str> = texts.iter().step_by(997).take(100).map(|s| s.as_str()).collect();
    let t = Instant::now();
    let mut found = 0;
    for q in &queries {
        found += v.search_words(q).unwrap().len();
    }
    println!(
        "word search: {:.1?} per query ({} hits per query)",
        t.elapsed() / queries.len() as u32,
        found / queries.len()
    );
    let t = Instant::now();
    let mut found = 0;
    for q in &queries {
        let m = anend_core::graph::gematria::Measure::of(q);
        found += v.by_sum(Mode::Ordinal, m.ordinal).unwrap().len();
    }
    println!(
        "sum lookup: {:.1?} per query ({} hits per query)",
        t.elapsed() / queries.len() as u32,
        found / queries.len()
    );
    let t = Instant::now();
    let page = v
        .query(
            &Query {
                words: Some("the".into()),
                ..Default::default()
            },
            Page::first(100),
        )
        .unwrap();
    println!(
        "query \"the\", first page of 100 of {}: {:.1?}",
        page.total,
        t.elapsed()
    );
    let ids: Vec<u64> = (0..10_000u64).map(|i| i * 7 % n_texts as u64).collect();
    let t = Instant::now();
    let rendered = v.render_many(&ids);
    let borrowed = ids
        .iter()
        .filter(|&&id| v.record(id).unwrap().is_some_and(|r| r.has_refs()))
        .count();
    println!(
        "render 10k entries ({borrowed} with borrowed parts): {:.2?} per entry",
        t.elapsed() / rendered.len() as u32
    );
    let t = Instant::now();
    let text = v.render(ids[1]).unwrap();
    println!("render one: {:.1?} ({} bytes)", t.elapsed(), text.len());
    drop(v);
    drop(g);

    // ---- memory mode: what opening costs
    let t = Instant::now();
    let g = Graph::open(&results[2].1).unwrap();
    println!(
        "\nopen with the indexes rebuilt into memory: {:.2?} (about {} of ids and keys)",
        t.elapsed(),
        mb(g.memory_index_bytes() as u64)
    );
    drop(g);

    for p in results.iter().map(|r| &r.1).chain([&old_path]) {
        let _ = std::fs::remove_file(p);
    }
}
