import { isErrorContentType, MAX_AUDIO_PREFIX, mp3Signature } from "./audioFormat.ts";
export type DownloadProgress = { received: number; total: number; percent: number };
export type DownloadFailure = "network" | "storage" | "size" | "incomplete" | "format";

export class AudioDownloadError extends Error {
  readonly reason: DownloadFailure;
  constructor(reason: DownloadFailure) {
    super(reason);
    this.reason = reason;
    this.name = "AudioDownloadError";
  }
}

/** Stream directly into the offline cache, enforcing the storage budget.
 * Progress reaches 100 only after the cache write has completed. */
export async function downloadAudio(
  url: string,
  cache: Cache,
  options: {
    usedBytes: number;
    maxBytes: number;
    signal: AbortSignal;
    onProgress: (progress: DownloadProgress) => void;
    inactivityMs?: number;
  },
) {
  const timeout = new AbortController();
  let timer: ReturnType<typeof setTimeout>;
  const resetTimeout = () => {
    clearTimeout(timer);
    timer = setTimeout(() => timeout.abort(), options.inactivityMs ?? 30000);
  };
  const signal = AbortSignal.any([options.signal, timeout.signal]);
  resetTimeout();
  try {
    const response = await fetch(url, { signal });
    if (!response.ok || !response.body) throw new AudioDownloadError("network");
    const total = Number(response.headers.get("content-length"));
    if (!Number.isSafeInteger(total) || total <= 0) {
      await response.body.cancel();
      throw new AudioDownloadError("size");
    }
    if (total + options.usedBytes > options.maxBytes) {
      await response.body.cancel();
      throw new AudioDownloadError("storage");
    }
    if (isErrorContentType(response.headers.get("content-type"))) {
      await response.body.cancel();
      throw new AudioDownloadError("format");
    }
    let received = 0;
    let lastPercent = -1;
    let validated = false;
    let prefix = new Uint8Array(0);
    options.onProgress({ received, total, percent: 0 });
    const bounded = response.body.pipeThrough(new TransformStream<Uint8Array, Uint8Array>({
      transform(chunk, sink) {
        resetTimeout();
        received += chunk.byteLength;
        if (received > total) throw new AudioDownloadError("incomplete");
        const percent = Math.min(99, Math.floor(received / total * 100));
        if (percent !== lastPercent) {
          lastPercent = percent;
          options.onProgress({ received, total, percent });
        }
        if (validated) sink.enqueue(chunk);
        else {
          const length = Math.min(MAX_AUDIO_PREFIX, prefix.length + chunk.length);
          const next = new Uint8Array(length); next.set(prefix);
          next.set(chunk.subarray(0, length - prefix.length), prefix.length);
          const signature = mp3Signature(next);
          if (signature === "invalid" || signature === "pending" && length === MAX_AUDIO_PREFIX) throw new AudioDownloadError("format");
          if (signature === "valid") {
            validated = true;
            if (prefix.length) sink.enqueue(prefix);
            sink.enqueue(chunk); prefix = new Uint8Array(0);
          } else prefix = next;
        }
      },
      flush() {
        if (received !== total) throw new AudioDownloadError("incomplete");
        if (!validated) throw new AudioDownloadError("format");
      },
    }), { signal });
    await cache.put(url, new Response(bounded, {
      headers: { "content-type": "audio/mpeg", "content-length": String(total) },
    }));
    options.onProgress({ received: total, total, percent: 100 });
  } catch (error) {
    if (timeout.signal.aborted && !options.signal.aborted) throw new AudioDownloadError("network");
    throw error;
  } finally { clearTimeout(timer!); }
}

