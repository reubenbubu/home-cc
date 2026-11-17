// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { ownAddresses, scanRanges } from "./net.ts";

const subnet = (cidr, name, ownIp, best = false) => ({ cidr, interface: name, ownIp, best });
const found = (ip, isSelf) => ({ ip, mac: null, vendor: null, hostname: null, looksLikeMesh: false, rttMs: null, isSelf });

test("one adapter gives one range, labelled with the adapter and this PC's address", () => {
  assert.deepEqual(scanRanges([subnet("192.168.68.0/22", "Ethernet", "192.168.68.60", true)]), [
    { cidr: "192.168.68.0/22", label: "192.168.68.0/22 · Ethernet, this PC is 192.168.68.60" },
  ]);
});

test("cable and Wi-Fi on the same network are one range, not two with the same value", () => {
  const ranges = scanRanges([
    subnet("192.168.68.0/22", "Ethernet", "192.168.68.60", true),
    subnet("172.28.160.0/20", "vEthernet (WSL)", "172.28.160.1"),
    subnet("192.168.68.0/22", "Wi-Fi", "192.168.68.61"),
  ]);
  assert.deepEqual(
    ranges.map((r) => r.cidr),
    ["192.168.68.0/22", "172.28.160.0/20"],
  );
  assert.equal(ranges[0].label, "192.168.68.0/22 · Ethernet and Wi-Fi, this PC is 192.168.68.60 and 192.168.68.61");
  assert.equal(new Set(ranges.map((r) => r.cidr)).size, ranges.length);
});

test("an adapter listed twice with the same address is named once", () => {
  const twice = [subnet("10.0.0.0/24", "eth0", "10.0.0.5"), subnet("10.0.0.0/24", "eth0", "10.0.0.5")];
  assert.deepEqual(scanRanges(twice), [{ cidr: "10.0.0.0/24", label: "10.0.0.0/24 · eth0, this PC is 10.0.0.5" }]);
});

test("missing pieces are left out of the label, and an entry with no range is skipped", () => {
  assert.deepEqual(scanRanges([subnet("10.0.0.0/24", "", "10.0.0.5"), subnet("", "eth9", "10.9.9.9"), subnet("10.1.0.0/24", "", "")]), [
    { cidr: "10.0.0.0/24", label: "10.0.0.0/24 · this PC is 10.0.0.5" },
    { cidr: "10.1.0.0/24", label: "10.1.0.0/24" },
  ]);
  assert.deepEqual(scanRanges([]), []);
});

test("this PC's addresses come from every adapter and from the last scan", () => {
  const subnets = [subnet("192.168.68.0/22", "Ethernet", "192.168.68.60"), subnet("192.168.68.0/22", "Wi-Fi", "192.168.68.61")];
  const own = ownAddresses(subnets, [found("192.168.68.10", false), found("192.168.68.62", true)]);
  assert.deepEqual([...own].sort(), ["192.168.68.60", "192.168.68.61", "192.168.68.62"]);
  assert.equal(own.has("192.168.68.10"), false);
  assert.equal(ownAddresses([]).size, 0);
});
