// the space's view: the room, its terminal (the command bar here), the position line,
// the metrics card a hovered text shows, and the search line in the top bar

import { el, clear, typing, textOfOrdinal, KEY } from '../util.js';
import { Space, triangle } from '../space.js';
import { pick, listCommand, onPick, pickLine } from '../picks.js';
import { colours } from '../theme.js';
import { icon, iconButton, tip } from '../icons.js';

let app, section, host, engine, pos, card, help, helpBtn, empty, searchline, chips;
const st = { gsearch: true, reversed: false, saturate: true, keep: false };
let texts = []; // [{id, text, hit}] in the spread now
let term = null; // the last search: {n, bucket, pf, s, r, t, buckets}
let history = [];
let at = -1;
let hideTimer;

function loadPrefs() {
  try {
    Object.assign(st, JSON.parse(localStorage.getItem('anend.space') || '{}'));
  } catch {
    /* private window */
  }
}
function savePrefs() {
  try {
    localStorage.setItem('anend.space', JSON.stringify(st));
  } catch {
    /* private window */
  }
}

const measures = new Map();
async function measure(text) {
  if (!measures.has(text)) {
    if (measures.size > 2000) measures.clear();
    measures.set(text, app.api.call('mem.measure', { text }));
  }
  try {
    return await measures.get(text);
  } catch (e) {
    measures.delete(text);
    throw e;
  }
}

function drawChips() {
  const t = (k, label, title) =>
    el('button.toggle', { type: 'button', className: `toggle${st[k] ? ' on' : ''}`, title, onclick: () => flip(k) }, label);
  clear(
    chips,
    t('gsearch', 'gsearch', 'text gsearches its sum (off: searches its words)'),
    t('reversed', 'reversed', 'buckets by the reversed sum (F)'),
    t('saturate', 'saturate', 'searched texts are saved as memories'),
    t('keep', 'keep', 'new results join the spread instead of replacing it'),
  );
}

function flip(k) {
  st[k] = !st[k];
  savePrefs();
  drawChips();
  if (k === 'reversed' && texts.length) spread(texts, true);
}

// ---------------------------------------------------------------- searching

async function spread(list, again = false) {
  if (!list.length) {
    texts = [];
    engine.clear();
    drawLine();
    return;
  }
  try {
    const sp = await app.call('space.spread', { texts: list.map((t) => ({ id: t.id, text: t.text })), reversed: st.reversed });
    texts = list;
    engine.place(list, sp, again);
    if (term) term.buckets = sp.buckets;
    drawLine();
    engine.info();
  } catch {
    /* reported */
  }
}

// new hits: in place of the spread, or (keep) added to it
function show(hits) {
  const fresh = hits.map((h) => ({ id: h.key, text: h.text, hit: h }));
  if (st.keep && texts.length) {
    const have = new Set(texts.map((t) => t.id));
    spread([...texts, ...fresh.filter((t) => !have.has(t.id))], true);
  } else spread(fresh);
}

async function gsearch(n, bucket, m) {
  try {
    const r = await app.call('mem.gsearch', { bucket, key: n });
    const pfm = await measure(textOfOrdinal(n));
    // the triangle: the term's own s and r, or (a number) its bucket's first text's
    let tm = m;
    if (!tm && r.hits.length) tm = await measure(r.hits[0].text);
    term = { n, bucket, pf: pfm.pf_ordinal, s: tm?.s ?? 0, r: tm?.r ?? 0, t: tm?.triangle ?? 0, count: r.hits.length, buckets: 0 };
    show(r.hits);
    if (!r.hits.length) app.status(`nothing in ${bucket} ${n} yet`);
  } catch {
    /* reported */
  }
}

async function enter(e) {
  const t = app.bar.get().trim();
  if (!t) return;
  history = [t, ...history.filter((x) => x !== t)].slice(0, 100);
  at = -1;
  const listed = await listCommand(app, t);
  if (listed) {
    if (listed.length) {
      term = null;
      show(listed);
    }
    return;
  }
  if (t === 'Clear') {
    term = null;
    spread([]);
    return;
  }
  if (t === 'Count') {
    app.status(`${texts.length} texts in the space · ${app.counts.memories} memories`);
    return;
  }
  const bucket = st.reversed ? 'reversed' : 'ordinal';
  if (e.shiftKey) {
    try {
      const h = await app.call('mem.remember', { key: t });
      const keep = st.keep;
      st.keep = true;
      show([h]);
      st.keep = keep;
    } catch {
      /* reported */
    }
    return;
  }
  const num = t.match(/^#?(r?)(\d+)$/);
  if (num) return gsearch(+num[2], num[1] ? 'reversed' : bucket);
  if (!st.gsearch && !(e.ctrlKey || e.metaKey)) {
    try {
      const hits = await app.call('mem.search', { q: t });
      term = { n: t, bucket: 'words', pf: '', count: hits.length, buckets: 0, ...(await triOf(t)) };
      show(hits);
    } catch {
      /* reported */
    }
    return;
  }
  if (st.saturate || e.ctrlKey || e.metaKey) {
    try {
      const s = await app.call('mem.saturate', { text: t });
      if (s.new) {
        app.counts.memories++;
        app.status(`remembered “${t}” under ${s.key}`);
      }
    } catch {
      return;
    }
  }
  const m = await measure(t);
  return gsearch(st.reversed ? m.r : m.s, bucket, m);
}

async function triOf(t) {
  const m = await measure(t);
  return { s: m.s, r: m.r, t: m.triangle };
}

// ---------------------------------------------------------------- the search line

function drawLine() {
  if (empty) empty.hidden = texts.length > 0;
  searchline.hidden = app.view !== 'space' || !app.prefs.searchLine || !term;
  if (!term) {
    clear(searchline);
    return;
  }
  clear(
    searchline,
    el('b', String(term.n)),
    ` ${term.bucket} · ${texts.length} text${texts.length === 1 ? '' : 's'} · `,
    tip(el('span.buckets', { tabindex: '0' }, icon('bucket'), String(term.buckets)), `${term.buckets} bucket${term.buckets === 1 ? '' : 's'}: the spread's rings, by the ${st.reversed ? 'ordinal' : 'reversed'} sum`),
    term.pf ? el('span.pf', ` (${term.pf})`) : null,
    term.s && term.r ? triangle(term.s, term.r, term.t) : null,
  );
}

// ---------------------------------------------------------------- the metrics card

async function hover(it, e) {
  clearTimeout(hideTimer);
  let m;
  try {
    m = await measure(it.hit.text);
  } catch {
    return;
  }
  const n = (v) => el('span.num', { title: `gsearch ${v}`, onclick: () => (app.bar.set(String(v)), gsearch(v, st.reversed ? 'reversed' : 'ordinal')) }, String(v));
  const { s, r } = m;
  clear(
    card,
    el('div.title', it.hit.text),
    el(
      'table',
      el('tr', el('th', 's'), el('td', n(s)), el('th', 'r'), el('td', n(r))),
      el('tr', el('th', 's+r'), el('td', n(s + r)), el('th', '|s−r|'), el('td', n(Math.abs(s - r)))),
      el('tr', el('th', 's·r'), el('td', n(s * r)), el('th', '⌊√s·r⌋'), el('td', n(m.triangle))),
      el('tr', el('th', 'atbash'), el('td', n(m.atbash)), el('th', 'letters'), el('td', m.frequency.replace(/(\D)(\d+)/g, (_, c, k) => c.repeat(+k)))),
    ),
    el('div.gsum', m.gsum),
    el('div.dim', `${it.hit.key} · bucket `, el('span.swatch', { style: { background: `hsl(${it.node?.hue ?? 0} 70% 45%)` } }), ` ${st.reversed ? 'ordinal' : 'reversed'} ${it.node?.q ?? ''}`),
    pickLine() ? el('div.dim', pickLine()) : null,
  );
  card.hidden = false;
  const box = section.getBoundingClientRect();
  const lr = it.el.getBoundingClientRect();
  let x = lr.right - box.left + 12;
  let y = lr.top - box.top;
  const w = card.offsetWidth;
  const h = card.offsetHeight;
  if (x + w > box.width - 8) x = Math.max(8, lr.left - box.left - w - 12);
  y = Math.max(8, Math.min(y, box.height - h - 8));
  card.style.transform = `translate(${x}px, ${y}px)`;
}

function leave() {
  clearTimeout(hideTimer);
  hideTimer = setTimeout(() => (card.hidden = true), 350);
}

function click(it, e) {
  if (e.ctrlKey || e.metaKey) {
    pick('candidate', it.hit);
    app.status(`candidate: ${it.hit.text}`);
  } else if (e.shiftKey) {
    pick('list', it.hit);
    app.status(`list: ${it.hit.text}`);
  } else {
    app.bar.set(it.hit.text);
    app.bar.focus();
  }
}

function drawPicks() {
  for (const it of engine.items.values()) {
    it.el.classList.toggle('is-list', !!(pickLine() && it.hit.key === listKey()));
    it.el.classList.toggle('is-candidate', it.hit.key === candKey());
  }
}
function toggleHelp() {
  help.hidden = !help.hidden;
  helpBtn.classList.toggle('on', !help.hidden);
}

let listKey = () => null;
let candKey = () => null;

// ---------------------------------------------------------------- the view

export default {
  barName: 'space',
  barPlaceholder: 'a number, a text, or a key (Esc to fly)',

  searchLineOn() {
    return app.prefs.searchLine && !!term;
  },

  init(s, a) {
    app = a;
    section = s;
    loadPrefs();
    host = el('div.space-host');
    pos = el('div.poscard');
    card = el('div.metrics', { hidden: true, onmouseenter: () => clearTimeout(hideTimer), onmouseleave: leave });
    help = el(
      'div.space-help',
      { hidden: true },
      el('b', 'flying'),
      ' right mouse drag looks · W A S D move · Space / C up and down · Q / E roll · Shift fast · [ ] or the wheel speed · R level · Home back',
      el('br'),
      el('b', 'texts'),
      ' hover for its numbers · click puts it in the terminal · Ctrl+click list candidate · Shift+click the list',
      el('br'),
      el('b', 'terminal'),
      ' a number gsearches that bucket · text gsearches its sum · Shift+Enter a key · Delete forgets a key · ↑ ↓ history · Enlist Delist List Unlist Clear Count',
    );
    helpBtn = iconButton('help', 'how to fly and search', { key: 'H', cls: 'plain.space-helpbtn', onclick: () => toggleHelp() });
    empty = el(
      'div.space-empty',
      el('p', 'The room is empty.'),
      el('p.dim', 'Type a word, a number or a key in the bar below and press Enter: its bucket spreads out here. Right-drag to look around, W A S D to fly.'),
    );
    section.append(host, pos, card, help, empty, helpBtn);
    searchline = document.getElementById('searchline');
    chips = el('span.toggles');
    this.barRight = chips;
    drawChips();
    engine = new Space(host, {
      hover,
      leave,
      click,
      info: (i) => {
        pos.textContent = `${i.x.toFixed(1)} ${i.y.toFixed(1)} ${(-i.z).toFixed(1)}   speed ${i.speed.toFixed(1)}   roll ${Math.round(i.roll)}°   ${i.texts} texts`;
      },
    });
    engine.info();
    // the room's colour comes with the palette
    const room = () => engine.setRoom(colours()['space room']);
    room();
    window.addEventListener('colours', room);
    onPick((p) => {
      listKey = () => p.list?.key;
      candKey = () => p.candidate?.key;
      drawPicks();
    });
    app.on('save', () => {
      texts = [];
      term = null;
      measures.clear();
      engine.clear();
      drawLine();
    });
  },

  show() {
    engine.start();
    drawLine();
  },

  hide() {
    engine.stop();
    card.hidden = true;
    searchline.hidden = true;
  },

  redrawLine() {
    drawLine();
  },

  barKey(e) {
    if (e.key === 'Enter') {
      enter(e);
      return true;
    }
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      if (!history.length) return true;
      at = Math.max(-1, Math.min(history.length - 1, at + (e.key === 'ArrowUp' ? 1 : -1)));
      app.bar.set(at < 0 ? '' : history[at]);
      return true;
    }
    if (e.key === 'Delete') {
      const input = e.target;
      const t = input.value.trim();
      if (KEY.test(t) && !t.startsWith('note:') && input.selectionStart === input.value.length) {
        app
          .call('mem.forget', { key: t })
          .then((r) => {
            app.status(`forgot “${r.text}” (${t})`);
            app.counts.memories = Math.max(0, app.counts.memories - 1);
            spread(texts.filter((x) => x.id !== t), true);
            app.bar.set('');
          })
          .catch(() => {});
        return true;
      }
    }
  },

  barEscape() {
    host.focus?.();
  },

  key(e) {
    if (typing(e) || e.ctrlKey || e.metaKey || e.altKey) return false;
    if (e.key === 'Enter') {
      app.bar.focus();
      return true;
    }
    if (e.key === 'h' || e.key === 'H') {
      toggleHelp();
      return true;
    }
    if (e.key === 'f' || e.key === 'F') {
      flip('reversed');
      return true;
    }
    return false;
  },
};
