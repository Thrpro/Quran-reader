import { AyahActions } from "./AyahActions";
import { RustTitleBar, isRustNativeHost } from "./RustTitleBar";
import {
  type CSSProperties,
  type FormEvent,
  type ReactNode,
  memo,
  lazy,
  Suspense,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { surahHeaderLigatures } from "./surahHeaderLigatures";
import { AsyncPageCache } from "./asyncPageCache";
import { createAyahSearcher } from "./ayahSearch";
import { usePageDrag } from "./usePageDrag";
import { hasSavedLanguage, LANGUAGE_SETUP_VERSION } from "./languagePreferences";
import { LanguageChoices, LanguageSetup } from "./LanguagePicker";
import type { TafsirSelection, TafsirSource } from "./TafsirPanel";
import { ReadingTools, type ListeningRequest } from "./ReadingTools";
import { ShapedQuranLine, refreshShapedInk, type ShapedPage } from "./ShapedQuranLine";

// Study HTML sanitizing and the optional library are needed only when opened.
const TafsirPanel = lazy(() => import("./TafsirPanel").then(module => ({ default: module.TafsirPanel })));

const PAGE_COUNT = 604;
const PAGE_ASPECT_WIDTH = 617;
const PAGE_ASPECT_HEIGHT = 919;
const PAGE_GAP = 0;
const MAX_FIT_SCALE = 1.18;
const STORAGE_KEY = "quran-mushaf-qul-reader:v1";
const SETTINGS_VERSION = 5;
const V4_BASMALLAH_WORDS = ["\uFCAA", "\uFCAB", "\uFCAE", "\uFCB4"] as const;
const V4_BASMALLAH_GLYPHS = V4_BASMALLAH_WORDS.join(" ");
const APP_STARTED_AT = performance.now();

// Measure a whole render batch before writing any fitted widths. Measuring each
// line after changing the previous one forces layout repeatedly during cold turns.
const pendingLineFits = new Set<HTMLElement>();
let lineFitScheduled = false;
function scheduleLineFit(element: HTMLElement) {
  pendingLineFits.add(element);
  if (lineFitScheduled) return;
  lineFitScheduled = true;
  queueMicrotask(() => {
    lineFitScheduled = false;
    const elements = [...pendingLineFits].filter(element => element.isConnected);
    pendingLineFits.clear();
    for (const element of elements) {
      element.classList.remove("line-condensedToFit");
      element.style.removeProperty("--line-fit-scale");
    }
    const measurements = elements.map(element => ({ element, available: element.clientWidth, natural: element.scrollWidth }));
    for (const { element, available, natural } of measurements) {
      if (available > 0 && natural > available + 1) {
        element.style.setProperty("--line-fit-scale", String(Math.max(0.84, available / natural)));
        element.classList.add("line-condensedToFit");
      }
    }
  });
}

type ReaderMode = "single" | "spread";
type ThemeMode = "dark" | "black" | "navy" | "light" | "sepia" | "system";
type TextMode = "natural";
type SizePreset = "auto" | "smaller" | "normal" | "larger" | "custom";
type MetadataLanguage = "arabic" | "english";
type ClockFormat = "12" | "24";
type SearchShortcut = "ctrl-f" | "s";

type LineType = "ayah" | "surah_name" | "basmallah";

type QulLine = {
  lineNumber: number;
  type: LineType;
  isCentered: boolean;
  firstWordId: number | null;
  lastWordId: number | null;
  surahNumber: number | null;
  text: string;
  words: number[];
};

type QulPage = {
  pageNumber: number;
  lines: QulLine[];
};

type Surah = {
  id: number;
  name: string;
  nameSimple: string;
  nameArabic: string;
  versesCount: number;
};

type PageIndex = {
  pageNumber: number;
  firstSurah: number | null;
  firstAyah: number | null;
  juz: number | null;
  hizb: number | null;
  rub: number | null;
};

type JuzIndex = {
  juz: number;
  pageNumber: number;
};

type HizbIndex = {
  hizb: number;
  pageNumber: number;
  surah: number;
  ayah: number;
};

type RubIndex = {
  rub: number;
  hizb: number;
  quarter: number;
  pageNumber: number;
  surah: number;
  ayah: number;
};

type QulData = {
  surahs: Surah[];
  pageIndex: PageIndex[];
  juz: JuzIndex[];
  hizb: HizbIndex[];
  rub: RubIndex[];
  surahStarts: SurahStartPage[];
};

type HighlightedAyah = {
  surah: number;
  ayah: number;
  pageNumber: number;
  firstWordId: number;
  lastWordId: number;
};

type AyahSearchEntry = {
  key: string;
  surah: number;
  ayah: number;
  pageNumber: number;
  lineNumbers: number[];
  firstWordId: number;
  lastWordId: number;
  text: string;
  normalizedText: string;
  expandedText?: string;
};

type SurahStartPage = {
  surah: number;
  pageNumber: number;
};

type QpcPageFont = {
  pageNumber: number;
  fontPath: string;
  fontFamily: string;
  fontFormat: string;
};

type QpcLine = {
  lineNumber: number;
  type: LineType;
  isCentered: boolean;
  firstWordId: number | null;
  lastWordId: number | null;
  surahNumber: number | null;
  text: string;
  words: { id: number; text: string; surah: number; ayah: number }[];
};

type QpcPage = {
  pageNumber: number;
  lines: QpcLine[];
  font: QpcPageFont;
  shaped?: ShapedPage;
  outlines?: Record<number, string>;
};

type QpcData = {
  pages: Map<number, QpcPage>;
};

type SearchResult =
  | {
      type: "page";
      key: string;
      pageNumber: number;
    }
  | {
      type: "surah";
      key: string;
      surah: Surah;
      pageNumber: number;
    }
  | {
      type: "ayah";
      key: string;
      pageNumber: number;
      surah: Surah;
      ayah: number;
      firstWordId: number;
      lastWordId: number;
      snippet: string;
    };

type ReaderSettings = {
  settingsVersion: number;
  languageSetupVersion: number;
  page: number;
  mode: ReaderMode;
  sizePreset: SizePreset;
  textScale: number;
  tafsirTextSize: number;
  highlightColor: "auto" | "green" | "gold";
  textMode: TextMode;
  contextualText: boolean;
  theme: ThemeMode;
  showClock: boolean;
  clockFormat: ClockFormat;
  searchShortcut: SearchShortcut;
  metadataLanguage: MetadataLanguage;
  hasChosenLanguage: boolean;
};

type PageChromeMeta = {
  surahName: string;
  juz: number | null;
  hizbStart: number | null;
  rubStart: number | null;
  side: "right" | "left";
};

const defaultSettings: ReaderSettings = {
  settingsVersion: SETTINGS_VERSION,
  languageSetupVersion: 0,
  page: 1,
  mode: "spread",
  sizePreset: "custom",
  textScale: 145,
  tafsirTextSize: 18,
  highlightColor: "auto",
  textMode: "natural",
  contextualText: false,
  theme: "dark",
  showClock: true,
  clockFormat: "24",
  searchShortcut: "ctrl-f",
  metadataLanguage: "arabic",
  hasChosenLanguage: false,
};

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

function normalizePage(value: number) {
  return clamp(Math.trunc(value), 1, PAGE_COUNT);
}

function readSettings(): ReaderSettings {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (!stored) {
      return defaultSettings;
    }

    const parsed = JSON.parse(stored) as Omit<
      Partial<ReaderSettings>,
      "textMode" | "sizePreset"
    > & {
      textMode?: TextMode | "justified" | "mushaf" | "mushafFit";
      sizePreset?: SizePreset;
    };
    const shouldApplyDefaults = parsed.settingsVersion !== SETTINGS_VERSION;

    return {
      settingsVersion: SETTINGS_VERSION,
      languageSetupVersion: Number(parsed.languageSetupVersion) || 0,
      page: normalizePage(Number(parsed.page) || defaultSettings.page),
      mode: shouldApplyDefaults
        ? defaultSettings.mode
        : parsed.mode === "single"
          ? "single"
          : "spread",
      sizePreset: shouldApplyDefaults
        ? defaultSettings.sizePreset
        : parsed.sizePreset === "smaller" ||
            parsed.sizePreset === "auto" ||
            parsed.sizePreset === "normal" ||
            parsed.sizePreset === "larger" ||
            parsed.sizePreset === "custom"
          ? parsed.sizePreset
          : defaultSettings.sizePreset,
      textScale: clamp(Number(parsed.textScale) || defaultSettings.textScale, 78, 150),
      highlightColor: parsed.highlightColor === "green" || parsed.highlightColor === "gold" ? parsed.highlightColor : "auto",
      tafsirTextSize: clamp(Number(parsed.tafsirTextSize) || defaultSettings.tafsirTextSize, 16, 26),
      textMode: "natural",
      contextualText: parsed.contextualText === true,
      theme: shouldApplyDefaults
        ? defaultSettings.theme
        : parsed.theme === "black" ||
            parsed.theme === "navy" ||
            parsed.theme === "light" ||
            parsed.theme === "sepia" ||
            parsed.theme === "system"
          ? parsed.theme
          : "dark",
      showClock: shouldApplyDefaults ? defaultSettings.showClock : parsed.showClock !== false,
      clockFormat: parsed.clockFormat === "12" ? "12" : "24",
      searchShortcut: parsed.searchShortcut === "s" ? "s" : "ctrl-f",
      metadataLanguage: parsed.metadataLanguage === "english" ? "english" : "arabic",
      hasChosenLanguage: hasSavedLanguage(parsed),
    };
  } catch {
    return defaultSettings;
  }
}

async function fetchJson<T>(path: string): Promise<T> {
  const response = await fetch(path);
  if (!response.ok) {
    throw new Error(`Could not load ${path}`);
  }
  return (await response.json()) as T;
}

function getSpreadPages(page: number) {
  const rightPage = page % 2 === 1 ? page : page - 1;
  const leftPage = rightPage + 1 <= PAGE_COUNT ? rightPage + 1 : null;
  return { leftPage, rightPage };
}

const ARABIC_INDIC_DIGITS = "٠١٢٣٤٥٦٧٨٩";

function formatMetadataNumber(value: number, language: MetadataLanguage) {
  const text = String(value);
  return language === "arabic"
    ? text.replace(/\d/g, (digit) => ARABIC_INDIC_DIGITS[Number(digit)])
    : text;
}

function getMetadataLabels(language: MetadataLanguage) {
  return language === "arabic"
    ? {
        juz: "الجزء",
        hizb: "الحزب",
        rub: "الربع",
        page: "الصفحة",
        selectJuz: "اختر الجزء",
        selectHizb: "اختر الحزب",
        selectRub: "اختر الربع",
      }
    : {
        juz: "Juz",
        hizb: "Hizb",
        rub: "Rub",
        page: "page",
        selectJuz: "Select juz",
        selectHizb: "Select hizb",
        selectRub: "Select rub",
      };
}

function getPageSectionMark(meta: PageChromeMeta, language: MetadataLanguage) {
  const labels = getMetadataLabels(language);

  if (meta.hizbStart) {
    return {
      label: labels.hizb,
      number: formatMetadataNumber(meta.hizbStart, language),
    };
  }

  if (!meta.rubStart) {
    return null;
  }

  const hizb = Math.ceil(meta.rubStart / 4);
  const positionInHizb = ((meta.rubStart - 1) % 4) + 1;
  const hizbNumber = formatMetadataNumber(hizb, language);

  if (language === "arabic") {
    switch (positionInHizb) {
      case 2:
        return { label: "ربع الحزب", number: hizbNumber };
      case 3:
        return { label: "نصف الحزب", number: hizbNumber };
      case 4:
        return { label: "ثلاثة أرباع الحزب", number: hizbNumber };
      default:
        return { label: labels.hizb, number: hizbNumber };
    }
  }

  switch (positionInHizb) {
    case 2:
      return { label: "Quarter of Hizb", number: hizbNumber };
    case 3:
      return { label: "Half of Hizb", number: hizbNumber };
    case 4:
      return { label: "Three quarters of Hizb", number: hizbNumber };
    default:
      return { label: labels.hizb, number: hizbNumber };
  }
}

function getInterfaceLabels(language: MetadataLanguage) {
  return language === "arabic"
    ? {
        settings: "الإعدادات",
        closeSettings: "إغلاق الإعدادات",
        theme: "المظهر",
        themeGraphite: "رمادي داكن",
        themeBlack: "أسود",
        themeNavy: "كحلي",
        themeLight: "فاتح",
        themeSepia: "ورقي",
        themeSystem: "حسب النظام",
        tafsirTextSize: "حجم نص التفسير",
        pageView: "عرض الصفحات",
        pageLayout: "تنسيق الصفحات",
        onePage: "صفحة واحدة",
        twoPages: "صفحتان",
        interfaceLanguage: "لغة الواجهة",
        pageClock: "ساعة الصفحة",
        pageClockDescription: "عرض الوقت الحالي أعلى صفحات المصحف",
        timeFormat: "تنسيق الوقت",
        twelveHour: "١٢ ساعة",
        twentyFourHour: "٢٤ ساعة",
        keyboardShortcuts: "اختصارات لوحة المفاتيح",
        openSearchShortcut: "فتح البحث",
        nextPageShortcut: "الصفحة التالية",
        previousPageShortcut: "الصفحة السابقة",
        resetSettings: "إعادة ضبط الإعدادات",
        jumpTo: "الانتقال",
        search: "بحث",
        closeJump: "إغلاق قائمة الانتقال",
        page: "الصفحة",
        jumpToPage: "الانتقال إلى رقم الصفحة",
        go: "اذهب",
        surah: "السورة",
        findSurah: "ابحث باسم السورة أو رقمها",
        searchOrSelectSurah: "ابحث عن سورة أو اخترها",
        surahs: "السور",
        noMatchingSurah: "لا توجد سورة مطابقة",
        searchQuran: "البحث في القرآن",
        searchPlaceholder: "ابحث بكلمة أو آية، أو اكتب رقم صفحة مثل ٢٥٥",
        preparingSearch: "جارٍ تجهيز البحث...",
        noSearchResults: "لا توجد سورة أو آية مطابقة. جرّب كلمات أقل أو مرجعًا مثل ٢:٢٥٥.",
        browseQuran: "التصفح والانتقال",
        readerControls: "أدوات القارئ",
        showReaderControls: "إظهار أدوات القارئ",
        searchAndOptions: "البحث والخيارات",
        openJump: "الانتقال إلى صفحة أو سورة أو جزء أو حزب أو ربع",
        openSettings: "الإعدادات",
        nextPage: "الصفحة التالية",
        previousPage: "الصفحة السابقة",
        rightPage: "يمين",
        leftPage: "يسار",
        quranPage: "صفحة القرآن",
        quranPages: "صفحات القرآن",
        loadingMushaf: "جارٍ تحميل المصحف...",
        loadingPage: "جارٍ تحميل الصفحة...",
        pageLoadFailed: "تعذر تحميل الصفحة.",
        retry: "إعادة المحاولة",
      }
    : {
        settings: "Settings",
        closeSettings: "Close settings",
        theme: "Theme",
        themeGraphite: "Graphite",
        themeBlack: "Pure black",
        themeNavy: "Navy night",
        themeLight: "Light",
        themeSepia: "Sepia",
        themeSystem: "System",
        tafsirTextSize: "Tafsir text size",
        pageView: "Page view",
        pageLayout: "Page layout",
        onePage: "One page",
        twoPages: "Two pages",
        interfaceLanguage: "Interface language",
        pageClock: "Page clock",
        pageClockDescription: "Show the current time above the Quran pages",
        timeFormat: "Time format",
        twelveHour: "12-hour",
        twentyFourHour: "24-hour",
        keyboardShortcuts: "Keyboard shortcuts",
        openSearchShortcut: "Open search",
        nextPageShortcut: "Next page",
        previousPageShortcut: "Previous page",
        resetSettings: "Reset settings",
        jumpTo: "Jump to",
        search: "Search",
        closeJump: "Close jump menu",
        page: "Page",
        jumpToPage: "Jump to page number",
        go: "Go",
        surah: "Surah",
        findSurah: "Find by name, Arabic, or number",
        searchOrSelectSurah: "Search or select a Surah",
        surahs: "Surahs",
        noMatchingSurah: "No matching Surah",
        searchQuran: "Search Quran",
        searchPlaceholder: "Search words or an ayah, or enter a page such as 255",
        preparingSearch: "Preparing Quran search...",
        noSearchResults: "No matching Surah or ayah. Try fewer words or a reference such as 2:255.",
        browseQuran: "Browse and jump",
        readerControls: "Reader controls",
        showReaderControls: "Show reader controls",
        searchAndOptions: "Search & options",
        openJump: "Jump to page, Surah, Juz, Hizb, or Rub",
        openSettings: "Settings",
        nextPage: "Next page",
        previousPage: "Previous page",
        rightPage: "Right",
        leftPage: "Left",
        quranPage: "Quran page",
        quranPages: "Quran pages",
        loadingMushaf: "Loading Mushaf...",
        loadingPage: "Loading page...",
        pageLoadFailed: "Could not load this page.",
        retry: "Retry",
      };
}

function formatSectionOption(
  label: string,
  section: number,
  pageNumber: number,
  language: MetadataLanguage,
) {
  const labels = getMetadataLabels(language);
  return `${label} ${formatMetadataNumber(section, language)} - ${labels.page} ${formatMetadataNumber(pageNumber, language)}`;
}

function normalizeSearchText(value: string, expandSmallLetters = false) {
  return value
    .toLocaleLowerCase()
    .normalize("NFKD")
    .replace(/\u0670/g, expandSmallLetters ? "ا" : "")
    .replace(/\u06E5/g, "و")
    .replace(/\u06E6/g, "ي")
    .replace(/\p{M}/gu, "")
    .replace(/[\u0610-\u061A\u06D6-\u06ED\u08D3-\u08FF]/g, "")
    .replace(/[ٱأإآا]/g, "ا")
    .replace(/ؤ/g, "و")
    .replace(/[ئىيیۍېے]/g, "ي")
    .replace(/[ةۀە]/g, "ه")
    .replace(/[كڪ]/g, "ك")
    .replace(/ـ/g, "")
    .replace(/[٠-٩]/g, (digit) => String("٠١٢٣٤٥٦٧٨٩".indexOf(digit)))
    .replace(/[۰-۹]/g, (digit) => String("۰۱۲۳۴۵۶۷۸۹".indexOf(digit)))
    .replace(/[^a-z0-9\u0621-\u063A\u0641-\u064A]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function getSearchTextVariants(value: string) {
  return Array.from(
    new Set([
      normalizeSearchText(value),
      normalizeSearchText(value, true),
    ]),
  ).filter(Boolean);
}

function stripTrailingVerseNumber(value: string) {
  return value.replace(/\s*[۝﴿]?\s*[0-9٠-٩۰-۹]+\s*[۝﴾]?\s*$/u, "").trim();
}

function surahSearchText(surah: Surah) {
  return normalizeSearchText(
    [
      surah.id,
      surah.name,
      surah.nameSimple,
      surah.nameArabic,
      surah.nameSimple.replace(/^Al-/i, ""),
    ].join(" "),
  );
}

function getTextSnippet(text: string, query: string) {
  const normalizedText = normalizeSearchText(text);
  const normalizedQuery = normalizeSearchText(query);
  const index = normalizedText.indexOf(normalizedQuery);

  if (index < 0) {
    return text.slice(0, 96);
  }

  return text.slice(Math.max(0, index - 32), index + normalizedQuery.length + 64);
}

function parseQuranReference(query: string) {
  const normalized = normalizeSearchText(query);
  const match = normalized.match(/^(\d{1,3})\s+(\d{1,3})$/);
  if (!match) {
    return null;
  }

  return {
    surah: Number(match[1]),
    ayah: Number(match[2]),
  };
}

function getSurahNameLigature(surahNumber: number) {
  return surahHeaderLigatures[surahNumber] ?? "";
}

function wordMatchesAyah(wordId: number, highlightedAyah: HighlightedAyah | null) {
  if (!highlightedAyah) {
    return false;
  }

  return wordId >= highlightedAyah.firstWordId && wordId <= highlightedAyah.lastWordId;
}

function lineMatchesAyah(
  wordIds: number[],
  highlightedAyah: HighlightedAyah | null,
) {
  return wordIds.some((wordId) => wordMatchesAyah(wordId, highlightedAyah));
}

function QuranLine({
  line,
  text,
  isHighlighted,
}: {
  line: QulLine;
  text: string;
  isHighlighted: boolean;
}) {
  return (
    <div
      className={[
        "qulLine",
        `line-${line.type}`,
        line.isCentered ? "line-centered" : "line-ayahText",
        isHighlighted ? "line-highlighted" : "",
      ].join(" ")}
      key={line.lineNumber}
      style={{ "--line-number": line.lineNumber } as CSSProperties}
    >
      {text}
    </div>
  );
}

const DigitalKhattLine = memo(function DigitalKhattLine({
  line,
  text,
  highlightedAyah,
  recitationAyah,
}: {
  line: QpcLine;
  text: string;
  highlightedAyah: HighlightedAyah | null;
  recitationAyah: string | null;
}) {
  const lineRef = useRef<HTMLDivElement>(null);
  const shouldRenderAyahRuns = line.type === "ayah" && line.words.length > 0;
  const shouldJustifyLine = line.type === "ayah" && !line.isCentered;
  const ayahRuns = shouldRenderAyahRuns
    ? line.words.reduce<
        {
          key: string;
          isHighlighted: boolean;
          glyphs: string[];
        }[]
      >((runs, word) => {
        const isHighlighted = wordMatchesAyah(word.id, highlightedAyah);
        const ayahKey = `${word.surah}:${word.ayah}`;
        const lastRun = runs.at(-1);

        if (lastRun && lastRun.key === ayahKey) {
          lastRun.glyphs.push(word.text);
          lastRun.isHighlighted ||= isHighlighted;
          return runs;
        }

        runs.push({
          key: ayahKey,
          isHighlighted,
          glyphs: [word.text],
        });
        return runs;
      }, [])
    : [];
  const style = {
    "--line-number": line.lineNumber,
  } as CSSProperties;

  useLayoutEffect(() => {
    const element = lineRef.current;
    if (!element || !shouldJustifyLine) return;

    let cancelled = false;
    const fitLongLine = () => {
      if (cancelled) return;
      scheduleLineFit(element);
    };

    fitLongLine();
    if (document.fonts.status !== "loaded") void document.fonts.ready.then(fitLongLine);
    return () => {
      cancelled = true;
    };
  }, [shouldJustifyLine, text]);

  return (
    <div
      ref={lineRef}
      className={[
        "qulLine",
        "qpcLine",
        "digitalKhattLine",
        `line-${line.type}`,
        line.isCentered ? "line-centered" : "line-ayahText",
        shouldRenderAyahRuns ? "line-ayahInteractions" : "",
        shouldJustifyLine ? "line-stableJustified" : "",
      ].join(" ")}
      key={line.lineNumber}
      style={style}
    >
      {shouldRenderAyahRuns
        ? (
            <span className="digitalKhattLineContent">
              {ayahRuns.map((run, index) => {
                return (
                  <span
                    className={`ayahRun qpcWordRun ${run.isHighlighted ? "ayahHighlightRun" : ""} ${run.key === recitationAyah ? "ayahRun-reciting" : ""}`}
                    data-ayah-key={run.key}
                    key={run.key}
                  >
                    {run.glyphs.map((glyph, glyphIndex) => (
                      <span className="ayahWord" key={`${run.key}-${glyphIndex}`}>
                        {glyph}
                        {glyphIndex < run.glyphs.length - 1 ? " " : ""}
                      </span>
                    ))}
                    {index < ayahRuns.length - 1 ? " " : ""}
                  </span>
                );
              })}
            </span>
          )
        : line.type === "surah_name"
          ? <span className="surahHeaderGlyph">{text}</span>
          : line.type === "basmallah"
            ? (
                <span
                  className="bismillahGlyph"
                  aria-label="Bismillah ir-Rahman ir-Rahim"
                >
                  {V4_BASMALLAH_WORDS.map((word, index) => (
                    <span className="bismillahWord" key={word}>
                      {word}
                      {index < V4_BASMALLAH_WORDS.length - 1 ? " " : ""}
                    </span>
                  ))}
                </span>
              )
            : <span className="digitalKhattLineContent">{text}</span>}
    </div>
  );
});

function QuranPage({
  page,
  surahsById,
  textMode,
  meta,
  showSideLabel,
  metadataLanguage,
  currentTime,
  clockFormat,
  highlightedAyah,
}: {
  page: QulPage;
  surahsById: Map<number, Surah>;
  textMode: TextMode;
  meta: PageChromeMeta | null;
  showSideLabel: boolean;
  metadataLanguage: MetadataLanguage;
  currentTime: Date;
  clockFormat: ClockFormat;
  highlightedAyah: HighlightedAyah | null;
}) {
  return (
    <article
      className={[
        "qulPage",
        `textMode-${textMode}`,
        page.pageNumber <= 2 ? "openingMushafPage" : "",
        meta?.side === "left" ? "leftMushafPage" : "rightMushafPage",
      ].join(" ")}
      translate="no" lang="ar" data-page-number={page.pageNumber}
      aria-label={`${getInterfaceLabels(metadataLanguage).quranPage} ${formatMetadataNumber(page.pageNumber, metadataLanguage)}`}
    >
      <PageTopMeta
        meta={meta}
        showSideLabel={showSideLabel}
        metadataLanguage={metadataLanguage}
        currentTime={currentTime}
        clockFormat={clockFormat}
      />
      {page.lines.map((line) => {
        const surah = line.surahNumber ? surahsById.get(line.surahNumber) : null;
        const text =
          line.type === "surah_name" && surah
            ? `سورة ${surah.nameArabic}`
            : line.type === "basmallah"
              ? V4_BASMALLAH_GLYPHS
              : line.text;

        return (
          <QuranLine
            key={line.lineNumber}
            line={line}
            isHighlighted={lineMatchesAyah(line.words, highlightedAyah)}
            text={text}
          />
        );
      })}
      <div
        className="pageNumberMark"
        dir={metadataLanguage === "arabic" ? "rtl" : "ltr"}
        aria-hidden="true"
      >
        {formatMetadataNumber(page.pageNumber, metadataLanguage)}
      </div>
      <PageSectionMark meta={meta} metadataLanguage={metadataLanguage} />
    </article>
  );
}

const DigitalKhattPage = memo(function DigitalKhattPage({
  page,
  contextualText,
  contextualInkWeight,
  surahsById,
  meta,
  showSideLabel,
  metadataLanguage,
  currentTime,
  clockFormat,
  highlightedAyah,
  recitationAyah,
  renderScale,
  inkTheme,
}: {
  page: QpcPage;
  contextualText: boolean;
  contextualInkWeight: boolean;
  surahsById: Map<number, Surah>;
  meta: PageChromeMeta | null;
  showSideLabel: boolean;
  metadataLanguage: MetadataLanguage;
  currentTime: Date;
  clockFormat: ClockFormat;
  highlightedAyah: HighlightedAyah | null;
  recitationAyah: string | null;
  renderScale: number;
  inkTheme: string;
}) {
  return (
    <article
      className={[
        "qulPage",
        "qpcPage",
        "digitalKhattPage",
        page.pageNumber <= 2 ? "openingMushafPage" : "",
        meta?.side === "left" ? "leftMushafPage" : "rightMushafPage",
      ].join(" ")}
      translate="no" lang="ar" data-page-number={page.pageNumber}
      aria-label={`${getInterfaceLabels(metadataLanguage).quranPage} ${formatMetadataNumber(page.pageNumber, metadataLanguage)}`}
    >
      <PageTopMeta
        meta={meta}
        showSideLabel={showSideLabel}
        metadataLanguage={metadataLanguage}
        currentTime={currentTime}
        clockFormat={clockFormat}
      />
      {page.lines.map((line) => {
        const shape = contextualText ? page.shaped?.lines[line.lineNumber] : undefined;
        if (shape && page.outlines) return <ShapedQuranLine key={line.lineNumber}
          shape={shape} outlines={page.outlines} words={line.words} lineNumber={line.lineNumber}
          highlighted={highlightedAyah} reciting={recitationAyah} darkInk={contextualInkWeight} renderScale={renderScale} inkTheme={inkTheme} />;
        const surah = line.surahNumber ? surahsById.get(line.surahNumber) : null;
        const text =
          line.type === "surah_name" && surah
            ? getSurahNameLigature(surah.id)
            : line.type === "basmallah"
              ? V4_BASMALLAH_GLYPHS
              : line.text;

        return (
          <DigitalKhattLine
            key={line.lineNumber}
            line={line}
            highlightedAyah={highlightedAyah}
            recitationAyah={recitationAyah}
            text={text}
          />
        );
      })}
      <div
        className="pageNumberMark"
        dir={metadataLanguage === "arabic" ? "rtl" : "ltr"}
        aria-hidden="true"
      >
        {formatMetadataNumber(page.pageNumber, metadataLanguage)}
      </div>
      <PageSectionMark meta={meta} metadataLanguage={metadataLanguage} />
    </article>
  );
}, (previous, next) => {
  // Page chrome is derived afresh by the parent. Compare its scalar fields so
  // hover/menu updates do not walk every line of all four offscreen previews.
  const { meta: oldMeta, ...oldProps } = previous;
  const { meta: newMeta, ...newProps } = next;
  return Object.keys(oldProps).every(key =>
    Object.is(oldProps[key as keyof typeof oldProps], newProps[key as keyof typeof newProps]))
    && (oldMeta === newMeta || (!!oldMeta && !!newMeta
      && Object.keys(oldMeta).every(key => Object.is(oldMeta[key as keyof PageChromeMeta], newMeta[key as keyof PageChromeMeta]))));
});

function PageTurnPreviews({ pageNumbers, cache, renderPage }: {
  pageNumbers: number[];
  cache: AsyncPageCache<QpcPage>;
  renderPage: (pageNumber: number, page: QpcPage) => ReactNode;
}) {
  const [, setRevision] = useState(0);
  useEffect(() => {
    let cancelled = false;
    void Promise.allSettled(pageNumbers.map((page) => cache.load(page)))
      .then((results) => {
        if (cancelled) return;
        for (const result of results) {
          if (result.status === "rejected") console.warn("[Reader] Could not prepare page turn:", result.reason);
        }
        // A failed distant preview must not hide successfully prepared neighbors.
        setRevision((revision) => revision + 1);
      });
    return () => { cancelled = true; };
  }, [pageNumbers, cache]);
  // Cached destinations render synchronously after a turn. This avoids a one-render
  // preview gap that could make the next rapid command skip its fold animation.
  const pages = pageNumbers
    .map((pageNumber) => cache.peek(pageNumber))
    .filter((page): page is QpcPage => page !== undefined);
  return pages.map((page) => (
    <div key={page.pageNumber} data-preview-page={page.pageNumber}>
      {renderPage(page.pageNumber, page)}
    </div>
  ));
}

function PageTopMeta({
  meta,
  showSideLabel,
  metadataLanguage,
  currentTime,
  clockFormat,
}: {
  meta: PageChromeMeta | null;
  showSideLabel: boolean;
  metadataLanguage: MetadataLanguage;
  currentTime: Date;
  clockFormat: ClockFormat;
}) {
  if (!meta) {
    return null;
  }

  const labels = getMetadataLabels(metadataLanguage);
  const interfaceLabels = getInterfaceLabels(metadataLanguage);
  const juzNumber = meta.juz
    ? formatMetadataNumber(meta.juz, metadataLanguage)
    : "";

  return (
    <div className="pageTopMeta" aria-hidden="true">
      <span className="pageTopSurah" lang={metadataLanguage === "english" ? "en" : "ar"}
        dir={metadataLanguage === "english" ? "ltr" : "rtl"}>{meta.surahName}</span>
      <span className="pageTopCenter">
        <PageClock currentTime={currentTime} format={clockFormat} />
        {showSideLabel ? (
          <span className="pageSideLabel">
            {meta.side === "right" ? interfaceLabels.rightPage : interfaceLabels.leftPage}
          </span>
        ) : null}
      </span>
      <span
        className="pageTopSections"
        dir={metadataLanguage === "arabic" ? "rtl" : "ltr"}
      >
        {juzNumber ? (
          <span className="metadataPhrase">
            <span className="metadataLabel">{labels.juz}</span>
            <span className="metadataNumber">{juzNumber}</span>
          </span>
        ) : null}
      </span>
    </div>
  );
}

function PageSectionMark({
  meta,
  metadataLanguage,
}: {
  meta: PageChromeMeta | null;
  metadataLanguage: MetadataLanguage;
}) {
  if (!meta || (!meta.hizbStart && !meta.rubStart)) {
    return null;
  }

  const sectionMark = getPageSectionMark(meta, metadataLanguage);

  if (!sectionMark) {
    return null;
  }

  return (
    <div
      className="pageSectionMark"
      dir={metadataLanguage === "arabic" ? "rtl" : "ltr"}
      aria-hidden="true"
    >
      <span className="metadataPhrase">
        <span className="metadataLabel">{sectionMark.label}</span>
        <span className="metadataNumber">{sectionMark.number}</span>
      </span>
    </div>
  );
}

function useMinuteClock(enabled: boolean) {
  const [currentTime, setCurrentTime] = useState(() => new Date());

  useEffect(() => {
    if (!enabled) {
      return undefined;
    }

    let timer = 0;

    const synchronize = () => {
      window.clearTimeout(timer);
      const now = new Date();
      setCurrentTime(now);
      const millisecondsIntoMinute = now.getSeconds() * 1000 + now.getMilliseconds();
      timer = window.setTimeout(synchronize, 60_000 - millisecondsIntoMinute + 30);
    };

    synchronize();
    window.addEventListener("focus", synchronize);
    document.addEventListener("visibilitychange", synchronize);

    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("focus", synchronize);
      document.removeEventListener("visibilitychange", synchronize);
    };
  }, [enabled]);

  return currentTime;
}

const clockFormatters = {
  "12": new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h12",
  }),
  "24": new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }),
};

function formatClockTime(currentTime: Date, format: ClockFormat) {
  const formatter = clockFormatters[format];

  if (format === "24") {
    return formatter.format(currentTime);
  }

  const parts = formatter.formatToParts(currentTime);
  const hour = parts.find((part) => part.type === "hour")?.value ?? "";
  const minute = parts.find((part) => part.type === "minute")?.value ?? "";
  return `${hour}:${minute}`;
}

function PageClock({ currentTime, format }: { currentTime: Date; format: ClockFormat }) {

  return (
    <time dateTime={currentTime.toISOString()}>
      {formatClockTime(currentTime, format)}
    </time>
  );
}

export default function App() {
  const [data, setData] = useState<QulData | null>(null);
  const [qpcData, setQpcData] = useState<QpcData>(() => ({ pages: new Map() }));
  const [loadError, setLoadError] = useState<string | null>(null);
  const [pageLoadError, setPageLoadError] = useState(false);
  const [pageLoadAttempt, setPageLoadAttempt] = useState(0);
  const [settings, setSettings] = useState<ReaderSettings>(() => readSettings());
  const [pageInput, setPageInput] = useState(String(settings.page));
  const [isJumpOpen, setIsJumpOpen] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [readingToolsOpen, setReadingToolsOpen] = useState(false);
  const [playbackDockVisible, setPlaybackDockVisible] = useState(0);
  const [controlsVisible, setControlsVisible] = useState(() => !isRustNativeHost());
  const controlsHideTimerRef = useRef(0);
  const [searchQuery, setSearchQuery] = useState("");
  const deferredSearchQuery = useDeferredValue(searchQuery);
  const [surahQuery, setSurahQuery] = useState("");
  const [highlightedAyah, setHighlightedAyah] = useState<HighlightedAyah | null>(null);
  const [recitationAyah, setRecitationAyah] = useState<string | null>(null);
  const recitationAyahRef = useRef<string | null>(null);
  const [ayahActions, setAyahActions] = useState<{ run: HTMLElement; x: number; y: number } | null>(null);
  const [tafsirSelection, setTafsirSelection] = useState<TafsirSelection | null>(null);
  const [retainedTafsir, setRetainedTafsir] = useState<TafsirSelection | null>(null);
  const [listeningRequest, setListeningRequest] = useState<ListeningRequest | null>(null);
  const [readingToolsDismiss, setReadingToolsDismiss] = useState(0);
  const listeningRequestId = useRef(0);
  const tafsirContextPage = useRef<{ key: string; page: number } | null>(null);
  const panelSelection = tafsirSelection ?? retainedTafsir;
  useLayoutEffect(() => {
    if (tafsirSelection) { setRetainedTafsir(tafsirSelection); return; }
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      setRetainedTafsir(null); return;
    }
    const timer = window.setTimeout(() => setRetainedTafsir(null), 220);
    return () => window.clearTimeout(timer);
  }, [tafsirSelection]);

  const [browseTab, setBrowseTab] = useState<"surah" | "page" | "section">("surah");
  const [showReaderTips, setShowReaderTips] = useState(() => {
    try { return localStorage.getItem("quran-reader-onboarding-v2") !== "done"; } catch { return true; }
  });
  const [onboardingStep, setOnboardingStep] = useState(0);
  const [isFullscreen, setIsFullscreen] = useState(Boolean(document.fullscreenElement));

  const [tafsirSource, setTafsirSource] = useState<TafsirSource>("mukhtasar");
  const [ayahSearchIndex, setAyahSearchIndex] = useState<AyahSearchEntry[] | null>(null);
  const [isSearchDataLoading, setIsSearchDataLoading] = useState(false);
  const [readerFontsReady, setReaderFontsReady] = useState(false);
  const didLogFirstPage = useRef(false);
  const appShellRef = useRef<HTMLElement | null>(null);
  const lastNativeThemeRef = useRef<string | null>(null);
  const studyArrowRef = useRef<(direction: "left" | "right", repeated?: boolean) => boolean>(() => false);
  const readerViewportRef = useRef<HTMLElement | null>(null);
  const pageTurnPreviewsRef = useRef<HTMLDivElement | null>(null);
  const surahOptionsRef = useRef<HTMLDivElement | null>(null);
  const quranSearchInputRef = useRef<HTMLInputElement | null>(null);
  const outlinesRef = useRef<Promise<Record<number, string>> | null>(null);
  const [pageCache] = useState(() => new AsyncPageCache<QpcPage>(32, async (pageNumber) => {
    const page = await fetchJson<QpcPage>(`/experimental/digital-khatt/pages/p${pageNumber}.json`);
    if (page.pageNumber !== pageNumber || !Array.isArray(page.lines) || page.lines.length === 0) {
      throw new Error(`Invalid page data for page ${pageNumber}`);
    }
    return page;
  }));
  // Enhanced artwork has independent ownership: optional work never gates reading.
  const [shapedCache] = useState(() => new AsyncPageCache<QpcPage>(12, async (pageNumber) => {
    outlinesRef.current ??= fetchJson<Record<number, string>>("/experimental/digital-khatt/shaped/glyphs.json")
      .catch(error => { outlinesRef.current = null; throw error; });
    const [page, shaped, outlines] = await Promise.all([
      pageCache.load(pageNumber),
      fetchJson<ShapedPage>(`/experimental/digital-khatt/shaped/p${pageNumber}.json`),
      outlinesRef.current,
    ]);
    return { ...page, shaped, outlines };
  }));
  const [, setArtworkRevision] = useState(0);
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  const navigationRequestRef = useRef(0);
  const searchIndexLoadRef = useRef<Promise<AyahSearchEntry[]> | null>(null);
  const [readerScale, setReaderScale] = useState(1);
  const [contextualInkWeight, setContextualInkWeight] = useState(() =>
    window.devicePixelRatio < 1.5 && !["light", "sepia"].includes(settings.theme)
      && (settings.theme !== "system" || !window.matchMedia("(prefers-color-scheme: light)").matches));
  const currentTime = useMinuteClock(settings.showClock);
  const searchShortcutRef = useRef<SearchShortcut>(settings.searchShortcut);
  searchShortcutRef.current = settings.searchShortcut;
  const languageReadyRef = useRef(settings.hasChosenLanguage);
  languageReadyRef.current = settings.hasChosenLanguage;
  const hoveredAyahRef = useRef<string | null>(null);

  useEffect(() => {
    document.documentElement.lang = settings.metadataLanguage === "english" ? "en" : "ar";
  }, [settings.metadataLanguage]);

  useEffect(() => {
    window.clearTimeout(controlsHideTimerRef.current);
    if (isRustNativeHost()) {
      if (isJumpOpen || isSettingsOpen || readingToolsOpen) setControlsVisible(false);
      return;
    }
    if ((settings.hasChosenLanguage && showReaderTips) || isJumpOpen || isSettingsOpen || readingToolsOpen) {
      setControlsVisible(true);
      return;
    }
    controlsHideTimerRef.current = window.setTimeout(hideControls, 2200);
    return () => window.clearTimeout(controlsHideTimerRef.current);
  }, [isJumpOpen, isSettingsOpen, readingToolsOpen, settings.hasChosenLanguage, showReaderTips]);

  function revealControls(delay = 2200) {
    window.clearTimeout(controlsHideTimerRef.current);
    setControlsVisible(true);
    if (!isRustNativeHost() && !isJumpOpen && !isSettingsOpen && !readingToolsOpen) {
      controlsHideTimerRef.current = window.setTimeout(hideControls, delay);
    }
  }
  function hideControls() {
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.closest('.floatingControls')) {
      if (active.matches(':focus-visible')) return;
      appShellRef.current?.focus({ preventScroll: true });
    }
    setControlsVisible(false);
  }

  useEffect(() => {
    if (!isRustNativeHost() || !controlsVisible) return;
    const dismiss = (event: PointerEvent) => {
      if (event.target instanceof Element && !event.target.closest('.rustTitleBar, .floatingControls')) hideControls();
    };
    document.addEventListener('pointerdown', dismiss);
    return () => document.removeEventListener('pointerdown', dismiss);
  }, [controlsVisible]);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([
      document.fonts.load('48px "Digital Khatt V2"'),
      document.fonts.load('64px "QUL Surah Header Color"'),
      document.fonts.load('32px "QUL Surah Name V4"'),
    ]).then(() => document.fonts.ready).then(() => {
      if (!cancelled) setReaderFontsReady(true);
    }).catch((error: unknown) => {
      console.warn("[Reader] Could not prewarm all reader fonts:", error);
      if (!cancelled) setReaderFontsReady(true);
    });
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    const loadStartedAt = performance.now();
    Promise.all([
      fetchJson<Surah[]>("/data/qul/processed/surahs.json"),
      fetchJson<PageIndex[]>("/data/qul/processed/page-index.json"),
      fetchJson<JuzIndex[]>("/data/qul/processed/juz.json"),
      fetchJson<HizbIndex[]>("/data/qul/processed/hizb.json"),
      fetchJson<RubIndex[]>("/data/qul/processed/rub.json"),
      fetchJson<SurahStartPage[]>("/experimental/digital-khatt/surah-start-pages.json"),
    ])
      .then(([surahs, pageIndex, juz, hizb, rub, surahStarts]) => {
        setData({ surahs, pageIndex, juz, hizb, rub, surahStarts });
        console.info(
          `[Digital Khatt] Reader metadata loaded in ${Math.round(performance.now() - loadStartedAt)}ms.`,
        );
      })
      .catch((error: unknown) => {
        setLoadError(error instanceof Error ? error.message : "Could not load Mushaf data");
      });
  }, []);

  useEffect(() => {
    window.focus();
    appShellRef.current?.focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    } catch (error) {
      console.warn("[Reader] Could not save settings:", error);
    }
    setPageInput(String(settings.page));
  }, [settings]);

  useEffect(() => {
    if (isJumpOpen) {
      setPageInput(String(settings.page));
      setSurahQuery("");
    }
  }, [isJumpOpen, settings.page]);

  useEffect(() => {
    if (!highlightedAyah) {
      return undefined;
    }

    const timeout = window.setTimeout(() => {
      setHighlightedAyah(null);
    }, 5000);

    return () => window.clearTimeout(timeout);
  }, [highlightedAyah]);

  useEffect(() => {
    if (!tafsirSelection) return undefined;
    const key = `${tafsirSelection.surah}:${tafsirSelection.ayah}`;
    const runs = ayahRuns(key);
    for (const run of runs) run.classList.add("ayahRun-selected");
    return () => {
      for (const run of runs) run.classList.remove("ayahRun-selected");
    };
  }, [tafsirSelection, qpcData, settings.page]);

  useEffect(() => {
    if (!tafsirSelection) return;
    let cancelled = false;
    void loadSearchIndex().then(entries => {
      if (cancelled) return;
      const entry = entries.find(item => item.surah === tafsirSelection.surah && item.ayah === tafsirSelection.ayah);
      if (entry) setSettings(current => current.page === entry.pageNumber ? current : { ...current, page: entry.pageNumber });
    }).catch(() => {});
    return () => { cancelled = true; };
  }, [tafsirSelection]);

  useEffect(() => {
    if (!isJumpOpen && searchQuery.trim().length < 2) {
      return;
    }

    loadSearchIndex();
  }, [isJumpOpen, searchQuery]);

  useEffect(() => {
    if (isJumpOpen || (!searchQuery && !surahQuery)) {
      return undefined;
    }

    const timeout = window.setTimeout(() => {
      setSearchQuery("");
      setSurahQuery("");
    }, 50000);

    return () => window.clearTimeout(timeout);
  }, [isJumpOpen, searchQuery, surahQuery]);

  const qpcPageByNumber = useMemo(() => {
    return qpcData?.pages ?? new Map<number, QpcPage>();
  }, [qpcData]);

  const pageIndexByNumber = useMemo(() => {
    return new Map(data?.pageIndex.map((page) => [page.pageNumber, page]) ?? []);
  }, [data]);

  const hizbStartByPage = useMemo(() => {
    return new Map(data?.hizb.map((entry) => [entry.pageNumber, entry.hizb]) ?? []);
  }, [data]);

  const rubStartByPage = useMemo(() => {
    return new Map(data?.rub.map((entry) => [entry.pageNumber, entry.rub]) ?? []);
  }, [data]);

  const surahsById = useMemo(() => {
    return new Map(data?.surahs.map((surah) => [surah.id, surah]) ?? []);
  }, [data]);

  studyArrowRef.current = (direction, repeated = false) => {
    if (!tafsirSelection || isJumpOpen || isSettingsOpen) return false;
    if (repeated) return true;
    const step = direction === "left" ? 1 : -1;
    setTafsirSelection(current => {
      if (!current) return current;
      const count = surahsById.get(current.surah)?.versesCount;
      if (!count) return current;
      const ayah = current.ayah + step;
      if (ayah >= 1 && ayah <= count) return { ...current, ayah };
      const next = surahsById.get(current.surah + step);
      return next ? { surah: next.id, ayah: step > 0 ? 1 : next.versesCount } : current;
    });
    return true;
  };
  const surahStartPages = useMemo(() => {
    return new Map(data?.surahStarts.map((entry) => [entry.surah, entry.pageNumber]) ?? []);
  }, [data]);

  const surahMatches = useMemo(() => {
    if (!data) {
      return [];
    }

    const query = normalizeSearchText(surahQuery);
    return data.surahs
      .filter((surah) => !query || surahSearchText(surah).includes(query));
  }, [data, surahQuery]);

  const currentSurahId = useMemo(() => {
    const pageIndex = pageIndexByNumber.get(settings.page);
    return pageIndex?.firstSurah ?? null;
  }, [pageIndexByNumber, settings.page]);

  useLayoutEffect(() => {
    if (!isJumpOpen || surahQuery || currentSurahId === null) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      surahOptionsRef.current
        ?.querySelector<HTMLElement>(`[data-surah-id="${currentSurahId}"]`)
        ?.scrollIntoView({ block: "center" });
    });

    return () => window.cancelAnimationFrame(frame);
  }, [currentSurahId, isJumpOpen, surahMatches, surahQuery]);

  const findAyahs = useMemo(() => createAyahSearcher(ayahSearchIndex ?? []), [ayahSearchIndex]);

  const searchResults = useMemo(() => {
    const searchQuery = deferredSearchQuery;
    const normalizedInput = normalizeSearchText(searchQuery);
    const requestedPage = /^\d{1,3}$/.test(normalizedInput)
      ? Number(normalizedInput)
      : null;
    const pageResult: SearchResult[] = requestedPage !== null && requestedPage >= 1 && requestedPage <= PAGE_COUNT
      ? [{ type: "page", key: `page-${requestedPage}`, pageNumber: requestedPage }]
      : [];

    if (!isJumpOpen || !data || (searchQuery.trim().length < 2 && pageResult.length === 0)) {
      return [] satisfies SearchResult[];
    }

    const reference = parseQuranReference(searchQuery);
    const searchableQuery = reference ? searchQuery : stripTrailingVerseNumber(searchQuery);
    const queryVariants = getSearchTextVariants(searchableQuery);
    const query = queryVariants[0] ?? "";
    if (!query && !reference) {
      return pageResult;
    }

    const surahResults: SearchResult[] = data.surahs
      .filter((surah) => surahSearchText(surah).includes(query))
      .slice(0, 8)
      .map((surah) => ({
        type: "surah",
        key: `surah-${surah.id}`,
        surah,
        pageNumber: surahStartPages.get(surah.id) ?? 1,
      }));

    const ayahResults: SearchResult[] = findAyahs(queryVariants, reference)
      .flatMap((ayah) => {
        const surah = surahsById.get(ayah.surah);
        if (!surah) {
          return [];
        }

        return [{
          type: "ayah" as const,
          key: `ayah-${ayah.key}`,
          pageNumber: ayah.pageNumber,
          surah,
          ayah: ayah.ayah,
          firstWordId: ayah.firstWordId,
          lastWordId: ayah.lastWordId,
          snippet: getTextSnippet(ayah.text, searchQuery),
        }];
      });

    return [...pageResult, ...surahResults, ...ayahResults];
  }, [findAyahs, data, deferredSearchQuery, isJumpOpen, surahsById, surahStartPages]);

  const spread = useMemo(() => getSpreadPages(settings.page), [settings.page]);
  const isSpreadMode = settings.mode === "spread";
  const metadataLabels = getMetadataLabels(settings.metadataLanguage);
  const interfaceLabels = getInterfaceLabels(settings.metadataLanguage);
  const spreadStart = isSpreadMode ? spread.rightPage : settings.page;
  const canGoPrevious = isSpreadMode ? spreadStart > 1 : settings.page > 1;
  const canGoNext = isSpreadMode
    ? spreadStart + 2 <= PAGE_COUNT
    : settings.page < PAGE_COUNT;
  const readerStyle = {
    "--manual-text-scale": settings.textScale / 100,
    "--reader-fit-scale": readerScale,
  } as CSSProperties;
  const spreadNaturalWidth = PAGE_ASPECT_WIDTH * 2 + PAGE_GAP;
  const singleNaturalWidth = PAGE_ASPECT_WIDTH;
  const currentQpcPageNumbers = useMemo(() => {
    const pages = new Set<number>();
    const addPage = (page: number | null) => {
      if (page !== null && page >= 1 && page <= PAGE_COUNT) {
        pages.add(page);
      }
    };

    if (isSpreadMode) {
      addPage(spread.rightPage);
      addPage(spread.leftPage);
    } else {
      addPage(settings.page);
    }

    return Array.from(pages);
  }, [isSpreadMode, settings.page, spread.leftPage, spread.rightPage]);

  const preloadQpcPageNumbers = useMemo(() => {
    const pages = new Set<number>();
    const addPage = (page: number | null) => {
      if (page !== null && page >= 1 && page <= PAGE_COUNT) {
        pages.add(page);
      }
    };

    if (isSpreadMode) {
      for (const offset of [-2, 2]) {
        addPage(spread.rightPage + offset);
        addPage(spread.leftPage === null ? null : spread.leftPage + offset);
      }
    } else {
      for (const offset of [-2, -1, 1, 2]) {
        addPage(settings.page + offset);
      }
    }

    return Array.from(pages).filter((pageNumber) => !currentQpcPageNumbers.includes(pageNumber));
  }, [currentQpcPageNumbers, isSpreadMode, settings.page, spread.leftPage, spread.rightPage]);

  const readAheadQpcPageNumbers = useMemo(() => {
    const distance = settings.contextualText ? 4 : 8;
    const pages = new Set<number>();
    const addPage = (page: number | null) => {
      if (page !== null && page >= 1 && page <= PAGE_COUNT) pages.add(page);
    };

    if (isSpreadMode) {
      for (let offset = -distance; offset <= distance; offset += 2) {
        if (offset === 0) continue;
        addPage(spread.rightPage + offset);
        addPage(spread.leftPage === null ? null : spread.leftPage + offset);
      }
    } else {
      for (let offset = -distance; offset <= distance; offset++) {
        if (offset === 0) continue;
        addPage(settings.page + offset);
      }
    }

    return Array.from(pages);
  }, [isSpreadMode, settings.page, spread.leftPage, spread.rightPage, settings.contextualText]);

  useEffect(() => {
    if (!settings.contextualText || !data) return;
    let cancelled = false;
    const pages = currentQpcPageNumbers;
    void Promise.allSettled(preloadQpcPageNumbers.map(page => shapedCache.load(page))).then(() => {
      if (!cancelled) setArtworkRevision(n => n + 1);
    });
    // Retain the verified source renderer while artwork loads or if it fails.
    void Promise.allSettled(pages.map(page => shapedCache.load(page))).then(results => {
      if (cancelled) return;
      if (results.some(result => result.status === "fulfilled")) setArtworkRevision(n => n + 1);
      if (results.some(result => result.status === "rejected")) console.warn("[Digital Khatt] Optional artwork unavailable; retaining source text.");
    });
    return () => { cancelled = true; };
  }, [settings.contextualText, currentQpcPageNumbers, preloadQpcPageNumbers, shapedCache, data, pageLoadAttempt]);

  const areCurrentQpcPagesLoaded =
    readerFontsReady &&
    qpcData !== null &&
    currentQpcPageNumbers.length > 0 &&
    currentQpcPageNumbers.every((pageNumber) => qpcData.pages.has(pageNumber) || pageCache.peek(pageNumber));

  const { turnPage, releaseKeyboardTurns } = usePageDrag({
    viewport: readerViewportRef,
    previews: pageTurnPreviewsRef,
    pageKey: `${settings.mode}:${spreadStart}`,
    layoutKey: tafsirSelection ? "study" : "reader",
    page: spreadStart,
    spread: isSpreadMode,
    enabled: areCurrentQpcPagesLoaded && settings.hasChosenLanguage && !tafsirSelection && !isJumpOpen && !isSettingsOpen && !readingToolsOpen && !ayahActions,
    commandsEnabled: settings.hasChosenLanguage && !tafsirSelection && !isJumpOpen && !isSettingsOpen && !readingToolsOpen && !ayahActions,
    canGoNext,
    canGoPrevious,
    onTurn: (direction) => commitPageTurn(direction),
  });

  useEffect(() => {
    if (!data || !qpcData || !areCurrentQpcPagesLoaded || didLogFirstPage.current) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      didLogFirstPage.current = true;
      console.info(
        `[QUL] First page rendered in ${Math.round(performance.now() - APP_STARTED_AT)}ms.`,
      );
    });

    return () => window.cancelAnimationFrame(frame);
  }, [areCurrentQpcPagesLoaded, data, qpcData]);

  useEffect(() => {
    // Preview caching has no visible state and waits for navigation metadata.
    if (data) void Promise.allSettled(preloadQpcPageNumbers.map(page => pageCache.load(page)));
  }, [data, pageCache, preloadQpcPageNumbers]);

  useEffect(() => {
    if (currentQpcPageNumbers.length === 0) {
      return undefined;
    }

    let isCancelled = false;
    setPageLoadError(false);

    // Page identities come from saved settings, so the visible spread can load
    // alongside metadata. Optional previews wait for metadata to avoid competing
    // with the initial source/font requests.
    const visiblePages = Promise.all(currentQpcPageNumbers.map((pageNumber) => pageCache.load(pageNumber)));
    void visiblePages
      .then((pages) => {
        if (isCancelled) {
          return;
        }

        // Keep only the visible spread in React state; prefetching must not redraw it.
        const currentPages = pages.filter((page) => currentQpcPageNumbers.includes(page.pageNumber));
        setQpcData({ pages: new Map(currentPages.map((page) => [page.pageNumber, page])) });
      })
      .catch((error: unknown) => {
        if (isCancelled) return;
        setPageLoadError(true);
        console.warn("[Digital Khatt] Could not load current page data:", error);
      });

    return () => {
      isCancelled = true;
    };
  }, [currentQpcPageNumbers, pageCache, pageLoadAttempt]);

  useEffect(() => {
    if (!data || !areCurrentQpcPagesLoaded) return;
    // Distant pages warm in small idle batches. Cancel the queue on navigation;
    // otherwise old read-ahead requests compete with the next visible spread.
    const distantPages = readAheadQpcPageNumbers.filter(pageNumber =>
      !currentQpcPageNumbers.includes(pageNumber) && !preloadQpcPageNumbers.includes(pageNumber));
    let cancelled = false;
    let idleHandle = 0;
    let timeoutHandle = 0;
    let nextIndex = 0;
    const schedule = () => {
      if (cancelled || nextIndex >= distantPages.length) return;
      if (typeof window.requestIdleCallback === "function") {
        idleHandle = window.requestIdleCallback(warm, { timeout: 1200 });
      } else {
        timeoutHandle = globalThis.setTimeout(warm, 250);
      }
    };
    const warm = () => {
      idleHandle = 0;
      timeoutHandle = 0;
      if (cancelled) return;
      const batch = distantPages.slice(nextIndex, nextIndex + 2);
      nextIndex += batch.length;
      void Promise.allSettled(batch.map(pageNumber => settings.contextualText ? shapedCache.load(pageNumber) : pageCache.load(pageNumber)))
        .then(schedule);
    };
    schedule();
    return () => {
      cancelled = true;
      if (idleHandle) window.cancelIdleCallback(idleHandle);
      window.clearTimeout(timeoutHandle);
    };
  }, [areCurrentQpcPagesLoaded, currentQpcPageNumbers, data, pageCache, preloadQpcPageNumbers, readAheadQpcPageNumbers, settings.contextualText, shapedCache]);

  const targetReaderScale = useRef(readerScale);
  useLayoutEffect(() => {
    const viewport = readerViewportRef.current;
    if (!viewport) {
      return undefined;
    }

    const measuredViewport = viewport;
    let frame = 0;

    function updateScale() {
      const viewportStyle = window.getComputedStyle(measuredViewport);
      const horizontalPadding =
        (Number.parseFloat(viewportStyle.paddingLeft) || 0) +
        (Number.parseFloat(viewportStyle.paddingRight) || 0);
      const verticalPadding =
        (Number.parseFloat(viewportStyle.paddingTop) || 0) +
        (Number.parseFloat(viewportStyle.paddingBottom) || 0);
      const naturalWidth = isSpreadMode
        ? PAGE_ASPECT_WIDTH * 2 + PAGE_GAP
        : PAGE_ASPECT_WIDTH;
      const naturalHeight = PAGE_ASPECT_HEIGHT;
      const viewportWidth = Math.min(
        measuredViewport.clientWidth,
        window.visualViewport?.width ?? window.innerWidth,
      );
      const viewportHeight = Math.min(
        measuredViewport.clientHeight,
        window.visualViewport?.height ?? window.innerHeight,
      );
      const availableWidth = Math.max(0, viewportWidth - horizontalPadding);
      const availableHeight = Math.max(
        0,
        viewportHeight - verticalPadding,
      );
      const nextScale = Math.min(
        isFullscreen ? Number.POSITIVE_INFINITY : MAX_FIT_SCALE,
        availableWidth / naturalWidth,
        availableHeight / naturalHeight,
      );

      targetReaderScale.current = Number(nextScale.toFixed(4));
      setReaderScale((current) =>
        Math.abs(current - nextScale) > 0.003 ? Number(nextScale.toFixed(4)) : current,
      );
    }

    function scheduleUpdate() {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(updateScale);
    }

    updateScale();
    const observer = new ResizeObserver(scheduleUpdate);
    observer.observe(measuredViewport);
    observer.observe(document.documentElement);
    window.addEventListener("resize", scheduleUpdate);
    window.visualViewport?.addEventListener("resize", scheduleUpdate);
    window.visualViewport?.addEventListener("scroll", scheduleUpdate);

    return () => {
      observer.disconnect();
      window.removeEventListener("resize", scheduleUpdate);
      window.visualViewport?.removeEventListener("resize", scheduleUpdate);
      window.visualViewport?.removeEventListener("scroll", scheduleUpdate);
      window.cancelAnimationFrame(frame);
    };
  }, [isSpreadMode, Boolean(tafsirSelection), isFullscreen, playbackDockVisible]);

  const previousReaderBounds = useRef<DOMRect | null>(null);
  const readerLayoutAnimation = useRef<Animation | null>(null);
  useLayoutEffect(() => {
    // Wait for the synchronous fit update so one animation targets the final layout.
    if (Math.abs(readerScale - targetReaderScale.current) > 0.003) return;
    const surface = readerViewportRef.current?.querySelector<HTMLElement>(".readerScaleSurface");
    if (!surface) return;
    const previous = readerLayoutAnimation.current?.playState === "running"
      ? surface.getBoundingClientRect() : previousReaderBounds.current;
    readerLayoutAnimation.current?.cancel();
    const next = surface.getBoundingClientRect();
    previousReaderBounds.current = next;
    if (!previous || !next.width || !next.height || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    if (Math.abs(previous.x - next.x) + Math.abs(previous.y - next.y) + Math.abs(previous.width - next.width) < 1) return;
    const baseTransform = getComputedStyle(surface).transform;
    const animation = surface.animate([
      { transformOrigin: "0 0", transform: `translate(${previous.x - next.x}px, ${previous.y - next.y}px) scale(${previous.width / next.width}, ${previous.height / next.height}) ${baseTransform === "none" ? "" : baseTransform}` },
      { transformOrigin: "0 0", transform: baseTransform },
    ], { duration: 220, easing: "cubic-bezier(0.2, 0, 0, 1)" });
    readerLayoutAnimation.current = animation;
    // A layout animation can temporarily scale the measured canvas geometry.
    // Rasterize once at its final display size, without a continuous paint loop.
    void animation.finished.then(() => {
      if (readerLayoutAnimation.current === animation) refreshShapedInk();
    }, () => {});
  }, [readerScale, Boolean(tafsirSelection), areCurrentQpcPagesLoaded, isSpreadMode]);

  useEffect(() => {
    const viewport = readerViewportRef.current;
    if (!viewport) {
      return;
    }

    viewport.scrollTo({ left: 0, top: 0 });
  }, [settings.page, settings.mode]);

  function updateSettings(next: Partial<ReaderSettings>) {
    setSettings((current) => ({
      ...current,
      ...next,
      settingsVersion: SETTINGS_VERSION,
      page: normalizePage(next.page ?? current.page),
      textScale: clamp(next.textScale ?? current.textScale, 78, 150),
    }));
  }

  function chooseLanguage(metadataLanguage: MetadataLanguage) {
    updateSettings({
      metadataLanguage,
      hasChosenLanguage: true,
      languageSetupVersion: LANGUAGE_SETUP_VERSION,
    });
  }

  function loadSearchIndex(): Promise<AyahSearchEntry[]> {
    if (ayahSearchIndex) return Promise.resolve(ayahSearchIndex);
    if (searchIndexLoadRef.current) return searchIndexLoadRef.current;

    setIsSearchDataLoading(true);
    const request = fetchJson<AyahSearchEntry[]>(
      "/experimental/digital-khatt/ayah-search-index.json",
    );
    searchIndexLoadRef.current = request;
    void request
      .then((entries) => {
        setAyahSearchIndex(entries);
      })
      .catch((error: unknown) => {
        searchIndexLoadRef.current = null;
        console.warn("[QUL] Could not load Quran search index:", error);
      })
      .finally(() => {
        setIsSearchDataLoading(false);
      });
    return request;
  }

  function followRecitationAyah(key: string | null) {
    recitationAyahRef.current = key;
    setRecitationAyah(key);
    if (!key) return;
    void loadSearchIndex().then(entries => {
      if (recitationAyahRef.current !== key) return;
      const entry = entries.find(item => item.key === key);
      if (entry && !currentQpcPageNumbers.includes(entry.pageNumber)) {
        updateSettings({ page: entry.pageNumber });
      }
    });
  }

  function goPrevious(repeated = false, source: "discrete" | "keyboard" = "discrete") {
    turnPage("previous", repeated, source);
  }

  function goNext(repeated = false, source: "discrete" | "keyboard" = "discrete") {
    turnPage("next", repeated, source);
  }

  function commitPageTurn(direction: "next" | "previous") {
    setHighlightedAyah(null);
    const current = settingsRef.current;
    const currentSpread = getSpreadPages(current.page).rightPage;
    const delta = direction === "next" ? 1 : -1;
    const targetPage = normalizePage(
      current.mode === "spread" ? currentSpread + delta * 2 : current.page + delta,
    );
    const targetPages = current.mode === "spread"
      ? [getSpreadPages(targetPage).rightPage, getSpreadPages(targetPage).leftPage]
          .filter((page): page is number => page !== null)
      : [targetPage];
    const request = ++navigationRequestRef.current;
    const applyTurn = () => {
      if (navigationRequestRef.current !== request) return;
      setSettings((latestSettings) => {
        if (latestSettings.page !== current.page || latestSettings.mode !== current.mode) {
          return latestSettings;
        }
        return { ...latestSettings, page: targetPage };
      });
    };

    if (targetPages.every((page) => pageCache.peek(page))) {
      applyTurn();
      return;
    }

    void Promise.all(targetPages.map((page) => pageCache.load(page)))
      .then(applyTurn)
      .catch((error: unknown) => {
        if (navigationRequestRef.current !== request) return;
        setPageLoadError(true);
        console.warn("[Digital Khatt] Could not prepare destination page:", error);
      });
  }

  function handleNativeArrow(direction: "left" | "right", repeated = false) {
    const target = document.activeElement;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable)) {
      return;
    }
    if (studyArrowRef.current(direction, repeated)) return;
    if (direction === "left") {
      goNext(repeated, "keyboard");
      return;
    }

    goPrevious(repeated, "keyboard");
  }

  function ayahRuns(key: string) {
    return readerViewportRef.current?.querySelectorAll<HTMLElement>(
      `.pageSlot > .qulPage .ayahRun[data-ayah-key="${key}"]`,
    ) ?? [];
  }

  function setHoveredAyah(key: string | null) {
    if (hoveredAyahRef.current === key) return;
    if (hoveredAyahRef.current) {
      for (const run of ayahRuns(hoveredAyahRef.current)) run.classList.remove("ayahRun-hovered");
    }
    hoveredAyahRef.current = key;
    if (key) {
      for (const run of ayahRuns(key)) run.classList.add("ayahRun-hovered");
    }
  }

  function openTafsir(run: HTMLElement) {
    const key = run.dataset.ayahKey ?? "";
    const match = key.match(/^(\d+):(\d+)$/);
    if (!match) return;
    tafsirContextPage.current = { key, page: Number(run.closest<HTMLElement>("[data-page-number]")?.dataset.pageNumber) };
    setIsJumpOpen(false);
    setIsSettingsOpen(false);
    const nextSelection = {
      surah: Number(match[1]),
      ayah: Number(match[2]),
    };
    setTafsirSelection((current) =>
      current?.surah === nextSelection.surah && current.ayah === nextSelection.ayah
        ? null
        : nextSelection,
    );
  }

  function startContextListening(scope: "ayah" | "page" | "surah" | "quran" | "whole-surah" | "whole-page", selection = tafsirSelection) {
    if (!selection) return;
    const selected = selection;
    const entry = ayahSearchIndex?.find(item => item.surah === selected.surah && item.ayah === selected.ayah);
    const context = tafsirContextPage.current;
    const pageNumber = context?.key === selected.surah + ":" + selected.ayah && context.page
      ? context.page : entry?.pageNumber;
    const page = pageNumber ? qpcPageByNumber.get(pageNumber) ?? pageCache.peek(pageNumber) : undefined;
    const words = page?.lines.flatMap(line => line.words) ?? [];
    const first = words.at(0), last = words.at(-1);
    if ((scope === "page" || scope === "whole-page") && (!first || !last)) return;
    const start = scope === "whole-surah" ? { surah: selected.surah, ayah: 1 }
      : scope === "whole-page" && first ? { surah: first.surah, ayah: first.ayah } : selected;
    const end = scope === "ayah" ? selected
      : (scope === "page" || scope === "whole-page") && last ? { surah: last.surah, ayah: last.ayah }
      : scope === "quran" ? { surah: 114, ayah: 6 }
      : { surah: selected.surah, ayah: surahsById.get(selected.surah)?.versesCount ?? selected.ayah };
    const labels = {
      ayah: ["Selected ayah", "الآية المختارة"], page: ["To end of page", "حتى نهاية الصفحة"],
      surah: ["To end of surah", "حتى نهاية السورة"], quran: ["To end of Quran", "حتى نهاية القرآن"],
      "whole-surah": ["Whole surah", "السورة كاملة"], "whole-page": ["This page", "هذه الصفحة"],
    };
    setTafsirSelection(null);
    setListeningRequest({ id: ++listeningRequestId.current, ...start, end,
      label: labels[scope][settings.metadataLanguage === "arabic" ? 1 : 0] });
  }

  function submitPage(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const requestedPage = Number(pageInput);

    if (Number.isFinite(requestedPage)) {
      jumpToPage(requestedPage);
    } else {
      setPageInput(String(settings.page));
    }
  }

  function jumpToPage(page: number) {
    navigationRequestRef.current += 1;
    setHighlightedAyah(null);
    setTafsirSelection(null);
    setSearchQuery("");
    updateSettings({ page });
    setIsJumpOpen(false);
  }

  function jumpToAyah(result: Extract<SearchResult, { type: "ayah" }>) {
    navigationRequestRef.current += 1;
    setHighlightedAyah({
      surah: result.surah.id,
      ayah: result.ayah,
      pageNumber: result.pageNumber,
      firstWordId: result.firstWordId,
      lastWordId: result.lastWordId,
    });
    setTafsirSelection(null);
    setSearchQuery("");
    updateSettings({ page: result.pageNumber });
    setIsJumpOpen(false);
  }

  function jumpToSurah(surah: Surah) {
    const page = surahStartPages.get(surah.id);
    if (page) {
      navigationRequestRef.current += 1;
      setHighlightedAyah(null);
      setTafsirSelection(null);
      setSearchQuery("");
      setSurahQuery("");
      updateSettings({ page });
      setIsJumpOpen(false);
    }
  }

  function submitSurahSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (surahMatches.length > 0) {
      jumpToSurah(surahMatches[0]);
    }
  }

  function getPageMeta(pageNumber: number): PageChromeMeta | null {
    const pageIndex = pageIndexByNumber.get(pageNumber);
    const surah = pageIndex?.firstSurah ? surahsById.get(pageIndex.firstSurah) : null;

    if (!surah && !pageIndex?.juz) {
      return null;
    }

    return {
      surahName: (settings.metadataLanguage === "english" ? surah?.nameSimple : surah?.nameArabic) ?? "",
      juz: pageIndex?.juz ?? null,
      hizbStart: hizbStartByPage.get(pageNumber) ?? null,
      rubStart: rubStartByPage.get(pageNumber) ?? null,
      side: pageNumber % 2 === 1 ? "right" : "left",
    };
  }

  function renderReaderPage(pageNumber: number, preparedPage?: QpcPage) {
    const meta = getPageMeta(pageNumber);
    const showSideLabel = !isSpreadMode;
    const qpcPage = (settings.contextualText ? shapedCache.peek(pageNumber) : undefined)
      ?? preparedPage ?? qpcPageByNumber.get(pageNumber) ?? pageCache.peek(pageNumber);
    if (qpcPage) {
      return (
        <DigitalKhattPage
          key={`digital-khatt-page-${pageNumber}`}
          contextualText={settings.contextualText}
          contextualInkWeight={contextualInkWeight}
          renderScale={readerScale}
          inkTheme={settings.theme}
          page={qpcPage}
          surahsById={surahsById}
          meta={meta}
          showSideLabel={showSideLabel}
          metadataLanguage={settings.metadataLanguage}
          currentTime={currentTime}
          clockFormat={settings.clockFormat}
          highlightedAyah={highlightedAyah}
          recitationAyah={recitationAyah}
        />
      );
    }

    return null;
  }

  useEffect(() => {
    function openSearch() {
      setIsSettingsOpen(false);
      setIsJumpOpen(true);
      window.setTimeout(() => {
        quranSearchInputRef.current?.focus();
        quranSearchInputRef.current?.select();
      }, 0);
    }

    function onKeyDown(event: KeyboardEvent) {
      if (!languageReadyRef.current) return;
      const target = event.target as HTMLElement | null;
      if (target?.closest(".ayahActionsMenu")) return;
      const isTyping =
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        target?.isContentEditable;

      if (event.key === "Escape") {
        if (isRustNativeHost()) hideControls();
        const bridge = (window as Window & { chrome?: { webview?: { postMessage: (message: unknown) => void } } }).chrome?.webview;
        if (bridge) bridge.postMessage({ type: "fullscreen-request", active: false });
        else if (document.fullscreenElement) void document.exitFullscreen().catch(() => {});
        setIsJumpOpen(false);
        setIsSettingsOpen(false);
        setTafsirSelection(null);
        return;
      }

      if (event.key === "F11") {
        event.preventDefault();
        const bridge = (window as Window & { chrome?: { webview?: { postMessage: (message: unknown) => void } } }).chrome?.webview;
        if (!bridge) {
          if (document.fullscreenElement) void document.exitFullscreen().catch(() => {});
          else void document.documentElement.requestFullscreen().catch(() => {});
        }
        return;
      }

      const key = event.key.toLocaleLowerCase();
      const isSearchShortcut =
        ((event.ctrlKey || event.metaKey) && key === "f") ||
        (!isTyping &&
          searchShortcutRef.current === "s" &&
          !event.ctrlKey &&
          !event.metaKey &&
          !event.altKey &&
          key === "s");

      if (isSearchShortcut) {
        event.preventDefault();
        event.stopPropagation();
        openSearch();
        return;
      }

      if (isTyping) {
        return;
      }

      if (key === "t" && !event.ctrlKey && !event.metaKey && !event.altKey) {
        event.preventDefault();
        const run = readerViewportRef.current?.querySelector<HTMLElement>(".ayahRun-hovered")
          ?? readerViewportRef.current?.querySelector<HTMLElement>(".pageSlot > .qulPage .ayahRun");
        if (run) openTafsir(run);
        return;
      }

      if ((event.key === "ArrowLeft" || event.key === "ArrowRight") &&
          !event.ctrlKey && !event.metaKey && !event.altKey &&
          studyArrowRef.current(event.key === "ArrowLeft" ? "left" : "right", event.repeat)) {
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      if (event.key === "ArrowLeft") {
        event.preventDefault();
        goNext(event.repeat, "keyboard");
      }

      if (event.key === "ArrowRight") {
        event.preventDefault();
        goPrevious(event.repeat, "keyboard");
      }
    }

    function onKeyUp(event: KeyboardEvent) {
      if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
        releaseKeyboardTurns();
      }
    }

    function onWheel(event: WheelEvent) {
      if (event.ctrlKey) {
        event.preventDefault();
      }
    }

    window.addEventListener("keydown", onKeyDown, { capture: true });
    window.addEventListener("keyup", onKeyUp, { capture: true });
    window.addEventListener("wheel", onWheel, { capture: true, passive: false });
    return () => {
      window.removeEventListener("keydown", onKeyDown, { capture: true });
      window.removeEventListener("keyup", onKeyUp, { capture: true });
      window.removeEventListener("wheel", onWheel, { capture: true });
    };
  }, []);

  useEffect(() => {
    const webview = (
      window as Window & {
        chrome?: {
          webview?: {
            addEventListener: (
              eventName: "message",
              listener: (event: MessageEvent<string>) => void,
            ) => void;
            removeEventListener: (
              eventName: "message",
              listener: (event: MessageEvent<string>) => void,
            ) => void;
          };
        };
      }
    ).chrome?.webview;

    if (!webview) {
      return undefined;
    }

    function onWebViewMessage(event: MessageEvent<string>) {
      if (!languageReadyRef.current) return;
      if (event.data === "native-open-search") {
        setIsSettingsOpen(false);
        setTafsirSelection(null);
        setIsJumpOpen(true);
        window.setTimeout(() => {
          quranSearchInputRef.current?.focus();
          quranSearchInputRef.current?.select();
        }, 0);
        return;
      }

      if (event.data === "native-toggle-controls") {
        setControlsVisible(value => !value);
        return;
      }

      if (event.data === "native-fullscreen-on" || event.data === "native-fullscreen-off") {
        setIsFullscreen(event.data === "native-fullscreen-on");
        return;
      }

      if (event.data === "native-arrow-left") {
        handleNativeArrow("left");
      }

      if (event.data === "native-arrow-right") {
        handleNativeArrow("right");
      }

      if (event.data === "native-arrow-left-repeat") {
        handleNativeArrow("left", true);
      }

      if (event.data === "native-arrow-right-repeat") {
        handleNativeArrow("right", true);
      }

      if (event.data === "native-arrow-release") {
        releaseKeyboardTurns();
      }
    }

    webview.addEventListener("message", onWebViewMessage);
    return () => webview.removeEventListener("message", onWebViewMessage);
  }, []);

  const nativeWebView = (window as Window & { chrome?: { webview?: { postMessage: (message: unknown) => void } } }).chrome?.webview;
  const isNativeHost = Boolean(nativeWebView);
  const isRustHost = isRustNativeHost();

  useEffect(() => {
    if (isRustHost) nativeWebView?.postMessage({ type: "reader-ready" });
  }, [isRustHost, nativeWebView]);

  useEffect(() => {
    const updateFullscreenState = () => setIsFullscreen(Boolean(document.fullscreenElement));
    document.addEventListener("fullscreenchange", updateFullscreenState);
    return () => document.removeEventListener("fullscreenchange", updateFullscreenState);
  }, []);
  function toggleFullscreen() {
    if (nativeWebView) {
      nativeWebView.postMessage({ type: "fullscreen-request", active: !isFullscreen });
      return;
    }
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => {});
    else void document.documentElement.requestFullscreen().catch(() => {});
  }
  function dismissOnboarding() {
    setShowReaderTips(false);
    try { localStorage.setItem("quran-reader-onboarding-v2", "done"); } catch { /* Optional preference. */ }
  }
  useEffect(() => {
    nativeWebView?.postMessage({
      type: "reader-controls-state",
      open: controlsVisible,
      language: settings.metadataLanguage,
    });
  }, [controlsVisible, nativeWebView, settings.metadataLanguage]);

  useEffect(() => {
    const scheme = window.matchMedia("(prefers-color-scheme: light)");
    const denseDisplay = window.matchMedia("(min-resolution: 1.5dppx)");
    const sendTheme = () => {
      const theme = settings.theme === "system" ? (scheme.matches ? "light" : "dark") : settings.theme;
      setContextualInkWeight(!denseDisplay.matches && theme !== "light" && theme !== "sepia");
      const previous = lastNativeThemeRef.current;
      lastNativeThemeRef.current = theme;
      nativeWebView?.postMessage({
        type: "reader-theme",
        theme,
        animate: previous !== null && previous !== theme && !window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      });
    };
    sendTheme();
    scheme.addEventListener("change", sendTheme);
    denseDisplay.addEventListener("change", sendTheme);
    return () => {
      scheme.removeEventListener("change", sendTheme);
      denseDisplay.removeEventListener("change", sendTheme);
    };
  }, [nativeWebView, settings.theme]);

  return (
    <main
      ref={appShellRef}
      data-interface-language={settings.metadataLanguage}
      className={`appShell ${playbackDockVisible ? "playbackDockVisible" : ""} ${isRustHost ? "rustHost" : ""} ${isJumpOpen || isSettingsOpen || readingToolsOpen ? "toolPanelOpen" : ""} highlight-${settings.highlightColor} ${isFullscreen ? "fullscreenReader" : ""} theme-${settings.theme} sizing-${settings.sizePreset} ${settings.showClock ? "clock-visible" : "clock-hidden"} ${tafsirSelection ? "studyOpen" : retainedTafsir ? "studyClosing" : ""} ${controlsVisible ? "controlsVisible" : "controlsHidden"}`}
      style={{ ...readerStyle, "--playback-dock-reserve": `${playbackDockVisible}px` } as CSSProperties}
      tabIndex={-1}
      onPointerMove={(event) => { if (!isRustHost && event.clientY <= 12) revealControls(); }}
      onFocusCapture={(event) => {
        if (event.target instanceof HTMLElement && event.target.closest(".floatingControls")) {
          revealControls(3000);
        }
      }}
    >
      <RustTitleBar metadataLanguage={settings.metadataLanguage} readerControlsVisible={controlsVisible && !isJumpOpen && !isSettingsOpen && !readingToolsOpen}
        controlsEnabled={settings.hasChosenLanguage} searchOpen={isJumpOpen}
        onSearch={() => { setIsJumpOpen(open => !open); setIsSettingsOpen(false); setTafsirSelection(null); hideControls(); }}
        onToggleControls={() => {
          const menuWasOpen = controlsVisible && !isJumpOpen && !isSettingsOpen && !readingToolsOpen;
          setIsJumpOpen(false); setIsSettingsOpen(false); setReadingToolsDismiss(value => value + 1);
          if (menuWasOpen) hideControls(); else revealControls(6000);
        }} />
      {!settings.hasChosenLanguage ? <LanguageSetup onChoose={chooseLanguage} /> : null}
      {settings.hasChosenLanguage && !isRustHost && !isFullscreen && !tafsirSelection ? (
        <button className="sideToolsAccess" type="button" aria-controls="reader-controls"
          aria-expanded={controlsVisible} onClick={() => controlsVisible ? hideControls() : revealControls(6000)}>
          <span aria-hidden="true">☰</span>
          <span>{settings.metadataLanguage === "arabic" ? "الخيارات" : "Options"}</span>
        </button>
      ) : null}
      {settings.hasChosenLanguage && showReaderTips && !tafsirSelection && !isJumpOpen && !isSettingsOpen && !readingToolsOpen ? (
        <aside className="readerTips" dir={settings.metadataLanguage === "arabic" ? "rtl" : "ltr"} aria-label={settings.metadataLanguage === "arabic" ? "التعريف بالتطبيق" : "App introduction"}>
          <div className="readerTipsProgress" aria-hidden="true">{[0, 1, 2].map(step => <span key={step} className={step === onboardingStep ? "current" : ""} />)}</div>
          <strong>{settings.metadataLanguage === "arabic"
            ? ["مرحبًا بك في قارئ المصحف", "افتح التفسير من الآية", "اقرأ بالطريقة التي تناسبك"][onboardingStep]
            : ["Welcome to Quran Mushaf Reader", "Open explanation from an ayah", "Read in the way that suits you"][onboardingStep]}</strong>
          <p>{settings.metadataLanguage === "arabic"
            ? [isRustHost ? "استخدم زري البحث والاستماع في الشريط العلوي. افتح الخيارات للإعدادات وملء الشاشة والورد اليومي." : "اضغط «البحث والخيارات» بجانب اسم التطبيق لإظهار أدوات البحث والإعدادات والاستماع والورد اليومي.", "انقر بزر الفأرة الأيمن على أي آية لعرض خيارات التفسير والاستماع، وانقر على الآية نفسها مرة أخرى لإغلاق التفسير. استخدم مفتاحي السهم للتنقل بين الآيات أثناء فتح التفسير.", "ابدأ وردًا يوميًا وحدد هدفًا بالصفحات أو الدقائق. يمكنك إظهار المؤقت أو تفعيل تذكير يومي، واستخدام ملء الشاشة من زر ⛶ أو المفتاح F11."][onboardingStep]
            : [isRustHost ? "Use Search and Listen in the top bar. Open Options for settings, full screen, and Daily reading." : "Select “Search & options” beside the app name to reveal search, settings, listening, and Daily reading tools.", "Right-click an ayah for tafsir and listening options. Right-click the same ayah again to close its explanation. Use the arrow keys to move between ayahs while the explanation is open.", "Start Daily reading with a page or minute goal. You can show a timer, enable a daily reminder, and use full screen with the ⛶ button or F11."][onboardingStep]}</p>
          <div className="readerTipsActions">
            <button className="readerTipsSkip" onClick={dismissOnboarding}>{settings.metadataLanguage === "arabic" ? "تخطي" : "Skip"}</button>
            {onboardingStep > 0 ? <button onClick={() => setOnboardingStep(step => step - 1)}>{settings.metadataLanguage === "arabic" ? "السابق" : "Back"}</button> : null}
            <button className="readerTipsPrimary" onClick={() => onboardingStep < 2 ? setOnboardingStep(step => step + 1) : dismissOnboarding()}>{onboardingStep < 2 ? (settings.metadataLanguage === "arabic" ? "التالي" : "Next") : (settings.metadataLanguage === "arabic" ? "ابدأ القراءة" : "Start reading")}</button>
          </div>
        </aside>
      ) : null}
      {isFullscreen && !tafsirSelection ? <nav className="fullscreenAccess" aria-label={settings.metadataLanguage === "arabic" ? "خيارات العرض" : "View options"}>
        <button type="button" aria-controls="reader-controls" aria-expanded={controlsVisible} onClick={() => controlsVisible ? hideControls() : revealControls(4000)}>{interfaceLabels.searchAndOptions}</button>
        <button type="button" onClick={toggleFullscreen}>{settings.metadataLanguage === "arabic" ? "إنهاء ملء الشاشة" : "Exit full screen"} <span aria-hidden="true">⤢</span></button>
      </nav> : null}      {settings.hasChosenLanguage && !controlsVisible && (!isNativeHost && !isFullscreen) ? (
        <button
          className="readerControlsReveal"
          type="button"
          aria-controls="reader-controls"
          aria-label={interfaceLabels.showReaderControls}
          title={interfaceLabels.showReaderControls}
          onClick={() => revealControls(4000)}
        >
          <span>{interfaceLabels.searchAndOptions}</span>
          <span className="readerControlsRevealChevron" aria-hidden="true">⌄</span>
        </button>
      ) : null}
      <header id="reader-controls" className="floatingControls" dir={settings.metadataLanguage === "arabic" ? "rtl" : "ltr"} aria-label={interfaceLabels.readerControls}
        onPointerEnter={() => window.clearTimeout(controlsHideTimerRef.current)}
        onPointerLeave={() => revealControls(1200)}
        onBlur={() => revealControls(1200)}>
        <button
          className="readerControl readerSearchControl"
          type="button"
          aria-expanded={isJumpOpen}
          aria-label={interfaceLabels.openJump}
          title={interfaceLabels.jumpTo}
          onClick={() => {
            setIsJumpOpen((isOpen) => !isOpen);
            setIsSettingsOpen(false);
            setTafsirSelection(null);
          }}
        >
          <span className="searchControlIcon" aria-hidden="true" />
          <span className="readerControlLabel">{interfaceLabels.search}</span>
        </button>

        <div id="reading-controls-host" style={{ display: "contents" }} />
        <button className="readerControl fullscreenControl" type="button" aria-pressed={isFullscreen}
          aria-label={isFullscreen ? (settings.metadataLanguage === "arabic" ? "إنهاء ملء الشاشة" : "Exit full screen") : (settings.metadataLanguage === "arabic" ? "ملء الشاشة" : "Full screen")}
          title={isFullscreen ? (settings.metadataLanguage === "arabic" ? "إنهاء ملء الشاشة (F11 أو Esc)" : "Exit full screen (F11 or Esc)") : (settings.metadataLanguage === "arabic" ? "ملء الشاشة (F11)" : "Full screen (F11)")}
          onClick={toggleFullscreen}><span className="fullscreenControlIcon" aria-hidden="true">{isFullscreen ? "⤢" : "⛶"}</span><span className="nativeToolLabel">{isFullscreen ? (settings.metadataLanguage === "arabic" ? "إنهاء ملء الشاشة" : "Exit full screen") : (settings.metadataLanguage === "arabic" ? "ملء الشاشة" : "Full screen")}</span></button>
        <button
          className="readerControl readerSettingsControl"
          type="button"
          aria-expanded={isSettingsOpen}
          aria-label={interfaceLabels.openSettings}
          title={interfaceLabels.settings}
          onClick={() => {
            setIsSettingsOpen((isOpen) => !isOpen);
            setIsJumpOpen(false);
            setTafsirSelection(null);
          }}
        >
          <span className="settingsControlIcon" aria-hidden="true">&#9881;</span>
          <span className="nativeToolLabel">{interfaceLabels.settings}</span>
        </button>
      </header>

      {!tafsirSelection ? <button
        className="sideNavButton sideNavNext"
        type="button"
        aria-label={interfaceLabels.nextPage}
        title={interfaceLabels.nextPage}
        onPointerDown={(event) => {
          event.preventDefault();
          goNext();
        }}
        onClick={(event) => {
          if (event.detail === 0) {
            goNext();
          }
        }}
        disabled={!canGoNext}
      >
        &#8249;
      </button> : null}

      {!tafsirSelection ? <button
        className="sideNavButton sideNavPrevious"
        type="button"
        aria-label={interfaceLabels.previousPage}
        title={interfaceLabels.previousPage}
        onPointerDown={(event) => {
          event.preventDefault();
          goPrevious();
        }}
        onClick={(event) => {
          if (event.detail === 0) {
            goPrevious();
          }
        }}
        disabled={!canGoPrevious}
      >
        &#8250;
      </button> : null}

      {isSettingsOpen ? (
        <aside
          className="panel settingsPanel"
          aria-label={interfaceLabels.settings}
          dir={settings.metadataLanguage === "arabic" ? "rtl" : "ltr"}
          lang={settings.metadataLanguage === "arabic" ? "ar" : "en"}
        >
          <div className="panelHeader">
            <h2>{interfaceLabels.settings}</h2>
            <button
              type="button"
              aria-label={interfaceLabels.closeSettings}
              onClick={() => setIsSettingsOpen(false)}
            >
              &#215;
            </button>
          </div>

          <fieldset className="settingGroup languageSetting">
            <legend><span lang="en">Language</span> / <span lang="ar" dir="rtl">اللغة</span></legend>
            <LanguageChoices selected={settings.metadataLanguage} onChoose={chooseLanguage} />
          </fieldset>

          <label className="settingGroup">
            <span>{interfaceLabels.theme}</span>
            <select aria-label={interfaceLabels.theme} value={settings.theme} onChange={(event) => updateSettings({ theme: event.target.value as ThemeMode })}>
              <option value="dark">{interfaceLabels.themeGraphite}</option>
              <option value="black">{interfaceLabels.themeBlack}</option>
              <option value="navy">{interfaceLabels.themeNavy}</option>
              <option value="light">{interfaceLabels.themeLight}</option>
              <option value="sepia">{interfaceLabels.themeSepia}</option>
              <option value="system">{interfaceLabels.themeSystem}</option>
            </select>
          </label>

          <label className="settingGroup">
            <span>{settings.metadataLanguage === "arabic" ? "لون تمييز الآية" : "Ayah highlight color"}</span>
            <select aria-label={settings.metadataLanguage === "arabic" ? "لون تمييز الآية" : "Ayah highlight color"} value={settings.highlightColor} onChange={event => updateSettings({ highlightColor: event.target.value as ReaderSettings["highlightColor"] })}>
              <option value="auto">{settings.metadataLanguage === "arabic" ? "تلقائي حسب المظهر" : "Automatic (match theme)"}</option>
              <option value="green">{settings.metadataLanguage === "arabic" ? "أخضر" : "Green"}</option>
              <option value="gold">{settings.metadataLanguage === "arabic" ? "ذهبي" : "Gold"}</option>
            </select>
          </label>
          <label className="settingToggle"><span>{settings.metadataLanguage === "arabic" ? "إطالة الحروف السياقية (اختياري)" : "Contextual letter elongation (optional)"}</span><input type="checkbox" checked={settings.contextualText} onChange={event => updateSettings({ contextualText: event.target.checked })} /></label>
          <label className="settingGroup tafsirTextSizeSetting">
            <span>{interfaceLabels.tafsirTextSize}</span>
            <span className="rangeSettingValue">
              <input
                aria-label={interfaceLabels.tafsirTextSize}
                type="range"
                min="16"
                max="26"
                value={settings.tafsirTextSize}
                onChange={(event) => updateSettings({ tafsirTextSize: Number(event.target.value) })}
              />
              <output>{settings.tafsirTextSize}</output>
            </span>
          </label>

          <fieldset className="settingGroup">
            <legend>{interfaceLabels.pageView}</legend>
            <div className="segmented" aria-label={interfaceLabels.pageLayout}>
              <button type="button" aria-pressed={settings.mode === "single"} onClick={() => updateSettings({ mode: "single" })}>
                {interfaceLabels.onePage}
              </button>
              <button type="button" aria-pressed={settings.mode === "spread"} onClick={() => updateSettings({ mode: "spread" })}>
                {interfaceLabels.twoPages}
              </button>
            </div>
          </fieldset>

          <label className="settingToggle">
            <span>
              <strong>{interfaceLabels.pageClock}</strong>
              <small>{interfaceLabels.pageClockDescription}</small>
            </span>
            <input
              type="checkbox"
              role="switch"
              checked={settings.showClock}
              onChange={(event) => updateSettings({ showClock: event.target.checked })}
            />
          </label>

          <fieldset className="settingGroup" disabled={!settings.showClock}>
            <legend>{interfaceLabels.timeFormat}</legend>
            <div className="segmented" aria-label={interfaceLabels.timeFormat}>
              <button
                type="button"
                aria-pressed={settings.clockFormat === "12"}
                onClick={() => updateSettings({ clockFormat: "12" })}
              >
                {interfaceLabels.twelveHour}
              </button>
              <button
                type="button"
                aria-pressed={settings.clockFormat === "24"}
                onClick={() => updateSettings({ clockFormat: "24" })}
              >
                {interfaceLabels.twentyFourHour}
              </button>
            </div>
          </fieldset>

          <fieldset className="settingGroup shortcutSettings">
            <legend>{interfaceLabels.keyboardShortcuts}</legend>
            <div className="shortcutRow"><span>{settings.metadataLanguage === "arabic" ? "تفسير الآية تحت المؤشر" : "Study the ayah under your pointer"}</span><kbd>{settings.metadataLanguage === "arabic" ? "زر الفأرة الأيمن / T" : "Right-click / T"}</kbd></div>
            <div className="shortcutRow"><span>{settings.metadataLanguage === "arabic" ? "إغلاق النافذة" : "Close a panel"}</span><kbd>Esc</kbd></div>
            <label className="shortcutRow">
              <span>{interfaceLabels.openSearchShortcut}</span>
              <select
                value={settings.searchShortcut}
                onChange={(event) =>
                  updateSettings({ searchShortcut: event.target.value as SearchShortcut })
                }
              >
                <option value="ctrl-f">Ctrl + F</option>
                <option value="s">S</option>
              </select>
            </label>
            <div className="shortcutRow">
              <span>{interfaceLabels.nextPageShortcut}</span>
              <kbd>←</kbd>
            </div>
            <div className="shortcutRow">
              <span>{interfaceLabels.previousPageShortcut}</span>
              <kbd>→</kbd>
            </div>
          </fieldset>

          <button className="resetButton" type="button" onClick={() => setSettings({
            ...defaultSettings,
            metadataLanguage: settings.metadataLanguage,
            hasChosenLanguage: true,
            languageSetupVersion: LANGUAGE_SETUP_VERSION,
          })}>
            {interfaceLabels.resetSettings}
          </button>
        </aside>
      ) : null}

      {isJumpOpen && data ? (
        <aside
          id="quran-search-panel" className="panel jumpPanel"
          aria-label={interfaceLabels.jumpTo}
          dir={settings.metadataLanguage === "arabic" ? "rtl" : "ltr"}
          lang={settings.metadataLanguage === "arabic" ? "ar" : "en"}
        >
          <div className="panelHeader">
            <h2>{settings.metadataLanguage === "arabic" ? "ابحث في القرآن" : "Find your place"}</h2>
            <button
              type="button"
              aria-label={interfaceLabels.closeJump}
              onClick={() => setIsJumpOpen(false)}
            >
              &#215;
            </button>
          </div>

          <div className="primaryQuranSearch">
            <label className="settingGroup">
              <span>{interfaceLabels.searchQuran}</span>
              <input
                ref={quranSearchInputRef}
                className="searchInput"
                type="search"
                placeholder={interfaceLabels.searchPlaceholder}
                value={searchQuery}
                onChange={(event) => setSearchQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key !== "Enter" || event.nativeEvent.isComposing) return;
                  event.preventDefault();
                  // Use the input immediately: deferred results may still show the previous query.
                  const input = normalizeSearchText(event.currentTarget.value);
                  if (/^\d{1,3}$/.test(input)) {
                    const page = Number(input);
                    if (page >= 1 && page <= PAGE_COUNT) jumpToPage(page);
                    return;
                  }
                  if (searchQuery !== deferredSearchQuery) return;
                  const first = searchResults[0];
                  if (first?.type === "ayah") jumpToAyah(first);
                  else if (first) jumpToPage(first.pageNumber);
                }}
              />
            </label>

            {isSearchDataLoading && searchQuery.trim().length >= 2 && searchResults.length === 0 ? (
              <div className="searchLoading">{interfaceLabels.preparingSearch}</div>
            ) : null}

            <div className="searchResults" aria-busy={searchQuery !== deferredSearchQuery}>
              {searchResults.map((result) => (
                <button
                  key={result.key}
                  type="button"
                  onClick={() => {
                    if (result.type === "ayah") {
                      jumpToAyah(result);
                    } else {
                      jumpToPage(result.pageNumber);
                    }
                  }}
                >
                  {result.type === "page" ? (
                    <>
                      <span className="directPageResult">
                        {settings.metadataLanguage === "arabic" ? "الانتقال إلى الصفحة" : "Go to page"}
                      </span>
                      <small>{formatMetadataNumber(result.pageNumber, settings.metadataLanguage)}</small>
                    </>
                  ) : result.type === "surah" ? (
                    <>
                      <span>
                        {settings.metadataLanguage === "arabic"
                          ? `سورة ${result.surah.nameArabic} — ${formatMetadataNumber(result.surah.id, "arabic")}`
                          : `Surah — ${result.surah.id} — ${result.surah.nameSimple}`}
                      </span>
                      <small>
                        {metadataLabels.page}{" "}
                        {formatMetadataNumber(result.pageNumber, settings.metadataLanguage)}
                      </small>
                    </>
                  ) : (
                    <>
                      <span>
                        {settings.metadataLanguage === "arabic"
                          ? `سورة ${result.surah.nameArabic} — ${formatMetadataNumber(result.surah.id, "arabic")}:${formatMetadataNumber(result.ayah, "arabic")}`
                          : `Surah ${result.surah.nameSimple} — ${result.surah.id}:${result.ayah}`}
                      </span>
                      <small>
                        {metadataLabels.page}{" "}
                        {formatMetadataNumber(result.pageNumber, settings.metadataLanguage)}
                        {" — "}
                        {result.snippet}
                      </small>
                    </>
                  )}
                </button>
              ))}
            </div>

            {!isSearchDataLoading && searchQuery === deferredSearchQuery && searchQuery.trim().length >= 1 && searchResults.length === 0 ? (
              <div className="searchNoResults">
                {interfaceLabels.noSearchResults}
              </div>
            ) : null}
          </div>

          <div className="jumpBrowseSection" hidden={searchQuery.trim().length > 0}>
          <div className="jumpBrowseDivider"><span>{interfaceLabels.browseQuran}</span></div>
          <div className="segmented browseTabs" aria-label={settings.metadataLanguage === "arabic" ? "التصفح حسب" : "Browse by"}>
            <button aria-pressed={browseTab === "surah"} onClick={() => setBrowseTab("surah")}>{interfaceLabels.surah}</button>
            <button aria-pressed={browseTab === "page"} onClick={() => setBrowseTab("page")}>{interfaceLabels.page}</button>
            <button aria-pressed={browseTab === "section"} onClick={() => setBrowseTab("section")}>{settings.metadataLanguage === "arabic" ? "رقم الجزء / الحزب" : "Juz, Hizb & Rub"}</button>
          </div>

          <form className="jumpPageForm" hidden={browseTab !== "page"} onSubmit={submitPage}>
            <label>
              <span>{interfaceLabels.page}</span>
              <input
                aria-label={interfaceLabels.jumpToPage}
                inputMode="numeric"
                min={1}
                max={PAGE_COUNT}
                type="number"
                value={pageInput}
                onChange={(event) => setPageInput(event.target.value)}
              />
            </label>
            <button type="submit">{interfaceLabels.go}</button>
          </form>

          <div
            className="jumpSectionGrid"
            hidden={browseTab !== "section"}
            dir={settings.metadataLanguage === "arabic" ? "rtl" : "ltr"}
          >
            <label className="settingGroup">
              <span>{metadataLabels.juz}</span>
              <select
                value=""
                aria-label={metadataLabels.selectJuz}
                onChange={(event) => {
                  const juz = data.juz.find((entry) => entry.juz === Number(event.target.value));
                  if (juz) {
                    jumpToPage(juz.pageNumber);
                  }
                }}
              >
                <option value="" disabled>
                  {metadataLabels.selectJuz}
                </option>
                {data.juz.map((entry) => (
                  <option key={entry.juz} value={entry.juz}>
                    {formatSectionOption(
                      metadataLabels.juz,
                      entry.juz,
                      entry.pageNumber,
                      settings.metadataLanguage,
                    )}
                  </option>
                ))}
              </select>
            </label>

            <label className="settingGroup">
              <span>{metadataLabels.hizb}</span>
              <select
                value=""
                aria-label={metadataLabels.selectHizb}
                onChange={(event) => {
                  const hizb = data.hizb.find(
                    (entry) => entry.hizb === Number(event.target.value),
                  );
                  if (hizb) {
                    jumpToPage(hizb.pageNumber);
                  }
                }}
              >
                <option value="" disabled>
                  {metadataLabels.selectHizb}
                </option>
                {data.hizb.map((entry) => (
                  <option key={entry.hizb} value={entry.hizb}>
                    {formatSectionOption(
                      metadataLabels.hizb,
                      entry.hizb,
                      entry.pageNumber,
                      settings.metadataLanguage,
                    )}
                  </option>
                ))}
              </select>
            </label>

            <label className="settingGroup">
              <span>{metadataLabels.rub}</span>
              <select
                value=""
                aria-label={metadataLabels.selectRub}
                onChange={(event) => {
                  const rub = data.rub.find(
                    (entry) => entry.rub === Number(event.target.value),
                  );
                  if (rub) {
                    jumpToPage(rub.pageNumber);
                  }
                }}
              >
                <option value="" disabled>
                  {metadataLabels.selectRub}
                </option>
                {data.rub.map((entry) => (
                  <option key={entry.rub} value={entry.rub}>
                    {formatSectionOption(
                      metadataLabels.rub,
                      entry.rub,
                      entry.pageNumber,
                      settings.metadataLanguage,
                    )}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <div className="settingGroup surahPicker" hidden={browseTab !== "surah"}>
            <form className="surahSearchForm" onSubmit={submitSurahSearch}>
              <label>
                <span>{interfaceLabels.surah}</span>
                <input
                  aria-label={interfaceLabels.searchOrSelectSurah}
                  className="searchInput surahComboInput"
                  type="text"
                  placeholder={interfaceLabels.findSurah}
                  value={surahQuery}
                  onChange={(event) => setSurahQuery(event.target.value)}
                  role="combobox"
                  aria-autocomplete="list"
                  aria-controls="surah-picker-options"
                  aria-expanded="true"
                  autoComplete="off"
                  onKeyDown={(event) => {
                    if (event.key === "ArrowDown") {
                      event.preventDefault();
                      surahOptionsRef.current?.querySelector<HTMLButtonElement>("button")?.focus();
                    }
                  }}
                />
              </label>
              <div
                className="surahOptions"
                id="surah-picker-options"
                role="listbox"
                aria-label={interfaceLabels.surahs}
                ref={surahOptionsRef}
              >
                {surahMatches.map((surah) => {
                  const page = surahStartPages.get(surah.id) ?? 1;
                  const isCurrentSurah = surah.id === currentSurahId;
                  return (
                    <button
                      className={isCurrentSurah ? "surahOptionCurrent" : undefined}
                      data-surah-id={surah.id}
                      key={surah.id}
                      type="button"
                      role="option"
                      aria-selected={isCurrentSurah}
                      onClick={() => jumpToSurah(surah)}
                      onKeyDown={(event) => {
                        if (event.key !== "ArrowDown" && event.key !== "ArrowUp") {
                          return;
                        }
                        event.preventDefault();
                        const options = Array.from(
                          surahOptionsRef.current?.querySelectorAll<HTMLButtonElement>("button") ?? [],
                        );
                        const currentIndex = options.indexOf(event.currentTarget);
                        const nextIndex = event.key === "ArrowDown"
                          ? Math.min(options.length - 1, currentIndex + 1)
                          : Math.max(0, currentIndex - 1);
                        options[nextIndex]?.focus();
                      }}
                    >
                      <span className="surahOptionNumber">
                        {formatMetadataNumber(surah.id, settings.metadataLanguage)}
                      </span>
                      <span className="surahOptionNames">
                        <strong>
                          {settings.metadataLanguage === "arabic"
                            ? surah.nameArabic
                            : surah.nameSimple}
                        </strong>
                        {settings.metadataLanguage === "english" ? (
                          <small>{surah.nameArabic}</small>
                        ) : null}
                      </span>
                      <span className="surahOptionPage">
                        {metadataLabels.page}{" "}
                        {formatMetadataNumber(page, settings.metadataLanguage)}
                      </span>
                    </button>
                  );
                })}
                {surahMatches.length === 0 ? (
                  <div className="surahNoResults">{interfaceLabels.noMatchingSurah}</div>
                ) : null}
              </div>
            </form>
          </div>
          </div>

        </aside>
      ) : null}

      {ayahActions ? <AyahActions x={ayahActions.x} y={ayahActions.y} arabic={settings.metadataLanguage==='arabic'}
        title={(settings.metadataLanguage==='arabic' ? 'الآية ' : 'Ayah ') + (ayahActions.run.dataset.ayahKey ?? '')}
        onClose={()=>setAyahActions(null)} onStudy={()=>{openTafsir(ayahActions.run);setAyahActions(null);}}
        onListen={scope=>{
          const key=ayahActions.run.dataset.ayahKey ?? '';const [surah,ayah]=key.split(':').map(Number);
          tafsirContextPage.current={key,page:Number(ayahActions.run.closest<HTMLElement>('[data-page-number]')?.dataset.pageNumber)};
          startContextListening(scope,{surah,ayah});setAyahActions(null);
        }}/>:null}
      {panelSelection ? (
        <Suspense fallback={<aside className="tafsirPanel ayahStudyPanel" role="status" dir={settings.metadataLanguage === "arabic" ? "rtl" : "ltr"}>
          <header className="tafsirPanelHeader"><strong>{settings.metadataLanguage === "arabic" ? "جارٍ فتح التفسير…" : "Opening ayah study…"}</strong>
            <button type="button" aria-label={settings.metadataLanguage === "arabic" ? "إغلاق التفسير" : "Close ayah study"} onClick={() => setTafsirSelection(null)}>&#215;</button>
          </header>
        </aside>}>
        <TafsirPanel
          closing={!tafsirSelection}
          onListen={startContextListening}
          onExitFullscreen={isFullscreen ? toggleFullscreen : undefined}
          ayahText={ayahSearchIndex?.find(entry => entry.surah === panelSelection.surah && entry.ayah === panelSelection.ayah)?.text ?? ""}
          verseCount={surahsById.get(panelSelection.surah)?.versesCount ?? 1}
          onNavigate={ayah => setTafsirSelection({ ...panelSelection, ayah })}
          onSurahStep={step => {
            const surah = panelSelection.surah + step;
            const next = surahsById.get(surah);
            if (next) setTafsirSelection({ surah, ayah: step > 0 ? 1 : next.versesCount });
          }}
          selection={panelSelection}
          source={tafsirSource}
          textSize={settings.tafsirTextSize}
          onTextSizeChange={size => updateSettings({ tafsirTextSize: size })}
          onSourceChange={setTafsirSource}
          onClose={() => setTafsirSelection(null)}
          language={settings.metadataLanguage}
        />
        </Suspense>
      ) : null}

      {data ? <ReadingTools listeningRequest={listeningRequest} language={settings.metadataLanguage} surahs={data.surahs}
        onPlaybackDockChange={setPlaybackDockVisible}
        integrateTitleBar={isRustHost && !isFullscreen} controlsEnabled={settings.hasChosenLanguage}
        dismissRequest={readingToolsDismiss}
        onBeforeOpen={() => { setIsJumpOpen(false); setIsSettingsOpen(false); setTafsirSelection(null); hideControls(); }}
        currentSurah={currentSurahId ?? 1} pages={currentQpcPageNumbers}
        onOpenChange={setReadingToolsOpen}
        onSurahChange={id => { const selected = data.surahs.find(s => s.id === id); if (selected) jumpToSurah(selected); }}
        onAyahChange={followRecitationAyah}
        enabled={settings.hasChosenLanguage && !isJumpOpen && !isSettingsOpen && !tafsirSelection} /> : null}

      <div className="pageTurnPreviews" ref={pageTurnPreviewsRef} inert aria-hidden="true">
        {areCurrentQpcPagesLoaded && !panelSelection ? (
          <PageTurnPreviews pageNumbers={preloadQpcPageNumbers} cache={pageCache} renderPage={renderReaderPage} />
        ) : null}
      </div>

      <section
        className="readerViewport"
        aria-label={interfaceLabels.quranPages}
        ref={readerViewportRef}
        onPointerOver={(event) => {
          const target = event.target instanceof HTMLElement
            ? event.target.closest<HTMLElement>(".ayahRun")
            : null;
          if (target?.dataset.ayahKey) setHoveredAyah(target.dataset.ayahKey);
        }}
        onPointerOut={(event) => {
          const related = event.relatedTarget instanceof HTMLElement
            ? event.relatedTarget.closest<HTMLElement>(".ayahRun")
            : null;
          if (related?.dataset.ayahKey !== hoveredAyahRef.current) setHoveredAyah(null);
        }}
        onPointerLeave={() => setHoveredAyah(null)}
        onContextMenu={(event) => {
          const run = event.target instanceof HTMLElement
            ? event.target.closest<HTMLElement>(".ayahRun")
            : null;
          if (!run) return;
          event.preventDefault();
          if (run.dataset.ayahKey === tafsirSelection?.surah + ":" + tafsirSelection?.ayah) {
            setTafsirSelection(null); setAyahActions(null);
          } else {
            setAyahActions({ run, x: event.clientX, y: event.clientY });
          }
        }}
      >
        {loadError ? <div className="loadState">{loadError}</div> : null}
        {(!data || !qpcData) && !loadError ? (
          <div className="loadState" role="status"><strong>Quran Mushaf Reader</strong><p dir={settings.metadataLanguage === "arabic" ? "rtl" : "ltr"}>{interfaceLabels.loadingMushaf}</p></div>
        ) : null}

        {data && qpcData && !areCurrentQpcPagesLoaded && !loadError ? (
          <div className="loadState" role={pageLoadError ? "alert" : "status"}>
            {pageLoadError ? (
              <>
                <p>{interfaceLabels.pageLoadFailed}</p>
                <button type="button" onClick={() => setPageLoadAttempt((attempt) => attempt + 1)}>
                  {interfaceLabels.retry}
                </button>
              </>
            ) : interfaceLabels.loadingPage}
          </div>
        ) : null}

        {data && qpcData && areCurrentQpcPagesLoaded && isSpreadMode ? (
          <div
            className="readerScaleSurface spreadSurface"
            style={
              {
                "--reader-natural-width": `${spreadNaturalWidth}px`,
                "--reader-natural-height": `${PAGE_ASPECT_HEIGHT}px`,
                "--reader-scaled-width": `${spreadNaturalWidth * readerScale}px`,
                "--reader-scaled-height": `${PAGE_ASPECT_HEIGHT * readerScale}px`,
              } as CSSProperties
            }
          >
            <div className="spreadView">
              <div className="pageSlot leftSlot">
                {spread.leftPage ? renderReaderPage(spread.leftPage) : null}
              </div>
              <div className="pageSlot rightSlot">
                {renderReaderPage(spread.rightPage)}
              </div>
            </div>
          </div>
        ) : null}

        {data && qpcData && areCurrentQpcPagesLoaded && !isSpreadMode ? (
          <div
            className="readerScaleSurface singleSurface"
            style={
              {
                "--reader-natural-width": `${singleNaturalWidth}px`,
                "--reader-natural-height": `${PAGE_ASPECT_HEIGHT}px`,
                "--reader-scaled-width": `${singleNaturalWidth * readerScale}px`,
                "--reader-scaled-height": `${PAGE_ASPECT_HEIGHT * readerScale}px`,
              } as CSSProperties
            }
          >
            <div className="singleView">
              <div className="pageSlot">
                {renderReaderPage(settings.page)}
              </div>
            </div>
          </div>
        ) : null}
      </section>
    </main>
  );
}
