// The Docker screen's small decisions, kept free of Svelte and the back end so
// they can be tested with plain Node (logic.test.mjs).
import type { CleanupPick, CleanupPreview, Container, DockerHost, Port, Stack } from "./api";

export type Tone = "ok" | "warn" | "bad" | "idle";

/** How a container ends when it is stopped on purpose: 0, or 143 and 137 when `docker stop` had to signal it. */
const NORMAL_STOP = [0, 137, 143];

/** "8096→8096/tcp", "192.168.68.21:1900→1900/udp", or "8920/tcp" for a port that isn't published. */
export function portText(p: Port): string {
  const inside = `${p.containerPort}/${p.protocol}`;
  if (p.hostPort == null) return inside;
  const host = p.hostIp ? `${p.hostIp.includes(":") ? `[${p.hostIp}]` : p.hostIp}:${p.hostPort}` : String(p.hostPort);
  return `${host}→${inside}`;
}

/** Published ports only, as the list shows them; the rest are only in the details. */
export function publishedPorts(c: Container): Port[] {
  return c.ports.filter((p) => p.hostPort != null);
}

export function isRunning(c: Container): boolean {
  return c.state === "running" || c.state === "restarting" || c.state === "paused";
}

/** The colour of a container's state dot. */
export function stateTone(c: Container): Tone {
  switch (c.state) {
    case "running":
      return c.health === "unhealthy" ? "bad" : c.health === "starting" ? "warn" : "ok";
    case "restarting":
    case "paused":
      return "warn";
    case "dead":
      return "bad";
    case "exited":
      return c.exitCode != null && !NORMAL_STOP.includes(c.exitCode) ? "bad" : "idle";
    default:
      return "idle";
  }
}

/** "4 d 3 h": shorter than uptime() needs to be here. */
function span(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d > 0) return `${d} d ${h} h`;
  if (h > 0) return `${h} h ${m} min`;
  if (m > 0) return `${m} min`;
  return `${s} s`;
}

/** "Up 4 d 3 h", "Up 12 min · unhealthy", "Stopped 2 h ago (exit 1)", "Restarting", "Never started". */
export function stateText(c: Container, now: number): string {
  const health = c.health && c.health !== "healthy" ? ` · ${c.health}` : "";
  switch (c.state) {
    case "running":
      return (c.startedAt != null ? `Up ${span(now - c.startedAt)}` : "Up") + health;
    case "restarting":
      return `Restarting${c.exitCode != null && c.exitCode !== 0 ? ` (exit ${c.exitCode})` : ""}`;
    case "paused":
      return "Paused";
    case "created":
      return "Never started";
    case "exited":
    case "dead": {
      const word = c.state === "dead" ? "Dead" : "Stopped";
      const when = c.finishedAt != null ? ` ${span(now - c.finishedAt)} ago` : "";
      const code = c.exitCode != null && !NORMAL_STOP.includes(c.exitCode) ? ` (exit ${c.exitCode})` : "";
      return `${word}${when}${code}`;
    }
    default:
      return c.state.charAt(0).toUpperCase() + c.state.slice(1);
  }
}

export interface StackSummary {
  running: number;
  total: number;
  tone: Tone;
  text: string;
}

export function stackSummary(s: Stack): StackSummary {
  const total = s.containers.length;
  const running = s.containers.filter((c) => c.state === "running").length;
  if (total === 0) return { running, total, tone: "idle", text: "Down" };
  const troubled = s.containers.some((c) => stateTone(c) === "bad");
  const tone: Tone = troubled ? "bad" : running === total ? "ok" : running === 0 ? "idle" : "warn";
  const text = running === total ? (total === 1 ? "Running" : `All ${total} running`) : `${running} of ${total} running`;
  return { running, total, tone, text };
}

/** Stacks that are up first, then those that are down, each by name. */
export function stackOrder(stacks: Stack[]): Stack[] {
  return [...stacks].sort((a, b) => Number(a.containers.length === 0) - Number(b.containers.length === 0) || a.name.localeCompare(b.name));
}

/** Hosts the screen shows: marked by the owner, or found by home-cc (and not marked off). */
export function shownHosts(hosts: DockerHost[]): DockerHost[] {
  return hosts.filter((h) => h.mode === "on" || (h.mode === "auto" && h.found === true));
}

/** Docker's --timestamps prefix: "2026-11-17T14:03:09.123456789Z message". */
const STAMP = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(\.\d+)?(Z|[+-]\d{2}:\d{2}) ?/;

export interface LogLine {
  /** Milliseconds since 1970, when the line had Docker's timestamp. */
  at: number | null;
  text: string;
}

export function logLine(raw: string): LogLine {
  const m = STAMP.exec(raw);
  if (!m) return { at: null, text: stripAnsi(raw) };
  const frac = m[2] ? m[2].slice(0, 4) : "";
  const at = Date.parse(`${m[1]}${frac}${m[3]}`);
  return { at: Number.isFinite(at) ? at : null, text: stripAnsi(raw.slice(m[0].length)) };
}

/** Colour and cursor codes some programs print, removed so the text reads cleanly. */
export function stripAnsi(s: string): string {
  // eslint-disable-next-line no-control-regex
  return s.replace(/\u001b\[[0-?]*[ -/]*[@-~]|\u001b\][^\u0007\u001b]*(\u0007|\u001b\\)|\u001b[@-Z\\-_]/g, "");
}

/** Lines that hold the search text, ignoring case. An empty search matches everything. */
export function matches(text: string, query: string): boolean {
  const q = query.trim().toLowerCase();
  return q === "" || text.toLowerCase().includes(q);
}

/** The line split around each match, for highlighting. */
export function highlight(text: string, query: string): { text: string; hit: boolean }[] {
  const q = query.trim().toLowerCase();
  if (!q) return [{ text, hit: false }];
  const parts: { text: string; hit: boolean }[] = [];
  const lower = text.toLowerCase();
  let from = 0;
  for (let i = lower.indexOf(q); i !== -1; i = lower.indexOf(q, i + q.length)) {
    if (i > from) parts.push({ text: text.slice(from, i), hit: false });
    parts.push({ text: text.slice(i, i + q.length), hit: true });
    from = i + q.length;
  }
  if (from < text.length) parts.push({ text: text.slice(from), hit: false });
  return parts;
}

/** Keeps at most `max` lines, dropping the oldest. */
export function keepLast<T>(list: T[], max: number): T[] {
  return list.length > max ? list.slice(list.length - max) : list;
}

/** A file name for a saved log: "homelab-vm-jellyfin-2026-11-17-1403.log". */
export function logFileName(host: string, container: string, at: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  const stamp = `${at.getFullYear()}-${p(at.getMonth() + 1)}-${p(at.getDate())}-${p(at.getHours())}${p(at.getMinutes())}`;
  const clean = (s: string) =>
    s
      .toLowerCase()
      .replace(/[^a-z0-9._-]+/g, "-")
      .replace(/^-+|-+$/g, "");
  return `${[clean(host), clean(container), stamp].filter(Boolean).join("-")}.log`;
}

/** What a cleanup picks by itself: everything except named volumes, which may hold a stack's data. */
export function defaultPick(p: CleanupPreview): CleanupPick {
  return {
    containers: p.containers.map((c) => c.key),
    images: p.images.map((i) => i.key),
    volumes: p.volumes.filter((v) => isUnnamedVolume(v.detail)).map((v) => v.key),
    buildCache: p.buildCacheCount > 0,
  };
}

export function isUnnamedVolume(detail: string): boolean {
  return detail === "Unnamed volume";
}

/** How many things the pick removes and the space that frees, from the preview's sizes. */
export function pickTotals(p: CleanupPreview, pick: CleanupPick): { count: number; bytes: number } {
  let count = 0;
  let bytes = 0;
  const add = (items: { key: string; sizeBytes: number | null }[], keys: string[]) => {
    for (const i of items) {
      if (keys.includes(i.key)) {
        count++;
        bytes += i.sizeBytes ?? 0;
      }
    }
  };
  add(p.containers, pick.containers);
  add(p.images, pick.images);
  add(p.volumes, pick.volumes);
  if (pick.buildCache && p.buildCacheCount > 0) {
    count += p.buildCacheCount;
    bytes += p.buildCacheBytes;
  }
  return { count, bytes };
}

/** Problems that need the owner on the Machines screen before anything is tried again. */
export function isLoginProblem(kind: string | undefined | null): boolean {
  return kind === "newHost" || kind === "hostKeyChanged" || kind === "auth";
}

/** The command a terminal into this container would run (for the Terminals module, once it is there). */
export function execCommand(c: Container): string {
  return `docker exec -it ${c.id.slice(0, 12)} sh -c 'command -v bash >/dev/null && exec bash || exec sh'`;
}

/** The last part of a path, for tabs and titles. */
export function baseName(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
