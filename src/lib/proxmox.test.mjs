// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  actionWords,
  consoleCommand,
  consoleTitle,
  devicesAt,
  guestKey,
  guestName,
  isHibernated,
  lockBlocks,
  lockLabel,
  powerActions,
  rollbackPlan,
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
  // Proxmox won't shut down a paused VM; it has to be resumed.
  assert.deepEqual(powerActions(guest({ status: "paused" })), ["resume", "stop"]);
  assert.deepEqual(powerActions(guest({ status: "suspended" })), ["resume", "stop"]);
  assert.deepEqual(powerActions(guest({ status: "unknown" })), ["start", "stop"]);
  assert.deepEqual(powerActions(guest({ status: "stopped", template: true })), []);
});

test("whatever stops a guest asks first; starting and resuming don't", () => {
  for (const [action, confirm] of [["shutdown", "Yes, shut down"], ["reboot", "Yes, restart"], ["stop", "Yes, force stop"]]) {
    assert.equal(actionWords(guest(), action).confirm, confirm, action);
  }
  assert.equal(actionWords(guest({ status: "stopped" }), "start").confirm, null);
  assert.equal(actionWords(guest({ status: "paused" }), "resume").confirm, null);
  assert.deepEqual(actionWords(guest({ status: "paused" }), "resume"), { label: "Resume", confirm: null, busy: "Resuming…", done: "Resumed." });
});

test("a hibernated VM can be started, which resumes it; other locks hold everything back", () => {
  const hibernated = guest({ status: "stopped", lock: "suspended" });
  assert.equal(isHibernated(hibernated), true);
  assert.deepEqual(powerActions(hibernated), ["start"]);
  assert.equal(lockBlocks(hibernated, "start"), null);
  assert.equal(actionWords(hibernated, "start").label, "Resume");
  assert.equal(actionWords(guest({ status: "stopped" }), "start").label, "Start");
  assert.equal(lockLabel("suspended"), "Hibernated");

  const backingUp = guest({ lock: "backup" });
  for (const action of ["shutdown", "reboot", "stop"]) assert.equal(lockBlocks(backingUp, action), "Backup running", action);
  assert.equal(lockBlocks(guest({ status: "stopped", lock: "backup" }), "start"), "Backup running");
  assert.equal(lockBlocks(guest(), "shutdown"), null);
});

test("rolling back says when the guest will be stopped and started again", () => {
  const bare = { name: "clean", withRam: false };
  const withRam = { name: "live", withRam: true };
  // Proxmox stops a running guest to roll back to a snapshot without RAM and leaves it stopped.
  assert.deepEqual(rollbackPlan(guest(), bare), {
    startAfter: true,
    confirm: "Yes, roll back and restart",
    done: 'Rolled back to "clean" and started again.',
  });
  assert.equal(rollbackPlan(guest({ status: "paused" }), bare).startAfter, true);
  assert.deepEqual(rollbackPlan(guest({ status: "stopped" }), bare), { startAfter: false, confirm: "Yes, roll back", done: 'Rolled back to "clean".' });
  // A snapshot with RAM comes back running by itself, even from stopped.
  assert.equal(rollbackPlan(guest(), withRam).startAfter, false);
  assert.equal(rollbackPlan(guest(), withRam).confirm, "Yes, roll back");
  assert.equal(rollbackPlan(guest({ status: "stopped" }), withRam).confirm, "Yes, roll back and start");
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

test("a console runs pct enter or qm terminal, and only for a real guest ID", () => {
  assert.equal(consoleCommand({ kind: "lxc", vmid: 101 }), "pct enter 101");
  assert.equal(consoleCommand({ kind: "qemu", vmid: 110 }), "qm terminal 110");
  for (const vmid of [99, 0, -5, 1.5, NaN, 1e12, Number("101; reboot")]) {
    assert.equal(consoleCommand({ kind: "lxc", vmid }), null, String(vmid));
  }
  assert.equal(consoleTitle(guest({ kind: "lxc", vmid: 101, name: "pihole1" })), "pihole1 (CT 101)");
  assert.equal(consoleTitle(guest({ kind: "qemu", vmid: 110, name: "" })), "VM 110 (VM 110)");
});

test("a guest's addresses are matched to the machines in home-cc", () => {
  const dev = (id, host, kind = "vm") => ({ id, host, kind, name: `d${id}` });
  const list = [dev(1, "192.168.68.21"), dev(2, " 192.168.68.22 "), dev(3, "192.168.68.23", "ignore")];
  assert.deepEqual(
    devicesAt(["192.168.68.21", "192.168.68.22", "192.168.68.23", "10.0.0.1"], list).map((x) => x.device?.id ?? null),
    [1, 2, null, null],
  );
});
