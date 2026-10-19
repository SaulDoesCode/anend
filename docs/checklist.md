# The checklist

One linear list. Work goes down it in order, and each line is ticked when it is done. There
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
- **The user's ideas are built as given.** When a choice is needed and a default is
  reasonable, pick it, keep it reversible, build it, and list it in docs/wip/questions.md.
  Otherwise ask plainly and directly. Take answers as given. Own and fix what turns out wrong.
- **Pushes after every piece.** docs/next.md's Resuming block says which line is in progress
  and where it stopped.
- **Standing requirements:**
  - Core functionality first, usable visually, working code before write-ups.
  - The game experience has to be enjoyable to spend time in, fit for people to use.
  - Local-first: everything works offline without a hub; saves are complete, documented and
    exportable; models are deterministic and inspectable.
  - The game, logic and language matter most. The everyday gematria features get fixes, not
    new features, unless asked.
  - No glass ceilings: limits are settings, not constants.
  - **The desktop app is the primary target.** The web build follows it, not the other way
    round. Heavy 3D moves off three.js to wgpu (the renderer line).
  - **Models from scratch come first.** The project's own engine trains and generates from
    the save's data. Pretrained imports (safetensors, llama-style checkpoints, already in
    core/src/classify/pretrained.rs) stay an option, never the default path.
  - **Defaults are explicit.** When a convenient default narrows what the user asked for (a
    framework, a pretrained model, a limit, a schedule), it is named in
    docs/wip/questions.md, not adopted silently.
  - If it builds on the user's machine, that's enough; CI can wait until it matters.

## The list

- [x] Fixes from using the app, F1–F8 (19 Oct; commits in docs/next.md).
- [x] The typing bar: 15.5 cm minimum, items beside it or above, Enter lets go (#1–3).
- [x] One notes count; long notes type as plain text (#12, first part).
- [x] Space search modes: normal, classifier, random.
- [x] Save checkpoints.
- [x] The IME check (web).
- [x] The instance design written, with the user's answers 1–9 (docs/format.md).
- [ ] Rewrite the design section's body in docs/format.md so it agrees with answers 1–9 (the
      conflicts are listed in docs/wip/queued-requests.md), and delete the superseded parts.
- [ ] The GPU for classifying, not only training:
  - `classify.predict`, `search`, indexing, buckets and tints, batched through `GpuNet`;
  - a CPU fallback that says why;
  - `gpu.report`: a settings button that writes a JSON file with the adapter and driver,
    the limits, the exactness probe against the CPU for every kernel, and timings of a
    fixed training run and of predicting 10k texts on CPU and GPU. The user runs it and
    pastes the file.
- [ ] Starter classifiers whose correctness a machine can check:
  - **generated with rule-given labels:** number properties, gematria properties, English or
    not, spelled right or corrupted, sentence or fragment, and fruit, stones and animals;
  - **from public-domain data sets:** CC0 or public domain first (Gutenberg, US government
    works), then permissive licences with attribution noted, each licence checked, large
    sets fetched on demand;
  - each shows its held-out score.
- [ ] ▶ **usable:** fast classifier search on the GPU, with starters that show it working.
- [ ] Instances: placements, many scenes inside a save (a scene table), blocks with the
      scene, packed transforms with defaults costing nothing, the migration from today's
      tables (#5, F4, answers 1, 2, 6).
- [ ] Inheritance: placements follow their entry or prefab, kept in two tables, inheriting
      and detached; detach or modify chosen up front (answers 3, 8).
- [ ] Text placements carry a `u8` font size, 0 for the default (answer 6).
- [ ] Long texts with no length limit:
  - pieces as a storage detail;
  - the user's approach: separate typing segments, each updated on its own and stitched
    back, so only the segment being typed in is sent and re-highlighted;
  - visual splitting at word boundaries, 80 characters a line and six lines a piece;
  - every such limit a setting (#6, #12, answers 7, 9).
- [ ] Chains as their own entries (a title plus ordered members); a note placed in a scene
      shows as a chain of two (#8, answer 4).
- [ ] Merging works on whole things, with overlapping placements settled by deterministic
      physics (answer 5).
- [ ] ▶ **usable:** scenes in saves, copies that are safe, long texts that are smooth.
- [ ] The language: a readable syntax for the 5-bit format, mapping one-to-one; documented
      in docs/format.md (#11).
- [ ] The notes view as its code editor: highlighting, outline, go to definition, inline
      results, step debugging, completion (#11, #12).
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
- [ ] The rest of parity.md's missing and partial rows: the saves folder setting, system
      fonts on the desktop, the window frame colour, HSL colour editing, the space's full
      search line, WebGPU training in the browser (#24 of the old task list).

**Alongside the list, not as a line of its own:** the unified asset and ontology browser (#26)
grows a little with each new kind of data, rather than being built in one go.
