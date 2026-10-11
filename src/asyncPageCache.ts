// Keep only a small reading window; failed requests must never poison the cache.
export class AsyncPageCache<T> {
  private readonly values = new Map<number, T>();
  private readonly pending = new Map<number, Promise<T>>();
  private readonly capacity: number;
  private readonly loader: (pageNumber: number) => Promise<T>;

  constructor(capacity: number, loader: (pageNumber: number) => Promise<T>) {
    if (!Number.isInteger(capacity) || capacity < 1) {
      throw new RangeError("Page cache capacity must be a positive integer");
    }
    this.capacity = capacity;
    this.loader = loader;
  }

  peek(pageNumber: number): T | undefined {
    return this.values.get(pageNumber);
  }

  get size() {
    return this.values.size;
  }

  load(pageNumber: number): Promise<T> {
    if (this.values.has(pageNumber)) {
      const value = this.values.get(pageNumber)!;
      this.values.delete(pageNumber);
      this.values.set(pageNumber, value);
      return Promise.resolve(value);
    }

    const existing = this.pending.get(pageNumber);
    if (existing) return existing;

    const request = Promise.resolve()
      .then(() => this.loader(pageNumber))
      .then((value) => {
        this.values.set(pageNumber, value);
        while (this.values.size > this.capacity) {
          this.values.delete(this.values.keys().next().value!);
        }
        return value;
      })
      .finally(() => this.pending.delete(pageNumber));

    this.pending.set(pageNumber, request);
    return request;
  }
}
