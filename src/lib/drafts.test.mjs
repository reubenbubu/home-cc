// Run with "npm test" (Node reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { differs, fileOnReturn, sameValues } from "./drafts.ts";

test("the same values, looking inside lists and objects", () => {
  assert.ok(sameValues("a", "a"));
  assert.ok(sameValues(8006, 8006));
  assert.ok(sameValues(NaN, NaN));
  assert.ok(sameValues(null, null));
  assert.ok(sameValues({ a: 1, b: ["x", { c: true }] }, { b: ["x", { c: true }], a: 1 }));
  // A field left out is the same as one that is undefined.
  assert.ok(sameValues({ a: 1 }, { a: 1, b: undefined }));

  assert.ok(!sameValues("8006", 8006));
  assert.ok(!sameValues(null, undefined));
  assert.ok(!sameValues(null, {}));
  assert.ok(!sameValues([1, 2], [2, 1]));
  assert.ok(!sameValues([1], { 0: 1 }));
  assert.ok(!sameValues({ a: [1, 2] }, { a: [1, 2, 3] }));
  assert.ok(!sameValues({ a: { b: "x" } }, { a: { b: "y" } }));
});

test("a draft is kept only when it holds something that isn't saved", () => {
  const saved = { tokenId: "root@pam!homecc", port: 8006, secret: "" };
  assert.ok(!differs({ ...saved }, saved));
  assert.ok(differs({ ...saved, secret: "s3cret" }, saved));
  assert.ok(differs({ ...saved, port: 8007 }, saved));
  // Nothing read from the back end yet: what was typed is kept.
  assert.ok(differs({ ...saved }, undefined));
});

test("a file seen again keeps the owner's text, and says when the host's copy changed meanwhile", () => {
  // Not edited: whatever is on the host now is shown.
  assert.deepEqual(fileOnReturn({ original: "a", text: "a" }, "b"), { original: "b", text: "b", hostChanged: false });
  // Edited, host unchanged: the owner's text, nothing to say.
  assert.deepEqual(fileOnReturn({ original: "a", text: "a2" }, "a"), { original: "a", text: "a2", hostChanged: false });
  // Edited, and the host's copy changed too: the owner's text stays, and the screen says so.
  assert.deepEqual(fileOnReturn({ original: "a", text: "a2" }, "b"), { original: "b", text: "a2", hostChanged: true });
  // The host now holds exactly what the owner typed (saved elsewhere): nothing left unsaved.
  assert.deepEqual(fileOnReturn({ original: "a", text: "b" }, "b"), { original: "b", text: "b", hostChanged: false });
});
