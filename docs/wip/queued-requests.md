# Queued requests (for after the current parity round)

The user asked for these to be held until the current round is done, then picked up in order.

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
