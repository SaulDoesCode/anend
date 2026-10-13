# The app's API

The hub runs the app for the web: each member's saves live on the server in a
workspace of their own. With `anend-hub --local` there's one workspace and no login,
which makes the hub the app's daemon on your own machine.

## Calls

Over a WebSocket at `/app/ws`, send `{"id": 1, "op": "notes.list", "args": {...}}`; the
answer comes back as `{"id": 1, "ok": true, "data": ...}` or `{"id": 1, "ok": false,
"error": "..."}`. Answers can arrive out of order, so match them by `id`. The same calls
also work as `POST /app/api/{op}` with the args as the JSON body.

The login cookie goes along with both (unless `--local`). The account must be active.

A **hit** is `{"key": "0,12", "text": "...", "note": false, "ordinal": 79, "reversed": 83}`.
A key is a memory's bytes joined by commas, or `note:<key>` for a note.

| op | args | data |
|---|---|---|
| `hello` | | `{save, saves: [name], counts: {notes, memories}}` |
| `saves.list` | | `[{name, open, notes, memories, size, modified}]` |
| `saves.open` / `saves.new` | `{name}` | `{save}` |
| `saves.copy` | `{name, to?}` | `{save}` |
| `saves.rename` | `{name, to}` | `{save}` |
| `saves.delete` | `{name}` (not the open one) | `null` |
| `saves.export` | `{name?}` (the open save if left out) | the save as a [bundle](#bundles) |
| `saves.import` | `{bundle, name?, merge?}`: a new save keeping the bundle's keys (named after it, or `name`, with a number when taken), or with `merge: true` into the open save, where what's new gets fresh keys | `{save, memories, notes, lists}` or `{merged: true, memories, notes, lists}` |
| `notes.list` | `{q?}` (words the notes must have) | `[{key, title, tags, created, modified, edits, preview, words}]`, newest first |
| `notes.get` | `{key}` | the note: `{_key, title, markdown, tags, created, edits}` |
| `notes.save` | `{note, text?}`: `text` is the editor's `# title` + markdown | the stored note (new notes get a key) |
| `notes.delete` | `{key}` | `null` |
| `mem.measure` | `{text}` | `{ordinal, reversed, atbash, frequency, gsum, pf_ordinal, pf_reversed, triangle, s, r}`; `triangle` is ⌊√(s·r)⌋ |
| `mem.gsearch` | `{bucket: ordinal\|reversed\|atbash\|frequency, key: number or text}` | `{bucket, hits}` |
| `mem.search` | `{q, variations?: true}` | `[hit]` |
| `mem.saturate` | `{text}` | `{key, new}` |
| `mem.forget` | `{key}` | `{text}` |
| `mem.remember` | `{key}` | hit |
| `mem.all` | `{limit?: 5000}` | `[hit]` |
| `lists.stats` | | `{lists: [{list, name, members: [hit], count, ordinal_sum, reversed_sum, ordinal_mean, reversed_mean, whole: measure, vector: [26]}], dot: [[cosine]]}` |
| `lists.enlist` / `lists.delist` | `{list, member}` (keys) | `null` |
| `lists.unlist` | `{list}` | `{removed}` |
| `space.spread` | `{texts: [{id, text}], reversed?}` | `{d, buckets, nodes: [{id, x, y, z, s, r, q, hue}]}` |
| `forge.preview` | `{ingredients: [{save, effect: add\|take away\|keep shared}], filter?}` | `{memories, notes, lists, sample, titles}` |
| `forge.make` | the same and `{name}` | `{save, memories, notes, lists}` |
| `forge.pour` | the same | what went into the open save |
| `stats` | | `{notes, words, saves, memories, tags: [[tag, n]], first, latest}` |

The forge's `filter` takes `{words, ordinal_from, ordinal_to, reversed_from, reversed_to, memories, notes, lists}`.

A spread node goes into the world in front of a camera at the origin, looking down
-z, as `(x·k, y·k, -(d + z))` with `k = (d + z) / d`.

## Saves in and out

| | |
|---|---|
| `GET /app/saves/{name}` | a workspace save, to keep |
| `POST /app/saves` | multipart `file` (and `name`): a save from your computer into the workspace |
| `POST /app/hub/{id}` | an approved hub save into the workspace; `?merge=1` merges it into the open save instead |
| `POST /app/share` | JSON `{name?, description, tags}`: the open save onto the hub |

## Bundles

A save as plain JSON, keys and all (`core/src/bundle.rs`). The web app keeps one in the
browser's IndexedDB when it runs without a hub, and moves saves between the browser and a
workspace with `saves.export` and `saves.import`; a bundle is also a file you can keep.

```json
{ "anend": 1, "name": "notes",
  "notes": [{ "_key": "1000000", "title": "a note", "markdown": "body", "tags": [], "created": "2026-10-13T09:00:00Z", "edits": [] }],
  "memories": [["listen", [0]], ["silent", [1]]],
  "lists": [["0", "1"], ["0", "note:1000000"]],
  "settings": { "space map: home": "7b2278223a317d" } }
```

`memories` are weum's `[text, key bytes]` (1 to 4 bytes); `lists` are `[list, member]`
labels (a memory's key bytes comma separated, or `note:<key>`); `settings` values are hex.
