//! One cancellable worker publishes only complete original recording bundles.
//! Incomplete/unknown stages are preserved and count against the bounded quota.
use crate::{
    audio_cache::{self, COUNTS},
    data::Result,
    file_integrity::{self, VerifiedFile},
    mp3,
    timed_audio::{Recording, Timeline},
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::Networking::WinHttp::*,
    core::{PCWSTR, w},
};
const QUOTA: u64 = 512 * 1024 * 1024;
static NONCE: AtomicU64 = AtomicU64::new(1);
pub struct Ready {
    pub generation: u64,
    pub surah: u32,
    pub timeline: Timeline,
    pub path: PathBuf,
    pub _verified: VerifiedFile,
    // Releasing a recording never hashes or deletes files on the UI thread.
    pub(crate) _scoped: Option<crate::scoped_store::Lease>,
}
pub enum Event {
    Ready(Box<Ready>),
    Progress(u64, Option<u64>),
    Failed(u64, String),
}
pub enum StreamReady {
    Local(Box<Ready>),
    Remote {
        generation: u64,
        surah: u32,
        timeline: Timeline,
        source: Arc<crate::range_stream::Source>,
    },
}
pub struct StreamJob {
    cancelled: Arc<AtomicBool>,
    events: mpsc::Receiver<Result<StreamReady>>,
}
impl StreamJob {
    pub fn try_recv(&self) -> Option<Result<StreamReady>> {
        self.events.try_recv().ok()
    }
}
impl Drop for StreamJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
/// Automatic online playback never publishes a full chapter to disk. Existing
/// verified downloads remain read-only and are preferred without network I/O.
struct StreamTask {
    root: PathBuf,
    reciter: usize,
    surah: u32,
    generation: u64,
    cancelled: Arc<AtomicBool>,
    sender: mpsc::SyncSender<Result<StreamReady>>,
}
static STREAM_WORKERS: std::sync::Mutex<Option<mpsc::SyncSender<StreamTask>>> =
    std::sync::Mutex::new(None);
pub fn prepare_stream(
    root: PathBuf,
    reciter: usize,
    surah: u32,
    generation: u64,
) -> Result<StreamJob> {
    if Recording::for_reciter(reciter).is_none() || !(1..=114).contains(&surah) {
        return Err("Invalid streaming request".into());
    }
    let mut workers = STREAM_WORKERS.lock().unwrap_or_else(|e| e.into_inner());
    if workers.is_none() {
        let (queue, receiver) = mpsc::sync_channel::<StreamTask>(2);
        let receiver = Arc::new(std::sync::Mutex::new(receiver));
        for n in 0..2 {
            let receiver = receiver.clone();
            std::thread::Builder::new()
                .name(format!("recitation-stream-open-{n}"))
                .stack_size(512 * 1024)
                .spawn(move || {
                    loop {
                        let task = receiver.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        let Ok(task) = task else {
                            break;
                        };
                        if !task.cancelled.load(Ordering::Acquire) {
                            open_stream(task);
                        }
                    }
                })?;
        }
        *workers = Some(queue);
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let (sender, events) = mpsc::sync_channel(1);
    workers
        .as_ref()
        .unwrap()
        .try_send(StreamTask {
            root,
            reciter,
            surah,
            generation,
            cancelled: cancelled.clone(),
            sender,
        })
        .map_err(|_| "Audio preparation is busy; retry shortly")?;
    Ok(StreamJob { cancelled, events })
}
fn open_stream(task: StreamTask) {
    let StreamTask {
        root,
        reciter,
        surah,
        generation,
        cancelled,
        sender,
    } = task;
    let recording = Recording::for_reciter(reciter).unwrap();
    let result = (|| {
        let (events, _receiver) = event_channel();
        let task = Task {
            reciter,
            surah,
            generation,
            cancelled: cancelled.clone(),
            events,
        };
        let folder = root
            .join("Continuous")
            .join(format!("r{}-s{surah:03}", recording.read));
        if folder.exists() {
            return Ok(StreamReady::Local(Box::new(validate(&folder, &task)?)));
        }
        let mut catalogue = Vec::new();
        fetch(
            "https://www.mp3quran.net/api/v3/ayat_timing/reads",
            &mut catalogue,
            1024 * 1024,
            false,
            &task,
        )?;
        recording.verify_catalog(&catalogue)?;
        let mut raw = Vec::new();
        fetch(
            &format!(
                "https://www.mp3quran.net/api/v3/ayat_timing?surah={surah}&read={}",
                recording.read
            ),
            &mut raw,
            1024 * 1024,
            false,
            &task,
        )?;
        let timeline = Timeline::parse(&raw, COUNTS[(surah - 1) as usize])?;
        let source =
            crate::range_stream::Source::open(recording.audio_url(surah)?, cancelled.clone())?;
        check(&cancelled)?;
        Ok(StreamReady::Remote {
            generation,
            surah,
            timeline,
            source,
        })
    })();
    let _ = sender.send(result);
}
fn event_channel() -> (mpsc::SyncSender<Event>, mpsc::Receiver<Event>) {
    mpsc::sync_channel(4)
}
pub struct Job {
    cancelled: Arc<AtomicBool>,
    events: mpsc::Receiver<Event>,
}
#[cfg(test)]
mod notification_tests {
    use super::*;
    #[test]
    fn progress_is_bounded_and_terminal_failure_is_not_lost() {
        let (sender, receiver) = event_channel();
        for n in 0..4 {
            sender.try_send(Event::Progress(n, None)).unwrap();
        }
        assert!(matches!(
            sender.try_send(Event::Progress(5, None)),
            Err(mpsc::TrySendError::Full(_))
        ));
        let worker = std::thread::spawn(move || {
            sender
                .send(Event::Failed(7, "provider failed".into()))
                .is_ok()
        });
        for _ in 0..4 {
            assert!(matches!(receiver.recv().unwrap(), Event::Progress(_, _)));
        }
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            Event::Failed(7, _)
        ));
        assert!(worker.join().unwrap());
    }
    #[test]
    fn cancelling_job_releases_a_blocked_notification_sender() {
        let (sender, receiver) = event_channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let job = Job {
            cancelled: cancelled.clone(),
            events: receiver,
        };
        for n in 0..4 {
            sender.try_send(Event::Progress(n, None)).unwrap();
        }
        let worker = std::thread::spawn(move || {
            sender
                .send(Event::Failed(7, "provider failed".into()))
                .is_ok()
        });
        drop(job);
        assert!(cancelled.load(Ordering::Acquire));
        assert!(!worker.join().unwrap());
    }
}
impl Job {
    pub fn try_recv(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
pub(crate) struct Task {
    pub(crate) reciter: usize,
    pub(crate) surah: u32,
    pub(crate) generation: u64,
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) events: mpsc::SyncSender<Event>,
}
pub struct Store {
    sender: mpsc::SyncSender<Task>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: String,
    read: u32,
    surah: u32,
    bytes: u64,
    sha256: String,
    timings_sha256: String,
    catalogue_sha256: String,
}
fn check(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err("Recording preparation cancelled".into())
    } else {
        Ok(())
    }
}

#[cfg(feature = "qa")]
pub fn scoped_smoke(out: &Path) -> Result<()> {
    use serde_json::json;
    let report = out.join("scoped-store-verification.json");
    if report.exists() {
        return Err("Preserve the scoped verification receipt; use a fresh output".into());
    }
    let root = out.join("profile").join("AudioStore");
    audio_cache::safe_existing_directory(&root)?;
    let borrowed = root.join("Continuous").join("r92-s112");
    audio_cache::safe_existing_directory(&borrowed)?;
    let original = scoped_fingerprints(&borrowed)?;
    let started = Instant::now();
    let store = Store::new_scoped(&root)?;
    let mut rows = Vec::new();
    let mut high_water = 0u64;
    let first = scoped_await(&store, 112, 201)?;
    if first
        .path
        .components()
        .any(|part| part.as_os_str() == "ScopedContinuous")
    {
        return Err("Original persistent chapter was not borrowed read-only".into());
    }
    rows.push(scoped_row(&first, "borrowed", 0.));
    let mut held = Vec::new();
    for (surah, generation) in [(113, 202), (114, 203)] {
        let began = Instant::now();
        let ready = scoped_await(&store, surah, generation)?;
        if !ready
            .path
            .components()
            .any(|part| part.as_os_str() == "ScopedContinuous")
        {
            return Err(
                "Fixture must seed only persistent chapter112; missing chapters must be owned"
                    .into(),
            );
        }
        rows.push(scoped_row(&ready, "owned", began.elapsed().as_secs_f64()));
        high_water = high_water.max(crate::scoped_store::qa_usage(&root)?.0);
        held.push(ready);
    }
    let old_path = held[0].path.clone();
    std::thread::sleep(Duration::from_millis(400));
    if !old_path.exists() {
        return Err("An active original recording lease was retired".into());
    }
    let began = Instant::now();
    drop(held.remove(0));
    scoped_wait_gone(&old_path, Duration::from_secs(20))?;
    let retirement_seconds = began.elapsed().as_secs_f64();
    let other_path = held[0].path.clone();
    drop(held);
    scoped_wait_gone(&other_path, Duration::from_secs(20))?;
    drop(first);
    // Audio may retain Store for another listening mode. An idle Store must
    // release its session and ancestry pins without requiring Store itself to
    // be destroyed; the next preparation recreates a safe session on worker.
    scoped_wait_roots(&root, 0, Duration::from_secs(30))?;
    {
        use std::os::windows::fs::OpenOptionsExt;
        let _idle_directory = OpenOptions::new()
            .access_mode(0x4000_0000)
            .share_mode(7)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(&root)?;
    }
    let repeated = scoped_await(&store, 113, 204)?;
    rows.push(scoped_row(&repeated, "owned-repeat", 0.));
    if repeated.path == old_path {
        return Err("Retired original chapter was not prepared in a fresh owned bundle".into());
    }
    let repeat_path = repeated.path.clone();
    drop(repeated);
    scoped_wait_gone(&repeat_path, Duration::from_secs(20))?;
    scoped_wait_roots(&root, 0, Duration::from_secs(30))?;
    let cancelled = store.prepare(0, 114, 205)?;
    drop(cancelled);
    drop(store);
    scoped_wait_roots(&root, 0, Duration::from_secs(30))?;
    // Reproduce interruption after a sealed publication without cleanup. This
    // uses the same fixed-provider pipeline, not fabricated recording receipts.
    let (events, _receiver) = event_channel();
    let task = Task {
        reciter: 0,
        surah: 113,
        generation: 206,
        cancelled: Arc::new(AtomicBool::new(false)),
        events,
    };
    let mut interrupted = crate::scoped_store::Session::new(&root, Some(&task))?;
    let ready = interrupted.prepare(&task)?;
    let recovery_path = ready.path.clone();
    rows.push(scoped_row(&ready, "sealed-before-interruption", 0.));
    drop(ready);
    drop(interrupted); // Simulated process interruption: locks release, files remain.
    if !recovery_path.exists() {
        return Err("Interrupted owned bundle was not retained for recovery".into());
    }
    let recovering = Store::new_scoped(&root)?;
    let ready = scoped_await(&recovering, 112, 207)?;
    scoped_wait_gone(&recovery_path, Duration::from_secs(20))?;
    drop(ready);
    drop(recovering);
    scoped_wait_roots(&root, 0, Duration::from_secs(20))?;
    let after = scoped_fingerprints(&borrowed)?;
    if original != after {
        return Err("Persistent original bundle changed during scoped storage QA".into());
    }
    let result = json!({"schema":"native-scoped-store-qa-v1","passed":true,
        "seconds":started.elapsed().as_secs_f64(),"quotaBytes":crate::scoped_store::QUOTA,
        "scopedDiskHighWaterBytes":high_water,"releasedLeaseRetirementSeconds":retirement_seconds,
        "persistentOriginalBefore":original,"persistentOriginalAfter":after,"chapters":rows,
        "checks":["borrowed persistent bundle unchanged","original missing113/114 publisher downloads",
            "active lease retained","released lease retirement","whole-plan repeat fresh original publication",
            "idle session releases ancestry pins while Store remains alive",
            "cancel before preparation","sealed inactive-owner recovery","background close cleanup"],
        "limits":["no complete-Quran decoder claim","chapter preparation remains sequential",
            "unsealed crash stages and unknown/modified files are preserved and consume quota"]});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(report)?;
    file.write_all(&serde_json::to_vec_pretty(&result)?)?;
    file.sync_all()?;
    Ok(())
}
#[cfg(feature = "qa")]
fn scoped_await(store: &Store, surah: u32, generation: u64) -> Result<Ready> {
    let job = store.prepare(0, surah, generation)?;
    let began = Instant::now();
    loop {
        match job.try_recv() {
            Some(Event::Ready(ready)) if ready.generation == generation && ready.surah == surah => {
                return Ok(*ready);
            }
            Some(Event::Failed(value, error)) if value == generation => return Err(error.into()),
            Some(Event::Ready(_)) | Some(Event::Failed(_, _)) => {
                return Err("Scoped fixture received a stale terminal event".into());
            }
            _ => {}
        }
        if began.elapsed() > Duration::from_secs(180) {
            return Err("Scoped original preparation fixture timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(feature = "qa")]
fn scoped_row(ready: &Ready, storage: &str, seconds: f64) -> serde_json::Value {
    serde_json::json!({"surah":ready.surah,"storage":storage,"path":ready.path,
        "bytes":ready._verified.info.bytes,"sha256":ready._verified.sha256,
        "durationSeconds":ready._verified.info.duration_seconds,"preparationSeconds":seconds})
}
#[cfg(feature = "qa")]
fn scoped_fingerprints(
    folder: &Path,
) -> Result<std::collections::BTreeMap<String, serde_json::Value>> {
    let mut hashes = std::collections::BTreeMap::new();
    for name in [
        "audio.mp3",
        "catalogue.json",
        "timings.json",
        "receipt.json",
    ] {
        let (_, bytes, sha256) = crate::owned_file::fingerprint(&folder.join(name))?;
        hashes.insert(
            name.into(),
            serde_json::json!({"bytes":bytes,"sha256":sha256}),
        );
    }
    Ok(hashes)
}
#[cfg(feature = "qa")]
fn scoped_wait_gone(path: &Path, timeout: Duration) -> Result<()> {
    let began = Instant::now();
    while path.exists() {
        if began.elapsed() > timeout {
            return Err(format!("Owned recording did not retire: {}", path.display()).into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}
#[cfg(feature = "qa")]
fn scoped_wait_roots(root: &Path, roots: usize, timeout: Duration) -> Result<()> {
    let began = Instant::now();
    while crate::scoped_store::qa_usage(root)?.1 != roots {
        if began.elapsed() > timeout {
            return Err("Scoped session close did not release its owned namespace".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}
impl Store {
    /// Temporary multi-chapter storage; persistent bundles are borrowed only.
    pub fn new_scoped(audio_root: &Path) -> Result<Self> {
        audio_cache::safe_existing_directory(audio_root)?;
        let audio_root = audio_root.to_owned();
        let (sender, receiver) = mpsc::sync_channel::<Task>(2);
        std::thread::Builder::new()
            .name("scoped-continuous-recitation".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                let mut session: Option<crate::scoped_store::Session> = None;
                loop {
                    match receiver.recv_timeout(Duration::from_millis(250)) {
                        Ok(task) => {
                            let result = (|| {
                                if session.is_none() {
                                    session = Some(crate::scoped_store::Session::new(
                                        &audio_root,
                                        Some(&task),
                                    )?);
                                }
                                session.as_mut().unwrap().prepare(&task)
                            })();
                            match result {
                                Ok(ready) => {
                                    let _ = task.events.send(Event::Ready(Box::new(ready)));
                                }
                                Err(error) if !task.cancelled.load(Ordering::Acquire) => {
                                    let _ = task
                                        .events
                                        .send(Event::Failed(task.generation, error.to_string()));
                                }
                                Err(_) => {}
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            let idle = if let Some(active) = session.as_mut() {
                                let _ = active.retire_released();
                                if !active.has_leases() {
                                    let _ = active.close();
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            if idle {
                                // No fetch runs concurrently with this timeout.
                                // Preserved backlog remains identity-marked for
                                // the next task's bounded recovery; no UI Drop
                                // performs filesystem work.
                                session = None;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            // Ready may outlive Store. Keep the ownership lock and
                            // namespace pins until the final recording lease ends.
                            if let Some(session) = &mut session {
                                while session.has_leases() {
                                    std::thread::sleep(Duration::from_millis(100));
                                }
                                let _ = session.close();
                            }
                            break;
                        }
                    }
                }
            })?;
        Ok(Self { sender })
    }
    pub fn new(audio_root: &Path) -> Result<Self> {
        audio_cache::safe_existing_directory(audio_root)?;
        let root = audio_root.join("Continuous");
        audio_cache::safe_directory(&root)?;
        let (sender, receiver) = mpsc::sync_channel::<Task>(2);
        std::thread::Builder::new()
            .name("continuous-recitation".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                while let Ok(task) = receiver.recv() {
                    let result = prepare(&root, &task);
                    match result {
                        Ok(ready) => {
                            let _ = task.events.send(Event::Ready(Box::new(ready)));
                        }
                        Err(error) => {
                            if !task.cancelled.load(Ordering::Acquire) {
                                let _ = task
                                    .events
                                    .send(Event::Failed(task.generation, error.to_string()));
                            }
                        }
                    }
                }
            })?;
        Ok(Self { sender })
    }
    pub fn prepare(&self, reciter: usize, surah: u32, generation: u64) -> Result<Job> {
        if Recording::for_reciter(reciter).is_none() || !(1..=114).contains(&surah) {
            return Err("This reciter has no verified continuous recording".into());
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let (events, receiver) = event_channel();
        self.sender
            .try_send(Task {
                reciter,
                surah,
                generation,
                cancelled: cancelled.clone(),
                events,
            })
            .map_err(|_| "Continuous recording worker is busy; retry shortly")?;
        Ok(Job {
            cancelled,
            events: receiver,
        })
    }
}
pub(crate) fn validate(folder: &Path, task: &Task) -> Result<Ready> {
    check(&task.cancelled)?;
    audio_cache::safe_existing_directory(folder)?;
    let recording = Recording::for_reciter(task.reciter).ok_or("Unknown recording")?;
    let receipt: Receipt = serde_json::from_slice(&audio_cache::read_bounded(
        &folder.join("receipt.json"),
        8192,
    )?)?;
    if receipt.schema != "native-continuous-v1"
        || receipt.read != recording.read
        || receipt.surah != task.surah
    {
        return Err("Continuous recording receipt does not match its publisher".into());
    }
    let catalogue = audio_cache::read_bounded(&folder.join("catalogue.json"), 1024 * 1024)?;
    let raw = audio_cache::read_bounded(&folder.join("timings.json"), 1024 * 1024)?;
    if audio_cache::sha256(&catalogue)? != receipt.catalogue_sha256
        || audio_cache::sha256(&raw)? != receipt.timings_sha256
    {
        return Err("Stored timing/catalogue bytes changed; preserving them".into());
    }
    recording.verify_catalog(&catalogue)?;
    let timeline = Timeline::parse(&raw, COUNTS[(task.surah - 1) as usize])?;
    let path = folder.join("audio.mp3");
    let verified = file_integrity::inspect_cancellable(&path, Some(&task.cancelled))?;
    if verified.info.bytes != receipt.bytes || verified.sha256 != receipt.sha256 {
        return Err("Stored recording changed; preserving it".into());
    }
    timeline.validate_duration((verified.info.duration_seconds * 1000.).round() as u64)?;
    check(&task.cancelled)?;
    Ok(Ready {
        generation: task.generation,
        surah: task.surah,
        timeline,
        path,
        _verified: verified,
        _scoped: None,
    })
}
fn prepare(root: &Path, task: &Task) -> Result<Ready> {
    check(&task.cancelled)?;
    audio_cache::safe_existing_directory(root)?;
    let recording = Recording::for_reciter(task.reciter).ok_or("Unknown recording")?;
    let destination = root.join(format!("r{}-s{:03}", recording.read, task.surah));
    if destination.exists() {
        return validate(&destination, task);
    }
    if audio_cache::directory_bytes(root)?.saturating_add(mp3::MAX_CHAPTER_BYTES + 3 * 1024 * 1024)
        > QUOTA
    {
        return Err(
            "Continuous recording cache has insufficient space; existing files are preserved"
                .into(),
        );
    }
    let stage = root.join(format!(
        "stage-{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&stage)?;
    download_bundle(&stage, task, |path| {
        Ok(crate::scoped_store::Output::persistent(
            OpenOptions::new().write(true).create_new(true).open(path)?,
        ))
    })?;
    check(&task.cancelled)?;
    if destination.exists() {
        return Err(
            "Continuous recording destination already exists; preserving both copies".into(),
        );
    }
    fs::rename(&stage, &destination)?;
    validate(&destination, task)
}
/// Shared publisher/MP3/timing pipeline. Ownership policy only supplies files.
pub(crate) fn download_bundle(
    stage: &Path,
    task: &Task,
    mut create: impl FnMut(&Path) -> Result<crate::scoped_store::Output>,
) -> Result<()> {
    let recording = Recording::for_reciter(task.reciter).ok_or("Unknown recording")?;
    let mut catalogue = Vec::new();
    fetch(
        "https://www.mp3quran.net/api/v3/ayat_timing/reads",
        &mut catalogue,
        1024 * 1024,
        false,
        task,
    )?;
    recording.verify_catalog(&catalogue)?;
    let mut raw = Vec::new();
    fetch(
        &format!(
            "https://www.mp3quran.net/api/v3/ayat_timing?surah={}&read={}",
            task.surah, recording.read
        ),
        &mut raw,
        1024 * 1024,
        false,
        task,
    )?;
    let timeline = Timeline::parse(&raw, COUNTS[(task.surah - 1) as usize])?;
    for (name, bytes) in [("catalogue.json", &catalogue), ("timings.json", &raw)] {
        let mut file = create(&stage.join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    let mut file = create(&stage.join("audio.mp3"))?;
    fetch(
        &recording.audio_url(task.surah)?,
        &mut file,
        mp3::MAX_CHAPTER_BYTES,
        true,
        task,
    )?;
    file.sync_all()?;
    drop(file);
    let verified =
        file_integrity::inspect_cancellable(&stage.join("audio.mp3"), Some(&task.cancelled))?;
    timeline.validate_duration((verified.info.duration_seconds * 1000.).round() as u64)?;
    let receipt = Receipt {
        schema: "native-continuous-v1".into(),
        read: recording.read,
        surah: task.surah,
        bytes: verified.info.bytes,
        sha256: verified.sha256.clone(),
        timings_sha256: audio_cache::sha256(&raw)?,
        catalogue_sha256: audio_cache::sha256(&catalogue)?,
    };
    drop(verified);
    let mut file = create(&stage.join("receipt.json"))?;
    file.write_all(&serde_json::to_vec(&receipt)?)?;
    file.sync_all()?;
    drop(file);
    check(&task.cancelled)?;
    Ok(())
}
struct InternetHandle(*mut c_void);
impl InternetHandle {
    fn new(value: *mut c_void) -> Result<Self> {
        if value.is_null() {
            Err(windows::core::Error::from_thread().into())
        } else {
            Ok(Self(value))
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
fn fetch(url: &str, output: &mut impl Write, limit: u64, audio: bool, task: &Task) -> Result<()> {
    let suffix = url
        .strip_prefix("https://")
        .ok_or("Only HTTPS recordings are permitted")?;
    let (host, path) = suffix.split_once('/').ok_or("Invalid provider URL")?;
    if !matches!(host, "www.mp3quran.net" | "cdn.mp3quran.net") {
        return Err("Untrusted recording host".into());
    }
    let host: Vec<u16> = host.encode_utf16().chain(Some(0)).collect();
    let path: Vec<u16> = format!("/{path}").encode_utf16().chain(Some(0)).collect();
    check(&task.cancelled)?;
    let started = Instant::now();
    unsafe {
        let session = InternetHandle::new(WinHttpOpen(
            w!("QuranMushafReader-Native/continuous"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 8000, 8000, 8000, 8000)?;
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
        let headers: Vec<u16> = "Accept-Encoding: identity\r\n".encode_utf16().collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0)?;
        check(&task.cancelled)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())?;
        let mut status = 0u32;
        let mut size = 4;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut size,
            std::ptr::null_mut(),
        )?;
        if status != 200 {
            return Err(format!("Continuous recording provider returned HTTP {status}").into());
        }
        let content = audio_cache::header(request.0, WINHTTP_QUERY_CONTENT_TYPE)?;
        if audio && mp3::error_content_type(&content) {
            return Err("Provider returned an error page instead of audio".into());
        }
        let length = audio_cache::header(request.0, WINHTTP_QUERY_CONTENT_LENGTH)?;
        let expected = if length.is_empty() {
            None
        } else {
            Some(length.parse::<u64>()?)
        };
        if expected.is_some_and(|n| n == 0 || n > limit) {
            return Err("Provider response exceeds its limit".into());
        }
        let mut bytes = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        let mut progress = Instant::now();
        loop {
            check(&task.cancelled)?;
            if started.elapsed() > Duration::from_secs(600) {
                return Err("Continuous recording download exceeded its time limit".into());
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
            bytes += u64::from(count);
            if bytes > limit {
                return Err("Provider response grew beyond its limit".into());
            }
            output.write_all(&buffer[..count as usize])?;
            if audio && progress.elapsed() > Duration::from_millis(200) {
                // Progress is replaceable. Keep the UI queue bounded without
                // allowing a paused consumer to block download or cancellation.
                let _ = task.events.try_send(Event::Progress(bytes, expected));
                progress = Instant::now();
            }
        }
        if bytes == 0 || expected.is_some_and(|n| n != bytes) {
            return Err("Incomplete provider response".into());
        }
        check(&task.cancelled)?;
    }
    Ok(())
}
