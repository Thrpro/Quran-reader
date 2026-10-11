use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
pub fn initialize(directory: &std::path::Path) {
    let _ = fs::create_dir_all(directory);
    let _ = LOG_PATH.set(directory.join("rust-host.log"));
}

// Startup and host errors only; never include Quran text, preferences or audio URLs.
// At most two 1 MiB files are retained. Logging failure must not stop the reader.
pub fn write(message: &str) {
    let Some(path) = LOG_PATH.get() else { return };
    if fs::metadata(path).is_ok_and(|meta| meta.len() >= 1024 * 1024) {
        let previous = path.with_file_name("rust-host.previous.log");
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(path, previous);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(
            file,
            "{seconds} {}",
            message.chars().take(1024).collect::<String>()
        );
    }
}
