//! MP3 to stereo samples at the output device's rate.
use crate::data::Result;
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use symphonia::core::{
    codecs::{CodecParameters, audio::AudioDecoder, audio::AudioDecoderOptions},
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType, probe::Hint},
    io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    units::Time,
};

/// Decodes one MP3 file into interleaved stereo at `output_rate`.
pub struct Decoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    channels: usize,
    resample: Resampler,
    scratch: Vec<f32>,
    stereo: Vec<f32>,
}

impl Decoder {
    pub fn open(source: Box<dyn MediaSource>, output_rate: u32) -> Result<Self> {
        let stream = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        hint.with_extension("mp3");
        let reader = symphonia::default::get_probe().probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )?;
        let track = reader
            .default_track(TrackType::Audio)
            .ok_or("The recording has no audio")?;
        let Some(CodecParameters::Audio(params)) = &track.codec_params else {
            return Err("The recording has no audio".into());
        };
        let rate = params
            .sample_rate
            .ok_or("The recording has no sample rate")?;
        let channels = params
            .channels
            .as_ref()
            .map_or(2, |c| c.count())
            .clamp(1, 8);
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())?;
        Ok(Self {
            track: track.id,
            reader,
            decoder,
            channels,
            resample: Resampler::new(rate, output_rate),
            scratch: Vec::new(),
            stereo: Vec::new(),
        })
    }

    /// The next stretch of audio, or None at the end of the recording.
    pub fn next(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(false),
                // A recording that ends inside a frame ends there.
                Err(symphonia::core::errors::Error::IoError(e))
                    if e.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(false);
                }
                Err(e) => return Err(e.into()),
            };
            if packet.track_id != self.track {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                // One damaged frame is skipped, as players do.
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                Err(e) => return Err(e.into()),
            };
            self.channels = decoded.spec().channels().count().clamp(1, 8);
            self.scratch.clear();
            decoded.copy_to_vec_interleaved(&mut self.scratch);
            self.stereo.clear();
            to_stereo(&self.scratch, self.channels, &mut self.stereo);
            self.resample.process(&self.stereo, out);
            return Ok(true);
        }
    }

    /// Moves to `ms` into the recording; returns where decoding resumes.
    pub fn seek(&mut self, ms: u64) -> Result<u64> {
        let time = Time::try_from_secs_f64(ms as f64 / 1000.).ok_or("Invalid position")?;
        let seeked = self.reader.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(self.track),
            },
        )?;
        self.decoder.reset();
        self.resample.reset();
        let track = self
            .reader
            .tracks()
            .iter()
            .find(|t| t.id == self.track)
            .ok_or("The recording has no audio")?;
        let actual = track
            .time_base
            .and_then(|base| base.calc_time(seeked.actual_ts))
            .map_or(ms, |t| (t.as_secs_f64() * 1000.).round().max(0.) as u64);
        Ok(actual)
    }
}

fn to_stereo(samples: &[f32], channels: usize, out: &mut Vec<f32>) {
    match channels {
        1 => out.extend(samples.iter().flat_map(|&v| [v, v])),
        2 => out.extend_from_slice(samples),
        n => out.extend(samples.chunks_exact(n).flat_map(|f| [f[0], f[1]])),
    }
}

/// Converts stereo between sample rates with cubic (Catmull-Rom)
/// interpolation, keeping its place across calls.
pub struct Resampler {
    step: f64,
    /// Position of the next output frame, relative to `history[1]`.
    phase: f64,
    /// The last three input frames before the current call's input.
    history: Vec<f32>,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self {
            step: from as f64 / to as f64,
            phase: 0.,
            history: Vec::new(),
        }
    }
    pub fn reset(&mut self) {
        self.phase = 0.;
        self.history.clear();
    }
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if (self.step - 1.).abs() < 1e-12 {
            out.extend_from_slice(input);
            return;
        }
        // Frames: three of history, then the new input. The first call
        // repeats its first frame as history.
        if self.history.is_empty() {
            let first = [
                input.first().copied().unwrap_or(0.),
                input.get(1).copied().unwrap_or(0.),
            ];
            for _ in 0..3 {
                self.history.extend_from_slice(&first);
            }
            self.phase = 1.;
        }
        let mut frames = std::mem::take(&mut self.history);
        frames.extend_from_slice(input);
        let count = frames.len() / 2;
        let at = |i: usize, c: usize| frames[i * 2 + c];
        // Output at position x (frames) needs frames floor(x)-1 .. floor(x)+2.
        while self.phase + 2. < count as f64 {
            let i = self.phase.floor() as usize;
            let t = (self.phase - i as f64) as f32;
            for c in 0..2 {
                let (p0, p1, p2, p3) = (
                    at(i.saturating_sub(1), c),
                    at(i, c),
                    at(i + 1, c),
                    at(i + 2, c),
                );
                let v = p1
                    + 0.5
                        * t
                        * (p2 - p0
                            + t * (2. * p0 - 5. * p1 + 4. * p2 - p3
                                + t * (3. * (p1 - p2) + p3 - p0)));
                out.push(v);
            }
            self.phase += self.step;
        }
        // Keep the last three frames; positions move back by what was dropped.
        let keep = 3.min(count);
        let dropped = count - keep;
        self.phase -= dropped as f64;
        self.history = frames[dropped * 2..].to_vec();
    }
}

/// A file that another thread is still downloading. Reads wait for the bytes
/// they need, so playback can start long before the download ends.
pub struct Growing {
    file: File,
    progress: Arc<Progress>,
}

/// How far a download has come, shared with its readers.
pub struct Progress {
    state: Mutex<ProgressState>,
    changed: Condvar,
    /// Set when playback stops; waiting readers give up.
    stopped: Arc<AtomicBool>,
}
#[derive(Default, Clone, Copy)]
struct ProgressState {
    written: u64,
    total: Option<u64>,
    finished: bool,
    failed: bool,
}

impl Progress {
    pub fn new(stopped: Arc<AtomicBool>) -> Self {
        Self {
            state: Mutex::default(),
            changed: Condvar::new(),
            stopped,
        }
    }
    pub fn set_total(&self, total: Option<u64>) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).total = total;
        self.changed.notify_all();
    }
    pub fn wrote(&self, bytes: u64) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).written = bytes;
        self.changed.notify_all();
    }
    pub fn finish(&self, ok: bool) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.finished = true;
        state.failed = !ok;
        self.changed.notify_all();
    }
    pub fn bytes(&self) -> (u64, Option<u64>) {
        let state = *self.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.written, state.total)
    }
    pub fn finished(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finished
    }
    /// Waits until byte `until` is written or the download ends; whether it is there.
    fn wait_for(&self, until: u64) -> io::Result<u64> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if self.stopped.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Playback stopped",
                ));
            }
            if state.written >= until || state.finished {
                if state.failed && state.written < until {
                    return Err(io::Error::other("The recording download failed"));
                }
                return Ok(state.written);
            }
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(250))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

impl Growing {
    pub fn new(path: &Path, progress: Arc<Progress>) -> io::Result<Self> {
        Ok(Self {
            file: File::open(path)?,
            progress,
        })
    }
}

impl Read for Growing {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let position = self.file.stream_position()?;
        let available = self.progress.wait_for(position + 1)?;
        let allowed = (available.saturating_sub(position) as usize).min(buf.len());
        if allowed == 0 {
            return Ok(0);
        }
        self.file.read(&mut buf[..allowed])
    }
}

impl Seek for Growing {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        // Seeking from the end needs the full length, which the server gave.
        if let SeekFrom::End(offset) = to {
            let total = self.progress.bytes().1.ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "Unknown recording length")
            })?;
            return self
                .file
                .seek(SeekFrom::Start(total.saturating_add_signed(offset)));
        }
        self.file.seek(to)
    }
}

impl MediaSource for Growing {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        self.progress.bytes().1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_length_and_tone() {
        let input: Vec<f32> = (0..44_100)
            .flat_map(|i| {
                let v = (i as f32 / 44_100. * 2. * std::f32::consts::PI * 440.).sin();
                [v, v]
            })
            .collect();
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        for chunk in input.chunks(1152 * 2) {
            r.process(chunk, &mut out);
        }
        let frames = out.len() / 2;
        assert!((frames as i64 - 48_000).abs() < 10, "{frames}");
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let crossings = left.windows(2).filter(|w| w[0] < 0. && w[1] >= 0.).count();
        assert!((438..=442).contains(&crossings), "{crossings}");
        // Smooth: no jumps between neighbouring samples beyond the tone's slope.
        let max_step = left
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0., f32::max);
        assert!(max_step < 0.06, "{max_step}");
    }

    #[test]
    fn mono_and_multichannel_become_stereo() {
        let mut out = Vec::new();
        to_stereo(&[0.1, 0.2], 1, &mut out);
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2]);
        out.clear();
        to_stereo(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6], 3, &mut out);
        assert_eq!(out, [0.1, 0.2, 0.4, 0.5]);
    }
}
