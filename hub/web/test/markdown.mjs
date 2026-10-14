// the markdown renderer keeps a note's html off the page: every text is escaped and
// links only go to the web, mail or the page itself. Run with `node hub/web/test/markdown.mjs`.

import { render, safeUrl } from '../markdown.js';

let failed = 0;
function check(what, ok) {
  if (!ok) {
    failed++;
    console.log(`not ok: ${what}`);
  }
}

// links that must not run anything
for (const u of [
  'javascript:alert(1)',
  'JaVaScRiPt:alert(1)',
  '\x01javascript:alert(1)',
  '\x00javascript:alert(1)',
  ' javascript:alert(1)',
  'java\tscript:alert(1)',
  'java\nscript:alert(1)',
  'vbscript:x',
  'data:text/html,<script>alert(1)</script>',
  '\x7fjavascript:alert(1)',
]) {
  check(`safeUrl(${JSON.stringify(u)})`, safeUrl(u) === '#');
}
// links that go where they say
for (const u of ['https://example.com/a?b=c', 'http://x', 'mailto:a@b.c', '/guide', '#top', './x', '?q=1', 'notes/x']) {
  check(`safeUrl(${JSON.stringify(u)}) keeps it`, safeUrl(u) === u);
}

const html = [
  render('[click](\x01javascript:alert(1))'),
  render('![x](javascript:alert(1))'),
  render('<img src=x onerror=alert(1)> <script>alert(1)</script>'),
  render('[a"b](https://x.example/"onmouseover="alert(1))'),
  render('| a |\n|---|\n| <b>x</b> |'),
  render('```\n</code><script>x</script>\n```'),
  render('# <i>title</i>'),
].join('\n');
check('no javascript: href', !/href="[^"]*javascript:/i.test(html));
check('no tags from the text', !/<(script|img src=x|b>|i>)/i.test(html));
check('no attribute broken out of', !/"\s*onmouseover=/i.test(html));
check('links still render', render('[a](https://x.example)').includes('<a href="https://x.example" target="_blank" rel="noopener noreferrer">a</a>'));

if (failed) {
  console.log(`${failed} failed`);
  process.exit(1);
}
console.log('markdown: all safe');
