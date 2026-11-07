//! Native regression checks for menu composition, RTL transport and report text.
use super::*;
use windows::Win32::UI::Controls::TBM_GETTHUMBRECT;
use windows_numerics::Matrix3x2;

impl App {
    pub(super) unsafe fn qa_enhancements22(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let pinned_hot = self.qa_menu_hot.take();
            let prefs = self.prefs.clone();
            let panel = self.panel;
            let selected = self.selected;
            let hovered = self.hovered;
            let source = self.source;
            let search = self.search;
            let popup = self.popup.take();
            let audio = self.audio.take();
            let panel_animation = self.panel_animation.take();
            let search_animation = self.search_animation.take();
            let title_animation = self.title_animation.take();
            let page_turn = self.page_turn.take();
            let page_drag = self.page_drag.take();
            let theme_start = self.theme_start.take();
            let theme_frame = self.theme_frame.take();
            let storage_pending = self.storage_pending.take();
            let notice_until = self.notice_until;
            let notice = self.text(NOTICE);
            let study_text = self.text(STUDY);
            let verse_text_end = self.verse_text_end;
            let attribution = self.study_attribution_start;
            let show_clock = self.renderer.show_clock;
            let slider = self.controls[&AUDIO_POSITION];
            let slider_position = SendMessageW(slider, TBM_GETPOS, None, None);
            let slider_enabled = IsWindowEnabled(slider).as_bool();
            let focus = GetFocus();
            let mut cursor = POINT::default();
            GetCursorPos(&mut cursor).map_err(|e| format!("QA22 initial cursor: {e}"))?;
            let _cursor = CursorRestore(cursor);

            let result = (|| -> Result<()> {
                self.search = false;
                self.panel = Panel::None;
                self.prefs.show_clock = false;
                self.renderer.show_clock = false;
                self.prefs.page = 49;
                self.notice_until = Some(Instant::now() + std::time::Duration::from_secs(60));
                self.set_text(NOTICE, "Native hover regression");
                let mut directions = Vec::new();
                let mut contexts = Vec::new();
                let mut hover = Vec::new();
                let mut storage = Vec::new();

                for arabic in [false, true] {
                    self.prefs.arabic = arabic;
                    self.refresh_labels();
                    self.panel = Panel::Listen;
                    self.panel_animation = None;
                    self.resize();
                    self.layout_transport();
                    let _ = EnableWindow(slider, true);
                    let mirrored =
                        GetWindowLongPtrW(slider, GWL_EXSTYLE) & WS_EX_LAYOUTRTL.0 as isize != 0;
                    if mirrored != arabic {
                        return Err(
                            "Playback trackbar does not follow the interface direction".into()
                        );
                    }
                    let mut endpoints = Vec::new();
                    for position in [0, 2500, 10000] {
                        SendMessageW(slider, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(position)));
                        let actual = SendMessageW(slider, TBM_GETPOS, None, None).0;
                        if actual != position {
                            return Err(
                                "RTL transport inverted its accessible elapsed value".into()
                            );
                        }
                        endpoints.push(thumb_center(slider));
                    }
                    if if arabic {
                        !(endpoints[0] > endpoints[1] && endpoints[1] > endpoints[2])
                    } else {
                        !(endpoints[0] < endpoints[1] && endpoints[1] < endpoints[2])
                    } {
                        return Err("Playback thumb does not advance in reading order".into());
                    }
                    let mut keys = Vec::new();
                    for key in [VK_LEFT, VK_RIGHT] {
                        let _ = EnableWindow(slider, true);
                        SendMessageW(slider, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(5000)));
                        SendMessageW(slider, WM_KEYDOWN, Some(WPARAM(key.0 as usize)), None);
                        keys.push(SendMessageW(slider, TBM_GETPOS, None, None).0);
                        SendMessageW(slider, WM_KEYUP, Some(WPARAM(key.0 as usize)), None);
                    }
                    if if arabic {
                        !(keys[0] > 5000 && keys[1] < 5000)
                    } else {
                        !(keys[0] < 5000 && keys[1] > 5000)
                    } {
                        return Err(
                            "Playback arrow keys oppose their physical thumb direction".into()
                        );
                    }
                    directions.push(serde_json::json!({
                        "arabic":arabic,"nativeMirrored":mirrored,"positions":[0,2500,10000],
                        "thumbScreenCenters":endpoints,"leftKeyValue":keys[0],"rightKeyValue":keys[1],
                        "nativeAccessibleValuesRemainElapsed":true
                    }));

                    self.panel = Panel::None;
                    self.resize();
                    self.context_at((2, 286), self.width / 2., 200., false)?;
                    let pin = self.popup.as_ref().and_then(|p| p.context_target);
                    if pin.is_none() || self.canvas_selection() != Some((2, 286)) {
                        return Err("Ayah context menu failed to pin its Quran highlight".into());
                    }
                    let bookmark_visible = |app: &Self| {
                        app.popup.as_ref().is_some_and(|p| {
                            p.rows.iter().enumerate().any(|(i, r)| {
                                r.id == BOOKMARK
                                    && IsWindowVisible(app.controls[&(MENU_ROW + i as u32)])
                                        .as_bool()
                            })
                        })
                    };
                    if !bookmark_visible(self) {
                        return Err("Collapsed ayah menu lost its bookmark action".into());
                    }
                    self.popup_command(MENU_ROW + 2)?;
                    self.hovered = Some((3, 1));
                    let expanded = self
                        .popup
                        .as_ref()
                        .ok_or("Expanded ayah menu disappeared")?;
                    if !expanded.listening_expanded
                        || expanded.motion.is_some()
                        || expanded.context_target != pin
                        || bookmark_visible(self)
                        || self.canvas_selection() != Some((2, 286))
                    {
                        return Err(
                            "Inline listening choices lost immediate layout or ayah pin".into()
                        );
                    }
                    self.invalidate();
                    self.paint()?;
                    self.capture(&out.join(format!(
                        "native-context22-{}-expanded.png",
                        language(arabic)
                    )))?;
                    self.popup_command(MENU_ROW + 2)?;
                    if !bookmark_visible(self)
                        || self.popup.as_ref().is_none_or(|p| {
                            p.listening_expanded || p.motion.is_some() || p.context_target != pin
                        })
                    {
                        return Err(
                            "Collapsing listening choices did not restore the bookmark".into()
                        );
                    }
                    self.paint()?;
                    self.capture(&out.join(format!(
                        "native-context22-{}-collapsed.png",
                        language(arabic)
                    )))?;
                    contexts.push(serde_json::json!({"arabic":arabic,"target":pin,"immediateExpansion":true,"bookmarkHidesAndReturns":true,"highlightPinned":true}));

                    self.popup = None;
                    self.options()?;
                    self.popup_command(MENU_ROW + 1)?;
                    self.popup
                        .as_mut()
                        .ok_or("Appearance drawer missing")?
                        .motion = None;
                    self.theme_start = None;
                    self.theme_frame = None;
                    self.resize();
                    focus_window(self.hwnd)?;
                    let row_index = self
                        .popup
                        .as_ref()
                        .unwrap()
                        .rows
                        .iter()
                        .position(|r| r.id == CLOCK)
                        .ok_or("Appearance hover row missing")?;
                    let row = self.controls[&(MENU_ROW + row_index as u32)];
                    let row_bounds = client_bounds(self.hwnd, row)
                        .map_err(|e| format!("QA22 row bounds: {e}"))?;
                    let mut protected = Vec::new();
                    for id in [
                        THEME_LABEL,
                        THEME_SELECTOR,
                        SEARCH,
                        LISTEN,
                        SETTINGS,
                        NOTICE,
                    ] {
                        let bounds = client_bounds(self.hwnd, self.controls[&id])
                            .map_err(|e| format!("QA22 protected control {id}: {e}"))?;
                        if IsWindowVisible(self.controls[&id]).as_bool()
                            && intersects(row_bounds, bounds)
                        {
                            return Err(
                                "Appearance hover row overlaps a protected native control".into()
                            );
                        }
                        protected.push(serde_json::json!({"id":id,"bounds":rect_json(bounds)}));
                    }
                    let mut origin = POINT::default();
                    ClientToScreen(self.hwnd, &mut origin)
                        .ok()
                        .map_err(|e| format!("QA22 client origin: {e}"))?;
                    let system_cursor = SetCursorPos(
                        origin.x + (self.width * self.dpi / 96. / 2.) as i32,
                        origin.y + (20. * self.dpi / 96.) as i32,
                    )
                    .is_ok();
                    // Some isolated desktop processes can read but cannot move
                    // the system cursor. Keep native hover messages and exact
                    // pixel/damage assertions; pin only the paint hit test.
                    self.qa_menu_hot = (!system_cursor).then_some(HWND::default());
                    self.invalidate();
                    self.paint()?;
                    if !self.popup_fields_in_frame {
                        return Err("Popup native fields fell back to a second presentation".into());
                    }
                    let before = format!("native-hover22-{}-before.png", language(arabic));
                    self.capture(&out.join(&before))?;
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_CANCEL | TME_LEAVE,
                        hwndTrack: row,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut track);
                    if system_cursor {
                        SetCursorPos(
                            origin.x + (row_bounds.left + row_bounds.right) / 2,
                            origin.y + (row_bounds.top + row_bounds.bottom) / 2,
                        )
                        .map_err(|e| format!("QA22 cursor into row: {e}"))?;
                    } else {
                        self.qa_menu_hot = Some(row);
                    }
                    let _ = ValidateRect(Some(self.hwnd), None);
                    SendMessageW(row, WM_MOUSEMOVE, None, Some(LPARAM((4 << 16) | 4)));
                    let enter_damage = bounded_damage(self.hwnd, row_bounds)?;
                    self.paint()?;
                    let entered = format!("native-hover22-{}-entered.png", language(arabic));
                    self.capture(&out.join(&entered))?;
                    if system_cursor {
                        SetCursorPos(
                            origin.x + (self.width * self.dpi / 96. / 2.) as i32,
                            origin.y + (20. * self.dpi / 96.) as i32,
                        )
                        .map_err(|e| format!("QA22 cursor away after hover: {e}"))?;
                    } else {
                        self.qa_menu_hot = Some(HWND::default());
                    }
                    let _ = ValidateRect(Some(self.hwnd), None);
                    SendMessageW(row, NATIVE_MOUSE_LEAVE, None, None);
                    let leave_damage = bounded_damage(self.hwnd, row_bounds)?;
                    self.paint()?;
                    let left = format!("native-hover22-{}-left.png", language(arabic));
                    self.capture(&out.join(&left))?;
                    hover.push(serde_json::json!({
                        "arabic":arabic,"cursorMode":if system_cursor { "native-system-cursor" } else { "native-hover-messages-with-pinned-paint-hit-test" },"allowedChangedBounds":rect_json(row_bounds),
                        "enterDamage":enter_damage,"leaveDamage":leave_damage,"protected":protected,
                        "before":before,"entered":entered,"left":left,
                        "nativeFieldsInSamePresentation":true,"pixelComparisonRequired":true
                    }));

                    self.popup = None;
                    self.qa_menu_hot = None;
                    self.storage_details()?;
                    self.panel_animation = None;
                    let began = Instant::now();
                    while self.storage_pending.is_some() {
                        self.poll_storage()?;
                        if began.elapsed().as_secs() >= 5 {
                            return Err(
                                "Storage report worker did not finish in its QA deadline".into()
                            );
                        }
                        if self.storage_pending.is_some() {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                    self.format_storage_details();
                    let report = self.controls[&STUDY];
                    let mut bidi = BIDIOPTIONS {
                        cbSize: std::mem::size_of::<BIDIOPTIONS>() as u32,
                        wMask: BOM_UNICODEBIDI as u16,
                        ..Default::default()
                    };
                    SendMessageW(
                        report,
                        EM_GETBIDIOPTIONS,
                        None,
                        Some(LPARAM((&mut bidi as *mut BIDIOPTIONS) as isize)),
                    );
                    if bidi.wEffects & BOE_UNICODEBIDI as u16 == 0 {
                        return Err("Storage path uses legacy bidirectional ordering".into());
                    }
                    let (body, paragraph) = text_format_at(report, 0);
                    if body.Base.yHeight > 240
                        || body.Base.yHeight <= 0
                        || paragraph.Base.wAlignment != if arabic { PFA_RIGHT } else { PFA_LEFT }
                        || (paragraph.Base.Anonymous.wEffects & 1 != 0) != arabic
                        || (GetWindowLongPtrW(report, GWL_EXSTYLE) & WS_EX_LEFTSCROLLBAR.0 as isize
                            != 0)
                            != arabic
                    {
                        return Err(
                            "Storage report lost compact typography or native paragraph direction"
                                .into(),
                        );
                    }
                    let normalized = self.text(STUDY).replace("\r\n", "\n").replace('\r', "\n");
                    let path_index = normalized
                        .rfind('\n')
                        .ok_or("Storage path paragraph missing")?
                        + 1;
                    let path_start = normalized[..path_index].encode_utf16().count();
                    let (path_font, path_paragraph) = text_format_at(report, path_start);
                    if path_paragraph.Base.wAlignment != PFA_LEFT
                        || path_paragraph.Base.Anonymous.wEffects & 1 != 0
                        || path_font.Base.yHeight > 240
                    {
                        return Err("Storage path is not its own compact LTR paragraph".into());
                    }
                    self.resize();
                    self.invalidate();
                    self.paint()?;
                    self.capture(&out.join(format!("native-storage22-{}.png", language(arabic))))?;
                    storage.push(serde_json::json!({"arabic":arabic,"bodyTwips":body.Base.yHeight,"pathTwips":path_font.Base.yHeight,"rightAlignedRtlBody":arabic,"leftScrollbar":arabic,"pathOwnLtrParagraph":true,"unicodeBidiEnabled":true}));
                    self.prefs.arabic = false;
                    self.source = 2;
                    self.panel = Panel::Study;
                    self.selected = Some((2, 10));
                    self.study()?;
                    let (restored, paragraph) = text_format_at(report, 0);
                    if self.study_font_key != Some((self.study_size, self.dpi.to_bits()))
                        || (restored.Base.yHeight - self.study_size * 15).abs() > 20
                        || paragraph.Base.wAlignment != PFA_LEFT
                        || paragraph.Base.Anonymous.wEffects & 1 != 0
                        || GetWindowLongPtrW(report, GWL_EXSTYLE) & WS_EX_LEFTSCROLLBAR.0 as isize
                            != 0
                    {
                        return Err("Returning from storage did not restore the English tafsir font/direction".into());
                    }
                    self.panel = Panel::None;
                }

                self.popup = None;
                self.resize();
                self.invalidate();
                self.paint()?;
                let mut client = RECT::default();
                GetClientRect(self.hwnd, &mut client)?;
                let target = self.renderer.window_target(
                    self.hwnd,
                    client.right as u32,
                    client.bottom as u32,
                    self.dpi,
                    false,
                )?;
                let mut original = Matrix3x2::identity();
                target.GetTransform(&mut original);
                let translated = Matrix3x2 {
                    M31: 193.,
                    M32: 147.,
                    ..Matrix3x2::identity()
                };
                let mut drawing = DrawSession::begin(target.clone());
                target.SetTransform(&translated);
                let painted = self.chrome.paint_button(
                    &target,
                    Rect {
                        x: 0.,
                        y: 0.,
                        w: 300.,
                        h: 64.,
                    },
                    self.palette(),
                    crate::chrome::Button {
                        text: "Tafsir",
                        detail: "Read the explanation",
                        quran_detail: false,
                        menu: true,
                        icon: crate::chrome::Icon::Book,
                        active: false,
                        disabled: false,
                        focus: false,
                        flat: true,
                        rtl: false,
                        submenu: false,
                        check: None,
                    },
                );
                let mut after = Matrix3x2::identity();
                target.GetTransform(&mut after);
                target.SetTransform(&original);
                let ended = drawing.finish();
                painted?;
                ended?;
                if matrix_values(after) != matrix_values(translated) {
                    return Err("Book icon discarded the caller's popup transform".into());
                }
                self.invalidate();
                self.paint()?;
                std::fs::write(
                    out.join("enhancements22.json"),
                    serde_json::to_vec_pretty(&serde_json::json!({
                        "passed":true,"nativeTransportDirection":directions,"contextMenus":contexts,
                        "appearanceHover":hover,"storageReports":storage,"tafsirFontRestored":true,
                        "bookCallerTransformPreserved":true,
                        "scope":"Actual native HWND messages, geometry, RichEdit formats and caller transform; frozen captures require outside-row pixel comparison. Not physical-input or display-FPS certification."
                    }))?,
                )?;
                Ok(())
            })();

            self.qa_menu_hot = pinned_hot;
            self.prefs = prefs;
            self.panel = panel;
            self.selected = selected;
            self.hovered = hovered;
            self.source = source;
            self.search = search;
            self.popup = popup;
            self.audio = audio;
            self.panel_animation = panel_animation;
            self.search_animation = search_animation;
            self.title_animation = title_animation;
            self.page_turn = page_turn;
            self.page_drag = page_drag;
            self.theme_start = theme_start;
            self.theme_frame = theme_frame;
            self.storage_pending = storage_pending;
            self.notice_until = notice_until;
            self.verse_text_end = verse_text_end;
            self.study_attribution_start = attribution;
            self.renderer.show_clock = show_clock;
            self.set_text(NOTICE, &notice);
            self.set_text(STUDY, &study_text);
            self.study_font_key = None;
            self.refresh_labels();
            self.refresh_study_font();
            self.resize();
            SendMessageW(
                slider,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(slider_position.0)),
            );
            let _ = EnableWindow(slider, slider_enabled);
            if !focus.is_invalid() && IsWindow(Some(focus)).as_bool() {
                let _ = SetFocus(Some(focus));
            }
            self.invalidate();
            result
        }
    }
}

fn language(arabic: bool) -> &'static str {
    if arabic { "arabic" } else { "english" }
}
fn rect_json(r: RECT) -> [i32; 4] {
    [r.left, r.top, r.right, r.bottom]
}
fn intersects(a: RECT, b: RECT) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}
fn matrix_values(m: Matrix3x2) -> [u32; 6] {
    [
        m.M11.to_bits(),
        m.M12.to_bits(),
        m.M21.to_bits(),
        m.M22.to_bits(),
        m.M31.to_bits(),
        m.M32.to_bits(),
    ]
}
unsafe fn thumb_center(hwnd: HWND) -> f64 {
    unsafe {
        let mut thumb = RECT::default();
        SendMessageW(
            hwnd,
            TBM_GETTHUMBRECT,
            None,
            Some(LPARAM((&mut thumb as *mut RECT) as isize)),
        );
        let mut points = [
            POINT {
                x: thumb.left,
                y: thumb.top,
            },
            POINT {
                x: thumb.right,
                y: thumb.bottom,
            },
        ];
        MapWindowPoints(Some(hwnd), None, &mut points);
        (points[0].x as f64 + points[1].x as f64) / 2.
    }
}
unsafe fn client_bounds(parent: HWND, child: HWND) -> Result<RECT> {
    unsafe {
        let mut r = RECT::default();
        let mut origin = POINT::default();
        GetWindowRect(child, &mut r)?;
        ClientToScreen(parent, &mut origin).ok()?;
        Ok(RECT {
            left: r.left - origin.x,
            top: r.top - origin.y,
            right: r.right - origin.x,
            bottom: r.bottom - origin.y,
        })
    }
}
unsafe fn bounded_damage(hwnd: HWND, allowed: RECT) -> Result<Option<[i32; 4]>> {
    unsafe {
        let mut dirty = RECT::default();
        if !GetUpdateRect(hwnd, Some(&mut dirty), false).as_bool() {
            return Ok(None);
        }
        if dirty.left < allowed.left
            || dirty.top < allowed.top
            || dirty.right > allowed.right
            || dirty.bottom > allowed.bottom
        {
            return Err("Hover damaged controls outside its own row".into());
        }
        Ok(Some(rect_json(dirty)))
    }
}
unsafe fn text_format_at(hwnd: HWND, position: usize) -> (CHARFORMAT2W, PARAFORMAT2) {
    unsafe {
        SendMessageW(
            hwnd,
            EM_SETSEL,
            Some(WPARAM(position)),
            Some(LPARAM(position as isize)),
        );
        let mut character = CHARFORMAT2W::default();
        character.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
        SendMessageW(
            hwnd,
            EM_GETCHARFORMAT,
            Some(WPARAM(SCF_SELECTION as usize)),
            Some(LPARAM((&mut character as *mut CHARFORMAT2W) as isize)),
        );
        let mut paragraph = PARAFORMAT2::default();
        paragraph.Base.cbSize = std::mem::size_of::<PARAFORMAT2>() as u32;
        SendMessageW(
            hwnd,
            EM_GETPARAFORMAT,
            None,
            Some(LPARAM((&mut paragraph as *mut PARAFORMAT2) as isize)),
        );
        (character, paragraph)
    }
}
struct CursorRestore(POINT);
impl Drop for CursorRestore {
    fn drop(&mut self) {
        unsafe {
            let _ = SetCursorPos(self.0.x, self.0.y);
        }
    }
}
