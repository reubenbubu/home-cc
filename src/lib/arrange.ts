// The order of the cards on the Machines screen, which the owner can change by dragging them. Kept apart
// from the screen so it can be tested without a browser (arrange.test.mjs).

/** A card's place on screen, in the grid's own pixels. */
export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Reads the saved order back. Anything that isn't a list of machine numbers counts as no order at all,
 *  so a damaged value never stops the screen from showing. */
export function parseOrder(text: string | null): number[] {
  if (!text) return [];
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return [];
  }
  if (!Array.isArray(value)) return [];
  const seen = new Set<number>();
  for (const id of value) {
    if (Number.isSafeInteger(id) && id > 0) seen.add(id);
  }
  return [...seen];
}

/** Puts the machines in the saved order. Machines without a saved place come after, in the order they
 *  were given (name order); saved places of machines that are gone are skipped. */
export function arrange<T extends { id: number }>(items: T[], saved: number[]): T[] {
  const byId = new Map(items.map((item) => [item.id, item]));
  const placed: T[] = [];
  for (const id of saved) {
    const item = byId.get(id);
    if (item) {
      placed.push(item);
      byId.delete(id);
    }
  }
  return [...placed, ...items.filter((item) => byId.has(item.id))];
}

/** The list with the item at `from` taken out and put back at `to`. */
export function moveItem<T>(list: T[], from: number, to: number): T[] {
  if (from < 0 || from >= list.length) return list.slice();
  const next = list.slice();
  const [item] = next.splice(from, 1);
  next.splice(Math.max(0, Math.min(to, next.length)), 0, item);
  return next;
}

/** The order to save after the owner has moved a card: the cards as they are now on screen, with the
 *  saved places of machines that still exist but aren't shown (one set to "Ignore" for a while, say)
 *  kept among them, so they come back where they were. Each goes back straight after the card that
 *  came before it in the saved order, or at the very start if none did. Machines that were removed
 *  are forgotten. */
export function remember(shown: number[], saved: number[], existing: Iterable<number>): number[] {
  const exists = new Set(existing);
  const onScreen = new Set(shown);
  // The hidden machines, by the shown card they follow (null: before every card).
  const after = new Map<number | null, number[]>();
  let anchor: number | null = null;
  for (const id of saved) {
    if (onScreen.has(id)) anchor = id;
    else if (exists.has(id)) after.set(anchor, [...(after.get(anchor) ?? []), id]);
  }
  const next = [...(after.get(null) ?? [])];
  for (const id of shown) next.push(id, ...(after.get(id) ?? []));
  return [...new Set(next)];
}

/** Where a dragged card would land: the card under the pointer, or else the nearest card in the nearest
 *  row, so the empty space after the last card means "at the end". -1 when there are no cards. */
export function slotAt(boxes: Box[], x: number, y: number): number {
  let best = -1;
  let bestDy = Infinity;
  let bestDx = Infinity;
  boxes.forEach((b, i) => {
    const dx = x < b.x ? b.x - x : x > b.x + b.w ? x - (b.x + b.w) : 0;
    const dy = y < b.y ? b.y - y : y > b.y + b.h ? y - (b.y + b.h) : 0;
    if (dy < bestDy || (dy === bestDy && dx < bestDx)) {
      best = i;
      bestDy = dy;
      bestDx = dx;
    }
  });
  return best;
}

// Groups: the owner can give machines a group name ("Proxmox lab"), and the cards are then shown under a
// heading per group. The saved order above is kept per card for all of them together; each group shows
// its own cards in that order, so moving a card only ever changes its place among its own group.

/** The longest group name, in characters (GROUP_MAX_CHARS in the back end's inventory.rs). */
export const GROUP_MAX = 40;

/** A group name as the back end stores it: trimmed, with runs of spaces made one. "" means no group. */
export function tidyGroup(text: string): string {
  return text.split(/\s+/).filter(Boolean).join(" ");
}

/** Group names in the order their headings are shown: alphabetical, ignoring letter case, "Lab 2" before
 *  "Lab 10". */
export function compareGroups(a: string, b: string): number {
  return a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" }) || (a < b ? -1 : a > b ? 1 : 0);
}

/** A group name with letter case taken out, to tell names that differ only in case: "NAS PC", "nas pc",
 *  and in any alphabet ("Équipe", "équipe"). The same in every language setting, and the same as
 *  fold_group in the back end's inventory.rs, which decides what is stored. */
export function foldGroup(name: string): string {
  return name.toUpperCase().toLowerCase();
}

/** The existing group a typed name puts a machine in: the one with the same name in any letter case (the
 *  back end stores it under that spelling), or null when it would be a new group. */
export function sameGroup(typed: string, existing: Iterable<string>): string | null {
  const name = tidyGroup(typed);
  if (!name) return null;
  const folded = foldGroup(name);
  let match: string | null = null;
  for (const group of existing) {
    if (group === name) return group;
    if (match === null && foldGroup(group) === folded) match = group;
  }
  return match;
}

/** The group to send when the Machines form is saved: nothing when the field still holds the group the
 *  form opened with, which leaves the machine's group as it is now. Its group may have been changed since
 *  (with the card's group menu, or by renaming or ungrouping its group), and a save of the other fields
 *  must not undo that. "" when the field was emptied, which takes the machine out of its group. */
export function groupToSend(opened: string, typed: string): string | undefined {
  const tidy = tidyGroup(typed);
  return tidy === tidyGroup(opened) ? undefined : tidy;
}

/** One heading's worth of cards. `group` is null for the machines in no group. */
export interface Section<T> {
  group: string | null;
  items: T[];
}

/** The cards, in the order they are shown, sorted under their groups: the groups by name, then the
 *  machines in no group. Each group keeps the cards in the order they were given. */
export function sections<T extends { group?: string | null }>(items: T[]): Section<T>[] {
  const byGroup = new Map<string, T[]>();
  const loose: T[] = [];
  for (const item of items) {
    const group = item.group || null;
    if (group === null) loose.push(item);
    else byGroup.set(group, [...(byGroup.get(group) ?? []), item]);
  }
  const named = [...byGroup.keys()].sort(compareGroups).map((group) => ({ group, items: byGroup.get(group) ?? [] }));
  return loose.length > 0 ? [...named, { group: null, items: loose }] : named;
}

/** The order of every card (`shown`) after the card `id` has joined another group: it goes after the last
 *  of that group's other cards (`members`), so it shows at the end of the group. With no other cards in
 *  the group the order stays as it is. */
export function toEndOfGroup(shown: number[], id: number, members: number[]): number[] {
  const others = new Set(members.filter((m) => m !== id));
  const rest = shown.filter((m) => m !== id);
  let last = -1;
  rest.forEach((m, i) => {
    if (others.has(m)) last = i;
  });
  if (last < 0) return shown.slice();
  return [...rest.slice(0, last + 1), id, ...rest.slice(last + 1)];
}

/** Reads back a saved list of names (the groups that are collapsed). Anything else counts as none. */
export function parseNames(text: string | null): string[] {
  if (!text) return [];
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return [];
  }
  if (!Array.isArray(value)) return [];
  return [...new Set(value.filter((name): name is string => typeof name === "string"))];
}

/** The new place for a card moved with the keyboard from place `index` of `count`: the arrow keys move
 *  it one place earlier or later (with or without Alt), Home and End to the start or the end. Null for
 *  any other key. */
export function keyMove(key: string, index: number, count: number): number | null {
  const last = Math.max(0, count - 1);
  switch (key) {
    case "ArrowUp":
    case "ArrowLeft":
      return Math.max(0, index - 1);
    case "ArrowDown":
    case "ArrowRight":
      return Math.min(last, index + 1);
    case "Home":
      return 0;
    case "End":
      return last;
    default:
      return null;
  }
}
