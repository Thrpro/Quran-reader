const MAX_IDLE_CANVASES = 60;
const IDLE_MILLISECONDS = 1500;
const available = new Map<string, HTMLCanvasElement[]>();
const leased = new WeakSet<HTMLCanvasElement>();
let idleCount = 0;
let expiry: ReturnType<typeof setTimeout> | undefined;

function discard(canvas: HTMLCanvasElement) {
  canvas.width = 0;
  canvas.height = 0;
}

function expireIdle() {
  expiry = undefined;
  for (const canvases of available.values()) for (const canvas of canvases) discard(canvas);
  available.clear();
  idleCount = 0;
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
  const reuseKey = available.has(key) ? key : available.keys().next().value;
  const bucket = reuseKey === undefined ? undefined : available.get(reuseKey);
  const canvas = bucket?.pop() ?? document.createElement("canvas");
  if (bucket) {
    idleCount--;
    if (bucket.length === 0) available.delete(reuseKey!);
    if (idleCount === 0) {
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

export function releaseInkCanvas(canvas: HTMLCanvasElement): void {
  if (!leased.delete(canvas)) return;
  canvas.remove();
  if (idleCount < MAX_IDLE_CANVASES) {
    const key = `${canvas.width}:${canvas.height}`;
    const bucket = available.get(key) ?? [];
    bucket.push(canvas);
    available.set(key, bucket);
    idleCount++;
  } else discard(canvas);
  // Active page-turn pipelines reuse these stores. Once releases stop, idle
  // canvases expire together; visible or active fold canvases are never reset.
  clearTimeout(expiry);
  expiry = setTimeout(expireIdle, IDLE_MILLISECONDS);
}
