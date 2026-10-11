import test from "node:test";
import assert from "node:assert/strict";
import { normalizeTafsirChapter, readDownloadedTafsir, removeTafsir, fetchPage } from "../src/tafsirDownloads.ts";
test("tafsir imports preserve text and reject wrong resources, duplicates and missing ayahs", () => {
  const rows = [1, 2].map(n => ({ resource_id: 16, verse_key: "1:" + n, text: "<p>تفسير</p>" }));
  const result = normalizeTafsirChapter(rows, 16, 1, 2);
  assert.equal(result.groups["1"].text, rows[0].text);
  assert.deepEqual(result.groups["1"].ayahKeys, [1, 2]);
  for (const invalid of [rows.slice(0, 1), [rows[0], rows[0]], [{ ...rows[0], resource_id: 14 }, rows[1]], [{ ...rows[0], verse_key: "2:1" }, rows[1]]]) {
    assert.throws(() => normalizeTafsirChapter(invalid, 16, 1, 2));
  }
});

test("downloaded tafsir chapters reuse bounded parsed data and invalidate on removal", async () => {
  const previousCaches = globalThis.caches, previousLocation = globalThis.location;
  let reads = 0, installed = true;
  globalThis.location = { origin: "https://reader.test" };
  globalThis.caches = { async open() { return { async match(url) {
    if (url.endsWith("complete")) return installed ? new Response("{}") : undefined;
    reads++; return new Response(JSON.stringify({ lookup: { 1: "1" }, groups: {} }));
  } }; }, async delete() { installed = false; } };
  try {
    const first = await readDownloadedTafsir("quran-94", 1);
    assert.equal(await readDownloadedTafsir("quran-94", 1), first);
    assert.equal(reads, 1);
    await removeTafsir("quran-94");
    await assert.rejects(readDownloadedTafsir("quran-94", 1), /not installed/);
    installed = true;
    await readDownloadedTafsir("quran-94", 1);
    assert.equal(reads, 2, "failed loads can retry");
  } finally { globalThis.caches = previousCaches; globalThis.location = previousLocation; }
});

test("tafsir service retries temporary failures, but not authentication errors", async context => {
  let calls = 0;
  context.mock.method(globalThis, 'fetch', async () => { calls++; return calls === 1 ? new Response('busy', {status:429,headers:{'retry-after':'0'}}) : new Response('{"tafsirs":[]}'); });
  assert.deepEqual(await fetchPage('https://api.quran.com/test', new AbortController().signal), {tafsirs:[]});
  assert.equal(calls, 2);
  calls = 0;
  context.mock.method(globalThis, 'fetch', async () => { calls++; return new Response('unauthorized', {status:401}); });
  await assert.rejects(fetchPage('https://api.quran.com/test', new AbortController().signal), /401/);
  assert.equal(calls, 1);
});

test("compressed offline books preserve tafsir text and legacy books remain compatible", async context => {
  const originalCaches = globalThis.caches, originalLocation = globalThis.location;
  context.after(() => { globalThis.caches = originalCaches; globalThis.location = originalLocation; });
  const chapter = {lookup:{1:"1"},groups:{1:{fromAyah:1,toAyah:1,ayahKeys:[1],text:"<p>تفسير الآية</p>"}}};
  const raw = JSON.stringify(chapter);
  const packed = await new Response(new Blob([raw]).stream().pipeThrough(new CompressionStream('gzip'))).arrayBuffer();
  globalThis.location = {origin:'https://reader.test'};
  globalThis.caches = {open:async()=>({match:async url=>url.endsWith('complete')?new Response('{}'):new Response(packed,{headers:{'x-quran-pack-encoding':'gzip'}})})};
  assert.deepEqual(await readDownloadedTafsir('quran-14',112),chapter);
});
