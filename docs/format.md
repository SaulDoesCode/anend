# The entry graph format

How `core/src/graph/` stores a save. Everything is an **entry** (a memory text, a note, a
list's name, a tag, a map, a function) under a u64 id, and every relation between
entries is an **edge**. Derived indexes (sums, words, letters, time, hashes, mentions)
can always be rebuilt from entries alone. The module docs hold the same detail next to
the code.

## Ids and weum keys

weum keys (1–4 bytes, shown as `3,14`) map to ids in weum's allocation order: `[0]..[255]`
are 0–255, `[0,0]..[255,255]` are 256–65791, then three and four byte keys. So id `n + 1`
is weum's `next_key` of id `n`, and every id under 4,311,810,304 has a weum label
(`record::key_of`, `id_of`, `label`, `parse_label`). New ids reuse forgotten ones first,
the first in weum's key byte order (as weum's `reusables`), else the one after the last
allocated. Ids stay under 2^48.

## The text codec (`codec`)

5-bit tokens, bit-packed most significant bit first:

| token | meaning |
|---|---|
| 0 | space |
| 1–26 | `a`–`z` (`A`–`Z` while caps lock is on) |
| 27 `ESC` | 8 payload bits follow (below) |
| 28 `UPPER` | the next letter takes the other case; `UPPER UPPER` toggles caps lock |
| 29 `REF` | an entry id as a varint: transclude that entry's rendered text |
| 30 `CALL` | a callee varint, an argument count varint, then per argument a bit-length varint and that many bits of a nested run |
| 31 | newline |

- **Varint:** 5-bit chunks, low group first; bit 4 says more follow, bits 0–3 are data.
- **Escape payload `b`:** `0x00–0x7F` that ASCII byte; `0xC2–0xF4` a UTF-8 lead byte with
  its continuation bytes as 6 bits each (it becomes the run's last lead); `0x80–0xBF`
  `b − 0x7F` characters sharing the last lead, continuation bits only; `0xF5` a run of 2–33
  digits (5 bits for `n − 2`, then 4 bits a digit); anything else is reserved.
- **Callee:** under 32 a builtin, else entry `id + 32`.
- **Runs:** caps lock and the last lead reset at the start of each run (a body, each
  argument), so arguments decode on their own.
- **Body:** a 3-bit header holding the padding bit count, then the run; empty is 0 bytes.

Any UTF-8 round-trips exactly. Measured (packed / UTF-8): lowercase prose 0.63, mixed
case markdown 0.86, Hebrew 0.61, Russian 0.85, Japanese 0.88, digits 0.87, polytonic Greek
1.07 (it alternates 2- and 3-byte characters, so the shared-lead runs rarely apply).

## Rendering and the interpreter (`interp`)

Rendering a body decodes it, transcludes `REF`s and evaluates `CALL`s. A function is any
entry (by convention mime `application/x-anend-fn`) whose body is a template; it reads
its arguments with `arg(n)`. Arguments are evaluated before a call, except `if`'s
branches. Builtins (codes): `arg` 0, `if` 1, `eq` 2, `concat` 3, `len` 4, `sum` 5
(ordinal or reversed), `slice` 6, `lookup` 7, `add` 8, `sub` 9, `mul` 10, `div`
11, `mod` 12, `lt` 13, `not` 14, `argc` 15; 16–31 reserved. Numbers are decimal `i64`
text (empty is 0); truth is non-empty text.

**Tail positions.** Evaluation runs on an explicit continuation stack, never native
recursion. A call in tail position replaces its frame instead of pushing one:
- the last segment of a run (a function body, an `if` branch, an argument, an entry's
  body): text before it is already in the frame's output, which the call continues;
- the branch `if` takes, when that `if` is in tail position;
- the last argument of `concat`, when that `concat` is in tail position.

So tail recursion runs in constant stack and memory, bounded only by fuel (a counter to
1,000,000 and mutual even/odd recursion are tests). Any other call pushes frames, up to
the depth limit, past which evaluation stops with an error. Limits (all errors, never
panics): fuel, depth, output size; refs that transclude themselves are an error; calls
recurse freely. Non-tail entry calls are memoised by (entry, arguments).

## Records (`record`)

An entry's record: `version (1)`, `flags`, then LEB128 varints `created ms`,
`edited − created`, `mime`, then the packed body. Flags: 1 *plain* (text plus borrowed
refs only: its rendered text changes only when it's edited; it's in the hash index), 2
*has refs*, bits 4–7 the borrow depth. Mimes 0–7 are built in (`text/plain`,
`text/markdown`, `application/x-anend-fn`, `-tag`, `-list`, `-map`, `-classifier`,
`application/json`); others are registered from 64.

**Blocks.** Entries are stored 32 to a row: key `id >> 5`, value a u32 bitmap of occupied
slots, a varint length per occupied slot, then the records. Reading one entry reads one
row and skips to its slot.

**Past versions** (`graph history`, key `(id, edited ms of that version)`): byte 0
version, byte 1 kind — 0 a whole record, 1 a delta: varint mime, prefix and suffix byte
counts kept from the next newer version, then the packed middle. An edit of a plain
entry stores the old text as such a delta, usually a few bytes.

## Borrowing

Saving a plain text probes the content hash index for existing plain entries equal to a
prefix ending before a space or newline (longest first) and to a suffix starting after
one, and stores `REF prefix + rest + REF suffix` when that's smaller (each borrow also
costs a mentions row). Sources must be within the depth limit and must not themselves
read the entry being saved. Rendered text never changes: when a source is edited or
deleted, every plain entry borrowing from it is stored again from its old rendered text.
Saving a text equal to an existing entry's (same mime) returns that entry.

## Edges

`graph edges`: multimap `(from, relation)` → `to` (8 bytes big endian) ++ payload;
`graph edges in`: multimap `(to, relation)` → `from`. Relations: 1 NOTE (a note entry
annotates an entry, as weum's `notes`/`noted on`; notes read oldest first), 2 MEMBER (a
list holds a member; payload a u64 order key), 3 TAG (an entry carries a tag entry), 4
PLACE (a map places an entry; payload a 34-byte transform, see `place`), 5 VALIDATES;
256 and up are free.

**Placements** (`place`): version 2, kind, flags, reserved, position 3×f32, rotation in
32 bits (smallest three, 10 bits each), scale 3×f16, RGBA8, asset u32. Packing is
canonical: re-packing a value always gives the same bytes (the rotation search runs
quantise∘dequantise to a fixed point, or the smallest value of the cycle it falls in).

## Derived indexes (`postings`)

One row per (term, id chunk), not per (term, id): the value is the chunk's ids
(`id >> 16`) as LEB128 differences after a 0 byte, or a 65536-bit bitmap after a 1 byte
when smaller.

| table | key | holds |
|---|---|---|
| `graph sums` | (mode, value, chunk) | ordinal (mode 0) and reversed (mode 1) sums |
| `graph frequency` | (letter-frequency-key hash bucket, chunk) | anagram classes (readers check the key) |
| `graph words` | (word bytes, chunk) | tokens from the SIMD tokenizer |
| `graph created` | (created ms >> 22, chunk) | creation time buckets (readers check exact times) |
| `graph mentions` | (entry, chunk) | who refers to or borrows from an entry |
| `graph hashes` | top 14 bits of the content hash | `(next 32 bits, id)` pairs |

**Index modes** (`graph meta` `index`): `disk` keeps the tables above; `memory` keeps
only entries, edges and history on disk and rebuilds the indexes into memory at open.

## Other tables

`graph free ids` (weum-ordered keys of forgotten ids), `graph meta` (`format`,
`allocated`, `index`), `graph settings` (name → bytes), `graph mimes` (name → id).

What a save holds besides entries (`core/src/graph/assets.rs`): `graph spec models`
(model name → its record as JSON: the spec as written, created, modified, version,
trainings; [models](models.md#where-a-save-keeps-its-spec-models)), `graph pictures`
(picture id, a 128-bit FNV-1a hash of its bytes as 32 hex digits → `{id, mime, width,
height, name, added, size}` JSON), `graph picture bytes` (id → the PNG or JPEG file) and
`graph picture lists` (list name → picture ids, a multimap).

## Backup

Two files. `texts.json`: `[[text, [key bytes], created, edited, mime?], ...]` (the first
two elements are weum's `all-of-it.json` rows, which import the same way). `other.json`:
`format`, `version`, `allocated`, `free`, `mimes`, `sources` (structured bodies as JSON
runs), `edges`, `history` (past versions as text), `settings`, `specs` (spec model
records), `pictures` (`[{id, mime, width, height, name, added, size, data: base64}]`)
and `picture_lists` (`[[list, id]]`).

## One model for a definition and its placed instances (Stage B design, not built yet)

This is the design for Stage B of docs/wip/queued-requests.md, written before any of it is
built (19 Oct). Today the same idea is stored four ways: text copies in the room are
`entry~2` node keys that only live in the page until a scene is saved; saved scenes are one
JSON setting (`space.maps`: name → rows `[text, x, y, z, {key, size}]`, the text repeated in
every row); blocks are 32-byte values in `graph blocks` plus a `graph block ids` row each;
and core's `PLACE` edges carry a 34-byte placement. Stage B puts all of them in one table.

### The model

- A **definition** is what gets placed: a text (an entry), a picture, a plain colour, a
  free text (a room text that isn't in the memory), or a **container**.
- A **container** is an entry whose instances live inside it. One kind of thing covers
  three uses:
  - a **scene** (mime `map`, 5, as now; its text is the scene's name);
  - a **prefab** (a new built-in mime 8, `application/x-anend-prefab`): a scene that can
    itself be placed. Its parts' transforms are relative to its origin;
  - a **chain** (a new built-in mime 9, `application/x-anend-chain`): a container whose
    instances are in reading order (by their number, below). The space links neighbours
    with springs. The stored transforms are the chain's resting layout relative to its
    head.
- Container 0 is **the room**: the save's own space, which holds today's blocks.
- An **instance** is one placement of a definition in a container. Its address is
  `(container, definition, n)`, where `n` is its number among that definition's instances
  in that container: 1 for the first, then 2, 3, ... (the next free number). This is F4's
  `entry~2` made durable. The first placement is n = 1, and the copies count is how many
  numbers are present.
- Placing a prefab or a chain is one instance of its container's definition. The space
  draws it by expanding the container's instances under that instance's transform. The
  depth is limited, and placing a container inside itself, directly or not, is refused
  (the same rule as borrowing's cycles). Each placed copy addresses its parts as
  `outer instance / inner address`. So a prefab's internal links (a sticky ray target
  inside the group, item 19) are stored relative to the container, and they work in
  every copy without being remapped. Editing a prefab changes every copy. To keep a copy
  as it was, it is *detached*: the prefab is copied to a new container, and the chosen
  instances point at the copy.

### Tables

| table | key | value |
|---|---|---|
| `graph instances` (multimap) | `(definition ref: &[u8], container: u64)` | one value per chunk of up to 64 instances (numbers `64c .. 64c+63`), below |
| `graph container defs` (multimap) | container `u64` | the definition refs it holds instances of (one row per pair, not per instance) |
| `graph free texts` | free text id `u64` | its body, packed with the text codec (a 3-bit header, then the run) |
| `graph meta` `instances` | | the next free text id (varint) |

The key starts with **what is placed**, so a definition's instances in every container are
one key range: `(def, 0) .. (def, u64::MAX)`. That range answers "how many copies, and
where" (the F4 count, a forget-the-entry confirmation, "which chains hold this text").
`graph container defs` gives a scene's definitions, so loading a scene reads its keys and
nothing else.

**Definition ref** (byte order is the sort order):
- `0` then a sized id: an entry (a text, a prefab, a chain);
- `1` then 16 bytes: a picture, by its 128-bit content hash;
- `2` then `r g b`: a plain colour;
- `3` then a sized id: a free text.

A **sized id** is one length byte (0–8) followed by that many big-endian bytes, with no
leading zero bytes. Comparing the bytes therefore compares the numbers, and entry
`3,14` (id 1038) takes 3 bytes: `02 04 0e`.

**A chunk value:** first the chunk number `c` as a sized id, so a key's chunks sort in
number order. Then a bit stream, most significant bit first, as the text codec writes it:
a 3-bit padding count, then one record per present instance in number order. Varints are
the codec's 5-bit varints (four data bits a chunk, low group first); `zz` is zigzag.

| field | bits | meaning |
|---|---|---|
| gap | varint | `n − previous n − 1` (the first: `n − 64c`) |
| R | 2 | rotation: 0 none (the identity), 1 own (32 bits, `place`'s smallest three), 2 same as an earlier instance (varint `d`, that instance is `n − d` in this chunk), 3 reserved |
| S | 2 | scale: 0 none (1 on every axis), 1 uniform (one number), 2 per axis (three numbers), 3 same as `n − d` (varint `d`) |
| C | 1 | colour present: 32 bits RGBA8 |
| F | 1 | flags present: a varint (1 pinned, 2 hidden, 4 spread, 8–64 the block shape as `place::Kind`, the rest kept) |
| X | 1 | extra present: a varint length, then that many bytes (an asset id, a chain link's spring, later a block's program; unknown extras are kept as they are) |
| P | 1 | position mode: 0 grid, 1 raw |
| position | | grid: three varints `zz(q − previous q)`, `q = round(v·100)` per axis (the previous instance in the chunk, or 0); raw: three f32 (32 bits each) |
| numbers (scale) | | per number: 1 bit grid/raw, then `varint zz(round(v·100))` or 32 bits f32 |

**Defaults cost nothing.** No rotation, unit scale, no colour and no flags take 6 bits,
so most instances are a gap and a position.

**Lossless.** The grid is used only when `f32(q / 100) == v` bit for bit, for every axis
(and per number for a scale). Anything else, such as NaN, −0, infinities, subnormals or
an unrounded drag, goes raw. Today's room rows are already rounded to hundredths, so they
always fit the grid.

**Canonical,** as `place` is. Writing the same instances always gives the same bytes:
- the encoder chooses the smallest of the encodings that decode to the same value;
- a self-reference is used when an earlier instance in the chunk has exactly the same
  rotation or scale, choosing the nearest one, and only when it is smaller;
- `uniform` is used when all three scale axes are equal.

**Self-references stay in their chunk.** Decoding needs only that value, and an edit
rewrites only that value. When the referenced instance changes or goes, its referrers are
written out in full in the same write, so the result is the same. Rendered texts never
change for the same reason.

### Migration (on open, one write transaction, lossless)

`graph meta` `format` goes up by one. A save of the old format is migrated when it opens,
and an old backup imports through the same path:
- **`graph blocks`** go to the room (container 0). The key `text:<label>` becomes ref `0`,
  `picture:<hash>` ref `1`, and `plain:#rrggbb` ref `2`. A key's values already sort by
  block id, so their order gives the numbers 1, 2, .... The position is x y z. The scale
  is w h d (none when all three are 1, uniform when they are equal). Each block is
  flagged with shape `cube`. Both old tables are then dropped.
- **`PLACE` edges** become instances in their map (the edge's `from`). The placement's
  kind becomes the shape flag, its flags carry over, colour goes to C (left out when it
  is today's default), and the asset goes to X when it isn't 0. The edges are then
  unlinked. Identical placements were already one edge, so they are one instance.
- **`space.maps`**: each named scene becomes a scene entry, and each row becomes an
  instance. A row with a key is an instance of that entry, with its `size` as a uniform
  scale. A row without one becomes a free text. Each entry's rows are numbered in their
  row order, so the first stays the first placement. The order of rows *across*
  different texts carries no meaning today (every loaded row is pinned at its position),
  so it isn't kept. To be strict anyway, the old JSON stays in `graph settings` as
  `space.maps (before stage B)` until the user removes it, and backups carry it.
- **Tests:**
  - old saves round-trip: the migrated instances give back every old block, edge and row;
  - edge values: NaN, ±0, ±∞, f32::MAX, subnormals, scales at the f16 limits, and a
    rotation at the smallest-three cycle;
  - canonical bytes: `pack(unpack(v)) == v`;
  - the size measurements below, in the commit message.

### Large texts (item 6)

**Storing.** Entries live 32 to a row (`record.rs`, `BLOCK_BITS`). With a 10 MB text in a
row, reading any of its 31 neighbours reads 10 MB, and editing one rewrites 10 MB. So a
record whose packed body is over 64 KiB moves out of its row:
- **The slot keeps a stub:** a flag bit *long*, the total UTF-8 length, the piece count,
  the whole text's sums and letter counts, and its content hash.
- **The body goes to `graph long`** (key `(id, piece)` → value). Each piece is up to about
  64 KiB of packed body, cut at a newline (or else a space) so it decodes on its own (each
  piece is a run, so caps lock and the last lead reset).
- **A piece's value** starts with its index: the UTF-8 offset, the character offset and
  line number where it starts, and its own ordinal and reversed sums. Sums are additive,
  so the whole text's figures are the sum of the pieces' figures.
- **An edit** repacks only the pieces it touches (a piece that grows past twice the size
  splits) and updates the stub's totals by the difference. History stores the old
  versions of the changed pieces, as deltas, as now. The words index is updated by the
  difference between the old and new pieces' terms.

**Reading in windows.** A `text.window` call (key, first line, line count) reads only the
pieces that cover the window, and returns those lines with that window's figures: the
gematria underline groups and the word types. Callers cache the results per piece and
version.
- **Memory view:** a virtualised list of lines over that call.
- **Space:** shows a long text as a slug (first line, length, sum) that opens into a
  reader. Zooming in reveals paragraphs, window by window.
- **Notes editor:** sends Rust only the edited window, not the whole text per key.
  Stage A left this open: about 100 ms a key at 526 KB on the web.
- **Benchmarks:** a 1 MB and a 10 MB text (open, scroll, search, spread), with the
  timings recorded in parity.md.

**Per-text size caps today** (searched 19 Oct; each to be removed or justified when the
item is built):

| where | what | plan |
|---|---|---|
| core/src/graph/interp.rs:115 | `Limits::output` 4 MiB: rendering a text that has refs or calls fails past 4 MiB. Plain texts skip the interpreter (core/src/graph/store.rs:366), so they have no cap | keep the cap for `CALL` output, the runaway guard; let `REF` transclusion stream pieces instead of counting against it |
| core/src/graph/store.rs:1171 | borrowing (prefix/suffix `REF`s) isn't tried for texts under 12 bytes or over 1 MiB | justified, not a cap: the text is stored whole. Keep, and note it in parity.md |
| core/src/ops/mod.rs:752 | `grammar.check`: at most 200 000 bytes a call | justified per call; the editor checks the visible window, so no text is too long to check |
| core/src/classify/io.rs:31 | `MAX_LEN` 8192: a classifier reads at most its `max_len` tokens of a text | a model limit, not a storage one; say in the classifier view when a text is longer than what it read |
| core/src/graph/record.rs:412 | 32 entries to a row | not a cap, a cost: fixed by moving long records out of the row (above) |
| hub/src/appweb.rs:211, :252 | the web app's calls and WebSocket messages: 64 MiB each (outside core) | a text over that is sent in windows; listed here for completeness |
| app/src/views/notes.rs:99 | `BIG` 100 000: above it the notes pad types plain text, without highlighting (the app, not core) | goes once highlighting is per window |

There are count caps too (`ops/mod.rs:212` and `:302`: at most 2000 texts per call), but
they aren't per-text sizes.

### Behaviour

- **Taking an instance out** (Delete, the menu) removes it from its chunk. Referrers are
  rewritten in full, and the definition is untouched. That is F4's rule, now in the save.
- **Forgetting a definition:**
  - **An entry or a picture:** its key range is removed with it. The confirmation names
    how many instances go and in how many scenes, for example "12 placements in 3
    scenes".
  - **A container** that is placed somewhere asks first: take the instances out, or
    *explode* them (each instance is replaced by the container's parts, transformed).
- **Undo:** the app's undo keeps the chunk values from before an operation, and Ctrl+Z
  writes them back byte for byte. A forgotten entry comes back together with its
  instances.
- **Merge** (`ops::merge`):
  - Definitions merge as entries do: the same text is the same entry, and ids are
    remapped. Pictures merge by hash, and colours and free texts by value.
  - Containers merge by name; a clash comes in as `<name> 2`, as spec models do.
  - The incoming instances of a (definition, container) pair are numbered after the
    existing ones, and an instance with exactly the same value as an existing one is kept
    once.
- **Bundles and the hub's share and take:** a bundle carries its entries' instances only
  in containers that are also in the bundle. A container in a bundle brings its
  definitions along, recursively through prefabs and chains.
- **Backups:**
  - `other.json` gets `instances`:
    `[[container label, "text:3,14" | "picture:<hash>" | "plain:#rrggbb" | "free:<id>", n, {pos: [x, y, z], rot?: [x, y, z, w], scale?: s | [x, y, z], colour?: "#rrggbbaa", flags?, extra?: base64}]]`.
    Floats are printed shortest round-trip, so they stay exact.
  - `other.json` also gets `free_texts`: `[[id, text]]`.
  - Packing is canonical, so an import gives back the same bytes.
- **Checkpoints:** the whole save, so nothing changes.

### Sizes: 10k copies of one text in one scene

Payload bytes, measured with a script over this layout. redb adds a few bytes of overhead
per stored value: today that is per instance, and after Stage B it is per chunk of 64.

| stored as | 38-character text | 1000-character text |
|---|---|---|
| today, a saved scene (`space.maps` JSON) | 0.74 MB (74 B a copy), rewritten whole on every save | 10.4 MB (1036 B a copy) |
| today, blocks showing it (`graph blocks` + `graph block ids`) | 490 KB (32 + 8 + ~9 B) | the same |
| today, `PLACE` edges | ~500 KB (42 B out-edge value + 8 B in-edge, plus the keys) | the same |
| after, scattered over ±50 units | 68 KB (6.8 B a copy) | 68 KB |
| after, in a row (3.4 apart, 100 to a line) | 49 KB (4.9 B a copy) | 49 KB |

After Stage B the text is stored once whatever its length, and an instance costs the same
for any definition. A non-default uniform scale adds about 2 bytes, or under 1 byte when
the instance refers back to the same scale earlier in its chunk.

### Open questions for the user

1. **Free texts** (room texts that aren't in the memory): should they stay outside the
   memory, as their own kind of definition as above, or become ordinary memory entries?
2. **The room:** should it keep its texts and copies between sessions, as instances in
   container 0 alongside the blocks? Today only named scenes keep texts.
3. **Prefab edits:** is "every copy updates; detach a copy to keep it as it was" the right
   default?
4. **Chains:** a chain as a container (its own entry, its members in order), with the same
   text allowed in several chains. Or as links straight between entries (item 8 says
   edges)? The container keeps a chain's layout and lets a chain be placed like a prefab.
5. **Merges:** should identical instances (same definition, scene and transform) collapse
   into one, as above?
6. **Positions:** is a 1/100-unit grid right for the compact positions? Anything else is
   stored raw, so nothing is lost either way.
7. **Long texts:** is the out-of-row threshold of 64 KiB, cut into pieces at newlines, a
   good split? Should the memory view show a long text's slug, or its first window?
8. **Forgetting a placed entry:** keep today's F4 behaviour (confirm, then remove every
   placement, undoable), or refuse while it has placements elsewhere?

### The user's answers to the open questions (19 Oct)

These settle the Stage B design. Where they differ from the draft above, the answers win.

1. **Texts in scenes are entries from memory.** A placed text is a reference to its entry,
   keyed like block textures: the key is the entry, and the multimap values are the
   instantiations. Texts are billboards, so an instance is only a position: no rotation and
   no scale.
2. **Scenes are saves.** A scene table with hierarchical features, sharing the text
   entries it places.
   - Each multimap value is a prefix and a list: the text, then the positions where it is
     instantiated.
   - Blocks and their metadata are stored with the scene in the same way.
3. **Prototype inheritance, like JavaScript.** When a prefab changes, its instances change
   too.
   - The exception is a discrete copy, which severs the link to the prototype.
   - The choice is made up front: **detach** (no inheritance from then on) or **modify**
     (keep inheriting, with local overrides).
4. **A chain is a note,** as weum meant it: a text key as the title and a text key as the
   body, two entries in a trenchcoat.
   - In the space, a chain is one transform for the head text, and the rest is positioned
     relative to it deterministically.
   - The rest never collides with blocks or other texts, using the same physics as the
     spread.
   - The texts float together and stay readable left to right. A rotation constraint flips
     them to face the viewer depending on the viewing angle.
5. **Merging works on wholes,** CRDT-like with no finer granularity (weights can't be
   blended), using the forge's set operations: each thing is included or excluded.
   - Where two scenes overlap in space, placements settle by deterministic physics and
     collision, like keep semantics but done up front.
6. **Sizes and positions.**
   - Texts at about hand size, rendered smallish, is liked; keep it.
   - Blocks default to about melon size or a little smaller.
   - Positions are free transforms, not a grid. Blocks scale freely. Any storage
     quantisation must be lossless or invisible, and must never snap anything.
7. **Long texts are split visually at word boundaries.** At most 80 characters a line and six
   lines a piece, spliced into whole pieces. No glass ceilings: these limits, like the hub's
   limits, are configurable settings, not constants.
8. **Texts inherit too.** When an entry's text changes, its placements show the new text,
   unless they are detached. No flag in the metadata: keep two tables, inheriting and
   detached.
