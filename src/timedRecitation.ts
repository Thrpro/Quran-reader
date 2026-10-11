import { loadTimings, timedAudioUrl, timingAt, type AyahTiming } from './timedAudio.ts';
import { validAudioResponse } from './audioFormat.ts';
import type { StreamStatus } from './recitationStream';
import type { AudioSegment } from './recitationPlan';

/** One publisher recording per chapter. Timings only move the highlight; they
 * never splice audio or change the source between verses. */
export class TimedRecitation {
  private abort = new AbortController();
  private timings: AyahTiming[] = [];
  private chapter: number;
  private firstAyah: number;
  private wanted = false;
  private ready = false;
  private stopped = false;
  private objectUrl = '';
  private lastAyah = -1;
  private media: HTMLAudioElement;
  private reciter: number;
  private end: {surah: number; ayah: number};
  private surahs: {id: number; versesCount: number}[];
  private speed: () => number;
  private onSegment: (segment: AudioSegment) => void;
  private onStatus: (status: StreamStatus) => void;
  constructor(media: HTMLAudioElement, reciter: number,
    start: { surah: number; ayah: number }, end: { surah: number; ayah: number },
    surahs: { id: number; versesCount: number }[], speed: () => number,
    onSegment: (segment: AudioSegment) => void, onStatus: (status: StreamStatus) => void) {
    this.media=media; this.reciter=reciter; this.end=end; this.surahs=surahs; this.speed=speed; this.onSegment=onSegment; this.onStatus=onStatus;
    this.chapter = start.surah; this.firstAyah = start.ayah;
    media.addEventListener('loadedmetadata', this.loaded);
    media.addEventListener('timeupdate', this.tick);
    media.addEventListener('ended', this.ended);
    void this.prepare();
  }
  get duration() { return Number.isFinite(this.media.duration) ? this.media.duration : 0; }
  get seekStart() { return this.firstAyah === 1 ? 0 : this.timings[this.firstAyah - 1]?.start ?? 0; }
  get seekEnd() { return this.chapter === this.end.surah && this.end.ayah < this.timings.length ? this.timings[this.end.ayah - 1]?.end ?? 0 : this.duration; }
  get bufferedSeconds() { const b = this.media.buffered; return b.length ? Math.max(0, b.end(b.length - 1) - this.media.currentTime) : 0; }
  private async prepare() {
    this.ready = false; this.lastAyah = -1; this.onStatus('buffering');
    try {
      const count = this.surahs.find(s => s.id === this.chapter)?.versesCount;
      if (!count) throw Error('Missing chapter');
      this.timings = await loadTimings(this.reciter, this.chapter, count, this.abort.signal);
      const url = timedAudioUrl(this.reciter, this.chapter);
      let response: Response | undefined;
      try { response = await (await caches.open('quran-audio-v1')).match(url); } catch { /* Streaming needs no cache. */ }
      if (response && !await validAudioResponse(response.clone())) { await (await caches.open('quran-audio-v1')).delete(url); response = undefined; }
      if (this.stopped) return;
      if (this.objectUrl) URL.revokeObjectURL(this.objectUrl);
      this.objectUrl = response ? URL.createObjectURL(await response.blob()) : '';
      if (this.stopped) { if (this.objectUrl) URL.revokeObjectURL(this.objectUrl); return; }
      this.media.src = this.objectUrl || url; this.media.defaultPlaybackRate = this.speed(); this.media.playbackRate = this.speed(); this.media.load();
    } catch { if (!this.stopped) this.onStatus('error'); }
  }
  private loaded = () => {
    if (this.stopped) return;
    if (!this.duration || (this.timings.at(-1)?.end ?? Infinity) > this.duration + .25) { this.media.pause(); this.onStatus('error'); return; }
    this.media.currentTime = this.seekStart; this.ready = true; this.onStatus('ready'); this.tick();
    if (this.wanted) this.play();
  };
  private tick = () => {
    if (this.stopped || !this.ready) return;
    if (this.chapter === this.end.surah && this.end.ayah < this.timings.length && this.media.currentTime >= this.seekEnd) {
      // pause() can enqueue another timeupdate at the same boundary. Finish
      // exactly once so repeat playback cannot restart competing timelines.
      this.ready = false;
      this.pause(); this.onStatus('finished'); return;
    }
    const row = timingAt(this.timings, this.media.currentTime);
    const ayah = row?.ayah ?? 0;
    if (ayah !== this.lastAyah) { this.lastAyah = ayah; this.onSegment({ surah: this.chapter, ayah: Math.max(1, ayah), basmala: ayah === 0, url: timedAudioUrl(this.reciter, this.chapter) }); }
    // A whole chapter finishes on the native ended event, including its tail.

  };
  private ended = () => {
    if (this.stopped || !this.ready) return;
    if (this.chapter >= this.end.surah) { this.ready = false; this.wanted = false; this.onStatus('finished'); return; }
    this.chapter++; this.firstAyah = 1; void this.prepare();
  };
  play() { this.wanted = true; if (this.ready) { this.media.defaultPlaybackRate = this.speed(); this.media.playbackRate = this.speed();
    void this.media.play().catch(error => { if (!this.stopped && this.wanted && error.name !== 'AbortError') this.onStatus('error'); }); } }
  pause() { this.wanted = false; this.media.pause(); }
  dispose() { this.stopped = true; this.abort.abort(); this.media.pause();
    this.media.removeEventListener('loadedmetadata', this.loaded); this.media.removeEventListener('timeupdate', this.tick); this.media.removeEventListener('ended', this.ended);
    this.media.removeAttribute('src'); this.media.load(); if (this.objectUrl) URL.revokeObjectURL(this.objectUrl); }
}
