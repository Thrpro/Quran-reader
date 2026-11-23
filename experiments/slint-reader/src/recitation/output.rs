//! The sound device: a short queue of stretched stereo samples, played by the
//! device's own thread. Volume and pause act on the very next device buffer,
//! with brief fades so they never click.
use super::stretch::CHANNELS;
use crate::data::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Where a run of queued output came from: from output frame `at`, source
/// position `source` (in the program's frames), `rate` source frames per frame.
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub at: u64,
    pub source: f64,
    pub rate: f64,
}

pub struct Queue {
    samples: VecDeque<f32>,
    marks: VecDeque<Mark>,
    /// Output frames ever queued and ever played.
    pub written: u64,
    pub played: u64,
    pub paused: bool,
    pub volume: f32,
    /// The current fade (0 silent to 1 full) and volume as applied, which
    /// move towards their targets a little every sample.
    fade: f32,
    gain: f32,
    /// Bumped on every flush, so a waiting producer can tell.
    pub generation: u64,
    /// Frames the device had to fill with silence while playing because
    /// nothing was queued: audible gaps. For verification.
    pub starved: u64,
    /// When set, a copy of everything played (verification only).
    pub tap: Option<Vec<f32>>,
}

pub struct Shared {
    pub queue: Mutex<Queue>,
    /// Signalled when the device takes samples, so the producer can refill.
    pub space: Condvar,
    pub failed: AtomicBool,
}

impl Shared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(Queue {
                samples: VecDeque::new(),
                marks: VecDeque::new(),
                written: 0,
                played: 0,
                paused: false,
                volume: 1.,
                fade: 1.,
                gain: 1.,
                generation: 0,
                starved: 0,
                tap: None,
            }),
            space: Condvar::new(),
            failed: AtomicBool::new(false),
        })
    }
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Queue {
    /// Frames queued but not yet played.
    pub fn pending(&self) -> u64 {
        self.written - self.played
    }
    pub fn push(&mut self, samples: &[f32], source: f64, rate: f64) {
        let frames = (samples.len() / CHANNELS) as u64;
        if frames == 0 {
            return;
        }
        self.marks.push_back(Mark {
            at: self.written,
            source,
            rate,
        });
        self.samples.extend(samples);
        self.written += frames;
    }
    /// Drops everything queued (a seek or stop).
    pub fn flush(&mut self) {
        self.samples.clear();
        self.marks.clear();
        self.written = self.played;
        self.generation += 1;
    }
    /// The program position being heard now, in source frames.
    pub fn audible(&self) -> Option<f64> {
        let mark = self.marks.iter().rev().find(|m| m.at <= self.played)?;
        Some(mark.source + (self.played - mark.at) as f64 * mark.rate)
    }

    /// Fills one device buffer of `channels`-channel frames.
    fn fill(&mut self, out: &mut [f32], channels: usize, sample_rate: u32) {
        // Fades of about 8 ms for pause and resume, 30 ms for volume.
        let fade_step = 1. / (sample_rate as f32 * 0.008);
        let gain_step = 1. / (sample_rate as f32 * 0.03);
        for frame in out.chunks_mut(channels) {
            let fade_target = if self.paused { 0. } else { 1. };
            self.fade += (fade_target - self.fade).clamp(-fade_step, fade_step);
            self.gain += (self.volume - self.gain).clamp(-gain_step, gain_step);
            // Paused and faded out: hold the place without consuming.
            if self.paused && self.fade <= 0. || self.samples.len() < CHANNELS {
                if !self.paused && !self.marks.is_empty() {
                    self.starved += 1;
                }
                frame.fill(0.);
                continue;
            }
            let left = self.samples.pop_front().unwrap_or(0.);
            let right = self.samples.pop_front().unwrap_or(0.);
            self.played += 1;
            if let Some(tap) = &mut self.tap {
                tap.extend([left * self.fade, right * self.fade]);
            }
            let level = self.fade * self.gain;
            match channels {
                1 => frame[0] = (left + right) * 0.5 * level,
                _ => {
                    frame[0] = left * level;
                    frame[1] = right * level;
                    frame[2..].fill(0.);
                }
            }
        }
        while self.marks.len() > 1 && self.marks[1].at <= self.played {
            self.marks.pop_front();
        }
    }
}

/// An open output stream on the default device.
pub struct Output {
    _stream: cpal::Stream,
    pub sample_rate: u32,
}

impl Output {
    pub fn open(shared: Arc<Shared>) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("No sound output device was found")?;
        let supported = device.default_output_config()?;
        let config = supported.config();
        let sample_rate = config.sample_rate;
        let channels = config.channels.max(1) as usize;
        let failed = shared.clone();
        let on_error = move |error: cpal::Error| {
            eprintln!("Sound output error: {error}");
            failed.failed.store(true, Ordering::Release);
            failed.space.notify_all();
        };
        let mut buffer = Vec::new();
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => {
                let shared = shared.clone();
                device.build_output_stream::<f32, _, _>(
                    config,
                    move |out, _| {
                        shared.lock().fill(out, channels, sample_rate);
                        shared.space.notify_one();
                    },
                    on_error,
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let shared = shared.clone();
                device.build_output_stream::<i16, _, _>(
                    config,
                    move |out, _| {
                        buffer.resize(out.len(), 0.);
                        shared.lock().fill(&mut buffer, channels, sample_rate);
                        shared.space.notify_one();
                        for (o, v) in out.iter_mut().zip(&buffer) {
                            *o = (v.clamp(-1., 1.) * i16::MAX as f32) as i16;
                        }
                    },
                    on_error,
                    None,
                )?
            }
            cpal::SampleFormat::I32 => {
                let shared = shared.clone();
                device.build_output_stream::<i32, _, _>(
                    config,
                    move |out, _| {
                        buffer.resize(out.len(), 0.);
                        shared.lock().fill(&mut buffer, channels, sample_rate);
                        shared.space.notify_one();
                        for (o, v) in out.iter_mut().zip(&buffer) {
                            *o = (v.clamp(-1., 1.) as f64 * i32::MAX as f64) as i32;
                        }
                    },
                    on_error,
                    None,
                )?
            }
            other => return Err(format!("Unsupported sound output format {other}").into()),
        };
        stream.play()?;
        Ok(Self {
            _stream: stream,
            sample_rate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_follow_what_was_played() {
        let shared = Shared::new();
        let mut q = shared.lock();
        q.push(&[0.5; 200], 1000., 1.5);
        q.push(&[0.5; 200], 1150., 1.5);
        let mut out = vec![0.; 300];
        q.fill(&mut out, 2, 48_000);
        assert_eq!(q.played, 150);
        assert_eq!(q.audible(), Some(1000. + 150. * 1.5));
        q.fill(&mut out, 2, 48_000);
        // 200 frames were queued; the rest of the buffer is silence.
        assert_eq!(q.played, 200);
        assert!(out[100..].iter().all(|&v| v == 0.));
    }

    #[test]
    fn pause_fades_out_then_holds_the_place() {
        let shared = Shared::new();
        let mut q = shared.lock();
        q.push(&[0.5; 20_000], 0., 1.);
        q.paused = true;
        let mut out = vec![0.; 2000];
        q.fill(&mut out, 2, 48_000);
        let held = q.played;
        assert!(held > 300 && held < 500, "{held}");
        assert!(out[0] > 0. && out[out.len() - 1] == 0.);
        q.fill(&mut out, 2, 48_000);
        assert_eq!(q.played, held);
        q.paused = false;
        q.fill(&mut out, 2, 48_000);
        assert!(q.played > held);
    }

    #[test]
    fn flushing_drops_queued_audio() {
        let shared = Shared::new();
        let mut q = shared.lock();
        q.push(&[0.5; 2000], 0., 1.);
        q.flush();
        assert_eq!(q.pending(), 0);
        assert_eq!(q.audible(), None);
    }
}
