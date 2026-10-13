// the app's shell: the top bar, the command bar every view shares, the routes, the
// shortcuts, zen, and the page shown when the hub wants a login

import { api } from './api.js';
import { el, clear, typing } from './util.js';
import * as theme from './theme.js';
import editor from './views/editor.js';
import reader from './views/reader.js';
import memory from './views/memory.js';
import stats from './views/stats.js';
import space from './views/space.js';
import lists from './views/lists.js';
import classifiers from './views/classify.js';
import settings from './views/settings.js';

const VIEWS = { editor, reader, memory, stats, space, lists, classifiers };
const ORDER = ['editor', 'reader', 'memory', 'stats', 'space', 'lists', 'classifiers'];

const $ = (id) => document.getElementById(id);
const barInput = $('bar-input');

// what the views share
export const app = {
  api,
  view: null,
  save: '',
  counts: { notes: 0, memories: 0 },
  barText: {},
  listeners: {},

  on(ev, f) {
    (this.listeners[ev] ||= []).push(f);
  },
  emit(ev, data) {
    for (const f of this.listeners[ev] || []) f(data);
  },

  // a message in the top bar; errors stay a while longer
  status(msg, bad = false) {
    const s = $('status');
    s.textContent = msg || '';
    s.classList.toggle('bad', !!bad);
    clearTimeout(this.statusTimer);
    const ms = bad ? 7000 : 3500;
    this.statusUntil = Date.now() + ms;
    this.statusTimer = setTimeout(() => this.idleStatus(), ms + 20);
  },
  // the open save and its counts, once a message has had its time
  idleStatus() {
    if (Date.now() < (this.statusUntil || 0)) return;
    const s = $('status');
    s.classList.remove('bad');
    const where = api.offline ? (api.kept ? ' · offline · in this browser' : ' · offline · not kept (no IndexedDB here)') : '';
    s.textContent = this.save ? `${this.save}: ${this.counts.notes} notes, ${this.counts.memories} memories${where}` : where.slice(3);
  },
  fail(e) {
    app.status(e && e.message ? e.message : String(e), true);
  },

  // calls that report their errors in the top bar
  async call(op, args) {
    try {
      return await api.call(op, args);
    } catch (e) {
      app.fail(e);
      throw e;
    }
  },

  go(route) {
    if (location.hash !== `#/${route}`) location.hash = `#/${route}`;
    else route_();
  },

  // the command bar
  bar: {
    get: () => barInput.value,
    set(t) {
      barInput.value = t;
      app.barText[app.view] = t;
    },
    focus() {
      barInput.focus();
      barInput.select();
    },
  },

  async hello() {
    try {
      const h = await api.call('hello');
      app.save = h.save;
      app.counts = h.counts;
      app.idleStatus();
      return h;
    } catch (e) {
      app.fail(e);
    }
  },

  // after something changed which save is open
  async saveChanged() {
    await app.hello();
    app.emit('save');
    VIEWS[app.view]?.show?.(app.route || []);
  },
};

// ------------------------------------------------------------------ preferences

const ZEN_KEY = 'anend.view';
export const prefs = (() => {
  try {
    return { zen: 0, searchLine: true, ...JSON.parse(localStorage.getItem(ZEN_KEY) || '{}') };
  } catch {
    return { zen: 0, searchLine: true };
  }
})();
app.prefs = prefs;
app.savePrefs = () => {
  try {
    localStorage.setItem(ZEN_KEY, JSON.stringify(prefs));
  } catch {
    /* private window */
  }
};

// ------------------------------------------------------------------ the top bar

const tabs = $('tabs');
for (const name of ORDER) {
  tabs.append(el('button.tab', { type: 'button', dataset: { view: name }, title: name, onclick: () => app.go(name) }, name));
}
$('settings-button').addEventListener('click', () => settings.toggle());

// ------------------------------------------------------------------ the views

const main = $('views');
for (const name of ORDER) {
  const section = el(`section.view.view-${name}`, { hidden: true, id: `view-${name}` });
  main.append(section);
  VIEWS[name].init(section, app);
}
settings.init($('settings'), app);
app.on('searchline', () => space.redrawLine());

function route_() {
  const parts = location.hash.replace(/^#\/?/, '').split('/').map(decodeURIComponent);
  const name = VIEWS[parts[0]] ? parts[0] : 'editor';
  const rest = parts.slice(1);
  if (app.view !== name) {
    if (app.view) {
      app.barText[app.view] = barInput.value;
      VIEWS[app.view].hide?.();
      $(`view-${app.view}`).hidden = true;
    }
    app.view = name;
    document.body.dataset.view = name;
    $(`view-${name}`).hidden = false;
    for (const b of tabs.children) b.classList.toggle('on', b.dataset.view === name);
    const v = VIEWS[name];
    $('bar-name').textContent = v.barName || name;
    barInput.placeholder = v.barPlaceholder || '';
    barInput.value = app.barText[name] ?? '';
    clear($('bar-right'), v.barRight || null);
    $('searchline').hidden = name !== 'space' || !space.searchLineOn();
  }
  app.route = rest;
  VIEWS[name].show?.(rest);
}
window.addEventListener('hashchange', route_);

barInput.addEventListener('input', () => {
  app.barText[app.view] = barInput.value;
  VIEWS[app.view].barInput?.(barInput.value);
});
barInput.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    barInput.blur();
    VIEWS[app.view].barEscape?.();
    e.preventDefault();
    return;
  }
  if (VIEWS[app.view].barKey?.(e)) e.preventDefault();
});

// ------------------------------------------------------------------ shortcuts

window.addEventListener('keydown', (e) => {
  const ctrl = e.ctrlKey || e.metaKey;
  if (ctrl && e.key === ',') {
    e.preventDefault();
    settings.toggle();
    return;
  }
  if (ctrl && /^[1-7]$/.test(e.key)) {
    e.preventDefault();
    app.go(ORDER[+e.key - 1]);
    return;
  }
  if (e.key === 'Escape' && settings.isOpen()) {
    e.preventDefault();
    settings.close();
    return;
  }
  if (settings.isOpen()) return;
  if (VIEWS[app.view].key?.(e)) {
    e.preventDefault();
    return;
  }
  if (!typing(e) && !ctrl && !e.altKey && e.key === '/') {
    e.preventDefault();
    barInput.focus();
  }
});

// ------------------------------------------------------------------ zen

let zenTimer;
function wake() {
  document.body.classList.remove('zen');
  clearTimeout(zenTimer);
  if (prefs.zen > 0) zenTimer = setTimeout(() => document.body.classList.add('zen'), prefs.zen * 1000);
}
app.wake = wake;
for (const ev of ['mousemove', 'pointerdown', 'wheel', 'touchstart']) window.addEventListener(ev, wake, { passive: true });

// ------------------------------------------------------------------ connection

// no hub behind the page: the saves live in this browser
document.body.classList.toggle('offline', !!api.offline);
if (api.offline) $('status').title = 'no hub served this page: the app runs here, its saves kept in this browser (IndexedDB)';

api.addEventListener('state', (e) => {
  if (String(e.detail).startsWith('not kept')) app.status(`couldn’t keep that in this browser (${e.detail.slice(10)})`, true);
  else if (e.detail === 'reconnecting') app.status('reconnecting…');
  else if (e.detail === 'offline') app.status("can't reach the hub", true);
  else if (e.detail === 'online') app.idleStatus();
});

let loginShown = false;
api.addEventListener('login', (e) => {
  if (loginShown) return;
  loginShown = true;
  document.body.classList.add('login');
  const waiting = /waiting/.test(e.detail);
  clear(
    main,
    el(
      'section.view.login-page',
      el(
        'div.card',
        el('h1', 'anend'),
        waiting
          ? el('p', 'Your account is waiting for an admin to let you in. Once they have, the app opens here.')
          : el('p', 'The app keeps your notes and memories on this hub, in saves of your own. Log in to use it here, or join if you are new.'),
        el('p.row', el('a.btn.primary', { href: '/login' }, 'log in'), el('a.btn', { href: '/join' }, 'join'), el('a', { href: '/' }, 'the hub')),
      ),
    ),
  );
});

// ------------------------------------------------------------------ start

theme.apply();
wake();
route_();
app.hello();
