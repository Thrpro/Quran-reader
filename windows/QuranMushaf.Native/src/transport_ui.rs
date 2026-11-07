//! Windows controls for the small, shared listening transport.
use super::*;
impl App {
    pub(super) fn compact_player_frame(&self) -> Option<Rect> {
        let book = crate::app_core::book_geometry(self.reader_area()).bounds;
        let w = (self.width - book.x - book.w - 24.).min(232.);
        (w >= 160.).then_some(Rect {
            x: self.width - w - 12.,
            y: self.height - 214.,
            w,
            h: 202.,
        })
    }

    pub(super) unsafe fn layout_transport(&self) {
        unsafe {
            if self.transport_font_dpi.get() != self.dpi as u32 {
                let font = CreateFontW(
                    -(13. * self.dpi / 96.).round() as i32,
                    0,
                    0,
                    0,
                    400,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    0,
                    w!("Segoe UI"),
                );
                if !font.is_invalid() {
                    for id in [STATUS, AUDIO_TIME] {
                        SendMessageW(
                            self.controls[&id],
                            WM_SETFONT,
                            Some(WPARAM(font.0 as usize)),
                            Some(LPARAM(0)),
                        );
                    }
                    let old = self.transport_font.replace(font);
                    if !old.is_invalid() {
                        let _ = DeleteObject(old.into());
                    }
                    self.transport_font_dpi.set(self.dpi as u32);
                }
            }
            crate::control_vectors::set_slider_rtl(
                self.controls[&AUDIO_POSITION],
                self.prefs.arabic,
            );
            // The numeric clocks remain ordinary LTR numbers within a native
            // RTL paragraph. Unicode isolate characters render as boxes in
            // some GDI STATIC implementations.
            let clock = self.controls[&AUDIO_TIME];
            let style = GetWindowLongPtrW(clock, GWL_STYLE);
            let desired = (style & !3) | 1;
            if style != desired {
                SetWindowLongPtrW(clock, GWL_STYLE, desired);
            }
            let extended = GetWindowLongPtrW(clock, GWL_EXSTYLE);
            let desired = (extended & !(WS_EX_RTLREADING.0 as isize))
                | if self.prefs.arabic {
                    WS_EX_RTLREADING.0 as isize
                } else {
                    0
                };
            if extended != desired {
                SetWindowLongPtrW(clock, GWL_EXSTYLE, desired);
            }
            let panel = self.displayed_panel();
            let has_panel = panel != Panel::None;
            let compact = self.compact_player_frame();
            if panel != Panel::Listen
                && (self.player_collapsed || (!has_panel && compact.is_none()))
            {
                let x = if self.fullscreen {
                    8.
                } else if self.prefs.arabic {
                    235.
                } else {
                    550f32.min(self.width - 250.)
                };
                let y = if self.fullscreen { 188. } else { 3. };
                let roomy = !self.fullscreen && self.width >= 1400.;
                if roomy {
                    self.place(
                        STATUS,
                        Rect {
                            x,
                            y: y + 3.,
                            w: 126.,
                            h: 24.,
                        },
                        true,
                    );
                }
                let offset = if roomy { 130. } else { 0. };
                self.place(
                    QUICK_PAUSE,
                    Rect {
                        x: x + offset,
                        y,
                        w: if self.fullscreen { 44. } else { 36. },
                        h: 30.,
                    },
                    true,
                );
                self.place(
                    COLLAPSE_PLAYER,
                    Rect {
                        x: if self.fullscreen { x } else { x + offset + 40. },
                        y: if self.fullscreen { y + 36. } else { y },
                        w: if self.fullscreen { 44. } else { 30. },
                        h: 30.,
                    },
                    true,
                );
                self.place(
                    FOLLOW_AUDIO,
                    Rect {
                        x: if self.fullscreen { x } else { x + offset + 76. },
                        y: if self.fullscreen { y + 72. } else { y },
                        w: if self.fullscreen {
                            44.
                        } else if roomy {
                            110.
                        } else {
                            74.
                        },
                        h: 30.,
                    },
                    true,
                );
                self.place(
                    QUICK_STOP,
                    Rect {
                        x: if self.fullscreen {
                            x
                        } else {
                            x + offset + 192.
                        },
                        y: if self.fullscreen { y + 108. } else { y },
                        w: if self.fullscreen { 44. } else { 62. },
                        h: 30.,
                    },
                    self.fullscreen || roomy,
                );
                return;
            }
            let x = if has_panel {
                self.panel_origin() + 16.
            } else {
                compact.unwrap().x + 12.
            };
            let w = if has_panel {
                360f32.min((self.width - 180.).max(250.)) - 32.
            } else {
                compact.unwrap().w - 24.
            };
            let y = if panel == Panel::Listen {
                ListenLayout::new(self.height).y(500.)
            } else if has_panel {
                self.transport_top()
            } else {
                compact.unwrap().y + 10.
            };
            let embedded = has_panel;
            self.place(
                QUICK_SETTINGS,
                Rect {
                    x: if embedded { x + w - 60. } else { x },
                    y: if embedded { y } else { y + 154. },
                    w: if embedded { 28. } else { w },
                    h: if embedded { 26. } else { 32. },
                },
                panel != Panel::Listen,
            );
            self.place(
                COLLAPSE_PLAYER,
                Rect {
                    x: x + w - 26.,
                    y,
                    w: 26.,
                    h: 26.,
                },
                panel != Panel::Listen,
            );
            // Loading and ready states use exactly the same geometry.
            self.place(
                STATUS,
                Rect {
                    x,
                    y,
                    w: w - if panel == Panel::Listen {
                        0.
                    } else if embedded {
                        66.
                    } else {
                        30.
                    },
                    h: if embedded { 22. } else { 26. },
                },
                true,
            );
            self.place(
                AUDIO_TIME,
                Rect {
                    x,
                    y: y + if embedded { 24. } else { 30. },
                    w,
                    h: 24.,
                },
                true,
            );
            self.place(
                AUDIO_POSITION,
                Rect {
                    x,
                    y: y + if embedded { 48. } else { 56. },
                    w,
                    h: 20.,
                },
                true,
            );
            let seek_width = if w < 180. { 32. } else { 40. };
            let gap = 6.;
            let row = y + if embedded { 70. } else { 84. };
            self.place(
                SEEK_BACK,
                Rect {
                    x: if self.prefs.arabic {
                        x + w - seek_width
                    } else {
                        x
                    },
                    y: row,
                    w: seek_width,
                    h: 30.,
                },
                true,
            );
            self.place(
                QUICK_PAUSE,
                Rect {
                    x: x + seek_width + gap,
                    y: row,
                    w: w - 2. * (seek_width + gap),
                    h: 30.,
                },
                true,
            );
            self.place(
                SEEK_FORWARD,
                Rect {
                    x: if self.prefs.arabic {
                        x
                    } else {
                        x + w - seek_width
                    },
                    y: row,
                    w: seek_width,
                    h: 30.,
                },
                true,
            );
            let row = y + if embedded { 106. } else { 120. };
            self.place(
                QUICK_STOP,
                Rect {
                    x: if self.prefs.arabic { x + w - 66. } else { x },
                    y: row,
                    w: 66.,
                    h: 30.,
                },
                true,
            );
            self.place(
                FOLLOW_AUDIO,
                Rect {
                    x: if self.prefs.arabic { x } else { x + 72. },
                    y: row,
                    w: w - 72.,
                    h: 30.,
                },
                true,
            );
        }
    }
    pub(super) unsafe fn refresh_transport(&self) {
        unsafe {
            let t = self.audio.as_ref().and_then(Audio::transport);
            if let Some(audio) = self.audio.as_ref().filter(|a| a.has_session()) {
                let key = audio
                    .current()
                    .or_else(|| self.active_range.map(|(r, _)| (r.surah, r.first)));
                let caption = key
                    .and_then(|(surah, ayah)| {
                        self.resources.surahs.get(surah as usize - 1).map(|source| {
                            format!(
                                "{} · {ayah}",
                                if self.prefs.arabic {
                                    &source.name_arabic
                                } else {
                                    &source.name_simple
                                }
                            )
                        })
                    })
                    .unwrap_or_else(|| {
                        audio
                            .status(self.prefs.arabic)
                            .lines()
                            .next()
                            .unwrap_or(self.tr("Recitation", "التلاوة"))
                            .to_owned()
                    });
                self.set_text(STATUS, &caption);
            }
            for id in [AUDIO_POSITION, SEEK_BACK, SEEK_FORWARD] {
                let enabled = t.is_some_and(|t| t.can_seek);
                if IsWindowEnabled(self.controls[&id]).as_bool() != enabled {
                    let _ = EnableWindow(self.controls[&id], enabled);
                }
            }
            if let Some(t) = t {
                let position = t.position_ms.saturating_add(t.offset_ms).min(t.duration_ms);
                let elapsed = crate::listening_layout::clock(position);
                let total = crate::listening_layout::clock(t.duration_ms);
                // A clip must never be presented as the complete surah. Only
                // continuous sessions have a validated full-recording clock.
                let label = crate::listening_layout::progress_label(
                    t.continuous,
                    self.prefs.arabic,
                    &elapsed,
                    &total,
                );
                self.set_text(AUDIO_TIME, &label);
                let scope = self
                    .listening_session
                    .as_ref()
                    .map(|session| {
                        let label = if session.label_arabic == self.prefs.arabic {
                            session.label.clone()
                        } else {
                            self.listening_scope_label(session.scope, session.page)
                        };
                        format!(
                            "{label} · {}:{}–{}:{} · ",
                            session.start.0, session.start.1, session.end.0, session.end.1
                        )
                    })
                    .unwrap_or_default();
                self.set_text(
                    AUDIO_POSITION,
                    &format!(
                        "{scope}{} · {} {} · {} {}",
                        self.tr(
                            if t.continuous {
                                "Surah position"
                            } else {
                                "Ayah position"
                            },
                            if t.continuous {
                                "موضع السورة"
                            } else {
                                "موضع الآية"
                            }
                        ),
                        self.tr("Elapsed", "المنقضي"),
                        elapsed,
                        self.tr("Total duration", "المدة الكاملة"),
                        total
                    ),
                );
                if GetCapture() != self.controls[&AUDIO_POSITION] {
                    let value =
                        ((position as u128 * 10000) / t.duration_ms.max(1) as u128) as usize;
                    if SendMessageW(self.controls[&AUDIO_POSITION], TBM_GETPOS, None, None).0
                        != value as isize
                    {
                        SendMessageW(
                            self.controls[&AUDIO_POSITION],
                            TBM_SETPOS,
                            Some(WPARAM(1)),
                            Some(LPARAM(value as isize)),
                        );
                    }
                }
            } else if self.audio.as_ref().is_some_and(|a| a.has_session()) {
                if self.audio.as_ref().is_some_and(Audio::needs_retry) {
                    self.set_text(
                        AUDIO_TIME,
                        self.tr("Retry in settings", "أعد المحاولة من الإعدادات"),
                    );
                } else if self.text(AUDIO_TIME).is_empty() {
                    self.set_text(AUDIO_TIME, self.tr("Loading audio…", "جارٍ تحميل الصوت…"));
                }
                // A transient decoder query must not blank an established clock.
            } else {
                self.set_text(AUDIO_TIME, "");
            }
            if let Some(audio) = &self.audio {
                let paused = audio.is_paused();
                self.set_text(
                    QUICK_PAUSE,
                    self.tr(
                        if paused {
                            "Resume · Space"
                        } else {
                            "Pause · Space"
                        },
                        if paused {
                            "متابعة · مسافة"
                        } else {
                            "إيقاف مؤقت · مسافة"
                        },
                    ),
                );
            }
        }
    }
    pub(super) unsafe fn seek_audio(&mut self, delta_ms: Option<i64>, slider: u32) -> Result<()> {
        unsafe {
            if let Some(audio) = &mut self.audio {
                audio.seek(delta_ms, slider)?;
            }
            self.refresh_transport();
            // The next normal poll samples decoder position and its authentic
            // ayah timing; never synthesize an ayah from slider arithmetic.
            Ok(())
        }
    }
}
