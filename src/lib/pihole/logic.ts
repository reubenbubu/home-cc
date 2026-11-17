// The Pi-hole screen's thinking, with no Svelte in it so "npm test" can try it: how blocking is described,
// how long a pause has left, which lists differ between Pi-holes, what the Overview should flag, and which
// machines look like Pi-holes.
import type { AdList, Blocking, ListKind, PiholeErrorKind, PiholeStatus } from "./api";

/** Seconds between readings while a Pi-hole screen or the Overview is open. */
export const POLL_SECS = 30;

/** The pauses the screen offers. null: until the owner turns blocking back on. */
export const PAUSES: { secs: number | null; label: string }[] = [
  { secs: 30, label: "30 seconds" },
  { secs: 300, label: "5 minutes" },
  { secs: 1800, label: "30 minutes" },
  { secs: null, label: "Until I turn it back on" },
];

/** "for 5 minutes", or "until you turn it back on". */
export function pauseWords(secs: number | null): string {
  if (secs == null) return "until you turn it back on";
  const choice = PAUSES.find((p) => p.secs === secs);
  if (choice) return `for ${choice.label}`;
  return `for ${duration(secs)}`;
}

/** 45 → "45 seconds", 300 → "5 minutes", 5400 → "1 hour 30 minutes". */
export function duration(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  const plural = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;
  if (s < 60) return plural(s, "second");
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h === 0) return plural(m, "minute");
  return m ? `${plural(h, "hour")} ${plural(m, "minute")}` : plural(h, "hour");
}

/** Seconds left of Pi-hole's timer now, from what it said at fetchedAt. Never below 0. */
export function timeLeft(timerSecs: number | null, fetchedAt: number | null, now: number): number | null {
  if (timerSecs == null) return null;
  const since = fetchedAt == null ? 0 : Math.max(0, now - fetchedAt);
  return Math.max(0, Math.round(timerSecs - since));
}

/** A countdown: "0:45", "4:12", "1:02:03". */
export function clockText(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  const two = (n: number) => String(n).padStart(2, "0");
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h ? `${h}:${two(m)}:${two(s % 60)}` : `${m}:${two(s % 60)}`;
}

export type Tone = "ok" | "warn" | "bad" | "idle";

/** How blocking is shown: a word, its colour, and what comes next. */
export interface BlockingView {
  tone: Tone;
  label: string;
  /** Such as "back on in 4:12"; empty when nothing is due. */
  detail: string;
  /** Blocking is off, for a while or for good. */
  paused: boolean;
}

export function blockingView(b: Blocking, fetchedAt: number | null, now: number): BlockingView {
  const left = timeLeft(b.timerSecs, fetchedAt, now);
  switch (b.state) {
    case "enabled":
      return { tone: "ok", label: "Blocking", detail: left != null ? `switches off in ${clockText(left)}` : "", paused: false };
    case "disabled":
      return left != null
        ? { tone: "warn", label: "Paused", detail: left > 0 ? `back on in ${clockText(left)}` : "back on any moment", paused: true }
        : { tone: "bad", label: "Blocking off", detail: "until you turn it back on", paused: true };
    case "failed":
      return { tone: "bad", label: "Blocking failed", detail: "Pi-hole couldn't switch blocking on", paused: false };
    default:
      return { tone: "idle", label: "Blocking state not known", detail: "", paused: false };
  }
}

/** 48213 → "48,213". */
export function number(n: number | null | undefined): string {
  return n == null ? "–" : Math.round(n).toLocaleString("en-US");
}

/** 20.4758 → "20.5%". */
export function percent(n: number | null | undefined): string {
  return n == null ? "–" : `${(Math.round(n * 10) / 10).toFixed(1)}%`;
}

/** What a list's status number means, when it isn't fine. */
export function listTrouble(status: number | null): string | null {
  if (status === 3) return "Couldn't be downloaded at the last update; an older copy is used.";
  if (status === 4) return "Couldn't be downloaded, and there is no older copy: it blocks nothing.";
  return null;
}

/** A list's state on one Pi-hole: on, there but switched off, or not there. */
export type Presence = "on" | "off" | "missing";

export interface ListRow {
  address: string;
  kind: ListKind;
  /** By device id. */
  on: Record<number, Presence>;
}

export interface Comparison {
  /** The Pi-holes compared, in the order given (those whose lists could be read). */
  compared: { id: number; name: string }[];
  /** Lists switched on everywhere. */
  same: number;
  /** Lists not switched on everywhere: blocklists first, then allowlists, each by address. */
  differences: ListRow[];
}

const listKey = (kind: ListKind, address: string) => `${kind} ${address.trim()}`;

/** Compares the lists of the Pi-holes given. Null when fewer than two have lists to compare. */
export function compareLists(hosts: { id: number; name: string; lists: AdList[] | null | undefined }[]): Comparison | null {
  const compared = hosts.filter((h) => h.lists != null);
  if (compared.length < 2) return null;
  const rows = new Map<string, ListRow>();
  for (const h of compared) {
    for (const l of h.lists ?? []) {
      const key = listKey(l.kind, l.address);
      let row = rows.get(key);
      if (!row) rows.set(key, (row = { address: l.address.trim(), kind: l.kind, on: {} }));
      // The same list twice on one Pi-hole: on if either is.
      if (row.on[h.id] !== "on") row.on[h.id] = l.enabled ? "on" : "off";
    }
  }
  let same = 0;
  const differences: ListRow[] = [];
  for (const row of rows.values()) {
    for (const h of compared) row.on[h.id] ??= "missing";
    if (compared.every((h) => row.on[h.id] === "on")) same++;
    else differences.push(row);
  }
  differences.sort((a, b) => (a.kind === b.kind ? a.address.localeCompare(b.address) : a.kind === "block" ? -1 : 1));
  return { compared: compared.map(({ id, name }) => ({ id, name })), same, differences };
}

/** One line about a list that differs: "Only on Pi-hole 1", "Switched off on Pi-hole 2", … */
export function differenceText(row: ListRow, compared: { id: number; name: string }[]): string {
  const where = (p: Presence) => compared.filter((h) => row.on[h.id] === p).map((h) => h.name);
  const on = where("on");
  const off = where("off");
  const missing = where("missing");
  const parts: string[] = [];
  if (on.length === 1 && compared.length > 1 && off.length === 0) parts.push(`Only on ${on[0]}`);
  else if (on.length) parts.push(`On ${joinNames(on)}`);
  if (off.length) parts.push(`switched off on ${joinNames(off)}`);
  if (missing.length && !(on.length === 1 && off.length === 0)) parts.push(`missing on ${joinNames(missing)}`);
  const text = parts.join("; ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function joinNames(names: string[]): string {
  return names.length <= 1 ? (names[0] ?? "") : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/** Short words for a failed reading, for the Overview and a card's header. */
export const TROUBLE: Record<PiholeErrorKind, string> = {
  noPassword: "needs its password",
  newCert: "needs its certificate approved",
  certChanged: "shows a different certificate",
  auth: "didn't accept the password",
  tooManySessions: "has no free API session",
  rateLimited: "is holding back logins",
  v5: "runs Pi-hole v5, not supported yet",
  unreachable: "can't be reached",
  other: "couldn't be read",
};

export interface Issue {
  key: string;
  text: string;
}

/** What the Overview flags about the Pi-holes: one that can't be read, and blocking that is off or paused.
 *  Nothing when all is well. */
export function piholeIssues(
  hosts: { id: number; name: string; status: PiholeStatus | null | undefined; readError?: string | null }[],
  now: number,
): Issue[] {
  const out: Issue[] = [];
  for (const h of hosts) {
    const s = h.status;
    if (h.readError) {
      out.push({ key: `pihole-${h.id}`, text: `${h.name} couldn't be read` });
      continue;
    }
    if (!s) continue;
    if (s.error) {
      out.push({ key: `pihole-${h.id}`, text: `${h.name} ${TROUBLE[s.errorKind ?? "other"]}` });
      continue;
    }
    const r = s.reading;
    if (!r) continue;
    const v = blockingView(r.blocking, s.fetchedAt, now);
    if (v.paused) out.push({ key: `pihole-${h.id}-blocking`, text: `${h.name}: ${v.label.toLowerCase()}${v.detail ? `, ${v.detail}` : ""}` });
    else if (r.blocking.state === "failed") out.push({ key: `pihole-${h.id}-blocking`, text: `${h.name}: Pi-hole couldn't switch blocking on` });
  }
  return out;
}

/** Saved machines whose name or hostname says Pi-hole, and that aren't Pi-holes in home-cc yet. */
export function suggestedByName<D extends { id: number; name: string; hostname: string | null; kind: string }>(
  devices: D[],
  already: Set<number>,
): D[] {
  const looksLike = (s: string | null) => !!s && /pi[\s._-]?hole/i.test(s);
  return devices.filter((d) => d.kind !== "ignore" && !already.has(d.id) && (looksLike(d.name) || looksLike(d.hostname)));
}

/** The port to show when the owner switches between HTTPS and HTTP: Pi-hole's own for each, unless they
 *  typed another one. */
export function portFor(https: boolean, current: number): number {
  if (https && current === 80) return 443;
  if (!https && current === 443) return 80;
  return current;
}
