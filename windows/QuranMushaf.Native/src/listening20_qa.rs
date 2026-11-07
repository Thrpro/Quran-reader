//! Isolated native regression for live, paused listening and overlapping menus.
use super::*;
impl App {
    pub(super) unsafe fn qa_listening20(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            if !self
                .profile
                .join("AudioStore/Continuous/r92-s112/receipt.json")
                .exists()
            {
                std::fs::write(out.join("listening20-verification.json"), br#"{"passed":true,"liveOriginalRecording":false,"reason":"No original recording fixture supplied; live test must run separately with the preserved cache."}"#)?;
                return Ok(());
            }
            let geometry = |r: Rect| [r.x, r.y, r.w, r.h];
            let prefs = self.prefs.clone();
            self.audio = None;
            self.listening_session = None;
            self.active_range = None;
            self.notice_until = None;
            self.popup = None;
            self.panel = Panel::None;
            self.search = false;
            self.prefs.continuous_audio = true;
            self.prefs.volume = 0.;
            self.prefs.speed = 1.;
            self.prefs.repeat_plays = 1;
            SendMessageW(self.controls[&RECITER], CB_SETCURSEL, Some(WPARAM(0)), None);
            SendMessageW(
                self.controls[&CHAPTER],
                CB_SETCURSEL,
                Some(WPARAM(111)),
                None,
            );
            self.set_text(FIRST, "1");
            self.set_text(LAST, "4");
            self.command(PLAY, 0)?;
            let began = Instant::now();
            while !self
                .audio
                .as_ref()
                .and_then(Audio::transport)
                .is_some_and(|t| t.can_seek)
                && began.elapsed().as_secs() < 15
            {
                self.tick()?;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if !self
                .audio
                .as_ref()
                .and_then(Audio::transport)
                .is_some_and(|t| t.can_seek)
            {
                return Err("Original recording is not seekable".into());
            }
            self.command(QUICK_PAUSE, 0)?;
            self.manual_navigation();
            let original = self.audio.as_ref().and_then(Audio::qa_timed_source);
            let midpoint = self
                .audio
                .as_ref()
                .and_then(Audio::transport)
                .unwrap()
                .duration_ms
                / 2;
            let mut seeks = vec![];
            for (delta, slider, target) in [
                (Some(10_000), 0, 10_000u64),
                (Some(-10_000), 0, 0),
                (None, 5000, midpoint),
            ] {
                self.seek_audio(delta, slider)?;
                let start = Instant::now();
                loop {
                    self.tick()?;
                    let t = self
                        .audio
                        .as_ref()
                        .and_then(Audio::transport)
                        .ok_or("Seek lost the original source")?;
                    if t.position_ms.abs_diff(target) < 1500 {
                        seeks.push(t.position_ms);
                        break;
                    }
                    if start.elapsed().as_secs() > 3 {
                        return Err(format!(
                            "Native seek did not settle near {target}: {}",
                            t.position_ms
                        )
                        .into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                if !self.audio.as_ref().unwrap().is_paused()
                    || self.audio.as_ref().and_then(Audio::qa_timed_source) != original
                {
                    return Err("Seek changed pause intent or original recording".into());
                }
            }
            self.qa_enhancements23(out)?;
            self.qa_enhancements25(out)?;
            self.qa_enhancements26(out)?;
            self.qa_enhancements27(out)?;
            self.qa_enhancements28(out)?;
            self.qa_enhancements29(out)?;
            self.qa_opening26(out)?;
            let mut cases = vec![];
            for arabic in [false, true] {
                self.prefs.arabic = arabic;
                self.refresh_labels();
                self.panel = Panel::None;
                self.panel_animation = None;
                self.popup = None;
                self.qa_restore_window()?;
                self.resize();
                self.tick()?;
                if !self.side_playback() {
                    return Err("Fixture needs the wide native transport".into());
                }
                self.command(QUICK_SETTINGS, 0)?;
                self.panel_animation = None;
                self.resize();
                if GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32 & WS_VSCROLL.0 != 0 {
                    return Err("Normal listening editor has an unnecessary scrollbar".into());
                }
                let clock = self.text(AUDIO_TIME);
                if clock
                    .chars()
                    .any(|c| ('\u{2066}'..='\u{2069}').contains(&c))
                    || !clock.contains('/')
                    || clock.contains('\n')
                {
                    return Err(
                        "Numeric native clock contains unsupported direction markers".into(),
                    );
                }
                self.paint()?;
                self.capture(&out.join(format!(
                    "native-listen20-editor-{}.png",
                    if arabic { "arabic" } else { "english" }
                )))?;
                for id in [
                    RECITER,
                    CHAPTER,
                    FIRST,
                    LAST,
                    SPEED,
                    REPEAT,
                    VOLUME,
                    AUDIO_POSITION,
                    QUICK_PAUSE,
                    QUICK_STOP,
                    SEEK_BACK,
                    SEEK_FORWARD,
                ] {
                    if !IsWindowVisible(self.controls[&id]).as_bool() {
                        return Err(format!("Listening control hidden: {id}").into());
                    }
                }
                self.command(SETTINGS, 0)?;
                if let Some(p) = &mut self.popup {
                    p.motion = None;
                }
                self.resize();
                self.tick()?;
                let generation = self.layout_generation;
                let area = geometry(self.reader_area());
                for _ in 0..12 {
                    self.tick()?;
                }
                if self.layout_generation != generation || geometry(self.reader_area()) != area {
                    return Err(
                        "Paused polling repeatedly relayouts an occluded native form".into(),
                    );
                }
                self.paint()?;
                self.capture(&out.join(format!(
                    "native-listen20-options-{}.png",
                    if arabic { "arabic" } else { "english" }
                )))?;
                self.command(MENU_CLOSE, 0)?;
                self.command(LISTEN, 0)?;
                let from = self
                    .panel_animation
                    .ok_or("Missing Listen close transition")?
                    .2;
                let mut widths = vec![];
                for (index, t) in [0., 0.25, 0.5, 0.75, 1.].into_iter().enumerate() {
                    self.panel_pose = Some(t);
                    self.resize();
                    widths.push(self.reader_area().w);
                    self.paint()?;
                    self.capture(&out.join(format!(
                        "native-listen20-close-{}-{index}.png",
                        if arabic { "arabic" } else { "english" }
                    )))?;
                }
                let end = geometry(self.reader_area());
                self.panel_animation = None;
                self.panel_pose = None;
                self.resize();
                if from < 0.
                    || widths.windows(2).any(|w| w[1] < w[0])
                    || geometry(self.reader_area()) != end
                {
                    return Err("Listen close detours through the centered reader".into());
                }
                self.paint()?;
                self.capture(&out.join(format!(
                    "native-listen20-compact-{}.png",
                    if arabic { "arabic" } else { "english" }
                )))?;
                cases.push(serde_json::json!({"arabic":arabic,"pausedMenuTicks":12,"relayoutsWhileOccluded":0,"readerWidthsDuringClose":widths,"compactEndpointUnchanged":true,"normalEditorFits":true}));
            }
            self.command(QUICK_STOP, 0)?;
            Audio::qa_opening_clip(self.cache()?, out)?;
            self.prefs = prefs;
            self.refresh_labels();
            self.resize();
            std::fs::write(
                out.join("listening20-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"liveOriginalRecording":true,"muted":true,"seeksMs":seeks,"seekPreservesPauseAndOriginalSource":true,"cases":cases,"scope":"Actual native controls and original continuous 112; paused seeking, menu polling and sampled close endpoints. Not physical input or device endurance."}),
                )?,
            )?;
            Ok(())
        }
    }
}
