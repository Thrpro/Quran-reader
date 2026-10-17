import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { setImmediate as nextTurn } from 'node:timers/promises';
import { RecitationStream } from '../src/recitationStream.ts';
const clip=readFileSync(new URL('./fixtures/yasser-112001.mp3',import.meta.url));
class Source extends EventTarget { readyState='open'; endOfStream(){this.readyState='ended';} }
class Media extends EventTarget { currentTime=0; playbackRate=1.5; paused=false; src=''; play(){this.paused=false;return Promise.resolve();} pause(){this.paused=true;} removeAttribute(){this.src='';} load(){} }
async function fixture(context) {
  const originals={MediaSource:globalThis.MediaSource,window:globalThis.window,caches:globalThis.caches};
  globalThis.MediaSource=Source; globalThis.window={setInterval(){return 0;}};
  context.mock.method(URL,'createObjectURL',()=> 'blob:unchanged'); context.mock.method(URL,'revokeObjectURL',()=>{});
  globalThis.caches={open:async()=>({match:async()=>undefined})};
  context.mock.method(globalThis,'fetch',async()=>new Response(clip));
  const statuses=[],media=new Media();
  const plan=[1,2].map(ayah=>({surah:112,ayah,url:`https://example.test/112-${ayah}.mp3`}));
  const player=new RecitationStream(media,plan,()=>{},status=>statuses.push(status));
  let end=0;
  const buffer=new EventTarget(); buffer.buffered={length:1,start:()=>0,end:()=>end};
  buffer.appendBuffer=()=>{player.source.readyState='open';end+=3;queueMicrotask(()=>buffer.dispatchEvent(new Event('updateend')));};
  player.buffer=buffer; player.wanted=true;
  context.after(()=>{player.dispose();Object.assign(globalThis,originals);});
  await player.pump();
  return {player,media,statuses};
}

test('enabling continuation after endOfStream extends the existing timeline without restarting the current ayah',async context=>{
  const {player,media}=await fixture(context);
  assert.equal(player.source.readyState,'ended'); assert.equal(player.cursor,2);
  media.currentTime=2.5; const source=media.src;
  const future=[1,2].map(ayah=>({surah:113,ayah,url:`https://example.test/113-${ayah}.mp3`}));
  player.extendPlan(future); player.extendPlan(future); // UI state reapplication is harmless.
  player.setEnd({surah:113,ayah:2});
  await nextTurn(); await nextTurn();
  assert.equal(player.plan.length,4); assert.equal(player.cursor,4); assert.equal(player.duration,12);
  assert.equal(media.src,source); assert.equal(media.currentTime,2.5); assert.equal(media.playbackRate,1.5);
});

test('disabling continuation leaves the current ayah playing and stops only after the current chapter tail',async context=>{
  const {player,media,statuses}=await fixture(context);
  player.extendPlan([{surah:113,ayah:1,url:'https://example.test/113-1.mp3'}]); player.setEnd({surah:113,ayah:1});
  await nextTurn(); await nextTurn(); assert.equal(player.cursor,3);
  media.currentTime=4.123; player.setEnd({surah:112,ayah:2}); player.tick();
  assert.equal(media.currentTime,4.123); assert.equal(media.paused,false); assert.equal(statuses.includes('finished'),false);
  media.currentTime=5.99; player.tick(); assert.equal(media.paused,false);
  media.currentTime=6; player.tick(); assert.equal(media.paused,true); assert.equal(statuses.at(-1),'finished');
  player.tick(); player.ended(); assert.equal(statuses.filter(status=>status==='finished').length,1);
});
