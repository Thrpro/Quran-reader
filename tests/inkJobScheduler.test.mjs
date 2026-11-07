import test from 'node:test';
import assert from 'node:assert/strict';
import { InkJobScheduler } from '../src/inkJobScheduler.ts';

function fixture(batch = 4) {
  const deferred = [], yielded = new Map(), painted = [];
  let serial = 0;
  const scheduler = new InkJobScheduler({
    current: job => job.current !== false,
    visible: job => job.visible === true,
    paint: jobs => painted.push(jobs.map(job => job.id)),
    optionalBatch: batch,
    defer: callback => deferred.push(callback),
    yieldTask: callback => { yielded.set(++serial, callback); return serial; },
    cancelTask: timer => yielded.delete(timer),
  });
  const microtask = () => { const callbacks = deferred.splice(0); callbacks.forEach(callback => callback()); };
  const yieldOnce = () => { const [timer, callback] = yielded.entries().next().value; yielded.delete(timer); callback(); };
  return { scheduler, painted, yielded, deferred, microtask, yieldOnce };
}

test('visible artwork paints together before any optional preview job', () => {
  const f = fixture();
  for (const job of [{id:'preview'}, {id:'visible-1',visible:true}, {id:'visible-2',visible:true}]) f.scheduler.schedule(job);
  assert.equal(f.deferred.length, 1, 'scheduling coalesces the visible paint pass');
  f.microtask();
  assert.deepEqual(f.painted, [['visible-1','visible-2']]);
  assert.equal(f.yielded.size, 1);
  f.yieldOnce();
  assert.deepEqual(f.painted.at(-1), ['preview']);
  assert.equal(f.yielded.size, 0);
});

test('preview batches are bounded and yield between paint passes', () => {
  const f = fixture();
  for (let id = 0; id < 10; id++) f.scheduler.schedule({id});
  f.microtask(); assert.equal(f.painted.length, 0);
  f.yieldOnce(); assert.deepEqual(f.painted.at(-1), [0,1,2,3]);
  f.yieldOnce(); assert.deepEqual(f.painted.at(-1), [4,5,6,7]);
  f.yieldOnce(); assert.deepEqual(f.painted.at(-1), [8,9]);
  assert.equal(f.yielded.size, 0, 'no idle paint loop survives a finished queue');
});

test('new visible work does not wait behind an existing preview yield', () => {
  const f = fixture();
  f.scheduler.schedule({id:'preview'}); f.microtask();
  f.scheduler.schedule({id:'visible',visible:true}); f.microtask();
  assert.deepEqual(f.painted, [['visible']]);
  f.yieldOnce(); assert.deepEqual(f.painted.at(-1), ['preview']);
});

test('a preview promoted to visible bypasses the optional batch limit', () => {
  const f = fixture(1), first = {id:'first'}, promoted = {id:'promoted'};
  f.scheduler.schedule(first); f.scheduler.schedule(promoted); f.microtask();
  promoted.visible = true;
  f.yieldOnce(); assert.deepEqual(f.painted, [['promoted','first']]);
});

test('unmounted jobs are dropped and removing the last job cancels its yield', () => {
  const f = fixture(), stale = {id:'stale'}, removed = {id:'removed'};
  f.scheduler.schedule(stale); f.scheduler.schedule(removed); f.microtask();
  stale.current = false; f.scheduler.remove(removed); f.yieldOnce();
  assert.deepEqual(f.painted, []); assert.equal(f.yielded.size, 0);
  const cancelled = {id:'cancelled'}; f.scheduler.schedule(cancelled); f.microtask();
  assert.equal(f.yielded.size, 1); f.scheduler.remove(cancelled);
  assert.equal(f.yielded.size, 0);
});

test('fold-source flushing finishes its artwork without consuming other previews', () => {
  const f = fixture(), bottom = {id:'bottom'}, back = {id:'back'}, other = {id:'other'};
  [bottom,back,other].forEach(job => f.scheduler.schedule(job)); f.microtask();
  f.scheduler.flush([bottom,back]);
  assert.deepEqual(f.painted, [['bottom','back']]);
  f.yieldOnce(); assert.deepEqual(f.painted.at(-1), ['other']);
  f.scheduler.flush([bottom]);
  assert.deepEqual(f.painted.at(-1), ['bottom'], 'source geometry is revalidated even if no paint was pending');
});

test('invalid preview batch sizes are rejected', () => {
  for (const batch of [0,-1,1.5,NaN]) assert.throws(() => fixture(batch), RangeError);
});

test('explicit fold flush bypasses deferred layout painting', () => {
  const calls = [], deferred = [];
  const scheduler = new InkJobScheduler({ current: () => true, visible: () => true,
    paint: (jobs, immediate = false) => calls.push({ jobs, immediate }), defer: callback => deferred.push(callback) });
  const job = { id: 'source' };
  scheduler.schedule(job); deferred.shift()();
  scheduler.flush([job]);
  assert.deepEqual(calls, [{ jobs: [job], immediate: false }, { jobs: [job], immediate: true }]);
});
