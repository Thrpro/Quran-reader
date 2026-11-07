//! Portable, bounded guide content. Demonstrations use the real application
//! commands; this model never starts playback, downloads, or edits Quran data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tour {
    index: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Search,
    AyahMenu,
    Tafsir,
    Listening,
    Options,
}
impl Tour {
    pub const COUNT: usize = 5;
    pub fn index(self) -> usize {
        self.index
    }
    pub fn step(self) -> Step {
        [
            Step::Search,
            Step::AyahMenu,
            Step::Tafsir,
            Step::Listening,
            Step::Options,
        ][self.index]
    }
    /// False at the final step lets the host dismiss rather than wrap.
    pub fn advance(&mut self) -> bool {
        if self.index + 1 == Self::COUNT {
            return false;
        }
        self.index += 1;
        true
    }
    pub fn back(&mut self) {
        self.index = self.index.saturating_sub(1);
    }
    pub fn text(self, arabic: bool) -> (&'static str, &'static str) {
        match (self.step(), arabic) {
            (Step::Search, false) => (
                "Find an ayah or your place",
                "Try the Search above. Enter Quran words, a surah name, an ayah such as 2:255, or a page number. Browse Surah, Page or Juz below. Ctrl+F opens Search any time.",
            ),
            (Step::Search, true) => (
                "ابحث عن آية أو موضعك",
                "جرّب البحث أعلاه. اكتب كلمات من القرآن، أو اسم سورة، أو مرجع آية مثل 2:255، أو رقم صفحة. ويمكنك تصفح السور والصفحات والأجزاء. يفتح Ctrl+F البحث في أي وقت.",
            ),
            (Step::AyahMenu, false) => (
                "Start with an ayah",
                "Right-click an ayah to open this menu. Its highlight stays visible. Tafsir opens its explanation; the listening arrow offers this ayah, the page, the surah or the rest of the Quran. No audio starts during this guide.",
            ),
            (Step::AyahMenu, true) => (
                "ابدأ من آية",
                "انقر بزر الفأرة الأيمن على آية لفتح هذه القائمة مع بقاء تمييزها. يفتح التفسير شرحها، ويعرض سهم الاستماع خيارات الآية أو الصفحة أو السورة أو بقية القرآن. لا يبدأ تشغيل الصوت أثناء الدليل.",
            ),
            (Step::Tafsir, false) => (
                "Follow the explanation",
                "The highlighted ayah identifies what you are studying. Use the arrows to move between ayahs, choose a source, or adjust text size. Manage tafsirs shows optional Arabic books; English uses the bundled translation by default.",
            ),
            (Step::Tafsir, true) => (
                "تابع التفسير",
                "توضح الآية المميزة موضع دراستك. انتقل بين الآيات بالسهمين، واختر مصدر التفسير أو حجم النص. تعرض إدارة التفاسير الكتب العربية الإضافية المتاحة للتنزيل والقراءة دون اتصال.",
            ),
            (Step::Listening, false) => (
                "Listen at your own pace",
                "Choose a reciter and ayah range, then press Play when ready. Speed and repeat count are here. Pause, Stop and Show ayah remain available beside the reader. Editing the next range does not change an active recitation.",
            ),
            (Step::Listening, true) => (
                "استمع بالسرعة المناسبة لك",
                "اختر القارئ ونطاق الآيات، ثم اضغط التشغيل عندما تريد. تجد هنا السرعة وعدد مرات التكرار. تبقى أزرار الإيقاف المؤقت والإيقاف وإظهار الآية الحالية بجانب المصحف. لا يغيّر تعديل النطاق تلاوة جارية.",
            ),
            (Step::Options, false) => (
                "Make the reader yours",
                "Options contains appearance, language, saved marks, daily reading and accessible text. Ctrl+B saves your place; F11 enters or leaves full screen. You can reopen this guide from How to use the reader. Finish returns to your reading.",
            ),
            (Step::Options, true) => (
                "خصص تجربة القراءة",
                "تجد في الخيارات المظهر واللغة والعلامات المحفوظة والورد اليومي وعرض النص الميسر. يحفظ Ctrl+B موضعك، ويدخل F11 ملء الشاشة أو يخرج منه. يمكنك فتح هذا الدليل مجددًا من كيفية استخدام التطبيق. تعيدك النهاية إلى قراءتك.",
            ),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_forward_back_and_no_wrap() {
        let mut tour = Tour::default();
        tour.back();
        assert_eq!(tour.index(), 0);
        for n in 0..Tour::COUNT {
            assert_eq!(tour.index(), n);
            for language in [false, true] {
                let (title, body) = tour.text(language);
                assert!(!title.is_empty() && !body.is_empty());
            }
            assert_eq!(tour.advance(), n + 1 < Tour::COUNT);
        }
        assert_eq!(tour.step(), Step::Options);
        tour.back();
        assert_eq!(tour.step(), Step::Listening);
    }
}
