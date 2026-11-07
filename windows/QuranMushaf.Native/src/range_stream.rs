//! Read-only, demand-driven original MP3 bytes. No disk cache, transcoding or
//! complete-chapter download. Each stream cursor shares a bounded range cache.
use crate::{audio_cache, data::Result, mp3};
use std::{
    collections::VecDeque,
    ffi::c_void,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use windows::{
    Foundation::{IClosable, IClosable_Impl},
    Storage::Streams::*,
    Win32::{
        Foundation::{E_ABORT, E_ACCESSDENIED, E_FAIL, E_INVALIDARG},
        Networking::WinHttp::*,
    },
    core::{PCWSTR, Ref, implement, w},
};
use windows_future::{IAsyncOperation, IAsyncOperationWithProgress};
const BLOCK: u64 = 256 * 1024;
const BLOCKS: usize = 8;
pub const MAX_RETAINED_BYTES: u64 = BLOCK * BLOCKS as u64;
struct Handle(*mut c_void);
impl Handle {
    fn new(p: *mut c_void) -> Result<Self> {
        if p.is_null() {
            Err(windows::core::Error::from_thread().into())
        } else {
            Ok(Self(p))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}
pub struct Source {
    url: String,
    pub size: u64,
    validator: String,
    cancelled: Arc<AtomicBool>,
    cache: Mutex<VecDeque<(u64, Vec<u8>)>>,
    fetched: AtomicU64,
    #[cfg(feature = "qa")]
    ranges: Mutex<Vec<(u64, usize, String)>>,
}
impl Source {
    pub fn open(url: String, cancelled: Arc<AtomicBool>) -> Result<Arc<Self>> {
        let (bytes, size, validator) = request(&url, 0, BLOCK - 1, None, &cancelled)?;
        if size == 0 || size > mp3::MAX_CHAPTER_BYTES {
            return Err("Invalid recording size".into());
        }
        if bytes.len() < 4 || (bytes.starts_with(b"<") || bytes.starts_with(b"{")) {
            return Err("Invalid recording header".into());
        }
        let fetched = bytes.len() as u64;
        #[cfg(feature = "qa")]
        let ranges = Mutex::new(vec![(0, bytes.len(), audio_cache::sha256(&bytes)?)]);
        Ok(Arc::new(Self {
            url,
            size,
            validator,
            cancelled,
            cache: Mutex::new(VecDeque::from([(0, bytes)])),
            fetched: AtomicU64::new(fetched),
            #[cfg(feature = "qa")]
            ranges,
        }))
    }
    pub fn stream(self: &Arc<Self>) -> IRandomAccessStream {
        Cursor::at(self.clone(), 0).into()
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    #[cfg(feature = "qa")]
    pub fn stats(&self) -> (u64, u64) {
        (
            self.fetched.load(Ordering::Acquire),
            self.cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|(_, b)| b.len() as u64)
                .sum(),
        )
    }
    #[cfg(feature = "qa")]
    pub fn ranges(&self) -> Vec<(u64, usize, String)> {
        self.ranges
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn read(&self, position: u64, count: usize) -> Result<Vec<u8>> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err("Streaming cancelled".into());
        }
        let count = count.min(self.size.saturating_sub(position) as usize);
        let mut out = Vec::with_capacity(count);
        let mut offset = position;
        // Serializes cache fills, bounding both retained and in-flight buffers
        // even when Media Foundation creates several independent cursors.
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        while out.len() < count {
            let start = offset / BLOCK * BLOCK;
            let block = if let Some(i) = cache.iter().position(|(p, _)| *p == start) {
                cache.remove(i).unwrap()
            } else {
                let (bytes, size, validator) = request(
                    &self.url,
                    start,
                    (start + BLOCK - 1).min(self.size - 1),
                    Some(&self.validator),
                    &self.cancelled,
                )?;
                if size != self.size || validator != self.validator {
                    return Err("Recording changed during streaming; retry".into());
                }
                self.fetched
                    .fetch_add(bytes.len() as u64, Ordering::Release);
                #[cfg(feature = "qa")]
                {
                    let mut rows = self.ranges.lock().unwrap_or_else(|e| e.into_inner());
                    if rows.len() < 128 {
                        rows.push((start, bytes.len(), audio_cache::sha256(&bytes)?));
                    }
                }
                (start, bytes)
            };
            let within = (offset - start) as usize;
            let length = (count - out.len()).min(block.1.len().saturating_sub(within));
            if length == 0 {
                return Err("Incomplete recording range".into());
            }
            out.extend_from_slice(&block.1[within..within + length]);
            offset += length as u64;
            cache.push_front(block);
            cache.truncate((MAX_RETAINED_BYTES / BLOCK) as usize);
        }
        Ok(out)
    }
}
fn request(
    url: &str,
    first: u64,
    last: u64,
    expected: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<(Vec<u8>, u64, String)> {
    // URL is constructed from the fixed publisher binding, never user input.
    let suffix = url
        .strip_prefix("https://cdn.mp3quran.net/audio/")
        .ok_or("Untrusted stream URL")?;
    if suffix.contains(['?', '#', '\\'])
        || suffix.contains("..")
        || last < first
        || last - first >= BLOCK
    {
        return Err("Invalid stream range".into());
    }
    if cancelled.load(Ordering::Acquire) {
        return Err("Streaming cancelled".into());
    }
    unsafe {
        let session = Handle::new(WinHttpOpen(
            w!("QuranMushafReader-Native/range"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 5000)?;
        crate::http_security::configure_protocols(session.0)?;
        WinHttpSetOption(
            Some(session.0),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes()),
        )?;
        let connection = Handle::new(WinHttpConnect(session.0, w!("cdn.mp3quran.net"), 443, 0))?;
        let path: Vec<u16> = format!("/audio/{suffix}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let req = Handle::new(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        WinHttpSetOption(
            Some(req.0),
            WINHTTP_OPTION_AUTOLOGON_POLICY,
            Some(&WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH.to_ne_bytes()),
        )?;
        WinHttpSetOption(
            Some(req.0),
            WINHTTP_OPTION_DISABLE_FEATURE,
            Some(
                &(WINHTTP_DISABLE_AUTHENTICATION
                    | WINHTTP_DISABLE_COOKIES
                    | WINHTTP_DISABLE_REDIRECTS)
                    .to_ne_bytes(),
            ),
        )?;
        let headers: Vec<u16> = format!(
            "Accept-Encoding: identity\r\nRange: bytes={first}-{last}\r\n{}\r\n",
            expected
                .map(|v| format!("If-Range: {v}"))
                .unwrap_or_default()
        )
        .encode_utf16()
        .collect();
        WinHttpSendRequest(req.0, Some(&headers), None, 0, 0, 0)?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut())?;
        let mut status = 0u32;
        let mut n = 4;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut n,
            std::ptr::null_mut(),
        )?;
        if status != 206 {
            return Err(
                format!("Provider does not support bounded streaming (HTTP {status})").into(),
            );
        }
        let content = audio_cache::header(req.0, WINHTTP_QUERY_CONTENT_TYPE)?;
        if mp3::error_content_type(&content) {
            return Err("Provider returned an error document".into());
        }
        let range = audio_cache::header(req.0, WINHTTP_QUERY_CONTENT_RANGE)?;
        let size = parse_range(&range, first, last)?;
        let mut validator = audio_cache::header(req.0, WINHTTP_QUERY_ETAG)?;
        if validator.starts_with("W/") || validator.is_empty() {
            validator = audio_cache::header(req.0, WINHTTP_QUERY_LAST_MODIFIED)?;
        }
        if validator.is_empty()
            || validator.len() > 512
            || validator.contains(['\r', '\n'])
            || expected.is_some_and(|v| v != validator)
        {
            return Err("Missing or changed recording identity".into());
        }
        let length = last.min(size - 1) - first + 1;
        let mut bytes = Vec::with_capacity(length as usize);
        let mut buffer = [0u8; 16384];
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err("Streaming cancelled".into());
            }
            let mut got = 0;
            WinHttpReadData(
                req.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut got,
            )?;
            if got == 0 {
                break;
            }
            if bytes.len() + got as usize > length as usize {
                return Err("Recording range exceeded its bound".into());
            }
            bytes.extend_from_slice(&buffer[..got as usize]);
        }
        if bytes.len() as u64 != length {
            return Err("Truncated recording range".into());
        }
        Ok((bytes, size, validator))
    }
}
fn parse_range(range: &str, first: u64, last: u64) -> Result<u64> {
    let (span, total) = range
        .strip_prefix("bytes ")
        .and_then(|r| r.split_once('/'))
        .ok_or("Missing byte range")?;
    let (a, b) = span.split_once('-').ok_or("Invalid byte range")?;
    let size = total.parse::<u64>()?;
    if size == 0
        || size > mp3::MAX_CHAPTER_BYTES
        || first >= size
        || last < first
        || a.parse::<u64>()? != first
        || b.parse::<u64>()? != last.min(size - 1)
    {
        return Err("Unexpected byte range".into());
    }
    Ok(size)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_must_match_requested_bytes_and_bounded_total() {
        assert_eq!(parse_range("bytes 0-99/100", 0, 255).unwrap(), 100);
        assert_eq!(parse_range("bytes 256-511/1024", 256, 511).unwrap(), 1024);
        for value in [
            "bytes 1-99/100",
            "bytes 0-99/*",
            "bytes 0-99/0",
            "bytes 0-99/999999999999",
            "bytes 0-99/18446744073709551616",
            "items 0-99/100",
            "bytes 0-100/100",
        ] {
            assert!(parse_range(value, 0, 255).is_err(), "{value}");
        }
        assert!(parse_range("bytes 100-100/100", 100, 101).is_err());
    }
    #[test]
    fn independent_read_only_cursors_share_exact_original_bytes() {
        let source = Arc::new(Source {
            url: String::new(),
            size: 2 * BLOCK,
            validator: String::new(),
            cancelled: Arc::new(AtomicBool::new(false)),
            cache: Mutex::new(VecDeque::from([
                (0, vec![0x23; BLOCK as usize]),
                (BLOCK, vec![0x72; BLOCK as usize]),
            ])),
            fetched: AtomicU64::new(0),
            #[cfg(feature = "qa")]
            ranges: Mutex::new(Vec::new()),
        });
        assert_eq!(source.read(BLOCK - 2, 4).unwrap(), [0x23, 0x23, 0x72, 0x72]);
        let stream = source.stream();
        stream.Seek(BLOCK).unwrap();
        let clone = stream.CloneStream().unwrap();
        clone.Seek(0).unwrap();
        assert_eq!(stream.Position().unwrap(), BLOCK);
        assert_eq!(clone.Position().unwrap(), 0);
        assert!(!stream.CanWrite().unwrap());
        assert!(stream.SetSize(0).is_err());
        assert!(stream.Seek(2 * BLOCK + 1).is_err());
        source.cancel();
        assert!(source.read(0, 1).is_err());
    }
}
#[implement(IRandomAccessStream, IInputStream, IOutputStream, IClosable)]
struct Cursor {
    source: Arc<Source>,
    position: Arc<Mutex<u64>>,
    closed: Arc<AtomicBool>,
}
impl Cursor {
    fn at(source: Arc<Source>, position: u64) -> Self {
        Self {
            source,
            position: Arc::new(Mutex::new(position)),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl IClosable_Impl for Cursor_Impl {
    fn Close(&self) -> windows::core::Result<()> {
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}
impl IInputStream_Impl for Cursor_Impl {
    fn ReadAsync(
        &self,
        buffer: Ref<IBuffer>,
        count: u32,
        _options: InputStreamOptions,
    ) -> windows::core::Result<IAsyncOperationWithProgress<IBuffer, u32>> {
        let capacity = buffer
            .as_ref()
            .ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?
            .Capacity()?;
        if count > capacity || count > 1024 * 1024 {
            return Err(E_INVALIDARG.into());
        }
        let source = self.source.clone();
        let position = self.position.clone();
        let closed = self.closed.clone();
        Ok(IAsyncOperationWithProgress::spawn(move || {
            if closed.load(Ordering::Acquire) {
                return Err(E_ABORT.into());
            }
            let mut p = position.lock().unwrap_or_else(|e| e.into_inner());
            let bytes = source
                .read(*p, count as usize)
                .map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?;
            let writer = DataWriter::new()?;
            writer.WriteBytes(&bytes)?;
            let result = writer.DetachBuffer()?;
            *p += bytes.len() as u64;
            Ok(result)
        }))
    }
}
impl IOutputStream_Impl for Cursor_Impl {
    fn WriteAsync(
        &self,
        _buffer: Ref<IBuffer>,
    ) -> windows::core::Result<IAsyncOperationWithProgress<u32, u32>> {
        Err(E_ACCESSDENIED.into())
    }
    fn FlushAsync(&self) -> windows::core::Result<IAsyncOperation<bool>> {
        Ok(IAsyncOperation::ready(Ok(true)))
    }
}
impl IRandomAccessStream_Impl for Cursor_Impl {
    fn Size(&self) -> windows::core::Result<u64> {
        Ok(self.source.size)
    }
    fn SetSize(&self, _: u64) -> windows::core::Result<()> {
        Err(E_ACCESSDENIED.into())
    }
    fn Position(&self) -> windows::core::Result<u64> {
        Ok(*self.position.lock().unwrap_or_else(|e| e.into_inner()))
    }
    fn Seek(&self, p: u64) -> windows::core::Result<()> {
        if p > self.source.size {
            return Err(E_INVALIDARG.into());
        }
        *self.position.lock().unwrap_or_else(|e| e.into_inner()) = p;
        Ok(())
    }
    fn GetInputStreamAt(&self, p: u64) -> windows::core::Result<IInputStream> {
        if p > self.source.size {
            return Err(E_INVALIDARG.into());
        }
        Ok(Cursor::at(self.source.clone(), p).into())
    }
    fn GetOutputStreamAt(&self, _: u64) -> windows::core::Result<IOutputStream> {
        Err(E_ACCESSDENIED.into())
    }
    fn CloneStream(&self) -> windows::core::Result<IRandomAccessStream> {
        Ok(Cursor::at(self.source.clone(), self.Position()?).into())
    }
    fn CanRead(&self) -> windows::core::Result<bool> {
        Ok(!self.closed.load(Ordering::Acquire))
    }
    fn CanWrite(&self) -> windows::core::Result<bool> {
        Ok(false)
    }
}
