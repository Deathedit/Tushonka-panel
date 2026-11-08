// One WebSocket for the whole app: server status (text frames) and console output (binary frames).
// Output is kept in a client-side buffer so the console page can show history when it mounts.

const MAX_BUFFER = 1024 * 1024;

class Live {
  status = $state(null);
  connected = $state(false);

  #ws = null;
  #chunks = [];
  #size = 0;
  #listeners = new Set();
  #retry = 0;
  #timer = null;
  #stopped = true;

  start() {
    this.#stopped = false;
    this.#connect();
  }

  stop() {
    this.#stopped = true;
    clearTimeout(this.#timer);
    this.#ws?.close();
    this.#ws = null;
  }

  /** Subscribe to console output. `fn({ type: 'data', data })` or `fn({ type: 'reset' })`. */
  subscribe(fn) {
    this.#listeners.add(fn);
    return () => this.#listeners.delete(fn);
  }

  /** Everything received since the last (re)connect, oldest first. */
  history() {
    return this.#chunks;
  }

  send(msg) {
    if (this.#ws?.readyState === WebSocket.OPEN) this.#ws.send(JSON.stringify(msg));
  }

  input(data) {
    this.send({ type: 'input', data });
  }

  resize(cols, rows) {
    this.send({ type: 'resize', cols, rows });
  }

  #emit(ev) {
    for (const fn of this.#listeners) fn(ev);
  }

  #connect() {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/api/ws`);
    ws.binaryType = 'arraybuffer';
    this.#ws = ws;
    let first = true;

    ws.onopen = () => {
      this.connected = true;
      this.#retry = 0;
    };
    ws.onmessage = (e) => {
      if (typeof e.data === 'string') {
        const msg = JSON.parse(e.data);
        if (msg.type === 'status') this.status = msg.status;
        return;
      }
      // The server replays its scrollback on every connect: start the local copy over.
      if (first) {
        first = false;
        this.#chunks = [];
        this.#size = 0;
        this.#emit({ type: 'reset' });
      }
      const data = new Uint8Array(e.data);
      this.#chunks.push(data);
      this.#size += data.length;
      while (this.#size > MAX_BUFFER && this.#chunks.length > 1) {
        this.#size -= this.#chunks.shift().length;
      }
      this.#emit({ type: 'data', data });
    };
    ws.onclose = () => {
      this.connected = false;
      if (this.#ws === ws) this.#ws = null;
      if (this.#stopped) return;
      const delay = Math.min(10000, 500 * 2 ** this.#retry++);
      this.#timer = setTimeout(() => this.#connect(), delay);
    };
  }
}

export const live = new Live();
