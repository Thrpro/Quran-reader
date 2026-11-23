//! The recitation engine on every platform: ayah clips and whole-surah
//! recordings, fetched from the providers in PRIVACY.md, kept in the profile's
//! AudioStore in the native engine's formats, decoded in Rust, and played
//! through the system's sound output (WASAPI, CoreAudio, ALSA/PipeWire).
//!
//! Its interface matches the one audio_bridge.rs uses on the native Windows
//! engine, which a `native-audio` build still selects. Unlike Windows media
//! playback, speed changes here are made by time-stretching the sound already
//! playing, so the recitation never pauses when the speed changes.
mod decode;
mod net;
mod output;
mod player;
pub mod qa;
mod store;
mod stretch;

pub use player::{Audio, ListeningRange, Transport};
pub use store::{AudioCache, COUNTS, Event as ClipEvent, RecordingEvent};
// For the tafsir library on Linux and macOS, and its tests on Windows.
#[cfg(any(test, not(windows)))]
pub use store::sha256;

pub type ClipJob = store::Job<ClipEvent>;
pub type RecordingJob = store::Job<RecordingEvent>;
