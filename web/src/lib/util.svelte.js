// Formatting helpers, a shared clock, and toasts.

export function bytes(n) {
  if (n == null) return '—';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 / 1024).toFixed(n < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

export function duration(ms) {
  if (ms == null || ms < 0) return '--:--:--';
  const s = Math.floor(ms / 1000);
  const pad = (x) => String(x).padStart(2, '0');
  const d = Math.floor(s / 86400);
  const hms = `${pad(Math.floor(s / 3600) % 24)}:${pad(Math.floor(s / 60) % 60)}:${pad(s % 60)}`;
  return d ? `${d}d ${hms}` : hms;
}

export const STATE_LABEL = {
  running: 'Running',
  starting: 'Starting',
  stopping: 'Stopping',
  stopped: 'Stopped',
  crashed: 'Crashed',
};

class Clock {
  now = $state(Date.now());
  constructor() {
    setInterval(() => (this.now = Date.now()), 1000);
  }
}
export const clock = new Clock();

class Toasts {
  items = $state([]);
  #id = 0;
  push(text, kind = 'info', ms = 4500) {
    const id = ++this.#id;
    this.items.push({ id, text, kind });
    setTimeout(() => this.dismiss(id), ms);
  }
  error(e) {
    this.push(e?.message || String(e), 'err', 7000);
  }
  dismiss(id) {
    this.items = this.items.filter((t) => t.id !== id);
  }
}
export const toasts = new Toasts();
