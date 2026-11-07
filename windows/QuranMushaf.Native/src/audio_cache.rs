//! Fully validated local recitation clips, with fixed-provider HTTPS transport.
//! Two persistent workers keep network, hashing and filesystem work off the UI.
use crate::{
    audio,
    data::Result,
    mp3::{self, Mp3Info},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use windows::{
    Win32::{Networking::WinHttp::*, Security::Cryptography::*},
    core::{PCWSTR, w},
};

const CACHE_QUOTA: u64 = 32 * 1024 * 1024;
const LIBRARY_QUOTA: u64 = 1024 * 1024 * 1024;
const AHEAD_SECONDS: f64 = 180.0;
const AHEAD_CLIPS: usize = 16;
const RETRIES: usize = 3;
const LOOKAHEAD_RECOVERY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
const REPARSE_POINT: u32 = 0x400;
static NONCE: AtomicU64 = AtomicU64::new(1);
pub(crate) const COUNTS: [u32; 114] = [
    7, 286, 200, 176, 120, 165, 206, 75, 129, 109, 123, 111, 43, 52, 99, 128, 111, 110, 98, 135,
    112, 78, 118, 64, 77, 227, 93, 88, 69, 60, 34, 30, 73, 54, 45, 83, 182, 88, 75, 85, 54, 53, 89,
    59, 37, 35, 38, 29, 18, 45, 60, 49, 62, 55, 78, 96, 29, 22, 24, 13, 14, 11, 11, 18, 12, 12, 30,
    52, 52, 44, 28, 28, 20, 56, 40, 31, 50, 40, 46, 42, 29, 19, 36, 25, 22, 17, 19, 26, 30, 20, 15,
    21, 11, 8, 8, 19, 5, 8, 8, 11, 11, 8, 3, 9, 5, 4, 7, 3, 6, 3, 5, 4, 5, 6,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ClipKey {
    pub reciter: usize,
    pub surah: u32,
    pub ayah: u32,
}
impl ClipKey {
    pub fn new(reciter: usize, surah: u32, ayah: u32) -> Result<Self> {
        if reciter >= audio::RECITERS.len()
            || !(1..=114).contains(&surah)
            || ayah == 0
            || ayah > COUNTS[(surah - 1) as usize]
        {
            return Err("Invalid recitation reference".into());
        }
        Ok(Self {
            reciter,
            surah,
            ayah,
        })
    }
    fn prefix(self) -> String {
        format!("r{:02}-s{:03}-a{:03}", self.reciter, self.surah, self.ayah)
    }
}

#[derive(Debug)]
pub enum Event {
    Ready {
        generation: u64,
        index: usize,
        key: ClipKey,
        path: PathBuf,
        info: Mp3Info,
    },
    Progress {
        generation: u64,
        completed: usize,
        total: usize,
        percent: u32,
        bytes: u64,
    },
    Complete {
        generation: u64,
        folder: PathBuf,
    },
    Failed {
        generation: u64,
        message: String,
    },
    Cancelled {
        generation: u64,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct StorageStats {
    pub cache_bytes: u64,
    pub library_bytes: u64,
    pub cache_limit: u64,
    pub library_limit: u64,
}

struct JobState {
    cancelled: AtomicBool,
    consumed: AtomicUsize,
    generation: u64,
    pins: Mutex<BTreeMap<usize, ClipKey>>,
    cache: Weak<AudioCache>,
}
impl JobState {
    fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err("Recitation preparation cancelled".into())
        } else {
            Ok(())
        }
    }
}
impl Drop for JobState {
    fn drop(&mut self) {
        if let Some(cache) = self.cache.upgrade() {
            let mut pins = cache.pins.lock().unwrap_or_else(|e| e.into_inner());
            for key in self
                .pins
                .get_mut()
                .unwrap_or_else(|e| e.into_inner())
                .values()
            {
                if let Some(count) = pins.get_mut(key) {
                    *count -= 1;
                    if *count == 0 {
                        pins.remove(key);
                    }
                }
            }
        }
    }
}
pub struct Job {
    state: Arc<JobState>,
    events: mpsc::Receiver<Event>,
}
impl Job {
    pub fn try_recv(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
    pub fn set_consumed(&self, index: usize) {
        self.state.consumed.fetch_max(index, Ordering::Release);
        // Native playback keeps only the current and previous item open. Do
        // not pin the entire chapter: that prevents eviction halfway through
        // a long surah. Keep the previous item and every prepared future item.
        if let Some(cache) = self.state.cache.upgrade() {
            let mut held = self.state.pins.lock().unwrap_or_else(|e| e.into_inner());
            let mut pins = cache.pins.lock().unwrap_or_else(|e| e.into_inner());
            held.retain(|position, key| {
                if *position >= index.saturating_sub(1) {
                    return true;
                }
                if let Some(count) = pins.get_mut(key) {
                    *count -= 1;
                    if *count == 0 {
                        pins.remove(key);
                    }
                }
                false
            });
        }
    }
    pub fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::Release);
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}
/// QA-only delayed preparation messages. The silent WAVs exercise the native
/// playlist state machine; production jobs can only emit inspected MP3 files.
#[cfg(feature = "qa")]
pub(crate) fn qa_events(generation: u64, schedule: Vec<(u64, Event)>) -> Result<Job> {
    let state = Arc::new(JobState {
        cancelled: AtomicBool::new(false),
        consumed: AtomicUsize::new(0),
        generation,
        pins: Mutex::new(BTreeMap::new()),
        cache: Weak::new(),
    });
    let (sender, events) = mpsc::channel();
    let worker_state = state.clone();
    thread::Builder::new()
        .name("recitation-qa-events".into())
        .spawn(move || {
            let start = Instant::now();
            for (milliseconds, event) in schedule {
                while start.elapsed() < Duration::from_millis(milliseconds) {
                    if worker_state.check().is_err() {
                        return;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                if worker_state.check().is_err() || sender.send(event).is_err() {
                    return;
                }
            }
        })?;
    Ok(Job { state, events })
}

enum Mode {
    Playback,
    Download { reciter: usize, surah: u32 },
}
struct Task {
    keys: Vec<ClipKey>,
    state: Arc<JobState>,
    events: mpsc::Sender<Event>,
    mode: Mode,
}
#[derive(Clone, Serialize, Deserialize)]
struct Receipt {
    schema: String,
    key: ClipKey,
    file: String,
    bytes: u64,
    frames: u32,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
struct SurahManifest {
    schema: String,
    reciter: usize,
    surah: u32,
    clips: Vec<Receipt>,
}
struct CachedReceipt {
    value: Receipt,
    metadata_hash: String,
}
pub struct AudioCache {
    timed_store: Mutex<Option<Arc<crate::timed_store::Store>>>,
    root: PathBuf,
    clips: PathBuf,
    library: PathBuf,
    index: Mutex<BTreeMap<ClipKey, CachedReceipt>>,
    pins: Mutex<BTreeMap<ClipKey, usize>>,
    playback: mpsc::SyncSender<Task>,
    downloads: mpsc::SyncSender<Task>,
    prior_playback: Mutex<Weak<JobState>>,
    prior_download: Mutex<Weak<JobState>>,
    transport: Arc<dyn Transport>,
    index_loaded: AtomicBool,
    library_limit: AtomicU64,
}
impl AudioCache {
    pub fn new(profile_root: PathBuf) -> Result<Arc<Self>> {
        Self::with_transport(profile_root, Arc::new(WinHttpTransport))
    }
    fn with_transport(profile_root: PathBuf, transport: Arc<dyn Transport>) -> Result<Arc<Self>> {
        safe_directory(&profile_root)?;
        let root = profile_root.join("AudioStore");
        safe_directory(&root)?;
        let clips = root.join("Cache");
        safe_directory(&clips)?;
        let library = root.join("Downloads");
        safe_directory(&library)?;
        let (playback, playback_rx) = mpsc::sync_channel(4);
        let (downloads, downloads_rx) = mpsc::sync_channel(4);
        let cache = Arc::new(Self {
            timed_store: Mutex::new(None),
            root,
            clips,
            library,
            index: Mutex::new(BTreeMap::new()),
            pins: Mutex::new(BTreeMap::new()),
            playback,
            downloads,
            prior_playback: Mutex::new(Weak::new()),
            prior_download: Mutex::new(Weak::new()),
            transport,
            index_loaded: AtomicBool::new(false),
            library_limit: AtomicU64::new(LIBRARY_QUOTA),
        });
        for (name, receiver) in [
            ("recitation-buffer", playback_rx),
            ("recitation-download", downloads_rx),
        ] {
            let weak = Arc::downgrade(&cache);
            thread::Builder::new()
                .name(name.into())
                .stack_size(512 * 1024)
                .spawn(move || {
                    while let Ok(task) = receiver.recv() {
                        if let Some(cache) = weak.upgrade() {
                            cache.run(task);
                        } else {
                            break;
                        }
                    }
                })?;
        }
        Ok(cache)
    }
    pub fn folder(&self) -> &Path {
        &self.root
    }
    pub(crate) fn timed_store(&self) -> Result<Arc<crate::timed_store::Store>> {
        let mut store = self.timed_store.lock().unwrap_or_else(|e| e.into_inner());
        if store.is_none() {
            *store = Some(Arc::new(crate::timed_store::Store::new(&self.root)?));
        }
        Ok(store.as_ref().unwrap().clone())
    }
    /// Storage enumeration belongs on a worker. UI may show the fixed limits and
    /// folder immediately; calling stats repeatedly from a paint handler is wrong.
    pub fn stats(&self) -> Result<StorageStats> {
        Ok(StorageStats {
            cache_bytes: directory_bytes(&self.clips)?,
            library_bytes: directory_bytes(&self.library)?,
            cache_limit: CACHE_QUOTA,
            library_limit: self.library_limit.load(Ordering::Acquire),
        })
    }
    pub fn set_library_limit(&self, bytes: u64) {
        self.library_limit.store(bytes, Ordering::Release);
    }
    pub fn prepare_playback(self: &Arc<Self>, keys: Vec<ClipKey>, generation: u64) -> Result<Job> {
        if keys.is_empty() || keys.len() > 6237 {
            return Err("Invalid recitation range".into());
        }
        self.submit(keys, generation, Mode::Playback)
    }
    pub fn download_surah(
        self: &Arc<Self>,
        reciter: usize,
        surah: u32,
        generation: u64,
    ) -> Result<Job> {
        ClipKey::new(reciter, surah, 1)?;
        let mut keys = Vec::new();
        if surah != 1 && surah != 9 {
            keys.push(ClipKey::new(reciter, 1, 1)?);
        }
        for ayah in 1..=COUNTS[(surah - 1) as usize] {
            keys.push(ClipKey::new(reciter, surah, ayah)?);
        }
        self.submit(keys, generation, Mode::Download { reciter, surah })
    }
    fn submit(self: &Arc<Self>, keys: Vec<ClipKey>, generation: u64, mode: Mode) -> Result<Job> {
        for key in &keys {
            ClipKey::new(key.reciter, key.surah, key.ayah)?;
        }
        let state = Arc::new(JobState {
            cancelled: AtomicBool::new(false),
            consumed: AtomicUsize::new(0),
            generation,
            pins: Mutex::new(BTreeMap::new()),
            cache: Arc::downgrade(self),
        });
        let (sender, events) = mpsc::channel();
        let (queue, prior) = match mode {
            Mode::Playback => (&self.playback, &self.prior_playback),
            Mode::Download { .. } => (&self.downloads, &self.prior_download),
        };
        let mut old = prior.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = old.upgrade() {
            previous.cancelled.store(true, Ordering::Release);
        }
        *old = Arc::downgrade(&state);
        queue
            .try_send(Task {
                keys,
                state: state.clone(),
                events: sender,
                mode,
            })
            .map_err(|_| "Recitation preparation is busy; please try again shortly")?;
        Ok(Job { state, events })
    }
    fn run(&self, task: Task) {
        let outcome = match task.mode {
            Mode::Playback => self.buffer(&task),
            Mode::Download { reciter, surah } => self.save_surah(&task, reciter, surah),
        };
        if let Err(error) = outcome {
            let event = if task.state.cancelled.load(Ordering::Acquire) {
                Event::Cancelled {
                    generation: task.state.generation,
                }
            } else {
                Event::Failed {
                    generation: task.state.generation,
                    message: error.to_string(),
                }
            };
            let _ = task.events.send(event);
        }
    }
    fn buffer(&self, task: &Task) -> Result<()> {
        self.load_index()?;
        self.prune(&mut self.index.lock().unwrap_or_else(|e| e.into_inner()), 0)?;
        let mut durations = Vec::with_capacity(task.keys.len());
        let mut bytes = 0;
        for (index, key) in task.keys.iter().copied().enumerate() {
            loop {
                task.state.check()?;
                let consumed = task.state.consumed.load(Ordering::Acquire).min(index);
                if index.saturating_sub(consumed) < AHEAD_CLIPS
                    && durations[(consumed + 1).min(index)..].iter().sum::<f64>() < AHEAD_SECONDS
                {
                    break;
                }
                thread::sleep(Duration::from_millis(80));
            }
            {
                let mut held = task.state.pins.lock().unwrap_or_else(|e| e.into_inner());
                let mut pins = self.pins.lock().unwrap_or_else(|e| e.into_inner());
                *pins.entry(key).or_default() += 1;
                held.insert(index, key);
            }
            // A brief connection failure several clips ahead must not retire
            // an otherwise playable range. Recovery is bounded, stays on this
            // worker, and never retries integrity, provider or storage errors.
            let (path, info) =
                recover_lookahead(index, &task.state, &LOOKAHEAD_RECOVERY_DELAYS, || {
                    self.get_clip(key, &task.state, |_, _| {})
                })?;
            durations.push(info.duration_seconds);
            bytes += info.bytes;
            task.events.send(Event::Ready {
                generation: task.state.generation,
                index,
                key,
                path,
                info,
            })?;
            task.events.send(Event::Progress {
                generation: task.state.generation,
                completed: index + 1,
                total: task.keys.len(),
                percent: ((index + 1) * 100 / task.keys.len()) as u32,
                bytes,
            })?;
        }
        task.events.send(Event::Complete {
            generation: task.state.generation,
            folder: self.clips.clone(),
        })?;
        Ok(())
    }
    fn save_surah(&self, task: &Task, reciter: usize, surah: u32) -> Result<()> {
        let name = format!("reciter-{reciter:02}-surah-{surah:03}");
        let target = self.library.join(&name);
        if target.exists() {
            self.verify_surah(&target, reciter, surah, &task.keys)?;
            task.events.send(Event::Progress {
                generation: task.state.generation,
                completed: task.keys.len(),
                total: task.keys.len(),
                percent: 100,
                bytes: directory_bytes(&target)?,
            })?;
            task.events.send(Event::Complete {
                generation: task.state.generation,
                folder: target,
            })?;
            return Ok(());
        }
        let staging = self.library.join(format!(".{name}-{}.partial", nonce()));
        let existing_bytes = directory_bytes(&self.library)?;
        fs::create_dir(&staging)?;
        let mut transaction = Staging {
            path: staging.clone(),
            committed: false,
            files: BTreeMap::new(),
        };
        let marker = staging.join("transaction.json");
        let mut marker_hash = None;
        let mut receipts = Vec::new();
        let mut bytes = 0u64;
        for (index, key) in task.keys.iter().copied().enumerate() {
            task.state.check()?;
            let _pin = self.pin_clip(key);
            let (path, info) = self.get_clip(key, &task.state, |received, total| {
                let fraction = total
                    .map_or(0.0, |total| received as f64 / total as f64)
                    .min(0.99);
                let percent =
                    (((index as f64 + fraction) / task.keys.len() as f64) * 100.0).min(99.0) as u32;
                let _ = task.events.send(Event::Progress {
                    generation: task.state.generation,
                    completed: index,
                    total: task.keys.len(),
                    percent,
                    bytes: bytes + received,
                });
            })?;
            if existing_bytes
                .saturating_add(bytes)
                .saturating_add(info.bytes)
                .saturating_add(256 * 1024)
                > self.library_limit.load(Ordering::Acquire)
            {
                return Err("The offline download limit has been reached. Increase it in Options > Listening, or remove a saved surah".into());
            }
            let data = read_bounded(&path, mp3::MAX_AUDIO_BYTES)?;
            mp3::inspect(&data)?;
            let digest = sha256(&data)?;
            let file = format!("{}_{digest}.mp3", key.prefix());
            let destination = staging.join(&file);
            write_new(&destination, &data)?;
            transaction.files.insert(destination, digest.clone());
            bytes += info.bytes;
            receipts.push(Receipt {
                schema: "native-audio-clip-v1".into(),
                key,
                file,
                bytes: info.bytes,
                frames: info.frames,
                sha256: digest,
            });
            let partial = SurahManifest {
                schema: "native-audio-partial-v1".into(),
                reciter,
                surah,
                clips: receipts.clone(),
            };
            let marker_bytes = serde_json::to_vec(&partial)?;
            replace_owned_metadata_limit(
                &marker,
                &marker_bytes,
                marker_hash.as_deref(),
                256 * 1024,
            )?;
            let hash = sha256(&marker_bytes)?;
            transaction.files.insert(marker.clone(), hash.clone());
            marker_hash = Some(hash);
            task.events.send(Event::Progress {
                generation: task.state.generation,
                completed: index + 1,
                total: task.keys.len(),
                percent: (((index + 1) * 100 / task.keys.len()).min(99)) as u32,
                bytes,
            })?;
        }
        task.state.check()?;
        let manifest = SurahManifest {
            schema: "native-audio-surah-v1".into(),
            reciter,
            surah,
            clips: receipts,
        };
        let manifest_path = staging.join("surah.json");
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        write_new(&manifest_path, &manifest_bytes)?;
        transaction
            .files
            .insert(manifest_path, sha256(&manifest_bytes)?);
        self.verify_surah(&staging, reciter, surah, &task.keys)?;
        task.state.check()?;
        if marker_hash.as_deref() != Some(sha256(&read_bounded(&marker, 256 * 1024)?)?.as_str()) {
            return Err(
                "Download transaction metadata changed; existing files were preserved".into(),
            );
        }
        fs::remove_file(&marker)?;
        transaction.files.remove(&marker);
        publish_new(&staging, &target)?;
        transaction.committed = true;
        task.events.send(Event::Progress {
            generation: task.state.generation,
            completed: task.keys.len(),
            total: task.keys.len(),
            percent: 100,
            bytes,
        })?;
        task.events.send(Event::Complete {
            generation: task.state.generation,
            folder: target,
        })?;
        Ok(())
    }
    fn verify_surah(
        &self,
        folder: &Path,
        reciter: usize,
        surah: u32,
        keys: &[ClipKey],
    ) -> Result<()> {
        safe_existing_directory(folder)?;
        let manifest: SurahManifest =
            serde_json::from_slice(&read_bounded(&folder.join("surah.json"), 256 * 1024)?)?;
        if manifest.schema != "native-audio-surah-v1"
            || manifest.reciter != reciter
            || manifest.surah != surah
            || manifest.clips.len() != keys.len()
        {
            return Err("Downloaded surah metadata changed; existing files were preserved".into());
        }
        for (receipt, key) in manifest.clips.iter().zip(keys) {
            validate_receipt(receipt)?;
            if receipt.key != *key {
                return Err("Downloaded surah has an incorrect ayah list".into());
            }
            verify_recording(&folder.join(&receipt.file), receipt)?;
        }
        Ok(())
    }
    fn load_index(&self) -> Result<()> {
        if self.index_loaded.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if self.index_loaded.load(Ordering::Acquire) {
            return Ok(());
        }
        for entry in fs::read_dir(&self.clips)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|v| v.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = read_bounded(&path, 4096) else {
                continue;
            };
            let Ok(receipt) = serde_json::from_slice::<Receipt>(&bytes) else {
                continue;
            };
            if validate_receipt(&receipt).is_err()
                || entry.file_name() != format!("{}.json", receipt.key.prefix()).as_str()
            {
                continue;
            }
            index.insert(
                receipt.key,
                CachedReceipt {
                    value: receipt,
                    metadata_hash: sha256(&bytes)?,
                },
            );
        }
        self.cleanup_interrupted()?;
        self.index_loaded.store(true, Ordering::Release);
        Ok(())
    }
    fn cleanup_interrupted(&self) -> Result<()> {
        for entry in fs::read_dir(&self.library)? {
            let entry = entry?;
            let folder = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(".reciter-")
                || !name.ends_with(".partial")
                || safe_existing_directory(&folder).is_err()
            {
                continue;
            }
            let marker = if folder.join("transaction.json").exists() {
                folder.join("transaction.json")
            } else {
                folder.join("surah.json")
            };
            let Ok(marker_bytes) = read_bounded(&marker, 256 * 1024) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_slice::<SurahManifest>(&marker_bytes) else {
                continue;
            };
            let prefix = format!(
                ".reciter-{:02}-surah-{:03}-",
                manifest.reciter, manifest.surah
            );
            let valid_suffix = name
                .strip_prefix(&prefix)
                .and_then(|suffix| suffix.strip_suffix(".partial"))
                .and_then(|suffix| suffix.split_once('-'))
                .is_some_and(|(pid, counter)| {
                    !pid.is_empty()
                        && !counter.is_empty()
                        && pid.bytes().all(|b| b.is_ascii_digit())
                        && counter.bytes().all(|b| b.is_ascii_digit())
                });
            if !valid_suffix
                || !["native-audio-partial-v1", "native-audio-surah-v1"]
                    .contains(&manifest.schema.as_str())
                || ClipKey::new(manifest.reciter, manifest.surah, 1).is_err()
            {
                continue;
            }
            let mut expected = BTreeSet::from([marker.file_name().unwrap().to_os_string()]);
            let mut valid = true;
            for receipt in &manifest.clips {
                if validate_receipt(receipt).is_err()
                    || receipt.key.reciter != manifest.reciter
                    || verify_recording(&folder.join(&receipt.file), receipt).is_err()
                {
                    valid = false;
                    break;
                }
                expected.insert(receipt.file.clone().into());
            }
            if folder.join("surah.json") != marker && folder.join("surah.json").exists() {
                let Ok(bytes) = read_bounded(&folder.join("surah.json"), 256 * 1024) else {
                    continue;
                };
                let Ok(full) = serde_json::from_slice::<SurahManifest>(&bytes) else {
                    continue;
                };
                if full.schema != "native-audio-surah-v1"
                    || full.reciter != manifest.reciter
                    || full.surah != manifest.surah
                    || serde_json::to_vec(&full.clips)? != serde_json::to_vec(&manifest.clips)?
                {
                    continue;
                }
                expected.insert("surah.json".into());
            }
            let actual = fs::read_dir(&folder)?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<std::io::Result<BTreeSet<_>>>()?;
            if !valid || expected != actual {
                continue;
            }
            for file in expected {
                fs::remove_file(folder.join(file))?;
            }
            fs::remove_dir(folder)?;
        }
        Ok(())
    }
    fn pin_clip(&self, key: ClipKey) -> ClipPin<'_> {
        *self
            .pins
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key)
            .or_default() += 1;
        ClipPin { cache: self, key }
    }
    fn get_clip(
        &self,
        key: ClipKey,
        state: &JobState,
        mut progress: impl FnMut(u64, Option<u64>),
    ) -> Result<(PathBuf, Mp3Info)> {
        state.check()?;
        // Downloaded surahs remain available even after the temporary cache ages.
        let surah_folder = self
            .library
            .join(format!("reciter-{:02}-surah-{:03}", key.reciter, key.surah));
        if surah_folder.exists() {
            safe_existing_directory(&surah_folder)?;
            let manifest: SurahManifest = serde_json::from_slice(&read_bounded(
                &surah_folder.join("surah.json"),
                256 * 1024,
            )?)?;
            if manifest.schema == "native-audio-surah-v1"
                && manifest.reciter == key.reciter
                && manifest.surah == key.surah
            {
                if let Some(receipt) = manifest.clips.iter().find(|r| r.key == key) {
                    validate_receipt(receipt)?;
                    let path = surah_folder.join(&receipt.file);
                    return Ok((path.clone(), verify_recording(&path, receipt)?));
                }
            }
        }
        // Every downloaded surah includes its opening clip. Reuse that saved
        // Basmala offline even when the temporary 1:1 cache has been evicted.
        if key.surah == 1 && key.ayah == 1 {
            for entry in fs::read_dir(&self.library)? {
                let entry = entry?;
                let folder = entry.path();
                if !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("reciter-{:02}-surah-", key.reciter))
                    || safe_existing_directory(&folder).is_err()
                {
                    continue;
                }
                let Ok(bytes) = read_bounded(&folder.join("surah.json"), 256 * 1024) else {
                    continue;
                };
                let Ok(manifest) = serde_json::from_slice::<SurahManifest>(&bytes) else {
                    continue;
                };
                if manifest.schema != "native-audio-surah-v1" || manifest.reciter != key.reciter {
                    continue;
                }
                if let Some(receipt) = manifest.clips.iter().find(|r| r.key == key) {
                    if validate_receipt(receipt).is_ok() {
                        if let Ok(info) = verify_recording(&folder.join(&receipt.file), receipt) {
                            return Ok((folder.join(&receipt.file), info));
                        }
                    }
                }
            }
        }
        // A validated saved clip can start without enumerating the entire
        // cache or repairing unrelated interrupted downloads. Maintenance and
        // quota accounting still run before any new cache publication.
        let metadata = self.clips.join(format!("{}.json", key.prefix()));
        if let Ok(bytes) = read_bounded(&metadata, 4096) {
            if let Ok(receipt) = serde_json::from_slice::<Receipt>(&bytes) {
                if receipt.key == key && validate_receipt(&receipt).is_ok() {
                    let path = self.clips.join(&receipt.file);
                    if let Ok(info) = verify_recording(&path, &receipt) {
                        state.check()?;
                        return Ok((path, info));
                    }
                }
            }
        }
        self.load_index()?;
        let cached = {
            self.index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&key)
                .map(|r| r.value.clone())
        };
        if let Some(receipt) = cached {
            let path = self.clips.join(&receipt.file);
            if let Ok(info) = verify_recording(&path, &receipt) {
                return Ok((path, info));
            }
            // A changed clip is never played or overwritten. A content-addressed
            // replacement may be saved under a different name after validation.
        }
        let url = audio::url(key.reciter, key.surah, key.ayah)?;
        let bytes = fetch_with_retry(&url, state, &mut progress, self.transport.as_ref())?;
        let info = mp3::inspect(&bytes)?;
        let digest = sha256(&bytes)?;
        let mut receipt = Receipt {
            schema: "native-audio-clip-v1".into(),
            key,
            file: format!("{}_{digest}.mp3", key.prefix()),
            bytes: info.bytes,
            frames: info.frames,
            sha256: digest,
        };
        let mut path = self.clips.join(&receipt.file);
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if path.exists() && verify_recording(&path, &receipt).is_err() {
            receipt.file = format!("{}_{}-repair-{}.mp3", key.prefix(), receipt.sha256, nonce());
            path = self.clips.join(&receipt.file);
        }
        if !path.exists() {
            self.prune(&mut index, info.bytes)?;
            state.check()?;
            write_recording_atomic(&path, &bytes)?;
        }
        let metadata = self.clips.join(format!("{}.json", key.prefix()));
        let previous = index.get(&key).map(|r| r.metadata_hash.as_str());
        let receipt_bytes = serde_json::to_vec(&receipt)?;
        replace_owned_metadata(&metadata, &receipt_bytes, previous)?;
        index.insert(
            key,
            CachedReceipt {
                value: receipt,
                metadata_hash: sha256(&receipt_bytes)?,
            },
        );
        Ok((path, info))
    }
    fn prune(&self, index: &mut BTreeMap<ClipKey, CachedReceipt>, incoming: u64) -> Result<()> {
        let mut size = directory_bytes(&self.clips)?;
        let pins = self.pins.lock().unwrap_or_else(|e| e.into_inner());
        let mut candidates: Vec<_> = index
            .iter()
            .filter(|(key, _)| !pins.contains_key(key))
            .map(|(key, r)| {
                (
                    *key,
                    fs::metadata(self.clips.join(&r.value.file))
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH),
                )
            })
            .collect();
        candidates.sort_by_key(|(_, modified)| *modified);
        for (key, _) in candidates {
            if size.saturating_add(incoming + 4096) <= CACHE_QUOTA {
                break;
            }
            let receipt = &index[&key];
            let path = self.clips.join(&receipt.value.file);
            let metadata = self.clips.join(format!("{}.json", key.prefix()));
            if verify_recording(&path, &receipt.value).is_err() {
                continue;
            }
            let Ok(current) = read_bounded(&metadata, 4096) else {
                continue;
            };
            if sha256(&current)? != receipt.metadata_hash {
                continue;
            }
            // Media Foundation may release a played item's file handle after
            // publishing its index change. Skip that file and try another
            // verified candidate; preserve its receipt until deletion succeeds.
            if fs::remove_file(&path).is_err() {
                continue;
            }
            fs::remove_file(&metadata)?;
            size = size.saturating_sub(receipt.value.bytes + current.len() as u64);
            index.remove(&key);
        }
        if size.saturating_add(incoming + 4096) > CACHE_QUOTA {
            return Err(
                "Temporary audio storage is full; active and modified files were preserved".into(),
            );
        }
        Ok(())
    }
}

fn nonce() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    )
}
pub(crate) fn safe_existing_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("Recitation storage cannot use a linked directory".into());
    }
    Ok(())
}
pub(crate) fn safe_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    safe_existing_directory(path)
}
pub(crate) fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.file_attributes() & REPARSE_POINT != 0
        || metadata.len() > limit as u64
    {
        return Err("Invalid recitation storage file".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("Recitation file grew beyond its limit".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}
fn write_recording_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.partial", nonce()));
    write_new(&temporary, bytes)?;
    let result = publish_new(&temporary, path);
    if result.is_err()
        && read_bounded(&temporary, mp3::MAX_AUDIO_BYTES)
            .ok()
            .is_some_and(|data| data == bytes)
    {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn publish_new(source: &Path, destination: &Path) -> Result<()> {
    unsafe {
        use windows::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let source = wide_path(source);
        let destination = wide_path(destination);
        // Absence of REPLACE_EXISTING is intentional: a file arriving between
        // the check and commit is preserved instead of silently overwritten.
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )?;
    }
    Ok(())
}
fn validate_receipt(receipt: &Receipt) -> Result<()> {
    ClipKey::new(receipt.key.reciter, receipt.key.surah, receipt.key.ayah)?;
    let base = format!("{}_{}", receipt.key.prefix(), receipt.sha256);
    let valid_name = receipt.file == format!("{base}.mp3")
        || receipt
            .file
            .strip_prefix(&format!("{base}-repair-"))
            .and_then(|suffix| suffix.strip_suffix(".mp3"))
            .and_then(|suffix| suffix.split_once('-'))
            .is_some_and(|(pid, counter)| {
                !pid.is_empty()
                    && !counter.is_empty()
                    && pid.bytes().all(|b| b.is_ascii_digit())
                    && counter.bytes().all(|b| b.is_ascii_digit())
            });
    if receipt.schema != "native-audio-clip-v1"
        || receipt.sha256.len() != 64
        || !receipt
            .sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || !valid_name
        || receipt.bytes == 0
        || receipt.bytes > mp3::MAX_AUDIO_BYTES as u64
        || receipt.frames == 0
        || u64::from(receipt.frames) > receipt.bytes / 4
    {
        return Err("Invalid recitation metadata".into());
    }
    Ok(())
}
fn verify_recording(path: &Path, receipt: &Receipt) -> Result<Mp3Info> {
    let bytes = read_bounded(path, mp3::MAX_AUDIO_BYTES)?;
    if bytes.len() as u64 != receipt.bytes || sha256(&bytes)? != receipt.sha256 {
        return Err("Cached recitation failed its integrity check".into());
    }
    let info = mp3::inspect(&bytes)?;
    if info.frames != receipt.frames {
        return Err("Cached recitation frame count changed".into());
    }
    Ok(info)
}
pub(crate) fn directory_bytes(path: &Path) -> Result<u64> {
    directory_bytes_at(path, 0)
}
fn directory_bytes_at(path: &Path, depth: usize) -> Result<u64> {
    if depth > 4 {
        return Err("Recitation storage contains an unexpected nested directory".into());
    }
    safe_existing_directory(path)?;
    let mut size = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err("Recitation storage contains an unexpected link".into());
        }
        size = size.saturating_add(if metadata.is_dir() {
            directory_bytes_at(&entry.path(), depth + 1)?
        } else {
            metadata.len()
        });
    }
    Ok(size)
}
fn replace_owned_metadata(path: &Path, bytes: &[u8], previous: Option<&str>) -> Result<()> {
    replace_owned_metadata_limit(path, bytes, previous, 4096)
}
fn replace_owned_metadata_limit(
    path: &Path,
    bytes: &[u8],
    previous: Option<&str>,
    limit: usize,
) -> Result<()> {
    if path.exists() {
        let current = read_bounded(path, limit)?;
        if previous != Some(sha256(&current)?.as_str()) {
            return Err("Cache metadata changed; existing files were preserved".into());
        }
    } else if previous.is_some() {
        return Err("Cache metadata disappeared; existing files were preserved".into());
    }
    let temporary = path.with_extension(format!("{}.partial", nonce()));
    write_new(&temporary, bytes)?;
    let result = unsafe {
        use windows::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let source = wide_path(&temporary);
        let destination = wide_path(path);
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(())
}
fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
pub(crate) fn sha256(bytes: &[u8]) -> Result<String> {
    unsafe {
        let mut algorithm = BCRYPT_ALG_HANDLE::default();
        BCryptOpenAlgorithmProvider(
            &mut algorithm,
            BCRYPT_SHA256_ALGORITHM,
            PCWSTR::null(),
            BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0),
        )
        .ok()?;
        let mut digest = [0; 32];
        let result = BCryptHash(algorithm, None, bytes, &mut digest).ok();
        let _ = BCryptCloseAlgorithmProvider(algorithm, 0);
        result?;
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}
struct Staging {
    path: PathBuf,
    files: BTreeMap<PathBuf, String>,
    committed: bool,
}
struct ClipPin<'a> {
    cache: &'a AudioCache,
    key: ClipKey,
}
impl Drop for ClipPin<'_> {
    fn drop(&mut self) {
        let mut pins = self.cache.pins.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = pins.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                pins.remove(&self.key);
            }
        }
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        if !self.committed {
            // Only exact files created in this transaction; never recursive delete.
            for (path, digest) in &self.files {
                if read_bounded(path, mp3::MAX_AUDIO_BYTES)
                    .ok()
                    .and_then(|bytes| sha256(&bytes).ok())
                    .as_ref()
                    == Some(digest)
                {
                    let _ = fs::remove_file(path);
                }
            }
            let _ = fs::remove_dir(&self.path);
        }
    }
}

trait Transport: Send + Sync {
    fn fetch(
        &self,
        url: &str,
        state: &JobState,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>>;
}
#[derive(Debug)]
struct TemporaryFetchFailure(String);
impl std::fmt::Display for TemporaryFetchFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for TemporaryFetchFailure {}
fn transient_fetch_error(error: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    error.is::<TemporaryFetchFailure>()
        || error
            .downcast_ref::<windows::core::Error>()
            .is_some_and(|error| {
                [
                    ERROR_WINHTTP_TIMEOUT,
                    ERROR_WINHTTP_NAME_NOT_RESOLVED,
                    ERROR_WINHTTP_CANNOT_CONNECT,
                    ERROR_WINHTTP_CONNECTION_ERROR,
                ]
                .iter()
                .any(|code| error.code() == windows::core::HRESULT::from_win32(*code))
            })
}
fn recover_lookahead<T>(
    index: usize,
    state: &JobState,
    delays: &[Duration],
    mut prepare: impl FnMut() -> Result<T>,
) -> Result<T> {
    let mut recovery = 0;
    loop {
        state.check()?;
        match prepare() {
            Ok(value) => return Ok(value),
            Err(error)
                if index > 0
                    && recovery < delays.len()
                    && transient_fetch_error(error.as_ref()) =>
            {
                let begin = Instant::now();
                let delay = delays[recovery];
                while begin.elapsed() < delay {
                    state.check()?;
                    thread::sleep(
                        delay
                            .saturating_sub(begin.elapsed())
                            .min(Duration::from_millis(50)),
                    );
                }
                recovery += 1;
            }
            Err(error) => return Err(error),
        }
    }
}
fn fetch_with_retry(
    url: &str,
    state: &JobState,
    progress: &mut dyn FnMut(u64, Option<u64>),
    transport: &(impl Transport + ?Sized),
) -> Result<Vec<u8>> {
    let mut last = "Could not download this recitation".to_string();
    let mut temporary = false;
    for attempt in 0..RETRIES {
        state.check()?;
        match transport.fetch(url, state, progress).and_then(|bytes| {
            mp3::inspect(&bytes)?;
            Ok(bytes)
        }) {
            Ok(bytes) => return Ok(bytes),
            Err(error) => {
                temporary = transient_fetch_error(error.as_ref());
                last = error.to_string();
            }
        }
        state.check()?;
        if attempt + 1 < RETRIES {
            for _ in 0..(5 * (attempt + 1)) {
                state.check()?;
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
    let message = format!(
        "Could not prepare recitation after {RETRIES} attempts: {last}. Check the connection and retry"
    );
    if temporary {
        Err(TemporaryFetchFailure(message).into())
    } else {
        Err(message.into())
    }
}
struct InternetHandle(*mut c_void);
impl InternetHandle {
    fn new(handle: *mut c_void) -> Result<Self> {
        if handle.is_null() {
            Err(windows::core::Error::from_thread().into())
        } else {
            Ok(Self(handle))
        }
    }
}
impl Drop for InternetHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}
struct WinHttpTransport;
impl Transport for WinHttpTransport {
    fn fetch(
        &self,
        url: &str,
        state: &JobState,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>> {
        let (host, path) = provider_url(url)?;
        let host = wide(&host);
        let path = wide(&path);
        state.check()?;
        let started = Instant::now();
        unsafe {
            let session = InternetHandle::new(WinHttpOpen(
                w!("QuranMushafReader-Native/0.1"),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ))?;
            WinHttpSetTimeouts(session.0, 8_000, 8_000, 8_000, 8_000)?;
            // TLS 1.2 works on the minimum Windows release. The OS certificate
            // chain and hostname validation remain enabled; no insecure flags.
            crate::http_security::configure_protocols(session.0)?;
            WinHttpSetOption(
                Some(session.0),
                WINHTTP_OPTION_REDIRECT_POLICY,
                Some(&WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes()),
            )?;
            let connection =
                InternetHandle::new(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), 443, 0))?;
            let request = InternetHandle::new(WinHttpOpenRequest(
                connection.0,
                w!("GET"),
                PCWSTR(path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ))?;
            WinHttpSetOption(
                Some(request.0),
                WINHTTP_OPTION_AUTOLOGON_POLICY,
                Some(&WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH.to_ne_bytes()),
            )?;
            WinHttpSetOption(
                Some(request.0),
                WINHTTP_OPTION_DISABLE_FEATURE,
                Some(
                    &(WINHTTP_DISABLE_AUTHENTICATION
                        | WINHTTP_DISABLE_COOKIES
                        | WINHTTP_DISABLE_REDIRECTS)
                        .to_ne_bytes(),
                ),
            )?;
            let headers: Vec<_> =
                "Accept: audio/mpeg, application/octet-stream\r\nAccept-Encoding: identity\r\n"
                    .encode_utf16()
                    .collect();
            WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0)?;
            state.check()?;
            WinHttpReceiveResponse(request.0, std::ptr::null_mut())?;
            let mut status = 0u32;
            let mut length = 4;
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some((&mut status as *mut u32).cast()),
                &mut length,
                std::ptr::null_mut(),
            )?;
            let content_type = header(request.0, WINHTTP_QUERY_CONTENT_TYPE)?;
            let length_text = header(request.0, WINHTTP_QUERY_CONTENT_LENGTH)?;
            let expected = response_length(status, &content_type, &length_text)?;
            let mut bytes = Vec::with_capacity(
                expected
                    .unwrap_or(32 * 1024)
                    .min(mp3::MAX_AUDIO_BYTES as u64) as usize,
            );
            let mut buffer = [0u8; 32 * 1024];
            let mut last_progress = Instant::now();
            loop {
                state.check()?;
                if started.elapsed() > Duration::from_secs(45) {
                    return Err(TemporaryFetchFailure(
                        "Recitation download stalled or exceeded its time limit".into(),
                    )
                    .into());
                }
                let mut count = 0;
                WinHttpReadData(
                    request.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut count,
                )?;
                if count == 0 {
                    break;
                }
                if bytes.len() + count as usize > mp3::MAX_AUDIO_BYTES {
                    return Err("Recitation download grew beyond its limit".into());
                }
                bytes.extend_from_slice(&buffer[..count as usize]);
                if last_progress.elapsed() >= Duration::from_millis(150) {
                    progress(bytes.len() as u64, expected);
                    last_progress = Instant::now();
                }
            }
            complete_response(&bytes, expected)?;
            state.check()?;
            progress(bytes.len() as u64, expected);
            mp3::inspect(&bytes)?;
            Ok(bytes)
        }
    }
}
fn response_length(status: u32, content_type: &str, length_text: &str) -> Result<Option<u64>> {
    if status != 200 {
        return Err(format!("Recitation provider returned HTTP {status}").into());
    }
    if mp3::error_content_type(content_type) {
        return Err("Recitation provider returned an error page instead of audio".into());
    }
    let expected = if length_text.is_empty() {
        None
    } else {
        Some(
            length_text
                .parse::<u64>()
                .map_err(|_| "Invalid audio Content-Length")?,
        )
    };
    if expected.is_some_and(|n| n == 0 || n > mp3::MAX_AUDIO_BYTES as u64) {
        return Err("Recitation download exceeds the clip limit".into());
    }
    Ok(expected)
}
fn complete_response(bytes: &[u8], expected: Option<u64>) -> Result<()> {
    if expected.is_some_and(|n| n != bytes.len() as u64) {
        return Err("Recitation download is incomplete (Content-Length mismatch)".into());
    }
    mp3::inspect(bytes)?;
    Ok(())
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn provider_url(url: &str) -> Result<(String, String)> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err("Recitation transport requires HTTPS".into());
    };
    let Some((host, suffix)) = rest.split_once('/') else {
        return Err("Invalid recitation URL".into());
    };
    if !["everyayah.com", "verses.quran.foundation"].contains(&host)
        || suffix.contains("..")
        || suffix
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && !b"/_-.".contains(&b))
        || !suffix.ends_with(".mp3")
        || suffix.len() > 160
    {
        return Err("Recitation URL is outside the trusted provider paths".into());
    }
    // Verify against the complete provider/reciter URL table, rather than merely
    // accepting another endpoint on a permitted host.
    let allowed = audio::RECITERS.iter().enumerate().any(|(reciter, _)| {
        let prefix = audio::url(reciter, 1, 1).unwrap_or_default();
        prefix.rsplit_once('/').is_some_and(|(base, _)| {
            url.rsplit_once('/')
                .is_some_and(|(actual, _)| actual == base)
        })
    });
    let name = suffix.rsplit('/').next().unwrap_or("");
    if !allowed || name.len() != 10 || !name[..6].bytes().all(|b| b.is_ascii_digit()) {
        return Err("Recitation URL has an invalid clip path".into());
    }
    let surah = name[..3].parse::<u32>()?;
    let ayah = name[3..6].parse::<u32>()?;
    if !(1..=114).contains(&surah) || ayah == 0 || ayah > COUNTS[(surah - 1) as usize] {
        return Err("Invalid recitation URL reference".into());
    }
    Ok((host.into(), format!("/{suffix}")))
}
pub(crate) unsafe fn header(request: *mut c_void, kind: u32) -> Result<String> {
    let mut bytes = 0;
    let first = unsafe {
        WinHttpQueryHeaders(
            request,
            kind,
            PCWSTR::null(),
            None,
            &mut bytes,
            std::ptr::null_mut(),
        )
    };
    if first.is_err() && bytes == 0 {
        return Ok(String::new());
    }
    if bytes > 8192 || bytes % 2 != 0 {
        return Err("Audio response header exceeds its limit".into());
    }
    let mut buffer = vec![0u16; bytes as usize / 2];
    unsafe {
        WinHttpQueryHeaders(
            request,
            kind,
            PCWSTR::null(),
            Some(buffer.as_mut_ptr().cast()),
            &mut bytes,
            std::ptr::null_mut(),
        )?;
    }
    Ok(String::from_utf16(&buffer)
        .map_err(|_| "Invalid audio response header")?
        .trim_end_matches('\0')
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Responses {
        items: Mutex<VecDeque<std::result::Result<Vec<u8>, &'static str>>>,
        calls: AtomicUsize,
    }
    impl Responses {
        fn new(items: Vec<std::result::Result<Vec<u8>, &'static str>>) -> Arc<Self> {
            Arc::new(Self {
                items: Mutex::new(items.into()),
                calls: AtomicUsize::new(0),
            })
        }
    }
    impl Transport for Responses {
        fn fetch(
            &self,
            _: &str,
            state: &JobState,
            progress: &mut dyn FnMut(u64, Option<u64>),
        ) -> Result<Vec<u8>> {
            state.check()?;
            self.calls.fetch_add(1, Ordering::Relaxed);
            let bytes = self
                .items
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err("Unexpected fixture request"))
                .map_err(|e| e.to_string())?;
            progress(bytes.len() as u64, Some(bytes.len() as u64));
            Ok(bytes)
        }
    }
    struct TestFolder(PathBuf);
    impl TestFolder {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("quran-native-audio-test-{}", nonce()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestFolder {
        fn drop(&mut self) {
            // A freshly create_new-owned fixture, never a user-selected folder.
            if self.0.parent() == Some(std::env::temp_dir().as_path())
                && self
                    .0
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("quran-native-audio-test-")
            {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }
    fn state() -> JobState {
        JobState {
            cancelled: AtomicBool::new(false),
            consumed: AtomicUsize::new(0),
            generation: 77,
            pins: Mutex::new(BTreeMap::new()),
            cache: Weak::new(),
        }
    }

    #[test]
    fn https_provider_paths_and_references_are_closed() {
        for reciter in 0..audio::RECITERS.len() {
            assert!(provider_url(&audio::url(reciter, 2, 255).unwrap()).is_ok());
        }
        for url in [
            "http://everyayah.com/data/Alafasy/001001.mp3",
            "https://everyayah.com.evil/data/Alafasy/001001.mp3",
            "https://everyayah.com/data/Yasser_Ad-Dussary_128kbps/../001001.mp3",
            "https://everyayah.com/data/Yasser_Ad-Dussary_128kbps/extra/001001.mp3",
            "https://everyayah.com/data/Yasser_Ad-Dussary_128kbps/001001.mp3?q=secret",
            "https://everyayah.com/data/Yasser_Ad-Dussary_128kbps/112005.mp3",
        ] {
            assert!(provider_url(url).is_err(), "{url}");
        }
        assert!(ClipKey::new(0, 112, 5).is_err());
        assert_eq!(COUNTS.iter().sum::<u32>(), 6236);
    }
    #[test]
    fn retries_reject_partial_html_and_stalled_responses_before_playback() {
        let bytes = mp3::fixture(10);
        let transport = Responses::new(vec![
            Err("receive timeout"),
            Ok(bytes[..bytes.len() - 1].to_vec()),
            Ok(bytes.clone()),
        ]);
        let output = fetch_with_retry(
            "fixed-provider",
            &state(),
            &mut |_, _| {},
            transport.as_ref(),
        )
        .unwrap();
        assert_eq!(output, bytes);
        assert_eq!(transport.calls.load(Ordering::Relaxed), 3);
        let html = Responses::new(vec![Ok(b"<html>HTTP200 error</html>".to_vec()); 3]);
        assert!(
            fetch_with_retry("fixed-provider", &state(), &mut |_, _| {}, html.as_ref()).is_err()
        );
        assert_eq!(html.calls.load(Ordering::Relaxed), 3);
    }
    #[test]
    fn lookahead_recovers_only_temporary_connection_errors_with_a_fixed_limit() {
        let state = state();
        let delays = [Duration::ZERO; 2];
        let mut attempts = 0;
        let value = recover_lookahead(3, &state, &delays, || -> Result<_> {
            attempts += 1;
            if attempts < 3 {
                return Err(TemporaryFetchFailure("Temporary connection loss".into()).into());
            }
            Ok("validated clip")
        })
        .unwrap();
        assert_eq!(value, "validated clip");
        assert_eq!(attempts, 3);

        attempts = 0;
        assert!(
            recover_lookahead(3, &state, &delays, || -> Result<()> {
                attempts += 1;
                Err(TemporaryFetchFailure("Temporary connection loss".into()).into())
            })
            .is_err()
        );
        assert_eq!(attempts, 3, "No unbounded retries while a range is paused");
        for index in [0, 3] {
            attempts = 0;
            assert!(
                recover_lookahead(index, &state, &delays, || -> Result<()> {
                    attempts += 1;
                    Err("Cached recitation failed its integrity check".into())
                })
                .is_err()
            );
            assert_eq!(
                attempts, 1,
                "An integrity failure is never background-retried"
            );
        }
        attempts = 0;
        assert!(
            recover_lookahead(0, &state, &delays, || -> Result<()> {
                attempts += 1;
                Err(TemporaryFetchFailure("Temporary connection loss".into()).into())
            })
            .is_err()
        );
        assert_eq!(
            attempts, 1,
            "Initial playback retains its existing retry bound"
        );
    }
    #[test]
    fn lookahead_backoff_observes_cancellation_without_another_request() {
        let state = state();
        let begin = Instant::now();
        let mut attempts = 0;
        assert!(
            recover_lookahead(1, &state, &[Duration::from_secs(5)], || -> Result<()> {
                attempts += 1;
                state.cancelled.store(true, Ordering::Release);
                Err(TemporaryFetchFailure("Temporary connection loss".into()).into())
            })
            .is_err()
        );
        assert_eq!(attempts, 1);
        assert!(begin.elapsed() < Duration::from_millis(250));
    }
    #[test]
    fn temporary_fetch_classification_excludes_certificate_and_provider_errors() {
        for code in [
            ERROR_WINHTTP_TIMEOUT,
            ERROR_WINHTTP_NAME_NOT_RESOLVED,
            ERROR_WINHTTP_CANNOT_CONNECT,
            ERROR_WINHTTP_CONNECTION_ERROR,
        ] {
            let error =
                windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(code));
            assert!(transient_fetch_error(&error));
        }
        let certificate = windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(
            ERROR_WINHTTP_SECURE_FAILURE,
        ));
        assert!(!transient_fetch_error(&certificate));
        let provider: Box<dyn std::error::Error + Send + Sync> =
            "Recitation provider returned HTTP 404".into();
        assert!(!transient_fetch_error(provider.as_ref()));
    }
    #[test]
    fn pruning_skips_a_decoder_locked_clip_and_preserves_its_receipt() {
        use std::os::windows::fs::OpenOptionsExt;
        let folder = TestFolder::new();
        let bytes = mp3::fixture(20_000);
        let transport = Responses::new(vec![Ok(bytes.clone()), Ok(bytes)]);
        let cache = AudioCache::with_transport(folder.0.clone(), transport).unwrap();
        let first = ClipKey::new(0, 112, 1).unwrap();
        let second = ClipKey::new(0, 112, 2).unwrap();
        let (first_path, _) = cache.get_clip(first, &state(), |_, _| {}).unwrap();
        let (second_path, _) = cache.get_clip(second, &state(), |_, _| {}).unwrap();
        OpenOptions::new()
            .write(true)
            .open(&first_path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        let first_receipt = cache.clips.join(format!("{}.json", first.prefix()));
        let original_receipt = fs::read(&first_receipt).unwrap();
        // An unrelated file remains untouched even if it occupies quota.
        let unrelated = cache.clips.join("unrelated-user-content.bin");
        write_new(&unrelated, &vec![0; 18 * 1024 * 1024]).unwrap();
        let decoder = OpenOptions::new()
            .read(true)
            .share_mode(1) // FILE_SHARE_READ, deliberately no FILE_SHARE_DELETE.
            .open(&first_path)
            .unwrap();
        assert!(directory_bytes(&cache.clips).unwrap() > CACHE_QUOTA);
        cache.prune(&mut cache.index.lock().unwrap(), 0).unwrap();
        assert!(first_path.exists());
        assert_eq!(fs::read(first_receipt).unwrap(), original_receipt);
        assert!(!second_path.exists());
        assert!(
            !cache
                .clips
                .join(format!("{}.json", second.prefix()))
                .exists()
        );
        assert_eq!(fs::metadata(unrelated).unwrap().len(), 18 * 1024 * 1024);
        assert!(cache.index.lock().unwrap().contains_key(&first));
        assert!(!cache.index.lock().unwrap().contains_key(&second));
        assert!(directory_bytes(&cache.clips).unwrap() < CACHE_QUOTA);
        drop(decoder);
    }
    #[test]
    fn response_size_and_error_mime_validation_are_independent() {
        let bytes = mp3::fixture(8);
        assert_eq!(
            response_length(200, "application/octet-stream", &bytes.len().to_string()).unwrap(),
            Some(bytes.len() as u64)
        );
        assert!(response_length(200, "text/html", "200").is_err());
        assert!(response_length(302, "audio/mpeg", "200").is_err());
        assert!(response_length(206, "audio/mpeg", "200").is_err());
        assert!(response_length(200, "audio/mpeg", "999999999").is_err());
        assert!(response_length(200, "audio/mpeg", "4096,4096").is_err());
        assert!(complete_response(&bytes, Some(bytes.len() as u64 + 417)).is_err());
        assert!(complete_response(b"<html>error with octet-stream MIME</html>", None).is_err());
        assert!(complete_response(&bytes, None).is_ok());
    }
    #[test]
    fn cancellation_performs_no_more_requests() {
        let transport = Responses::new(vec![Ok(mp3::fixture(8))]);
        let state = state();
        state.cancelled.store(true, Ordering::Release);
        assert!(
            fetch_with_retry("fixed-provider", &state, &mut |_, _| {}, transport.as_ref()).is_err()
        );
        assert_eq!(transport.calls.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn atomic_new_clip_commit_never_replaces_an_existing_file() {
        let folder = TestFolder::new();
        let destination = folder.0.join("existing.mp3");
        write_new(&destination, b"Keep this user file").unwrap();
        assert!(write_recording_atomic(&destination, &mp3::fixture(8)).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"Keep this user file");
        assert_eq!(fs::read_dir(&folder.0).unwrap().count(), 1);
        let new = folder.0.join("new.mp3");
        write_recording_atomic(&new, &mp3::fixture(8)).unwrap();
        assert_eq!(mp3::inspect(&fs::read(new).unwrap()).unwrap().frames, 8);
    }
    #[test]
    fn verified_cached_clip_starts_before_global_cache_maintenance() {
        let folder = TestFolder::new();
        let transport = Responses::new(vec![]);
        let cache = AudioCache::with_transport(folder.0.clone(), transport.clone()).unwrap();
        let key = ClipKey::new(0, 112, 1).unwrap();
        let bytes = mp3::fixture(8);
        let hash = sha256(&bytes).unwrap();
        let receipt = Receipt {
            schema: "native-audio-clip-v1".into(),
            key,
            file: format!("{}_{hash}.mp3", key.prefix()),
            bytes: bytes.len() as u64,
            frames: 8,
            sha256: hash,
        };
        let path = cache.clips.join(&receipt.file);
        write_new(&path, &bytes).unwrap();
        write_new(
            &cache.clips.join(format!("{}.json", key.prefix())),
            &serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        let unrelated = cache.clips.join("unrelated.json");
        write_new(&unrelated, b"Preserve this unrelated user content").unwrap();
        let (ready, info) = cache.get_clip(key, &state(), |_, _| {}).unwrap();
        assert_eq!(ready, path);
        assert_eq!(info.frames, 8);
        assert!(!cache.index_loaded.load(Ordering::Acquire));
        assert_eq!(transport.calls.load(Ordering::Relaxed), 0);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            fs::read(unrelated).unwrap(),
            b"Preserve this unrelated user content"
        );
    }
    #[test]
    fn lookahead_excludes_current_long_ayah_and_waits_for_consumption() {
        let folder = TestFolder::new();
        let transport = Responses::new(vec![
            Ok(mp3::fixture(8_000)),
            Ok(mp3::fixture(8_000)),
            Ok(mp3::fixture(8)),
            Ok(mp3::fixture(8)),
        ]);
        let cache = AudioCache::with_transport(folder.0.clone(), transport.clone()).unwrap();
        let keys = (1..=4)
            .map(|ayah| ClipKey::new(0, 112, ayah).unwrap())
            .collect();
        let job = cache.prepare_playback(keys, 77).unwrap();
        let mut ready = Vec::new();
        let begin = Instant::now();
        while ready.len() < 2 && begin.elapsed() < Duration::from_secs(5) {
            if let Some(event) = job.try_recv() {
                match event {
                    Event::Ready {
                        index, generation, ..
                    } => {
                        assert_eq!(generation, 77);
                        ready.push(index);
                    }
                    Event::Failed { message, .. } => panic!("{message}"),
                    _ => {}
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert_eq!(
            ready,
            [0, 1],
            "A current long ayah must not deadlock its next clip"
        );
        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            transport.calls.load(Ordering::Relaxed),
            2,
            "Future duration must bound preparation"
        );
        job.set_consumed(1);
        while ready.len() < 4 && begin.elapsed() < Duration::from_secs(5) {
            if let Some(event) = job.try_recv() {
                match event {
                    Event::Ready { index, .. } => ready.push(index),
                    Event::Failed { message, .. } => panic!("{message}"),
                    _ => {}
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        job.set_consumed(3);
        let held: Vec<_> = job.state.pins.lock().unwrap().keys().copied().collect();
        assert_eq!(
            held,
            vec![2, 3],
            "Played files must become eligible for eviction"
        );
        assert_eq!(ready, [0, 1, 2, 3]);
    }
    #[test]
    fn corrupt_cached_audio_is_not_played_or_overwritten() {
        let folder = TestFolder::new();
        let original = mp3::fixture(10);
        let transport = Responses::new(vec![Ok(original.clone()), Ok(original.clone())]);
        let cache = AudioCache::with_transport(folder.0.clone(), transport.clone()).unwrap();
        let key = ClipKey::new(0, 112, 1).unwrap();
        let (first, _) = cache.get_clip(key, &state(), |_, _| {}).unwrap();
        fs::write(&first, b"<html>cache damaged</html>").unwrap();
        let (replacement, info) = cache.get_clip(key, &state(), |_, _| {}).unwrap();
        assert_ne!(first, replacement);
        assert_eq!(info.frames, 10);
        assert_eq!(fs::read(&first).unwrap(), b"<html>cache damaged</html>");
        assert_eq!(fs::read(&replacement).unwrap(), original);
        assert_eq!(transport.calls.load(Ordering::Relaxed), 2);
        let (_, _) = cache.get_clip(key, &state(), |_, _| {}).unwrap();
        assert_eq!(transport.calls.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn complete_surah_commits_once_with_basmala_and_survives_cache_eviction() {
        let folder = TestFolder::new();
        let transport = Responses::new(vec![Ok(mp3::fixture(8)); 5]);
        let cache = AudioCache::with_transport(folder.0.clone(), transport.clone()).unwrap();
        let job = cache.download_surah(0, 112, 77).unwrap();
        let begin = Instant::now();
        let mut committed = None;
        let mut reached_100 = false;
        while begin.elapsed() < Duration::from_secs(5) {
            if let Some(event) = job.try_recv() {
                match event {
                    Event::Progress { percent: 100, .. } => {
                        reached_100 = true;
                        assert!(
                            cache
                                .library
                                .join("reciter-00-surah-112/surah.json")
                                .exists()
                        );
                    }
                    Event::Complete { folder, .. } => {
                        committed = Some(folder);
                        break;
                    }
                    Event::Failed { message, .. } => panic!("{message}"),
                    _ => {}
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        let saved = committed.expect("Whole surah transaction did not finish");
        assert!(reached_100);
        let manifest: SurahManifest =
            serde_json::from_slice(&fs::read(saved.join("surah.json")).unwrap()).unwrap();
        assert_eq!(manifest.clips.len(), 5);
        assert_eq!(manifest.clips[0].key, ClipKey::new(0, 1, 1).unwrap());
        assert_eq!(
            manifest
                .clips
                .iter()
                .skip(1)
                .map(|r| r.key.ayah)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        let mut bad = manifest.clips[1].clone();
        bad.file = "../../outside.mp3".into();
        assert!(validate_receipt(&bad).is_err());
        for entry in fs::read_dir(&cache.clips).unwrap() {
            fs::remove_file(entry.unwrap().path()).unwrap();
        }
        let (_, info) = cache
            .get_clip(ClipKey::new(0, 112, 2).unwrap(), &state(), |_, _| {})
            .unwrap();
        assert_eq!(info.frames, 8);
        assert_eq!(transport.calls.load(Ordering::Relaxed), 5);
        let (_, basmala) = cache
            .get_clip(ClipKey::new(0, 1, 1).unwrap(), &state(), |_, _| {})
            .unwrap();
        assert_eq!(basmala.frames, 8);
        assert_eq!(transport.calls.load(Ordering::Relaxed), 5);
    }
    #[test]
    fn interrupted_surah_never_appears_as_a_finished_download() {
        let folder = TestFolder::new();
        let transport = Responses::new(vec![
            Ok(mp3::fixture(8)),
            Err("provider unavailable"),
            Err("provider unavailable"),
            Err("provider unavailable"),
        ]);
        let cache = AudioCache::with_transport(folder.0.clone(), transport).unwrap();
        let job = cache.download_surah(0, 112, 77).unwrap();
        let begin = Instant::now();
        let mut failed = false;
        while begin.elapsed() < Duration::from_secs(5) {
            if let Some(event) = job.try_recv() {
                match event {
                    Event::Failed { .. } => {
                        failed = true;
                        break;
                    }
                    Event::Progress { percent: 100, .. } | Event::Complete { .. } => {
                        panic!("Incomplete surah was marked complete")
                    }
                    _ => {}
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(failed);
        assert!(!cache.library.join("reciter-00-surah-112").exists());
        assert_eq!(fs::read_dir(&cache.library).unwrap().count(), 0);
    }
    #[test]
    fn startup_temp_cleanup_requires_exact_owned_hashes_and_names() {
        let folder = TestFolder::new();
        let transport = Responses::new(vec![]);
        let cache = AudioCache::with_transport(folder.0.clone(), transport).unwrap();
        let key = ClipKey::new(0, 112, 1).unwrap();
        let bytes = mp3::fixture(8);
        let digest = sha256(&bytes).unwrap();
        let receipt = Receipt {
            schema: "native-audio-clip-v1".into(),
            key,
            file: format!("{}_{digest}.mp3", key.prefix()),
            sha256: digest,
            bytes: bytes.len() as u64,
            frames: 8,
        };
        let manifest = SurahManifest {
            schema: "native-audio-partial-v1".into(),
            reciter: 0,
            surah: 112,
            clips: vec![receipt.clone()],
        };
        let owned = cache.library.join(".reciter-00-surah-112-888-1.partial");
        let modified = cache.library.join(".reciter-00-surah-112-888-2.partial");
        for destination in [&owned, &modified] {
            fs::create_dir(destination).unwrap();
            write_new(&destination.join(&receipt.file), &bytes).unwrap();
            write_new(
                &destination.join("transaction.json"),
                &serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        }
        fs::write(modified.join(&receipt.file), b"User changed this file").unwrap();
        cache.load_index().unwrap();
        assert!(!owned.exists());
        assert!(modified.exists());
        assert_eq!(
            fs::read(modified.join(&receipt.file)).unwrap(),
            b"User changed this file"
        );
    }
}
