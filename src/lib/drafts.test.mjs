// Run with "npm test" (Node reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { differs, draftFor, fileOnReturn, rebased, replacesFileDraft, sameValues } from "./drafts.ts";

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

test("a file's draft is used for every read of that file, and isn't overwritten by another file unasked", () => {
  const edited = { path: "/srv/media/compose.yml", original: "a: 1\n", text: "a: 2\n" };
  assert.equal(draftFor(edited, "/srv/media/compose.yml"), edited);
  assert.equal(draftFor(edited, "/srv/media/.env"), undefined);
  assert.equal(draftFor(undefined, "/srv/media/.env"), undefined);

  const other = { path: "/srv/media/.env", original: "X=1\n", text: "X=1\n" };
  // Its file couldn't be read again, and another one was opened: the edits stay held.
  assert.ok(!replacesFileDraft(edited, other));
  // Once the owner types into the other file, that is what they are working on now.
  assert.ok(replacesFileDraft(edited, { ...other, text: "X=2\n" }));
  // A draft without edits, or one for the same file, simply follows what is shown.
  assert.ok(replacesFileDraft({ ...edited, text: edited.original }, other));
  assert.ok(replacesFileDraft(edited, { ...edited, text: "a: 3\n" }));
  assert.ok(replacesFileDraft(undefined, other));
});

test("a form kept from before takes what was changed elsewhere in the fields the owner left alone", () => {
  const base = { name: "Printer", notes: "", group: "", sshPort: 22 };
  const draft = { name: "Printer", notes: "by the stairs", group: "", sshPort: 22 };
  // Put in a group with its card's menu while the form was kept: the group follows, the notes stay.
  assert.deepEqual(rebased(draft, base, { ...base, group: "Office" }), { ...draft, group: "Office" });
  // The owner typed a group too: theirs is kept.
  assert.deepEqual(rebased({ ...draft, group: "Lab" }, base, { ...base, group: "Office" }).group, "Lab");
  // Nothing changed elsewhere: the draft as it was.
  assert.deepEqual(rebased(draft, base, base), draft);
  // A field the saved machine has now and the draft didn't know about.
  assert.deepEqual(rebased({ name: "a" }, { name: "a" }, { name: "b", group: "G" }), { name: "b", group: "G" });
});
