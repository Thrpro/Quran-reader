//! A single native MediaPlayer owns rate/pause/volume for every playlist item.
//! MediaPlaybackList advances on decoded end-of-stream, never a duration timer.
#[cfg(feature = "qa")]
#[path = "rate_qa.rs"]
mod rate_qa;
use crate::audio_cache::{AudioCache, ClipKey, Event as CacheEvent, Job};
use crate::data::Result;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use windows::{
    Foundation::{PropertyValue, TimeSpan, TypedEventHandler, Uri},
    Media::{Core::MediaSource, Playback::*},
    core::{HSTRING, IInspectable, Interface},
};

pub struct Reciter {
    pub english: &'static str,
    pub arabic: &'static str,
    pub path: &'static str,
    pub everyayah: bool,
}
pub const RECITERS: &[Reciter] = &[
    Reciter {
        english: "Yasser Al-Dosari",
        arabic: "ياسر الدوسري",
        path: "Yasser_Ad-Dussary_128kbps",
        everyayah: true,
    },
    Reciter {
        english: "Maher Al-Muaiqly",
        arabic: "ماهر المعيقلي",
        path: "MaherAlMuaiqly128kbps",
        everyayah: true,
    },
    Reciter {
        english: "Nasser Al-Qatami",
        arabic: "ناصر القطامي",
        path: "Nasser_Alqatami_128kbps",
        everyayah: true,
    },
    Reciter {
        english: "Fares Abbad",
        arabic: "فارس عباد",
        path: "Fares_Abbad_64kbps",
        everyayah: true,
    },
    Reciter {
        english: "Abdullah Al-Juhany",
        arabic: "عبد الله الجهني",
        path: "Abdullaah_3awwaad_Al-Juhaynee_128kbps",
        everyayah: true,
    },
    Reciter {
        english: "Mishari Rashid Alafasy",
        arabic: "مشاري راشد العفاسي",
        path: "Alafasy",
        everyayah: false,
    },
    Reciter {
        english: "AbdulBaset AbdulSamad (Murattal)",
        arabic: "عبد الباسط عبد الصمد (مرتل)",
        path: "AbdulBaset/Murattal",
        everyayah: false,
    },
    Reciter {
        english: "AbdulBaset AbdulSamad (Mujawwad)",
        arabic: "عبد الباسط عبد الصمد (مجود)",
        path: "AbdulBaset/Mujawwad",
        everyayah: false,
    },
    Reciter {
        english: "Abdur-Rahman as-Sudais",
        arabic: "عبد الرحمن السديس",
        path: "Sudais",
        everyayah: false,
    },
    Reciter {
        english: "Abu Bakr al-Shatri",
        arabic: "أبو بكر الشاطري",
        path: "Shatri",
        everyayah: false,
    },
    Reciter {
        english: "Hani ar-Rifai",
        arabic: "هاني الرفاعي",
        path: "Rifai",
        everyayah: false,
    },
    Reciter {
        english: "Mahmoud Khalil Al-Husary (Muallim)",
        arabic: "محمود خليل الحصري (معلم)",
        path: "Husary/Muallim",
        everyayah: false,
    },
    Reciter {
        english: "Mahmoud Khalil Al-Husary",
        arabic: "محمود خليل الحصري",
        path: "Husary",
        everyayah: false,
    },
    Reciter {
        english: "Mohamed Siddiq al-Minshawi (Murattal)",
        arabic: "محمد صديق المنشاوي (مرتل)",
        path: "Minshawy/Murattal",
        everyayah: false,
    },
    Reciter {
        english: "Mohamed Siddiq al-Minshawi (Mujawwad)",
        arabic: "محمد صديق المنشاوي (مجود)",
        path: "Minshawy/Mujawwad",
        everyayah: false,
    },
    Reciter {
        english: "Saud ash-Shuraym",
        arabic: "سعود الشريم",
        path: "Shuraym",
        everyayah: false,
    },
    Reciter {
        english: "Mohamed al-Tablawi",
        arabic: "محمد الطبلاوي",
        path: "Tablawi",
        everyayah: false,
    },
];
pub fn url(reciter: usize, surah: u32, ayah: u32) -> Result<String> {
    let r = RECITERS.get(reciter).ok_or("Invalid reciter")?;
    if !(1..=114).contains(&surah) || !(1..=286).contains(&ayah) {
        return Err("Invalid audio reference".into());
    }
    Ok(if r.everyayah {
        format!(
            "https://everyayah.com/data/{}/{surah:03}{ayah:03}.mp3",
            r.path
        )
    } else {
        format!(
            "https://verses.quran.foundation/{}/mp3/{surah:03}{ayah:03}.mp3",
            r.path
        )
    })
}
/// Resolve editable reciter names conservatively. English transliterations may
/// vary in vowels/repeated letters; ambiguous or unrelated input never silently
/// selects a different reciter.
pub fn match_reciter(query: &str) -> Option<usize> {
    fn key(text: &str) -> String {
        let text = crate::data::normalize(text);
        let mut result = String::new();
        for c in text
            .chars()
            .filter(|c| c.is_alphabetic() && !"aeiou".contains(*c))
        {
            if !result.ends_with(c) {
                result.push(c);
            }
        }
        result
    }
    let needle = key(query);
    if needle.chars().count() < 3 {
        return None;
    }
    let matches: Vec<_> = RECITERS
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            [key(r.english), key(r.arabic)]
                .iter()
                .any(|name| name.contains(&needle))
        })
        .map(|(i, _)| i)
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}
#[cfg(test)]
mod reciter_tests {
    #[test]
    fn tolerant_but_not_ambiguous() {
        assert_eq!(super::match_reciter("Yaser Al Desari"), Some(0));
        assert_eq!(super::match_reciter("ياسر الدوسري"), Some(0));
        assert_eq!(super::match_reciter("Mahmoud Khalil Al Husary"), None);
        assert_eq!(super::match_reciter("unknown"), None);
    }
}
pub struct Audio {
    timed_store: Option<Arc<crate::timed_store::Store>>,
    scoped_timed_store: Option<Arc<crate::timed_store::Store>>,
    timed_job: Option<crate::timed_store::Job>,
    timed: Option<TimedSession>,
    stream_job: Option<crate::timed_store::StreamJob>,
    streamed: Option<StreamedSession>,
    timed_range: Option<(u32, u32)>,
    timed_progress: Option<(u64, Option<u64>)>,
    player: MediaPlayer,
    list: Option<MediaPlaybackList>,
    keys: Vec<Option<(u32, u32)>>,
    failed: Arc<AtomicBool>,
    failure_detail: Arc<std::sync::Mutex<String>>,
    failed_token: Option<i64>,
    changed_token: Option<i64>,
    item_failed_token: Option<i64>,
    ended_token: Option<i64>,
    opened_token: i64,
    state_token: i64,
    finished: Arc<AtomicBool>,
    desired_rate: Arc<RateControl>,
    live_rate: Option<LiveRateWorker>,
    generation: Arc<AtomicU64>,
    cache: Option<Arc<AudioCache>>,
    job: Option<Job>,
    pending_keys: Vec<Option<(u32, u32)>>,
    // Only a deliberately inserted, validated Fatiha 1:1 clip represents an
    // unnumbered opening. A continuous recording's untyped prefix cannot.
    opening_clip_surah: Option<u32>,
    clip_durations: Vec<f64>,
    preparation_complete: Arc<AtomicBool>,
    queue_ended: Arc<AtomicBool>,
    ended_index: Arc<AtomicUsize>,
    wanted_playing: bool,
    attached: bool,
    preparation_error: Option<String>,
    pub repeat: bool,
}
/// Serializes rate commits without making UI requests wait for a decoder call.
/// Callbacks may run synchronously during a native setter, so they defer a busy
/// gate instead of waiting recursively. The regular poll retries the latest
/// atomic value; queued commands never capture an obsolete speed or source.
struct RateControl {
    desired: AtomicU64,
    gate: std::sync::Mutex<()>,
}
impl RateControl {
    fn new() -> Self {
        Self {
            desired: AtomicU64::new(1.0f64.to_bits()),
            gate: std::sync::Mutex::new(()),
        }
    }
    fn value(&self) -> f64 {
        f64::from_bits(self.desired.load(Ordering::Acquire))
    }
    fn store(&self, rate: f64) {
        self.desired.store(rate.to_bits(), Ordering::Release);
    }
    fn apply(&self, player: &MediaPlayer) -> windows::core::Result<()> {
        self.apply_session(&player.PlaybackSession()?)
    }
    fn apply_session(&self, session: &MediaPlaybackSession) -> windows::core::Result<()> {
        let _guard = match self.gate.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(()),
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        };
        apply_session_rate(session, self.value())
    }
}
struct LiveRateWorker {
    wake: std::sync::mpsc::SyncSender<()>,
    error: Arc<std::sync::Mutex<Option<String>>>,
    cancelled: Arc<AtomicBool>,
}
impl LiveRateWorker {
    fn new(player: MediaPlayer, rate: Arc<RateControl>) -> Result<Self> {
        let (wake, receiver) = std::sync::mpsc::sync_channel(1);
        let error = Arc::new(std::sync::Mutex::new(None));
        let failed = error.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopped = cancelled.clone();
        std::thread::Builder::new()
            .name("recitation-rate".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                // MediaPlayer is ThreadingModel.Both / MarshalingType.Agile. Its
                // generated projection is Send + Sync; no custom unsafe impl is used.
                let initialized = unsafe {
                    windows::Win32::System::WinRT::RoInitialize(
                        windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
                    )
                };
                if let Err(cause) = initialized {
                    *failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(cause.to_string());
                    return;
                }
                while receiver.recv().is_ok() {
                    // Drain only the bounded wake token. The speed is read under
                    // the commit gate, so rapid edits apply the newest preference.
                    while receiver.try_recv().is_ok() {}
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    // Only this background worker waits for a concurrent native
                    // callback. A try_lock here could lose the final user edit.
                    let guard = rate.gate.lock().unwrap_or_else(|e| e.into_inner());
                    let result = (|| -> windows::core::Result<()> {
                        let session = player.PlaybackSession()?;
                        loop {
                            if stopped.load(Ordering::Acquire) {
                                return Ok(());
                            }
                            let desired = rate.value();
                            apply_session_rate(&session, desired)?;
                            if rate.value() == desired {
                                return Ok(());
                            }
                        }
                    })();
                    drop(guard);
                    if let Err(cause) = result {
                        if stopped.load(Ordering::Acquire) {
                            break;
                        }
                        *failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(cause.to_string());
                    }
                }
                drop(player);
                unsafe {
                    windows::Win32::System::WinRT::RoUninitialize();
                }
            })?;
        Ok(Self {
            wake,
            error,
            cancelled,
        })
    }
    fn wake(&self) -> Result<()> {
        match self.wake.try_send(()) {
            Ok(()) | Err(std::sync::mpsc::TrySendError::Full(())) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Disconnected(())) => {
                Err("Playback speed worker stopped".into())
            }
        }
    }
}
impl Drop for LiveRateWorker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        // Dropping the sender wakes a waiting receiver. Never join a decoder
        // operation from the UI thread; the worker releases its agile player.
    }
}
#[derive(Clone, Copy)]
pub struct Transport {
    pub position_ms: u64,
    pub range_ms: u64,
    pub offset_ms: u64,
    pub duration_ms: u64,
    pub continuous: bool,
    pub can_seek: bool,
}
struct TimedSession {
    ready: crate::timed_store::Ready,
    offset_ms: u64,
    first: u32,
    last: u32,
}
struct StreamedSession {
    surah: u32,
    timeline: crate::timed_audio::Timeline,
    source: MediaSource,
    bytes: Arc<crate::range_stream::Source>,
    offset_ms: u64,
    first: u32,
    last: u32,
    duration_ms: u64,
}
#[cfg(feature = "qa")]
impl Audio {
    pub fn range_stream_smoke(out: &std::path::Path) -> Result<()> {
        use std::time::{Duration, Instant};
        std::fs::create_dir_all(out)?;
        let report = out.join("range-stream.json");
        if report.exists() {
            return Err("Preserve previous streaming evidence".into());
        }
        let cache = AudioCache::new(out.join("profile"))?;
        let mut audio = Audio::with_cache(cache.clone())?;
        let began = Instant::now();
        audio.play_timed(
            ListeningRange {
                reciter: 0,
                surah: 2,
                first: 1,
                last: 286,
                count: 286,
            },
            1.,
            0.,
        )?;
        let mut first = None;
        let mut samples = Vec::new();
        let result: Result<()> = (|| {
            while began.elapsed() < Duration::from_secs(55) {
                audio.poll()?;
                if audio.failed.load(Ordering::Acquire) {
                    return Err(audio.status(false).into());
                }
                if audio.current().is_some()
                    && audio
                        .transport()
                        .is_some_and(|t| t.can_seek && t.position_ms > 200)
                {
                    first = Some(began.elapsed().as_secs_f64() * 1000.);
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            if first.is_none() {
                return Err(
                    format!("Streaming did not become seekable: {}", audio.status(false)).into(),
                );
            }
            audio.toggle()?;
            for (delta, slider) in [
                (Some(10_000), 0),
                (Some(-10_000), 0),
                (None, 6500),
                (None, 1200),
            ] {
                let before = audio.transport().ok_or("Missing transport")?;
                if !audio.seek(delta, slider)? {
                    return Err("Stream seek rejected".into());
                }
                let expected = delta.map_or_else(
                    || {
                        crate::listening_layout::slider_target(
                            slider,
                            before.duration_ms,
                            before.offset_ms,
                            before.range_ms,
                        )
                    },
                    |d| {
                        crate::listening_layout::seek_target(before.position_ms, before.range_ms, d)
                    },
                );
                let wait = Instant::now();
                let after = loop {
                    audio.poll()?;
                    let after = audio.transport().ok_or("Stream clock disappeared")?;
                    if after.position_ms.abs_diff(expected) < 2000 {
                        break after;
                    }
                    if wait.elapsed() > Duration::from_secs(12) {
                        return Err(format!(
                            "Seek mismatch: expected {expected}, got {}",
                            after.position_ms
                        )
                        .into());
                    }
                    std::thread::sleep(Duration::from_millis(40));
                };
                let s = audio
                    .streamed
                    .as_ref()
                    .ok_or("Expected a cold HTTP stream, not a disk source")?;
                let (fetched, retained) = s.bytes.stats();
                samples.push(serde_json::json!({"deltaMs":delta,"slider":slider,"positionMs":after.position_ms,"durationMs":after.duration_ms,"fetchedBytes":fetched,"retainedBytes":retained,"paused":audio.is_paused(),"surah":audio.current()}));
                if after.duration_ms < 6_000_000
                    || retained > crate::range_stream::MAX_RETAINED_BYTES
                    || fetched >= 16 * 1024 * 1024
                    || !audio.is_paused()
                {
                    return Err("Streaming violated duration/buffering/pause bounds".into());
                }
                audio.toggle()?;
                let resume = Instant::now();
                while resume.elapsed() < Duration::from_millis(1600) {
                    audio.poll()?;
                    std::thread::sleep(Duration::from_millis(30));
                }
                if audio.failed.load(Ordering::Acquire)
                    || audio
                        .transport()
                        .is_none_or(|t| t.position_ms < expected + 500)
                {
                    return Err("Stream did not resume decoding at the seek target".into());
                }
                audio.toggle()?;
            }
            let seconds = std::env::args()
                .skip_while(|s| s != "--stream-endurance-seconds")
                .nth(1)
                .map(|s| s.parse::<u64>())
                .transpose()?
                .unwrap_or(0)
                .min(600);
            if seconds > 0 {
                audio.request_speed(1.25)?;
                audio.toggle()?;
                let endurance = Instant::now();
                while endurance.elapsed() < Duration::from_secs(seconds) {
                    audio.poll()?;
                    if audio.failed.load(Ordering::Acquire) {
                        return Err(audio.status(false).into());
                    }
                    let source = audio.streamed.as_ref().ok_or("Stream disappeared")?;
                    let (fetched, retained) = source.bytes.stats();
                    if retained > crate::range_stream::MAX_RETAINED_BYTES
                        || fetched >= 24 * 1024 * 1024
                    {
                        return Err("Longer streaming exceeded its bounds".into());
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                audio.toggle()?;
                let t = audio.transport().ok_or("Missing endurance clock")?;
                samples.push(serde_json::json!({"enduranceSeconds":seconds,"positionMs":t.position_ms,"durationMs":t.duration_ms,"playbackRate":audio.player.PlaybackSession()?.PlaybackRate()?,"statistics":audio.streamed.as_ref().unwrap().bytes.stats()}));
            }
            Ok(())
        })();
        let stats = audio.streamed.as_ref().map(|s| s.bytes.stats());
        let ranges = audio
            .streamed
            .as_ref()
            .map(|s| s.bytes.ranges())
            .unwrap_or_default();
        let mut reference_verified = 0;
        if let Some(reference) = std::env::args()
            .skip_while(|s| s != "--stream-reference")
            .nth(1)
        {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = std::fs::File::open(reference)?;
            for (offset, len, digest) in &ranges {
                file.seek(SeekFrom::Start(*offset))?;
                let mut bytes = vec![0; *len];
                file.read_exact(&mut bytes)?;
                if crate::audio_cache::sha256(&bytes)? != *digest {
                    return Err("Original recording range differed".into());
                }
                reference_verified += 1;
            }
        }
        audio.stop()?;
        let disk = crate::audio_cache::directory_bytes(cache.folder())?;
        std::fs::write(
            &report,
            serde_json::to_vec_pretty(
                &serde_json::json!({"passed":result.is_ok(),"error":result.as_ref().err().map(|e|e.to_string()),"firstSeekableMs":first,"samples":samples,"networkAndCacheBytes":stats,"ranges":ranges,"rangesComparedToOriginal":reference_verified,"audioDiskBytes":disk,"scope":"Actual Windows MediaPlayer, cold original Al-Baqarah HTTP ranges and publisher timing table; resumed at four seek targets, muted. No audible/device endurance claim."}),
            )?,
        )?;
        result?;
        if disk != 0 {
            return Err("Ordinary streaming wrote audio to disk".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub struct ListeningRange {
    pub reciter: usize,
    pub surah: u32,
    pub first: u32,
    pub last: u32,
    pub count: u32,
}
/// QA observes the decoder's own transition reason, independent of the
/// non-atomic playback-session position and current-index properties.
#[cfg(feature = "qa")]
struct PlaylistTrace {
    list: MediaPlaybackList,
    token: i64,
    events: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
}
#[cfg(feature = "qa")]
impl PlaylistTrace {
    fn new(list: &MediaPlaybackList) -> Result<Self> {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = events.clone();
        let token = list.CurrentItemChanged(&TypedEventHandler::<
            MediaPlaybackList,
            CurrentMediaPlaybackItemChangedEventArgs,
        >::new(move |sender, args| {
            if let (Some(list), Some(args)) = (sender.as_ref(), args.as_ref()) {
                let index = |item: windows::core::Result<MediaPlaybackItem>| {
                    item.ok().and_then(|item| {
                        let mut index = 0;
                        list.Items()
                            .ok()?
                            .IndexOf(&item, &mut index)
                            .ok()?
                            .then_some(index)
                    })
                };
                let event = serde_json::json!({"oldIndex":index(args.OldItem()),"newIndex":index(args.NewItem()),"reason":args.Reason()?.0});
                if let Ok(mut events) = recorded.lock() {
                    events.push(event);
                }
            }
            Ok(())
        }))?;
        Ok(Self {
            list: list.clone(),
            token,
            events,
        })
    }
    fn snapshot(&self) -> Result<Vec<serde_json::Value>> {
        Ok(self
            .events
            .lock()
            .map_err(|_| "Audio QA event trace was poisoned")?
            .clone())
    }
}
#[cfg(feature = "qa")]
impl Drop for PlaylistTrace {
    fn drop(&mut self) {
        let _ = self.list.RemoveCurrentItemChanged(self.token);
    }
}
#[cfg(feature = "qa")]
fn decoded_transition(events: &[serde_json::Value], old: u32, new: u32) -> bool {
    let matching: Vec<_> = events
        .iter()
        .filter(|event| event["oldIndex"] == old && event["newIndex"] == new)
        .collect();
    matching.len() == 1 && matching[0]["reason"] == MediaPlaybackItemChangedReason::EndOfStream.0
}
impl Audio {
    pub fn new() -> Result<Self> {
        let player = MediaPlayer::new()?;
        player.SetAutoPlay(false)?;
        let failed = Arc::new(AtomicBool::new(false));
        let desired_rate = Arc::new(RateControl::new());
        let rate = desired_rate.clone();
        let opened_token = player.MediaOpened(
            &TypedEventHandler::<MediaPlayer, IInspectable>::new(move |sender, _| {
                if let Some(player) = sender.as_ref() {
                    rate.apply(player)?;
                }
                Ok(())
            }),
        )?;
        let rate = desired_rate.clone();
        let state_token = player
            .PlaybackSession()?
            .PlaybackStateChanged(
                &TypedEventHandler::<MediaPlaybackSession, IInspectable>::new(move |sender, _| {
                    if let Some(session) = sender.as_ref() {
                        rate.apply_session(session)?;
                    }
                    Ok(())
                }),
            );
        let state_token = match state_token {
            Ok(token) => token,
            Err(error) => {
                let _ = player.RemoveMediaOpened(opened_token);
                return Err(error.into());
            }
        };
        Ok(Self {
            player,
            list: None,
            keys: vec![],
            failed,
            failure_detail: Arc::new(std::sync::Mutex::new(String::new())),
            failed_token: None,
            changed_token: None,
            item_failed_token: None,
            ended_token: None,
            opened_token,
            state_token,
            finished: Arc::new(AtomicBool::new(false)),
            desired_rate,
            live_rate: None,
            generation: Arc::new(AtomicU64::new(0)),
            cache: None,
            timed_store: None,
            scoped_timed_store: None,
            timed_job: None,
            timed: None,
            stream_job: None,
            streamed: None,
            timed_range: None,
            timed_progress: None,
            job: None,
            pending_keys: Vec::new(),
            opening_clip_surah: None,
            clip_durations: Vec::new(),
            preparation_complete: Arc::new(AtomicBool::new(true)),
            queue_ended: Arc::new(AtomicBool::new(false)),
            ended_index: Arc::new(AtomicUsize::new(0)),
            wanted_playing: false,
            attached: false,
            preparation_error: None,
            repeat: false,
        })
    }
    pub fn with_cache(cache: Arc<AudioCache>) -> Result<Self> {
        let mut audio = Self::new()?;
        audio.timed_store = Some(cache.timed_store()?);
        audio.cache = Some(cache);
        Ok(audio)
    }
    pub fn play_range(&mut self, range: ListeningRange, speed: f64, volume: f64) -> Result<()> {
        let ListeningRange {
            reciter,
            surah,
            first,
            last,
            count,
        } = range;
        validate_range(reciter, surah, first, last, count)?;
        checked_rate(speed)?;
        checked_volume(volume)?;
        let cache = self
            .cache
            .clone()
            .ok_or("Validated recitation storage has not been initialized")?;
        let mut clips = Vec::new();
        let mut keys = Vec::new();
        let opening_surah = clip_opening_surah(surah, first);
        if opening_surah.is_some() {
            clips.push(ClipKey::new(reciter, 1, 1)?);
            keys.push(None);
        }
        for ayah in first..=last {
            clips.push(ClipKey::new(reciter, surah, ayah)?);
            keys.push(Some((surah, ayah)));
        }
        self.stop()?;
        self.failure_detail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.pending_keys = keys;
        self.opening_clip_surah = opening_surah;
        self.preparation_complete.store(false, Ordering::Release);
        self.wanted_playing = true;
        self.speed(speed)?;
        self.volume(volume)?;
        self.install_sources(Vec::new())?;
        match cache.prepare_playback(clips, self.generation.load(Ordering::Acquire)) {
            Ok(job) => self.job = Some(job),
            Err(error) => {
                let _ = self.stop();
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn play_timed(&mut self, range: ListeningRange, speed: f64, volume: f64) -> Result<()> {
        self.play_timed_using(range, speed, volume, false)
    }
    /// Multi-chapter requests keep their temporary recordings in a separate
    /// owned store. Existing persistent recordings are only borrowed read-only.
    pub fn play_timed_scoped(
        &mut self,
        range: ListeningRange,
        speed: f64,
        volume: f64,
    ) -> Result<()> {
        self.play_timed_using(range, speed, volume, true)
    }
    #[cfg(feature = "qa")]
    pub fn qa_timed_source(&self) -> Option<serde_json::Value> {
        self.timed.as_ref().map(|session| {
            serde_json::json!({"surah":session.ready.surah,"path":session.ready.path,
                "sha256":session.ready._verified.sha256,"bytes":session.ready._verified.info.bytes})
        })
    }
    fn play_timed_using(
        &mut self,
        range: ListeningRange,
        speed: f64,
        volume: f64,
        scoped: bool,
    ) -> Result<()> {
        validate_range(
            range.reciter,
            range.surah,
            range.first,
            range.last,
            range.count,
        )?;
        checked_rate(speed)?;
        checked_volume(volume)?;
        if crate::timed_audio::Recording::for_reciter(range.reciter).is_none() {
            return Err("Choose Yasser Al-Dosari or Alafasy for continuous recording mode".into());
        }
        if scoped && self.scoped_timed_store.is_none() {
            let cache = self
                .cache
                .as_ref()
                .ok_or("Validated recitation storage has not been initialized")?;
            self.scoped_timed_store = Some(Arc::new(crate::timed_store::Store::new_scoped(
                cache.folder(),
            )?));
        }
        let store = if scoped {
            &self.scoped_timed_store
        } else {
            &self.timed_store
        }
        .clone()
        .ok_or("Continuous recording storage is unavailable")?;
        self.stop()?;
        self.failure_detail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.timed_range = Some((range.first, range.last));
        self.preparation_complete.store(false, Ordering::Release);
        self.wanted_playing = true;
        self.speed(speed)?;
        self.volume(volume)?;
        self.install_sources(Vec::new())?;
        let root = self.cache.as_ref().unwrap().folder().to_path_buf();
        let recording = crate::timed_audio::Recording::for_reciter(range.reciter).unwrap();
        let generation = self.generation.load(Ordering::Acquire);
        let result = if root
            .join("Continuous")
            .join(format!("r{}-s{:03}", recording.read, range.surah))
            .exists()
        {
            store
                .prepare(range.reciter, range.surah, generation)
                .map(|job| self.timed_job = Some(job))
        } else {
            crate::timed_store::prepare_stream(root, range.reciter, range.surah, generation)
                .map(|job| self.stream_job = Some(job))
        };
        if let Err(error) = result {
            let _ = self.stop();
            return Err(error);
        }
        Ok(())
    }
    #[cfg(feature = "qa")]
    fn set_sources(
        &mut self,
        sources: Vec<(String, Option<(u32, u32)>)>,
        speed: f64,
        volume: f64,
    ) -> Result<()> {
        checked_rate(speed)?;
        checked_volume(volume)?;
        if sources.is_empty() || sources.len() > 287 {
            return Err("Invalid native playlist length".into());
        }
        self.stop()
            .map_err(|e| format!("Reset media player: {e}"))?;
        self.failed.store(false, Ordering::Release);
        self.wanted_playing = true;
        self.speed(speed)?;
        self.install_sources(sources)?;
        self.attached = true;
        self.speed(speed)?;
        self.volume(volume)?;
        self.player.Play()?;
        self.attached = true;
        Ok(())
    }
    fn install_sources(&mut self, sources: Vec<(String, Option<(u32, u32)>)>) -> Result<()> {
        let list = MediaPlaybackList::new()?;
        let prefetch = PropertyValue::CreateTimeSpan(TimeSpan {
            Duration: 60 * 10_000_000,
        })?
        .cast::<windows::Foundation::IReference<TimeSpan>>()?;
        list.SetMaxPrefetchTime(&prefetch)
            .map_err(|e| format!("Set native prefetch: {e}"))?;
        let keep =
            PropertyValue::CreateUInt32(1)?.cast::<windows::Foundation::IReference<u32>>()?;
        list.SetMaxPlayedItemsToKeepOpen(&keep)
            .map_err(|e| format!("Set native retained items: {e}"))?;
        list.SetAutoRepeatEnabled(
            self.repeat && self.preparation_complete.load(Ordering::Acquire),
        )?;
        let items = list.Items()?;
        let mut keys = Vec::with_capacity(sources.len());
        for (uri, key) in sources {
            let source = MediaSource::CreateFromUri(&Uri::CreateUri(&HSTRING::from(uri))?)
                .map_err(|e| format!("Create native media source: {e}"))?;
            items.Append(&MediaPlaybackItem::Create(&source)?)?;
            keys.push(key);
        }
        // Own the playlist before registering callbacks. Any later failure can
        // revoke every event and detach the source through the same stop path.
        self.keys = keys;
        self.list = Some(list.clone());
        let generation = self.generation.load(Ordering::Acquire);
        let result = (|| -> Result<()> {
            let finished = self.finished.clone();
            let active = self.generation.clone();
            let prepared = self.preparation_complete.clone();
            let exhausted = self.queue_ended.clone();
            let ended_index = self.ended_index.clone();
            let current_list = list.clone();
            self.ended_token = Some(self.player.MediaEnded(&TypedEventHandler::<
                MediaPlayer,
                IInspectable,
            >::new(move |_, _| {
                if active.load(Ordering::Acquire) == generation {
                    ended_index.store(
                        current_list.CurrentItemIndex().unwrap_or(0) as usize,
                        Ordering::Release,
                    );
                    exhausted.store(true, Ordering::Release);
                    if prepared.load(Ordering::Acquire) {
                        finished.store(true, Ordering::Release);
                    }
                }
                Ok(())
            }))?);
            let failed = self.failed.clone();
            let active = self.generation.clone();
            let detail = self.failure_detail.clone();
            self.failed_token =
                Some(self.player.MediaFailed(&TypedEventHandler::<
                    MediaPlayer,
                    MediaPlayerFailedEventArgs,
                >::new(move |sender, args| {
                    if active.load(Ordering::Acquire) == generation {
                        if let Some(args) = args.as_ref() {
                            *detail.lock().unwrap_or_else(|e| e.into_inner()) = format!(
                                "Decoder: {:?}; {}",
                                args.ExtendedErrorCode().ok(),
                                args.ErrorMessage()
                                    .map(|v| v.to_string())
                                    .unwrap_or_default()
                            );
                        }
                        failed.store(true, Ordering::Release);
                        if let Some(player) = sender.as_ref() {
                            player.Pause()?;
                        }
                    }
                    Ok(())
                }))?);
            let player = self.player.clone();
            let failed = self.failed.clone();
            let active = self.generation.clone();
            let detail = self.failure_detail.clone();
            self.item_failed_token =
                Some(list.ItemFailed(&TypedEventHandler::<
                    MediaPlaybackList,
                    MediaPlaybackItemFailedEventArgs,
                >::new(move |_, args| {
                    if active.load(Ordering::Acquire) == generation {
                        if let Some(args) = args.as_ref() {
                            *detail.lock().unwrap_or_else(|e| e.into_inner()) = format!(
                                "Playlist decoder: {:?}",
                                args.Error().and_then(|e| e.ExtendedError()).ok()
                            );
                        }
                        // This includes errors in an upcoming prefetched ayah: stop
                        // here rather than allow Windows to silently skip that item.
                        failed.store(true, Ordering::Release);
                        player.Pause()?;
                    }
                    Ok(())
                }))?);
            // Windows can skip an item after a network/decoder error by default.
            // Pause immediately and expose the error, rather than silently skip a verse.
            let player = self.player.clone();
            let failed = self.failed.clone();
            let rate = self.desired_rate.clone();
            let active = self.generation.clone();
            self.changed_token =
                Some(list.CurrentItemChanged(&TypedEventHandler::<
                    MediaPlaybackList,
                    CurrentMediaPlaybackItemChangedEventArgs,
                >::new(move |_, args| {
                    if active.load(Ordering::Acquire) != generation {
                        return Ok(());
                    }
                    if let Some(args) = args.as_ref() {
                        if args.Reason()? == MediaPlaybackItemChangedReason::Error {
                            failed.store(true, Ordering::Release);
                            player.Pause()?;
                        } else if failed.load(Ordering::Acquire) {
                            player.Pause()?;
                        } else {
                            rate.apply(&player)?;
                        }
                    }
                    Ok(())
                }))?);
            // An empty playlist is not a playable source. Attaching it before
            // the download worker delivers a file can asynchronously report a
            // decoder failure, racing the first successful append.
            if !self.keys.is_empty() {
                self.player
                    .SetSource(&list)
                    .map_err(|e| format!("Attach native playlist: {e}"))?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = self.stop();
            return Err(error);
        }
        Ok(())
    }
    /// Called by the app's regular UI tick. The worker only delivers complete,
    /// verified local files; no network or hashing occurs here.
    pub fn poll(&mut self) -> Result<()> {
        let generation = self.generation.load(Ordering::Acquire);
        let mut local_ready = None;
        if let Some(event) = self
            .stream_job
            .as_ref()
            .and_then(crate::timed_store::StreamJob::try_recv)
        {
            // Keep the cancellation token alive for the streaming source. Stop
            // releases the job only after detaching the decoder.
            match event {
                Ok(crate::timed_store::StreamReady::Local(ready)) => {
                    local_ready = Some(crate::timed_store::Event::Ready(ready));
                }
                Ok(crate::timed_store::StreamReady::Remote {
                    generation: g,
                    surah,
                    timeline,
                    source: bytes,
                }) if g == generation => {
                    let (first, last) = self.timed_range.ok_or("Missing streaming range")?;
                    let (start, end) = timeline.range(first, last)?;
                    let source = MediaSource::CreateFromStream(
                        &bytes.stream(),
                        &HSTRING::from("audio/mpeg"),
                    )?;
                    let item = if let Some(end) = end {
                        MediaPlaybackItem::CreateWithStartTimeAndDurationLimit(
                            &source,
                            TimeSpan {
                                Duration: start as i64 * 10_000,
                            },
                            TimeSpan {
                                Duration: (end - start) as i64 * 10_000,
                            },
                        )?
                    } else if start > 0 {
                        MediaPlaybackItem::CreateWithStartTime(
                            &source,
                            TimeSpan {
                                Duration: start as i64 * 10_000,
                            },
                        )?
                    } else {
                        MediaPlaybackItem::Create(&source)?
                    };
                    self.list
                        .as_ref()
                        .ok_or("Missing stream playlist")?
                        .Items()?
                        .Append(&item)?;
                    self.keys.push(None);
                    self.streamed = Some(StreamedSession {
                        surah,
                        timeline,
                        source,
                        bytes,
                        offset_ms: start,
                        first,
                        last,
                        duration_ms: 0,
                    });
                    self.preparation_complete.store(true, Ordering::Release);
                }
                Err(e) => {
                    self.preparation_error = Some(e.to_string());
                    self.failed.store(true, Ordering::Release);
                }
                _ => {}
            }
        }
        while let Some(event) = self
            .timed_job
            .as_ref()
            .and_then(crate::timed_store::Job::try_recv)
            .or_else(|| local_ready.take())
        {
            match event {
                crate::timed_store::Event::Ready(ready) if ready.generation == generation => {
                    let result: Result<()> = (|| {
                        let (first, last) = self.timed_range.ok_or("Missing timed range")?;
                        let (start, end) = ready.timeline.range(first, last)?;
                        let source = MediaSource::CreateFromUri(&Uri::CreateUri(&HSTRING::from(
                            local_uri(&ready.path)?,
                        ))?)?;
                        let item = if let Some(end) = end {
                            MediaPlaybackItem::CreateWithStartTimeAndDurationLimit(
                                &source,
                                TimeSpan {
                                    Duration: start as i64 * 10_000,
                                },
                                TimeSpan {
                                    Duration: (end - start) as i64 * 10_000,
                                },
                            )?
                        } else if start > 0 {
                            MediaPlaybackItem::CreateWithStartTime(
                                &source,
                                TimeSpan {
                                    Duration: start as i64 * 10_000,
                                },
                            )?
                        } else {
                            MediaPlaybackItem::Create(&source)?
                        };
                        self.list
                            .as_ref()
                            .ok_or("Missing timed source list")?
                            .Items()?
                            .Append(&item)?;
                        self.keys.push(None);
                        self.timed = Some(TimedSession {
                            ready: *ready,
                            offset_ms: start,
                            first,
                            last,
                        });
                        self.preparation_complete.store(true, Ordering::Release);
                        self.list
                            .as_ref()
                            .unwrap()
                            .SetAutoRepeatEnabled(self.repeat)?;
                        Ok(())
                    })();
                    if let Err(error) = result {
                        self.preparation_error =
                            Some(format!("Open continuous recording: {error}"));
                        self.failed.store(true, Ordering::Release);
                        self.wanted_playing = false;
                        self.timed_job = None;
                    }
                }
                crate::timed_store::Event::Failed(event_generation, message)
                    if event_generation == generation =>
                {
                    self.preparation_error = Some(message);
                    self.failed.store(true, Ordering::Release);
                    self.wanted_playing = false;
                }
                crate::timed_store::Event::Progress(bytes, total) => {
                    self.timed_progress = Some((bytes, total))
                }
                _ => {}
            }
        }
        while let Some(event) = self.job.as_ref().and_then(Job::try_recv) {
            match event {
                CacheEvent::Ready {
                    generation: event_generation,
                    index,
                    key,
                    path,
                    info,
                } if event_generation == generation => {
                    if index != self.keys.len() || self.pending_keys.get(index).is_none() {
                        return Err("Prepared recitation arrived out of order".into());
                    }
                    let expected = self.pending_keys[index];
                    if expected.is_some_and(|value| value != (key.surah, key.ayah)) {
                        return Err(
                            "Prepared recitation reference does not match the listening range"
                                .into(),
                        );
                    }
                    if expected.is_none()
                        && (index != 0
                            || self.opening_clip_surah.is_none()
                            || key.surah != 1
                            || key.ayah != 1)
                    {
                        return Err(
                            "Prepared opening does not match the validated Bismillah clip".into(),
                        );
                    }
                    let uri = local_uri(&path)?;
                    let source = MediaSource::CreateFromUri(&Uri::CreateUri(&HSTRING::from(uri))?)?;
                    self.list
                        .as_ref()
                        .ok_or("Missing prepared playlist")?
                        .Items()?
                        .Append(&MediaPlaybackItem::Create(&source)?)?;
                    self.keys.push(expected);
                    self.clip_durations.push(info.duration_seconds);
                }
                CacheEvent::Complete {
                    generation: event_generation,
                    ..
                } if event_generation == generation => {
                    self.preparation_complete.store(true, Ordering::Release);
                    if let Some(list) = &self.list {
                        list.SetAutoRepeatEnabled(self.repeat)?;
                    }
                }
                CacheEvent::Failed {
                    generation: event_generation,
                    message,
                } if event_generation == generation => {
                    // Every queued item is already a complete validated file.
                    // Keep that playable prefix intact if a later fetch fails;
                    // the last queued decoded end becomes the stop boundary.
                    self.preparation_error = Some(message);
                    if self.keys.is_empty() {
                        self.failed.store(true, Ordering::Release);
                    }
                }
                _ => {}
            }
        }
        let current = self
            .list
            .as_ref()
            .and_then(|list| list.CurrentItemIndex().ok())
            .unwrap_or(0) as usize;
        if let Some(job) = &self.job {
            job.set_consumed(current);
        }
        if self.queue_ended.load(Ordering::Acquire) {
            let ended = self.ended_index.load(Ordering::Acquire);
            if ended + 1 < self.keys.len() && !self.failed.load(Ordering::Acquire) {
                self.list
                    .as_ref()
                    .ok_or("Missing prepared playlist")?
                    .MoveTo((ended + 1) as u32)
                    .map_err(|e| format!("Resume prepared playlist: {e}"))?;
                self.queue_ended.store(false, Ordering::Release);
                self.finished.store(false, Ordering::Release);
                if self.wanted_playing {
                    self.player.Play()?;
                }
            } else if self.preparation_error.is_some() {
                self.player.Pause()?;
                self.failed.store(true, Ordering::Release);
            } else if self.preparation_complete.load(Ordering::Acquire) && !self.repeat {
                self.finished.store(true, Ordering::Release);
            }
        }
        if !self.attached && !self.keys.is_empty() && !self.failed.load(Ordering::Acquire) {
            // Attach only once a complete playable item exists. This avoids
            // empty-source errors on Windows Media Foundation implementations.
            self.player
                .SetSource(self.list.as_ref().ok_or("Missing prepared playlist")?)
                .map_err(|e| format!("Attach prepared playlist: {e}"))?;
            // A fresh list opens its first item automatically. MoveTo(0)
            // during asynchronous source opening can reject the request;
            // no seek is needed here. Later queue resumptions still select
            // their explicit next item after decoded completion.
            self.queue_ended.store(false, Ordering::Release);
            self.finished.store(false, Ordering::Release);
            if self.wanted_playing {
                self.player.Play()?;
            }
            self.attached = true;
        }
        if let Some(streamed) = self.streamed.as_mut().filter(|s| s.duration_ms == 0) {
            if let Ok(duration) = streamed.source.Duration().and_then(|d| d.Value()) {
                let milliseconds = duration.Duration.max(0) as u64 / 10_000;
                if milliseconds > 0 {
                    if let Err(error) = streamed.timeline.validate_duration(milliseconds) {
                        self.failed.store(true, Ordering::Release);
                        self.player.Pause()?;
                        return Err(error.into());
                    }
                    streamed.duration_ms = milliseconds;
                }
            }
        }
        if self.attached && !self.failed.load(Ordering::Acquire) {
            if let Some(worker) = &self.live_rate {
                if let Some(error) = worker
                    .error
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                {
                    return Err(format!("Change playback speed: {error}").into());
                }
                worker.wake()?;
            } else {
                self.desired_rate.apply(&self.player)?;
            }
        }
        Ok(())
    }
    pub fn stop(&mut self) -> Result<()> {
        // Queued callbacks from a previous session must not fail the next one.
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(streamed) = &self.streamed {
            streamed.bytes.cancel();
        }
        if let Some(job) = self.stream_job.take() {
            drop(job);
        }
        self.live_rate = None;
        let mut first_error = if self.attached {
            self.player.Pause().err()
        } else {
            None
        };
        if let Err(error) = self.player.SetSource(None::<&IMediaPlaybackSource>) {
            first_error.get_or_insert(error);
        }
        if let Some(token) = self.failed_token.take() {
            if let Err(error) = self.player.RemoveMediaFailed(token) {
                first_error.get_or_insert(error);
            }
        }
        if let Some(token) = self.ended_token.take() {
            if let Err(error) = self.player.RemoveMediaEnded(token) {
                first_error.get_or_insert(error);
            }
        }
        if let (Some(list), Some(token)) = (&self.list, self.changed_token.take()) {
            if let Err(error) = list.RemoveCurrentItemChanged(token) {
                first_error.get_or_insert(error);
            }
        }
        if let (Some(list), Some(token)) = (&self.list, self.item_failed_token.take()) {
            if let Err(error) = list.RemoveItemFailed(token) {
                first_error.get_or_insert(error);
            }
        }
        self.list = None;
        self.timed_job = None;
        self.timed = None;
        if let Some(streamed) = self.streamed.take() {
            streamed.bytes.cancel();
        }
        self.stream_job = None;
        self.timed_range = None;
        self.timed_progress = None;
        self.job = None;
        self.keys.clear();
        self.pending_keys.clear();
        self.opening_clip_surah = None;
        self.clip_durations.clear();
        self.preparation_error = None;
        self.preparation_complete.store(true, Ordering::Release);
        self.queue_ended.store(false, Ordering::Release);
        self.wanted_playing = false;
        self.attached = false;
        self.failed.store(false, Ordering::Release);
        self.finished.store(false, Ordering::Release);
        first_error.map_or(Ok(()), |error| Err(error.into()))
    }
    pub fn toggle(&mut self) -> Result<()> {
        if self.wanted_playing {
            self.wanted_playing = false;
            // Before any source exists this is a pending user preference;
            // Windows may still be resetting its previous decoder.
            if self.attached {
                self.player.Pause()?;
            }
        } else if !self.failed.load(Ordering::Acquire) {
            self.wanted_playing = true;
            if self.attached && !self.queue_ended.load(Ordering::Acquire) {
                self.player.Play()?;
            }
        }
        Ok(())
    }
    pub fn speed(&self, value: f64) -> Result<()> {
        let value = checked_rate(value)?;
        self.desired_rate.store(value);
        // Detaching a source resets the Windows media pipeline asynchronously.
        // SetPlaybackRate during that interval can return MF_E_INVALIDREQUEST.
        // Store the requested rate now; MediaOpened/CurrentItemChanged apply
        // it to the next source. Live, attached sources still update at once.
        if self.attached {
            if let Some(worker) = &self.live_rate {
                worker.wake()?;
            } else {
                self.desired_rate.apply(&self.player)?;
            }
        }
        Ok(())
    }
    /// Live form edits enqueue a bounded, coalescing rate command. The UI never
    /// detaches, seeks or waits for Media Foundation to reconfigure its decoder.
    pub fn request_speed(&mut self, value: f64) -> Result<()> {
        // The measured worker path is for a single continuous recording.
        // Keep the established per-ayah adapter until its native short-clip
        // rate/end boundary behavior has independent completion coverage.
        if self.timed.is_none() && self.streamed.is_none() {
            return self.speed(value);
        }
        self.desired_rate.store(checked_rate(value)?);
        if self.attached {
            if self.live_rate.is_none() {
                self.live_rate = Some(LiveRateWorker::new(
                    self.player.clone(),
                    self.desired_rate.clone(),
                )?);
            }
            self.live_rate.as_ref().unwrap().wake()?;
        }
        Ok(())
    }
    pub fn volume(&self, value: f64) -> Result<()> {
        self.player.SetVolume(checked_volume(value)?)?;
        Ok(())
    }
    pub fn repeat(&mut self, value: bool) -> Result<()> {
        self.repeat = value;
        if let Some(list) = &self.list {
            list.SetAutoRepeatEnabled(value && self.preparation_complete.load(Ordering::Acquire))?;
        }
        Ok(())
    }
    /// A retained player is idle after Stop. Paused, preparing, completed and
    /// failed sessions still own a playlist or job and keep their status visible.
    pub fn completed(&self) -> bool {
        self.finished.load(Ordering::Acquire) && !self.failed.load(Ordering::Acquire)
    }
    pub fn has_session(&self) -> bool {
        self.list.is_some()
            || self.job.is_some()
            || self.timed_job.is_some()
            || self.stream_job.is_some()
    }
    pub fn is_paused(&self) -> bool {
        !self.wanted_playing
    }
    pub fn needs_retry(&self) -> bool {
        // A failed lookahead request is not a failure of the complete files
        // already queued. Ask for intervention only at an actual stop boundary.
        self.preparation_error.is_some() && self.failed.load(Ordering::Acquire)
    }
    /// Terminal sessions keep their status, but need only idle UI polling.
    /// Paused or buffering sessions still observe preparation and decoder events.
    pub fn needs_fast_poll(&self) -> bool {
        if self.failed.load(Ordering::Acquire) {
            return false;
        }
        (self.job.is_some()
            && !self.preparation_complete.load(Ordering::Acquire)
            && self.preparation_error.is_none())
            || (self.list.is_some() && !self.finished.load(Ordering::Acquire))
    }
    /// The distinct unnumbered opening has a proven identity only when it is
    /// the explicit validated 1:1 clip before another chapter's first ayah.
    /// Publisher timing rows labelled zero and silence before the first verse
    /// do not identify Bismillah, so continuous recordings never infer it.
    pub fn opening_surah(&self) -> Option<u32> {
        let surah = self.opening_clip_surah?;
        if !self.attached
            || self.failed.load(Ordering::Acquire)
            || self.finished.load(Ordering::Acquire)
            || self.queue_ended.load(Ordering::Acquire)
            || self.timed.is_some()
            || self.streamed.is_some()
            || self.keys.first() != Some(&None)
            || self.list.as_ref()?.CurrentItemIndex().ok()? != 0
        {
            return None;
        }
        let session = self.player.PlaybackSession().ok()?;
        let duration = session.NaturalDuration().ok()?.Duration;
        let position = session.Position().ok()?.Duration;
        if duration <= 0
            || position < 0
            || position >= duration
            || !matches!(
                session.PlaybackState().ok()?,
                MediaPlaybackState::Playing
                    | MediaPlaybackState::Paused
                    | MediaPlaybackState::Buffering
            )
        {
            return None;
        }
        Some(surah)
    }
    #[cfg(feature = "qa")]
    pub fn qa_opening_clip(cache: Arc<AudioCache>, out: &std::path::Path) -> Result<()> {
        use std::{
            fs::{self, OpenOptions},
            io::{Read, Write},
            os::windows::fs::{MetadataExt, OpenOptionsExt},
            thread,
            time::{Duration, Instant},
        };
        #[derive(serde::Deserialize)]
        struct FixtureReceipt {
            schema: String,
            key: ClipKey,
            file: String,
            bytes: u64,
            frames: u32,
            sha256: String,
        }
        fn wait_for(
            audio: &mut Audio,
            deadline: Instant,
            predicate: impl Fn(&Audio) -> bool,
        ) -> Result<()> {
            loop {
                if !pump_messages() {
                    return Err("Opening clip QA interrupted by window shutdown".into());
                }
                audio.poll()?;
                if audio.failed.load(Ordering::Acquire) {
                    return Err(audio.status(false).into());
                }
                if predicate(audio) {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err("Opening clip native decoder exceeded five seconds".into());
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
        fn sample(audio: &Audio, stage: &str) -> Result<serde_json::Value> {
            let session = audio.player.PlaybackSession()?;
            Ok(serde_json::json!({
                "stage":stage,"openingSurah":audio.opening_surah(),"current":audio.current(),
                "nativeIndex":audio.list.as_ref().and_then(|list|list.CurrentItemIndex().ok()),
                "nativeState":session.PlaybackState()?.0,
                "nativePosition100ns":session.Position()?.Duration,
                "nativeDuration100ns":session.NaturalDuration()?.Duration,
            }))
        }
        fs::create_dir_all(out)?;
        let mut sources = Vec::new();
        let mut source_leases = Vec::new();
        let mut receipt_leases = Vec::new();
        // Preflight every requested clip and hold read leases. Missing or
        // changed fixtures fail before production preparation can use HTTPS.
        // Native playback still passes through the real cache worker/controller.
        for (surah, ayah) in [(1, 1), (112, 1)] {
            let prefix = format!("r00-s{surah:03}-a{ayah:03}");
            let metadata = cache.folder().join("Cache").join(format!("{prefix}.json"));
            let mut lease = OpenOptions::new()
                .read(true)
                .share_mode(1)
                .custom_flags(0x00200000)
                .open(&metadata)?;
            let attributes = lease.metadata()?;
            if !attributes.is_file()
                || attributes.file_attributes() & 0x400 != 0
                || attributes.len() > 4096
            {
                return Err("Opening clip fixture receipt is not a bounded regular file".into());
            }
            let mut bytes = Vec::new();
            (&mut lease).take(4097).read_to_end(&mut bytes)?;
            let receipt: FixtureReceipt = serde_json::from_slice(&bytes)?;
            if receipt.schema != "native-audio-clip-v1"
                || receipt.key != ClipKey::new(0, surah, ayah)?
                || receipt.sha256.len() != 64
                || !receipt
                    .sha256
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                || receipt.file != format!("{prefix}_{}.mp3", receipt.sha256)
                || receipt.bytes > crate::mp3::MAX_AUDIO_BYTES as u64
            {
                return Err(
                    "Opening clip fixture identity is not the original cache receipt".into(),
                );
            }
            let path = cache.folder().join("Cache").join(&receipt.file);
            let verified = crate::file_integrity::inspect(&path)?;
            if verified.sha256 != receipt.sha256
                || verified.info.bytes != receipt.bytes
                || verified.info.frames != receipt.frames
            {
                return Err("Opening clip fixture failed its source integrity check".into());
            }
            sources.push(serde_json::json!({
                "key":receipt.key,"file":receipt.file,"sha256":verified.sha256,
                "bytes":verified.info.bytes,"frames":verified.info.frames,
                "receiptSha256":crate::audio_cache::sha256(&bytes)?,
                "readLeaseHeld":true,
            }));
            source_leases.push(verified);
            receipt_leases.push(lease);
        }
        let mut audio = Self::with_cache(cache)?;
        let began = Instant::now();
        let deadline = began + Duration::from_secs(5);
        let mut samples = Vec::new();
        let result = (|| -> Result<()> {
            audio.play_range(
                ListeningRange {
                    reciter: 0,
                    surah: 112,
                    first: 1,
                    last: 1,
                    count: 4,
                },
                1.,
                0.,
            )?;
            wait_for(&mut audio, deadline, |audio| {
                audio.opening_surah() == Some(112)
                    && audio
                        .player
                        .PlaybackSession()
                        .and_then(|s| s.PlaybackState())
                        .is_ok_and(|state| state == MediaPlaybackState::Playing)
            })?;
            if audio.current().is_some() {
                return Err("Unnumbered Bismillah acquired a verse identity".into());
            }
            samples.push(sample(&audio, "opening-decoded")?);
            audio.toggle()?;
            wait_for(&mut audio, deadline, |audio| {
                audio
                    .player
                    .PlaybackSession()
                    .and_then(|s| s.PlaybackState())
                    .is_ok_and(|state| state == MediaPlaybackState::Paused)
            })?;
            let session = audio.player.PlaybackSession()?;
            let paused_position = session.Position()?.Duration;
            let pause_end = Instant::now() + Duration::from_millis(80);
            while Instant::now() < pause_end {
                if Instant::now() >= deadline || !pump_messages() {
                    return Err("Opening clip pause verification interrupted or timed out".into());
                }
                audio.poll()?;
                thread::sleep(Duration::from_millis(5));
            }
            if audio.opening_surah() != Some(112)
                || (session.Position()?.Duration - paused_position).abs() > 500_000
            {
                return Err("Paused Bismillah lost its identity or kept advancing".into());
            }
            samples.push(sample(&audio, "opening-paused")?);
            wait_for(&mut audio, deadline, |audio| audio.keys.len() == 2)?;
            audio.toggle()?;
            let duration = session.NaturalDuration()?.Duration;
            // Let MediaPlaybackList cross the real decoder end boundary. No
            // synthetic current-ayah or opening state is injected by this test.
            session.SetPosition(TimeSpan {
                Duration: (duration - 1_200_000).max(0),
            })?;
            wait_for(&mut audio, deadline, |audio| {
                audio.current() == Some((112, 1))
                    && audio.opening_surah().is_none()
                    && audio
                        .player
                        .PlaybackSession()
                        .and_then(|s| s.PlaybackState())
                        .is_ok_and(|state| state == MediaPlaybackState::Playing)
            })?;
            samples.push(sample(&audio, "native-advance-to-ayah")?);
            audio.stop()?;
            if audio.opening_surah().is_some() || audio.current().is_some() {
                return Err("Stop retained an opening or verse identity".into());
            }
            audio.play_range(
                ListeningRange {
                    reciter: 0,
                    surah: 1,
                    first: 1,
                    last: 1,
                    count: 7,
                },
                1.,
                0.,
            )?;
            wait_for(&mut audio, deadline, |audio| {
                audio.current() == Some((1, 1))
                    && audio.opening_surah().is_none()
                    && audio
                        .player
                        .PlaybackSession()
                        .and_then(|s| s.PlaybackState())
                        .is_ok_and(|state| state == MediaPlaybackState::Playing)
            })?;
            samples.push(sample(&audio, "fatiha-numbered-ayah")?);
            Ok(())
        })();
        let stopped = audio.stop();
        let cleared = audio.opening_surah().is_none() && audio.current().is_none();
        let report = serde_json::json!({
            "passed":result.is_ok() && stopped.is_ok() && cleared,
            "error":result.as_ref().err().map(|e|e.to_string()),
            "sources":sources,"samples":samples,"elapsedSeconds":began.elapsed().as_secs_f64(),
            "stopClearsOpening":cleared,"muted":true,"sourceBytesPreservedByReadLease":true,
            "continuousIntroInference":false,
            "nativeDecoderEndAdvancesToFirstAyah":samples.iter().any(|sample|sample["stage"] == "native-advance-to-ayah"),
        });
        let mut receipt = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out.join("opening-clip26.json"))?;
        receipt.write_all(&serde_json::to_vec_pretty(&report)?)?;
        result?;
        stopped?;
        if !cleared {
            return Err("Opening clip state survived final stop".into());
        }
        Ok(())
    }
    pub fn current(&self) -> Option<(u32, u32)> {
        if self.failed.load(Ordering::Acquire) {
            return None;
        }
        if let Some(s) = &self.streamed {
            let position = self
                .player
                .PlaybackSession()
                .ok()?
                .Position()
                .ok()?
                .Duration
                .max(0) as u64
                / 10_000;
            return s
                .timeline
                .at(position.saturating_add(s.offset_ms))
                .filter(|r| r.ayah >= s.first && r.ayah <= s.last)
                .map(|r| (s.surah, r.ayah));
        }
        if let Some(timed) = &self.timed {
            let position = self
                .player
                .PlaybackSession()
                .ok()?
                .Position()
                .ok()?
                .Duration
                .max(0) as u64
                / 10_000;
            return timed
                .ready
                .timeline
                .at(position.saturating_add(timed.offset_ms))
                .filter(|row| row.ayah >= timed.first && row.ayah <= timed.last)
                .map(|row| (timed.ready.surah, row.ayah));
        }
        self.list
            .as_ref()
            .and_then(|list| list.CurrentItemIndex().ok())
            .and_then(|i| self.keys.get(i as usize).copied())
            .flatten()
    }
    /// Immutable UI snapshot; never substitutes a clip clock for a surah clock.
    pub fn transport(&self) -> Option<Transport> {
        if self.failed.load(Ordering::Acquire) || self.list.is_none() {
            return None;
        }
        let session = self.player.PlaybackSession().ok()?;
        let position_ms = session.Position().ok()?.Duration.max(0) as u64 / 10_000;
        let range_ms = session.NaturalDuration().ok()?.Duration.max(0) as u64 / 10_000;
        if range_ms == 0 {
            return None;
        }
        let offset_ms = self.streamed.as_ref().map_or_else(
            || self.timed.as_ref().map_or(0, |t| t.offset_ms),
            |s| s.offset_ms,
        );
        if self.streamed.as_ref().is_some_and(|s| s.duration_ms == 0) {
            return None;
        }
        let duration_ms = self.timed.as_ref().map_or_else(
            || self.streamed.as_ref().map_or(range_ms, |s| s.duration_ms),
            |t| (t.ready._verified.info.duration_seconds * 1000.).round() as u64,
        );
        Some(Transport {
            position_ms: position_ms.min(range_ms),
            range_ms,
            offset_ms,
            duration_ms,
            continuous: self.timed.is_some() || self.streamed.is_some(),
            can_seek: !self.completed() && session.CanSeek().unwrap_or(false),
        })
    }
    /// Seek only inside the current validated item/range. This retains the
    /// original media source, listening plan, rate and pause intent.
    pub fn seek(&mut self, delta_ms: Option<i64>, slider: u32) -> Result<bool> {
        let Some(t) = self.transport().filter(|t| t.can_seek) else {
            return Ok(false);
        };
        let position = delta_ms.map_or_else(
            || {
                crate::listening_layout::slider_target(
                    slider,
                    t.duration_ms,
                    t.offset_ms,
                    t.range_ms,
                )
            },
            |delta| crate::listening_layout::seek_target(t.position_ms, t.range_ms, delta),
        );
        self.player.PlaybackSession()?.SetPosition(TimeSpan {
            Duration: position.saturating_mul(10_000).min(i64::MAX as u64) as i64,
        })?;
        Ok(true)
    }
    pub fn status(&self, arabic: bool) -> String {
        if self.failed.load(Ordering::Acquire) {
            let detail = self.preparation_error.clone().unwrap_or_else(|| {
                self.failure_detail
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            });
            return format!(
                "{} {}",
                if arabic {
                    "تعذّر تشغيل التسجيل. أعد المحاولة."
                } else {
                    "Could not play this recording. Please retry."
                },
                detail
            );
        }
        if (self.timed_job.is_some() || self.stream_job.is_some()) && self.keys.is_empty() {
            let label = if arabic {
                "جارٍ تحميل التلاوة…"
            } else {
                "Loading recitation…"
            };
            return if let Some((bytes, total)) = self.timed_progress {
                format!(
                    "{label} {:.1} MiB{}",
                    bytes as f64 / (1024. * 1024.),
                    total
                        .map(|n| format!(" / {:.1} MiB", n as f64 / (1024. * 1024.)))
                        .unwrap_or_default()
                )
            } else {
                label.to_owned()
            };
        }
        if self.job.is_some()
            && (self.keys.is_empty()
                || (self.queue_ended.load(Ordering::Acquire)
                    && !self.preparation_complete.load(Ordering::Acquire)))
        {
            return if arabic {
                if self.wanted_playing {
                    "جارٍ تحميل التلاوة…"
                } else {
                    "متوقف مؤقتًا — جارٍ تحميل التلاوة…"
                }
            } else if self.wanted_playing {
                "Loading recitation…"
            } else {
                "Paused — loading recitation…"
            }
            .to_owned();
        }
        if self.list.is_none() {
            return String::new();
        }
        if self.finished.load(Ordering::Acquire) {
            return if arabic {
                "اكتمل التشغيل"
            } else {
                "Playback finished"
            }
            .to_owned();
        }
        let Ok(session) = self.player.PlaybackSession() else {
            return String::new();
        };
        let state = session.PlaybackState().unwrap_or(MediaPlaybackState::None);
        let label = match (state, arabic) {
            (MediaPlaybackState::Playing, true) => "يتم التشغيل",
            (MediaPlaybackState::Paused, true) => "متوقف مؤقتًا",
            (MediaPlaybackState::Playing, false) => "Playing",
            (MediaPlaybackState::Paused, false) => "Paused",
            (_, true) => "جارٍ التحميل",
            (_, false) => "Buffering",
        };
        let relative_ms = session
            .Position()
            .map(|t| t.Duration.max(0) as u64 / 10_000)
            .unwrap_or(0);
        let range_duration_ms = session
            .NaturalDuration()
            .map(|t| t.Duration.max(0) as u64 / 10_000)
            .unwrap_or(0);
        let rate = self.desired_rate.value();
        let chapter_clock = self.timed.as_ref().map(|timed| {
            crate::timed_audio::PlaybackClock::chapter(
                relative_ms,
                timed.offset_ms,
                (timed.ready._verified.info.duration_seconds * 1000.).round() as u64,
                range_duration_ms,
                rate,
            )
        });
        let pos = chapter_clock
            .as_ref()
            .map_or(relative_ms, |clock| clock.chapter_position_ms)
            / 1000;
        let duration = chapter_clock
            .as_ref()
            .map_or(range_duration_ms, |clock| clock.chapter_duration_ms)
            / 1000;
        let key = self
            .current()
            .map(|(s, a)| format!("{s}:{a} · "))
            .unwrap_or_default();
        let clock_label = if chapter_clock.is_some() {
            if arabic { "السورة " } else { "Surah " }
        } else {
            ""
        };
        let mut message = format!(
            "{key}{label} · {clock_label}{}:{:02} / {}:{:02}",
            pos / 60,
            pos % 60,
            duration / 60,
            duration % 60
        );
        if let Some(clock) = chapter_clock.filter(|_| range_duration_ms > 0) {
            let seconds = clock.remaining_ms.div_ceil(1000);
            message.push_str(&format!(
                " · {} {}:{:02}",
                if arabic {
                    "الوقت المتبقي"
                } else {
                    "Time remaining"
                },
                seconds / 60,
                seconds % 60
            ));
        } else if self.preparation_complete.load(Ordering::Acquire)
            && !self.clip_durations.is_empty()
        {
            let index = self
                .list
                .as_ref()
                .and_then(|list| list.CurrentItemIndex().ok())
                .unwrap_or(0) as usize;
            let remaining = (self.clip_durations.iter().skip(index).sum::<f64>()
                - relative_ms as f64 / 1000.)
                .max(0.0)
                / rate;
            let seconds = remaining.ceil() as u64;
            message.push_str(&format!(
                " · {} {}:{:02}",
                if arabic {
                    "الوقت المتبقي"
                } else {
                    "Time remaining"
                },
                seconds / 60,
                seconds % 60
            ));
        }
        message
    }
    #[cfg(feature = "qa")]
    pub fn timed_smoke(&mut self, folder: &std::path::Path) -> Result<()> {
        use crate::timed_audio::{Recording, Timeline};
        use std::{
            fs, thread,
            time::{Duration, Instant},
        };
        let recording = Recording::for_reciter(0).ok_or("Missing bound recording")?;
        let catalogue = fs::read(folder.join("reads.json"))?;
        recording.verify_catalog(&catalogue)?;
        let raw = fs::read(folder.join("yasser-112-timings.json"))?;
        let timeline = Timeline::parse(&raw, crate::audio_cache::COUNTS[111])?;
        let path = folder.join("yasser-112.mp3");
        if fs::metadata(&path)?.len() > crate::mp3::MAX_AUDIO_BYTES as u64 {
            return Err("Prototype recording exceeds its limit".into());
        }
        let bytes = fs::read(&path)?;
        let info = crate::mp3::inspect(&bytes)?;
        timeline.validate_duration((info.duration_seconds * 1000.).round() as u64)?;
        if timeline.range(1, 4)? != (0, None) {
            return Err("Whole chapter cannot truncate its source tail".into());
        }
        self.set_sources(vec![(local_uri(&path)?, None)], 1.0, 0.0)?;
        let trace = PlaylistTrace::new(self.list.as_ref().ok_or("Missing prototype source")?)?;
        let begin = Instant::now();
        let mut observed = Vec::new();
        let mut samples = Vec::new();
        let mut paused = false;
        let mut decoded_duration = 0.;
        let result = (|| -> Result<()> {
            while begin.elapsed() < Duration::from_secs(30) {
                pump_messages();
                self.poll()?;
                if self.failed.load(Ordering::Acquire) {
                    return Err(self.status(false).into());
                }
                let session = self.player.PlaybackSession()?;
                let duration = session.NaturalDuration()?.Duration;
                if duration > 0 {
                    timeline.validate_duration((duration / 10_000) as u64)?;
                    decoded_duration = duration as f64 / 10_000_000.;
                }
                if session.PlaybackState()? == MediaPlaybackState::Playing {
                    let position = (session.Position()?.Duration.max(0) / 10_000) as u64;
                    let ayah = timeline.at(position).map(|row| row.ayah);
                    if let Some(ayah) = ayah {
                        if observed.last() != Some(&ayah) {
                            observed.push(ayah);
                        }
                    }
                    samples.push(serde_json::json!({"elapsedSeconds":begin.elapsed().as_secs_f64(),"sourcePositionMs":position,"ayah":ayah}));
                    if !paused && position > 600 && position < 1500 {
                        self.toggle()?;
                        let before = session.Position()?.Duration;
                        let pause = Instant::now();
                        while pause.elapsed() < Duration::from_millis(200) {
                            pump_messages();
                            thread::sleep(Duration::from_millis(5));
                        }
                        if (session.Position()?.Duration - before).abs() > 500_000 {
                            return Err("Timed source pause advanced playback".into());
                        }
                        self.speed(1.25)?;
                        self.poll()?;
                        if (session.PlaybackRate()? - 1.25).abs() > 0.01 {
                            return Err("Timed source rate change failed".into());
                        }
                        self.speed(1.0)?;
                        self.toggle()?;
                        paused = true;
                    }
                }
                if self.finished.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            if observed != vec![1, 2, 3, 4] || !paused || !self.finished.load(Ordering::Acquire) {
                return Err(format!(
                    "Incomplete timed playback: ayahs={observed:?}, paused={paused}, ended={}",
                    self.finished.load(Ordering::Acquire)
                )
                .into());
            }
            if self
                .list
                .as_ref()
                .ok_or("Missing prototype source")?
                .Items()?
                .Size()?
                != 1
            {
                return Err("Timed source switched recordings between ayahs".into());
            }
            Ok(())
        })();
        let report = serde_json::json!({"passed":result.is_ok(),"error":result.as_ref().err().map(|e|e.to_string()),"read":recording.read,"recordingUrl":recording.audio_url(112)?,"observedAyahs":observed,"pauseResume":paused,"decodedDurationSeconds":decoded_duration,"inspectedDurationSeconds":info.duration_seconds,"frames":info.frames,"samples":samples,"sourceEvents":trace.snapshot()?,"singleCompleteRecording":true,"fullChapterUsesDecodedEnd":self.finished.load(Ordering::Acquire),"muted":true,"rangePlan2To3":timeline.range(2,3)?,"scope":"Native single-recording decoder prototype; not yet connected to the listening UI, large-surah cache or long-session validation"});
        let stopped = self.stop();
        fs::write(
            folder.join("native-timed-audio.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        result?;
        stopped?;
        Ok(())
    }
    #[cfg(feature = "qa")]
    pub fn timed_store_smoke(folder: &std::path::Path) -> Result<()> {
        use std::{
            fs, thread,
            time::{Duration, Instant},
        };
        fs::create_dir_all(folder)?;
        let cache = AudioCache::new(folder.join("profile"))?;
        let mut audio = Self::with_cache(cache)?;
        let mut cases = Vec::new();
        for (reciter, first, last) in [(0, 1, 4), (0, 2, 3), (0, 3, 4), (5, 1, 4)] {
            audio.play_timed(
                ListeningRange {
                    reciter,
                    surah: 112,
                    first,
                    last,
                    count: 4,
                },
                1.0,
                0.0,
            )?;
            let begin = Instant::now();
            let mut observed = Vec::new();
            let mut samples = Vec::new();
            while begin.elapsed() < Duration::from_secs(45) {
                pump_messages();
                audio.poll()?;
                if audio.failed.load(Ordering::Acquire) {
                    return Err(audio.status(false).into());
                }
                let session = audio.player.PlaybackSession()?;
                if session.PlaybackState()? == MediaPlaybackState::Playing {
                    let key = audio.current();
                    if let Some((_, ayah)) = key {
                        if observed.last() != Some(&ayah) {
                            observed.push(ayah);
                        }
                    }
                    samples.push(serde_json::json!({"elapsed":begin.elapsed().as_secs_f64(),"rawPosition":session.Position()?.Duration,"nativeDuration":session.NaturalDuration()?.Duration,"current":key}));
                }
                if audio.finished.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            cases.push(serde_json::json!({"reciter":reciter,"first":first,"last":last,"observedAyahs":observed,"finished":audio.finished.load(Ordering::Acquire),"samples":samples}));
            fs::write(
                folder.join("native-timed-store-audio.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"cases":cases,"muted":true,"scope":"Production continuous worker/controller, whole-source and native clipped-range decoder"}),
                )?,
            )?;
            if observed != (first..=last).collect::<Vec<_>>()
                || !audio.finished.load(Ordering::Acquire)
            {
                return Err(format!("Timed range mismatch {first}..{last}: {observed:?}").into());
            }
        }
        audio.stop()?;
        // The cases above now exercise online byte-range playback. File lease,
        // offline repeat and changed-receipt assertions below require an
        // explicitly saved original bundle, not an automatic streaming cache.
        let offline_job = audio
            .cache
            .as_ref()
            .ok_or("Missing fixture cache")?
            .timed_store()?
            .prepare(0, 112, 999)?;
        let waiting = Instant::now();
        loop {
            if let Some(event) = offline_job.try_recv() {
                match event {
                    crate::timed_store::Event::Ready(ready) => {
                        drop(ready);
                        break;
                    }
                    crate::timed_store::Event::Failed(_, message) => return Err(message.into()),
                    _ => {}
                }
            }
            if waiting.elapsed() > Duration::from_secs(25) {
                return Err("Explicit offline fixture did not become ready".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        drop(offline_job);
        let range = ListeningRange {
            reciter: 0,
            surah: 112,
            first: 2,
            last: 3,
            count: 4,
        };
        audio.play_timed(range, 1.0, 0.0)?;
        audio.toggle()?;
        let waiting = Instant::now();
        while !audio.attached && waiting.elapsed() < Duration::from_secs(10) {
            pump_messages();
            audio.poll()?;
            thread::sleep(Duration::from_millis(10));
        }
        if !audio.attached
            || audio.wanted_playing
            || audio.player.PlaybackSession()?.Position()?.Duration > 100_000
        {
            return Err("Timed pause-before-ready was not retained".into());
        }
        let path = audio
            .timed
            .as_ref()
            .ok_or("Missing timed lease")?
            .ready
            .path
            .clone();
        if fs::OpenOptions::new().write(true).open(&path).is_ok() {
            return Err("Active original recording can be replaced during playback".into());
        }
        audio.repeat(true)?;
        audio.toggle()?;
        let begin = Instant::now();
        let mut repeated = Vec::new();
        while begin.elapsed() < Duration::from_secs(12) {
            pump_messages();
            audio.poll()?;
            if let Some((_, ayah)) = audio.current() {
                if repeated.last() != Some(&ayah) {
                    repeated.push(ayah);
                }
            }
            if repeated.len() >= 3 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if repeated != vec![2, 3, 2] {
            return Err(format!("Timed native repeat failed: {repeated:?}").into());
        }
        audio.stop()?;
        audio.repeat(false)?;
        audio.play_timed(range, 1.0, 0.0)?;
        audio.stop()?;
        let wait = Instant::now();
        while wait.elapsed() < Duration::from_millis(250) {
            pump_messages();
            audio.poll()?;
            thread::sleep(Duration::from_millis(10));
        }
        if audio.has_session() || audio.current().is_some() {
            return Err("Cancelled timed session attached stale preparation".into());
        }
        let receipt_path = path.with_file_name("receipt.json");
        let original = fs::read(&receipt_path)?;
        let mut altered: serde_json::Value = serde_json::from_slice(&original)?;
        altered["sha256"] = serde_json::Value::String("0".repeat(64));
        fs::write(&receipt_path, serde_json::to_vec(&altered)?)?;
        audio.play_timed(range, 1.0, 0.0)?;
        let begin = Instant::now();
        while !audio.failed.load(Ordering::Acquire) && begin.elapsed() < Duration::from_secs(5) {
            pump_messages();
            audio.poll()?;
            thread::sleep(Duration::from_millis(10));
        }
        let corrupt_rejected = audio.failed.load(Ordering::Acquire) && !audio.attached;
        audio.stop()?;
        fs::write(&receipt_path, &original)?;
        if !corrupt_rejected {
            return Err("Modified cache receipt was accepted".into());
        }
        fs::write(
            folder.join("native-timed-store-audio.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"passed":true,"cases":cases,"muted":true,"pauseBeforeReady":true,"activeRecordingWriteLease":true,"repeatAyahs":repeated,"cancelledSessionCannotAttach":true,"modifiedReceiptRejectedAndPreserved":true,"scope":"Production continuous worker/controller, whole chapter, native range clipping, repeat and cache integrity; not long-session coverage"}),
            )?,
        )?;
        Ok(())
    }
    #[cfg(feature = "qa")]
    pub fn long_timed_smoke(folder: &std::path::Path) -> Result<()> {
        use std::{
            fs, thread,
            time::{Duration, Instant},
        };
        fs::create_dir_all(folder)?;
        let cache = AudioCache::new(folder.join("profile"))?;
        let mut audio = Self::with_cache(cache)?;
        let scoped = std::env::args().any(|arg| arg == "--qa-long-scoped");
        audio.play_timed_using(
            ListeningRange {
                reciter: 0,
                surah: 2,
                first: 1,
                last: 286,
                count: 286,
            },
            1.,
            0.,
            scoped,
        )?;
        let preparing = Instant::now();
        while !audio.attached && preparing.elapsed() < Duration::from_secs(180) {
            pump_messages();
            audio.poll()?;
            if audio.failed.load(Ordering::Acquire) {
                return Err(audio.status(false).into());
            }
            thread::sleep(Duration::from_millis(25));
        }
        if !audio.attached {
            return Err("Long recording did not become ready".into());
        }
        let ready = &audio
            .timed
            .as_ref()
            .ok_or("Missing long timed source")?
            .ready;
        let path = ready.path.clone();
        let original_hash = ready._verified.sha256.clone();
        if original_hash != "d390ae18bf320e6073e4736a72e8dd4a24ea363a80ab4696281e2b990d2aeb25" {
            return Err("Long recording differs from the previously verified original".into());
        }
        let original_bytes = ready._verified.info.bytes;
        let initial_memory = crate::streamed_audio_qa::memory()?;
        let began = Instant::now();
        let mut last_sample = Instant::now();
        let mut samples = Vec::new();
        let mut observed = Vec::new();
        let mut paused = false;
        let mut rate_up = false;
        let mut rate_restored = false;
        let result = (|| -> Result<()> {
            while began.elapsed() < Duration::from_secs(1200) {
                pump_messages();
                audio.poll()?;
                if audio.failed.load(Ordering::Acquire) || audio.finished.load(Ordering::Acquire) {
                    return Err(
                        format!("Long playback ended early: {}", audio.status(false)).into(),
                    );
                }
                let session = audio.player.PlaybackSession()?;
                if let Some((_, ayah)) = audio.current() {
                    if observed.last() != Some(&ayah) {
                        observed.push(ayah);
                    }
                }
                let elapsed = began.elapsed().as_secs_f64();
                if !paused && elapsed >= 60. {
                    audio.toggle()?;
                    let wait = Instant::now();
                    while session.PlaybackState()? != MediaPlaybackState::Paused
                        && wait.elapsed() < Duration::from_secs(2)
                    {
                        pump_messages();
                        thread::sleep(Duration::from_millis(10));
                    }
                    if session.PlaybackState()? != MediaPlaybackState::Paused {
                        return Err("Long-session pause failed".into());
                    }
                    let position = session.Position()?.Duration;
                    let wait = Instant::now();
                    while wait.elapsed() < Duration::from_secs(2) {
                        pump_messages();
                        audio.poll()?;
                        thread::sleep(Duration::from_millis(20));
                    }
                    if (session.Position()?.Duration - position).abs() > 100_000 {
                        return Err("Paused long-session clock kept moving".into());
                    }
                    audio.toggle()?;
                    paused = true;
                }
                if !rate_up && elapsed >= 120. {
                    audio.qa_request_and_wait_rate(1.25)?;
                    rate_up = true;
                }
                if !rate_restored && elapsed >= 180. {
                    audio.qa_request_and_wait_rate(1.)?;
                    rate_restored = true;
                }
                if last_sample.elapsed() >= Duration::from_secs(5) {
                    let memory = crate::streamed_audio_qa::memory()?;
                    let items = audio
                        .list
                        .as_ref()
                        .ok_or("Missing long source list")?
                        .Items()?
                        .Size()?;
                    if items != 1 {
                        return Err("Long playback switched source items".into());
                    }
                    let rate = session.PlaybackRate()?;
                    let expected = if rate_up && !rate_restored { 1.25 } else { 1. };
                    if (rate - expected).abs() > 0.01 {
                        return Err("Long-session speed was not retained".into());
                    }
                    samples.push(serde_json::json!({"elapsedSeconds":elapsed,"positionSeconds":session.Position()?.Duration as f64/10_000_000.,"rate":rate,"ayah":audio.current(),"workingSetBytes":memory.WorkingSetSize,"privateBytes":memory.PagefileUsage,"sourceItems":items}));
                    fs::write(
                        folder.join("long-audio-progress.json"),
                        serde_json::to_vec_pretty(
                            &serde_json::json!({"completed":false,"elapsedSeconds":elapsed,"observedAyahs":observed,"latest":samples.last(),"muted":true}),
                        )?,
                    )?;
                    last_sample = Instant::now();
                }
                thread::sleep(Duration::from_millis(25));
            }
            if observed.len() < 20
                || observed.windows(2).any(|pair| pair[1] < pair[0])
                || audio.player.PlaybackSession()?.Position()?.Duration < 1100 * 10_000_000
            {
                return Err(
                    "Long recording failed to advance through original timing intervals".into(),
                );
            }
            Ok(())
        })();
        let verified = crate::file_integrity::inspect(&path)?;
        let unchanged = verified.sha256 == original_hash && verified.info.bytes == original_bytes;
        let stopped = audio.stop();
        drop(verified);
        drop(audio);
        if scoped {
            let cleanup = Instant::now();
            while fs::read_dir(folder.join("profile/AudioStore/ScopedContinuous"))?.any(|entry| {
                entry.is_ok_and(|entry| entry.file_name().to_string_lossy().starts_with("session-"))
            }) {
                if cleanup.elapsed() > Duration::from_secs(20) {
                    return Err("Long scoped recording did not retire after Stop".into());
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
        let report = serde_json::json!({"passed":result.is_ok() && stopped.is_ok() && unchanged,"scopedStorage":scoped,"ownedRecordingRetiredAfterStop":scoped,"error":result.as_ref().err().map(|e|e.to_string()),"elapsedSeconds":began.elapsed().as_secs_f64(),"preparationSeconds":preparing.elapsed().as_secs_f64()-began.elapsed().as_secs_f64(),"recordingSha256":original_hash,"originalBytes":original_bytes,"sourceBytesUnchanged":unchanged,"observedAyahs":observed,"pauseResume":paused,"speedChangedAndRestored":rate_up&&rate_restored,"initialWorkingSetBytes":initial_memory.WorkingSetSize,"samples":samples,"muted":true,"scope":"Twenty real minutes through the production continuous worker/controller and native decoder; not full chapter completion, live GUI, device interruption or Windows 10/N coverage"});
        fs::write(
            folder.join("native-long-timed-audio.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        result?;
        stopped?;
        if !unchanged {
            return Err("Original recording changed during the long session".into());
        }
        Ok(())
    }
    #[cfg(feature = "qa")]
    pub fn network_smoke(&mut self, folder: &std::path::Path) -> Result<serde_json::Value> {
        use std::{
            fs, thread,
            time::{Duration, Instant},
        };
        let args: Vec<_> = std::env::args().collect();
        let fixture = args
            .iter()
            .position(|a| a == "--qa-network-range")
            .map(|i| -> Result<(usize, u32, u32, u32)> {
                Ok((
                    args.get(i + 1).ok_or("Missing reciter")?.parse()?,
                    args.get(i + 2).ok_or("Missing surah")?.parse()?,
                    args.get(i + 3).ok_or("Missing first ayah")?.parse()?,
                    args.get(i + 4).ok_or("Missing last ayah")?.parse()?,
                ))
            })
            .transpose()?
            .unwrap_or((0, 112, 1, 4));
        let (reciter, surah, first, last) = fixture;
        let count = *crate::audio_cache::COUNTS
            .get(surah.saturating_sub(1) as usize)
            .ok_or("Invalid test surah")?;
        validate_range(reciter, surah, first, last, count)?;
        let expected: Vec<_> = (first..=last).collect();
        fs::create_dir_all(folder)?;
        self.cache = Some(AudioCache::new(folder.join("validated-network-cache"))?);
        self.repeat(false)?;
        let request_started = Instant::now();
        self.play_range(
            ListeningRange {
                reciter,
                surah,
                first,
                last,
                count,
            },
            1.5,
            0.0,
        )?;
        let ended = Arc::new(AtomicBool::new(false));
        let completed = ended.clone();
        let token =
            self.player
                .MediaEnded(&TypedEventHandler::<MediaPlayer, IInspectable>::new(
                    move |_, _| {
                        completed.store(true, Ordering::Release);
                        Ok(())
                    },
                ))?;
        struct CompletionEvent(MediaPlayer, i64);
        impl Drop for CompletionEvent {
            fn drop(&mut self) {
                let _ = self.0.RemoveMediaEnded(self.1);
            }
        }
        let completion = CompletionEvent(self.player.clone(), token);
        let begin = Instant::now();
        let mut observed = Vec::new();
        let mut boundaries = Vec::new();
        let mut samples = Vec::new();
        let mut previous: Option<((u32, u32), i64, i64)> = None;
        let mut paused = false;
        let mut rates_ok = true;
        let mut last_progress = Instant::now();
        let mut last_position = 0;
        let mut first_playback_ms = None;
        let result = (|| -> Result<()> {
            while begin.elapsed() < Duration::from_secs(240) {
                pump_messages();
                self.poll()?;
                if self.failed.load(Ordering::Acquire) {
                    return Err(self.status(false).into());
                }
                let session = self.player.PlaybackSession()?;
                if session.PlaybackState()? == MediaPlaybackState::Playing {
                    first_playback_ms
                        .get_or_insert_with(|| request_started.elapsed().as_secs_f64() * 1000.);
                    let key = self.current();
                    let position = session.Position()?.Duration;
                    let duration = session.NaturalDuration()?.Duration;
                    if samples.len() < 9000 {
                        samples.push(serde_json::json!({"elapsed":begin.elapsed().as_secs_f64(),"key":key,"position":position as f64/10_000_000.,"duration":duration as f64/10_000_000.}));
                    }
                    rates_ok &= (session.PlaybackRate()? - 1.5).abs() < 0.01;
                    if let Some(key) = key {
                        if previous.is_some_and(|(last, _, _)| last != key) {
                            let (last, pos, dur) = previous.unwrap();
                            let remaining = (dur - pos).max(0) as f64 / 10_000_000.;
                            boundaries.push(serde_json::json!({"ayah":format!("{}:{}",last.0,last.1),"lastObservedPosition":pos as f64/10_000_000.,"naturalDuration":dur as f64/10_000_000.,"unobservedTailSeconds":remaining}));
                            if dur == 0 || remaining > 0.3 {
                                return Err(format!("Native network ayah boundary moved early at {}:{}; {remaining:.3}s tail",last.0,last.1).into());
                            }
                        }
                        if observed.last() != Some(&key.1) {
                            observed.push(key.1);
                            last_progress = Instant::now();
                            last_position = 0;
                        }
                        if position > last_position + 100_000 {
                            last_position = position;
                            last_progress = Instant::now();
                        }
                        previous = Some(network_progress_sample(previous, key, position, duration));
                        if !paused && position > 5_000_000 && duration - position > 5_000_000 {
                            self.toggle()?;
                            let before = session.Position()?.Duration;
                            let pause = Instant::now();
                            while pause.elapsed() < Duration::from_millis(200) {
                                pump_messages();
                                thread::sleep(Duration::from_millis(5));
                            }
                            let after = session.Position()?.Duration;
                            if (after - before).abs() > 500_000 {
                                return Err("Native network pause advanced playback".into());
                            }
                            self.speed(1.25)?;
                            if (session.PlaybackRate()? - 1.25).abs() > 0.01 {
                                return Err("Native network speed adjustment failed".into());
                            }
                            self.speed(1.5)?;
                            self.toggle()?;
                            paused = true;
                            last_progress = Instant::now();
                        }
                    }
                }
                if ended.load(Ordering::Acquire)
                    && self.preparation_complete.load(Ordering::Acquire)
                    && self.keys.iter().flatten().count() == expected.len()
                    && self.finished.load(Ordering::Acquire)
                {
                    break;
                }
                if last_progress.elapsed() > Duration::from_secs(25) {
                    return Err("Native network playback stalled for 25 seconds".into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            if let Some((last, pos, dur)) = previous {
                let remaining = (dur - pos).max(0) as f64 / 10_000_000.;
                boundaries.push(serde_json::json!({"ayah":format!("{}:{}",last.0,last.1),"lastObservedPosition":pos as f64/10_000_000.,"naturalDuration":dur as f64/10_000_000.,"unobservedTailSeconds":remaining}));
                if dur == 0 || remaining > 0.3 {
                    return Err(
                        "Native final network ayah ended before its observed duration".into(),
                    );
                }
            }
            if observed != expected || !ended.load(Ordering::Acquire) || !rates_ok || !paused {
                return Err(format!("Incomplete native network playback: {observed:?}, completed={}, rate={rates_ok}, pause={paused}",ended.load(Ordering::Acquire)).into());
            }
            Ok(())
        })();
        drop(completion);
        let stop = self.stop();
        let report = serde_json::json!({"status":if result.is_ok(){"passed"}else{"failed"},"error":result.as_ref().err().map(|e|e.to_string()),"provider":if RECITERS[reciter].everyayah { "EveryAyah" } else { "Quran Foundation" },"reciter":RECITERS[reciter].english,"surah":surah,"firstPlaybackMs":first_playback_ms,"startupMeasurement":"Request accepted to first native Playing observation; muted complete local file playback, excludes physical input and audible-device latency","expectedAyahs":expected,"observedAyahs":observed,"boundaries":boundaries,"samples":samples,"boundaryMeasurement":"Greatest observed position and its paired duration per item, requiring a sample near each item start; this fixture never seeks","persistentRate":1.5,"ratesMatched":rates_ok,"pauseResume":paused,"completed":ended.load(Ordering::Acquire),"seconds":begin.elapsed().as_secs_f64(),"muted":true,"completeFileValidation":true,"localPlaybackOnly":true,"scope":"Authentic production download-worker and native-decoder range; intentional provider recording boundaries remain separate"});
        fs::write(
            folder.join("native-network-audio.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        result?;
        stop?;
        Ok(report)
    }
    #[cfg(feature = "qa")]
    pub fn smoke(&mut self, folder: &std::path::Path) -> Result<serde_json::Value> {
        use std::{
            fs, thread,
            time::{Duration, Instant},
        };
        fs::create_dir_all(folder)?;
        let path = folder.join("native-audio-test.wav");
        // A silent test fixture, not Quran audio. Six 600 ms items exercise the
        // actual Windows decoder, playlist boundaries, rate and pause/resume.
        let samples = vec![0u8; 48_000 * 2 * 6 / 10];
        let size = samples.len() as u32;
        let mut wav = b"RIFF".to_vec();
        wav.extend((36 + size).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(48_000u32.to_le_bytes());
        wav.extend(96_000u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(size.to_le_bytes());
        wav.extend(samples);
        fs::write(&path, wav)?;
        let uri = format!("file:///{}", path.to_string_lossy().replace('\\', "/"));
        self.set_sources(
            (1..=6).map(|n| (uri.clone(), Some((1, n)))).collect(),
            1.5,
            0.0,
        )?;
        let begin = Instant::now();
        let mut observed = Vec::new();
        let mut rate_ok = true;
        let mut paused = false;
        let ended = Arc::new(AtomicBool::new(false));
        let completed = ended.clone();
        let end_token =
            self.player
                .MediaEnded(&TypedEventHandler::<MediaPlayer, IInspectable>::new(
                    move |_, _| {
                        completed.store(true, Ordering::Release);
                        Ok(())
                    },
                ))?;
        struct CompletionEvent(MediaPlayer, i64);
        impl Drop for CompletionEvent {
            fn drop(&mut self) {
                let _ = self.0.RemoveMediaEnded(self.1);
            }
        }
        let completion_event = CompletionEvent(self.player.clone(), end_token);
        let mut updated_rate = false;
        let mut rate_observations = Vec::new();
        while begin.elapsed() < Duration::from_secs(10) {
            pump_messages();
            if self.player.PlaybackSession()?.PlaybackState()? == MediaPlaybackState::Playing {
                if let Some(key) = self.current() {
                    if observed.last() != Some(&key.1) {
                        observed.push(key.1);
                        let expected = if updated_rate { 1.25 } else { 1.5 };
                        let session = self.player.PlaybackSession()?;
                        let actual = session.PlaybackRate()?;
                        rate_observations.push(serde_json::json!({"ayah":key.1,"actual":actual,"expected":expected,"position":session.Position()?.Duration}));
                        rate_ok &= (actual - expected).abs() < 0.01;
                    }
                }
            }
            if observed.len() == 2 && !paused {
                self.player.Pause()?;
                let before = self.player.PlaybackSession()?.Position()?.Duration;
                let pause = Instant::now();
                while pause.elapsed() < Duration::from_millis(100) {
                    pump_messages();
                    thread::sleep(Duration::from_millis(5));
                }
                let after = self.player.PlaybackSession()?.Position()?.Duration;
                if (after - before).abs() > 500_000 {
                    return Err("Native pause advanced playback".into());
                }
                if !self.has_session() || !self.needs_fast_poll() {
                    return Err("Paused playback lost its session or fast polling".into());
                }
                self.player.Play()?;
                paused = true;
            }
            if observed.len() == 3 && !updated_rate {
                self.speed(1.25)?;
                updated_rate = true;
            }
            if ended.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let fully_completed = ended.load(Ordering::Acquire);
        let session = self.player.PlaybackSession()?;
        fs::write(
            folder.join("native-short-clip-completion.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "mediaEndedObserved":fully_completed,"items":observed,"position100ns":session.Position()?.Duration,
                "duration100ns":session.NaturalDuration()?.Duration,"state":format!("{:?}",session.PlaybackState()?),
                "elapsedSeconds":begin.elapsed().as_secs_f64(),"rateMatched":rate_ok,"pausePassed":paused,
                "scope":"Existing strict six 600 ms WAV fixture; terminal event remains required. No production source data."
            }))?,
        )?;

        self.poll()?;
        if fully_completed
            && (!self.has_session()
                || self.needs_fast_poll()
                || self.status(false) != "Playback finished")
        {
            return Err("Completed playback lost its status or retained fast polling".into());
        }
        drop(completion_event);
        self.stop()?;
        fs::write(
            folder.join("native-rate-observations.json"),
            serde_json::to_vec_pretty(&rate_observations)?,
        )?;
        if observed != vec![1, 2, 3, 4, 5, 6] || !rate_ok || !paused || !fully_completed {
            return Err(format!(
                "Native audio test failed: {observed:?}, rate={rate_ok}, pause={paused}"
            )
            .into());
        }
        self.repeat(true)?;
        self.set_sources(
            (1..=2).map(|n| (uri.clone(), Some((1, n)))).collect(),
            2.0,
            0.0,
        )?;
        let begin = Instant::now();
        let mut repeated = Vec::new();
        while begin.elapsed() < Duration::from_secs(5) {
            pump_messages();
            if self.player.PlaybackSession()?.PlaybackState()? == MediaPlaybackState::Playing {
                if let Some((_, item)) = self.current() {
                    if repeated.last() != Some(&item) {
                        repeated.push(item);
                    }
                }
            }
            if repeated == vec![1, 2, 1] {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.stop()?;
        self.repeat(false)?;
        if repeated != vec![1, 2, 1] {
            return Err("Native range repeat did not wrap in order".into());
        }
        let missing = format!(
            "file:///{}",
            folder
                .join("missing-ayah-for-error-test.wav")
                .to_string_lossy()
                .replace('\\', "/")
        );
        self.set_sources(
            vec![
                (uri.clone(), Some((1, 1))),
                (missing, Some((1, 2))),
                (uri.clone(), Some((1, 3))),
            ],
            1.0,
            0.0,
        )?;
        let begin = Instant::now();
        let mut skipped = false;
        while begin.elapsed() < Duration::from_secs(5) {
            pump_messages();
            skipped |= self.current() == Some((1, 3));
            if self.failed.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if !self.failed.load(Ordering::Acquire) || skipped {
            return Err("Failed native ayah was silently skipped".into());
        }
        if !self.has_session() || self.needs_fast_poll() || self.status(false).is_empty() {
            return Err("Failed playback lost its status or retained fast polling".into());
        }
        self.toggle()?;
        for _ in 0..20 {
            pump_messages();
            thread::sleep(Duration::from_millis(5));
        }
        if self.player.PlaybackSession()?.PlaybackState()? == MediaPlaybackState::Playing {
            return Err("Failed native playlist resumed without retry".into());
        }
        self.stop()?;
        self.set_sources(vec![(uri, Some((1, 1)))], 1.0, 0.0)?;
        let begin = Instant::now();
        let mut recovered = false;
        while begin.elapsed() < Duration::from_secs(5) {
            pump_messages();
            if self.player.PlaybackSession()?.PlaybackState()? == MediaPlaybackState::Playing {
                recovered = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.stop()?;
        if !recovered {
            return Err("Native playback did not recover after an explicit new range".into());
        }
        let dynamic = self.dynamic_smoke(folder, &path)?;
        Ok(
            serde_json::json!({"items":observed,"persistentRate":1.5,"changedRate":1.25,"pauseResume":paused,"lastItemFullyCompleted":fully_completed,"terminalSessionsUseIdlePolling":true,"repeatItems":repeated,"failedItemDoesNotSkip":true,"retryRecovers":recovered,"prefetchSeconds":60,"dynamicPreparation":dynamic}),
        )
    }
    #[cfg(feature = "qa")]
    fn dynamic_smoke(
        &mut self,
        folder: &std::path::Path,
        short: &std::path::Path,
    ) -> Result<serde_json::Value> {
        use crate::{audio_cache::qa_events, mp3::Mp3Info};
        use std::{
            collections::BTreeMap,
            fs, thread,
            time::{Duration, Instant},
        };
        let ready =
            |generation, index: usize, path: &std::path::Path, seconds| -> Result<CacheEvent> {
                Ok(CacheEvent::Ready {
                    generation,
                    index,
                    key: ClipKey::new(0, 112, index as u32 + 1)?,
                    path: path.to_path_buf(),
                    info: Mp3Info {
                        bytes: fs::metadata(path)?.len(),
                        frames: 1,
                        duration_seconds: seconds,
                    },
                })
            };
        // Production ayahs have distinct local URIs. Reusing one URI for every
        // fixture can let the Windows media pipeline share the same cached
        // stream/timeline across playlist items, unlike real prepared clips.
        let short_bytes = fs::read(short)?;
        let short_paths: Vec<_> = (1..=3)
            .map(|index| folder.join(format!("native-prepared-ayah-{index}.wav")))
            .collect();
        for path in &short_paths {
            fs::write(path, &short_bytes)?;
        }
        self.stop()?;
        self.pending_keys = (1..=3).map(|ayah| Some((112, ayah))).collect();
        self.preparation_complete.store(false, Ordering::Release);
        self.wanted_playing = true;
        for rate in [1.25, 2.0, 1.5] {
            self.speed(rate)
                .map_err(|e| format!("Set requested speed before source attachment: {e}"))?;
        }
        self.volume(0.0)?;
        self.install_sources(Vec::new())?;
        let trace = PlaylistTrace::new(self.list.as_ref().ok_or("Missing QA playlist")?)?;
        if self.player.Source().is_ok() {
            return Err("Empty preparation playlist was attached to the decoder".into());
        }
        let generation = self.generation.load(Ordering::Acquire);
        self.job = Some(qa_events(
            generation,
            vec![
                (
                    10,
                    CacheEvent::Ready {
                        generation: generation.wrapping_sub(1),
                        index: 666,
                        key: ClipKey::new(0, 112, 1)?,
                        path: folder.join("stale-missing-generation.wav"),
                        info: Mp3Info {
                            bytes: 1,
                            frames: 1,
                            duration_seconds: 0.6,
                        },
                    },
                ),
                (50, ready(generation, 0, &short_paths[0], 0.6)?),
                (1400, ready(generation, 1, &short_paths[1], 0.6)?),
                (1450, ready(generation, 2, &short_paths[2], 0.6)?),
                (
                    1500,
                    CacheEvent::Complete {
                        generation,
                        folder: folder.to_path_buf(),
                    },
                ),
            ],
        )?);
        self.toggle()?; // Pause before the first prepared clip exists.
        let begin = Instant::now();
        let mut resumed = false;
        let mut observed = Vec::new();
        let mut stalled = false;
        let mut completed = false;
        let mut rates = true;
        let mut progress = BTreeMap::new();
        let mut samples = Vec::new();
        while begin.elapsed() < Duration::from_secs(7) {
            pump_messages();
            let session = self.player.PlaybackSession()?;
            let before = serde_json::json!({"index":self.list.as_ref().and_then(|list|list.CurrentItemIndex().ok()),"position":session.Position()?.Duration,"duration":session.NaturalDuration()?.Duration,"state":session.PlaybackState()?.0,"queuedEnd":self.queue_ended.load(Ordering::Acquire),"endedIndex":self.ended_index.load(Ordering::Acquire),"keys":self.keys.len()});
            self.poll()?;
            let session = self.player.PlaybackSession()?;
            samples.push(serde_json::json!({"milliseconds":begin.elapsed().as_secs_f64()*1000.0,"beforePoll":before,"index":self.list.as_ref().and_then(|list|list.CurrentItemIndex().ok()),"position":session.Position()?.Duration,"duration":session.NaturalDuration()?.Duration,"state":session.PlaybackState()?.0,"queuedEnd":self.queue_ended.load(Ordering::Acquire),"endedIndex":self.ended_index.load(Ordering::Acquire),"keys":self.keys.len(),"prepared":self.preparation_complete.load(Ordering::Acquire),"finished":self.finished.load(Ordering::Acquire)}));
            if !resumed && begin.elapsed() > Duration::from_millis(250) && self.attached {
                if session.PlaybackState()? == MediaPlaybackState::Playing
                    || session.Position()?.Duration > 500_000
                {
                    return Err("Prepared audio ignored pause before its first clip".into());
                }
                self.toggle()?;
                resumed = true;
            }
            if matches!(
                session.PlaybackState()?,
                MediaPlaybackState::Playing | MediaPlaybackState::Paused
            ) {
                rates &= (session.PlaybackRate()? - 1.5).abs() < 0.01;
                if let Some((_, ayah)) = self.current() {
                    if observed.last() != Some(&ayah) {
                        observed.push(ayah);
                    }
                    let position = session.Position()?.Duration;
                    let duration = session.NaturalDuration()?.Duration;
                    let entry = progress.entry(ayah).or_insert((0, 0));
                    let sample = network_progress_sample(
                        Some(((112, ayah), entry.0, entry.1)),
                        (112, ayah),
                        position,
                        duration,
                    );
                    *entry = (sample.1, sample.2);
                }
            }
            if observed == [1]
                && self.queue_ended.load(Ordering::Acquire)
                && !self.preparation_complete.load(Ordering::Acquire)
            {
                stalled = true;
                if self.finished.load(Ordering::Acquire) {
                    return Err("Prepared prefix was mistaken for a completed range".into());
                }
            }
            if self.finished.load(Ordering::Acquire)
                && self.preparation_complete.load(Ordering::Acquire)
            {
                completed = true;
                break;
            }
            if self.failed.load(Ordering::Acquire) {
                return Err("Dynamic prepared native playlist failed".into());
            }
            thread::sleep(Duration::from_millis(5));
        }
        fs::write(
            folder.join("native-dynamic-audio-debug.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"samples":samples,"progress":progress,"observed":observed,"decoderTransitions":trace.snapshot()?}),
            )?,
        )?;
        let transitions = trace.snapshot()?;
        drop(trace);
        if completed
            && (self.job.is_none()
                || !self.has_session()
                || self.needs_fast_poll()
                || self.status(false) != "Playback finished")
        {
            return Err("Completed prepared range retained fast polling or lost its status".into());
        }
        self.stop()?;
        if observed != [1, 2, 3] || !resumed || !stalled || !completed || !rates {
            return Err(format!("Dynamic preparation failed: items={observed:?}, paused={resumed}, waited={stalled}, completed={completed}, rate={rates}").into());
        }
        // Prefix and final-item MediaEnded have terminal positions. During a
        // gapless intermediate transition, Windows can reset Position before
        // publishing the new index; its decoder EndOfStream event is the
        // authoritative completion boundary instead of a UI polling sample.
        if !decoded_transition(&transitions, 1, 2) {
            return Err(format!(
                "Intermediate ayah did not finish decoded playback: {transitions:?}"
            )
            .into());
        }
        for ayah in [1, 3] {
            let (position, duration) = progress
                .get(&ayah)
                .ok_or("Missing terminal audio progress")?;
            if *duration <= 0 || (duration - position).max(0) > 1_000_000 {
                return Err(format!("Dynamic preparation cut item {ayah} early: position={position}, duration={duration}").into());
            }
        }

        let long = folder.join("native-preparation-long.wav");
        let mut silence = fs::read(short)?;
        silence.truncate(44);
        let size = 48_000u32 * 2 * 2;
        silence[4..8].copy_from_slice(&(36 + size).to_le_bytes());
        silence[40..44].copy_from_slice(&size.to_le_bytes());
        silence.resize(44 + size as usize, 0);
        fs::write(&long, silence)?;
        self.pending_keys = (1..=4).map(|ayah| Some((112, ayah))).collect();
        self.preparation_complete.store(false, Ordering::Release);
        self.wanted_playing = true;
        self.speed(1.0)?;
        self.volume(0.0)?;
        self.install_sources(Vec::new())?;
        let generation = self.generation.load(Ordering::Acquire);
        self.job = Some(qa_events(
            generation,
            vec![
                (10, ready(generation, 0, &long, 2.0)?),
                (20, ready(generation, 1, &short_paths[1], 0.6)?),
                (30, ready(generation, 2, &short_paths[2], 0.6)?),
                (
                    550,
                    CacheEvent::Failed {
                        generation,
                        message: "Injected upcoming-ayah preparation failure".into(),
                    },
                ),
                (
                    600,
                    CacheEvent::Complete {
                        generation: generation.wrapping_sub(1),
                        folder: folder.to_path_buf(),
                    },
                ),
            ],
        )?);
        let begin = Instant::now();
        let mut failed = false;
        let mut failure_while_playing = false;
        let mut played = Vec::new();
        let mut latest = (0i64, 0i64);
        let mut latest_key = None;
        while begin.elapsed() < Duration::from_secs(7) {
            pump_messages();
            self.poll()?;
            let session = self.player.PlaybackSession()?;
            if self.current() != latest_key {
                latest_key = self.current();
                latest = (0, 0);
            }
            if session.PlaybackState()? == MediaPlaybackState::Playing {
                if self.preparation_error.is_some() && !self.failed.load(Ordering::Acquire) {
                    let status = self.status(false);
                    if self.needs_retry()
                        || !status.contains("Playing")
                        || status.contains("interrupted")
                        || status.contains("retry")
                    {
                        return Err(
                            "A lookahead failure replaced healthy queued playback status".into(),
                        );
                    }
                    failure_while_playing = true;
                }
                if let Some((_, ayah)) = self.current() {
                    if played.last() != Some(&ayah) {
                        played.push(ayah);
                    }
                }
            }
            if matches!(
                session.PlaybackState()?,
                MediaPlaybackState::Playing | MediaPlaybackState::Paused
            ) {
                let position = session.Position()?.Duration;
                let duration = session.NaturalDuration()?.Duration;
                if position >= latest.0 && duration > 0 {
                    latest = (position, duration);
                }
            }
            if self.failed.load(Ordering::Acquire) {
                if !self.needs_retry() || !self.status(false).contains("retry") {
                    return Err("An exhausted prepared prefix did not expose retry".into());
                }
                failed = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let generation_filtered = !self.preparation_complete.load(Ordering::Acquire);
        self.stop()?;
        if played != [1, 2, 3]
            || !failed
            || !failure_while_playing
            || !generation_filtered
            || latest.1 <= 0
            || (latest.1 - latest.0).max(0) > 1_000_000
        {
            return Err(format!("Preparation failure cut or skipped current audio: items={played:?}, stopped={failed}, deferred={failure_while_playing}, current={latest:?}, stale={generation_filtered}").into());
        }
        // Prove the independent decoder-event check rejects a real early
        // application-requested jump, even if every fixture file is complete.
        self.set_sources(
            vec![
                (local_uri(&long)?, Some((112, 1))),
                (local_uri(&short_paths[1])?, Some((112, 2))),
            ],
            1.0,
            0.0,
        )?;
        let trace = PlaylistTrace::new(self.list.as_ref().ok_or("Missing skip QA playlist")?)?;
        let begin = Instant::now();
        let mut injected = false;
        let mut early_position = (0, 0);
        while begin.elapsed() < Duration::from_secs(5) {
            pump_messages();
            let session = self.player.PlaybackSession()?;
            if !injected
                && session.PlaybackState()? == MediaPlaybackState::Playing
                && session.Position()?.Duration > 1_000_000
            {
                early_position = (
                    session.Position()?.Duration,
                    session.NaturalDuration()?.Duration,
                );
                self.list
                    .as_ref()
                    .ok_or("Missing skip QA playlist")?
                    .MoveTo(1)?;
                injected = true;
            }
            if injected
                && trace
                    .snapshot()?
                    .iter()
                    .any(|event| event["oldIndex"] == 0 && event["newIndex"] == 1)
            {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let skip_events = trace.snapshot()?;
        fs::write(
            folder.join("native-injected-skip-debug.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"position":early_position,"transitions":skip_events}),
            )?,
        )?;
        drop(trace);
        self.stop()?;
        if !injected
            || early_position.1 - early_position.0 <= 1_000_000
            || decoded_transition(&skip_events, 0, 1)
            || !skip_events.iter().any(|event| {
                event["oldIndex"] == 0
                    && event["newIndex"] == 1
                    && event["reason"] == MediaPlaybackItemChangedReason::AppRequested.0
            })
        {
            return Err(
                "The native decoder-event check did not reject the deliberately cut-off fixture"
                    .into(),
            );
        }
        Ok(
            serde_json::json!({"pauseBeforeReady":true,"lateAppendItems":observed,
            "queueExhaustionWaits":stalled,"preparedPrefixDoesNotFinishRange":true,"allItemsFinish":completed,
            "persistentRate":1.5,"rateChangesBeforeAttachmentSafe":true,"generationFiltering":generation_filtered,"failureFinishesCurrentThenStops":true,"failurePreservesPreparedPrefix":true,"emptyPlaylistNotAttached":true,
            "failurePlayedItems":played,"failureTailSeconds":(latest.1-latest.0).max(0) as f64/10_000_000.0,"terminalTailToleranceSeconds":0.1,
            "lookaheadFailureKeepsHealthyPlaybackStatus":true,"retryOnlyAfterPreparedPrefixEnds":true,
            "intermediateDecoderEndOfStream":true,"injectedEarlyMoveRejected":true,"distinctFixtureUris":true,
            "measurementScope":"Native decoder events and playback-session terminal positions; no hardware audio loopback capture"}),
        )
    }
}
#[cfg(any(feature = "qa", test))]
fn network_progress_sample(
    previous: Option<((u32, u32), i64, i64)>,
    key: (u32, u32),
    position: i64,
    duration: i64,
) -> ((u32, u32), i64, i64) {
    // WinRT does not publish the playlist index, position and duration as one
    // atomic snapshot. At a boundary, the position can reset and the duration
    // can belong to the next item while the old index is still visible. This
    // no-seek fixture retains monotonic progress with its matching duration.
    // Start a new item only after seeing its position near zero: never carry
    // completed progress from the old item into the new item's measurement.
    let last = previous.filter(|(last_key, _, _)| *last_key == key);
    let progress = last.unwrap_or((key, 0, 0));
    if position < 0
        || duration <= 0
        || position < progress.1
        || (progress.2 == 0 && position > 3_000_000)
    {
        progress
    } else {
        (key, position, duration)
    }
}
#[cfg(feature = "qa")]
pub(crate) fn pump_messages() -> bool {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::*;
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            if message.message == WM_QUIT {
                // A nested decoder fixture must not steal the outer UI loop's
                // shutdown signal. Repost its original exit code and stop this
                // pump before it can remove the reposted message again.
                PostQuitMessage(message.wParam.0 as i32);
                return false;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        true
    }
}
#[cfg(all(test, feature = "qa"))]
#[test]
fn qa_message_pump_preserves_quit_for_the_outer_message_loop() {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::*;
        PostQuitMessage(23);
        assert!(!pump_messages());
        let mut message = MSG::default();
        assert!(PeekMessageW(&mut message, None, WM_QUIT, WM_QUIT, PM_REMOVE).as_bool());
        assert_eq!(message.message, WM_QUIT);
        assert_eq!(message.wParam.0, 23);
        assert!(pump_messages());
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.stop();
        let _ = self.player.RemoveMediaOpened(self.opened_token);
        if let Ok(session) = self.player.PlaybackSession() {
            let _ = session.RemovePlaybackStateChanged(self.state_token);
        }
        let _ = self.player.Close();
    }
}
fn local_uri(path: &std::path::Path) -> Result<String> {
    let absolute = path.canonicalize()?;
    let raw = absolute
        .to_str()
        .ok_or("Recitation path is not valid Unicode")?;
    let raw = raw
        .strip_prefix("\\\\?\\")
        .unwrap_or(raw)
        .replace('\\', "/");
    if raw.starts_with('/') {
        return Err("Network recitation storage is unsupported".into());
    }
    let mut uri = String::from("file:///");
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~:".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(uri)
}
fn clip_opening_surah(surah: u32, first: u32) -> Option<u32> {
    (first == 1 && (2..=114).contains(&surah) && surah != 9).then_some(surah)
}
fn validate_range(reciter: usize, surah: u32, first: u32, last: u32, count: u32) -> Result<()> {
    if reciter >= RECITERS.len()
        || !(1..=114).contains(&surah)
        || !(1..=286).contains(&count)
        || first == 0
        || first > last
        || last > count
    {
        return Err("Invalid listening range".into());
    }
    Ok(())
}
// A source can be attached while Windows is still opening it. Keep the desired
// rate separately and retry on MediaOpened or the next regular poll. Only the
// observed MF_E_INVALIDREQUEST state race is deferred; other errors propagate.
fn apply_session_rate(session: &MediaPlaybackSession, rate: f64) -> windows::core::Result<()> {
    // Opening can accept a requested rate before the first decoded sample.
    // A truly detached source cannot. State-change events retry if opening
    // raced a reset, before regular UI polling is involved.
    if session.PlaybackState()? == MediaPlaybackState::None {
        return Ok(());
    }
    if (session.PlaybackRate()? - rate).abs() <= 0.0001 {
        return Ok(());
    }
    match session.SetPlaybackRate(rate) {
        Err(error) if error.code().0 as u32 == 0xc00d36b2 => Ok(()),
        result => result,
    }
}
fn checked_rate(value: f64) -> Result<f64> {
    if !value.is_finite() || !(0.5..=2.0).contains(&value) {
        return Err("Playback speed must be between 0.5 and 2".into());
    }
    Ok(value)
}
fn checked_volume(value: f64) -> Result<f64> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err("Playback volume must be between 0 and 1".into());
    }
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_clip_is_distinct_from_fatiha_tawbah_and_mid_chapter_ranges() {
        for surah in 0..=115 {
            let expected = ((2..=114).contains(&surah) && surah != 9).then_some(surah);
            assert_eq!(clip_opening_surah(surah, 1), expected);
            for first in [0, 2, 286, u32::MAX] {
                assert_eq!(clip_opening_surah(surah, first), None);
            }
        }
    }
    #[test]
    fn network_boundary_sampling_keeps_progress_without_hiding_early_end() {
        let old_key = (112, 2);
        let next_key = (112, 3);
        let finished = (old_key, 21_390_000, 20_370_000);
        let reset = network_progress_sample(Some(finished), old_key, 30_000, 27_240_000);
        assert_eq!(reset, finished);

        // An updated index with stale old-item progress must not credit the
        // next ayah with having completed before its start has been sampled.
        let waiting = network_progress_sample(Some(reset), next_key, 21_390_000, 20_370_000);
        assert_eq!(waiting, (next_key, 0, 0));
        assert_eq!(
            network_progress_sample(Some(waiting), next_key, 30_000, 27_240_000),
            (next_key, 30_000, 27_240_000)
        );

        // A genuine early end still retains its large unobserved tail.
        let early = (old_key, 5_000_000, 20_000_000);
        let early_reset = network_progress_sample(Some(early), old_key, 30_000, 27_240_000);
        assert_eq!(early_reset, early);
        assert!(early_reset.2 - early_reset.1 > 3_000_000);
    }
    #[test]
    fn urls_are_fixed_and_bounded() {
        assert_eq!(
            url(0, 2, 255).unwrap(),
            "https://everyayah.com/data/Yasser_Ad-Dussary_128kbps/002255.mp3"
        );
        assert!(url(0, 115, 1).is_err());
        assert!(url(99, 1, 1).is_err());
    }
    #[test]
    fn rejects_bad_ranges_and_nonfinite_settings() {
        assert!(validate_range(0, 112, 1, 4, 4).is_ok());
        for (r, s, first, last, count) in [
            (99, 1, 1, 1, 7),
            (0, 0, 1, 1, 7),
            (0, 115, 1, 1, 7),
            (0, 1, 0, 1, 7),
            (0, 1, 4, 3, 7),
            (0, 1, 1, 8, 7),
            (0, 1, 1, 1, 0),
            (0, 1, 1, 1, u32::MAX),
        ] {
            assert!(validate_range(r, s, first, last, count).is_err());
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 3.0] {
            assert!(checked_rate(value).is_err());
            assert!(checked_volume(value).is_err());
        }
        assert_eq!(checked_rate(1.5).unwrap(), 1.5);
        assert_eq!(checked_volume(0.8).unwrap(), 0.8);
    }
}
