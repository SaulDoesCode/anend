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

## Phase 4 (in progress, 14 Oct): general models, generation, critics

The user (14 Oct): transformers are general purpose, not gematria transformers; the byte
sums are a separate measure; a high-level declarative spec is the primary interface;
sizes are honest (tiny is for tests only).

Done (tested; `cargo test -p anend-core classify` ~12 s in debug):

| area | where | tests |
|---|---|---|
| byte sums (byte + 1, 256 − byte, digit sums, word sums), standalone, not a model input | `core/src/measure.rs` | `measure::tests` |
| feature channels optional: `Config::{letters, words, side}` (`Config::gematria(on)`); files saved before keep theirs on | model.rs, io.rs | `version_1_files_still_load_bit_exact` (fixture `testdata/v1_tiny.anendcls`) |
| NUM tokens (`text::NUM`, `Window::push_numbers`, `Config::numbers`): row + value × learned direction | text.rs, model.rs | in the generative gradient check |
| causal attention (`Config::causal`, row by row via `attend_row`), causal word channels | model.rs, text.rs | gradient check; `causal_channels_and_numbers` |
| generative head (`GenSpec`, `Net::{generator, gen_w, gen_b}`, `gen_logits`, `backward_gen`), next byte over 256 + END | model.rs | `generative_gradients_match_finite_differences` |
| `Decoder`: KV cache, bit-equal to the full pass, restarts past `max_len` with the last half window | model.rs | `decoder_equals_the_full_pass` |
| multi-output: heads + generative head share a trunk; `Example::and(head, target)`, `HeadSpec::weight` / `GenSpec::weight` loss weights; `Target::{Next{from}, Avoid{from}}`; `Example::raw` bytes; `Evaluation::bits_per_byte`; `train::{tune, nll, gen_loss}` | train.rs | `multi_output_models_train_heads_jointly` |
| file format v2 (adds generator, flags; v1 reads) | io.rs | round trip in the tests above |
| sampling (greedy, temperature, top-k, top-p, max length, stop byte, END), keyed by (seed, index) | generate.rs | `learns_sequences_and_samples_deterministically` (same seed same bytes, threads 2 = 3) |
| critics (`Judge` for `Classifier`, `generate::Node` of a `Graph`, `classifiers::Stack`; `Want::{Class, Not, Near, Range}`), `Constraints` (gates must/must-not, sum ranges over `Measure::{Bytes, BytesReversed, Ordinal, Reversed, Length}`, length, contains/excludes), `draw` / `generate` (rank), `Guidance` (top-k next bytes reweighted by critics), `rewards` (score − mean), `reinforce` (reward-weighted likelihood + unlikelihood, replay), `Rewarded::{yes, no}` | generate.rs | `critics_rank_constrain_guide_and_reinforce` |
| numbers as fixed-width text (`NumberCodec`), pictures as patch-codebook bytes (`Codebook::learn` integer k-means, `ROW` = 255, `encode/decode`), `png` writer (stored deflate), `toy` pictures | codec.rs | `codec::tests` |

| declarative `ModelSpec` (versioned serde/JSON: input text with opt-in `letters`/`word_sums`/`side` features, bytes, image (patch codebook), numbers (fixed-width text + dense head inputs); trunk preset `test`/`compact`/`small`/`base`/`auto` or sizes, causal by default with a generate head; heads classify/multi/regress/generate with loss weights; data queries `list`/`tagged`/`pairs`/`inline` over a `Source` (`MemSource`); critics by model name; constraints; sampling), `build / train / Model::run / spec::generate`, `Model::{to_bytes, from_bytes, from_classifier}` (spec + codebook kept in `Meta::spec`), checks with clear errors | spec.rs, `docs/models.md` | `spec::tests` (classifier, steered generator, number regressor, picture generator, error messages) |
| machine benchmark: `bench::{measure, load_or_measure, suggest}` (training throughput over a size ladder, forward cost by sequence length); the spec's `"auto"` preset uses it | bench.rs | `suggests_within_budget` |

Examples (release, this machine: 4 vCPU, AVX2):

- `cargo run --release -p anend-core --example generate [scale] [authors]`: author critic (359 k
  weights, 71 % validation accuracy after 51 s) and one 670 k-weight generator per author (390
  steps, ~55 s, ~14 k tokens/s, 2.1–2.65 held-out bits/byte). The critic agrees a 120-byte
  passage is the right author's: plain sampling 61 %, the critic's top 3 of 12 89 %, guided
  sampling 72 %, after one reinforce round 74 % (mean over 6 authors; per author 8–100 %,
  Melville lowest). Drawing 12 candidates takes 0.1 s, guided 0.8–1.0 s, reinforce ~5 s.
- `cargo run --release -p anend-core --example pictures [out-dir] [epochs]`: 192 toy 64×64
  pictures → 8×8 patches, 128 codes, 72 tokens each; 870 k weights, 720 steps in 69 s, 0.87
  bits per code held out; 16 samples in 0.25 s, written as PNG contact sheets. Forward cost of
  one 64×64 picture by tokenisation (d 128, 4 blocks): 16×16 patches 20 tokens 1.5 ms, 8×8 72
  tokens 4.9 ms, 4×4 272 tokens 28 ms, 2×2 1056 tokens 263 ms, a byte a pixel 4160 tokens
  5.3 s (per-token cost ×16 from 72 to 4160 tokens: attention). Samples show the shapes and
  backgrounds but are often blank or fragmentary; more data, steps and a critic would help.

Left:
- NUM tokens are in the engine but not fed from `Example` (numbers reach models as text and as
  dense head inputs).
- ~~The entry store's `Source`, the UI editing specs, hub ops for specs and generation~~:
  done 19 Oct: `ops::models` (the save's lists by name or key, tags, pairs, snapshotted when
  training starts; `models.*` in docs/app-api.md, the same on the desktop and the hub),
  stored by `crate::specs` in the save's `.models` file and bundles, and the models view's
  spec models tab. Left: the scripting language writing specs; picture data from the save.
- A patch-embedding (continuous) input for picture classifiers; images now always go through
  the codebook.
- Guidance is per byte and judges partial text; it helped 4 authors of 6 here. Sentence-level
  guidance (judge at word or sentence ends) is the obvious next try.
- `git`: commit 79b25ff on the branch duplicates another agent's hub commit (an `--amend` on a
  shared working tree); harmless (identical changes, merged cleanly), noted here.

## Pretrained open models (done in core, 18 Oct; synthetic checkpoints only)

| area | where | tests |
|---|---|---|
| safetensors: header, F32/F16/BF16/F64 → f32 (halves exact), shards by `model.safetensors.index.json`, tensors read on demand, a writer | safetensors.rs | every half/bfloat16 value round trips; shards; bad headers refused in words |
| regex subset for pre-tokenizers (alternation, groups, `(?i:)`, lookahead, quantifiers, classes, `\p{L}`/`\p{N}`…), leftmost-first backtracking | pattern.rs | GPT-2 and Llama 3 patterns split as the reference; unsupported constructs named |
| BPE from `tokenizer.json`: byte-level and SentencePiece style (byte fallback, ▁), merges as strings or pairs, added/special tokens (lstrip/rstrip), normalizers, pre-tokenizers (ByteLevel, Split, Digits, Metaspace), decoders; `Lexicon` (start/end/stop tokens) | bpe.rs | hand-made fixtures of both kinds: encode, decode, specials, every byte round trips, unsupported parts named |
| engine: grouped key/value heads (`Config::kv_heads`), q/k/v biases (`qkv_bias`), rope scaling (`linear`, `llama3`), a token vocabulary (`vocab_size`, `Net::lexicon`) through windows, crops, losses, evaluation, `Decoder` (`push_token`, `from_tokens`), sampling, guidance, the file format (tokenizer saved; larger shape limits for these models); growth refuses head changes there; byte models bit for bit as before (old fixtures, saved bytes, gradients hashed against the previous commit) | model.rs, train.rs, generate.rs, io.rs, grow.rs | model gradient checks; decoder = full pass with GQA |
| `config.json` of llama / mistral / qwen2 / qwen3 → `Config`, weights mapped (transposed into `[in × out]`, q/k/v side by side, tied heads copied), unsupported fields named; spec `"from": {"pretrained": dir}` | pretrained.rs, spec.rs | four synthetic architectures (GQA, biases + BF16 shards, QK-norm + F16 + odd head_dim, SentencePiece + MHA) match an f64 reference from the transformers code; generate with critics and guidance (threads 1 = 2), fine-tune lowers NLL, save/load equal; a spec trains, runs, generates with a critic model and saves |

Example for the user's machine: `core/examples/pretrained.rs` (SmolLM2-135M; download
command in its doc). Left: see docs/next.md "Lifting the model ceiling".

## The GPU backend (done, 18 Oct; tested on llvmpipe, benchmarked there only)

`core/src/classify/gpu/` behind the cargo feature `gpu` (wgpu 30, WGSL compute shaders; the
default build and wasm do not pull wgpu: `cargo tree -p anend-core | grep wgpu` is empty).

| area | where | tests (`cargo test -p anend-core --features gpu gpu::`) |
|---|---|---|
| device, buffers, `Run` (dispatches recorded, one submission, reads), shader source with exact/fast preludes | gpu/mod.rs | — |
| kernels: tiled GEMM 64×64 (strided A and B, batched with grouped B, causal row limits, accumulate), per-output GEMM for few rows, 8-lane dot rows, RMSNorm fwd/back + column sums, softmax fwd (causal rows)/back, SwiGLU fwd/back, RoPE fwd/back, causal scores, strided copies, elementwise ops; `math::exp`/sigmoid ported operation for operation | gpu/kernels.wgsl | `kernels_match_the_cpu` |
| `Precision::Exact` (products laundered through an XOR with a uniform zero so they cannot fuse; ÷ and √ correctly rounded in integer arithmetic) and `Fast` (plain operators) | gpu/mod.rs | both probed |
| exactness probe: every kernel on device and CPU, values differing, max ulps, max relative difference | gpu/probe.rs | `kernels_match_the_cpu` |
| `GpuNet`: the trunk resident on the device (grouped KV heads, QKV biases, QK-norm, causal and bidirectional attention, the generative head); embedding, pooling and classification heads on the CPU | gpu/net.rs | `trunk_and_heads_match_the_cpu` |
| `GpuDecoder`: KV cache on the device, `Decoder`'s restarts | gpu/net.rs | `decoder_matches_the_cpu` |
| training: forward with dropout masks, backward through the trunk on the device (per-example sums in example order, like `batch_grad`), embedding/heads/losses/AdamW on the CPU | gpu/train.rs, device.rs | `training_matches_the_cpu` |
| `device::Device::{Cpu, Gpu, GpuFast}`: `TrainOpts::device`, `GenOpts::device`, spec `"device"`; CPU default; a build without the feature refuses with a message | device.rs, train.rs, generate.rs, spec.rs | `devices_parse_and_the_gpu_needs_the_feature` |

The probe on llvmpipe (LLVM 20, Mesa 25.2, lavapipe Vulkan; the container has no GPU):
**every kernel bit-exact in both precisions** (llvmpipe does not fuse or approximate), and
whole forward passes, decoding and 2 epochs of training with dropout give the CPU's bits
(the tests assert equality wherever the probe reports exact, a tolerance elsewhere). One
fix the probe found: the shader compiler simplified exp's `(x + 1.5·2^23) − 1.5·2^23`
rounding trick away; `keep()` (XOR with the uniform zero) stops that in both precisions.

`cargo run --release -p anend-core --features gpu --example gpu_bench -- 1 3` on this
container (llvmpipe is software rendering on the same 4 vCPUs, so it measures overhead,
not a GPU): small (0.9 M weights, 256 tokens) forward 10 842 tokens/s CPU vs 560 GPU,
decoding 5090 vs 86, training 4829 vs 175; every GPU row matched the CPU's logits bit
for bit. The numbers that matter come from running it on the user's hardware.

Left: per-dispatch bind groups and a submission + read per decoding step cost the most on
small models (batch decoding several candidates per submission, and bind group caching,
would cut it); weights are re-uploaded after every optimizer step (AdamW on the device
would avoid it); a subgroup/cooperative-matrix GEMM for large models (still one chain per
output for exactness, so no split-k); classification-only inference (`Model::run`,
`predict`) still on the CPU; pictures/patch paths untouched; the browser (WebGPU) build
needs an async device request and a wasm feature set.

## Next (not done)

- ~~the desktop app's classifiers view~~: done for the Bevy app (retired), and again for
  the Dioxus app on 18 Oct (`app/src/views/classifiers.rs`, the memory view's colouring
  and classifier search) over one implementation of the ops (`core/src/ops/classify.rs`)
  for the desktop and the hub. `classify.unfeedback` undoes a yes/no bit for bit.
- JS port of inference for offline use; search filter panel; default classifiers for
  new saves; filter presets as lists.
- GPU backend: done (above); what is left is listed there.
- Row-parallel matmuls are used only for single long windows at inference; training is
  data-parallel over examples.
- JS port of math/kernels with Math.fround.
