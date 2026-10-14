// the colours: every one has a name (as in the desktop app's settings) and a css variable;
// sand is the old admin panel's, the others start from it

export const ROLES = [
  ['window', '--window'], ['bars', '--bars'], ['bar text', '--bar-text'], ['bar labels', '--bar-dim'],
  ['cards', '--cards'], ['writing pad', '--pad'], ['buttons', '--buttons'], ['accent', '--accent'],
  ['text', '--text'], ['dim text', '--dim'], ['links', '--link'], ['code', '--code'], ['space room', '--room'],
];

const SAND = {
  window: '#927a4f', bars: '#404040', 'bar text': '#ebebeb', 'bar labels': '#9e9e9e', cards: '#ffffff',
  'writing pad': '#dbd4c8', buttons: '#c4b28f', accent: '#c4a265', text: '#2e2e2e', 'dim text': '#6b6b6b',
  links: '#86592a', code: '#e9e5de', 'space room': '#e4d4be',
};

export const PRESETS = {
  sand: {},
  night: {
    window: '#1d1b19', bars: '#2b2825', 'bar text': '#e8e2d8', 'bar labels': '#9a9184', cards: '#2f2c29',
    'writing pad': '#3a3632', buttons: '#6b5d48', accent: '#a8875a', text: '#e6dfd4', 'dim text': '#a39a8d',
    links: '#d9a066', code: '#26231f', 'space room': '#26221e',
  },
  paper: {
    window: '#e9e6df', bars: '#d9d4ca', 'bar text': '#2e2b27', 'bar labels': '#77706a', cards: '#fbfaf7',
    'writing pad': '#f2efe8', buttons: '#d6cfc2', accent: '#b9a688', text: '#2b2926', 'dim text': '#7a746c',
    links: '#8a4f22', code: '#ece8df', 'space room': '#f1ede4',
  },
  sea: {
    window: '#4d6e78', bars: '#2c3b40', cards: '#f6fafa', 'writing pad': '#d5e2e4', buttons: '#9dbcc2',
    accent: '#6f9ea7', links: '#2d6470', code: '#e3eced', 'space room': '#d8e4e3',
  },
};

const STORE = 'anend.colours';

function load() {
  try {
    return JSON.parse(localStorage.getItem(STORE)) || { preset: 'sand', own: {} };
  } catch {
    return { preset: 'sand', own: {} };
  }
}

let state = load();

export function colours() {
  return { ...SAND, ...(PRESETS[state.preset] || {}), ...state.own };
}

export function preset() {
  return state.preset;
}

export function apply() {
  const c = colours();
  const root = document.documentElement;
  for (const [name, v] of ROLES) root.style.setProperty(v, c[name]);
  // the ink on buttons follows how light they are
  root.style.setProperty('--on-button', light(c.buttons) ? '#1f1f1f' : '#f1ebe2');
  root.style.setProperty('--room-ink', light(c['space room']) ? 'hsl(0 0% 14%)' : 'hsl(35 25% 88%)');
  root.dataset.dark = light(c.cards) ? '' : 'dark';
  document.querySelector('meta[name=theme-color]')?.setAttribute('content', c.bars);
  window.dispatchEvent(new CustomEvent('colours'));
}

export function light(hex) {
  const n = parseInt(String(hex).slice(1), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > 140;
}

function save() {
  try {
    localStorage.setItem(STORE, JSON.stringify(state));
  } catch {
    /* private window */
  }
}

export function choose(name) {
  state = { preset: name, own: {} };
  save();
  apply();
}

export function set(role, hex) {
  state.own[role] = hex;
  save();
  apply();
}

export function reset(role) {
  delete state.own[role];
  save();
  apply();
}
