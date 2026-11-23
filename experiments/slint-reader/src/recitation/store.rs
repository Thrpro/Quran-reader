//! Recitation files in the profile's AudioStore, in exactly the native
//! engine's layout and record formats, so downloads made by either engine
//! play in the other:
//!
//! - `Cache/`: recently played ayah clips (`r00-s002-a255.json` records and
//!   `r00-s002-a255_<sha256>.mp3`), at most 32 MiB.
//! - `Downloads/reciter-00-surah-002/`: a surah saved for offline listening,
//!   `surah.json` naming every clip with its size and SHA-256.
//! - `Continuous/r92-s002/`: a whole-surah recording with its publisher's ayah
//!   timings (`audio.mp3`, `timings.json`, `catalogue.json`, `receipt.json`).
//! - `Streaming/`: this engine's temporary copies of recordings being played
//!   before they are downloaded; removed when playback ends.
use super::net;
use crate::{
    audio::RECITERS,
    data::Result,
    mp3,
    timed_audio::{Recording, Timeline},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

const CACHE_QUOTA: u64 = 32 * 1024 * 1024;
const CLIP_LIMIT: u64 = 16 * 1024 * 1024;
const RECORD_LIMIT: u64 = 256 * 1024;
static NONCE: AtomicU64 = AtomicU64::new(1);

/// Ayahs per surah (Hafs).
pub const COUNTS: [u32; 114] = [
    7, 286, 200, 176, 120, 165, 206, 75, 129, 109, 123, 111, 43, 52, 99, 128, 111, 110, 98, 135,
    112, 78, 118, 64, 77, 227, 93, 88, 69, 60, 34, 30, 73, 54, 45, 83, 182, 88, 75, 85, 54, 53, 89,
    59, 37, 35, 38, 29, 18, 45, 60, 49, 62, 55, 78, 96, 29, 22, 24, 13, 14, 11, 11, 18, 12, 12, 30,
    52, 52, 44, 28, 28, 20, 56, 40, 31, 50, 40, 46, 42, 29, 19, 36, 25, 22, 17, 19, 26, 30, 20, 15,
    21, 11, 8, 8, 19, 5, 8, 8, 11, 11, 8, 3, 9, 5, 4, 7, 3, 6, 3, 5, 4, 5, 6,
];

fn nonce() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClipKey {
    pub reciter: usize,
    pub surah: u32,
    pub ayah: u32,
}
impl ClipKey {
    pub fn new(reciter: usize, surah: u32, ayah: u32) -> Result<Self> {
        if reciter >= RECITERS.len()
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
    /// The provider address, as the native engine builds it.
    pub fn url(self) -> String {
        let r = &RECITERS[self.reciter];
        if r.everyayah {
            format!(
                "https://everyayah.com/data/{}/{:03}{:03}.mp3",
                r.path, self.surah, self.ayah
            )
        } else {
            format!(
                "https://verses.quran.foundation/{}/mp3/{:03}{:03}.mp3",
                r.path, self.surah, self.ayah
            )
        }
    }
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
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordingReceipt {
    schema: String,
    read: u32,
    surah: u32,
    bytes: u64,
    sha256: String,
    timings_sha256: String,
    catalogue_sha256: String,
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn sha256_file(path: &Path, cancelled: &AtomicBool) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Cancelled".into());
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn read_small(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(RECORD_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Err("Recitation record is too large".into());
    }
    Ok(bytes)
}

/// Writes a file whole or not at all: a temporary file, then a rename.
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.partial", nonce()));
    let result = (|| -> Result<()> {
        let mut file = File::create_new(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })()
    .and_then(|()| Ok(fs::rename(&temporary, path)?));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// A plain file name inside its folder: no paths, links or parent references.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && !name.contains("..")
}

/// A verified clip ready to play.
#[derive(Clone, Debug)]
pub struct Clip {
    pub path: PathBuf,
    pub duration_ms: u64,
}

pub struct AudioCache {
    clips: PathBuf,
    library: PathBuf,
    continuous: PathBuf,
    streaming: PathBuf,
    /// Serialises writes to the clip cache between playback and downloads.
    cache_lock: Mutex<()>,
}

impl AudioCache {
    pub fn new(profile_root: PathBuf) -> Result<Arc<Self>> {
        let root = profile_root.join("AudioStore");
        let cache = Self {
            clips: root.join("Cache"),
            library: root.join("Downloads"),
            continuous: root.join("Continuous"),
            streaming: root.join("Streaming"),
            cache_lock: Mutex::new(()),
        };
        for folder in [
            &cache.clips,
            &cache.library,
            &cache.continuous,
            &cache.streaming,
        ] {
            fs::create_dir_all(folder)?;
        }
        // Temporary recordings left by an earlier run that ended abruptly.
        if let Ok(entries) = fs::read_dir(&cache.streaming) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_file()) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        Ok(Arc::new(cache))
    }

    /// The clip for `key`: from a saved surah, else the cache, else the
    /// provider (then kept in the cache). Every file's SHA-256 is checked.
    pub fn clip(&self, key: ClipKey, cancelled: &AtomicBool) -> Result<Clip> {
        if let Some(clip) = self.saved_clip(key) {
            return Ok(clip);
        }
        if let Some(clip) = self.cached_clip(key) {
            return Ok(clip);
        }
        let bytes = net::with_retries(cancelled, || {
            net::fetch_bytes(&key.url(), CLIP_LIMIT, true, cancelled)
        })?;
        let info = mp3::inspect(&bytes)?;
        let digest = sha256(&bytes);
        let receipt = Receipt {
            schema: "native-audio-clip-v1".into(),
            key,
            file: format!("{}_{digest}.mp3", key.prefix()),
            bytes: bytes.len() as u64,
            frames: info.frames,
            sha256: digest,
        };
        let _guard = self.cache_lock.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.clips.join(&receipt.file);
        if !path.is_file() {
            write_new(&path, &bytes)?;
        }
        let record = self.clips.join(format!("{}.json", key.prefix()));
        let _ = fs::remove_file(&record);
        write_new(&record, &serde_json::to_vec(&receipt)?)?;
        self.evict(key);
        Ok(Clip {
            path,
            duration_ms: (info.duration_seconds * 1000.).round() as u64,
        })
    }

    fn verified(folder: &Path, receipt: &Receipt, key: ClipKey) -> Option<Clip> {
        if receipt.schema != "native-audio-clip-v1"
            || receipt.key != key
            || !plain_name(&receipt.file)
        {
            return None;
        }
        let path = folder.join(&receipt.file);
        let bytes = fs::read(&path).ok()?;
        if bytes.len() as u64 != receipt.bytes || sha256(&bytes) != receipt.sha256 {
            return None;
        }
        let info = mp3::inspect(&bytes).ok()?;
        Some(Clip {
            path,
            duration_ms: (info.duration_seconds * 1000.).round() as u64,
        })
    }

    fn manifest(folder: &Path) -> Option<SurahManifest> {
        let manifest: SurahManifest =
            serde_json::from_slice(&read_small(&folder.join("surah.json")).ok()?).ok()?;
        (manifest.schema == "native-audio-surah-v1").then_some(manifest)
    }

    /// From the surah's own download, or (for the opening basmala, saved with
    /// every surah) from any of the reciter's downloads.
    fn saved_clip(&self, key: ClipKey) -> Option<Clip> {
        let own = self
            .library
            .join(format!("reciter-{:02}-surah-{:03}", key.reciter, key.surah));
        let mut folders = vec![own];
        if key.surah == 1 && key.ayah == 1 {
            let prefix = format!("reciter-{:02}-surah-", key.reciter);
            if let Ok(entries) = fs::read_dir(&self.library) {
                folders.extend(
                    entries
                        .flatten()
                        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                        .map(|e| e.path()),
                );
            }
        }
        folders.into_iter().find_map(|folder| {
            let manifest = Self::manifest(&folder)?;
            if manifest.reciter != key.reciter {
                return None;
            }
            let receipt = manifest.clips.iter().find(|r| r.key == key)?;
            Self::verified(&folder, receipt, key)
        })
    }

    fn cached_clip(&self, key: ClipKey) -> Option<Clip> {
        let record = self.clips.join(format!("{}.json", key.prefix()));
        let receipt: Receipt = serde_json::from_slice(&read_small(&record).ok()?).ok()?;
        let clip = Self::verified(&self.clips, &receipt, key)?;
        // Most recently used stays longest.
        let _ = File::options()
            .append(true)
            .open(&record)
            .and_then(|f| f.set_modified(std::time::SystemTime::now()));
        Some(clip)
    }

    /// Keeps the clip cache under its quota, oldest first, never `keep`.
    fn evict(&self, keep: ClipKey) {
        let Ok(entries) = fs::read_dir(&self.clips) else {
            return;
        };
        let mut records = Vec::new();
        let mut total = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !(name.starts_with('r') && name.ends_with(".json")) {
                continue;
            }
            let Some(receipt) = read_small(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Receipt>(&b).ok())
            else {
                continue;
            };
            let modified = entry.metadata().and_then(|m| m.modified()).ok();
            total += receipt.bytes;
            records.push((modified, path, receipt));
        }
        records.sort_by_key(|(modified, ..)| *modified);
        for (_, record, receipt) in records {
            if total <= CACHE_QUOTA {
                break;
            }
            if receipt.key == keep || !plain_name(&receipt.file) {
                continue;
            }
            let _ = fs::remove_file(&record);
            let _ = fs::remove_file(self.clips.join(&receipt.file));
            total = total.saturating_sub(receipt.bytes);
        }
    }

    /// Saves a whole surah's clips (with the opening basmala, as the native
    /// engine does) into Downloads for offline listening.
    pub fn download_surah(
        self: &Arc<Self>,
        reciter: usize,
        surah: u32,
        generation: u64,
    ) -> Result<Job<Event>> {
        ClipKey::new(reciter, surah, 1)?;
        let mut keys = Vec::new();
        if surah != 1 && surah != 9 {
            keys.push(ClipKey::new(reciter, 1, 1)?);
        }
        for ayah in 1..=COUNTS[(surah - 1) as usize] {
            keys.push(ClipKey::new(reciter, surah, ayah)?);
        }
        let (job, sender, cancelled) = Job::new();
        let cache = self.clone();
        thread::Builder::new()
            .name("surah-download".into())
            .spawn(move || {
                let event = match cache
                    .save_surah(reciter, surah, &keys, generation, &sender, &cancelled)
                {
                    Ok(folder) => Event::Complete { generation, folder },
                    Err(_) if cancelled.load(Ordering::Acquire) => Event::Cancelled { generation },
                    Err(error) => Event::Failed {
                        generation,
                        message: error.to_string(),
                    },
                };
                let _ = sender.send(event);
            })?;
        Ok(job)
    }

    fn save_surah(
        &self,
        reciter: usize,
        surah: u32,
        keys: &[ClipKey],
        generation: u64,
        events: &mpsc::Sender<Event>,
        cancelled: &AtomicBool,
    ) -> Result<PathBuf> {
        let name = format!("reciter-{reciter:02}-surah-{surah:03}");
        let target = self.library.join(&name);
        if Self::manifest(&target).is_some() {
            return Ok(target);
        }
        let staging = self.library.join(format!(".{name}-{}.partial", nonce()));
        fs::create_dir(&staging)?;
        let result = (|| -> Result<()> {
            let mut receipts = Vec::with_capacity(keys.len());
            let mut bytes_total = 0;
            for (index, &key) in keys.iter().enumerate() {
                if cancelled.load(Ordering::Acquire) {
                    return Err("Download cancelled".into());
                }
                let clip = self.clip(key, cancelled)?;
                let bytes = fs::read(&clip.path)?;
                let info = mp3::inspect(&bytes)?;
                let digest = sha256(&bytes);
                let file = format!("{}_{digest}.mp3", key.prefix());
                write_new(&staging.join(&file), &bytes)?;
                bytes_total += bytes.len() as u64;
                receipts.push(Receipt {
                    schema: "native-audio-clip-v1".into(),
                    key,
                    file,
                    bytes: bytes.len() as u64,
                    frames: info.frames,
                    sha256: digest,
                });
                let _ = events.send(Event::Progress {
                    generation,
                    completed: index + 1,
                    total: keys.len(),
                    percent: ((index + 1) * 100 / keys.len()).min(99) as u32,
                    bytes: bytes_total,
                });
            }
            let manifest = SurahManifest {
                schema: "native-audio-surah-v1".into(),
                reciter,
                surah,
                clips: receipts,
            };
            write_new(&staging.join("surah.json"), &serde_json::to_vec(&manifest)?)?;
            if target.exists() {
                return Err("This surah's download folder already exists".into());
            }
            fs::rename(&staging, &target)?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        Ok(target)
    }

    pub fn timed_store(self: &Arc<Self>) -> Result<Arc<TimedStore>> {
        Ok(Arc::new(TimedStore {
            cache: self.clone(),
        }))
    }

    /// A saved whole-surah recording, checked against its receipt.
    pub fn saved_recording(
        &self,
        reciter: usize,
        surah: u32,
        cancelled: &AtomicBool,
    ) -> Option<(PathBuf, Timeline, u64)> {
        let recording = Recording::for_reciter(reciter)?;
        let folder = self
            .continuous
            .join(format!("r{}-s{surah:03}", recording.read));
        let receipt: RecordingReceipt =
            serde_json::from_slice(&read_small(&folder.join("receipt.json")).ok()?).ok()?;
        if receipt.schema != "native-continuous-v1"
            || receipt.read != recording.read
            || receipt.surah != surah
        {
            return None;
        }
        let catalogue = read_small(&folder.join("catalogue.json")).ok()?;
        let timings = fs::read(folder.join("timings.json")).ok()?;
        if sha256(&catalogue) != receipt.catalogue_sha256
            || sha256(&timings) != receipt.timings_sha256
            || recording.verify_catalog(&catalogue).is_err()
        {
            return None;
        }
        let timeline = Timeline::parse(&timings, COUNTS[(surah - 1) as usize]).ok()?;
        let audio = folder.join("audio.mp3");
        if fs::metadata(&audio).ok()?.len() != receipt.bytes
            || sha256_file(&audio, cancelled).ok()? != receipt.sha256
        {
            return None;
        }
        let mut file = File::open(&audio).ok()?;
        let info = mp3::inspect_reader(&mut file, receipt.bytes, mp3::MAX_CHAPTER_BYTES).ok()?;
        Some((
            audio,
            timeline,
            (info.duration_seconds * 1000.).round() as u64,
        ))
    }

    /// The publisher's timing table for a recording, checked against its catalogue.
    pub fn fetch_timings(
        &self,
        recording: &Recording,
        surah: u32,
        cancelled: &AtomicBool,
    ) -> Result<(Vec<u8>, Vec<u8>, Timeline)> {
        let catalogue = net::with_retries(cancelled, || {
            net::fetch_bytes(
                "https://www.mp3quran.net/api/v3/ayat_timing/reads",
                1024 * 1024,
                false,
                cancelled,
            )
        })?;
        recording.verify_catalog(&catalogue)?;
        let timings = net::with_retries(cancelled, || {
            net::fetch_bytes(
                &format!(
                    "https://www.mp3quran.net/api/v3/ayat_timing?surah={surah}&read={}",
                    recording.read
                ),
                1024 * 1024,
                false,
                cancelled,
            )
        })?;
        let timeline = Timeline::parse(&timings, COUNTS[(surah - 1) as usize])?;
        Ok((catalogue, timings, timeline))
    }

    /// A fresh temporary file for a recording being streamed.
    pub fn streaming_file(&self, read: u32, surah: u32) -> PathBuf {
        self.streaming
            .join(format!("r{read}-s{surah:03}-{}.mp3", nonce()))
    }
}

/// Whole-surah recordings saved for offline listening.
pub struct TimedStore {
    cache: Arc<AudioCache>,
}

// The same events as the native engine's; the bridge reads only some fields.
#[allow(dead_code)]
pub struct Ready {
    pub generation: u64,
    pub folder: PathBuf,
}

pub enum RecordingEvent {
    Progress(u64, Option<u64>),
    Ready(Ready),
    Failed(u64, String),
}

impl TimedStore {
    pub fn prepare(
        &self,
        reciter: usize,
        surah: u32,
        generation: u64,
    ) -> Result<Job<RecordingEvent>> {
        let recording = Recording::for_reciter(reciter).ok_or("Unknown recording")?;
        if !(1..=114).contains(&surah) {
            return Err("Invalid chapter".into());
        }
        let (job, sender, cancelled) = Job::new();
        let cache = self.cache.clone();
        thread::Builder::new()
            .name("recording-download".into())
            .spawn(move || {
                let event = match save_recording(&cache, &recording, surah, &sender, &cancelled) {
                    Ok(folder) => RecordingEvent::Ready(Ready { generation, folder }),
                    Err(error) => RecordingEvent::Failed(generation, error.to_string()),
                };
                let _ = sender.send(event);
            })?;
        Ok(job)
    }
}

fn save_recording(
    cache: &AudioCache,
    recording: &Recording,
    surah: u32,
    events: &mpsc::Sender<RecordingEvent>,
    cancelled: &AtomicBool,
) -> Result<PathBuf> {
    let name = format!("r{}-s{surah:03}", recording.read);
    let target = cache.continuous.join(&name);
    if target.join("receipt.json").is_file() {
        return Ok(target);
    }
    let staging = cache
        .continuous
        .join(format!(".{name}-{}.partial", nonce()));
    fs::create_dir(&staging)?;
    let result = (|| -> Result<()> {
        let (catalogue, timings, timeline) = cache.fetch_timings(recording, surah, cancelled)?;
        write_new(&staging.join("catalogue.json"), &catalogue)?;
        write_new(&staging.join("timings.json"), &timings)?;
        let audio = staging.join("audio.mp3");
        net::with_retries(cancelled, || {
            let mut file = File::create(&audio)?;
            net::fetch(
                &recording.audio_url(surah)?,
                &mut file,
                mp3::MAX_CHAPTER_BYTES,
                true,
                cancelled,
                |bytes, total| {
                    let _ = events.send(RecordingEvent::Progress(bytes, total));
                },
            )?;
            file.sync_all()?;
            Ok(())
        })?;
        let bytes = fs::metadata(&audio)?.len();
        let info = mp3::inspect_reader(&mut File::open(&audio)?, bytes, mp3::MAX_CHAPTER_BYTES)?;
        timeline.validate_duration((info.duration_seconds * 1000.).round() as u64)?;
        let receipt = RecordingReceipt {
            schema: "native-continuous-v1".into(),
            read: recording.read,
            surah,
            bytes: info.bytes,
            sha256: sha256_file(&audio, cancelled)?,
            timings_sha256: sha256(&timings),
            catalogue_sha256: sha256(&catalogue),
        };
        write_new(
            &staging.join("receipt.json"),
            &serde_json::to_vec(&receipt)?,
        )?;
        if target.exists() {
            return Err("This recording's download folder already exists".into());
        }
        fs::rename(&staging, &target)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    Ok(target)
}

#[allow(dead_code)]
pub enum Event {
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

/// A background download; dropping it cancels the work.
pub struct Job<E> {
    receiver: mpsc::Receiver<E>,
    cancelled: Arc<AtomicBool>,
}
impl<E> Job<E> {
    fn new() -> (Self, mpsc::Sender<E>, Arc<AtomicBool>) {
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        (
            Self {
                receiver,
                cancelled: cancelled.clone(),
            },
            sender,
            cancelled,
        )
    }
    pub fn try_recv(&self) -> Option<E> {
        self.receiver.try_recv().ok()
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl<E> Drop for Job<E> {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_names_match_the_native_engine() {
        let key = ClipKey::new(0, 2, 255).unwrap();
        assert_eq!(key.prefix(), "r00-s002-a255");
        assert_eq!(
            key.url(),
            "https://everyayah.com/data/Yasser_Ad-Dussary_128kbps/002255.mp3"
        );
        assert!(net::allowed(&key.url()));
        assert!(ClipKey::new(0, 1, 8).is_err());
        assert!(ClipKey::new(0, 115, 1).is_err());
        assert!(ClipKey::new(RECITERS.len(), 1, 1).is_err());
        for (reciter, r) in RECITERS.iter().enumerate() {
            assert!(
                net::allowed(&ClipKey::new(reciter, 114, 6).unwrap().url()),
                "{}",
                r.path
            );
        }
    }

    #[test]
    fn receipts_keep_the_native_format() {
        let receipt = Receipt {
            schema: "native-audio-clip-v1".into(),
            key: ClipKey::new(1, 112, 1).unwrap(),
            file: "r01-s112-a001_ab.mp3".into(),
            bytes: 10,
            frames: 3,
            sha256: "ab".into(),
        };
        assert_eq!(
            serde_json::to_string(&receipt).unwrap(),
            r#"{"schema":"native-audio-clip-v1","key":{"reciter":1,"surah":112,"ayah":1},"file":"r01-s112-a001_ab.mp3","bytes":10,"frames":3,"sha256":"ab"}"#
        );
    }

    #[test]
    fn hashes_match_the_standard() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
