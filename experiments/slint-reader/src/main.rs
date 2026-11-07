#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![deny(unsafe_op_in_unsafe_fn)]
#[allow(dead_code)]
#[allow(clippy::collapsible_if)] // Shared production code retains Rust 1.85 syntax.
#[path = "../../../windows/QuranMushaf.Native/src/audio.rs"]
mod audio;
mod audio_bridge;
#[allow(dead_code)]
#[allow(clippy::collapsible_if)] // Shared production code retains Rust 1.85 syntax.
#[path = "../../../windows/QuranMushaf.Native/src/audio_cache.rs"]
mod audio_cache;
mod audio_qa;
mod controls;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/file_integrity.rs"]
mod file_integrity;
mod gpu_choice;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/http_security.rs"]
mod http_security;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/listening_layout.rs"]
mod listening_layout;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/mp3.rs"]
mod mp3;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/owned_file.rs"]
mod owned_file;
mod page_animation;
mod player_qa;
mod preferences;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/profile_file.rs"]
mod profile_file;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/range_stream.rs"]
mod range_stream;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/scoped_store.rs"]
mod scoped_store;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/timed_audio.rs"]
mod timed_audio;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/timed_store.rs"]
mod timed_store;
// These source files are shared verbatim. Slint receives pixels and source hit
// coordinates; it never shapes, normalizes or substitutes the Quran text.
#[allow(dead_code)]
// Keep the shared production source's Rust 1.85-compatible spelling intact.
#[allow(clippy::manual_is_multiple_of)]
#[path = "../../../windows/QuranMushaf.Native/src/app_core.rs"]
mod app_core;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/data.rs"]
mod data;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/outlines.rs"]
mod outlines;
mod page_cache;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/page_turn.rs"]
mod page_turn;
#[allow(dead_code)]
#[path = "../../../windows/QuranMushaf.Native/src/quran_shape.rs"]
mod quran_shape;
mod reader;
#[allow(dead_code)]
#[allow(clippy::collapsible_if)]
#[path = "../../../windows/QuranMushaf.Native/src/render.rs"]
mod render;
mod turn_queue;
pub(crate) use reader::bundled_only as tafsir_library;

use reader::{Engine, Rendered, SearchResult};
use serde_json::{Value, json};
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use slint::{ComponentHandle, Model, ModelRc, SharedPixelBuffer, VecModel};
use std::{
    cell::{Cell, RefCell},
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
slint::include_modules!();
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy, PartialEq)]
struct View {
    page: u32,
    width: f32,
    height: f32,
    scale: f32,
    dark: bool,
    arabic: bool,
    selection: Option<(u32, u32)>,
    style: reader::RenderStyle,
    clock_minute: u64,
}
#[derive(Clone)]
struct RenderRequest {
    motion: bool,
    navigation: bool,
    from_page: i32,
    generation: i32,
    view: View,
}
enum Task {
    Render(RenderRequest),
    Search(i32, String, bool, i32),
    Pick(i32, f32, f32, PickKind),
    Hover(i32, i32, f32, f32),
    Resolve(i32, (u32, u32), i32, bool, ResolveKind),
    Prefetch(i32, View),
}
#[derive(Clone, Copy)]
enum PickKind {
    Study,
    Menu { x: f32, y: f32 },
}
#[derive(Clone, Copy)]
enum ResolveKind {
    Study,
    Recitation,
}
struct Verse {
    key: (u32, u32),
    page: u32,
}
type PendingResolve = (i32, (u32, u32), i32, bool, ResolveKind);
#[derive(Default)]
struct Pending {
    render: Option<RenderRequest>,
    search: Option<(i32, String, bool, i32)>,
    pick: Option<(i32, f32, f32, PickKind)>,
    hover: Option<(i32, i32, f32, f32)>,
    resolve: Option<PendingResolve>,
    stop: bool,
    prefetch: std::collections::VecDeque<(i32, View)>,
}
#[derive(Default)]
struct Queue {
    pending: Mutex<Pending>,
    wake: Condvar,
}
impl Queue {
    fn push(&self, task: Task) {
        if let Ok(mut pending) = self.pending.lock() {
            match task {
                Task::Render(value) => pending.render = Some(value),
                Task::Search(id, value, arabic, mode) => {
                    pending.search = Some((id, value, arabic, mode))
                }
                Task::Pick(id, x, y, kind) => pending.pick = Some((id, x, y, kind)),
                Task::Hover(id, sequence, x, y) => pending.hover = Some((id, sequence, x, y)),
                Task::Resolve(id, key, step, arabic, kind) => {
                    pending.resolve = Some((id, key, step, arabic, kind))
                }
                Task::Prefetch(id, view) => {
                    if pending.prefetch.len() < 2 {
                        pending.prefetch.push_back((id, view));
                    }
                }
            }
            self.wake.notify_one();
        }
    }
    fn next(&self) -> Option<Task> {
        let mut p = self.pending.lock().ok()?;
        loop {
            if p.stop {
                return None;
            }
            if let Some((id, x, y, kind)) = p.pick.take() {
                return Some(Task::Pick(id, x, y, kind));
            }
            if let Some((id, key, step, arabic, kind)) = p.resolve.take() {
                return Some(Task::Resolve(id, key, step, arabic, kind));
            }
            if let Some((id, q, arabic, mode)) = p.search.take() {
                return Some(Task::Search(id, q, arabic, mode));
            }
            if let Some(r) = p.render.take() {
                return Some(Task::Render(r));
            }
            if let Some((id, sequence, x, y)) = p.hover.take() {
                return Some(Task::Hover(id, sequence, x, y));
            }
            if let Some((id, view)) = p.prefetch.pop_front() {
                return Some(Task::Prefetch(id, view));
            }
            p = self.wake.wait(p).ok()?;
        }
    }
    fn stop(&self) {
        if let Ok(mut p) = self.pending.lock() {
            p.stop = true;
            self.wake.notify_one();
        }
    }
    fn prepare_neighbors(&self, generation: i32, view: View, single: bool) {
        if let Ok(mut p) = self.pending.lock() {
            p.prefetch.clear();
            if view.selection.is_some() {
                return;
            }
            for direction in [-1, 1] {
                let page = app_core::destination(view.page, direction, single);
                if page != view.page {
                    p.prefetch.push_back((
                        generation,
                        View {
                            page,
                            selection: None,
                            ..view
                        },
                    ));
                }
            }
            self.wake.notify_one();
        }
    }
}
struct RenderOutput {
    generation: i32,
    page: u32,
    raster: Rendered,
    probe: Option<(f32, f32, (u32, u32))>,
    study: Option<(String, String)>,
    navigation: bool,
    prefetched: bool,
}
enum Output {
    Motion(i32, u32, f32, bool, page_animation::Surface, f64),
    Render(i32, std::result::Result<RenderOutput, String>),
    Search(i32, std::result::Result<Vec<SearchResult>, String>),
    Pick(i32, PickKind, std::result::Result<Option<Verse>, String>),
    Hover(i32, i32, Arc<reader::HoverResult>),
    Resolve(i32, ResolveKind, std::result::Result<Verse, String>),
    Fatal(String),
    Prefetched(i32, Vec<u32>, usize),
}
struct Worker {
    queue: Arc<Queue>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.queue.stop();
        // Delivery observes this stop signal even if an early UI error leaves
        // the receiver alive. COM is always released on the worker's thread.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn start_worker(root: PathBuf, ui: slint::Weak<ReaderWindow>) -> (Worker, mpsc::Receiver<Output>) {
    let queue = Arc::new(Queue::default());
    let work = queue.clone();
    // One waiting output and one coalesced render request bound retained pixels.
    let (tx, rx) = mpsc::sync_channel(1);
    let thread = thread::spawn(move || {
        let send = |mut output| {
            loop {
                match tx.try_send(output) {
                    Ok(()) => {
                        return ui
                            .upgrade_in_event_loop(|ui| ui.invoke_worker_ready())
                            .is_ok();
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => return false,
                    Err(mpsc::TrySendError::Full(waiting)) => {
                        output = waiting;
                        let Ok(pending) = work.pending.lock() else {
                            return false;
                        };
                        if pending.stop {
                            return false;
                        }
                        // One retained result remains bounded. Sleeping on the
                        // queue condition avoids polling CPU and lets stop()
                        // interrupt delivery on every success/error close path.
                        let Ok((pending, _)) =
                            work.wake.wait_timeout(pending, Duration::from_millis(25))
                        else {
                            return false;
                        };
                        if pending.stop {
                            return false;
                        }
                    }
                }
            }
        };
        let mut engine = match Engine::open(root) {
            Ok(engine) => engine,
            Err(error) => {
                send(Output::Fatal(error.to_string()));
                return;
            }
        };
        let mut native_generation = 0;
        let mut prepared = page_cache::Cache::<View, reader::Prepared>::default();
        let mut previous: Option<(View, page_animation::Surface)> = None;
        while let Some(task) = work.next() {
            let output = match task {
                Task::Render(request) => {
                    let v = request.view;
                    let cached = prepared.take(&v);
                    let prefetched = cached.is_some();
                    let result = match cached {
                        Some(page) => Ok(engine.activate(page)),
                        None => engine.render_styled(
                            v.page,
                            v.width,
                            v.height,
                            v.scale,
                            v.arabic,
                            v.selection,
                            v.style,
                        ),
                    }
                    .and_then(|raster| {
                        native_generation = request.generation;
                        let probe = engine.probe_hit();
                        let study = v
                            .selection
                            .map(|key| {
                                engine
                                    .study_source(key, v.style.study_source)
                                    .map(|text| (engine.reference_label(key, v.arabic), text))
                            })
                            .transpose()?;
                        Ok(RenderOutput {
                            generation: request.generation,
                            page: v.page,
                            raster,
                            probe,
                            study,
                            navigation: request.navigation,
                            prefetched,
                        })
                    })
                    .map_err(|error| error.to_string());
                    if let Ok(output) = &result {
                        let r = &output.raster;
                        work.prepare_neighbors(request.generation, v, r.single_page);
                        let next = page_animation::Surface {
                            pixels: Arc::new(r.rgba.clone()),
                            width: r.pixel_width,
                            height: r.pixel_height,
                            source: r.source,
                        };
                        if let Some((old_view, old)) = &previous {
                            let theme_turn =
                                old_view.page == v.page && old_view.style.theme != v.style.theme;
                            // Page turns are presented by Slint from the two retained
                            // native textures. Only theme recoloring still needs this
                            // bounded software compositor; never upload a whole
                            // newly composed spread for every page-turn frame.
                            if request.motion
                                && old_view.page as i32 == request.from_page
                                && old.compatible(&next)
                                && theme_turn
                                && old_view.width == v.width
                                && old_view.height == v.height
                                && old_view.scale == v.scale
                            {
                                let began = Instant::now();
                                let duration = 0.18;
                                loop {
                                    let pending = work.pending.lock().ok();
                                    if pending
                                        .as_ref()
                                        .is_none_or(|p| p.stop || p.render.is_some())
                                    {
                                        break;
                                    }
                                    drop(pending);
                                    let t = began.elapsed().as_secs_f32() / duration;
                                    if t >= 1. {
                                        break;
                                    }
                                    let p = page_animation::eased(t);
                                    let frame_start = Instant::now();
                                    let pixels = page_animation::blend(
                                        old,
                                        &next,
                                        p,
                                        old_view.style.theme,
                                        v.style.theme,
                                    );
                                    if !send(Output::Motion(
                                        request.generation,
                                        v.style.theme,
                                        p,
                                        theme_turn,
                                        page_animation::Surface {
                                            pixels: Arc::new(pixels),
                                            ..next.clone()
                                        },
                                        frame_start.elapsed().as_secs_f64() * 1000.,
                                    )) {
                                        return;
                                    }
                                    if let Ok(pending) = work.pending.lock() {
                                        let _ = work.wake.wait_timeout_while(
                                            pending,
                                            Duration::from_micros(16_667)
                                                .saturating_sub(frame_start.elapsed()),
                                            |p| !p.stop && p.render.is_none(),
                                        );
                                    }
                                }
                            }
                        }
                        previous = Some((v, next));
                    }
                    Output::Render(request.generation, result)
                }
                Task::Search(id, query, arabic, mode) => Output::Search(
                    id,
                    engine
                        .search_mode(&query, arabic, mode)
                        .map_err(|e| e.to_string()),
                ),
                Task::Prefetch(id, view) => {
                    if id != native_generation {
                        continue;
                    }
                    if !prepared.contains(&view) {
                        let original_hit = engine.probe_hit();
                        if let Ok(page) = engine.prepare_styled(
                            view.page,
                            view.width,
                            view.height,
                            view.scale,
                            view.arabic,
                            view.selection,
                            view.style,
                        ) {
                            let bytes = page.raster.rgba.len();
                            prepared.insert(view, page, bytes);
                        }
                        if engine.probe_hit() != original_hit {
                            send(Output::Fatal(
                                "Preparing an adjacent page changed the active hit map".into(),
                            ));
                            return;
                        }
                    }
                    Output::Prefetched(
                        id,
                        prepared
                            .keys()
                            .filter_map(|key| {
                                let mut comparable = *key;
                                comparable.page = view.page;
                                (comparable == view).then_some(key.page)
                            })
                            .collect(),
                        prepared.bytes(),
                    )
                }
                Task::Pick(id, x, y, kind) => {
                    let key = (id == native_generation)
                        .then(|| engine.hit(x, y))
                        .flatten();
                    let result = key
                        .map(|key| engine.page_for(key).map(|page| Verse { key, page }))
                        .transpose()
                        .map_err(|e| e.to_string());
                    Output::Pick(id, kind, result)
                }
                Task::Hover(id, sequence, x, y) => {
                    let rectangles = if id == native_generation {
                        engine.hover_result(x, y)
                    } else {
                        Arc::new(reader::HoverResult::default())
                    };
                    Output::Hover(id, sequence, rectangles)
                }
                Task::Resolve(id, key, step, arabic, kind) => {
                    let _ = arabic;
                    let result = engine
                        .step_ayah(key, step)
                        .and_then(|key| engine.page_for(key).map(|page| Verse { key, page }))
                        .map_err(|e| e.to_string());
                    Output::Resolve(id, kind, result)
                }
            };
            if !send(output) {
                break;
            }
        }
    });
    (
        Worker {
            queue,
            thread: Some(thread),
        },
        rx,
    )
}

#[derive(Default)]
struct State {
    prefetched_pages: Vec<u32>,
    prefetch_bytes: usize,
    prime_scheduled: Option<i32>,
    settle_scheduled: Option<i32>,
    drag_finish: Option<(bool, u32)>,
    book_turns: u64,
    primed_turns: u64,
    page: u32,
    selection: Option<(u32, u32)>,
    selection_page: u32,
    generation: i32,
    search_id: i32,
    search_completed_id: Option<i32>,
    requested: Option<View>,
    committed: i32,
    hover_sequence: i32,
    render_count: u64,
    hits: Vec<SearchResult>,
    probe: Option<(f32, f32, (u32, u32))>,
    last: Value,
    qa_pixels: Option<(Vec<u8>, u32, u32)>,
    qa: bool,
    motion_times: Vec<f64>,
    motion_progress: f32,
    turn_last_redraw: Option<Instant>,
    turns: turn_queue::TurnQueue,
    turn_redraw_intervals: Vec<f64>,
    turn_progress_samples: Vec<f32>,
    turn_monitor_millihertz: Option<u32>,
    reference_generation: i32,
    context: Option<((u32, u32), u32)>,
    saved_bookmarks: Vec<preferences::Bookmark>,
    recitation: Option<(u32, u32)>,
    opening: Option<u32>,
    surahs: Vec<data::Surah>,
}
fn hover_view_matches(ui: &ReaderWindow, state: &State) -> bool {
    state.generation == state.committed
        && state.requested.is_some_and(|view| {
            view.width == ui.get_canvas_w()
                && view.height == ui.get_canvas_h()
                && view.scale == ui.window().scale_factor()
        })
}
fn request_render(ui: &ReaderWindow, state: &Rc<RefCell<State>>, queue: &Queue) {
    request_render_with_intent(ui, state, queue, false);
}
fn request_render_with_intent(
    ui: &ReaderWindow,
    state: &Rc<RefCell<State>>,
    queue: &Queue,
    navigation: bool,
) {
    let width = ui.get_canvas_w();
    let height = ui.get_canvas_h();
    if width < 200. || height < 160. {
        return;
    }
    let mut state = state.borrow_mut();
    let view = View {
        page: state.page.max(1),
        width,
        height,
        scale: ui.window().scale_factor(),
        dark: ui.get_dark(),
        arabic: ui.get_arabic(),
        selection: state.selection,
        style: reader::RenderStyle {
            theme: ui.get_theme().clamp(0, 4) as u32,
            contextual: ui.get_contextual(),
            show_clock: ui.get_show_clock(),
            clock_24: ui.get_clock_24(),
            single_page: ui.get_single_page(),
            study_source: ui.get_study_source().clamp(0, 2) as u32,
            recitation: state.recitation,
            opening: state.opening,
        },
        clock_minute: if ui.get_show_clock() && !state.qa {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() / 60)
        } else {
            0
        },
    };
    if state.requested == Some(view) {
        return;
    }
    // Jumps, resizes and setting changes replace whatever was waiting to turn.
    if !navigation {
        state.turns.clear();
    }
    state.motion_times.clear();
    state.motion_progress = 0.;
    state.turn_last_redraw = None;
    state.turn_redraw_intervals.clear();
    state.turn_progress_samples.clear();
    state.prefetched_pages.clear();
    state.prefetch_bytes = 0;
    state.prime_scheduled = None;
    state.settle_scheduled = None;
    state.generation = state.generation.wrapping_add(1).max(1);
    state.hover_sequence = state.hover_sequence.wrapping_add(1);
    state.requested = Some(view);
    let generation = state.generation;
    drop(state);
    ui.set_hover_rects(ModelRc::new(VecModel::from(Vec::<HoverRect>::new())));
    ui.set_turn_running(false);
    ui.set_turn_priming(false);
    ui.set_previous_book(slint::Image::default());
    ui.set_next_book(slint::Image::default());
    ui.set_turn_blend(0.);
    ui.set_render_generation(generation);
    ui.set_render_busy(true);
    ui.set_render_status(
        if view.arabic {
            "جارٍ فتح الصفحة…"
        } else {
            "Opening page…"
        }
        .into(),
    );
    queue.push(Task::Render(RenderRequest {
        motion: ui.get_motion_enabled(),
        navigation,
        from_page: ui.get_rendered_page(),
        generation,
        view,
    }));
}
fn set_page(
    ui: &ReaderWindow,
    state: &Rc<RefCell<State>>,
    queue: &Queue,
    page: u32,
    selection: Option<(u32, u32)>,
) {
    set_page_with_intent(ui, state, queue, page, selection, false);
}
fn set_page_with_intent(
    ui: &ReaderWindow,
    state: &Rc<RefCell<State>>,
    queue: &Queue,
    page: u32,
    selection: Option<(u32, u32)>,
    navigation: bool,
) {
    let visible_page = {
        let s = state.borrow();
        let single = s.last["singlePage"]
            .as_bool()
            .unwrap_or(ui.get_single_page());
        if !single && app_core::spread(s.page) == app_core::spread(page) {
            s.page
        } else {
            page
        }
    };
    {
        let mut s = state.borrow_mut();
        s.page = visible_page;
        if s.selection != selection {
            s.selection_page = if selection.is_some() { page } else { 0 };
        }
        s.selection = selection;
        s.context = None;
        s.reference_generation = s.reference_generation.wrapping_add(1).max(1);
    }
    ui.set_page(visible_page as i32);
    ui.set_page_input(visible_page.to_string().into());
    if selection.is_none() {
        ui.set_study_body("".into());
        if ui.get_panel() == "study" {
            ui.set_panel("".into());
        }
    }
    request_render_with_intent(ui, state, queue, navigation);
    ui.invoke_preferences_changed();
}
fn page_number(value: &str) -> Option<u32> {
    if value.chars().count() > 12 {
        return None;
    }
    let ascii: String = value
        .trim()
        .chars()
        .map(|c| match c {
            '\u{0660}'..='\u{0669}' => char::from_u32('0' as u32 + c as u32 - 0x0660).unwrap(),
            '\u{06f0}'..='\u{06f9}' => char::from_u32('0' as u32 + c as u32 - 0x06f0).unwrap(),
            _ => c,
        })
        .collect();
    ascii.parse::<u32>().ok().filter(|n| (1..=604).contains(n))
}
fn motion_preference() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, value: *mut i32, flags: u32) -> i32;
    }
    let mut enabled = 0;
    unsafe { SystemParametersInfoW(0x1042, 0, &mut enabled, 0) == 0 || enabled != 0 }
}

fn resource_root() -> Result<PathBuf> {
    let args: Vec<_> = std::env::args_os().collect();
    if let Some(index) = args.iter().position(|a| a == "--resources") {
        return Ok(
            PathBuf::from(args.get(index + 1).ok_or("Missing resources path")?).canonicalize()?,
        );
    }
    let local = std::env::current_exe()?
        .parent()
        .ok_or("Executable parent missing")?
        .join("Resources");
    if local.is_dir() {
        return Ok(local.canonicalize()?);
    }
    // Developer fallback reads only the already verified native package. Release
    // builds leave it out, so the executable carries no build-machine path.
    #[cfg(debug_assertions)]
    {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../outputs/native-preview-v0.2.29/Resources")
            .canonicalize()?)
    }
    #[cfg(not(debug_assertions))]
    {
        Err("The Resources folder was not found beside the application. Keep it next to the .exe, or pass --resources <folder>.".into())
    }
}
fn png(path: &Path, bytes: &[u8], width: u32, height: u32) -> Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(bytes)?;
    writer.finish()?;
    Ok(())
}
fn json_file(path: &Path, value: &Value) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn next_queued_turn(ui: &ReaderWindow, state: &Rc<RefCell<State>>) {
    if ui.get_render_busy() || ui.get_drag_active() {
        return;
    }
    let next = state.borrow_mut().turns.pop();
    if let Some(delta) = next {
        if ui.get_panel().is_empty() && !ui.get_context_open() {
            ui.invoke_navigate(delta);
        } else {
            state.borrow_mut().turns.clear();
        }
    }
}

fn settle_turn(ui: &ReaderWindow, state: &Rc<RefCell<State>>, queue: &Arc<Queue>) {
    if ui.get_turn_priming() || ui.get_drag_active() || !ui.get_turn_running() {
        return;
    }
    let mut s = state.borrow_mut();
    let generation = s.generation;
    if s.settle_scheduled == Some(generation) {
        return;
    }
    s.settle_scheduled = Some(generation);
    let (commit, original) = s.drag_finish.take().unwrap_or((true, 0));
    drop(s);
    let weak = ui.as_weak();
    let state = state.clone();
    let queue = queue.clone();
    slint::Timer::single_shot(
        Duration::from_millis(if commit {
            ui.get_turn_duration().max(0) as u64 + 10
        } else {
            190
        }),
        move || {
            if let Some(ui) = weak.upgrade()
                && state.borrow().generation == generation
                && ui.get_turn_running()
            {
                ui.set_book_image(if commit {
                    ui.get_next_book()
                } else {
                    ui.get_previous_book()
                });
                ui.set_rendered_page(if commit {
                    ui.get_turn_target_page()
                } else {
                    original as i32
                });
                ui.set_turn_running(false);
                ui.set_turn_priming(false);
                ui.set_drag_canceling(false);
                ui.set_previous_book(slint::Image::default());
                ui.set_next_book(slint::Image::default());
                ui.set_render_busy(false);
                if commit {
                    next_queued_turn(&ui, &state);
                } else {
                    set_page(&ui, &state, &queue, original, None);
                }
            }
        },
    );
}

/// Make the page raster transparent outside its rounded paper frame, so the
/// sheets drawn behind the book show through without clipping every frame.
/// Pixels inside the frame keep their exact native values; only the canvas
/// margin and the anti-aliased rounded edge receive alpha.
fn clear_outside_frame(
    buffer: &mut SharedPixelBuffer<slint::Rgba8Pixel>,
    source_w: f32,
    source_h: f32,
    frame: [f32; 4],
) {
    let (width, height) = (buffer.width() as usize, buffer.height() as usize);
    if width == 0
        || height == 0
        || source_w <= 0.
        || source_h <= 0.
        || frame[2] <= 0.
        || frame[3] <= 0.
    {
        return;
    }
    let (sx, sy) = (width as f32 / source_w, height as f32 / source_h);
    // Frame edges and corner radius in pixels.
    let (left, top) = (frame[0] * sx, frame[1] * sy);
    let (right, bottom) = (left + frame[2] * sx, top + frame[3] * sy);
    let radius = 12. * sx.min(sy);
    let pixels = buffer.make_mut_slice();
    for row in 0..height {
        let y = row as f32 + 0.5;
        let line = &mut pixels[row * width..(row + 1) * width];
        let vertical = (y - top + 0.5)
            .clamp(0., 1.)
            .min((bottom - y + 0.5).clamp(0., 1.));
        if vertical <= 0. {
            line.iter_mut()
                .for_each(|p| *p = slint::Rgba8Pixel::new(0, 0, 0, 0));
            continue;
        }
        // Horizontal inset of the rounded corner on this row.
        let corner = if y < top + radius {
            top + radius - y
        } else if y > bottom - radius {
            y - (bottom - radius)
        } else {
            0.
        };
        let inset = if corner > 0. {
            radius
                - (radius * radius - corner.min(radius).powi(2))
                    .max(0.)
                    .sqrt()
        } else {
            0.
        };
        let (from, to) = (left + inset, right - inset);
        for (col, p) in line.iter_mut().enumerate() {
            let x = col as f32 + 0.5;
            let horizontal = (x - from + 0.5)
                .clamp(0., 1.)
                .min((to - x + 0.5).clamp(0., 1.));
            let coverage = horizontal * vertical;
            if coverage < 1. {
                p.a = (p.a as f32 * coverage).round() as u8;
                if p.a == 0 {
                    *p = slint::Rgba8Pixel::new(0, 0, 0, 0);
                }
            }
        }
    }
}

fn apply_output(ui: &ReaderWindow, state: &Rc<RefCell<State>>, queue: &Queue, output: Output) {
    match output {
        Output::Prefetched(id, pages, bytes) => {
            let mut s = state.borrow_mut();
            if id == s.generation {
                s.prefetched_pages = pages;
                s.prefetch_bytes = bytes;
            }
        }
        Output::Motion(id, theme, phase, theme_turn, frame, compute_ms) => {
            if id == state.borrow().generation {
                let mut pixels = SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                    &frame.pixels,
                    frame.width,
                    frame.height,
                );
                clear_outside_frame(
                    &mut pixels,
                    ui.get_image_w(),
                    ui.get_image_h(),
                    [
                        ui.get_book_x(),
                        ui.get_book_y(),
                        ui.get_book_w(),
                        ui.get_book_h(),
                    ],
                );
                ui.set_presented_theme(theme as i32);
                ui.set_theme_phase(if theme_turn { phase } else { 1. });
                let mut s = state.borrow_mut();
                s.motion_progress = phase;
                if s.motion_times.len() < 128 {
                    s.motion_times.push(compute_ms);
                }
                drop(s);
                ui.set_previous_book(slint::Image::from_rgba8(pixels));
                ui.set_turn_blend(1.);
            }
        }
        Output::Render(id, Ok(output)) => {
            let mut s = state.borrow_mut();
            if id != s.generation {
                return;
            }
            let r = output.raster;
            let old_book = ui.get_book_image();
            let old_page = ui.get_rendered_page();
            let native_turn = output.navigation
                && output.study.is_none()
                && ui.get_panel().is_empty()
                && !ui.get_study_closing()
                && ui.get_motion_enabled()
                && old_page != output.page as i32
                && old_book.size().width == r.pixel_width
                && old_book.size().height == r.pixel_height
                && ui.get_image_x() == r.source.x
                && ui.get_image_y() == r.source.y
                && ui.get_image_w() == r.source.w
                && ui.get_image_h() == r.source.h
                && s.last["singlePage"].as_bool() == Some(r.single_page)
                && s.requested
                    .is_some_and(|view| ui.get_presented_theme() == view.style.theme as i32)
                && r.rgba.len().saturating_mul(3) <= 48 * 1024 * 1024;
            let mut buffer = SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                &r.rgba,
                r.pixel_width,
                r.pixel_height,
            );
            clear_outside_frame(
                &mut buffer,
                r.source.w,
                r.source.h,
                [
                    r.book_bounds.x - r.source.x,
                    r.book_bounds.y - r.source.y,
                    r.book_bounds.w,
                    r.book_bounds.h,
                ],
            );
            let presented_theme = s.requested.map_or(ui.get_theme(), |v| v.style.theme as i32);
            if native_turn {
                s.book_turns += 1;
                s.turn_monitor_millihertz = ui
                    .window()
                    .with_winit_window(|window| {
                        window
                            .current_monitor()
                            .and_then(|monitor| monitor.refresh_rate_millihertz())
                    })
                    .flatten();
            }
            s.committed = output.generation;
            s.render_count += 1;
            s.probe = output.probe;
            s.last = json!({"generation":output.generation,"page":output.page,"nativePreparationMs":r.elapsed_ms,"pixelWidth":r.pixel_width,"pixelHeight":r.pixel_height,"transferBytes":r.rgba.len(),"source":[r.source.x,r.source.y,r.source.w,r.source.h],"bookBounds":[r.book_bounds.x,r.book_bounds.y,r.book_bounds.w,r.book_bounds.h],"singlePage":r.single_page,"interactiveLines":r.interactive_lines,"pixelFingerprintNotAuthenticityHash":format!("{:016x}",r.rgba_fingerprint),"probe":output.probe});
            s.last["prefetched"] = json!(output.prefetched);
            if s.qa {
                s.qa_pixels = Some((r.rgba.clone(), r.pixel_width, r.pixel_height));
            }
            // UI property change callbacks may open a panel and clear hover,
            // both of which access State. Never hold its borrow across them.
            drop(s);
            ui.set_previous_book(if native_turn {
                old_book
            } else {
                slint::Image::default()
            });
            ui.set_turn_blend(0.);
            ui.set_presented_theme(presented_theme);
            ui.set_theme_phase(1.);
            let next_book = slint::Image::from_rgba8(buffer);
            if native_turn {
                ui.set_turn_priming(true);
                ui.set_next_book(next_book);
                ui.set_turn_target_page(output.page as i32);
            } else {
                ui.set_book_image(next_book);
                ui.set_next_book(slint::Image::default());
            }
            ui.set_image_x(r.source.x);
            ui.set_image_y(r.source.y);
            ui.set_image_w(r.source.w);
            ui.set_image_h(r.source.h);
            ui.set_book_x(r.book_bounds.x - r.source.x);
            ui.set_book_y(r.book_bounds.y - r.source.y);
            ui.set_book_w(r.book_bounds.w);
            ui.set_book_h(r.book_bounds.h);
            if !native_turn {
                ui.set_rendered_page(output.page as i32);
            }
            ui.set_render_busy(native_turn);
            if native_turn {
                ui.set_turn_single(r.single_page);
                ui.set_turn_direction(if output.page as i32 > old_page { 1 } else { -1 });
                // Animated bindings are lazy. Their initial zero must be
                // evaluated before the consuming turn subtree is created;
                // otherwise its first read can start from an old dirty tick
                // and immediately resolve to the final value.
                let _ = ui.get_turn_progress();
                ui.set_turn_running(true);
            }
            ui.set_render_status(
                if ui.get_arabic() {
                    "اختر آية لقراءة تفسيرها"
                } else {
                    "Select a verse to read its translation"
                }
                .into(),
            );
            let (right, left) = data::spread(output.page);
            ui.set_label(
                if r.single_page && ui.get_arabic() {
                    format!("الصفحة {} من 604", output.page)
                } else if r.single_page {
                    format!("Page {} of 604", output.page)
                } else if ui.get_arabic() {
                    format!("الصفحات {}–{} من 604", right, left.unwrap_or(right))
                } else {
                    format!("Pages {}–{} of 604", right, left.unwrap_or(right))
                }
                .into(),
            );
            if let Some((title, body)) = output.study {
                ui.set_study_title(title.into());
                ui.set_study_body(body.into());
            }
            if !native_turn {
                next_queued_turn(ui, state);
            }
        }
        Output::Render(id, Err(error)) => {
            let mut s = state.borrow_mut();
            if id != s.generation {
                return;
            }
            s.requested = None;
            s.turns.clear();
            drop(s);
            ui.set_render_busy(false);
            ui.set_render_status(format!("Reader prototype: {error}").into());
        }
        Output::Fatal(error) => {
            ui.set_render_busy(false);
            ui.set_render_status(format!("Reader prototype: {error}").into());
        }
        Output::Search(id, result) => {
            let mut s = state.borrow_mut();
            if id != s.search_id {
                return;
            }
            s.search_completed_id = Some(id);
            ui.set_search_busy(false);
            match result {
                Ok(hits) => {
                    let rows: Vec<_> = hits
                        .iter()
                        .map(|hit| SearchEntry {
                            badge: hit.badge.clone().into(),
                            title: hit.title.clone().into(),
                            secondary: hit.secondary.clone().into(),
                            detail: hit.text.clone().into(),
                        })
                        .collect();
                    s.hits = hits;
                    ui.set_results(ModelRc::new(VecModel::from(rows)));
                    ui.set_search_status(
                        if s.hits.is_empty() {
                            if ui.get_arabic() {
                                "لا توجد نتائج مطابقة."
                            } else {
                                "No matching results."
                            }
                        } else {
                            ""
                        }
                        .into(),
                    );
                }
                Err(error) => {
                    s.hits.clear();
                    ui.set_results(ModelRc::new(VecModel::from(Vec::<SearchEntry>::new())));
                    ui.set_search_status(
                        if ui.get_arabic() {
                            "تعذّر البحث. حاول مرة أخرى."
                        } else {
                            "Search failed. Please try again."
                        }
                        .into(),
                    );
                    ui.set_render_status(error.into());
                }
            }
        }
        Output::Pick(id, kind, result) => {
            if id != state.borrow().committed || ui.get_render_busy() {
                return;
            }
            match result {
                Ok(Some(verse)) => match kind {
                    PickKind::Study => {
                        ui.set_follow_recitation(false);
                        state.borrow_mut().selection = Some(verse.key);
                        state.borrow_mut().selection_page = verse.page;
                        ui.set_panel("study".into());
                        request_render(ui, state, queue);
                    }
                    PickKind::Menu { x, y } => {
                        let mut s = state.borrow_mut();
                        s.context = Some((verse.key, verse.page));
                        ui.set_context_title(
                            controls::reference_title(&s.surahs, verse.key, ui.get_arabic()).into(),
                        );
                        ui.set_bookmark_present(
                            s.saved_bookmarks.iter().any(|b| b.key() == verse.key),
                        );
                        ui.set_context_x(x);
                        ui.set_context_y(y);
                        // Opening the menu clears hover through a UI callback;
                        // release the state borrow before that callback runs.
                        drop(s);
                        ui.set_context_open(true);
                    }
                },
                Ok(None) => {}
                Err(error) => ui.set_render_status(error.into()),
            }
        }
        Output::Hover(id, sequence, rectangles) => {
            let s = state.borrow();
            if id != s.committed
                || sequence != s.hover_sequence
                || !hover_view_matches(ui, &s)
                || ui.get_render_busy()
                || ui.get_context_open()
            {
                return;
            }
            drop(s);
            let rows = rectangles
                .regions
                .iter()
                .map(|r| HoverRect {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                })
                .collect::<Vec<_>>();
            let existing = ui.get_hover_rects();
            if existing.row_count() == rows.len()
                && rows.iter().enumerate().all(|(index, row)| {
                    existing.row_data(index).is_some_and(|current| {
                        current.x == row.x
                            && current.y == row.y
                            && current.w == row.w
                            && current.h == row.h
                    })
                })
            {
                return;
            }
            ui.set_hover_rects(ModelRc::new(VecModel::from(rows)));
            let patches = rectangles
                .patches
                .iter()
                .map(|p| HoverPatch {
                    x: p.rect.x,
                    y: p.rect.y,
                    w: p.rect.w,
                    h: p.rect.h,
                    image: slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                        &p.pixels, p.width, p.height,
                    )),
                })
                .collect::<Vec<_>>();
            ui.set_hover_patches(ModelRc::new(VecModel::from(patches)));
        }
        Output::Resolve(id, kind, result) => {
            if id != state.borrow().reference_generation {
                return;
            }
            match result {
                Ok(verse) => {
                    let selection = match kind {
                        ResolveKind::Study => Some(verse.key),
                        ResolveKind::Recitation => state.borrow().selection,
                    };
                    if matches!(kind, ResolveKind::Study) {
                        ui.set_panel("study".into());
                    }
                    set_page(ui, state, queue, verse.page, selection);
                }
                Err(error) => ui.set_render_status(error.into()),
            }
        }
    }
}

fn image_geometry(ui: &ReaderWindow) -> [f32; 8] {
    [
        ui.get_image_x(),
        ui.get_image_y(),
        ui.get_image_w(),
        ui.get_image_h(),
        ui.get_canvas_x(),
        ui.get_canvas_y(),
        ui.get_canvas_w(),
        ui.get_canvas_h(),
    ]
}

// Route an unpressed pointer through Slint's actual hit testing and TouchArea.
// Calling hover-moved directly cannot detect a broken pointer-event hookup.
fn qa_hover_pointer(ui: &ReaderWindow, x: f32, y: f32) {
    ui.set_pointer_hover_enabled(true);
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(
                ui.get_canvas_x() + ui.get_image_x() + x,
                ui.get_canvas_y() + ui.get_image_y() + y,
            ),
        });
    ui.set_pointer_hover_enabled(false);
}

fn qa_leave_pointer(ui: &ReaderWindow) {
    ui.set_pointer_hover_enabled(true);
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerExited);
}

fn qa_corner_pointer(ui: &ReaderWindow, progress: f32, action: u8) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(
        ui.get_canvas_x()
            + ui.get_image_x()
            + 6.
            + progress * ui.get_image_w() * if ui.get_turn_single() { 2. } else { 1. },
        ui.get_canvas_y() + ui.get_image_y() + 12.,
    );
    ui.window().dispatch_event(match action {
        0 => WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        1 => WindowEvent::PointerMoved { position },
        _ => WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    });
}

const MAXIMIZED_P95_BUDGET_MS: f64 = 25.;

/// How far a turn has travelled this long after its first visible movement.
fn progress_after(samples: &[f32], intervals: &[f64], after_ms: f64) -> f32 {
    let mut elapsed = 0.;
    for (index, interval) in intervals.iter().enumerate() {
        if samples.get(index).is_some_and(|progress| *progress > 0.) {
            elapsed += interval;
        }
        if elapsed >= after_ms {
            return samples.get(index + 1).copied().unwrap_or(0.);
        }
    }
    0.
}

// Exercise the actual Slint texture clips against the independent native
// polygon oracle. Flat diagnostic faces expose premature or missing reveals.
fn qa_fold_faces(ui: &ReaderWindow, output: &Path) -> Result<()> {
    let saved = (
        ui.get_book_image(),
        ui.get_previous_book(),
        ui.get_next_book(),
        ui.get_turn_single(),
        ui.get_turn_direction(),
        ui.get_turn_top(),
    );
    let old = slint::Image::from_rgba8(SharedPixelBuffer::clone_from_slice(
        &[180u8, 40, 40, 255],
        1,
        1,
    ));
    let new = slint::Image::from_rgba8(SharedPixelBuffer::clone_from_slice(
        &[40u8, 180, 80, 255],
        1,
        1,
    ));
    ui.set_fold_shading(false);
    ui.set_book_image(old.clone());
    ui.set_previous_book(old);
    ui.set_next_book(new);
    let w = ui.get_image_w();
    let h = ui.get_image_h();
    let scale = ui.window().scale_factor();
    let ox = ui.get_canvas_x() + ui.get_image_x();
    let oy = ui.get_canvas_y() + ui.get_image_y();
    let contains = |poly: &[(f32, f32)], x: f32, y: f32| -> (bool, f32) {
        let mut positive = false;
        let mut negative = false;
        let mut edge = f32::MAX;
        for (a, b) in poly
            .iter()
            .zip(poly.iter().cycle().skip(1))
            .take(poly.len())
        {
            let dx = b.0 - a.0;
            let dy = b.1 - a.1;
            let cross = dx * (y - a.1) - dy * (x - a.0);
            positive |= cross > 0.01;
            negative |= cross < -0.01;
            edge = edge.min(cross.abs() / dx.hypot(dy).max(0.001));
        }
        (poly.len() >= 3 && !(positive && negative), edge)
    };
    let mut samples = 0;
    for single in [false, true] {
        for direction in [-1, 1] {
            for top in [false, true] {
                ui.set_turn_single(single);
                ui.set_turn_direction(direction);
                ui.set_turn_top(top);
                let leaf = app_core::Rect {
                    x: if single || direction > 0 { 0. } else { w / 2. },
                    y: 0.,
                    w: if single { w } else { w / 2. },
                    h,
                };
                for progress in [0.25, 0.5, 0.75, 1.] {
                    ui.set_drag_progress(progress);
                    let scene = page_animation::fold_scene(w, h, single, direction, progress, top);
                    if (ui.get_fold_plane_x() - scene.plane[0]).abs() > 0.001 {
                        return Err("Fold geometry lagged behind the current pointer frame".into());
                    }
                    let paper = page_turn::paper(leaf, direction, progress, top);
                    let pixels = ui.window().take_snapshot()?;
                    if progress == 0.5 {
                        png(
                            &output.join(format!("fold-oracle-{single}-{direction}-{top}.png")),
                            pixels.as_bytes(),
                            pixels.width(),
                            pixels.height(),
                        )?;
                    }
                    let mut mismatches = 0;
                    for row in 1..20 {
                        for col in 1..24 {
                            let x = col as f32 * w / 24.;
                            let y = row as f32 * h / 20.;
                            let (retained, retained_edge) = contains(&paper.retained, x, y);
                            let (reverse, reverse_edge) = contains(&paper.reverse, x, y);
                            let angle = scene.plane[2].to_radians();
                            let crease = ((x - scene.plane[0]) * (-angle.sin())
                                + (y - scene.plane[1]) * angle.cos())
                            .abs();
                            if crease < 20.
                                || retained_edge < 3.
                                || reverse_edge < 3.
                                || (x - w / 2.).abs() < 3.
                            {
                                continue;
                            }
                            let lifted = x > leaf.x && x < leaf.x + leaf.w && !retained;
                            let expected = if reverse || lifted {
                                [40u8, 180, 80]
                            } else {
                                [180u8, 40, 40]
                            };
                            let px = ((ox + x) * scale).floor() as u32;
                            let py = ((oy + y) * scale).floor() as u32;
                            let i = ((py * pixels.width() + px) * 4) as usize;
                            samples += 1;
                            if (0..3).any(|c| pixels.as_bytes()[i + c].abs_diff(expected[c]) > 8) {
                                mismatches += 1;
                            }
                        }
                    }
                    if mismatches > 0 {
                        return Err(format!("Fold texture reveal disagrees with native polygons: single={single}, direction={direction}, top={top}, progress={progress}, mismatches={mismatches}").into());
                    }
                }
            }
        }
    }
    ui.set_book_image(saved.0);
    ui.set_previous_book(saved.1);
    ui.set_next_book(saved.2);
    ui.set_turn_single(saved.3);
    ui.set_turn_direction(saved.4);
    ui.set_turn_top(saved.5);
    ui.set_drag_progress(0.);
    ui.set_fold_shading(true);
    json_file(
        &output.join("fold-faces-qa.json"),
        &json!({"passed":true,"samples":samples,"frames":32,"nativePolygonOracle":true,"bothDirections":true,"topAndBottom":true,"singleAndSpread":true}),
    )?;
    Ok(())
}

fn qa_caption_click(ui: &ReaderWindow, from_right: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(
        ui.window().size().width as f32 / ui.window().scale_factor() - from_right,
        14.,
    );
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn qa_named_snapshot(ui: &ReaderWindow, output: &Path, name: &str) -> Result<()> {
    let pixels = ui.window().take_snapshot()?;
    png(
        &output.join(format!("{name}.png")),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
    )
}

fn qa_search_key(ui: &ReaderWindow, key: slint::platform::Key) {
    use slint::platform::WindowEvent;
    let text: slint::SharedString = key.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn qa_chrome_geometry(ui: &ReaderWindow) -> Result<Value> {
    let width = ui.window().size().width as f32 / ui.window().scale_factor();
    let height = ui.get_titlebar_height();
    let brand_x = ui.get_brand_x();
    let brand_w = ui.get_brand_w();
    let nav_x = ui.get_nav_x();
    let nav_w = ui.get_nav_w();
    let brand_end = brand_x + brand_w;
    let nav_end = nav_x + nav_w;
    let caption_start = width - 132.;
    if (height - 28.).abs() > 0.5
        || !(4.0..=12.0).contains(&brand_x)
        || !(100.0..=200.0).contains(&brand_w)
        || ui.get_nav_count() != 3
        || nav_x < brand_end + 3.
        || nav_end > caption_start + 1.
        || !(220.0..=400.0).contains(&nav_w)
    {
        return Err(format!(
            "Title-bar controls overlap or escaped their bounds: height={height}, brand=({brand_x},{brand_w}), nav=({nav_x},{nav_w}), width={width}"
        ).into());
    }
    if (!ui.get_arabic() && nav_x - brand_end > 24.)
        || (ui.get_arabic() && caption_start - nav_end > 24.)
    {
        return Err("Title-bar navigation did not follow the interface language".into());
    }
    Ok(json!({
        "arabic":ui.get_arabic(),"titlebarHeight":height,
        "brandX":brand_x,"brandWidth":brand_w,"navigationX":nav_x,
        "navigationWidth":nav_w,"navigationCount":ui.get_nav_count(),
        "brandOnLeft":true,"nonoverlapping":true,
        "navigationSide":if ui.get_arabic() {"right"} else {"left"}
    }))
}

struct Qa {
    turns_before_study: u64,
    output: PathBuf,
    began: Instant,
    index: usize,
    phase: u32,
    since: Instant,
    records: Vec<Value>,
    chrome_records: Vec<Value>,
    unchanged: (i32, u64, [f32; 8]),
    expected_selection: Option<(u32, u32)>,
    expected_search_id: i32,
    before_drag: Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>>,
    software_rendering: bool,
    maximized_frames: Vec<f64>,
    maximized_turns: u32,
    maximized_start_page: u32,
    fast_start_progress: f32,
    shaded_turns: u32,
    done: bool,
}
impl Qa {
    fn step(
        &mut self,
        ui: &ReaderWindow,
        state: &Rc<RefCell<State>>,
        queue: &Queue,
    ) -> Result<bool> {
        if self.began.elapsed() > Duration::from_secs(120) {
            return Err(format!("Reading prototype QA exceeded 120 seconds at index {} phase {}; probe={:?}; panel={}; hoverRects={}", self.index, self.phase, state.borrow().probe, ui.get_panel(), ui.get_hover_rects().row_count()).into());
        }
        if self.index == 8 {
            match self.phase {
                0 => {
                    ui.set_panel("".into());
                    ui.set_single_page(false);
                    ui.set_contextual(false);
                    ui.set_motion_enabled(true);
                    ui.window().set_size(slint::LogicalSize::new(780., 540.));
                    set_page(ui, state, queue, 319, None);
                    self.phase = 100;
                    self.since = Instant::now();
                }
                100 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(400) =>
                {
                    ui.invoke_navigate(1);
                    self.phase = 101;
                    self.since = Instant::now();
                }
                101 if state.borrow().motion_progress > 0.15 && ui.get_render_busy() => {
                    let pixels = ui.window().take_snapshot()?;
                    png(
                        &self.output.join("page-fold.png"),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                    )?;
                    self.phase = 102;
                }
                101 | 102
                    if !ui.get_render_busy()
                        && self.since.elapsed() > Duration::from_millis(400) =>
                {
                    let s = state.borrow();
                    let expected_page = app_core::destination(
                        319,
                        1,
                        s.last["singlePage"].as_bool().unwrap_or(false),
                    );
                    let intermediates = s
                        .turn_progress_samples
                        .iter()
                        .filter(|p| **p > 0.01 && **p < 0.99)
                        .count();
                    if s.turn_redraw_intervals.len() < 3
                        || intermediates < 3
                        || ui.get_rendered_page() as u32 != expected_page
                    {
                        return Err(format!("Native page turn failed: landed={}, expected={expected_page}, intermediateRedraws={intermediates}, redrawIntervalsMs={:?}, progressSamples={:?}, turnRunning={}, monitorMillihertz={:?}", ui.get_rendered_page(), s.turn_redraw_intervals, s.turn_progress_samples, ui.get_turn_running(), s.turn_monitor_millihertz).into());
                    }
                    drop(s);
                    json_file(
                        &self.output.join("motion-qa.json"),
                        &json!({"passed":true,"applicationRedraws":state.borrow().turn_redraw_intervals.len()+1,"redrawIntervalsMs":state.borrow().turn_redraw_intervals,"progressSamples":state.borrow().turn_progress_samples,"monitorRefreshMillihertz":state.borrow().turn_monitor_millihertz,"perFrameWorkerCompositions":0,"retainedNativeTextures":2,"clock":"Slint declarative animation, monitor-aware Winit redraw throttle","scope":"Application redraw requests, not measured monitor presentation FPS"}),
                    )?;
                    self.maximized_start_page = state.borrow().page;
                    ui.window()
                        .with_winit_window(|window| window.set_maximized(true));
                    self.phase = 9301;
                    self.since = Instant::now();
                }
                // Motion at reading size, with nothing else interrupting the
                // frames. This is the case a small window cannot reveal.
                9301 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(900) =>
                {
                    self.maximized_frames.clear();
                    self.maximized_turns = 0;
                    ui.invoke_navigate(1);
                    self.phase = 9302;
                    self.since = Instant::now();
                }
                9302 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(100) =>
                {
                    let s = state.borrow();
                    self.maximized_frames
                        .extend(s.turn_redraw_intervals.iter().skip(1));
                    if self.maximized_turns == 0 {
                        self.fast_start_progress =
                            progress_after(&s.turn_progress_samples, &s.turn_redraw_intervals, 40.);
                    }
                    drop(s);
                    self.maximized_turns += 1;
                    if self.maximized_turns < 8 {
                        ui.invoke_navigate(1);
                        self.since = Instant::now();
                    } else {
                        let mut frames = self.maximized_frames.clone();
                        frames.sort_by(f64::total_cmp);
                        let at = |share: f64| {
                            frames
                                .get(
                                    ((frames.len() as f64 * share) as usize)
                                        .min(frames.len().saturating_sub(1)),
                                )
                                .copied()
                                .unwrap_or(f64::MAX)
                        };
                        let (median, p95) = (at(0.5), at(0.95));
                        let size = ui.window().size();
                        let passed = frames.len() >= 16
                            && p95 <= MAXIMIZED_P95_BUDGET_MS
                            && self.fast_start_progress >= 0.2;
                        json_file(
                            &self.output.join("maximized-motion-qa.json"),
                            &json!({"passed":passed,"enforced":!self.software_rendering,"renderer":if self.software_rendering {"skia-software"} else {"skia"},"window":[size.width,size.height],"turns":self.maximized_turns,"frames":frames.len(),"medianFrameMs":median,"p95FrameMs":p95,"p95BudgetMs":MAXIMIZED_P95_BUDGET_MS,"progressAfter40Ms":self.fast_start_progress,"monitorRefreshMillihertz":state.borrow().turn_monitor_millihertz,"scope":"Application redraw requests, not measured monitor presentation FPS"}),
                        )?;
                        if !passed && !self.software_rendering {
                            return Err(format!("Maximized page turn missed its budget: frames={}, medianMs={median}, p95Ms={p95}, progressAfter40Ms={}", frames.len(), self.fast_start_progress).into());
                        }
                        self.phase = 9303;
                        self.since = Instant::now();
                    }
                }
                // Held mid-turn in both directions, so the paper shading can be inspected.
                9303 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(500) =>
                {
                    if self.shaded_turns < 2 {
                        ui.invoke_begin_page_drag(if self.shaded_turns == 0 { 1 } else { -1 });
                        ui.set_drag_progress(0.45);
                        self.phase = 9304;
                    } else {
                        ui.window()
                            .with_winit_window(|window| window.set_maximized(false));
                        ui.window().set_size(slint::LogicalSize::new(780., 540.));
                        set_page(ui, state, queue, self.maximized_start_page, None);
                        self.phase = 9305;
                        self.since = Instant::now();
                    }
                }
                9304 if ui.get_turn_running() && ui.get_drag_active() && !ui.get_turn_priming() => {
                    let pixels = ui.window().take_snapshot()?;
                    png(
                        &self
                            .output
                            .join(format!("page-turn-shaded-{}.png", self.shaded_turns)),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                    )?;
                    ui.invoke_end_page_drag(false);
                    self.shaded_turns += 1;
                    self.phase = 9303;
                    self.since = Instant::now();
                }
                9305 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(500) =>
                {
                    ui.set_theme(if ui.get_theme() == 0 { 1 } else { 0 });
                    self.phase = 110;
                    self.since = Instant::now();
                }
                110 if state.borrow().motion_progress > 0.2 && ui.get_render_busy() => {
                    let pixels = ui.window().take_snapshot()?;
                    png(
                        &self.output.join("theme-transition.png"),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                    )?;
                    self.phase = 111;
                }
                110 | 111
                    if !ui.get_render_busy()
                        && self.since.elapsed() > Duration::from_millis(400) =>
                {
                    if state.borrow().motion_times.len() < 2
                        || ui.get_presented_theme() != ui.get_theme()
                        || ui.get_theme_phase() != 1.
                    {
                        return Err("Theme transition did not present and settle".into());
                    }
                    json_file(
                        &self.output.join("theme-qa.json"),
                        &json!({"passed":true,"presentedFrames":state.borrow().motion_times.len(),"workerComposeMs":state.borrow().motion_times}),
                    )?;
                    ui.invoke_fullscreen_requested();
                    self.phase = 103;
                    self.since = Instant::now();
                }

                103 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(500) =>
                {
                    let full = ui
                        .window()
                        .with_winit_window(|w| w.fullscreen().is_some())
                        .unwrap_or(false);
                    if !full || !ui.get_fullscreen() {
                        return Err("Fullscreen callback did not enter native fullscreen".into());
                    }
                    let pixels = ui.window().take_snapshot()?;
                    png(
                        &self.output.join("fullscreen.png"),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                    )?;
                    ui.invoke_fullscreen_requested();
                    self.phase = 104;
                    self.since = Instant::now();
                }
                104 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(500) =>
                {
                    if ui
                        .window()
                        .with_winit_window(|w| w.fullscreen().is_some())
                        .unwrap_or(true)
                        || ui.get_fullscreen()
                    {
                        return Err("Fullscreen exit did not restore native window".into());
                    }
                    use winit::platform::windows::IconExtWindows;
                    winit::window::Icon::from_resource(1, None)
                        .map_err(|e| format!("Missing embedded application icon: {e}"))?;
                    json_file(
                        &self.output.join("window-qa.json"),
                        &json!({"passed":true,"nativeFullscreenEnterExit":true,"embeddedIconReadable":true}),
                    )?;
                    self.phase = 120;
                    self.since = Instant::now();
                }
                120 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(150) =>
                {
                    let s = state.borrow();
                    let probe = s.probe.ok_or("Missing native hover probe")?;
                    self.unchanged = (s.committed, s.render_count, image_geometry(ui));
                    drop(s);
                    qa_hover_pointer(ui, probe.0, probe.1);
                    self.phase = 121;
                    self.since = Instant::now();
                }
                121 if ui.get_hover_rects().row_count() > 0 => {
                    let s = state.borrow();
                    if self.unchanged != (s.committed, s.render_count, image_geometry(ui)) {
                        return Err("Hover unexpectedly rerendered or moved the book".into());
                    }
                    let probe = s.probe.ok_or("Missing hover probe")?;
                    let regions = ui.get_hover_rects();
                    if !(0..regions.row_count())
                        .filter_map(|i| regions.row_data(i))
                        .any(|r| {
                            probe.0 >= r.x - 4.
                                && probe.0 <= r.x + r.w + 4.
                                && probe.1 >= r.y - 4.
                                && probe.1 <= r.y + r.h + 4.
                        })
                    {
                        return Err("Hover highlight did not cover the native hit probe".into());
                    }
                    drop(s);
                    let capture = ui.window().take_snapshot()?;
                    png(
                        &self.output.join("hover-highlight.png"),
                        capture.as_bytes(),
                        capture.width(),
                        capture.height(),
                    )?;
                    qa_leave_pointer(ui);
                    self.phase = 122;
                    self.since = Instant::now();
                }
                122 if self.since.elapsed() > Duration::from_millis(200) => {
                    ui.set_pointer_hover_enabled(false);
                    let s = state.borrow();
                    if ui.get_hover_rects().row_count() != 0
                        || self.unchanged != (s.committed, s.render_count, image_geometry(ui))
                    {
                        return Err("Hover did not clear on leave without rerendering".into());
                    }
                    drop(s);
                    json_file(
                        &self.output.join("hover-qa.json"),
                        &json!({"passed":true,"nativeProbeCovered":true,"leaveClears":true,"nativeRerenders":0,"slintPointerRouting":true,"noButtonPressed":true}),
                    )?;
                    let probe = state.borrow().probe.ok_or("Missing listen hover probe")?;
                    let width = ui.window().size().width as f32 / ui.window().scale_factor();
                    // Put the panel on the opposite side of this authoritative
                    // probe so it tests exposed Quran rather than a control.
                    ui.set_arabic(ui.get_canvas_x() + ui.get_image_x() + probe.0 < width / 2.);
                    ui.set_panel("listen".into());
                    self.phase = 140;
                    self.since = Instant::now();
                }
                140 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(350) =>
                {
                    let s = state.borrow();
                    let probe = s.probe.ok_or("Missing settled listen hover probe")?;
                    self.unchanged = (s.committed, s.render_count, image_geometry(ui));
                    drop(s);
                    qa_hover_pointer(ui, probe.0, probe.1);
                    self.phase = 141;
                    self.since = Instant::now();
                }
                141 if ui.get_hover_rects().row_count() > 0 => {
                    let s = state.borrow();
                    if self.unchanged != (s.committed, s.render_count, image_geometry(ui))
                        || ui.get_panel() != "listen"
                    {
                        return Err(
                            "Hover beside listening settings rerendered or moved the book".into(),
                        );
                    }
                    drop(s);
                    json_file(
                        &self.output.join("hover-listen-qa.json"),
                        &json!({"passed":true,"visibleQuranBesideListen":true,"slintPointerRouting":true,"noButtonPressed":true,"nativeRerenders":0}),
                    )?;
                    ui.invoke_hover_left();
                    ui.set_panel("".into());
                    ui.set_arabic(true);
                    self.unchanged.0 = state.borrow().committed;
                    ui.set_contextual(true);
                    ui.invoke_preferences_changed();
                    self.phase = 123;
                    self.since = Instant::now();
                }
                123..=125
                    if !ui.get_render_busy()
                        && state.borrow().committed != self.unchanged.0
                        && self.since.elapsed() > Duration::from_millis(200) =>
                {
                    let s = state.borrow();
                    let expected = self.phase != 124;
                    if s.probe.is_none()
                        || s.requested.is_none_or(|v| v.style.contextual != expected)
                    {
                        return Err(
                            "Contextual toggle did not complete with a valid native hit probe"
                                .into(),
                        );
                    }
                    self.unchanged.0 = s.committed;
                    drop(s);
                    if self.phase == 125 {
                        json_file(
                            &self.output.join("contextual-qa.json"),
                            &json!({"passed":true,"sequence":[true,false,true],"nativeHitProbes":true,"scope":"Render completion and original hit probes; not a visual certification"}),
                        )?;
                        ui.set_panel("search".into());
                        ui.set_search_query("2:255".into());
                        ui.invoke_search("2:255".into());
                        self.expected_search_id = state.borrow().search_id;
                        self.phase = 126;
                        self.since = Instant::now();
                    } else {
                        ui.set_contextual(self.phase == 124);
                        ui.invoke_preferences_changed();
                        self.phase += 1;
                        self.since = Instant::now();
                    }
                }
                126 if state.borrow().search_completed_id == Some(self.expected_search_id)
                    && !ui.get_search_busy() =>
                {
                    if ui.get_results().row_count() == 0 {
                        return Err("Verse search did not display results".into());
                    }
                    let previous_rows = ui.get_results();
                    // Simulate a failing subsequent worker search while old rows exist.
                    ui.invoke_search("2:255".into());
                    let id = state.borrow().search_id;
                    apply_output(
                        ui,
                        state,
                        queue,
                        Output::Search(id, Err("QA search failure".into())),
                    );
                    if ui.get_results().row_count() != 0
                        || ui.get_search_status().is_empty()
                        || ui.invoke_choose_result(0)
                        || ui.get_panel() != "search"
                    {
                        return Err("Search failure left stale actionable results".into());
                    }
                    // Oversized input cancels the pending search and must be equally safe.
                    // Restore the real prior rows so the test detects a failure to clear them.
                    ui.set_results(previous_rows);
                    ui.invoke_search("x".repeat(257).into());
                    if ui.get_results().row_count() != 0
                        || ui.get_search_busy()
                        || ui.get_search_status().is_empty()
                        || ui.invoke_choose_result(0)
                        || ui.get_panel() != "search"
                    {
                        return Err("Oversized search left stale actionable results".into());
                    }
                    ui.invoke_search("2:255".into());
                    self.expected_search_id = state.borrow().search_id;
                    self.phase = 129;
                }
                129 if state.borrow().search_completed_id == Some(self.expected_search_id)
                    && !ui.get_search_busy() =>
                {
                    let index = state
                        .borrow()
                        .hits
                        .iter()
                        .position(|hit| hit.key == Some((2, 255)))
                        .ok_or("Verse search did not expose 2:255")?;
                    if !ui.invoke_choose_result(index as i32) {
                        return Err("Valid search result was rejected".into());
                    }
                    // Match the actual search-result action, which dismisses the panel.
                    ui.set_panel("".into());
                    self.phase = 127;
                    self.since = Instant::now();
                }
                127 if !ui.get_render_busy()
                    && state.borrow().selection == Some((2, 255))
                    && state.borrow().committed == state.borrow().generation
                    && ui.get_panel() == "study"
                    && !ui.get_study_body().is_empty()
                    && self.since.elapsed() > Duration::from_millis(250) =>
                {
                    self.unchanged.0 = state.borrow().committed;
                    ui.set_context_open(true);
                    ui.set_contextual(!ui.get_contextual());
                    ui.invoke_preferences_changed();
                    self.phase = 128;
                    self.since = Instant::now();
                }
                128 if !ui.get_render_busy()
                    && state.borrow().committed == state.borrow().generation
                    && state.borrow().committed != self.unchanged.0
                    && self.since.elapsed() > Duration::from_millis(250) =>
                {
                    if ui.get_context_open()
                        || ui.get_panel() != "study"
                        || state.borrow().selection != Some((2, 255))
                    {
                        return Err(
                            "Search-to-study or context cancellation failed during rerender".into(),
                        );
                    }
                    json_file(
                        &self.output.join("search-study-qa.json"),
                        &json!({"passed":true,"selectedVerse":"2:255","studyOpened":true,"rerenderClosedContext":true,"searchFailureClearsResults":true,"oversizedQueryClearsResults":true,"invalidSelectionRejected":true}),
                    )?;
                    set_page(ui, state, queue, 319, None);
                    self.phase = 150;
                    self.since = Instant::now();
                }
                150 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(350) =>
                {
                    self.expected_search_id = app_core::destination(
                        state.borrow().page,
                        1,
                        state.borrow().last["singlePage"].as_bool().unwrap_or(false),
                    ) as i32;
                    if !state
                        .borrow()
                        .prefetched_pages
                        .contains(&(self.expected_search_id as u32))
                    {
                        return Ok(false);
                    }
                    if state.borrow().prefetch_bytes > page_cache::MAX_BYTES {
                        return Err("Adjacent cache exceeded its pixel budget".into());
                    }
                    self.before_drag = Some(ui.window().take_snapshot()?);
                    qa_corner_pointer(ui, 0., 0);
                    self.phase = 151;
                }
                151 if ui.get_turn_running() && ui.get_drag_active() && !ui.get_turn_priming() => {
                    if state.borrow().last["prefetched"] != true {
                        return Err(
                            "A ready adjacent page was rendered again instead of reused".into()
                        );
                    }
                    json_file(
                        &self.output.join("prefetch-qa.json"),
                        &json!({"passed":true,"cacheHit":true,"pixelBudget":page_cache::MAX_BYTES,"nativeHitMapPreservedWhilePreparing":true,"primedBeforeMotion":state.borrow().primed_turns > 0}),
                    )?;
                    let baseline = self.before_drag.as_ref().ok_or("Missing drag baseline")?;
                    let now = ui.window().take_snapshot()?;
                    let scale = ui.window().scale_factor();
                    let x = ((ui.get_canvas_x() + ui.get_image_x()) * scale).round() as u32;
                    let y = ((ui.get_canvas_y() + ui.get_image_y()) * scale).round() as u32;
                    let w = (ui.get_image_w() * scale).floor() as u32;
                    let h = (ui.get_image_h() * scale).floor() as u32;
                    let mut changed = 0;
                    let mut first = None;
                    for row in y..(y + h).min(now.height()) {
                        for col in x..(x + w).min(now.width()) {
                            let i = ((row * now.width() + col) * 4) as usize;
                            if now.as_bytes()[i..i + 4] != baseline.as_bytes()[i..i + 4] {
                                changed += 1;
                                first.get_or_insert((
                                    col,
                                    row,
                                    baseline.as_bytes()[i..i + 4].to_vec(),
                                    now.as_bytes()[i..i + 4].to_vec(),
                                ));
                            }
                        }
                    }
                    if changed != 0 {
                        return Err(format!(
                            "Before a drag moved, {changed} visible page pixels changed; first (x, y, before, now)={first:?}; imageOrigin=({x}, {y}); imageSize=({w}, {h}); progress={}; dragProgress={}; image=[{}, {}, {}, {}]",
                            ui.get_turn_progress(), ui.get_drag_progress(), ui.get_image_x(), ui.get_image_y(), ui.get_image_w(), ui.get_image_h()
                        )
                        .into());
                    }
                    png(
                        &self.output.join("drag-before-move.png"),
                        now.as_bytes(),
                        now.width(),
                        now.height(),
                    )?;
                    qa_corner_pointer(ui, 0.03, 1);
                    let lifted = ui.window().take_snapshot()?;
                    // A small corner lift must leave the far side readable
                    // with its original page, in both narrow and spread views.
                    for row in y + h / 5..(y + h * 4 / 5).min(lifted.height()) {
                        for col in x + w * 3 / 5..(x + w * 9 / 10).min(lifted.width()) {
                            let i = ((row * lifted.width() + col) * 4) as usize;
                            if lifted.as_bytes()[i..i + 4] != baseline.as_bytes()[i..i + 4] {
                                return Err(
                                    "The far page changed before the fold reached it".into()
                                );
                            }
                        }
                    }
                    png(
                        &self.output.join("drag-small-lift.png"),
                        lifted.as_bytes(),
                        lifted.width(),
                        lifted.height(),
                    )?;
                    qa_fold_faces(ui, &self.output)?;
                    qa_corner_pointer(ui, 1., 1);
                    let held = ui.window().take_snapshot()?;
                    png(
                        &self.output.join("drag-fully-open.png"),
                        held.as_bytes(),
                        held.width(),
                        held.height(),
                    )?;
                    let target = ui
                        .get_next_book()
                        .to_rgba8()
                        .ok_or("Missing destination pixels")?;
                    let mut mismatched = 0;
                    let mut sampled = 0;
                    for row in (y + 4..(y + h - 4).min(held.height())).step_by(3) {
                        for col in (x + 4..(x + w - 4).min(held.width())).step_by(3) {
                            let tx = (((col - x) as f32 / (ui.get_image_w() * scale))
                                * target.width() as f32)
                                as u32;
                            let ty = (((row - y) as f32 / (ui.get_image_h() * scale))
                                * target.height() as f32)
                                as u32;
                            let i = ((row * held.width() + col) * 4) as usize;
                            let j = ((ty.min(target.height() - 1) * target.width()
                                + tx.min(target.width() - 1))
                                * 4) as usize;
                            sampled += 1;
                            if (0..3).any(|c| {
                                held.as_bytes()[i + c].abs_diff(target.as_bytes()[j + c]) > 24
                            }) {
                                mismatched += 1;
                            }
                        }
                    }
                    if mismatched * 100 > sampled * 3 {
                        return Err(format!("Held fully opened page differs from its destination: {mismatched}/{sampled}").into());
                    }
                    qa_corner_pointer(ui, 1., 2);
                    self.phase = 152;
                }
                152 if !ui.get_render_busy() => {
                    if state.borrow().page != self.expected_search_id as u32 || ui.get_drag_active()
                    {
                        return Err("Corner drag did not commit the destination spread".into());
                    }
                    qa_corner_pointer(ui, 0., 0);
                    self.phase = 153;
                }
                153 if ui.get_turn_running() && ui.get_drag_active() => {
                    qa_corner_pointer(ui, 0.12, 1);
                    qa_corner_pointer(ui, 0.12, 2);
                    self.phase = 154;
                }
                154 if !ui.get_render_busy() => {
                    if state.borrow().page != self.expected_search_id as u32
                        || ui.get_drag_active()
                        || ui.get_drag_canceling()
                    {
                        return Err("Short corner drag did not restore the original spread".into());
                    }
                    self.unchanged.0 = state.borrow().render_count as i32;
                    let visible = if state.borrow().last["singlePage"].as_bool().unwrap_or(false) {
                        self.expected_search_id as u32
                    } else {
                        self.expected_search_id as u32 + 1
                    };
                    set_page(ui, state, queue, visible, None);
                    self.phase = 155;
                    self.since = Instant::now();
                }
                155 if self.since.elapsed() > Duration::from_millis(300) => {
                    if state.borrow().page != self.expected_search_id as u32
                        || state.borrow().render_count as i32 != self.unchanged.0
                        || ui.get_turn_running()
                    {
                        return Err(
                            "Navigation within the visible spread caused a page turn".into()
                        );
                    }
                    self.expected_search_id = app_core::destination(
                        state.borrow().page,
                        1,
                        state.borrow().last["singlePage"].as_bool().unwrap_or(false),
                    ) as i32;
                    let position = slint::LogicalPosition::new(
                        ui.get_canvas_x() + ui.get_image_x() + 6.,
                        ui.get_canvas_y() + ui.get_image_y() + ui.get_image_h() / 2.,
                    );
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
                            position,
                            button: slint::platform::PointerEventButton::Left,
                        });
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
                            position,
                            button: slint::platform::PointerEventButton::Left,
                        });
                    self.phase = 156;
                    self.since = Instant::now();
                }
                156 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(350) =>
                {
                    if state.borrow().page != self.expected_search_id as u32 {
                        return Err("Clicking the middle of the page edge did not turn it".into());
                    }
                    json_file(
                        &self.output.join("corner-drag-qa.json"),
                        &json!({"passed":true,"pointerRouting":true,"commit":true,"cancelRestoresOriginal":true,"visibleSpreadDoesNotTurn":true,"edgeClickTurns":true,"beforeMovementPixelChanges":0,"smallLiftKeepsFarPage":true,"heldFullDragShowsDestination":true,"twoPageLayout":!state.borrow().last["singlePage"].as_bool().unwrap_or(false),"destination":self.expected_search_id}),
                    )?;
                    ui.set_panel("search".into());
                    self.phase = 170;
                    self.since = Instant::now();
                }
                170 if self.since.elapsed() > Duration::from_millis(220) => {
                    let _ = ui.get_side_reveal();
                    ui.set_panel("".into());
                    self.phase = 171;
                    self.since = Instant::now();
                }
                171 if self.since.elapsed() > Duration::from_millis(220) => {
                    if ui.get_side_reveal() > 0.01 {
                        return Err("Search did not finish closing".into());
                    }
                    ui.set_panel("study".into());
                    let x = ui.get_side_x();
                    let width = ui.window().size().width as f32 / ui.window().scale_factor();
                    if (ui.get_arabic() && x < width - 10.) || (!ui.get_arabic() && x > -300.) {
                        return Err(
                            format!("Tafsir started from a hidden central dialog: x={x}").into(),
                        );
                    }
                    let pixels = ui.window().take_snapshot()?;
                    png(
                        &self.output.join("tafsir-side-start.png"),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                    )?;
                    self.phase = 172;
                    self.since = Instant::now();
                }
                172 if self.since.elapsed() > Duration::from_millis(220)
                    && !ui.get_render_busy() =>
                {
                    if ui.get_side_reveal() < 0.99 {
                        return Err("Tafsir did not finish its side entrance".into());
                    }
                    json_file(
                        &self.output.join("tafsir-side-qa.json"),
                        &json!({"passed":true,"searchThenTafsirStartsAtEdge":true,"settlesAtSide":true}),
                    )?;
                    ui.set_panel("".into());
                    ui.set_onboarding_complete(false);
                    self.phase = 173;
                    self.since = Instant::now();
                }
                173 if self.since.elapsed() > Duration::from_millis(220)
                    && !ui.get_render_busy() =>
                {
                    let pixels = ui.window().take_snapshot()?;
                    png(
                        &self.output.join("onboarding.png"),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                    )?;
                    if ui
                        .window()
                        .with_winit_window(|w| w.is_maximized())
                        .unwrap_or(true)
                    {
                        return Err("Unexpected pre-test maximize".into());
                    }
                    qa_caption_click(ui, 66.);
                    self.phase = 174;
                    self.since = Instant::now();
                }
                174 if self.since.elapsed() > Duration::from_millis(350) => {
                    if !ui
                        .window()
                        .with_winit_window(|w| w.is_maximized())
                        .unwrap_or(false)
                        || ui.get_onboarding_complete()
                    {
                        return Err("Onboarding blocked the maximize caption".into());
                    }
                    qa_caption_click(ui, 66.);
                    self.phase = 175;
                    self.since = Instant::now();
                }
                175 if self.since.elapsed() > Duration::from_millis(350) => {
                    if ui
                        .window()
                        .with_winit_window(|w| w.is_maximized())
                        .unwrap_or(true)
                    {
                        return Err("Onboarding blocked restore".into());
                    }
                    qa_caption_click(ui, 110.);
                    self.phase = 176;
                    self.since = Instant::now();
                }
                176 if self.since.elapsed() > Duration::from_millis(350) => {
                    if !ui
                        .window()
                        .with_winit_window(|w| w.is_minimized().unwrap_or(false))
                        .unwrap_or(false)
                    {
                        return Err("Onboarding blocked the minimize caption".into());
                    }
                    ui.window().with_winit_window(|w| w.set_minimized(false));
                    json_file(
                        &self.output.join("onboarding-window-qa.json"),
                        &json!({"passed":true,"pointerDispatchedToCaptions":true,"maximized":true,"restored":true,"minimized":true,"introductionStayedOpen":!ui.get_onboarding_complete()}),
                    )?;
                    ui.set_onboarding_complete(true);
                    ui.set_arabic(false);
                    ui.set_panel("".into());
                    self.phase = 177;
                    self.since = Instant::now();
                }
                177 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(350) =>
                {
                    // Under a software scale override, Winit restores the OS-DPI
                    // physical size. Request physical pixels explicitly so this
                    // test really exercises a 780-DIP viewport at every scale.
                    let scale = ui.window().scale_factor();
                    ui.window().set_size(slint::PhysicalSize::new(
                        (780. * scale).round() as u32,
                        (540. * scale).round() as u32,
                    ));
                    self.phase = 180;
                    self.since = Instant::now();
                }
                180 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(350) =>
                {
                    let locale = if ui.get_arabic() { "ar" } else { "en" };
                    qa_named_snapshot(ui, &self.output, &format!("chrome-{locale}"))?;
                    self.chrome_records.push(qa_chrome_geometry(ui)?);
                    ui.set_search_query("".into());
                    ui.set_search_mode(0);
                    ui.invoke_open_search();
                    self.expected_search_id = state.borrow().search_id;
                    self.phase = 181;
                    self.since = Instant::now();
                }
                181 if !ui.get_render_busy()
                    && !ui.get_search_busy()
                    && state.borrow().search_completed_id == Some(self.expected_search_id)
                    && self.since.elapsed() > Duration::from_millis(300) =>
                {
                    let locale = if ui.get_arabic() { "ar" } else { "en" };
                    qa_named_snapshot(ui, &self.output, &format!("search-{locale}"))?;
                    if ui.get_panel() != "search" || ui.get_results().row_count() != 114 {
                        return Err("Search did not open with the complete surah list".into());
                    }
                    let field = ui.get_search_field_height();
                    let tabs = ui.get_search_tab_height();
                    let result = ui.get_search_result_height();
                    if !(36.0..=44.5).contains(&field)
                        || !(28.0..=40.0).contains(&tabs)
                        || !(48.0..=96.0).contains(&result)
                        || !ui.get_search_scroll_left()
                    {
                        return Err(format!(
                            "Search controls escaped their layout bounds: field={field}, tabs={tabs}, result={result}, leftScroll={}",
                            ui.get_search_scroll_left()
                        ).into());
                    }
                    let record = self
                        .chrome_records
                        .last_mut()
                        .ok_or("Missing chrome record")?;
                    record["search"] = json!({"surahRows":114,"fieldHeight":field,"tabHeight":tabs,"resultHeight":result,"separateLeftScrollGutter":true});
                    if ui.get_search_active_index() != -1 {
                        return Err("Search opened with a stale active result".into());
                    }
                    qa_search_key(ui, slint::platform::Key::DownArrow);
                    if ui.get_search_active_index() != 0 {
                        return Err("Search input did not receive the first Down key".into());
                    }
                    qa_search_key(ui, slint::platform::Key::DownArrow);
                    if ui.get_search_active_index() != 1 {
                        return Err("Search Down key did not advance the active result".into());
                    }
                    qa_search_key(ui, slint::platform::Key::UpArrow);
                    if ui.get_search_active_index() != 0 {
                        return Err("Search Up key did not restore the previous result".into());
                    }
                    record["keyboardDownUp"] = json!(true);
                    ui.set_search_query("52".into());
                    ui.invoke_search("52".into());
                    self.expected_search_id = state.borrow().search_id;
                    self.phase = 182;
                    self.since = Instant::now();
                }
                182 if !ui.get_search_busy()
                    && state.borrow().search_completed_id == Some(self.expected_search_id)
                    && self.since.elapsed() > Duration::from_millis(220) =>
                {
                    let locale = if ui.get_arabic() { "ar" } else { "en" };
                    qa_named_snapshot(ui, &self.output, &format!("search-number-{locale}"))?;
                    let s = state.borrow();
                    if !s.hits.iter().any(|hit| hit.badge == "52")
                        || !s
                            .hits
                            .iter()
                            .any(|hit| hit.page == 52 && hit.badge.is_empty())
                    {
                        return Err(
                            "Numeric search did not distinguish surah 52 from page 52".into()
                        );
                    }
                    let (surah_index, surah_page) = s
                        .hits
                        .iter()
                        .enumerate()
                        .find(|(_, hit)| hit.badge == "52")
                        .map(|(index, hit)| (index as i32, hit.page))
                        .ok_or("Numeric search lost surah 52")?;
                    drop(s);
                    self.chrome_records
                        .last_mut()
                        .ok_or("Missing chrome record")?["numericSearchDistinguishesSurahAndPage"] =
                        json!(true);
                    if ui.get_search_active_index() != -1 {
                        return Err("A changed search retained its previous active result".into());
                    }
                    for _ in 0..=surah_index {
                        qa_search_key(ui, slint::platform::Key::DownArrow);
                    }
                    if ui.get_search_active_index() != surah_index {
                        return Err(
                            "Keyboard selection did not reach the numeric surah result".into()
                        );
                    }
                    qa_search_key(ui, slint::platform::Key::Return);
                    if !ui.get_panel().is_empty()
                        || state.borrow().page != surah_page
                        || state.borrow().selection.is_some()
                    {
                        return Err(
                            "Search Enter did not choose the validated result and close the dialog"
                                .into(),
                        );
                    }
                    let record = self
                        .chrome_records
                        .last_mut()
                        .ok_or("Missing chrome record")?;
                    record["keyboardEnterChoosesResult"] = json!(true);
                    record["chosenSurahPage"] = json!(surah_page);
                    self.phase = 183;
                    self.since = Instant::now();
                }
                183 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(220) =>
                {
                    ui.set_panel("settings".into());
                    self.phase = 184;
                    self.since = Instant::now();
                }
                184 if !ui.get_render_busy()
                    && self.since.elapsed() > Duration::from_millis(300) =>
                {
                    let locale = if ui.get_arabic() { "ar" } else { "en" };
                    qa_named_snapshot(ui, &self.output, &format!("settings-{locale}"))?;
                    if !ui.get_arabic() {
                        ui.set_panel("".into());
                        ui.set_arabic(true);
                        self.phase = 180;
                        self.since = Instant::now();
                    } else {
                        if self.chrome_records.len() != 2 {
                            return Err(
                                "Chrome QA did not complete both interface languages".into()
                            );
                        }
                        json_file(
                            &self.output.join("chrome-search-qa.json"),
                            &json!({"passed":true,"locales":self.chrome_records,"windowWidth":ui.window().size().width as f32 / ui.window().scale_factor(),"scope":"Slint layout bounds, native search results and renderer snapshots; not screen-reader or physical mixed-monitor certification"}),
                        )?;
                        self.phase = 105;
                    }
                }
                _ => {}
            }
            if self.phase != 105 {
                return Ok(false);
            }
            self.done = true;
            json_file(
                &self.output.join("qa-report.json"),
                &json!({"passed":true,"cases":self.records,"scope":"Native source pixels, real hit/study/search, fixed settings overlay, separate study column and window checks; not displayed-FPS, Narrator or production-migration certification"}),
            )?;
            return Ok(true);
        }
        let pages = [1, 319, 337, 367, 531, 603, 2, 295];
        match self.phase {
            0 => {
                self.expected_selection = None;
                ui.set_panel("".into());
                ui.set_study_body("".into());
                ui.set_motion_enabled(self.index.is_multiple_of(2));
                ui.set_arabic(self.index >= 4);
                ui.set_theme(if self.index == 3 {
                    2
                } else {
                    (self.index % 2) as i32
                });
                ui.set_dark(matches!(ui.get_theme(), 0 | 3 | 4));
                ui.set_contextual(self.index % 3 == 1);
                ui.set_single_page(self.index == 7);
                ui.set_show_clock(false);
                let size = if self.index.is_multiple_of(3) {
                    (780., 540.)
                } else if self.index % 3 == 1 {
                    (1180., 820.)
                } else {
                    (1420., 900.)
                };
                ui.window()
                    .set_size(slint::LogicalSize::new(size.0, size.1));
                set_page(ui, state, queue, pages[self.index], None);
                self.phase = 1;
                self.since = Instant::now();
            }
            1 if !ui.get_render_busy()
                && ui.get_previous_book().size().width == 0
                && state.borrow().committed == state.borrow().generation
                && self.since.elapsed() > Duration::from_millis(350) =>
            {
                let s = state.borrow();
                let pixels = ui.window().take_snapshot()?;
                png(
                    &self
                        .output
                        .join(format!("case-{:02}-reader.png", self.index + 1)),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                )?;
                let (expected, w, h) = s
                    .qa_pixels
                    .as_ref()
                    .ok_or("Missing expected native pixels")?;
                let scale = ui.window().scale_factor();
                let x = ((ui.get_canvas_x() + ui.get_image_x()) * scale).round() as u32;
                let y = ((ui.get_canvas_y() + ui.get_image_y()) * scale).round() as u32;
                if x + w > pixels.width() || y + h > pixels.height() {
                    return Err("Native book extends outside window snapshot".into());
                }
                let mut differences = 0u64;
                for row in 0..*h {
                    for col in 0..*w {
                        let expected_index = ((row * w + col) * 4) as usize;
                        let actual_index = (((row + y) * pixels.width() + col + x) * 4) as usize;
                        if expected[expected_index..expected_index + 4]
                            != pixels.as_bytes()[actual_index..actual_index + 4]
                        {
                            differences += 1;
                        }
                    }
                }
                png(
                    &self
                        .output
                        .join(format!("case-{:02}-native.png", self.index + 1)),
                    expected,
                    *w,
                    *h,
                )?;
                let mut record = s.last.clone();
                record["arabic"] = json!(ui.get_arabic());
                record["dark"] = json!(ui.get_dark());
                record["theme"] = json!(ui.get_theme());
                record["contextual"] = json!(ui.get_contextual());
                record["forceSinglePage"] = json!(ui.get_single_page());
                record["scale"] = json!(scale);
                record["compositorPixelDifferences"] = json!(differences);
                record["cropOrigin"] = json!([x, y]);
                self.records.push(record);
                if differences > 0 {
                    return Err(format!("Native-to-Slint pixel comparison differed at {differences} pixels in case {}",self.index+1).into());
                }
                self.unchanged = (s.committed, s.render_count, image_geometry(ui));
                drop(s);
                ui.set_panel("settings".into());
                self.phase = 2;
                self.since = Instant::now();
            }
            2 if self.since.elapsed() > Duration::from_millis(300) => {
                let s = state.borrow();
                if self.unchanged != (s.committed, s.render_count, image_geometry(ui)) {
                    return Err(
                        "Opening settings changed native book geometry or rerendered it".into(),
                    );
                }
                let pixels = ui.window().take_snapshot()?;
                png(
                    &self
                        .output
                        .join(format!("case-{:02}-settings.png", self.index + 1)),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                )?;
                drop(s);
                ui.set_panel("search".into());
                ui.set_search_query("2:255".into());
                ui.invoke_search("2:255".into());
                self.expected_search_id = state.borrow().search_id;
                self.phase = 3;
                self.since = Instant::now();
            }
            3 if state.borrow().search_completed_id == Some(self.expected_search_id)
                && self.since.elapsed() > Duration::from_millis(300) =>
            {
                let s = state.borrow();
                if !s.hits.iter().any(|hit| hit.key == Some((2, 255))) {
                    return Err("Real verse-reference search did not return 2:255".into());
                }
                self.records.last_mut().unwrap()["searchReferenceVerified"] = json!(true);
                let probe = s.probe.ok_or("No source-based hit probe")?;
                self.turns_before_study = s.book_turns;
                self.expected_selection = Some(probe.2);
                drop(s);
                ui.set_panel("".into());
                ui.invoke_pick_ayah(ui.get_image_x() + probe.0, ui.get_image_y() + probe.1);
                self.phase = 4;
                self.since = Instant::now();
            }
            4 if !ui.get_render_busy()
                && !ui.get_study_body().is_empty()
                && self.since.elapsed() > Duration::from_millis(350) =>
            {
                let s = state.borrow();
                if s.selection != self.expected_selection
                    || s.committed != s.generation
                    || s.committed == self.unchanged.0
                    || ui.get_panel() != "study"
                {
                    return Err("Source hit did not commit the expected study selection".into());
                }
                if s.book_turns != self.turns_before_study {
                    return Err("Opening tafsir started a book-fold transition".into());
                }
                let logical_width = ui.window().size().width as f32 / ui.window().scale_factor();
                let reserved = if ui.get_arabic() {
                    ui.get_canvas_x() + ui.get_image_x() + ui.get_image_w()
                        <= logical_width - (344_f32.min(logical_width * 0.4 - 20.)) - 16.
                } else {
                    ui.get_canvas_x() + ui.get_image_x()
                        >= 344_f32.min(logical_width * 0.4 - 20.) + 16.
                };
                if !reserved {
                    return Err("Study column did not reserve reading space".into());
                }
                drop(s);
                let pixels = ui.window().take_snapshot()?;
                if !(34.0..=38.0).contains(&ui.get_study_choice_height()) {
                    return Err(format!(
                        "Tafsir source field grew to {} DIP",
                        ui.get_study_choice_height()
                    )
                    .into());
                }
                png(
                    &self
                        .output
                        .join(format!("case-{:02}-study.png", self.index + 1)),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                )?;
                self.records.last_mut().unwrap()["originalHitAndBundledStudyVerified"] =
                    json!(true);
                self.records.last_mut().unwrap()["studyReservedColumn"] = json!(true);
                self.records.last_mut().unwrap()["studyHasNoBookFold"] = json!(true);
                self.records.last_mut().unwrap()["studyChoiceHeight"] =
                    json!(ui.get_study_choice_height());
                json_file(
                    &self.output.join(format!("case-{:02}.json", self.index + 1)),
                    self.records.last().unwrap(),
                )?;
                if self.index == 0 {
                    let s = state.borrow();
                    let probe = s.probe.ok_or("Missing study hover probe")?;
                    self.unchanged = (s.committed, s.render_count, image_geometry(ui));
                    drop(s);
                    qa_hover_pointer(ui, probe.0, probe.1);
                    self.phase = 5;
                } else {
                    self.index += 1;
                    self.phase = 0;
                }
                self.since = Instant::now();
            }
            5 if ui.get_hover_rects().row_count() > 0 => {
                let s = state.borrow();
                if self.unchanged != (s.committed, s.render_count, image_geometry(ui)) {
                    return Err("Study hover changed the native book geometry".into());
                }
                drop(s);
                ui.set_context_open(true);
                self.phase = 6;
                self.since = Instant::now();
            }
            6 if self.since.elapsed() > Duration::from_millis(100) => {
                let s = state.borrow();
                if ui.get_hover_rects().row_count() != 0
                    || self.unchanged != (s.committed, s.render_count, image_geometry(ui))
                {
                    return Err("Opening the context menu did not clear study hover".into());
                }
                let probe = s.probe.ok_or("Missing study hover probe")?;
                drop(s);
                ui.set_context_open(false);
                ui.invoke_hover_moved(ui.get_image_x() + probe.0, ui.get_image_y() + probe.1);
                // Cancel a pending throttled request before it can publish.
                ui.set_context_open(true);
                self.phase = 7;
                self.since = Instant::now();
            }
            7 if self.since.elapsed() > Duration::from_millis(120) => {
                let s = state.borrow();
                if ui.get_hover_rects().row_count() != 0
                    || self.unchanged != (s.committed, s.render_count, image_geometry(ui))
                {
                    return Err("A cancelled study hover was published".into());
                }
                drop(s);
                json_file(
                    &self.output.join("hover-study-qa.json"),
                    &json!({"passed":true,"nativeProbe":true,"contextClears":true,"pendingHoverCancelled":true,"nativeRerenders":0,"slintPointerRouting":true,"noButtonPressed":true}),
                )?;
                ui.set_context_open(false);
                self.index += 1;
                self.phase = 0;
                self.since = Instant::now();
            }
            _ => {}
        }
        Ok(false)
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if let Some(index) = args.iter().position(|arg| arg == "--qa-audio") {
        let out = PathBuf::from(
            args.get(index + 1)
                .ok_or("Missing audio verification output")?,
        );
        let fixture = PathBuf::from(
            args.get(index + 2)
                .ok_or("Missing original audio fixture directory")?,
        );
        return audio_qa::run(&out, &fixture);
    }
    let qa_output = args
        .iter()
        .position(|a| a == "--qa-output")
        .map(|i| {
            args.get(i + 1)
                .map(PathBuf::from)
                .ok_or("Missing QA output")
        })
        .transpose()?;
    let player_output = args
        .iter()
        .position(|arg| arg == "--qa-player")
        .map(|i| {
            args.get(i + 1)
                .map(PathBuf::from)
                .ok_or("Missing main-player QA output")
        })
        .transpose()?;
    if qa_output.is_some() && player_output.is_some() {
        return Err("Choose one QA mode".into());
    }
    let player_profile = if let Some(out) = &player_output {
        let i = args.iter().position(|arg| arg == "--qa-player").unwrap();
        let fixture = PathBuf::from(args.get(i + 2).ok_or("Missing original audio fixture")?);
        fs::create_dir(out)?;
        let fixture_out = out.join("verified-fixture");
        let proof = audio_qa::seed(&fixture_out, &fixture)?;
        json_file(
            &out.join("audio-fixture.json"),
            &json!({"files":proof,"source":fixture,"copiedProfile":fixture_out.join("profile")}),
        )?;
        Some(fixture_out.join("profile"))
    } else {
        None
    };
    if let Some(output) = &qa_output {
        fs::create_dir(output)?;
    }
    let profile_directory = if let Some(profile) = player_profile {
        profile
    } else if let Some(index) = args.iter().position(|arg| arg == "--profile") {
        PathBuf::from(
            args.get(index + 1)
                .ok_or("Missing isolated profile directory")?,
        )
    } else if let Some(out) = &qa_output {
        out.join("profile")
    } else {
        preferences::default_profile_directory()?
    };
    // Software rendering held a maximized fold under 60 Hz on the test PC, and
    // the GPU kept settled native pixels identical. "skia" falls back to
    // software by itself when no GPU surface can start, and this override
    // remains for drivers that start but misbehave.
    let software_rendering = match args
        .iter()
        .position(|arg| arg == "--renderer")
        .map(|i| args.get(i + 1).and_then(|value| value.to_str()))
    {
        Some(Some("software")) => true,
        Some(Some("gpu")) => false,
        Some(_) => return Err("--renderer must be gpu or software".into()),
        None => preferences::saved_software_rendering(&profile_directory),
    };
    // For comparing turn speeds without a rebuild, e.g. `--turn-ms 285`.
    if let Some(i) = args.iter().position(|arg| arg == "--turn-ms") {
        match args
            .get(i + 1)
            .and_then(|value| value.to_str())
            .and_then(|value| value.parse::<i64>().ok())
        {
            Some(ms) => turn_queue::set_turn_ms(ms),
            None => return Err("--turn-ms needs a number of milliseconds".into()),
        }
    }
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(
            if software_rendering {
                "skia-software"
            } else {
                "skia"
            }
            .into(),
        )
        .select()?;
    let ui = ReaderWindow::new()?;
    ui.on_compute_fold(|progress, width, height, single, direction, top| {
        let scene = page_animation::fold_scene(width, height, single, direction, progress, top);
        FoldGeometry {
            plane_x: scene.plane[0],
            plane_y: scene.plane[1],
            plane_angle: scene.plane[2],
            back_x: scene.back[0],
            back_y: scene.back[1],
            back_angle: scene.back[2],
        }
    });
    let about = ReaderAbout::new()?;
    let resource_directory = resource_root()?;
    let controller = Rc::new(controls::Controller::open(
        &ui,
        &resource_directory,
        profile_directory,
        qa_output.is_some() || player_output.is_some(),
    )?);
    ui.set_pointer_hover_enabled(qa_output.is_none());
    let (worker, rx) = start_worker(resource_directory, ui.as_weak());
    let state = Rc::new(RefCell::new(State {
        prefetched_pages: Vec::new(),
        prefetch_bytes: 0,
        prime_scheduled: None,
        settle_scheduled: None,
        drag_finish: None,
        book_turns: 0,
        page: 1,
        qa: qa_output.is_some(),
        ..State::default()
    }));
    controller.initialize_state(&ui, &state);
    let queue = worker.queue.clone();
    {
        let weak = ui.as_weak();
        ui.on_window_action(move |action| {
            if let Some(ui) = weak.upgrade() {
                if action == 2 {
                    let _ = slint::quit_event_loop();
                    return;
                }
                ui.window().with_winit_window(|window| match action {
                    0 => window.set_minimized(true),
                    1 => {
                        window.set_maximized(!window.is_maximized());
                        ui.set_window_maximized(window.is_maximized());
                    }
                    3 => {
                        let _ = window.drag_window();
                    }
                    _ => {}
                });
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_resize_edge(move |edge| {
            if let Some(ui) = weak.upgrade() {
                let direction = match edge {
                    0 => winit::window::ResizeDirection::West,
                    1 => winit::window::ResizeDirection::East,
                    2 => winit::window::ResizeDirection::North,
                    _ => winit::window::ResizeDirection::South,
                };
                ui.window().with_winit_window(|window| {
                    let _ = window.drag_resize_window(direction);
                });
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let queue = queue.clone();
        let modifiers = std::cell::Cell::new(winit::keyboard::ModifiersState::empty());
        ui.window().on_winit_window_event(move |_, event| {
            if let Some(ui) = weak.upgrade() {
                if matches!(event, winit::event::WindowEvent::KeyboardInput { .. }) {
                    ui.set_keyboard_focus(true);
                }
                if matches!(
                    event,
                    winit::event::WindowEvent::MouseInput {
                        state: winit::event::ElementState::Pressed,
                        ..
                    }
                ) {
                    ui.set_keyboard_focus(false);
                }
            }
            if let winit::event::WindowEvent::ModifiersChanged(value) = event {
                modifiers.set(value.state());
            }
            if let winit::event::WindowEvent::Focused(false) = event {
                modifiers.set(winit::keyboard::ModifiersState::empty());
                // Windows may never deliver the key-up for a key held across focus loss.
                state.borrow_mut().turns.release_repeats();
            }
            if let winit::event::WindowEvent::KeyboardInput { event: key, .. } = event
                && key.state == winit::event::ElementState::Pressed
                && !key.repeat
                && let Some(ui) = weak.upgrade()
                && ui.get_onboarding_complete()
            {
                use winit::keyboard::{KeyCode, PhysicalKey};
                let ctrl = modifiers.get().control_key()
                    && !modifiers.get().alt_key()
                    && !modifiers.get().super_key()
                    && !modifiers.get().shift_key();
                let code = key.physical_key;
                let search = [KeyCode::KeyF, KeyCode::KeyK, KeyCode::Slash]
                    [ui.get_search_shortcut().clamp(0, 2) as usize];
                let listen =
                    [KeyCode::KeyL, KeyCode::KeyP][ui.get_listen_shortcut().clamp(0, 1) as usize];
                let options =
                    [KeyCode::Comma, KeyCode::KeyO][ui.get_options_shortcut().clamp(0, 1) as usize];
                if ctrl && code == PhysicalKey::Code(search) {
                    ui.invoke_open_search();
                    return EventResult::PreventDefault;
                }
                if ctrl && code == PhysicalKey::Code(listen) {
                    ui.set_panel("listen".into());
                    return EventResult::PreventDefault;
                }
                if ctrl && code == PhysicalKey::Code(options) {
                    ui.set_panel("settings".into());
                    return EventResult::PreventDefault;
                }
                if ctrl && code == PhysicalKey::Code(KeyCode::Space) && ui.get_audio_active() {
                    ui.invoke_audio_toggle();
                    return EventResult::PreventDefault;
                }
                if code == PhysicalKey::Code(KeyCode::F11) {
                    ui.invoke_fullscreen_requested();
                    return EventResult::PreventDefault;
                }
            }
            if matches!(
                event,
                winit::event::WindowEvent::Focused(true)
                    | winit::event::WindowEvent::Occluded(false)
            ) {
                let invalidate = matches!(event, winit::event::WindowEvent::Occluded(false));
                let weak = weak.clone();
                let state = state.clone();
                let queue = queue.clone();
                slint::Timer::single_shot(Duration::from_millis(20), move || {
                    if let Some(ui) = weak.upgrade() {
                        if invalidate {
                            state.borrow_mut().requested = None;
                            request_render(&ui, &state, &queue);
                        }
                        ui.window()
                            .with_winit_window(|window| window.request_redraw());
                    }
                });
            }
            if let Some(ui) = weak.upgrade() {
                if matches!(event, winit::event::WindowEvent::RedrawRequested)
                    && ui.get_turn_running()
                {
                    let now = Instant::now();
                    let mut s = state.borrow_mut();
                    s.motion_progress = ui.get_turn_progress();
                    if s.turn_progress_samples.len() < 240 {
                        let progress = s.motion_progress;
                        s.turn_progress_samples.push(progress);
                    }
                    if let Some(previous) = s.turn_last_redraw.replace(now)
                        && s.turn_redraw_intervals.len() < 240
                    {
                        s.turn_redraw_intervals
                            .push(now.duration_since(previous).as_secs_f64() * 1000.);
                    }
                    let generation = s.generation;
                    let prime = ui.get_turn_priming() && s.prime_scheduled != Some(generation);
                    if prime {
                        s.prime_scheduled = Some(generation);
                    }
                    let turn_ms = turn_queue::turn_ms(!s.turns.is_empty());
                    let keep_release_ms = s.drag_finish.is_some();
                    drop(s);
                    if prime {
                        if !keep_release_ms {
                            ui.set_turn_duration(turn_ms);
                        }
                        let weak = weak.clone();
                        let state = state.clone();
                        let queue = queue.clone();
                        // This event propagates to Slint's first paint with
                        // both textures installed and progress held at zero.
                        // Start motion on the following event-loop turn.
                        slint::Timer::single_shot(Duration::ZERO, move || {
                            if let Some(ui) = weak.upgrade()
                                && state.borrow().generation == generation
                                && ui.get_turn_running()
                                && ui.get_turn_priming()
                            {
                                state.borrow_mut().primed_turns += 1;
                                ui.set_turn_priming(false);
                                settle_turn(&ui, &state, &queue);
                            }
                        });
                    }
                }
                ui.window()
                    .with_winit_window(|window| ui.set_window_maximized(window.is_maximized()));
            }
            EventResult::Propagate
        });
    }
    let render_timer = Rc::new(slint::Timer::default());
    let search_timer = Rc::new(slint::Timer::default());
    {
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        let timer = render_timer.clone();
        ui.on_render_requested(move |_, _| {
            let weak = weak.clone();
            let state = state.clone();
            let queue = queue.clone();
            timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(65),
                move || {
                    if let Some(ui) = weak.upgrade() {
                        request_render(&ui, &state, &queue);
                    }
                },
            );
        });
    }
    {
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        ui.on_worker_ready(move || {
            if let Some(ui) = weak.upgrade() {
                // Each successful send schedules its own callback. Process
                // one result per turn so a fast stream of fold frames cannot
                // monopolize the UI event loop and starve painting or input.
                if let Ok(output) = rx.try_recv() {
                    queue.wake.notify_one();
                    apply_output(&ui, &state, &queue, output);
                }
            }
        });
    }
    {
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        ui.on_navigate(move |delta| {
            if let Some(ui) = weak.upgrade() {
                // A running turn is never cancelled or restarted. Later requests
                // wait for it, and a drag owns the page until it is released.
                if ui.get_render_busy() {
                    if !ui.get_drag_active() {
                        state.borrow_mut().turns.push(delta.clamp(-1, 1), false);
                    }
                    return;
                }
                if !ui.get_drag_active() {
                    ui.set_turn_top(false);
                    state.borrow_mut().drag_finish = None;
                }
                let page = state.borrow().page;
                let geometry = app_core::book_geometry(app_core::Rect {
                    x: 0.,
                    y: 4.,
                    w: if ui.get_single_page() {
                        ui.get_canvas_w().min(649.)
                    } else {
                        ui.get_canvas_w()
                    },
                    h: ui.get_canvas_h() - 8.,
                });
                let target = app_core::destination(page, delta.clamp(-1, 1), geometry.single);
                if target == page {
                    state.borrow_mut().turns.clear();
                }
                set_page_with_intent(&ui, &state, &queue, target, None, true);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.on_navigate_key(move |delta, repeated| {
            if let Some(ui) = weak.upgrade() {
                if !ui.get_render_busy() {
                    ui.invoke_navigate(delta);
                } else if !ui.get_drag_active() {
                    state.borrow_mut().turns.push(delta, repeated);
                }
            }
        });
    }
    {
        let state = state.clone();
        ui.on_navigate_key_released(move || state.borrow_mut().turns.release_repeats());
    }
    {
        let origin = Rc::new(std::cell::Cell::new(1u32));
        let weak = ui.as_weak();
        let state_begin = state.clone();
        let origin_begin = origin.clone();
        ui.on_begin_page_drag(move |direction| {
            if let Some(ui) = weak.upgrade()
                && !ui.get_render_busy()
            {
                let page = state_begin.borrow().page;
                let single = state_begin.borrow().last["singlePage"]
                    .as_bool()
                    .unwrap_or(false);
                if app_core::destination(page, direction, single) == page {
                    return;
                }
                origin_begin.set(page);
                state_begin.borrow_mut().drag_finish = None;
                ui.set_turn_single(single);
                ui.set_drag_progress(0.);
                ui.set_drag_canceling(false);
                ui.set_drag_active(true);
                ui.invoke_hover_left();
                ui.invoke_navigate(direction);
            }
        });
        let weak = ui.as_weak();
        let state = state.clone();
        let queue = queue.clone();
        ui.on_end_page_drag(move |commit| {
            if let Some(ui) = weak.upgrade()
                && ui.get_drag_active()
            {
                let _ = ui.get_turn_progress();
                let loaded = ui.get_turn_running();
                ui.set_turn_duration(turn_queue::release_ms(ui.get_drag_progress()));
                ui.set_drag_canceling(!commit && loaded);
                ui.set_drag_active(false);
                if commit && !loaded {
                    state.borrow_mut().drag_finish = Some((true, origin.get()));
                    return;
                }
                if !commit && !loaded {
                    set_page(&ui, &state, &queue, origin.get(), None);
                    return;
                }
                state.borrow_mut().drag_finish = Some((commit, origin.get()));
                settle_turn(&ui, &state, &queue);
            }
        });
    }
    {
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        ui.on_go_page(move |value| {
            if let Some(ui) = weak.upgrade() {
                if let Some(page) = page_number(&value) {
                    set_page(&ui, &state, &queue, page, None);
                } else {
                    ui.set_render_status(
                        if ui.get_arabic() {
                            "أدخل رقم صفحة من 1 إلى 604"
                        } else {
                            "Enter a page from 1 to 604"
                        }
                        .into(),
                    );
                }
            }
        });
    }
    {
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        let timer = search_timer.clone();
        ui.on_search(move |query| {
            let mut s = state.borrow_mut();
            s.search_id = s.search_id.wrapping_add(1).max(1);
            s.search_completed_id = None;
            s.hits.clear();
            let id = s.search_id;
            drop(s);
            if let Some(ui) = weak.upgrade() {
                ui.set_search_busy(true);
                ui.set_search_status(
                    if ui.get_arabic() {
                        "جارٍ البحث…"
                    } else {
                        "Searching…"
                    }
                    .into(),
                );
            }
            timer.stop();
            if query.len() > 1024 || query.chars().count() > 256 {
                if let Some(ui) = weak.upgrade() {
                    ui.set_search_busy(false);
                    ui.set_results(ModelRc::new(VecModel::from(Vec::<SearchEntry>::new())));
                    ui.set_search_status(
                        if ui.get_arabic() {
                            "يرجى تقصير عبارة البحث."
                        } else {
                            "Please shorten the search text."
                        }
                        .into(),
                    );
                }
                return;
            }
            let queue = queue.clone();

            let weak = weak.clone();
            let query = query.to_string();
            timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(130),
                move || {
                    if let Some(ui) = weak.upgrade() {
                        queue.push(Task::Search(
                            id,
                            query.clone(),
                            ui.get_arabic(),
                            ui.get_search_mode(),
                        ));
                    }
                },
            );
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_search_mode_changed(move |mode| {
            if let Some(ui) = weak.upgrade() {
                ui.set_search_mode(mode.clamp(0, 5));
                ui.invoke_search(ui.get_search_query());
            }
        });
    }
    {
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        ui.on_choose_result(move |index| {
            if let Some(ui) = weak.upgrade() {
                if ui.get_search_busy() {
                    return false;
                }
                let hit = usize::try_from(index)
                    .ok()
                    .and_then(|i| state.borrow().hits.get(i).cloned());
                if let Some(hit) = hit {
                    set_page(&ui, &state, &queue, hit.page, hit.key);
                    return true;
                }
            }
            false
        });
    }
    {
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        ui.on_pick_ayah(move |x, y| {
            if let Some(ui) = weak.upgrade()
                && !ui.get_render_busy()
            {
                queue.push(Task::Pick(
                    state.borrow().committed,
                    x - ui.get_image_x(),
                    y - ui.get_image_y(),
                    PickKind::Study,
                ));
            }
        });
    }
    let hover_timer = Rc::new(slint::Timer::default());
    let hover_pending: Rc<Cell<Option<(i32, f32, f32)>>> = Rc::new(Cell::new(None));
    {
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        let timer = hover_timer.clone();
        let pending = hover_pending.clone();
        ui.on_hover_moved(move |x, y| {
            if let Some(ui) = weak.upgrade() {
                if ui.get_render_busy() || ui.get_context_open() {
                    return;
                }
                let s = state.borrow();
                if !hover_view_matches(&ui, &s) {
                    return;
                }
                pending.set(Some((
                    s.committed,
                    x - ui.get_image_x(),
                    y - ui.get_image_y(),
                )));
                drop(s);
                if timer.running() {
                    return;
                }
                let weak = weak.clone();
                let state = state.clone();
                let queue = queue.clone();
                let pending = pending.clone();
                timer.start(
                    slint::TimerMode::SingleShot,
                    Duration::from_millis(16),
                    move || {
                        let Some((generation, x, y)) = pending.take() else {
                            return;
                        };
                        let Some(ui) = weak.upgrade() else {
                            return;
                        };
                        if ui.get_render_busy() || ui.get_context_open() {
                            return;
                        }
                        let mut s = state.borrow_mut();
                        if generation != s.committed || !hover_view_matches(&ui, &s) {
                            return;
                        }
                        s.hover_sequence = s.hover_sequence.wrapping_add(1);
                        queue.push(Task::Hover(generation, s.hover_sequence, x, y));
                    },
                );
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        let timer = hover_timer.clone();
        let pending = hover_pending.clone();
        ui.on_hover_left(move || {
            timer.stop();
            pending.set(None);
            if let Some(ui) = weak.upgrade() {
                let mut s = state.borrow_mut();
                s.hover_sequence = s.hover_sequence.wrapping_add(1);
                if ui.get_hover_rects().row_count() != 0 {
                    ui.set_hover_rects(ModelRc::new(VecModel::from(Vec::<HoverRect>::new())));
                }
            }
        });
    }
    controller.connect(&ui, &state, &queue);
    {
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        ui.on_close_study(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_study_closing(true);
                let weak = ui.as_weak();
                let state = state.clone();
                let queue = queue.clone();
                slint::Timer::single_shot(
                    Duration::from_millis(if ui.get_motion_enabled() { 170 } else { 0 }),
                    move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_study_closing(false);
                            if ui.get_panel() != "study" {
                                state.borrow_mut().selection = None;
                                ui.set_study_body("".into());
                            }
                            request_render(&ui, &state, &queue);
                        }
                    },
                );
                ui.invoke_focus_reading();
            }
        });
    }
    {
        let weak = about.as_weak();
        ui.on_about_requested(move || {
            if let Some(about) = weak.upgrade() {
                let _ = about.show();
            }
        });
    }
    {
        let weak = about.as_weak();
        ui.window().on_close_requested(move || {
            if let Some(about) = weak.upgrade() {
                let _ = about.hide();
            }
            let _ = slint::quit_event_loop();
            slint::CloseRequestResponse::HideWindow
        });
    }
    let failure = Rc::new(RefCell::new(None::<String>));
    let qa = qa_output.as_ref().map(|output| {
        Rc::new(RefCell::new(Qa {
            turns_before_study: 0,
            output: output.clone(),
            began: Instant::now(),
            index: 0,
            phase: 0,
            since: Instant::now(),
            records: Vec::new(),
            chrome_records: Vec::new(),
            unchanged: (0, 0, [0.; 8]),
            expected_selection: None,
            expected_search_id: 0,
            before_drag: None,
            software_rendering,
            maximized_frames: Vec::new(),
            maximized_turns: 0,
            maximized_start_page: 1,
            fast_start_progress: 0.,
            shaded_turns: 0,
            done: false,
        }))
    });
    let qa_timer = slint::Timer::default();
    if let Some(qa) = &qa {
        let qa = qa.clone();
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        let failure = failure.clone();
        qa_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(150),
            move || {
                if let Some(ui) = weak.upgrade() {
                    match qa.borrow_mut().step(&ui, &state, &queue) {
                        Ok(false) => {}
                        Ok(true) => {
                            let _ = slint::quit_event_loop();
                        }
                        Err(error) => {
                            *failure.borrow_mut() = Some(error.to_string());
                            let _ = slint::quit_event_loop();
                        }
                    }
                }
            },
        );
    }
    let player_qa = player_output
        .as_ref()
        .map(|out| Rc::new(RefCell::new(player_qa::PlayerQa::new(out.clone()))));
    if let Some(qa) = &player_qa {
        let qa = qa.clone();
        let state = state.clone();
        let queue = queue.clone();
        let weak = ui.as_weak();
        let failure = failure.clone();
        let controller = controller.clone();
        qa_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(80),
            move || {
                if let Some(ui) = weak.upgrade() {
                    match qa.borrow_mut().step(&ui, &state, &queue, &controller) {
                        Ok(false) => {}
                        Ok(true) => {
                            let _ = slint::quit_event_loop();
                        }
                        Err(error) => {
                            *failure.borrow_mut() = Some(error.to_string());
                            let _ = slint::quit_event_loop();
                        }
                    }
                }
            },
        );
    }
    ui.show()?;
    ui.window().with_winit_window(|window| {
        use winit::platform::windows::IconExtWindows;
        if let Ok(icon) = winit::window::Icon::from_resource(1, None) {
            window.set_window_icon(Some(icon));
        }
    });

    // The first render waits until the native window has its actual logical
    // dimensions and monitor scale. No pre-show physical resize is applied.
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let queue = queue.clone();
        render_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(65),
            move || {
                if let Some(ui) = weak.upgrade() {
                    request_render(&ui, &state, &queue);
                    ui.invoke_focus_reading();
                }
            },
        );
    }
    slint::run_event_loop()?;
    qa_timer.stop();
    controller.finish(&ui, &state);
    render_timer.stop();
    search_timer.stop();
    queue.stop();
    let _ = about.hide();
    let _ = ui.hide();
    if let Some(qa) = &qa {
        if !qa.borrow().done && failure.borrow().is_none() {
            *failure.borrow_mut() = Some("QA closed before completion".into());
        }
        if let Some(error) = failure.borrow().as_ref() {
            json_file(
                &qa.borrow().output.join("qa-failure.json"),
                &json!({"error":error,"records":qa.borrow().records}),
            )?;
        }
    }
    if let Some(qa) = &player_qa {
        if !qa.borrow().done && failure.borrow().is_none() {
            *failure.borrow_mut() = Some("Main player QA closed before completion".into());
        }
        if let Some(error) = failure.borrow().as_ref() {
            json_file(
                &qa.borrow()
                    .output_directory()
                    .join("player-qa-failure.json"),
                &json!({"error":error}),
            )?;
        }
    }
    // Dropping UI drops the output receiver callback before joining its worker.
    drop(ui);
    drop(about);
    drop(state);
    drop(worker);
    if let Some(error) = failure.borrow().as_ref() {
        return Err(error.clone().into());
    }
    Ok(())
}
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if let Err(error) = gpu_choice::apply(&args) {
        eprintln!("Reading prototype failed: {error}");
        std::process::exit(1);
    }
    if let Err(error) = run() {
        eprintln!("Reading prototype failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_pixels_outside_the_rounded_frame_become_transparent() {
        let page = slint::Rgba8Pixel::new(250, 240, 220, 255);
        let mut buffer = SharedPixelBuffer::<slint::Rgba8Pixel>::new(200, 100);
        buffer.make_mut_slice().iter_mut().for_each(|p| *p = page);
        // A 1:1 raster with a frame inset by 10 pixels.
        clear_outside_frame(&mut buffer, 200., 100., [10., 10., 180., 80.]);
        let at = |x: usize, y: usize| buffer.as_slice()[y * 200 + x];
        assert_eq!(at(100, 50), page, "the page itself is unchanged");
        assert_eq!(at(10, 50), page, "the frame's straight edge stays opaque");
        assert_eq!(at(5, 50).a, 0, "the margin is cleared");
        assert_eq!(at(100, 95).a, 0, "the margin below is cleared");
        assert_eq!(
            at(10, 10).a,
            0,
            "the square corner outside the rounded frame is cleared"
        );
        assert_eq!(at(22, 22), page, "inside the corner's curve stays opaque");
        let edge = (0..12)
            .map(|i| at(10 + i, 12).a)
            .find(|a| *a > 0 && *a < 255);
        assert!(edge.is_some(), "the rounded edge is antialiased");
        let opaque = buffer.as_slice().iter().filter(|p| **p == page).count();
        assert!(opaque > 180 * 80 - 4 * 40);
    }
    #[test]
    fn page_input_accepts_local_digits_and_checks_bounds() {
        assert_eq!(page_number(" ٢٥٥ "), Some(255));
        assert_eq!(page_number("۶۰۴"), Some(604));
        assert_eq!(page_number("0"), None);
        assert_eq!(page_number("605"), None);
        assert_eq!(page_number("-1"), None);
    }
    #[test]
    fn render_slot_keeps_only_latest_request() {
        let q = Queue::default();
        let v = View {
            page: 1,
            width: 1000.,
            height: 700.,
            scale: 1.,
            dark: true,
            arabic: false,
            selection: None,
            style: reader::RenderStyle::default(),
            clock_minute: 0,
        };
        for generation in 1..100 {
            q.push(Task::Render(RenderRequest {
                motion: false,
                navigation: false,
                from_page: 0,
                generation,
                view: v,
            }));
        }
        match q.next() {
            Some(Task::Render(r)) => assert_eq!(r.generation, 99),
            _ => panic!("Expected latest render"),
        };
        q.stop();
        assert!(q.next().is_none());
    }
}
