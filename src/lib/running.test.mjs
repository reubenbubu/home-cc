// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { OneAtATime } from "./running.ts";

/** A job that ends when told to. */
function job() {
  let end;
  let fail;
  let starts = 0;
  const promise = new Promise((ok, no) => {
    end = ok;
    fail = no;
  });
  return {
    run: () => {
      starts++;
      return promise;
    },
    end: () => end(),
    fail: (e) => fail(e),
    get starts() {
      return starts;
    },
  };
}

const turn = () => new Promise((r) => setImmediate(r));

test("a second job for the same key is not started while the first is going", async () => {
  const jobs = new OneAtATime();
  const first = job();
  const second = job();
  jobs.run(1, first.run);
  const waiting = jobs.run(1, second.run);
  assert.equal(first.starts, 1);
  assert.equal(second.starts, 0);
  assert.equal(jobs.has(1), true);

  let over = false;
  waiting.then(() => (over = true));
  await turn();
  assert.equal(over, false, "the second caller waits for the job that is going");
  first.end();
  await waiting;
  assert.equal(jobs.has(1), false);
  assert.equal(second.starts, 0);
});

test("jobs for different keys run side by side", () => {
  const jobs = new OneAtATime();
  const a = job();
  const b = job();
  jobs.run(1, a.run);
  jobs.run(2, b.run);
  assert.equal(a.starts, 1);
  assert.equal(b.starts, 1);
});

test("settled waits for the job under way, and a job asked for after it does start", async () => {
  // The order the Machines screen needs after an identity is approved: the reading that was already
  // going is let finish, then a new one is made.
  const jobs = new OneAtATime();
  const old = job();
  const fresh = job();
  jobs.run(7, old.run);

  const order = [];
  const approved = (async () => {
    await jobs.settled(7);
    order.push("old reading over");
    const reading = jobs.run(7, fresh.run);
    order.push(`new reading started: ${fresh.starts}`);
    fresh.end();
    await reading;
    order.push("new reading over");
  })();

  await turn();
  assert.deepEqual(order, [], "nothing new is started while the old reading is going");
  old.end();
  await approved;
  assert.deepEqual(order, ["old reading over", "new reading started: 1", "new reading over"]);
  assert.equal(jobs.has(7), false);
});

test("settled ends at once when nothing is going", async () => {
  const jobs = new OneAtATime();
  await jobs.settled(3);
  assert.equal(jobs.has(3), false);
});

test("a job that fails tells the caller that started it, not the ones waiting, and frees the key", async () => {
  const jobs = new OneAtATime();
  const bad = job();
  const started = jobs.run(1, bad.run);
  const waiting = jobs.run(1, bad.run);
  const settled = jobs.settled(1);
  bad.fail(new Error("no"));
  await assert.rejects(started, /no/);
  await waiting;
  await settled;
  assert.equal(jobs.has(1), false);

  const next = job();
  jobs.run(1, next.run);
  assert.equal(next.starts, 1);
});
