// Runs before the application bundle: one short message in the reader's language.
(() => {
  let arabic = navigator.language.toLowerCase().startsWith('ar');
  try {
    const saved = JSON.parse(localStorage.getItem('quran-mushaf-qul-reader:v1') || 'null');
    if (saved?.metadataLanguage === 'arabic' || saved?.metadataLanguage === 'english') {
      arabic = saved.metadataLanguage === 'arabic';
    }
  } catch { /* Use the device language when saved preferences are unavailable. */ }
  const label = document.getElementById('startup-label');
  if (!label) return;
  label.lang = arabic ? 'ar' : 'en';
  label.dir = arabic ? 'rtl' : 'ltr';
  label.textContent = arabic ? 'جارٍ تحميل التطبيق…' : 'Loading…';
})();
