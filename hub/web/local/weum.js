// core/src/weum.rs, ported: the prime factor reduction line (pfR), the gematria detail
// line (gsum(text, 1)) and the search variations (tryfulSearchVariations)

import { sums, splitWhitespace, trim, chars } from './text.js';

// ---------------------------------------------------------------- pfR

export function primeFactors(n) {
  const found = [];
  let d = 2;
  while (n > 1) {
    while (n % d === 0) {
      found.push(d);
      n /= d;
    }
    d += 1;
    if (d * d > n && n > 1) {
      found.push(n);
      break;
    }
  }
  return found;
}

export function digitSum(n) {
  let s = 0;
  for (const c of String(n)) s += c.charCodeAt(0) - 48;
  return s;
}

const pfSum = (n) => primeFactors(n).reduce((a, b) => a + b, 0);

/// weum's pfR(s), as the line it prints
export function pfReduction(s) {
  const LIMIT = 32;
  const pf = primeFactors(s);
  const pfr = pf.reduce((a, b) => a + b, 0);
  const ds = digitSum(s);
  const dp = ds * pfr;
  const chain = [];
  for (let i = 0; i <= LIMIT; i++) {
    const last = chain[chain.length - 1];
    const xn = chain.length ? pfSum(last) * digitSum(last) : pfSum(dp) * ds;
    if (chain.includes(xn)) break;
    chain.push(xn);
  }
  const total = chain.reduce((a, b) => a + b, 0) + pfr;
  const fds = digitSum(total);
  let out = '';
  if (pf.length === 1) out += `${pf[0]} `;
  else if (pf.length > 1) out += `${pf.join(' + ')} = ${pfr} -> `;
  out += `* ${ds} = ${dp}`;
  for (const t of chain) out += `+${t}`;
  return out + ` = ${total} ${fds} | ${ds + fds}`;
}

// ---------------------------------------------------------------- gsum detail

const VOWELS = { a: 1, e: 2, o: 3, u: 4, i: 5, y: 6 };
// weum's table, t and v both 16 included
const CONSONANTS = { b: 1, c: 2, d: 3, f: 4, g: 5, h: 6, j: 7, k: 8, l: 9, m: 10, n: 11, p: 12, q: 13, r: 14, s: 15, t: 16, v: 16, w: 17, x: 18, z: 19 };

/// weum's gsum(text, 1) line with the Atbash measure appended
export function gsumDetail(text) {
  const s = sums(text);
  const sum = s.ordinal;
  const r = s.reversed;
  let v = 0;
  let c = 0;
  for (let i = 0; i < text.length; i++) {
    const b = text.charCodeAt(i);
    if (b >= 128) continue;
    const l = String.fromCharCode(b).toLowerCase();
    v += VOWELS[l] || 0;
    c += CONSONANTS[l] || 0;
  }
  const words = Math.max(splitWhitespace(text).length, 1);
  const a = sum + r + Math.abs(sum - r);
  const bb = sum + r + (sum - r);
  return `(${sum} + ${r} = ${sum + r}; diff ${sum - r}, ${a} ${bb} ${a - bb}) v ${v} c ${c} w ${words} a ${s.atbash}`;
}

// ---------------------------------------------------------------- search variations

const DEFAULT_VARIATIONS = ['man', 'ton', 'able', 'y', 'ly', 'ings', 'ing', 'ed', 's,', '.', '?', ':', ',', 's'];

const dropLast = (s, n) => {
  const c = chars(s);
  return c.slice(0, Math.max(0, c.length - n)).join('');
};
const capitalize = (s) => {
  const c = chars(s);
  return c.length ? c[0].toUpperCase() + c.slice(1).join('') : '';
};
const blank = (list = []) => ({ list, lc: false, sless: false, erer: false, encs: false, mic: false, ll: false });

// one call of tryfulSearchVariations; searches and timers go to `tasks`
function tsv(query, vars, f, max, caseful, tasks) {
  if (max <= 0 || query === '') return;
  const exact = query.length >= 2 && query.startsWith('"') && query.endsWith('"');
  if (exact) {
    query = trim(query.slice(1, -1));
    if (query === '') return;
  }
  const cs = chars(query);
  const last = cs[cs.length - 1];
  if (!vars.list.length) {
    if (!vars.lc) {
      vars.lc = true;
      if (query !== query.toLowerCase()) query = query.toLowerCase();
    }
    const push = (...items) => vars.list.push(...items);
    const ends = (x) => query.endsWith(x);
    switch (last) {
      case 'y':
        if (ends('iety')) {
          query = dropLast(query, 4);
          push('iation', 'iations', 'ieties');
        } else if (ends('ey')) {
          query = dropLast(query, 2);
          push('ies', 'ied');
        } else {
          query = dropLast(query, 1);
          push('ies', 's');
        }
        break;
      case 's':
        if (ends('ngs')) {
          query = dropLast(query, 3);
          push('ngies');
        } else if (!vars.sless) {
          query = dropLast(query, 1);
          vars.sless = true;
        } else return;
        break;
      case 'r':
      case 'd':
        if (!vars.erer) {
          vars.erer = true;
          push(!ends('er') ? 'er' : 's', 'ences');
          vars.encs = true;
        } else if (!vars.encs) {
          vars.encs = true;
          if (!ends('ences')) push('ences');
        } else return;
        break;
      case 'm':
      case 'n':
        if (!vars.mic) {
          push('ic');
          vars.mic = true;
        } else return;
        break;
      case 'e':
        if (ends('ime')) {
          query = dropLast(query, 1);
          push('ing');
        } else if (ends('ane')) {
          query = dropLast(query, 1);
          push('ity');
        } else return;
        break;
      case 't':
        if (ends('ent')) {
          query = dropLast(query, 1);
          push('cy', 'ial', 'ce');
        } else return;
        break;
      case 'l':
        if (vars.ll) return;
        vars.ll = true;
        {
          const c = chars(query);
          if (c.length >= 2 && c[c.length - 2] === 'l') push('er', 'ity');
        }
        break;
      default:
        return;
    }
  }
  if (f) tasks.push({ search: trim(query) });
  // the clone only keeps the lowercase flag, like `[...variations]` with `_lc` set
  const clone = { ...blank([...vars.list]), lc: true };
  if (!caseful) {
    const fancy = capitalize(query);
    if (query !== fancy) tsv(fancy, clone, true, max - 1, false, tasks);
  }
  if (!exact) tasks.push({ query, vars, max });
}

/// every query weum's ordinary search sends for `query`, in the order it sends them
export function searchVariations(query) {
  const tasks = [];
  const out = [];
  tsv(query, blank([...DEFAULT_VARIATIONS]), true, 50, false, tasks);
  let guard = 0;
  while (tasks.length) {
    const t = tasks.shift();
    if (++guard > 100000) break;
    if ('search' in t) out.push(t.search);
    else {
      const suffix = t.vars.list.pop() ?? '';
      out.push(trim(t.query) + suffix);
      tsv(t.query, t.vars, false, t.max - 1, true, tasks);
    }
  }
  return out;
}
