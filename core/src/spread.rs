//! spread.js, ported: memory texts laid out by their sums.
//!
//! ```text
//!   s(t)     ordinal sum: a=1 … z=26, case folded, other characters 0
//!   r(t)     reversed sum: Σ (27 − letter), equal to 27·letters − s(t)
//!   P        query axis: r when reversed, else s
//!   Q        bucket axis: s when reversed, else r
//!   bucket   nodes with equal Q; underline colour, depth and clustering follow it
//!   rank(b)  index of b among the distinct Q values, ascending
//!   home(b)  ring · √(rank/π) · (cos √(4π·rank), sin √(4π·rank))
//!            an Archimedean spiral by rank, so neighbouring values sit side by side and
//!            distance from the centre grows with the value
//!   zt       (Q − min Q) · depth
//!   k        D / (D + z)
//!   x, y     position in the screen plane seen from the spread eye at distance D;
//!            world = eye + F·(D + z) + (R·x + U·y) / k
//!   extents  half width hw·k, half height hh·k in that plane, so packing holds on
//!            screen at every depth
//!   atb(A,B) (|s_A − r_B| + |s_B − r_A|) / (s_A + r_A + s_B + r_B), 0 when A is the
//!            atbash of B
//!
//!   forces per unpinned node
//!     gravity · (centroid(bucket) − p)                       when the bucket has more than one node
//!     anchor  · (home(bucket) − centroid(bucket))
//!     repel · (1 + atbash · atb) · (1 − gap/range) / (gap + 1)²   along p_A − p_B, gap = distance between rectangles
//!   forces per bucket pair C, E, spread over the members
//!     aff = 1 / (1 + |Q_C − Q_E| / qscale)
//!     groupAttract · aff · (d − rad_C − rad_E) − groupRepel · n_C · n_E / (d² + 1)   along the centroid line
//!     groupAlign · aff · (v̄_E − v̄_C)                                                 group velocity
//!   cooling
//!     α ← α − α · cool each step; every force above is multiplied by α; any change
//!     sets α back up (wake)
//!   integration
//!     v ← (v + α·f·dt) · (1 − damping), |v| ≤ vmax;  p ← p + v·dt
//!     overlapping rectangles (extents + pad) are separated along the axis of least
//!     overlap; pinned nodes do not move
//!   asleep when α < 0.004 and mean v² < 0.002
//! ```
//!
//! Numbers are f64 like JavaScript's, so the port can be checked step by step against
//! spread.js (see the tests).

use std::collections::HashMap;
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub damping: f64,
    pub gravity: f64,
    pub repel: f64,
    pub range: f64,
    pub atbash: f64,
    pub pad: f64,
    pub group_attract: f64,
    pub group_repel: f64,
    pub group_align: f64,
    pub qscale: f64,
    pub anchor: f64,
    pub ring: f64,
    pub depth: f64,
    pub d: f64,
    pub cool: f64,
    pub reversed: bool,
    pub vmax: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            damping: 0.12,
            gravity: 1.2,
            repel: 8.0,
            range: 3.0,
            atbash: 1.0,
            pad: 0.3,
            group_attract: 0.3,
            group_repel: 60.0,
            group_align: 0.3,
            qscale: 12.0,
            anchor: 0.25,
            ring: 10.0,
            depth: 0.2,
            d: 30.0,
            cool: 0.0228,
            reversed: false,
            vmax: 40.0,
        }
    }
}

/// `[s, r]` of a text
pub fn sums(t: &str) -> (i64, i64) {
    let (mut s, mut n) = (0i64, 0i64);
    // charCodeAt walks UTF-16 code units; only ASCII letters count, so bytes do the same
    for c in t.encode_utf16() {
        let c = c | 32;
        if (97..=122).contains(&c) {
            s += (c - 96) as i64;
            n += 1;
        }
    }
    (s, 27 * n - s)
}

/// spread.js `metrics(t)`
#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    pub s: i64,
    pub r: i64,
    pub sr: i64,
    pub product: i64,
    pub root: f64,
    pub jump: i64,
    pub diff: i64,
    pub wide: i64,
    pub lean: i64,
    pub gap: i64,
    pub v: i64,
    pub c: i64,
    pub w: usize,
    pub letters: i64,
}

pub fn metrics(t: &str) -> Metrics {
    let (mut s, mut n, mut v, mut c) = (0i64, 0i64, 0i64, 0i64);
    for ch in t.chars() {
        let mut lower = ch.to_lowercase();
        let l = lower.next().unwrap_or(ch);
        let single = lower.next().is_none();
        if single && l.is_ascii_lowercase() {
            s += (l as u8 - 96) as i64;
            n += 1;
        }
        if single {
            v += match l {
                'a' => 1,
                'e' => 2,
                'o' => 3,
                'u' => 4,
                'i' => 5,
                'y' => 6,
                _ => 0,
            };
            c += match l {
                'b' => 1,
                'c' => 2,
                'd' => 3,
                'f' => 4,
                'g' => 5,
                'h' => 6,
                'j' => 7,
                'k' => 8,
                'l' => 9,
                'm' => 10,
                'n' => 11,
                'p' => 12,
                'q' => 13,
                'r' => 14,
                's' => 15,
                't' | 'v' => 16,
                'w' => 17,
                'x' => 18,
                'z' => 19,
                _ => 0,
            };
        }
    }
    let r = 27 * n - s;
    let root = ((s * r) as f64).sqrt();
    Metrics {
        s,
        r,
        sr: s + r,
        product: s * r,
        root,
        jump: root.floor() as i64,
        diff: s - r,
        wide: s + r + (s - r).abs(),
        lean: s + r + (s - r),
        gap: (s - r).abs() - (s - r),
        v,
        c,
        w: t.split_whitespace().count().max(1),
        letters: n,
    }
}

pub fn atb(a: &Node, b: &Node) -> f64 {
    ((a.s - b.r).abs() + (b.s - a.r).abs()) as f64 / (a.s + a.r + b.s + b.r).max(1) as f64
}

pub fn home(rank: usize, ring: f64) -> (f64, f64) {
    let th = (4.0 * PI * rank as f64).sqrt();
    let rr = ring * (rank as f64 / PI).sqrt();
    (rr * th.cos(), rr * th.sin())
}

/// FNV-1a over UTF-16 code units, as spread.js' `hash01`
pub fn hash01(s: &str, salt: u32) -> f64 {
    let mut h: u32 = 2166136261 ^ salt;
    for c in s.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(16777619);
    }
    h as f64 / 4294967296.0
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub text: String,
    pub s: i64,
    pub r: i64,
    pub p: i64,
    pub q: i64,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub z: f64,
    pub zt: f64,
    pub k: f64,
    pub hw: f64,
    pub hh: f64,
    pub fx: f64,
    pub fy: f64,
    pub pinned: bool,
    pub placed: bool,
    /// index into `Spread::order`
    pub bucket: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Bucket {
    pub q: i64,
    pub members: Vec<usize>,
    pub n: usize,
    pub rank: usize,
    pub hx: f64,
    pub hy: f64,
    pub cx: f64,
    pub cy: f64,
    pub vx: f64,
    pub vy: f64,
    pub rad: f64,
    pub fx: f64,
    pub fy: f64,
}

pub struct Spread {
    pub p: Params,
    pub nodes: Vec<Node>,
    by_id: HashMap<String, usize>,
    /// buckets ascending by Q
    pub order: Vec<Bucket>,
    pub min_q: i64,
    pub dirty: bool,
    pub alpha: f64,
    pub energy: f64,
    sorted: Vec<usize>,
}

impl Spread {
    pub fn new(p: Params) -> Spread {
        Spread {
            p,
            nodes: Vec::new(),
            by_id: HashMap::new(),
            order: Vec::new(),
            min_q: 0,
            dirty: false,
            alpha: 1.0,
            energy: 0.0,
            sorted: Vec::new(),
        }
    }

    pub fn node(id: &str, text: &str, hw: f64, hh: f64) -> Node {
        let (s, r) = sums(text);
        Node {
            id: id.to_string(),
            text: text.to_string(),
            s,
            r,
            p: s,
            q: r,
            x: 0.0,
            y: 0.0,
            vx: 0.0,
            vy: 0.0,
            z: 0.0,
            zt: 0.0,
            k: 1.0,
            hw,
            hh,
            fx: 0.0,
            fy: 0.0,
            pinned: false,
            placed: false,
            bucket: 0,
        }
    }

    /// adds a node, returning its index (an existing node's if the id is taken)
    pub fn add(&mut self, n: Node) -> usize {
        if let Some(&i) = self.by_id.get(&n.id) {
            return i;
        }
        self.by_id.insert(n.id.clone(), self.nodes.len());
        self.nodes.push(n);
        self.dirty = true;
        self.wake(1.0);
        self.nodes.len() - 1
    }

    pub fn index(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(i) = self.by_id.remove(id) {
            self.nodes.remove(i);
            for v in self.by_id.values_mut() {
                if *v > i {
                    *v -= 1;
                }
            }
            self.dirty = true;
            self.wake(1.0);
        }
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.by_id.clear();
        self.order.clear();
        self.sorted.clear();
        self.dirty = true;
    }

    pub fn set_text(&mut self, i: usize, text: &str) {
        let (s, r) = sums(text);
        let n = &mut self.nodes[i];
        n.s = s;
        n.r = r;
        n.text = text.to_string();
        self.dirty = true;
        self.wake(1.0);
    }

    pub fn wake(&mut self, a: f64) {
        self.alpha = self.alpha.max(a);
    }

    pub fn asleep(&self) -> bool {
        self.alpha < 0.004 && self.energy < 0.002
    }

    pub fn rekey(&mut self) {
        let rv = self.p.reversed;
        let mut buckets: HashMap<i64, Bucket> = HashMap::new();
        for (i, n) in self.nodes.iter_mut().enumerate() {
            n.p = if rv { n.r } else { n.s };
            n.q = if rv { n.s } else { n.r };
            buckets
                .entry(n.q)
                .or_insert_with(|| Bucket {
                    q: n.q,
                    ..Default::default()
                })
                .members
                .push(i);
        }
        let mut qs: Vec<i64> = buckets.keys().copied().collect();
        qs.sort();
        self.min_q = qs.first().copied().unwrap_or(0);
        self.order = qs
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let mut b = buckets.remove(q).unwrap();
                b.n = b.members.len();
                b.rank = i;
                (b.hx, b.hy) = home(i, self.p.ring);
                b
            })
            .collect();
        let slot: HashMap<i64, usize> = self.order.iter().enumerate().map(|(i, b)| (b.q, i)).collect();
        for n in self.nodes.iter_mut() {
            n.bucket = slot[&n.q];
            n.zt = (n.q - self.min_q) as f64 * self.p.depth;
        }
        for b in &self.order {
            let placed: Vec<usize> = b.members.iter().copied().filter(|&m| self.nodes[m].placed).collect();
            let (mut cx, mut cy) = (b.hx, b.hy);
            if !placed.is_empty() {
                cx = placed.iter().map(|&m| self.nodes[m].x).sum::<f64>() / placed.len() as f64;
                cy = placed.iter().map(|&m| self.nodes[m].y).sum::<f64>() / placed.len() as f64;
            }
            let spread_r = 1.0 + (b.n as f64).sqrt() * 1.5;
            for &m in &b.members {
                let n = &mut self.nodes[m];
                if !n.placed {
                    let a = hash01(&n.id, 7) * PI * 2.0;
                    let rr = spread_r * hash01(&n.id, 13).sqrt();
                    n.x = cx + a.cos() * rr;
                    n.y = cy + a.sin() * rr;
                    n.z = n.zt;
                    n.vx = 0.0;
                    n.vy = 0.0;
                    n.placed = true;
                }
            }
        }
        self.sorted = (0..self.nodes.len()).collect();
        self.dirty = false;
        self.wake(1.0);
    }

    /// every pair whose x extents come within `margin`, by sweeping the nodes sorted by
    /// their left edge (the sort order is kept between calls, like the JS array)
    fn pairs(&mut self, margin: f64, mut f: impl FnMut(&mut [Node], usize, usize)) {
        let nodes = &mut self.nodes;
        let left = |n: &Node| n.x - n.hw * n.k;
        self.sorted.sort_by(|&a, &b| {
            left(&nodes[a])
                .partial_cmp(&left(&nodes[b]))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in 0..self.sorted.len() {
            let a = self.sorted[i];
            let right = nodes[a].x + nodes[a].hw * nodes[a].k + margin;
            for j in i + 1..self.sorted.len() {
                let b = self.sorted[j];
                if nodes[b].x - nodes[b].hw * nodes[b].k > right {
                    break;
                }
                f(nodes, a, b);
            }
        }
    }

    pub fn step(&mut self, dt: f64) -> f64 {
        if self.dirty {
            self.rekey();
        }
        let p = self.p;
        let d = p.d;
        if self.nodes.is_empty() {
            self.alpha = 0.0;
            self.energy = 0.0;
            return 0.0;
        }
        let a = self.alpha;

        for n in self.nodes.iter_mut() {
            if !n.pinned {
                n.z += (n.zt - n.z) * 0.08;
            }
            n.k = d / (d + n.z).max(1e-3);
            n.fx = 0.0;
            n.fy = 0.0;
        }

        for b in self.order.iter_mut() {
            let (mut cx, mut cy, mut vx, mut vy, mut area) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for &m in &b.members {
                let n = &self.nodes[m];
                cx += n.x;
                cy += n.y;
                vx += n.vx;
                vy += n.vy;
                area += 4.0 * n.hw * n.hh * n.k * n.k;
            }
            let bn = b.n as f64;
            b.cx = cx / bn;
            b.cy = cy / bn;
            b.vx = vx / bn;
            b.vy = vy / bn;
            b.rad = area.sqrt() * 0.6;
            b.fx = 0.0;
            b.fy = 0.0;
        }

        for n in self.nodes.iter_mut() {
            if n.pinned {
                continue;
            }
            let b = &self.order[n.bucket];
            if b.n > 1 {
                n.fx += p.gravity * (b.cx - n.x);
                n.fy += p.gravity * (b.cy - n.y);
            }
            n.fx += p.anchor * (b.hx - b.cx);
            n.fy += p.anchor * (b.hy - b.cy);
        }

        if p.group_attract != 0.0 || p.group_repel != 0.0 || p.group_align != 0.0 {
            let g = &mut self.order;
            for i in 0..g.len() {
                for j in i + 1..g.len() {
                    let (c, e) = (&g[i], &g[j]);
                    let (mut dx, mut dy) = (e.cx - c.cx, e.cy - c.cy);
                    let mut dd = dx.hypot(dy);
                    if dd < 1e-6 {
                        let ang = (c.q as f64 * 0.618 + e.q as f64) % (PI * 2.0);
                        dx = ang.cos();
                        dy = ang.sin();
                        dd = 1e-6;
                    } else {
                        dx /= dd;
                        dy /= dd;
                    }
                    let aff = 1.0 / (1.0 + (e.q - c.q).abs() as f64 / p.qscale);
                    let f = p.group_attract * aff * (dd - c.rad - e.rad)
                        - p.group_repel * (c.n * e.n) as f64 / (dd * dd + 1.0);
                    let ax = p.group_align * aff * (e.vx - c.vx);
                    let ay = p.group_align * aff * (e.vy - c.vy);
                    g[i].fx += f * dx + ax;
                    g[i].fy += f * dy + ay;
                    g[j].fx -= f * dx + ax;
                    g[j].fy -= f * dy + ay;
                }
            }
        }
        for b in &self.order {
            if b.fx == 0.0 && b.fy == 0.0 {
                continue;
            }
            let (gx, gy) = (b.fx / b.n as f64, b.fy / b.n as f64);
            for &m in &b.members {
                let n = &mut self.nodes[m];
                if !n.pinned {
                    n.fx += gx;
                    n.fy += gy;
                }
            }
        }

        if p.repel != 0.0 && p.range > 0.0 {
            self.pairs(p.range, |nodes, ia, ib| {
                let (na, nb) = (&nodes[ia], &nodes[ib]);
                let (dx, dy) = (na.x - nb.x, na.y - nb.y);
                let gx = dx.abs() - (na.hw * na.k + nb.hw * nb.k);
                let gy = dy.abs() - (na.hh * na.k + nb.hh * nb.k);
                let gap = gx.max(0.0).hypot(gy.max(0.0));
                if gap >= p.range {
                    return;
                }
                let len = dx.hypot(dy);
                let (ux, uy) = if len < 1e-6 {
                    let ang = hash01(&format!("{}{}", na.id, nb.id), 3) * PI * 2.0;
                    (ang.cos(), ang.sin())
                } else {
                    (dx / len, dy / len)
                };
                let m = p.repel * (1.0 + p.atbash * atb(na, nb)) * (1.0 - gap / p.range) / ((gap + 1.0) * (gap + 1.0));
                if !nodes[ia].pinned {
                    nodes[ia].fx += m * ux;
                    nodes[ia].fy += m * uy;
                }
                if !nodes[ib].pinned {
                    nodes[ib].fx -= m * ux;
                    nodes[ib].fy -= m * uy;
                }
            });
        }

        let keep = 1.0 - p.damping;
        let (mut e, mut moving) = (0.0, 0usize);
        for n in self.nodes.iter_mut() {
            if n.pinned {
                n.vx = 0.0;
                n.vy = 0.0;
                continue;
            }
            n.vx = (n.vx + a * n.fx * dt) * keep;
            n.vy = (n.vy + a * n.fy * dt) * keep;
            let sp = n.vx.hypot(n.vy);
            if sp > p.vmax {
                n.vx *= p.vmax / sp;
                n.vy *= p.vmax / sp;
            }
            n.x += n.vx * dt;
            n.y += n.vy * dt;
            e += n.vx * n.vx + n.vy * n.vy;
            moving += 1;
        }

        for _ in 0..3 {
            self.pairs(p.pad, |nodes, ia, ib| {
                if nodes[ia].pinned && nodes[ib].pinned {
                    return;
                }
                let (na, nb) = (&nodes[ia], &nodes[ib]);
                let (dx, dy) = (nb.x - na.x, nb.y - na.y);
                let ox = na.hw * na.k + nb.hw * nb.k + p.pad - dx.abs();
                let oy = na.hh * na.k + nb.hh * nb.k + p.pad - dy.abs();
                if ox <= 0.0 || oy <= 0.0 {
                    return;
                }
                let wa = if na.pinned {
                    0.0
                } else if nb.pinned {
                    1.0
                } else {
                    0.5
                };
                let wb = 1.0 - wa;
                let pair_id = format!("{}{}", na.id, nb.id);
                if ox < oy {
                    let sg = if dx < 0.0 {
                        -1.0
                    } else if dx > 0.0 {
                        1.0
                    } else if hash01(&pair_id, 5) < 0.5 {
                        -1.0
                    } else {
                        1.0
                    };
                    nodes[ia].x -= sg * ox * wa;
                    nodes[ib].x += sg * ox * wb;
                } else {
                    let sg = if dy < 0.0 {
                        -1.0
                    } else if dy > 0.0 {
                        1.0
                    } else if hash01(&pair_id, 9) < 0.5 {
                        -1.0
                    } else {
                        1.0
                    };
                    nodes[ia].y -= sg * oy * wa;
                    nodes[ib].y += sg * oy * wb;
                }
            });
        }

        self.energy = if moving > 0 { e / moving as f64 } else { 0.0 };
        self.alpha -= self.alpha * p.cool;
        self.energy
    }

    /// steps until still, or the step or time budget runs out; returns the steps taken
    pub fn settle(&mut self, max_steps: usize, ms: u128) -> usize {
        let t0 = std::time::Instant::now();
        let mut i = 0;
        while i < max_steps && !self.asleep() && t0.elapsed().as_millis() < ms {
            self.step(1.0 / 60.0);
            i += 1;
        }
        i
    }

    #[cfg(test)]
    pub fn overlaps(&self, tol: f64) -> usize {
        let n = &self.nodes;
        let mut c = 0;
        for i in 0..n.len() {
            for j in i + 1..n.len() {
                let (a, b) = (&n[i], &n[j]);
                if a.hw * a.k + b.hw * b.k - (a.x - b.x).abs() > tol
                    && a.hh * a.k + b.hh * b.k - (a.y - b.y).abs() > tol
                {
                    c += 1;
                }
            }
        }
        c
    }
}

/// remem's `seedCLR`: the hue of a bucket's underline from its Q
pub fn seed_hue(q: i64) -> f32 {
    let mut hash: i32 = 0;
    for c in q.to_string().encode_utf16() {
        hash = (c as i32).wrapping_add(hash.wrapping_shl(5).wrapping_sub(hash));
    }
    (hash % 360).unsigned_abs() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn reference() -> Value {
        serde_json::from_str(include_str!("spread_reference.json")).unwrap()
    }

    fn build(case: &Value) -> Spread {
        let mut p = Params::default();
        if case["reversed"].as_bool().unwrap_or(false) {
            p.reversed = true;
        }
        let mut sp = Spread::new(p);
        for n in case["nodes"].as_array().unwrap() {
            let mut node = Spread::node(
                n[0].as_str().unwrap(),
                n[1].as_str().unwrap(),
                n[2].as_f64().unwrap(),
                n[3].as_f64().unwrap(),
            );
            if n[4].as_bool().unwrap_or(false) {
                node.pinned = true;
                node.placed = true;
                node.x = n[5].as_f64().unwrap();
                node.y = n[6].as_f64().unwrap();
            }
            sp.add(node);
        }
        sp
    }

    #[test]
    fn matches_spread_js() {
        let r = reference();
        for case in r["cases"].as_array().unwrap() {
            let mut sp = build(case);
            let steps = case["steps"].as_u64().unwrap();
            for _ in 0..steps {
                sp.step(1.0 / 60.0);
            }
            let want = case["out"].as_array().unwrap();
            for (n, w) in sp.nodes.iter().zip(want) {
                for (got, key) in [(n.x, "x"), (n.y, "y"), (n.z, "z")] {
                    let w = w[key].as_f64().unwrap();
                    assert!(
                        (got - w).abs() < 1e-6 * (1.0 + w.abs()),
                        "{} {}: {} vs {} ({})",
                        n.id,
                        key,
                        got,
                        w,
                        case["name"]
                    );
                }
            }
            assert!((sp.alpha - case["alpha"].as_f64().unwrap()).abs() < 1e-9);
            assert!((sp.energy - case["energy"].as_f64().unwrap()).abs() < 1e-6 * (1.0 + sp.energy));
            assert_eq!(sp.order.len(), case["buckets"].as_u64().unwrap() as usize);
        }
    }

    #[test]
    fn helpers_match_spread_js() {
        let r = reference();
        for h in r["helpers"].as_array().unwrap() {
            let t = h["t"].as_str().unwrap();
            let (s, rr) = sums(t);
            assert_eq!((s, rr), (h["s"].as_i64().unwrap(), h["r"].as_i64().unwrap()), "{}", t);
            let m = metrics(t);
            assert_eq!(m.v, h["v"].as_i64().unwrap(), "{}", t);
            assert_eq!(m.c, h["c"].as_i64().unwrap(), "{}", t);
            assert_eq!(m.jump, h["jump"].as_i64().unwrap(), "{}", t);
            assert_eq!(m.w, h["w"].as_u64().unwrap() as usize, "{}", t);
            assert!((hash01(t, 7) - h["hash"].as_f64().unwrap()).abs() < 1e-15);
            assert_eq!(seed_hue(s), h["hue"].as_f64().unwrap() as f32, "{}", t);
        }
        let (x, y) = home(5, 10.0);
        assert!(
            (x - r["home5"][0].as_f64().unwrap()).abs() < 1e-12 && (y - r["home5"][1].as_f64().unwrap()).abs() < 1e-12
        );
    }

    #[test]
    fn settles_without_overlaps() {
        let mut sp = Spread::new(Params::default());
        for (i, t) in [
            "listen", "silent", "enlist", "dog", "god", "light", "wisdom", "truth", "love", "seven",
        ]
        .iter()
        .enumerate()
        {
            sp.add(Spread::node(&i.to_string(), t, 1.5, 0.4));
        }
        sp.settle(3000, 2000);
        assert_eq!(sp.overlaps(0.05), 0);
        // anagrams share a bucket
        let b = |t: &str| sp.nodes[sp.nodes.iter().position(|n| n.text == t).unwrap()].bucket;
        assert_eq!(b("listen"), b("silent"));
        assert_ne!(b("listen"), b("dog"));
    }
}

/// the triangle a text makes: sides s, r and t = ⌊√(s·r)⌋, the corners laid out with r
/// along the bottom; `None` when the three can't close (one longer than the other two)
#[derive(Debug, Clone, PartialEq)]
pub struct Triangle {
    pub sides: [i64; 3],
    /// corners: (0, 0), (r, 0) and the apex, in the sides' units
    pub apex: (f64, f64),
    /// the angles opposite s, r and t, in degrees
    pub angles: [f64; 3],
}

pub fn triangle(text: &str) -> Option<Triangle> {
    let (s, r) = sums(text);
    if s <= 0 || r <= 0 {
        return None;
    }
    let t = ((s * r) as f64).sqrt().floor() as i64;
    let (a, b, c) = (s as f64, r as f64, t as f64);
    if a + c <= b || b + c <= a || a + b <= c {
        return None;
    }
    // apex: t from the left corner, s from the right one
    let x = (c * c - a * a + b * b) / (2.0 * b);
    let y = (c * c - x * x).max(0.0).sqrt();
    let angle = |opp: f64, p: f64, q: f64| {
        ((p * p + q * q - opp * opp) / (2.0 * p * q))
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    };
    Some(Triangle {
        sides: [s, r, t],
        apex: (x, y),
        angles: [angle(a, b, c), angle(b, a, c), angle(c, a, b)],
    })
}

#[cfg(test)]
mod triangle_tests {
    use super::*;

    #[test]
    fn truth_and_flat_ones() {
        let t = triangle("truth").unwrap();
        assert_eq!(t.sides, [87, 48, 64]);
        assert!((t.angles.iter().sum::<f64>() - 180.0).abs() < 1e-9);
        assert!(t.apex.1 > 0.0);
        // a: s = 1, r = 26 and t = 5 don't close
        assert!(triangle("a").is_none());
        assert!(triangle("").is_none());
    }
}
