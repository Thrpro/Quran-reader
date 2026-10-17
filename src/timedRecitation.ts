import { loadTimings, timedAudioUrl, timingAt, type AyahTiming } from './timedAudio.ts';
import { validAudioResponse } from './audioFormat.ts';
import type { StreamStatus } from './recitationStream';
import type { AudioPoint, AudioSegment } from './recitationPlan';

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
  private completed = false;
  private preparing = false;
  private objectUrl = '';
  private lastAyah = -1;
  private retryTimer?: ReturnType<typeof setTimeout>;
  private stallTimer?: ReturnType<typeof setTimeout>;
  private retryAttempts = 0;
  private resumeAt: number | undefined;
  private recoveredAt = 0;
  private knownDuration = 0;
  private sourceChapter = 0;
  private media: HTMLAudioElement;
  private reciter: number;
  private end: AudioPoint;
  private surahs: {id: number; versesCount: number}[];
  private speed: () => number;
  private onSegment: (segment: AudioSegment) => void;
  private onStatus: (status: StreamStatus) => void;
  constructor(media: HTMLAudioElement, reciter: number,
    start: AudioPoint, end: AudioPoint,
    surahs: { id: number; versesCount: number }[], speed: () => number,
    onSegment: (segment: AudioSegment) => void, onStatus: (status: StreamStatus) => void) {
    this.media=media; this.reciter=reciter; this.end=end; this.surahs=surahs; this.speed=speed; this.onSegment=onSegment; this.onStatus=onStatus;
    this.chapter = start.surah; this.firstAyah = start.ayah;
    media.addEventListener('loadedmetadata', this.loaded);
    media.addEventListener('timeupdate', this.tick);
    media.addEventListener('ended', this.ended);
    media.addEventListener('waiting', this.stalled);
    media.addEventListener('stalled', this.stalled);
    media.addEventListener('playing', this.playing);
    media.addEventListener('error', this.failed);
    if (typeof window !== 'undefined') window.addEventListener('online', this.online);
    void this.prepare();
  }
  get duration() { return Number.isFinite(this.media.duration) ? this.media.duration : 0; }
  get seekStart() { return this.firstAyah === 1 ? 0 : this.timings[this.firstAyah - 1]?.start ?? 0; }
  get seekEnd() { return this.chapter === this.end.surah && this.end.ayah < this.timings.length ? this.timings[this.end.ayah - 1]?.end ?? 0 : this.duration; }
  get bufferedSeconds() { const b = this.media.buffered; return b.length ? Math.max(0, b.end(b.length - 1) - this.media.currentTime) : 0; }
  /** Continuation preferences take effect at their boundary, never by restarting an ayah. */
  setEnd(end: AudioPoint) {
    if (this.stopped || !Number.isInteger(end.surah) || end.surah < this.chapter || end.surah > 114 || !Number.isInteger(end.ayah) || end.ayah < 1) return;
    this.end = { ...end };
  }
  private clearTimers() { clearTimeout(this.retryTimer); clearTimeout(this.stallTimer); this.retryTimer = undefined; this.stallTimer = undefined; }
  private async prepare() {
    if (this.preparing || this.stopped || this.completed) return;
    this.preparing = true; this.ready = false; this.onStatus('buffering');
    try {
      const count = this.surahs.find(s => s.id === this.chapter)?.versesCount;
      if (!count) throw Error('Missing chapter');
      this.timings = await loadTimings(this.reciter, this.chapter, count, this.abort.signal);
      const url = timedAudioUrl(this.reciter, this.chapter);
      let response: Response | undefined;
      try { response = await (await caches.open('quran-audio-v1')).match(url); } catch { /* Streaming needs no cache. */ }
      if (response && !await validAudioResponse(response.clone())) { await (await caches.open('quran-audio-v1')).delete(url); response = undefined; }
      if (this.stopped) return;
      const objectUrl = response ? URL.createObjectURL(await response.blob()) : '';
      if (this.stopped) { if (objectUrl) URL.revokeObjectURL(objectUrl); return; }
      if (this.objectUrl) URL.revokeObjectURL(this.objectUrl); this.objectUrl = objectUrl;
      this.media.src = objectUrl || url;
      this.sourceChapter = this.chapter;
      this.media.defaultPlaybackRate = this.speed(); this.media.playbackRate = this.speed(); this.media.load();
      // A source that never reports metadata also has a bounded recovery path.
      if (this.wanted) this.watchStall();
    } catch { if (!this.stopped) this.retry(); }
    finally { this.preparing = false; }
  }
  private loaded = () => {
    if (this.stopped || this.completed) return;
    if (!this.duration || (this.timings.at(-1)?.end ?? Infinity) > this.duration + .25 || this.duration + .25 < this.knownDuration) {
      this.media.pause(); this.retry(); return;
    }
    this.knownDuration = this.duration;
    this.media.currentTime = Math.min(this.seekEnd, Math.max(this.seekStart, this.resumeAt ?? this.seekStart));
    clearTimeout(this.stallTimer); this.stallTimer = undefined;
    this.resumeAt = undefined; this.recoveredAt = this.media.currentTime;
    this.media.defaultPlaybackRate = this.speed(); this.media.playbackRate = this.speed();
    this.ready = true; this.onStatus('ready'); this.tick();
    if (this.wanted) this.startMedia();
  };
  private tick = () => {
    if (this.stopped || !this.ready || this.completed) return;
    if (this.chapter === this.end.surah && this.end.ayah < this.timings.length && this.media.currentTime >= this.seekEnd) {
      this.finish(); return;
    }
    if (this.media.currentTime > this.recoveredAt + 5) this.retryAttempts = 0;
    const row = timingAt(this.timings, this.media.currentTime);
    const ayah = row?.ayah ?? 0;
    if (ayah !== this.lastAyah) { this.lastAyah = ayah; this.onSegment({ surah: this.chapter, ayah: Math.max(1, ayah), basmala: ayah === 0, url: timedAudioUrl(this.reciter, this.chapter) }); }
    // A whole chapter finishes on the native ended event, including its tail.
  };
  private finish() { this.completed = true; this.ready = false; this.wanted = false; this.clearTimers(); this.media.pause(); this.onStatus('finished'); }
  private ended = () => {
    if (this.stopped || !this.ready || this.completed) return;
    // A prematurely closed network response must retry this recording/position,
    // not treat the current chapter as complete and skip to the next chapter.
    const expectedEnd = this.chapter === this.end.surah && this.end.ayah < this.timings.length
      ? this.seekEnd : Math.max(this.knownDuration, this.timings.at(-1)?.end ?? 0, this.duration);
    if (this.media.currentTime + .25 < expectedEnd) { this.retry(); return; }
    if (this.chapter >= this.end.surah) { this.finish(); return; }
    this.clearTimers(); this.chapter++; this.firstAyah = 1; this.lastAyah = -1;
    this.resumeAt = undefined; this.knownDuration = 0; this.retryAttempts = 0; void this.prepare();
  };
  private watchStall() {
    if (this.stallTimer || this.stopped || !this.wanted || this.completed) return;
    const position = this.media.currentTime;
    this.stallTimer = setTimeout(() => {
      this.stallTimer = undefined;
      if (!this.stopped && this.wanted && this.media.currentTime <= position + .05) this.retry();
    }, 8000);
  }
  private stalled = () => { if (this.wanted && !this.stopped && !this.completed) { this.onStatus('buffering'); this.watchStall(); } };
  private playing = () => { clearTimeout(this.stallTimer); this.stallTimer = undefined; if (this.wanted && !this.stopped && !this.completed) this.onStatus('ready'); };
  private failed = () => { if (!this.stopped && !this.completed) this.retry(); };
  private online = () => { if (this.wanted && !this.stopped && !this.completed && !this.ready && this.retryAttempts < 4) { clearTimeout(this.retryTimer); this.retryTimer = undefined; void this.prepare(); } };
  private retry() {
    if (this.stopped || this.completed || this.retryTimer) return;
    this.resumeAt ??= this.sourceChapter === this.chapter && Number.isFinite(this.media.currentTime) ? this.media.currentTime : this.seekStart;
    this.media.pause();
    this.ready = false; clearTimeout(this.stallTimer); this.stallTimer = undefined;
    if (!this.wanted) { this.onStatus('error'); return; }
    if (this.retryAttempts >= 4) { this.onStatus('error'); return; }
    this.onStatus('retrying');
    if (typeof navigator !== 'undefined' && navigator.onLine === false) return;
    const delay = 1000 * 2 ** this.retryAttempts++;
    this.retryTimer = setTimeout(() => { this.retryTimer = undefined; if (this.wanted && !this.stopped) void this.prepare(); }, delay);
  }
  private startMedia() {
    this.media.defaultPlaybackRate = this.speed(); this.media.playbackRate = this.speed();
    this.watchStall();
    void this.media.play().catch(error => { if (!this.stopped && this.wanted && error.name !== 'AbortError') this.retry(); });
  }
  play() {
    if (this.stopped || this.completed) return;
    this.wanted = true;
    if (this.ready) this.startMedia();
    else if (!this.preparing && !this.retryTimer) { this.retryAttempts = 0; void this.prepare(); }
  }
  pause() { this.wanted = false; this.clearTimers(); this.media.pause(); }
  dispose() {
    this.stopped = true; this.abort.abort(); this.clearTimers(); this.media.pause();
    this.media.removeEventListener('loadedmetadata', this.loaded); this.media.removeEventListener('timeupdate', this.tick); this.media.removeEventListener('ended', this.ended);
    this.media.removeEventListener('waiting', this.stalled); this.media.removeEventListener('stalled', this.stalled); this.media.removeEventListener('playing', this.playing); this.media.removeEventListener('error', this.failed);
    if (typeof window !== 'undefined') window.removeEventListener('online', this.online);
    this.media.removeAttribute('src'); this.media.load(); if (this.objectUrl) URL.revokeObjectURL(this.objectUrl);
  }
}
