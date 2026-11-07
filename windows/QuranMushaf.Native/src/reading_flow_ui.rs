//! Deliberate entry and a permanent way back from accessible text presentation.
use super::*;
impl App {
    pub(super) unsafe fn accessible_view(&mut self) -> Result<()> {
        unsafe {
            if self.text_view {
                return self.set_accessible_view(false);
            }
            self.show_popup(
                self.tr("Switch to text view?", "تفعيل العرض النصي؟").into(),
                vec![
                    MenuRow {
                        id: ENABLE_TEXT_VIEW,
                        label: self.tr("Enable text view", "تفعيل العرض النصي").into(),
                        detail: self
                            .tr(
                                "Replaces pages; for assistive technology.",
                                "يستبدل عرض الصفحات بنص لقارئات الشاشة.",
                            )
                            .into(),
                        children: vec![],
                    },
                    MenuRow {
                        id: CANCEL_TEXT_VIEW,
                        label: self
                            .tr("Keep Quran pages", "الإبقاء على صفحات المصحف")
                            .into(),
                        detail: self.tr("Cancel", "إلغاء").into(),
                        children: vec![],
                    },
                ],
                self.width / 2. + if self.prefs.arabic { 172. } else { -172. },
                100.,
                false,
            )?;
            self.resize();
            // Confirmation always opens with its non-changing choice focused.
            focus_window(self.controls[&(MENU_ROW + 1)])?;
            Ok(())
        }
    }
    pub(super) unsafe fn set_accessible_view(&mut self, enabled: bool) -> Result<()> {
        unsafe {
            self.popup = None;
            self.text_view = enabled;
            self.text_pages.clear();
            self.refresh_study_font();
            self.resize();
            focus_window(if enabled {
                self.controls[&TEXT_VIEW]
            } else {
                self.hwnd
            })?;
            self.invalidate();
            Ok(())
        }
    }
    pub(super) unsafe fn prepare_recitation_rate(&mut self) {
        unsafe {
            if std::mem::take(&mut self.speed_selection_pending) {
                return;
            }
            if self.prefs.begin_recitation() {
                self.dirty = true;
                SendMessageW(self.controls[&SPEED], CB_SETCURSEL, Some(WPARAM(1)), None);
            }
        }
    }
}
