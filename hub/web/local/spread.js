// core/src/spread.rs (itself spread.js, ported), back in JavaScript: memory texts laid
// out by their sums. The same numbers in the same order, so the layout comes out as the
// hub's does; see the Rust for the formulas.

import { spreadSums as sums } from './text.js';
import { hypot, sin, cos } from './fmath.js';

const PI = Math.PI;

export const PARAMS = {
  damping: 0.12,
  gravity: 1.2,
  repel: 8.0,
  range: 3.0,
  atbash: 1.0,
  pad: 0.3,
  groupAttract: 0.3,
  groupRepel: 60.0,
  groupAlign: 0.3,
  qscale: 12.0,
  anchor: 0.25,
  ring: 10.0,
  depth: 0.2,
  d: 30.0,
  cool: 0.0228,
  reversed: false,
  vmax: 40.0,
};

export const atb = (a, b) => (Math.abs(a.s - b.r) + Math.abs(b.s - a.r)) / Math.max(a.s + a.r + b.s + b.r, 1);

export function home(rank, ring) {
  const th = Math.sqrt(4 * PI * rank);
  const rr = ring * Math.sqrt(rank / PI);
  return [rr * cos(th), rr * sin(th)];
}

/// FNV-1a over UTF-16 code units
export function hash01(s, salt) {
  let h = (2166136261 ^ salt) >>> 0;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619) >>> 0;
  }
  return h / 4294967296;
}

/// remem's seedCLR: a bucket's hue from its Q
export function seedHue(q) {
  let hash = 0;
  const t = String(q);
  for (let i = 0; i < t.length; i++) hash = (t.charCodeAt(i) + ((hash << 5) - hash)) | 0;
  return Math.abs(hash % 360);
}

export class Spread {
  constructor(p = {}) {
    this.p = { ...PARAMS, ...p };
    this.nodes = [];
    this.byId = new Map();
    this.order = [];
    this.minQ = 0;
    this.dirty = false;
    this.alpha = 1;
    this.energy = 0;
    this.sorted = [];
  }

  static node(id, text, hw, hh) {
    const [s, r] = sums(text);
    return { id, text, s, r, p: s, q: r, x: 0, y: 0, vx: 0, vy: 0, z: 0, zt: 0, k: 1, hw, hh, fx: 0, fy: 0, pinned: false, placed: false, bucket: 0 };
  }

  add(n) {
    if (this.byId.has(n.id)) return this.byId.get(n.id);
    this.byId.set(n.id, this.nodes.length);
    this.nodes.push(n);
    this.dirty = true;
    this.wake(1);
    return this.nodes.length - 1;
  }

  wake(a) {
    this.alpha = Math.max(this.alpha, a);
  }

  asleep() {
    return this.alpha < 0.004 && this.energy < 0.002;
  }

  rekey() {
    const rv = this.p.reversed;
    const buckets = new Map();
    this.nodes.forEach((n, i) => {
      n.p = rv ? n.r : n.s;
      n.q = rv ? n.s : n.r;
      if (!buckets.has(n.q)) buckets.set(n.q, { q: n.q, members: [], n: 0, rank: 0, hx: 0, hy: 0, cx: 0, cy: 0, vx: 0, vy: 0, rad: 0, fx: 0, fy: 0 });
      buckets.get(n.q).members.push(i);
    });
    const qs = [...buckets.keys()].sort((a, b) => a - b);
    this.minQ = qs.length ? qs[0] : 0;
    this.order = qs.map((q, i) => {
      const b = buckets.get(q);
      b.n = b.members.length;
      b.rank = i;
      [b.hx, b.hy] = home(i, this.p.ring);
      return b;
    });
    const slot = new Map(this.order.map((b, i) => [b.q, i]));
    for (const n of this.nodes) {
      n.bucket = slot.get(n.q);
      n.zt = (n.q - this.minQ) * this.p.depth;
    }
    for (const b of this.order) {
      const placed = b.members.filter((m) => this.nodes[m].placed);
      let cx = b.hx;
      let cy = b.hy;
      if (placed.length) {
        cx = placed.reduce((a, m) => a + this.nodes[m].x, 0) / placed.length;
        cy = placed.reduce((a, m) => a + this.nodes[m].y, 0) / placed.length;
      }
      const spreadR = 1 + Math.sqrt(b.n) * 1.5;
      for (const m of b.members) {
        const n = this.nodes[m];
        if (!n.placed) {
          const a = hash01(n.id, 7) * PI * 2;
          const rr = spreadR * Math.sqrt(hash01(n.id, 13));
          n.x = cx + cos(a) * rr;
          n.y = cy + sin(a) * rr;
          n.z = n.zt;
          n.vx = 0;
          n.vy = 0;
          n.placed = true;
        }
      }
    }
    this.sorted = this.nodes.map((_, i) => i);
    this.dirty = false;
    this.wake(1);
  }

  // every pair whose x extents come within `margin`, swept by left edge
  pairs(margin, f) {
    const nodes = this.nodes;
    const left = (n) => n.x - n.hw * n.k;
    this.sorted.sort((a, b) => {
      const x = left(nodes[a]);
      const y = left(nodes[b]);
      return x < y ? -1 : x > y ? 1 : 0;
    });
    const s = this.sorted;
    for (let i = 0; i < s.length; i++) {
      const a = s[i];
      const right = nodes[a].x + nodes[a].hw * nodes[a].k + margin;
      for (let j = i + 1; j < s.length; j++) {
        const b = s[j];
        if (nodes[b].x - nodes[b].hw * nodes[b].k > right) break;
        f(nodes, a, b);
      }
    }
  }

  step(dt) {
    if (this.dirty) this.rekey();
    const p = this.p;
    const d = p.d;
    if (!this.nodes.length) {
      this.alpha = 0;
      this.energy = 0;
      return 0;
    }
    const a = this.alpha;

    for (const n of this.nodes) {
      if (!n.pinned) n.z += (n.zt - n.z) * 0.08;
      n.k = d / Math.max(d + n.z, 1e-3);
      n.fx = 0;
      n.fy = 0;
    }

    for (const b of this.order) {
      let cx = 0;
      let cy = 0;
      let vx = 0;
      let vy = 0;
      let area = 0;
      for (const m of b.members) {
        const n = this.nodes[m];
        cx += n.x;
        cy += n.y;
        vx += n.vx;
        vy += n.vy;
        area += 4 * n.hw * n.hh * n.k * n.k;
      }
      b.cx = cx / b.n;
      b.cy = cy / b.n;
      b.vx = vx / b.n;
      b.vy = vy / b.n;
      b.rad = Math.sqrt(area) * 0.6;
      b.fx = 0;
      b.fy = 0;
    }

    for (const n of this.nodes) {
      if (n.pinned) continue;
      const b = this.order[n.bucket];
      if (b.n > 1) {
        n.fx += p.gravity * (b.cx - n.x);
        n.fy += p.gravity * (b.cy - n.y);
      }
      n.fx += p.anchor * (b.hx - b.cx);
      n.fy += p.anchor * (b.hy - b.cy);
    }

    if (p.groupAttract !== 0 || p.groupRepel !== 0 || p.groupAlign !== 0) {
      const g = this.order;
      for (let i = 0; i < g.length; i++) {
        for (let j = i + 1; j < g.length; j++) {
          const c = g[i];
          const e = g[j];
          let dx = e.cx - c.cx;
          let dy = e.cy - c.cy;
          let dd = hypot(dx, dy);
          if (dd < 1e-6) {
            const ang = (c.q * 0.618 + e.q) % (PI * 2);
            dx = cos(ang);
            dy = sin(ang);
            dd = 1e-6;
          } else {
            dx /= dd;
            dy /= dd;
          }
          const aff = 1 / (1 + Math.abs(e.q - c.q) / p.qscale);
          const f = p.groupAttract * aff * (dd - c.rad - e.rad) - (p.groupRepel * (c.n * e.n)) / (dd * dd + 1);
          const ax = p.groupAlign * aff * (e.vx - c.vx);
          const ay = p.groupAlign * aff * (e.vy - c.vy);
          c.fx += f * dx + ax;
          c.fy += f * dy + ay;
          e.fx -= f * dx + ax;
          e.fy -= f * dy + ay;
        }
      }
    }
    for (const b of this.order) {
      if (b.fx === 0 && b.fy === 0) continue;
      const gx = b.fx / b.n;
      const gy = b.fy / b.n;
      for (const m of b.members) {
        const n = this.nodes[m];
        if (!n.pinned) {
          n.fx += gx;
          n.fy += gy;
        }
      }
    }

    if (p.repel !== 0 && p.range > 0) {
      this.pairs(p.range, (nodes, ia, ib) => {
        const na = nodes[ia];
        const nb = nodes[ib];
        const dx = na.x - nb.x;
        const dy = na.y - nb.y;
        const gx = Math.abs(dx) - (na.hw * na.k + nb.hw * nb.k);
        const gy = Math.abs(dy) - (na.hh * na.k + nb.hh * nb.k);
        const gap = hypot(Math.max(gx, 0), Math.max(gy, 0));
        if (gap >= p.range) return;
        const len = hypot(dx, dy);
        let ux;
        let uy;
        if (len < 1e-6) {
          const ang = hash01(`${na.id}${nb.id}`, 3) * PI * 2;
          ux = cos(ang);
          uy = sin(ang);
        } else {
          ux = dx / len;
          uy = dy / len;
        }
        const m = (p.repel * (1 + p.atbash * atb(na, nb)) * (1 - gap / p.range)) / ((gap + 1) * (gap + 1));
        if (!na.pinned) {
          na.fx += m * ux;
          na.fy += m * uy;
        }
        if (!nb.pinned) {
          nb.fx -= m * ux;
          nb.fy -= m * uy;
        }
      });
    }

    const keep = 1 - p.damping;
    let e = 0;
    let moving = 0;
    for (const n of this.nodes) {
      if (n.pinned) {
        n.vx = 0;
        n.vy = 0;
        continue;
      }
      n.vx = (n.vx + a * n.fx * dt) * keep;
      n.vy = (n.vy + a * n.fy * dt) * keep;
      const sp = hypot(n.vx, n.vy);
      if (sp > p.vmax) {
        n.vx *= p.vmax / sp;
        n.vy *= p.vmax / sp;
      }
      n.x += n.vx * dt;
      n.y += n.vy * dt;
      e += n.vx * n.vx + n.vy * n.vy;
      moving++;
    }

    for (let it = 0; it < 3; it++) {
      this.pairs(p.pad, (nodes, ia, ib) => {
        const na = nodes[ia];
        const nb = nodes[ib];
        if (na.pinned && nb.pinned) return;
        const dx = nb.x - na.x;
        const dy = nb.y - na.y;
        const ox = na.hw * na.k + nb.hw * nb.k + p.pad - Math.abs(dx);
        const oy = na.hh * na.k + nb.hh * nb.k + p.pad - Math.abs(dy);
        if (ox <= 0 || oy <= 0) return;
        const wa = na.pinned ? 0 : nb.pinned ? 1 : 0.5;
        const wb = 1 - wa;
        const pairId = `${na.id}${nb.id}`;
        if (ox < oy) {
          const sg = dx < 0 ? -1 : dx > 0 ? 1 : hash01(pairId, 5) < 0.5 ? -1 : 1;
          na.x -= sg * ox * wa;
          nb.x += sg * ox * wb;
        } else {
          const sg = dy < 0 ? -1 : dy > 0 ? 1 : hash01(pairId, 9) < 0.5 ? -1 : 1;
          na.y -= sg * oy * wa;
          nb.y += sg * oy * wb;
        }
      });
    }

    this.energy = moving > 0 ? e / moving : 0;
    this.alpha -= this.alpha * p.cool;
    return this.energy;
  }

  /// steps until still, or the step or time budget runs out; returns the steps taken
  settle(maxSteps, ms) {
    const t0 = Date.now();
    let i = 0;
    while (i < maxSteps && !this.asleep() && Date.now() - t0 < ms) {
      this.step(1 / 60);
      i++;
    }
    return i;
  }
}
