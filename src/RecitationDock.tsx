import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChevronDown, ChevronUp, Headphones, Pause, Pin, Play, RotateCcw, RotateCw, SlidersHorizontal, Square } from "lucide-react";

type Props = {
  placement?: "bottom" | "side";
  suspended?: boolean; compactOnly?: boolean; compactHostId?: string;
  arabic: boolean; playing: boolean; title: string; subtitle: string;
  elapsed: number; duration: number; minimum: number; maximum: number;
  onToggle: () => void; onSeek: (seconds: number) => void; onSettings: () => void; onStop: () => void;
  onSpaceChange: (height: number) => void;
  onSideSpaceChange?: (width: number) => void;
};
function duration(seconds: number) {
  const value = Math.max(0, Math.floor(seconds)), hours = Math.floor(value / 3600);
  return `${hours ? `${hours}:` : ""}${hours ? String(Math.floor(value / 60) % 60).padStart(2, "0") : Math.floor(value / 60)}:${String(value % 60).padStart(2, "0")}`;
}
const COMPACT_SPACE = 36;
const HIDE_DELAY = 3200;
const SIDE_SPACE = 198;

/** Side controls use existing space only. Bottom layout changes reserve once;
 * neither mode watches glyph mutations or measures on playback-clock updates. */
export function RecitationDock(props: Props) {
  const { placement = "bottom", suspended = false, compactOnly = false, compactHostId, arabic, playing, title, subtitle, elapsed, duration: total, minimum, maximum,
    onToggle, onSeek, onSettings, onStop, onSpaceChange, onSideSpaceChange } = props;
  const text = (english: string, translated: string) => arabic ? translated : english;
  const sideMode = placement === "side" || compactOnly;
  const [host, setHost] = useState<HTMLElement | null>(null);
  const [expanded, setExpanded] = useState(true);
  const [pinned, setPinned] = useState(false);
  const [popover, setPopover] = useState(false);
  const [gutter, setGutter] = useState({ roomy: false, side: arabic ? "right" : "left" });
  const layout = !compactOnly && gutter.roomy && expanded ? "card" : popover ? "popover" : "compact";
  const panelVisible = !suspended && (sideMode ? layout === "popover" || layout === "card" && expanded : expanded);
  const compactVisible = sideMode && !suspended && !panelVisible;
  useLayoutEffect(() => { setHost(compactHostId ? document.getElementById(compactHostId) : null); }, [compactHostId]);
  const root = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const timer = useRef(0);
  const pointerInside = useRef(false);
  const pointerDown = useRef(false);
  const restoreFocus = useRef(false);
  const focusPanel = useRef(false);
  const keyboardFocus = useRef(false);
  const suppressHover = useRef(false);
  const syncSideSurface = useRef<(() => void) | null>(null);
  const current = useRef({ playing, pinned, expanded, suspended, sideMode });
  current.current = { playing, pinned, expanded, suspended, sideMode };
  const clearHide = useCallback(() => { window.clearTimeout(timer.current); timer.current = 0; }, []);
  const turning = () => !!document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup), .readerViewport .bookTurning');
  const scheduleHide = useCallback(() => {
    clearHide();
    const hide = () => {
      timer.current = 0;
      if (current.current.sideMode || current.current.suspended || !current.current.playing || current.current.pinned || !current.current.expanded) return;
      if (pointerInside.current || keyboardFocus.current && root.current?.contains(document.activeElement)) return;
      if (pointerDown.current || turning()) { timer.current = window.setTimeout(hide, 400); return; }
      if (root.current?.contains(document.activeElement)) document.querySelector<HTMLElement>('main.appShell')?.focus({ preventScroll: true });
      setExpanded(false);
    };
    if (!current.current.sideMode && !current.current.suspended && current.current.playing && !current.current.pinned && current.current.expanded)
      timer.current = window.setTimeout(hide, HIDE_DELAY);
  }, [clearHide]);
  const reveal = () => {
    if (pointerDown.current || turning()) return;
    clearHide(); setExpanded(true);
  };
  function showSide() {
    if (turning()) return;
    focusPanel.current = true; setExpanded(true);
    if (compactOnly || !gutter.roomy) setPopover(true);
  }
  function collapse() {
    clearHide(); suppressHover.current = true; restoreFocus.current = true; setPopover(false);
    if (!sideMode || layout !== "popover") setExpanded(false);
  }
  useLayoutEffect(() => {
    if (panelVisible && focusPanel.current) {
      focusPanel.current = false;
      panel.current?.querySelector<HTMLButtonElement>('.primaryPlayback')?.focus({ preventScroll: true });
    }
    if (!panelVisible && !suspended && restoreFocus.current) {
      trigger.current?.focus({ preventScroll: true }); restoreFocus.current = false;
    }
  }, [panelVisible, suspended]);
  useLayoutEffect(() => {
    const element = panel.current;
    const measure = () => onSpaceChange(suspended || sideMode ? 0 : expanded && element ? Math.ceil(element.offsetHeight) + 16 : COMPACT_SPACE);
    measure();
    const observer = new ResizeObserver(measure);
    if (element && !suspended && !sideMode) observer.observe(element);
    return () => observer.disconnect();
  }, [sideMode, suspended, expanded, onSpaceChange]);
  useLayoutEffect(() => { onSideSpaceChange?.(0); }, [onSideSpaceChange]);
  useLayoutEffect(() => {
    if (!sideMode || suspended) return;
    let surface: HTMLElement | null = null;
    const viewport = document.querySelector<HTMLElement>(".readerViewport");
    const measure = () => {
      if (!surface || !viewport) return;
      const bounds = viewport.getBoundingClientRect();
      // Layout offsets exclude App's temporary FLIP animation. A conservative
      // 16px allowance also covers the reader's small resting x translation.
      const left = bounds.left + surface.offsetLeft;
      const right = left + surface.offsetWidth;
      const leftGap = Math.max(0, left - bounds.left - 16), rightGap = Math.max(0, bounds.right - right - 16);
      setGutter(previous => {
        const side = leftGap > rightGap + 8 ? "left" : rightGap > leftGap + 8 ? "right" : arabic ? "right" : "left";
        const roomy = Math.max(leftGap, rightGap) >= SIDE_SPACE + (previous.roomy ? -1 : 1);
        return previous.roomy === roomy && previous.side === side ? previous : { roomy, side };
      });
      const edge = gutter.side === "right" ? window.innerWidth - bounds.right : bounds.left;
      root.current?.style.setProperty("--recitation-side-inset", `${Math.max(0, edge)}px`);
      root.current?.style.setProperty("--recitation-side-bottom", `${Math.max(18, window.innerHeight - bounds.bottom + 8)}px`);
    };
    const observer = new ResizeObserver(measure);
    const sync = () => {
      const next = viewport?.querySelector<HTMLElement>(".readerScaleSurface") ?? null;
      if (next === surface) return;
      if (surface) observer.unobserve(surface);
      surface = next;
      if (surface) observer.observe(surface);
      measure();
    };
    syncSideSurface.current = sync;
    if (viewport) observer.observe(viewport);
    sync();
    window.addEventListener("resize", measure);
    return () => { syncSideSurface.current = null; observer.disconnect(); window.removeEventListener("resize", measure); };
  }, [sideMode, suspended, arabic, gutter.side]);
  useLayoutEffect(() => { syncSideSurface.current?.(); });
  useLayoutEffect(() => () => { onSpaceChange(0); onSideSpaceChange?.(0); }, [onSpaceChange, onSideSpaceChange]);
  useEffect(() => { setPinned(false); setExpanded(true); setPopover(false); }, [placement]);
  useEffect(() => { if (!sideMode) setExpanded(true); }, [playing, sideMode]);
  useEffect(() => { setPopover(false); }, [suspended, compactOnly, compactHostId, gutter.roomy]);
  useEffect(() => { scheduleHide(); return clearHide; }, [playing, pinned, expanded, suspended, sideMode, scheduleHide, clearHide]);
  useEffect(() => {
    if (!sideMode || !popover || suspended) return;
    const outside = (event: PointerEvent) => { if (event.target instanceof Node && !root.current?.contains(event.target)) setPopover(false); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); collapse(); } };
    document.addEventListener("pointerdown", outside, true); document.addEventListener("keydown", escape, true);
    return () => { document.removeEventListener("pointerdown", outside, true); document.removeEventListener("keydown", escape, true); };
  }, [sideMode, popover, suspended]);
  useEffect(() => {
    const down = () => { pointerDown.current = true; keyboardFocus.current = false; };
    const key = () => { keyboardFocus.current = true; };
    const up = () => { pointerDown.current = false; scheduleHide(); };
    const blur = () => { pointerDown.current = false; pointerInside.current = false; scheduleHide(); };
    window.addEventListener('pointerdown', down, true); window.addEventListener('pointerup', up, true);
    window.addEventListener('pointercancel', up, true); window.addEventListener('keydown', key, true); window.addEventListener('blur', blur);
    return () => { clearHide(); window.removeEventListener('pointerdown', down, true); window.removeEventListener('pointerup', up, true);
      window.removeEventListener('pointercancel', up, true); window.removeEventListener('keydown', key, true); window.removeEventListener('blur', blur); };
  }, [scheduleHide, clearHide]);
  const compact = compactVisible ? <div className="sidePlayerCompact" dir={arabic ? "rtl" : "ltr"}>
    <button className="primaryPlayback" onClick={onToggle} aria-label={playing ? text("Pause", "إيقاف مؤقت") : text("Resume", "متابعة")} title={title}>{playing ? <Pause size={17} /> : <Play size={17} />}</button>
    <button ref={trigger} onClick={showSide} aria-label={text("Show recitation controls", "إظهار أدوات التلاوة")} aria-expanded={panelVisible} aria-controls="recitation-player-controls"><SlidersHorizontal size={17} /></button>
  </div> : null;
  return <>
    {compact ? host ? createPortal(compact, host) : <div className="sidePlayerCompactFloating" data-side={gutter.side}>{compact}</div> : null}
    <div ref={root} className="recitationDock" dir={arabic ? "rtl" : "ltr"} data-placement={sideMode ? "side" : "bottom"} data-preference={placement} data-side={gutter.side} data-layout={sideMode ? layout : "bottom"} data-expanded={panelVisible} data-suspended={suspended}
      onPointerEnter={event => { pointerInside.current = true; if (!sideMode && !event.buttons && !suppressHover.current) reveal(); }}
      onPointerMove={event => { if (!sideMode && !expanded && !event.buttons && !suppressHover.current) reveal(); }}
      onPointerLeave={() => { pointerInside.current = false; suppressHover.current = false; scheduleHide(); }}
      onFocusCapture={() => {
        if (sideMode || restoreFocus.current) return;
        keyboardFocus.current = !pointerDown.current;
        if (document.activeElement === trigger.current && !expanded && keyboardFocus.current) focusPanel.current = true;
        reveal();
      }}
      onBlurCapture={event => { if (!event.currentTarget.contains(event.relatedTarget)) scheduleHide(); }}
      onKeyDown={event => { if (event.key === 'Escape' && panelVisible) { event.preventDefault(); event.stopPropagation(); collapse(); } }}>
      {!sideMode ? <button ref={trigger} className="recitationReveal" aria-label={text("Show recitation controls", "إظهار أدوات التلاوة")}
        aria-expanded={panelVisible} aria-controls="recitation-player-controls" tabIndex={expanded || suspended ? -1 : 0} disabled={suspended}
        onClick={reveal}><Headphones size={14} /><span>{playing ? text("Listening", "جارٍ الاستماع") : text("Paused", "متوقف مؤقتًا")}</span><ChevronUp size={14} /></button> : null}
      <div ref={panel} id="recitation-player-controls" className={`recitationMini ${sideMode ? "sideRecitationPlayer" : "bottomRecitationPlayer"}`} data-expanded={panelVisible}
        inert={!panelVisible} aria-hidden={!panelVisible} role="region" aria-label={text("Recitation controls", "أدوات التلاوة")}>
        <header className="miniRecording"><div><strong>{title}</strong><small>{subtitle}</small></div>
          {sideMode ? <button onClick={collapse} aria-label={text("Collapse player", "تصغير أدوات التلاوة")} title={text("Collapse player", "تصغير أدوات التلاوة")}><ChevronDown size={17} /></button> : null}</header>
        <div className="miniTransport" dir={arabic ? "rtl" : "ltr"}>
          <button disabled={!maximum} onClick={() => onSeek(elapsed - 10)} aria-label={text("Back 10 seconds", "رجوع ١٠ ثوانٍ")}><RotateCcw size={18} /><span>10</span></button>
          <button className="primaryPlayback" onClick={onToggle} aria-label={playing ? text("Pause", "إيقاف مؤقت") : text("Resume", "متابعة")}>{playing ? <Pause size={23} /> : <Play size={23} />}</button>
          <button disabled={!maximum} onClick={() => onSeek(elapsed + 10)} aria-label={text("Forward 10 seconds", "تقديم ١٠ ثوانٍ")}><span>10</span><RotateCw size={18} /></button>
        </div>
        <div className="miniActions">
          <button onClick={onSettings} aria-label={text("Listening settings", "إعدادات الاستماع")} title={text("Listening settings", "إعدادات الاستماع")}><SlidersHorizontal size={16} />{sideMode ? <span>{text("Settings", "الإعدادات")}</span> : null}</button>
          <button onClick={onStop} aria-label={text("Stop recitation", "إنهاء التلاوة")} title={text("Stop recitation", "إنهاء التلاوة")}><Square size={16} />{sideMode ? <span>{text("Stop", "إنهاء")}</span> : null}</button>
          {!sideMode ? <><button onClick={() => setPinned(value => !value)} aria-pressed={pinned} aria-label={text("Keep player visible", "إبقاء أدوات التلاوة ظاهرة")} title={text("Keep player visible", "إبقاء أدوات التلاوة ظاهرة")}><Pin size={16} /></button>
            <button onClick={collapse} aria-label={text("Collapse player", "تصغير أدوات التلاوة")} title={text("Collapse player", "تصغير أدوات التلاوة")}><ChevronDown size={17} /></button></> : null}
        </div>
        <div className="miniTimeline" dir={arabic ? "rtl" : "ltr"}><span className="miniElapsed" dir="ltr">{duration(elapsed)}</span>
          <input dir={arabic ? "rtl" : "ltr"} aria-label={text("Playback position", "موضع التشغيل")} aria-valuetext={duration(elapsed)} type="range" min={minimum} max={Math.max(minimum + .1, maximum)} step="1" value={Math.max(minimum, Math.min(maximum, elapsed))} disabled={!maximum} onChange={event => onSeek(Number(event.target.value))} />
          <span className="miniDuration" dir={total ? "ltr" : undefined}>{total ? duration(total) : text("Buffered", "جاهز للتشغيل")}</span>
        </div>
      </div>
    </div>
  </>;
}
