//! Real muted native decoder checks for asynchronous live rate edits.
use super::*;
use std::{
    fs, thread,
    time::{Duration, Instant},
};
impl Audio {
    pub(super) fn qa_request_and_wait_rate(&mut self, rate: f64) -> Result<()> {
        self.request_speed(rate)?;
        let start = Instant::now();
        let session = self.player.PlaybackSession()?;
        while (session.PlaybackRate()? - rate).abs() > 0.001 {
            pump_messages();
            self.poll()?;
            if start.elapsed() > Duration::from_secs(3) {
                return Err("Live rate was not applied before QA deadline".into());
            }
            thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
    pub fn live_rate_smoke(folder: &std::path::Path) -> Result<()> {
        fs::create_dir_all(folder)?;
        let cache = AudioCache::new(folder.join("profile"))?;
        let mut audio = Self::with_cache(cache)?;
        let mut cases = vec![];
        let result = (|| -> Result<()> {
            let modes = if std::env::args().any(|arg| arg == "--qa-rate-clips") {
                &[false][..]
            } else {
                &[true][..]
            };
            for &continuous in modes {
                let range = ListeningRange {
                    reciter: 0,
                    surah: 112,
                    first: 2,
                    last: 4,
                    count: 4,
                };
                let begin = Instant::now();
                if continuous {
                    audio.play_timed(range, 1., 0.)?;
                } else {
                    audio.play_range(range, 1., 0.)?;
                }
                loop {
                    if !pump_messages() {
                        return Err("Rate QA interrupted".into());
                    }
                    audio.poll()?;
                    if audio.failed.load(Ordering::Acquire) {
                        return Err(audio.status(false).into());
                    }
                    let session = audio.player.PlaybackSession()?;
                    if session.PlaybackState()? == MediaPlaybackState::Playing
                        && session.Position()?.Duration > 1_000_000
                    {
                        break;
                    }
                    if begin.elapsed() > Duration::from_secs(30) {
                        return Err("Rate QA did not reach native Playing".into());
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                let start_ms = begin.elapsed().as_secs_f64() * 1000.;
                let list = audio
                    .list
                    .as_ref()
                    .ok_or("Rate QA missing playlist")?
                    .clone();
                let source = list.CurrentItem()?;
                let trace = PlaylistTrace::new(&list)?;
                let generation = audio.generation.load(Ordering::Acquire);
                let session = audio.player.PlaybackSession()?;
                let mut edits = vec![];
                for rates in [&[1.25][..], &[0.75, 1.5, 2., 1.][..]] {
                    let before = session.Position()?.Duration;
                    let mut dispatch = vec![];
                    for &rate in rates {
                        let start = Instant::now();
                        if std::env::args().any(|arg| arg == "--qa-synchronous-rate") {
                            audio.speed(rate)?;
                        } else {
                            audio.request_speed(rate)?;
                        }
                        dispatch.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                    let target = *rates.last().unwrap();
                    let wait = Instant::now();
                    while (session.PlaybackRate()? - target).abs() > 0.001 {
                        pump_messages();
                        audio.poll()?;
                        if wait.elapsed() > Duration::from_secs(3) {
                            return Err("Latest rate edit was lost".into());
                        }
                        thread::sleep(Duration::from_millis(2));
                    }
                    // Continue decoding, so this checks sustained playback and
                    // a real current item rather than only a stored preference.
                    let dwell = Instant::now();
                    while dwell.elapsed() < Duration::from_millis(160) {
                        pump_messages();
                        audio.poll()?;
                        thread::sleep(Duration::from_millis(5));
                    }
                    let after = session.Position()?.Duration;
                    if list.CurrentItem()? != source
                        || audio.generation.load(Ordering::Acquire) != generation
                        || after < before
                        || after - before > 30_000_000
                        || session.PlaybackState()? != MediaPlaybackState::Playing
                    {
                        return Err(format!("Live rate continuity check: sameItem={}, generation={}/{}, position={before}->{after}, state={}, rate={}, waitMs={}, events={:?}, status={}", list.CurrentItem()? == source, audio.generation.load(Ordering::Acquire), generation, session.PlaybackState()?.0, session.PlaybackRate()?, wait.elapsed().as_secs_f64()*1000., trace.snapshot()?, audio.status(false)).into());
                    }
                    edits.push(serde_json::json!({"requested":rates,"nativeRate":session.PlaybackRate()?,"dispatchMs":dispatch,"settleMs":wait.elapsed().as_secs_f64()*1000.,"beforeTicks":before,"afterTicks":after}));
                }
                let status = audio.status(false);
                let before_progress = session.Position()?.Duration;
                let progress_deadline = Instant::now();
                while session.Position()?.Duration < before_progress + 5_000_000 {
                    pump_messages();
                    audio.poll()?;
                    if progress_deadline.elapsed() > Duration::from_secs(5) || audio.completed() {
                        return Err("Native decoder did not advance after rate edits".into());
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                if !status.contains("Time remaining") {
                    return Err("Playback status lacks remaining time".into());
                }
                let timed_source = audio.qa_timed_source();
                if continuous {
                    let timed = audio.timed.as_ref().ok_or("Missing timed source clock")?;
                    if timed.offset_ms == 0 || !status.contains("Surah") {
                        return Err("Mid-surah playback did not retain its chapter clock".into());
                    }
                }
                audio.stop()?;
                if audio.live_rate.is_some() || audio.has_session() {
                    return Err("Stop retained the live rate worker/session".into());
                }
                cases.push(serde_json::json!({"continuous":continuous,"startToNativePlayingMs":start_ms,"edits":edits,"sameSourceAndGeneration":true,"positionContinuous":true,"status":status,"timedSource":timed_source,"stopReleasesWorker":true}));
            }
            Ok(())
        })();
        let _ = audio.stop();
        fs::write(
            folder.join("native-live-rate.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"passed":result.is_ok(),"error":result.as_ref().err().map(|e|e.to_string()),"cases":cases,"muted":true,"scope":"Actual native decoder and live rate worker with original saved files; dispatch time is not input-to-photon, audible onset or display FPS."}),
            )?,
        )?;
        result
    }
}
