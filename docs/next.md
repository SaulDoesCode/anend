# What's next

Where the work stands, for picking it up in a new session. Branch:
`claude/adoring-pascal-5nk7o1`.

## Resuming (read first)

As of 13 Oct 2026, evening UTC:
- **Done:** items 1 (classifiers phase 2) and 3 (web polish).
- **Were running when the session ran low:** item 2 (desktop polish, in `src/`) and the
  core, hub and web part of item 4 (classifiers phase 3). Their progress is under their
  items below and in `core/src/classify/STATUS.md`.

To pick up:
1. Check what landed: `git log --oneline -15`, `git status`.
2. Finish item 2 and item 4.
3. Then do the desktop side of item 4: the classifiers view, colouring in the memory and
   space views, and the note-indexing panel, using `core/src/classifiers.rs` directly.
4. Then item 5 (the guide) and item 6 (the review pass).
5. Then the backlog under "Not started".

- **Shortcuts differ between the apps:** the desktop has tags on Ctrl+7, while the web
  app has classifiers on Ctrl+7 and tags on Ctrl+8. Match them when the desktop
  classifiers view lands: classifiers on Ctrl+7 and tags on Ctrl+8 in both.
- **Desktop to check:** the editor doesn't scroll with the mouse wheel (bevy's editable
  text), and a long note once showed every note-list row filled in.

Checks to run before every push:
- `cargo clippy -p anend -p anend-core -p anend-hub`
- `cargo test -p anend -p anend-core -p anend-hub`
- `node hub/web/test/parity.mjs`, which must print "all the same"

## The queue (asked for on 13 Oct, in this order)

Overall aim, in the user's words: "sensibilization, ergonomics, visual appeal of the app,
feature security"; the game for players; every feature properly supported in the
interface, not stubs.

1. **Classifiers phase 2** (running): storage in saves, the hub's `classify.*` calls,
   the web app's classifiers view.
2. **Desktop polish** (`src/`): done.
   - icons: a shared set of 24px line icons drawn for anend (`assets/icons/*.svg`),
     rasterized white on transparent by `node scripts/icons.mjs` into
     `assets/icons/png/` and embedded (`src/icons.rs`); an icon takes its button's text
     colour, so the palette and zen reach it. Icons now: the top bar's views and
     settings, the editor's list toggle, view/edit, save, delete and tag box, "new
     note", the settings' forge/keys/close, a save's copy/rename/delete, the hub's
     connect/search/upload/download/merge and its star and download counts, the space's
     text and bucket counts, the reader's edit.
   - tooltips: anything with a `Tip` shows it after a short rest, with the key bound to
     its `TipKey` action (from the remap, so it follows rebinding); hidden while zen
     fades, placed inside the window, scaled with the interface. The memory view's
     bucket and toggle words carry tips with their keys too.
   - the tags view (Ctrl+7, `src/tags_ui.rs`): every tag with its count, narrowed by
     the command bar (Enter adds the first match, so typing tag after tag chooses
     them); click picks a tag, Ctrl/Shift-click adds or removes, "all"/"any" decides
     how several combine; the notes found show title, date, tags and a preview, and
     open in the editor when clicked. Stats' tags open it too. Counts come from
     `note::tag_counts`, matching from `note::has_tags` (anend-core), so memories can
     join later.
   - highlighting (`src/highlight.rs`): the pad's laid out glyphs are copied into an
     overlay text whose spans carry the class colours (headings, marks, emphasis,
     strike, code spans and blocks, links and urls, list markers and task boxes,
     quotes, table pipes, tags, rules, IME composition); the pad's own glyphs go
     transparent, so editing, cursor, selection, IME and scrolling are untouched. Only
     visible glyphs are drawn; classes are recomputed once per layout change. Colours
     come from the palette, nudged until they read on the pad (night and paper work);
     a settings toggle turns it off. If glyphs ever fail to match the text, the pad
     shows its own text again.
   - the window icon from `assets/icon.png` (embedded, set through winit).
   - polish: plain buttons (the note list, the tag list) take the card's text colour
     and button text picks dark or light by contrast, so dark palettes read; the
     editor's selection and cursor follow the palette instead of a stock blue; the
     memory view says how to start instead of showing an empty page; the note list
     toggle's setting names the right edge too; stats' "+ new" hint updated; icons in
     buttons take the button's text colour (ThemedText) and fade with zen.
   - left for later: the memory view's and space's text toggles in the bar stay words
     (they carry state); the lists view's "choose"/"unlist" stay words; IME composition
     was checked only by code (colour class `Compose`), not with a real input method;
     the pad doesn't scroll with the mouse wheel (bevy's EditableText; the overlay
     follows whatever offset it has).
3. **Web polish** (`hub/web/`): done.
   - icons: the shared set (`assets/icons/`, copied into `hub/web/iconset.js` by
     `node hub/web/tools/icons.mjs`; `classifier.svg` added) for the top bar's views and
     settings, the editor's actions, the space's buckets and help, the memory view's
     bucket, save rows, the forge, the hub, classifier actions (yes/no as ✓/✗), lists;
     every one with an aria-label and a styled tip (name and shortcut), keyboard focus
     visible; the app's mark in the top bar
   - the tags view (Ctrl+8, `#/tags/a+b`, `#/tags/a+b/any`): tags with counts, found from
     the command bar, picked with all/any, their notes beside; the hub's `notes.tags` and
     `notes.list {tags, mode}` (in the offline engine too, in the parity run)
   - editor highlighting: a see-through textarea over a mirror of highlighted lines,
     redrawn line by line (plain past 250 000 characters)
   - empty states for the memory view and the space, a shortcuts table in settings, the
     stats' tags opening the tags view, a clean console from a static server
4. **Classifiers phase 3** (running). Core done and tested (see
   `core/src/classify/STATUS.md`, "Phase 3"): `core/src/memo.rs` (memoized classifier
   buckets, near search, per-note indexing choices, tints/palette for colouring),
   `core/src/filters.rs` (boolean filter trees, presets, validators, disagreements),
   `core/src/clsmodule.rs` (`.anendcls` export/import, strict reading). The desktop app
   can use these `Store` methods directly. Left, in this order:
   - **hub ops: done** (tested in `classify::tests::classifier_calls`, documented in
     docs/app-api.md; offline engine refuses `filters.*` too, parity all the same).
     What was listed: `classify.index {name, keys?}`
     as a background job like training (progress via `classify.index_job`, cancel),
     `classify.bucket {name, class|text|key, band?}`, `classify.buckets {name}` (counts),
     `classify.near {name, key|text, by: outputs|embedding}`, `classify.indexed {key}`
     (chosen classifiers with verdicts: probs/top/members + validator checks),
     `classify.set_index {key, names}`, `classify.tints {name, keys}` (+ palette hex),
     `classify.validators {name, set?}`, `classify.disagreements {name}`,
     `classify.export {name, upstream?}` (base64 or a GET download route
     `/app/classifiers/{name}.anendcls`), `classify.import` (multipart POST
     `/app/classifiers`); `filters.list/save/delete/run/enlist`; make `classify.search`
     use `memo_rows`/`checked_rows`. Offline engine (`hub/web/local/engine.js`): answer
     `filters.*` with the same "classifiers need a hub for now"; parity must stay "all
     the same".
   - **hub sharing**: classifier records in hub/src/store.rs (table + files
     `classifiers/{id}.anendcls`, stars/downloads keyed `c:<id>` in the existing
     multimaps), `Hub::accept_classifier` (size limit, `clsmodule::read` in
     spawn_blocking, pending unless trusted, points as saves), pages `/classifiers`,
     `/classifiers/{id}` (+download, star, edit, delete), upload form, admin review
     section, API `/api/classifiers`, app routes "take into my save" and "share from my
     save"; askama templates in the sand look; tests in hub.rs.
   - **web UI**: classifiers view (index button with progress, bucket counts, near by
     outputs/embedding, validators editor, disagreements with yes/no, export/import,
     hub browse with take), editor drawer (checkboxes of classifiers per note, verdict
     chips, yes/no), memory and space colouring by a chosen classifier (legend;
     `classify.tints`), a filters view (Ctrl+9: tree editor, presets, run, enlist).
   - Playwright screenshots into the scratchpad `shots-phase3/`; docs.
   - **Desktop classifiers view** (src/, next phase).
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
