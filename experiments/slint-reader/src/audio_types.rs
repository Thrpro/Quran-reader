//! Plain recitation state shared by the platform audio bridges: the Windows
//! bridge over the native engine (audio_bridge.rs) and the Linux and macOS one
//! (portable/audio_bridge.rs).

/// Plain observed state. All times are actual content milliseconds, not a
/// synthetic timer or an estimate derived from ayah count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioSnapshot {
    pub active: bool,
    pub paused: bool,
    pub loading: bool,
    pub completed: bool,
    pub error: bool,
    pub continuous: bool,
    pub can_seek: bool,
    pub current: Option<(u32, u32)>,
    pub opening_surah: Option<u32>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub range_start_ms: u64,
    pub range_end_ms: u64,
    pub slider: u32,
    pub clock: String,
    pub status: String,
}

/// A surah saved for offline listening. Reciters with a verified continuous
/// recording keep that recording (with its ayah timing); others keep ayah clips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadedSurah {
    pub reciter: usize,
    pub surah: u32,
    pub bytes: u64,
    pub continuous: bool,
    /// Every file its record names is present and non-empty. Playback still
    /// verifies each file's hash; this only keeps the list honest.
    pub complete: bool,
}

// Linux and macOS cannot download recitations yet.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
pub enum DownloadNotice {
    Progress(u32),
    Done,
    Failed(String),
    Cancelled,
}
