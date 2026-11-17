// How the Overview screen lays out the machines: a card for each PC, server, VM and container, under a
// heading for its type, and appliances (printers, TVs and the like, which mostly have no stats to show) as
// small tiles like the other devices. Kept apart from the screen so it can be tested without a browser.
import type { DeviceKind } from "./api";

/** The kinds shown as cards, in the order their headings come. */
export const CARD_KINDS = ["pc", "server", "vm", "container"] as const satisfies readonly DeviceKind[];
export type CardKind = (typeof CARD_KINDS)[number];

/** A heading's name: "PCs", or "PC" when there is just one. */
export function kindHeading(kind: CardKind, count: number): string {
  const one = { pc: "PC", server: "Server", vm: "VM", container: "Container" }[kind];
  return count === 1 ? one : `${one}s`;
}

export interface KindGroup<T> {
  kind: CardKind;
  items: T[];
}

/** The cards under a heading per type, in CARD_KINDS order, each keeping the order it was given. Types
 *  with nothing in them have no heading. Anything not shown as a card (appliances) is left out. */
export function byKind<T extends { kind: DeviceKind }>(items: T[]): KindGroup<T>[] {
  return CARD_KINDS.map((kind) => ({ kind, items: items.filter((d) => d.kind === kind) })).filter((g) => g.items.length > 0);
}

/** A down machine the owner has silenced on the Overview (one known to be off, say); `at` is when, in Unix
 *  seconds. It is left out of Needs attention and the Down counts until it is up again or the owner undoes it. */
export interface Silence {
  id: number;
  at: number;
}

/** What a silence is checked against: whether the machine is up now, and when it was last seen up. */
export interface Seen {
  id: number;
  health: "up" | "down" | "unknown";
  lastSeen: number | null;
}

/** Reads the saved silences back. Anything that isn't a list of them counts as none, so a damaged value
 *  never stops the screen from showing; a machine listed twice keeps the first. */
export function parseSilences(text: string | null): Silence[] {
  if (!text) return [];
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return [];
  }
  if (!Array.isArray(value)) return [];
  const out: Silence[] = [];
  for (const item of value) {
    if (typeof item !== "object" || item === null) continue;
    const { id, at } = item as { id?: unknown; at?: unknown };
    if (typeof id !== "number" || !Number.isSafeInteger(id) || id <= 0) continue;
    if (typeof at !== "number" || !Number.isSafeInteger(at) || at < 0) continue;
    if (out.some((s) => s.id === id)) continue;
    out.push({ id, at });
  }
  return out;
}

/** True while a silence still holds: the machine isn't up now and hasn't been seen up since it was
 *  silenced, so one that came back while another screen was open (or the app was closed) and went down
 *  again isn't silenced any more. Not checked yet counts as still silenced. */
export function stillSilenced(s: Silence, seen: Seen): boolean {
  return seen.health !== "up" && !(seen.lastSeen != null && seen.lastSeen > s.at);
}

/** The silences to keep: those of machines that still exist and are still silenced. */
export function keepSilences(list: Silence[], devices: Seen[]): Silence[] {
  const byId = new Map(devices.map((d) => [d.id, d]));
  return list.filter((s) => {
    const d = byId.get(s.id);
    return !!d && stillSilenced(s, d);
  });
}
