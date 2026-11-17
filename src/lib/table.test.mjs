// Run with "npm test" (Node reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  arrange,
  choicesOf,
  compareIp,
  dirWords,
  filterRows,
  filterWords,
  groupRows,
  keepOrder,
  nextSort,
  parseSaved,
  parseSortChoice,
  rowMatches,
  sortChoices,
  sortRows,
  STATUS_ORDER,
  usableGroup,
  usableSort,
} from "./table.ts";

const names = (rows) => rows.map((r) => r.name);

const ROWS = [
  { name: "Media PC", ip: "192.168.68.70", rtt: 12, kind: "PC", seen: 1700000300, health: "up", group: null },
  { name: "NAS", ip: "192.168.68.10", rtt: null, kind: "Server", seen: 1700000100, health: "up", group: "NAS PC" },
  { name: "pc 10", ip: "192.168.68.9", rtt: 0.4, kind: "PC", seen: null, health: "down", group: "NAS PC" },
  { name: "PC 2", ip: "192.168.68.101", rtt: 3, kind: "Phone", seen: 1700000200, health: null, group: "" },
  { name: "Café", ip: "fe80::1", rtt: 120, kind: "PC", seen: 1700000050, health: "warning", group: "Lab" },
  { name: "printer.lan", ip: "printer.lan", rtt: 9, kind: "Appliance", seen: 1700000400, health: "up", group: "Lab" },
];

const COLS = {
  name: { id: "name", label: "Name", value: (r) => r.name },
  ip: { id: "ip", label: "Address", kind: "ip", value: (r) => r.ip, search: (r) => [r.ip, r.ip === "192.168.68.10" ? "nas-box" : null] },
  rtt: { id: "rtt", label: "Response", kind: "number", value: (r) => r.rtt, text: (r) => (r.rtt == null ? null : `${r.rtt} ms`) },
  kind: { id: "kind", label: "Type", value: (r) => r.kind, group: true, choose: true },
  seen: { id: "seen", label: "Last seen", kind: "time", value: (r) => r.seen },
  health: { id: "health", label: "Status", kind: "rank", order: STATUS_ORDER, value: (r) => r.health, group: true, none: "Not checked yet" },
  group: { id: "group", label: "Group", value: (r) => r.group, group: true, none: "In no group" },
  actions: { id: "actions", label: "Actions", sortable: false, value: () => null },
};
const ALL = Object.values(COLS);

test("IP addresses sort number by number, IPv6 after IPv4, names last", () => {
  assert.deepEqual(names(sortRows(ROWS, COLS.ip, "asc")), ["pc 10", "NAS", "Media PC", "PC 2", "Café", "printer.lan"]);
  assert.ok(compareIp("192.168.68.9", "192.168.68.10") < 0);
  assert.ok(compareIp("10.0.0.2", "9.255.255.255") > 0);
  assert.ok(compareIp("192.168.68.10", "::ffff:192.168.68.10") < 0);
  assert.ok(compareIp("fe80::2", "fe80::10") < 0);
  assert.ok(compareIp("2001:db8::1", "fe80::1") < 0);
  // Not addresses: 256 is too big, so it is a name and goes after every address.
  assert.ok(compareIp("192.168.68.256", "fe80::1") > 0);
  // Backwards, still number by number (not 9 after 10 as text would have it).
  assert.deepEqual(names(sortRows(ROWS, COLS.ip, "desc")).slice(-3), ["Media PC", "NAS", "pc 10"]);
});

test("numbers sort by value, not by how they are written, and empty ones go last both ways", () => {
  assert.deepEqual(names(sortRows(ROWS, COLS.rtt, "asc")), ["pc 10", "PC 2", "printer.lan", "Media PC", "Café", "NAS"]);
  assert.deepEqual(names(sortRows(ROWS, COLS.rtt, "desc")), ["Café", "Media PC", "printer.lan", "PC 2", "pc 10", "NAS"]);
});

test("text sorts with numbers in number order, ignoring capitals and accents", () => {
  const rows = ["pc 10", "PC 2", "Pc 1", "cafe 3", "Café 20", "ápple"].map((name) => ({ name }));
  assert.deepEqual(names(sortRows(rows, COLS.name, "asc")), ["ápple", "cafe 3", "Café 20", "Pc 1", "PC 2", "pc 10"]);
  assert.deepEqual(names(sortRows(rows, COLS.name, "desc")), ["pc 10", "PC 2", "Pc 1", "Café 20", "cafe 3", "ápple"]);
});

test("times sort by time, with never last both ways", () => {
  assert.deepEqual(names(sortRows(ROWS, COLS.seen, "asc")), ["Café", "NAS", "PC 2", "Media PC", "printer.lan", "pc 10"]);
  assert.equal(sortRows(ROWS, COLS.seen, "desc").at(-1).name, "pc 10");
});

test("status sorts down, warning, up, with not checked last both ways", () => {
  assert.deepEqual(names(sortRows(ROWS, COLS.health, "asc")), ["pc 10", "Café", "Media PC", "NAS", "printer.lan", "PC 2"]);
  assert.deepEqual(names(sortRows(ROWS, COLS.health, "desc")), ["Media PC", "NAS", "printer.lan", "Café", "pc 10", "PC 2"]);
});

test("rows that tie keep the order they came in, either way round", () => {
  const rows = [1, 2, 3, 4, 5].map((n) => ({ name: `row ${n}`, kind: n % 2 ? "odd" : "even" }));
  assert.deepEqual(names(sortRows(rows, COLS.kind, "asc")), ["row 2", "row 4", "row 1", "row 3", "row 5"]);
  assert.deepEqual(names(sortRows(rows, COLS.kind, "desc")), ["row 1", "row 3", "row 5", "row 2", "row 4"]);
  // The rows given are not changed.
  assert.equal(rows[0].name, "row 1");
});

test("a header click sorts one way, then the other, then back to the table's own order", () => {
  let s = nextSort(null, "name");
  assert.deepEqual(s, { col: "name", dir: "asc" });
  s = nextSort(s, "name");
  assert.deepEqual(s, { col: "name", dir: "desc" });
  assert.equal(nextSort(s, "name"), null);
  assert.deepEqual(nextSort({ col: "ip", dir: "desc" }, "name"), { col: "name", dir: "asc" });
});

test("the sort menu offers each sortable column both ways, in words", () => {
  const offered = sortChoices(ALL);
  assert.ok(!offered.some((o) => o.value.startsWith("actions:")));
  assert.deepEqual(offered.slice(0, 2), [
    { value: "name:asc", label: "Name, A to Z" },
    { value: "name:desc", label: "Name, Z to A" },
  ]);
  assert.deepEqual(dirWords(COLS.seen), ["oldest first", "newest first"]);
  assert.deepEqual(dirWords(COLS.health), ["down first", "up first"]);
  assert.deepEqual(parseSortChoice("ip:desc"), { col: "ip", dir: "desc" });
  assert.equal(parseSortChoice("ip"), null);
});

test("groups come in the column's order with counts, and rows without a value last", () => {
  const sorted = sortRows(ROWS, COLS.name, "asc");
  const groups = groupRows(sorted, COLS.group);
  assert.deepEqual(
    groups.map((g) => [g.label, g.rows.length]),
    [["Lab", 2], ["NAS PC", 2], ["In no group", 2]],
  );
  // Inside each group, the rows keep the sort.
  assert.deepEqual(names(groups[0].rows), ["Café", "printer.lan"]);
  assert.deepEqual(names(groups[2].rows), ["Media PC", "PC 2"]);
  assert.equal(groups[2].key, "");
});

test("groups by a ranked column follow its order, and the other way round when sorted by it backwards", () => {
  assert.deepEqual(groupRows(ROWS, COLS.health).map((g) => g.label), ["down", "warning", "up", "Not checked yet"]);
  assert.deepEqual(
    groupRows(ROWS, COLS.health, { col: "health", dir: "desc" }).map((g) => g.label),
    ["up", "warning", "down", "Not checked yet"],
  );
  // Sorted by another column backwards: the groups stay in their own order.
  assert.deepEqual(groupRows(ROWS, COLS.health, { col: "name", dir: "desc" }).map((g) => g.label)[0], "down");
});

test("choices list each value once with its count", () => {
  assert.deepEqual(choicesOf(ROWS, COLS.kind), [
    { key: "Appliance", label: "Appliance", count: 1 },
    { key: "PC", label: "PC", count: 3 },
    { key: "Phone", label: "Phone", count: 1 },
    { key: "Server", label: "Server", count: 1 },
  ]);
});

test("the filter matches across columns, ignoring capitals and accents, every word somewhere", () => {
  assert.deepEqual(filterWords("  NAS   Box "), ["nas", "box"]);
  assert.deepEqual(names(filterRows(ROWS, ALL, "cafe")), ["Café"]);
  assert.deepEqual(names(filterRows(ROWS, ALL, "68.1")), ["NAS", "PC 2"]);
  // The hostname shown beside an address counts.
  assert.deepEqual(names(filterRows(ROWS, ALL, "NAS-BOX")), ["NAS"]);
  // Words can be in different columns, and all must be there.
  assert.deepEqual(names(filterRows(ROWS, ALL, "pc 0.4 ms")), ["pc 10"]);
  // Part of a word is enough: "12" is in "12 ms" and in "120 ms".
  assert.deepEqual(names(filterRows(ROWS, ALL, "12")), ["Media PC", "Café"]);
  assert.deepEqual(names(filterRows(ROWS, ALL, "pc nothing-like-this")), []);
  // Words as shown: "ms" is only in the response column's words.
  assert.equal(filterRows(ROWS, ALL, "ms").length, 5);
  assert.equal(filterRows(ROWS, ALL, "").length, ROWS.length);
  assert.ok(rowMatches(ROWS[0], ALL, []));
});

test("a choice keeps only the rows with that value, and works with the typed words", () => {
  assert.deepEqual(names(filterRows(ROWS, ALL, "", { kind: "PC" })), ["Media PC", "pc 10", "Café"]);
  assert.deepEqual(names(filterRows(ROWS, ALL, "cafe", { kind: "PC" })), ["Café"]);
  assert.deepEqual(names(filterRows(ROWS, ALL, "", { group: "" })), ["Media PC", "PC 2"]);
  // A choice for a column this table doesn't have is left out.
  assert.equal(filterRows(ROWS, ALL, "", { gone: "x" }).length, ROWS.length);
});

test("a saved sort or group is only used when the table has that column and allows it", () => {
  assert.deepEqual(usableSort({ col: "ip", dir: "desc" }, ALL), { col: "ip", dir: "desc" });
  assert.equal(usableSort({ col: "node", dir: "asc" }, ALL), null);
  assert.equal(usableSort({ col: "actions", dir: "asc" }, ALL), null);
  assert.equal(usableGroup("kind", ALL)?.id, "kind");
  assert.equal(usableGroup("name", ALL), null);
  assert.equal(usableGroup(null, ALL), null);
});

test("what is kept in storage is checked", () => {
  assert.deepEqual(parseSaved('{"sort":{"col":"ip","dir":"desc"},"group":"kind"}'), { sort: { col: "ip", dir: "desc" }, group: "kind" });
  assert.deepEqual(parseSaved('{"sort":{"col":"ip","dir":"sideways"}}'), { sort: null, group: null });
  assert.deepEqual(parseSaved("not json"), { sort: null, group: null });
  assert.deepEqual(parseSaved(null), { sort: null, group: null });
  assert.deepEqual(parseSaved("[1,2]"), { sort: null, group: null });
});

test("while the owner works in a table, rows keep their places and new ones go after them", () => {
  const before = ["b", "a", "c"];
  const rows = [{ k: "a" }, { k: "d" }, { k: "b" }, { k: "e" }];
  assert.deepEqual(keepOrder(rows, before, (r) => r.k).map((r) => r.k), ["b", "a", "d", "e"]);
});

test("held while the owner works in a row, a reading that comes in moves no row to another group or out of the filter", () => {
  const devices = [
    { id: 1, name: "NAS", health: "up" },
    { id: 2, name: "Printer", health: "up" },
    { id: 3, name: "TV", health: "down" },
  ];
  const how = { cols: [COLS.name, COLS.health], filter: "", only: {}, sort: { col: "name", dir: "asc" }, group: COLS.health, keyOf: (r) => r.id };
  const first = arrange(devices, how);
  const read = (a) => a.groups.map((g) => `${g.label}: ${names(g.rows).join(", ")}`).join(" | ");
  assert.equal(read(first), "down: TV | up: NAS, Printer");

  // The NAS goes down (while it is being renamed, say), and a new device turns up.
  const later = [{ ...devices[0], health: "down" }, devices[1], devices[2], { id: 4, name: "Camera", health: "up" }];
  const held = arrange(later, how, first.shown);
  assert.equal(read(held), "down: TV | up: NAS, Printer, Camera");
  // Held again on the next reading, it is still where it was.
  assert.equal(read(arrange(later, how, held.shown)), "down: TV | up: NAS, Printer, Camera");
  // Let go, everything is where its values put it.
  assert.equal(read(arrange(later, how)), "down: NAS, TV | up: Camera, Printer");

  // Only the devices that are up: the NAS stays shown while held, and goes once let go.
  const onlyUp = { ...how, group: null, only: { health: "up" } };
  const shown = arrange(devices, onlyUp);
  assert.deepEqual(names(shown.rows), ["NAS", "Printer"]);
  assert.deepEqual(names(arrange(later, onlyUp, shown.shown).rows), ["NAS", "Printer", "Camera"]);
  assert.deepEqual(names(arrange(later, onlyUp).rows), ["Camera", "Printer"]);
  // A row that was filtered out stays out while held: the TV comes back up, but nothing is put above the row worked in.
  const tvUp = [devices[0], devices[1], { ...devices[2], health: "up" }];
  assert.deepEqual(names(arrange(tvUp, onlyUp, shown.shown).rows), ["NAS", "Printer"]);
});

test("a group whose rows were all held from another value keeps its heading and place", () => {
  const rows = [
    { id: 1, name: "a", health: "down" },
    { id: 2, name: "b", health: "up" },
  ];
  const how = { cols: [COLS.name, COLS.health], filter: "", only: {}, sort: null, group: COLS.health, keyOf: (r) => r.id };
  const first = arrange(rows, how);
  // Both swap: each stays under the heading it was under, in the same order.
  const swapped = [{ ...rows[0], health: "up" }, { ...rows[1], health: "down" }];
  const held = arrange(swapped, how, first.shown);
  assert.deepEqual(held.groups.map((g) => `${g.label}: ${names(g.rows)}`), ["down: a", "up: b"]);
});
