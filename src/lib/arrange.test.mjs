// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  arrange,
  compareGroups,
  foldGroup,
  groupToSend,
  keyMove,
  moveItem,
  parseNames,
  parseOrder,
  remember,
  sameGroup,
  sections,
  slotAt,
  tidyGroup,
  toEndOfGroup,
} from "./arrange.ts";

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
  assert.deepEqual(remember([2, 1, 3], [1, 5, 6, 2, 3], [1, 2, 3, 5]), [2, 1, 5, 3]);
  assert.deepEqual(remember([2, 1], [], [1, 2]), [2, 1]);
});

test("a hidden machine keeps its place among the others when another card is moved", () => {
  // Saved A B C D (1 2 3 4); B is set to "Ignore", so A C D are shown. D is moved one place earlier.
  const saved = remember([1, 4, 3], [1, 2, 3, 4], [1, 2, 3, 4]);
  assert.deepEqual(saved, [1, 2, 4, 3]);
  // Set back from "Ignore", B is second again, after A.
  const back = [1, 2, 3, 4].map((id) => ({ id }));
  assert.deepEqual(ids(arrange(back, saved)), [1, 2, 4, 3]);
  // Hidden machines at the very start stay at the start, and two in a row stay together, in order.
  assert.deepEqual(remember([3, 1], [5, 1, 6, 7, 3], [1, 3, 5, 6, 7]), [5, 3, 1, 6, 7]);
  // A machine that only shows up now (no saved place yet) stays where it is on screen.
  assert.deepEqual(remember([8, 1, 3], [1, 2, 3], [1, 2, 3, 8]), [8, 1, 2, 3]);
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

// Groups. In the order the cards are shown, each with the group the owner gave it.
const grouped = [
  { id: 9, group: "Proxmox lab" },
  { id: 4, group: null },
  { id: 2, group: "NAS PC" },
  { id: 31, group: "Proxmox lab" },
  { id: 7, group: null },
  { id: 1, group: "NAS PC" },
  { id: 32, group: "lab 10" },
  { id: 33, group: "Lab 2" },
];

test("cards are sorted under their groups by name, keeping their order, with the ungrouped last", () => {
  const got = sections(grouped).map((s) => [s.group, ids(s.items)]);
  assert.deepEqual(got, [
    ["Lab 2", [33]],
    ["lab 10", [32]],
    ["NAS PC", [2, 1]],
    ["Proxmox lab", [9, 31]],
    [null, [4, 7]],
  ]);
});

test("with no groups there is one section, and with no machines none", () => {
  assert.deepEqual(sections(machines).map((s) => [s.group, ids(s.items)]), [[null, [4, 1, 7, 2, 9]]]);
  assert.deepEqual(sections([]), []);
  // An empty name is no group (the back end never stores one, but nothing should break on it).
  assert.deepEqual(sections([{ id: 1, group: "" }]).map((s) => s.group), [null]);
});

test("group names sort ignoring letter case, with numbers in number order", () => {
  assert.deepEqual(["proxmox", "NAS PC", "Lab 10", "lab 2", "Attic"].sort(compareGroups), ["Attic", "lab 2", "Lab 10", "NAS PC", "proxmox"]);
  // Names that differ only in letter case still get a fixed order.
  assert.deepEqual(["b", "B"].sort(compareGroups), ["B", "b"]);
  assert.deepEqual(["B", "b"].sort(compareGroups), ["B", "b"]);
});

test("a group name is trimmed with one space between words", () => {
  assert.equal(tidyGroup("  Proxmox \t  lab "), "Proxmox lab");
  assert.equal(tidyGroup("   "), "");
});

test("a typed group name joins the group with that name in any letter case", () => {
  const groups = ["NAS PC", "Proxmox lab", "proxmox LAB"];
  assert.equal(sameGroup("nas pc", groups), "NAS PC");
  assert.equal(sameGroup("  NAS   PC ", groups), "NAS PC");
  assert.equal(sameGroup("proxmox LAB", groups), "proxmox LAB", "the exact spelling first");
  assert.equal(sameGroup("PROXMOX lab", groups), "Proxmox lab");
  assert.equal(sameGroup("Attic", groups), null);
  assert.equal(sameGroup(" ", groups), null);
});

test("letter case is ignored in every alphabet, the way the back end does it", () => {
  // The back end's fold_group (inventory.rs) has the same cases in its tests.
  assert.equal(sameGroup("ħal far", ["Ħal Far"]), "Ħal Far");
  assert.equal(sameGroup("équipe", ["ÉQUIPE"]), "ÉQUIPE");
  assert.equal(sameGroup("οδοσ", ["ΟΔΟΣ"]), "ΟΔΟΣ", "a final sigma written either way");
  assert.equal(sameGroup("STRASSE", ["Straße"]), "Straße");
  assert.equal(foldGroup("Ħal Far"), "ħal far");
  assert.equal(foldGroup("ΟΔΟΣ"), foldGroup("οδοσ"));
});

test("the form sends a group only when the owner changed the field", () => {
  // Left as it opened: nothing is sent, so a group set meanwhile from the card menu stays.
  assert.equal(groupToSend("", ""), undefined);
  assert.equal(groupToSend("NAS PC", "NAS PC"), undefined);
  assert.equal(groupToSend("NAS PC", " NAS  PC "), undefined, "only spaces added");
  // Changed: the new name, or "" to take the machine out of its group.
  assert.equal(groupToSend("", " Proxmox  lab"), "Proxmox lab");
  assert.equal(groupToSend("NAS PC", "nas pc"), "nas pc", "letter case is a change");
  assert.equal(groupToSend("NAS PC", "  "), "");
});

test("a card that joins a group goes after that group's other cards", () => {
  // All the cards as shown: Proxmox lab (9, 31), NAS PC (2, 1), then the ungrouped (4, 7).
  const shown = [9, 31, 2, 1, 4, 7];
  // 4 has joined NAS PC.
  assert.deepEqual(toEndOfGroup(shown, 4, [2, 1, 4]), [9, 31, 2, 1, 4, 7]);
  // 9 has joined NAS PC.
  assert.deepEqual(toEndOfGroup(shown, 9, [2, 9, 1]), [31, 2, 1, 9, 4, 7]);
  // 1 is now alone in a new group: nothing to go after.
  assert.deepEqual(toEndOfGroup(shown, 1, [1]), shown);
});

test("the collapsed groups read back, and a damaged value counts as none", () => {
  assert.deepEqual(parseNames('["NAS PC","","NAS PC","Proxmox lab"]'), ["NAS PC", "", "Proxmox lab"]);
  assert.deepEqual(parseNames('["NAS PC", 3, null]'), ["NAS PC"]);
  assert.deepEqual(parseNames(null), []);
  assert.deepEqual(parseNames("{"), []);
  assert.deepEqual(parseNames('{"NAS PC":true}'), []);
});
