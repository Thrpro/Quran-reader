//! Small, bounded native profile state. Quran resources are never written here.
//! Session timing is monotonic and never resumes across an application restart.
use crate::data::{Result, Surah, valid_reference};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};
#[cfg(any(test, not(windows)))]
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_STATE_BYTES: usize = 256 * 1024;
const MAX_BOOKMARKS: usize = 200;
const STATE_FILE: &str = "reader-state.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalMode {
    #[default]
    Pages,
    Minutes,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadingGoal {
    pub day: String,
    pub mode: GoalMode,
    pub target: u32,
    pub pages: Vec<u32>,
    pub seconds: u64,
}
impl Default for ReadingGoal {
    fn default() -> Self {
        Self {
            day: String::new(),
            mode: GoalMode::Pages,
            target: 5,
            pages: Vec::new(),
            seconds: 0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadingPreferences {
    pub show_timer: bool,
    pub reminder_enabled: bool,
    pub reminder_hour: u32,
    pub reminder_minute: u32,
}
impl Default for ReadingPreferences {
    fn default() -> Self {
        Self {
            show_timer: false,
            reminder_enabled: false,
            reminder_hour: 19,
            reminder_minute: 0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bookmark {
    pub page: u32,
    #[serde(default)]
    pub selected: Option<(u32, u32)>,
    #[serde(default)]
    pub label: String,
}
struct ReadingSession {
    last_tick: Instant,
    remainder: Duration,
    elapsed: u64,
    visible_pages: Vec<u32>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReaderState {
    pub bookmarks: Vec<Bookmark>,
    pub goal: ReadingGoal,
    pub reading: ReadingPreferences,
    pub onboarding_completed: bool,
    pub last_reminder_day: String,
    #[serde(skip)]
    session: Option<ReadingSession>,
}
impl ReaderState {
    /// The flag lets the UI explain recovery instead of failing to open.
    /// This file is separate from the original native preferences.json profile.
    pub fn load(profile: &Path, surahs: &[Surah]) -> Result<(Self, bool)> {
        let path = profile.join(STATE_FILE);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Self::default(), false));
            }
            Err(error) => return Err(error.into()),
        };
        if !regular_file(&metadata) || metadata.len() > MAX_STATE_BYTES as u64 {
            return Ok((Self::default(), true));
        }
        let file = fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.take(MAX_STATE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        let parsed = if bytes.len() <= MAX_STATE_BYTES {
            serde_json::from_slice::<Self>(&bytes).ok()
        } else {
            None
        };
        match parsed {
            Some(mut state) => {
                state.sanitize(surahs);
                Ok((state, false))
            }
            None => Ok((Self::default(), true)),
        }
    }
    pub fn save(&self, profile: &Path) -> Result<()> {
        ensure_profile_directory(profile)?;
        save_json_atomic(&profile.join(STATE_FILE), self, MAX_STATE_BYTES)
    }
    pub fn sanitize(&mut self, surahs: &[Surah]) {
        let mut pages = BTreeSet::new();
        self.bookmarks.retain_mut(|bookmark| {
            if !(1..=604).contains(&bookmark.page)
                || !pages.insert(bookmark.page)
                || bookmark
                    .selected
                    .is_some_and(|(surah, ayah)| !valid_reference(surahs, surah, ayah))
            {
                return false;
            }
            bookmark.label = bookmark
                .label
                .chars()
                .filter(|character| !character.is_control())
                .take(128)
                .collect();
            true
        });
        self.bookmarks.truncate(MAX_BOOKMARKS);
        self.goal.target = self.goal.target.clamp(
            1,
            if self.goal.mode == GoalMode::Pages {
                604
            } else {
                1440
            },
        );
        self.goal.seconds = self.goal.seconds.min(86_400);
        self.goal.pages.retain(|page| (1..=604).contains(page));
        self.goal.pages.sort_unstable();
        self.goal.pages.dedup();
        if !valid_day(&self.goal.day) {
            self.goal.day.clear();
            self.goal.pages.clear();
            self.goal.seconds = 0;
        }
        self.reading.reminder_hour = self.reading.reminder_hour.min(23);
        self.reading.reminder_minute = self.reading.reminder_minute.min(59);
        if !valid_day(&self.last_reminder_day) {
            self.last_reminder_day.clear();
        }
    }
    pub fn toggle_bookmark(
        &mut self,
        page: u32,
        selected: Option<(u32, u32)>,
        surahs: &[Surah],
    ) -> Result<bool> {
        if !(1..=604).contains(&page)
            || selected.is_some_and(|(surah, ayah)| !valid_reference(surahs, surah, ayah))
        {
            return Err("Invalid bookmark reference".into());
        }
        if let Some(index) = self.bookmarks.iter().position(|entry| entry.page == page) {
            self.bookmarks.remove(index);
            return Ok(false);
        }
        if self.bookmarks.len() >= MAX_BOOKMARKS {
            return Err("Bookmark limit reached".into());
        }
        self.bookmarks.push(Bookmark {
            page,
            selected,
            label: String::new(),
        });
        Ok(true)
    }
    pub fn start_reading(&mut self, day: &str, pages: &[u32], now: Instant) -> Result<()> {
        validate_session_input(day, pages)?;
        self.today(day);
        if self.session.is_none() {
            self.session = Some(ReadingSession {
                last_tick: now,
                remainder: Duration::ZERO,
                elapsed: 0,
                visible_pages: pages.to_vec(),
            });
        }
        Ok(())
    }
    /// Count departed pages, as in stable. A long suspend or minimized window
    /// does not silently fulfill a daily reading target.
    pub fn update_reading(
        &mut self,
        day: &str,
        pages: &[u32],
        is_visible: bool,
        now: Instant,
    ) -> Result<bool> {
        validate_session_input(day, pages)?;
        let rolled_over = self.today(day);
        let Some(session) = self.session.as_mut() else {
            return Ok(rolled_over);
        };
        let before_seconds = self.goal.seconds;
        let before_pages = self.goal.pages.len();
        let interval = now.saturating_duration_since(session.last_tick);
        if now >= session.last_tick {
            session.last_tick = now;
        }
        if is_visible && !rolled_over {
            if interval <= Duration::from_secs(30) {
                session.remainder += interval;
                let seconds = session.remainder.as_secs();
                session.remainder -= Duration::from_secs(seconds);
                self.goal.seconds = self.goal.seconds.saturating_add(seconds).min(86_400);
                session.elapsed = session.elapsed.saturating_add(seconds).min(86_400);
            } else {
                session.remainder = Duration::ZERO;
            }
            for departed in session
                .visible_pages
                .iter()
                .filter(|previous| !pages.contains(previous))
            {
                if !self.goal.pages.contains(departed) {
                    self.goal.pages.push(*departed);
                }
            }
            self.goal.pages.sort_unstable();
        } else {
            session.remainder = Duration::ZERO;
        }
        session.visible_pages = pages.to_vec();
        Ok(rolled_over
            || before_seconds != self.goal.seconds
            || before_pages != self.goal.pages.len())
    }
    pub fn stop_reading(&mut self, day: &str, pages: &[u32], now: Instant) -> Result<()> {
        self.update_reading(day, pages, true, now)?;
        self.session = None;
        Ok(())
    }
    pub fn session_seconds(&self) -> Option<u64> {
        self.session.as_ref().map(|session| session.elapsed)
    }
    pub fn goal_reached(&self, day: &str) -> bool {
        self.goal.day == day
            && match self.goal.mode {
                GoalMode::Pages => self.goal.pages.len() as u32 >= self.goal.target,
                GoalMode::Minutes => self.goal.seconds >= u64::from(self.goal.target) * 60,
            }
    }
    /// Caller supplies local calendar/time, avoiding UTC date errors. Only one
    /// in-app reminder is due each day, after its configured time.
    pub fn reminder_due(&mut self, day: &str, hour: u32, minute: u32) -> bool {
        if !valid_day(day)
            || hour > 23
            || minute > 59
            || !self.reading.reminder_enabled
            || self.session.is_some()
            || self.last_reminder_day == day
            || self.goal_reached(day)
            || hour * 60 + minute < self.reading.reminder_hour * 60 + self.reading.reminder_minute
        {
            return false;
        }
        self.last_reminder_day = day.to_owned();
        true
    }
    fn today(&mut self, day: &str) -> bool {
        if self.goal.day == day {
            return false;
        }
        self.goal.day = day.to_owned();
        self.goal.pages.clear();
        self.goal.seconds = 0;
        true
    }
}
fn validate_session_input(day: &str, pages: &[u32]) -> Result<()> {
    if !valid_day(day) || pages.len() > 2 || pages.iter().any(|page| !(1..=604).contains(page)) {
        return Err("Invalid reading session date or page".into());
    }
    Ok(())
}
fn valid_day(day: &str) -> bool {
    let bytes = day.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, value)| index != 4 && index != 7 && !value.is_ascii_digit())
    {
        return false;
    }
    let year = day[..4].parse::<u32>().unwrap_or(0);
    let month = day[5..7].parse::<u32>().unwrap_or(0);
    let date = day[8..].parse::<u32>().unwrap_or(0);
    let last = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28 + u32::from(year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)),
        _ => 0,
    };
    year >= 1970 && (1..=last).contains(&date)
}
fn regular_file(metadata: &fs::Metadata) -> bool {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}
/// Validate an existing profile without an unnecessary directory creation.
pub(crate) fn ensure_profile_directory(profile: &Path) -> Result<()> {
    fn ordinary_directory(metadata: &fs::Metadata) -> bool {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return false;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return false;
            }
        }
        true
    }
    match fs::symlink_metadata(profile) {
        Ok(metadata) if ordinary_directory(&metadata) => Ok(()),
        Ok(_) => Err("Native profile path must be a directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(profile)?;
            if ordinary_directory(&fs::symlink_metadata(profile)?) {
                Ok(())
            } else {
                Err("Native profile path must be a directory".into())
            }
        }
        Err(error) => Err(error.into()),
    }
}
pub(crate) fn save_json_atomic<T: Serialize>(path: &Path, value: &T, limit: usize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() > limit {
        return Err("Native profile state exceeds its size limit".into());
    }
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !regular_file(&metadata) {
            return Err("Native profile state must be a regular file".into());
        }
    }
    #[cfg(windows)]
    {
        crate::profile_file::write_atomic(path, &bytes)
    }
    #[cfg(not(windows))]
    {
        use std::io::Write;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let filename = path
            .file_name()
            .ok_or("Missing profile filename")?
            .to_string_lossy();
        let temporary =
            path.with_file_name(format!("{filename}.{}.{}.tmp", std::process::id(), nonce));
        struct TemporaryFile(PathBuf);
        impl Drop for TemporaryFile {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        let _cleanup = TemporaryFile(temporary.clone());
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        // The existing profile survives serialization, write or flush failure;
        // there is no delete gap.
        fs::rename(temporary, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn surahs() -> Vec<Surah> {
        crate::data::Resources::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public-qul"),
        )
        .unwrap()
        .surahs
    }
    fn temporary_profile() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "native-reader-state-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }
    #[test]
    fn profile_directory_is_created_only_when_missing_and_file_targets_are_preserved() {
        let root = temporary_profile();
        let profile = root.join("new-profile");
        ensure_profile_directory(&profile).unwrap();
        assert!(profile.is_dir());
        let file = root.join("existing-file");
        fs::write(&file, b"authored fixture bytes").unwrap();
        assert!(ensure_profile_directory(&file).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"authored fixture bytes");
        fs::remove_file(file).unwrap();
        fs::remove_dir(profile).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn atomic_profile_saves_work_while_scoped_audio_ancestry_is_pinned() {
        let profile = temporary_profile();
        let audio = profile.join("AudioStore");
        let namespace = audio.join("ScopedContinuous");
        fs::create_dir_all(&namespace).unwrap();
        let prefs = profile.join("preferences.json");
        save_json_atomic(&prefs, &serde_json::json!({"volume":0.8}), 4096).unwrap();
        ReaderState::default().save(&profile).unwrap();
        let pin = crate::owned_file::PinnedDirectory::open(&namespace).unwrap();
        ensure_profile_directory(&profile).unwrap();
        for value in [0.6, 0.4] {
            save_json_atomic(&prefs, &serde_json::json!({"volume":value}), 4096).unwrap();
            ReaderState::default().save(&profile).unwrap();
        }
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&prefs).unwrap()).unwrap();
        assert_eq!(saved["volume"], 0.4);
        assert!(fs::read_dir(&profile).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
        // The fix must retain the audio namespace's replacement protection.
        assert!(fs::rename(&profile, profile.with_extension("replacement")).is_err());
        drop(pin);
        fs::remove_file(prefs).unwrap();
        fs::remove_file(profile.join(STATE_FILE)).unwrap();
        fs::remove_dir(namespace).unwrap();
        fs::remove_dir(audio).unwrap();
        fs::remove_dir(profile).unwrap();
    }
    #[test]
    fn monotonic_reading_excludes_hidden_time_and_suspension_and_counts_departed_pages() {
        let mut state = ReaderState::default();
        let now = Instant::now();
        state.start_reading("2026-10-11", &[45, 46], now).unwrap();
        state
            .update_reading("2026-10-11", &[47, 48], true, now + Duration::from_secs(5))
            .unwrap();
        assert_eq!(state.goal.pages, [45, 46]);
        assert_eq!(state.goal.seconds, 5);
        state
            .update_reading(
                "2026-10-11",
                &[47, 48],
                false,
                now + Duration::from_secs(20),
            )
            .unwrap();
        state
            .update_reading("2026-10-11", &[47, 48], true, now + Duration::from_secs(21))
            .unwrap();
        state
            .update_reading(
                "2026-10-11",
                &[47, 48],
                true,
                now + Duration::from_secs(400),
            )
            .unwrap();
        assert_eq!(state.goal.seconds, 6);
        assert_eq!(state.session_seconds(), Some(6));
        state
            .update_reading(
                "2026-10-12",
                &[47, 48],
                true,
                now + Duration::from_secs(401),
            )
            .unwrap();
        assert!(state.goal.pages.is_empty());
        assert_eq!(state.goal.seconds, 0);
    }
    #[test]
    fn fractional_ticks_are_counted_and_backward_instants_are_harmless() {
        let mut state = ReaderState::default();
        let now = Instant::now();
        state.start_reading("2026-10-11", &[1, 2], now).unwrap();
        for i in 1..=10 {
            state
                .update_reading(
                    "2026-10-11",
                    &[1, 2],
                    true,
                    now + Duration::from_millis(i * 100),
                )
                .unwrap();
        }
        assert_eq!(state.goal.seconds, 1);
        state
            .update_reading("2026-10-11", &[1, 2], true, now)
            .unwrap();
        assert_eq!(state.goal.seconds, 1);
        state
            .update_reading("2026-10-11", &[1, 2], true, now + Duration::from_secs(2))
            .unwrap();
        assert_eq!(state.goal.seconds, 2);
    }
    #[test]
    fn reminder_is_opt_in_once_per_local_day_and_not_due_after_goal() {
        let mut state = ReaderState::default();
        assert!(!state.reminder_due("2026-10-11", 20, 0));
        state.reading.reminder_enabled = true;
        assert!(!state.reminder_due("2026-10-11", 18, 59));
        assert!(state.reminder_due("2026-10-11", 19, 0));
        assert!(!state.reminder_due("2026-10-11", 23, 59));
        state.goal.day = "2026-10-12".into();
        state.goal.pages = vec![1, 2, 3, 4, 5];
        assert!(!state.reminder_due("2026-10-12", 20, 0));
        assert!(!state.reminder_due("2026-02-30", 20, 0));
        assert!(valid_day("2028-02-29"));
        assert!(!valid_day("2026-02-29"));
    }
    #[test]
    fn persistence_survives_replacement_and_never_resumes_a_session() {
        let profile = temporary_profile();
        let chapters = surahs();
        let mut state = ReaderState::default();
        assert!(
            state
                .toggle_bookmark(45, Some((2, 269)), &chapters)
                .unwrap()
        );
        state.onboarding_completed = true;
        state.reading.show_timer = true;
        state
            .start_reading("2026-10-11", &[45, 46], Instant::now())
            .unwrap();
        state.save(&profile).unwrap();
        state.goal.seconds = 70;
        state.save(&profile).unwrap();
        let (saved, recovered) = ReaderState::load(&profile, &chapters).unwrap();
        assert!(!recovered);
        assert!(saved.onboarding_completed && saved.reading.show_timer);
        assert_eq!(saved.bookmarks[0].selected, Some((2, 269)));
        assert_eq!(saved.goal.seconds, 70);
        assert_eq!(saved.session_seconds(), None);
        assert_eq!(fs::read_dir(&profile).unwrap().count(), 1);
        fs::remove_dir_all(profile).unwrap();
    }
    #[test]
    fn malformed_and_large_state_recover_and_invalid_references_are_bounded() {
        let profile = temporary_profile();
        let chapters = surahs();
        fs::write(profile.join(STATE_FILE), b"{broken").unwrap();
        assert!(ReaderState::load(&profile, &chapters).unwrap().1);
        fs::write(profile.join(STATE_FILE), b"{}").unwrap();
        assert!(!ReaderState::load(&profile, &chapters).unwrap().1);
        let file = fs::File::create(profile.join(STATE_FILE)).unwrap();
        file.set_len(MAX_STATE_BYTES as u64 + 1).unwrap();
        drop(file);
        assert!(ReaderState::load(&profile, &chapters).unwrap().1);
        let mut state = ReaderState {
            bookmarks: vec![
                Bookmark {
                    page: 45,
                    selected: Some((2, 269)),
                    label: "x\n".repeat(200),
                },
                Bookmark {
                    page: 45,
                    selected: None,
                    label: String::new(),
                },
                Bookmark {
                    page: 604,
                    selected: Some((115, 1)),
                    label: String::new(),
                },
            ],
            ..Default::default()
        };
        state.goal.target = u32::MAX;
        state.goal.day = "not a day".into();
        state.goal.seconds = u64::MAX;
        state.sanitize(&chapters);
        assert_eq!(state.bookmarks.len(), 1);
        assert_eq!(state.bookmarks[0].label.len(), 128);
        assert_eq!(state.goal.target, 604);
        assert_eq!(state.goal.seconds, 0);
        assert!(state.toggle_bookmark(1, Some((0, 1)), &chapters).is_err());
        assert!(
            state
                .start_reading("2026-10-11", &[605], Instant::now())
                .is_err()
        );
        fs::remove_dir_all(profile).unwrap();
    }
    #[test]
    fn bookmark_limit_and_failed_save_preserve_existing_state() {
        let profile = temporary_profile();
        let chapters = surahs();
        let mut state = ReaderState::default();
        for page in 1..=MAX_BOOKMARKS as u32 {
            assert!(state.toggle_bookmark(page, None, &chapters).unwrap());
        }
        assert!(state.toggle_bookmark(201, None, &chapters).is_err());
        assert!(!state.toggle_bookmark(1, None, &chapters).unwrap());
        assert!(state.toggle_bookmark(201, None, &chapters).unwrap());
        state.save(&profile).unwrap();
        let path = profile.join(STATE_FILE);
        let before = fs::read(&path).unwrap();
        assert!(save_json_atomic(&path, &state, 1).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let directory = profile.join("directory.json");
        fs::create_dir(&directory).unwrap();
        assert!(save_json_atomic(&directory, &state, MAX_STATE_BYTES).is_err());
        assert!(directory.is_dir());
        assert_eq!(fs::read_dir(&profile).unwrap().count(), 2);
        fs::remove_dir_all(profile).unwrap();
    }
}
