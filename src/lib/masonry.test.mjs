// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
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

// Two columns, 100 wide and 10 apart, packed: 0 and 2 on the left, 1 (tall) and 3 on the right.
const boxes = [
  { x: 0, y: 0, w: 100, h: 80 },
  { x: 110, y: 0, w: 100, h: 300 },
  { x: 0, y: 90, w: 100, h: 60 },
  { x: 110, y: 310, w: 100, h: 50 },
];

test("a dragged card takes the place of the card under the pointer", () => {
  assert.equal(landing(boxes, 0, 150, 200), 1);
  assert.equal(landing(boxes, 0, 50, 120), 2);
  assert.equal(landing(boxes, 3, 50, 10), 0);
  // On its own place it stays.
  assert.equal(landing(boxes, 2, 50, 120), 2);
});

test("below a column's last card it goes after that card, not to a card in the next column", () => {
  // Under card 2 on the left, beside the tall card 1.
  assert.equal(landing(boxes, 0, 50, 250), 2);
  assert.equal(landing(boxes, 3, 50, 250), 3);
  assert.equal(landing(boxes, 1, 50, 250), 2);
  // Under the last card of all: the end.
  assert.equal(landing(boxes, 0, 150, 500), 3);
  assert.equal(landing(boxes, 3, 150, 500), 3);
});

test("in a gap it goes by the nearer card of the column, and between columns by the nearer column", () => {
  assert.equal(landing(boxes, 3, 50, 83), 0);
  assert.equal(landing(boxes, 3, 50, 88), 2);
  assert.equal(landing(boxes, 0, 104, 120), 2);
  assert.equal(landing(boxes, 0, 107, 120), 1);
  // Above the first card of a column: before it.
  assert.equal(landing(boxes, 3, 150, -20), 1);
  assert.equal(landing(boxes, 0, 150, -20), 0);
  assert.equal(landing([], 0, 10, 10), -1);
});
