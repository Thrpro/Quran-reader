//! Regression checks for the clock, fullscreen margins, small study header and language catalogue.
use super::*;
impl App {
    pub(super) unsafe fn qa_enhancements25(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let prefs = self.prefs.clone();
            let selected = self.selected;
            let source = self.source;
            let mut window = RECT::default();
            GetWindowRect(self.hwnd, &mut window)?;
            let mut cases = vec![];
            let result = (|| -> Result<()> {
                if !self
                    .audio
                    .as_ref()
                    .and_then(Audio::transport)
                    .is_some_and(|t| t.continuous && t.can_seek)
                {
                    return Err("Player25 requires the original seekable recording".into());
                }
                if !self.audio.as_ref().unwrap().is_paused() {
                    self.command(QUICK_PAUSE, 0)?;
                }
                for arabic in [false, true] {
                    self.prefs.arabic = arabic;
                    self.panel = Panel::None;
                    self.panel_animation = None;
                    self.popup = None;
                    self.search = false;
                    self.player_collapsed = false;
                    self.refresh_labels();
                    SetWindowPos(
                        self.hwnd,
                        None,
                        0,
                        0,
                        (1536. * self.dpi / 96.) as i32,
                        (864. * self.dpi / 96.) as i32,
                        SWP_NOMOVE | SWP_NOZORDER,
                    )?;
                    self.resize();
                    self.refresh_transport();
                    let clock = self.text(AUDIO_TIME);
                    let rect = self.placements.borrow()[&AUDIO_TIME].0;
                    let status = self.placements.borrow()[&STATUS].0;
                    if !clock.contains('/')
                        || clock.contains('\n')
                        || self.text(STATUS).contains('\n')
                        || status.y + status.h > rect.y + 1.
                    {
                        return Err("Player25 clock/caption geometry is unstable".into());
                    }
                    let lang = if arabic { "arabic" } else { "english" };
                    for frame in 0..5 {
                        self.tick()?;
                        self.invalidate_reader();
                        self.paint()?;
                        if !self.transport_fields_in_frame || self.text(AUDIO_TIME) != clock {
                            return Err(
                                "Player25 clock was cleared or omitted from the shared present"
                                    .into(),
                            );
                        }
                        self.capture(
                            &out.join(format!("native-player25-stable-{lang}-{frame}.png")),
                        )?;
                    }
                    self.command(FULLSCREEN, 0)?;
                    self.panel_animation = None;
                    self.resize();
                    self.refresh_transport();
                    if self.width / self.height > 1.7 && self.compact_player_frame().is_none() {
                        return Err("Fullscreen unnecessarily collapsed the margin player".into());
                    }
                    if self.text(FULLSCREEN).is_empty()
                        || self.placements.borrow()[&FULLSCREEN].0.w < 58.
                    {
                        return Err("Fullscreen exit lost its explicit caption".into());
                    }
                    self.paint()?;
                    self.capture(&out.join(format!("native-player25-fullscreen-{lang}.png")))?;
                    self.selected = Some((46, 21));
                    self.source = if arabic { 1 } else { 2 };
                    self.command(STUDY, 0)?;
                    self.panel_animation = None;
                    self.resize();
                    let full = crate::app_core::book_geometry(self.reader_area()).bounds;
                    self.fullscreen = false;
                    let normal = crate::app_core::book_geometry(self.reader_area()).bounds;
                    self.fullscreen = true;
                    if full.h < normal.h * 0.9 {
                        return Err(
                            "Fullscreen study shrank pages substantially more than windowed study"
                                .into(),
                        );
                    }
                    let body = self.placements.borrow()[&STUDY].0;
                    let footer = self.placements.borrow()[&STATUS].0;
                    if body.y > 184.
                        || body.y + body.h + 4. > footer.y
                        || IsWindowVisible(self.controls[&TEXT_SMALL]).as_bool()
                        || !IsWindowVisible(self.controls[&TEXT_SIZE]).as_bool()
                    {
                        return Err("Study25 retained a bulky or overlapping header/footer".into());
                    }
                    if !arabic
                        && (IsWindowVisible(self.controls[&SOURCE]).as_bool()
                            || !IsWindowVisible(self.controls[&SOURCE_SINGLE]).as_bool())
                    {
                        return Err("The sole English source still opens an empty selector".into());
                    }
                    self.command(TEXT_SIZE, 0)?;
                    self.popup_command(MENU_ROW + 2)?;
                    if self.study_size != 18 {
                        return Err("Study size chooser did not apply the selected size".into());
                    }
                    self.paint()?;
                    self.capture(&out.join(format!("native-study25-fullscreen-{lang}.png")))?;
                    self.command(MANAGE_TAFSIR, 0)?;
                    self.panel_animation = None;
                    for group in [TAFSIR_ENGLISH, TAFSIR_ARABIC] {
                        self.command(group, 0)?;
                        let rows = self.tafsir_rows();
                        if rows.len() != if group == TAFSIR_ENGLISH { 3 } else { 4 }
                            || rows.iter().any(|id| {
                                tafsir_library::SOURCES[*id].arabic != (group == TAFSIR_ARABIC)
                            })
                        {
                            return Err(
                                "Tafsir language categories mix languages or omit available books"
                                    .into(),
                            );
                        }
                        for id in rows {
                            if !IsWindowVisible(self.controls[&(TAFSIR_ACTION_FIRST + id as u32)])
                                .as_bool()
                            {
                                return Err("A grouped tafsir action is inaccessible".into());
                            }
                        }
                        self.paint()?;
                        self.capture(&out.join(format!("native-tafsirs25-{lang}-{group}.png")))?;
                    }
                    self.command(CLOSE, 0)?;
                    self.panel_animation = None;
                    self.command(FULLSCREEN, 0)?;
                    self.panel_animation = None;
                    self.resize();
                    self.options()?;
                    let mut stack = self.popup.as_ref().unwrap().rows.clone();
                    while let Some(row) = stack.pop() {
                        if row.id == OTHER_STUDY_LANGUAGES {
                            return Err("Removed extra-language preference is still exposed".into());
                        }
                        stack.extend(row.children);
                    }
                    self.popup = None;
                    self.resize();
                    cases.push(serde_json::json!({"arabic":arabic,"clock":clock,"clockRect":[rect.x,rect.y,rect.w,rect.h],"dpi":self.dpi,"stablePresents":5,"sharedTransportPresent":true,"fullscreenFloatingPlayer":true,"explicitFullscreenExit":true,"fullscreenStudyHeight":full.h,"windowedStudyHeightAtSameSize":normal.h,"studyHeaderHeight":body.y,"nonoverlappingFooter":true,"singleSourceLabel":!arabic,"sizeChooser":true,"englishTafsirs":3,"arabicTafsirs":4,"obsoleteOptionRemoved":true}));
                }
                self.panel = Panel::None;
                self.page_turn = None;
                self.turn_queue.clear();
                self.prefs.page = 41;
                for _ in 0..12 {
                    self.request_turn(1, crate::app_core::TurnSource::Keyboard)?;
                    if !self.page_turn.is_some_and(|t| t.first_frame) {
                        return Err(
                            "Cold navigation started its clock before its first frame".into()
                        );
                    }
                    self.paint()?;
                    if self
                        .page_turn
                        .is_none_or(|t| t.first_frame || t.start.elapsed().as_millis() > 100)
                    {
                        return Err("Page preparation consumed the visible turn".into());
                    }
                    let turn = self.page_turn.as_mut().unwrap();
                    let start = turn.start;
                    let from = turn.from;
                    turn.release();
                    if turn.start != start || turn.from != from {
                        return Err("Key release restarted the visible page-turn curve".into());
                    }
                    turn.first_frame = false;
                    turn.start = Instant::now() - std::time::Duration::from_millis(150);
                    let (progress, done) = turn.progress();
                    if done || !(0.49..=0.52).contains(&progress) {
                        return Err(
                            "A released turn accelerated past its continuous midpoint".into()
                        );
                    }
                    turn.start = Instant::now() - std::time::Duration::from_millis(301);
                    self.invalidate_reader();
                    self.paint()?;
                    if self.page_turn.is_some() {
                        return Err(
                            "Released keyboard turn did not finish its original bounded curve"
                                .into(),
                        );
                    }
                }
                std::fs::write(
                    out.join("enhancements25.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"passed":true,"cases":cases,"coldTurnsChecked":12,"visibleFullTurnMs":300,"keyReleasePreservesPhaseAndVelocity":true,"scope":"Native controls and shared-frame composition with original paused Al-Ikhlas recording; programmed key release/turn poses, not physical-key endurance or displayed FPS."}),
                    )?,
                )?;
                Ok(())
            })();
            if self.fullscreen {
                self.command(FULLSCREEN, 0)?;
            }
            self.prefs = prefs;
            self.selected = selected;
            self.source = source;
            self.popup = None;
            self.panel = Panel::None;
            self.panel_animation = None;
            self.page_turn = None;
            self.keyboard_turn = false;
            SetWindowPos(
                self.hwnd,
                None,
                window.left,
                window.top,
                window.right - window.left,
                window.bottom - window.top,
                SWP_NOZORDER,
            )?;
            self.refresh_labels();
            self.resize();
            result
        }
    }
}
