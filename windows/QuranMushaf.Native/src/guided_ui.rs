//! Windows presentation of the portable guide. A modeless overlay keeps audio
//! polling alive and demonstrates the same controls used during normal reading.
use super::*;
pub(super) struct State {
    pub(super) tour: crate::guided_tour::Tour,
    page: u32,
    selected: Option<(u32, u32)>,
    panel: Panel,
    text_view: bool,
}
impl App {
    pub(super) unsafe fn create_guide_controls(&mut self) -> Result<()> {
        unsafe {
            for id in [GUIDE_BACK, GUIDE_NEXT, GUIDE_CLOSE, GUIDE_SHOW] {
                self.child(id, "BUTTON", "", BS_OWNERDRAW as u32)?;
            }
            self.child(GUIDE_BODY, "STATIC", "", 0)?;
            Ok(())
        }
    }
    fn guide_rect(&self) -> Rect {
        let w = (self.width - 32.).clamp(260., 600.);
        let h = if w < 460. { 270. } else { 224. };
        Rect {
            x: (self.width - w) / 2.,
            y: (self.height - h - 16.).max(42.),
            w,
            h,
        }
    }
    pub(super) unsafe fn start_guide(&mut self) -> Result<()> {
        unsafe {
            if self.guide.is_none() {
                self.guide = Some(State {
                    tour: crate::guided_tour::Tour::default(),
                    page: self.prefs.page,
                    selected: self.selected,
                    panel: self.panel,
                    text_view: self.text_view,
                });
            }
            self.text_view = false;
            self.preview_guide()?;
            focus_window(self.controls[&GUIDE_NEXT])?;
            Ok(())
        }
    }
    pub(super) unsafe fn close_guide(&mut self) -> Result<()> {
        unsafe {
            let Some(state) = self.guide.take() else {
                return Ok(());
            };
            self.popup = None;
            self.search = false;
            self.search_animation = None;
            KillTimer(Some(self.hwnd), 8).ok();
            if self.prefs.page == state.page {
                self.selected = state.selected;
                self.set_panel(state.panel);
                if state.panel == Panel::Study {
                    self.study()?;
                }
            } else {
                // A real search during the guide is an intentional navigation.
                self.set_panel(Panel::None);
                self.selected = None;
            }
            self.reader.onboarding_completed = true;
            self.text_view = state.text_view;
            self.reader_dirty = true;
            self.resize();
            focus_window(self.hwnd)?;
            Ok(())
        }
    }
    pub(super) unsafe fn guide_command(&mut self, id: u32) -> Result<()> {
        unsafe {
            if id == GUIDE_CLOSE {
                return self.close_guide();
            }
            let Some(state) = &mut self.guide else {
                return Ok(());
            };
            match id {
                GUIDE_BACK => state.tour.back(),
                GUIDE_NEXT if !state.tour.advance() => return self.close_guide(),
                _ => {}
            }
            self.preview_guide()?;
            focus_window(self.controls[&GUIDE_NEXT])?;
            Ok(())
        }
    }
    pub(super) unsafe fn guide_key(&mut self, key: u16, focus: HWND) -> Result<bool> {
        unsafe {
            if self.guide.is_none() {
                return Ok(false);
            }
            let ids = [GUIDE_BACK, GUIDE_SHOW, GUIDE_NEXT, GUIDE_CLOSE];
            let Some(index) = ids.iter().position(|id| self.controls[id] == focus) else {
                return Ok(false);
            };
            if key == 13 {
                self.guide_command(ids[index])?;
                return Ok(true);
            }
            if key == 9 {
                let backwards = GetKeyState(VK_SHIFT.0 as i32) < 0;
                for offset in 1..=ids.len() {
                    let next = if backwards {
                        (index + ids.len() - offset) % ids.len()
                    } else {
                        (index + offset) % ids.len()
                    };
                    let handle = self.controls[&ids[next]];
                    if IsWindowEnabled(handle).as_bool() {
                        focus_window(handle)?;
                        break;
                    }
                }
                return Ok(true);
            }
            Ok(matches!(key, 37..=40))
        }
    }
    unsafe fn preview_guide(&mut self) -> Result<()> {
        unsafe {
            let Some(state) = &self.guide else {
                return Ok(());
            };
            let step = state.tour.step();
            self.popup = None;
            self.search = false;
            self.search_animation = None;
            KillTimer(Some(self.hwnd), 8).ok();
            match step {
                crate::guided_tour::Step::Search => {
                    self.command(SEARCH, 0)?;
                }
                crate::guided_tour::Step::AyahMenu | crate::guided_tour::Step::Tafsir => {
                    self.set_panel(Panel::None);
                    let page = self.resources.page(self.prefs.page)?;
                    let key = page
                        .lines
                        .iter()
                        .find_map(|line| line.words.first().map(|word| (word.surah, word.ayah)))
                        .ok_or("No ayah on the current page")?;
                    self.selected = Some(key);
                    if step == crate::guided_tour::Step::Tafsir {
                        // Demonstrating a panel is not a deliberate reading
                        // navigation. Preserve live following; real user
                        // commands inside the guide keep their normal effect.
                        self.set_panel(Panel::Study);
                        self.study()?;
                    } else {
                        self.resize();
                        self.context_at(key, (self.width / 2. - 175.).max(16.), 90., false)?;
                    }
                }
                crate::guided_tour::Step::Listening => self.open_listen(),
                crate::guided_tour::Step::Options => {
                    self.set_panel(Panel::None);
                    self.options()?;
                }
            }
            self.resize();
            Ok(())
        }
    }
    pub(super) unsafe fn layout_guide(&self) {
        unsafe {
            let Some(state) = &self.guide else {
                return;
            };
            let r = self.guide_rect();
            // HWND children are composed above the parent. Occlude overlapping
            // demo controls just as popovers do; resize restores them on exit.
            let hidden: Vec<_> = self
                .placements
                .borrow()
                .iter()
                .filter_map(|(&id, &(bounds, visible))| {
                    let visible_height = if matches!(
                        id,
                        SOURCE | RECITER | CHAPTER | SPEED | REPEAT | BROWSE_KIND | THEME_SELECTOR
                    ) {
                        34.
                    } else {
                        bounds.h
                    };
                    (visible
                        && !(GUIDE_BACK..=GUIDE_BODY).contains(&id)
                        && bounds.x < r.x + r.w
                        && bounds.x + bounds.w > r.x
                        && bounds.y < r.y + r.h
                        && bounds.y + visible_height > r.y)
                        .then_some((id, bounds))
                })
                .collect();
            for (id, mut bounds) in hidden {
                if matches!(id, RESULTS | STUDY | TAFSIR_INFO | TEXT_VIEW) && bounds.y + 40. < r.y {
                    // Keep the actual document/list visible above the guide.
                    bounds.h = r.y - bounds.y - 8.;
                    self.place(id, bounds, true);
                    if id == STUDY {
                        let mut client = RECT::default();
                        GetClientRect(self.controls[&id], &mut client).ok();
                        let scale = self.dpi / 96.;
                        let rect = RECT {
                            left: (16. * scale) as i32,
                            top: (12. * scale) as i32,
                            right: (client.right - (16. * scale) as i32).max((48. * scale) as i32),
                            bottom: (client.bottom - (12. * scale) as i32)
                                .max((40. * scale) as i32),
                        };
                        SendMessageW(
                            self.controls[&id],
                            EM_SETRECT,
                            None,
                            Some(LPARAM((&rect as *const RECT) as isize)),
                        );
                        self.study_format_rect.set(Some(rect));
                    }
                } else {
                    self.place(id, bounds, false);
                }
            }
            let (_, body) = state.tour.text(self.prefs.arabic);
            self.set_text(GUIDE_BODY, body);
            let handle = self.controls[&GUIDE_BODY];
            let style = GetWindowLongPtrW(handle, GWL_STYLE);
            SetWindowLongPtrW(
                handle,
                GWL_STYLE,
                (style & !3) | if self.prefs.arabic { 2 } else { 0 },
            );
            SetWindowLongPtrW(
                handle,
                GWL_EXSTYLE,
                if self.prefs.arabic {
                    WS_EX_RTLREADING.0 as isize
                } else {
                    0
                },
            );
            self.place(
                GUIDE_BODY,
                Rect {
                    x: r.x + 20.,
                    y: r.y + 60.,
                    w: r.w - 40.,
                    h: r.h - 116.,
                },
                true,
            );
            let gap = 8.;
            let button_w = (r.w - 40. - gap * 2.) / 3.;
            for (i, (id, text)) in [
                (GUIDE_BACK, self.tr("Back", "السابق")),
                (GUIDE_SHOW, self.tr("Show again", "عرض مجددًا")),
                (
                    GUIDE_NEXT,
                    if state.tour.index() + 1 == crate::guided_tour::Tour::COUNT {
                        self.tr("Finish", "إنهاء")
                    } else {
                        self.tr("Next", "التالي")
                    },
                ),
            ]
            .into_iter()
            .enumerate()
            {
                self.set_text(id, text);
                self.place(
                    id,
                    Rect {
                        x: r.x
                            + 20.
                            + (if self.prefs.arabic { 2 - i } else { i }) as f32 * (button_w + gap),
                        y: r.y + r.h - 48.,
                        w: button_w,
                        h: 32.,
                    },
                    true,
                );
            }
            let _ = EnableWindow(self.controls[&GUIDE_BACK], state.tour.index() != 0);
            self.set_text(GUIDE_CLOSE, self.tr("Close guide", "إغلاق الدليل"));
            self.place(
                GUIDE_CLOSE,
                Rect {
                    x: r.x + r.w - 44.,
                    y: r.y + 10.,
                    w: 32.,
                    h: 32.,
                },
                true,
            );
        }
    }
    pub(super) unsafe fn paint_guide(
        &self,
        target: &windows::Win32::Graphics::Direct2D::ID2D1RenderTarget,
        palette: Palette,
    ) -> Result<()> {
        unsafe {
            let Some(state) = &self.guide else {
                return Ok(());
            };
            let r = self.guide_rect();
            let surface = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                rect: r.d2d(),
                radiusX: 12.,
                radiusY: 12.,
            };
            let background = target.CreateSolidColorBrush(&palette.background, None)?;
            let border = target.CreateSolidColorBrush(&palette.muted, None)?;
            target.FillRoundedRectangle(&surface, &background);
            target.DrawRoundedRectangle(&surface, &border, 1., None);
            let (title, _) = state.tour.text(self.prefs.arabic);
            self.renderer.label_start(
                target,
                &format!(
                    "{} / {} · {title}",
                    state.tour.index() + 1,
                    crate::guided_tour::Tour::COUNT
                ),
                Rect {
                    x: r.x + 20.,
                    y: r.y + 14.,
                    w: r.w - 76.,
                    h: 32.,
                },
                16.,
                palette.ink,
                self.prefs.arabic,
            )?;
            Ok(())
        }
    }
}
