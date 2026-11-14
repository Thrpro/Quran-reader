//! Slint-side ownership of the existing, validated native recitation engine.
//! No new transport, decoder, timing interpretation or provider is introduced.
//! Construct and use on the UI thread; its event loop pumps the WinRT apartment.
use crate::{
    audio::{Audio, ListeningRange, RECITERS, Transport},
    audio_cache::{AudioCache, COUNTS, Event as ClipEvent, Job as ClipJob},
    data::Result,
    listening_layout,
    timed_audio::Recording,
    timed_store::{Event as RecordingEvent, Job as RecordingJob},
};
use std::{
    fs,
    marker::PhantomData,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use windows::Win32::System::WinRT::{
    RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize,
};

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

#[derive(Clone, Copy)]
struct Request {
    reciter: usize,
    key: (u32, u32),
    /// Last verse of this surah to play; None plays to the end of the surah.
    last: Option<u32>,
}

struct PendingSeek {
    target_ms: u64,
    paused: bool,
    began: Instant,
    sent_at: Option<Instant>,
}

struct Apartment;
impl Apartment {
    fn initialize() -> Result<Self> {
        // Slint or accessibility may already own a compatible COM apartment.
        // Retry the other apartment only for the precise changed-mode result;
        // each successful RoInitialize is balanced once on this same thread.
        unsafe {
            match RoInitialize(RO_INIT_SINGLETHREADED) {
                Ok(()) => {}
                Err(error) if error.code().0 as u32 == 0x80010106 => {
                    RoInitialize(RO_INIT_MULTITHREADED)?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
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

#[derive(Debug, PartialEq, Eq)]
pub enum DownloadNotice {
    Progress(u32),
    Done,
    Failed(String),
    Cancelled,
}

enum DownloadJob {
    Clips(ClipJob),
    Recording(RecordingJob),
}

/// Lazy: no player, cache, workers or profile directory exists before Play
/// or a download.
pub struct AudioBridge {
    audio: Option<Audio>,
    cache: Option<Arc<AudioCache>>,
    download: Option<(DownloadJob, usize, u32)>,
    download_generation: u64,
    profile_root: PathBuf,
    request: Option<Request>,
    last_transport: Option<Transport>,
    last_key: Option<(u32, u32)>,
    last_error: Option<String>,
    pending_seek: Option<PendingSeek>,
    rate: f64,
    volume: f64,
    // Audio owns its cache. Keep the apartment guard after it so media/cache
    // resources are dropped before the final RoUninitialize on this thread.
    _apartment: Option<Apartment>,
    _owning_thread: PhantomData<Rc<()>>,
}

impl AudioBridge {
    pub fn new(profile_root: PathBuf) -> Self {
        Self {
            audio: None,
            cache: None,
            download: None,
            download_generation: 0,
            profile_root,
            request: None,
            last_transport: None,
            last_key: None,
            last_error: None,
            pending_seek: None,
            rate: 1.,
            volume: 1.,
            _apartment: None,
            _owning_thread: PhantomData,
        }
    }

    fn initialize(&mut self) -> Result<()> {
        if self.audio.is_some() {
            return Ok(());
        }
        let apartment = Apartment::initialize()?;
        let cache = AudioCache::new(self.profile_root.clone())?;
        self.cache = Some(cache.clone());
        let audio = Audio::with_cache(cache)?;
        self._apartment = Some(apartment);
        self.audio = Some(audio);
        Ok(())
    }

    /// A deliberate Play request is the only entry into the native network
    /// preparation paths. Continuous recording defaults to the same supported
    /// publisher bindings as the native app; another voice stays that voice.
    pub fn play(
        &mut self,
        reciter: usize,
        key: (u32, u32),
        last: Option<u32>,
        rate: f64,
        volume: f64,
    ) -> Result<()> {
        let range = checked_range(reciter, key, last)?;
        checked_controls(rate, volume)?;
        self.initialize()?;
        let audio = self.audio.as_mut().ok_or("Audio player is unavailable")?;
        if Recording::for_reciter(reciter).is_some() {
            audio.play_timed_scoped(range, rate, volume)?;
        } else {
            audio.play_range(range, rate, volume)?;
        }
        self.request = Some(Request { reciter, key, last });
        self.rate = rate;
        self.volume = volume;
        self.last_transport = None;
        self.last_key = Some(key);
        self.last_error = None;
        self.pending_seek = None;
        Ok(())
    }

    /// The reciter and surah being recited, or last recited.
    pub fn current_surah(&self) -> Option<(usize, u32)> {
        self.request.map(|request| (request.reciter, request.key.0))
    }

    /// The reciter and surah of a recitation that has played to its end.
    pub fn finished(&self) -> Option<(usize, u32)> {
        if self.last_error.is_some() || self.pending_seek.is_some() {
            return None;
        }
        let audio = self.audio.as_ref()?;
        if !(audio.has_session() && audio.completed()) {
            return None;
        }
        self.request.map(|request| (request.reciter, request.key.0))
    }

    pub fn toggle(&mut self) -> Result<()> {
        if let Some(pending) = &mut self.pending_seek {
            pending.paused = !pending.paused;
            return Ok(());
        }
        if self.last_error.is_some()
            || self
                .audio
                .as_ref()
                .is_some_and(|audio| audio.completed() || native_failed(audio))
        {
            let request = self.request.ok_or("Choose a verse to play first")?;
            return self.play(
                request.reciter,
                request.key,
                request.last,
                self.rate,
                self.volume,
            );
        }
        self.audio
            .as_mut()
            .filter(|audio| audio.has_session())
            .ok_or("Choose a verse to play first")?
            .toggle()
    }

    /// Save one surah for offline listening with the native, validated stores:
    /// the continuous recording where one exists, otherwise every ayah clip.
    pub fn start_download(&mut self, reciter: usize, surah: u32) -> Result<()> {
        if reciter >= RECITERS.len() || !(1..=114).contains(&surah) {
            return Err("Invalid surah or reciter".into());
        }
        self.initialize()?;
        // Dropping a previous job cancels it.
        self.download = None;
        self.download_generation = self.download_generation.wrapping_add(1);
        let generation = self.download_generation;
        let cache = self
            .cache
            .clone()
            .ok_or("Recitation storage is unavailable")?;
        let job = if Recording::for_reciter(reciter).is_some() {
            DownloadJob::Recording(cache.timed_store()?.prepare(reciter, surah, generation)?)
        } else {
            DownloadJob::Clips(cache.download_surah(reciter, surah, generation)?)
        };
        self.download = Some((job, reciter, surah));
        Ok(())
    }

    pub fn cancel_download(&mut self) {
        self.download = None;
    }

    pub fn downloading(&self) -> Option<(usize, u32)> {
        self.download
            .as_ref()
            .map(|(_, reciter, surah)| (*reciter, *surah))
    }

    /// The latest state of the running download, if anything changed.
    pub fn poll_download(&mut self) -> Option<DownloadNotice> {
        let generation = self.download_generation;
        let mut latest = None;
        let mut finished = false;
        match &self.download {
            None => return None,
            Some((DownloadJob::Clips(job), ..)) => {
                while let Some(event) = job.try_recv() {
                    match event {
                        ClipEvent::Progress {
                            generation: g,
                            percent,
                            ..
                        } if g == generation => {
                            latest = Some(DownloadNotice::Progress(percent.min(100)));
                        }
                        ClipEvent::Complete { generation: g, .. } if g == generation => {
                            latest = Some(DownloadNotice::Done);
                            finished = true;
                        }
                        ClipEvent::Failed {
                            generation: g,
                            message,
                        } if g == generation => {
                            latest = Some(DownloadNotice::Failed(message));
                            finished = true;
                        }
                        ClipEvent::Cancelled { generation: g } if g == generation => {
                            latest = Some(DownloadNotice::Cancelled);
                            finished = true;
                        }
                        _ => {}
                    }
                    if finished {
                        break;
                    }
                }
            }
            Some((DownloadJob::Recording(job), ..)) => {
                while let Some(event) = job.try_recv() {
                    match event {
                        RecordingEvent::Progress(bytes, expected) => {
                            let percent = expected.filter(|total| *total > 0).map_or(0, |total| {
                                (bytes.saturating_mul(100) / total).min(99) as u32
                            });
                            latest = Some(DownloadNotice::Progress(percent));
                        }
                        RecordingEvent::Ready(ready) if ready.generation == generation => {
                            latest = Some(DownloadNotice::Done);
                            finished = true;
                        }
                        RecordingEvent::Failed(g, message) if g == generation => {
                            latest = Some(DownloadNotice::Failed(message));
                            finished = true;
                        }
                        _ => {}
                    }
                    if finished {
                        break;
                    }
                }
            }
        }
        if finished {
            self.download = None;
        }
        latest
    }

    /// Surahs saved for offline listening, read from the native stores' own
    /// folder names. Partial or unknown folders are ignored.
    pub fn downloaded(&self) -> Vec<DownloadedSurah> {
        let store = self.profile_root.join("AudioStore");
        let mut found = Vec::new();
        for (folder, continuous) in [
            (store.join("Downloads"), false),
            (store.join("Continuous"), true),
        ] {
            let Ok(entries) = fs::read_dir(&folder) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let parsed = if continuous {
                    parse_recording_folder(&name)
                } else {
                    parse_clip_folder(&name)
                };
                let Some((reciter, surah)) = parsed else {
                    continue;
                };
                let path = entry.path();
                let marker = if continuous {
                    "receipt.json"
                } else {
                    "surah.json"
                };
                if plain_directory(&path) && path.join(marker).is_file() {
                    found.push(DownloadedSurah {
                        reciter,
                        surah,
                        bytes: folder_bytes(&path),
                        continuous,
                        complete: download_complete(&path, continuous),
                    });
                }
            }
        }
        found.sort_by_key(|d| (d.surah, d.reciter, d.continuous));
        found
    }

    /// Remove one saved surah. Only plain files directly inside its exactly
    /// named folder are removed, and never while it downloads or plays.
    pub fn delete_download(&mut self, item: DownloadedSurah) -> Result<()> {
        if self.downloading() == Some((item.reciter, item.surah)) {
            return Err("This surah is still downloading.".into());
        }
        if self
            .request
            .is_some_and(|r| r.reciter == item.reciter && r.key.0 == item.surah)
            && self.audio.as_ref().is_some_and(Audio::has_session)
        {
            return Err("Stop this recitation before deleting its download.".into());
        }
        let store = self.profile_root.join("AudioStore");
        let folder = if item.continuous {
            let recording = Recording::for_reciter(item.reciter).ok_or("Unknown recording")?;
            store
                .join("Continuous")
                .join(format!("r{}-s{:03}", recording.read, item.surah))
        } else {
            store.join("Downloads").join(format!(
                "reciter-{:02}-surah-{:03}",
                item.reciter, item.surah
            ))
        };
        if !plain_directory(&folder) {
            return Err("This download was not found.".into());
        }
        let mut files = Vec::new();
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                return Err(
                    "This download folder contains unexpected items and was left unchanged.".into(),
                );
            }
            files.push(entry.path());
        }
        for file in files {
            fs::remove_file(file)?;
        }
        fs::remove_dir(&folder)?;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        let result = self.audio.as_mut().map(Audio::stop).transpose();
        self.request = None;
        self.last_transport = None;
        self.last_key = None;
        self.last_error = None;
        self.pending_seek = None;
        result.map(|_| ())
    }

    pub fn set_speed(&mut self, rate: f64) -> Result<()> {
        checked_controls(rate, self.volume)?;
        if let Some(audio) = &mut self.audio {
            audio.request_speed(rate)?;
        }
        self.rate = rate;
        Ok(())
    }

    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        checked_controls(self.rate, volume)?;
        if let Some(audio) = &self.audio
            && self.pending_seek.is_none()
        {
            audio.volume(volume)?;
        }
        self.volume = volume;
        Ok(())
    }

    pub fn seek_delta(&mut self, milliseconds: i64) -> Result<bool> {
        let Some(transport) = self.observed_transport() else {
            return Ok(false);
        };
        let from = self.pending_seek.as_ref().map_or_else(
            || transport.offset_ms.saturating_add(transport.position_ms),
            |pending| pending.target_ms,
        );
        let target = from
            .saturating_add_signed(milliseconds)
            .min(transport.duration_ms.saturating_sub(50));
        self.seek_absolute(target)
    }

    /// A chapter slider can leave a from-here range: reopen the same validated
    /// recording from its beginning, then perform the desired absolute seek.
    /// The new source remains paused and muted until its real position agrees.
    pub fn seek_slider(&mut self, value: u32) -> Result<bool> {
        let Some(transport) = self.observed_transport() else {
            return Ok(false);
        };
        let target =
            ((u128::from(transport.duration_ms) * u128::from(value.min(10_000))) / 10_000) as u64;
        self.seek_absolute(target.min(transport.duration_ms.saturating_sub(50)))
    }

    fn observed_transport(&self) -> Option<Transport> {
        self.audio
            .as_ref()
            .and_then(Audio::transport)
            .or(self.last_transport)
    }

    fn seek_absolute(&mut self, target_ms: u64) -> Result<bool> {
        if let Some(pending) = &mut self.pending_seek {
            pending.target_ms = target_ms;
            pending.began = Instant::now();
            pending.sent_at = None;
            return Ok(true);
        }
        let Some(transport) = self.observed_transport().filter(|value| value.can_seek) else {
            return Ok(false);
        };
        let end = transport.offset_ms.saturating_add(transport.range_ms);
        if transport.continuous && (target_ms < transport.offset_ms || target_ms >= end) {
            return self.reopen_for_seek(target_ms);
        }
        let relative_target = target_ms.saturating_sub(transport.offset_ms);
        let delta = i64::try_from(i128::from(relative_target) - i128::from(transport.position_ms))?;
        self.audio
            .as_mut()
            .map_or(Ok(false), |audio| audio.seek(Some(delta), 0))
    }

    fn reopen_for_seek(&mut self, target_ms: u64) -> Result<bool> {
        let request = self.request.ok_or("Choose a verse to play first")?;
        let range = checked_range(request.reciter, (request.key.0, 1), request.last)?;
        let audio = self.audio.as_mut().ok_or("Audio player is unavailable")?;
        let paused = audio.is_paused();
        let result = (|| -> Result<()> {
            audio.play_timed_scoped(range, self.rate, 0.)?;
            // Set the pause intent before any asynchronous source is attached.
            audio.toggle()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = audio.stop();
            self.last_error = Some(error.to_string());
            return Err(error);
        }
        self.request = Some(Request {
            reciter: request.reciter,
            key: (request.key.0, 1),
            last: request.last,
        });
        self.last_error = None;
        self.pending_seek = Some(PendingSeek {
            target_ms,
            paused,
            began: Instant::now(),
            sent_at: None,
        });
        Ok(true)
    }

    fn advance_pending_seek(&mut self) -> Result<()> {
        let Some(pending) = &mut self.pending_seek else {
            return Ok(());
        };
        if pending.began.elapsed() > Duration::from_secs(30) {
            return Err("The recording did not become ready for seeking".into());
        }
        let audio = self.audio.as_mut().ok_or("Audio player is unavailable")?;
        if native_failed(audio) {
            return Err("The recording could not be reopened for seeking".into());
        }
        let Some(transport) = audio
            .transport()
            .filter(|value| value.continuous && value.can_seek)
        else {
            return Ok(());
        };
        let target = pending
            .target_ms
            .min(transport.duration_ms.saturating_sub(50));
        if pending.sent_at.is_none() {
            let current = transport.offset_ms.saturating_add(transport.position_ms);
            let delta = i64::try_from(i128::from(target) - i128::from(current))?;
            if audio.seek(Some(delta), 0)? {
                pending.target_ms = target;
                pending.sent_at = Some(Instant::now());
            }
            return Ok(());
        }
        if pending
            .sent_at
            .is_some_and(|time| time.elapsed() < Duration::from_millis(50))
            || transport
                .offset_ms
                .saturating_add(transport.position_ms)
                .abs_diff(target)
                > 1_000
        {
            return Ok(());
        }
        let paused = pending.paused;
        audio.volume(self.volume)?;
        if !paused {
            audio.toggle()?;
        }
        self.pending_seek = None;
        Ok(())
    }

    /// Stop UI polling after End/completion/failure; no audio timer is needed
    /// before a user starts listening. Paused preparation still needs polling.
    pub fn needs_poll(&self) -> bool {
        self.last_error.is_none()
            && (self.pending_seek.is_some()
                || self.audio.as_ref().is_some_and(Audio::needs_fast_poll))
    }

    pub fn poll(&mut self, arabic: bool) -> Result<AudioSnapshot> {
        let result = self.audio.as_mut().map(Audio::poll).transpose();
        let result = result.and_then(|_| self.advance_pending_seek());
        if let Err(error) = result {
            if let Some(audio) = &mut self.audio {
                let _ = audio.stop();
            }
            self.last_error = Some(error.to_string());
            self.pending_seek = None;
        }
        if let Some(audio) = &mut self.audio {
            // A merely failed lookahead remains healthy until the native
            // decoder reaches its actual stop boundary. Do not invent errors.
            let current = audio.current();
            if let Some(key) = current
                && self.pending_seek.is_none()
                && !audio.completed()
            {
                if self.last_key != Some(key)
                    && self
                        .last_transport
                        .is_some_and(|transport| !transport.continuous)
                {
                    self.last_transport = None;
                }
                self.last_key = Some(key);
            }
            if let Some(transport) = audio.transport()
                && self.pending_seek.is_none()
            {
                self.last_transport = Some(transport);
            }
        }
        Ok(self.snapshot(arabic))
    }

    pub fn snapshot(&self, arabic: bool) -> AudioSnapshot {
        let Some(audio) = &self.audio else {
            return AudioSnapshot::default();
        };
        let active = self.request.is_some() && (audio.has_session() || self.last_error.is_some());
        if !active {
            return AudioSnapshot::default();
        }
        let completed = audio.completed();
        let failed = self.last_error.is_some() || native_failed(audio);
        let mut transport = if self.pending_seek.is_some() {
            self.last_transport
        } else {
            audio.transport().or(self.last_transport)
        };
        if completed && let Some(transport) = &mut transport {
            // End-of-stream is decoder-confirmed. Some sessions reset their
            // position immediately afterward; retain the selected range end
            // instead of flashing its chapter/clip clock back to zero.
            transport.position_ms = transport.range_ms;
        }
        let (position_ms, duration_ms, range_start_ms, range_end_ms, slider, clock) =
            clock_fields(transport, arabic);
        let paused = self
            .pending_seek
            .as_ref()
            .map_or_else(|| audio.is_paused(), |pending| pending.paused);
        let loading = !failed && !completed && (self.pending_seek.is_some() || transport.is_none());
        let status = if failed {
            if arabic {
                "تعذّر تشغيل التسجيل. أعد المحاولة."
            } else {
                "Could not play this recording. Try again."
            }
        } else if completed {
            if arabic {
                "اكتملت التلاوة"
            } else {
                "Recitation finished"
            }
        } else if self.pending_seek.is_some() {
            if arabic {
                "جارٍ الانتقال في التلاوة…"
            } else {
                "Seeking in recitation…"
            }
        } else if loading {
            if arabic {
                "جارٍ تحميل التلاوة…"
            } else {
                "Loading recitation…"
            }
        } else if paused {
            if arabic {
                "متوقف مؤقتًا"
            } else {
                "Paused"
            }
        } else if arabic {
            "يتم التشغيل"
        } else {
            "Playing"
        };
        AudioSnapshot {
            active,
            paused,
            loading,
            completed,
            error: failed,
            continuous: transport.is_some_and(|value| value.continuous),
            can_seek: !failed
                && !completed
                && self.pending_seek.is_none()
                && transport.is_some_and(|value| value.can_seek),
            current: if self.pending_seek.is_some() || completed {
                self.last_key
            } else {
                audio.current().or(self.last_key)
            },
            opening_surah: audio.opening_surah(),
            position_ms,
            duration_ms,
            range_start_ms,
            range_end_ms,
            slider,
            clock,
            status: status.into(),
        }
    }
}

impl Drop for AudioBridge {
    fn drop(&mut self) {
        if let Some(audio) = &mut self.audio {
            let _ = audio.stop();
        }
    }
}

/// Whether a saved surah's record and the files it names are all there.
fn download_complete(folder: &Path, continuous: bool) -> bool {
    let present = |name: &str| {
        !name.is_empty()
            && !name.contains(['/', '\\'])
            && fs::metadata(folder.join(name)).is_ok_and(|m| m.is_file() && m.len() > 0)
    };
    let record = |name: &str| -> Option<serde_json::Value> {
        let bytes = fs::read(folder.join(name)).ok()?;
        (bytes.len() <= 256 * 1024).then_some(())?;
        serde_json::from_slice(&bytes).ok()
    };
    if continuous {
        record("receipt.json").is_some() && present("audio.mp3") && present("timings.json")
    } else {
        record("surah.json")
            .and_then(|manifest| manifest.get("clips")?.as_array().cloned())
            .is_some_and(|clips| {
                !clips.is_empty()
                    && clips.iter().all(|clip| {
                        clip.get("file")
                            .and_then(|f| f.as_str())
                            .is_some_and(present)
                    })
            })
    }
}

fn checked_range(reciter: usize, key: (u32, u32), last: Option<u32>) -> Result<ListeningRange> {
    let count = key
        .0
        .checked_sub(1)
        .and_then(|index| COUNTS.get(index as usize));
    let Some(&count) = count else {
        return Err("Invalid recitation chapter".into());
    };
    if reciter >= RECITERS.len() || key.1 == 0 || key.1 > count {
        return Err("Invalid recitation verse or reciter".into());
    }
    Ok(ListeningRange {
        reciter,
        surah: key.0,
        first: key.1,
        last: match last {
            Some(last) if (key.1..=count).contains(&last) => last,
            Some(_) => return Err("Invalid recitation range".into()),
            None => count,
        },
        count,
    })
}

fn checked_controls(rate: f64, volume: f64) -> Result<()> {
    if !rate.is_finite() || !(0.5..=2.).contains(&rate) {
        return Err("Playback speed must be between 0.5 and 2".into());
    }
    if !volume.is_finite() || !(0. ..=1.).contains(&volume) {
        return Err("Playback volume must be between 0 and 1".into());
    }
    Ok(())
}

fn native_failed(audio: &Audio) -> bool {
    // Native policy retains terminal playlists for the UI. Its fast-poll
    // predicate excludes an actual decoder failure, but deliberately includes
    // paused/buffering sessions and healthy prepared prefixes after lookahead
    // failure. This avoids treating an ordinary preparation delay as an error.
    audio.has_session() && !audio.completed() && !audio.needs_fast_poll()
}

fn clock_fields(transport: Option<Transport>, arabic: bool) -> (u64, u64, u64, u64, u32, String) {
    let Some(transport) = transport.filter(|value| value.duration_ms > 0) else {
        return (0, 0, 0, 0, 0, String::new());
    };
    let position = transport
        .offset_ms
        .saturating_add(transport.position_ms)
        .min(transport.duration_ms);
    let end = transport
        .offset_ms
        .saturating_add(transport.range_ms)
        .min(transport.duration_ms);
    let slider = ((u128::from(position) * 10_000) / u128::from(transport.duration_ms)) as u32;
    (
        position,
        transport.duration_ms,
        transport.offset_ms,
        end,
        slider,
        if arabic {
            // Read right to left: the time played first, then the length,
            // kept as one left-to-right run so the digits stay in place.
            format!(
                "{}\u{2066}{} / {}\u{2069}",
                if transport.continuous {
                    ""
                } else {
                    "الآية "
                },
                listening_layout::clock(transport.duration_ms),
                listening_layout::clock(position)
            )
        } else {
            listening_layout::progress_label(
                transport.continuous,
                arabic,
                &listening_layout::clock(position),
                &listening_layout::clock(transport.duration_ms),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_range_uses_native_chapter_boundaries() {
        let range = checked_range(0, (2, 145), None).unwrap();
        assert_eq!((range.first, range.last, range.count), (145, 286, 286));
        let range = checked_range(5, (112, 2), Some(2)).unwrap();
        assert_eq!((range.first, range.last, range.count), (2, 2, 4));
        for key in [(0, 1), (115, 1), (1, 0), (1, 8), (112, 5)] {
            assert!(checked_range(0, key, None).is_err());
        }
        assert!(checked_range(RECITERS.len(), (1, 1), None).is_err());
        assert!(checked_range(0, (2, 10), Some(9)).is_err());
        assert!(checked_range(0, (2, 10), Some(287)).is_err());
        assert_eq!(checked_range(0, (2, 10), Some(20)).unwrap().last, 20);
    }

    #[test]
    fn chapter_clock_includes_offset_without_fabricating_duration() {
        let transport = Transport {
            position_ms: 15_000,
            range_ms: 900_000,
            offset_ms: 600_000,
            duration_ms: 1_500_000,
            continuous: true,
            can_seek: true,
        };
        let (position, duration, start, end, slider, text) = clock_fields(Some(transport), false);
        assert_eq!(
            (position, duration, start, end),
            (615_000, 1_500_000, 600_000, 1_500_000)
        );
        assert_eq!(slider, 4100);
        assert_eq!(text, "10:15 / 25:00");
        assert_eq!(clock_fields(None, true), (0, 0, 0, 0, 0, String::new()));
    }

    #[test]
    fn clip_clock_is_explicit_and_controls_reject_nonfinite_values() {
        let transport = Transport {
            position_ms: 2000,
            range_ms: 8000,
            offset_ms: 0,
            duration_ms: 8000,
            continuous: false,
            can_seek: true,
        };
        assert_eq!(clock_fields(Some(transport), false).5, "Ayah 0:02 / 0:08");
        assert!(checked_controls(f64::NAN, 1.).is_err());
        assert!(checked_controls(1., f64::INFINITY).is_err());
        assert!(checked_controls(1., -0.1).is_err());
        assert!(checked_controls(0.5, 0.).is_ok());
        assert!(checked_controls(2., 1.).is_ok());
    }
}

fn parse_clip_folder(name: &str) -> Option<(usize, u32)> {
    let rest = name.strip_prefix("reciter-")?;
    let (reciter, surah) = rest.split_once("-surah-")?;
    if reciter.len() != 2
        || surah.len() != 3
        || !reciter
            .bytes()
            .chain(surah.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let (reciter, surah) = (reciter.parse::<usize>().ok()?, surah.parse::<u32>().ok()?);
    (reciter < RECITERS.len() && (1..=114).contains(&surah)).then_some((reciter, surah))
}

fn parse_recording_folder(name: &str) -> Option<(usize, u32)> {
    let rest = name.strip_prefix('r')?;
    let (read, surah) = rest.split_once("-s")?;
    if read.is_empty()
        || surah.len() != 3
        || !read
            .bytes()
            .chain(surah.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let (read, surah) = (read.parse::<u32>().ok()?, surah.parse::<u32>().ok()?);
    let reciter = (0..RECITERS.len())
        .find(|r| Recording::for_reciter(*r).is_some_and(|rec| rec.read == read))?;
    (1..=114).contains(&surah).then_some((reciter, surah))
}

/// A real directory, not a link or junction.
fn plain_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && m.file_attributes() & 0x400 == 0)
}

fn folder_bytes(path: &Path) -> u64 {
    fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| fs::symlink_metadata(e.path()).ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod download_tests {
    use super::*;
    #[test]
    fn download_folder_names_parse_only_exact_native_forms() {
        assert_eq!(parse_clip_folder("reciter-01-surah-036"), Some((1, 36)));
        assert_eq!(parse_clip_folder("reciter-1-surah-036"), None);
        assert_eq!(parse_clip_folder("reciter-01-surah-115"), None);
        assert_eq!(parse_clip_folder(".reciter-01-surah-036-7.partial"), None);
        assert_eq!(parse_clip_folder("reciter-99-surah-001"), None);
        let read = Recording::for_reciter(0).unwrap().read;
        assert_eq!(
            parse_recording_folder(&format!("r{read}-s018")),
            Some((0, 18))
        );
        assert_eq!(parse_recording_folder(&format!("r{read}-s18")), None);
        assert_eq!(parse_recording_folder("r1-s018"), None);
    }
}
