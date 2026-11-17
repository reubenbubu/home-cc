// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { byKind, keepSilences, kindHeading, parseSilences, stillSilenced } from "./overview.ts";

test("the Overview's cards come under a heading per type, PCs first, appliances left for the tiles", () => {
  const list = [
    { name: "Pi-hole 1", kind: "container" },
    { name: "Dev VM", kind: "vm" },
    { name: "Main PC", kind: "pc" },
    { name: "Printer", kind: "appliance" },
    { name: "Homelab VM", kind: "vm" },
    { name: "NAS", kind: "server" },
  ];
  const got = byKind(list).map((g) => [g.kind, g.items.map((d) => d.name)]);
  assert.deepEqual(got, [
    ["pc", ["Main PC"]],
    ["server", ["NAS"]],
    ["vm", ["Dev VM", "Homelab VM"]],
    ["container", ["Pi-hole 1"]],
  ]);
  // A type with nothing in it has no heading.
  assert.deepEqual(byKind([{ kind: "vm" }, { kind: "appliance" }]).map((g) => g.kind), ["vm"]);
  assert.deepEqual(byKind([]), []);
});

test("a type's heading says one or many", () => {
  assert.equal(kindHeading("pc", 1), "PC");
  assert.equal(kindHeading("pc", 3), "PCs");
  assert.equal(kindHeading("vm", 2), "VMs");
  assert.equal(kindHeading("container", 0), "Containers");
});

test("saved silences are read back, and anything damaged counts as none", () => {
  assert.deepEqual(parseSilences(JSON.stringify([{ id: 4, at: 1000 }, { id: 9, at: 1200 }])), [
    { id: 4, at: 1000 },
    { id: 9, at: 1200 },
  ]);
  assert.deepEqual(parseSilences(null), []);
  assert.deepEqual(parseSilences(""), []);
  assert.deepEqual(parseSilences("not json"), []);
  assert.deepEqual(parseSilences('{"id":4,"at":1000}'), []);
  // Entries that aren't a machine and a time are skipped; a machine listed twice keeps the first.
  const mixed = [{ id: 4, at: 1000 }, { id: 4, at: 2000 }, { id: 0, at: 5 }, { id: 2.5, at: 5 }, { id: 3 }, { id: 3, at: "1" }, null, 7, { id: 6, at: 0 }];
  assert.deepEqual(parseSilences(JSON.stringify(mixed)), [
    { id: 4, at: 1000 },
    { id: 6, at: 0 },
  ]);
});

test("a silence holds until the machine is up again, also when it was up only while nobody looked", () => {
  const s = { id: 4, at: 1000 };
  // Still down, last seen before it was silenced (or never).
  assert.equal(stillSilenced(s, { id: 4, health: "down", lastSeen: 900 }), true);
  assert.equal(stillSilenced(s, { id: 4, health: "down", lastSeen: null }), true);
  // Not checked yet (the app just started): kept.
  assert.equal(stillSilenced(s, { id: 4, health: "unknown", lastSeen: 900 }), true);
  // Up now.
  assert.equal(stillSilenced(s, { id: 4, health: "up", lastSeen: 1100 }), false);
  // Down now, but seen up after it was silenced: it came back and went again, so it counts as down.
  assert.equal(stillSilenced(s, { id: 4, health: "down", lastSeen: 1030 }), false);
});

test("silences of machines that are up again or no longer exist are dropped", () => {
  const list = [
    { id: 1, at: 1000 },
    { id: 2, at: 1000 },
    { id: 3, at: 1000 },
    { id: 4, at: 1000 },
    { id: 5, at: 1000 },
  ];
  const devices = [
    { id: 1, health: "down", lastSeen: 500 },
    { id: 2, health: "up", lastSeen: 1060 },
    { id: 3, health: "down", lastSeen: 1060 },
    { id: 5, health: "unknown", lastSeen: null },
  ];
  assert.deepEqual(keepSilences(list, devices), [
    { id: 1, at: 1000 },
    { id: 5, at: 1000 },
  ]);
  assert.deepEqual(keepSilences(list, []), []);
  assert.deepEqual(keepSilences([], devices), []);
});
