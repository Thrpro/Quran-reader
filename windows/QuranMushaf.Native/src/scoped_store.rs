//! Bounded, disposable publisher bundles. Persistent recordings are read-only.
//! The worker alone hashes/retires owned files; Ready drops only lease counters.
use crate::{
    data::Result,
    mp3,
    owned_file::{self, DeleteOutcome, FileIdentity, PinnedDirectory},
    timed_audio::Recording,
    timed_store::{self, Ready, Task},
};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const QUOTA: u64 = 512 * 1024 * 1024;
const RESERVE: u64 = mp3::MAX_CHAPTER_BYTES + 3 * 1024 * 1024;
const LEDGER_LIMIT: usize = 64 * 1024;
const MAX_SESSIONS: usize = 32;
const MAX_BUNDLES: usize = 8;
const NAMES: [&str; 4] = [
    "catalogue.json",
    "timings.json",
    "audio.mp3",
    "receipt.json",
];
static NONCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owned {
    name: String,
    identity: FileIdentity,
    bytes: u64,
    sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    serial: u64,
    directory: FileIdentity,
    reciter: usize,
    surah: u32,
    complete: bool,
    files: Vec<Owned>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    schema: String,
    directory: FileIdentity,
    owner: FileIdentity,
    records: Vec<Record>,
}
impl Ledger {
    fn validate(&self, directory: FileIdentity, owner: FileIdentity) -> Result<()> {
        if self.schema != "native-scoped-continuous-v1"
            || self.directory != directory
            || self.owner != owner
            || self.records.len() > MAX_BUNDLES
        {
            return Err("Unrecognized scoped recording ownership; preserving its files".into());
        }
        let mut serials = std::collections::BTreeSet::new();
        for record in &self.records {
            if record.serial == 0
                || record.serial > 1_000_000_000
                || !serials.insert(record.serial)
                || !(1..=114).contains(&record.surah)
                || Recording::for_reciter(record.reciter).is_none()
                || record.files.len() > 4
                || (record.complete && record.files.len() != 4)
            {
                return Err("Invalid scoped recording ownership; preserving its files".into());
            }
            let mut names = std::collections::BTreeSet::new();
            for file in &record.files {
                let max = if file.name == "audio.mp3" {
                    mp3::MAX_CHAPTER_BYTES
                } else {
                    1024 * 1024
                };
                if !NAMES.contains(&file.name.as_str())
                    || !names.insert(&file.name)
                    || file.bytes > max
                    || file.sha256.len() != 64
                    || !file
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err("Invalid scoped file ownership; preserving its files".into());
                }
            }
        }
        Ok(())
    }
}
struct Bundle {
    record: Record,
    pin: Option<PinnedDirectory>,
    leases: Arc<AtomicUsize>,
    blocked: bool,
}
pub(crate) struct Lease {
    session: Arc<AtomicUsize>,
    bundle: Option<Arc<AtomicUsize>>,
    _borrowed: Option<PinnedDirectory>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(bundle) = &self.bundle {
            bundle.fetch_sub(1, Ordering::Release);
        }
        self.session.fetch_sub(1, Ordering::Release);
    }
}
#[derive(Default)]
struct Seals {
    files: Vec<Owned>,
    error: Option<String>,
}
/// A scoped output seals the exact creation handle before releasing it. Partial
/// downloads can therefore be retired without adopting later replacement bytes.
pub(crate) struct Output {
    file: Option<File>,
    owned: Option<(String, FileIdentity, Rc<RefCell<Seals>>)>,
}
impl Output {
    pub(crate) fn persistent(file: File) -> Self {
        Self {
            file: Some(file),
            owned: None,
        }
    }
    pub(crate) fn sync_all(&self) -> std::io::Result<()> {
        self.file.as_ref().unwrap().sync_all()
    }
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.file.as_mut().unwrap().write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.as_mut().unwrap().flush()
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        if let Some((name, expected, seals)) = &self.owned {
            let result: Result<(FileIdentity, u64, String)> = self
                .file
                .as_mut()
                .unwrap()
                .sync_all()
                .map_err(Into::into)
                .and_then(|()| owned_file::fingerprint_file(self.file.as_mut().unwrap()));
            let mut seals = seals.borrow_mut();
            match result {
                Ok((identity, bytes, sha256)) if identity == *expected => {
                    seals.files.push(Owned {
                        name: name.clone(),
                        identity,
                        bytes,
                        sha256,
                    });
                }
                Ok(_) => {
                    seals.error =
                        Some("Scoped creation identity changed; preserving the file".into())
                }
                Err(error) => seals.error = Some(error.to_string()),
            }
        }
    }
}

pub(crate) struct Session {
    _audio: PinnedDirectory,
    namespace: PinnedDirectory,
    persistent: Option<PinnedDirectory>,
    directory: Option<PinnedDirectory>,
    owner: Option<File>,
    owner_identity: FileIdentity,
    bundles: Vec<Bundle>,
    leases: Arc<AtomicUsize>,
    next_serial: u64,
    recovered: bool,
}
fn linked(path: &Path) -> Result<bool> {
    Ok(fs::symlink_metadata(path)?.file_attributes() & 0x400 != 0)
}
fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
fn global_lock(namespace: &PinnedDirectory, task: Option<&Task>) -> Result<File> {
    global_lock_wait(namespace, task, task.is_some())
}
fn global_lock_wait(namespace: &PinnedDirectory, task: Option<&Task>, wait: bool) -> Result<File> {
    let path = namespace.path().join("namespace.lock");
    let began = Instant::now();
    loop {
        if task.is_some_and(|task| task.cancelled.load(Ordering::Acquire)) {
            return Err("Recording preparation cancelled".into());
        }
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .custom_flags(0x00200000)
            .open(&path)
        {
            Ok(file) => {
                owned_file::identity_file(&file)?;
                return Ok(file);
            }
            Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => {
                if !wait || began.elapsed() > Duration::from_secs(30) {
                    return Err("Another scoped recording is being prepared; retry shortly".into());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error.into()),
        }
    }
}
// Pin each directory while traversing, reject links, and bound zero-byte clutter
// as well as bytes. Unknown content is counted, never traversed for deletion.
fn usage(namespace: &PinnedDirectory) -> Result<(u64, usize)> {
    fn walk(pin: &PinnedDirectory, depth: usize, visited: &mut usize) -> Result<u64> {
        if depth > 3 {
            return Err("Unexpected scoped storage nesting; files are preserved".into());
        }
        let mut bytes = 0u64;
        for entry in fs::read_dir(pin.path())? {
            let entry = entry?;
            *visited += 1;
            if *visited > 4096 {
                return Err("Scoped storage contains too many files; preserving them".into());
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if linked(&path)? {
                return Err("Scoped storage contains a link; preserving it".into());
            }
            let size = if metadata.is_dir() {
                walk(&PinnedDirectory::open(&path)?, depth + 1, visited)?
            } else if entry.file_name() == "namespace.lock" && depth == 0 {
                // The caller holds this exclusive handle and has already
                // checked its link/stream identity in global_lock.
                metadata.len()
            } else {
                let file = OpenOptions::new()
                    .read(true)
                    .share_mode(3)
                    .custom_flags(0x00200000)
                    .open(&path)?;
                // Permit a live owner's metadata writer, but never ignore a
                // hardlink, reparse point or unaccounted alternate stream.
                owned_file::identity_file(&file)?;
                file.metadata()?.len()
            };
            bytes = bytes
                .checked_add(size)
                .ok_or("Scoped storage byte count overflow")?;
        }
        Ok(bytes)
    }
    let sessions = fs::read_dir(namespace.path())?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("session-"))
        .count();
    Ok((walk(namespace, 0, &mut 0)?, sessions))
}
fn bundle_path(directory: &PinnedDirectory, serial: u64) -> PathBuf {
    directory.path().join(format!("bundle-{serial:09}"))
}
impl Session {
    pub(crate) fn new(audio_root: &Path, task: Option<&Task>) -> Result<Self> {
        let audio = PinnedDirectory::open(audio_root)?;
        let root = audio.path().join("ScopedContinuous");
        if !exists(&root)? {
            fs::create_dir(&root)?;
        }
        let namespace = PinnedDirectory::open(&root)?;
        let _lock = global_lock(&namespace, task)?;
        if usage(&namespace)?.0 > QUOTA {
            return Err(
                "Scoped recording storage exceeds its quota; existing files are preserved".into(),
            );
        }
        // The factory is called on the worker. Recover recognized inactive
        // owners before enforcing the root cap, without UI hashing/deletion.
        recover_namespace(&namespace, None, task)?;
        // Metadata accounting does not read recording payloads. Reserve the
        // complete bounded ledger even before the first worker request.
        let (bytes, roots) = usage(&namespace)?;
        if roots >= MAX_SESSIONS || bytes.saturating_add(LEDGER_LIMIT as u64) > QUOTA {
            return Err(
                "Scoped recording storage needs recovery; existing files are preserved".into(),
            );
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = namespace.path().join(format!(
            "session-{}-{stamp}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        let directory = PinnedDirectory::open(&path)?;
        let (owner, owner_identity) = owned_file::create_new(&directory.path().join("owner.lock"))?;
        let persistent_path = audio.path().join("Continuous");
        let persistent = if exists(&persistent_path)? {
            Some(PinnedDirectory::open(&persistent_path)?)
        } else {
            None
        };
        let mut session = Self {
            _audio: audio,
            namespace,
            persistent,
            directory: Some(directory),
            owner: Some(owner),
            owner_identity,
            bundles: Vec::new(),
            leases: Arc::new(AtomicUsize::new(0)),
            next_serial: 1,
            recovered: false,
        };
        session.persist()?;
        drop(_lock);
        Ok(session)
    }
    fn persist(&mut self) -> Result<()> {
        let ledger = Ledger {
            schema: "native-scoped-continuous-v1".into(),
            directory: self.directory.as_ref().unwrap().identity(),
            owner: self.owner_identity,
            records: self
                .bundles
                .iter()
                .map(|bundle| bundle.record.clone())
                .collect(),
        };
        let bytes = serde_json::to_vec(&ledger)?;
        if bytes.len() > LEDGER_LIMIT {
            return Err("Scoped ownership ledger exceeds its limit".into());
        }
        let owner = self.owner.as_mut().unwrap();
        owner.seek(SeekFrom::Start(0))?;
        owner.write_all(&bytes)?;
        owner.set_len(bytes.len() as u64)?;
        owner.sync_all()?;
        Ok(())
    }
    fn lease(&self, bundle: Option<Arc<AtomicUsize>>, pin: Option<PinnedDirectory>) -> Lease {
        self.leases.fetch_add(1, Ordering::Relaxed);
        if let Some(counter) = &bundle {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        Lease {
            session: self.leases.clone(),
            bundle,
            _borrowed: pin,
        }
    }
    pub(crate) fn has_leases(&self) -> bool {
        self.leases.load(Ordering::Acquire) != 0
    }
    pub(crate) fn prepare(&mut self, task: &Task) -> Result<Ready> {
        let _lock = global_lock(&self.namespace, Some(task))?;
        if !self.recovered {
            self.recover_inactive()?;
            self.recovered = true;
        }
        self.retire_locked(Some((task.reciter, task.surah)))?;
        let recording = Recording::for_reciter(task.reciter).ok_or("Unknown recording")?;
        if let Some(persistent) = &self.persistent {
            let path = persistent
                .path()
                .join(format!("r{}-s{:03}", recording.read, task.surah));
            if exists(&path)? {
                let pin = PinnedDirectory::open(&path)?;
                let mut ready = timed_store::validate(pin.path(), task)?;
                ready._scoped = Some(self.lease(None, Some(pin)));
                return Ok(ready);
            }
        }
        if let Some(index) = self.bundles.iter().position(|bundle| {
            bundle.record.reciter == task.reciter
                && bundle.record.surah == task.surah
                && bundle.record.complete
                && !bundle.blocked
        }) {
            let bundle = &self.bundles[index];
            let mut ready = validate_owned(bundle, task)?;
            ready._scoped = Some(self.lease(Some(bundle.leases.clone()), None));
            return Ok(ready);
        }
        let retirement_wait = Instant::now();
        let (bytes, roots) = loop {
            let usage = usage(&self.namespace)?;
            if usage.0.saturating_add(RESERVE) <= QUOTA
                || retirement_wait.elapsed() >= Duration::from_secs(10)
                || !self.bundles.iter().any(|bundle| {
                    !bundle.blocked
                        && bundle.leases.load(Ordering::Acquire) == 0
                        && (bundle.record.reciter, bundle.record.surah)
                            != (task.reciter, task.surah)
                })
            {
                break usage;
            }
            // Only a decoder's independently held read handle remains here.
            // Never wait for active Ready leases or unknown/modified content.
            if task.cancelled.load(Ordering::Acquire) {
                return Err("Recording preparation cancelled".into());
            }
            std::thread::sleep(Duration::from_millis(50));
            self.retire_locked(Some((task.reciter, task.surah)))?;
        };
        if bytes.saturating_add(RESERVE) > QUOTA
            || roots > MAX_SESSIONS
            || self.bundles.len() >= MAX_BUNDLES
        {
            return Err("Scoped recording cache has insufficient space; existing and unreleased files are preserved".into());
        }
        let serial = self.next_serial;
        self.next_serial = serial
            .checked_add(1)
            .ok_or("Scoped recording sequence overflow")?;
        let path = bundle_path(self.directory.as_ref().unwrap(), serial);
        fs::create_dir(&path)?;
        let pin = PinnedDirectory::open(&path)?;
        let seals = Rc::new(RefCell::new(Seals::default()));
        let result = timed_store::download_bundle(pin.path(), task, |path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("Invalid scoped file name")?;
            if !NAMES.contains(&name) || path.parent() != Some(pin.path()) {
                return Err("Scoped output is outside its owned bundle".into());
            }
            let (file, identity) = owned_file::create_new(path)?;
            Ok(Output {
                file: Some(file),
                owned: Some((name.into(), identity, seals.clone())),
            })
        });
        let sealed = seals.borrow();
        let complete = result.is_ok() && sealed.error.is_none() && sealed.files.len() == 4;
        let record = Record {
            serial,
            directory: pin.identity(),
            reciter: task.reciter,
            surah: task.surah,
            complete,
            files: sealed.files.clone(),
        };
        let sealing_error = sealed.error.clone();
        drop(sealed);
        self.bundles.push(Bundle {
            record,
            pin: Some(pin),
            leases: Arc::new(AtomicUsize::new(0)),
            blocked: false,
        });
        // Save even a cancelled, sealed partial before cleanup. A crash while
        // this metadata is written leaves unrecognized state preserved.
        self.persist()?;
        result?;
        if let Some(error) = sealing_error {
            return Err(error.into());
        }
        let bundle = self.bundles.last().unwrap();
        let mut ready = validate_owned(bundle, task)?;
        ready._scoped = Some(self.lease(Some(bundle.leases.clone()), None));
        Ok(ready)
    }
    fn retire_locked(&mut self, keep: Option<(usize, u32)>) -> Result<()> {
        let directory = self.directory.as_ref().unwrap();
        let mut changed = false;
        for index in (0..self.bundles.len()).rev() {
            let bundle = &mut self.bundles[index];
            if bundle.blocked
                || bundle.leases.load(Ordering::Acquire) != 0
                || keep == Some((bundle.record.reciter, bundle.record.surah))
            {
                continue;
            }
            let path = bundle_path(directory, bundle.record.serial);
            match retire_bundle(&path, bundle) {
                Ok(true) => {
                    self.bundles.remove(index);
                    changed = true;
                }
                Ok(false) => {}
                Err(_) => {
                    bundle.blocked = true;
                }
            }
        }
        if changed {
            self.persist()?;
        }
        Ok(())
    }
    pub(crate) fn retire_released(&mut self) -> Result<()> {
        let _lock = global_lock(&self.namespace, None)?;
        self.retire_locked(None)
    }
    fn recover_inactive(&mut self) -> Result<()> {
        recover_namespace(
            &self.namespace,
            Some(self.directory.as_ref().unwrap().path()),
            None,
        )
    }
    pub(crate) fn close(&mut self) -> Result<()> {
        let _lock = global_lock_wait(&self.namespace, None, true)?;
        self.retire_locked(None)?;
        let began = Instant::now();
        while self
            .bundles
            .iter()
            .any(|bundle| !bundle.blocked && bundle.leases.load(Ordering::Acquire) == 0)
            && began.elapsed() < Duration::from_secs(10)
        {
            std::thread::sleep(Duration::from_millis(50));
            self.retire_locked(None)?;
        }
        if !self.bundles.is_empty() {
            return Ok(());
        }
        let (identity, bytes, hash) = owned_file::fingerprint_file(self.owner.as_mut().unwrap())?;
        let directory = self.directory.as_ref().unwrap();
        // Unexpected files prevent owner removal: retain the recovery marker.
        if fs::read_dir(directory.path())?.count() != 1 {
            return Ok(());
        }
        let path = directory.path().to_owned();
        let directory_identity = directory.identity();
        let owner_path = path.join("owner.lock");
        self.owner.take();
        if owned_file::delete_if_unchanged(&owner_path, identity, &hash, bytes)?
            != DeleteOutcome::Deleted
        {
            return Ok(());
        }
        self.directory.take();
        let began = Instant::now();
        while owned_file::delete_empty_directory(&path, directory_identity)? == DeleteOutcome::Busy
            && began.elapsed() < Duration::from_secs(2)
        {
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    }
}
fn validate_owned(bundle: &Bundle, task: &Task) -> Result<Ready> {
    let pin = bundle
        .pin
        .as_ref()
        .ok_or("Owned recording has no directory lease")?;
    if pin.identity() != bundle.record.directory || !bundle.record.complete {
        return Err("Owned recording identity changed; preserving it".into());
    }
    let mut metadata_leases = Vec::new();
    for expected in &bundle.record.files {
        if expected.name == "audio.mp3" {
            continue;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(pin.path().join(&expected.name))?;
        let (identity, bytes, hash) = owned_file::fingerprint_file(&mut file)?;
        if identity != expected.identity || bytes != expected.bytes || hash != expected.sha256 {
            return Err("Owned recording metadata changed; preserving it".into());
        }
        metadata_leases.push(file);
    }
    // These read-only leases prevent a metadata replacement between the seal
    // check and the authentic timing/catalogue parser.
    let ready = timed_store::validate(pin.path(), task)?;
    let audio = bundle
        .record
        .files
        .iter()
        .find(|file| file.name == "audio.mp3")
        .ok_or("Owned recording has no original audio seal")?;
    if owned_file::identity_file(&ready._verified._lease)? != audio.identity
        || ready._verified.info.bytes != audio.bytes
        || ready._verified.sha256 != audio.sha256
    {
        return Err("Owned recording changed; preserving it".into());
    }
    drop(metadata_leases);
    Ok(ready)
}
fn recover_namespace(
    namespace: &PinnedDirectory,
    own: Option<&Path>,
    task: Option<&Task>,
) -> Result<()> {
    for entry in fs::read_dir(namespace.path())?.take(4097) {
        let entry = entry?;
        if task.is_some_and(|task| task.cancelled.load(Ordering::Acquire)) {
            return Err("Recording preparation cancelled".into());
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("session-")
            || name.len() > 96
            || !name[8..].bytes().all(|b| b.is_ascii_digit() || b == b'-')
            || own == Some(entry.path().as_path())
        {
            continue;
        }
        // A live owner's WRITE handle denies this recovery opener. Any
        // unrecognized marker or unsafe directory remains intact.
        let _ = recover_one(&entry.path());
    }
    Ok(())
}
fn retire_bundle(path: &Path, bundle: &mut Bundle) -> Result<bool> {
    if !exists(path)? {
        bundle.pin.take();
        return Ok(true);
    }
    if bundle.pin.is_none() {
        bundle.pin = Some(PinnedDirectory::open(path)?);
    }
    let pin = bundle.pin.as_ref().unwrap();
    if pin.identity() != bundle.record.directory {
        return Err("Owned bundle identity changed; preserving it".into());
    }
    // The decoder may retain its own read handle briefly after Ready drops.
    // Probe/retire audio first so a sharing conflict leaves metadata usable.
    let mut files: Vec<_> = bundle.record.files.iter().collect();
    files.sort_by_key(|file| usize::from(file.name != "audio.mp3"));
    for file in files {
        match owned_file::delete_if_unchanged(
            &pin.path().join(&file.name),
            file.identity,
            &file.sha256,
            file.bytes,
        )? {
            DeleteOutcome::Deleted | DeleteOutcome::Missing => {}
            DeleteOutcome::Busy => return Ok(false),
            DeleteOutcome::Changed => return Err("Owned recording changed; preserving it".into()),
        }
    }
    let path = pin.path().to_owned();
    bundle.pin.take();
    match owned_file::delete_empty_directory(&path, bundle.record.directory)? {
        DeleteOutcome::Deleted | DeleteOutcome::Missing => Ok(true),
        DeleteOutcome::Busy => {
            bundle.pin = Some(PinnedDirectory::open(&path)?);
            Ok(false)
        }
        DeleteOutcome::Changed => Err("Owned directory changed; preserving it".into()),
    }
}

#[cfg(feature = "qa")]
pub(crate) fn qa_usage(audio_root: &Path) -> Result<(u64, usize)> {
    let namespace = PinnedDirectory::open(&audio_root.join("ScopedContinuous"))?;
    // A measurement must not transiently pin a session directory during its
    // deletion, or turn the observer itself into an artificial cleanup blocker.
    let _lock = global_lock_wait(&namespace, None, true)?;
    usage(&namespace)
}
fn recover_one(path: &Path) -> Result<()> {
    let pin = PinnedDirectory::open(path)?;
    let owner_path = pin.path().join("owner.lock");
    let mut owner = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(&owner_path)?;
    let identity = owned_file::identity_file(&owner)?;
    if owner.metadata()?.len() > LEDGER_LIMIT as u64 {
        return Err("Oversized ownership marker; preserving it".into());
    }
    let mut bytes = Vec::new();
    (&mut owner)
        .take(LEDGER_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    let ledger: Ledger = serde_json::from_slice(&bytes)?;
    ledger.validate(pin.identity(), identity)?;
    for record in ledger.records {
        let path = bundle_path(&pin, record.serial);
        let mut bundle = Bundle {
            record,
            pin: None,
            leases: Arc::new(AtomicUsize::new(0)),
            blocked: false,
        };
        if !retire_bundle(&path, &mut bundle)? {
            return Ok(());
        }
    }
    if fs::read_dir(pin.path())?.count() != 1 {
        return Ok(());
    }
    let (identity, len, hash) = owned_file::fingerprint_file(&mut owner)?;
    drop(owner);
    if owned_file::delete_if_unchanged(&owner_path, identity, &hash, len)? == DeleteOutcome::Deleted
    {
        let directory_identity = pin.identity();
        let path = pin.path().to_owned();
        drop(pin);
        let _ = owned_file::delete_empty_directory(&path, directory_identity)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(n: u32) -> FileIdentity {
        FileIdentity {
            volume: 1,
            index_high: 0,
            index_low: n,
        }
    }
    #[test]
    fn ownership_ledger_rejects_foreign_ids_paths_and_duplicate_targets() {
        let mut ledger = Ledger {
            schema: "native-scoped-continuous-v1".into(),
            directory: identity(1),
            owner: identity(2),
            records: vec![],
        };
        assert!(ledger.validate(identity(9), identity(2)).is_err());
        let file = Owned {
            name: "../audio.mp3".into(),
            identity: identity(3),
            bytes: 4,
            sha256: "a".repeat(64),
        };
        ledger.records.push(Record {
            serial: 1,
            directory: identity(4),
            reciter: 0,
            surah: 113,
            complete: false,
            files: vec![file],
        });
        assert!(ledger.validate(identity(1), identity(2)).is_err());
        ledger.records[0].files[0].name = "audio.mp3".into();
        assert!(ledger.validate(identity(1), identity(2)).is_ok());
        ledger.records.push(ledger.records[0].clone());
        assert!(ledger.validate(identity(1), identity(2)).is_err());
    }
    #[test]
    fn ready_drop_releases_only_counters_without_owning_worker_sender() {
        let session = Arc::new(AtomicUsize::new(1));
        let bundle = Arc::new(AtomicUsize::new(1));
        drop(Lease {
            session: session.clone(),
            bundle: Some(bundle.clone()),
            _borrowed: None,
        });
        assert_eq!(session.load(Ordering::Acquire), 0);
        assert_eq!(bundle.load(Ordering::Acquire), 0);
    }
    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "quran-scoped-storage-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }
    // Synthetic bytes here exercise ownership only; they are never accepted as
    // Quran recordings or passed to a decoder. Integration QA uses originals.
    fn bundle(session: &mut Session) -> PathBuf {
        let serial = session.next_serial;
        session.next_serial += 1;
        let path = bundle_path(session.directory.as_ref().unwrap(), serial);
        fs::create_dir(&path).unwrap();
        let pin = PinnedDirectory::open(&path).unwrap();
        let seals = Rc::new(RefCell::new(Seals::default()));
        for name in NAMES {
            let (file, id) = owned_file::create_new(&path.join(name)).unwrap();
            let mut output = Output {
                file: Some(file),
                owned: Some((name.into(), id, seals.clone())),
            };
            output
                .write_all(b"test-owned partial bytes, never a recording")
                .unwrap();
            output.sync_all().unwrap();
        }
        assert!(seals.borrow().error.is_none());
        session.bundles.push(Bundle {
            record: Record {
                serial,
                directory: pin.identity(),
                reciter: 0,
                surah: 112,
                complete: false,
                files: seals.borrow().files.clone(),
            },
            pin: Some(pin),
            leases: Arc::new(AtomicUsize::new(0)),
            blocked: false,
        });
        session.persist().unwrap();
        path
    }
    fn remove_created(path: &Path) {
        let (id, bytes, hash) = owned_file::fingerprint(path).unwrap();
        assert_eq!(
            owned_file::delete_if_unchanged(path, id, &hash, bytes).unwrap(),
            DeleteOutcome::Deleted
        );
    }
    fn finish(root: PathBuf) {
        let namespace = root.join("ScopedContinuous");
        assert_eq!(fs::read_dir(&namespace).unwrap().count(), 1);
        remove_created(&namespace.join("namespace.lock"));
        fs::remove_dir(namespace).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn worker_retirement_waits_for_ready_and_decoder_leases_then_removes_exact_files() {
        let root = root();
        let mut session = Session::new(&root, None).unwrap();
        let path = bundle(&mut session);
        let lease = session.lease(Some(session.bundles[0].leases.clone()), None);
        session.retire_released().unwrap();
        assert_eq!(fs::read_dir(&path).unwrap().count(), 4);
        drop(lease);
        let decoder = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(path.join("audio.mp3"))
            .unwrap();
        session.retire_released().unwrap();
        assert_eq!(
            fs::read_dir(&path).unwrap().count(),
            4,
            "busy decoder must keep metadata intact"
        );
        drop(decoder);
        session.retire_released().unwrap();
        assert!(!path.exists());
        assert!(session.bundles.is_empty());
        session.close().unwrap();
        drop(session);
        finish(root);
    }
    #[test]
    fn modified_and_unknown_files_survive_retirement_and_inactive_recovery() {
        let root = root();
        let mut session = Session::new(&root, None).unwrap();
        let modified = bundle(&mut session);
        fs::write(
            modified.join("audio.mp3"),
            b"changed by this test after the creation seal",
        )
        .unwrap();
        let unknown = bundle(&mut session);
        fs::write(
            unknown.join("unknown-test-input"),
            b"preserve this unknown input",
        )
        .unwrap();
        session.retire_released().unwrap();
        assert_eq!(
            fs::read(modified.join("audio.mp3")).unwrap(),
            b"changed by this test after the creation seal"
        );
        assert_eq!(
            fs::read(unknown.join("unknown-test-input")).unwrap(),
            b"preserve this unknown input"
        );
        session.close().unwrap();
        drop(session);
        let mut recovering = Session::new(&root, None).unwrap();
        assert!(modified.join("audio.mp3").exists());
        assert!(unknown.join("unknown-test-input").exists());
        recovering.close().unwrap();
        drop(recovering);
        // Remove only this test's explicit inputs after preservation assertions.
        for name in NAMES {
            remove_created(&modified.join(name));
        }
        remove_created(&unknown.join("unknown-test-input"));
        fs::remove_dir(&modified).unwrap();
        fs::remove_dir(&unknown).unwrap();
        let mut recovered = Session::new(&root, None).unwrap();
        recovered.close().unwrap();
        drop(recovered);
        finish(root);
    }
    #[test]
    fn live_owner_is_excluded_but_sealed_inactive_owner_is_recovered() {
        let root = root();
        let mut active = Session::new(&root, None).unwrap();
        let path = bundle(&mut active);
        let mut concurrent = Session::new(&root, None).unwrap();
        assert!(path.join("audio.mp3").exists());
        concurrent.close().unwrap();
        drop(concurrent);
        drop(active); // A process interruption after the ledger was synced.
        let mut recovered = Session::new(&root, None).unwrap();
        assert!(!path.exists());
        recovered.close().unwrap();
        drop(recovered);
        finish(root);
    }
    #[test]
    fn root_cap_recovers_inactive_owners_before_admitting_another_session() {
        let root = root();
        let mut live = Vec::new();
        for _ in 0..MAX_SESSIONS {
            live.push(Session::new(&root, None).unwrap());
        }
        assert!(Session::new(&root, None).is_err());
        drop(live);
        let mut recovered = Session::new(&root, None).unwrap();
        assert_eq!(usage(&recovered.namespace).unwrap().1, 1);
        recovered.close().unwrap();
        drop(recovered);
        finish(root);
    }
    #[test]
    fn changed_backlog_is_bounded_and_cannot_trigger_unbounded_downloads() {
        let root = root();
        let mut session = Session::new(&root, None).unwrap();
        let mut paths = Vec::new();
        for _ in 0..MAX_BUNDLES {
            let path = bundle(&mut session);
            fs::write(path.join("audio.mp3"), b"explicit modified test input").unwrap();
            paths.push(path);
        }
        let (events, _receiver) = std::sync::mpsc::sync_channel(4);
        let task = Task {
            reciter: 0,
            surah: 113,
            generation: 1,
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            events,
        };
        let error = session.prepare(&task).err().unwrap().to_string();
        assert!(error.contains("insufficient space"));
        assert_eq!(session.bundles.len(), MAX_BUNDLES);
        drop(session);
        for path in paths {
            for name in NAMES {
                remove_created(&path.join(name));
            }
            fs::remove_dir(path).unwrap();
        }
        let mut recovered = Session::new(&root, None).unwrap();
        recovered.close().unwrap();
        drop(recovered);
        finish(root);
    }
}
