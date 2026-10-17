import { Component, type ErrorInfo, type ReactNode } from "react";
import { BookOpen, Copy, Download, RotateCw, X } from "lucide-react";
import { version } from "../package.json";
import "./ReaderErrorBoundary.css";

type State = { failed: boolean; errorName: string; occurredAt: string; attempts: number; notice: string };
type RecoveryWindow = Window & { __QURAN_RUST_HOST__?: boolean; chrome?: { webview?: { postMessage: (data: unknown) => void } } };

function preferences() {
  try {
    const value = JSON.parse(localStorage.getItem("quran-mushaf-qul-reader:v1") ?? "{}");
    const theme = value?.theme;
    return { arabic: value?.metadataLanguage !== "english", light: theme === "light" || theme === "sepia"
      || (theme === "system" && window.matchMedia("(prefers-color-scheme: light)").matches) };
  } catch { return { arabic: true, light: false }; }
}

// Reports deliberately exclude exception text/stacks, Quran/tafsir content,
// profile contents, URLs, local file paths, and download information.
function diagnosticReport(state: State, arabic: boolean) {
  const browser = navigator.userAgent.match(/(?:Edg|Chrome)\/[\d.]+/)?.[0] ?? "Unavailable";
  return JSON.stringify({ app: "Quran Mushaf Reader", version, stage: "React rendering", errorType: state.errorName,
    occurredAt: state.occurredAt, language: arabic ? "ar" : "en", browser, retryCount: state.attempts }, null, 2);
}

export class ReaderErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { failed: false, errorName: "Error", occurredAt: "", attempts: 0, notice: "" };

  static getDerivedStateFromError(error: unknown): Partial<State> {
    let name = "Error";
    try { if (error instanceof Error && /^[A-Za-z]{1,40}Error$/.test(error.name)) name = error.name; } catch { /* Use a generic type. */ }
    return { failed: true, errorName: name, occurredAt: new Date().toISOString(), notice: "" };
  }

  componentDidCatch(_error: unknown, _info: ErrorInfo) {
    // React already reports to the local developer console. Do not transmit errors.
  }

  async copyReport(arabic: boolean) {
    try {
      await navigator.clipboard.writeText(diagnosticReport(this.state, arabic));
      this.setState({ notice: arabic ? "تم نسخ معلومات الخطأ." : "Diagnostic information copied." });
    } catch {
      this.setState({ notice: arabic ? "تعذر النسخ. يمكنك حفظ المعلومات في ملف." : "Could not copy. You can save the information to a file instead." });
    }
  }

  saveReport(arabic: boolean) {
    try {
      const url = URL.createObjectURL(new Blob([diagnosticReport(this.state, arabic)], { type: "application/json" }));
      const link = document.createElement("a");
      link.href = url; link.download = `QuranMushafReader-diagnostics-${version}.json`;
      document.body.append(link); link.click(); link.remove();
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
      this.setState({ notice: arabic ? "تم تجهيز ملف معلومات الخطأ." : "Diagnostic file prepared." });
    } catch {
      this.setState({ notice: arabic ? "تعذر حفظ المعلومات. حاول إعادة فتح التطبيق." : "Could not save the information. Try reopening the app." });
    }
  }

  render() {
    if (!this.state.failed) return this.props.children;
    const { arabic, light } = preferences();
    const text = (english: string, translated: string) => arabic ? translated : english;
    const nativeWindow = window as RecoveryWindow;
    const native = nativeWindow.__QURAN_RUST_HOST__ === true;
    return <div className={`readerRecovery${light ? " readerRecoveryLight" : ""}`} lang={arabic ? "ar" : "en"} dir={arabic ? "rtl" : "ltr"}>
      {native ? <header className="readerRecoveryTitle" dir="ltr" onMouseDown={event => {
        if (event.button === 0 && !(event.target instanceof Element && event.target.closest("button"))) {
          nativeWindow.chrome?.webview?.postMessage({ type: "window-action", action: "drag" });
        }
      }}><span>Quran Mushaf Reader</span><button type="button" aria-label={text("Close app", "إغلاق التطبيق")}
        onClick={() => nativeWindow.chrome?.webview?.postMessage({ type: "window-action", action: "close" })}><X size={18} aria-hidden="true" /></button></header> : null}
      <main className="readerRecoveryCard" aria-labelledby="reader-recovery-heading">
        <BookOpen className="readerRecoveryIcon" size={36} strokeWidth={1.4} aria-hidden="true" />
        <h1 id="reader-recovery-heading">{text("The reader ran into a problem", "حدث خطأ أثناء عرض المصحف")}</h1>
        <p>{text("Try opening the reader again. If the problem returns, reload the app.", "حاول فتح المصحف من جديد. إذا تكرر الخطأ، أعد تحميل التطبيق.")}</p>
        <p className="readerRecoveryHint">{text("Your settings and downloaded recordings will be kept.", "ستبقى إعداداتك والتسجيلات التي نزّلتها محفوظة.")}</p>
        <div className="readerRecoveryActions">
          {this.state.attempts < 2 ? <button type="button" className="readerRecoveryPrimary" autoFocus
            onClick={() => this.setState(state => ({ failed: false, attempts: state.attempts + 1, notice: "" }))}>
            <RotateCw size={17} aria-hidden="true" />{text("Try again", "المحاولة مجددًا")}</button> : null}
          <button type="button" onClick={() => window.location.reload()}>{text("Reload app", "إعادة تحميل التطبيق")}</button>
        </div>
        <details className="readerRecoveryDetails"><summary>{text("Diagnostic information", "معلومات الخطأ")}</summary>
          <p>{text("Only the app version, error type, time, language and browser version are included. Nothing is sent automatically.",
            "تتضمن المعلومات إصدار التطبيق ونوع الخطأ ووقته واللغة وإصدار المتصفح فقط. لن تُرسل أي معلومات تلقائيًا.")}</p>
          <pre dir="ltr">{diagnosticReport(this.state, arabic)}</pre>
          <div className="readerRecoveryActions"><button type="button" onClick={() => void this.copyReport(arabic)}><Copy size={16} aria-hidden="true" />{text("Copy", "نسخ")}</button>
            <button type="button" onClick={() => this.saveReport(arabic)}><Download size={16} aria-hidden="true" />{text("Save file", "حفظ ملف")}</button></div>
        </details>
        <p className="readerRecoveryNotice" role="status">{this.state.notice}</p>
      </main>
    </div>;
  }
}
