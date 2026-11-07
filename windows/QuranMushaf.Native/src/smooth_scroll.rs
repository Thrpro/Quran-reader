//! Pixel scrolling for the native explanation control. State stays on its HWND;
//! no text copies, worker thread, or application-state callbacks are involved.
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::SystemInformation::GetTickCount64,
        UI::{
            Controls::{EM_GETLINECOUNT, RichEdit::*},
            Shell::DefSubclassProc,
            WindowsAndMessaging::*,
        },
    },
    core::{BOOL, w},
};

const TARGET: windows::core::PCWSTR = w!("QuranMushaf.ScrollTarget");
const CURRENT: windows::core::PCWSTR = w!("QuranMushaf.ScrollCurrent");
const REMAINDER: windows::core::PCWSTR = w!("QuranMushaf.ScrollRemainder");
const TICK: windows::core::PCWSTR = w!("QuranMushaf.ScrollTick");
const TIMER: usize = 0x514d;
const SCALE: f64 = 1024.;

unsafe fn read(hwnd: HWND, key: windows::core::PCWSTR) -> Option<u64> {
    unsafe {
        (GetPropW(hwnd, key).0 as usize)
            .checked_sub(1)
            .map(|v| v as u64)
    }
}
unsafe fn write(hwnd: HWND, key: windows::core::PCWSTR, value: u64) -> bool {
    unsafe { SetPropW(hwnd, key, Some(HANDLE((value as usize + 1) as *mut _))).is_ok() }
}
pub unsafe fn stop(hwnd: HWND) {
    unsafe {
        if read(hwnd, TARGET).is_some() {
            let _ = KillTimer(Some(hwnd), TIMER);
        }
        for key in [TARGET, CURRENT, TICK, REMAINDER] {
            let _ = RemovePropW(hwnd, key);
        }
    }
}
pub unsafe fn position(hwnd: HWND) -> POINT {
    unsafe {
        let mut p = POINT::default();
        SendMessageW(
            hwnd,
            EM_GETSCROLLPOS,
            None,
            Some(LPARAM((&mut p as *mut POINT) as isize)),
        );
        p
    }
}
#[cfg(feature = "qa")]
pub unsafe fn active(hwnd: HWND) -> bool {
    unsafe { read(hwnd, TARGET).is_some() }
}

fn approach(current: f64, target: f64, milliseconds: u64) -> f64 {
    current + (target - current) * (1. - (-(milliseconds.clamp(1, 32) as f64) / 32.).exp())
}

unsafe fn frame(hwnd: HWND) {
    unsafe {
        let Some(target) = read(hwnd, TARGET).map(|v| v as f64 / SCALE) else {
            return;
        };
        let before = position(hwnd);
        let current = read(hwnd, CURRENT).map_or(before.y as f64, |v| v as f64 / SCALE);
        let now = GetTickCount64();
        let next = approach(
            current,
            target,
            now.saturating_sub(read(hwnd, TICK).unwrap_or(now)),
        );
        let desired = if (target - next).abs() < 0.5 {
            target
        } else {
            next
        };
        let p = POINT {
            x: before.x,
            y: desired.round().clamp(0., 65535.) as i32,
        };
        SendMessageW(
            hwnd,
            EM_SETSCROLLPOS,
            None,
            Some(LPARAM((&p as *const POINT) as isize)),
        );
        let after = position(hwnd);
        if p.y != before.y {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        // Native RichEdit owns document bounds. Stop immediately at its end,
        // and relinquish the timer after the last sub-pixel has settled.
        if p.y != after.y {
            stop(hwnd);
        } else if (target - desired).abs() < 0.5 {
            stop(hwnd);
            // Preserve sub-pixel wheel input across settled gestures without
            // keeping a timer alive. An external navigation/content change
            // clears this remainder along with the other scalar properties.
            let remainder = ((target - after.y as f64) * SCALE).round() as i64;
            let _ = write(hwnd, REMAINDER, (remainder + SCALE as i64) as u64);
        } else if !write(hwnd, CURRENT, (desired * SCALE) as u64) || !write(hwnd, TICK, now) {
            stop(hwnd);
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
        if message == WM_MOUSEWHEEL && wparam.0 & 0x0c == 0 {
            let mut enabled = BOOL::from(true);
            let mut lines = 3u32;
            let animated = SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some((&mut enabled as *mut BOOL).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .is_ok()
                && enabled.as_bool();
            let configured = SystemParametersInfoW(
                SPI_GETWHEELSCROLLLINES,
                0,
                Some((&mut lines as *mut u32).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .is_ok();
            if animated && configured && lines != 0 {
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let mut metrics = TEXTMETRICW::default();
                let dc = GetDC(Some(hwnd));
                if !dc.is_invalid() {
                    let font = SendMessageW(hwnd, WM_GETFONT, None, None);
                    let previous = SelectObject(dc, HGDIOBJ(font.0 as *mut _));
                    let _ = GetTextMetricsW(dc, &mut metrics);
                    SelectObject(dc, previous);
                    let _ = ReleaseDC(Some(hwnd), dc);
                }
                // RichEdit documents this pixel API as 16-bit. Very long
                // documents retain unrestricted native scrolling instead.
                let mut range = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_RANGE,
                    ..Default::default()
                };
                let _ = GetScrollInfo(hwnd, SB_VERT, &mut range);
                let rows = SendMessageW(hwnd, EM_GETLINECOUNT, None, None).0.max(0) as u64;
                if range.nMax > 55000
                    || rows
                        .saturating_mul(metrics.tmHeight.max(1) as u64)
                        .saturating_mul(2)
                        > 55000
                {
                    stop(hwnd);
                    return DefSubclassProc(hwnd, message, wparam, lparam);
                }
                let page = (rect.bottom - rect.top).max(1) as f64;
                let distance = if lines == u32::MAX {
                    page * 0.9
                } else {
                    lines.min(100) as f64
                        * (metrics.tmHeight + metrics.tmExternalLeading).max(1) as f64
                };
                let current = position(hwnd).y.max(0) as f64;
                let residual = read(hwnd, REMAINDER).map_or(0., |v| v as f64 / SCALE - 1.);
                let previous = read(hwnd, TARGET).map_or(current + residual, |v| v as f64 / SCALE);
                let delta = ((wparam.0 >> 16) as u16 as i16) as f64 / 120.;
                let target = (previous - delta * distance).clamp(
                    (current - page * 3.).max(0.),
                    (current + page * 3.).min(65535.),
                );
                let starting = read(hwnd, TARGET).is_none();
                if write(hwnd, TARGET, (target * SCALE) as u64)
                    && (!starting
                        || (write(hwnd, CURRENT, (current * SCALE) as u64)
                            && write(hwnd, TICK, GetTickCount64())))
                    && (!starting || SetTimer(Some(hwnd), TIMER, 8, None) != 0)
                {
                    return LRESULT(0);
                }
                stop(hwnd);
            }
        } else if message == WM_TIMER && wparam.0 == TIMER {
            frame(hwnd);
            return LRESULT(0);
        } else if matches!(
            message,
            WM_NCDESTROY
                | WM_SETTEXT
                | WM_SETFONT
                | WM_SIZE
                | WM_VSCROLL
                | WM_KEYDOWN
                | WM_LBUTTONDOWN
                | WM_SETTINGCHANGE
                | EM_SETPARAFORMAT
                | EM_SETCHARFORMAT
        ) || (message == WM_SHOWWINDOW && wparam.0 == 0)
        {
            stop(hwnd);
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::approach;
    #[test]
    fn settled_gestures_retain_fractional_distance() {
        let mut pixel = 0f64;
        let mut residual = 0.;
        for _ in 0..20 {
            let target = pixel + residual + 0.25;
            pixel = target.round();
            residual = target - pixel;
        }
        assert_eq!(pixel, 5.);
        assert_eq!(residual, 0.);
    }
    #[test]
    fn easing_is_bounded_and_retargets_without_overshoot() {
        let mut position = 0.;
        for _ in 0..30 {
            let next = approach(position, 100., 8);
            assert!(next >= position && next <= 100.);
            position = next;
        }
        assert!(100. - position < 0.1);
        let next = approach(position, 0., 8);
        assert!(next > 0. && next < position);
        assert_eq!(approach(50., 50., 500), 50.);
    }
}
