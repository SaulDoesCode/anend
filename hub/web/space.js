// the space: a warm room where memory texts float as camera-facing labels (css 3d
// sprites over a webgl room, so the text stays crisp), and flying through it.
//
// right mouse drag looks, W A S D move, Space / C up and down, Q / E roll, Shift fast,
// [ ] or the wheel over empty space change the speed, R levels, Home goes back.

import * as THREE from './vendor/three.module.min.js';
import { CSS3DRenderer, CSS3DSprite } from './vendor/CSS3DRenderer.js';
import { typing } from './util.js';

// a label's font size in css pixels, and how big a pixel is in the world
const FONT = 40;
const PX = 0.014;
const LOOK = 0.0026;
const ROLL = 1.4;

export class Space {
  constructor(host, hooks = {}) {
    this.host = host;
    this.hooks = hooks;
    this.items = new Map(); // id → {obj, el, hit, node}
    this.keys = new Set();
    this.speed = 6;
    this.running = false;
    this.looking = false;
    // the frame a spread is laid out in front of: the camera's when it was searched
    this.frame = { pos: new THREE.Vector3(), quat: new THREE.Quaternion() };

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
      // no webgl: the room is the page's background, the texts still float
      this.gl = null;
    }
    this.css = new CSS3DRenderer();
    this.css.domElement.className = 'space-css';
    host.append(this.css.domElement);

    // the floor, faint, so flying has something to go by
    this.grid = new THREE.GridHelper(600, 120, 0x000000, 0x000000);
    this.grid.position.y = -9;
    for (const m of [].concat(this.grid.material)) {
      m.transparent = true;
      m.opacity = 0.06;
      m.depthWrite = false;
    }
    this.scene.add(this.grid);

    this.resize = this.resize.bind(this);
    new ResizeObserver(this.resize).observe(host);
    this.resize();
    this.listen();
    this.loop = this.loop.bind(this);
  }

  setRoom(color) {
    this.room.set(color);
    this.scene.fog.color.set(color);
    this.host.style.background = color;
    // the floor's lines go light in a dark room
    const dark = this.room.r * 0.3 + this.room.g * 0.59 + this.room.b * 0.11 < 0.35;
    for (const m of [].concat(this.grid.material)) {
      m.color.set(dark ? 0xffffff : 0x000000);
      m.opacity = dark ? 0.05 : 0.06;
    }
  }

  resize() {
    const w = this.host.clientWidth || 1;
    const h = this.host.clientHeight || 1;
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.gl?.setSize(w, h);
    this.css.setSize(w, h);
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
    this.endLook();
  }

  // ---------------------------------------------------------------- texts

  // lays a spread out: nodes {id, x, y, z, hue}, d the spread's depth; keep the frame
  // when `again` (texts added to what's there)
  place(texts, spread, again = false) {
    if (!again) {
      this.frame.pos.copy(this.camera.position);
      this.frame.quat.copy(this.camera.quaternion);
    }
    const byId = new Map(spread.nodes.map((n) => [String(n.id), n]));
    const keep = new Set();
    for (const t of texts) {
      const n = byId.get(String(t.id));
      if (!n) continue;
      keep.add(String(t.id));
      let it = this.items.get(String(t.id));
      if (!it) {
        const el = document.createElement('div');
        el.className = 'label';
        el.textContent = t.hit.note ? `✎ ${t.text}` : t.text;
        el.style.fontSize = `${FONT}px`;
        const obj = new CSS3DSprite(el);
        obj.scale.setScalar(PX);
        it = { obj, el, hit: t.hit };
        this.wire(it);
        this.scene.add(obj);
        this.items.set(String(t.id), it);
      }
      it.node = n;
      it.el.style.setProperty('--hue', n.hue);
      it.el.style.borderBottomColor = `hsl(${n.hue} 70% 45%)`;
      const d = spread.d;
      const k = (d + n.z) / d;
      const local = new THREE.Vector3(n.x * k, n.y * k, -(d + n.z));
      it.obj.position.copy(local.applyQuaternion(this.frame.quat).add(this.frame.pos));
    }
    for (const [id, it] of this.items) {
      if (!keep.has(id)) this.drop(id, it);
    }
    this.render();
  }

  clear() {
    for (const [id, it] of this.items) this.drop(id, it);
    this.render();
  }

  drop(id, it) {
    this.scene.remove(it.obj);
    it.el.remove();
    this.items.delete(id);
  }

  wire(it) {
    const { el } = it;
    el.addEventListener('pointerenter', (e) => !this.looking && this.hooks.hover?.(it, e));
    el.addEventListener('pointerleave', (e) => this.hooks.leave?.(it, e));
    el.addEventListener('click', (e) => {
      if (e.button === 0) this.hooks.click?.(it, e);
    });
  }

  // ---------------------------------------------------------------- flying

  home() {
    this.camera.position.set(0, 0, 0);
    this.camera.quaternion.identity();
    this.render();
  }

  level() {
    const fwd = new THREE.Vector3(0, 0, -1).applyQuaternion(this.camera.quaternion);
    if (Math.abs(fwd.y) > 0.999) fwd.set(0, 0, -1);
    this.camera.up.set(0, 1, 0);
    this.camera.lookAt(this.camera.position.clone().add(fwd));
    this.render();
  }

  roll() {
    const q = this.camera.quaternion;
    const right = new THREE.Vector3(1, 0, 0).applyQuaternion(q);
    const up = new THREE.Vector3(0, 1, 0).applyQuaternion(q);
    return (Math.atan2(right.y, up.y) * 180) / Math.PI;
  }

  endLook() {
    if (!this.looking) return;
    this.looking = false;
    this.host.classList.remove('looking');
  }

  listen() {
    const h = this.host;
    h.addEventListener('contextmenu', (e) => e.preventDefault());
    h.addEventListener('pointerdown', (e) => {
      if (e.button !== 2) return;
      e.preventDefault();
      this.looking = true;
      this.lastX = e.clientX;
      this.lastY = e.clientY;
      h.classList.add('looking');
      this.hooks.leave?.();
      try {
        h.setPointerCapture(e.pointerId);
      } catch {
        /* fine without */
      }
    });
    h.addEventListener('pointermove', (e) => {
      if (!this.looking) return;
      const dx = e.movementX ?? e.clientX - this.lastX;
      const dy = e.movementY ?? e.clientY - this.lastY;
      this.lastX = e.clientX;
      this.lastY = e.clientY;
      this.look(dx, dy);
    });
    const up = (e) => {
      if (e.button === 2 || e.type === 'lostpointercapture') this.endLook();
    };
    h.addEventListener('pointerup', up);
    h.addEventListener('lostpointercapture', up);
    h.addEventListener(
      'wheel',
      (e) => {
        if (e.target.closest('.label')) return;
        e.preventDefault();
        this.speed = Math.min(200, Math.max(0.5, this.speed * (e.deltaY > 0 ? 0.9 : 1.1)));
        this.info();
      },
      { passive: false },
    );
    window.addEventListener('keydown', (e) => {
      if (!this.running || typing(e) || e.ctrlKey || e.metaKey || e.altKey) return;
      const k = e.code;
      if (FLY.has(k)) {
        this.keys.add(k);
        e.preventDefault();
      } else if (k === 'KeyR') this.level();
      else if (k === 'Home') this.home();
      else if (k === 'BracketLeft') this.speed = Math.max(0.5, this.speed / 1.25);
      else if (k === 'BracketRight') this.speed = Math.min(200, this.speed * 1.25);
      else return;
      this.info();
    });
    window.addEventListener('keyup', (e) => this.keys.delete(e.code));
    window.addEventListener('blur', () => this.keys.clear());
  }

  look(dx, dy) {
    this.camera.rotateY(-dx * LOOK);
    this.camera.rotateX(-dy * LOOK);
    this.render();
  }

  loop(now) {
    if (!this.running) return;
    const dt = Math.min(0.1, (now - this.last) / 1000);
    this.last = now;
    const k = this.keys;
    if (k.size) {
      const v = this.speed * (k.has('ShiftLeft') || k.has('ShiftRight') ? 4 : 1) * dt;
      const c = this.camera;
      if (k.has('KeyW')) c.translateZ(-v);
      if (k.has('KeyS')) c.translateZ(v);
      if (k.has('KeyA')) c.translateX(-v);
      if (k.has('KeyD')) c.translateX(v);
      if (k.has('Space')) c.translateY(v);
      if (k.has('KeyC')) c.translateY(-v);
      if (k.has('KeyQ')) c.rotateZ(ROLL * dt);
      if (k.has('KeyE')) c.rotateZ(-ROLL * dt);
      this.info();
    }
    this.render();
    requestAnimationFrame(this.loop);
  }

  info() {
    const p = this.camera.position;
    this.hooks.info?.({ x: p.x, y: p.y, z: p.z, speed: this.speed, roll: this.roll(), texts: this.items.size });
  }

  render() {
    this.gl?.render(this.scene, this.camera);
    this.css.render(this.scene, this.camera);
  }
}

const FLY = new Set(['KeyW', 'KeyA', 'KeyS', 'KeyD', 'Space', 'KeyC', 'KeyQ', 'KeyE', 'ShiftLeft', 'ShiftRight']);

// ------------------------------------------------------------------ the triangle

// the triangle with sides s, r and t = ⌊√(s·r)⌋, fitted to `height` pixels; when the three
// can't close (one side as long as the other two), the flat shape they come closest to,
// dashed and marked
export function triangle(s, r, t, height = 22, maxWidth = 64) {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('class', 'triangle');
  if (!(s > 0 && r > 0 && t > 0)) {
    svg.setAttribute('width', 0);
    svg.setAttribute('height', 0);
    return svg;
  }
  const real = s < r + t && r < s + t && t < s + r;
  // base s from A to B, C at t from A and r from B
  let x = (s * s + t * t - r * r) / (2 * s);
  x = Math.max(-t, Math.min(t, x));
  const y = real ? Math.sqrt(Math.max(0, t * t - x * x)) : 0;
  const minX = Math.min(0, x);
  const maxX = Math.max(s, x);
  const w = maxX - minX;
  const pad = 2;
  const k = Math.min((height - pad * 2) / Math.max(y, 1e-9), (maxWidth - pad * 2) / w);
  const W = w * k + pad * 2;
  const H = real ? y * k + pad * 2 : height;
  const base = real ? H - pad : H / 2;
  const P = (px, py) => `${((px - minX) * k + pad).toFixed(1)},${(base - py * k).toFixed(1)}`;
  svg.setAttribute('width', W.toFixed(1));
  svg.setAttribute('height', H.toFixed(1));
  svg.setAttribute('viewBox', `0 0 ${W.toFixed(1)} ${H.toFixed(1)}`);
  const poly = document.createElementNS(ns, 'polygon');
  poly.setAttribute('points', `${P(0, 0)} ${P(s, 0)} ${P(x, y)}`);
  if (!real) poly.setAttribute('stroke-dasharray', '2 2');
  svg.append(poly);
  const deg = (v) => ((Math.acos(Math.max(-1, Math.min(1, v))) * 180) / Math.PI).toFixed(1);
  const angles = real
    ? `angles ${deg((s * s + t * t - r * r) / (2 * s * t))}° (facing r), ${deg((s * s + r * r - t * t) / (2 * s * r))}° (facing t), ${deg((r * r + t * t - s * s) / (2 * r * t))}° (facing s)`
    : `not a triangle: ${Math.max(s, r, t)} is at least the other two sides together (${s + r + t - Math.max(s, r, t)}), so it lies flat`;
  const title = document.createElementNS(ns, 'title');
  title.textContent = `s ${s}, r ${r}, t = ⌊√(s·r)⌋ ${t}\n${angles}`;
  svg.append(title);
  if (!real) svg.classList.add('flat');
  return svg;
}
