// the editor: the note list next to the editor card. The first `# line` is the note's
// title, the rest markdown; unsaved notes are marked • and saved when you switch away.

import { el, clear, debounce, confirmClick } from '../util.js';
import { render } from '../markdown.js';
import { lines as hlLines } from '../highlight.js';
import { icon, iconButton, tip } from '../icons.js';
import { drawer } from './notecls.js';

let app;
let nd, drawerBtn, wrap, listEl, chips, tagInput, pad, padWrap, mirror, rendered, toggleBtn, saveBtn, deleteBtn, count, hint;
let notes = [];
let q = '';
let mode = 'edit';
// the note being edited: what's stored, and what's in the editor
let cur = null; // {note, text, tags, dirty}
// unsaved notes you switched away from that couldn't be saved yet (no title)
const drafts = new Map();

function compose(n) {
  const body = n.markdown || '';
  return `# ${n.title || ''}\n\n${body}`.replace(/\n\n$/, '\n');
}

function blank() {
  return { note: { _key: '', title: '', markdown: '', tags: [] }, text: '# ', tags: [], dirty: false };
}

function titleOf(text) {
  const first = text.split('\n').find((l) => l.trim()) || '';
  const m = first.match(/^\s*#\s+(.*)$/);
  return m ? m[1].trim() : '';
}

async function load() {
  try {
    notes = await app.call('notes.list', q.trim() ? { q } : {});
  } catch {
    notes = [];
  }
  drawList();
}

function drawList() {
  const items = [];
  // the new note, while it's unsaved
  if (cur && !cur.note._key) {
    items.push(el('li.on', { title: 'not saved yet' }, `${titleOf(cur.text) || 'untitled'} •`));
  }
  for (const n of notes) {
    const on = cur && cur.note._key === n.key;
    const dirty = (on && cur.dirty) || drafts.has(n.key);
    items.push(
      el(
        'li',
        { className: on ? 'on' : '', title: `${n.words} words · ${n.tags.map((t) => '#' + t).join(' ')}`, onclick: () => app.go(`editor/${encodeURIComponent(n.key)}`) },
        `${(on ? titleOf(cur.text) : '') || n.title}${dirty ? ' •' : ''}`,
      ),
    );
  }
  if (!items.length) items.push(el('li.dim.empty', q ? 'no notes match' : 'no notes yet'));
  clear(listEl, items);
  count.textContent = `${notes.length} note${notes.length === 1 ? '' : 's'}`;
}

function drawTags() {
  clear(
    chips,
    (cur?.tags || []).map((t) =>
      el('button.chip', { type: 'button', title: `remove #${t}`, 'aria-label': `remove the tag ${t}`, onclick: () => removeTag(t) }, `#${t}`, el('span.x', '×')),
    ),
  );
}

function removeTag(t) {
  cur.tags = cur.tags.filter((x) => x !== t);
  touch();
  drawTags();
}

function addTags(raw) {
  for (const t of raw.split(',').map((s) => s.trim().replace(/^#/, '')).filter(Boolean)) {
    if (!cur.tags.includes(t)) cur.tags.push(t);
  }
  touch();
  drawTags();
}

function touch() {
  if (!cur.dirty) {
    cur.dirty = true;
    drawList();
  } else if (!cur.note._key) drawList();
  hint.textContent = 'unsaved';
  saveBtn.classList.add('dirty');
}

function drawBody() {
  const viewing = mode === 'view';
  padWrap.hidden = viewing;
  rendered.hidden = !viewing;
  toggleBtn.replaceChildren(icon(viewing ? 'edit' : 'view'));
  tip(toggleBtn, viewing ? 'edit' : 'view', 'Ctrl+E');
  if (!viewing) paint();
  if (viewing) {
    const title = titleOf(cur.text);
    const body = cur.text.replace(/^\s*#\s+.*(\n|$)/, '');
    rendered.innerHTML = (title ? `<h1>${title.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c])}</h1>` : '') + render(body);
  }
}

// ---------------------------------------------------------------- highlighting

// past this the text is shown plainly (highlighting it would slow typing down)
const PLAIN_PAST = 250_000;
let painted = []; // the highlighted lines in the mirror now
let paintFrame = 0;

function paintSoon() {
  if (!paintFrame) paintFrame = requestAnimationFrame(paint);
}

// the mirror catches up with the textarea: only the lines that changed are redrawn
function paint() {
  cancelAnimationFrame(paintFrame);
  paintFrame = 0;
  const text = pad.value;
  const plain = text.length > PLAIN_PAST;
  padWrap.classList.toggle('plain', plain);
  const next = plain ? [] : hlLines(text);
  let a = 0;
  while (a < next.length && a < painted.length && next[a] === painted[a]) a++;
  let b = 0;
  while (b < next.length - a && b < painted.length - a && next[next.length - 1 - b] === painted[painted.length - 1 - b]) b++;
  const kids = mirror.children;
  // lines changed in place (typing within a line): their divs are kept
  const same = Math.min(painted.length - b, next.length - b) - a;
  for (let i = a; i < a + same; i++) {
    const l = next[i];
    const k = kids[i];
    k.className = l.cls ? `ln ${l.cls}` : 'ln';
    k.innerHTML = l.html;
  }
  a += same;
  for (let i = painted.length - b - 1; i >= a; i--) kids[i].remove();
  if (next.length - b > a) {
    const tpl = document.createElement('template');
    tpl.innerHTML = next
      .slice(a, next.length - b)
      .map((l) => `<div class="ln${l.cls ? ' ' + l.cls : ''}">${l.html}</div>`)
      .join('');
    mirror.insertBefore(tpl.content, kids[a] || null);
  }
  painted = next;
  mirror.scrollTop = pad.scrollTop;
}

// the same width for the text as the textarea has beside its scrollbar (its gutter is
// kept whether it scrolls or not, so this only changes when the pad's size does)
let gutter = -1;
function fitMirror() {
  const bar = pad.offsetWidth - pad.clientWidth;
  if (bar !== gutter) {
    gutter = bar;
    mirror.style.paddingRight = `${10 + bar}px`;
  }
  mirror.scrollTop = pad.scrollTop;
  mirror.scrollLeft = pad.scrollLeft;
}

function show_(state) {
  cur = state;
  pad.value = cur.text;
  pad.scrollTop = 0;
  paint();
  hint.textContent = cur.dirty ? 'unsaved' : '';
  saveBtn.classList.toggle('dirty', cur.dirty);
  deleteBtn.disabled = !cur.note._key;
  drawTags();
  drawBody();
  drawList();
  nd.setKey(cur.note._key);
}

// leaving a note: save it if it changed (or keep it as a draft when it can't be yet)
async function leave() {
  if (!cur || !cur.dirty) return;
  if (titleOf(cur.text)) {
    await save(true);
  } else if (cur.note._key) {
    drafts.set(cur.note._key, cur);
  }
}

async function open(key) {
  if (cur && cur.note._key === key) return;
  await leave();
  if (drafts.has(key)) {
    const d = drafts.get(key);
    drafts.delete(key);
    show_(d);
    return;
  }
  try {
    const n = await app.call('notes.get', { key });
    show_({ note: n, text: compose(n), tags: [...(n.tags || [])], dirty: false });
  } catch {
    show_(blank());
  }
}

async function newNote() {
  await leave();
  show_(blank());
  if (location.hash !== '#/editor') history.replaceState(null, '', '#/editor');
  mode = 'edit';
  drawBody();
  pad.focus();
  pad.setSelectionRange(2, 2);
}

async function save(quiet = false) {
  if (!cur) return;
  if (!titleOf(cur.text)) {
    app.status('a note needs a # title on its first line', true);
    return;
  }
  const was = cur;
  try {
    const stored = await app.call('notes.save', { note: { ...cur.note, tags: cur.tags }, text: cur.text });
    const isNew = !was.note._key;
    // the editor may have moved on while this saved
    if (cur === was) {
      cur.note = stored;
      cur.dirty = pad.value !== cur.text ? true : false;
      hint.textContent = cur.dirty ? 'unsaved' : '';
      saveBtn.classList.toggle('dirty', cur.dirty);
      deleteBtn.disabled = false;
      if (isNew) history.replaceState(null, '', `#/editor/${encodeURIComponent(stored._key)}`);
      nd.setKey(stored._key);
      nd.refresh();
    }
    if (!quiet) app.status(`saved “${stored.title}”`);
    await load();
    app.hello();
  } catch {
    /* reported */
  }
}

async function remove() {
  if (!cur || !cur.note._key) return;
  const key = cur.note._key;
  try {
    await app.call('notes.delete', { key });
    app.status(`deleted “${cur.note.title}”`);
    cur.dirty = false;
    drafts.delete(key);
    await load();
    app.hello();
    const next = notes[0];
    if (next) app.go(`editor/${encodeURIComponent(next.key)}`);
    else newNote();
  } catch {
    /* reported */
  }
}

function toggleList() {
  wrap.classList.toggle('collapsed');
  wrap.classList.remove('peek');
  try {
    localStorage.setItem('anend.list', wrap.classList.contains('collapsed') ? 'away' : '');
  } catch {
    /* private window */
  }
}

function toggleMode() {
  mode = mode === 'edit' ? 'view' : 'edit';
  drawBody();
  if (mode === 'edit') pad.focus();
}

export default {
  barName: 'notes',
  barPlaceholder: 'find notes',

  init(section, a) {
    app = a;
    count = el('span.count');
    this.barRight = count;
    listEl = el('ul.notes');
    const side = el(
      'aside.card.notelist',
      iconButton('new', 'new note', { key: 'Ctrl+N', text: 'new note', cls: 'primary.new', onclick: () => newNote() }),
      listEl,
    );
    chips = el('div.chips');
    tagInput = el('input.tag-input', {
      type: 'text',
      placeholder: 'tag',
      'aria-label': 'add a tag',
      onkeydown: (e) => {
        if ((e.key === 'Enter' || e.key === ',') && tagInput.value.trim()) {
          e.preventDefault();
          addTags(tagInput.value);
          tagInput.value = '';
        }
      },
      onblur: () => {
        if (tagInput.value.trim()) {
          addTags(tagInput.value);
          tagInput.value = '';
        }
      },
    });
    toggleBtn = iconButton('view', 'view', { key: 'Ctrl+E', onclick: toggleMode });
    saveBtn = iconButton('save', 'save', { key: 'Ctrl+S', onclick: () => save() });
    deleteBtn = confirmClick(iconButton('delete', 'delete this note (click twice)'), 'delete?', remove);
    hint = el('span.hint');
    nd = drawer(app);
    drawerBtn = iconButton('classifier', 'classifiers on this note', { key: 'Ctrl+I', onclick: () => nd.toggle() });
    drawerBtn.classList.toggle('on', nd.open);
    drawerBtn.setAttribute('aria-pressed', String(nd.open));
    nd.onToggle = (on) => {
      drawerBtn.classList.toggle('on', on);
      drawerBtn.setAttribute('aria-pressed', String(on));
    };
    pad = el('textarea.pad', {
      spellcheck: true,
      'aria-label': 'the note',
      oninput: () => {
        cur.text = pad.value;
        touch();
        paintSoon();
      },
      onscroll: () => {
        mirror.scrollTop = pad.scrollTop;
        mirror.scrollLeft = pad.scrollLeft;
      },
      onkeydown: (e) => {
        // tab indents instead of leaving (as an edit the browser can undo)
        if (e.key === 'Tab' && !e.ctrlKey && !e.altKey && !e.shiftKey) {
          e.preventDefault();
          if (!document.execCommand('insertText', false, '  ')) {
            pad.setRangeText('  ', pad.selectionStart, pad.selectionEnd, 'end');
            pad.dispatchEvent(new Event('input'));
          }
        }
      },
    });
    // the highlighted copy of the text under the (see-through) textarea
    mirror = el('div.pad-mirror', { 'aria-hidden': 'true' });
    padWrap = el('div.pad-wrap', mirror, pad);
    new ResizeObserver(() => fitMirror()).observe(pad);
    rendered = el('article.rendered.md', { hidden: true });
    const card = el(
      'div.card.editor-card',
      el(
        'div.editor-head',
        iconButton('sidebar', 'the note list', { key: 'Ctrl+B', cls: 'plain.slide', onclick: toggleList }),
        chips,
        el('span.grow'),
        hint,
        el('label.tagbox', { title: 'add tags: Enter or a comma after each' }, icon('tag'), tagInput),
        drawerBtn,
        toggleBtn,
        saveBtn,
        deleteBtn,
      ),
      el('div.editor-body', padWrap, rendered, nd.el),
    );
    const edge = el('div.edge', { onmouseenter: () => wrap.classList.add('peek') });
    side.addEventListener('mouseleave', () => wrap.classList.remove('peek'));
    wrap = el('div.editor-wrap', edge, side, card);
    try {
      if (localStorage.getItem('anend.list') === 'away') wrap.classList.add('collapsed');
    } catch {
      /* private window */
    }
    section.append(wrap);
    app.on('save', () => {
      cur = null;
      drafts.clear();
    });
    window.addEventListener('beforeunload', (e) => {
      if ((cur && cur.dirty) || drafts.size) e.preventDefault();
    });
  },

  async show(rest) {
    await load();
    const key = rest[0];
    if (key) await open(key);
    else if (!cur) {
      if (notes[0]) app.go(`editor/${encodeURIComponent(notes[0].key)}`);
      else show_(blank());
    } else drawList();
  },

  hide() {
    leave();
  },

  barInput: debounce((text) => {
    q = text;
    load();
  }, 150),

  barKey(e) {
    if (e.key === 'Enter' && notes[0]) {
      app.go(`editor/${encodeURIComponent(notes[0].key)}`);
      pad.focus();
      return true;
    }
  },

  key(e) {
    const ctrl = e.ctrlKey || e.metaKey;
    if (!ctrl) return false;
    const k = e.key.toLowerCase();
    if (k === 's') save();
    else if (k === 'n') newNote();
    else if (k === 'e') toggleMode();
    else if (k === 'b') toggleList();
    else if (k === 'i') nd.toggle();
    else return false;
    return true;
  },
};
