import assert from 'node:assert/strict';
import test from 'node:test';
import {cachedAudioKey,audioByteRange,cachedAudioResponse,validateCachedAudio} from '../src/cachedAudioRange.ts';
const bytes=Uint8Array.from([255,251,144,0,0,0,0,0,0,0,0,0,0,0,...Array.from({length:86},(_,i)=>i)]);
function fixture(body=bytes,declared=body.length,chunk=7){
 let position=0,cancelled=false,reads=0;
 const response=new Response(new ReadableStream({pull(sink){reads++;if(position===body.length){sink.close();return;}const end=Math.min(body.length,position+chunk);sink.enqueue(body.subarray(position,end));position=end;},cancel(){cancelled=true;}},{highWaterMark:0}),{headers:{'content-type':'audio/mpeg','content-length':String(declared)}});
 return {response,state:()=>({position,cancelled,reads})};
}
const request=(range)=>new Request('https://quran-mushaf.local/__offline_audio__/recording',{headers:range?{range}:{}});
test('offline endpoint accepts only known HTTPS audio providers, without credentials or arbitrary URLs',()=>{
 assert.equal(cachedAudioKey('https://cdn.mp3quran.net/audio/yasser-dosari/r1/002.mp3'),'https://cdn.mp3quran.net/audio/yasser-dosari/r1/002.mp3');
 for(const url of ['https://evilmp3quran.net/a.mp3','https://cdn.mp3quran.net.evil.test/a.mp3','https://u:p@server.mp3quran.net/a.mp3','http://server.mp3quran.net/a.mp3','https://server.mp3quran.net:444/a.mp3','https://www.mp3quran.net/api/v3/reciters','https://everyayah.com/a.mp3?url=x','file:///a.mp3'])assert.equal(cachedAudioKey(url),null,url);
});
test('byte ranges support bounded, open-ended and suffix seeks and reject malformed or oversized ranges',()=>{
 for(const [header,value] of [[null,[0,99]],['bytes=10-19',[10,19]],['bytes=50-',[50,99]],['bytes=-10',[90,99]],['bytes=-200',[0,99]],['bytes=50-200',[50,99]]])assert.deepEqual(Object.values(audioByteRange(header,100)),value);
 for(const value of ['bytes=100-','bytes=10-5','bytes=-0','bytes=-','bytes=0-1,3-4','items=0-10','bytes=999999999999999999999-','bytes=-999999999999999999999'])assert.equal(audioByteRange(value,100),null,value);
});
test('streamed cached audio preserves exact bytes for full responses and repeated fragmented range seeks',async()=>{
 for(const range of [undefined,'bytes=0-0','bytes=10-29','bytes=50-','bytes=-7']){
  const input=fixture(bytes,100,1),result=cachedAudioResponse(input.response,request(range)),bounds=audioByteRange(range??null,100);
  assert.equal(result.status,range?206:200);
  assert.equal(Number(result.headers.get('content-length')),bounds.end-bounds.start+1);
  assert.deepEqual(new Uint8Array(await result.arrayBuffer()),bytes.subarray(bounds.start,bounds.end+1));
  assert.equal(input.state().cancelled,true);
 }
});
test('range delivery obeys consumer backpressure and cancels the disk reader on Stop',async()=>{
 const input=fixture(bytes,100,16),result=cachedAudioResponse(input.response,request('bytes=0-'));
 assert.equal(input.state().reads,0,'no eager whole-recording read');
 const reader=result.body.getReader();await reader.read();assert.equal(input.state().position,16);
 await reader.cancel();assert.equal(input.state().cancelled,true);assert.equal(input.state().position,16);
});
test('cached-file validation consumes bounded chunks and rejects truncated, oversized or invalid media',async()=>{
 for(const [body,length,valid] of [[bytes,100,true],[bytes,101,false],[bytes,99,false],[new Uint8Array(100),100,false]]){
  const input=fixture(body,length);assert.equal(await validateCachedAudio(input.response),valid);assert.ok(input.state().cancelled || input.state().position===body.length,'validation cancels or exhausts its reader');
 }
 const input=fixture();const controller=new AbortController();controller.abort();await assert.rejects(validateCachedAudio(input.response,controller.signal),{name:'AbortError'});assert.equal(input.state().cancelled,true);
});
test('range response rejects unsatisfiable seeks and reports corrupt cached bodies as stream errors',async()=>{
 const input=fixture(),result=cachedAudioResponse(input.response,request('bytes=100-'));
 assert.equal(result.status,416);assert.equal(result.headers.get('content-range'),'bytes */100');assert.equal(input.state().cancelled,true);
 for(const [body,size] of [[bytes.subarray(0,60),100],[new Uint8Array(100),100]])await assert.rejects(cachedAudioResponse(fixture(body,size).response,request()).arrayBuffer());
});
