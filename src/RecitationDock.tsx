import { useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChevronDown, ChevronUp, Pause, Play, RotateCcw, RotateCw, SlidersHorizontal, Square } from "lucide-react";

type Props = {
  arabic: boolean; playing: boolean; title: string; subtitle: string;
  elapsed: number; duration: number; minimum: number; maximum: number;
  onToggle: () => void; onSeek: (seconds: number) => void; onSettings: () => void; onStop: () => void;
};
function duration(seconds: number) {
  const value = Math.max(0, Math.floor(seconds)), hours = Math.floor(value / 3600);
  return `${hours ? `${hours}:` : ""}${hours ? String(Math.floor(value / 60) % 60).padStart(2, "0") : Math.floor(value / 60)}:${String(value % 60).padStart(2, "0")}`;
}

/** Uses the existing page gutter. A narrow window gets a toolbar control and
 * an explicitly opened popover, never a reserved strip below the Mushaf. */
export function RecitationDock(props: Props) {
  const { arabic, playing, title, subtitle, elapsed, duration: total, minimum, maximum, onToggle, onSeek, onSettings, onStop } = props;
  const text = (english: string, translated: string) => arabic ? translated : english;
  const [placement, setPlacement] = useState<{ roomy: boolean; side: "left" | "right" }>({ roomy: false, side: arabic ? "right" : "left" });
  const [expanded, setExpanded] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const panel = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const restoreTrigger = useRef(false);
  useLayoutEffect(() => {
    let frame = 0, settling = 0;
    const observed = new Set<Element>();
    const resize = new ResizeObserver(() => schedule());
    const measure = () => {
      frame = 0;
      const elements = new Set(document.querySelectorAll(".readerViewport, .readerViewport .readerScaleSurface, .readerViewport .pageSlot > .qulPage"));
      for (const element of observed) if (!elements.has(element)) { resize.unobserve(element); observed.delete(element); }
      for (const element of elements) if (!observed.has(element)) { observed.add(element); resize.observe(element); }
      const pages = [...document.querySelectorAll<HTMLElement>(".readerViewport .pageSlot > .qulPage")].map(page => page.getBoundingClientRect());
      if (!pages.length) return;
      const left = Math.min(...pages.map(page => page.left)), right = innerWidth - Math.max(...pages.map(page => page.right));
      const side = left > right + 8 ? "left" : right > left + 8 ? "right" : arabic ? "right" : "left";
      const roomy = Math.max(left, right) >= 198;
      setPlacement(previous => previous.roomy === roomy && previous.side === side ? previous : { roomy, side });
    };
    const schedule = () => {
      cancelAnimationFrame(frame); frame = requestAnimationFrame(measure);
      clearTimeout(settling); settling = window.setTimeout(measure, 260);
    };
    const viewport = document.querySelector(".readerViewport");
    const mutations = new MutationObserver(schedule); if (viewport) mutations.observe(viewport, { childList: true, subtree: true, attributes: true, attributeFilter: ["style", "class"] });
    window.addEventListener("resize", schedule); schedule();
    return () => { cancelAnimationFrame(frame); clearTimeout(settling); resize.disconnect(); mutations.disconnect(); window.removeEventListener("resize", schedule); };
  }, [arabic]);
  const showPanel = expanded || (placement.roomy && !collapsed);
  useLayoutEffect(() => {
    if (!showPanel && restoreTrigger.current) {
      restoreTrigger.current = false;
      trigger.current?.focus({ preventScroll: true });
    }
  }, [showPanel]);
  const collapse = () => { restoreTrigger.current = true; setExpanded(false); setCollapsed(true); };
  useLayoutEffect(() => {
    if (!expanded) return;
    panel.current?.querySelector<HTMLButtonElement>('button')?.focus({ preventScroll: true });
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") { event.stopPropagation(); collapse(); } };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [expanded]);
  const compact = <div className="sidePlayerCompact" dir={arabic ? "rtl" : "ltr"}>
    <button className="primaryPlayback" onClick={onToggle} aria-label={playing ? text("Pause", "إيقاف مؤقت") : text("Resume", "متابعة")} title={title}>{playing ? <Pause size={18} /> : <Play size={18} />}</button>
    <button ref={trigger} onClick={() => { setCollapsed(false); setExpanded(true); }} aria-label={text("Show recitation controls", "إظهار أدوات التلاوة")} aria-expanded={showPanel} aria-controls="side-recitation-player"><SlidersHorizontal size={17} /></button>
  </div>;
  const host = !showPanel ? document.getElementById("title-listen-host") : null;
  return <>
    {!showPanel ? host ? createPortal(compact, host) : <div className="sidePlayerCompactFloating" data-side={placement.side}>{compact}</div> : null}
    {showPanel ? <div ref={panel} id="side-recitation-player" className={`recitationMini sideRecitationPlayer${placement.roomy ? "" : " sidePlayerPopover"}`} data-side={placement.side}
      dir={arabic ? "rtl" : "ltr"} role="region" aria-label={text("Recitation controls", "أدوات التلاوة")}>
      <header className="miniRecording"><div><strong>{title}</strong><small>{subtitle}</small></div>
        <button onClick={collapse} aria-label={text("Collapse player", "تصغير أدوات التلاوة")} title={text("Collapse player", "تصغير أدوات التلاوة")}>{placement.roomy ? <ChevronDown size={17} /> : <ChevronUp size={17} />}</button></header>
      <div className="miniTransport" dir="ltr">
        <button disabled={!maximum} onClick={() => onSeek(elapsed - 10)} aria-label={text("Back 10 seconds", "رجوع ١٠ ثوانٍ")}><RotateCcw size={17} /><span>10</span></button>
        <button className="primaryPlayback" onClick={onToggle} aria-label={playing ? text("Pause", "إيقاف مؤقت") : text("Resume", "متابعة")}>{playing ? <Pause size={22} /> : <Play size={22} />}</button>
        <button disabled={!maximum} onClick={() => onSeek(elapsed + 10)} aria-label={text("Forward 10 seconds", "تقديم ١٠ ثوانٍ")}><span>10</span><RotateCw size={17} /></button>
      </div>
      <div className="miniTimeline" dir="ltr"><input aria-label={text("Playback position", "موضع التشغيل")} type="range" min={minimum} max={Math.max(minimum + .1, maximum)} step="0.1" value={Math.max(minimum, Math.min(maximum, elapsed))} disabled={!maximum} onChange={event => onSeek(Number(event.target.value))} />
        <div><span>{duration(elapsed)}</span><span>{total ? duration(total) : text("Buffered", "جاهز للتشغيل")}</span></div></div>
      <div className="miniActions"><button onClick={onSettings} aria-label={text("Listening settings", "إعدادات الاستماع")}><SlidersHorizontal size={16} /><span>{text("Settings", "الإعدادات")}</span></button>
        <button aria-label={text("Stop recitation", "إنهاء التلاوة")} onClick={onStop}><Square size={16} /><span>{text("Stop", "إنهاء")}</span></button></div>
    </div> : null}
  </>;
}
