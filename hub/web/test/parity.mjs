// the browser's engine (hub/web/local) against the hub's app: the reference fixtures
// core/src tests spread.rs and weum.rs with, then one sequence of calls through both,
// answers compared. Times differ and are set aside; everything else must match.
//
//   cargo build -p anend-hub && node hub/web/test/parity.mjs
//
// It starts `target/debug/anend-hub --local` on 127.0.0.1:18121 over a scratch data dir
// (HUB=path to use another binary, PORT=another port, --fixtures for those alone).

import { spawn } from 'node:child_process';
import { readFileSync, mkdtempSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { Engine } from '../local/engine.js';
import { Spread, hash01, seedHue, home } from '../local/spread.js';
import { spreadSums, splitWhitespace } from '../local/text.js';
import { searchVariations, pfReduction, gsumDetail } from '../local/weum.js';
import { plain } from '../local/plain.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '../../..');
let failures = 0;
let quiet = false;
const fail = (...m) => {
  failures++;
  if (failures <= 60 && !quiet) console.log('  ✗', ...m);
};

// ---------------------------------------------------------------- the fixtures

function fixtures() {
  const sr = JSON.parse(readFileSync(join(root, 'core/src/spread_reference.json'), 'utf8'));
  let exact = 0;
  let total = 0;
  for (const c of sr.cases) {
    const sp = new Spread({ reversed: !!c.reversed });
    for (const n of c.nodes) {
      const node = Spread.node(n[0], n[1], n[2], n[3]);
      if (n[4]) Object.assign(node, { pinned: true, placed: true, x: n[5], y: n[6] });
      sp.add(node);
    }
    for (let i = 0; i < c.steps; i++) sp.step(1 / 60);
    sp.nodes.forEach((n, i) => {
      for (const k of ['x', 'y', 'z']) {
        const w = c.out[i][k];
        total++;
        if (n[k] === w) exact++;
        else if (Math.abs(n[k] - w) > 1e-6 * (1 + Math.abs(w))) fail(`spread ${c.name}: ${n.id}.${k} ${n[k]} vs ${w}`);
      }
    });
    if (Math.abs(sp.alpha - c.alpha) > 1e-9) fail(`spread ${c.name}: alpha`);
    if (Math.abs(sp.energy - c.energy) > 1e-6 * (1 + sp.energy)) fail(`spread ${c.name}: energy`);
    if (sp.order.length !== c.buckets) fail(`spread ${c.name}: buckets`);
  }
  for (const h of sr.helpers) {
    const [s, r] = spreadSums(h.t);
    if (s !== h.s || r !== h.r) fail(`sums ${h.t}`);
    if (Math.max(1, splitWhitespace(h.t).length) !== h.w) fail(`words ${h.t}`);
    if (hash01(h.t, 7) !== h.hash) fail(`hash01 ${h.t}`);
    if (seedHue(s) !== h.hue) fail(`hue ${h.t}`);
  }
  const [hx, hy] = home(5, 10);
  if (hx !== sr.home5[0] || hy !== sr.home5[1]) fail('home(5)');
  console.log(`spread_reference: ${sr.cases.length} cases, ${exact}/${total} coordinates bit for bit, ${sr.helpers.length} helpers`);

  const wr = JSON.parse(readFileSync(join(root, 'core/src/weum_reference.json'), 'utf8'));
  let n = 0;
  for (const [q, want] of Object.entries(wr.variations)) {
    n++;
    if (JSON.stringify(searchVariations(q)) !== JSON.stringify(want)) fail(`variations of ${q}`);
  }
  for (const [x, want] of Object.entries(wr.pfR)) {
    n++;
    if (pfReduction(+x) !== want) fail(`pfR(${x}): ${pfReduction(+x)} vs ${want}`);
  }
  for (const [t, want] of Object.entries(wr.gsum)) {
    n++;
    if (!gsumDetail(t).startsWith(want[2])) fail(`gsum(${t}): ${gsumDetail(t)} vs ${want[2]}`);
  }
  console.log(`weum_reference: ${n} checks`);
}

// ---------------------------------------------------------------- the sequence

// app.rs `saves.import` opens (so makes) the new save before bundle::import checks the
// bundle's version, so a refused bundle leaves an empty save behind on the hub
const HUB_BUG = 'hub bug: a refused saves.import leaves an empty save behind';

const WORDS = `listen silent enlist tinsel inlets dog god light wisdom truth love seven eleven
stone notes onset tones heart earth hater rate tear dusty study night thing below elbow
Rusty gears turn monkeys things Society lantern ocean canoe north thorn worth throw`.split(/\s+/);

const MARKDOWN = [
  '# A note\n\nsome **bold** and *it* `code` [link](http://example.com "t")\nnext line\n\n---\n',
  '- a\n- [x] done task\n  1. one\n  2. two\n\n> quoted text\n> more\nlazy line\n\n```rust\nfn x() { a*b }\n```\n',
  '| h1 | h2 |\n|---|---|\n| c1 | c2 |\n| *e*1 | `x|y` |\n\nafter table',
  'snake_case_word a*b*c __strong__ ***both*** ~~gone~~ ~one~ &amp; &copy; &#35; &#x41; \\*escaped\\* ![alt *x*](i.png)',
  'Setext heading\n===\n\nparagraph\nunder\n---\n\n    indented code\n    more\n\nend <span class="x">inline</span> <http://auto.link/x> <me@example.com>',
  '<div>\nhtml block\n</div>\n\n[ref link][r] and [r] and [missing]\n\n[r]: http://r.example "title"\n\ntrailing  \nhard break\\\nagain',
  'Λόγος καὶ שלום — émigré naïve 日本語 tokens\ttabbed\r\nwindows line',
];

function corpus() {
  const out = [...WORDS];
  for (let i = 0; i < 280; i++) out.push(`${WORDS[i % WORDS.length]} ${i}`);
  out.push('Λόγος', 'שלום light', 'αω', 'The quick brown fox', 'abc xyz!', '@`[{', 'Hello World', 'émigré', 'quick fox jumps');
  return out;
}

// one run of the sequence over `call`, which answers {ok, data} or {ok: false, error}
async function sequence(call) {
  const log = [];
  const c = async (op, args = {}, known) => {
    const a = await call(op, args);
    log.push({ op, args, known, ...a });
    return a.ok ? a.data : undefined;
  };
  await c('hello');
  const texts = corpus();
  for (const t of texts) await c('mem.saturate', { text: t });
  // forgotten keys come back before new ones are minted, smallest first
  for (const k of ['5', '3', '0,7', '200', '0,20', '9']) await c('mem.forget', { key: k });
  await c('mem.forget', { key: '5' });
  for (const t of ['reborn one', 'reborn two', 'listen', 'reborn three', 'reborn four', 'reborn five', 'reborn six', 'reborn seven']) await c('mem.saturate', { text: t });
  await c('mem.saturate', { text: '   ' });
  await c('mem.forget', { key: 'note:1000000' });
  await c('mem.forget', { key: 'nonsense' });

  const notes = [];
  for (const [i, md] of MARKDOWN.entries()) {
    const n = await c('notes.save', { note: { tags: i % 2 ? ['odd', 'md'] : ['even'] }, text: `# Note ${i} ${WORDS[i]}\n\n${md}` });
    notes.push(n);
  }
  await c('notes.save', { note: {}, text: 'no title here' });
  await c('notes.save', { note: {}, text: '#\n\nempty title' });
  const edited = await c('notes.save', { note: { ...notes[1], tags: ['edited'] }, text: '# Note 1 edited\n\nnew body with listen' });
  if (edited) await c('notes.save', { note: edited, text: '# Note 1 edited again\n\nbody' });
  await c('notes.delete', { key: notes[3]?._key });
  await c('notes.delete', { key: 'nope' });
  await c('notes.get', { key: notes[0]?._key });
  await c('notes.get', { key: 'nope' });
  for (const q of ['', 'note', 'NOTE wor', 'list', 'bol', 'h1', 'tok', 'zzz', 'émi', 'Λόγ']) await c('notes.list', { q });
  // the tags view: counts, and notes by all or any of the tags
  await c('notes.save', { note: { tags: ['odd', 'even', 'Ünï'] }, text: '# both\n\nbody' });
  await c('notes.tags', {});
  for (const [tags, mode] of [[['odd'], undefined], [['odd', 'md'], 'and'], [['odd', 'even'], undefined], [['odd', 'even'], 'or'], [['edited', 'Ünï', 'nope'], 'or'], [[], 'or'], [['nope'], undefined], ['odd', undefined], [[1, 'even'], 'and']]) {
    await c('notes.list', { tags, mode });
    await c('notes.list', { q: 'note', tags, mode });
  }

  // every token the notes could have been indexed under, searched plainly
  const tokens = new Set();
  for (const md of [...MARKDOWN, 'new body with listen', 'body']) {
    for (const t of splitWhitespace(md)) tokens.add(t);
    for (const t of splitWhitespace(plain('', md))) tokens.add(t);
  }
  for (const t of tokens) await c('mem.search', { q: t, variations: false });

  for (const q of ['listen', 'rust', 'Society', 'monkeys', 'quick fox', 'quick fox jumps over', 'brown', 'things', '"listen"', 'note', 'light 5', 'tones 12 13']) await c('mem.search', { q });
  for (const b of ['ordinal', 'reversed', 'atbash', 'frequency', 'nonsense']) {
    for (const key of [79, 83, 87, 48, 52, 0, -3, 'listen', 'enlist', '79', ' 83 ', '+79', 'Λόγος', 'שלום light', 'truth 5', 'Note 0 listen', 99999999999]) await c('mem.gsearch', { bucket: b, key });
  }
  await c('mem.gsearch', { bucket: 'ordinal' });
  for (const t of ['truth', 'listen', '', 'a', 'Λόγος', 'שלום light', 'The quick brown fox', 'xyzzy plugh', 'Hello World', '12345']) await c('mem.measure', { text: t });
  for (const k of ['0', '1', '0,0', '0,7', '3', 'note:1000000', 'note:1000003', '1,2,3,4,5', ' 4 ', 'x']) await c('mem.remember', { key: k });
  await c('mem.all', {});
  await c('mem.all', { limit: 7 });

  // lists
  const pairs = [['0', '1'], ['0', '2'], ['0', 'note:1000000'], ['1', '0'], ['note:1000002', '4'], ['note:1000002', '6'], ['2', '0,5'], ['2', '0,6'], ['2', '1']];
  for (const [list, member] of pairs) await c('lists.enlist', { list, member });
  await c('lists.enlist', { list: 'bad', member: '1' });
  await c('lists.stats');
  await c('lists.delist', { list: '0', member: '2' });
  await c('mem.forget', { key: '6' });
  await c('lists.stats');
  await c('lists.unlist', { list: '1' });
  await c('lists.stats');

  // the space
  const space = texts.slice(0, 24).map((t, i) => ({ id: `m${i}`, text: t }));
  await c('space.spread', { texts: space });
  await c('space.spread', { texts: space, reversed: true });
  await c('space.spread', { texts: [...space.slice(0, 5), space[0]] });
  await c('space.spread', { texts: texts.slice(20, 170).map((t, i) => ({ id: `w${i}`, text: t })) });

  await c('stats');

  // saves and the forge
  await c('saves.new', { name: 'second' });
  await c('saves.new', { name: 'second' });
  for (const t of ['silent', 'enlist', 'stone', 'new in second', 'listen 3']) await c('mem.saturate', { text: t });
  await c('notes.save', { note: {}, text: `# Note 0 ${WORDS[0]}\n\n${MARKDOWN[0]}` });
  await c('notes.save', { note: {}, text: '# only in second\n\nbody' });
  await c('lists.enlist', { list: '0', member: '1' });
  await c('saves.list');
  const filters = [undefined, { words: 'listen' }, { words: '-1' }, { ordinal_from: 79, ordinal_to: 79 }, { reversed_from: 80, notes: false }, { lists: false, memories: true }];
  for (const filter of filters) {
    await c('forge.preview', { ingredients: [{ save: 'notes' }, { save: 'second', effect: 'take away' }], filter });
    await c('forge.preview', { ingredients: [{ save: 'notes', effect: 'add' }, { save: 'second', effect: 'keep shared' }], filter });
    await c('forge.preview', { ingredients: [{ save: 'second' }, { save: 'notes', effect: 'add' }], filter });
  }
  await c('forge.make', { name: 'forged', ingredients: [{ save: 'notes' }, { save: 'second', effect: 'take away' }], filter: { words: 'e' } });
  await c('forge.make', { name: 'forged', ingredients: [] });
  await c('forge.pour', { ingredients: [{ save: 'notes' }] });
  await c('saves.export', {});
  await c('saves.open', { name: 'notes' });
  await c('saves.open', { name: 'nope' });
  await c('saves.export', { name: 'forged' });
  await c('saves.export', { name: 'nope' });
  await c('saves.copy', { name: 'second' });
  await c('saves.copy', { name: 'second', to: 'third' });
  await c('saves.rename', { name: 'third', to: 'Third/renamed' });
  await c('saves.delete', { name: 'notes' });
  await c('saves.delete', { name: 'second copy' });
  await c('saves.list');

  // bundles: out, back in as a new save, merged into another
  const b = await c('saves.export', {});
  await c('saves.import', { bundle: b });
  await c('saves.import', { bundle: b, name: 'from bundle' });
  await c('saves.import', { bundle: null });
  await c('saves.export', { name: 'notes 2' });
  await c('saves.open', { name: 'forged' });
  await c('saves.import', { bundle: b, merge: true });
  await c('saves.import', { bundle: b, merge: true });
  await c('saves.export', {});
  // keys after an import continue from the highest one
  await c('saves.open', { name: 'from bundle' });
  for (const t of ['after import', 'listen', 'after import two']) await c('mem.saturate', { text: t });
  await c('notes.save', { note: {}, text: '# after import\n\nbody' });
  await c('saves.export', {});
  await c('hello');
  await c('saves.list');
  await c('nonsense');
  for (const name of ['notes', 'second', 'forged', 'Third-renamed', 'notes 2']) await c('saves.export', { name });
  // last, as it leaves the hub with a save the engine doesn't make (see HUB_BUG)
  await c('saves.import', { bundle: { ...b, anend: 2 } });
  await c('saves.list', {}, HUB_BUG);
  return log;
}

// ---------------------------------------------------------------- comparing

const TIME_KEYS = new Set(['created', 'modified', 'first', 'latest']);

// times and file sizes differ, and that's all that may
function normal(v, key) {
  if (Array.isArray(v)) return v.map((x) => normal(x, key === 'edits' ? 'edit' : undefined));
  if (v && typeof v === 'object') {
    const out = {};
    for (const k of Object.keys(v).sort()) {
      if (k === 'size') continue;
      out[k] = TIME_KEYS.has(k) && v[k] != null ? '<time>' : normal(v[k], k);
    }
    return out;
  }
  if (key === 'edit' && typeof v === 'string') return '<time>';
  return v;
}

const isFloat = (x) => typeof x === 'number' && !Number.isInteger(x);

// [equal, floats compared, floats bit for bit, worst relative difference]
function compare(a, b, path, st) {
  if (isFloat(a) || isFloat(b)) {
    if (typeof a !== 'number' || typeof b !== 'number') return fail(`${path}: ${a} vs ${b}`);
    st.floats++;
    if (a === b) st.exact++;
    const rel = Math.abs(a - b) / (1 + Math.abs(b));
    st.worst = Math.max(st.worst, rel);
    if (rel > 1e-6) fail(`${path}: ${a} vs ${b}`);
    return;
  }
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return fail(`${path}: ${JSON.stringify(a)?.slice(0, 300)} vs ${JSON.stringify(b)?.slice(0, 300)}`);
    a.forEach((x, i) => compare(x, b[i], `${path}[${i}]`, st));
    return;
  }
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    const ka = Object.keys(a);
    const kb = Object.keys(b);
    if (ka.join() !== kb.join()) return fail(`${path}: keys ${ka} vs ${kb}`);
    for (const k of ka) compare(a[k], b[k], `${path}.${k}`, st);
    return;
  }
  if (a !== b) fail(`${path}: ${JSON.stringify(a)} vs ${JSON.stringify(b)}`);
}

async function live() {
  const bin = process.env.HUB || join(root, 'target/debug/anend-hub');
  if (!existsSync(bin)) {
    console.log(`no hub at ${bin}: cargo build -p anend-hub first`);
    process.exitCode = 1;
    return;
  }
  const port = process.env.PORT || '18121';
  const data = mkdtempSync(join(process.env.SCRATCH || tmpdir(), 'anend-parity-'));
  const hub = spawn(bin, ['--local', '--addr', `127.0.0.1:${port}`, '--data', data], { stdio: ['ignore', 'pipe', 'pipe'] });
  let said = '';
  hub.stdout.on('data', (d) => (said += d));
  hub.stderr.on('data', (d) => (said += d));
  const base = `http://127.0.0.1:${port}/app/api/`;
  try {
    for (let i = 0; ; i++) {
      try {
        await fetch(base + 'hello', { method: 'POST', body: '{}' });
        break;
      } catch {
        if (i > 100) throw new Error(`the hub didn't start:\n${said}`);
        await new Promise((r) => setTimeout(r, 100));
      }
    }
    const hubCall = async (op, args) => {
      const r = await fetch(base + encodeURIComponent(op), { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(args) });
      const a = await r.json();
      return a.ok ? { ok: true, data: a.data } : { ok: false, error: a.error };
    };
    const engine = new Engine();
    const localCall = async (op, args) => {
      try {
        const data = engine.call(op, JSON.parse(JSON.stringify(args)));
        return { ok: true, data: JSON.parse(JSON.stringify(data ?? null)) };
      } catch (e) {
        return { ok: false, error: e.message };
      }
    };
    const want = await sequence(hubCall);
    const got = await sequence(localCall);
    const st = { floats: 0, exact: 0, worst: 0 };
    const errors = [];
    const ops = {};
    for (let i = 0; i < Math.max(want.length, got.length); i++) {
      const w = want[i];
      const g = got[i];
      const where = `#${i} ${w?.op} ${JSON.stringify(w?.args)?.slice(0, 80)}`;
      ops[w.op] = (ops[w.op] || 0) + 1;
      if (w.ok !== g.ok) {
        fail(`${where}: hub ${w.ok ? 'answered' : 'refused: ' + w.error}, engine ${g.ok ? 'answered' : 'refused: ' + g.error}`);
        continue;
      }
      if (!w.ok) {
        if (w.error !== g.error) errors.push(`${where}: hub "${w.error}", engine "${g.error}"`);
        continue;
      }
      if (w.known) {
        const before = failures;
        quiet = true;
        compare(normal(g.data), normal(w.data), where, st);
        quiet = false;
        if (failures > before) {
          failures = before;
          console.log(`  ! ${where}: ${w.known} (counted apart)`);
        }
        continue;
      }
      compare(normal(g.data), normal(w.data), where, st);
    }
    console.log(`live: ${want.length} calls each (${Object.entries(ops).map(([k, v]) => `${k} ${v}`).join(', ')})`);
    console.log(`  floats: ${st.floats}, ${st.exact} bit for bit, worst relative difference ${st.worst}`);
    if (errors.length) {
      console.log(`  ${errors.length} refusals worded differently (both refused):`);
      for (const e of errors) console.log('   ~', e);
    }
  } finally {
    hub.kill();
    await new Promise((r) => hub.once('exit', r));
    rmSync(data, { recursive: true, force: true });
  }
}

fixtures();
if (!process.argv.includes('--fixtures')) await live();
console.log(failures ? `${failures} differences` : 'all the same');
if (failures) process.exitCode = 1;
