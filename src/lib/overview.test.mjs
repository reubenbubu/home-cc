// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { byKind, kindHeading } from "./overview.ts";

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
