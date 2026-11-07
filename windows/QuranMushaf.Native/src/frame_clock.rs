//! Optional UI-thread frame scheduling. This measures application cadence;
//! a waitable timer is not a display-vblank or compositor-presentation fence.
use crate::data::Result;
use std::{
    marker::PhantomData,
    rc::Rc,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CancelWaitableTimer, CreateWaitableTimerExW,
        INFINITE, SetWaitableTimer, TIMER_ALL_ACCESS, WaitForSingleObject,
    },
    UI::WindowsAndMessaging::{
        GetMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE,
        PeekMessageW, QS_ALLINPUT, WM_QUIT,
    },
};

pub enum FrameEvent {
    Tick,
    Message(MSG),
    Quit(i32),
}

pub struct FrameClock {
    timer: HANDLE,
    high_resolution: bool,
    period: Duration,
    deadline: Option<Instant>,
    offer_tick: bool,
    _ui_thread: PhantomData<Rc<()>>,
}
impl FrameClock {
    pub fn new() -> Result<Self> {
        unsafe {
            let (timer, high_resolution) = match CreateWaitableTimerExW(
                None,
                None,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            ) {
                Ok(timer) => (timer, true),
                Err(_) => (
                    CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0)?,
                    false,
                ),
            };
            Ok(Self {
                timer,
                high_resolution,
                period: Duration::from_millis(16),
                deadline: None,
                offer_tick: false,
                _ui_thread: PhantomData,
            })
        }
    }
    pub fn high_resolution(&self) -> bool {
        self.high_resolution
    }
    /// Idempotent while the same cadence is active. The caller arms only for
    /// visible motion and cancels at rest; no global timer-resolution change.
    pub fn arm(&mut self, period: Duration) -> Result<()> {
        if !(Duration::from_millis(1)..=Duration::from_millis(16)).contains(&period) {
            return Err("Frame period must be between 1 and 16 milliseconds".into());
        }
        if self.deadline.is_some() && self.period == period {
            return Ok(());
        }
        let deadline = Instant::now() + period;
        self.schedule(deadline)?;
        self.period = period;
        self.deadline = Some(deadline);
        self.offer_tick = false;
        Ok(())
    }
    pub fn disarm(&mut self) -> Result<()> {
        if self.deadline.is_some() {
            unsafe {
                CancelWaitableTimer(self.timer)?;
            }
            self.deadline = None;
        }
        self.offer_tick = false;
        Ok(())
    }
    fn schedule(&self, deadline: Instant) -> Result<()> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        // Negative 100 ns units express a relative deadline. A minimum of one
        // avoids converting an already elapsed deadline into an absolute time.
        let ticks = (remaining.as_nanos().div_ceil(100))
            .max(1)
            .min(i64::MAX as u128) as i64;
        let due = -ticks;
        unsafe {
            SetWaitableTimer(self.timer, &due, 0, None, None, false)?;
        }
        Ok(())
    }
    fn tick(&mut self) -> Result<FrameEvent> {
        let now = Instant::now();
        let previous = self.deadline.ok_or("Inactive frame timer signaled")?;
        let elapsed = now.saturating_duration_since(previous);
        let period_ns = self.period.as_nanos();
        // Coalesce missed deadlines rather than replaying a backlog. Calculate
        // the next relative interval without multiplying an unbounded counter.
        let remaining_ns = period_ns - elapsed.as_nanos() % period_ns;
        let deadline = now + Duration::from_nanos(remaining_ns as u64);
        self.schedule(deadline)?;
        self.deadline = Some(deadline);
        self.offer_tick = false;
        Ok(FrameEvent::Tick)
    }
    /// Wait for one message or one coalesced frame. The caller keeps its normal
    /// accelerator/dialog dispatch and explicitly routes Tick to animation work.
    /// No worker thread, synchronous App callback or busy wait is introduced.
    pub fn next_event(&mut self) -> Result<FrameEvent> {
        unsafe {
            if self.deadline.is_none() {
                let mut message = MSG::default();
                return match GetMessageW(&mut message, None, 0, 0).0 {
                    -1 => Err(windows::core::Error::from_thread().into()),
                    0 => Ok(FrameEvent::Quit(message.wParam.0 as i32)),
                    _ => Ok(FrameEvent::Message(message)),
                };
            }
            loop {
                // After a queued message, allow one ready timer before taking
                // another message. After a tick, allow a message first. This
                // keeps continuous input and slow frames from starving either.
                if self.offer_tick {
                    match WaitForSingleObject(self.timer, 0) {
                        value if value == WAIT_OBJECT_0 => return self.tick(),
                        value if value == WAIT_TIMEOUT => {}
                        value if value == WAIT_FAILED => {
                            return Err(windows::core::Error::from_thread().into());
                        }
                        _ => return Err("Unexpected frame timer poll result".into()),
                    }
                }
                let mut message = MSG::default();
                if PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    self.offer_tick = true;
                    return if message.message == WM_QUIT {
                        Ok(FrameEvent::Quit(message.wParam.0 as i32))
                    } else {
                        Ok(FrameEvent::Message(message))
                    };
                }
                let ready = MsgWaitForMultipleObjectsEx(
                    Some(&[self.timer]),
                    INFINITE,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                );
                if ready == WAIT_OBJECT_0 {
                    return self.tick();
                }
                if ready == WAIT_FAILED {
                    return Err(windows::core::Error::from_thread().into());
                }
                if ready.0 != WAIT_OBJECT_0.0 + 1 {
                    return Err("Unexpected frame/message wait result".into());
                }
                // Message availability can include sent messages serviced by
                // PeekMessage itself. Loop back and wait again when no posted
                // message remains; no arbitrary polling sleep is necessary.
            }
        }
    }
}
impl Drop for FrameClock {
    fn drop(&mut self) {
        unsafe {
            let _ = CancelWaitableTimer(self.timer);
            let _ = CloseHandle(self.timer);
        }
    }
}
