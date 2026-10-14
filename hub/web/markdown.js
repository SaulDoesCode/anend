// a small markdown renderer: headings, paragraphs, lists, quotes, code, tables, rules,
// and inline code, bold, italics, strikes, links and images. Everything is escaped first,
// so a note can't put html of its own on the page.

import { esc } from './util.js';

// links only go to the web, mail, or places on this site. A browser drops control
// characters and spaces at a url's start and tabs and newlines inside it before reading
// its scheme, so they go first here too ("\x01javascript:" would otherwise pass as a
// relative link); an entity is already escaped, so it can't spell a scheme either.
export function safeUrl(u) {
  const t = String(u).replace(/[\x00-\x20\x7f]/g, '');
  if (/^(https?:|mailto:)/i.test(t) || /^[/#.?]/.test(t) || !/^[a-z][a-z0-9+.-]*:/i.test(t)) return t;
  return '#';
}

export function inline(src) {
  // code spans first, so nothing inside them is touched
  const parts = String(src).split(/(`+)([\s\S]*?[^`])\1(?!`)/);
  let out = '';
  for (let i = 0; i < parts.length; i++) {
    if (i % 3 === 0) out += spans(esc(parts[i]));
    else if (i % 3 === 2) out += `<code>${esc(parts[i].trim())}</code>`;
  }
  return out;
}

function spans(s) {
  return s
    .replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (_, alt, u) => `<img alt="${alt}" src="${safeUrl(u)}">`)
    .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, t, u) => {
      const url = safeUrl(u);
      const ext = /^https?:/i.test(url) ? ' target="_blank" rel="noopener noreferrer"' : '';
      return `<a href="${url}"${ext}>${t}</a>`;
    })
    .replace(/(^|[\s(])(https?:\/\/[^\s<]+[^\s<.,;:!?)])/g, '$1<a href="$2" target="_blank" rel="noopener noreferrer">$2</a>')
    .replace(/\*\*([^*]+)\*\*|__([^_]+)__/g, (_, a, b) => `<strong>${a ?? b}</strong>`)
    .replace(/(^|[^*\w])\*([^*\s][^*]*?)\*(?!\w)|(^|[^_\w])_([^_\s][^_]*?)_(?!\w)/g, (_, p1, a, p2, b) => `${p1 ?? p2}<em>${a ?? b}</em>`)
    .replace(/~~([^~]+)~~/g, '<del>$1</del>')
    .replace(/ {2,}\n|\\\n/g, '<br>');
}

const isRule = (l) => /^ {0,3}([-*_])( *\1){2,} *$/.test(l);
const isFence = (l) => /^ {0,3}(```|~~~)/.test(l);
const isHeading = (l) => /^ {0,3}#{1,6}(\s|$)/.test(l);
const isQuote = (l) => /^ {0,3}>/.test(l);
const isItem = (l) => /^ {0,3}([-*+]|\d{1,9}[.)])\s+/.test(l);
const isTableSep = (l) => /^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?\s*$/.test(l) && l.includes('-');

function cells(l) {
  let t = l.trim();
  if (t.startsWith('|')) t = t.slice(1);
  if (t.endsWith('|')) t = t.slice(0, -1);
  return t.split('|').map((c) => c.trim());
}

export function render(md) {
  const lines = String(md ?? '').replace(/\r\n?/g, '\n').split('\n');
  return blocks(lines);
}

function blocks(lines) {
  let out = '';
  let i = 0;
  while (i < lines.length) {
    const l = lines[i];
    if (!l.trim()) {
      i++;
      continue;
    }
    if (isFence(l)) {
      const fence = l.trim().slice(0, 3);
      const lang = l.trim().slice(3).trim();
      const code = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(fence)) code.push(lines[i++]);
      i++;
      out += `<pre><code${lang ? ` data-lang="${esc(lang)}"` : ''}>${esc(code.join('\n'))}</code></pre>`;
      continue;
    }
    if (isHeading(l)) {
      const m = l.trim().match(/^(#{1,6})\s*(.*?)\s*#*\s*$/);
      out += `<h${m[1].length}>${inline(m[2])}</h${m[1].length}>`;
      i++;
      continue;
    }
    if (isRule(l)) {
      out += '<hr>';
      i++;
      continue;
    }
    if (isQuote(l)) {
      const q = [];
      while (i < lines.length && lines[i].trim() && (isQuote(lines[i]) || !isItem(lines[i]))) {
        q.push(lines[i].replace(/^ {0,3}> ?/, ''));
        i++;
        if (i < lines.length && !isQuote(lines[i])) break;
      }
      out += `<blockquote>${blocks(q)}</blockquote>`;
      continue;
    }
    if (isItem(l)) {
      const ordered = /^ {0,3}\d/.test(l);
      const start = ordered ? parseInt(l.trim(), 10) : 1;
      const items = [];
      while (i < lines.length) {
        const cur = lines[i];
        if (isItem(cur) && /^ {0,3}\d/.test(cur) === ordered) {
          items.push([cur.replace(/^ {0,3}([-*+]|\d{1,9}[.)])\s+/, '')]);
          i++;
        } else if (cur.trim() && /^\s{2,}/.test(cur) && items.length) {
          items[items.length - 1].push(cur.replace(/^\s{2,4}/, ''));
          i++;
        } else break;
      }
      const li = items
        .map((it) => {
          let body = it.length > 1 ? blocks(it) : inline(it[0]);
          const task = /^\[([ xX])\]\s+/.exec(it[0]);
          if (task && it.length === 1) body = `<input type="checkbox" disabled${task[1] !== ' ' ? ' checked' : ''}> ${inline(it[0].slice(task[0].length))}`;
          return `<li>${body}</li>`;
        })
        .join('');
      out += ordered ? `<ol${start !== 1 ? ` start="${start}"` : ''}>${li}</ol>` : `<ul>${li}</ul>`;
      continue;
    }
    if (l.includes('|') && i + 1 < lines.length && isTableSep(lines[i + 1])) {
      const head = cells(l);
      const align = cells(lines[i + 1]).map((c) => (c.startsWith(':') && c.endsWith(':') ? 'center' : c.endsWith(':') ? 'right' : ''));
      i += 2;
      const rows = [];
      while (i < lines.length && lines[i].includes('|') && lines[i].trim()) rows.push(cells(lines[i++]));
      const td = (tag, c, j) => `<${tag}${align[j] ? ` style="text-align:${align[j]}"` : ''}>${inline(c)}</${tag}>`;
      out += `<table><thead><tr>${head.map((c, j) => td('th', c, j)).join('')}</tr></thead><tbody>${rows
        .map((r) => `<tr>${head.map((_, j) => td('td', r[j] ?? '', j)).join('')}</tr>`)
        .join('')}</tbody></table>`;
      continue;
    }
    // a paragraph runs to a blank line or another block
    const p = [];
    while (i < lines.length && lines[i].trim() && !(p.length && (isFence(lines[i]) || isHeading(lines[i]) || isQuote(lines[i]) || isItem(lines[i]) || isRule(lines[i])))) {
      p.push(lines[i++]);
    }
    out += `<p>${inline(p.join('\n'))}</p>`;
  }
  return out;
}
