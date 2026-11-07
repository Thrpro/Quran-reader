//! Readback of real native field geometry, keyboard behavior and source direction.
use super::*;
use windows::Win32::UI::Controls::{EM_GETMARGINS, EM_GETSEL, EM_REPLACESEL};

impl App {
    pub(super) unsafe fn qa_enhancements29(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let _dispatch = DispatchGuard::enter();
            let prefs = self.prefs.clone();
            let selected = self.selected;
            let source = self.source;
            let arabic_source = self.arabic_source;
            let panel = self.panel;
            let panel_scroll = self.panel_scroll;
            let fullscreen = self.fullscreen;
            let placement = self.placement;
            let window_style = self.window_style;
            let keyboard = self.keyboard_focus;
            let focus = GetFocus();
            let search = self.search;
            let text_view = self.text_view;
            let browse_tab = self.browse_tab;
            let browse_kind = self.select(BROWSE_KIND);
            let collapsed = self.player_collapsed;
            let study_size = self.study_size;
            let tafsir_arabic = self.tafsir_arabic;
            let opening = self.opening_highlight;
            let hovered = self.hovered;
            let search_reveal = self.search_reveal;
            let last_input = self.last_input;
            let dirty = self.dirty;
            let reader_dirty = self.reader_dirty;
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
            let saved_edits: Vec<_> = [QUERY, BROWSE_QUERY, FIRST, LAST]
                .into_iter()
                .map(|id| (id, edit_state29(self, id)))
                .collect();
            let saved_study = rich_state29(self, STUDY);
            let mut window = RECT::default();
            GetWindowRect(self.hwnd, &mut window)?;
            let mut cases = Vec::new();
            let result = (|| -> Result<()> {
                self.search = false;
                self.text_view = false;
                self.search_animation = None;
                self.title_animation = None;
                self.panel_animation = None;
                self.study_reveal_pending = false;
                self.theme_start = None;
                self.theme_frame = None;
                self.keyboard_turn = false;
                self.prefs.follow_tafsir = false;
                self.player_collapsed = true;
                // Fullscreen removes user-facing minimum tracking constraints;
                // SetWindowPos below exercises actual 360/400/540-DIP HWNDs.
                if !self.fullscreen {
                    self.toggle_fullscreen()?;
                }
                let _ = SetForegroundWindow(self.hwnd);
                for arabic in [false, true] {
                    for theme in [0, 1, 2] {
                        for height in [540., 400., 360.] {
                            self.prefs.arabic = arabic;
                            self.prefs.theme = theme;
                            self.theme_start = None;
                            self.theme_frame = None;
                            self.replace_background(self.palette());
                            self.refresh_labels();
                            self.panel = Panel::None;
                            self.panel_scroll = 0.;
                            self.search = true;
                            self.browse_tab = BROWSE_SECTIONS;
                            self.set_text(QUERY, "");
                            self.set_text(BROWSE_QUERY, "1");
                            SendMessageW(
                                self.controls[&BROWSE_KIND],
                                CB_SETCURSEL,
                                Some(WPARAM(0)),
                                None,
                            );
                            self.update_search()?;
                            SetWindowPos(
                                self.hwnd,
                                None,
                                0,
                                0,
                                (1100. * self.dpi / 96.).round() as i32,
                                (height * self.dpi / 96.).round() as i32,
                                SWP_NOZORDER,
                            )?;
                            self.resize();
                            if (self.height - height).abs() > 1. {
                                return Err(
                                    "Search29 did not exercise its requested client height".into(),
                                );
                            }
                            let metrics = crate::field_style::Metrics::new(self.dpi);
                            let margin = metrics.horizontal_padding();
                            let mut margins = Vec::new();
                            for id in [QUERY, BROWSE_QUERY] {
                                let packed =
                                    SendMessageW(self.controls[&id], EM_GETMARGINS, None, None).0
                                        as u32;
                                let (left, right) =
                                    ((packed & 65535) as i32, (packed >> 16) as i32);
                                if left != margin || right != margin {
                                    return Err(format!("Field29 {id} has inconsistent DPI-scaled margins: {left}/{right}, expected {margin}").into());
                                }
                                margins.push(
                                    serde_json::json!({"id":id,"leftPx":left,"rightPx":right}),
                                );
                            }
                            let query = native_rect29(self, QUERY)?;
                            let filter = native_rect29(self, BROWSE_QUERY)?;
                            let results = native_rect29(self, RESULTS)?;
                            if (query.h - 24.).abs() > 1.
                                || (filter.h - 24.).abs() > 1.
                                || filter.y <= query.y + query.h + 32.
                                || results.y < filter.y + filter.h + 8.
                                || results.y + results.h > self.height - 15.
                                || results.h < 1.
                                || filter.w < 200.
                            {
                                return Err(
                                    "Compact Search29 has clipped or crowded native fields/results"
                                        .into(),
                                );
                            }
                            let combo = self.controls[&BROWSE_KIND];
                            let item_height = SendMessageW(
                                combo,
                                CB_GETITEMHEIGHT,
                                Some(WPARAM(usize::MAX)),
                                None,
                            )
                            .0;
                            if item_height != metrics.combo_selection_height() as isize {
                                return Err(
                                    format!("Search29 native combo height {item_height} did not track DPI {} (expected {})", self.dpi, metrics.combo_selection_height()).into()
                                );
                            }
                            let mut info = COMBOBOXINFO {
                                cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                                ..Default::default()
                            };
                            GetComboBoxInfo(combo, &mut info)?;
                            let combo_rtl = GetWindowLongPtrW(combo, GWL_EXSTYLE) as u32
                                & WS_EX_RTLREADING.0
                                != 0;
                            let list_rtl = GetWindowLongPtrW(info.hwndList, GWL_EXSTYLE) as u32
                                & WS_EX_RTLREADING.0
                                != 0;
                            if combo_rtl != arabic || list_rtl != arabic {
                                return Err("Search29 combo/list reading direction differs from the interface".into());
                            }
                            self.keyboard_focus = true;
                            focus_window(self.controls[&QUERY])?;
                            let before_query = edit_state29(self, QUERY);
                            let tab = MSG {
                                hwnd: self.controls[&QUERY],
                                message: WM_KEYDOWN,
                                wParam: WPARAM(VK_TAB.0 as usize),
                                lParam: LPARAM(1),
                                ..Default::default()
                            };
                            if !IsDialogMessageW(self.hwnd, &tab).as_bool()
                                || GetFocus() == self.controls[&QUERY]
                                || GetFocus() == self.hwnd
                                || !IsChild(self.hwnd, GetFocus()).as_bool()
                                || !IsWindowVisible(GetFocus()).as_bool()
                                || edit_state29(self, QUERY) != before_query
                            {
                                return Err("Search29 native Tab lost focus or changed edit contents/selection/font".into());
                            }
                            let stem = format!(
                                "native29-{}-theme-{theme}-h{}",
                                if arabic { "arabic" } else { "english" },
                                height as u32
                            );
                            self.paint()?;
                            self.capture(&out.join(format!("{stem}-search.png")))?;

                            self.search = false;
                            self.panel = Panel::Listen;
                            self.panel_scroll = 0.;
                            self.resize();
                            let mut numeric = Vec::new();
                            for id in [FIRST, LAST] {
                                let bounds = native_rect29(self, id)?;
                                if (bounds.w - 64.).abs() > 1.
                                    || (bounds.h - 24.).abs() > 1.
                                    || GetWindowLongPtrW(self.controls[&id], GWL_STYLE) as u32
                                        & WS_BORDER.0
                                        != 0
                                {
                                    return Err("Listening29 numeric edits retained a classic frame or lost compact text bounds".into());
                                }
                                numeric.push(serde_json::json!({"id":id,"boundsDip":[bounds.x,bounds.y,bounds.w,bounds.h]}));
                            }
                            self.set_text(FIRST, "12");
                            focus_window(self.controls[&FIRST])?;
                            SendMessageW(
                                self.controls[&FIRST],
                                EM_SETSEL,
                                Some(WPARAM(1)),
                                Some(LPARAM(1)),
                            );
                            SendMessageW(
                                self.controls[&FIRST],
                                WM_CHAR,
                                Some(WPARAM('3' as usize)),
                                Some(LPARAM(1)),
                            );
                            let numeric_edit = edit_state29(self, FIRST);
                            if numeric_edit.text != "132" {
                                return Err(
                                    "Listening29 lost native numeric edit/caret behavior".into()
                                );
                            }
                            self.resize();
                            self.paint()?;
                            if edit_state29(self, FIRST) != numeric_edit {
                                return Err("Listening29 layout changed native edit text, selection or font".into());
                            }

                            self.panel = Panel::Study;
                            self.selected = Some((112, 1));
                            // Load the authentic English source in its supported
                            // interface mode. This native fixture deliberately
                            // exercises independent body/attribution paragraph
                            // direction; it does not expose a cross-language UI
                            // source option or change the production source policy.
                            self.prefs.arabic = false;
                            self.source = 2;
                            self.prefs.follow_tafsir = false;
                            self.study()?;
                            self.prefs.arabic = arabic;
                            let attribution_start = self.study_attribution_start;
                            let localized_attribution = self
                                .tr(
                                    "Source: Quranic Universal Library",
                                    "المصدر: المكتبة القرآنية الشاملة",
                                )
                                .to_owned();
                            self.set_text(STUDY_ATTRIBUTION, &localized_attribution);
                            {
                                let study = self.controls[&STUDY];
                                let _redraw = crate::reading_control::RedrawBatch::begin(study);
                                SendMessageW(
                                    study,
                                    EM_SETSEL,
                                    Some(WPARAM(attribution_start as usize)),
                                    Some(LPARAM(-1)),
                                );
                                let attribution = HSTRING::from(localized_attribution.as_str());
                                SendMessageW(
                                    study,
                                    EM_REPLACESEL,
                                    Some(WPARAM(0)),
                                    Some(LPARAM(attribution.as_ptr() as isize)),
                                );
                                self.format_study_attribution(self.palette());
                                SendMessageW(study, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(0)));
                            }
                            self.resize();
                            let original = self.resources.study("sahih", 112, 1)?;
                            if !self.text(STUDY).starts_with(&original)
                                || !self.text(STUDY).ends_with(&localized_attribution)
                                || self.study_attribution_start != attribution_start
                                || self.study_attribution_start == 0
                            {
                                return Err("Study29 changed or omitted its original English source/attribution".into());
                            }
                            let body = paragraph29(self.controls[&STUDY], 0);
                            let attribution = paragraph29(
                                self.controls[&STUDY],
                                self.study_attribution_start as i32,
                            );
                            check_paragraph29(&body, false)?;
                            check_paragraph29(&attribution, arabic)?;
                            SendMessageW(
                                self.controls[&STUDY],
                                EM_SETSEL,
                                Some(WPARAM(2)),
                                Some(LPARAM(7)),
                            );
                            let stable_study = rich_state29(self, STUDY);
                            for _ in 0..3 {
                                self.resize();
                                self.paint()?;
                            }
                            if rich_state29(self, STUDY) != stable_study {
                                return Err("Study29 repeated layout changed native text, format rectangle, font, selection or scroll".into());
                            }
                            self.capture(&out.join(format!("{stem}-study.png")))?;
                            if self.audio.as_ref().and_then(Audio::qa_timed_source)
                                != original_recording
                            {
                                return Err(
                                    "Field29 UI verification changed the original recording".into(),
                                );
                            }
                            cases.push(serde_json::json!({
                                "arabic":arabic,"theme":theme,"clientHeightDip":self.height,
                                "margins":margins,"comboSelectionHeightPx":item_height,
                                "comboRtl":combo_rtl,"listRtl":list_rtl,
                                "queryDip":[query.x,query.y,query.w,query.h],
                                "filterDip":[filter.x,filter.y,filter.w,filter.h],
                                "resultsDip":[results.x,results.y,results.w,results.h],
                                "nativeTabPreservesEdit":true,"numericEdits":numeric,
                                "nativeNumericInputPreserved":true,"englishSourceBodyRtl":false,
                                "attributionRtl":arabic,"repeatedStudyLayoutStable":true,
                                "independentParagraphNativeFixture":true,
                                "originalTextAndRecordingPreserved":true
                            }));
                        }
                    }
                }
                Ok(())
            })();
            // Restore the fixture even if a readback assertion fails.
            let restore = (|| -> Result<()> {
                if self.fullscreen {
                    self.toggle_fullscreen()?;
                }
                self.prefs = prefs;
                self.selected = selected;
                self.source = source;
                self.arabic_source = arabic_source;
                self.panel = panel;
                self.panel_scroll = panel_scroll;
                self.keyboard_focus = keyboard;
                self.search = search;
                self.text_view = text_view;
                self.browse_tab = browse_tab;
                self.player_collapsed = collapsed;
                self.study_size = study_size;
                self.tafsir_arabic = tafsir_arabic;
                self.opening_highlight = opening;
                self.hovered = hovered;
                self.search_reveal = search_reveal;
                self.playback = saved_playback;
                self.popup = saved_popup;
                self.guide = saved_guide;
                self.page_turn = saved_turn;
                self.turn_queue = saved_queue;
                self.page_drag = saved_drag;
                self.drag_start = saved_drag_start;
                self.keyboard_turn = saved_keyboard_turn;
                self.search_animation = None;
                self.title_animation = None;
                self.panel_animation = None;
                self.study_reveal_pending = false;
                self.theme_start = None;
                self.theme_frame = None;
                self.notice_until = saved_notice_until;
                self.set_text(NOTICE, &saved_notice);
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
                for (id, edit) in &saved_edits {
                    self.set_text(*id, &edit.text);
                    SendMessageW(
                        self.controls[id],
                        EM_SETSEL,
                        Some(WPARAM(edit.selection.0 as usize)),
                        Some(LPARAM(edit.selection.1 as isize)),
                    );
                }
                SendMessageW(
                    self.controls[&BROWSE_KIND],
                    CB_SETCURSEL,
                    Some(WPARAM(browse_kind)),
                    None,
                );
                self.update_search()?;
                self.study()?;
                self.resize();
                restore_rich_state29(self.controls[&STUDY], &saved_study);
                self.refresh_transport();
                self.last_input = last_input;
                self.dirty = dirty;
                self.reader_dirty = reader_dirty;
                focus_window(
                    if IsWindow(Some(focus)).as_bool() && IsWindowVisible(focus).as_bool() {
                        focus
                    } else {
                        self.hwnd
                    },
                )?;
                Ok(())
            })();
            result?;
            restore?;
            std::fs::write(
                out.join("enhancements29.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "passed":true,"cases":cases,
                    "scope":"Eighteen actual HWND cases: both interface languages, three themes and 360/400/540-DIP client heights. Native edit margins, combo height/direction, dialog Tab, numeric typing and repeated-layout state readback; client PNG captures. Paragraph independence uses the original English source loaded in supported English mode, with only its attribution tail relocalized in a native fixture. It does not assert cross-language source exposure. No displayed-FPS claim or Quran resource modification."
                }))?,
            )?;
            Ok(())
        }
    }
}

#[derive(PartialEq, Eq)]
struct EditState29 {
    text: String,
    font: isize,
    selection: (u32, u32),
}
unsafe fn edit_state29(app: &App, id: u32) -> EditState29 {
    unsafe {
        let hwnd = app.controls[&id];
        let mut start = 0u32;
        let mut end = 0u32;
        SendMessageW(
            hwnd,
            EM_GETSEL,
            Some(WPARAM((&mut start as *mut u32) as usize)),
            Some(LPARAM((&mut end as *mut u32) as isize)),
        );
        EditState29 {
            text: app.text(id),
            font: SendMessageW(hwnd, WM_GETFONT, None, None).0,
            selection: (start, end),
        }
    }
}
#[derive(PartialEq, Eq)]
struct RichState29 {
    text: String,
    font: isize,
    selection: (i32, i32),
    scroll: (i32, i32),
    format: (i32, i32, i32, i32),
}
unsafe fn rich_state29(app: &App, id: u32) -> RichState29 {
    unsafe {
        let hwnd = app.controls[&id];
        let mut selection = CHARRANGE::default();
        let mut scroll = POINT::default();
        let mut rect = RECT::default();
        SendMessageW(
            hwnd,
            EM_EXGETSEL,
            None,
            Some(LPARAM((&mut selection as *mut CHARRANGE) as isize)),
        );
        SendMessageW(
            hwnd,
            EM_GETSCROLLPOS,
            None,
            Some(LPARAM((&mut scroll as *mut POINT) as isize)),
        );
        SendMessageW(
            hwnd,
            EM_GETRECT,
            None,
            Some(LPARAM((&mut rect as *mut RECT) as isize)),
        );
        RichState29 {
            text: app.text(id),
            font: SendMessageW(hwnd, WM_GETFONT, None, None).0,
            selection: (selection.cpMin, selection.cpMax),
            scroll: (scroll.x, scroll.y),
            format: (rect.left, rect.top, rect.right, rect.bottom),
        }
    }
}
unsafe fn restore_rich_state29(hwnd: HWND, state: &RichState29) {
    unsafe {
        let selection = CHARRANGE {
            cpMin: state.selection.0,
            cpMax: state.selection.1,
        };
        let scroll = POINT {
            x: state.scroll.0,
            y: state.scroll.1,
        };
        SendMessageW(
            hwnd,
            EM_EXSETSEL,
            None,
            Some(LPARAM((&selection as *const CHARRANGE) as isize)),
        );
        SendMessageW(
            hwnd,
            EM_SETSCROLLPOS,
            None,
            Some(LPARAM((&scroll as *const POINT) as isize)),
        );
    }
}
unsafe fn paragraph29(hwnd: HWND, index: i32) -> PARAFORMAT2 {
    unsafe {
        let mut selection = CHARRANGE::default();
        let mut scroll = POINT::default();
        SendMessageW(
            hwnd,
            EM_EXGETSEL,
            None,
            Some(LPARAM((&mut selection as *mut CHARRANGE) as isize)),
        );
        SendMessageW(
            hwnd,
            EM_GETSCROLLPOS,
            None,
            Some(LPARAM((&mut scroll as *mut POINT) as isize)),
        );
        SendMessageW(
            hwnd,
            EM_SETSEL,
            Some(WPARAM(index as usize)),
            Some(LPARAM(index as isize)),
        );
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
        SendMessageW(
            hwnd,
            EM_SETSCROLLPOS,
            None,
            Some(LPARAM((&scroll as *const POINT) as isize)),
        );
        paragraph
    }
}
unsafe fn check_paragraph29(paragraph: &PARAFORMAT2, rtl: bool) -> Result<()> {
    unsafe {
        if (paragraph.Base.dwMask & PFM_RTLPARA).0 == 0
            || (paragraph.Base.Anonymous.wEffects & 1 != 0) != rtl
            || paragraph.Base.wAlignment != if rtl { PFA_RIGHT } else { PFA_LEFT }
        {
            return Err("Study29 body/attribution native paragraph direction is incorrect".into());
        }
        Ok(())
    }
}
unsafe fn native_rect29(app: &App, id: u32) -> Result<Rect> {
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
