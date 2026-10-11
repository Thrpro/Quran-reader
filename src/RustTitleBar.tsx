import { useEffect, useLayoutEffect, useRef, useState, type MouseEvent } from "react";
import { BookOpen, ChevronDown, ChevronUp, Copy, Minus, Search, Settings2, Square, X } from "lucide-react";
import "./RustTitleBar.css";

type WindowAction = "minimize" | "maximize" | "close" | "drag";
type ReaderBridge = {
  postMessage: (message: unknown) => void;
  addEventListener: (type: "message", listener: (event: MessageEvent<string>) => void) => void;
  removeEventListener: (type: "message", listener: (event: MessageEvent<string>) => void) => void;
};
type RustHostWindow = Window & {
  __QURAN_RUST_HOST__?: boolean;
  chrome?: { webview?: ReaderBridge };
};

export function isRustNativeHost() {
  return (window as RustHostWindow).__QURAN_RUST_HOST__ === true;
}

function windowAction(action: WindowAction) {
  (window as RustHostWindow).chrome?.webview?.postMessage({ type: "window-action", action });
}

export function RustTitleBar({ metadataLanguage, readerControlsVisible, onToggleControls, readingSessionActive, onSearch, searchOpen, controlsEnabled }: {
  metadataLanguage: "arabic" | "english";
  readerControlsVisible: boolean;
  onToggleControls: () => void;
  readingSessionActive?: boolean;
  onSearch: () => void;
  searchOpen: boolean;
  controlsEnabled: boolean;
}) {
  const [maximized, setMaximized] = useState(false);
  const [nativeSessionActive, setNativeSessionActive] = useState(false);
  const nativeHost = isRustNativeHost();
  const arabic = metadataLanguage === "arabic";
  const sessionActive = readingSessionActive ?? nativeSessionActive;
  const text = (english: string, translated: string) => arabic ? translated : english;
  const actionsRef = useRef<HTMLElement>(null);
  const previousPosition = useRef<number | null>(null);

  useLayoutEffect(() => {
    const actions = actionsRef.current;
    if (!actions) return;
    const left = actions.getBoundingClientRect().left;
    const previous = previousPosition.current;
    previousPosition.current = left;
    if (previous === null || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const animation = actions.animate(
      [{ transform: `translateX(${previous - left}px)`, opacity: .65 }, { transform: "none", opacity: 1 }],
      { duration: 220, easing: "cubic-bezier(.2,.8,.2,1)" },
    );
    return () => animation.cancel();
  }, [metadataLanguage]);

  useEffect(() => {
    if (!nativeHost) return;
    const bridge = (window as RustHostWindow).chrome?.webview;
    if (!bridge) return;
    const onMessage = ({ data }: MessageEvent<string>) => {
      if (data === "native-window-maximized") setMaximized(true);
      if (data === "native-window-restored") setMaximized(false);
      if (data === "native-reading-session-on") setNativeSessionActive(true);
      if (data === "native-reading-session-off") setNativeSessionActive(false);
    };
    bridge.addEventListener("message", onMessage);
    return () => bridge.removeEventListener("message", onMessage);
  }, [nativeHost]);

  if (!nativeHost) return null;

  function isDragSurface(event: MouseEvent<HTMLElement>) {
    return event.target instanceof Element && !event.target.closest("button");
  }

  function beginDrag(event: MouseEvent<HTMLElement>) {
    if (event.button !== 0 || event.detail > 1 || !isDragSurface(event)) return;
    event.preventDefault();
    windowAction("drag");
  }

  const optionsTooltip = sessionActive
    ? text("Reading in progress — Search & options", "القراءة جارية — البحث والخيارات")
    : readerControlsVisible
      ? text("Hide reading tools", "إخفاء أدوات القراءة")
      : text("Show reading tools", "إظهار أدوات القراءة");

  return (
    <header className="rustTitleBar" data-language={metadataLanguage} aria-label={text("Application title bar", "شريط عنوان التطبيق")}
      onMouseDown={beginDrag}
      onDoubleClick={event => { if (isDragSurface(event)) windowAction("maximize"); }}>
      <div className="rustTitleIdentity" aria-hidden="true">
        <BookOpen className="rustTitleIcon" size={18} strokeWidth={1.5} />
        <span>Quran Mushaf Reader</span>
      </div>
      <nav ref={actionsRef} className="rustReaderActions" aria-label={text("Reading tools", "أدوات القراءة")} dir={arabic ? "rtl" : "ltr"}>
      <button className="rustSearchButton" type="button" disabled={!controlsEnabled} onClick={onSearch}
        aria-expanded={searchOpen} aria-controls="quran-search-panel">
        <Search size={15} aria-hidden="true" /><span>{text("Search", "بحث")}</span>
      </button>
      <div id="title-listen-host" style={{ display: "contents" }} />
      <button className="rustTitleOptions" type="button" onClick={onToggleControls}
        disabled={!controlsEnabled} aria-controls="reader-controls" aria-expanded={readerControlsVisible} title={optionsTooltip}>
        {sessionActive ? <span className="rustSessionDot" role="img"
          aria-label={text("Reading in progress", "القراءة جارية")} /> : null}
        <Settings2 size={15} aria-hidden="true" /><span>{text("Options", "الخيارات")}</span>
        {readerControlsVisible ? <ChevronUp size={12} aria-hidden="true" /> : <ChevronDown size={12} aria-hidden="true" />}
      </button>
      </nav>
      <div className="rustTitleDragSpace" aria-hidden="true" />
      <nav className="rustWindowActions" aria-label={text("Window controls", "عناصر التحكم بالنافذة")}>
        <button type="button" onClick={() => windowAction("minimize")}
          aria-label={text("Minimize", "تصغير")} title={text("Minimize", "تصغير")}>
          <Minus size={13} aria-hidden="true" />
        </button>
        <button type="button" onClick={() => windowAction("maximize")}
          aria-label={maximized ? text("Restore window", "استعادة النافذة") : text("Maximize", "تكبير")}
          title={maximized ? text("Restore window", "استعادة النافذة") : text("Maximize", "تكبير")}>
          {maximized ? <Copy size={12} aria-hidden="true" /> : <Square size={12} aria-hidden="true" />}
        </button>
        <button className="rustWindowClose" type="button" onClick={() => windowAction("close")}
          aria-label={text("Close", "إغلاق")} title={text("Close", "إغلاق")}>
          <X size={15} aria-hidden="true" />
        </button>
      </nav>
    </header>
  );
}
