//! `--qa-recitation <folder>`: plays real recitations through the sound
//! device and records what happened, for checking the engine without
//! listening. Writes `recitation-qa.json` and the exact sound played as WAV.
use super::{Audio, AudioCache, ListeningRange, store::COUNTS};
use crate::data::Result;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

struct Step {
    at: Duration,
    action: Action,
}
enum Action {
    Speed(f64),
    Seek(i64),
    Pause,
    Resume,
}

pub fn run(out: &Path) -> Result<()> {
    fs::create_dir_all(out)?;
    let cache = AudioCache::new(out.join("profile"))?;
    let mut audio = Audio::with_cache(cache)?;
    let report = vec![
        // Ayah clips: Maher Al-Muaiqly, Al-Ikhlas from the opening basmala.
        scenario(
            &mut audio,
            out,
            "clips",
            ListeningRange {
                reciter: 1,
                surah: 112,
                first: 1,
                last: 4,
                count: COUNTS[111],
            },
            false,
            &[
                Step {
                    at: Duration::from_millis(6000),
                    action: Action::Speed(1.5),
                },
                Step {
                    at: Duration::from_millis(9000),
                    action: Action::Speed(0.75),
                },
            ],
        )?,
        // A whole-surah recording streamed while it plays: Yasser Al-Dosari,
        // Al-Mulk 1-5, with speed changes, a pause and seeks.
        scenario(
            &mut audio,
            out,
            "recording",
            ListeningRange {
                reciter: 0,
                surah: 67,
                first: 1,
                last: 5,
                count: COUNTS[66],
            },
            true,
            &[
                Step {
                    at: Duration::from_millis(5000),
                    action: Action::Speed(1.75),
                },
                Step {
                    at: Duration::from_millis(8000),
                    action: Action::Pause,
                },
                Step {
                    at: Duration::from_millis(9000),
                    action: Action::Resume,
                },
                Step {
                    at: Duration::from_millis(11000),
                    action: Action::Speed(1.),
                },
                Step {
                    at: Duration::from_millis(13000),
                    action: Action::Seek(10_000),
                },
                Step {
                    at: Duration::from_millis(16000),
                    action: Action::Seek(-5_000),
                },
            ],
        )?,
    ];
    let downloads = downloads(out)?;
    fs::write(
        out.join("recitation-qa.json"),
        serde_json::to_vec_pretty(&json!({ "scenarios": report, "downloads": downloads }))?,
    )?;
    Ok(())
}

/// Saves Al-Ikhlas as ayah clips (Maher Al-Muaiqly) and as a whole recording
/// (Yasser Al-Dosari), then lists the downloads as the app does.
fn downloads(out: &Path) -> Result<Value> {
    let profile = out.join("profile");
    let cache = AudioCache::new(profile.clone())?;
    let began = Instant::now();
    let clips = cache.download_surah(1, 112, 7)?;
    let clip_result = loop {
        match clips.try_recv() {
            Some(super::ClipEvent::Complete { generation, .. }) => {
                break format!("complete {generation}");
            }
            Some(super::ClipEvent::Failed { message, .. }) => break format!("failed: {message}"),
            Some(super::ClipEvent::Cancelled { .. }) => break "cancelled".into(),
            Some(_) => {}
            None if began.elapsed() > Duration::from_secs(120) => break "timed out".into(),
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let clip_seconds = began.elapsed().as_secs_f64();
    let began = Instant::now();
    let recording = cache.timed_store()?.prepare(0, 112, 8)?;
    let mut progress = 0;
    let recording_result = loop {
        match recording.try_recv() {
            Some(super::RecordingEvent::Ready(ready)) => {
                break format!("ready {}", ready.generation);
            }
            Some(super::RecordingEvent::Failed(_, message)) => break format!("failed: {message}"),
            Some(super::RecordingEvent::Progress(..)) => progress += 1,
            None if began.elapsed() > Duration::from_secs(120) => break "timed out".into(),
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let listed: Vec<Value> = crate::audio_bridge::scan_downloads(&profile)
        .into_iter()
        .map(|d| {
            json!({"reciter": d.reciter, "surah": d.surah, "bytes": d.bytes,
                   "continuous": d.continuous, "complete": d.complete})
        })
        .collect();
    Ok(json!({
        "clips": clip_result, "clipSeconds": clip_seconds,
        "recording": recording_result, "recordingSeconds": began.elapsed().as_secs_f64(),
        "recordingProgressEvents": progress,
        "listed": listed,
    }))
}

fn scenario(
    audio: &mut Audio,
    out: &Path,
    name: &str,
    range: ListeningRange,
    continuous: bool,
    steps: &[Step],
) -> Result<Value> {
    {
        let mut queue = audio.shared.lock();
        queue.tap = Some(Vec::new());
        queue.starved = 0;
    }
    let began = Instant::now();
    if continuous {
        audio.play_timed_scoped(range, 1., 0.1)?;
    } else {
        audio.play_range(range, 1., 0.1)?;
    }
    let mut samples = Vec::new();
    let mut events = Vec::new();
    let mut next = 0;
    let mut first_sound = None;
    let mut last_key = None;
    let limit = Duration::from_secs(150);
    while began.elapsed() < limit {
        audio.poll()?;
        let elapsed = began.elapsed();
        // Actions run relative to the first sound, not to the request.
        if let Some(sound) = first_sound
            && let Some(step) = steps.get(next)
            && elapsed >= sound + step.at
        {
            let (label, result) = match step.action {
                Action::Speed(rate) => (
                    format!("speed {rate}"),
                    audio.request_speed(rate).map(|_| true),
                ),
                Action::Seek(ms) => (format!("seek {ms:+} ms"), audio.seek(Some(ms), 0)),
                Action::Pause => ("pause".into(), audio.toggle().map(|_| true)),
                Action::Resume => ("resume".into(), audio.toggle().map(|_| true)),
            };
            events.push(json!({"ms": elapsed.as_millis() as u64, "action": label, "ok": result.is_ok_and(|done| done)}));
            next += 1;
        }
        let transport = audio.transport();
        let current = audio.current();
        let (played, starved) = {
            let queue = audio.shared.lock();
            (queue.played, queue.starved)
        };
        if transport.is_some() && first_sound.is_none() {
            first_sound = Some(elapsed);
        }
        if current != last_key {
            events.push(json!({"ms": elapsed.as_millis() as u64, "ayah": current.map(|k| k.1)}));
            last_key = current;
        }
        samples.push(json!({
            "ms": elapsed.as_millis() as u64,
            "played": played,
            "starved": starved,
            "position": transport.map(|t| t.offset_ms + t.position_ms),
            "ayah": current.map(|k| k.1),
            "opening": audio.opening_surah(),
            "paused": audio.is_paused(),
        }));
        if audio.completed() || !audio.needs_fast_poll() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let completed = audio.completed();
    let failed = audio.has_session() && !completed && !audio.needs_fast_poll();
    let starved_before_end = audio.shared.lock().starved;
    let tap = audio.shared.lock().tap.take().unwrap_or_default();
    audio.stop()?;
    let rate = audio.output.as_ref().map_or(48_000, |o| o.sample_rate);
    write_wav(&out.join(format!("{name}.wav")), &tap, rate)?;
    Ok(json!({
        "name": name,
        "completed": completed,
        "failed": failed,
        "secondsToFirstSound": first_sound.map(|d| d.as_secs_f64()),
        "elapsedSeconds": began.elapsed().as_secs_f64(),
        "starvedFrames": starved_before_end,
        "outputRate": rate,
        "playedSeconds": tap.len() as f64 / 2. / rate as f64,
        "events": events,
        "samples": samples,
    }))
}

/// 16-bit stereo WAV.
fn write_wav(path: &Path, samples: &[f32], rate: u32) -> Result<()> {
    let mut file = fs::File::create(path)?;
    let data = (samples.len() * 2) as u32;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(&(rate * 4).to_le_bytes())?;
    file.write_all(&4u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data.to_le_bytes())?;
    let bytes: Vec<u8> = samples
        .iter()
        .flat_map(|v| ((v.clamp(-1., 1.) * 32767.) as i16).to_le_bytes())
        .collect();
    file.write_all(&bytes)?;
    Ok(())
}
