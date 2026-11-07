import {cachedAudioKey,OFFLINE_AUDIO_PATH} from './cachedAudioRange.ts';
let registration: Promise<boolean> | undefined;
function controlled() { return navigator.serviceWorker.controller?.scriptURL === new URL('/audio-cache-worker.js',location.href).href; }
async function audioWorker(): Promise<boolean> {
  if(typeof navigator==='undefined' || !('serviceWorker' in navigator) || typeof location==='undefined')return false;
  if(controlled())return true;
  return registration ??= new Promise<boolean>(resolve=>{
    let finished=false;
    const finish=(success:boolean)=>{if(finished)return;finished=true;clearTimeout(timer);navigator.serviceWorker.removeEventListener('controllerchange',changed);resolve(success);};
    const changed=()=>{if(controlled())finish(true);};
    const timer=setTimeout(()=>finish(false),2000);
    navigator.serviceWorker.addEventListener('controllerchange',changed);
    void navigator.serviceWorker.register('/audio-cache-worker.js',{scope:'/',type:'module',updateViaCache:'none'}).then(changed,()=>finish(false));
  });
}
/** undefined: use the established Blob fallback; null: invalid cached audio.
 * Preparation is lazy and only happens after an explicit playback request. */
export async function cachedAudioSource(url:string,signal:AbortSignal):Promise<string|null|undefined> {
  if(!cachedAudioKey(url) || !await audioWorker())return undefined;
  signal.throwIfAborted();
  const endpoint=OFFLINE_AUDIO_PATH+'?url='+encodeURIComponent(url);
  try { const response=await fetch(endpoint,{method:'HEAD',cache:'no-store',signal}); return response.ok?new URL(endpoint,location.href).href:response.status===404?null:undefined; }
  catch(error){signal.throwIfAborted();return undefined;}
}
