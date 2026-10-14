// a note's markdown as the plain text the search index reads (memory.rs `note_texts`,
// over core/src/markdown.rs, over pulldown-cmark): every block's text after a newline,
// table cells run together, code as it is, emphasis, links and escapes taken out. Not a
// renderer, a CommonMark reading close enough that the same words land in the index.

const PUNCT = /[\p{P}\p{S}]/u;
const WSC = /\s/u;
const isWs = (c) => c === undefined || c === '' || WSC.test(c);
const isPunct = (c) => c !== undefined && c !== '' && PUNCT.test(c);

const ENTITIES = {
  amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: String.fromCharCode(0xa0), copy: String.fromCharCode(0xa9), reg: String.fromCharCode(0xae),
  trade: String.fromCharCode(0x2122), hellip: String.fromCharCode(0x2026), mdash: String.fromCharCode(0x2014), ndash: String.fromCharCode(0x2013),
  lsquo: String.fromCharCode(0x2018), rsquo: String.fromCharCode(0x2019), ldquo: String.fromCharCode(0x201c), rdquo: String.fromCharCode(0x201d),
  laquo: String.fromCharCode(0xab), raquo: String.fromCharCode(0xbb), times: String.fromCharCode(0xd7), divide: String.fromCharCode(0xf7),
  deg: String.fromCharCode(0xb0), euro: String.fromCharCode(0x20ac), pound: String.fromCharCode(0xa3), yen: String.fromCharCode(0xa5),
  cent: String.fromCharCode(0xa2), sect: String.fromCharCode(0xa7), para: String.fromCharCode(0xb6), middot: String.fromCharCode(0xb7),
  bull: String.fromCharCode(0x2022), larr: String.fromCharCode(0x2190), rarr: String.fromCharCode(0x2192), uarr: String.fromCharCode(0x2191),
  darr: String.fromCharCode(0x2193), harr: String.fromCharCode(0x2194), hearts: String.fromCharCode(0x2665), plusmn: String.fromCharCode(0xb1),
  frac12: String.fromCharCode(0xbd), shy: String.fromCharCode(0xad), ensp: String.fromCharCode(0x2002), emsp: String.fromCharCode(0x2003),
  thinsp: String.fromCharCode(0x2009), zwj: String.fromCharCode(0x200d), zwnj: String.fromCharCode(0x200c), iexcl: String.fromCharCode(0xa1),
  iquest: String.fromCharCode(0xbf), micro: String.fromCharCode(0xb5), alpha: String.fromCharCode(0x3b1), beta: String.fromCharCode(0x3b2),
  pi: String.fromCharCode(0x3c0), Omega: String.fromCharCode(0x3a9), omega: String.fromCharCode(0x3c9), infin: String.fromCharCode(0x221e),
  ne: String.fromCharCode(0x2260), le: String.fromCharCode(0x2264), ge: String.fromCharCode(0x2265), check: String.fromCharCode(0x2713),
};

const label = (s) => s.trim().replace(/\s+/g, ' ').toLowerCase();

// ---------------------------------------------------------------- inline

const SCHEME_LINK = /^<([A-Za-z][A-Za-z0-9+.-]{1,31}:[^\s<>]*)>/;
const EMAIL_LINK = /^<([a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*)>/;
const ATTR = `(?:\\s+[A-Za-z_:][A-Za-z0-9_.:-]*(?:\\s*=\\s*(?:[^\\s"'=<>\`]+|'[^']*'|"[^"]*"))?)`;
const HTML_INLINE = new RegExp(`^(?:<[A-Za-z][A-Za-z0-9-]*${ATTR}*\\s*/?>|</[A-Za-z][A-Za-z0-9-]*\\s*>|<!--[\\s\\S]*?-->|<\\?[\\s\\S]*?\\?>|<![A-Za-z][^>]*>|<!\\[CDATA\\[[\\s\\S]*?\\]\\]>)`);
const ENTITY = /^&(?:#[xX]([0-9a-fA-F]{1,6})|#([0-9]{1,7})|([A-Za-z][A-Za-z0-9]{0,31}));/;
const ASCII_PUNCT = /[!-/:-@[-`{-~]/;

// a link's (destination "title") after the `]`; the index after it, or -1
function inlineLink(s, i) {
  if (s[i] !== '(') return -1;
  let j = i + 1;
  const ws = () => {
    let nl = 0;
    while (j < s.length && (s[j] === ' ' || s[j] === '\t' || s[j] === '\n')) {
      if (s[j] === '\n' && ++nl > 1) return false;
      j++;
    }
    return true;
  };
  if (!ws()) return -1;
  if (s[j] === '<') {
    j++;
    while (j < s.length && s[j] !== '>' && s[j] !== '\n' && s[j] !== '<') j += s[j] === '\\' ? 2 : 1;
    if (s[j] !== '>') return -1;
    j++;
  } else {
    let depth = 0;
    const start = j;
    while (j < s.length) {
      const c = s[j];
      if (c === '\\' && j + 1 < s.length && ASCII_PUNCT.test(s[j + 1])) {
        j += 2;
        continue;
      }
      if (c === ' ' || c === '\t' || c === '\n' || c.charCodeAt(0) < 32) break;
      if (c === '(') depth++;
      if (c === ')') {
        if (depth === 0) break;
        depth--;
      }
      j++;
    }
    if (depth !== 0 && j > start) return -1;
  }
  const before = j;
  if (!ws()) return -1;
  if (j > before && (s[j] === '"' || s[j] === "'" || s[j] === '(')) {
    const close = s[j] === '(' ? ')' : s[j];
    j++;
    while (j < s.length && s[j] !== close) j += s[j] === '\\' ? 2 : 1;
    if (s[j] !== close) return -1;
    j++;
    if (!ws()) return -1;
  }
  return s[j] === ')' ? j + 1 : -1;
}

// a `[label]` at i: [the label, the index after it], or null
function linkLabel(s, i) {
  if (s[i] !== '[') return null;
  let j = i + 1;
  while (j < s.length && s[j] !== ']') {
    if (s[j] === '[') return null;
    j += s[j] === '\\' ? 2 : 1;
  }
  if (s[j] !== ']' || j - i > 1000) return null;
  return [s.slice(i + 1, j), j + 1];
}

function flankOpen(c, run, prev, next) {
  if (isWs(next)) return false;
  if (prev === undefined) return true;
  if (c === '*' && !isPunct(next)) return true;
  if (c === '~' && run > 1) return true;
  if (c === '~' && prev === '~' && !isPunct(next)) return true;
  return isWs(prev) || isPunct(prev);
}

function flankClose(c, run, prev, next) {
  if (prev === undefined || isWs(prev)) return false;
  if (next === undefined) return true;
  if ((c === '*' || (c === '~' && run > 1)) && !isPunct(prev)) return true;
  if (c === '~' && prev === '~') return true;
  return isWs(next) || isPunct(next);
}

// pairs up the emphasis delimiters from `bottom` on (pulldown-cmark's rules)
function emphasis(delims, bottom) {
  let i = bottom;
  while (i < delims.length) {
    const closer = delims[i];
    if (!closer.close || closer.count === 0) {
      i++;
      continue;
    }
    let found = -1;
    for (let j = i - 1; j >= bottom; j--) {
      const o = delims[j];
      if (o.c !== closer.c || !o.open || o.count === 0) continue;
      if (o.c === '~' && o.run !== closer.run) continue;
      const both = closer.open && closer.close;
      if ((both || (o.open && o.close)) && (o.run + closer.run) % 3 === 0 && closer.run % 3 !== 0) continue;
      found = j;
      break;
    }
    if (found < 0) {
      if (!closer.open) delims.splice(i, 1);
      else i++;
      continue;
    }
    const o = delims[found];
    const n = Math.min(o.count, closer.count);
    o.count -= n;
    closer.count -= n;
    delims.splice(found + 1, i - found - 1);
    i = found + 1;
    if (o.count === 0) {
      delims.splice(found, 1);
      i = found;
    }
    if (closer.count === 0) delims.splice(i, 1);
  }
}

function render(nodes) {
  let out = '';
  for (const n of nodes) {
    if (n.t === 'text') out += n.s;
    else if (n.t === 'delim') out += n.c.repeat(n.count);
    else if (n.t === 'link') out += n.image ? `[image: ${render(n.kids)}]` : render(n.kids);
  }
  return out;
}

/// the text of a run of inline markdown
export function inline(s, defs = new Map()) {
  const nodes = [];
  const delims = [];
  const brackets = [];
  const text = (t) => {
    const last = nodes[nodes.length - 1];
    if (last && last.t === 'text' && !last.mark) last.s += t;
    else nodes.push({ t: 'text', s: t });
  };
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === '\\') {
      if (s[i + 1] === '\n') {
        text('\n');
        i += 2;
      } else if (i + 1 < s.length && ASCII_PUNCT.test(s[i + 1])) {
        text(s[i + 1]);
        i += 2;
      } else {
        text('\\');
        i++;
      }
    } else if (c === '`') {
      let n = 1;
      while (s[i + n] === '`') n++;
      let j = i + n;
      let end = -1;
      while (j < s.length) {
        const k = s.indexOf('`', j);
        if (k < 0) break;
        let m = 1;
        while (s[k + m] === '`') m++;
        if (m === n) {
          end = k;
          break;
        }
        j = k + m;
      }
      if (end < 0) {
        text('`'.repeat(n));
        i += n;
      } else {
        let code = s.slice(i + n, end).replace(/\n/g, ' ');
        if (code.length > 1 && code[0] === ' ' && code[code.length - 1] === ' ' && code.trim() !== '') code = code.slice(1, -1);
        text(code);
        i = end + n;
      }
    } else if (c === '<') {
      const rest = s.slice(i);
      const m = SCHEME_LINK.exec(rest) || EMAIL_LINK.exec(rest);
      const h = !m && HTML_INLINE.exec(rest);
      if (m) {
        text(m[1]);
        i += m[0].length;
      } else if (h) {
        text(h[0].replace(/\n+$/, ''));
        i += h[0].length;
      } else {
        text('<');
        i++;
      }
    } else if (c === '&') {
      const m = ENTITY.exec(s.slice(i));
      let v = null;
      if (m && (m[1] || m[2])) {
        const cp = parseInt(m[1] || m[2], m[1] ? 16 : 10);
        v = String.fromCodePoint(cp === 0 || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff) ? 0xfffd : cp);
      } else if (m && Object.hasOwn(ENTITIES, m[3])) v = ENTITIES[m[3]];
      if (v !== null) {
        text(v);
        i += m[0].length;
      } else {
        text('&');
        i++;
      }
    } else if (c === '\n') {
      // a hard break (two spaces before it) or a soft one: whitespace either way
      const last = nodes[nodes.length - 1];
      if (last && last.t === 'text') last.s = last.s.replace(/ +$/, '');
      text('\n');
      i++;
      while (s[i] === ' ' || s[i] === '\t') i++;
    } else if (c === '*' || c === '_' || c === '~') {
      let n = 1;
      while (s[i + n] === c) n++;
      const prev = i > 0 ? String.fromCodePoint(s.codePointAt(i - 1 - (i > 1 && /[\udc00-\udfff]/.test(s[i - 1]) ? 1 : 0))) : undefined;
      const next = i + n < s.length ? String.fromCodePoint(s.codePointAt(i + n)) : undefined;
      const open = flankOpen(c, n, prev, next);
      const close = flankClose(c, n, prev, next);
      if ((open || close) && (c !== '~' || n <= 2)) {
        const d = { t: 'delim', c, count: n, run: n, open, close };
        nodes.push(d);
        delims.push(d);
      } else text(c.repeat(n));
      i += n;
    } else if (c === '[' || (c === '!' && s[i + 1] === '[')) {
      const image = c === '!';
      const node = { t: 'text', s: image ? '![' : '[', mark: true };
      nodes.push(node);
      brackets.push({ node, image, active: true, bottom: delims.length, from: i + (image ? 2 : 1) });
      i += image ? 2 : 1;
    } else if (c === ']') {
      const b = brackets.pop();
      if (!b) {
        text(']');
        i++;
        continue;
      }
      if (!b.active) {
        text(']');
        i++;
        continue;
      }
      let after = inlineLink(s, i + 1);
      if (after < 0) {
        const l = linkLabel(s, i + 1);
        const name = l && l[0].trim() ? l[0] : s.slice(b.from, i);
        if (defs.has(label(name))) after = l ? l[1] : i + 1;
      }
      if (after < 0) {
        text(']');
        i++;
        continue;
      }
      const at = nodes.indexOf(b.node);
      emphasis(delims, b.bottom);
      delims.length = Math.min(delims.length, b.bottom);
      const kids = nodes.splice(at + 1);
      nodes[at] = { t: 'link', image: b.image, kids };
      if (!b.image) for (const o of brackets) if (!o.image) o.active = false;
      i = after;
    } else {
      text(c);
      i++;
    }
  }
  emphasis(delims, 0);
  return render(nodes);
}

// ---------------------------------------------------------------- blocks

const BLANK = /^[ \t]*$/;
const ATX = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/;
const RULE = /^ {0,3}(?:(?:\*[ \t]*){3,}|(?:-[ \t]*){3,}|(?:_[ \t]*){3,})$/;
const SETEXT = /^ {0,3}(?:=+|-+)[ \t]*$/;
const FENCE = /^( {0,3})(`{3,}|~{3,})(.*)$/;
const QUOTE = /^ {0,3}> ?/;
const ITEM = /^( {0,3})([-+*]|(\d{1,9})[.)])([ \t]+|$)/;
const DEF = /^ {0,3}\[((?:[^\]\\]|\\.){1,999})\]:[ \t]*(<[^>\n]*>|\S+)(?:[ \t]+(?:"[^"]*"|'[^']*'|\([^)]*\)))?[ \t]*$/;
const HTML_TAGS = 'address|article|aside|base|basefont|blockquote|body|caption|center|col|colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption|figure|footer|form|frame|frameset|h1|h2|h3|h4|h5|h6|head|header|hr|html|iframe|legend|li|link|main|menu|menuitem|nav|noframes|ol|optgroup|option|p|param|search|section|summary|table|tbody|td|tfoot|th|thead|title|tr|track|ul';
const HTML_STARTS = [
  [/^ {0,3}<(?:script|pre|style|textarea)(?:[ \t>]|$)/i, /<\/(?:script|pre|style|textarea)>/i],
  [/^ {0,3}<!--/, /-->/],
  [/^ {0,3}<\?/, /\?>/],
  [/^ {0,3}<![A-Za-z]/, />/],
  [/^ {0,3}<!\[CDATA\[/, /\]\]>/],
  [new RegExp(`^ {0,3}</?(?:${HTML_TAGS})(?:[ \\t>]|/>|$)`, 'i'), null],
];
const HTML_7 = new RegExp(`^ {0,3}(?:<[A-Za-z][A-Za-z0-9-]*${ATTR}*\\s*/?>|</[A-Za-z][A-Za-z0-9-]*\\s*>)[ \\t]*$`);
const DELIM_ROW = /^[ \t]*\|?[ \t]*:?-+:?[ \t]*(?:\|[ \t]*:?-+:?[ \t]*)*\|?[ \t]*$/;

// leading tabs as spaces to the next stop of four
function detab(line) {
  if (!line.includes('\t')) return line;
  let col = 0;
  let i = 0;
  let out = '';
  for (; i < line.length && (line[i] === ' ' || line[i] === '\t'); i++) {
    const w = line[i] === '\t' ? 4 - (col % 4) : 1;
    out += ' '.repeat(w);
    col += w;
  }
  return out + line.slice(i);
}

const indent = (l) => l.length - l.trimStart().length;

function cells(row) {
  let r = row.trim();
  if (r.startsWith('|')) r = r.slice(1);
  if (r.endsWith('|') && !r.endsWith('\\|')) r = r.slice(0, -1);
  const out = [];
  let cur = '';
  for (let i = 0; i < r.length; i++) {
    if (r[i] === '\\' && r[i + 1] === '|') {
      cur += '|';
      i++;
    } else if (r[i] === '|') {
      out.push(cur);
      cur = '';
    } else cur += r[i];
  }
  out.push(cur);
  return out.map((c) => c.trim());
}

function htmlStart(line, inPara) {
  for (const [start, end] of HTML_STARTS) if (start.test(line)) return { end, blank: end === null };
  if (!inPara && HTML_7.test(line)) return { end: null, blank: true };
  return null;
}

function listStart(line, inPara) {
  const m = ITEM.exec(line);
  if (!m || RULE.test(line)) return null;
  const rest = line.slice(m[0].length);
  if (inPara && (BLANK.test(rest) || (m[3] !== undefined && +m[3] !== 1))) return null;
  const marker = m[1].length + m[2].length;
  const spaces = m[4].length;
  const width = BLANK.test(rest) || spaces >= 5 ? marker + 1 : marker + spaces;
  return { width, first: BLANK.test(rest) ? '' : ' '.repeat(Math.max(0, marker + spaces - width)) + rest, bullet: m[3] === undefined ? m[2] : m[2].slice(-1) };
}

// whether a line starts a block that breaks a paragraph
function interrupts(line) {
  return BLANK.test(line) || RULE.test(line) || ATX.test(line) || FENCE.test(line) || QUOTE.test(line) || !!listStart(line, true) || !!htmlStart(line, true) || line.trimStart().startsWith('|');
}

// leaf blocks into `out`: {inline} to read as markdown, {raw} as it is, {cells} a table row
function blocks(lines, out, defs) {
  let para = null;
  const endPara = () => {
    if (!para) return;
    let p = para;
    para = null;
    while (p.length) {
      const m = DEF.exec(p[0]);
      if (!m) break;
      const k = label(m[1]);
      if (k && !defs.has(k)) defs.set(k, m[2]);
      p = p.slice(1);
    }
    if (p.length) out.push({ inline: p.map((l) => l.trimStart()).join('\n').trimEnd() });
  };
  let i = 0;
  while (i < lines.length) {
    const line = detab(lines[i]);
    if (BLANK.test(line)) {
      endPara();
      i++;
      continue;
    }
    if (para && SETEXT.test(line)) {
      // a paragraph underlined: a heading, the same text
      const p = para;
      para = null;
      out.push({ inline: p.map((l) => l.trimStart()).join('\n').trimEnd() });
      i++;
      continue;
    }
    if (!para && indent(line) >= 4) {
      const code = [];
      while (i < lines.length && (BLANK.test(lines[i]) || indent(detab(lines[i])) >= 4)) code.push(detab(lines[i++]).slice(4));
      while (code.length && BLANK.test(code[code.length - 1])) code.pop();
      out.push({ raw: code.join('\n') });
      continue;
    }
    let m;
    if ((m = FENCE.exec(line)) && !(m[2][0] === '`' && m[3].includes('`'))) {
      endPara();
      const fence = m[2];
      const pad = m[1].length;
      const code = [];
      i++;
      while (i < lines.length) {
        const l = detab(lines[i]);
        const close = new RegExp(`^ {0,3}${fence[0] === '`' ? '`' : '~'}{${fence.length},}[ \\t]*$`);
        if (close.test(l)) {
          i++;
          break;
        }
        let k = 0;
        while (k < pad && l[k] === ' ') k++;
        code.push(l.slice(k));
        i++;
      }
      out.push({ raw: code.join('\n').replace(/\n+$/, '') });
      continue;
    }
    if ((m = ATX.exec(line))) {
      endPara();
      let t = m[2] || '';
      if (/^#+$/.test(t)) t = '';
      out.push({ inline: t.trim() });
      i++;
      continue;
    }
    if (RULE.test(line)) {
      endPara();
      out.push({ raw: '' });
      i++;
      continue;
    }
    if (QUOTE.test(line)) {
      endPara();
      const inner = [];
      let lazy = false;
      while (i < lines.length) {
        const l = detab(lines[i]);
        if (QUOTE.test(l)) {
          const stripped = l.replace(QUOTE, '');
          inner.push(stripped);
          lazy = !BLANK.test(stripped) && indent(stripped) < 4 && !FENCE.test(stripped);
        } else if (lazy && !interrupts(l)) inner.push(l);
        else break;
        i++;
      }
      blocks(inner, out, defs);
      continue;
    }
    const item = listStart(line, !!para);
    if (item) {
      endPara();
      const inner = [item.first];
      let lazy = !BLANK.test(item.first);
      let blank = false;
      i++;
      while (i < lines.length) {
        const l = detab(lines[i]);
        if (BLANK.test(l)) {
          inner.push('');
          blank = true;
          lazy = false;
        } else if (indent(l) >= item.width) {
          inner.push(l.slice(item.width));
          blank = false;
          lazy = true;
        } else if (lazy && !blank && !interrupts(l)) inner.push(l);
        else break;
        i++;
      }
      // a task box is a marker, not text
      if (/^\[[ xX]\](?:[ \t]|$)/.test(inner[0])) inner[0] = inner[0].slice(3).trimStart();
      blocks(inner, out, defs);
      continue;
    }
    const html = htmlStart(line, !!para);
    if (html) {
      endPara();
      const raw = [];
      while (i < lines.length) {
        const l = lines[i];
        if (html.blank && BLANK.test(l)) break;
        raw.push(l);
        i++;
        if (html.end && html.end.test(l)) break;
      }
      out.push({ raw: raw.join('') });
      continue;
    }
    if ((!para || line.trimStart().startsWith('|')) && line.includes('|') && i + 1 < lines.length && DELIM_ROW.test(lines[i + 1]) && lines[i + 1].includes('-')) {
      const head = cells(line);
      const n = cells(lines[i + 1]).length;
      if (head.length === n) {
        endPara();
        out.push({ cells: head });
        i += 2;
        while (i < lines.length && !BLANK.test(lines[i]) && !interrupts(detab(lines[i]).replace(/^\s*\|/, 'x'))) {
          const row = cells(lines[i]).slice(0, n);
          while (row.length < n) row.push('');
          out.push({ cells: row });
          i++;
        }
        continue;
      }
    }
    (para ||= []).push(line);
    i++;
  }
  endPara();
}

/// what memory.rs indexes a note's tokens by: the title, then each block's text after
/// a newline
export function plain(title, markdown) {
  const out = [];
  const defs = new Map();
  blocks(String(markdown).replace(/\r\n?/g, '\n').split('\n'), out, defs);
  let text = title;
  for (const b of out) {
    text += '\n';
    if (b.raw !== undefined) text += b.raw;
    else if (b.cells) text += b.cells.map((c) => inline(c, defs)).join('');
    else text += inline(b.inline, defs);
  }
  return text;
}
