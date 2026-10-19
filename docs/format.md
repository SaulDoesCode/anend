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
