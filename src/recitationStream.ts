import { isErrorContentType, completeMp3 } from './audioFormat.ts';
import type { AudioPoint, AudioSegment } from './recitationPlan';

type TimedSegment = { segment: AudioSegment; start: number; end: number; bytes: number };
export type StreamStatus = 'ready' | 'buffering' | 'retrying' | 'error' | 'finished';
const MAX_BYTES = 8 * 1024 * 1024;
const MAX_AUTOMATIC_RETRIES = 4;
/** One media timeline: no element/source switch at verse boundaries. MP3 sequence
 * mode preserves the recordings, including natural pauses, without crossfading. */
export class RecitationStream {
  private media: HTMLAudioElement;
  private plan: AudioSegment[];
  private onSegment: (segment: AudioSegment) => void;
  private onStatus: (status: StreamStatus) => void;
  private source = new MediaSource();
  private buffer?: SourceBuffer;
  private objectUrl = '';
  private abort = new AbortController();
  private timed: TimedSegment[] = [];
  private cursor = 0;
  private notified?: AudioSegment;
  private busy = false;
  private wanted = false;
  private retryAt = 0;
  private failures = 0;
  private exhausted = false;
  private timer = 0;
  private stopped = false;
  private status: StreamStatus = 'ready';
  private cachedClips = new Set<string>();
  private pending?: Uint8Array<ArrayBuffer>;
  private end?: AudioPoint;
  private completed = false;
  constructor(media: HTMLAudioElement, plan: AudioSegment[], onSegment: (s: AudioSegment) => void, onStatus: (s: StreamStatus) => void) {
    this.media = media; this.plan = [...plan]; this.end = plan.at(-1); this.onSegment = onSegment; this.onStatus = onStatus;
    this.objectUrl = URL.createObjectURL(this.source);
    this.source.addEventListener('sourceopen', () => {
      if (this.stopped || this.buffer) return;
      try { this.buffer = this.source.addSourceBuffer('audio/mpeg'); this.buffer.mode = 'sequence'; void this.pump(); }
      catch { this.wanted = false; this.exhausted = true; this.stopWatchdog(); this.setStatus('error'); }
    });
    media.src = this.objectUrl;
    media.addEventListener('timeupdate', this.tick);
    media.addEventListener('play', this.mediaPlayed);
    media.addEventListener('pause', this.mediaPaused);
    media.addEventListener('ended', this.ended);
    media.addEventListener('error', this.failed);
    if (typeof window !== 'undefined') window.addEventListener?.('online', this.online);
  }
  static supported() { return typeof MediaSource !== 'undefined' && MediaSource.isTypeSupported('audio/mpeg'); }
  get bufferedSeconds() { return Math.max(0, (this.timed.at(-1)?.end ?? 0) - this.media.currentTime); }
  get duration() { return this.cursor >= this.planLength ? this.boundary?.end ?? 0 : 0; }
  private withinEnd(point: AudioPoint) { return !this.end || point.surah < this.end.surah || point.surah === this.end.surah && point.ayah <= this.end.ayah; }
  private get boundary() { for (let index = this.timed.length - 1; index >= 0; index--) if (this.withinEnd(this.timed[index].segment)) return this.timed[index]; }
  private get planLength() { const index = this.plan.findIndex(point => !this.withinEnd(point)); return index < 0 ? this.plan.length : index; }
  /** Change the future boundary without reloading the current audio timeline. */
  setEnd(end: AudioPoint) {
    if (this.stopped || !Number.isInteger(end.surah) || end.surah < 1 || end.surah > 114 || !Number.isInteger(end.ayah) || end.ayah < 1) return;
    this.end = { ...end }; if (this.cursor >= this.planLength) this.pending = undefined;
    if (this.wanted && !this.completed) void this.pump();
  }
  /** Append future chapters only. Already buffered clips and position are preserved. */
  extendPlan(segments: AudioSegment[]) {
    if (this.stopped || this.completed) return;
    const last = this.plan.at(-1);
    this.plan.push(...segments.filter(point => !last || point.surah > last.surah || point.surah === last.surah && point.ayah > last.ayah));
  }
  private setStatus(status: StreamStatus) { if (!this.stopped && this.status !== status) { this.status = status; this.onStatus(status); } }
  private startWatchdog() {
    if (!this.timer) this.timer = window.setInterval(this.tick, 1000);
  }
  private stopWatchdog() {
    if (this.timer) window.clearInterval(this.timer);
    this.timer = 0;
  }
  private mediaPlayed = () => {
    if (this.stopped || this.completed || this.wanted) return;
    if (this.exhausted) { this.failures = 0; this.exhausted = false; }
    this.wanted = true; this.retryAt = 0; this.startWatchdog(); void this.pump();
  };
  private mediaPaused = () => {
    // Native/system media controls can pause the element without calling our
    // public pause(). Ignore a queued pause event from an older play request.
    if (!this.stopped && this.media.paused) { this.wanted = false; this.stopWatchdog(); }
  };
  play() {
    if (this.stopped || this.completed) return;
    // A user action starts a fresh finite recovery budget. Background polling
    // cannot reset this budget or silently skip the recording that failed.
    if (this.exhausted) { this.failures = 0; this.exhausted = false; }
    this.wanted = true; this.retryAt = 0;
    this.startWatchdog();
    void this.pump();
    void this.media.play().catch(error => {
      if (!this.stopped && this.wanted && error.name !== 'AbortError') {
        this.wanted = false; this.stopWatchdog(); this.setStatus('error');
      }
    });
  }
  /** Retry a network/clip failure on the existing timeline and exact offset.
   * A terminal decoder failure needs the caller's existing rebuild path. */
  retryPending() {
    if (this.stopped || this.completed || !this.buffer || this.media.error) return false;
    this.failures = 0; this.exhausted = false; this.retryAt = 0;
    this.setStatus('buffering'); this.play(); return true;
  }
  pause() { this.wanted = false; this.stopWatchdog(); this.media.pause(); }
  private finish() { if (this.completed) return; this.completed = true; this.wanted = false; this.pending = undefined; this.stopWatchdog(); this.media.pause(); this.setStatus('finished'); }
  private ended = () => { if (this.cursor >= this.planLength) this.finish(); };
  private failed = () => { this.wanted = false; this.stopWatchdog(); void this.invalidateBufferedCache(); this.pending = undefined; this.setStatus('error'); };
  private online = () => {
    if (!this.stopped && !this.completed && this.wanted && !this.exhausted && this.status === 'retrying') {
      this.retryAt = 0; void this.pump();
    }
  };
  private tick = () => {
    if (this.stopped || this.completed) return;
    const position = this.media.currentTime;
    const boundary = this.boundary;
    if (this.end && boundary && boundary.segment.surah === this.end.surah && boundary.segment.ayah === this.end.ayah && position >= boundary.end) {
      this.finish(); return;
    }
    // Never choose a future preloaded verse when a timestamp falls in a gap.
    // Keep the last verse that has actually started until the next one starts.
    let active: TimedSegment | undefined;
    for (let i = this.timed.length - 1; i >= 0; i--) {
      if (position >= this.timed[i].start) { active = this.timed[i]; break; }
    }
    if (active && active.segment !== this.notified) { this.notified = active.segment; this.onSegment(active.segment); }
    if (this.wanted && this.bufferedSeconds < 0.15 && this.cursor < this.planLength && this.status !== 'retrying') this.setStatus('buffering');
    if (this.wanted && this.bufferedSeconds < 25 * this.media.playbackRate) void this.pump();
  };
  private operation(action: () => void) {
    return new Promise<void>((resolve, reject) => {
      const buffer = this.buffer!;
      const done = () => { clean(); resolve(); };
      const fail = () => { clean(); reject(new Error('Audio buffer failed')); };
      const clean = () => { buffer.removeEventListener('updateend', done); buffer.removeEventListener('error', fail); this.abort.signal.removeEventListener('abort', fail); };
      buffer.addEventListener('updateend', done, {once:true}); buffer.addEventListener('error', fail, {once:true}); this.abort.signal.addEventListener('abort', fail, {once:true});
      try { action(); } catch(error) { clean(); reject(error); }
    });
  }
  private async fetchClip(url: string, allowCache = true): Promise<Uint8Array<ArrayBuffer>> {
    const signal = AbortSignal.any([this.abort.signal, AbortSignal.timeout(20000)]);
    let response: Response | undefined;
    try { if (allowCache) response = await (await caches.open('quran-audio-v1')).match(url); } catch { /* Playback does not require persistent storage. */ }
    const cached = Boolean(response);
    response ??= await fetch(url, {signal});
    if (!response.ok || !response.body) throw new Error('Audio download failed');
    if (signal.aborted) { void response.body.cancel().catch(() => {}); signal.throwIfAborted(); }
    const declaredLength = Number(response.headers.get('content-length'));
    const expectedLength = declaredLength > 0 && Number.isSafeInteger(declaredLength) && !response.headers.get('content-encoding') ? declaredLength : 0;
    // Known-length clips need only their final bounded array. Keeping every
    // chunk as well would duplicate the complete recording before append.
    const allocated = expectedLength && expectedLength <= MAX_BYTES ? new Uint8Array(expectedLength) : undefined;
    const reader = response.body.getReader(); const chunks: Uint8Array<ArrayBuffer>[] = []; let size = 0, readDone = false;
    const cancel = () => { void reader.cancel().catch(() => {}); };
    signal.addEventListener('abort', cancel, {once:true});
    try {
      while (true) {
        signal.throwIfAborted(); const {value,done} = await reader.read(); if (done) { readDone = true; break; }
        size += value.byteLength; if (size > MAX_BYTES) { await reader.cancel(); throw new Error('Audio clip too large'); }
        if (allocated) {
          if (size > allocated.length) { await reader.cancel(); break; }
          allocated.set(value, size - value.byteLength);
        } else chunks.push(value);
      }
      signal.throwIfAborted();
    } finally { signal.removeEventListener('abort', cancel); if (!readDone) cancel(); reader.releaseLock(); }
    if (!size) throw new Error('Empty audio clip');
    const single = chunks.length === 1 && chunks[0].byteLength === chunks[0].buffer.byteLength ? chunks[0] : undefined;
    const result = allocated ?? single ?? new Uint8Array(size);
    if (!allocated && !single) { let offset = 0; for(const chunk of chunks) { result.set(chunk,offset); offset += chunk.length; } }
    const wrongLength = expectedLength && size !== expectedLength;
    if (wrongLength || isErrorContentType(response.headers.get("content-type")) || !completeMp3(result)) {
      if (cached) {
        await this.invalidateClip(url);
        return this.fetchClip(url, false);
      }
      throw new Error("Incomplete or invalid audio clip");
    }
    if (cached) this.cachedClips.add(url);
    return result;
  }
  private async invalidateBufferedCache() {
    await Promise.all([...this.cachedClips].map(url => this.invalidateClip(url)));
    this.cachedClips.clear();
  }
  private async invalidateClip(url: string) {
    try { await (await caches.open('quran-audio-v1')).delete(url); } catch { /* Retry can still stream without storage. */ }
  }
  private async evictPlayed() {
    if (!this.buffer || this.stopped) return;
    // Keep the current verse and a small rewind window; discard older media.
    const keep = this.timed.find(item => item.end > this.media.currentTime - 10)?.start ?? 0;
    if (keep > 0 && this.buffer.buffered.length && this.buffer.buffered.start(0) + .1 < keep) {
      await this.operation(() => this.buffer!.remove(0, keep));
      this.timed = this.timed.filter(item => item.end > keep);
      const retainedUrls = new Set(this.timed.map(item => item.segment.url));
      for (const url of this.cachedClips) if (!retainedUrls.has(url)) this.cachedClips.delete(url);
    }

  }
  private async pump() {
    if (this.busy || this.stopped || this.completed || !this.wanted || !this.buffer || !['open', 'ended'].includes(this.source.readyState) || Date.now() < this.retryAt) return;
    this.busy = true;
    try {
      await this.evictPlayed();
      while (!this.stopped && !this.completed && this.wanted && this.cursor < this.planLength && this.bufferedSeconds < 60 * this.media.playbackRate) {
        await this.evictPlayed();
        if (this.stopped || this.completed || !this.wanted || this.cursor >= this.planLength) return;
        const segment = this.plan[this.cursor];
        const bytes = this.pending ?? await this.fetchClip(segment.url);
        // A read can finish after disposal/completion. Do not retain its bytes
        // again after dispose() has already cleared the pending clip.
        if (this.stopped || this.completed || this.cursor >= this.planLength) return;
        this.pending = bytes;
        if (!this.wanted) return;
        if (this.timed.reduce((n,item)=>n+item.bytes,0) + bytes.length > MAX_BYTES && this.bufferedSeconds > 1) break;
        const start = this.timed.at(-1)?.end ?? 0;
        await this.operation(() => this.buffer!.appendBuffer(bytes));
        if (this.stopped || this.completed) return;
        const end = this.buffer.buffered.end(this.buffer.buffered.length-1);
        if (end <= start) throw new Error('Invalid audio duration');
        this.timed.push({segment,start,end,bytes:bytes.length}); this.pending = undefined; this.cursor++;
        this.failures = 0; this.setStatus('ready'); this.tick();
      }
      if (this.cursor >= this.planLength && !this.stopped && this.source.readyState === 'open') this.source.endOfStream();
    } catch (error) {
      if (this.stopped || this.completed) return;
      if (this.media.error && this.plan[this.cursor]) {
        await this.invalidateBufferedCache(); await this.invalidateClip(this.plan[this.cursor].url); this.pending = undefined;
      }
      if (!this.stopped && !this.completed) {
        this.failures++;
        if (this.media.error || error instanceof Error && error.message === 'Audio clip too large' || this.failures > MAX_AUTOMATIC_RETRIES) {
          this.wanted = false; this.exhausted = true; this.stopWatchdog(); this.setStatus('error');
          // Buffered recitation may still contain the current verse. Let it
          // finish naturally rather than cutting it off for a future failure.
          return;
        }
        this.retryAt = Date.now() + Math.min(10000, 1000 * 2 ** Math.min(this.failures,3));
        this.setStatus('retrying');
      }
    } finally { this.busy = false; }
  }
  dispose() {
    if (this.stopped) return;
    this.stopped = true; this.wanted = false; this.abort.abort(); this.stopWatchdog();
    if (typeof window !== 'undefined') window.removeEventListener?.('online', this.online);
    this.media.removeEventListener('timeupdate', this.tick); this.media.removeEventListener('ended', this.ended); this.media.removeEventListener('error', this.failed);
    this.media.removeEventListener('play', this.mediaPlayed); this.media.removeEventListener('pause', this.mediaPaused);
    this.media.pause(); this.media.removeAttribute('src'); this.media.load(); URL.revokeObjectURL(this.objectUrl); this.pending = undefined; this.timed = [];
    this.plan = []; this.cachedClips.clear();
  }
}
