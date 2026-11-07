//! A compact native listening form. Playback keeps its independent transport.
use super::*;

impl App {
    pub(super) unsafe fn layout_listening_controls(&self, x: f32, w: f32) {
        unsafe {
            let layout = ListenLayout::new(self.height);
            let field = |id, x, y, w, h| {
                self.place(
                    id,
                    Rect {
                        x,
                        y: layout.y(y),
                        w,
                        h,
                    },
                    true,
                );
            };
            for (label, control, y) in [
                (RECITER_LABEL, RECITER, 104.),
                (CHAPTER_LABEL, CHAPTER, 166.),
            ] {
                field(label, x, y, w, 20.);
                field(control, x, y + 22., w, 300.);
            }

            // Keep each label beside its own small number field. The two groups
            // follow reading order without mirroring numeric input or its keys.
            let gap = 16.;
            let column = (w - gap) / 2.;
            for (label, control, leading) in [(FIRST_LABEL, FIRST, true), (LAST_LABEL, LAST, false)]
            {
                let right = leading == self.prefs.arabic;
                let column_x = x + if right { column + gap } else { 0. };
                field(
                    label,
                    column_x + if self.prefs.arabic { 72. } else { 0. },
                    233.,
                    column - 72.,
                    20.,
                );
                field(
                    control,
                    column_x + if self.prefs.arabic { 0. } else { column - 64. },
                    231.,
                    64.,
                    24.,
                );
            }

            let speed_width = 110.;
            let repeat_width = w - speed_width - gap;
            let speed_x = x + if self.prefs.arabic {
                w - speed_width
            } else {
                0.
            };
            let repeat_x = x + if self.prefs.arabic {
                0.
            } else {
                speed_width + gap
            };
            field(SPEED_LABEL, speed_x, 266., speed_width, 20.);
            field(REPEAT_LABEL, repeat_x, 266., repeat_width, 20.);
            field(SPEED, speed_x, 288., speed_width, 210.);
            field(REPEAT, repeat_x, 288., repeat_width, 210.);

            let play_width = 88.;
            field(
                RECORDING_MODE,
                x + if self.prefs.arabic {
                    play_width + 12.
                } else {
                    0.
                },
                330.,
                w - play_width - 12.,
                180.,
            );
            field(
                PLAY,
                x + if self.prefs.arabic {
                    0.
                } else {
                    w - play_width
                },
                330.,
                play_width,
                layout.button_height(),
            );

            // The complete volume caption owns a line, including "100%".
            // Do not squeeze it into the old 100-DIP slot beside the slider.
            field(VOLUME_LABEL, x, 380., w, 20.);
            field(VOLUME, x + 36., 404., w - 72., 24.);
            let details_width = 84.;
            field(
                DOWNLOAD,
                x + if self.prefs.arabic {
                    details_width + 8.
                } else {
                    0.
                },
                430.,
                w - details_width - 8.,
                layout.button_height(),
            );
            field(
                STORAGE,
                x + if self.prefs.arabic {
                    0.
                } else {
                    w - details_width
                },
                430.,
                details_width,
                layout.button_height(),
            );
            field(DOWNLOAD_STATUS, x, 472., w, 20.);
        }
    }

    pub(super) unsafe fn localize_listening_fields(&self) {
        unsafe {
            // RTL reading and right alignment preserve the native dropdown's
            // input/accessibility. WS_EX_LAYOUTRTL would mirror geometry twice.
            let mask = (WS_EX_RTLREADING | WS_EX_RIGHT).0 as isize;
            for id in [
                RECITER,
                CHAPTER,
                REPEAT,
                RECORDING_MODE,
                SOURCE,
                BROWSE_KIND,
            ] {
                let combo = self.controls[&id];
                let mut info = COMBOBOXINFO {
                    cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                    ..Default::default()
                };
                let mut handles = vec![combo];
                if GetComboBoxInfo(combo, &mut info).is_ok() && !info.hwndList.is_invalid() {
                    handles.push(info.hwndList);
                }
                for hwnd in handles {
                    let old = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    let wanted = (old & !mask) | if self.prefs.arabic { mask } else { 0 };
                    if old != wanted {
                        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
        }
    }

    #[cfg(feature = "qa")]
    pub(super) unsafe fn qa_listening27_geometry(&self) -> Result<serde_json::Value> {
        unsafe {
            if self.displayed_panel() != Panel::Listen {
                return Err("Listening geometry requires the visible listening form".into());
            }
            let rect = |id| -> Result<RECT> {
                let mut bounds = RECT::default();
                GetWindowRect(self.controls[&id], &mut bounds)?;
                Ok(bounds)
            };
            let first = rect(FIRST)?;
            let last = rect(LAST)?;
            let speed = rect(SPEED)?;
            let repeat = rect(REPEAT)?;
            let recording = rect(RECORDING_MODE)?;
            let play = rect(PLAY)?;
            let volume = rect(VOLUME)?;
            let volume_label = rect(VOLUME_LABEL)?;
            let reciter = rect(RECITER)?;
            let expected_width = (64. * self.dpi / 96.) as i32;
            if (first.right - first.left - expected_width).abs() > 1
                || (last.right - last.left - expected_width).abs() > 1
                || (first.top - last.top).abs() > 1
                || (first.left > last.left) != self.prefs.arabic
                || (speed.left > repeat.left) != self.prefs.arabic
                || (recording.left > play.left) != self.prefs.arabic
            {
                return Err("Listening rows lost compact directional grouping".into());
            }
            for (label, control) in [(FIRST_LABEL, FIRST), (LAST_LABEL, LAST)] {
                let label_rect = rect(label)?;
                let field_rect = rect(control)?;
                if (label_rect.left > field_rect.left) != self.prefs.arabic
                    || (label_rect.left < field_rect.right && label_rect.right > field_rect.left)
                {
                    return Err(
                        "An ayah label is detached from, or overlaps, its number field".into(),
                    );
                }
            }
            let mut directions = vec![];
            for id in [RECITER, CHAPTER, REPEAT, RECORDING_MODE] {
                let combo = self.controls[&id];
                let rtl = GetWindowLongPtrW(combo, GWL_EXSTYLE) as u32 & WS_EX_RTLREADING.0 != 0;
                let mut info = COMBOBOXINFO {
                    cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                    ..Default::default()
                };
                GetComboBoxInfo(combo, &mut info)?;
                let list_rtl =
                    GetWindowLongPtrW(info.hwndList, GWL_EXSTYLE) as u32 & WS_EX_RTLREADING.0 != 0;
                if rtl != self.prefs.arabic || list_rtl != self.prefs.arabic {
                    return Err("Native listening dropdown reading direction is incorrect".into());
                }
                directions.push(serde_json::json!({"control":id,"rtl":rtl,"listRtl":list_rtl}));
            }
            if (volume_label.right - volume_label.left - (reciter.right - reciter.left)).abs() > 1
                || volume_label.bottom >= volume.top
                || volume.bottom > rect(DOWNLOAD)?.top
            {
                return Err(
                    "The volume caption or slider no longer has its own bounded line".into(),
                );
            }

            // Measure the actual localized 100% caption in the selected native
            // font. Testing only its rectangle missed the original truncation.
            let dc = GetDC(Some(self.controls[&VOLUME_LABEL]));
            if dc.is_invalid() {
                return Err(windows::core::Error::from_thread().into());
            }
            let font = SendMessageW(self.controls[&VOLUME_LABEL], WM_GETFONT, None, None);
            let old = SelectObject(dc, HGDIOBJ(font.0 as *mut _));
            let mut text: Vec<_> = self.text(VOLUME_LABEL).encode_utf16().collect();
            let mut measured = RECT::default();
            let text_height = DrawTextW(
                dc,
                &mut text,
                &mut measured,
                DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
            );
            SelectObject(dc, old);
            let _ = ReleaseDC(Some(self.controls[&VOLUME_LABEL]), dc);
            if text_height <= 0
                || measured.right - measured.left > volume_label.right - volume_label.left
            {
                return Err("The localized volume percentage is clipped".into());
            }
            if IsWindowVisible(self.controls[&STATUS]).as_bool()
                && volume.bottom >= rect(STATUS)?.top
            {
                return Err("Listening form overlaps the playback transport".into());
            }
            Ok(serde_json::json!({
                "arabic":self.prefs.arabic,
                "rangeFieldWidthDip":64,
                "pairedRangeLabels":true,
                "mirroredRows":true,
                "nativeDirections":directions,
                "volumeCaption":self.text(VOLUME_LABEL),
                "volumeCaptionFits":true,
                "volumeTextWidthPx":measured.right-measured.left,
                "volumeAvailableWidthPx":volume_label.right-volume_label.left,
            }))
        }
    }
}
