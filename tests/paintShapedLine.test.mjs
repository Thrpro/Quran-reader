import assert from "node:assert/strict";
import test from "node:test";
import { paintShapedLine } from "../src/paintShapedLine.ts";

function mockCanvas(context, { width = 1200, height = 320, pixels = [] } = {}) {
  const originalPath = globalThis.Path2D;
  const parsed = [];
  globalThis.Path2D = class {
    constructor(path) { this.path = path; parsed.push(this); }
  };
  context.after(() => {
    if (originalPath === undefined) delete globalThis.Path2D;
    else globalThis.Path2D = originalPath;
  });
  const calls = [];
  const image = { data: new Uint8ClampedArray(pixels) };
  const drawing = {
    setTransform: (...args) => calls.push(["setTransform", ...args]),
    clearRect: (...args) => calls.push(["clearRect", ...args]),
    save: () => calls.push(["save"]),
    restore: () => calls.push(["restore"]),
    translate: (...args) => calls.push(["translate", ...args]),
    scale: (...args) => calls.push(["scale", ...args]),
    fill: path => calls.push(["fill", path, drawing.fillStyle]),
    getImageData: (...args) => { calls.push(["getImageData", ...args]); return image; },
    putImageData: (...args) => {
      // A browser copies these bytes before the temporary buffer is released.
      image.writtenPixels = [...args[0].data];
      calls.push(["putImageData", ...args]);
    },
  };
  const canvas = { width, height, getContext: (...args) => { calls.push(["getContext", ...args]); return drawing; } };
  return { canvas, drawing, calls, parsed, image };
}

const shape = (glyphs, overrides = {}) => ({ width: 10000, shift: -42, scale: 1.25,
  baselineOffset: 4, glyphs, wordBounds: {}, ...overrides });

test("canvas painting preserves the SVG viewBox and supplied glyph placements", context => {
  const { canvas, calls, parsed } = mockCanvas(context);
  const outlines = { 5: "M0 0H20V30Z", 7: "M5 3L8 9Z" };
  const line = shape([[5, -900, 37, 0], [7, -650, -12, 0]]);
  const before = JSON.stringify(line);
  paintShapedLine(canvas, line, outlines, "rgb(251, 251, 251)", false);
  assert.deepEqual(calls.slice(0, 7), [
    ["getContext", "2d", { willReadFrequently: true }],
    ["setTransform", 1, 0, 0, 1, 0, 0],
    ["clearRect", 0, 0, 1200, 320],
    ["save"],
    ["setTransform", 0.1, 0, 0, 0.1, 1100, 195],
    ["translate", -42, 0],
    ["scale", 1.25, -1.25],
  ]);
  assert.deepEqual(calls.filter(call => call[0] === "translate"),
    [["translate", -42, 0], ["translate", -900, 37], ["translate", -650, -12]]);
  assert.deepEqual(parsed.map(path => path.path), Object.values(outlines));
  assert.deepEqual(calls.filter(call => call[0] === "fill").map(call => call.slice(1)),
    [[parsed[0], "rgb(251, 251, 251)"], [parsed[1], "rgb(251, 251, 251)"]]);
  assert.equal(JSON.stringify(line), before, "source geometry stays immutable");
  assert.equal(calls.filter(call => call[0] === "getImageData").length, 0,
    "normal ink does not copy or read the canvas pixels");
});

test("meet alignment also centers tall canvases vertically", context => {
  const { canvas, calls } = mockCanvas(context, { width: 1000, height: 640 });
  paintShapedLine(canvas, shape([]), {}, "black", false);
  assert.deepEqual(calls.filter(call => call[0] === "setTransform")[1],
    ["setTransform", 0.1, 0, 0, 0.1, 1000, 355]);
});

test("parsed outlines are reused and bounded by an LRU of 256 paths", context => {
  const { canvas, parsed } = mockCanvas(context);
  const outlines = Object.fromEntries(Array.from({ length: 257 }, (_, id) => [id, `M${id} 0H1Z`]));
  const draw = id => paintShapedLine(canvas, shape([[id, 0, 0, 0]]), outlines, "white", false);
  for (let id = 0; id < 256; id++) draw(id);
  assert.equal(parsed.length, 256);
  draw(0); // Keep 0 recent; 1 becomes the oldest path.
  assert.equal(parsed.length, 256, "repeated visible glyphs share parsed paths");
  draw(256);
  draw(0);
  assert.equal(parsed.length, 257, "a recently used glyph survives eviction");
  draw(1);
  assert.equal(parsed.length, 258, "the least recently used outline was released");
});

test("different outline collections cannot share an ID's parsed geometry", context => {
  const { canvas, parsed } = mockCanvas(context);
  const line = shape([[4, 0, 0, 0]]);
  paintShapedLine(canvas, line, { 4: "M0 0H10Z" }, "black", false);
  paintShapedLine(canvas, line, { 4: "M0 0H20Z" }, "black", false);
  assert.deepEqual(parsed.map(path => path.path), ["M0 0H10Z", "M0 0H20Z"]);
});

test("dark contrast changes alpha coverage once without painting transparent contours", context => {
  const alphas = [0, 1, 16, 64, 128, 254, 255];
  const { canvas, calls, image } = mockCanvas(context, {
    pixels: alphas.flatMap(alpha => [17, 83, 251, alpha]),
  });
  paintShapedLine(canvas, shape([]), {}, "white", true);
  assert.deepEqual(image.writtenPixels, alphas.flatMap(alpha =>
    [17, 83, 251, Math.round(255 * Math.pow(alpha / 255, 0.85))]));
  assert.equal(image.writtenPixels[3], 0);
  assert.equal(image.writtenPixels.at(-1), 255);
  if (typeof ArrayBuffer.prototype.transfer === "function") {
    assert.equal(image.data.buffer.byteLength, 0, "readback storage is released after copying");
  }
  assert.equal(calls.filter(call => call[0] === "getImageData").length, 1);
  assert.equal(calls.filter(call => call[0] === "putImageData").length, 1);
});

test("a zero-size canvas is cleared without parsing paths or reading pixels", context => {
  const { canvas, calls, parsed } = mockCanvas(context, { width: 0 });
  paintShapedLine(canvas, shape([[1, 0, 0, 0]]), { 1: "M0 0H10Z" }, "white", true);
  assert.equal(parsed.length, 0);
  assert.equal(calls.some(call => call[0] === "getImageData"), false);
});

test("a missing 2D context is handled without changing source data", () => {
  assert.doesNotThrow(() => paintShapedLine({ getContext: () => null }, shape([]), {}, "black", false));
});

test("fractional viewport alignment preserves the original glyph coordinates", context => {
  const { canvas, calls } = mockCanvas(context, { width: 1002, height: 322 });
  const viewport = { width: 1000.5, height: 320.25, offsetX: 0.25, offsetY: 0.5 };
  paintShapedLine(canvas, shape([[7, -900, 37, 0]]), { 7: "M0 0H10Z" }, "white", false, viewport);
  const transform = calls.filter(call => call[0] === "setTransform")[1];
  assert.ok(Math.abs(transform[1] - 0.10005) < 1e-12);
  assert.ok(Math.abs(transform[4] - 0.10005) < 1e-12);
  assert.ok(Math.abs(transform[5] - 1000.75) < 1e-12);
  assert.ok(Math.abs(transform[6] - 195.6425) < 1e-12);
  assert.deepEqual(calls.find(call => call[0] === "clearRect"), ["clearRect", 0, 0, 1002, 322],
    "the whole expanded backing store is cleared");
  assert.deepEqual(calls.filter(call => call[0] === "translate"),
    [["translate", -42, 0], ["translate", -900, 37]], "source placement is not pixel-snapped");
});
