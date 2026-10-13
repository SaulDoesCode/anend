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

## Classifiers

A save's classifiers (`core/src/classifiers.rs`; the engine is `core/src/classify/`):
each is trained on lists, one class per list, the members' texts as examples (a note
reads as its title and markdown). `name` picks the classifier in every call. A `class` is
a class name or its index. A text to judge is given as `key` (a memory or note) or
`text`. Without a hub, the web app answers these with "classifiers need a hub for now".

| op | args | data |
|---|---|---|
| `classify.list` | | `[summary]`: `{name, classes, lists, not, head, preset, inputs, trained, version, accuracy, macro_f1, examples, outputs, modified, training}` |
| `classify.create` | `{name, classes: [list keys], not?: list key, preset?: tiny\|compact\|small\|base (compact), head?: single\|multi, inputs?: [name or {from, coupling}]}` | details (below) |
| `classify.get` | `{name}` | details: the summary and `{steer, history, train, created, feedback_count, feedback: [latest 50], size: {weights, d_model, layers, blocks, max_len, steps, growth}, job}` |
| `classify.delete` | `{name}` (not while it trains, nor while another builds on it) | `null` |
| `classify.couple` | `{name, from, coupling}` (0 to 10; 0 keeps end-to-end feedback out of `from`) | summary |
| `classify.train` | `{name, epochs?, lr?, batch?, max_steps?, threads?, rounds?, fresh?, wait?}` | the job: `{name, state: running\|done\|failed\|cancelled, round, rounds, epoch, step, steps, loss, lr, tokens_per_sec, losses: [[step, loss]], evals: [{step, round, loss, accuracy, macro_f1}], seconds, examples, error?, summary?}` |
| `classify.job` | `{name}` | the job, or `{name, state: "idle"}` |
| `classify.cancel` | `{name}` | the job (it ends `cancelled`, writing nothing) |
| `classify.predict` | `{name, texts?: [text], keys?: [key]}` | `{classes, trained, results: [{key, text, probs, top, class, members}]}` |
| `classify.search` | `{name, class, q?, limit?: 50}` | `{classes, class, stack: [names], trained, scanned, results: [{key, text, note, prob, top, top_class, member}]}`, most likely first |
| `classify.contributions` | `{name, key or text, class?}` (the top class if left out) | `{class, text, results: [{name, share, input_share, grad_norm, classes, probs, top, top_class, judged}]}` |
| `classify.steer` | `{name, steer: {bias?: [..] or {class: value}, temperature?, threshold?, gains?: {letter, word, side, extra}, reset?}, save?: true}` | `{steer, classes, saved, bias: {texts, mean_kl, max_kl, flipped, shift, prior, prior_skew, imbalance}}` (`bias` null until trained) |
| `classify.feedback` | `{name, key or text, class, yes, weight?: 1}` | `{entry: {classifier, key, text, class, yes, weight, time, stack, before, after}}` |
| `classify.feedback_stack` | the same | `{entry, report: {before, after, grad_norms, losses}, touched: [names]}` |
| `classify.enlist` | `{name, class, threshold?}` (the steered threshold if left out) | `{list, name, members}` |
| `classify.represent` | `{name, key or text}` | `{vector}` |
| `classify.similar` | `{name, key or text, limit?: 20}` | `{text, results: [{key, text, note, cosine}]}` |

- **Training** runs on a thread of its own: `classify.train` reads the lists then and
  answers at once with the running job (`wait: true` answers when it's done). The
  weights are written when it finishes, so the save stays usable meanwhile; feedback
  on a classifier that's training is refused, and a run whose classifier changed
  meanwhile (feedback, another run) isn't written. It continues from the trained
  weights unless `fresh`. `rounds` above 1 trains that many rounds of `epochs`,
  deciding after each to widen, add attention heads, deepen or prune (growth events go
  in the history). The presets: `tiny` (15 k weights, for tests), `compact` (89 k,
  the default: seconds for a few lists of short memories), `small` (0.9 M), `base`
  (5 M).
- **Steering** is the player's: a bias per class (added to the logits after the
  temperature), the temperature, the threshold for membership, and gains on the input
  channels. Training never changes it. `save: false` measures without keeping it. The
  bias report compares steered with unsteered predictions over the classifier's own
  training texts (up to 300).
- **Feedback** on one classifier (`classify.feedback`) changes that classifier's weights
  only: a few small steps on the text, replaying its earlier feedback and some of its
  training texts. `classify.feedback_stack` goes end to end through the classifiers it
  builds on (`inputs`), each link scaled by its coupling, and writes every classifier it
  reached. `classify.contributions` names the classifiers that had a part in a result,
  so each can be told yes or no on its own.
- **Outputs as lists:** `classify.enlist` writes the texts at or above the threshold for a
  class as a list headed by the memory `"<classifier>: <class>"` (replacing what that list
  held), which other classifiers can be trained on.
- Search, enlist and similar look through up to 20 000 memories and notes.

In a [bundle](#bundles), `classifiers` carries each classifier as `{definition, weights?,
feedback?}`, the weights base64 (the `ANENDCLS` file). It is left out when there are none,
so older bundles read as before. Merging a bundle into a save leaves its classifiers out
(their lists get new keys there), and so does the forge.

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

The app decides once, as it loads: it posts `api/hello` beside its page, and a JSON
envelope back means a hub served it, so every call goes to the hub and IndexedDB is never
opened. Anything else (a static server's 404 or 501, no server at all, or `?offline` in
the address) and it runs offline: `hub/web/local` answers the same calls in the page,
the way app.rs does, and keeps each save in IndexedDB as a bundle and the key allocator's
state, written after every call that changes something. A hub that drops out later is
still the hub; the app reconnects. `node hub/web/test/parity.mjs` holds the two to the
same answers.

```json
{ "anend": 1, "name": "notes",
  "notes": [{ "_key": "1000000", "title": "a note", "markdown": "body", "tags": [], "created": "2026-10-13T09:00:00Z", "edits": [] }],
  "memories": [["listen", [0]], ["silent", [1]]],
  "lists": [["0", "1"], ["0", "note:1000000"]],
  "settings": { "space map: home": "7b2278223a317d" } }
```

`memories` are weum's `[text, key bytes]` (1 to 4 bytes); `lists` are `[list, member]`
labels (a memory's key bytes comma separated, or `note:<key>`); `settings` values are hex.
