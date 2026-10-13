# classify: status

Phase 1 (the engine) is done: a from-scratch, bit-deterministic transformer encoder
classifier library in `core/src/classify`, std only.

## Done and tested (`cargo test -p anend-core`, ~8 s in debug)

| area | where | tests |
|---|---|---|
| deterministic exp/ln/tanh/sigmoid/softplus (f32), exp64/ln64/sincos64, xoshiro RNG, keyed hashes | math.rs | against std within ulps |
| kernels: GEMM (portable + AVX2 6×16 packed tiles), dot/sum (8-lane rule), RMSNorm, softmax, exp/sigmoid slices, transpose, scratch buffer reuse, profiler | kernels.rs | AVX2 vs portable vs naive bit-for-bit; finite-difference grads for norm, softmax, SiLU |
| persistent thread pool (`Pool::run`), nested runs go serial | pool.rs | every index once |
| bytes + CLS/SEP/PAD tokens, gematria channels (letter value, word ordinal/reversed/Atbash sums mod 96, digit roots), 46-wide side vector, sliding windows | text.rs | channels, window starts |
| model: embedder + pre-norm blocks (RMSNorm, MHA with RoPE and optional QK-norm, SwiGLU, no linear biases) + final RMSNorm + pooling (CLS/mean/CLS+mean/attention) + heads (softmax, sigmoid, regression); hand-written backward | model.rs | whole-model finite differences for every tensor, every pooling, QK-norm on/off, extra-input gradient |
| training: AdamW (decoupled decay on matrices only), warmup + cosine, global clipping, dropout keyed by (seed, step, example, site), label smoothing, class weights or sampling, hash split, early stopping with best checkpoint, EMA of weights, progress callback with cancel, metrics | train.rs, metrics.rs | learns synthetic task; 1 vs 3 vs 4 threads and portable vs AVX2 give identical saved bytes; cancel; loss gradients; schedule; split |
| feedback per classifier (yes/no → Class/Not/Label target, loss on window-averaged logits, replay of kept feedback, newer supersedes older, optional frozen trunk) | train.rs | yes raises, no lowers, frozen trunk leaves blocks |
| steering (bias, temperature, threshold, channel gains) + bias metric (KL, flipped share, shift, prior skew, imbalance) | steer.rs, mod.rs | unit + trained classifier |
| growth: Net2Wider SwiGLU widening, exact head adding, exact identity deepening, importance pruning with accept/revert, `Grower` policy, `cycle` | grow.rs | outputs preserved (heads/deepen bit-exact), prune kept/reverted, policy ladder, cycle |
| composition: `Graph` of classifiers (extra inputs = upstream steered outputs), outputs → lists, end-to-end feedback with per-edge coupling, `contributions` (grad-norm share, input attribution) | compose.rs | lists, stack gradient vs finite differences, coupling 0 freezes upstream, determinism |
| file format `ANENDCLS` v1 (JSON header + f32 LE + FNV checksum) | io.rs | round trip, corruption detected |

Examples: `core/examples/authors.rs` (proof of concept), `core/examples/classify_bench.rs`.
Corpus: `core/corpus/authors/*.txt` (6 authors × ~47 KB, 10 regions each, README).

## Results on this machine (4 vCPU Xeon 2.8 GHz, AVX2)

- small preset (0.89 M weights) training: 2800 tokens/s on 1 thread, ~9600 on 4.
  base (5.0 M): 422 / 1362. Portable kernels: small 1214 tokens/s on 1 thread.
- authors: 86 % accuracy (macro-F1 0.86) on 240 unseen 300–600 character selections,
  6 authors, ~7 min training on 4 threads; nearest neighbour by `represent` is the same
  author 88 % of the time.

## Design decisions

- Determinism: two summation rules (products: ascending chain per output element;
  vector reductions: 8 lanes + fixed fold), no FMA, own transcendentals, keyed dropout,
  per-example gradients summed in example order. Threads only split independent work.
- RMSNorm/SwiGLU/RoPE/QK-norm per the user; no biases except heads. Widening of d_model is
  not offered (not function-preserving under RMSNorm gains).
- Sequences run at their true length, so the padding mask is structural (PAD exists as a
  token for future batched/GPU paths).
- Feedback uses the trained (unsteered) logits for the loss; steering is never trained.

## Next (not done)

- redb/table integration and UI (the user's part).
- GPU backend behind `kernels` (see its module doc for bit-exactness notes).
- Row-parallel matmuls are used only for single long windows at inference; training is
  data-parallel over examples.
- JS port of math/kernels with Math.fround.
