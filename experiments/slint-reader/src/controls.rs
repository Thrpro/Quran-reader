//! Slint-facing application controls. Native resources, decoding and profile
//! publication retain their existing implementations and ownership boundaries.
use crate::*;
use std::cell::{Cell, RefCell};

pub fn reference_title(surahs: &[data::Surah], key: (u32, u32), arabic: bool) -> String {
    surahs
        .iter()
        .find(|s| s.id == key.0)
        .map(|s| {
            if arabic {
                format!("{} · الآية {}", s.name_arabic, key.1)
            } else {
                format!("{} · Ayah {}", s.name_simple, key.1)
            }
        })
        .unwrap_or_else(|| format!("{}:{}", key.0, key.1))
}
fn resolve(
    ui: &ReaderWindow,
    state: &Rc<RefCell<State>>,
    queue: &Queue,
    key: (u32, u32),
    step: i32,
    kind: ResolveKind,
) -> i32 {
    let mut s = state.borrow_mut();
    s.reference_generation = s.reference_generation.wrapping_add(1).max(1);
    queue.push(Task::Resolve(
        s.reference_generation,
        key,
        step.clamp(-1, 1),
        ui.get_arabic(),
        kind,
    ));
    s.reference_generation
}
struct Inner {
    profile: RefCell<Option<preferences::Profile>>,
    prefs: RefCell<preferences::Preferences>,
    surahs: Vec<data::Surah>,
    audio: RefCell<audio_bridge::AudioBridge>,
    last_audio: RefCell<Option<audio_bridge::AudioSnapshot>>,
    follow_request: Cell<Option<((u32, u32), i32)>>,
    language: Cell<bool>,
    initial_motion: bool,
    explicit_rate: Cell<bool>,
    qa: bool,
}
pub struct Controller {
    inner: Rc<Inner>,
    save_timer: Rc<slint::Timer>,
    audio_timer: Rc<slint::Timer>,
    notice_timer: Rc<slint::Timer>,
    clock_timer: slint::Timer,
}
impl Controller {
    pub fn open(ui: &ReaderWindow, root: &Path, profile: PathBuf, qa: bool) -> Result<Self> {
        let requested_profile = profile.clone();
        let surahs = data::Resources::open(root.to_path_buf())?.surahs;
        let (profile, prefs, error) = if qa {
            (
                None,
                preferences::Preferences {
                    arabic: false,
                    onboarding_complete: true,
                    show_clock: false,
                    study_source: 2,
                    ..Default::default()
                },
                false,
            )
        } else {
            match preferences::Profile::load(&profile, &surahs) {
                Ok(p) => {
                    let prefs = p.preferences().clone();
                    (Some(p), prefs, false)
                }
                Err(_) => (None, preferences::Preferences::default(), true),
            }
        };
        let motion = prefs.motion.unwrap_or_else(motion_preference);
        ui.set_arabic(prefs.arabic);
        ui.set_onboarding_complete(prefs.onboarding_complete);
        ui.set_theme(prefs.theme as i32);
        ui.set_dark(prefs.is_dark());
        ui.set_contextual(prefs.contextual);
        ui.set_show_clock(prefs.show_clock);
        ui.set_clock_24(prefs.clock_24);
        ui.set_single_page(prefs.single_page);
        ui.set_study_source(prefs.study_source as i32);
        ui.set_study_size(prefs.study_text_size as f32);
        ui.set_follow_recitation(prefs.follow_recitation);
        ui.set_open_tafsir_on_click(prefs.open_tafsir_on_click);
        ui.set_software_rendering(prefs.software_rendering);
        ui.set_search_shortcut(prefs.search_shortcut as i32);
        ui.set_listen_shortcut(prefs.listen_shortcut as i32);
        ui.set_options_shortcut(prefs.options_shortcut as i32);
        ui.set_motion_enabled(motion);
        ui.set_volume(prefs.volume as f32);
        ui.set_remember_speed(prefs.remember_speed);
        ui.set_playback_rate(if prefs.remember_speed {
            prefs.playback_rate as f32
        } else {
            1.
        });
        ui.set_reciter_index(prefs.reciter.min(audio::RECITERS.len() - 1) as i32);
        ui.set_session_only(error);
        let profile_path = profile
            .as_ref()
            .map(|p| p.directory().to_path_buf())
            .unwrap_or(requested_profile);
        let inner = Rc::new(Inner {
            profile: RefCell::new(profile),
            prefs: RefCell::new(prefs.clone()),
            surahs,
            audio: RefCell::new(audio_bridge::AudioBridge::new(profile_path)),
            last_audio: RefCell::new(None),
            follow_request: Cell::new(None),
            language: Cell::new(prefs.arabic),
            initial_motion: motion,
            explicit_rate: Cell::new(false),
            qa,
        });
        let result = Self {
            inner,
            save_timer: Rc::new(slint::Timer::default()),
            audio_timer: Rc::new(slint::Timer::default()),
            notice_timer: Rc::new(slint::Timer::default()),
            clock_timer: slint::Timer::default(),
        };
        result.reciter_names(ui);
        if error {
            result.notice(
                ui,
                if prefs.arabic {
                    "تعذّر فتح الإعدادات المحفوظة. يمكنك مواصلة القراءة لهذه الجلسة."
                } else {
                    "Saved preferences could not be opened. You can still read in this session."
                },
            );
        }
        Ok(result)
    }
    pub fn initialize_state(&self, ui: &ReaderWindow, state: &Rc<RefCell<State>>) {
        let prefs = self.inner.prefs.borrow();
        let mut s = state.borrow_mut();
        s.page = prefs.current_page;
        s.surahs = self.inner.surahs.clone();
        s.saved_bookmarks = prefs.bookmarks.clone();
        drop(s);
        ui.set_page(prefs.current_page as i32);
        ui.set_page_input(prefs.current_page.to_string().into());
        drop(prefs);
        self.bookmark_rows(ui, state);
    }
    fn notice(&self, ui: &ReaderWindow, text: &str) {
        Self::show_notice(ui, text, self.notice_timer.clone());
    }
    fn show_notice(ui: &ReaderWindow, text: &str, timer: Rc<slint::Timer>) {
        ui.set_notice(text.into());
        let weak = ui.as_weak();
        timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(2400),
            move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_notice("".into());
                }
            },
        );
    }
    fn reciter_names(&self, ui: &ReaderWindow) {
        Self::names(ui);
    }
    fn names(ui: &ReaderWindow) {
        ui.set_reciters(ModelRc::new(VecModel::from(
            audio::RECITERS
                .iter()
                .map(|r| {
                    slint::SharedString::from(if ui.get_arabic() { r.arabic } else { r.english })
                })
                .collect::<Vec<_>>(),
        )));
    }
    fn bookmark_rows(&self, ui: &ReaderWindow, state: &Rc<RefCell<State>>) {
        Self::rows(ui, state, &self.inner);
    }
    fn rows(ui: &ReaderWindow, state: &Rc<RefCell<State>>, inner: &Inner) {
        let rows = state
            .borrow()
            .saved_bookmarks
            .iter()
            .rev()
            .map(|b| BookmarkEntry {
                title: reference_title(&inner.surahs, b.key(), ui.get_arabic()).into(),
                detail: if ui.get_arabic() {
                    format!("الصفحة {}", b.page)
                } else {
                    format!("Page {}", b.page)
                }
                .into(),
            })
            .collect::<Vec<_>>();
        ui.set_bookmarks(ModelRc::new(VecModel::from(rows)));
    }
    fn capture(
        ui: &ReaderWindow,
        state: &Rc<RefCell<State>>,
        inner: &Inner,
    ) -> preferences::Preferences {
        let mut prefs = inner.prefs.borrow().clone();
        prefs.arabic = ui.get_arabic();
        prefs.onboarding_complete = ui.get_onboarding_complete();
        prefs.theme = ui.get_theme().clamp(0, 2) as u32;
        prefs.contextual = ui.get_contextual();
        prefs.show_clock = ui.get_show_clock();
        prefs.clock_24 = ui.get_clock_24();
        prefs.single_page = ui.get_single_page();
        prefs.follow_recitation = ui.get_follow_recitation();
        prefs.open_tafsir_on_click = ui.get_open_tafsir_on_click();
        prefs.software_rendering = ui.get_software_rendering();
        prefs.search_shortcut = ui.get_search_shortcut().max(0) as u32;
        prefs.listen_shortcut = ui.get_listen_shortcut().max(0) as u32;
        prefs.options_shortcut = ui.get_options_shortcut().max(0) as u32;
        prefs.study_source = if prefs.arabic {
            ui.get_study_source().clamp(0, 1) as u32
        } else {
            2
        };
        prefs.study_text_size = ui.get_study_size().clamp(14., 26.).round() as u32;
        prefs.current_page = state.borrow().page;
        prefs.bookmarks = state.borrow().saved_bookmarks.clone();
        prefs.volume = f64::from(ui.get_volume());
        prefs.remember_speed = ui.get_remember_speed();
        prefs.playback_rate = f64::from(ui.get_playback_rate());
        prefs.reciter = ui
            .get_reciter_index()
            .clamp(0, audio::RECITERS.len() as i32 - 1) as usize;
        if ui.get_motion_enabled() != inner.initial_motion || prefs.motion.is_some() {
            prefs.motion = Some(ui.get_motion_enabled());
        }
        prefs.sanitize(&inner.surahs);
        prefs
    }
    fn save_now(ui: &ReaderWindow, state: &Rc<RefCell<State>>, inner: &Inner) {
        let prefs = Self::capture(ui, state, inner);
        *inner.prefs.borrow_mut() = prefs.clone();
        if inner.qa {
            return;
        }
        let result = if let Some(profile) = inner.profile.borrow_mut().as_mut() {
            profile.replace(prefs, &inner.surahs)
        } else {
            return;
        };
        if result.is_err() {
            ui.set_session_only(true);
            ui.set_render_status(
                if ui.get_arabic() {
                    "تعذّر حفظ التغييرات. تظل خيارات هذه الجلسة فعّالة."
                } else {
                    "Changes could not be saved. Your session still works."
                }
                .into(),
            );
        }
    }
    fn publish_audio(
        ui: &ReaderWindow,
        state: &Rc<RefCell<State>>,
        queue: &Queue,
        inner: &Inner,
        snapshot: audio_bridge::AudioSnapshot,
    ) {
        if inner.last_audio.borrow().as_ref() != Some(&snapshot) {
            ui.set_audio_active(snapshot.active || snapshot.error);
            ui.set_audio_paused(snapshot.paused || snapshot.completed || snapshot.error);
            ui.set_audio_terminal(snapshot.completed || snapshot.error);
            ui.set_audio_loading(snapshot.loading);
            ui.set_audio_can_seek(snapshot.can_seek);
            ui.set_audio_progress(snapshot.slider as f32 / 10000.);
            ui.set_audio_clock(snapshot.clock.clone().into());
            // Pause/Resume already communicates the steady playback state.
            // Keep the small status line for loading, completion and errors.
            ui.set_audio_status(
                if snapshot.loading || snapshot.completed || snapshot.error {
                    snapshot.status.clone().into()
                } else {
                    "".into()
                },
            );
            ui.set_audio_title(
                snapshot
                    .current
                    .map(|key| reference_title(&inner.surahs, key, ui.get_arabic()))
                    .unwrap_or_default()
                    .into(),
            );
        }
        let current = if snapshot.active {
            snapshot.current
        } else {
            None
        };
        let opening = if snapshot.active {
            snapshot.opening_surah
        } else {
            None
        };
        let mut s = state.borrow_mut();
        let changed = s.recitation != current || s.opening != opening;
        s.recitation = current;
        s.opening = opening;
        let follow = ui.get_follow_recitation()
            && ui.get_panel() == "study"
            && current.is_some_and(|key| s.selection != Some(key));
        let generation = s.reference_generation;
        drop(s);
        if follow {
            let key = current.unwrap();
            // One pending resolve per key/generation. Transport ticks must not
            // invalidate the worker's own reply while it is still resolving.
            if inner.follow_request.get() != Some((key, generation)) {
                let generation = resolve(ui, state, queue, key, 0, ResolveKind::Study);
                inner.follow_request.set(Some((key, generation)));
            }
        } else if changed {
            inner.follow_request.set(None);
            request_render(ui, state, queue);
        } else {
            inner.follow_request.set(None);
        }
        *inner.last_audio.borrow_mut() = Some(snapshot);
    }
    fn poll_once(ui: &ReaderWindow, state: &Rc<RefCell<State>>, queue: &Queue, inner: &Inner) {
        let observed = { inner.audio.borrow_mut().poll(ui.get_arabic()) };
        match observed {
            Ok(snapshot) => Self::publish_audio(ui, state, queue, inner, snapshot),
            Err(_) => {
                ui.set_audio_status(
                    if ui.get_arabic() {
                        "تعذّر تشغيل التسجيل. حاول تشغيله مجددًا."
                    } else {
                        "The recording could not play. Try Play again."
                    }
                    .into(),
                );
            }
        }
    }
    fn start_poll(
        ui: &ReaderWindow,
        state: Rc<RefCell<State>>,
        queue: Arc<Queue>,
        inner: Rc<Inner>,
        timer: Rc<slint::Timer>,
    ) {
        Self::poll_once(ui, &state, &queue, &inner);
        if !inner.audio.borrow().needs_poll() {
            return;
        }
        let weak = ui.as_weak();
        let timer_weak = Rc::downgrade(&timer);
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(80),
            move || {
                if let Some(ui) = weak.upgrade() {
                    Self::poll_once(&ui, &state, &queue, &inner);
                }
                if !inner.audio.borrow().needs_poll()
                    && let Some(timer) = timer_weak.upgrade()
                {
                    timer.stop();
                }
            },
        );
    }
    fn play(
        ui: &ReaderWindow,
        state: Rc<RefCell<State>>,
        queue: Arc<Queue>,
        inner: Rc<Inner>,
        timer: Rc<slint::Timer>,
    ) {
        let key = {
            let s = state.borrow();
            s.context
                .map(|v| v.0)
                .or(s.selection)
                .or_else(|| s.probe.map(|p| p.2))
        };
        let Some(key) = key else {
            ui.set_audio_status(
                if ui.get_arabic() {
                    "اختر آية من الصفحة لبدء الاستماع."
                } else {
                    "Select a verse on the page to start listening."
                }
                .into(),
            );
            return;
        };
        let rate = Self::new_play_rate(ui, &inner);
        let volume = f64::from(ui.get_volume()) / 100.;
        let result = inner.audio.borrow_mut().play(
            ui.get_reciter_index()
                .clamp(0, audio::RECITERS.len() as i32 - 1) as usize,
            key,
            ui.get_listen_only_ayah(),
            rate,
            volume,
        );
        match result {
            Ok(()) => {
                *inner.last_audio.borrow_mut() = None;
                ui.set_context_open(false);
                ui.set_player_expanded(true);
                Self::start_poll(ui, state, queue, inner, timer);
            }
            Err(error) => ui.set_audio_status(error.to_string().into()),
        }
    }
    fn new_play_rate(ui: &ReaderWindow, inner: &Inner) -> f64 {
        // A deliberate edit before Play applies once. Live speed edits never
        // become an implicit preference for the next new recitation.
        let explicit = inner.explicit_rate.replace(false);
        if !ui.get_remember_speed() && !explicit {
            ui.set_playback_rate(1.);
        }
        f64::from(ui.get_playback_rate())
    }
    pub fn connect(&self, ui: &ReaderWindow, state: &Rc<RefCell<State>>, queue: &Arc<Queue>) {
        {
            let inner = self.inner.clone();
            let state = state.clone();
            let queue = queue.clone();
            let weak = ui.as_weak();
            let timer = self.save_timer.clone();
            ui.on_preferences_changed(move || {
                if let Some(ui) = weak.upgrade() {
                    if inner.language.replace(ui.get_arabic()) != ui.get_arabic() {
                        ui.set_study_source(if ui.get_arabic() { 0 } else { 2 });
                        Self::names(&ui);
                        Self::rows(&ui, &state, &inner);
                        if !ui.get_search_query().is_empty() {
                            ui.invoke_search(ui.get_search_query());
                        }
                        *inner.last_audio.borrow_mut() = None;
                    }
                    let source = if ui.get_arabic() {
                        ui.get_study_source().clamp(0, 1)
                    } else {
                        2
                    };
                    if source != ui.get_study_source() {
                        ui.set_study_source(source);
                    }
                    // Also update terminal/paused players, whose polling timer
                    // may have stopped, and respond immediately to Follow.
                    let snapshot = { inner.audio.borrow().snapshot(ui.get_arabic()) };
                    Self::publish_audio(&ui, &state, &queue, &inner, snapshot);
                    request_render(&ui, &state, &queue);
                    if !inner.qa {
                        let weak = ui.as_weak();
                        let inner = inner.clone();
                        let state = state.clone();
                        timer.start(
                            slint::TimerMode::SingleShot,
                            Duration::from_millis(350),
                            move || {
                                if let Some(ui) = weak.upgrade() {
                                    Self::save_now(&ui, &state, &inner);
                                }
                            },
                        );
                    }
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let weak = ui.as_weak();
            ui.on_context_ayah(move |x, y| {
                if let Some(ui) = weak.upgrade()
                    && !ui.get_render_busy()
                {
                    queue.push(Task::Pick(
                        state.borrow().committed,
                        x - ui.get_image_x(),
                        y - ui.get_image_y(),
                        PickKind::Menu { x, y },
                    ));
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let weak = ui.as_weak();
            ui.on_study_selected(move || {
                if let Some(ui) = weak.upgrade() {
                    let key = state.borrow().context.map(|v| v.0);
                    if let Some(key) = key {
                        ui.set_follow_recitation(false);
                        ui.set_context_open(false);
                        resolve(&ui, &state, &queue, key, 0, ResolveKind::Study);
                    }
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let weak = ui.as_weak();
            ui.on_study_step(move |direction| {
                if let Some(ui) = weak.upgrade() {
                    let key = state.borrow().selection;
                    if let Some(key) = key {
                        ui.set_follow_recitation(false);
                        resolve(&ui, &state, &queue, key, direction, ResolveKind::Study);
                    }
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let weak = ui.as_weak();
            ui.on_study_source_changed(move |source| {
                if let Some(ui) = weak.upgrade() {
                    ui.set_study_source(source);
                    ui.invoke_preferences_changed();
                    request_render(&ui, &state, &queue);
                }
            });
        }
        {
            let state = state.clone();
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            let timer = self.notice_timer.clone();
            ui.on_bookmark_selected(move || {
                if let Some(ui) = weak.upgrade() {
                    let selected = {
                        let s = state.borrow();
                        s.context
                            .or_else(|| s.selection.map(|key| (key, s.selection_page.max(1))))
                    };
                    if let Some((key, page)) = selected {
                        let mut s = state.borrow_mut();
                        let added = if let Some(i) =
                            s.saved_bookmarks.iter().position(|b| b.key() == key)
                        {
                            s.saved_bookmarks.remove(i);
                            false
                        } else {
                            if s.saved_bookmarks.len() >= preferences::MAX_BOOKMARKS {
                                drop(s);
                                Self::show_notice(
                                    &ui,
                                    if ui.get_arabic() {
                                        "وصلت إلى الحد الأقصى للعلامات."
                                    } else {
                                        "The bookmark limit has been reached."
                                    },
                                    timer.clone(),
                                );
                                return;
                            }
                            s.saved_bookmarks.push(preferences::Bookmark {
                                surah: key.0,
                                ayah: key.1,
                                page,
                            });
                            true
                        };
                        drop(s);
                        ui.set_bookmark_present(added);
                        ui.set_context_open(false);
                        Self::rows(&ui, &state, &inner);
                        ui.invoke_preferences_changed();
                        Self::show_notice(
                            &ui,
                            match (added, ui.get_arabic()) {
                                (true, true) => "حُفظت العلامة",
                                (false, true) => "حُذفت العلامة",
                                (true, false) => "Bookmark saved",
                                _ => "Bookmark removed",
                            },
                            timer.clone(),
                        );
                    }
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let weak = ui.as_weak();
            ui.on_choose_bookmark(move |index| {
                if let Some(ui) = weak.upgrade() {
                    let key = {
                        let s = state.borrow();
                        usize::try_from(index)
                            .ok()
                            .and_then(|index| s.saved_bookmarks.iter().rev().nth(index))
                            .map(|b| b.key())
                    };
                    if let Some(key) = key {
                        ui.set_follow_recitation(false);
                        resolve(&ui, &state, &queue, key, 0, ResolveKind::Study);
                    }
                }
            });
        }
        {
            let state = state.clone();
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            let timer = self.notice_timer.clone();
            ui.on_remove_bookmark(move |index| {
                if let Some(ui) = weak.upgrade() {
                    let mut s = state.borrow_mut();
                    if let Ok(i) = usize::try_from(index)
                        && i < s.saved_bookmarks.len()
                    {
                        let index = s.saved_bookmarks.len() - 1 - i;
                        s.saved_bookmarks.remove(index);
                    }
                    drop(s);
                    Self::rows(&ui, &state, &inner);
                    ui.invoke_preferences_changed();
                    Self::show_notice(
                        &ui,
                        if ui.get_arabic() {
                            "حُذفت العلامة"
                        } else {
                            "Bookmark removed"
                        },
                        timer.clone(),
                    );
                }
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_fullscreen_requested(move || {
                if let Some(ui) = weak.upgrade() {
                    let fullscreen = !ui.get_fullscreen();
                    ui.set_fullscreen(fullscreen);
                    ui.window().with_winit_window(|window| {
                        window.set_fullscreen(
                            fullscreen.then_some(winit::window::Fullscreen::Borderless(None)),
                        );
                        window.request_redraw();
                    });
                    ui.invoke_focus_reading();
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            let timer = self.audio_timer.clone();
            ui.on_play_selected(move || {
                if let Some(ui) = weak.upgrade() {
                    Self::play(
                        &ui,
                        state.clone(),
                        queue.clone(),
                        inner.clone(),
                        timer.clone(),
                    );
                }
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_listen_selected(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_listen_only_ayah(false);
                    ui.invoke_play_selected();
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            let timer = self.audio_timer.clone();
            ui.on_audio_toggle(move || {
                if let Some(ui) = weak.upgrade() {
                    let snapshot = { inner.audio.borrow().snapshot(ui.get_arabic()) };
                    if snapshot.completed || snapshot.error {
                        let rate = Self::new_play_rate(&ui, &inner);
                        let _ = inner.audio.borrow_mut().set_speed(rate);
                    }
                    let result = inner.audio.borrow_mut().toggle();
                    if result.is_ok() {
                        Self::start_poll(
                            &ui,
                            state.clone(),
                            queue.clone(),
                            inner.clone(),
                            timer.clone(),
                        );
                    }
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            let timer = self.audio_timer.clone();
            ui.on_audio_end(move || {
                if let Some(ui) = weak.upgrade() {
                    timer.stop();
                    let _ = inner.audio.borrow_mut().stop();
                    inner.explicit_rate.set(false);
                    inner.follow_request.set(None);
                    if !ui.get_remember_speed() {
                        ui.set_playback_rate(1.);
                    }
                    *inner.last_audio.borrow_mut() = None;
                    let snapshot = { inner.audio.borrow().snapshot(ui.get_arabic()) };
                    Self::publish_audio(&ui, &state, &queue, &inner, snapshot);
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            let timer = self.audio_timer.clone();
            ui.on_audio_seek(move |fraction| {
                if let Some(ui) = weak.upgrade()
                    && fraction.is_finite()
                {
                    let _ = inner
                        .audio
                        .borrow_mut()
                        .seek_slider((fraction.clamp(0., 1.) * 10000.).round() as u32);
                    Self::start_poll(
                        &ui,
                        state.clone(),
                        queue.clone(),
                        inner.clone(),
                        timer.clone(),
                    );
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            let timer = self.audio_timer.clone();
            ui.on_audio_skip(move |seconds| {
                if let Some(ui) = weak.upgrade() {
                    let _ = inner
                        .audio
                        .borrow_mut()
                        .seek_delta(i64::from(seconds.clamp(-10, 10)) * 1000);
                    Self::start_poll(
                        &ui,
                        state.clone(),
                        queue.clone(),
                        inner.clone(),
                        timer.clone(),
                    );
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            ui.on_audio_rate(move |rate| {
                if let Some(ui) = weak.upgrade()
                    && rate.is_finite()
                {
                    ui.set_playback_rate(rate.clamp(0.5, 2.));
                    let snapshot = { inner.audio.borrow().snapshot(ui.get_arabic()) };
                    inner
                        .explicit_rate
                        .set(!snapshot.active || snapshot.completed || snapshot.error);
                    let _ = inner
                        .audio
                        .borrow_mut()
                        .set_speed(f64::from(rate.clamp(0.5, 2.)));
                    Self::poll_once(&ui, &state, &queue, &inner);
                    ui.invoke_preferences_changed();
                }
            });
        }
        {
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            ui.on_audio_volume(move |volume| {
                if let Some(ui) = weak.upgrade()
                    && volume.is_finite()
                {
                    ui.set_volume(volume.clamp(0., 100.));
                    let _ = inner
                        .audio
                        .borrow_mut()
                        .set_volume(f64::from(volume.clamp(0., 100.)) / 100.);
                    ui.invoke_preferences_changed();
                }
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_reciter_changed(move |index| {
                if let Some(ui) = weak.upgrade() {
                    ui.set_reciter_index(index.clamp(0, audio::RECITERS.len() as i32 - 1));
                    ui.invoke_preferences_changed();
                }
            });
        }
        {
            let state = state.clone();
            let queue = queue.clone();
            let weak = ui.as_weak();
            ui.on_show_recited_ayah(move || {
                if let Some(ui) = weak.upgrade() {
                    let key = state.borrow().recitation;
                    if let Some(key) = key {
                        resolve(&ui, &state, &queue, key, 0, ResolveKind::Recitation);
                    }
                }
            });
        }
        if !self.inner.qa {
            let weak = ui.as_weak();
            let state = state.clone();
            let queue = queue.clone();
            self.clock_timer.start(
                slint::TimerMode::Repeated,
                Duration::from_secs(30),
                move || {
                    if let Some(ui) = weak.upgrade()
                        && ui.get_show_clock()
                    {
                        request_render(&ui, &state, &queue);
                    }
                },
            );
        }
    }
    pub(crate) fn qa_snapshot(&self) -> audio_bridge::AudioSnapshot {
        self.inner
            .audio
            .borrow()
            .snapshot(self.inner.language.get())
    }
    pub(crate) fn qa_polling(&self) -> bool {
        self.audio_timer.running()
    }
    pub fn finish(&self, ui: &ReaderWindow, state: &Rc<RefCell<State>>) {
        self.save_timer.stop();
        self.audio_timer.stop();
        self.notice_timer.stop();
        self.clock_timer.stop();
        Self::save_now(ui, state, &self.inner);
        let _ = self.inner.audio.borrow_mut().stop();
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.save_timer.stop();
        self.audio_timer.stop();
        self.notice_timer.stop();
        self.clock_timer.stop();
        let _ = self.inner.audio.borrow_mut().stop();
    }
}
