import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { mp3Signature, validAudioResponse, completeMp3 } from "../src/audioFormat.ts";

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
