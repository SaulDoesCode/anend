# What's next

Where the work stands, for picking it up in a new session. Branch:
`claude/adoring-pascal-5nk7o1`.

## The queue (asked for on 13 Oct, in this order)

Overall aim, in the user's words: "sensibilization, ergonomics, visual appeal of the app,
feature security"; the game for players; every feature properly supported in the
interface, not stubs.

1. **Classifiers phase 2** (running): storage in saves, the hub's `classify.*` calls,
   the web app's classifiers view.
2. **Desktop polish** (`src/`):
   - icon buttons with hover titles in place of large or common text buttons (e.g. a
     bucket icon for "2 buckets")
   - a tags view: type tags to find what's tagged, scroll all tags with counts, click
     and multi-select to see the results
   - markdown syntax highlighting in the editor
   - the window icon (`assets/icon.png`)
3. **Web polish** (`hub/web/`), the same items: icons with titles, the tags view, editor
   highlighting. The favicon is done.
4. **Classifiers phase 3:**
   - **Classifier buckets:** memoized classifier outputs per memory and note, kept with
     the classifier's version, giving similarity search by classifier and buckets beside
     ordinal, reversed, atbash and frequency.
   - **Colouring:** a classifier-based colour scheme in the memory and space views.
   - **Boolean filters:** hierarchical include/exclude filters over classifiers (A and
     not B...), and classifiers that validate or invalidate other classifiers.
   - **Indexing a note:** a view where a note picks which classifiers to index it on;
     search memoizes what's checked.
   - **Sharing:** baked classifiers export on their own; the hub shares classifier
     weights as modules beside saves (browse, upload, download into a save).
   - **Desktop classifiers view.**
5. **Guide:** `docs/guide.md` on what the app does and how to use it, linked from the
   README and the apps' help.
6. **Review pass:** security (hub upload and op limits, classifier file checks),
   ergonomics, visual consistency, and no stubs.

## In progress: classifiers (`core/src/classify/`)

A deterministic transformer encoder written from scratch, trained on lists. Its own
progress is in `core/src/classify/STATUS.md`. The plan:

- **Engine (phase 1):**
  - byte tokens plus gematria channels (letter values; each word's ordinal, reversed and
    atbash sums and digit roots), and a whole-text side vector
  - pre-norm RMSNorm blocks, SwiGLU, RoPE, optional QK-norm
  - heads: one class, several classes at once, or a number
  - long texts in windows
  - AdamW with warm-up and cosine decay, clipping, label smoothing, class balance, early
    stopping, and metrics (accuracy, F1, confusion matrix, log-loss, calibration)
  - growth: widen or deepen without changing what the model does, and prune only when
    accuracy holds
  - `Steer`, set by the player and never changed by training: per-class bias,
    temperature, channel gains, threshold, plus a bias metric
  - `represent()` as a deterministic embedding
  - composition: one classifier's outputs feed another, and its predictions become
    lists
  - feedback: yes/no per classifier on a result (the main kind), or end to end through a
    stack with a coupling strength per link; a result lists which classifiers
    contributed, so each can get its own yes/no
  - bit-exact on any thread count and CPU: no FMA, fixed summation order, its own
    exp/ln/tanh
  - SIMD and threads where measured to help
  - proof: `cargo run --release -p anend-core --example authors` (corpus in
    `core/corpus/authors/`)
- **Phase 2 (done):** classifiers kept in the save (`core/src/classifiers.rs`: tables,
  `Store` API for the desktop app too, bundles carrying them), the hub's `classify.*` ops
  with background training (`hub/src/classify.rs`, docs/app-api.md), and the web app's
  classifiers view (Ctrl+7: make from lists, train with progress and metrics, steer with
  the bias metric live, classifier search with yes/no per contributing classifier,
  outputs as lists, near texts). Offline the web app says classifiers need a hub.
- **Then:**
  - the desktop app's classifiers view (src/), on `anend_core::classifiers`
  - a JavaScript port of inference for offline use (as exact as `hub/web/local/` is for
    the sums)
  - classifier search inside the memory view's search modes, filter presets saved as
    their own list system, and default classifiers for new saves
- **Later:** a GPU backend behind the kernel boundary (wgpu in the app, WebGPU in the
  browser). Drivers may fuse multiply-adds, so saved models answer on the exact CPU path
  unless the GPU can be proven exact.

## Not started

- **Entries:** timestamps and mime types (detected by pattern, fixed with a right click),
  word types shown on hover in the memory view, and a search filter panel that opens from
  a square at the right of the search (mime types, word-type frequency, classifiers,
  feedback, bias metric).
- **Editor:** entry ids as replacements (recursive, with references to saved filter
  views; sums stay unchanged), and synonym/register lists chosen from a dropdown, with
  their own management view.
- **Spatial memory table:** slug texts placed by hand in 3D, with coloured
  cubes/blocks/bricks and a gimbal to move, scale and rotate.
- **Space search modes:** normal, classifier, and weum's multithreaded random hashmap
  search; spreads by classifier.
- **Save checkpoints:** redb savepoints, grouped under each save in the save menu.
