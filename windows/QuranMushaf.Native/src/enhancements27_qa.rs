//! Native form direction, calmer study controls and independent study/audio state.
use super::*;

impl App {
    pub(super) unsafe fn qa_enhancements27(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let _dispatch_guard = DispatchGuard::enter();
            let prefs = self.prefs.clone();
            let selected = self.selected;
            let source = self.source;
            let arabic_source = self.arabic_source;
            let panel = self.panel;
            let collapsed = self.player_collapsed;
            let fullscreen = self.fullscreen;
            let placement = self.placement;
            let window_style = self.window_style;
            let keyboard = self.keyboard_focus;
            let focus = GetFocus();
            let search = self.search;
            let panel_scroll = self.panel_scroll;
            let tafsir_arabic = self.tafsir_arabic;
            let study_size = self.study_size;
            let opening = self.opening_highlight;
            let hovered = self.hovered;
            let search_reveal = self.search_reveal;
            let dirty = self.dirty;
            let reader_dirty = self.reader_dirty;
            let reader = serde_json::to_vec(&self.reader)?;
            let mut window = RECT::default();
            GetWindowRect(self.hwnd, &mut window)?;
            let mut cursor = POINT::default();
            GetCursorPos(&mut cursor)?;
            let _cursor_restore = CursorRestore27(cursor);
            let saved_playback = std::mem::take(&mut self.playback);
            let saved_popup = self.popup.take();
            let saved_guide = self.guide.take();
            let saved_turn = self.page_turn.take();
            let saved_queue = std::mem::take(&mut self.turn_queue);
            let saved_drag = self.page_drag.take();
            let saved_drag_start = self.drag_start.take();
            let saved_keyboard_turn = self.keyboard_turn;
            let saved_notice_until = self.notice_until.take();
            let saved_notice = self.text(NOTICE);
            let original_recording = self.audio.as_ref().and_then(Audio::qa_timed_source);
            let mut cases = Vec::new();
            let result = (|| -> Result<()> {
                if !self.audio.as_ref().is_some_and(|audio| {
                    audio.is_paused()
                        && audio
                            .transport()
                            .is_some_and(|t| t.continuous && t.can_seek)
                }) {
                    return Err("Study27 requires the original paused, seekable recording".into());
                }
                if self.fullscreen {
                    self.toggle_fullscreen()?;
                }
                self.search = false;
                self.search_animation = None;
                self.title_animation = None;
                self.panel_animation = None;
                self.theme_start = None;
                self.theme_frame = None;
                self.keyboard_turn = false;
                SetWindowPos(
                    self.hwnd,
                    None,
                    0,
                    0,
                    (1536. * self.dpi / 96.) as i32,
                    (864. * self.dpi / 96.) as i32,
                    SWP_NOZORDER,
                )?;
                let _ = SetForegroundWindow(self.hwnd);

                let dual = self
                    .renderer
                    .verify_dual_highlights(&mut self.resources, &out.join("dual-highlights27"))?;
                std::fs::write(
                    out.join("dual-highlights27.json"),
                    serde_json::to_vec_pretty(&dual)?,
                )?;
                std::fs::write(
                    out.join("native-chrome-direction27.json"),
                    serde_json::to_vec_pretty(&self.chrome.verify_label_direction()?)?,
                )?;

                for arabic in [false, true] {
                    for theme in [0, 1, 2] {
                        self.prefs.arabic = arabic;
                        self.prefs.theme = theme;
                        self.prefs.follow_tafsir = false;
                        self.prefs.volume = 1.;
                        self.theme_start = None;
                        self.theme_frame = None;
                        self.replace_background(self.palette());
                        self.player_collapsed = false;
                        self.panel = Panel::Listen;
                        self.panel_scroll = 0.;
                        self.panel_animation = None;
                        self.popup = None;
                        self.refresh_labels();
                        self.resize();
                        self.refresh_transport();
                        let language = if arabic { "arabic" } else { "english" };
                        let stem = format!("native27-{language}-theme-{theme}");
                        let form = self.qa_listening27_geometry()?;
                        let ids = [
                            RECITER,
                            CHAPTER,
                            FIRST,
                            LAST,
                            SPEED,
                            REPEAT,
                            RECORDING_MODE,
                            VOLUME_LABEL,
                        ];
                        let captions: Vec<_> = ids.iter().map(|&id| self.text(id)).collect();
                        let geometry: Vec<_> = ids
                            .iter()
                            .map(|&id| self.placements.borrow()[&id])
                            .collect();
                        let fields_font =
                            SendMessageW(self.controls[&RECITER], WM_GETFONT, None, None).0;
                        let clock = self.text(AUDIO_TIME);
                        for _ in 0..4 {
                            self.tick()?;
                            self.paint()?;
                        }
                        if ids.iter().map(|&id| self.text(id)).collect::<Vec<_>>() != captions
                            || ids.iter().enumerate().any(|(i, &id)| {
                                let (current, shown) = self.placements.borrow()[&id];
                                let (old, old_shown) = geometry[i];
                                !same_rect27(current, old) || shown != old_shown
                            })
                            || SendMessageW(self.controls[&RECITER], WM_GETFONT, None, None).0
                                != fields_font
                            || self.text(AUDIO_TIME) != clock
                        {
                            return Err("Paused listening polls changed stable captions, geometry or native font".into());
                        }
                        self.capture(&out.join(format!("{stem}-listening.png")))?;

                        self.panel = Panel::Study;
                        self.panel_animation = None;
                        self.source = if arabic { 0 } else { 2 };
                        self.selected = Some((2, 282));
                        let independent_page = self
                            .resources
                            .index()?
                            .iter()
                            .find(|hit| (hit.surah, hit.ayah) == (2, 282))
                            .map(|hit| hit.page_number)
                            .ok_or("The original long-source ayah has no page")?;
                        self.prefs.page = independent_page;
                        self.refresh_labels();
                        self.study()?;
                        self.resize();
                        let body = native_rect27(self, STUDY)?;
                        let size = native_rect27(self, TEXT_SIZE)?;
                        if body.y < 174.
                            || size.w > 30.
                            || size.h > 30.
                            || !IsWindowVisible(self.controls[&TEXT_SIZE]).as_bool()
                            || IsWindowVisible(self.controls[&MANAGE_TAFSIR]).as_bool()
                            || !IsWindowVisible(self.controls[&STUDY_OPTIONS]).as_bool()
                            || !IsWindowVisible(self.controls[&STUDY_FOLLOW]).as_bool()
                        {
                            return Err(
                                "Study27 header did not retain its compact, discoverable controls"
                                    .into(),
                            );
                        }
                        let source_combo = self.controls[&SOURCE];
                        let source_rtl = GetWindowLongPtrW(source_combo, GWL_EXSTYLE) as u32
                            & WS_EX_RTLREADING.0
                            != 0;
                        let mut source_info = COMBOBOXINFO {
                            cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                            ..Default::default()
                        };
                        GetComboBoxInfo(source_combo, &mut source_info)?;
                        let source_list_rtl = GetWindowLongPtrW(source_info.hwndList, GWL_EXSTYLE)
                            as u32
                            & WS_EX_RTLREADING.0
                            != 0;
                        if source_rtl != arabic || source_list_rtl != arabic {
                            return Err(
                                "The native Tafsir source dropdown reading direction is incorrect"
                                    .into(),
                            );
                        }

                        // This sequence exercises the UI observer deliberately.
                        // It does not claim the paused decoder advanced verses.
                        let original = self.resources.study(
                            if arabic { "mukhtasar" } else { "sahih" },
                            2,
                            282,
                        )?;
                        if !self.text(STUDY).starts_with(&original) {
                            return Err("Study27 changed the original bundled source text".into());
                        }
                        SendMessageW(
                            self.controls[&STUDY],
                            EM_SCROLL,
                            Some(WPARAM(SB_PAGEDOWN.0 as usize)),
                            None,
                        );
                        SendMessageW(
                            self.controls[&STUDY],
                            EM_SETSEL,
                            Some(WPARAM(5)),
                            Some(LPARAM(14)),
                        );
                        let before = study_state27(self)?;
                        self.playback.begin();
                        self.follow_recitation(Some((112, 1)))?;
                        self.follow_recitation(Some((112, 2)))?;
                        let after = study_state27(self)?;
                        if self.selected != Some((2, 282))
                            || self.prefs.page != independent_page
                            || before != after
                            || self.prefs.follow_tafsir
                        {
                            return Err("Independent study changed its ayah, source, font, selection or scroll with the recitation".into());
                        }
                        self.paint()?;
                        self.capture(&out.join(format!("{stem}-independent-study.png")))?;

                        self.command(STUDY_FOLLOW, 0)?;
                        if !self.prefs.follow_tafsir {
                            return Err("Study27 following could not be enabled through its visible control".into());
                        }
                        self.follow_recitation(Some((112, 1)))?;
                        if self.selected != Some((112, 1)) {
                            return Err(
                                "Study27 following did not join the observed recitation".into()
                            );
                        }
                        let follow_source = self.source;
                        let first = self.resources.study(
                            if arabic { "mukhtasar" } else { "sahih" },
                            112,
                            1,
                        )?;
                        if !self.text(STUDY).starts_with(&first) {
                            return Err(
                                "Study27 following loaded a different original explanation".into(),
                            );
                        }
                        SendMessageW(
                            self.controls[&STUDY],
                            EM_SETSEL,
                            Some(WPARAM(2)),
                            Some(LPARAM(5)),
                        );
                        let first_state = study_state27(self)?;
                        for _ in 0..4 {
                            self.follow_recitation(Some((112, 1)))?;
                        }
                        if first_state != study_state27(self)? {
                            return Err("Repeated observations reloaded or scrolled the current explanation".into());
                        }
                        self.follow_recitation(Some((112, 2)))?;
                        let second = self.resources.study(
                            if arabic { "mukhtasar" } else { "sahih" },
                            112,
                            2,
                        )?;
                        if self.selected != Some((112, 2))
                            || self.source != follow_source
                            || !self.text(STUDY).starts_with(&second)
                        {
                            return Err(
                                "Study27 following failed to load the next source ayah".into()
                            );
                        }

                        // Switching the commentary source is independent of
                        // the choice to follow. Use existing bundled sources.
                        if arabic {
                            let source_row = self
                                .study_sources
                                .iter()
                                .position(|id| *id == 1)
                                .ok_or("The second bundled Arabic source is missing")?;
                            SendMessageW(
                                self.controls[&SOURCE],
                                CB_SETCURSEL,
                                Some(WPARAM(source_row)),
                                None,
                            );
                        }
                        self.command(SOURCE, CBN_SELCHANGE)?;
                        if !self.prefs.follow_tafsir {
                            return Err(
                                "Changing a commentary source silently disabled following".into()
                            );
                        }
                        self.command(AYAH_NEXT, 0)?;
                        if self.prefs.follow_tafsir || self.selected != Some((112, 3)) {
                            return Err(
                                "Manual ayah navigation did not leave automatic following".into()
                            );
                        }
                        self.follow_recitation(Some((112, 4)))?;
                        if self.selected != Some((112, 3)) {
                            return Err(
                                "Recitation observations undid deliberate ayah navigation".into()
                            );
                        }

                        // Return to the actual paused native observation and
                        // prove polling cannot reset an independently read text.
                        let paused_key = self.audio.as_ref().and_then(Audio::current);
                        self.follow_recitation(paused_key)?;
                        SendMessageW(
                            self.controls[&STUDY],
                            EM_SETSEL,
                            Some(WPARAM(2)),
                            Some(LPARAM(5)),
                        );
                        let paused_state = study_state27(self)?;
                        for _ in 0..4 {
                            self.tick()?;
                            self.paint()?;
                        }
                        if paused_state != study_state27(self)? {
                            return Err("Paused native polling altered the independently selected explanation".into());
                        }
                        self.capture(&out.join(format!("{stem}-manual-study.png")))?;

                        self.open_tafsirs()?;
                        self.panel_animation = None;
                        self.resize();
                        let expected_arabic = self.tr("Arabic tafsirs", "تفاسير عربية");
                        let expected_english = self.tr("English tafsirs", "تفاسير إنجليزية");
                        if self.text(TAFSIR_ARABIC) != expected_arabic
                            || self.text(TAFSIR_ENGLISH) != expected_english
                        {
                            return Err("The tafsir language categories lost their natural localized captions".into());
                        }
                        let mut manager = Vec::new();
                        for (category, is_arabic) in
                            [(TAFSIR_ARABIC, true), (TAFSIR_ENGLISH, false)]
                        {
                            self.command(category, 0)?;
                            self.panel_animation = None;
                            self.resize();
                            let paragraph = manager_direction27(self)?;
                            let rows = self.tafsir_rows();
                            if rows.is_empty()
                                || rows
                                    .iter()
                                    .any(|&id| tafsir_library::SOURCES[id].arabic != is_arabic)
                            {
                                return Err(
                                    "The manager mixed Arabic and English tafsir categories".into(),
                                );
                            }
                            for &id in &rows {
                                let original = &tafsir_library::SOURCES[id];
                                let name = if arabic {
                                    original.name_arabic
                                } else {
                                    original.name_english
                                };
                                let action = TAFSIR_ACTION_FIRST + id as u32;
                                if !IsWindowVisible(self.controls[&action]).as_bool()
                                    || !self.text(action).ends_with(name)
                                {
                                    return Err(
                                        "A manager action lost the exact original source name"
                                            .into(),
                                    );
                                }
                                if !self.installed_tafsirs.contains(&id)
                                    && original.download_issue().is_some()
                                    && IsWindowEnabled(self.controls[&action]).as_bool()
                                {
                                    return Err("A source that failed integrity validation became downloadable".into());
                                }
                            }
                            self.paint()?;
                            self.capture(&out.join(format!(
                                "{stem}-manager-{}.png",
                                if is_arabic { "arabic" } else { "english" }
                            )))?;
                            manager.push(serde_json::json!({"arabicSources":is_arabic,"rows":rows,"nativeParagraph":paragraph,"originalSourceNamesPreserved":true,"validationGuardsPreserved":true}));
                        }
                        if !self.audio.as_ref().is_some_and(Audio::is_paused)
                            || self.audio.as_ref().and_then(Audio::qa_timed_source)
                                != original_recording
                        {
                            return Err(
                                "Study27 UI checks changed the paused original recording".into()
                            );
                        }
                        cases.push(serde_json::json!({
                            "arabic":arabic,"theme":theme,"form":form,
                            "stablePausedForm":true,"compactStudyHeader":true,
                            "studySourceDirection":{"rtl":source_rtl,"listRtl":source_list_rtl},
                            "independentStudyRetainsSourceFontSelectionAndScroll":true,
                            "independentStudyPagePreserved":independent_page,
                            "followToggleJoinsObservedAyah":true,"unchangedObservationDoesNotReload":true,
                            "sourceChangeRetainsFollowMode":true,"manualNavigationDisablesFollowing":true,
                            "pausedNativePollingPreservesIndependentStudy":true,"manager":manager,
                            "controlledUiObserverSequence":[[112,1],[112,2],[112,1],[112,2],[112,4]],
                            "actualPausedDecoderAyah":paused_key,
                        }));
                    }
                }
                Ok(())
            })();

            let restore = (|| -> Result<()> {
                if self.fullscreen {
                    self.toggle_fullscreen()?;
                }
                self.prefs = prefs;
                self.selected = selected;
                self.source = source;
                self.arabic_source = arabic_source;
                self.panel = panel;
                self.player_collapsed = collapsed;
                self.keyboard_focus = keyboard;
                self.search = search;
                self.panel_scroll = panel_scroll;
                self.tafsir_arabic = tafsir_arabic;
                self.study_size = study_size;
                self.playback = saved_playback;
                self.popup = saved_popup;
                self.guide = saved_guide;
                self.page_turn = saved_turn;
                self.turn_queue = saved_queue;
                self.page_drag = saved_drag;
                self.drag_start = saved_drag_start;
                self.keyboard_turn = saved_keyboard_turn;
                self.opening_highlight = opening;
                self.hovered = hovered;
                self.search_reveal = search_reveal;
                self.search_animation = None;
                self.title_animation = None;
                self.panel_animation = None;
                self.theme_start = None;
                self.theme_frame = None;
                self.notice_until = saved_notice_until;
                self.set_text(NOTICE, &saved_notice);
                self.reader = serde_json::from_slice(&reader)?;
                SetWindowPos(
                    self.hwnd,
                    None,
                    window.left,
                    window.top,
                    window.right - window.left,
                    window.bottom - window.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )?;
                if fullscreen {
                    self.toggle_fullscreen()?;
                }
                self.placement = placement;
                self.window_style = window_style;
                self.replace_background(self.palette());
                self.refresh_labels();
                self.study()?;
                self.resize();
                self.refresh_transport();
                self.dirty = dirty;
                self.reader_dirty = reader_dirty;
                if IsWindow(Some(focus)).as_bool() && IsWindowVisible(focus).as_bool() {
                    focus_window(focus)?;
                } else {
                    focus_window(self.hwnd)?;
                }
                Ok(())
            })();
            result?;
            restore?;
            std::fs::write(
                out.join("enhancements27.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "passed":true,"cases":cases,
                    "dualHighlightEvidence":"dual-highlights27.json",
                    "nativeChromeDirectionEvidence":"native-chrome-direction27.json",
                    "scope":"Actual HWND form geometry, localized volume text metrics, native RichEdit paragraph direction/selection/scroll and UI observer behavior. The real original recording remains paused; explicit observer events test following policy and are not claims about decoder verse timestamps. Client-area screenshots cover both interface languages and three themes. No displayed-FPS claim."
                }))?,
            )?;
            Ok(())
        }
    }
}

#[derive(PartialEq, Eq)]
struct StudyState27 {
    source: usize,
    font: isize,
    text: String,
    selection: (i32, i32),
    scroll: (i32, i32),
}

unsafe fn study_state27(app: &App) -> Result<StudyState27> {
    unsafe {
        let hwnd = app.controls[&STUDY];
        let mut selection = CHARRANGE::default();
        SendMessageW(
            hwnd,
            EM_EXGETSEL,
            None,
            Some(LPARAM((&mut selection as *mut CHARRANGE) as isize)),
        );
        let mut scroll = POINT::default();
        SendMessageW(
            hwnd,
            EM_GETSCROLLPOS,
            None,
            Some(LPARAM((&mut scroll as *mut POINT) as isize)),
        );
        Ok(StudyState27 {
            source: app.source,
            font: SendMessageW(hwnd, WM_GETFONT, None, None).0,
            text: app.text(STUDY),
            selection: (selection.cpMin, selection.cpMax),
            scroll: (scroll.x, scroll.y),
        })
    }
}

unsafe fn manager_direction27(app: &App) -> Result<serde_json::Value> {
    unsafe {
        let hwnd = app.controls[&TAFSIR_INFO];
        let mut selection = CHARRANGE::default();
        SendMessageW(
            hwnd,
            EM_EXGETSEL,
            None,
            Some(LPARAM((&mut selection as *mut CHARRANGE) as isize)),
        );
        SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(0)));
        let mut paragraph = PARAFORMAT2::default();
        paragraph.Base.cbSize = std::mem::size_of::<PARAFORMAT2>() as u32;
        SendMessageW(
            hwnd,
            EM_GETPARAFORMAT,
            None,
            Some(LPARAM((&mut paragraph as *mut PARAFORMAT2) as isize)),
        );
        SendMessageW(
            hwnd,
            EM_EXSETSEL,
            None,
            Some(LPARAM((&selection as *const CHARRANGE) as isize)),
        );
        let rtl = paragraph.Base.Anonymous.wEffects & 1 != 0;
        if (paragraph.Base.dwMask & PFM_RTLPARA).0 == 0
            || rtl != app.prefs.arabic
            || paragraph.Base.wAlignment
                != if app.prefs.arabic {
                    PFA_RIGHT
                } else {
                    PFA_LEFT
                }
        {
            return Err(
                "The manager's actual RichEdit paragraph reading direction is incorrect".into(),
            );
        }
        Ok(
            serde_json::json!({"rtl":rtl,"alignment":paragraph.Base.wAlignment.0,"nativeDirectionMask":paragraph.Base.dwMask.0}),
        )
    }
}

unsafe fn native_rect27(app: &App, id: u32) -> Result<Rect> {
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(app.controls[&id], &mut rect)?;
        let mut origin = POINT {
            x: rect.left,
            y: rect.top,
        };
        ScreenToClient(app.hwnd, &mut origin).ok()?;
        let scale = 96. / app.dpi;
        Ok(Rect {
            x: origin.x as f32 * scale,
            y: origin.y as f32 * scale,
            w: (rect.right - rect.left) as f32 * scale,
            h: (rect.bottom - rect.top) as f32 * scale,
        })
    }
}

fn same_rect27(first: Rect, second: Rect) -> bool {
    (first.x - second.x).abs() < 1.
        && (first.y - second.y).abs() < 1.
        && (first.w - second.w).abs() < 1.
        && (first.h - second.h).abs() < 1.
}

struct CursorRestore27(POINT);
impl Drop for CursorRestore27 {
    fn drop(&mut self) {
        unsafe {
            let _ = SetCursorPos(self.0.x, self.0.y);
        }
    }
}
