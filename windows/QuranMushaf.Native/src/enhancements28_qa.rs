//! Native regressions for the deliberate text view, pixel scrolling and spacing.
use super::*;
impl App {
    pub(super) unsafe fn qa_enhancements28(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let prefs = self.prefs.clone();
            let selected = self.selected;
            let source = self.source;
            let panel = self.panel;
            let keyboard = self.keyboard_focus;
            let playback = std::mem::take(&mut self.playback);
            let popup = self.popup.take();
            let pending_speed = self.speed_selection_pending;
            let mut cases = vec![];
            let result = (|| -> Result<()> {
                self.search = false;
                self.text_view = false;
                self.panel_animation = None;
                self.study_reveal_pending = false;
                self.prefs.speed = 1.5;
                self.prefs.remember_speed = false;
                self.speed_selection_pending = false;
                self.prepare_recitation_rate();
                if self.prefs.speed != 1. || self.select(SPEED) != 1 {
                    return Err("A fresh recitation did not default to 1x".into());
                }
                self.prefs.remember_speed = true;
                self.prefs.speed = 1.5;
                self.prepare_recitation_rate();
                if self.prefs.speed != 1.5 {
                    return Err("The explicit speed-memory preference was ignored".into());
                }
                self.prefs.remember_speed = false;
                self.prefs.speed = 1.25;
                self.speed_selection_pending = true;
                self.prepare_recitation_rate();
                if self.prefs.speed != 1.25 || self.speed_selection_pending {
                    return Err(
                        "An explicit speed choice before Play was lost or retained twice".into(),
                    );
                }
                self.prepare_recitation_rate();
                if self.prefs.speed != 1. {
                    return Err("A one-time speed choice leaked to the next recitation".into());
                }
                for arabic in [false, true] {
                    for theme in [0, 1, 2] {
                        self.prefs.arabic = arabic;
                        self.prefs.theme = theme;
                        self.replace_background(self.palette());
                        self.refresh_labels();
                        self.keyboard_focus = false;
                        self.panel = Panel::None;
                        self.accessible_view()?;
                        if self.text_view
                            || self.popup.as_ref().is_none_or(|p| {
                                p.rows.iter().map(|r| r.id).collect::<Vec<_>>()
                                    != [ENABLE_TEXT_VIEW, CANCEL_TEXT_VIEW]
                            })
                            || GetFocus() != self.controls[&(MENU_ROW + 1)]
                        {
                            return Err(
                                "Text view opened without its non-changing confirmation choice"
                                    .into(),
                            );
                        }
                        self.paint()?;
                        self.capture(&out.join(format!(
                            "native28-{}-theme-{theme}-confirmation.png",
                            if arabic { "arabic" } else { "english" }
                        )))?;
                        self.popup_command(MENU_ROW + 1)?;
                        if self.text_view || self.popup.is_some() {
                            return Err(
                                "Cancelling accessible text changed the page presentation".into()
                            );
                        }
                        self.accessible_view()?;
                        self.popup_command(MENU_ROW)?;
                        if !self.text_view
                            || !IsWindowVisible(self.controls[&EXIT_TEXT_VIEW]).as_bool()
                        {
                            return Err("Accessible text lacks a persistent return button".into());
                        }
                        if self.text(EXIT_TEXT_VIEW)
                            != self.tr("Return to Quran pages", "العودة إلى صفحات المصحف")
                        {
                            return Err("The persistent text-view exit is not localized".into());
                        }
                        self.paint()?;
                        self.capture(&out.join(format!(
                            "native28-{}-theme-{theme}-text-view.png",
                            if arabic { "arabic" } else { "english" }
                        )))?;
                        self.command(EXIT_TEXT_VIEW, 0)?;
                        if self.text_view
                            || IsWindowVisible(self.controls[&EXIT_TEXT_VIEW]).as_bool()
                        {
                            return Err(
                                "Return to Quran pages did not restore normal presentation".into(),
                            );
                        }

                        self.search = true;
                        self.set_text(QUERY, "");
                        self.search_animation = None;
                        self.browse_tab = BROWSE_SURAH;
                        self.update_search()?;
                        self.resize();
                        let query = self.placements.borrow()[&QUERY].0;
                        let browse = self.placements.borrow()[&BROWSE_QUERY].0;
                        if query.y != 142. || query.h != 24. || browse.y != 326. || browse.h != 24.
                        {
                            return Err(
                                "Search edits lost their vertically centered geometry".into()
                            );
                        }
                        self.paint()?;
                        self.capture(&out.join(format!(
                            "native28-{}-theme-{theme}-search.png",
                            if arabic { "arabic" } else { "english" }
                        )))?;
                        self.search = false;
                        self.keyboard_focus = false;
                        self.command(SETTINGS, 0)?;
                        let appearance = self
                            .popup
                            .as_ref()
                            .unwrap()
                            .rows
                            .iter()
                            .position(|row| {
                                row.children.iter().any(|child| child.id == THEME_SELECTOR)
                            })
                            .ok_or("Appearance is missing")?;
                        self.popup_command(MENU_ROW + appearance as u32)?;
                        if GetFocus() != self.hwnd
                            || self.text(THEME_LABEL) != self.tr("Theme", "المظهر")
                        {
                            return Err("Pointer entry preselected the theme field or left unconventional wording".into());
                        }
                        self.paint()?;
                        self.capture(&out.join(format!(
                            "native28-{}-theme-{theme}-appearance.png",
                            if arabic { "arabic" } else { "english" }
                        )))?;
                        self.keyboard_focus = true;
                        self.key(9)?;
                        if GetFocus() != self.controls[&THEME_SELECTOR] {
                            return Err("Keyboard entry cannot reach the theme dropdown".into());
                        }
                        self.popup_command(MENU_CLOSE)?;
                        self.keyboard_focus = false;
                        self.selected = Some((2, 282));
                        self.prefs.page = self
                            .resources
                            .index()?
                            .iter()
                            .find(|h| h.surah == 2 && h.ayah == 282)
                            .ok_or("Missing original ayah")?
                            .page_number;
                        self.source = if arabic { 0 } else { 2 };
                        self.panel = Panel::Study;
                        self.panel_animation = None;
                        self.study()?;
                        self.resize();
                        let original = self.text(STUDY);
                        let font = SendMessageW(self.controls[&STUDY], WM_GETFONT, None, None);
                        let mut selection = CHARRANGE {
                            cpMin: 35,
                            cpMax: 78,
                        };
                        SendMessageW(
                            self.controls[&STUDY],
                            EM_EXSETSEL,
                            None,
                            Some(LPARAM((&selection as *const CHARRANGE) as isize)),
                        );
                        let mut positions =
                            vec![crate::smooth_scroll::position(self.controls[&STUDY]).y];
                        SendMessageW(
                            self.controls[&STUDY],
                            WM_MOUSEWHEEL,
                            Some(WPARAM((-120i16 as u16 as usize) << 16)),
                            None,
                        );
                        let animated = crate::smooth_scroll::active(self.controls[&STUDY]);
                        for _ in 0..45 {
                            if !crate::smooth_scroll::active(self.controls[&STUDY]) {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(8));
                            SendMessageW(
                                self.controls[&STUDY],
                                WM_TIMER,
                                Some(WPARAM(0x514d)),
                                None,
                            );
                            positions.push(crate::smooth_scroll::position(self.controls[&STUDY]).y);
                        }
                        SendMessageW(
                            self.controls[&STUDY],
                            EM_EXGETSEL,
                            None,
                            Some(LPARAM((&mut selection as *mut CHARRANGE) as isize)),
                        );
                        if crate::smooth_scroll::active(self.controls[&STUDY])
                            || self.text(STUDY) != original
                            || SendMessageW(self.controls[&STUDY], WM_GETFONT, None, None) != font
                            || (selection.cpMin, selection.cpMax) != (35, 78)
                            || positions.windows(2).any(|pair| pair[1] < pair[0])
                            || positions.last() <= positions.first()
                        {
                            return Err("Pixel scrolling changed text, selection, font, direction or failed to settle".into());
                        }
                        if animated
                            && positions
                                .iter()
                                .copied()
                                .collect::<std::collections::BTreeSet<_>>()
                                .len()
                                < 4
                        {
                            return Err("Smooth scrolling jumped instead of producing intermediate positions".into());
                        }
                        SendMessageW(
                            self.controls[&STUDY],
                            WM_MOUSEWHEEL,
                            Some(WPARAM((-30i16 as u16 as usize) << 16)),
                            None,
                        );
                        SendMessageW(self.controls[&STUDY], WM_KEYDOWN, Some(WPARAM(0x24)), None);
                        if crate::smooth_scroll::active(self.controls[&STUDY]) {
                            return Err(
                                "Native keyboard navigation did not cancel pending scrolling"
                                    .into(),
                            );
                        }
                        let mut tiny_gesture_px = None;
                        if animated {
                            crate::smooth_scroll::stop(self.controls[&STUDY]);
                            let zero = POINT::default();
                            SendMessageW(
                                self.controls[&STUDY],
                                EM_SETSCROLLPOS,
                                None,
                                Some(LPARAM((&zero as *const POINT) as isize)),
                            );
                            for _ in 0..4 {
                                SendMessageW(
                                    self.controls[&STUDY],
                                    WM_MOUSEWHEEL,
                                    Some(WPARAM((-1i16 as u16 as usize) << 16)),
                                    None,
                                );
                                for _ in 0..45 {
                                    if !crate::smooth_scroll::active(self.controls[&STUDY]) {
                                        break;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(8));
                                    SendMessageW(
                                        self.controls[&STUDY],
                                        WM_TIMER,
                                        Some(WPARAM(0x514d)),
                                        None,
                                    );
                                }
                            }
                            let actual = crate::smooth_scroll::position(self.controls[&STUDY]).y;
                            let expected = ((*positions.last().unwrap() - positions[0]) as f64
                                / 30.)
                                .round() as i32;
                            if actual != expected
                                || crate::smooth_scroll::active(self.controls[&STUDY])
                            {
                                return Err(
                                    "Small settled wheel gestures lost their fractional distance"
                                        .into(),
                                );
                            }
                            tiny_gesture_px = Some(actual);
                        }
                        self.paint()?;
                        self.capture(&out.join(format!(
                            "native28-{}-theme-{theme}-study.png",
                            if arabic { "arabic" } else { "english" }
                        )))?;
                        self.panel = Panel::None;
                        self.set_panel(Panel::Study);
                        let cold_pending = self.study_reveal_pending;
                        if self.panel_animation.is_some() {
                            let width = self.panel_width();
                            self.study()?;
                            std::thread::sleep(std::time::Duration::from_millis(30));
                            if self.panel_width() != width {
                                return Err(
                                    "Cold Tafsir layout consumed its unpainted opening animation"
                                        .into(),
                                );
                            }
                            self.resize();
                            self.paint()?;
                            if self.study_reveal_pending {
                                return Err(
                                    "The visible Tafsir reveal clock did not start after drawing"
                                        .into(),
                                );
                            }
                        }
                        self.panel_animation = None;
                        self.study_reveal_pending = false;
                        cases.push(serde_json::json!({"arabic":arabic,"theme":theme,"confirmationRequired":true,"cancelFocusedByDefault":true,"returnButtonPersistsWhileEnabled":true,"searchLabelGapDip":10,"nativeEditHeightDip":24,"scrollAnimated":animated,"scrollPositionsPx":positions,"scrollSettled":true,"tinySettledGesturesPreserved":true,"tinyGesturesPx":tiny_gesture_px,"nativeSelectionTextAndFontUnchanged":true,"keyboardCancelsScroll":true,"themePointerKeepsParentFocus":true,"themeKeyboardFocusRetained":true,"returnButtonLocalized":true,"coldRevealFirstFrame":cold_pending}));
                    }
                }
                std::fs::write(
                    out.join("enhancements28.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"passed":true,"cases":cases,"freshSpeedDefaultsToNormal":true,"rememberSpeedOptIn":true,"explicitPrePlaySpeedIsOneTime":true,"scope":"Actual native controls and text/selection/font/scroll readback. Deterministic scroll timer dispatch with real elapsed time; not physical wheel or compositor FPS certification. Text-view return stays visible until disabled, rather than expiring after 20 seconds."}),
                    )?,
                )?;
                Ok(())
            })();
            crate::smooth_scroll::stop(self.controls[&STUDY]);
            self.prefs = prefs;
            self.selected = selected;
            self.source = source;
            self.panel = panel;
            self.keyboard_focus = keyboard;
            self.playback = playback;
            self.popup = popup;
            self.speed_selection_pending = pending_speed;
            self.panel_animation = None;
            self.study_reveal_pending = false;
            self.search = false;
            self.text_view = false;
            self.replace_background(self.palette());
            self.refresh_labels();
            self.study()?;
            self.resize();
            result
        }
    }
}
