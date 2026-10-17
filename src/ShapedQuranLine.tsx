import { memo, useLayoutEffect, useRef, type CSSProperties } from "react";
import { paintShapedLine } from "./paintShapedLine";
import { acquireInkCanvas, releaseInkCanvas } from "./inkCanvasPool";

export type ShapedLine = {
  width: number; scale: number; shift: number;
  baselineOffset?: number;
  glyphs: [number, number, number, number][];
  wordBounds: Record<number, [number, number]>;
};
export type ShapedPage = { pageNumber: number; lines: Record<number, ShapedLine> };
type InkJob = { element: HTMLDivElement; canvas: HTMLCanvasElement | null; shape: ShapedLine; outlines: Record<number, string>; darkInk: boolean; lastPaint: string };
const pendingInk = new Set<InkJob>();
const activeInk = new Set<InkJob>();
let inkScheduled = false;
export function refreshShapedInk() {
  for (const job of activeInk) scheduleInk(job);
}
function scheduleInk(job: InkJob) {
  pendingInk.add(job);
  if (inkScheduled) return;
  inkScheduled = true;
  queueMicrotask(() => {
    inkScheduled = false;
    const jobs = [...pendingInk].filter(item => item.element.isConnected);
    pendingInk.clear();
    // Read all geometry before allocating backing stores, avoiding a layout per
    // line. Rasterize at the actual display density, including reader zoom.
    const sizes = jobs.map(item => {
      const rect = item.element.getBoundingClientRect(), style = getComputedStyle(item.element);
      const dpr = Math.max(2, devicePixelRatio), left = Math.floor(rect.left * dpr), top = Math.floor(rect.top * dpr);
      return { item, width: Math.ceil(rect.right * dpr) - left, height: Math.ceil(rect.bottom * dpr) - top,
        viewport: { width: rect.width * dpr, height: rect.height * dpr, offsetX: rect.left * dpr - left, offsetY: rect.top * dpr - top },
        zoomX: rect.width / parseFloat(style.width), zoomY: rect.height / parseFloat(style.height), dpr,
        // Theme colors can transition; use the destination ink token so a
        // canvas never freezes an intermediate color during the crossfade.
        color: style.getPropertyValue("--mushaf-text").trim() || style.color };
    });
    for (const { item, width, height, viewport, zoomX, zoomY, dpr, color } of sizes) {
      if (!width || !height || !zoomX || !zoomY) continue;
      const key = `${width}:${height}:${viewport.width}:${viewport.height}:${viewport.offsetX}:${viewport.offsetY}:${color}`;
      if (key === item.lastPaint) continue;
      item.lastPaint = key;
      // Reuse a bounded set of drawing contexts across page turns.
      if (!item.canvas || item.canvas.width !== width || item.canvas.height !== height) {
        if (item.canvas) releaseInkCanvas(item.canvas);
        item.canvas = acquireInkCanvas(width, height);
        item.canvas.className = "shapedInkCanvas";
        item.element.append(item.canvas);
      }
      // Align the bitmap to physical pixels, but offset its original paths by
      // the opposite amount. This avoids a second interpolation at fractional
      // reader zoom without shifting a single glyph on the page.
      item.canvas.style.left = `${-viewport.offsetX / (dpr * zoomX)}px`;
      item.canvas.style.top = `${-viewport.offsetY / (dpr * zoomY)}px`;
      item.canvas.style.width = `${item.canvas.width / (dpr * zoomX)}px`;
      item.canvas.style.height = `${item.canvas.height / (dpr * zoomY)}px`;
      paintShapedLine(item.canvas, item.shape, item.outlines, color, item.darkInk, viewport);
    }
  });
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
  const runs: { key: string; words: Word[]; left: number; right: number; selected: boolean }[] = [];
  const ink = useRef<HTMLDivElement | null>(null);
  useLayoutEffect(() => {
    const element = ink.current;
    if (!element) return;
    const job: InkJob = { element, canvas: null, shape, outlines, darkInk, lastPaint: "" };
    activeInk.add(job);
    const draw = () => scheduleInk(job);
    draw();
    const observer = new ResizeObserver(draw); observer.observe(element);
    const systemTheme = matchMedia("(prefers-color-scheme: dark)");
    systemTheme.addEventListener("change", draw);
    window.addEventListener("resize", draw);
    return () => {
      observer.disconnect(); systemTheme.removeEventListener("change", draw); window.removeEventListener("resize", draw);
      pendingInk.delete(job);
      activeInk.delete(job);
      // Reuse these buffers during rapid navigation, then release idle storage.
      if (job.canvas) releaseInkCanvas(job.canvas);
      job.canvas = null;
    };
  }, [shape, outlines, darkInk, renderScale, inkTheme]);
  for (const word of words) {
    const key = `${word.surah}:${word.ayah}`;
    const bounds = shape.wordBounds[word.id];
    const left = (bounds[0] * shape.scale + shape.shift + shape.width) / shape.width * 100;
    const right = (bounds[1] * shape.scale + shape.shift + shape.width) / shape.width * 100;
    const selected = !!highlighted && word.id >= highlighted.firstWordId && word.id <= highlighted.lastWordId;
    const previous = runs.at(-1);
    if (previous?.key === key) {
      previous.words.push(word); previous.left = Math.min(previous.left, left);
      previous.right = Math.max(previous.right, right); previous.selected ||= selected;
    } else runs.push({ key, words: [word], left, right, selected });
  }
  return <div translate="no" lang="ar" className="qulLine digitalKhattLine shapedQuranLine line-ayah line-ayahInteractions"
    style={{ "--line-number": lineNumber, transform: `translateY(${shape.baselineOffset ?? 0}px)` } as CSSProperties}>
    <div ref={ink} className="shapedLineInk" aria-hidden="true" data-ink-contrast={darkInk} style={{
      aspectRatio: `${shape.width} / 3200`,
    } as CSSProperties} />
    {runs.map(run => <span key={run.key} data-ayah-key={run.key}
      className={`ayahRun shapedAyahRun ${run.selected ? "ayahHighlightRun" : ""} ${run.key === reciting ? "ayahRun-reciting" : ""}`}
      style={{ left: `${run.left}%`, width: `${run.right - run.left}%` }}>
      {run.words.map(word => <span className="ayahWord" key={word.id}>{word.text}{word.id !== words.at(-1)?.id ? " " : ""}</span>)}
    </span>)}
  </div>;
});
