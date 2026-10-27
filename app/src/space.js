// the space: memory texts float as camera-facing labels (html laid over a webgl room at
// each text's projected place) and you fly through it. Rust sends the texts with what
// they are (their sums, length, kind, word types, lists, time, class) and the physics'
// settings; this file runs the free spread and draws, and tells Rust what happens.
//
// The spread is a free force-directed layout (no spiral, no anchors): every two texts
// repel, and attract each other as much as they are alike, where "alike" is a weighted
// sum of what the physics panel weighs (sums, bucket, length, word types, lists, class,
// time, letters), each weight from −1 (alike texts push apart) to +1 (pull together). A
// weak gravity keeps the cloud together, labels never overlap, and the integration runs
// in fixed steps with damping and cooling, so it settles whatever the frame rate.
//
// Smooth labels: each label keeps one font size and is placed and scaled with a
// sub-pixel transform every frame (no font-size steps, no whole-pixel snapping).
//
// right mouse drag looks, W A S D move, Space / C up and down, Q / E roll, Shift fast,
// [ ] or the wheel change the speed, R levels, Home goes back; the arrows ease the
// camera onto the nearest text that way. Drag a text to move it (it stays pinned), the
// wheel on a text pushes it away or pulls it near. Blocks (spatial saves) are boxes in
// the webgl room, moved and resized with a gizmo (see "blocks" below).

import * as THREE from '__THREE__';

const LOOK = 0.0026;
const ROLL = 1.4;
const BASE_PX = 32;
// a block's texture repeats once per unit of its sides (core's graph::blocks::TILE)
const BLOCK_TILE = 1;
const STEP = 1 / 60;
// the flying keys by action, as the keymap binds them (Rust sends them: `keys`)
const DEFAULT_KEYS = { fly_forward: 'KeyW', fly_back: 'KeyS', fly_left: 'KeyA', fly_right: 'KeyD', fly_up: 'Space', fly_down: 'KeyC', roll_left: 'KeyQ', roll_right: 'KeyE', level: 'KeyR', back_to_start: 'Home', slower: 'BracketLeft', faster: 'BracketRight' };
const FLYING = ['fly_forward', 'fly_back', 'fly_left', 'fly_right', 'fly_up', 'fly_down', 'roll_left', 'roll_right'];
// the physics' settings (the panel's sliders); Rust sends changes as `physics`
export const PHYSICS = {
  damping: 2.2, gravity: 0.06, repel: 9, link: 4, attract: 0.9, pad: 0.3, cool: 0.012, substeps: 2,
  size: 0.5, wrap: 60, minPx: 1.2, depth: 0, flat: 1.5, qscale: 12, distance: 26,
  w_ordinal: 0.6, w_reversed: 0.3, w_bucket: 0.8, w_length: 0.2, w_types: 0.3, w_lists: 0.7, w_class: 0.5, w_time: 0.2, w_letters: 0.4,
  depth_by: 'none', reversed: 0,
  look: 1, smooth: 14,
};

const typing = (e) => {
  const t = e.target || {};
  return t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable;
};

const hash01 = (s, salt) => {
  let h = 2166136261 ^ salt;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return ((h >>> 0) % 100000) / 100000;
};

const cosine = (a, b) => {
  let d = 0, na = 0, nb = 0;
  for (let i = 0; i < a.length; i++) { d += a[i] * b[i]; na += a[i] * a[i]; nb += b[i] * b[i]; }
  return na && nb ? d / Math.sqrt(na * nb) : 0;
};

const TYPES = ['word', 'name', 'number', 'date', 'time', 'link', 'email', 'hashtag', 'noun', 'verb', 'adjective', 'adverb', 'pronoun', 'determiner', 'preposition', 'conjunction', 'interjection'];

// a block's bend at u across its width (0 the left edge, 1 the right): how far that
// slice moves along the block's depth. The same as core's `Bend::offset`: a ripple whose
// one-period profile is the cubic bezier 0, c1, c2, 0 scaled to peak at its amount, plus
// a fold about the line `at`, each side the bezier 0, c1, c2, 1 times its amount.
export function bendAt(bend, u) {
  let z = 0;
  const r = bend && bend.ripple;
  if (r && r.amount && r.freq > 0) {
    const s = (((u * r.freq + (r.phase || 0)) % 1) + 1) % 1;
    z += (r.amount * wave(s, r.c1, r.c2)) / wavePeak(r.c1, r.c2);
  }
  const f = bend && bend.fold;
  if (f && f.amount) {
    const line = 0.5 + (f.at || 0) * 0.5;
    let t = u >= line ? (u - line) / Math.max(1e-6, 1 - line) : (line - u) / Math.max(1e-6, line);
    t = Math.min(1, Math.max(0, t));
    z += f.amount * (3 * (1 - t) * (1 - t) * t * f.c1 + 3 * (1 - t) * t * t * f.c2 + t * t * t);
  }
  return z;
}
function wave(s, c1, c2) { return 3 * (1 - s) * (1 - s) * s * c1 + 3 * (1 - s) * s * s * c2; }
function wavePeak(c1, c2) {
  let m = 0;
  for (let i = 0; i <= 64; i++) m = Math.max(m, Math.abs(wave(i / 64, c1, c2)));
  return m > 1e-6 ? m : 1;
}
// what a block's geometry depends on (its sides and bend)
function shapeKey(t) { return JSON.stringify([t.w, t.h, t.d, t.bend || null, t.shape || 'box']); }
function localAxis(i) { const a = new THREE.Vector3(); a.setComponent(i, 1); return a; }
// which variant the n-th block of a recipe shows: core's graph::textures::variant, the
// same 32-bit mix
function variantOf(seed, n, variants) {
  if (!(variants > 1)) return 0;
  const lo = n >>> 0, hi = Math.floor(n / 4294967296) >>> 0;
  let h = ((seed >>> 0) ^ Math.imul(lo, 0x9e3779b9) ^ hi) >>> 0;
  h ^= h >>> 16; h = Math.imul(h, 0x85ebca6b) >>> 0;
  h ^= h >>> 13; h = Math.imul(h, 0xc2b2ae35) >>> 0;
  h ^= h >>> 16;
  return (h >>> 0) % variants;
}
export { variantOf };

// a CSS cubic-bezier timing at x (0 to 1): core's graph::anims::bezier, by bisection
function cubicBezier(t, x) {
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  const at = (a, b, s) => 3 * (1 - s) * (1 - s) * s * a + 3 * (1 - s) * s * s * b + s * s * s;
  let lo = 0, hi = 1;
  for (let i = 0; i < 40; i++) { const mid = (lo + hi) / 2; if (at(t[0], t[2], mid) < x) lo = mid; else hi = mid; }
  return at(t[1], t[3], (lo + hi) / 2);
}

function turnOf(b) { const r = b.rot || [0, 0, 0, 1]; return new THREE.Quaternion(r[0], r[1], r[2], r[3]); }

export class Space {
  constructor(host, send) {
    this.host = host;
    this.send = send;
    this.nodes = new Map(); // key → node
    this.order = []; // nodes, in a stable order for the pair loops
    this.sim = null; // the pair weights, row-major over `order`
    this.keys = new Set();
    this.bind = { ...DEFAULT_KEYS };
    this.p = { ...PHYSICS };
    this.speed = 6;
    // the camera's turn as yaw (about the world's up), pitch (clamped short of straight up
    // or down) and roll, kept apart: looking round in circles never builds up a roll.
    // The mouse moves the targets; the camera eases onto them.
    this.yaw = 0; this.pitch = 0; this.rollA = 0;
    this.tyaw = 0; this.tpitch = 0;
    this.running = false;
    this.looking = false;
    this.paused = false;
    this.alpha = 0;
    this.acc = 0;
    this.ease = null;
    this.selected = null;
    this.hover = null;
    this.drag = null;
    this.undo = [];
    this.freeN = 0;
    this.infoAt = 0;
    this.origin = new THREE.Vector3();
    this.quat = new THREE.Quaternion();
    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(50, 1, 0.05, 4000);
    this.room = new THREE.Color('#e4d4be');
    this.scene.background = this.room;
    this.scene.fog = new THREE.Fog(this.room, 60, 260);
    try {
      this.gl = new THREE.WebGLRenderer({ antialias: true });
      this.gl.setPixelRatio(Math.min(devicePixelRatio || 1, 2));
      this.gl.domElement.className = 'space-gl';
      host.append(this.gl.domElement);
    } catch {
      this.gl = null;
    }
    this.overlay = document.createElement('div');
    this.overlay.className = 'space-css';
    host.append(this.overlay);
    this.size = { w: 1, h: 1 };
    this.grid = new THREE.GridHelper(600, 120, 0x000000, 0x000000);
    this.grid.position.y = -9;
    for (const m of [].concat(this.grid.material)) {
      m.transparent = true;
      m.opacity = 0.06;
      m.depthWrite = false;
    }
    this.scene.add(this.grid);
    this.blocks = new Map(); // id → {b, mesh, key, about}
    this.mats = new Map(); // texture key (and a recipe's variant) → material
    this.abouts = new Map(); // texture key → what it is (as Rust sent it)
    this.sky = null;
    this.lights = []; // the room's lights (core's graph::lights), baked onto the blocks
    this.anims = new Map(); // animation key → the animation (core's graph::anims)
    this.animT0 = performance.now(); // when the room's animations started
    this.animPlaying = true;
    this.lightMarks = null; // where the point and cone lights are, while the lights window is open
    this.selBlock = null;
    this.bdrag = null;
    this.ray = new THREE.Raycaster();
    this.gizmo = this.makeGizmo();
    this.resize = this.resize.bind(this);
    new ResizeObserver(this.resize).observe(host);
    this.resize();
    this.listen();
    this.loop = this.loop.bind(this);
    this.setFrame();
  }

  // ------------------------------------------------------------------ settings

  setKeys(map) {
    this.bind = { ...DEFAULT_KEYS, ...map };
    this.keys.clear();
  }

  held(action) {
    return this.keys.has(this.bind[action]);
  }

  setPhysics(p) {
    const old = this.p;
    this.p = { ...PHYSICS, ...p };
    const weights = Object.keys(PHYSICS).filter((k) => k.startsWith('w_') || k === 'qscale' || k === 'reversed');
    if (weights.some((k) => old[k] !== this.p[k])) this.sim = null;
    if (old.wrap !== this.p.wrap) for (const n of this.order) this.measure(n);
    this.wake(0.6);
  }

  setRoom(color, ink) {
    this.room.set(color);
    this.scene.fog.color.set(color);
    this.host.style.setProperty('--room-bg', color);
    this.host.style.setProperty('--room-ink', ink);
    this.host.style.background = color;
    const dark = this.room.r * 0.3 + this.room.g * 0.59 + this.room.b * 0.11 < 0.35;
    for (const m of [].concat(this.grid.material)) {
      m.color.set(dark ? 0xffffff : 0x000000);
      m.opacity = dark ? 0.05 : 0.06;
    }
    this.render();
  }

  resize() {
    const w = this.host.clientWidth || 1;
    const h = this.host.clientHeight || 1;
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.gl?.setSize(w, h);
    this.size = { w, h };
    this.render();
  }

  start() {
    if (this.running) return;
    this.running = true;
    this.last = performance.now();
    this.resize();
    requestAnimationFrame(this.loop);
  }

  stop() {
    this.running = false;
    this.keys.clear();
    this.looking = false;
  }

  // the cloud's centre: ahead of where the camera is when a search lays texts out
  setFrame() {
    this.quat.copy(this.camera.quaternion);
    this.origin.set(0, 0, -this.p.distance).applyQuaternion(this.quat).add(this.camera.position);
  }

  world(n, out = new THREE.Vector3()) {
    return out.set(n.x, n.y, n.z).applyQuaternion(this.quat).add(this.origin);
  }

  local(w, n) {
    const v = w.clone().sub(this.origin).applyQuaternion(this.quat.clone().invert());
    n.x = v.x; n.y = v.y; n.z = v.z;
  }

  // ------------------------------------------------------------------ texts

  // texts: [{key, text, note, s, r, q, hue, chars, words, kind, types, created, lists, freq, cls, strength, colour}]
  place(texts, again) {
    if (!again) {
      this.setFrame();
      for (const n of [...this.order]) if (!n.pinned && !n.free) this.drop(n);
    }
    const before = this.order.length;
    for (const t of texts) {
      let n = this.nodes.get(String(t.key));
      if (n) {
        // a detached placement keeps the text it was detached with
        const kept = n.f.text;
        Object.assign(n.f, t);
        if (n.det) n.f.text = kept;
        this.showText(n);
        const was = (n.links || []).map((c) => c.f.key + '|' + c.f.text).join('\n');
        const now = (Array.isArray(t.chain) ? t.chain : []).map((m) => m.key + '|' + m.text).join('\n');
        if (was !== now) this.link(n, t.chain);
        n.el.style.borderBottomColor = t.under || 'transparent';
        continue;
      }
      n = this.make(t, false);
      this.seed(n);
    }
    this.sim = null;
    this.wake(1);
    this.render();
    this.info(true);
    return this.order.length - before;
  }

  // a placement of a text: `free` is the room's own text (no entry); `how` says how a
  // copy relates to its entry: { det } detached at that version (it keeps the text it was
  // copied with), { own } an inheriting copy's own text, shadowing the entry's
  make(t, free, nodeKey, how) {
    const el = document.createElement('div');
    el.className = 'slabel' + (free ? ' free' : '') + (t.note ? ' note' : '');
    el.textContent = t.note ? `✎ ${t.text}` : t.text;
    // the gematria grouping underline (the memory view's colour), on every text that has one
    el.style.borderBottomColor = t.under || 'transparent';
    if (t.colour) el.style.color = t.colour;
    el.tabIndex = -1;
    const n = { key: nodeKey || String(t.key), f: t, el, x: 0, y: 0, z: 0, vx: 0, vy: 0, vz: 0, hw: 1, hh: 0.4, pinned: false, free, det: 0, own: null };
    if (how && how.det) n.det = how.det;
    if (how && how.own != null) n.own = how.own;
    this.showText(n);
    this.overlay.append(el);
    this.nodes.set(n.key, n);
    this.order.push(n);
    this.wire(n);
    this.measure(n);
    if (t.isChain) el.classList.add('chain-head');
    this.link(n, t.chain);
    return n;
  }

  // a chain's members under its head (a chain entry's, or a note's body as a chain of
  // two): nodes of their own, laid out in reading order by `follow`, never moved by the
  // spread; dragging any of them drags the head
  link(n, members) {
    for (const c of n.links || []) this.drop(c);
    n.links = [];
    if (!Array.isArray(members)) return;
    members.forEach((m, i) => {
      const c = this.make({ key: String(m.key), text: m.text, note: !!m.note, virt: !!m.virt }, false, `${n.key}>${i}>${m.key}`);
      c.head = n;
      c.idx = i;
      c.el.classList.add('member');
      c.x = n.x; c.y = n.y - (i + 1) * 0.9; c.z = n.z;
      n.links.push(c);
    });
    this.sim = null;
  }

  // a chain changed (or broke, `chain` null): every head showing it takes its members
  // again; a broken one becomes its title again
  setChain(key, chain) {
    for (const n of [...this.order]) {
      if (n.head || String(n.f.key) !== String(key)) continue;
      if (!chain) {
        // broken: the head shows its title, and the members stay as texts where they are
        n.f.key = n.f.titleKey || n.f.key;
        n.f.isChain = false;
        for (const c of n.links || []) {
          c.head = null;
          c.el.classList.remove('member');
          c.pinned = true;
          c.el.classList.add('pinned');
        }
        n.links = [];
        n.el.classList.remove('chain-head');
        continue;
      }
      n.f.text = chain.title.text;
      n.f.titleKey = chain.title.key;
      n.f.isChain = true;
      n.el.classList.add('chain-head');
      this.showText(n);
      this.measure(n);
      this.link(n, chain.members);
    }
    this.wake(0.5);
    this.render();
  }

  // a text in the room becomes a chain's head (a chain just made with it as title)
  chainify(node, chain) {
    const n = this.nodes.get(node);
    if (!n) return;
    n.f.key = chain.key;
    this.setChain(chain.key, chain);
  }

  // a new chain (from a cut) beside the head it came from
  addChain(near, chain) {
    const h = this.order.find((x) => !x.head && String(x.f.key) === String(near));
    const n = this.make({ key: chain.key, text: chain.title.text, note: false, titleKey: chain.title.key, isChain: true, chain: chain.members }, false,
      this.nodes.has(String(chain.key)) ? this.copyKey(chain.key) : null);
    n.el.classList.add('chain-head');
    if (h) { n.x = h.x + h.hw * 2 + 1.5; n.y = h.y; n.z = h.z; }
    n.pinned = true;
    n.el.classList.add('pinned');
    this.render();
  }

  // members placed in reading order under their head, left-aligned, spaced by their own
  // heights, eased there like a spring (so a dragged chain follows and settles)
  follow(h) {
    const k = Math.min(1, 10 * h);
    let moved = 0;
    for (const n of this.order) {
      if (!n.links || !n.links.length) continue;
      const gap = 0.12;
      const left = n.x - n.hw;
      let y = n.y - n.hh - gap;
      for (const c of n.links) {
        const tx = left + c.hw, ty = y - c.hh;
        y -= c.hh * 2 + gap;
        const dx = tx - c.x, dy = ty - c.y, dz = n.z - c.z;
        c.x += dx * k; c.y += dy * k; c.z += dz * k;
        c.vx = c.vy = c.vz = 0;
        moved += dx * dx + dy * dy + dz * dz;
      }
    }
    return moved;
  }

  // a new text starts beside the texts most like it, or at a spot of its own
  seed(n) {
    const R = 2 + Math.sqrt(this.order.length) * 1.4;
    const a = hash01(n.key, 1) * Math.PI * 2;
    const r = R * Math.sqrt(hash01(n.key, 2));
    n.x = Math.cos(a) * r;
    n.y = Math.sin(a) * r;
    n.z = (hash01(n.key, 3) - 0.5) * 0.5;
  }

  measure(n) {
    const free = n.free;
    const hidden = n.el.style.display === 'none';
    if (hidden) n.el.style.display = '';
    n.el.style.fontSize = `${BASE_PX}px`;
    n.el.style.maxWidth = free ? '80ch' : `${this.p.wrap}ch`;
    const w = n.el.offsetWidth || BASE_PX * 0.55 * n.f.text.length;
    const h = n.el.offsetHeight || BASE_PX * 1.2;
    const size = n.size || this.p.size;
    n.hw = (w / BASE_PX / 2) * size;
    n.hh = (h / BASE_PX / 2) * size;
    if (hidden) n.el.style.display = 'none';
  }

  drop(n) {
    for (const c of n.links || []) if (c !== n) this.drop(c);
    n.links = [];
    n.el.remove();
    this.nodes.delete(n.key);
    this.order = this.order.filter((x) => x !== n);
    if (this.selected === n) this.selected = null;
    if (this.hover === n) this.hover = null;
    this.sim = null;
  }

  clear(all) {
    for (const n of [...this.order]) if (all || (!n.pinned && !n.free)) this.drop(n);
    this.undo = [];
    this.render();
    this.info(true);
  }

  mark(list, candidate) {
    for (const n of this.order) n.el.classList.toggle('picked', n.key === list || n.key === candidate);
  }

  colours(map) {
    for (const n of this.order) {
      const c = map[n.key];
      n.el.style.color = c || '';
    }
  }

  // ------------------------------------------------------------------ the spread

  wake(a) {
    this.dirty = true;
    this.alpha = Math.max(this.alpha, a);
  }

  // how alike two texts are, from −1 to 1, by the weighted measures
  alike(a, b) {
    const p = this.p, A = a.f, B = b.f;
    let sum = 0, w = 0;
    const add = (weight, m) => {
      if (!weight || m === null || m === undefined || Number.isNaN(m)) return;
      sum += weight * m;
      w += Math.abs(weight);
    };
    add(p.w_ordinal, Math.exp(-Math.abs((A.s || 0) - (B.s || 0)) / p.qscale));
    add(p.w_reversed, Math.exp(-Math.abs((A.r || 0) - (B.r || 0)) / p.qscale));
    add(p.w_bucket, A.q === B.q ? 1 : 0);
    add(p.w_length, Math.exp(-Math.abs(Math.log1p(A.chars || 1) - Math.log1p(B.chars || 1)) * 1.5));
    if (A.types && B.types) {
      const va = TYPES.map((t) => A.types[t] || 0), vb = TYPES.map((t) => B.types[t] || 0);
      add(p.w_types, 0.5 * cosine(va, vb) + 0.5 * (A.kind === B.kind ? 1 : 0));
    }
    const la = A.lists || [], lb = B.lists || [];
    if (la.length || lb.length) {
      const shared = la.filter((x) => lb.includes(x)).length;
      add(p.w_lists, shared / (la.length + lb.length - shared));
    } else add(p.w_lists, 0);
    if (A.cls !== undefined && A.cls !== null && B.cls !== undefined && B.cls !== null) {
      add(p.w_class, A.cls === B.cls ? Math.min(A.strength ?? 1, B.strength ?? 1) : 0);
    }
    if (A.created && B.created) add(p.w_time, Math.exp(-Math.abs(A.created - B.created) / (7 * 864e5)));
    if (A.freq && B.freq) add(p.w_letters, cosine(A.freq, B.freq));
    return w ? sum / w : 0;
  }

  // ------------------------------------------------------------------ staging zones
  //
  // A block can be a staging zone (core's graph::blocks::Zone): the texts inside it spread
  // among themselves, pulled to its centre and kept inside its box, under its own physics
  // (repel, attract, link, gravity, damping; each left out follows the room's). Texts
  // outside and inside don't push or pull each other. Zones nest: a text belongs to the
  // smallest zone it's in. Moving a zone carries its texts along.

  // the zones in the spread's own frame (texts' x y z are relative to `origin`, `quat`)
  zoneFrames() {
    if (!this.zoneBlocks) this.zoneBlocks = [...this.blocks.values()].filter((o) => o.b.zone).map((o) => o.b);
    if (!this.zoneBlocks.length) return [];
    const qi = this.quat.clone().invert();
    return this.zoneBlocks.map((b) => {
      const c = new THREE.Vector3(b.x, b.y, b.z).sub(this.origin).applyQuaternion(qi);
      const q = qi.clone().multiply(turnOf(b));
      return { id: b.id, c, q, qi: q.clone().invert(), half: [b.w / 2, b.h / 2, b.d / 2], vol: b.w * b.h * b.d, p: { ...this.p, ...Object.fromEntries(Object.entries(b.zone).filter(([, v]) => v != null)) } };
    });
  }

  // the smallest zone a text is in (its id), or null
  zoneOf(n, frames) {
    let best = null;
    const v = new THREE.Vector3();
    for (const f of frames) {
      v.set(n.x, n.y, n.z).sub(f.c).applyQuaternion(f.qi);
      if (Math.abs(v.x) <= f.half[0] && Math.abs(v.y) <= f.half[1] && Math.abs(v.z) <= f.half[2] && (!best || f.vol < best.vol)) best = f;
    }
    return best ? best.id : null;
  }

  // a text kept inside its zone's box (its label's half size in from the sides, at most
  // half the box)
  keepIn(n, f) {
    const v = new THREE.Vector3(n.x, n.y, n.z).sub(f.c).applyQuaternion(f.qi);
    const m = [Math.min(n.hw || 0.3, f.half[0] * 0.45), Math.min(n.hh || 0.2, f.half[1] * 0.45), Math.min(0.3, f.half[2] * 0.45)];
    const c = [v.x, v.y, v.z].map((x, i) => Math.max(-f.half[i] + m[i], Math.min(f.half[i] - m[i], x)));
    if (c[0] === v.x && c[1] === v.y && c[2] === v.z) return false;
    v.set(c[0], c[1], c[2]).applyQuaternion(f.q).add(f.c);
    n.x = v.x; n.y = v.y; n.z = v.z;
    return true;
  }

  // which zone each text is in: members of a zone that moved come along first
  assignZones(frames) {
    const byId = new Map(frames.map((f) => [f.id, f]));
    for (const n of this.order) {
      const f = n.zone && byId.get(n.zone);
      if (f && !n.pinned) this.keepIn(n, f);
      const was = n.zone;
      n.zone = frames.length ? this.zoneOf(n, frames) : null;
      if (was !== n.zone && this.live) {
        if (was) this.wireZone(was, n.key, 'leave');
        if (n.zone) this.wireZone(n.zone, n.key, 'enter');
      }
    }
  }

  weights() {
    this.zonesDirty = true;
    const n = this.order.length;
    this.sim = new Float32Array(n * n);
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        const s = this.order[i].free || this.order[j].free ? 0 : this.alike(this.order[i], this.order[j]);
        this.sim[i * n + j] = s;
        this.sim[j * n + i] = s;
      }
    }
  }

  // the depth a text is drawn toward: a measure, or none
  depthOf(n) {
    const p = this.p, f = n.f;
    switch (p.depth_by) {
      case 'ordinal': return ((f.s || 0) - this.mean.s) / Math.max(1, this.mean.ss);
      case 'length': return Math.log1p(f.chars || 1) - this.mean.l;
      case 'time': return f.created ? (f.created - this.mean.t) / Math.max(864e5, this.mean.tt) : 0;
      default: return 0;
    }
  }

  step(h) {
    const p = this.p, N = this.order;
    const n = N.length;
    if (!n) return;
    if (!this.sim || this.sim.length !== n * n) this.weights();
    if (p.depth_by !== 'none') {
      let s = 0, l = 0, t = 0, k = 0;
      for (const x of N) { s += x.f.s || 0; l += Math.log1p(x.f.chars || 1); if (x.f.created) { t += x.f.created; k++; } }
      const ms = s / n, ml = l / n, mt = k ? t / k : 0;
      let ss = 0, tt = 0;
      for (const x of N) { ss += ((x.f.s || 0) - ms) ** 2; if (x.f.created) tt += (x.f.created - mt) ** 2; }
      this.mean = { s: ms, ss: Math.sqrt(ss / n), l: ml, t: mt, tt: Math.sqrt(tt / Math.max(1, k)) };
    }
    const a = this.alpha;
    const frames = this.zoneFrames();
    if (this.zonesDirty) {
      this.assignZones(frames);
      this.zonesDirty = false;
    }
    const zones = new Map(frames.map((f) => [f.id, f]));
    const fx = new Float64Array(n), fy = new Float64Array(n), fz = new Float64Array(n);
    for (let i = 0; i < n; i++) {
      const A = N[i];
      // a text in a zone moves under the zone's physics, toward its centre
      const zf = A.zone ? zones.get(A.zone) : null;
      const pp = zf ? zf.p : p;
      const L = pp.link;
      for (let j = i + 1; j < n; j++) {
        const B = N[j];
        // a chain's members take no part in the spread's forces: their head stands for them
        if (A.head || B.head) continue;
        // texts in different zones (or one in, one out) leave each other alone
        if ((A.zone || null) !== (B.zone || null)) continue;
        let dx = B.x - A.x, dy = B.y - A.y, dz = B.z - A.z;
        let d2 = dx * dx + dy * dy + dz * dz;
        if (d2 < 1e-6) { dx = (hash01(A.key + B.key, 7) - 0.5) * 0.01; dy = (hash01(B.key + A.key, 9) - 0.5) * 0.01; d2 = dx * dx + dy * dy + 1e-6; }
        const d = Math.sqrt(d2);
        const s = this.sim[i * n + j];
        // every two repel (more when the weights say alike texts should part), softened near
        let f = -pp.repel * (1 + Math.max(0, -s) * 2) / (d2 + 0.5);
        // alike texts pull toward a rest length that shrinks the more alike they are
        if (s > 0) f += pp.attract * s * (d - L * (1.15 - s));
        const ux = dx / d, uy = dy / d, uz = dz / d;
        fx[i] += f * ux; fy[i] += f * uy; fz[i] += f * uz;
        fx[j] -= f * ux; fy[j] -= f * uy; fz[j] -= f * uz;
      }
      // gravity to the centre (a zone's own), and toward the depth the measure gives
      if (zf) {
        fx[i] -= pp.gravity * (A.x - zf.c.x);
        fy[i] -= pp.gravity * (A.y - zf.c.y);
        fz[i] -= p.flat * (A.z - zf.c.z);
        continue;
      }
      fx[i] -= p.gravity * A.x;
      fy[i] -= p.gravity * A.y;
      const zt = p.depth_by === 'none' ? 0 : this.depthOf(A) * p.depth;
      fz[i] -= p.flat * (A.z - zt);
    }
    let ke = 0;
    for (let i = 0; i < n; i++) {
      const A = N[i];
      if (A.pinned || A.head || A === this.drag?.n) { A.vx = A.vy = A.vz = 0; continue; }
      const zf = A.zone ? zones.get(A.zone) : null;
      const drag = Math.exp(-(zf ? zf.p.damping : p.damping) * h);
      A.vx = (A.vx + a * fx[i] * h) * drag;
      A.vy = (A.vy + a * fy[i] * h) * drag;
      A.vz = (A.vz + a * fz[i] * h) * drag;
      const v2 = A.vx * A.vx + A.vy * A.vy + A.vz * A.vz;
      if (v2 > 900) { const k = 30 / Math.sqrt(v2); A.vx *= k; A.vy *= k; A.vz *= k; }
      A.x += A.vx * h; A.y += A.vy * h; A.z += A.vz * h;
      // bounded by its zone: the speed into a side is lost there
      if (zf && this.keepIn(A, zf)) { A.vx *= 0.5; A.vy *= 0.5; A.vz *= 0.5; }
      ke += v2;
    }
    ke += this.follow(h) * 100;
    // labels never overlap: separate rectangles in the facing plane, along the shorter way
    for (let pass = 0; pass < 2; pass++) {
      for (let i = 0; i < n; i++) {
        const A = N[i];
        for (let j = i + 1; j < n; j++) {
          const B = N[j];
          if (Math.abs(A.z - B.z) > 1.5) continue;
          if ((A.zone || null) !== (B.zone || null)) continue;
          const ox = A.hw + B.hw + p.pad - Math.abs(B.x - A.x);
          const oy = A.hh + B.hh + p.pad * 0.5 - Math.abs(B.y - A.y);
          if (ox <= 0 || oy <= 0) continue;
          // a chain's members aren't pushed (their head is), nor pushed off their own head
          if ((A.head && (A.head === B || A.head === B.head)) || (B.head && B.head === A)) continue;
          const am = A.pinned || A.head || A === this.drag?.n ? 0 : 1, bm = B.pinned || B.head || B === this.drag?.n ? 0 : 1;
          if (!am && !bm) continue;
          const share = am && bm ? 0.5 : 1;
          if (ox / (A.hw + B.hw) < oy / (A.hh + B.hh)) {
            const sgn = B.x >= A.x ? 1 : -1;
            A.x -= sgn * ox * share * am; B.x += sgn * ox * share * bm;
          } else {
            const sgn = B.y >= A.y ? 1 : -1;
            A.y -= sgn * oy * share * am; B.y += sgn * oy * share * bm;
          }
        }
      }
    }
    this.alpha = Math.max(0, a - a * p.cool);
    this.energy = ke / n;
  }

  asleep() {
    return this.alpha < 0.01 && (this.energy ?? 0) < 0.0005;
  }

  settle() {
    this.wake(1);
    const t0 = performance.now();
    let i = 0;
    while (i < 6000 && performance.now() - t0 < 400) {
      this.step(STEP);
      i++;
      if (this.asleep()) break;
    }
    this.render();
    return i;
  }

  // ------------------------------------------------------------------ editing

  pick(n) {
    if (this.selected) this.selected.el.classList.remove('selected');
    this.selected = n;
    if (n) n.el.classList.add('selected');
    if (n && this.selBlock != null) this.pickBlock(null);
    this.send(this.ev('select', n));
  }

  record(u) {
    this.undo.push(u);
    if (this.undo.length > 300) this.undo.shift();
  }

  undoLast() {
    const u = this.undo.pop();
    if (!u) return 'nothing';
    if (u.kind === 'move') {
      Object.assign(u.n, { x: u.x, y: u.y, z: u.z, pinned: u.pinned });
    } else if (u.kind === 'pin') {
      u.n.pinned = u.was;
    } else if (u.kind === 'remove') {
      this.overlay.append(u.n.el);
      this.nodes.set(u.n.key, u.n);
      this.order.push(u.n);
      this.sim = null;
    } else if (u.kind === 'add') {
      this.drop(u.n);
    } else if (u.kind === 'text') {
      this.setText(u.n, u.from);
    } else if (u.kind === 'size') {
      u.n.size = u.from;
      this.measure(u.n);
      this.send(this.ev('select', u.n));
    } else if (u.kind === 'own') {
      u.n.own = u.from;
      this.showText(u.n);
      this.measure(u.n);
    } else if (u.kind === 'blockadd') {
      for (const id of u.ids) this.dropBlock(id);
      this.send({ ...this.ev('blockgone', null), data: u.ids });
    } else if (u.kind === 'unblock') {
      const textures = {};
      for (const i of u.items) textures[i.key] = i.about;
      this.setBlocks(u.items.map((i) => i.b), textures);
      this.send({ ...this.ev('blockrestore', null), data: u.items.map((i) => i.b) });
    } else if (u.kind === 'block') {
      const o = this.blocks.get(u.id);
      if (o) {
        Object.assign(o.b, u.t);
        this.shapeBlock(o);
        this.send({ ...this.ev('blockset', null), data: this.blockOut(o) });
      }
    }
    this.wake(0.5);
    this.render();
    return u.kind;
  }

  pinSelected() {
    const n = this.selected;
    if (!n) return null;
    this.record({ kind: 'pin', n, was: n.pinned });
    n.pinned = !n.pinned;
    n.el.classList.toggle('pinned', n.pinned);
    this.wake(0.5);
    return n.pinned ? 'pinned' : 'unpinned';
  }

  removeSelected() {
    const n = this.selected;
    if (!n) return null;
    this.record({ kind: 'remove', n });
    this.drop(n);
    this.render();
    this.info(true);
    return n.key;
  }

  copyKey(entry) {
    let i = 2;
    while (this.nodes.has(`${entry}~${i}`)) i++;
    return `${entry}~${i}`;
  }

  // what a placement shows: its own text, else the text it was detached with or its
  // entry's (both in f.text), and its marks
  showText(n) {
    // a long text shows its first piece (`shown`, made by Rust); its whole text stays in f.text
    const t = n.own != null ? n.own : n.det ? n.f.text : n.f.shown || n.f.text;
    n.el.textContent = n.f.note && !n.f.virt ? `✎ ${t}` : t;
    n.el.classList.toggle('det', !!n.det);
    n.el.classList.toggle('own', n.own != null);
  }

  // another placement of the selected (or named) text beside it: the same entry, not a
  // new one; Ctrl+Z takes it away. `det` (the entry's edited time now, ms) makes a
  // detached copy, which keeps the text as it reads now; without it the copy inherits
  // (and keeps the original's own text, if it has one)
  duplicate(node, det) {
    const n = node ? this.nodes.get(node) : this.selected;
    if (!n) return null;
    const how = n.free ? null : det ? { det } : n.det ? { det: n.det } : { own: n.own };
    const f = { ...n.f };
    const c = this.make(f, n.free, n.free ? `t:${Date.now().toString(36)}${this.freeN++}` : this.copyKey(n.f.key), how);
    if (n.size) { c.size = n.size; this.measure(c); }
    c.x = n.x + n.hw * 2 + 0.6; c.y = n.y; c.z = n.z;
    // below any copy already beside it, so copies don't land on each other
    const near = (x) => x !== c && Math.abs(x.x - c.x) < c.hw && Math.abs(x.y - c.y) < c.hh * 2 && Math.abs(x.z - c.z) < 0.5;
    for (let i = 0; i < 50 && this.order.some(near); i++) c.y -= c.hh * 2 + 0.25;
    c.pinned = true;
    c.el.classList.add('pinned');
    this.record({ kind: 'add', n: c });
    this.pick(c);
    this.sim = null;
    this.render();
    this.info(true);
    return c.key;
  }

  // takes one placement out of the room (the entry stays in the save)
  removeNode(node) {
    const n = this.nodes.get(node);
    if (!n) return null;
    this.record({ kind: 'remove', n });
    this.drop(n);
    this.render();
    this.info(true);
    return n.key;
  }

  pinNode(node) {
    const n = this.nodes.get(node);
    if (!n) return null;
    this.pick(n);
    return this.pinSelected();
  }

  // a free text (not a memory) ahead of the camera
  addFree(text) {
    const key = `t:${Date.now().toString(36)}${this.freeN++}`;
    const n = this.make({ key, text: text || 'text', note: false }, true);
    const ahead = new THREE.Vector3(0, 0, -6).applyQuaternion(this.camera.quaternion).add(this.camera.position);
    this.local(ahead, n);
    n.pinned = true;
    this.record({ kind: 'add', n });
    this.pick(n);
    this.render();
    return key;
  }

  setText(n, text) {
    n.f.text = text;
    // the first piece Rust made was of the old text
    n.f.shown = null;
    this.showText(n);
    this.measure(n);
    this.wake(0.3);
  }

  // a text edited: a free text's own, or an entry's, which every placement inheriting
  // from it shows (a detached copy keeps its text)
  editFree(key, text) {
    const n = this.nodes.get(key);
    if (n && n.free) {
      this.record({ kind: 'text', n, from: n.f.text });
      this.setText(n, text);
    } else {
      for (const x of this.order) {
        if (!x.free && !x.det && String(x.f.key) === String(key)) this.setText(x, text);
      }
    }
    this.render();
  }

  // a placement's font size in steps of 1/50 of a unit per em (format.md: a u8, 0 the
  // default, which is the "text size" slider); `by` steps it, `to` sets it (0: default);
  // Ctrl+Z undoes it
  setSize(node, by, to) {
    const n = node ? this.nodes.get(node) : this.selected;
    if (!n) return null;
    const now = n.size ? Math.round(n.size * 50) : 0;
    const base = now || Math.round(this.p.size * 50);
    const step = to != null ? to : Math.max(1, Math.min(255, base + by));
    this.record({ kind: 'size', n, from: n.size });
    n.size = step ? step / 50 : undefined;
    this.measure(n);
    this.sim = null;
    this.wake(0.3);
    this.render();
    this.send(this.ev('select', n));
    return step;
  }

  // an inheriting copy's own text (null: it follows its entry again); Ctrl+Z undoes it
  setOwn(node, text) {
    const n = this.nodes.get(node);
    if (!n || n.free || n.det) return null;
    this.record({ kind: 'own', n, from: n.own });
    n.own = text == null || text === '' ? null : text;
    this.showText(n);
    this.measure(n);
    this.wake(0.3);
    this.render();
    return n.key;
  }

  // a map: every text with its place, as the Bevy app kept them, [text, x, y, z, {size,
  // key}], with how a copy relates to its entry (detached: the version it keeps; own: its
  // own text)
  rows() {
    const w = new THREE.Vector3();
    // a chain is one row, its head's (its members are laid out from it)
    return this.order.filter((n) => !n.head).map((n) => {
      this.world(n, w);
      const o = {};
      if (!n.free) o.key = String(n.f.key);
      if (n.size) o.size = n.size;
      if (!n.free && n.det) o.detached = n.det;
      if (!n.free && n.own != null) o.own = n.own;
      return [n.own != null ? n.own : n.f.text, +w.x.toFixed(2), +w.y.toFixed(2), +w.z.toFixed(2), o];
    });
  }

  loadRows(rows, feats) {
    this.clear(true);
    this.setFrame();
    const w = new THREE.Vector3();
    for (const [text, x, y, z, o] of rows) {
      const key = o && o.key ? String(o.key) : `t:${Date.now().toString(36)}${this.freeN++}`;
      const f = { ...((o && o.key && feats[o.key]) || { key, text, note: false }) };
      const how = o && o.key ? { det: o.detached || 0, own: o.own != null ? o.own : null } : null;
      // an own text shadows the entry's, which the features carry; a detached copy keeps
      // the text the row has
      if (!(how && how.own != null && f.text != null)) f.text = text;
      const n = this.make(f, !(o && o.key), this.nodes.has(key) ? this.copyKey(key) : null, how);
      if (o && o.size) { n.size = o.size; this.measure(n); }
      this.local(w.set(x, y, z), n);
      n.pinned = true;
      n.el.classList.add('pinned');
    }
    this.sim = null;
    this.render();
    this.info(true);
  }

  // ------------------------------------------------------------------ blocks
  //
  // Spatial saves: boxes in the room (world coordinates, kept by Rust in the save's
  // `graph blocks` table) showing a text, a picture or a colour. One texture and one
  // material per texture key, shared by every block showing it (as the table keeps one
  // key per texture); each block's geometry has its UVs scaled to its size with repeat
  // wrapping, so a bigger block shows more tiles, never a stretched one. The selected
  // block has a gizmo: the arrows move it along an axis, the knobs on its faces resize
  // it (the opposite face stays put), the ball moves it in the view's plane; Shift snaps
  // to quarter units.

  textCanvas(text, colour) {
    const c = document.createElement('canvas');
    c.width = c.height = 256;
    const g = c.getContext('2d');
    g.fillStyle = '#f3ead9';
    g.fillRect(0, 0, 256, 256);
    // the text's grouping colour as a band at the bottom, as the labels' underline
    g.fillStyle = colour || '#8a7350';
    g.fillRect(0, 244, 256, 12);
    g.strokeStyle = 'rgba(60, 45, 25, 0.25)';
    g.lineWidth = 2;
    g.strokeRect(1, 1, 254, 254);
    g.fillStyle = '#2b2219';
    g.textAlign = 'center';
    g.textBaseline = 'middle';
    const words = String(text || '').split(/\s+/).filter(Boolean);
    let size = 64;
    let lines = [];
    for (; size >= 12; size -= 4) {
      g.font = `${size}px "Fira Sans", sans-serif`;
      lines = [];
      let line = '';
      for (const w of words) {
        const t = line ? `${line} ${w}` : w;
        if (g.measureText(t).width > 224 && line) { lines.push(line); line = w; } else line = t;
      }
      if (line) lines.push(line);
      if (lines.length * size * 1.15 <= 220 && lines.every((l) => g.measureText(l).width <= 232)) break;
    }
    const top = 122 - ((lines.length - 1) * size * 1.15) / 2;
    lines.slice(0, 12).forEach((l, i) => g.fillText(l, 128, top + i * size * 1.15));
    return c;
  }

  // ------------------------------------------------------------------ texture recipes
  //
  // A recipe (core's graph::textures) is drawn into a tile: its source (a picture, a
  // text drawn as a text block's is, or a colour), cropped, mirrored, turned a quarter
  // at a time and tinted. The tile repeats every `scale` units, shifted by `offset`; a
  // block's variant k shifts it by (k·0.381966, k·0.618034) of a tile and, with
  // `turn_variants`, turns it k more quarters. The same tile shows in the textures
  // window's previews (`preview`).

  // a recipe's tile; a picture's arrives later, and `redraw` is called when it has
  recipeCanvas(about, extraTurns, redraw, size = 256) {
    const r = about.recipe || {};
    const src = r.source || {};
    const c = document.createElement('canvas');
    c.width = c.height = size;
    const draw = (img) => {
      const g = c.getContext('2d');
      g.save();
      g.clearRect(0, 0, size, size);
      g.translate(size / 2, size / 2);
      g.rotate((((r.turn || 0) + (extraTurns || 0)) % 4) * Math.PI / 2);
      if (r.mirror) g.scale(-1, 1);
      g.translate(-size / 2, -size / 2);
      if (src.plain) {
        g.fillStyle = src.plain;
        g.fillRect(0, 0, size, size);
      } else if (img) {
        const [l, t, rr, b] = r.crop || [0, 0, 1, 1];
        const w = img.width, h = img.height;
        g.drawImage(img, l * w, t * h, Math.max(1, (rr - l) * w), Math.max(1, (b - t) * h), 0, 0, size, size);
      } else {
        g.fillStyle = about.gone || about.picture_gone ? '#9a8f84' : '#b8a07a';
        g.fillRect(0, 0, size, size);
      }
      g.restore();
      if (r.tint && r.tint_amount) {
        g.globalAlpha = r.tint_amount;
        g.fillStyle = r.tint;
        g.fillRect(0, 0, size, size);
        g.globalAlpha = 1;
      }
    };
    if (src.text != null) draw(this.textCanvas(about.text_gone ? '(gone)' : about.text, about.colour));
    else if (src.picture && about.src) {
      draw(null);
      const im = new Image();
      im.onload = () => { draw(im); if (redraw) redraw(); };
      im.src = about.src;
    } else draw(null);
    return c;
  }

  // a recipe's texture for a block's variant (or the sky, `sky` repeats round it)
  recipeTexture(about, variant, sky) {
    const r = about.recipe || {};
    const v = variant || 0;
    const turns = r.turn_variants ? v % 4 : 0;
    let tex = null;
    const canvas = this.recipeCanvas(about, turns, () => { if (tex) tex.needsUpdate = true; this.render(); });
    tex = new THREE.CanvasTexture(canvas);
    tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
    tex.colorSpace = THREE.SRGBColorSpace;
    tex.anisotropy = 4;
    const [sw, sh] = r.scale || [1, 1];
    // round the sky: four tiles a turn for a one-unit tile (fewer, bigger, for a bigger one)
    if (sky) tex.repeat.set(Math.max(1, Math.round(4 / sw)), Math.max(1, Math.round(2 / sh)));
    else tex.repeat.set(BLOCK_TILE / sw, BLOCK_TILE / sh);
    const [ou, ov] = r.offset || [0, 0];
    tex.offset.set(ou + v * 0.381966, ov + v * 0.618034);
    return tex;
  }

  recipeMaterial(key, about, variant) {
    const k = `${key}|${variant || 0}`;
    let m = this.mats.get(k);
    if (m) return m;
    if (!about.recipe) m = new THREE.MeshBasicMaterial({ vertexColors: true, color: new THREE.Color('#9a8f84') });
    else m = new THREE.MeshBasicMaterial({ vertexColors: true, map: this.recipeTexture(about, variant) });
    this.mats.set(k, m);
    return m;
  }

  // a block's material: its texture's (a recipe's for its variant), or one per face when
  // some faces show recipes of their own
  blockMaterial(b) {
    const key = `${b.kind}:${b.texture}`;
    const about = this.abouts.get(key) || {};
    const base = b.kind === 'texture' ? this.recipeMaterial(key, about, b.variant || 0) : this.material(key, about);
    const all = b.shape === 'sphere' || !b.faces || !b.faces.some(Boolean) ? base : b.faces.map((f) => (f ? this.recipeMaterial(`texture:${f}`, this.abouts.get(`texture:${f}`) || {}, 0) : base));
    if (!b.zone) return all;
    return Array.isArray(all) ? all.map((m) => this.seeThrough(m)) : this.seeThrough(all);
  }

  // a zone shows the texts in it: its material, see-through (made once per material)
  seeThrough(m) {
    if (!m.userData.glass) {
      const g = m.clone();
      g.transparent = true;
      g.opacity = 0.22;
      g.depthWrite = false;
      m.userData.glass = g;
    }
    return m.userData.glass;
  }

  // the sky: a recipe round the room, far off and following the camera (null: none)
  setSky(about) {
    if (this.sky) {
      this.scene.remove(this.sky);
      this.sky.geometry.dispose();
      this.sky.material.map?.dispose();
      this.sky.material.dispose();
      this.sky = null;
    }
    if (about && about.recipe) {
      const m = new THREE.MeshBasicMaterial({ map: this.recipeTexture(about, 0, true), side: THREE.BackSide, fog: false, depthWrite: false });
      this.sky = new THREE.Mesh(new THREE.SphereGeometry(1800, 48, 24), m);
      this.sky.renderOrder = -1000;
      this.sky.userData.sky = true;
      this.scene.add(this.sky);
    }
    this.render();
  }

  // draws a recipe's tiles into the canvas with that id on the page (the textures
  // window's previews): `tiles` across, each block variant in turn when `variants`
  preview(id, about, tiles, variants, tries = 0) {
    const el = document.getElementById(id);
    if (!el) {
      if (tries < 30) requestAnimationFrame(() => this.preview(id, about, tiles, variants, tries + 1));
      return;
    }
    const n = Math.max(1, tiles || 1);
    const W = el.width, H = el.height;
    const g = el.getContext('2d');
    const r = about.recipe || {};
    const drawAll = () => {
      g.clearRect(0, 0, W, H);
      // a tile keeps its shape (scale w by h)
      const [sw, sh] = r.scale || [1, 1];
      const cw0 = W / n, ch0 = cw0 * (sh / sw);
      const rows = Math.max(1, Math.ceil(H / ch0 - 1e-6));
      for (let y = 0; y < rows; y++) for (let x = 0; x < n; x++) {
        const v = variants ? (y * n + x) % Math.max(1, r.variants || 1) : 0;
        const tile = tiles_[v] || tiles_[0];
        const [ou, ov] = r.offset || [0, 0];
        // as the room samples it: the tile moved left by its u offset, down by its v
        const fu = (((ou + v * 0.381966) % 1) + 1) % 1, fv = (((ov + v * 0.618034) % 1) + 1) % 1;
        const cw = cw0, ch = ch0, x0 = x * cw, y0 = y * ch;
        g.save();
        g.beginPath();
        g.rect(x0, y0, cw, ch);
        g.clip();
        for (const i of [0, 1]) for (const j of [0, 1]) g.drawImage(tile, x0 - fu * cw + i * cw, y0 + fv * ch - j * ch, cw, ch);
        g.restore();
      }
    };
    const kinds = variants && r.turn_variants ? 4 : 1;
    const tiles_ = [];
    for (let k = 0; k < Math.max(kinds, variants ? Math.max(1, r.variants || 1) : 1); k++) {
      tiles_.push(this.recipeCanvas(about, r.turn_variants ? k % 4 : 0, drawAll, 128));
    }
    drawAll();
  }

  // a texture key's material, made once
  material(key, about) {
    if (key.startsWith('texture:')) return this.recipeMaterial(key, about, 0);
    let m = this.mats.get(key);
    if (m) return m;
    const [kind, ...rest] = key.split(':');
    const texture = rest.join(':');
    let map = null;
    if (kind === 'text') map = new THREE.CanvasTexture(this.textCanvas(about.gone ? '(gone)' : about.text, about.colour));
    else if (kind === 'picture' && about.src) {
      map = new THREE.TextureLoader().load(about.src, () => this.render());
    }
    if (map) {
      map.wrapS = map.wrapT = THREE.RepeatWrapping;
      map.colorSpace = THREE.SRGBColorSpace;
      map.anisotropy = 4;
    }
    // unlit, so a texture shows its own colours; the faces are shaded by vertex colours
    m = new THREE.MeshBasicMaterial({ vertexColors: true, ...(map ? { map } : { color: new THREE.Color(kind === 'plain' ? texture : '#b8a07a') }) });
    this.mats.set(key, m);
    return m;
  }

  // a box whose UVs count tiles: each face's u and v run over its sides in units. A bent
  // block (`bend`, core's `graph::blocks::Bend`) is cut into slices across its width
  // and each slice moved along its depth by `bendAt`, so its texture, picking and ray
  // hits all follow the bent shape exactly. Faces are shaded by their direction (the top
  // brightest, the bottom darkest), so a box and a curve both read.
  boxGeometry(t) {
    if (t.shape === 'sphere') return this.sphereGeometry(t);
    const bent = !!(t.bend && ((t.bend.ripple && t.bend.ripple.amount) || (t.bend.fold && t.bend.fold.amount)));
    const freq = bent && t.bend.ripple && t.bend.ripple.amount ? Math.max(1, t.bend.ripple.freq || 0) : 1;
    const seg = bent ? Math.min(512, Math.max(16, Math.ceil(freq * 24))) : 1;
    // faces cut into cells about a unit across (at most 16 a side), so baked light has
    // somewhere to land
    const cells = (v) => Math.min(16, Math.max(1, Math.ceil(v - 1e-6)));
    const g = new THREE.BoxGeometry(t.w, t.h, t.d, Math.max(seg, cells(t.w)), cells(t.h), cells(t.d));
    const uv = g.attributes.uv, pos = g.attributes.position, nor = g.attributes.normal;
    for (let i = 0; i < uv.count; i++) {
      const ax = Math.abs(nor.getX(i)), ay = Math.abs(nor.getY(i));
      const span = ax > 0.5 ? [t.d, t.h] : ay > 0.5 ? [t.w, t.d] : [t.w, t.h];
      uv.setXY(i, uv.getX(i) * span[0] / BLOCK_TILE, uv.getY(i) * span[1] / BLOCK_TILE);
    }
    uv.needsUpdate = true;
    if (bent) {
      for (let i = 0; i < pos.count; i++) {
        const u = (pos.getX(i) + t.w / 2) / t.w;
        pos.setZ(i, pos.getZ(i) + bendAt(t.bend, u));
      }
      pos.needsUpdate = true;
      g.computeVertexNormals();
    }
    const col = new Float32Array(pos.count * 3);
    for (let i = 0; i < pos.count; i++) {
      const v = 0.86 + 0.14 * nor.getY(i) + 0.08 * Math.abs(nor.getZ(i));
      col.set([v, v, v], i * 3);
    }
    g.setAttribute('color', new THREE.BufferAttribute(col, 3));
    g.userData.shape = shapeKey(t);
    return g;
  }

  // ------------------------------------------------------------------ animations
  //
  // A block playing an animation (core's graph::anims) is posed every frame: its move added
  // to its place, its turn after its own, its sides scaled, its texture switched from a
  // key on; the block itself (its place in the save, its gizmo, picking for editing) stays
  // where it was put. `animPose` follows core's `sample`.

  setAnims(list, textures) {
    for (const [k, v] of Object.entries(textures || {})) this.abouts.set(k, v);
    this.anims = new Map((list || []).map((x) => [x.key, x.anim]));
    this.animT0 = performance.now();
    for (const o of this.blocks.values()) this.unpose(o);
    this.dirty = true;
  }

  // play or pause every animation (paused: each block back where it was put)
  playAnims(on) {
    this.animPlaying = !!on;
    if (on) this.animT0 = performance.now();
    else for (const o of this.blocks.values()) this.unpose(o);
    this.dirty = true;
  }

  // ------------------------------------------------------------------ prefabs
  //
  // A prefab's copies are drawn as their parts, ordinary blocks whose ids name their copy
  // (core's graph::prefabs). The prefab menu shows each prefab as a small picture of its
  // parts, drawn here on a renderer of its own: `thumbs` answers id → a PNG data URL.
  thumbs(list, textures) {
    for (const [k, v] of Object.entries(textures || {})) this.abouts.set(k, v);
    const size = 112;
    if (!this.thumbR) {
      this.thumbR = new THREE.WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: true });
      this.thumbR.setPixelRatio(1);
      this.thumbR.setSize(size, size);
    }
    const out = {};
    for (const p of list || []) {
      const sc = new THREE.Scene();
      const box = new THREE.Box3();
      const made = [];
      for (const b of p.parts || []) {
        const mesh = new THREE.Mesh(this.boxGeometry(b), this.blockMaterial(b));
        mesh.position.set(b.x, b.y, b.z);
        const r = b.rot || [0, 0, 0, 1];
        mesh.quaternion.set(r[0], r[1], r[2], r[3]);
        const o = { b, mesh };
        this.bake(o);
        sc.add(mesh);
        made.push(mesh);
        box.expandByObject(mesh);
      }
      if (!made.length) continue;
      const c = box.getCenter(new THREE.Vector3());
      const r = Math.max(0.5, box.getSize(new THREE.Vector3()).length() / 2);
      const cam = new THREE.PerspectiveCamera(35, 1, 0.05, r * 20);
      cam.position.copy(c).add(new THREE.Vector3(0.9, 0.7, 1.4).normalize().multiplyScalar(r / Math.sin((35 / 2) * Math.PI / 180)));
      cam.lookAt(c);
      this.thumbR.setClearColor(0x000000, 0);
      this.thumbR.render(sc, cam);
      out[p.id] = this.thumbR.domElement.toDataURL('image/png');
      for (const m of made) m.geometry.dispose();
    }
    return out;
  }

  // ------------------------------------------------------------------ wiring
  //
  // Blocks that cast rays and run programs (core's `ops::wiring`). The room only knows
  // which blocks listen for what; core runs the programs and casts the rays. Playing
  // (`live`), a click on a block listening for clicks fires it instead of selecting it,
  // timers fire their blocks, and a zone listening for texts coming or going is told.
  // The rays a cause casts show for a moment.
  setWires(list, live) {
    this.wires = new Map((list || []).map((w) => [w.id, w]));
    // timers keep their beat across a reload of the list (a cause reloads it)
    const was = this.live;
    this.live = !!live;
    const old = (was && this.live && this.wireT) || new Map();
    this.wireT = new Map([...old].filter(([k]) => this.wires.has(k)));
    this.dirty = true;
    // the looping sound emitters play while the room plays
    if (this.live && !was) this.startLoops();
    if (!this.live && was) this.hushAll();
  }

  // ------------------------------------------------------------------ sounds
  //
  // Blocks can be sound emitters (core's graph::sounds): a sound of the save, a volume,
  // how far it carries (it fades to nothing there) and whether it loops. While the room
  // plays, looping emitters sound from the start and the others when a program prints
  // `sound`; each is placed where its block is and heard from the camera (Web Audio).
  // The room asks Rust for a sound's bytes the first time it needs them (`soundwant`).
  audio() {
    if (!this.ac) {
      const AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) return null;
      this.ac = new AC();
      this.buffers = new Map(); // sound id → AudioBuffer, or a Promise while decoding
      this.voices = new Map(); // block id → { src, gain, pan }
      this.waiting = new Map(); // sound id → block ids to start when it arrives
    }
    if (this.ac.state === 'suspended') this.ac.resume().catch(() => {});
    return this.ac;
  }

  // a sound's bytes from Rust: decoded, then the blocks waiting on it start
  soundData(id, data) {
    const ac = this.audio();
    if (!ac) return;
    const bin = atob(data);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const p = ac.decodeAudioData(bytes.buffer).then((buf) => {
      this.buffers.set(id, buf);
      const want = this.waiting.get(id) || [];
      this.waiting.delete(id);
      for (const w of want) w();
    }).catch((e) => {
      this.buffers.delete(id);
      this.send({ ...this.ev('toast', null), text: `that sound doesn't play here: ${e && e.message || e}` });
    });
    this.buffers.set(id, p);
  }

  // runs `then` with the sound's buffer, once it's here
  withSound(id, then) {
    const b = this.buffers.get(id);
    if (b instanceof AudioBuffer) return then(b);
    const q = this.waiting.get(id) || [];
    q.push(() => then(this.buffers.get(id)));
    this.waiting.set(id, q);
    if (!b) {
      this.buffers.set(id, 'asked');
      this.send({ ...this.ev('soundwant', null), data: id });
    }
  }

  // a block's sound starts (again, from the beginning)
  playSound(blockId) {
    const o = this.blocks.get(blockId);
    const s = o && o.b.sound;
    if (!s || !this.audio()) return;
    this.stopSound(blockId);
    this.withSound(s.id, (buf) => {
      if (!(buf instanceof AudioBuffer) || !this.blocks.has(blockId)) return;
      const ac = this.ac;
      const src = ac.createBufferSource();
      src.buffer = buf;
      src.loop = !!s.loop;
      const gain = ac.createGain();
      gain.gain.value = s.volume == null ? 1 : s.volume;
      const pan = ac.createPanner();
      pan.panningModel = 'equalpower';
      pan.distanceModel = 'linear';
      pan.refDistance = 1;
      pan.maxDistance = Math.max(1.01, s.range || 20);
      pan.rolloffFactor = 1;
      src.connect(gain).connect(pan).connect(ac.destination);
      const v = { src, gain, pan };
      this.voices.set(blockId, v);
      this.placeVoice(blockId, v);
      src.onended = () => { if (this.voices.get(blockId) === v) this.voices.delete(blockId); };
      src.start();
      this.listen();
    });
  }

  stopSound(blockId) {
    const v = this.voices && this.voices.get(blockId);
    if (!v) return;
    this.voices.delete(blockId);
    try { v.src.stop(); } catch { /* ended already */ }
  }

  startLoops() {
    for (const [id, o] of this.blocks) if (o.b.sound && o.b.sound.loop) this.playSound(id);
  }

  hushAll() {
    if (!this.voices) return;
    for (const id of [...this.voices.keys()]) this.stopSound(id);
  }

  // what programs asked for: `[{block, play}]`
  sounds(list) {
    for (const s of list || []) (s.play ? this.playSound(s.block) : this.stopSound(s.block));
  }

  // a sound heard as it is, not placed (the sounds window's ▶)
  previewSound(id) {
    if (!this.audio()) return;
    if (this.preview && this.preview.stop) { try { this.preview.stop(); } catch { /* done */ } }
    this.withSound(id, (buf) => {
      if (!(buf instanceof AudioBuffer)) return;
      const src = this.ac.createBufferSource();
      src.buffer = buf;
      src.connect(this.ac.destination);
      src.start();
      this.preview = src;
    });
  }

  placeVoice(id, v) {
    const o = this.blocks.get(id);
    if (!o) return;
    const p = o.mesh.position;
    const t = this.ac.currentTime;
    if (v.pan.positionX) {
      v.pan.positionX.setValueAtTime(p.x, t);
      v.pan.positionY.setValueAtTime(p.y, t);
      v.pan.positionZ.setValueAtTime(p.z, t);
    } else v.pan.setPosition(p.x, p.y, p.z);
  }

  // the listener is the camera; voices follow their blocks (an animation moves them)
  listen() {
    if (!this.ac || !this.voices || !this.voices.size) return;
    const l = this.ac.listener, c = this.camera, t = this.ac.currentTime;
    const f = new THREE.Vector3(0, 0, -1).applyQuaternion(c.quaternion);
    const u = new THREE.Vector3(0, 1, 0).applyQuaternion(c.quaternion);
    if (l.positionX) {
      l.positionX.setValueAtTime(c.position.x, t);
      l.positionY.setValueAtTime(c.position.y, t);
      l.positionZ.setValueAtTime(c.position.z, t);
      l.forwardX.setValueAtTime(f.x, t);
      l.forwardY.setValueAtTime(f.y, t);
      l.forwardZ.setValueAtTime(f.z, t);
      l.upX.setValueAtTime(u.x, t);
      l.upY.setValueAtTime(u.y, t);
      l.upZ.setValueAtTime(u.z, t);
    } else {
      l.setPosition(c.position.x, c.position.y, c.position.z);
      l.setOrientation(f.x, f.y, f.z, u.x, u.y, u.z);
    }
    for (const [id, v] of this.voices) this.placeVoice(id, v);
  }

  wireFire(id, event, from) {
    this.send({ ...this.ev('blockfire', null), data: { id, event, from: from || '' } });
  }

  // the timers of wired blocks, on the frame's clock (playing only)
  wireTimers(now) {
    if (!this.live || !this.wires || !this.wires.size) return false;
    let any = false;
    for (const [id, w] of this.wires) {
      if (!(w.on || []).includes('timer')) continue;
      any = true;
      const every = Math.max(0.2, w.every || 1) * 1000;
      const last = this.wireT.get(id);
      if (last == null) this.wireT.set(id, now);
      else if (now - last >= every) {
        this.wireT.set(id, now);
        this.wireFire(id, 'timer');
      }
    }
    return any;
  }

  // a text came into or left a zone: the zone's block is told, when it listens
  wireZone(zone, text, event) {
    if (!this.live || !zone || !this.wires) return;
    const w = this.wires.get(zone);
    if (w && (w.on || []).includes(event)) this.wireFire(zone, event, text);
  }

  // the rays a cause cast, drawn for a moment
  showRays(rays) {
    if (!rays || !rays.length) return;
    const pts = [];
    for (const r of rays) pts.push(new THREE.Vector3(...r.from), new THREE.Vector3(...r.to));
    const g = new THREE.BufferGeometry().setFromPoints(pts);
    const line = new THREE.LineSegments(g, new THREE.LineBasicMaterial({ color: 0xffcc33, transparent: true, opacity: 0.95, depthTest: false }));
    line.renderOrder = 10;
    this.scene.add(line);
    this.rayLines = (this.rayLines || []).concat([{ line, t0: performance.now() }]);
    this.dirty = true;
  }

  // fades the rays out over 0.8 s; whether any are left
  fadeRays(now) {
    if (!this.rayLines || !this.rayLines.length) return false;
    this.rayLines = this.rayLines.filter((r) => {
      const a = 1 - (now - r.t0) / 800;
      if (a <= 0) {
        this.scene.remove(r.line);
        r.line.geometry.dispose();
        r.line.material.dispose();
        return false;
      }
      r.line.material.opacity = 0.95 * a;
      return true;
    });
    return true;
  }

  // the animations window shows one on a block: at a phase (scrubbing), or playing
  previewAnim(id, anim, p) {
    const o = this.blocks.get(id);
    if (!o) return;
    o.preview = anim ? { anim, p: p == null ? null : p, t0: performance.now() } : null;
    if (!anim) this.unpose(o);
    this.dirty = true;
  }

  animating() {
    for (const o of this.blocks.values()) if (o.preview || (this.animPlaying && o.b.anim && this.anims.has(o.b.anim.key))) return true;
    return false;
  }

  unpose(o) {
    o.mesh.scale.set(1, 1, 1);
    o.mesh.position.set(o.b.x, o.b.y, o.b.z);
    o.mesh.quaternion.copy(turnOf(o.b));
    if (o.animTex) {
      o.animTex = null;
      o.mesh.material = this.blockMaterial(o.b);
    }
  }

  // where in an animation (0 to 1) `t` seconds after it started
  animPhase(a, t) {
    const x = Math.max(0, t) / Math.max(0.05, a.duration || 2);
    if (a.repeat === 'loop') return x % 1;
    if (a.repeat === 'alternate') { const f = x % 2; return f <= 1 ? f : 2 - f; }
    return Math.min(1, x);
  }

  animPose(a, p) {
    const keys = [...(a.keys || [])].sort((x, y) => x.at - y.at);
    const timing = a.timing || [0.25, 0.1, 0.25, 1];
    const field = (name, rest) => {
      const set = keys.filter((k) => k[name]);
      if (!set.length) return rest;
      const f = set[0];
      if (p <= f.at) {
        if (f.at <= 0) return f[name];
        const u = cubicBezier(timing, p / f.at);
        return rest.map((r, i) => r + (f[name][i] - r) * u);
      }
      for (let i = 0; i + 1 < set.length; i++) {
        const k0 = set[i], k1 = set[i + 1];
        if (p <= k1.at) {
          const u = cubicBezier(k0.timing || timing, (p - k0.at) / Math.max(1e-6, k1.at - k0.at));
          return k0[name].map((v, j) => v + (k1[name][j] - v) * u);
        }
      }
      return set[set.length - 1][name];
    };
    let texture = null;
    for (const k of keys) if (k.at <= p && k.texture) texture = k.texture;
    return { move: field('move', [0, 0, 0]), turn: field('turn', [0, 0, 0]), scale: field('scale', [1, 1, 1]), texture };
  }

  // every playing block posed for this moment
  applyAnims(now) {
    const e = new THREE.Euler(), q = new THREE.Quaternion();
    for (const o of this.blocks.values()) {
      let a = null, p = 0;
      if (o.preview) {
        a = o.preview.anim;
        p = o.preview.p != null ? o.preview.p : this.animPhase(a, (now - o.preview.t0) / 1000);
      } else if (this.animPlaying && o.b.anim && this.anims.has(o.b.anim.key)) {
        a = this.anims.get(o.b.anim.key);
        const t = ((now - this.animT0) / 1000 - (o.b.anim.delay || 0)) * (o.b.anim.speed || 1);
        p = this.animPhase(a, t);
      }
      if (!a) continue;
      const pose = this.animPose(a, p);
      const b = o.b;
      o.mesh.position.set(b.x + pose.move[0], b.y + pose.move[1], b.z + pose.move[2]);
      e.set((pose.turn[0] * Math.PI) / 180, (pose.turn[1] * Math.PI) / 180, (pose.turn[2] * Math.PI) / 180, 'XYZ');
      o.mesh.quaternion.copy(turnOf(b)).multiply(q.setFromEuler(e));
      o.mesh.scale.set(pose.scale[0], pose.scale[1], pose.scale[2]);
      if ((pose.texture || null) !== (o.animTex || null)) {
        o.animTex = pose.texture;
        const m = pose.texture ? this.material(pose.texture, this.abouts.get(pose.texture) || {}) : this.blockMaterial(b);
        o.mesh.material = b.zone ? (Array.isArray(m) ? m.map((x) => this.seeThrough(x)) : this.seeThrough(m)) : m;
      }
    }
  }

  // ------------------------------------------------------------------ lights
  //
  // The room's lights are baked: each block's vertex colours are the light reaching them
  // (ambient, directional, point and cone lights summed; points and cones fade to nothing
  // at their range, a cone's edge softened), worked out when a light or the block changes
  // and not every frame. No shadows. With no lights, faces are shaded by their direction
  // as before. Texts (html labels) aren't lit.

  // a block's vertex colours from the lights
  bake(o) {
    const g = o.mesh.geometry;
    const pos = g.attributes.position, nor = g.attributes.normal;
    let col = g.attributes.color;
    if (!col || col.count !== pos.count) {
      col = new THREE.BufferAttribute(new Float32Array(pos.count * 3), 3);
      g.setAttribute('color', col);
    }
    const lights = this.lights || [];
    if (!lights.length) {
      const round = o.b.shape === 'sphere';
      for (let i = 0; i < pos.count; i++) {
        const v = round ? 0.82 + 0.18 * nor.getY(i) + 0.06 * Math.abs(nor.getZ(i)) : 0.86 + 0.14 * nor.getY(i) + 0.08 * Math.abs(nor.getZ(i));
        col.setXYZ(i, v, v, v);
      }
      col.needsUpdate = true;
      return;
    }
    o.mesh.updateMatrixWorld(true);
    const m = o.mesh.matrixWorld, q = o.mesh.quaternion;
    const p = new THREE.Vector3(), n = new THREE.Vector3(), l = new THREE.Vector3();
    const ls = lights.map((x) => ({ ...x, c: new THREE.Color(x.colour || '#ffffff'), d: new THREE.Vector3(...(x.dir || [0, -1, 0])).normalize(), at: new THREE.Vector3(x.x || 0, x.y || 0, x.z || 0) }));
    for (let i = 0; i < pos.count; i++) {
      p.fromBufferAttribute(pos, i).applyMatrix4(m);
      n.fromBufferAttribute(nor, i).applyQuaternion(q).normalize();
      let r = 0, gg = 0, bb = 0;
      for (const x of ls) {
        let f = 0;
        if (x.kind === 'ambient') f = x.intensity;
        else if (x.kind === 'directional') f = x.intensity * Math.max(0, -n.dot(x.d));
        else {
          l.copy(x.at).sub(p);
          const dist = l.length();
          if (dist > x.range) continue;
          l.divideScalar(dist || 1);
          const fade = (1 - dist / x.range) ** 2;
          f = x.intensity * Math.max(0, n.dot(l)) * fade;
          if (x.kind === 'spot') {
            const cos = -l.dot(x.d);
            const outer = Math.cos((x.angle * Math.PI) / 180), inner = Math.cos((x.angle * (1 - x.softness) * Math.PI) / 180);
            const t = inner > outer ? Math.min(1, Math.max(0, (cos - outer) / (inner - outer))) : cos >= outer ? 1 : 0;
            f *= t * t * (3 - 2 * t);
          }
        }
        r += x.c.r * f; gg += x.c.g * f; bb += x.c.b * f;
      }
      col.setXYZ(i, Math.min(r, 2), Math.min(gg, 2), Math.min(bb, 2));
    }
    col.needsUpdate = true;
  }

  // the lights from Rust; `changed` (ids, with where they were) bakes only the blocks they
  // reach, else every block
  setLights(list, changed) {
    const before = new Map((this.lights || []).map((x) => [x.id, x]));
    this.lights = list || [];
    const now = new Map(this.lights.map((x) => [x.id, x]));
    let all = !changed || (before.size === 0) !== (now.size === 0);
    const reach = [];
    for (const id of changed || []) {
      for (const x of [before.get(id), now.get(id)]) {
        if (!x) continue;
        if (x.kind === 'ambient' || x.kind === 'directional') all = true;
        else reach.push(x);
      }
    }
    for (const o of this.blocks.values()) {
      if (all) { this.bake(o); continue; }
      const b = o.b, half = Math.hypot(b.w, b.h, b.d) / 2;
      if (reach.some((x) => Math.hypot(b.x - x.x, b.y - x.y, b.z - x.z) <= x.range + half)) this.bake(o);
    }
    this.markLights();
    this.render();
  }

  // the lights window shows where the point and cone lights are, and which is picked
  showLights(on, picked) {
    this.lightsShown = !!on;
    this.lightPicked = picked == null ? null : picked;
    this.markLights();
    this.render();
  }

  markLights() {
    if (this.lightMarks) {
      this.scene.remove(this.lightMarks);
      this.lightMarks.traverse((x) => { x.geometry?.dispose(); x.material?.dispose(); });
      this.lightMarks = null;
    }
    if (!this.lightsShown) return;
    const g = new THREE.Group();
    for (const x of this.lights) {
      if (x.kind !== 'point' && x.kind !== 'spot') continue;
      const picked = x.id === this.lightPicked;
      const mat = new THREE.MeshBasicMaterial({ color: x.colour, fog: false, depthTest: false, transparent: true, opacity: picked ? 1 : 0.8 });
      const ball = new THREE.Mesh(new THREE.SphereGeometry(picked ? 0.22 : 0.15, 16, 12), mat);
      ball.position.set(x.x, x.y, x.z);
      ball.renderOrder = 998;
      g.add(ball);
      const ring = new THREE.Mesh(new THREE.TorusGeometry(picked ? 0.34 : 0.26, 0.02, 6, 32), new THREE.MeshBasicMaterial({ color: picked ? 0xffffff : 0x333333, fog: false, depthTest: false, transparent: true }));
      ring.position.copy(ball.position);
      ring.userData.facing = true;
      ring.renderOrder = 998;
      g.add(ring);
      if (x.kind === 'spot') {
        // the cone's axis, as long as its reach (up to 3 units)
        const d = new THREE.Vector3(...x.dir).normalize();
        const len = Math.min(2, x.range);
        const cone = new THREE.Mesh(new THREE.ConeGeometry(Math.tan((x.angle * Math.PI) / 180) * len, len, 24, 1, true), new THREE.MeshBasicMaterial({ color: x.colour, fog: false, transparent: true, opacity: 0.08, side: THREE.DoubleSide, depthWrite: false }));
        cone.quaternion.setFromUnitVectors(new THREE.Vector3(0, -1, 0), d);
        cone.position.copy(ball.position).add(d.clone().multiplyScalar(len / 2));
        g.add(cone);
      }
    }
    this.lightMarks = g;
    this.scene.add(g);
  }

  // where a new light goes: a little ahead of the camera
  lightSpot() {
    const at = new THREE.Vector3(0, 0, -4).applyQuaternion(this.camera.quaternion).add(this.camera.position);
    const d = new THREE.Vector3(0, 0, -1).applyQuaternion(this.camera.quaternion);
    return { x: +at.x.toFixed(2), y: +at.y.toFixed(2), z: +at.z.toFixed(2), dir: [+d.x.toFixed(3), +d.y.toFixed(3), +d.z.toFixed(3)] };
  }

  // a sphere block: an ellipsoid with the block's sides as its diameters. Its texture
  // tiles once per unit round its middle and from pole to pole, as a box's does over its
  // faces; it neither bends nor has faces of its own (core's graph::blocks::Shape)
  sphereGeometry(t) {
    const g = new THREE.SphereGeometry(0.5, 48, 32);
    const pos = g.attributes.position, uv = g.attributes.uv;
    for (let i = 0; i < pos.count; i++) pos.setXYZ(i, pos.getX(i) * t.w, pos.getY(i) * t.h, pos.getZ(i) * t.d);
    const round = (Math.PI * (t.w + t.d)) / 2, meridian = (Math.PI * t.h) / 2;
    for (let i = 0; i < uv.count; i++) uv.setXY(i, (uv.getX(i) * round) / BLOCK_TILE, (uv.getY(i) * meridian) / BLOCK_TILE);
    g.computeVertexNormals();
    const nor = g.attributes.normal;
    const col = new Float32Array(pos.count * 3);
    for (let i = 0; i < pos.count; i++) {
      const v = 0.82 + 0.18 * nor.getY(i) + 0.06 * Math.abs(nor.getZ(i));
      col.set([v, v, v], i * 3);
    }
    g.setAttribute('color', new THREE.BufferAttribute(col, 3));
    g.userData.shape = shapeKey(t);
    return g;
  }

  // blocks from Rust: [{id, kind, texture, x, y, z, w, h, d}] and their textures by key
  setBlocks(list, textures, replace) {
    if (replace) {
      for (const id of [...this.blocks.keys()]) this.dropBlock(id);
    }
    for (const [k, v] of Object.entries(textures || {})) this.abouts.set(k, v);
    for (const b of list || []) {
      const key = `${b.kind}:${b.texture}`;
      const about = this.abouts.get(key) || {};
      const had = this.blocks.get(b.id);
      if (had) {
        had.b = b;
        had.animTex = null;
        had.mesh.material = this.blockMaterial(b);
        this.shapeBlock(had);
        continue;
      }
      const mesh = new THREE.Mesh(this.boxGeometry(b), this.blockMaterial(b));
      mesh.userData.block = b.id;
      const o = { b, mesh, key, about };
      this.blocks.set(b.id, o);
      this.shapeBlock(o);
      this.scene.add(mesh);
    }
    this.render();
  }

  shapeBlock(o) {
    const { b, mesh } = o;
    // a zone's place or size changed: its texts are looked at again (and come along)
    if (b.zone || o.wasZone) {
      this.zoneBlocks = null;
      this.zonesDirty = true;
      this.wake(1);
    }
    if (!!b.zone !== !!o.wasZone) mesh.material = this.blockMaterial(b);
    o.wasZone = !!b.zone;
    if (mesh.geometry.userData.shape !== shapeKey(b)) {
      mesh.geometry.dispose();
      mesh.geometry = this.boxGeometry(b);
      // a sphere has one face: one material, whatever faces the box had
      mesh.material = this.blockMaterial(b);
    }
    mesh.position.set(b.x, b.y, b.z);
    const r = b.rot || [0, 0, 0, 1];
    mesh.quaternion.set(r[0], r[1], r[2], r[3]);
    if (!o.preview && !b.anim) mesh.scale.set(1, 1, 1);
    this.bake(o);
    if (this.selBlock === b.id) this.placeGizmo();
  }

  dropBlock(id) {
    const o = this.blocks.get(id);
    if (!o) return;
    this.stopSound(id);
    if (o.b.zone) {
      this.zoneBlocks = null;
      this.zonesDirty = true;
      this.wake(1);
    }
    this.scene.remove(o.mesh);
    o.mesh.geometry.dispose();
    this.blocks.delete(id);
    if (this.selBlock === id) this.pickBlock(null);
  }

  blockOut(o) {
    const { id, kind, texture, x, y, z, w, h, d, rot, bend, faces, variant, shape, zone, anim, part_of, sound } = o.b;
    return { id, kind, texture, x, y, z, w, h, d, rot: rot || [0, 0, 0, 1], bend: bend || null, faces: faces || null, shape: shape || 'box', zone: zone || null, anim: anim || null, ...(variant != null ? { variant } : {}), ...(part_of ? { part_of } : {}), ...(sound ? { sound } : {}) };
  }

  pickBlock(id) {
    this.selBlock = id;
    if (id != null) {
      if (this.selected) this.pick(null);
      this.placeGizmo();
      this.gizmo.visible = true;
    } else {
      this.gizmo.visible = false;
      for (const k of this.gizmoKnobs) k.visible = false;
    }
    const o = id != null ? this.blocks.get(id) : null;
    this.send({ ...this.ev('block', null), data: o ? { ...this.blockOut(o), about: o.about } : null });
    this.render();
  }

  // where a new block goes: ahead of the camera, or at the selected text
  blockSpot() {
    let at;
    if (this.selected) at = this.world(this.selected);
    else at = new THREE.Vector3(0, 0, -8).applyQuaternion(this.camera.quaternion).add(this.camera.position);
    return { x: +at.x.toFixed(2), y: +at.y.toFixed(2), z: +at.z.toFixed(2) };
  }

  makeGizmo() {
    const g = new THREE.Group();
    const axes = [new THREE.Vector3(1, 0, 0), new THREE.Vector3(0, 1, 0), new THREE.Vector3(0, 0, 1)];
    const colours = [0xd0453a, 0x3f9b4b, 0x3a6fd0];
    const mat = (c) => new THREE.MeshBasicMaterial({ color: c, fog: false, depthTest: false, depthWrite: false, transparent: true, opacity: 0.95 });
    const up = new THREE.Vector3(0, 1, 0);
    this.gizmoKnobs = [];
    axes.forEach((a, i) => {
      const q = new THREE.Quaternion().setFromUnitVectors(up, a);
      const shaft = new THREE.Mesh(new THREE.CylinderGeometry(0.025, 0.025, 0.8, 8), mat(colours[i]));
      shaft.position.copy(a.clone().multiplyScalar(0.4));
      shaft.quaternion.copy(q);
      const tip = new THREE.Mesh(new THREE.ConeGeometry(0.08, 0.22, 14), mat(colours[i]));
      tip.position.copy(a.clone().multiplyScalar(0.9));
      tip.quaternion.copy(q);
      // a fatter invisible grip, easier to hit
      const grip = new THREE.Mesh(new THREE.CylinderGeometry(0.1, 0.1, 1.05, 8), new THREE.MeshBasicMaterial({ visible: false }));
      grip.position.copy(a.clone().multiplyScalar(0.52));
      grip.quaternion.copy(q);
      for (const m of [shaft, tip, grip]) { m.userData.handle = { mode: 'move', axis: i }; m.renderOrder = 1000; g.add(m); }
      const knob = new THREE.Mesh(new THREE.BoxGeometry(0.16, 0.16, 0.16), mat(colours[i]));
      knob.userData.handle = { mode: 'scale', axis: i };
      knob.renderOrder = 1001;
      this.gizmoKnobs.push(knob);
      this.scene.add(knob);
    });
    // the rotation rings: one about each axis, dragged around the block's centre
    const flat = new THREE.Vector3(0, 0, 1);
    axes.forEach((a, i) => {
      const q = new THREE.Quaternion().setFromUnitVectors(flat, a);
      const ring = new THREE.Mesh(new THREE.TorusGeometry(1.25, 0.018, 8, 72), mat(colours[i]));
      ring.material.opacity = 0.75;
      ring.quaternion.copy(q);
      const grip = new THREE.Mesh(new THREE.TorusGeometry(1.25, 0.09, 6, 48), new THREE.MeshBasicMaterial({ visible: false }));
      grip.quaternion.copy(q);
      for (const m of [ring, grip]) { m.userData.handle = { mode: 'rotate', axis: i }; m.renderOrder = 999; g.add(m); }
    });
    const ball = new THREE.Mesh(new THREE.SphereGeometry(0.1, 16, 12), mat(0xf0e6d0));
    ball.userData.handle = { mode: 'plane', axis: -1 };
    ball.renderOrder = 1002;
    g.add(ball);
    g.visible = false;
    for (const k of this.gizmoKnobs) k.visible = false;
    this.scene.add(g);
    return g;
  }

  // the gizmo at the selected block, the same size on screen wherever it is
  placeGizmo() {
    const o = this.blocks.get(this.selBlock);
    const on = !!o;
    this.gizmo.visible = on;
    for (const k of this.gizmoKnobs) k.visible = on;
    if (!o) return;
    const b = o.b;
    const c = new THREE.Vector3(b.x, b.y, b.z);
    const s = Math.max(0.05, c.distanceTo(this.camera.position) * 0.11);
    this.gizmo.position.copy(c);
    this.gizmo.scale.setScalar(s);
    const half = [b.w / 2, b.h / 2, b.d / 2];
    const q = turnOf(b);
    this.gizmoKnobs.forEach((k, i) => {
      // on the block's own faces, turned with it
      k.position.copy(c).add(localAxis(i).applyQuaternion(q).multiplyScalar(half[i]));
      k.quaternion.copy(q);
      k.scale.setScalar(s);
    });
  }

  rayFrom(e) {
    const r = this.host.getBoundingClientRect();
    const ndc = new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
    this.ray.setFromCamera(ndc, this.camera);
    return this.ray;
  }

  // the gizmo handle or block under the pointer
  blockHit(e) {
    const ray = this.rayFrom(e);
    if (this.selBlock != null && this.gizmo.visible) {
      const h = ray.intersectObjects([...this.gizmo.children, ...this.gizmoKnobs], false)[0];
      if (h) return { handle: h.object.userData.handle };
    }
    const meshes = [...this.blocks.values()].map((o) => o.mesh);
    const h = ray.intersectObjects(meshes, false)[0];
    return h ? { block: h.object.userData.block, point: h.point } : null;
  }

  // pixels on screen per unit along a world axis at a point
  screenAxis(at, axis, along) {
    const a = along || localAxis(axis);
    return this.toPx(at.clone().add(a)).sub(this.toPx(at));
  }

  toPx(v) {
    const p = v.clone().project(this.camera);
    return new THREE.Vector2((p.x * 0.5 + 0.5) * this.size.w, (-p.y * 0.5 + 0.5) * this.size.h);
  }

  startBlockDrag(e, hit) {
    const o = this.blocks.get(this.selBlock);
    if (!o) return false;
    const b = o.b;
    const at = new THREE.Vector3(b.x, b.y, b.z);
    const fwd = new THREE.Vector3(0, 0, -1).applyQuaternion(this.camera.quaternion);
    const depth = at.clone().sub(this.camera.position).dot(fwd);
    const mode = hit.handle ? hit.handle.mode : 'plane', axis = hit.handle ? hit.handle.axis : -1;
    // a side's knob pulls along the block's own (turned) axis
    const along = mode === 'scale' ? localAxis(axis).applyQuaternion(turnOf(b)) : null;
    const c = this.toPx(at);
    this.bdrag = {
      o, mode, axis, along,
      x0: e.clientX, y0: e.clientY, from: { ...b, rot: [...(b.rot || [0, 0, 0, 1])] }, depth, moved: false,
      sdir: axis >= 0 && mode !== 'rotate' ? this.screenAxis(at, axis, along) : null,
      off: at.clone().sub(this.rayAt(e.clientX, e.clientY, depth)),
      centre: c, a0: Math.atan2(e.clientY - c.y, e.clientX - c.x),
      // turning about an axis that points at the viewer turns the other way on screen
      toward: localAxis(Math.max(0, axis)).dot(this.camera.position.clone().sub(at)) > 0,
    };
    try { this.host.setPointerCapture(e.pointerId); } catch { /* fine without */ }
    return true;
  }

  dragBlock(e) {
    const d = this.bdrag;
    if (!d) return;
    if (!d.moved && Math.hypot(e.clientX - d.x0, e.clientY - d.y0) < 3) return;
    d.moved = true;
    const snap = (v) => (e.shiftKey ? Math.round(v * 4) / 4 : v);
    const b = d.o.b;
    const f = d.from;
    const keys = ['x', 'y', 'z'];
    const sides = ['w', 'h', 'd'];
    if (d.mode === 'plane') {
      const w = this.rayAt(e.clientX, e.clientY, d.depth).add(d.off);
      b.x = snap(w.x); b.y = snap(w.y); b.z = snap(w.z);
    } else if (d.mode === 'rotate') {
      let ang = Math.atan2(e.clientY - d.centre.y, e.clientX - d.centre.x) - d.a0;
      if (d.toward) ang = -ang;
      // Shift: steps of 15°
      if (e.shiftKey) ang = Math.round(ang / (Math.PI / 12)) * (Math.PI / 12);
      const q = new THREE.Quaternion().setFromAxisAngle(localAxis(d.axis), ang).multiply(turnOf(d.from)).normalize();
      b.rot = [q.x, q.y, q.z, q.w];
    } else {
      const m = new THREE.Vector2(e.clientX - d.x0, e.clientY - d.y0);
      const len2 = Math.max(1e-6, d.sdir.lengthSq());
      const units = m.dot(d.sdir) / len2;
      const k = keys[d.axis];
      if (d.mode === 'move') b[k] = snap(f[k] + units);
      else {
        const s = sides[d.axis];
        const size = Math.max(0.05, snap(f[s] + units));
        b[s] = size;
        // the opposite face stays put, along the block's own axis
        const shift = (size - f[s]) / 2;
        b.x = f.x + d.along.x * shift; b.y = f.y + d.along.y * shift; b.z = f.z + d.along.z * shift;
      }
    }
    this.shapeBlock(d.o);
    this.render();
  }

  endBlockDrag() {
    const d = this.bdrag;
    this.bdrag = null;
    if (!d || !d.moved) return;
    this.record({ kind: 'block', id: d.o.b.id, t: d.from });
    this.send({ ...this.ev('blockset', null), data: this.blockOut(d.o) });
    this.pickBlock(d.o.b.id);
  }

  // blocks Rust deleted: out of the room, kept for an undo
  unblock(ids) {
    const items = ids.map((id) => this.blocks.get(id)).filter(Boolean).map((o) => ({ b: this.blockOut(o), key: o.key, about: o.about }));
    if (!items.length) return;
    this.record({ kind: 'unblock', items });
    for (const id of ids) this.dropBlock(id);
    this.render();
  }

  // a change from the window's fields
  setBlockFields(id, t) {
    const o = this.blocks.get(id);
    if (!o) return;
    this.record({ kind: 'block', id, t: { ...o.b } });
    Object.assign(o.b, t);
    this.shapeBlock(o);
    this.send({ ...this.ev('blockset', null), data: this.blockOut(o) });
    this.render();
  }

  // ------------------------------------------------------------------ input

  // the placements of one entry in the room (a copy is another placement of the same
  // entry, never another entry)
  placements(entry) {
    return this.order.filter((x) => !x.free && !x.head && String(x.f.key) === String(entry));
  }

  ev(kind, n, e, extra) {
    const same = n && !n.free ? this.placements(n.f.key) : [];
    return {
      kind, key: n ? (n.free ? n.key : String(n.f.key)) : '', node: n ? n.key : '',
      copies: same.length, primary: !!(n && !n.free && same[0] === n), text: n ? (n.own != null ? n.own : n.f.text) : '', note: !!(n && n.f.note), free: !!(n && n.free),
      det: n ? n.det || 0 : 0,
      chain: n ? (n.head ? String(n.head.f.key) : n.f.isChain ? String(n.f.key) : '') : '', idx: n && n.head ? n.idx : -1,
      members: n && n.links ? n.links.length : 0, virt: !!(n && n.f.virt), head_node: n && n.head ? n.head.key : '', font: n && n.size ? Math.round(n.size * 50) : 0, font_default: Math.round(this.p.size * 50), own: n ? n.own : null, entry_text: n ? n.f.text : '',
      x: e ? e.clientX : 0, y: e ? e.clientY : 0, ctrl: !!(e && (e.ctrlKey || e.metaKey)), shift: !!(e && e.shiftKey), alt: !!(e && e.altKey),
      q: n ? n.f.q || 0 : 0, hue: n ? n.f.hue || 0 : 0, pinned: !!(n && n.pinned), data: extra || '',
    };
  }

  wire(n) {
    const { el } = n;
    el.addEventListener('pointerenter', (e) => {
      if (this.looking || this.drag) return;
      this.hover = n;
      el.classList.add('hover');
      this.send(this.ev('hover', n, e));
    });
    el.addEventListener('pointerleave', () => {
      el.classList.remove('hover');
      if (this.hover === n) this.hover = null;
    });
    el.addEventListener('pointerdown', (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      try { el.setPointerCapture(e.pointerId); } catch { /* fine without */ }
      // a chain's member drags its head (the chain comes along)
      const t = n.head || n;
      const w = this.world(t);
      const fwd = new THREE.Vector3(0, 0, -1).applyQuaternion(this.camera.quaternion);
      this.drag = { n: t, el: n, x0: e.clientX, y0: e.clientY, moved: false, depth: this.world(n).sub(this.camera.position).dot(fwd), from: { x: t.x, y: t.y, z: t.z, pinned: t.pinned }, off: null };
      // where the pointer is on the text, so it doesn't jump to its centre
      const hit = this.rayAt(e.clientX, e.clientY, this.drag.depth);
      this.drag.off = w.clone().sub(hit);
    });
    el.addEventListener('pointermove', (e) => {
      const d = this.drag;
      if (!d || d.el !== n) return;
      if (!d.moved && Math.hypot(e.clientX - d.x0, e.clientY - d.y0) < 4) return;
      d.moved = true;
      const t = d.n;
      t.pinned = true;
      t.el.classList.add('pinned', 'dragging');
      const w = this.rayAt(e.clientX, e.clientY, d.depth).add(d.off);
      this.local(w, t);
      this.wake(0.3);
    });
    const up = (e) => {
      const d = this.drag;
      if (!d || d.el !== n) return;
      this.drag = null;
      d.n.el.classList.remove('dragging');
      if (d.moved) {
        // dropped into a zone (or out of one)
        d.n.zone = this.zoneOf(d.n, this.zoneFrames());
        this.sim = null;
        this.record({ kind: 'move', n: d.n, ...d.from });
        this.send(this.ev('moved', d.n, e));
      } else {
        this.pick(n);
        const now = performance.now();
        if (this.lastClick && this.lastClick.n === n && now - this.lastClick.t < 450) {
          this.lastClick = null;
          this.send(this.ev('dblclick', n, e));
        } else {
          this.lastClick = { n, t: now };
          this.send(this.ev('click', n, e));
        }
      }
    };
    el.addEventListener('pointerup', up);
    el.addEventListener('lostpointercapture', up);
    el.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      this.send(this.ev('menu', n, e));
    });
    el.addEventListener('wheel', (e) => {
      e.preventDefault();
      e.stopPropagation();
      // push away or pull near, along the line from the eye
      const k = Math.exp(-(e.deltaMode === 1 ? e.deltaY * 33 : e.deltaY) * 0.0015);
      const w = this.world(n);
      const from = w.clone().sub(this.camera.position);
      const len = Math.min(2000, Math.max(0.3, from.length() * k));
      const now = performance.now();
      if (!(this.wheelUndo && this.wheelUndo.n === n && now - this.wheelUndo.t < 800)) this.record({ kind: 'move', n, x: n.x, y: n.y, z: n.z, pinned: n.pinned });
      this.wheelUndo = { n, t: now };
      this.local(this.camera.position.clone().add(from.setLength(len)), n);
      n.pinned = true;
      n.el.classList.add('pinned');
      this.wake(0.3);
    }, { passive: false });
  }

  // the point under the pointer at a depth in front of the camera
  rayAt(cx, cy, depth) {
    const r = this.host.getBoundingClientRect();
    const ndc = new THREE.Vector3(((cx - r.left) / r.width) * 2 - 1, -((cy - r.top) / r.height) * 2 + 1, 0.5);
    const dir = ndc.unproject(this.camera).sub(this.camera.position).normalize();
    const fwd = new THREE.Vector3(0, 0, -1).applyQuaternion(this.camera.quaternion);
    const t = depth / Math.max(1e-4, dir.dot(fwd));
    return this.camera.position.clone().add(dir.multiplyScalar(t));
  }

  applyTurn() {
    this.camera.quaternion.setFromEuler(new THREE.Euler(this.pitch, this.yaw, this.rollA, 'YXZ'));
  }

  // yaw, pitch and roll from where an ease left the camera
  syncTurn() {
    const e = new THREE.Euler().setFromQuaternion(this.camera.quaternion, 'YXZ');
    this.yaw = this.tyaw = e.y;
    this.pitch = this.tpitch = Math.max(-1.55, Math.min(1.55, e.x));
    this.rollA = e.z;
    this.applyTurn();
  }

  home() {
    this.camera.position.set(0, 0, 0);
    this.yaw = this.tyaw = this.pitch = this.tpitch = this.rollA = 0;
    this.ease = null;
    this.applyTurn();
    this.render();
  }

  level() {
    this.rollA = 0;
    this.applyTurn();
    this.render();
  }

  roll() {
    return (this.rollA * 180) / Math.PI;
  }

  // the arrow keys: the nearest text in that direction on screen, eased onto
  hop(dir) {
    const cam = this.camera;
    cam.updateMatrixWorld();
    let best = null;
    let bestScore = Infinity;
    const want = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, 1], ArrowDown: [0, -1] }[dir];
    const w = new THREE.Vector3();
    for (const n of this.order) {
      const local = this.world(n, w).applyMatrix4(cam.matrixWorldInverse);
      const dist = local.length();
      if (dist < 0.5) continue;
      const sx = local.x / dist;
      const sy = local.y / dist;
      const along = sx * want[0] + sy * want[1];
      if (along <= 0.02) continue;
      const across = Math.abs(sx * want[1] - sy * want[0]);
      const score = (Math.acos(Math.min(1, along)) + across * 2) * 10 + dist * 0.05 + (local.z > 0 ? 50 : 0);
      if (score < bestScore) {
        bestScore = score;
        best = n;
      }
    }
    if (!best) return;
    const target = this.world(best);
    const fwd = target.clone().sub(cam.position).normalize();
    const stand = Math.max(4, Math.min(14, best.f.text.length * 0.35));
    const toPos = target.clone().sub(fwd.clone().multiplyScalar(stand));
    const look = new THREE.Object3D();
    look.position.copy(toPos);
    look.up.copy(new THREE.Vector3(0, 1, 0).applyQuaternion(cam.quaternion));
    look.lookAt(target);
    // a camera looks down -z, an Object3D's lookAt points +z: turn it round
    look.rotateY(Math.PI);
    this.ease = { fromPos: cam.position.clone(), toPos, fromQuat: cam.quaternion.clone(), toQuat: look.quaternion.clone(), t: 0 };
    this.pick(best);
    this.send(this.ev('focus', best));
  }

  listen() {
    const h = this.host;
    h.addEventListener('contextmenu', (e) => e.preventDefault());
    h.addEventListener('pointerdown', (e) => {
      if (e.target.closest('.slabel')) return;
      if (e.button === 0) {
        // a block's gizmo, or a block: select it (and drag it in the view's plane)
        // playing, a click on a block listening for clicks fires it (through the gizmo)
        if (this.live && this.wires && this.wires.size) {
          const m = this.rayFrom(e).intersectObjects([...this.blocks.values()].map((o) => o.mesh), false)[0];
          const id = m && m.object.userData.block;
          const w = id != null && this.wires.get(id);
          if (w && (w.on || []).includes('click')) {
            this.wireFire(id, 'click');
            return;
          }
        }
        const hit = this.blocks.size ? this.blockHit(e) : null;
        if (hit) {
          if (hit.block != null && hit.block !== this.selBlock) this.pickBlock(hit.block);
          this.startBlockDrag(e, hit);
          return;
        }
        if (this.selBlock != null) this.pickBlock(null);
        // a click on empty room lets go of the selection; a double click makes a text
        const now = performance.now();
        if (this.lastEmpty && now - this.lastEmpty < 450) {
          this.lastEmpty = 0;
          this.send(this.ev('newtext', null, e));
        } else this.lastEmpty = now;
        if (this.selected) this.pick(null);
        return;
      }
      if (e.button !== 2) return;
      e.preventDefault();
      this.rdown = { x: e.clientX, y: e.clientY };
      this.looking = true;
      if (this.ease) { this.ease = null; this.syncTurn(); }
      h.classList.add('looking');
      try { h.setPointerCapture(e.pointerId); } catch { /* fine without */ }
    });
    h.addEventListener('pointermove', (e) => {
      if (this.bdrag) { this.dragBlock(e); return; }
      if (!this.looking) return;
      const k = LOOK * (this.p.look || 1);
      this.tyaw -= (e.movementX || 0) * k;
      this.tpitch = Math.max(-1.55, Math.min(1.55, this.tpitch - (e.movementY || 0) * k));
    });
    const up = (e) => {
      if (this.bdrag && (e.button === 0 || e.type === 'lostpointercapture')) this.endBlockDrag();
      if (e.button === 2 || e.type === 'lostpointercapture') {
        this.looking = false;
        h.classList.remove('looking');
        // a right click that didn't look around, on a block: its menu
        const r = this.rdown;
        this.rdown = null;
        if (e.button === 2 && r && Math.hypot(e.clientX - r.x, e.clientY - r.y) < 4 && this.blocks.size) {
          const hit = this.blockHit(e);
          if (hit && hit.block != null) {
            this.pickBlock(hit.block);
            this.send({ ...this.ev('blockmenu', null, e), data: hit.block });
          }
        }
      }
    };
    h.addEventListener('pointerup', up);
    h.addEventListener('lostpointercapture', up);
    h.addEventListener('wheel', (e) => {
      if (e.target.closest('.slabel')) return;
      e.preventDefault();
      this.speed = Math.min(200, Math.max(0.5, this.speed * (e.deltaY > 0 ? 0.9 : 1.1)));
      this.info(true);
    }, { passive: false });
    window.addEventListener('keydown', (e) => {
      if (!this.running || typing(e) || e.ctrlKey || e.metaKey || e.altKey) return;
      const k = e.code;
      const b = this.bind;
      if (FLYING.some((a) => b[a] === k) || k === 'ShiftLeft' || k === 'ShiftRight') {
        this.keys.add(k);
        if (this.ease) { this.ease = null; this.syncTurn(); }
        e.preventDefault();
      } else if (k.startsWith('Arrow') && !e.shiftKey) {
        e.preventDefault();
        this.hop(k);
      } else if (k === b.level) this.level();
      else if (k === b.back_to_start) this.home();
      else if (k === b.slower) this.speed = Math.max(0.5, this.speed / 1.25);
      else if (k === b.faster) this.speed = Math.min(200, this.speed * 1.25);
    });
    window.addEventListener('keyup', (e) => this.keys.delete(e.code));
    window.addEventListener('blur', () => this.keys.clear());
  }

  // ------------------------------------------------------------------ the frame

  loop(now) {
    if (!this.running) return;
    const dt = Math.min(0.1, (now - this.last) / 1000);
    this.last = now;
    const c = this.camera;
    if (this.keys.size) {
      const k = this.keys;
      const v = this.speed * (k.has('ShiftLeft') || k.has('ShiftRight') ? 4 : 1) * dt;
      if (this.held('fly_forward')) c.translateZ(-v);
      if (this.held('fly_back')) c.translateZ(v);
      if (this.held('fly_left')) c.translateX(-v);
      if (this.held('fly_right')) c.translateX(v);
      if (this.held('fly_up')) c.translateY(v);
      if (this.held('fly_down')) c.translateY(-v);
      if (this.held('roll_left')) this.rollA += ROLL * dt;
      if (this.held('roll_right')) this.rollA -= ROLL * dt;
    }
    if (this.ease) {
      const e = this.ease;
      e.t = Math.min(1, e.t + dt / 0.6);
      const s = e.t * e.t * (3 - 2 * e.t);
      c.position.lerpVectors(e.fromPos, e.toPos, s);
      c.quaternion.slerpQuaternions(e.fromQuat, e.toQuat, s);
      if (e.t >= 1) {
        this.ease = null;
        this.syncTurn();
      }
    } else {
      // ease the turn onto where the mouse put it (frame-rate independent)
      const a = 1 - Math.exp(-(this.p.smooth || 14) * dt);
      this.yaw += (this.tyaw - this.yaw) * a;
      this.pitch += (this.tpitch - this.pitch) * a;
      this.applyTurn();
    }
    // the physics in fixed steps, however long the frame was
    const moving = !this.paused && this.order.length && !this.asleep();
    if (moving) {
      this.acc = Math.min(this.acc + dt * this.p.substeps, 0.25);
      while (this.acc >= STEP) {
        this.step(STEP);
        this.acc -= STEP;
      }
    }
    // a still room isn't drawn again: nothing flies, turns, eases, drags or spreads, and
    // the turn has caught up with the mouse (it keeps the page light while one types
    // elsewhere)
    const turning = Math.abs(this.tyaw - this.yaw) > 1e-5 || Math.abs(this.tpitch - this.pitch) > 1e-5;
    const animated = this.blocks.size && this.animating();
    if (animated) this.applyAnims(now);
    this.wireTimers(now);
    this.listen();
    const rays = this.fadeRays(now);
    if (moving || turning || this.keys.size || this.ease || this.looking || this.drag || this.bdrag || this.dirty || animated || rays) {
      this.dirty = false;
      this.render();
      this.info(false);
    }
    requestAnimationFrame(this.loop);
  }

  render() {
    if (this.selBlock != null) this.placeGizmo();
    if (this.sky) this.sky.position.copy(this.camera.position);
    // the lights' rings face the camera
    if (this.lightMarks) for (const m of this.lightMarks.children) if (m.userData.facing) m.quaternion.copy(this.camera.quaternion);
    this.gl?.render(this.scene, this.camera);
    const c = this.camera;
    c.updateMatrixWorld();
    const { w, h } = this.size;
    const focal = h / 2 / Math.tan((c.fov * Math.PI) / 360);
    const v = new THREE.Vector3();
    for (const n of this.order) {
      this.world(n, v).applyMatrix4(c.matrixWorldInverse);
      const dist = -v.z;
      const el = n.el;
      if (dist < 0.3) {
        if (el.style.display !== 'none') el.style.display = 'none';
        continue;
      }
      const size = n.size || this.p.size;
      // a text's em on screen, in pixels
      const px = (size * focal) / dist;
      const x = w / 2 + (v.x / dist) * focal;
      const y = h / 2 - (v.y / dist) * focal;
      if (px < this.p.minPx || x < -w || x > 2 * w || y < -h || y > 2 * h) {
        if (el.style.display !== 'none') el.style.display = 'none';
        continue;
      }
      if (el.style.display === 'none') el.style.display = '';
      const s = Math.min(px, 220) / BASE_PX;
      el.style.transform = `translate3d(${x.toFixed(2)}px, ${y.toFixed(2)}px, 0) translate(-50%, -50%) scale(${s.toFixed(4)})`;
      el.style.zIndex = String(100000 - Math.round(dist * 10));
      el.style.opacity = String(Math.max(0.15, Math.min(1, 1.4 - dist / 220)).toFixed(3));
    }
  }

  info(now) {
    const t = performance.now();
    if (!now && t - this.infoAt < 120) return;
    this.infoAt = t;
    const p = this.camera.position;
    const state = this.paused ? 'paused' : this.asleep() ? 'still' : 'moving';
    const text = `${p.x.toFixed(1)} ${p.y.toFixed(1)} ${(-p.z).toFixed(1)}   speed ${this.speed.toFixed(1)}   roll ${Math.round(this.roll())}°   ${this.order.length} texts   spread ${state}`;
    if (text === this.lastInfo) return;
    this.lastInfo = text;
    this.send({ ...this.ev('info', null), text });
  }
}
