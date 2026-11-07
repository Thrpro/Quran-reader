//! Antialiased control decoration; Windows retains input and accessibility.
use crate::data::Result;
use std::cell::RefCell;
use windows::Win32::{
    Foundation::*,
    Graphics::{
        Direct2D::{Common::*, *},
        Dxgi::Common::*,
        Gdi::*,
    },
    UI::{
        Controls::*, HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::GetFocus,
        Shell::DefSubclassProc, WindowsAndMessaging::*,
    },
};
use windows_numerics::Vector2;
thread_local! { static TARGET: RefCell<Option<ID2D1DCRenderTarget>> = const { RefCell::new(None) }; }
/// Matched QA experiment for the small GDI-bound control surfaces only.
pub(crate) fn dc_target_type() -> D2D1_RENDER_TARGET_TYPE {
    #[cfg(feature = "qa")]
    if std::env::args().any(|arg| arg == "--qa-default-chrome") {
        return D2D1_RENDER_TARGET_TYPE_DEFAULT;
    }
    D2D1_RENDER_TARGET_TYPE_SOFTWARE
}
/// Release cached Windows graphics interfaces before the COM apartment closes.
pub fn shutdown() {
    TARGET.with(|cache| {
        cache.borrow_mut().take();
    });
}
fn color(rgb: COLORREF) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (rgb.0 & 255) as f32 / 255.,
        g: ((rgb.0 >> 8) & 255) as f32 / 255.,
        b: ((rgb.0 >> 16) & 255) as f32 / 255.,
        a: 1.,
    }
}
unsafe fn draw(
    dc: HDC,
    rect: RECT,
    dpi: f32,
    painter: impl FnOnce(&ID2D1DCRenderTarget, f32, f32) -> Result<()>,
) -> Result<()> {
    unsafe {
        TARGET.with(|cache| {
            let mut cached = cache
                .try_borrow_mut()
                .map_err(|_| "Reentrant control decoration")?;
            if cached.is_none() {
                let factory: ID2D1Factory =
                    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
                *cached = Some(
                    factory.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                        r#type: dc_target_type(),
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_IGNORE,
                        },
                        ..Default::default()
                    })?,
                );
            }
            let target = cached.as_ref().unwrap();
            target.BindDC(dc, &rect)?;
            target.SetDpi(dpi, dpi);
            target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            target.BeginDraw();
            let painted = painter(
                target,
                (rect.right - rect.left) as f32 * 96. / dpi,
                (rect.bottom - rect.top) as f32 * 96. / dpi,
            );
            let ended = target.EndDraw(None, None);
            if ended.is_err() {
                *cached = None;
            }
            painted?;
            ended?;
            Ok(())
        })
    }
}
pub unsafe fn chevron(
    dc: HDC,
    rect: RECT,
    dpi: f32,
    background: COLORREF,
    ink: COLORREF,
) -> Result<()> {
    unsafe {
        draw(dc, rect, dpi, |target, width, height| {
            target.Clear(Some(&color(background)));
            let brush = target.CreateSolidColorBrush(&color(ink), None)?;
            let factory = target.GetFactory()?;
            let style = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )?;
            let x = width / 2.;
            let y = height / 2.;
            target.DrawLine(
                Vector2 {
                    X: x - 4.,
                    Y: y - 2.,
                },
                Vector2 { X: x, Y: y + 2. },
                &brush,
                1.5,
                &style,
            );
            target.DrawLine(
                Vector2 { X: x, Y: y + 2. },
                Vector2 {
                    X: x + 4.,
                    Y: y - 2.,
                },
                &brush,
                1.5,
                &style,
            );
            Ok(())
        })
    }
}
pub unsafe extern "system" fn slider_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        // Mirroring handles pointer coordinates, but comctl32 retains the
        // unmirrored left/right key mapping. Keep arrows moving the thumb in
        // their physical direction while its accessible value stays elapsed.
        let wparam = if matches!(message, WM_KEYDOWN | WM_KEYUP)
            && GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_LAYOUTRTL.0 as isize != 0
        {
            match wparam.0 {
                0x25 => WPARAM(0x27),
                0x27 => WPARAM(0x25),
                _ => wparam,
            }
        } else {
            wparam
        };
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        if matches!(
            message,
            WM_PAINT | WM_SETFOCUS | WM_KILLFOCUS | WM_LBUTTONUP
        ) {
            let _ = slider(hwnd);
        }
        result
    }
}

/// Mirror the native trackbar, not its numeric range. Pointer hit testing
/// and the native accessibility value then share the same
/// elapsed-position coordinate system. The subclass normalizes arrow keys;
/// TBS_REVERSED alone does not reverse the native control.
pub(crate) unsafe fn set_slider_rtl(hwnd: HWND, rtl: bool) {
    unsafe {
        let existing = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let mirror = (WS_EX_LAYOUTRTL | WS_EX_NOINHERITLAYOUT).0 as isize;
        let desired = (existing & !mirror) | if rtl { mirror } else { 0 };
        if existing != desired {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

fn progress_span(left: f32, right: f32, thumb: f32, rtl: bool) -> (f32, f32) {
    let thumb = thumb.clamp(left, right);
    if rtl { (thumb, right) } else { (left, thumb) }
}

/// Native mirrored windows expose logical client coordinates. Map through
/// desktop space so D2D can paint physical pixels without a second reflection.
unsafe fn physical_rect(hwnd: HWND, rect: RECT) -> RECT {
    unsafe {
        let mut points = [
            POINT {
                x: rect.left,
                y: rect.top,
            },
            POINT {
                x: rect.right,
                y: rect.bottom,
            },
        ];
        MapWindowPoints(Some(hwnd), None, &mut points);
        RECT {
            left: points[0].x.min(points[1].x),
            top: points[0].y.min(points[1].y),
            right: points[0].x.max(points[1].x),
            bottom: points[0].y.max(points[1].y),
        }
    }
}
unsafe fn slider(hwnd: HWND) -> Result<()> {
    unsafe {
        let dc = GetDC(Some(hwnd));
        if dc.is_invalid() {
            return Ok(());
        }
        let result = slider_to_dc(hwnd, dc);
        let _ = ReleaseDC(Some(hwnd), dc);
        result
    }
}

/// Paint the existing high-contrast track/knob into a caller-owned snapshot DC.
pub(crate) unsafe fn slider_to_dc(hwnd: HWND, dc: HDC) -> Result<()> {
    unsafe {
        let saved = SaveDC(dc);
        if saved == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let _state = SavedDc { dc, saved };
        // ID2D1DCRenderTarget copies a bitmap into the DC. Prevent GDI from
        // reflecting that bitmap again for an RTL native control.
        SetLayout(dc, LAYOUT_BITMAPORIENTATIONPRESERVED);
        SendMessageW(
            GetParent(hwnd).unwrap_or_default(),
            WM_CTLCOLORSTATIC,
            Some(WPARAM(dc.0 as usize)),
            Some(LPARAM(hwnd.0 as isize)),
        );
        let bg = color(GetBkColor(dc));
        let ink = color(GetTextColor(dc));
        let mut client = RECT::default();
        let mut channel = RECT::default();
        let mut thumb = RECT::default();
        GetClientRect(hwnd, &mut client)?;
        SendMessageW(
            hwnd,
            TBM_GETCHANNELRECT,
            None,
            Some(LPARAM((&mut channel as *mut RECT) as isize)),
        );
        SendMessageW(
            hwnd,
            TBM_GETTHUMBRECT,
            None,
            Some(LPARAM((&mut thumb as *mut RECT) as isize)),
        );
        let mirrored = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_LAYOUTRTL.0 as isize != 0;
        if mirrored {
            let bounds = physical_rect(hwnd, client);
            let local = |rect: RECT| {
                let rect = physical_rect(hwnd, rect);
                RECT {
                    left: rect.left - bounds.left,
                    top: rect.top - bounds.top,
                    right: rect.right - bounds.left,
                    bottom: rect.bottom - bounds.top,
                }
            };
            channel = local(channel);
            thumb = local(thumb);
        }
        let dpi = GetDpiForWindow(hwnd).max(96) as f32;
        let scale = 96. / dpi;
        draw(dc, client, dpi, |target, _, _| {
            target.Clear(Some(&bg));
            let mix = |amount: f32| D2D1_COLOR_F {
                r: bg.r + (ink.r - bg.r) * amount,
                g: bg.g + (ink.g - bg.g) * amount,
                b: bg.b + (ink.b - bg.b) * amount,
                a: 1.,
            };
            let track = target.CreateSolidColorBrush(&mix(0.4), None)?;
            let active = target.CreateSolidColorBrush(&mix(0.85), None)?;
            let knob = target.CreateSolidColorBrush(&ink, None)?;
            let x = (thumb.left + thumb.right) as f32 * scale / 2.;
            let y = (thumb.top + thumb.bottom) as f32 * scale / 2.;
            let left = channel.left as f32 * scale;
            let right = channel.right as f32 * scale;
            if right <= left {
                return Ok(());
            }
            let round = |a, b| D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: a,
                    top: y - 2.,
                    right: b,
                    bottom: y + 2.,
                },
                radiusX: 2.,
                radiusY: 2.,
            };
            target.FillRoundedRectangle(&round(left, right), &track);
            let (start, end) = progress_span(left, right, x, mirrored);
            target.FillRoundedRectangle(&round(start, end), &active);
            let circle = D2D1_ELLIPSE {
                point: Vector2 { X: x, Y: y },
                radiusX: 6.5,
                radiusY: 6.5,
            };
            target.FillEllipse(&circle, &knob);
            if GetFocus() == hwnd {
                target.DrawEllipse(
                    &D2D1_ELLIPSE {
                        radiusX: 9.,
                        radiusY: 9.,
                        ..circle
                    },
                    &active,
                    1.,
                    None,
                );
            }
            Ok(())
        })
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

#[cfg(test)]
mod tests {
    use super::progress_span;

    #[test]
    fn elapsed_track_fills_towards_the_native_thumb_in_reading_order() {
        assert_eq!(progress_span(10., 210., 60., false), (10., 60.));
        assert_eq!(progress_span(10., 210., 160., true), (160., 210.));
        assert_eq!(progress_span(10., 210., 210., true), (210., 210.));
        assert_eq!(progress_span(10., 210., 10., true), (10., 210.));
    }

    #[test]
    fn native_thumb_rounding_cannot_paint_past_the_track() {
        assert_eq!(progress_span(10., 210., -2., true), (10., 210.));
        assert_eq!(progress_span(10., 210., 220., false), (10., 210.));
    }
}
