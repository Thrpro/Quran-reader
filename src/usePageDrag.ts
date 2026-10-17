import { useEffect, useLayoutEffect, useRef, type RefObject } from "react";
import { BookTurnLayer } from "./bookTurnLayer";
import { getDragPageTurn, type PageTurn } from "./pageGesture";
import {
  enqueuePageTurn,
  releasePageTurnSource,
  type PageTurnSource,
  type QueuedPageTurn,
} from "./pageTurnQueue";

type Options = {
  viewport: RefObject<HTMLElement | null>;
  previews: RefObject<HTMLDivElement | null>;
  pageKey: string;
  layoutKey: string;
  page: number;
  spread: boolean;
  enabled: boolean;
  commandsEnabled: boolean;
  canGoNext: boolean;
  canGoPrevious: boolean;
  onTurn: (direction: PageTurn) => void;
};

type Gesture = {
  pointer: number; x: number; y: number; start: number; width: number;
  page: HTMLElement; view: HTMLElement; horizontal: boolean; vertical: boolean;
  dx: number; dy: number; direction: PageTurn | null; progress: number;
};

const MAX_QUEUED_TURNS = 10;

export function usePageDrag(options: Options) {
  const latest = useRef(options);
  latest.current = options;
  const gesture = useRef<Gesture | null>(null);
  const layer = useRef<BookTurnLayer | null>(null);
  const frame = useRef(0);
  const busy = useRef(false);
  const pending = useRef<string | null>(null);
  const queued = useRef<QueuedPageTurn[]>([]);
  const warmLayer = useRef<BookTurnLayer | null>(null);
  const warmFrame = useRef(0);
  const warmExpiry = useRef(0);
  const warmDirection = useRef<PageTurn | null>(null);
  const warmPageKey = useRef<string | null>(null);
  const warmRetry = useRef(0);
  const warmAttempts = useRef(0);
  const warmUsable = useRef(false);
  const warmReady = useRef(false);
  const firstWarmTurn = useRef<PageTurn | null>(null);
  const prewarm = useRef<(() => void) | null>(null);
  const requestTurn = useRef<((direction: PageTurn, repeated: boolean, source: PageTurnSource) => void) | null>(null);
  const drainQueue = useRef<() => void>(() => {});

  function release() {
    const current = gesture.current;
    gesture.current = null;
    if (!current) return;
    current.page.style.removeProperty("user-select");
    if (latest.current.viewport.current?.hasPointerCapture(current.pointer)) {
      latest.current.viewport.current.releasePointerCapture(current.pointer);
    }
  }

  function cancel(clearQueue = true) {
    release();
    cancelAnimationFrame(frame.current);
    frame.current = 0;
    layer.current?.destroy();
    layer.current = null;
    cancelAnimationFrame(warmFrame.current);
    window.clearTimeout(warmExpiry.current);
    window.clearTimeout(warmRetry.current);
    warmFrame.current = 0;
    warmExpiry.current = 0;
    warmLayer.current?.destroy();
    warmLayer.current = null;
    warmDirection.current = null;
    warmPageKey.current = null;
    warmUsable.current = false;
    warmReady.current = false;
    firstWarmTurn.current = null;
    busy.current = false;
    pending.current = null;
    if (clearQueue) queued.current = [];
  }

  useLayoutEffect(() => {
    // Keep the final fold visible until React has painted the destination pages.
    if (pending.current === options.pageKey && !options.enabled) return;
    const continueQueuedTurns = pending.current === options.pageKey && options.enabled;
    cancel(!continueQueuedTurns);
    if (continueQueuedTurns && queued.current.length > 0) {
      frame.current = requestAnimationFrame(() => {
        frame.current = 0;
        drainQueue.current();
      });
    }
  }, [options.pageKey, options.layoutKey, options.enabled]);

  useEffect(() => {
    const viewport = options.viewport.current;
    if (!viewport) return;
    const activeViewport = viewport;
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
    const canTurn = (direction: PageTurn) => direction === "next" ? latest.current.canGoNext : latest.current.canGoPrevious;

    function createLayer(current: Gesture, direction: PageTurn, active = true) {
      const { page, spread, previews } = latest.current;
      const target = page + (direction === "next" ? 1 : -1) * (spread ? 2 : 1);
      const bottom = spread && direction === "next" ? target + 1 : target;
      const back = spread ? (direction === "next" ? target : target + 1) : null;
      const source = (number: number) => previews.current?.querySelector<HTMLElement>('[data-preview-page="' + number + '"] > .qulPage');
      const bottomSource = source(bottom);
      const backSource = back === null ? null : source(back);
      if (!bottomSource || (back !== null && !backSource)) return null;
      const bounds = current.page.getBoundingClientRect();
      const created = new BookTurnLayer(current.view, current.page.offsetWidth, current.page.offsetHeight,
        direction, current.y - bounds.top < bounds.height / 2 ? "top" : "bottom",
        bottomSource, backSource ?? null, spread);
      if (active) created.activate();
      else created.prepare();
      return created;
    }

    function paintDrag(current: Gesture) {
      const direction = current.dx > 0 ? "next" : "previous";
      if (direction !== current.direction) {
        layer.current?.destroy();
        layer.current = null;
        current.direction = direction;
      }
      if (!canTurn(direction) || reducedMotion.matches) return;
      layer.current ??= createLayer(current, direction);
      current.progress = Math.min(0.92, Math.abs(current.dx) / (current.width * 1.35));
      layer.current?.draw(current.progress);
    }

    function down(event: PointerEvent) {
      if (!event.isPrimary) { cancel(); return; }
      if (!latest.current.enabled || event.button !== 0 || event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return;
      const target = event.target;
      if (!(target instanceof HTMLElement) || target.closest("button, input, select, textarea, a, [contenteditable=true]")) return;
      const page = target.closest<HTMLElement>(".pageSlot > .qulPage");
      const view = page?.closest<HTMLElement>(".spreadView, .singleView");
      if (page && busy.current) { event.preventDefault(); return; }
      if (!page || !view || !window.getSelection()?.isCollapsed) return;
      if (event.pointerType === "mouse") event.preventDefault();
      cancel();
      page.style.userSelect = "none";
      gesture.current = { pointer: event.pointerId, x: event.clientX, y: event.clientY,
        start: event.timeStamp, width: page.getBoundingClientRect().width, page, view,
        horizontal: false, vertical: false, dx: 0, dy: 0, direction: null, progress: 0 };
      viewport!.setPointerCapture(event.pointerId);
    }

    function move(event: PointerEvent) {
      const current = gesture.current;
      if (!current || current.pointer !== event.pointerId) return;
      current.dx = event.clientX - current.x;
      current.dy = event.clientY - current.y;
      if (current.vertical) return;
      if (!current.horizontal) {
        if (Math.max(Math.abs(current.dx), Math.abs(current.dy)) < 8) return;
        if (Math.abs(current.dy) > Math.abs(current.dx)) { current.vertical = true; return; }
        current.horizontal = true;
      }
      event.preventDefault();
      if (!frame.current) frame.current = requestAnimationFrame(() => {
        frame.current = 0;
        if (gesture.current === current) paintDrag(current);
      });
    }

    function up(event: PointerEvent) {
      const current = gesture.current;
      if (!current || current.pointer !== event.pointerId) return;
      cancelAnimationFrame(frame.current);
      frame.current = 0;
      current.dx = event.clientX - current.x;
      current.dy = event.clientY - current.y;
      const direction = current.horizontal ? getDragPageTurn(current.dx, current.dy, event.timeStamp - current.start, current.width) : null;
      if (current.horizontal) paintDrag(current);
      const accepted = direction !== null && canTurn(direction);
      release();
      settle(direction, accepted, current.progress);
    }

    function settle(direction: PageTurn | null, accepted: boolean, from: number) {
      const fromKey = latest.current.pageKey;
      const commit = () => {
        if (latest.current.pageKey !== fromKey || !latest.current.commandsEnabled) { cancel(); return; }
        if (!accepted || !direction) { cancel(); return; }
        pending.current = (latest.current.spread ? "spread:" : "single:") + (latest.current.page + (direction === "next" ? 1 : -1) * (latest.current.spread ? 2 : 1));
        latest.current.onTurn(direction);
      };
      busy.current = accepted;
      if (!layer.current || reducedMotion.matches) { commit(); return; }
      let start = 0;
      const destination = accepted ? 1 : 0;
      const duration = accepted
        ? Math.max(130, (queued.current.length > 0 ? 175 : 285) * (1 - from))
        : 140;
      const tick = (now: number) => {
        frame.current = 0;
        if (start === 0) start = now;
        const elapsed = Math.min(1, (now - start) / duration);
        const eased = 1 - (1 - elapsed) ** 3;
        layer.current?.draw(from + (destination - from) * eased);
        if (elapsed < 1) frame.current = requestAnimationFrame(tick);
        else commit();
      };
      if (from <= 0.001 && layer.current) {
        // Let the newly cloned fold faces rasterize before their first visible movement.
        frame.current = requestAnimationFrame(() => {
          frame.current = requestAnimationFrame(tick);
        });
      } else {
        frame.current = requestAnimationFrame(tick);
      }
    }

    function startTurn(direction: PageTurn, previewAttempts = 0) {
      if (!canTurn(direction)) return;
      const prepared = warmDirection.current === direction && warmPageKey.current === latest.current.pageKey
        ? warmLayer.current
        : null;
      if (prepared) {
        cancelAnimationFrame(warmFrame.current);
        window.clearTimeout(warmExpiry.current);
        warmFrame.current = 0;
        warmExpiry.current = 0;
        warmLayer.current = null;
        warmDirection.current = null;
        warmPageKey.current = null;
      }
      cancel(false);
      if (!latest.current.enabled || reducedMotion.matches) {
        settle(direction, true, 0);
        return;
      }
      const page = activeViewport.querySelector<HTMLElement>(direction === "next"
        ? ".leftSlot > .qulPage" : ".rightSlot > .qulPage")
        ?? activeViewport.querySelector<HTMLElement>(".pageSlot > .qulPage");
      const view = page?.closest<HTMLElement>(".spreadView, .singleView");
      if (prepared) {
        layer.current = prepared;
        layer.current.draw(0.001);
        layer.current.activate();
      } else if (page && view) {
        const bounds = page.getBoundingClientRect();
        layer.current = createLayer({ pointer: -1, x: bounds.x, y: bounds.bottom - 1,
          start: 0, width: bounds.width, page, view, horizontal: true, vertical: false,
          dx: 0, dy: 0, direction, progress: 0 }, direction);
      }
      if (!layer.current && previewAttempts < 10) {
        busy.current = true;
        frame.current = requestAnimationFrame(() => {
          frame.current = 0;
          busy.current = false;
          startTurn(direction, previewAttempts + 1);
        });
        return;
      }
      settle(direction, true, 0);
    }

    prewarm.current = () => {
      if (warmLayer.current || busy.current || pending.current ||
          !latest.current.enabled || reducedMotion.matches) return;
      const direction: PageTurn = canTurn("next") ? "next" : "previous";
      if (!canTurn(direction)) return;
      const page = activeViewport.querySelector<HTMLElement>(direction === "next"
        ? ".leftSlot > .qulPage" : ".rightSlot > .qulPage")
        ?? activeViewport.querySelector<HTMLElement>(".pageSlot > .qulPage");
      const view = page?.closest<HTMLElement>(".spreadView, .singleView");
      if (!page || !view) return;
      const bounds = page.getBoundingClientRect();
      const prepared = createLayer({ pointer: -1, x: bounds.x, y: bounds.bottom - 1,
        start: 0, width: bounds.width, page, view, horizontal: true, vertical: false,
        dx: 0, dy: 0, direction, progress: 0 }, direction, false);
      if (!prepared) {
        // Preview pages can mount after the initial enabled effect. Retry only
        // during this short preparation window, rather than missing warmup.
        if (warmAttempts.current++ < 12) warmRetry.current = window.setTimeout(() => prewarm.current?.(), 50);
        return;
      }
      warmAttempts.current = 0;
      warmLayer.current = prepared;
      warmDirection.current = direction;
      warmPageKey.current = latest.current.pageKey;
      warmUsable.current = false;
      warmReady.current = false;
      const stages = [0.22, 0.52, 0.78];
      let stage = 0;
      let previousFrame = 0;
      let stableFrames = 0;
      let settlingFrames = 0;
      const warm = (now: number) => {
        warmFrame.current = 0;
        if (!warmLayer.current) return;
        if (stage < stages.length) {
          warmLayer.current.draw(stages[stage++]);
          if (stage === stages.length) {
            warmUsable.current = true;
            const first = firstWarmTurn.current;
            firstWarmTurn.current = null;
            if (first) {
              startTurn(first);
              return;
            }
          }
          warmFrame.current = requestAnimationFrame(warm);
        } else {
          // Let the first compositor/raster work finish before accepting the
          // prepared fold. This finite warmup never leaves an idle frame loop.
          stableFrames = previousFrame && now - previousFrame < 25 ? stableFrames + 1 : 0;
          if (stableFrames < 4 && settlingFrames++ < 24) warmFrame.current = requestAnimationFrame(warm);
          else {
            warmReady.current = true;
            // Keep a short readiness window for the next turn, then release
            // cloned pages/compositor surfaces instead of retaining them idle.
            warmExpiry.current = window.setTimeout(() => {
              warmExpiry.current = 0;
              warmLayer.current?.destroy();
              warmLayer.current = null;
              warmDirection.current = null;
              warmPageKey.current = null;
              warmUsable.current = false;
              warmReady.current = false;
            }, 1500);
            drainQueue.current();
          }
        }
        previousFrame = now;
      };
      warmFrame.current = requestAnimationFrame(warm);
    };

    drainQueue.current = () => {
      if (busy.current || pending.current || !latest.current.enabled || (warmLayer.current && !warmReady.current)) return;
      const turn = queued.current.shift();
      if (!turn) return;
      if (!canTurn(turn.direction)) { drainQueue.current(); return; }
      startTurn(turn.direction);
    };

    requestTurn.current = (direction, repeated, source) => {
      if (!canTurn(direction)) return;
      if (!busy.current && !pending.current && warmLayer.current && !warmReady.current) {
        if (warmUsable.current) startTurn(direction);
        else firstWarmTurn.current ??= direction;
        return;
      }
      if (busy.current || pending.current || (warmLayer.current && !warmReady.current)) {
        enqueuePageTurn(queued.current, { direction, repeated, source }, MAX_QUEUED_TURNS);
        return;
      }
      startTurn(direction);
    };

    function lostCapture() { if (gesture.current) cancel(); }
    function hidden() { if (document.hidden) cancel(); }
    function cancelInteraction() { cancel(); }
    function prepareAfterLayout() {
      cancel();
      warmAttempts.current = 0;
      warmRetry.current = window.setTimeout(() => prewarm.current?.(), 120);
    }
    viewport.addEventListener("pointerdown", down);
    viewport.addEventListener("pointermove", move, { passive: false });
    viewport.addEventListener("pointerup", up);
    viewport.addEventListener("pointercancel", cancelInteraction);
    viewport.addEventListener("lostpointercapture", lostCapture);
    window.addEventListener("blur", cancelInteraction);
    window.addEventListener("resize", prepareAfterLayout);
    window.addEventListener("focus", prepareAfterLayout);
    document.addEventListener("visibilitychange", hidden);
    reducedMotion.addEventListener("change", cancelInteraction);
    systemTheme.addEventListener("change", cancelInteraction);
    return () => {
      requestTurn.current = null;
      prewarm.current = null;
      drainQueue.current = () => {};
      cancel();
      viewport.removeEventListener("pointerdown", down);
      viewport.removeEventListener("pointermove", move);
      viewport.removeEventListener("pointerup", up);
      viewport.removeEventListener("pointercancel", cancelInteraction);
      viewport.removeEventListener("lostpointercapture", lostCapture);
      window.removeEventListener("blur", cancelInteraction);
      window.removeEventListener("resize", prepareAfterLayout);
      window.removeEventListener("focus", prepareAfterLayout);
      document.removeEventListener("visibilitychange", hidden);
      reducedMotion.removeEventListener("change", cancelInteraction);
      systemTheme.removeEventListener("change", cancelInteraction);
    };
  }, [options.viewport]);

  useEffect(() => {
    if (!options.enabled) return;
    // Fast navigation cancels speculative rasterization before it starts.
    // Prepare a fold only after the reader has rested on the current spread.
    const timer = window.setTimeout(() => prewarm.current?.(), 600);
    return () => window.clearTimeout(timer);
  }, [options.enabled, options.pageKey, options.layoutKey]);
  return {
    turnPage(direction: PageTurn, repeated = false, source: PageTurnSource = "discrete") {
      const handler = requestTurn.current;
      if (handler) handler(direction, repeated, source);
      else if (latest.current.commandsEnabled) latest.current.onTurn(direction);
    },
    releaseKeyboardTurns() {
      queued.current = releasePageTurnSource(queued.current, "keyboard");
    },
  };
}
