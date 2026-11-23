//! Linux and macOS counterpart of the native owned_file.rs: file identities and
//! directories held open while the profile uses them. Only what the Slint
//! reader's preferences need.
use crate::data::Result;
use std::{
    fs::{self, File},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

/// Which file this is, and its size and modification time, so a file replaced
/// or edited by someone else is noticed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    device: u64,
    inode: u64,
    size: u64,
    modified_ns: i128,
}

fn identity(metadata: &fs::Metadata) -> FileIdentity {
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.size(),
        modified_ns: i128::from(metadata.mtime()) * 1_000_000_000
            + i128::from(metadata.mtime_nsec()),
    }
}

pub fn identity_file(file: &File) -> Result<FileIdentity> {
    Ok(identity(&file.metadata()?))
}

/// A real directory (not a symbolic link, and with no linked ancestor), kept
/// open while in use.
pub struct PinnedDirectory {
    path: PathBuf,
    _directory: File,
}

impl PinnedDirectory {
    pub fn open(path: &Path) -> Result<Self> {
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        {
            return Err("Scoped storage requires a resolved absolute path".into());
        }
        if !fs::symlink_metadata(path)?.is_dir() {
            return Err("Scoped storage cannot pin a linked directory".into());
        }
        // The resolved path names the same directory without any link; a
        // different one means an ancestor is a link.
        let path_resolved = path.canonicalize()?;
        let directory = File::open(&path_resolved)?;
        if identity(&directory.metadata()?) != identity(&fs::metadata(path)?) {
            return Err("Scoped storage directory changed while opening".into());
        }
        Ok(Self {
            path: path_resolved,
            _directory: directory,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_real_directories_and_refuses_links() {
        let root = std::env::temp_dir().join(format!("qr-pin-{}", std::process::id()));
        let real = root.join("real");
        fs::create_dir_all(&real).unwrap();
        assert!(PinnedDirectory::open(&real).is_ok());
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(PinnedDirectory::open(&link).is_err());
        assert!(PinnedDirectory::open(Path::new("relative")).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn identity_changes_when_the_file_is_rewritten() {
        let path = std::env::temp_dir().join(format!("qr-id-{}", std::process::id()));
        fs::write(&path, b"one").unwrap();
        let before = identity_file(&File::open(&path).unwrap()).unwrap();
        fs::write(&path, b"three").unwrap();
        let after = identity_file(&File::open(&path).unwrap()).unwrap();
        assert_ne!(before, after);
        fs::remove_file(&path).unwrap();
    }
}
