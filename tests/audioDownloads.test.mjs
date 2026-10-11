import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { downloadAudio } from "../src/audioDownloads.ts";

test("audio downloads report progress, commit completion, and reject invalid streams", async () => {
  const originalFetch = globalThis.fetch;
  const progress = [];
  let saved = false;
  const cache = { async put(url, response) { await response.arrayBuffer(); saved = true; } };
  const options = { usedBytes: 0, maxBytes: 1000000, signal: new AbortController().signal,
    onProgress: value => { if (value.percent === 100) assert.ok(saved); progress.push(value); } };
  try {
    globalThis.fetch = async () => new Response(new ReadableStream({
      start(controller) { const clip = readFileSync(new URL("./fixtures/yasser-112001.mp3", import.meta.url)); controller.enqueue(clip.subarray(0, 10)); controller.enqueue(clip.subarray(10)); controller.close(); },
    }), { headers: { "content-length": "38912" } });
    await downloadAudio("https://example.test/audio", cache, options);
    assert.deepEqual(progress.map(p => p.percent), [0, 0, 99, 100]);
    saved = false;
    await assert.rejects(downloadAudio("x", cache, { ...options, maxBytes: 9 }), { reason: "storage" });
    assert.equal(saved, false);
    globalThis.fetch = async () => { const clip = readFileSync(new URL("./fixtures/yasser-112001.mp3", import.meta.url)); return new Response(clip.subarray(0, clip.length - 20), { headers: { "content-length": String(clip.length) } }); };
    await assert.rejects(downloadAudio("x", cache, options), { reason: "incomplete" });
    globalThis.fetch = async () => new Response("unknown");
    await assert.rejects(downloadAudio("x", cache, options), { reason: "size" });
    globalThis.fetch = async () => new Response("", { status: 503 });
    await assert.rejects(downloadAudio("x", cache, options), { reason: "network" });
  } finally { globalThis.fetch = originalFetch; }
});

test("downloads reject error documents, even with an audio MIME type", async () => {
  const originalFetch = globalThis.fetch;
  let commits = 0;
  const cache = { async put(_url, response) { await response.arrayBuffer(); commits++; } };
  const options = { usedBytes: 0, maxBytes: 10000, signal: new AbortController().signal, onProgress() {} };
  try {
    for (const type of ["text/html", "application/json", "audio/mpeg", "application/octet-stream"]) {
      const body = "<html>Temporary error</html>";
      globalThis.fetch = async () => new Response(body, { headers: { "content-type": type, "content-length": String(body.length) } });
      await assert.rejects(downloadAudio("x", cache, options), { reason: "format" });
    }
    assert.equal(commits, 0);
  } finally { globalThis.fetch = originalFetch; }
});

test("downloads cancel a stalled body without reporting completion", async () => {
  const originalFetch = globalThis.fetch;
  let cancelled = false;
  try {
    globalThis.fetch = async () => new Response(new ReadableStream({ cancel() { cancelled = true; } }), {
      headers: { "content-length": "100" },
    });
    await assert.rejects(downloadAudio("x", { async put(_url, response) { await response.arrayBuffer(); } }, {
      usedBytes: 0, maxBytes: 1000, signal: new AbortController().signal, inactivityMs: 20,
      onProgress(progress) { assert.notEqual(progress.percent, 100); },
    }), { reason: "network" });
    assert.equal(cancelled, true);
  } finally { globalThis.fetch = originalFetch; }
});
