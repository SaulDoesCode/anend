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
`graph picture lists` (list name → picture ids, a multimap). `graph textures`
(`core/src/graph/textures.rs`): texture recipes, recipe id (FNV-1a 128 of the normalized
recipe's JSON, 32 hex digits) → `{id, name, recipe, added}` JSON; a block placement whose
definition is `Def::Texture` (tag 4, the 16-byte id) shows one. A block placement's extra
holds records one after another: `B` 1 and nine f32s (its bend), then `F` 1, a byte with
a bit per face that has a recipe, and those faces' 16-byte recipe ids (right, left, top,
bottom, front, back).

## Backup

Two files. `texts.json`: `[[text, [key bytes], created, edited, mime?], ...]` (the first
two elements are weum's `all-of-it.json` rows, which import the same way). `other.json`:
`format`, `version`, `allocated`, `free`, `mimes`, `sources` (structured bodies as JSON
runs), `edges`, `history` (past versions as text), `settings`, `specs` (spec model
records), `pictures` (`[{id, mime, width, height, name, added, size, data: base64}]`)
and `picture_lists` (`[[list, id]]`).

## One model for a definition and its placed instances (design, partly built)

The design for the instance lines of docs/checklist.md (instances, inheritance, font sizes,
long texts, chains, merging). It follows the user's answers 1–9 at the end of this section;
where anything here disagrees with them, the answers win and this text is wrong. The first
draft (`b5b6b22`) asked eight questions; the answers settled them, and the parts of the
draft they overturned (a position grid, chains and scenes as containers, free texts, a
fixed piece size) are gone from here. Choices the answers leave open are defaults listed in
docs/wip/questions.md.

**Built so far (20 Oct):** the tables and the packed placements (`core/src/graph/scenes.rs`);
the migration of `graph blocks`, `PLACE` edges and `space.maps`; the room's blocks and
the saved scenes' texts on scenes (`blocks.*`, `scenes.*`); backups and whole-scene
merges; inheritance for texts (25 Oct: inheriting, own text, detached); chains (25 Oct:
mime 8 `application/x-anend-chain`, `graph::chains`). Not yet: prefabs, chains, font
sizes in the space beyond a saved text's size, physics settling overlaps on merge, long
texts. The rest of this section is the design; "today" below means before scenes.

Before scenes, the same idea was stored four ways: text copies in the room are `entry~2` node keys
that live only in the page until a scene is saved; saved scenes are one JSON setting
(`space.maps`: name → rows `[text, x, y, z, {key, size}]`, the text repeated in every row);
blocks are 32-byte values in `graph blocks` plus a `graph block ids` row each; and core's
`PLACE` edges carry a 34-byte placement. This design puts all of them into scenes.

### The model

- **Texts in scenes are memory entries** (answer 1). A placed text refers to its entry;
  there is no separate kind of room-only text. A text placement is a billboard: a position
  and a font size, with no rotation and no scale (answers 1, 6).
- **A save holds many scenes** (answer 2), in a scene table. Scenes share the save's
  entries. A scene has a name and an optional parent scene, so scenes nest into a
  hierarchy (folders, a scene inside a larger one). The room the space opens on is the
  save's first scene, and it keeps its texts and copies between sessions like any other.
- **What a scene holds** is placements of definitions:
  - a **text** (an entry), shown as a billboard;
  - a **block**, textured by an entry's text, a picture (by content hash) or a plain
    colour, with its own rotation, scale, colour, shape and metadata;
  - a **prefab**: a scene placed inside another scene, its parts' transforms relative to
    its origin;
  - a **chain** (an entry of its own, below), placed by its head.
- **An instance's address** is `(scene, kind, definition, n)`, where `n` is its number
  among that definition's placements of that kind in that scene: 1 for the first, then
  the next free number. This is F4's `entry~2` made durable. The copies count is how many
  numbers are present.
- **Inheritance** (answers 3, 8), as prototypes work in JavaScript:
  - a text placement shows its entry's current text, and a prefab placement shows its
    prefab's current parts. Editing the entry or the prefab changes every placement that
    inherits from it;
  - when a copy is made, the user chooses up front: **modify** (it keeps inheriting, and
    local overrides shadow the prototype's values, as an object's own properties shadow
    its prototype's) or **detach** (a discrete copy that no longer inherits);
  - inheriting and detached placements are kept in **two tables**, with no flag in the
    placement's metadata saying which it is.
- **Chains are entries of their own** (answer 4), with a new built-in mime,
  `application/x-anend-chain` (8). A chain is a title entry plus an ordered list of member
  entries, such as a run of paragraphs:
  - the chain entry's body is a `REF` to its title entry, so it renders, searches, lists
    and opens as its title does;
  - its members are `MEMBER` edges with order keys, as a list's are;
  - the same text can be a member of several chains;
  - notes are not changed. Placing a note in a scene shows it as a chain of two, its title
    and its body, worked out when it is shown; no chain entry is made for it.
- **A chain in a scene has one position, for its head.** Its members lay out from there in
  order, deterministically, with the spread's physics: they don't collide with blocks or
  other texts, they float together, they read left to right, and a rotation constraint
  turns them to face the viewer. The layout is computed, not stored.
- **A prefab cannot contain itself**, directly or through other prefabs (the same rule as
  borrowing's cycles), and the expansion depth is limited by a setting. A placed prefab
  addresses its parts as `outer instance / inner address`, so its internal links (a sticky
  ray target inside the group, #19) are stored relative to the prefab and work in every
  copy without being remapped.

### Sizes and positions (answer 6)

- **Positions are free.** They are stored as the f32 values they are, never snapped or
  rounded. Any compact encoding added later must decode to the same bits.
- **Hand-sized objects stay small,** as now.
- **Blocks default to about melon size or a little smaller,** and scale freely. The default
  edge is a setting; today's default is 1 unit, and the new default is set by eye beside
  the texts when the instance line is built.
- **A text placement's font size is a `u8`:** 0 is the default, set in the settings; 1–255
  are explicit sizes. One step is 1/50 of a world unit per em, so today's default text size
  of 0.5 units is 25 and the range is 0.02 to 5.1 units (the step is a default, listed in
  docs/wip/questions.md).

### Tables

| table | key | value |
|---|---|---|
| `graph scenes` | scene id `u64` | the scene's record: name, parent scene id (0 for none), created and edited times, its settings (JSON) |
| `graph scene instances` (multimap) | scene id `u64` | the inheriting placements: one value per (kind, definition, chunk of up to 64 numbers), below |
| `graph scene detached` (multimap) | scene id `u64` | the detached placements, in the same layout |
| `graph placed` (multimap) | definition ref | the scene ids that place it (one row per pair, not per instance) |

Each value starts with a **prefix** saying what is placed, followed by the **list** of its
placements (answer 2):
- a kind byte: 0 a text billboard, 1 a block, 2 a prefab;
- the definition ref (below);
- the chunk number `c` as a sized id (numbers `64c .. 64c + 63`);
- the packed placements.

A redb multimap keeps a key's values sorted by their bytes, so a scene's values group by
kind, then by definition, then by chunk. Loading a scene reads that scene's key and
nothing else. `graph placed` answers "how many copies, and where" (the F4 count, a
forget-the-entry confirmation, which scenes show a text) without reading every scene.

**Definition ref** (byte order is the sort order):
- `0` then a sized id: an entry (a text, a chain, or a block's text texture);
- `1` then 16 bytes: a picture, by its 128-bit content hash;
- `2` then `r g b`: a plain colour;
- `3` then a sized id: a scene (placed as a prefab).

A text billboard takes ref 0; a prefab takes ref 3; a block takes 0, 1 or 2.

In `graph scene detached`, an entry ref (`0`, for a text or a block's text texture) is
followed by the entry's version: the `edited` time (a varint, in ms) of the version it was
detached at, which `graph history` keeps once the entry is edited again. A detached text therefore still refers to its entry, at the version it was copied
from. A detached prefab refers to a new scene made at detach time as a copy of the
prefab's parts, so later edits to the prefab don't reach it.

A **sized id** is one length byte (0–8) followed by that many big-endian bytes, with no
leading zero bytes. Comparing the bytes therefore compares the numbers, and entry `3,14`
(id 1038) takes 3 bytes: `02 04 0e`.

**The packed placements:** a bit stream, most significant bit first, as the text codec
writes it: a 3-bit padding count, then one record per present instance in number order.
Varints are the codec's 5-bit varints (four data bits a chunk, low group first).

Every record starts with:

| field | bits | meaning |
|---|---|---|
| gap | varint | `n − previous n − 1` (the first: `n − 64c`) |
| position | 96 | x, y, z as f32, 32 bits each, exactly as given |
| F | 1 | flags present: a varint (1 pinned, 2 hidden, 4 spread, 8–64 a block's shape as `place::Kind`, 128 the extra holds this copy's own values, the rest kept) |
| X | 1 | extra present: a varint length, then that many bytes (an asset id, a prefab copy's overrides, later a block's program; unknown extras are kept as they are) |

Then, by kind:

| kind | fields |
|---|---|
| text | font size: a varint (`0` the default, one 5-bit chunk) |
| block | R (2 bits): rotation 0 none (the identity), 1 own (32 bits, `place`'s smallest three), 2 same as an earlier instance (varint `d`: that instance is `n − d` in this chunk), 3 reserved; S (2 bits): scale 0 none (1 on every axis), 1 uniform (one f32), 2 per axis (three f32), 3 same as `n − d` (varint `d`); C (1 bit): colour present, 32 bits RGBA8 |
| prefab | R and S as for a block |

**Defaults cost little.** A default-sized text with no flags or extra is a gap (5 bits), a
position (96 bits), 2 bits and a font size of 0 (5 bits): 108 bits, 13.5 bytes. A block
with no rotation, unit scale and no colour costs the same: its R, S and C take the 5 bits
the font size takes in a text.

**A block's bend** (26 Oct) is kept in X: `B`, version 1, then nine little endian f32s
(the ripple's amount, frequency, phase, c1, c2; the fold's amount, line, c1, c2), left
out when the block is straight (`graph::blocks::Bend`). Its turn is R's packed rotation,
left out for no turn.

**An inheriting text with its own text** (25 Oct) keeps it in X as a JSON object,
`{"text": "…"}`, with flag 128 set; it shows that text instead of its entry's until the
override is dropped. A saved scene's rows carry it as `own`, and a detached text as
`detached` (the entry's edited time it keeps), `[text, x, y, z, {key, size?, own?,
detached?}]`.

**A prefab copy that modifies** keeps its overrides in X: a list of (inner address, field,
value) that replace the prototype's values for that copy only. A part the prefab later
removes takes its overrides with it.

**Canonical,** as `place` is. Writing the same placements always gives the same bytes:
- the encoder chooses the smallest of the encodings that decode to the same value;
- a self-reference is used when an earlier instance in the chunk has exactly the same
  rotation or scale, choosing the nearest one, and only when it is smaller;
- `uniform` is used when all three scale axes are equal.

**Self-references stay in their chunk.** Decoding needs only that value, and an edit
rewrites only that value. When the referenced instance changes or goes, its referrers are
written out in full in the same write, so the result is the same.

### Migration (on open, one write transaction, lossless)

`graph meta` `format` goes up by one. A save of the old format is migrated when it opens,
and an old backup imports through the same path:
- **The room:** the save gets its first scene, named "room".
- **`graph blocks`** become block placements in the room. The key `text:<label>` becomes
  ref `0`, `picture:<hash>` ref `1`, and `plain:#rrggbb` ref `2`. A key's values already
  sort by block id, so their order gives the numbers 1, 2, .... The position is x y z.
  The scale is w h d (none when all three are 1, uniform when they are equal). Each block
  is flagged with shape `cube`. Both old tables are then dropped.
- **`PLACE` edges:** each map entry (mime 5) with `PLACE` edges becomes a scene named by
  its text, and each edge a placement in it: a block when the placement's kind is a block
  shape, else a text. Its flags carry over, colour goes to C (left out when it is today's
  default), and the asset goes to X when it isn't 0. The edges are then unlinked; the map
  entry itself stays an entry. Identical placements were already one edge, so they are one
  instance.
- **`space.maps`:** each named scene becomes a scene, and each row a text placement. A row
  with a key places that entry. A row without one was a room-only text: it becomes a new
  memory entry with that text, saved as any text is (so an equal existing text is reused),
  and is placed. A row's `size` becomes the font size, rounded to the nearest step (0 when
  it equals the default). Each entry's rows are numbered in their row order, so the first
  stays the first placement. The order of rows *across* different texts carries no meaning
  today (every loaded row is pinned at its position), so it isn't kept. To be strict
  anyway, the old JSON stays in `graph settings` as `space.maps (before scenes)` until the
  user removes it, and backups carry it, so a rounded size can be recovered.
- **Tests:**
  - old saves round-trip: the migrated placements give back every old block, edge and row;
  - edge values: NaN, ±0, ±∞, f32::MAX, subnormals, scales at the f16 limits, and a
    rotation at the smallest-three cycle;
  - canonical bytes: `pack(unpack(v)) == v`;
  - the size measurements below, in the commit message.

### Long texts (#6, #12, answers 7, 9)

There is no limit on how long a text can be. Every limit below is a setting with a
default, as the hub's limits are.

**Built (25 Oct), simpler than the design below in two ways:** a record whose encoded
bytes are over the piece size (the save's setting `graph.piece_bytes`, 64 KiB) keeps a
stub in its row (`0xF5`, its length, its piece count) and its bytes go to `graph long`,
`(id, piece)` → up to a piece of bytes. The pieces are cut by bytes, not at newlines, and
the stub has no sums of its own; an edit writes the record's pieces again whole (history
still keeps deltas). Reading in windows (`text.window`) isn't built: the views get the
whole text and show it piece by piece (`anend_core::pieces`, the settings `piece_line`
and `piece_lines`). The notes view types in segments (only the changed range crosses to
Rust; the colour layer is patched per segment). The limits are the save's settings
`limits.*` (`graph::store::LIMITS`, the settings' "limits of this save").

**Storing in pieces, as a storage detail.** Entries live 32 to a row (`record.rs`,
`BLOCK_BITS`). With a 10 MB text in a row, reading any of its 31 neighbours would read
10 MB, and editing one would rewrite 10 MB. So a record whose packed body is over the
piece size (a setting, 64 KiB by default) moves out of its row:
- **The slot keeps a stub:** a flag bit *long*, the total UTF-8 length, the piece count,
  the whole text's sums and letter counts, and its content hash.
- **The body goes to `graph long`** (key `(id, piece)` → value), as many pieces as it
  needs. Each piece is up to about the piece size of packed body, cut at a newline (or
  else a space) so it decodes on its own (each piece is a run, so caps lock and the last
  lead reset). No single redb value then holds the whole text, so redb's per-value limit
  doesn't cap a text either.
- **A piece's value** starts with its index: the UTF-8 offset, the character offset and
  line number where it starts, and its own ordinal and reversed sums. Sums are additive,
  so the whole text's figures are the sum of the pieces' figures.
- **An edit** repacks only the pieces it touches (a piece that grows past twice the piece
  size splits) and updates the stub's totals by the difference. History stores the old
  versions of the changed pieces, as deltas, as now. The words index is updated by the
  difference between the old and new pieces' terms.

**Splitting for show, at word boundaries** (answer 7): at most 80 characters a line and
six lines a piece, both settings. These visual pieces are separate from the storage
pieces. They are spliced into whole pieces (a piece holds whole lines and lines hold whole
words; a word longer than a line is broken), and a view shows and highlights a long text
piece by piece.

**Reading in windows.** A `text.window` call (key, first visual piece, piece count) reads
only the storage pieces that cover the window, and returns those lines with that window's
figures: the gematria underline groups and the word types. Callers cache the results per
piece and version.
- **Memory view:** a virtualised list of visual pieces over that call; a long text shows
  its first piece and opens into the rest.
- **Space:** a long text shows as a slug (first line, length, sum) that opens into a
  reader. Zooming in reveals more of it, piece by piece.
- **Notes editor, typing in segments** (the user's approach): the text is edited as
  separate typing segments. Each segment is updated on its own and stitched back into the
  text, so a key sends Rust only the segment being typed in, and only that segment is
  highlighted again. Today a key costs about 100 ms at 526 KB on the web.
- **Benchmarks:** a 1 MB and a 10 MB text (open, scroll, search, spread), with the
  timings recorded in parity.md.

**Per-text limits today** (searched 19 Oct), and what each becomes (answer 9):

| where | what | becomes |
|---|---|---|
| core/src/graph/interp.rs:115 | `Limits::output` 4 MiB: rendering a text that has refs or calls fails past 4 MiB. Plain texts skip the interpreter (core/src/graph/store.rs:366), so they have no cap | a setting, by default no cap. A program that loops forever is stopped by its step budget (fuel), not by the size of a text |
| core/src/graph/store.rs:1171 | borrowing (prefix/suffix `REF`s) isn't tried for texts under 12 bytes or over 1 MiB | a setting, defaults as now. It caps nothing: a text outside the range is stored whole |
| core/src/ops/mod.rs:752 | `grammar.check`: at most 200 000 bytes a call | a setting; the editor checks the visible window, so no text is too long to check |
| core/src/classify/io.rs:31 | `MAX_LEN` 8192: a classifier reads at most its `max_len` tokens of a text | a setting for the ceiling; it is a model's input length, not a storage limit. The classifier view says when a text is longer than what was read |
| core/src/graph/record.rs:412 | 32 entries to a row | the row layout, not a limit on texts: long records leave the row (above) |
| hub/src/appweb.rs:211, :252 | the web app's calls and WebSocket messages: 64 MiB each (outside core) | a hub setting; a text over it is sent in windows |
| app/src/views/notes.rs:99 | `BIG` 100 000: above it the notes pad types plain text, without highlighting (the app, not core) | a setting until typing in segments makes it unneeded, then removed |

The count limits (`ops/mod.rs:212` and `:302`: at most 2000 texts per call) become
settings too.

### Behaviour

- **Taking a placement out** (Delete, the menu) removes it from its chunk. Referrers are
  rewritten in full, and the definition is untouched. That is F4's rule, now in the save.
- **Copying a placement** asks up front: modify (inherit, with overrides) or detach
  (answer 3). Double-click stays deliberate, as F4 left it.
- **Forgetting a definition:**
  - **An entry or a picture:** its placements in every scene, inheriting and detached, go
    with it. The confirmation names how many and in how many scenes, for example "12
    placements in 3 scenes" (F4's behaviour, kept).
  - **A scene placed as a prefab elsewhere** asks first: take those placements out, or
    *explode* them (each is replaced by the prefab's parts, transformed, as detached
    placements).
- **Undo:** the app's undo keeps the values from before an operation, and Ctrl+Z writes
  them back byte for byte. A forgotten entry comes back together with its placements.
- **Merge** (`ops::merge`, answer 5): merging works on wholes, with no finer granularity,
  using the forge's set operations; each thing is included or excluded.
  - Entries merge as they do now: the same text is the same entry, and ids are remapped.
    Pictures merge by hash, colours by value.
  - A scene comes in whole. A clash of names comes in as `<name> 2`, as spec models do.
  - Where two scenes are combined into one and their placements overlap in space, the
    placements settle by deterministic physics and collision when the merge is made (keep
    semantics, done up front), and the settled positions are what is written.
- **Bundles and the hub's share and take:** a bundle carries a scene's placements together
  with their definitions, recursively through prefabs and chains.
- **Backups:**
  - `other.json` gets `scenes`: `[[id, name, parent, created, edited, settings]]`;
  - and `instances`:
    `[[scene id, "inherit" | "detached", "text" | "block" | "prefab", "text:3,14" | "text:3,14@<edited ms>" | "picture:<hash>" | "plain:#rrggbb" | "scene:<id>", n, {pos: [x, y, z], font?: n, rot?: [x, y, z, w], scale?: s | [x, y, z], colour?: "#rrggbbaa", flags?, extra?: base64}]]`.
    Floats are printed shortest round-trip, so they stay exact.
  - Packing is canonical, so an import gives back the same bytes.
- **Checkpoints:** the whole save, so nothing changes.

### Sizes: 10k copies of one text in one scene

Today's figures were measured with a script over the stored layouts. The new figure is
the encoder's output (`core/src/graph/scenes.rs`, test `defaults_cost_what_the_design_says`:
136 094 bytes of values for 10k default-sized text placements, 108 bits each plus each
chunk's prefix), without redb's own overhead. redb adds a few bytes of overhead per stored
value: today that is per placement, and after this change it is per chunk of 64.

| stored as | 38-character text | 1000-character text |
|---|---|---|
| today, a saved scene (`space.maps` JSON) | 0.74 MB (74 B a copy), rewritten whole on every save | 10.4 MB (1036 B a copy) |
| today, blocks showing it (`graph blocks` + `graph block ids`) | 490 KB (32 + 8 + ~9 B) | the same |
| today, `PLACE` edges | ~500 KB (42 B out-edge value + 8 B in-edge, plus the keys) | the same |
| after, text placements at free positions | 136 KB (13.6 B a copy) | the same |

The text is stored once whatever its length, and a placement costs the same for any
definition. Most of a placement is its 12-byte position. The first draft's 1/100 grid
made that 5–7 bytes, but answer 6 rules out a grid; a lossless compact encoding (for
example, each axis's bits written as their difference from the previous placement's) can
be added later behind the canonical rule, and is measured before it is kept.

### The user's answers to the open questions (19 Oct)

These settle the Stage B design. Where they differ from the draft above, the answers win.

1. **Texts in scenes are entries from memory.** A placed text is a reference to its entry,
   keyed like block textures: the key is the entry, and the multimap values are the
   instantiations. Texts are billboards, so an instance is only a position: no rotation and
   no scale.
2. **Many scenes live inside one save,** like Logic World saves inside the larger save. A
   scene is not a save of its own.
   - Each save holds a scene table with hierarchical features, sharing the save's text
     entries across all its scenes.
   - In a scene, each multimap value is a prefix and a list: the text, then the positions
     where it is instantiated.
   - Blocks and their metadata are stored with the scene in the same way.
3. **Prototype inheritance, like JavaScript.** When a prefab changes, its instances change
   too.
   - The exception is a discrete copy, which severs the link to the prototype.
   - The choice is made up front: **detach** (no inheritance from then on) or **modify**
     (keep inheriting, with local overrides).
4. **Chains are their own kind of thing; notes stay exactly as they are.** (The user's first
   wording was "a chain is a note"; it was changed so notes, in the notes view and attached
   to entries, keep working unchanged.)
   - A chain is an entry: a title entry plus an ordered list of member entries, such as a run
     of paragraphs. It can be searched, listed and opened like any entry.
   - In a scene, a chain has one position, for its head. The members lay out from it in order,
     deterministically, without colliding with blocks or other texts (the same physics as the
     spread). They float together and stay readable left to right, with a rotation
     constraint that flips them to face the viewer.
   - Weum's "two entries in a trenchcoat" becomes one use of chains: placing a note in a scene
     shows it as a chain of two, its title and its body. The note itself isn't changed.
5. **Merging works on wholes,** CRDT-like with no finer granularity (weights can't be
   blended), using the forge's set operations: each thing is included or excluded.
   - Where two scenes overlap in space, placements settle by deterministic physics and
     collision, like keep semantics but done up front.
6. **Sizes and positions.**
   - Hand-sized objects render fairly small, and that's liked; keep it.
   - Blocks default to about melon size or a little smaller, and scale freely. Positions are
     free transforms, never snapped to a grid. Any storage quantisation must be lossless or
     invisible.
   - **A text placement carries a font size as a plain `u8`:**
     - `0` means the default size, set in settings;
     - `1`–`255` set it explicitly.

     No `Option` is needed: in the packed format a `0` costs one 5-bit chunk, so
     default-sized texts stay as cheap as now. Texts are still billboards, with no rotation,
     and the font size is their only scale.
7. **Long texts are split visually at word boundaries.** At most 80 characters a line and six
   lines a piece, spliced into whole pieces. No glass ceilings: these limits, like the hub's
   limits, are configurable settings, not constants.
8. **Texts inherit too.** When an entry's text changes, its placements show the new text,
   unless they are detached. No flag in the metadata: keep two tables, inheriting and
   detached.
9. **No limit on how long a text can be.** The user doesn't want texts foreclosed early.
   - Splitting a long text into pieces is only a storage detail. A text of any length is
     stored as however many pieces it needs, so not even redb's per-value limit (3 GiB per
     value, 3.75 GiB per pair in redb 4.3) caps a text.
   - The piece size is a setting, 64 KiB by default.
   - The 4 MiB cap on rendered output in `core/src/graph/interp.rs:115` becomes a setting
     too (default: no cap). A program that loops forever is stopped by its step budget, not
     by the size of a text.
   - The other limits in the list above become settings as well.
