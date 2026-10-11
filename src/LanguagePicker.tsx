import { useEffect, useRef } from "react";

type Language = "arabic" | "english";

export function LanguageChoices({ selected, onChoose }: {
  selected?: Language;
  onChoose: (language: Language) => void;
}) {
  return (
    <div className="segmented languageChoices" dir="ltr">
      <button type="button" lang="en" dir="ltr"
        aria-pressed={selected ? selected === "english" : undefined}
        onClick={() => onChoose("english")}>English</button>
      <button type="button" lang="ar" dir="rtl"
        aria-pressed={selected ? selected === "arabic" : undefined}
        onClick={() => onChoose("arabic")}>العربية</button>
    </div>
  );
}

export function LanguageSetup({ onChoose }: { onChoose: (language: Language) => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = dialogRef.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);

  return (
    <dialog ref={dialogRef} className="languageDialog" aria-labelledby="language-setup-title"
      onCancel={(event) => event.preventDefault()}>
      <header className="languageSetupHeader" id="language-setup-title">
        <h2 lang="en" dir="ltr">Choose your language</h2>
        <h2 lang="ar" dir="rtl">اختر لغتك</h2>
      </header>
      <div className="languageSetupChoices" dir="ltr">
        <button type="button" lang="en" dir="ltr" aria-label="English"
          onClick={() => onChoose("english")}>
          <strong>English</strong>
          <span>Continue in English</span>
        </button>
        <button type="button" lang="ar" dir="rtl" aria-label="العربية"
          onClick={() => onChoose("arabic")}>
          <strong>العربية</strong>
          <span>المتابعة باللغة العربية</span>
        </button>
      </div>
      <div className="languageTafsirHint">
        <p lang="en" dir="ltr">Right-click any ayah for translation and listening options, or point to it and press T for translation. Open Daily reading and choose Start reading to track your goal.</p>
        <p lang="ar" dir="rtl">انقر بزر الفأرة الأيمن على أي آية لعرض خيارات التفسير والاستماع، أو مرّر المؤشر عليها واضغط T للتفسير. افتح «الورد اليومي» واختر «ابدأ القراءة» لاحتساب تقدمك.</p>
      </div>
    </dialog>
  );
}

