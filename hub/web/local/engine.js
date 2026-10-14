// the hub's app (hub/src/app.rs) in the browser, for when there's no hub: the same ops
// answering the same way over saves held in memory. What's kept, and where, is the
// caller's business: `records()` hands out what changed, `load` takes it back.

import { Save, label, parseRef, readNote, splitNote, readBundle, importBundle, checkBundle } from './save.js';
import * as forge from './forge.js';
import { measure, spreadSums, trim, chars, charCount, debugStr, splitWhitespace, cmpStr } from './text.js';
import { gsumDetail, pfReduction } from './weum.js';
import { Spread, seedHue, PARAMS } from './spread.js';

const BUCKETS = ['ordinal', 'reversed', 'atbash', 'frequency'];

/// a save's name made safe for a file name (app.rs `clean`)
export function clean(name) {
  const n = trim(
    chars(trim(String(name)))
      .map((c) => (/[\p{Alphabetic}\p{N}]/u.test(c) || ' -_.'.includes(c) ? c : '-'))
      .join('')
      .replace(/^\.+|\.+$/g, ''),
  );
  if (n === '' || charCount(n) > 80) throw new Error("a save's name is 1 to 80 letters");
  return n;
}

// str::eq_ignore_ascii_case
function same(a, b) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    let x = a.charCodeAt(i);
    let y = b.charCodeAt(i);
    if (x >= 65 && x <= 90) x += 32;
    if (y >= 65 && y <= 90) y += 32;
    if (x !== y) return false;
  }
  return true;
}

const s = (args, k) => (args && typeof args[k] === 'string' ? args[k] : '');

// every tag the notes have and how many have it, the most used first
function tagCounts(notes) {
  const tags = new Map();
  for (const n of notes) for (const t of n.tags) tags.set(t, (tags.get(t) || 0) + 1);
  return [...tags].sort((a, b) => b[1] - a[1] || cmpStr(a[0], b[0]));
}

function hit(h) {
  const [o, r] = spreadSums(h.text);
  return { key: label(h.ref), text: h.text, note: h.ref.startsWith('note:') && h.ref.length > 4, ordinal: o, reversed: r };
}
const hits = (v) => v.map(hit);

function ref(l) {
  const r = parseRef(l);
  if (r === null) throw new Error(`${debugStr(l)} isn't a key (like 0,12 or note:1000001)`);
  return r;
}

/// everything worth knowing about a text's numbers (app.rs `measure_json`)
export function measureJson(text) {
  const m = measure(text);
  const [s_, r] = spreadSums(text);
  return {
    ordinal: m.ordinal,
    reversed: m.reversed,
    atbash: m.atbash,
    frequency: m.frequency,
    gsum: gsumDetail(text),
    pf_ordinal: s_ > 1 ? pfReduction(s_) : '',
    pf_reversed: r > 1 ? pfReduction(r) : '',
    triangle: Math.floor(Math.sqrt(Math.max(s_, 0) * Math.max(r, 0))),
    s: s_,
    r,
  };
}

function noteSummary(n) {
  return {
    key: n._key,
    title: n.title === '' ? 'untitled' : n.title,
    tags: n.tags,
    created: n.created,
    modified: n.edits.length ? n.edits[n.edits.length - 1] : n.created,
    edits: n.edits.length,
    preview: chars(n.markdown).slice(0, 160).join(''),
    words: splitWhitespace(n.markdown).length,
  };
}

// the letters of texts as a unit vector, and two of them compared (lists.rs)
function letterVector(texts) {
  const v = new Array(26).fill(0);
  for (const t of texts) {
    for (let i = 0; i < t.length; i++) {
      const b = t.charCodeAt(i);
      if (b >= 128) continue;
      const lc = b | 0x20;
      if (lc >= 97 && lc <= 122) v[lc - 97] += 1;
    }
  }
  let sq = 0;
  for (const x of v) sq += x * x;
  const n = Math.sqrt(sq);
  if (n > 0) for (let i = 0; i < 26; i++) v[i] /= n;
  return v;
}
const cosine = (a, b) => a.reduce((acc, x, i) => acc + x * b[i], 0);
// f64::round: halves away from zero
const round = (x) => (x < 0 ? -Math.round(-x) : Math.round(x));

function listStats(save) {
  const seen = new Set();
  const lists = [];
  for (const [l] of save.allLists()) {
    if (seen.has(l)) continue;
    seen.add(l);
    const members = save.list(l);
    const h = save.hit(l);
    let so = 0;
    let sr = 0;
    for (const m of members) {
      const [o, r] = spreadSums(m.text);
      so += o;
      sr += r;
    }
    lists.push({ l, name: h ? h.text : label(l), members, so, sr, joined: members.map((m) => m.text).join(' '), vector: letterVector(members.map((m) => m.text)) });
  }
  // like the hub, no grid past 1000 lists (lists² numbers)
  const dot = lists.length > 1000 ? [] : lists.map((a) => lists.map((b) => round(cosine(a.vector, b.vector) * 1000) / 1000));
  return {
    lists: lists.map((x) => ({
      list: label(x.l),
      name: x.name,
      members: hits(x.members),
      count: x.members.length,
      ordinal_sum: x.so,
      reversed_sum: x.sr,
      ordinal_mean: x.so / Math.max(x.members.length, 1),
      reversed_mean: x.sr / Math.max(x.members.length, 1),
      whole: measureJson(x.joined),
      vector: x.vector,
    })),
    dot,
  };
}

function spreadLayout(args) {
  const reversed = args && typeof args.reversed === 'boolean' ? args.reversed : false;
  const sp = new Spread({ reversed });
  const items = args && Array.isArray(args.texts) ? args.texts : [];
  if (items.length > 2000) throw new Error('at most 2000 texts in a spread');
  for (const it of items) {
    const text = s(it, 'text');
    // about the size the app's texts are: half a unit per character, half a unit high
    const hw = Math.min(charCount(text) * 0.27 + 0.1, 12);
    sp.add(Spread.node(s(it, 'id'), text, hw, 0.35));
  }
  sp.rekey();
  sp.wake(1);
  sp.settle(4000, 400);
  return {
    d: PARAMS.d,
    nodes: sp.nodes.map((n) => ({ id: n.id, x: n.x, y: n.y, z: n.z, s: n.s, r: n.r, q: n.q, hue: seedHue(n.q) })),
    buckets: sp.order.length,
  };
}

const EFFECTS = { 'take away': 'take away', take_away: 'take away', '-': 'take away', 'keep shared': 'keep shared', keep_shared: 'keep shared', shared: 'keep shared' };
const i64 = (v) => (Number.isInteger(v) ? v : null);
const flag = (v) => (typeof v === 'boolean' ? v : true);

/// what every classify.* call answers without a hub
export const CLASSIFIERS_NEED_A_HUB = 'classifiers need a hub for now';

export class Engine {
  /// `kept`: {saves: [{name, bundle, allocator, modified}], open}
  constructor(kept = {}) {
    this.kept = new Map(); // name -> what's kept of a save not loaded yet
    this.loaded = new Map(); // name -> Save
    this.written = new Map(); // name -> the version last handed out
    this.gone = new Set();
    this.last = kept.open || null;
    this.openName = null;
    this.openChanged = false;
    for (const r of kept.saves || []) this.kept.set(r.name, r);
  }

  // ---- the workspace

  names() {
    return [...new Set([...this.kept.keys(), ...this.loaded.keys()])].sort((a, b) => cmpStr(a.toLowerCase(), b.toLowerCase()));
  }

  exists(name) {
    const n = clean(name);
    return this.kept.has(n) || this.loaded.has(n);
  }

  get(name) {
    const n = clean(name);
    if (this.loaded.has(n)) return this.loaded.get(n);
    const r = this.kept.get(n);
    // like Store::open, a name that isn't there yet is a new, empty save
    const save = r ? Save.from(readBundle(r.bundle), r.allocator) : new Save();
    if (r) {
      save.version = 0;
      save.modified = r.modified || save.modified;
      this.written.set(n, 0);
    }
    this.kept.delete(n);
    this.loaded.set(n, save);
    this.gone.delete(n);
    return save;
  }

  /// the open save, opening the last one (or making `notes`) the first time
  store() {
    if (this.openName === null) {
      const names = this.names();
      const name = this.last && names.includes(this.last) ? this.last : names[0] || 'notes';
      this.get(name);
      this.openName = name;
    }
    return [this.openName, this.loaded.get(this.openName)];
  }

  switch(name) {
    const n = clean(name);
    this.get(n);
    this.openName = n;
    this.last = n;
    this.openChanged = true;
    return this.store();
  }

  remove(name) {
    const n = clean(name);
    if (!this.kept.has(n) && !this.loaded.has(n)) throw new Error('No such file or directory (os error 2)');
    this.kept.delete(n);
    this.loaded.delete(n);
    this.written.delete(n);
    this.gone.add(n);
  }

  /// what changed since the last time: {put: [{name, bundle, allocator, modified}], gone, open}
  records() {
    const put = [];
    for (const [name, save] of this.loaded) {
      if (this.written.get(name) === save.version) continue;
      this.written.set(name, save.version);
      put.push({ name, bundle: save.bundle(name), allocator: save.allocator(), modified: save.modified });
    }
    const gone = [...this.gone];
    this.gone.clear();
    const open = this.openChanged || put.length ? this.openName : null;
    this.openChanged = false;
    return { put, gone, open };
  }

  saveInfo(name, open) {
    const save = this.loaded.get(name);
    const kept = this.kept.get(name);
    const counts = save ? save.counts() : { notes: kept.bundle.notes.length, memories: kept.bundle.memories.length };
    const size = save ? JSON.stringify(save.bundle(name)).length : JSON.stringify(kept.bundle).length;
    return { name, open: same(name, open), notes: counts.notes, memories: counts.memories, size, modified: save ? save.modified : kept.modified || null };
  }

  // ---- the calls

  /// one operation, answered like the hub answers it (throws its error)
  call(op, args = {}) {
    args = args ?? {};
    if (op.startsWith('saves.')) return this.savesCall(this.store()[0], op, args);
    // the classifiers' engine (core/src/classify) isn't ported to the browser yet
    if (op.startsWith('classify.') || op.startsWith('filters.')) throw new Error(CLASSIFIERS_NEED_A_HUB);
    const [open, save] = this.store();
    switch (op) {
      case 'hello':
        return { save: open, saves: this.names(), counts: save.counts() };

      // ---- notes
      case 'notes.list': {
        const q = trim(s(args, 'q'));
        const keep = q === '' ? null : save.noteFilter(q);
        // tags: the notes must have all of them (or any, with mode "or")
        const tags = Array.isArray(args.tags) ? args.tags.filter((t) => typeof t === 'string') : [];
        const any = s(args, 'mode') === 'or';
        const tagged = (n) => tags.length === 0 || (any ? tags.some((t) => n.tags.includes(t)) : tags.every((t) => n.tags.includes(t)));
        return save.all().filter((n) => (!keep || keep.has(n._key)) && tagged(n)).map(noteSummary);
      }
      case 'notes.tags':
        return tagCounts(save.all()).map(([tag, count]) => ({ tag, count }));
      case 'notes.get': {
        const n = save.all().find((n) => n._key === s(args, 'key'));
        if (!n) throw new Error('no such note');
        return n;
      }
      case 'notes.save': {
        const note = readNote(args.note ?? null);
        if (typeof args.text === 'string') [note.title, note.markdown] = splitNote(args.text);
        if (trim(note.title) === '') throw new Error('a note needs a # title');
        // keep what the editor didn't send
        const old = note._key !== '' && save.notes.get(note._key);
        if (old) {
          note.created = old.created;
          note.edits = [...old.edits];
        }
        return save.save(note);
      }
      case 'notes.delete':
        save.delete(s(args, 'key'));
        return null;

      // ---- memories
      case 'mem.measure':
        return measureJson(s(args, 'text'));
      case 'mem.gsearch': {
        const b = BUCKETS.includes(s(args, 'bucket')) ? s(args, 'bucket') : 'ordinal';
        const k = args.key;
        let key;
        if (b === 'frequency' && typeof k === 'string') key = measure(k).frequency;
        else if (typeof k === 'number') key = Number.isInteger(k) && k >= 0 ? k % 4294967296 : 0;
        else if (typeof k === 'string') {
          const t = trim(k);
          key = /^\+?\d+$/.test(t) && +t <= 4294967295 && b !== 'frequency' ? +t : b === 'frequency' ? measure(k).frequency : measure(k)[b];
        } else throw new Error('gsearch needs a key: a number, or a text to measure');
        return { bucket: b, hits: hits(save.gsearch(b, key)) };
      }
      case 'mem.search': {
        const q = s(args, 'q');
        return hits(typeof args.variations === 'boolean' && !args.variations ? save.search(q) : save.searchWithVariations(q));
      }
      case 'mem.saturate': {
        const text = trim(s(args, 'text'));
        if (text === '') throw new Error('nothing to remember');
        const [key, fresh] = save.saturate(text);
        return { key: label(key), new: fresh };
      }
      case 'mem.forget': {
        const r = ref(s(args, 'key'));
        if (r.startsWith('note:') && r.length > 4) throw new Error('notes are deleted from the editor');
        return { text: save.forget(r) };
      }
      case 'mem.remember':
        return hit(save.remember(ref(s(args, 'key'))));
      case 'mem.all': {
        const limit = Number.isInteger(args.limit) && args.limit >= 0 ? args.limit : 5000;
        return save.allMemories().slice(0, limit).map(([text, k]) => hit({ ref: k, text }));
      }

      // ---- lists
      case 'lists.stats':
        return listStats(save);
      case 'lists.enlist':
        save.enlist(ref(s(args, 'list')), ref(s(args, 'member')));
        return null;
      case 'lists.delist':
        save.delist(ref(s(args, 'list')), ref(s(args, 'member')));
        return null;
      case 'lists.unlist':
        return { removed: save.unlist(ref(s(args, 'list'))) };

      // ---- the space
      case 'space.spread':
        return spreadLayout(args);

      // ---- the forge
      case 'forge.preview': {
        const c = this.forgeParts(args);
        return {
          memories: c.memories.length,
          notes: c.notes.length,
          lists: c.lists.length,
          sample: c.memories.slice(0, 24).map((t) => {
            const [o, r] = spreadSums(t);
            return { text: t, ordinal: o, reversed: r };
          }),
          titles: c.notes.slice(0, 12).map((n) => (n.title === '' ? 'untitled' : n.title)),
        };
      }
      case 'forge.make': {
        const name = clean(s(args, 'name'));
        if (this.exists(name)) throw new Error("there's already a save called that");
        const c = this.forgeParts(args);
        const w = forge.writeInto(this.get(name), c);
        return { save: name, ...w };
      }
      case 'forge.pour':
        return forge.writeInto(save, this.forgeParts(args));

      // ---- stats
      case 'stats': {
        const notes = save.all();
        const words = notes.reduce((a, n) => a + splitWhitespace(n.markdown).length, 0);
        const edits = notes.reduce((a, n) => a + n.edits.length, 0);
        const sorted = tagCounts(notes);
        return {
          notes: notes.length,
          words,
          saves: notes.length + edits,
          memories: save.memoryCount(),
          tags: sorted,
          first: notes.length ? notes[notes.length - 1].created : null,
          latest: notes.length ? notes[0].created : null,
        };
      }
    }
    throw new Error(`there's no ${debugStr(op)} here`);
  }

  forgeParts(args) {
    const [openName, open] = this.store();
    const parts = [];
    const ingredients = Array.isArray(args.ingredients) ? args.ingredients : [];
    if (ingredients.length > 32) throw new Error('the forge takes at most 32 saves at once');
    for (const ing of ingredients) {
      const name = s(ing, 'save');
      const effect = EFFECTS[s(ing, 'effect')] || 'add';
      // the hub opens (and so makes) a save it doesn't have; here it's just empty
      const save = same(name, openName) ? open : this.exists(name) ? this.get(name) : new Save();
      parts.push([effect, forge.read(save)]);
    }
    const f = args.filter && typeof args.filter === 'object' ? args.filter : {};
    return forge.filter(
      {
        words: s(f, 'words'),
        ordinal: [i64(f.ordinal_from), i64(f.ordinal_to)],
        reversed: [i64(f.reversed_from), i64(f.reversed_to)],
        memories: flag(f.memories),
        notes: flag(f.notes),
        lists: flag(f.lists),
      },
      forge.combine(parts),
    );
  }

  /// the save operations (app.rs `saves_call`)
  savesCall(open, op, args) {
    switch (op) {
      case 'saves.list':
        return this.names().map((n) => this.saveInfo(n, open));
      case 'saves.open': {
        const name = s(args, 'name');
        if (!this.exists(name)) throw new Error("there's no save by that name");
        return { save: this.switch(name)[0] };
      }
      case 'saves.new': {
        const name = clean(s(args, 'name'));
        if (this.exists(name)) throw new Error("there's already a save called that");
        const [n, save] = this.switch(name);
        save.changed();
        return { save: n };
      }
      case 'saves.copy': {
        const from = s(args, 'name');
        let to;
        try {
          to = clean(s(args, 'to'));
        } catch {
          to = clean(`${from} copy`);
        }
        if (this.exists(to)) throw new Error("there's already a save called that");
        if (!this.exists(from)) throw new Error('No such file or directory (os error 2)');
        const src = this.get(from);
        const copy = Save.from(readBundle(src.bundle(to)), src.allocator());
        this.loaded.set(to, copy);
        return { save: to };
      }
      case 'saves.rename': {
        const from = s(args, 'name');
        const to = clean(s(args, 'to'));
        if (this.exists(to)) throw new Error("there's already a save called that");
        const wasOpen = same(from, open);
        if (!this.exists(from)) throw new Error('No such file or directory (os error 2)');
        const save = this.get(from);
        this.remove(from);
        save.changed();
        this.loaded.set(to, save);
        if (wasOpen) {
          this.openName = null;
          this.switch(to);
        }
        return { save: to };
      }
      case 'saves.delete': {
        const name = s(args, 'name');
        if (same(name, open)) throw new Error('open another save before deleting this one');
        this.remove(name);
        return null;
      }
      case 'saves.export': {
        const name = s(args, 'name') || open;
        if (same(name, open)) return this.loaded.get(open).bundle(open);
        if (!this.exists(name)) throw new Error("there's no save by that name");
        return this.get(name).bundle(name);
      }
      case 'saves.import': {
        const b = readBundle(args.bundle ?? null);
        if (typeof args.merge === 'boolean' && args.merge) {
          checkBundle(b);
          const w = forge.writeInto(this.loaded.get(open), forge.bundleContents(b, parseRef));
          return { merged: true, ...w };
        }
        const want = s(args, 'name') || b.name || 'imported';
        let n = clean(want);
        for (let i = 2; this.exists(n); i++) n = clean(`${want} ${i}`);
        checkBundle(b);
        const save = new Save();
        const w = importBundle(save, b);
        this.loaded.set(n, save);
        return { save: n, ...w };
      }
    }
    throw new Error(`there's no ${debugStr(op)} here`);
  }
}

