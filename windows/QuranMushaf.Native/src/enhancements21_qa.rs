use super::*;
impl App {
    pub(super) unsafe fn qa_enhancements21(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let preferences = self.prefs.clone();
            self.popup = None;
            self.search = false;
            self.panel = Panel::Study;
            self.panel_animation = None;
            self.prefs.arabic = true;
            self.refresh_labels();
            self.selected = Some((2, 10));
            self.source = 0;
            self.study()?;
            self.resize();
            let source = self.source;
            focus_window(self.controls[&SOURCE])?;
            if !self.key(37)? || self.selected != Some((2, 11)) || self.source != source {
                return Err("Closed tafsir selector stole next-ayah key".into());
            }
            focus_window(self.controls[&SOURCE])?;
            if !self.key(39)? || self.selected != Some((2, 10)) || self.source != source {
                return Err("Closed tafsir selector stole previous-ayah key".into());
            }
            self.command(SOURCE, CBN_CLOSEUP)?;
            if GetFocus() != self.controls[&STUDY] {
                return Err("Tafsir source close retained native combo focus".into());
            }
            self.panel = Panel::None;
            let mut cases = vec![];
            for arabic in [false, true] {
                self.prefs.arabic = arabic;
                self.refresh_labels();
                self.resize();
                self.options()?;
                let mut frames = vec![];
                for ms in [0, 45, 90, 135, 180] {
                    self.popup.as_mut().unwrap().motion =
                        Some(Instant::now() - std::time::Duration::from_millis(ms));
                    self.resize();
                    self.paint()?;
                    let composed = self
                        .placements
                        .borrow()
                        .iter()
                        .filter(|(id, (_, show))| (**id >= MENU_ROW && **id <= MENU_CLOSE) && *show)
                        .count();
                    if composed < 3
                        || self
                            .motion_controls
                            .borrow()
                            .keys()
                            .any(|id| (MENU_ROW..=MENU_CLOSE).contains(id))
                    {
                        return Err("Popup retained independently presented child text".into());
                    }
                    self.capture(&out.join(format!(
                        "native-options21-{}-{ms}.png",
                        if arabic { "arabic" } else { "english" }
                    )))?;
                    frames.push(composed);
                }
                self.popup = None;
                cases.push(serde_json::json!({"arabic":arabic,"composedControls":frames}));
            }
            self.startup_reveal = Some(Instant::now() - std::time::Duration::from_millis(200));
            let generation = self.layout_generation;
            self.advance_control_motion();
            if self.startup_reveal.is_some() || self.layout_generation != generation {
                return Err("Startup reveal relaid out the reader or failed to finish".into());
            }
            for (i, gib) in [1, 4, 16, 0].into_iter().enumerate() {
                self.command(OFFLINE_LIMIT_FIRST + i as u32, 0)?;
                let stats = self.cache()?.stats()?;
                let expected = if gib == 0 {
                    u64::MAX
                } else {
                    gib * 1024 * 1024 * 1024
                };
                if stats.library_limit != expected {
                    return Err("Offline limit was not applied".into());
                }
            }
            self.prefs = preferences;
            self.refresh_labels();
            self.panel = Panel::None;
            self.cache()?;
            self.resize();
            std::fs::write(
                out.join("enhancements21.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"tafsirClosedSelectorNavigatesAyahs":true,"tafsirCloseReturnsReadingFocus":true,"popupFrames":cases,"startupRevealFinishesWithoutLayout":true,"offlineLimitsGiB":[1,4,16,"unlimited"],"scope":"Actual native controls, frozen animation frames and direct keyboard dispatch; not physical input or display FPS certification"}),
                )?,
            )?;
            Ok(())
        }
    }
}
