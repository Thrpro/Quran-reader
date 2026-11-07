export type AudioCacheSnapshot = { urls: string[]; bytes: number };

/** Session metadata only. Cache Storage remains the source of truth; bodies
 * are never read to list downloads. Reconcile keys to detect eviction and cap
 * concurrent header reads, even for large verse-by-verse libraries. */
export class AudioCacheIndex {
  private sizes = new Map<string, number>();
  private pending: Promise<AudioCacheSnapshot> | null = null;
  private pendingRevision = 0;
  private revision = 0;
  private readonly concurrency: number;
  constructor(concurrency = 4) {
    if (!Number.isInteger(concurrency) || concurrency < 1) throw new RangeError('Invalid cache metadata concurrency');
    this.concurrency = concurrency;
  }
  forget(url?: string) {
    if (url === undefined) this.sizes.clear(); else this.sizes.delete(url);
    this.revision++;
  }
  remember(url: string, bytes: number) {
    if (Number.isSafeInteger(bytes) && bytes >= 0) { this.sizes.set(url, bytes); this.revision++; }
  }
  async read(cache: Cache, freshSizes = false): Promise<AudioCacheSnapshot> {
    if (this.pending) {
      const revision = this.revision, stale = this.pendingRevision !== revision;
      const result = await this.pending;
      return freshSizes || stale || this.revision !== revision ? this.read(cache, freshSizes) : result;
    }
    const revision = this.revision;
    const work = async () => {
      const requests = await cache.keys();
      const entries = requests.map(request => ({ url: request.url, request }));
      const next = new Map<string, number>();
      let cursor = 0;
      let failed = false, firstError: unknown;
      const worker = async () => {
        try {
          while (!failed && cursor < entries.length) {
            const { url, request } = entries[cursor++];
            const saved = freshSizes ? undefined : this.sizes.get(url);
            if (saved !== undefined) { next.set(url, saved); continue; }
            const response = await cache.match(request);
            if (!response) continue;
            const length = Number(response.headers.get('content-length') ?? 0);
            next.set(url, Number.isSafeInteger(length) && length >= 0 ? length : 0);
            // Reading headers must not retain an unused cached media stream.
            await response.body?.cancel().catch(() => {});
          }
        } catch (error) {
          if (!failed) firstError = error;
          failed = true; throw error;
        }
      };
      // Retain ownership until every worker settles. A fast failure must not
      // let a retry start another batch while older header reads are active.
      await Promise.allSettled(Array.from({ length: Math.min(this.concurrency, entries.length) }, worker));
      if (failed) throw firstError;
      // A download/removal may have invalidated this snapshot while it ran.
      // Do not overwrite that newer local bookkeeping with stale metadata.
      if (this.revision === revision) this.sizes = next;
      return { urls: [...next.keys()], bytes: [...next.values()].reduce((sum, bytes) => sum + bytes, 0) };
    };
    const pending = work(); this.pending = pending; this.pendingRevision = revision;
    try { return await pending; }
    finally { if (this.pending === pending) this.pending = null; }
  }
}
