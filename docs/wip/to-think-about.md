# Things to think about

Open questions where the project's direction depends on judgement more than on code. None of
them is urgent; any answer, sketch or half-thought written into this file or brought to a
session moves the work.

## The language (the 5-bit format as something people write)

1. **What does a program look like on the page?** The packed form is fixed; the readable
   syntax is open. Lisp-like brackets, a stack language (Forth), indentation, or something
   closer to the texts themselves, where a program reads almost like prose with markers?
   Since programs are entries, they will sit in the memory next to ordinary texts and have
   sums like everything else. Should a program's gematria mean anything?
2. **What are the first ten words of its standard library?** Picking the smallest set that
   makes something interesting possible (a block that reacts, a pipe that colours, a scene
   that changes) says a lot about what the language is for.
3. **Errors in a deterministic world.** When a program fails, should it fail the same way
   every time and leave a trace that can be read like a text, or be allowed to recover?

## Taxonomies and pipes

4. **What does a "no" mean in a taxonomy?** If "English?" says no, is the text then Afrikaans,
   or "not English" with the question open? Trees answer one way, DAGs another; "none of
   them" lists already exist. Where does uncertainty go: a third branch, a threshold, a
   score carried down the tree?
5. **Do effects commute?** If a pipe colours by one classifier and sizes by another, does
   the order matter? Spell systems often let effects interact (fire plus wind). Is that
   wanted here, and what would two classifier effects "interacting" mean?
6. **What is the item in a pipe?** A text, a word inside a text, a span, a placement, a
   whole list? Word-level items make grammar and word types pipes too; whole-list items
   make set theory pipes. One item kind or several?

## Scenes and the game

7. **What is play here?** A walking simulator, a logic sandbox, a puzzle with goals, a
   garden that grows? Each asks for different first pieces: goals and rewards, saving and
   sharing, or slow change over time. Which experience is wanted first, for a person who
   has never seen the project?
8. **Superflight writes by flying.** Texts collected in flight become an entry, a chain or a
   list. What should the order and the speed of collection mean? Could the path itself
   (its shape, its length) become a figure like a sum?
9. **What is a text in 3D?** Billboards facing the viewer read best. Texts could also be
   carved on blocks (as textures), or be the blocks (a texture of the text's own glyphs).
   Which of these carry meaning in a scene, and which are decoration?
10. **Logic gates as texts.** A switch and an AND gate are prefabs. Could the inputs and
    outputs be texts (a gate passes on a text when a classifier says yes), so the logic
    world and the semantic world are one machine?

## Classifiers as instruments

11. **Badly trained models as fiction of the Real.** A classifier that scores poorly still
    tracks something. Should the app have a place for reading such models: what a weak
    classifier latched onto, shown as its own kind of result, rather than discarded?
12. **What would a corpus of one's own look like as a starter?** The bundled starters are
    rule-made or public domain. Which of the texts already gathered over the years would make
    the most telling first taxonomy?
13. **Tag sets outside linguistics.** Word types can be any set. Which non-standard tag set
    would show the idea best in a demo: rhyme-bearing, sacred, technical, numerical, or
    something of one's own?

## The project as a whole

14. **The first public version.** At a ▶ usable point the project can be shared. What
    should a newcomer see in the first five minutes so the idea lands: a note app, the
    gematria, a classifier learning from examples, a scene?
15. **What stays the same forever?** The 5-bit format, the sums, determinism and
    local-first are candidates for fixed points that everything else can change around.
    Writing those down makes later decisions easier.
16. **A day with the app.** Imagining an ordinary day of use (what gets opened first, what
    gets typed, what gets looked at) is the best test of whether the views are right.
    Notes from such a day, even imagined, would steer the polish more than any list.
