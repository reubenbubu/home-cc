// Run with "npm test" (Node reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { mayReplace, offersAuto, systemAfterSave, systemLabel, systemNote, systemSource, systemTitle, systemToSend } from "./system.ts";

const LABELS = { windows: "Windows", linux: "Linux", proxmox: "Proxmox VE", other: "Other" };
const at = (os, osSource) => ({ os, osSource });

// The same cases as a_stronger_answer_replaces_a_weaker_one_and_nothing_replaces_the_owner in inventory.rs.
test("a stronger answer replaces a weaker one, and nothing replaces the owner's choice", () => {
  const found = ["this-pc", "ssh", "proxmox", "network"];
  for (const source of found) assert.ok(mayReplace(at("other", null), "linux", source), source);
  assert.ok(!mayReplace(at("other", null), "other", "ssh"), "Other says nothing");
  assert.ok(!mayReplace(at("other", null), "linux", "manual"));
  for (const source of found) {
    assert.ok(!mayReplace(at("linux", "manual"), "windows", source), source);
    assert.ok(!mayReplace(at("other", "manual"), "windows", source), `Other chosen on purpose, ${source}`);
  }
  assert.ok(mayReplace(at("windows", "network"), "linux", "ssh"));
  assert.ok(mayReplace(at("linux", "network"), "windows", "network"), "a newer guess");
  assert.ok(mayReplace(at("linux", "ssh"), "windows", "ssh"), "reinstalled since");
  assert.ok(mayReplace(at("linux", "ssh"), "windows", "this-pc"));
  assert.ok(mayReplace(at("linux", "network"), "proxmox", "proxmox"));
  assert.ok(!mayReplace(at("linux", "ssh"), "windows", "network"));
  assert.ok(!mayReplace(at("linux", "this-pc"), "windows", "ssh"));
  assert.ok(!mayReplace(at("windows", "ssh"), "proxmox", "proxmox"));
  assert.ok(!mayReplace(at("proxmox", "proxmox"), "linux", "ssh"), "Proxmox's API answering isn't undone");
  assert.ok(mayReplace(at("proxmox", "network"), "linux", "ssh"), "only guessed from its web port");
});

// The same cases as the_system_chosen_in_the_form_is_the_owners_and_stays in inventory.rs.
test("the System chosen in the form is the owner's, and one left alone stays as found", () => {
  // A new machine.
  assert.deepEqual(systemAfterSave(null, { os: "linux" }), at("linux", "manual"));
  assert.deepEqual(systemAfterSave(null, { os: "other" }), at("other", null));
  assert.deepEqual(systemAfterSave(null, { os: "windows", osAuto: true }), at("other", null));
  // Untouched: what is stored stays, also when it was found after the form opened.
  const found = at("linux", "ssh");
  assert.deepEqual(systemAfterSave(found, { os: "linux" }), found);
  assert.deepEqual(systemAfterSave(found, { os: "other", osWas: "other" }), found);
  // Changed: the owner's, Other too.
  assert.deepEqual(systemAfterSave(found, { os: "windows", osWas: "linux" }), at("windows", "manual"));
  assert.deepEqual(systemAfterSave(at("windows", "manual"), { os: "other", osWas: "windows" }), at("other", "manual"));
  // Without osWas, compared with what is stored.
  assert.deepEqual(systemAfterSave(at("linux", "network"), { os: "windows" }), at("windows", "manual"));
  // Let home-cc work it out.
  assert.deepEqual(systemAfterSave(at("other", "manual"), { os: "other", osWas: "other", osAuto: true }), at("other", null));
  // Pointed at another address: what was found at the old one goes; the owner's choice stays.
  assert.deepEqual(systemAfterSave(found, { os: "other", osWas: "other" }, true), at("other", null));
  assert.deepEqual(systemAfterSave(at("windows", "manual"), { os: "windows", osWas: "windows" }, true), at("windows", "manual"));
});

test("the System says where it came from", () => {
  assert.equal(systemLabel(at("linux", "ssh"), LABELS), "Linux");
  assert.equal(systemLabel(at("windows", "network"), LABELS), "Windows (guessed)");
  assert.equal(systemLabel(at("other", null), LABELS), "Other");
  assert.equal(systemSource(at("linux", "ssh")), "Found from its SSH login");
  assert.equal(systemSource(at("windows", "network")), "Guessed from the network");
  assert.equal(systemSource(at("proxmox", "proxmox")), "Found from its Proxmox API");
  assert.equal(systemSource(at("windows", "this-pc")), "Found on this PC, where home-cc runs");
  assert.equal(systemSource(at("linux", "manual")), "Set by you");
  assert.equal(systemSource(at("other", null)), "Not known yet");
  assert.equal(systemTitle(at("windows", "network"), LABELS), "System: Windows · Guessed from the network");
});

test("the form's note and its Let home-cc work it out", () => {
  const form = (os, osAuto = false) => ({ os, osAuto });
  // A new machine left at Other, or given a System.
  assert.match(systemNote(form("other"), null, "other"), /^home-cc works it out from its SSH login or the network\. Choose one/);
  assert.equal(systemNote(form("linux"), null, "other"), "Set by you.");
  // As stored.
  assert.equal(systemNote(form("linux"), at("linux", "ssh"), "linux"), "Found from its SSH login.");
  assert.match(systemNote(form("windows"), at("windows", "network"), "windows"), /^Guessed from the network\./);
  assert.equal(systemNote(form("linux"), at("linux", "manual"), "linux"), "Set by you. home-cc won't change it.");
  assert.match(systemNote(form("other"), at("other", null), "other"), /^Not known yet: home-cc works it out/);
  // Changed in the form, or left to home-cc.
  assert.equal(systemNote(form("windows"), at("linux", "ssh"), "linux"), "Set by you once you save. home-cc won't change it.");
  assert.match(systemNote(form("other", true), at("linux", "manual"), "linux"), /^Saving lets home-cc work it out/);

  // Offered only for a System the owner set, until it is pressed.
  assert.ok(offersAuto(form("linux"), at("linux", "manual")));
  assert.ok(offersAuto(form("other"), at("other", "manual")), "Other chosen on purpose");
  assert.ok(!offersAuto(form("other", true), at("linux", "manual")));
  assert.ok(!offersAuto(form("linux"), at("linux", "ssh")));
  assert.ok(!offersAuto(form("other"), at("other", null)));
  assert.ok(!offersAuto(form("other"), null), "a new machine");
});

test("the form sends the System it started from with the one chosen", () => {
  assert.deepEqual(systemToSend({ os: "windows" }, "linux"), { os: "windows", osWas: "linux", osAuto: false });
  assert.deepEqual(systemToSend({ os: "linux", osAuto: true }, "linux"), { os: "other", osWas: "linux", osAuto: true });
});
