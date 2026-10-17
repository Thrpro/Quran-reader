// One request per page, with visible work ahead of optional preview/read-ahead.
export type PageLoadPriority = "visible" | "neighbor" | "idle";
const priorities: Record<PageLoadPriority, number> = { visible: 0, neighbor: 1, idle: 2 };
type Request<T> = {
  page: number; priority: number; running: boolean; promise: Promise<T>;
  resolve: (value: T) => void; reject: (reason: unknown) => void;
};
export class AsyncPageCache<T> {
  private readonly values = new Map<number, T>();
  private readonly pending = new Map<number, Request<T>>();
  private readonly capacity: number;
  private readonly loader: (pageNumber: number, priority: PageLoadPriority) => Promise<T>;
  private readonly maxConcurrent: number;
  private running = 0;
  private pumpScheduled = false;

  constructor(capacity: number, loader: (pageNumber: number, priority: PageLoadPriority) => Promise<T>, options: { maxConcurrent?: number } = {}) {
    if (!Number.isInteger(capacity) || capacity < 1) throw new RangeError("Page cache capacity must be a positive integer");
    const concurrency = options.maxConcurrent ?? 4;
    if (!Number.isInteger(concurrency) || concurrency < 1) throw new RangeError("Page cache concurrency must be a positive integer");
    this.capacity = capacity;
    this.loader = loader;
    this.maxConcurrent = concurrency;
  }

  peek(pageNumber: number): T | undefined { return this.values.get(pageNumber); }
  get size() { return this.values.size; }
  get pendingCount() { return this.pending.size; }

  // Navigation drops only optional work that has not started. A shared request
  // promoted to visible is never aborted by a preview's cleanup or window change.
  retainPending(pageNumbers: readonly number[]) {
    const keep = new Set(pageNumbers);
    for (const [page, request] of this.pending) {
      if (request.running || request.priority === priorities.visible || keep.has(page)) continue;
      this.pending.delete(page);
      request.reject(new DOMException("Outdated page preview", "AbortError"));
    }
  }

  load(pageNumber: number, priority: PageLoadPriority = "visible"): Promise<T> {
    if (this.values.has(pageNumber)) {
      const value = this.values.get(pageNumber)!;
      this.values.delete(pageNumber);
      this.values.set(pageNumber, value);
      return Promise.resolve(value);
    }
    const existing = this.pending.get(pageNumber);
    if (existing) {
      existing.priority = Math.min(existing.priority, priorities[priority]);
      return existing.promise;
    }
    let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
    const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
    this.pending.set(pageNumber, { page: pageNumber, priority: priorities[priority], running: false, promise, resolve, reject });
    this.schedulePump();
    return promise;
  }

  private schedulePump() {
    if (this.pumpScheduled) return;
    this.pumpScheduled = true;
    queueMicrotask(() => {
      this.pumpScheduled = false;
      while (this.running < this.maxConcurrent) {
        let next: Request<T> | undefined;
        for (const request of this.pending.values()) {
          if (!request.running && (!next || request.priority < next.priority)) next = request;
        }
        if (!next) break;
        this.start(next);
      }
    });
  }

  private start(request: Request<T>) {
    request.running = true;
    this.running++;
    const release = () => {
      this.pending.delete(request.page);
      this.running--;
      this.schedulePump();
    };
    // Loader errors, including synchronous throws, always release the slot.
    let result: Promise<T>;
    try { result = this.loader(request.page, (["visible", "neighbor", "idle"] as const)[request.priority]); }
    catch (error) { release(); request.reject(error); return; }
    Promise.resolve(result).then(value => {
      this.values.set(request.page, value);
      while (this.values.size > this.capacity) this.values.delete(this.values.keys().next().value!);
      release();
      request.resolve(value);
    }, error => { release(); request.reject(error); });
  }
}
