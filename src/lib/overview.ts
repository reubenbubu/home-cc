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
