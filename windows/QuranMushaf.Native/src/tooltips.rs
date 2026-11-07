//! Small, owned native hover labels for existing controls.
//!
//! The tooltip subclasses each registered HWND itself; it never requests text
//! through App's window procedure. Stable UTF-16 buffers outlive registration,
//! and the native window is destroyed before those buffers are released.
use crate::data::Result;
use std::{collections::BTreeMap, marker::PhantomData, rc::Rc};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::HFONT,
        System::Threading::GetCurrentThreadId,
        UI::{Controls::*, WindowsAndMessaging::*},
    },
    core::{PWSTR, w},
};

const MAX_TOOLS: usize = 32;
const MAX_TEXT_UNITS: usize = 256;

struct Tool {
    text: Box<[u16]>,
    arabic: bool,
}

/// UI-thread-owned Win32 tooltip; it never takes focus or reserves reader space.
pub(crate) struct Tooltips {
    hwnd: HWND,
    owner: HWND,
    tools: BTreeMap<usize, Tool>,
    appearance: Option<(u32, usize, u32, u32)>,
    active: bool,
    _ui_thread: PhantomData<Rc<()>>,
}

impl Tooltips {
    /// The owner and all registered controls must belong to this UI thread.
    pub(crate) fn new(owner: HWND) -> Result<Self> {
        unsafe {
            if !IsWindow(Some(owner)).as_bool()
                || GetWindowThreadProcessId(owner, None) != GetCurrentThreadId()
            {
                return Err("Tooltip owner must belong to the current UI thread".into());
            }
            if !InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES,
            })
            .as_bool()
            {
                return Err(windows::core::Error::from_thread().into());
            }
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                TOOLTIPS_CLASSW,
                None,
                WS_POPUP | WINDOW_STYLE(TTS_NOPREFIX | TTS_NOANIMATE | TTS_NOFADE),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                Some(owner),
                None,
                None,
                None,
            )?;
            let tips = Self {
                hwnd,
                owner,
                tools: BTreeMap::new(),
                appearance: None,
                active: true,
                _ui_thread: PhantomData,
            };
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )?;
            for (kind, milliseconds) in [
                (TTDT_INITIAL, 450),
                (TTDT_RESHOW, 100),
                (TTDT_AUTOPOP, 8_000),
            ] {
                SendMessageW(
                    hwnd,
                    TTM_SETDELAYTIME,
                    Some(WPARAM(kind as usize)),
                    Some(LPARAM(milliseconds)),
                );
            }
            Ok(tips)
        }
    }

    /// Register a known child control or replace its full, localized action.
    /// Text is plain Unicode, with bounded storage and no markup/callbacks.
    pub(crate) fn add_or_update(&mut self, control: HWND, text: &str, arabic: bool) -> Result<()> {
        let text = tooltip_text(text)?;
        let key = control.0 as usize;
        unsafe {
            if !IsWindow(Some(control)).as_bool()
                || GetParent(control)? != self.owner
                || GetWindowThreadProcessId(control, None) != GetCurrentThreadId()
            {
                return Err("Tooltip control must be an owned UI-thread child".into());
            }
            let existing = self.tools.get(&key);
            if existing.is_some_and(|tool| tool.text == text && tool.arabic == arabic) {
                return Ok(());
            }
            if existing.is_none() && self.tools.len() >= MAX_TOOLS {
                return Err("Native tooltip control limit reached".into());
            }
            self.pop();
            let mut tool = Tool { text, arabic };
            let info = self.info(control, &mut tool);
            if existing.is_some() {
                SendMessageW(
                    self.hwnd,
                    TTM_SETTOOLINFOW,
                    None,
                    Some(LPARAM((&info as *const TTTOOLINFOW) as isize)),
                );
            } else if SendMessageW(
                self.hwnd,
                TTM_ADDTOOLW,
                None,
                Some(LPARAM((&info as *const TTTOOLINFOW) as isize)),
            )
            .0 == 0
            {
                return Err("Windows could not register the native hover label".into());
            }
            // Keep both old and new buffers alive throughout the native update.
            self.tools.insert(key, tool);
        }
        Ok(())
    }

    fn info(&self, control: HWND, tool: &mut Tool) -> TTTOOLINFOW {
        // The application's top-level owner is LTR even in its Arabic layout.
        // TTF_RTLREADING reverses text direction relative to that owner.
        let owner_rtl = unsafe {
            GetWindowLongPtrW(self.owner, GWL_EXSTYLE) & WS_EX_RTLREADING.0 as isize != 0
        };
        TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_IDISHWND
                | TTF_SUBCLASS
                | if tool.arabic != owner_rtl {
                    TTF_RTLREADING
                } else {
                    TOOLTIP_FLAGS(0)
                },
            hwnd: self.owner,
            uId: control.0 as usize,
            lpszText: PWSTR(tool.text.as_mut_ptr()),
            ..Default::default()
        }
    }

    /// Match the app palette and DPI. The caller retains ownership of the font
    /// and must keep it alive until the next appearance update or tooltip drop.
    pub(crate) fn set_appearance(
        &mut self,
        dpi: f32,
        font: HFONT,
        background_rgb: u32,
        ink_rgb: u32,
    ) -> Result<()> {
        let dpi = if dpi.is_finite() {
            dpi.clamp(48., 768.)
        } else {
            96.
        }
        .round() as u32;
        let key = (dpi, font.0 as usize, background_rgb, ink_rgb);
        if self.appearance == Some(key) {
            return Ok(());
        }
        self.pop();
        unsafe {
            // TTM_SETTIP*COLOR is ignored while visual styles are enabled.
            // Native text, wrapping, positioning and hover behavior remain.
            SetWindowTheme(self.hwnd, w!(""), w!(""))?;
            SendMessageW(
                self.hwnd,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(0)),
            );
            SendMessageW(
                self.hwnd,
                TTM_SETTIPBKCOLOR,
                Some(WPARAM(background_rgb as usize)),
                None,
            );
            SendMessageW(
                self.hwnd,
                TTM_SETTIPTEXTCOLOR,
                Some(WPARAM(ink_rgb as usize)),
                None,
            );
            SendMessageW(
                self.hwnd,
                TTM_SETMAXTIPWIDTH,
                None,
                Some(LPARAM((320 * dpi / 96) as isize)),
            );
        }
        self.appearance = Some(key);
        Ok(())
    }

    /// Hide a stale hover label before moving/hiding its control or changing
    /// languages, opening a popup, starting a gesture, or losing activation.
    pub(crate) fn pop(&self) {
        unsafe {
            SendMessageW(self.hwnd, TTM_POP, None, None);
        }
    }

    /// Temporarily disable hover labels while another surface owns interaction.
    pub(crate) fn set_active(&mut self, active: bool) {
        if self.active != active {
            self.pop();
            unsafe {
                SendMessageW(self.hwnd, TTM_ACTIVATE, Some(WPARAM(active as usize)), None);
            }
            self.active = active;
        }
    }

    /// Read back the native tool registration and the actual visible tooltip.
    /// No pointer returned by Windows is dereferenced here.
    #[cfg(feature = "qa")]
    pub(crate) fn inspect(&self, control: HWND) -> (String, bool, bool) {
        unsafe {
            let mut text = [0u16; MAX_TEXT_UNITS + 1];
            let mut info = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                hwnd: self.owner,
                uId: control.0 as usize,
                lpszText: PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            SendMessageW(
                self.hwnd,
                TTM_GETTEXTW,
                Some(WPARAM(text.len())),
                Some(LPARAM((&mut info as *mut TTTOOLINFOW) as isize)),
            );
            let length = text
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(text.len());
            let label = String::from_utf16_lossy(&text[..length]);
            let mut current = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                ..Default::default()
            };
            let visible = IsWindowVisible(self.hwnd).as_bool()
                && SendMessageW(
                    self.hwnd,
                    TTM_GETCURRENTTOOLW,
                    None,
                    Some(LPARAM((&mut current as *mut TTTOOLINFOW) as isize)),
                )
                .0 != 0
                && current.hwnd == self.owner
                && current.uId == control.0 as usize;
            let mut registered_text = [0u16; MAX_TEXT_UNITS + 1];
            let mut registered = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                hwnd: self.owner,
                uId: control.0 as usize,
                lpszText: PWSTR(registered_text.as_mut_ptr()),
                ..Default::default()
            };
            let found = SendMessageW(
                self.hwnd,
                TTM_GETTOOLINFOW,
                None,
                Some(LPARAM((&mut registered as *mut TTTOOLINFOW) as isize)),
            )
            .0 != 0;
            let rtl = found && registered.uFlags & TTF_RTLREADING != TOOLTIP_FLAGS(0);
            (label, visible, rtl)
        }
    }
}

impl Drop for Tooltips {
    fn drop(&mut self) {
        unsafe {
            // Owner destruction may already have destroyed its owned popup.
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

fn tooltip_text(text: &str) -> Result<Box<[u16]>> {
    if text.is_empty() || text.chars().any(char::is_control) {
        return Err("Native hover label must be nonempty plain text".into());
    }
    let mut encoded: Vec<u16> = text.encode_utf16().take(MAX_TEXT_UNITS + 1).collect();
    if encoded.len() > MAX_TEXT_UNITS {
        return Err("Native hover label is too long".into());
    }
    encoded.push(0);
    Ok(encoded.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_hover_labels_are_bounded_terminated_plain_unicode() {
        let label = tooltip_text("انتقل إلى الآية الحالية").unwrap();
        assert_eq!(label.last(), Some(&0));
        assert_eq!(label.iter().filter(|&&unit| unit == 0).count(), 1);
        assert_eq!(
            String::from_utf16(&label[..label.len() - 1]).unwrap(),
            "انتقل إلى الآية الحالية"
        );
        for invalid in [
            "".to_owned(),
            "a\0b".into(),
            "a\nb".into(),
            "x".repeat(MAX_TEXT_UNITS + 1),
        ] {
            assert!(tooltip_text(&invalid).is_err());
        }
        assert!(tooltip_text(&"x".repeat(MAX_TEXT_UNITS)).is_ok());
    }
}
