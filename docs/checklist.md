# The checklist

One list of what's to be done, each line ticked when it is done. The order is a sensible
default, not a script: take lines in whatever order the work makes sense in (dependencies,
shared code, what unblocks what, what's cheap while nearby), and say why when departing from
it. There
are no stages and no dates. A **▶ usable** line marks where the lines above it add up to
something usable as a whole; at each of those, a pull request is offered (merging stays up
to the user) and the README and guide are written up.

Details for a line are in docs/wip/queued-requests.md under the item number given (#n), in
docs/format.md (the instance design and the user's answers 1–9), and in
docs/architecture.md (the whole).

## How the work goes

- **Ticking a line:** it is done when its checks pass:
  - the four local checks (`cargo fmt --all --check`, workspace clippy with
    `-D warnings`, `cargo test --workspace`, the wasm32 web build) and tests for what it
    adds;
  - a by-hand check of every visible change on the desktop (Xvfb) and the web
    (Playwright), with screenshots in app/docs/shots/;
  - docs/parity.md updated, with nothing removed without a line saying why.

  Then the next line starts. Nothing waits on the user.
- **The user's ideas are the aim; making them work well is the job.** Bring engineering
  judgement to them: how they fit together, what they need first, what makes them good to
  use. Their intent is kept, without literal transcription and without second-guessing it.
  When a real choice is the user's, pick a reasonable default, keep it reversible, build it,
  and list it in docs/wip/questions.md; ask plainly when no default is reasonable. Own and fix
  what turns out wrong.
- **Describe what things are for as the project means them.** UI text, docs, comments and
  commit messages say what a feature does and why it exists, at the scope it was asked for,
  never narrowed to one use case. A reference case, 20 Oct: the classifiers view said "A
  classifier learns to tell your lists apart". That misrepresented the project in three
  ways:
  1. Lists are only where the examples come from. Classifiers read any text, which is what
     classifier search, colouring and the reader's measures are for.
  2. Classifiers are one use of the transformer engine. The core is meant for more than
     classification: spec models, generation, taggers and critics, some still under
     construction.
  3. "Your" added a possessive nothing called for.

  When writing such a line, check it against docs/architecture.md and the features that
  exist, and leave out possessives and framings that limit a feature to less than it does.
- **Pushes after every piece.** docs/next.md's Resuming block says which line is in progress
  and where it stopped.
- **Standing requirements:**
  - Core functionality first, usable visually, working code before write-ups.
  - The game experience has to be enjoyable to spend time in, fit for people to use.
  - Local-first: everything works offline without a hub; saves are complete, documented and
    exportable; models are deterministic and inspectable.
  - The game, logic and language matter most. The everyday gematria features get fixes, not
    new features, unless asked.
  - Limits the project sets for itself (piece sizes, render caps, hub quotas) are settings
    with defaults. Real limits remain, and are stated where they bite: the user's hardware,
    what can be built and run there, and what the tools doing the work can manage.
  - **The desktop app is the primary target.** The web build follows it. Heavy 3D moves off three.js to wgpu (the renderer line).
  - **Models from scratch come first.** The project's own engine trains and generates from
    the save's data. Pretrained imports (safetensors, llama-style checkpoints, already in
    core/src/classify/pretrained.rs) stay an option, never the default path.
  - **Defaults are explicit.** When a convenient default narrows what the user asked for (a
    framework, a pretrained model, a limit, a schedule), it is named in
    docs/wip/questions.md.
  - If it builds on the user's machine, that's enough; CI can wait until it matters.

## The list

- [x] Fixes from using the app, F1–F8 (19 Oct; commits in docs/next.md).
- [x] The typing bar: 15.5 cm minimum, items beside it or above, Enter lets go (#1–3).
- [x] One notes count; long notes type as plain text (#12, first part).
- [x] Space search modes: normal, classifier, random.
- [x] Save checkpoints.
- [x] The IME check (web).
- [x] The instance design written, with the user's answers 1–9 (docs/format.md).
- [x] Rewrite the design section's body in docs/format.md so it agrees with answers 1–9 (the
      conflicts are listed in docs/wip/queued-requests.md), and delete the superseded parts.
- [x] The GPU for classifying, not only training:
  - `classify.predict`, `search`, indexing, buckets and tints, batched through `GpuNet`;
  - a CPU fallback that says why;
  - `gpu.report`: a settings button that writes a JSON file with the adapter and driver,
    the limits, the exactness probe against the CPU for every kernel, and timings of a
    fixed training run and of predicting 10k texts on CPU and GPU. The user runs it and
    pastes the file.
- [x] Starter classifiers whose correctness a machine can check:
  - **generated with rule-given labels:** number properties, gematria properties, English or
    not, spelled right or corrupted, sentence or fragment, and fruit, stones and animals;
  - **from public-domain data sets:** CC0 or public domain first (Gutenberg, US government
    works), then permissive licences with attribution noted, each licence checked, large
    sets fetched on demand;
  - each shows its held-out score.
- [x] ▶ **usable:** fast classifier search on the GPU, with starters that show it working.
      (20 Oct: built, and checked against the CPU on a software GPU, where reading is
      about 60 times slower than the CPU; the speed on a real GPU is unmeasured until the
      user's `gpu.report`. Offered as an update on pull request #8.)
- [x] Instances: placements, many scenes inside a save (a scene table), blocks with the
      scene, packed transforms with defaults costing nothing, the migration from today's
      tables (#5, F4, answers 1, 2, 6).
      (20 Oct: the scene table, packed placements, blocks in the room's scene, saved scenes
      on scenes, the migration, backups and whole-scene merges. Copies made in the live room
      stay page-only until the room is saved; see docs/wip/questions.md 7.)
- [x] Inheritance: placements follow their entry or prefab, kept in two tables, inheriting
      and detached; detach or modify chosen up front (answers 3, 8).
      (25 Oct: texts. A copy is made inheriting or detached from the menu; an inheriting
      copy can have its own text. Prefabs inherit when they exist, #19.)
- [x] Text placements carry a `u8` font size, 0 for the default (answer 6).
      (25 Oct: stored since the instances line; now set per placement from the menu and
      the details window.)
- [x] Long texts with no length limit:
  - pieces as a storage detail;
  - the user's approach: separate typing segments, each updated on its own and stitched
    back, so only the segment being typed in is sent and re-highlighted;
  - visual splitting at word boundaries, 80 characters a line and six lines a piece;
  - every such limit a setting (#6, #12, answers 7, 9).

  (25 Oct: typing segments in the notes view; pieces shown in the memory view and the
  space; records over 64 KiB stored in pieces; the limits as the save's settings. Simpler
  than format.md's design: pieces cut by bytes, edits rewrite a record's pieces whole,
  no `text.window` yet; see format.md "Long texts".)
- [ ] Chains as their own entries (a title plus ordered members); a note placed in a scene
      shows as a chain of two (#8, answer 4).
- [ ] Merging works on whole things, with overlapping placements settled by deterministic
      physics (answer 5).
- [ ] ▶ **usable:** scenes in saves, copies that are safe, long texts that are smooth.
- [ ] The language: s (docs/s.md, the user's spec) as the high-level form of the 5-bit
      format. Allocate the 5-bit format's unused escape space efficiently for s's tokens
      and suffix forms, and document the mapping in docs/format.md. Tail calls run in
      constant space, as s §6 requires (#11).
- [ ] The notes view as s's code editor: a right-aligned mode (the acting word stands at the
      right), highlighting by s's word classes (commands, suffix forms, literals, locals),
      outline, go to definition, inline results, step debugging, completion (#11, #12).
- [ ] A headless standard library over every app op, and `anend run` (#11).
- [ ] Classifiers and models as language primitives (#18).
- [ ] ▶ **usable:** scripting the project from the notes view and the command line.
- [ ] Taxonomies: classifiers in hierarchies that route a text to further classifiers,
      trained by hand in the UI, with accuracy per node (docs/architecture.md 1).
- [ ] Pipes: sources through filters to stacked effects, computed rather than stored; the
      filters page becomes the pipe editor (docs/architecture.md 2, 3).
- [ ] Views as programs: today's views rebuilt as presets, with parity.md's guards passing
      (docs/architecture.md 4).
- [ ] Tag sets of the user's own; word types for other languages through a language
      taxonomy; conlang checking trained by hand (#13, #10).
- [ ] ▶ **usable:** the taxonomy engine, with effects stacked onto views.
- [ ] The renderer on wgpu, with WebGPU on the web sharing the code (#25).
- [ ] Culling (#24); far billboards with deterministic fall-off (#22).
- [ ] Block rotation and packed storage (#5); the picture-to-texture pipeline (#7); the skybox
      (#23).
- [ ] Spheres and curved blocks (#21); lights baked onto blocks (#20).
- [ ] Local spreads inside staging blocks (#14).
- [ ] The visual spec builder, with no JSON typing needed (#4).
- [ ] ▶ **usable:** building scenes that look the way they are meant to.
- [ ] Blocks that cast rays and run programs (#15), with ray modes including last hit (#28).
- [ ] Keyframes with a bezier timing editor (#16).
- [ ] Subassemblies and the prefab menu (#19).
- [ ] Switch and logic-gate prefabs (#27).
- [ ] Audio emitters and the sounds table (#17).
- [ ] ▶ **usable:** logic-world scenes, with blocks acting on each other.
- [ ] First-person mode with Thief-like movement, the grapple, and superflight that collects
      texts (#9).
- [ ] Placing prefabs in first-person mode (#19).
- [ ] ▶ **usable:** walking through what has been built.
- [ ] GPU: reading above the CPU at classifier-search batch sizes; training overhead (one
      submission per step, kept buffers, AdamW on the device). Back burner (user, 25 Oct):
      CPU training is fine for now.
- [ ] The rest of parity.md's missing and partial rows: the saves folder setting, system
      fonts on the desktop, the window frame colour, HSL colour editing, the space's full
      search line, WebGPU training in the browser (#24 of the old task list).

**Alongside the list:** the unified asset and ontology browser (#26) grows a little with each
new kind of data.
