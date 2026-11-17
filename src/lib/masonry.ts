// How the Machines cards are packed: each card only as tall as what it shows, put in turn in the column
// that is shortest so far, so a short card sits right under the card above it instead of after a gap left
// by a tall card beside it. The owner's order still reads left to right, top to bottom as far as that
// allows, and the page keeps the cards in that order for the keys and screen readers. Kept apart from the
// screen so it can be tested without a browser (masonry.test.mjs).
//
// The screen measures each card and lays them out in a CSS grid: the rows are the gaps and the spans
// worked out here, in pixels, and each card is placed in its column and rows. The grid stays an ordinary
// part of the page, so its height, and the scrolling, are as before.

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

const columnsOf = (columns: number) => Math.max(1, Math.floor(columns) || 1);
const gapOf = (gap: number) => Math.max(0, gap || 0);

/** Each card's column (from 0) and its top and bottom in pixels, packed in turn into the shortest column. */
function spans(heights: number[], count: number, space: number) {
  const ends = new Array<number>(count).fill(0);
  return heights.map((h) => {
    let column = 0;
    const shortest = Math.min(...ends);
    while (ends[column] > shortest + SAME) column++;
    const top = round(ends[column]);
    const bottom = round(top + Math.max(0, Number.isFinite(h) ? h : 0));
    ends[column] = bottom + space;
    return { column, top, bottom };
  });
}

/** Packs cards of these heights into `columns` columns, `gap` pixels apart both ways. Each card in turn
 *  goes in the column that is shortest so far (the leftmost of those about as short). */
export function pack(heights: number[], columns: number, gap: number): Packing {
  const placed = spans(heights, columnsOf(columns), gapOf(gap));
  const lines = [...new Set([0, ...placed.flatMap((s) => [s.top, s.bottom])])].sort((a, b) => a - b);
  const at = new Map(lines.map((y, i) => [y, i + 1]));
  const rows = lines.slice(1).map((y, i) => round(y - lines[i]));
  const places = placed.map((s) => {
    const row = at.get(s.top) ?? 1;
    // A card with nothing in it still takes a row, so the grid never moves it elsewhere.
    return { column: s.column + 1, row, rowEnd: Math.max(row + 1, at.get(s.bottom) ?? row + 1) };
  });
  return { rows, places, height: lines[lines.length - 1] };
}

/** Where the dragged card goes when the pointer is at x, y in a grid `width` pixels wide. `heights` are
 *  the cards' heights in the order they are shown now, with the dragged card at place `from`.
 *
 *  Moving a card in packed columns moves the cards after it to other columns too, so "the card under the
 *  pointer" is no guide: the card would jump about and back. Instead each place it could take is packed,
 *  and it takes the one where it would be drawn under the pointer (or, with none, nearest to it). The
 *  other cards keep their order whichever place that is, so the answer depends only on where the pointer
 *  is, never on where the card was shown a moment ago, and it can't swap back and forth. Where it is now
 *  wins a tie, so it stays put while the pointer is on it. -1 when there are no cards. */
export function landing(
  heights: number[],
  from: number,
  columns: number,
  gap: number,
  width: number,
  x: number,
  y: number,
): number {
  if (from < 0 || from >= heights.length) return -1;
  const count = columnsOf(columns);
  const space = gapOf(gap);
  const wide = Math.max(0, (width - space * (count - 1)) / count);
  const card = heights[from];
  const others = heights.filter((_, i) => i !== from);
  const ways = heights.map((_, to) => {
    const s = spans([...others.slice(0, to), card, ...others.slice(to)], count, space)[to];
    const left = s.column * (wide + space);
    const dx = x < left ? left - x : x > left + wide ? x - (left + wide) : 0;
    const dy = y < s.top ? s.top - y : y > s.bottom ? y - s.bottom : 0;
    return { to, off: Math.hypot(dx, dy), centre: Math.hypot(x - (left + wide / 2), y - (s.top + s.bottom) / 2) };
  });
  const least = Math.min(...ways.map((w) => w.off));
  const near = ways.filter((w) => w.off <= least + 0.5);
  if (near.some((w) => w.to === from)) return from;
  return near.reduce((a, b) => (b.centre < a.centre ? b : a)).to;
}
