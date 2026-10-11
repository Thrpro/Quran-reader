import {loadHarfbuzz, loadAndCacheFont, harfbuzzFonts, getWidth, HarfBuzzBuffer, hb} from './vendor/digitalkhatt/harfbuzz';
import {analyzeLineForJust, justifyLine, SpaceType} from './vendor/digitalkhatt/just.service';

await loadHarfbuzz('/hb.wasm');
await loadAndCacheFont('madina', '/madina.otf');
const font = harfbuzzFonts.get('madina')!;
const space = getWidth(' ', font, 1000, []);
// Use the upstream contextual justification at a quieter type size. Keep the
// baseline size independent of each line's final horizontal fitting correction.
const fontSize = 38;
const width = 1000 * 606 / fontSize;
const outlines: Record<number, string> = {};
const bounds: Record<number, DOMRect> = {};
const measureSvg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
document.body.append(measureSvg);
const service = {quranText: [] as string[][]};
const round = (n: number) => Math.round(n * 1000) / 1000;

// Only shape a display copy. The original strings remain the integrity source.
(window as any).shapePage = (page: any) => {
  const index = page.pageNumber - 1;
  service.quranText[index] = page.lines.map((l: any) => l.text);
  const lines: Record<number, any> = {};
  for (let i = 0; i < page.lines.length; i++) {
    const line = page.lines[i];
    if (line.type !== 'ayah' || line.isCentered) continue;
    const info = analyzeLineForJust(service, index, i);
    const just = justifyLine(info, font, fontSize / 606, space);
    const features = [...(info.features || [])];
    for (const [start, values] of just.fontFeatures) {
      for (const f of values) features.push({tag: f.name, value: f.value, start, end: start + 1});
    }
    const buffer = new HarfBuzzBuffer();
    let glyphs;
    try {
      buffer.setDirection('rtl'); buffer.setLanguage(hb.arabLanguage);
      buffer.setScript(hb.arabScript); buffer.setClusterLevel(1);
      buffer.addText(line.text); glyphs = buffer.shape(font, features);
    } finally { buffer.destroy(); }
    let x = 0;
    let minX=Infinity, maxX=-Infinity, minY=Infinity, maxY=-Infinity;
    const positions: number[][] = [];
    const wordBounds: Record<number, [number, number]> = {};
    const words: {start: number; end: number; id: number}[] = [];
    let offset = 0;
    for (const word of line.words) {
      words.push({start: offset, end: offset + word.text.length, id: word.id});
      offset += word.text.length + 1;
    }
    for (const glyph of glyphs.reverse()) {
      if (glyph.GlyphId === 0) throw Error(`Missing glyph: page ${page.pageNumber}, line ${line.lineNumber}`);
      const kind = info.spaces.get(glyph.Cluster);
      x -= kind === SpaceType.Aya ? just.ayaSpacing : kind === SpaceType.Simple ? just.simpleSpacing : glyph.XAdvance;
      if (kind) continue;
      outlines[glyph.GlyphId] ??= font.glyphToSvgPath(glyph.GlyphId);
      if (!bounds[glyph.GlyphId]) {
        const path=document.createElementNS('http://www.w3.org/2000/svg','path');
        path.setAttribute('d',outlines[glyph.GlyphId]); measureSvg.append(path);
        bounds[glyph.GlyphId]=path.getBBox(); path.remove();
      }
      const box=bounds[glyph.GlyphId];
      if(box.width || box.height) {
        minX=Math.min(minX,x+glyph.XOffset+box.x); maxX=Math.max(maxX,x+glyph.XOffset+box.x+box.width);
        minY=Math.min(minY,glyph.YOffset+box.y); maxY=Math.max(maxY,glyph.YOffset+box.y+box.height);
      }
      const word = words.find(w => glyph.Cluster >= w.start && glyph.Cluster < w.end);
      if (!word) throw Error('Unmapped glyph cluster');
      const left = x + glyph.XOffset + box.x;
      const right = left + box.width;
      const previous = wordBounds[word.id];
      wordBounds[word.id] = previous ? [Math.min(previous[0], left), Math.max(previous[1], right)] : [left, right];
      positions.push([glyph.GlyphId, round(x + glyph.XOffset), glyph.YOffset, word.id]);
    }
    // Contextual substitutions can change advances after the word-level estimate.
    // Fit the actual final glyph run uniformly in both dimensions.
    // Never squeeze the letters horizontally; marks retain their proportions.
    if (!Number.isFinite(x) || x >= 0) throw Error('Invalid shaped advance');
    const scale=width/(maxX-minX);
    lines[line.lineNumber] = {width, scale, shift:-width-minX*scale, bounds:[minX,minY,maxX,maxY], glyphs: positions, wordBounds};
  }
  // Tall stacked marks need more than a uniform row gap. Measure neighboring
  // glyphs that overlap horizontally, then distribute only the missing clearance
  // between their baselines. Outlines, word order and line breaks stay untouched.
  const rows = Object.keys(lines).map(Number);
  const pixelRatio = 513 / width;
  const pitch = (919 - 24 - 34) / 15;
  const gaps: { previous: number; current: number; clearance: number }[] = [];
  for (const row of rows) {
    const a = lines[row - 1], b = lines[row];
    if (!a) continue;
    let clearance = Infinity;
    for (const ga of a.glyphs) for (const gb of b.glyphs) {
      const ba = bounds[ga[0]], bb = bounds[gb[0]];
      const ax = (ga[1] + ba.x) * a.scale + a.shift;
      const bx = (gb[1] + bb.x) * b.scale + b.shift;
      if (ax + ba.width * a.scale < bx || bx + bb.width * b.scale < ax) continue;
      clearance = Math.min(clearance, pitch - ((gb[2] + bb.y + bb.height) * b.scale - (ga[2] + ba.y) * a.scale) * pixelRatio);
    }
    gaps.push({ previous: row - 1, current: row, clearance });
  }
  for (const row of rows) lines[row].baselineOffset = 0;
  for (let pass = 0; pass < 200; pass++) {
    let largest = 0;
    for (const gap of gaps) {
      const a = lines[gap.previous], b = lines[gap.current];
      const missing = 3 - (gap.clearance + b.baselineOffset - a.baselineOffset);
      if (missing <= 0.001) continue;
      largest = Math.max(largest, missing);
      // Anchor the outer rows so headers and footers keep their clearance.
      if (gap.previous === rows[0]) b.baselineOffset += missing;
      else if (gap.current === rows.at(-1)) a.baselineOffset -= missing;
      else { a.baselineOffset -= missing / 2; b.baselineOffset += missing / 2; }
    }
    if (largest <= 0.001) break;
  }
  for (const gap of gaps) {
    if (gap.clearance + lines[gap.current].baselineOffset - lines[gap.previous].baselineOffset < 2.99)
      throw Error(`Insufficient mark clearance: page ${page.pageNumber}, row ${gap.current}`);
  }
  return {pageNumber: page.pageNumber, lines};
};
(window as any).getOutlines = () => outlines;
(window as any).ready = true;
