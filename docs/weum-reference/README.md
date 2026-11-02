# weum reference

weum's own code, shared by its author as the canonical reference for how anend's memory
view, gematria and storage must behave: `main.rs` (the Salvo server and its redb memory
tables), `api.js` (the client helpers: `gsum`, `pfR`, `memory.*`), and `memory.js` (the
memory page). Read these, not a reconstruction, when matching weum's behaviour.

## What they settle

- **Underline colours** (`baroeskwidjel` in memory.js):
  - **Grouping:** every result is grouped by its *opposite* sum. With reversed off, that's
    the reversed sum (`gsum(text, 3)[1]`); with reversed on, it's the ordinal sum.
  - **Colour:** the underline is `seedCLR(that sum)`, so texts sharing an opposite sum share
    a colour.
  - **Who gets underlined:** the check is `m.length >= 2` on the *key string*, so in
    practice every text whose opposite sum has two or more digits is underlined, even when
    alone in its group.
  - **Atbash and frequency:** new modes in anend, per the user's rules in docs/next.md (B).
- **Ctrl + right click** on a text: `envale(floor(sqrt(ordinal × reversed)))`, which
  gsearches the triangle number ⌊√(s·r)⌋ in the current mode (reversed if that's checked).
- **Clicks on a text:**
  - click: the text goes into the bar, and the sidecar shows its sums;
  - Ctrl+click: it becomes the list candidate;
  - Shift+click: it becomes the list;
  - double click: its key goes into the bar;
  - middle click: speak it (`enwave`);
  - hover title: `key` plus `gsum(text, 1)`.
- **The bar:**
  - **Enter:**
    - a number gsearches it;
    - text gsearches its sum (with gsearch on) and searches its words;
    - Ctrl+Enter gsearches the text's sum, saturates it, and toasts its key;
    - Shift+Enter remembers by key.
  - **Delete:** forgets the key typed.
  - **List commands:** `Enlist`, `List`, `Delist`, `Unlist`.
- **Keys outside the bar:**
  - `r` toggles reversed;
  - ArrowLeft / ArrowRight gsearch the sidecar's first or last value.
- **Results header:** the sum, then `pfR(sum)`.
- **Toasts:** they last 15 s; click one to put it in the bar, double click to dismiss.
- **Storage** (main.rs):
  - memories by key length, in tables `memories 1B..4B`;
  - `memg` and `memg reversed` (sum → keys);
  - `lists`, `notes` and `noted on` multimaps (notes are memories noted on memories);
  - `reusables` (forgotten keys);
  - commented-out `prefixed:` / `sufixed:` dedup ideas;
  - `R/G/B/RGB` multimaps (space colours);
  - `backup_memories_to_file`.
- **`gematria_dual_sum`:** the SIMD ordinal and reversed sum (main.rs ~1500).

The shop, orders and accounts parts of main.rs aren't part of anend.
