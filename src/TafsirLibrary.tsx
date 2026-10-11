import { useEffect, useRef, useState } from "react";
import { EXTRA_TAFSIRS, downloadTafsir, installedTafsirs, removeTafsir, type ExtraTafsirId, type PackInfo } from "./tafsirDownloads";

export function TafsirDownloads({ onChange, textSize, onTextSizeChange }: {
  onChange: () => void;
  textSize: number;
  onTextSizeChange: (size: number) => void;
}) {
  const [packs, setPacks] = useState<Partial<Record<ExtraTafsirId, PackInfo>>>({});
  const [active, setActive] = useState<ExtraTafsirId | null>(null);
  const [progress, setProgress] = useState({ chapters: 0, bytes: 0 });
  const [error, setError] = useState("");
  const controller = useRef<AbortController | null>(null);
  useEffect(() => {
    void installedTafsirs().then(setPacks).catch(() => setError("تعذر فتح مساحة التخزين."));
    return () => controller.current?.abort();
  }, []);
  async function install(id: ExtraTafsirId) {
    if (controller.current) return;
    const abort = new AbortController(); controller.current = abort;
    setActive(id); setError(""); setProgress({ chapters: 0, bytes: 0 });
    try {
      await downloadTafsir(id, abort.signal, (chapters, bytes) => setProgress({ chapters, bytes }));
      setPacks(await installedTafsirs()); onChange();
    } catch (error) {
      if (!abort.signal.aborted && error instanceof Error && /unavailable: (401|403|404)/.test(error.message)) setError("لا يتيح المصدر تنزيل هذا الكتاب حاليًا. لا يلزم إدخال مفاتيح أو كلمات مرور في التطبيق.");
      else if (!abort.signal.aborted) setError("تعذر إكمال التنزيل. تحقق من الاتصال والمساحة المتاحة ثم حاول مجددًا. لم يُثبّت ملف غير مكتمل.");
    } finally { controller.current = null; setActive(null); }
  }
  return <details name="study-actions" className="tafsirDownloads">
    <summary title="تنزيل التفاسير وإدارتها">إدارة التفاسير</summary>
    <div className="tafsirLibraryPopover">
    <label className="tafsirLibraryTextSize">
      <span>حجم نص التفسير <output>{textSize}</output></span>
      <input aria-label="حجم نص التفسير" type="range" min="16" max="26"
        value={textSize} onChange={event => onTextSizeChange(Number(event.target.value))} />
    </label>
    <h3>تنزيل التفاسير</h3>
    <p>المصدر: Quran.com. تنزيل اختياري حسب إتاحة المصدر؛ لا يتطلب إدخال حساب أو مفتاح في التطبيق. تتوفر الكتب بعد اكتمال التنزيل. إغلاق التفسير يوقف التنزيل الجاري.</p>
    <p>الحد الأقصى: ٦٤ ميجابايت للكتاب، و٢٠٠ ميجابايت لجميع الكتب الإضافية.</p>
    {EXTRA_TAFSIRS.map(item => <div className="tafsirDownloadRow" key={item.id}>
      <span>{item.name}{packs[item.id] ? <small> · {(packs[item.id]!.bytes / 1048576).toFixed(1)} MB</small> : null}</span>
      {packs[item.id] ? <button disabled={!!active} onClick={async () => {
        try { await removeTafsir(item.id); setPacks(await installedTafsirs()); onChange(); }
        catch { setError("تعذر حذف الكتاب."); }
      }}>حذف</button> : <button disabled={!!active} onClick={() => void install(item.id)}>تنزيل</button>}
      {active === item.id ? <div className="tafsirDownloadProgress">
        <progress max={114} value={progress.chapters} aria-label="تقدم تنزيل التفسير" />
        <span>{Math.floor(progress.chapters / 114 * 100)}% · {progress.chapters}/114 · {(progress.bytes / 1048576).toFixed(1)} MB</span>
        <button onClick={() => controller.current?.abort()}>إلغاء</button>
      </div> : null}
    </div>)}
    {error ? <p role="alert">{error}</p> : null}
    </div>
  </details>;
}
