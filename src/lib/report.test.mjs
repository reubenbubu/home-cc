// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { AREA_FILES, MAX_LINES, agentPrompt, fenced } from "./report.ts";

const proxmox = {
  area: "stats",
  what: 'Reading stats from "proxmox" over SSH',
  error:
    "Could not log in to 192.168.68.54 as reuben. The key file id_rsa was not accepted. Check the user name, and that " +
    "this PC's public key has been added to that machine, or store a password for it.",
  details: { Device: "proxmox", Address: "192.168.68.54", "SSH user": "reuben", "Password stored for it": false, Maker: null },
};
const ctx = { version: "0.1.0", os: "windows", arch: "x86_64", release: true, commit: "e2296f1", screen: "/machines", now: 1_800_000_000 };

test("the prompt holds the message word for word, the details, the app and where to look", () => {
  const text = agentPrompt(proxmox, ctx);
  assert.ok(text.includes(proxmox.error), "message kept whole");
  assert.ok(text.includes('Reading stats from "proxmox" over SSH. home-cc showed this message:'));
  assert.ok(text.includes("- SSH user: reuben"));
  assert.ok(text.includes("- Password stored for it: no"), "booleans read as yes or no");
  assert.ok(!text.includes("Maker"), "empty details are left out");
  assert.ok(text.includes("- home-cc 0.1.0\n"));
  assert.ok(text.includes("- Built from commit: e2296f1"));
  assert.ok(text.includes("- Running on: windows (x86_64)"));
  assert.ok(text.includes("- Screen: /machines"));
  for (const f of AREA_FILES.stats) assert.ok(text.includes(`- ${f}`), f);
  assert.ok(text.includes("https://github.com/reubenbubu/home-cc"));
});

test("missing parts are said, not invented", () => {
  const text = agentPrompt(proxmox, { now: ctx.now, missing: ["The back-end log: not allowed"] });
  assert.ok(text.includes("home-cc, version not known"));
  assert.ok(text.includes("- Built from commit: not known"));
  assert.ok(text.includes("## Not included\n\n- The back-end log: not allowed"));
  assert.ok(!text.includes("Running on"));
});

test("a development build is named as one", () => {
  assert.ok(agentPrompt(proxmox, { ...ctx, release: false }).includes("- home-cc 0.1.0, development build"));
});

test("long lists keep only the newest lines and say how many were left out", () => {
  const recent = Array.from({ length: MAX_LINES + 3 }, (_, i) => ({ at: ctx.now - 100 + i, what: `try ${i}`, error: "x\n  y" }));
  const backendLog = Array.from({ length: MAX_LINES + 2 }, (_, i) => ({ at: ctx.now, text: `line ${i}` }));
  const text = agentPrompt(proxmox, { ...ctx, recent, backendLog });
  assert.ok(text.includes("(3 older ones left out)"));
  assert.ok(text.includes("(2 older lines left out)"));
  assert.ok(!text.includes("try 0:") && text.includes(`try ${MAX_LINES + 2}: x y`), "newest kept, on one line");
  assert.ok(!text.includes(" line 1\n") && text.includes(` line ${MAX_LINES + 1}\n`));
});

test("a stack trace and a standing problem get their own sections", () => {
  const text = agentPrompt({ ...proxmox, area: "screen", trace: "TypeError: x\n  at f (app.js:1:2)" }, { ...ctx, checkProblem: "Pings are not allowed." });
  assert.ok(text.includes("## Stack trace\n\n```text\nTypeError: x\n  at f (app.js:1:2)\n```"));
  assert.ok(text.includes("## Standing problem"));
});

test("backticks in a message can't close its block early", () => {
  assert.equal(fenced("a ``` b"), "````text\na ``` b\n````");
  assert.equal(fenced("plain"), "```text\nplain\n```");
});
