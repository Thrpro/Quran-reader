//! Native control pixel snapshots for a measured motion prototype.
//! These copies keep the Windows HWND responsible for input/accessibility.
use crate::data::Result;
use std::{ffi::c_void, marker::PhantomData, rc::Rc};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::{
        Direct2D::{
            Common::{D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT},
            D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_BITMAP_PROPERTIES,
            ID2D1RenderTarget,
        },
        Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
        Gdi::*,
    },
    UI::WindowsAndMessaging::GetClientRect,
};

const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;

pub struct GdiSnapshot {
    dc: HDC,
    bitmap: HBITMAP,
    original: HGDIOBJ,
    pixels: *mut c_void,
    width: i32,
    height: i32,
    bytes: usize,
    // A memory DC and its selected objects belong to the creating UI thread.
    _ui_thread: PhantomData<Rc<()>>,
}

impl GdiSnapshot {
    /// Paint once at the control's actual physical size, without scaling.
    /// The owner separately bounds the aggregate cache and fingerprints theme,
    /// DPI, text, focus and other control state before reusing a snapshot.
    pub fn new(width: i32, height: i32, painter: impl FnOnce(HDC) -> Result<()>) -> Result<Self> {
        if width <= 0 || height <= 0 {
            return Err("Control snapshot has an empty physical size".into());
        }
        let bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .filter(|bytes| *bytes <= MAX_SNAPSHOT_BYTES)
            .ok_or("Control snapshot exceeds its pixel budget")?;
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return Err(windows::core::Error::from_thread().into());
            }
            let mut snapshot = Self {
                dc,
                bitmap: HBITMAP::default(),
                original: HGDIOBJ::default(),
                pixels: std::ptr::null_mut(),
                width,
                height,
                bytes,
                _ui_thread: PhantomData,
            };
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = std::ptr::null_mut();
            snapshot.bitmap =
                CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut pixels, None, 0)?;
            if pixels.is_null() {
                return Err("Control snapshot returned no pixel storage".into());
            }
            snapshot.pixels = pixels;
            // GDI has not touched this fresh DIB, so initialization needs no
            // synchronization. The painter must cover the complete client area.
            std::ptr::write_bytes(pixels.cast::<u8>(), 0, bytes);
            snapshot.original = SelectObject(dc, snapshot.bitmap.into());
            if snapshot.original.is_invalid() {
                return Err(windows::core::Error::from_thread().into());
            }
            let saved = SaveDC(dc);
            if saved == 0 {
                return Err(windows::core::Error::from_thread().into());
            }
            let state = SavedDc { dc, saved };
            painter(dc)?;
            drop(state);
            Ok(snapshot)
        }
    }

    pub fn byte_len(&self) -> usize {
        self.bytes
    }

    /// Include native control pixels in the parent's presentation surface.
    /// Keep one physical source pixel per destination pixel: a changed size or
    /// DPI requires a fresh snapshot, never resampling native text.
    pub fn draw_to_target(
        &self,
        target: &ID2D1RenderTarget,
        bounds: crate::render::Rect,
        dpi: f32,
    ) -> Result<()> {
        if !dpi.is_finite()
            || dpi <= 0.
            || ![bounds.x, bounds.y, bounds.w, bounds.h]
                .iter()
                .all(|value| value.is_finite())
            || bounds.w <= 0.
            || bounds.h <= 0.
            || self.pixels.is_null()
        {
            return Err("Invalid native control presentation bounds".into());
        }
        let density = dpi / 96.;
        if (bounds.w * density).round() as i32 != self.width
            || (bounds.h * density).round() as i32 != self.height
        {
            return Err("Control snapshot physical size changed".into());
        }
        unsafe {
            let mut target_dpi_x = 0.;
            let mut target_dpi_y = 0.;
            target.GetDpi(&mut target_dpi_x, &mut target_dpi_y);
            if (target_dpi_x - dpi).abs() > 0.01 || (target_dpi_y - dpi).abs() > 0.01 {
                return Err("Control snapshot and presentation DPI differ".into());
            }
            let maximum = target.GetMaximumBitmapSize();
            if self.width as u32 > maximum || self.height as u32 > maximum {
                return Err("Control snapshot exceeds the presentation device limit".into());
            }
            // GDI can batch writes to the selected DIB. Complete them before
            // Direct2D reads its top-down BGRA storage; GDI's unused alpha byte
            // must never make native control text transparent.
            GdiFlush().ok()?;
            let bitmap = target.CreateBitmap(
                D2D_SIZE_U {
                    width: self.width as u32,
                    height: self.height as u32,
                },
                Some(self.pixels.cast_const()),
                self.width as u32 * 4,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    dpiX: dpi,
                    dpiY: dpi,
                },
            )?;
            let destination = crate::render::Rect {
                x: (bounds.x * density).round() / density,
                y: (bounds.y * density).round() / density,
                w: self.width as f32 / density,
                h: self.height as f32 / density,
            };
            target.DrawBitmap(
                &bitmap,
                Some(&destination.d2d()),
                1.,
                D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
            );
        }
        Ok(())
    }

    /// Copy the original physical pixels to the control's new client origin.
    /// A size change invalidates the snapshot rather than resampling its text.
    pub fn blit(&self, hwnd: HWND) -> Result<()> {
        unsafe {
            let mut client = RECT::default();
            GetClientRect(hwnd, &mut client)?;
            if client.right - client.left != self.width || client.bottom - client.top != self.height
            {
                return Err("Control snapshot physical size changed".into());
            }
            let dc = GetDC(Some(hwnd));
            if dc.is_invalid() {
                return Err(windows::core::Error::from_thread().into());
            }
            let window_dc = WindowDc { hwnd, dc };
            BitBlt(
                window_dc.dc,
                0,
                0,
                self.width,
                self.height,
                Some(self.dc),
                0,
                0,
                SRCCOPY | NOMIRRORBITMAP,
            )?;
            Ok(())
        }
    }
}

impl Drop for GdiSnapshot {
    fn drop(&mut self) {
        unsafe {
            if !self.original.is_invalid() {
                SelectObject(self.dc, self.original);
            }
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(self.bitmap.into());
            }
            if !self.dc.is_invalid() {
                let _ = DeleteDC(self.dc);
            }
        }
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

struct WindowDc {
    hwnd: HWND,
    dc: HDC,
}
impl Drop for WindowDc {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(Some(self.hwnd), self.dc);
        }
    }
}
