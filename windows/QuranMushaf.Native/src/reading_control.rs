//! Windows text-control adapter: private authentic font and read-only caret.
use crate::data::Result;
use std::{io::Read, path::Path};
use windows::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{
        Controls::{
            EM_GETRECT, EM_POSFROMCHAR, EM_SETSEL,
            RichEdit::{EM_EXSETSEL, EM_SETCHARFORMAT, EM_SETPARAFORMAT},
        },
        HiDpi::GetDpiForWindow,
        Shell::DefSubclassProc,
        WindowsAndMessaging::*,
    },
};

// Store scalar presentation metadata on this HWND; no pointer or content is
// shared across threads. The rule follows its attribution paragraph as RichEdit
// wraps and scrolls, instead of being a fixed footer or literal text glyphs.
const RULE_INDEX: windows::core::PCWSTR = windows::core::w!("QuranMushaf.AttributionIndex");
const RULE_COLOR: windows::core::PCWSTR = windows::core::w!("QuranMushaf.AttributionColor");

pub unsafe fn set_attribution_rule(hwnd: HWND, index: u32, color: COLORREF) {
    unsafe {
        let wanted_index = index as usize + 1;
        let wanted_color = color.0 as usize + 1;
        if GetPropW(hwnd, RULE_INDEX).0 as usize == wanted_index
            && GetPropW(hwnd, RULE_COLOR).0 as usize == wanted_color
        {
            return;
        }
        // A failed property allocation omits this decorative rule; reading and
        // selection still use the complete original plain text.
        if SetPropW(hwnd, RULE_INDEX, Some(HANDLE(wanted_index as *mut _))).is_ok()
            && SetPropW(hwnd, RULE_COLOR, Some(HANDLE(wanted_color as *mut _))).is_ok()
        {
            let _ = InvalidateRect(Some(hwnd), None, false);
        } else {
            let _ = RemovePropW(hwnd, RULE_INDEX);
            let _ = RemovePropW(hwnd, RULE_COLOR);
        }
    }
}

unsafe fn attribution_rule_geometry(hwnd: HWND) -> Option<(RECT, i32, COLORREF)> {
    unsafe {
        let index = GetPropW(hwnd, RULE_INDEX).0 as usize;
        let color = GetPropW(hwnd, RULE_COLOR).0 as usize;
        if index == 0 || color == 0 {
            return None;
        }
        let mut point = POINT {
            x: i32::MIN,
            y: i32::MIN,
        };
        SendMessageW(
            hwnd,
            EM_POSFROMCHAR,
            Some(WPARAM((&mut point as *mut POINT) as usize)),
            Some(LPARAM((index - 1) as isize)),
        );
        if point.y == i32::MIN {
            return None;
        }
        let mut rect = RECT::default();
        SendMessageW(
            hwnd,
            EM_GETRECT,
            None,
            Some(LPARAM((&mut rect as *mut RECT) as isize)),
        );
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.;
        let y = point.y.saturating_sub((9. * scale).round() as i32);
        if y < rect.top || y >= rect.bottom || rect.right <= rect.left {
            return None;
        }
        Some((rect, y, COLORREF((color - 1) as u32)))
    }
}

unsafe fn draw_attribution_rule(hwnd: HWND, dc: HDC) {
    unsafe {
        let Some((rect, y, color)) = attribution_rule_geometry(hwnd) else {
            return;
        };
        let pen = CreatePen(PS_SOLID, 1, color);
        if pen.is_invalid() {
            return;
        }
        let old = SelectObject(dc, pen.into());
        let _ = MoveToEx(dc, rect.left, y, None);
        let _ = LineTo(dc, rect.right, y);
        SelectObject(dc, old);
        let _ = DeleteObject(pen.into());
    }
}

#[cfg(feature = "qa")]
pub unsafe fn visible_attribution_rule(hwnd: HWND) -> Option<(RECT, i32, COLORREF)> {
    unsafe { attribution_rule_geometry(hwnd) }
}

pub struct PrivateFont {
    handle: HANDLE,
    _bytes: Vec<u8>,
}
impl PrivateFont {
    pub fn load(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() > 2 * 1024 * 1024 {
            return Err("Original study font exceeds its read limit".into());
        }
        let mut count = 0u32;
        let handle = unsafe {
            AddFontMemResourceEx(
                bytes.as_ptr().cast(),
                bytes.len() as u32,
                None,
                &raw mut count,
            )
        };
        if handle.is_invalid() || count == 0 {
            if !handle.is_invalid() {
                unsafe {
                    let _ = RemoveFontMemResourceEx(handle);
                }
            }
            return Err("Unable to register original study font".into());
        }
        Ok(Self {
            handle,
            _bytes: bytes,
        })
    }
}
impl Drop for PrivateFont {
    fn drop(&mut self) {
        unsafe {
            let _ = RemoveFontMemResourceEx(self.handle);
        }
    }
}
pub unsafe fn hide_caret(hwnd: HWND) {
    unsafe {
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        // Read-only views need selection and focus, but no insertion caret.
        // Remove only this control's thread-owned caret, including its off
        // blink phase. HideCaret's visibility bit cannot distinguish that
        // phase from a hidden caret and repeated calls accumulate a counter.
        if GetGUIThreadInfo(0, &mut info).is_ok() && info.hwndCaret == hwnd {
            let _ = DestroyCaret();
        }
    }
}

/// Publish one complete text/format/scroll update to a visible native reader.
/// Hidden controls need no freeze and must not receive WM_SETREDRAW(TRUE):
/// DefWindowProc would add WS_VISIBLE and expose a closed study panel.
#[must_use]
pub struct RedrawBatch {
    hwnd: HWND,
    was_visible: bool,
}
impl RedrawBatch {
    /// The window must stay alive on this UI thread until the guard is dropped.
    pub unsafe fn begin(hwnd: HWND) -> Self {
        unsafe {
            let was_visible = GetWindowLongPtrW(hwnd, GWL_STYLE) & WS_VISIBLE.0 as isize != 0;
            if was_visible {
                SendMessageW(hwnd, WM_SETREDRAW, Some(WPARAM(0)), None);
            }
            Self { hwnd, was_visible }
        }
    }
}
impl Drop for RedrawBatch {
    fn drop(&mut self) {
        unsafe {
            if self.was_visible && IsWindow(Some(self.hwnd)).as_bool() {
                SendMessageW(self.hwnd, WM_SETREDRAW, Some(WPARAM(1)), None);
                hide_caret(self.hwnd);
                // Include the nonclient scrollbar in the single final update.
                // Leave dispatch deferred, so no intermediate format can paint.
                let _ = RedrawWindow(
                    Some(self.hwnd),
                    None,
                    None,
                    RDW_ERASE | RDW_FRAME | RDW_INVALIDATE | RDW_ALLCHILDREN,
                );
            }
        }
    }
}

pub unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        if message == WM_NCDESTROY {
            let _ = RemovePropW(hwnd, RULE_INDEX);
            let _ = RemovePropW(hwnd, RULE_COLOR);
        } else if message == WM_PAINT {
            let dc = GetDC(Some(hwnd));
            if !dc.is_invalid() {
                draw_attribution_rule(hwnd, dc);
                ReleaseDC(Some(hwnd), dc);
            }
        } else if message == WM_PRINTCLIENT {
            draw_attribution_rule(hwnd, HDC(wparam.0 as *mut _));
        } else if matches!(message, WM_VSCROLL | WM_MOUSEWHEEL)
            && GetPropW(hwnd, RULE_INDEX).0 as usize != 0
        {
            // A native scroll can move retained pixels. Repaint the decorative
            // rule from the new character position with the following text frame.
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        if matches!(
            message,
            WM_SETFOCUS
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_KEYDOWN
                | WM_CHAR
                | WM_PAINT
                | WM_SETTEXT
                | WM_SIZE
                | WM_VSCROLL
                | WM_MOUSEWHEEL
                | EM_SETSEL
                | EM_EXSETSEL
                | EM_SETCHARFORMAT
                | EM_SETPARAFORMAT
        ) {
            hide_caret(hwnd);
        }
        result
    }
}
