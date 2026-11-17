// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { AREA_FILES, AREA_REACH, MAX_LINES, PROMPT_KINDS, agentPrompt, fenced, promptKinds, reachOf } from "./report.ts";

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

// --- Three prompts: Investigate, Remediate, I have a bug ---

const nas = {
  area: "docker",
  what: 'Reading the containers on "nas"',
  error: "The container homeassistant keeps restarting.",
  details: {
    Device: "nas",
    Address: "192.168.68.20",
    "SSH user": "reuben",
    "SSH port": 2222,
    "Resolved hostname": "nas.lan",
    "Docker version": "27.1",
  },
};

test("a problem about a machine or service offers all three prompts, one about home-cc itself only the bug one", () => {
  assert.deepEqual(promptKinds(nas), ["investigate", "remediate", "bug"]);
  assert.deepEqual(promptKinds(proxmox), ["investigate", "remediate", "bug"], "stats read over SSH");
  assert.deepEqual(promptKinds({ area: "status", what: "Checking whether devices are up", error: "Pings are not allowed." }), [
    "investigate",
    "remediate",
    "bug",
  ]);
  for (const area of ["devices", "secrets", "settings", "live", "zoom", "screen"]) {
    assert.deepEqual(promptKinds({ ...proxmox, area }), ["bug"], area);
  }
  // In a machine's area, but with no machine named (the terminals' clipboard, a list that wouldn't load).
  assert.deepEqual(promptKinds({ area: "terminal", what: "Copying or pasting in a terminal tab", error: "denied" }), ["bug"]);
  assert.deepEqual(promptKinds({ ...nas, details: { "Docker version": "27.1" } }), ["bug"]);
  assert.deepEqual(promptKinds({ area: "status", what: "Loading device status", error: "x", inHomeCc: true }), ["bug"]);
  assert.deepEqual(promptKinds({ ...nas, inHomeCc: true }), ["bug"]);
});

test("every area says whether it is about a machine, this PC or home-cc, and every prompt has its words", () => {
  assert.deepEqual(Object.keys(AREA_REACH).sort(), Object.keys(AREA_FILES).sort());
  for (const k of ["investigate", "remediate", "bug"]) assert.ok(PROMPT_KINDS[k].label && PROMPT_KINDS[k].asks, k);
});

test("the machine is named with how to reach it: address, hostname, SSH user and port, or the service's own address", () => {
  assert.deepEqual(reachOf(nas), {
    on: "machine",
    name: "nas",
    address: "192.168.68.20",
    hostname: "nas.lan",
    ssh: { user: "reuben", port: 2222 },
  });
  // No SSH user: no login offered. A hostname that is a sentence ("none found") is not a hostname.
  const plain = reachOf({ ...nas, area: "stats", details: { Device: "deco", Address: "192.168.68.1", "Resolved hostname": "none found" } });
  assert.deepEqual(plain, { on: "machine", name: "deco", address: "192.168.68.1" });
  const pve = reachOf({ area: "proxmox", what: "x", error: "y", details: { Device: "pve", Address: "192.168.68.54", "API port": 8006 } });
  assert.equal(pve.api.url, "https://192.168.68.54:8006");
  const ph = reachOf({ area: "pihole", what: "x", error: "y", details: { Device: "ph", Address: "192.168.68.2", Connection: "plain HTTP", Port: 80 } });
  assert.equal(ph.api.url, "http://192.168.68.2:80/admin");
  // This PC's own stats are read without SSH: the agent works here.
  assert.deepEqual(reachOf({ ...proxmox, details: { Device: "desk", Address: "127.0.0.1", "Read on this PC without SSH": true } }), {
    on: "this PC",
    name: "desk",
  });
  assert.deepEqual(reachOf({ area: "scan", what: "Scanning", error: "no network" }), { on: "this PC" });
});

test("Investigate asks to look on the machine and change nothing, with the same details as the bug prompt", () => {
  const text = agentPrompt(nas, ctx, "investigate");
  assert.ok(text.includes('about my machine "nas"'));
  assert.ok(text.includes("Don't change anything."));
  assert.ok(text.includes("change nothing"));
  assert.ok(text.includes("## Where it is"));
  assert.ok(text.includes('- Machine: "nas" at 192.168.68.20 (hostname nas.lan)'));
  assert.ok(text.includes("`ssh -p 2222 reuben@192.168.68.20`"));
  assert.ok(text.includes("- Docker: run docker and docker compose there"));
  assert.ok(text.includes(nas.error), "message kept whole");
  assert.ok(text.includes("- Docker version: 27.1"), "the details");
  assert.ok(text.includes("- Built from commit: e2296f1"), "the app");
  assert.ok(text.includes("## home-cc's code, if the cause turns out to be there"));
  for (const f of AREA_FILES.docker) assert.ok(text.includes(`- ${f}`), f);
  assert.ok(!text.includes("Please find out why it happened"), "not the bug prompt");
  assert.ok(text.includes("No passwords or other secrets are included."));
  assert.ok(text.includes("never ask me to paste one here"));
});

test("Remediate asks to fix it on the machine, saying first what will change and asking before anything destructive", () => {
  const text = agentPrompt(nas, ctx, "remediate");
  assert.ok(text.includes("Please fix it there."));
  assert.ok(text.includes("Before you change anything, tell me what you will change and why."));
  assert.ok(text.includes("Ask me, and wait for my yes, before anything destructive"));
  assert.ok(text.includes("`ssh -p 2222 reuben@192.168.68.20`"));
  assert.ok(!text.includes("Don't change anything."));
});

test("the bug prompt is unchanged by the other two, and they fall back to it for a problem about home-cc", () => {
  const bug = agentPrompt(nas, ctx);
  assert.equal(agentPrompt(nas, ctx, "bug"), bug);
  assert.ok(bug.includes("Please find out why it happened."));
  assert.ok(bug.includes("## Where to start looking"));
  assert.ok(!bug.includes("## Where it is"));
  const settingsProblem = { area: "settings", what: "Saving the settings", error: "Not a number" };
  assert.equal(agentPrompt(settingsProblem, ctx, "remediate"), agentPrompt(settingsProblem, ctx));
});

test("a machine without SSH, a service with its own address, and this PC are each reached their own way", () => {
  const deco = agentPrompt({ area: "stats", what: "x", error: "y", details: { Device: "deco", Address: "192.168.68.1" } }, ctx, "investigate");
  assert.ok(deco.includes("home-cc has no SSH login for it."));
  assert.ok(!deco.includes("ssh -p"));
  const pve = agentPrompt(
    { area: "proxmox", what: "x", error: "y", details: { Device: "pve", Address: "192.168.68.54", "SSH user": "root", "API port": 8006, "Token ID": "root@pam!home-cc" } },
    ctx,
    "remediate",
  );
  assert.ok(pve.includes("- Proxmox's web page and API: https://192.168.68.54:8006 (the token or password home-cc uses is not included)"));
  assert.ok(pve.includes("`ssh -p 22 root@192.168.68.54`"), "port 22 when none is given");
  const pings = agentPrompt({ area: "status", what: "Checking whether devices are up", error: "Pings are not allowed." }, ctx, "investigate");
  assert.ok(pings.includes("about this PC, the one home-cc runs on, or the network around it"));
  assert.ok(pings.includes("Work here; no SSH is needed."));
  assert.ok(!pings.includes("- Machine:"));
});

test("each of the three says no secret is included, and names only whether a password is stored", () => {
  const withStored = { ...nas, details: { ...nas.details, "Password stored for it": true } };
  for (const kind of ["investigate", "remediate", "bug"]) {
    const text = agentPrompt(withStored, ctx, kind);
    assert.ok(text.includes("- Password stored for it: yes"), kind);
    assert.ok(text.includes("No passwords or other secrets are included."), kind);
  }
});
