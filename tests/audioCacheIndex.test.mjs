import test from 'node:test';
import assert from 'node:assert/strict';
import { AudioCacheIndex } from '../src/audioCacheIndex.ts';

function fixture(count = 20) {
  const entries = new Map(Array.from({ length: count }, (_, index) => [`https://audio.test/${index}.mp3`, index + 10]));
  let active = 0, maximum = 0, matches = 0, cancelled = 0, keys = 0;
  const cache = {
    async keys() { keys++; return [...entries.keys()].map(url => new Request(url)); },
    async match(request) {
      matches++; active++; maximum = Math.max(maximum, active);
      await new Promise(done => setTimeout(done, 2)); active--;
      const bytes = entries.get(request.url);
      if (bytes === undefined) return undefined;
      return new Response(new ReadableStream({ cancel() { cancelled++; } }), { headers: { 'content-length': String(bytes) } });
    },
  };
  return { cache, entries, stats: () => ({ maximum, matches, cancelled, keys }) };
}

test('offline metadata listing is bounded, coalesced and never retains unread response bodies', async () => {
  const index = new AudioCacheIndex(3), f = fixture();
  const [first, shared] = await Promise.all([index.read(f.cache), index.read(f.cache)]);
  assert.deepEqual(shared, first);
  assert.equal(first.urls.length, 20); assert.equal(first.bytes, 390);
  assert.deepEqual(f.stats(), { maximum: 3, matches: 20, cancelled: 20, keys: 1 });
  await index.read(f.cache);
  assert.equal(f.stats().matches, 20, 'known headers are not reread when the menu refreshes');
  f.entries.delete('https://audio.test/0.mp3'); f.entries.set('https://audio.test/new.mp3', 100);
  const changed = await index.read(f.cache);
  assert.equal(changed.bytes, 480); assert.equal(f.stats().matches, 21);
  assert.ok(!changed.urls.includes('https://audio.test/0.mp3'), 'evicted cache entries leave the listing');
});

test('download quota validation rechecks stored sizes and completed writes can update session metadata', async () => {
  const index = new AudioCacheIndex(), f = fixture(2);
  await index.read(f.cache);
  f.entries.set('https://audio.test/0.mp3', 500);
  assert.equal((await index.read(f.cache, true)).bytes, 511, 'quota checks read current headers instead of trusting session metadata');
  f.entries.set('https://audio.test/completed.mp3', 200); index.remember('https://audio.test/completed.mp3', 200);
  const matches = f.stats().matches;
  assert.equal((await index.read(f.cache)).bytes, 711); assert.equal(f.stats().matches, matches);
  index.forget('https://audio.test/completed.mp3'); await index.read(f.cache);
  assert.equal(f.stats().matches, matches + 1);
  index.forget(); await index.read(f.cache);
  assert.equal(f.stats().matches, matches + 4);
});

test('invalidations during an older listing cannot restore obsolete metadata, and failed reads are retryable', async () => {
  const index = new AudioCacheIndex(1), f = fixture(1);
  const pending = index.read(f.cache); index.forget(); await pending;
  await index.read(f.cache); assert.equal(f.stats().matches, 2, 'an invalidated in-flight scan is not memoized');
  index.forget();
  const match = f.cache.match; f.cache.match = async () => { throw Error('storage unavailable'); };
  await assert.rejects(index.read(f.cache), /storage unavailable/);
  f.cache.match = match;
  assert.equal((await index.read(f.cache)).bytes, 10);
  assert.throws(() => new AudioCacheIndex(0), /concurrency/);
});

test('a refresh after cache removal cannot join an obsolete in-flight listing', async () => {
  const index = new AudioCacheIndex(1), f = fixture(5);
  const old = index.read(f.cache);
  index.forget();
  const current = index.read(f.cache);
  await Promise.all([old, current]);
  assert.equal(f.stats().keys, 2, 'the post-invalidation caller reconciles current keys instead of accepting the older snapshot');
});

test('failed scans keep ownership until other workers finish, so immediate retries cannot stack reads', async () => {
  const index = new AudioCacheIndex(4);
  let release; const gate = new Promise(done => { release = done; });
  let calls = 0, active = 0, maximum = 0;
  const cache = {
    async keys() { return Array.from({ length: 8 }, (_, i) => new Request(`https://audio.test/${i}.mp3`)); },
    async match() {
      const call = calls++; active++; maximum = Math.max(maximum, active);
      try {
        if (call === 0) throw Error('header failure');
        await gate;
        return new Response('', { headers: { 'content-length': '1' } });
      } finally { active--; }
    },
  };
  const failed = assert.rejects(index.read(cache), /header failure/);
  await new Promise(done => setImmediate(done));
  const immediate = assert.rejects(index.read(cache), /header failure/);
  await new Promise(done => setImmediate(done));
  assert.equal(calls, 4, 'the retry joins the still-owned failed batch');
  assert.equal(active, 3); assert.ok(maximum <= 4, 'a synchronous failure may release its read before the other workers start');
  release(); await Promise.all([failed, immediate]);
  assert.equal(calls, 4, 'failure stops workers from starting additional entries');
  assert.equal((await index.read(cache)).bytes, 8, 'a later retry succeeds after the failed batch settles');
  assert.ok(maximum <= 4, 'the configured concurrency cap applies across failures and retries');
});
