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
7. **The room** is the save's first scene, named "room", and keeps its texts and copies
   between sessions.
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
