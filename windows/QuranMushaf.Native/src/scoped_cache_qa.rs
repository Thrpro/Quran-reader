//! QA fixture preparation through the unchanged production recording adapters.
//! No decoder, presentation window, synthesized audio or hand-written receipts.
use crate::{
    audio_cache::{self, AudioCache, ClipKey, Event},
    data::{Resources, Result},
    mp3, timed_store,
};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

pub fn prepare(resources: &Resources, out: &Path) -> Result<()> {
    audio_cache::safe_directory(out)?;
    let report_path = out.join("prepared-scoped-cache.json");
    if report_path.exists() {
        return Err(
            "Preserve the existing scoped cache receipt; use a fresh preparation output".into(),
        );
    }
    if resources
        .surahs
        .iter()
        .map(|surah| surah.verses_count)
        .collect::<Vec<_>>()
        != audio_cache::COUNTS
    {
        return Err("Bundled authentic surah counts do not match recording validation".into());
    }
    let began = Instant::now();
    let profile = out.join("profile");
    let cache = AudioCache::new(profile)?;
    let references = [
        (1, 1),
        (112, 4),
        (113, 1),
        (113, 2),
        (113, 3),
        (113, 4),
        (113, 5),
        (114, 1),
        (114, 2),
        (114, 3),
        (114, 4),
        (114, 5),
        (114, 6),
    ];
    let keys = references
        .iter()
        .map(|(surah, ayah)| ClipKey::new(0, *surah, *ayah))
        .collect::<Result<Vec<_>>>()?;
    let generation = 101;
    let job = cache.prepare_playback(keys.clone(), generation)?;
    let mut clips = BTreeMap::new();
    loop {
        if began.elapsed() > Duration::from_secs(180) {
            return Err("Original scoped clip preparation timed out".into());
        }
        if let Some(event) = job.try_recv() {
            match event {
                Event::Ready {
                    generation: value,
                    index,
                    key,
                    path,
                    info,
                } => {
                    if value != generation
                        || keys.get(index) != Some(&key)
                        || clips.contains_key(&index)
                    {
                        return Err("Scoped clip preparation changed or duplicated its captured source reference".into());
                    }
                    let bytes = audio_cache::read_bounded(&path, mp3::MAX_AUDIO_BYTES)?;
                    let sha256 = audio_cache::sha256(&bytes)?;
                    let receipt_path = cache
                        .folder()
                        .join("Cache")
                        .join(format!("r00-s{:03}-a{:03}.json", key.surah, key.ayah));
                    let receipt: serde_json::Value =
                        serde_json::from_slice(&audio_cache::read_bounded(&receipt_path, 8192)?)?;
                    if receipt["sha256"].as_str() != Some(sha256.as_str())
                        || receipt["bytes"].as_u64() != Some(info.bytes)
                        || receipt["frames"].as_u64() != Some(u64::from(info.frames))
                    {
                        return Err("Adapter-produced scoped clip receipt does not match its original bytes".into());
                    }
                    clips.insert(index, serde_json::json!({"surah":key.surah,"ayah":key.ayah,
                        "bytes":info.bytes,"frames":info.frames,"durationSeconds":info.duration_seconds,
                        "sha256":sha256,"productionReceipt":receipt}));
                    // This is a fixture downloader, not playback. Consume each
                    // validated result so the production buffer remains bounded.
                    job.set_consumed(index);
                }
                Event::Complete {
                    generation: value, ..
                } if value == generation => break,
                Event::Failed {
                    generation: value,
                    message,
                } if value == generation => {
                    return Err(
                        format!("Original scoped clip preparation failed: {message}").into(),
                    );
                }
                _ => {}
            }
        } else {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    if clips.len() != references.len() {
        return Err("Scoped preparation did not validate every original clip".into());
    }
    drop(job);
    let clips: Vec<_> = clips.into_values().collect();
    let store = cache.timed_store()?;
    let mut recordings = Vec::new();
    for surah in [112, 113, 114] {
        let generation = 200 + u64::from(surah);
        let job = store.prepare(0, surah, generation)?;
        let began_chapter = Instant::now();
        loop {
            if began_chapter.elapsed() > Duration::from_secs(180) {
                return Err(format!("Original chapter {surah} preparation timed out").into());
            }
            match job.try_recv() {
                Some(timed_store::Event::Ready(ready)) => {
                    if ready.generation != generation || ready.surah != surah {
                        return Err(
                            "Scoped recording preparation changed its source reference".into()
                        );
                    }
                    let folder = ready
                        .path
                        .parent()
                        .ok_or("Recording has no source folder")?;
                    let receipt: serde_json::Value = serde_json::from_slice(
                        &audio_cache::read_bounded(&folder.join("receipt.json"), 8192)?,
                    )?;
                    recordings.push(serde_json::json!({"surah":surah,"read":92,
                        "bytes":ready._verified.info.bytes,"frames":ready._verified.info.frames,
                        "durationSeconds":ready._verified.info.duration_seconds,
                        "sha256":ready._verified.sha256,"productionReceipt":receipt,
                        "originalTimingCount":resources.surahs[(surah-1) as usize].verses_count}));
                    break;
                }
                Some(timed_store::Event::Failed(value, message)) if value == generation => {
                    return Err(
                        format!("Original chapter {surah} preparation failed: {message}").into(),
                    );
                }
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    }
    let report = serde_json::json!({"passed":true,"reciter":0,"read":92,
        "clipCount":clips.len(),"clips":clips,"recordings":recordings,
        "seconds":began.elapsed().as_secs_f64(),"productionAdapters":true,"playbackPerformed":false,
        "scope":"Unmodified publisher files, authentic timing and chapter-count validation, native MP3 parsing and SHA-256; generated receipts come exclusively from AudioCache and TimedStore"});
    let mut report_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(report_path)?;
    report_file.write_all(&serde_json::to_vec_pretty(&report)?)?;
    report_file.sync_all()?;
    Ok(())
}
