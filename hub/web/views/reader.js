// the reader: every note, newest first, the way the old front page listed writs;
// click a #tag to keep to it

import { el, clear, debounce, longDate, relative } from '../util.js';
import { render } from '../markdown.js';
import { icon, tip } from '../icons.js';

let app, list, count;
let q = '';
let tag = '';
const full = new Map(); // key → note (with its markdown), until a save changes things
const PAGE = 40;
let shown = PAGE;

async function draw() {
  let notes;
  try {
    notes = await app.call('notes.list', q.trim() ? { q } : {});
  } catch {
    return;
  }
  if (tag) notes = notes.filter((n) => n.tags.includes(tag));
  count.textContent = `${notes.length} note${notes.length === 1 ? '' : 's'}`;
  const page = notes.slice(0, shown);
  // the whole of each note, fetched together
  await Promise.all(
    page.map(async (n) => {
      const have = full.get(n.key);
      if (have && have.modified === n.modified) return;
      try {
        const got = await app.api.call('notes.get', { key: n.key });
        full.set(n.key, { ...got, modified: n.modified });
      } catch {
        /* gone meanwhile */
      }
    }),
  );
  const cards = page.map((s) => {
    const n = full.get(s.key) || { title: s.title, markdown: s.preview, tags: s.tags, created: s.created, edits: [] };
    const md = el('div.md');
    md.innerHTML = render(n.markdown);
    const edited = n.edits && n.edits.length ? n.edits[n.edits.length - 1] : '';
    return el(
      'article.card.writ',
      el('header', el('h2', n.title || 'untitled'), tip(el('a.edit.btn.small.ibtn', { href: `#/editor/${encodeURIComponent(s.key)}` }, icon('edit')), `edit “${n.title || 'untitled'}”`)),
      el('div.date', longDate(n.created), edited ? el('span', ` · edited ${relative(edited)}`) : null),
      n.tags.length
        ? el(
            'div.tags',
            n.tags.map((t) => el('button.chip', { type: 'button', className: t === tag ? 'chip on' : 'chip', onclick: () => pick(t) }, `#${t}`)),
          )
        : null,
      md,
    );
  });
  clear(
    list,
    tag ? el('div.filter', el('span', 'notes tagged '), el('button.chip.on', { type: 'button', title: 'show every note', onclick: () => pick('') }, `#${tag} ×`)) : null,
    cards.length ? cards : el('div.card.dim', q || tag ? 'no notes match' : 'no notes yet: write one in the editor'),
    notes.length > shown ? el('button.more', { type: 'button', onclick: () => ((shown += PAGE), draw()) }, `${notes.length - shown} more`) : null,
  );
}

function pick(t) {
  tag = tag === t ? '' : t;
  shown = PAGE;
  history.replaceState(null, '', tag ? `#/reader/tag/${encodeURIComponent(tag)}` : '#/reader');
  draw();
  list.parentElement.scrollTop = 0;
}

export default {
  barName: 'notes',
  barPlaceholder: 'find notes',
  init(section, a) {
    app = a;
    count = el('span.count');
    this.barRight = count;
    list = el('div.reader');
    section.append(list);
    app.on('save', () => full.clear());
  },
  show(rest) {
    if (rest[0] === 'tag') tag = rest[1] || '';
    draw();
  },
  barInput: debounce((text) => {
    q = text;
    shown = PAGE;
    draw();
  }, 150),
  barKey(e) {
    if (e.key === 'Enter') {
      const first = list.querySelector('a.edit');
      if (first) location.hash = first.getAttribute('href');
      return true;
    }
  },
};
