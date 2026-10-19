# Feature parity: the Dioxus app against the Bevy app and the old JS web client

This file is the contract. Every user-visible feature, setting, panel, key, visual and
interaction the earlier apps had is listed here with where it was and where the Dioxus app
stands, on the desktop (D) and on the web (W, the same code against `anend-hub --mode app`).
**Nothing is removed without a line here saying why.** Change this file in the same commit
as any change that adds, moves or drops a feature (docs/next.md's Resuming block says so too).

- Sources: the Bevy app at `cf08139^` (`git show cf08139^:src/<file>`), the JS web client at
  `49e3930^` (`hub/web/`), retired on 18 Oct 2026.
- Status words: **has** (as before, or better), **partly** (what's missing follows), **missing**,
  **dropped** (on purpose, with the reason). "D/W" is given only where the two differ.
- Guards: `app/src/parity.rs` tests pin the key actions and the settings keys (a removed one fails
  the test until this file and the expected list are changed together).

Audited 19 Oct 2026 (before the fixes of that day); the status column is kept current as items close.

## The shell (every view)

| feature | was in | status |
|---|---|---|
| top bar: the views as icon tabs with tooltips (name + key) | ui.rs `nav` | has |
| top bar: the app's mark at its left | ui.rs `AppMark` | dropped: the icon is the window's and the tab's (title bar, favicon), as the user asked 19 Oct |
| top bar: status text, settings button | ui.rs | has |
| top bar: undo button (latest action) | Dioxus only | has |
| the window frame: bars and panels as inset rounded cards on the sand window colour (6px margin and gaps) | ui.rs `root` padding 6 / row_gap 6, theme.rs | has (19 Oct); the memory view's black field joins its black bar, the space fills the window behind the bars, as in Bevy |
| one command bar shared by every view, each view keeping its own text | bar.rs | has |
| command bar: the view's name at its left | bar.rs `BarMode` | dropped: the user asked 19 Oct for the bar's left side not to name the view |
| command bar: the view's additions (counts, hints) at its right end | bar.rs `BarSide`, `notes_side` | has (19 Oct): one bar; the notes' count is there |
| the memory view's and the space's sums and toggles in the bar | bar.rs `BarStrip` | has (19 Oct): in the one bar's right end, not a strip over it (the user asked for one bar) |
| the bar's undo / redo (Ctrl+Z / Ctrl+Shift+Z in the bar) | Dioxus only | has |
| toasts | memory_ui.rs, space_ui.rs | has |
| right-click menu on a text (edit, tag, select, list, candidate, key, forget, kind) | Dioxus only | has |
| zen: chrome fades while the mouse rests, delay setting | zen.rs | has |
| hide the top bar until the pointer nears the top | zen.rs | has |
| bars swapped (top bar at the bottom, command bar at the top) | config `bars_swapped` | has (19 Oct) |
| sizes: interface scale, top bar height, command bar height, "default sizes" | sizing.rs | has (zoom, text size, both bars' heights) |
| fonts: interface font and memory view font pickers (system fonts listed) | typeface.rs, settings.rs | partly (19 Oct): a family name for each, with common ones offered; a webview can't list the system's fonts |
| window icon (desktop title bar) | main.rs | has (19 Oct: it never reached the window before, `Config::with_icon` was undone by `with_window`; now on the window, checked as `_NET_WM_ICON` under Xvfb; a desktop entry is put in place for Wayland desktops) |
| web tab favicon and title | hub/web/index.html | has |
| saves the open note when the window closes | ui.rs `on_close` | partly: the Dioxus editor saves on Ctrl+S / button only |
| the guide button (opens docs/guide.md) | settings.rs | has |

## Keys

| feature | was in | status |
|---|---|---|
| every action rebindable in a keys menu (click a binding, press the key, Esc cancels, swap on clash, reset to defaults), kept in the config | keys.rs, ui.rs `rebuild_remap` | has (19 Oct): app/src/keys.rs, the settings' keys section, `Prefs::keymap`; the page's listener keeps exactly the bound keys |
| 57 actions: views, save, new note, view/edit, slide the list, focus bar, buckets, toggles, sidecar arrows, flying, roll, level, home, speed, new/edit/delete/copy text, undo, reversed, pin, sliders, pause, settle, side sums, help, hud, export, save map, keys menu | keys.rs `Action` | has: 64 actions (the Bevy ones, the atbash ones dropped with atbash, plus zoom, word types, details, the reader's steps, the note's classifiers and checking); space.js flies on the bound keys. Some actions wait for their feature (see the space and memory rows) |
| Ctrl+1..9 to the views | keys.rs | has (order differs: Dioxus is memory, space, lists, reader, notes, tags, stats, models, filters; Bevy was editor, reader, memory, stats, space, lists, classifiers, tags, filters) |
| Ctrl+, settings, Ctrl+K / K the keys menu | keys.rs | has (Ctrl+K opens the settings at the keys) |
| `/` focuses the command bar | keys.rs `FocusTerm` | has (and Enter outside the bar) |
| zoom keys (Ctrl+= / Ctrl+− / Ctrl+0, PageUp/PageDown) | Dioxus only | has |

## Theme and visuals

| feature | was in | status |
|---|---|---|
| colour roles (13) with presets sand, night, paper, sea; per-role colour; reset | palette.rs, settings.rs | has |
| the sand preset's exact colours (window hsl(39 30% 44%), bars hsl(0 0% 25%), cards white, pad hsl(39 19% 82%), buttons hsl(39 30% 66%), accent hsl(39 42% 56%), text 18%, dim 42%, link hsl(30 55% 34%), code hsl(39 19% 90%), room #e4d4be, bar text 92%, bar labels 62%) | theme.rs, bar.rs, space.rs | partly: the hex values match, but the window colour only shows as a thin gap, not the frame around inset cards |
| colour editing with hue / saturation / lightness sliders and a hex box | settings.rs | partly: a colour input per role |
| Fira Sans / Fira Mono, bundled | typeface.rs (Bevy feathers fonts) | has (19 Oct): subset woff2 in assets/fonts, embedded as @font-face |
| buttons: sand fill, accent when selected, dark/light text chosen by contrast | theme.rs | has |
| the editor's markdown highlighting (headings, marks, emphasis, strike, code, links, urls, lists, quotes, tables, tags, rules, composing), colours derived from the palette for contrast | highlight.rs | has (19 Oct): app/src/highlight.rs (its tests ported), a layer under a transparent-glyph textarea; theme.rs `hl_vars`; composing is the textarea's own |
| setting: highlight on/off | config `highlight` | has (`Prefs::highlight`) |

## Notes (the editor)

| feature | was in | status |
|---|---|---|
| note list newest first, new note button, the open one marked, unsaved marker (•) | ui.rs `rebuild_list` | has |
| `# title` + markdown in the pad, mono font | ui.rs | has |
| tags as chips with ×, a tag box (Enter or comma adds) | ui.rs `rebuild_meta`, `tag_input_key` | has |
| view / edit toggle (Ctrl+E) rendering the note | ui.rs | has |
| save (Ctrl+S), new note (Ctrl+N) | keys.rs | has; a note without a # title isn't saved, as before |
| delete with a confirmation | ui.rs `confirm_delete` | has (two clicks) |
| slide the note list away / back (Ctrl+B), list on the right, hide until the pointer nears the edge | ui.rs, config `hide_list`, `list_right` | partly: Ctrl+B and the button; the list on the right (settings); no pointer reveal |
| the note's classifiers: pick which classifiers index it, each one's class chip and a yes/no | index_panel.rs, JS notecls.js | has (19 Oct): views/judge.rs, every class's probability too (Ctrl+I) |
| markdown highlighting in the pad | highlight.rs | has |
| the bar finds notes by words; Enter opens the first; "n of m notes" at the bar's end | bar.rs | partly: Enter filters, the count is there; Enter doesn't open the first |
| written date and edits count | ui.rs | has |
| replacements `{{key}}`, `{{filter:name}}`, alternatives from lists | Dioxus only | has |
| the notes view has no horizontal scrollbar dragging the content off to the left | — | has (19 Oct): the view never scrolls itself; its list and pad do |

## Reader

| feature | was in | status |
|---|---|---|
| notes as cards with title, date (long date, "edited …" relative), tags, rendered markdown | ui.rs `rebuild_reader` | has: the opened one (19 Oct); the cards show the text |
| a tag chip filters the reader to that tag, "show all" | ui.rs | has (from the opened note's tags; × shows all) |
| edit button on a card opens the editor | ui.rs | has (on the opened note) |
| the bar narrows by words | bar.rs | has |
| notes, texts or everything; pages; expand one to read big, ← → steps | Dioxus only | has |
| the opened item's dates (created, updated) | asked 19 Oct | has (19 Oct): written and last changed, long and relative, the edits count |
| classifiers on the opened item: pick classifiers, see each class's probability | asked 19 Oct | has (19 Oct): views/judge.rs, with yes / no |

## Memory view

| feature | was in | status |
|---|---|---|
| weum's black field, the results card (#c9ab99), articles #282626, serif | memory_ui.rs | has |
| Enter / Ctrl+Enter / Shift+Enter / Delete as weum | memory_ui.rs | has |
| buckets O / R / F (R and F toggle back) | memory_ui.rs | has |
| the atbash bucket (A, ↑) | memory_ui.rs | dropped: an atbash sum equals the reversed sum letter for letter; the entry graph files ordinal and reversed only (`graph::gematria`) |
| sidecar (sums, letters), each clickable; ← → | memory_ui.rs | has |
| ↓ gsearches the letters (frequency) | memory_ui.rs `SideFrequency` | has |
| toggles: gsearch text (G), saturate (S), white mode (W) | memory_ui.rs | has (white mode back 19 Oct) |
| list commands Enlist / List / Delist / Unlist; Shift+click list, Ctrl+click candidate | memory_ui.rs | has |
| click: into the bar; double click: key; hover line (key + gsum) | memory_ui.rs | has |
| middle click opens a note in the editor | memory_ui.rs | missing |
| Ctrl+right click: forget and re-saturate under a new key | memory_ui.rs | dropped for weum's Ctrl+right click (gsearch ⌊√(s·r)⌋), which the space had too; re-keying is not offered |
| underlines by shared reversed sum (the gematria colour of a text's reversed-ordinal match group) | memory_ui.rs | has; **must stay**: word-type labels (T) are a separate channel (a coloured label under each word) and never take, replace or recolour the underline (guard: `parity::tests::word_types_keep_the_gematria_underline`) |
| colouring by a classifier (bar at the left, legend) | tint.rs | has |
| empty-state hint | memory_ui.rs | has |
| word types on hover: which types, not only how many (noun, verb, name, number, link…, coloured) | asked 19 Oct | has (19 Oct): core `kinds::tag`, a rule tagger (noun, verb, adjective, adverb, pronoun, determiner, preposition, conjunction, interjection, name, number, date, time, link, email, hashtag); the hover line labels each word in its type's colour; T labels every text, with a legend |
| search filter panel, buckets strip at the left edge, field width, selection | Dioxus only | has |

## Space

| feature | was in | status |
|---|---|---|
| fly: right drag looks, WASD, Space/C, Q/E roll, Shift fast, [ ] / wheel speed, R level, Home | space.rs | has (space.js) |
| texts as camera-facing labels, sized by distance | space.rs (slug text) | has (19 Oct): one font size, placed and scaled by a sub-pixel transform each frame, so they move smoothly |
| the spread: physics with damping, gravity, repulsion, ranges, buckets attract/repel/align, anchor, depth, distance, cooling, steps per frame, text size, wrap, hide under px | space.rs `SLIDERS` | has, as a free spread (19 Oct): live physics in space.js; damping, gravity, repulsion, attraction, link length, gap, cooling, speed, depth (by a measure) and flatness, distance, text size, wrap, hide under px. The bucket attract/repel/align and the spiral anchor are gone on purpose: the user asked for no spiral; bucket-ness is the "same bucket" weight |
| the sliders panel (O), reset all, double-click a name resets it, settle (K), pause (P) | space_ui.rs `build_opts` | has (19 Oct): the physics window, movable; settings kept in `Prefs::space` |
| a proper free force-directed spread whose attraction and repulsion are weighted by the texts' metrics | asked 19 Oct (not the Archimedean spiral) | has (19 Oct): weights −1..1 for ordinal, reversed, the same bucket, length, kind and word types, shared lists, the colouring classifier's class, time, letters (`space.features` in core) |
| drag a text to move it (pinned after), wheel on a text pushes/pulls it, pin / unpin (U) | space.rs | has |
| new text (N, double click empty), edit (Enter), delete, copy (Ctrl+C), undo (Ctrl+Z) | space.rs | has (edits go through the bar; undo covers moves, pins, removals, new texts and edits, then the app's undo) |
| the metrics card (s, r, s+r, s·r, √, ⌊√⌋, s−r, wide, lean, gap, v, c, w, letters; key, bucket colour ×n, pinned; pf line), numbers gsearch | space_ui.rs | has (19 Oct): every number; the bucket's count ×n is not shown |
| the details panel movable, its close button hidden until hover | asked 19 Oct | has (19 Oct): dragged by its title, where it was left kept; M shows / hides it |
| position card (I): x y z, speed, roll, texts, spread state | space_ui.rs | has |
| help card (H) with the keys as bound | space_ui.rs | has |
| first-time hint | space_ui.rs | has: the empty room says how to start |
| terminal: Enter, Ctrl+Enter, Shift+Enter, Delete, ↑ ↓ history (kept) | space.rs | has |
| `#123`, `#r123`, `#name` addresses; the last address reopened | space.rs `route` | has |
| words: Enlist List Delist Unlist Count Backup Clear ClearAll Settle Maps Save name | space.rs `COMMANDS` | has (and Help) |
| maps: save (Shift+X / Save name), list (Maps), open (#name); export the texts to a file (X) | space.rs | has: maps in the save's `space.maps` setting; export to the data folder's exports (desktop) or a download (web) |
| chips: gsearch, reversed (F), saturate, keep | space_ui.rs | has |
| the search line in the top bar (what, counts with icons, pf chain, the triangle with sides and angles) | space_ui.rs `rebuild_qline` | partly: title, mode, count, triangle |
| bucket underline colours (seedCLR), class colours from a classifier | space.rs, tint.rs | has |
| the solid box in the room | space.rs `SOLIDS` | dropped: a test fixture of remem's, never used for anything |
| list / candidate marks | Dioxus | has |
| one bar (the space's strip merged into the command bar) | asked 19 Oct | has (19 Oct) |
| arrows hop to the nearest text | Dioxus only | has |

## Lists

| feature | was in | status |
|---|---|---|
| every list with members, figures (members, ordinal, ordinal mean, reversed, reversed mean) | lists_ui.rs | has (19 Oct): a card per list |
| the list as one text (ordinal, reversed, letters), gsum detail, pf chain | lists_ui.rs | has |
| its letters as a vector (bars a–z) on each list | lists_ui.rs | has |
| "how alike": the cosine matrix of every two lists' letter vectors, coloured | lists_ui.rs | has |
| members as chips with their s·r and × to delist | lists_ui.rs | has (and editable rows on the list's page) |
| choose a list; the bar's Enter adds to it (saturating new texts); "let go of it"; with none chosen, Enter starts a list | lists_ui.rs `bar_key` | has (and Enter on a list's name chooses it) |
| unlist from the overview | lists_ui.rs | has (two clicks, Ctrl+Z brings it back) |
| rename a list, edit members in place, pages, pictures panel, undo | Dioxus only | has |

## Tags, stats

| feature | was in | status |
|---|---|---|
| tags with counts, Ctrl/Shift+click multi-pick, all/any, the notes carrying them, open in the editor | tags_ui.rs | has (the open-in-editor link: partly) |
| stats: notes, words, saves/edits, first and latest note, tags with counts opening the tags view | ui.rs `rebuild_stats` | partly: entry-graph counts, no words written, tag chips don't open the tags view |

## Classifiers (the models view)

| feature | was in | status |
|---|---|---|
| list, new from lists (classes, not-list, builds on, size, one class / several) | classify_ui.rs | has |
| starter examples (fruit, stones, animals) | classify_ui.rs | missing |
| training: epochs, rounds, from scratch, progress, loss line, cancel, metrics, confusion, growth | classify_ui.rs | has (growth history back 19 Oct); the page has the Bevy tabs again: learn, use, buckets, checks |
| use: try a text, yes/no per class, "why" (each classifier's part, end to end) | classify_ui.rs, `classify.contributions` | has |
| steering: bias per class, temperature, threshold, channel gains, the bias it adds (flipped, KL) | classify_ui.rs | has |
| buckets: every text filed by class and sureness (tenths), index again | classify_ui.rs, `classify.buckets` | has (19 Oct): views/clsmore.rs |
| near texts by meaning (embedding) or verdict | classify_ui.rs, `classify.near` | has |
| outputs as a list (write the list) | classify_ui.rs, `classify.enlist` | has |
| checks: validators, disagreements | cls_more.rs | has |
| share / take classifiers on the hub | cls_more.rs, JS clsmore.js | missing |
| export / import `.anendcls` | classify_ui.rs | has |
| colour views by a classifier | classify_ui.rs | has (picked in the memory strip) |
| device: CPU / GPU / GPU fast, and which device a job ran on | asked 19 Oct | missing |
| spec models (templates, JSON editor, train, run, write) | Dioxus only | has |

## Filters

| feature | was in | status |
|---|---|---|
| conditions by classifier verdict, words, lists, tags, kind | filters_ui.rs, `filters.*` | has (19 Oct): views/flt.rs, beside the query filters (words, sum, time, tag, kind) |
| "all of" / "any of" groups, nested | filters_ui.rs | has, and "not" on any condition or group |
| presets that include each other | filters_ui.rs | has |
| write the results out as a list | filters_ui.rs, `filters.enlist` | has |
| select dropdowns sized so the text sits inside them | — | has (19 Oct): every select draws its own arrow, text centred (style.css) |

## Forge

| feature | was in | status |
|---|---|---|
| the forge: saves in, ingredients (+ − ∩), result preview, filters (words, ordinal, reversed), name, forge / pour into the open save | forge_ui.rs, JS forge.js, `forge.*` | has (19 Oct): views/forge.rs, from the settings' head |

## Saves, the hub (settings)

| feature | was in | status |
|---|---|---|
| saves: list, open, new, copy, rename, delete, backup, import weum JSON | settings.rs | has |
| saves folder setting (a folder lists its saves, a .redb path opens one) | settings.rs | missing |
| export / import a bundle (.anend.json) | JS settings.js | has (19 Oct) |
| the hub: connect with an app token, browse and download shared saves, share this one | hub_ui.rs | missing (the address box only) |
| local-only browser saves (IndexedDB) | JS local/ | dropped: the web app runs against the hub's app mode (`anend-hub --mode app --local` for one person) |

## New since the Bevy app (keep)

The Dioxus app adds: the undo stack and button, the right-click menu with kinds, the memory
view's search filter, bucket strip, field width and selection, pictures, spec models,
replacements and alternatives in the editor, the reader's pages and big reading, zoom.

## Not yet (planned)

| feature | status |
|---|---|
| WebGPU training in the browser (task #24) | not yet |
| spatial saves (blocks textured with pictures and text, gizmo) | not yet |
| grammar and spelling proof of concept in the editor | not yet |
| space search modes (normal, classifier, random), save checkpoints, a real IME test | not yet |
