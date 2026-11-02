//! Each starter classifier trained from scratch and scored on its held-out texts, as the
//! app does it (the preset's training options), printed as a table:
//!
//! ```text
//! cargo run --release -p anend-core --example starters [preset] [epochs] [texts a class] [patience] [starter id]
//! ```
//!
//! The fetched starter is left out unless its books are in `./starters`.

use std::time::Instant;

use anend_core::classifiers::{preset, preset_opts};
use anend_core::classify::train::{self, Example};
use anend_core::classify::{Classifier, Flow, HeadSpec, Pool, pool};
use anend_core::starters::{self, Source, Split};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let size = args.get(1).map(String::as_str).unwrap_or("compact");
    let epochs: Option<usize> = args.get(2).and_then(|s| s.parse().ok());
    let per: usize = args
        .get(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(starters::TRAIN_PER_CLASS);
    let patience: Option<usize> = args.get(4).and_then(|s| s.parse().ok());
    let only: Option<&str> = args.get(5).map(String::as_str);
    let books = std::path::Path::new("starters");
    let p = Pool::new(pool::available());
    println!(
        "{:<28} {:>6} {:>6} {:>9} {:>8} {:>7}",
        "starter", "train", "held", "held-out", "val acc", "seconds"
    );
    for s in starters::STARTERS.iter().filter(|s| only.is_none_or(|o| o == s.id)) {
        if s.source == Source::Fetched && !starters::fetched(books) {
            println!("{:<28} (its books aren't in ./starters)", s.title);
            continue;
        }
        let train_set = starters::examples_with(s.id, Split::Train, Some(books), per).unwrap();
        let held = starters::examples(s.id, Split::Held, Some(books)).unwrap();
        let cfg = preset(size).expect("a preset").gematria(s.gematria);
        let mut clf = Classifier::new(cfg, &[HeadSpec::single(s.id, s.classes)], 1).unwrap();
        let mut o = preset_opts(size);
        if let Some(e) = epochs {
            o.epochs = e;
        }
        if let Some(n) = patience {
            o.patience = n;
        }
        let data: Vec<Example> = train_set.iter().map(|(t, c)| Example::class(t, *c)).collect();
        let t = Instant::now();
        let r = train::train(&mut clf, &data, &o, |_| Flow::Go).unwrap();
        let secs = t.elapsed().as_secs_f64();
        let texts: Vec<&str> = held.iter().map(|(t, _)| t.as_str()).collect();
        let outs = clf.predict_many(&texts, &[], &p);
        let right = held.iter().zip(&outs).filter(|((_, c), o)| o[0].top == *c).count();
        let val = r
            .eval
            .as_ref()
            .and_then(|e| e.heads.first().cloned().flatten())
            .map(|m| m.accuracy)
            .unwrap_or(f64::NAN);
        println!(
            "{:<28} {:>6} {:>6} {:>8.1}% {:>7.1}% {:>7.1}",
            s.title,
            train_set.len(),
            held.len(),
            100.0 * right as f64 / held.len() as f64,
            100.0 * val,
            secs
        );
    }
}
