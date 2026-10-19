# Queued requests (for after the current parity round)

The user asked for these to be held until the current round is done, then picked up in order.

## Do first: fixes from using the app (19 Oct, evening UTC)

These come from the user using the app and go before every numbered item below.

- **F1. Space details window stays in the window.** When the window is resized, the movable
  details window is clamped back inside the viewport. It got lost off-screen after the window
  was made smaller. Clamp on resize, on drag and on restore from saved positions.
- **F2. Right-click menu on texts in the space view:** edit, pin/unpin, duplicate, delete, open
  in notes/reader, add to list, chain, copy, and the rest the details window offers.
- **F3. The bar lets go of focus after Enter in the space view** (still open; see item 3). The
  keys move again until the bar is focused.
- **F4. Duplicating and deleting copies safely.**
  - Double-click currently duplicates a text, and deleting a copy risks deleting the original
    entry.
  - A copy is an instance (a placement) of the same entry. Deleting a copy removes only that
    placement. Deleting the entry itself is a separate, clearly named action with a
    confirmation, shown when it is the last placement.
  - Show which placement is the original or primary, and how many copies exist.
  - Make double-click duplication deliberate (a modifier or the menu), or confirm it with an
    undoable toast.
- **F5. Space view camera and controls.** Right-drag look moved in a circle spins the whole view,
  so the rotation maths is wrong.
  - Use yaw and pitch kept separately (pitch clamped, no roll) or a proper orbit camera around
    a pivot, with no accumulated roll from composing incremental rotations.
  - Smooth damping, and sensitivity in the settings.
  - Make it feel smooth and fun.
- **F6. Saved scenes (spaces/maps) in the space view:** see the list of saved scenes, and load,
  rename and delete them, with a confirmation and undo for delete.
- **F7. The notes view opens with its sidebar shown,** not the blank page; remember the user's
  choice after that.
- **F8. GPU build warning.** `static SHARED: OnceLock<Result<Gpu, String>>` in
  core/src/classify/gpu/mod.rs:152 triggers `recursion_depth_exceeding_limit` (future
  incompatible) while proving `Gpu: Send/Sync` through wgpu's types. Fix it without raising
  the crate's recursion limit if a cleaner way exists, e.g. assert Send/Sync on a small
  wrapper, or keep the device in an `Arc` behind an explicitly typed accessor. Check that the
  gpu feature build is warning-free.

## 19 Oct, afternoon UTC

1. **Bottom bar width:** the typing bar has a minimum width of 15.5 cm. Use CSS `min-width: 15.5cm`
   and check its real size at 96 dpi and at other scale factors on the desktop and on the web.
2. **Everything else beside the typing bar, overflow above it:** the bar's other items (hints,
   counts, chips, the view's figures) sit beside the typing bar on the same line while they
   fit next to its 15.5 cm minimum. Only the items that don't fit move into a transparent
   bar just above it, so no space is wasted. Wide windows get one line; narrow windows or a
   crowded view get the overflow above. The typing bar keeps at least 15.5 cm, and its text
   never shifts as items move between the two places.
3. **Space view, after Enter:** pressing Enter in the bar blurs it, so the keys move again
   (moving mode). Typing starts only after the bar is focused again by a click or the focus
   key. Keep every bar action that Enter triggers.

Keep everything that exists: record each of these in docs/parity.md as must-keep, and check the
views touched against their previous behaviour before committing.

4. **A visual builder for spec models, with no JSON typing needed:** every part of a spec
   that currently has to be typed as JSON gets a UI control. That covers inputs, trunk and
   preset, heads and their classes, data sources (lists, pictures, inline), critics, `from`
   (pretrained), `device`, the seed, and the training options.
   - **Ergonomic and visual:** the spec's ontology becomes forms, pickers from the save's lists,
     pictures and classifiers, sliders and steppers, add/remove rows for heads, data and
     critics, and a diagram of how the parts connect. Each control explains itself where it
     sits, and validation errors show on the control they concern.
   - **Full coverage:** everything the spec format can express can be built without touching
     JSON. A test walks the spec schema and checks that each field has a control.
   - **The JSON editor stays,** as an advanced view kept in sync both ways with the builder.
   - **Same on desktop and web.**

5. **Spatial blocks: rotation, compact defaults, packed storage** (folded into the spatial-saves
   work):
   - **Rotation:** blocks can be rotated as well as moved and scaled. Add a rotation ring to the
     gimbal. Textures still tile correctly on rotated, scaled faces.
   - **Defaults cost nothing:** an instance's transform stores its rotation and scale as
     `Option`s. `None` means the default orientation (identity) and the default scale (1), and
     takes no bytes beyond a flag bit, so most instances are only a position.
   - **Packed storage for block and text entries:** reuse the project's existing bit-packing and
     self-reference encodings. They live in `core/src/graph/codec.rs`, `record.rs` and `postings.rs`.
     - Per instance: a flags byte or bits for which optional parts are present; positions as
       varints or quantised deltas, sorted within a key so neighbours delta well; a
       self-reference for an instance that repeats an earlier one's rotation or scale.
     - Text entries packed the same way where it pays off.
     - Old records still read correctly: version the format and migrate on open.
     - Tests: round-trips (including edge values) and size measurements in the commit message,
       e.g. bytes per instance before and after for 10k copies of one text.

6. **Very large texts in the space and memory views.** The project is not limited to short texts,
   neither by design nor by intent. Texts can be any length (a chapter, a book), so both views
   need a story for them:
   - **Space view:** a long text shows as a compact slug (title or first line, length,
     sum), never its whole body as a label. It opens into a scrollable reader-like pane, and
     zooming in can reveal more of it (paragraph by paragraph, not all at once).
   - **Memory view:** long texts are paged or folded into lines and paragraphs with a
     virtualised list. The underlines (gematria match groups) and word types are computed per
     visible window, not for the whole text on every frame.
   - **Everywhere:**
     - nothing loads, measures or renders a whole huge text at once when a window is enough;
     - the figures (sum, letters, word types) are computed incrementally and cached;
     - the bar, the editor and search stay responsive with a 1 MB text;
     - core stores and indexes long texts without per-text size caps. Remove or justify any
       cap found and record it in parity.md.
   - **Tests and benchmarks:** a 1 MB and a 10 MB text, with timings for open, scroll, search
     and spread recorded.

7. **Images to block textures, as a pipeline the app models.** The path from making a picture
   to texturing a block needs its own ontology in the app, rather than ad hoc steps:
   - **Sources:** generated pictures (spec models with a picture output), pictures added to the
     save, and rendered texts.
   - **Steps:** pick or generate, then crop, tile settings (repeat, offset, scale per face) and
     variants, then assign to a block kind or to one face.
   - **Shown in the UI:** sources, steps and what feeds what, using the visual-builder style of
     item 4, with previews at each step.
   - **Stored compactly:** textures are kept by content hash and shared across blocks. A block
     refers to its texture recipe, which is regenerated deterministically from its seed.
   - **Desktop and web alike.**

## The project plan from here (19 Oct, evening UTC)

8. **Chains of texts in the space's text and block modes.** A text can be linked in a chain with
   other texts, e.g. the paragraphs of one piece.
   - **Dragging:** dragging the head or any node brings the whole chain along. It follows with
     spring links and settles into a readable arrangement (reading order kept, spacing by each
     text's size, no overlap or scrunching), so a chain of paragraphs can be carried around as a
     unit.
   - **Model:** chains are stored in the save as ordered links between entries (the entry
     graph's edges), and work with the free spread and pins. Make, break, reorder and cut a
     chain from the UI.

9. **First-person mode in the 3D block and text space** (optional; the orbit/editor camera stays
   the default):
   - **Walking:** collisions with blocks and gravity, with Thief 3-like controls: walk, crouch,
     lean, mantle, sneak speed, and the keys rebindable through keys.rs.
   - **Grapple hook:** aim at a block or text, attach, swing and reel in, with rope physics.
   - **Superflight:** a Just Cause 2-like fly mode with fast flight and boosts. Flying through
     texts adds them to a buffer, in the order hit and shown as a trail. The buffer can be
     minted into an entry, a chain or a list, or typed into the bar, so you write by flying.
   - **Deterministic physics,** in fixed steps, the same on desktop and web.

10. **Grammar checking for constructed languages, by shaping the checker in the UI.** The
    transformers are general, so the spelling and grammar checker must not be preset-only.
    - **Teach it in the UI,** Skinner-box style: mark spans right or wrong, accept or reject
      suggestions, reward or punish the checker, build a conlang's lexicon and rules as lists.
    - **It learns:** the detector and the corrector train on that feedback, plus generated
      corruptions of the user's own conlang texts.
    - **Many languages:** each one is a set of classifiers and spec models saved with the save,
      chosen per note.
    - **Visible progress:** precision and recall over held-out marked spans.

11. **The 5-bit text format as a usable esolang.** core/src/graph/codec.rs is the 5-bit token
    codec, and graph/interp.rs is already Turing complete with recursion and `if`. Give it a
    high-level representation people can read and write: a surface syntax that maps one-to-one
    to the packed form, documented in docs/format.md.
    - **Notes view (the typing view) is its code editor:** written and edited there with code
      features: syntax highlighting, structure and
      outline, go to definition and references, inline evaluation and results, step
      debugging, errors on the node, completion from the standard library, formatting.
    - **Part of the whole ontology:** programs are entries, they reference entries, lists,
      classifiers, spec models, pictures, blocks and chains, and they can act on them.
    - **A standard library** with headless access to the project's functions: everything the
      app API's ops do (entries, lists, filters, classifiers, specs, pictures, space, blocks,
      hub), so programs script the project without the UI. Run from the bar, the notes view and
      a CLI (`anend run`), with the hub's limits on shared hubs.

12. **The notes view is the typing view, for long texts and code.** (Item 11 said "nodes view"
    by mistake; it means the notes view.)
    - **Long texts:** it is where large texts are written and read. Virtualised rendering, and
      highlighting and figures computed for the visible window, so it stays smooth with a 1 MB
      text (see item 6).
    - **Code:** it is also the code editor for the 5-bit esolang of item 11, a mode or a note
      kind with the code features listed there.
    - **Fix: one notes count.** The notes count shows twice in the notes view; keep one, in the
      most useful place, and record it in parity.md.

13. **Word types for text that isn't English.** The word-type labels misread non-English text,
    e.g. Afrikaans.
    - **Short term:** a confidence threshold below which no type is shown, plus a language guess
      per text (a small language-id classifier, or stopword and character n-gram statistics)
      that turns English-only rules off for other languages.
    - **Longer term, one holistic design:** word-type tagging, the grammar and spelling
      checker (b2ee841, 30a1ac9) and the conlang checker (item 10) become one trainable family.
      Each language has its own tagger, checker and corrector, chosen by the language guess or
      per note, and all are correctable in the UI.
    - **Custom word types, not only standard linguistics:** users define their own tag sets
      (names, colours, descriptions), e.g. a conlang's categories, a project's own ontology
      or tags like "sacred", "technical" or "rhyme-bearing". A tag set is kept with the save,
      can be chosen per note or language, and is trained from examples like any classifier.
      The built-in noun/verb/... set is only one tag set among others, with nothing hard-coded
      to it in the UI or the filters.
    - **Correcting word types should be fun:** click a label and pick the right type; the tagger
      learns, and its accuracy and progress are shown, with the same Skinner-box feedback loop
      as item 10.

14. **Local spreads inside staging blocks.** A block can define a zone, and the free spread can
    run inside that block's area only, so there can be aesthetic worlds with local spreads.
    Texts in the zone spread among themselves, bounded by the block, without special handling
    elsewhere. Zones can nest, and each zone has its own physics settings.

15. **Blocks that cast rays and run programs (causal dominos).**
    - **Rays:** clicking a block can cast a ray. A per-block menu sets:
      - whether it listens for events (click, ray hit, zone enter/leave, timer, a program
        signal);
      - where its ray goes: **sticky** (specific target blocks or texts), or **fixed** (a set
        direction that hits whatever it crosses), with an option to start loose and become
        sticky on first contact.
    - **Programs:** on trigger, the block runs a program, an entry in the project's 5-bit
      language (item 11), chosen per block.
    - **Chains of cause:** blocks act on each other like dominos and build stateful systems.
      One action moves blocks, so a fixed ray carried by an animated block triggers the
      blocks it sweeps across.
    - **Deterministic:** the same fixed physics step, and the same on desktop and web.

16. **Scripted changes to blocks, with keyframes.**
    - **What programs can change:** texture, position, orientation, scale, and procedures for
      moving.
    - **Keyframes, like CSS:** attribute values at moments, interpolated, with an animation and a
      timing function.
    - **Visual editing:** a cubic-bezier curve editor for the timing, a timeline of keyframes,
      and scrubbing and preview.
    - **Shared storage:** animations are entries, reusable across blocks.

17. **Audio emitters in scenes.** Place sound sources, with triggers on zone enter/leave or from
    programs, falloff and looping. Together with first-person mode (item 9) and items 14–16,
    this makes walking-simulator experiences that change, not static ones. Sounds come from
    files in the save, as a sounds table like pictures.

18. **Classifiers and transformers as primitives in the programming language.** Programs can
    call classifiers (predict, steer, feedback), spec models (run, generate) and searches, as
    standard-library functions with typed results. Blocks and scenes can then behave richly,
    e.g. a block that reacts to how a text classifies or that generates its next texture.
    The hub's limits apply on shared hubs.
