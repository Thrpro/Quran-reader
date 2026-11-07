import {AUDIO_CACHE_NAME,OFFLINE_AUDIO_PATH,cachedAudioKey,cachedAudioResponse,validateCachedAudio} from './cachedAudioRange.ts';
// This worker caches no app code and intercepts only its private audio endpoint.
// Its fixed root scope lets the existing reader remain an ordinary WebView app.
const worker = globalThis as unknown as {
  location: Location; skipWaiting():Promise<void>; clients:{claim():Promise<void>};
  addEventListener(type:'install'|'activate', listener:(event:{waitUntil(promise:Promise<unknown>):void})=>void):void;
  addEventListener(type:'fetch', listener:(event:{request:Request;respondWith(response:Promise<Response>):void})=>void):void;
};
worker.addEventListener('install',event=>event.waitUntil(worker.skipWaiting()));
worker.addEventListener('activate',event=>event.waitUntil(worker.clients.claim()));
worker.addEventListener('fetch',event=>{
  const endpoint=new URL(event.request.url);
  if(endpoint.origin!==worker.location.origin || endpoint.pathname!==OFFLINE_AUDIO_PATH)return;
  event.respondWith((async()=>{
    const key=cachedAudioKey(endpoint.searchParams.get('url')??'');
    if(!key || !['HEAD','GET'].includes(event.request.method))return new Response(null,{status:400});
    const cache=await caches.open(AUDIO_CACHE_NAME),response=await cache.match(key);
    if(!response)return new Response(null,{status:404});
    if(event.request.method==='HEAD'){
      const valid=await validateCachedAudio(response,event.request.signal);
      if(!valid){await cache.delete(key);return new Response(null,{status:404});}
      return new Response(null,{headers:{'content-type':'audio/mpeg','content-length':response.headers.get('content-length')!,'cache-control':'no-store'}});
    }
    return cachedAudioResponse(response,event.request);
  })().catch(()=>new Response(null,{status:503})));
});
