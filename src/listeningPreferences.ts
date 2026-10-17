export const LISTENING_PREFERENCES_KEY = "quran-listening-preferences-v1";
export const LISTENING_RESUME_KEY = "quran-listening-resume-v1";
export const PLAYBACK_SPEEDS = [.5, .75, 1, 1.25, 1.5, 1.75, 2] as const;
export type ListeningPreferences = {
  mode: "ayah" | "surah"; ayahReciterId: number; reciterId: number; recordingId: number;
  speed: number; volume: number; continuous: boolean;
};
export type ListeningResume = ListeningPreferences & { surah: number; ayah: number; seconds: number };
const defaults: ListeningPreferences = { mode: "ayah", ayahReciterId: 7, reciterId: 0, recordingId: 0,
  speed: 1, volume: 1, continuous: false };
const positiveId = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) > 0;
export function parseListeningPreferences(raw: string | null): ListeningPreferences {
  try {
    const value = JSON.parse(raw ?? "null");
    return {
      mode: value?.mode === "surah" ? "surah" : "ayah",
      ayahReciterId: positiveId(value?.ayahReciterId) ? value.ayahReciterId : defaults.ayahReciterId,
      reciterId: positiveId(value?.reciterId) ? value.reciterId : 0,
      recordingId: positiveId(value?.recordingId) ? value.recordingId : 0,
      speed: PLAYBACK_SPEEDS.includes(value?.speed) ? value.speed : defaults.speed,
      volume: typeof value?.volume === "number" && Number.isFinite(value.volume) && value.volume >= 0 && value.volume <= 1 ? value.volume : defaults.volume,
      continuous: typeof value?.continuous === "boolean" ? value.continuous : false,
    };
  } catch { return { ...defaults }; }
}
export function parseListeningResume(raw: string | null): ListeningResume | null {
  try {
    const value = JSON.parse(raw ?? "null");
    if (!Number.isInteger(value?.surah) || value.surah < 1 || value.surah > 114 ||
        !Number.isInteger(value.ayah) || value.ayah < 1 || value.ayah > 286 ||
        !Number.isFinite(value.seconds) || value.seconds < 0 || value.seconds > 86400) return null;
    return { ...parseListeningPreferences(raw), surah: value.surah, ayah: value.ayah, seconds: value.seconds };
  } catch { return null; }
}
export function storedListeningPreferences(): ListeningPreferences {
  try { return parseListeningPreferences(localStorage.getItem(LISTENING_PREFERENCES_KEY)); }
  catch { return { ...defaults }; }
}
export function storedListeningResume(): ListeningResume | null {
  try { return parseListeningResume(localStorage.getItem(LISTENING_RESUME_KEY)); }
  catch { return null; }
}
