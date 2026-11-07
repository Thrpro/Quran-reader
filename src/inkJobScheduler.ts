type Options<T> = {
  current: (job: T) => boolean;
  visible: (job: T) => boolean;
  paint: (jobs: T[], immediate?: boolean) => void;
  optionalBatch?: number;
  defer?: (callback: () => void) => void;
  yieldTask?: (callback: () => void) => ReturnType<typeof setTimeout>;
  cancelTask?: (timer: ReturnType<typeof setTimeout>) => void;
};

/** Visible ink is committed before optional artwork. Preview batches yield to
 * input/rendering; a fold can explicitly flush its source pages before cloning. */
export class InkJobScheduler<T> {
  private pending = new Set<T>();
  private microtask = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private options: Options<T>;
  private batch: number;

  constructor(options: Options<T>) {
    this.options = options;
    this.batch = options.optionalBatch ?? 4;
    if (!Number.isInteger(this.batch) || this.batch < 1) throw new RangeError('Invalid optional ink batch size');
  }

  schedule(job: T) {
    this.pending.add(job);
    if (this.microtask) return;
    this.microtask = true;
    (this.options.defer ?? queueMicrotask)(() => {
      this.microtask = false;
      this.prune();
      const visible = [...this.pending].filter(this.options.visible);
      for (const item of visible) this.pending.delete(item);
      try { if (visible.length) this.options.paint(visible); }
      finally { this.scheduleOptional(); }
    });
  }

  remove(job: T) {
    this.pending.delete(job);
    this.cancelIfEmpty();
  }

  /** Finish and revalidate an exact set even when geometry changed without a
   * resize notification. No unfinished preview can be copied into a fold. */
  flush(jobs: T[]) {
    const current = jobs.filter(this.options.current);
    for (const job of current) this.pending.delete(job);
    this.cancelIfEmpty();
    if (current.length) this.options.paint(current, true);
  }

  private prune() {
    for (const job of this.pending) if (!this.options.current(job)) this.pending.delete(job);
    this.cancelIfEmpty();
  }

  private cancelIfEmpty() {
    if (this.pending.size || this.timer === undefined) return;
    (this.options.cancelTask ?? clearTimeout)(this.timer);
    this.timer = undefined;
  }

  private scheduleOptional() {
    this.cancelIfEmpty();
    if (!this.pending.size || this.timer !== undefined) return;
    this.timer = (this.options.yieldTask ?? (callback => setTimeout(callback, 0)))(() => {
      this.timer = undefined;
      this.prune();
      // Recheck priority after yielding: a former preview may now be visible.
      const jobs = [...this.pending];
      const visible = jobs.filter(this.options.visible);
      const optional = jobs.filter(job => !this.options.visible(job)).slice(0, this.batch);
      const next = [...visible, ...optional];
      for (const job of next) this.pending.delete(job);
      try { if (next.length) this.options.paint(next); }
      finally { this.scheduleOptional(); }
    });
  }
}
