// markdown source, highlighted for the editor's mirror: each line as html (escaped),
// with spans the stylesheet colours. It never changes the text, only wraps it, so the
// mirror lines up with the textarea letter for letter.

import { esc } from './util.js';

const span = (cls, s) => (s === '' ? '' : `<span class="${cls}">${s}</span>`);

// inline: code spans, links and images, autolinks, bold, italics, strikes, #tags
const INLINE = new RegExp(
  [
    '(?<code>(?<ticks>`+)[^`][\\s\\S]*?(?<!`)\\k<ticks>(?!`))',
    '(?<link>!?\\[[^\\]\\n]*\\]\\([^)\\s]*(?:\\s+"[^"]*")?\\))',
    '(?<auto><https?:\\/\\/[^>\\s]+>|https?:\\/\\/[^\\s<>]*[^\\s<>.,;:!?)\\]])',
    '(?<se>\\*\\*\\*(?=[^\\s*])[\\s\\S]*?[^\\s*]\\*\\*\\*)',
    '(?<strong>(?<sd>\\*\\*|__)(?=\\S)[\\s\\S]*?\\S\\k<sd>)',
    '(?<em>(?<![\\w*])\\*(?=[^\\s*])[^*]*?[^\\s*]\\*(?![\\w*])|(?<![\\w_])_(?=[^\\s_])[^_]*?[^\\s_]_(?![\\w_])|(?<![\\w*])\\*[^\\s*]\\*(?![\\w*])|(?<![\\w_])_[^\\s_]_(?![\\w_]))',
    '(?<strike>~~(?=\\S)[\\s\\S]*?\\S~~)',
    '(?<tag>(?<![\\p{L}\\p{N}_&#/])#[\\p{L}\\p{N}_][\\p{L}\\p{N}_\\-/]*)',
  ].join('|'),
  'gu',
);

export function inline(s, depth = 0) {
  let out = '';
  let last = 0;
  INLINE.lastIndex = 0;
  for (let m; (m = INLINE.exec(s)); ) {
    if (m.index > last) out += esc(s.slice(last, m.index));
    const t = m[0];
    const g = m.groups;
    if (g.code) {
      const ticks = g.ticks.length;
      out += span('hl-code', span('hl-mark', esc(t.slice(0, ticks))) + esc(t.slice(ticks, -ticks)) + span('hl-mark', esc(t.slice(-ticks))));
    } else if (g.link) {
      const lm = t.match(/^(!?\[)([^\]]*)(\]\()([\s\S]*)(\))$/);
      out += span(
        t[0] === '!' ? 'hl-link hl-img' : 'hl-link',
        span('hl-mark', esc(lm[1])) + span('hl-ltext', depth < 3 ? inline(lm[2], depth + 1) : esc(lm[2])) + span('hl-mark', esc(lm[3])) + span('hl-url', esc(lm[4])) + span('hl-mark', ')'),
      );
    } else if (g.auto) out += span('hl-url hl-auto', esc(t));
    else if (g.se) out += span('hl-strong', wrap('hl-em', t, 3, depth));
    else if (g.strong) out += wrap('hl-strong', t, 2, depth);
    else if (g.em) out += wrap('hl-em', t, 1, depth);
    else if (g.strike) out += wrap('hl-strike', t, 2, depth);
    else if (g.tag) out += span('hl-tag', esc(t));
    // (the parts inside were highlighted with the same regex: carry on from here)
    last = m.index + t.length;
    INLINE.lastIndex = t.length ? last : last + 1;
  }
  return out + esc(s.slice(last));
}

function wrap(cls, t, n, depth) {
  const inner = t.slice(n, -n);
  return span(cls, span('hl-mark', esc(t.slice(0, n))) + (depth < 3 ? inline(inner, depth + 1) : esc(inner)) + span('hl-mark', esc(t.slice(-n))));
}

const FENCE = /^( {0,3})(`{3,}|~{3,})(.*)$/;
const HEADING = /^( {0,3})(#{1,6})(?=\s|$)(\s*)(.*?)(\s+#+\s*)?$/;
const RULE = /^ {0,3}([-*_])( *\1){2,} *$/;
const QUOTE = /^( {0,3}>\s?)+/;
const LIST = /^(\s*)([-*+]|\d{1,9}[.)])(\s+|$)(\[[ xX]\](?=\s|$))?/;
const TABLE_SEP = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;

// the state between lines: '' or the fence that's open
export function line(src, state) {
  if (state) {
    const f = src.match(FENCE);
    if (f && f[2][0] === state[0] && f[2].length >= state.length && !f[3].trim()) {
      return { html: span('hl-fence', esc(src)), state: '', cls: 'hl-block hl-block-end' };
    }
    return { html: esc(src), state, cls: 'hl-block' };
  }
  let m;
  if ((m = src.match(FENCE)) && !(m[2][0] === '`' && m[3].includes('`'))) {
    return { html: span('hl-fence', esc(m[1] + m[2])) + span('hl-lang', esc(m[3])), state: m[2], cls: 'hl-block hl-block-start' };
  }
  if ((m = src.match(HEADING))) {
    const level = m[2].length;
    return {
      html: esc(m[1]) + span('hl-mark', esc(m[2])) + esc(m[3]) + span(`hl-h hl-h${level}`, inline(m[4])) + (m[5] ? span('hl-mark', esc(m[5])) : ''),
      state: '',
      cls: `hl-hline hl-hline${level}`,
    };
  }
  if (RULE.test(src)) return { html: span('hl-rule', esc(src)), state: '', cls: '' };
  let html = '';
  let rest = src;
  if ((m = rest.match(QUOTE))) {
    html += span('hl-quote-mark', esc(m[0]));
    rest = rest.slice(m[0].length);
    return { html: html + span('hl-quote', listAndInline(rest)), state: '', cls: '' };
  }
  if (/^\s*\|/.test(rest) || (rest.includes('|') && TABLE_SEP.test(rest))) {
    if (TABLE_SEP.test(rest)) return { html: span('hl-table-sep', esc(rest)), state: '', cls: '' };
    return { html: rest.split(/(\|)/).map((p) => (p === '|' ? span('hl-pipe', '|') : inline(p))).join(''), state: '', cls: '' };
  }
  return { html: listAndInline(rest), state: '', cls: '' };
}

function listAndInline(s) {
  const m = s.match(LIST);
  if (!m || (m[3] === '' && !/^\d/.test(m[2]) && s.length > m[0].length)) return inline(s);
  const box = m[4] ? span(/x/i.test(m[4]) ? 'hl-box hl-done' : 'hl-box', esc(m[4])) : '';
  const after = s.slice(m[0].length);
  return esc(m[1]) + span('hl-bullet', esc(m[2])) + esc(m[3]) + box + (m[4] && /x/i.test(m[4]) ? span('hl-done-text', inline(after)) : inline(after));
}

// a cache of lines already highlighted, by their state and text
const cache = new Map();

// every line of a text: [{html, cls}]
export function lines(text) {
  const out = [];
  let state = '';
  for (const src of text.split('\n')) {
    const k = state + '\u0000' + src;
    let r = cache.get(k);
    if (!r) {
      r = line(src, state);
      if (cache.size > 60000) cache.clear();
      cache.set(k, r);
    }
    out.push(r);
    state = r.state;
  }
  return out;
}
