import { memo, useLayoutEffect, useMemo, useRef, type CSSProperties } from "react";
import { paintShapedLine } from "./paintShapedLine";
import { acquireInkCanvas, releaseInkCanvas } from "./inkCanvasPool";
import { InkJobScheduler } from "./inkJobScheduler";
import { cropShapedInk } from "./inkCrop";
import { groupAyahWords, scopedHighlight } from "./ayahRenderScope";

export type ShapedLine = {
  width: number; scale: number; shift: number;
  baselineOffset?: number;
  glyphs: [number, number, number, number][];
  wordBounds: Record<number, [number, number]>;
};
export type ShapedPage = { pageNumber: number; lines: Record<number, ShapedLine> };
type InkJob = { element: HTMLDivElement; canvas: HTMLCanvasElement | null; shape: ShapedLine; outlines: Record<number, string>; darkInk: boolean; inkTheme: string; lastPaint: string; lastPaintColor?: string; layoutAnimation?: Animation };
const activeInk = new Set<InkJob>();
const PREVIEW_IDLE_MS = 6000;
let previewIdleTimer: ReturnType<typeof setTimeout> | undefined;
function sleepPreviewInk() {
  previewIdleTimer = undefined;
  if (document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)')) {
    previewIdleTimer = setTimeout(sleepPreviewInk, 1000);
    return;
  }
  for (const job of activeInk) {
    if (job.element.closest('.readerViewport') || !job.canvas) continue;
    // Keep source, geometry and mounted nodes ready; release only invisible
    // raster storage. An actual fold explicitly paints its exact source first.
    releaseInkCanvas(job.canvas, false);
    job.canvas = null;
    job.lastPaint = '';
  }
}
function wakePreviewInk() {
  clearTimeout(previewIdleTimer);
  previewIdleTimer = setTimeout(sleepPreviewInk, PREVIEW_IDLE_MS);
}
function scheduleInk(job: InkJob) {
  if (job.element.closest('.readerViewport')) scheduler.schedule(job);
}
const scheduler = new InkJobScheduler<InkJob>({
  current: job => activeInk.has(job) && job.element.isConnected,
  visible: job => job.element.closest('.readerViewport') !== null,
  paint: drawInk,
});
export function refreshShapedInk() {
  for (const job of activeInk) scheduleInk(job);
}
export function flushShapedInkForPage(page: HTMLElement) {
  if (activeInk.size) wakePreviewInk();
  scheduler.flush([...activeInk].filter(job => page.contains(job.element)));
}
function drawInk(jobs: InkJob[], immediate = false) {
    const animations = new Map<HTMLElement, Animation | undefined>();
    const ready = jobs.filter(job => {
      // Keep neighbor source/geometry ready, but rasterize hidden pages only
      // when an actual fold requests them. Painting all four neighbors after
      // every turn feeds the compositor cache with never-displayed images.
      if (!immediate && !job.element.closest('.readerViewport')) return false;
      if (immediate || !job.canvas || !job.lastPaint) return true;
      const surface = job.element.closest<HTMLElement>('.readerScaleSurface');
      if (!surface) return true;
      if (!animations.has(surface)) animations.set(surface, surface.getAnimations().find(animation => animation.playState === 'running' || animation.pending));
      const animation = animations.get(surface);
      if (!animation) return true;
      const style = getComputedStyle(job.element);
      if (job.lastPaintColor !== (style.getPropertyValue('--mushaf-text').trim() || style.color)) return true;
      // An existing bitmap already follows the reader's temporary FLIP scale.
      // Do not allocate another backing store at that intermediate geometry.
      // First/content/theme paints and explicit fold snapshots stay immediate.
      if (job.layoutAnimation !== animation) {
        job.layoutAnimation = animation;
        void animation.finished.catch(() => {}).then(() => {
          if (job.layoutAnimation !== animation) return;
          job.layoutAnimation = undefined;
          if (activeInk.has(job) && job.element.isConnected) scheduleInk(job);
        });
      }
      return false;
    });
    // Read all geometry before allocating backing stores, avoiding a layout per
    // line. Rasterize at the actual display density, including reader zoom.
    const sizes = ready.map(item => {
      const rect = item.element.getBoundingClientRect(), style = getComputedStyle(item.element);
      const dpr = Math.max(2, devicePixelRatio), left = Math.floor(rect.left * dpr), top = Math.floor(rect.top * dpr);
      return { item, width: Math.ceil(rect.right * dpr) - left, height: Math.ceil(rect.bottom * dpr) - top,
        viewport: { width: rect.width * dpr, height: rect.height * dpr, offsetX: rect.left * dpr - left, offsetY: rect.top * dpr - top },
        zoomX: rect.width / parseFloat(style.width), zoomY: rect.height / parseFloat(style.height), dpr,
        // Theme colors can transition; use the destination ink token so a
        // canvas never freezes an intermediate color during the crossfade.
        color: style.getPropertyValue("--mushaf-text").trim() || style.color };
    });
    for (const { item, width, height: fullHeight, viewport, zoomX, zoomY, dpr, color } of sizes) {
      if (!width || !fullHeight || !zoomX || !zoomY) continue;
      // Keep the established high-density presentation. In software Edge its
      // subpixel sampling differs for transformed/cropped surfaces; the crop
      // path is enabled only at densities verified against the original ink.
      const trimmed = devicePixelRatio < 1.5;
      const crop = trimmed ? cropShapedInk(item.shape, item.outlines, fullHeight, viewport) : {top:0,height:fullHeight};
      const height = crop.height;
      const key = `${width}:${height}:${crop.top}:${viewport.width}:${viewport.height}:${viewport.offsetX}:${viewport.offsetY}:${color}:${item.darkInk}`;
      // Keep exact backing extents: even transparent capacity can change
      // browser edge sampling. Mounted contexts still survive navigation.
      if (!item.canvas) {
        item.canvas = acquireInkCanvas(width, height);
        item.canvas.className = "shapedInkCanvas";
        item.element.append(item.canvas);
        item.lastPaint = "";
      }
      if (item.canvas.width !== width) { item.canvas.width = width; item.lastPaint = ""; }
      if (item.canvas.height !== height) { item.canvas.height = height; item.lastPaint = ""; }
      // Align the bitmap to physical pixels, but offset its original paths by
      // the opposite amount. This avoids a second interpolation at fractional
      // reader zoom without shifting a single glyph on the page.
      // CSS layout rounds lengths before applying reader zoom. Keep integer
      // bitmap dimensions and put the precise conversion in a 2D transform;
      // cropped rows then share the full viewport's physical pixel grid.
      if (trimmed) {
        item.canvas.style.left = "0"; item.canvas.style.top = "0";
        item.canvas.style.width = `${item.canvas.width}px`;
        item.canvas.style.height = `${item.canvas.height}px`;
        item.canvas.style.transformOrigin = "0 0";
        item.canvas.style.transform = `matrix(${1 / (dpr * zoomX)},0,0,${1 / (dpr * zoomY)},${-viewport.offsetX / (dpr * zoomX)},${(crop.top - viewport.offsetY) / (dpr * zoomY)})`;
      } else {
        item.canvas.style.left = `${-viewport.offsetX / (dpr * zoomX)}px`;
        item.canvas.style.top = `${-viewport.offsetY / (dpr * zoomY)}px`;
        item.canvas.style.width = `${item.canvas.width / (dpr * zoomX)}px`;
        item.canvas.style.height = `${item.canvas.height / (dpr * zoomY)}px`;
        item.canvas.style.removeProperty('transform');
        item.canvas.style.removeProperty('transform-origin');
      }
      if (key === item.lastPaint) continue;
      paintShapedLine(item.canvas, item.shape, item.outlines, color, item.darkInk, {...viewport, offsetY: viewport.offsetY - crop.top});
      item.lastPaint = key; item.lastPaintColor = color;
    }
}

type Word = { id: number; text: string; surah: number; ayah: number };

// The outlines are produced by Digital Khatt's shaper at build time. Runtime
// work is limited to placing them and retaining source text for accessibility.
export const ShapedQuranLine = memo(function ShapedQuranLine({ shape, outlines, words, lineNumber, highlighted, reciting, darkInk, renderScale, inkTheme }: {
  shape: ShapedLine; outlines: Record<number, string>; words: Word[]; lineNumber: number;
  highlighted: { firstWordId: number; lastWordId: number } | null; reciting: string | null;
  darkInk: boolean;
  renderScale: number; inkTheme: string;
}) {
  const runs = useMemo(() => groupAyahWords(words).map(run => {
    let left = Infinity, right = -Infinity;
    for (const word of run.words) {
      const bounds = shape.wordBounds[word.id];
      left = Math.min(left, (bounds[0] * shape.scale + shape.shift + shape.width) / shape.width * 100);
      right = Math.max(right, (bounds[1] * shape.scale + shape.shift + shape.width) / shape.width * 100);
    }
    return { ...run, left, right };
  }), [shape, words]);
  const ink = useRef<HTMLDivElement | null>(null);
  const inkJob = useRef<InkJob | null>(null);
  useLayoutEffect(() => {
    const element = ink.current;
    if (!element) return;
    const job: InkJob = { element, canvas: null, shape, outlines, darkInk, inkTheme, lastPaint: "" };
    inkJob.current = job;
    activeInk.add(job);
    const draw = () => scheduleInk(job);
    draw();
    const observer = new ResizeObserver(draw); observer.observe(element);
    const systemTheme = matchMedia("(prefers-color-scheme: dark)");
    systemTheme.addEventListener("change", draw);
    window.addEventListener("resize", draw);
    return () => {
      observer.disconnect(); systemTheme.removeEventListener("change", draw); window.removeEventListener("resize", draw);
      scheduler.remove(job);
      activeInk.delete(job);
      if (!activeInk.size) { clearTimeout(previewIdleTimer); previewIdleTimer = undefined; }
      // Reuse these buffers during rapid navigation, then release idle storage.
      if (job.canvas) releaseInkCanvas(job.canvas);
      job.canvas = null;
      if (inkJob.current === job) inkJob.current = null;
    };
    // This job, its observers and canvas follow the mounted DOM lifetime.
    // Prop changes below update it without destroying reusable drawing state.
  }, []);
  useLayoutEffect(() => {
    const job = inkJob.current;
    if (!job) return;
    if (job.shape !== shape || job.outlines !== outlines || job.darkInk !== darkInk || job.inkTheme !== inkTheme) job.lastPaint = "";
    job.shape = shape; job.outlines = outlines; job.darkInk = darkInk; job.inkTheme = inkTheme;
    scheduleInk(job);
  }, [shape, outlines, darkInk, renderScale, inkTheme]);
  return <div translate="no" lang="ar" className="qulLine digitalKhattLine shapedQuranLine line-ayah line-ayahInteractions"
    style={{ "--line-number": lineNumber, transform: `translateY(${shape.baselineOffset ?? 0}px)` } as CSSProperties}>
    <div ref={ink} className="shapedLineInk" aria-hidden="true" data-ink-contrast={darkInk} style={{
      aspectRatio: `${shape.width} / 3200`,
    } as CSSProperties} />
    {runs.map(run => <span key={run.key} data-ayah-key={run.key}
      className={`ayahRun shapedAyahRun ${scopedHighlight(run.scope, highlighted) ? "ayahHighlightRun" : ""} ${run.key === reciting ? "ayahRun-reciting" : ""}`}
      style={{ left: `${run.left}%`, width: `${run.right - run.left}%` }}>
      {run.words.map(word => <span className="ayahWord" key={word.id}>{word.text}{word.id !== words.at(-1)?.id ? " " : ""}</span>)}
    </span>)}
  </div>;
});
