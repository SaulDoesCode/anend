//! A pretrained open model in the engine, steered by a classifier of your own.
//!
//! Download a Llama-family model folder first (model hubs are reachable from your
//! machine). Recommended small one: SmolLM2-135M (Apache 2.0, 135 M weights, ~270 MB in
//! bf16, about 0.5 GB of RAM as f32 here):
//!
//! ```sh
//! pip install -U "huggingface_hub[cli]"
//! huggingface-cli download HuggingFaceTB/SmolLM2-135M \
//!     config.json generation_config.json tokenizer.json model.safetensors \
//!     --local-dir models/SmolLM2-135M
//! # or without Python:
//! mkdir -p models/SmolLM2-135M && cd models/SmolLM2-135M && for f in config.json \
//!   generation_config.json tokenizer.json model.safetensors; do \
//!   curl -LO https://huggingface.co/HuggingFaceTB/SmolLM2-135M/resolve/main/$f; done
//! ```
//!
//! Then `cargo run --release -p anend-core --example pretrained -- models/SmolLM2-135M
//! ["prompt"] [--tune]`. Bigger ones of the same family work the same way
//! (Qwen/Qwen2.5-0.5B, Qwen/Qwen3-0.6B, TinyLlama/TinyLlama-1.1B-Chat-v1.0,
//! meta-llama/Llama-3.2-1B, mistralai/Mistral-7B-v0.1, …), given the memory: weights
//! are held as f32, 4 bytes each.
//!
//! What it does: loads the model (timing it), writes a greedy continuation of the
//! prompt (tokens per second), trains a small byte-level author classifier on
//! `core/corpus/authors` (Austen against Melville, about a minute), draws candidate
//! continuations from the pretrained model and ranks them by the classifier as a
//! critic, then samples with the critic guiding each step. With `--tune` it also
//! fine-tunes the pretrained model a few steps on Austen and reports her passages'
//! bits per token before and after.

use std::path::Path;
use std::time::Instant;

use anend_core::classify::generate::{self, Constraints, Critic, GenOpts, Guidance, Sampling, Want};
use anend_core::classify::pretrained::{self, Options};
use anend_core::classify::{Classifier, Config, Example, Flow, HeadSpec, TrainOpts, train};

fn passages(text: &str, len: usize, step: usize) -> Vec<String> {
    let mut v = Vec::new();
    for region in text.split("\n* * *\n") {
        let chars: Vec<char> = region.trim().chars().collect();
        let mut at = 0;
        while at + len <= chars.len() {
            v.push(chars[at..at + len].iter().collect());
            at += step;
        }
    }
    v
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tune = args.iter().any(|a| a == "--tune");
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let Some(dir) = pos.first() else {
        return Err("usage: pretrained <model-dir> [prompt] [--tune] (see the file's doc for a download)".into());
    };
    let prompt = pos
        .get(1)
        .map_or("It is a truth universally acknowledged that", |s| s.as_str());

    let t = Instant::now();
    let folder = pretrained::Folder::open(Path::new(dir.as_str()))?;
    let a = folder.arch.clone();
    let mut lm = folder.model(&Options {
        max_len: Some(512),
        ..Options::default()
    })?;
    drop(folder);
    println!(
        "{}: {} blocks, width {}, {} heads ({} key/value), head {}, ff {}, vocab {}, {:.1} M weights; loaded in {:.1} s",
        a.model_type,
        a.layers,
        a.hidden,
        a.heads,
        a.kv_heads,
        a.head_dim,
        a.ff,
        a.vocab,
        lm.net.weight_count() as f64 / 1e6,
        t.elapsed().as_secs_f64()
    );

    let t = Instant::now();
    let (out, _) = generate::sample(&lm, prompt.as_bytes(), &Sampling::greedy(160), 0);
    let toks = lm
        .net
        .lexicon
        .as_ref()
        .unwrap()
        .tokenizer
        .encode(&String::from_utf8_lossy(&out))
        .len();
    let secs = t.elapsed().as_secs_f64();
    println!(
        "\ngreedy ({toks} tokens, {:.1} tokens/s):\n{prompt}{}\n",
        toks as f64 / secs,
        String::from_utf8_lossy(&out)
    );

    // a critic of our own: Austen (1) against Melville (0), on bytes
    let read =
        |n: &str| std::fs::read_to_string(format!("core/corpus/authors/{n}.txt")).map_err(|e| format!("{n}: {e}"));
    let (austen, melville) = (read("austen")?, read("melville")?);
    let (pa, pm) = (passages(&austen, 160, 120), passages(&melville, 160, 120));
    let mut data: Vec<Example> = pa.iter().map(|p| Example::class(p, 1)).collect();
    data.extend(pm.iter().map(|p| Example::class(p, 0)));
    let mut critic = Classifier::new(
        Config {
            max_len: 192,
            stride: 95,
            ..Config::small().gematria(false)
        },
        &[HeadSpec::single("author", &["melville", "austen"])],
        1,
    )?;
    let t = Instant::now();
    let rep = train::train(
        &mut critic,
        &data,
        &TrainOpts {
            epochs: 3,
            val_fraction: 0.1,
            ..TrainOpts::default()
        },
        |_| Flow::Go,
    )?;
    println!(
        "critic: {} steps in {:.0} s, validation accuracy {:.0} %",
        rep.steps,
        t.elapsed().as_secs_f64(),
        100.0 * rep.eval.as_ref().map_or(0.0, |e| e.accuracy())
    );

    let critics = [Critic::new(&critic, 0, Want::Class(1), 1.0)];
    let opts = GenOpts {
        prompt: prompt.as_bytes().to_vec(),
        sampling: Sampling {
            temperature: 0.9,
            top_p: 0.95,
            max_len: 160,
            seed: 7,
            ..Sampling::default()
        },
        candidates: 8,
        judge_prompt: false,
        ..GenOpts::default()
    };
    let show = |title: &str, cands: &[generate::Candidate], secs: f64| {
        println!("\n{title} ({secs:.1} s):");
        for c in cands.iter().take(3) {
            println!("  [austen {:.2}] {}", c.scores[0], c.text().replace('\n', " "));
        }
        let mean = cands.iter().map(|c| c.scores[0]).sum::<f32>() / cands.len().max(1) as f32;
        println!("  mean critic score {mean:.2} over {}", cands.len());
    };
    let t = Instant::now();
    let ranked = generate::generate(&lm, &critics, &Constraints::default(), &opts)?;
    show("ranked by the critic", &ranked, t.elapsed().as_secs_f64());
    let t = Instant::now();
    let guided = generate::generate(
        &lm,
        &critics,
        &Constraints::default(),
        &GenOpts {
            guidance: Some(Guidance {
                top_k: 4,
                strength: 2.0,
                every: 4,
            }),
            candidates: 4,
            ..opts.clone()
        },
    )?;
    show("guided by the critic", &guided, t.elapsed().as_secs_f64());

    if tune {
        let held: Vec<&String> = pa.iter().step_by(10).take(8).collect();
        let bits = |m: &Classifier| {
            let (n, c) = held.iter().fold((0.0, 0), |(n, c), p| {
                let (a, b) = train::nll(m, p.as_bytes(), 0);
                (n + a, c + b)
            });
            n / c.max(1) as f64 / std::f64::consts::LN_2
        };
        let before = bits(&lm);
        let tune_data: Vec<Example> = pa
            .iter()
            .skip(1)
            .step_by(5)
            .take(48)
            .map(|p| Example::generate(p))
            .collect();
        let t = Instant::now();
        train::train(
            &mut lm,
            &tune_data,
            &TrainOpts {
                epochs: 1,
                batch: 8,
                lr: 2e-5,
                warmup: 0.1,
                val_fraction: 0.0,
                ..TrainOpts::default()
            },
            |_| Flow::Go,
        )?;
        println!(
            "\nfine-tuned on {} Austen passages in {:.0} s: held-out bits per token {before:.3} -> {:.3}",
            tune_data.len(),
            t.elapsed().as_secs_f64(),
            bits(&lm)
        );
    }
    Ok(())
}
