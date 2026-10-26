# Questions for the user

When a choice is the user's to make, a default is chosen so work isn't blocked, and
written here. Change any of them whenever you like; each one is built to be reversible.

## The instance design (docs/format.md, rewritten 20 Oct to agree with answers 1–9)

The answers left these open. Each is written into the design as its default.

1. **Font size steps.** A text placement's `u8` font size counts in steps of 1/50 of a
   world unit per em, so today's default text size (0.5) is 25 and the range is 0.02 to
   5.1. Old saved scenes' sizes are rounded to the nearest step on migration (the old JSON
   is kept, so nothing is lost). A finer step would narrow the range.
2. **What a detached text holds.** It refers to its entry at the version it was detached
   at (the entry's `edited` time, read back from history), rather than holding a copy of
   the text. So it stays an entry from memory (answer 1) and the text is stored once.
3. **Forgetting an entry that has placements** (the draft's question 8, which the answers
   didn't take up): F4's behaviour is kept. One confirmation names how many placements go
   and in how many scenes, detached ones included, and it is undoable.
4. **A long text in the memory view** (the second half of the draft's question 7): it shows
   its first visual piece (six lines of up to 80 characters) and opens into the rest.
5. **How a chain is stored.** A chain entry (mime `application/x-anend-chain`, 8) has a body
   that is a `REF` to its title entry, and its members are ordered `MEMBER` edges, as a
   list's are. A placed note is shown as a chain of two without making a chain entry.
6. **Scene hierarchy.** "Hierarchical features" is read as each scene having an optional
   parent scene, so scenes nest like folders. Prefabs are scenes placed inside other
   scenes.
7. **The room** is a scene of the save (the setting `scenes.room` names it) and keeps its
   blocks. Its texts stay a live spread from searches, as before, and are kept when the
   room is saved as a named scene. (Revised 20 Oct while building it: keeping every
   search's results as placements would fill the room with texts nobody placed.) Copies
   made in the room are still page-only until the room is saved.
8. **Old maps with `PLACE` edges** become scenes named by the map entry's text; the map
   entry itself stays in the memory as an ordinary entry.
9. **Positions are stored as plain f32s** (12 bytes a placement). A lossless compact
   encoding may come later, measured before it is kept.

## Reading texts on the GPU (checklist: the GPU for classifying)

10. **Where classifiers read texts by default: `auto`.** The GPU is used when it is not a
    software renderer and the exactness probe finds it bit-exact on every kernel in exact
    precision, so the results are the CPU's; otherwise the CPU. The settings' GPU section
    can pick the CPU, the GPU or the GPU fast instead. The other reasonable default is the
    CPU always, which is slower but needs no probe at start.
11. **The choice is for the machine, not the save.** It is kept in the app's settings and
    applies to the whole process (on a `--local` hub, to everything it serves), since
    whether a GPU exists depends on the machine. A shared hub reads on its CPU unless
    started with `--gpu`, and its members can't change it.

## Starter classifiers (checklist: starters a machine can check)

12. **"Not English" is Afrikaans.** The English-or-not starter contrasts common English
    words with common Afrikaans words (the language the word types misread). Other
    languages can be added as more classes.
13. **The public-domain sets are the bundled authors and four Gutenberg books** (Pride and
    Prejudice, Die Verwandlung, Les Misérables I, Max Havelaar) for a language starter.
    No US government works yet: none is bundled, and this container can't reach outside
    sites to choose and check one. The book numbers were chosen from memory; each fetch
    is refused unless the book's own `Language:` line matches, but the fetch itself is
    untested against gutenberg.org (the container's network refuses it).
14. **A starter writes its texts into the save** as memory entries and lists, so it trains
    like any other classifier and its texts can be looked at. A rule starter writes 400
    texts a class (800 to 1200 entries), which is what got most of them well above
    chance; making one in a save of its own keeps them apart from your texts. The other
    choice would keep them outside the save; that would need a second way of training.
15. **The default size is the compact preset**, as for a new classifier; the starter call
    takes `preset` to pick another.
16. **Starters train longer than a hand-made classifier** (at least 40 epochs, patience
    10). With the defaults (14 epochs, patience 4) early stopping on a few dozen held-back
    texts ended training within a second and left several starters near chance.

## Auto by measured speed (25 Oct, after the GTX 1070 Ti's report)

17. **Auto now also needs the GPU to be faster** (this narrows answer 10). On the first
    reading with each model size, it reads 192 fixed texts on the CPU and 32 on the GPU
    (all 192 when the 32 came within a factor of 2 of the CPU's rate), and uses whichever
    was faster. The rates are kept per machine and model size in the user's cache folder
    (`gpu-speed.json`), and the machine includes the build's version, so a new build
    measures again. The measuring costs about a second on a real GPU; it is skipped for a
    software renderer, which is always the slower. Other reasonable choices: measure on a
    larger batch (closer to a search over thousands of texts, where the GPU gains most),
    or let the user decide from the report alone.
18. **A stack reads on one device, chosen by its largest net.** A classifier that builds
    on others reads them all on the device its largest net is faster on, rather than
    choosing per net.

## Inheritance (checklist: inheritance, 25 Oct)

19. **A text's override is its text.** A text placement has no rotation or scale, so the
    one value an inheriting copy can shadow is the text it shows ("edit this placement
    only"); position and size were already each placement's own. Prefab copies will
    shadow their parts' fields when prefabs exist.
20. **Both kinds of copy are menu items side by side,** with no dialog in between. A
    detached copy of a copy with its own text keeps the entry's text, not the own text.
21. **A detached text keeps the entry's version by its edited time** and is shown from
    `graph history`. If that history were removed, it would show the oldest version kept.
22. **A size click is 0.1 units (5 steps of 0.02).** Single steps are too small to see;
    the stored value still has the full 0.02 resolution. The first click from "default"
    starts at the default's value (the "text size" slider).

## Long texts (checklist: long texts, 25 Oct, first part)

23. **Typing segments are cut at blank lines outside code fences, in one text box.**
    The user's approach (each segment updated on its own, only it sent and highlighted)
    is kept for what crosses to Rust and what is coloured; the editing stays one native
    text box, so selection across segments, the pad's undo, find and input methods work
    as before. Separate text boxes per segment would also be possible, at the cost of
    those.
24. **Showing pieces in the memory view: the first piece, then four more a click.**
    "all" lays out every piece (slow for a megabyte), so it's a click of its own. The
    space's label shows the first piece and how many more; opening the rest there is
    left to the reader and the details window.
25. **A rendered text has no size cap by default** (`limits.render_output_bytes` = 0), as
    answer 9 asks; before, rendering a text with refs or calls failed past 4 MiB. A program
    that doubles a string in a loop can now use a lot of memory before its step budget
    (10 million steps) stops it; setting the limit brings a cap back.
26. **The notes view's spelling check pauses at the save's spelling limit** (200 000
    bytes by default; it was 100 000 before typing in segments).
27. **The model input ceiling (`classify::io::MAX_LEN`) and the hub's 64 MiB message size
    stay as they are:** the first is a file-safety bound on a model's window, the second
    the hub's own limit, not a save's; neither limits how long a text can be.

## Chains (25 Oct)

28. **A chain hangs downward under its title, left-aligned,** like paragraphs on a page,
    eased into place each frame (the springs), and its members take no part in the
    spread's pull and push (the head stands for the chain). Laying them out left to
    right is the other reading; it would suit short phrases better than paragraphs.
29. **Adding a text to a chain takes that placement out of the room** (it now hangs in
    the chain); the entry is unchanged.
30. **Breaking a chain keeps its members in the room** as texts where they hung.

## Turning and bending blocks (26 Oct)

31. **Both bends move the block through its depth (its own z), as a curtain or a folded
    page does,** and stack by adding. A ripple's amount is its peak; its period's shape is
    a cubic bezier with ends at 0 (default controls 1 and -1, near a sine). The fold bends
    both sides of its line back by the same profile (default controls 0 and 1, a smooth
    rise); it could instead bend each side its own way.
32. **The bend is made on the geometry, not in a GPU shader,** so picking and ray hits
    follow it exactly with no second copy of the maths; a bent block has up to 512 slices.
33. **The gizmo's arrows still move along the world's axes; its knobs resize along the
    block's own.** Moving along the block's axes would be the other choice.
