import { AudioPrefixValidator, isErrorContentType } from './audioFormat.ts';

export const OFFLINE_AUDIO_PATH = '/__offline_audio__/recording';
export const AUDIO_CACHE_NAME = 'quran-audio-v1';
export function cachedAudioKey(raw: string): string | null {
  try {
    const url = new URL(raw);
    if (url.protocol !== 'https:' || url.username || url.password || url.port || url.hash || url.search ||
      !/\.mp3$/i.test(url.pathname) || !(url.hostname.endsWith('.mp3quran.net') || url.hostname === 'everyayah.com' || url.hostname === 'verses.quran.foundation')) return null;
    return url.href;
  } catch { return null; }
}
export function audioByteRange(header: string | null, size: number): {start:number;end:number} | null {
  if (!Number.isSafeInteger(size) || size <= 0) return null;
  if (!header) return {start:0,end:size-1};
  const match = /^bytes=(\d*)-(\d*)$/.exec(header);
  if (!match || !match[1] && !match[2]) return null;
  if ([match[1],match[2]].some(value=>value && !Number.isSafeInteger(Number(value)))) return null;
  const first = match[1] ? Number(match[1]) : Math.max(0, size-Number(match[2]));
  const last = match[1] ? match[2] ? Math.min(size-1,Number(match[2])) : size-1 : size-1;
  if (!Number.isSafeInteger(first) || !Number.isSafeInteger(last) || first < 0 || first >= size || last < first || !match[1] && Number(match[2]) === 0) return null;
  return {start:first,end:last};
}
/** A bounded streaming integrity pass for legacy cached entries, without Blob,
 * clone/tee, whole-recording arrays, transcoding or persistent JS media data. */
export async function validateCachedAudio(response: Response, signal?: AbortSignal): Promise<boolean> {
  const expected = Number(response.headers.get('content-length'));
  if (!response.ok || !response.body || isErrorContentType(response.headers.get('content-type')) ||
    response.headers.has('content-encoding') || !Number.isSafeInteger(expected) || expected <= 0) {
    void response.body?.cancel().catch(()=>{}); return false;
  }
  const reader = response.body.getReader(), prefix = new AudioPrefixValidator();
  const abort = () => { void reader.cancel().catch(()=>{}); };
  signal?.addEventListener('abort',abort,{once:true});
  let size = 0;
  try {
    for (;;) {
      signal?.throwIfAborted();
      const {value,done} = await reader.read();
      signal?.throwIfAborted();
      if (done) return size === expected && prefix.signature === 'valid';
      size += value.byteLength;
      if (size > expected || prefix.write(value) === 'invalid') return false;
    }
  } finally { signal?.removeEventListener('abort',abort); void reader.cancel().catch(()=>{}); reader.releaseLock(); }
}
/** CacheStorage streams from its existing disk entry. Range seeks discard
 * preceding chunks with bounded memory; no second full recording is assembled. */
export function cachedAudioResponse(response: Response, request: Request): Response {
  const size = Number(response.headers.get('content-length'));
  const range = audioByteRange(request.headers.get('range'),size);
  if (!response.ok || !response.body || isErrorContentType(response.headers.get('content-type')) || response.headers.has('content-encoding')) {
    void response.body?.cancel().catch(()=>{}); return new Response(null,{status:404});
  }
  if (!range) { void response.body.cancel().catch(()=>{}); return new Response(null,{status:416,headers:{'content-range':`bytes */${size}`}}); }
  const reader=response.body.getReader(),prefix=new AudioPrefixValidator();let offset=0,finished=false;const pending:Uint8Array[]=[];
  const cancel=()=>{if(!finished){finished=true;void reader.cancel().catch(()=>{});request.signal.removeEventListener('abort',cancel);}};
  request.signal.addEventListener('abort',cancel,{once:true});
  const stream=new ReadableStream<Uint8Array>({
    async pull(sink) {
      try {
        request.signal.throwIfAborted();
        for (;;) {
          const {value,done}=await reader.read();
          request.signal.throwIfAborted();
          if(done) {if(offset<=range.end || prefix.signature!=='valid')throw Error('Incomplete cached recording');cancel();sink.close();return;}
          if(prefix.write(value)==='invalid')throw Error('Invalid cached recording');
          const next=offset+value.byteLength;
          if(next>size)throw Error('Invalid cached length');
          const start=Math.max(range.start,offset),end=Math.min(range.end+1,next);
          if(end>start)pending.push(value.subarray(start-offset,end-offset));
          if(prefix.signature==='valid') {for(const chunk of pending)sink.enqueue(chunk);pending.length=0;}
          offset=next;
          if(offset>range.end && prefix.signature==='valid'){cancel();sink.close();return;}
          if(end>start && prefix.signature==='valid')return;
        }
      }catch(error){cancel();sink.error(error);}
    },
    cancel,
  },{highWaterMark:0});
  return new Response(stream,{status:request.headers.has('range')?206:200,headers:{
    'content-type':'audio/mpeg','content-length':String(range.end-range.start+1),'accept-ranges':'bytes','cache-control':'no-store',
    ...(request.headers.has('range')?{'content-range':`bytes ${range.start}-${range.end}/${size}`}:{})}});
}
