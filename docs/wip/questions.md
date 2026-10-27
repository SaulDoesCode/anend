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

## Merging (26 Oct)

34. **Settling keeps what was there and moves only what comes in,** each incoming thing in
    turn along the axis it overlaps least (keep semantics, done once at the merge). A
    spread-like relaxation that moves both would also be deterministic, but would move
    the user's own things.
35. **Combining scenes copies:** the combined scene stays as it was. A merge of two saves
    still brings a same-named scene in beside the target's as `<name> 2`; combining them
    is then one click.

## Textures, spheres and lights (26 Oct)

36. **A texture recipe is kept by the hash of its normalized JSON, its name beside it,**
    so two blocks made with the same choices share one row, and renaming doesn't change
    the id. A recipe from a text names the text's key; a merge gives it the target's key,
    so it becomes another recipe there.
37. **Variants shift the tile and (optionally) turn it; they don't regenerate pixels.**
    A block picks its variant from its number among its texture's blocks, so copies
    differ but each block always shows the same one.
38. **The sky is one recipe for the save's room (`space.sky`),** drawn as a sphere round
    the camera with the recipe's tile four times round and twice up; saved scenes don't
    carry a sky of their own.
39. **A sphere is a block shape, not a kind:** it keeps its texture and sides (as
    diameters); a bend or faces set on a box are dropped when it becomes a sphere.
40. **Lights are baked onto block vertices in the room, not stored as lightmaps.** The
    bake follows from the lights and blocks and is redone when either changes, so the
    save keeps only the lights. Storing the bake would only pay off for very large
    scenes. There are no shadows yet (a block doesn't shade another); that would be the
    next step for baked light. With no lights the room shades faces by direction as
    before.
41. **A staging zone holds the texts inside its box,** decided when texts are placed,
    dropped, or the zone changes; texts outside pass through it unaffected rather than
    being kept out. Moving a zone carries its texts, and may take in texts it passes over.
42. **The builder writes the spec's JSON back with its keys in alphabetical order** (the
    app's JSON maps are ordered by key), so a spec typed in another order comes back
    reordered after a change in the builder; nothing in it changes meaning. Keeping the
    typed order would mean ordered maps in every JSON the core writes.
43. **An animation poses a block as it plays without moving it in the save:** its gizmo,
    zone and baked light stay where it was put. Turns are degrees about x, y and z after
    the block's own turn; a texture switches at its key rather than blending.

## s programs (26 Oct)

44. **An s program is a note tagged `s`;** its body (below the `# title`) is the code. A
    run's files are the save's s programs, each under its title and its title with `.s`,
    so `greet.` runs the note titled "greet". What a run writes to a file stays in that
    run; the save's notes aren't changed by a program.
45. **Programs run as the spec's browser runtime does** (s §10, §14): `env` gives 404,
    `sleep` returns at once, `tcp` fails ("isn't available here"), `now` is the clock's
    milliseconds. The random numbers are seeded from the save's clock each run, so two
    runs differ unless the program sets the seed itself.
46. **A run's step limit is 2 million on a shared hub and 50 million on one's own
    machine;** past it the run stops with an error naming the limit.
47. **The code mode colours each word by its class alone** (commands, suffix forms,
    literals, numbers, library words, escapes, `:` locals). It doesn't follow
    definitions, so a word a program defines with `$` stays plain where it's used; that
    comes with the outline and go to definition (the rest of the code-editor line).

## Rays and programs (26 Oct)

48. **A program acts on its block by printing lines:** `move`, `place`, `turn`, `size`,
    `colour`, `texture`, `play`, `stop`, `signal`, `cast`, `keep` and `say`, each a word
    at the line's start. Other lines are its output. s itself gets no new words for this.
49. **A block with no program casts its ray when an event reaches it;** one with a
    program casts only when the program prints `cast`.
50. **Rays and causes run in core, against the blocks as kept** (boxes and spheres,
    turned; not their bends nor where an animation shows them), so a cause does the same
    thing on the desktop and the web. Texts aren't targets yet; their places come from
    the room's physics, which runs in the page.
51. **One cause sets off at most 200 events, sharing one budget of steps** (2 million on a
    shared hub, 50 million on one's own machine), so blocks that hit each other stop.
    Events run in the order they were set off (first in, first out); a ray's hits go
    nearest first.
52. **What a cause changes is kept in the save** (a lifted block stays lifted after a
    reload), and Ctrl+Z doesn't take it back yet. A program's state is the `keep` text
    in its block's wiring.
53. **Sticky targets are block addresses,** so a target retextured (by hand or by
    `colour`) drops out of them, as its address changes. A merge drops sticky targets,
    since the merged blocks come in under new addresses.
54. **A wiring is shared by copies of a block until one is changed,** like an animation.
    Changing it from one block gives that block its own entry.
55. **Playing is a state of the room, not the save:** it starts off each time.

## Prefabs (27 Oct)

56. **A prefab's origin is the middle of its blocks' boxes,** so a copy turns about its
    middle; the first copy stands where the grouped blocks were.
57. **A part changed in a copy (by hand or by a program) changes in that copy only:** a
    switch's lamp lifted in one copy stays down in the others. The copy can drop its own
    values or make them the prefab's.
58. **"…but not the other copies" makes the old prefab a prefab of its own,** named
    `<name> (before)`, which the other copies then show; that's what a detached prefab is
    in format.md.
59. **A copy's own values don't travel in a merge;** the copy arrives showing the prefab.
    Prefabs keep their inner links through a merge; links to the source room's blocks are
    dropped, as a merged block's are.
60. **Prefabs are left out of the scenes window** (and of `#name` scene search); they have
    their own window.
61. **Grouping takes only room blocks,** not parts of other copies (no prefab inside a
    prefab yet), and a prefab's set of parts can't be edited after it's made: change a
    copy and make its changes the prefab's, or detach, edit and group again.

## Switches and gates (27 Oct)

62. **A hit or a signal carries a value: the first word of the sender's kept state,**
    given to the program as its fourth argument (a switch keeps 1 or 0, a gate its
    output first). A click, a timer or a zone event carries none.
63. **A gate keeps each input's last value by who sent it,** so it takes any number of
    inputs. AND is on while two inputs are on (not "all of them", since a gate doesn't know
    of an input until it hears from it); OR while any is; XOR while an odd number are;
    NOT while none is.
64. **The latch flips each time an input turns on, and holds** (a toggle), rather than
    having separate set and reset inputs.
65. **The starters' rays go out of their right side (+x) at the copy's height,** so a row
    wires itself; the switch's paddle tilts about that axis, so its ray stays level. The
    door swings a quarter turn rather than lifting, so the ray that opened it still
    reaches it to close it.
66. **The gate labels are texts of the save** (AND, OR, XOR, NOT, LATCH), shown on a
    plate above each gate, so they appear among the save's texts.

## Sounds (27 Oct)

67. **Sounds play only while the room plays** (the wiring window's ▶ play), so building is
    quiet: looping emitters start with it, the others when a program prints `sound`, and
    stopping play silences everything. Zone enter and leave reach a sound through a
    program (`{sound} >`), not as a separate setting on the emitter.
68. **A sound fades linearly to nothing at its reach** (Web Audio's linear distance model,
    full volume within 1 unit), panned by where it is from the camera; no echo or
    occlusion.
69. **The save keeps sound files as they were added;** the page decodes them, so a format
    the browser or the desktop's web view can't play is reported when it's first played.

## First-person mode (27 Oct)

70. **The keys:** walk V, superflight B, grapple G, sneak Z (held); walking reuses the
    flying keys (W A S D, Space to jump, C to crouch, Q and E to lean), so one set of
    keys moves you in every mode. All of them rebind in the keys menu.
71. **The body:** 0.6 wide, 1.7 tall standing and 1.05 crouched; walking 3.2 units a
    second, running 6, sneaking 1.4, crouched 1.6; a jump of 6.2 under a gravity of 20;
    steps up to 0.35 are climbed, ledges up to 1.3 mantled. The floor is the room's grid.
    Blocks are bumped into as their boxes (turned and bent ones by the box around them);
    staging zones and texts let you through.
72. **The grapple hooks within 60 units** (a block where you look, else the text nearest
    where you look), the rope as long as the distance; the wheel reels it from 1.2 to 60.
73. **Superflight passes through blocks** and collects a text when its path comes within
    0.6 units plus the text's size; a text is collected once. A chain or a list takes the
    collected texts that are entries of the save (free texts join only the new entry or
    the bar).
74. **No pointer lock on the desktop:** under Xvfb, WebKitGTK stopped passing key presses
    on once the pointer was locked, so the desktop keeps right-drag looking.
75. **Placing by hand snaps to half units** across the surface (Shift: free) and rests the
    ghost against the face it points at; with nothing under the view it hangs 12 units
    ahead, since blocks float in the room anyway. A click where it would overlap a block
    is refused rather than settled.

## The code editor (27 Oct)

76. **Inline results go at the left of the line whose token wrote them** (in the
    right-aligned code, that space is free); output written inside a call is put beside
    the program's line that made the call. They stay until the text changes.
77. **Stepping runs the program again from the start to the step asked for,** with one
    seed for the whole session, rather than holding a paused run: every step is the same
    run, at the cost of time on long programs (one run's steps at most).
78. **Go to definition is Ctrl+click** (a word's script or `><` definition in this program,
    else the program it names); completion offers at most eight words, shortest first, and
    Tab takes the first (Tab moves on as usual when there's none).

## The standard library and `anend run` (27 Oct)

79. **The standard library is one module, `std:anend`, with one word per kind of thing
    rather than a word per op:** `op` takes any op's name and JSON arguments, so every op
    the app has (and every new one) is there at once; `json`, `#json`, `obj` and `quote`
    read answers and build arguments. Answers stay JSON text, since s's values are text.
80. **A program can't start a run through `op`** (`s.run`, `s.debug`, `wiring.fire`), so
    runs don't nest without bound; programs call each other with `name.`.
81. **The command line is `anend-app run`** (the desktop binary, which already opens saves);
    the program is a file, else the save's program of that title; output on stdout, an
    error on stderr, the program's exit code, and piped stdin as its stdin.
82. **`Run <title> [arguments]` in the bar** (the memory view and the space) shows the
    output as a toast (it can be clicked into the bar) and an error in the top bar.
83. **Classifiers and models get a module of their own, `std:learn`,** whose words answer
    in s values (a record for a classifier's answer, lists for texts, numbers for
    probabilities and scores) rather than JSON, so a program needs no `json` to use them.
84. **A class is named as the classifier names it** (a starter's carry its name, `fruit,
    stones and animals: fruit`), so the name `class` gives back works in `feedback`,
    `steer` and `likely` unchanged.
85. **A block generating its next texture waits:** a generate head writes text, and a
    texture is a recipe; a model that writes recipes would be its own piece of work.

## Taxonomies (27 Oct)

86. **A taxonomy is a tree, not a DAG** (architecture.md's open question): a node has one
    parent, and the same classifier can stand at several nodes, which covers the shared
    cases without a node's counts mixing texts that came by different ways.
87. **Routing follows each node's top class.** A multi head's other classes don't branch
    the routing; one text gives one path.
88. **Marking is the classifier's feedback too:** right is a yes for the class given;
    wrong is a no for it and, when the right class is picked, a yes for that one. A node
    keeps only its counts (right, wrong), in the taxonomy's own entry; nothing about the
    texts is stored, and routing reads them afresh each time.
89. **A node's accuracy is shown two ways:** by hand (right out of marked, at that node)
    and its classifier's held-out accuracy. Saving keeps a node's counts while the same
    classifier stands at the same place.

## Pipes (27 Oct)

90. **A pipe's source and filter are one thing, the filter builder's expression** (words,
    sums, lists, tags, kinds, classifier verdicts, kept filters); an empty one lets every
    text through. A scene or a taxonomy branch as a source waits.
91. **Pipes are turned on per viewer** (the app's settings, like the colouring
    classifier), for the memory view and the space, and stack in the order turned on: a
    later colour, underline or glow wins, sizes multiply, labels join.
92. **Effects are worked out when a view asks**, from the classifiers' memoized outputs;
    nothing is cached beyond that memo, so an edit or a retrain shows at once.
93. **Actions run only when asked** ("act"), never as a view is drawn: a list (by name,
    made when missing), a tag, or an s program on each text (at most 200 a time). A sound
    as an action waits for sound outside the space; programs can't apply pipes themselves.
94. **In the space, a pipe's force is "gather"**: its texts pull together in the spread
    and away from the rest. Other forces and set positions wait for the views as programs.

## Views as programs (27 Oct)

95. **The reader hosts views; the other views stay their own code.** A view is a source
    (a pipe, or a search), an order, headings, a layout and effects, which is what the
    reader is, so its three views became presets of that and any view program shows
    there. The memory view (gsearch, buckets, picking lists and candidates), the lists,
    the space and the editors are tools with their own interactions; rebuilding them as a
    source and a layout would lose those, so they take pipes' effects instead.
96. **A view is an s program that loads `std/view.s.`** (no tag of its own), and its words
    set fields rather than draw; the reader draws. Its run gets the bar's words as its
    first argument, and the bar's words narrow whatever it says.
97. **Layouts are cards, a tight grid, a list, and auto** (the reader's old choice between
    cards and the grid); headings group by kind, a sum, length, or a classifier's classes.

## Views as programs (27 Oct)

95. **The reader hosts views; the other views stay their own code.** A view is a source
    (a pipe, or a search), an order, headings, a layout and effects, which is what the
    reader is, so its three views became presets of that and any view program shows
    there. The memory view (gsearch, buckets, picking lists and candidates), the lists,
    the space and the editors are tools with their own interactions; rebuilding them as a
    source and a layout would lose those, so they take pipes' effects instead.
96. **A view is an s program that loads `std/view.s.`** (no tag of its own), and its words
    set fields rather than draw; the reader draws. Its run gets the bar's words as its
    first argument, and the bar's words narrow whatever it says.
97. **Layouts are cards, a tight grid, a list, and auto** (the reader's old choice between
    cards and the grid); headings group by kind, a sum, length, or a classifier's classes.

## Tag sets (27 Oct)

98. **A tag set learns from words alone** (its classifier reads each word on its own, not
    its neighbours): marked words are few, and a word's own letters carry most of what a
    small set can learn. Context-aware tagging can come with a spec model later.
99. **A set's marked words are lists** (`<set>: <type>`), so they show, merge and back up
    like any list, and the set's classifier (`tags: <set>`) is an ordinary classifier.
100. **Training a set starts from scratch, quick and brisk** (200 epochs at 0.01), with
    nothing held out under forty marked words (holding three words out stopped it at once);
    from forty, a fifth are held out and its precision and recall are on those.
101. **English types only on English:** a text gets the built-in types when 60 % of its
    words are English ones and, from four words, it has one of English's commonest words.
    In the browser build, without the bundled lexicon, the word lists the rules use stand
    in for it (it errs towards English).
102. **A checker's suggestions are the nearest words marked right** (edit distance ≤ 3, the
    three nearest), not a trained corrector; #10's corruption-trained corrector waits.

## Prefabs as one group (27 Oct)

103. **A copy's gizmo stands at the middle of its parts,** and turning turns about that
    middle; the copy's own origin (where the prefab's zero lands) is carried along. Size
    stays a step in the prefabs window, not a knob, since a copy scales uniformly.
104. **Opening a copy is a double-click** (as entering a group in drawing tools and
    Logic World's boards is), or its menu; Esc or a click outside closes it.
105. **An undone removal is a new copy** with the same place, turn, size and own changes;
    rays from outside that aimed at its parts aim at the old address until re-aimed.
