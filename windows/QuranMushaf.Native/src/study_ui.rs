//! Quiet Tafsir toolbar and explicit, optional recitation following.
use super::*;
impl App {
    pub(super) unsafe fn layout_study_controls(&self, x: f32, w: f32) {
        unsafe {
            let panel = x - 16.;
            let panel_w = w + 32.;
            for (id, left, width) in [
                (STUDY_FOLLOW, 92., 120.),
                (TEXT_SIZE, 220., 30.),
                (STUDY_OPTIONS, 264., 32.),
            ] {
                self.place(
                    id,
                    Rect {
                        x: panel
                            + if self.prefs.arabic {
                                panel_w - left - width
                            } else {
                                left
                            },
                        y: 66.,
                        w: width,
                        h: 30.,
                    },
                    id != STUDY_FOLLOW || self.side_playback(),
                );
            }
            let multiple = self.study_sources.len() > 1;
            self.place(
                SOURCE,
                Rect {
                    x,
                    y: 104.,
                    w,
                    h: 240.,
                },
                multiple,
            );
            self.place(
                SOURCE_SINGLE,
                Rect {
                    x,
                    y: 104.,
                    w,
                    h: 28.,
                },
                !multiple,
            );
            for (id, left) in [(AYAH_PREV, x), (AYAH_NEXT, x + w - 38.)] {
                self.place(
                    id,
                    Rect {
                        x: left,
                        y: 137.,
                        w: 38.,
                        h: 32.,
                    },
                    true,
                );
            }
            let bottom = if self.embedded_transport() {
                self.transport_top() - 38.
            } else {
                self.height - 66.
            };
            self.place(
                STUDY,
                Rect {
                    x,
                    y: 182.,
                    w,
                    h: (bottom - 182.).max(1.),
                },
                true,
            );
        }
    }

    pub(super) unsafe fn refresh_study_follow(&self) {
        unsafe {
            self.set_text(
                STUDY_FOLLOW,
                self.tr(
                    if self.prefs.follow_tafsir {
                        "Follow recitation in Tafsir: On"
                    } else {
                        "Follow recitation in Tafsir: Off"
                    },
                    if self.prefs.follow_tafsir {
                        "مزامنة التفسير مع التلاوة: مفعّلة"
                    } else {
                        "مزامنة التفسير مع التلاوة: متوقفة"
                    },
                ),
            );
        }
    }

    pub(super) unsafe fn study_options(&mut self) -> Result<()> {
        unsafe {
            let mut rows = vec![
                MenuRow {
                    id: MANAGE_TAFSIR,
                    label: self.tr("Manage tafsirs", "إدارة التفاسير").into(),
                    detail: self
                        .tr(
                            "Download or remove optional sources",
                            "تنزيل التفاسير الإضافية أو إزالتها",
                        )
                        .into(),
                    children: vec![],
                },
                MenuRow {
                    id: TEXT_SIZE,
                    label: self.tr("Text size", "حجم النص").into(),
                    detail: String::new(),
                    children: vec![],
                },
            ];
            if self.audio.as_ref().is_some_and(Audio::has_session) {
                rows.push(MenuRow {
                    id: STUDY_FOLLOW,
                    label: format!(
                        "{}{}",
                        if self.prefs.follow_tafsir { "✓ " } else { "" },
                        self.tr("Follow recitation in Tafsir", "مزامنة التفسير مع التلاوة")
                    ),
                    detail: self
                        .tr(
                            "Manual browsing turns following off",
                            "يتوقف التزامن عند اختيار آية يدويًا",
                        )
                        .into(),
                    children: vec![],
                });
            }
            let anchor = self.placements.borrow()[&STUDY_OPTIONS].0;
            self.show_popup(
                self.tr("Reading options", "خيارات القراءة").into(),
                rows,
                if self.prefs.arabic {
                    anchor.x + anchor.w
                } else {
                    anchor.x
                },
                anchor.y + anchor.h + 4.,
                false,
            )
        }
    }

    pub(super) unsafe fn toggle_study_follow(&mut self) -> Result<()> {
        unsafe {
            self.prefs.follow_tafsir = !self.prefs.follow_tafsir;
            self.dirty = true;
            if self.prefs.follow_tafsir {
                self.playback.begin();
                let key = self.audio.as_ref().and_then(Audio::current);
                self.follow_recitation(key)?;
            }
            self.refresh_study_follow();
            self.resize();
            self.invalidate();
            Ok(())
        }
    }

    pub(super) unsafe fn reveal_recitation(&mut self) -> Result<()> {
        unsafe {
            let follow = self.prefs.follow_tafsir;
            self.manual_navigation();
            self.prefs.follow_tafsir = follow;
            self.refresh_study_follow();
            self.playback.begin();
            let key = self.audio.as_ref().and_then(|audio| {
                audio
                    .current()
                    .or_else(|| audio.opening_surah().map(|surah| (surah, 1)))
            });
            if let Some(key) = key {
                if let Some(hit) = self
                    .resources
                    .index()?
                    .iter()
                    .find(|h| (h.surah, h.ayah) == key)
                {
                    self.prefs.page = hit.page_number;
                    self.dirty = true;
                }
            }
            self.follow_recitation(key)?;
            self.resize();
            self.invalidate_reader();
            Ok(())
        }
    }

    pub(super) fn paint_study_legend(
        &self,
        target: &windows::Win32::Graphics::Direct2D::ID2D1RenderTarget,
        body: Rect,
        palette: Palette,
    ) -> Result<bool> {
        if self
            .canvas_recitation()
            .is_none_or(|key| Some(key) == self.selected)
        {
            return Ok(false);
        }
        unsafe {
            let filled = target.CreateSolidColorBrush(&palette.highlight, None)?;
            let edge = target.CreateSolidColorBrush(&palette.muted, None)?;
            let y = body.y + body.h + 8.;
            for (left, label, fill) in [
                (0., self.tr("Tafsir", "التفسير"), true),
                (94., self.tr("Recitation", "التلاوة"), false),
            ] {
                let x = if self.prefs.arabic {
                    body.x + body.w - left - 10.
                } else {
                    body.x + left
                };
                let frame = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                    rect: Rect {
                        x,
                        y: y + 5.,
                        w: 9.,
                        h: 9.,
                    }
                    .d2d(),
                    radiusX: 2.,
                    radiusY: 2.,
                };
                if fill {
                    target.FillRoundedRectangle(&frame, &filled);
                } else {
                    target.DrawRoundedRectangle(&frame, &edge, 1.25, None);
                }
                self.renderer.label_start(
                    target,
                    label,
                    Rect {
                        x: if self.prefs.arabic { x - 76. } else { x + 14. },
                        y,
                        w: 72.,
                        h: 18.,
                    },
                    11.,
                    palette.muted,
                    self.prefs.arabic,
                )?;
            }
            if self.study_overflows() {
                self.renderer.label_start(
                    target,
                    self.tr("Scroll", "مرّر"),
                    Rect {
                        x: if self.prefs.arabic {
                            body.x
                        } else {
                            body.x + body.w - 86.
                        },
                        y,
                        w: 86.,
                        h: 18.,
                    },
                    11.,
                    palette.muted,
                    self.prefs.arabic,
                )?;
            }
        }
        Ok(true)
    }
}
