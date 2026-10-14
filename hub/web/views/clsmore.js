// the classifiers view's third part: its buckets (every text filed by class and how sure,
// memoized, with a live indexing job), near texts on that memo, validators (classifiers
// that check another) with the texts they disagree on, and classifiers as .anendcls
// modules to download and bring in

import { el, clear, confirmClick, saveFile, fromBase64, fileBase64, openHit, KEY } from '../util.js';
import { icon, iconButton, tip } from '../icons.js';

let ctx; // {app, detail(), all(), lists(), draw(), open(name), visible()}
const pct = (x) => (x == null ? '–' : `${(x * 100).toFixed(1)}%`);
const num = (x, d = 3) => (x == null || !isFinite(x) ? '–' : (+x).toFixed(d));
const BANDS = 10;

// what the cards hold for the open classifier
let st = fresh();
function fresh(name = null) {
  return {
    name,
    buckets: null, // classify.buckets
    job: null, // classify.index_job
    pick: null, // {class, band}
    bucket: null, // classify.bucket
    near: null, // classify.near
    nearBy: 'embedding',
    nearText: '',
    validators: null, // [{by, class, min, mode}] being edited
    savedValidators: '',
    disagreements: null,
  };
}

let poll = null;
let module_ = null; // {file, data, summary, as, lists: {class: key}, result}

export function init(c) {
  ctx = c;
}

// a classifier was opened: its cards start over
export function reset(name) {
  if (st.name === name) return;
  clearTimeout(poll);
  st = fresh(name);
}

export function stop() {
  clearTimeout(poll);
}

const draw = () => ctx.draw();

// ------------------------------------------------------------------ buckets

async function loadBuckets() {
  const d = ctx.detail();
  if (!d || !d.trained) return;
  try {
    const [b, job] = await Promise.all([ctx.app.call('classify.buckets', { name: d.name }), ctx.app.api.call('classify.index_job', { name: d.name })]);
    if (st.name !== d.name) return;
    st.buckets = b;
    if (job.state === 'running') st.job = job;
    draw();
    if (st.job?.state === 'running') watch();
  } catch {
    /* reported */
  }
}

// the indexing runs as a call that answers when it's done (`wait`), while its job is
// polled for the progress bar; the answer, not a poll, says it has finished (a poll can
// still see the last run's job before this one starts)
let indexing = null; // the classifier being indexed by this page
async function index() {
  const d = ctx.detail();
  const name = d.name;
  indexing = name;
  st.job = { name, state: 'running', done: 0, total: 0 };
  draw();
  watch();
  let job;
  try {
    job = await ctx.app.call('classify.index', { name, wait: true });
  } catch {
    job = null;
  }
  indexing = null;
  clearTimeout(poll);
  if (st.name !== name) return;
  st.job = job;
  if (job) {
    const r = job.report;
    if (job.state === 'done') ctx.app.status(`“${name}” indexed: ${r.computed} read, ${r.cached} already known, ${num(r.seconds, 1)} s`);
    else ctx.app.status(`indexing “${name}”: ${job.state}${job.error ? ` (${job.error})` : ''}`, job.state === 'failed');
  }
  st.buckets = null;
  loadBuckets();
}

async function cancelIndex() {
  try {
    await ctx.app.call('classify.index_cancel', { name: st.name });
    ctx.app.status('stopping…');
  } catch {
    /* reported */
  }
}

function watch() {
  clearTimeout(poll);
  poll = setTimeout(async () => {
    if (!ctx.visible() || !st.name) return;
    const name = st.name;
    let job;
    try {
      job = await ctx.app.api.call('classify.index_job', { name });
    } catch {
      return;
    }
    if (st.name !== name) return;
    const mine = indexing === name;
    if (job.state === 'running') st.job = job;
    else if (!mine) {
      // indexed from elsewhere (another tab): it has finished
      st.job = job;
      st.buckets = null;
      loadBuckets();
      return;
    }
    const bar = document.querySelector('.clf-index .clf-progress');
    if (bar) bar.replaceWith(indexProgress(st.job));
    watch();
  }, 300);
}

function indexProgress(job) {
  const frac = job.total ? job.done / job.total : 0;
  const running = job.state === 'running';
  return el(
    'div.clf-progress',
    el('div.track', { role: 'progressbar', 'aria-valuemin': 0, 'aria-valuemax': job.total || 0, 'aria-valuenow': job.done || 0, 'aria-label': 'indexing' }, el('div', { className: `fill ${job.state}`, style: { width: `${(running ? frac : 1) * 100}%` } })),
    el(
      'div.row.small',
      el('span', running ? `reading ${job.done} of ${job.total || '…'} texts` : job.state === 'failed' ? `failed: ${job.error}` : `${job.state}${job.report ? `: ${job.report.computed} read, ${job.report.cached} known already, ${num(job.report.seconds, 1)} s` : ''}`),
    ),
  );
}

async function openBucket(cls, band) {
  const d = ctx.detail();
  st.pick = { class: cls, band };
  try {
    st.bucket = await ctx.app.call('classify.bucket', { name: d.name, class: cls, ...(band == null ? {} : { band }), limit: 300 });
  } catch {
    st.bucket = null;
  }
  draw();
  document.querySelector('.clf-bucket-hits')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

async function judge(name, key, cls, yes) {
  try {
    const r = await ctx.app.call('classify.feedback', { name, key, class: cls, yes });
    const e = r.entry;
    ctx.app.status(`${name}: ${yes ? 'yes' : 'no'}, “${e.class}” ${pct(e.before)} → ${pct(e.after)}`);
    return true;
  } catch {
    return false;
  }
}

export function bucketsCards() {
  const d = ctx.detail();
  if (!d.trained) return el('div.card.dim', 'train it first: then every memory and note can be filed in its buckets');
  if (!st.buckets) {
    loadBuckets();
    return el('div.card.clf-index', el('h3', 'buckets'), el('p.dim.small', 'counting…'));
  }
  return [indexCard(), bucketHitsCard(), nearCard()];
}

function indexCard() {
  const b = st.buckets;
  const total = (ctx.app.counts.memories || 0) + (ctx.app.counts.notes || 0);
  const running = st.job?.state === 'running';
  const max = Math.max(1, ...b.buckets.flatMap((x) => x.bands));
  const colourOf = (i) => (i == null ? 'var(--unsure)' : b.palette[i]);
  const rows = b.buckets.map((x) => {
    const on = st.pick && st.pick.class === (x.index ?? 'uncertain');
    return el(
      'div.bk-row',
      { className: `bk-row${on ? ' on' : ''}`, style: { '--c': colourOf(x.index) } },
      el(
        'button.bk-name',
        { type: 'button', onclick: () => openBucket(x.index ?? 'uncertain', null), 'aria-label': `the texts in ${x.class}` },
        el('span.sw'),
        el('span.nm', x.class),
        el('span.n', x.count),
      ),
      el(
        'div.bk-bands',
        { role: 'group', 'aria-label': `${x.class} by sureness` },
        x.bands.map((n, band) =>
          tip(
            el(
              'button.bk-band',
              {
                type: 'button',
                className: `bk-band${n ? '' : ' empty'}${on && st.pick.band === band ? ' on' : ''}`,
                disabled: !n,
                style: { '--h': n / max, '--a': 0.25 + band * 0.075 },
                onclick: () => openBucket(x.index ?? 'uncertain', band),
              },
              el('span.bar'),
            ),
            `${x.class}, ${band * 10}–${band * 10 + 10}% sure: ${n} text${n === 1 ? '' : 's'}`,
          ),
        ),
      ),
    );
  });
  return el(
    'div.card.clf-index',
    el(
      'div.row',
      el('h3', 'buckets'),
      el('span.dim.small', `${b.indexed} of ${total} texts filed`),
      el('span.grow'),
      running
        ? iconButton('close', 'stop indexing', { text: 'stop', onclick: cancelIndex })
        : iconButton('bolt', 'index the save: read every memory and note once, so buckets, near texts and filters answer at once', { text: b.indexed ? 'index again' : 'index the save', cls: b.indexed >= total ? '' : 'primary', onclick: index }),
    ),
    st.job ? indexProgress(st.job) : null,
    el('div.bk-grid', el('div.bk-axis.small.dim', el('span', 'class'), el('span', '0%'), el('span', 'sureness →'), el('span', '100%')), rows),
    el('p.dim.small', 'each class’s texts by how sure it is, in tenths; “uncertain” holds the texts it puts in no class. Click a class or a column for its texts. What’s filed is kept until training, feedback or steering changes it.'),
  );
}

function bucketHitsCard() {
  const d = ctx.detail();
  const bk = st.bucket;
  if (!bk) return null;
  const cls = st.pick.class;
  const isClass = cls !== 'uncertain';
  const band = st.pick.band;
  const ci = typeof cls === 'number' ? cls : null;
  return el(
    'div.card.clf-bucket-hits',
    { style: { '--c': ci == null ? 'var(--unsure)' : st.buckets?.palette?.[ci] || 'var(--accent)' } },
    el(
      'div.row',
      el('h3', `${bk.class}${band == null ? '' : ` · ${band * 10}–${band * 10 + 10}%`}`),
      el('span.dim.small', `${bk.count} text${bk.count === 1 ? '' : 's'}${bk.stale ? ` · ${bk.stale} changed since (index again)` : ''}`),
      el('span.grow'),
      iconButton('close', 'close the bucket', { small: true, cls: 'plain', onclick: () => ((st.bucket = null), (st.pick = null), draw()) }),
    ),
    bk.hits.length
      ? el(
          'ol.clf-results',
          bk.hits.map((h) =>
            el(
              'li',
              el(
                'div.res',
                el('span.prob', { style: { '--p': h.prob } }, pct(h.prob)),
                el('button.text.linkish', { type: 'button', onclick: () => openHit(ctx.app, h) }, h.note ? el('span.dim', 'note · ') : null, h.text),
                el('span.grow'),
                iconButton('near', 'texts near this one', { small: true, cls: 'plain', onclick: () => near(h.key, h.text) }),
                isClass ? iconButton('check', `yes, it is ${bk.class}`, { small: true, cls: 'yes', onclick: async () => (await judge(d.name, h.key, cls, true)) && refresh() }) : null,
                isClass ? iconButton('close', `no, it isn't ${bk.class}`, { small: true, cls: 'no', onclick: async () => (await judge(d.name, h.key, cls, false)) && refresh() }) : null,
              ),
            ),
          ),
        )
      : el('p.dim.small', 'empty'),
    bk.hits.length < bk.count ? el('p.dim.small', `the surest ${bk.hits.length} shown`) : null,
  );
}

// after feedback the classifier's version moves, so its memo is read again
async function refresh() {
  ctx.reload();
  st.buckets = null;
  if (st.pick) {
    const { class: c, band } = st.pick;
    await loadBuckets();
    openBucket(c, band);
  }
}

async function near(key, text) {
  const d = ctx.detail();
  st.nearText = key || text;
  try {
    st.near = await ctx.app.call('classify.near', { name: d.name, ...(key ? { key } : { text }), by: st.nearBy, limit: 24 });
  } catch {
    st.near = null;
  }
  draw();
  document.querySelector('.clf-near')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

function verdict(row, classes, palette) {
  if (!row) return null;
  return el(
    'div.verdict',
    classes.map((c, i) =>
      el('span.vchip', { className: `vchip${row.members?.includes(c) ? ' in' : ''}${i === row.top ? ' top' : ''}`, style: { '--c': palette?.[i] || 'var(--accent)', '--p': row.probs[i] } }, el('span.lab', c), el('span.p', pct(row.probs[i]))),
    ),
  );
}

function nearCard() {
  const d = ctx.detail();
  const t = el('input', { type: 'search', placeholder: 'a text, or a key like 0,12 or note:1000000', value: st.nearText, 'aria-label': 'text to find near texts of' });
  const go = () => {
    const v = t.value.trim();
    st.nearText = v;
    if (!v) return;
    if (KEY.test(v)) near(v);
    else near(null, v);
  };
  t.addEventListener('keydown', (e) => e.key === 'Enter' && go());
  const by = el(
    'div.seg',
    { role: 'group', 'aria-label': 'near by' },
    ['embedding', 'outputs'].map((b) =>
      tip(
        el('button', {
          type: 'button',
          className: st.nearBy === b ? 'on' : '',
          'aria-pressed': String(st.nearBy === b),
          onclick: () => {
            st.nearBy = b;
            if (st.nearText) go();
            else draw();
          },
        }, b === 'embedding' ? 'meaning' : 'verdict'),
        b === 'embedding' ? 'near by what it represents the text as (its embedding)' : 'near by its verdict: texts it judges alike',
      ),
    ),
  );
  const n = st.near;
  return el(
    'div.card.clf-near',
    el('div.row', el('h3', 'near texts'), by, t, iconButton('near', 'find the near texts', { key: 'Enter', cls: 'primary', onclick: go })),
    n
      ? [
          el('div.row.small', el('span.dim', `“${n.text.length > 60 ? n.text.slice(0, 60) + '…' : n.text}” reads as`), verdict(n.row, st.buckets.classes, st.buckets.palette)),
          n.results.length
            ? el(
                'ol.clf-results',
                n.results.map((r) =>
                  el(
                    'li',
                    el(
                      'div.res',
                      el('span.prob', { style: { '--p': Math.max(0, r.cosine) } }, num(r.cosine, 2)),
                      el('button.text.linkish', { type: 'button', onclick: () => openHit(ctx.app, r) }, r.note ? el('span.dim', 'note · ') : null, r.text),
                      el('span.grow'),
                      iconButton('near', 'near this one instead', { small: true, cls: 'plain', onclick: () => near(r.key, r.text) }),
                    ),
                  ),
                ),
              )
            : el('p.dim.small', n.indexed ? 'nothing near among the filed texts' : 'nothing filed yet: index the save first'),
          el('p.dim.small', `among ${n.indexed} filed texts`),
        ]
      : el('p.dim.small', 'the filed texts closest to a text: by meaning (what it represents it as) or by verdict (how it judges it). Fast, from the memo.'),
  );
}

// ------------------------------------------------------------------ validators

async function loadValidators() {
  const d = ctx.detail();
  try {
    const r = await ctx.app.call('classify.validators', { name: d.name });
    if (st.name !== d.name) return;
    st.validators = r.validators.map((v) => ({ ...v }));
    st.savedValidators = JSON.stringify(r.validators);
    draw();
  } catch {
    /* reported */
  }
}

async function saveValidators() {
  const d = ctx.detail();
  const set = st.validators.filter((v) => v.by && v.class);
  try {
    const r = await ctx.app.call('classify.validators', { name: d.name, set });
    st.validators = r.validators.map((v) => ({ ...v }));
    st.savedValidators = JSON.stringify(r.validators);
    st.disagreements = null;
    ctx.app.status(set.length ? `“${d.name}” is checked by ${set.map((v) => v.by).join(', ')}` : `“${d.name}” has no validators now`);
    draw();
  } catch {
    /* reported */
  }
}

async function findDisagreements() {
  const d = ctx.detail();
  try {
    st.disagreements = await ctx.app.call('classify.disagreements', { name: d.name, limit: 40 });
  } catch {
    st.disagreements = null;
  }
  draw();
}

export function checksCards() {
  const d = ctx.detail();
  if (!st.validators) {
    loadValidators();
    return el('div.card.dim', 'reading its validators…');
  }
  return [validatorsCard(d), disagreementsCard(d)];
}

function validatorsCard(d) {
  const others = ctx.all().filter((c) => c.name !== d.name);
  const changed = JSON.stringify(st.validators) !== st.savedValidators;
  const rows = st.validators.map((v, i) => {
    const by = others.find((c) => c.name === v.by);
    const byEl = el(
      'select',
      {
        'aria-label': 'checked by',
        onchange: () => {
          v.by = byEl.value;
          const c = others.find((x) => x.name === v.by);
          v.class = c?.classes?.[0] || '';
          draw();
        },
      },
      el('option', { value: '' }, 'pick a classifier'),
      others.map((c) => el('option', { value: c.name, selected: c.name === v.by }, `${c.name}${c.trained ? '' : ' (untrained)'}`)),
    );
    const cls = el(
      'select',
      { 'aria-label': 'its class', onchange: () => ((v.class = cls.value), draw()) },
      (by?.classes || (v.class ? [v.class] : [])).map((c) => el('option', { value: c, selected: c === v.class }, c)),
    );
    const out = el('span.val', pct(v.min));
    const min = el('input', {
      type: 'range', min: 0, max: 1, step: 0.01, value: v.min, 'aria-label': 'at least',
      oninput: () => ((v.min = +min.value), (out.textContent = pct(v.min))),
      onchange: () => draw(),
    });
    const mode = el(
      'div.seg',
      { role: 'group', 'aria-label': 'how it checks' },
      tip(el('button', { type: 'button', className: v.mode !== 'weigh' ? 'on' : '', onclick: () => ((v.mode = 'gate'), draw()) }, 'gate'), 'gate: a text it doesn’t pass is in none of the classes'),
      tip(el('button', { type: 'button', className: v.mode === 'weigh' ? 'on' : '', onclick: () => ((v.mode = 'weigh'), draw()) }, 'scale'), 'scale: the probabilities are multiplied by the validator’s'),
    );
    return el(
      'div.vrow',
      el('label.vfield', el('span.dim.small', 'checked by'), byEl),
      el('label.vfield', el('span.dim.small', 'saying'), cls),
      el('label.vfield.vmin', el('span.dim.small', 'at least'), el('span.row', min, out)),
      mode,
      iconButton('delete', 'remove this validator', { small: true, cls: 'plain', onclick: () => (st.validators.splice(i, 1), draw()) }),
    );
  });
  return el(
    'div.card.clf-validators',
    el('div.row', el('h3', 'validators'), el('span.dim.small', 'classifiers that check this one'), el('span.grow'), changed ? el('span.hint', 'unsaved') : null),
    rows.length ? el('div.vrows', rows) : el('p.dim.small', others.length ? `none: “${d.name}” answers on its own` : 'make another classifier first: it can check this one'),
    el(
      'div.row',
      iconButton('new', 'add a validator', {
        text: 'add a validator',
        disabled: !others.length,
        onclick: () => {
          const c = others.find((x) => x.trained) || others[0];
          st.validators.push({ by: c.name, class: c.classes?.[0] || '', min: 0.5, mode: 'gate' });
          draw();
        },
      }),
      el('span.grow'),
      iconButton('save', 'keep these validators', { text: 'keep', cls: changed ? 'primary' : '', disabled: !changed, onclick: saveValidators }),
    ),
    el('p.dim.small', `A text counts in “${d.name}”’s classes only when each validator says its class at least that sure (gate), or its probabilities are scaled by the validator’s (scale). Filters and buckets use the checked verdicts.`),
  );
}

function disagreementsCard(d) {
  const r = st.disagreements;
  const has = st.savedValidators !== '[]';
  return el(
    'div.card.clf-disagree',
    el('div.row', el('h3', 'disagreements'), el('span.grow'), iconButton('alert', 'find the texts it counts in a class that a validator doesn’t pass', { text: 'find them', disabled: !has || !d.trained, onclick: findDisagreements })),
    !has
      ? el('p.dim.small', 'add a validator first: then this lists the texts the two don’t agree on')
      : !r
        ? el('p.dim.small', 'the texts it puts in a class while a validator says no; tell either one it was right')
        : r.results.length
          ? el(
              'ol.clf-results.dis',
              r.results.map((x) =>
                el(
                  'li',
                  el(
                    'div.res',
                    el('span.prob', { style: { '--p': x.prob } }, pct(x.prob)),
                    el('button.text.linkish', { type: 'button', onclick: () => openHit(ctx.app, x) }, x.text),
                    el('span.dim.small', x.class),
                    el('span.grow'),
                    el('span.dim.small', `${d.name}:`),
                    iconButton('check', `yes, it is ${x.class}: ${d.name} is right`, { small: true, cls: 'yes', onclick: async () => (await judge(d.name, x.key, x.class, true)) && (ctx.reload(), findDisagreements()) }),
                    iconButton('close', `no, it isn't ${x.class}: ${d.name} is wrong`, { small: true, cls: 'no', onclick: async () => (await judge(d.name, x.key, x.class, false)) && (ctx.reload(), findDisagreements()) }),
                  ),
                  el(
                    'div.checks.small',
                    x.checks.map(([by, p]) => {
                      const v = st.validators.find((v) => v.by === by);
                      return el(
                        'span.check-chip',
                        el('span.dim', `${by}: ${v?.class || ''} `),
                        el('b', pct(p)),
                        v ? iconButton('check', `yes: ${by} is right, it is ${v.class}`, { small: true, cls: 'yes', onclick: async () => (await judge(by, x.key, v.class, true)) && findDisagreements() }) : null,
                        v ? iconButton('close', `no: ${by} is wrong, it isn't ${v.class}`, { small: true, cls: 'no', onclick: async () => (await judge(by, x.key, v.class, false)) && findDisagreements() }) : null,
                      );
                    }),
                  ),
                ),
              ),
            )
          : el('p.dim.small', 'they agree on every text'),
  );
}

// ------------------------------------------------------------------ modules

export function shareCards() {
  return [exportCard()];
}

function exportCard() {
  const d = ctx.detail();
  const desc = el('input', { type: 'text', placeholder: 'what it tells apart (optional)', 'aria-label': 'description' });
  const author = el('input', { type: 'text', placeholder: 'by (optional)', 'aria-label': 'author' });
  const up = el('input', { type: 'checkbox', checked: true });
  const go = async () => {
    try {
      const r = await ctx.app.call('classify.export', { name: d.name, upstream: up.checked, description: desc.value, author: author.value });
      saveFile(r.file, fromBase64(r.data));
      ctx.app.status(`${r.file}: ${(r.size / 1024).toFixed(0)} KB`);
    } catch {
      /* reported */
    }
  };
  return el(
    'div.card.clf-export',
    el('h3', 'as a file'),
    el('p.dim.small', `“${d.name}” as a .anendcls module: its weights, steering and classes (not its lists), to keep or to bring into another save.`),
    el('div.row', desc, author),
    el(
      'div.row',
      d.inputs.length ? el('label.check.small', up, ` with what it builds on (${d.inputs.map((i) => i.from).join(', ')})`) : null,
      el('span.grow'),
      iconButton('download', 'download it as a .anendcls file', { text: 'download', cls: 'primary', disabled: !d.trained, onclick: go }),
    ),
    d.trained ? null : el('p.dim.small', 'train it first'),
  );
}

// bringing a module in: the file, what's in it, where its classes go
export function importCard() {
  const file = el('input', {
    type: 'file',
    accept: '.anendcls',
    hidden: true,
    onchange: async () => {
      const f = file.files[0];
      if (!f) return;
      try {
        const data = await fileBase64(f);
        const summary = await ctx.app.call('classify.inspect', { data });
        module_ = { file: f.name, data, summary, as: summary.name, lists: {}, result: null };
      } catch {
        module_ = null;
      }
      draw();
    },
  });
  const pickBtn = iconButton('upload', 'bring in a classifier from a .anendcls file', { text: 'from a file', onclick: () => file.click() });
  if (!module_) return el('div.card.clf-import', el('h3', 'bring one in'), el('p.dim.small', 'a classifier someone shared as a .anendcls file'), el('div.row', pickBtn, file));
  const m = module_.summary;
  if (module_.result) {
    const r = module_.result;
    return el(
      'div.card.clf-import',
      el('h3', `brought in “${r.name}”`),
      el('p.small', r.created.length ? `made ${r.created.join(', ')}` : '', r.reused.length ? ` · already here: ${r.reused.join(', ')}` : ''),
      el('ul.small.imp-lists', r.lists.map(([c, key, isNew]) => el('li', `${c} → list ${key}${isNew ? ' (new)' : ''}`))),
      el('div.row', iconButton('check', 'done', { text: 'open it', cls: 'primary', onclick: () => ((module_ = null), ctx.open(r.name)) }), el('span.grow'), pickBtn, file),
    );
  }
  const lists = ctx.lists();
  const as = el('input', { type: 'text', value: module_.as, 'aria-label': 'its name here', oninput: () => (module_.as = as.value) });
  const take = async () => {
    try {
      const lists_ = Object.fromEntries(Object.entries(module_.lists).filter(([, v]) => v));
      module_.result = await ctx.app.call('classify.import', { data: module_.data, as: module_.as, lists: lists_ });
      ctx.app.status(`brought in “${module_.result.name}”`);
      ctx.reloadAll();
    } catch {
      /* reported */
    }
  };
  return el(
    'div.card.clf-import',
    el('div.row', el('h3', m.name), el('span.grow'), iconButton('close', 'never mind', { small: true, cls: 'plain', onclick: () => ((module_ = null), draw()) })),
    el('p.dim.small', module_.file, m.author ? ` · by ${m.author}` : ''),
    m.description ? el('p.small', m.description) : null,
    el(
      'dl.imp-facts.small',
      el('dt', 'classes'), el('dd', m.classes.join(', ')),
      el('dt', 'kind'), el('dd', `${m.head === 'multi' ? 'labels' : m.head === 'regress' ? 'a number' : 'one class'} · ${m.preset} · ${m.weights.toLocaleString()} weights`),
      el('dt', 'measured'), el('dd', m.accuracy == null ? 'not measured' : `${pct(m.accuracy)} accuracy, F1 ${num(m.macro_f1)} on ${m.examples} texts${m.measured_on ? ` (${m.measured_on})` : ''}`),
      m.upstream.length ? [el('dt', 'builds on'), el('dd', m.upstream.map(([n, b]) => `${n}${b ? '' : ' (not in the file: needs one by that name here)'}`).join(', '))] : null,
    ),
    el('label.vfield', el('span.dim.small', 'its name here'), as),
    el('p.dim.small', 'each class keeps its texts in a list: a new one named after the class (or the one by that name), or one of yours'),
    el(
      'div.imp-map',
      m.classes.map((c) => {
        const s = el(
          'select',
          { 'aria-label': `the list for ${c}`, onchange: () => (module_.lists[c] = s.value) },
          el('option', { value: '' }, `“${c}” (by name, or new)`),
          lists.map((l) => el('option', { value: l.list, selected: module_.lists[c] === l.list }, `${l.name} (${l.count})`)),
        );
        return el('label.imp-row', el('span.cls', c), el('span.dim', '→'), s);
      }),
    ),
    el('div.row', el('span.grow'), iconButton('download', 'bring it into this save', { text: 'bring it in', cls: 'primary', onclick: take })),
  );
}
