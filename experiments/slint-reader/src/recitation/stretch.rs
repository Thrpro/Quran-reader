//! Speed change without changing pitch: WSOLA (waveform-similarity overlap-add).
//!
//! The input is cut into overlapping windows taken `hop × rate` apart and laid
//! down `hop` apart, so playback is `rate` times as fast. Each window is moved
//! by up to `tolerance` frames to where its waveform best continues the one
//! before it, which keeps voiced speech free of the warble of plain
//! overlap-add. A new rate applies from the next window (about 12 ms), so
//! changing speed never interrupts the sound. At rate 1 the best continuation
//! is the input itself, and the output reproduces it exactly.

/// Interleaved stereo throughout.
pub const CHANNELS: usize = 2;

pub struct Stretch {
    /// Window length in frames (even); windows are laid down `hop = window/2` apart.
    window: usize,
    hop: usize,
    tolerance: usize,
    /// Periodic Hann window: copies `hop` apart sum to exactly one.
    shape: Vec<f32>,
    /// Input not yet consumed, interleaved, starting at absolute frame `start`.
    input: Vec<f32>,
    /// The same input mixed to mono, for the similarity search.
    mono: Vec<f32>,
    start: i64,
    /// Where the next window would be taken without searching (absolute frame).
    nominal: f64,
    /// Where the previous window was taken.
    previous: Option<i64>,
    /// Overlap-add accumulator, `window` frames.
    sum: Vec<f32>,
}

/// One block of output: `frames` frames whose first comes from source frame
/// `source`, advancing `rate` source frames per output frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub frames: usize,
    pub source: f64,
    pub rate: f64,
}

impl Stretch {
    /// For audio at `sample_rate`, starting at absolute source frame `at`.
    pub fn new(sample_rate: u32, at: i64) -> Self {
        // About 24 ms windows and 9 ms search: long enough for the lowest
        // voices' pitch periods, short enough to keep consonants crisp.
        let window = ((sample_rate as f64 * 0.024) as usize).max(64) & !1;
        let hop = window / 2;
        let tolerance = (sample_rate as f64 * 0.009) as usize;
        let shape = (0..window)
            .map(|i| {
                0.5 - 0.5 * (2. * std::f64::consts::PI * i as f64 / window as f64).cos() as f32
            })
            .collect();
        let mut stretch = Self {
            window,
            hop,
            tolerance,
            shape,
            input: Vec::new(),
            mono: Vec::new(),
            start: at,
            nominal: at as f64,
            previous: None,
            sum: vec![0.; window * CHANNELS],
        };
        // Half a window of silence in front, so the first real frames are
        // covered by two windows like every other frame, not faded in.
        stretch.start -= hop as i64;
        stretch.nominal -= hop as f64;
        stretch.push(&vec![0.; hop * CHANNELS]);
        stretch
    }

    pub fn push(&mut self, frames: &[f32]) {
        debug_assert!(frames.len().is_multiple_of(CHANNELS));
        self.input.extend_from_slice(frames);
        self.mono.extend(
            frames
                .as_chunks::<CHANNELS>()
                .0
                .iter()
                .map(|f| (f[0] + f[1]) * 0.5),
        );
    }

    /// Absolute frame just past the last input frame.
    pub fn input_end(&self) -> i64 {
        self.start + self.mono.len() as i64
    }

    /// Output frames still waiting in the overlap when the input ends.
    pub fn tail(&self) -> usize {
        self.window + self.tolerance
    }

    /// Produces one block of `hop` frames at `rate` into `out`, if enough
    /// input has arrived.
    pub fn step(&mut self, rate: f64, out: &mut Vec<f32>) -> Option<Block> {
        let rate = rate.clamp(0.25, 4.);
        let analysis = self.hop as f64 * rate;
        let nominal = self.nominal.round() as i64;
        // The natural continuation of the previous window, which a candidate
        // should resemble.
        let target = self.previous.map(|p| p + self.hop as i64);
        let need = (nominal + self.tolerance as i64 + self.window as i64)
            .max(target.map_or(0, |t| t + self.window as i64));
        if need > self.input_end() {
            return None;
        }
        let chosen = match target {
            None => nominal,
            Some(target) => self.best_match(nominal, target),
        };
        let at = (chosen - self.start) as usize;
        for (i, w) in self.shape.iter().enumerate() {
            for c in 0..CHANNELS {
                self.sum[i * CHANNELS + c] += self.input[(at + i) * CHANNELS + c] * w;
            }
        }
        out.extend_from_slice(&self.sum[..self.hop * CHANNELS]);
        self.sum.copy_within(self.hop * CHANNELS.., 0);
        let len = self.sum.len();
        self.sum[len - self.hop * CHANNELS..].fill(0.);
        let block = Block {
            frames: self.hop,
            source: chosen as f64,
            rate,
        };
        self.previous = Some(chosen);
        self.nominal += analysis;
        self.discard();
        Some(block)
    }

    /// The candidate start within `tolerance` of `nominal` whose waveform best
    /// matches the one starting at `target`; the nearest wins ties, so an
    /// exact continuation is always kept.
    fn best_match(&self, nominal: i64, target: i64) -> i64 {
        // At the target itself the match is exact: no search needed.
        if target == nominal {
            return nominal;
        }
        let length = self.window;
        let at = |frame: i64| (frame - self.start) as usize;
        let reference = &self.mono[at(target)..at(target) + length];
        let lowest = (nominal - self.tolerance as i64).max(self.start);
        let mut best = (f32::MIN, i64::MAX);
        for offset in 0..=self.tolerance as i64 {
            for candidate in [nominal - offset, nominal + offset] {
                if candidate < lowest || (offset == 0 && candidate != nominal) {
                    continue;
                }
                let samples = &self.mono[at(candidate)..at(candidate) + length];
                let (mut dot, mut energy) = (0f32, 1e-9f32);
                // Every second sample: half the work, no loss of accuracy for speech.
                for i in (0..length).step_by(2) {
                    dot += samples[i] * reference[i];
                    energy += samples[i] * samples[i];
                }
                let score = dot / energy.sqrt();
                if score > best.0 {
                    best = (score, candidate);
                }
            }
        }
        best.1
    }

    fn discard(&mut self) {
        let keep_from = (self.nominal.floor() as i64 - self.tolerance as i64)
            .min(self.previous.map_or(i64::MAX, |p| p + self.hop as i64))
            .max(self.start);
        let drop = (keep_from - self.start) as usize;
        if drop > 4096 {
            self.input.drain(..drop * CHANNELS);
            self.mono.drain(..drop);
            self.start += drop as i64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, rate: u32) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                let v = (t * 2. * std::f32::consts::PI * 180.).sin() * 0.4
                    + (t * 2. * std::f32::consts::PI * 523.).sin() * 0.2;
                [v, v * 0.9]
            })
            .collect()
    }

    fn run(input: &[f32], rate: f64) -> (Vec<f32>, Vec<Block>) {
        let mut s = Stretch::new(44_100, 0);
        s.push(input);
        let (mut out, mut blocks) = (Vec::new(), Vec::new());
        while let Some(block) = s.step(rate, &mut out) {
            blocks.push(block);
        }
        (out, blocks)
    }

    #[test]
    fn normal_speed_reproduces_the_input_exactly() {
        let input = tone(44_100, 44_100);
        let (out, _) = run(&input, 1.);
        // The first output block is the leading half window of silence.
        let hop = Stretch::new(44_100, 0).hop;
        let produced = &out[hop * CHANNELS..];
        assert!(produced.len() > 30_000 * CHANNELS);
        for (a, b) in produced.iter().zip(&input) {
            assert!((a - b).abs() < 1e-5, "{a} vs {b}");
        }
    }

    #[test]
    fn speed_changes_length_but_not_pitch() {
        let input = tone(88_200, 44_100);
        for rate in [0.5, 0.75, 1.25, 1.5, 2.] {
            let (out, blocks) = run(&input, rate);
            let frames = out.len() / CHANNELS;
            let expected = 88_200. / rate;
            assert!(
                (frames as f64 - expected).abs() < expected * 0.06,
                "rate {rate}: {frames} frames, expected about {expected}"
            );
            // The 180 Hz fundamental survives: the strongest repetition of the
            // left channel, in the middle, is still one 180 Hz period.
            let mid = &out[(frames / 4) * CHANNELS..(frames * 3 / 4) * CHANNELS];
            let left: Vec<f32> = mid.iter().step_by(CHANNELS).copied().collect();
            let correlation =
                |lag: usize| -> f32 { left.iter().zip(&left[lag..]).map(|(a, b)| a * b).sum() };
            // Periods between 100 and 400 Hz.
            let lag = (110..=441)
                .max_by(|&a, &b| correlation(a).total_cmp(&correlation(b)))
                .unwrap();
            let hz = 44_100. / lag as f64;
            assert!((176. ..=184.).contains(&hz), "rate {rate}: {hz} Hz");
            assert!(blocks.iter().all(|b| b.rate == rate));
        }
    }

    #[test]
    fn a_new_rate_applies_without_a_gap() {
        let input = tone(88_200, 44_100);
        let mut s = Stretch::new(44_100, 0);
        s.push(&input);
        let mut out = Vec::new();
        let mut rate = 1.;
        let mut steps = 0;
        while s.step(rate, &mut out).is_some() {
            steps += 1;
            if steps == 40 {
                rate = 1.5;
            }
        }
        // No silent stretch anywhere after the first block: the loudest
        // sample of every 5 ms stays well above zero.
        let hop = s.hop;
        for chunk in out[hop * 2 * CHANNELS..].chunks(220 * CHANNELS) {
            let peak = chunk.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.2, "quiet chunk, peak {peak}");
        }
    }

    #[test]
    fn positions_follow_the_source() {
        let input = tone(44_100, 44_100);
        let (_, blocks) = run(&input, 2.);
        for pair in blocks.windows(2) {
            let advance = pair[1].source - pair[0].source;
            let tolerance = 2. * Stretch::new(44_100, 0).tolerance as f64;
            assert!((advance - pair[0].frames as f64 * 2.).abs() <= tolerance);
        }
    }
}
