// tags: every tag the notes have, with how many have it. Type in the command bar to find
// tags, click them to pick (several: notes with all of them, or any), and the notes they
// tag are listed beside, a click away from the editor. Routes: #/tags/a+b (all) and
// #/tags/a+b/any.

import { el, clear, debounce, relative } from '../util.js';
import { icon, iconButton } from '../icons.js';

let app, side, listEl, picked, modeEl, sortEl, results, count;
let tags = []; // [{tag, count}]
let chosen = []; // the picked tags, in the order picked
let any = false; // false: notes with all the picked tags; true: with any of them
let find = '';
let byName = false;
let asked = 0; // the newest notes.list call, so a slow answer can't land over a newer one

const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;

function route() {
  const t = chosen.map((x) => encodeURIComponent(x)).join('+');
  const want = t ? `#/tags/${t}${any && chosen.length > 1 ? '/any' : ''}` : '#/tags';
  if (location.hash !== want) history.replaceState(null, '', want);
}

// the route's tags: read from the hash itself, so a tag with a + or a / in it survives
function fromHash() {
  const raw = location.hash.replace(/^#\/?tags\/?/, '');
  const [t, m] = raw.split('/');
  chosen = t ? t.split('+').map((x) => decodeURIComponent(x)).filter(Boolean) : [];
  any = m === 'any';
}

async function loadTags() {
  try {
    tags = await app.call('notes.tags');
  } catch {
    tags = [];
  }
  drawTags();
}

function shown() {
  const f = find.trim().replace(/^#/, '').toLowerCase();
  let list = f ? tags.filter((t) => t.tag.toLowerCase().includes(f)) : tags.slice();
  if (byName) list.sort((a, b) => a.tag.localeCompare(b.tag));
  // tags that start with what's typed come first
  if (f) list.sort((a, b) => b.tag.toLowerCase().startsWith(f) - a.tag.toLowerCase().startsWith(f));
  return list;
}

function drawTags() {
  const list = shown();
  const max = Math.max(1, ...tags.map((t) => t.count));
  count.textContent = find.trim() ? `${list.length} of ${plural(tags.length, 'tag')}` : plural(tags.length, 'tag');
  clear(
    listEl,
    list.length
      ? list.map((t) => {
          const on = chosen.includes(t.tag);
          return el(
            'li',
            el(
              'button.tagrow',
              {
                type: 'button',
                className: `tagrow${on ? ' on' : ''}`,
                'aria-pressed': on ? 'true' : 'false',
                title: `${on ? 'unpick' : 'pick'} #${t.tag}: ${plural(t.count, 'note')}`,
                onclick: (e) => toggle(t.tag, e.shiftKey || e.ctrlKey || e.metaKey || chosen.length > 0 || on),
              },
              el('span.box', on ? icon('check') : null),
              el('span.name', `#${t.tag}`),
              el('span.track', el('span.fill', { style: { width: `${(t.count / max) * 100}%` } })),
              el('span.n', t.count),
            ),
          );
        })
      : // with no tags at all, the card beside says so (once)
        tags.length ? el('li.dim.empty', `no tag has “${find.trim()}”`) : null,
  );
  drawPicked();
}

function drawPicked() {
  clear(
    picked,
    chosen.length
      ? [
          ...chosen.map((t, i) => [
            i ? el('span.join', any ? 'or' : 'and') : null,
            el('button.chip.on', { type: 'button', title: `unpick #${t}`, 'aria-label': `unpick #${t}`, onclick: () => toggle(t, true) }, `#${t}`, el('span.x', '×')),
          ]),
          iconButton('close', 'unpick them all', { onclick: () => set([]), small: true, key: 'Esc' }),
        ]
      : el('span.dim.small', 'pick tags to see their notes; Shift or Ctrl+click adds to the pick'),
  );
  modeEl.hidden = chosen.length < 2;
  for (const b of modeEl.querySelectorAll('button')) b.classList.toggle('on', (b.dataset.mode === 'any') === any);
}

// a click picks the tag alone; with Shift/Ctrl (or once some are picked) it adds or removes it
function toggle(t, add) {
  if (!add) set([t]);
  else set(chosen.includes(t) ? chosen.filter((x) => x !== t) : [...chosen, t]);
}

function set(list) {
  chosen = list;
  route();
  drawTags();
  loadNotes();
}

function setMode(m) {
  any = m;
  route();
  drawPicked();
  loadNotes();
}

function preview(md) {
  return md
    .replace(/```[\s\S]*?(```|$)/g, ' ')
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}(#{1,6}\s+|>\s?|[-*+]\s+(\[[ xX]\]\s+)?|\d+[.)]\s+)/gm, '')
    .replace(/[*_~`|]+/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}

async function loadNotes() {
  const me = ++asked;
  if (!chosen.length) {
    clear(
      results,
      el(
        'div.card.tags-empty',
        el('div.big', icon('tag')),
        tags.length
          ? [
              el('p', 'Pick a tag to see the notes it tags. Pick more to narrow (all of them) or widen (any of them).'),
              el('p.dim.small', 'Type in the bar below to find a tag; Enter picks the first one, Backspace on an empty bar unpicks the last.'),
            ]
          : el('p', 'No tags yet. Give a note tags in the box at the top of the editor, and they gather here.'),
      ),
    );
    return;
  }
  let notes;
  try {
    notes = await app.call('notes.list', { tags: chosen, mode: any ? 'or' : 'and' });
  } catch {
    return;
  }
  if (me !== asked) return;
  const what = chosen.map((t) => `#${t}`).join(any ? ' or ' : ' and ');
  clear(
    results,
    el('div.tags-found', el('h2', `${plural(notes.length, 'note')} tagged `, el('span.what', what))),
    notes.length
      ? el(
          'ol.tagnotes',
          notes.map((n) =>
            el(
              'li.card.tagnote',
              el(
                'a.title',
                { href: `#/editor/${encodeURIComponent(n.key)}`, title: 'open in the editor' },
                n.title || 'untitled',
              ),
              el('div.meta.dim.small', `${relative(n.modified)} · ${plural(n.words, 'word')}`),
              n.tags.length
                ? el(
                    'div.chips',
                    n.tags.map((t) =>
                      el(
                        'button.chip',
                        { type: 'button', className: chosen.includes(t) ? 'chip on' : 'chip', title: chosen.includes(t) ? `unpick #${t}` : `add #${t} to the pick`, onclick: () => toggle(t, true) },
                        `#${t}`,
                      ),
                    ),
                  )
                : null,
              n.preview ? el('p.excerpt', preview(n.preview)) : null,
            ),
          ),
        )
      : el('div.card.dim', any ? 'no note has any of these' : 'no note has all of these: try “any”'),
  );
}

export default {
  barName: 'tags',
  barPlaceholder: 'find a tag (Enter picks it)',

  init(section, a) {
    app = a;
    count = el('span.count');
    this.barRight = count;
    listEl = el('ul.taglist', { 'aria-label': 'tags' });
    picked = el('div.picked');
    modeEl = el(
      'div.seg',
      { role: 'group', 'aria-label': 'notes with' },
      el('button', { type: 'button', dataset: { mode: 'all' }, title: 'notes with every picked tag', onclick: () => setMode(false) }, 'all'),
      el('button', { type: 'button', dataset: { mode: 'any' }, title: 'notes with any picked tag', onclick: () => setMode(true) }, 'any'),
    );
    sortEl = iconButton('sort', 'order: by count or by name', {
      small: true,
      text: 'count',
      cls: 'plain.sort',
      onclick: () => {
        byName = !byName;
        sortEl.querySelector('.lab').textContent = byName ? 'a–z' : 'count';
        drawTags();
      },
    });
    side = el('aside.card.tags-side', el('div.row.tags-head', el('h2', 'tags'), el('span.grow'), modeEl, sortEl), picked, listEl);
    results = el('div.tags-results');
    section.append(el('div.tags-view', side, results));
    app.on('save', () => {
      chosen = [];
    });
  },

  async show() {
    fromHash();
    await loadTags();
    loadNotes();
  },

  // (reads the bar when it fires: Enter may have cleared it meanwhile)
  barInput: debounce(() => {
    find = app.bar.get();
    drawTags();
  }, 60),

  barKey(e) {
    if (e.key === 'Enter') {
      find = app.bar.get(); // what's typed, even before the filter caught up
      const first = shown().find((t) => !chosen.includes(t.tag));
      if (first) {
        toggle(first.tag, true);
        app.bar.set('');
        find = '';
        drawTags();
      }
      return true;
    }
    if (e.key === 'Backspace' && !app.bar.get() && chosen.length) {
      set(chosen.slice(0, -1));
      return true;
    }
    if (e.key === 'ArrowDown') {
      listEl.querySelector('button')?.focus();
      return true;
    }
  },

  barEscape() {
    if (find) {
      app.bar.set('');
      find = '';
      drawTags();
    }
  },

  key(e) {
    if (e.key === 'Escape' && !e.defaultPrevented && chosen.length) {
      set([]);
      return true;
    }
    // arrows walk the tag list
    if (!e.target.closest?.('.taglist')) return false;
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return false;
    const all = [...listEl.querySelectorAll('button')];
    const i = all.indexOf(e.target);
    const next = all[i + (e.key === 'ArrowDown' ? 1 : -1)];
    if (next) next.focus();
    else if (e.key === 'ArrowUp') app.bar.focus();
    return true;
  },
};
