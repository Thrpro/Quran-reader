//! `--first-run-probe fresh|relaunch`, with `--profile <folder>`: walks the
//! welcome screen the way a new user does, with a real profile, and reports
//! how long the interface stops answering. For CI on macOS, where a tester's
//! Mac froze on this screen.
//!
//! `fresh` expects a new profile: it chooses English, starts reading and turns
//! a page; closing saves the profile. `relaunch` then checks that both choices
//! were kept. The steps call the same properties and callbacks as the welcome
//! card's buttons; window-system input and hit testing are not exercised.
use crate::ReaderWindow;
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

pub struct Probe {
    _heartbeat: slint::Timer,
    _steps: slint::Timer,
    failure: Rc<RefCell<Option<String>>>,
}

impl Probe {
    pub fn failure(&self) -> Option<String> {
        self.failure.borrow().clone()
    }
}

pub fn start(ui: &ReaderWindow, relaunch: bool) -> Probe {
    let failure = Rc::new(RefCell::new(None::<String>));
    // A free event loop runs this every 10 ms; a long gap is a stall.
    let heartbeat = slint::Timer::default();
    let longest = Rc::new(Cell::new(0f64));
    {
        // Starts at the first tick: the time before it is the app starting up.
        let last = Cell::new(None::<Instant>);
        let longest = longest.clone();
        heartbeat.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(10),
            move || {
                let now = Instant::now();
                let Some(previous) = last.replace(Some(now)) else {
                    return;
                };
                let gap = now.duration_since(previous).as_secs_f64() * 1000.;
                longest.set(longest.get().max(gap));
                if gap > 250. {
                    eprintln!("[probe] the interface did not answer for {gap:.0} ms");
                }
            },
        );
    }
    let steps = slint::Timer::default();
    {
        let weak = ui.as_weak();
        let failure = failure.clone();
        let began = Instant::now();
        let step = Cell::new(0u32);
        let since = Cell::new(Instant::now());
        let page = Cell::new(0i32);
        steps.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(50),
            move || {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                if step.get() == u32::MAX {
                    return;
                }
                let finish = |error: Option<String>| {
                    match &error {
                        Some(error) => eprintln!("[probe] failed: {error}"),
                        None => eprintln!("[probe] passed"),
                    }
                    *failure.borrow_mut() = error;
                    step.set(u32::MAX);
                    let _ = slint::quit_event_loop();
                };
                if began.elapsed() > Duration::from_secs(60) {
                    finish(Some(format!("timed out at step {}", step.get())));
                    return;
                }
                let waited = |ms: u64| since.get().elapsed() > Duration::from_millis(ms);
                let settled = !ui.get_render_busy() && !ui.get_turn_running();
                let next = |n: u32, what: &str| {
                    eprintln!(
                        "[probe] {what} at {:.0} ms; longest pause so far {:.0} ms",
                        began.elapsed().as_secs_f64() * 1000.,
                        longest.get()
                    );
                    step.set(n);
                    since.set(Instant::now());
                };
                match step.get() {
                    0 if waited(1500) && settled => {
                        if relaunch {
                            if !ui.get_onboarding_complete() {
                                finish(Some("the welcome screen came back after relaunch".into()));
                            } else if ui.get_arabic() {
                                finish(Some("English was not kept after relaunch".into()));
                            } else {
                                next(1, "relaunch kept English and the finished welcome");
                                finish(None);
                            }
                        } else if ui.get_onboarding_complete() {
                            finish(Some("a fresh profile skipped the welcome screen".into()));
                        } else if !ui.get_arabic() {
                            finish(Some("a fresh profile did not start in Arabic".into()));
                        } else {
                            longest.set(0.);
                            // The English half of the welcome card's language switch.
                            ui.set_arabic(false);
                            next(1, "chose English");
                        }
                    }
                    // Long enough for the 350 ms delayed save to have run.
                    1 if waited(1200) && settled => {
                        if ui.get_arabic() {
                            finish(Some("choosing English did not change the language".into()));
                            return;
                        }
                        // What the card's Start reading does.
                        ui.set_onboarding_complete(true);
                        ui.set_onboarding_step(0);
                        ui.invoke_preferences_changed();
                        ui.invoke_focus_reading();
                        next(2, "started reading");
                    }
                    2 if waited(1200) && settled => {
                        page.set(ui.get_rendered_page());
                        ui.invoke_navigate(1);
                        next(3, "turned a page");
                    }
                    3 if waited(1500) && settled => {
                        if ui.get_rendered_page() == page.get() {
                            finish(Some("the page did not turn".into()));
                        } else {
                            next(4, "the page turned");
                            finish(None);
                        }
                    }
                    _ => {}
                }
            },
        );
    }
    Probe {
        _heartbeat: heartbeat,
        _steps: steps,
        failure,
    }
}
