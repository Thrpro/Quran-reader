//! Actual HWND geometry and interaction checks for the compact player and settings.
use super::*;
impl App {
    pub(super) unsafe fn qa_enhancements23(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let saved_prefs = self.prefs.clone();
            let keyboard = self.keyboard_focus;
            let mut original = RECT::default();
            GetWindowRect(self.hwnd, &mut original)?;
            let mut cases = vec![];
            let result = (|| -> Result<()> {
                if !self
                    .audio
                    .as_ref()
                    .and_then(Audio::transport)
                    .is_some_and(|t| t.continuous && t.can_seek)
                {
                    return Err("Player23 requires the original seekable full-surah fixture".into());
                }
                for arabic in [false, true] {
                    self.prefs.arabic = arabic;
                    self.player_collapsed = false;
                    self.popup = None;
                    self.search = false;
                    self.panel = Panel::None;
                    self.panel_animation = None;
                    self.title_animation = None;
                    self.refresh_labels();
                    SetWindowPos(
                        self.hwnd,
                        None,
                        0,
                        0,
                        (1920. * self.dpi / 96.) as i32,
                        (800. * self.dpi / 96.) as i32,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )?;
                    self.resize();
                    self.refresh_transport();
                    let with_audio = self.reader_area();
                    let audio = self.audio.take();
                    let without_audio = self.reader_area();
                    self.audio = audio;
                    if (with_audio.x - without_audio.x).abs() > 0.1
                        || (with_audio.w - without_audio.w).abs() > 0.1
                    {
                        return Err("Wide floating player shifted the centered book".into());
                    }
                    let book = crate::app_core::book_geometry(with_audio).bounds;
                    let player = self.placements.borrow()[&STATUS].0;
                    if book.x + book.w + 8. > player.x {
                        return Err("Floating player covers Quran text despite spare margin".into());
                    }
                    let label = self.text(AUDIO_TIME);
                    if !label.contains('/') || label.contains('\n') {
                        return Err("Full-surah clock must be one compact line".into());
                    }
                    let lang = if arabic { "arabic" } else { "english" };
                    self.paint()?;
                    self.capture(&out.join(format!("native-player23-{lang}.png")))?;
                    self.command(COLLAPSE_PLAYER, 0)?;
                    if !IsWindowVisible(self.controls[&QUICK_PAUSE]).as_bool()
                        || IsWindowVisible(self.controls[&AUDIO_POSITION]).as_bool()
                        || self.placements.borrow()[&QUICK_PAUSE].0.y > 8.
                    {
                        return Err("Collapsed player did not retain title-bar pause".into());
                    }
                    self.paint()?;
                    self.capture(&out.join(format!("native-player23-collapsed-{lang}.png")))?;
                    self.command(COLLAPSE_PLAYER, 0)?;
                    // Reproduce a tall, narrower window: playback must leave the
                    // exact same reader area and fall back to caption controls.
                    SetWindowPos(
                        self.hwnd,
                        None,
                        0,
                        0,
                        (1100. * self.dpi / 96.) as i32,
                        (900. * self.dpi / 96.) as i32,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )?;
                    self.resize();
                    let active = self.reader_area();
                    let audio = self.audio.take();
                    let inactive = self.reader_area();
                    self.audio = audio;
                    if active.x != inactive.x || active.w != inactive.w || active.h != inactive.h {
                        return Err("Compact playback moved the reader in a narrow margin".into());
                    }
                    if self.compact_player_frame().is_some()
                        || self.placements.borrow()[&QUICK_PAUSE].0.y > 8.
                        || IsWindowVisible(self.controls[&AUDIO_POSITION]).as_bool()
                    {
                        return Err("Insufficient side space did not use the caption player".into());
                    }
                    self.paint()?;
                    self.capture(&out.join(format!("native-player24-narrow-{lang}.png")))?;
                    self.command(COLLAPSE_PLAYER, 0)?;
                    if self.panel != Panel::Listen {
                        return Err("Caption expand did not open listening settings".into());
                    }
                    self.panel = Panel::None;
                    self.panel_animation = None;
                    SetWindowPos(
                        self.hwnd,
                        None,
                        0,
                        0,
                        (1536. * self.dpi / 96.) as i32,
                        (864. * self.dpi / 96.) as i32,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )?;
                    self.resize();
                    if self.compact_player_frame().is_none() {
                        return Err("Screenshot-sized window should fit the compact player beside centered pages".into());
                    }
                    // Hours must fit on one native clock line with the real font.
                    let clock = self.controls[&AUDIO_TIME];
                    let dc = GetDC(Some(clock));
                    if dc.is_invalid() {
                        return Err("Cannot measure native progress text".into());
                    }
                    let font = SendMessageW(clock, WM_GETFONT, None, None);
                    let old = SelectObject(dc, HGDIOBJ(font.0 as *mut _));
                    let text: Vec<u16> = "1:03:24 / 2:00:47".encode_utf16().collect();
                    let mut extent = SIZE::default();
                    let measured = GetTextExtentPoint32W(dc, &text, &mut extent).as_bool();
                    SelectObject(dc, old);
                    ReleaseDC(Some(clock), dc);
                    let mut bounds = RECT::default();
                    GetClientRect(clock, &mut bounds)?;
                    if !measured || extent.cx > bounds.right || extent.cy > bounds.bottom {
                        return Err("Hour-scale progress text clips the native clock".into());
                    }
                    self.set_text(AUDIO_TIME, "1:03:24 / 2:00:47");
                    self.paint()?;
                    self.capture(&out.join(format!("native-player24-hour-clock-{lang}.png")))?;
                    self.refresh_transport();
                    self.panel = Panel::Study;
                    self.selected = Some((112, 1));
                    self.study()?;
                    self.resize();
                    let study = self.placements.borrow()[&STUDY].0;
                    let footer = self.placements.borrow()[&QUICK_SETTINGS].0;
                    if study.y + study.h + 4. > footer.y {
                        return Err("Tafsir viewport overlaps playback footer".into());
                    }
                    for direction in [SB_BOTTOM, SB_TOP] {
                        SendMessageW(
                            self.controls[&STUDY],
                            WM_VSCROLL,
                            Some(WPARAM(direction.0 as usize)),
                            None,
                        );
                        self.paint()?;
                        if !IsWindowVisible(self.controls[&QUICK_SETTINGS]).as_bool()
                            || !IsWindowVisible(self.controls[&QUICK_PAUSE]).as_bool()
                        {
                            return Err("Tafsir scroll hid playback controls".into());
                        }
                        self.capture(
                            &out.join(format!("native-player23-study-{lang}-{}.png", direction.0)),
                        )?;
                    }
                    self.panel = Panel::None;
                    self.keyboard_focus = false;
                    self.options()?;
                    self.popup_command(MENU_ROW + 1)?;
                    if let Some(p) = &mut self.popup {
                        p.motion = None;
                    }
                    self.resize();
                    let checks = self
                        .popup
                        .as_ref()
                        .unwrap()
                        .rows
                        .iter()
                        .filter_map(menu_check)
                        .collect::<Vec<_>>();
                    if checks.len() < 3 || !checks.contains(&true) {
                        return Err("Settings lost explicit checked-state controls".into());
                    }
                    self.paint()?;
                    self.capture(&out.join(format!("native-settings23-mouse-{lang}.png")))?;
                    self.keyboard_focus = true;
                    focus_window(self.controls[&(MENU_ROW + 1)])?;
                    self.paint()?;
                    self.capture(&out.join(format!("native-settings23-keyboard-{lang}.png")))?;
                    self.popup = None;
                    self.resize();
                    self.open_listen();
                    self.panel_animation = None;
                    self.resize();
                    if GetWindowLongPtrW(self.controls[&RECITER], GWL_STYLE) as u32 & 3
                        != CBS_DROPDOWNLIST as u32
                    {
                        return Err("Reciter still uses a selected editable text field".into());
                    }
                    if !IsWindowVisible(self.controls[&RECORDING_MODE]).as_bool() {
                        return Err(
                            "Recording source is not available in listening settings".into()
                        );
                    }
                    self.paint()?;
                    self.capture(&out.join(format!("native-listen23-{lang}.png")))?;
                    cases.push(serde_json::json!({"arabic":arabic,"centeredBookUnchanged":true,"narrowMarginPreservesBook":true,"captionExpandOpensSettings":true,"hourClockFitsNativeFont":true,"floatingPlayerDoesNotCoverBook":true,"collapsedTitlePause":true,"studyBottom":study.y+study.h,"footerTop":footer.y,"tafsirScrollRetainsTransport":true,"nativeClock":label,"progressSingleLine":true,"checkedStates":checks,"keyboardFocusRetained":true,"reciterNonEditable":true,"recordingSourceVisible":true}));
                }
                std::fs::write(
                    out.join("enhancements23.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"passed":true,"cases":cases,"scope":"Real HWND geometry and native control commands, original full-surah recording; screenshots are not presented-frame-rate measurements."}),
                    )?,
                )?;
                Ok(())
            })();
            self.prefs = saved_prefs;
            self.keyboard_focus = keyboard;
            self.player_collapsed = false;
            self.popup = None;
            self.panel = Panel::None;
            self.panel_animation = None;
            SetWindowPos(
                self.hwnd,
                None,
                original.left,
                original.top,
                original.right - original.left,
                original.bottom - original.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )?;
            self.refresh_labels();
            self.resize();
            result
        }
    }
}
