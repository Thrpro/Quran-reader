import { ayahAudioUrl, type AyahReciter } from './audioCatalog.ts';
export type AudioPoint = { surah: number; ayah: number };
export type AudioSegment = AudioPoint & { url: string; basmala?: boolean };
export function recitationPlan(reciter: AyahReciter, start: AudioPoint, end: AudioPoint, surahs: {id:number;versesCount:number}[]): AudioSegment[] {
  const result: AudioSegment[] = [];
  for (let surah = start.surah; surah <= end.surah; surah++) {
    const count = surahs.find(s => s.id === surah)?.versesCount ?? 0;
    const first = surah === start.surah ? start.ayah : 1;
    const last = surah === end.surah ? Math.min(count, end.ayah) : count;
    if (first > last) continue;
    // Al-Fatihah already starts with 1:1; At-Tawbah has no opening basmala.
    if (first === 1 && surah !== 1 && surah !== 9) result.push({ surah, ayah: 1, basmala: true, url: ayahAudioUrl(reciter, 1, 1) });
    for (let ayah = first; ayah <= last; ayah++) result.push({surah, ayah, url: ayahAudioUrl(reciter, surah, ayah)});
  }
  return result;
}
