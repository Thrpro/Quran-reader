import assert from 'node:assert/strict';
import test from 'node:test';
import { compareInk, assertInkMatch } from '../scripts/ink-comparison.mjs';

// Geometric test marks only; no Quran text is generated or modified.
function fixture({ shift = 0, missing = false, extra = false, faded = false, edge = false } = {}) {
  const width = 80, height = 40, data = new Uint8Array(width * height * 4);
  for (let p = 0; p < width * height; p++) data.set([32, 32, 32, 255], p * 4);
  function rect(x, y, w, h, value = faded ? 220 : 240) {
    for (let yy = y; yy < y + h; yy++) for (let xx = x; xx < x + w; xx++) data.set([value, value, value, 255], (yy * width + xx + shift) * 4);
  }
  rect(10, 16, 35, 3); rect(15, 10, 3, 16); rect(35, 12, 3, 12);
  if (!missing) rect(57, 7, 3, 3);
  if (extra) rect(10, 32, 40, 1);
  if (edge) rect(10, 15, 35, 1, 105);
  else rect(10, 15, 35, 1, 95);
  return { width, height, data };
}
test('one-pixel antialias changes preserve placement and coverage', () => {
  const comparison = compareInk(fixture({edge:true}), fixture());
  assert.ok(comparison.overlap < 1);
  assertInkMatch(comparison);
});
test('real two-pixel displacement still fails the artwork check', () => {
  assert.throws(() => assertInkMatch(compareInk(fixture({shift:2}), fixture())));
});
test('a whole-image one-pixel shift is not mistaken for antialias edge changes', () => {
  const comparison = compareInk(fixture({shift:1}), fixture());
  assert.equal(comparison.unmatchedActual, 0);
  assert.equal(comparison.unmatchedReference, 0);
  assert.ok(comparison.centroidShift > .5);
  assert.throws(() => assertInkMatch(comparison));
});
test('a missing isolated mark fails even when whole-image error is small', () => {
  const comparison = compareInk(fixture({missing:true}), fixture());
  assert.ok(comparison.meanError < 3);
  assert.throws(() => assertInkMatch(comparison), /missing\/displaced ink/);
});
test('an unwanted thin line fails the artwork check', () => {
  assert.throws(() => assertInkMatch(compareInk(fixture({extra:true}), fixture())), /extra\/displaced ink/);
});
test('reduced brightness cannot pass just because the binary silhouette matches', () => {
  const comparison = compareInk(fixture({faded:true}), fixture());
  assert.ok(comparison.meanError < 3);
  assert.throws(() => assertInkMatch(comparison), /ink coverage\/contrast differs/);
});
test('empty or differently sized images cannot pass', () => {
  const empty = fixture(); empty.data.fill(32);
  assert.throws(() => assertInkMatch(compareInk(empty, empty)), /missing ink/);
  assert.throws(() => compareInk(fixture(), {...fixture(),width:81}), /widths differ/);
});
