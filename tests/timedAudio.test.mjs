import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeTimings, timingAt, timedAudioUrl } from '../src/timedAudio.ts';
import { TimedRecitation } from '../src/timedRecitation.ts';
const raw = [
  {ayah:1,start_time:63,end_time:2310}, {ayah:2,start_time:2310,end_time:4515},
  {ayah:3,start_time:4515,end_time:7287}, {ayah:4,start_time:7287,end_time:10668},
];
test('timings require complete ordered verses and valid monotonic intervals', () => {
  const rows=normalizeTimings(raw,4);
  assert.equal(timingAt(rows,2.309).ayah,1); assert.equal(timingAt(rows,2.31).ayah,2);
  assert.equal(timingAt(rows,0),null);
  for(const invalid of [raw.slice(1), [...raw,raw[0]], raw.map((r,i)=>i===1?{...r,start_time:2000}:r), raw.map((r,i)=>i===2?{...r,end_time:NaN}:r)]) assert.throws(()=>normalizeTimings(invalid,4));
  assert.equal(timedAudioUrl(1001,112),'https://cdn.mp3quran.net/audio/yasser-dosari/r1/112.mp3');
  assert.equal(timedAudioUrl(7,112),'https://cdn.mp3quran.net/audio/mishary-alafasy/r1/112.mp3');
  assert.equal(timedAudioUrl(1001,115),''); assert.equal(timedAudioUrl(99,1),'');
});
class Media extends EventTarget {
  duration=10.8; currentTime=0; src=''; playbackRate=1; defaultPlaybackRate=1; paused=true;
  buffered={length:0};
  load(){if(this.src)queueMicrotask(()=>this.dispatchEvent(new Event('loadedmetadata')));}
  play(){this.paused=false;return Promise.resolve();} pause(){this.paused=true;}
  removeAttribute(){this.src='';}
}
test('continuous follow-along keeps one source, speed, verse tail and explicit range boundaries', async context => {
  context.mock.method(globalThis,'fetch',async()=>new Response(JSON.stringify(raw)));
  const storage=new Map();
  globalThis.localStorage={getItem:key=>storage.get(key)??null,setItem:(key,value)=>storage.set(key,value)};
  globalThis.caches={open:async()=>({match:async()=>undefined})};
  const media=new Media(), verses=[], statuses=[];
  const player=new TimedRecitation(media,1001,{surah:112,ayah:1},{surah:112,ayah:4},[{id:112,versesCount:4}],()=>1.5,s=>verses.push(s.basmala ? 0 : s.ayah),s=>statuses.push(s));
  player.play(); await new Promise(r=>setTimeout(r,25));
  assert.equal(media.paused,false);assert.equal(media.playbackRate,1.5);const source=media.src;
  for(const time of [2.2,2.31,4.515,7.287,10.7]){media.currentTime=time;media.dispatchEvent(new Event('timeupdate'));}
  assert.equal(media.src,source);assert.deepEqual(verses,[0,1,2,3,4]);assert.equal(statuses.includes('finished'),false,'Whole surah must retain its audio tail');
  media.dispatchEvent(new Event('ended'));assert.equal(statuses.at(-1),'finished');player.dispose();
  const partial=new TimedRecitation(media,1001,{surah:112,ayah:2},{surah:112,ayah:2},[{id:112,versesCount:4}],()=>2,()=>{},s=>statuses.push(s));
  partial.play();await new Promise(r=>setTimeout(r,25));assert.equal(media.currentTime,2.31);assert.equal(media.playbackRate,2);
  media.currentTime=4.514;media.dispatchEvent(new Event('timeupdate'));assert.equal(media.paused,false);
  media.currentTime=4.515;media.dispatchEvent(new Event('timeupdate'));assert.equal(media.paused,true);assert.equal(statuses.at(-1),'finished');
  const completedCount=statuses.filter(s=>s==='finished').length;
  media.dispatchEvent(new Event('timeupdate'));media.dispatchEvent(new Event('ended'));
  assert.equal(statuses.filter(s=>s==='finished').length,completedCount,'A completed range reports completion once, including late media events');
  partial.dispose();
  delete globalThis.localStorage;delete globalThis.caches;
});
