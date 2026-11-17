// Run with "npm test" (Node reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { startingUser, userAfterKind } from "./newmachine.ts";

test("a new machine starts with the default SSH user, or none", () => {
  assert.equal(startingUser("vm", "sam"), "sam");
  assert.equal(startingUser("pc", " sam "), "sam");
  assert.equal(startingUser("server", ""), "");
  // No name written into the app: nothing set means an empty field.
  assert.equal(startingUser("vm", "   "), "");
  // Most appliances have no SSH.
  assert.equal(startingUser("appliance", "sam"), "");
});

test("the default SSH user follows the type until the owner types another", () => {
  // Untouched: goes for an appliance, comes back after.
  assert.equal(userAfterKind("sam", "vm", "appliance", "sam"), "");
  assert.equal(userAfterKind("", "appliance", "pc", "sam"), "sam");
  assert.equal(userAfterKind("sam", "vm", "server", "sam"), "sam");
  // Typed by the owner: stays, whatever the type.
  assert.equal(userAfterKind("admin", "vm", "appliance", "sam"), "admin");
  assert.equal(userAfterKind("admin", "appliance", "pc", "sam"), "admin");
  // An appliance given a user by hand keeps it.
  assert.equal(userAfterKind("root", "appliance", "server", "sam"), "root");
  // Emptied by the owner: stays empty.
  assert.equal(userAfterKind("", "pc", "server", "sam"), "");
  assert.equal(userAfterKind("", "pc", "appliance", "sam"), "");
  // With no default, nothing is ever filled in.
  assert.equal(userAfterKind("", "vm", "pc", ""), "");
  assert.equal(userAfterKind("", "appliance", "vm", ""), "");
});
