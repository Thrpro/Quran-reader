//! Portable publisher binding and timing policy. No platform or I/O APIs.
//! Provider bytes stay unchanged; these intervals only select the highlight.
use serde::Deserialize;

pub struct Recording {
    pub read: u32,
    pub folder: &'static str,
}
impl Recording {
    /// A preference for full recordings cannot replace an unavailable reciter.
    /// Other reciters retain their original ayah clips, without changing the
    /// saved preference or selecting a different voice.
    pub fn enabled(preferred: bool, reciter: usize) -> bool {
        preferred && Self::for_reciter(reciter).is_some()
    }
    pub fn for_reciter(reciter: usize) -> Option<Self> {
        match reciter {
            0 => Some(Self {
                read: 92,
                folder: "https://cdn.mp3quran.net/audio/yasser-dosari/r1/",
            }),
            5 => Some(Self {
                read: 123,
                folder: "https://cdn.mp3quran.net/audio/mishary-alafasy/r1/",
            }),
            _ => None,
        }
    }
    pub fn audio_url(&self, surah: u32) -> Result<String, &'static str> {
        if !(1..=114).contains(&surah) {
            return Err("Invalid chapter");
        }
        Ok(format!("{}{surah:03}.mp3", self.folder))
    }
    pub fn verify_catalog(&self, bytes: &[u8]) -> Result<(), &'static str> {
        #[derive(Deserialize)]
        struct Read {
            id: u32,
            folder_url: String,
            soar_count: u32,
        }
        if bytes.len() > 1024 * 1024 {
            return Err("Recording catalogue too large");
        }
        let rows: Vec<Read> =
            serde_json::from_slice(bytes).map_err(|_| "Invalid recording catalogue")?;
        if rows.len() > 1024 {
            return Err("Recording catalogue too large");
        }
        let matching: Vec<_> = rows.iter().filter(|row| row.id == self.read).collect();
        if matching.len() != 1
            || matching[0].folder_url != self.folder
            || matching[0].soar_count != 114
        {
            return Err("Publisher read and recording folder do not match");
        }
        Ok(())
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct PlaybackClock {
    pub chapter_position_ms: u64,
    pub chapter_duration_ms: u64,
    pub remaining_ms: u64,
}
impl PlaybackClock {
    /// Native limited items report a range-relative clock. Present the
    /// publisher's chapter clock while keeping remaining time inside the
    /// selected range and adjusted for the requested playback speed.
    pub fn chapter(
        position_ms: u64,
        offset_ms: u64,
        chapter_duration_ms: u64,
        range_duration_ms: u64,
        rate: f64,
    ) -> Self {
        let rate = if rate.is_finite() && rate > 0. {
            rate
        } else {
            1.
        };
        Self {
            chapter_position_ms: offset_ms
                .saturating_add(position_ms)
                .min(chapter_duration_ms),
            chapter_duration_ms,
            remaining_ms: (range_duration_ms.saturating_sub(position_ms) as f64 / rate).ceil()
                as u64,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub ayah: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}
pub struct Timeline {
    rows: Vec<Interval>,
}
impl Timeline {
    pub fn parse(bytes: &[u8], count: u32) -> Result<Self, &'static str> {
        #[derive(Deserialize)]
        struct Raw {
            ayah: u32,
            start_time: u64,
            end_time: u64,
        }
        if bytes.len() > 1024 * 1024 || !(1..=286).contains(&count) {
            return Err("Invalid timing table size/count");
        }
        let raw: Vec<Raw> = serde_json::from_slice(bytes).map_err(|_| "Invalid timing table")?;
        if raw.len() < count as usize || raw.len() > count as usize + 1 {
            return Err("Incomplete timing table");
        }
        let mut rows = Vec::with_capacity(count as usize);
        let mut previous_end = 0;
        for (index, row) in raw.iter().enumerate() {
            if row.start_time < previous_end
                || row.end_time <= row.start_time
                || row.end_time > 86_400_000
            {
                return Err("Invalid or overlapping timing interval");
            }
            previous_end = row.end_time;
            if row.ayah == 0 {
                if index != 0 || raw.len() != count as usize + 1 {
                    return Err("Invalid introductory timing row");
                }
                continue;
            }
            if row.ayah != rows.len() as u32 + 1 {
                return Err("Missing, duplicate or unordered ayah timing");
            }
            rows.push(Interval {
                ayah: row.ayah,
                start_ms: row.start_time,
                end_ms: row.end_time,
            });
        }
        if rows.len() != count as usize {
            return Err("Incomplete timing table");
        }
        Ok(Self { rows })
    }
    pub fn validate_duration(&self, duration_ms: u64) -> Result<(), &'static str> {
        if duration_ms == 0 || self.rows.last().unwrap().end_ms > duration_ms.saturating_add(250) {
            return Err("Timing table exceeds the matching recording duration");
        }
        Ok(())
    }
    pub fn at(&self, position_ms: u64) -> Option<Interval> {
        // Preserve the previous verse during a publisher's natural pause.
        let index = self.rows.partition_point(|row| row.start_ms <= position_ms);
        index.checked_sub(1).map(|index| self.rows[index])
    }
    pub fn range(&self, first: u32, last: u32) -> Result<(u64, Option<u64>), &'static str> {
        if first == 0 || first > last || last as usize > self.rows.len() {
            return Err("Invalid timed listening range");
        }
        let start = if first == 1 {
            0
        } else {
            self.rows[first as usize - 1].start_ms
        };
        // A complete chapter waits for the decoder's end, including its tail.
        let end = ((last as usize) < self.rows.len()).then(|| self.rows[last as usize - 1].end_ms);
        Ok((start, end))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const GOOD: &[u8]=br#"[{"ayah":1,"start_time":63,"end_time":2310},{"ayah":2,"start_time":2400,"end_time":4515}]"#;
    #[test]
    fn chapter_clock_retains_mid_surah_position_and_selected_range_end() {
        assert_eq!(
            PlaybackClock::chapter(15_000, 180_000, 3_000_000, 120_000, 1.25),
            PlaybackClock {
                chapter_position_ms: 195_000,
                chapter_duration_ms: 3_000_000,
                remaining_ms: 84_000
            }
        );
        assert_eq!(
            PlaybackClock::chapter(u64::MAX, 1, 50_000, 10_000, 1.).chapter_position_ms,
            50_000
        );
        assert_eq!(
            PlaybackClock::chapter(20_000, 0, 20_000, 10_000, 1.).remaining_ms,
            0
        );
        assert_eq!(
            PlaybackClock::chapter(0, 0, 20_000, 10_000, f64::NAN).remaining_ms,
            10_000
        );
    }
    #[test]
    fn full_recording_preference_never_changes_an_unavailable_voice() {
        for reciter in 0..20 {
            assert_eq!(Recording::enabled(true, reciter), matches!(reciter, 0 | 5));
            assert!(!Recording::enabled(false, reciter));
        }
    }
    #[test]
    fn gaps_intro_and_tail_keep_source_boundaries() {
        let t = Timeline::parse(GOOD, 2).unwrap();
        assert_eq!(t.at(62), None);
        assert_eq!(t.at(63).unwrap().ayah, 1);
        assert_eq!(t.at(2399).unwrap().ayah, 1);
        assert_eq!(t.at(2400).unwrap().ayah, 2);
        assert_eq!(t.range(1, 2).unwrap(), (0, None));
        assert_eq!(t.range(1, 1).unwrap(), (0, Some(2310)));
        assert_eq!(t.range(2, 2).unwrap(), (2400, None));
        assert!(t.validate_duration(4000).is_err());
        assert!(t.validate_duration(4500).is_ok());
        assert!(t.range(0, 2).is_err());
        assert!(t.range(2, 1).is_err());
    }
    #[test]
    fn malformed_or_wrong_recording_metadata_is_rejected() {
        for raw in [
            br#"[{"ayah":1,"start_time":0,"end_time":-1}]"#.as_slice(),
            br#"[{"ayah":1,"start_time":0.5,"end_time":2}]"#,
            br#"[{"ayah":2,"start_time":0,"end_time":2}]"#,
            br#"[{"ayah":1,"start_time":0,"end_time":0}]"#,
        ] {
            assert!(Timeline::parse(raw, 1).is_err());
        }
        assert!(Timeline::parse(GOOD, 1).is_err());
        let r = Recording::for_reciter(0).unwrap();
        assert!(
            r.verify_catalog(
                br#"[{"id":92,"folder_url":"https://everyayah.com/","soar_count":114}]"#
            )
            .is_err()
        );
        assert!(r.verify_catalog(br#"[]"#).is_err());
        assert!(r.audio_url(0).is_err());
        assert!(Recording::for_reciter(1).is_none());
    }
}
