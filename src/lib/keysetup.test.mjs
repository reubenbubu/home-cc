// Run with "npm test" (Node reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { manualSteps, passwordSource, psQuote, setupBlocker, shQuote, whatHappens } from "./keysetup.ts";

const KEY = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGn3x0Xc8kq sam@DESKTOP-MAIN";

const device = (more = {}) => ({
  id: 9,
  name: "proxmox",
  host: "192.168.68.54",
  kind: "server",
  os: "proxmox",
  sshUser: "reuben",
  sshPort: 22,
  notes: "",
  mac: null,
  vendor: null,
  hostname: null,
  source: "manual",
  reviewed: true,
  firstSeen: 0,
  lastSeen: null,
  ...more,
});
const typed = (more = {}) => ({ host: "192.168.68.54", user: "reuben", port: 22, ...more });

test("quotes keep the text whole in each shell", () => {
  assert.equal(shQuote("a b"), "'a b'");
  assert.equal(shQuote("it's"), "'it'\\''s'");
  assert.equal(psQuote("it's"), "'it''s'");
});

test("by hand on Linux: one command that makes the folder and adds the key with the right permissions", () => {
  const steps = manualSteps(` ${KEY}\n`, "proxmox");
  assert.equal(steps.length, 1);
  assert.equal(
    steps[0].command,
    `mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo '${KEY}' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys`,
  );
  // A quote in the key can't end the quoted text early.
  assert.ok(manualSteps("ssh-ed25519 AAAA x'; rm -rf ~; '", "linux")[0].command.includes(`'ssh-ed25519 AAAA x'\\''; rm -rf ~; '\\'''`));
});

test("by hand on Windows: the administrators file with its permissions, or the user's own file", () => {
  const [admin, user] = manualSteps(KEY, "windows");
  assert.match(admin.where, /administrator/);
  assert.ok(admin.command.includes('"$env:ProgramData\\ssh\\administrators_authorized_keys"'));
  assert.ok(admin.command.includes(`-Value '${KEY}'`));
  assert.ok(admin.command.includes("/inheritance:r /grant '*S-1-5-32-544:F' '*S-1-5-18:F'"));
  assert.ok(user.command.includes('"$env:USERPROFILE\\.ssh\\authorized_keys"'));
  assert.ok(!user.command.includes("icacls"));
  // ascii, so Windows PowerShell doesn't write UTF-16, which the SSH server can't read.
  for (const s of [admin, user]) assert.ok(s.command.includes("-Encoding ascii"));
});

test("setting up waits for a saved machine with a user and for this PC's key", () => {
  assert.match(setupBlocker(null, typed(), true), /Save this machine first/);
  assert.match(setupBlocker(device({ sshUser: null }), typed({ user: "" }), true), /Add an SSH user/);
  assert.match(setupBlocker(device({ sshUser: "  " }), typed(), true), /Save the SSH user/);
  assert.match(setupBlocker(device(), typed({ host: "192.168.68.55" }), true), /Save your changes/);
  assert.match(setupBlocker(device(), typed({ user: "root" }), true), /Save your changes/);
  assert.match(setupBlocker(device(), typed({ port: 2222 }), true), /Save your changes/);
  assert.match(setupBlocker(device(), typed(), false), /needs an SSH key/);
  assert.equal(setupBlocker(device(), typed({ host: " 192.168.68.54 ", user: "reuben " }), true), null);
  // A port field holds text while it is typed in.
  assert.equal(setupBlocker(device(), typed({ port: "22" }), true), null);
});

test("the typed password is named before the stored one", () => {
  assert.equal(passwordSource(true, true), "the password typed above");
  assert.equal(passwordSource(false, true), "the stored password");
  assert.equal(passwordSource(false, false), null);
});

test("what happens names the file on each kind of machine", () => {
  const linux = whatHappens("linux", "reuben", "the stored password");
  assert.match(linux, /with the stored password/);
  assert.match(linux, /~\/\.ssh\/authorized_keys for reuben/);
  assert.match(linux, /with the key alone/);
  const windows = whatHappens("windows", "sam", null);
  assert.match(windows, /administrators_authorized_keys if sam is an administrator/);
  assert.match(windows, /asks you for the password/);
});
