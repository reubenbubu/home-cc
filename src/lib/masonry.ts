// How the Machines cards are packed: each card only as tall as what it shows, put in turn in the column
// that is shortest so far, so a short card sits right under the card above it instead of after a gap left
// by a tall card beside it. The owner's order still reads left to right, top to bottom as far as that
// allows, and the page keeps the cards in that order for the keys and screen readers. Kept apart from the
// screen so it can be tested without a browser (masonry.test.mjs).
//
// The screen measures each card and lays them out in a CSS grid: the rows are the gaps and the spans
// worked out here, in pixels, and each card is placed in its column and rows. The grid stays an ordinary
// part of the page, so its height, and the scrolling, are as before.
import type { Box } from "./arrange";

/** Where a card goes: its column and its first and last grid line, all counted from 1 as CSS does. */
export interface Place {
  column: number;
  row: number;
  rowEnd: number;
}

export interface Packing {
  /** The grid's rows, top to bottom, in pixels. */
  rows: number[];
  /** Where each card goes, in the order they were given. */
  places: Place[];
  /** How tall the cards are together. */
  height: number;
}

/** Columns that end less than this far apart count as the same length, so the leftmost of them is used:
 *  cards of about the same height then read left to right, and a reading a fraction of a pixel off never
 *  moves a card to another column. */
const SAME = 1;

/** Lines closer than this are one line. */
const round = (v: number) => Math.round(v * 100) / 100;

/** Packs cards of these heights into `columns` columns, `gap` pixels apart both ways. Each card in turn
 *  goes in the column that is shortest so far (the leftmost of those about as short). */
export function pack(heights: number[], columns: number, gap: number): Packing {
  const count = Math.max(1, Math.floor(columns) || 1);
  const space = Math.max(0, gap || 0);
  const ends = new Array<number>(count).fill(0);
  const spans = heights.map((h) => {
    let column = 0;
    const shortest = Math.min(...ends);
    while (ends[column] > shortest + SAME) column++;
    const top = round(ends[column]);
    const bottom = round(top + Math.max(0, Number.isFinite(h) ? h : 0));
    ends[column] = bottom + space;
    return { column, top, bottom };
  });
  const lines = [...new Set([0, ...spans.flatMap((s) => [s.top, s.bottom])])].sort((a, b) => a - b);
  const at = new Map(lines.map((y, i) => [y, i + 1]));
  const rows = lines.slice(1).map((y, i) => round(y - lines[i]));
  const places = spans.map((s) => {
    const row = at.get(s.top) ?? 1;
    // A card with nothing in it still takes a row, so the grid never moves it elsewhere.
    return { column: s.column + 1, row, rowEnd: Math.max(row + 1, at.get(s.bottom) ?? row + 1) };
  });
  return { rows, places, height: lines[lines.length - 1] };
}

/** The new place for the dragged card (now at place `from`) when it is let go at x, y, from where the
 *  cards are on screen. It goes by the column under the pointer: on a card, it takes that card's place;
 *  above a column's first card it goes before it, and in the space below a column's last card, after it.
 *  Between columns it goes by the nearest one. -1 when there are no cards. */
export function landing(boxes: Box[], from: number, x: number, y: number): number {
  if (boxes.length === 0) return -1;
  const dx = (b: Box) => (x < b.x ? b.x - x : x > b.x + b.w ? x - (b.x + b.w) : 0);
  const nearest = Math.min(...boxes.map(dx));
  // The cards of the column under the pointer (or the nearest), top to bottom.
  const left = boxes.find((b) => dx(b) === nearest)?.x ?? 0;
  const column = boxes
    .map((b, i) => ({ ...b, i }))
    .filter((b) => Math.abs(b.x - left) < 1)
    .sort((a, b) => a.y - b.y);
  const before = (k: number) => (from < k ? k - 1 : k);
  const after = (k: number) => (k === from ? from : from < k ? k : Math.min(k + 1, boxes.length - 1));
  const first = column[0];
  const last = column[column.length - 1];
  if (y < first.y) return before(first.i);
  if (y > last.y + last.h) return after(last.i);
  // On a card, or in the gap between two: the nearer of them.
  let best = first;
  let bestDy = Infinity;
  for (const b of column) {
    const dy = y < b.y ? b.y - y : y > b.y + b.h ? y - (b.y + b.h) : 0;
    if (dy < bestDy) {
      best = b;
      bestDy = dy;
    }
  }
  return best.i;
}
