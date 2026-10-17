import test from 'node:test';
import assert from 'node:assert/strict';
import { setImmediate as nextTurn } from 'node:timers/promises';
import { TIMED_RECORDINGS, hasOfflineTimings, loadTimings, saveOfflineTimings, timingCacheUrl } from '../src/timedAudio.ts';
import { TimedRecitation } from '../src/timedRecitation.ts';

const raw = [
  {ayah:1,start_time:63,end_time:2310}, {ayah:2,start_time:2310,end_time:4515},
  {ayah:3,start_time:4515,end_time:7287}, {ayah:4,start_time:7287,end_time:10668},
];
const signal = () => new AbortController().signal;
async function flush() { await nextTurn(); await nextTurn(); }
function storage(context) {
  const entries = new Map(), legacy = new Map();
  const cache = {
    async match(key) { return entries.get(key)?.clone(); },
    async put(key,response) { entries.set(key,new Response(await response.arrayBuffer(), {headers:response.headers})); },
    async delete(key) { return entries.delete(key); },
  };
  context.mock.method(globalThis, 'fetch', async () => new Response(JSON.stringify(raw)));
  const original = { caches: globalThis.caches, localStorage: globalThis.localStorage, window: globalThis.window };
  globalThis.caches={open:async()=>cache};
  globalThis.localStorage={getItem:key=>legacy.get(key)??null, setItem:(key,value)=>legacy.set(key,value), removeItem:key=>legacy.delete(key)};
  globalThis.window=new EventTarget();
  context.after(()=>Object.assign(globalThis,original));
  return {cache,entries,legacy};
}
class Media extends EventTarget {
  duration=10.8; currentTime=0; src=''; playbackRate=1; defaultPlaybackRate=1; paused=true; loads=0; plays=0;
  buffered={length:0};
  load() { if(this.src) { this.loads++; this.currentTime=0; queueMicrotask(()=>this.dispatchEvent(new Event('loadedmetadata'))); } }
  play() { this.paused=false; this.plays++; return Promise.resolve(); }
  pause() { this.paused=true; }
  removeAttribute() { this.src=''; }
}
function playback(context,end={surah:114,ayah:6}) {
  storage(context); const media=new Media(), statuses=[], verses=[];
  const player=new TimedRecitation(media,1001,{surah:112,ayah:1},end,[{id:112,versesCount:4},{id:113,versesCount:4}],()=>1.5,point=>verses.push(point),status=>statuses.push(status));
  context.after(()=>player.dispose()); player.play();
  return {player,media,statuses,verses};
}

test('all matched whole recordings have their own verified folders and no unsupported style is inferred', () => {
  assert.equal(Object.keys(TIMED_RECORDINGS).length,11);
  assert.equal(new Set(Object.values(TIMED_RECORDINGS).map(row=>row.read)).size,11);
  assert.equal(TIMED_RECORDINGS[2].read,53);
  for(const unsupported of [1002,8,12,1,9,10]) assert.equal(TIMED_RECORDINGS[unsupported],undefined);
});

test('offline follow-along persists validated timings with audio and survives localStorage loss/service outage', async context => {
  const {cache,entries}=storage(context);
  assert.equal(await hasOfflineTimings(cache,1001,112,4),false,'An MP3-only download is incomplete');
  const saved=await saveOfflineTimings(cache,1001,112,4,signal());
  assert.ok(entries.has(timingCacheUrl(1001,112)));
  delete globalThis.localStorage;
  context.mock.method(globalThis,'fetch',async()=>{throw Error('offline');});
  assert.equal(await hasOfflineTimings(cache,1001,112,4),true);
  assert.deepEqual(await loadTimings(1001,112,4,signal()),saved);
  await cache.delete(timingCacheUrl(1001,112));
  assert.equal(await hasOfflineTimings(cache,1001,112,4),false);
  await assert.rejects(loadTimings(1001,112,4,signal()),/offline/);
});

test('offline completion rejects missing/corrupt timings, oversized data, quota failures and cancellation', async context => {
  const {cache,entries}=storage(context);
  entries.set(timingCacheUrl(1001,112),new Response(JSON.stringify(raw.slice(1))));
  assert.equal(await hasOfflineTimings(cache,1001,112,4),false);
  assert.equal(entries.size,0,'Corrupt metadata is evicted');
  context.mock.method(globalThis,'fetch',async()=>new Response(' '.repeat(1024*1024+1)));
  await assert.rejects(saveOfflineTimings(cache,1001,112,4,signal()),/too large/);
  context.mock.method(globalThis,'fetch',async()=>new Response(JSON.stringify(raw)));
  context.mock.method(cache,'put',async()=>{throw new DOMException('Full','QuotaExceededError');});
  await assert.rejects(saveOfflineTimings(cache,1001,112,4,signal()),{name:'QuotaExceededError'});
  const cancelled=new AbortController(); cancelled.abort();
  await assert.rejects(saveOfflineTimings(cache,1001,112,4,cancelled.signal),{name:'AbortError'});
});

test('valid legacy timings migrate without a network request and are removed only after persistence', async context => {
  const {cache,legacy}=storage(context); const key='quran-mp3quran-timings-v1-92-112';
  legacy.set(key,JSON.stringify(raw)); context.mock.method(globalThis,'fetch',async()=>{throw Error('not needed');});
  await loadTimings(1001,112,4,signal());
  assert.equal(await hasOfflineTimings(cache,1001,112,4),true); assert.equal(legacy.has(key),false);
});

test('timed recovery restores exact position/speed and never advances on a premature ended event', async context => {
  context.mock.timers.enable({apis:['setTimeout']});
  const {player,media,statuses}=playback(context); await flush();
  media.currentTime=6.123; media.dispatchEvent(new Event('ended'));
  assert.equal(player.chapter,112); assert.equal(statuses.at(-1),'retrying');
  context.mock.timers.tick(1000); await flush();
  assert.equal(media.loads,2); assert.equal(media.currentTime,6.123); assert.equal(media.playbackRate,1.5); assert.equal(media.paused,false);
  assert.equal(player.chapter,112); assert.equal(statuses.includes('finished'),false);
});

test('pausing during recovery cancels automatic playback, including after reconnect', async context => {
  context.mock.timers.enable({apis:['setTimeout']});
  const {player,media}=playback(context); await flush();
  media.currentTime=5.25; media.dispatchEvent(new Event('error')); player.pause();
  globalThis.window.dispatchEvent(new Event('online')); context.mock.timers.tick(60000); await flush();
  assert.equal(media.loads,1); assert.equal(media.paused,true);
  player.play(); await flush();
  assert.equal(media.loads,2); assert.equal(media.currentTime,5.25); assert.equal(media.paused,false);
});

test('a truncated response cannot shorten duration and masquerade as the end of a chapter', async context => {
  context.mock.timers.enable({apis:['setTimeout']});
  const {player,media,statuses}=playback(context); await flush();
  media.duration=3; media.currentTime=3; media.dispatchEvent(new Event('ended'));
  assert.equal(player.chapter,112); assert.equal(statuses.at(-1),'retrying');
  media.duration=10.8; context.mock.timers.tick(1000); await flush();
  assert.equal(media.currentTime,3); assert.equal(player.chapter,112); assert.equal(media.paused,false);
});

test('network recovery has a finite retry budget and transient waiting does not reload playing audio', async context => {
  context.mock.timers.enable({apis:['setTimeout']});
  const {media,statuses}=playback(context); await flush();
  media.currentTime=3.2; media.dispatchEvent(new Event('waiting'));
  context.mock.timers.tick(2000); media.currentTime=4.2; media.dispatchEvent(new Event('playing'));
  context.mock.timers.tick(8000); await flush(); assert.equal(media.loads,1);
  for(const delay of [1000,2000,4000,8000]) {
    media.dispatchEvent(new Event('error')); context.mock.timers.tick(delay); await flush();
  }
  assert.equal(media.loads,5);
  media.dispatchEvent(new Event('error')); globalThis.window.dispatchEvent(new Event('online')); context.mock.timers.tick(60000); await flush();
  assert.equal(statuses.at(-1),'error'); assert.equal(media.loads,5,'No infinite reconnect/reload loop');
});

test('changing chapter continuation preserves the active ayah and includes the final recording tail', async context => {
  const {player,media,statuses}=playback(context); await flush();
  const source=media.src; media.currentTime=6.123;
  player.setEnd({surah:112,ayah:4});
  assert.equal(media.src,source); assert.equal(media.currentTime,6.123); assert.equal(media.loads,1); assert.equal(media.paused,false);
  media.currentTime=10.7; media.dispatchEvent(new Event('timeupdate'));
  assert.equal(statuses.includes('finished'),false,'Chapter tail is preserved');
  media.currentTime=10.8; media.dispatchEvent(new Event('ended'));
  assert.equal(statuses.at(-1),'finished'); assert.equal(player.chapter,112);
});
