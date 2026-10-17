import type { ShapedLine } from "./ShapedQuranLine";

const MAX_PARSED_GLYPHS = 256;
const parsedGlyphs = new WeakMap<Record<number, string>, Map<number, Path2D>>();
const darkAlpha = Uint8ClampedArray.from({ length: 256 }, (_, alpha) =>
  Math.round(255 * Math.pow(alpha / 255, 0.85)));

function glyphPath(outlines: Record<number, string>, id: number): Path2D {
  let cache = parsedGlyphs.get(outlines);
  if (!cache) {
    cache = new Map();
    parsedGlyphs.set(outlines, cache);
  }
  let path = cache.get(id);
  if (path) cache.delete(id);
  else path = new Path2D(outlines[id]);
  cache.set(id, path);
  if (cache.size > MAX_PARSED_GLYPHS) cache.delete(cache.keys().next().value!);
  return path;
}

// Paint the supplied Digital Khatt outlines directly. The source paths and
// placements match the former SVG viewBox and xMidYMid meet transformation.
// No decoded SVG documents or image URLs survive page changes.
export function paintShapedLine(canvas: HTMLCanvasElement, shape: ShapedLine,
  outlines: Record<number, string>, color: string, darkInk: boolean,
  viewport?: { width: number; height: number; offsetX: number; offsetY: number }): void {
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return;
  const width = canvas.width;
  const height = canvas.height;
  context.setTransform(1, 0, 0, 1, 0, 0);
  context.clearRect(0, 0, width, height);
  if (width === 0 || height === 0) return;

  // A screen-aligned backing store can include fractional padding around the
  // original viewport. Keep that viewport exact rather than rescaling glyphs.
  const frame = viewport ?? { width, height, offsetX: 0, offsetY: 0 };
  const scale = Math.min(frame.width / shape.width, frame.height / 3200);
  context.save();
  context.setTransform(scale, 0, 0, scale,
    (frame.width + shape.width * scale) / 2 + frame.offsetX,
    (frame.height - 3200 * scale) / 2 + 1950 * scale + frame.offsetY);
  context.translate(shape.shift, 0);
  context.scale(shape.scale, -shape.scale);
  context.fillStyle = color;
  for (const [id, x, y] of shape.glyphs) {
    context.save();
    context.translate(x, y);
    context.fill(glyphPath(outlines, id));
    context.restore();
  }
  context.restore();

  if (darkInk) {
    // Match the original alpha-only contrast correction. Transparent contour
    // edges stay transparent and solid source ink remains fully opaque.
    const pixels = context.getImageData(0, 0, width, height);
    for (let index = 3; index < pixels.data.length; index += 4) {
      pixels.data[index] = darkAlpha[pixels.data[index]];
    }
    context.putImageData(pixels, 0, 0);
    // putImageData copies synchronously. Release its temporary readback buffer
    // now instead of leaving many pages of pixel arrays waiting for a JS GC.
    const storage = pixels.data.buffer as ArrayBuffer & { transfer?: (size: number) => ArrayBuffer };
    storage.transfer?.(0);
  }
}
