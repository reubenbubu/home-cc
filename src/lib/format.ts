// Turns numbers from the back end into short text for the screens.

/** "12 s ago", "3 min ago", "yesterday". Both times are Unix seconds. */
export function relTime(ts: number | null | undefined, now: number): string {
  if (ts == null) return "never";
  const d = Math.max(0, Math.floor(now - ts));
  if (d < 5) return "just now";
  if (d < 60) return `${d} s ago`;
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  if (d < 2 * 86400) return "yesterday";
  if (d < 30 * 86400) return `${Math.floor(d / 86400)} days ago`;
  return new Date(ts * 1000).toLocaleDateString();
}

/** "4 d 3 h", "3 h 12 min", "12 min", "45 s". */
export function uptime(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs) || secs < 0) return "–";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d} d ${h} h`;
  if (h > 0) return `${h} h ${m} min`;
  if (m > 0) return `${m} min`;
  return `${Math.floor(secs)} s`;
}

const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

function unitFor(n: number): number {
  let i = 0;
  while (n >= 1024 && i < UNITS.length - 1) {
    n /= 1024;
    i++;
  }
  return i;
}

function inUnit(n: number, unit: number): string {
  const v = n / 1024 ** unit;
  return v >= 100 || unit === 0 ? String(Math.round(v)) : v.toFixed(1);
}

/** "1.8 TB", "512 GB". Counts in steps of 1024, the way Windows shows sizes. */
export function bytes(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n) || n < 0) return "–";
  const u = unitFor(n);
  return `${inUnit(n, u)} ${UNITS[u]}`;
}

/** "42.1 / 48.0 GB": both numbers in the unit of the total. */
export function usedOfTotal(used: number, total: number): string {
  if (!Number.isFinite(used) || !Number.isFinite(total) || total <= 0) return "–";
  const u = unitFor(total);
  return `${inUnit(Math.max(0, used), u)} / ${inUnit(total, u)} ${UNITS[u]}`;
}

/** Response time: "0.4 ms", "12 ms". */
export function ms(rtt: number | null | undefined): string {
  if (rtt == null || !Number.isFinite(rtt)) return "–";
  if (rtt < 0.1) return "<0.1 ms";
  return rtt < 10 ? `${rtt.toFixed(1)} ms` : `${Math.round(rtt)} ms`;
}

/** Share of a total as 0 to 100, or null when it can't be worked out. */
export function share(used: number | null | undefined, total: number | null | undefined): number | null {
  if (used == null || total == null || !Number.isFinite(used) || !Number.isFinite(total) || total <= 0) return null;
  return Math.min(100, Math.max(0, (used / total) * 100));
}

export function pct(n: number | null | undefined): string {
  return n == null || !Number.isFinite(n) ? "–" : `${Math.round(n)}%`;
}

/** "1 device", "3 devices". */
export function count(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}
