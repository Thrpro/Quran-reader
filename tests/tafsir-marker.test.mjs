import assert from 'node:assert/strict';
import test from 'node:test';
import { parseTafsirMarker } from '../src/tafsirPassage.ts';

test('tafsir source labels support individual ayahs, ranges and explicit lists', () => {
  assert.deepEqual(parseTafsirMarker('{21}'), [21]);
  assert.deepEqual(parseTafsirMarker('{24 ـ 25}'), [24, 25]);
  assert.deepEqual(parseTafsirMarker('{١٦٥ ـ ١٦٦ ـ ١٦٧}'), [165, 166, 167]);
  assert.deepEqual(parseTafsirMarker('{۲۱}'), [21]);
  assert.deepEqual(parseTafsirMarker('{1–5}'), [1, 2, 3, 4, 5]);
});

test('quotations and invalid markers cannot become ayah boundaries', () => {
  for (const marker of ['{quoted words}', '{0}', '{287}', '{5-1}', '{21} more', '21', '{1,2}', '{1.2}']) {
    assert.equal(parseTafsirMarker(marker), null);
  }
});
