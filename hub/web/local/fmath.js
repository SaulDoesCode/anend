// the floating point the spread needs, made the same everywhere: JavaScript leaves
// Math.hypot, Math.sin and Math.cos to each engine, so the hub (core/src/fmath.rs) and
// this use the same code: hypot is glibc's own kernel (e_hypot.c), sin and cos are
// correctly rounded in double-double.

// ---------------------------------------------------------------- hypot (glibc 2.35+)

const SCALE = 2 ** -600;
const LARGE_VAL = 2 ** 511;
const TINY_VAL = 2 ** -511;
const EPS = 2 ** -54;

function kernel(ax, ay) {
  let h = Math.sqrt(ax * ax + ay * ay);
  let t1;
  let t2;
  if (h <= 2.0 * ay) {
    const delta = h - ay;
    t1 = ax * (2.0 * delta - ax);
    t2 = (delta - 2.0 * (ax - ay)) * delta;
  } else {
    const delta = h - ax;
    t1 = 2.0 * delta * (ax - 2.0 * ay);
    t2 = (4.0 * delta - ay) * ay + delta * delta;
  }
  h -= (t1 + t2) / (2.0 * h);
  return h;
}

export function hypot(x, y) {
  if (!Number.isFinite(x) || !Number.isFinite(y)) return Math.abs(x) === Infinity || Math.abs(y) === Infinity ? Infinity : NaN;
  x = Math.abs(x);
  y = Math.abs(y);
  const ax = x < y ? y : x;
  const ay = x < y ? x : y;
  if (ax > LARGE_VAL) {
    if (ay <= ax * EPS) return ax + ay;
    return kernel(ax * SCALE, ay * SCALE) / SCALE;
  }
  if (ay < TINY_VAL) {
    if (ax >= ay / EPS) return ax + ay;
    return kernel(ax / SCALE, ay / SCALE) * SCALE;
  }
  if (ay <= ax * EPS) return ax + ay;
  return kernel(ax, ay);
}

// ---------------------------------------------------------------- double-double

const SPLIT = 134217729; // 2^27 + 1

function twoSum(a, b) {
  const s = a + b;
  const bb = s - a;
  return [s, a - (s - bb) + (b - bb)];
}
function quickTwoSum(a, b) {
  const s = a + b;
  return [s, b - (s - a)];
}
function twoProd(a, b) {
  const p = a * b;
  let t = SPLIT * a;
  const ah = t - (t - a);
  const al = a - ah;
  t = SPLIT * b;
  const bh = t - (t - b);
  const bl = b - bh;
  return [p, ah * bh - p + ah * bl + al * bh + al * bl];
}
function add(a, b) {
  let [s, e] = twoSum(a[0], b[0]);
  const [t, f] = twoSum(a[1], b[1]);
  e += t;
  [s, e] = quickTwoSum(s, e);
  e += f;
  return quickTwoSum(s, e);
}
function mul(a, b) {
  const [p, e] = twoProd(a[0], b[0]);
  return quickTwoSum(p, e + (a[0] * b[1] + a[1] * b[0]));
}
function mulD(a, d) {
  const [p, e] = twoProd(a[0], d);
  return quickTwoSum(p, e + a[1] * d);
}
function divD(a, d) {
  const q1 = a[0] / d;
  const r = add(a, mulD([q1, 0], -d));
  return quickTwoSum(q1, r[0] / d);
}

// 1/n!, and π/2 in three parts
const INV_FACT = [[1, 0]];
for (let i = 1, f = [1, 0]; i <= 30; i++) INV_FACT.push((f = divD(f, i)));
const PIO2 = [1.5707963267948966, 6.123233995736766e-17, -1.4973849048591698e-33];

// x - n·π/2 for |x| up to a few thousand (the spread's angles are far smaller)
function reduce(x) {
  const n = Math.round(x / PIO2[0]);
  let r = [x, 0];
  for (const p of PIO2) r = add(r, mulD([p, 0], -n));
  return [((n % 4) + 4) % 4, r];
}

function series(r, odd) {
  const r2 = mul(r, r);
  let t = odd ? r : [1, 0];
  let s = t;
  for (let i = odd ? 3 : 2, sign = -1; i <= 30; i += 2, sign = -sign) {
    t = mul(t, r2);
    const term = mul(t, INV_FACT[i]);
    s = add(s, sign < 0 ? [-term[0], -term[1]] : term);
    if (Math.abs(term[0]) < 1e-36) break;
  }
  return s[0] + s[1];
}

export function sin(x) {
  if (!Number.isFinite(x)) return NaN;
  if (Math.abs(x) < 2 ** -26 || Math.abs(x) > 2 ** 20) return Math.abs(x) < 1 ? x : Math.sin(x);
  const [q, r] = reduce(x);
  const v = series(r, q % 2 === 0);
  return q >= 2 ? -v : v;
}

export function cos(x) {
  if (!Number.isFinite(x)) return NaN;
  if (Math.abs(x) < 2 ** -27) return 1;
  if (Math.abs(x) > 2 ** 20) return Math.cos(x);
  const [q, r] = reduce(x);
  const v = series(r, q % 2 === 1);
  return q === 1 || q === 2 ? -v : v;
}
