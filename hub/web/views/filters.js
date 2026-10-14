// filters (Ctrl+9): a tree of conditions built by hand (all of / any of / not, a
// classifier's class at a sureness, words, sums, a list, tags, memories or notes, another
// saved filter), run live over the save, kept as presets, and written to a list

import { el, clear, confirmClick, debounce, openHit } from '../util.js';
import { icon, iconButton, tip } from '../icons.js';

let app, side, main, count;
let presets = []; // filters.list
let classifiers = []; // classify.list
let lists = []; // lists.stats' lists
let tags = []; // notes.tags
let root = blank();
let name = '';
let description = '';
let saved = ''; // the expression as last saved or loaded (to tell unsaved changes)
let results = null; // filters.run
let runError = '';
let running = false;
let visible = false;

function blank() {
  return { op: 'and', of: [] };
}

const pct = (x) => `${Math.round(x * 100)}%`;
const KINDS = [
  ['class', 'classifier', 'a classifier’s class, at least this sure'],
  ['words', 'search', 'texts with these words'],
  ['sum', 'hash', 'a gematria sum in a range'],
  ['list', 'list', 'members of a list'],
  ['tags', 'tag', 'notes with these tags'],
  ['kind', 'memory', 'memories only, or notes only'],
  ['preset', 'filter', 'another saved filter'],
  ['and', 'and', 'a group: all of / any of'],
];

// ------------------------------------------------------------------ loading

async function load() {
  if (app.api.offline) {
    clear(side);
    clear(
      main,
      el(
        'div.card.clf-offline',
        el('h2', 'filters'),
        el('p', 'Filters need a hub for now: they ask classifiers, whose engine runs on the hub (or anend-hub --local on your own machine), not yet in the browser.'),
      ),
    );
    count.textContent = '';
    return;
  }
  try {
    [presets, classifiers, lists, tags] = await Promise.all([
      app.call('filters.list'),
      app.call('classify.list'),
      app.call('lists.stats').then((s) => s.lists),
      app.call('notes.tags'),
    ]);
  } catch {
    return;
  }
  count.textContent = `${presets.length} filter${presets.length === 1 ? '' : 's'}`;
  drawSide();
  drawMain();
}

function loadPreset(p) {
  name = p.name;
  description = p.description || '';
  const e = JSON.parse(JSON.stringify(p.expr));
  root = e.op === 'and' || e.op === 'or' ? e : { op: 'and', of: [e] };
  saved = JSON.stringify(clean(root));
  results = null;
  drawSide();
  drawMain();
  run();
}

function newFilter() {
  name = '';
  description = '';
  root = blank();
  saved = '';
  results = null;
  runError = '';
  if (location.hash !== '#/filters') history.replaceState(null, '', '#/filters');
  drawSide();
  drawMain();
}

// ------------------------------------------------------------------ the expression

// what can run: conditions not filled in yet are left out, and so are empty groups
function clean(e) {
  switch (e.op) {
    case 'and':
    case 'or':
      return { op: e.op, of: e.of.map(clean).filter((x) => x && !((x.op === 'and' || x.op === 'or') && !x.of.length)) };
    case 'not': {
      const x = clean(e.of);
      return x && !((x.op === 'and' || x.op === 'or') && !x.of.length) ? { op: 'not', of: x } : null;
    }
    case 'class':
      return e.classifier && e.class ? { op: 'class', classifier: e.classifier, class: e.class, ...(e.min != null ? { min: e.min } : {}), ...(e.max != null ? { max: e.max } : {}), ...(e.top ? { top: true } : {}) } : null;
    case 'words':
      return e.q.trim() ? { op: 'words', q: e.q.trim() } : null;
    case 'sum':
      return e.from == null && e.to == null ? null : { op: 'sum', bucket: e.bucket, ...(e.from != null ? { from: e.from } : {}), ...(e.to != null ? { to: e.to } : {}) };
    case 'list':
      return e.list ? { op: 'list', list: e.list } : null;
    case 'tags': {
      const t = e.tags.filter(Boolean);
      return t.length ? { op: 'tags', tags: t, ...(e.any ? { any: true } : {}) } : null;
    }
    case 'kind':
      return { op: 'kind', kind: e.kind };
    case 'preset':
      return e.name ? { op: 'preset', name: e.name } : null;
  }
  return null;
}

function make(kind) {
  switch (kind) {
    case 'class': {
      const c = classifiers.find((x) => x.trained) || classifiers[0];
      return { op: 'class', classifier: c?.name || '', class: c?.classes?.[0] || '' };
    }
    case 'words':
      return { op: 'words', q: '' };
    case 'sum':
      return { op: 'sum', bucket: 'ordinal' };
    case 'list':
      return { op: 'list', list: lists[0]?.list || '' };
    case 'tags':
      return { op: 'tags', tags: [] };
    case 'kind':
      return { op: 'kind', kind: 'notes' };
    case 'preset':
      return { op: 'preset', name: presets.find((p) => p.name !== name)?.name || '' };
    default:
      return { op: kind, of: [] };
  }
}

// the expression in words (as the hub says it)
function describe(e) {
  switch (e.op) {
    case 'and':
    case 'or':
      return e.of.length ? `(${e.of.map(describe).join(e.op === 'and' ? ' and ' : ' or ')})` : e.op === 'and' ? 'everything' : 'nothing';
    case 'not':
      return `not ${describe(e.of)}`;
    case 'class':
      return `${e.classifier}: ${e.class}${e.top ? ' (top)' : ''}${e.min != null ? ` ≥ ${pct(e.min)}` : ''}${e.max != null ? ` ≤ ${pct(e.max)}` : ''}`;
    case 'words':
      return `“${e.q}”`;
    case 'sum':
      return `${e.bucket} ${e.from ?? ''}–${e.to ?? ''}`;
    case 'list':
      return `in “${lists.find((l) => l.list === e.list)?.name || e.list}”`;
    case 'tags':
      return e.tags.map((t) => '#' + t).join(e.any ? ' or ' : ' and ');
    case 'kind':
      return e.kind;
    case 'preset':
      return `[${e.name}]`;
  }
  return '?';
}

const changed = debounce(() => {
  drawState();
  run();
}, 300);

// ------------------------------------------------------------------ running

let asked = 0;
async function run() {
  let expr = clean(root);
  // nothing chosen yet: every memory and note, as the tree says
  if (!expr.of?.length && (expr.op === 'and' || expr.op === 'or')) expr = { op: 'and', of: [] };
  const me = ++asked;
  running = true;
  drawResults();
  try {
    const asSaved = name && JSON.stringify(expr) === saved;
    const r = await app.api.call('filters.run', asSaved ? { name, limit: 200 } : { expr, limit: 200 });
    if (me !== asked) return;
    results = r;
    runError = '';
  } catch (e) {
    if (me !== asked) return;
    runError = e.message;
  }
  running = false;
  drawResults();
}

async function save() {
  const n = nameEl.value.trim();
  if (!n) {
    app.status('give the filter a name first', true);
    nameEl.focus();
    return false;
  }
  try {
    const p = await app.call('filters.save', { name: n, expr: clean(root), description: descEl.value });
    name = p.name;
    saved = JSON.stringify(clean(root));
    app.status(`kept the filter “${p.name}”`);
    history.replaceState(null, '', `#/filters/${encodeURIComponent(p.name)}`);
    presets = await app.call('filters.list');
    count.textContent = `${presets.length} filter${presets.length === 1 ? '' : 's'}`;
    drawSide();
    drawState();
    return true;
  } catch {
    return false;
  }
}

async function enlist() {
  if (!name || JSON.stringify(clean(root)) !== saved || nameEl.value.trim() !== name) {
    if (!(await save())) return;
  }
  try {
    const r = await app.call('filters.enlist', { name });
    app.status(`${r.members} texts in the list “${r.name}”`);
    presets = await app.call('filters.list');
    drawSide();
  } catch {
    /* reported */
  }
}

// ------------------------------------------------------------------ drawing

function drawSide() {
  clear(
    side,
    el('div.row.flt-side-head', el('h2', 'filters'), el('span.grow'), iconButton('new', 'a new filter', { text: 'new', small: true, onclick: newFilter })),
    presets.length
      ? el(
          'ul.flt-presets',
          presets.map((p) => {
            const del = confirmClick(iconButton('delete', `delete “${p.name}” (click twice)`, { small: true, cls: 'plain' }), 'delete?', async () => {
              try {
                await app.call('filters.delete', { name: p.name });
                app.status(`deleted the filter “${p.name}”`);
                if (p.name === name) newFilter();
                load();
              } catch {
                /* reported */
              }
            });
            return el(
              'li',
              { className: p.name === name ? 'on' : '' },
              el(
                'button.flt-preset',
                { type: 'button', onclick: () => app.go(`filters/${encodeURIComponent(p.name)}`) },
                el('span.nm', p.name),
                el('span.reads', p.reads),
                el('span.meta', p.count != null ? `${p.count} found` : 'not run yet', p.list ? ' · in a list' : '', p.description ? ` · ${p.description}` : ''),
              ),
              del,
            );
          }),
        )
      : el('p.dim.small', 'none kept yet: build one, then keep it under a name. Filters can use each other.'),
  );
}

let nameEl, descEl, stateEl, treeEl, resultsEl;

function drawMain() {
  nameEl = el('input.flt-name', { type: 'text', placeholder: 'its name', value: name, 'aria-label': 'the filter’s name', oninput: () => drawState() });
  descEl = el('input', { type: 'text', placeholder: 'what it finds (optional)', value: description, 'aria-label': 'description', oninput: () => drawState() });
  stateEl = el('div.flt-state');
  treeEl = el('div.flt-tree');
  resultsEl = el('div.card.flt-results');
  clear(
    main,
    el(
      'div.card.flt-build',
      el(
        'div.row.flt-top',
        nameEl,
        descEl,
        iconButton('save', 'keep this filter under its name', { text: 'keep', cls: 'primary', onclick: save }),
        iconButton('list', 'write what it finds to a list (“filter: name”)', { text: 'to a list', onclick: enlist }),
      ),
      stateEl,
      treeEl,
    ),
    resultsEl,
  );
  drawTree();
  drawState();
  drawResults();
}

function drawState() {
  if (!stateEl) return;
  const now = JSON.stringify(clean(root));
  const dirty = now !== saved || (nameEl && nameEl.value.trim() !== name);
  clear(stateEl, el('span.flt-reads', describe(clean(root))), dirty && (name || root.of.length) ? el('span.hint', name ? 'unsaved changes' : 'not kept yet') : null);
}

function drawTree() {
  clear(treeEl, groupEl(root, null, -1, 0));
}

// a change to the tree's shape: draw it again and run
function reshape() {
  drawTree();
  changed();
}

function replace(parent, i, node) {
  if (parent) parent.of[i] = node;
  reshape();
}

function notBtn(node, parent, i) {
  const negated = node.op === 'not';
  return tip(
    el('button.flt-not', {
      type: 'button',
      className: `flt-not${negated ? ' on' : ''}`,
      'aria-pressed': String(negated),
      onclick: () => replace(parent, i, negated ? node.of : { op: 'not', of: node }),
    }, icon('not'), el('span', 'not')),
    negated ? 'take the “not” away' : 'turn it around: everything this doesn’t match',
  );
}

function removeBtn(parent, i) {
  return iconButton('close', 'remove this', { small: true, cls: 'plain', onclick: () => (parent.of.splice(i, 1), reshape()) });
}

function addMenu(group) {
  const s = el(
    'select.flt-add',
    {
      'aria-label': 'add a condition',
      onchange: () => {
        if (!s.value) return;
        group.of.push(make(s.value));
        reshape();
      },
    },
    el('option', { value: '' }, '+ add a condition…'),
    KINDS.map(([k, , about]) => el('option', { value: k }, about)),
  );
  return s;
}

// a node, which may be a "not" around something
function nodeEl(node, parent, i, depth) {
  const inner = node.op === 'not' ? node.of : node;
  const holder = node.op === 'not' ? node : null;
  if (inner.op === 'and' || inner.op === 'or') return groupEl(inner, parent, i, depth, holder);
  return condEl(inner, parent, i, holder);
}

function groupEl(g, parent, i, depth, holder = null) {
  const outer = holder || g;
  const seg = el(
    'div.seg',
    { role: 'group', 'aria-label': 'how they combine' },
    tip(el('button', { type: 'button', className: g.op === 'and' ? 'on' : '', onclick: () => ((g.op = 'and'), reshape()) }, icon('and'), ' all of'), 'every condition must hold'),
    tip(el('button', { type: 'button', className: g.op === 'or' ? 'on' : '', onclick: () => ((g.op = 'or'), reshape()) }, icon('or'), ' any of'), 'any one condition is enough'),
  );
  return el(
    'div.flt-group',
    { className: `flt-group ${g.op}${holder ? ' negated' : ''}${depth ? '' : ' root'}`, style: { '--depth': depth } },
    el(
      'div.flt-ghead',
      depth ? notBtn(outer, parent, i) : null,
      seg,
      el('span.dim.small', depth ? '' : 'texts matching'),
      el('span.grow'),
      addMenu(g),
      depth ? removeBtn(parent, i) : null,
    ),
    g.of.length
      ? el('div.flt-kids', g.of.map((k, j) => nodeEl(k, g, j, depth + 1)))
      : el('p.dim.small.flt-empty', depth ? 'empty: add a condition' : 'no conditions yet: every memory and note matches. Add a condition to narrow it down.'),
  );
}

function condEl(c, parent, i, holder) {
  const outer = holder || c;
  const [, ic, about] = KINDS.find((k) => k[0] === c.op) || [null, 'help', c.op];
  return el(
    'div.flt-cond',
    { className: `flt-cond${holder ? ' negated' : ''}` },
    tip(el('span.flt-kind', icon(ic)), about),
    notBtn(outer, parent, i),
    el('div.flt-fields', fields(c)),
    removeBtn(parent, i),
  );
}

function fields(c) {
  const sel = (label, value, options, onchange) => {
    const s = el('select', { 'aria-label': label, onchange: () => onchange(s.value) }, options.map(([v, t]) => el('option', { value: v, selected: v === value }, t)));
    return s;
  };
  switch (c.op) {
    case 'class': {
      const cl = classifiers.find((x) => x.name === c.classifier);
      const out = el('span.val', c.min == null ? 'its threshold' : pct(c.min));
      const r = el('input', {
        type: 'range', min: 0, max: 1, step: 0.01, value: c.min ?? 0.5, 'aria-label': 'at least this sure',
        className: c.min == null ? 'unset' : '',
        oninput: () => {
          c.min = +r.value;
          r.classList.remove('unset');
          out.textContent = pct(c.min);
          changed();
        },
      });
      const top = el('input', { type: 'checkbox', checked: !!c.top, onchange: () => ((c.top = top.checked), changed()) });
      return [
        sel('classifier', c.classifier, [['', 'a classifier'], ...classifiers.map((x) => [x.name, `${x.name}${x.trained ? '' : ' (untrained)'}`])], (v) => {
          c.classifier = v;
          c.class = classifiers.find((x) => x.name === v)?.classes?.[0] || '';
          reshape();
        }),
        el('span.dim', 'says'),
        sel('class', c.class, (cl?.classes || []).map((x) => [x, x]), (v) => ((c.class = v), changed())),
        el('label.flt-min', el('span.dim', 'at least'), r, out, c.min != null ? iconButton('undo', 'back to its own threshold', { small: true, cls: 'plain', onclick: () => ((c.min = undefined), reshape()) }) : null),
        tip(el('label.check.small', top, ' its top class'), 'only when this is the class it is surest of'),
      ];
    }
    case 'words': {
      const q = el('input', { type: 'search', value: c.q, placeholder: 'words (with their variations)', 'aria-label': 'words', oninput: () => ((c.q = q.value), changed()) });
      return [el('span.dim', 'has'), q];
    }
    case 'sum': {
      const n = (k, label) => {
        const x = el('input', { type: 'number', value: c[k] ?? '', placeholder: label, 'aria-label': label, oninput: () => ((c[k] = x.value === '' ? undefined : Math.trunc(+x.value)), changed()) });
        return x;
      };
      return [sel('sum', c.bucket, [['ordinal', 'ordinal'], ['reversed', 'reversed'], ['atbash', 'atbash']], (v) => ((c.bucket = v), changed())), el('span.dim', 'from'), n('from', 'from'), el('span.dim', 'to'), n('to', 'to')];
    }
    case 'list':
      return [el('span.dim', 'in the list'), sel('list', c.list, [['', 'a list'], ...lists.map((l) => [l.list, `${l.name} (${l.count})`])], (v) => ((c.list = v), changed()))];
    case 'tags': {
      const id = 'flt-tags-known';
      const t = el('input', { type: 'text', value: c.tags.join(', '), placeholder: 'tags, comma separated', 'aria-label': 'tags', list: id, oninput: () => ((c.tags = t.value.split(',').map((x) => x.trim().replace(/^#/, '')).filter(Boolean)), changed()) });
      t.setAttribute('list', id);
      const any = el('input', { type: 'checkbox', checked: !!c.any, onchange: () => ((c.any = any.checked), changed()) });
      return [el('span.dim', 'notes tagged'), t, el('datalist', { id }, tags.map((x) => el('option', { value: x.tag }))), tip(el('label.check.small', any, ' any of them'), 'off: every one of them')];
    }
    case 'kind':
      return [
        el('span.dim', 'only'),
        el(
          'div.seg',
          { role: 'group', 'aria-label': 'which texts' },
          ['memories', 'notes'].map((k) => el('button', { type: 'button', className: c.kind === k ? 'on' : '', onclick: () => ((c.kind = k), reshape()) }, k)),
        ),
      ];
    case 'preset':
      return [el('span.dim', 'matches the filter'), sel('filter', c.name, [['', 'a saved filter'], ...presets.filter((p) => p.name !== name).map((p) => [p.name, p.name])], (v) => ((c.name = v), changed()))];
  }
  return [];
}

function drawResults() {
  if (!resultsEl) return;
  const r = results;
  clear(
    resultsEl,
    el(
      'div.row',
      el('h3', 'found'),
      r ? el('span.dim.small', `${r.matched} of ${r.scanned} texts${r.results.length < r.matched ? ` · the first ${r.results.length} shown` : ''}`) : null,
      running ? el('span.dim.small', '· running…') : null,
      el('span.grow'),
    ),
    runError ? el('p.bad.small', runError) : null,
    r && r.untrained.length ? el('p.small.flt-warn', icon('alert'), ` not trained, so left out: ${r.untrained.join(', ')}`) : null,
    r
      ? r.results.length
        ? el(
            'ol.clf-results.flt-found',
            r.results.map((x) =>
              el(
                'li',
                el(
                  'div.res',
                  r.classifiers.length ? tip(el('span.prob', { style: { '--p': x.score } }, pct(x.score)), 'how strongly its classifier conditions hold (the weakest of them)') : null,
                  el('button.text.linkish', { type: 'button', onclick: () => openHit(app, x) }, x.note ? el('span.dim', 'note · ') : null, x.text),
                ),
              ),
            ),
          )
        : el('p.dim.small', 'nothing matches')
      : running
        ? el('p.dim.small', 'running…')
        : null,
  );
}

export default {
  barName: 'filters',
  barPlaceholder: 'words: Enter adds them as a condition',
  init(section, a) {
    app = a;
    count = el('span.count');
    this.barRight = count;
    side = el('aside.card.flt-side');
    main = el('div.flt-main');
    section.append(el('div.flt-view', side, main));
    app.on('save', () => {
      newFilter();
    });
  },
  async show(rest) {
    visible = true;
    await load();
    if (app.api.offline) return;
    const want = rest[0];
    if (want && want !== name) {
      const p = presets.find((x) => x.name === want);
      if (p) loadPreset(p);
      else app.status(`there's no filter “${want}”`, true);
    } else run();
  },
  hide() {
    visible = false;
  },
  barKey(e) {
    if (e.key === 'Enter' && !app.api.offline && visible) {
      const q = app.bar.get().trim();
      if (!q) return true;
      root.of.push({ op: 'words', q });
      app.bar.set('');
      reshape();
      return true;
    }
  },
};
