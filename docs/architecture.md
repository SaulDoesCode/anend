# The whole: a taxonomy engine with typed pipes, views as programs

This is the idea that holds the project together. Each stage builds toward it, so that
features stop being one-off hard-coded views that drift or get lost.

## In the user's words (20 Oct)

> classifiers in hierarchies can solve many problems here, like is it afrikaans, or english
> or a set of languages, finite set of questions yes/no that narrows down which relevant
> further classifiers determine things, they were meant to have materialized views the
> filter page, but that's also meant to be wholistically integrated such that it can be
> used, in the way, like for these grammar/spellcheck/word-type/particle-type.. it's a
> taxonomy engine, in that sense, the inter-view pipeline is also underdefined, since
> ideally we would be able to define visuals attached to results that are typed by
> classifiers without storing things about things, and arbitrarilly use and stack effects,
> like it's items in pipes in minecraft with filters or oblivion magic spell system, a
> gamified flow for using statical techniques, transformers, and spatialized views to enjoy
> turing complete effects with semantic categorization use, and essentially in some sense a
> ui story from a scripting/5-bit language escape space so that what views mean in 2D can be
> generalized and is not dependent in your hard coded work, as well the 3D view with its
> scenes and the separate spread functionality of how it works without the placements and
> blocks.. and the blocks with their programmability, yeah, semantic localism, turing
> complete, permaculture in latent-space surplus on transformer-example-tender, as
> main-feature testably

## Draft of the parts (not built yet; open to change)

1. **Taxonomies: classifiers in hierarchies.** A taxonomy is a tree (or DAG) of classifiers.
   - Each node is a yes/no question, or a choice among a finite set: "English? Afrikaans?
     another language?".
   - An answer routes the text to the next, more specific classifiers: the language decides
     which word-type tagger, spelling checker or particle classifier applies.
   - Grammar, spelling, word types and conlang checking become taxonomies, not separate
     features.
   - A taxonomy is an entry in the save. It is trained by hand in the UI (marking right and
     wrong, Skinner-box style) and shows its accuracy per node.
2. **Pipes: items flow through filters to effects,** like Minecraft item pipes with filters,
   or Oblivion's spellmaking.
   - **A source:** a search, a list, a scene, the notes, a taxonomy branch.
   - **Filters:** classifier verdicts, rules, sums, kinds, tags.
   - **Effects:** visual ones (colour, underline channel, label, size, glow, a force in the
     spread, a position) or actions (add to a list, run a program, play a sound).
   - Effects stack and compose. The forge's set operations and the filter builder are early
     pieces of this.
3. **No storing things about things.** Effects and verdicts are computed when needed and
   cached in the memo (derived and rebuildable at any time), never written into entries as
   facts. A materialized view is a cached pipe result, kept until its inputs change.
4. **Views are programs.** A view is a pipe plus a layout, written in the 5-bit language's
   readable syntax (stage C).
   - Today's hand-made views (memory, lists, reader, filters) become built-in presets of
     the same system, so a new kind of view doesn't need new Rust.
   - Parity is kept by turning each existing view into a preset that behaves exactly as now,
     with parity.md's guards still passing.
5. **The 3D world has three separate layers:**
   - the **spread**, which is free and physics-driven: pipe effects supply its forces, with
     no placements involved;
   - **scenes**, placed texts and blocks (Stage B's instance model);
   - **programmable blocks**, rays, events and programs (stage F).

   Pipes can drive all three.
6. **Testable as the main feature.** Every taxonomy node and pipe has held-out scores or
   rule-checked results (the machine-checkable starter classifiers are the first ones), so
   "semantic localism, Turing complete" is shown working, not claimed.

## Where it lands in the stages

- **Stage B** gives instances, which pipes can place.
- **Stage C** gives the language. Pipes, views and taxonomies get a readable syntax there,
  and the standard library exposes classifiers and models.
- **A new stage C2, right after C:** taxonomies and pipes as the shared ontology. The
  filters page becomes the pipe editor, and the built-in views are rebuilt as presets with
  parity.md guarding them. Then the conlang and word-type work (stage H) is built on
  taxonomies.
- **Stages D–G:** the renderer and scenes use pipes for their effects.

## Open questions

- Should a taxonomy node be allowed several parents (a DAG), or only one (a tree)?
- Should the pipe editor be graphical (nodes and wires), or a spell-crafting list like
  Oblivion's? Both could edit the same program.
