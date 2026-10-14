//! Whole-classifier tests on the tiny preset: learning, determinism across thread
//! counts and kernel paths, the file format, windows, steering and feedback.

use super::train::{self, Feedback, FeedbackOpts};
use super::*;

const A: [&str; 8] = ["apple", "banana", "cherry", "mango", "peach", "plum", "grape", "melon"];
const B: [&str; 8] = [
    "rock", "stone", "granite", "basalt", "flint", "slate", "quartz", "shale",
];

/// texts of fruit words (class 0) or rock words (class 1)
pub fn synthetic(n: usize, seed: u64) -> Vec<Example> {
    let mut r = math::Rng::new(seed);
    (0..n)
        .map(|i| {
            let c = i % 2;
            let words = if c == 0 { &A } else { &B };
            let t: Vec<&str> = (0..3 + r.below(4)).map(|_| words[r.below(8)]).collect();
            Example::class(&t.join(" "), c)
        })
        .collect()
}

pub fn tiny_clf(seed: u64) -> Classifier {
    Classifier::new(Config::tiny(), &[HeadSpec::single("kind", &["fruit", "rock"])], seed).unwrap()
}

fn quick_opts(threads: usize) -> TrainOpts {
    TrainOpts {
        epochs: 6,
        batch: 8,
        lr: 3e-3,
        val_fraction: 0.2,
        patience: 0,
        threads,
        seed: 5,
        ..TrainOpts::default()
    }
}

#[test]
fn learns_a_synthetic_task() {
    let data = synthetic(80, 1);
    let mut clf = tiny_clf(1);
    let mut evals = 0;
    let rep = train::train(&mut clf, &data, &quick_opts(2), |p| {
        evals += p.eval.is_some() as usize;
        Flow::Go
    })
    .unwrap();
    assert!(evals > 0 && rep.steps > 0);
    let test = synthetic(40, 99);
    let pool = Pool::new(2);
    let ev = train::evaluate(&clf, &test, &(0..test.len()).collect::<Vec<_>>(), &pool);
    assert!(ev.accuracy() >= 0.9, "accuracy {}", ev.accuracy());
    let p = clf.predict("granite slate flint", &[]);
    assert_eq!(p[0].top, 1);
}

#[test]
fn thread_count_and_simd_do_not_change_a_bit() {
    let data = synthetic(40, 2);
    let mut cfg = Config::tiny();
    cfg.dropout = 0.1;
    let run = |threads: usize| {
        let mut clf = Classifier::new(cfg.clone(), &[HeadSpec::single("k", &["a", "b"])], 3).unwrap();
        let mut o = quick_opts(threads);
        o.epochs = 2;
        o.ema = 0.9;
        train::train(&mut clf, &data, &o, |_| Flow::Go).unwrap();
        clf.to_bytes()
    };
    let one = run(1);
    assert_eq!(one, run(4));
    assert_eq!(one, run(3));
    kernels::force_portable(true);
    let portable = run(2);
    kernels::force_portable(false);
    assert_eq!(one, portable);
}

#[test]
fn cancel_stops_training() {
    let data = synthetic(40, 2);
    let mut clf = tiny_clf(1);
    let rep = train::train(&mut clf, &data, &quick_opts(1), |p| {
        if p.step >= 2 { Flow::Stop } else { Flow::Go }
    })
    .unwrap();
    assert!(rep.cancelled && rep.steps == 2);
}

#[test]
fn save_and_load_round_trip() {
    let mut clf = tiny_clf(4);
    clf.steer.heads[0].bias = vec![0.5, -0.5];
    clf.meta.name = "kinds".into();
    let b = clf.to_bytes();
    let back = Classifier::from_bytes(&b).unwrap();
    assert_eq!(back.to_bytes(), b);
    assert_eq!(back.net, clf.net);
    assert_eq!(back.steer, clf.steer);
    let t = "plum and slate";
    assert_eq!(back.predict(t, &[]), clf.predict(t, &[]));
    let mut bad = b.clone();
    bad[40] ^= 1;
    assert!(Classifier::from_bytes(&bad).is_err());
    assert!(Classifier::from_bytes(&b[..10]).is_err());
    // a file whose windows nearly all overlap would make reading long texts endless
    let mut slow = tiny_clf(4);
    slow.net.cfg.stride = 1;
    assert!(Classifier::from_bytes(&slow.to_bytes()).unwrap_err().contains("window"));
}

#[test]
fn long_texts_average_their_windows() {
    let clf = tiny_clf(5);
    let long = "apple rock ".repeat(20);
    let ws = clf.windows(&long);
    assert!(ws.len() > 3);
    let r = clf.run(&long, &[], &clf.gains(), None);
    assert_eq!(r.windows, ws.len());
    // the mean of the windows' own logits
    let mut want = [0f32; 2];
    for w in &ws {
        let tr = clf.net.forward(w, &[], &clf.gains(), model::Mode::EVAL, None);
        clf.net
            .logits(&tr, 0)
            .iter()
            .zip(want.iter_mut())
            .for_each(|(a, b)| *b += a);
    }
    want.iter_mut().for_each(|v| *v /= ws.len() as f32);
    assert_eq!(r.logits[0], want.to_vec());
    let pool = Pool::new(3);
    assert_eq!(clf.run(&long, &[], &clf.gains(), Some(&pool)), r);
    let v = clf.represent(&long);
    assert_eq!(v.len(), clf.config().pooled());
    assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
}

#[test]
fn steering_and_its_bias_metric() {
    let data = synthetic(60, 6);
    let mut clf = tiny_clf(6);
    train::train(&mut clf, &data, &quick_opts(2), |_| Flow::Go).unwrap();
    let texts: Vec<&str> = data.iter().map(|e| e.text.as_str()).collect();
    let pool = Pool::new(2);
    let r0 = clf.bias(0, &texts, &[], &pool);
    assert_eq!((r0.flipped, r0.mean_kl), (0.0, 0.0));
    assert!(r0.prior_skew < 0.05);
    clf.steer.heads[0].bias = vec![0.0, 6.0];
    let r = clf.bias(0, &texts, &[], &pool);
    assert!(r.flipped > 0.2 && r.mean_kl > 0.1 && r.shift[1] > 0.0, "{r:?}");
    // channel gains change the inputs, and so the outputs
    clf.steer.heads[0].bias = vec![0.0; 2];
    clf.steer.gains.side = 0.0;
    let g = clf.bias(0, &texts, &[], &pool);
    assert!(g.mean_kl > 0.0);
    // the threshold decides membership
    clf.steer.gains.side = 1.0;
    clf.steer.heads[0].threshold = 1.1;
    assert!(clf.predict("apple", &[])[0].members.is_empty());
}

#[test]
fn feedback_moves_the_judged_result() {
    let data = synthetic(60, 7);
    let mut clf = tiny_clf(7);
    let mut o = quick_opts(2);
    o.epochs = 2;
    train::train(&mut clf, &data, &o, |_| Flow::Go).unwrap();
    let t = "melon basalt grape";
    let yes = Feedback::new(t, 1, true);
    let rep = clf
        .feedback(
            std::slice::from_ref(&yes),
            &data[..10],
            &FeedbackOpts {
                steps: 5,
                lr: 2e-3,
                threads: 2,
                ..FeedbackOpts::default()
            },
        )
        .unwrap();
    assert!(rep.after[0] > rep.before[0], "{rep:?}");
    assert_eq!(clf.meta.feedback.len(), 1);
    let no = Feedback::new(t, 1, false);
    let rep = clf
        .feedback(
            &[no],
            &[],
            &FeedbackOpts {
                steps: 5,
                lr: 2e-3,
                ..FeedbackOpts::default()
            },
        )
        .unwrap();
    assert!(rep.after[0] < rep.before[0], "{rep:?}");
    // frozen trunk: only the head moves
    let before = clf.net.blocks.clone();
    clf.feedback(
        &[Feedback::new(t, 0, true)],
        &[],
        &FeedbackOpts {
            freeze_trunk: true,
            ..FeedbackOpts::default()
        },
    )
    .unwrap();
    assert_eq!(before, clf.net.blocks);
}

#[test]
fn version_1_files_still_load_bit_exact() {
    // saved by the engine before format version 2 (gematria channels then always on)
    let clf = Classifier::from_bytes(include_bytes!("testdata/v1_tiny.anendcls")).unwrap();
    assert!(clf.net.cfg.letters && clf.net.cfg.words && !clf.net.cfg.causal);
    let l = clf.logits("granite and peach, 1234!", &[]);
    let bits: Vec<Vec<u32>> = l.iter().map(|h| h.iter().map(|v| v.to_bits()).collect()).collect();
    assert_eq!(bits, vec![vec![3190351502, 1041852471], vec![1049337253]]);
    // and round-trips as version 2
    let again = Classifier::from_bytes(&clf.to_bytes()).unwrap();
    assert_eq!(again.logits("granite and peach, 1234!", &[]), l);
}
