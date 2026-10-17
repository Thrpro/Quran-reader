import assert from "node:assert/strict";
import test from "node:test";
import { acquireInkCanvas, releaseInkCanvas } from "../src/inkCanvasPool.ts";

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
  assert.deepEqual([canvases[60].width, canvases[60].height], [0, 0]);
  advance(1500);
  assert.ok(canvases.every(canvas => canvas.width === 0 && canvas.height === 0));
});

test("invalid sizes fail before allocating a canvas", context => {
  const { created } = environment(context);
  for (const [width, height] of [[0, 1], [1, 0], [-1, 2], [NaN, 1], [1, Infinity]]) {
    assert.throws(() => acquireInkCanvas(width, height), RangeError);
  }
  assert.equal(created.length, 0);
});
