//! Inspect complete Layer III recordings before handing them to MediaPlayer.
//! Validation reads frames incrementally; source recording bytes are never changed.
use crate::data::Result;
use std::io::{Cursor, Read};

pub const MAX_AUDIO_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_CHAPTER_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ID3_PREFIX: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Mp3Info {
    pub bytes: u64,
    pub frames: u32,
    pub duration_seconds: f64,
}
pub fn error_content_type(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    ["text/", "json", "xml", "javascript"]
        .iter()
        .any(|part| value.contains(part))
}
struct Frame {
    length: usize,
    version: u8,
    rate: u32,
    samples: u32,
}
fn frame_header(header: [u8; 4]) -> Result<Frame> {
    let [a, b, c, _] = header;
    let version = (b >> 3) & 3;
    let rate_index = (c >> 2) & 3;
    let bitrate_index = c >> 4;
    if a != 255
        || b & 224 != 224
        || version == 1
        || b & 6 != 2
        || rate_index == 3
        || bitrate_index == 0
        || bitrate_index == 15
    {
        return Err("Response is not a valid Layer III recording".into());
    }
    let rate = [44_100u32, 48_000, 32_000][rate_index as usize]
        / if version == 3 {
            1
        } else if version == 2 {
            2
        } else {
            4
        };
    let rates = if version == 3 {
        [
            0u32, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ]
    } else {
        [
            0u32, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160,
        ]
    };
    let length = ((if version == 3 { 144_000 } else { 72_000 }) * rates[bitrate_index as usize]
        / rate
        + u32::from((c >> 1) & 1)) as usize;
    if !(4..=2048).contains(&length) {
        return Err("Invalid MP3 frame length".into());
    }
    Ok(Frame {
        length,
        version,
        rate,
        samples: if version == 3 { 1152 } else { 576 },
    })
}
fn declared_frames(bytes: &[u8], version: u8) -> u32 {
    let be32 = |offset: usize| u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let mono = bytes[3] >> 6 == 3;
    let side_info = match (version == 3, mono) {
        (true, true) => 17,
        (true, false) => 32,
        (false, true) => 9,
        (false, false) => 17,
    };
    let xing = 4 + if bytes[1] & 1 == 0 { 2 } else { 0 } + side_info;
    let mut frames = 0;
    if xing + 12 <= bytes.len()
        && (&bytes[xing..xing + 4] == b"Xing" || &bytes[xing..xing + 4] == b"Info")
        && be32(xing + 4) & 1 != 0
    {
        frames = be32(xing + 8);
    }
    let vbri = 36;
    if vbri + 18 <= bytes.len() && &bytes[vbri..vbri + 4] == b"VBRI" {
        frames = frames.max(be32(vbri + 14));
    }
    frames
}
pub fn inspect(bytes: &[u8]) -> Result<Mp3Info> {
    inspect_reader(
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        MAX_AUDIO_BYTES as u64,
    )
}
/// The caller supplies the exact complete-file length and a storage policy cap.
/// Only a 2 KiB frame and 8 KiB discard buffer are retained, even for large chapters.
/// Reading all bytes also lets a hash/cancellation adapter wrap this reader.
pub fn inspect_reader(reader: &mut impl Read, length: u64, limit: u64) -> Result<Mp3Info> {
    if length < 4 || length > limit || limit > MAX_CHAPTER_BYTES {
        return Err("Audio recording is empty or exceeds its limit".into());
    }
    let mut consumed = 4;
    let mut header = [0u8; 4];
    reader.read_exact(&mut header)?;
    let mut scratch = [0u8; 8192];
    if header[..3] == *b"ID3" {
        if length < 14 || !(2..=4).contains(&header[3]) {
            return Err("Invalid ID3 header".into());
        }
        let mut rest = [0u8; 6];
        reader.read_exact(&mut rest)?;
        consumed += 6;
        if rest[2..].iter().any(|byte| *byte > 127) {
            return Err("Invalid ID3 header".into());
        }
        let size = rest[2..]
            .iter()
            .fold(0u64, |size, byte| (size << 7) | u64::from(*byte));
        let extra = size
            + if header[3] == 4 && rest[1] & 16 != 0 {
                10
            } else {
                0
            };
        if consumed + extra + 4 > length || consumed + extra + 4 > MAX_ID3_PREFIX {
            return Err("Incomplete or excessively large ID3 tag".into());
        }
        let mut left = extra;
        while left > 0 {
            let size = left.min(scratch.len() as u64) as usize;
            reader.read_exact(&mut scratch[..size])?;
            left -= size as u64;
        }
        consumed += extra;
        reader.read_exact(&mut header)?;
        consumed += 4;
    }
    let mut frames = 0u32;
    let mut expected_frames = 0;
    let mut duration = 0.;
    let mut format = None;
    let mut frame = [0u8; 2048];
    loop {
        let remaining = length - consumed + 4;
        if (remaining == 128 && header[..3] == *b"TAG") || header == [0; 4] {
            let zero_tail = header == [0; 4];
            let mut left = remaining - 4;
            while left > 0 {
                let size = left.min(scratch.len() as u64) as usize;
                reader.read_exact(&mut scratch[..size])?;
                if zero_tail && scratch[..size].iter().any(|byte| *byte != 0) {
                    return Err("Invalid MP3 trailing bytes".into());
                }
                left -= size as u64;
            }
            break;
        }
        let parsed = frame_header(header)?;
        if format.is_some_and(|prior| prior != (parsed.version, parsed.rate)) {
            return Err("Inconsistent MP3 stream format".into());
        }
        format = Some((parsed.version, parsed.rate));
        if parsed.length as u64 > remaining {
            return Err("Audio recording ends inside an MP3 frame".into());
        }
        frame[..4].copy_from_slice(&header);
        reader.read_exact(&mut frame[4..parsed.length])?;
        consumed += (parsed.length - 4) as u64;
        if frames == 0 {
            expected_frames = declared_frames(&frame[..parsed.length], parsed.version);
        }
        frames = frames.checked_add(1).ok_or("Too many MP3 frames")?;
        duration += f64::from(parsed.samples) / f64::from(parsed.rate);
        let remaining = length - consumed;
        if remaining == 0 {
            break;
        }
        if remaining < 4 {
            reader.read_exact(&mut scratch[..remaining as usize])?;
            if scratch[..remaining as usize].iter().any(|byte| *byte != 0) {
                return Err("Incomplete MP3 frame header".into());
            }
            break;
        }
        reader.read_exact(&mut header)?;
        consumed += 4;
    }
    let mut extra = [0u8; 1];
    if reader.read(&mut extra)? != 0 {
        return Err("Recording grew beyond its declared length".into());
    }
    if frames == 0 || frames < expected_frames {
        return Err("Audio recording is missing declared MP3 frames".into());
    }
    Ok(Mp3Info {
        bytes: length,
        frames,
        duration_seconds: duration,
    })
}

#[cfg(test)]
pub(crate) fn fixture(frames: usize) -> Vec<u8> {
    let mut bytes = vec![0; 417 * frames];
    for offset in (0..bytes.len()).step_by(417) {
        bytes[offset..offset + 4].copy_from_slice(&[255, 251, 144, 0]);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FrameStream {
        remaining: u64,
        position: u64,
        max_read: usize,
    }
    impl Read for FrameStream {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.max_read = self.max_read.max(buffer.len());
            let count = buffer.len().min(self.remaining as usize).min(37);
            for byte in &mut buffer[..count] {
                let offset = (self.position % 417) as usize;
                *byte = if offset < 4 {
                    [255, 251, 144, 0][offset]
                } else {
                    0
                };
                self.position += 1;
            }
            self.remaining -= count as u64;
            Ok(count)
        }
    }
    #[test]
    fn chapter_stream_is_bounded_and_clip_policy_stays_strict() {
        let length = 65_000 * 417;
        let mut stream = FrameStream {
            remaining: length,
            position: 0,
            max_read: 0,
        };
        assert!(inspect_reader(&mut stream, length, MAX_AUDIO_BYTES as u64).is_err());
        assert_eq!(stream.position, 0);
        let info = inspect_reader(&mut stream, length, MAX_CHAPTER_BYTES).unwrap();
        assert_eq!(info.frames, 65_000);
        assert_eq!(info.bytes, length);
        assert!(stream.max_read <= 8192);
        assert_eq!(stream.remaining, 0);
    }
    #[test]
    fn stream_rejects_wrong_lengths_and_propagates_cancellation() {
        let bytes = fixture(12);
        assert!(
            inspect_reader(
                &mut Cursor::new(&bytes),
                bytes.len() as u64 - 417,
                MAX_CHAPTER_BYTES
            )
            .is_err()
        );
        assert!(
            inspect_reader(
                &mut Cursor::new(&bytes[..bytes.len() - 1]),
                bytes.len() as u64,
                MAX_CHAPTER_BYTES
            )
            .is_err()
        );
        struct Cancelled;
        impl Read for Cancelled {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("cancelled"))
            }
        }
        assert!(
            inspect_reader(&mut Cancelled, 1000, MAX_CHAPTER_BYTES)
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        let mut padded = bytes.clone();
        padded.extend_from_slice(&[0; 9]);
        padded.push(1);
        assert!(inspect(&padded).is_err());
        let mut tagged = bytes;
        tagged.extend_from_slice(b"TAG");
        tagged.extend_from_slice(&[0; 125]);
        assert_eq!(inspect(&tagged).unwrap().frames, 12);
    }
    #[test]
    fn crc_protected_xing_checks_full_recording_count() {
        let mut bytes = fixture(12);
        bytes[1] = 250;
        bytes[38..42].copy_from_slice(b"Xing");
        bytes[42..46].copy_from_slice(&1u32.to_be_bytes());
        bytes[46..50].copy_from_slice(&12u32.to_be_bytes());
        assert_eq!(inspect(&bytes).unwrap().frames, 12);
        assert!(inspect(&bytes[..417 * 10]).is_err());
    }
    #[test]
    fn complete_frames_and_editorial_silence_remain_unchanged() {
        let mut bytes = fixture(12);
        bytes.extend_from_slice(&[0; 6]);
        let info = inspect(&bytes).unwrap();
        assert_eq!(info.frames, 12);
        assert!((info.duration_seconds - 12.0 * 1152.0 / 44_100.0).abs() < 1e-9);
        let mut id3 = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
        id3.extend_from_slice(&bytes);
        assert_eq!(inspect(&id3).unwrap().frames, 12);
    }
    #[test]
    fn truncated_html_reserved_headers_and_id3_are_rejected() {
        assert!(inspect(b"<html>temporarily unavailable</html>").is_err());
        assert!(inspect(br#"{"error":"request denied"}"#).is_err());
        let bytes = fixture(6);
        assert!(inspect(&bytes[..bytes.len() - 1]).is_err());
        let mut reserved = bytes.clone();
        reserved[2] = 0;
        assert!(inspect(&reserved).is_err());
        assert!(inspect(b"ID3\x04\x00\x00\x7f\x7f\x7f\x7f").is_err());
        assert!(error_content_type("text/html; charset=utf-8"));
        assert!(!error_content_type("application/octet-stream"));
    }
    #[test]
    fn xing_and_vbri_detect_frame_aligned_truncation() {
        for signature in [b"Xing", b"Info"] {
            let mut bytes = fixture(12);
            bytes[36..40].copy_from_slice(signature);
            bytes[40..44].copy_from_slice(&1u32.to_be_bytes());
            bytes[44..48].copy_from_slice(&12u32.to_be_bytes());
            assert!(inspect(&bytes).is_ok());
            assert!(inspect(&bytes[..417 * 10]).is_err());
            bytes[44..48].copy_from_slice(&11u32.to_be_bytes());
            assert!(inspect(&bytes).is_ok()); // Header excluded by some encoders.
        }
        let mut bytes = fixture(12);
        bytes[36..40].copy_from_slice(b"VBRI");
        bytes[50..54].copy_from_slice(&12u32.to_be_bytes());
        assert!(inspect(&bytes).is_ok());
        assert!(inspect(&bytes[..417 * 10]).is_err());
    }
}
