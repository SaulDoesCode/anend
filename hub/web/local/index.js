// the app without a hub: the same calls answered by the engine in this page, the saves
// kept in IndexedDB, written through after every call that changed something.
// api.js loads this only when the page didn't come from a hub (or with ?offline).

import { Engine } from './engine.js';
import * as idb from './idb.js';

// api.js hands its error class over (importing api.js here would wait on its own probe)
let ApiError = Error;

export class LocalApi extends EventTarget {
  constructor(engine, db) {
    super();
    this.offline = true;
    this.engine = engine;
    this.db = db; // null: IndexedDB isn't there, nothing outlives the page
    this.writing = Promise.resolve();
  }

  async call(op, args = {}) {
    let data;
    try {
      // through JSON, as over the wire: the engine never shares objects with the views
      data = this.engine.call(op, JSON.parse(JSON.stringify(args ?? {})));
    } catch (e) {
      throw new ApiError(e.message || String(e));
    } finally {
      this.flush();
    }
    return data === undefined ? null : JSON.parse(JSON.stringify(data));
  }

  flush() {
    const changed = this.engine.records();
    if (!this.db) return this.writing;
    this.writing = this.writing
      .then(() => idb.write(this.db, changed))
      .catch((e) => this.dispatchEvent(new CustomEvent('state', { detail: `not kept: ${e && e.message}` })));
    return this.writing;
  }

  // the hub's own endpoints (saves as files, the hub's saves, sharing)
  async send() {
    throw new ApiError('that needs the app served by a hub');
  }
}

export async function offline(errors = Error) {
  ApiError = errors;
  let db = null;
  let kept = {};
  try {
    db = await idb.openDb();
    kept = await idb.load(db);
  } catch {
    db = null;
  }
  const api = new LocalApi(new Engine(kept), db);
  api.kept = !!db;
  return api;
}
