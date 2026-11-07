//! Optional publisher-provided tafsir, isolated from the bundled Quran data.
//! Complete books are published atomically; network HTML is displayed as text.
use crate::{
    audio_cache::sha256,
    data::{self, Group, Result, Study},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{Networking::WinHttp::*, Storage::FileSystem::MoveFileW},
    core::{PCWSTR, w},
};

const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const CHAPTER_LIMIT: u64 = 32 * 1024 * 1024;
const PACK_LIMIT: u64 = 64 * 1024 * 1024;
const LIBRARY_LIMIT: u64 = 200 * 1024 * 1024;
const MANIFEST_LIMIT: u64 = 64 * 1024;
const JOURNAL_LIMIT: u64 = 64 * 1024;
const STAGE_SCHEMA: &str = "native-tafsir-stage-v1";
const STAGE_JOURNAL: &str = "transaction.json";
const MAX_STAGING_ENTRIES: usize = 256;
const CACHE_LIMIT: usize = 4 * 1024 * 1024;
const REPARSE_POINT: u32 = 0x400;
const SCHEMA: &str = "native-tafsir-pack-v1";
static NONCE: AtomicU64 = AtomicU64::new(1);
// Hafs chapter boundaries, checked against the bundled QUL metadata by tests.
const COUNTS: [u32; 114] = [
    7, 286, 200, 176, 120, 165, 206, 75, 129, 109, 123, 111, 43, 52, 99, 128, 111, 110, 98, 135,
    112, 78, 118, 64, 77, 227, 93, 88, 69, 60, 34, 30, 73, 54, 45, 83, 182, 88, 75, 85, 54, 53, 89,
    59, 37, 35, 38, 29, 18, 45, 60, 49, 62, 55, 78, 96, 29, 22, 24, 13, 14, 11, 11, 18, 12, 12, 30,
    52, 52, 44, 28, 28, 20, 56, 40, 31, 50, 40, 46, 42, 29, 19, 36, 25, 22, 17, 19, 26, 30, 20, 15,
    21, 11, 8, 8, 19, 5, 8, 8, 11, 11, 8, 3, 9, 5, 4, 7, 3, 6, 3, 5, 4, 5, 6,
];

pub struct Source {
    pub id: &'static str,
    pub resource: u32,
    pub name_arabic: &'static str,
    pub name_english: &'static str,
    pub arabic: bool,
}
impl Source {
    pub fn download_issue(&self) -> Option<&'static str> {
        matches!(self.resource, 169 | 168 | 817).then_some("Publisher verse mappings did not pass verification; this English download is temporarily unavailable.")
    }
}
pub const SOURCES: [Source; 7] = [
    Source {
        id: "quran-14",
        resource: 14,
        name_arabic: "تفسير ابن كثير",
        name_english: "Ibn Kathir",
        arabic: true,
    },
    Source {
        id: "quran-15",
        resource: 15,
        name_arabic: "تفسير الطبري",
        name_english: "Al-Tabari",
        arabic: true,
    },
    Source {
        id: "quran-90",
        resource: 90,
        name_arabic: "تفسير القرطبي",
        name_english: "Al-Qurtubi",
        arabic: true,
    },
    Source {
        id: "quran-93",
        resource: 93,
        name_arabic: "التفسير الوسيط للطنطاوي",
        name_english: "Al-Wasit (Tantawi)",
        arabic: true,
    },
    Source {
        id: "quran-169",
        resource: 169,
        name_arabic: "ابن كثير (مختصر، إنجليزي)",
        name_english: "Ibn Kathir (Abridged)",
        arabic: false,
    },
    Source {
        id: "quran-168",
        resource: 168,
        name_arabic: "معارف القرآن (إنجليزي)",
        name_english: "Ma’arif al-Qur’an",
        arabic: false,
    },
    Source {
        id: "quran-817",
        resource: 817,
        name_arabic: "تذكير القرآن (إنجليزي)",
        name_english: "Tazkirul Quran",
        arabic: false,
    },
];
pub enum Event {
    Progress {
        generation: u64,
        chapters: u32,
        total: u32,
        bytes: u64,
        percent: u32,
    },
    Complete {
        generation: u64,
        source: usize,
    },
    Removed {
        generation: u64,
        source: usize,
    },
    Failed {
        generation: u64,
        message: String,
    },
    Cancelled {
        generation: u64,
    },
}
pub struct Job {
    receiver: mpsc::Receiver<Event>,
    state: Arc<JobState>,
}
impl Job {
    pub fn try_recv(&self) -> std::result::Result<Event, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
    pub fn cancel(&self) {
        self.state.cancel.store(true, Ordering::Release);
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[derive(Default)]
struct JobState {
    cancel: AtomicBool,
}
impl JobState {
    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            Err("Tafsir download cancelled".into())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    surah: u32,
    file: String,
    bytes: u64,
    sha256: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    resource: u32,
    verses: u32,
    bytes: u64,
    chapters: Vec<Receipt>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StageReceipt {
    file: String,
    bytes: u64,
    sha256: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StageJournal {
    schema: String,
    stage: String,
    files: Vec<StageReceipt>,
}
struct CachedChapter {
    source: usize,
    surah: u32,
    bytes: usize,
    study: Arc<Study>,
}
#[derive(Default)]
struct Cache {
    bytes: usize,
    chapters: VecDeque<CachedChapter>,
}
impl Cache {
    fn get(&mut self, source: usize, surah: u32) -> Option<Arc<Study>> {
        let index = self
            .chapters
            .iter()
            .position(|e| e.source == source && e.surah == surah)?;
        let entry = self.chapters.remove(index)?;
        let study = entry.study.clone();
        self.chapters.push_back(entry);
        Some(study)
    }
    fn insert(&mut self, source: usize, surah: u32, bytes: usize, study: Arc<Study>) {
        if bytes > CACHE_LIMIT {
            return;
        }
        while self.chapters.len() >= 6 || self.bytes.saturating_add(bytes) > CACHE_LIMIT {
            let Some(entry) = self.chapters.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(entry.bytes);
        }
        self.bytes += bytes;
        self.chapters.push_back(CachedChapter {
            source,
            surah,
            bytes,
            study,
        });
    }
    fn invalidate(&mut self, source: usize) {
        self.chapters.retain(|entry| entry.source != source);
        self.bytes = self.chapters.iter().map(|entry| entry.bytes).sum();
    }
}
pub struct Library {
    pub root: PathBuf,
    transport: Arc<dyn Transport>,
    active: AtomicBool,
    files: Mutex<()>,
    cache: Mutex<Cache>,
}
impl Library {
    pub fn new(profile: &Path) -> Result<Arc<Self>> {
        Self::with_transport(profile, Arc::new(WinHttpTransport))
    }
    fn with_transport(profile: &Path, transport: Arc<dyn Transport>) -> Result<Arc<Self>> {
        regular_directory(profile)?;
        let root = profile.join("TafsirLibrary");
        ensure_directory(&root)?;
        ensure_directory(&root.join("Books"))?;
        ensure_directory(&root.join("Staging"))?;
        Ok(Arc::new(Self {
            root,
            transport,
            active: AtomicBool::new(false),
            files: Mutex::new(()),
            cache: Mutex::new(Cache::default()),
        }))
    }
    /// Lightweight metadata scan. Chapter content is hash-checked before use.
    pub fn installed_ids(&self) -> Result<Vec<usize>> {
        let _guard = self
            .files
            .try_lock()
            .map_err(|_| "Tafsir library is being updated; please retry")?;
        let mut installed = vec![];
        for index in 0..SOURCES.len() {
            if self.manifest(index).is_ok() {
                installed.push(index);
            }
        }
        Ok(installed)
    }
    pub fn download(self: &Arc<Self>, source: usize, generation: u64) -> Result<Job> {
        if let Some(issue) = source_at(source)?.download_issue() {
            return Err(issue.into());
        }
        self.job(source, generation, false)
    }
    pub fn remove_async(self: &Arc<Self>, source: usize, generation: u64) -> Result<Job> {
        self.job(source, generation, true)
    }
    fn job(self: &Arc<Self>, source: usize, generation: u64, remove: bool) -> Result<Job> {
        source_at(source)?;
        if self
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("A tafsir library operation is already running".into());
        }
        let state = Arc::new(JobState::default());
        let (sender, receiver) = mpsc::sync_channel(8);
        let library = self.clone();
        let worker_state = state.clone();
        if let Err(error) = thread::Builder::new()
            .name("tafsir-library".into())
            .stack_size(1024 * 1024)
            .spawn(move || {
                let result = if remove {
                    library.remove(source, &worker_state)
                } else {
                    library.download_book(source, generation, &worker_state, &sender)
                };
                library.active.store(false, Ordering::Release);
                let event = match result {
                    Ok(()) if remove => Event::Removed { generation, source },
                    Ok(()) => Event::Complete { generation, source },
                    Err(_) if worker_state.cancel.load(Ordering::Acquire) => {
                        Event::Cancelled { generation }
                    }
                    Err(error) => Event::Failed {
                        generation,
                        message: error.to_string(),
                    },
                };
                let _ = sender.send(event);
            })
        {
            self.active.store(false, Ordering::Release);
            return Err(error.into());
        }
        Ok(Job { receiver, state })
    }
    pub fn read_ayah(&self, source: usize, surah: u32, ayah: u32) -> Result<Option<String>> {
        source_at(source)?;
        validate_reference(surah, ayah)?;
        if let Some(study) = self
            .cache
            .lock()
            .map_err(|_| "Tafsir cache is unavailable")?
            .get(source, surah)
        {
            return Ok(Some(data::study_text(&study, false, ayah)));
        }
        // Removal verifies a sizeable book on its worker. Native UI reads never
        // wait for it: existing cached content remains usable during that work.
        let _guard = self
            .files
            .try_lock()
            .map_err(|_| "Tafsir library is being updated; please retry")?;
        let directory = self.book_path(source)?;
        if !directory.try_exists()? {
            return Ok(None);
        }
        let manifest = self.manifest(source)?;
        let receipt = &manifest.chapters[(surah - 1) as usize];
        let bytes = receipt_bytes(&directory, receipt)?;
        let study: Study = serde_json::from_slice(&bytes)?;
        validate_chapter(&study, surah)?;
        let study = Arc::new(study);
        let text = data::study_text(&study, false, ayah);
        self.cache
            .lock()
            .map_err(|_| "Tafsir cache is unavailable")?
            .insert(source, surah, chapter_memory(&study), study);
        Ok(Some(text))
    }
    fn book_path(&self, source: usize) -> Result<PathBuf> {
        regular_directory(&self.root)?;
        regular_directory(&self.root.join("Books"))?;
        Ok(self.root.join("Books").join(source_at(source)?.id))
    }
    fn manifest(&self, source: usize) -> Result<Manifest> {
        let directory = self.book_path(source)?;
        regular_directory(&directory)?;
        let bytes = read_bounded(&directory.join("pack.json"), MANIFEST_LIMIT)?;
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        validate_manifest(&manifest, source)?;
        for receipt in &manifest.chapters {
            let metadata = regular_file(&directory.join(&receipt.file))?;
            if metadata.len() != receipt.bytes {
                return Err("Tafsir chapter size does not match its receipt".into());
            }
        }
        Ok(manifest)
    }
    fn download_book(
        &self,
        source: usize,
        generation: u64,
        state: &JobState,
        sender: &mpsc::SyncSender<Event>,
    ) -> Result<()> {
        // Hashing old partial books belongs on this worker, not Library::new or
        // the native UI. Unknown or modified stages remain untouched.
        recover_stages(&self.root, state)?;
        let target = self.book_path(source)?;
        if target.try_exists()? {
            let manifest = self.manifest(source)?;
            verify_directory(&target, &manifest, state)?;
            return Ok(());
        }
        let mut stage = Stage::new(&self.root)?;
        let mut manifest = Manifest {
            schema: SCHEMA.into(),
            resource: source_at(source)?.resource,
            verses: 6236,
            bytes: 0,
            chapters: vec![],
        };
        let starting_bytes = library_bytes(&self.root)?;
        for surah in 1..=114 {
            state.check()?;
            let study = fetch_chapter(self.transport.as_ref(), source, surah, state)?;
            let bytes = serde_json::to_vec(&study)?;
            if bytes.len() as u64 > CHAPTER_LIMIT {
                return Err("Tafsir chapter exceeds the storage limit".into());
            }
            manifest.bytes = manifest
                .bytes
                .checked_add(bytes.len() as u64)
                .ok_or("Tafsir size overflow")?;
            if manifest.bytes > PACK_LIMIT
                || starting_bytes
                    .saturating_add(manifest.bytes)
                    .saturating_add(MANIFEST_LIMIT)
                    .saturating_add(JOURNAL_LIMIT)
                    > LIBRARY_LIMIT
            {
                return Err(
                    "Tafsir download would exceed the optional library storage limit".into(),
                );
            }
            let digest = sha256(&bytes)?;
            let name = format!("s{surah:03}-{digest}.json");
            stage.write(&name, &bytes)?;
            manifest.chapters.push(Receipt {
                surah,
                file: name,
                bytes: bytes.len() as u64,
                sha256: digest,
            });
            let _ = sender.try_send(Event::Progress {
                generation,
                chapters: surah,
                total: 114,
                bytes: manifest.bytes,
                percent: surah * 100 / 114,
            });
        }
        validate_manifest(&manifest, source)?;
        stage.write("pack.json", &serde_json::to_vec(&manifest)?)?;
        // The published book contains no transaction metadata. A crash after
        // this removal is recovered only from the complete pack manifest.
        stage.remove_journal()?;
        verify_directory(&stage.path, &manifest, state)?;
        stage.verify_owned_file("pack.json")?;
        state.check()?;
        let _guard = self.files.lock().map_err(|_| "Tafsir library is busy")?;
        if library_bytes(&self.root)? > LIBRARY_LIMIT {
            return Err("Tafsir library storage limit was reached".into());
        }
        // MoveFileW fails rather than replacing an existing directory. Other
        // software or unknown files never become collateral of an installation.
        regular_directory(&self.root.join("Books"))?;
        move_new(&stage.path, &target)?;
        stage.committed = true;
        self.cache
            .lock()
            .map_err(|_| "Tafsir cache is unavailable")?
            .invalidate(source);
        Ok(())
    }
    fn remove(&self, source: usize, state: &JobState) -> Result<()> {
        let _guard = self.files.lock().map_err(|_| "Tafsir library is busy")?;
        let directory = self.book_path(source)?;
        let manifest = self.manifest(source)?;
        verify_directory(&directory, &manifest, state)?;
        // Verify the complete set first, preserving a modified or unknown book
        // in its entirety instead of deleting only its recognizable portions.
        let pack = read_bounded(&directory.join("pack.json"), MANIFEST_LIMIT)?;
        let pack_hash = sha256(&pack)?;
        state.check()?;
        for receipt in &manifest.chapters {
            receipt_bytes(&directory, receipt)?;
            fs::remove_file(directory.join(&receipt.file))?;
        }
        if sha256(&read_bounded(&directory.join("pack.json"), MANIFEST_LIMIT)?)? != pack_hash {
            return Err(
                "Tafsir manifest changed during removal; remaining files were preserved".into(),
            );
        }
        fs::remove_file(directory.join("pack.json"))?;
        fs::remove_dir(directory)?;
        self.cache
            .lock()
            .map_err(|_| "Tafsir cache is unavailable")?
            .invalidate(source);
        Ok(())
    }
}

#[cfg(feature = "qa")]
pub fn network_smoke(output: &Path, selected: usize) -> Result<()> {
    let selected_source = source_at(selected)?;
    fs::create_dir_all(output)?;
    let state = JobState::default();
    let mut sources = Vec::new();
    for (index, source) in SOURCES.iter().enumerate() {
        let study = fetch_chapter(&WinHttpTransport, index, 1, &state)?;
        validate_chapter(&study, 1)?;
        sources.push(serde_json::json!({"source":source.id,"verses":study.lookup.len(),"groups":study.groups.len()}));
    }
    fs::write(
        output.join("native-tafsir-provider-preflight.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"sources":sources,"chapter":1,"transport":"Native WinHTTP HTTPS","fullBookVerified":false
        }))?,
    )?;
    let profile = output.join("isolated-profile");
    fs::create_dir(&profile)?;
    let library = Library::new(&profile)?;
    let job = library.download(selected, 1)?;
    let began = Instant::now();
    let mut progress = 0;
    loop {
        if began.elapsed() > Duration::from_secs(480) {
            job.cancel();
            return Err("Complete tafsir QA download timed out".into());
        }
        match job.receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(Event::Progress { chapters, .. }) => {
                progress = progress.max(chapters);
            }
            Ok(Event::Complete {
                source,
                generation: 1,
            }) if source == selected => break,
            Ok(Event::Failed { message, .. }) => return Err(message.into()),
            Ok(Event::Cancelled { .. }) => {
                return Err("Complete tafsir QA download cancelled".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            _ => return Err("Unexpected tafsir QA worker result".into()),
        }
    }
    let manifest = library.manifest(selected)?;
    verify_directory(&library.book_path(selected)?, &manifest, &state)?;
    let reloaded = Library::new(&profile)?;
    let mut verses = 0;
    for surah in 1..=114 {
        for ayah in 1..=COUNTS[(surah - 1) as usize] {
            if reloaded
                .read_ayah(selected, surah, ayah)?
                .is_none_or(|text| text.is_empty())
            {
                return Err("Completed tafsir lost a publisher verse mapping".into());
            }
            verses += 1;
        }
    }
    fs::write(
        output.join("native-network-tafsir.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"sources":sources,"completeBook":selected_source.id,"chapters":manifest.chapters.len(),
            "verses":verses,"bytes":manifest.bytes,"progressChapters":progress,"hashesVerified":true,
            "readableAfterRestart":true,"seconds":began.elapsed().as_secs_f64(),"profile":profile
        }))?,
    )?;
    Ok(())
}
fn source_at(index: usize) -> Result<&'static Source> {
    SOURCES
        .get(index)
        .ok_or_else(|| "Unknown optional tafsir source".into())
}
fn validate_reference(surah: u32, ayah: u32) -> Result<()> {
    if !(1..=114).contains(&surah) || !(1..=COUNTS[(surah - 1) as usize]).contains(&ayah) {
        return Err("Invalid tafsir verse reference".into());
    }
    Ok(())
}
fn canonical_reference(key: &str, surah: u32) -> Result<u32> {
    let (chapter, verse) = key
        .split_once(':')
        .ok_or("Invalid publisher verse reference")?;
    let chapter: u32 = chapter.parse()?;
    let verse: u32 = verse.parse()?;
    if chapter != surah || key != format!("{chapter}:{verse}") {
        return Err(
            "Publisher returned a different chapter or noncanonical verse reference".into(),
        );
    }
    validate_reference(chapter, verse)?;
    Ok(verse)
}
fn validate_chapter(study: &Study, surah: u32) -> Result<()> {
    validate_reference(surah, 1)?;
    let count = COUNTS[(surah - 1) as usize] as usize;
    if study.lookup.len() != count || study.groups.is_empty() || study.groups.len() > count {
        return Err("Optional tafsir chapter is incomplete".into());
    }
    for (key, group) in &study.groups {
        if key.len() > 64
            || key.is_empty()
            || group.text.is_empty()
            || group.text.len() > RESPONSE_LIMIT
            || group.ayah_keys.is_empty()
            || group.ayah_keys.len() > count
            || group.ayah_keys.windows(2).any(|p| p[0] >= p[1])
        {
            return Err("Invalid optional tafsir group".into());
        }
        for ayah in &group.ayah_keys {
            validate_reference(surah, *ayah)?;
        }
    }
    for ayah in 1..=count as u32 {
        if !study
            .lookup
            .get(&ayah.to_string())
            .and_then(Value::as_str)
            .and_then(|key| study.groups.get(key))
            .is_some_and(|group| group.ayah_keys.contains(&ayah))
        {
            return Err("Optional tafsir ayah mapping is incomplete".into());
        }
    }
    Ok(())
}
fn chapter_memory(study: &Study) -> usize {
    study
        .groups
        .iter()
        .map(|(key, group)| {
            key.len()
                .saturating_add(group.text.capacity())
                .saturating_add(group.ayah_keys.capacity() * 4)
                .saturating_add(192)
        })
        .chain(study.lookup.iter().map(|(key, value)| {
            key.len()
                .saturating_add(value.as_str().map_or(32, str::len))
                .saturating_add(192)
        }))
        .fold(0, usize::saturating_add)
}
fn validate_manifest(manifest: &Manifest, source: usize) -> Result<()> {
    if manifest.schema != SCHEMA
        || manifest.resource != source_at(source)?.resource
        || manifest.verses != 6236
        || manifest.chapters.len() != 114
        || manifest.bytes == 0
        || manifest.bytes > PACK_LIMIT
    {
        return Err("Invalid or incomplete tafsir pack manifest".into());
    }
    let mut bytes = 0u64;
    for (i, receipt) in manifest.chapters.iter().enumerate() {
        if receipt.surah != i as u32 + 1
            || receipt.bytes == 0
            || receipt.bytes > CHAPTER_LIMIT
            || !valid_hash(&receipt.sha256)
            || receipt.file != format!("s{:03}-{}.json", receipt.surah, receipt.sha256)
        {
            return Err("Invalid tafsir chapter receipt".into());
        }
        bytes = bytes
            .checked_add(receipt.bytes)
            .ok_or("Tafsir size overflow")?;
    }
    if bytes != manifest.bytes {
        return Err("Tafsir book size does not match its receipts".into());
    }
    Ok(())
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn regular_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("Tafsir storage is not a regular directory".into());
    }
    Ok(())
}
fn ensure_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    regular_directory(path)
}
fn regular_file(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("Tafsir storage contains a nonregular file".into());
    }
    Ok(metadata)
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = regular_file(path)?;
    if metadata.len() > limit {
        return Err("Tafsir file exceeds its read limit".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("Tafsir file grew beyond its read limit".into());
    }
    Ok(bytes)
}
fn receipt_bytes(directory: &Path, receipt: &Receipt) -> Result<Vec<u8>> {
    regular_directory(directory)?;
    let bytes = read_bounded(&directory.join(&receipt.file), CHAPTER_LIMIT)?;
    if bytes.len() as u64 != receipt.bytes || sha256(&bytes)? != receipt.sha256 {
        return Err("Tafsir chapter failed its integrity check".into());
    }
    Ok(bytes)
}
fn verify_directory(directory: &Path, manifest: &Manifest, state: &JobState) -> Result<()> {
    regular_directory(directory)?;
    let expected: BTreeSet<_> = manifest
        .chapters
        .iter()
        .map(|receipt| receipt.file.as_str())
        .chain(Some("pack.json"))
        .collect();
    let mut seen = 0;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry
            .file_name()
            .to_str()
            .is_some_and(|name| expected.contains(name))
        {
            return Err("Tafsir folder contains unknown files; it was preserved".into());
        }
        regular_file(&entry.path())?;
        seen += 1;
    }
    if seen != expected.len() {
        return Err("Tafsir pack is incomplete".into());
    }
    for receipt in &manifest.chapters {
        state.check()?;
        receipt_bytes(directory, receipt)?;
    }
    Ok(())
}
fn library_bytes(root: &Path) -> Result<u64> {
    fn scan(path: &Path, depth: usize, files: &mut usize) -> Result<u64> {
        regular_directory(path)?;
        let mut size = 0u64;
        for entry in fs::read_dir(path)? {
            *files += 1;
            if *files > 4096 {
                return Err("Optional tafsir library has too many files".into());
            }
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_attributes() & REPARSE_POINT != 0 {
                return Err("Optional tafsir library contains a reparse point".into());
            }
            let bytes = if metadata.is_dir() {
                if depth >= 3 {
                    return Err("Unexpected nested tafsir storage".into());
                }
                scan(&entry.path(), depth + 1, files)?
            } else if metadata.is_file() {
                metadata.len()
            } else {
                return Err("Unexpected tafsir storage entry".into());
            };
            size = size
                .checked_add(bytes)
                .ok_or("Tafsir library size overflow")?;
            if size > LIBRARY_LIMIT {
                return Err("Optional tafsir library exceeds its storage limit".into());
            }
        }
        Ok(size)
    }
    scan(root, 0, &mut 0)
}
struct Stage {
    path: PathBuf,
    owned: BTreeMap<String, (u64, String)>,
    journal_hash: Option<String>,
    committed: bool,
}
impl Stage {
    fn new(root: &Path) -> Result<Self> {
        let parent = root.join("Staging");
        regular_directory(&parent)?;
        let path = parent.join(format!(
            "{}-{}.partial",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        let mut stage = Self {
            path,
            owned: BTreeMap::new(),
            journal_hash: None,
            committed: false,
        };
        stage.persist_journal()?;
        Ok(stage)
    }
    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        self.write_with(name, bytes, |file, bytes| {
            file.write_all(bytes)?;
            file.sync_all()
        })
    }
    fn write_with(
        &mut self,
        name: &str,
        bytes: &[u8],
        write: impl FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
    ) -> Result<()> {
        regular_directory(&self.path)?;
        let hash = sha256(bytes)?;
        validate_stage_file(name, bytes.len() as u64, &hash)?;
        let path = self.path.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        if let Err(error) = write(&mut file, bytes) {
            // Only this create_new-owned file is removed. Otherwise a short
            // write makes the stage look unknown and strands its earlier files.
            drop(file);
            if let Err(cleanup) = fs::remove_file(&path) {
                return Err(format!(
                    "Tafsir write failed: {error}; could not remove its partial file: {cleanup}"
                )
                .into());
            }
            return Err(error.into());
        }
        self.owned.insert(name.into(), (bytes.len() as u64, hash));
        self.persist_journal()?;
        Ok(())
    }
    fn clean(&self) -> Result<()> {
        clean_stage(
            &self.path,
            &self.owned,
            self.journal_hash.as_deref(),
            &JobState::default(),
        )
    }
    fn verify_owned_file(&self, name: &str) -> Result<()> {
        regular_directory(&self.path)?;
        let (length, hash) = self
            .owned
            .get(name)
            .ok_or("Unknown staging files were preserved")?;
        let bytes = read_bounded(&self.path.join(name), CHAPTER_LIMIT.max(MANIFEST_LIMIT))?;
        if bytes.len() as u64 != *length || sha256(&bytes)? != *hash {
            return Err("Modified staging files were preserved".into());
        }
        Ok(())
    }
    fn persist_journal(&mut self) -> Result<()> {
        regular_directory(&self.path)?;
        let stage = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid tafsir stage name")?;
        let journal = StageJournal {
            schema: STAGE_SCHEMA.into(),
            stage: stage.into(),
            files: self
                .owned
                .iter()
                .map(|(file, (bytes, sha256))| StageReceipt {
                    file: file.clone(),
                    bytes: *bytes,
                    sha256: sha256.clone(),
                })
                .collect(),
        };
        validate_stage_journal(&journal, stage)?;
        let bytes = serde_json::to_vec(&journal)?;
        if bytes.len() as u64 > JOURNAL_LIMIT {
            return Err("Tafsir staging journal exceeds its size limit".into());
        }
        let hash = sha256(&bytes)?;
        let target = self.path.join(STAGE_JOURNAL);
        if let Some(expected) = &self.journal_hash {
            if sha256(&read_bounded(&target, JOURNAL_LIMIT)?)? != *expected {
                return Err("Tafsir staging journal changed; files were preserved".into());
            }
        } else if target.try_exists()? {
            return Err("Unknown tafsir staging journal was preserved".into());
        }
        let temporary = self.path.join(format!(
            "transaction-{}-{}.partial",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        drop(file);
        let result = move_journal(&temporary, &target, self.journal_hash.is_some());
        if result.is_err() && sha256(&read_bounded(&temporary, JOURNAL_LIMIT)?)? == hash {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        self.journal_hash = Some(hash);
        Ok(())
    }
    fn remove_journal(&mut self) -> Result<()> {
        let Some(expected) = self.journal_hash.as_deref() else {
            return Ok(());
        };
        if sha256(&read_bounded(
            &self.path.join(STAGE_JOURNAL),
            JOURNAL_LIMIT,
        )?)? != expected
        {
            return Err("Tafsir staging journal changed; files were preserved".into());
        }
        fs::remove_file(self.path.join(STAGE_JOURNAL))?;
        self.journal_hash = None;
        Ok(())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.clean();
        }
    }
}
fn valid_stage_name(name: &str) -> bool {
    if name.len() > 64 {
        return false;
    }
    name.strip_suffix(".partial")
        .and_then(|n| n.split_once('-'))
        .is_some_and(|(pid, nonce)| {
            pid.parse::<u32>()
                .ok()
                .is_some_and(|n| n > 0 && n.to_string() == pid)
                && nonce
                    .parse::<u64>()
                    .ok()
                    .is_some_and(|n| n > 0 && n.to_string() == nonce)
        })
}
fn validate_stage_file(name: &str, bytes: u64, hash: &str) -> Result<()> {
    if !valid_hash(hash) || bytes == 0 {
        return Err("Invalid tafsir staging receipt".into());
    }
    if name == "pack.json" && bytes <= MANIFEST_LIMIT {
        return Ok(());
    }
    if bytes <= CHAPTER_LIMIT && (1..=114).any(|surah| name == format!("s{surah:03}-{hash}.json")) {
        return Ok(());
    }
    Err("Invalid tafsir staging path or size".into())
}
fn validate_stage_journal(
    journal: &StageJournal,
    stage: &str,
) -> Result<BTreeMap<String, (u64, String)>> {
    if journal.schema != STAGE_SCHEMA
        || journal.stage != stage
        || !valid_stage_name(stage)
        || journal.files.len() > 115
    {
        return Err("Invalid tafsir staging journal".into());
    }
    let mut owned = BTreeMap::new();
    let mut total = 0u64;
    let mut surahs = BTreeSet::new();
    for file in &journal.files {
        validate_stage_file(&file.file, file.bytes, &file.sha256)?;
        if file.file != "pack.json" && !surahs.insert(file.file[..4].to_owned()) {
            return Err("Duplicate tafsir staging chapter".into());
        }
        total = total
            .checked_add(file.bytes)
            .ok_or("Tafsir staging size overflow")?;
        if total > PACK_LIMIT + MANIFEST_LIMIT
            || owned
                .insert(file.file.clone(), (file.bytes, file.sha256.clone()))
                .is_some()
        {
            return Err("Tafsir staging journal exceeds its bounds".into());
        }
    }
    Ok(owned)
}
fn bounded_stage_entries(path: &Path) -> Result<Vec<fs::DirEntry>> {
    regular_directory(path)?;
    let entries = fs::read_dir(path)?
        .take(117)
        .collect::<std::io::Result<Vec<_>>>()?;
    if entries.len() > 116 {
        return Err("Tafsir stage has too many entries; files were preserved".into());
    }
    Ok(entries)
}
fn clean_stage(
    path: &Path,
    owned: &BTreeMap<String, (u64, String)>,
    journal_hash: Option<&str>,
    state: &JobState,
) -> Result<()> {
    let entries = bounded_stage_entries(path)?;
    if entries.len() > owned.len() + usize::from(journal_hash.is_some()) {
        return Err("Unknown staging files were preserved".into());
    }
    let mut present = Vec::new();
    let mut found_journal = false;
    // Validate the entire exact set before deleting any file. Linked, changed,
    // oversized, or additional entries preserve this whole stage.
    for entry in &entries {
        state.check()?;
        let name = entry.file_name();
        let name = name.to_str().ok_or("Unknown staging filename")?;
        if name == STAGE_JOURNAL {
            if journal_hash != Some(sha256(&read_bounded(&entry.path(), JOURNAL_LIMIT)?)?.as_str())
            {
                return Err("Modified staging journal was preserved".into());
            }
            found_journal = true;
        } else {
            let (length, hash) = owned
                .get(name)
                .ok_or("Unknown staging files were preserved")?;
            let bytes = read_bounded(
                &entry.path(),
                if name == "pack.json" {
                    MANIFEST_LIMIT
                } else {
                    CHAPTER_LIMIT
                },
            )?;
            if bytes.len() as u64 != *length || sha256(&bytes)? != *hash {
                return Err("Modified staging files were preserved".into());
            }
            present.push(name.to_owned());
        }
    }
    if found_journal != journal_hash.is_some() {
        return Err("Tafsir staging journal disappeared; files were preserved".into());
    }
    state.check()?;
    // Recheck each owned file immediately before its nonrecursive removal.
    // Missing receipt files may have been removed by an interrupted cleanup.
    // Once deletion starts, finish rather than leaving a half-deleted stage on
    // cancellation. Unknown or modified files failed the preflight above.
    for name in present {
        let (length, hash) = &owned[&name];
        regular_directory(path)?;
        let bytes = read_bounded(
            &path.join(&name),
            if name == "pack.json" {
                MANIFEST_LIMIT
            } else {
                CHAPTER_LIMIT
            },
        )?;
        if bytes.len() as u64 != *length || sha256(&bytes)? != *hash {
            return Err("Modified staging files were preserved".into());
        }
        fs::remove_file(path.join(&name))?;
    }
    if let Some(hash) = journal_hash {
        if sha256(&read_bounded(&path.join(STAGE_JOURNAL), JOURNAL_LIMIT)?)? != hash {
            return Err("Modified staging journal was preserved".into());
        }
        fs::remove_file(path.join(STAGE_JOURNAL))?;
    }
    fs::remove_dir(path)?;
    Ok(())
}
fn recover_stages(root: &Path, state: &JobState) -> Result<()> {
    regular_directory(root)?;
    let staging = root.join("Staging");
    regular_directory(&staging)?;
    let entries = fs::read_dir(&staging)?
        .take(MAX_STAGING_ENTRIES + 1)
        .collect::<std::io::Result<Vec<_>>>()?;
    if entries.len() > MAX_STAGING_ENTRIES {
        return Err("Tafsir staging has too many entries; files were preserved".into());
    }
    // Another running app can be writing a valid journal in this profile.
    // Preserve its stage. If enumeration fails or is truncated, preserve all
    // stages conservatively instead of guessing whether their owner exited.
    let mut process_ids = vec![0u32; 16_384];
    let mut process_bytes = 0;
    let capacity_bytes = (process_ids.len() * std::mem::size_of::<u32>()) as u32;
    if unsafe {
        windows::Win32::System::ProcessStatus::EnumProcesses(
            process_ids.as_mut_ptr(),
            capacity_bytes,
            &mut process_bytes,
        )
    }
    .is_err()
        || process_bytes >= capacity_bytes
        || process_bytes % 4 != 0
    {
        return Ok(());
    }
    process_ids.truncate(process_bytes as usize / 4);
    let running: BTreeSet<_> = process_ids.into_iter().collect();
    let mut hashed_bytes = 0u64;
    for entry in entries {
        state.check()?;
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|name| valid_stage_name(name)) else {
            continue;
        };
        if name
            .split_once('-')
            .and_then(|(pid, _)| pid.parse::<u32>().ok())
            .is_none_or(|pid| running.contains(&pid))
        {
            continue;
        }
        if regular_directory(&path).is_err() {
            continue;
        }
        let candidate = (|| -> Result<_> {
            let journal_path = path.join(STAGE_JOURNAL);
            if journal_path.try_exists()? {
                let bytes = read_bounded(&journal_path, JOURNAL_LIMIT)?;
                let journal: StageJournal = serde_json::from_slice(&bytes)?;
                let owned = validate_stage_journal(&journal, name)?;
                Ok((owned, Some(sha256(&bytes)?)))
            } else {
                // Commit removes the journal immediately before the folder
                // rename. Only an exact complete publisher pack can recover.
                let bytes = read_bounded(&path.join("pack.json"), MANIFEST_LIMIT)?;
                let manifest: Manifest = serde_json::from_slice(&bytes)?;
                let source = SOURCES
                    .iter()
                    .position(|source| source.resource == manifest.resource)
                    .ok_or("Unknown tafsir pack source")?;
                validate_manifest(&manifest, source)?;
                let mut owned: BTreeMap<_, _> = manifest
                    .chapters
                    .into_iter()
                    .map(|r| (r.file, (r.bytes, r.sha256)))
                    .collect();
                owned.insert("pack.json".into(), (bytes.len() as u64, sha256(&bytes)?));
                Ok((owned, None))
            }
        })();
        let Ok((owned, journal_hash)) = candidate else {
            continue;
        };
        let candidate_bytes = owned.values().map(|(bytes, _)| *bytes).sum::<u64>();
        hashed_bytes = hashed_bytes
            .checked_add(candidate_bytes)
            .ok_or("Tafsir recovery size overflow")?;
        if hashed_bytes > LIBRARY_LIMIT {
            return Err(
                "Tafsir recovery exceeds its storage bound; remaining files were preserved".into(),
            );
        }
        // Bad candidates do not prevent healthy candidates from recovering.
        let _ = clean_stage(&path, &owned, journal_hash.as_deref(), state);
        state.check()?;
    }
    Ok(())
}
fn move_journal(from: &Path, to: &Path, replace: bool) -> Result<()> {
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_WRITE_THROUGH
                | if replace {
                    MOVEFILE_REPLACE_EXISTING
                } else {
                    Default::default()
                },
        )?;
    }
    Ok(())
}
fn move_new(from: &Path, to: &Path) -> Result<()> {
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()))?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Request {
    Chapter {
        source: usize,
        surah: u32,
        page: u32,
    },
    Ayah {
        source: usize,
        surah: u32,
        ayah: u32,
    },
}
impl Request {
    fn path(self) -> Result<String> {
        let (source, surah) = match self {
            Self::Chapter { source, surah, .. } | Self::Ayah { source, surah, .. } => {
                (source, surah)
            }
        };
        validate_reference(surah, 1)?;
        let resource = source_at(source)?.resource;
        Ok(match self {
            Self::Chapter { page, .. } if (1..=7).contains(&page) => {
                format!("/api/v4/tafsirs/{resource}/by_chapter/{surah}?per_page=50&page={page}")
            }
            Self::Ayah { ayah, .. } => {
                validate_reference(surah, ayah)?;
                format!("/api/v4/tafsirs/{resource}/by_ayah/{surah}:{ayah}")
            }
            _ => return Err("Invalid tafsir pagination request".into()),
        })
    }
}
trait Transport: Send + Sync {
    fn fetch(&self, request: Request, state: &JobState) -> Result<Vec<u8>>;
}
#[derive(Deserialize)]
struct RemoteRow {
    resource_id: u32,
    verse_key: String,
    text: String,
}
#[derive(Deserialize)]
struct Pagination {
    current_page: u32,
    next_page: Option<u32>,
    total_pages: u32,
    per_page: u32,
}
#[derive(Deserialize)]
struct ChapterResponse {
    tafsirs: Vec<RemoteRow>,
    pagination: Pagination,
}
#[derive(Deserialize)]
struct AyahResponse {
    tafsir: SharedRow,
}
#[derive(Deserialize)]
struct SharedRow {
    resource_id: u32,
    text: String,
    verses: BTreeMap<String, Value>,
}
struct Row {
    text: String,
    references: Vec<u32>,
}
fn response_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.is_empty() || bytes.len() > RESPONSE_LIMIT {
        return Err("Tafsir provider response exceeds its size limit".into());
    }
    Ok(serde_json::from_slice(bytes)?)
}
fn fetch_response<T: serde::de::DeserializeOwned>(
    transport: &dyn Transport,
    request: Request,
    state: &JobState,
) -> Result<T> {
    // Paths come exclusively from bounded source/chapter/ayah identifiers and
    // contain no credentials or remotely supplied error-body text.
    let path = request.path()?;
    let bytes = transport
        .fetch(request, state)
        .map_err(|error| format!("Tafsir request {path} failed: {error}"))?;
    response_json(&bytes)
        .map_err(|error| format!("Tafsir response {path} was invalid: {error}").into())
}
fn fetch_chapter(
    transport: &dyn Transport,
    source: usize,
    surah: u32,
    state: &JobState,
) -> Result<Study> {
    validate_reference(surah, 1)?;
    let count = COUNTS[(surah - 1) as usize];
    let resource = source_at(source)?.resource;
    let mut rows = BTreeMap::<u32, Row>::new();
    let mut chapter_references = BTreeSet::new();
    let mut text_bytes = 0u64;
    let mut page = 1;
    let mut total_pages = None;
    loop {
        state.check()?;
        let response: ChapterResponse = fetch_response(
            transport,
            Request::Chapter {
                source,
                surah,
                page,
            },
            state,
        )?;
        let paging = response.pagination;
        if paging.current_page != page
            || !(1..=7).contains(&paging.total_pages)
            || !(1..=50).contains(&paging.per_page)
            || response.tafsirs.len() > 50
            || paging
                .next_page
                .is_some_and(|next| next != page + 1 || next > paging.total_pages || next > 7)
            || (paging.next_page.is_none() && page != paging.total_pages)
        {
            return Err("Invalid tafsir provider pagination".into());
        }
        if total_pages.is_some_and(|total| total != paging.total_pages) {
            return Err("Tafsir pagination changed during the download".into());
        }
        total_pages = Some(paging.total_pages);
        for row in response.tafsirs {
            if row.resource_id != resource || row.text.len() > RESPONSE_LIMIT {
                return Err(format!("Tafsir resource {resource} chapter {surah} returned a different or invalid resource").into());
            }
            let ayah = canonical_reference(&row.verse_key, surah)
                .map_err(|error| format!("Tafsir resource {resource} chapter {surah} returned an invalid verse reference: {error}"))?;
            // Empty rows are explicit publisher placeholders for shared
            // explanations (for example Al-Wasit 6:5). Validate their identity
            // and duplicates before resolving the exact by_ayah response below.
            // Never guess a neighboring explanation or treat a server error as
            // proof that the author omitted a verse.
            if !chapter_references.insert(ayah) {
                return Err(format!(
                    "Tafsir resource {resource} chapter {surah} has a duplicate ayah {ayah}"
                )
                .into());
            }
            if row.text.trim().is_empty() {
                continue;
            }
            text_bytes = text_bytes
                .checked_add(row.text.len() as u64)
                .ok_or("Tafsir chapter size overflow")?;
            if text_bytes > CHAPTER_LIMIT
                || rows
                    .insert(
                        ayah,
                        Row {
                            text: row.text,
                            references: vec![ayah],
                        },
                    )
                    .is_some()
            {
                return Err("Tafsir chapter has duplicates or exceeds its limit".into());
            }
        }
        if chapter_references.len() > count as usize {
            return Err("Tafsir chapter has too many ayahs".into());
        }
        if let Some(next) = paging.next_page {
            page = next;
        } else {
            break;
        }
    }
    // Some publishers return one shared record for several ayahs. The by_ayah
    // response explicitly lists its references; never infer missing ranges.
    for ayah in 1..=count {
        if rows.contains_key(&ayah) {
            continue;
        }
        state.check()?;
        let response: AyahResponse = fetch_response(
            transport,
            Request::Ayah {
                source,
                surah,
                ayah,
            },
            state,
        )?;
        let shared = response.tafsir;
        if shared.resource_id != resource
            || shared.text.trim().is_empty()
            || shared.text.len() > RESPONSE_LIMIT
            || shared.verses.is_empty()
            || shared.verses.len() > count as usize
        {
            return Err(format!(
                "Invalid shared tafsir resource {resource} response for ayah {surah}:{ayah}"
            )
            .into());
        }
        let mut references = shared
            .verses
            .keys()
            .map(|key| canonical_reference(key, surah))
            .collect::<Result<Vec<_>>>()?;
        references.sort_unstable();
        references.dedup();
        if !references.contains(&ayah) {
            return Err(format!("Publisher tafsir resource {resource} mapping does not include requested ayah {surah}:{ayah}").into());
        }
        for reference in &references {
            if rows.contains_key(reference) {
                continue;
            }
            text_bytes = text_bytes
                .checked_add(shared.text.len() as u64)
                .ok_or("Tafsir chapter size overflow")?;
            if text_bytes > CHAPTER_LIMIT {
                return Err("Shared tafsir chapter exceeds its limit".into());
            }
            rows.insert(
                *reference,
                Row {
                    text: shared.text.clone(),
                    references: references.clone(),
                },
            );
        }
    }
    let mut study = Study {
        lookup: BTreeMap::new(),
        groups: BTreeMap::new(),
    };
    // Equal text alone does not establish that a publisher intended a shared
    // explanation. Retain the exact reference set as part of group identity.
    let mut group_hashes = BTreeMap::<(String, Vec<u32>), String>::new();
    for (ayah, row) in rows {
        let digest = sha256(row.text.as_bytes())?;
        let identity = (digest, row.references.clone());
        let group_key = if let Some(key) = group_hashes.get(&identity) {
            let group = study.groups.get(key).ok_or("Missing tafsir group")?;
            if group.text != row.text || group.ayah_keys != row.references {
                return Err("Tafsir text hash collision".into());
            }
            key.clone()
        } else {
            let key = (study.groups.len() + 1).to_string();
            study.groups.insert(
                key.clone(),
                Group {
                    ayah_keys: row.references,
                    text: row.text,
                },
            );
            group_hashes.insert(identity, key.clone());
            key
        };
        study
            .lookup
            .insert(ayah.to_string(), Value::String(group_key));
    }
    validate_chapter(&study, surah)?;
    Ok(study)
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
    fn fetch(&self, request: Request, state: &JobState) -> Result<Vec<u8>> {
        let mut last = String::new();
        for attempt in 0..3 {
            state.check()?;
            match fetch_https(request, state) {
                Ok(bytes) => return Ok(bytes),
                Err(error) => last = error.to_string(),
            }
            state.check()?;
            if attempt < 2 {
                for _ in 0..(20 * (attempt + 1)) {
                    state.check()?;
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
        Err(format!(
            "Could not download tafsir {} after 3 attempts: {last}",
            request.path()?
        )
        .into())
    }
}
fn fetch_https(request: Request, state: &JobState) -> Result<Vec<u8>> {
    let path: Vec<_> = request.path()?.encode_utf16().chain(Some(0)).collect();
    let started = Instant::now();
    unsafe {
        let session = InternetHandle::new(WinHttpOpen(
            w!("QuranMushafReader-Native/0.2"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 8_000, 8_000, 8_000, 8_000)?;
        crate::http_security::configure_protocols(session.0)?;
        WinHttpSetOption(
            Some(session.0),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes()),
        )?;
        let connection =
            InternetHandle::new(WinHttpConnect(session.0, w!("api.quran.com"), 443, 0))?;
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
        let headers: Vec<_> = "Accept: application/json\r\nAccept-Encoding: identity\r\n"
            .encode_utf16()
            .collect();
        state.check()?;
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
        let encoding = header(request.0, WINHTTP_QUERY_CONTENT_ENCODING)?;
        let length = header(request.0, WINHTTP_QUERY_CONTENT_LENGTH)?;
        let expected = response_headers(status, &content_type, &encoding, &length)?;
        let mut bytes = Vec::with_capacity(expected.unwrap_or(32 * 1024) as usize);
        let mut buffer = [0u8; 32 * 1024];
        loop {
            state.check()?;
            if started.elapsed() > Duration::from_secs(45) {
                return Err("Tafsir download exceeded its time limit".into());
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
            if bytes.len() + count as usize > RESPONSE_LIMIT {
                return Err("Tafsir response grew beyond its size limit".into());
            }
            bytes.extend_from_slice(&buffer[..count as usize]);
        }
        state.check()?;
        if bytes.is_empty() || expected.is_some_and(|n| n != bytes.len() as u64) {
            return Err("Tafsir response is empty or incomplete".into());
        }
        Ok(bytes)
    }
}
fn response_headers(
    status: u32,
    content_type: &str,
    encoding: &str,
    length: &str,
) -> Result<Option<u64>> {
    if status != 200 {
        return Err(format!(
            "Quran.com tafsir provider returned HTTP {status}; no account credentials are used"
        )
        .into());
    }
    let media_type = content_type.split(';').next().unwrap_or("").trim();
    if !media_type.eq_ignore_ascii_case("application/json")
        || (!encoding.is_empty() && !encoding.eq_ignore_ascii_case("identity"))
    {
        return Err("Tafsir provider returned a non-JSON or encoded response".into());
    }
    let expected = if length.is_empty() {
        None
    } else {
        Some(
            length
                .parse::<u64>()
                .map_err(|_| "Invalid tafsir Content-Length")?,
        )
    };
    if expected.is_some_and(|n| n == 0 || n > RESPONSE_LIMIT as u64) {
        return Err("Tafsir response exceeds its size limit".into());
    }
    Ok(expected)
}
unsafe fn header(request: *mut c_void, kind: u32) -> Result<String> {
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
        return Err("Tafsir response header exceeds its limit".into());
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
        .map_err(|_| "Invalid tafsir response header")?
        .trim_end_matches('\0')
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    struct Profile(PathBuf);
    impl Profile {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "qmr-tafsir-test-{}-{}",
                std::process::id(),
                NONCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn known_inconsistent_english_books_cannot_start_a_download() {
        let profile = Profile::new();
        let library = Library::new(&profile.0).unwrap();
        for source in 4..7 {
            let result = library.download(source, 1);
            assert!(result.is_err());
            assert!(
                result
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("did not pass verification")
            );
            assert!(!library.active.load(Ordering::Acquire));
        }
        assert!(library.installed_ids().unwrap().is_empty());
        assert!(
            SOURCES[..4]
                .iter()
                .all(|source| source.download_issue().is_none())
        );
    }
    #[derive(Default)]
    struct Publisher {
        calls: AtomicUsize,
        cancel_after: usize,
        shared_first: bool,
        empty_shared_rows: bool,
    }
    impl Transport for Publisher {
        fn fetch(&self, request: Request, state: &JobState) -> Result<Vec<u8>> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if self.cancel_after > 0 && call >= self.cancel_after {
                state.cancel.store(true, Ordering::Release);
            }
            match request {
                Request::Chapter {
                    source,
                    surah,
                    page,
                } => {
                    let count = COUNTS[(surah - 1) as usize];
                    let refs: Vec<_> = (1..=count)
                        .filter(|ayah| {
                            !(self.shared_first
                                && !self.empty_shared_rows
                                && surah == 1
                                && [2, 3].contains(ayah))
                        })
                        .collect();
                    let total_pages = (refs.len() as u32).div_ceil(50);
                    let rows: Vec<_> = refs.into_iter().skip((page as usize - 1) * 50).take(50).map(|ayah| serde_json::json!({
                        "resource_id": SOURCES[source].resource,
                        "verse_key": format!("{surah}:{ayah}"),
                        // Synthetic commentary only: no Quran text is generated.
                        "text": if self.empty_shared_rows && surah == 1 && [2, 3].contains(&ayah) { String::new() } else { format!("<p>Publisher explanation {surah}:{ayah} &amp; notes.</p>") }
                    })).collect();
                    Ok(serde_json::to_vec(
                        &serde_json::json!({ "tafsirs": rows, "pagination": { "current_page":page,"per_page":50,"next_page":if page < total_pages {Some(page+1)}else{None},"total_pages":total_pages } }),
                    )?)
                }
                Request::Ayah {
                    source,
                    surah,
                    ayah,
                } => {
                    if !self.shared_first || surah != 1 || ayah != 2 {
                        return Err("Unexpected test ayah request".into());
                    }
                    Ok(serde_json::to_vec(
                        &serde_json::json!({"tafsir":{"resource_id":SOURCES[source].resource,"text":"<p>Shared publisher explanation.</p>","verses":{"1:2":{"id":2},"1:3":{"id":3}}}}),
                    )?)
                }
            }
        }
    }
    struct Response(Vec<u8>);
    impl Transport for Response {
        fn fetch(&self, _request: Request, _state: &JobState) -> Result<Vec<u8>> {
            Ok(self.0.clone())
        }
    }
    fn download(library: &Library, source: usize, state: &JobState) -> Result<()> {
        let (sender, _receiver) = mpsc::sync_channel(8);
        library.download_book(source, 7, state, &sender)
    }
    #[test]
    fn complete_books_shared_publisher_ranges_and_safe_removal() {
        let profile = Profile::new();
        let library = Library::with_transport(
            &profile.0,
            Arc::new(Publisher {
                shared_first: true,
                ..Default::default()
            }),
        )
        .unwrap();
        download(&library, 0, &JobState::default()).unwrap();
        assert_eq!(library.installed_ids().unwrap(), vec![0]);
        assert_eq!(
            library.read_ayah(0, 1, 2).unwrap().unwrap(),
            "2–3\r\n\r\nShared publisher explanation."
        );
        assert_eq!(
            library.read_ayah(0, 2, 286).unwrap().unwrap(),
            "Publisher explanation 2:286 & notes."
        );
        let manifest = library.manifest(0).unwrap();
        assert_eq!(manifest.chapters.len(), 114);
        assert_eq!(COUNTS.iter().sum::<u32>(), 6236);
        let book = library.book_path(0).unwrap();
        let unknown = book.join("my-notes.txt");
        fs::write(&unknown, b"User notes").unwrap();
        assert!(library.remove(0, &JobState::default()).is_err());
        assert!(book.join("pack.json").exists());
        assert_eq!(fs::read(&unknown).unwrap(), b"User notes");
        fs::remove_file(unknown).unwrap();
        let chapter = book.join(&manifest.chapters[1].file);
        let original = fs::read(&chapter).unwrap();
        fs::write(&chapter, b"changed").unwrap();
        assert!(library.remove(0, &JobState::default()).is_err());
        assert!(book.join(&manifest.chapters[0].file).exists());
        assert_eq!(fs::read(&chapter).unwrap(), b"changed");
        fs::write(&chapter, original).unwrap();
        library.remove(0, &JobState::default()).unwrap();
        assert!(library.installed_ids().unwrap().is_empty());
        assert_eq!(library.read_ayah(0, 1, 2).unwrap(), None);
    }
    #[test]
    fn empty_chapter_placeholders_use_only_explicit_publisher_ayah_mappings() {
        let publisher = Publisher {
            shared_first: true,
            empty_shared_rows: true,
            ..Default::default()
        };
        let study = fetch_chapter(&publisher, 3, 1, &JobState::default()).unwrap();
        validate_chapter(&study, 1).unwrap();
        assert_eq!(study.lookup.len(), 7);
        assert_eq!(
            data::study_text(&study, false, 2),
            "2–3\r\n\r\nShared publisher explanation."
        );
        assert_eq!(
            data::study_text(&study, false, 3),
            "2–3\r\n\r\nShared publisher explanation."
        );
        // One chapter request and one exact requested-ayah response explicitly
        // supply both placeholders. No inferred neighboring ranges are needed.
        assert_eq!(publisher.calls.load(Ordering::Relaxed), 2);
        let response = |rows: Value| {
            Response(serde_json::to_vec(&serde_json::json!({"tafsirs":rows,"pagination":{"current_page":1,"next_page":null,"total_pages":1,"per_page":50}})).unwrap())
        };
        let empty = serde_json::json!({"resource_id":93,"verse_key":"1:2","text":""});
        assert!(
            fetch_chapter(
                &response(serde_json::json!([empty, empty])),
                3,
                1,
                &JobState::default()
            )
            .err()
            .unwrap()
            .to_string()
            .contains("duplicate ayah 2")
        );
        assert!(
            fetch_chapter(
                &response(serde_json::json!([{"resource_id":14,"verse_key":"1:2","text":""}])),
                3,
                1,
                &JobState::default()
            )
            .is_err()
        );
        assert!(
            fetch_chapter(
                &response(serde_json::json!([{"resource_id":93,"verse_key":"2:2","text":""}])),
                3,
                1,
                &JobState::default()
            )
            .is_err()
        );
        // An empty chapter row alone cannot establish absence. The exact
        // by_ayah schema/resource/reference checks must still succeed.
        let error = fetch_chapter(
            &response(serde_json::json!([empty])),
            3,
            1,
            &JobState::default(),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("/tafsirs/93/by_ayah/1:1"));
    }
    #[test]
    fn identical_independent_commentary_does_not_infer_a_shared_group() {
        let rows: Vec<_> = (1..=7).map(|ayah|serde_json::json!({"resource_id":93,"verse_key":format!("1:{ayah}"),"text":if [2,4].contains(&ayah) { "Identical publisher notes.".to_owned() } else {format!("Publisher notes {ayah}.")} })).collect();
        let response = Response(serde_json::to_vec(&serde_json::json!({"tafsirs":rows,"pagination":{"current_page":1,"next_page":null,"total_pages":1,"per_page":50}})).unwrap());
        let study = fetch_chapter(&response, 3, 1, &JobState::default()).unwrap();
        assert_eq!(study.groups.len(), 7);
        assert_ne!(study.lookup["2"], study.lookup["4"]);
        for ayah in [2, 4] {
            assert_eq!(
                data::study_text(&study, false, ayah),
                "Identical publisher notes."
            );
            let key = study.lookup[&ayah.to_string()].as_str().unwrap();
            assert_eq!(study.groups[key].ayah_keys, vec![ayah]);
        }
    }
    #[test]
    fn shared_groups_retain_noncontiguous_publisher_references_without_inferred_ranges() {
        struct ExplicitPublisher;
        impl Transport for ExplicitPublisher {
            fn fetch(&self, request: Request, _state: &JobState) -> Result<Vec<u8>> {
                let response = match request {
                    Request::Chapter {
                        source: 3,
                        surah: 1,
                        page: 1,
                    } => {
                        let rows:Vec<_>=(1..=7).map(|ayah|serde_json::json!({"resource_id":93,"verse_key":format!("1:{ayah}"),"text":if [2,4].contains(&ayah) {String::new()}else{format!("Publisher notes {ayah}.")}})).collect();
                        serde_json::json!({"tafsirs":rows,"pagination":{"current_page":1,"next_page":null,"total_pages":1,"per_page":50}})
                    }
                    Request::Ayah {
                        source: 3,
                        surah: 1,
                        ayah: 2,
                    } => {
                        serde_json::json!({"tafsir":{"resource_id":93,"text":"Exact shared publisher notes.","verses":{"1:2":{"id":2},"1:4":{"id":4}}}})
                    }
                    _ => return Err("Unexpected explicit-reference test request".into()),
                };
                Ok(serde_json::to_vec(&response)?)
            }
        }
        let study = fetch_chapter(&ExplicitPublisher, 3, 1, &JobState::default()).unwrap();
        validate_chapter(&study, 1).unwrap();
        assert_eq!(study.groups.len(), 6);
        assert_eq!(study.lookup["2"], study.lookup["4"]);
        let group = study
            .groups
            .get(study.lookup["2"].as_str().unwrap())
            .unwrap();
        assert_eq!(group.ayah_keys, vec![2, 4]);
        assert_eq!(group.text, "Exact shared publisher notes.");
        for ayah in [2, 4] {
            assert_eq!(
                data::study_text(&study, false, ayah),
                "2, 4\r\n\r\nExact shared publisher notes."
            );
        }
        assert_eq!(data::study_text(&study, false, 3), "Publisher notes 3.");
    }
    #[test]
    fn cancelled_download_never_publishes_a_partial_book() {
        let profile = Profile::new();
        let library = Library::with_transport(
            &profile.0,
            Arc::new(Publisher {
                cancel_after: 3,
                ..Default::default()
            }),
        )
        .unwrap();
        let state = JobState::default();
        assert!(download(&library, 0, &state).is_err());
        assert!(library.installed_ids().unwrap().is_empty());
        assert_eq!(
            fs::read_dir(library.root.join("Staging")).unwrap().count(),
            0
        );
    }
    #[test]
    fn failed_stage_writes_do_not_strand_previously_downloaded_chapters() {
        let profile = Profile::new();
        let library = Library::with_transport(&profile.0, Arc::new(Publisher::default())).unwrap();
        for fail_after_complete_write in [false, true] {
            let mut stage = Stage::new(&library.root).unwrap();
            let completed_name = format!(
                "s001-{}.json",
                sha256(b"Previously downloaded commentary").unwrap()
            );
            let failed_name = format!("s002-{}.json", sha256(b"Next downloaded chapter").unwrap());
            stage
                .write(&completed_name, b"Previously downloaded commentary")
                .unwrap();
            let path = stage.path.clone();
            let result =
                stage.write_with(&failed_name, b"Next downloaded chapter", |file, bytes| {
                    file.write_all(if fail_after_complete_write {
                        bytes
                    } else {
                        &bytes[..3]
                    })?;
                    Err(std::io::Error::new(
                        std::io::ErrorKind::StorageFull,
                        "Injected storage failure",
                    ))
                });
            assert!(result.is_err());
            assert!(!path.join(&failed_name).exists());
            assert_eq!(
                fs::read(path.join(&completed_name)).unwrap(),
                b"Previously downloaded commentary"
            );
            drop(stage);
            assert!(!path.exists());
        }
        let mut stage = Stage::new(&library.root).unwrap();
        let existing = stage.path.join("existing.json");
        fs::write(&existing, b"Unrecognized file").unwrap();
        assert!(stage.write("existing.json", b"Replacement").is_err());
        assert_eq!(fs::read(&existing).unwrap(), b"Unrecognized file");
        drop(stage);
        assert_eq!(fs::read(existing).unwrap(), b"Unrecognized file");
    }
    fn interrupted_stage(library: &Library) -> (PathBuf, String) {
        let mut stage = Stage::new(&library.root).unwrap();
        let bytes = b"Verified publisher commentary fixture";
        let name = format!("s001-{}.json", sha256(bytes).unwrap());
        stage.write(&name, bytes).unwrap();
        let old_path = stage.path.clone();
        // Simulate process termination without in-memory Drop cleanup, using a
        // nonexistent owner PID so active-process preservation is exercised.
        std::mem::forget(stage);
        let path = library.root.join("Staging").join(format!(
            "{}-{}.partial",
            u32::MAX,
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        move_new(&old_path, &path).unwrap();
        let mut journal: StageJournal =
            serde_json::from_slice(&fs::read(path.join(STAGE_JOURNAL)).unwrap()).unwrap();
        journal.stage = path.file_name().unwrap().to_str().unwrap().into();
        fs::write(
            path.join(STAGE_JOURNAL),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        (path, name)
    }
    #[test]
    fn interrupted_stages_recover_on_worker_and_preserve_cancelled_recovery() {
        let profile = Profile::new();
        let library = Library::with_transport(&profile.0, Arc::new(Publisher::default())).unwrap();
        let (path, _) = interrupted_stage(&library);
        let reloaded = Library::with_transport(&profile.0, Arc::new(Publisher::default())).unwrap();
        assert!(
            path.exists(),
            "UI initialization must not hash or remove old stages"
        );
        let state = JobState::default();
        state.cancel.store(true, Ordering::Release);
        assert!(recover_stages(&reloaded.root, &state).is_err());
        assert!(path.exists());
        recover_stages(&reloaded.root, &JobState::default()).unwrap();
        assert!(!path.exists());
        assert!(reloaded.installed_ids().unwrap().is_empty());
        let (half_cleaned, file) = interrupted_stage(&reloaded);
        fs::remove_file(half_cleaned.join(file)).unwrap();
        recover_stages(&reloaded.root, &JobState::default()).unwrap();
        assert!(!half_cleaned.exists());
        let mut active = Stage::new(&library.root).unwrap();
        active.write("pack.json", b"Active transaction").unwrap();
        recover_stages(&reloaded.root, &JobState::default()).unwrap();
        assert_eq!(
            fs::read(active.path.join("pack.json")).unwrap(),
            b"Active transaction"
        );
        drop(active);
        // The small window after journal removal recovers via the complete
        // publisher manifest; an unfinished manifest never authorizes removal.
        download(&reloaded, 0, &JobState::default()).unwrap();
        let completed_stage = reloaded.root.join("Staging").join(format!(
            "{}-{}.partial",
            u32::MAX,
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        move_new(&reloaded.book_path(0).unwrap(), &completed_stage).unwrap();
        recover_stages(&reloaded.root, &JobState::default()).unwrap();
        assert!(!completed_stage.exists());
    }
    #[test]
    fn recovery_preserves_entire_unknown_modified_and_malformed_stages() {
        let profile = Profile::new();
        let library = Library::with_transport(&profile.0, Arc::new(Publisher::default())).unwrap();
        let (modified, file) = interrupted_stage(&library);
        fs::write(modified.join(&file), b"Changed notes").unwrap();
        let (unknown, original) = interrupted_stage(&library);
        fs::write(unknown.join("my-notes.txt"), b"User notes").unwrap();
        let (malformed, intact) = interrupted_stage(&library);
        fs::write(malformed.join(STAGE_JOURNAL), b"{ broken journal").unwrap();
        let (oversized, saved) = interrupted_stage(&library);
        let journal = OpenOptions::new()
            .write(true)
            .open(oversized.join(STAGE_JOURNAL))
            .unwrap();
        journal.set_len(JOURNAL_LIMIT + 1).unwrap();
        drop(journal);
        let (valid, _) = interrupted_stage(&library);
        recover_stages(&library.root, &JobState::default()).unwrap();
        assert!(!valid.exists());
        assert_eq!(fs::read(modified.join(file)).unwrap(), b"Changed notes");
        assert_eq!(
            fs::read(unknown.join("my-notes.txt")).unwrap(),
            b"User notes"
        );
        assert!(unknown.join(original).exists());
        assert!(malformed.join(intact).exists());
        assert!(oversized.join(saved).exists());
        assert_eq!(
            fs::metadata(oversized.join(STAGE_JOURNAL)).unwrap().len(),
            JOURNAL_LIMIT + 1
        );
    }
    #[test]
    fn staging_journal_paths_counts_sizes_and_directory_links_are_bounded() {
        let profile = Profile::new();
        let library = Library::with_transport(&profile.0, Arc::new(Publisher::default())).unwrap();
        let (path, _) = interrupted_stage(&library);
        let stage_name = path.file_name().unwrap().to_str().unwrap();
        let bytes = fs::read(path.join(STAGE_JOURNAL)).unwrap();
        let mut journal: StageJournal = serde_json::from_slice(&bytes).unwrap();
        validate_stage_journal(&journal, stage_name).unwrap();
        journal.files[0].file = "../user-notes.json".into();
        assert!(validate_stage_journal(&journal, stage_name).is_err());
        assert!(validate_stage_file("C:\\user-notes.json", 8, &"a".repeat(64)).is_err());
        journal = serde_json::from_slice(&bytes).unwrap();
        journal.files[0].bytes = CHAPTER_LIMIT + 1;
        assert!(validate_stage_journal(&journal, stage_name).is_err());
        journal = serde_json::from_slice(&bytes).unwrap();
        journal.files.push(StageReceipt {
            file: journal.files[0].file.clone(),
            bytes: journal.files[0].bytes,
            sha256: journal.files[0].sha256.clone(),
        });
        assert!(validate_stage_journal(&journal, stage_name).is_err());
        journal.files.extend((0..115).map(|_| StageReceipt {
            file: "pack.json".into(),
            bytes: 1,
            sha256: "a".repeat(64),
        }));
        assert!(validate_stage_journal(&journal, stage_name).is_err());
        journal = serde_json::from_slice(&bytes).unwrap();
        journal.stage = "../outside.partial".into();
        assert!(validate_stage_journal(&journal, stage_name).is_err());
        journal = serde_json::from_slice(&bytes).unwrap();
        for surah in 2..=4 {
            let hash = "a".repeat(64);
            journal.files.push(StageReceipt {
                file: format!("s{surah:03}-{hash}.json"),
                bytes: CHAPTER_LIMIT,
                sha256: hash,
            });
        }
        assert!(validate_stage_journal(&journal, stage_name).is_err());
        assert!(!valid_stage_name("0-1.partial"));
        assert!(!valid_stage_name("01-1.partial"));
        assert!(!valid_stage_name("1-0.partial"));
        assert!(!valid_stage_name("1-1/../../outside.partial"));
        // Reparse fixture creation may require Windows Developer Mode. When
        // available, recovery must preserve both the link and its outside data.
        let outside = profile.0.join("outside-notes");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("notes.txt"), b"Keep outside notes").unwrap();
        let link = library.root.join("Staging").join(format!(
            "{}-{}.partial",
            u32::MAX,
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        if std::os::windows::fs::symlink_dir(&outside, &link).is_ok() {
            assert!(regular_directory(&link).is_err());
            recover_stages(&library.root, &JobState::default()).unwrap();
            assert_eq!(
                fs::read(outside.join("notes.txt")).unwrap(),
                b"Keep outside notes"
            );
            fs::remove_dir(&link).unwrap();
        }
        // Entry enumeration remains bounded even for zero-byte unknown files.
        for n in 0..=MAX_STAGING_ENTRIES {
            fs::write(
                library
                    .root
                    .join("Staging")
                    .join(format!("unknown-{n}.txt")),
                b"",
            )
            .unwrap();
        }
        assert!(recover_stages(&library.root, &JobState::default()).is_err());
        assert!(outside.join("notes.txt").exists());
    }
    #[test]
    fn untrusted_existing_targets_and_modified_staging_are_preserved() {
        let profile = Profile::new();
        let library = Library::with_transport(&profile.0, Arc::new(Publisher::default())).unwrap();
        let target = library.book_path(0).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("notes.txt"), b"Keep this").unwrap();
        assert!(download(&library, 0, &JobState::default()).is_err());
        assert_eq!(fs::read(target.join("notes.txt")).unwrap(), b"Keep this");
        let mut stage = Stage::new(&library.root).unwrap();
        stage.write("pack.json", b"owned").unwrap();
        fs::write(stage.path.join("pack.json"), b"changed").unwrap();
        let path = stage.path.clone();
        drop(stage);
        assert_eq!(fs::read(path.join("pack.json")).unwrap(), b"changed");
    }
    #[test]
    fn provider_boundaries_reject_wrong_sources_duplicates_and_guessed_ranges() {
        let state = JobState::default();
        let row = serde_json::json!({"resource_id":14,"verse_key":"1:1","text":"Publisher notes"});
        let page = |rows: Value| {
            serde_json::to_vec(&serde_json::json!({"tafsirs":rows,"pagination":{"current_page":1,"next_page":null,"total_pages":1,"per_page":50}})).unwrap()
        };
        assert!(
            fetch_chapter(&Response(page(serde_json::json!([row, row]))), 0, 1, &state).is_err()
        );
        assert!(fetch_chapter(&Response(page(serde_json::json!([{"resource_id":15,"verse_key":"1:1","text":"Wrong publisher"}]))),0,1,&state).is_err());
        assert!(
            fetch_chapter(
                &Response(page(
                    serde_json::json!([{"resource_id":14,"verse_key":"2:1","text":"Wrong chapter"}])
                )),
                0,
                1,
                &state
            )
            .is_err()
        );
        assert!(fetch_chapter(&Response(page(serde_json::json!([]))), 0, 1, &state).is_err());
        assert!(canonical_reference("01:1", 1).is_err());
        assert!(canonical_reference("1:8", 1).is_err());
        assert!(
            Request::Chapter {
                source: 0,
                surah: 1,
                page: 8
            }
            .path()
            .is_err()
        );
        assert!(
            Request::Ayah {
                source: SOURCES.len(),
                surah: 1,
                ayah: 1
            }
            .path()
            .is_err()
        );
        assert_eq!(
            Request::Chapter {
                source: 0,
                surah: 1,
                page: 1
            }
            .path()
            .unwrap(),
            "/api/v4/tafsirs/14/by_chapter/1?per_page=50&page=1"
        );
    }
    #[test]
    fn responses_and_receipts_have_independent_size_and_path_limits() {
        assert!(response_headers(302, "application/json", "", "10").is_err());
        assert!(response_headers(200, "text/html", "", "10").is_err());
        assert!(response_headers(200, "application/json", "gzip", "10").is_err());
        assert!(response_headers(200, "application/json", "", "8388609").is_err());
        assert!(response_json::<Value>(&vec![b' '; RESPONSE_LIMIT + 1]).is_err());
        assert_eq!(
            response_headers(200, "application/json; charset=utf-8", "identity", "10").unwrap(),
            Some(10)
        );
        let receipts: Vec<_> = (1..=114)
            .map(|surah| {
                let hash = "a".repeat(64);
                Receipt {
                    surah,
                    file: format!("s{surah:03}-{hash}.json"),
                    bytes: 10,
                    sha256: hash,
                }
            })
            .collect();
        let mut manifest = Manifest {
            schema: SCHEMA.into(),
            resource: 14,
            verses: 6236,
            bytes: 1140,
            chapters: receipts,
        };
        validate_manifest(&manifest, 0).unwrap();
        manifest.chapters[0].file = "../outside.json".into();
        assert!(validate_manifest(&manifest, 0).is_err());
        manifest.chapters.pop();
        assert!(validate_manifest(&manifest, 0).is_err());
    }
    #[test]
    fn cache_budget_and_nonregular_paths_are_bounded() {
        let profile = Profile::new();
        fs::write(profile.0.join("TafsirLibrary"), b"Not a directory").unwrap();
        assert!(Library::with_transport(&profile.0, Arc::new(Publisher::default())).is_err());
        let mut cache = Cache::default();
        for surah in 1..=10 {
            cache.insert(
                0,
                surah,
                1024 * 1024,
                Arc::new(Study {
                    lookup: BTreeMap::new(),
                    groups: BTreeMap::new(),
                }),
            );
        }
        assert_eq!(cache.chapters.len(), 4);
        assert!(cache.bytes <= CACHE_LIMIT);
        cache.invalidate(0);
        assert!(cache.chapters.is_empty());
    }
    #[test]
    fn hafs_counts_match_bundled_source_metadata() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chapters: Vec<data::Surah> =
            data::read_json(&root.join("data/qul/processed/surahs.json")).unwrap();
        assert_eq!(
            chapters
                .iter()
                .map(|chapter| chapter.verses_count)
                .collect::<Vec<_>>(),
            COUNTS
        );
    }
}
