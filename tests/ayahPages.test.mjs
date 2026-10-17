import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import test from "node:test";
import { ayahFirstPage, ayahPageSpan } from "../src/ayahPages.ts";
import { orderedAyahKeys } from "../src/ayahKeyboard.ts";
const read = path => JSON.parse(readFileSync(new URL(path, import.meta.url), "utf8"));

test("compact ayah lookup matches all 6236 authentic search references", () => {
  const entries = read("../public-qul/experimental/digital-khatt/ayah-search-index.json");
  for (const entry of entries) assert.equal(ayahFirstPage(entry.surah, entry.ayah), entry.pageNumber, entry.key);
  for (const [surah, ayah] of [[0, 1], [115, 1], [2, 287], [1, 0], [1.5, 1], [1, NaN], [NaN, 1]]) {
    assert.equal(ayahFirstPage(surah, ayah), null);
    assert.equal(ayahPageSpan(surah, ayah), null);
  }
});

test("generated lookup is reproducible and verified from all604 source pages", () => {
  execFileSync(process.execPath, ["scripts/build-ayah-page-map.mjs", "--check"], { cwd: new URL("../", import.meta.url) });
});

test("study quotes reconstructed across mapped source pages exactly match search text", () => {
  const entries = read("../public-qul/experimental/digital-khatt/ayah-search-index.json");
  const pages = read("../public-qul/experimental/digital-khatt/pages.json");
  const wordsByReference = new Map();
  for (const page of pages) for (const line of page.lines) for (const word of line.words) {
    const key = `${word.surah}:${word.ayah}`, span = ayahPageSpan(word.surah, word.ayah);
    assert.ok(page.pageNumber >= span.first && page.pageNumber <= span.last, `${key} page fragment is retained`);
    if (/^\u06DD[\d٠-٩]+$/u.test(word.text.trim())) continue;
    const words = wordsByReference.get(key) ?? [];
    words.push(word.text); wordsByReference.set(key, words);
  }
  for (const entry of entries) assert.equal(wordsByReference.get(entry.key).join(" "), entry.text, entry.key);
});

test("keyboard ayah order crosses surahs and deduplicates line fragments", () => {
  assert.deepEqual(orderedAyahKeys(["3:1", "2:286", "2:285", "2:286", "", "invalid"]), ["2:285", "2:286", "3:1"]);
  assert.deepEqual(orderedAyahKeys(["114:6", "1:7", "1:2", "1:1", "1:2"]), ["1:1", "1:2", "1:7", "114:6"]);
});
