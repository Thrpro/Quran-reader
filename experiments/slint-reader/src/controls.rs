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
    downloads: RefCell<Vec<audio_bridge::DownloadedSurah>>,
    last_audio: RefCell<Option<audio_bridge::AudioSnapshot>>,
    follow_request: Cell<Option<((u32, u32), i32)>>,
    language: Cell<bool>,
    initial_motion: bool,
    explicit_rate: Cell<bool>,
    /// Where a recitation that spans surahs ends; each surah plays in turn.
    listen_end: Cell<Option<(u32, u32)>>,
    /// Whether the surah being recited is saved, for its reciter: (reciter, surah, saved).
    saved_check: Cell<Option<(usize, u32, bool)>>,
    qa: bool,
}
pub struct Controller {
    inner: Rc<Inner>,
    save_timer: Rc<slint::Timer>,
    audio_timer: Rc<slint::Timer>,
    download_timer: Rc<slint::Timer>,
    notice_timer: Rc<slint::Timer>,
    clock_timer: slint::Timer,
}
impl Controller {
    pub fn open(ui: &ReaderWindow, root: &Path, profile: PathBuf, qa: bool) -> Result<Self> {
        let requested_profile = profile.clone();
        let surahs = data::Resources::open(root.to_path_buf())?.surahs;
        let mut load_error = None;
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
                Err(error) => {
                    eprintln!("Opening the profile failed: {error}");
                    load_error = Some(error.to_string());
                    (None, preferences::Preferences::default(), true)
                }
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
        // Say why, so an already-open window is not mistaken for lost settings.
        if let Some(reason) = &load_error {
            let in_use = reason.contains("already open") || reason.contains("exclusively");
            ui.set_session_reason(
                match (in_use, prefs.arabic) {
                    (true, true) => "التطبيق مفتوح في نافذة أخرى، لذا لا تستطيع هذه النافذة حفظ التغييرات. أغلق النافذة الأخرى ثم أعد فتح التطبيق.",
                    (true, false) => "The app is already open in another window, so this window can't save changes. Close the other window and reopen the app.",
                    (false, true) => "تعذّرت قراءة إعداداتك المحفوظة، فتستخدم هذه الجلسة الإعدادات الافتراضية. لم يُغيَّر ملفك المحفوظ.",
                    (false, false) => "Your saved settings couldn't be read, so this session uses the defaults. Your saved file was left untouched.",
                }
                .into(),
            );
        }
        let profile_path = profile
            .as_ref()
            .map(|p| p.directory().to_path_buf())
            .unwrap_or(requested_profile);
        let inner = Rc::new(Inner {
            profile: RefCell::new(profile),
            prefs: RefCell::new(prefs.clone()),
            surahs,
            audio: RefCell::new(audio_bridge::AudioBridge::new(profile_path)),
            downloads: RefCell::new(Vec::new()),
            last_audio: RefCell::new(None),
            follow_request: Cell::new(None),
            language: Cell::new(prefs.arabic),
            initial_motion: motion,
            explicit_rate: Cell::new(false),
            listen_end: Cell::new(None),
            saved_check: Cell::new(None),
            qa,
        });
        let result = Self {
            inner,
            save_timer: Rc::new(slint::Timer::default()),
            audio_timer: Rc::new(slint::Timer::default()),
            download_timer: Rc::new(slint::Timer::default()),
            notice_timer: Rc::new(slint::Timer::default()),
            clock_timer: slint::Timer::default(),
        };
        result.reciter_names(ui);
        Self::surah_names(ui, &result.inner);
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
        s.shortcuts = shortcuts::resolve(
            &prefs.shortcuts,
            [
                prefs.search_shortcut,
                prefs.listen_shortcut,
                prefs.options_shortcut,
            ],
        );
        drop(s);
        ui.set_app_version(env!("CARGO_PKG_VERSION").into());
        ui.set_page(prefs.current_page as i32);
        ui.set_page_input(prefs.current_page.to_string().into());
        drop(prefs);
        self.bookmark_rows(ui, state);
        Self::shortcut_rows(ui, state);
    }
    fn shortcut_rows(ui: &ReaderWindow, state: &Rc<RefCell<State>>) {
        let arabic = ui.get_arabic();
        let rows = shortcuts::ACTIONS
            .iter()
            .zip(state.borrow().shortcuts.iter())
            .map(|(action, keys)| BookmarkEntry {
                title: if arabic {
                    action.arabic
                } else {
                    action.english
                }
                .into(),
                detail: shortcuts::display(keys, arabic).into(),
            })
            .collect::<Vec<_>>();
        ui.set_shortcut_rows(ModelRc::new(VecModel::from(rows)));
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
    fn surah_names(ui: &ReaderWindow, inner: &Inner) {
        let arabic = ui.get_arabic();
        ui.set_surah_names(ModelRc::new(VecModel::from(
            inner
                .surahs
                .iter()
                .map(|s| {
                    slint::SharedString::from(if arabic {
                        format!("{} · {}", s.id, s.name_arabic)
                    } else {
                        format!("{}. {}", s.id, s.name_simple)
                    })
                })
                .collect::<Vec<_>>(),
        )));
    }
    fn megabytes(bytes: u64, arabic: bool) -> String {
        let value = bytes as f64 / 1_048_576.;
        if arabic {
            format!("{value:.1} ميغابايت")
        } else {
            format!("{value:.1} MB")
        }
    }
    fn download_rows(ui: &ReaderWindow, inner: &Inner) {
        let arabic = ui.get_arabic();
        let list = inner.audio.borrow().downloaded();
        let rows = list
            .iter()
            .map(|d| {
                let surah = inner.surahs.iter().find(|s| s.id == d.surah);
                let reciter = &audio::RECITERS[d.reciter];
                BookmarkEntry {
                    title: surah
                        .map(|s| {
                            if arabic {
                                s.name_arabic.clone()
                            } else {
                                s.name_simple.clone()
                            }
                        })
                        .unwrap_or_else(|| d.surah.to_string())
                        .into(),
                    detail: if d.complete {
                        format!(
                            "{} · {}",
                            if arabic {
                                reciter.arabic
                            } else {
                                reciter.english
                            },
                            Self::megabytes(d.bytes, arabic)
                        )
                    } else if arabic {
                        format!("{} · غير مكتمل، نزّله مرة أخرى", reciter.arabic)
                    } else {
                        format!("{} · Incomplete, download it again", reciter.english)
                    }
                    .into(),
                }
            })
            .collect::<Vec<_>>();
        let total = list.iter().map(|d| d.bytes).sum::<u64>();
        ui.set_downloads_storage(Self::megabytes(total, arabic).into());
        ui.set_downloads(ModelRc::new(VecModel::from(rows)));
        *inner.downloads.borrow_mut() = list;
    }
    fn surah_title(inner: &Inner, surah: u32, arabic: bool) -> String {
        inner
            .surahs
            .iter()
            .find(|s| s.id == surah)
            .map(|s| {
                if arabic {
                    s.name_arabic.clone()
                } else {
                    s.name_simple.clone()
                }
            })
            .unwrap_or_else(|| surah.to_string())
    }
    fn begin_download(
        ui: &ReaderWindow,
        inner: &Rc<Inner>,
        timer: &Rc<slint::Timer>,
        reciter: usize,
        surah: u32,
    ) {
        let started = inner.audio.borrow_mut().start_download(reciter, surah);
        match started {
            Ok(()) => {
                ui.set_download_busy(true);
                ui.set_download_progress(0.);
                ui.set_download_status(
                    if ui.get_arabic() {
                        "جارٍ تجهيز التنزيل…"
                    } else {
                        "Preparing the download…"
                    }
                    .into(),
                );
                Self::poll_downloads(ui, inner.clone(), timer.clone());
            }
            Err(error) => {
                ui.set_download_status(
                    if ui.get_arabic() {
                        "تعذّر بدء التنزيل. حاول مرة أخرى.".to_string()
                    } else {
                        format!("The download could not start: {error}")
                    }
                    .into(),
                );
            }
        }
    }
    fn poll_downloads(ui: &ReaderWindow, inner: Rc<Inner>, timer: Rc<slint::Timer>) {
        let weak = ui.as_weak();
        let timer_weak = Rc::downgrade(&timer);
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(150),
            move || {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let arabic = ui.get_arabic();
                let target = inner.audio.borrow().downloading();
                let notice = inner.audio.borrow_mut().poll_download();
                let stop = match notice {
                    Some(audio_bridge::DownloadNotice::Progress(percent)) => {
                        ui.set_download_progress(percent as f32 / 100.);
                        ui.set_download_status(
                            if arabic {
                                format!("جارٍ التنزيل… {percent}٪")
                            } else {
                                format!("Downloading… {percent}%")
                            }
                            .into(),
                        );
                        false
                    }
                    Some(audio_bridge::DownloadNotice::Done) => {
                        inner.saved_check.set(None);
                        if inner.audio.borrow().current_surah() == target {
                            ui.set_current_saved(true);
                        }
                        let title = target
                            .map(|(_, surah)| Self::surah_title(&inner, surah, arabic))
                            .unwrap_or_default();
                        ui.set_download_progress(1.);
                        ui.set_download_status(
                            if arabic {
                                format!("حُفظت سورة {title} للاستماع دون اتصال.")
                            } else {
                                format!("{title} is saved for offline listening.")
                            }
                            .into(),
                        );
                        Self::download_rows(&ui, &inner);
                        true
                    }
                    Some(audio_bridge::DownloadNotice::Failed(message)) => {
                        ui.set_download_status(
                            if arabic {
                                "تعذّر التنزيل. تحقّق من الاتصال بالإنترنت ثم حاول مرة أخرى."
                                    .to_string()
                            } else {
                                format!("The download could not finish: {message}")
                            }
                            .into(),
                        );
                        Self::download_rows(&ui, &inner);
                        true
                    }
                    Some(audio_bridge::DownloadNotice::Cancelled) => true,
                    None => target.is_none(),
                };
                if stop {
                    ui.set_download_busy(false);
                    if let Some(timer) = timer_weak.upgrade() {
                        timer.stop();
                    }
                }
            },
        );
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
        prefs.theme = ui.get_theme().clamp(0, 4) as u32;
        prefs.contextual = ui.get_contextual();
        prefs.show_clock = ui.get_show_clock();
        prefs.clock_24 = ui.get_clock_24();
        prefs.single_page = ui.get_single_page();
        prefs.follow_recitation = ui.get_follow_recitation();
        prefs.open_tafsir_on_click = ui.get_open_tafsir_on_click();
        // No longer offered in Options: clear an earlier saved choice.
        prefs.software_rendering = false;
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
        if result.is_ok() && ui.get_session_only() {
            ui.set_session_only(false);
            ui.set_session_reason("".into());
        }
        if let Err(error) = &result {
            eprintln!("Saving preferences failed: {error}");
            ui.set_session_only(true);
            ui.set_session_reason(
                if ui.get_arabic() {
                    "تعذّر حفظ آخر تغيير. سيُعاد المحاولة عند التغيير التالي."
                } else {
                    "The last change could not be saved. It will be tried again with your next change."
                }
                .into(),
            );
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
            ui.set_audio_finished(snapshot.completed && !snapshot.error);
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
            let recited = inner.audio.borrow().current_surah();
            let saved = recited.is_some_and(|(reciter, surah)| match inner.saved_check.get() {
                Some((r, s, saved)) if r == reciter && s == surah => saved,
                _ => {
                    let saved = inner
                        .audio
                        .borrow()
                        .downloaded()
                        .iter()
                        .any(|d| d.reciter == reciter && d.surah == surah && d.complete);
                    inner.saved_check.set(Some((reciter, surah, saved)));
                    saved
                }
            });
            ui.set_current_saved(saved);
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
        let previous = s.recitation;
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
            if !Self::follow_page(ui, state, queue, previous, current) {
                request_render(ui, state, queue);
            }
        } else {
            inner.follow_request.set(None);
        }
        *inner.last_audio.borrow_mut() = Some(snapshot);
    }
    /// Turn to the page of the ayah being recited, as a reader following along
    /// would. Only when the previous ayah was on screen, so someone who has
    /// paged elsewhere is left where they are. Returns whether it navigated.
    fn follow_page(
        ui: &ReaderWindow,
        state: &Rc<RefCell<State>>,
        queue: &Queue,
        previous: Option<(u32, u32)>,
        current: Option<(u32, u32)>,
    ) -> bool {
        let Some(key) = current else { return false };
        if ui.get_drag_active() || ui.get_turn_running() || ui.get_book_sliding() {
            return false;
        }
        let s = state.borrow();
        let single = s.last["singlePage"].as_bool().unwrap_or(false);
        let page_of = |key: (u32, u32)| {
            s.page_last
                .iter()
                .position(|last| *last >= key)
                .map(|index| index as u32 + 1)
        };
        let shows = |page: u32, at: u32| {
            if single {
                page == at
            } else {
                app_core::spread(page) == app_core::spread(at)
            }
        };
        let Some(target) = page_of(key) else {
            return false;
        };
        if shows(target, s.page) {
            return false;
        }
        if previous.is_some_and(|previous| !page_of(previous).is_some_and(|p| shows(p, s.page))) {
            return false;
        }
        let next = app_core::destination(s.page, 1, single);
        let selection = s.selection;
        let turn = shows(target, next);
        drop(s);
        if turn {
            ui.invoke_navigate(1);
        } else {
            set_page(ui, state, queue, target, selection);
        }
        true
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
                    if Self::continue_range(&ui, &inner) {
                        return;
                    }
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
        let count = |surah: u32| {
            inner
                .surahs
                .iter()
                .find(|s| s.id == surah)
                .map_or(1, |s| s.verses_count)
        };
        let (start, end) = {
            let s = state.borrow();
            let page = s
                .context
                .map(|v| v.1)
                .filter(|_| s.context.is_some_and(|c| c.0 == key))
                .unwrap_or(if s.selection == Some(key) && s.selection_page > 0 {
                    s.selection_page
                } else {
                    s.page
                });
            match ui.get_listen_scope() {
                // This ayah only.
                1 => (key, key),
                // To the end of the page, which may run into the next surahs.
                2 => (
                    key,
                    s.page_last
                        .get(page.saturating_sub(1) as usize)
                        .copied()
                        .filter(|end| *end >= key)
                        .unwrap_or((key.0, count(key.0))),
                ),
                // The whole surah from its first verse.
                3 => ((key.0, 1), (key.0, count(key.0))),
                // On through the following surahs to the end of the Quran.
                4 => (key, (114, count(114))),
                _ => (key, (key.0, count(key.0))),
            }
        };
        let reciter = ui
            .get_reciter_index()
            .clamp(0, audio::RECITERS.len() as i32 - 1) as usize;
        let rate = Self::new_play_rate(ui, &inner);
        let volume = f64::from(ui.get_volume()) / 100.;
        inner.listen_end.set(Some(end));
        let result = inner.audio.borrow_mut().play(
            reciter,
            start,
            Some(if end.0 == start.0 {
                end.1
            } else {
                count(start.0)
            }),
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
    /// When a surah in a longer range has finished, play the next one with the
    /// same reciter, speed and volume. The timer keeps polling the new session.
    fn continue_range(ui: &ReaderWindow, inner: &Inner) -> bool {
        let Some(end) = inner.listen_end.get() else {
            return false;
        };
        let finished = inner.audio.borrow().finished();
        let Some((reciter, surah)) = finished.filter(|(_, surah)| *surah < end.0) else {
            return false;
        };
        let next = surah + 1;
        let last = if next == end.0 {
            end.1
        } else {
            inner
                .surahs
                .iter()
                .find(|s| s.id == next)
                .map_or(1, |s| s.verses_count)
        };
        let rate = f64::from(ui.get_playback_rate());
        let volume = f64::from(ui.get_volume()) / 100.;
        let result = inner
            .audio
            .borrow_mut()
            .play(reciter, (next, 1), Some(last), rate, volume);
        match result {
            Ok(()) => {
                *inner.last_audio.borrow_mut() = None;
                true
            }
            Err(error) => {
                inner.listen_end.set(None);
                ui.set_audio_status(error.to_string().into());
                false
            }
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
                        Self::surah_names(&ui, &inner);
                        Self::download_rows(&ui, &inner);
                        Self::rows(&ui, &state, &inner);
                        Self::shortcut_rows(&ui, &state);
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
            ui.on_listen_selected(move |scope| {
                if let Some(ui) = weak.upgrade() {
                    ui.set_listen_scope(scope.clamp(0, 4));
                    ui.invoke_play_selected();
                }
            });
        }
        {
            let state = state.clone();
            let weak = ui.as_weak();
            let inner = self.inner.clone();
            ui.on_shortcut_captured(move |index, combo| {
                let Some(ui) = weak.upgrade() else { return };
                let Some(action) = usize::try_from(index)
                    .ok()
                    .and_then(|i| shortcuts::ACTIONS.get(i))
                else {
                    return;
                };
                let arabic = ui.get_arabic();
                let clash = (!combo.is_empty())
                    .then(|| {
                        state
                            .borrow()
                            .shortcuts
                            .iter()
                            .position(|s| s.as_str() == combo.as_str())
                    })
                    .flatten()
                    .filter(|other| *other != index as usize);
                if let Some(other) = clash {
                    let other = &shortcuts::ACTIONS[other];
                    ui.set_shortcut_status(
                        if arabic {
                            format!("هذا الاختصار مستخدم في «{}».", other.arabic)
                        } else {
                            format!("Already used for {}.", other.english)
                        }
                        .into(),
                    );
                    return;
                }
                state.borrow_mut().shortcuts[index as usize] = combo.to_string();
                inner
                    .prefs
                    .borrow_mut()
                    .shortcuts
                    .insert(action.name.to_string(), combo.to_string());
                ui.set_shortcut_capture(-1);
                ui.set_shortcut_status("".into());
                Self::shortcut_rows(&ui, &state);
                ui.invoke_preferences_changed();
            });
        }
        {
            let state = state.clone();
            let weak = ui.as_weak();
            let inner = self.inner.clone();
            ui.on_shortcuts_reset(move || {
                let Some(ui) = weak.upgrade() else { return };
                {
                    let mut prefs = inner.prefs.borrow_mut();
                    prefs.shortcuts.clear();
                    prefs.search_shortcut = 0;
                    prefs.listen_shortcut = 0;
                    prefs.options_shortcut = 0;
                }
                ui.set_search_shortcut(0);
                ui.set_listen_shortcut(0);
                ui.set_options_shortcut(0);
                state.borrow_mut().shortcuts =
                    shortcuts::resolve(&std::collections::BTreeMap::new(), [0, 0, 0]);
                ui.set_shortcut_capture(-1);
                ui.set_shortcut_status("".into());
                Self::shortcut_rows(&ui, &state);
                ui.invoke_preferences_changed();
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
                    inner.listen_end.set(None);
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
            let inner = self.inner.clone();
            let state = state.clone();
            let weak = ui.as_weak();
            ui.on_downloads_refresh(move || {
                if let Some(ui) = weak.upgrade() {
                    if inner.audio.borrow().downloading().is_none() {
                        let s = state.borrow();
                        let key = s
                            .context
                            .map(|v| v.0)
                            .or(s.selection)
                            .or_else(|| s.probe.map(|p| p.2));
                        if let Some(key) = key {
                            ui.set_download_surah(key.0 as i32 - 1);
                        }
                        drop(s);
                        if !ui.get_download_busy() {
                            ui.set_download_status("".into());
                        }
                    }
                    Self::download_rows(&ui, &inner);
                }
            });
        }
        {
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            let timer = self.download_timer.clone();
            ui.on_download_start(move |index| {
                if let Some(ui) = weak.upgrade() {
                    let reciter = ui
                        .get_reciter_index()
                        .clamp(0, audio::RECITERS.len() as i32 - 1)
                        as usize;
                    let surah = (index.clamp(0, 113) + 1) as u32;
                    Self::begin_download(&ui, &inner, &timer, reciter, surah);
                }
            });
        }
        {
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            let timer = self.download_timer.clone();
            ui.on_download_current(move || {
                if let Some(ui) = weak.upgrade() {
                    let recited = inner.audio.borrow().current_surah();
                    if let Some((reciter, surah)) = recited {
                        ui.set_download_surah(surah as i32 - 1);
                        Self::begin_download(&ui, &inner, &timer, reciter, surah);
                    }
                }
            });
        }
        {
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            let timer = self.download_timer.clone();
            ui.on_download_cancel(move || {
                if let Some(ui) = weak.upgrade() {
                    inner.audio.borrow_mut().cancel_download();
                    timer.stop();
                    ui.set_download_busy(false);
                    ui.set_download_status(
                        if ui.get_arabic() {
                            "أُلغي التنزيل."
                        } else {
                            "Download cancelled."
                        }
                        .into(),
                    );
                    Self::download_rows(&ui, &inner);
                }
            });
        }
        {
            let inner = self.inner.clone();
            let weak = ui.as_weak();
            ui.on_download_delete(move |index| {
                if let Some(ui) = weak.upgrade() {
                    let item = inner.downloads.borrow().get(index.max(0) as usize).copied();
                    let Some(item) = item else {
                        return;
                    };
                    let arabic = ui.get_arabic();
                    let result = inner.audio.borrow_mut().delete_download(item);
                    match result {
                        Ok(()) => {
                            let title = Self::surah_title(&inner, item.surah, arabic);
                            ui.set_download_status(
                                if arabic {
                                    format!("حُذف تنزيل سورة {title}.")
                                } else {
                                    format!("Deleted the download of {title}.")
                                }
                                .into(),
                            );
                        }
                        Err(error) => {
                            ui.set_download_status(
                                if arabic {
                                    "تعذّر حذف التنزيل. أوقف التلاوة إن كانت تعمل ثم حاول مرة أخرى."
                                        .to_string()
                                } else {
                                    error.to_string()
                                }
                                .into(),
                            );
                        }
                    }
                    Self::download_rows(&ui, &inner);
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
