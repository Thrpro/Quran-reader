//! Shared WinHTTP protocol floor. Certificate and OS cipher policy stay enabled.
use crate::data::Result;
use std::ffi::c_void;
use windows::{Win32::Networking::WinHttp::*, core::HRESULT};
fn unsupported_option(code: HRESULT) -> bool {
    code.0 as u32 == 0x80070057
}
pub unsafe fn configure_protocols(session: *mut c_void) -> Result<()> {
    unsafe {
        // Permit TLS 1.3 on supporting Windows versions, always retaining the
        // TLS 1.2 floor. Older WinHTTP versions can reject the unknown flag.
        let modern = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
        match WinHttpSetOption(
            Some(session),
            WINHTTP_OPTION_SECURE_PROTOCOLS,
            Some(&modern.to_ne_bytes()),
        ) {
            Ok(()) => Ok(()),
            Err(error) if unsupported_option(error.code()) => {
                WinHttpSetOption(
                    Some(session),
                    WINHTTP_OPTION_SECURE_PROTOCOLS,
                    Some(&WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2.to_ne_bytes()),
                )?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_fallback_is_limited_to_an_unsupported_option() {
        assert!(unsupported_option(HRESULT(0x80070057u32 as i32)));
        for code in [0x80072f8fu32, 0x80070005, 0x80072ee2, 0x80004005] {
            assert!(!unsupported_option(HRESULT(code as i32)));
        }
    }
}
