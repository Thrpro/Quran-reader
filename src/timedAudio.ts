export type AyahTiming = { ayah: number; start: number; end: number };
export const TIMED_RECORDINGS: Readonly<Record<number, { read: number; folder: string }>> = {
  // Exact read/folder pairs from MP3Quran's timing catalog, checked 2026-10-17.
  // These IDs belong to our curated reciter list, not MP3Quran's read IDs.
  // Timings are never applied to an EveryAyah/Quran Foundation recording.
  1001: { read: 92, folder: 'https://cdn.mp3quran.net/audio/yasser-dosari/r1/' },
  1003: { read: 86, folder: 'https://cdn.mp3quran.net/audio/nasser-qatami/r1/' },
  1004: { read: 81, folder: 'https://cdn.mp3quran.net/audio/fares-abbad/r1/' },
  1005: { read: 62, folder: 'https://cdn.mp3quran.net/audio/abdullah-juhani/r1/' },
  2: { read: 53, folder: 'https://cdn.mp3quran.net/audio/abdulbasit-abdulsamad/r3/' }, // Murattal/Hafs
  3: { read: 54, folder: 'https://cdn.mp3quran.net/audio/abdulrahman-sudais/r1/' },
  4: { read: 4, folder: 'https://cdn.mp3quran.net/audio/abubakr-shatri/r1/' },
  5: { read: 89, folder: 'https://cdn.mp3quran.net/audio/hani-rifai/r1/' },
  6: { read: 118, folder: 'https://cdn.mp3quran.net/audio/mahmoud-husary/r1/' },
  7: { read: 123, folder: 'https://cdn.mp3quran.net/audio/mishary-alafasy/r1/' },
  11: { read: 106, folder: 'https://cdn.mp3quran.net/audio/muhammad-tablawi/r1/' },
  // Reads 31, 51 and 112 have incomplete Tawbah numbering in the current
  // provider table. Keep those reciter choices on their validated clip engine.
};
const AUDIO_CACHE = 'quran-audio-v1';
const MAX_TIMING_BYTES = 1024 * 1024;
type RawTiming = { ayah: number; start_time: number; end_time: number };
export function timedAudioUrl(reciter: number, surah: number) {
  return TIMED_RECORDINGS[reciter] && Number.isInteger(surah) && surah >= 1 && surah <= 114
    ? TIMED_RECORDINGS[reciter].folder + String(surah).padStart(3, '0') + '.mp3' : '';
}
/** Metadata lives alongside its MP3, so deleting offline audio removes both. */
export function timingCacheUrl(reciter: number, surah: number) {
  return timedAudioUrl(reciter, surah)
    ? `https://www.mp3quran.net/api/v3/ayat_timing?surah=${surah}&read=${TIMED_RECORDINGS[reciter].read}` : '';
}
export function normalizeTimings(value: unknown, count: number): AyahTiming[] {
  if (!Number.isSafeInteger(count) || count < 1 || count > 286 || !Array.isArray(value) || value.length > count + 1) throw Error('Invalid timing data');
  const rows = value.filter(row => row?.ayah !== 0);
  if (rows.length !== count) throw Error('Incomplete ayah timings');
  let previousEnd = 0;
  return rows.map((row, index) => {
    if (row?.ayah !== index + 1 || !Number.isSafeInteger(row.start_time) || !Number.isSafeInteger(row.end_time) ||
      row.start_time < previousEnd || row.end_time <= row.start_time || row.end_time > 24 * 3600 * 1000) throw Error('Invalid ayah interval');
    previousEnd = row.end_time;
    return { ayah: row.ayah, start: row.start_time / 1000, end: row.end_time / 1000 };
  });
}
export function timingAt(rows: AyahTiming[], seconds: number) {
  // Keep the last started verse during natural pauses. Never highlight a future verse.
  let low = 0, high = rows.length - 1, found = -1;
  while (low <= high) { const mid = (low + high) >>> 1; if (rows[mid].start <= seconds) { found = mid; low = mid + 1; } else high = mid - 1; }
  return found < 0 ? null : rows[found];
}
function legacyKey(reciter: number, surah: number) { return `quran-mp3quran-timings-v1-${TIMED_RECORDINGS[reciter].read}-${surah}`; }
function canonical(rows: AyahTiming[]): RawTiming[] {
  return rows.map(row => ({ ayah: row.ayah, start_time: Math.round(row.start * 1000), end_time: Math.round(row.end * 1000) }));
}
async function readTimingResponse(response: Response, count: number, signal?: AbortSignal) {
  if (!response.ok) throw Error('Timing service unavailable');
  const reader = response.body?.getReader(); if (!reader) throw Error('Missing timing data');
  let bytes = 0, text = ''; const decoder = new TextDecoder();
  const cancel = () => { void reader.cancel().catch(() => {}); };
  signal?.addEventListener('abort', cancel, { once: true });
  try {
    while (true) {
      signal?.throwIfAborted(); const chunk = await reader.read(); if (chunk.done) break;
      bytes += chunk.value.byteLength; if (bytes > MAX_TIMING_BYTES) throw Error('Timing table too large');
      text += decoder.decode(chunk.value, { stream: true });
    }
    signal?.throwIfAborted();
    return normalizeTimings(JSON.parse(text + decoder.decode()), count);
  } finally { signal?.removeEventListener('abort', cancel); await reader.cancel().catch(() => {}); }
}
async function cachedTimings(cache: Cache, reciter: number, surah: number, count: number) {
  const url = timingCacheUrl(reciter, surah); if (!url) return null;
  const saved = await cache.match(url); if (!saved) return null;
  try { return await readTimingResponse(saved, count); }
  catch { await cache.delete(url); return null; }
}
async function persistTimings(cache: Cache, reciter: number, surah: number, rows: AyahTiming[]) {
  const body = JSON.stringify(canonical(rows));
  await cache.put(timingCacheUrl(reciter, surah), new Response(body, {
    headers: { 'content-type': 'application/json', 'content-length': String(new TextEncoder().encode(body).length) },
  }));
  try { localStorage.removeItem(legacyKey(reciter, surah)); } catch { /* Migration does not require localStorage. */ }
}
/** A local-only package check. An MP3 alone is not a playable follow-along download. */
export async function hasOfflineTimings(cache: Cache, reciter: number, surah: number, count: number) {
  try { return !!await cachedTimings(cache, reciter, surah, count); } catch { return false; }
}
export async function loadTimings(reciter: number, surah: number, count: number, signal: AbortSignal) {
  if (!timingCacheUrl(reciter, surah)) throw Error('Recording has no verified timings');
  signal.throwIfAborted();
  let cache: Cache | undefined;
  try {
    cache = await caches.open(AUDIO_CACHE); const saved = await cachedTimings(cache, reciter, surah, count);
    signal.throwIfAborted(); if (saved) return saved;
  } catch { signal.throwIfAborted(); /* Streaming still works when storage is unavailable. */ }
  let legacy: AyahTiming[] | undefined;
  try { const saved = localStorage.getItem(legacyKey(reciter, surah)); if (saved) legacy = normalizeTimings(JSON.parse(saved), count); } catch { /* Ignore invalid legacy metadata. */ }
  if (legacy) {
    if (cache) try { await persistTimings(cache, reciter, surah, legacy); } catch { /* Preserve legacy data on migration failure. */ }
    signal.throwIfAborted(); return legacy;
  }
  const requestSignal = AbortSignal.any([signal, AbortSignal.timeout(15000)]);
  const response = await fetch(timingCacheUrl(reciter, surah), { signal: requestSignal });
  const rows = await readTimingResponse(response, count, requestSignal);
  if (cache) try { await persistTimings(cache, reciter, surah, rows); } catch { /* Downloads call the strict helper below. */ }
  signal.throwIfAborted(); return rows;
}
/** Must succeed before the UI reports a complete saved surah. Storage failures propagate. */
export async function saveOfflineTimings(cache: Cache, reciter: number, surah: number, count: number, signal: AbortSignal) {
  const rows = await loadTimings(reciter, surah, count, signal);
  signal.throwIfAborted(); await persistTimings(cache, reciter, surah, rows); signal.throwIfAborted();
  if (!await hasOfflineTimings(cache, reciter, surah, count)) throw Error('Offline timing write failed');
  return rows;
}
