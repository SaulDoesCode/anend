// colouring texts by a classifier (classify.tints): each text takes its top class's
// colour, as strong as the classifier is sure, and stays neutral when it is in no class.
// The memory view and the space each keep their own choice (off unless picked).

import { el, clear } from './util.js';
import { icon, tip } from './icons.js';

export const UNCERTAIN = '#8a8580';

// a picker, its legend and the tints for a view: tinter(app, 'memory', redraw)
export function tinter(app, view, onChange) {
  const KEY = `anend.tint.${view}`;
  let name = '';
  try {
    name = localStorage.getItem(KEY) || '';
  } catch {
    /* private window */
  }
  let classifiers = []; // trained ones' names
  let classes = [];
  let palette = [];
  let tints = new Map(); // key -> {class, strength}
  let asked = 0;

  const select = el('select.tint-select', {
    'aria-label': 'colour by classifier',
    onchange: () => {
      name = select.value;
      try {
        localStorage.setItem(KEY, name);
      } catch {
        /* private window */
      }
      tints = new Map();
      classes = [];
      onChange();
    },
  });
  const picker = tip(el('label.tint-picker', icon('palette'), select), 'colour the texts by a classifier: each by its top class, as strong as it is sure');
  const legend = el('div.tint-legend', { hidden: true });

  function drawSelect() {
    clear(
      select,
      el('option', { value: '' }, 'no colours'),
      classifiers.map((c) => el('option', { value: c, selected: c === name }, c)),
      name && !classifiers.includes(name) ? el('option', { value: name, selected: true }, `${name} (gone)`) : null,
    );
    picker.hidden = app.api.offline || (!classifiers.length && !name);
  }

  function drawLegend(counted) {
    if (!name || !classes.length) {
      legend.hidden = true;
      clear(legend);
      return;
    }
    legend.hidden = false;
    const n = counted || new Map();
    clear(
      legend,
      el('span.tint-name', name),
      classes.map((c, i) =>
        el('span.tint-key', el('span.sw', { style: { background: palette[i] } }), c, n.has(i) ? el('span.n', n.get(i)) : null),
      ),
      el('span.tint-key.unsure', el('span.sw', { style: { background: UNCERTAIN } }), 'unsure', n.has(null) ? el('span.n', n.get(null)) : null),
      el('span.tint-ramp', { title: 'paler: less sure' }, el('span.sw.ramp'), 'surer'),
    );
  }

  return {
    picker,
    legend,
    get on() {
      return !!name && !app.api.offline;
    },
    // the trained classifiers, for the picker
    async refresh() {
      if (app.api.offline) {
        picker.hidden = true;
        legend.hidden = true;
        return;
      }
      try {
        classifiers = (await app.api.call('classify.list')).filter((c) => c.trained).map((c) => c.name);
      } catch {
        classifiers = [];
      }
      drawSelect();
    },
    // the tints of these keys (a Map key -> {colour, strength}); null when off
    async load(keys) {
      if (!this.on || !keys.length) {
        drawLegend();
        return null;
      }
      const want = [...new Set(keys)].slice(0, 5000);
      const me = ++asked;
      let r;
      try {
        r = await app.api.call('classify.tints', { name, keys: want });
      } catch (e) {
        if (me === asked) app.status(`colours by “${name}”: ${e.message}`, true);
        return null;
      }
      if (me !== asked) return null;
      classes = r.classes;
      palette = r.palette;
      tints = new Map();
      const counted = new Map();
      for (const t of r.tints) {
        const unsure = t.class == null;
        tints.set(t.key, { colour: unsure ? UNCERTAIN : palette[t.class] || UNCERTAIN, strength: unsure ? 0.25 : Math.max(0.15, Math.min(1, t.strength ?? 0)), class: t.class, label: unsure ? 'unsure' : classes[t.class] });
        counted.set(t.class ?? null, (counted.get(t.class ?? null) || 0) + 1);
      }
      drawLegend(counted);
      return tints;
    },
    get(key) {
      return tints.get(key);
    },
  };
}
