//! HTTPS downloads from the recitation providers named in PRIVACY.md, and no
//! other host. Requests carry only the file being fetched.
use crate::data::Result;
use std::{
    io::{Read, Write},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const HOSTS: [&str; 4] = [
    "everyayah.com",
    "verses.quran.foundation",
    "www.mp3quran.net",
    "cdn.mp3quran.net",
];

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .https_only(true)
            // A provider redirect could lead anywhere; they are refused.
            .max_redirects(0)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(60)))
            .user_agent(concat!("QuranMushafReader/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    })
}

/// Whether `url` is an HTTPS address on one of the providers, with a plain path.
pub fn allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    HOSTS.contains(&host)
        && !path.contains("..")
        && !path.contains('#')
        && path.chars().all(|c| c.is_ascii_graphic())
}

/// Streams `url` into `output`, up to `limit` bytes. `progress` hears the
/// bytes so far and the expected total. Stops when `cancelled` is set.
pub fn fetch(
    url: &str,
    output: &mut impl Write,
    limit: u64,
    audio: bool,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<u64> {
    if !allowed(url) {
        return Err("Untrusted recitation address".into());
    }
    let mut response = agent().get(url).call().map_err(|e| match e {
        ureq::Error::StatusCode(status) => {
            format!("The recitation provider returned HTTP {status}")
        }
        other => format!("The recitation could not be downloaded: {other}"),
    })?;
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if audio
        && ["text/", "json", "xml", "javascript"]
            .iter()
            .any(|t| content_type.contains(t))
    {
        return Err("The recitation provider returned a page instead of audio".into());
    }
    let total = response.body().content_length();
    if total.is_some_and(|t| t > limit) {
        return Err("The recitation is larger than expected".into());
    }
    let mut reader = response.body_mut().with_config().limit(limit).reader();
    let mut buffer = vec![0; 64 * 1024];
    let mut written = 0u64;
    progress(0, total);
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Download cancelled".into());
        }
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        output.write_all(&buffer[..n])?;
        written += n as u64;
        progress(written, total);
    }
    if total.is_some_and(|t| t != written) {
        return Err("The recitation download ended early".into());
    }
    Ok(written)
}

/// Fetches a small file into memory.
pub fn fetch_bytes(url: &str, limit: u64, audio: bool, cancelled: &AtomicBool) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fetch(url, &mut bytes, limit, audio, cancelled, |_, _| {})?;
    Ok(bytes)
}

/// Retries a fetch after brief pauses, as connections drop.
pub fn with_retries<T>(
    cancelled: &AtomicBool,
    mut attempt: impl FnMut() -> Result<T>,
) -> Result<T> {
    let mut last = None;
    for delay in [0, 1, 3] {
        if delay > 0 {
            std::thread::sleep(Duration::from_secs(delay));
        }
        if cancelled.load(Ordering::Acquire) {
            return Err("Download cancelled".into());
        }
        match attempt() {
            Ok(value) => return Ok(value),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| "Download failed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_providers_over_https() {
        assert!(allowed(
            "https://everyayah.com/data/Alafasy_128kbps/001001.mp3"
        ));
        assert!(allowed(
            "https://cdn.mp3quran.net/audio/yasser-dosari/r1/112.mp3"
        ));
        assert!(allowed(
            "https://www.mp3quran.net/api/v3/ayat_timing?surah=112&read=92"
        ));
        for bad in [
            "http://everyayah.com/data/Alafasy_128kbps/001001.mp3",
            "https://everyayah.com.evil/data/x.mp3",
            "https://evil.example/https://everyayah.com/x.mp3",
            "https://everyayah.com/data/../secret.mp3",
            "https://everyayah.com/data/a b.mp3",
            "https://user@everyayah.com/data/x.mp3",
        ] {
            assert!(!allowed(bad), "{bad}");
        }
    }
}
