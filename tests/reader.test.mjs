import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { AsyncPageCache } from "../src/asyncPageCache.ts";
import { createAyahSearcher, isLooseArabicTokenMatch, isLoosePhraseMatch } from "../src/ayahSearch.ts";
import { getDragPageTurn } from "../src/pageGesture.ts";
import { enqueuePageTurn, releasePageTurnSource } from "../src/pageTurnQueue.ts";
import { hasSavedLanguage, LANGUAGE_SETUP_VERSION } from "../src/languagePreferences.ts";

const readJson = (path) => JSON.parse(readFileSync(new URL(path, import.meta.url), "utf8"));
const entries = readJson("../public-qul/experimental/digital-khatt/ayah-search-index.json");

test("horizontal page drags follow right-to-left mushaf order", () => {
  assert.equal(getDragPageTurn(150, 8, 500, 617), "next");
  assert.equal(getDragPageTurn(-150, 8, 500, 617), "previous");
});

test("short movements and vertical scrolling do not flip pages", () => {
  for (const [x, y, time] of [[12, 0, 10], [30, 0, 500], [90, 150, 100], [80, 80, 100]]) {
    assert.equal(getDragPageTurn(x, y, time, 617), null);
  }
});

test("deliberate flicks and small-screen drags are recognized", () => {
  assert.equal(getDragPageTurn(35, 2, 50, 617), "next");
  assert.equal(getDragPageTurn(-50, 2, 500, 320), "previous");
});

test("releasing an arrow key drops every queued keyboard turn", () => {
  const queue = [];
  enqueuePageTurn(queue, { direction: "next", repeated: false, source: "keyboard" }, 10);
  enqueuePageTurn(queue, { direction: "next", repeated: true, source: "keyboard" }, 10);
  enqueuePageTurn(queue, { direction: "next", repeated: true, source: "keyboard" }, 10);
  enqueuePageTurn(queue, { direction: "next", repeated: false, source: "discrete" }, 10);

  assert.deepEqual(releasePageTurnSource(queue, "keyboard"), [
    { direction: "next", repeated: false, source: "discrete" },
  ]);
});

test("language setup is completed only by the current onboarding flow", () => {
  assert.equal(hasSavedLanguage({}), false);
  assert.equal(hasSavedLanguage({ metadataLanguage: "unknown" }), false);
  for (const metadataLanguage of ["arabic", "english"]) {
    assert.equal(hasSavedLanguage({ metadataLanguage }), false);
    assert.equal(hasSavedLanguage({ metadataLanguage, hasChosenLanguage: false }), false);
    assert.equal(hasSavedLanguage({ metadataLanguage, hasChosenLanguage: true }), false);
    assert.equal(hasSavedLanguage({
      metadataLanguage,
      hasChosenLanguage: true,
      languageSetupVersion: LANGUAGE_SETUP_VERSION,
    }), true);
  }
});

test("concurrent page requests share one load", async () => {
  let calls = 0;
  const cache = new AsyncPageCache(2, async (page) => { calls++; return { page }; });
  const first = cache.load(1);
  assert.equal(first, cache.load(1));
  assert.deepEqual(await first, { page: 1 });
  assert.equal(await cache.load(1), await first);
  assert.equal(calls, 1);
});

test("failed loads are removed and can be retried", async () => {
  let calls = 0;
  const cache = new AsyncPageCache(2, async () => {
    if (++calls === 1) throw new Error("temporary read failure");
    return "recovered";
  });
  await assert.rejects(cache.load(1), /temporary read failure/);
  assert.equal(cache.size, 0);
  assert.equal(await cache.load(1), "recovered");
  assert.equal(calls, 2);
});

test("synchronous loader failures can also be retried", async () => {
  const cache = new AsyncPageCache(1, () => { throw new Error("failed"); });
  await assert.rejects(cache.load(1), /failed/);
  await assert.rejects(cache.load(1), /failed/);
});

test("cache evicts the least recently read page", async () => {
  const cache = new AsyncPageCache(2, async (page) => page);
  await cache.load(1);
  await cache.load(2);
  await cache.load(1);
  await cache.load(3);
  assert.equal(cache.peek(1), 1);
  assert.equal(cache.peek(2), undefined);
  assert.equal(cache.peek(3), 3);
});

test("reading all 604 pages keeps at most 24 cached pages", async () => {
  const cache = new AsyncPageCache(24, async (page) => ({ page }));
  for (let page = 1; page <= 604; page++) {
    await cache.load(page);
    assert.ok(cache.size <= 24);
  }
  assert.equal(cache.peek(1), undefined);
  assert.equal(cache.peek(604).page, 604);
});

test("out-of-order completions keep page identities and the cache bound", async () => {
  const resolve = new Map();
  const cache = new AsyncPageCache(2, (page) => new Promise((done) => resolve.set(page, done)));
  const loads = [1, 2, 3].map((page) => cache.load(page));
  await Promise.resolve();
  for (const page of [3, 1, 2]) resolve.get(page)({ page });
  assert.deepEqual(await Promise.all(loads), [{ page: 1 }, { page: 2 }, { page: 3 }]);
  assert.equal(cache.size, 2);
});

test("invalid cache capacities are rejected", () => {
  for (const size of [0, -1, 1.5, Infinity, NaN]) {
    assert.throws(() => new AsyncPageCache(size, async (p) => p), RangeError);
  }
});

export function exhaustiveSearch(entries, queries) {
  const variants = (entry) => [entry.normalizedText, entry.expandedText ?? entry.normalizedText];
  const exact = entries.filter((entry) => variants(entry).some((text) => queries.some((q) => text.includes(q))));
  const keys = new Set(exact.map((entry) => entry.key));
  const tolerant = exact.length >= 16 ? [] : entries.filter((entry) => !keys.has(entry.key) &&
    variants(entry).some((text) => queries.some((q) =>
      isLoosePhraseMatch(text.split(" ").filter(Boolean), q.split(" ").filter(Boolean)))));
  return [...exact, ...tolerant].slice(0, 16);
}

test("optimized search preserves exact-first results on the real Quran index", () => {
  const find = createAyahSearcher(entries);
  const queries = ["الله", "بسم الله", "الرحمن", "الصلوه", "الصلاه", "موسى", "xyzzy"];
  for (let i = 0; i < entries.length; i += 71) {
    queries.push(entries[i].normalizedText.split(" ").slice(0, 4).join(" "));
  }
  for (const query of queries) {
    assert.deepEqual(find([query], null).map((e) => e.key),
      exhaustiveSearch(entries, [query]).map((e) => e.key), query);
  }
});

test("reference lookup, variants, limits, and empty queries", () => {
  const find = createAyahSearcher(entries);
  assert.equal(find([], { surah: 2, ayah: 255 })[0].key, "2:255");
  assert.deepEqual(find([], { surah: 115, ayah: 1 }), []);
  assert.deepEqual(find(["", ""], null), []);
  assert.deepEqual(find(["الله"], null, 0), []);
  assert.equal(find(["الله", "الله"], null, 3).length, 3);
  assert.deepEqual(find(["الصلوه", "الصلاه"], null).map((e) => e.key),
    exhaustiveSearch(entries, ["الصلوه", "الصلاه"]).map((e) => e.key));
});

test("tolerant search only permits the existing Arabic letter substitutions", () => {
  assert.equal(isLooseArabicTokenMatch("كتاب", "كتب"), false);
  assert.equal(isLooseArabicTokenMatch("كتابا", "كتاب"), true);
  assert.equal(isLooseArabicTokenMatch("كتابب", "كتاب"), false);
  assert.equal(isLooseArabicTokenMatch("الصلوه", "الصلاه"), true);
});

test("all 604 runtime pages exactly match the aggregate and word text", () => {
  const pages = readJson("../public-qul/experimental/digital-khatt/pages.json");
  assert.equal(pages.length, 604);
  let headers = 0;
  let basmallahs = 0;
  for (let i = 0; i < pages.length; i++) {
    const page = readJson(`../public-qul/experimental/digital-khatt/pages/p${i + 1}.json`);
    assert.equal(page.pageNumber, i + 1);
    assert.deepEqual(page, pages[i], `page ${i + 1}`);
    for (let lineIndex = 0; lineIndex < page.lines.length; lineIndex++) {
      const line = page.lines[lineIndex];
      if (line.type === "ayah") {
        assert.equal(line.text, line.words.map((word) => word.text).join(" "),
          `page ${i + 1}, line ${line.lineNumber}`);
        for (const word of line.words) {
          assert.ok(Number.isInteger(word.surah) && word.surah >= 1 && word.surah <= 114,
            `page ${i + 1}, word ${word.id} has a valid surah`);
          assert.ok(Number.isInteger(word.ayah) && word.ayah >= 1,
            `page ${i + 1}, word ${word.id} has a valid ayah`);
        }
      }
      if (line.type === "surah_name") headers++;
      if (line.type === "basmallah") {
        basmallahs++;
        const previous = page.lines[lineIndex - 1] ?? pages[i - 1]?.lines.at(-1);
        assert.equal(previous?.type, "surah_name", `basmallah follows header on page ${i + 1}`);
        if (lineIndex > 0) assert.ok(previous.lineNumber < line.lineNumber);
        assert.notEqual(previous.surahNumber, 9);
      }
    }
  }
  assert.equal(headers, 114);
  assert.equal(basmallahs, 112);
});
