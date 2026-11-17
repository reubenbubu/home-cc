// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  addWheel,
  clampZoom,
  parseZoom,
  terminalBaseSize,
  terminalFontSize,
  WHEEL_STEP,
  zoomIn,
  zoomKeyAction,
  zoomOut,
  zoomShows,
  zoomTarget,
  ZOOM_STEPS,
} from "./zoom.ts";

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

test("a window that has zoomed is told apart from one that was only resized", () => {
  const before = { ratio: 1.25, width: 1536 };
  // WebView2: both the pixel ratio and the page width follow the zoom.
  assert.equal(zoomShows(before, { ratio: 2.5, width: 768 }, 200), true);
  // A window that only changes the width.
  assert.equal(zoomShows(before, { ratio: 1.25, width: 1229 }, 125), true);
  assert.equal(zoomShows({ ratio: 1, width: 1000 }, { ratio: 1, width: 667 }, 150), true);
  // Not yet zoomed, or only made bigger or smaller.
  assert.equal(zoomShows(before, before, 200), false);
  assert.equal(zoomShows(before, { ratio: 1.25, width: 1280 }, 200), false);
  // Nothing to wait for at the normal size.
  assert.equal(zoomShows(before, before, 100), true);
});

test("in a terminal the keys and the wheel change its text; everywhere else, the app", () => {
  const where = (focusInTerminal, pointerInTerminal) => ({ focusInTerminal, pointerInTerminal });
  assert.equal(zoomTarget("key", where(true, false)), "terminal");
  assert.equal(zoomTarget("key", where(false, true)), "app");
  // The wheel follows the pointer, not the keyboard.
  assert.equal(zoomTarget("wheel", where(false, true)), "terminal");
  assert.equal(zoomTarget("wheel", where(true, false)), "app");
  assert.equal(zoomTarget("key", where(false, false)), "app");
});

test("the terminals' normal text size is a little larger than the app's", () => {
  assert.equal(terminalBaseSize(13), 14);
  assert.equal(terminalBaseSize(16), 17.5);
  assert.equal(terminalBaseSize(NaN), 14);
  assert.equal(terminalBaseSize(0), 14);
});

test("a terminal's text is the same size on the screen whatever the app is zoomed to", () => {
  // At 100% the app and the terminal agree.
  assert.equal(terminalFontSize(13, 100, 100), 14);
  // The app at 175%: the terminal is given less, so it looks the same.
  assert.equal(terminalFontSize(13, 100, 175), 8);
  assert.equal(terminalFontSize(13, 100, 67), 20.9);
  assert.equal(terminalFontSize(13, 150, 200), 10.5);
  for (const app of ZOOM_STEPS) {
    for (const term of ZOOM_STEPS) {
      const onScreen = (terminalFontSize(13, term, app) * app) / 100;
      const want = (14 * term) / 100;
      assert.ok(Math.abs(onScreen - want) < 0.02, `terminal ${term}% with the app at ${app}%: ${onScreen}px, not ${want}px`);
    }
  }
});

test("zooming the app back and forth leaves the terminal where it was", () => {
  const start = terminalFontSize(13, 110, 100);
  // Up to 200%, down to 50%, and back up to 100%, one step at a time.
  let app = 100;
  for (let i = 0; i < 10; i++) app = zoomIn(app);
  for (let i = 0; i < 10; i++) app = zoomOut(app);
  for (let i = 0; i < 5; i++) app = zoomIn(app);
  assert.equal(app, 100);
  assert.equal(terminalFontSize(13, 110, app), start);
  // Odd sizes are kept in range, as the app's are.
  assert.equal(terminalFontSize(13, 900, 100), 28);
  assert.equal(terminalFontSize(13, NaN, 100), 14);
});
