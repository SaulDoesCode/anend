// core/src/forge.rs, ported: saves as ingredients (add, take away, keep shared), the
// filters, and writing what comes out into a save with fresh keys. Things are compared
// by content: a memory by its text, a note by title and body, a list link by its ends.

import { spreadSums, splitWhitespace } from './text.js';
import { isNoteRef, noteRef, parseU64 } from './save.js';
import { unbin } from './text.js';

// an item: 'm' and a memory's text, or 'n' and a note's id
export const noteId = (n) => `${n.title}\0${n.markdown}`;
const pairKey = (p) => JSON.stringify(p);

function tidy(c) {
  const mems = new Set(c.memories);
  const notes = new Set(c.notes.map(noteId));
  const has = (i) => (i[0] === 'm' ? mems.has(i.slice(1)) : notes.has(i.slice(1)));
  const seen = new Set();
  c.lists = c.lists.filter(([l, m]) => has(l) && has(m)).filter((p) => {
    const k = pairKey(p);
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}

/// everything in a save
export function read(save) {
  const byKey = new Map();
  const memories = [];
  const seen = new Set();
  for (const [text, key] of save.allMemories()) {
    byKey.set(key, text);
    if (!seen.has(text)) {
      seen.add(text);
      memories.push(text);
    }
  }
  const notes = save.all();
  const ids = new Map(notes.map((n) => [n._key, noteId(n)]));
  const unique = [];
  const seenNotes = new Set();
  for (const n of notes) {
    if (!seenNotes.has(noteId(n))) {
      seenNotes.add(noteId(n));
      unique.push(n);
    }
  }
  const item = (r) => {
    if (isNoteRef(r)) {
      const id = ids.get(unbin(r.slice(5)));
      return id === undefined ? null : 'n' + id;
    }
    const t = byKey.get(r);
    return t === undefined ? null : 'm' + t;
  };
  const lists = [];
  for (const [l, m] of save.allLists()) {
    const a = item(l);
    const b = item(m);
    if (a !== null && b !== null) lists.push([a, b]);
  }
  const c = { memories, notes: unique, lists };
  tidy(c);
  return c;
}

/// the ingredients in order, onto an empty result
export function combine(parts) {
  const out = { memories: [], notes: [], lists: [] };
  for (const [effect, c] of parts) {
    const mems = new Set(c.memories);
    const notes = new Set(c.notes.map(noteId));
    const links = new Set(c.lists.map(pairKey));
    if (effect === 'add') {
      const have = new Set(out.memories);
      out.memories.push(...c.memories.filter((t) => !have.has(t)));
      const haveNotes = new Set(out.notes.map(noteId));
      out.notes.push(...c.notes.filter((n) => !haveNotes.has(noteId(n))));
      out.lists.push(...c.lists);
    } else if (effect === 'take away') {
      out.memories = out.memories.filter((t) => !mems.has(t));
      out.notes = out.notes.filter((n) => !notes.has(noteId(n)));
      out.lists = out.lists.filter((p) => !links.has(pairKey(p)));
    } else {
      out.memories = out.memories.filter((t) => mems.has(t));
      out.notes = out.notes.filter((n) => notes.has(noteId(n)));
      out.lists = out.lists.filter((p) => links.has(pairKey(p)));
    }
    tidy(out);
  }
  return out;
}

function wordsMatch(words, text) {
  text = text.toLowerCase();
  return splitWhitespace(words).every((w) => {
    w = w.toLowerCase();
    const not = w.startsWith('-') ? w.slice(1) : '';
    return not ? !text.includes(not) : text.includes(w);
  });
}

const within = (v, [lo, hi]) => (lo == null || v >= lo) && (hi == null || v <= hi);

/// narrows contents: {words, ordinal: [lo, hi], reversed: [lo, hi], memories, notes, lists}
export function filter(f, c) {
  if (!f.memories) c.memories = [];
  if (!f.notes) c.notes = [];
  if (!f.lists) c.lists = [];
  c.memories = c.memories.filter((t) => {
    const [s, r] = spreadSums(t);
    return wordsMatch(f.words, t) && within(s, f.ordinal) && within(r, f.reversed);
  });
  // a sum range is about memories; notes only answer to words
  const sumsSet = f.ordinal.some((x) => x != null) || f.reversed.some((x) => x != null);
  c.notes = c.notes.filter((n) => wordsMatch(f.words, `${n.title} ${n.markdown}`) && !sumsSet);
  tidy(c);
  return c;
}

/// puts contents into a save, skipping what it has; everything gets fresh keys there
export function writeInto(target, c) {
  const keys = new Map();
  for (const [text, key] of target.allMemories()) if (!keys.has(text)) keys.set(text, key);
  const fresh = c.memories.filter((t) => !keys.has(t));
  target.saturateMany(fresh).forEach((k, i) => keys.set(fresh[i], k));

  // notes: new keys after the highest one there
  const existing = target.all();
  const noteKeys = new Map(existing.map((n) => [noteId(n), n._key]));
  let next = 1000000n;
  let max = null;
  for (const n of existing) {
    const k = parseU64(n._key);
    if (k !== null && (max === null || k > max)) max = k;
  }
  if (max !== null && max + 1n > next) next = max + 1n;
  const newNotes = [];
  for (const n of c.notes) {
    const id = noteId(n);
    if (noteKeys.has(id)) continue;
    const m = { ...n, tags: [...n.tags], edits: [...n.edits], _key: String(next) };
    next += 1n;
    noteKeys.set(id, m._key);
    newNotes.push(m);
  }
  target.import(newNotes);

  // lists, between the keys things have now
  const toRef = (i) => {
    if (i[0] === 'm') return keys.has(i.slice(1)) ? keys.get(i.slice(1)) : null;
    return noteKeys.has(i.slice(1)) ? noteRef(noteKeys.get(i.slice(1))) : null;
  };
  const have = new Set(target.allLists().map(pairKey));
  const pairs = [];
  for (const [l, m] of c.lists) {
    const a = toRef(l);
    const b = toRef(m);
    if (a === null || b === null) continue;
    if (have.has(pairKey([a, b]))) continue;
    pairs.push([a, b]);
  }
  target.enlistMany(pairs);
  return { memories: fresh.length, notes: newNotes.length, lists: pairs.length };
}

/// a bundle's contents by what they are (bundle.rs `contents`), for merging
export function bundleContents(b, parseRef) {
  const texts = new Map(b.memories.map(([t, k]) => [String.fromCharCode(...k), t]));
  const notes = new Map(b.notes.map((n) => [n._key, n]));
  const item = (l) => {
    const r = parseRef(l);
    if (r === null) return null;
    if (isNoteRef(r)) {
      const n = notes.get(unbin(r.slice(5)));
      return n ? 'n' + noteId(n) : null;
    }
    return texts.has(r) ? 'm' + texts.get(r) : null;
  };
  const seen = new Set();
  const lists = [];
  for (const [l, m] of b.lists) {
    const a = item(l);
    const c = item(m);
    if (a !== null && c !== null) lists.push([a, c]);
  }
  return {
    memories: b.memories.map(([t]) => t).filter((t) => (seen.has(t) ? false : (seen.add(t), true))),
    notes: b.notes,
    lists,
  };
}
