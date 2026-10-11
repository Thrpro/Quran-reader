import { memo, type CSSProperties } from "react";

export type ShapedLine = {
  width: number; scale: number; shift: number;
  baselineOffset?: number;
  glyphs: [number, number, number, number][];
  wordBounds: Record<number, [number, number]>;
};
export type ShapedPage = { pageNumber: number; lines: Record<number, ShapedLine> };
// Cache the immutable line artwork once. Page folds can reuse the decoded SVG
// image instead of cloning and repainting hundreds of individual glyph nodes.
// Weak keys release artwork when its shaped page leaves the page cache.
const lineArtwork = new WeakMap<ShapedLine, { normal?: string; dark?: string }>();
function getLineArtwork(shape: ShapedLine, outlines: Record<number, string>, darkInk: boolean) {
  const variant = darkInk ? "dark" : "normal";
  let artwork = lineArtwork.get(shape);
  if (!artwork?.[variant]) {
    const paths = shape.glyphs.map(([id, x, y]) =>
      `<path d="${outlines[id]}" transform="translate(${x} ${y})"/>`).join("");
    // Extra transparent vertical room preserves tall marks outside the old
    // inline SVG viewport. The centre stays at y=-350 in font coordinates.
    const svg = (ink: string) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${-shape.width} -1950 ${shape.width} 3200"><g fill="white" ${ink} transform="translate(${shape.shift} 0) scale(${shape.scale} ${-shape.scale})">${paths}</g></svg>`;
    const image = (markup: string) => `url("data:image/svg+xml,${encodeURIComponent(markup)}")`;
    // SVG masks use grayscale coverage, unlike the hinted font text in normal
    // mode. A small optical weight on dark backgrounds keeps thin marks legible.
    // 0.12 natural-page pixels is display styling, not a new glyph outline or
    // horizontal stretch. Keep the cached images so folds stay inexpensive.
    const opticalWeight = shape.width / (513 * shape.scale) * 0.12;
    artwork ??= {};
    artwork[variant] = image(svg(darkInk ? `stroke="white" stroke-width="${opticalWeight}" stroke-linejoin="round"` : ""));
    lineArtwork.set(shape, artwork);
  }
  return artwork[variant];
}

type Word = { id: number; text: string; surah: number; ayah: number };

// The outlines are produced by Digital Khatt's shaper at build time. Runtime
// work is limited to placing them and retaining source text for accessibility.
export const ShapedQuranLine = memo(function ShapedQuranLine({ shape, outlines, words, lineNumber, highlighted, reciting, darkInk }: {
  shape: ShapedLine; outlines: Record<number, string>; words: Word[]; lineNumber: number;
  highlighted: { firstWordId: number; lastWordId: number } | null; reciting: string | null;
  darkInk: boolean;
}) {
  const runs: { key: string; words: Word[]; left: number; right: number; selected: boolean }[] = [];
  const artwork = getLineArtwork(shape, outlines, darkInk);
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
    <div className="shapedLineInk" aria-hidden="true" style={{
      aspectRatio: `${shape.width} / 3200`,
      maskImage: artwork,
    } as CSSProperties} />
    {runs.map(run => <span key={run.key} data-ayah-key={run.key}
      className={`ayahRun shapedAyahRun ${run.selected ? "ayahHighlightRun" : ""} ${run.key === reciting ? "ayahRun-reciting" : ""}`}
      style={{ left: `${run.left}%`, width: `${run.right - run.left}%` }}>
      {run.words.map(word => <span className="ayahWord" key={word.id}>{word.text}{word.id !== words.at(-1)?.id ? " " : ""}</span>)}
    </span>)}
  </div>;
});
