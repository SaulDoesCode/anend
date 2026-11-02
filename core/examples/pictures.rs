//! A picture generator from a declarative spec: toy 64×64 pictures (discs, squares,
//! crosses, stripes) are cut into 8×8 patches, each patch one of 128 learned codes (one
//! token), so a picture is 72 tokens. A causal transformer learns to write code
//! sequences; sampled sequences decode back to pictures, written as PNGs.
//!
//! It also measures what limits picture size here: one forward pass at each sequence
//! length a patch size implies (attention grows with the square of the length), and
//! training throughput, so the limit is shown with numbers.
//!
//! `cargo run --release -p anend-core --example pictures [out-dir] [epochs]`

use std::time::Instant;

use anend_core::classify::codec::{self, Image, png, toy};
use anend_core::classify::model::{Config, Gains, HeadSpec, Mode, Net, Pooling};
use anend_core::classify::spec::{self, Answer, MemSource, ModelSpec, Value};
use anend_core::classify::{Flow, generate, kernels, pool, text};

fn sheet(ims: &[Image], cols: usize, scale: usize) -> Image {
    let (w, h) = (ims[0].width * scale, ims[0].height * scale);
    let rows = ims.len().div_ceil(cols);
    let pad = 4;
    let mut out = Image::new(cols * (w + pad) + pad, rows * (h + pad) + pad, [128, 128, 128]);
    for (i, im) in ims.iter().enumerate() {
        let s = im.scaled(scale);
        let (ox, oy) = (pad + (i % cols) * (w + pad), pad + (i / cols) * (h + pad));
        for y in 0..h {
            for x in 0..w {
                out.set(ox + x, oy + y, s.get(x, y));
            }
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(
        args.get(1)
            .cloned()
            .unwrap_or_else(|| std::env::temp_dir().display().to_string()),
    );
    let epochs: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    std::fs::create_dir_all(&out).expect("out dir");
    println!("{} threads, {} kernels", pool::available(), kernels::simd_name());

    // what sequence length costs: one forward pass of a 128-wide, 4-block trunk
    println!("\nforward pass cost by picture tokenisation (64×64 picture, d_model 128, 4 blocks):");
    let base = Config {
        pooling: Pooling::Mean,
        causal: true,
        ..Config::small().gematria(false)
    };
    for (what, len) in [
        ("16×16 patches", 4 * 5),
        ("8×8 patches", 8 * 9),
        ("4×4 patches", 16 * 17),
        ("2×2 patches", 32 * 33),
        ("1 byte a pixel (palette)", 64 * 65),
    ] {
        let net = Net::new(
            Config {
                max_len: len + 2,
                ..base.clone()
            },
            &[HeadSpec::single("k", &["a"])],
            1,
        )
        .unwrap();
        let w = text::window(&vec![7u8; len]);
        let t = Instant::now();
        let mut n = 0;
        while n == 0 || t.elapsed().as_secs_f64() < 0.5 {
            drop(net.forward(&w, &[], &Gains::default(), Mode::EVAL, None));
            n += 1;
        }
        let ms = t.elapsed().as_secs_f64() * 1e3 / n as f64;
        println!(
            "  {what:>26}: {len:>5} tokens, {ms:>9.2} ms per pass, {:>7.1} µs per token",
            ms * 1e3 / len as f64
        );
    }

    let train_ims: Vec<Value> = (0..192).map(|i| Value::Image(toy(64, i % 4, i as u64))).collect();
    let spec = ModelSpec::from_json(&format!(
        r#"{{
          "name": "toy pictures",
          "input": {{"type": "image", "width": 64, "height": 64, "patch": 8, "codes": 128}},
          "trunk": {{"preset": "small", "dropout": 0.05}},
          "heads": [{{"name": "draw", "kind": "generate"}}],
          "data": [{{"from": "list", "list": "toys", "head": "draw"}}],
          "sampling": {{"temperature": 0.9, "top_p": 0.95, "max_len": 72, "seed": 1}},
          "train": {{"epochs": {epochs}, "batch": 16, "lr": 0.002, "val_fraction": 0.05, "patience": 0}}
        }}"#
    ))
    .unwrap();
    let src = MemSource::default().with_list("toys", train_ims.clone());
    let t = Instant::now();
    let (m, rep) = spec::train(None, &spec, &src, None, |_| Flow::Go).unwrap();
    println!(
        "\ntrained {} weights for {} steps in {:.1}s ({:.0} tokens/s); held-out {:.2} bits per code",
        m.clf.net.weight_count(),
        rep.steps,
        t.elapsed().as_secs_f64(),
        rep.tokens_per_sec,
        rep.eval.as_ref().and_then(|e| e.bits_per_byte).unwrap_or(f64::NAN)
    );
    let cb = m.codebook.as_ref().unwrap();
    let originals: Vec<Image> = train_ims[..16]
        .iter()
        .map(|v| match v {
            Value::Image(im) => im.clone(),
            _ => unreachable!(),
        })
        .collect();
    let through: Vec<Image> = originals.iter().map(|im| cb.decode(&cb.encode(im), 64, 64)).collect();
    let t = Instant::now();
    let samples: Vec<Image> = (0..16)
        .map(|i| {
            let (b, _) = generate::sample(&m.clf, &[], &spec.sampling, i);
            cb.decode(&b, 64, 64)
        })
        .collect();
    println!("sampled 16 pictures in {:.2}s", t.elapsed().as_secs_f64());
    // `run` continues a picture's codes: here from an empty prompt via the generate head
    if let Ok(a) = m.run(&Value::Image(toy(64, 0, 999))) {
        assert!(a.values().any(|x| matches!(x, Answer::Generated(_))));
    }
    for (name, ims) in [
        ("pictures-training.png", &originals),
        ("pictures-codebook.png", &through),
        ("pictures-sampled.png", &samples),
    ] {
        let p = out.join(name);
        std::fs::write(&p, png(&sheet(ims, 4, 2))).unwrap();
        println!("wrote {}", p.display());
    }
    let _ = codec::ROW;
}
