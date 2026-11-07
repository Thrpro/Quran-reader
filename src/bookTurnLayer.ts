import { FlipCalculation } from "page-flip/geometry";
import type { PageTurn } from "./pageGesture";
import { acquireInkCanvas, releaseInkCanvas } from "./inkCanvasPool";

type Point = { x: number; y: number };
let turnSequence = 0;
type FoldSurface = { element: HTMLDivElement; under: HTMLDivElement; back: HTMLDivElement; shadow: HTMLDivElement; ink: HTMLCanvasElement[] };
// At most one detached fold tree per mounted view, briefly retained between
// queued turns. Its leased buffers have a separate hard 32MiB limit, then expire.
const idleSurfaces = new WeakMap<HTMLElement, { surface: FoldSurface; timer: ReturnType<typeof setTimeout> }>();
function clearSurface(surface: FoldSurface) {
  for (const canvas of surface.ink) releaseInkCanvas(canvas, false);
  surface.ink.length = 0;
  surface.under.replaceChildren(); surface.back.replaceChildren();
  surface.element.replaceChildren();
}
function acquireSurface(view: HTMLElement): FoldSurface {
  const idle = idleSurfaces.get(view);
  if (idle) { clearTimeout(idle.timer); idleSurfaces.delete(view); return idle.surface; }
  const surface = {element: document.createElement('div'), under: document.createElement('div'), back: document.createElement('div'), shadow: document.createElement('div'), ink: [] as HTMLCanvasElement[]};
  surface.element.append(surface.under, surface.back, surface.shadow);
  return surface;
}
function retainSurface(view: HTMLElement, surface: FoldSurface) {
  const old = idleSurfaces.get(view);
  if (old) { clearTimeout(old.timer); clearSurface(old.surface); }
  idleSurfaces.delete(view);
  const bytes = surface.ink.reduce((sum, canvas) => sum + canvas.width * canvas.height * 4, 0);
  if (!view.isConnected || bytes > 32 * 1024 * 1024) { clearSurface(surface); return; }
  const entry = {surface, timer: setTimeout(() => {
    if (idleSurfaces.get(view) !== entry) return;
    idleSurfaces.delete(view); clearSurface(surface);
  }, 1500)};
  idleSurfaces.set(view, entry);
}

export class BookTurnLayer {
  readonly element: HTMLDivElement;
  private readonly surface: FoldSurface;
  private destroyed = false;
  private readonly under: HTMLElement;
  private readonly back: HTMLElement;
  private readonly shadow: HTMLDivElement;
  private readonly calc: FlipCalculation;
  private readonly sign: number;
  private readonly spine: number;
  private readonly pageRect: Point[];
  private active = false;
  private readonly bitmapFold: boolean;
  private readonly inkCanvases: HTMLCanvasElement[] = [];

  constructor(private readonly view: HTMLElement, private readonly width: number,
    private readonly height: number, direction: PageTurn, private readonly corner: "top" | "bottom",
    bottomSource: HTMLElement, backSource: HTMLElement | null, spread: boolean) {
    this.surface = acquireSurface(view);
    this.element = this.surface.element;
    this.shadow = this.surface.shadow;
    this.sign = direction === "next" ? -1 : 1;
    this.spine = spread || direction === "next" ? width : 0;
    this.pageRect = [
      { x: 0, y: 0 },
      { x: width, y: 0 },
      { x: width, y: height },
      { x: 0, y: height },
    ];
    this.calc = new FlipCalculation(direction === "next" ? 1 : 0, corner, String(width), String(height));
    this.bitmapFold = bottomSource.querySelector('canvas.shapedInkCanvas') !== null;
    this.element.className = "bookFoldOverlay";
    delete this.element.dataset.artwork;
    if (this.bitmapFold) this.element.dataset.artwork = 'canvas';
    this.element.inert = true;
    this.element.setAttribute("aria-hidden", "true");
    this.element.dataset.direction = direction;
    this.element.dataset.turnId = String(++turnSequence);
    this.under = this.sheet(bottomSource, "bookFoldUnder", false, this.surface.under);
    this.back = this.sheet(backSource ?? bottomSource, "bookFoldBack", !backSource, this.surface.back);
    // Release old surplus artwork when a page has fewer lines or changes mode.
    for (const canvas of this.surface.ink) if (!this.inkCanvases.includes(canvas)) releaseInkCanvas(canvas, false);
    this.surface.ink = this.inkCanvases;
    this.shadow.className = "bookFoldShadow";
    this.shadow.removeAttribute('style');
    this.element.append(this.under, this.back, this.shadow);
    view.append(this.element);
    this.draw(0.001);
  }

  activate() {
    if (this.destroyed || this.active) return;
    this.active = true;
    this.element.classList.remove("bookFoldWarmup");
    this.view.classList.add("bookTurning");
  }

  prepare() {
    if (this.destroyed) return;
    this.element.classList.add("bookFoldWarmup");
  }

  private sheet(source: HTMLElement, className: string, blank: boolean, wrapper: HTMLElement) {
    wrapper.className = `bookFoldSheet ${className}`;
    wrapper.removeAttribute('style');
    wrapper.style.width = `${this.width}px`;
    wrapper.style.height = `${this.height}px`;
    const attributes = (node: Element, clone: Element) => {
      for (const attr of [...clone.attributes]) if (!node.hasAttribute(attr.name) || attr.name === 'id') clone.removeAttribute(attr.name);
      for (const attr of node.attributes) if (attr.name !== 'id' && clone.getAttribute(attr.name) !== attr.value) clone.setAttribute(attr.name, attr.value);
    };
    const copy = (node: Node, previous: Node | null, shaped = false, shallow = false): Node | null => {
      if (!(node instanceof Element)) {
        if (previous?.nodeType === node.nodeType) { if (previous.nodeValue !== node.nodeValue) previous.nodeValue = node.nodeValue; return previous; }
        return node.cloneNode(false);
      }
      if (shaped && node.classList.contains('ayahRun')) return null;
      if (node instanceof HTMLCanvasElement && node.classList.contains('shapedInkCanvas')) {
        if (!node.width || !node.height) return null;
        const ink = previous instanceof HTMLCanvasElement ? previous : acquireInkCanvas(node.width, node.height);
        attributes(node, ink);
        if (ink.width !== node.width) ink.width = node.width;
        if (ink.height !== node.height) ink.height = node.height;
        const context = ink.getContext('2d', {willReadFrequently:true});
        context?.setTransform(1,0,0,1,0,0);
        context?.clearRect(0,0,ink.width,ink.height);
        context?.drawImage(node,0,0);
        this.inkCanvases.push(ink);
        return ink;
      }
      const compatible = previous instanceof Element && previous.tagName === node.tagName && previous.namespaceURI === node.namespaceURI;
      const clone = compatible ? previous as Element : node.cloneNode(false) as Element;
      attributes(node, clone);
      const inShapedLine = shaped || node.classList.contains('shapedQuranLine');
      const oldChildren = [...clone.childNodes], nextChildren: Node[] = [];
      if (!shallow) for (const child of node.childNodes) {
        // Select by output index: invisible ayah subtrees allocate no node.
        const result = copy(child, oldChildren[nextChildren.length] ?? null, inShapedLine);
        if (result) nextChildren.push(result);
      }
      // Avoid removing/reinserting a complete unchanged subtree between turns.
      for (let index=0; index<nextChildren.length; index++) {
        if (clone.childNodes[index] !== nextChildren[index]) clone.insertBefore(nextChildren[index], clone.childNodes[index] ?? null);
      }
      while (clone.childNodes.length > nextChildren.length) clone.lastChild!.remove();
      return clone;
    };
    const face = copy(source, wrapper.firstChild, false, blank) as HTMLElement;
    face.removeAttribute('aria-label'); face.removeAttribute('id');
    if (wrapper.firstChild !== face) wrapper.replaceChildren(face);
    return wrapper;
  }

  private localPolygon(points: (Point | null)[], position: Point, angle: number, dx = 0, dy = 0) {
    const cos = Math.cos(angle);
    const sin = Math.sin(angle);
    return `polygon(${points.filter((point): point is Point => point !== null).map((point) => {
      const x = this.sign * (point.x - position.x);
      const y = point.y - position.y;
      return `${x * cos + y * sin + dx}px ${y * cos - x * sin + dy}px`;
    }).join(",")})`;
  }

  private paint(sheet: HTMLElement, position: Point, angle: number, points: (Point | null)[]) {
    sheet.style.clipPath = this.localPolygon(points, position, angle);
    const move = this.bitmapFold
      ? `translate(${this.spine + this.sign * position.x}px, ${position.y}px)`
      : `translate3d(${this.spine + this.sign * position.x}px, ${position.y}px, 0)`;
    sheet.style.transform = `${move} rotate(${angle}rad)`;
  }

  draw(progress: number) {
    if (this.destroyed) return;
    const p = Math.max(0.001, Math.min(0.9999, progress));
    const lift = Math.sin(Math.PI * p) * Math.min(this.width * 0.26, this.height * 0.18);
    if (!this.calc.calc({ x: this.width * (1 - 2 * p), y: this.corner === "top" ? lift : this.height - lift })) return;
    this.element.dataset.progress = p.toFixed(3);
    this.paint(this.under, this.calc.getBottomPagePosition(), 0, this.calc.getBottomClipArea());
    this.paint(this.back, this.calc.getActiveCorner(), this.calc.getAngle(), this.calc.getFlippingClipArea());

    const position = this.calc.getShadowStartPoint();
    if (!position) { this.shadow.style.opacity = "0"; return; }
    const size = 32;
    const translate = this.sign < 0 ? size : 0;
    const angle = this.calc.getShadowAngle() + 3 * Math.PI / 2;
    this.shadow.style.width = `${size}px`;
    this.shadow.style.height = `${this.height * 2}px`;
    this.shadow.style.transformOrigin = `${translate}px 100px`;
    const move = this.bitmapFold
      ? `translate(${this.spine + this.sign * position.x - translate}px, ${position.y - 100}px)`
      : `translate3d(${this.spine + this.sign * position.x - translate}px, ${position.y - 100}px, 0)`;
    this.shadow.style.transform = `${move} rotate(${angle}rad)`;
    this.shadow.style.clipPath = this.localPolygon(this.pageRect, position, angle, translate, 100);
    this.shadow.style.opacity = String(Math.sin(Math.PI * p) * 0.24);
  }

  destroy() {
    if (this.destroyed) return;
    this.destroyed = true;
    this.element.remove();
    if (this.active) this.view.classList.remove('bookTurning');
    retainSurface(this.view, this.surface);
  }
}
