// where an offline app keeps its saves: one IndexedDB object store of saves by name,
// each {name, bundle, allocator, modified}, and which one was open under `.open`.
// Only local/index.js uses it, and only when no hub served the page.

const DB = 'anend';
const STORE = 'saves';
const OPEN = '.open';

const done = (req) =>
  new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });

export async function openDb() {
  const req = indexedDB.open(DB, 1);
  req.onupgradeneeded = () => req.result.createObjectStore(STORE);
  return done(req);
}

/// everything kept: {saves: [record], open}
export async function load(db) {
  const tx = db.transaction(STORE, 'readonly');
  const st = tx.objectStore(STORE);
  const [keys, values] = await Promise.all([done(st.getAllKeys()), done(st.getAll())]);
  const saves = [];
  let open = null;
  keys.forEach((k, i) => {
    if (k === OPEN) open = values[i];
    else saves.push(values[i]);
  });
  return { saves, open };
}

/// what the engine says changed, in one transaction
export function write(db, { put, gone, open }) {
  if (!put.length && !gone.length && open === null) return Promise.resolve();
  const tx = db.transaction(STORE, 'readwrite');
  const st = tx.objectStore(STORE);
  for (const r of put) st.put(r, r.name);
  for (const n of gone) st.delete(n);
  if (open !== null) st.put(open, OPEN);
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error);
  });
}
