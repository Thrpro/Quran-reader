import type { ListeningPreferences, ListeningResume } from './listeningPreferences';

/** Identity of the performance currently attached to the audio element. */
export type ListeningResumeSource = Pick<ListeningResume,
  'mode' | 'ayahReciterId' | 'reciterId' | 'recordingId' | 'surah' | 'ayah'> & { timedRecording: boolean };

/** Selection state may already refer to the next performance when teardown
 * emits pause/timeupdate. Never attach the previous source's offset to it. */
export function resumeForSource(selection: ListeningPreferences & ListeningResumeSource,
  source: ListeningResumeSource | null, seconds: number): ListeningResume | null {
  if (!source || source.mode !== selection.mode || source.timedRecording !== selection.timedRecording ||
      source.surah !== selection.surah || source.ayah !== selection.ayah ||
      (source.mode === 'ayah' ? source.ayahReciterId !== selection.ayahReciterId
        : source.reciterId !== selection.reciterId || source.recordingId !== selection.recordingId) ||
      !Number.isFinite(seconds) || seconds < 0 || seconds > 86400) return null;
  return {
    mode: source.mode, ayahReciterId: selection.ayahReciterId,
    reciterId: selection.reciterId, recordingId: selection.recordingId,
    speed: selection.speed, volume: selection.volume, continuous: selection.continuous,
    surah: source.surah, ayah: source.ayah,
    seconds: source.mode === 'ayah' && !source.timedRecording ? 0 : seconds,
  };
}
