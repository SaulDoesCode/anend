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
