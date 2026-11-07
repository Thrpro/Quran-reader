//! Slint-side ownership of the existing, validated native recitation engine.
//! No new transport, decoder, timing interpretation or provider is introduced.
//! Construct and use on the UI thread; its event loop pumps the WinRT apartment.
use crate::{
    audio::{Audio, ListeningRange, RECITERS, Transport},
    audio_cache::{AudioCache, COUNTS},
    data::Result,
    listening_layout,
    timed_audio::Recording,
};
use std::{
    marker::PhantomData,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use windows::Win32::System::WinRT::{
    RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize,
};

/// Plain observed state. All times are actual content milliseconds, not a
/// synthetic timer or an estimate derived from ayah count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioSnapshot {
    pub active: bool,
    pub paused: bool,
    pub loading: bool,
    pub completed: bool,
    pub error: bool,
    pub continuous: bool,
    pub can_seek: bool,
    pub current: Option<(u32, u32)>,
    pub opening_surah: Option<u32>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub range_start_ms: u64,
    pub range_end_ms: u64,
    pub slider: u32,
    pub clock: String,
    pub status: String,
}

#[derive(Clone, Copy)]
struct Request {
    reciter: usize,
    key: (u32, u32),
    only_ayah: bool,
}

struct PendingSeek {
    target_ms: u64,
    paused: bool,
    began: Instant,
    sent_at: Option<Instant>,
}

struct Apartment;
impl Apartment {
    fn initialize() -> Result<Self> {
        // Slint or accessibility may already own a compatible COM apartment.
        // Retry the other apartment only for the precise changed-mode result;
        // each successful RoInitialize is balanced once on this same thread.
        unsafe {
            match RoInitialize(RO_INIT_SINGLETHREADED) {
                Ok(()) => {}
                Err(error) if error.code().0 as u32 == 0x80010106 => {
                    RoInitialize(RO_INIT_MULTITHREADED)?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

/// Lazy: no player, cache, workers or profile directory exists before Play.
pub struct AudioBridge {
    audio: Option<Audio>,
    profile_root: PathBuf,
    request: Option<Request>,
    last_transport: Option<Transport>,
    last_key: Option<(u32, u32)>,
    last_error: Option<String>,
    pending_seek: Option<PendingSeek>,
    rate: f64,
    volume: f64,
    // Audio owns its cache. Keep the apartment guard after it so media/cache
    // resources are dropped before the final RoUninitialize on this thread.
    _apartment: Option<Apartment>,
    _owning_thread: PhantomData<Rc<()>>,
}

impl AudioBridge {
    pub fn new(profile_root: PathBuf) -> Self {
        Self {
            audio: None,
            profile_root,
            request: None,
            last_transport: None,
            last_key: None,
            last_error: None,
            pending_seek: None,
            rate: 1.,
            volume: 1.,
            _apartment: None,
            _owning_thread: PhantomData,
        }
    }

    fn initialize(&mut self) -> Result<()> {
        if self.audio.is_some() {
            return Ok(());
        }
        let apartment = Apartment::initialize()?;
        let cache = AudioCache::new(self.profile_root.clone())?;
        let audio = Audio::with_cache(cache)?;
        self._apartment = Some(apartment);
        self.audio = Some(audio);
        Ok(())
    }

    /// A deliberate Play request is the only entry into the native network
    /// preparation paths. Continuous recording defaults to the same supported
    /// publisher bindings as the native app; another voice stays that voice.
    pub fn play(
        &mut self,
        reciter: usize,
        key: (u32, u32),
        only_ayah: bool,
        rate: f64,
        volume: f64,
    ) -> Result<()> {
        let range = checked_range(reciter, key, only_ayah)?;
        checked_controls(rate, volume)?;
        self.initialize()?;
        let audio = self.audio.as_mut().ok_or("Audio player is unavailable")?;
        if Recording::for_reciter(reciter).is_some() {
            audio.play_timed_scoped(range, rate, volume)?;
        } else {
            audio.play_range(range, rate, volume)?;
        }
        self.request = Some(Request {
            reciter,
            key,
            only_ayah,
        });
        self.rate = rate;
        self.volume = volume;
        self.last_transport = None;
        self.last_key = Some(key);
        self.last_error = None;
        self.pending_seek = None;
        Ok(())
    }

    pub fn toggle(&mut self) -> Result<()> {
        if let Some(pending) = &mut self.pending_seek {
            pending.paused = !pending.paused;
            return Ok(());
        }
        if self.last_error.is_some()
            || self
                .audio
                .as_ref()
                .is_some_and(|audio| audio.completed() || native_failed(audio))
        {
            let request = self.request.ok_or("Choose a verse to play first")?;
            return self.play(
                request.reciter,
                request.key,
                request.only_ayah,
                self.rate,
                self.volume,
            );
        }
        self.audio
            .as_mut()
            .filter(|audio| audio.has_session())
            .ok_or("Choose a verse to play first")?
            .toggle()
    }

    pub fn stop(&mut self) -> Result<()> {
        let result = self.audio.as_mut().map(Audio::stop).transpose();
        self.request = None;
        self.last_transport = None;
        self.last_key = None;
        self.last_error = None;
        self.pending_seek = None;
        result.map(|_| ())
    }

    pub fn set_speed(&mut self, rate: f64) -> Result<()> {
        checked_controls(rate, self.volume)?;
        if let Some(audio) = &mut self.audio {
            audio.request_speed(rate)?;
        }
        self.rate = rate;
        Ok(())
    }

    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        checked_controls(self.rate, volume)?;
        if let Some(audio) = &self.audio
            && self.pending_seek.is_none()
        {
            audio.volume(volume)?;
        }
        self.volume = volume;
        Ok(())
    }

    pub fn seek_delta(&mut self, milliseconds: i64) -> Result<bool> {
        let Some(transport) = self.observed_transport() else {
            return Ok(false);
        };
        let from = self.pending_seek.as_ref().map_or_else(
            || transport.offset_ms.saturating_add(transport.position_ms),
            |pending| pending.target_ms,
        );
        let target = from
            .saturating_add_signed(milliseconds)
            .min(transport.duration_ms.saturating_sub(50));
        self.seek_absolute(target)
    }

    /// A chapter slider can leave a from-here range: reopen the same validated
    /// recording from its beginning, then perform the desired absolute seek.
    /// The new source remains paused and muted until its real position agrees.
    pub fn seek_slider(&mut self, value: u32) -> Result<bool> {
        let Some(transport) = self.observed_transport() else {
            return Ok(false);
        };
        let target =
            ((u128::from(transport.duration_ms) * u128::from(value.min(10_000))) / 10_000) as u64;
        self.seek_absolute(target.min(transport.duration_ms.saturating_sub(50)))
    }

    fn observed_transport(&self) -> Option<Transport> {
        self.audio
            .as_ref()
            .and_then(Audio::transport)
            .or(self.last_transport)
    }

    fn seek_absolute(&mut self, target_ms: u64) -> Result<bool> {
        if let Some(pending) = &mut self.pending_seek {
            pending.target_ms = target_ms;
            pending.began = Instant::now();
            pending.sent_at = None;
            return Ok(true);
        }
        let Some(transport) = self.observed_transport().filter(|value| value.can_seek) else {
            return Ok(false);
        };
        let end = transport.offset_ms.saturating_add(transport.range_ms);
        if transport.continuous && (target_ms < transport.offset_ms || target_ms >= end) {
            return self.reopen_for_seek(target_ms);
        }
        let relative_target = target_ms.saturating_sub(transport.offset_ms);
        let delta = i64::try_from(i128::from(relative_target) - i128::from(transport.position_ms))?;
        self.audio
            .as_mut()
            .map_or(Ok(false), |audio| audio.seek(Some(delta), 0))
    }

    fn reopen_for_seek(&mut self, target_ms: u64) -> Result<bool> {
        let request = self.request.ok_or("Choose a verse to play first")?;
        let range = checked_range(request.reciter, (request.key.0, 1), false)?;
        let audio = self.audio.as_mut().ok_or("Audio player is unavailable")?;
        let paused = audio.is_paused();
        let result = (|| -> Result<()> {
            audio.play_timed_scoped(range, self.rate, 0.)?;
            // Set the pause intent before any asynchronous source is attached.
            audio.toggle()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = audio.stop();
            self.last_error = Some(error.to_string());
            return Err(error);
        }
        self.request = Some(Request {
            reciter: request.reciter,
            key: (request.key.0, 1),
            only_ayah: false,
        });
        self.last_error = None;
        self.pending_seek = Some(PendingSeek {
            target_ms,
            paused,
            began: Instant::now(),
            sent_at: None,
        });
        Ok(true)
    }

    fn advance_pending_seek(&mut self) -> Result<()> {
        let Some(pending) = &mut self.pending_seek else {
            return Ok(());
        };
        if pending.began.elapsed() > Duration::from_secs(30) {
            return Err("The recording did not become ready for seeking".into());
        }
        let audio = self.audio.as_mut().ok_or("Audio player is unavailable")?;
        if native_failed(audio) {
            return Err("The recording could not be reopened for seeking".into());
        }
        let Some(transport) = audio
            .transport()
            .filter(|value| value.continuous && value.can_seek)
        else {
            return Ok(());
        };
        let target = pending
            .target_ms
            .min(transport.duration_ms.saturating_sub(50));
        if pending.sent_at.is_none() {
            let current = transport.offset_ms.saturating_add(transport.position_ms);
            let delta = i64::try_from(i128::from(target) - i128::from(current))?;
            if audio.seek(Some(delta), 0)? {
                pending.target_ms = target;
                pending.sent_at = Some(Instant::now());
            }
            return Ok(());
        }
        if pending
            .sent_at
            .is_some_and(|time| time.elapsed() < Duration::from_millis(50))
            || transport
                .offset_ms
                .saturating_add(transport.position_ms)
                .abs_diff(target)
                > 1_000
        {
            return Ok(());
        }
        let paused = pending.paused;
        audio.volume(self.volume)?;
        if !paused {
            audio.toggle()?;
        }
        self.pending_seek = None;
        Ok(())
    }

    /// Stop UI polling after End/completion/failure; no audio timer is needed
    /// before a user starts listening. Paused preparation still needs polling.
    pub fn needs_poll(&self) -> bool {
        self.last_error.is_none()
            && (self.pending_seek.is_some()
                || self.audio.as_ref().is_some_and(Audio::needs_fast_poll))
    }

    pub fn poll(&mut self, arabic: bool) -> Result<AudioSnapshot> {
        let result = self.audio.as_mut().map(Audio::poll).transpose();
        let result = result.and_then(|_| self.advance_pending_seek());
        if let Err(error) = result {
            if let Some(audio) = &mut self.audio {
                let _ = audio.stop();
            }
            self.last_error = Some(error.to_string());
            self.pending_seek = None;
        }
        if let Some(audio) = &mut self.audio {
            // A merely failed lookahead remains healthy until the native
            // decoder reaches its actual stop boundary. Do not invent errors.
            let current = audio.current();
            if let Some(key) = current
                && self.pending_seek.is_none()
                && !audio.completed()
            {
                if self.last_key != Some(key)
                    && self
                        .last_transport
                        .is_some_and(|transport| !transport.continuous)
                {
                    self.last_transport = None;
                }
                self.last_key = Some(key);
            }
            if let Some(transport) = audio.transport()
                && self.pending_seek.is_none()
            {
                self.last_transport = Some(transport);
            }
        }
        Ok(self.snapshot(arabic))
    }

    pub fn snapshot(&self, arabic: bool) -> AudioSnapshot {
        let Some(audio) = &self.audio else {
            return AudioSnapshot::default();
        };
        let active = self.request.is_some() && (audio.has_session() || self.last_error.is_some());
        if !active {
            return AudioSnapshot::default();
        }
        let completed = audio.completed();
        let failed = self.last_error.is_some() || native_failed(audio);
        let mut transport = if self.pending_seek.is_some() {
            self.last_transport
        } else {
            audio.transport().or(self.last_transport)
        };
        if completed && let Some(transport) = &mut transport {
            // End-of-stream is decoder-confirmed. Some sessions reset their
            // position immediately afterward; retain the selected range end
            // instead of flashing its chapter/clip clock back to zero.
            transport.position_ms = transport.range_ms;
        }
        let (position_ms, duration_ms, range_start_ms, range_end_ms, slider, clock) =
            clock_fields(transport, arabic);
        let paused = self
            .pending_seek
            .as_ref()
            .map_or_else(|| audio.is_paused(), |pending| pending.paused);
        let loading = !failed && !completed && (self.pending_seek.is_some() || transport.is_none());
        let status = if failed {
            if arabic {
                "تعذّر تشغيل التسجيل. أعد المحاولة."
            } else {
                "Could not play this recording. Try again."
            }
        } else if completed {
            if arabic {
                "اكتملت التلاوة"
            } else {
                "Recitation finished"
            }
        } else if self.pending_seek.is_some() {
            if arabic {
                "جارٍ الانتقال في التلاوة…"
            } else {
                "Seeking in recitation…"
            }
        } else if loading {
            if arabic {
                "جارٍ تحميل التلاوة…"
            } else {
                "Loading recitation…"
            }
        } else if paused {
            if arabic {
                "متوقف مؤقتًا"
            } else {
                "Paused"
            }
        } else if arabic {
            "يتم التشغيل"
        } else {
            "Playing"
        };
        AudioSnapshot {
            active,
            paused,
            loading,
            completed,
            error: failed,
            continuous: transport.is_some_and(|value| value.continuous),
            can_seek: !failed
                && !completed
                && self.pending_seek.is_none()
                && transport.is_some_and(|value| value.can_seek),
            current: if self.pending_seek.is_some() || completed {
                self.last_key
            } else {
                audio.current().or(self.last_key)
            },
            opening_surah: audio.opening_surah(),
            position_ms,
            duration_ms,
            range_start_ms,
            range_end_ms,
            slider,
            clock,
            status: status.into(),
        }
    }
}

impl Drop for AudioBridge {
    fn drop(&mut self) {
        if let Some(audio) = &mut self.audio {
            let _ = audio.stop();
        }
    }
}

fn checked_range(reciter: usize, key: (u32, u32), only_ayah: bool) -> Result<ListeningRange> {
    let count = key
        .0
        .checked_sub(1)
        .and_then(|index| COUNTS.get(index as usize));
    let Some(&count) = count else {
        return Err("Invalid recitation chapter".into());
    };
    if reciter >= RECITERS.len() || key.1 == 0 || key.1 > count {
        return Err("Invalid recitation verse or reciter".into());
    }
    Ok(ListeningRange {
        reciter,
        surah: key.0,
        first: key.1,
        last: if only_ayah { key.1 } else { count },
        count,
    })
}

fn checked_controls(rate: f64, volume: f64) -> Result<()> {
    if !rate.is_finite() || !(0.5..=2.).contains(&rate) {
        return Err("Playback speed must be between 0.5 and 2".into());
    }
    if !volume.is_finite() || !(0. ..=1.).contains(&volume) {
        return Err("Playback volume must be between 0 and 1".into());
    }
    Ok(())
}

fn native_failed(audio: &Audio) -> bool {
    // Native policy retains terminal playlists for the UI. Its fast-poll
    // predicate excludes an actual decoder failure, but deliberately includes
    // paused/buffering sessions and healthy prepared prefixes after lookahead
    // failure. This avoids treating an ordinary preparation delay as an error.
    audio.has_session() && !audio.completed() && !audio.needs_fast_poll()
}

fn clock_fields(transport: Option<Transport>, arabic: bool) -> (u64, u64, u64, u64, u32, String) {
    let Some(transport) = transport.filter(|value| value.duration_ms > 0) else {
        return (0, 0, 0, 0, 0, String::new());
    };
    let position = transport
        .offset_ms
        .saturating_add(transport.position_ms)
        .min(transport.duration_ms);
    let end = transport
        .offset_ms
        .saturating_add(transport.range_ms)
        .min(transport.duration_ms);
    let slider = ((u128::from(position) * 10_000) / u128::from(transport.duration_ms)) as u32;
    (
        position,
        transport.duration_ms,
        transport.offset_ms,
        end,
        slider,
        listening_layout::progress_label(
            transport.continuous,
            arabic,
            &listening_layout::clock(position),
            &listening_layout::clock(transport.duration_ms),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_range_uses_native_chapter_boundaries() {
        let range = checked_range(0, (2, 145), false).unwrap();
        assert_eq!((range.first, range.last, range.count), (145, 286, 286));
        let range = checked_range(5, (112, 2), true).unwrap();
        assert_eq!((range.first, range.last, range.count), (2, 2, 4));
        for key in [(0, 1), (115, 1), (1, 0), (1, 8), (112, 5)] {
            assert!(checked_range(0, key, false).is_err());
        }
        assert!(checked_range(RECITERS.len(), (1, 1), false).is_err());
    }

    #[test]
    fn chapter_clock_includes_offset_without_fabricating_duration() {
        let transport = Transport {
            position_ms: 15_000,
            range_ms: 900_000,
            offset_ms: 600_000,
            duration_ms: 1_500_000,
            continuous: true,
            can_seek: true,
        };
        let (position, duration, start, end, slider, text) = clock_fields(Some(transport), false);
        assert_eq!(
            (position, duration, start, end),
            (615_000, 1_500_000, 600_000, 1_500_000)
        );
        assert_eq!(slider, 4100);
        assert_eq!(text, "10:15 / 25:00");
        assert_eq!(clock_fields(None, true), (0, 0, 0, 0, 0, String::new()));
    }

    #[test]
    fn clip_clock_is_explicit_and_controls_reject_nonfinite_values() {
        let transport = Transport {
            position_ms: 2000,
            range_ms: 8000,
            offset_ms: 0,
            duration_ms: 8000,
            continuous: false,
            can_seek: true,
        };
        assert_eq!(clock_fields(Some(transport), false).5, "Ayah 0:02 / 0:08");
        assert!(checked_controls(f64::NAN, 1.).is_err());
        assert!(checked_controls(1., f64::INFINITY).is_err());
        assert!(checked_controls(1., -0.1).is_err());
        assert!(checked_controls(0.5, 0.).is_ok());
        assert!(checked_controls(2., 1.).is_ok());
    }
}
