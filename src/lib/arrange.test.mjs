// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { arrange, keyMove, moveItem, parseOrder, remember, slotAt } from "./arrange.ts";

// In name order, as the Machines screen hands them over.
const machines = [
  { id: 4, name: "Dev VM" },
  { id: 1, name: "Homelab VM" },
  { id: 7, name: "Media PC" },
  { id: 2, name: "NAS" },
  { id: 9, name: "Proxmox" },
];
const ids = (list) => list.map((m) => m.id);

test("with no saved order the cards stay in name order", () => {
  assert.deepEqual(ids(arrange(machines, [])), [4, 1, 7, 2, 9]);
});

test("the saved order comes first, and machines without a place follow in name order", () => {
  assert.deepEqual(ids(arrange(machines, [9, 2])), [9, 2, 4, 1, 7]);
  assert.deepEqual(ids(arrange(machines, [7, 9, 4, 1, 2])), [7, 9, 4, 1, 2]);
});

test("a removed machine drops out and the others keep their order", () => {
  assert.deepEqual(ids(arrange(machines, [9, 3, 2, 5, 4])), [9, 2, 4, 1, 7]);
});

test("a machine added later goes at the end", () => {
  const added = [...machines.slice(0, 2), { id: 12, name: "Aardvark" }, ...machines.slice(2)];
  assert.deepEqual(ids(arrange(added, [9, 2, 4, 1, 7])), [9, 2, 4, 1, 7, 12]);
});

test("arranging doesn't change the list it was given", () => {
  const copy = machines.slice();
  arrange(machines, [9, 2]);
  assert.deepEqual(machines, copy);
});

test("moving a card takes it out and puts it back at the new place", () => {
  assert.deepEqual(moveItem([1, 2, 3, 4], 0, 2), [2, 3, 1, 4]);
  assert.deepEqual(moveItem([1, 2, 3, 4], 3, 0), [4, 1, 2, 3]);
  assert.deepEqual(moveItem([1, 2, 3, 4], 1, 1), [1, 2, 3, 4]);
  assert.deepEqual(moveItem([1, 2, 3, 4], 1, 99), [1, 3, 4, 2]);
  assert.deepEqual(moveItem([1, 2, 3, 4], 9, 0), [1, 2, 3, 4]);
});

test("a saved order that isn't a list of machine numbers counts as none", () => {
  assert.deepEqual(parseOrder(null), []);
  assert.deepEqual(parseOrder(""), []);
  assert.deepEqual(parseOrder("not json"), []);
  assert.deepEqual(parseOrder('{"a":1}'), []);
  assert.deepEqual(parseOrder("[3, 1, 2]"), [3, 1, 2]);
  // Odd entries are skipped, and a machine listed twice keeps its first place.
  assert.deepEqual(parseOrder('[3, "x", 1.5, -2, 0, null, 3, 1]'), [3, 1]);
});

test("the order saved after a move keeps hidden machines that still exist, and forgets removed ones", () => {
  // 5 is set to "Ignore" for now, 6 was removed.
  assert.deepEqual(remember([2, 1, 3], [1, 5, 6, 2, 3], [1, 2, 3, 5]), [2, 1, 3, 5]);
  assert.deepEqual(remember([2, 1], [], [1, 2]), [2, 1]);
});

// Three cards in a row and two below, 100 wide with 10 between them.
const grid = [
  { x: 0, y: 0, w: 100, h: 80 },
  { x: 110, y: 0, w: 100, h: 80 },
  { x: 220, y: 0, w: 100, h: 80 },
  { x: 0, y: 90, w: 100, h: 120 },
  { x: 110, y: 90, w: 100, h: 120 },
];

test("a dragged card lands on the card under the pointer", () => {
  assert.equal(slotAt(grid, 50, 40), 0);
  assert.equal(slotAt(grid, 300, 10), 2);
  assert.equal(slotAt(grid, 150, 200), 4);
});

test("between cards or past the end, it lands on the nearest card in the nearest row", () => {
  assert.equal(slotAt(grid, 104, 40), 0);
  assert.equal(slotAt(grid, 107, 40), 1);
  // The empty space after the last card is the end, not the card above it.
  assert.equal(slotAt(grid, 300, 150), 4);
  assert.equal(slotAt(grid, 230, 92), 4);
  assert.equal(slotAt(grid, 60, 500), 3);
  assert.equal(slotAt(grid, -40, -40), 0);
  assert.equal(slotAt([], 10, 10), -1);
});

test("arrow keys move a card one place, Home and End to the start and the end", () => {
  assert.equal(keyMove("ArrowLeft", 2, 5), 1);
  assert.equal(keyMove("ArrowUp", 2, 5), 1);
  assert.equal(keyMove("ArrowRight", 2, 5), 3);
  assert.equal(keyMove("ArrowDown", 2, 5), 3);
  assert.equal(keyMove("Home", 2, 5), 0);
  assert.equal(keyMove("End", 2, 5), 4);
  // Already at the start or the end: it stays where it is.
  assert.equal(keyMove("ArrowLeft", 0, 5), 0);
  assert.equal(keyMove("ArrowRight", 4, 5), 4);
  assert.equal(keyMove("Enter", 2, 5), null);
  assert.equal(keyMove("a", 2, 5), null);
});
