const MAX_IDLE_CANVASES = 60;
// Budget only released stores. Visible pages and in-flight folds keep their
// full-resolution leases, regardless of their size or display density.
export const MAX_IDLE_INK_BYTES = 32 * 1024 * 1024;
const IDLE_MILLISECONDS = 1500;
const available = new Map<string, HTMLCanvasElement[]>();
const idle = new Map<HTMLCanvasElement, { key: string; bytes: number }>();
const leased = new WeakSet<HTMLCanvasElement>();
let idleBytes = 0;
let expiry: ReturnType<typeof setTimeout> | undefined;

function discard(canvas: HTMLCanvasElement) {
  canvas.width = 0;
  canvas.height = 0;
}

function expireIdle() {
  expiry = undefined;
  for (const canvas of idle.keys()) discard(canvas);
  available.clear();
  idle.clear();
  idleBytes = 0;
}

function takeIdle(canvas: HTMLCanvasElement) {
  const entry = idle.get(canvas)!;
  idle.delete(canvas);
  idleBytes -= entry.bytes;
  const bucket = available.get(entry.key)!;
  bucket.splice(bucket.indexOf(canvas), 1);
  if (bucket.length === 0) available.delete(entry.key);
}

function evictOldestIdle() {
  const canvas = idle.keys().next().value;
  if (!canvas) return;
  takeIdle(canvas);
  discard(canvas);
}

export function acquireInkCanvas(width: number, height: number): HTMLCanvasElement {
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    throw new RangeError("Ink canvas dimensions must be positive finite numbers");
  }
  const bucketWidth = Math.ceil(width);
  const bucketHeight = Math.ceil(height);
  const key = `${bucketWidth}:${bucketHeight}`;
  // Prefer an exact backing size; otherwise reuse an idle context and resize
  // it. Keep exact pixel extents so texture padding cannot change edge sampling.
  const reusable = available.get(key)?.at(-1) ?? idle.keys().next().value;
  const canvas = reusable ?? document.createElement("canvas");
  if (reusable) {
    takeIdle(reusable);
    if (idle.size === 0) {
      clearTimeout(expiry);
      expiry = undefined;
    }
  }
  // A reused backing store keeps the same native allocation. Reset drawing
  // state and old styles without assigning identical dimensions again.
  canvas.removeAttribute("style");
  canvas.className = "";
  if (canvas.width !== bucketWidth) canvas.width = bucketWidth;
  if (canvas.height !== bucketHeight) canvas.height = bucketHeight;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (context) {
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, canvas.width, canvas.height);
  }
  leased.add(canvas);
  return canvas;
}

export function releaseInkCanvas(canvas: HTMLCanvasElement, retain = true): void {
  if (!leased.delete(canvas)) return;
  canvas.remove();
  if (!retain) { discard(canvas); return; }
  const bytes = canvas.width * canvas.height * 4;
  if (Number.isSafeInteger(bytes) && bytes > 0 && bytes <= MAX_IDLE_INK_BYTES) {
    // Keep recent sizes useful to the current layout. Evict released stores
    // only; never resize a leased canvas to satisfy the cache budget.
    while (idle.size >= MAX_IDLE_CANVASES || idleBytes + bytes > MAX_IDLE_INK_BYTES) evictOldestIdle();
    const key = `${canvas.width}:${canvas.height}`;
    const bucket = available.get(key) ?? [];
    bucket.push(canvas);
    available.set(key, bucket);
    idle.set(canvas, { key, bytes });
    idleBytes += bytes;
  } else discard(canvas);
  // Active page-turn pipelines reuse these stores. Once releases stop, idle
  // canvases expire together; visible or active fold canvases are never reset.
  clearTimeout(expiry);
  expiry = idle.size ? setTimeout(expireIdle, IDLE_MILLISECONDS) : undefined;
}
