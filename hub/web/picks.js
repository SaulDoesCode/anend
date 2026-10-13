// the list and the list candidate picked in the memory view or the space (Shift+click,
// Ctrl+click), and the words typed in the bar that act on them

export const picks = { list: null, candidate: null };

const listeners = new Set();
export function onPick(f) {
  listeners.add(f);
}
function changed() {
  for (const f of listeners) f(picks);
}

export function pick(kind, hit) {
  picks[kind] = hit;
  changed();
}

// a line saying what's picked
export function pickLine() {
  const p = [];
  if (picks.list) p.push(`list: ${picks.list.text}`);
  if (picks.candidate) p.push(`candidate: ${picks.candidate.text}`);
  return p.join(' · ');
}

export const COMMANDS = ['Enlist', 'List', 'Delist', 'Unlist'];

// runs a list command; gives back the hits to show (List), or null when it isn't one
export async function listCommand(app, word) {
  if (!COMMANDS.includes(word)) return null;
  const { list, candidate } = picks;
  if (!list) {
    app.status('Shift+click a text to pick the list first', true);
    return [];
  }
  if ((word === 'Enlist' || word === 'Delist') && !candidate) {
    app.status('Ctrl+click a text to pick the candidate first', true);
    return [];
  }
  try {
    if (word === 'Enlist') {
      await app.call('lists.enlist', { list: list.key, member: candidate.key });
      app.status(`“${candidate.text}” is in the list “${list.text}”`);
    } else if (word === 'Delist') {
      await app.call('lists.delist', { list: list.key, member: candidate.key });
      app.status(`“${candidate.text}” left the list “${list.text}”`);
    } else if (word === 'Unlist') {
      const r = await app.call('lists.unlist', { list: list.key });
      app.status(`the list “${list.text}” is gone (${r.removed} members)`);
      pick('list', null);
      return [];
    }
    const st = await app.call('lists.stats');
    const l = st.lists.find((x) => x.list === list.key);
    return l ? [list, ...l.members] : [list];
  } catch {
    return [];
  }
}
