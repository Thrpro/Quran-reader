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

test('known-length fragmented clips retain byte identity and complete recording validation', async context => {
  const stream=player(), chunks=[new Uint8Array(clip.subarray(0,100)),new Uint8Array(clip.subarray(100))];
  const response=new Response(new ReadableStream({start(sink){for(const chunk of chunks)sink.enqueue(chunk);sink.close();}}),{
    headers:{'content-type':'audio/mpeg','content-length':String(clip.length)},
  });
  context.mock.method(globalThis,'fetch',async()=>response);
  const result=await stream.fetchClip('https://example.test/known.mp3',false);
  assert.equal(result.buffer.byteLength,clip.length);assert.deepEqual(result,new Uint8Array(clip));
});

test('unknown-length single chunks are reused only when they own a bounded backing buffer', async context => {
  const stream=player(),single=new Uint8Array(clip);
  const streamResponse=chunk=>new Response(new ReadableStream({start(sink){sink.enqueue(chunk);sink.close();}}));
  context.mock.method(globalThis,'fetch',async()=>streamResponse(single));
  assert.equal(await stream.fetchClip('https://example.test/single.mp3',false),single,'A complete standalone chunk needs no second full copy');
  const large=new Uint8Array(8*1024*1024+1);large.set(clip,12);const view=large.subarray(12,12+clip.length);
  context.mock.method(globalThis,'fetch',async()=>streamResponse(view));
  const copied=await stream.fetchClip('https://example.test/view.mp3',false);
  assert.notEqual(copied.buffer,large.buffer,'A small view must not retain an unbounded backing allocation');
  assert.equal(copied.buffer.byteLength,clip.length);assert.deepEqual(copied,single);
});

test('declared clip length overruns cancel the body and retry a damaged cache on the same URL', async context => {
  const stream=player(),url='https://example.test/short-length.mp3';let cancelled=false,deleted=0,requests=0;
  context.mock.method(globalThis,'fetch',async request=>{assert.equal(request,url);requests++;return new Response(clip);});
  const originalCaches=globalThis.caches;context.after(()=>{globalThis.caches=originalCaches;});
  globalThis.caches={async open(){return{
    async match(){return new Response(new ReadableStream({
      start(sink){sink.enqueue(new Uint8Array(clip));},cancel(){cancelled=true;},
    }),{headers:{'content-length':String(clip.length-1)}});},
    async delete(){deleted++;},
  };}};
  assert.deepEqual(await stream.fetchClip(url),new Uint8Array(clip));
  assert.equal(cancelled,true);assert.equal(deleted,1);assert.equal(requests,1);assert.equal(stream.cursor,0);
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

function managedPlayer(context) {
  const saved = { MediaSource:globalThis.MediaSource, window:globalThis.window,
    createObjectURL:URL.createObjectURL, revokeObjectURL:URL.revokeObjectURL };
  const intervals = new Map(), statuses = [];
  let nextTimer = 0, end = 10;
  const windowEvents = new EventTarget();
  windowEvents.setInterval = (callback, delay) => { const id=++nextTimer; intervals.set(id,{callback,delay}); return id; };
  windowEvents.clearInterval = id => intervals.delete(id);
  globalThis.window = windowEvents; globalThis.MediaSource = Source;
  URL.createObjectURL = () => 'blob:managed-timeline'; URL.revokeObjectURL = () => {};
  const media = new Media();
  media.currentTime=3.25; media.playbackRate=1.5; media.pauseCalls=0; media.playCalls=0;
  media.play=()=>{media.playCalls++;return Promise.resolve();};
  media.pause=()=>{media.pauseCalls++;};media.removeAttribute=()=>{};media.load=()=>{};
  const plan=[1,2].map(ayah=>({surah:112,ayah,url:`https://example.test/${ayah}.mp3`}));
  const stream=new RecitationStream(media,plan,()=>{},status=>statuses.push(status));
  stream.source.readyState='open';stream.source.endOfStream=()=>{stream.source.readyState='ended';};
  const buffer=new EventTarget();buffer.buffered={length:1,start:()=>0,end:()=>end};
  buffer.appendBuffer=()=>{end+=10;queueMicrotask(()=>buffer.dispatchEvent(new Event('updateend')));};
  stream.buffer=buffer;stream.timed=[{segment:plan[0],start:0,end:10,bytes:clip.length}];stream.cursor=1;
  context.after(()=>{stream.dispose();Object.assign(globalThis,{MediaSource:saved.MediaSource,window:saved.window});URL.createObjectURL=saved.createObjectURL;URL.revokeObjectURL=saved.revokeObjectURL;});
  return {stream,media,intervals,statuses,windowEvents};
}

test('clip watchdog sleeps while paused and stops on completion/dispose', async context => {
  const {stream,media,intervals}=managedPlayer(context);
  assert.equal(intervals.size,0,'constructing a paused recording has no periodic callback');
  stream.fetchClip=async()=>clip;
  stream.play();assert.equal(intervals.size,1);
  await new Promise(done=>setImmediate(done));
  stream.pause();assert.equal(intervals.size,0);assert.equal(stream.wanted,false);
  stream.play();assert.equal(intervals.size,1,'resume has exactly one watchdog');
  media.currentTime=20;media.dispatchEvent(new Event('ended'));
  assert.equal(intervals.size,0,'finished playback has no idle watchdog');
  stream.dispose();assert.equal(intervals.size,0);
});

test('permanent future-clip failure has a finite budget without cutting buffered audio', async context => {
  const {stream,media,intervals,windowEvents}=managedPlayer(context);
  let requests=0;
  stream.fetchClip=async()=>{requests++;throw Error('Network unavailable');};
  stream.wanted=true;stream.startWatchdog();
  for(let attempt=0;attempt<5;attempt++){stream.retryAt=0;await stream.pump();}
  assert.equal(requests,5,'one initial attempt and at most four automatic retries');
  assert.equal(stream.status,'error');assert.equal(stream.exhausted,true);
  assert.equal(stream.cursor,1,'the exact failed ayah remains pending');
  assert.equal(media.currentTime,3.25);assert.equal(media.pauseCalls,0,'buffered current verse is allowed to finish');
  assert.equal(intervals.size,0,'exhaustion stops periodic work');
  windowEvents.dispatchEvent(new Event('online'));stream.tick();await stream.pump();
  assert.equal(requests,5,'background/online events cannot silently reset an exhausted budget');
});

test('explicit retry retains source, position, speed and pending verse on a healthy decoder', async context => {
  const {stream,media,intervals}=managedPlayer(context);
  let requests=0;
  stream.fetchClip=async()=>{requests++;return clip;};
  stream.failures=5;stream.exhausted=true;stream.status='error';
  const source=media.src;
  assert.equal(stream.retryPending(),true);
  await new Promise(done=>setImmediate(done));
  assert.equal(requests,1);assert.equal(stream.cursor,2);assert.equal(stream.failures,0);
  assert.equal(media.src,source);assert.equal(media.currentTime,3.25);assert.equal(media.playbackRate,1.5);
  assert.equal(media.playCalls,1);assert.equal(intervals.size,1);
  stream.pause();assert.equal(intervals.size,0);
});

test('paused clip recovery waits for user action and decoder failure requests a rebuild', async context => {
  const {stream,media,windowEvents,intervals}=managedPlayer(context);
  let requests=0;
  stream.fetchClip=async()=>{requests++;throw Error('Network unavailable');};
  stream.wanted=true;await stream.pump();stream.pause();
  windowEvents.dispatchEvent(new Event('online'));await stream.pump();
  assert.equal(requests,1,'an online event never resumes paused playback');assert.equal(intervals.size,0);
  media.error={code:3};assert.equal(stream.retryPending(),false);
  assert.equal(media.playCalls,0,'a terminal decoder is not restarted on the broken timeline');
});

test('native pause/play events suspend and resume background work without source changes', async context => {
  const {stream,media,intervals}=managedPlayer(context);
  stream.fetchClip=async()=>clip;
  const source=media.src;
  media.paused=false;media.dispatchEvent(new Event('play'));
  assert.equal(stream.wanted,true);assert.equal(intervals.size,1);
  await new Promise(done=>setImmediate(done));
  media.paused=true;media.dispatchEvent(new Event('pause'));
  assert.equal(stream.wanted,false);assert.equal(intervals.size,0);
  media.paused=false;media.dispatchEvent(new Event('play'));
  assert.equal(stream.wanted,true);assert.equal(intervals.size,1);
  media.dispatchEvent(new Event('pause'));
  assert.equal(stream.wanted,true,'a queued stale pause event cannot interrupt a current play');
  assert.equal(media.src,source);assert.equal(media.currentTime,3.25);assert.equal(media.playbackRate,1.5);
  stream.dispose();const pauseCalls=media.pauseCalls;
  media.dispatchEvent(new Event('play'));assert.equal(intervals.size,0,'disposed event listeners cannot restart work');
  stream.dispose();assert.equal(media.pauseCalls,pauseCalls,'dispose is idempotent');
});

test('a late clip read cannot retain pending bytes or append after disposal', async context => {
  const {stream}=managedPlayer(context);let resolveClip,appended=0;
  stream.fetchClip=()=>new Promise(resolve=>{resolveClip=resolve;});
  stream.buffer.appendBuffer=()=>{appended++;};stream.wanted=true;
  const pumping=stream.pump();await new Promise(done=>setImmediate(done));
  stream.dispose();resolveClip(new Uint8Array(clip));await pumping;
  assert.equal(stream.pending,undefined);assert.equal(stream.timed.length,0);assert.equal(appended,0);
});

test('finishing a range drops a late future clip without changing the finished timeline', async context => {
  const {stream,media}=managedPlayer(context);let resolveClip,appended=0;
  stream.fetchClip=()=>new Promise(resolve=>{resolveClip=resolve;});
  stream.buffer.appendBuffer=()=>{appended++;};stream.wanted=true;
  const pumping=stream.pump();await new Promise(done=>setImmediate(done));
  stream.setEnd({surah:112,ayah:1});media.currentTime=10;stream.tick();
  resolveClip(new Uint8Array(clip));await pumping;
  assert.equal(stream.completed,true);assert.equal(stream.pending,undefined);assert.equal(stream.cursor,1);assert.equal(appended,0);
});

test('pausing during a clip read defers append and resumes with the same pending recording', async context => {
  const {stream,media}=managedPlayer(context);let resolveClip,requests=0;
  stream.fetchClip=()=>{requests++;return new Promise(resolve=>{resolveClip=resolve;});};
  stream.wanted=true;const pumping=stream.pump();await new Promise(done=>setImmediate(done));
  stream.pause();const bytes=new Uint8Array(clip);resolveClip(bytes);await pumping;
  assert.equal(stream.cursor,1);assert.equal(stream.pending,bytes);assert.equal(media.currentTime,3.25);
  stream.play();await new Promise(done=>setImmediate(done));
  assert.equal(requests,1,'A paused complete read is reused at the exact pending ayah');
  assert.equal(stream.cursor,2);assert.equal(stream.pending,undefined);assert.equal(media.currentTime,3.25);assert.equal(media.playbackRate,1.5);
});

test('a cache match that arrives after disposal is cancelled before allocating a clip', async context => {
  const {stream}=managedPlayer(context);let resolveMatch,cancelled=false;
  const originalCaches=globalThis.caches;context.after(()=>{globalThis.caches=originalCaches;});
  globalThis.caches={async open(){return{match:()=>new Promise(resolve=>{resolveMatch=resolve;})};}};
  const response=new Response(new ReadableStream({cancel(){cancelled=true;}}),{headers:{'content-length':String(8*1024*1024)}});
  const reading=stream.fetchClip('https://example.test/late-cache.mp3');await new Promise(done=>setImmediate(done));
  stream.dispose();resolveMatch(response);
  await assert.rejects(reading,{name:'AbortError'});assert.equal(cancelled,true);assert.equal(response.body.locked,false);
});

test('shortening continuation drops a pending future clip while preserving the playing verse', context => {
  const {stream,media}=managedPlayer(context);stream.pending=new Uint8Array(clip);
  const source=media.src;stream.setEnd({surah:112,ayah:1});
  assert.equal(stream.pending,undefined);assert.equal(stream.cursor,1);assert.equal(stream.completed,false);
  assert.equal(media.src,source);assert.equal(media.currentTime,3.25);assert.equal(media.playbackRate,1.5);assert.equal(media.pauseCalls,0);
});

test('a future clip failure after range completion cannot restart recovery or replace finished status', async context => {
  const {stream,media,statuses}=managedPlayer(context);let rejectClip;
  stream.fetchClip=()=>new Promise((resolve,reject)=>{rejectClip=reject;});
  stream.wanted=true;const pumping=stream.pump();await new Promise(done=>setImmediate(done));
  stream.setEnd({surah:112,ayah:1});media.currentTime=10;stream.tick();
  rejectClip(Error('Future request failed'));await pumping;
  assert.equal(stream.completed,true);assert.equal(stream.pending,undefined);assert.equal(stream.failures,0);assert.equal(stream.retryAt,0);
  assert.equal(statuses.at(-1),'finished');assert.equal(stream.cursor,1);
});

test('a future append completing after the range cannot replace finished status or retain a new verse', async context => {
  const {stream,media,statuses}=managedPlayer(context);let appended=0;
  stream.fetchClip=async()=>new Uint8Array(clip);stream.buffer.appendBuffer=()=>{appended++;};
  stream.wanted=true;const pumping=stream.pump();await new Promise(done=>setImmediate(done));assert.equal(appended,1);
  stream.setEnd({surah:112,ayah:1});media.currentTime=10;stream.tick();
  stream.buffer.dispatchEvent(new Event('updateend'));await pumping;
  assert.equal(stream.completed,true);assert.equal(stream.pending,undefined);assert.equal(stream.cursor,1);assert.equal(stream.timed.length,1);
  assert.equal(statuses.at(-1),'finished');
});
