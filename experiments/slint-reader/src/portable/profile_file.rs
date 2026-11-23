//! Linux and macOS counterpart of the native profile_file.rs: replace a small
//! profile file so that a crash or power cut leaves the old or the new
//! contents, never a mixture.
use crate::data::Result;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

static NONCE: AtomicU64 = AtomicU64::new(1);

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().ok_or("Profile file has no directory")?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Profile file has an invalid name")?;
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file()) {
        return Err("Profile file is a link or not a file".into());
    }
    let temporary = directory.join(format!(
        ".{name}.{}-{}.tmp",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    // The rename itself is durable once the directory is synced.
    File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_contents_and_leaves_no_temporary_files() {
        let root = std::env::temp_dir().join(format!("qr-atomic-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("state.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(&root).unwrap();
    }
}
