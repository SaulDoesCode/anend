// the text kernels of core/src/simd.rs, memory.rs and spread.rs, ported: the tokenizer,
// the gematria sums, the letter frequency key, and Rust's notions of whitespace, case
// and byte order. Bytes travel as binary strings (one char per byte, 0..255), so they
// compare, hash and sort the way redb's byte keys do.

const enc = new TextEncoder();
const dec = new TextDecoder();

/// a string's UTF-8 bytes as a binary string
export function bin(s) {
  const b = enc.encode(s);
  let out = '';
  for (let i = 0; i < b.length; i += 8192) out += String.fromCharCode.apply(null, b.subarray(i, i + 8192));
  return out;
}

/// a binary string of UTF-8 back to text (lossy, like String::from_utf8_lossy)
export function unbin(b) {
  const a = new Uint8Array(b.length);
  for (let i = 0; i < b.length; i++) a[i] = b.charCodeAt(i);
  return dec.decode(a);
}

export const bytesOf = (b) => Array.from(b, (c) => c.charCodeAt(0));
export const fromBytes = (a) => String.fromCharCode(...a);

/// redb's order for byte keys (binary strings compare byte by byte)
export const cmpBin = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

/// Rust's order for strings: by code point, which is UTF-8's byte order
export function cmpStr(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    const x = a.codePointAt(i);
    const y = b.codePointAt(i);
    if (x !== y) return x < y ? -1 : 1;
    if (x > 0xffff) i++;
  }
  return a.length < b.length ? -1 : a.length > b.length ? 1 : 0;
}

// ---------------------------------------------------------------- Rust's str helpers

// char::is_whitespace: Unicode's White_Space
const WS_CHARS = [9, 10, 11, 12, 13, 32, 0x85, 0xa0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000];
const WS_SET = new Set(WS_CHARS.map((c) => String.fromCharCode(c)));
const WS_CLASS = '[' + WS_CHARS.map((c) => '\\u' + c.toString(16).padStart(4, '0')).join('') + ']';
const WS_RUN = new RegExp(WS_CLASS + '+');
const WS_LEAD = new RegExp('^' + WS_CLASS + '+');
const WS_EDGES = new RegExp('^' + WS_CLASS + '+|' + WS_CLASS + '+$', 'g');

export const isWhitespace = (c) => WS_SET.has(c);
export const trim = (s) => String(s).replace(WS_EDGES, '');
export const trimStart = (s) => String(s).replace(WS_LEAD, '');
export const splitWhitespace = (s) => String(s).split(WS_RUN).filter((x) => x !== '');
export const chars = (s) => Array.from(s);
export const charCount = (s) => {
  let n = 0;
  for (const _ of s) n++;
  return n;
};

/// `format!("{:?}", s)` for a str, near enough: quoted, with escapes
export function debugStr(s) {
  let out = '"';
  for (const c of s) {
    const n = c.codePointAt(0);
    if (c === '"') out += '\\"';
    else if (c === '\\') out += '\\\\';
    else if (c === '\n') out += '\\n';
    else if (c === '\r') out += '\\r';
    else if (c === '\t') out += '\\t';
    else if (c === '\0') out += '\\0';
    else if (n < 0x20 || n === 0x7f) out += `\\u{${n.toString(16)}}`;
    else out += c;
  }
  return out + '"';
}

// ---------------------------------------------------------------- gematria

const GREEK = Array.from('αβγδεζηθικλμνξοπρστυφχψω');
const HEBREW = Array.from('אבגדהוזחטיכלמנסעפצקרשת');

/// Greek and Hebrew letters' Atbash, `n + 1 - v`, over the lower-cased text
export function otherAlphabetsAtbash(text) {
  let sum = 0;
  for (const c of text.toLowerCase()) {
    let i = GREEK.indexOf(c);
    if (i >= 0) sum += GREEK.length - i;
    i = HEBREW.indexOf(c);
    if (i >= 0) sum += HEBREW.length - i;
  }
  return sum;
}

// only ASCII letters count, and every byte of a multi-byte character is >= 0x80, so the
// string's UTF-16 units below 128 are exactly the bytes simd.rs looks at

/// simd::sums: ordinal, reversed and Atbash
export function sums(text) {
  let o = 0;
  let r = 0;
  let ascii = true;
  for (let i = 0; i < text.length; i++) {
    const b = text.charCodeAt(i);
    if (b >= 128) {
      ascii = false;
      continue;
    }
    const lc = b | 0x20;
    if (lc >= 97 && lc <= 122) {
      const v = lc - 96;
      o += v;
      r += 27 - v;
    }
  }
  return { ordinal: o, reversed: r, atbash: ascii ? r : r + otherAlphabetsAtbash(text) };
}

/// simd::frequencies: how often each of a..z occurs
export function frequencies(text) {
  const c = new Array(26).fill(0);
  for (let i = 0; i < text.length; i++) {
    const b = text.charCodeAt(i);
    if (b >= 128) continue;
    const lc = b | 0x20;
    if (lc >= 97 && lc <= 122) c[lc - 97]++;
  }
  return c;
}

/// simd::frequency_key: "listen" -> "e1i1l1n1s1t1"
export function frequencyKey(text) {
  let k = '';
  frequencies(text).forEach((n, i) => {
    if (n > 0) k += String.fromCharCode(97 + i) + n;
  });
  return k;
}

/// memory::measure: what a text is filed under in each gsearch bucket
export function measure(text) {
  const s = sums(text);
  const cap = (n) => Math.min(n, 4294967295);
  return { ordinal: cap(s.ordinal), reversed: cap(s.reversed), atbash: cap(s.atbash), frequency: frequencyKey(text) };
}

/// spread::sums: [s, r] over UTF-16 units, like charCodeAt
export function spreadSums(t) {
  let s = 0;
  let n = 0;
  for (let i = 0; i < t.length; i++) {
    const c = t.charCodeAt(i) | 32;
    if (c >= 97 && c <= 122) {
      s += c - 96;
      n++;
    }
  }
  return [s, 27 * n - s];
}

// ---------------------------------------------------------------- tokenizer

/// simd::tokenize: the bytes between spaces, newlines, tabs and carriage returns
export function tokenize(text) {
  return text.split(/[ \n\t\r]+/).filter((t) => t !== '');
}

/// distinct tokens, first appearance first, as binary strings (the index's keys)
export function uniqueTokens(text) {
  const seen = new Set();
  const out = [];
  for (const t of tokenize(text)) {
    const b = bin(t);
    if (!seen.has(b)) {
      seen.add(b);
      out.push(b);
    }
  }
  return out;
}
