import { useEffect, useMemo, useRef, useState } from "react";
import DOMPurify from "dompurify";
import { ChevronLeft, ChevronRight } from "lucide-react";

import { extraTafsir, installedTafsirs, readDownloadedTafsir, EXTRA_TAFSIRS, type TafsirSource, type TafsirGroup, type TafsirSurah, type ExtraTafsirId } from "./tafsirDownloads";
import { TafsirDownloads } from "./TafsirLibrary";
import { selectTafsirPassage } from "./tafsirPassage";
export type { TafsirSource } from "./tafsirDownloads";

export type TafsirSelection = {
  surah: number;
  ayah: number;
};

type TranslationEntry = {
  text: string;
  footnotes: Record<string, string>;
};

type TranslationSurah = Record<string, TranslationEntry>;
type Language = "arabic" | "english";

const jsonCache = new Map<string, Promise<unknown>>();

function loadJson<T>(path: string) {
  const cached = jsonCache.get(path);
  if (cached) return cached as Promise<T>;
  const request = fetch(path).then((response) => {
    if (!response.ok) throw new Error(`Could not load ${path}`);
    return response.json() as Promise<T>;
  });
  jsonCache.set(path, request);
  if (jsonCache.size > 8) jsonCache.delete(jsonCache.keys().next().value!);
  request.catch(() => jsonCache.delete(path));
  return request;
}

function sanitizeStudyHtml(source: string) {
  source = DOMPurify.sanitize(source);
  const document = new DOMParser().parseFromString(source, "text/html");
  const allowed = new Set(["P", "DIV", "SPAN", "B", "STRONG", "I", "EM", "BR", "SUP"]);
  const blocked = new Set(["SCRIPT", "STYLE", "IFRAME", "OBJECT", "EMBED"]);

  for (const element of Array.from(document.body.querySelectorAll("*"))) {
    if (blocked.has(element.tagName)) {
      element.remove();
      continue;
    }
    if (!allowed.has(element.tagName)) {
      element.replaceWith(...Array.from(element.childNodes));
      continue;
    }

    const sourceClass = element.getAttribute("class") ?? "";
    for (const attribute of Array.from(element.attributes)) {
      element.removeAttribute(attribute.name);
    }
    if (sourceClass.includes("blue")) element.className = "tafsirEmphasis";
    if (sourceClass.includes("brown")) element.className = "tafsirQuote";
  }

  return document.body.innerHTML;
}

export function TafsirPanel({
  selection,
  ayahText,
  language,
  source,
  textSize,
  onTextSizeChange,
  onSourceChange,
  onClose,
  onNavigate,
  verseCount,
  onSurahStep,
  onExitFullscreen,
  closing = false,
  onListen,
}: {
  closing?: boolean;
  onListen: (scope: "ayah" | "page" | "surah" | "quran" | "whole-surah" | "whole-page") => void;
  selection: TafsirSelection;
  ayahText: string;
  language: Language;
  source: TafsirSource;
  textSize: number;
  onTextSizeChange: (size: number) => void;
  onSourceChange: (source: TafsirSource) => void;
  onClose: () => void;
  onNavigate: (ayah: number) => void;
  verseCount: number;
  onSurahStep: (step: number) => void;
  onExitFullscreen?: () => void;
}) {

  const panel = useRef<HTMLElement>(null);
  const [installed, setInstalled] = useState<ExtraTafsirId[]>([]);
  const [packRevision, setPackRevision] = useState(0);
  useEffect(() => {
    let cancelled = false;
    void installedTafsirs().then(packs => {
      if (cancelled) return;
      const ids = Object.keys(packs) as ExtraTafsirId[];
      setInstalled(ids);
      if (extraTafsir(source) && !ids.includes(source as ExtraTafsirId)) onSourceChange("mukhtasar");
    }).catch(() => {});
    return () => { cancelled = true; };
  }, [packRevision, source, onSourceChange]);

  const [tafsir, setTafsir] = useState<TafsirGroup | null>(null);
  const [translation, setTranslation] = useState<TranslationEntry | null>(null);
  const [loadState, setLoadState] = useState<"loading" | "ready" | "error">("loading");
  const requestKey = `${language}:${source}:${selection.surah}:${selection.ayah}:${packRevision}`;
  const [loadedFor, setLoadedFor] = useState("");
  // A changed selection must not briefly paint the previous ayah/source's text.
  const visibleLoadState = loadedFor === requestKey ? loadState : "loading";
  const labels = language === "arabic"
    ? {
        title: "تدبر الآية",
        close: "إغلاق التفسير",
        tafsir: "التفسير",
        mukhtasar: "التفسير المختصر",
        saadi: "تفسير السعدي",
        loading: "جارٍ فتح التفسير...",
        failed: "تعذر فتح التفسير لهذه الآية.",
        footnotes: "الهوامش",
        source: "المصدر: المكتبة القرآنية الشاملة",
      }
    : {
        title: "Ayah study",
        close: "Close ayah study",
        translation: "Saheeh International",
        loading: "Opening translation...",
        failed: "Translation could not be opened for this ayah.",
        footnotes: "Footnotes",
        source: "Source: Quranic Universal Library",
      };

  useEffect(() => {
    panel.current?.querySelector(".studyBody")?.scrollTo({ top: 0 });
    let cancelled = false;
    setLoadState("loading");
    setTafsir(null);
    setTranslation(null);

    const request = language === "arabic"
      ? (extraTafsir(source) ? readDownloadedTafsir(source as ExtraTafsirId, selection.surah) : loadJson<TafsirSurah>(`/data/study/${source}/s${selection.surah}.json`)).then((tafsirSurah) => {
          const groupKey = tafsirSurah.lookup[String(selection.ayah)];
          return { tafsir: groupKey ? tafsirSurah.groups[groupKey] ?? null : null, translation: null };
        })
      : loadJson<TranslationSurah>(`/data/study/sahih/s${selection.surah}.json`).then((translationSurah) => ({
          tafsir: null,
          translation: translationSurah[String(selection.ayah)] ?? null,
        }));

    void request.then((result) => {
      if (cancelled) return;
      setTafsir(result.tafsir);
      setTranslation(result.translation);
      setLoadState("ready");
      setLoadedFor(requestKey);
    }).catch(() => {
      if (!cancelled) { setLoadState("error"); setLoadedFor(requestKey); }
    });

    return () => { cancelled = true; };
  }, [language, selection.ayah, selection.surah, source, packRevision, requestKey]);

  const safeTafsir = useMemo(() => sanitizeStudyHtml(tafsir?.text ?? ""), [tafsir?.text]);
  const passage = useMemo(() => selectTafsirPassage(tafsir, safeTafsir, selection.ayah, source === "saadi"),
    [tafsir, safeTafsir, selection.ayah, source]);
  const safeTranslation = useMemo(
    () => sanitizeStudyHtml(translation?.text ?? ""),
    [translation?.text],
  );
  const footnotes = Object.values(translation?.footnotes ?? {});

  return (
    <aside
      ref={panel}
      className="tafsirPanel ayahStudyPanel"
      inert={closing}
      aria-hidden={closing || undefined}
      role="dialog"
      aria-modal="false"
      aria-labelledby="tafsir-panel-title"
      dir={language === "arabic" ? "rtl" : "ltr"}
      lang={language === "arabic" ? "ar" : "en"}
    >
      <header className="tafsirPanelHeader">
        <h2 id="tafsir-panel-title">{labels.tafsir ?? labels.title}</h2>
        <details name="study-actions" className="studyListenMenu" key={`${selection.surah}:${selection.ayah}`}>
          <summary>{language === "arabic" ? "استماع" : "Listen"}</summary>
          <div className="studyListenOptions">
            <p>{language === "arabic" ? "ابدأ من الآية المختارة" : "Start from the selected ayah"}</p>
            {([
              ["ayah", "This ayah only", "هذه الآية فقط"],
              ["page", "To end of page", "حتى نهاية الصفحة"],
              ["surah", "To end of surah", "حتى نهاية السورة"],
              ["quran", "To end of Quran", "حتى نهاية القرآن"],
              ["whole-page", "Listen to this page", "استمع لهذه الصفحة"],
              ["whole-surah", "Listen to whole surah", "استمع للسورة كاملة"],
            ] as const).map(([scope, en, ar]) => <button type="button" key={scope}
              onClick={() => onListen(scope)}>{language === "arabic" ? ar : en}</button>)}
          </div>
        </details>
        {onExitFullscreen ? <button className="studyExitFullscreen" type="button" onClick={onExitFullscreen}>{language === "arabic" ? "إنهاء ملء الشاشة" : "Exit full screen"}</button> : null}
        {language === "arabic" ? <TafsirDownloads onChange={() => setPackRevision(value => value + 1)}
          textSize={textSize} onTextSizeChange={onTextSizeChange} /> : null}
        <button type="button" aria-label={labels.close} title={labels.close} onClick={onClose}>
          &#215;
        </button>
      </header>

      <nav className="studyNavigation" aria-label={language === "arabic" ? "التنقل بين الآيات" : "Ayah navigation"}>
        <div className="studyAyahControls"><button type="button" title={language === "arabic" ? "الآية السابقة" : "Previous ayah"}
          aria-label={language === "arabic" ? "الآية السابقة" : "Previous ayah"}
          disabled={selection.surah === 1 && selection.ayah === 1}
          onClick={() => selection.ayah > 1 ? onNavigate(selection.ayah - 1) : onSurahStep(-1)}>
          {language === "arabic" ? <ChevronRight size={18} /> : <ChevronLeft size={18} />}
        </button>
        <label>{language === "arabic" ? "الآية" : "Ayah"}
          <select value={selection.ayah} onChange={event => onNavigate(Number(event.target.value))}>
            {Array.from({ length: verseCount }, (_, index) => <option key={index} value={index + 1}>{index + 1}</option>)}
          </select><span> / {verseCount}</span>
        </label>
        <button type="button" title={language === "arabic" ? "الآية التالية" : "Next ayah"}
          aria-label={language === "arabic" ? "الآية التالية" : "Next ayah"}
          disabled={selection.surah === 114 && selection.ayah === verseCount}
          onClick={() => selection.ayah < verseCount ? onNavigate(selection.ayah + 1) : onSurahStep(1)}>
          {language === "arabic" ? <ChevronLeft size={18} /> : <ChevronRight size={18} />}
        </button></div>
      </nav>

      <div className="studyBody">
      <section className="studyVerse" aria-label={language === "arabic" ? "نص الآية" : "Selected ayah"}>
        <p dir="rtl" lang="ar">{ayahText || (language === "arabic" ? "جارٍ تحميل الآية…" : "Loading ayah…")}</p>
      </section>

      {language === "arabic" ? (
        <section className="tafsirContentSection" aria-labelledby="tafsir-source-heading">
          <div className="tafsirSectionHeading" id="tafsir-source-heading">
            {!installed.length ? <div className="segmented tafsirSourcePicker" aria-label={labels.tafsir}>
              <button type="button" aria-pressed={source === "mukhtasar"}
                onClick={() => onSourceChange("mukhtasar")}>{labels.mukhtasar}</button>
              <button type="button" aria-pressed={source === "saadi"}
                onClick={() => onSourceChange("saadi")}>{labels.saadi}</button>
            </div> : null}
          </div>

          {installed.length ? <label className="extraTafsirPicker">التفسير
            <select aria-label="اختيار التفسير" value={source} onChange={event => onSourceChange(event.target.value as TafsirSource)}>
              <option value="mukhtasar">{labels.mukhtasar}</option><option value="saadi">{labels.saadi}</option>
              {EXTRA_TAFSIRS.filter(item => installed.includes(item.id)).map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
            </select>
          </label> : null}
          {visibleLoadState === "loading" ? <p className="tafsirStatus">{labels.loading}</p> : null}
          {visibleLoadState === "error" ? (
            <p className="tafsirStatus" role="alert">{labels.failed}</p>
          ) : null}
          {visibleLoadState === "ready" && passage.kind === "unavailable" ? <p className="tafsirStatus" role="status">
            {tafsir ? "لا يتوفر تفسير مستقل لهذه الآية في بيانات المصدر الحالي." : "لا يتوفر نص تفسير لهذه الآية في المصدر الحالي."}
          </p> : null}
          {visibleLoadState === "ready" && passage.kind === "individual" ? (
            <div className="tafsirArabicText" dir="rtl" lang="ar" style={{ fontSize: textSize }}
              dangerouslySetInnerHTML={{ __html: passage.html }} />
          ) : null}
          {visibleLoadState === "ready" && passage.kind === "shared" ? <p className="tafsirStatus" role="status">
            ورد تفسير هذه الآية ضمن تفسير مشترك للآيات {passage.ayahs.join("، ")}.
          </p> : null}
          {visibleLoadState === "ready" && passage.kind !== "individual" && passage.contextHtml ?
            <details className="tafsirSharedPassage" key={`${selection.surah}:${selection.ayah}:${source}`}>
              <summary>{passage.kind === "shared" ? "عرض التفسير المشترك" : "عرض السياق الكامل في المصدر"}</summary>
              <div className="tafsirArabicText" dir="rtl" lang="ar" style={{ fontSize: textSize }}
                dangerouslySetInnerHTML={{ __html: passage.html || passage.contextHtml }} />
            </details> : null}
        </section>
      ) : (
        <section className="tafsirContentSection translationSection" lang="en" dir="ltr">
          <h3>{labels.translation}</h3>
          {visibleLoadState === "loading" ? <p className="tafsirStatus">{labels.loading}</p> : null}
          {visibleLoadState === "error" || (visibleLoadState === "ready" && !translation) ? (
            <p className="tafsirStatus" role="alert">{labels.failed}</p>
          ) : null}
          {visibleLoadState === "ready" && translation ? (
            <div className="tafsirTranslationText" style={{ fontSize: textSize }}
              dangerouslySetInnerHTML={{ __html: safeTranslation }} />
          ) : null}
          {visibleLoadState === "ready" && footnotes.length > 0 ? (
            <details className="tafsirFootnotes">
              <summary>{labels.footnotes}</summary>
              <ol>{footnotes.map((note, index) => <li key={`${index}-${note}`}>{note}</li>)}</ol>
            </details>
          ) : null}
        </section>
      )}

      <footer className="tafsirAttribution">{language === "arabic" && extraTafsir(source) ? `المصدر: Quran.com — ${extraTafsir(source)!.name}` : labels.source}</footer>
      </div>
    </aside>
  );
}


