//! Presentation adapter retaining the Windows combo's input and accessibility.
use std::cell::Cell;
thread_local! { static APPLYING_METRICS: Cell<bool> = const { Cell::new(false) }; }
use windows::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{
        Controls::*, HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::GetFocus,
        Shell::DefSubclassProc, WindowsAndMessaging::*,
    },
};
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
        if matches!(
            message,
            WM_WINDOWPOSCHANGED | WM_SETFONT | WM_STYLECHANGED | WM_THEMECHANGED
        ) && !APPLYING_METRICS.with(|guard| guard.replace(true))
        {
            // Windows recalculates a native dropdown's collapsed text height
            // during placement/style changes. Restore the shared DIP height
            // after that work, rather than only before the first layout.
            crate::field_style::apply_combo_metrics(hwnd, GetDpiForWindow(hwnd).max(96) as f32);
            APPLYING_METRICS.with(|guard| guard.set(false));
        }
        if matches!(
            message,
            WM_PAINT | WM_SETFOCUS | WM_KILLFOCUS | WM_LBUTTONUP
        ) {
            paint(hwnd);
        }
        result
    }
}
unsafe fn paint(hwnd: HWND) {
    unsafe {
        let dc = GetDC(Some(hwnd));
        if dc.is_invalid() {
            return;
        }
        let _ = paint_to_dc(hwnd, dc);
        let _ = ReleaseDC(Some(hwnd), dc);
    }
}

/// Apply the existing native combo presentation to a caller-owned DC.
/// Use this after WM_PRINT paints the native text in a motion snapshot.
pub(crate) unsafe fn paint_to_dc(hwnd: HWND, dc: HDC) -> crate::data::Result<()> {
    unsafe {
        let saved = SaveDC(dc);
        if saved == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let _state = SavedDc { dc, saved };
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect)?;
        {
            let brush = HBRUSH(
                SendMessageW(
                    GetParent(hwnd).unwrap_or_default(),
                    WM_CTLCOLORSTATIC,
                    Some(WPARAM(dc.0 as usize)),
                    Some(LPARAM(hwnd.0 as isize)),
                )
                .0 as *mut _,
            );
            let ink = GetTextColor(dc);
            let background = GetBkColor(dc);
            let mut info = COMBOBOXINFO {
                cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                ..Default::default()
            };
            let editable = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & 3 == CBS_DROPDOWN as u32;
            if editable && GetComboBoxInfo(hwnd, &mut info).is_ok() {
                ExcludeClipRect(
                    dc,
                    info.rcItem.left,
                    info.rcItem.top,
                    info.rcItem.right,
                    info.rcItem.bottom,
                );
            }
            if !brush.is_invalid() {
                FillRect(dc, &rect, brush);
            }
            let dpi = GetDpiForWindow(hwnd).max(96) as f32;
            let metrics = crate::field_style::Metrics::new(dpi);
            if !editable {
                // Bounded stack storage avoids allocating on every native paint.
                let mut text = [0u16; 2049];
                let count = GetWindowTextW(hwnd, &mut text);
                let text = &mut text[..count.max(0) as usize];
                let font = SendMessageW(hwnd, WM_GETFONT, None, None);
                SelectObject(dc, HGDIOBJ(font.0 as *mut _));
                SetBkMode(dc, TRANSPARENT);
                let mut label = metrics.combo_label_rect(rect);
                DrawTextW(
                    dc,
                    text,
                    &mut label,
                    DT_SINGLELINE
                        | DT_VCENTER
                        | DT_END_ELLIPSIS
                        | DT_NOPREFIX
                        | if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_RTLREADING.0 != 0 {
                            DT_RTLREADING | DT_RIGHT
                        } else {
                            DT_LEFT
                        },
                );
            }
            crate::control_vectors::chevron(
                dc,
                metrics.combo_chevron_rect(rect),
                dpi,
                background,
                ink,
            )?;
            // The chevron's DC surface clears its own background. Draw the
            // shared outline last so its rounded corners remain intact.
            let focus = GetFocus();
            crate::field_style::draw_frame(
                dc,
                rect,
                dpi,
                crate::field_style::frame_colors(background, ink),
                focus == hwnd || (editable && focus == info.hwndItem),
            )?;
        }
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
