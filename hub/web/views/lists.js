// the lists: every list (a memory as its head, memories as its members) as a card with
// its numbers and letters, and how alike every two lists' letters are (cosine)

import { el, clear, confirmClick } from '../util.js';
import { pick, picks } from '../picks.js';

const LETTERS = 'abcdefghijklmnopqrstuvwxyz';
let app, body, count;
// heads picked for new lists that have no members yet
const fresh = new Map(); // key → text

async function draw() {
  let st;
  try {
    st = await app.call('lists.stats');
  } catch {
    return;
  }
  for (const l of st.lists) fresh.delete(l.list);
  count.textContent = `${st.lists.length} list${st.lists.length === 1 ? '' : 's'}`;
  const newInput = el('input', { type: 'text', placeholder: 'a text to head a new list', 'aria-label': 'new list head' });
  const make = async () => {
    const t = newInput.value.trim();
    if (!t) return;
    try {
      const s = await app.call('mem.saturate', { text: t });
      fresh.set(s.key, t);
      app.status(`“${t}” (${s.key}) heads a new list: enlist something in it`);
      draw();
    } catch {
      /* reported */
    }
  };
  newInput.addEventListener('keydown', (e) => e.key === 'Enter' && make());
  const cards = st.lists.map((l) => listCard(l));
  for (const [key, text] of fresh) cards.push(listCard({ list: key, name: text, members: [], count: 0 }));
  clear(
    body,
    el(
      'div.card.lists-head',
      el('div.row', el('h2', 'lists'), el('span.dim.small', 'a list is a memory heading other memories; Shift+click a text in the memory view or the space to pick one')),
      el('div.row', newInput, el('button.primary', { type: 'button', onclick: make }, 'new list')),
    ),
    st.lists.length > 1 ? dotCard(st) : null,
    cards.length ? el('div.list-grid', cards) : el('div.card.dim', 'no lists yet'),
  );
}

// a list's 26 letters as a strip of bars
function strip(vector) {
  const max = Math.max(1e-9, ...vector);
  return el(
    'div.strip',
    { role: 'img', 'aria-label': 'letters: ' + vector.map((v, i) => (v > 0 ? LETTERS[i] : '')).join('') },
    vector.map((v, i) =>
      el(
        'span.col',
        { title: `${LETTERS[i]} ${v.toFixed(3)}` },
        el('span.bar', { style: { height: `${(v / max) * 100}%` } }),
        el('span.letter', LETTERS[i]),
      ),
    ),
  );
}

const letters = (f) => String(f || '').replace(/(\D)(\d+)/g, (_, c, n) => c.repeat(+n));
const r1 = (x) => (Math.round(x * 10) / 10).toLocaleString();

function listCard(l) {
  const add = el('input', { type: 'text', placeholder: 'enlist a text', 'aria-label': `enlist in ${l.name}` });
  const enlist = async () => {
    const t = add.value.trim();
    if (!t) return;
    try {
      const s = await app.call('mem.saturate', { text: t });
      if (s.key === l.list) {
        app.status("a list can't be in itself", true);
        return;
      }
      await app.call('lists.enlist', { list: l.list, member: s.key });
      app.status(`“${t}” is in “${l.name}”`);
      draw();
    } catch {
      /* reported */
    }
  };
  add.addEventListener('keydown', (e) => e.key === 'Enter' && enlist());
  const w = l.whole;
  const members = l.members.map((m) =>
    el(
      'span.member',
      { title: `${m.key} · ${m.ordinal} / ${m.reversed}` },
      m.text,
      el('button.x', {
        type: 'button',
        title: 'delist',
        'aria-label': `delist ${m.text}`,
        onclick: async () => {
          try {
            await app.call('lists.delist', { list: l.list, member: m.key });
            draw();
          } catch {
            /* reported */
          }
        },
      }, '×'),
    ),
  );
  const unlist = confirmClick(el('button.small', { type: 'button', title: 'take the whole list apart (the memories stay)' }, 'unlist'), 'sure?', async () => {
    if (!l.count) {
      fresh.delete(l.list);
      draw();
      return;
    }
    try {
      const r = await app.call('lists.unlist', { list: l.list });
      app.status(`“${l.name}” is no longer a list (${r.removed} members)`);
      draw();
    } catch {
      /* reported */
    }
  });
  const isPicked = picks.list && picks.list.key === l.list;
  return el(
    'article.card.list-card',
    el(
      'header',
      el('h3', l.name),
      el('span.dim.small', `${l.count} member${l.count === 1 ? '' : 's'} · ${l.list}`),
      el('span.grow'),
      el('button.small', {
        type: 'button',
        className: isPicked ? 'small on' : 'small',
        title: 'the list the memory view and the space act on (Enlist, Delist)',
        onclick: () => {
          pick('list', { key: l.list, text: l.name });
          draw();
        },
      }, isPicked ? 'picked' : 'pick'),
      unlist,
    ),
    w
      ? [
          strip(l.vector),
          el(
            'table.nums',
            el('tr', el('th', ''), el('th', 'sum'), el('th', 'mean'), el('th', 'whole')),
            el('tr', el('th', 'ordinal'), el('td', l.ordinal_sum), el('td', r1(l.ordinal_mean)), el('td', w.ordinal)),
            el('tr', el('th', 'reversed'), el('td', l.reversed_sum), el('td', r1(l.reversed_mean)), el('td', w.reversed)),
            el('tr', el('th', 'atbash'), el('td', ''), el('td', ''), el('td', w.atbash)),
          ),
          el('div.gsum', w.gsum),
          w.pf_ordinal ? el('div.pf', el('span.dim', 'pf ordinal '), w.pf_ordinal) : null,
          w.pf_reversed ? el('div.pf', el('span.dim', 'pf reversed '), w.pf_reversed) : null,
          el('div.letters.dim.small', { title: 'the letter frequency bucket' }, letters(w.frequency)),
        ]
      : el('p.dim.small', 'no members yet'),
    el('div.members', members),
    el('div.row', add, el('button', { type: 'button', onclick: enlist }, 'enlist')),
  );
}

// how alike every two lists' letters are, as a grid shaded by the cosine
function dotCard(st) {
  const names = st.lists.map((l) => l.name);
  const cell = (v, i, j) =>
    el(
      'td.cell',
      {
        title: `${names[i]} · ${names[j]}: ${v.toFixed(3)}`,
        style: { '--v': Math.max(0, Math.min(1, v)) },
        className: `cell${v > 0.6 ? ' strong' : ''}`,
      },
      v.toFixed(2),
    );
  return el(
    'div.card.dot-card',
    el('h3', 'alike letters ', el('span.dim.small', 'cosine of the lists’ letter vectors: 1 the same letters in the same proportions, 0 none shared')),
    el(
      'div.dot-scroll',
      el(
        'table.dot',
        el('tr', el('th', ''), names.map((n) => el('th.col-name', { title: n }, el('span', n)))),
        st.dot.map((row, i) => el('tr', el('th.row-name', { title: names[i] }, names[i]), row.map((v, j) => cell(v, i, j)))),
      ),
    ),
    el('div.scale', el('span', '0'), el('span.ramp'), el('span', '1')),
  );
}

export default {
  barName: 'lists',
  barPlaceholder: 'a text to head a new list (Enter)',
  init(section, a) {
    app = a;
    count = el('span.count');
    this.barRight = count;
    body = el('div.lists-view');
    section.append(body);
    app.on('save', () => fresh.clear());
  },
  show() {
    draw();
  },
  barKey(e) {
    if (e.key === 'Enter') {
      const t = app.bar.get().trim();
      if (!t) return true;
      app
        .call('mem.saturate', { text: t })
        .then((s) => {
          fresh.set(s.key, t);
          app.bar.set('');
          draw();
        })
        .catch(() => {});
      return true;
    }
  },
};
