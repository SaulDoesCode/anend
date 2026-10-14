// stats: counts of notes, words, saves and tags (and, with the bar, the notes it finds)

import { el, clear, debounce, longDate } from '../util.js';

let app, body, count;
let q = '';

async function draw() {
  let s, found;
  try {
    [s, found] = await Promise.all([app.call('stats'), q.trim() ? app.call('notes.list', { q }) : Promise.resolve(null)]);
  } catch {
    return;
  }
  count.textContent = found ? `${found.length} note${found.length === 1 ? '' : 's'}` : `${s.notes} notes`;
  const stat = (n, label) => el('div.stat', el('b', n.toLocaleString()), el('span', label));
  const max = Math.max(1, ...s.tags.map((t) => t[1]));
  clear(
    body,
    el(
      'div.card',
      el('h2', 'this save'),
      el('div.stats', stat(s.notes, 'notes'), stat(s.words, 'words'), stat(s.saves, 'saves (each edit too)'), stat(s.memories, 'memories'), stat(s.tags.length, 'tags')),
      s.first ? el('p.dim.small', `first note ${longDate(s.first)} · latest ${longDate(s.latest)}`) : el('p.dim.small', 'no notes yet'),
    ),
    found
      ? el(
          'div.card',
          el('h2', `notes with “${q.trim()}”`),
          found.length
            ? el('ul.found', found.map((n) => el('li', el('a', { href: `#/editor/${encodeURIComponent(n.key)}` }, n.title), el('span.dim', ` · ${n.words} words`))))
            : el('p.dim', 'none'),
        )
      : null,
    el(
      'div.card',
      el('h2', 'tags'),
      s.tags.length
        ? el(
            'div.tagbars',
            s.tags.map(([t, n]) =>
              el(
                'a.tagbar',
                { href: `#/tags/${encodeURIComponent(t)}`, title: `${n} note${n === 1 ? '' : 's'}: see them in the tags view` },
                el('span.name', `#${t}`),
                el('span.track', el('span.fill', { style: { width: `${(n / max) * 100}%` } })),
                el('span.n', n),
              ),
            ),
          )
        : el('p.dim', 'no tags yet'),
    ),
  );
}

export default {
  barName: 'notes',
  barPlaceholder: 'find notes',
  init(section, a) {
    app = a;
    count = el('span.count');
    this.barRight = count;
    body = el('div.stats-view');
    section.append(body);
  },
  show() {
    draw();
  },
  barInput: debounce((t) => {
    q = t;
    draw();
  }, 150),
  barKey(e) {
    if (e.key === 'Enter') {
      const a = body.querySelector('ul.found a');
      if (a) location.hash = a.getAttribute('href');
      return true;
    }
  },
};
