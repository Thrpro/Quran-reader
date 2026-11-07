use crate::{data::Result, file_integrity};
use std::{path::Path, time::Instant};
use windows::Win32::System::{ProcessStatus::*, Threading::GetCurrentProcess};
pub(crate) fn memory() -> Result<PROCESS_MEMORY_COUNTERS> {
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of_val(&counters) as u32,
        )?;
    }
    Ok(counters)
}
pub fn verify(path: &Path, output: &Path) -> Result<()> {
    let before = memory()?;
    let start = Instant::now();
    let verified = file_integrity::inspect(path)?;
    let milliseconds = start.elapsed().as_secs_f64() * 1000.;
    let info = verified.info;
    let length = info.bytes;
    let hash = &verified.sha256;
    let after = memory()?;
    let timing_path = output.join("timings.json");
    let timed = if timing_path.exists() {
        if std::fs::metadata(&timing_path)?.len() > 1024 * 1024 {
            return Err("Timing table too large".into());
        }
        let raw = std::fs::read(&timing_path)?;
        let timeline = crate::timed_audio::Timeline::parse(&raw, 286)?;
        timeline.validate_duration((info.duration_seconds * 1000.).round() as u64)?;
        Some(
            serde_json::json!({"ayahs":286,"sha256":crate::audio_cache::sha256(&raw)?,"recordingDurationMatches":true,"fullChapterRange":timeline.range(1,286)?}),
        )
    } else {
        None
    };
    std::fs::create_dir_all(output)?;
    std::fs::write(
        output.join("streamed-mp3-verification.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"source":path.to_string_lossy(),"bytes":length,"sha256":hash,"frames":info.frames,"durationSeconds":info.duration_seconds,
            "parserAndHashMs":milliseconds,"beforeWorkingSetBytes":before.WorkingSetSize,"afterWorkingSetBytes":after.WorkingSetSize,"peakWorkingSetBytes":after.PeakWorkingSetSize,
            "readerBufferBytes":64*1024,"parserFrameBufferBytes":2048,"parserDiscardBufferBytes":8192,"wholeFileReadAndHashed":true,"timings":timed,
            "scope":"Complete original recording validation and hash, not playback or network interruption testing"
        }))?,
    )?;
    Ok(())
}
