import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { RecitationStream } from '../src/recitationStream.ts';

const clip = readFileSync(new URL('./fixtures/yasser-112001.mp3', import.meta.url));
class Source extends EventTarget {}
class Media extends EventTarget { currentTime = 0; playbackRate = 1; play() { return Promise.resolve(); } pause() {} }

function player(notifications = []) {
  const original = { MediaSource: globalThis.MediaSource, window: globalThis.window, createObjectURL: URL.createObjectURL };
  globalThis.MediaSource = Source;
  globalThis.window = { setInterval() { return 0; } };
  URL.createObjectURL = () => 'blob:test';
  try { return new RecitationStream(new Media(), [], item => notifications.push(item.ayah), () => {}); }
  finally { Object.assign(globalThis, { MediaSource: original.MediaSource, window: original.window }); URL.createObjectURL = original.createObjectURL; }
}

test('preloading future ayahs never advances the highlight before their start time', () => {
  const notifications = [], stream = player(notifications);
  stream.timed = [
    { segment: { ayah: 1 }, start: .03, end: 5 },
    { segment: { ayah: 2 }, start: 5.03, end: 10 },
    { segment: { ayah: 3 }, start: 10.03, end: 15 },
  ];
  for (const [time, expected] of [[0, []], [.03, [1]], [5.01, [1]], [5.03, [1, 2]], [10.01, [1, 2]], [10.03, [1, 2, 3]]]) {
    stream.media.currentTime = time; stream.tick(); assert.deepEqual(notifications, expected);
  }
});

test('incomplete ayahs retry the same URL and a damaged cached clip is replaced', async () => {
  const saved = { fetch: globalThis.fetch, caches: globalThis.caches };
  const stream = player(); let requests = 0, deleted = 0;
  try {
    globalThis.caches = { async open() { return { async match() { return new Response(clip.subarray(0, clip.length - 20)); }, async delete() { deleted++; } }; } };
    globalThis.fetch = async () => { requests++; return new Response(clip); };
    assert.equal((await stream.fetchClip('https://example.test/001002.mp3')).length, clip.length);
    assert.equal(deleted, 1); assert.equal(requests, 1);
    globalThis.fetch = async () => new Response(clip.subarray(0, clip.length - 20));
    await assert.rejects(stream.fetchClip('x', false), /Incomplete/);
    globalThis.fetch = async () => new Response(clip, { headers: { 'content-length': String(clip.length + 1) } });
    await assert.rejects(stream.fetchClip('x', false), /Incomplete/);
    assert.equal(stream.cursor, 0, 'failed download must not advance the plan');
  } finally { Object.assign(globalThis, saved); }
});

test('a truncated middle ayah blocks advancement until that exact recording is downloaded completely', async () => {
  const saved = { fetch: globalThis.fetch, caches: globalThis.caches };
  const stream = player(), requests = [], appended = [];
  let end = 0, damaged = true;
  const buffer = new EventTarget();
  buffer.buffered = { length: 1, start: () => 0, end: () => end };
  buffer.appendBuffer = bytes => { appended.push(bytes.length); end += 3; queueMicrotask(() => buffer.dispatchEvent(new Event('updateend'))); };
  stream.buffer = buffer; stream.source.readyState = 'open';
  stream.source.endOfStream = () => { stream.source.readyState = 'ended'; };
  stream.plan = [1, 2, 3].map(ayah => ({ surah: 112, ayah, url: `https://example.test/${ayah}.mp3` }));
  stream.wanted = true;
  try {
    globalThis.caches = { async open() { return { async match() {} }; } };
    globalThis.fetch = async url => { requests.push(url); return new Response(url.endsWith('/2.mp3') && damaged ? clip.subarray(0, clip.length - 20) : clip); };
    await stream.pump();
    assert.equal(stream.cursor, 1); assert.equal(stream.status, 'retrying');
    assert.deepEqual(appended, [clip.length]);
    assert.deepEqual(requests.map(url => url.split('/').at(-1)), ['1.mp3', '2.mp3']);
    damaged = false; stream.retryAt = 0;
    await stream.pump();
    assert.deepEqual(requests.map(url => url.split('/').at(-1)), ['1.mp3', '2.mp3', '2.mp3', '3.mp3']);
    assert.deepEqual(appended, [clip.length, clip.length, clip.length]);
    assert.equal(stream.cursor, 3);
  } finally { Object.assign(globalThis, saved); }
});
