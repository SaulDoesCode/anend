// the editor's classifiers drawer: which classifiers a note is filed under (ticked here,
// kept per note), what each says of it (every class with its probability, the classes it
// counts it in, what its validators say), and a yes or no for each

import { el, clear } from '../util.js';
import { icon, iconButton, tip } from '../icons.js';

const pct = (x) => (x == null ? '–' : `${Math.round(x * 100)}%`);
const OPEN_KEY = 'anend.drawer';

export function drawer(app) {
  let key = ''; // the note's key (without note:)
  let data = null; // classify.indexed
  let busy = false;
  let asked = 0;
  let open = false;
  try {
    open = localStorage.getItem(OPEN_KEY) === 'open';
  } catch {
    /* private window */
  }

  const body = el('div.nd-body');
  const root = el(
    'aside.note-drawer',
    { hidden: !open, 'aria-label': 'classifiers on this note' },
    el('div.nd-head', icon('classifier'), el('h3', 'classifiers'), el('span.grow'), iconButton('refresh', 'read the note again', { small: true, cls: 'plain', onclick: () => load() }), iconButton('close', 'close the drawer', { small: true, cls: 'plain', key: 'Ctrl+I', onclick: () => toggle(false) })),
    body,
  );

  function toggle(on = !open) {
    open = on;
    root.hidden = !open;
    try {
      localStorage.setItem(OPEN_KEY, open ? 'open' : '');
    } catch {
      /* private window */
    }
    api.onToggle?.(open);
    if (open) load();
  }

  async function load() {
    if (!open) return;
    if (app.api.offline) {
      clear(body, el('p.dim.small', 'Classifiers need a hub for now: their engine runs on the hub (or anend-hub --local on your own machine), not yet in the browser.'));
      return;
    }
    if (!key) {
      data = null;
      clear(body, el('p.dim.small', 'Save the note first: then tick the classifiers to file it under, and see what each makes of it.'));
      return;
    }
    const me = ++asked;
    if (!data) clear(body, el('p.dim.small', 'reading…'));
    try {
      const d = await app.api.call('classify.indexed', { key: `note:${key}` });
      if (me !== asked) return;
      data = d;
    } catch (e) {
      if (me !== asked) return;
      clear(body, el('p.bad.small', e.message));
      return;
    }
    draw();
  }

  async function choose(name, on) {
    if (busy) return;
    busy = true;
    const names = new Set(data.chosen);
    if (on) names.add(name);
    else names.delete(name);
    try {
      await app.call('classify.set_index', { key: `note:${key}`, names: [...names] });
    } catch {
      /* reported */
    }
    busy = false;
    load();
  }

  async function judge(v, cls, yes) {
    try {
      const r = await app.call('classify.feedback', { name: v.name, key: `note:${key}`, class: cls, yes });
      const e = r.entry;
      app.status(`${v.name}: ${yes ? 'yes' : 'no'}, “${e.class}” ${pct(e.before)} → ${pct(e.after)}`);
    } catch {
      return;
    }
    load();
  }

  function verdict(v) {
    if (!v.trained) return el('p.dim.small', 'not trained yet');
    const sure = v.valid !== false && v.members.length > 0;
    const probs = v.checked || v.probs;
    return el(
      'div.nd-verdict',
      el(
        'div.verdict',
        v.classes.map((c, i) =>
          tip(
            el('span.vchip', { className: `vchip${v.valid !== false && v.members.includes(c) ? ' in' : ''}${i === v.top ? ' top' : ''}`, style: { '--c': v.palette?.[i] || 'var(--accent)', '--p': probs[i] } }, el('span.lab', c), el('span.p', pct(probs[i]))),
            `${v.name}: ${c} ${pct(probs[i])}${v.members.includes(c) ? ' (counted in)' : ''}${v.checked && Math.abs(v.checked[i] - v.probs[i]) > 0.005 ? ` · ${pct(v.probs[i])} before its validators` : ''}`,
          ),
        ),
      ),
      v.validators?.length
        ? el(
            'div.nd-checks.small',
            v.validators.map(([by, p]) => el('span', { className: v.valid ? 'pass' : 'fail' }, icon(v.valid ? 'shield' : 'alert'), ` ${by} ${pct(p)}`)),
            v.valid ? null : el('span.dim', ' · not passed'),
          )
        : null,
      el(
        'div.row.nd-judge',
        el('span.small', sure ? el('b', v.class) : el('span.dim', `unsure (most like ${v.classes[v.top]})`)),
        el('span.grow'),
        iconButton('check', `yes, it is ${v.classes[v.top]}`, { small: true, cls: 'yes', onclick: () => judge(v, v.top, true) }),
        iconButton('close', `no, it isn't ${v.classes[v.top]}`, { small: true, cls: 'no', onclick: () => judge(v, v.top, false) }),
      ),
    );
  }

  function draw() {
    const d = data;
    if (!d.classifiers.length) {
      clear(body, el('p.dim.small', 'No classifiers in this save yet: make one in the classifiers view (Ctrl+7).'));
      return;
    }
    const byName = new Map(d.verdicts.map((v) => [v.name, v]));
    clear(
      body,
      el('p.dim.small.nd-about', 'Tick the classifiers to file this note under: they read it now and whenever it changes.'),
      el(
        'ul.nd-list',
        d.classifiers.map((c) => {
          const box = el('input', { type: 'checkbox', checked: c.chosen, onchange: () => choose(c.name, box.checked) });
          const v = byName.get(c.name);
          return el(
            'li',
            { className: c.chosen ? 'on' : '' },
            el('label.check.nd-name', box, el('span', c.name), c.trained ? null : el('span.dim.small', 'untrained')),
            c.chosen && v ? verdict(v) : null,
          );
        }),
      ),
    );
  }

  const api = {
    el: root,
    get open() {
      return open;
    },
    toggle,
    // the note shown now ('' while it isn't saved)
    setKey(k) {
      if (k === key) return;
      key = k || '';
      data = null;
      load();
    },
    refresh: () => load(),
    onToggle: null,
  };
  return api;
}
