//! Atomic app-profile metadata replacement under read-only ancestry pins.
//! Publisher recordings and Quran resources never pass through this adapter.
use crate::{
    data::Result,
    owned_file::{self, PinnedDirectory},
};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use windows::{
    Wdk::Storage::FileSystem::{
        FILE_RENAME_INFORMATION, FileRenameInformation, NtSetInformationFile,
    },
    Win32::{
        Foundation::{HANDLE, NTSTATUS, RtlNtStatusToDosError},
        Storage::FileSystem::{
            FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
        },
        System::IO::IO_STATUS_BLOCK,
    },
};
const MAX_BYTES: usize = 256 * 1024;
const MAX_NAME_UNITS: usize = 128;
static NONCE: AtomicU64 = AtomicU64::new(1);

fn basename(path: &Path) -> Result<Vec<u16>> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid profile filename")?;
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|character| character.is_control() || "<>:\"/\\|?*".contains(character))
    {
        return Err("Profile replacement requires a simple unaliased filename".into());
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|number| {
                matches!(
                    number,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
    {
        return Err("Profile filename cannot name a Windows device".into());
    }
    let units: Vec<_> = name.encode_utf16().collect();
    if units.len() > MAX_NAME_UNITS {
        return Err("Profile filename exceeds its bound".into());
    }
    Ok(units)
}
struct Created {
    file: File,
    cleanup: bool,
}
impl Drop for Created {
    fn drop(&mut self) {
        if self.cleanup {
            // This exact create-new handle excludes external writes/deletion.
            // A failed replacement must never unlink the destination by name.
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            unsafe {
                let _ = SetFileInformationByHandle(
                    HANDLE(self.file.as_raw_handle()),
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                );
            }
        }
    }
}
fn status_error(status: NTSTATUS) -> Box<dyn std::error::Error + Send + Sync> {
    std::io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32).into()
}
fn rename_same_parent(created: &mut Created, name: &[u16]) -> Result<()> {
    let name_bytes = name
        .len()
        .checked_mul(2)
        .ok_or("Profile filename byte count overflow")?;
    let length = std::mem::size_of::<FILE_RENAME_INFORMATION>()
        .checked_add(name_bytes)
        .ok_or("Profile rename buffer size overflow")?;
    // usize elements provide the pointer alignment required by the ABI on
    // either Windows architecture; the extra flexible-array bytes are bounded.
    let mut buffer = vec![0usize; length.div_ceil(std::mem::size_of::<usize>())];
    let information = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    let mut completion = IO_STATUS_BLOCK::default();
    let result = unsafe {
        (*information).Anonymous.ReplaceIfExists = true;
        (*information).RootDirectory = HANDLE::default();
        (*information).FileNameLength = name_bytes as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*information).FileName).cast::<u16>(),
            name.len(),
        );
        // RootDirectory=NULL + a simple basename renames within the source's
        // current parent. No CWD resolution, access bypass or POSIX flags.
        NtSetInformationFile(
            HANDLE(created.file.as_raw_handle()),
            &mut completion,
            information.cast(),
            length as u32,
            FileRenameInformation,
        )
    };
    if result.0 < 0 {
        return Err(status_error(result));
    }
    // The ordinary File handle is synchronous. If an unexpected positive or
    // inconsistent completion occurs, preserve its file: it may already have
    // been renamed, so disposing it could delete the published destination.
    created.cleanup = false;
    if result.0 != 0 {
        return Err("Profile replacement did not report synchronous completion".into());
    }
    let completed = unsafe { completion.Anonymous.Status };
    if completed.0 < 0 {
        return Err(status_error(completed));
    }
    if completed.0 != 0 {
        return Err("Profile replacement returned an unexpected completion".into());
    }
    Ok(())
}

/// The caller serializes and applies its own smaller limit before this adapter.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = basename(path)?;
    if bytes.len() > MAX_BYTES {
        return Err("Profile metadata exceeds its bound".into());
    }
    let parent = path
        .parent()
        .ok_or("Profile metadata has no parent directory")?;
    let pin = PinnedDirectory::open(parent)?;
    let filename = path.file_name().unwrap();
    let destination = pin.path().join(filename);
    // Validate and release the old target. Native classic replacement refuses
    // even our own open validation handle; independent readers remain protected
    // by their handles. The basename operation only replaces this directory
    // entry, never resolves a target stream or follows it outside the parent.
    match OpenOptions::new()
        .read(true)
        .share_mode(7)
        .custom_flags(0x0020_0000)
        .open(&destination)
    {
        Ok(file) => {
            owned_file::identity_file(&file)?;
            drop(file);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    };
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temporary = pin.path().join(format!(
        ".qm-profile-{}-{stamp}-{}.tmp",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000)
        .share_mode(1)
        .custom_flags(0x0020_0000)
        .create_new(true)
        .open(&temporary)?;
    let mut created = Created {
        file,
        cleanup: true,
    };
    owned_file::identity_file(&created.file)?;
    created.file.write_all(bytes)?;
    created.file.sync_all()?;
    rename_same_parent(&mut created, &name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};
    fn folder() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "native-profile-handle-test-{}-{}-{}-ملف",
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
    #[test]
    fn same_directory_unicode_replacement_succeeds_while_ancestry_stays_pinned() {
        let root = folder();
        let path = root.join("إعدادات القارئ.json");
        write_atomic(&path, b"old settings").unwrap();
        let namespace = root.join("AudioStore").join("ScopedContinuous");
        fs::create_dir_all(&namespace).unwrap();
        let pin = PinnedDirectory::open(&namespace).unwrap();
        write_atomic(&path, b"new settings").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new settings");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        assert!(fs::rename(&root, root.with_extension("moved")).is_err());
        drop(pin);
        fs::remove_file(path).unwrap();
        fs::remove_dir(namespace).unwrap();
        fs::remove_dir(root.join("AudioStore")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn active_target_reader_preserves_old_bytes_and_removes_only_failed_temporary() {
        let root = folder();
        let path = root.join("preferences.json");
        write_atomic(&path, b"old settings").unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let failure = write_atomic(&path, b"replacement must wait").unwrap_err();
        assert!(matches!(
            failure
                .downcast_ref::<std::io::Error>()
                .and_then(|error| error.raw_os_error()),
            Some(5 | 32)
        ));
        assert_eq!(fs::read(&path).unwrap(), b"old settings");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        drop(reader);
        write_atomic(&path, b"new settings").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new settings");
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn invalid_or_aliased_targets_and_oversized_bytes_create_no_files() {
        let root = folder();
        for name in [
            "preferences.json:stream",
            "preferences.json ",
            "preferences.json.",
            "CON.json",
            "COM1.json",
            "NUL",
            "bad\0name",
        ] {
            assert!(write_atomic(&root.join(name), b"{}").is_err(), "{name:?}");
            assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        }
        assert!(write_atomic(&root.join("x".repeat(MAX_NAME_UNITS + 1)), b"{}").is_err());
        assert!(write_atomic(&root.join("preferences.json"), &vec![0; MAX_BYTES + 1]).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir(root).unwrap();
    }
}
