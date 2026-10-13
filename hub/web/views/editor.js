// the editor: the note list next to the editor card. The first `# line` is the note's
// title, the rest markdown; unsaved notes are marked • and saved when you switch away.

import { el, clear, debounce, confirmClick } from '../util.js';
import { render } from '../markdown.js';

let app;
let wrap, listEl, chips, tagInput, pad, rendered, toggleBtn, saveBtn, deleteBtn, count, hint;
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
      el('button.chip', { type: 'button', title: 'remove this tag', onclick: () => removeTag(t) }, t, el('span.x', '×')),
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
}

function drawBody() {
  const viewing = mode === 'view';
  pad.hidden = viewing;
  rendered.hidden = !viewing;
  toggleBtn.textContent = viewing ? 'edit' : 'view';
  if (viewing) {
    const title = titleOf(cur.text);
    const body = cur.text.replace(/^\s*#\s+.*(\n|$)/, '');
    rendered.innerHTML = (title ? `<h1>${title.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c])}</h1>` : '') + render(body);
  }
}

function show_(state) {
  cur = state;
  pad.value = cur.text;
  hint.textContent = cur.dirty ? 'unsaved' : '';
  deleteBtn.disabled = !cur.note._key;
  drawTags();
  drawBody();
  drawList();
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
      hint.textContent = '';
      deleteBtn.disabled = false;
      if (isNew) history.replaceState(null, '', `#/editor/${encodeURIComponent(stored._key)}`);
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
      el('button.primary.new', { type: 'button', title: 'new note (Ctrl+N)', onclick: () => newNote() }, '+ new'),
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
    toggleBtn = el('button', { type: 'button', title: 'view / edit (Ctrl+E)', onclick: toggleMode }, 'view');
    saveBtn = el('button', { type: 'button', title: 'save (Ctrl+S)', onclick: () => save() }, 'save');
    deleteBtn = confirmClick(el('button', { type: 'button', title: 'delete this note' }, 'delete'), 'sure?', remove);
    hint = el('span.hint');
    pad = el('textarea.pad', {
      spellcheck: true,
      'aria-label': 'the note',
      oninput: () => {
        cur.text = pad.value;
        touch();
      },
      onkeydown: (e) => {
        // tab indents instead of leaving
        if (e.key === 'Tab' && !e.ctrlKey && !e.altKey) {
          e.preventDefault();
          pad.setRangeText('  ', pad.selectionStart, pad.selectionEnd, 'end');
          cur.text = pad.value;
          touch();
        }
      },
    });
    rendered = el('article.rendered.md', { hidden: true });
    const card = el(
      'div.card.editor-card',
      el(
        'div.editor-head',
        el('button.slide', { type: 'button', title: 'the note list (Ctrl+B)', onclick: toggleList }, '‹'),
        chips,
        el('span.grow'),
        hint,
        tagInput,
        toggleBtn,
        saveBtn,
        deleteBtn,
      ),
      el('div.editor-body', pad, rendered),
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
    else return false;
    return true;
  },
};
