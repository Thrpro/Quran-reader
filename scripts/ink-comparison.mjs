import assert from 'node:assert/strict';

// Compare silhouettes in physical screenshot pixels. A one-pixel edge halo
// accommodates different SVG/canvas antialiasing, not a whole glyph shift.
export function compareInk(actual, reference) {
  assert.equal(actual.width, reference.width, 'ink image widths differ');
  assert.equal(actual.height, reference.height, 'ink image heights differ');
  const { width, height } = actual, pixels = width * height;
  function stats(image) {
    let background = 255, count = 0, mass = 0, x = 0, y = 0;
    for (let p = 0; p < pixels; p++) background = Math.min(background, image.data[p * 4]);
    for (let p = 0; p < pixels; p++) {
      const value = image.data[p * 4] - background;
      mass += value; x += value * (p % width); y += value * Math.floor(p / width);
      count += Number(image.data[p * 4] > 100);
    }
    return { count, mass, x: x / mass, y: y / mass };
  }
  function unmatched(image, other) {
    let count = 0;
    for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
      if (image.data[(y * width + x) * 4] <= 100) continue;
      let found = false;
      for (let dy = -1; dy <= 1 && !found; dy++) for (let dx = -1; dx <= 1; dx++) {
        const xx = x + dx, yy = y + dy;
        if (xx >= 0 && yy >= 0 && xx < width && yy < height && other.data[(yy * width + xx) * 4] > 100) { found = true; break; }
      }
      if (!found) count++;
    }
    return count;
  }
  let error = 0, intersection = 0, union = 0, bright = 0;
  for (let p = 0; p < pixels; p++) {
    const a = actual.data[p * 4], b = reference.data[p * 4];
    error += Math.abs(a - b); intersection += Number(a > 100 && b > 100);
    union += Number(a > 100 || b > 100); bright += Number(a >= 225);
  }
  const a = stats(actual), b = stats(reference);
  return { width, height, meanError: error / pixels, overlap: intersection / Math.max(1, union), bright,
    actualInkPixels: a.count, referenceInkPixels: b.count,
    unmatchedActual: unmatched(actual, reference), unmatchedReference: unmatched(reference, actual),
    centroidShift: Math.hypot(a.x - b.x, a.y - b.y), massRatio: a.mass / b.mass };
}

export function assertInkMatch(comparison) {
  const details = JSON.stringify(comparison);
  assert.ok(comparison.actualInkPixels > 0 && comparison.referenceInkPixels > 0, `missing ink: ${details}`);
  assert.ok(comparison.meanError < 3, `same-outline reference mismatch: ${details}`);
  assert.equal(comparison.unmatchedActual, 0, `extra/displaced ink outside the one-pixel edge halo: ${details}`);
  assert.equal(comparison.unmatchedReference, 0, `missing/displaced ink outside the one-pixel edge halo: ${details}`);
  assert.ok(comparison.centroidShift < .5, `ink placement centroid differs: ${details}`);
  assert.ok(comparison.massRatio > .95 && comparison.massRatio < 1.05, `ink coverage/contrast differs: ${details}`);
}
