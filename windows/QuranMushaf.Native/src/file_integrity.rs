//! Windows complete-file validation with a read lease preventing replacement.
use crate::{data::Result, mp3};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, Read},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use windows::{Win32::Security::Cryptography::*, core::PCWSTR};
pub struct VerifiedFile {
    pub info: mp3::Mp3Info,
    pub sha256: String,
    pub _lease: File,
}
struct HashedFile<'a> {
    cancelled: Option<&'a AtomicBool>,
    file: Option<File>,
    algorithm: BCRYPT_ALG_HANDLE,
    hash: BCRYPT_HASH_HANDLE,
    bytes: u64,
}
impl<'a> HashedFile<'a> {
    fn open(path: &Path, cancelled: Option<&'a AtomicBool>) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
            return Err("Recording storage is not a regular file".into());
        }
        let mut algorithm = BCRYPT_ALG_HANDLE::default();
        let mut hash = BCRYPT_HASH_HANDLE::default();
        unsafe {
            BCryptOpenAlgorithmProvider(
                &mut algorithm,
                BCRYPT_SHA256_ALGORITHM,
                PCWSTR::null(),
                BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0),
            )
            .ok()?;
            if let Err(error) = BCryptCreateHash(algorithm, &mut hash, None, None, 0).ok() {
                let _ = BCryptCloseAlgorithmProvider(algorithm, 0);
                return Err(error.into());
            }
        }
        Ok(Self {
            cancelled,
            file: Some(file),
            algorithm,
            hash,
            bytes: 0,
        })
    }
    fn finish(&self) -> Result<String> {
        let mut digest = [0u8; 32];
        unsafe {
            BCryptFinishHash(self.hash, &mut digest, 0).ok()?;
        }
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}
impl Read for HashedFile<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self
            .cancelled
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err(std::io::Error::other("Recording validation cancelled"));
        }
        let count = self.file.as_mut().unwrap().read(buffer)?;
        if count > 0 {
            unsafe {
                BCryptHashData(self.hash, &buffer[..count], 0)
                    .ok()
                    .map_err(std::io::Error::other)?;
            }
            self.bytes += count as u64;
        }
        Ok(count)
    }
}
impl Drop for HashedFile<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = BCryptDestroyHash(self.hash);
            let _ = BCryptCloseAlgorithmProvider(self.algorithm, 0);
        }
    }
}
#[cfg(feature = "qa")]
pub fn inspect(path: &Path) -> Result<VerifiedFile> {
    inspect_cancellable(path, None)
}
pub fn inspect_cancellable(path: &Path, cancelled: Option<&AtomicBool>) -> Result<VerifiedFile> {
    let file = HashedFile::open(path, cancelled)?;
    let length = file.file.as_ref().unwrap().metadata()?.len();
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let info = mp3::inspect_reader(&mut reader, length, mp3::MAX_CHAPTER_BYTES)?;
    let sha256 = reader.get_ref().finish()?;
    if reader.get_ref().bytes != length {
        return Err("Recording length changed during validation".into());
    }
    let mut file = reader.into_inner();
    Ok(VerifiedFile {
        info,
        sha256,
        _lease: file.file.take().unwrap(),
    })
}
