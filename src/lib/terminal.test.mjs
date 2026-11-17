// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { clipboardKey, decodeBase64, Drawn, EarlyOutput, endedText, InOrder, rightClickAction, tabTitle } from "./terminal.ts";

const key = (k, mods = {}) => ({ key: k, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods });

test("output decodes to the exact bytes, even half a character", () => {
  // "é" is two bytes; each half arrives on its own and must not be turned into a replacement character.
  assert.deepEqual([...decodeBase64(Buffer.from([0xc3]).toString("base64"))], [0xc3]);
  assert.deepEqual([...decodeBase64(Buffer.from([0xa9]).toString("base64"))], [0xa9]);
  const all = Buffer.from([0, 27, 91, 65, 255, 128, 10]);
  assert.deepEqual([...decodeBase64(all.toString("base64"))], [...all]);
  assert.equal(decodeBase64("").length, 0);
});

test("Ctrl+Shift+C and Ctrl+Shift+V copy and paste, whichever way the letter comes", () => {
  assert.equal(clipboardKey(key("C", { ctrlKey: true, shiftKey: true }), false), "copy");
  assert.equal(clipboardKey(key("c", { ctrlKey: true, shiftKey: true }), true), "copy");
  assert.equal(clipboardKey(key("V", { ctrlKey: true, shiftKey: true }), false), "paste");
  assert.equal(clipboardKey(key("Insert", { ctrlKey: true }), true), "copy");
  assert.equal(clipboardKey(key("Insert", { shiftKey: true }), false), "paste");
});

test("Ctrl+C copies only while text is selected; otherwise it interrupts the program", () => {
  assert.equal(clipboardKey(key("c", { ctrlKey: true }), true), "copy");
  assert.equal(clipboardKey(key("c", { ctrlKey: true }), false), null);
});

test("Ctrl+V, Escape, arrows and other keys go to the program", () => {
  assert.equal(clipboardKey(key("v", { ctrlKey: true }), false), null);
  assert.equal(clipboardKey(key("v", { ctrlKey: true }), true), null);
  for (const k of ["Escape", "ArrowUp", "Tab", "a", "F10", "Insert"]) assert.equal(clipboardKey(key(k), true), null, k);
  assert.equal(clipboardKey(key("x", { ctrlKey: true, shiftKey: true }), true), null);
  assert.equal(clipboardKey(key("c", { ctrlKey: true, altKey: true }), true), null);
  assert.equal(clipboardKey(key("c", { metaKey: true }), true), null);
});

test("right-click copies a selection, pastes without one, and leaves a program its mouse", () => {
  assert.equal(rightClickAction(true, false, false), "copy");
  assert.equal(rightClickAction(false, false, false), "paste");
  assert.equal(rightClickAction(true, true, false), null);
  assert.equal(rightClickAction(false, true, true), "paste");
});

test("drawn output is reported in steps, with nothing lost", () => {
  const drawn = new Drawn(100);
  assert.equal(drawn.add(40), 0);
  assert.equal(drawn.add(40), 0);
  assert.equal(drawn.add(40), 120);
  assert.equal(drawn.add(99), 0);
  assert.equal(drawn.add(1), 100);
});

test("typing is sent in order, one piece at a time, joining what came meanwhile", async () => {
  const sent = [];
  const replies = [];
  const queue = new InOrder((data) => {
    sent.push(data);
    return new Promise((resolve) => replies.push(resolve));
  });
  queue.push("l");
  queue.push("s");
  queue.push(" -l\r");
  assert.deepEqual(sent, ["l"]);
  replies.shift()();
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(sent, ["l", "s -l\r"]);
  replies.shift()();
  await new Promise((r) => setTimeout(r, 0));
  queue.push("x");
  assert.deepEqual(sent, ["l", "s -l\r", "x"]);
});

test("a failed send is reported and the next piece still goes", async () => {
  const sent = [];
  const errors = [];
  const queue = new InOrder(
    async (data) => {
      sent.push(data);
      if (data === "bad") throw "This terminal has already closed.";
    },
    (e) => errors.push(e),
  );
  queue.push("bad");
  await new Promise((r) => setTimeout(r, 0));
  queue.push("ok");
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(sent, ["bad", "ok"]);
  assert.deepEqual(errors, ["This terminal has already closed."]);
  queue.stop();
  queue.push("after");
  assert.deepEqual(sent, ["bad", "ok"]);
});

test("output that comes before its tab knows it is kept, with the end, until claimed", () => {
  const early = new EarlyOutput(10);
  early.output(5, Uint8Array.of(1, 2, 3), 1000);
  early.output(5, Uint8Array.of(4), 1001);
  early.exit(5, "The program ended with exit code 1.", 1, 1002);
  const got = early.take(5);
  assert.deepEqual(got.chunks.map((c) => [...c]), [[1, 2, 3], [4]]);
  assert.deepEqual(got.exit, { reason: "The program ended with exit code 1.", exitStatus: 1 });
  assert.equal(early.take(5), null, "taken only once");
  assert.equal(early.take(6), null);
});

test("early output has a limit, and unclaimed terminals are given up after a while", () => {
  const early = new EarlyOutput(4);
  early.output(1, Uint8Array.of(1, 2, 3), 0);
  early.output(1, Uint8Array.of(4, 5), 0);
  early.output(2, Uint8Array.of(9), 5000);
  early.exit(3, null, 0, 0);
  assert.deepEqual(early.unclaimed(3000, 4000), [
    { id: 1, running: true },
    { id: 3, running: false },
  ]);
  assert.equal(early.take(1), null);
  assert.equal(early.take(2).bytes, 1);
});

test("tabs to the same machine are numbered", () => {
  assert.equal(tabTitle("NAS", []), "NAS");
  assert.equal(tabTitle("NAS", ["NAS"]), "NAS (2)");
  assert.equal(tabTitle("NAS", ["NAS", "NAS (2)"]), "NAS (3)");
  assert.equal(tabTitle("  ", []), "Terminal");
});

test("an ended session says why", () => {
  assert.equal(endedText(null), "Session ended.");
  assert.equal(endedText("The connection to nas was lost."), "Session ended. The connection to nas was lost.");
});
