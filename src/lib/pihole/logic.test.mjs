// Run with "npm test" (Node reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  blockingView,
  clockText,
  compareLists,
  differenceText,
  duration,
  number,
  pauseWords,
  percent,
  piholeIssues,
  portFor,
  suggestedByName,
  timeLeft,
} from "./logic.ts";

const list = (address, more = {}) => ({ address, kind: "block", enabled: true, comment: null, domains: 1000, status: 2, ...more });

test("pauses are worded plainly", () => {
  assert.equal(pauseWords(300), "for 5 minutes");
  assert.equal(pauseWords(30), "for 30 seconds");
  assert.equal(pauseWords(null), "until you turn it back on");
  assert.equal(pauseWords(5400), "for 1 hour 30 minutes");
  assert.equal(duration(1), "1 second");
  assert.equal(duration(60), "1 minute");
  assert.equal(duration(7200), "2 hours");
});

test("the time left counts down from the reading", () => {
  assert.equal(timeLeft(300, 1000, 1000), 300);
  assert.equal(timeLeft(300, 1000, 1060), 240);
  assert.equal(timeLeft(300, 1000, 2000), 0, "never below 0");
  assert.equal(timeLeft(null, 1000, 1060), null);
  assert.equal(clockText(45), "0:45");
  assert.equal(clockText(252), "4:12");
  assert.equal(clockText(3723), "1:02:03");
});

test("blocking is described with what comes next", () => {
  assert.deepEqual(blockingView({ state: "enabled", timerSecs: null }, 1000, 1000), { tone: "ok", label: "Blocking", detail: "", paused: false });
  const paused = blockingView({ state: "disabled", timerSecs: 300 }, 1000, 1048);
  assert.deepEqual(paused, { tone: "warn", label: "Paused", detail: "back on in 4:12", paused: true });
  assert.equal(blockingView({ state: "disabled", timerSecs: 30 }, 1000, 1100).detail, "back on any moment");
  const off = blockingView({ state: "disabled", timerSecs: null }, 1000, 1000);
  assert.deepEqual(off, { tone: "bad", label: "Blocking off", detail: "until you turn it back on", paused: true });
  assert.equal(blockingView({ state: "failed", timerSecs: null }, 0, 0).tone, "bad");
  assert.equal(blockingView({ state: "unknown", timerSecs: null }, 0, 0).tone, "idle");
});

test("numbers are shown the same everywhere", () => {
  assert.equal(number(48213), "48,213");
  assert.equal(number(null), "–");
  assert.equal(percent(20.475805), "20.5%");
  assert.equal(percent(0), "0.0%");
});

test("the lists of two Pi-holes are compared", () => {
  const a = { id: 1, name: "Pi-hole 1", lists: [list("https://a.example/hosts"), list("https://b.example/list.txt"), list("https://c.example/x", { enabled: false }), list("https://allow.example/ok", { kind: "allow" })] };
  const b = { id: 2, name: "Pi-hole 2", lists: [list(" https://a.example/hosts "), list("https://c.example/x"), list("https://allow.example/ok", { kind: "allow" }), list("https://allow.example/extra", { kind: "allow" })] };
  const c = compareLists([a, b]);
  assert.deepEqual(c.compared, [{ id: 1, name: "Pi-hole 1" }, { id: 2, name: "Pi-hole 2" }]);
  assert.equal(c.same, 2, "the hosts file (spaces aside) and the allowlist are on both");
  assert.deepEqual(
    c.differences.map((r) => [r.kind, r.address, r.on[1], r.on[2]]),
    [
      ["block", "https://b.example/list.txt", "on", "missing"],
      ["block", "https://c.example/x", "off", "on"],
      ["allow", "https://allow.example/extra", "missing", "on"],
    ],
  );
  assert.deepEqual(
    c.differences.map((r) => differenceText(r, c.compared)),
    ["Only on Pi-hole 1", "On Pi-hole 2; switched off on Pi-hole 1", "Only on Pi-hole 2"],
  );
});

test("the same list type matters, and nothing to compare gives null", () => {
  const a = { id: 1, name: "A", lists: [list("https://x.example/l")] };
  const b = { id: 2, name: "B", lists: [list("https://x.example/l", { kind: "allow" })] };
  assert.equal(compareLists([a, b]).differences.length, 2, "a blocklist and an allowlist at one address are two lists");
  assert.equal(compareLists([a, { id: 2, name: "B", lists: null }]), null, "one Pi-hole's lists couldn't be read");
  assert.equal(compareLists([a]), null);
  const same = compareLists([a, { id: 2, name: "B", lists: [list("https://x.example/l")] }]);
  assert.deepEqual([same.same, same.differences.length], [1, 0]);
});

test("three Pi-holes: what each one lacks", () => {
  const hosts = [
    { id: 1, name: "A", lists: [list("https://x.example/l", { enabled: false })] },
    { id: 2, name: "B", lists: [] },
    { id: 3, name: "C", lists: [list("https://x.example/l"), list("https://y.example/l")] },
  ];
  const c = compareLists(hosts);
  const [x, y] = c.differences;
  assert.equal(differenceText(x, c.compared), "On C; switched off on A; missing on B");
  assert.equal(differenceText(y, c.compared), "Only on C");
});

const reading = (blocking) => ({
  summary: { queries: 100, blocked: 20, percentBlocked: 20, clientsActive: 3, clientsTotal: 4, domainsOnLists: 10, gravityUpdated: 1 },
  blocking,
  lists: [],
  listsError: null,
  version: null,
});
const status = (more) => ({
  deviceId: 1,
  reading: null,
  fetchedAt: 1000,
  error: null,
  errorKind: null,
  offeredFingerprint: null,
  expectedFingerprint: null,
  pinnedFingerprint: null,
  https: true,
  port: 443,
  ...more,
});

test("the Overview is quiet when every Pi-hole is fine", () => {
  const fine = [
    { id: 1, name: "Pi-hole 1", status: status({ reading: reading({ state: "enabled", timerSecs: null }) }) },
    { id: 2, name: "Pi-hole 2", status: null },
  ];
  assert.deepEqual(piholeIssues(fine, 1000), []);
});

test("the Overview flags a Pi-hole that can't be read, or whose blocking is off", () => {
  const issues = piholeIssues(
    [
      { id: 1, name: "Pi-hole 1", status: status({ reading: reading({ state: "disabled", timerSecs: 300 }) }) },
      { id: 2, name: "Pi-hole 2", status: status({ error: "Couldn't connect", errorKind: "unreachable" }) },
      { id: 3, name: "Pi-hole 3", status: status({ reading: reading({ state: "disabled", timerSecs: null }) }) },
      { id: 4, name: "Pi-hole 4", status: null, readError: "Not found" },
      { id: 5, name: "Pi-hole 5", status: status({ error: "Pi-hole didn't accept the password.", errorKind: "auth" }) },
    ],
    1060,
  );
  assert.deepEqual(
    issues.map((i) => i.text),
    [
      "Pi-hole 1: paused, back on in 4:00",
      "Pi-hole 2 can't be reached",
      "Pi-hole 3: blocking off, until you turn it back on",
      "Pi-hole 4 couldn't be read",
      "Pi-hole 5 didn't accept the password",
    ],
  );
  assert.equal(new Set(issues.map((i) => i.key)).size, issues.length);
});

test("machines named like a Pi-hole are suggested, once", () => {
  const d = (id, name, hostname = null, kind = "container") => ({ id, name, hostname, kind });
  const devices = [d(1, "Pi-hole 1"), d(2, "dns", "pihole2"), d(3, "PI_HOLE backup"), d(4, "Phone"), d(5, "pihole old", null, "ignore"), d(6, "pi hole")];
  assert.deepEqual(suggestedByName(devices, new Set([1])).map((x) => x.id), [2, 3, 6]);
});

test("switching between HTTPS and HTTP takes Pi-hole's own port, unless another one was typed", () => {
  assert.equal(portFor(false, 443), 80);
  assert.equal(portFor(true, 80), 443);
  assert.equal(portFor(false, 8443), 8443);
});
