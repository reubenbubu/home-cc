// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  guestKey,
  guestName,
  lockLabel,
  powerActions,
  snapshotNameProblem,
  stateLabel,
  suggestedSnapshotName,
  tokenCommands,
} from "./proxmox.ts";

const guest = (more = {}) => ({
  vmid: 110,
  kind: "qemu",
  name: "homeassistant",
  node: "pve",
  status: "running",
  template: false,
  lock: null,
  cpuPct: 3,
  cpuCount: 2,
  memUsed: 1,
  memTotal: 2,
  diskUsed: null,
  diskTotal: 3,
  uptimeSecs: 60,
  tags: [],
  ...more,
});

test("only the power buttons that fit the guest's state are offered", () => {
  assert.deepEqual(powerActions(guest()), ["shutdown", "reboot", "stop"]);
  assert.deepEqual(powerActions(guest({ status: "stopped" })), ["start"]);
  assert.deepEqual(powerActions(guest({ status: "paused" })), ["shutdown", "stop"]);
  assert.deepEqual(powerActions(guest({ status: "unknown" })), ["start", "stop"]);
  assert.deepEqual(powerActions(guest({ status: "stopped", template: true })), []);
});

test("guests are named and described plainly", () => {
  assert.equal(guestName(guest()), "homeassistant");
  assert.equal(guestName(guest({ name: " " })), "VM 110");
  assert.equal(guestName(guest({ name: "", kind: "lxc", vmid: 101 })), "Container 101");
  assert.equal(stateLabel(guest()), "Running");
  assert.equal(stateLabel(guest({ template: true, status: "stopped" })), "Template");
  assert.equal(stateLabel(guest({ status: "" })), "Unknown");
  assert.equal(lockLabel(null), null);
  assert.equal(lockLabel("backup"), "Backup running");
  assert.equal(lockLabel("something-new"), "Busy (something-new)");
  assert.equal(guestKey({ node: "pve", kind: "lxc", vmid: 101 }), "pve/lxc/101");
});

test("snapshot names follow Proxmox's rules", () => {
  assert.equal(snapshotNameProblem("before_update"), null);
  assert.equal(snapshotNameProblem("  b4  "), null);
  assert.equal(snapshotNameProblem("a".repeat(40)), null);
  assert.match(snapshotNameProblem(""), /name/);
  for (const bad of ["a", "1st", "with space", "dots.no", "a".repeat(41), "ünicode"]) {
    assert.ok(snapshotNameProblem(bad), bad);
  }
  assert.match(snapshotNameProblem("Current"), /present state/);
  // The suggestion is always a good name.
  const suggestion = suggestedSnapshotName(new Date(2026, 10, 7, 9, 5));
  assert.equal(suggestion, "snap_20261107_0905");
  assert.equal(snapshotNameProblem(suggestion), null);
});

test("the token commands use the token typed, or the usual one", () => {
  assert.deepEqual(tokenCommands("homecc@pve!screen"), {
    create: "pveum user token add homecc@pve screen --privsep 1",
    grant: "pveum acl modify / --tokens 'homecc@pve!screen' --roles PVEAuditor,PVEVMAdmin",
  });
  for (const unusable of ["", "root@pam", "root@pam!x=secret", "it's me@pam!x"]) {
    assert.equal(tokenCommands(unusable).create, "pveum user token add root@pam homecc --privsep 1", unusable);
  }
});
