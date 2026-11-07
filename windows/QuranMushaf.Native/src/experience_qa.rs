//! Compiled-in native QA only. Exercises actual controls in an isolated profile.
use super::*;
impl App {
    pub(super) unsafe fn qa_experience19(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let prefs = self.prefs.clone();
            let selected = self.selected;
            let panel = self.panel;
            let source = self.source;
            let following = self.playback.following;
            let audio = self.audio.take();
            let installed = self.installed_tafsirs.clone();
            let mut cases = vec![];
            let result = (|| -> Result<()> {
                for arabic in [false, true] {
                    self.prefs.arabic = arabic;
                    self.prefs.page = 1;
                    self.selected = Some((1, 1));
                    self.panel = Panel::None;
                    self.panel_animation = None;
                    self.popup = None;
                    self.search = false;
                    self.theme_start = None;
                    self.theme_frame = None;
                    self.prefs.theme = if arabic { 0 } else { 1 };
                    self.replace_background(self.palette());
                    self.refresh_labels();
                    self.resize();
                    let original_page = self.prefs.page;
                    let original_follow = self.playback.following;
                    self.command(HELP, 0)?;
                    for step in 0..crate::guided_tour::Tour::COUNT {
                        if self
                            .guide
                            .as_ref()
                            .is_none_or(|state| state.tour.index() != step)
                            || !IsWindowVisible(self.controls[&GUIDE_NEXT]).as_bool()
                            || self.text(GUIDE_BODY).is_empty()
                        {
                            return Err("Guided help lost its step or native controls".into());
                        }
                        let body_rtl = GetWindowLongPtrW(self.controls[&GUIDE_BODY], GWL_EXSTYLE)
                            & WS_EX_RTLREADING.0 as isize
                            != 0;
                        if body_rtl != arabic {
                            return Err("Guide body direction does not follow language".into());
                        }
                        self.search_animation = None;
                        self.panel_animation = None;
                        if let Some(p) = &mut self.popup {
                            p.motion = None;
                        }
                        self.resize();
                        let expected = match step {
                            0 => self.search,
                            1 => self.popup.as_ref().is_some_and(|p| p.context),
                            2 => {
                                self.panel == Panel::Study
                                    && self.selected == Some((1, 1))
                                    && IsWindowVisible(self.controls[&STUDY]).as_bool()
                            }
                            3 => {
                                self.panel == Panel::Listen
                                    && IsWindowVisible(self.controls[&SPEED]).as_bool()
                                    && IsWindowVisible(self.controls[&REPEAT]).as_bool()
                            }
                            _ => self
                                .popup
                                .as_ref()
                                .is_some_and(|p| p.docked && p.rtl == arabic),
                        };
                        if !expected
                            || self.audio.is_some()
                            || self.tafsir_job.is_some()
                            || self.download.is_some()
                        {
                            return Err(
                                "Guide demonstration differs or started a media/download job"
                                    .into(),
                            );
                        }
                        self.invalidate();
                        self.paint()?;
                        self.capture(&out.join(format!(
                            "native-guide-{}-{step}.png",
                            if arabic { "arabic" } else { "english" }
                        )))?;
                        self.command(GUIDE_NEXT, 0)?;
                    }
                    if self.guide.is_some()
                        || self.search
                        || self.popup.is_some()
                        || self.prefs.page != original_page
                        || self.panel != Panel::None
                        || self.selected != Some((1, 1))
                        || self.playback.following != original_follow
                    {
                        return Err("Finishing guide did not restore reading".into());
                    }
                    self.command(HELP, 0)?;
                    self.command(GUIDE_NEXT, 0)?;
                    self.command(GUIDE_BACK, 0)?;
                    if self
                        .guide
                        .as_ref()
                        .is_none_or(|state| state.tour.index() != 0)
                    {
                        return Err("Guide Back did not restore the first step".into());
                    }
                    self.key(27)?;
                    if self.guide.is_some() {
                        return Err("Escape did not close guided help".into());
                    }
                    self.command(MANAGE_TAFSIR, 0)?;
                    self.panel_animation = None;
                    self.resize();
                    self.installed_tafsirs.clear();
                    self.refresh_tafsir_actions();
                    for i in 0..tafsir_library::SOURCES.len() {
                        self.tafsir_arabic = tafsir_library::SOURCES[i].arabic;
                        self.resize();
                        let id = TAFSIR_ACTION_FIRST + i as u32;
                        let (bounds, visible) = self.placements.borrow()[&id];
                        let blocked = tafsir_library::SOURCES[i].download_issue().is_some();
                        if !self.text(id).starts_with(if blocked {
                            self.tr("Pending", "غير متاح")
                        } else {
                            self.tr("Download", "تنزيل")
                        }) || IsWindowEnabled(self.controls[&id]).as_bool() == blocked
                            || !visible
                            || bounds.x < 0.
                            || bounds.x + bounds.w > self.width
                            || bounds.y < 100.
                            || bounds.y + bounds.h > self.height
                        {
                            return Err("Missing per-source Download action".into());
                        }
                    }
                    self.installed_tafsirs = vec![1];
                    self.refresh_tafsir_actions();
                    if !self
                        .text(TAFSIR_ACTION_FIRST + 1)
                        .starts_with(self.tr("Remove", "حذف"))
                        || !self
                            .text(TAFSIR_ACTION_FIRST)
                            .starts_with(self.tr("Download", "تنزيل"))
                    {
                        return Err("Installed tafsir action applied to the wrong row".into());
                    }
                    self.invalidate();
                    self.paint()?;
                    self.capture(&out.join(format!(
                        "native-tafsir-cards-{}.png",
                        if arabic { "arabic" } else { "english" }
                    )))?;
                    self.source = if arabic { 0 } else { 2 };
                    self.command(STUDY, 0)?;
                    self.panel_animation = None;
                    self.resize();
                    SendMessageW(
                        self.controls[&STUDY],
                        WM_VSCROLL,
                        Some(WPARAM(SB_BOTTOM.0 as usize)),
                        None,
                    );
                    self.invalidate();
                    self.paint()?;
                    let (rect, y, color) =
                        crate::reading_control::visible_attribution_rule(self.controls[&STUDY])
                            .ok_or(
                                "Attribution rule is not attached to the scrolled source paragraph",
                            )?;
                    if rect.right - rect.left < 180
                        || y < rect.top
                        || self.text(STUDY).contains("────────")
                        || color.0 == self.palette().ink_rgb()
                    {
                        return Err(
                            "Attribution divider is narrow, literal, or full ink contrast".into(),
                        );
                    }
                    self.capture(&out.join(format!(
                        "native-tafsir-attribution-{}.png",
                        if arabic { "arabic" } else { "english" }
                    )))?;
                    cases.push(serde_json::json!({"arabic":arabic,"steps":5,"realDemonstrationControls":true,"finishRestoresReading":true,"backAndEscape":true,"noPlaybackOrDownloadStarted":true,"localizedGuideDirection":true,"perSourceDownloadRemove":true,"scrollingFaintFullWidthAttribution":true,"dividerPixels":[rect.left,rect.right,y],"dividerColor":color.0}));
                    self.popup = None;
                    self.panel = Panel::None;
                    self.panel_animation = None;
                }
                Ok(())
            })();
            self.guide = None;
            self.prefs = prefs;
            self.selected = selected;
            self.panel = panel;
            self.panel_animation = None;
            self.popup = None;
            self.search = false;
            self.source = source;
            self.playback.following = following;
            self.audio = audio;
            self.installed_tafsirs = installed;
            self.refresh_labels();
            self.replace_background(self.palette());
            self.resize();
            result?;
            std::fs::write(
                out.join("experience19-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"cases":cases,"scope":"Actual native controls and client captures; source installation states simulated without downloading/removing any original book. Not physical-input or display FPS certification."}),
                )?,
            )?;
            Ok(())
        }
    }
}
