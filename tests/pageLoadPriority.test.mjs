import assert from "node:assert/strict";
import test from "node:test";
import { AsyncPageCache } from "../src/asyncPageCache.ts";
const tick = () => new Promise(resolve => setImmediate(resolve));
function controlled(concurrency = 2) {
  const starts = [], finishes = new Map();
  const cache = new AsyncPageCache(8, (page, priority) => {
    starts.push([page, priority]);
    return new Promise((resolve, reject) => finishes.set(page, { resolve, reject }));
  }, { maxConcurrent: concurrency });
  return { cache, starts, finishes };
}

test("optional page reads are bounded and visible pages are promoted ahead of idle work", async () => {
  const { cache, starts, finishes } = controlled();
  const idle = [1, 2, 3, 4].map(page => cache.load(page, "idle"));
  await tick();
  assert.deepEqual(starts, [[1, "idle"], [2, "idle"]]);
  const shared = cache.load(4, "visible");
  assert.equal(shared, idle[3]);
  finishes.get(1).resolve(1);
  await tick();
  assert.deepEqual(starts.at(-1), [4, "visible"]);
  finishes.get(2).resolve(2); finishes.get(4).resolve(4);
  await tick();
  finishes.get(3).resolve(3);
  assert.deepEqual(await Promise.all(idle), [1, 2, 3, 4]);
  assert.equal(cache.pendingCount, 0);
});

test("cancelling stale preview windows preserves shared visible and running requests", async () => {
  const { cache, starts, finishes } = controlled(1);
  const running = cache.load(1, "idle");
  await tick();
  const stale = cache.load(2, "neighbor");
  const staleOutcome = assert.rejects(stale, { name: "AbortError" });
  const shared = cache.load(3, "idle");
  assert.equal(cache.load(3, "visible"), shared);
  const kept = cache.load(4, "neighbor");
  cache.retainPending([4]);
  await staleOutcome;
  assert.equal(cache.pendingCount, 3);
  finishes.get(1).resolve(1); await running; await tick();
  assert.deepEqual(starts.at(-1), [3, "visible"]);
  finishes.get(3).resolve(3); await shared; await tick();
  assert.deepEqual(starts.at(-1), [4, "neighbor"]);
  finishes.get(4).resolve(4); await kept;
  assert.equal(cache.peek(2), undefined);
});

test("repeated navigation drops optional queued work rather than growing a backlog", async () => {
  const { cache, finishes } = controlled(1);
  const running = cache.load(1, "idle"); await tick();
  const queued = [];
  for (let page = 2; page <= 604; page++) {
    cache.retainPending([page]);
    queued.push(cache.load(page, "idle").catch(error => { assert.equal(error.name, "AbortError"); }));
    assert.ok(cache.pendingCount <= 2);
  }
  cache.retainPending([]);
  finishes.get(1).resolve(1);
  await Promise.all([running, ...queued]);
  assert.equal(cache.pendingCount, 0);
});

test("failed reads free a slot and can be retried with visible priority", async () => {
  const { cache, finishes, starts } = controlled(1);
  const failed = cache.load(1, "idle");
  const failedOutcome = assert.rejects(failed, /unavailable/);
  await tick();
  const next = cache.load(2, "neighbor");
  finishes.get(1).reject(new Error("unavailable")); await failedOutcome; await tick();
  assert.deepEqual(starts.at(-1), [2, "neighbor"]);
  finishes.get(2).resolve(2); await next;
  const retry = cache.load(1); await tick(); finishes.get(1).resolve(1);
  assert.equal(await retry, 1);
});
