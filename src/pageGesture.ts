export type PageTurn = "next" | "previous";

export function getDragPageTurn(dx: number, dy: number, elapsedMs: number, pageWidth: number): PageTurn | null {
  const distance = Math.abs(dx);
  if (distance < 24 || distance < Math.abs(dy) * 1.25) return null;
  const threshold = Math.max(40, Math.min(96, pageWidth * 0.12));
  const isFlick = elapsedMs > 0 && distance / elapsedMs >= 0.5;
  if (distance < threshold && !isFlick) return null;
  return dx > 0 ? "next" : "previous";
}
