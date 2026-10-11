// Pinned StPageFlip geometry entry point; Vite compiles the upstream TypeScript.
declare module "page-flip/geometry" {
  type Point = { x: number; y: number };
  export class FlipCalculation {
    constructor(direction: 0 | 1, corner: "top" | "bottom", width: string, height: string);
    calc(point: Point): boolean;
    getFlippingClipArea(): (Point | null)[];
    getBottomClipArea(): (Point | null)[];
    getAngle(): number;
    getActiveCorner(): Point;
    getBottomPagePosition(): Point;
    getShadowStartPoint(): Point | null;
    getShadowAngle(): number;
  }
}
