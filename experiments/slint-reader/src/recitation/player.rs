//! Playback: a listening range becomes a program of items (ayah clips, or one
//! whole-surah recording), which a player thread decodes, time-stretches and
//! queues for the sound device. Speed, volume and pause change the sound in
//! place, so nothing is interrupted.
use super::{
    decode::{Decoder, Growing, Progress},
    net,
    output::{Output, Shared},
    store::{AudioCache, COUNTS, ClipKey},
    stretch::{CHANNELS, Stretch},
};
use crate::{
    data::Result,
    listening_layout, mp3,
    timed_audio::{Recording, Timeline},
};
use std::{
    fs::File,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

/// How much stretched sound waits for the device: a speed change is heard
/// within this time.
const QUEUE_SECONDS: f64 = 0.15;
/// Clips fetched ahead of the one playing.
const AHEAD_CLIPS: usize = 16;

#[derive(Clone, Copy, Debug)]
pub struct ListeningRange {
    pub reciter: usize,
    pub surah: u32,
    pub first: u32,
    pub last: u32,
    pub count: u32,
}

/// The audible position. Clips: within the current ayah's clip. Recordings:
/// `position_ms` within the selected range, which starts `offset_ms` into a
/// recording `duration_ms` long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transport {
    pub position_ms: u64,
    pub range_ms: u64,
    pub offset_ms: u64,
    pub duration_ms: u64,
    pub continuous: bool,
    pub can_seek: bool,
}

/// Something the player can decode.
#[derive(Clone)]
struct Ready {
    path: PathBuf,
    /// For a recording still downloading.
    progress: Option<Arc<Progress>>,
    duration_ms: u64,
}

struct Item {
    /// The ayah this item recites; None for the opening basmala.
    key: Option<(u32, u32)>,
    clip: Option<ClipKey>,
    ready: Option<Ready>,
    failed: Option<String>,
}

/// Where the program's frames come from: from program frame `frame`, item
/// `item` at `ms` milliseconds.
#[derive(Clone, Copy)]
struct Anchor {
    frame: f64,
    item: usize,
    ms: f64,
}

#[derive(Default)]
struct Inner {
    items: Vec<Item>,
    /// The item the player thread is decoding.
    decoding: usize,
    anchors: Vec<Anchor>,
    /// Program frame where the sound ends, once known.
    end: Option<f64>,
    failed: Option<String>,
    /// A requested move: item and milliseconds into it.
    seek: Option<(usize, u64)>,
    /// Recordings: the publisher's timing, the selected range and the length.
    timeline: Option<Timeline>,
    range_start_ms: u64,
    range_end_ms: Option<u64>,
}

struct SessionState {
    inner: Mutex<Inner>,
    changed: Condvar,
    stop: AtomicBool,
    cancelled: Arc<AtomicBool>,
    /// Playback rate as f64 bits, read by the player for every block.
    rate: AtomicU64,
    /// A temporary recording to remove when the session ends.
    temporary: Mutex<Option<PathBuf>>,
}

impl SessionState {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
    fn fail(&self, message: String) {
        let mut inner = self.lock();
        inner.failed.get_or_insert(message);
        self.changed.notify_all();
    }
}

impl Drop for SessionState {
    fn drop(&mut self) {
        if let Some(path) = self
            .temporary
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

struct Session {
    state: Arc<SessionState>,
    surah: u32,
    first: u32,
    last: u32,
    continuous: bool,
}

pub struct Audio {
    cache: Arc<AudioCache>,
    pub(super) shared: Arc<Shared>,
    pub(super) output: Option<Output>,
    session: Option<Session>,
    rate: f64,
}

impl Audio {
    pub fn with_cache(cache: Arc<AudioCache>) -> Result<Self> {
        Ok(Self {
            cache,
            shared: Shared::new(),
            output: None,
            session: None,
            rate: 1.,
        })
    }

    fn output_rate(&mut self) -> Result<u32> {
        if self.output.is_none() || self.shared.failed.load(Ordering::Acquire) {
            self.output = None;
            self.shared.failed.store(false, Ordering::Release);
            self.output = Some(Output::open(self.shared.clone())?);
        }
        Ok(self.output.as_ref().map_or(48_000, |o| o.sample_rate))
    }

    /// Ayah clips, with the opening basmala before a surah's first ayah.
    pub fn play_range(&mut self, range: ListeningRange, speed: f64, volume: f64) -> Result<()> {
        validate(range)?;
        let mut items = Vec::new();
        if range.first == 1 && range.surah != 1 && range.surah != 9 {
            items.push(Item {
                key: None,
                clip: Some(ClipKey::new(range.reciter, 1, 1)?),
                ready: None,
                failed: None,
            });
        }
        for ayah in range.first..=range.last {
            items.push(Item {
                key: Some((range.surah, ayah)),
                clip: Some(ClipKey::new(range.reciter, range.surah, ayah)?),
                ready: None,
                failed: None,
            });
        }
        let state = self.start(range, items, speed, volume, false)?;
        let cache = self.cache.clone();
        thread::Builder::new()
            .name("recitation-fetch".into())
            .spawn(move || fetch_clips(&cache, &state))?;
        Ok(())
    }

    /// The whole-surah recording with its publisher's ayah timing, from the
    /// first ayah's start to the last ayah's end.
    pub fn play_timed_scoped(
        &mut self,
        range: ListeningRange,
        speed: f64,
        volume: f64,
    ) -> Result<()> {
        validate(range)?;
        let recording = Recording::for_reciter(range.reciter).ok_or("Unknown recording")?;
        let items = vec![Item {
            key: None,
            clip: None,
            ready: None,
            failed: None,
        }];
        let state = self.start(range, items, speed, volume, true)?;
        let cache = self.cache.clone();
        thread::Builder::new()
            .name("recitation-fetch".into())
            .spawn(move || fetch_recording(&cache, &state, &recording, range))?;
        Ok(())
    }

    fn start(
        &mut self,
        range: ListeningRange,
        items: Vec<Item>,
        speed: f64,
        volume: f64,
        continuous: bool,
    ) -> Result<Arc<SessionState>> {
        checked(speed, volume)?;
        self.stop()?;
        let output_rate = self.output_rate()?;
        self.rate = speed;
        {
            let mut queue = self.shared.lock();
            queue.flush();
            queue.paused = false;
            queue.volume = volume as f32;
        }
        let state = Arc::new(SessionState {
            inner: Mutex::new(Inner {
                items,
                ..Inner::default()
            }),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
            cancelled: Arc::new(AtomicBool::new(false)),
            rate: AtomicU64::new(speed.to_bits()),
            temporary: Mutex::new(None),
        });
        let player = state.clone();
        let shared = self.shared.clone();
        thread::Builder::new()
            .name("recitation-player".into())
            .spawn(move || {
                if let Err(error) = play(&player, &shared, output_rate)
                    && !player.stopped()
                {
                    player.fail(error.to_string());
                }
            })?;
        self.session = Some(Session {
            state: state.clone(),
            surah: range.surah,
            first: range.first,
            last: range.last,
            continuous,
        });
        Ok(state)
    }

    pub fn poll(&mut self) -> Result<()> {
        // The device went away (headphones unplugged): reopen the default one,
        // keeping what was queued.
        if self.session.is_some() && self.shared.failed.load(Ordering::Acquire) {
            self.output_rate()?;
        }
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(session) = self.session.take() {
            session.state.stop.store(true, Ordering::Release);
            session.state.cancelled.store(true, Ordering::Release);
            session.state.changed.notify_all();
            self.shared.space.notify_all();
        }
        let mut queue = self.shared.lock();
        queue.flush();
        queue.paused = false;
        Ok(())
    }

    pub fn toggle(&mut self) -> Result<()> {
        let mut queue = self.shared.lock();
        queue.paused = !queue.paused;
        Ok(())
    }

    /// Takes effect within about 0.15 s, without a pause.
    pub fn request_speed(&mut self, value: f64) -> Result<()> {
        checked(value, 0.)?;
        self.rate = value;
        if let Some(session) = &self.session {
            session.state.rate.store(value.to_bits(), Ordering::Release);
        }
        Ok(())
    }

    pub fn volume(&self, value: f64) -> Result<()> {
        checked(1., value)?;
        self.shared.lock().volume = value as f32;
        Ok(())
    }

    /// The program position heard now, as (item, milliseconds into it).
    fn audible(&self) -> Option<(usize, f64)> {
        let session = self.session.as_ref()?;
        let frame = self.shared.lock().audible()?;
        let inner = session.state.lock();
        let anchor = inner.anchors.iter().rev().find(|a| a.frame <= frame)?;
        let rate = self.output.as_ref()?.sample_rate as f64;
        Some((
            anchor.item,
            anchor.ms + (frame - anchor.frame) / rate * 1000.,
        ))
    }

    pub fn completed(&self) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        let end = session.state.lock().end;
        end.is_some_and(|end| self.shared.lock().audible().is_some_and(|f| f >= end))
    }

    pub fn has_session(&self) -> bool {
        self.session.is_some()
    }

    pub fn is_paused(&self) -> bool {
        self.shared.lock().paused
    }

    /// Healthy and not finished: the bridge keeps polling.
    pub fn needs_fast_poll(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.state.lock().failed.is_none())
            && !self.completed()
    }

    pub fn opening_surah(&self) -> Option<u32> {
        let session = self.session.as_ref()?;
        if session.continuous || self.completed() {
            return None;
        }
        let (item, _) = self.audible()?;
        let inner = session.state.lock();
        (item == 0 && inner.items.first()?.key.is_none()).then_some(session.surah)
    }

    pub fn current(&self) -> Option<(u32, u32)> {
        let session = self.session.as_ref()?;
        let (item, ms) = self.audible()?;
        let inner = session.state.lock();
        if session.continuous {
            let interval = inner.timeline.as_ref()?.at(ms.max(0.) as u64)?;
            // The selected range's ayahs only: the publisher's next ayah can
            // start within the range's last few milliseconds.
            return (interval.ayah >= session.first)
                .then_some((session.surah, interval.ayah.min(session.last)));
        }
        inner.items.get(item)?.key
    }

    pub fn transport(&self) -> Option<Transport> {
        let session = self.session.as_ref()?;
        let (item, ms) = self.audible()?;
        let completed = self.completed();
        let inner = session.state.lock();
        let duration_ms = inner.items.get(item)?.ready.as_ref()?.duration_ms;
        let ms = ms.max(0.) as u64;
        if session.continuous {
            let offset = inner.range_start_ms;
            let end = inner.range_end_ms.unwrap_or(duration_ms).max(offset + 1);
            Some(Transport {
                position_ms: ms.saturating_sub(offset).min(end - offset),
                range_ms: end - offset,
                offset_ms: offset,
                duration_ms: duration_ms.max(end),
                continuous: true,
                can_seek: !completed,
            })
        } else {
            Some(Transport {
                position_ms: ms.min(duration_ms),
                range_ms: duration_ms,
                offset_ms: 0,
                duration_ms,
                continuous: false,
                can_seek: !completed && duration_ms > 0,
            })
        }
    }

    /// Moves within the current clip or the selected range of the recording.
    pub fn seek(&mut self, delta_ms: Option<i64>, slider: u32) -> Result<bool> {
        let Some(t) = self.transport().filter(|t| t.can_seek) else {
            return Ok(false);
        };
        let Some((item, _)) = self.audible() else {
            return Ok(false);
        };
        let relative = delta_ms.map_or_else(
            || listening_layout::slider_target(slider, t.duration_ms, t.offset_ms, t.range_ms),
            |delta| listening_layout::seek_target(t.position_ms, t.range_ms, delta),
        );
        let session = self.session.as_ref().ok_or("Nothing is playing")?;
        let mut inner = session.state.lock();
        inner.seek = Some((item, relative + t.offset_ms));
        session.state.changed.notify_all();
        self.shared.space.notify_all();
        Ok(true)
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn validate(range: ListeningRange) -> Result<()> {
    let count = range
        .surah
        .checked_sub(1)
        .and_then(|i| COUNTS.get(i as usize))
        .copied();
    if count != Some(range.count)
        || range.first == 0
        || range.first > range.last
        || range.last > range.count
        || range.reciter >= crate::audio::RECITERS.len()
    {
        return Err("Invalid recitation range".into());
    }
    Ok(())
}

fn checked(rate: f64, volume: f64) -> Result<()> {
    if !rate.is_finite() || !(0.5..=2.).contains(&rate) {
        return Err("Playback speed must be between 0.5 and 2".into());
    }
    if !volume.is_finite() || !(0. ..=1.).contains(&volume) {
        return Err("Playback volume must be between 0 and 1".into());
    }
    Ok(())
}

/// Resolves clips in order, staying a few ahead of the one being decoded.
fn fetch_clips(cache: &AudioCache, state: &SessionState) {
    let count = state.lock().items.len();
    for index in 0..count {
        let key = {
            let mut inner = state.lock();
            while index > inner.decoding + AHEAD_CLIPS && !state.stopped() {
                inner = state
                    .changed
                    .wait_timeout(inner, Duration::from_millis(250))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            if state.stopped() {
                return;
            }
            inner.items[index].clip
        };
        let Some(key) = key else { continue };
        let result = cache.clip(key, &state.cancelled);
        let mut inner = state.lock();
        match result {
            Ok(clip) => {
                inner.items[index].ready = Some(Ready {
                    path: clip.path,
                    progress: None,
                    duration_ms: clip.duration_ms,
                });
            }
            Err(error) => inner.items[index].failed = Some(error.to_string()),
        }
        state.changed.notify_all();
        if inner.items[index].failed.is_some() {
            return;
        }
    }
}

/// A saved recording, or else the publisher's timing and a download that
/// playback follows as it arrives.
fn fetch_recording(
    cache: &AudioCache,
    state: &SessionState,
    recording: &Recording,
    range: ListeningRange,
) {
    let result = (|| -> Result<()> {
        if let Some((path, timeline, duration_ms)) =
            cache.saved_recording(range.reciter, range.surah, &state.cancelled)
        {
            set_recording(state, &timeline, range, path, None, duration_ms)?;
            let mut inner = state.lock();
            inner.timeline = Some(timeline);
            state.changed.notify_all();
            return Ok(());
        }
        let (_, _, timeline) = cache.fetch_timings(recording, range.surah, &state.cancelled)?;
        let path = cache.streaming_file(recording.read, range.surah);
        let mut file = File::create_new(&path)?;
        *state.temporary.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.clone());
        let progress = Arc::new(Progress::new(state.cancelled.clone()));
        // Until the whole file is here, the length is where the timing ends.
        let estimate = timeline.at(u64::MAX).map_or(0, |last| last.end_ms);
        set_recording(
            state,
            &timeline,
            range,
            path.clone(),
            Some(progress.clone()),
            estimate,
        )?;
        state.lock().timeline = Some(timeline);
        state.changed.notify_all();
        let url = recording.audio_url(range.surah)?;
        let fetched = net::fetch(
            &url,
            &mut file,
            mp3::MAX_CHAPTER_BYTES,
            true,
            &state.cancelled,
            |bytes, total| {
                progress.set_total(total);
                progress.wrote(bytes);
            },
        );
        let fetched = fetched.and_then(|bytes| {
            file.sync_all()?;
            Ok(bytes)
        });
        progress.finish(fetched.is_ok());
        let bytes = fetched?;
        // Now the real length is known.
        if let Ok(info) =
            mp3::inspect_reader(&mut File::open(&path)?, bytes, mp3::MAX_CHAPTER_BYTES)
        {
            let mut inner = state.lock();
            if let Some(ready) = inner.items[0].ready.as_mut() {
                ready.duration_ms = (info.duration_seconds * 1000.).round() as u64;
            }
        }
        Ok(())
    })();
    if let Err(error) = result
        && !state.stopped()
    {
        let mut inner = state.lock();
        inner.items[0].failed = Some(error.to_string());
        state.changed.notify_all();
    }
}

fn set_recording(
    state: &SessionState,
    timeline: &Timeline,
    range: ListeningRange,
    path: PathBuf,
    progress: Option<Arc<Progress>>,
    duration_ms: u64,
) -> Result<()> {
    let (start, end) = timeline.range(range.first, range.last)?;
    let mut inner = state.lock();
    inner.range_start_ms = start;
    inner.range_end_ms = end;
    inner.items[0].ready = Some(Ready {
        path,
        progress,
        duration_ms,
    });
    Ok(())
}

/// The player thread: decodes each item in turn, stretches it to the current
/// rate and keeps the device queue about `QUEUE_SECONDS` full. Seeks restart
/// it at the requested item and time.
fn play(state: &SessionState, shared: &Shared, output_rate: u32) -> Result<()> {
    let target = (output_rate as f64 * QUEUE_SECONDS) as u64;
    let mut stretch = Stretch::new(output_rate, 0);
    let mut out = Vec::new();
    let mut pcm = Vec::new();
    let mut item = 0;
    let mut start_ms = None;
    let mut drained = false;
    'items: loop {
        if state.stopped() {
            return Ok(());
        }
        let seek = state.lock().seek.take();
        if let Some((to_item, ms)) = seek {
            shared.lock().flush();
            let next = stretch.input_end() + output_rate as i64;
            stretch = Stretch::new(output_rate, next);
            let mut inner = state.lock();
            inner.end = None;
            inner.anchors.clear();
            drop(inner);
            item = to_item;
            start_ms = Some(ms);
            drained = false;
        }
        let count = state.lock().items.len();
        if item >= count {
            // The end: let the stretcher's last windows out, then mark where
            // the sound stops so the device can report completion there.
            if !drained {
                let end = stretch.input_end() as f64;
                stretch.push(&vec![0.; stretch.tail() * CHANNELS]);
                state.lock().end = Some(end);
                drained = true;
            }
            match top_up(state, shared, &mut stretch, &mut out, target) {
                Fill::Full => wait_for_space(shared, state),
                Fill::Produced => {}
                // All out: stay until stopped or sent back by a seek.
                Fill::NeedsInput => idle(state),
            }
            continue;
        }
        // Wait for the item, unless stopped or sent elsewhere.
        let ready = {
            let mut inner = state.lock();
            inner.decoding = item;
            state.changed.notify_all();
            loop {
                if state.stopped() {
                    return Ok(());
                }
                if inner.seek.is_some() {
                    continue 'items;
                }
                if let Some(message) = &inner.items[item].failed {
                    return Err(message.clone().into());
                }
                if let Some(ready) = inner.items[item].ready.clone() {
                    break ready;
                }
                inner = state
                    .changed
                    .wait_timeout(inner, Duration::from_millis(100))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        };
        let source: Box<dyn symphonia::core::io::MediaSource> = match &ready.progress {
            Some(progress) if !progress.finished() => {
                Box::new(Growing::new(&ready.path, progress.clone())?)
            }
            _ => Box::new(File::open(&ready.path)?),
        };
        let mut decoder = Decoder::open(source, output_rate)?;
        let (range_start, range_end) = {
            let inner = state.lock();
            (inner.range_start_ms, inner.range_end_ms)
        };
        let begin = start_ms.take().unwrap_or(range_start);
        let mut at_ms = if begin > 0 { decoder.seek(begin)? } else { 0 } as f64;
        anchor(state, stretch.input_end() as f64, item, at_ms);
        loop {
            if state.stopped() {
                return Ok(());
            }
            if state.lock().seek.is_some() {
                continue 'items;
            }
            match top_up(state, shared, &mut stretch, &mut out, target) {
                Fill::Full => {
                    wait_for_space(shared, state);
                    continue;
                }
                Fill::Produced => continue,
                Fill::NeedsInput => {}
            }
            pcm.clear();
            if !decoder.next(&mut pcm)? {
                break;
            }
            let frames = pcm.len() / CHANNELS;
            let next_ms = at_ms + frames as f64 * 1000. / output_rate as f64;
            if let Some(end) = range_end
                && next_ms >= end as f64
            {
                // The range ends inside this stretch of audio.
                let keep = ((end as f64 - at_ms).max(0.) / 1000. * output_rate as f64) as usize;
                stretch.push(&pcm[..(keep * CHANNELS).min(pcm.len())]);
                break;
            }
            stretch.push(&pcm);
            at_ms = next_ms;
        }
        item += 1;
    }
}

enum Fill {
    Full,
    Produced,
    NeedsInput,
}

/// Moves stretched blocks into the device queue until it holds `target` frames.
fn top_up(
    state: &SessionState,
    shared: &Shared,
    stretch: &mut Stretch,
    out: &mut Vec<f32>,
    target: u64,
) -> Fill {
    let rate = f64::from_bits(state.rate.load(Ordering::Acquire));
    let mut produced = false;
    loop {
        if shared.lock().pending() >= target {
            return Fill::Full;
        }
        out.clear();
        let Some(block) = stretch.step(rate, out) else {
            return if produced {
                Fill::Produced
            } else {
                Fill::NeedsInput
            };
        };
        shared.lock().push(out, block.source, block.rate);
        produced = true;
    }
}

fn idle(state: &SessionState) {
    let inner = state.lock();
    if state.stopped() || inner.seek.is_some() {
        return;
    }
    let _ = state
        .changed
        .wait_timeout(inner, Duration::from_millis(100))
        .unwrap_or_else(|e| e.into_inner());
}

fn anchor(state: &SessionState, frame: f64, item: usize, ms: f64) {
    let mut inner = state.lock();
    inner.anchors.push(Anchor { frame, item, ms });
    let excess = inner.anchors.len().saturating_sub(16);
    inner.anchors.drain(..excess);
}

fn wait_for_space(shared: &Shared, state: &SessionState) {
    let queue = shared.lock();
    if state.stopped() || state.lock().seek.is_some() {
        return;
    }
    let _ = shared
        .space
        .wait_timeout(queue, Duration::from_millis(20))
        .unwrap_or_else(|e| e.into_inner());
}
