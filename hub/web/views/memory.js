// the memory view: weum's memory.html. A black page, the results band in the middle and
// the term bar at the bottom, its sidecar of the term's four keys at the right.

import { el, clear, debounce, typing, underline, textOfOrdinal, KEY } from '../util.js';
import { picks, pick, pickLine, listCommand, onPick } from '../picks.js';

const BUCKETS = ['ordinal', 'reversed', 'atbash', 'frequency'];

let app, root, head, band, toast, sidecar, toggles;
const st = {
  bucket: 'ordinal',
  gsearchText: true,
  saturate: true,
  white: false,
  header: null, // {title, pf}
  results: [],
  measure: null, // the term's
};

function loadPrefs() {
  try {
    Object.assign(st, JSON.parse(localStorage.getItem('anend.memory') || '{}'));
  } catch {
    /* private window */
  }
}
function savePrefs() {
  try {
    const { bucket, gsearchText, saturate, white } = st;
    localStorage.setItem('anend.memory', JSON.stringify({ bucket, gsearchText, saturate, white }));
  } catch {
    /* private window */
  }
}

function drawHead() {
  const p = pickLine();
  clear(head, `${app.counts.memories} memories · ${st.bucket} bucket`, p ? el('span.picks', ` · ${p}`) : null);
}

// the prime factor chain with its numbers clickable
function pfLine(pf) {
  if (!pf) return null;
  const parts = pf.split(/(\d+)/);
  return el(
    'div.pf',
    '(',
    parts.map((x, i) => (i % 2 ? el('span.num', { title: `gsearch ${x}`, onclick: () => gsearch(+x) }, x) : x)),
    ')',
  );
}

function drawResults() {
  root.classList.toggle('white', st.white);
  if (!st.header && !st.results.length) {
    clear(band);
    band.hidden = true;
    return;
  }
  band.hidden = false;
  // which texts share a reversed sum with another one here
  const by = new Map();
  for (const h of st.results) by.set(h.reversed, (by.get(h.reversed) || 0) + 1);
  const arts = st.results.map((h) => {
    const shared = by.get(h.reversed) > 1;
    const a = el(
      'span.art',
      {
        title: h.key,
        className: [
          'art',
          picks.list && picks.list.key === h.key ? 'is-list' : '',
          picks.candidate && picks.candidate.key === h.key ? 'is-candidate' : '',
        ].join(' '),
        style: { borderBottomColor: shared ? underline(h.ordinal, h.reversed) : 'transparent' },
        onclick: (e) => {
          if (e.ctrlKey || e.metaKey) pick('candidate', h);
          else if (e.shiftKey) pick('list', h);
          else app.bar.set(h.text), app.bar.focus(), measureTerm(h.text);
        },
        ondblclick: () => app.bar.set(h.key),
        onauxclick: (e) => {
          if (e.button === 1 && h.note) app.go(`editor/${encodeURIComponent(h.key.replace(/^note:/, ''))}`);
        },
        onmouseenter: () => hover(h),
        onmouseleave: () => (toast.textContent = ''),
      },
      h.note ? `✎ ${h.text}` : h.text,
    );
    return a;
  });
  clear(
    band,
    st.header ? el('div.big', st.header.title) : null,
    st.header ? pfLine(st.header.pf) : null,
    el('div.arts', arts.length ? arts : el('span.none', 'nothing in this bucket yet')),
  );
}

const measures = new Map();
async function measureOf(text) {
  if (measures.has(text)) return measures.get(text);
  const m = await app.api.call('mem.measure', { text });
  if (measures.size > 2000) measures.clear();
  measures.set(text, m);
  return m;
}

async function hover(h) {
  try {
    const m = await measureOf(h.text);
    toast.textContent = `${h.key} · ${m.gsum}`;
  } catch {
    /* quiet */
  }
}

// the sidecar: the term's four keys, each one a gsearch
function drawSidecar() {
  const m = st.measure;
  if (!m) {
    clear(sidecar);
    return;
  }
  const n = (b, v, key = v) => el('span.num', { className: `num${st.bucket === b ? ' on' : ''}`, title: `gsearch ${b} ${v}`, onclick: () => gsearch(key, b) }, String(v));
  const term = app.bar.get().trim();
  clear(sidecar, '(', n('ordinal', m.ordinal), ', ', n('reversed', m.reversed), ', ', n('atbash', m.atbash), ', ', n('frequency', letters(m.frequency) || '·', term), ')');
}

const measureTerm = debounce(async (text) => {
  const t = text.trim();
  if (!t || /^\d+$/.test(t) || KEY.test(t)) {
    st.measure = null;
  } else {
    try {
      st.measure = await measureOf(t);
    } catch {
      st.measure = null;
    }
  }
  drawSidecar();
}, 120);

function drawToggles() {
  const t = (label, on, f, title) => el('button.toggle', { type: 'button', className: `toggle${on ? ' on' : ''}`, title, onclick: f }, label);
  clear(
    toggles,
    BUCKETS.map((b) => t(b, st.bucket === b, () => setBucket(b), `the ${b} bucket (${b[0].toUpperCase()})`)),
    el('span.gap'),
    t('gsearch text', st.gsearchText, () => flip('gsearchText'), 'Enter on text: gsearch its key, or a word search (G)'),
    t('saturate', st.saturate, () => flip('saturate'), 'save searched texts as memories (S)'),
    t('white', st.white, () => flip('white'), 'white mode (W)'),
  );
}

function flip(k) {
  st[k] = !st[k];
  savePrefs();
  drawToggles();
  drawResults();
}

function setBucket(b) {
  st.bucket = b;
  savePrefs();
  drawToggles();
  drawHead();
  drawSidecar();
}

async function pfOf(n) {
  if (!(n > 1)) return '';
  try {
    return (await measureOf(textOfOrdinal(n))).pf_ordinal;
  } catch {
    return '';
  }
}

// gsearch a number or a text's key in a bucket
async function gsearch(key, bucket = st.bucket) {
  if (bucket !== st.bucket) setBucket(bucket);
  try {
    const r = await app.call('mem.gsearch', { bucket, key });
    st.results = r.hits;
    const title = typeof key === 'number' ? key : await keyOf(key, bucket);
    st.header = { title, pf: typeof title === 'number' ? await pfOf(title) : '' };
    drawResults();
  } catch {
    /* reported */
  }
}

// weum's letter table as the sorted letters: e1i1l1n1s1t1 → eilnst
export function letters(f) {
  return String(f || '').replace(/(\D)(\d+)/g, (_, c, n) => c.repeat(+n));
}

async function keyOf(text, bucket) {
  const m = await measureOf(text);
  return bucket === 'frequency' ? letters(m.frequency) : m[bucket];
}

async function enter(e) {
  const raw = app.bar.get();
  const t = raw.trim();
  if (!t) return;
  const listed = await listCommand(app, t);
  if (listed) {
    if (listed.length) {
      st.results = listed;
      st.header = { title: picks.list ? picks.list.text : 'list', pf: '' };
      drawResults();
    }
    drawHead();
    return;
  }
  if (e.shiftKey) {
    try {
      const h = await app.call('mem.remember', { key: t });
      st.results = [h];
      st.header = { title: h.key, pf: '' };
      drawResults();
    } catch {
      /* reported */
    }
    return;
  }
  if (/^\d+$/.test(t)) return gsearch(+t);
  if (!st.gsearchText && !(e.ctrlKey || e.metaKey)) {
    try {
      st.results = await app.call('mem.search', { q: t });
      st.header = { title: t, pf: '' };
      drawResults();
    } catch {
      /* reported */
    }
    return;
  }
  if (st.saturate || e.ctrlKey || e.metaKey) {
    try {
      const s = await app.call('mem.saturate', { text: t });
      if (s.new) {
        app.status(`remembered “${t}” under ${s.key}`);
        app.counts.memories++;
        drawHead();
      }
    } catch {
      return;
    }
  }
  await gsearch(st.bucket === 'frequency' ? t : await keyOf(t, st.bucket));
}

async function forget(t) {
  try {
    const r = await app.call('mem.forget', { key: t });
    app.status(`forgot “${r.text}” (${t})`);
    st.results = st.results.filter((h) => h.key !== t);
    app.counts.memories = Math.max(0, app.counts.memories - 1);
    drawHead();
    drawResults();
    app.bar.set('');
  } catch {
    /* reported */
  }
}

export default {
  barName: 'memory',
  barPlaceholder: 'a number, a text, or a key',

  init(section, a) {
    app = a;
    loadPrefs();
    root = section;
    head = el('div.mem-head');
    band = el('div.mem-band', { hidden: true });
    toast = el('div.mem-toast');
    section.append(head, el('div.mem-main', band, toast));
    sidecar = el('span.sidecar');
    toggles = el('span.toggles');
    this.barRight = el('div.mem-right', sidecar, toggles);
    drawToggles();
    onPick(() => {
      drawHead();
      drawResults();
    });
    app.on('save', () => {
      st.results = [];
      st.header = null;
      measures.clear();
    });
  },

  async show() {
    await app.hello();
    drawHead();
    drawResults();
    measureTerm(app.bar.get());
  },

  barInput(t) {
    measureTerm(t);
  },

  barKey(e) {
    if (e.key === 'Enter') {
      enter(e);
      return true;
    }
    if (e.key === 'Delete') {
      const input = e.target;
      const t = input.value.trim();
      if (KEY.test(t) && !t.startsWith('note:') && input.selectionStart === input.value.length) {
        forget(t);
        return true;
      }
    }
  },

  key(e) {
    if (typing(e) || e.ctrlKey || e.metaKey || e.altKey) return false;
    const k = e.key.toLowerCase();
    const m = st.measure;
    if (k === 'o') setBucket('ordinal');
    else if (k === 'r') setBucket(st.bucket === 'reversed' ? 'ordinal' : 'reversed');
    else if (k === 'a') setBucket('atbash');
    else if (k === 'f') setBucket('frequency');
    else if (k === 'g') flip('gsearchText');
    else if (k === 's') flip('saturate');
    else if (k === 'w') flip('white');
    else if (m && e.key === 'ArrowLeft') gsearch(m.ordinal, 'ordinal');
    else if (m && e.key === 'ArrowRight') gsearch(m.reversed, 'reversed');
    else if (m && e.key === 'ArrowUp') gsearch(m.atbash, 'atbash');
    else if (m && e.key === 'ArrowDown') gsearch(app.bar.get().trim(), 'frequency');
    else return false;
    return true;
  },
};
