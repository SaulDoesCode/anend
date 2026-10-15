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

- **Desktop classifiers (14 Oct): done**, in `src/` (details under item 4, "Desktop").
  Shortcuts now match the web app: classifiers Ctrl+7, tags Ctrl+8, filters Ctrl+9 (a
  keymap kept from before moves tags off Ctrl+7 unless the player had rebound them).
- **Desktop follow-ups: done.** The wheel scrolls the editor (`ui::pad_scroll`, through
  `TextEdit::ScrollBy`). The window's root node could grow past the window when a view's
  content was tall (taffy took the content's height for `height: 100%`), which kept
  scroll areas from scrolling and is the likely cause of the "every note-list row filled
  in" sighting; the root is now `min_height: 0, max_height: 100%`. Not seen again with a
  160-line note.

- **Desktop gaps, guide link, guide update (14 Oct, unfinished; pick up here).** Built,
  clippy clean, `cargo test -p anend` passes; NOT yet verified by driving except where said.
  - Done in code: `src/cls_more.rs` (new): validators editor (card "validators" in a
    trained classifier's page: checked by / saying / at least slider / gate|scale /
    remove / "+ checked by"; edits kept by `tick_classifiers` 0.5 s later through
    `Store::set_validators`, then tints/panel/search refreshed), disagreements card
    ("find them" on a thread via `Store::disagreements`, ✓/✗ for the classifier and for
    each validator through `classify_ui::judge_pair`, re-found after a judge); hub
    classifiers: side button "from the hub" (`HubBrowse` in `ClsState.hub`), sorts,
    refresh, bar Enter searches, cards with classes/accuracy/stars/downloads/upstream
    warnings, "more", "take" (download `/api/classifiers/{id}/download`, cap 64 MB,
    `clsmodule::read` + `import_module` with default opts = classes to lists by name);
    "share on the hub" icon (Upload) in the classifier's header (`POST /api/classifiers`,
    bearer app token). `src/hub.rs`: `HubClassifier`, `classifiers()`,
    `classifier_module()`, `share_classifier()`, `id_ok`, `guide_url`, `open_url`
    (xdg-open / open / explorer with the URL as one argument, no shell; tested).
    Settings header has a "guide" button (Icon::Help) opening the hub's `/guide` or the
    GitHub copy. Filters view: sum conditions ("+ ordinal/reversed/atbash" from a number
    or range typed in the bar, `sum_range`, tested). Icons `Shield`, `Alert` added to
    `src/icons.rs`. The classifier list's names no longer wrap word by word (NoWrap).
    `seed_demo` with `ANEND_SEED_TRAIN=1` also trains "kind" and "fruit or not" (tiny).
  - **The validators card is fine (checked 14 Oct):** the "bug" was the drive scripts
    clicking a row too high (y 470 is the "none: …" line; "+ checked by kind" is at y≈498).
    Clicking it adds the validator; its class buttons, slider, gate/scale and × work, and
    "find them" lists the disagreements with ✓/✗ per classifier.
  - **The app's icon** is at the left of the desktop top bar (`icons::AppMark`), as on the
    hub and in the web app.
  - Left: drive and screenshot (scratchpad `shots-desktop4/`, demo save
    there `demo.redb`): hub browse/take/share against a fresh
    `anend-hub --admin you --addr 127.0.0.1:18221 --data <scratch>` with a seeded
    classifier, sums in filters, the guide button. The "indented text in name boxes after
    a redraw" is NOT investigated yet. docs/guide.md NOT updated yet (item 3 of the brief:
    desktop filters Ctrl+9, validators, hub classifiers, tags Ctrl+8, classifier strip
    button, "colour views", export to Downloads / take in; fix "where the apps differ"
    and the shortcut tables).

- **Hub icons (asked for on 14 Oct): done.** `hub/web/tools/icons.mjs` also writes the
  sprite `hub/static/icons.svg` (served at `/static/icons.svg?v=…`, cached for a year);
  templates draw an icon with `{{ crate::web::icon("name") }}`. Icon and words for primary
  actions, the icon alone with an `aria-label` and a CSS `[data-tip]` tooltip (the web
  app's look) for commonplace ones; the bar's tabs drop their words on a phone. The test
  `web::tests::icons_in_the_sprite` fails when a page asks for a missing icon or the
  sprite is behind `assets/icons` (run the script again).

- **Next big item: one concept, the entry (asked for on 14 Oct).** Notes, memory texts and
  list names are meant to be one thing, not several. Today a note is a separate
  record with its own key (`note:<key>`), memories live in `memories`, and list heads are
  memories or notes. With timestamps (and mime types) coming, do a single schema change
  that unifies them:
  - **What an entry is:** text (a note is an entry whose text is a `# title` plus
    markdown), tags, created and edited times, mime type, and its key.
  - **What reads entries:** lists, classifiers, buckets, placements and the forge all
    refer to entries.
  - **Schema version:** the save carries one, and opening an older save **migrates it in
    place** (with a backup copy first) so players never have to start fresh.
  - **Bundles:** they gain the version too, and old bundles migrate on import.
  - Do this before any more features build on the split.

Checks to run before every push:
- `cargo clippy -p anend -p anend-core -p anend-hub`
- `cargo test -p anend -p anend-core -p anend-hub`
- `node hub/web/test/parity.mjs`, which must print "all the same"

## Step 1 status: the entry graph store (`core/src/graph/`)

New module beside the old store (apps not rewired yet; nothing old deleted). 37 tests,
`cargo test -p anend-core graph::` (about 15 s in debug). Clippy clean on the module.
- **Done:** `codec` (5-bit tokens, ESC payloads incl. UTF-8 lead runs and digit runs,
  caps lock, REF/CALL, fuzzed); `interp` (explicit continuation stack, tail calls in
  constant stack incl. through `if` and the last `concat` argument, memo, fuel/depth/output
  limits, cycle-safe refs; counter to 1,000,000, mutual even/odd, factorial, Fibonacci);
  `gematria` (ordinal and reversed sums and the frequency key; atbash removed 14 Oct,
  it gave the same groupings as reversed); `record` (varint record header, blocks of 32 entries per row,
  past versions as deltas, weum key ⇄ id mapping, content hash); `postings` (one row per
  term and id chunk, varint deltas or a bitmap; bucketed hash pairs); `place` (34-byte
  transform, rotation re-pack now idempotent); `store` (Graph/View/Tx: dedup, borrowing of
  prefix/suffix entries with rehoming when a source changes, edits with versions, delete
  and undelete, weum-order id reuse, edges NOTE/MEMBER/TAG/PLACE/VALIDATES, indexes on disk
  or in memory, compact, per-table storage stats, in-memory backend, injectable clock);
  `query` (words, sums, letters, tags, time, mime, relations, order, paging, stats);
  `backup` (weum `all-of-it.json` import, texts.json + other.json backup and restore).
- **Bench** (`cargo run --release -p anend-core --example graph_bench`, 100k texts with
  many prefix/suffix variants + 2k notes, 9.1 MB of UTF-8): old store 54.0 MB; new store
  17.1 MB with disk indexes (19.8 MB without borrowing), 9.1 MB with indexes in memory;
  packed bodies 5.84 MB, 4.45 MB with borrowing (48k of 102k entries borrow). Durable
  insert 3.1 ms (fsync), batched 0.18 ms; sum lookup 8 µs; word search 3 ms; render
  5.8 µs per entry. Rebuilding the indexes into memory at open: 2.6 s for 102k entries.
- **Format:** `docs/format.md`.
- **Left for later:** the indexes' B-tree pages are about half full (redb splits pages
  50/50; a bulk rebuild in key order doesn't help); the memory-mode rebuild needs to get
  faster (or be persisted as a snapshot) before it's the default; borrowing only finds
  sources the new text contains (a shorter variant saved after its longer form isn't
  stored as a slice); structured entries are indexed by their render at save time; no
  wasm check (the wasm32 target isn't installed here).

## The Dioxus app (`app/`, package `anend-app`): status

Dioxus 0.7.10 (pinned); desktop through dioxus-desktop/wry (needs `libwebkit2gtk-4.1-dev
libxdo-dev`), web through WebAssembly. Features: `desktop` (default), `web`, `hub`
(default; the hub's settings). In the workspace and CI (build, clippy, a wasm32 build).
- **Done:** `gem.rs`: weum's numbers checked against api.js/memory.js (the gsum line letter
  for letter, pfR via core, seedCLR as JavaScript's string hash, underline groups per mode
  with weum's `key.length >= 2` rule, the triangle, ⌊√(s·r)⌋, the byte sum), tested.
  `backend/mod.rs`: the `Backend` trait (one `call(op, args)` over the app API, typed
  methods for entries, search by sum/words/tags/time, lists, notes, tags, models, filters,
  settings, backup, weum import). **The desktop backend is the entry graph**
  (`backend/local.rs`): the ops answered from `graph::Graph` in process on worker threads
  (keys are weum labels of ids, notes markdown entries, lists member edges, tags tag edges),
  plus `mem.put`, `mem.buckets`, `tags.*`, `notes.on`, `entries.query`, `saves.backup`
  (texts.json + other.json in `backups/`), `saves.import_texts` (weum all-of-it.json),
  `settings.*`; saves in `<data dir>/anend/saves/*.redb`, default `main`; tested. Models
  answer "not yet" until the model spec lands (docs/models.md). Atbash is gone (the user's
  decision): modes are ordinal, reversed and frequency; frequency underlines by ordinal.
- **Found:** "every text in ordinal bucket 345 underlined purple" is weum's own behaviour:
  in reversed mode the grouping sum is the ordinal one, and seedCLR("345") is hue 276.
- **Shell and views (15 Oct):** `backend/hub.rs` (WebSocket, queue, reconnect; the wasm32
  build checks). `shell.rs`: top bar (mark, view icons with styled tips, the centered search
  line with the triangle, status, undo, settings), the dock (the strip only in memory/space,
  the centered bar not bound to a signal so typing never waits), one JS key listener (views,
  zoom Ctrl+=/−/PageUp/PageDown/0, settings Ctrl+,, Ctrl+Z bar undo / action undo, Esc),
  toasts, the right-click menu (edit keeps the key, tag…, select, list, candidate, key,
  forget). `views/memory.rs`: weum parity (Enter/Ctrl/Shift/Delete, list commands, click /
  Ctrl / Shift / double / Ctrl+right click, hover title and line, underlines, pfR, toasts,
  r/o/f/g/s/l, ← →), the left-edge bucket circles, Alt+click multi-select → pick a list or
  tag, the draggable centered field, spacing and size. `views/settings.rs`: dialog closed
  by outside click/Esc, folding sections remembered, rules between them, palettes + own
  colours, text size, zoom, memory sizes/spacing/field, space background, saves (open,
  new, copy, two-click delete, backup, the step-by-step weum import), keys, hub (folded,
  only with `hub`). `views/lists.rs` (tiles → a list's page: editable name and members,
  pagination, figures, two-click delete) and `views/reader.rs` (pages, a tiny grid for
  short texts, expand one with ← →). Driven under Xvfb: shots in the scratchpad's
  `shots-dioxus/` (memory view, menu, hover line, buckets).
- **Left:** notes editor, tags, stats, models (minimal until the model spec), filters
  (over `entries.query`), the space (three.js interop), more screenshots.
- **Checking while core is mid-edit:** `scratchpad/wtcheck.sh cargo clippy -p anend-app`
  builds `app/` against the committed core in a worktree.

## Decisions (14 Oct): the redesign

The user asked for "the route that provides the most feature coverage and performance and
ergonomics" and approved this plan.

1. **One entry graph is the core.**
   - **`entries`:**
     - **The record:** a u64 id, created and edited times, a mime type, and a body.
     - **The body:** runs of 5-bit packed text and references to other entries
       (borrowing and dedup; templating uses the same mechanism).
   - **`edges (from, relation, to [, payload])`:** one table for every relation (notes-on,
     member-of, tagged, placed-at with a packed transform, validated-by), plus a reverse
     index.
   - **Derived indexes:** sums in every mode, tokens, time and classifier buckets. They
     can always be rebuilt from the two tables above.
   - **Compatibility:** none with old saves or blog writs. Only weum's `all-of-it.json` is
     imported. Backups write a texts JSON and an other-data JSON.
2. **Gematria checked against weum:** atbash per `alphavert`, and the colouring groups per
   mode.
3. **The UI is written once, in Rust, with Dioxus** (decided 14 Oct, replacing the
   earlier HTML/CSS + Tauri idea; the user asked for Rust).
   - **One codebase:** RSX components shared by the desktop app (Dioxus desktop: the
     system webview, so browser-quality text, zoom and fast input) and the web app
     (WebAssembly rendering real HTML).
   - **Parity by construction:** the desktop, web app and hub look the same.
   - **The core runs in the browser:** the Rust core compiles to WebAssembly, with redb on
     a storage backend over OPFS/IndexedDB. That replaces the JS offline engine
     (`hub/web/local`) and its parity test.
   - **The hub's pages and API:** served from the same Rust code (Dioxus fullstack/SSR or
     Salvo serving the built app).
   - **The space:** a canvas in the app, starting with the existing three.js space through
     interop; a wgpu/Rust renderer later if it earns it.
   - **Bevy and the old JS app:** retired once the Dioxus app reaches parity.
   - **Priority:** the desktop app is the primary product; the browser version exists for
     reach (the user: "only reason I desire the browser action is because it might not
     be as popular otherwise"). Build and polish the desktop first. The web build follows
     from the same code, and where something only works well on the desktop, the desktop
     wins and the web gets a lighter version.
4. **One API:** the app ops (WebSocket/REST) are the API. The desktop, web app, scripts and
   lenses all use the same calls.
5. **Views are lenses over one query model:** words, sums, tags, time, classifiers and
   relations.
6. **Then:**
   - server modes (hub only, web app only, both), invite-only, private sharing, sign-in
     for the web app, a hub-free build;
   - generative heads and classifier-guided generation over bytes;
   - the scripting language.

Order of work: (1) the entry graph store, the weum import and backup → (2) gematria
correctness → (3) the API on the new store → (4) the Dioxus app (desktop and web), text quality and
latency → (5) memory parity, right-click actions, undo → (6) lists, reader and stats as
lenses → (7) server modes and security → (8) generation and the language.

## Feedback after running the desktop app (14 Oct): the next round

The user built and ran the app and imported their weum texts. Everything below comes from
that feedback. Several items (the entry store, the gematria fixes, text quality) come
first, because the rest builds on them. The unfinished work from the cut-off jobs (space
placements in `core/src/places.rs`, partial keyboard passes in `src/` and `hub/web/`) is
kept in `docs/wip/2026-10-14-unfinished.patch` (`git apply` it to look). It's superseded
by the store redesign below, apart from the keyboard bits, which may be reused.

### A. The data (first, the foundation)
- **One flat entry table.** Notes, entries, memory texts, space texts and list names are
  one concept, on the same keys in one flat redb table of texts.
- **Timestamps** (created and edited) on every entry. Search, tags, classifier search and
  filters get time filters.
- **Compression and dedup.** The data is fat, and the user keeps many variants of a text
  (with a prefix or suffix missing, written differently). An entry can be made of
  references to other entries (borrowing), so variants share storage instead of
  duplicating it.
- **A 5-bit text encoding, bit-packed:**
  - 26 lowercase letters plus space take 27 of the 32 codes.
  - An escape code means the next token is a raw byte.
  - An uppercase code makes the next token uppercase.
  - A reference code means a replacement key follows. Decoding fetches that entry, which
    is also how the editor's replacements/templating are represented.
- **Notes are a relation, not a separate kind of thing.** As in weum, notes go on entries:
  a graph with a single relation called "notes". Notes are chronological, not organised
  by title. Rethink what a title is, since nothing else has one.
- **Compatibility:**
  - Drop all compatibility, forwards and backwards, including the old blog (writs)
    import.
  - Keep only importing weum's `all-of-it.json`.
  - Importing must be a visual, per-save process in settings, because import and save
    semantics conflict today.
- **A backup button** per save: all the texts in one JSON and everything else in another,
  like weum's `await memory.backup`.

### B. Gematria correctness (weum is canonical)
- **Atbash removed (14 Oct):** it gave the same values as reversed, so only ordinal and
  reversed remain (the store's modes are ordinal 0 and reversed 1).
- **Underline colours** for matches in the memory view (as weum's
  `docs/weum-reference/memory.js`):
  - ordinal mode: colour by shared reversed sums;
  - reversed mode: colour by shared ordinal sums;
  - frequency mode: colour by ordinal sums.
- **A bug to fix:** every entry in ordinal bucket 345 showed a purple underline (seen in
  the reversed view). Study weum's memory to recreate its behaviour exactly.
- **Frequency letters:** the sidecar's frequency letters grow long. Let them be toggled,
  or shown smaller or in another form.
- **Later idea:** generalise from lowercase gematria to summing bytes 1–256.

### C. Memory view: parity with weum
- **Ctrl+right click** is gone; restore weum's behaviour.
- **Hovering a text:** it comes to the top so it isn't hidden under another entry. The way
  text expands on hover is blurry and was always buggy; rethink it.
- **Spacing:** spacing between texts as a setting. The texts sit too close to the digit
  sums above them, and long texts need an overflow story (every view needs one for really
  large texts).
- **The centre square:** dragging its edge sets its default width, and it stays centered.
- **Multi-select** texts: they stay highlighted, then pick a list visually to put them in.
- **A left-edge strip**, shown only on hover and fading in and out: a line with circles,
  one per sum bucket, each with its number and sized by how many entries it holds.
  Hovering shows which bucket it is, for the active mode.
- **The triangle** the space shows, here too.

### D. Text quality and responsiveness (the main quality-of-life issue)
- **Blurry text** everywhere: the memory view, the space popup, hover. Prefer no
  antialiasing at all to bad antialiasing. Try other fonts and bigger sizes.
- **A font size setting.**
- **Typing feels sluggish** in the command bar: debounce the sum work, run it async, and
  cut latency.
- **Zoom:** Ctrl and Page Up / Page Down zoom in and out, like a website.

### E. The space
- **The triangle:**
  - clicking a text should put its triangle in the top bar; it currently stays plain text;
  - the triangle's middle bar should be centered.
- **Colours:**
  - the bracketed text at the right needs enough contrast against the space's
    background;
  - the background colour should be a setting;
  - on a dark background, texts must keep readable contrast.
- **The popup on the right:** it should be dismissable, and its text is blurry.
- **Arrow keys** ease the camera onto the nearest text in that direction.
- **Texts shrink** as the camera comes close, so they stay readable.
- **For the editable space later:** the camera can't fly through blocks or glTF models.

### F. Acting on texts
- **A right-click menu** on any text in the memory view and the space:
  - edit: the text goes into the bar and the result replaces it under the same key;
  - tag: the next thing typed in the bar tags it;
  - and other actions.
- **Undo:** the bar's typing in the main views and the space can be undone back to the
  previous state. There's also an undo button for any recent action, including yes/no/why
  given to a classifier.

### G. The views
- **Lists:** redo the view.
  - **Structure:** a list's page shows its name at the top and its members as editable
    entries, with pagination. From there you go back out to the list of lists. A list's
    figures go at the bottom.
  - **Delete:** deleting a list takes two steps.
  - **Clarity:** icons with titles, and a clear meaning for "unlist" and "choose" when
    several lists are shown.
  - **Classifiers depend on it:** you can't really make lists today, so the classifiers
    are untested.
- **Reader:**
  - pagination;
  - an expand mode for reading one thing big at a time;
  - thousands of tiny texts flowing in a grid instead of one row each.
- **Stats:** should cover the whole app, not just notes.
- **Notes editor:**
  - it resizes when the bars fade;
  - long titles show in full on hover, like a website's title attribute.

### H. The app's shell
- **Collapsed menus:** remember which menus were collapsed.
- **Settings:**
  - it closes on a click outside it;
  - the rebind menu closes the same way, with its close button in the same place;
  - horizontal lines separate its sections;
  - hub-related settings are collapsed, unless compiled out.
- **Spacing:** smaller gaps between elements, none at the window's edges.

### I. Build, server modes, security
- **Desktop build:** a compile feature that leaves the hub out of the desktop app.
- **Server modes, set by flags:** hub only, web app only, or both, coherent and complete.
  This matters because the user is thinking of selling access as a cloud service.
- **Web app:** sign-in required, even without the hub.
- **Hub:**
  - invite-only;
  - public and private toggles on everything shared;
  - a professional finish that works nicely with the app, with promise as the app's
    social side.
- **Parity:** the web app and the hub are kept up to date with the desktop app, and look
  the same.

**Status (14 Oct, evening): built, tested, pushed.** In `hub/`:
- **Modes** (`hub/src/mode.rs`): `--mode hub|app|both` (default both), `--local` = the app
  alone, no login. `main.rs` `router(mode)`/`service(mode)`: routes outside the mode don't
  exist (404); nav, front page, footer and the admin panel adapt (`Layout.hub_on/app_on/
  local`). App-only `/` is a landing page (`pages.rs`); `/about`, `/terms`, `/privacy`
  (markdown from `--terms`/`--privacy` or `<data>/terms.md`/`privacy.md`, placeholder
  otherwise) in every mode. Cargo features `hub`, `app` (default both); all three builds
  are clippy-clean and pass their tests. `x-anend: app` tells the web app there's no hub
  (`hub/web/api.js` `api.noHub`; settings and classifiers hide hub browsing).
- **Sign-in for the web app:** `/app/` redirects to `/login?next=/app/` (a waiting account
  gets a 403 page); its files, `/app/api/*` and `/app/ws` answer 401 (no one) / 403
  (waiting). `next` only goes to a local path.
- **Joining** (`invites.rs`, `Hub::join_policy` in META): `--join invite|apply|open`
  (`--open` kept); with no flag a new server is invite-only and an existing one with
  people keeps `apply`. Invites: hashed codes `xxxx-xxxx-xxxx`, uses 1-100, 0-365 days,
  shown once, `/invites` (admins; trusted members when the admin panel allows),
  `/invite/{code}` fills the join form, revoke, all listed in `/admin#invites` with the
  policy settings; a wrong code costs a try (10/h per address); `--invite` prints one.
  `User.invited_by`.
- **Visibility** (`store::Visibility`, `store::may_see`): private/unlisted/public on saves
  and classifiers; only public+approved are listed (grids, search, feeds, profiles,
  leaderboard, `/api/*` lists, counts); private/unlisted skip the queue; private is the
  owner's alone (pages, downloads, API); downloads/stars/points only for listed ones;
  `--share-quota` (512 MB) counts all. Forms have a visibility choice, pages an owner
  control (`/saves/{id}/visibility`, `/classifiers/{id}/visibility`); API takes and
  returns `visibility`; `GET /api/mine`.
- **Ops** (`ops.rs`): `/health`, a log line per request (`--log json`; login/invite secrets
  redacted), error pages for empty error answers, graceful stop on SIGTERM/Ctrl-C.
- **Tests:** `hub/src/modetests.rs` (routes per mode, app sign-in incl. socket, invites,
  visibility leaks); `testkit.rs` shared helpers.

- **Deploying:** `docs/deploy.md` (modes, flags, joining, visibility, TLS, backups,
  running it as a paid service), `hub/Dockerfile` (+ `.dockerignore`, `HEALTHCHECK`),
  `hub/anend-hub.service` updated, `hub/config/{terms,privacy}.example.md`;
  `docs/hub-api.md` and `docs/security.md` cover modes, invites and visibility.
  Screenshots of each mode's landing, the invite flow and the visibility controls were
  taken (15 Oct, scratchpad `shots-modes/`).

Left: the desktop build without the hub (`src/`, not started; a cargo feature there).
Ideas not done: per-member quotas (today they're per server), a "copy link" button for
a just-made invite, a README section pointing at docs/deploy.md.

### J. Docs
- **README:** rewrite it, because it's very out of date (including the old blog import).
  Redo the screenshots or remove them.
- **Guide:** touch it up after these changes land.

### K. Direction (under discussion, 14 Oct)
- **Transformers that generate** (in progress; done/left in `core/src/classify/STATUS.md`,
  phase 4):
  - multi-output transformers, not only classifiers — done in the engine: heads and a
    generative head share a trunk, trained jointly with per-head loss weights;
  - generators shaped by classifiers, which act as the feedback that trains them — done in
    the engine (`classify::generate`: critics, constraints, guidance, reinforce, yes/no);
  - working over any data, not just text: bytes, numbers, images — codecs done (fixed-width
    numbers, pictures as patch-codebook bytes, PNG writer);
  - byte sums (bytes valued 1–256) are a separate measure (`core/src/measure.rs`), not a
    model feature; gematria channels are opt-in model inputs, off for new models;
  - left: the declarative model spec (`classify/spec.rs`, `docs/models.md`) as the primary
    interface, the machine benchmark for honest default sizes, the examples (authors
    generator, picture generator with measured cost).
- **A small scripting/templating language** (cf. the user's s-lang):
  - built around references, transformer outputs, data types, mutation and closures;
  - transpiled to concatenated Rust calls over a state machine, linked against a
    precompiled preamble so it compiles fast.
- **Views as templated modes of consumption,** like little websites over the core's API:
  memory, space, notes, blog, and more.
- **Lifting the model ceiling (queued for when the generative job finishes, 14 Oct).** The
  user doesn't want preference-driven "toy" limits. Next:
  - **A GPU backend:** wgpu, behind the engine's kernel boundary; checked against the CPU
    results; benchmarked on the user's hardware, since this container has no GPU.
  - **Importing pretrained open models:** safetensors weights plus a BPE tokenizer. The
    engine already has Llama-family blocks (RMSNorm, SwiGLU, RoPE), so it can run real
    pretrained LMs, steered by the user's classifiers as critics. Downloads happen on the
    user's machine (model hubs are blocked here); test the loader with synthetic weights.
  - **Rule:** small sizes only where measured compute requires them, and never gematria
    by default.

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
   - **hub sharing: done** (tests `clshub::tests` and `clstests::classifier_site`; docs
     in docs/hub-api.md "A classifier" and docs/app-api.md "Classifiers between the hub
     and a save"). `hub/src/clshub.rs`: records in table `classifiers` (store.rs
     `ClassifierRec`), modules at `<data>/classifiers/{id}.anendcls`, stars/downloads in
     the saves' multimaps keyed `c:<id>`; `Hub::accept_classifier(_bytes)` (size cap
     min(--max-upload, 64 MB), `clsmodule::read` in spawn_blocking + catch_unwind, empty
     name/description from the module, pending unless trusted, new versions keep stars
     and downloads, points as saves), find/star/download/review (approve, reject with a
     note, feature, remove); badges "first classifier", "trainer", "oracle", "widely
     taught", "sharp eye" (game.rs). Pages (`hub/src/clsweb.rs`, templates
     classifiers.html, clscard.html, classifier.html): `/classifiers` grid (search,
     new/stars/downloads/featured), `/classifiers/{id}` (classes, metrics, preset, head,
     weights, builds-on, take/download/star, edit/delete for the author or an admin,
     admin verdicts), `/classifiers/upload`, edit/delete, `/admin/classifiers/{id}`, a
     "classifiers" section in /admin and on profiles, nav link. API:
     `GET /api/classifiers`, `/api/classifiers/{id}`, `/api/classifiers/{id}/download`,
     `POST /api/classifiers` (bearer token only). App: `POST /app/classifiers/{id}/take`
     (import_module, classes to lists by name, optional `as`/`lists`) and
     `POST /app/classifiers/share` (`{name, title?, description?, tags?}`, exports with
     upstream bundled). Ids checked (`store::id_ok`) before any path is made.
     Screenshots (grid, detail, upload, admin, profile, dark grid) were taken into the
     scratchpad `shots-hubcls/` against a non-`--local` hub.
   - **web UI: done** (`hub/web/`; Playwright screenshots in the scratchpad
     `shots-web3/`, at 1280 and 390 px wide, no console errors; parity all the same):
     - classifiers view (Ctrl+7) in tabs, the choice kept (`anend.classifier.tab`):
       learn (training, steering), use (search, outputs as lists, similar), buckets
       (`views/clsmore.js`: "index the save" with a live progress bar and stop, a grid of
       each class's counts by tenth of sureness in the classifier's palette plus
       "uncertain", a click on a class or a column lists its texts with yes/no and "near
       this", near texts by meaning or by verdict from the memo), checks (validators
       editor: checked by / saying / at least / gate or scale, kept with "keep";
       disagreements with yes/no for the classifier and for each validator), share
       (download as `.anendcls` with description/author/upstream; share on the hub). The
       side has "bring one in" (a file: `classify.inspect` summary, its name here, each
       class to a list of yours or by name, then `classify.import`) and "from the hub"
       (browse/search/sort `/api/classifiers`, "take…" with the same class→list mapping
       through `POST /app/classifiers/{id}/take`, a warning when an upstream classifier
       isn't bundled and isn't in the save). Share on the hub needs a login (a `--local`
       hub says "log in to share on the hub").
     - editor drawer (`views/notecls.js`, Ctrl+I or the classifier icon by view/save,
       kept open or shut in `anend.drawer`): every classifier with a tick
       (`classify.set_index`), each ticked one's verdict as chips (class colour, bar by
       probability, the classes it counts the note in, validators passed or not) and ✓/✗
       on its top class. On a phone it sits under the pad.
     - colouring (`tints.js`): a palette picker in the memory view's head and at the
       space's top right (off by default, per view in `anend.tint.memory` /
       `anend.tint.space`), texts tinted by top class, strength by probability, grey when
       unsure, a legend with counts. Hidden offline.
     - filters view (`views/filters.js`, Ctrl+9, `#/filters/<name>`): presets at the side
       (what each reads, last count, whether it's in a list, delete); the tree: all of /
       any of groups, "not" on any node, "+ add a condition" (class with classifier,
       class, "at least" (its own threshold until moved) and "its top class"; words;
       sums; list; tags with any; memories/notes; another preset; a group), run live
       (debounced; by name when unchanged so counts are kept), keep, write to a list.
       Enter in the bar adds a words condition. Offline it says filters need a hub.
     - icons added: not, shield, near, bolt, alert (rasterized for the desktop too).
     - **hub bug to fix (hub/src, not touched here):** `classify.index` without `wait`
       panics spawning its thread: the thread name is `format!("index {}", job_key(..))`
       and `job_key` holds a `\0`, so `Builder::name` fails ("thread name may not contain
       interior null bytes") and the job stays "running" forever, refusing every later
       index until the hub restarts. The web app works around it by calling with
       `wait: true` while polling `classify.index_job` for progress; once the name is
       fixed it can go back to not waiting.
   - **Desktop: done** (14 Oct), using the core `Store` API directly; heavy work runs on
     threads through `src/jobs.rs` (`Work::spawn` returns a closure applied to the world;
     results from a closed save are dropped; `Db` holds an `Arc<Store>`, and closing or
     renaming the open save is refused while a job holds it).
     - `src/classify_ui.rs`, the classifiers view (Ctrl+7, top bar icon `classifier`):
       list with accuracy; new from lists (classes, "not" list, builds-on, preset, one
       class / labels); training on a thread with a live bar, the loss as bars, held-out
       metrics, stop, epochs and growth-round sliders, from-scratch; accuracy, macro-F1,
       log-loss, confusion grid, growth and history events; steering sliders (bias per
       class, temperature, threshold, gains, coupling per upstream link) with the bias
       report measured live on a thread and kept after the hand stops; search by class
       (memoized, validators applied; bar words narrow it) with ✓/✗ per result, "why"
       (each classifier's share with its own ✓/✗, and end to end); near texts by
       embedding or verdict; buckets: index the save with a moving bar and stop, counts
       per class with confidence histograms, open a bucket; outputs to a list; export
       `.anendcls` to Downloads, take in from Downloads / saves folder / home or a path
       typed in the bar; delete (asks).
     - `src/index_panel.rs`: the editor's classifier button opens a strip: pick the
       classifiers the note is indexed on, verdict chips in the class colours, ✓/✗.
     - `src/tint.rs`: "colour views" in a classifier's header (setting `tint_by` in the
       config, off by default): memory results get a class-coloured bar, space texts take
       the class colour (fainter: less sure), legend with a close button.
     - `src/filters_ui.rs`, the filters view (Ctrl+9): all-of / any-of groups,
       include/exclude per condition, class conditions with a min slider, words (from the
       bar), lists, tags, memories/notes, other presets; run on a thread; keep under a
       name; presets list with run, write-as-list, delete.
     - Demo save for trying it: `ANEND_SEED=<file.redb> cargo test -p anend seed_demo --
       --ignored` (three lists, notes, one long note).
     - Left: validators editor and disagreements (core has them; the web app shows
       them), hub browse/take of shared classifiers, sum conditions in the filter
       builder, a file dialog (no dialog crate; Downloads is used), the name inputs show
       their text indented after a redraw (cosmetic).
5. **Guide: done.** `docs/guide.md`, for players: what anend is, then every part task-first
   (notes, memories and gsearch, the space, lists, classifiers with training figures,
   steering and bias, feedback per classifier and end to end, stacking, buckets,
   validators, colouring, sharing; filters, saves and bundles, the forge, the hub's game,
   settings), a shortcut table for desktop and web with their differences, a glossary.
   Screenshots (web app, 1100 px, quantized PNGs, ~250 KB) in `docs/guide/`. The hub
   serves it at `/guide` (`hub/src/guide.rs`: built in, rendered with pulldown-cmark,
   text escaped, raw HTML shown as text, links only to its headings, the repository and
   https; pictures at `/guide/{name}.png`; tests check every `#anchor` and picture),
   linked from every hub page's footer, the README, and the web app's settings ("guide":
   `/guide`, or the GitHub copy on `main` offline). Left: the desktop app's help link
   (src/), and keeping the guide in step as features land (filters and validators on the
   desktop, the web space's editing and maps).
6. **Review pass** (security (hub upload and op limits, classifier file checks),
   ergonomics, visual consistency, and no stubs): **security and CI done, a first UX walk done.** The findings and what's enforced where are in `docs/security.md`.
   - **Fixed and tested (14 Oct):** `POST /api/saves` takes only a bearer token (it was
     open to CSRF through the cookie); a `--local` hub answers only to localhost, IP
     addresses and its `--url` (DNS rebinding) and warns when it listens beyond
     loopback; framing, nosniff and referrer headers on every answer, and
     `script-src 'self'` for the web app (its inline script is now `hub/web/early.js`);
     `/app/api` reads no body and `/app/ws` opens no socket without an owner; 4 calls at
     once and 8 sockets per member; `--quota` (1 GB) and 200 saves per workspace;
     training limits (threads, batch, lr, 2 jobs per member, cores-many per hub, 2 h),
     and a training thread that fails to start no longer leaves the job "running";
     steering checked when only measured; classifier files with windows overlapping
     past 3/4 refused; `/admin/server` isn't "local" behind a forwarding proxy; logins
     for unknown names take the same time as for known ones; filter presets are checked
     and run once each however often they're referred to (it was exponential), a loop
     from a bundle errors, and imported presets' size is checked; the forge takes at most 32 saves and
     `lists.stats` leaves its grid out past 1000 lists (offline engine too); markdown links with control
     characters before `javascript:` no longer get through (stored XSS through shared
     saves). Tests: `clstests::doors`, `app::tests::quota`, `appweb::tests::turnstiles`,
     `web::tests::local_hosts`, `server::tests::file_round_trip`,
     `classify::tests::job_folders`, `classifiers::tests::hostile_arguments`, `classifiers::tests::filter_presets_stay_cheap`,
     `classify::tests::save_and_load_round_trip`, `node hub/web/test/markdown.mjs`.
   - **CI (done):** ci.yml now also builds the hub and runs `node hub/web/test/parity.mjs`
     (must print "all the same") and `node hub/web/test/markdown.mjs` on Node 22. core/
     and hub/ are rustfmt-clean now; `cargo fmt --all --check` still fails on `src/`
     (the desktop's files, left to that work). The release workflow builds the whole
     workspace, and the web app's files (`early.js` too) are embedded by rust-embed, so
     it needs no change.
   - The note under item 4 about `classify.index` panicking on its thread name is stale:
     the thread names no longer carry the job key, and the web app doesn't wait.
   - **Left, in this order:**
   - **Quality sweep (14 Oct, first walk):** clippy clean, tests and parity pass, no
     TODOs or stubs in hub/ or core/. Playwright over every web view at 1280 and 390 px
     against a seeded `--local` hub (screenshots in the scratchpad `shots-review/`): no
     console errors under the new content security policy, no sideways scrolling, every
     top-bar and editor control reachable by Tab with a visible focus ring, Ctrl+1/2/3/7/8/9
     switch views. Fixed: the classifier's five tabs now fit across a phone (the last hid
     past the edge), the tags view said "no tags yet" twice, a list card's letters line
     ran out of its card.
   - **Left:** a deeper keyboard-only pass inside views (the filter builder, buckets grid,
     the space); settings' "share this save" on a `--local` hub offers a form that can
     only answer "log in to share on the hub" (hide it, or say so up front); "one class" /
     "labels" for a classifier's head reads oddly beside two classes, but the guide and
     the desktop use the same words, so change all three together if at all; the hub's
     own pages (not the app) weren't re-walked in this pass.

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
  - ~~the desktop app's classifiers view~~: done (item 4, "Desktop")
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
