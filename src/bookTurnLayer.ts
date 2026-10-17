import { FlipCalculation } from "page-flip/geometry";
import type { PageTurn } from "./pageGesture";
import { acquireInkCanvas, releaseInkCanvas } from "./inkCanvasPool";

type Point = { x: number; y: number };

export class BookTurnLayer {
  readonly element = document.createElement("div");
  private readonly under: HTMLElement;
  private readonly back: HTMLElement;
  private readonly shadow = document.createElement("div");
  private readonly calc: FlipCalculation;
  private readonly sign: number;
  private readonly spine: number;
  private readonly pageRect: Point[];
  private active = false;
  private readonly inkCanvases: HTMLCanvasElement[] = [];

  constructor(private readonly view: HTMLElement, private readonly width: number,
    private readonly height: number, direction: PageTurn, private readonly corner: "top" | "bottom",
    bottomSource: HTMLElement, backSource: HTMLElement | null, spread: boolean) {
    this.sign = direction === "next" ? -1 : 1;
    this.spine = spread || direction === "next" ? width : 0;
    this.pageRect = [
      { x: 0, y: 0 },
      { x: width, y: 0 },
      { x: width, y: height },
      { x: 0, y: height },
    ];
    this.calc = new FlipCalculation(direction === "next" ? 1 : 0, corner, String(width), String(height));
    this.element.className = "bookFoldOverlay";
    this.element.inert = true;
    this.element.setAttribute("aria-hidden", "true");
    this.element.dataset.direction = direction;
    this.under = this.sheet(bottomSource, "bookFoldUnder");
    this.back = this.sheet(backSource ?? bottomSource, "bookFoldBack", !backSource);
    this.shadow.className = "bookFoldShadow";
    this.element.append(this.under, this.back, this.shadow);
    view.append(this.element);
    this.draw(0.001);
  }

  activate() {
    if (this.active) return;
    this.active = true;
    this.element.classList.remove("bookFoldWarmup");
    this.view.classList.add("bookTurning");
  }

  prepare() {
    this.element.classList.add("bookFoldWarmup");
  }

  private sheet(source: HTMLElement, className: string, blank = false) {
    const wrapper = document.createElement("div");
    wrapper.className = `bookFoldSheet ${className}`;
    wrapper.style.width = `${this.width}px`;
    wrapper.style.height = `${this.height}px`;
    const face = source.cloneNode(true) as HTMLElement;
    // cloneNode copies canvas dimensions, not its pixels. Copy the verified
    // artwork once per fold; rotating the face then needs no glyph rendering.
    const sourceInk = source.querySelectorAll<HTMLCanvasElement>("canvas.shapedInkCanvas");
    face.querySelectorAll<HTMLCanvasElement>("canvas.shapedInkCanvas").forEach((placeholder, index) => {
      const original = sourceInk[index];
      if (blank || !original?.width || !original.height) { placeholder.remove(); return; }
      const ink = acquireInkCanvas(original.width, original.height);
      ink.className = original.className;
      ink.style.cssText = original.style.cssText;
      ink.getContext("2d", { willReadFrequently: true })?.drawImage(original, 0, 0);
      placeholder.replaceWith(ink);
      this.inkCanvases.push(ink);
    });
    face.removeAttribute("aria-label");
    for (const element of face.querySelectorAll("[id]")) element.removeAttribute("id");
    // Fold faces are inert artwork. Shaped lines keep source text in invisible
    // interaction spans; cloning it adds hundreds of unnecessary DOM nodes.
    for (const span of face.querySelectorAll(".shapedQuranLine .ayahRun")) span.remove();
    if (blank) face.replaceChildren();
    wrapper.append(face);
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
    sheet.style.transform = `translate3d(${this.spine + this.sign * position.x}px, ${position.y}px, 0) rotate(${angle}rad)`;
  }

  draw(progress: number) {
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
    this.shadow.style.transform = `translate3d(${this.spine + this.sign * position.x - translate}px, ${position.y - 100}px, 0) rotate(${angle}rad)`;
    this.shadow.style.clipPath = this.localPolygon(this.pageRect, position, angle, translate, 100);
    this.shadow.style.opacity = String(Math.sin(Math.PI * p) * 0.24);
  }

  destroy() {
    this.element.remove();
    for (const ink of this.inkCanvases) releaseInkCanvas(ink);
    this.inkCanvases.length = 0;
    this.under.replaceChildren();
    this.back.replaceChildren();
    this.element.replaceChildren();
    if (this.active) this.view.classList.remove("bookTurning");
  }
}
