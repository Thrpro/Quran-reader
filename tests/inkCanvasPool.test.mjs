import assert from "node:assert/strict";
import test from "node:test";
import { acquireInkCanvas, releaseInkCanvas, MAX_IDLE_INK_BYTES } from "../src/inkCanvasPool.ts";

function environment(context) {
  const previous = { document: globalThis.document, setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout };
  let now = 0, nextTimer = 0;
  const timers = new Map(), created = [];
  globalThis.setTimeout = (callback, delay) => { const id = ++nextTimer; timers.set(id, { callback, at: now + delay }); return id; };
  globalThis.clearTimeout = id => timers.delete(id);
  const advance = duration => {
    now += duration;
    for (const [id, timer] of [...timers]) if (timer.at <= now) { timers.delete(id); timer.callback(); }
  };
  globalThis.document = { createElement(tag) {
    assert.equal(tag, "canvas");
    const calls = [];
    let width = 300, height = 150;
    const drawing = { setTransform: (...args) => calls.push(["setTransform", ...args]), clearRect: (...args) => calls.push(["clearRect", ...args]) };
    const canvas = {
      get width() { return width; }, set width(value) { width = value; calls.push(["width", value]); },
      get height() { return height; }, set height(value) { height = value; calls.push(["height", value]); },
      calls, className: "", styleText: "", removed: 0,
      removeAttribute(name) { assert.equal(name, "style"); this.styleText = ""; },
      remove() { this.removed++; },
      getContext: (...args) => { calls.push(["getContext", ...args]); return drawing; },
    };
    created.push(canvas); return canvas;
  } };
  context.after(() => {
    advance(1500);
    if (previous.document === undefined) delete globalThis.document;
    else globalThis.document = previous.document;
    globalThis.setTimeout = previous.setTimeout;
    globalThis.clearTimeout = previous.clearTimeout;
  });
  return { created, advance };
}

test("pool uses exact pixel sizes and reuses matching CPU drawing stores", context => {
  const { created } = environment(context);
  const first = acquireInkCanvas(300.5, 70);
  assert.deepEqual([first.width, first.height], [301, 70]);
  assert.ok(first.calls.some(call => call[0] === "getContext" && call[2].willReadFrequently === true));
  first.className = "old-fold"; first.styleText = "left:200px";
  const dimensionAssignments = first.calls.filter(call => call[0] === "width" || call[0] === "height").length;
  releaseInkCanvas(first);
  const second = acquireInkCanvas(300.5, 70);
  assert.equal(second, first);
  assert.equal(created.length, 1);
  assert.equal(second.className, ""); assert.equal(second.styleText, "");
  assert.equal(second.calls.filter(call => call[0] === "width" || call[0] === "height").length, dimensionAssignments,
    "same-size reuse does not reset the native allocation");
  assert.deepEqual(second.calls.at(-1), ["clearRect", 0, 0, 301, 70]);
  releaseInkCanvas(second);
});

test("a different size reuses an idle context and resets its exact backing size", context => {
  const { created } = environment(context);
  const first = acquireInkCanvas(64, 64); releaseInkCanvas(first);
  const different = acquireInkCanvas(65, 64);
  assert.equal(different, first);
  assert.deepEqual([different.width, different.height], [65, 64]);
  assert.equal(created.length, 1);
  releaseInkCanvas(different);
});

test("idle expiry releases backing stores while leaving active canvases untouched", context => {
  const { advance } = environment(context);
  const first = acquireInkCanvas(64, 64);
  const second = acquireInkCanvas(128, 64);
  releaseInkCanvas(first); advance(100); releaseInkCanvas(second);
  advance(1400);
  const active = acquireInkCanvas(64, 64);
  assert.equal(active, first);
  advance(100);
  assert.deepEqual([second.width, second.height], [0, 0]);
  assert.deepEqual([active.width, active.height], [64, 64]);
  releaseInkCanvas(active); advance(1499);
  assert.equal(active.width, 64);
  advance(1); assert.deepEqual([active.width, active.height], [0, 0]);
});

test("double release cannot put the same canvas in the free pool twice", context => {
  environment(context);
  const first = acquireInkCanvas(64, 64);
  releaseInkCanvas(first); releaseInkCanvas(first);
  assert.equal(first.removed, 1);
  const reacquired = acquireInkCanvas(64, 64);
  const other = acquireInkCanvas(64, 64);
  assert.equal(reacquired, first);
  assert.notEqual(other, first);
  releaseInkCanvas(reacquired); releaseInkCanvas(other);
});

test("at most sixty idle backing stores survive a release burst", context => {
  const { advance } = environment(context);
  const canvases = Array.from({ length: 61 }, () => acquireInkCanvas(128, 192));
  canvases.forEach(releaseInkCanvas);
  assert.equal(canvases.filter(canvas => canvas.width > 0).length, 60);
  assert.deepEqual([canvases[0].width, canvases[0].height], [0, 0], "oldest idle canvas is evicted first");
  assert.deepEqual([canvases[60].width, canvases[60].height], [128, 192], "recent layout dimensions remain reusable");
  advance(1500);
  assert.ok(canvases.every(canvas => canvas.width === 0 && canvas.height === 0));
});

test("byte pressure evicts oldest released stores but never active artwork", context => {
  const { created } = environment(context);
  const active = acquireInkCanvas(4096, 4096);
  const first = acquireInkCanvas(2048, 2048);
  const second = acquireInkCanvas(2048, 2048);
  const latest = acquireInkCanvas(2048, 2048);
  releaseInkCanvas(first); releaseInkCanvas(second);
  assert.equal(first.width * first.height * 4 + second.width * second.height * 4, MAX_IDLE_INK_BYTES);
  releaseInkCanvas(latest);
  assert.deepEqual([first.width, first.height], [0, 0]);
  assert.deepEqual([second.width, second.height], [2048, 2048]);
  assert.deepEqual([latest.width, latest.height], [2048, 2048]);
  assert.deepEqual([active.width, active.height], [4096, 4096], "active stores are not part of the idle budget");
  assert.equal(acquireInkCanvas(2048, 2048), latest, "recent exact-size stores are reused without allocation");
  releaseInkCanvas(latest); releaseInkCanvas(active);
  assert.equal(created.filter(canvas => canvas.width > 0).reduce((bytes, canvas) => bytes + canvas.width * canvas.height * 4, 0), MAX_IDLE_INK_BYTES);
});

test("an oversized release is discarded without evicting useful small idle stores", context => {
  environment(context);
  const small = acquireInkCanvas(100, 100);
  const oversized = acquireInkCanvas(4096, 4096);
  releaseInkCanvas(small); releaseInkCanvas(oversized); releaseInkCanvas(oversized);
  assert.deepEqual([oversized.width, oversized.height], [0, 0]);
  assert.equal(oversized.removed, 1, "discarded leases still reject duplicate release");
  const reused = acquireInkCanvas(100, 100);
  assert.equal(reused, small);
  releaseInkCanvas(reused);
});

test("resized reuse debits the previous store before accounting for its new dimensions", context => {
  const { advance } = environment(context);
  const first = acquireInkCanvas(2048, 2048);
  const second = acquireInkCanvas(2048, 2048);
  releaseInkCanvas(first); releaseInkCanvas(second);
  const resized = acquireInkCanvas(1024, 1024);
  assert.equal(resized, first, "a nonmatching request reuses the oldest idle context");
  releaseInkCanvas(resized);
  assert.equal(second.width, 2048, "shrinking a reused store does not spuriously evict another one");
  const current = acquireInkCanvas(2048, 2048);
  assert.equal(current, second, "exact-size lookup survives byte-accounting changes");
  advance(1500);
  assert.deepEqual([resized.width, resized.height], [0, 0]);
  assert.deepEqual([current.width, current.height], [2048, 2048]);
  releaseInkCanvas(current);
  const companion = acquireInkCanvas(2048, 2048);
  const other = acquireInkCanvas(2048, 2048);
  releaseInkCanvas(companion); releaseInkCanvas(other);
  assert.deepEqual([companion.width, companion.height], [2048, 2048], "expiry clears byte accounting before later releases");
  assert.deepEqual([other.width, other.height], [2048, 2048]);
});

test("a full-budget store remains reusable, then expiry frees every byte", context => {
  const { advance } = environment(context);
  const canvas = acquireInkCanvas(4096, 2048);
  releaseInkCanvas(canvas);
  assert.equal(canvas.width * canvas.height * 4, MAX_IDLE_INK_BYTES);
  const leased = acquireInkCanvas(4096, 2048);
  assert.equal(leased, canvas);
  advance(2000);
  assert.equal(leased.width, 4096, "a stale expiry cannot clear an acquired store");
  releaseInkCanvas(leased); advance(1500);
  assert.deepEqual([leased.width, leased.height], [0, 0]);
});

test("explicit discard clears only the returned lease without retaining its pixels", context => {
  const { advance } = environment(context);
  const visible=acquireInkCanvas(100,100),preview=acquireInkCanvas(100,100);
  releaseInkCanvas(preview,false);
  assert.deepEqual([preview.width,preview.height],[0,0]);
  const next=acquireInkCanvas(100,100);
  assert.notEqual(next,preview,'a discarded preview is not a pooled backing store');
  advance(2000);
  assert.deepEqual([visible.width,visible.height],[100,100],'discard never resets the visible lease');
  releaseInkCanvas(visible);releaseInkCanvas(next);
});

test("invalid sizes fail before allocating a canvas", context => {
  const { created } = environment(context);
  for (const [width, height] of [[0, 1], [1, 0], [-1, 2], [NaN, 1], [1, Infinity]]) {
    assert.throws(() => acquireInkCanvas(width, height), RangeError);
  }
  assert.equal(created.length, 0);
});
