//! Outer-message-loop observation of real large-recording scoped playback.
//! This helper never owns a player, pumps messages, sleeps, scans storage,
//! downloads, hashes or edits an original recording. App owns the actual plan.
use crate::{audio::Audio, audio_cache, data::Result, mp3};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const ORIGINAL_LARGE_BYTES: u64 = 115_965_282;
const ORIGINAL_LARGE_SHA256: &str =
    "d390ae18bf320e6073e4736a72e8dd4a24ea363a80ab4696281e2b990d2aeb25";
const DEADLINE: Duration = Duration::from_secs(420);
const MAX_OBSERVATIONS: usize = 512;
const MAX_HEARTBEATS: usize = 4096;

pub struct State {
    output: PathBuf,
    began: Instant,
    last_observe: Option<Instant>,
    observed_large_ayah: Option<f64>,
    decoded_large_completion: Option<f64>,
    observed_next_ayah: Option<f64>,
    sources: Vec<Value>,
    observations: Vec<Value>,
    observe_intervals_ms: Vec<f64>,
    pending_heartbeat: Option<Instant>,
    heartbeat_latencies_ms: Vec<f64>,
    heartbeat_samples_omitted: u32,
    done: bool,
}

impl State {
    /// `out` is the caller's fresh isolated QA output, not a recording folder.
    pub fn new(out: &Path) -> Result<Self> {
        audio_cache::safe_directory(out)?;
        let output = out.join("large-handoff-verification.json");
        if output.exists() {
            return Err("Preserve the existing large recording handoff receipt".into());
        }
        Ok(Self {
            output,
            began: Instant::now(),
            last_observe: None,
            observed_large_ayah: None,
            decoded_large_completion: None,
            observed_next_ayah: None,
            sources: Vec::with_capacity(2),
            observations: Vec::with_capacity(MAX_OBSERVATIONS),
            observe_intervals_ms: Vec::with_capacity(MAX_OBSERVATIONS),
            pending_heartbeat: None,
            heartbeat_latencies_ms: Vec::with_capacity(MAX_HEARTBEATS),
            heartbeat_samples_omitted: 0,
            done: false,
        })
    }

    /// Call from App::tick immediately after genuine Audio::completed and
    /// before the production captured plan advances. This is a polled decoded
    /// completion observation, not a decoder callback's timestamp.
    pub fn record_decoded_completion(&mut self, surah: u32) -> Result<()> {
        if self.done {
            return Ok(());
        }
        if surah != 2 {
            return self.fail(format!(
                "Unexpected decoded chapter completion in large handoff QA: {surah}"
            ));
        }
        if self.observed_large_ayah.is_none() {
            return self.fail("Large original ayah was not observed before completion".into());
        }
        self.decoded_large_completion
            .get_or_insert_with(|| self.began.elapsed().as_secs_f64());
        Ok(())
    }

    /// The caller posts one private window message only when this returns true.
    /// One pending timestamp prevents backlog. No OS input is synthesized.
    pub fn posted_heartbeat(&mut self) -> bool {
        if self.done || self.pending_heartbeat.is_some() {
            return false;
        }
        self.pending_heartbeat = Some(Instant::now());
        true
    }

    /// Call only upon dispatch of the caller's private heartbeat message.
    pub fn ack_heartbeat(&mut self) {
        if let Some(posted) = self.pending_heartbeat.take() {
            if self.heartbeat_latencies_ms.len() < MAX_HEARTBEATS {
                self.heartbeat_latencies_ms
                    .push(posted.elapsed().as_secs_f64() * 1000.);
            } else {
                self.heartbeat_samples_omitted = self.heartbeat_samples_omitted.saturating_add(1);
            }
        }
    }

    /// Observe the App-owned production player from the regular outer QA timer.
    /// Returns true only after genuine chapter 3 ayah 1 is reported while the
    /// native decoder is Playing and chapter 2's decoded completion was seen.
    /// The caller then stops the actual plan, captures its view, and waits for
    /// asynchronous owned-file retirement separately.
    pub fn observe(&mut self, audio: &Audio) -> Result<bool> {
        if self.done {
            return Ok(true);
        }
        let now = Instant::now();
        if now.duration_since(self.began) > DEADLINE {
            return self.fail("Large recording handoff exceeded 420 seconds".into());
        }
        if self.observations.len() >= MAX_OBSERVATIONS {
            return self.fail("Large recording handoff observation limit exceeded".into());
        }
        if let Some(previous) = self.last_observe.replace(now) {
            self.observe_intervals_ms
                .push(now.duration_since(previous).as_secs_f64() * 1000.);
        }
        let elapsed = now.duration_since(self.began).as_secs_f64();
        let current = audio.current();
        let status = audio.status(false);
        let source = audio.qa_timed_source();
        self.observations.push(json!({
            "elapsedSeconds": elapsed, "current": current, "status": status,
            "sourceSurah": source.as_ref().and_then(|value| value["surah"].as_u64()),
            "completed": audio.completed(), "hasSession": audio.has_session(),
        }));
        if status.starts_with("Could not") {
            return self.fail(format!("Native large recording handoff failed: {status}"));
        }
        if let Some(source) = source {
            self.observe_source(source, elapsed)?;
        }
        match current {
            Some((2, 286)) => {
                if self.sources.first().is_none_or(|value| value["surah"] != 2) {
                    return self.fail("Large ayah was reported without its verified source".into());
                }
                if status.contains("Playing") {
                    self.observed_large_ayah.get_or_insert(elapsed);
                }
            }
            Some((3, 1)) => {
                if self.observed_large_ayah.is_none()
                    || self.decoded_large_completion.is_none()
                    || self.sources.len() != 2
                    || self.sources[1]["surah"] != 3
                {
                    return self.fail(
                        "Next ayah appeared without original tail and decoded completion proof"
                            .into(),
                    );
                }
                if status.contains("Playing") {
                    self.observed_next_ayah = Some(elapsed);
                    self.write_report(true, None)?;
                    self.done = true;
                    return Ok(true);
                }
            }
            Some((surah, ayah)) => {
                return self.fail(format!(
                    "Large handoff changed its captured reference to {surah}:{ayah}"
                ));
            }
            None => {}
        }
        Ok(false)
    }

    fn observe_source(&mut self, source: Value, elapsed: f64) -> Result<()> {
        let surah = source["surah"]
            .as_u64()
            .ok_or("Timed source has no chapter reference")?;
        let bytes = source["bytes"].as_u64().ok_or("Timed source has no size")?;
        let hash = source["sha256"]
            .as_str()
            .ok_or("Timed source has no verified hash")?;
        let path = source["path"].as_str().ok_or("Timed source has no path")?;
        if ![2, 3].contains(&surah)
            || bytes == 0
            || bytes > mp3::MAX_CHAPTER_BYTES
            || hash.len() != 64
            || !hash.bytes().all(|value| value.is_ascii_hexdigit())
            || path.is_empty()
        {
            return self.fail("Invalid source metadata during large recording handoff".into());
        }
        if surah == 2
            && (bytes != ORIGINAL_LARGE_BYTES || !hash.eq_ignore_ascii_case(ORIGINAL_LARGE_SHA256))
        {
            return self.fail("Large recording differs from the preserved authentic source".into());
        }
        if let Some(previous) = self.sources.last() {
            if previous["surah"] == surah {
                if previous["source"] != source {
                    return self.fail("Active original recording metadata changed".into());
                }
                return Ok(());
            }
        }
        if self.sources.len() >= 2
            || (self.sources.is_empty() && surah != 2)
            || (self.sources.len() == 1 && surah != 3)
        {
            return self.fail("Unexpected original source sequence during large handoff".into());
        }
        let owned = Path::new(path).components().any(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("ScopedContinuous")
        });
        self.sources.push(json!({
            "surah": surah, "observedAttachedSourceAtSeconds": elapsed,
            "storageNamespace": if owned { "owned rolling" } else { "persistent borrowed" },
            "source": source,
        }));
        Ok(())
    }

    fn fail<T>(&mut self, message: String) -> Result<T> {
        self.write_report(false, Some(&message))?;
        self.done = true;
        Err(message.into())
    }

    fn write_report(&self, passed: bool, error: Option<&str>) -> Result<()> {
        let handoff = self
            .observed_next_ayah
            .zip(self.decoded_large_completion)
            .map(|(next, end)| (next - end) * 1000.);
        let report = json!({
            "passed": passed, "error": error,
            "elapsedSeconds": self.began.elapsed().as_secs_f64(),
            "muted": true, "requestedRate": 2.0,
            "start": [2, 286], "observedNext": [3, 1],
            "originalLargeBytes": ORIGINAL_LARGE_BYTES,
            "originalLargeSha256": ORIGINAL_LARGE_SHA256,
            "originalTailPlayingObservedAtSeconds": self.observed_large_ayah,
            "decodedChapterCompletionPolledAtSeconds": self.decoded_large_completion,
            "nextChapterFirstAyahPlayingObservedAtSeconds": self.observed_next_ayah,
            "observedCompletionToNextAyahMilliseconds": handoff,
            "sources": self.sources, "observations": self.observations,
            "observeIntervalsMilliseconds": stats(&self.observe_intervals_ms),
            "postedMessageHeartbeat": {
                "measurement": "Private window message posting to dispatch latency; not physical input latency, compositor timing or displayed FPS.",
                "statisticsMilliseconds": stats(&self.heartbeat_latencies_ms),
                "rawMilliseconds": self.heartbeat_latencies_ms,
                "samplesOmittedAtBound": self.heartbeat_samples_omitted,
                "pendingMessageAtReport": self.pending_heartbeat.is_some(),
            },
            "productionPlanAdvanced": self.decoded_large_completion.is_some() && self.observed_next_ayah.is_some(),
            "scope": "Actual App-owned continuous Quran scope through unchanged large Al-Baqarah recording ayah286, production chapter advancement and next chapter ayah1. Stop is requested after this observation; full-Quran completion and subsequent cleanup are outside this receipt.",
            "limits": [
                "900ms outer QA timer samples source/current/state. Source observed times are sampled UI attachment observations, not worker Ready timestamps.",
                "Decoded end is recorded by regular App polling before actual plan advancement, not by a timed substitute or exact decoder callback timestamp.",
                "Observed completion-to-next-ayah includes network/preparation/decode plus sampling delay; it is not exact audible-gap or gapless evidence.",
                "Requested rate is supplied by the App fixture. Native decoder rate retention and external disk/memory high-water measurements are separate evidence.",
                "Recording hashes and sizes come from the existing production complete-file validator; this helper performs no hashing on the UI thread.",
                "The actual full plan remains captured by App; this test observes only2:286 through3:1 before deliberate Stop.",
            ],
        });
        // Receipt is new, bounded QA metadata. Preserve every earlier attempt.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.output)?;
        file.write_all(&serde_json::to_vec_pretty(&report)?)?;
        file.sync_all()?;
        Ok(())
    }
}

fn stats(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({"count":0});
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let count = sorted.len();
    let median = if count % 2 == 0 {
        (sorted[count / 2 - 1] + sorted[count / 2]) / 2.
    } else {
        sorted[count / 2]
    };
    let rank95 = (count * 95).div_ceil(100) - 1;
    json!({"count":count,"minimum":sorted[0],"median":median,
        "p95":sorted[rank95],"maximum":sorted[count-1],
        "p95Method":"Nearest rank ceil(0.95*n)"})
}

#[cfg(test)]
mod tests {
    #[test]
    fn timing_statistics_handle_empty_singleton_and_outlier() {
        assert_eq!(super::stats(&[])["count"], 0);
        assert_eq!(super::stats(&[7.])["p95"], 7.);
        let mut values = vec![1.; 20];
        values[19] = 400.;
        let result = super::stats(&values);
        assert_eq!(result["median"], 1.);
        assert_eq!(result["p95"], 1.);
        assert_eq!(result["maximum"], 400.);
    }
}
