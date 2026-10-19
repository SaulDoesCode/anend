# Queued requests (for after the current parity round)

The user asked for these to be held until the current round is done, then picked up in order.

## 19 Oct, afternoon UTC

1. **Bottom bar width:** the typing bar has a minimum width of 15.5 cm. Use CSS `min-width: 15.5cm`
   and check its real size at 96 dpi and at other scale factors on the desktop and on the web.
2. **A second bar above it:** everything else that shares the bar (hints, counts, chips, the view's
   figures) moves into a transparent bar above it. The typing bar holds only the text.
3. **Space view, after Enter:** pressing Enter in the bar blurs it, so the keys move again
   (moving mode). Typing starts only after the bar is focused again by a click or the focus
   key. Keep every bar action that Enter triggers.

Keep everything that exists: record each of these in docs/parity.md as must-keep, and check the
views touched against their previous behaviour before committing.
