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
Without one, a post's body isn't read and the socket isn't opened. A member's app runs at
most 4 calls at once (more wait their turn) over at most 8 sockets. On a shared hub (not
`--local`) a member's saves may fill the hub's `--quota` (1 GB by default) and number at
most 200; past that, calls that would add something answer "your saves fill …" while
reading and deleting still work. A `--local` hub answers only requests addressed to
`localhost`, an IP address or its `--url` (see [security](security.md)).

A save is an **entry graph** ([format](format.md)): every text, note, list name and tag
is an entry under a key, and lists, tags and notes-on are edges between entries. The
desktop app answers the same ops in process with the same code (`anend_core::ops`), so
both answer alike.

A **key** is an entry's weum label, its key bytes joined by commas (`0,12`); `note:0,12`
reads as `0,12`. A **hit** is `{"key": "0,12", "text": "...", "note": false, "ordinal":
79, "reversed": 83}`: a note (an entry of mime `text/markdown`, `# title` and markdown)
shows as its title. Sums are weum's ordinal and reversed; **atbash is gone** (it grouped
texts as reversed does), and asking for it answers an error.

| op | args | data |
|---|---|---|
| `hello` | | `{save, saves: [name], counts: {notes, memories}}` (memories: entries that are neither notes nor tags) |
| `saves.list` | | `[{name, open, notes, memories, size, modified}]` (counts for the open save only) |
| `saves.open` / `saves.new` | `{name}` | `{save}` |
| `saves.copy` | `{name, to?}` | `{save}` (through a backup and restore, classifiers copied along) |
| `saves.rename` | `{name, to}` (the open one too) | `{save}` |
| `saves.delete` | `{name}` (not the open one) | `null` |
| `saves.export` | `{name?}` (the open save if left out) | the save as a [bundle](#bundles) |
| `saves.import` | `{bundle, name?, merge?}`: a new save keeping the bundle's keys (named after it, or `name`, with a number when taken); with `merge: true` into the open save: a whole bundle's entries are merged (the same text is one entry; new ones get fresh keys, their lists, tags and notes-on come along), weum rows alone keep their keys | `{save, memories, notes, lists, ...}` or `{merged: true, memories, notes, lists}` (what was new) |
| `saves.backup` | `{name?}` | the desktop writes `texts.json` and `other.json` into `backups/` and answers `{paths}`; the hub answers `{paths: [], texts, other}` |
| `saves.import_texts` | `{data, other?, name?}`: weum's `all-of-it.json` or a backup's `texts.json` (`data`, text), with a backup's `other.json` (`other`) a whole save; into a new save `name`, or (texts alone) into the open save | `{save, memories, replaced, skipped, borrowing}` |
| `notes.list` | `{q?, tags?, mode?, since?, until?}`: `q` words the notes must have; `tags` all of them, or any with `mode: "or"`; created between `since` and `until` (RFC 3339) | `[{key, title, tags, created, modified, edits, preview, words}]`, newest first |
| `notes.tags` / `tags.all` | | `[{tag, count}]`: every tag and how many entries carry it, the most used first (then by name) |
| `notes.get` | `{key}` | `{_key, title, markdown, tags, created, edits, modified, source?, rendered?}`: `markdown` as it renders (replacements put in); `source` the note as written when it has replacements; `rendered` the markdown with saved filters' results put in, when it names any |
| `notes.save` | `{note, text?, on?}`: `text` is the editor's `# title` + markdown; `on` a key the note is about (a note edge) | the stored note (new notes get a key; an edit keeps the old text as a version). `{{key}}` in the text is a replacement (below) |
| `notes.alternatives` | `{word}` | `{word, ordinal, reversed, alternatives: [{text, key, list, ordinal, reversed, same_ordinal, same_reversed}]}`: the other words of the lists `word` is in (each list's head too), those keeping the ordinal sum first, then the reversed |
| `notes.delete` | `{key}` | `null` |
| `notes.on` | `{key}` | `[hit]`: the notes on an entry, oldest first |
| `tags.add` / `tags.remove` | `{key, tag}` | `null` |
| `tags.entries` | `{tags, mode?}` | `[hit]` carrying all (or any) of them |
| `mem.measure` | `{text}` | `{ordinal, reversed, frequency, gsum, pf_ordinal, pf_reversed, triangle, s, r}`; `triangle` is ⌊√(s·r)⌋ |
| `mem.gsearch` | `{bucket: ordinal\|reversed\|frequency, key: number or text}` | `{bucket, hits}` |
| `mem.search` | `{q, variations?: true}` | `[hit]` |
| `mem.random` | `{q?, limit?: 60, seed?}` | hits: any `limit` texts sharing a word with `q` (any text when it's empty), shuffled by `seed` (a fresh one when left out); weum's random hashmap search |
| `mem.saturate` | `{text}` | `{key, new}` (the entry with that text, made when there's none) |
| `mem.forget` | `{key}` (any entry) | `{text}` |
| `mem.remember` | `{key}` | hit |
| `mem.put` | `{key, text}` | `null`: edits the entry, or puts a forgotten one back with its edges |
| `mem.all` | `{limit?: 5000}` | `[hit]` |
| `mem.buckets` | `{bucket: ordinal\|reversed}` | `{buckets: [{sum, count}]}` |
| `entries.query` | `{words?, bucket?, from?, to?, since?, until?, tag?, kind?: notes\|memories, limit?: 300}` | `{total, hits}` |
| `entries.info` | `{key}` | `{key, mime, created, edited, kind, detected, fixed, kinds}`: when it was written and edited (RFC 3339), and its kind: `detected` by its pattern (`core/src/kinds.rs`: link, email, number, date, time, json, hashtag, word, name, phrase, sentence, text), or the one set by hand (`fixed`) |
| `entries.set_kind` | `{key, kind}` (one of `kinds`; empty goes back to the detected one) | `null`; kept in the save's settings as `kind <key>` |
| `lists.stats` | | `{lists: [{list, name, members: [hit], count, ordinal_sum, reversed_sum, ordinal_mean, reversed_mean, whole: measure, vector: [26]}], dot: [[cosine]]}` (`dot` is empty past 1000 lists) |
| `lists.enlist` / `lists.delist` | `{list, member}` (keys) | `null` |
| `lists.unlist` | `{list}` | `{removed}` |
| `space.spread` | `{texts: [{id, text}], reversed?}` | `{d, buckets, nodes: [{id, x, y, z, s, r, q, hue}]}` (the old spiral layout; the app's space no longer uses it) |
| `space.features` | `{keys: [key]}` (at most 2000) | `{features: [{key, s, r, chars, words, kind, types: {type: n}, created (ms), lists: [list key], frequency}]}`: what the space's free spread weighs texts by; unknown keys are left out |
| `forge.preview` | `{ingredients: [{save, effect: add\|take away\|keep shared}] (at most 32), filter?}` | `{memories, notes, lists, sample, titles}` |
| `forge.make` | the same and `{name}` | `{save, memories, notes, lists}` |
| `forge.pour` | the same | what went into the open save |
| `settings.get` / `settings.set` | `{name}` / `{name, value}` | `{value}` / `null` |
| `stats` | | `{notes, words, saves, memories, tags: [[tag, n]], first, latest, entries, by_mime, edges, versions, body_bytes, borrowing}` |

The forge's `filter` takes `{words, ordinal_from, ordinal_to, reversed_from,
reversed_to, memories, notes, lists}`. In the forge an entry is its mime and rendered
text: `add` joins saves (the same text once), `take away` drops what another save has,
`keep shared` keeps only what it has too; list pairs follow the same rules, tags come
with the entries carrying them.

A spread node goes into the world in front of a camera at the origin, looking down
-z, as `(x·k, y·k, -(d + z))` with `k = (d + z) / d`.

### Replacements

- **`{{key}}`** in a note (a weum key such as `{{0,12}}`, of an entry other than the note)
  is kept as a live reference (the graph's transclusion, `codec::Seg::Ref`): the note
  shows that entry's text as it is now, and that entry may hold replacements of its own
  (recursive; one that comes back to itself is an error, shown as the note failing to
  render). Anything else between double braces stays as written.
- **Sums**: a note is indexed by what it renders, so writing `{{key}}` where that entry's
  text stood leaves its sums as they were; when a referenced entry changes, the notes
  that refer to it directly are indexed again (their sums and words follow).
- **`{{filter:name}}`** names a saved filter of the filters view (the save's `filters`
  setting): a materialized view whose results (at most 50, as a markdown list) are put
  in when the note is read (`notes.get`'s `rendered`); the stored note keeps the name.
- **Synonym and register lists**: any list of the save is a set of words that may stand
  for each other; `notes.alternatives` answers a word's, and the editor offers them in a
  dropdown (the lists view manages the lists).

## Classifiers

A save's classifiers (`core/src/classifiers.rs`; the engine is `core/src/classify/`):
each is trained on lists, one class per list, the members' texts as examples (a note
reads as its title and markdown). They keep their own tables in a file beside the save (`<name>.models`), reading the graph's lists and texts. `name` picks the classifier in every call. A `class` is
a class name or its index. A text to judge is given as `key` (a memory or note) or
`text`. One implementation answers them on the desktop and the hub alike
(`core/src/ops/classify.rs`, through `ops::Saves::with_models`); the hub adds only its
quotas and job limits (two trainings or indexings at once per member, two hours each).

| op | args | data |
|---|---|---|
| `classify.list` | | `[summary]`: `{name, classes, lists, not, head, preset, gematria, inputs, trained, version, accuracy, macro_f1, examples, outputs, modified, training}` |
| `classify.create` | `{name, classes: [list keys], not?: list key, preset?: tiny\|compact\|small\|base (compact), head?: single\|multi, inputs?: [name or {from, coupling}], gematria?: false}` (`gematria: true` adds the letter-value, word-sum and side-vector input channels; off by default, and classifiers made before it was optional keep theirs on) | details (below) |
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
| `classify.unfeedback` | `{name}` | `{entry, left}`: takes back the latest yes/no on `name` (one or through its stack): the classifiers it wrote get their weights back exactly, and the entry leaves the feedback log; the last 20 can be taken back, until the next training |
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
  in the history). On a shared hub a member runs at most 2 trainings and indexings at
  once, the hub at most as many as it has cores (4 at least), and a training stops (as
  `failed`, writing nothing) after 2 hours. `threads` is at most the hub's cores, `batch`
  at most 4096, `lr` above 0 and at most 1. The presets: `tiny` (15 k weights, for tests), `compact` (89 k,
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

### Classifier buckets, filters and modules

Outputs are memoized per classifier and memory/note (`core/src/memo.rs`), under a stamp
of the weights' versions and steering, so a retrain, feedback or steering change reads
them again; an edited text is read again too. Calls fill what they need lazily;
`classify.index` fills ahead. A bucket is a class (or `uncertain`: in no class) and a
probability band (0 for 0–10 % … 9 for 90–100 %).

| op | args | data |
|---|---|---|
| `classify.index` | `{name, keys?, wait?}` (every memory and note if no keys) | the job `{name, state: running\|done\|failed\|cancelled, done, total, report?: {total, computed, cached, stopped, seconds}, error?}` |
| `classify.index_job` / `classify.index_cancel` | `{name}` | the job (or `{state: "idle"}`) |
| `classify.buckets` | `{name}` | `{name, classes, indexed, palette: [hex], buckets: [{class, index, bands: [10 counts], count}]}` (the last is `uncertain`) |
| `classify.bucket` | `{name, class (name, index or "uncertain") or key or text, band?, min?, limit?: 500}` | `{class, count, stale, indexed, hits: [hit + prob]}`, surest first |
| `classify.near` | `{name, key or text, by?: embedding\|outputs, limit?: 20}` | `{text, indexed, row, results: [{key, text, note, cosine}]}` among the indexed texts |
| `classify.tints` | `{name, keys}` (≤ 5000) | `{classes, palette, tints: [{key, class (null: uncertain), strength}]}` |
| `classify.indexed` | `{key}` | `{key, chosen, verdicts: [{name, classes, probs, top, class, members, confidence, valid, checked, validators: [[by, p]], palette}], classifiers: [{name, trained, chosen}]}` |
| `classify.set_index` | `{key, names}` | `{chosen}` |
| `classify.validators` | `{name, set?: [{by, class, min?: 0.5, mode?: gate\|weigh}]}` | `{validators}` |
| `classify.disagreements` | `{name, limit?: 40}` | `{results: [{key, text, class, prob, checks: [[by, p]]}]}`: texts it counts in a class that a validator doesn't pass |
| `classify.export` | `{name, upstream?: true, description?, author?}` | `{file, size, data}`: the `.anendcls` module in base64 |
| `classify.inspect` | `{data}` | the module's summary `{name, description, author, classes, head, preset, weights, accuracy, macro_f1, examples, measured_on, upstream: [[name, bundled]]}` |
| `classify.import` | `{data, as?, lists?: {class: list key}}` | `{name, created, reused, lists: [[class, key, new]]}` |
| `filters.list` | | `[{name, expr, description, created, modified, list?, count?, reads}]` |
| `filters.save` | `{name, expr, description?}` | the preset |
| `filters.delete` | `{name}` (not while another uses it) | `null` |
| `filters.run` | `{name or expr, limit?: 200}` | `{scanned, matched, results: [{key, text, note, score}], classifiers, untrained}` |
| `filters.enlist` | `{name}` | `{list, name: "filter: <name>", members}` |

A filter expression is JSON with `op`: `and`/`or` `{of: [..]}`, `not` `{of}`, `class`
`{classifier, class, min?, max?, top?}` (min defaults to the steered threshold; the
classifier's validators apply), `words` `{q}`, `sum` `{bucket: ordinal|reversed,
from?, to?}`, `list` `{list}`, `tags` `{tags, any?}`, `kind` `{kind: memories|notes}`,
`preset` `{name}`. A validator in `gate` mode zeroes a classifier's probabilities on
texts it doesn't pass; in `weigh` mode multiplies them by its own.

## Spec models

Models made from a declarative spec ([models](models.md)): JSON saying what a model
reads, how big its trunk is, its heads (classify, multi, regress, generate), the data it
learns from (queries over the save's lists and tags, or inline), and, for a generator,
the critics that judge what it writes. Each one's definition (its spec's JSON, created,
modified, its trainings) lives in the save itself (table `graph spec models`), so it goes
along wherever the save file goes: copies, renames, backups, bundles, the hub's share and
take, and merges. The trained weights (megabytes; hundreds of them for a pretrained
model) stay in the save's models file (`<name>.models`, beside its classifiers), which is
copied, renamed, deleted and bundled with it; weights that no longer fit their
definition's shape are unused, and the model answers as untrained. A save from before
this keeps its definitions in the models file; opening it moves them into the save
(nothing is lost). One implementation answers them on the desktop and the hub
(`core/src/ops/models.rs`). `name` picks the model; a spec's `name` is its model's name
(no classifier of the save may have it, since critics find models by name).

| op | args | data |
|---|---|---|
| `models.list` | | `[summary]`: `{name, trained, version, created, modified, input: text\|bytes\|image\|numbers, heads: [{name, kind, classes}], generates, critics: [model names], pretrained, device, loss, accuracy, macro_f1, bits_per_byte, steps, training}` (the metrics from the latest training) |
| `models.get` | `{name}` | the summary and `{source, history: [training], job}`; `source` is the spec as written |
| `models.check` | `{spec}` (an object, or its JSON text) | `{ok: true, name}` or `{ok: false, path, message}`: what `models.create` would say, without storing |
| `models.create` | `{spec}` | details; a spec that doesn't pass answers an error naming where: `model spec "x" at heads[0].classes: head "a": classify needs at least two classes` |
| `models.update` | `{name, spec}` (not while it trains; a new `name` in the spec renames it) | details and `weights_kept`: edits to the data, critics, constraints, sampling, training options or device keep the trained weights; a change to the input, trunk, heads, `from` or `seed` drops them |
| `models.delete` | `{name}` (not while it trains) | `null` |
| `models.train` | `{name, epochs?, max_steps?, fresh?, wait?}` | the job: `{name, state: running\|done\|failed\|cancelled, epoch, step, steps, loss, lr, tokens_per_sec, losses: [[step, loss]], evals: [{step, loss, accuracy, macro_f1, bits_per_byte?}], seconds, examples, error?, model?: summary}` |
| `models.job` / `models.cancel` | `{name}` | the job (or `{name, state: "idle"}`); a cancelled run writes nothing |
| `models.run` | `{name, text or input, temperature?, top_k?, top_p?, max_len?, seed?}` | `{name, trained, seconds, outputs: {head: answer}}`; an answer is `{kind: "classes", probs: {class: p}, top, members}`, `{kind: "values", values: {name: v}}` or `{kind: "generated", text}` (what a generate head writes after the input; `bytes`, `numbers` or `image: {width, height, png}` for other inputs) |
| `models.generate` | `{name, prompt?, candidates?: 8, temperature?, top_k?, top_p?, max_len?, seed?, guided?: false}` | `{name, trained, seed, seconds, critics: [{model, head, want, weight}], candidates: [{text, index, score, scores: [per critic, 0 to 1], logp, kept, broken: [rules]}]}`: kept candidates first, best score first |
| `models.pretrained` | | `{any, folders}`: whether any folder may be named (one's own machine), or the folders a shared hub offers |

- `input` is a value of the model's input type: `{"text": ".."}`, `{"numbers": [..]}`,
  `{"bytes": [..]}`, `{"picture": id}` (a picture of the save, scaled to the model's
  size); `text` is short for the first. `prompt` is text for the generator
  to continue (empty: from nothing).
- **Data** comes from the save when training starts: a `list` query names a list by its
  name (the text of its head entry) or its key; `tagged` an entry tag; `pairs` two lists
  side by side; each member's text (a note: its title and markdown) is one example. For a
  `numbers` input a member's text is the numbers, separated by spaces or commas. A
  picture model's data is the save's [pictures](#pictures) (`pictures`, a list of them
  or all), or inline.
- **Training** runs as the classifiers' does: on a thread of its own, continuing from the
  trained weights unless `fresh`, written when it finishes unless the spec changed
  meanwhile. `epochs` and `max_steps` override the spec's `train` for this run. On a
  shared hub it counts toward the member's two trainings and indexings at once, stops
  after two hours, and its `threads` are at most the hub's cores.
- **Critics** are other spec models of the save, or its trained classifiers (a
  classifier's head is its own name). They judge the prompt and what was written
  together (a `pairs` model: what was written). The same seed gives the same candidates; `guided`
  steers each byte toward what the critics want (slower).
- **Untrained**, a model answers with its starting weights: random for a new trunk, the
  checkpoint's for a pretrained one (`"from": {"pretrained": dir}`), so a pretrained
  generator writes before any tuning.
- **On a shared hub** (not `--local`), a pretrained folder must be one under a folder the
  hub was started with (`--pretrained DIR`, repeatable), named relative to it
  (`models.pretrained` lists them); without any the hub loads none. A trunk may have
  about 40 M weights at most, `"device"` must be the CPU, a call writes at most 2000 bytes
  and draws at most 32 candidates. On one's own machine (the desktop, `--local`) any
  folder and device work.
- `trained` is true when the model has trained and its weights fit its spec.
- In a [bundle](#bundles), `models` carries each as `{record, weights?}` (the record is
  `{name, source, created, modified, version, history}`, the weights the model file in
  base64); the records are in its `other.specs` too.
- **Merging** a save into the open one (`saves.import` with `merge`, `POST
  /app/hub/{id}?merge=1`) brings its spec models in untrained: one with a name the open
  save has already is the same model when its spec is the same JSON (nothing changes),
  and otherwise comes in beside it as `<name> 2` (3, ...; its spec's `name` too), so the
  open save's own models, and the critics that name them, answer as before. The merge's
  answer counts them in `models` (and pictures in `pictures`).

## Pictures

Pictures live in the save itself (tables `graph pictures`, `graph picture bytes`,
`graph picture lists`; [format](format.md#other-tables)), so they go wherever the save
goes: copies, renames, backups (`other.json`'s `pictures`, the bytes in base64), bundles,
the hub's share and take, and merges (a picture is known by the hash of its bytes, so
the same picture in both saves stays one, and its lists are joined). A picture is a PNG
or JPEG, decoded when it comes in so only ones that read are kept. Its `id` is a 128-bit
FNV-1a hash of its bytes (32 hex digits). Lists hold pictures by name: the pictures of
list `"cats"` sit beside the texts of the list whose head is `cats` (or on their own).
Spec models train on them ([models](models.md): `{"from": "pictures", "list": "cats",
"head": .., "class": ..}`) and read one by id (`models.run` with `input: {picture: id}`).
One implementation answers them on the desktop and the hub (`core/src/ops/pictures.rs`).

A picture's details: `{id, mime: "image/png"|"image/jpeg", width, height, name, added (ms
since the Unix epoch), size (bytes), lists: [names]}`.

| op | args | data |
|---|---|---|
| `pictures.add` | `{data (base64, or a data: URL) or path (one's own machine only), name?, list?, thumb?}` | the details and `new` (false when the save had those bytes already: nothing changes) and, with `thumb`, a thumbnail; `name` defaults to the file's name for a path; `list` puts it into that list |
| `pictures.list` | `{list?, thumb?}` | `[details]`, oldest first: the save's pictures or a list's; `thumb` (16 to 512) adds each one's thumbnail, a PNG in base64 fitting `thumb` × `thumb` |
| `pictures.lists` | | `[{list, count}]`: the lists that hold pictures |
| `pictures.get` | `{id, thumb?}` | the details and `data`, the file in base64, or with `thumb` only the thumbnail |
| `pictures.name` | `{id, name}` | the details |
| `pictures.tag` | `{id, list, remove?: false}` | `{id, lists, changed}`: the picture put into a list, or with `remove` taken out |
| `pictures.delete` | `{id}` | `null`; it leaves its lists too |

- **Limits**: on a shared hub (not `--local`) a picture is at most 4 MB and 8192 pixels a
  side, a save holds at most 2000 of them, they come as data (not paths), and they count
  in the member's quota (they're in the save's file); `pictures.add` and `pictures.tag`
  need room under it, the rest work when it's full. On one's own machine a picture is at
  most 64 MB and 32768 pixels a side, with no count limit.

## Blocks (spatial saves)

Blocks placed in the space, kept in the save's own tables (`graph blocks`, a redb
multimap, and `graph block ids`), so they travel with the file, its backups and bundles
(`blocks` in the backup's other file, restored with their ids) and merges (fresh ids; a
text block finds its text in the target by what it says, a picture block its picture by
hash). A block shows a **texture**: a text of the save (`kind: "text"`, `texture` its
key: the entry label, a note's `note:` prefix taken off), a picture (`"picture"`, its
id) or a plain colour (`"plain"`, `#rrggbb`). The table's key is `kind:texture` and its
values are one 32 byte transform per block (the id, then x y z w h d as f32), so a text
shown a thousand times is stored once with a thousand transforms. One implementation
answers them on the desktop and the hub (`core/src/ops/blocks.rs`, `core/src/graph/blocks.rs`).

A block: `{id, kind, texture, x, y, z, w, h, d}`, its centre and its sides in space units.
Sides are kept between 0.05 and 10000, places within 10⁶ of the origin. The texture tiles
at one tile per unit, so a bigger block shows more tiles rather than a stretched one (the
app sets each face's UVs from its size, with repeat wrapping). The answers that list
blocks are `{blocks: [block], textures: {"kind:texture": about}}`, where `about` is
`{text}` for a text (sent once however many blocks show it), `{name, width, height}` for
a picture, `{colour}` for a colour, and `{gone: true}` when what it showed was deleted.

| op | args | data |
|---|---|---|
| `blocks.list` | `{lo?, hi?, kind?, texture?}` | the blocks, by id; `lo` and `hi` (`[x, y, z]` each) keep those whose boxes meet that region |
| `blocks.stats` | | `{blocks, textures}` |
| `blocks.add` | `{kind, texture, x?, y?, z?, w?, h?, d?}` or `{blocks: [..]}` | the new blocks and `ids` (place 0, sides 1 unless given) |
| `blocks.move` | `{id, x?, y?, z?, by?}` | the block, moved (to x y z, or `by` `[dx, dy, dz]`); its size stays |
| `blocks.scale` | `{id, w?, h?, d?}` | the block, resized; its place stays |
| `blocks.set` | `{id, x?..d?, by?}` or `{changes: [..]}` | the blocks, each field given replaced |
| `blocks.duplicate` | `{id or ids, by?: [1, 0, 0]}` | the copies (same texture, fresh ids) and `ids` |
| `blocks.delete` | `{id or ids}` | `{removed: [block]}`: what was taken, to hand back to `blocks.restore` |
| `blocks.restore` | `{blocks}` | `{restored}`: blocks put back with their ids (an undo); taken ids and missing textures are skipped |

- **Limits**: at most 5000 blocks a call; on a shared hub a save holds at most 50 000.
  `blocks.list`, `blocks.stats` and `blocks.delete` work when a member's quota is full.

## Spelling and word choice

A proof of concept of the classifiers (`core/src/grammar`): a detector finds words that
are probably errors (misspellings, repeated words, a confusable word where another of its
set fits better: their/there/they're, its/it's, then/than, ...), and a suggester ranks
replacements. Both are small logistic regressions trained, the first time they're asked
for, on deterministic corruptions of `core/src/grammar/corpus.txt`; a fifth of it is held
out to measure them (`report`). Words the save holds elsewhere are never errors.

| op | args | data |
|---|---|---|
| `grammar.check` | `{text, threshold?: 0.5, exclude?}` | `{flags: [{start, end (bytes), from, to (UTF-16 units), word, p, kind: "spelling"\|"repeated"\|"confused", suggestions: [{text, p}]}], report}`; an empty suggestion takes the word out with the space before it; `exclude` is the key of the note being checked, whose own words don't count as the save's |
| `grammar.report` | | `{train_sentences, test_sentences, precision, recall, f1, errors, flagged, caught, top1, top3, false_alarms_per_1000, threshold, by_kind: [[kind, caught, errors]]}` |

## Saves in and out

| | |
|---|---|
| `GET /app/saves/{name}` | a workspace save, to keep |
| `POST /app/saves` | multipart `file` (and `name`): a save from your computer into the workspace |
| `POST /app/hub/{id}` | an approved hub save into the workspace; `?merge=1` merges it into the open save instead |
| `POST /app/share` | JSON `{name?, description, tags}`: the open save onto the hub |

### Classifiers between the hub and a save

Same rules as the saves' routes: the login cookie (or `--local`), posts only from the
hub's own pages (`Origin` must match), answers in the usual envelope.

| | |
|---|---|
| `POST /app/classifiers/{id}/take` | an approved hub classifier (`id` from `/api/classifiers`) into the open save. Optional JSON `{as?: name, lists?: {class: list key}}`; without `lists` each class goes to the list of its name (made when there's none). Returns `classify.import`'s answer plus `save`: `{name, created, reused, lists: [[class, key, new]], save}`. Counts a download. At most 120 an hour from an address |
| `POST /app/classifiers/share` | JSON `{name, title?, description?, tags?}`: the open save's trained classifier `name` (with everything it builds on bundled) onto the hub, as `title` (its own name when left out). Returns `{id, status: "pending"\|"approved", url}`. At most 20 uploads an hour |

## Bundles

A save as plain JSON: its [backup](format.md#backup) in one document,
`{"anend": 2, "name", "memories": texts.json's rows, "other": other.json}`, plus
`classifiers` (`{definition, weights?, feedback?}` each, the weights base64) and `models`
(spec models, `{record, weights?}` each) when it has any. Importing it restores the save
keys and all; merging it leaves the classifiers out and brings the spec models (untrained)
and pictures in as described under [spec models](#spec-models).
A bundle of weum rows alone (`{"memories": [[text, [key bytes]], ...]}`) imports as weum's
`all-of-it.json` does. Old bundles (`anend: 1` with notes as records) aren't read.

The old JavaScript web app and its offline engine are retired: the web app is the Dioxus
app's web build (`app/`), served at `/app/`. Running without a hub comes with the Rust
core compiled to WebAssembly ([next](next.md)).
