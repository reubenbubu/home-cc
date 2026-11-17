// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { isThisPc, macToSend, ownAddresses, scanRanges } from "./net.ts";

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

test("a device the back end marks as this PC is this PC, whatever its address looks like", () => {
  const own = new Set(["192.168.68.60"]);
  assert.equal(isThisPc("localhost", true, true, own), true);
  assert.equal(isThisPc("main-pc.lan", true, true, new Set()), true);
});

test("before the back end has marked any device, the guess from this PC's addresses stands in", () => {
  const own = new Set(["192.168.68.60"]);
  assert.equal(isThisPc(" 192.168.68.60 ", false, false, own), true);
  // Not checked yet at all: there is no status to ask.
  assert.equal(isThisPc("192.168.68.60", undefined, false, own), true);
  assert.equal(isThisPc("192.168.68.10", false, false, own), false);
});

test("once the back end has marked a device, the guess is dropped for the others", () => {
  // A stale list of this PC's addresses must not label another device "this PC".
  const stale = new Set(["192.168.68.60"]);
  assert.equal(isThisPc("192.168.68.60", false, true, stale), false);
  assert.equal(isThisPc("192.168.68.60", undefined, true, stale), false);
});

test("a hardware address box left as it was sends nothing, so the stored one is kept", () => {
  assert.equal(macToSend("a0:36:bc:5e:11:90", "a0:36:bc:5e:11:90"), null);
  assert.equal(macToSend("a0:36:bc:5e:11:90", "  A0:36:BC:5E:11:90 "), null);
  assert.equal(macToSend(null, ""), null);
  assert.equal(macToSend(null, "   "), null);
});

test("an emptied hardware address box sends an empty text, which removes the stored one", () => {
  assert.equal(macToSend("a0:36:bc:5e:11:90", ""), "");
  assert.equal(macToSend("a0:36:bc:5e:11:90", "   "), "");
});

test("a changed hardware address is sent as typed, for the back end to check", () => {
  assert.equal(macToSend("a0:36:bc:5e:11:90", " a0:36:bc:5e:11:91 "), "a0:36:bc:5e:11:91");
  assert.equal(macToSend(null, "aa-bb-cc-dd-ee-ff"), "aa-bb-cc-dd-ee-ff");
  assert.equal(macToSend(null, "not an address"), "not an address");
});
