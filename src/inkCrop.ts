import type { ShapedLine } from './ShapedQuranLine';

type Bounds = [number, number];
const cached = new WeakMap<Record<number,string>, Map<number, Bounds | null>>();
const coordinates = /[-+]?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?/gi;

/** The bundled paths use absolute M/L/C/Z. A Bezier curve lies inside its
 * control-point bounds, so these conservative limits cannot trim a mark.
 * Unknown future syntax falls back to the complete source viewport. */
export function outlineYBounds(path: string): Bounds | null {
  if (typeof path !== 'string' || /[a-df-z]/gi.test(path.replace(/[MLCQZ]/g,''))) return null;
  const values = path.match(coordinates)?.map(Number);
  if (!values?.length || values.length % 2 || values.some(value => !Number.isFinite(value))) return null;
  let low=Infinity, high=-Infinity;
  for (let index=1; index<values.length; index+=2) {low=Math.min(low,values[index]);high=Math.max(high,values[index]);}
  return [low,high];
}

/** Allocate only rows that may contain ink. Keep the full original viewport
 * for drawing, shifting its origin by an integer pixel crop. No resampling. */
export function cropShapedInk(shape: ShapedLine, outlines: Record<number,string>, height: number,
  frame: {width:number;height:number;offsetY:number}): {top:number;height:number} {
  const full = {top:0,height};
  if (!(shape.width>0) || !(shape.scale>0) || !shape.glyphs.length) return full;
  let cache=cached.get(outlines);
  if (!cache) {cache=new Map();cached.set(outlines,cache);}
  let low=Infinity, high=-Infinity;
  for (const [id,,y] of shape.glyphs) {
    let bounds=cache.get(id);
    if (bounds===undefined) {bounds=outlineYBounds(outlines[id]);cache.set(id,bounds);}
    if (!bounds || !Number.isFinite(y)) return full;
    low=Math.min(low,y+bounds[0]);high=Math.max(high,y+bounds[1]);
  }
  const scale=Math.min(frame.width/shape.width,frame.height/3200);
  const origin=(frame.height-3200*scale)/2+1950*scale+frame.offsetY;
  // Two physical pixels also retain the antialias fringe at every density.
  const top=Math.max(0,Math.floor(origin-high*shape.scale*scale)-2);
  const bottom=Math.min(height,Math.ceil(origin-low*shape.scale*scale)+2);
  if (!Number.isFinite(top+bottom) || bottom<=top) return full;
  return {top,height:bottom-top};
}
