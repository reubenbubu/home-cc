// The Docker screen's small decisions, kept free of Svelte and the back end so
// they can be tested with plain Node (logic.test.mjs).
import type { CleanupPick, CleanupPreview, Container, DockerHost, Overview, Port, Stack } from "./api";

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

/** A published port as the list shows it: "1900→1900/udp", and the address it is published on when that is
 *  one address rather than all of them, which the list shows beside it with its hostname (Address). */
export function portShown(p: Port): { text: string; ip: string | null } {
  const inside = `${p.containerPort}/${p.protocol}`;
  if (p.hostPort == null) return { text: inside, ip: null };
  return { text: `${p.hostPort}→${inside}`, ip: p.hostIp?.trim() || null };
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

/** The host the screen should show: the one it shows, while it still has a tab, else the first that has one. */
export function keptSelection(selected: number | null, shown: DockerHost[]): number | null {
  if (selected != null && shown.some((h) => h.deviceId === selected)) return selected;
  return shown[0]?.deviceId ?? null;
}

/** Where an action on a stack is kept. Stacks on two hosts often share a name ("monitoring"), so the host is part of it. */
export function stackKey(hostId: number, stack: string): string {
  return `s:${hostId}:${stack}`;
}

/** Where an action on a container is kept. */
export function containerKey(hostId: number, containerId: string): string {
  return `c:${hostId}:${containerId}`;
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

/** The names Docker gives containers, and full or short container ids. Nothing else goes into a command. */
const CONTAINER_NAME = /^[a-zA-Z0-9][a-zA-Z0-9_.-]*$/;
const CONTAINER_ID = /^[0-9a-f]{12,64}$/;

/** Where docker is looked for on a Docker host besides the login's own PATH, /snap/bin among them: a command
 *  sent over SSH gets no login shell, so on Debian with the docker snap, for one, /snap/bin isn't on its
 *  PATH. The same list as PREAMBLE in src-tauri/src/docker/scripts.rs, whose tests check that they agree. */
export const DOCKER_PATH = "/usr/local/bin:/usr/bin:/bin:/snap/bin";

/** Put before docker in a command run on a Docker host. `env`, because `PATH=… docker` isn't understood by
 *  csh or tcsh, which can be the login shell that runs the command. */
const DOCKER_ENV = `env PATH="$PATH:${DOCKER_PATH}" `;

/** The command a terminal into this running container runs on its Docker host: bash if the container has it,
 *  else sh. It is run the way the Docker screen runs docker: as the SSH user, who must be allowed to use
 *  Docker, and with docker looked for in the same places. The container goes in by its name (easier to
 *  read), or by its id; anything that isn't a name or id Docker could have given is refused (null) rather
 *  than quoted, so nothing odd ever reaches a shell. */
export function execCommand(c: { id: string; name: string }): string | null {
  const target = CONTAINER_NAME.test(c.name) && c.name.length <= 255 ? c.name : CONTAINER_ID.test(c.id) ? c.id : null;
  if (!target) return null;
  return `${DOCKER_ENV}docker exec -it ${target} sh -c 'command -v bash >/dev/null && exec bash || exec sh'`;
}

/** A command as shown beside a terminal's tabs: without the PATH that `execCommand` puts first, so the part
 *  that says what runs is the part still in view when the line is cut short. */
export function commandShown(command: string): string {
  return command.startsWith(DOCKER_ENV) ? command.slice(DOCKER_ENV.length) : command;
}

/** The last part of a path, for tabs and titles. */
export function baseName(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** A container on a host that the owner should look at, and why, in a few words. */
export interface ContainerAttention {
  name: string;
  stack: string | null;
  why: string;
}

/** One host's containers in a few numbers, for the Overview screen. */
export interface HostSummary {
  containers: number;
  running: number;
  /** Compose stacks with at least one container. */
  stacks: number;
  attention: ContainerAttention[];
}

/** Why a container needs a look, said so it follows its name ("zigbee keeps restarting"), or null if it is fine. */
export function containerAttention(c: Container): string | null {
  if (c.health === "unhealthy") return "is unhealthy";
  if (c.state === "restarting") return c.exitCode ? `keeps restarting (exit ${c.exitCode})` : "keeps restarting";
  if (c.state === "dead") return "is dead";
  if (c.state === "exited" && c.exitCode != null && !NORMAL_STOP.includes(c.exitCode)) return `stopped with an error (exit ${c.exitCode})`;
  return null;
}

export function summarise(o: Overview): HostSummary {
  const all = [...o.stacks.flatMap((s) => s.containers), ...o.others];
  const attention: ContainerAttention[] = [];
  for (const c of all) {
    const why = containerAttention(c);
    if (why) attention.push({ name: c.name, stack: c.project, why });
  }
  attention.sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
  return {
    containers: all.length,
    running: all.filter((c) => c.state === "running").length,
    stacks: o.stacks.filter((s) => s.containers.length > 0).length,
    attention,
  };
}
