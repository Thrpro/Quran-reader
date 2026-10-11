export type AyahTiming = { ayah: number; start: number; end: number };
export const TIMED_RECORDINGS: Readonly<Record<number, { read: number; folder: string }>> = {
  // IDs and folders come together from MP3Quran /api/v3/ayat_timing/reads.
  // Never attach these timings to a recording from EveryAyah or another server.
  1001: { read: 92, folder: 'https://cdn.mp3quran.net/audio/yasser-dosari/r1/' },
  7: { read: 123, folder: 'https://cdn.mp3quran.net/audio/mishary-alafasy/r1/' },
};
export function timedAudioUrl(reciter: number, surah: number) {
  return TIMED_RECORDINGS[reciter] && Number.isInteger(surah) && surah >= 1 && surah <= 114
    ? TIMED_RECORDINGS[reciter].folder + String(surah).padStart(3, '0') + '.mp3' : '';
}
export function normalizeTimings(value: unknown, count: number): AyahTiming[] {
  if (!Array.isArray(value) || value.length > count + 1) throw Error('Invalid timing data');
  const rows = value.filter(row => row?.ayah !== 0);
  if (rows.length !== count) throw Error('Incomplete ayah timings');
  let previousEnd = 0;
  return rows.map((row, index) => {
    if (row.ayah !== index + 1 || !Number.isSafeInteger(row.start_time) || !Number.isSafeInteger(row.end_time) ||
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
export async function loadTimings(reciter: number, surah: number, count: number, signal: AbortSignal) {
  const recording = TIMED_RECORDINGS[reciter]; if (!recording) throw Error('Recording has no verified timings');
  const key = `quran-mp3quran-timings-v1-${recording.read}-${surah}`;
  try { const saved = localStorage.getItem(key); if (saved) return normalizeTimings(JSON.parse(saved), count); } catch { /* Fetch a fresh verified table. */ }
  const response = await fetch(`https://www.mp3quran.net/api/v3/ayat_timing?surah=${surah}&read=${recording.read}`, { signal: AbortSignal.any([signal, AbortSignal.timeout(15000)]) });
  if (!response.ok) throw Error('Timing service unavailable');
  // Read with a bound even if the provider sends no content-length.
  const reader = response.body?.getReader(); if (!reader) throw Error('Missing timing data');
  let bytes = 0, text = ''; const decoder = new TextDecoder();
  try {
    while (true) { const chunk = await reader.read(); if (chunk.done) break; bytes += chunk.value.byteLength;
      if (bytes > 1024 * 1024) throw Error('Timing table too large'); text += decoder.decode(chunk.value, { stream: true }); }
  } finally { await reader.cancel().catch(() => {}); }
  const raw = JSON.parse(text + decoder.decode()); const timings = normalizeTimings(raw, count);
  signal.throwIfAborted(); try { localStorage.setItem(key, JSON.stringify(raw)); } catch { /* Offline metadata is optional. */ }
  return timings;
}
