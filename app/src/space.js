// the space: memory texts float as camera-facing labels (html laid over a webgl room at
// each text's projected place, its font sized by distance, so the text stays crisp and
// no css 3d is needed) and you fly through it. Adapted from hub/web/space.js for the
// Dioxus app: it talks to Rust through `send` (events) and the commands Rust sends in.
//
// right mouse drag looks, W A S D move, Space / C up and down, Q / E roll, Shift fast,
// [ ] or the wheel change the speed, R levels, Home goes back; the arrow keys ease the
// camera onto the nearest text in that direction. Texts shrink as the camera nears them.

import * as THREE from '__THREE__';

const FONT = 40;
const PX = 0.014;
const LOOK = 0.0026;
const ROLL = 1.4;
// the flying keys by action, as the keymap binds them (Rust sends them: `keys`)
const DEFAULT_KEYS = { fly_forward: 'KeyW', fly_back: 'KeyS', fly_left: 'KeyA', fly_right: 'KeyD', fly_up: 'Space', fly_down: 'KeyC', roll_left: 'KeyQ', roll_right: 'KeyE', level: 'KeyR', back_to_start: 'Home', slower: 'BracketLeft', faster: 'BracketRight' };

const typing = (e) => {
  const t = e.target || {};
  return t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable;
};

export class Space {
  constructor(host, send) {
    this.host = host;
    this.send = send;
    this.items = new Map(); // id → {obj, el, hit, node}
    this.keys = new Set();
    this.bind = { ...DEFAULT_KEYS };
    this.speed = 6;
    this.running = false;
    this.looking = false;
    this.ease = null; // {fromPos, toPos, fromQuat, toQuat, t}
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
    this.resize = this.resize.bind(this);
    new ResizeObserver(this.resize).observe(host);
    this.resize();
    this.listen();
    this.loop = this.loop.bind(this);
  }

  setKeys(map) {
    this.bind = { ...DEFAULT_KEYS, ...map };
    this.keys.clear();
  }

  held(action) {
    return this.keys.has(this.bind[action]);
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

  place(texts, spread, again) {
    if (!again) {
      this.frame.pos.copy(this.camera.position);
      this.frame.quat.copy(this.camera.quaternion);
    }
    const byId = new Map(spread.nodes.map((n) => [String(n.id), n]));
    const keep = new Set();
    for (const t of texts) {
      const n = byId.get(String(t.key));
      if (!n) continue;
      keep.add(String(t.key));
      let it = this.items.get(String(t.key));
      if (!it) {
        const el = document.createElement('div');
        el.className = 'slabel';
        el.textContent = t.note ? `✎ ${t.text}` : t.text;
        el.tabIndex = -1;
        const obj = new THREE.Object3D();
        it = { obj, el, hit: t };
        this.wire(it);
        this.overlay.append(el);
        this.items.set(String(t.key), it);
      }
      it.node = n;
      it.el.style.borderBottomColor = `hsl(${n.hue} 70% 45%)`;
      const d = spread.d;
      const k = (d + n.z) / d;
      const local = new THREE.Vector3(n.x * k, n.y * k, -(d + n.z));
      it.obj.position.copy(local.applyQuaternion(this.frame.quat).add(this.frame.pos));
    }
    for (const [id, it] of this.items) if (!keep.has(id)) this.drop(id, it);
    this.render();
    this.info();
  }

  clear() {
    for (const [id, it] of this.items) this.drop(id, it);
    this.render();
    this.info();
  }

  drop(id, it) {
    it.el.remove();
    this.items.delete(id);
  }

  mark(list, candidate) {
    for (const it of this.items.values()) it.el.classList.toggle('picked', it.hit.key === list || it.hit.key === candidate);
  }

  wire(it) {
    const { el } = it;
    const ev = (kind, e) =>
      this.send({ kind, key: it.hit.key, text: it.hit.text, note: !!it.hit.note, x: e.clientX, y: e.clientY, ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey, alt: e.altKey, q: it.node?.q ?? 0, hue: it.node?.hue ?? 0 });
    el.addEventListener('pointerenter', (e) => !this.looking && ev('hover', e));
    el.addEventListener('click', (e) => e.button === 0 && ev('click', e));
    el.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      ev('menu', e);
    });
  }

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

  // the arrow keys: the nearest text in that direction on screen, eased onto
  hop(dir) {
    const cam = this.camera;
    cam.updateMatrixWorld();
    let best = null;
    let bestScore = Infinity;
    const want = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, 1], ArrowDown: [0, -1] }[dir];
    for (const it of this.items.values()) {
      const local = it.obj.position.clone().applyMatrix4(cam.matrixWorldInverse);
      const dist = local.length();
      if (dist < 0.5) continue;
      // its direction from where the camera looks, in the view's plane
      const sx = local.x / dist;
      const sy = local.y / dist;
      const along = sx * want[0] + sy * want[1];
      if (along <= 0.02) continue;
      const across = Math.abs(sx * want[1] - sy * want[0]);
      const score = (Math.acos(Math.min(1, along)) + across * 2) * 10 + dist * 0.05 + (local.z > 0 ? 50 : 0);
      if (score < bestScore) {
        bestScore = score;
        best = it;
      }
    }
    if (!best) return;
    const target = best.obj.position.clone();
    const fwd = target.clone().sub(cam.position).normalize();
    const stand = Math.max(4, Math.min(14, best.hit.text.length * 0.35));
    const toPos = target.clone().sub(fwd.clone().multiplyScalar(stand));
    const look = new THREE.Object3D();
    look.position.copy(toPos);
    look.up.copy(new THREE.Vector3(0, 1, 0).applyQuaternion(cam.quaternion));
    look.lookAt(target);
    // a camera looks down -z, an Object3D's lookAt points +z: turn it round
    look.rotateY(Math.PI);
    this.ease = { fromPos: cam.position.clone(), toPos, fromQuat: cam.quaternion.clone(), toQuat: look.quaternion.clone(), t: 0 };
    this.send({ kind: 'focus', key: best.hit.key, text: best.hit.text, note: !!best.hit.note, x: 0, y: 0, ctrl: false, shift: false, alt: false, q: best.node?.q ?? 0, hue: best.node?.hue ?? 0 });
  }

  listen() {
    const h = this.host;
    h.addEventListener('contextmenu', (e) => e.preventDefault());
    h.addEventListener('pointerdown', (e) => {
      if (e.button !== 2 || e.target.closest('.slabel')) return;
      e.preventDefault();
      this.looking = true;
      this.ease = null;
      h.classList.add('looking');
      try {
        h.setPointerCapture(e.pointerId);
      } catch {
        /* fine without */
      }
    });
    h.addEventListener('pointermove', (e) => {
      if (!this.looking) return;
      this.camera.rotateY(-(e.movementX || 0) * LOOK);
      this.camera.rotateX(-(e.movementY || 0) * LOOK);
      this.render();
    });
    const up = (e) => {
      if (e.button === 2 || e.type === 'lostpointercapture') {
        this.looking = false;
        h.classList.remove('looking');
      }
    };
    h.addEventListener('pointerup', up);
    h.addEventListener('lostpointercapture', up);
    h.addEventListener(
      'wheel',
      (e) => {
        if (e.target.closest('.slabel')) return;
        e.preventDefault();
        this.speed = Math.min(200, Math.max(0.5, this.speed * (e.deltaY > 0 ? 0.9 : 1.1)));
        this.info();
      },
      { passive: false },
    );
    window.addEventListener('keydown', (e) => {
      if (!this.running || typing(e) || e.ctrlKey || e.metaKey || e.altKey) return;
      const k = e.code;
      const b = this.bind;
      const flying = ['fly_forward', 'fly_back', 'fly_left', 'fly_right', 'fly_up', 'fly_down', 'roll_left', 'roll_right'];
      if (flying.some((a) => b[a] === k) || k === 'ShiftLeft' || k === 'ShiftRight') {
        this.keys.add(k);
        this.ease = null;
        e.preventDefault();
      } else if (k.startsWith('Arrow') && !e.shiftKey) {
        e.preventDefault();
        this.hop(k);
      } else if (k === b.level) this.level();
      else if (k === b.back_to_start) this.home();
      else if (k === b.slower) this.speed = Math.max(0.5, this.speed / 1.25);
      else if (k === b.faster) this.speed = Math.min(200, this.speed * 1.25);
      else return;
      this.info();
    });
    window.addEventListener('keyup', (e) => this.keys.delete(e.code));
    window.addEventListener('blur', () => this.keys.clear());
  }

  loop(now) {
    if (!this.running) return;
    const dt = Math.min(0.1, (now - this.last) / 1000);
    this.last = now;
    const k = this.keys;
    const c = this.camera;
    if (k.size) {
      const v = this.speed * (k.has('ShiftLeft') || k.has('ShiftRight') ? 4 : 1) * dt;
      if (this.held('fly_forward')) c.translateZ(-v);
      if (this.held('fly_back')) c.translateZ(v);
      if (this.held('fly_left')) c.translateX(-v);
      if (this.held('fly_right')) c.translateX(v);
      if (this.held('fly_up')) c.translateY(v);
      if (this.held('fly_down')) c.translateY(-v);
      if (this.held('roll_left')) c.rotateZ(ROLL * dt);
      if (this.held('roll_right')) c.rotateZ(-ROLL * dt);
      this.info();
    }
    if (this.ease) {
      const e = this.ease;
      e.t = Math.min(1, e.t + dt / 0.6);
      const s = e.t * e.t * (3 - 2 * e.t);
      c.position.lerpVectors(e.fromPos, e.toPos, s);
      c.quaternion.slerpQuaternions(e.fromQuat, e.toQuat, s);
      if (e.t >= 1) this.ease = null;
      this.info();
    }
    this.render();
    requestAnimationFrame(this.loop);
  }

  render() {
    this.gl?.render(this.scene, this.camera);
    // each label at its projected place, sized by distance; texts shrink as the camera
    // nears them (up to a fifth), so a close one stays readable rather than filling the room
    const c = this.camera;
    c.updateMatrixWorld();
    const { w, h } = this.size;
    const focal = h / 2 / Math.tan((c.fov * Math.PI) / 360);
    const v = new THREE.Vector3();
    for (const it of this.items.values()) {
      v.copy(it.obj.position).applyMatrix4(c.matrixWorldInverse);
      const dist = -v.z;
      if (dist < 0.3) {
        it.el.style.display = 'none';
        continue;
      }
      const shrink = Math.max(0.22, Math.min(1, dist / 14));
      const px = (FONT * PX * focal * shrink) / dist;
      const x = w / 2 + (v.x / dist) * focal;
      const y = h / 2 - (v.y / dist) * focal;
      if (px < 2 || x < -w || x > 2 * w || y < -h || y > 2 * h) {
        it.el.style.display = 'none';
        continue;
      }
      it.el.style.display = '';
      it.el.style.fontSize = `${Math.min(px, 160).toFixed(1)}px`;
      it.el.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px) translate(-50%, -50%)`;
      it.el.style.zIndex = String(100000 - Math.round(dist * 10));
      it.el.style.opacity = String(Math.max(0.15, Math.min(1, 1.4 - dist / 220)));
    }
  }

  info() {
    const p = this.camera.position;
    this.send({ kind: 'info', key: '', text: `${p.x.toFixed(1)} ${p.y.toFixed(1)} ${(-p.z).toFixed(1)}   speed ${this.speed.toFixed(1)}   roll ${Math.round(this.roll())}°   ${this.items.size} texts`, note: false, x: 0, y: 0, ctrl: false, shift: false, alt: false, q: 0, hue: 0 });
  }
}
