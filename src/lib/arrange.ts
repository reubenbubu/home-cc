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

/** The order to save after the owner has moved a card: the cards as they are now on screen, then the
 *  saved places of machines that still exist but aren't shown (one set to "Ignore" for a while, say),
 *  so they get their place back. Machines that were removed are forgotten. */
export function remember(shown: number[], saved: number[], existing: Iterable<number>): number[] {
  const exists = new Set(existing);
  const onScreen = new Set(shown);
  return [...shown, ...saved.filter((id) => exists.has(id) && !onScreen.has(id))];
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
