# Models: the declarative spec

A model is described by a **spec**: plain data (JSON today; the planned scripting
language and the UI will write the same structure) saying what goes in, how big the
trunk is, what comes out, what it learns from, and — for generators — which other models
judge what it writes. The engine underneath (`core/src/classify/`) is a general-purpose,
bit-deterministic transformer; nothing in it assumes gematria. Gematria channels are
optional input features you list, off unless you do.

```rust
use anend_core::classify::spec::{self, ModelSpec, MemSource, Value};
let spec = ModelSpec::from_json(json)?;               // parsed and checked
let (model, report) = spec::train(None, &spec, &source, profile, |_| Flow::Go)?;
let answers = model.run(&Value::Text("…".into()))?;   // head name → answer
let bytes = model.to_bytes();                          // weights + spec + codebook
```

- `build(&spec, profile)` makes an untrained model.
- `train(model, &spec, &source, profile, progress)` resolves the data queries through a
  `Source` (the save; `MemSource` for memory) and trains every head together.
- `run(&model, &input)` returns each head's answer by name: class probabilities with the
  top class and members, values, or what the generate head writes after the input.
- `spec::generate(&model, &models, &prompt, candidates, guidance)` draws candidates and
  ranks them by the spec's critics (other models, looked up by name in `models`).
- `Model::from_classifier(clf)` turns any existing classifier (and any classifier file,
  through `Model::from_bytes`) into a model with a spec, so the classifiers made so far
  are instances of this format.

## The format (version 1)

| field | meaning |
|---|---|
| `version` | spec version, default 1; a newer version than the build reads is refused |
| `name` | the model's name (critics refer to models by it) |
| `input` | what the model reads, see below |
| `trunk` | sizes; anything left out comes from `preset` |
| `heads` | the outputs: `{name, kind, classes, weight}` |
| `data` | training data as queries, see below |
| `critics` | for a generate head: `{model, head, want, weight}` |
| `constraints` | for a generate head: what a kept candidate must satisfy |
| `sampling` | for a generate head: `{temperature, top_k, top_p, max_len, stop, seed}` |
| `train` | `{epochs, batch, lr, val_fraction, patience, max_steps, threads, budget_seconds}` |
| `seed` | weights' and data split's seed |
| `device` | where training and generation run: `"cpu"` (default), `"gpu"`, `"gpu_fast"`; see below |

**Inputs** (`input.type`):

- `text` with `features`: any of `letters` (a = 1 … z = 26 per token), `word_sums`
  (ordinal, reversed and Atbash sums of the word a token is in, and digit roots),
  `side` (whole-text measures at the heads). None by default.
- `bytes`: any data.
- `image` with `width`, `height`, `patch` (default 8) and `codes` (default 128, at most
  255): the picture is cut into patch × patch squares and each square becomes the
  nearest of `codes` learned prototypes (one token each, rows ended by a marker). The
  codebook is learned from the training pictures (integer k-means, deterministic) and
  saved with the model.
- `numbers` with `names` and `codec` (`{digits, decimals, signed}`): the vector is
  written as fixed-width text for the trunk and given to the heads as numbers too.

**Heads** (`kind`): `classify` (one of `classes`, softmax), `multi` (any of `classes`,
a sigmoid each), `regress` (one value per name in `classes`), `generate` (the next byte:
writes text, bytes, pictures or numbers; at most one; makes the trunk causal).
`weight` (default 1) is the head's share of the loss when heads train together.

**Data** (`from`):

- `list` — the members of a list as a class of a head (`class`), as values (`value`),
  or, for a generate head, as texts to learn to write;
- `tagged` — the same for entries with a tag;
- `pairs` — `input` list → `output` list, side by side, for a generate head: it learns to
  write the output after the input (sequence to sequence; `run` then continues an input);
- `inline` — `items: [{input, head, class | value}]` written into the spec.

**Critics** `want`: `{"class": c}` (the probability of c), `{"not": c}`,
`{"near": {"value": v, "target": t, "scale": s}}`,
`{"range": {"value": v, "lo": a, "hi": b, "scale": s}}`. A candidate's score is
Σ weight·ln(score) / Σ weight.

**Constraints**: `min_len`, `max_len`, `gates` (`{critic, class, member}`: that critic's
class must, or must not, be among its members), `sums` (`{measure, lo, hi, modulo}` over
`bytes` (Σ byte + 1), `bytes_reversed`, `ordinal`, `reversed`, `length`), `contains`,
`excludes` (byte strings).

Checks give the first broken rule in words, e.g. `model spec "x": head "a": classify
needs at least two classes`, `image 30×32 must be a positive multiple of the 8 pixel
patch`, `head "a" has no class "z"`, `unknown trunk preset "huge"`.

## Examples

An author classifier (the corpus example), no gematria:

```json
{
  "name": "author",
  "input": {"type": "text"},
  "trunk": {"preset": "small"},
  "heads": [{"name": "author", "kind": "classify",
             "classes": ["austen", "melville", "whitman"]}],
  "data": [
    {"from": "list", "list": "austen passages", "head": "author", "class": "austen"},
    {"from": "list", "list": "melville passages", "head": "author", "class": "melville"},
    {"from": "list", "list": "whitman passages", "head": "author", "class": "whitman"}
  ]
}
```

A text generator steered by that classifier, keeping only short candidates with a byte
sum in a range:

```json
{
  "name": "melville writer",
  "input": {"type": "text"},
  "trunk": {"preset": "small", "max_len": 128},
  "heads": [{"name": "next", "kind": "generate"}],
  "data": [{"from": "list", "list": "melville passages", "head": "next"}],
  "critics": [{"model": "author", "head": "author", "want": {"class": "melville"}}],
  "constraints": {"max_len": 140,
                  "sums": [{"measure": "bytes", "lo": 5000, "hi": 9000}]},
  "sampling": {"temperature": 0.8, "top_p": 0.95, "max_len": 120, "seed": 1}
}
```

A number regressor (two inputs, one output), learning the sum:

```json
{
  "name": "sum",
  "input": {"type": "numbers", "names": ["a", "b"],
            "codec": {"digits": 2, "decimals": 1, "signed": false}},
  "trunk": {"preset": "compact"},
  "heads": [{"name": "y", "kind": "regress", "classes": ["sum"]}],
  "data": [{"from": "inline", "items": [
    {"input": {"numbers": [1.0, 2.5]}, "head": "y", "value": [3.5]},
    {"input": {"numbers": [4.0, 0.5]}, "head": "y", "value": [4.5]}
  ]}]
}
```

A small picture generator (see `core/examples/pictures.rs`):

```json
{
  "name": "toy pictures",
  "input": {"type": "image", "width": 64, "height": 64, "patch": 8, "codes": 128},
  "trunk": {"preset": "small"},
  "heads": [{"name": "draw", "kind": "generate"}],
  "data": [{"from": "list", "list": "toys", "head": "draw"}],
  "sampling": {"temperature": 0.9, "top_p": 0.95, "max_len": 72}
}
```

## Below the spec

The same machinery is open in Rust: `Classifier::with_gen` (any heads plus a generative
head on one trunk), `Example::and` (several targets in one pass), `HeadSpec::weighted`,
`Target::{Next, Avoid}`, `generate::{sample, draw, generate, reinforce, rewards}`,
`generate::Judge` (implemented by classifiers, graph nodes, stored stacks and models),
`codec::{NumberCodec, Codebook, png}`, and `model::Decoder` (cached decoding, equal to the
full pass bit for bit). Everything is deterministic: the same data, spec and seeds give
the same weights and the same samples on any thread count and CPU.

## On the GPU (opt-in per run)

Every model runs on the CPU unless a run asks for the GPU: `"device": "gpu"` in a spec
(training and generation), `TrainOpts::device` and `GenOpts::device` in Rust
(`classify::device::Device::{Cpu, Gpu, GpuFast}`). The backend is `classify::gpu`
(wgpu compute shaders in WGSL — Vulkan, Metal, DX12, GL; the same WGSL is valid WebGPU),
built only with the cargo feature `gpu`; a default build refuses `"gpu"` with "this build
has no GPU backend (build with the cargo feature `gpu`)", and wasm builds never pull
wgpu in.

- What runs on the device: the trunk (every block's norms, projections, attention with
  its KV cache when decoding, SwiGLU, the final norm) and the generative head, forward
  and backward. The embedding lookup, pooling, the classification heads, the losses and
  the optimizer stay on the CPU (a sliver of the work); weights are uploaded after each
  optimizer step. Critics judge on the CPU.
- `"gpu"` (exact precision) follows the CPU's operation order kernel by kernel and guards
  against what GPUs usually do differently (fused multiply-adds, approximate ÷ and √). On
  a device that rounds each operation once it gives the CPU's bits; whether a device does
  is measured, not assumed: `gpu::probe::probe(gpu, precision)` runs every kernel on both
  and reports per kernel how many values differ and by how many ulps. On llvmpipe
  (lavapipe Vulkan) every kernel, whole forward passes, decoding and training steps match
  the CPU bit for bit (`cargo test -p anend-core --features gpu gpu::`).
- `"gpu_fast"` uses the plain operators: faster on real GPUs, within about 1e-4 relative
  of the CPU per kernel, so results are close but not the CPU's bits.
- Saved models answer on the CPU (the reference) unless a run asks for the GPU; a model
  trained on a device the probe did not find exact is a different (equally valid) set of
  weights from the CPU's.
- `ANEND_GPU=off` turns the GPU off; `WGPU_BACKEND` (vulkan, metal, dx12, gl) and
  `WGPU_ADAPTER_NAME` choose among several. Sizes are bounded by the device: the largest
  buffer the adapter allows (all its limits are requested).
- `cargo run --release -p anend-core --features gpu --example gpu_bench` prints the
  device, the probe, and tokens/s on the CPU and GPU (forward, decoding) for the presets,
  the machine's "auto" size and a ladder of larger trunks up to what the device holds.

## Sizes, and why each limit is what it is

| limit | reason |
|---|---|
| `"preset": "test"` (16-wide, 2 blocks) | **test speed only**: keeps `cargo test` to seconds in a debug build. Never a default. |
| default trunk when sizes are left out: `small` (128 wide, 4 blocks, ~0.9 M weights) | a size that trains in minutes on a laptop CPU (measured: ~9600 tokens/s on 4 cores) |
| `"preset": "auto"` | **measured compute**: `bench::measure` (one time, `bench::load_or_measure(path)`) trains a ladder of sizes on this machine; `suggest` picks the largest whose training fits `train.budget_seconds` (default 300) |
| window length (`max_len`) | **measured compute**: attention costs grow with the square of the length, so cost per token rises with the window (see the pictures example's table) |
| pictures as patches rather than pixels | **measured compute**: a 64×64 picture is 72 tokens with 8×8 patches, 4160 at a byte a pixel; the pictures example prints the cost of each |
| codes per codebook ≤ 255 | format: a code is one byte, 255 ends a row |
| file limits in `io` (width 2048, 64 blocks, 8192 tokens…) | safety: a damaged or hostile file can't make the reader allocate without bound |

The main lever to lift the compute limits is the GPU backend (above; `"device": "gpu"`).
