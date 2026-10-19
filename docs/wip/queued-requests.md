# Queued requests (for after the current parity round)

The user asked for these to be held until the current round is done, then picked up in order.

## 19 Oct, afternoon UTC

1. **Bottom bar width:** the typing bar has a minimum width of 15.5 cm. Use CSS `min-width: 15.5cm`
   and check its real size at 96 dpi and at other scale factors on the desktop and on the web.
2. **Everything else beside the typing bar, overflow above it:** the bar's other items (hints,
   counts, chips, the view's figures) sit beside the typing bar on the same line while they
   fit next to its 15.5 cm minimum. Only the items that don't fit move into a transparent
   bar just above it, so no space is wasted. Wide windows get one line; narrow windows or a
   crowded view get the overflow above. The typing bar keeps at least 15.5 cm, and its text
   never shifts as items move between the two places.
3. **Space view, after Enter:** pressing Enter in the bar blurs it, so the keys move again
   (moving mode). Typing starts only after the bar is focused again by a click or the focus
   key. Keep every bar action that Enter triggers.

Keep everything that exists: record each of these in docs/parity.md as must-keep, and check the
views touched against their previous behaviour before committing.
