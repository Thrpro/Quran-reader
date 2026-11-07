//! Native study/player separation, title-bar handoff and real hover labels.
use super::*;

impl App {
    pub(super) unsafe fn qa_enhancements26(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let _dispatch_guard = DispatchGuard::enter();
            let prefs = self.prefs.clone();
            let selected = self.selected;
            let source = self.source;
            let panel = self.panel;
            let collapsed = self.player_collapsed;
            let fullscreen = self.fullscreen;
            let placement = self.placement;
            let window_style = self.window_style;
            let keyboard = self.keyboard_focus;
            let focus = GetFocus();
            let search = self.search;
            let mut window = RECT::default();
            GetWindowRect(self.hwnd, &mut window)?;
            let mut cursor = POINT::default();
            GetCursorPos(&mut cursor)?;
            let _cursor_restore = CursorRestore26(cursor);
            let saved_playback = std::mem::take(&mut self.playback);
            let saved_popup = self.popup.take();
            let saved_guide = self.guide.take();
            let saved_turn = self.page_turn.take();
            let saved_queue = std::mem::take(&mut self.turn_queue);
            let mut cases = Vec::new();
            let result = (|| -> Result<()> {
                if !self.audio.as_ref().is_some_and(|audio| {
                    audio.is_paused()
                        && audio
                            .transport()
                            .is_some_and(|t| t.continuous && t.can_seek)
                }) {
                    return Err("Study26 requires the original paused, seekable recording".into());
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

                for arabic in [false, true] {
                    for theme in [1, 0] {
                        self.prefs.arabic = arabic;
                        self.prefs.theme = theme;
                        self.theme_start = None;
                        self.theme_frame = None;
                        self.replace_background(self.palette());
                        self.player_collapsed = false;
                        self.panel = Panel::Study;
                        self.panel_animation = None;
                        self.selected = Some((46, 21));
                        self.source = if arabic { 1 } else { 2 };
                        self.refresh_labels();
                        self.study()?;
                        self.resize();
                        self.refresh_transport();
                        let language = if arabic { "arabic" } else { "english" };
                        let mode = if theme == 0 { "dark" } else { "light" };
                        let stem = format!("native-study26-{language}-{mode}");
                        let body = native_rect26(self, STUDY)?;
                        let status = native_rect26(self, STATUS)?;
                        let player_top = self.transport_top() - 8.;
                        if body.y < 174.
                            || body.y > 184.
                            || body.y + body.h + 20. > player_top + 1.
                            || status.y < player_top
                            || !self.embedded_transport()
                            || !IsWindowVisible(self.controls[&COLLAPSE_PLAYER]).as_bool()
                        {
                            return Err(
                                "Study26 body frame overlaps the separate player card".into()
                            );
                        }
                        if self.panel_on_left() == arabic {
                            return Err("Study26 panel is on the wrong language edge".into());
                        }
                        self.paint()?;
                        self.capture(&out.join(format!("{stem}-separated.png")))?;

                        // Genuine long content must expose a usable native
                        // scroll range, with no synthetic filler in the reader.
                        self.selected = Some((2, 282));
                        self.study()?;
                        self.resize();
                        self.paint()?;
                        if !self.study_overflows() {
                            return Err("Long Study26 source lost its visible scrolling cue".into());
                        }
                        let original =
                            self.resources
                                .study(if arabic { "saadi" } else { "sahih" }, 2, 282)?;
                        if !self.text(STUDY).starts_with(&original) {
                            return Err("Study26 did not retain the exact long source text".into());
                        }
                        let mut scroll = SCROLLINFO {
                            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                            ..Default::default()
                        };
                        GetScrollInfo(self.controls[&STUDY], SB_VERT, &mut scroll)?;
                        if scroll.nPage == 0
                            || scroll.nMax.saturating_sub(scroll.nMin) as u32 <= scroll.nPage
                        {
                            return Err(
                                "Long Study26 source has no native scrolling affordance".into()
                            );
                        }
                        let long_body = native_rect26(self, STUDY)?;
                        let cue_bottom = long_body.y + long_body.h + 26.;
                        if cue_bottom > player_top + 1. {
                            return Err("Study26 scrolling cue overlaps the player card".into());
                        }
                        SendMessageW(
                            self.controls[&STUDY],
                            EM_SCROLL,
                            Some(WPARAM(SB_PAGEDOWN.0 as usize)),
                            None,
                        );
                        let mut scrolled = scroll;
                        GetScrollInfo(self.controls[&STUDY], SB_VERT, &mut scrolled)?;
                        if scrolled.nPos <= scroll.nPos
                            || !self.text(STUDY).starts_with(&original)
                            || !IsWindowVisible(self.controls[&AUDIO_POSITION]).as_bool()
                        {
                            return Err(
                                "Native study scrolling hid playback or altered its source".into(),
                            );
                        }
                        self.paint()?;
                        self.capture(&out.join(format!("{stem}-long-source-scrolled.png")))?;

                        self.selected = Some((112, 1));
                        self.study()?;
                        self.resize();
                        self.paint()?;
                        if self.study_overflows() {
                            return Err(
                                "Short Study26 source incorrectly asks the reader to scroll".into(),
                            );
                        }
                        self.capture(&out.join(format!("{stem}-short-source.png")))?;

                        let hover = hover_label26(self, FOLLOW_AUDIO)?;
                        self.capture(&out.join(format!("{stem}-native-hover.png")))?;
                        self.resize();
                        if self
                            .tooltips
                            .as_ref()
                            .unwrap()
                            .inspect(self.controls[&FOLLOW_AUDIO])
                            .1
                        {
                            return Err(
                                "Study26 hover label survived its control layout change".into()
                            );
                        }
                        let book = crate::app_core::book_geometry(self.reader_area()).bounds;
                        let expanded_body = native_rect26(self, STUDY)?;
                        self.command(COLLAPSE_PLAYER, 0)?;
                        let collapsed_body = native_rect26(self, STUDY)?;
                        let collapsed_book =
                            crate::app_core::book_geometry(self.reader_area()).bounds;
                        if !self.player_collapsed
                            || collapsed_body.h < expanded_body.h + 100.
                            || !same_rect26(book, collapsed_book)
                            || self.panel != Panel::Study
                            || IsWindowVisible(self.controls[&AUDIO_POSITION]).as_bool()
                        {
                            return Err("Study26 collapse failed to reclaim text space without moving the book".into());
                        }
                        for id in [QUICK_PAUSE, COLLAPSE_PLAYER, FOLLOW_AUDIO] {
                            if !IsWindowVisible(self.controls[&id]).as_bool()
                                || native_rect26(self, id)?.y > 8.
                            {
                                return Err(
                                    "Study26 collapsed action is missing from the top bar".into()
                                );
                            }
                        }
                        self.paint()?;
                        self.capture(&out.join(format!("{stem}-top-bar.png")))?;
                        self.command(COLLAPSE_PLAYER, 0)?;
                        if self.player_collapsed
                            || !same_rect26(expanded_body, native_rect26(self, STUDY)?)
                            || !same_rect26(
                                book,
                                crate::app_core::book_geometry(self.reader_area()).bounds,
                            )
                            || !IsWindowVisible(self.controls[&AUDIO_POSITION]).as_bool()
                        {
                            return Err(
                                "Study26 expansion did not restore its separate footer".into()
                            );
                        }
                        cases.push(serde_json::json!({
                            "arabic":arabic,"theme":theme,"bodyRect":rect26(body),
                            "playerTop":player_top,"scrollCueBottom":cue_bottom,
                            "originalLongSource":[2,282],"scrollRange":[scroll.nMin,scroll.nMax,scroll.nPage],
                            "scrollPositionBefore":scroll.nPos,"scrollPositionAfter":scrolled.nPos,
                            "longSourceScrollCue":true,"shortSourceNoFalseScrollCue":true,
                            "exactSourcePreserved":true,"nativeHover":hover,
                            "layoutDismissesHover":true,"bodyHeightExpanded":expanded_body.h,
                            "bodyHeightCollapsed":collapsed_body.h,"bookUnchangedByCollapse":true,
                            "topBarPauseExpandCurrentAyah":true,"embeddedPlayerRestored":true
                        }));
                    }

                    self.selected = Some((46, 21));
                    self.study()?;
                    self.toggle_fullscreen()?;
                    self.panel_animation = None;
                    self.resize();
                    let book = crate::app_core::book_geometry(self.reader_area()).bounds;
                    self.command(COLLAPSE_PLAYER, 0)?;
                    for id in [QUICK_PAUSE, COLLAPSE_PLAYER, FOLLOW_AUDIO] {
                        let rect = native_rect26(self, id)?;
                        if !IsWindowVisible(self.controls[&id]).as_bool()
                            || rect.x < 0.
                            || rect.x + rect.w > 60.
                            || rect.y < 180.
                        {
                            return Err(
                                "Fullscreen Study26 handoff lost an accessible rail action".into(),
                            );
                        }
                    }
                    if !same_rect26(
                        book,
                        crate::app_core::book_geometry(self.reader_area()).bounds,
                    ) {
                        return Err("Fullscreen Study26 collapse moved the Quran pages".into());
                    }
                    let hover = hover_label26(self, FOLLOW_AUDIO)?;
                    self.paint()?;
                    self.capture(&out.join(format!(
                        "native-study26-fullscreen-{}.png",
                        if arabic { "arabic" } else { "english" }
                    )))?;
                    self.command(COLLAPSE_PLAYER, 0)?;
                    if self.player_collapsed
                        || !IsWindowVisible(self.controls[&AUDIO_POSITION]).as_bool()
                    {
                        return Err("Fullscreen Study26 expand lost the embedded player".into());
                    }
                    cases.push(serde_json::json!({"arabic":arabic,"fullscreen":true,"railPauseExpandCurrentAyah":true,"bookUnchangedByCollapse":true,"nativeHover":hover,"expandedPlayerRestored":true}));
                    self.toggle_fullscreen()?;
                    self.panel_animation = None;
                }
                self.panel = Panel::None;
                self.panel_animation = None;
                self.player_collapsed = true;
                self.toggle_fullscreen()?;
                for height in [400., 540., 617., 650.] {
                    SetWindowPos(
                        self.hwnd,
                        None,
                        0,
                        0,
                        (1200. * self.dpi / 96.) as i32,
                        (height * self.dpi / 96.) as i32,
                        SWP_NOZORDER,
                    )?;
                    self.resize();
                    let previous = native_rect26(self, PREV)?;
                    for id in [
                        QUICK_PAUSE,
                        COLLAPSE_PLAYER,
                        FOLLOW_AUDIO,
                        QUICK_STOP,
                        FULLSCREEN,
                    ] {
                        let control = native_rect26(self, id)?;
                        if IsWindowVisible(self.controls[&id]).as_bool()
                            && previous.x < control.x + control.w
                            && previous.x + previous.w > control.x
                            && previous.y < control.y + control.h
                            && previous.y + previous.h > control.y
                        {
                            return Err(
                                "Fullscreen page arrow overlaps a collapsed player action".into()
                            );
                        }
                    }
                    cases.push(serde_json::json!({"fullscreenRailHeight":self.height,"previousPageRect":rect26(previous),"noControlOverlap":true}));
                }
                self.toggle_fullscreen()?;
                Ok(())
            })();

            // Restore every fixture preference and window/player mode even when
            // an assertion fails, so subsequent QA is not testing stale state.
            let restore = (|| -> Result<()> {
                if self.fullscreen {
                    self.toggle_fullscreen()?;
                }
                self.prefs = prefs;
                self.selected = selected;
                self.source = source;
                self.panel = panel;
                self.player_collapsed = collapsed;
                self.keyboard_focus = keyboard;
                self.search = search;
                self.search_animation = None;
                self.title_animation = None;
                self.panel_animation = None;
                self.theme_start = None;
                self.theme_frame = None;
                self.playback = saved_playback;
                self.popup = saved_popup;
                self.guide = saved_guide;
                self.page_turn = saved_turn;
                self.turn_queue = saved_queue;
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
                out.join("enhancements26.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "passed":true,"cases":cases,
                    "scope":"Actual HWND geometry, read-only original commentary and native scrolling; original paused continuous recording. Hover checks move this process's cursor, wait for the native tooltip timer and inspect its visible current-tool state and text. PNGs capture this app's client area; they are not a separate capture of the owned tooltip popup. No displayed-FPS claim."
                }))?,
            )?;
            Ok(())
        }
    }
}

unsafe fn hover_label26(app: &mut App, id: u32) -> Result<serde_json::Value> {
    unsafe {
        let control = app.controls[&id];
        let expected = app.tr("Go to current ayah", "انتقل إلى الآية الحالية");
        let tips = app
            .tooltips
            .as_ref()
            .ok_or("Native hover labels are unavailable")?;
        if tips.inspect(control).0 != expected {
            return Err("Native current-ayah tooltip lost its full localized label".into());
        }
        tips.pop();
        focus_window(app.controls[&STUDY])?;
        let focus = GetFocus();
        let mut owner = RECT::default();
        GetWindowRect(app.hwnd, &mut owner)?;
        SetCursorPos(owner.left + 4, owner.top + 20)?;
        SendMessageW(control, NATIVE_MOUSE_LEAVE, None, None);
        if !crate::audio::pump_messages() {
            return Err("Native tooltip QA was interrupted by window closure".into());
        }
        let mut bounds = RECT::default();
        let mut client = RECT::default();
        GetWindowRect(control, &mut bounds)?;
        GetClientRect(control, &mut client)?;
        let x = (client.right - client.left) / 2;
        let y = (client.bottom - client.top) / 2;
        SetCursorPos(
            (bounds.left + bounds.right) / 2,
            (bounds.top + bounds.bottom) / 2,
        )?;
        SendMessageW(
            control,
            WM_MOUSEMOVE,
            None,
            Some(LPARAM(
                ((y as u32 & 0xffff) << 16 | (x as u32 & 0xffff)) as isize,
            )),
        );
        let began = Instant::now();
        let mut visible = false;
        while began.elapsed() < std::time::Duration::from_millis(950) {
            if !crate::audio::pump_messages() {
                return Err("Native tooltip QA was interrupted by window closure".into());
            }
            if app.tooltips.as_ref().unwrap().inspect(control).1 {
                visible = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (label, showing, rtl) = app.tooltips.as_ref().unwrap().inspect(control);
        if !visible
            || !showing
            || label != expected
            || rtl != app.prefs.arabic
            || GetFocus() != focus
        {
            return Err(format!("Native tooltip did not show its complete action without taking focus: visible={visible}, showing={showing}, rtl={rtl}, arabic={}, label={label:?}, foregroundIsApp={}", app.prefs.arabic, GetForegroundWindow() == app.hwnd).into());
        }
        Ok(
            serde_json::json!({"label":label,"nativeVisible":showing,"rtl":rtl,"focusUnchanged":true,"hoverWaitMs":began.elapsed().as_secs_f64()*1000.}),
        )
    }
}

unsafe fn native_rect26(app: &App, id: u32) -> Result<Rect> {
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

fn same_rect26(first: Rect, second: Rect) -> bool {
    (first.x - second.x).abs() < 1.
        && (first.y - second.y).abs() < 1.
        && (first.w - second.w).abs() < 1.
        && (first.h - second.h).abs() < 1.
}

fn rect26(rect: Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.w, rect.h]
}

struct CursorRestore26(POINT);
impl Drop for CursorRestore26 {
    fn drop(&mut self) {
        unsafe {
            let _ = SetCursorPos(self.0.x, self.0.y);
        }
    }
}
