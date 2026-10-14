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

## Phase 2 (done): in the save, the hub, the web app

- `core/src/classifiers.rs`: tables `classifiers` (name → `Definition` JSON: classes as
  list keys, optional "not" list, head, preset, steering, upstream inputs with coupling,
  training options, history of training/growth/enlist events, outputs, version),
  `classifier weights` (name → `ANENDCLS` bytes), `classifier feedback`
  ((name, n) → `FeedbackEntry`). `Store::{create_classifier, classifiers, classifier,
  stack, examples, prepare / Training::run / commit, train_classifier, steer_classifier,
  feedback_classifier, feedback_stack, enlist_outputs, candidates, delete_classifier}`;
  `Stack::{outputs, extras, bias, contributions}`. Presets tiny / compact (default, 89 k
  weights) / small / base. Bundles carry `classifiers` (weights base64), optional, so old
  bundles still read; merge and the forge leave them out. The Bevy app can use this API
  directly (training: prepare on the UI thread, `run` on a worker, `commit`).
- `hub/src/classify.rs`: the `classify.*` ops (docs/app-api.md), training as a
  background thread per classifier with progress/cancel.
- `hub/web/views/classify.js`: the classifiers view (Ctrl+7). Offline, the engine answers
  "classifiers need a hub for now" and the view says so; a browser save carries a hub's
  classifiers through untouched.
- Tests: core `classifiers::tests` (storage, bundle, stacks, enlist), hub
  `classify::tests::classifier_calls` (every op). Playwright run of the view: 3 lists ×
  50 memories, compact preset trains in ~8 s in a debug hub; 0.37 s in release
  (`cargo test --release -p anend-core timing -- --ignored --nocapture`).

## Phase 3 (in progress): buckets, filters, modules

Done in core (tested by `classifiers::phase3_tests::memo_filters_and_modules`):

- `core/src/memo.rs` (classifier buckets): tables `classifier memo` ((name, ref) -> `Row`:
  stamp, text hash, steered probs, top, members, embedding), `classifier buckets`
  ((name, class·16+band) -> refs; `UNCERTAIN` = 4095 for texts in no class),
  `classifier memo stamps`, `classifier indexing` (ref -> chosen classifier names).
  Stamp = hash of every stack classifier's name, version and steering, so training,
  feedback and steering invalidate; rows also keep a text hash. API on `Store`:
  `memo_fresh, memo_clear, memo_kept, memo_rows (lazy fill on the pool), memo_index
  (batches, progress cb, Flow::Stop), memo_count, memo_all, class_bucket(stack, class,
  bands), bucket_counts, memo_near(stack, row, By::Outputs|Embedding, except, limit),
  indexed_on / set_indexed_on / all_indexed / unindex_classifier, tints(stack, refs,
  pool)`; free fns `stamp, read, read_many, band, bucket_key, palette(k) -> [[u8;3]],
  tint(row)`. One pass gives outputs and embedding (`Graph::read_node`).
- `core/src/filters.rs`: `Expr` (JSON `op`: and/or/not/class/words/sum/list/tags/kind/
  preset), presets in table `filters` (`save_filter, filter_presets, filter_preset,
  delete_filter, check_expr, run_filter(expr, limit, scan, pool) -> Results,
  enlist_filter -> list "filter: <name>", note_filter_count, import_filters`);
  validators in table `classifier validators` (`Validator {by, class, min, mode:
  gate|weigh}`; `validators, set_validators, drop_validators_of, checked_rows,
  disagreements`). Bundles carry `filters` and each classifier's `validators` (optional).
- `core/src/clsmodule.rs`: `.anendcls` modules (magic `ANENDCLM`, manifest JSON + bundled
  `ANENDCLS` parts + FNV); `Store::export_module(name, ExportOpts{description, author,
  upstream})`, `read(bytes, max)` strict (sizes, names, steering, hash, widths, catch_unwind),
  `Store::import_module(&Module, ImportOpts{name, lists})` (classes -> lists by name or
  given keys; reuses classifiers with the same `weights_hash`), `summary(&Module)`.
- `classify/io.rs`: header shapes bounded and weight count checked before allocating;
  `Classifier::weights_hash`; reader wrapped in catch_unwind.
- delete_classifier clears memo, indexing choices and validators.

Hub ops done (`hub/src/classify.rs` `phase3`: classify.index/index_job/index_cancel/
buckets/bucket/near/tints/indexed/set_index/validators/disagreements/export/inspect/
import, filters.list/save/delete/run/enlist; docs/app-api.md). The web app uses them all (docs/next.md item 4, "web UI"). Left (hub sharing site,
web UI, screenshots): see docs/next.md item 4.

## Next (not done)

- ~~the desktop app's classifiers view~~: done (src/classify_ui.rs, filters_ui.rs, index_panel.rs, tint.rs; see docs/next.md item 4).
- JS port of inference for offline use; search filter panel; default classifiers for
  new saves; filter presets as lists.
- GPU backend behind `kernels` (see its module doc for bit-exactness notes).
- Row-parallel matmuls are used only for single long windows at inference; training is
  data-parallel over examples.
- JS port of math/kernels with Math.fround.
