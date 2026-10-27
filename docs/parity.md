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
| **must keep:** typing in the command bar, the text stays put: the input spans the bar with equal room at both ends and the view's additions sit over its right end, so their changing sizes never move the text (19 Oct, measured under Xvfb: the text's centre stayed at x = 600.5 over four words while the sums changed; guard `parity::tests::the_bar_text_stays_put`) | asked 19 Oct | has |
| **must keep:** the typing field is never narrower than 15.5 cm (`min-width: 15.5cm`; the room at each end is what's left of the bar's width once the field has its 15.5 cm, at most 520px a side, and depends on nothing else, so the text never shifts); measured: 585.8px of field (15.5 cm at 96 px per inch) in a 1268px bar on the web and the desktop. CSS centimetres are 96 per inch at every scale factor, so the field is 15.5 cm of CSS length on every display (19 Oct, queued 1) | asked 19 Oct | has |
| **must keep:** the view's items (sums, counts, chips) sit beside the typing field on the bar's line while they fit in the room at its right; those that don't wrap upward into a see-through bar just above it, and in a bar narrower than about 866px all of them go there (19 Oct, queued 2; checked under Xvfb in the memory and space views at 1200 and 820 wide, and on the web) | asked 19 Oct | has |
| the memory view's and the space's sums and toggles in the bar | bar.rs `BarStrip` | has (19 Oct): in the one bar's right end, not a strip over it (the user asked for one bar) |
| the bar's undo / redo (Ctrl+Z / Ctrl+Shift+Z in the bar) | Dioxus only | has |
| toasts | memory_ui.rs, space_ui.rs | has |
| right-click menu on a text (edit, tag, select, list, candidate, key, forget, kind) | Dioxus only | has |
| zen: chrome fades while the mouse rests, delay setting | zen.rs | has |
| hide the top bar until the pointer nears the top | zen.rs | has |
| bars swapped (top bar at the bottom, command bar at the top) | config `bars_swapped` | has (19 Oct) |
| sizes: interface scale, top bar height, command bar height, "default sizes" | sizing.rs | has (zoom, text size, both bars' heights) |
| fonts: interface font and memory view font pickers (system fonts listed) | typeface.rs, settings.rs | has on the desktop (26 Oct): `fonts::scan_system` reads the family from each font file in the system's font folders (TrueType/OpenType `name` tables, collections, Type 1) at start, and both fields offer them (the hint counts them); checked under Xvfb (all of the container's families, Courier 10 Pitch and Bitstream Charter included; Caladea applied), app/docs/shots/system-fonts.png. The web offers common names: a page can't list the system's fonts |
| window icon (desktop title bar) | main.rs | has (19 Oct: it never reached the window before, `Config::with_icon` was undone by `with_window`; now on the window, checked as `_NET_WM_ICON` under Xvfb; a desktop entry is put in place for Wayland desktops) |
| web tab favicon and title | hub/web/index.html | has |
| saves the open note when the window closes | ui.rs `on_close` | has (19 Oct): closing the desktop window saves a titled note with changes, then the app ends (tested under Xvfb with a WM_DELETE_WINDOW message: the note was there on the next start); leaving the notes view saves it too |
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
| the sand preset's exact colours (window hsl(39 30% 44%), bars hsl(0 0% 25%), cards white, pad hsl(39 19% 82%), buttons hsl(39 30% 66%), accent hsl(39 42% 56%), text 18%, dim 42%, link hsl(30 55% 34%), code hsl(39 19% 90%), room #e4d4be, bar text 92%, bar labels 62%) | theme.rs, bar.rs, space.rs | has (26 Oct, checked again): the hex values match, and the window colour is the frame around the inset cards and bars (6 px, the shell's row above); it's editable like any role (below) |
| colour editing with hue / saturation / lightness sliders and a hex box | settings.rs | has (26 Oct): "edit a colour" in looks: pick a role, then hue, saturation and lightness sliders and a hex box, each change shown at once; the colour inputs per role stay; "the palette's own" puts them back (checked on the desktop under Xvfb: the bars, and the web: the window frame) |
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
| slide the note list away / back (Ctrl+B), list on the right, hide until the pointer nears the edge | ui.rs, config `hide_list`, `list_right` | has (19 Oct): Ctrl+B and the button; the list on the right (settings); slid away, it waits off its edge and slides in while the pointer is within 10 px of that edge or on the list (CSS only; checked under Xvfb: hidden, shown at the edge, kept on the list, gone on leaving) |
| **must keep:** the notes view opens with its note list shown, never a blank page; sliding the list away holds while a note is open, and comes back when none is (19 Oct, F7; checked under Xvfb with `hide_list` on) | asked 19 Oct | has |
| the note's classifiers: pick which classifiers index it, each one's class chip and a yes/no | index_panel.rs, JS notecls.js | has (19 Oct): views/judge.rs, every class's probability too (Ctrl+I) |
| markdown highlighting in the pad | highlight.rs | has |
| the bar finds notes by words; Enter opens the first; "n of m notes" at the bar's end | bar.rs | has (Enter narrows, Enter again opens the first) |
| **must keep:** one notes count in the notes view: at the bar's end ("n of m notes" while narrowed); the top line leaves its notes count out there and keeps it in every other view (19 Oct, queued 12) | asked 19 Oct | has |
| the notes view as the typing view for long texts: smooth with a 1 MB text | asked 19 Oct (queued 12, 6) | partly (19 Oct): above 100 KB the colour layer and the spelling check pause, so a key doesn't re-lay out the whole text twice; measured on the web (headless Chromium): a 526 KB note takes about 100 ms from a key to the second frame after it, a short note 33 ms (two frames); highlighting itself is 6 ms for 526 KB natively (`highlight::timing`). Windowed highlighting and not sending the whole text to Rust on every key come with Stage B's large texts (item 6). 25 Oct, typing in segments (the user's approach): a key sends Rust only the range it changed, and the colour layer is kept in segments cut at blank lines outside code fences, so only the segments around the key are coloured and laid out again; the colours stay on at any length (the spelling check still pauses above 100 KB). Measured in this container's software-rendered Chromium, ms a key: 90 KB note 171 → 36; 526 KB note 94 without colours → 142 with them, where a bare 526 KB textarea alone takes 82 (checked on the desktop under Xvfb too: typing, a new code fence, the pad's undo). Storage in pieces and reading in windows are still to come |
| the spelling underlines show with the markdown colours off too | — | has (19 Oct): the layer shows when there are flags, coloured or not |
| written date and edits count | ui.rs | has |
| replacements `{{key}}`, `{{filter:name}}`, alternatives from lists | Dioxus only | has |
| the notes view has no horizontal scrollbar dragging the content off to the left | — | has (19 Oct): the view never scrolls itself; its list and pad do |
| spelling and word choice: likely errors underlined as one writes (red: not a word; blue: repeated or confused), a click shows what each may be with its probability, a click on one puts it in, Ctrl+Z takes it out; a toggle (the check button, setting `check_spelling`); its measured precision and recall under the pad | asked 19 Oct | has (19 Oct, desktop and web): `core::grammar`, a detector and a separate suggester trained on deterministic corruptions; guard `parity::tests::the_spelling_check_stays` |
| Ctrl+Z / Ctrl+Shift+Z in the pad | the browser's own | has (19 Oct): the pad keeps its own undo steps (the desktop web view had none: Ctrl+Z did nothing there) |
| s programs (#11, 26 Oct): a note tagged `s` is a program (docs/s.md). The pad's "s" button tags it; then the pad is s's code editor: the text stands right-aligned (the acting word at the right), coloured by s's word classes (commands, suffix forms, brace and quote literals, numbers, the library's words, `\` escapes), with the spelling check off. ▶ runs the note's body (`s.run`): the output, or the error, shows under the pad with the steps it took; a program's files are the save's s programs by title, so `name.` runs another. A run takes at most 2 million steps on a shared hub, 50 million on one's own machine. The language itself is `anend_core::s` (tail calls in constant space, s §6) and the 5-bit format's s mode stores programs shorter (docs/format.md). Checked on the web and the desktop under Xvfb (a fibonacci loop in tail position that calls a second program: "fib of 30 is 832040", "HELLO, WORLD!", 488 steps) | queued requests #11 | partly: right-aligned mode, word-class colours and running are in; outline, go to definition, inline results, step debugging and completion are not yet. Shots: app/docs/shots/web-s-run.png, desktop-s-run.png; guard `parity::tests::s_programs_stay` |

## Reader

| feature | was in | status |
|---|---|---|
| notes as cards with title, date (long date, "edited …" relative), tags, rendered markdown | ui.rs `rebuild_reader` | has: the opened one (19 Oct); the cards show the text |
| **must keep:** a note's card in the reader shows when it was written (and "edited …" when it was), and the opened note its full dates (19 Oct; guard `parity::tests::reader_cards_keep_their_dates`) | ui.rs `rebuild_reader` | has |
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
| middle click opens a note in the editor | memory_ui.rs | has |
| Ctrl+right click: forget and re-saturate under a new key | memory_ui.rs | dropped for weum's Ctrl+right click (gsearch ⌊√(s·r)⌋), which the space had too; re-keying is not offered |
| underlines by shared reversed sum (the gematria colour of a text's reversed-ordinal match group) | memory_ui.rs | has; **must stay**: word-type labels (T) are a separate channel (a coloured label under each word) and never take, replace or recolour the underline (guard: `parity::tests::word_types_keep_the_gematria_underline`) |
| colouring by a classifier (bar at the left, legend) | tint.rs | has |
| empty-state hint | memory_ui.rs | has |
| word types on hover: which types, not only how many (noun, verb, name, number, link…, coloured) | asked 19 Oct | has (19 Oct): core `kinds::tag`, a rule tagger (noun, verb, adjective, adverb, pronoun, determiner, preposition, conjunction, interjection, name, number, date, time, link, email, hashtag); the hover line labels each word in its type's colour; T labels every text, with a legend |
| search filter panel, buckets strip at the left edge, field width, selection | Dioxus only | has |
| a long text shows piece by piece: its first piece (lines of at most 80 characters broken at word boundaries, six lines a piece, both settings in "the memory view and the space"), then "more" (four pieces more), "all" and "less", with how many pieces and KB; the words' types strip shows its first piece. Before 25 Oct a long text's card held the whole text, clipped, and opened whole on hover (checked on the desktop under Xvfb with word types on, and on the web: a 10 KB text in 21 pieces) | asked 19 Oct (answer 7) | has; the space shows a long text's first piece and how many more (its label); the reader and storage in pieces still to come |

## Space

| feature | was in | status |
|---|---|---|
| fly: right drag looks, WASD, Space/C, Q/E roll, Shift fast, [ ] / wheel speed, R level, Home | space.rs | has (space.js) |
| **must keep:** looking round is yaw about the world's up and pitch clamped short of straight up/down, kept apart from roll (Q/E only), eased smoothly onto where the mouse puts it; circling the mouse never spins the view; look sensitivity and smoothness in the physics window's camera group (19 Oct, F5; checked under Xvfb: three right-drag circles left the view where it started, roll 0°) | asked 19 Oct | has |
| texts as camera-facing labels, sized by distance | space.rs (slug text) | has (19 Oct): one font size, placed and scaled by a sub-pixel transform each frame, so they move smoothly |
| the spread: physics with damping, gravity, repulsion, ranges, buckets attract/repel/align, anchor, depth, distance, cooling, steps per frame, text size, wrap, hide under px | space.rs `SLIDERS` | has, as a free spread (19 Oct): live physics in space.js; damping, gravity, repulsion, attraction, link length, gap, cooling, speed, depth (by a measure) and flatness, distance, text size, wrap, hide under px. The bucket attract/repel/align and the spiral anchor are gone on purpose: the user asked for no spiral; bucket-ness is the "same bucket" weight |
| the sliders panel (O), reset all, double-click a name resets it, settle (K), pause (P) | space_ui.rs `build_opts` | has (19 Oct): the physics window, movable; settings kept in `Prefs::space` |
| a proper free force-directed spread whose attraction and repulsion are weighted by the texts' metrics | asked 19 Oct (not the Archimedean spiral) | has (19 Oct): weights −1..1 for ordinal, reversed, the same bucket, length, kind and word types, shared lists, the colouring classifier's class, time, letters (`space.features` in core) |
| drag a text to move it (pinned after), wheel on a text pushes/pulls it, pin / unpin (U) | space.rs | has |
| new text (N, double click empty), edit (Enter), delete, copy (Ctrl+C), undo (Ctrl+Z) | space.rs | has (edits go through the bar; undo covers moves, pins, removals, new texts and edits, then the app's undo) |
| **must keep:** a right-click menu on every text in the room: edit, pin/unpin, a copy that inherits, a detached copy, edit this placement only (and follow the entry again), copy the text, open in notes (a note) or into the memory view's bar, put into the chosen list, make it the list / candidate, tag, gsearch its sum / reversed sum / ⌊√(s·r)⌋, its key into the bar, the details window, take this placement out, forget the entry (19 Oct, F2; the chain items since 25 Oct; guard `parity::tests::space_texts_have_a_menu_and_safe_copies`) | asked 19 Oct | has |
| chains of texts (queued #8, answer 4): a chain is an entry of its own (its title, then its members in order) that searches and opens as its title; in the space its members hang under the title in reading order, spaced by their sizes, and follow it like springs; dragging any member drags the chain; the menu starts a chain from a text, adds texts to the chain being built, moves a member earlier or later, cuts the chain before a member (that member heads a new chain), takes one out, and breaks the chain (its texts stay in the room); a saved scene keeps a chain as one placement, its head's; a note in the space shows as a chain of two, its title and its body, with a short menu on the body (open the note). Checked on the desktop under Xvfb (start, add) and the web (start, add three, reorder, drag a member, save, reopen, cut, break; a note) | asked 19 Oct | has (25 Oct); blocks in chains (the "block mode" of #8) not yet |
| **must keep:** a copy of a text in the room is another placement of the same entry: taking a placement out (Delete, the menu) never touches the entry; forgetting the entry is its own item, confirmed with a second click and undoable, naming how many placements go with it; the menu and the details window say which placement this is (the first or a copy) and how many there are; a double click on the empty room makes an empty new text (Shift: with the bar's text), so it no longer copies the text just clicked (19 Oct, F4; checked under Xvfb: copy, remove the copy, the entry count unchanged) | asked 19 Oct | has |
| **must keep:** copies inherit or are detached, chosen up front (format.md answers 3, 8): a copy that inherits shows its entry's text as it changes, and editing the entry updates every inheriting placement in the room (before 25 Oct only the first placement updated); "edit this placement only" gives one inheriting copy its own text over the entry's (◆), "follow the entry again" drops it; a detached copy keeps the text as it read when it was copied (◇), stored in the scene's detached table with the entry's version and shown from its history; the menu and the details window say which a placement is. Saved scenes, backups and merges keep all three (a merge brings a detached copy in as the text it shows). Copies made beside a text step down so they don't land on each other (25 Oct; checked on the desktop under Xvfb and the web: copy both ways, edit the entry, own text, save, clear, reopen) | asked 19 Oct (answers 3, 8) | has: texts; prefabs inherit when prefabs exist (#19); blocks textured by an entry show its text now, with no detached block yet |
| each text placement has its own size: − and + (0.1 a click; sizes are steps of 0.02 units per em, a `u8` in the save, 0 meaning the physics window's text size) and "default", in the right-click menu and the details window, undone by Ctrl+Z; saved scenes keep it (25 Oct, format.md answer 6; checked on the desktop under Xvfb, also after restarting the app, and on the web) | asked 19 Oct (answer 6) | has |
| the save's limits as settings ("limits of this save": a rendered text's size, 0 for no cap by default as answer 9 asks; the piece size for stored long texts; borrowing's range; texts per call; the spelling check's bytes), each with its default; the notes view pauses its spelling check at that last one (25 Oct; checked on the desktop under Xvfb and the web) | asked 19 Oct (answer 9) | has |
| the space's windows (scenes, blocks, physics) scroll only up and down: before 25 Oct a tooltip could make one scroll sideways and hide its left edge | found 25 Oct | has |
| the metrics card (s, r, s+r, s·r, √, ⌊√⌋, s−r, wide, lean, gap, v, c, w, letters; key, bucket colour ×n, pinned; pf line), numbers gsearch | space_ui.rs | has (19 Oct): every number; the bucket's count ×n is not shown |
| the details panel movable, its close button hidden until hover | asked 19 Oct | has (19 Oct): dragged by its title, where it was left kept; M shows / hides it |
| **must keep:** the space's windows (details, physics, blocks) stay inside the room: clamped by the layout when the window is resized (and on restore from a saved place), clamped again when a drag ends; dragging them selects no text (19 Oct, F1; checked under Xvfb by shrinking the window to 700×600 and back) | asked 19 Oct | has |
| position card (I): x y z, speed, roll, texts, spread state | space_ui.rs | has |
| help card (H) with the keys as bound | space_ui.rs | has |
| first-time hint | space_ui.rs | has: the empty room says how to start |
| terminal: Enter, Ctrl+Enter, Shift+Enter, Delete, ↑ ↓ history (kept) | space.rs | has |
| **must keep:** Enter in the space's bar does what it did and then lets go of the bar, so W A S D and the arrows fly again; typing starts again with a click on the bar or the focus key (19 Oct, F3 / queued 3; checked under Xvfb: Enter, then W moved the camera 6.2 units; guard `parity::tests::the_space_bar_lets_go_after_enter`) | asked 19 Oct | has |
| `#123`, `#r123`, `#name` addresses; the last address reopened | space.rs `route` | has |
| space search modes: normal (sums and words), classifier (a class of the colouring classifier, then words: its members, or the likeliest), random (weum's random hashmap search: texts sharing a word with what's typed, `*` any, filtered on several threads and shuffled; `mem.random`) | weum, next.md backlog | has (19 Oct): the select at the bar's right in the space; checked under Xvfb (random “red” found the red fox); `ops::tests::random_search_samples_texts_sharing_a_word` |
| words: Enlist List Delist Unlist Count Backup Clear ClearAll Settle Maps Save name | space.rs `COMMANDS` | has (and Help) |
| maps: save (Shift+X / Save name), list (Maps), open (#name); export the texts to a file (X) | space.rs | has: saved as scenes in the save (`scenes.*`, 20 Oct; before that the `space.maps` setting, which moves into scenes when an older save opens); export to the data folder's exports (desktop) or a download (web) |
| **must keep:** the scenes window (folder chip): every saved scene with its count, a click opens it, rename in place, delete with a second click and an undo, and the room saved under a name (19 Oct, F6; checked under Xvfb: save, rename, delete, undo; guard `parity::tests::scenes_window_stays`) | asked 19 Oct | has |
| chips: gsearch, reversed (F), saturate, keep | space_ui.rs | has |
| the search line in the top bar (what, counts with icons, pf chain, the triangle with sides and angles) | space_ui.rs `rebuild_qline` | has (26 Oct): in the space, what was searched and its mode, the room's texts and buckets with their icons (counted when the texts are placed), the triangle drawn and written (`s r t · a° b° c°`, or that it lies flat), and a gsearched number's pf chain last (its numbers go into the bar; the chain gives way when the bar is narrow, its title has it whole). The memory view's line gets the written triangle too; its pf chain stays in the results' head. Checked on the desktop (Xvfb) and the web (Playwright): app/docs/shots/space-searchline.png, web-space-searchline.png |
| bucket underline colours (seedCLR), class colours from a classifier | space.rs, tint.rs | has |
| **must keep:** every space text underlined in its gematria grouping colour, the same as the memory view's (the reversed-ordinal match groups), together with the spread and the smooth labels (lost in the 19 Oct spread rewrite, which underlined only texts sharing a bucket in the room; back the same day; guard `parity::tests::space_keeps_the_grouping_underlines`) | space.rs, the earlier Dioxus space.js | has |
| the solid box in the room | space.rs `SOLIDS` | dropped: a test fixture of remem's, never used for anything |
| list / candidate marks | Dioxus | has |
| one bar (the space's strip merged into the command bar) | asked 19 Oct | has (19 Oct) |
| arrows hop to the nearest text | Dioxus only | has |
| spatial saves: blocks in the room showing a text, a picture or a colour, kept in the save (since 20 Oct as placements in the room's scene, one packed value per texture and chunk of 64; an older save's `graph blocks` move there when it opens), in its backups, bundles and merges | asked 19 Oct | has (19 Oct, desktop and web; on scenes 20 Oct, checked on the web with a save made by the previous build: its blocks and saved scene came across, copy, delete and undo work): the blocks window (cube chip) makes them from the selected text, a picture or a colour; copy, delete, x y z w h d fields; Ctrl+Z undoes moves, resizes, adds and deletes; `blocks.*` in docs/app-api.md |
| blocks turn and bend (#5, #21; one schema change, 26 Oct): the gizmo has a ring about each axis to turn a block (Shift: 15° steps), its knobs resize along the block's own turned axes, and the blocks window has quarter turns and "no turn". A block bends in two ways, added together: a curtain ripple (a wave across its width; how much, how many waves, shifted by; one period's shape a bezier whose two control points are dragged on a drawn curve) and a fold about a vertical line (how much, where the line is; the shape from the line to the edges a bezier dragged the same way); "straighten" undoes both. A right-click on a block gives its menu: bend it…, straighten, a quarter turn about the upright, no turn, copy, delete. The turn is kept in the placement's packed rotation (none for no turn) and the bend in its extra bytes (none when straight); no migration was needed (older saves read as not turned and straight; the user starts a new save anyway). The bend is applied to the block's geometry (cut into slices across its width), so its texture follows it and picking and ray hits use the bent shape exactly; blocks don't collide with anything yet. Checked on the desktop under Xvfb (menu, bend editor, sliders, quarter turn; the texture following the bend) and the web (a bend from sliders and a dragged control point, a ring turn of 37°, the menu, the stored values, a view from above) | asked 25 Oct | has |
| texture recipes and the sky (#7, #23; 26 Oct): a textures window (the image chip in the space's strip, a block's menu "texture it…", the blocks window's "texture…") builds a recipe as numbered steps from source to use, each with a preview the room draws itself: 1 source (a picture of the save, the selected text drawn as a text block draws it, or a colour), 2 crop (drag a box over the source, "all of it"), 3 tile (width and height in units on log sliders, offset across and down, quarter turns, mirror, a tint with its strength, "keep the crop's shape"), 4 variants (1–16, shuffle, turned too; a block shows the variant its number picks), 5 use (keep, a new block, on the selected block, on one of its six faces or clear the faces, as the sky or no sky), with a name. Saved recipes are listed at the top to open again or delete (two clicks). Recipes are kept by the hash of their normalized JSON in `graph textures` (equal recipes are one, shared by every block and face); blocks show one as kind `texture`, faces in the placement's extra; the sky is the save's `space.sky` setting, drawn as a sphere round the camera. Backups carry the recipes; merges bring them in (a text source finds its text in the target). Generated pictures come in as pictures. Checked on the web (Playwright: a crop of a bricks picture with four variants on a new block, a blue colour on its top face, a starry picture as the sky; all back after a reload) and on the desktop under Xvfb (the same save: the block, its face and the sky; the window with a saved recipe opened, its crop shown) | queued requests #7, #23 | has: app/docs/shots/textures-window.png, textures-sky.png, web-textures-window.png, web-textures-room.png |
| sphere blocks (#21, 26 Oct): a block is a box or a sphere (an ellipsoid with its sides as diameters): the blocks window's "shape" row and the block's menu ("make it a sphere" / "make it a box"). Kept in the placement's shape flag (`place::Kind::Sphere`, 5). Its texture tiles once per unit round its middle and from pole to pole; it doesn't bend and has no faces of its own (a bend or faces set on a box are dropped when it becomes a sphere); picking hits the sphere itself, regions use its box. Checked on the web (a brick recipe on a sphere) and the desktop under Xvfb (sphere and back to a box from the menu) | queued requests #21 | has: app/docs/shots/web-sphere.png, sphere-menu.png |
| lights baked onto blocks (#20, 26 Oct): a lights window (the sun chip) adds an ambient light, a sun (directional, the way you look but downward), a point or a cone ahead of you; picking one sets its colour, strength, reach (log slider), direction (round and up/down sliders, or "the way I look"), cone angle and soft edge, or moves it to where you are; delete takes two clicks. Changes show as the slider moves and are kept when it's let go. Lights live in `graph lights` (at most 64), travel in backups and merges, and are baked onto the blocks' vertices (faces cut into cells about a unit across), redone for the blocks a light reaches when it changes and for a block when it changes; no shadows; texts aren't lit; with no lights faces are shaded by direction as before. Point and cone lights show as dots (a cone with its axis) while the window is open. Nothing moves yet, so there are no live lights. Checked on the web (ambient, a warm point and a blue cone over a floor, a box and two spheres; back after a reload) and the desktop under Xvfb (the same save, a strength slider dragged) | queued requests #20 | has: app/docs/shots/web-lights.png, lights.png |
| staging zones (#14, 26 Oct): a block can be a staging zone (the blocks window's "a staging zone"): the texts inside it spread among themselves, pulled to its centre and kept inside its box (turned with it), under its own push apart, pull together, spacing, gravity and damping (each slider starts at the room's value; "as the room" lets it follow the room again). Texts inside and outside don't push or pull each other; zones nest (a text belongs to the smallest zone it's in); moving a zone carries its texts along; dragging a text in or out changes its zone. A zone is drawn see-through. Kept in the block's extra (`Z` record). Checked on the web (a zone over two of five anagrams, moved six units with them; its window) and the desktop under Xvfb (a text dragged into the zone) | queued requests #14 | has: app/docs/shots/web-zone.png, web-zone-window.png, zone.png |
| keyframe animations (#16, 26 Oct): an animations window (the play chip) lists the save's animations and edits one: its name, length, how it repeats (once, in a loop, back and forth), a timing curve whose two control points are dragged (CSS's named curves one click away), and a timeline of keys with a scrubber that shows any moment on the selected block. "a key here" adds a key at the scrubber; a picked key's move, turn and scale are sliders (each set or carried), its texture a recipe or a colour from that moment, and it can have its own timing to the next key. "play" previews it on the block, "keep" saves it, and the selected block can play it with a delay and a speed, or stop. Animations are entries (mime 9, left out of search and the counts), shared by every block playing one; a block refers to it from its placement. Playing blocks are posed every frame (the block's place in the save and its gizmo stay where it was put; light isn't re-baked as it moves). Checked on the web (a hop and half turn eased in and out: half way the block is 0.5 up and turned 90°; kept, played, still playing after a reload) and the desktop under Xvfb (the same save, playing, the window opened) | queued requests #16 | has: app/docs/shots/anims.png, web-anim-scrub.png, web-anim-window.png |
| blocks that cast rays and run programs (#15, #28, 26 Oct): a wiring window (the bolt chip) wires the selected block: the events it listens for (a click, another block's ray, a text coming into or leaving its zone, a timer every so many seconds, a signal by name), the s program it runs then (a note tagged `s`, by title; none: it only casts its ray), and its ray: fixed (out of its front, back, sides, top or bottom, as the block is turned; registering the first, last, nth or all of the blocks it crosses within its reach, of the kinds it sees, optionally sticking to what it first hits) or sticky (to target blocks picked in the room). "play" makes the room live: a click on a wired block fires it instead of selecting it, timers run, zones tell of texts coming and going; "try it" fires the block once. Core runs each cause and what it sets off in order (`wiring.fire`): the program gets the event, who caused it and its block as arguments and its kept state as stdin, and its printed lines `move`, `place`, `turn`, `size`, `colour`, `texture`, `play`, `stop`, `signal`, `cast`, `keep` and `say` act; blocks its ray registers get a hit. At most 200 events a cause, in one budget of steps. Rays show for a moment; the window lists what the last cause set off. Wirings are entries (mime 10), shared by copies until one changes. Checked on the web (a click casts through the grey block to the blue one, which a program lifts each time; a counter kept its count; a 0.5 s timer fired while playing and stopped with it) and on the desktop under Xvfb (the same save, playing, the timer and the ray). Not yet: texts as targets, rays cast from where an animation shows a block, Ctrl+Z for what a cause changed; zone enter/leave is wired in the room but wasn't tried by hand | queued requests #15, #28 | has: app/docs/shots/web-wiring-ray.png, web-wiring-lifted.png, desktop-wiring.png; guard `parity::tests::wiring_stays` |
| prefabs: subassemblies and the prefab menu (#19, 27 Oct): a prefabs window (the stacked-blocks chip) groups blocks picked in the room into a prefab (a scene of its own; one copy stands where they were). Their textures, turns, bends, animations, wirings and the links between them go with it: a sticky ray aimed inside the group names the part's address inside the prefab, so each copy's ray reaches its own part, and a block outside that aimed at a grouped block aims at that part of the first copy. The menu shows each prefab with a picture of it (drawn by the room), its blocks and copies; "place" puts a copy ahead of you, a double click renames, delete takes its copies too (two clicks). A copy is one placement (a place, a turn and a size); its parts show as blocks. Selecting a part shows its copy: move it by steps, a quarter turn either way, size ×1.25 or ÷1.25, detach it into ordinary blocks, remove it. A part moved, resized or retextured by hand, or by a program (and a part's wiring state), changes in its copy only, as an override that shadows the prefab; the copy can take them back or make them the prefab's, every copy following or the others keeping it as it was (`<name> (before)`). Merges bring prefabs and copies; backups keep them. Checked on the web (grouping a switch and its lamp, a second copy moved and turned, playing: the second switch lifted the second lamp only) and on the desktop under Xvfb (the menu, a copy's controls, "…but not the other copies" making `switch and lamp (before)`). Not yet: prefabs inside prefabs, adding or taking parts out of a prefab after it's made, Ctrl+Z for prefab changes, sharing a prefab from one save to another except by merging, placing in first-person mode (its own line) | queued requests #19 | has: app/docs/shots/web-prefabs.png, web-prefabs-play.png, desktop-prefabs.png, desktop-prefabs-before.png; guard `parity::tests::prefabs_stay` |
| merging settles overlaps (answer 5, 26 Oct): a saved scene can be combined into another from the scenes window ("combine into…"): its placements are copied in, and those landing on what's there (or on each other) are pushed out of the way, deterministically, while what was there stays put; the scene combined stays as it was. Merging another save's room brings its blocks in settled the same way. Texts settle sideways or up and down (labels face the viewer), blocks by their turned and bent boxes. Checked on the desktop under Xvfb (combining two saved scenes) and the web (two copies landing on a text settle above it) | asked 19 Oct (answer 5) | has |
| a translate / scale gizmo on the selected block, the texture tiling with its size (UVs per unit, repeat wrapping) rather than stretching | asked 19 Oct | has (19 Oct): arrows move along an axis, the face knobs resize (the far face stays), the ball moves across the view, Shift snaps to quarter units; guard `parity::tests::blocks_tile_and_have_a_gizmo` |

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
| tags with counts, Ctrl/Shift+click multi-pick, all/any, the notes carrying them, open in the editor | tags_ui.rs | has |
| stats: notes, words, saves/edits, first and latest note, tags with counts opening the tags view | ui.rs `rebuild_stats` | has (19 Oct: a tag opens the tags view with it picked) |

## Classifiers (the models view)

| feature | was in | status |
|---|---|---|
| list, new from lists (classes, not-list, builds on, size, one class / several) | classify_ui.rs | has |
| starter examples (fruit, stones, animals) | classify_ui.rs | has (20 Oct): the starters panel (the models view's "starters" button, and the empty state) makes fruit, stones and animals and seven more (number, gematria, English, spelling, sentence, authors, languages), each with its source and licence; a classifier made from one shows its held-out score (`classify.starter*`) |
| training: epochs, rounds, from scratch, progress, loss line, cancel, metrics, confusion, growth | classify_ui.rs | has (growth history back 19 Oct); the page has the Bevy tabs again: learn, use, buckets, checks |
| use: try a text, yes/no per class, "why" (each classifier's part, end to end) | classify_ui.rs, `classify.contributions` | has |
| steering: bias per class, temperature, threshold, channel gains, the bias it adds (flipped, KL) | classify_ui.rs | has |
| buckets: every text filed by class and sureness (tenths), index again | classify_ui.rs, `classify.buckets` | has (19 Oct): views/clsmore.rs |
| near texts by meaning (embedding) or verdict | classify_ui.rs, `classify.near` | has |
| outputs as a list (write the list) | classify_ui.rs, `classify.enlist` | has |
| checks: validators, disagreements | cls_more.rs | has |
| share / take classifiers on the hub | cls_more.rs, JS clsmore.js | has (in the settings' hub section) |
| export / import `.anendcls` | classify_ui.rs | has |
| colour views by a classifier | classify_ui.rs | has (picked in the memory strip) |
| device: CPU / GPU / GPU fast, and which device a job ran on | asked 19 Oct | has (classifier and spec training; falls back to the CPU with a note when there's no adapter or the hub keeps members on its CPU; desktop and `anend-hub` build with `gpu`, a shared hub needs `--gpu`) |
| classifiers read texts on the GPU too: predicting, classifier search, indexing, buckets, colouring by a classifier and filters by one, every text's windows packed into a few submissions; the settings' GPU section picks auto (the GPU when it is a real one, bit-exact, and measured faster than the CPU for the model's size, else the CPU), the CPU, the GPU or the GPU fast, and says where reading runs now and why when it fell back to the CPU | asked 20 Oct (checklist) | has (20 Oct, desktop and web): `classify.device`, `GpuNet::text_runs`; tested against the CPU on the container's software renderer (llvmpipe, bit-exact there); speed and behaviour on real GPU hardware not verified yet. 25 Oct: the user's GTX 1070 Ti read 0.12× to 0.38× the CPU's speed, so auto now measures each model size on first use (kept per machine in the cache folder), reads where it's faster, and the section lists the measured rates with a "measure again" button (checked on llvmpipe with `ANEND_AUTO_SOFTWARE_GPU=1`) |
| the GPU report: a settings button that writes a JSON file with the adapter and driver, its limits, the exactness probe of every kernel, and timings of a fixed training run and of reading 10 000 texts on the CPU and the GPU | asked 20 Oct (checklist) | has (20 Oct): `gpu.report`, `core/src/classify/report.rs`; to be run by the user on their machine |
| the gpu build is warning-free (no `recursion_depth_exceeding_limit` on the shared device) | asked 19 Oct (F8) | has (19 Oct): `Gpu` is Send + Sync by an explicit impl, each field checked on its own (`fields_are_send_and_sync`) |
| spec models (templates, JSON editor, train, run, write) | Dioxus only | has |
| the visual spec builder (#4, 26 Oct): a spec model's page and "new" show the spec two ways, the builder (the default) and the JSON, one text under both, so a change in either shows in the other. The builder has a control for every field (the model; what it reads with its channels, picture size, patches and codes, or named numbers and their width; the trunk's preset and sizes; heads with kind, classes as chips and weight; data rows picking the save's lists, tags, list pairs, picture lists, or examples written in, with a picture of the save turned into an inline picture; critics picking another model, its head and what it should say; length, contains, excludes, sums and gates; sampling; training), pickers from what the save has, sliders and steppers, add and remove for rows, a diagram of data → heads, input → trunk → heads and critics → the generate head, and a broken rule on the control it concerns (and at the top). A test walks specs using every part of the format and checks each field is in `COVERED` and written by a control. Constraint sums' measures now read and write as docs/models.md has them (`bytes`, `ordinal`, ...; the old `Bytes` names still read). Checked on the web (a classifier: a class taken out showed "classify needs at least two classes" on its chips, put back; a window size set, seen in the JSON; a preset changed in the JSON, seen in the builder; a writer with a critic, sums and a gate) and the desktop under Xvfb (new from a template in the builder, renamed and a channel ticked, seen in the JSON) | queued requests #4 | has: app/docs/shots/spec-builder.png, spec-builder-json.png, web-spec-builder.png, web-spec-builder-error.png, web-spec-builder-writer.png |

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
| saves folder setting (a folder lists its saves, a .redb path opens one) | settings.rs | has (26 Oct): the saves settings' "saves folder" row on the desktop (`saves.folder`; a hub refuses it and the web doesn't show it). A folder lists its saves and opens the first, or makes a `main`; a `.redb` file opens that save. Opening a save keeps its path in the prefs, so the desktop starts on it. Checked under Xvfb (a new folder got a `main`, then a `.redb` path back to the first); app/docs/shots/saves-folder.png |
| save checkpoints: named redb savepoints of the open save, listed under it in the saves settings, gone back to (two clicks; later ones go) and deleted | next.md backlog | has (19 Oct): `graph::checkpoints`, `saves.checkpoint(s)`; checked under Xvfb (a checkpoint, a new text, back: 51 entries to 50, the later checkpoint dropped) |
| input methods (IME): composing in the bar and the notes pad; Enter while composing picks the candidate and does nothing else | asked (brief) | has on the web (19 Oct): `scripts/ime-check.mjs` drives Chromium's IME path (CDP `imeSetComposition`, Enter with key code 229, `insertText`): にほん → 日本 in the memory bar, the composing Enter searched nothing, a real Enter then remembered it; 漢字 composed into the pad. The desktop (WebKitGTK with IBus/Fcitx) wasn't tried: this container has no input method; the bar's guard (`isComposing` / 229) is the same code |
| **must keep:** a still space isn't drawn again every frame (nothing flying, turning, easing, dragging or spreading), so typing elsewhere while the space is open keeps every key; it lost keys typed quickly in the desktop's web view before (19 Oct; checked under Xvfb: 25 characters at 40 ms each, all there) | — | has |
| export / import a bundle (.anend.json) | JS settings.js | has (19 Oct) |
| the hub: connect with an app token, browse and download shared saves, share this one | hub_ui.rs | has (19 Oct): views/hubui.rs + hubnet.rs (ureq on the desktop, fetch on the web); the web shares saves from the desktop only (no same-origin route for it) |
| local-only browser saves (IndexedDB) | JS local/ | dropped: the web app runs against the hub's app mode (`anend-hub --mode app --local` for one person) |

## New since the Bevy app (keep)

The Dioxus app adds: the undo stack and button, the right-click menu with kinds, the memory
view's search filter, bucket strip, field width and selection, pictures, spec models,
replacements and alternatives in the editor, the reader's pages and big reading, zoom.

## Not yet (planned)

| feature | status |
|---|---|
| WebGPU training in the browser (task #24) | not yet: goes with the GPU line on the back burner (the user, 25 Oct: CPU training is fine for now) |
