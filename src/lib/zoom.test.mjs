// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { addWheel, clampZoom, parseZoom, WHEEL_STEP, zoomIn, zoomKeyAction, zoomOut, ZOOM_STEPS } from "./zoom.ts";

test("sizes stay whole percentages from 50 to 200", () => {
  assert.equal(clampZoom(100), 100);
  assert.equal(clampZoom(10), 50);
  assert.equal(clampZoom(500), 200);
  assert.equal(clampZoom(133.4), 133);
  assert.equal(clampZoom(NaN), 100);
  assert.equal(clampZoom(Infinity), 100);
  assert.equal(clampZoom("150"), 100);
});

test("zooming in and out walks the steps, and stops at the ends", () => {
  const up = [50];
  while (up.at(-1) < 200) up.push(zoomIn(up.at(-1)));
  assert.deepEqual(up, ZOOM_STEPS);
  const down = [200];
  while (down.at(-1) > 50) down.push(zoomOut(down.at(-1)));
  assert.deepEqual(down, [...ZOOM_STEPS].reverse());
  assert.equal(zoomIn(200), 200);
  assert.equal(zoomOut(50), 50);
});

test("from a size between two steps, the next step is the nearest one that way", () => {
  assert.equal(zoomIn(130), 150);
  assert.equal(zoomOut(130), 125);
  assert.equal(zoomIn(1000), 200);
  assert.equal(zoomOut(1), 50);
});

test("the remembered size: damaged or missing means 100%", () => {
  assert.equal(parseZoom("150"), 150);
  assert.equal(parseZoom(" 75 "), 75);
  assert.equal(parseZoom("900"), 200);
  assert.equal(parseZoom(null), 100);
  assert.equal(parseZoom(""), 100);
  assert.equal(parseZoom("big"), 100);
});

test("the zoom keys", () => {
  const key = (key, extra = {}) => zoomKeyAction({ key, ctrlKey: true, ...extra });
  assert.equal(key("="), "in");
  assert.equal(key("+"), "in");
  assert.equal(key("-"), "out");
  assert.equal(key("0"), "reset");
  assert.equal(key("9"), null);
  assert.equal(key("a"), null);
  // Without Ctrl it is just typing.
  assert.equal(zoomKeyAction({ key: "+", ctrlKey: false }), null);
  // AltGr is Ctrl and Alt together, and types characters on many keyboards.
  assert.equal(key("0", { altKey: true }), null);
});

test("one notch of a mouse wheel is one step, away from you is bigger", () => {
  assert.deepEqual(addWheel(0, -100), { total: 0, action: "in" });
  assert.deepEqual(addWheel(0, 100), { total: 0, action: "out" });
  // Some mice report about half a step per notch.
  assert.deepEqual(addWheel(0, 53), { total: 0, action: "out" });
});

test("small touchpad turns add up to one step", () => {
  let total = 0;
  const actions = [];
  for (let i = 0; i < 30; i++) {
    const r = addWheel(total, -10);
    total = r.total;
    if (r.action) actions.push(r.action);
  }
  assert.deepEqual(actions, ["in", "in", "in"]);
  assert.ok(Math.abs(total) < WHEEL_STEP);
});

test("turning the other way starts again from nothing", () => {
  const r = addWheel(-90, 20);
  assert.deepEqual(r, { total: 20, action: null });
});
