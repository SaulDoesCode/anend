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
    this.mats = new Map(); // texture key → material
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
    return n;
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

  weights() {
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
    const fx = new Float64Array(n), fy = new Float64Array(n), fz = new Float64Array(n);
    const L = p.link;
    for (let i = 0; i < n; i++) {
      const A = N[i];
      for (let j = i + 1; j < n; j++) {
        const B = N[j];
        let dx = B.x - A.x, dy = B.y - A.y, dz = B.z - A.z;
        let d2 = dx * dx + dy * dy + dz * dz;
        if (d2 < 1e-6) { dx = (hash01(A.key + B.key, 7) - 0.5) * 0.01; dy = (hash01(B.key + A.key, 9) - 0.5) * 0.01; d2 = dx * dx + dy * dy + 1e-6; }
        const d = Math.sqrt(d2);
        const s = this.sim[i * n + j];
        // every two repel (more when the weights say alike texts should part), softened near
        let f = -p.repel * (1 + Math.max(0, -s) * 2) / (d2 + 0.5);
        // alike texts pull toward a rest length that shrinks the more alike they are
        if (s > 0) f += p.attract * s * (d - L * (1.15 - s));
        const ux = dx / d, uy = dy / d, uz = dz / d;
        fx[i] += f * ux; fy[i] += f * uy; fz[i] += f * uz;
        fx[j] -= f * ux; fy[j] -= f * uy; fz[j] -= f * uz;
      }
      // gravity to the centre, and toward the depth the measure gives
      fx[i] -= p.gravity * A.x;
      fy[i] -= p.gravity * A.y;
      const zt = p.depth_by === 'none' ? 0 : this.depthOf(A) * p.depth;
      fz[i] -= p.flat * (A.z - zt);
    }
    const drag = Math.exp(-p.damping * h);
    let ke = 0;
    for (let i = 0; i < n; i++) {
      const A = N[i];
      if (A.pinned || A === this.drag?.n) { A.vx = A.vy = A.vz = 0; continue; }
      A.vx = (A.vx + a * fx[i] * h) * drag;
      A.vy = (A.vy + a * fy[i] * h) * drag;
      A.vz = (A.vz + a * fz[i] * h) * drag;
      const v2 = A.vx * A.vx + A.vy * A.vy + A.vz * A.vz;
      if (v2 > 900) { const k = 30 / Math.sqrt(v2); A.vx *= k; A.vy *= k; A.vz *= k; }
      A.x += A.vx * h; A.y += A.vy * h; A.z += A.vz * h;
      ke += v2;
    }
    // labels never overlap: separate rectangles in the facing plane, along the shorter way
    for (let pass = 0; pass < 2; pass++) {
      for (let i = 0; i < n; i++) {
        const A = N[i];
        for (let j = i + 1; j < n; j++) {
          const B = N[j];
          if (Math.abs(A.z - B.z) > 1.5) continue;
          const ox = A.hw + B.hw + p.pad - Math.abs(B.x - A.x);
          const oy = A.hh + B.hh + p.pad * 0.5 - Math.abs(B.y - A.y);
          if (ox <= 0 || oy <= 0) continue;
          const am = A.pinned || A === this.drag?.n ? 0 : 1, bm = B.pinned || B === this.drag?.n ? 0 : 1;
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
    const t = n.own != null ? n.own : n.f.text;
    n.el.textContent = n.f.note ? `✎ ${t}` : t;
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
    return this.order.map((n) => {
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

  // a texture key's material, made once
  material(key, about) {
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

  // a box whose UVs count tiles: each face's u and v run over its sides in units
  boxGeometry(t) {
    const g = new THREE.BoxGeometry(t.w, t.h, t.d);
    const uv = g.attributes.uv;
    // BoxGeometry's faces: +x, -x, +y, -y, +z, -z, four corners each
    const spans = [[t.d, t.h], [t.d, t.h], [t.w, t.d], [t.w, t.d], [t.w, t.h], [t.w, t.h]];
    for (let f = 0; f < 6; f++) {
      for (let i = 0; i < 4; i++) {
        const k = f * 4 + i;
        uv.setXY(k, uv.getX(k) * spans[f][0] / BLOCK_TILE, uv.getY(k) * spans[f][1] / BLOCK_TILE);
      }
    }
    uv.needsUpdate = true;
    // each face a shade: the top brightest, the bottom darkest, so the box reads
    const shade = [0.86, 0.86, 1, 0.72, 0.94, 0.94];
    const col = new Float32Array(24 * 3);
    for (let f = 0; f < 6; f++) for (let i = 0; i < 4; i++) col.set([shade[f], shade[f], shade[f]], (f * 4 + i) * 3);
    g.setAttribute('color', new THREE.BufferAttribute(col, 3));
    return g;
  }

  // blocks from Rust: [{id, kind, texture, x, y, z, w, h, d}] and their textures by key
  setBlocks(list, textures, replace) {
    if (replace) {
      for (const id of [...this.blocks.keys()]) this.dropBlock(id);
    }
    for (const b of list || []) {
      const key = `${b.kind}:${b.texture}`;
      const about = (textures && textures[key]) || {};
      const had = this.blocks.get(b.id);
      if (had) {
        had.b = b;
        this.shapeBlock(had);
        continue;
      }
      const mesh = new THREE.Mesh(this.boxGeometry(b), this.material(key, about));
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
    const p = mesh.geometry.parameters;
    if (p.width !== b.w || p.height !== b.h || p.depth !== b.d) {
      mesh.geometry.dispose();
      mesh.geometry = this.boxGeometry(b);
    }
    mesh.position.set(b.x, b.y, b.z);
    if (this.selBlock === b.id) this.placeGizmo();
  }

  dropBlock(id) {
    const o = this.blocks.get(id);
    if (!o) return;
    this.scene.remove(o.mesh);
    o.mesh.geometry.dispose();
    this.blocks.delete(id);
    if (this.selBlock === id) this.pickBlock(null);
  }

  blockOut(o) {
    const { id, kind, texture, x, y, z, w, h, d } = o.b;
    return { id, kind, texture, x, y, z, w, h, d };
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
    this.gizmoKnobs.forEach((k, i) => {
      k.position.copy(c);
      k.position.setComponent(i, c.getComponent(i) + half[i]);
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
  screenAxis(at, axis) {
    const toPx = (v) => {
      const p = v.clone().project(this.camera);
      return new THREE.Vector2((p.x * 0.5 + 0.5) * this.size.w, (-p.y * 0.5 + 0.5) * this.size.h);
    };
    const a = new THREE.Vector3();
    a.setComponent(axis, 1);
    return toPx(at.clone().add(a)).sub(toPx(at));
  }

  startBlockDrag(e, hit) {
    const o = this.blocks.get(this.selBlock);
    if (!o) return false;
    const b = o.b;
    const at = new THREE.Vector3(b.x, b.y, b.z);
    const fwd = new THREE.Vector3(0, 0, -1).applyQuaternion(this.camera.quaternion);
    const depth = at.clone().sub(this.camera.position).dot(fwd);
    this.bdrag = {
      o, mode: hit.handle ? hit.handle.mode : 'plane', axis: hit.handle ? hit.handle.axis : -1,
      x0: e.clientX, y0: e.clientY, from: { ...b }, depth, moved: false,
      sdir: hit.handle && hit.handle.axis >= 0 ? this.screenAxis(at, hit.handle.axis) : null,
      off: at.clone().sub(this.rayAt(e.clientX, e.clientY, depth)),
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
        // the opposite face stays put
        b[k] = f[k] + (size - f[s]) / 2;
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
    return this.order.filter((x) => !x.free && String(x.f.key) === String(entry));
  }

  ev(kind, n, e, extra) {
    const same = n && !n.free ? this.placements(n.f.key) : [];
    return {
      kind, key: n ? (n.free ? n.key : String(n.f.key)) : '', node: n ? n.key : '',
      copies: same.length, primary: !!(n && !n.free && same[0] === n), text: n ? (n.own != null ? n.own : n.f.text) : '', note: !!(n && n.f.note), free: !!(n && n.free),
      det: n ? n.det || 0 : 0, font: n && n.size ? Math.round(n.size * 50) : 0, font_default: Math.round(this.p.size * 50), own: n ? n.own : null, entry_text: n ? n.f.text : '',
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
      const w = this.world(n);
      const fwd = new THREE.Vector3(0, 0, -1).applyQuaternion(this.camera.quaternion);
      this.drag = { n, x0: e.clientX, y0: e.clientY, moved: false, depth: w.clone().sub(this.camera.position).dot(fwd), from: { x: n.x, y: n.y, z: n.z, pinned: n.pinned }, off: null };
      // where the pointer is on the text, so it doesn't jump to its centre
      const hit = this.rayAt(e.clientX, e.clientY, this.drag.depth);
      this.drag.off = w.clone().sub(hit);
    });
    el.addEventListener('pointermove', (e) => {
      const d = this.drag;
      if (!d || d.n !== n) return;
      if (!d.moved && Math.hypot(e.clientX - d.x0, e.clientY - d.y0) < 4) return;
      d.moved = true;
      n.pinned = true;
      n.el.classList.add('pinned', 'dragging');
      const w = this.rayAt(e.clientX, e.clientY, d.depth).add(d.off);
      this.local(w, n);
      this.wake(0.3);
    });
    const up = (e) => {
      const d = this.drag;
      if (!d || d.n !== n) return;
      this.drag = null;
      n.el.classList.remove('dragging');
      if (d.moved) {
        this.record({ kind: 'move', n, ...d.from });
        this.send(this.ev('moved', n, e));
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
    if (moving || turning || this.keys.size || this.ease || this.looking || this.drag || this.bdrag || this.dirty) {
      this.dirty = false;
      this.render();
      this.info(false);
    }
    requestAnimationFrame(this.loop);
  }

  render() {
    if (this.selBlock != null) this.placeGizmo();
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
