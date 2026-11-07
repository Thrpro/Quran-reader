import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { mp3Signature, validAudioResponse, completeMp3, audioResponseBlob, MAX_AUDIO_PREFIX } from "../src/audioFormat.ts";

test("audio validation accepts real MP3 with fragmented metadata and octet-stream MIME", async () => {
  const clip = readFileSync(new URL("./fixtures/yasser-112001.mp3", import.meta.url));
  assert.equal(mp3Signature(clip), "valid");
  assert.equal(await validAudioResponse(new Response(new ReadableStream({ start(controller) {
    for (let offset = 0; offset < clip.length; offset += 17) controller.enqueue(clip.subarray(offset, offset + 17));
    controller.close();
  } }), { headers: { "content-type": "application/octet-stream" } })), true);
  assert.equal(mp3Signature(new Uint8Array([73,68,51,4,0,0,127,127,127,127])), "invalid", "unbounded metadata rejected");
  assert.equal(await validAudioResponse(new Response("<html>error</html>", { headers: { "content-type": "audio/mpeg" } })), false);
});

test("complete ayah validation rejects a cut-off final MP3 frame before it can play", () => {
  const clip = readFileSync(new URL("./fixtures/yasser-112001.mp3", import.meta.url));
  assert.equal(completeMp3(clip), true);
  for (const length of [10, 100, Math.floor(clip.length / 2), clip.length - 1, clip.length - 20]) {
    assert.equal(completeMp3(clip.subarray(0, length)), false, `truncation at ${length}`);
  }
});

test("a declared MP3 frame count catches truncation even at a complete frame boundary", () => {
  const bytes = new Uint8Array(417 * 2);
  for (const offset of [0, 417]) bytes.set([255,251,144,0], offset);
  bytes.set(new TextEncoder().encode('Xing'), 36);
  const view = new DataView(bytes.buffer);
  view.setUint32(40, 1); view.setUint32(44, 10);
  assert.equal(completeMp3(bytes), false);
  view.setUint32(44, 2);
  assert.equal(completeMp3(bytes), true);
});

test('cached playback consumes one stream without cloning or a full-response ArrayBuffer', async context => {
  const clip = readFileSync(new URL('./fixtures/yasser-112001.mp3', import.meta.url));
  let offset = 0;
  const response = new Response(new ReadableStream({
    pull(controller) {
      if (offset === clip.length) { controller.close(); return; }
      const end = Math.min(clip.length, offset + 17);
      controller.enqueue(clip.subarray(offset, end)); offset = end;
    },
  }), { headers: { 'content-type': 'application/octet-stream', 'content-length': String(clip.length) } });
  context.mock.method(response, 'clone', () => { throw Error('Unexpected response clone'); });
  context.mock.method(response, 'arrayBuffer', () => { throw Error('Unexpected full recording ArrayBuffer'); });
  const blob = await audioResponseBlob(response);
  assert.equal(response.bodyUsed, true); assert.equal(blob.size, clip.length);
  assert.deepEqual(new Uint8Array(await blob.arrayBuffer()), new Uint8Array(clip), 'Original recording bytes are preserved');
});

test('cached Blob validation rejects error documents and mismatched byte lengths', async () => {
  const clip = readFileSync(new URL('./fixtures/yasser-112001.mp3', import.meta.url));
  for (const headers of [
    { 'content-type': 'application/json' },
    { 'content-length': String(clip.length - 1) },
    { 'content-length': String(clip.length + 1) },
  ]) assert.equal(await audioResponseBlob(new Response(clip, { headers })), undefined);
  assert.equal(await audioResponseBlob(new Response('<html>Temporary error</html>', { headers: { 'content-type': 'audio/mpeg' } })), undefined);
  assert.equal(await audioResponseBlob(new Response(clip.subarray(0, 9))), undefined, 'Incomplete MPEG prefix cannot become a playback source');
  const encoded = await audioResponseBlob(new Response(clip, { headers: { 'content-length': '1', 'content-encoding': 'gzip' } }));
  assert.equal(encoded.size, clip.length, 'An encoded transfer length does not describe the decoded body');
});

test('cached Blob validation cancels invalid bodies before reading a whole recording', async () => {
  let cancelled = false, reads = 0;
  const response = new Response(new ReadableStream({
    pull(controller) { reads++; controller.enqueue(new TextEncoder().encode('<html>Temporary error</html>')); },
    cancel() { cancelled = true; },
  }));
  assert.equal(await audioResponseBlob(response), undefined);
  await new Promise(done => setImmediate(done));
  assert.equal(cancelled, true); assert.ok(reads <= 3, `Only the bounded prefix was read (${reads} pulls)`);
});

test('cached Blob validation bounds ID3 metadata and accepts a fragmented header at the bound', async () => {
  const bytes = new Uint8Array(MAX_AUDIO_PREFIX + 64), size = MAX_AUDIO_PREFIX - 14;
  bytes.set([73, 68, 51, 4, 0, 0, size >>> 21 & 127, size >>> 14 & 127, size >>> 7 & 127, size & 127]);
  bytes.set([255, 251, 144, 0], MAX_AUDIO_PREFIX - 4);
  let offset = 0;
  const blob = await audioResponseBlob(new Response(new ReadableStream({
    pull(controller) {
      if (offset === bytes.length) { controller.close(); return; }
      const end = Math.min(bytes.length, offset + 997);
      controller.enqueue(bytes.subarray(offset, end)); offset = end;
    },
  })));
  assert.equal(blob.size, bytes.length);
  bytes[9]++;
  assert.equal(await audioResponseBlob(new Response(bytes)), undefined, 'Metadata beyond the existing prefix limit remains invalid');
});

test('disposing cached playback cancels a pending Blob read', async () => {
  const controller = new AbortController(); let cancelled = false;
  const response = new Response(new ReadableStream({
    start(sink) { sink.enqueue(new Uint8Array([255, 251, 144, 0, 0, 0, 0, 0, 0, 0])); },
    cancel() { cancelled = true; },
  }));
  const reading = audioResponseBlob(response, controller.signal);
  controller.abort();
  await assert.rejects(reading, { name: 'AbortError' }); assert.equal(cancelled, true);
  const preCancelled = new AbortController(); preCancelled.abort();
  let unopenedCancelled = false;
  await assert.rejects(audioResponseBlob(new Response(new ReadableStream({ cancel() { unopenedCancelled = true; } })), preCancelled.signal), { name: 'AbortError' });
  assert.equal(unopenedCancelled, true);
});
