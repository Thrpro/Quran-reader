//! Isolated, bounded Slint-preview preferences. Native profiles and Quran
//! resources never pass through this module. The caller explicitly selects
//! an app-owned profile directory and may use session-only defaults on error.
use crate::{
    data::{Result, Surah, valid_reference},
    owned_file::{self, FileIdentity, PinnedDirectory},
    profile_file,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::Read,
    marker::PhantomData,
    os::windows::ffi::OsStrExt,
    os::windows::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf},
    rc::Rc,
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HANDLE, SetLastError,
            WAIT_ABANDONED, WAIT_OBJECT_0,
        },
        System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
    core::HSTRING,
};

const STATE_FILE: &str = "slint-reader-state-v1.json";
const SCHEMA: &str = "quran-mushaf-slint-profile-v1";
const MAX_STATE_BYTES: usize = 64 * 1024;
pub const MAX_BOOKMARKS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bookmark {
    pub surah: u32,
    pub ayah: u32,
    pub page: u32,
}
impl Bookmark {
    pub fn key(self) -> (u32, u32) {
        (self.surah, self.ayah)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Preferences {
    pub arabic: bool,
    #[serde(default = "legacy_onboarding_complete")]
    pub onboarding_complete: bool,
    /// 0: dark, 1: light, 2: sepia. There is one source of theme truth.
    pub theme: u32,
    pub contextual: bool,
    pub show_clock: bool,
    pub clock_24: bool,
    pub single_page: bool,
    pub follow_recitation: bool,
    /// Opt in to opening the explanation with a primary pointer click.
    pub open_tafsir_on_click: bool,
    pub search_shortcut: u32,
    pub listen_shortcut: u32,
    pub options_shortcut: u32,
    /// None follows the current OS preference until an intentional UI choice.
    pub motion: Option<bool>,
    /// 0: bundled Al-Mukhtasar, 1: Al-Saadi, 2: Saheeh International.
    pub study_source: u32,
    pub study_text_size: u32,
    pub current_page: u32,
    pub volume: f64,
    pub remember_speed: bool,
    pub playback_rate: f64,
    /// The UI additionally checks this against its actual reciter catalogue.
    pub reciter: usize,
    pub bookmarks: Vec<Bookmark>,
    /// Applies at the next launch, because the renderer is chosen before the window exists.
    pub software_rendering: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            arabic: true,
            theme: 0,
            onboarding_complete: false,
            contextual: true,
            show_clock: true,
            clock_24: false,
            single_page: false,
            follow_recitation: false,
            open_tafsir_on_click: false,
            search_shortcut: 0,
            listen_shortcut: 0,
            options_shortcut: 0,
            motion: None,
            study_source: 0,
            study_text_size: 18,
            current_page: 1,
            volume: 80.,
            remember_speed: false,
            playback_rate: 1.,
            reciter: 0,
            bookmarks: Vec::new(),
            software_rendering: false,
        }
    }
}
fn legacy_onboarding_complete() -> bool {
    true
}
impl Preferences {
    pub fn is_dark(&self) -> bool {
        self.theme == 0
    }
    pub fn sanitize(&mut self, surahs: &[Surah]) -> bool {
        let before = self.clone();
        if self.theme > 2 {
            self.theme = 0;
        }
        if self.arabic && self.study_source > 1 {
            self.study_source = 0;
        } else if !self.arabic {
            self.study_source = 2;
        }
        self.study_text_size = self.study_text_size.clamp(14, 26);
        self.search_shortcut = self.search_shortcut.min(2);
        self.listen_shortcut = self.listen_shortcut.min(1);
        self.options_shortcut = self.options_shortcut.min(1);
        self.current_page = self.current_page.clamp(1, 604);
        self.volume = if self.volume.is_finite() {
            self.volume.clamp(0., 100.)
        } else {
            80.
        };
        self.playback_rate = if self.playback_rate.is_finite() {
            self.playback_rate.clamp(0.5, 2.)
        } else {
            1.
        };
        if self.reciter >= 64 {
            self.reciter = 0;
        }
        let mut seen = BTreeSet::new();
        self.bookmarks.retain(|bookmark| {
            (1..=604).contains(&bookmark.page)
                && valid_reference(surahs, bookmark.surah, bookmark.ayah)
                && seen.insert(bookmark.key())
        });
        self.bookmarks.truncate(MAX_BOOKMARKS);
        *self != before
    }
    #[cfg(test)]
    fn contains_bookmark(&self, key: (u32, u32)) -> bool {
        self.bookmarks.iter().any(|bookmark| bookmark.key() == key)
    }
}

/// Read before the window and profile owner exist, so nothing is written or
/// locked. Any problem with the file leaves the default for `Profile::load` to report.
pub fn saved_software_rendering(directory: &Path) -> bool {
    let path = directory.join(STATE_FILE);
    if fs::metadata(&path).map_or(true, |file| file.len() > 64 * 1024) {
        return false;
    }
    fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Envelope>(&bytes).ok())
        .is_some_and(|envelope| {
            envelope.schema == SCHEMA && envelope.preferences.software_rendering
        })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    preferences: Preferences,
}
#[derive(Clone)]
struct Saved {
    identity: FileIdentity,
    bytes: Vec<u8>,
}
/// Mutex ownership belongs to a Windows thread. The marker prevents moving or
/// sharing the profile to another thread and accidentally releasing elsewhere.
struct ProfileOwner {
    handle: HANDLE,
    _thread_bound: PhantomData<Rc<()>>,
}
impl ProfileOwner {
    fn acquire(directory: &PinnedDirectory) -> Result<Self> {
        let mut identity = Vec::new();
        // GetFinalPathNameByHandleW already supplied this canonical, pinned
        // directory name; hash UTF-16 bytes without lossy Unicode conversion.
        for unit in directory.path().as_os_str().encode_wide() {
            identity.extend_from_slice(&unit.to_le_bytes());
        }
        let digest = crate::audio_cache::sha256(&identity)?;
        let name = HSTRING::from(format!("Local\\QuranMushafSlintProfile-{digest}"));
        unsafe {
            SetLastError(ERROR_SUCCESS);
            let handle = CreateMutexW(None, false, &name)?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                // Windows mutex acquisition is recursive on the same thread;
                // rejecting an existing object also excludes a second Profile
                // in this process, not just another running app instance.
                let _ = CloseHandle(handle);
                return Err("This Slint preview profile is already open in another owner".into());
            }
            let acquired = WaitForSingleObject(handle, 0);
            if acquired != WAIT_OBJECT_0 && acquired != WAIT_ABANDONED {
                let _ = CloseHandle(handle);
                return Err("The Slint preview profile could not be owned exclusively".into());
            }
            Ok(Self {
                handle,
                _thread_bound: PhantomData,
            })
        }
    }
}
impl Drop for ProfileOwner {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}
pub struct Profile {
    preferences: Preferences,
    directory: PinnedDirectory,
    saved: Option<Saved>,
    _owner: ProfileOwner,
}

/// Portable namespace, separate from the native application's profile.
pub fn default_profile_directory() -> Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .ok_or("The preview executable has no parent directory")?
        .join("SlintProfile"))
}
impl Profile {
    /// Missing state gives defaults without writing. Parse/schema/size/ownership
    /// errors preserve the old file and return an error, never auto-repair it.
    pub fn load(directory: &Path, surahs: &[Surah]) -> Result<Self> {
        let directory = create_and_pin_directory(directory)?;
        let owner = ProfileOwner::acquire(&directory)?;
        let saved = read_saved(&directory.path().join(STATE_FILE))?;
        let mut preferences = match &saved {
            Some(saved) => {
                let envelope: Envelope = serde_json::from_slice(&saved.bytes)?;
                if envelope.schema != SCHEMA {
                    return Err("This is not a supported Slint-preview profile".into());
                }
                envelope.preferences
            }
            None => Preferences::default(),
        };
        preferences.sanitize(surahs);
        Ok(Self {
            preferences,
            directory,
            saved,
            _owner: owner,
        })
    }
    pub fn preferences(&self) -> &Preferences {
        &self.preferences
    }
    pub fn directory(&self) -> &Path {
        self.directory.path()
    }
    /// The in-memory value changes only after a successful bounded atomic save.
    pub fn replace(&mut self, next: Preferences, surahs: &[Surah]) -> Result<()> {
        let mut clean = next.clone();
        if clean.sanitize(surahs) {
            return Err("Preview preferences contain invalid values".into());
        }
        self.write_preferences(&next)?;
        self.preferences = next;
        Ok(())
    }
    #[cfg(test)]
    fn save(&mut self, surahs: &[Surah]) -> Result<()> {
        self.replace(self.preferences.clone(), surahs)
    }
    #[cfg(test)]
    fn toggle_bookmark(&mut self, key: (u32, u32), page: u32, surahs: &[Surah]) -> Result<bool> {
        if !valid_reference(surahs, key.0, key.1) || !(1..=604).contains(&page) {
            return Err("Cannot bookmark an invalid Quran reference".into());
        }
        let mut next = self.preferences.clone();
        let added = if let Some(index) = next.bookmarks.iter().position(|item| item.key() == key) {
            next.bookmarks.remove(index);
            false
        } else {
            if next.bookmarks.len() >= MAX_BOOKMARKS {
                return Err("The preview bookmark limit has been reached".into());
            }
            next.bookmarks.push(Bookmark {
                surah: key.0,
                ayah: key.1,
                page,
            });
            true
        };
        self.replace(next, surahs)?;
        Ok(added)
    }
    fn write_preferences(&mut self, next: &Preferences) -> Result<()> {
        let path = self.directory.path().join(STATE_FILE);
        // Refuse to clobber a changed/replaced profile or a new authored file
        // observed since load. The native replacement is pinned to this single
        // basename and cannot follow a redirect outside the owned directory.
        let current = read_saved(&path)?;
        let unchanged = match (&self.saved, &current) {
            (None, None) => true,
            (Some(expected), Some(current)) => {
                expected.identity == current.identity && expected.bytes == current.bytes
            }
            _ => false,
        };
        if !unchanged {
            return Err("Preview profile changed; reload before saving".into());
        }
        let bytes = serde_json::to_vec_pretty(&Envelope {
            schema: SCHEMA.into(),
            preferences: next.clone(),
        })?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err("Preview preferences exceed their 64 KiB limit".into());
        }
        profile_file::write_atomic(&path, &bytes)?;
        let published = read_saved(&path)?.ok_or("Published preview profile disappeared")?;
        if published.bytes != bytes {
            return Err("Published preview profile changed during verification".into());
        }
        self.saved = Some(published);
        Ok(())
    }
}

fn read_saved(path: &Path) -> Result<Option<Saved>> {
    let mut file = match OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() > MAX_STATE_BYTES as u64 {
        return Err("Preview preferences exceed their 64 KiB read limit".into());
    }
    if file.metadata()?.permissions().readonly() {
        return Err("Preview preferences are read-only".into());
    }
    let identity = owned_file::identity_file(&file)?;
    let bytes = bounded_read(&mut file)?;
    if owned_file::identity_file(&file)? != identity {
        return Err("Preview profile identity changed during reading".into());
    }
    Ok(Some(Saved { identity, bytes }))
}
fn bounded_read(file: &mut File) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(MAX_STATE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_STATE_BYTES {
        return Err("Preview preferences grew beyond their read limit".into());
    }
    Ok(bytes)
}
fn valid_directory_path(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        && path.components().all(|part| {
            let Component::Normal(name) = part else {
                return true;
            };
            name.to_str().is_some_and(|name| {
                !name.is_empty()
                    && !name.ends_with(['.', ' '])
                    && !name
                        .chars()
                        .any(|c| c.is_control() || "<>:\"|?*".contains(c))
            })
        })
}
fn create_and_pin_directory(path: &Path) -> Result<PinnedDirectory> {
    if !valid_directory_path(path) {
        return Err("Preview profile requires an absolute, unaliased directory".into());
    }
    let mut ancestor = path.to_path_buf();
    let mut missing: Vec<OsString> = Vec::new();
    loop {
        match fs::symlink_metadata(&ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or("Preview profile has no existing ancestor")?
                        .to_os_string(),
                );
                if !ancestor.pop() || missing.len() > 32 {
                    return Err("Preview profile ancestry exceeds its bound".into());
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    let mut pin = PinnedDirectory::open(&ancestor)?;
    for name in missing.into_iter().rev() {
        let next = pin.path().join(name);
        match fs::create_dir(&next) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        // Existing and newly created entries alike must pass reparse/ancestry
        // validation. The previous pin stays live until the next one is ready.
        pin = PinnedDirectory::open(&next)?;
    }
    Ok(pin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn surahs() -> Vec<Surah> {
        vec![
            Surah {
                id: 1,
                name_simple: "Al-Fatihah".into(),
                name_arabic: "الفاتحة".into(),
                verses_count: 7,
            },
            Surah {
                id: 2,
                name_simple: "Al-Baqarah".into(),
                name_arabic: "البقرة".into(),
                verses_count: 286,
            },
        ]
    }
    fn folder() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "slint-preferences-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        root
    }
    fn encode(preferences: Preferences) -> Vec<u8> {
        serde_json::to_vec(&Envelope {
            schema: SCHEMA.into(),
            preferences,
        })
        .unwrap()
    }
    #[test]
    fn new_profiles_show_onboarding_but_existing_profiles_keep_their_choice() {
        let new = Preferences::default();
        assert!(!new.onboarding_complete && new.contextual);
        let legacy: Preferences = serde_json::from_str(r#"{"contextual":false}"#).unwrap();
        assert!(legacy.onboarding_complete);
        assert!(!legacy.contextual);
        let chosen: Preferences =
            serde_json::from_str(r#"{"onboardingComplete":false,"contextual":false}"#).unwrap();
        assert!(!chosen.onboarding_complete && !chosen.contextual);
    }
    #[test]
    fn defaults_match_native_and_validation_restores_safe_values() {
        let prefs = Preferences::default();
        assert!(prefs.contextual && prefs.show_clock && !prefs.clock_24);
        assert!(prefs.motion.is_none() && !prefs.remember_speed);
        assert!(!prefs.open_tafsir_on_click);
        assert_eq!(prefs.playback_rate, 1.);
        let mut prefs = Preferences {
            arabic: false,
            theme: 99,
            current_page: 999,
            study_source: 1,
            study_text_size: 90,
            volume: f64::NAN,
            playback_rate: f64::INFINITY,
            reciter: 64,
            ..prefs
        };
        assert!(prefs.sanitize(&surahs()));
        assert_eq!(
            (
                prefs.theme,
                prefs.current_page,
                prefs.study_source,
                prefs.study_text_size
            ),
            (0, 604, 2, 26)
        );
        assert_eq!(
            (prefs.volume, prefs.playback_rate, prefs.reciter),
            (80., 1., 0)
        );
    }
    #[test]
    fn invalid_duplicate_and_excessive_bookmarks_are_bounded() {
        let mut prefs = Preferences {
            bookmarks: (1..=200)
                .map(|ayah| Bookmark {
                    surah: 2,
                    ayah,
                    page: 42,
                })
                .collect(),
            ..Preferences::default()
        };
        prefs.bookmarks.insert(
            0,
            Bookmark {
                surah: 0,
                ayah: 1,
                page: 1,
            },
        );
        prefs.bookmarks.insert(
            1,
            Bookmark {
                surah: 1,
                ayah: 8,
                page: 1,
            },
        );
        prefs.bookmarks.insert(
            2,
            Bookmark {
                surah: 2,
                ayah: 1,
                page: 0,
            },
        );
        prefs.bookmarks.push(Bookmark {
            surah: 2,
            ayah: 10,
            page: 44,
        });
        assert!(prefs.sanitize(&surahs()));
        assert_eq!(prefs.bookmarks.len(), MAX_BOOKMARKS);
        assert_eq!(prefs.bookmarks[0].key(), (2, 1));
        assert_eq!(prefs.bookmarks.last().unwrap().key(), (2, 128));
    }
    #[test]
    fn missing_profile_round_trips_and_toggling_preserves_unrelated_files() {
        let root = folder();
        fs::write(root.join("authored.txt"), b"preserved").unwrap();
        let mut profile = Profile::load(&root, &surahs()).unwrap();
        assert!(!root.join(STATE_FILE).exists());
        assert!(profile.toggle_bookmark((2, 255), 42, &surahs()).unwrap());
        let mut next = profile.preferences().clone();
        next.current_page = 42;
        next.motion = Some(false);
        next.theme = 2;
        next.open_tafsir_on_click = true;
        next.search_shortcut = 1;
        next.listen_shortcut = 1;
        next.options_shortcut = 1;
        profile.replace(next, &surahs()).unwrap();
        drop(profile);
        let mut profile = Profile::load(&root, &surahs()).unwrap();
        assert_eq!(profile.preferences().current_page, 42);
        assert_eq!(profile.preferences().theme, 2);
        assert_eq!(profile.preferences().motion, Some(false));
        assert!(profile.preferences().open_tafsir_on_click);
        assert_eq!(
            (
                profile.preferences().search_shortcut,
                profile.preferences().listen_shortcut,
                profile.preferences().options_shortcut
            ),
            (1, 1, 1)
        );
        assert!(profile.preferences().contains_bookmark((2, 255)));
        assert!(!profile.toggle_bookmark((2, 255), 42, &surahs()).unwrap());
        assert_eq!(fs::read(root.join("authored.txt")).unwrap(), b"preserved");
        drop(profile);
        fs::remove_file(root.join(STATE_FILE)).unwrap();
        fs::remove_file(root.join("authored.txt")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn bad_json_oversize_and_unknown_schema_are_never_overwritten() {
        let root = folder();
        let path = root.join(STATE_FILE);
        for bytes in [
            b"{bad json".to_vec(),
            vec![b' '; MAX_STATE_BYTES + 1],
            b"{\"schema\":\"other-app\",\"preferences\":{}}".to_vec(),
        ] {
            fs::write(&path, &bytes).unwrap();
            assert!(Profile::load(&root, &surahs()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn valid_schema_out_of_bound_values_are_normalized_without_rewriting() {
        let root = folder();
        let path = root.join(STATE_FILE);
        let original = encode(Preferences {
            current_page: 900,
            theme: 80,
            bookmarks: vec![Bookmark {
                surah: 2,
                ayah: 999,
                page: 42,
            }],
            ..Preferences::default()
        });
        fs::write(&path, &original).unwrap();
        let profile = Profile::load(&root, &surahs()).unwrap();
        assert_eq!(profile.preferences().current_page, 604);
        assert_eq!(profile.preferences().theme, 0);
        assert!(profile.preferences().bookmarks.is_empty());
        assert_eq!(fs::read(&path).unwrap(), original);
        drop(profile);
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn externally_changed_and_read_only_files_are_preserved() {
        let root = folder();
        let path = root.join(STATE_FILE);
        fs::write(&path, encode(Preferences::default())).unwrap();
        let mut profile = Profile::load(&root, &surahs()).unwrap();
        fs::write(&path, b"new authored contents").unwrap();
        assert!(profile.save(&surahs()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"new authored contents");
        drop(profile);
        let original_permissions = fs::metadata(&path).unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        assert!(Profile::load(&root, &surahs()).is_err());
        fs::set_permissions(&path, original_permissions).unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn active_target_reader_prevents_replacement_and_keeps_in_memory_value() {
        let root = folder();
        let mut profile = Profile::load(&root, &surahs()).unwrap();
        profile.save(&surahs()).unwrap();
        let path = root.join(STATE_FILE);
        let original = fs::read(&path).unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let mut next = profile.preferences().clone();
        next.current_page = 50;
        assert!(profile.replace(next, &surahs()).is_err());
        assert_eq!(profile.preferences().current_page, 1);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        drop(reader);
        drop(profile);
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn hard_link_and_alternate_stream_targets_are_rejected() {
        let root = folder();
        let path = root.join(STATE_FILE);
        fs::write(&path, encode(Preferences::default())).unwrap();
        let link = root.join("linked.json");
        fs::hard_link(&path, &link).unwrap();
        assert!(Profile::load(&root, &surahs()).is_err());
        fs::remove_file(link).unwrap();
        let stream = PathBuf::from(format!("{}:unknown", path.display()));
        let mut extra = File::create(&stream).unwrap();
        extra.write_all(b"authored stream").unwrap();
        drop(extra);
        assert!(Profile::load(&root, &surahs()).is_err());
        fs::remove_file(stream).unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn missing_child_directory_is_created_under_pinned_ancestry_only() {
        let root = folder();
        let child = root.join("SlintProfile");
        let profile = Profile::load(&child, &surahs()).unwrap();
        assert!(child.is_dir());
        assert!(Profile::load(&child.join(".."), &surahs()).is_err());
        assert!(Profile::load(&root.join("bad."), &surahs()).is_err());
        assert!(!root.join("bad.").exists());
        drop(profile);
        fs::remove_dir(child).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn same_thread_and_other_thread_owners_are_excluded_until_drop() {
        let root = folder();
        let owner = Profile::load(&root, &surahs()).unwrap();
        assert!(Profile::load(&root, &surahs()).is_err());
        let other_root = root.clone();
        let blocked = std::thread::spawn(move || Profile::load(&other_root, &surahs()).is_err())
            .join()
            .unwrap();
        assert!(blocked);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        drop(owner);
        let reopened = Profile::load(&root, &surahs()).unwrap();
        drop(reopened);
        fs::remove_dir(root).unwrap();
    }
}
