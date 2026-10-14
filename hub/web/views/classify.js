// the classifiers: made from lists (each list a class), trained with a live progress bar,
// steered by the player's sliders with the bias metric beside them, searched by a
// class's probability with a yes or no for each classifier that had a part in a result,
// their outputs written back as lists, and texts near each other by what they represent

import { el, clear, confirmClick, debounce } from '../util.js';
import { iconButton, icon, tip } from '../icons.js';
import * as more from './clsmore.js';

let app, body, count;
let selected = null; // the open classifier's name
let detail = null; // its classify.get
let all = []; // classify.list
let lists = []; // lists.stats' lists
let poll = null;
let visible = false;
let searchClass = 0;
let query = '';
let results = null; // classify.search
let why = new Map(); // result key -> contributions
let similar = null;
let biasReport = null;
let enlisted = null;

const pct = (x) => (x == null ? '–' : `${(x * 100).toFixed(1)}%`);
const num = (x, d = 3) => (x == null || !isFinite(x) ? '–' : (+x).toFixed(d));
const SEL_KEY = 'anend.classifier';
const TAB_KEY = 'anend.classifier.tab';
// the open classifier's parts, one at a time
const TABS = [
  ['train', 'sliders', 'learn', 'training and steering'],
  ['use', 'search', 'use', 'search by class, outputs as lists, near by meaning'],
  ['buckets', 'bucket', 'buckets', 'every text filed by class and sureness, and near texts'],
  ['checks', 'shield', 'checks', 'validators: classifiers that check this one, and where they disagree'],
  ['share', 'export', 'share', 'download it as a file'],
];
let tab = (() => {
  try {
    const t = localStorage.getItem(TAB_KEY);
    return TABS.some((x) => x[0] === t) ? t : 'train';
  } catch {
    return 'train';
  }
})();
function setTab(t) {
  tab = t;
  try {
    localStorage.setItem(TAB_KEY, t);
  } catch {
    /* private window */
  }
  draw();
}

function remember(name) {
  selected = name;
  try {
    localStorage.setItem(SEL_KEY, name || '');
  } catch {
    /* private window */
  }
}

async function load() {
  if (app.api.offline) {
    clear(
      body,
      el(
        'div.card.clf-offline',
        el('h2', 'classifiers'),
        el('p', 'Classifiers need a hub for now: their engine runs on the hub (or anend-hub --local on your own machine), not yet in the browser.'),
        el('p.dim.small', 'Saves you bring here from a hub keep their classifiers, ready for when they go back.'),
      ),
    );
    count.textContent = '';
    return;
  }
  try {
    [all, lists] = await Promise.all([app.call('classify.list'), app.call('lists.stats').then((s) => s.lists)]);
  } catch {
    return;
  }
  if (!selected) {
    try {
      selected = localStorage.getItem(SEL_KEY) || null;
    } catch {
      /* none */
    }
  }
  if (selected && !all.some((c) => c.name === selected)) selected = null;
  if (!selected && all.length) selected = all[0].name;
  count.textContent = `${all.length} classifier${all.length === 1 ? '' : 's'}`;
  await open(selected, true);
}

async function open(name, keep = false) {
  if (name !== detail?.name || !keep) {
    results = null;
    why = new Map();
    similar = null;
    biasReport = null;
    enlisted = null;
    searchClass = 0;
  }
  remember(name);
  more.reset(name);
  let d = null;
  if (name) {
    try {
      d = await app.call('classify.get', { name });
    } catch {
      d = null;
    }
  }
  if (selected !== name) return; // another was opened meanwhile
  detail = d;
  draw();
  watch();
}

// ------------------------------------------------------------------ drawing

function draw() {
  const y = body.parentElement?.scrollTop;
  clear(
    body,
    el('div.clf-side', listCard(), newCard(), more.importCard()),
    el(
      'div.clf-main',
      more.importing()
        ? more.importPanel()
        : detail
        ? [headCard(), tabBar(), ...[].concat(tabCards())]
        : el('div.card.clf-none', el('span.big', icon('classifier')), el('p', all.length ? 'pick a classifier' : 'no classifiers yet'), el('p.dim.small', 'make one from two or more lists (each list a class), or bring one in from a .anendcls file')),
    ),
  );
  if (y && body.parentElement) body.parentElement.scrollTop = y;
}

function tabBar() {
  return el(
    'nav.clf-tabs',
    { role: 'tablist', 'aria-label': 'the classifier’s parts' },
    TABS.map(([t, ic, label, about]) =>
      tip(
        el('button', { type: 'button', role: 'tab', className: tab === t ? 'on' : '', 'aria-selected': String(tab === t), onclick: () => setTab(t) }, icon(ic), el('span.lab', label)),
        about,
      ),
    ),
  );
}

function tabCards() {
  if (tab === 'use') return [searchCard(), outputCard(), similarCard()];
  if (tab === 'buckets') return more.bucketsCards();
  if (tab === 'checks') return more.checksCards();
  if (tab === 'share') return more.shareCards();
  return [trainCard(), steerCard()];
}

function listCard() {
  return el(
    'div.card.clf-list',
    el('h2', 'classifiers'),
    all.length
      ? el(
          'ul.clf-names',
          all.map((c) =>
            el(
              'li',
              { className: c.name === selected ? 'on' : '', onclick: () => open(c.name) },
              el('span.name', c.name),
              el('span.dim.small', c.training ? 'training…' : c.trained ? pct(c.accuracy) : 'untrained'),
            ),
          ),
        )
      : el('p.dim.small', 'none yet'),
  );
}

function newCard() {
  const name = el('input', { type: 'text', placeholder: 'name', 'aria-label': 'new classifier name' });
  const preset = el('select', { 'aria-label': 'preset' }, ['tiny', 'compact', 'small', 'base'].map((p) => el('option', { value: p, selected: p === 'compact' }, p)));
  const head = el('select', { 'aria-label': 'head' }, el('option', { value: 'single' }, 'one class'), el('option', { value: 'multi' }, 'labels'));
  const usable = lists.filter((l) => l.count > 0);
  const boxes = usable.map((l) => ({ l, box: el('input', { type: 'checkbox', value: l.list }) }));
  const not = el('select', { 'aria-label': 'not list' }, el('option', { value: '' }, 'no “not” list'), usable.map((l) => el('option', { value: l.list }, l.name)));
  const ins = all.filter((c) => c.trained).map((c) => ({ c, box: el('input', { type: 'checkbox', value: c.name }) }));
  const make = async () => {
    const classes = boxes.filter((b) => b.box.checked).map((b) => b.l.list);
    try {
      const d = await app.call('classify.create', {
        name: name.value,
        classes,
        not: not.value || undefined,
        preset: preset.value,
        head: head.value,
        inputs: ins.filter((i) => i.box.checked).map((i) => i.c.name),
      });
      app.status(`made “${d.name}”: train it next`);
      remember(d.name);
      load();
    } catch {
      /* reported */
    }
  };
  return el(
    'div.card.clf-new',
    el('h3', 'new classifier'),
    usable.length < 1
      ? el('p.dim.small', 'make some lists first (the lists view): each list becomes a class')
      : [
          el('div.row', name),
          el('p.dim.small', 'its classes: one per list'),
          el(
            'div.clf-checks',
            boxes.map(({ l, box }) => el('label.check', box, ` ${l.name} `, el('span.dim.small', l.count))),
          ),
          el('div.row', not),
          ins.length ? [el('p.dim.small', 'builds on (their outputs as inputs)'), el('div.clf-checks', ins.map(({ c, box }) => el('label.check', box, ` ${c.name}`)))] : null,
          el('div.row', preset, head, iconButton('new', 'make the classifier', { text: 'make', cls: 'primary', onclick: make })),
        ],
  );
}

function headCard() {
  const d = detail;
  const del = confirmClick(iconButton('delete', 'delete this classifier (click twice)', { small: true }), 'delete?', async () => {
    try {
      await app.call('classify.delete', { name: d.name });
      app.status(`deleted “${d.name}”`);
      selected = null;
      load();
    } catch {
      /* reported */
    }
  });
  const size = d.size;
  return el(
    'div.card.clf-head',
    el(
      'header.row',
      el('h2', d.name),
      el('span.dim.small', `${d.head === 'multi' ? 'labels' : 'one class'} · ${d.preset}${size ? ` · ${size.weights.toLocaleString()} weights, ${size.layers} blocks` : ''} · version ${d.version}`),
      el('span.grow'),
      del,
    ),
    el(
      'div.members',
      d.classes.map((c, i) =>
        el('span.member.clf-class', { title: d.lists[i] ? `list ${d.lists[i]}` : 'the “not” list' }, c, d.outputs[c] ? el('span.dim.small', ` → ${d.outputs[c]}`) : null),
      ),
    ),
    d.inputs.length
      ? el(
          'div.row.small',
          el('span.dim', 'builds on'),
          d.inputs.map((i) => couple(i)),
        )
      : null,
  );
}

// an upstream link's coupling: how much end-to-end feedback reaches it
function couple(i) {
  const out = el('span.dim', num(i.coupling, 2));
  const r = el('input', {
    type: 'range', min: 0, max: 2, step: 0.05, value: i.coupling, 'aria-label': `coupling to ${i.from}`,
    oninput: () => (out.textContent = num(+r.value, 2)),
    onchange: async () => {
      try {
        await app.call('classify.couple', { name: detail.name, from: i.from, coupling: +r.value });
      } catch {
        /* reported */
      }
    },
  });
  return el('label.clf-couple', el('a', { href: '#', onclick: (e) => (e.preventDefault(), open(i.from)) }, i.from), r, out);
}

// ------------------------------------------------------------------ training

function trainCard() {
  const d = detail;
  const job = d.job;
  const running = job && job.state === 'running';
  const epochs = el('input', { type: 'number', min: 1, max: 1000, value: d.train.epochs, 'aria-label': 'epochs', title: 'epochs' });
  const rounds = el('input', { type: 'number', min: 1, max: 50, value: 1, 'aria-label': 'growth rounds', title: 'rounds: above 1, it may widen, deepen or prune between them' });
  const fresh = el('input', { type: 'checkbox' });
  const go = async () => {
    try {
      await app.call('classify.train', { name: d.name, epochs: +epochs.value, rounds: +rounds.value, fresh: fresh.checked });
      await open(d.name, true);
    } catch {
      /* reported */
    }
  };
  const cancel = async () => {
    try {
      await app.call('classify.cancel', { name: d.name });
    } catch {
      /* reported */
    }
  };
  const last = [...d.history].reverse().find((e) => e.kind === 'train');
  const m = (job && job.state === 'done' && job.summary?.metrics) || last?.metrics;
  const growth = d.history.filter((e) => e.kind === 'growth' || e.kind === 'enlist' || e.kind === 'feedback stack').slice(-8).reverse();
  return el(
    'div.card.clf-train',
    el(
      'div.row',
      el('h3', 'training'),
      el('span.grow'),
      running
        ? iconButton('close', 'stop the training', { text: 'stop', onclick: cancel })
        : [
            el('label.small.dim', 'epochs ', epochs),
            el('label.small.dim', 'rounds ', rounds),
            el('label.small.dim', fresh, ' from scratch'),
            iconButton('play', d.trained ? 'train it more (on top of what it learned)' : 'train it', { text: d.trained ? 'train more' : 'train', cls: 'primary', onclick: go }),
          ],
    ),
    job ? progress(job) : null,
    m
      ? el(
          'div.clf-metrics',
          el(
            'div.stats',
            el('div.stat', el('b', pct(m.accuracy)), el('span', 'accuracy')),
            el('div.stat', el('b', num(m.macro_f1)), el('span', 'macro-F1')),
            el('div.stat', el('b', num(m.log_loss)), el('span', 'log-loss')),
            el('div.stat', el('b', m.n), el('span', `texts ${last?.measured_on || ''}`)),
          ),
          confusion(m, d.classes),
        )
      : el('p.dim.small', d.trained ? '' : 'not trained yet: it learns its lists’ texts, then tells any text which list it belongs with'),
    growth.length ? el('ul.clf-history.small', growth.map((e) => el('li', el('span.dim', `${e.kind} · `), e.note))) : null,
    last ? el('p.dim.small', `last run: ${last.note}, ${num(last.seconds, 1)} s`) : null,
  );
}

function progress(job) {
  const frac = job.steps ? job.step / job.steps : 0;
  const ev = job.evals[job.evals.length - 1];
  const label =
    job.state === 'running'
      ? `${job.rounds > 1 ? `round ${job.round + 1}/${job.rounds} · ` : ''}epoch ${job.epoch + 1} · step ${job.step}/${job.steps} · loss ${num(job.loss)} · ${Math.round(job.tokens_per_sec)} tokens/s`
      : job.state === 'failed'
        ? `failed: ${job.error}`
        : `${job.state} in ${num(job.seconds, 1)} s${job.summary ? ` · ${job.summary.steps} steps on ${job.summary.examples} texts` : ''}`;
  return el(
    'div.clf-progress',
    el('div.track', el('div.fill', { style: { width: `${(job.state === 'running' ? frac : 1) * 100}%` }, className: `fill ${job.state}` })),
    el('div.row.small', el('span', label), ev ? el('span.dim', `· held out: ${pct(ev.accuracy)}, F1 ${num(ev.macro_f1)}`) : null),
    job.losses.length > 2 ? spark(job.losses) : null,
  );
}

// the training loss as a line
function spark(points) {
  const w = 300;
  const h = 40;
  const xs = points.map((p) => p[0]);
  const ys = points.map((p) => p[1]);
  const [x0, x1] = [Math.min(...xs), Math.max(...xs) || 1];
  const [y0, y1] = [Math.min(...ys), Math.max(...ys)];
  const d = points
    .map((p, i) => `${i ? 'L' : 'M'}${(((p[0] - x0) / (x1 - x0 || 1)) * w).toFixed(1)},${(h - ((p[1] - y0) / (y1 - y0 || 1)) * (h - 4) - 2).toFixed(1)}`)
    .join('');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', `0 0 ${w} ${h}`);
  svg.setAttribute('class', 'spark');
  svg.setAttribute('role', 'img');
  svg.setAttribute('aria-label', `training loss from ${num(ys[0])} to ${num(ys[ys.length - 1])}`);
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.setAttribute('d', d);
  svg.append(path);
  return svg;
}

function confusion(m, classes) {
  if (!m.confusion || !m.confusion.length) return null;
  const max = Math.max(1, ...m.confusion.flat());
  return el(
    'div.dot-scroll',
    el(
      'table.dot.clf-confusion',
      el('caption.dim.small', 'truth ↓ · predicted →'),
      el('tr', el('th', ''), classes.map((c) => el('th.col-name', { title: c }, el('span', c)))),
      m.confusion.map((row, i) =>
        el(
          'tr',
          el('th.row-name', { title: classes[i] }, classes[i]),
          row.map((v, j) => el('td.cell', { className: `cell${v / max > 0.6 ? ' strong' : ''}${i === j ? ' diag' : ''}`, style: { '--v': v / max }, title: `${classes[i]} → ${classes[j]}: ${v}` }, v)),
        ),
      ),
    ),
  );
}

// ------------------------------------------------------------------ steering

function steerCard() {
  const d = detail;
  const st = JSON.parse(JSON.stringify(d.steer));
  const h = st.heads[0];
  const report = el('div.clf-bias');
  const show = (b) => {
    biasReport = b;
    clear(report, biasView(b, d.classes));
  };
  if (biasReport) show(biasReport);
  // one call at a time, the latest settings winning, so a held arrow key can't queue
  // up a backlog of bias reports
  let busy = false;
  let wanted = null; // null, or whether the next call saves
  const send = async (save) => {
    if (busy) {
      wanted = wanted === true || save;
      return;
    }
    busy = true;
    try {
      const r = await app.call('classify.steer', { name: d.name, steer: { bias: h.bias, temperature: h.temperature, threshold: h.threshold, gains: st.gains }, save });
      show(r.bias);
      if (save) d.steer = r.steer;
    } catch {
      /* reported */
    } finally {
      busy = false;
      if (wanted !== null) {
        const w = wanted;
        wanted = null;
        send(w);
      }
    }
  };
  const live = debounce(() => send(false), 120);
  const keep = debounce(() => send(true), 300);
  const slider = (label, min, max, step, get, set, title) => {
    const out = el('span.val', num(get(), 2));
    const r = el('input', {
      type: 'range', min, max, step, value: get(), 'aria-label': label, title,
      oninput: () => {
        set(+r.value);
        out.textContent = num(+r.value, 2);
        live();
      },
      onchange: keep,
    });
    return el('label.slider', el('span.lab', label), r, out);
  };
  const reset = async () => {
    try {
      await app.call('classify.steer', { name: d.name, steer: { reset: true } });
      biasReport = null;
      open(d.name, true);
    } catch {
      /* reported */
    }
  };
  if (d.trained && !biasReport) send(false);
  return el(
    'div.card.clf-steer',
    el('div.row', el('h3', 'steering'), el('span.dim.small', 'yours: training never touches it'), el('span.grow'), iconButton('undo', 'reset the steering', { small: true, onclick: reset })),
    el(
      'div.clf-sliders',
      el('div', el('h4', 'bias per class'), d.classes.map((c, i) => slider(c, -8, 8, 0.1, () => h.bias[i], (v) => (h.bias[i] = v), 'added to the class’s logit'))),
      el(
        'div',
        el('h4', 'reading'),
        slider('temperature', 0.2, 5, 0.05, () => h.temperature, (v) => (h.temperature = v), 'above 1 softens, below 1 sharpens'),
        slider('threshold', 0, 1, 0.01, () => h.threshold, (v) => (h.threshold = v), 'how sure it must be to count a text in'),
        el('h4', 'channel gains'),
        (d.inputs.length ? ['letter', 'word', 'side', 'extra'] : ['letter', 'word', 'side']).map((g) => slider(g, 0, 2, 0.05, () => st.gains[g], (v) => (st.gains[g] = v), `the ${g} input channel`)),
      ),
      report,
    ),
  );
}

function biasView(b, classes) {
  if (!b) return el('p.dim.small', 'the bias metric shows once it’s trained');
  return [
    el('h4', 'bias'),
    el(
      'div.stats',
      el('div.stat', el('b', pct(b.flipped)), el('span', 'decisions flipped')),
      el('div.stat', el('b', num(b.mean_kl)), el('span', 'mean KL (nats)')),
      el('div.stat', el('b', num(b.prior_skew)), el('span', 'class skew')),
    ),
    el(
      'div.clf-shift',
      classes.map((c, i) => {
        const v = b.shift[i] || 0;
        return el(
          'div.shift',
          { title: `${c}: ${v >= 0 ? '+' : ''}${(v * 100).toFixed(1)} points on average` },
          el('span.lab', c),
          el('span.track', el('span.bar', { className: `bar ${v < 0 ? 'neg' : 'pos'}`, style: { width: `${Math.min(50, Math.abs(v) * 100)}%` } })),
          el('span.val', `${v >= 0 ? '+' : ''}${(v * 100).toFixed(1)}`),
        );
      }),
    ),
    el('p.dim.small', `over ${b.texts} of its texts; prior ${b.prior.map((p) => pct(p)).join(' / ')}`),
  ];
}

// ------------------------------------------------------------------ search

async function search() {
  if (!detail) return;
  try {
    results = await app.call('classify.search', { name: detail.name, class: searchClass, q: query, limit: 40 });
    why = new Map();
  } catch {
    results = null;
  }
  draw();
}

async function judge(name, key, cls, yes, stack = false) {
  try {
    const r = await app.call(stack ? 'classify.feedback_stack' : 'classify.feedback', { name, key, class: cls, yes });
    const e = r.entry;
    app.status(`${name}: ${yes ? 'yes' : 'no'}, “${e.class}” ${pct(e.before)} → ${pct(e.after)}${stack ? ` (${r.touched.join(', ')})` : ''}`);
    detail = await app.call('classify.get', { name: detail.name });
    biasReport = null;
    await search();
  } catch {
    /* reported */
  }
}

async function explain(r) {
  try {
    why.set(r.key, await app.call('classify.contributions', { name: detail.name, key: r.key, class: searchClass }));
  } catch {
    return;
  }
  draw();
}

function searchCard() {
  const d = detail;
  const cls = el('select', { 'aria-label': 'class', onchange: () => ((searchClass = +cls.value), search()) }, d.classes.map((c, i) => el('option', { value: i, selected: i === searchClass }, c)));
  const q = el('input', { type: 'search', placeholder: 'only texts with these words (optional)', value: query, 'aria-label': 'words', onkeydown: (e) => e.key === 'Enter' && ((query = q.value), search()) });
  const stacked = d.inputs.length > 0;
  return el(
    'div.card.clf-search',
    el('div.row', el('h3', 'search by class'), cls, q, iconButton('search', 'search', { key: 'Enter', cls: 'primary', onclick: () => ((query = q.value), search()) })),
    !d.trained
      ? el('p.dim.small', 'train it first')
      : !results
        ? el('p.dim.small', 'memories and notes, most likely first; tell it yes or no on each')
        : el(
            'ol.clf-results',
            results.results.map((r) => {
              const w = why.get(r.key);
              return el(
                'li',
                el(
                  'div.res',
                  el('span.prob', { style: { '--p': r.prob } }, pct(r.prob)),
                  el('span.text', { title: r.key }, r.note ? el('span.dim', 'note · ') : null, r.text),
                  r.top_class !== results.class ? el('span.dim.small', `(${r.top_class})`) : null,
                  el('span.grow'),
                  iconButton('check', `yes, it is ${results.class}`, { small: true, cls: 'yes', onclick: () => judge(d.name, r.key, searchClass, true) }),
                  iconButton('close', `no, it isn't ${results.class}`, { small: true, cls: 'no', onclick: () => judge(d.name, r.key, searchClass, false) }),
                  stacked ? iconButton('help', 'why: which classifiers had a part in this', { small: true, onclick: () => explain(r) }) : null,
                ),
                w ? whyView(w, r) : null,
              );
            }),
          ),
    results ? el('p.dim.small', `${results.scanned} texts looked through · ${results.stack.join(' → ')}`) : null,
  );
}

// each classifier's part in a result, and a yes or no for each on what it said
function whyView(w, r) {
  const d = detail;
  return el(
    'div.clf-why',
    w.results.map((c) =>
      el(
        'div.row.small',
        el('span.share', { style: { '--p': c.share } }, pct(c.share)),
        el('b', c.name),
        el('span.dim', `says ${c.top_class} (${pct(c.probs[c.top])})`),
        el('span.grow'),
        iconButton('check', `yes: ${c.name} was right`, { small: true, cls: 'yes', onclick: () => judge(c.name, r.key, c.judged ? searchClass : c.top, true) }),
        iconButton('close', `no: ${c.name} was wrong`, { small: true, cls: 'no', onclick: () => judge(c.name, r.key, c.judged ? searchClass : c.top, false) }),
      ),
    ),
    el(
      'div.row.small',
      el('span.dim', 'end to end, through the couplings:'),
      iconButton('check', 'yes, end to end', { small: true, cls: 'yes', onclick: () => judge(d.name, r.key, searchClass, true, true) }),
      iconButton('close', 'no, end to end', { small: true, cls: 'no', onclick: () => judge(d.name, r.key, searchClass, false, true) }),
    ),
  );
}

// ------------------------------------------------------------------ outputs and neighbours

function outputCard() {
  const d = detail;
  const cls = el('select', { 'aria-label': 'class to write' }, d.classes.map((c, i) => el('option', { value: i }, c)));
  const thr = el('input', { type: 'number', min: 0, max: 1, step: 0.05, value: d.steer.heads[0].threshold, 'aria-label': 'threshold' });
  const write = async () => {
    try {
      enlisted = await app.call('classify.enlist', { name: d.name, class: +cls.value, threshold: +thr.value });
      app.status(`${enlisted.members} texts in the list “${enlisted.name}”`);
      detail = await app.call('classify.get', { name: d.name });
      lists = (await app.call('lists.stats')).lists;
      draw();
    } catch {
      /* reported */
    }
  };
  return el(
    'div.card.clf-out',
    el('div.row', el('h3', 'outputs as a list'), cls, el('label.small.dim', 'at least ', thr), iconButton('list', 'write the outputs as a list', { text: 'write the list', disabled: !d.trained, onclick: write })),
    el('p.dim.small', enlisted ? `“${enlisted.name}” (${enlisted.list}) holds ${enlisted.members} texts now: train another classifier on it` : 'the texts it counts in a class, as a list another classifier can learn from'),
  );
}

function similarCard() {
  const d = detail;
  const t = el('input', { type: 'text', placeholder: 'a text, or a key like 0,12', 'aria-label': 'text to find neighbours of' });
  const go = async () => {
    const v = t.value.trim();
    if (!v) return;
    try {
      similar = await app.call('classify.similar', /^(note:\d+|\d{1,3}(,\d{1,3}){0,3})$/.test(v) ? { name: d.name, key: v, limit: 12 } : { name: d.name, text: v, limit: 12 });
      draw();
    } catch {
      /* reported */
    }
  };
  t.addEventListener('keydown', (e) => e.key === 'Enter' && go());
  return el(
    'div.card.clf-similar',
    el('div.row', el('h3', 'near texts'), t, iconButton('search', 'find the nearest texts', { disabled: !d.trained, onclick: go })),
    similar
      ? el(
          'ol.clf-results',
          similar.results.map((r) => el('li', el('div.res', el('span.prob', { style: { '--p': Math.max(0, r.cosine) } }, num(r.cosine, 2)), el('span.text', { title: r.key }, r.note ? el('span.dim', 'note · ') : null, r.text)))),
        )
      : el('p.dim.small', 'what it represents a text as (a deterministic embedding), and the memories and notes nearest it'),
  );
}

// ------------------------------------------------------------------ the job

function watch() {
  clearTimeout(poll);
  if (!visible || !detail) return;
  const running = detail.job && detail.job.state === 'running';
  if (!running) return;
  poll = setTimeout(async () => {
    if (!visible || !detail) return;
    const name = detail.name;
    let job;
    try {
      job = await app.api.call('classify.job', { name });
    } catch {
      return;
    }
    if (!detail || detail.name !== name) return;
    detail.job = job;
    if (job.state !== 'running') {
      app.status(job.state === 'done' ? `“${name}” trained: ${pct(job.summary?.metrics?.accuracy)} on ${job.summary?.measured_on} texts` : `“${name}”: ${job.state}${job.error ? ` (${job.error})` : ''}`, job.state === 'failed');
      biasReport = null;
      load();
      return;
    }
    const card = body.querySelector('.clf-train .clf-progress');
    if (card) card.replaceWith(progress(job));
    watch();
  }, 400);
}

export default {
  barName: 'classifiers',
  barPlaceholder: 'search the open classifier’s class (Enter)',
  init(section, a) {
    app = a;
    more.init({
      app,
      detail: () => detail,
      all: () => all,
      lists: () => lists,
      draw: () => draw(),
      open: (n) => (remember(n), load()),
      visible: () => visible,
      // the open classifier read again (its version after feedback)
      reload: async () => {
        if (!detail) return;
        try {
          detail = await app.api.call('classify.get', { name: detail.name });
        } catch {
          /* gone */
        }
      },
      reloadAll: () => load(),
    });
    count = el('span.count');
    this.barRight = count;
    body = el('div.clf-view');
    section.append(body);
    app.on('save', () => {
      selected = null;
      detail = null;
    });
  },
  show() {
    visible = true;
    load();
  },
  hide() {
    visible = false;
    clearTimeout(poll);
    more.stop();
  },
  barKey(e) {
    if (e.key === 'Enter') {
      query = app.bar.get().trim();
      search();
      return true;
    }
  },
};
