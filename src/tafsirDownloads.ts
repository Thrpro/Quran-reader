export type TafsirGroup = { fromAyah: number; toAyah: number; ayahKeys: number[]; text: string };
export type TafsirSurah = { lookup: Record<string, string>; groups: Record<string, TafsirGroup> };
export const EXTRA_TAFSIRS = [
  { id: "quran-94", resource: 94, name: "تفسير البغوي" },
  { id: "quran-14", resource: 14, name: "تفسير ابن كثير" },
  { id: "quran-15", resource: 15, name: "تفسير الطبري" },
  { id: "quran-90", resource: 90, name: "تفسير القرطبي" },
  { id: "quran-93", resource: 93, name: "التفسير الوسيط" },
] as const;
export type ExtraTafsirId = typeof EXTRA_TAFSIRS[number]["id"];
export type TafsirSource = "mukhtasar" | "saadi" | ExtraTafsirId;
export type PackInfo = { bytes: number; downloadedAt: string };
const prefix = "quran-tafsir-v1-";
const limit = 64 * 1024 * 1024;
const totalLimit = 200 * 1024 * 1024;
const key = (surah: number) => new URL("/offline-tafsir/" + surah, location.origin).href;
const manifestKey = () => new URL("/offline-tafsir/complete", location.origin).href;

export function extraTafsir(id: string) { return EXTRA_TAFSIRS.find(item => item.id === id); }
export async function installedTafsirs(): Promise<Partial<Record<ExtraTafsirId, PackInfo>>> {
  const names = await caches.keys();
  const result: Partial<Record<ExtraTafsirId, PackInfo>> = {};
  for (const item of EXTRA_TAFSIRS) {
    if (!names.includes(prefix + item.id)) continue;
    const manifest = await (await caches.open(prefix + item.id)).match(manifestKey());
    if (manifest) result[item.id] = await manifest.json();
  }
  return result;
}
const chapters = new Map<string, Promise<TafsirSurah>>();
export async function removeTafsir(id: ExtraTafsirId) {
  for (const chapter of chapters.keys()) if (chapter.startsWith(id + ':')) chapters.delete(chapter);
  await caches.delete(prefix + id);
}
export function readDownloadedTafsir(id: ExtraTafsirId, surah: number): Promise<TafsirSurah> {
  const chapterKey = id + ':' + surah;
  const existing = chapters.get(chapterKey);
  if (existing) { chapters.delete(chapterKey); chapters.set(chapterKey, existing); return existing; }
  const request = (async () => {
    const cache = await caches.open(prefix + id);
    if (!await cache.match(manifestKey())) throw Error("Tafsir is not installed");
    const response = await cache.match(key(surah));
    if (!response) throw Error("Missing chapter");
    if (response.headers.get("x-quran-pack-encoding") !== "gzip") return response.json() as Promise<TafsirSurah>;
    if (!response.body) throw Error("Missing chapter");
    return readJson(new Response(response.body.pipeThrough(new DecompressionStream("gzip"))), 32 * 1024 * 1024) as Promise<TafsirSurah>;
  })().catch(error => { if (chapters.get(chapterKey) === request) chapters.delete(chapterKey); throw error; });
  chapters.set(chapterKey, request);
  while (chapters.size > 8) chapters.delete(chapters.keys().next().value!);
  return request;
}

/** Reject partial chapters, wrong resource IDs and duplicate/misassigned verses. */
export function normalizeTafsirChapter(rows: unknown[], resource: number, surah: number, count: number): TafsirSurah {
  const result: TafsirSurah = { lookup: {}, groups: {} };
  const textKeys = new Map<string, string>();
  for (const value of rows) {
    if (!value || typeof value !== "object") throw Error("Invalid tafsir");
    const row = value as { resource_id: number; verse_key: string; text: string };
    const match = /^(\d+):(\d+)$/.exec(row.verse_key);
    const ayah = match ? Number(match[2]) : 0;
    if (!match || Number(match[1]) !== surah || row.resource_id !== resource ||
        ayah < 1 || ayah > count || result.lookup[ayah] || typeof row.text !== "string" || !row.text.trim()) {
      throw Error("Invalid tafsir verse");
    }
    const groupKey = textKeys.get(row.text) ?? String(ayah);
    textKeys.set(row.text, groupKey);
    result.lookup[ayah] = groupKey;
    if (!result.groups[groupKey]) result.groups[groupKey] = { fromAyah: ayah, toAyah: ayah, ayahKeys: [], text: row.text };
    const group = result.groups[groupKey];
    group.ayahKeys.push(ayah); group.fromAyah = Math.min(group.fromAyah, ayah); group.toAyah = Math.max(group.toAyah, ayah);
  }
  if (Object.keys(result.lookup).length !== count) throw Error(`Incomplete tafsir chapter ${surah}: ${Object.keys(result.lookup).length}/${count}`);
  return result;
}

export async function fetchPage(url: string, signal: AbortSignal) {
  let response: Response | undefined;
  for (let attempt = 0; attempt < 3; attempt++) {
    signal.throwIfAborted();
    response = await fetch(url, { signal: AbortSignal.any([signal, AbortSignal.timeout(30000)]) });
    if (response.ok || ![429, 502, 503, 504].includes(response.status) || attempt === 2) break;
    const retryAfter = Number(response.headers.get("retry-after"));
    await response.body?.cancel();
    const delay = Number.isFinite(retryAfter) && retryAfter > 0 ? Math.min(10000, retryAfter * 1000) : 500 * 2 ** attempt;
    await new Promise<void>((resolve, reject) => {
      const abort = () => { clearTimeout(timer); reject(signal.reason); };
      const timer = setTimeout(() => { signal.removeEventListener("abort", abort); resolve(); }, delay);
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) { signal.removeEventListener("abort", abort); abort(); }
    });
  }
  if (!response) throw Error("Tafsir service unavailable");
  if (!response.ok || !response.body) throw Error(`Tafsir service unavailable: ${response.status} ${url}`);
  return readJson(response, 8 * 1024 * 1024);
}

async function readJson(response: Response, maximumBytes: number) {
  if (!response.body) throw Error("Missing response body");
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let text = "", bytes = 0;
  try {
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) break;
      bytes += chunk.value.byteLength;
      if (bytes > maximumBytes) throw Error("Response exceeds size limit");
      text += decoder.decode(chunk.value, { stream: true });
    }
    return JSON.parse(text + decoder.decode());
  } finally { await reader.cancel().catch(() => {}); }
}

export async function downloadTafsir(id: ExtraTafsirId, signal: AbortSignal,
  onProgress: (chapters: number, bytes: number) => void) {
  const item = extraTafsir(id);
  if (!item) throw Error("Unknown source");
  const installed = await installedTafsirs();
  if (installed[id]) return;
  const used = Object.values(installed).reduce((sum, info) => sum + info!.bytes, 0);
  const surahs: { id: number; versesCount: number }[] = await (await fetch("/data/qul/processed/surahs.json", { signal })).json();
  if (surahs.length !== 114) throw Error("Missing chapter index");
  await removeTafsir(id);
  const cache = await caches.open(prefix + id);
  let bytes = 0;
  try {
    for (const surah of surahs) {
      signal.throwIfAborted();
      const rows: unknown[] = [];
      let page = 1;
      while (true) {
        const data = await fetchPage(`https://api.quran.com/api/v4/tafsirs/${item.resource}/by_chapter/${surah.id}?per_page=50&page=${page}`, signal);
        if (!Array.isArray(data.tafsirs) || !data.pagination || data.pagination.current_page !== page) throw Error("Invalid pagination");
        rows.push(...data.tafsirs);
        if (rows.length > surah.versesCount) throw Error("Too many verses");
        const next = data.pagination.next_page;
        if (next === null) break;
        if (next !== page + 1 || next > 7) throw Error("Invalid pagination");
        page = next;
      }

      // Some chapter listings omit verses covered by a shared passage.
      // Resolve these through the publisher's ayah endpoint, never guess a mapping.
      const present = new Set(rows.map(row => (row as { verse_key?: string }).verse_key));
      for (let ayah = 1; ayah <= surah.versesCount; ayah++) {
        const verseKey = surah.id + ":" + ayah;
        if (present.has(verseKey)) continue;
        const data = await fetchPage(`https://api.quran.com/api/v4/tafsirs/${item.resource}/by_ayah/${verseKey}`, signal);
        const tafsir = data.tafsir;
        if (!tafsir || tafsir.resource_id !== item.resource || !tafsir.verses?.[verseKey]) throw Error("Missing verse mapping " + verseKey);
        
        for (const mappedKey of Object.keys(tafsir.verses)) {
          const [mappedSurah, mappedAyah] = mappedKey.split(":").map(Number);
          if (mappedSurah !== surah.id || !Number.isInteger(mappedAyah) || mappedAyah < 1 || mappedAyah > surah.versesCount || present.has(mappedKey)) continue;
          rows.push({ resource_id: item.resource, verse_key: mappedKey, text: tafsir.text });
          present.add(mappedKey);
        }
      }
      const chapter = normalizeTafsirChapter(rows, item.resource, surah.id, surah.versesCount);
      const body = JSON.stringify(chapter);
      const raw = new TextEncoder().encode(body);
      if (raw.byteLength > 32 * 1024 * 1024) throw Error("Chapter exceeds size limit");
      // Native streaming gzip needs no library and keeps large books economical.
      const compressed = typeof CompressionStream !== "undefined";
      const packed = compressed ? await new Response(new Blob([raw]).stream().pipeThrough(new CompressionStream("gzip"))).arrayBuffer() : raw;
      bytes += packed.byteLength;
      if (bytes > limit || bytes + used > totalLimit) throw Error("Tafsir storage limit reached");
      await cache.put(key(surah.id), new Response(packed, { headers: { "content-type": compressed ? "application/gzip" : "application/json", "x-quran-pack-encoding": compressed ? "gzip" : "identity" } }));
      onProgress(surah.id, bytes);
    }
    signal.throwIfAborted();
    await cache.put(manifestKey(), new Response(JSON.stringify({ bytes, downloadedAt: new Date().toISOString() })));
  } catch (error) {
    await removeTafsir(id);
    throw error;
  }
}




