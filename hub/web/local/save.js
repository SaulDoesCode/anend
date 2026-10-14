// one save, in memory: core/src/store.rs and memory.rs over maps instead of redb tables.
// Keys, refs and tokens are binary strings, read back in redb's byte order, so hits,
// keys and lists come out in the order the hub gives them. What's kept is a bundle
// (core/src/bundle.rs) and the allocator's state; the search tables are rebuilt from it.

import { bin, unbin, bytesOf, fromBytes, cmpBin, cmpStr, measure, uniqueTokens, trim, trimStart, debugStr, splitWhitespace } from './text.js';
import { searchVariations } from './weum.js';
import { plain } from './plain.js';

export const NOTE_PREFIX = 'note:';

// ---------------------------------------------------------------- refs

/// a ref's bytes: a memory's key, or `note:` and a note's key
export const noteRef = (key) => NOTE_PREFIX + bin(key);
export const isNoteRef = (b) => b.startsWith(NOTE_PREFIX) && b.length > 4;

/// how weum shows a ref: "0,12", or "note:<key>"
export function label(b) {
  return isNoteRef(b) ? `note:${unbin(b.slice(5))}` : bytesOf(b).join(',');
}

/// a label typed into the term bar, back to a ref (memory.rs Ref::parse)
export function parseRef(l) {
  l = trim(l);
  if (l.startsWith('note:')) {
    const k = trim(l.slice(5));
    return k ? noteRef(k) : null;
  }
  const parts = l.split(',').map((p) => trim(p));
  if (!parts.length || parts.length > 4) return null;
  const bytes = [];
  for (const p of parts) {
    if (!/^\+?\d+$/.test(p) || +p > 255) return null;
    bytes.push(+p);
  }
  return fromBytes(bytes);
}

/// Rust's `u64::from_str`, or null
export function parseU64(s) {
  if (typeof s !== 'string' || !/^\+?\d+$/.test(s)) return null;
  const n = BigInt(s);
  return n > 18446744073709551615n ? null : n;
}

// ---------------------------------------------------------------- notes

export function now() {
  // RFC 3339 in local time like chrono's to_rfc3339, nanoseconds and all; never twice
  // the same, so newest first stays the order things were made in
  let ms = Date.now();
  if (ms <= now.ms) {
    now.sub = (now.sub || 0) + 1;
    ms = now.ms;
  } else {
    now.ms = ms;
    now.sub = 0;
  }
  const d = new Date(ms);
  const p = (n, w = 2) => String(n).padStart(w, '0');
  const off = -d.getTimezoneOffset();
  const sign = off < 0 ? '-' : '+';
  const nanos = p(d.getMilliseconds(), 3) + p(now.sub, 6);
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${nanos}${sign}${p(Math.floor(Math.abs(off) / 60))}:${p(Math.abs(off) % 60)}`;
}
now.ms = 0;

/// an RFC 3339 time as [seconds, nanoseconds] since the epoch, or null
export function parseTime(s) {
  const m = /^(\d{4})-(\d{2})-(\d{2})[Tt ](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?([Zz]|[+-]\d{2}:\d{2})$/.exec(s || '');
  if (!m) return null;
  const secs = Date.parse(`${m[1]}-${m[2]}-${m[3]}T${m[4]}:${m[5]}:${m[6] === '60' ? '59' : m[6]}${m[8].toUpperCase()}`);
  if (Number.isNaN(secs)) return null;
  const nanos = m[7] ? +m[7].slice(0, 9).padEnd(9, '0') : 0;
  return [secs / 1000, nanos];
}

const timeCmp = (a, b) => a[0] - b[0] || a[1] - b[1];

/// newest first by creation time, notes without a readable one last (stable)
export function sortNewestFirst(notes) {
  const t = new Map(notes.map((n) => [n, parseTime(n.created)]));
  return notes.sort((a, b) => {
    const x = t.get(a);
    const y = t.get(b);
    if (!x || !y) return x ? -1 : y ? 1 : 0;
    return timeCmp(y, x);
  });
}

const displayTitle = (n) => (n.title === '' ? 'untitled' : n.title);

const isStr = (v) => typeof v === 'string';
const strList = (v) => Array.isArray(v) && v.every(isStr);

/// a note as serde reads one: every field optional, wrong types refused, extras dropped
export function readNote(v, what = 'that isn\'t a note') {
  if (!v || typeof v !== 'object' || Array.isArray(v)) throw new Error(`${what}: invalid type: ${v === null || v === undefined ? 'null' : Array.isArray(v) ? 'sequence' : typeof v}, expected struct Note`);
  const n = { _key: '', title: '', markdown: '', tags: [], created: '', edits: [] };
  for (const k of ['_key', 'title', 'markdown', 'created']) {
    if (k in v) {
      if (!isStr(v[k])) throw new Error(`${what}: invalid type for \`${k}\`, expected a string`);
      n[k] = v[k];
    }
  }
  for (const k of ['tags', 'edits']) {
    if (k in v) {
      if (!strList(v[k])) throw new Error(`${what}: invalid type for \`${k}\`, expected a sequence of strings`);
      n[k] = [...v[k]];
    }
  }
  return n;
}

const copyNote = (n) => ({ _key: n._key, title: n.title, markdown: n.markdown, tags: [...n.tags], created: n.created, edits: [...n.edits] });

/// note.rs `split`: the editor's `# title` line and the rest
export function splitNote(text) {
  const lines = text.split('\n');
  // str::lines: a trailing newline doesn't make an empty last line, and \r\n counts as one
  if (lines.length && lines[lines.length - 1] === '') lines.pop();
  for (let i = 0; i < lines.length; i++) if (lines[i].endsWith('\r')) lines[i] = lines[i].slice(0, -1);
  let title = '';
  let at = -1;
  for (let i = 0; i < lines.length; i++) {
    const t = trimStart(lines[i]);
    if (t.startsWith('#')) {
      title = trim(t.replace(/^#+/, ''));
      at = i;
      break;
    }
  }
  return [title, trim(lines.filter((_, i) => i !== at).join('\n'))];
}

// ---------------------------------------------------------------- key allocation

/// weum's next key: 0..=255, then two bytes, three, four
export function nextKey(b) {
  const a = bytesOf(b);
  const be = (bytes) => bytes.reduce((x, y) => x * 256 + y, 0);
  const of = (n, w) => {
    const out = [];
    for (let i = 0; i < w; i++) {
      out.unshift(n % 256);
      n = Math.floor(n / 256);
    }
    return fromBytes(out);
  };
  switch (a.length) {
    case 1:
      return a[0] === 255 ? fromBytes([0, 0]) : fromBytes([a[0] + 1]);
    case 2:
      return be(a) === 0xffff ? fromBytes([0, 0, 0]) : of(be(a) + 1, 2);
    case 3:
      return be(a) === 0xffffff ? fromBytes([0, 0, 0, 0]) : of(be(a) + 1, 3);
    case 4:
      if (be(a) === 0xffffffff) throw new Error('every memory key is taken');
      return of(be(a) + 1, 4);
    default:
      throw new Error('weird sized allocated, not normal');
  }
}

/// allocation order: shorter keys first, then by value
const keyOrder = (a, b) => a.length - b.length || cmpBin(a, b);

// ---------------------------------------------------------------- multimaps

class Multimap {
  constructor() {
    this.m = new Map();
  }
  add(k, v) {
    let s = this.m.get(k);
    if (!s) this.m.set(k, (s = new Set()));
    s.add(v);
  }
  remove(k, v) {
    const s = this.m.get(k);
    if (!s) return false;
    const had = s.delete(v);
    if (!s.size) this.m.delete(k);
    return had;
  }
  /// a key's values in byte order
  get(k) {
    const s = this.m.get(k);
    return s ? [...s].sort(cmpBin) : [];
  }
  removeAll(k) {
    const s = this.m.get(k);
    this.m.delete(k);
    return s ? s.size : 0;
  }
  keys() {
    return [...this.m.keys()].sort(cmpBin);
  }
}

// ---------------------------------------------------------------- the save

export class Save {
  constructor() {
    this.notes = new Map(); // key -> note
    this.nextNote = null; // meta's next_key
    this.memories = new Map(); // key bytes -> text
    this.allocated = null; // the last key minted
    this.reuse = new Set(); // forgotten keys, handed out again first
    this.tokens = new Multimap();
    this.sums = { ordinal: new Multimap(), reversed: new Multimap(), atbash: new Multimap() };
    this.frequency = new Multimap();
    this.lists = new Multimap();
    this.settings = new Map(); // name -> hex
    // a hub's classifiers, carried as they came (bundle.rs `classifiers`): kept, not run
    this.classifiers = [];
    this.version = 0; // bumped by every change, for the write-through
    this.modified = now();
  }

  changed() {
    this.version++;
    this.modified = now();
  }

  counts() {
    return { notes: this.notes.size, memories: this.memories.size };
  }

  // ---- the search tables

  index(ref, gem, tokenText, add) {
    for (const tk of uniqueTokens(tokenText)) add ? this.tokens.add(tk, ref) : this.tokens.remove(tk, ref);
    if (trim(gem) === '') return;
    const m = measure(gem);
    for (const b of ['ordinal', 'reversed', 'atbash']) add ? this.sums[b].add(m[b], ref) : this.sums[b].remove(m[b], ref);
    add ? this.frequency.add(m.frequency, ref) : this.frequency.remove(m.frequency, ref);
  }

  indexNote(n, add) {
    this.index(noteRef(n._key), n.title, plain(n.title, n.markdown), add);
  }

  hit(ref) {
    if (isNoteRef(ref)) {
      const n = this.notes.get(unbin(ref.slice(5)));
      return n ? { ref, text: displayTitle(n) } : null;
    }
    const t = this.memories.get(ref);
    return t === undefined ? null : { ref, text: t };
  }

  hits(refs) {
    const out = [];
    for (const r of refs) {
      const h = this.hit(r);
      if (h) out.push(h);
    }
    return out;
  }

  // ---- notes (store.rs)

  /// every note, newest first
  all() {
    const keys = [...this.notes.keys()].sort(cmpStr);
    return sortNewestFirst(keys.map((k) => copyNote(this.notes.get(k))));
  }

  save(note) {
    const n = copyNote(note);
    const previous = n._key === '' ? null : this.notes.get(n._key);
    if (previous) this.indexNote(previous, false);
    if (n._key === '') {
      const next = this.nextNote ?? 1000000n;
      this.nextNote = next + 1n;
      n._key = String(next);
      n.created = now();
    } else {
      if (n.created === '') n.created = now();
      n.edits.push(now());
    }
    this.notes.set(n._key, n);
    this.indexNote(n, true);
    this.changed();
    return copyNote(n);
  }

  delete(key) {
    const n = this.notes.get(key);
    if (!n) throw new Error(`there is no note ${debugStr(key)}`);
    this.notes.delete(key);
    this.indexNote(n, false);
    this.changed();
  }

  /// notes as they are, keeping their keys and dates
  import(notes) {
    let next = this.nextNote ?? 1000000n;
    for (const note of notes) {
      const n = copyNote(note);
      if (n._key === '') {
        n._key = String(next);
        next += 1n;
      } else {
        const k = parseU64(n._key);
        if (k !== null && k + 1n > next) next = k + 1n;
      }
      if (n.created === '') n.created = now();
      const old = this.notes.get(n._key);
      this.notes.set(n._key, n);
      if (old) this.indexNote(old, false);
      this.indexNote(n, true);
    }
    this.nextNote = next;
    this.changed();
    return notes.length;
  }

  // ---- memories (memory.rs)

  addMemory(text) {
    let key;
    if (this.reuse.size) {
      key = [...this.reuse].sort(cmpBin)[0];
      this.reuse.delete(key);
    } else {
      key = this.allocated === null ? fromBytes([0]) : nextKey(this.allocated);
      this.allocated = key;
    }
    this.memories.set(key, text);
    this.index(key, text, text, true);
    return key;
  }

  saturateNew(text) {
    const k = this.addMemory(text);
    this.changed();
    return k;
  }

  saturateMany(texts) {
    const keys = texts.map((t) => this.addMemory(t));
    if (texts.length) this.changed();
    return keys;
  }

  findExact(text) {
    for (const h of this.gsearch('ordinal', measure(text).ordinal)) if (!isNoteRef(h.ref) && h.text === text) return h.ref;
    return null;
  }

  /// [key, new]
  saturate(text) {
    const k = this.findExact(text);
    return k !== null ? [k, false] : [this.saturateNew(text), true];
  }

  remember(ref) {
    const h = this.hit(ref);
    if (!h) throw new Error("didn't find that");
    return h;
  }

  forget(key) {
    const text = this.memories.get(key);
    if (text === undefined) throw new Error('there was nothing with that name anyways');
    this.memories.delete(key);
    this.reuse.add(key);
    this.index(key, text, text, false);
    // drop it from every list, and drop the list it named
    for (const l of this.lists.keys()) for (const m of this.lists.get(l)) if (m === key || l === key) this.lists.remove(l, m);
    this.changed();
    return text;
  }

  /// weum's token search: refs under at least three quarters of the query's tokens
  search(query) {
    const tokens = uniqueTokens(query);
    if (!tokens.length) return [];
    const threshold = Math.max(Math.ceil(Math.fround(tokens.length * 0.75)), 1);
    const scores = new Map();
    const order = [];
    for (const tk of tokens) {
      for (const r of this.tokens.get(tk)) {
        const n = scores.get(r) || 0;
        if (n === 0) order.push(r);
        scores.set(r, n + 1);
      }
    }
    return this.hits(order.filter((r) => scores.get(r) >= threshold));
  }

  searchWithVariations(query) {
    const seen = new Set();
    const out = [];
    for (const q of searchVariations(trim(query))) {
      for (const h of this.search(q)) {
        if (!seen.has(h.ref)) {
          seen.add(h.ref);
          out.push(h);
        }
      }
    }
    return out;
  }

  /// everything under a key in a bucket; a number searches the sums (the ordinal ones for
  /// the frequency bucket, as the hub does), a string the letter table
  gsearch(bucket, key) {
    if (typeof key === 'string') return bucket === 'frequency' ? this.hits(this.frequency.get(key)) : [];
    const table = bucket === 'reversed' || bucket === 'atbash' ? this.sums[bucket] : this.sums.ordinal;
    return this.hits(table.get(key));
  }

  /// keys of the notes with a token starting with every word of `query`, any case
  noteFilter(query) {
    let result = null;
    const keys = this.tokens.keys();
    for (const word of splitWhitespace(query)) {
      const found = new Set();
      const lower = word.toLowerCase();
      const cap = [...lower];
      const forms = new Set([word, lower, cap.length ? cap[0].toUpperCase() + cap.slice(1).join('') : '']);
      for (const form of forms) {
        const f = bin(form);
        for (const tk of keys) {
          if (!tk.startsWith(f)) continue;
          for (const r of this.tokens.get(tk)) if (isNoteRef(r)) found.add(unbin(r.slice(5)));
        }
      }
      result = result === null ? found : new Set([...result].filter((k) => found.has(k)));
    }
    return result || new Set();
  }

  // ---- lists

  enlist(list, member) {
    this.lists.add(list, member);
    this.changed();
  }

  enlistMany(pairs) {
    for (const [l, m] of pairs) this.lists.add(l, m);
    this.changed();
  }

  delist(list, member) {
    this.lists.remove(list, member);
    this.changed();
  }

  /// [list, member] pairs as stored
  allLists() {
    const out = [];
    for (const l of this.lists.keys()) for (const m of this.lists.get(l)) out.push([l, m]);
    return out;
  }

  list(ref) {
    return this.hits(this.lists.get(ref));
  }

  unlist(ref) {
    const n = this.lists.removeAll(ref);
    this.changed();
    return n;
  }

  /// every memory, [text, key], in key order
  allMemories() {
    return [...this.memories.keys()].sort(cmpBin).map((k) => [this.memories.get(k), k]);
  }

  /// weum's backup, keys kept
  importMemories(entries) {
    let highest = null;
    let n = 0;
    for (const [text, key] of entries) {
      if (key.length === 0 || key.length > 4) continue;
      const old = this.memories.get(key);
      this.memories.set(key, text);
      if (old !== undefined) this.index(key, old, old, false);
      this.index(key, text, text, true);
      if (highest === null || keyOrder(key, highest) > 0) highest = key;
      n++;
    }
    if (highest !== null && (this.allocated === null || keyOrder(highest, this.allocated) > 0)) this.allocated = highest;
    this.changed();
    return n;
  }

  memoryCount() {
    return this.memories.size;
  }

  // ---- what's kept

  /// the save as a bundle (bundle.rs `export`)
  bundle(name) {
    const settings = {};
    for (const k of [...this.settings.keys()].sort(cmpStr)) settings[k] = this.settings.get(k);
    const memories = this.allMemories().sort((a, b) => keyOrder(a[1], b[1]));
    return {
      anend: 1,
      name,
      notes: this.all(),
      memories: memories.map(([t, k]) => [t, bytesOf(k)]),
      lists: this.allLists().map(([l, m]) => [label(l), label(m)]),
      settings,
      ...(this.classifiers.length ? { classifiers: this.classifiers } : {}),
    };
  }

  /// the allocator's state, which a bundle doesn't carry
  allocator() {
    return {
      allocated: this.allocated === null ? null : bytesOf(this.allocated),
      reuse: [...this.reuse].sort(cmpBin).map(bytesOf),
      next_note: this.nextNote === null ? null : String(this.nextNote),
    };
  }

  /// a save from what was kept: a checked bundle and, if there is one, the allocator
  static from(b, alloc) {
    const s = new Save();
    importBundle(s, b);
    if (alloc) {
      if (alloc.allocated) s.allocated = fromBytes(alloc.allocated);
      s.reuse = new Set((alloc.reuse || []).map(fromBytes));
      if (alloc.next_note != null) s.nextNote = BigInt(alloc.next_note);
    }
    return s;
  }
}

// ---------------------------------------------------------------- bundles

/// a bundle as serde reads one (bundle.rs `Bundle`), or an error saying why not
export function readBundle(v) {
  const no = (why) => {
    throw new Error(`that bundle doesn't read: ${why}`);
  };
  if (!v || typeof v !== 'object' || Array.isArray(v)) no(`invalid type: ${v == null ? 'null' : Array.isArray(v) ? 'sequence' : typeof v}, expected struct Bundle`);
  if (!('anend' in v)) no('missing field `anend`');
  if (!Number.isInteger(v.anend) || v.anend < 0 || v.anend > 4294967295) no('invalid value for `anend`, expected u32');
  const b = { anend: v.anend, name: '', notes: [], memories: [], lists: [], settings: {} };
  if ('name' in v) {
    if (!isStr(v.name)) no('invalid type for `name`, expected a string');
    b.name = v.name;
  }
  if ('notes' in v) {
    if (!Array.isArray(v.notes)) no('invalid type for `notes`, expected a sequence');
    b.notes = v.notes.map((n) => {
      try {
        return readNote(n, 'a note');
      } catch (e) {
        return no(e.message);
      }
    });
  }
  if ('memories' in v) {
    if (!Array.isArray(v.memories)) no('invalid type for `memories`, expected a sequence');
    b.memories = v.memories.map((m) => {
      if (!Array.isArray(m) || m.length !== 2 || !isStr(m[0]) || !Array.isArray(m[1]) || !m[1].every((x) => Number.isInteger(x) && x >= 0 && x <= 255)) no('a memory is [text, [key bytes]]');
      return [m[0], [...m[1]]];
    });
  }
  if ('lists' in v) {
    if (!Array.isArray(v.lists) || !v.lists.every((p) => Array.isArray(p) && p.length === 2 && isStr(p[0]) && isStr(p[1]))) no('lists are [list, member] labels');
    b.lists = v.lists.map((p) => [p[0], p[1]]);
  }
  if ('settings' in v) {
    if (!v.settings || typeof v.settings !== 'object' || Array.isArray(v.settings) || !Object.values(v.settings).every(isStr)) no('settings are names to hex');
    b.settings = { ...v.settings };
  }
  if ('classifiers' in v) {
    if (!Array.isArray(v.classifiers) || !v.classifiers.every((c) => c && typeof c === 'object' && c.definition && typeof c.definition === 'object')) no('classifiers are {definition, weights?, feedback?}');
    b.classifiers = v.classifiers;
  }
  return b;
}

export function checkBundle(b) {
  if (b.anend === 0 || b.anend > 1) throw new Error(`that isn't an anend save bundle this version reads (${b.anend})`);
}

const unhex = (s) => (s.length % 2 === 0 && /^[0-9a-fA-F]*$/.test(s) ? s.toLowerCase() : null);

/// fills an empty save with a bundle, keeping its keys (bundle.rs `import`)
export function importBundle(s, b) {
  checkBundle(b);
  const memories = s.importMemories(b.memories.map(([t, k]) => [t, fromBytes(k)]));
  const notes = s.import(b.notes);
  const pairs = [];
  for (const [l, m] of b.lists) {
    const a = parseRef(l);
    const c = parseRef(m);
    if (a !== null && c !== null) pairs.push([a, c]);
  }
  s.enlistMany(pairs);
  s.classifiers = b.classifiers || [];
  for (const [n, v] of Object.entries(b.settings)) {
    const h = unhex(v);
    if (h !== null) s.settings.set(n, h);
  }
  s.changed();
  return { memories, notes, lists: pairs.length };
}
