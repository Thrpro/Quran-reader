//! Muted, offline decoder verification using the original 112 recording bundle.
//! This entry point owns a fresh profile; it never reads the normal reader's
//! profile, synthesizes recordings, or enters a missing-provider fallback.
use crate::{audio_bridge::AudioBridge, audio_cache, data::Result};
use serde_json::{Value, json};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

slint::slint! {
    export component AudioProbe inherits Window {
        title: "Offline recitation verification";
        preferred-width: 460px;
        preferred-height: 170px;
        default-font-family: "Segoe UI";
        in-out property <string> status: "Opening the verified local recording…";
        background: #202620;
        Text {
            x: 24px; y: 24px; width: parent.width - 48px; height: parent.height - 48px;
            text: root.status; color: #e8efe8; font-size: 17px;
            wrap: word-wrap; vertical-alignment: center;
        }
    }
}

const FIXTURES: [(&str, usize, &str); 4] = [
    (
        "audio.mp3",
        189_334,
        "34888ed503996c7a6d97a95f3c35e793aaf3e856f482682e66bd98c7b661eddd",
    ),
    (
        "timings.json",
        866,
        "5554fa3590018ec72e22a8a2b0c1bca0ea7c0a5a7cf76f3a6691ea13582164d0",
    ),
    (
        "catalogue.json",
        38_306,
        "b22204b9af280a4bcfbbf9bfd44129a6744d6b3d97b14fed3306207df8ac7e78",
    ),
    (
        "receipt.json",
        316,
        "e2660ab041f4e7ae02a4c003b25d346930291239bfa230599bc3af2c29b612e9",
    ),
];

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

pub(crate) fn seed(out: &Path, fixture: &Path) -> Result<Vec<Value>> {
    if out.exists() {
        return Err("Audio verification requires a new output directory".into());
    }
    let mut verified = Vec::new();
    for (name, size, digest) in FIXTURES {
        let path = fixture.join(name);
        if path.symlink_metadata()?.file_type().is_symlink()
            || !path.is_file()
            || path.metadata()?.len() != size as u64
        {
            return Err(
                format!("Original audio fixture has an unexpected size/type: {name}").into(),
            );
        }
        let bytes = fs::read(&path)?;
        if audio_cache::sha256(&bytes)? != digest {
            return Err(format!("Original audio fixture hash differs: {name}").into());
        }
        verified.push((name, digest, bytes));
    }
    fs::create_dir(out)?;
    let profile = out.join("profile");
    let store = profile.join("AudioStore");
    let continuous = store.join("Continuous");
    let target = continuous.join("r92-s112");
    for path in [&profile, &store, &continuous, &target] {
        fs::create_dir(path)?;
    }
    let mut proofs = Vec::new();
    for (name, digest, bytes) in verified {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(target.join(name))?;
        file.write_all(&bytes)?;
        file.flush()?;
        if audio_cache::sha256(&fs::read(target.join(name))?)? != digest {
            return Err(format!("Copied audio fixture hash differs: {name}").into());
        }
        proofs.push(json!({"name":name,"bytes":bytes.len(),"sha256":digest}));
    }
    Ok(proofs)
}

struct Probe {
    audio: AudioBridge,
    began: Instant,
    phase: u32,
    since: Instant,
    start_position: u64,
    duration: u64,
    observations: Vec<Value>,
    checks: Vec<Value>,
    done: bool,
}
impl Probe {
    fn mark(&mut self, name: &str, details: Value) {
        self.checks
            .push(json!({"name":name,"details":details,"passed":true}));
    }

    fn step(&mut self, ui: &AudioProbe) -> Result<bool> {
        if self.began.elapsed() > Duration::from_secs(30) {
            return Err(format!(
                "Offline audio verification exceeded 30 seconds at phase {}",
                self.phase
            )
            .into());
        }
        let state = self.audio.poll(false)?;
        if state.error {
            return Err(format!(
                "Offline decoder failed during phase {}: {}",
                self.phase, state.status
            )
            .into());
        }
        if self.observations.len() < 650 {
            self.observations.push(json!({
                "seconds":self.began.elapsed().as_secs_f64(),"phase":self.phase,
                "positionMs":state.position_ms,"durationMs":state.duration_ms,
                "rangeStartMs":state.range_start_ms,"rangeEndMs":state.range_end_ms,
                "current":state.current,"clock":state.clock,
                "paused":state.paused,"loading":state.loading,"canSeek":state.can_seek,
            }));
        }
        ui.set_status(
            format!(
                "Offline recitation check {}\n{}\n{}",
                self.phase, state.status, state.clock
            )
            .into(),
        );
        match self.phase {
            0 => {
                self.audio.play(0, (112, 3), None, 1., 0.)?;
                self.audio.toggle()?;
                self.phase = 1;
            }
            1 if state.active && !state.loading && state.can_seek => {
                if !state.continuous
                    || !state.paused
                    || state.range_start_ms != 4515
                    || state.current != Some((112, 3))
                    || !(11_300..=11_600).contains(&state.duration_ms)
                    || state.clock.starts_with("Ayah")
                {
                    return Err("Original chapter clock or pre-attachment pause did not match the 112 fixture".into());
                }
                self.duration = state.duration_ms;
                self.start_position = state.position_ms;
                self.since = Instant::now();
                self.mark("validated local recording with whole-chapter clock", json!({"durationMs":state.duration_ms,"rangeStartMs":state.range_start_ms,"current":state.current}));
                self.phase = 2;
            }
            2 if self.since.elapsed() >= Duration::from_millis(350) => {
                if !state.paused || state.position_ms.abs_diff(self.start_position) > 160 {
                    return Err("The decoder advanced while paused before its first sample".into());
                }
                self.audio.set_speed(1.25)?;
                self.since = Instant::now();
                self.phase = 3;
            }
            3 if self.since.elapsed() >= Duration::from_millis(250) => {
                if !state.paused
                    || state.position_ms.abs_diff(self.start_position) > 160
                    || state.duration_ms != self.duration
                {
                    return Err(
                        "Changing speed while paused reset the recording or its chapter clock"
                            .into(),
                    );
                }
                self.mark(
                    "pause and live speed edit preserve position",
                    json!({"requestedRate":1.25,"positionMs":state.position_ms}),
                );
                if !self.audio.seek_slider(5000)? {
                    return Err(
                        "The original decoder did not accept the chapter slider seek".into(),
                    );
                }
                self.phase = 4;
            }
            4 if !state.loading
                && state.paused
                && state.position_ms.abs_diff(self.duration / 2) <= 300 =>
            {
                self.mark(
                    "chapter slider reaches recording midpoint",
                    json!({"positionMs":state.position_ms,"expectedMs":self.duration/2}),
                );
                if !self.audio.seek_delta(-10_000)? {
                    return Err("The original decoder did not accept backward seeking".into());
                }
                self.phase = 5;
            }
            5 if !state.loading
                && state.can_seek
                && state.range_start_ms == 0
                && state.position_ms <= 300 =>
            {
                if !state.paused || state.duration_ms != self.duration {
                    return Err(
                        "Cross-range seek lost pause intent or the full chapter duration".into(),
                    );
                }
                self.mark("back ten seconds crosses original ayah-three start", json!({"originalRangeStartMs":4515,"newRangeStartMs":state.range_start_ms,"positionMs":state.position_ms,"paused":state.paused}));
                self.audio.toggle()?;
                self.since = Instant::now();
                self.phase = 6;
            }
            6 if !state.paused
                && state.position_ms >= 350
                && self.since.elapsed() >= Duration::from_millis(400) =>
            {
                self.start_position = state.position_ms;
                self.since = Instant::now();
                self.phase = 7;
            }
            7 if self.since.elapsed() >= Duration::from_secs(2) => {
                let wall = self.since.elapsed().as_secs_f64();
                let progress = state.position_ms.saturating_sub(self.start_position) as f64 / 1000.;
                let ratio = progress / wall;
                if state.paused || !(1.10..=1.40).contains(&ratio) {
                    return Err(format!("Observed decoder progress did not reflect requested 1.25 speed: {progress:.3}s/{wall:.3}s").into());
                }
                self.mark("real decoder progress at live requested speed", json!({"requestedRate":1.25,"contentSeconds":progress,"wallSeconds":wall,"ratio":ratio}));
                self.audio.toggle()?;
                self.start_position = state.position_ms;
                self.since = Instant::now();
                self.phase = 8;
            }
            8 if self.since.elapsed() >= Duration::from_millis(350) => {
                if !state.paused || state.position_ms.abs_diff(self.start_position) > 200 {
                    return Err("Pause did not retain the observed decoder position".into());
                }
                self.mark(
                    "pause after playback retains position",
                    json!({"positionMs":state.position_ms}),
                );
                self.audio.stop()?;
                let ended = self.audio.snapshot(false);
                if ended.active || self.audio.needs_poll() || !ended.clock.is_empty() {
                    return Err("End did not clear the listening session and stop polling".into());
                }
                self.mark(
                    "End releases session and polling",
                    json!({"active":ended.active,"needsPoll":self.audio.needs_poll()}),
                );
                self.done = true;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}

pub fn run(out: &Path, fixture: &Path) -> Result<()> {
    let fixtures = seed(out, fixture)?;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("skia-software".into())
        .select()?;
    let ui = AudioProbe::new()?;
    let state = Rc::new(RefCell::new(Probe {
        audio: AudioBridge::new(out.join("profile")),
        began: Instant::now(),
        phase: 0,
        since: Instant::now(),
        start_position: 0,
        duration: 0,
        observations: Vec::new(),
        checks: Vec::new(),
        done: false,
    }));
    let error = Rc::new(RefCell::new(None::<String>));
    {
        let error = error.clone();
        ui.window().on_close_requested(move || {
            *error.borrow_mut() =
                Some("Offline audio verification was closed before completion".into());
            let _ = slint::quit_event_loop();
            slint::CloseRequestResponse::HideWindow
        });
    }
    let timer = Rc::new(slint::Timer::default());
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let error = error.clone();
        let weak_timer = Rc::downgrade(&timer);
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(50),
            move || {
                let result = weak
                    .upgrade()
                    .ok_or_else(|| "Audio verification window was dropped".into())
                    .and_then(|ui| state.borrow_mut().step(&ui));
                let done = match result {
                    Ok(done) => done,
                    Err(failure) => {
                        *error.borrow_mut() = Some(failure.to_string());
                        true
                    }
                };
                if done {
                    if let Some(timer) = weak_timer.upgrade() {
                        timer.stop();
                    }
                    let _ = slint::quit_event_loop();
                }
            },
        );
    }
    ui.show()?;
    let event_result = slint::run_event_loop();
    timer.stop();
    let _ = ui.hide();
    let _ = state.borrow_mut().audio.stop();
    if let Err(failure) = event_result {
        *error.borrow_mut() = Some(failure.to_string());
    }
    let state = state.borrow();
    if !state.done && error.borrow().is_none() {
        *error.borrow_mut() = Some("Offline audio verification did not complete".into());
    }
    for (name, _, digest) in FIXTURES {
        let original = fixture.join(name);
        let copy = out
            .join("profile/AudioStore/Continuous/r92-s112")
            .join(name);
        if audio_cache::sha256(&fs::read(original)?)? != digest
            || audio_cache::sha256(&fs::read(copy)?)? != digest
        {
            *error.borrow_mut() =
                Some("An original recording fixture or copied bundle changed".into());
        }
    }
    let failure = error.borrow().clone();
    write_json(
        &out.join("audio-verification.json"),
        &json!({
            "passed":failure.is_none(),"error":failure,"fixtures":fixtures,
            "checks":state.checks,"observations":state.observations,
            "elapsedSeconds":state.began.elapsed().as_secs_f64(),"muted":true,
            "profileIsolated":true,"networkRequested":false,"sourceBytesUnchanged":true,
            "scope":"Actual native Media Foundation session progress and chapter clocks. No hardware audio loopback capture, no synthetic recording, no network test."
        }),
    )?;
    if let Some(failure) = failure {
        return Err(failure.into());
    }
    Ok(())
}
