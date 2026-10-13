// the app's calls: {id, op, args} over the websocket, answers matched by id; posts to
// /app/api/{op} while the socket can't open

export class ApiError extends Error {}

export class Api extends EventTarget {
  constructor(base = '/app') {
    super();
    this.base = base;
    this.next = 1;
    this.waiting = new Map();
    this.ws = null;
    this.opening = null;
    this.backoff = 400;
    this.failed = false; // the socket hasn't opened lately: post instead
    this.connect();
  }

  state(s) {
    this.dispatchEvent(new CustomEvent('state', { detail: s }));
  }

  connect() {
    const url = `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}${this.base}/ws`;
    let ws;
    try {
      ws = new WebSocket(url);
    } catch {
      this.failed = true;
      this.retry();
      return;
    }
    this.ws = ws;
    this.opening = new Promise((resolve) => {
      ws.onopen = () => {
        this.failed = false;
        this.backoff = 400;
        this.state('online');
        resolve(true);
      };
      ws.onerror = () => resolve(false);
      ws.onclose = () => {
        resolve(false);
        if (this.ws !== ws) return;
        this.ws = null;
        this.failed = true;
        this.state('reconnecting');
        // what was in flight goes again as posts
        for (const [id, w] of this.waiting) {
          this.waiting.delete(id);
          this.post(w.op, w.args).then(w.resolve, w.reject);
        }
        this.retry();
      };
    });
    ws.onmessage = (m) => {
      let a;
      try {
        a = JSON.parse(m.data);
      } catch {
        return;
      }
      const w = this.waiting.get(a.id);
      if (!w) return;
      this.waiting.delete(a.id);
      if (a.ok) w.resolve(a.data);
      else w.reject(this.error(a.error));
    };
  }

  retry() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.connect(), this.backoff);
    this.backoff = Math.min(this.backoff * 2, 15000);
  }

  error(msg) {
    const e = new ApiError(msg || 'something went wrong');
    if (/log in to use the app|waiting for an admin/.test(msg || '')) {
      this.dispatchEvent(new CustomEvent('login', { detail: msg }));
    }
    return e;
  }

  async call(op, args = {}) {
    if (!this.failed && this.ws && this.ws.readyState === WebSocket.CONNECTING) {
      // give a socket that's opening a moment
      await Promise.race([this.opening, new Promise((r) => setTimeout(r, 2000))]);
    }
    const ws = this.ws;
    if (ws && ws.readyState === WebSocket.OPEN) {
      const id = this.next++;
      return new Promise((resolve, reject) => {
        this.waiting.set(id, { resolve, reject, op, args });
        ws.send(JSON.stringify({ id, op, args }));
      });
    }
    return this.post(op, args);
  }

  async post(op, args) {
    let r;
    try {
      r = await fetch(`${this.base}/api/${encodeURIComponent(op)}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(args ?? {}),
        credentials: 'same-origin',
      });
    } catch {
      this.state('offline');
      throw new ApiError("can't reach the hub");
    }
    let a;
    try {
      a = await r.json();
    } catch {
      throw new ApiError(`the hub answered ${r.status}`);
    }
    if (this.failed) this.state('posting');
    if (a.ok) return a.data;
    throw this.error(a.error);
  }

  // the non-call endpoints (saves in and out, the hub): json answers in the same envelope
  async send(path, init = {}) {
    let r;
    try {
      r = await fetch(path, { credentials: 'same-origin', ...init });
    } catch {
      throw new ApiError("can't reach the hub");
    }
    let a = null;
    try {
      a = await r.json();
    } catch {
      /* not json */
    }
    if (!r.ok) throw this.error((a && a.error) || `the hub answered ${r.status}`);
    if (a && a.ok === false) throw this.error(a.error);
    return a && 'data' in a ? a.data : a;
  }
}

export const api = new Api();
