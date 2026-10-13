// settings: the saves (and moving them in and out), the forge, the hub, the colours
// and the view's options, over whatever view is open. Bundles (a save as JSON, keys and
// all) move saves between a browser running offline and a hub's workspace.

import { el, clear, size, shortDate, debounce, confirmClick } from '../util.js';
import * as theme from '../theme.js';

let app, overlay, panel;
let saves = [];
const forge = { effects: {}, filter: { words: '', ordinal_from: '', ordinal_to: '', reversed_from: '', reversed_to: '', memories: true, notes: true, lists: true } };
const hubState = { q: '', sort: 'new', page: 0, saves: [], more: false };

function section(title, ...kids) {
  return el('section.set', el('h3', title), ...kids);
}

// ---------------------------------------------------------------- saves

async function loadSaves() {
  try {
    saves = await app.call('saves.list');
  } catch {
    saves = [];
  }
  // the open one first, then the most recently changed
  saves.sort((a, b) => (b.open - a.open) || String(b.modified).localeCompare(String(a.modified)));
}

async function act(op, args, msg) {
  try {
    const r = await app.call(op, args);
    if (msg) app.status(typeof msg === 'function' ? msg(r) : msg);
    return r ?? true;
  } catch {
    return null;
  }
}

function savesSection() {
  const nameInput = el('input', { type: 'text', placeholder: 'a new save’s name', 'aria-label': 'new save name' });
  const create = async () => {
    const name = nameInput.value.trim();
    if (!name) return;
    if (await act('saves.new', { name }, `started the save “${name}”`)) {
      await app.saveChanged();
      redraw();
    }
  };
  nameInput.addEventListener('keydown', (e) => e.key === 'Enter' && create());
  const offline = app.api.offline;
  const file = el('input', { type: 'file', accept: '.redb', hidden: true, onchange: () => upload(file) });
  const bundle = el('input', { type: 'file', accept: '.json,application/json', hidden: true, onchange: () => importBundle(bundle) });
  const pickBundle = (m) => {
    bundle.dataset.merge = m ? '1' : '';
    bundle.click();
  };
  const rows = saves.map((s) => saveRow(s));
  return section(
    'saves',
    el(
      'p.dim.small',
      offline
        ? 'each save holds notes, memories and lists. No hub served this page, so they live in this browser (IndexedDB), and only here: export a bundle to keep one, or to take it to a hub, whose app imports it with the same keys.'
        : 'each save is a database of its own: notes, memories, lists. They live on this hub, in your workspace. A bundle exported from the app running offline in a browser imports here with its keys.',
    ),
    el(
      'div.row',
      nameInput,
      el('button.primary', { type: 'button', onclick: create }, 'new save'),
      el('span.grow'),
      el('button', { type: 'button', onclick: () => pickBundle(false), title: 'a save bundle (.anend.json) in as a new save, keeping its keys' }, 'import bundle…'),
      el('button', { type: 'button', onclick: () => pickBundle(true), title: `a save bundle merged into “${app.save}”: what’s new gets fresh keys` }, 'merge bundle…'),
      offline ? null : el('button', { type: 'button', onclick: () => file.click(), title: 'a .redb save from your computer' }, 'upload a save…'),
      file,
      bundle,
    ),
    el('div.saves', rows.length ? rows : el('p.dim', 'no saves yet')),
  );
}

// a file to save, made here
function download(name, text, type = 'application/json') {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = el('a', { href: url, download: name, hidden: true });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
}

async function exportBundle(name) {
  const b = await act('saves.export', { name });
  if (!b || b === true) return;
  download(`${b.name || name}.anend.json`, JSON.stringify(b));
  app.status(`exported “${name}”: ${b.notes.length} notes, ${b.memories.length} memories, ${b.lists.length} list links`);
}

async function importBundle(input) {
  const f = input.files && input.files[0];
  const merge = input.dataset.merge === '1';
  input.value = '';
  if (!f) return;
  let bundle;
  try {
    bundle = JSON.parse(await f.text());
  } catch {
    return app.status(`${f.name} isn’t JSON`, true);
  }
  const r = await act('saves.import', merge ? { bundle, merge: true } : { bundle }, (r) =>
    r.merged ? `merged ${r.memories} memories, ${r.notes} notes, ${r.lists} lists into “${app.save}”` : `“${r.save}” is in your saves: ${r.memories} memories, ${r.notes} notes, ${r.lists} lists`,
  );
  if (r && r.merged) await app.saveChanged();
  if (r) redraw();
}

function saveRow(s) {
  const nameCell = el('div.name', el('b', s.name), s.open ? el('span.badge', 'open') : null);
  const rename = el('button.small', {
    type: 'button',
    onclick: () => {
      const input = el('input', { type: 'text', value: s.name, 'aria-label': 'new name' });
      const done = async () => {
        const to = input.value.trim();
        if (to && to !== s.name && (await act('saves.rename', { name: s.name, to }, `renamed to “${to}”`))) {
          if (s.open) await app.saveChanged();
          redraw();
        } else redraw();
      };
      input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') done();
        if (e.key === 'Escape') {
          e.stopPropagation();
          redraw();
        }
      });
      clear(nameCell, input);
      input.focus();
      input.select();
    },
  }, 'rename');
  const del = s.open
    ? null
    : confirmClick(el('button.small.bad', { type: 'button' }, 'delete'), 'sure?', async () => {
        if (await act('saves.delete', { name: s.name }, `deleted “${s.name}”`)) redraw();
      });
  return el(
    'div.save-row',
    { className: s.open ? 'save-row open' : 'save-row' },
    nameCell,
    el('div.meta.dim.small', `${s.notes ?? '?'} notes · ${s.memories ?? '?'} memories · ${size(s.size)}${s.modified ? ' · ' + shortDate(s.modified) : ''}`),
    el(
      'div.acts',
      s.open
        ? null
        : el('button.small.primary', {
            type: 'button',
            onclick: async () => {
              if (await act('saves.open', { name: s.name }, `opened “${s.name}”`)) {
                await app.saveChanged();
                redraw();
              }
            },
          }, 'load'),
      el('button.small', {
        type: 'button',
        onclick: async () => {
          if (await act('saves.copy', { name: s.name }, (r) => `copied to “${r.save}”`)) redraw();
        },
      }, 'copy'),
      rename,
      el('button.small', { type: 'button', onclick: () => exportBundle(s.name), title: 'this save as a bundle (.anend.json): keys and all, for a hub or a browser' }, 'export bundle'),
      app.api.offline ? null : el('a.btn.small', { href: `saves/${encodeURIComponent(s.name)}`, download: `${s.name}.redb`, title: 'download this save' }, 'download'),
      del,
    ),
  );
}

async function upload(input) {
  const f = input.files && input.files[0];
  if (!f) return;
  const form = new FormData();
  form.append('file', f);
  form.append('name', f.name.replace(/\.redb$/i, ''));
  app.status(`uploading ${f.name}…`);
  try {
    const r = await app.api.send('saves', { method: 'POST', body: form });
    app.status(`“${r.save}” is in your saves`);
    redraw();
  } catch (e) {
    app.fail(e);
  }
  input.value = '';
}

// ---------------------------------------------------------------- the forge

function forgeArgs() {
  const ingredients = Object.entries(forge.effects)
    .filter(([, e]) => e)
    .map(([save, effect]) => ({ save, effect }));
  const f = forge.filter;
  const num = (v) => (v === '' || v == null || isNaN(+v) ? undefined : +v);
  const filter = {
    words: f.words,
    ordinal_from: num(f.ordinal_from),
    ordinal_to: num(f.ordinal_to),
    reversed_from: num(f.reversed_from),
    reversed_to: num(f.reversed_to),
    memories: f.memories,
    notes: f.notes,
    lists: f.lists,
  };
  return { ingredients, filter };
}

let preview;
const refreshPreview = debounce(async () => {
  if (!preview) return;
  const args = forgeArgs();
  if (!args.ingredients.length) {
    clear(preview, el('span.dim', 'pick a save or two above to see what would come out'));
    return;
  }
  try {
    const p = await app.api.call('forge.preview', args);
    clear(
      preview,
      el('div', el('b', p.memories), ' memories · ', el('b', p.notes), ' notes · ', el('b', p.lists), ' lists'),
      p.sample.length ? el('div.sample', p.sample.map((s) => el('span.member', { title: `${s.ordinal} / ${s.reversed}` }, s.text))) : null,
      p.titles.length ? el('div.dim.small', 'notes: ', p.titles.join(' · ')) : null,
    );
  } catch (e) {
    clear(preview, el('span.bad', e.message));
  }
}, 250);

function forgeSection() {
  const effects = ['', 'add', 'take away', 'keep shared'];
  const ingredients = saves.map((s) =>
    el(
      'label.ingredient',
      el('span', s.name),
      el(
        'select',
        {
          'aria-label': `what ${s.name} does`,
          onchange: (e) => {
            forge.effects[s.name] = e.target.value;
            refreshPreview();
          },
        },
        effects.map((x) => el('option', { value: x, selected: (forge.effects[s.name] || '') === x }, x || '—')),
      ),
    ),
  );
  const f = forge.filter;
  const field = (k, ph, type = 'text') =>
    el('input', {
      type,
      placeholder: ph,
      value: f[k],
      'aria-label': ph,
      oninput: (e) => {
        f[k] = e.target.value;
        refreshPreview();
      },
    });
  const kind = (k) =>
    el('label.check', el('input', { type: 'checkbox', checked: f[k], onchange: (e) => ((f[k] = e.target.checked), refreshPreview()) }), k);
  const name = el('input', { type: 'text', placeholder: 'the new save’s name', 'aria-label': 'forged save name' });
  preview = el('div.preview');
  refreshPreview();
  const pour = confirmClick(el('button', { type: 'button', title: 'add what comes out to the open save' }, 'pour into the open save'), 'sure?', async () => {
    const r = await act('forge.pour', forgeArgs(), (r) => `poured ${r.memories} memories, ${r.notes} notes, ${r.lists} lists into “${app.save}”`);
    if (r) {
      await app.saveChanged();
      redraw();
    }
  });
  return section(
    'the forge',
    el('p.dim.small', 'make a save out of others: add what they hold, take away what another has, or keep only what they share; then filter it.'),
    el('div.ingredients', ingredients),
    el('div.filters', field('words', 'words it must have'), el('span.range', field('ordinal_from', 'ordinal from', 'number'), field('ordinal_to', 'to', 'number')), el('span.range', field('reversed_from', 'reversed from', 'number'), field('reversed_to', 'to', 'number')), el('span.kinds', kind('memories'), kind('notes'), kind('lists'))),
    preview,
    el(
      'div.row',
      name,
      el('button.primary', {
        type: 'button',
        onclick: async () => {
          const n = name.value.trim();
          if (!n) return app.status('give the new save a name', true);
          if (await act('forge.make', { ...forgeArgs(), name: n }, (r) => `forged “${r.save}”: ${r.memories} memories, ${r.notes} notes, ${r.lists} lists`)) redraw();
        },
      }, 'forge'),
      pour,
    ),
  );
}

// ---------------------------------------------------------------- the hub

let hubList;
async function loadHub(more = false) {
  hubState.page = more ? hubState.page + 1 : 0;
  const u = `/api/saves?q=${encodeURIComponent(hubState.q)}&sort=${hubState.sort}&page=${hubState.page}`;
  try {
    const r = await app.api.send(u);
    hubState.saves = more ? [...hubState.saves, ...r.saves] : r.saves;
    hubState.more = r.more;
  } catch (e) {
    hubState.saves = [];
    hubState.more = false;
    if (hubList) clear(hubList, el('p.bad', e.message));
    return;
  }
  drawHub();
}

function drawHub() {
  if (!hubList) return;
  clear(
    hubList,
    hubState.saves.length
      ? hubState.saves.map((s) =>
          el(
            'div.hub-save',
            el('div', el('b', s.name), el('span.dim.small', ` by ${s.author}`), s.featured ? el('span.badge', 'featured') : null),
            s.description ? el('div.small', s.description) : null,
            el('div.dim.small', `${s.memories} memories · ${s.notes} notes · ${size(s.size)} · ★ ${s.stars} · ↓ ${s.downloads}`, s.tags.length ? ' · ' + s.tags.map((t) => '#' + t).join(' ') : ''),
            el(
              'div.acts',
              el('button.small.primary', {
                type: 'button',
                title: 'into your saves, beside the others',
                onclick: async () => {
                  try {
                    const r = await app.api.send(`hub/${encodeURIComponent(s.id)}`, { method: 'POST' });
                    app.status(`“${r.save}” is in your saves`);
                    redraw();
                  } catch (e) {
                    app.fail(e);
                  }
                },
              }, 'take'),
              confirmClick(el('button.small', { type: 'button', title: 'into the open save' }, 'merge'), 'sure?', async () => {
                try {
                  const r = await app.api.send(`hub/${encodeURIComponent(s.id)}?merge=1`, { method: 'POST' });
                  app.status(`merged ${r.memories} memories, ${r.notes} notes, ${r.lists} lists into “${app.save}”`);
                  await app.saveChanged();
                  redraw();
                } catch (e) {
                  app.fail(e);
                }
              }),
              el('a.small', { href: `/saves/${encodeURIComponent(s.id)}`, target: '_blank', rel: 'noopener' }, 'its page'),
            ),
          ),
        )
      : el('p.dim', hubState.q ? 'nothing on the hub matches' : 'nothing shared on this hub yet'),
    hubState.more ? el('button.small', { type: 'button', onclick: () => loadHub(true) }, 'more') : null,
  );
}

function hubSection() {
  if (app.api.offline) {
    return section(
      'the hub',
      el('p.dim.small', 'browsing, sharing and taking saves from a hub, and .redb files, need the app served by a hub; here, export a bundle and import it in a hub’s app.'),
    );
  }
  const q = el('input', { type: 'search', placeholder: 'search the hub', value: hubState.q, 'aria-label': 'search the hub' });
  q.addEventListener('input', debounce(() => ((hubState.q = q.value), loadHub()), 250));
  const sort = el(
    'select',
    { 'aria-label': 'sort', onchange: (e) => ((hubState.sort = e.target.value), loadHub()) },
    ['new', 'stars', 'downloads', 'featured'].map((x) => el('option', { value: x, selected: hubState.sort === x }, x)),
  );
  hubList = el('div.hub-list', el('p.dim', 'looking…'));
  loadHub();
  const sName = el('input', { type: 'text', placeholder: `name (${app.save || 'the open save'})`, 'aria-label': 'shared name' });
  const sDesc = el('input', { type: 'text', placeholder: 'what’s in it', 'aria-label': 'description' });
  const sTags = el('input', { type: 'text', placeholder: 'tags, comma separated', 'aria-label': 'tags' });
  return section(
    'the hub',
    el('div.row', q, sort),
    hubList,
    el('h4', 'share this save'),
    el('div.share', sName, sDesc, sTags),
    el(
      'div.row',
      el('button.primary', {
        type: 'button',
        onclick: async () => {
          try {
            const r = await app.api.send('share', {
              method: 'POST',
              headers: { 'content-type': 'application/json' },
              body: JSON.stringify({ name: sName.value.trim(), description: sDesc.value.trim(), tags: sTags.value.trim() }),
            });
            app.status(r.status === 'approved' ? 'shared on the hub' : 'shared: it waits for an admin');
          } catch (e) {
            app.fail(e);
          }
        },
      }, 'share this save'),
      el('span.dim.small', 'sends the open save to the hub under your name'),
    ),
  );
}

// ---------------------------------------------------------------- colours and the view

let colourRole = 'window';
function coloursSection() {
  const c = theme.colours();
  const presets = Object.keys(theme.PRESETS).map((p) =>
    el('button', { type: 'button', className: theme.preset() === p ? 'on' : '', onclick: () => (theme.choose(p), redraw()) }, p),
  );
  const roles = theme.ROLES.map(([name]) =>
    el(
      'button.role',
      { type: 'button', className: `role${colourRole === name ? ' on' : ''}`, onclick: () => ((colourRole = name), redraw()) },
      el('span.swatch', { style: { background: c[name] } }),
      name,
    ),
  );
  const pickColour = el('input', {
    type: 'color',
    value: c[colourRole],
    'aria-label': `the colour of ${colourRole}`,
    oninput: (e) => {
      theme.set(colourRole, e.target.value);
      hex.value = e.target.value;
    },
    onchange: () => redraw(),
  });
  const hex = el('input', {
    type: 'text',
    value: c[colourRole],
    'aria-label': 'hex',
    onchange: (e) => {
      if (/^#[0-9a-f]{6}$/i.test(e.target.value.trim())) {
        theme.set(colourRole, e.target.value.trim().toLowerCase());
        redraw();
      }
    },
  });
  return section(
    'colours',
    el('div.row', el('span.dim.small', 'start from'), presets),
    el('div.roles', roles),
    el('div.row', el('span.dim.small', colourRole), pickColour, hex, el('button.small', { type: 'button', onclick: () => (theme.reset(colourRole), redraw()) }, 'reset')),
    el('p.dim.small', 'the memory view keeps weum’s own colours'),
  );
}

function viewSection() {
  const p = app.prefs;
  const zen = el(
    'select',
    {
      'aria-label': 'zen',
      onchange: (e) => {
        p.zen = +e.target.value;
        app.savePrefs();
        app.wake();
      },
    },
    [[0, 'off'], [1, 'after 1 s'], [2.5, 'after 2.5 s'], [5, 'after 5 s'], [10, 'after 10 s']].map(([v, l]) => el('option', { value: v, selected: p.zen === v }, l)),
  );
  return section(
    'view',
    el('label.row', 'zen: fade the bars and panels away while the mouse rests', zen),
    el(
      'label.check',
      el('input', {
        type: 'checkbox',
        checked: p.searchLine,
        onchange: (e) => {
          p.searchLine = e.target.checked;
          app.savePrefs();
          app.emit('searchline');
        },
      }),
      'in the space, show the search line (the sums and their chain) in the top bar',
    ),
    el('p.dim.small', 'shortcuts: Ctrl+S save · Ctrl+E view/edit · Ctrl+B the note list · Ctrl+1..7 the views · Ctrl+, settings · / the command bar · Esc leaves it'),
  );
}

// ---------------------------------------------------------------- the overlay

async function redraw() {
  if (overlay.hidden) return;
  const scroll = panel.scrollTop;
  await loadSaves();
  clear(
    panel,
    el('div.set-head', el('h2', 'settings'), el('span.grow'), el('button.small', { type: 'button', onclick: () => close(), title: 'close (Esc)' }, 'close')),
    savesSection(),
    forgeSection(),
    hubSection(),
    coloursSection(),
    viewSection(),
  );
  panel.scrollTop = scroll;
}

function open() {
  overlay.hidden = false;
  document.body.classList.add('settings-open');
  document.getElementById('settings-button').classList.add('on');
  redraw();
}

function close() {
  overlay.hidden = true;
  document.body.classList.remove('settings-open');
  document.getElementById('settings-button').classList.remove('on');
}

export default {
  init(o, a) {
    app = a;
    overlay = o;
    panel = el('div.panel.card', { role: 'dialog', 'aria-label': 'settings' });
    overlay.append(panel);
    overlay.addEventListener('mousedown', (e) => e.target === overlay && close());
  },
  open,
  close,
  toggle: () => (overlay.hidden ? open() : close()),
  isOpen: () => !overlay.hidden,
};
