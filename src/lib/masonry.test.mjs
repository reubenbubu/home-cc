// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { moveItem } from "./arrange.ts";
import { landing, pack } from "./masonry.ts";

/** Where each card ends up, as [column, top, bottom] in pixels, from the rows the grid is given. */
function drawn({ rows, places }) {
  const line = (n) => rows.slice(0, n - 1).reduce((sum, h) => sum + h, 0);
  return places.map((p) => [p.column, line(p.row), line(p.rowEnd)]);
}

test("in one column the cards follow each other, a gap apart", () => {
  const p = pack([100, 50, 80], 1, 10);
  assert.deepEqual(drawn(p), [
    [1, 0, 100],
    [1, 110, 160],
    [1, 170, 250],
  ]);
  assert.equal(p.height, 250);
});

test("each card is only as tall as it is, and the next one goes under the shortest column", () => {
  // The owner's screenshot: a short card, a very tall one, a short one, then three more.
  const p = pack([200, 900, 220, 180, 150, 210], 3, 16);
  assert.deepEqual(drawn(p), [
    [1, 0, 200],
    [2, 0, 900],
    [3, 0, 220],
    // Right under the first card, not after the tall one.
    [1, 216, 396],
    [3, 236, 386],
    [3, 402, 612],
  ]);
  assert.equal(p.height, 900);
});

test("cards of the same height read left to right, row by row", () => {
  const p = pack([100, 100, 100, 100, 100], 2, 10);
  assert.deepEqual(
    drawn(p).map(([c, top]) => [c, top]),
    [
      [1, 0],
      [2, 0],
      [1, 110],
      [2, 110],
      [1, 220],
    ],
  );
});

test("a column a fraction of a pixel shorter doesn't take the card from the one on its left", () => {
  const p = pack([120.4, 120, 50], 2, 10);
  assert.equal(p.places[2].column, 1);
  // A whole card shorter does.
  assert.equal(pack([160, 120, 50], 2, 10).places[2].column, 2);
});

test("the rows add up to the cards' height, and no card spans no row", () => {
  const p = pack([95.5, 0, 40, 260.25, 33], 3, 12.5);
  assert.equal(
    p.rows.reduce((a, b) => a + b, 0),
    p.height,
  );
  for (const place of p.places) assert.ok(place.rowEnd > place.row, JSON.stringify(place));
  assert.ok(p.rows.every((h) => h > 0));
});

test("more columns than cards, nothing at all, and readings that make no sense", () => {
  assert.deepEqual(
    pack([100, 60], 4, 10).places.map((p) => p.column),
    [1, 2],
  );
  assert.deepEqual(pack([], 3, 10), { rows: [], places: [], height: 0 });
  assert.deepEqual(
    pack([NaN, -5, 30], 0, -2).places.map((p) => p.column),
    [1, 1, 1],
  );
});

// Three columns 300 wide and 16 apart (932 in all), as in the owner's screenshot: A, a tall card, C, then
// D under A and E under C.
const cards = [200, 900, 220, 180, 150];
const at = (from, x, y) => landing(cards, from, 3, 16, 932, x, y);

test("a dragged card takes the place where it would be drawn under the pointer", () => {
  // E onto D, under A: E then sits under A, and D goes under C.
  assert.equal(at(4, 150, 300), 3);
  // D to the top of the first column, where A is: before A.
  assert.equal(at(3, 150, 20), 0);
  // On its own place it stays.
  assert.equal(at(4, 780, 300), 4);
  assert.equal(at(1, 466, 800), 1);
});

test("in the space under a short column it goes to the place drawn nearest the pointer", () => {
  // D, far down beside the tall card in the last column: last, which draws it under C (and E under A).
  assert.equal(at(3, 780, 600), 4);
  assert.equal(landing([], 0, 3, 16, 932, 10, 10), -1);
});

test("once moved, the same pointer keeps it there: it never swaps back and forth", () => {
  let seed = 7;
  const random = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
  for (let n = 0; n < 500; n++) {
    const heights = Array.from({ length: 2 + Math.floor(random() * 8) }, () => 60 + random() * (random() < 0.2 ? 900 : 250));
    const columns = 1 + Math.floor(random() * 4);
    const width = columns * 300 + (columns - 1) * 16;
    const from = Math.floor(random() * heights.length);
    const x = random() * width;
    const y = random() * 1400;
    const to = landing(heights, from, columns, 16, width, x, y);
    const moved = moveItem(heights, from, to);
    assert.equal(landing(moved, to, columns, 16, width, x, y), to, JSON.stringify({ heights, from, columns, x, y }));
  }
});

