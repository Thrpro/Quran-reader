import { validAudioResponse } from "./audioFormat";
import { TimedRecitation } from "./timedRecitation";
import { TIMED_RECORDINGS, timedAudioUrl } from "./timedAudio";
import { ReciterPicker } from "./ReciterPicker";
import { RecitationStream, type StreamStatus } from "./recitationStream";
import { recitationPlan } from "./recitationPlan";
import { AudioDownloadError, downloadAudio, type DownloadFailure, type DownloadProgress } from "./audioDownloads";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Download, Headphones, Pause, Play, Target, Trash2, X, Square, Volume2, RotateCcw, RotateCw, SlidersHorizontal } from "lucide-react";
import { AYAH_RECITERS, audioUrl, ayahAudioUrl, readReciterCatalog, reciterDisplayName, reciterNameMatches, type Reciter } from "./audioCatalog";

type Surah = { id: number; nameSimple: string; nameArabic: string; versesCount: number };
type Goal = { day: string; mode: "pages" | "minutes"; target: number; pages: number[]; seconds: number };
type ReadingPreferences = { showTimer: boolean; reminderEnabled: boolean; reminderTime: string };
const GOAL_KEY = "quran-reading-goal-v1";
const READING_PREFERENCES_KEY = "quran-reading-preferences-v1";
const REMINDER_DAY_KEY = "quran-reading-reminder-day-v1";
const AUDIO_CACHE = "quran-audio-v1";
const MAX_AUDIO_BYTES = 200 * 1024 * 1024;
const dayKey = () => { const now = new Date(); return `${now.getFullYear()}-${now.getMonth() + 1}-${now.getDate()}`; };
function today(goal: Goal): Goal { return goal.day === dayKey() ? goal : { ...goal, day: dayKey(), pages: [], seconds: 0 }; }
function readGoal(): Goal {
  const fallback: Goal = { day: dayKey(), mode: "pages", target: 5, pages: [], seconds: 0 };
  try {
    const saved = JSON.parse(localStorage.getItem(GOAL_KEY) ?? "null");
    if (!saved || !["pages", "minutes"].includes(saved.mode) || !Number.isFinite(saved.target)) return fallback;
    return today({ ...fallback, ...saved, target: Math.max(1, Math.min(saved.mode === "minutes" ? 1440 : 604, saved.target)),
      seconds: Number.isFinite(saved.seconds) ? Math.max(0, saved.seconds) : 0,
      pages: Array.isArray(saved.pages) ? [...new Set<number>(saved.pages.filter((p: number) => Number.isInteger(p) && p >= 1 && p <= 604))] : [] });
  } catch { return fallback; }
}
function readReadingPreferences(): ReadingPreferences {
  const fallback = { showTimer: false, reminderEnabled: false, reminderTime: "19:00" };
  try {
    const saved = JSON.parse(localStorage.getItem(READING_PREFERENCES_KEY) ?? "null");
    return {
      showTimer: typeof saved?.showTimer === "boolean" ? saved.showTimer : fallback.showTimer,
      reminderEnabled: typeof saved?.reminderEnabled === "boolean" ? saved.reminderEnabled : fallback.reminderEnabled,
      reminderTime: typeof saved?.reminderTime === "string" && /^([01]\d|2[0-3]):[0-5]\d$/.test(saved.reminderTime)
        ? saved.reminderTime : fallback.reminderTime,
    };
  } catch { return fallback; }
}
function formatDuration(totalSeconds: number) {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(remainder).padStart(2, "0")}`
    : `${minutes}:${String(remainder).padStart(2, "0")}`;
}
export type ListeningRequest = { id: number; surah: number; ayah: number; end: { surah: number; ayah: number }; label: string };
export function ReadingTools({ language, surahs, currentSurah, pages, enabled, onOpenChange, onSurahChange, onAyahChange, onPlaybackDockChange, listeningRequest, integrateTitleBar = false, controlsEnabled = enabled, onBeforeOpen, dismissRequest = 0 }: {
  onPlaybackDockChange: (height: number) => void;
  integrateTitleBar?: boolean;
  controlsEnabled?: boolean;
  onBeforeOpen?: () => void;
  dismissRequest?: number;
  listeningRequest?: ListeningRequest | null;
  language: "arabic" | "english"; surahs: Surah[]; currentSurah: number; pages: number[]; enabled: boolean; onOpenChange: (open: boolean) => void; onSurahChange: (id: number) => void; onAyahChange: (key: string | null) => void;
}) {
  const ar = language === "arabic";
  const t = (en: string, arabic: string) => ar ? arabic : en;
  const [tab, setTab] = useState<"audio" | "goal" | null>(null);
  const returnFocus = useRef<HTMLElement | null>(null);
  const [goal, setGoal] = useState(readGoal);
  const goalRef = useRef(goal); goalRef.current = goal;
  const [readingPreferences, setReadingPreferences] = useState(readReadingPreferences);
  const [running, setRunning] = useState(false);
  const [reciters, setReciters] = useState<Reciter[]>([]);
  const [reciterQuery, setReciterQuery] = useState("");
  const [ayahReciterQuery, setAyahReciterQuery] = useState(() => reciterDisplayName(AYAH_RECITERS.find(reciter => reciter.id === 7) ?? AYAH_RECITERS[0], language));
  const [celebration, setCelebration] = useState(false);
  const [reminderVisible, setReminderVisible] = useState(false);
  const visitedPages = useRef(pages);
  const completed = useRef((goal.mode === "pages" ? goal.pages.length : goal.seconds / 60) >= goal.target);
  const [playbackMode, setPlaybackMode] = useState<"surah" | "ayah">("ayah");
  const [ayahReciterId, setAyahReciterId] = useState(7);
  const [useContinuousRecording, setUseContinuousRecording] = useState(() => {
    try { return localStorage.getItem("quran-single-recording-v1") !== "false"; } catch { return true; }
  });
  useEffect(() => {
    try { localStorage.setItem("quran-single-recording-v1", String(useContinuousRecording)); } catch { /* Optional preference. */ }
  }, [useContinuousRecording]);
  const [ayah, setAyah] = useState(1);
  const [reciterId, setReciterId] = useState(0);
  const [recordingId, setRecordingId] = useState(0);
  const [surah, setSurah] = useState(currentSurah);
  const [catalogState, setCatalogState] = useState("idle");
  const [retry, setRetry] = useState(0);
  const [audioEnabled, setAudioEnabled] = useState(false);
  const [audioError, setAudioError] = useState(false);
  const [audioSource, setAudioSource] = useState("");
  const [playing, setPlaying] = useState(false);
  const [downloaded, setDownloaded] = useState<string[]>([]);
  const [downloadState, setDownloadState] = useState<"idle" | "saving" | "error">("idle");
  const [downloadProgress, setDownloadProgress] = useState<DownloadProgress | null>(null);
  const [downloadFailure, setDownloadFailure] = useState<DownloadFailure>("network");
  const [storageBytes, setStorageBytes] = useState(0);
  const [continuous, setContinuous] = useState(false);
  const [listeningRange, setListeningRange] = useState<ListeningRequest | null>(null);
  const [rangeStart, setRangeStart] = useState(1);
  const [rangeEnd, setRangeEnd] = useState(1);
  const [repeatRange, setRepeatRange] = useState(false);
  // Completion callbacks are installed with the timeline, while this preference
  // can change without rebuilding (or cutting off) the current recording.
  const repeatRangeRef = useRef(repeatRange); repeatRangeRef.current = repeatRange;
  const replayStart = useRef({ surah, ayah });
  const continuePlayback = useRef(false);
  const audio = useRef<HTMLAudioElement | null>(null);
  const stream = useRef<RecitationStream | TimedRecitation | null>(null);
  const [playbackRevision, setPlaybackRevision] = useState(0);
  const [streamStatus, setStreamStatus] = useState<StreamStatus>('ready');
  const [basmala, setBasmala] = useState(false);
  const [hasStarted, setHasStarted] = useState(false);
  const dockVisible = hasStarted && tab !== "audio";
  const miniPlayer = useRef<HTMLDivElement | null>(null);
  useLayoutEffect(() => {
    const dock = miniPlayer.current;
    if (!dockVisible || !dock) { onPlaybackDockChange(0); return; }
    const measure = () => onPlaybackDockChange(Math.ceil(dock.getBoundingClientRect().height) + 24);
    measure();
    const observer = new ResizeObserver(measure); observer.observe(dock);
    return () => observer.disconnect();
  }, [dockVisible, onPlaybackDockChange]);
  useEffect(() => () => onPlaybackDockChange(0), [onPlaybackDockChange]);
  const [speed, setSpeed] = useState(1);
  const [volume, setVolume] = useState(1);
  const speedRef = useRef(speed); speedRef.current = speed;
  const [clock, setClock] = useState({elapsed:0,duration:0,buffered:0,min:0,max:0});
  const [downloadBatch, setDownloadBatch] = useState<{ done: number; total: number; name: string } | null>(null);
  const abortDownload = useRef<AbortController | null>(null);
  const recording = reciters.find(r => r.id === reciterId)?.moshaf.find(m => m.id === recordingId);
  const ayahReciter = AYAH_RECITERS.find(reciter => reciter.id === ayahReciterId) ?? AYAH_RECITERS[0];
  const timedRecording = playbackMode === "ayah" && useContinuousRecording && !!TIMED_RECORDINGS[ayahReciterId];
  const url = timedRecording ? timedAudioUrl(ayahReciterId, surah) : playbackMode === "ayah" ? ayahAudioUrl(ayahReciter, surah, ayah) : audioUrl(recording, surah);
  const title = surahs.find(s => s.id === surah);
  const surahUrls = useMemo(() => timedRecording ? [url] : playbackMode === "ayah"
    ? [...new Set(recitationPlan(ayahReciter, {surah,ayah:1}, {surah,ayah:title?.versesCount ?? 1}, surahs).map(item => item.url))]
    : url ? [url] : [], [timedRecording, playbackMode, title?.versesCount, ayahReciter, surah, url]);
  const surahDownloaded = surahUrls.length > 0 && surahUrls.every(item => downloaded.includes(item));
  const selectedReciterName = reciters.find(r => r.id === reciterId)?.name ?? "";
  const matchingReciters = !reciterQuery.trim() || reciterQuery === selectedReciterName
    ? reciters
    : reciters.filter(r => reciterNameMatches(r.name, reciterQuery));
  const selectedAyahReciterName = reciterDisplayName(ayahReciter, language);
  const matchingAyahReciters = !ayahReciterQuery.trim() || ayahReciterQuery === selectedAyahReciterName
    ? AYAH_RECITERS
    : AYAH_RECITERS.filter(reciter =>
        reciterNameMatches(reciterDisplayName(reciter, "english"), ayahReciterQuery) ||
        reciterNameMatches(reciterDisplayName(reciter, "arabic"), ayahReciterQuery));

  useEffect(() => {
    const previous = visitedPages.current;
    visitedPages.current = pages;
    const departed = previous.filter(page => !pages.includes(page));
    if (!running || !enabled || !departed.length) return;
    setGoal(g => { const fresh = today(g); return { ...fresh, pages: [...new Set([...fresh.pages, ...departed])] }; });
  }, [pages, running, enabled]);
  useEffect(() => {
    const reached = (goal.mode === "pages" ? goal.pages.length : goal.seconds / 60) >= goal.target;
    if (reached && !completed.current) setCelebration(true);
    completed.current = reached;
  }, [goal]);
  useEffect(() => {
    if (!celebration) return;
    const timer = window.setTimeout(() => setCelebration(false), 4500);
    return () => clearTimeout(timer);
  }, [celebration]);
  useEffect(() => {
    setAyahReciterQuery(reciterDisplayName(ayahReciter, language));
  }, [language]);
  useEffect(() => {
    try { localStorage.setItem(READING_PREFERENCES_KEY, JSON.stringify(readingPreferences)); } catch { /* Optional preference. */ }
  }, [readingPreferences]);
  useEffect(() => {
    if (!readingPreferences.reminderEnabled) return;
    const checkReminder = () => {
      const fresh = today(goalRef.current);
      const reached = (fresh.mode === "pages" ? fresh.pages.length : fresh.seconds / 60) >= fresh.target;
      const now = new Date();
      const time = `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
      let lastDay = "";
      try { lastDay = localStorage.getItem(REMINDER_DAY_KEY) ?? ""; } catch { /* Storage is optional. */ }
      if (reached || time < readingPreferences.reminderTime || lastDay === dayKey()) return;
      try { localStorage.setItem(REMINDER_DAY_KEY, dayKey()); } catch { /* Storage is optional. */ }
      setReminderVisible(true);
      if ("Notification" in window && Notification.permission === "granted") {
        new Notification(t("Time for your Quran reading", "حان وقت وردك القرآني"), {
          body: t("Open Daily reading when you are ready to begin.", "افتح الورد اليومي عندما تكون مستعدًا للبدء."),
        });
      }
    };
    checkReminder();
    const timer = window.setInterval(checkReminder, 30000);
    return () => window.clearInterval(timer);
  }, [readingPreferences.reminderEnabled, readingPreferences.reminderTime, language]);
  useEffect(() => {
    if (!reminderVisible) return;
    const timer = window.setTimeout(() => setReminderVisible(false), 7000);
    return () => window.clearTimeout(timer);
  }, [reminderVisible]);

  useEffect(() => { onOpenChange(tab !== null); }, [tab, onOpenChange]);
  useEffect(() => { if (tab === "audio" || listeningRequest) setAudioEnabled(true); }, [tab, listeningRequest]);
  useEffect(() => {
    const webview = (window as Window & { chrome?: { webview?: { postMessage: (message: unknown) => void } } }).chrome?.webview;
    webview?.postMessage({ type: "reading-session-state", active: running, language });
  }, [language, running]);
  const onAyahChangeRef = useRef(onAyahChange); onAyahChangeRef.current = onAyahChange;
  useEffect(() => {
    onAyahChangeRef.current(playbackMode === "ayah" && playing && !basmala ? `${surah}:${ayah}` : null);
    return () => onAyahChangeRef.current(null);
  }, [ayah, playbackMode, playing, surah, basmala]);
  useEffect(() => { if (!enabled) { setTab(null); setRunning(false); } }, [enabled]);
  useEffect(() => { setTab(null); }, [dismissRequest]);
  useEffect(() => {
    setRangeStart(listeningRange?.surah === surah ? listeningRange.ayah : 1);
    setRangeEnd(listeningRange?.end.surah === surah ? listeningRange.end.ayah : title?.versesCount ?? 1);
  }, [surah, title?.versesCount, listeningRange?.id]);
  useEffect(() => {
    if (!listeningRequest) return;
    setListeningRange(listeningRequest);
    setPlaybackMode("ayah");
    setSurah(listeningRequest.surah);
    setAyah(listeningRequest.ayah);
    setContinuous(listeningRequest.end.surah > listeningRequest.surah);
    continuePlayback.current = true;
    setTab("audio");
  }, [listeningRequest]);
  useEffect(() => {
    const save = () => { try { localStorage.setItem(GOAL_KEY, JSON.stringify(goalRef.current)); } catch { /* Storage may be full. */ } };
    save();
    const id = running ? window.setInterval(save, 15000) : 0;
    window.addEventListener("pagehide", save);
    return () => { clearInterval(id); save(); window.removeEventListener("pagehide", save); };
  }, [running, goal.mode, goal.target, goal.pages]);
  useEffect(() => {
    if (!running) return;
    let last = performance.now();
    let awayTimer = 0;
    const tick = () => {
      const now = performance.now();
      if (document.hidden || !document.hasFocus()) { last = now; setRunning(false); return; }
      const seconds = Math.min(2, (now - last) / 1000); last = now;
      setGoal(g => ({ ...today(g), seconds: today(g).seconds + seconds }));
    };
    const id = window.setInterval(tick, 1000);
    const stop = () => {
      window.clearTimeout(awayTimer);
      awayTimer = window.setTimeout(() => {
        if (!document.hidden && document.hasFocus()) return;
        tick();
        setRunning(false);
      }, 180);
    };
    window.addEventListener("blur", stop);
    document.addEventListener("visibilitychange", stop);
    return () => { clearInterval(id); clearTimeout(awayTimer); window.removeEventListener("blur", stop); document.removeEventListener("visibilitychange", stop); };
  }, [running]);
  useEffect(() => {
    if (!tab) return;
    setGoal(g => today(g));
    const close = (event: KeyboardEvent) => { if (event.key === "Escape") { setTab(null); returnFocus.current?.focus({ preventScroll: true }); } };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [tab]);
  useEffect(() => {
    if (tab !== "audio" || playbackMode !== "surah") return;
    const controller = new AbortController();
    const cacheKey = `quran-reciter-catalog-${language}`;
    setCatalogState("loading");
    fetch(`https://www.mp3quran.net/api/v3/reciters?language=${ar ? "ar" : "eng"}&rewaya=1`, { signal: controller.signal })
      .then(r => { if (!r.ok) throw Error(); return r.json(); })
      .then(data => {
        const list = readReciterCatalog(data);
        if (!list.length) throw Error();
        try { localStorage.setItem(cacheKey, JSON.stringify(list)); } catch { /* Catalog caching is optional. */ }
        return list;
      }).catch(error => {
        if (controller.signal.aborted) throw error;
        try {
          const saved = readReciterCatalog(JSON.parse(localStorage.getItem(cacheKey) ?? "null"));
          if (!saved.length) throw error;
          return saved;
        } catch {
          throw error;
        }
      }).then(list => {
        if (controller.signal.aborted) return;
        const selectedReciter = list.find(r => r.id === reciterId) ?? list[0];
        const selectedRecording = selectedReciter.moshaf.find(m => m.id === recordingId) ?? selectedReciter.moshaf[0];
        setReciters(list); setCatalogState("ready");
        setReciterId(selectedReciter.id);
        setRecordingId(selectedRecording.id);
        setReciterQuery(selectedReciter.name);
      }).catch(() => { if (!controller.signal.aborted) setCatalogState("error"); });
    return () => controller.abort();
  }, [tab === "audio", language, retry, playbackMode]);
  async function cachedAudioSize(cache: Cache, request: Request) {
    const value = Number((await cache.match(request))?.headers.get("content-length") ?? 0);
    return Number.isSafeInteger(value) && value >= 0 ? value : 0;
  }
  async function refreshDownloads() {
    const cache = await caches.open(AUDIO_CACHE);
    const keys = await cache.keys();
    const sizes = await Promise.all(keys.map(key => cachedAudioSize(cache, key)));
    setDownloaded(keys.map(key => key.url)); setStorageBytes(sizes.reduce((a, b) => a + b, 0));
  }
  useEffect(() => { if (tab === "audio") void refreshDownloads().catch(() => {}); }, [tab]);
  useEffect(() => () => abortDownload.current?.abort(), []);
  useEffect(() => {
    const element = audio.current; if (!element || !audioEnabled) return;
    let disposed = false, objectUrl = '';
    const invalidateFailedCache = () => { if (objectUrl) void caches.open(AUDIO_CACHE).then(cache => cache.delete(url)).catch(() => {}); };

    const autoplay = continuePlayback.current; continuePlayback.current = false;
    replayStart.current = { surah, ayah };
    element.pause(); setPlaying(false); setAudioError(false); setStreamStatus('ready'); setBasmala(false);
    setClock({elapsed:0,duration:0,buffered:0,min:0,max:0}); setHasStarted(autoplay); setAudioSource(url);
    element.defaultPlaybackRate = speed; element.playbackRate = speed; element.volume = volume;
    if (!url) return;
    if (timedRecording) {
      const end = listeningRange?.end ?? (continuous ? {surah:114,ayah:6} : {surah,ayah:title?.versesCount ?? ayah});
      const player = new TimedRecitation(element, ayahReciterId, {surah,ayah}, end, surahs, () => speedRef.current,
        item => { if (!disposed) { setSurah(item.surah); setAyah(item.ayah); setBasmala(Boolean(item.basmala)); } },
        status => { if (!disposed) { setStreamStatus(status); if (status === 'error') setAudioError(true);
          if (status === 'finished') finishRange(); } });
      stream.current = player; if (autoplay) player.play();
      return () => { disposed = true; player.dispose(); stream.current = null; };
    }
    if (playbackMode === 'ayah' && RecitationStream.supported()) {
      const end = listeningRange?.end ?? (continuous ? {surah:114,ayah:6} : {surah,ayah:title?.versesCount ?? ayah});
      const plan = recitationPlan(ayahReciter, {surah,ayah}, end, surahs);
      const player = new RecitationStream(element, plan, item => {
        if (disposed) return;
        setSurah(item.surah); setAyah(item.ayah); setBasmala(Boolean(item.basmala));
      }, status => { if (!disposed) { setStreamStatus(status); if (status === 'error') setAudioError(true); } });
      stream.current = player;
      if (autoplay) player.play();
      return () => { disposed = true; player.dispose(); stream.current = null; };
    }
    if (playbackMode === 'ayah') { setAudioError(true); setStreamStatus('error'); return; }
    element.addEventListener('error', invalidateFailedCache);
    // Whole-surah recordings already contain their own opening; do not prepend it.
    void (async () => {
      let response: Response | undefined;
      try { response = await (await caches.open(AUDIO_CACHE)).match(url); } catch { /* Streaming also works when cache storage is unavailable. */ }
      if (response && !await validAudioResponse(response.clone())) {
        await (await caches.open(AUDIO_CACHE)).delete(url); response = undefined;
      }
      const source = response ? URL.createObjectURL(await response.blob()) : url;
      if (response) objectUrl = source;
      if (disposed) { if (objectUrl) URL.revokeObjectURL(objectUrl); return; }
      element.src = source; element.defaultPlaybackRate = speedRef.current; element.playbackRate = speedRef.current;
      if (autoplay) void element.play().catch(() => { if (!disposed) setAudioError(true); });
    })().catch(() => { if (!disposed) setAudioError(true); });
    return () => { disposed = true; element.removeEventListener("error", invalidateFailedCache); element.pause(); element.removeAttribute('src'); element.load(); if (objectUrl) URL.revokeObjectURL(objectUrl); };
    // The displayed ayah advances within the stream. Only explicit selection or
    // playback configuration changes rebuild its timeline.
  }, [audioEnabled, playbackRevision, playbackMode, ayahReciterId, reciterId, recordingId, listeningRange?.id, continuous, useContinuousRecording]);
  function updateClock() {
    const element = audio.current; if (!element) return;
    const duration = stream.current ? stream.current.duration : Number.isFinite(element.duration) ? element.duration : 0;
    const next = {elapsed:Math.floor(element.currentTime * 4) / 4,duration,buffered:Math.floor(stream.current?.bufferedSeconds ?? 0),
      min:stream.current instanceof TimedRecitation ? stream.current.seekStart : element.buffered.length ? element.buffered.start(0) : 0,
      max:stream.current instanceof TimedRecitation ? stream.current.seekEnd : duration || (element.buffered.length ? element.buffered.end(element.buffered.length-1) : 0)};
    setClock(previous => Object.keys(next).every(key => previous[key as keyof typeof next] === next[key as keyof typeof next]) ? previous : next);
  }
  function togglePlayback() {
    const element = audio.current; if (!element) return;
    if (!element.paused) { stream.current ? stream.current.pause() : element.pause(); return; }
    if (element.ended || streamStatus === 'finished') { continuePlayback.current = true; if (listeningRange) { setSurah(listeningRange.surah); setAyah(listeningRange.ayah); } else setAyah(1); setPlaybackRevision(n=>n+1); return; }
    setHasStarted(true);
    if (stream.current) stream.current.play(); else void element.play().catch(() => setAudioError(true));
  }
  function stopPlayback() { stream.current?.pause(); audio.current?.pause(); if (listeningRange) { setSurah(listeningRange.surah); setAyah(listeningRange.ayah); } else setAyah(1); setHasStarted(false); continuePlayback.current = false; setPlaybackRevision(n=>n+1); }
  function seek(seconds: number) {
    const element = audio.current; if (!element || !clock.max) return;
    element.currentTime = Math.max(clock.min, Math.min(clock.max - .05, seconds)); updateClock();
  }
  function changeSpeed(value: number) { speedRef.current = value; setSpeed(value); if (audio.current) { audio.current.defaultPlaybackRate = value; audio.current.playbackRate = value; } }
  function finishRange() {
    if (repeatRangeRef.current) {
      continuePlayback.current = true;
      setSurah(replayStart.current.surah); setAyah(replayStart.current.ayah);
      setPlaybackRevision(n => n + 1);
    } else { setPlaying(false); setHasStarted(false); }
  }
  function applyRange() {
    if (!title || rangeStart < 1 || rangeEnd < rangeStart || rangeEnd > title.versesCount) return;
    continuePlayback.current = playing;
    setAyah(rangeStart);
    setListeningRange({ id: Date.now(), surah, ayah: rangeStart, end: { surah, ayah: rangeEnd }, label: t('Selected ayah range', 'نطاق الآيات المختار') });
  }
  function finishRecording() {
    if (stream.current instanceof TimedRecitation) return;
    if (repeatRangeRef.current) { finishRange(); return; }
    setPlaying(false); setHasStarted(false);
    if (playbackMode === 'surah' && continuous && surah < 114 && audioUrl(recording, surah + 1)) {
      continuePlayback.current = true; setAyah(1); setSurah(surah + 1); onSurahChange(surah + 1); setPlaybackRevision(n=>n+1);
    }
  }
  async function saveAudio() {
    if (!url || downloadState === "saving") return;
    const controller = new AbortController(); abortDownload.current = controller; setDownloadState("saving");
    try {
      setDownloadProgress(null);
      const cache = await caches.open(AUDIO_CACHE);
      const keys = await cache.keys();
      let used = 0;
      for (const key of keys) used += await cachedAudioSize(cache, key);
      const targets = [...surahUrls];
      const name = (ar ? title?.nameArabic : title?.nameSimple) ?? String(surah);
      let completed = 0, bytes = 0;
      setDownloadBatch({ done: 0, total: targets.length, name });
      for (const target of targets) {
        if (controller.signal.aborted) throw new DOMException("Cancelled", "AbortError");
        const cached = await cache.match(target);
        if (!cached) {
          let fileBytes = 0;
          await downloadAudio(target, cache, {
            usedBytes: used, maxBytes: MAX_AUDIO_BYTES, signal: controller.signal,
            onProgress: progress => { fileBytes = progress.received; setDownloadProgress({ received: bytes + progress.received, total: 0,
              percent: Math.floor((completed + progress.percent / 100) / targets.length * 100) }); },
          });
          used += fileBytes; bytes += fileBytes;
        }
        completed++;
        setDownloadBatch({ done: completed, total: targets.length, name });
        setDownloadProgress({ received: bytes, total: bytes, percent: Math.floor(completed / targets.length * 100) });
      }
      await refreshDownloads();
      setDownloadState("idle");
    } catch (error) {
      setDownloadFailure(error instanceof AudioDownloadError ? error.reason
        : error instanceof DOMException && error.name === "QuotaExceededError" ? "storage" : "network");
      setDownloadState(controller.signal.aborted ? "idle" : "error");
    }
    finally { abortDownload.current = null; await refreshDownloads().catch(() => {}); }
  }
  const progress = goal.mode === "pages" ? goal.pages.length : Math.floor(goal.seconds / 60);
  const sessionTime = goal.mode === "minutes"
    ? `${formatDuration(goal.seconds)} / ${formatDuration(goal.target * 60)}`
    : formatDuration(goal.seconds);
  const controlsHost = document.getElementById("reading-controls-host");
  const titleHost = integrateTitleBar ? document.getElementById("title-listen-host") : null;
  function openTab(next: "audio" | "goal") { returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; onBeforeOpen?.(); setTab(value => value === next ? null : next); }
  const listenButton = <button title={t("Listen", "استماع")} aria-label={t("Listen", "استماع")} aria-expanded={tab === "audio"} onClick={() => { if (!audioSource) setSurah(currentSurah); openTab("audio"); }}><Headphones size={17} aria-hidden="true" /><span>{t("Listen", "استماع")}</span></button>;
  const dock = controlsEnabled ? <div className="readingToolsDock" dir={ar ? "rtl" : "ltr"}>
    <button className={`sessionButton ${running ? "sessionActive" : ""}`} aria-expanded={tab === "goal"}
      aria-label={t("Daily reading", "الورد اليومي")}
      title={t("Daily goal and reading session", "الورد اليومي وجلسة القراءة")}
      onClick={() => openTab("goal")}><Target size={16} />
      <span>{t("Daily reading", "الورد اليومي")}</span>
      {running ? <span className="sessionIndicator">{t("Active", "جارٍ")}</span> : null}</button>
    {!titleHost ? listenButton : null}
  </div> : null;
  return <>
    {dock ? createPortal(dock, controlsHost ?? document.body) : null}
    {titleHost && controlsEnabled ? createPortal(listenButton, titleHost) : null}
    <audio ref={audio} data-active="true" preload="auto" className="hiddenAudio" controlsList="nodownload"
      onPlay={() => { setPlaying(true); setHasStarted(true); }} onPause={() => setPlaying(false)}
      onTimeUpdate={updateClock} onProgress={updateClock} onDurationChange={updateClock}
      onRateChange={() => { if (audio.current && audio.current.playbackRate !== speedRef.current) audio.current.playbackRate = speedRef.current; }}
      onError={() => setAudioError(true)} onEnded={finishRecording} />
    {dockVisible ? <div ref={miniPlayer} className="recitationMini" dir={ar?'rtl':'ltr'} role="region" aria-label={t('Recitation controls','أدوات التلاوة')}>
      <div className="miniRecording"><strong>{ar?title?.nameArabic:title?.nameSimple}</strong><small>{playbackMode==='ayah' ? t('Ayah','الآية')+' '+ayah : t('Full recording','تسجيل السورة')}</small></div>
      <div className="miniTransport" dir="ltr">
        <button disabled={!clock.max} onClick={() => seek((audio.current?.currentTime ?? clock.elapsed)-10)} aria-label={t('Back 10 seconds','رجوع ١٠ ثوانٍ')}><RotateCcw size={18}/><span>10</span></button>
        <button className="primaryPlayback" onClick={togglePlayback} aria-label={playing?t('Pause','إيقاف مؤقت'):t('Resume','متابعة')}>{playing ? <Pause size={22}/> : <Play size={22}/>}</button>
        <button disabled={!clock.max} onClick={() => seek((audio.current?.currentTime ?? clock.elapsed)+10)} aria-label={t('Forward 10 seconds','تقديم ١٠ ثوانٍ')}><span>10</span><RotateCw size={18}/></button>
      </div>
      <div className="miniActions"><button onClick={() => openTab('audio')} aria-label={t('Listening settings','إعدادات الاستماع')} title={t('Listening settings','إعدادات الاستماع')}><SlidersHorizontal size={18}/></button>
        <button aria-label={t('Stop recitation','إنهاء التلاوة')} title={t('Stop recitation','إنهاء التلاوة')} onClick={stopPlayback}><Square size={17}/></button></div>
      <div className="miniTimeline" dir="ltr"><span>{formatDuration(clock.elapsed)}</span>
        <input aria-label={t('Playback position','موضع التشغيل')} type="range" min={clock.min} max={Math.max(clock.min+.1,clock.max)} step="0.1" value={Math.max(clock.min,Math.min(clock.max,clock.elapsed))} disabled={!clock.max} onChange={e=>seek(Number(e.target.value))}/>
        <span>{clock.duration ? formatDuration(clock.duration) : '—'}</span></div>
    </div> : null}
    {tab ? <aside className="tafsirPanel readingToolsPanel" dir={ar ? "rtl" : "ltr"} lang={ar ? "ar" : "en"} aria-label={t("Reading tools", "أدوات القراءة")}>
      <header className="tafsirPanelHeader"><h2>{tab === "audio" ? t("Listen", "استماع") : t("Daily reading", "الورد اليومي")}</h2>
        <button title={t("Close", "إغلاق")} aria-label={t("Close", "إغلاق")} onClick={() => { setTab(null); returnFocus.current?.focus({ preventScroll: true }); }}><X size={20} /></button></header>
      {tab === "goal" ? <section className="tafsirContentSection goalSection">
        <label>{t("Goal", "الهدف")}<select aria-label={t("Goal", "الهدف")} value={goal.mode} onChange={e => setGoal(g => ({ ...today(g), mode: e.target.value as Goal["mode"], target: e.target.value === "pages" ? 5 : 30 }))}>
          <option value="pages">{t("Pages", "صفحات")}</option><option value="minutes">{t("Minutes", "دقائق")}</option></select></label>
        <label>{t("Daily target", "الهدف اليومي")}<input type="number" min="1" max={goal.mode === "pages" ? 604 : 1440} value={goal.target} onChange={e => { const n = Number(e.target.value); if (Number.isInteger(n) && n > 0 && n <= (goal.mode === "pages" ? 604 : 1440)) setGoal(g => ({ ...today(g), target: n })); }} /></label>
        <p className="goalProgress">{progress} / {goal.target} {goal.mode === "pages" ? t("pages", "صفحات") : t("minutes", "دقائق")}</p>
        <progress max={goal.target} value={Math.min(progress, goal.target)} />
        {progress >= goal.target ? <p role="status">{t("Today's goal completed", "أتممت ورد اليوم")}</p> : null}
        <p className="sessionHint">{t("Start a session when you are ready to read. Browsing outside a session does not count. Opening search or study pauses your session.", "ابدأ جلسة عندما تستعد للقراءة. لا يُحتسب التصفح خارج الجلسة. تتوقف الجلسة مؤقتًا عند فتح البحث أو التفسير.")}</p>
        <label className="settingToggle"><span><strong>{t("Show daily reading time", "إظهار وقت قراءة اليوم")}</strong><small>{t("Shows your total reading time today while you read.", "يعرض مجموع وقت قراءتك اليوم أثناء القراءة.")}</small></span><input type="checkbox" checked={readingPreferences.showTimer} onChange={event => setReadingPreferences(value => ({ ...value, showTimer: event.target.checked }))} /></label>
        <label className="settingToggle"><span><strong>{t("Daily reminder", "تذكير يومي")}</strong><small>{t("Remind me at this time while the app is running.", "ذكّرني في هذا الوقت أثناء تشغيل التطبيق.")}</small></span><input type="checkbox" checked={readingPreferences.reminderEnabled} onChange={event => {
          const reminderEnabled = event.target.checked;
          setReadingPreferences(value => ({ ...value, reminderEnabled }));
          if (reminderEnabled && "Notification" in window && Notification.permission === "default") void Notification.requestPermission().catch(() => {});
        }} /></label>
        {readingPreferences.reminderEnabled ? <label>{t("Reminder time", "وقت التذكير")}<input type="time" value={readingPreferences.reminderTime} onChange={event => setReadingPreferences(value => ({ ...value, reminderTime: event.target.value }))} /></label> : null}
        <button className="sessionPrimaryAction" onClick={() => { setRunning(value => !value); setTab(null); }}>{running ? <Pause size={18} /> : <Play size={18} />}{running ? t("Pause reading", "إيقاف القراءة مؤقتًا") : t("Start reading", "ابدأ القراءة")}</button>
      </section> : <section className="tafsirContentSection audioSection">
        {listeningRange ? <p className="listeningRangeSummary" role="status">{listeningRange.label} · {listeningRange.surah}:{listeningRange.ayah} – {listeningRange.end.surah}:{listeningRange.end.ayah}
          <button onClick={() => { continuePlayback.current = playing; setListeningRange(null); }}>{t("Clear range", "إلغاء النطاق")}</button></p> : null}
        <div className="audioModeControl" role="group" aria-label={t("Playback mode", "نمط التشغيل")}>
          <button aria-pressed={playbackMode === "ayah"} onClick={() => { if (playbackMode !== "ayah") { continuePlayback.current = playing; setListeningRange(null); setPlaybackMode("ayah"); } }}>{t("Follow along", "تابع الآيات")}</button>
          <button aria-pressed={playbackMode === "surah"} onClick={() => { if (playbackMode !== "surah") { continuePlayback.current = playing; setListeningRange(null); setPlaybackMode("surah"); } }}>{t("Full recording", "تسجيل السورة")}</button>
        </div>
        <p className="listeningModeHint">{playbackMode === "ayah"
          ? t("Listen continuously from the selected ayah, with each ayah highlighted as it plays.", "استمع باستمرار من الآية المختارة، مع تمييز كل آية أثناء تلاوتها.")
          : t("Listen to one continuous surah recording. More reciters are available; individual ayahs are not highlighted.", "استمع إلى تسجيل متصل للسورة. يتوفر قرّاء أكثر، دون تمييز كل آية.")}</p>
        {catalogState === "loading" ? <p role="status">{t("Loading reciters...", "تحميل القراء...")}</p> : null}
        {catalogState === "error" ? <button onClick={() => setRetry(n => n + 1)}>{t("Connection failed. Retry", "تعذر الاتصال. إعادة المحاولة")}</button> : null}
        {playbackMode === "ayah" ? <ReciterPicker id="ayah-reciter" label={t("Reciter", "القارئ")}
          prompt={t("Search or choose a reciter", "ابحث عن قارئ أو اختره")} empty={t("No matching reciter", "لا يوجد قارئ مطابق")}
          query={ayahReciterQuery} onQuery={setAyahReciterQuery} selected={ayahReciterId}
          options={matchingAyahReciters.map(reciter => ({ id: reciter.id, name: reciterDisplayName(reciter, language) }))}
          onSelect={id => { setAyahReciterId(id); setAyahReciterQuery(reciterDisplayName(AYAH_RECITERS.find(r => r.id === id)!, language)); }}
        /> : <><ReciterPicker id="reciter" label={t("Reciter", "القارئ")}
          prompt={t("Search or choose a reciter", "ابحث عن قارئ أو اختره")} empty={t("No matching reciter", "لا يوجد قارئ مطابق")}
          query={reciterQuery} onQuery={setReciterQuery} selected={reciterId}
          options={matchingReciters.map(reciter => ({ id: reciter.id, name: reciter.name }))}
          onSelect={id => { const reciter = reciters.find(r => r.id === id)!; setReciterId(id); setRecordingId(reciter.moshaf[0].id); setReciterQuery(reciter.name); }}
        />
        {(reciters.find(r => r.id === reciterId)?.moshaf.length ?? 0) > 1 ? <label>{t("Recording", "التلاوة")}<select aria-label={t("Recording", "التلاوة")} value={recordingId} onChange={e => setRecordingId(Number(e.target.value))}>{reciters.find(r => r.id === reciterId)?.moshaf.map(m => <option key={m.id} value={m.id}>{m.name}</option>)}</select></label> : null}</>}
        {playbackMode === "ayah" && TIMED_RECORDINGS[ayahReciterId] ? <details className="audioRecordingOptions">
          <summary>{t("Recording options", "خيارات التسجيل")}<small>{useContinuousRecording ? t("Single surah recording", "تسجيل واحد للسورة") : t("Ayah recordings", "تسجيلات الآيات")}</small></summary>
          <p>{t("Both choices play consecutively and highlight each ayah. The single recording may be a different recitation by the same reciter. Downloads save the whole surah in either case.", "كلا الخيارين يشغّل الآيات بالتتابع مع تمييزها. قد يكون التسجيل الواحد تلاوة أخرى للقارئ نفسه. يُحفظ صوت السورة كاملة عند التنزيل في الحالتين.")}</p>
          <label>{t("Audio source", "مصدر الصوت")}<select value={useContinuousRecording ? "surah" : "ayah"} onChange={e => { continuePlayback.current = playing; setUseContinuousRecording(e.target.value === "surah"); }}>
            <option value="ayah">{t("Separate ayah recordings", "تسجيلات منفصلة للآيات")}</option>
            <option value="surah">{t("Single surah recording (recommended)", "تسجيل واحد للسورة (موصى به)")}</option>
          </select></label>
        </details> : null}
        <label>{t("Surah", "السورة")}<select aria-label={t("Surah", "السورة")} value={surah} onChange={e => { setListeningRange(null); const id = Number(e.target.value); setSurah(id); setAyah(1); setPlaybackRevision(n=>n+1); onSurahChange(id); }}>{surahs.map(s => <option key={s.id} value={s.id} disabled={playbackMode === "surah" && !audioUrl(recording, s.id)}>{ar ? s.nameArabic : s.nameSimple}</option>)}</select></label>
        {playbackMode === "ayah" ? <label>{t("Ayah", "الآية")}<select aria-label={t("Ayah", "الآية")} value={ayah} onChange={e => { setListeningRange(null); setAyah(Number(e.target.value)); setPlaybackRevision(n=>n+1); }}>{Array.from({ length: title?.versesCount ?? 1 }, (_, index) => <option value={index + 1} key={index + 1}>{index + 1}</option>)}</select></label> : null}
        {playbackMode === "ayah" ? <details className="listeningRangeEditor"><summary>{t("Choose an ayah range", "اختيار نطاق الآيات")}</summary>
          <div className="listeningRangeFields">
            <label>{t("From ayah", "من الآية")}<input aria-label={t("From ayah", "من الآية")} type="number" min="1" max={title?.versesCount ?? 1} value={rangeStart} onChange={e => setRangeStart(Number(e.target.value))} /></label>
            <label>{t("To ayah", "إلى الآية")}<input aria-label={t("To ayah", "إلى الآية")} type="number" min={rangeStart} max={title?.versesCount ?? 1} value={rangeEnd} onChange={e => setRangeEnd(Number(e.target.value))} /></label>
          </div>
          <button disabled={!Number.isInteger(rangeStart) || !Number.isInteger(rangeEnd) || rangeStart < 1 || rangeEnd < rangeStart || rangeEnd > (title?.versesCount ?? 1)} onClick={applyRange}>{t("Use this range", "استخدام هذا النطاق")}</button>
        </details> : null}
        <label className="settingToggle"><span><strong>{playbackMode === "ayah" ? t("Repeat the selected range", "تكرار النطاق المختار") : t("Repeat this surah", "تكرار هذه السورة")}</strong><small>{t("Starts again after the last ayah finishes. Turn off to play once.", "يبدأ مجددًا بعد انتهاء الآية الأخيرة. أوقفه للاستماع مرة واحدة.")}</small></span><input aria-label={t("Repeat playback", "تكرار التشغيل")} type="checkbox" checked={repeatRange} onChange={e => setRepeatRange(e.target.checked)} /></label>
        <div className="recitationPlayer">
          <h3>{ar ? title?.nameArabic : title?.nameSimple}</h3>
          <p className="recitationPosition">{basmala ? t('Bismillah', 'البسملة') : playbackMode === 'ayah' ? t('Ayah', 'الآية') + ' ' + ayah + ' / ' + (title?.versesCount ?? '') : t('Full recording','تسجيل السورة')}</p>
          <div className="recitationButtons">
            <button onClick={() => seek(clock.elapsed-10)} aria-label={t('Back 10 seconds','رجوع ١٠ ثوانٍ')}><RotateCcw size={18}/>10</button>
            <button className="primaryPlayback" onClick={togglePlayback}>{playing?<Pause size={22}/>:<Play size={22}/>} {playing?t('Pause','إيقاف مؤقت'):hasStarted?t('Resume','متابعة'):t('Play','تشغيل')}</button>
            <button onClick={() => seek(clock.elapsed+10)} aria-label={t('Forward 10 seconds','تقديم ١٠ ثوانٍ')}>10<RotateCw size={18}/></button>
            <button onClick={stopPlayback} title={t('Stop and reset','إنهاء وإعادة ضبط')} aria-label={t('Stop recitation','إنهاء التلاوة')}><Square size={18}/></button>
          </div>
          <input className="recitationSeek" aria-label={t('Playback position','موضع التشغيل')} type="range" min={clock.min} max={Math.max(clock.min+.1,clock.max)} step="0.1" value={Math.max(clock.min,Math.min(clock.max,clock.elapsed))} disabled={!clock.max} onChange={e=>seek(Number(e.target.value))}/>
          <div className="recitationTimes"><span dir="ltr">{formatDuration(clock.elapsed)} / {clock.duration ? formatDuration(clock.duration) : '—'}</span><span>{clock.max ? t('Remaining ','المتبقي ')+formatDuration((clock.max-clock.elapsed)/speed) : playbackMode==='ayah' ? t('Buffered ','جاهز للتشغيل ')+formatDuration(clock.buffered/speed) : '—'}</span></div>
          <div className="recitationPreferences"><label>{t('Speed','السرعة')}<select aria-label={t('Playback speed','سرعة التشغيل')} value={speed} onChange={e=>changeSpeed(Number(e.target.value))}>{[.5,.75,1,1.25,1.5,1.75,2].map(n=><option key={n} value={n}>{n}×</option>)}</select></label>
          <label><Volume2 size={17}/>{t('Volume','الصوت')}<input aria-label={t('Volume','الصوت')} type="range" min="0" max="1" step="0.05" value={volume} onChange={e=>{const value=Number(e.target.value);setVolume(value);if(audio.current)audio.current.volume=value;}}/></label></div>
          {streamStatus==='buffering'||streamStatus==='retrying'?<p role="status">{streamStatus==='retrying'?t('Connection interrupted. Retrying this ayah…','انقطع الاتصال. جارٍ إعادة تحميل الآية…'):t('Preparing audio…','جارٍ تجهيز التلاوة…')}</p>:null}
        </div>
        {audioError ? <div role="alert"><button onClick={()=>{continuePlayback.current=true;setPlaybackRevision(n=>n+1);}}>{t("Retry recitation", "إعادة تشغيل التلاوة")}</button><p>{t("Unable to play. Check your connection or choose another recording.", "تعذر التشغيل. تحقق من الاتصال أو اختر تلاوة أخرى.")}</p></div> : null}
        <label className="settingToggle"><span>{t("Continue to next surah", "متابعة السورة التالية")}</span><input type="checkbox" checked={continuous} onChange={e => { continuePlayback.current = playing; setListeningRange(null); setContinuous(e.target.checked); }} /></label>
        {downloadState === "saving" ? <div className="audioDownloadProgress">
          <progress max={100} value={downloadProgress?.percent} aria-label={t("Download progress", "تقدم التنزيل")} />
          <span>{downloadProgress ? `${downloadProgress.percent}% · ${downloadBatch?.name ?? ""} · ${downloadBatch?.done ?? 0}/${downloadBatch?.total ?? 1} · ${(downloadProgress.received / 1048576).toFixed(1)} MB` : t("Connecting…", "جارٍ الاتصال…")}</span>
        </div> : null}
        <details className="audioStorageInfo">
          <summary>{t("ⓘ Download storage", "ⓘ معلومات تخزين التنزيلات")}</summary>
          <p>{t("Recitations are saved on this device in the app’s offline storage, not your Downloads folder. Limit: 200 MB. Use the removal buttons below to free space.", "تُحفظ التلاوات على هذا الجهاز في مساحة التطبيق دون اتصال، وليس في مجلد التنزيلات. الحد: ٢٠٠ ميجابايت. استخدم أزرار الحذف أدناه لإخلاء المساحة.")}</p>
          <p>{t("On Windows, this storage is managed inside:", "في تطبيق ويندوز، تُدار هذه المساحة داخل:")} <code dir="ltr">%LOCALAPPDATA%\QuranMushafReader\WebView2</code></p>
        </details>
        <div className="goalPageActions"><button disabled={!surahUrls.length || surahDownloaded || downloadState === "saving"} onClick={() => void saveAudio()}><Download size={16} />{surahDownloaded ? t("Surah saved offline", "السورة محفوظة دون اتصال") : downloadState === "saving" ? t("Downloading...", "جارٍ التنزيل...") : t("Download surah", "تنزيل السورة")}</button>
          {downloadState === "saving" ? <button onClick={() => abortDownload.current?.abort()}>{t("Cancel", "إلغاء")}</button> : null}
          {surahUrls.some(item => downloaded.includes(item)) ? <button disabled={downloadState === "saving"} title={t("Remove download", "حذف التنزيل")} onClick={() => void caches.open(AUDIO_CACHE).then(cache => Promise.all(surahUrls.map(item => cache.delete(item)))).then(refreshDownloads).catch(() => setDownloadState("error"))}><Trash2 size={16} /></button> : null}</div>
        {downloadState === "error" ? <p role="alert">{downloadFailure === "storage"
          ? t("Not enough storage. Remove saved recitations and try again (200 MB limit).", "المساحة غير كافية. احذف بعض التلاوات المحفوظة ثم حاول مجددًا (الحد ٢٠٠ ميجابايت).")
          : downloadFailure === "format"
          ? t("The service returned invalid audio. Please retry later.", "أعاد المصدر ملفًا صوتيًا غير صالح. حاول لاحقًا.")
          : downloadFailure === "size"
          ? t("The server did not provide the file size, so this recording cannot be downloaded safely. Streaming is still available.", "لم يحدد الخادم حجم الملف، لذا تعذر تنزيل هذا التسجيل بأمان. لا يزال الاستماع المباشر متاحًا.")
          : downloadFailure === "incomplete"
          ? t("The download was incomplete. Please try again.", "لم يكتمل التنزيل. يرجى المحاولة مجددًا.")
          : t("Could not download. Check your connection and try again.", "تعذر التنزيل. تحقق من الاتصال وحاول مجددًا.")}</p> : null}


        <p className="toolNote">{t("Offline audio", "التلاوات المحفوظة")}: {(storageBytes / 1048576).toFixed(1)} / 200 MB</p>
        {downloaded.length ? <button disabled={downloadState === "saving"} onClick={() => void caches.delete(AUDIO_CACHE).then(refreshDownloads).catch(() => setDownloadState("error"))}><Trash2 size={16} />{t("Remove all downloaded audio", "حذف جميع التلاوات المحفوظة")}</button> : null}
        <footer className="tafsirAttribution">{timedRecording ? "MP3Quran.net · Ayah timings" : playbackMode === "ayah" ? (ayahReciter.provider === "everyayah" ? "EveryAyah.com" : "Quran Foundation") : "MP3Quran.net"}</footer>
      </section>}
    </aside> : null}
    {running && readingPreferences.showTimer ? <button className="sessionTimer" type="button" role="timer" aria-label={t(`Reading time today ${sessionTime}`, `وقت القراءة اليوم ${sessionTime}`)} title={t("Open Daily reading", "فتح الورد اليومي")} onClick={() => setTab("goal")}><span className="sessionTimerDot" />{sessionTime}</button> : null}
    {celebration ? <div className="goalToast" role="status">{t("Today's goal completed", "أتممت ورد اليوم")}</div> : null}
    {reminderVisible ? <button className="goalToast reminderToast" role="status" onClick={() => { setReminderVisible(false); setTab("goal"); }}>{t("Time for your Quran reading", "حان وقت وردك القرآني")}</button> : null}
  </>;
}

