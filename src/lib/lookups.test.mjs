// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { BATCH, isIpv4, KEEP_MS, Lookups } from "./lookups.ts";

/** A back end that knows two names, records every request, and answers when told to. */
function setup() {
  let time = 1_000_000;
  const requests = [];
  const pending = [];
  const runs = [];
  const lookups = new Lookups({
    send: (addresses) => {
      requests.push(addresses);
      return new Promise((resolve, reject) => pending.push({ addresses, resolve, reject }));
    },
    schedule: (run) => runs.push(run),
    now: () => time,
  });
  const names = { "192.168.68.10": "nas", "192.168.68.21": "homelab" };
  return {
    lookups,
    requests,
    /** Ends the current moment: what was asked for is sent. */
    tick: () => runs.splice(0).forEach((run) => run()),
    /** The back end answers every open request. */
    answerAll: async () => {
      for (const p of pending.splice(0)) p.resolve(p.addresses.map((address) => ({ address, name: names[address] ?? null })));
      await new Promise((r) => setTimeout(r, 0));
    },
    failAll: async (why) => {
      for (const p of pending.splice(0)) p.reject(why);
      await new Promise((r) => setTimeout(r, 0));
    },
    later: (ms) => (time += ms),
  };
}

test("only IPv4 addresses are looked up", () => {
  for (const yes of ["192.168.68.10", " 10.0.0.1 ", "0.0.0.0", "255.255.255.255"]) assert.ok(isIpv4(yes), yes);
  for (const no of ["nas.lan", "fe80::1", "", "192.168.68", "192.168.68.256", "1.2.3.4.5", "a.b.c.d"]) assert.ok(!isIpv4(no), no);
});

test("everything asked for in one moment goes in one request, each address once", async () => {
  const t = setup();
  assert.equal(t.lookups.get("192.168.68.10"), undefined);
  t.lookups.get("192.168.68.21");
  t.lookups.get(" 192.168.68.10 ");
  t.lookups.get("nas.lan");
  t.lookups.get("192.168.68.99");
  assert.deepEqual(t.requests, [], "nothing is sent while the screen is being drawn");
  t.tick();
  assert.deepEqual(t.requests, [["192.168.68.10", "192.168.68.21", "192.168.68.99"]]);

  // Asked for again before the answer is in: not sent a second time.
  t.lookups.get("192.168.68.10");
  t.tick();
  assert.equal(t.requests.length, 1);

  await t.answerAll();
  assert.equal(t.lookups.get("192.168.68.10")?.name, "nas");
  assert.equal(t.lookups.get("192.168.68.21")?.name, "homelab");
  const none = t.lookups.get("192.168.68.99");
  assert.ok(none && none.name === null && none.error === undefined, "no name found is an answer too");
  t.tick();
  assert.equal(t.requests.length, 1, "answered addresses are not asked about again");
});

test("a long list is sent in requests the back end accepts", () => {
  const t = setup();
  for (let i = 0; i < BATCH + 10; i++) t.lookups.get(`10.0.${Math.floor(i / 200)}.${(i % 200) + 1}`);
  t.tick();
  assert.deepEqual(t.requests.map((r) => r.length), [BATCH, 10]);
});

test("old answers are asked about again, and stay on screen until the new one is in", async () => {
  const t = setup();
  t.lookups.get("192.168.68.10");
  t.lookups.get("192.168.68.99");
  t.tick();
  await t.answerAll();

  t.later(KEEP_MS.none);
  assert.equal(t.lookups.get("192.168.68.99")?.name, null);
  assert.equal(t.lookups.get("192.168.68.10")?.name, "nas");
  t.tick();
  assert.deepEqual(t.requests.at(-1), ["192.168.68.99"], "only the one without a name is old yet");
  await t.answerAll();

  t.later(KEEP_MS.name);
  assert.equal(t.lookups.get("192.168.68.10")?.name, "nas");
  t.tick();
  assert.deepEqual(t.requests.at(-1), ["192.168.68.10"]);
});

test("a failed lookup says so, keeps a name found before, and is tried again later", async () => {
  const t = setup();
  t.lookups.get("192.168.68.10");
  t.lookups.get("192.168.68.99");
  t.tick();
  await t.failAll("The back end is busy");
  assert.deepEqual(t.lookups.get("192.168.68.99"), { name: null, at: 1_000_000, error: "The back end is busy" });
  t.tick();
  assert.equal(t.requests.length, 1, "not hammered straight away");

  t.later(KEEP_MS.error);
  t.lookups.get("192.168.68.10");
  t.tick();
  await t.answerAll();
  assert.equal(t.lookups.get("192.168.68.10")?.name, "nas");

  t.later(KEEP_MS.name);
  t.lookups.get("192.168.68.10");
  t.tick();
  await t.failAll(new Error("gone"));
  const kept = t.lookups.get("192.168.68.10");
  assert.equal(kept?.name, "nas");
  assert.equal(kept?.error, undefined);
});
