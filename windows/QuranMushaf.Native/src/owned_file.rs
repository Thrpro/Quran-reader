//! Windows ownership checks and same-handle retirement for scoped recordings.
//! The caller owns the namespace ledger and keeps its directory pins alive.
use crate::data::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Component, Path, PathBuf},
};
use windows::{
    Win32::{Foundation::HANDLE, Security::Cryptography::*, Storage::FileSystem::*},
    core::PCWSTR,
};

const MAX_OWNED_BYTES: u64 = 259 * 1024 * 1024;
const READ_ACCESS: u32 = 0x8000_0000;
const WRITE_ACCESS: u32 = 0x4000_0000;
const DELETE_ACCESS: u32 = 0x0001_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    pub volume: u32,
    pub index_high: u32,
    pub index_low: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteOutcome {
    Deleted,
    Missing,
    Busy,
    Changed,
}

pub struct PinnedDirectory {
    path: PathBuf,
    identity: FileIdentity,
    _directory: File,
    _ancestors: Vec<File>,
}
impl PinnedDirectory {
    pub fn open(path: &Path) -> Result<Self> {
        checked_path(path, false)?;
        let ancestry: Vec<_> = path.ancestors().collect();
        if ancestry.len() > 64 {
            return Err("Scoped storage path has too many ancestors".into());
        }
        let mut pins = Vec::with_capacity(ancestry.len());
        // Pin from the volume root down: an ancestor cannot be swapped for a
        // junction between its validation and opening a descendant.
        for ancestor in ancestry.into_iter().rev() {
            let file = OpenOptions::new()
                .access_mode(READ_ACCESS)
                .share_mode(FILE_SHARE_READ.0)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(ancestor)?;
            let info = information(&file)?;
            if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0
                || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
            {
                return Err("Scoped storage cannot pin a linked directory".into());
            }
            pins.push(file);
        }
        let directory = pins.pop().ok_or("Scoped storage has no directory")?;
        let identity = from_information(&information(&directory)?);
        let path = resolved_path(&directory)?;
        Ok(Self {
            path,
            identity,
            _directory: directory,
            _ancestors: pins,
        })
    }
    pub fn identity(&self) -> FileIdentity {
        self.identity
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn checked_path(path: &Path, file: bool) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err("Scoped storage requires a resolved absolute path".into());
    }
    for part in path.components() {
        if let Component::Normal(name) = part {
            let name = name
                .to_str()
                .ok_or("Scoped storage has an invalid path component")?;
            if name.contains(':') || name.ends_with(['.', ' ']) {
                return Err("Scoped storage cannot use alternate streams or aliased paths".into());
            }
        }
    }
    if file {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Scoped storage has an invalid filename")?;
        if name.contains(':') || name.ends_with(['.', ' ']) {
            return Err("Scoped storage cannot use an alternate stream or aliased filename".into());
        }
    }
    Ok(())
}

fn information(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)?;
    }
    Ok(info)
}
fn from_information(info: &BY_HANDLE_FILE_INFORMATION) -> FileIdentity {
    FileIdentity {
        volume: info.dwVolumeSerialNumber,
        index_high: info.nFileIndexHigh,
        index_low: info.nFileIndexLow,
    }
}
fn regular_file_information(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION> {
    let info = information(file)?;
    if !file.metadata()?.is_file()
        || info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_REPARSE_POINT.0)
            != 0
        || info.nNumberOfLinks != 1
        || (info.nFileIndexHigh == 0 && info.nFileIndexLow == 0)
    {
        return Err("Owned storage requires a regular file with one link".into());
    }
    // Removing a file also removes its alternate streams. An added stream is
    // unknown data even when the default payload's identity and SHA still match.
    let mut streams = [0u64; 2048];
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileStreamInfo,
            streams.as_mut_ptr().cast(),
            std::mem::size_of_val(&streams) as u32,
        )?;
    }
    let raw = unsafe {
        std::slice::from_raw_parts(
            streams.as_ptr().cast::<u8>(),
            std::mem::size_of_val(&streams),
        )
    };
    let next = u32::from_ne_bytes(raw[0..4].try_into()?);
    let name_bytes = u32::from_ne_bytes(raw[4..8].try_into()?) as usize;
    let expected: Vec<_> = "::$DATA".encode_utf16().collect();
    if next != 0 || name_bytes != expected.len() * 2 {
        return Err("Owned storage contains an unknown alternate stream".into());
    }
    for (index, expected) in expected.into_iter().enumerate() {
        let offset = 24 + index * 2;
        if u16::from_ne_bytes(raw[offset..offset + 2].try_into()?) != expected {
            return Err("Owned storage contains an unknown alternate stream".into());
        }
    }
    Ok(info)
}
fn resolved_path(file: &File) -> Result<PathBuf> {
    let mut units = vec![0u16; 32_768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            HANDLE(file.as_raw_handle()),
            &mut units,
            GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0),
        )
    } as usize;
    if length == 0 {
        return Err(windows::core::Error::from_thread().into());
    }
    if length >= units.len() {
        return Err("Scoped storage resolved path exceeds its bound".into());
    }
    use std::os::windows::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_wide(&units[..length]).into())
}

pub fn create_new(path: &Path) -> Result<(File, FileIdentity)> {
    checked_path(path, true)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(READ_ACCESS | WRITE_ACCESS)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .create_new(true)
        .open(path)?;
    let id = identity_file(&file)?;
    Ok((file, id))
}
#[cfg(test)]
pub fn identity(path: &Path) -> Result<FileIdentity> {
    checked_path(path, true)?;
    let file = OpenOptions::new()
        .access_mode(READ_ACCESS)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    identity_file(&file)
}
pub fn identity_file(file: &File) -> Result<FileIdentity> {
    Ok(from_information(&regular_file_information(file)?))
}
#[cfg(any(test, feature = "qa"))]
pub fn fingerprint(path: &Path) -> Result<(FileIdentity, u64, String)> {
    checked_path(path, true)?;
    let mut file = OpenOptions::new()
        .access_mode(READ_ACCESS)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    fingerprint_file(&mut file)
}
/// Seal partial or complete owned bytes while their creation handle still
/// denies external writes/deletion. Restore the caller's stream position.
pub fn fingerprint_file(file: &mut File) -> Result<(FileIdentity, u64, String)> {
    let info = regular_file_information(file)?;
    let id = from_information(&info);
    let length = file.metadata()?.len();
    if length > MAX_OWNED_BYTES {
        return Err("Owned storage fingerprint exceeds its read bound".into());
    }
    let previous = file.stream_position()?;
    file.seek(SeekFrom::Start(0))?;
    let result = hash(file, length);
    let restored = file.seek(SeekFrom::Start(previous));
    let digest = result?;
    restored?;
    if from_information(&regular_file_information(file)?) != id || file.metadata()?.len() != length
    {
        return Err("Owned storage changed during its fingerprint".into());
    }
    Ok((id, length, digest))
}

/// The caller must retain its directory pins and verify this target belongs to
/// its owned namespace. This API never opens persistent inputs for deletion.
pub fn delete_if_unchanged(
    path: &Path,
    expected_identity: FileIdentity,
    expected_sha256: &str,
    expected_bytes: u64,
) -> Result<DeleteOutcome> {
    checked_path(path, true)?;
    if expected_bytes > MAX_OWNED_BYTES
        || expected_sha256.len() != 64
        || !expected_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("Invalid owned storage fingerprint".into());
    }
    let mut file = match OpenOptions::new()
        .access_mode(READ_ACCESS | DELETE_ACCESS)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if matches!(error.raw_os_error(), Some(2 | 3)) => {
            return Ok(DeleteOutcome::Missing);
        }
        Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => {
            return Ok(DeleteOutcome::Busy);
        }
        Err(error) => return Err(error.into()),
    };
    let Ok(info) = regular_file_information(&file) else {
        return Ok(DeleteOutcome::Changed);
    };
    if from_information(&info) != expected_identity || file.metadata()?.len() != expected_bytes {
        return Ok(DeleteOutcome::Changed);
    }
    let (id, bytes, digest) = fingerprint_file(&mut file)?;
    if id != expected_identity || bytes != expected_bytes || digest != expected_sha256 {
        return Ok(DeleteOutcome::Changed);
    }
    // Mark this exact validated handle for deletion. No path reopening,
    // FILE_SHARE_DELETE or POSIX deletion can bypass an active reader's lease.
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )?;
    }
    drop(file);
    Ok(DeleteOutcome::Deleted)
}

/// Remove only the exact owned empty directory, through its validated handle.
/// Drop this directory's own pin first, but retain its parent's pins.
pub fn delete_empty_directory(path: &Path, expected: FileIdentity) -> Result<DeleteOutcome> {
    checked_path(path, false)?;
    let file = match OpenOptions::new()
        .access_mode(READ_ACCESS | DELETE_ACCESS)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if matches!(error.raw_os_error(), Some(2 | 3)) => {
            return Ok(DeleteOutcome::Missing);
        }
        Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => {
            return Ok(DeleteOutcome::Busy);
        }
        Err(error) => return Err(error.into()),
    };
    let info = information(&file)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0
        || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || from_information(&info) != expected
    {
        return Ok(DeleteOutcome::Changed);
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    if let Err(error) = unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } {
        if error.code().0 as u32 == 0x8007_0091 {
            // ERROR_DIR_NOT_EMPTY: unknown entries are never traversed/deleted.
            return Ok(DeleteOutcome::Changed);
        }
        return Err(error.into());
    }
    drop(file);
    Ok(DeleteOutcome::Deleted)
}

struct Sha256 {
    algorithm: BCRYPT_ALG_HANDLE,
    digest: BCRYPT_HASH_HANDLE,
}
impl Sha256 {
    fn new() -> Result<Self> {
        unsafe {
            let mut algorithm = BCRYPT_ALG_HANDLE::default();
            BCryptOpenAlgorithmProvider(
                &mut algorithm,
                BCRYPT_SHA256_ALGORITHM,
                PCWSTR::null(),
                BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0),
            )
            .ok()?;
            let mut digest = BCRYPT_HASH_HANDLE::default();
            if let Err(error) = BCryptCreateHash(algorithm, &mut digest, None, None, 0).ok() {
                let _ = BCryptCloseAlgorithmProvider(algorithm, 0);
                return Err(error.into());
            }
            Ok(Self { algorithm, digest })
        }
    }
}
impl Drop for Sha256 {
    fn drop(&mut self) {
        unsafe {
            let _ = BCryptDestroyHash(self.digest);
            let _ = BCryptCloseAlgorithmProvider(self.algorithm, 0);
        }
    }
}
fn hash(file: &mut File, expected: u64) -> Result<String> {
    let algorithm = Sha256::new()?;
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let length = file.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        total = total
            .checked_add(length as u64)
            .filter(|total| *total <= expected)
            .ok_or("Owned file grew during its fingerprint")?;
        unsafe {
            BCryptHashData(algorithm.digest, &buffer[..length], 0).ok()?;
        }
    }
    if total != expected {
        return Err("Owned file length changed during its fingerprint".into());
    }
    let mut bytes = [0u8; 32];
    unsafe {
        BCryptFinishHash(algorithm.digest, &mut bytes, 0).ok()?;
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn folder() -> PathBuf {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "quran-owned-file-{}-{time}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        path
    }
    fn created(path: &Path, bytes: &[u8]) -> (FileIdentity, u64, String) {
        let (mut file, id) = create_new(path).unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
        let fingerprint = fingerprint_file(&mut file).unwrap();
        assert_eq!(fingerprint.0, id);
        fingerprint
    }
    fn retire(path: &Path, expected: &(FileIdentity, u64, String)) -> DeleteOutcome {
        delete_if_unchanged(path, expected.0, &expected.2, expected.1).unwrap()
    }

    #[test]
    fn active_read_lease_blocks_retirement_and_writes() {
        let root = folder();
        let path = root.join("audio.mp3");
        let expected = created(&path, b"only this created test input");
        let lease = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        assert_eq!(identity(&path).unwrap(), expected.0);
        assert_eq!(retire(&path, &expected), DeleteOutcome::Busy);
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"only this created test input");
        drop(lease);
        assert_eq!(retire(&path, &expected), DeleteOutcome::Deleted);
        assert_eq!(retire(&path, &expected), DeleteOutcome::Missing);
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn modified_bytes_and_same_bytes_replacement_are_preserved() {
        let root = folder();
        let path = root.join("payload");
        let original = created(&path, b"original");
        fs::write(&path, b"modified").unwrap();
        assert_eq!(retire(&path, &original), DeleteOutcome::Changed);
        assert_eq!(fs::read(&path).unwrap(), b"modified");
        let modified = fingerprint(&path).unwrap();
        let retained = root.join("retained");
        fs::rename(&path, &retained).unwrap();
        let replacement = created(&path, b"original");
        assert_ne!(replacement.0, original.0);
        assert_eq!(retire(&path, &original), DeleteOutcome::Changed);
        assert_eq!(retire(&path, &replacement), DeleteOutcome::Deleted);
        assert_eq!(retire(&retained, &modified), DeleteOutcome::Deleted);
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn hardlinks_and_unknown_alternate_streams_are_preserved() {
        let root = folder();
        let path = root.join("payload");
        let expected = created(&path, b"owned");
        let alias = root.join("test-created-hardlink");
        fs::hard_link(&path, &alias).unwrap();
        assert!(identity(&path).is_err());
        assert_eq!(retire(&path, &expected), DeleteOutcome::Changed);
        assert_eq!(fs::read(&alias).unwrap(), b"owned");
        fs::remove_file(&alias).unwrap();
        let stream = root.join("payload:unknown");
        fs::write(&stream, b"unknown stream bytes created only by this test").unwrap();
        assert!(identity(&path).is_err());
        assert!(identity(&stream).is_err());
        assert_eq!(retire(&path, &expected), DeleteOutcome::Changed);
        assert_eq!(
            fs::read(&stream).unwrap(),
            b"unknown stream bytes created only by this test"
        );
        // Only the explicitly created test stream is removed, never an input.
        fs::remove_file(stream).unwrap();
        assert_eq!(retire(&path, &expected), DeleteOutcome::Deleted);
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn pins_allow_adjacent_writes_and_persistent_stage_creation() {
        let root = folder();
        let session = root.join("owned-session");
        fs::create_dir(&session).unwrap();
        let pin = PinnedDirectory::open(&session).unwrap();
        assert!(fs::rename(&session, root.join("moved-session")).is_err());
        let persistent = root.join("persistent");
        fs::create_dir(&persistent).unwrap();
        let persistent_input = persistent.join("original.mp3");
        let before = created(&persistent_input, b"persistent original test recording");
        let stage = persistent.join("new-stage");
        fs::create_dir(&stage).unwrap();
        let next = stage.join("audio.mp3");
        let prepared = created(&next, b"independently written persistent stage");
        let adjacent = root.join("adjacent");
        fs::create_dir(&adjacent).unwrap();
        let adjacent_file = adjacent.join("ordinary-file");
        let ordinary = created(&adjacent_file, b"ordinary adjacent writing works");
        assert_eq!(fingerprint(&persistent_input).unwrap(), before);
        assert_eq!(retire(&next, &prepared), DeleteOutcome::Deleted);
        assert_eq!(retire(&adjacent_file, &ordinary), DeleteOutcome::Deleted);
        assert_eq!(retire(&persistent_input, &before), DeleteOutcome::Deleted);
        drop(pin);
        fs::remove_dir(stage).unwrap();
        fs::remove_dir(persistent).unwrap();
        fs::remove_dir(adjacent).unwrap();
        fs::remove_dir(session).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn partial_creation_is_sealed_before_releasing_its_write_handle() {
        let root = folder();
        let path = root.join("incomplete.mp3");
        let (mut file, original) = create_new(&path).unwrap();
        file.write_all(b"not a complete MP3, but exact owned partial bytes")
            .unwrap();
        let position = file.stream_position().unwrap();
        let expected = fingerprint_file(&mut file).unwrap();
        assert_eq!(expected.0, original);
        assert_eq!(file.stream_position().unwrap(), position);
        assert!(fingerprint(&path).is_err());
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        drop(file);
        assert_eq!(fingerprint(&path).unwrap(), expected);
        assert_eq!(retire(&path, &expected), DeleteOutcome::Deleted);
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn directory_disposition_respects_pin_and_unknown_entries() {
        let root = folder();
        let owned = root.join("owned");
        fs::create_dir(&owned).unwrap();
        let pin = PinnedDirectory::open(&owned).unwrap();
        let id = pin.identity();
        assert_eq!(
            delete_empty_directory(&owned, id).unwrap(),
            DeleteOutcome::Busy
        );
        let unknown = owned.join("unknown-created-test-input");
        let expected = created(&unknown, b"preserve unknown entries");
        drop(pin);
        assert_eq!(
            delete_empty_directory(&owned, id).unwrap(),
            DeleteOutcome::Changed
        );
        assert!(unknown.exists());
        assert_eq!(retire(&unknown, &expected), DeleteOutcome::Deleted);
        assert_eq!(
            delete_empty_directory(&owned, id).unwrap(),
            DeleteOutcome::Deleted
        );
        fs::remove_dir(root).unwrap();
    }
}
