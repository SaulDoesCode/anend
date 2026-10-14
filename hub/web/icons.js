// icons (the shared set in assets/icons, copied into iconset.js by tools/icons.mjs), icon
// buttons, and the tooltip every [data-tip] (and every old title) shows: its name and,
// when it has one, its shortcut

import SET from './iconset.js';
import { el } from './util.js';

const NS = 'http://www.w3.org/2000/svg';

export function icon(name, cls = '') {
  const s = document.createElementNS(NS, 'svg');
  for (const [k, v] of Object.entries({
    viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2',
    'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'aria-hidden': 'true', focusable: 'false',
  })) s.setAttribute(k, v);
  s.setAttribute('class', `icon${cls ? ' ' + cls : ''}`);
  s.innerHTML = SET[name] || SET.help;
  return s;
}

// the name a control is known by (to screen readers too) and its shortcut, shown as a tip
export function tip(e, name, key) {
  e.setAttribute('aria-label', name);
  e.dataset.tip = name;
  if (key) {
    e.dataset.key = key;
    e.setAttribute('aria-keyshortcuts', key.replace(/Ctrl/g, 'Control').replace(/Esc\b/, 'Escape'));
  } else delete e.dataset.key;
  e.removeAttribute('title');
  return e;
}

// a button that is an icon: iconButton('save', 'save', {key: 'Ctrl+S', onclick})
// `text` keeps a short word beside the icon; `small` and `cls` style it
export function iconButton(name, label, { key, onclick, small, cls, text, ...props } = {}) {
  const b = el(`button.ibtn${small ? '.small' : ''}${cls ? '.' + cls : ''}`, { type: 'button', onclick, ...props }, icon(name), text ? el('span.lab', text) : null);
  if (text) b.classList.add('with-text');
  return tip(b, label, key);
}

// ------------------------------------------------------------------ the tooltip

let box, at, timer, lastHidden = 0;

function place(target) {
  const r = target.getBoundingClientRect();
  if (!r.width && !r.height) return hide();
  const b = box.getBoundingClientRect();
  const vw = document.documentElement.clientWidth;
  const vh = document.documentElement.clientHeight;
  let x = r.left + r.width / 2 - b.width / 2;
  x = Math.max(4, Math.min(vw - b.width - 4, x));
  let y = r.bottom + 6;
  if (y + b.height > vh - 4) y = r.top - b.height - 6;
  box.style.transform = `translate(${Math.round(x)}px, ${Math.round(Math.max(4, y))}px)`;
}

function show(target) {
  clearTimeout(timer);
  // an old-style title becomes a tip, so the browser's own doesn't show too
  if (target.hasAttribute('title')) {
    const t = target.getAttribute('title');
    target.removeAttribute('title');
    if (t) target.dataset.tip = t;
  }
  const name = target.dataset.tip;
  if (!name) return;
  at = target;
  box.replaceChildren(el('span', name), ...(target.dataset.key ? [el('kbd', target.dataset.key)] : []));
  box.hidden = false;
  box.classList.remove('in');
  place(target);
  requestAnimationFrame(() => box.classList.add('in'));
}

function hide() {
  clearTimeout(timer);
  if (!box || box.hidden) return;
  box.hidden = true;
  at = null;
  lastHidden = Date.now();
}

const tipOf = (n) => (n instanceof Element ? n.closest('[data-tip], [title]') : null);

export function startTips() {
  box = el('div.tip', { role: 'tooltip', hidden: true });
  box.id = 'tip';
  document.body.append(box);
  document.addEventListener('pointerover', (e) => {
    if (e.pointerType === 'touch') return;
    const t = tipOf(e.target);
    if (t === at) return;
    if (!t) return hide();
    clearTimeout(timer);
    // moving along a row of buttons: the next tip comes at once
    const soon = !box.hidden || Date.now() - lastHidden < 500;
    if (!box.hidden) hide();
    timer = setTimeout(() => show(t), soon ? 0 : 450);
  });
  document.addEventListener('pointerout', (e) => {
    const t = tipOf(e.target);
    if (t && !t.contains(e.relatedTarget)) {
      clearTimeout(timer);
      if (t === at) hide();
    }
  });
  document.addEventListener('focusin', (e) => {
    const t = tipOf(e.target);
    if (t && t === e.target && t.matches(':focus-visible')) show(t);
  });
  document.addEventListener('focusout', hide);
  for (const ev of ['pointerdown', 'wheel', 'keydown']) document.addEventListener(ev, (e) => (e.key === 'Tab' ? null : hide()), { capture: true, passive: true });
  window.addEventListener('blur', hide);
  window.addEventListener('resize', hide);
}
