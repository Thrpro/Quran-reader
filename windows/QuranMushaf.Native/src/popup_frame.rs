//! Popup content is drawn in the same parent frame as its panel. Keeping text
//! in separate child swap chains exposed an empty panel between presents.
use super::*;
impl App {
    pub(super) unsafe fn paint_popup_controls(
        &self,
        target: &windows::Win32::Graphics::Direct2D::ID2D1RenderTarget,
    ) -> Result<()> {
        unsafe {
            use crate::chrome::{Button, Icon};
            let Some(p) = &self.popup else {
                return Ok(());
            };
            let mut cursor = POINT::default();
            GetCursorPos(&mut cursor)?;
            let mut transform = windows_numerics::Matrix3x2::identity();
            target.GetTransform(&mut transform);
            let antialias = target.GetTextAntialiasMode();
            target.SetTextAntialiasMode(
                windows::Win32::Graphics::Direct2D::D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE,
            );
            let mut palette = self.palette();
            palette.background = palette.paper;
            let result = (|| {
                for id in (MENU_ROW..MENU_ROW + p.rows.len() as u32).chain([MENU_BACK, MENU_CLOSE])
                {
                    let child = self.controls[&id];
                    if !IsWindowVisible(child).as_bool() {
                        continue;
                    }
                    let Some((bounds, true)) = self.placements.borrow().get(&id).copied() else {
                        continue;
                    };
                    let row = p.rows.get(id.wrapping_sub(MENU_ROW) as usize);
                    let split = p.context && p.stack.is_empty() && id == MENU_ROW + 2;
                    let icon = if split {
                        Icon::Expand
                    } else if id == MENU_CLOSE {
                        Icon::Close
                    } else if id == MENU_BACK {
                        Icon::Back
                    } else if p.context && id == MENU_ROW + 1 {
                        Icon::Listen
                    } else if row.is_some_and(|r| r.id == STUDY) {
                        Icon::Book
                    } else if row.is_some_and(|r| r.id == BOOKMARK) {
                        Icon::Bookmark
                    } else {
                        Icon::None
                    };
                    let caption = if split {
                        ""
                    } else {
                        row.map_or("", |r| r.label.trim_start_matches('✓').trim_start())
                    };
                    let scale = self.dpi / 96.;
                    let x = (bounds.x * scale) as i32;
                    let y = (bounds.y * scale) as i32;
                    let rect = Rect {
                        x: 0.,
                        y: 0.,
                        w: (bounds.w * scale).max(1.) as i32 as f32 / scale,
                        h: (bounds.h * scale).max(1.) as i32 as f32 / scale,
                    };
                    target.SetTransform(&windows_numerics::Matrix3x2 {
                        M31: x as f32 / scale,
                        M32: y as f32 / scale,
                        ..windows_numerics::Matrix3x2::identity()
                    });
                    #[cfg(feature = "qa")]
                    let pointer_hot = self
                        .qa_menu_hot
                        .map_or_else(|| WindowFromPoint(cursor) == child, |hot| hot == child);
                    #[cfg(not(feature = "qa"))]
                    let pointer_hot = WindowFromPoint(cursor) == child;
                    self.chrome.paint_button(
                        target,
                        rect,
                        palette,
                        Button {
                            text: caption,
                            detail: row.map_or("", |r| r.detail.as_str()),
                            quran_detail: false,
                            menu: row.is_some(),
                            icon,
                            active: pointer_hot
                                || SendMessageW(child, BM_GETSTATE, None, None).0
                                    & BST_PUSHED as isize
                                    != 0,
                            disabled: !IsWindowEnabled(child).as_bool(),
                            focus: self.keyboard_focus && GetFocus() == child,
                            flat: true,
                            rtl: if row.is_some_and(|r| r.id == WELCOME_ENGLISH) {
                                false
                            } else {
                                self.prefs.arabic || row.is_some_and(|r| r.id == WELCOME_ARABIC)
                            },
                            submenu: false,
                            check: row.and_then(menu_check),
                        },
                    )?;
                }
                Ok(())
            })();
            target.SetTransform(&transform);
            target.SetTextAntialiasMode(antialias);
            result
        }
    }
}
