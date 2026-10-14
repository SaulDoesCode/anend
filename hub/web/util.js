// small helpers every view uses

// el('div.card', {onclick}, children...): an element, its class, props and children
export function el(spec, props, ...kids) {
  const [tag, ...classes] = spec.split('.');
  const e = document.createElement(tag || 'div');
  if (classes.length) e.className = classes.join(' ');
  if (props && (typeof props !== 'object' || props instanceof Node || Array.isArray(props))) {
    kids.unshift(props);
    props = null;
  }
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else if (k === 'style' && typeof v === 'object') {
      for (const [p, x] of Object.entries(v)) p.startsWith('--') ? e.style.setProperty(p, x) : (e.style[p] = x);
    }
    else if (k === 'dataset') Object.assign(e.dataset, v);
    else if (k in e && k !== 'list' && k !== 'type') e[k] = v;
    else e.setAttribute(k, v === true ? '' : v);
  }
  add(e, kids);
  return e;
}

function add(e, kids) {
  for (const k of kids) {
    if (k == null || k === false) continue;
    if (Array.isArray(k)) add(e, k);
    else e.append(k instanceof Node ? k : document.createTextNode(String(k)));
  }
}

export function clear(e, ...kids) {
  e.replaceChildren();
  add(e, kids);
  return e;
}

export function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
}

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

// the reader's date: 05 March 2019 | 07:30
export function longDate(iso) {
  const d = new Date(iso);
  if (isNaN(d)) return '';
  const p = (n) => String(n).padStart(2, '0');
  return `${p(d.getDate())} ${MONTHS[d.getMonth()]} ${d.getFullYear()} | ${p(d.getHours())}:${p(d.getMinutes())}`;
}

export function shortDate(iso) {
  const d = new Date(iso);
  if (isNaN(d)) return '';
  return `${d.getDate()} ${MONTHS[d.getMonth()].slice(0, 3)} ${d.getFullYear()}`;
}

// a rough "3 days ago"
export function relative(iso) {
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (isNaN(s)) return '';
  const steps = [
    [45, () => 'a few seconds ago'], [90, () => 'a minute ago'], [2700, () => `${Math.round(s / 60)} minutes ago`],
    [5400, () => 'an hour ago'], [79200, () => `${Math.round(s / 3600)} hours ago`], [129600, () => 'a day ago'],
    [2246400, () => `${Math.round(s / 86400)} days ago`], [3974400, () => 'a month ago'],
    [27648000, () => `${Math.round(s / 2592000)} months ago`], [47347200, () => 'a year ago'],
  ];
  for (const [lim, f] of steps) if (s < lim) return f();
  return `${Math.round(s / 31536000)} years ago`;
}

export function size(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1 << 20) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1 << 20)).toFixed(1)} MB`;
}

export function debounce(f, ms) {
  let t;
  return (...a) => {
    clearTimeout(t);
    t = setTimeout(() => f(...a), ms);
  };
}

// whether a key event is in a text box (single keys leave those alone)
export function typing(e) {
  const t = e.target;
  return t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
}

// a key the way the dispatcher writes them: 0,12 or note:1000001
export const KEY = /^(note:\d+|\d{1,3}(,\d{1,3}){0,3})$/;

// the underline colour of the memory view: hue from the reversed sum, shade from the ordinal
export function underline(ordinal, reversed) {
  const hue = (reversed * 137.50776) % 360;
  const light = 48 + (ordinal % 5) * 6;
  return `hsl(${hue.toFixed(1)} 85% ${light}%)`;
}

// a text whose ordinal sum is n, so mem.measure gives n's prime factor chain
export function textOfOrdinal(n) {
  n = Math.max(0, Math.floor(n));
  return 'z'.repeat(Math.floor(n / 26)) + (n % 26 ? String.fromCharCode(96 + (n % 26)) : '');
}

// a second click within a few seconds confirms (delete, unlist, pour...); an icon button
// shows the label in words while it waits
export function confirmClick(button, label, act) {
  let armed = false;
  let t;
  let kept = null;
  const disarm = () => {
    armed = false;
    button.classList.remove('armed');
    if (kept) button.replaceChildren(...kept);
  };
  button.addEventListener('click', (e) => {
    e.stopPropagation();
    if (armed) {
      clearTimeout(t);
      disarm();
      act();
      return;
    }
    armed = true;
    kept = [...button.childNodes];
    button.replaceChildren(document.createTextNode(label));
    button.classList.add('armed');
    t = setTimeout(disarm, 3000);
  });
  button.addEventListener('blur', () => armed && (clearTimeout(t), disarm()));
  return button;
}

// a file to save, made in the page (bytes or text)
export function saveFile(name, data, type = 'application/octet-stream') {
  const url = URL.createObjectURL(new Blob([data], { type }));
  const a = el('a', { href: url, download: name, hidden: true });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
}

export function fromBase64(b64) {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

// a file the player picked, as base64
export function fileBase64(file) {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result).replace(/^data:[^,]*,/, ''));
    r.onerror = () => reject(r.error || new Error("couldn't read that file"));
    r.readAsDataURL(file);
  });
}

// where a hit opens: a note in the editor, a memory in the memory view's bar
export function openHit(app, h) {
  if (h.note || String(h.key).startsWith('note:')) app.go(`editor/${encodeURIComponent(String(h.key).replace(/^note:/, ''))}`);
  else {
    app.barText.memory = h.text;
    app.go('memory');
  }
}
