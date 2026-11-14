//! Offline main-window integration checks. These invoke the same callbacks as
//! the controls and observe the real native decoder, not a simulated clock.
use crate::*;

pub(crate) struct PlayerQa {
    output: PathBuf,
    began: Instant,
    since: Instant,
    phase: u32,
    geometry: [f32; 8],
    checks: Vec<Value>,
    source_body: String,
    pub(crate) done: bool,
}
impl PlayerQa {
    pub(crate) fn output_directory(&self) -> &Path {
        &self.output
    }
    pub(crate) fn new(output: PathBuf) -> Self {
        Self {
            output,
            began: Instant::now(),
            since: Instant::now(),
            phase: 0,
            geometry: [0.; 8],
            checks: Vec::new(),
            source_body: String::new(),
            done: false,
        }
    }
    fn next(&mut self, phase: u32) {
        self.phase = phase;
        self.since = Instant::now();
    }
    fn mark(&mut self, name: &str) {
        self.checks.push(json!({"name":name,"passed":true}));
    }
    fn capture(&self, ui: &ReaderWindow, name: &str) -> Result<()> {
        let p = ui.window().take_snapshot()?;
        png(&self.output.join(name), p.as_bytes(), p.width(), p.height())
    }
    pub(crate) fn step(
        &mut self,
        ui: &ReaderWindow,
        state: &Rc<RefCell<State>>,
        queue: &Queue,
        controller: &controls::Controller,
    ) -> Result<bool> {
        if self.began.elapsed() > Duration::from_secs(45) {
            return Err(format!(
                "Main player QA timed out at phase {}: {:?}",
                self.phase,
                controller.qa_snapshot()
            )
            .into());
        }
        let observed = controller.qa_snapshot();
        if observed.error {
            return Err(format!("Main player failed: {}", observed.status).into());
        }
        let ready = !ui.get_render_busy() && state.borrow().committed == state.borrow().generation;
        match self.phase {
            0 => {
                ui.window().set_size(slint::LogicalSize::new(1180., 820.));
                ui.set_arabic(true);
                ui.set_theme(0);
                ui.set_show_clock(false);
                ui.set_contextual(false);
                ui.set_reciter_index(0);
                ui.set_volume(0.);
                ui.set_listen_scope(0);
                ui.set_remember_speed(false);
                ui.set_panel("".into());
                set_page(ui, state, queue, 604, None);
                self.next(1);
            }
            1 if ready && self.since.elapsed() > Duration::from_millis(350) => {
                self.geometry = image_geometry(ui);
                state.borrow_mut().context = Some(((112, 3), 604));
                ui.invoke_play_selected();
                self.next(2);
            }
            2 if observed.can_seek && !observed.loading => {
                if observed.duration_ms != 11_416
                    || observed.current != Some((112, 3))
                    || !observed.continuous
                    || !ui.get_audio_active()
                {
                    return Err(format!(
                        "Main player did not expose validated whole-surah timing: {observed:?}"
                    )
                    .into());
                }
                self.mark("Main Play callback starts verified whole-surah recording from ayah 3");
                ui.invoke_audio_toggle();
                self.next(3);
            }
            3 if observed.paused && !observed.loading => {
                ui.invoke_audio_rate(1.25);
                self.next(4);
            }
            4 if observed.paused && self.since.elapsed() > Duration::from_millis(200) => {
                state.borrow_mut().context = Some(((112, 1), 604));
                ui.invoke_study_selected();
                self.next(5);
            }
            5 if ready
                && state.borrow().selection == Some((112, 1))
                && !ui.get_study_body().is_empty()
                && self.since.elapsed() > Duration::from_millis(350) =>
            {
                if ui.get_panel() != "study"
                    || state.borrow().recitation == state.borrow().selection
                {
                    return Err(
                        "Study and paused playback must remain distinct in a reserved column"
                            .into(),
                    );
                }
                self.geometry = image_geometry(ui);
                self.capture(ui, "arabic-study-with-docked-player.png")?;
                self.mark("Arabic study reserves reading space and separates playback from scrollable study text");
                self.source_body = ui.get_study_body().to_string();
                ui.invoke_study_source_changed(1);
                self.next(6);
            }
            6 if ready
                && ui.get_study_source() == 1
                && ui.get_study_body() != self.source_body
                && self.since.elapsed() > Duration::from_millis(350) =>
            {
                if state.borrow().selection != Some((112, 1)) {
                    return Err("Changing tafsir source changed the selected verse".into());
                }
                self.mark("Changing bundled tafsir preserves selected ayah");
                state.borrow_mut().context = Some(((112, 1), 604));
                ui.invoke_bookmark_selected();
                if !state
                    .borrow()
                    .saved_bookmarks
                    .iter()
                    .any(|b| b.key() == (112, 1))
                {
                    return Err("Bookmark callback did not save selected verse".into());
                }
                self.next(7);
            }
            7 if self.since.elapsed() > Duration::from_millis(2600) => {
                if !ui.get_notice().is_empty() || image_geometry(ui) != self.geometry {
                    return Err("Bookmark feedback persisted or moved the Quran".into());
                }
                self.mark("Bookmark feedback expires without page reflow");
                ui.set_panel("".into());
                self.next(8);
            }
            8 if ready && self.since.elapsed() > Duration::from_millis(350) => {
                self.geometry = image_geometry(ui);
                self.capture(ui, "arabic-floating-player.png")?;
                ui.invoke_audio_seek(0.5);
                self.next(9);
            }
            9 if observed.paused
                && observed.can_seek
                && observed.position_ms.abs_diff(5708) < 450 =>
            {
                self.mark(
                    "Main seek callback reaches midpoint of whole-surah timeline while paused",
                );
                ui.invoke_audio_skip(-10);
                self.next(10);
            }
            10 if observed.paused
                && observed.can_seek
                && observed.range_start_ms == 0
                && observed.position_ms < 500 =>
            {
                self.mark(
                    "Back 10 seconds crosses original from-here boundary and preserves pause",
                );
                ui.set_arabic(false);
                ui.invoke_preferences_changed();
                ui.set_panel("listen".into());
                self.next(11);
            }
            11 if ready && self.since.elapsed() > Duration::from_millis(350) => {
                if image_geometry(ui) != self.geometry {
                    return Err("Listening settings resized the Quran".into());
                }
                self.capture(ui, "english-listening-settings.png")?;
                ui.invoke_audio_end();
                self.next(12);
            }
            12 if !observed.active
                && !ui.get_audio_active()
                && !controller.qa_polling()
                && ready =>
            {
                if state.borrow().recitation.is_some() {
                    return Err("End left a recitation highlight".into());
                }
                self.mark("End clears real playback, recitation highlight and polling timer");
                state.borrow_mut().context = Some(((112, 3), 604));
                ui.invoke_play_selected();
                self.next(13);
            }
            13 if observed.can_seek && !observed.loading => {
                if (ui.get_playback_rate() - 1.).abs() > 0.001 {
                    return Err(
                        "New recitation retained live speed despite Remember being off".into(),
                    );
                }
                self.mark("New recitation resets live speed to 1x when Remember is off");
                ui.invoke_audio_end();
                ui.set_panel("".into());
                self.next(14);
            }
            14 if !observed.active && !controller.qa_polling() && ready => {
                ui.set_context_title("Al-Ikhlas · Ayah 1".into());
                ui.set_context_x(ui.get_canvas_w() - 1.);
                ui.set_context_y(ui.get_canvas_h() - 1.);
                ui.set_context_open(true);
                self.next(15);
            }
            15 if self.since.elapsed() > Duration::from_millis(350) => {
                self.capture(ui, "context-clamped-at-canvas-edge.png")?;
                self.mark("Verse context popup captured at bottom-right canvas edge");
                ui.set_context_open(false);
                ui.set_search_query("".into());
                ui.invoke_search_mode_changed(1);
                self.next(16);
            }
            16..=20
                if !ui.get_search_busy() && self.since.elapsed() > Duration::from_millis(200) =>
            {
                let expected = [114, 30, 60, 240, 604][(self.phase - 16) as usize];
                if state.borrow().hits.len() != expected {
                    return Err(format!(
                        "Browse phase {} returned {} rows, expected {expected}",
                        self.phase,
                        state.borrow().hits.len()
                    )
                    .into());
                }
                self.mark(&format!(
                    "Complete browse category: {expected} original entries"
                ));
                if self.phase == 20 {
                    ui.set_search_query("604".into());
                    ui.invoke_search("604".into());
                    self.next(21);
                } else {
                    let mode = [3, 4, 5, 2][(self.phase - 16) as usize];
                    ui.invoke_search_mode_changed(mode);
                    self.next(self.phase + 1);
                }
            }
            21 if !ui.get_search_busy() && self.since.elapsed() > Duration::from_millis(200) => {
                if state.borrow().hits.len() != 1 || state.borrow().hits[0].page != 604 {
                    return Err("Page browse did not reach the final Quran page".into());
                }
                self.mark("Page browse reaches page 604");
                json_file(
                    &self.output.join("player-qa.json"),
                    &json!({"passed":true,"checks":self.checks,"seconds":self.began.elapsed().as_secs_f64(),"scope":"Actual main-window callbacks, native decoder, bundled study and overlay geometry. Not human pointer, accessibility or frame-rate certification."}),
                )?;
                self.done = true;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}
