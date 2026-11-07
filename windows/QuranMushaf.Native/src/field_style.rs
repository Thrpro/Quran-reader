//! Shared presentation metrics for native fields; Windows retains their input,
//! IME, keyboard navigation and accessibility behavior.
use crate::{
    data::Result,
    render::{Palette, color_rgb},
};
use std::cell::Cell;
use windows::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{Controls::*, WindowsAndMessaging::*},
};

pub(crate) const HORIZONTAL_PADDING: f32 = 10.;
pub(crate) const CORNER_RADIUS: f32 = 6.;
pub(crate) const BORDER_WIDTH: f32 = 1.;
pub(crate) const COMBO_SELECTION_HEIGHT: f32 = 28.;
const COMBO_TRAILING_SPACE: f32 = 32.;

#[derive(Clone, Copy)]
pub(crate) struct Metrics {
    scale: f32,
}
impl Metrics {
    pub(crate) fn new(dpi: f32) -> Self {
        Self {
            scale: if dpi.is_finite() {
                dpi.max(96.) / 96.
            } else {
                1.
            },
        }
    }
    pub(crate) fn px(self, dip: f32) -> i32 {
        (dip * self.scale).round() as i32
    }
    pub(crate) fn horizontal_padding(self) -> i32 {
        self.px(HORIZONTAL_PADDING)
    }
    pub(crate) fn combo_selection_height(self) -> i32 {
        self.px(COMBO_SELECTION_HEIGHT)
    }
    /// The native arrow stays on its native side. RTL affects the text reading
    /// direction, rather than reflecting a second independently painted bitmap.
    pub(crate) fn combo_label_rect(self, rect: RECT) -> RECT {
        let left = rect.left + self.horizontal_padding();
        RECT {
            left,
            top: rect.top,
            right: (rect.right - self.px(COMBO_TRAILING_SPACE)).max(left),
            bottom: rect.bottom,
        }
    }
    pub(crate) fn combo_chevron_rect(self, rect: RECT) -> RECT {
        RECT {
            left: (rect.right - self.px(30.)).max(rect.left),
            top: rect.top + self.px(2.),
            right: rect.right - self.px(2.),
            bottom: rect.bottom - self.px(2.),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FrameColors {
    pub(crate) border: COLORREF,
    pub(crate) focus: COLORREF,
}
thread_local! {
    // Only copied colors, so synchronous native paint callbacks never borrow App.
    static FRAME_COLORS: Cell<Option<FrameColors>> = const { Cell::new(None) };
}
fn blend(a: COLORREF, b: COLORREF, amount: f32) -> COLORREF {
    let channel = |shift: u32| {
        let a = ((a.0 >> shift) & 255u32) as f32;
        let b = ((b.0 >> shift) & 255u32) as f32;
        (a + (b - a) * amount).round() as u32
    };
    COLORREF(channel(0) | channel(8) << 8 | channel(16) << 16)
}
/// Call beside the UI's native-control color cache, including theme changes.
pub(crate) fn cache_palette(palette: Palette) {
    FRAME_COLORS.with(|colors| {
        colors.set(Some(FrameColors {
            border: COLORREF(color_rgb(palette.border)),
            focus: blend(
                COLORREF(color_rgb(palette.muted)),
                COLORREF(palette.ink_rgb()),
                0.22,
            ),
        }))
    });
}
pub(crate) fn frame_colors(background: COLORREF, ink: COLORREF) -> FrameColors {
    FRAME_COLORS.with(|colors| {
        colors.get().unwrap_or_else(|| FrameColors {
            border: blend(background, ink, 0.22),
            focus: blend(background, ink, 0.55),
        })
    })
}

/// Reapply after creation and DPI changes. This changes field metrics only,
/// without replacing the native caret, selection, keyboard or IME handling.
pub(crate) unsafe fn apply_edit_metrics(hwnd: HWND, dpi: f32) {
    unsafe {
        let margin = Metrics::new(dpi)
            .horizontal_padding()
            .clamp(0, u16::MAX as i32) as u32;
        SendMessageW(
            hwnd,
            EM_SETMARGINS,
            Some(WPARAM(3)), // EC_LEFTMARGIN | EC_RIGHTMARGIN.
            Some(LPARAM((margin | margin << 16) as isize)),
        );
    }
}
pub(crate) unsafe fn apply_combo_metrics(hwnd: HWND, dpi: f32) {
    unsafe {
        let height = Metrics::new(dpi).combo_selection_height() as isize;
        if SendMessageW(hwnd, CB_GETITEMHEIGHT, Some(WPARAM(usize::MAX)), None).0 != height {
            SendMessageW(
                hwnd,
                CB_SETITEMHEIGHT,
                Some(WPARAM(usize::MAX)), // -1 targets the collapsed selection field.
                Some(LPARAM(height)),
            );
        }
        if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & 3 == CBS_DROPDOWN as u32 {
            let mut info = COMBOBOXINFO {
                cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                ..Default::default()
            };
            if GetComboBoxInfo(hwnd, &mut info).is_ok() && !info.hwndItem.is_invalid() {
                apply_edit_metrics(info.hwndItem, dpi);
            }
        }
    }
}
/// Safe to call for a mixed list of UI child HWNDs; other classes are unchanged.
pub(crate) unsafe fn apply_field_metrics(hwnd: HWND, dpi: f32) {
    unsafe {
        let mut name = [0u16; 16];
        let length = GetClassNameW(hwnd, &mut name).max(0) as usize;
        let name = String::from_utf16_lossy(&name[..length.min(name.len())]);
        if name.eq_ignore_ascii_case("COMBOBOX") {
            apply_combo_metrics(hwnd, dpi);
        } else if name.eq_ignore_ascii_case("EDIT") {
            apply_edit_metrics(hwnd, dpi);
        }
    }
}

/// Paint a one-DIP rounded outline into a native or snapshot DC. The caller
/// owns the background/text. SaveDC preserves native clipping and selected font.
pub(crate) unsafe fn draw_frame(
    dc: HDC,
    rect: RECT,
    dpi: f32,
    colors: FrameColors,
    focused: bool,
) -> Result<()> {
    unsafe {
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return Ok(());
        }
        let saved = SaveDC(dc);
        if saved == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let _state = SavedDc { dc, saved };
        let metrics = Metrics::new(dpi);
        let stroke = metrics.px(BORDER_WIDTH).max(1);
        let pen = CreatePen(
            PS_SOLID,
            stroke,
            if focused { colors.focus } else { colors.border },
        );
        if pen.is_invalid() {
            return Err(windows::core::Error::from_thread().into());
        }
        let old_pen = SelectObject(dc, pen.into());
        let old_brush = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
        let inset = stroke / 2;
        // GDI takes the corner ellipse's diameter; Direct2D takes its radius.
        let diameter = metrics.px(CORNER_RADIUS * 2.).max(1);
        let result = RoundRect(
            dc,
            rect.left + inset,
            rect.top + inset,
            rect.right - inset,
            rect.bottom - inset,
            diameter,
            diameter,
        )
        .ok();
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        let _ = DeleteObject(pen.into());
        result?;
        Ok(())
    }
}
struct SavedDc {
    dc: HDC,
    saved: i32,
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        unsafe {
            let _ = RestoreDC(self.dc, self.saved);
        }
    }
}
