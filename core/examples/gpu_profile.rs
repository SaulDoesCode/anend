//! Where the time goes when classifiers read texts on the GPU: per text, the dispatches,
//! submissions, bind groups and buffers, and the time split between making and writing
//! buffers, encoding and submitting, waiting for the device (its compute and the sync),
//! reading results back, and the rest on the CPU (embedding, windows, recording), next to
//! the CPU reading the same texts. On a software renderer (llvmpipe) only the proportions
//! and the counts say much: its "device" is the same CPU.
//!
//! ```text
//! cargo run --release -p anend-core --features gpu --example gpu_profile [preset] [texts] [bytes each, 0: 20 to 400]
//! ```

use std::time::Instant;

use anend_core::classifiers::preset;
use anend_core::classify::device::{Device, Runner};
use anend_core::classify::gpu::PROFILE;
use anend_core::classify::{Classifier, HeadSpec, Pool, pool};

fn text(i: usize, fixed: usize) -> String {
    const W: [&str; 8] = ["river", "stone", "copper", "gear", "heron", "valve", "moss", "lamp"];
    let mut h = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let len = if fixed > 0 { fixed } else { 20 + (h % 380) as usize };
    let mut s = String::new();
    while s.len() < len {
        h ^= h << 13;
        h ^= h >> 7;
        h ^= h << 17;
        s.push_str(W[(h % 8) as usize]);
        s.push(' ');
    }
    s
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let size = args.get(1).map(String::as_str).unwrap_or("compact");
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(500);
    let clf = Classifier::new(preset(size).unwrap(), &[HeadSpec::single("p", &["a", "b"])], 3).unwrap();
    let fixed: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let texts: Vec<String> = (0..n)
        .map(|i| {
            let mut t = text(i, fixed);
            if fixed > 0 {
                t.truncate(fixed);
            }
            t
        })
        .collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let p = Pool::new(pool::available());
    let t = Instant::now();
    let cpu = clf.predict_many(&refs, &[], &p);
    let cpu_s = t.elapsed().as_secs_f64();
    println!("{size}, {n} texts; CPU: {:.0} texts/s", n as f64 / cpu_s);
    for dev in [Device::Gpu, Device::GpuFast] {
        let r = match Runner::open(dev, &clf.net) {
            Ok(Some(r)) => r,
            Ok(None) | Err(_) => {
                println!("no GPU");
                return;
            }
        };
        // a warm-up pass compiles the pipelines
        let warm: Vec<&[u8]> = refs.iter().take(8).map(|t| t.as_bytes()).collect();
        let zero = vec![0f32; clf.net.cfg.extra];
        let ex: Vec<&[f32]> = warm.iter().map(|_| zero.as_slice()).collect();
        r.text_runs(&clf.net, &warm, &ex, &clf.gains());
        PROFILE.take();
        let bytes: Vec<&[u8]> = refs.iter().map(|t| t.as_bytes()).collect();
        let ex: Vec<&[f32]> = bytes.iter().map(|_| zero.as_slice()).collect();
        let t = Instant::now();
        let out = r.text_runs(&clf.net, &bytes, &ex, &clf.gains());
        let s = t.elapsed().as_secs_f64();
        let prof = PROFILE.take();
        let same = out
            .iter()
            .zip(&cpu)
            .filter(|((l, _), c)| clf.read(l)[0].probs == c[0].probs)
            .count();
        println!(
            "{dev:?}: {:.0} texts/s ({:.2}x the CPU), {same}/{n} equal to the CPU's bits",
            n as f64 / s,
            cpu_s / s
        );
        let get = |k: &str| prof.iter().find(|(n, _)| *n == k).map_or(0, |x| x.1);
        let ms = |k: &str| get(k) as f64 / 1e6;
        println!(
            "  per text: {:.1} dispatches, {:.1} bind groups, {:.2} buffers, {:.3} submits",
            get("dispatches") as f64 / n as f64,
            get("bind_groups") as f64 / n as f64,
            get("buffers") as f64 / n as f64,
            get("submits") as f64 / n as f64
        );
        let total = s * 1000.0;
        let other = total - ms("ns_upload") - ms("ns_encode") - ms("ns_wait") - ms("ns_readback");
        println!(
            "  ms: total {total:.0} = upload (buffers made and written) {:.0}, encode+submit {:.0}, wait for the device (compute and sync) {:.0}, readback {:.0}, the rest (embedding, recording, heads) {other:.0}",
            ms("ns_upload"),
            ms("ns_encode"),
            ms("ns_wait"),
            ms("ns_readback")
        );
        let ops = std::mem::take(&mut *anend_core::classify::gpu::OP_NS.lock().unwrap());
        if !ops.is_empty() {
            let all: u64 = ops.values().map(|v| v.1).sum();
            let mut v: Vec<_> = ops.into_iter().collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1.1));
            for (k, (n, ns)) in v {
                println!("    {k:<16} {n:>7} dispatches {:>5.1}%", 100.0 * ns as f64 / all as f64);
            }
        }
    }
}
