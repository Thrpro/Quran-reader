//! Native controls retain Windows keyboard navigation and accessibility. All
//! window state belongs to the UI thread; media callbacks do not borrow it.
#[cfg(feature = "qa")]
#[path = "enhancements21_qa.rs"]
mod enhancements21_qa;
#[cfg(feature = "qa")]
#[path = "enhancements22_qa.rs"]
mod enhancements22_qa;
#[cfg(feature = "qa")]
#[path = "enhancements23_qa.rs"]
mod enhancements23_qa;
#[cfg(feature = "qa")]
#[path = "enhancements25_qa.rs"]
mod enhancements25_qa;
#[cfg(feature = "qa")]
#[path = "enhancements26_qa.rs"]
mod enhancements26_qa;
#[cfg(feature = "qa")]
#[path = "enhancements27_qa.rs"]
mod enhancements27_qa;
#[cfg(feature = "qa")]
#[path = "enhancements28_qa.rs"]
mod enhancements28_qa;
#[cfg(feature = "qa")]
#[path = "enhancements29_qa.rs"]
mod enhancements29_qa;
#[cfg(feature = "qa")]
#[path = "experience_qa.rs"]
mod experience_qa;
#[path = "guided_ui.rs"]
mod guide;
#[cfg(feature = "qa")]
#[path = "listening20_qa.rs"]
mod listening20_qa;
#[path = "listening_ui.rs"]
mod listening_ui;
#[cfg(feature = "qa")]
#[path = "opening26_qa.rs"]
mod opening26_qa;
#[path = "popup_frame.rs"]
mod popup_frame;
#[path = "reading_flow_ui.rs"]
mod reading_flow_ui;
#[path = "storage_ui.rs"]
mod storage_ui;
#[path = "study_ui.rs"]
mod study_ui;
#[path = "transport_ui.rs"]
mod transport_ui;
use crate::{
    audio::{Audio, ListeningRange, RECITERS},
    audio_cache::{AudioCache, Event as DownloadEvent, Job, StorageStats},
    data::{self, Preferences, Resources, Result, SearchHit, SearchKind},
    reader_state::{self, GoalMode, ReaderState},
    render::{BookView, Palette, Rect, Renderer},
    tafsir_library::{self, Event as TafsirEvent, Job as TafsirJob, Library as TafsirLibrary},
};
use std::{
    cell::Cell, collections::BTreeMap, ffi::c_void, path::PathBuf, sync::Arc, time::Instant,
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Dwm::*, Gdi::*},
        System::{LibraryLoader::*, SystemInformation::GetLocalTime},
        UI::{
            Controls::RichEdit::*,
            Controls::{
                COMBOBOXINFO, DRAWITEMSTRUCT, EM_GETRECT, EM_POSFROMCHAR, EM_SCROLL, EM_SETRECT,
                EM_SETSEL, GetComboBoxInfo, ICC_BAR_CLASSES, INITCOMMONCONTROLSEX,
                InitCommonControlsEx, ODS_FOCUS, ODS_SELECTED, SetScrollInfo, SetWindowTheme,
                ShowScrollBar, TBM_SETPOS, TBM_SETRANGE, TBS_NOTICKS,
            },
            HiDpi::*,
            Input::KeyboardAndMouse::*,
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
    core::{BOOL, HSTRING, w},
};

// Synchronous Win32 control notifications can re-enter the parent procedure.
// Gate them before taking a mutable App reference, including calls from the
// message loop. This prevents aliased mutable state during SetWindowText etc.
thread_local! { static DISPATCHING:Cell<bool>=const{Cell::new(false)}; }
thread_local! { static QUERY_COMPOSING:Cell<bool>=const{Cell::new(false)}; }
// Nonclient hit testing can arrive synchronously during a guarded layout.
// Read a separate copied flag, rather than App while it is mutably borrowed.
thread_local! { static FULLSCREEN_STATE:Cell<bool>=const{Cell::new(false)}; }
thread_local! { static RESULT_WHEEL:std::cell::RefCell<crate::list_scroll::WheelRows>=const{std::cell::RefCell::new(crate::list_scroll::WheelRows::new())}; }
/// STATIC/EDIT controls ask for colors synchronously while App is already
/// borrowed. Copy just the GDI values into a separate UI-thread cache so that
/// these callbacks never need to borrow App. App still owns the brush.
#[derive(Clone, Copy)]
struct ControlColors {
    brush: usize,
    ink: u32,
    background: u32,
}
impl ControlColors {
    const EMPTY: Self = Self {
        brush: 0,
        ink: 0,
        background: 0,
    };
}
thread_local! { static CONTROL_COLORS:Cell<ControlColors>=const{Cell::new(ControlColors::EMPTY)}; }
fn cache_control_colors(brush: HBRUSH, palette: Palette) {
    crate::field_style::cache_palette(palette);
    CONTROL_COLORS.with(|colors| {
        colors.set(ControlColors {
            brush: brush.0 as usize,
            ink: palette.ink_rgb(),
            background: palette.background_rgb(),
        })
    });
}
fn qa_disabled(flag: &str) -> bool {
    #[cfg(feature = "qa")]
    {
        std::env::args().any(|arg| arg == flag)
    }
    #[cfg(not(feature = "qa"))]
    {
        let _ = flag;
        false
    }
}
fn client_animation_enabled() -> bool {
    let mut enabled = BOOL::from(true);
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut enabled as *mut BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    enabled.as_bool()
}
/// SetFocus returns the previous focus handle, which can legitimately be null.
/// Check the resulting focus before propagating the binding's null-handle error;
/// a disabled or otherwise unavailable target still fails this check.
unsafe fn focus_window(hwnd: HWND) -> Result<()> {
    unsafe {
        let result = SetFocus(Some(hwnd));
        if GetFocus() == hwnd {
            return Ok(());
        }
        result?;
        Err("Keyboard focus did not reach the requested native control".into())
    }
}
struct DispatchGuard;
impl DispatchGuard {
    fn enter() -> Option<Self> {
        DISPATCHING.with(|busy| if busy.replace(true) { None } else { Some(Self) })
    }
}
impl Drop for DispatchGuard {
    fn drop(&mut self) {
        DISPATCHING.with(|busy| busy.set(false));
    }
}

const IDENTITY_ICON: u32 = 175;
const NOTICE: u32 = 176;
const MENU_ROW: u32 = 500;
const MENU_BACK: u32 = 540;
const MENU_CLOSE: u32 = 541;
const MINIMIZE: u32 = 170;
const MAXIMIZE: u32 = 171;
const WINDOW_CLOSE: u32 = 172;
const REPAIR_LAYOUT: u32 = WM_APP + 20;
const FRAME_TICK: u32 = WM_APP + 21;
#[cfg(feature = "qa")]
const QA_HEARTBEAT: u32 = WM_APP + 22;
const TBM_GETPOS: u32 = WM_USER;
const NATIVE_MOUSE_LEAVE: u32 = 0x02a3;
const SEARCH: u32 = 101;
const LISTEN: u32 = 102;
const SETTINGS: u32 = 103;
const FULLSCREEN: u32 = 104;
const PREV: u32 = 105;
const NEXT: u32 = 106;
const QUERY: u32 = 107;
const GO: u32 = 108;
const RESULTS: u32 = 109;
const CLOSE: u32 = 110;
const SOURCE: u32 = 111;
const STUDY: u32 = 112;
const AYAH_PREV: u32 = 113;
const AYAH_NEXT: u32 = 114;
const SEARCH_CLOSE: u32 = 115;
const BROWSE_SURAH: u32 = 180;
const BROWSE_PAGE: u32 = 181;
const BROWSE_SECTIONS: u32 = 182;
const BROWSE_QUERY: u32 = 183;
const BROWSE_KIND: u32 = 184;
const RESULT_PICK: u32 = 185;
const RECITER: u32 = 120;
const CHAPTER: u32 = 121;
const FIRST: u32 = 122;
const LAST: u32 = 123;
const PLAY: u32 = 124;
const PAUSE: u32 = 125;
const STOP: u32 = 126;
const SPEED: u32 = 127;
const REPEAT: u32 = 128;
const VOLUME: u32 = 129;
const STATUS: u32 = 130;
const DAILY: u32 = 131;
const QUICK_PAUSE: u32 = 132;
const QUICK_STOP: u32 = 133;
const FOLLOW_AUDIO: u32 = 134;
const QUICK_SETTINGS: u32 = 135;
const AUDIO_POSITION: u32 = 136;
const AUDIO_TIME: u32 = 137;
const SEEK_BACK: u32 = 138;
const SEEK_FORWARD: u32 = 139;
const SOURCE_LABEL: u32 = 140;
const RECITER_LABEL: u32 = 141;
const CHAPTER_LABEL: u32 = 142;
const FIRST_LABEL: u32 = 143;
const LAST_LABEL: u32 = 144;
const SPEED_LABEL: u32 = 145;
const VOLUME_LABEL: u32 = 146;
const TEXT_VIEW: u32 = 147;
const REPEAT_LABEL: u32 = 149;
const STUDY_FOLLOW: u32 = 330;
const STUDY_OPTIONS: u32 = 331;
const ENABLE_TEXT_VIEW: u32 = 332;
const EXIT_TEXT_VIEW: u32 = 333;
const REMEMBER_SPEED: u32 = 334;
const CANCEL_TEXT_VIEW: u32 = 335;
const DOWNLOAD: u32 = 150;
const DOWNLOAD_STATUS: u32 = 151;
const STORAGE: u32 = 152;
const MANAGE_TAFSIR: u32 = 160;
const TAFSIR_INFO: u32 = 165;
const TAFSIR_ACTION_FIRST: u32 = 260;
const BOOKMARK: u32 = 230;
const BOOKMARKS: u32 = 231;
const ACCESSIBLE: u32 = 232;
const HELP: u32 = 233;
const CLOCK: u32 = 234;
const CLOCK_24: u32 = 235;
const TIMER: u32 = 236;
const REMINDER: u32 = 237;
const CONTINUOUS_AUDIO: u32 = 238;
const COLLAPSE_PLAYER: u32 = 255;
const RECORDING_MODE: u32 = 256;
const SEARCH_KEY_T: u32 = 239;
const OTHER_STUDY_LANGUAGES: u32 = 244;
const OFFLINE_LIMIT_FIRST: u32 = 280;
const GUIDE_BACK: u32 = 270;
const GUIDE_NEXT: u32 = 271;
const GUIDE_CLOSE: u32 = 272;
const GUIDE_SHOW: u32 = 273;
const GUIDE_BODY: u32 = 274;
const WELCOME_ENGLISH: u32 = 245;
const WELCOME_ARABIC: u32 = 246;
const LISTEN_THIS: u32 = 248;
const LISTEN_PAGE: u32 = 249;
const LISTEN_SURAH: u32 = 250;
const LISTEN_QURAN: u32 = 251;
const STUDY_ATTRIBUTION: u32 = 247;
const GOAL: u32 = 240;
const BOOKMARK_FIRST: u32 = 900;
const LANG: u32 = 200;
const THEME: u32 = 210;
const THEME_SELECTOR: u32 = 215;
const THEME_LABEL: u32 = 216;
const TEXT_SMALL: u32 = 220;
const TEXT_LARGE: u32 = 221;
const TEXT_SIZE: u32 = 252;
const SOURCE_SINGLE: u32 = 253;
const TAFSIR_ENGLISH: u32 = 254;
const TAFSIR_ARABIC: u32 = 257;
const STUDY_SIZE_FIRST: u32 = 290;
const CONTEXTUAL: u32 = 222;
#[derive(PartialEq, Clone, Copy)]
enum Panel {
    None,
    Study,
    Listen,
    Storage,
    Tafsirs,
}
/// Keep the compact listening form above the status line on small windows.
/// Native combo heights include their dropdown; only their collapsed field is
/// visible here. Compress spacing rather than shrinking the UI font.
use crate::listening_layout::ListenLayout;
#[cfg(any(test, feature = "qa"))]
fn fit_window_rect(original: RECT, work: RECT) -> RECT {
    let width = (original.right - original.left).clamp(1, (work.right - work.left).max(1));
    let height = (original.bottom - original.top).clamp(1, (work.bottom - work.top).max(1));
    let left = original.left.clamp(work.left, work.right - width);
    let top = original.top.clamp(work.top, work.bottom - height);
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}
/// Created after App so this is dropped first on every exit, including errors.
/// The HWND must never retain a pointer to a freed Box<App>.
struct WindowLifetime(HWND);
impl Drop for WindowLifetime {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.0)).as_bool() {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}
/// Always end a Direct2D frame, including when loading a page fails midway.
struct DrawSession {
    target: windows::Win32::Graphics::Direct2D::ID2D1RenderTarget,
    finished: bool,
}
impl DrawSession {
    fn begin(target: windows::Win32::Graphics::Direct2D::ID2D1RenderTarget) -> Self {
        unsafe {
            target.BeginDraw();
        }
        Self {
            target,
            finished: false,
        }
    }
    fn finish(&mut self) -> windows::core::Result<()> {
        self.finished = true;
        unsafe { self.target.EndDraw(None, None) }
    }
}
impl Drop for DrawSession {
    fn drop(&mut self) {
        if !self.finished {
            unsafe {
                let _ = self.target.EndDraw(None, None);
            }
        }
    }
}
#[derive(Clone)]
struct MenuRow {
    id: u32,
    label: String,
    detail: String,
    children: Vec<MenuRow>,
}

fn menu_check(row: &MenuRow) -> Option<bool> {
    matches!(
        row.id,
        STUDY_FOLLOW
            | CONTEXTUAL
            | CLOCK
            | CLOCK_24
            | ACCESSIBLE
            | OTHER_STUDY_LANGUAGES
            | SEARCH_KEY_T
            | CONTINUOUS_AUDIO
            | REMEMBER_SPEED
            | TIMER
            | REMINDER
    )
    .then(|| row.label.starts_with('✓'))
}
struct Popup {
    title: String,
    rows: Vec<MenuRow>,
    stack: Vec<(String, Vec<MenuRow>)>,
    rect: Rect,
    context: bool,
    context_target: Option<(u32, (u32, u32))>,
    listening_expanded: bool,
    docked: bool,
    rtl: bool,
    motion: Option<Instant>,
    frame_pose: Option<(Instant, Rect)>,
}
struct ScopedListening {
    session: crate::app_core::ListeningSession,
    reciter: usize,
    continuous: bool,
    rolling: bool,
    paused: bool,
    start: (u32, u32),
    end: (u32, u32),
    scope: crate::app_core::ListeningScope,
    page: u32,
    label: String,
    label_arabic: bool,
}
impl Popup {
    fn display_rect(&self) -> Rect {
        if let Some((start, rect)) = self
            .frame_pose
            .filter(|(start, _)| self.motion == Some(*start))
        {
            let _ = start;
            return rect;
        }
        let mut rect = self.rect;
        let t = self
            .motion
            .map_or(1., |start| (start.elapsed().as_secs_f32() / 0.18).min(1.));
        let remaining = 1. - t * t * (3. - 2. * t);
        if self.docked {
            rect.x += if self.rtl { 28. } else { -28. } * remaining;
        }
        rect
    }
    fn contains(&self, x: f32, y: f32) -> bool {
        let rect = self.display_rect();
        x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h
    }
}
// Convert the existing command catalogue into modeless native buttons. No
// TrackPopupMenu modal loop can suspend the app's download/playback polling.
unsafe fn menu_rows(menu: HMENU) -> Vec<MenuRow> {
    unsafe {
        let mut rows = Vec::new();
        for n in 0..GetMenuItemCount(Some(menu)) {
            let child = GetSubMenu(menu, n);
            // For submenus GetMenuState packs the child count into the high
            // byte. Eight entries otherwise look like MF_SEPARATOR (0x800).
            let state = GetMenuState(menu, n as u32, MF_BYPOSITION)
                & if child.0.is_null() { u32::MAX } else { 0xff };
            if state & MF_SEPARATOR.0 != 0 {
                continue;
            }
            let mut buffer = [0u16; 512];
            let len = GetMenuStringW(menu, n as u32, Some(&mut buffer), MF_BYPOSITION);
            let mut label = String::from_utf16_lossy(&buffer[..len.max(0) as usize]);
            if state & MF_CHECKED.0 != 0 {
                label = format!("✓  {label}");
            }
            rows.push(MenuRow {
                id: GetMenuItemID(menu, n),
                label,
                detail: String::new(),
                children: if child.0.is_null() {
                    vec![]
                } else {
                    menu_rows(child)
                },
            });
        }
        rows
    }
}
#[derive(Clone, PartialEq, Eq)]
struct MotionKey {
    page: u32,
    theme: u32,
    arabic: bool,
    contextual: bool,
    selected: Option<(u32, u32)>,
    recitation: Option<(u32, u32)>,
    opening: Option<u32>,
    dpi: u32,
    width: u32,
    height: u32,
    clock: u16,
    clock_flags: (bool, bool),
    generation: u64,
    geometry: [u32; 4],
}
struct MotionBook {
    key: MotionKey,
    raster: crate::render::BookRaster,
    bitmap: windows::Win32::Graphics::Direct2D::ID2D1Bitmap,
}
struct MotionFold {
    key: MotionKey,
    old: u32,
    source: crate::render::BookRaster,
    destination: crate::render::BookRaster,
    reverse: crate::render::BookRaster,
    source_bitmap: windows::Win32::Graphics::Direct2D::ID2D1Bitmap,
    destination_bitmap: windows::Win32::Graphics::Direct2D::ID2D1Bitmap,
    reverse_bitmap: windows::Win32::Graphics::Direct2D::ID2D1Bitmap,
}
#[derive(PartialEq, Eq)]
struct MotionControlKey {
    width: i32,
    height: i32,
    dpi: u32,
    theme: u32,
    palette: [[u32; 4]; 6],
    rtl: bool,
    text: String,
    enabled: bool,
    focused: bool,
    keyboard_cues: bool,
    hot: bool,
    state: usize,
    context: (bool, bool, u32, bool),
    results: Option<(u64, isize, isize)>,
    reading: Option<(i32, i32, isize, i32, i32, isize)>,
}
struct App {
    guide: Option<guide::State>,
    hwnd: HWND,
    resources: Resources,
    renderer: Renderer,
    chrome: crate::chrome::Chrome,
    popup: Option<Popup>,
    prefs: Preferences,
    profile: PathBuf,
    controls: BTreeMap<u32, HWND>,
    placements: std::cell::RefCell<BTreeMap<u32, (Rect, bool)>>,
    placed: std::cell::RefCell<std::collections::BTreeSet<u32>>,
    position_batch: Cell<Option<HDWP>>,
    position_batch_failed: Cell<bool>,
    frame_paint_pending: bool,
    page_turn: Option<crate::page_turn::Turn>,
    turn_queue: crate::app_core::TurnQueue,
    repeating_key: bool,
    keyboard_turn: bool,
    tafsir_arabic: bool,
    prewarm_key: Option<(u32, bool, bool)>,
    prewarm_pages: std::collections::VecDeque<u32>,
    page_drag: Option<crate::page_turn::Drag>,
    drag_start: Option<(f32, f32)>,
    identity_icon: HICON,
    font: HFONT,
    study_font: HFONT,
    study_font_key: Option<(i32, u32)>,
    attribution_font: HFONT,
    #[cfg(feature = "qa")]
    transport_fields_in_frame: bool,
    transport_font: Cell<HFONT>,
    transport_font_dpi: Cell<u32>,
    attribution_font_dpi: u32,
    study_format_rect: Cell<Option<RECT>>,
    quran_study_font: Option<crate::reading_control::PrivateFont>,
    verse_text_end: u32,
    study_attribution_start: u32,
    study_control_dark: Option<bool>,
    background: HBRUSH,
    dpi: f32,
    width: f32,
    height: f32,
    panel: Panel,
    panel_animation: Option<(Instant, Panel, f32)>,
    study_reveal_pending: bool,
    speed_selection_pending: bool,
    motion_book: Option<MotionBook>,
    motion_fold: Option<MotionFold>,
    motion_failed: Option<(u32, u32, u32, u32, bool)>,
    motion_cpu: crate::motion_cache::CpuRasterCache,
    last_motion_direction: i32,
    motion_controls:
        std::cell::RefCell<BTreeMap<u32, (MotionControlKey, crate::motion_controls::GdiSnapshot)>>,
    #[cfg(feature = "qa")]
    motion_samples: [f64; 3],
    #[cfg(feature = "qa")]
    motion_cache_preparations: Vec<f64>,
    #[cfg(feature = "qa")]
    motion_cache_frames: u32,
    #[cfg(feature = "qa")]
    motion_errors: Vec<String>,
    #[cfg(feature = "qa")]
    popup_fields_in_frame: bool,
    #[cfg(feature = "qa")]
    motion_trace: Option<(Instant, Vec<[f64; 7]>)>,
    #[cfg(feature = "qa")]
    motion_live_runs: Vec<serde_json::Value>,
    #[cfg(feature = "qa")]
    last_layout_ms: f64,
    #[cfg(feature = "qa")]
    last_commit_ms: f64,
    #[cfg(feature = "qa")]
    layout_generation: u64,
    #[cfg(feature = "qa")]
    painted_layout_generation: u64,
    #[cfg(feature = "qa")]
    maximum_motion_pixel_bytes: u64,
    #[cfg(feature = "qa")]
    panel_pose: Option<f32>,
    search: bool,
    browse_tab: u32,
    search_animation: Option<Instant>,
    title_animation: Option<(Instant, f32)>,
    #[cfg(feature = "qa")]
    qa_menu_hot: Option<HWND>,
    keyboard_focus: bool,
    player_collapsed: bool,
    opening_highlight: Option<u32>,
    tooltips: Option<crate::tooltips::Tooltips>,
    chrome_failures: Cell<u32>,
    #[cfg(feature = "qa")]
    canvas_paints: u64,
    #[cfg(feature = "qa")]
    title_trace: Option<(Instant, Vec<(f64, f64)>)>,
    hits: Vec<SearchHit>,
    results_generation: u64,
    selected: Option<(u32, u32)>,
    hovered: Option<(u32, u32)>,
    search_reveal: Option<(Instant, (u32, u32))>,
    playback: crate::app_core::PlaybackFollow,
    repeats: crate::app_core::RepeatCount,
    active_range: Option<(ListeningRange, bool)>,
    listening_session: Option<ScopedListening>,
    audio: Option<Audio>,
    audio_cache: Option<Arc<AudioCache>>,
    download: Option<Job>,
    download_generation: u64,
    storage_pending: Option<std::sync::mpsc::Receiver<Result<StorageStats>>>,
    tafsir_library: Option<Arc<TafsirLibrary>>,
    tafsir_job: Option<TafsirJob>,
    tafsir_job_source: Option<usize>,
    tafsir_generation: u64,
    installed_tafsirs: Vec<usize>,
    study_sources: Vec<usize>,
    reader: ReaderState,
    reader_dirty: bool,
    last_reader_save: Instant,
    last_preferences_save: Instant,
    last_input: Instant,
    reading_page: u32,
    reading_pages: Vec<u32>,
    poll_interval: u32,
    text_view: bool,
    text_pages: Vec<u32>,
    panel_scroll: f32,
    listen_scroll_geometry: Option<(i32, u32, i32)>,
    clock_minute: u16,
    fullscreen: bool,
    placement: WINDOWPLACEMENT,
    window_style: isize,
    theme_start: Option<(Instant, Palette)>,
    // Publish one color sample for the parent, native controls and vector
    // decorations. Child paints must not each advance the animation clock.
    theme_frame: Option<Palette>,
    source: usize,
    arabic_source: usize,
    reciter: usize,
    study_size: i32,
    dirty: bool,
    daily: Option<Instant>,
    first_paint: bool,
    startup_reveal: Option<Instant>,
    notice_until: Option<Instant>,
    #[cfg(feature = "qa")]
    began: Instant,
    #[cfg(feature = "qa")]
    qa_step: u32,
    #[cfg(feature = "qa")]
    first_paint_ms: f64,
    #[cfg(feature = "qa")]
    qa_context: Option<(u32, u32)>,
    #[cfg(feature = "qa")]
    qa_original_rect: RECT,
    #[cfg(feature = "qa")]
    large_handoff: Option<crate::large_handoff_qa::State>,
}
impl Drop for App {
    fn drop(&mut self) {
        // Cached hit maps own DirectWrite layouts. Release them before the
        // renderer unregisters its authentic private font collections.
        self.tooltips.take();
        self.motion_book = None;
        self.motion_fold = None;
        self.motion_cpu.clear();
        self.motion_controls.get_mut().clear();
        unsafe {
            if !self.identity_icon.is_invalid() {
                DestroyIcon(self.identity_icon).ok();
            }
        }
        self.audio = None;
        self.download = None;
        self.tafsir_job = None;
        if self.reader_dirty {
            let _ = self.reader.save(&self.profile);
        }
        CONTROL_COLORS.with(|colors| {
            if colors.get().brush == self.background.0 as usize {
                colors.set(ControlColors::EMPTY);
            }
        });
        unsafe {
            let _ = DeleteObject(self.transport_font.get().into());
            let _ = DeleteObject(self.font.into());
            let _ = DeleteObject(self.study_font.into());
            let _ = DeleteObject(self.attribution_font.into());
            let _ = DeleteObject(self.background.into());
        }
    }
}
fn local_day() -> (String, u16, u16) {
    let now = unsafe { GetLocalTime() };
    (
        format!("{:04}-{:02}-{:02}", now.wYear, now.wMonth, now.wDay),
        now.wHour,
        now.wMinute,
    )
}
impl App {
    fn tr<'a>(&self, en: &'a str, ar: &'a str) -> &'a str {
        if self.prefs.arabic { ar } else { en }
    }
    unsafe fn child(&mut self, id: u32, class: &str, label: &str, style: u32) -> Result<HWND> {
        unsafe {
            let handle = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                &HSTRING::from(class),
                &HSTRING::from(label),
                WS_CHILD
                    | if class == "STATIC" {
                        WINDOW_STYLE(0)
                    } else {
                        WS_TABSTOP
                    }
                    | WINDOW_STYLE(style),
                0,
                0,
                1,
                1,
                Some(self.hwnd),
                Some(HMENU(id as usize as *mut c_void)),
                None,
                None,
            )?;
            SendMessageW(
                handle,
                WM_SETFONT,
                Some(WPARAM(self.font.0 as usize)),
                Some(LPARAM(0)),
            );
            self.controls.insert(id, handle);
            if matches!(id, QUERY | BROWSE_QUERY | FIRST | LAST)
                && !SetWindowSubclass(handle, Some(query_procedure), 5, 0).as_bool()
            {
                return Err(windows::core::Error::from_thread().into());
            }
            if class == "BUTTON"
                && style == BS_OWNERDRAW as u32
                && !SetWindowSubclass(handle, Some(button_procedure), 1, id as usize).as_bool()
            {
                return Err(windows::core::Error::from_thread().into());
            }
            // Use our palette with standard native controls instead of a light
            // themed combo/checkbox inside a dark reader.
            SetWindowTheme(handle, w!(""), w!(""))?;
            crate::field_style::apply_field_metrics(handle, self.dpi);
            if class == "COMBOBOX"
                && !SetWindowSubclass(handle, Some(crate::combo_control::procedure), 3, 0).as_bool()
            {
                return Err(windows::core::Error::from_thread().into());
            }
            Ok(handle)
        }
    }
    unsafe fn controls(&mut self) -> Result<()> {
        unsafe {
            for id in (MENU_ROW..MENU_ROW + 32).chain([MENU_BACK, MENU_CLOSE]) {
                self.child(id, "BUTTON", "", BS_OWNERDRAW as u32)?;
            }
            self.child(THEME_LABEL, "STATIC", "", 0)?;
            self.child(THEME_SELECTOR, "COMBOBOX", "", CBS_DROPDOWNLIST as u32)?;
            for (id, en, ar) in [
                (MINIMIZE, "Minimize", "تصغير"),
                (MAXIMIZE, "Maximize or restore", "تكبير أو استعادة"),
                (WINDOW_CLOSE, "Close", "إغلاق"),
                (SEARCH, "Search", "بحث"),
                (LISTEN, "Listen", "استماع"),
                (SETTINGS, "Options", "خيارات"),
                (FULLSCREEN, "Full screen", "ملء الشاشة"),
                (DAILY, "Start reading", "بدء القراءة"),
                (PREV, "‹", "‹"),
                (NEXT, "›", "›"),
                (GO, "Go", "انتقال"),
                (SEARCH_CLOSE, "Close search", "إغلاق البحث"),
                (CLOSE, "Close", "إغلاق"),
                (
                    EXIT_TEXT_VIEW,
                    "Return to Quran pages",
                    "العودة إلى صفحات المصحف",
                ),
                (AYAH_PREV, "Previous ayah", "الآية السابقة"),
                (AYAH_NEXT, "Next ayah", "الآية التالية"),
                (TEXT_SMALL, "Smaller text", "تصغير النص"),
                (TEXT_LARGE, "Larger text", "تكبير النص"),
                (PLAY, "Play range", "تشغيل النطاق"),
                (PAUSE, "Pause / Resume", "إيقاف مؤقت / متابعة"),
                (
                    QUICK_PAUSE,
                    "Pause / Resume · Space",
                    "إيقاف مؤقت / متابعة · مسافة",
                ),
                (QUICK_STOP, "End recitation", "إنهاء التلاوة"),
                (FOLLOW_AUDIO, "Go to current ayah", "انتقل إلى الآية الحالية"),
                (
                    COLLAPSE_PLAYER,
                    "Collapse or expand player",
                    "طي مشغل التلاوة أو توسيعه",
                ),
                (SEEK_BACK, "Back 10 seconds", "رجوع 10 ثوانٍ"),
                (SEEK_FORWARD, "Forward 10 seconds", "تقديم 10 ثوانٍ"),
                (QUICK_SETTINGS, "Listening settings", "إعدادات الاستماع"),
                (STOP, "Stop", "إيقاف"),
                (DOWNLOAD, "Download whole surah", "تنزيل السورة كاملة"),
                (STORAGE, "Details", "تفاصيل"),
                (MANAGE_TAFSIR, "Manage tafsirs", "إدارة التفاسير"),
            ] {
                let label = self.tr(en, ar).to_owned();
                self.child(id, "BUTTON", &label, BS_OWNERDRAW as u32)?;
            }
            self.child(IDENTITY_ICON, "STATIC", "", 3)?;
            self.refresh_icon();
            self.child(QUERY, "EDIT", "", ES_AUTOHSCROLL as u32)?;
            SendMessageW(
                self.controls[&QUERY],
                windows::Win32::UI::Controls::EM_SETLIMITTEXT,
                Some(WPARAM(512)),
                None,
            );
            for (id, label) in [
                (BROWSE_SURAH, "Surah"),
                (BROWSE_PAGE, "Page"),
                (BROWSE_SECTIONS, "Juz, Hizb & Rub"),
            ] {
                self.child(id, "BUTTON", label, BS_OWNERDRAW as u32)?;
            }
            self.child(BROWSE_QUERY, "EDIT", "", ES_AUTOHSCROLL as u32)?;
            SendMessageW(
                self.controls[&BROWSE_QUERY],
                windows::Win32::UI::Controls::EM_SETLIMITTEXT,
                Some(WPARAM(128)),
                None,
            );
            self.child(
                BROWSE_KIND,
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0,
            )?;
            self.fill(BROWSE_KIND, &["Juz".into(), "Hizb".into(), "Rub".into()], 0);
            self.child(
                RESULTS,
                "LISTBOX",
                "",
                LBS_NOTIFY as u32
                    | LBS_OWNERDRAWFIXED as u32
                    | LBS_HASSTRINGS as u32
                    | LBS_NOINTEGRALHEIGHT as u32
                    | WS_VSCROLL.0,
            )?;
            self.child(SOURCE_LABEL, "STATIC", "", 0)?;
            self.child(SOURCE_SINGLE, "STATIC", "", 0)?;
            for (id, text) in [
                (TEXT_SIZE, "Text size"),
                (STUDY_FOLLOW, "Follow recitation in Tafsir"),
                (STUDY_OPTIONS, "Reading options"),
                (TAFSIR_ENGLISH, "English"),
                (TAFSIR_ARABIC, "العربية"),
            ] {
                self.child(id, "BUTTON", text, BS_OWNERDRAW as u32)?;
            }
            self.child(STUDY_ATTRIBUTION, "STATIC", "", 0)?;
            self.child(
                SOURCE,
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0,
            )?;
            // Windows RichEdit provides accessible selection, proper paragraph
            // spacing and smooth scrolling without changing source content.
            LoadLibraryExW(w!("Msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)?;
            self.child(
                STUDY,
                "RICHEDIT50W",
                "",
                ES_MULTILINE as u32 | ES_READONLY as u32 | ES_AUTOVSCROLL as u32 | WS_VSCROLL.0,
            )?;
            if !SetWindowSubclass(self.controls[&RESULTS], Some(result_procedure), 6, 0).as_bool() {
                return Err("Unable to configure result activation".into());
            }
            self.quran_study_font = crate::reading_control::PrivateFont::load(
                &self
                    .resources
                    .root
                    .join("fonts/digital-khatt/DigitalKhattV2.otf"),
            )
            .ok();
            SetWindowTheme(
                self.controls[&STUDY],
                if self.prefs.theme == 0 || self.prefs.theme == 3 {
                    w!("DarkMode_Explorer")
                } else {
                    w!("Explorer")
                },
                None,
            )?;
            if !SetWindowSubclass(
                self.controls[&STUDY],
                Some(crate::reading_control::procedure),
                2,
                0,
            )
            .as_bool()
            {
                return Err(windows::core::Error::from_thread().into());
            }
            SetWindowSubclass(
                self.controls[&STUDY],
                Some(crate::smooth_scroll::procedure),
                0x514d,
                0,
            )
            .ok()?;
            SendMessageW(
                self.controls[&STUDY],
                EM_SETTYPOGRAPHYOPTIONS,
                Some(WPARAM(TO_ADVANCEDTYPOGRAPHY as usize)),
                Some(LPARAM(TO_ADVANCEDTYPOGRAPHY as isize)),
            );
            self.child(RECORDING_MODE, "COMBOBOX", "", CBS_DROPDOWNLIST as u32)?;
            self.child(RECITER_LABEL, "STATIC", "", 0)?;
            self.child(
                RECITER,
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0,
            )?;
            self.child(CHAPTER_LABEL, "STATIC", "", 0)?;
            self.child(
                CHAPTER,
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0,
            )?;
            self.child(FIRST_LABEL, "STATIC", "", 0)?;
            self.child(FIRST, "EDIT", "1", ES_AUTOHSCROLL as u32 | ES_CENTER as u32)?;
            self.child(LAST_LABEL, "STATIC", "", 0)?;
            self.child(LAST, "EDIT", "7", ES_AUTOHSCROLL as u32 | ES_CENTER as u32)?;
            for id in [FIRST, LAST] {
                SendMessageW(
                    self.controls[&id],
                    windows::Win32::UI::Controls::EM_SETLIMITTEXT,
                    Some(WPARAM(3)),
                    None,
                );
            }
            self.child(REPEAT_LABEL, "STATIC", "", 0)?;
            self.child(SPEED_LABEL, "STATIC", "", 0)?;
            self.child(SPEED, "COMBOBOX", "", CBS_DROPDOWNLIST as u32)?;
            self.child(VOLUME_LABEL, "STATIC", "", 0)?;
            InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES,
            })
            .ok()?;
            self.child(VOLUME, "msctls_trackbar32", "", TBS_NOTICKS)?;
            SetWindowSubclass(
                self.controls[&VOLUME],
                Some(crate::control_vectors::slider_procedure),
                4,
                0,
            )
            .ok()?;
            SendMessageW(
                self.controls[&VOLUME],
                TBM_SETRANGE,
                Some(WPARAM(1)),
                Some(LPARAM(100 << 16)),
            );
            self.child(AUDIO_TIME, "STATIC", "", 1)?;
            self.child(
                AUDIO_POSITION,
                "msctls_trackbar32",
                "Playback position",
                TBS_NOTICKS,
            )?;
            SetWindowSubclass(
                self.controls[&AUDIO_POSITION],
                Some(crate::control_vectors::slider_procedure),
                4,
                0,
            )
            .ok()?;
            SendMessageW(
                self.controls[&AUDIO_POSITION],
                TBM_SETRANGE,
                Some(WPARAM(1)),
                Some(LPARAM(10000 << 16)),
            );
            self.child(REPEAT, "COMBOBOX", "", CBS_DROPDOWNLIST as u32)?;
            self.child(DOWNLOAD_STATUS, "STATIC", "", 1)?;
            for i in 0..tafsir_library::SOURCES.len() {
                self.child(
                    TAFSIR_ACTION_FIRST + i as u32,
                    "BUTTON",
                    "",
                    BS_OWNERDRAW as u32,
                )?;
            }
            self.child(
                TAFSIR_INFO,
                "RICHEDIT50W",
                "",
                ES_MULTILINE as u32 | ES_READONLY as u32 | ES_AUTOVSCROLL as u32,
            )?;
            SetWindowSubclass(
                self.controls[&TAFSIR_INFO],
                Some(crate::reading_control::procedure),
                2,
                0,
            )
            .ok()?;
            self.child(
                TEXT_VIEW,
                "EDIT",
                "",
                ES_MULTILINE as u32
                    | ES_READONLY as u32
                    | ES_AUTOVSCROLL as u32
                    | WS_VSCROLL.0
                    | ES_RIGHT as u32,
            )?;
            SetWindowLongPtrW(
                self.controls[&TEXT_VIEW],
                GWL_EXSTYLE,
                WS_EX_RTLREADING.0 as isize,
            );
            self.child(STATUS, "STATIC", "", 1)?; // SS_CENTER from the Win32 STATIC contract.
            self.child(NOTICE, "STATIC", "", 1)?;
            self.create_guide_controls()?;
            self.tooltips = Some(crate::tooltips::Tooltips::new(self.hwnd)?);
            self.refresh_labels();
            self.replace_background(self.palette());
            self.resize();
            Ok(())
        }
    }
    unsafe fn refresh_labels(&mut self) {
        unsafe {
            for id in [QUERY, BROWSE_QUERY] {
                let handle = self.controls[&id];
                let style = GetWindowLongPtrW(handle, GWL_STYLE);
                SetWindowLongPtrW(
                    handle,
                    GWL_STYLE,
                    (style & !(ES_RIGHT as isize | ES_CENTER as isize))
                        | if self.prefs.arabic {
                            ES_RIGHT as isize
                        } else {
                            ES_LEFT as isize
                        },
                );
                let extended = GetWindowLongPtrW(handle, GWL_EXSTYLE);
                SetWindowLongPtrW(
                    handle,
                    GWL_EXSTYLE,
                    (extended & !(WS_EX_RTLREADING.0 as isize))
                        | if self.prefs.arabic {
                            WS_EX_RTLREADING.0 as isize
                        } else {
                            0
                        },
                );
                let _ = InvalidateRect(Some(handle), None, false);
            }
            for id in [AUDIO_TIME, STATUS] {
                let h = self.controls[&id];
                let extended = GetWindowLongPtrW(h, GWL_EXSTYLE);
                let desired = (extended & !(WS_EX_RTLREADING.0 as isize))
                    | if self.prefs.arabic {
                        WS_EX_RTLREADING.0 as isize
                    } else {
                        0
                    };
                if extended != desired {
                    SetWindowLongPtrW(h, GWL_EXSTYLE, desired);
                }
            }
            self.set_text(THEME_LABEL, self.tr("Theme", "المظهر"));
            self.fill(
                THEME_SELECTOR,
                &[
                    self.tr("Graphite", "رمادي داكن").into(),
                    self.tr("Light", "فاتح").into(),
                    self.tr("Sepia", "بني فاتح").into(),
                    self.tr("Pure black", "أسود").into(),
                ],
                self.prefs.theme.min(3) as usize,
            );
            for id in [THEME_LABEL, THEME_SELECTOR] {
                if id == THEME_LABEL {
                    let style = GetWindowLongPtrW(self.controls[&id], GWL_STYLE);
                    SetWindowLongPtrW(
                        self.controls[&id],
                        GWL_STYLE,
                        (style & !3) | if self.prefs.arabic { 2 } else { 0 },
                    );
                }
                SetWindowLongPtrW(
                    self.controls[&id],
                    GWL_EXSTYLE,
                    if self.prefs.arabic {
                        WS_EX_RTLREADING.0 as isize
                    } else {
                        0
                    },
                );
            }
            self.fill(
                REPEAT,
                &[
                    self.tr("Play once", "تشغيل مرة واحدة").into(),
                    self.tr("Play 2 times", "تشغيل مرتين").into(),
                    self.tr("Play 3 times", "تشغيل ٣ مرات").into(),
                    self.tr("Play 5 times", "تشغيل ٥ مرات").into(),
                    self.tr("Play 10 times", "تشغيل ١٠ مرات").into(),
                    self.tr("Until stopped", "حتى الإيقاف").into(),
                ],
                [1, 2, 3, 5, 10, 0]
                    .iter()
                    .position(|n| *n == self.prefs.repeat_plays)
                    .unwrap_or(0),
            );
            let kind = self.select(BROWSE_KIND).min(2);
            self.fill(
                BROWSE_KIND,
                &[
                    self.tr("Juz", "الجزء").into(),
                    self.tr("Hizb", "الحزب").into(),
                    self.tr("Rub", "الربع").into(),
                ],
                kind,
            );
            SendMessageW(
                self.controls[&QUERY],
                windows::Win32::UI::Controls::EM_SETCUEBANNER,
                Some(WPARAM(1)),
                Some(LPARAM(
                    HSTRING::from(self.tr(
                        "Search the Quran, a surah, or a page number…",
                        "ابحث في القرآن، أو عن سورة، أو رقم صفحة…",
                    ))
                    .as_ptr() as isize,
                )),
            );
            // Translating captions must not reset a listening form in progress.
            let chapter = SendMessageW(self.controls[&CHAPTER], CB_GETCURSEL, None, None).0;
            let reciter = SendMessageW(self.controls[&RECITER], CB_GETCURSEL, None, None).0;
            let reciter_text = self.text(RECITER);
            let info = self.controls[&TAFSIR_INFO];
            let style = GetWindowLongPtrW(info, GWL_STYLE);
            SetWindowLongPtrW(
                info,
                GWL_STYLE,
                (style & !3)
                    | if self.prefs.arabic {
                        ES_RIGHT as isize
                    } else {
                        ES_LEFT as isize
                    },
            );
            SetWindowLongPtrW(
                info,
                GWL_EXSTYLE,
                if self.prefs.arabic {
                    WS_EX_RTLREADING.0 as isize
                } else {
                    0
                },
            );
            for (id, en, ar) in [
                (MINIMIZE, "Minimize", "تصغير"),
                (MAXIMIZE, "Maximize or restore", "تكبير أو استعادة"),
                (WINDOW_CLOSE, "Close", "إغلاق"),
                (SEARCH, "Search", "بحث"),
                (LISTEN, "Listen", "استماع"),
                (SETTINGS, "Options", "خيارات"),
                (FULLSCREEN, "Full screen", "ملء الشاشة"),
                (DAILY, "Start reading", "بدء القراءة"),
                (GO, "Go", "انتقال"),
                (SEARCH_CLOSE, "Close search", "إغلاق البحث"),
                (CLOSE, "Close", "إغلاق"),
                (
                    EXIT_TEXT_VIEW,
                    "Return to Quran pages",
                    "العودة إلى صفحات المصحف",
                ),
                (AYAH_PREV, "Previous ayah", "الآية السابقة"),
                (AYAH_NEXT, "Next ayah", "الآية التالية"),
                (PLAY, "Play range", "تشغيل النطاق"),
                (PAUSE, "Pause / Resume", "إيقاف مؤقت / متابعة"),
                (
                    QUICK_PAUSE,
                    "Pause / Resume · Space",
                    "إيقاف مؤقت / متابعة · مسافة",
                ),
                (QUICK_STOP, "End recitation", "إنهاء التلاوة"),
                (FOLLOW_AUDIO, "Go to current ayah", "انتقل إلى الآية الحالية"),
                (
                    COLLAPSE_PLAYER,
                    "Collapse or expand player",
                    "طي مشغل التلاوة أو توسيعه",
                ),
                (SEEK_BACK, "Back 10 seconds", "رجوع 10 ثوانٍ"),
                (SEEK_FORWARD, "Forward 10 seconds", "تقديم 10 ثوانٍ"),
                (QUICK_SETTINGS, "Listening settings", "إعدادات الاستماع"),
                (STOP, "Stop", "إيقاف"),
                (DOWNLOAD, "Download whole surah", "تنزيل السورة كاملة"),
                (STORAGE, "Details", "تفاصيل"),
                (MANAGE_TAFSIR, "Manage tafsirs", "إدارة التفاسير"),
            ] {
                self.set_text(id, self.tr(en, ar));
            }
            for (id, en, ar) in [
                (SOURCE_LABEL, "Tafsir source", "مصدر التفسير"),
                (RECITER_LABEL, "Reciter", "القارئ"),
                (CHAPTER_LABEL, "Surah", "السورة"),
                (FIRST_LABEL, "From ayah", "من الآية"),
                (LAST_LABEL, "To ayah", "إلى الآية"),
                (SPEED_LABEL, "Playback speed", "سرعة التشغيل"),
                (REPEAT_LABEL, "Repeat", "التكرار"),
                (VOLUME_LABEL, "Volume", "مستوى الصوت"),
            ] {
                self.set_text(id, self.tr(en, ar));
                let handle = self.controls[&id];
                let style = GetWindowLongPtrW(handle, GWL_STYLE);
                // SS_LEFT=0 and SS_RIGHT=2 are native STATIC alignment flags.
                SetWindowLongPtrW(
                    handle,
                    GWL_STYLE,
                    (style & !3) | if self.prefs.arabic { 2 } else { 0 },
                );
                SetWindowLongPtrW(
                    handle,
                    GWL_EXSTYLE,
                    if self.prefs.arabic {
                        WS_EX_RTLREADING.0 as isize
                    } else {
                        0
                    },
                );
            }
            if self.download.is_some() {
                self.set_text(DOWNLOAD, self.tr("Cancel download", "إلغاء التنزيل"));
            }
            if self.fullscreen {
                self.set_text(
                    FULLSCREEN,
                    self.tr("Exit full screen", "الخروج من ملء الشاشة"),
                );
            }
            if self.daily.is_some() {
                self.set_text(DAILY, self.tr("Finish reading", "إنهاء القراءة"));
            }
            self.set_text(
                TAFSIR_ENGLISH,
                self.tr("English tafsirs", "تفاسير إنجليزية"),
            );
            self.set_text(TAFSIR_ARABIC, self.tr("Arabic tafsirs", "تفاسير عربية"));
            self.refresh_study_follow();
            self.refresh_sources();
            let single = self.controls[&SOURCE_SINGLE];
            let style = GetWindowLongPtrW(single, GWL_STYLE);
            SetWindowLongPtrW(
                single,
                GWL_STYLE,
                (style & !3) | if self.prefs.arabic { 2 } else { 0 },
            );
            let direction = if self.prefs.arabic {
                WS_EX_RTLREADING.0 as isize
            } else {
                0
            };
            SetWindowLongPtrW(single, GWL_EXSTYLE, direction);
            self.refresh_tafsir_actions();
            self.set_text(RECORDING_MODE, self.tr("Recording source", "مصدر التسجيل"));
            self.fill(
                RECORDING_MODE,
                &[
                    self.tr("Ayah recordings", "تسجيلات الآيات").to_owned(),
                    self.tr("Surah recording", "تسجيل السورة").to_owned(),
                ],
                usize::from(self.prefs.continuous_audio),
            );
            self.fill(
                RECITER,
                &RECITERS
                    .iter()
                    .map(|r| {
                        if self.prefs.arabic {
                            r.arabic
                        } else {
                            r.english
                        }
                        .to_owned()
                    })
                    .collect::<Vec<_>>(),
                if reciter >= 0 {
                    reciter as usize
                } else {
                    self.reciter
                },
            );
            if reciter < 0 && !reciter_text.is_empty() {
                self.set_text(RECITER, &reciter_text);
            }
            self.fill(
                CHAPTER,
                &self
                    .resources
                    .surahs
                    .iter()
                    .map(|s| {
                        if self.prefs.arabic {
                            format!("{} ({})", s.name_arabic, s.id)
                        } else {
                            format!("{}  {}", s.id, s.name_simple)
                        }
                    })
                    .collect::<Vec<_>>(),
                if chapter >= 0 {
                    chapter as usize
                } else {
                    self.selected.map(|s| s.0 as usize - 1).unwrap_or(0)
                },
            );
            self.fill(
                SPEED,
                &[
                    "0.75×".into(),
                    "1×".into(),
                    "1.25×".into(),
                    "1.5×".into(),
                    "2×".into(),
                ],
                match self.prefs.speed {
                    s if s < 0.9 => 0,
                    s if s < 1.1 => 1,
                    s if s < 1.4 => 2,
                    s if s < 1.8 => 3,
                    _ => 4,
                },
            );
            self.localize_listening_fields();
            SendMessageW(
                self.controls[&VOLUME],
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM((self.prefs.volume * 100.).round() as isize)),
            );
            self.set_text(
                VOLUME_LABEL,
                &format!(
                    "{} · {}%",
                    self.tr("Volume", "مستوى الصوت"),
                    (self.prefs.volume * 100.).round() as u32
                ),
            );
            let handle = self.controls[&STUDY];
            SetWindowLongPtrW(
                handle,
                GWL_EXSTYLE,
                if self.source == 2 {
                    0
                } else {
                    (WS_EX_RTLREADING | WS_EX_RIGHT).0 as isize
                },
            );
            self.format_storage_details();
            self.refresh_transport();
        }
    }
    unsafe fn fill(&self, id: u32, items: &[String], selected: usize) {
        unsafe {
            let handle = self.controls[&id];
            SendMessageW(handle, CB_RESETCONTENT, None, None);
            for item in items {
                let text = HSTRING::from(item);
                SendMessageW(
                    handle,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(text.as_ptr() as isize)),
                );
            }
            SendMessageW(handle, CB_SETCURSEL, Some(WPARAM(selected)), None);
            crate::field_style::apply_combo_metrics(handle, self.dpi);
        }
    }
    unsafe fn set_text(&self, id: u32, text: &str) {
        unsafe {
            // Status polling must not repaint unchanged child controls every tick.
            if self.text(id) != text {
                let _ = SetWindowTextW(self.controls[&id], &HSTRING::from(text));
                if id == STUDY {
                    self.format_study_color(self.palette());
                } else if id == TAFSIR_INFO {
                    let handle = self.controls[&id];
                    let mut selected = CHARRANGE::default();
                    SendMessageW(
                        handle,
                        EM_EXGETSEL,
                        None,
                        Some(LPARAM((&mut selected as *mut CHARRANGE) as isize)),
                    );
                    SendMessageW(handle, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                    let paragraph = PARAFORMAT {
                        cbSize: std::mem::size_of::<PARAFORMAT>() as u32,
                        dwMask: PFM_ALIGNMENT | PFM_RTLPARA,
                        Anonymous: PARAFORMAT_0 {
                            wEffects: u16::from(self.prefs.arabic),
                        },
                        wAlignment: if self.prefs.arabic {
                            PFA_RIGHT
                        } else {
                            PFA_LEFT
                        },
                        ..Default::default()
                    };
                    SendMessageW(
                        handle,
                        EM_SETPARAFORMAT,
                        None,
                        Some(LPARAM((&paragraph as *const PARAFORMAT) as isize)),
                    );
                    SendMessageW(
                        handle,
                        EM_EXSETSEL,
                        None,
                        Some(LPARAM((&selected as *const CHARRANGE) as isize)),
                    );
                    crate::reading_control::hide_caret(handle);
                }
            }
        }
    }
    unsafe fn text(&self, id: u32) -> String {
        unsafe {
            let capacity = if matches!(id, TEXT_VIEW | STUDY) {
                (GetWindowTextLengthW(self.controls[&id]).max(0) as usize + 1).min(64 * 1024)
            } else {
                4096
            };
            let mut text = vec![0u16; capacity];
            let len = GetWindowTextW(self.controls[&id], &mut text);
            String::from_utf16_lossy(&text[..len.max(0) as usize])
        }
    }
    unsafe fn select(&self, id: u32) -> usize {
        unsafe {
            SendMessageW(self.controls[&id], CB_GETCURSEL, None, None)
                .0
                .max(0) as usize
        }
    }
    unsafe fn place(&self, id: u32, mut rect: Rect, mut visible: bool) {
        unsafe {
            if self.displayed_panel() == Panel::Listen
                && (id != STATUS
                    || self.side_playback()
                    || self.audio.as_ref().is_some_and(Audio::has_session))
                && matches!(
                    id,
                    RECITER
                        | RECORDING_MODE
                        | CHAPTER
                        | FIRST
                        | LAST
                        | SPEED
                        | REPEAT
                        | PLAY
                        | PAUSE
                        | STOP
                        | VOLUME
                        | RECITER_LABEL
                        | CHAPTER_LABEL
                        | FIRST_LABEL
                        | LAST_LABEL
                        | SPEED_LABEL
                        | REPEAT_LABEL
                        | VOLUME_LABEL
                        | DOWNLOAD
                        | DOWNLOAD_STATUS
                        | STORAGE
                        | AUDIO_POSITION
                        | AUDIO_TIME
                        | SEEK_BACK
                        | SEEK_FORWARD
                        | QUICK_PAUSE
                        | QUICK_STOP
                        | FOLLOW_AUDIO
                        | STATUS
                )
            {
                rect.y -= self.panel_scroll;
                let collapsed = if matches!(id, RECITER | RECORDING_MODE | CHAPTER | SPEED | REPEAT)
                {
                    34.
                } else {
                    rect.h
                };
                visible &= rect.y >= 100. && rect.y + collapsed <= self.height - 35.;
            }
            // Decide occlusion before any HWND show/move. Repeated animation
            // layouts must not briefly expose controls behind a menu.
            if visible
                && id < MENU_ROW
                && !matches!(
                    id,
                    THEME_SELECTOR
                        | THEME_LABEL
                        | GUIDE_BACK
                        | GUIDE_NEXT
                        | GUIDE_CLOSE
                        | GUIDE_SHOW
                        | GUIDE_BODY
                )
            {
                if let Some(popup) = &self.popup {
                    let bounds = popup.display_rect();
                    let height = if matches!(
                        id,
                        RECITER | RECORDING_MODE | CHAPTER | SPEED | REPEAT | SOURCE | BROWSE_KIND
                    ) {
                        34.
                    } else {
                        rect.h
                    };
                    if rect.x < bounds.x + bounds.w
                        && rect.x + rect.w > bounds.x
                        && rect.y < bounds.y + bounds.h
                        && rect.y + height > bounds.y
                    {
                        visible = false;
                    }
                }
            }
            self.placed.borrow_mut().insert(id);
            let mut positions = self.placements.borrow_mut();
            let previous = positions.get(&id).copied();
            let scale = self.dpi / 96.;
            let pixels = |r: Rect| {
                (
                    (r.x * scale) as i32,
                    (r.y * scale) as i32,
                    (r.w * scale).max(1.) as i32,
                    (r.h * scale).max(1.) as i32,
                )
            };
            if id == RESULTS {
                let height = (self.search_row_height() * self.dpi / 96.).round() as isize;
                if SendMessageW(self.controls[&RESULTS], LB_GETITEMHEIGHT, None, None).0 != height {
                    SendMessageW(
                        self.controls[&RESULTS],
                        LB_SETITEMHEIGHT,
                        Some(WPARAM(0)),
                        Some(LPARAM(height)),
                    );
                }
            }
            if previous.is_some_and(|(previous, shown)| {
                pixels(previous) == pixels(rect) && shown == visible
            }) {
                // Keep logical geometry current without asking USER32 to move
                // a control again when its physical pixels have not changed.
                positions.insert(id, (rect, visible));
                return;
            }
            positions.insert(id, (rect, visible));
            drop(positions);
            let handle = self.controls[&id];
            let was_visible = previous.map(|(_, shown)| shown).unwrap_or_else(|| {
                GetWindowLongPtrW(handle, GWL_STYLE) & WS_VISIBLE.0 as isize != 0
            });
            let (x, y, width, height) = pixels(rect);
            let unchanged = previous.map(|(old, _)| pixels(old));
            let placement_flags =
                if unchanged.is_some_and(|(_, _, w, h)| w == width && h == height) {
                    SWP_NOSIZE
                } else {
                    SET_WINDOW_POS_FLAGS(0)
                } | if unchanged.is_some_and(|(old_x, old_y, _, _)| old_x == x && old_y == y) {
                    SWP_NOMOVE
                } else {
                    SET_WINDOW_POS_FLAGS(0)
                };
            if let Some(batch) = self.position_batch.get() {
                let flags = SWP_NOZORDER
                    | SWP_NOACTIVATE
                    | SWP_NOREDRAW
                    | placement_flags
                    | if was_visible == visible {
                        SET_WINDOW_POS_FLAGS(0)
                    } else if visible {
                        SWP_SHOWWINDOW
                    } else {
                        SWP_HIDEWINDOW
                    };
                // Motion candidate retains existing client pixels. The parent
                // and affected controls are explicitly repainted after commit.
                match DeferWindowPos(batch, handle, None, x, y, width, height, flags) {
                    Ok(next) => {
                        self.position_batch.set(Some(next));
                        if visible {
                            let _ = InvalidateRect(Some(handle), None, false);
                        }
                        return;
                    }
                    Err(_) => {
                        // A failed DeferWindowPos invalidates the entire batch;
                        // EndDeferWindowPos must not receive the old handle.
                        self.position_batch.set(None);
                        self.position_batch_failed.set(true);
                    }
                }
            }
            let positioned = SetWindowPos(
                handle,
                None,
                x,
                y,
                width,
                height,
                SWP_NOZORDER
                    | SWP_NOACTIVATE
                    | SWP_NOCOPYBITS
                    | placement_flags
                    | if self.title_animation.is_some() && matches!(id, SEARCH | LISTEN | SETTINGS)
                    {
                        SWP_NOREDRAW
                    } else {
                        SET_WINDOW_POS_FLAGS(0)
                    },
            )
            .is_ok();
            if !positioned {
                self.placements.borrow_mut().remove(&id);
            }
            if positioned && was_visible != visible {
                let _ = ShowWindow(handle, if visible { SW_SHOW } else { SW_HIDE });
            }
            if visible {
                let _ = InvalidateRect(Some(handle), None, false);
            }
        }
    }
    fn search_offset(&self) -> f32 {
        self.search_animation
            .map(|start| {
                let t = (start.elapsed().as_secs_f32() / 0.18).min(1.);
                -16. * (1. - t * t * (3. - 2. * t))
            })
            .unwrap_or(0.)
    }
    fn panel_width(&self) -> f32 {
        let target = if self.panel != Panel::None { 382. } else { 0. };
        if let Some((start, _, from)) = self.panel_animation {
            #[cfg(feature = "qa")]
            let t = self
                .panel_pose
                .unwrap_or_else(|| (start.elapsed().as_secs_f32() / 0.22).min(1.));
            #[cfg(not(feature = "qa"))]
            let t = (start.elapsed().as_secs_f32() / 0.22).min(1.);
            let t = if self.study_reveal_pending {
                #[cfg(feature = "qa")]
                {
                    self.panel_pose.unwrap_or(0.)
                }
                #[cfg(not(feature = "qa"))]
                {
                    0.
                }
            } else {
                t
            };
            let ease = t * t * (3. - 2. * t);
            from + (target - from) * ease
        } else {
            target
        }
    }
    fn displayed_panel(&self) -> Panel {
        if self.panel == Panel::None {
            self.panel_animation
                .map(|(_, old, _)| old)
                .unwrap_or(Panel::None)
        } else {
            self.panel
        }
    }
    fn panel_on_left(&self) -> bool {
        !self.prefs.arabic && matches!(self.displayed_panel(), Panel::Study | Panel::Tafsirs)
    }
    fn panel_origin(&self) -> f32 {
        let width = 360f32.min((self.width - 180.).max(250.));
        let offset = 382. - self.panel_width()
            + if self.panel == Panel::Study {
                self.panel_animation
                    .filter(|(_, _, from)| (*from - 382.).abs() < 0.01)
                    .map_or(0., |(start, _, _)| {
                        let t = if self.study_reveal_pending {
                            0.
                        } else {
                            (start.elapsed().as_secs_f32() / 0.22).min(1.)
                        };
                        16. * (1. - t * t * (3. - 2. * t))
                    })
            } else {
                0.
            };
        if self.panel_on_left() {
            14. + if self.fullscreen { 56. } else { 0. } - offset
        } else {
            self.width - width - 14. + offset
        }
    }
    fn set_panel(&mut self, panel: Panel) {
        if panel != Panel::None {
            self.search = false;
            self.search_animation = None;
        }
        if self.panel == panel {
            return;
        }
        let old = self.displayed_panel();
        let from = self.panel_width();
        self.panel = panel;
        let mut enabled = BOOL::from(true);
        unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some((&mut enabled as *mut BOOL).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .ok();
        }
        self.panel_animation = if enabled.as_bool()
            && (old == Panel::None
                || panel == Panel::None
                || panel == Panel::Study
                || (old == panel && self.panel_animation.is_some()))
        {
            Some((Instant::now(), old, from))
        } else {
            None
        };
        self.study_reveal_pending = panel == Panel::Study && self.panel_animation.is_some();
        if self.panel_animation.is_some() {
            unsafe {
                SetTimer(Some(self.hwnd), 6, 10, None);
            }
        }
    }
    unsafe fn refresh_icon(&mut self) {
        unsafe {
            let size = (18. * self.dpi / 96.).round() as i32;
            if let Ok(instance) = GetModuleHandleW(None) {
                if let Ok(icon) = LoadImageW(
                    Some(instance.into()),
                    windows::core::PCWSTR(std::ptr::without_provenance(1)),
                    IMAGE_ICON,
                    size,
                    size,
                    IMAGE_FLAGS(0),
                ) {
                    let old = self.identity_icon;
                    self.identity_icon = HICON(icon.0);
                    SendMessageW(
                        self.controls[&IDENTITY_ICON],
                        STM_SETICON,
                        Some(WPARAM(icon.0 as usize)),
                        None,
                    );
                    if !old.is_invalid() {
                        DestroyIcon(old).ok();
                    }
                }
            }
        }
    }
    unsafe fn advance_control_motion(&mut self) {
        unsafe {
            if self.startup_reveal.is_some() {
                if self
                    .startup_reveal
                    .is_some_and(|s| s.elapsed().as_millis() >= 160)
                {
                    self.startup_reveal = None;
                }
                if self.panel_animation.is_none()
                    && self.search_animation.is_none()
                    && self.title_animation.is_none()
                    && self.popup.as_ref().is_none_or(|p| p.motion.is_none())
                {
                    if self.startup_reveal.is_none() {
                        KillTimer(Some(self.hwnd), 6).ok();
                    }
                    self.invalidate_reader();
                    return;
                }
            }
            let caption_only = self.panel_animation.is_none()
                && self.search_animation.is_none()
                && self.popup.as_ref().is_none_or(|p| p.motion.is_none());
            if !self.study_reveal_pending
                && self
                    .panel_animation
                    .is_some_and(|(start, _, _)| start.elapsed().as_secs_f32() >= 0.22)
            {
                self.panel_animation = None;
            }
            if self
                .search_animation
                .is_some_and(|start| start.elapsed().as_secs_f32() >= 0.18)
            {
                self.search_animation = None;
            }
            if self
                .title_animation
                .is_some_and(|(start, _)| start.elapsed().as_secs_f32() >= 0.22)
            {
                self.title_animation = None;
            }
            if let Some(p) = &mut self.popup {
                if p.motion
                    .is_some_and(|start| start.elapsed().as_secs_f32() >= 0.18)
                {
                    p.motion = None;
                }
            }
            if self.panel_animation.is_none()
                && self.search_animation.is_none()
                && self.title_animation.is_none()
                && self.popup.as_ref().is_none_or(|p| p.motion.is_none())
                && self.startup_reveal.is_none()
            {
                KillTimer(Some(self.hwnd), 6).ok();
            }
            if caption_only && self.title_animation.is_some() && !self.fullscreen {
                self.resize_caption_motion();
            } else {
                self.resize_layout(caption_only);
            }
        }
    }
    // Language motion only changes three caption positions. Reformatting every
    // RichEdit and laying out the listening form on each frame is unnecessary.
    unsafe fn resize_caption_motion(&mut self) {
        unsafe {
            #[cfg(feature = "qa")]
            let began = Instant::now();
            let widths = [96., 96., 110.];
            let ids = [SEARCH, LISTEN, SETTINGS];
            let total = widths.iter().sum::<f32>() + 6.;
            let mut x = if self.prefs.arabic {
                (self.width - total - 144.).max(225.)
            } else {
                225.
            };
            if let Some((start, from)) = self.title_animation {
                let t = (start.elapsed().as_secs_f32() / 0.22).min(1.);
                x += (from - x) * (1. - t * t * (3. - 2. * t));
            }
            self.position_batch_failed.set(false);
            match BeginDeferWindowPos(3) {
                Ok(batch) => self.position_batch.set(Some(batch)),
                Err(_) => {
                    self.resize_layout(true);
                    return;
                }
            }
            for i in 0..3 {
                let index = if self.prefs.arabic { 2 - i } else { i };
                self.place(
                    ids[index],
                    Rect {
                        x,
                        y: 2.,
                        w: widths[index],
                        h: 32.,
                    },
                    true,
                );
                x += widths[index] + 3.;
            }
            if let Some(batch) = self.position_batch.take() {
                if EndDeferWindowPos(batch).is_err() {
                    self.position_batch_failed.set(true);
                }
            }
            if self.position_batch_failed.get() {
                self.resize_layout(true);
                return;
            }
            let rect = RECT {
                left: 0,
                top: 0,
                right: (self.width * self.dpi / 96.).ceil() as i32,
                bottom: (36. * self.dpi / 96.).ceil() as i32,
            };
            let _ = InvalidateRect(Some(self.hwnd), Some(&rect), false);
            #[cfg(feature = "qa")]
            {
                self.last_layout_ms = began.elapsed().as_secs_f64() * 1000.;
                self.layout_generation = self.layout_generation.wrapping_add(1);
            }
        }
    }
    unsafe fn resize(&mut self) {
        unsafe {
            self.resize_layout(false);
        }
    }
    unsafe fn resize_layout(&mut self, caption_only: bool) {
        #[cfg(feature = "qa")]
        let layout_began = Instant::now();
        if let Some(tips) = &self.tooltips {
            tips.pop();
        }
        if !caption_only {
            self.hovered = None;
        }
        unsafe {
            // ShowScrollBar can synchronously send WM_SIZE even when the
            // requested visibility is unchanged. Avoid a deferred-resize loop.
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let current_dpi = GetDpiForWindow(self.hwnd).max(96) as f32;
            let current_height = (rc.bottom - rc.top) as f32 * 96. / current_dpi;
            let active = self.audio.as_ref().is_some_and(Audio::has_session);
            let has_scrollbar = GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32 & WS_VSCROLL.0 != 0;
            let needs_scrollbar = self.displayed_panel() == Panel::Listen
                && ListenLayout::new(current_height).scroll_max(current_height, active) > 0.5;
            if has_scrollbar != needs_scrollbar {
                let _ = ShowScrollBar(self.hwnd, SB_VERT, needs_scrollbar);
                self.listen_scroll_geometry = None;
                let _ = GetClientRect(self.hwnd, &mut rc);
            }
            let dpi = GetDpiForWindow(self.hwnd).max(96) as f32;
            if dpi != self.dpi {
                self.dpi = dpi;
                self.placements.borrow_mut().clear();
                self.refresh_icon();
                let font = CreateFontW(
                    -(15. * dpi / 96.).round() as i32,
                    0,
                    0,
                    0,
                    400,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    0,
                    w!("Segoe UI"),
                );
                for handle in self.controls.values() {
                    SendMessageW(
                        *handle,
                        WM_SETFONT,
                        Some(WPARAM(font.0 as usize)),
                        Some(LPARAM(1)),
                    );
                    crate::field_style::apply_field_metrics(*handle, dpi);
                }
                let _ = DeleteObject(self.font.into());
                self.font = font;
                self.refresh_study_font();
            }
            let scale = self.dpi / 96.;
            self.width = (rc.right - rc.left) as f32 / scale;
            self.height = (rc.bottom - rc.top) as f32 / scale;
            // Resolve the complete popup bounds before placing any underlying
            // controls. A zero-height first layout briefly exposed Search and
            // Listen fields through the opening drawer.
            if let Some(p) = &mut self.popup {
                if p.docked {
                    let edge = if self.fullscreen && !p.rtl { 64. } else { 12. };
                    p.rect.w = 360f32.min((self.width - 24.).max(250.));
                    p.rect.x = if p.rtl {
                        (self.width - p.rect.w - edge).max(12.)
                    } else {
                        edge
                    };
                }
                p.rect.h = if p.docked {
                    (self.height - 64.).max(360.)
                } else {
                    50. + p
                        .rows
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            if p.context
                                && p.stack.is_empty()
                                && (i == 2 || p.listening_expanded && r.id == BOOKMARK)
                            {
                                0.
                            } else if r.detail.is_empty() {
                                38.
                            } else {
                                60.
                            }
                        })
                        .sum::<f32>()
                        + 10.
                };
                p.rect.y = p.rect.y.min((self.height - p.rect.h - 12.).max(42.));
                p.frame_pose = None;
                if let Some(start) = p.motion {
                    p.frame_pose = Some((start, p.display_rect()));
                }
            }
            // Popup vectors/text share the parent's single presentation. Child
            // HWNDs remain visible for native input and accessibility, but must
            // not punch holes in the composed surface. Restore ordinary child
            // clipping as soon as the popup closes.
            let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32;
            let wanted = if self.popup.is_some() {
                style & !WS_CLIPCHILDREN.0
            } else {
                style | WS_CLIPCHILDREN.0
            };
            if wanted != style {
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, wanted as isize);
            }
            if self.displayed_panel() == Panel::Listen {
                let extent = ListenLayout::new(self.height).bottom(active) - 100.;
                let viewport = (self.height - 135.).max(1.);
                self.panel_scroll = self.panel_scroll.clamp(0., (extent - viewport).max(0.));
                let scroll = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                    nMin: 0,
                    nMax: if needs_scrollbar {
                        (extent.ceil() as i32 - 1).max(0)
                    } else {
                        0
                    },
                    nPage: if needs_scrollbar {
                        viewport.floor() as u32
                    } else {
                        1
                    },
                    nPos: self.panel_scroll as i32,
                    ..Default::default()
                };
                let geometry = (scroll.nMax, scroll.nPage, scroll.nPos);
                if self.listen_scroll_geometry != Some(geometry) {
                    SetScrollInfo(self.hwnd, SB_VERT, &scroll, true);
                    self.listen_scroll_geometry = Some(geometry);
                }
            } else {
                self.panel_scroll = 0.;
            }
            let batched_controls = !qa_disabled("--qa-ordinary-controls")
                && (self.displayed_panel() == Panel::Listen
                    || self.panel_animation.is_some()
                    || self.title_animation.is_some()
                    || self.search_animation.is_some()
                    || self.popup.as_ref().is_some_and(|p| p.motion.is_some()));
            self.position_batch_failed.set(false);
            if batched_controls {
                match BeginDeferWindowPos(self.controls.len() as i32) {
                    Ok(batch) => self.position_batch.set(Some(batch)),
                    Err(_) => self.position_batch_failed.set(true),
                }
            }
            self.placed.borrow_mut().clear();
            self.place(
                IDENTITY_ICON,
                Rect {
                    x: if self.fullscreen { 21. } else { 9. },
                    y: if self.fullscreen { 16. } else { 9. },
                    w: 18.,
                    h: 18.,
                },
                true,
            );
            let ids = [SEARCH, LISTEN, SETTINGS];
            let widths = [96., 96., 110.];
            let total: f32 = widths.iter().sum::<f32>() + 6.;
            let mut x = if self.prefs.arabic {
                (self.width - total - if self.fullscreen { 156. } else { 144. }).max(225.)
            } else {
                225.
            };
            if let Some((start, from)) = self.title_animation {
                let t = (start.elapsed().as_secs_f32() / 0.22).min(1.);
                x += (from - x) * (1. - t * t * (3. - 2. * t));
            }
            for i in 0..if self.fullscreen { 0 } else { ids.len() } {
                let index = if self.prefs.arabic {
                    ids.len() - 1 - i
                } else {
                    i
                };
                self.place(
                    ids[index],
                    Rect {
                        x,
                        y: 2.,
                        w: widths[index],
                        h: 32.,
                    },
                    true,
                );
                x += widths[index] + 3.;
            }
            for (id, offset) in [(MINIMIZE, 138.), (MAXIMIZE, 92.), (WINDOW_CLOSE, 46.)] {
                self.place(
                    id,
                    Rect {
                        x: self.width - offset,
                        y: 0.,
                        w: 46.,
                        h: 36.,
                    },
                    !self.fullscreen,
                );
            }
            self.place(
                FULLSCREEN,
                Rect {
                    x: if self.fullscreen {
                        8.
                    } else {
                        self.width - 150.
                    },
                    y: if self.fullscreen {
                        self.height - 52.
                    } else {
                        2.
                    },
                    w: if self.fullscreen { 60. } else { 144. },
                    h: if self.fullscreen { 36. } else { 32. },
                },
                self.fullscreen,
            );
            if self.fullscreen {
                self.place(
                    IDENTITY_ICON,
                    Rect {
                        x: 21.,
                        y: 16.,
                        w: 18.,
                        h: 18.,
                    },
                    true,
                );
                for (i, id) in [SEARCH, LISTEN, SETTINGS].into_iter().enumerate() {
                    self.place(
                        id,
                        Rect {
                            x: 8.,
                            y: 56. + i as f32 * 44.,
                            w: 44.,
                            h: 36.,
                        },
                        true,
                    );
                }
                self.set_text(FULLSCREEN, self.tr("Exit full screen", "إنهاء ملء الشاشة"));
                self.place(
                    FULLSCREEN,
                    Rect {
                        x: 8.,
                        y: self.height - 52.,
                        w: 60.,
                        h: 36.,
                    },
                    true,
                );
            }
            let crowded_rail = self.fullscreen
                && self.side_playback()
                && (self.player_collapsed || self.compact_player_frame().is_none())
                && self.height / 2. - 25. < 326.
                && self.height / 2. + 25. > 188.;
            self.place(
                PREV,
                Rect {
                    x: if crowded_rail && self.height < 446. {
                        72.
                    } else {
                        8.
                    },
                    y: if crowded_rail && self.height >= 446. {
                        340.
                    } else {
                        self.height / 2. - 25.
                    },
                    w: 38.,
                    h: 50.,
                },
                self.displayed_panel() == Panel::None,
            );
            self.place(
                NEXT,
                Rect {
                    x: self.width - 46.,
                    y: self.height / 2. - 25.,
                    w: 38.,
                    h: 50.,
                },
                self.displayed_panel() == Panel::None,
            );
            if self.search {
                let w = (self.width - 40.).min(600.);
                let x = (self.width - w) / 2.;
                let offset = self.search_offset();
                let search_layout = crate::search_layout::SearchLayout::new(self.height);
                self.place(
                    SEARCH_CLOSE,
                    Rect {
                        x: x + w - 44.,
                        y: 58. + offset,
                        w: 32.,
                        h: 32.,
                    },
                    true,
                );
                self.place(
                    QUERY,
                    Rect {
                        x: x + 16.,
                        y: search_layout.query_frame + 12. + offset,
                        w: w - 32.,
                        h: 24.,
                    },
                    true,
                );
                let browse = self.text(QUERY).trim().is_empty();
                for (i, id) in [BROWSE_SURAH, BROWSE_PAGE, BROWSE_SECTIONS]
                    .into_iter()
                    .enumerate()
                {
                    self.set_text(
                        id,
                        match id {
                            BROWSE_SURAH => self.tr("Surah", "السورة"),
                            BROWSE_PAGE => self.tr("Page", "الصفحة"),
                            _ => self.tr("Juz, Hizb & Rub", "الجزء والحزب والربع"),
                        },
                    );
                    self.place(
                        id,
                        Rect {
                            x: x + 16.
                                + (if self.prefs.arabic { 2 - i } else { i }) as f32 * (w - 32.)
                                    / 3.,
                            y: search_layout.tabs + offset,
                            w: (w - 32.) / 3.,
                            h: search_layout.tab_height,
                        },
                        browse,
                    );
                }
                self.place(
                    BROWSE_QUERY,
                    Rect {
                        x: x + 16.
                            + if self.browse_tab == BROWSE_SECTIONS && !self.prefs.arabic {
                                120.
                            } else {
                                0.
                            },
                        y: search_layout.filter_frame + 12. + offset,
                        w: w - 32.
                            - if self.browse_tab == BROWSE_SECTIONS {
                                120.
                            } else {
                                0.
                            },
                        h: 24.,
                    },
                    browse,
                );
                self.place(
                    BROWSE_KIND,
                    Rect {
                        x: x + if self.prefs.arabic { w - 128. } else { 16. },
                        y: search_layout.filter_frame + 8. + offset,
                        w: 112.,
                        h: 240.,
                    },
                    browse && self.browse_tab == BROWSE_SECTIONS,
                );
                let cue = HSTRING::from(match self.browse_tab {
                    BROWSE_SURAH => self.tr(
                        "Find by name, Arabic, or number",
                        "ابحث بالاسم أو بالعربية أو بالرقم",
                    ),
                    BROWSE_PAGE => self.tr("Enter a page from 1 to 604", "أدخل صفحة من ١ إلى ٦٠٤"),
                    _ => self.tr("Enter a section number", "أدخل رقم الجزء أو الحزب أو الربع"),
                });
                // Cue changes cause the edit to repaint synchronously. Its
                // text is stable while the search surface moves.
                if self.search_animation.is_none() {
                    SendMessageW(
                        self.controls[&BROWSE_QUERY],
                        windows::Win32::UI::Controls::EM_SETCUEBANNER,
                        Some(WPARAM(1)),
                        Some(LPARAM(cue.as_ptr() as isize)),
                    );
                }
                self.place(
                    RESULTS,
                    Rect {
                        x: x + 16.,
                        y: self.search_list_y() + offset,
                        w: w - 32.,
                        h: self.search_result_height(),
                    },
                    !self.hits.is_empty(),
                );
            }
            if self.displayed_panel() != Panel::None {
                let w = 360f32.min((self.width - 180.).max(250.));
                let x = self.panel_origin();
                self.place(
                    CLOSE,
                    Rect {
                        x: x + if self.prefs.arabic { 12. } else { w - 44. },
                        y: 65.,
                        w: 32.,
                        h: 32.,
                    },
                    true,
                );
                let x = x + 16.;
                let w = w - 32.;
                if self.displayed_panel() == Panel::Study {
                    self.layout_study_controls(x, w);
                } else if self.displayed_panel() == Panel::Listen {
                    self.layout_listening_controls(x, w);
                } else if self.displayed_panel() == Panel::Tafsirs {
                    for (i, id) in [TAFSIR_ENGLISH, TAFSIR_ARABIC].into_iter().enumerate() {
                        self.place(
                            id,
                            Rect {
                                x: x + i as f32 * (w + 6.) / 2.,
                                y: 108.,
                                w: (w - 6.) / 2.,
                                h: 30.,
                            },
                            true,
                        );
                    }
                    for (row, id) in self.tafsir_rows().into_iter().enumerate() {
                        self.place(
                            TAFSIR_ACTION_FIRST + id as u32,
                            Rect {
                                x: x + if self.prefs.arabic { 8. } else { w - 104. },
                                y: 159. + row as f32 * 66.,
                                w: 96.,
                                h: 32.,
                            },
                            true,
                        );
                    }
                    let y = 154. + self.tafsir_rows().len() as f32 * 66.;
                    let bottom = if self.embedded_transport() {
                        self.transport_top() - 12.
                    } else {
                        self.height - 25.
                    };
                    self.place(
                        TAFSIR_INFO,
                        Rect {
                            x,
                            y,
                            w,
                            h: (bottom - y).max(1.),
                        },
                        bottom - y >= 48.,
                    );
                } else if self.displayed_panel() == Panel::Storage {
                    self.place(
                        STUDY,
                        Rect {
                            x,
                            y: 104.,
                            w,
                            h: (if self.embedded_transport() {
                                self.transport_top() - 48.
                            } else {
                                self.height - 36.
                            } - 104.)
                                .max(1.),
                        },
                        true,
                    );
                }
            }
            if self.text_view {
                self.place(
                    EXIT_TEXT_VIEW,
                    Rect {
                        x: if self.prefs.arabic {
                            self.reader_area().x + self.reader_area().w - 248.
                        } else {
                            self.reader_area().x + 8.
                        },
                        y: 44.,
                        w: 240.,
                        h: 32.,
                    },
                    true,
                );
                let top = 84.;
                self.place(
                    TEXT_VIEW,
                    Rect {
                        x: self.reader_area().x,
                        y: top,
                        w: (self.width - 96. - self.panel_width()).max(130.),
                        h: (self.height - top - self.status_height()).max(140.),
                    },
                    true,
                );
                if let Err(error) = self.refresh_text_view() {
                    self.report_error(&error);
                }
            }
            if self.side_playback()
                || (self.displayed_panel() == Panel::Listen
                    && self.audio.as_ref().is_some_and(Audio::has_session))
            {
                self.layout_transport();
            } else {
                self.place(
                    STATUS,
                    Rect {
                        x: 55.,
                        y: self.height - 27.,
                        w: (self.width - 110.).max(1.),
                        h: 23.,
                    },
                    self.status_height() > 5.,
                );
                if self.audio.as_ref().is_some_and(Audio::has_session) {
                    self.place(
                        QUICK_SETTINGS,
                        Rect {
                            x: self.width - 190.,
                            y: 74.,
                            w: 176.,
                            h: 32.,
                        },
                        self.panel == Panel::None,
                    );
                    self.place(
                        QUICK_PAUSE,
                        Rect {
                            x: self.width - 190.,
                            y: 38.,
                            w: 176.,
                            h: 32.,
                        },
                        self.panel == Panel::None,
                    );
                }
            }
            // A brief overlay never consumes reader height or replaces the
            // current recitation state. Dismissing it leaves book geometry intact.
            let notice_width = (self.width - 48.).min(620.);
            let notice_height = if self.text(NOTICE).chars().count() > 80 {
                64.
            } else {
                28.
            };
            self.place(
                NOTICE,
                Rect {
                    x: (self.width - notice_width) / 2.,
                    y: self.height - notice_height - 12.,
                    w: notice_width,
                    h: notice_height,
                },
                self.notice_until.is_some(),
            );
            // Commit child positions before asking RichEdit for its new client
            // size. All queued HWNDs belong to this one parent window.
            #[cfg(feature = "qa")]
            let commit_began = Instant::now();
            if let Some(batch) = self.position_batch.take() {
                if EndDeferWindowPos(batch).is_err() {
                    self.position_batch_failed.set(true);
                }
            }
            if self.position_batch_failed.replace(false) {
                // No queued move is trusted after a batch failure. Reapply the
                // complete intended layout with the ordinary native path. The
                // stored rect already includes panel scroll, so replay directly.
                let intended = std::mem::take(&mut *self.placements.borrow_mut());
                for (id, (rect, visible)) in intended {
                    let handle = self.controls[&id];
                    if SetWindowPos(
                        handle,
                        None,
                        (rect.x * scale) as i32,
                        (rect.y * scale) as i32,
                        (rect.w * scale).max(1.) as i32,
                        (rect.h * scale).max(1.) as i32,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
                    )
                    .is_ok()
                    {
                        let shown =
                            GetWindowLongPtrW(handle, GWL_STYLE) & WS_VISIBLE.0 as isize != 0;
                        if shown != visible {
                            let _ = ShowWindow(handle, if visible { SW_SHOW } else { SW_HIDE });
                        }
                        if visible {
                            let _ = InvalidateRect(Some(handle), None, false);
                        }
                        self.placements.borrow_mut().insert(id, (rect, visible));
                    }
                }
            }
            // Moving/showing a native dropdown can select its edit text again.
            // Clear only an unfocused, closed reciter after the position commit,
            // before a motion snapshot is painted. Active editing stays native.
            if self.displayed_panel() == Panel::Listen {
                self.clear_unfocused_reciter_selection();
            }
            #[cfg(feature = "qa")]
            {
                self.last_commit_ms = commit_began.elapsed().as_secs_f64() * 1000.;
            }
            if self.displayed_panel() == Panel::Study {
                let mut client = RECT::default();
                GetClientRect(self.controls[&STUDY], &mut client).ok();
                let rect = RECT {
                    left: (16. * scale) as i32,
                    top: (12. * scale) as i32,
                    right: (client.right - (16. * scale) as i32).max((48. * scale) as i32),
                    bottom: (client.bottom - (12. * scale) as i32).max((40. * scale) as i32),
                };
                if self.study_format_rect.get() != Some(rect) {
                    SendMessageW(
                        self.controls[&STUDY],
                        EM_SETRECT,
                        None,
                        Some(LPARAM((&rect as *const RECT) as isize)),
                    );
                    self.study_format_rect.set(Some(rect));
                }
            }
            if self.displayed_panel() == Panel::Storage {
                self.layout_storage_padding();
            }
            if self.displayed_panel() == Panel::Tafsirs {
                let info = self.controls[&TAFSIR_INFO];
                let mut client = RECT::default();
                GetClientRect(info, &mut client).ok();
                let rect = RECT {
                    left: (12. * scale) as i32,
                    top: (10. * scale) as i32,
                    right: client.right - (12. * scale) as i32,
                    bottom: client.bottom - (10. * scale) as i32,
                };
                let mut current = RECT::default();
                SendMessageW(
                    info,
                    EM_GETRECT,
                    None,
                    Some(LPARAM((&mut current as *mut RECT) as isize)),
                );
                if current != rect {
                    SendMessageW(
                        info,
                        EM_SETRECT,
                        None,
                        Some(LPARAM((&rect as *const RECT) as isize)),
                    );
                }
            }
            if let Some(p) = &self.popup {
                let rect = p.display_rect();
                // Underlying HWND visibility was resolved before placement.
                self.place(
                    MENU_CLOSE,
                    Rect {
                        x: rect.x + rect.w - 40.,
                        y: rect.y + 7.,
                        w: 32.,
                        h: 32.,
                    },
                    true,
                );
                self.place(
                    MENU_BACK,
                    Rect {
                        x: rect.x + 8.,
                        y: rect.y + 7.,
                        w: 32.,
                        h: 32.,
                    },
                    !p.stack.is_empty(),
                );
                self.set_text(MENU_CLOSE, self.tr("Close menu", "إغلاق القائمة"));
                self.set_text(MENU_BACK, self.tr("Back", "رجوع"));
                let mut y = rect.y + 48.;
                for (i, row) in p.rows.iter().enumerate() {
                    if p.context && p.listening_expanded && row.id == BOOKMARK {
                        continue;
                    }
                    let h = if row.detail.is_empty() { 38. } else { 60. };
                    let id = MENU_ROW + i as u32;
                    self.set_text(
                        id,
                        &format!(
                            "{}{}{}",
                            row.label,
                            if row.detail.is_empty() { "" } else { ". " },
                            row.detail
                        ),
                    );
                    let split = p.context && p.stack.is_empty();
                    let bounds = if split && i == 2 {
                        Rect {
                            x: if self.prefs.arabic {
                                rect.x + 8.
                            } else {
                                rect.x + rect.w - 48.
                            },
                            y: y - 60.,
                            w: 40.,
                            h: 56.,
                        }
                    } else {
                        Rect {
                            x: rect.x
                                + 8.
                                + if split && i == 1 && self.prefs.arabic {
                                    48.
                                } else {
                                    0.
                                },
                            y,
                            w: rect.w - 16. - if split && i == 1 { 48. } else { 0. },
                            h: h - 4.,
                        }
                    };
                    let visible = bounds.y + bounds.h <= rect.y + rect.h - 8.;
                    if row.id == THEME_SELECTOR && p.docked {
                        self.place(
                            THEME_LABEL,
                            Rect {
                                x: bounds.x + 8.,
                                y: bounds.y + 1.,
                                w: bounds.w - 16.,
                                h: 20.,
                            },
                            visible,
                        );
                        self.place(
                            THEME_SELECTOR,
                            Rect {
                                x: bounds.x + 8.,
                                y: bounds.y + 25.,
                                w: bounds.w - 16.,
                                h: 160.,
                            },
                            visible,
                        );
                    } else {
                        self.place(id, bounds, visible);
                    }
                    if !(split && i == 2) {
                        y += h;
                    }
                }
            }
            self.layout_guide();
            for (&id, &handle) in &self.controls {
                if !self.placed.borrow().contains(&id) {
                    if GetWindowLongPtrW(handle, GWL_STYLE) & WS_VISIBLE.0 as isize != 0 {
                        let _ = ShowWindow(handle, SW_HIDE);
                    }
                    self.placements.borrow_mut().remove(&id);
                }
            }
            self.refresh_tooltips();
            if caption_only {
                let rect = RECT {
                    left: 0,
                    top: 0,
                    right: ((if self.fullscreen { 156. } else { self.width }) * self.dpi / 96.)
                        .ceil() as i32,
                    bottom: ((if self.fullscreen { self.height } else { 36. }) * self.dpi / 96.)
                        .ceil() as i32,
                };
                let _ = InvalidateRect(Some(self.hwnd), Some(&rect), false);
            } else if self.panel_animation.is_some()
                && self.title_animation.is_none()
                && !self.fullscreen
                && !self.renderer.surface_dirty
            {
                let rect = RECT {
                    left: 0,
                    top: (36. * self.dpi / 96.).floor() as i32,
                    right: (self.width * self.dpi / 96.).ceil() as i32,
                    bottom: (self.height * self.dpi / 96.).ceil() as i32,
                };
                let _ = InvalidateRect(Some(self.hwnd), Some(&rect), false);
            } else {
                self.invalidate();
            }
        }
        #[cfg(feature = "qa")]
        {
            self.last_layout_ms = layout_began.elapsed().as_secs_f64() * 1000.;
            self.layout_generation = self.layout_generation.wrapping_add(1);
        }
    }
    fn transport_top(&self) -> f32 {
        self.height - 186.
    }
    unsafe fn study_overflows(&self) -> bool {
        unsafe {
            let control = self.controls[&STUDY];
            let length = GetWindowTextLengthW(control);
            if length <= 0 {
                return false;
            }
            let mut format = RECT::default();
            SendMessageW(
                control,
                EM_GETRECT,
                None,
                Some(LPARAM((&mut format as *mut RECT) as isize)),
            );
            let mut first = POINT::default();
            let mut last = POINT::default();
            for (index, point) in [(0, &mut first), (length - 1, &mut last)] {
                SendMessageW(
                    control,
                    EM_POSFROMCHAR,
                    Some(WPARAM((point as *mut POINT) as usize)),
                    Some(LPARAM(index as isize)),
                );
            }
            first.y < format.top
                || last.y + ((self.study_size as f32 + 4.) * self.dpi / 96.) as i32 > format.bottom
        }
    }
    fn embedded_transport(&self) -> bool {
        self.side_playback() && !self.player_collapsed
    }
    fn refresh_tooltips(&mut self) {
        let collapsed = self.player_collapsed
            || (self.displayed_panel() == Panel::None && self.compact_player_frame().is_none());
        let labels = [
            (
                STUDY_OPTIONS,
                self.tr("Reading options", "خيارات القراءة").to_owned(),
            ),
            (TEXT_SIZE, self.tr("Text size", "حجم النص").to_owned()),
            (
                STUDY_FOLLOW,
                self.tr(
                    "Follow recitation in Tafsir. Choosing an ayah manually turns this off.",
                    "مزامنة التفسير مع التلاوة. يتوقف التزامن عند اختيار آية يدويًا.",
                )
                .to_owned(),
            ),
            (
                FOLLOW_AUDIO,
                self.tr("Go to current ayah", "انتقل إلى الآية الحالية")
                    .to_owned(),
            ),
            (
                COLLAPSE_PLAYER,
                self.tr(
                    if collapsed {
                        "Expand recitation player"
                    } else {
                        "Move player to top bar"
                    },
                    if collapsed {
                        "إظهار مشغّل التلاوة"
                    } else {
                        "نقل المشغّل إلى الشريط العلوي"
                    },
                )
                .to_owned(),
            ),
            (
                QUICK_SETTINGS,
                self.tr("Listening settings", "إعدادات الاستماع").to_owned(),
            ),
            (
                QUICK_STOP,
                self.tr("End recitation", "إنهاء التلاوة").to_owned(),
            ),
            (
                SEEK_BACK,
                self.tr("Back 10 seconds", "الرجوع ١٠ ثوانٍ").to_owned(),
            ),
            (
                SEEK_FORWARD,
                self.tr("Forward 10 seconds", "التقدم ١٠ ثوانٍ").to_owned(),
            ),
            (
                QUICK_PAUSE,
                self.tr(
                    "Pause or resume recitation · Space",
                    "إيقاف التلاوة مؤقتًا أو متابعتها · مسافة",
                )
                .to_owned(),
            ),
            (
                FULLSCREEN,
                self.tr(
                    if self.fullscreen {
                        "Exit full screen · F11"
                    } else {
                        "Full screen · F11"
                    },
                    if self.fullscreen {
                        "الخروج من ملء الشاشة · F11"
                    } else {
                        "ملء الشاشة · F11"
                    },
                )
                .to_owned(),
            ),
        ];
        let palette = self.palette();
        let active = self.popup.is_none() && self.guide.is_none() && !self.search;
        if let Some(tips) = &mut self.tooltips {
            for (id, text) in labels {
                if !text.is_empty() {
                    let _ = tips.add_or_update(self.controls[&id], &text, self.prefs.arabic);
                }
            }
            let _ = tips.set_appearance(
                self.dpi,
                self.font,
                palette.background_rgb(),
                palette.ink_rgb(),
            );
            tips.set_active(active);
        }
    }
    fn side_playback(&self) -> bool {
        self.width >= 950. && self.audio.as_ref().is_some_and(Audio::has_session)
    }
    fn status_height(&self) -> f32 {
        if (self.audio.as_ref().is_some_and(Audio::has_session) && !self.side_playback())
            || self.daily.is_some()
            || self.download.is_some()
        {
            30.
        } else {
            2.
        }
    }
    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }
    fn invalidate_reader(&self) {
        let area = self.reader_area();
        let scale = self.dpi / 96.;
        let rect = RECT {
            left: (area.x * scale).floor() as i32,
            top: (area.y * scale).floor() as i32,
            right: ((area.x + area.w) * scale).ceil() as i32,
            bottom: ((area.y + area.h) * scale).ceil() as i32,
        };
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), Some(&rect), false);
        }
    }
    fn palette(&self) -> Palette {
        if self.theme_start.is_some() {
            self.theme_frame
                .unwrap_or_else(|| Palette::new(self.prefs.theme))
        } else {
            Palette::new(self.prefs.theme)
        }
    }
    fn advance_theme(&mut self, now: Instant) -> bool {
        let Some((start, old)) = self.theme_start else {
            return false;
        };
        let t = (now.saturating_duration_since(start).as_secs_f32() / 0.18).min(1.);
        self.theme_frame = Some(old.blend(Palette::new(self.prefs.theme), t * t * (3. - 2. * t)));
        if t >= 1. {
            self.theme_start = None;
            self.theme_frame = None;
            unsafe {
                KillTimer(Some(self.hwnd), 2).ok();
            }
        }
        self.replace_background(self.palette());
        self.redraw_controls();
        true
    }
    fn redraw_controls(&self) {
        unsafe {
            let _ = RedrawWindow(
                Some(self.hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ALLCHILDREN,
            );
        }
    }
    unsafe fn report_error(&mut self, error: &dyn std::fmt::Display) {
        unsafe {
            let text = format!(
                "{}{}",
                self.tr(
                    "Unable to complete this action: ",
                    "تعذر إتمام هذا الإجراء: "
                ),
                error
            );
            self.notice(text);
        }
    }
    fn theme(&mut self, number: u32) {
        let previous = self.palette();
        let mut animate = BOOL::from(true);
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some((&mut animate as *mut BOOL).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
        }
        self.theme_start = animate.as_bool().then(|| (Instant::now(), previous));
        self.theme_frame = self.theme_start.map(|_| previous);
        self.prefs.theme = number;
        unsafe {
            SendMessageW(
                self.controls[&THEME_SELECTOR],
                CB_SETCURSEL,
                Some(WPARAM(number.min(3) as usize)),
                None,
            );
        }
        self.dirty = true;
        unsafe {
            self.replace_background(self.palette());
            let dark = BOOL::from(number == 0 || number == 3);
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&dark as *const BOOL).cast(),
                std::mem::size_of::<BOOL>() as u32,
            );
            if self.theme_start.is_some() {
                SetTimer(Some(self.hwnd), 2, 10, None);
            }
        }
        self.redraw_controls();
    }
    fn replace_background(&mut self, palette: Palette) {
        unsafe {
            let brush = CreateSolidBrush(COLORREF(palette.background_rgb()));
            if brush.is_invalid() {
                return;
            }
            let old = self.background;
            self.background = brush;
            // Publish the new handle before releasing the previous brush.
            cache_control_colors(brush, palette);
            if let Some(&info) = self.controls.get(&TAFSIR_INFO) {
                SendMessageW(
                    info,
                    EM_SETBKGNDCOLOR,
                    None,
                    Some(LPARAM(palette.background_rgb() as isize)),
                );
            }
            if let Some(&study) = self.controls.get(&STUDY) {
                let dark = palette.background.r < 0.3;
                // Switching Explorer scrollbar themes cannot be interpolated.
                // Commit that native frame once the shared fade has landed.
                if self.theme_start.is_none() && self.study_control_dark != Some(dark) {
                    SetWindowTheme(
                        self.controls[&RESULTS],
                        if dark {
                            w!("DarkMode_Explorer")
                        } else {
                            w!("Explorer")
                        },
                        None,
                    )
                    .ok();
                    SetWindowTheme(
                        study,
                        if dark {
                            w!("DarkMode_Explorer")
                        } else {
                            w!("Explorer")
                        },
                        None,
                    )
                    .ok();
                    self.study_control_dark = Some(dark);
                    let _ = RedrawWindow(Some(study), None, None, RDW_INVALIDATE | RDW_FRAME);
                }
                SendMessageW(
                    study,
                    EM_SETBKGNDCOLOR,
                    None,
                    Some(LPARAM(crate::render::color_rgb(palette.paper) as isize)),
                );
                self.format_study_color(palette);
            }
            let _ = DeleteObject(old.into());
        }
    }
    unsafe fn format_study_color(&self, palette: Palette) {
        unsafe {
            let format = CHARFORMAT2W {
                Base: CHARFORMATW {
                    cbSize: std::mem::size_of::<CHARFORMAT2W>() as u32,
                    dwMask: CFM_COLOR,
                    crTextColor: COLORREF(palette.ink_rgb()),
                    ..Default::default()
                },
                ..Default::default()
            };
            // Set the default as well as existing text: WM_SETTEXT otherwise
            // restores RichEdit's default black ink on the next source/ayah.
            for (control, scope) in [STUDY, TAFSIR_INFO]
                .into_iter()
                .flat_map(|control| [0, SCF_ALL as usize].map(|scope| (control, scope)))
            {
                SendMessageW(
                    self.controls[&control],
                    EM_SETCHARFORMAT,
                    Some(WPARAM(scope)),
                    Some(LPARAM((&format as *const CHARFORMAT2W) as isize)),
                );
            }
            self.format_study_attribution(palette);
        }
    }
    unsafe fn paint(&mut self) -> Result<()> {
        unsafe {
            #[cfg(feature = "qa")]
            let measured = Instant::now();
            #[cfg(feature = "qa")]
            {
                self.motion_samples = [0.; 3];
            }
            let mut ps = PAINTSTRUCT::default();
            // Read the actual surface size: nested window messages may have
            // arrived while a command held App's exclusive dispatch guard.
            let mut client = RECT::default();
            GetClientRect(self.hwnd, &mut client)?;
            let dpi = GetDpiForWindow(self.hwnd).max(96) as f32;
            if dpi != self.dpi
                || (client.right as f32 * 96. / dpi - self.width).abs() > 0.5
                || (client.bottom as f32 * 96. / dpi - self.height).abs() > 0.5
            {
                self.resize();
            }
            BeginPaint(self.hwnd, &mut ps);
            let mut damage = RECT {
                left: 0,
                top: 0,
                right: client.right,
                bottom: client.bottom,
            };
            let mut popup_fields_in_frame = false;
            #[cfg(feature = "qa")]
            {
                self.transport_fields_in_frame = false;
            }
            let result = (|| -> Result<()> {
                let mut enabled = BOOL::from(true);
                SystemParametersInfoW(
                    SPI_GETCLIENTAREAANIMATION,
                    0,
                    Some((&mut enabled as *mut BOOL).cast()),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                )
                .ok();
                let motion_view = self.panel_motion_view();
                let fold_texture = enabled.as_bool() && self.fold_motion_enabled();
                if motion_view.is_none() {
                    if let Some(cache) = self.motion_book.take() {
                        drop(cache.bitmap);
                        let (cpu, uploaded) = self.motion_pixel_usage();
                        self.motion_cpu
                            .set_budget(crate::motion_policy::spare_cache_budget(cpu, uploaded));
                        if cache.key.selected.is_none() {
                            self.motion_cpu
                                .insert(Self::cpu_motion_key(&cache.key, false), cache.raster);
                        }
                    }
                }
                if !fold_texture {
                    if let Some(cache) = self.motion_fold.take() {
                        drop(cache.source_bitmap);
                        drop(cache.destination_bitmap);
                        drop(cache.reverse_bitmap);
                        let (cpu, uploaded) = self.motion_pixel_usage();
                        self.motion_cpu
                            .set_budget(crate::motion_policy::spare_cache_budget(cpu, uploaded));
                        let mut old_key = Self::cpu_motion_key(&cache.key, false);
                        old_key.page = cache.old;
                        self.motion_cpu
                            .insert(Self::cpu_motion_key(&cache.key, true), cache.reverse);
                        self.motion_cpu.insert(old_key, cache.source);
                        self.motion_cpu
                            .insert(Self::cpu_motion_key(&cache.key, false), cache.destination);
                    }
                }
                let target = self.renderer.window_target(
                    self.hwnd,
                    client.right.max(1) as u32,
                    client.bottom.max(1) as u32,
                    self.dpi,
                    motion_view.is_some() || fold_texture,
                )?;
                if !self.renderer.surface_dirty {
                    damage = ps.rcPaint;
                }
                let palette = self.palette();
                let mut drawing = DrawSession::begin(target.clone());
                let scale = 96. / self.dpi;
                target.PushAxisAlignedClip(
                    &Rect {
                        x: damage.left as f32 * scale,
                        y: damage.top as f32 * scale,
                        w: (damage.right - damage.left) as f32 * scale,
                        h: (damage.bottom - damage.top) as f32 * scale,
                    }
                    .d2d(),
                    windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED,
                );
                target.Clear(Some(&palette.background));
                let area = self.reader_area();
                let reader_top = (area.y * self.dpi / 96.).ceil() as i32;
                self.renderer.label(
                    &target,
                    if self.fullscreen {
                        ""
                    } else {
                        "Quran Mushaf Reader"
                    },
                    if self.fullscreen {
                        Rect {
                            x: 8.,
                            y: 40.,
                            w: 140.,
                            h: 36.,
                        }
                    } else {
                        Rect {
                            x: 32.,
                            y: 0.,
                            w: 187.,
                            h: 36.,
                        }
                    },
                    if self.fullscreen { 10. } else { 13. },
                    palette.ink,
                    false,
                )?;
                if !self.text_view
                    && damage.bottom > reader_top
                    && damage.right > (area.x * self.dpi / 96.).floor() as i32
                    && damage.left < ((area.x + area.w) * self.dpi / 96.).ceil() as i32
                {
                    #[cfg(feature = "qa")]
                    {
                        self.canvas_paints += 1;
                    }
                    let turn = self
                        .page_drag
                        .filter(|_| enabled.as_bool())
                        .map(|d| (d.old, d.new, d.direction, d.progress, d.top))
                        .or_else(|| {
                            self.page_turn.and_then(|t| {
                                let (p, done) = t.progress();
                                if done {
                                    None
                                } else {
                                    Some((t.old, t.new, t.direction, p, t.top))
                                }
                            })
                        });
                    #[cfg(feature = "qa")]
                    let book_began = Instant::now();
                    if let Some((old, new, direction, progress, top)) = turn {
                        let view = BookView {
                            page: new,
                            area,
                            palette,
                            selection: None,
                            recitation: None,
                            opening: None,
                            arabic: self.prefs.arabic,
                        };
                        let composed = if fold_texture {
                            let result =
                                self.draw_motion_fold(&target, view, old, direction, progress, top);
                            self.motion_result(result)
                        } else {
                            false
                        };
                        if !composed {
                            self.renderer.fold(
                                &target,
                                &mut self.resources,
                                view,
                                old,
                                direction,
                                progress,
                                top,
                            )?;
                        }
                    } else {
                        self.page_turn = None;
                        let selection = self.canvas_selection();
                        let view = BookView {
                            page: self.prefs.page,
                            area,
                            palette,
                            selection,
                            recitation: self.canvas_recitation(),
                            opening: self.canvas_opening(),
                            arabic: self.prefs.arabic,
                        };
                        let cached = if let Some(final_view) = motion_view {
                            let result = self.draw_motion_book(&target, final_view, view);
                            self.motion_result(result)
                        } else {
                            false
                        };
                        if !cached {
                            self.renderer.book(&target, &mut self.resources, view)?;
                        }
                    }
                    #[cfg(feature = "qa")]
                    {
                        self.motion_samples[0] = book_began.elapsed().as_secs_f64() * 1000.;
                    }
                }
                if let Some(start) = self.startup_reveal {
                    let progress = (start.elapsed().as_secs_f32() / 0.16).clamp(0., 1.);
                    let brush = target.CreateSolidColorBrush(
                        &windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
                            a: 0.18 * (1. - progress).powi(2),
                            ..palette.paper
                        },
                        None,
                    )?;
                    target.FillRectangle(&self.reader_area().d2d(), &brush);
                }
                if self.search {
                    let w = (self.width - 40.).min(600.);
                    let x = (self.width - w) / 2.;
                    let offset = self.search_offset();
                    let search_layout = crate::search_layout::SearchLayout::new(self.height);
                    let browse = self.text(QUERY).trim().is_empty();
                    let frame = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                        rect: Rect {
                            x,
                            y: 46. + offset,
                            w,
                            h: self.search_list_y() - 46. + self.search_result_height() + 16.,
                        }
                        .d2d(),
                        radiusX: 12.,
                        radiusY: 12.,
                    };
                    let fill = target.CreateSolidColorBrush(&palette.paper, None)?;
                    let border = target.CreateSolidColorBrush(&palette.border, None)?;
                    target.FillRoundedRectangle(&frame, &fill);
                    target.DrawRoundedRectangle(&frame, &border, 1., None);
                    for (text, y, size) in [
                        (self.tr("Search", "بحث"), search_layout.title, 18.),
                        (
                            self.tr("Quran, surah or page", "القرآن أو السورة أو الصفحة"),
                            search_layout.subtitle,
                            13.,
                        ),
                    ] {
                        self.renderer.label_start(
                            &target,
                            text,
                            Rect {
                                x: x + 20.,
                                y: y + offset,
                                w: w - 72.,
                                h: 24.,
                            },
                            size,
                            palette.ink,
                            self.prefs.arabic,
                        )?;
                    }
                    for (index, y) in [search_layout.query_frame, search_layout.filter_frame]
                        .into_iter()
                        .enumerate()
                    {
                        if index == 0 || browse {
                            let r = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                                rect: Rect {
                                    x: x + 12.
                                        + if index == 1
                                            && self.browse_tab == BROWSE_SECTIONS
                                            && !self.prefs.arabic
                                        {
                                            120.
                                        } else {
                                            0.
                                        },
                                    y: y + offset,
                                    w: w - 24.
                                        - if index == 1 && self.browse_tab == BROWSE_SECTIONS {
                                            120.
                                        } else {
                                            0.
                                        },
                                    h: 48.,
                                }
                                .d2d(),
                                radiusX: crate::field_style::CORNER_RADIUS,
                                radiusY: crate::field_style::CORNER_RADIUS,
                            };
                            let focused = self.keyboard_focus
                                && GetFocus()
                                    == self.controls
                                        [&if index == 0 { QUERY } else { BROWSE_QUERY }];
                            if focused {
                                let focus = target.CreateSolidColorBrush(&palette.muted, None)?;
                                target.DrawRoundedRectangle(&r, &focus, 1., None);
                            } else {
                                target.DrawRoundedRectangle(&r, &border, 1., None);
                            }
                        }
                    }
                    if browse {
                        if search_layout.browse_caption {
                            self.renderer.label_start(
                                &target,
                                self.tr("Browse and jump", "تصفح وانتقل"),
                                Rect {
                                    x: x + 20.,
                                    y: 190. + offset,
                                    w: w - 40.,
                                    h: 26.,
                                },
                                12.,
                                palette.muted,
                                self.prefs.arabic,
                            )?;
                        }
                        if search_layout.section_caption {
                            let name = match self.browse_tab {
                                BROWSE_SURAH => self.tr("Surah", "السورة"),
                                BROWSE_PAGE => self.tr("Page", "الصفحة"),
                                _ => self.tr("Juz, Hizb & Rub", "الجزء والحزب والربع"),
                            };
                            self.renderer.label_start(
                                &target,
                                name,
                                Rect {
                                    x: x + 20.,
                                    y: search_layout.section_label + offset,
                                    w: w - 40.,
                                    h: 24.,
                                },
                                13.,
                                palette.ink,
                                self.prefs.arabic,
                            )?;
                        }
                    }
                    if self.hits.is_empty()
                        && (!browse || !self.text(BROWSE_QUERY).trim().is_empty())
                    {
                        self.renderer.label_start(
                            &target,
                            self.tr("No matching results", "لا توجد نتائج مطابقة"),
                            Rect {
                                x: x + 20.,
                                y: self.search_list_y() + offset,
                                w: w - 40.,
                                h: 36.,
                            },
                            13.,
                            palette.muted,
                            self.prefs.arabic,
                        )?;
                    }
                }
                if std::mem::take(&mut self.renderer.contextual_unavailable) {
                    self.prefs.contextual = false;
                    self.dirty = true;
                    self.notice_until = Some(Instant::now() + std::time::Duration::from_secs(3));
                    self.set_text(NOTICE, self.tr(
                        "Letter elongation is unavailable. The Quran is displayed with its original font.",
                        "تعذر عرض إطالة الحروف. يُعرض المصحف بخطه الأصلي.",
                    ));
                }
                if self.displayed_panel() != Panel::None {
                    let x = self.panel_origin();
                    let r = Rect {
                        x,
                        y: 62.,
                        w: 360.,
                        h: self.height - 94.,
                    };
                    let brush = target.CreateSolidColorBrush(&palette.border, None)?;
                    target.DrawRoundedRectangle(
                        &windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                            rect: r.d2d(),
                            radiusX: 10.,
                            radiusY: 10.,
                        },
                        &brush,
                        1.,
                        None,
                    );
                    let title = if self.displayed_panel() == Panel::Storage {
                        self.tr("Download details", "تفاصيل التنزيل")
                    } else if self.displayed_panel() == Panel::Tafsirs {
                        self.tr("Manage tafsirs", "إدارة التفاسير")
                    } else {
                        self.tr(
                            if self.displayed_panel() == Panel::Study {
                                "Tafsir"
                            } else {
                                "Listen"
                            },
                            if self.displayed_panel() == Panel::Study {
                                "التفسير"
                            } else {
                                "الاستماع"
                            },
                        )
                    };
                    if self.displayed_panel() == Panel::Study {
                        let panel_w = 360f32.min((self.width - 180.).max(250.));
                        self.renderer.label_start(
                            &target,
                            title,
                            Rect {
                                x: self.panel_origin()
                                    + if self.prefs.arabic {
                                        panel_w - 84.
                                    } else {
                                        16.
                                    },
                                y: 64.,
                                w: 68.,
                                h: 32.,
                            },
                            18.,
                            palette.ink,
                            self.prefs.arabic,
                        )?;
                    } else {
                        self.renderer.label(
                            &target,
                            title,
                            Rect {
                                x,
                                y: 64.,
                                w: 230.,
                                h: 32.,
                            },
                            18.,
                            palette.ink,
                            self.prefs.arabic,
                        )?;
                    }
                }
                if self.displayed_panel() == Panel::Listen {
                    let border = target.CreateSolidColorBrush(&palette.border, None)?;
                    let focus = target.CreateSolidColorBrush(&palette.muted, None)?;
                    for id in [FIRST, LAST] {
                        if let Some((r, true)) = self.placements.borrow().get(&id) {
                            let frame = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                                rect: Rect {
                                    x: r.x - 4.,
                                    y: r.y - 4.,
                                    w: r.w + 8.,
                                    h: r.h + 8.,
                                }
                                .d2d(),
                                radiusX: crate::field_style::CORNER_RADIUS,
                                radiusY: crate::field_style::CORNER_RADIUS,
                            };
                            target.DrawRoundedRectangle(
                                &frame,
                                if self.keyboard_focus && GetFocus() == self.controls[&id] {
                                    &focus
                                } else {
                                    &border
                                },
                                1.,
                                None,
                            );
                        }
                    }
                    if let Some((volume, true)) = self.placements.borrow().get(&VOLUME) {
                        for (text, x) in
                            [("0%", volume.x - 36.), ("100%", volume.x + volume.w + 4.)]
                        {
                            self.renderer.label(
                                &target,
                                text,
                                Rect {
                                    x,
                                    y: volume.y + 2.,
                                    w: 32.,
                                    h: 20.,
                                },
                                11.,
                                palette.muted,
                                false,
                            )?;
                        }
                    }
                }
                if self.displayed_panel() == Panel::Tafsirs {
                    let x = self.panel_origin() + 16.;
                    let w = 360f32.min((self.width - 180.).max(250.)) - 32.;
                    let fill = target.CreateSolidColorBrush(&palette.paper, None)?;
                    let mut faint = palette.border;
                    faint.a = 0.65;
                    let border = target.CreateSolidColorBrush(&faint, None)?;
                    for (row, i) in self.tafsir_rows().into_iter().enumerate() {
                        let source = &tafsir_library::SOURCES[i];
                        let y = 150. + row as f32 * 66.;
                        let frame = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                            rect: Rect { x, y, w, h: 58. }.d2d(),
                            radiusX: 8.,
                            radiusY: 8.,
                        };
                        target.FillRoundedRectangle(&frame, &fill);
                        target.DrawRoundedRectangle(&frame, &border, 1., None);
                        let label_x = x + if self.prefs.arabic { 108. } else { 12. };
                        self.renderer.label_start(
                            &target,
                            if self.prefs.arabic {
                                source.name_arabic
                            } else {
                                source.name_english
                            },
                            Rect {
                                x: label_x,
                                y: y + 8.,
                                w: w - 120.,
                                h: 26.,
                            },
                            14.,
                            palette.ink,
                            self.prefs.arabic,
                        )?;
                        let status = if source.download_issue().is_some()
                            && !self.installed_tafsirs.contains(&i)
                        {
                            self.tr(
                                "Publisher mapping needs correction",
                                "بيانات الآيات تحتاج إلى تصحيح",
                            )
                        } else if self.tafsir_job.is_some() && self.tafsir_job_source == Some(i) {
                            self.tr("Updating…", "جارٍ التحديث…")
                        } else if self.installed_tafsirs.contains(&i) {
                            self.tr("Downloaded", "منزّل")
                        } else {
                            self.tr("Available to download", "متاح للتنزيل")
                        };
                        self.renderer.label_start(
                            &target,
                            status,
                            Rect {
                                x: label_x,
                                y: y + 30.,
                                w: w - 120.,
                                h: 24.,
                            },
                            12.,
                            palette.muted,
                            self.prefs.arabic,
                        )?;
                    }
                }
                if self.displayed_panel() == Panel::Study {
                    let x = self.panel_origin() + 16.;
                    let (s, a) = self.selected.unwrap_or((1, 1));
                    let chapter = self
                        .resources
                        .surahs
                        .get(s as usize - 1)
                        .map(|v| {
                            if self.prefs.arabic {
                                v.name_arabic.as_str()
                            } else {
                                v.name_simple.as_str()
                            }
                        })
                        .unwrap_or("");
                    self.renderer.label(
                        &target,
                        &format!("{chapter} · {} {a}", self.tr("Ayah", "الآية")),
                        Rect {
                            x: x + 40.,
                            y: 137.,
                            w: 248.,
                            h: 32.,
                        },
                        14.,
                        palette.ink,
                        self.prefs.arabic,
                    )?;
                    let border = target.CreateSolidColorBrush(&palette.border, None)?;
                    for y in [100., 168.] {
                        target.DrawLine(
                            windows_numerics::Vector2 { X: x - 16., Y: y },
                            windows_numerics::Vector2 { X: x + 344., Y: y },
                            &border,
                            1.,
                            None,
                        );
                    }
                    if let Some((body, true)) = self.placements.borrow().get(&STUDY) {
                        let rect = Rect {
                            x: body.x - 8.,
                            y: body.y - 8.,
                            w: body.w + 16.,
                            h: body.h + 16.,
                        };
                        let frame = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                            rect: rect.d2d(),
                            radiusX: 8.,
                            radiusY: 8.,
                        };
                        let fill = target.CreateSolidColorBrush(&palette.paper, None)?;
                        target.FillRoundedRectangle(&frame, &fill);
                        target.DrawRoundedRectangle(&frame, &border, 0.75, None);
                        if !self.paint_study_legend(&target, *body, palette)?
                            && self.study_overflows()
                        {
                            self.renderer.label(
                                &target,
                                self.tr("Scroll to read", "مرّر للقراءة"),
                                Rect {
                                    x: body.x,
                                    y: body.y + body.h + 8.,
                                    w: body.w,
                                    h: 18.,
                                },
                                11.,
                                palette.muted,
                                self.prefs.arabic,
                            )?;
                        }
                    }
                    if let Some((footer, true)) = self.placements.borrow().get(&STUDY_ATTRIBUTION) {
                        target.DrawLine(
                            windows_numerics::Vector2 {
                                X: footer.x,
                                Y: footer.y - 5.,
                            },
                            windows_numerics::Vector2 {
                                X: footer.x + footer.w,
                                Y: footer.y - 5.,
                            },
                            &border,
                            1.,
                            None,
                        );
                    }
                }
                if self.embedded_transport()
                    && self.displayed_panel() != Panel::None
                    && self.displayed_panel() != Panel::Listen
                {
                    let rect = Rect {
                        x: self.panel_origin() + 8.,
                        y: self.transport_top() - 8.,
                        w: 344.,
                        h: 154.,
                    };
                    let frame = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                        rect: rect.d2d(),
                        radiusX: 10.,
                        radiusY: 10.,
                    };
                    let paper = target.CreateSolidColorBrush(&palette.background, None)?;
                    let border = target.CreateSolidColorBrush(&palette.border, None)?;
                    target.FillRoundedRectangle(&frame, &paper);
                    target.DrawRoundedRectangle(&frame, &border, 1., None);
                }
                if self.side_playback()
                    && self.displayed_panel() == Panel::None
                    && !self.player_collapsed
                {
                    if let Some(rect) = self.compact_player_frame() {
                        let frame = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                            rect: rect.d2d(),
                            radiusX: 10.,
                            radiusY: 10.,
                        };
                        let border = target.CreateSolidColorBrush(&palette.border, None)?;
                        let paper = target.CreateSolidColorBrush(&palette.paper, None)?;
                        target.FillRoundedRectangle(&frame, &paper);
                        target.DrawRoundedRectangle(&frame, &border, 1., None);
                    }
                }
                if let Some(p) = &self.popup {
                    let rect = p.display_rect();
                    let brush = target.CreateSolidColorBrush(&palette.paper, None)?;
                    let border = target.CreateSolidColorBrush(&palette.border, None)?;
                    let r = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
                        rect: rect.d2d(),
                        radiusX: 12.,
                        radiusY: 12.,
                    };
                    target.FillRoundedRectangle(&r, &brush);
                    target.DrawRoundedRectangle(&r, &border, 1., None);
                    self.renderer.label(
                        &target,
                        &p.title,
                        Rect {
                            x: rect.x + 44.,
                            y: rect.y + 8.,
                            w: rect.w - 90.,
                            h: 32.,
                        },
                        14.,
                        palette.ink,
                        self.prefs.arabic,
                    )?;
                }
                self.paint_guide(&target, palette)?;
                if self.side_playback() && self.popup.is_none() {
                    let mut bounds = RECT {
                        left: client.right,
                        top: client.bottom,
                        right: 0,
                        bottom: 0,
                    };
                    for id in [
                        STATUS,
                        AUDIO_TIME,
                        AUDIO_POSITION,
                        SEEK_BACK,
                        SEEK_FORWARD,
                        QUICK_PAUSE,
                        QUICK_STOP,
                        FOLLOW_AUDIO,
                        QUICK_SETTINGS,
                        COLLAPSE_PLAYER,
                    ] {
                        if let Some((r, true)) = self.placements.borrow().get(&id) {
                            let scale = self.dpi / 96.;
                            bounds.left = bounds.left.min((r.x * scale).floor() as i32);
                            bounds.top = bounds.top.min((r.y * scale).floor() as i32);
                            bounds.right = bounds.right.max(((r.x + r.w) * scale).ceil() as i32);
                            bounds.bottom = bounds.bottom.max(((r.y + r.h) * scale).ceil() as i32);
                        }
                    }
                    if bounds.right > bounds.left && bounds.bottom > bounds.top {
                        target.PopAxisAlignedClip();
                        let _complete = self.compose_controls(bounds, Some(&target));
                        #[cfg(feature = "qa")]
                        {
                            self.transport_fields_in_frame = _complete;
                        }
                        target.PushAxisAlignedClip(
                            &Rect {
                                x: damage.left as f32 * scale,
                                y: damage.top as f32 * scale,
                                w: (damage.right - damage.left) as f32 * scale,
                                h: (damage.bottom - damage.top) as f32 * scale,
                            }
                            .d2d(),
                            windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED,
                        );
                    }
                }
                if self.popup.is_some() {
                    // Present covers the complete backing buffer. Refresh all
                    // native fields there too, including focus/selection changes
                    // outside this row's ordinary invalidation rectangle.
                    target.PopAxisAlignedClip();
                    popup_fields_in_frame = self.compose_controls(client, Some(&target));
                    #[cfg(feature = "qa")]
                    {
                        self.popup_fields_in_frame = popup_fields_in_frame;
                    }
                    target.PushAxisAlignedClip(
                        &Rect {
                            x: damage.left as f32 * scale,
                            y: damage.top as f32 * scale,
                            w: (damage.right - damage.left) as f32 * scale,
                            h: (damage.bottom - damage.top) as f32 * scale,
                        }
                        .d2d(),
                        windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED,
                    );
                }
                self.paint_popup_controls(&target)?;
                target.PopAxisAlignedClip();
                #[cfg(feature = "qa")]
                let end_began = Instant::now();
                drawing.finish()?;
                if let Some(turn) = self.page_turn.as_mut().filter(|turn| turn.first_frame) {
                    turn.first_frame = false;
                    turn.start = Instant::now();
                }
                #[cfg(feature = "qa")]
                {
                    self.motion_samples[1] = end_began.elapsed().as_secs_f64() * 1000.;
                }
                #[cfg(feature = "qa")]
                if damage.bottom <= reader_top && self.title_animation.is_some() {
                    if let Some((start, frames)) = &mut self.title_trace {
                        frames.push((
                            start.elapsed().as_secs_f64() * 1000.,
                            measured.elapsed().as_secs_f64() * 1000.,
                        ));
                    }
                }
                self.renderer.surface_dirty = false;
                Ok(())
            })();
            let _ = EndPaint(self.hwnd, &ps);
            if result.is_ok() {
                #[cfg(feature = "qa")]
                let compose_began = Instant::now();
                if !popup_fields_in_frame {
                    // A bounded snapshot failure falls back to the native
                    // controls, repainting all of them after a popup present.
                    self.compose_controls(if self.popup.is_some() { client } else { damage }, None);
                }
                #[cfg(feature = "qa")]
                {
                    self.motion_samples[2] = compose_began.elapsed().as_secs_f64() * 1000.;
                }
                self.schedule_prewarm();
            }
            if result.is_err() {
                self.motion_book = None;
                self.motion_fold = None;
                self.chrome_failures.set(self.chrome_failures.get() + 1);
                self.renderer.discard_target();
            }
            if result.is_ok() && !self.first_paint {
                self.first_paint = true;
                #[cfg(feature = "qa")]
                {
                    self.first_paint_ms = self.began.elapsed().as_secs_f64() * 1000.;
                }
                #[cfg(feature = "qa")]
                if let Some(path) = qa_report() {
                    std::fs::write(
                        path,
                        serde_json::to_vec_pretty(
                            &serde_json::json!({"firstPaintMs":self.began.elapsed().as_secs_f64()*1000.,"pid":std::process::id(),"webviewDependency":false}),
                        )?,
                    )?;
                }
            }
            #[cfg(feature = "qa")]
            {
                let (cpu, uploaded) = self.motion_pixel_usage();
                let bytes = cpu
                    .saturating_add(uploaded)
                    .saturating_add(self.motion_cpu.pixel_bytes());
                self.maximum_motion_pixel_bytes = self.maximum_motion_pixel_bytes.max(bytes);
                if bytes > crate::motion_policy::MAX_TOTAL_PIXEL_BYTES {
                    self.motion_errors
                        .push("Motion pixels exceeded the bounded cache budget".into());
                }
            }
            #[cfg(feature = "qa")]
            if let Some((start, samples)) = &mut self.motion_trace {
                let fresh_layout = self.layout_generation != self.painted_layout_generation;
                samples.push([
                    measured.duration_since(*start).as_secs_f64() * 1000.,
                    measured.elapsed().as_secs_f64() * 1000.,
                    if fresh_layout {
                        self.last_layout_ms
                    } else {
                        0.
                    },
                    self.motion_samples[0],
                    self.motion_samples[2],
                    self.motion_samples[1],
                    if fresh_layout {
                        self.last_commit_ms
                    } else {
                        0.
                    },
                ]);
                self.painted_layout_generation = self.layout_generation;
            }
            if result.is_ok() && self.study_reveal_pending {
                if let Some((start, _, _)) = &mut self.panel_animation {
                    *start = Instant::now();
                }
                self.study_reveal_pending = false;
            }
            result
        }
    }
    /// Direct2D presents the entire parent surface. While a popup disables
    /// child clipping, copy native fields into that same backing buffer before
    /// Present; partial hover paints then retain their pixels. Ordinary frames
    /// keep native child clipping and composition. Never reborrow App through
    /// synchronous WM_DRAWITEM while its dispatch guard is held.
    unsafe fn compose_controls(
        &self,
        damage: RECT,
        target: Option<&windows::Win32::Graphics::Direct2D::ID2D1RenderTarget>,
    ) -> bool {
        unsafe {
            let mut complete = true;
            let palette = self.palette();
            let palette_key = [
                palette.background,
                palette.paper,
                palette.ink,
                palette.muted,
                palette.border,
                palette.highlight,
            ]
            .map(|c| [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()]);
            let cached_controls = target.is_some()
                || !qa_disabled("--qa-live-controls")
                    && self.theme_start.is_none()
                    && (self.panel_animation.is_some()
                        || self.title_animation.is_some()
                        || self.search_animation.is_some()
                        || self.popup.as_ref().is_some_and(|p| p.motion.is_some()));
            if !cached_controls || self.theme_start.is_some() {
                self.motion_controls.borrow_mut().clear();
            }
            for (&id, &child) in &self.controls {
                if (MENU_ROW..=MENU_CLOSE).contains(&id) {
                    continue;
                }
                if !IsWindowVisible(child).as_bool() {
                    continue;
                }
                let mut bounds = RECT::default();
                if GetWindowRect(child, &mut bounds).is_ok() {
                    let mut origin = POINT { x: 0, y: 0 };
                    let _ = ClientToScreen(self.hwnd, &mut origin);
                    if bounds.right - origin.x <= damage.left
                        || bounds.left - origin.x >= damage.right
                        || bounds.bottom - origin.y <= damage.top
                        || bounds.top - origin.y >= damage.bottom
                    {
                        continue;
                    }
                }
                let mut name = [0u16; 32];
                let count = GetClassNameW(child, &mut name);
                let button = String::from_utf16_lossy(&name[..count.max(0) as usize])
                    .eq_ignore_ascii_case("BUTTON")
                    && GetWindowLongPtrW(child, GWL_STYLE) as u32 & 15 == BS_OWNERDRAW as u32;
                let input_focused = GetFocus() == child || IsChild(child, GetFocus()).as_bool();
                if cached_controls
                    && (target.is_some()
                        || !matches!(id, STUDY | TAFSIR_INFO | QUERY | BROWSE_QUERY))
                    && (target.is_some() || button || !input_focused)
                {
                    let mut rect = RECT::default();
                    GetClientRect(child, &mut rect).ok();
                    if target.is_some() {
                        let mut window = RECT::default();
                        if GetWindowRect(child, &mut window).is_ok() {
                            rect.right = window.right - window.left;
                            rect.bottom = window.bottom - window.top;
                        }
                    }
                    let mut cursor = POINT::default();
                    GetCursorPos(&mut cursor).ok();
                    let key = MotionControlKey {
                        width: rect.right,
                        height: rect.bottom,
                        dpi: self.dpi.to_bits(),
                        theme: self.prefs.theme,
                        palette: palette_key,
                        rtl: self.prefs.arabic,
                        text: self
                            .popup
                            .as_ref()
                            .and_then(|p| p.rows.get(id.wrapping_sub(MENU_ROW) as usize))
                            .map(|row| format!("{}\n{}", row.label, row.detail))
                            .unwrap_or_else(|| self.text(id)),
                        enabled: IsWindowEnabled(child).as_bool(),
                        focused: GetFocus() == child,
                        keyboard_cues: self.keyboard_focus,
                        hot: WindowFromPoint(cursor) == child,
                        state: if button {
                            SendMessageW(child, BM_GETSTATE, None, None).0 as usize
                        } else if matches!(id, VOLUME | AUDIO_POSITION) {
                            SendMessageW(child, TBM_GETPOS, None, None).0 as usize
                        } else {
                            0
                        },
                        context: (
                            self.search,
                            self.panel == Panel::Listen,
                            self.browse_tab,
                            self.popup.as_ref().is_some_and(|p| {
                                !p.context
                                    && !p
                                        .rows
                                        .iter()
                                        .any(|r| matches!(r.id, WELCOME_ENGLISH | WELCOME_ARABIC))
                            }),
                        ),
                        reading: matches!(id, STUDY | TAFSIR_INFO).then(|| {
                            let point = crate::smooth_scroll::position(child);
                            let mut selected = CHARRANGE::default();
                            SendMessageW(
                                child,
                                EM_EXGETSEL,
                                None,
                                Some(LPARAM((&mut selected as *mut CHARRANGE) as isize)),
                            );
                            (
                                point.x,
                                point.y,
                                SendMessageW(
                                    child,
                                    windows::Win32::UI::Controls::EM_GETFIRSTVISIBLELINE,
                                    None,
                                    None,
                                )
                                .0,
                                selected.cpMin,
                                selected.cpMax,
                                SendMessageW(child, WM_GETFONT, None, None).0,
                            )
                        }),
                        results: (id == RESULTS).then(|| {
                            (
                                self.results_generation,
                                SendMessageW(child, LB_GETTOPINDEX, None, None).0,
                                SendMessageW(child, LB_GETCURSEL, None, None).0,
                            )
                        }),
                    };
                    let mut cache = self.motion_controls.borrow_mut();
                    let matches = cache.get(&id).is_some_and(|(old, _)| old == &key)
                        && !(target.is_some()
                            && (input_focused
                                || matches!(id, STUDY | TAFSIR_INFO | QUERY | BROWSE_QUERY)));
                    if !matches {
                        cache.remove(&id);
                        let wanted = (rect.right.max(0) as u64) * (rect.bottom.max(0) as u64) * 4;
                        let retained: u64 = cache.values().map(|(_, s)| s.byte_len() as u64).sum();
                        if wanted + retained <= 8 * 1024 * 1024 {
                            let snapshot = crate::motion_controls::GdiSnapshot::new(
                                rect.right,
                                rect.bottom,
                                |dc| {
                                    FillRect(dc, &rect, self.background);
                                    if button {
                                        let state =
                                            if GetFocus() == child { ODS_FOCUS.0 } else { 0 }
                                                | if SendMessageW(child, BM_GETSTATE, None, None).0
                                                    & BST_PUSHED as isize
                                                    != 0
                                                {
                                                    ODS_SELECTED.0
                                                } else {
                                                    0
                                                };
                                        self.draw_button(&DRAWITEMSTRUCT {
                                            CtlID: id,
                                            hwndItem: child,
                                            hDC: dc,
                                            rcItem: rect,
                                            itemState: windows::Win32::UI::Controls::ODS_FLAGS(
                                                state,
                                            ),
                                            ..Default::default()
                                        });
                                    } else if id == RESULTS {
                                        self.draw_result_rows(child, dc, rect);
                                    } else {
                                        SendMessageW(
                                            child,
                                            WM_PRINT,
                                            Some(WPARAM(dc.0 as usize)),
                                            Some(LPARAM(
                                                (PRF_CLIENT
                                                    | PRF_CHILDREN
                                                    | PRF_ERASEBKGND
                                                    | if target.is_some() {
                                                        PRF_NONCLIENT
                                                    } else {
                                                        0
                                                    })
                                                    as isize,
                                            )),
                                        );
                                        if String::from_utf16_lossy(&name[..count.max(0) as usize])
                                            .eq_ignore_ascii_case("COMBOBOX")
                                        {
                                            crate::combo_control::paint_to_dc(child, dc)?;
                                        } else if matches!(id, VOLUME | AUDIO_POSITION) {
                                            crate::control_vectors::slider_to_dc(child, dc)?;
                                        }
                                    }
                                    Ok(())
                                },
                            );
                            if let Ok(snapshot) = snapshot {
                                cache.insert(id, (key, snapshot));
                            }
                        }
                    }
                    if let Some((_, pixels)) = cache.get(&id) {
                        let painted = if let Some(target) = target {
                            let mut origin = POINT::default();
                            let mut native = RECT::default();
                            ClientToScreen(self.hwnd, &mut origin).as_bool()
                                && GetWindowRect(child, &mut native).is_ok()
                                && pixels
                                    .draw_to_target(
                                        target,
                                        Rect {
                                            x: (native.left - origin.x) as f32 * 96. / self.dpi,
                                            y: (native.top - origin.y) as f32 * 96. / self.dpi,
                                            w: (native.right - native.left) as f32 * 96. / self.dpi,
                                            h: (native.bottom - native.top) as f32 * 96. / self.dpi,
                                        },
                                        self.dpi,
                                    )
                                    .is_ok()
                        } else {
                            pixels.blit(child).is_ok()
                        };
                        if painted {
                            let _ = ValidateRect(Some(child), None);
                            continue;
                        }
                    }
                }
                if target.is_some() {
                    complete = false;
                    continue;
                }
                if button || id == RESULTS {
                    let mut rect = RECT::default();
                    if GetClientRect(child, &mut rect).is_err() {
                        continue;
                    }
                    let dc = GetDC(Some(child));
                    if dc.is_invalid() {
                        continue;
                    }
                    if button {
                        let state = if GetFocus() == child { ODS_FOCUS.0 } else { 0 }
                            | if SendMessageW(child, BM_GETSTATE, None, None).0
                                & BST_PUSHED as isize
                                != 0
                            {
                                ODS_SELECTED.0
                            } else {
                                0
                            };
                        self.draw_button(&DRAWITEMSTRUCT {
                            CtlID: id,
                            hwndItem: child,
                            hDC: dc,
                            rcItem: rect,
                            itemState: windows::Win32::UI::Controls::ODS_FLAGS(state),
                            ..Default::default()
                        });
                    } else {
                        self.draw_result_rows(child, dc, rect);
                    }
                    let _ = ReleaseDC(Some(child), dc);
                } else {
                    let _ = RedrawWindow(
                        Some(child),
                        None,
                        None,
                        RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
                    );
                }
            }
            complete
        }
    }
    /// Paint the same native owner-drawn rows for live input and motion copies.
    /// Selection, scroll, model generation, language and physical size invalidate
    /// those copies; focused results always retain their live native painting.
    unsafe fn draw_result_rows(&self, child: HWND, dc: HDC, rect: RECT) {
        unsafe {
            let selected = SendMessageW(child, LB_GETCURSEL, None, None).0;
            let first = SendMessageW(child, LB_GETTOPINDEX, None, None).0.max(0) as usize;
            for i in first..self.hits.len() {
                let mut row = RECT::default();
                if SendMessageW(
                    child,
                    LB_GETITEMRECT,
                    Some(WPARAM(i)),
                    Some(LPARAM((&mut row as *mut RECT) as isize)),
                )
                .0 == -1
                    || row.top >= rect.bottom
                {
                    break;
                }
                if row.bottom <= 0 {
                    continue;
                }
                self.draw_button(&DRAWITEMSTRUCT {
                    CtlID: RESULTS,
                    itemID: i as u32,
                    hwndItem: child,
                    hDC: dc,
                    rcItem: row,
                    itemState: windows::Win32::UI::Controls::ODS_FLAGS(if selected == i as isize {
                        ODS_SELECTED.0
                    } else {
                        0
                    }),
                    ..Default::default()
                });
            }
        }
    }
    unsafe fn draw_button(&self, item: &DRAWITEMSTRUCT) {
        unsafe {
            if item.CtlID == RESULTS {
                if let Some(hit) = self.hits.get(item.itemID as usize) {
                    let mut palette = self.palette();
                    palette.background = palette.paper;
                    let chapter = self
                        .resources
                        .surahs
                        .get(hit.surah.saturating_sub(1) as usize)
                        .map(|s| {
                            if self.prefs.arabic {
                                s.name_arabic.as_str()
                            } else {
                                s.name_simple.as_str()
                            }
                        })
                        .unwrap_or("");
                    if hit.kind == SearchKind::Surah {
                        if let Some(surah) = self.resources.surahs.get(hit.surah as usize - 1) {
                            if self
                                .chrome
                                .surah_row(
                                    item.hDC,
                                    item.rcItem,
                                    self.dpi,
                                    palette,
                                    hit.surah,
                                    &surah.name_simple,
                                    &surah.name_arabic,
                                    item.itemState.0 & ODS_SELECTED.0 != 0,
                                    item.itemState.0 & ODS_FOCUS.0 != 0,
                                )
                                .is_err()
                            {
                                self.chrome_failures.set(self.chrome_failures.get() + 1);
                            }
                            return;
                        }
                    }
                    let result = self.chrome.button(
                        item.hDC,
                        item.rcItem,
                        self.dpi,
                        palette,
                        crate::chrome::Button {
                            text: &if hit.kind == SearchKind::Surah {
                                format!("{} · {chapter}", hit.surah)
                            } else if matches!(
                                hit.kind,
                                SearchKind::Juz | SearchKind::Hizb | SearchKind::Rub
                            ) {
                                format!(
                                    "{} {} · {} {}",
                                    match hit.kind {
                                        SearchKind::Juz => self.tr("Juz", "الجزء"),
                                        SearchKind::Hizb => self.tr("Hizb", "الحزب"),
                                        _ => self.tr("Rub", "الربع"),
                                    },
                                    hit.key,
                                    self.tr("Page", "صفحة"),
                                    hit.page_number
                                )
                            } else if hit.surah == 0 {
                                format!(
                                    "{} {}",
                                    self.tr("Go to page", "انتقل إلى صفحة"),
                                    hit.page_number
                                )
                            } else {
                                format!(
                                    "{chapter} · {} · {} {}",
                                    hit.key,
                                    self.tr("Page", "صفحة"),
                                    hit.page_number
                                )
                            },
                            detail: &hit.text,
                            quran_detail: hit.kind == SearchKind::Ayah && !hit.text.is_empty(),
                            menu: false,
                            icon: crate::chrome::Icon::None,
                            active: item.itemState.0 & ODS_SELECTED.0 != 0,
                            disabled: false,
                            focus: self.keyboard_focus && item.itemState.0 & ODS_FOCUS.0 != 0,
                            flat: true,
                            rtl: self.prefs.arabic,
                            submenu: false,
                            check: None,
                        },
                    );
                    if result.is_err() {
                        self.chrome_failures.set(self.chrome_failures.get() + 1);
                    }
                }
                return;
            }
            use crate::chrome::{Button, Icon};
            let mut cursor = POINT::default();
            let _ = GetCursorPos(&mut cursor);
            let active = WindowFromPoint(cursor) == item.hwndItem
                || item.itemState.0 & ODS_SELECTED.0 != 0
                || (item.CtlID == self.browse_tab
                    && self.search
                    && self.text(QUERY).trim().is_empty())
                || (item.CtlID == SEARCH && self.search)
                || (item.CtlID == LISTEN
                    && (self.panel == Panel::Listen
                        || (self.player_collapsed && self.side_playback())))
                || (item.CtlID == SETTINGS
                    && self.popup.as_ref().is_some_and(|p| {
                        !p.context
                            && !p
                                .rows
                                .iter()
                                .any(|row| matches!(row.id, WELCOME_ENGLISH | WELCOME_ARABIC))
                    }))
                || item.CtlID == TAFSIR_ARABIC && self.tafsir_arabic
                || item.CtlID == TAFSIR_ENGLISH && !self.tafsir_arabic;
            let icon = match item.CtlID {
                FULLSCREEN => {
                    if self.fullscreen {
                        Icon::None
                    } else {
                        Icon::ExitFullscreen
                    }
                }
                SEARCH => Icon::Search,
                STUDY => Icon::Book,
                LISTEN => Icon::Listen,
                SETTINGS => Icon::Settings,
                QUICK_SETTINGS | STUDY_OPTIONS => Icon::Menu,
                PAUSE | QUICK_PAUSE => {
                    if self.audio.as_ref().is_some_and(Audio::is_paused) {
                        Icon::Play
                    } else {
                        Icon::Pause
                    }
                }
                STOP => Icon::Stop,
                QUICK_STOP => Icon::None,
                SEEK_BACK => {
                    if self.prefs.arabic {
                        Icon::SeekForward
                    } else {
                        Icon::SeekBack
                    }
                }
                SEEK_FORWARD => {
                    if self.prefs.arabic {
                        Icon::SeekBack
                    } else {
                        Icon::SeekForward
                    }
                }
                COLLAPSE_PLAYER => {
                    if self.player_collapsed
                        || (self.displayed_panel() == Panel::None
                            && self.compact_player_frame().is_none())
                    {
                        Icon::Expand
                    } else {
                        Icon::Collapse
                    }
                }
                FOLLOW_AUDIO => Icon::Link,
                PLAY => Icon::Play,
                MINIMIZE => Icon::Minimize,
                MAXIMIZE => {
                    if IsZoomed(self.hwnd).as_bool() {
                        Icon::Restore
                    } else {
                        Icon::Maximize
                    }
                }
                WINDOW_CLOSE | MENU_CLOSE | CLOSE | SEARCH_CLOSE | GUIDE_CLOSE => Icon::Close,
                MENU_BACK | PREV => Icon::Back,
                NEXT => Icon::Forward,
                AYAH_PREV => {
                    if self.prefs.arabic {
                        Icon::Forward
                    } else {
                        Icon::Back
                    }
                }
                AYAH_NEXT => {
                    if self.prefs.arabic {
                        Icon::Back
                    } else {
                        Icon::Forward
                    }
                }
                _ => Icon::None,
            };
            let caption = match item.CtlID {
                SEARCH | LISTEN | SETTINGS if self.fullscreen => String::new(),
                QUICK_STOP => self.tr("End", "إنهاء").into(),
                TEXT_SIZE => "Aa".into(),
                STUDY_OPTIONS => String::new(),
                STUDY_FOLLOW => self.tr("Follow", "مع التلاوة").into(),
                FULLSCREEN if self.fullscreen => self.tr("Exit", "خروج").into(),
                TEXT_SMALL => "A−".into(),
                TEXT_LARGE => "A+".into(),
                FOLLOW_AUDIO => self.tr("Current ayah", "الآية الحالية").into(),
                QUICK_SETTINGS if self.displayed_panel() != Panel::None => String::new(),
                QUICK_SETTINGS if self.displayed_panel() == Panel::None => {
                    self.tr("Settings", "الإعدادات").into()
                }
                SEEK_BACK | SEEK_FORWARD => String::new(),
                PLAY => self.tr("Play", "تشغيل").into(),
                QUICK_PAUSE | PAUSE => {
                    if self.audio.as_ref().is_some_and(Audio::is_paused) {
                        self.tr("Resume", "متابعة").into()
                    } else {
                        self.tr("Pause", "إيقاف مؤقت").into()
                    }
                }
                n if (TAFSIR_ACTION_FIRST
                    ..TAFSIR_ACTION_FIRST + tafsir_library::SOURCES.len() as u32)
                    .contains(&n) =>
                {
                    self.text(n).split(" · ").next().unwrap_or("").to_owned()
                }
                _ => self.text(item.CtlID),
            };
            let row = self
                .popup
                .as_ref()
                .and_then(|p| p.rows.get(item.CtlID.wrapping_sub(MENU_ROW) as usize));
            let caption = row.map(|r| r.label.clone()).unwrap_or(caption);
            let split_arrow = self
                .popup
                .as_ref()
                .is_some_and(|p| p.context && p.stack.is_empty())
                && item.CtlID == MENU_ROW + 2;
            let caption = if split_arrow {
                String::new()
            } else {
                caption.trim_start_matches('✓').trim_start().to_owned()
            };
            let detail = row.map(|r| r.detail.as_str()).unwrap_or("");
            let icon = if split_arrow {
                Icon::Expand
            } else if item.CtlID == MENU_ROW + 1 && self.popup.as_ref().is_some_and(|p| p.context) {
                Icon::Listen
            } else if let Some(row) = row {
                if !row.children.is_empty()
                    || matches!(
                        row.id,
                        LISTEN_THIS | LISTEN_PAGE | LISTEN_SURAH | LISTEN_QURAN
                    )
                {
                    Icon::None
                } else if matches!(row.id, PLAY | LISTEN) {
                    Icon::Listen
                } else if row.id == STUDY {
                    Icon::Book
                } else if row.id == BOOKMARK {
                    Icon::Bookmark
                } else {
                    icon
                }
            } else {
                icon
            };
            let mut palette = self.palette();
            if row.is_some() || matches!(item.CtlID, MENU_BACK | MENU_CLOSE) {
                palette.background = palette.paper;
            }
            let result = self.chrome.button(
                item.hDC,
                item.rcItem,
                self.dpi,
                palette,
                Button {
                    text: &caption,
                    detail,
                    quran_detail: false,
                    menu: row.is_some(),
                    icon,
                    active,
                    disabled: !IsWindowEnabled(item.hwndItem).as_bool(),
                    focus: self.keyboard_focus && item.itemState.0 & ODS_FOCUS.0 != 0,
                    flat: matches!(item.CtlID, STUDY_OPTIONS | STUDY_FOLLOW | TEXT_SIZE)
                        || item.CtlID == FOLLOW_AUDIO
                        || item.CtlID == COLLAPSE_PLAYER
                        || row.is_some()
                        || matches!(
                            item.CtlID,
                            SEARCH
                                | LISTEN
                                | SETTINGS
                                | MINIMIZE
                                | MAXIMIZE
                                | WINDOW_CLOSE
                                | PREV
                                | NEXT
                                | MENU_CLOSE
                                | MENU_BACK
                        ),
                    rtl: if row.is_some_and(|row| row.id == WELCOME_ENGLISH) {
                        false
                    } else if row.is_some_and(|row| row.id == WELCOME_ARABIC) {
                        true
                    } else {
                        self.prefs.arabic && !matches!(item.CtlID, TEXT_SMALL | TEXT_LARGE)
                    },
                    submenu: false,
                    check: if item.CtlID == STUDY_FOLLOW {
                        Some(self.prefs.follow_tafsir)
                    } else {
                        row.and_then(menu_check)
                    },
                },
            );
            // A failed device is recreated on the next draw; keep the native
            // caption legible even if a driver temporarily rejects Direct2D.
            if result.is_err() {
                self.chrome_failures.set(self.chrome_failures.get() + 1);
                let mut text = caption.encode_utf16().collect::<Vec<_>>();
                let mut rect = item.rcItem;
                FillRect(item.hDC, &rect, self.background);
                SetBkMode(item.hDC, TRANSPARENT);
                SetTextColor(item.hDC, COLORREF(self.palette().ink_rgb()));
                DrawTextW(
                    item.hDC,
                    &mut text,
                    &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
    }
    fn reader_area(&self) -> Rect {
        let panel = self.panel_width();
        // The inset follows the same sampled panel pose, including the last
        // frame. A discrete displayed-panel check caused an 80-DIP end jump.
        let inset = if self.fullscreen {
            64. - 56. * (panel / 382.).clamp(0., 1.)
        } else {
            48. - 40. * (panel / 382.).clamp(0., 1.)
        };
        let rail = if self.fullscreen {
            56. * (panel / 382.).clamp(0., 1.)
        } else {
            0.
        };
        Rect {
            x: inset + rail + if self.panel_on_left() { panel } else { 0. },
            y: if self.fullscreen { 6. } else { 38. },
            w: (self.width - 2. * inset - panel - rail).max(130.),
            h: (self.height - if self.fullscreen { 6. } else { 38. } - self.status_height())
                .max(140.),
        }
    }
    fn panel_motion_view(&mut self) -> Option<BookView> {
        if !self.composed_motion_enabled() {
            return None;
        }
        {
            #[cfg(feature = "qa")]
            let frozen_panel = self.panel_pose.is_some();
            #[cfg(not(feature = "qa"))]
            let frozen_panel = false;
            let moving_panel = frozen_panel
                || self
                    .panel_animation
                    .is_some_and(|(start, _, _)| start.elapsed().as_secs_f32() < 0.22);
            let moving_search = self
                .search_animation
                .is_some_and(|start| start.elapsed().as_secs_f32() < 0.18);
            let moving_title = self
                .title_animation
                .is_some_and(|(start, _)| start.elapsed().as_secs_f32() < 0.22);
            let moving_popup = self.popup.as_ref().is_some_and(|popup| {
                popup
                    .motion
                    .is_some_and(|start| start.elapsed().as_secs_f32() < 0.18)
            });
            if !(moving_panel || moving_search || moving_title || moving_popup)
                || self.theme_start.is_some()
                || self.page_turn.is_some()
                || self.page_drag.is_some()
                || self.text_view
            {
                return None;
            }
            let current = crate::app_core::book_geometry(self.reader_area()).bounds;
            let animation = self.panel_animation.take();
            let area = self.reader_area();
            self.panel_animation = animation;
            let landed = crate::app_core::book_geometry(area).bounds;
            if !self.motion_memory_allowed(area, 1) {
                return None;
            }
            if (current.w - landed.w).abs() > 0.02 || (current.h - landed.h).abs() > 0.02 {
                return None;
            }
            Some(BookView {
                page: self.prefs.page,
                area,
                palette: self.palette(),
                selection: self.canvas_selection(),
                recitation: self.canvas_recitation(),
                opening: self.canvas_opening(),
                arabic: self.prefs.arabic,
            })
        }
    }
    fn fold_motion_enabled(&self) -> bool {
        {
            self.composed_motion_enabled()
                && (self.page_turn.is_some_and(|turn| {
                    let (progress, done) = turn.progress();
                    !done && progress < 0.995
                }) || self.page_drag.is_some_and(|drag| drag.progress < 0.995))
                && self.theme_start.is_none()
                && !self.text_view
                && self.motion_memory_allowed(self.reader_area(), 3)
        }
    }
    fn motion_environment(&self) -> (u32, u32, u32, u32, bool) {
        (
            self.width.to_bits(),
            self.height.to_bits(),
            self.dpi.to_bits(),
            self.prefs.theme,
            self.renderer.contextual,
        )
    }
    fn composed_motion_enabled(&self) -> bool {
        !qa_disabled("--qa-vector-motion") && self.motion_failed != Some(self.motion_environment())
    }
    fn motion_result(&mut self, result: Result<bool>) -> bool {
        match result {
            Ok(value) => value,
            Err(error) => {
                self.motion_failed = Some(self.motion_environment());
                #[cfg(feature = "qa")]
                self.motion_errors.push(error.to_string());
                #[cfg(not(feature = "qa"))]
                let _ = error;
                false
            }
        }
    }
    fn motion_memory_allowed(&self, area: Rect, faces: u64) -> bool {
        let b = crate::app_core::book_geometry(area).bounds;
        let d = self.dpi / 96.;
        let w = ((b.x + b.w + 2.) * d).ceil() - ((b.x - 2.) * d).floor();
        let h = ((b.y + b.h + 2.) * d).ceil() - ((b.y - 2.) * d).floor();
        w.is_finite()
            && h.is_finite()
            && (1. ..=16384.).contains(&w)
            && (1. ..=16384.).contains(&h)
            && (w as u64)
                .saturating_mul(h as u64)
                .saturating_mul(8 * faces)
                <= 64 * 1024 * 1024
    }
    fn draw_motion_fold(
        &mut self,
        target: &windows::Win32::Graphics::Direct2D::ID2D1RenderTarget,
        view: BookView,
        old: u32,
        direction: i32,
        progress: f32,
        top: bool,
    ) -> Result<bool> {
        // A book slide and a fold must never own uploaded faces together.
        // Fall back to the authentic vector path if that invariant is broken.
        if self.motion_book.is_some() {
            return Ok(false);
        }
        let bounds = crate::app_core::book_geometry(view.area).bounds;
        let density = self.dpi / 96.;
        let width = (((bounds.x + bounds.w + 2.) * density).ceil()
            - ((bounds.x - 2.) * density).floor()) as u64;
        let height = (((bounds.y + bounds.h + 2.) * density).ceil()
            - ((bounds.y - 2.) * density).floor()) as u64;
        // Three retained rasters, each with CPU and GPU pixels. Native backing
        // and driver metadata are additional and measured separately.
        if width.saturating_mul(height).saturating_mul(24) > 64 * 1024 * 1024 {
            return Ok(false);
        }
        let pixels = unsafe { target.GetPixelSize() };
        let mut key = MotionKey {
            page: view.page,
            theme: self.prefs.theme,
            arabic: view.arabic,
            contextual: self.renderer.contextual,
            selected: None,
            recitation: None,
            opening: None,
            dpi: self.dpi.to_bits(),
            width: pixels.width,
            height: pixels.height,
            clock: self.clock_minute,
            clock_flags: (self.renderer.show_clock, self.renderer.clock_24),
            generation: self.renderer.presentation_generation(),
            geometry: [
                bounds.x.to_bits(),
                bounds.y.to_bits(),
                bounds.w.to_bits(),
                bounds.h.to_bits(),
            ],
        };
        if self
            .motion_fold
            .as_ref()
            .is_none_or(|cache| cache.key != key || cache.old != old)
        {
            self.motion_fold = None;
            #[cfg(feature = "qa")]
            let began = Instant::now();
            let source_view = BookView { page: old, ..view };
            // Move prepared faces out first, then reserve the whole incoming
            // fold. Only unrelated cached pixels are evicted by that budget.
            let source = self
                .motion_cpu
                .take(&self.cpu_raster_key(source_view, false));
            let destination = self.motion_cpu.take(&self.cpu_raster_key(view, false));
            let reverse = self.motion_cpu.take(&self.cpu_raster_key(view, true));
            self.reserve_motion_pixels(view.area, 3);
            let source = match source {
                Some(raster) => raster,
                None => self.prepared_raster(source_view, false)?,
            };
            let destination = match destination {
                Some(raster) => raster,
                None => self.prepared_raster(view, false)?,
            };
            let reverse = match reverse {
                Some(raster) => raster,
                None => self.prepared_raster(view, true)?,
            };
            if source.contextual != destination.contextual
                || destination.contextual != reverse.contextual
            {
                return Ok(false);
            }
            key.contextual = destination.contextual;
            let source_bitmap = source.upload(target)?;
            let destination_bitmap = destination.upload(target)?;
            let reverse_bitmap = reverse.upload(target)?;
            #[cfg(feature = "qa")]
            self.motion_cache_preparations
                .push(began.elapsed().as_secs_f64() * 1000.);
            self.motion_fold = Some(MotionFold {
                key,
                old,
                source,
                destination,
                reverse,
                source_bitmap,
                destination_bitmap,
                reverse_bitmap,
            });
        }
        let cache = self.motion_fold.as_ref().ok_or("Missing fold rasters")?;
        self.renderer.draw_cached_fold(
            target,
            &cache.source_bitmap,
            &cache.destination_bitmap,
            &cache.source,
            &cache.destination,
            Some((&cache.reverse_bitmap, &cache.reverse)),
            view,
            direction,
            progress,
            top,
        )?;
        #[cfg(feature = "qa")]
        {
            self.motion_cache_frames += 1;
        }
        Ok(true)
    }
    fn draw_motion_book(
        &mut self,
        target: &windows::Win32::Graphics::Direct2D::ID2D1RenderTarget,
        final_view: BookView,
        current: BookView,
    ) -> Result<bool> {
        if self.motion_fold.is_some() {
            return Ok(false);
        }
        let pixels = unsafe { target.GetPixelSize() };
        let final_bounds = crate::app_core::book_geometry(final_view.area).bounds;
        let mut key = MotionKey {
            page: final_view.page,
            theme: self.prefs.theme,
            arabic: final_view.arabic,
            contextual: self.renderer.contextual,
            selected: final_view.selection,
            recitation: final_view.recitation,
            opening: final_view.opening,
            dpi: self.dpi.to_bits(),
            width: pixels.width,
            height: pixels.height,
            clock: self.clock_minute,
            clock_flags: (self.renderer.show_clock, self.renderer.clock_24),
            generation: self.renderer.presentation_generation(),
            geometry: [
                final_bounds.x.to_bits(),
                final_bounds.y.to_bits(),
                final_bounds.w.to_bits(),
                final_bounds.h.to_bits(),
            ],
        };
        if self
            .motion_book
            .as_ref()
            .is_none_or(|cache| cache.key != key)
        {
            self.motion_book = None;
            #[cfg(feature = "qa")]
            let began = Instant::now();
            let prepared = self
                .motion_cpu
                .take(&self.cpu_raster_key(final_view, false));
            self.reserve_motion_pixels(final_view.area, 1);
            let raster = match prepared {
                Some(raster) => raster,
                None => self.prepared_raster(final_view, false)?,
            };
            key.contextual = raster.contextual;
            let bitmap = raster.upload(target)?;
            #[cfg(feature = "qa")]
            self.motion_cache_preparations
                .push(began.elapsed().as_secs_f64() * 1000.);
            self.motion_book = Some(MotionBook {
                key,
                raster,
                bitmap,
            });
        }
        let cache = self.motion_book.as_ref().ok_or("Missing prepared book")?;
        let bounds = crate::app_core::book_geometry(current.area).bounds;
        self.renderer.draw_cached_book(
            target,
            &cache.bitmap,
            &cache.raster,
            windows_numerics::Vector2 {
                X: bounds.x - cache.raster.bounds.x,
                Y: bounds.y - cache.raster.bounds.y,
            },
        )?;
        #[cfg(feature = "qa")]
        {
            self.motion_cache_frames += 1;
        }
        Ok(true)
    }
    fn cpu_motion_key(key: &MotionKey, grayscale: bool) -> crate::motion_cache::CpuRasterKey {
        crate::motion_cache::CpuRasterKey {
            page: key.page,
            theme: key.theme,
            arabic: key.arabic,
            contextual: key.contextual,
            selected: key.selected,
            recitation: key.recitation,
            opening: key.opening,
            dpi: key.dpi,
            geometry: key.geometry,
            clock: key.clock,
            show_clock: key.clock_flags.0,
            clock_24: key.clock_flags.1,
            grayscale,
        }
    }
    fn cpu_raster_key(&self, view: BookView, grayscale: bool) -> crate::motion_cache::CpuRasterKey {
        let b = crate::app_core::book_geometry(view.area).bounds;
        crate::motion_cache::CpuRasterKey {
            page: view.page,
            theme: self.prefs.theme,
            arabic: view.arabic,
            contextual: self.renderer.contextual,
            selected: view.selection,
            recitation: view.recitation,
            opening: view.opening,
            dpi: self.dpi.to_bits(),
            geometry: [b.x.to_bits(), b.y.to_bits(), b.w.to_bits(), b.h.to_bits()],
            clock: self.clock_minute,
            show_clock: self.renderer.show_clock,
            clock_24: self.renderer.clock_24,
            grayscale,
        }
    }
    fn prepared_raster(
        &mut self,
        view: BookView,
        grayscale: bool,
    ) -> Result<crate::render::BookRaster> {
        let key = self.cpu_raster_key(view, grayscale);
        if let Some(raster) = self.motion_cpu.take(&key) {
            return Ok(raster);
        }
        if grayscale {
            self.renderer
                .raster_book_with_aa(&mut self.resources, view, self.dpi, true)
        } else {
            self.renderer
                .raster_book(&mut self.resources, view, self.dpi)
        }
    }
    fn raster_pixel_estimate(&self, area: Rect) -> u64 {
        let b = crate::app_core::book_geometry(area).bounds;
        let density = self.dpi / 96.;
        let width = ((b.x + b.w + 2.) * density).ceil() - ((b.x - 2.) * density).floor();
        let height = ((b.y + b.h + 2.) * density).ceil() - ((b.y - 2.) * density).floor();
        (width.max(0.) as u64)
            .saturating_mul(height.max(0.) as u64)
            .saturating_mul(4)
    }
    fn motion_pixel_usage(&self) -> (u64, u64) {
        let book = self
            .motion_book
            .as_ref()
            .map_or(0, |cache| cache.raster.pixel_bytes());
        let fold = self.motion_fold.as_ref().map_or(0, |cache| {
            cache
                .source
                .pixel_bytes()
                .saturating_add(cache.destination.pixel_bytes())
                .saturating_add(cache.reverse.pixel_bytes())
        });
        let active = book.saturating_add(fold);
        (active, active)
    }
    fn reserve_motion_pixels(&mut self, area: Rect, faces: u64) {
        let incoming = self.raster_pixel_estimate(area).saturating_mul(faces);
        let (cpu, uploaded) = self.motion_pixel_usage();
        self.motion_cpu
            .set_budget(crate::motion_policy::spare_cache_budget(
                cpu.saturating_add(incoming),
                uploaded.saturating_add(incoming),
            ));
    }
    fn prepare_motion_idle(&mut self) -> Result<bool> {
        {
            if !self.composed_motion_enabled()
                || self.text_view
                || self.search
                || self.popup.is_some()
                || self.theme_start.is_some()
                || self.page_turn.is_some()
                || self.page_drag.is_some()
                || self.panel_animation.is_some()
                || self.title_animation.is_some()
            {
                return Ok(false);
            }
            let view = BookView {
                page: self.prefs.page,
                area: self.reader_area(),
                palette: self.palette(),
                selection: None,
                recitation: None,
                opening: None,
                arabic: self.prefs.arabic,
            };
            if !self.motion_memory_allowed(view.area, 3) {
                self.motion_cpu.clear();
                return Ok(false);
            }
            let key = self.cpu_raster_key(view, false);
            let (cpu, uploaded) = self.motion_pixel_usage();
            let budget = crate::motion_policy::spare_cache_budget(cpu, uploaded);
            self.motion_cpu.set_budget(budget);
            self.motion_cpu.retain_environment(&key);
            let plan = crate::motion_policy::admit_rasters(
                view.page,
                self.turn_destination(-1),
                self.turn_destination(1),
                self.last_motion_direction,
                self.raster_pixel_estimate(view.area),
                budget,
            );
            if plan.is_empty() {
                return Ok(false);
            }
            let mut keys = Vec::with_capacity(plan.len());
            for candidate in plan.as_slice() {
                keys.push(self.cpu_raster_key(
                    BookView {
                        page: candidate.page,
                        ..view
                    },
                    candidate.grayscale,
                ));
            }
            self.motion_cpu.retain_keys(&keys);
            for (candidate, key) in plan.as_slice().iter().zip(keys) {
                if !self.motion_cpu.contains(&key) {
                    let raster = self.renderer.raster_book_with_aa(
                        &mut self.resources,
                        BookView {
                            page: candidate.page,
                            ..view
                        },
                        self.dpi,
                        candidate.grayscale,
                    )?;
                    return Ok(self.motion_cpu.insert(key, raster));
                }
            }
            Ok(false)
        }
    }
    fn canvas_opening(&self) -> Option<u32> {
        (!self.search
            && self.popup.is_none()
            && (self.playback.following || self.panel == Panel::Study))
            .then_some(self.opening_highlight)
            .flatten()
    }
    fn canvas_recitation(&self) -> Option<(u32, u32)> {
        if self.panel == Panel::Study
            && !self.search
            && self.popup.is_none()
            && self.audio.as_ref().is_some_and(Audio::has_session)
        {
            self.playback.current
        } else {
            None
        }
    }
    fn canvas_selection(&self) -> Option<(u32, u32)> {
        if self.panel != Panel::Study && self.canvas_opening().is_some() {
            return None;
        }
        if let Some((_, key)) = self.popup.as_ref().and_then(|popup| popup.context_target) {
            return Some(key);
        }
        if !self.search && self.popup.is_none() && self.panel != Panel::Study {
            if let Some((start, key)) = self.search_reveal {
                if start.elapsed() < std::time::Duration::from_secs(5) {
                    return Some(key);
                }
            }
        }
        if !self.search && self.popup.is_none() && self.panel != Panel::Study {
            if let Some(key) = self.playback.current {
                return Some(key);
            }
        }
        crate::app_core::canvas_selection(
            self.panel == Panel::Study,
            self.search || self.popup.is_some(),
            self.hovered,
            self.selected,
        )
    }
    fn search_list_y(&self) -> f32 {
        unsafe {
            if self.text(QUERY).trim().is_empty() {
                crate::search_layout::SearchLayout::new(self.height).browse_results
            } else {
                crate::search_layout::SearchLayout::new(self.height).query_frame + 60.
            }
        }
    }
    fn search_row_height(&self) -> f32 {
        if self.hits.iter().any(|h| h.kind == SearchKind::Ayah) {
            56.
        } else {
            42.
        }
    }
    fn search_result_height(&self) -> f32 {
        (self.hits.len().min(5) as f32 * self.search_row_height())
            .max(36.)
            .min((self.height - self.search_list_y() - 32.).max(0.))
    }
    fn turn_destination(&self, direction: i32) -> u32 {
        crate::app_core::destination(self.prefs.page, direction, self.single_page())
    }
    fn single_page(&self) -> bool {
        crate::app_core::book_geometry(self.reader_area()).single
    }
    fn maintenance_ready(&self) -> bool {
        self.last_input.elapsed().as_millis() >= 160
            && self.page_turn.is_none()
            && self.page_drag.is_none()
            && self.drag_start.is_none()
            && self.theme_start.is_none()
            && self.panel_animation.is_none()
            && self.title_animation.is_none()
            && self.search_animation.is_none()
            && self
                .popup
                .as_ref()
                .is_none_or(|popup| popup.motion.is_none())
    }
    fn schedule_prewarm(&mut self) {
        if self.page_turn.is_some()
            || self.page_drag.is_some()
            || self.panel_animation.is_some()
            || self.title_animation.is_some()
            || self.search_animation.is_some()
        {
            return;
        }
        let single = self.single_page();
        let key = (self.prefs.page, self.renderer.contextual, single);
        if self.prewarm_key == Some(key) {
            // Geometry/DPI can become admissible again without changing the
            // page key. One idle probe also recovers an emptied pixel cache;
            // it terminates immediately when the admitted set is complete.
            unsafe {
                SetTimer(Some(self.hwnd), 7, 40, None);
            }
            return;
        }
        self.prewarm_key = Some(key);
        self.prewarm_pages.clear();
        for offset in 1..=if single { 4 } else { 2 } {
            for direction in [1, -1] {
                let page =
                    crate::app_core::destination(self.prefs.page, direction * offset, single);
                let pages = if single {
                    [Some(page), None]
                } else {
                    let (right, left) = data::spread(page);
                    [Some(right), left]
                };
                for page in pages.into_iter().flatten() {
                    if !self.prewarm_pages.contains(&page) {
                        self.prewarm_pages.push_back(page);
                    }
                }
            }
        }
        unsafe {
            SetTimer(Some(self.hwnd), 7, 40, None);
        }
    }
    fn update_hover(&mut self, point: Option<(f32, f32)>) {
        let next = if self.popup.is_none()
            && !self.search
            && !self.text_view
            && self.page_turn.is_none()
            && self.page_drag.is_none()
            && self.drag_start.is_none()
        {
            point.and_then(|(x, y)| self.renderer.hit(x, y))
        } else {
            None
        };
        if self.hovered != next {
            self.hovered = next;
            self.invalidate_reader();
        }
    }
    fn next_queued_turn(&mut self) -> Option<crate::app_core::TurnRequest> {
        while let Some(request) = self.turn_queue.next() {
            if request.source == crate::app_core::TurnSource::Keyboard && request.repeated {
                let key = if request.direction > 0 {
                    VK_LEFT
                } else {
                    VK_RIGHT
                };
                if unsafe { GetAsyncKeyState(key.0 as i32) } >= 0 {
                    continue;
                }
            }
            self.keyboard_turn = request.source == crate::app_core::TurnSource::Keyboard;
            return Some(request);
        }
        None
    }
    fn request_turn(&mut self, direction: i32, source: crate::app_core::TurnSource) -> Result<()> {
        if self.panel == Panel::Study {
            return self.navigate(direction);
        }
        if self.page_turn.is_some() {
            self.turn_queue.enqueue(crate::app_core::TurnRequest {
                direction,
                repeated: self.repeating_key,
                source,
            });
            return Ok(());
        }
        self.keyboard_turn = source == crate::app_core::TurnSource::Keyboard;
        self.navigate(direction)
    }
    fn move_page_drag(&mut self, x: f32, y: f32) {
        self.hovered = None;
        let Some((sx, sy)) = self.drag_start else {
            return;
        };
        let dx = x - sx;
        let dy = y - sy;
        if self.page_drag.is_none() && dx.abs() < 8. {
            return;
        }
        if self.page_drag.is_none() && dy.abs() > dx.abs() {
            self.drag_start = None;
            unsafe {
                ReleaseCapture().ok();
            }
            return;
        }
        let direction = if dx > 0. { 1 } else { -1 };
        let new = self.turn_destination(direction);
        if new == self.prefs.page {
            return;
        }
        let bounds = crate::page_turn::leaf_bounds(self.reader_area(), direction);
        let progress = (dx.abs() / bounds.w).min(0.92);
        self.page_drag = Some(crate::page_turn::Drag {
            old: self.prefs.page,
            new,
            direction,
            progress,
            top: sy < bounds.y + bounds.h / 2.,
        });
        self.invalidate_reader();
    }
    fn finish_page_drag(&mut self) -> Result<bool> {
        let Some(d) = self.page_drag.take() else {
            return Ok(false);
        };
        self.playback.suspend();
        let commit = d.progress >= 0.18;
        let mut enabled = BOOL::from(true);
        unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some((&mut enabled as *mut BOOL).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .ok();
        }
        if commit {
            self.navigate(d.direction)?;
        }
        self.page_turn = enabled.as_bool().then(|| crate::page_turn::Turn {
            first_frame: false,
            start: Instant::now(),
            old: d.old,
            new: d.new,
            direction: d.direction,
            from: d.progress,
            to: if commit { 1. } else { 0. },
            top: d.top,
        });
        unsafe {
            if self.page_turn.is_some() {
                SetTimer(Some(self.hwnd), 4, 10, None);
            }
        }
        self.invalidate();
        Ok(true)
    }
    fn manual_navigation(&mut self) {
        if self.panel == Panel::Study && self.prefs.follow_tafsir {
            self.prefs.follow_tafsir = false;
            self.dirty = true;
            unsafe {
                self.refresh_study_follow();
            }
        }
        self.search_reveal = None;
        self.playback.suspend();
        self.page_turn = None;
        self.page_drag = None;
        self.drag_start = None;
        self.turn_queue.clear();
        self.hovered = None;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), 4);
            if GetCapture() == self.hwnd {
                let _ = ReleaseCapture();
            }
        }
    }
    fn follow_recitation(&mut self, key: Option<(u32, u32)>) -> Result<()> {
        let opening = self.audio.as_ref().and_then(Audio::opening_surah);
        let changed = self.playback.current != key || self.opening_highlight != opening;
        let study = self.panel == Panel::Study;
        let following = if study {
            self.prefs.follow_tafsir
        } else {
            self.playback.following
        };
        if self.opening_highlight != opening && following && self.popup.is_none() {
            if let Some(surah) = opening {
                if let Some(hit) = self
                    .resources
                    .index()?
                    .iter()
                    .find(|h| h.surah == surah && h.ayah == 1)
                {
                    self.prefs.page = hit.page_number;
                    self.dirty = true;
                }
            }
        }
        self.opening_highlight = opening;
        let advance = self.playback.observe(key);
        let update = if study {
            self.prefs.follow_tafsir && key.is_some() && self.selected != key
        } else {
            advance
        };
        if update && !self.popup.as_ref().is_some_and(|popup| popup.context) {
            self.selected = key;
            if let Some((s, a)) = key {
                if let Some(hit) = self
                    .resources
                    .index()?
                    .iter()
                    .find(|h| h.surah == s && h.ayah == a)
                {
                    self.prefs.page = hit.page_number;
                    self.dirty = true;
                    unsafe {
                        self.observe_reading(&local_day().0, Instant::now())?;
                    }
                }
                if study {
                    unsafe {
                        self.study()?;
                    }
                }
            }
        }
        if changed {
            self.invalidate_reader();
            // The ayah heading and the two-role legend live in the parent,
            // outside the Quran damage rectangle. This does not relayout or
            // reload the explanation during independent reading.
            if study {
                self.invalidate();
            }
        }
        Ok(())
    }
    fn navigate(&mut self, delta: i32) -> Result<()> {
        self.playback.suspend();
        self.hovered = None;
        unsafe {
            if self.panel == Panel::Study {
                return self.ayah(delta);
            }
            self.last_motion_direction = delta.signum();
            self.page_drag = None;
            self.drag_start = None;
            if GetCapture() == self.hwnd {
                ReleaseCapture().ok();
            }
            let old_page = self.prefs.page;
            self.prefs.page = self.turn_destination(delta);
            if old_page != self.prefs.page {
                let mut enabled = BOOL::from(true);
                let _ = SystemParametersInfoW(
                    SPI_GETCLIENTAREAANIMATION,
                    0,
                    Some((&mut enabled as *mut BOOL).cast()),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                );
                self.page_turn = enabled.as_bool().then(|| crate::page_turn::Turn {
                    first_frame: true,
                    start: Instant::now(),
                    old: old_page,
                    new: self.prefs.page,
                    direction: delta,
                    from: 0.,
                    to: 1.,
                    top: false,
                });
                if self.page_turn.is_some() {
                    SetTimer(Some(self.hwnd), 4, 10, None);
                }
            }
            self.selected = None;
            self.dirty = true;
            self.observe_reading(&local_day().0, Instant::now())?;
            self.invalidate();
            Ok(())
        }
    }
    unsafe fn ayah(&mut self, delta: i32) -> Result<()> {
        self.manual_navigation();
        unsafe {
            let key = self.selected.unwrap_or((1, 1));
            let index = self.resources.index()?;
            let current = index
                .iter()
                .position(|hit| (hit.surah, hit.ayah) == key)
                .ok_or("Missing ayah")?;
            let hit = index[(current as i32 + delta).clamp(0, 6235) as usize].clone();
            self.selected = Some((hit.surah, hit.ayah));
            self.prefs.page = hit.page_number;
            self.hovered = None;
            self.dirty = true;
            self.observe_reading(&local_day().0, Instant::now())?;
            self.study()?;
            self.invalidate();
            Ok(())
        }
    }
    unsafe fn study(&mut self) -> Result<()> {
        unsafe {
            let sources = crate::app_core::study_source_ids(
                self.prefs.arabic,
                self.installed_tafsirs.iter().copied(),
            );
            if sources != self.study_sources || !sources.contains(&self.source) {
                self.refresh_sources();
            }
            let (s, a) = self.selected.unwrap_or((1, 1));
            if self.tafsir_library.is_none() {
                if let Err(error) = self.ensure_tafsir_library() {
                    self.notice(format!(
                        "{}: {error}",
                        self.tr("Optional books unavailable", "تعذر فتح الكتب الإضافية")
                    ));
                }
            }
            let source = if self.source < 3 {
                ["mukhtasar", "saadi", "sahih"][self.source]
            } else {
                tafsir_library::SOURCES
                    .get(self.source - 3)
                    .ok_or("Invalid tafsir source")?
                    .id
            };
            let text = self.resources.study(source, s, a)?;
            self.verse_text_end = 0;
            let text = if text.is_empty() {
                self.tr(
                    "No separate explanation is available for this ayah in this source.",
                    "لا يتوفر تفسير مستقل لهذه الآية في هذا المصدر.",
                )
                .to_owned()
            } else {
                text
            };
            // Keep the old explanation on screen until the replacement text,
            // font, direction and scroll position are ready to paint together.
            let _redraw = crate::reading_control::RedrawBatch::begin(self.controls[&STUDY]);
            let prefix = format!("{text}\r\n\r\n");
            self.study_attribution_start =
                prefix.encode_utf16().filter(|unit| *unit != 13).count() as u32;
            self.set_text(
                STUDY_ATTRIBUTION,
                if self.source < 3 {
                    self.tr(
                        "Source: Quranic Universal Library",
                        "المصدر: المكتبة القرآنية الشاملة",
                    )
                } else {
                    self.tr(
                        "Source: Quran.com · Arabic tafsir",
                        "المصدر: Quran.com · تفسير عربي",
                    )
                },
            );
            let complete = format!("{prefix}{}", self.text(STUDY_ATTRIBUTION));
            self.set_text(STUDY, &complete);
            self.refresh_study_font();
            let footer = self.controls[&STUDY_ATTRIBUTION];
            let footer_style = GetWindowLongPtrW(footer, GWL_STYLE);
            SetWindowLongPtrW(
                footer,
                GWL_STYLE,
                (footer_style & !3) | if self.prefs.arabic { 2 } else { 0 },
            );
            SetWindowLongPtrW(
                footer,
                GWL_EXSTYLE,
                if self.prefs.arabic {
                    WS_EX_RTLREADING.0 as isize
                } else {
                    0
                },
            );
            SetWindowLongPtrW(
                self.controls[&STUDY],
                GWL_EXSTYLE,
                if self.source == 2 {
                    0
                } else {
                    WS_EX_RTLREADING.0 as isize
                },
            );
            let format = PARAFORMAT2 {
                Base: PARAFORMAT {
                    cbSize: std::mem::size_of::<PARAFORMAT2>() as u32,
                    dwMask: PFM_LINESPACING | PFM_ALIGNMENT | PFM_SPACEAFTER | PFM_RTLPARA,
                    Anonymous: PARAFORMAT_0 {
                        wEffects: u16::from(self.source != 2),
                    },
                    wAlignment: if self.source == 2 {
                        PFA_LEFT
                    } else {
                        PFA_RIGHT
                    },
                    ..Default::default()
                },
                dyLineSpacing: 28,
                dySpaceAfter: 120,
                bLineSpacingRule: 5,
                ..Default::default()
            };
            SendMessageW(
                self.controls[&STUDY],
                EM_SETSEL,
                Some(WPARAM(0)),
                Some(LPARAM(-1)),
            );
            SendMessageW(
                self.controls[&STUDY],
                EM_SETPARAFORMAT,
                None,
                Some(LPARAM((&format as *const PARAFORMAT2) as isize)),
            );
            SendMessageW(
                self.controls[&STUDY],
                EM_SETSEL,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            );
            SendMessageW(
                self.controls[&STUDY],
                EM_SCROLL,
                Some(WPARAM(SB_TOP.0 as usize)),
                None,
            );
            self.format_study_verse();
            self.format_study_attribution(self.palette());
            Ok(())
        }
    }
    unsafe fn format_study_verse(&self) {
        unsafe {
            if self.verse_text_end == 0 {
                return;
            }
            let study = self.controls[&STUDY];
            let mut previous = CHARRANGE::default();
            SendMessageW(
                study,
                EM_EXGETSEL,
                None,
                Some(LPARAM((&mut previous as *mut CHARRANGE) as isize)),
            );
            SendMessageW(
                study,
                EM_SETSEL,
                Some(WPARAM(0)),
                Some(LPARAM(self.verse_text_end as isize)),
            );
            let mut format = CHARFORMAT2W::default();
            format.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
            format.Base.dwMask = CFM_FACE | CFM_SIZE;
            format.Base.yHeight = (self.study_size + 2) * 15;
            let family = if self.quran_study_font.is_some() {
                self.renderer.font_family.to_string()
            } else {
                "Segoe UI".to_owned()
            };
            for (target, unit) in format.Base.szFaceName.iter_mut().zip(family.encode_utf16()) {
                *target = unit;
            }
            SendMessageW(
                study,
                EM_SETCHARFORMAT,
                Some(WPARAM(SCF_SELECTION as usize)),
                Some(LPARAM((&format as *const CHARFORMAT2W) as isize)),
            );
            let paragraph = PARAFORMAT {
                cbSize: std::mem::size_of::<PARAFORMAT>() as u32,
                dwMask: PFM_ALIGNMENT,
                wAlignment: PFA_RIGHT,
                ..Default::default()
            };
            SendMessageW(
                study,
                EM_SETPARAFORMAT,
                None,
                Some(LPARAM((&paragraph as *const PARAFORMAT) as isize)),
            );
            SendMessageW(
                study,
                EM_EXSETSEL,
                None,
                Some(LPARAM((&previous as *const CHARRANGE) as isize)),
            );
            crate::reading_control::hide_caret(study);
        }
    }
    unsafe fn format_study_attribution(&self, palette: Palette) {
        if self.study_attribution_start == 0 {
            return;
        }
        unsafe {
            let study = self.controls[&STUDY];
            let mut previous = CHARRANGE::default();
            SendMessageW(
                study,
                EM_EXGETSEL,
                None,
                Some(LPARAM((&mut previous as *mut CHARRANGE) as isize)),
            );
            SendMessageW(
                study,
                EM_SETSEL,
                Some(WPARAM(self.study_attribution_start as usize)),
                Some(LPARAM(-1)),
            );
            let paragraph = PARAFORMAT {
                cbSize: std::mem::size_of::<PARAFORMAT>() as u32,
                dwMask: PFM_ALIGNMENT | PFM_RTLPARA,
                Anonymous: PARAFORMAT_0 {
                    wEffects: u16::from(self.prefs.arabic),
                },
                wAlignment: if self.prefs.arabic {
                    PFA_RIGHT
                } else {
                    PFA_LEFT
                },
                ..Default::default()
            };
            let mut current = PARAFORMAT {
                cbSize: std::mem::size_of::<PARAFORMAT>() as u32,
                ..Default::default()
            };
            SendMessageW(
                study,
                EM_GETPARAFORMAT,
                None,
                Some(LPARAM((&mut current as *mut PARAFORMAT) as isize)),
            );
            if current.wAlignment != paragraph.wAlignment
                || current.Anonymous.wEffects & 1 != paragraph.Anonymous.wEffects
            {
                SendMessageW(
                    study,
                    EM_SETPARAFORMAT,
                    None,
                    Some(LPARAM((&paragraph as *const PARAFORMAT) as isize)),
                );
            }
            let color = palette.muted;
            let rule_color = |border: f32, background: f32| {
                ((border * 0.65 + background * 0.35) * 255.)
                    .round()
                    .clamp(0., 255.) as u32
            };
            crate::reading_control::set_attribution_rule(
                study,
                self.study_attribution_start,
                COLORREF(
                    rule_color(palette.border.r, palette.background.r)
                        | (rule_color(palette.border.g, palette.background.g) << 8)
                        | (rule_color(palette.border.b, palette.background.b) << 16),
                ),
            );
            let format = CHARFORMAT2W {
                Base: CHARFORMATW {
                    cbSize: std::mem::size_of::<CHARFORMAT2W>() as u32,
                    dwMask: CFM_SIZE | CFM_COLOR,
                    yHeight: 180,
                    crTextColor: COLORREF(
                        ((color.r * 255.) as u32)
                            | (((color.g * 255.) as u32) << 8)
                            | (((color.b * 255.) as u32) << 16),
                    ),
                    ..Default::default()
                },
                ..Default::default()
            };
            SendMessageW(
                study,
                EM_SETCHARFORMAT,
                Some(WPARAM(SCF_SELECTION as usize)),
                Some(LPARAM((&format as *const CHARFORMAT2W) as isize)),
            );
            SendMessageW(
                study,
                EM_EXSETSEL,
                None,
                Some(LPARAM((&previous as *const CHARRANGE) as isize)),
            );
            crate::reading_control::hide_caret(study);
        }
    }
    unsafe fn refresh_study_font(&mut self) {
        unsafe {
            if self.panel == Panel::Storage {
                self.format_storage_details();
                return;
            }
            let dpi_key = self.dpi.to_bits();
            if self.attribution_font_dpi != dpi_key {
                let font = CreateFontW(
                    -(12. * self.dpi / 96.).round() as i32,
                    0,
                    0,
                    0,
                    400,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    0,
                    w!("Segoe UI"),
                );
                if !font.is_invalid() {
                    SendMessageW(
                        self.controls[&STUDY_ATTRIBUTION],
                        WM_SETFONT,
                        Some(WPARAM(font.0 as usize)),
                        Some(LPARAM(0)),
                    );
                    if !self.attribution_font.is_invalid() {
                        let _ = DeleteObject(self.attribution_font.into());
                    }
                    self.attribution_font = font;
                    self.attribution_font_dpi = dpi_key;
                }
            }
            let key = (self.study_size, dpi_key);
            if self.study_font_key == Some(key) {
                return;
            }
            let font = CreateFontW(
                -((self.study_size as f32 * self.dpi / 96.) as i32),
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            if font.is_invalid() {
                return;
            }
            SendMessageW(
                self.controls[&STUDY],
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(0)),
            );
            if !self.study_font.is_invalid() {
                let _ = DeleteObject(self.study_font.into());
            }
            self.study_font = font;
            self.study_font_key = Some(key);
            // RichEdit's WM_SETFONT resets automatic ink; reapply the explicit
            // theme color after every explanation font/size update.
            self.format_study_color(self.palette());
            self.format_study_verse();
            SendMessageW(
                self.controls[&TEXT_VIEW],
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(0)),
            );
        }
    }
    unsafe fn find(&mut self) -> Result<()> {
        unsafe {
            let query = self.text(QUERY);
            if let Some(page) = data::jump_page(&query) {
                self.manual_navigation();
                self.prefs.page = page;
                self.search = false;
                self.hits.clear();
                self.selected = None;
                self.dirty = true;
                self.observe_reading(&local_day().0, Instant::now())?;
                self.resize();
                let _ = SetFocus(Some(self.hwnd));
                return Ok(());
            }
            self.update_search()?;
            if self.hits.len() == 1 {
                SendMessageW(self.controls[&RESULTS], LB_SETCURSEL, Some(WPARAM(0)), None);
                self.choose_hit()?;
            } else if !self.hits.is_empty() {
                SendMessageW(self.controls[&RESULTS], LB_SETCURSEL, Some(WPARAM(0)), None);
                focus_window(self.controls[&RESULTS])?;
            }
            Ok(())
        }
    }
    unsafe fn update_search(&mut self) -> Result<()> {
        unsafe {
            self.update_search_model()?;
            self.resize();
            SendMessageW(
                self.controls[&RESULTS],
                LB_SETTOPINDEX,
                Some(WPARAM(0)),
                None,
            );
            Ok(())
        }
    }
    unsafe fn update_search_model(&mut self) -> Result<()> {
        unsafe {
            let query = self.text(QUERY);
            self.hits = if !query.trim().is_empty() {
                self.resources.browse_search(&query)?
            } else {
                let filter = self.text(BROWSE_QUERY);
                match self.browse_tab {
                    BROWSE_SURAH => self.resources.browse_surahs(&filter)?,
                    BROWSE_PAGE => {
                        if let Some(page_number) = data::jump_page(&filter) {
                            self.resources.browse_search(&page_number.to_string())?
                        } else {
                            vec![]
                        }
                    }
                    _ => self.resources.browse_sections(
                        [SearchKind::Juz, SearchKind::Hizb, SearchKind::Rub]
                            [self.select(BROWSE_KIND).min(2)],
                        &filter,
                    )?,
                }
            };
            let list = self.controls[&RESULTS];
            self.results_generation = self.results_generation.wrapping_add(1);
            SendMessageW(list, LB_RESETCONTENT, None, None);
            for hit in &self.hits {
                let text = HSTRING::from(if hit.kind == SearchKind::Surah {
                    format!(
                        "{} · {} · {}",
                        hit.surah,
                        self.resources.surahs[hit.surah as usize - 1].name_simple,
                        self.resources.surahs[hit.surah as usize - 1].name_arabic
                    )
                } else {
                    format!("{} · {}", hit.key, hit.text)
                });
                SendMessageW(
                    list,
                    LB_ADDSTRING,
                    None,
                    Some(LPARAM(text.as_ptr() as isize)),
                );
            }
            // Filtering presents choices; selection follows an explicit click
            // or keyboard action, rather than implying the first hit is correct.
            // List resizing can scroll its selected first row above the
            // viewport. A newly filtered result set always starts at row zero;
            // later animation steps leave user scrolling alone.
            SendMessageW(list, LB_SETTOPINDEX, Some(WPARAM(0)), None);
            Ok(())
        }
    }
    unsafe fn choose_hit(&mut self) -> Result<()> {
        unsafe {
            let index = SendMessageW(self.controls[&RESULTS], LB_GETCURSEL, None, None).0;
            if index < 0 {
                return Ok(());
            }
            if let Some(hit) = self.hits.get(index as usize).cloned() {
                self.manual_navigation();
                self.selected = (hit.kind == SearchKind::Ayah).then_some((hit.surah, hit.ayah));
                self.search_reveal = self.selected.map(|key| (Instant::now(), key));
                if self.search_reveal.is_some() {
                    SetTimer(Some(self.hwnd), 9, 5000, None);
                }
                self.prefs.page = hit.page_number;
                self.hovered = None;
                self.dirty = true;
                self.search = false;
                self.hits.clear();
                self.observe_reading(&local_day().0, Instant::now())?;
                self.resize();
                focus_window(self.hwnd)?;
            }
            Ok(())
        }
    }
    unsafe fn popup_focus(&self, tab: bool, backward: bool) -> Result<()> {
        unsafe {
            let Some(p) = &self.popup else {
                return Ok(());
            };
            let mut ids: Vec<_> = (MENU_ROW..MENU_ROW + p.rows.len() as u32)
                .map(|id| {
                    if p.rows[(id - MENU_ROW) as usize].id == THEME_SELECTOR {
                        THEME_SELECTOR
                    } else {
                        id
                    }
                })
                .filter(|id| {
                    let handle = self.controls[id];
                    IsWindowVisible(handle).as_bool() && IsWindowEnabled(handle).as_bool()
                })
                .collect();
            if tab {
                for id in [MENU_BACK, MENU_CLOSE] {
                    let handle = self.controls[&id];
                    if IsWindowVisible(handle).as_bool() && IsWindowEnabled(handle).as_bool() {
                        ids.push(id);
                    }
                }
            }
            if ids.is_empty() {
                return Ok(());
            }
            let focus = GetFocus();
            let n = ids.len();
            // Mouse-opened Options keeps parent focus. Its first keyboard
            // action enters at the requested edge instead of skipping row zero.
            let next = ids
                .iter()
                .position(|id| self.controls[id] == focus)
                .map(|i| {
                    if backward {
                        (i + n - 1) % n
                    } else {
                        (i + 1) % n
                    }
                })
                .unwrap_or(if backward { n - 1 } else { 0 });
            focus_window(self.controls[&ids[next]])?;
            Ok(())
        }
    }
    unsafe fn native_combo_escape(&self, focus: HWND) -> bool {
        unsafe {
            for id in [
                RECITER,
                RECORDING_MODE,
                CHAPTER,
                SPEED,
                REPEAT,
                SOURCE,
                BROWSE_KIND,
                THEME_SELECTOR,
            ] {
                let handle = self.controls[&id];
                if !IsWindowVisible(handle).as_bool()
                    || SendMessageW(handle, CB_GETDROPPEDSTATE, None, None).0 == 0
                {
                    continue;
                }
                let mut info = COMBOBOXINFO {
                    cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                    ..Default::default()
                };
                let list_focused = GetComboBoxInfo(handle, &mut info).is_ok()
                    && (focus == info.hwndList || IsChild(info.hwndList, focus).as_bool());
                if focus == handle || IsChild(handle, focus).as_bool() || list_focused {
                    // Let the native combo cancel its selection and emit its
                    // normal notifications before a later Escape closes the
                    // form. CB_SHOWDROPDOWN alone does not provide that contract.
                    SendMessageW(
                        handle,
                        WM_KEYDOWN,
                        Some(WPARAM(VK_ESCAPE.0 as usize)),
                        Some(LPARAM(1)),
                    );
                    return true;
                }
            }
            false
        }
    }
    unsafe fn key(&mut self, key: u16) -> Result<bool> {
        unsafe {
            self.last_input = Instant::now();
            let focus = GetFocus();
            if key == 27 && self.text_view && self.popup.is_none() && !self.search {
                self.set_accessible_view(false)?;
                return Ok(true);
            }
            if key == 27 && self.native_combo_escape(focus) {
                return Ok(true);
            }
            if self.panel == Panel::Study
                && focus == self.controls[&SOURCE]
                && matches!(key, 37 | 39)
                && SendMessageW(focus, CB_GETDROPPEDSTATE, None, None).0 == 0
            {
                self.request_turn(
                    if key == 37 { 1 } else { -1 },
                    crate::app_core::TurnSource::Keyboard,
                )?;
                focus_window(self.controls[&STUDY])?;
                return Ok(true);
            }
            if key == 27 && self.guide.is_some() {
                self.close_guide()?;
                return Ok(true);
            }
            if self.guide_key(key, focus)? {
                return Ok(true);
            }
            if key == 27 {
                self.turn_queue.clear();
            }
            if key == 27 && self.drag_start.is_some() {
                self.drag_start = None;
                self.page_drag = None;
                ReleaseCapture().ok();
                self.invalidate();
                return Ok(true);
            }
            if key == 70 && GetKeyState(VK_CONTROL.0 as i32) < 0 {
                self.popup = None;
                if !self.search {
                    self.command(SEARCH, 0)?;
                }
                focus_window(self.controls[&QUERY])?;
                return Ok(true);
            }
            if (focus == self.controls[&QUERY] || focus == self.controls[&BROWSE_QUERY])
                && key == 13
                && QUERY_COMPOSING.with(Cell::get)
            {
                return Ok(false);
            }
            if focus == self.controls[&STUDY]
                && matches!(key, 37..=40)
                && (GetKeyState(VK_CONTROL.0 as i32) < 0 || GetKeyState(VK_SHIFT.0 as i32) < 0)
            {
                return Ok(false);
            }
            if self.popup.is_some()
                && focus == self.controls[&THEME_SELECTOR]
                && matches!(key, 13 | 32 | 38 | 40 | 115)
            {
                // Let the actual native dropdown own its arrows, Enter, Space
                // and F4. Tab still moves through the containing settings pane.
                return Ok(false);
            }
            if let Some(p) = &self.popup {
                match key {
                    9 => {
                        self.popup_focus(true, GetKeyState(VK_SHIFT.0 as i32) < 0)?;
                        return Ok(true);
                    }
                    27 => {
                        self.popup_command(if p.stack.is_empty() {
                            MENU_CLOSE
                        } else {
                            MENU_BACK
                        })?;
                        return Ok(true);
                    }
                    38 | 40 => {
                        self.popup_focus(false, key == 38)?;
                        return Ok(true);
                    }
                    13 => {
                        if let Some(id) = (MENU_ROW..MENU_ROW + 32)
                            .chain([MENU_BACK, MENU_CLOSE])
                            .find(|id| {
                                let handle = self.controls[id];
                                handle == focus
                                    && IsWindowVisible(handle).as_bool()
                                    && IsWindowEnabled(handle).as_bool()
                            })
                        {
                            self.popup_command(id)?;
                            return Ok(true);
                        }
                    }
                    37 | 39 => return Ok(true),
                    _ => {}
                }
            }
            let input = [
                QUERY,
                BROWSE_QUERY,
                BROWSE_KIND,
                BROWSE_SURAH,
                BROWSE_PAGE,
                BROWSE_SECTIONS,
                FIRST,
                LAST,
                RECITER,
                RECORDING_MODE,
                CHAPTER,
                SPEED,
                VOLUME,
                AUDIO_POSITION,
                SOURCE,
                STUDY,
                RESULTS,
                REPEAT,
                TEXT_VIEW,
            ]
            .iter()
            .any(|id| {
                let handle = self.controls[id];
                IsWindowVisible(handle).as_bool()
                    && (handle == focus || IsChild(handle, focus).as_bool())
            });
            if key == 66 && GetKeyState(VK_CONTROL.0 as i32) < 0 {
                self.command(
                    if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                        BOOKMARKS
                    } else {
                        BOOKMARK
                    },
                    0,
                )?;
                return Ok(true);
            }
            if self.panel == Panel::Listen && !input && matches!(key, 33 | 34) {
                self.panel_scroll += if key == 33 { -200. } else { 200. };
                self.resize();
                return Ok(true);
            }
            if key == 84
                && self.prefs.search_key_t
                && !input
                && self.popup.is_none()
                && GetKeyState(VK_CONTROL.0 as i32) >= 0
                && GetKeyState(VK_MENU.0 as i32) >= 0
            {
                if !self.search {
                    self.command(SEARCH, 0)?;
                }
                focus_window(self.controls[&QUERY])?;
                return Ok(true);
            }
            match key {
                38 | 40
                    if (focus == self.controls[&QUERY]
                        || focus == self.controls[&BROWSE_QUERY])
                        && !self.hits.is_empty() =>
                {
                    SendMessageW(
                        self.controls[&RESULTS],
                        LB_SETCURSEL,
                        Some(WPARAM(if key == 38 { self.hits.len() - 1 } else { 0 })),
                        None,
                    );
                    focus_window(self.controls[&RESULTS])?;
                }
                13 if focus == self.controls[&QUERY] || focus == self.controls[&BROWSE_QUERY] => {
                    self.find()?
                }
                13 if focus == self.controls[&RESULTS] => self.choose_hit()?,
                13 => {
                    let Some(id) = [
                        SEARCH,
                        LISTEN,
                        DAILY,
                        SETTINGS,
                        FULLSCREEN,
                        PREV,
                        NEXT,
                        GO,
                        CLOSE,
                        SEARCH_CLOSE,
                        BROWSE_SURAH,
                        BROWSE_PAGE,
                        BROWSE_SECTIONS,
                        AYAH_PREV,
                        AYAH_NEXT,
                        PLAY,
                        PAUSE,
                        STOP,
                        QUICK_PAUSE,
                        QUICK_STOP,
                        FOLLOW_AUDIO,
                        QUICK_SETTINGS,
                        COLLAPSE_PLAYER,
                        DOWNLOAD,
                        STORAGE,
                    ]
                    .into_iter()
                    .chain(
                        TAFSIR_ACTION_FIRST
                            ..TAFSIR_ACTION_FIRST + tafsir_library::SOURCES.len() as u32,
                    )
                    .find(|id| self.controls[id] == focus) else {
                        return Ok(false);
                    };
                    self.command(id, 0)?;
                }
                37 if !input || (self.panel == Panel::Study && focus == self.controls[&STUDY]) => {
                    self.request_turn(1, crate::app_core::TurnSource::Keyboard)?
                }
                39 if !input || (self.panel == Panel::Study && focus == self.controls[&STUDY]) => {
                    self.request_turn(-1, crate::app_core::TurnSource::Keyboard)?
                }
                32 if !input && self.audio.as_ref().is_some_and(Audio::has_session) => {
                    self.command(PAUSE, 0)?
                }
                93 if !input => self.keyboard_context()?,
                121 if !input && GetKeyState(VK_SHIFT.0 as i32) < 0 => self.keyboard_context()?,
                122 => self.toggle_fullscreen()?,
                27 => {
                    if self.search {
                        self.search = false;
                        self.resize();
                        focus_window(self.hwnd)?;
                    } else if self.panel != Panel::None {
                        self.set_panel(Panel::None);
                        self.resize();
                        focus_window(self.hwnd)?;
                    } else if self.fullscreen {
                        self.toggle_fullscreen()?;
                    }
                }
                _ => return Ok(false),
            }
            Ok(true)
        }
    }
    unsafe fn options(&mut self) -> Result<()> {
        unsafe {
            if self.popup.is_some() {
                self.popup = None;
                self.resize();
                return Ok(());
            }
            let menu = CreatePopupMenu()?;
            AppendMenuW(
                menu,
                MF_STRING,
                LANG as usize,
                &HSTRING::from(self.tr("العربية", "English")),
            )?;
            AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
            for (n, en, ar) in [
                (0, "Graphite", "رمادي داكن"),
                (1, "Light", "فاتح"),
                (2, "Sepia", "بني فاتح"),
                (3, "Pure black", "أسود"),
            ] {
                AppendMenuW(
                    menu,
                    MF_STRING
                        | if self.prefs.theme == n {
                            MF_CHECKED
                        } else {
                            MF_UNCHECKED
                        },
                    (THEME + n) as usize,
                    &HSTRING::from(self.tr(en, ar)),
                )?;
            }
            if self.panel == Panel::Study {
                AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
                AppendMenuW(
                    menu,
                    MF_STRING,
                    TEXT_SMALL as usize,
                    &HSTRING::from(self.tr("Smaller explanation text", "تصغير نص التفسير")),
                )?;
                AppendMenuW(
                    menu,
                    MF_STRING,
                    TEXT_LARGE as usize,
                    &HSTRING::from(self.tr("Larger explanation text", "تكبير نص التفسير")),
                )?;
            }
            AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
            AppendMenuW(
                menu,
                MF_STRING
                    | if self.prefs.contextual {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    },
                CONTEXTUAL as usize,
                &HSTRING::from(self.tr(
                    "Contextual letter elongation (optional)",
                    "إطالة الحروف السياقية (اختياري)",
                )),
            )?;
            AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
            for (id, en, ar, checked) in [
                (
                    BOOKMARK,
                    "Save / remove bookmark  Ctrl+B",
                    "إضافة / حذف علامة  Ctrl+B",
                    false,
                ),
                (
                    BOOKMARKS,
                    "Bookmarks…  Ctrl+Shift+B",
                    "العلامات المحفوظة…  Ctrl+Shift+B",
                    false,
                ),
                (
                    ACCESSIBLE,
                    "Accessible text view…",
                    "عرض نصي لقارئات الشاشة…",
                    self.text_view,
                ),
                (
                    CLOCK,
                    "Show page clock",
                    "إظهار الساعة",
                    self.prefs.show_clock,
                ),
                (
                    CLOCK_24,
                    "24-hour clock",
                    "الساعة بنظام ٢٤ ساعة",
                    self.prefs.clock_24,
                ),
            ] {
                AppendMenuW(
                    menu,
                    MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED },
                    id as usize,
                    &HSTRING::from(self.tr(en, ar)),
                )?;
            }
            let reading = CreatePopupMenu()?;
            for (n, en, ar, mode, target) in [
                (0, "5 pages a day", "٥ صفحات يوميًا", GoalMode::Pages, 5),
                (1, "10 pages a day", "١٠ صفحات يوميًا", GoalMode::Pages, 10),
                (2, "5 minutes a day", "٥ دقائق يوميًا", GoalMode::Minutes, 5),
                (
                    3,
                    "10 minutes a day",
                    "١٠ دقائق يوميًا",
                    GoalMode::Minutes,
                    10,
                ),
            ] {
                let checked = self.reader.goal.mode == mode && self.reader.goal.target == target;
                AppendMenuW(
                    reading,
                    MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED },
                    (GOAL + n) as usize,
                    &HSTRING::from(self.tr(en, ar)),
                )?;
            }
            AppendMenuW(
                reading,
                MF_STRING,
                DAILY as usize,
                &HSTRING::from(self.tr(
                    if self.daily.is_some() {
                        "Finish reading"
                    } else {
                        "Start reading"
                    },
                    if self.daily.is_some() {
                        "إنهاء القراءة"
                    } else {
                        "بدء القراءة"
                    },
                )),
            )?;
            AppendMenuW(reading, MF_SEPARATOR, 0, None)?;
            for (id, en, ar, checked) in [
                (
                    TIMER,
                    "Show reading stopwatch",
                    "إظهار مؤقت القراءة",
                    self.reader.reading.show_timer,
                ),
                (
                    REMINDER,
                    "Remind me at 7 pm while the app is open",
                    "تذكيري الساعة ٧ مساءً أثناء فتح التطبيق",
                    self.reader.reading.reminder_enabled,
                ),
            ] {
                AppendMenuW(
                    reading,
                    MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED },
                    id as usize,
                    &HSTRING::from(self.tr(en, ar)),
                )?;
            }
            AppendMenuW(
                menu,
                MF_POPUP,
                reading.0 as usize,
                &HSTRING::from(self.tr("Daily reading", "الورد اليومي")),
            )?;
            AppendMenuW(
                menu,
                MF_STRING
                    | if self.prefs.search_key_t {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    },
                SEARCH_KEY_T as usize,
                &HSTRING::from(self.tr("Optional T key for Search", "مفتاح T الاختياري للبحث")),
            )?;
            AppendMenuW(
                menu,
                MF_STRING,
                HELP as usize,
                &HSTRING::from(self.tr("How to use the reader…", "كيفية استخدام التطبيق…")),
            )?;
            let listening = CreatePopupMenu()?;
            AppendMenuW(
                listening,
                MF_STRING
                    | if self.prefs.remember_speed {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    },
                REMEMBER_SPEED as usize,
                &HSTRING::from(self.tr("Remember playback speed", "تذكّر سرعة التشغيل")),
            )?;
            let limit_menu = CreatePopupMenu()?;
            for (index, gib) in [1, 4, 16, 0].into_iter().enumerate() {
                let label = if gib == 0 {
                    self.tr("No download limit", "بلا حد للتنزيل").to_owned()
                } else {
                    format!("{gib} GiB")
                };
                AppendMenuW(
                    limit_menu,
                    MF_STRING
                        | if self.prefs.offline_limit_gib == gib {
                            MF_CHECKED
                        } else {
                            MF_UNCHECKED
                        },
                    (OFFLINE_LIMIT_FIRST + index as u32) as usize,
                    &HSTRING::from(label),
                )?;
            }
            AppendMenuW(
                listening,
                MF_POPUP,
                limit_menu.0 as usize,
                &HSTRING::from(self.tr("Offline download limit", "حد التنزيل دون اتصال")),
            )?;
            AppendMenuW(
                listening,
                MF_STRING
                    | if self.prefs.continuous_audio {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    },
                CONTINUOUS_AUDIO as usize,
                &HSTRING::from(self.tr(
                    "Continuous recording · Yasser / Alafasy",
                    "تسجيل متصل · الدوسري / العفاسي",
                )),
            )?;
            AppendMenuW(
                listening,
                MF_STRING,
                LISTEN as usize,
                &HSTRING::from(self.tr("Open listening controls", "فتح أدوات الاستماع")),
            )?;
            AppendMenuW(
                menu,
                MF_POPUP,
                listening.0 as usize,
                &HSTRING::from(self.tr("Listening", "الاستماع")),
            )?;
            AppendMenuW(
                menu,
                MF_STRING,
                FULLSCREEN as usize,
                &HSTRING::from(self.tr("Full screen · F11", "ملء الشاشة · F11")),
            )?;
            let mut button = RECT::default();
            GetWindowRect(self.controls[&SETTINGS], &mut button)?;
            let point = POINT {
                x: if self.prefs.arabic {
                    button.right
                } else {
                    button.left
                },
                y: button.bottom + (4. * self.dpi / 96.) as i32,
            };
            let rows = menu_rows(menu);
            DestroyMenu(menu)?;
            let mut appearance = Vec::new();
            let mut accessibility = Vec::new();
            let mut bookmarks = Vec::new();
            let mut interface = Vec::new();
            let mut primary = Vec::new();
            for row in rows {
                if (THEME..=THEME + 3).contains(&row.id) {
                    continue;
                }
                if matches!(row.id, LANG | OTHER_STUDY_LANGUAGES | SEARCH_KEY_T) {
                    interface.push(row);
                } else if row.id == ACCESSIBLE {
                    accessibility.push(row);
                } else if (THEME..=THEME + 3).contains(&row.id)
                    || matches!(
                        row.id,
                        CONTEXTUAL | CLOCK | CLOCK_24 | ACCESSIBLE | TEXT_SMALL | TEXT_LARGE
                    )
                {
                    appearance.push(row);
                } else if matches!(row.id, BOOKMARK | BOOKMARKS) {
                    bookmarks.push(row);
                } else {
                    primary.push(row);
                }
            }
            primary.push(MenuRow {
                id: 0,
                label: self.tr("Accessibility", "إمكانية الوصول").into(),
                detail: self
                    .tr(
                        "For Narrator and other screen readers",
                        "لراوي Windows وقارئات الشاشة الأخرى",
                    )
                    .into(),
                children: accessibility,
            });
            appearance.insert(
                0,
                MenuRow {
                    id: THEME_SELECTOR,
                    label: self.tr("Theme", "المظهر").into(),
                    // This row reserves room for a label and an accessible native
                    // dropdown, instead of rendering four checked menu commands.
                    detail: self
                        .tr("Choose an application theme", "اختر مظهر التطبيق")
                        .into(),
                    children: Vec::new(),
                },
            );
            primary.insert(
                0,
                MenuRow {
                    id: 0,
                    label: self.tr("Language and shortcuts", "اللغة والاختصارات").into(),
                    detail: self
                        .tr(
                            "Menus, settings and search keys",
                            "القوائم والإعدادات ومفاتيح البحث",
                        )
                        .into(),
                    children: interface,
                },
            );
            primary.insert(
                1,
                MenuRow {
                    id: 0,
                    label: self.tr("Appearance", "المظهر").into(),
                    detail: self
                        .tr(
                            "Theme, text and reading display",
                            "الألوان والخط وعرض القراءة",
                        )
                        .into(),
                    children: appearance,
                },
            );
            primary.insert(
                2,
                MenuRow {
                    id: 0,
                    label: self.tr("Bookmarks", "العلامات المحفوظة").into(),
                    detail: String::new(),
                    children: bookmarks,
                },
            );
            let mut client = point;
            let _ = ScreenToClient(self.hwnd, &mut client);
            self.show_popup_model(
                self.tr("Options", "الخيارات").into(),
                primary,
                client.x as f32 * 96. / self.dpi,
                client.y as f32 * 96. / self.dpi,
                false,
            )?;
            if let Some(p) = &mut self.popup {
                p.docked = true;
                p.rect.y = 52.;
                p.rect.w = 360f32.min((self.width - 24.).max(250.));
                p.motion = client_animation_enabled().then(Instant::now);
                if p.motion.is_some() {
                    SetTimer(Some(self.hwnd), 6, 10, None);
                }
            }
            self.resize();
            focus_window(self.hwnd)?;
            Ok(())
        }
    }
    unsafe fn context(&mut self, x: f32, y: f32) -> Result<()> {
        unsafe {
            if self.text_view {
                return Ok(());
            }
            let Some(key) = self.renderer.hit(x, y) else {
                return Ok(());
            };
            self.context_at(key, x, y, true)
        }
    }
    unsafe fn keyboard_context(&mut self) -> Result<()> {
        unsafe {
            let key = self.selected.or_else(|| {
                self.renderer
                    .visible
                    .iter()
                    .find_map(|line| line.runs.first().map(|run| (run.surah, run.ayah)))
            });
            let Some(key) = key else {
                return Ok(());
            };
            #[cfg(feature = "qa")]
            {
                self.qa_context = Some(key);
            }
            let point = self
                .renderer
                .visible
                .iter()
                .find(|line| line.runs.iter().any(|run| (run.surah, run.ayah) == key))
                .map(|line| {
                    (
                        line.rect.x + line.rect.w / 2.,
                        line.rect.y + line.rect.h / 2.,
                    )
                })
                .unwrap_or((self.width / 2., self.height / 2.));
            self.context_at(key, point.0, point.1, false)
        }
    }
    unsafe fn context_at(
        &mut self,
        key: (u32, u32),
        x: f32,
        y: f32,
        _toggle_study: bool,
    ) -> Result<()> {
        unsafe {
            self.hovered = None;
            self.selected = Some(key);
            self.invalidate();
            let menu = CreatePopupMenu()?;
            let listen = CreatePopupMenu()?;
            AppendMenuW(
                menu,
                MF_STRING,
                STUDY as usize,
                &HSTRING::from(self.tr("Read tafsir", "قراءة التفسير")),
            )?;
            for (id, en, ar) in [
                (
                    LISTEN_SURAH,
                    "Listen from here to the end of this surah",
                    "استمع من هنا حتى نهاية السورة",
                ),
                (LISTEN_THIS, "This ayah only", "هذه الآية فقط"),
                (LISTEN_PAGE, "To end of this page", "حتى نهاية هذه الصفحة"),
                (LISTEN_SURAH, "To end of this surah", "حتى نهاية هذه السورة"),
                (LISTEN_QURAN, "To end of Quran", "حتى نهاية القرآن"),
                (LISTEN, "Choose a listening range…", "اختيار نطاق للاستماع…"),
            ] {
                AppendMenuW(
                    listen,
                    MF_STRING,
                    id as usize,
                    &HSTRING::from(self.tr(en, ar)),
                )?;
            }
            AppendMenuW(
                menu,
                MF_POPUP,
                listen.0 as usize,
                &HSTRING::from(self.tr("Listen", "استماع")),
            )?;
            AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
            AppendMenuW(
                menu,
                MF_STRING,
                BOOKMARK as usize,
                &HSTRING::from(self.tr("Save / remove bookmark", "إضافة / حذف علامة")),
            )?;
            let mut point = POINT {
                x: (x * self.dpi / 96.) as i32,
                y: (y * self.dpi / 96.) as i32,
            };
            let _ = ClientToScreen(self.hwnd, &mut point);
            let mut rows = menu_rows(menu);
            DestroyMenu(menu)?;
            let listen = rows.remove(1);
            let mut listening = listen.children;
            let mut primary = listening.remove(0);
            primary.label = self.tr("Listen from here", "استمع من هنا").into();
            primary.detail = self
                .tr("Continue to the end of this surah", "حتى نهاية هذه السورة")
                .into();
            rows[0].label = self.tr("Tafsir", "التفسير").into();
            rows[0].detail = self
                .tr("Read the explanation of this ayah", "اقرأ تفسير هذه الآية")
                .into();
            rows.insert(1, primary);
            rows.insert(
                2,
                MenuRow {
                    id: 0,
                    label: self
                        .tr("More listening options", "خيارات استماع أخرى")
                        .into(),
                    detail: String::new(),
                    children: listening,
                },
            );
            if let Some(row) = rows.iter_mut().find(|r| r.id == BOOKMARK) {
                row.label = if self
                    .reader
                    .bookmarks
                    .iter()
                    .any(|b| b.page == self.prefs.page)
                {
                    self.tr("Remove bookmark", "حذف العلامة")
                } else {
                    self.tr("Save bookmark", "حفظ علامة")
                }
                .into();
                row.detail = "Ctrl+B".into();
            }
            self.show_popup(
                format!(
                    "{} · {} {}",
                    self.resources
                        .surahs
                        .get(key.0.saturating_sub(1) as usize)
                        .map(|s| if self.prefs.arabic {
                            s.name_arabic.as_str()
                        } else {
                            s.name_simple.as_str()
                        })
                        .unwrap_or(""),
                    self.tr("Ayah", "الآية"),
                    key.1
                ),
                rows,
                x,
                y,
                true,
            )?;
            Ok(())
        }
    }
    fn context_page(&mut self, key: (u32, u32), x: f32, y: f32) -> Result<u32> {
        let geometry = crate::app_core::book_geometry(self.reader_area());
        if geometry.bounds.contains(x, y) {
            let (right, left) = data::spread(self.prefs.page);
            let page = if geometry.single {
                self.prefs.page
            } else if x < geometry.bounds.x + geometry.page_width {
                left.unwrap_or(right)
            } else {
                right
            };
            if self
                .resources
                .page(page)?
                .lines
                .iter()
                .any(|line| line.words.iter().any(|word| (word.surah, word.ayah) == key))
            {
                return Ok(page);
            }
        }
        self.resources
            .index()?
            .iter()
            .find(|hit| (hit.surah, hit.ayah) == key)
            .map(|hit| hit.page_number)
            .ok_or_else(|| "The selected authentic ayah is unavailable".into())
    }
    fn context_listening_plan(
        &mut self,
        page: u32,
        key: (u32, u32),
        scope: crate::app_core::ListeningScope,
    ) -> Result<crate::app_core::ListeningPlan> {
        let page_end = if scope == crate::app_core::ListeningScope::Page {
            let source = self.resources.page(page)?;
            if !source
                .lines
                .iter()
                .flat_map(|line| line.words.iter())
                .any(|word| (word.surah, word.ayah) == key)
            {
                return Err("The selected ayah is not on this physical page".into());
            }
            source
                .lines
                .iter()
                .flat_map(|line| line.words.iter())
                .next_back()
                .map(|word| (word.surah, word.ayah))
        } else {
            None
        };
        let counts: Vec<_> = self
            .resources
            .surahs
            .iter()
            .map(|surah| surah.verses_count)
            .collect();
        crate::app_core::ListeningPlan::new(&counts, key, page_end, scope).map_err(Into::into)
    }
    unsafe fn show_popup(
        &mut self,
        title: String,
        rows: Vec<MenuRow>,
        x: f32,
        y: f32,
        context: bool,
    ) -> Result<()> {
        unsafe {
            self.show_popup_model(title, rows, x, y, context)?;
            self.resize();
            focus_window(self.controls[&MENU_ROW])?;
            Ok(())
        }
    }
    fn show_popup_model(
        &mut self,
        title: String,
        rows: Vec<MenuRow>,
        x: f32,
        y: f32,
        context: bool,
    ) -> Result<()> {
        let context_target = if context {
            if let Some(key) = self.selected {
                Some((self.context_page(key, x, y)?, key))
            } else {
                None
            }
        } else {
            None
        };
        self.popup = Some(Popup {
            title,
            rows,
            stack: vec![],
            rect: Rect {
                x: (if self.prefs.arabic { x - 344. } else { x })
                    .clamp(12., (self.width - 356.).max(12.)),
                y: y.max(42.),
                w: 344.,
                h: 0.,
            },
            context,
            context_target,
            listening_expanded: false,
            docked: false,
            rtl: self.prefs.arabic,
            motion: None,
            frame_pose: None,
        });
        Ok(())
    }
    unsafe fn popup_command(&mut self, id: u32) -> Result<()> {
        unsafe {
            if id == MENU_CLOSE {
                self.popup = None;
                self.resize();
                focus_window(self.hwnd)?;
                return Ok(());
            }
            if id == MENU_BACK {
                if let Some(p) = &mut self.popup {
                    if let Some((title, rows)) = p.stack.pop() {
                        p.title = title;
                        p.rows = rows;
                    }
                }
                self.resize();
                focus_window(self.controls[&MENU_ROW])?;
                return Ok(());
            }
            let Some(p) = &mut self.popup else {
                return Ok(());
            };
            let Some(row) = p.rows.get(id.wrapping_sub(MENU_ROW) as usize).cloned() else {
                return Ok(());
            };
            if p.context && p.stack.is_empty() && id == MENU_ROW + 2 && !row.children.is_empty() {
                if p.listening_expanded {
                    p.rows.drain(3..3 + row.children.len());
                } else {
                    p.rows.splice(3..3, row.children.clone());
                }
                p.listening_expanded = !p.listening_expanded;
                p.motion = None;
                p.frame_pose = None;
                self.resize();
                focus_window(self.controls[&id])?;
                return Ok(());
            }
            if !p.context && (THEME..=THEME + 3).contains(&row.id) {
                for item in &mut p.rows {
                    if (THEME..=THEME + 3).contains(&item.id) {
                        let label = item.label.trim_start_matches("✓ ").to_owned();
                        item.label = if item.id == row.id {
                            format!("✓ {label}")
                        } else {
                            label
                        };
                    }
                }
                self.theme(row.id - THEME);
                self.resize();
                focus_window(self.controls[&id])?;
                return Ok(());
            }
            if p.docked
                && row.children.is_empty()
                && matches!(
                    row.id,
                    CONTEXTUAL
                        | CLOCK
                        | CLOCK_24
                        | TEXT_SMALL
                        | TEXT_LARGE
                        | OTHER_STUDY_LANGUAGES
                        | SEARCH_KEY_T
                        | CONTINUOUS_AUDIO
                        | REMEMBER_SPEED
                        | TIMER
                        | REMINDER
                )
            {
                let mut retained = self.popup.take().unwrap();
                self.command(row.id, 0)?;
                let checked = match row.id {
                    CONTEXTUAL => self.prefs.contextual,
                    CLOCK => self.prefs.show_clock,
                    CLOCK_24 => self.prefs.clock_24,
                    REMEMBER_SPEED => self.prefs.remember_speed,
                    OTHER_STUDY_LANGUAGES => self.prefs.other_study_languages,
                    SEARCH_KEY_T => self.prefs.search_key_t,
                    CONTINUOUS_AUDIO => self.prefs.continuous_audio,
                    TIMER => self.reader.reading.show_timer,
                    REMINDER => self.reader.reading.reminder_enabled,
                    _ => false,
                };
                if let Some(item) = retained.rows.get_mut(id.wrapping_sub(MENU_ROW) as usize) {
                    let label = item.label.trim_start_matches("✓ ").to_owned();
                    item.label = if checked {
                        format!("✓ {label}")
                    } else {
                        label
                    };
                }
                self.popup = Some(retained);
                self.resize();
                focus_window(self.controls[&id])?;
                return Ok(());
            }
            if !row.children.is_empty() {
                p.stack.push((
                    std::mem::replace(&mut p.title, row.label),
                    std::mem::replace(&mut p.rows, row.children),
                ));
                self.resize();
                let first = if self.popup.as_ref().is_some_and(|popup| {
                    popup
                        .rows
                        .first()
                        .is_some_and(|row| row.id == THEME_SELECTOR)
                }) {
                    THEME_SELECTOR
                } else {
                    MENU_ROW
                };
                focus_window(if self.keyboard_focus {
                    self.controls[&first]
                } else {
                    self.hwnd
                })?;
            } else {
                let context = p.context;
                let context_target = p.context_target;
                self.popup = None;
                if let Some((page, key)) = context_target {
                    self.prefs.page = page;
                    self.selected = Some(key);
                    self.dirty = true;
                }
                self.resize();
                focus_window(self.hwnd)?;
                if context {
                    if row.id == LISTEN {
                        self.open_listen_form(false);
                        return Ok(());
                    }
                    let scope = match row.id {
                        LISTEN_THIS => Some(crate::app_core::ListeningScope::ThisAyah),
                        LISTEN_PAGE => Some(crate::app_core::ListeningScope::Page),
                        LISTEN_SURAH => Some(crate::app_core::ListeningScope::Surah),
                        LISTEN_QURAN => Some(crate::app_core::ListeningScope::Quran),
                        _ => None,
                    };
                    if let (Some(scope), Some((page, key))) = (scope, context_target) {
                        return self.start_context_audio(page, key, scope);
                    }
                }
                self.command(row.id, 0)?;
            }
            Ok(())
        }
    }
    unsafe fn open_listen(&mut self) {
        unsafe {
            self.open_listen_form(true);
        }
    }
    unsafe fn clear_unfocused_reciter_selection(&self) {
        unsafe {
            let reciter = self.controls[&RECITER];
            let mut info = COMBOBOXINFO {
                cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                ..Default::default()
            };
            if GetComboBoxInfo(reciter, &mut info).is_err()
                || GetFocus() == info.hwndItem
                || GetFocus() == reciter
                || SendMessageW(reciter, CB_GETDROPPEDSTATE, None, None).0 != 0
            {
                return;
            }
            let mut start = 0u32;
            let mut end = 0u32;
            SendMessageW(
                info.hwndItem,
                windows::Win32::UI::Controls::EM_GETSEL,
                Some(WPARAM((&mut start as *mut u32) as usize)),
                Some(LPARAM((&mut end as *mut u32) as isize)),
            );
            if start != end {
                SendMessageW(info.hwndItem, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(0)));
                let _ = InvalidateRect(Some(info.hwndItem), None, false);
                self.motion_controls.borrow_mut().remove(&RECITER);
            }
        }
    }
    unsafe fn open_listen_form(&mut self, retain_active: bool) {
        unsafe {
            self.panel_scroll = 0.;
            self.search = false;
            let active = self
                .active_range
                .filter(|_| retain_active && self.audio.as_ref().is_some_and(Audio::has_session));
            let (surah, ayah) = active
                .map(|(range, _)| (range.surah, range.first))
                .unwrap_or_else(|| self.selected.unwrap_or((1, 1)));
            SendMessageW(
                self.controls[&CHAPTER],
                CB_SETCURSEL,
                Some(WPARAM(surah as usize - 1)),
                None,
            );
            self.set_text(FIRST, &ayah.to_string());
            self.set_text(
                LAST,
                &active
                    .map_or(
                        self.resources.surahs[surah as usize - 1].verses_count,
                        |(range, _)| range.last,
                    )
                    .to_string(),
            );
            // Focus stays on the parent; resize clears the native edit selection
            // after showing/moving it and before motion snapshots can capture it.
            SetFocus(Some(self.hwnd)).ok();
            self.motion_controls.borrow_mut().remove(&RECITER);
            self.set_panel(Panel::Listen);
            self.resize();
        }
    }
    #[cfg_attr(feature = "qa", track_caller)]
    unsafe fn ayah_input(&self, id: u32) -> Result<u32> {
        unsafe {
            #[cfg(feature = "qa")]
            if data::normalize(&self.text(id)).parse::<u32>().is_err() {
                if let Some(out) = qa_argument("--qa-ui") {
                    let _ = std::fs::write(
                        out.join(format!("invalid-input-{id}.txt")),
                        format!(
                            "value={:?} caller={}",
                            self.text(id),
                            std::panic::Location::caller()
                        ),
                    );
                }
            }
            data::normalize(&self.text(id)).parse::<u32>().map_err(|_| {
                self.tr("Enter a valid ayah number", "أدخل رقم آية صالحًا")
                    .into()
            })
        }
    }
    unsafe fn chosen_reciter(&self) -> Result<usize> {
        unsafe {
            let selected = SendMessageW(self.controls[&RECITER], CB_GETCURSEL, None, None).0;
            let reciter = if selected >= 0 {
                Some(selected as usize)
            } else {
                crate::audio::match_reciter(&self.text(RECITER))
            };
            reciter
                .filter(|index| *index < RECITERS.len())
                .ok_or_else(|| "Choose a matching reciter from the list".into())
        }
    }
    fn play_plan_chunk(
        &mut self,
        chunk: crate::app_core::ListeningChunk,
        reciter: usize,
        continuous: bool,
        rolling: bool,
    ) -> Result<()> {
        unsafe {
            self.set_text(AUDIO_TIME, self.tr("Loading audio…", "جارٍ تحميل الصوت…"));
        }
        if self.audio.is_none() {
            self.audio = Some(Audio::with_cache(self.cache()?)?);
        }
        let range = ListeningRange {
            reciter,
            surah: chunk.surah,
            first: chunk.first,
            last: chunk.last,
            count: chunk.count,
        };
        let audio = self.audio.as_mut().unwrap();
        if continuous {
            if rolling {
                audio.play_timed_scoped(range, self.prefs.speed, self.prefs.volume)?;
            } else {
                audio.play_timed(range, self.prefs.speed, self.prefs.volume)?;
            }
        } else {
            audio.play_range(range, self.prefs.speed, self.prefs.volume)?;
        }
        audio.repeat(false)?;
        self.active_range = Some((range, continuous));
        Ok(())
    }
    fn listening_scope_label(&self, scope: crate::app_core::ListeningScope, page: u32) -> String {
        match scope {
            crate::app_core::ListeningScope::ThisAyah => {
                self.tr("This ayah only", "هذه الآية فقط").into()
            }
            crate::app_core::ListeningScope::Page => {
                format!("{} {page}", self.tr("To end of page", "حتى نهاية الصفحة"))
            }
            crate::app_core::ListeningScope::Surah => self
                .tr("To end of this surah", "حتى نهاية هذه السورة")
                .into(),
            crate::app_core::ListeningScope::Quran => {
                self.tr("To end of Quran", "حتى نهاية القرآن").into()
            }
        }
    }
    unsafe fn start_context_audio(
        &mut self,
        page: u32,
        key: (u32, u32),
        scope: crate::app_core::ListeningScope,
    ) -> Result<()> {
        unsafe {
            let plan = self.context_listening_plan(page, key, scope)?;
            self.speed_selection_pending = false;
            self.prepare_recitation_rate();
            let first = *plan
                .chunks()
                .first()
                .ok_or("The listening request is empty")?;
            let reciter = self.chosen_reciter()?;
            let continuous =
                crate::timed_audio::Recording::enabled(self.prefs.continuous_audio, reciter);
            let rolling = continuous && plan.chunks().len() > 1;
            let last = *plan
                .chunks()
                .last()
                .ok_or("The listening request is empty")?;
            let label = self.listening_scope_label(scope, page);
            self.listening_session = None;
            self.active_range = None;
            self.play_plan_chunk(first, reciter, continuous, rolling)?;
            self.reciter = reciter;
            self.listening_session = Some(ScopedListening {
                session: crate::app_core::ListeningSession::new(plan, self.prefs.repeat_plays),
                reciter,
                continuous,
                rolling,
                paused: false,
                start: (first.surah, first.first),
                end: (last.surah, last.last),
                scope,
                page,
                label,
                label_arabic: self.prefs.arabic,
            });
            self.manual_navigation();
            self.opening_highlight = None;
            self.playback.begin();
            // Direct listening needs a small transport, not the full range
            // editor. Listen can reopen that editor with the active range.
            self.set_panel(Panel::None);
            self.search = false;
            self.set_text(LAST, &first.last.to_string());
            self.set_poll_interval(100);
            self.resize();
            Ok(())
        }
    }
    unsafe fn start_audio(&mut self) -> Result<()> {
        unsafe {
            let surah = self.select(CHAPTER) as u32 + 1;
            let first = self.ayah_input(FIRST)?;
            let last = self.ayah_input(LAST)?;
            let reciter = self.chosen_reciter()?;
            self.prepare_recitation_rate();
            if self.audio.is_none() {
                self.audio = Some(Audio::with_cache(self.cache()?)?);
            }
            self.set_text(AUDIO_TIME, self.tr("Loading audio…", "جارٍ تحميل الصوت…"));
            self.reciter = reciter;
            let range = ListeningRange {
                reciter,
                surah,
                first,
                last,
                count: self
                    .resources
                    .surahs
                    .get(surah.saturating_sub(1) as usize)
                    .ok_or("Choose a valid surah")?
                    .verses_count,
            };
            let continuous =
                crate::timed_audio::Recording::enabled(self.prefs.continuous_audio, reciter);
            if continuous {
                self.audio.as_mut().unwrap().play_timed(
                    range,
                    self.prefs.speed,
                    self.prefs.volume,
                )?;
                self.selected = None;
                self.invalidate();
            } else {
                self.audio.as_mut().unwrap().play_range(
                    range,
                    self.prefs.speed,
                    self.prefs.volume,
                )?;
            }
            self.audio.as_mut().unwrap().repeat(false)?;
            self.listening_session = None;
            self.active_range = Some((range, continuous));
            self.repeats = crate::app_core::RepeatCount::new(self.prefs.repeat_plays);
            self.manual_navigation();
            self.opening_highlight = None;
            self.playback.begin();
            self.set_poll_interval(100);
            self.resize();
            Ok(())
        }
    }
    fn cache(&mut self) -> Result<Arc<AudioCache>> {
        if self.audio_cache.is_none() {
            self.audio_cache = Some(AudioCache::new(self.profile.clone())?);
        }
        let gib = self.prefs.offline_limit_gib.min(64);
        self.audio_cache
            .as_ref()
            .unwrap()
            .set_library_limit(if gib == 0 {
                u64::MAX
            } else {
                u64::from(gib) * 1024 * 1024 * 1024
            });
        Ok(self.audio_cache.as_ref().unwrap().clone())
    }
    fn set_poll_interval(&mut self, interval: u32) {
        if self.poll_interval != interval {
            unsafe {
                SetTimer(Some(self.hwnd), 1, interval, None);
            }
            self.poll_interval = interval;
        }
    }
    fn visible_pages(&self) -> Vec<u32> {
        if self.single_page() {
            vec![self.prefs.page]
        } else {
            let (right, left) = data::spread(self.prefs.page);
            let mut pages = vec![right];
            if let Some(left) = left {
                pages.push(left);
            }
            pages
        }
    }
    unsafe fn observe_reading(&mut self, day: &str, now: Instant) -> Result<()> {
        unsafe {
            let reached = self.reader.goal_reached(day);
            // Only navigation changes the observed pages. Opening a panel or
            // resizing must not count pages that disappear from the layout.
            if self.reading_page != self.prefs.page {
                self.reading_page = self.prefs.page;
                self.reading_pages = self.visible_pages();
            }
            self.reader_dirty |= self.reader.update_reading(
                day,
                &self.reading_pages,
                !IsIconic(self.hwnd).as_bool(),
                now,
            )?;
            if !reached && self.reader.goal_reached(day) {
                self.notice(
                    self.tr("Daily reading goal reached", "أتممت هدف القراءة اليومي")
                        .to_owned(),
                );
            }
            Ok(())
        }
    }
    unsafe fn refresh_text_view(&mut self) -> Result<()> {
        unsafe {
            let pages = self.visible_pages();
            if !self.text_view || self.text_pages == pages {
                return Ok(());
            }
            let mut texts = Vec::new();
            for page in &pages {
                texts.push(self.resources.page_text(*page)?);
            }
            self.set_text(TEXT_VIEW, &texts.join("\r\n\r\n"));
            self.text_pages = pages;
            Ok(())
        }
    }
    unsafe fn notice(&mut self, message: String) {
        unsafe {
            self.notice_until = Some(Instant::now() + std::time::Duration::from_secs(3));
            self.set_text(NOTICE, &message);
            let w = (self.width - 48.).min(620.);
            let h = if message.chars().count() > 80 {
                64.
            } else {
                28.
            };
            self.place(
                NOTICE,
                Rect {
                    x: (self.width - w) / 2.,
                    y: self.height - h - 12.,
                    w,
                    h,
                },
                true,
            );
            self.invalidate();
        }
    }
    unsafe fn download_surah(&mut self) -> Result<()> {
        unsafe {
            if let Some(job) = self.download.take() {
                job.cancel();
                self.set_text(
                    DOWNLOAD_STATUS,
                    self.tr("Download cancelled", "تم إلغاء التنزيل"),
                );
            } else {
                let selected = SendMessageW(self.controls[&RECITER], CB_GETCURSEL, None, None).0;
                let reciter = if selected >= 0 {
                    Some(selected as usize)
                } else {
                    crate::audio::match_reciter(&self.text(RECITER))
                }
                .filter(|n| *n < RECITERS.len())
                .ok_or("Choose a matching reciter from the list")?;
                self.download_generation = self.download_generation.wrapping_add(1);
                self.download = Some(self.cache()?.download_surah(
                    reciter,
                    self.select(CHAPTER) as u32 + 1,
                    self.download_generation,
                )?);
                self.set_poll_interval(100);
                self.set_text(
                    DOWNLOAD_STATUS,
                    self.tr("Preparing download…", "جارٍ تجهيز التنزيل…"),
                );
            }
            self.set_text(
                DOWNLOAD,
                self.tr(
                    if self.download.is_some() {
                        "Cancel download"
                    } else {
                        "Download whole surah"
                    },
                    if self.download.is_some() {
                        "إلغاء التنزيل"
                    } else {
                        "تنزيل السورة كاملة"
                    },
                ),
            );
            Ok(())
        }
    }
    unsafe fn poll_download(&mut self) {
        unsafe {
            let mut done = false;
            for _ in 0..32 {
                let Some(event) = self.download.as_ref().and_then(Job::try_recv) else {
                    break;
                };
                match event {
                    DownloadEvent::Progress {
                        generation,
                        completed,
                        total,
                        percent,
                        bytes,
                    } if generation == self.download_generation => {
                        self.set_text(
                            DOWNLOAD_STATUS,
                            &format!(
                                "{}%  ·  {}/{}  ·  {:.1} MB",
                                percent,
                                completed,
                                total,
                                bytes as f64 / 1_048_576.
                            ),
                        );
                    }
                    DownloadEvent::Complete { generation, folder }
                        if generation == self.download_generation =>
                    {
                        self.set_text(
                            DOWNLOAD_STATUS,
                            self.tr(
                                "Whole surah saved for offline listening",
                                "تم حفظ السورة كاملة للاستماع دون اتصال",
                            ),
                        );
                        self.notice(format!(
                            "{}: {}",
                            self.tr("Download complete", "اكتمل التنزيل"),
                            folder.display()
                        ));
                        done = true;
                    }
                    DownloadEvent::Failed {
                        generation,
                        message,
                    } if generation == self.download_generation => {
                        self.set_text(
                            DOWNLOAD_STATUS,
                            self.tr(
                                "Download failed. Press Download to retry.",
                                "تعذر التنزيل. اضغط تنزيل للمحاولة مجددًا.",
                            ),
                        );
                        self.report_error(&message);
                        done = true;
                    }
                    DownloadEvent::Cancelled { generation }
                        if generation == self.download_generation =>
                    {
                        done = true;
                    }
                    _ => {}
                }
            }
            if done {
                self.download = None;
                self.set_text(
                    DOWNLOAD,
                    self.tr("Download whole surah", "تنزيل السورة كاملة"),
                );
            }
        }
    }
    fn ensure_tafsir_library(&mut self) -> Result<Arc<TafsirLibrary>> {
        if let Some(library) = &self.tafsir_library {
            return Ok(library.clone());
        }
        reader_state::ensure_profile_directory(&self.profile)?;
        let library = TafsirLibrary::new(&self.profile)?;
        self.installed_tafsirs = library.installed_ids()?;
        self.resources.set_study_library(library.clone());
        self.tafsir_library = Some(library.clone());
        Ok(library)
    }
    unsafe fn refresh_sources(&mut self) {
        unsafe {
            self.study_sources = crate::app_core::study_source_ids(
                self.prefs.arabic,
                self.installed_tafsirs.iter().copied(),
            );
            if !self.study_sources.contains(&self.source) {
                self.source = if self.prefs.arabic { 0 } else { 2 };
            }
            let labels = self
                .study_sources
                .iter()
                .map(|id| match *id {
                    0 => self.tr("Al-Mukhtasar", "المختصر في التفسير").to_owned(),
                    1 => self.tr("As-Saadi", "تفسير السعدي").to_owned(),
                    2 => self
                        .tr(
                            "Saheeh International · translation",
                            "ترجمة صحيح إنترناشيونال",
                        )
                        .into(),
                    n => {
                        let source = &tafsir_library::SOURCES[n - 3];
                        if self.prefs.arabic {
                            source.name_arabic.into()
                        } else {
                            source.name_english.to_owned()
                        }
                    }
                })
                .collect::<Vec<String>>();
            self.set_text(
                SOURCE_SINGLE,
                labels
                    .first()
                    .map(|s| s.split(" · ").next().unwrap_or(s))
                    .unwrap_or(""),
            );
            let selected = self
                .study_sources
                .iter()
                .position(|source| *source == self.source)
                .unwrap_or(0);
            self.fill(SOURCE, &labels, selected);
        }
    }
    unsafe fn refresh_tafsir_actions(&self) {
        unsafe {
            let running = self.tafsir_job.is_some();
            for (i, source) in tafsir_library::SOURCES.iter().enumerate() {
                let installed = self.installed_tafsirs.contains(&i);
                let active = running && self.tafsir_job_source == Some(i);
                let action =
                    if !self.installed_tafsirs.contains(&i) && source.download_issue().is_some() {
                        self.tr("Pending", "غير متاح")
                    } else if active {
                        self.tr("Cancel", "إلغاء")
                    } else if installed {
                        self.tr("Remove", "حذف")
                    } else {
                        self.tr("Download", "تنزيل")
                    };
                let name = if self.prefs.arabic {
                    source.name_arabic
                } else {
                    source.name_english
                };
                let id = TAFSIR_ACTION_FIRST + i as u32;
                // Keep the source in the native accessible name, while the
                // small visible button displays only its action.
                self.set_text(id, &format!("{action} · {name}"));
                let _ = EnableWindow(
                    self.controls[&id],
                    (!running || active)
                        && (self.installed_tafsirs.contains(&i)
                            || source.download_issue().is_none()),
                );
            }
            self.invalidate();
        }
    }
    fn tafsir_rows(&self) -> Vec<usize> {
        tafsir_library::SOURCES
            .iter()
            .enumerate()
            .filter(|(_, source)| source.arabic == self.tafsir_arabic)
            .map(|(id, _)| id)
            .collect()
    }
    unsafe fn refresh_tafsir_help(&self) {
        unsafe {
            if self.tafsir_job.is_none() {
                self.set_text(TAFSIR_INFO, if self.tafsir_arabic { self.tr("Quran.com · Download a complete tafsir for offline reading. Included explanations remain available offline.", "نزّل كتاب التفسير كاملًا لقراءته دون اتصال. يبقى التفسيران المرفقان متاحين دائمًا.") } else { self.tr("Quran.com lists these English tafsirs. Their offline downloads are temporarily unavailable because the publisher's verse mappings did not pass verification. The included Saheeh International translation remains available offline.", "تنزيل هذه التفاسير الإنجليزية متوقف مؤقتًا لأن مراجع الآيات لدى المصدر لم تجتز التحقق. تبقى الترجمة الإنجليزية المرفقة متاحة دون اتصال.") });
            }
        }
    }
    unsafe fn open_tafsirs(&mut self) -> Result<()> {
        unsafe {
            self.ensure_tafsir_library()?;
            self.tafsir_arabic = self.prefs.arabic;
            self.set_panel(Panel::Tafsirs);
            self.search = false;
            self.refresh_labels();
            self.refresh_tafsir_help();
            self.resize();
            Ok(())
        }
    }
    unsafe fn tafsir_action(&mut self, source: usize) -> Result<()> {
        unsafe {
            if source >= tafsir_library::SOURCES.len() {
                return Err("Invalid tafsir action source".into());
            }
            if let Some(job) = self.tafsir_job.as_ref() {
                // A queued click on another row cannot cancel or retarget the
                // operation currently working on a different original source.
                if self.tafsir_job_source != Some(source) {
                    return Ok(());
                }
                job.cancel();
                self.notice(
                    self.tr("Cancelling tafsir download…", "جارٍ إلغاء تنزيل التفسير…")
                        .to_owned(),
                );
            } else {
                let library = self.ensure_tafsir_library()?;
                self.tafsir_generation = self.tafsir_generation.wrapping_add(1);
                let remove = self.installed_tafsirs.contains(&source);
                self.tafsir_job = Some(if remove {
                    library.remove_async(source, self.tafsir_generation)?
                } else {
                    library.download(source, self.tafsir_generation)?
                });
                self.tafsir_job_source = Some(source);
                self.set_text(
                    TAFSIR_INFO,
                    self.tr(
                        if remove {
                            "Removing this downloaded book…"
                        } else {
                            "Preparing the complete tafsir download…"
                        },
                        if remove {
                            "جارٍ حذف الكتاب المنزّل…"
                        } else {
                            "جارٍ تجهيز تنزيل كتاب التفسير كاملًا…"
                        },
                    ),
                );
                self.set_poll_interval(100);
            }
            self.refresh_tafsir_actions();
            Ok(())
        }
    }
    unsafe fn poll_tafsir(&mut self) -> Result<()> {
        unsafe {
            let mut done = false;
            for _ in 0..32 {
                let Some(job) = self.tafsir_job.as_ref() else {
                    break;
                };
                let event = match job.try_recv() {
                    Ok(event) => event,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(_) => {
                        self.report_error(&"Tafsir operation ended unexpectedly. Please retry.");
                        done = true;
                        break;
                    }
                };
                match event {
                    TafsirEvent::Progress {
                        generation,
                        chapters,
                        total,
                        bytes,
                        percent,
                    } if generation == self.tafsir_generation => {
                        self.set_text(
                            TAFSIR_INFO,
                            &format!(
                                "{}\r\n\r\n{}%  ·  {}/{}  ·  {:.1} MB",
                                self.tr(
                                    "Downloading complete tafsir…",
                                    "جارٍ تنزيل كتاب التفسير كاملًا…"
                                ),
                                percent,
                                chapters,
                                total,
                                bytes as f64 / 1_048_576.
                            ),
                        );
                    }
                    TafsirEvent::Complete { generation, source }
                    | TafsirEvent::Removed { generation, source }
                        if generation == self.tafsir_generation =>
                    {
                        if source >= tafsir_library::SOURCES.len() {
                            return Err("Invalid completed tafsir source".into());
                        }
                        self.installed_tafsirs = self.ensure_tafsir_library()?.installed_ids()?;
                        self.refresh_labels();
                        let message = self
                            .tr("Tafsir library updated", "تم تحديث مكتبة التفسير")
                            .to_owned();
                        self.set_text(TAFSIR_INFO, &message);
                        self.notice(message);
                        done = true;
                    }
                    TafsirEvent::Failed {
                        generation,
                        message,
                    } if generation == self.tafsir_generation => {
                        self.set_text(
                            TAFSIR_INFO,
                            self.tr(
                                "Unable to update this book. Check your connection and try again.",
                                "تعذر تحديث الكتاب. تحقق من اتصالك وحاول مجددًا.",
                            ),
                        );
                        self.report_error(&message);
                        done = true;
                    }
                    TafsirEvent::Cancelled { generation }
                        if generation == self.tafsir_generation =>
                    {
                        done = true;
                    }
                    _ => {}
                }
                if done {
                    break;
                }
            }
            if done {
                self.tafsir_job = None;
                self.tafsir_job_source = None;
                self.refresh_tafsir_actions();
            }
            Ok(())
        }
    }
    unsafe fn bookmarks(&mut self) -> Result<()> {
        unsafe {
            if self.reader.bookmarks.is_empty() {
                self.notice(
                    self.tr(
                        "No bookmarks yet. Press Ctrl+B to save this page.",
                        "لا توجد علامات محفوظة. اضغط Ctrl+B لحفظ هذه الصفحة.",
                    )
                    .to_owned(),
                );
                return Ok(());
            }
            let menu = CreatePopupMenu()?;
            for (index, bookmark) in self.reader.bookmarks.iter().enumerate() {
                let label = format!(
                    "{} {}{}",
                    self.tr("Page", "صفحة"),
                    bookmark.page,
                    bookmark
                        .selected
                        .map(|(s, a)| format!("  ·  {s}:{a}"))
                        .unwrap_or_default()
                );
                AppendMenuW(
                    menu,
                    MF_STRING,
                    BOOKMARK_FIRST as usize + index,
                    &HSTRING::from(label),
                )?;
            }
            let mut point = POINT { x: 16, y: 48 };
            let _ = ClientToScreen(self.hwnd, &mut point);
            let id = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY,
                point.x,
                point.y,
                None,
                self.hwnd,
                None,
            )
            .0 as u32;
            DestroyMenu(menu)?;
            if id != 0 {
                self.command(id, 0)?;
            }
            Ok(())
        }
    }
    unsafe fn welcome(&mut self) -> Result<()> {
        unsafe {
            self.show_popup(
                "Interface language · لغة الواجهة".into(),
                vec![
                    MenuRow {
                        id: WELCOME_ENGLISH,
                        label: "English".into(),
                        detail: "Menus and settings in English".into(),
                        children: vec![],
                    },
                    MenuRow {
                        id: WELCOME_ARABIC,
                        label: "العربية".into(),
                        detail: "القوائم والإعدادات باللغة العربية".into(),
                        children: vec![],
                    },
                ],
                self.width / 2. - 172.,
                100.,
                false,
            )
        }
    }
    unsafe fn help(&mut self) -> Result<()> {
        unsafe { self.start_guide() }
    }
    unsafe fn toggle_fullscreen(&mut self) -> Result<()> {
        unsafe {
            if self.fullscreen {
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, self.window_style);
                SetWindowPlacement(self.hwnd, &self.placement)?;
                SetWindowPos(
                    self.hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
                )?;
            } else {
                self.window_style = GetWindowLongPtrW(self.hwnd, GWL_STYLE);
                self.placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
                GetWindowPlacement(self.hwnd, &mut self.placement)?;
                let monitor = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                    return Err("Missing display bounds".into());
                }
                SetWindowLongPtrW(
                    self.hwnd,
                    GWL_STYLE,
                    ((self.window_style as u32 & !WS_OVERLAPPEDWINDOW.0) | WS_POPUP.0) as isize,
                );
                let r = info.rcMonitor;
                SetWindowPos(
                    self.hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_FRAMECHANGED | SWP_NOZORDER,
                )?;
            }
            self.fullscreen = !self.fullscreen;
            FULLSCREEN_STATE.with(|state| state.set(self.fullscreen));
            self.resize();
            self.set_text(
                FULLSCREEN,
                self.tr(
                    if self.fullscreen {
                        "Exit full screen"
                    } else {
                        "Full screen"
                    },
                    if self.fullscreen {
                        "الخروج من ملء الشاشة"
                    } else {
                        "ملء الشاشة"
                    },
                ),
            );
            Ok(())
        }
    }
    unsafe fn command(&mut self, id: u32, notification: u32) -> Result<()> {
        unsafe {
            if notification == 0 && self.popup.is_some() && id < MENU_ROW && id != SETTINGS {
                self.popup = None;
                self.resize();
            }
            match id {
                MENU_BACK | MENU_CLOSE => self.popup_command(id)?,
                n if (MENU_ROW..MENU_ROW + 32).contains(&n) => self.popup_command(n)?,
                SEARCH => {
                    self.search = !self.search;
                    if self.search {
                        self.browse_tab = BROWSE_SURAH;
                        self.set_text(QUERY, "");
                        self.set_text(BROWSE_QUERY, "");
                        self.panel = Panel::None;
                        self.panel_animation = None;
                        self.panel_scroll = 0.;
                    }
                    let mut enabled = BOOL::from(true);
                    SystemParametersInfoW(
                        SPI_GETCLIENTAREAANIMATION,
                        0,
                        Some((&mut enabled as *mut BOOL).cast()),
                        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                    )
                    .ok();
                    // Build rows and labels before starting the visual motion.
                    // A previously armed input debounce cannot reset the list
                    // halfway through its opening transition.
                    KillTimer(Some(self.hwnd), 8).ok();
                    if self.search {
                        self.update_search_model()?;
                    }
                    self.search_animation = if self.search && enabled.as_bool() {
                        Some(Instant::now())
                    } else {
                        None
                    };
                    if self.search_animation.is_some() {
                        SetTimer(Some(self.hwnd), 6, 10, None);
                    }
                    self.resize();
                    if self.search {
                        focus_window(self.controls[&QUERY])?;
                    } else {
                        focus_window(self.hwnd)?;
                    }
                }
                BROWSE_SURAH | BROWSE_PAGE | BROWSE_SECTIONS => {
                    self.browse_tab = id;
                    self.set_text(BROWSE_QUERY, "");
                    self.update_search()?;
                    focus_window(self.controls[&BROWSE_QUERY])?;
                }
                BROWSE_KIND if notification == CBN_SELCHANGE => self.update_search()?,
                RESULT_PICK if self.search => self.choose_hit()?,
                GO => self.find()?,
                SEARCH_CLOSE => {
                    self.search = false;
                    self.search_animation = None;
                    self.resize();
                    focus_window(self.hwnd)?;
                }
                QUERY | BROWSE_QUERY if notification == EN_CHANGE => {
                    if self.search {
                        SetTimer(Some(self.hwnd), 8, 120, None);
                    }
                }
                RESULTS if notification == LBN_DBLCLK => self.choose_hit()?,
                PREV => self.request_turn(1, crate::app_core::TurnSource::Discrete)?,
                NEXT => self.request_turn(-1, crate::app_core::TurnSource::Discrete)?,
                LISTEN => {
                    if self.panel == Panel::Listen {
                        self.set_panel(Panel::None);
                        self.resize();
                        focus_window(self.hwnd)?;
                    } else {
                        self.open_listen();
                    }
                }
                QUICK_SETTINGS => self.open_listen(),
                STUDY if notification == 0 => {
                    self.manual_navigation();
                    // RichEdit sends EN_UPDATE/EN_CHANGE while painting and
                    // formatting. They are notifications, not open commands.
                    if self.prefs.follow_tafsir {
                        self.prefs.follow_tafsir = false;
                        self.dirty = true;
                        self.refresh_study_follow();
                    }
                    self.set_panel(Panel::Study);
                    self.search = false;
                    self.study()?;
                    self.resize();
                }
                CLOSE => {
                    self.set_panel(Panel::None);
                    self.resize();
                    focus_window(self.hwnd)?;
                }
                AYAH_PREV => self.ayah(-1)?,
                AYAH_NEXT => self.ayah(1)?,
                MINIMIZE => {
                    let _ = ShowWindow(self.hwnd, SW_MINIMIZE);
                }
                MAXIMIZE => {
                    let _ = ShowWindow(
                        self.hwnd,
                        if IsZoomed(self.hwnd).as_bool() {
                            SW_RESTORE
                        } else {
                            SW_MAXIMIZE
                        },
                    );
                    self.resize();
                }
                WINDOW_CLOSE => {
                    PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                }
                SETTINGS => self.options()?,
                FULLSCREEN => self.toggle_fullscreen()?,
                LANG => {
                    self.prefs.language_chosen = true;
                    let left = self
                        .placements
                        .borrow()
                        .iter()
                        .filter(|(id, _)| matches!(**id, SEARCH | LISTEN | SETTINGS))
                        .map(|(_, (r, _))| r.x)
                        .fold(f32::INFINITY, f32::min);
                    let mut enabled = BOOL::from(true);
                    SystemParametersInfoW(
                        SPI_GETCLIENTAREAANIMATION,
                        0,
                        Some((&mut enabled as *mut BOOL).cast()),
                        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                    )
                    .ok();
                    self.title_animation = None;
                    if self.prefs.arabic {
                        if self.source != 2 {
                            self.arabic_source = self.source;
                        }
                        self.source = 2;
                    } else {
                        self.source = self.arabic_source;
                    }
                    self.prefs.arabic = !self.prefs.arabic;
                    self.dirty = true;
                    self.refresh_labels();
                    if self.panel == Panel::Study {
                        self.study()?;
                    }
                    self.title_animation =
                        (enabled.as_bool() && left.is_finite()).then(|| (Instant::now(), left));
                    #[cfg(feature = "qa")]
                    if self.title_trace.is_none() && self.title_animation.is_some() {
                        self.title_trace = Some((Instant::now(), vec![]));
                    }
                    if self.title_animation.is_some() {
                        SetTimer(Some(self.hwnd), 6, 10, None);
                    }
                    self.resize();
                }
                THEME_SELECTOR if notification == CBN_SELCHANGE => {
                    self.theme(self.select(THEME_SELECTOR).min(3) as u32);
                }
                n if (THEME..=THEME + 3).contains(&n) => self.theme(n - THEME),
                CONTEXTUAL => {
                    self.prefs.contextual = !self.prefs.contextual;
                    self.renderer.contextual = self.prefs.contextual;
                    self.dirty = true;
                    self.invalidate();
                }
                SOURCE if notification == CBN_SELCHANGE => {
                    self.source = *self
                        .study_sources
                        .get(self.select(SOURCE))
                        .ok_or("Invalid study selection")?;
                    if self.source != 2 {
                        self.arabic_source = self.source;
                    }
                    self.study()?;
                }
                SOURCE if notification == CBN_CLOSEUP => {
                    focus_window(self.controls[&STUDY])?;
                }
                MANAGE_TAFSIR => self.open_tafsirs()?,
                n if (TAFSIR_ACTION_FIRST
                    ..TAFSIR_ACTION_FIRST + tafsir_library::SOURCES.len() as u32)
                    .contains(&n) =>
                {
                    self.tafsir_action((n - TAFSIR_ACTION_FIRST) as usize)?;
                }
                CHAPTER if notification == CBN_SELCHANGE => {
                    let s = self.select(CHAPTER);
                    self.set_text(FIRST, "1");
                    self.set_text(LAST, &self.resources.surahs[s].verses_count.to_string());
                }
                RECORDING_MODE if notification == CBN_SELCHANGE => {
                    self.prefs.continuous_audio = self.select(RECORDING_MODE) == 1;
                    self.dirty = true;
                    self.notice(self.tr("Recording choice applies when you start playback. Full-surah timing is available for Yasser Al-Dosari and Mishary Alafasy.", "يُطبق الاختيار عند بدء التلاوة. يتوفر توقيت السورة لياسر الدوسري ومشاري العفاسي.").into());
                }
                SPEED if notification == CBN_SELCHANGE => {
                    self.prefs.speed = [0.75, 1., 1.25, 1.5, 2.][self.select(SPEED).min(4)];
                    self.speed_selection_pending =
                        !self.audio.as_ref().is_some_and(Audio::has_session);
                    self.dirty = true;
                    if let Some(audio) = &mut self.audio {
                        audio.request_speed(self.prefs.speed)?;
                    }
                }
                VOLUME => {
                    self.prefs.volume = SendMessageW(self.controls[&VOLUME], TBM_GETPOS, None, None)
                        .0
                        .clamp(0, 100) as f64
                        / 100.;
                    self.set_text(
                        VOLUME_LABEL,
                        &format!(
                            "{} · {}%",
                            self.tr("Volume", "مستوى الصوت"),
                            (self.prefs.volume * 100.).round() as u32
                        ),
                    );
                    self.dirty = true;
                    if let Some(audio) = &self.audio {
                        audio.volume(self.prefs.volume)?;
                    }
                }
                REPEAT if notification == CBN_SELCHANGE => {
                    self.prefs.repeat_plays = [1, 2, 3, 5, 10, 0][self.select(REPEAT).min(5)];
                    self.dirty = true;
                }
                WELCOME_ENGLISH | WELCOME_ARABIC => {
                    let arabic = id == WELCOME_ARABIC;
                    if self.prefs.arabic != arabic {
                        self.command(LANG, 0)?;
                    }
                    self.prefs.language_chosen = true;
                    self.reader.onboarding_completed = true;
                    self.reader_dirty = true;
                    self.dirty = true;
                    self.popup = None;
                    self.resize();
                }
                SEARCH_KEY_T => {
                    self.prefs.search_key_t = !self.prefs.search_key_t;
                    self.dirty = true;
                }
                CONTINUOUS_AUDIO => {
                    self.prefs.continuous_audio = !self.prefs.continuous_audio;
                    self.dirty = true;
                    self.notice(
                        self.tr(
                            "Recording mode will apply the next time you press Play.",
                            "يُطبق وضع التسجيل عند الضغط على التشغيل مجددًا.",
                        )
                        .into(),
                    );
                }
                n if (OFFLINE_LIMIT_FIRST..OFFLINE_LIMIT_FIRST + 4).contains(&n) => {
                    self.prefs.offline_limit_gib =
                        [1, 4, 16, 0][(n - OFFLINE_LIMIT_FIRST) as usize];
                    self.cache()?;
                    self.dirty = true;
                }
                COLLAPSE_PLAYER => {
                    if self.displayed_panel() == Panel::None
                        && self.compact_player_frame().is_none()
                    {
                        self.open_listen();
                        return Ok(());
                    }
                    self.player_collapsed = !self.player_collapsed;
                    self.resize();
                    self.invalidate();
                }
                PLAY => self.start_audio()?,
                SEEK_BACK | SEEK_FORWARD => {
                    self.seek_audio(Some(if id == SEEK_BACK { -10_000 } else { 10_000 }), 0)?;
                }
                FOLLOW_AUDIO => self.reveal_recitation()?,
                STUDY_OPTIONS => self.study_options()?,
                STUDY_FOLLOW => self.toggle_study_follow()?,
                PAUSE | QUICK_PAUSE => {
                    if let Some(audio) = &mut self.audio {
                        audio.toggle()?;
                        if let Some(scoped) = &mut self.listening_session {
                            scoped.paused = !scoped.paused;
                        }
                    }
                    self.refresh_transport();
                    self.invalidate();
                }
                STOP | QUICK_STOP => {
                    self.playback = crate::app_core::PlaybackFollow::default();
                    self.audio = None;
                    self.opening_highlight = None;
                    self.active_range = None;
                    self.listening_session = None;
                    self.repeats = crate::app_core::RepeatCount::new(1);
                    self.invalidate_reader();
                    self.set_text(STATUS, "");
                    self.resize();
                }
                TEXT_SIZE => {
                    let rows = (14..=28)
                        .step_by(2)
                        .map(|size| MenuRow {
                            id: STUDY_SIZE_FIRST + size as u32,
                            label: format!(
                                "{}{}",
                                if self.study_size == size { "✓ " } else { "" },
                                size
                            ),
                            detail: String::new(),
                            children: Vec::new(),
                        })
                        .collect();
                    let r = self.placements.borrow()[&TEXT_SIZE].0;
                    self.show_popup(
                        self.tr("Text size", "حجم النص").into(),
                        rows,
                        if self.prefs.arabic { r.x + r.w } else { r.x },
                        r.y + r.h + 4.,
                        false,
                    )?;
                }
                n if (STUDY_SIZE_FIRST + 14..=STUDY_SIZE_FIRST + 28).contains(&n)
                    && (n - STUDY_SIZE_FIRST) % 2 == 0 =>
                {
                    self.study_size = (n - STUDY_SIZE_FIRST) as i32;
                    self.prefs.study_text_size = self.study_size;
                    self.dirty = true;
                    self.refresh_study_font();
                }
                TAFSIR_ENGLISH | TAFSIR_ARABIC => {
                    self.tafsir_arabic = id == TAFSIR_ARABIC;
                    self.refresh_tafsir_help();
                    self.resize();
                }
                TEXT_SMALL | TEXT_LARGE => {
                    self.study_size =
                        (self.study_size + if id == TEXT_SMALL { -2 } else { 2 }).clamp(14, 28);
                    self.prefs.study_text_size = self.study_size;
                    self.dirty = true;
                    self.refresh_study_font();
                }
                DOWNLOAD => self.download_surah()?,
                STORAGE => self.storage_details()?,
                BOOKMARK => {
                    let added = self.reader.toggle_bookmark(
                        self.prefs.page,
                        self.selected,
                        &self.resources.surahs,
                    )?;
                    self.reader_dirty = true;
                    self.notice(
                        self.tr(
                            if added {
                                "Bookmark saved"
                            } else {
                                "Bookmark removed"
                            },
                            if added {
                                "تم حفظ العلامة"
                            } else {
                                "تم حذف العلامة"
                            },
                        )
                        .to_owned(),
                    );
                    self.reader.save(&self.profile)?;
                    self.reader_dirty = false;
                }
                BOOKMARKS => self.bookmarks()?,
                n if (BOOKMARK_FIRST..BOOKMARK_FIRST + self.reader.bookmarks.len() as u32)
                    .contains(&n) =>
                {
                    self.manual_navigation();
                    let bookmark = &self.reader.bookmarks[(n - BOOKMARK_FIRST) as usize];
                    self.prefs.page = bookmark.page;
                    self.selected = bookmark.selected;
                    self.dirty = true;
                    self.observe_reading(&local_day().0, Instant::now())?;
                    self.text_pages.clear();
                    if self.panel == Panel::Study {
                        self.study()?;
                    }
                    self.resize();
                }
                ACCESSIBLE => self.accessible_view()?,
                ENABLE_TEXT_VIEW => self.set_accessible_view(true)?,
                EXIT_TEXT_VIEW => self.set_accessible_view(false)?,
                CANCEL_TEXT_VIEW => {
                    self.popup = None;
                    self.resize();
                    focus_window(self.hwnd)?;
                }
                REMEMBER_SPEED => {
                    self.prefs.remember_speed = !self.prefs.remember_speed;
                    self.dirty = true;
                }
                CLOCK | CLOCK_24 => {
                    if id == CLOCK {
                        self.prefs.show_clock = !self.prefs.show_clock;
                    } else {
                        self.prefs.clock_24 = !self.prefs.clock_24;
                    }
                    self.renderer.show_clock = self.prefs.show_clock;
                    self.renderer.clock_24 = self.prefs.clock_24;
                    self.dirty = true;
                    self.invalidate();
                }
                TIMER | REMINDER => {
                    if id == TIMER {
                        self.reader.reading.show_timer = !self.reader.reading.show_timer;
                    } else {
                        self.reader.reading.reminder_enabled =
                            !self.reader.reading.reminder_enabled;
                    }
                    self.reader_dirty = true;
                }
                n if (GOAL..GOAL + 4).contains(&n) => {
                    self.reader.goal.mode = if n < GOAL + 2 {
                        GoalMode::Pages
                    } else {
                        GoalMode::Minutes
                    };
                    self.reader.goal.target = if (n - GOAL) % 2 == 0 { 5 } else { 10 };
                    self.reader_dirty = true;
                }
                HELP => self.help()?,
                GUIDE_BACK | GUIDE_NEXT | GUIDE_CLOSE | GUIDE_SHOW => self.guide_command(id)?,
                DAILY => {
                    let (day, _, _) = local_day();
                    if self.daily.is_some() {
                        self.observe_reading(&day, Instant::now())?;
                        self.reader
                            .stop_reading(&day, &self.reading_pages, Instant::now())?;
                    } else {
                        self.reading_page = self.prefs.page;
                        self.reading_pages = self.visible_pages();
                        self.reader
                            .start_reading(&day, &self.reading_pages, Instant::now())?;
                    }
                    self.reader_dirty = true;
                    self.daily = if self.daily.is_some() {
                        None
                    } else {
                        Some(Instant::now())
                    };
                    self.set_text(
                        DAILY,
                        self.tr(
                            if self.daily.is_some() {
                                "Finish reading"
                            } else {
                                "Start reading"
                            },
                            if self.daily.is_some() {
                                "إنهاء القراءة"
                            } else {
                                "بدء القراءة"
                            },
                        ),
                    );
                }
                _ => {}
            }
            Ok(())
        }
    }
    unsafe fn tick(&mut self) -> Result<()> {
        unsafe {
            // Occlusion by a menu/guide is not a change in reader geometry.
            let previous_status_height = self.status_height();
            let previous_side_playback = self.side_playback();
            if let Some(audio) = &mut self.audio {
                audio.poll()?;
            }
            #[cfg(feature = "qa")]
            if let Some(test) = &mut self.large_handoff {
                if test.posted_heartbeat() {
                    PostMessageW(Some(self.hwnd), QA_HEARTBEAT, WPARAM(0), LPARAM(0))?;
                }
                if self.audio.as_ref().is_some_and(Audio::completed) {
                    if let Some((range, _)) = self.active_range {
                        test.record_decoded_completion(range.surah)?;
                    }
                }
            }
            if !self.audio.as_ref().is_some_and(Audio::has_session)
                && self.playback.current.is_some()
            {
                self.playback.current = None;
                self.invalidate_reader();
            }
            let next_scoped = if self.audio.as_ref().is_some_and(Audio::completed) {
                self.listening_session
                    .as_mut()
                    .filter(|scoped| !scoped.session.finished() && !scoped.paused)
                    .map(|scoped| {
                        (
                            scoped.session.complete_current(),
                            scoped.reciter,
                            scoped.continuous,
                            scoped.rolling,
                        )
                    })
            } else {
                None
            };
            if let Some((next, reciter, continuous, rolling)) = next_scoped {
                if let Some(chunk) = next {
                    if let Err(error) = self.play_plan_chunk(chunk, reciter, continuous, rolling) {
                        // A failed chapter start must not leave the previous
                        // completed source able to advance again on idle ticks.
                        self.listening_session = None;
                        self.active_range = None;
                        if let Some(audio) = &mut self.audio {
                            let _ = audio.stop();
                        }
                        self.playback.current = None;
                        self.invalidate_reader();
                        return Err(error);
                    }
                    self.playback.current = None;
                    self.invalidate_reader();
                } else {
                    self.active_range = None;
                }
            }
            let repeat_range = self.active_range.filter(|_| {
                self.listening_session.is_none()
                    && self.audio.as_ref().is_some_and(Audio::completed)
                    && self.repeats.completed()
            });
            if let Some((range, continuous)) = repeat_range {
                let audio = self.audio.as_mut().unwrap();
                if continuous {
                    audio.play_timed(range, self.prefs.speed, self.prefs.volume)?;
                } else {
                    audio.play_range(range, self.prefs.speed, self.prefs.volume)?;
                }
                audio.repeat(false)?;
                self.playback.current = None;
                self.invalidate_reader();
            }
            self.poll_download();
            self.poll_tafsir()?;
            self.poll_storage()?;
            self.set_poll_interval(
                if self.audio.as_ref().is_some_and(Audio::needs_fast_poll)
                    || self.download.is_some()
                    || self.storage_pending.is_some()
                    || self.tafsir_job.is_some()
                {
                    100
                } else {
                    1000
                },
            );
            if self.text_view {
                self.refresh_text_view()?;
            }
            let (day, hour, minute) = local_day();
            let clock_minute = hour * 60 + minute;
            if self.prefs.show_clock && self.clock_minute != clock_minute {
                self.clock_minute = clock_minute;
                self.invalidate();
            }
            self.observe_reading(&day, Instant::now())?;
            if self.reader.reminder_due(&day, hour as u32, minute as u32) {
                self.reader_dirty = true;
                self.notice(
                    self.tr(
                        "A little time for your daily reading?",
                        "هل حان وقت وردك اليومي؟",
                    )
                    .to_owned(),
                );
                let info = FLASHWINFO {
                    cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                    hwnd: self.hwnd,
                    dwFlags: FLASHW_TRAY,
                    uCount: 3,
                    dwTimeout: 0,
                };
                let _ = FlashWindowEx(&info);
            }
            let notice = self
                .notice_until
                .is_some_and(|until| Instant::now() < until);
            if !notice {
                self.notice_until = None;
            }
            if IsWindowVisible(self.controls[&NOTICE]).as_bool() != notice {
                self.resize();
            }
            if let Some(audio) = self.audio.as_ref().filter(|audio| audio.has_session()) {
                let key = audio.current();
                self.refresh_transport();
                self.follow_recitation(key)?;
            } else if self.daily.is_some() {
                let (current, unit) = match self.reader.goal.mode {
                    GoalMode::Pages => (
                        self.reader.goal.pages.len() as u64,
                        self.tr("pages", "صفحات"),
                    ),
                    GoalMode::Minutes => {
                        (self.reader.goal.seconds / 60, self.tr("minutes", "دقائق"))
                    }
                };
                let timer = if self.reader.reading.show_timer {
                    let seconds = self.reader.session_seconds().unwrap_or(0);
                    format!("  ·  {}:{:02}", seconds / 60, seconds % 60)
                } else {
                    String::new()
                };
                self.set_text(
                    STATUS,
                    &format!(
                        "{}  {}/{} {}{}",
                        self.tr("Daily reading", "الورد اليومي"),
                        current,
                        self.reader.goal.target,
                        unit,
                        timer
                    ),
                );
            } else if self.download.is_some() {
                self.set_text(STATUS, &self.text(DOWNLOAD_STATUS));
            } else {
                self.set_text(STATUS, "");
            }
            if previous_status_height != self.status_height()
                || previous_side_playback != self.side_playback()
            {
                self.resize();
            }
            if self.maintenance_ready()
                && self.dirty
                && self.last_preferences_save.elapsed().as_secs() >= 1
            {
                self.last_preferences_save = Instant::now();
                reader_state::ensure_profile_directory(&self.profile)?;
                reader_state::save_json_atomic(
                    &self.profile.join("preferences.json"),
                    &self.prefs,
                    64 * 1024,
                )?;
                self.dirty = false;
            }
            if self.maintenance_ready()
                && self.reader_dirty
                && self.last_reader_save.elapsed().as_secs() >= 5
            {
                self.last_reader_save = Instant::now();
                self.reader.save(&self.profile)?;
                self.reader_dirty = false;
            }
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_restore_window(&mut self) -> Result<()> {
        unsafe {
            let monitor = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return Err(windows::core::Error::from_thread().into());
            }
            // GetWindowRect and monitor work areas are already physical pixels.
            // Applying DPI again here would move the capture off the monitor.
            let rect = fit_window_rect(self.qa_original_rect, info.rcWork);
            SetWindowPos(
                self.hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER,
            )?;
            self.resize();
            Ok(())
        }
    }
    /// A focused fixture exercises actual STATIC repaint and color messages
    /// while the normal dispatch guard owns App, in every supported theme.
    #[cfg(feature = "qa")]
    unsafe fn qa_color_tick(&mut self) -> Result<()> {
        unsafe {
            let Some(out) = qa_argument("--qa-control-colors") else {
                return Ok(());
            };
            if !self.first_paint {
                return Ok(());
            }
            std::fs::create_dir_all(&out)?;
            let result = (|| -> Result<()> {
                match self.qa_step {
                    0 => {
                        GetWindowRect(self.hwnd, &mut self.qa_original_rect)?;
                        self.qa_restore_window()?;
                        self.theme(0);
                    }
                    1..=4 => {
                        let palette = Palette::new(self.prefs.theme);
                        self.notice_until =
                            Some(Instant::now() + std::time::Duration::from_secs(8));
                        self.set_text(NOTICE, "Status repaint color regression");
                        let status = self.controls[&NOTICE];
                        let _ =
                            RedrawWindow(Some(status), None, None, RDW_INVALIDATE | RDW_UPDATENOW);
                        let dc = GetDC(Some(status));
                        let pixel = GetPixel(dc, 1, 1);
                        let response = SendMessageW(
                            self.hwnd,
                            WM_CTLCOLORSTATIC,
                            Some(WPARAM(dc.0 as usize)),
                            Some(LPARAM(status.0 as isize)),
                        );
                        let background = GetBkColor(dc);
                        let ink = GetTextColor(dc);
                        ReleaseDC(Some(status), dc);
                        if response.0 != self.background.0 as isize
                            || pixel.0 != palette.background_rgb()
                            || background.0 != palette.background_rgb()
                            || ink.0 != palette.ink_rgb()
                        {
                            return Err(format!("Theme {} STATIC repaint/color regression: pixel {:06x}, background {:06x}, ink {:06x}", self.prefs.theme, pixel.0, background.0, ink.0).into());
                        }
                        self.capture(&out.join(format!("theme-{}-status.png", self.prefs.theme)))?;
                        if self.qa_step < 4 {
                            self.theme(self.qa_step);
                        } else {
                            let scale = self.dpi / 96.;
                            SetWindowPos(
                                self.hwnd,
                                None,
                                0,
                                0,
                                (800. * scale) as i32,
                                (600. * scale) as i32,
                                SWP_NOMOVE | SWP_NOZORDER,
                            )?;
                        }
                    }
                    5 => self.qa_restore_window()?,
                    6 => {
                        let mut actual = RECT::default();
                        GetWindowRect(self.hwnd, &mut actual)?;
                        let monitor = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
                        let mut info = MONITORINFO {
                            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                            ..Default::default()
                        };
                        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                            return Err(windows::core::Error::from_thread().into());
                        }
                        let expected = fit_window_rect(self.qa_original_rect, info.rcWork);
                        if actual != expected {
                            return Err("QA did not restore the original physical window within the work area".into());
                        }
                        self.capture(&out.join("restored-window.png"))?;
                        std::fs::write(
                            out.join("control-colors-verification.json"),
                            serde_json::to_vec_pretty(
                                &serde_json::json!({"passed":true,"themes":4,
                                "guardedNativeColorMessages":true,"actualNoticePixelsMatchPalette":true,
                                "restoresOriginalPhysicalWindow":true,"dpi":self.dpi}),
                            )?,
                        )?;
                        PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(error) = result {
                std::fs::write(out.join("control-colors-error.txt"), error.to_string())?;
                PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                return Ok(());
            }
            self.qa_step += 1;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_live_motion(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            std::fs::create_dir_all(out)?;
            std::fs::write(
                out.join("live-motion-step.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"step":self.qa_step,"completedRuns":self.motion_live_runs.len()}),
                )?,
            )?;
            let result = (|| -> Result<()> {
                if let Some((_, samples)) = self.motion_trace.take() {
                    let intervals: Vec<_> = samples
                        .windows(2)
                        .map(|pair| pair[1][0] - pair[0][0])
                        .collect();
                    // Scope choices are now deliberately immediate. Keep the
                    // strict multi-frame assertion for every animated action.
                    let instant_scope = self.qa_step == 10;
                    if instant_scope
                        && self.popup.as_ref().is_none_or(|p| {
                            !p.context || !p.listening_expanded || p.motion.is_some()
                        })
                    {
                        return Err("Immediate listening choices retained an animation".into());
                    }
                    if samples.len() < if instant_scope { 1 } else { 3 }
                        || intervals.iter().any(|x| !x.is_finite() || *x <= 0.)
                    {
                        std::fs::write(
                            out.join("live-motion-incomplete.json"),
                            serde_json::to_vec_pretty(
                                &serde_json::json!({"step":self.qa_step,"frames":samples,"previousRuns":self.motion_live_runs,"cacheFrames":self.motion_cache_frames}),
                            )?,
                        )?;
                        return Err("Insufficient real message-loop animation frames".into());
                    }
                    self.motion_live_runs.push(serde_json::json!({"kind":match self.qa_step { 2=>"panel-open",3=>"panel-close",4=>"page-forward",5=>"page-back",6=>"title-language",7=>"queued-pages",8=>"search-open",9=>"settings-open",10=>"scope-expand",11=>"panel-reversal",12=>"theme-light",13=>"theme-dark",_=>"unknown"},"instant":instant_scope,"frames":samples,"intervalsMs":intervals}));
                }
                match self.qa_step {
                    0 => {
                        GetWindowRect(self.hwnd, &mut self.qa_original_rect)?;
                        self.popup = None;
                        self.search = false;
                        self.panel = Panel::None;
                        self.panel_animation = None;
                        self.page_turn = None;
                        self.theme_start = None;
                        self.renderer.contextual =
                            !std::env::args().any(|arg| arg == "--qa-live-natural");
                        self.prefs.page = 45;
                        self.selected = None;
                        self.notice_until = None;
                        SetWindowPos(self.hwnd, Some(HWND_TOP), 0, 0, 1920, 1000, SWP_SHOWWINDOW)?;
                        self.resize();
                        self.invalidate();
                    }
                    1 => {
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.command(LISTEN, 0)?;
                    }
                    2 => {
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.command(CLOSE, 0)?;
                    }
                    3 => {
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.navigate(1)?;
                    }
                    4 => {
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.navigate(-1)?;
                    }
                    5 => {
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.command(LANG, 0)?;
                    }
                    6 => {
                        self.prefs.page = 45;
                        self.selected = None;
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.request_turn(1, crate::app_core::TurnSource::Discrete)?;
                        self.request_turn(1, crate::app_core::TurnSource::Discrete)?;
                    }
                    7 => {
                        if self.prefs.page != 49 || self.page_turn.is_some() {
                            return Err(
                                "Timed queued page turns did not finish both requests".into()
                            );
                        }
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.command(SEARCH, 0)?;
                    }
                    8 => {
                        self.command(SEARCH, 0)?;
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.command(SETTINGS, 0)?;
                    }
                    9 => {
                        self.popup_command(MENU_CLOSE)?;
                        let key = self
                            .renderer
                            .visible
                            .iter()
                            .find_map(|line| line.runs.first().map(|run| (run.surah, run.ayah)))
                            .ok_or("Missing original ayah for live scope motion")?;
                        self.context_at(key, self.width / 2., 150., false)?;
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.popup_command(MENU_ROW + 2)?;
                    }
                    10 => {
                        self.popup_command(MENU_CLOSE)?;
                        self.command(LISTEN, 0)?;
                        self.panel_animation = None;
                        self.resize();
                        self.command(CLOSE, 0)?;
                        if let Some((start, _, _)) = &mut self.panel_animation {
                            *start = Instant::now() - std::time::Duration::from_millis(100);
                        }
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.command(LISTEN, 0)?;
                        if client_animation_enabled()
                            && self
                                .panel_animation
                                .is_none_or(|(_, _, from)| !(0. ..382.).contains(&from))
                        {
                            return Err(
                                "Rapid Listen reversal lost the sampled panel position".into()
                            );
                        }
                    }
                    11 => {
                        self.panel = Panel::None;
                        self.panel_animation = None;
                        self.resize();
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.theme(1);
                    }
                    12 => {
                        self.motion_trace = Some((Instant::now(), vec![]));
                        self.theme(0);
                    }
                    13 => {
                        std::fs::write(
                            out.join("live-motion-verification.json"),
                            serde_json::to_vec_pretty(
                                &serde_json::json!({"passed":true,"contextual":self.renderer.contextual,"runs":self.motion_live_runs,"frameColumns":["paintStartMs","paintMs","freshLayoutMs","bookMs","nativeControlsMs","endDrawMs","freshPositionCommitMs"],"cachedFrames":self.motion_cache_frames,"preparationMs":self.motion_cache_preparations,"cpuRasterCount":self.motion_cpu.len(),"cpuRasterBytes":self.motion_cpu.pixel_bytes(),"maximumMotionPixelBytes":self.maximum_motion_pixel_bytes,"pixelBudgetBytes":crate::motion_policy::MAX_TOTAL_PIXEL_BYTES,"cacheErrors":self.motion_errors,"scope":"Real WM_TIMER/message-loop paint-start intervals and complete paint/layout work. No capture waits. The panel reversal explicitly backdates its closing start to exercise a mid-transition command; all ensuing frames are real. These are application frame intervals, not compositor presentation FPS or input-to-photon latency."}),
                            )?,
                        )?;
                        self.qa_restore_window()?;
                        PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(error) = result {
                std::fs::write(out.join("native-ui-error.txt"), error.to_string())?;
                PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
            }
            self.qa_step += 1;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_large_handoff(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let result = (|| -> Result<bool> {
                if self.large_handoff.is_none() {
                    self.large_handoff = Some(crate::large_handoff_qa::State::new(out)?);
                    self.prefs.page = 49;
                    self.prefs.speed = 2.;
                    self.prefs.volume = 0.;
                    self.prefs.repeat_plays = 1;
                    self.prefs.continuous_audio = true;
                    SendMessageW(self.controls[&RECITER], CB_SETCURSEL, Some(WPARAM(0)), None);
                    self.start_context_audio(49, (2, 286), crate::app_core::ListeningScope::Quran)?;
                }
                let audio = self
                    .audio
                    .as_ref()
                    .ok_or("Large recording playback disappeared")?;
                self.large_handoff.as_mut().unwrap().observe(audio)
            })();
            match result {
                Ok(true) => {
                    self.paint()?;
                    self.capture(&out.join("large-handoff-reader.png"))?;
                    self.command(QUICK_STOP, 0)?;
                    KillTimer(Some(self.hwnd), 3).ok();
                    PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                }
                Ok(false) => {}
                Err(error) => {
                    std::fs::write(out.join("native-ui-error.txt"), error.to_string())?;
                    self.audio = None;
                    KillTimer(Some(self.hwnd), 3).ok();
                    PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                }
            }
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_tick(&mut self) -> Result<()> {
        unsafe {
            let Some(out) = qa_argument("--qa-ui") else {
                return Ok(());
            };
            if !self.first_paint {
                return Ok(());
            }
            if std::env::args().any(|arg| arg == "--qa-large-handoff") {
                return self.qa_large_handoff(&out);
            }
            if std::env::args().any(|arg| arg == "--qa-live-motion") {
                return self.qa_live_motion(&out);
            }
            if std::env::args().any(|arg| arg == "--qa-scoped-only") {
                KillTimer(Some(self.hwnd), 3).ok();
                let result = self.qa_scoped_recordings(&out);
                if let Err(error) = result {
                    std::fs::write(out.join("native-ui-error.txt"), error.to_string())?;
                }
                PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).ok();
                return Ok(());
            }
            if std::env::args().any(|arg| arg == "--qa-fields-only") {
                KillTimer(Some(self.hwnd), 3).ok();
                std::fs::create_dir_all(&out)?;
                if let Err(error) = self.qa_enhancements29(&out) {
                    std::fs::write(out.join("native-ui-error.txt"), error.to_string())?;
                }
                PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).ok();
                return Ok(());
            }
            std::fs::create_dir_all(&out)?;
            std::fs::write(out.join("current-ui-step.txt"), self.qa_step.to_string())?;
            let result = (|| -> Result<()> {
                match self.qa_step {
                    0 => {
                        GetWindowRect(self.hwnd, &mut self.qa_original_rect)?;
                        let mut client = RECT::default();
                        GetClientRect(self.hwnd, &mut client)?;
                        let mut origin = POINT::default();
                        let _ = ClientToScreen(self.hwnd, &mut origin);
                        let w = self.qa_original_rect;
                        if client.right != w.right - w.left
                            || client.bottom != w.bottom - w.top
                            || origin.x != w.left
                            || origin.y != w.top
                        {
                            return Err(
                                "Initial custom caption still reserves a nested Windows frame"
                                    .into(),
                            );
                        }
                        std::fs::write(
                            out.join("startup-frame.json"),
                            serde_json::to_vec_pretty(
                                &serde_json::json!({"initialClientEqualsWindow":true,"beforeAnyResize":true,"originMatches":true}),
                            )?,
                        )?;
                        self.prefs.page = 45;
                        self.resize();
                        self.command(SEARCH, 0)?;
                        self.set_text(QUERY, "");
                        for unit in "٢٥٥".encode_utf16() {
                            PostMessageW(
                                Some(self.controls[&QUERY]),
                                WM_CHAR,
                                WPARAM(unit as usize),
                                LPARAM(0),
                            )?;
                        }
                    }
                    1 => {
                        if self.prefs.page != 45
                            || self.hits.len() != 1
                            || self.hits[0].page_number != 255
                            || self.hits[0].surah != 0
                        {
                            return Err(
                                "Typing did not filter a page preview without navigating".into()
                            );
                        }
                        self.capture(&out.join("native-live-page-search.png"))?;
                        std::fs::write(
                            out.join("live-search-verification.json"),
                            serde_json::to_vec_pretty(
                                &serde_json::json!({"queuedNativeTyping":true,"debouncedResultsWithoutEnter":true,"pagePreviewDoesNotNavigate":true}),
                            )?,
                        )?;
                        self.search = false;
                        self.resize();
                        self.capture(&out.join("native-reader.png"))?;
                        std::fs::write(out.join("current-ui-action.txt"), "open search")?;
                        self.command(SEARCH, 0)?;
                        std::fs::write(out.join("current-ui-action.txt"), "set numeric query")?;
                        self.set_text(QUERY, "٢٥٥");
                        std::fs::write(out.join("current-ui-action.txt"), "numeric find")?;
                        self.find()?;
                        if self.prefs.page != 255 {
                            return Err("Arabic numeric page navigation failed".into());
                        }
                        self.command(SEARCH, 0)?;
                        std::fs::write(out.join("current-ui-action.txt"), "ayah query")?;
                        self.set_text(QUERY, "46:21");
                        self.find()?;
                        std::fs::write(out.join("current-ui-action.txt"), "choose ayah")?;
                        self.choose_hit()?;
                        if self.selected != Some((46, 21)) {
                            return Err("Ayah search failed".into());
                        }
                        self.source = 1;
                        std::fs::write(out.join("current-ui-action.txt"), "open study")?;
                        self.command(STUDY, 0)?;
                        std::fs::write(out.join("current-ui-action.txt"), "study completed")?;
                    }
                    2 => {
                        self.capture(&out.join("native-tafsir.png"))?;
                        self.navigate(1)?;
                        if self.selected != Some((46, 22)) {
                            return Err("Tafsir next ayah failed".into());
                        }
                        self.command(CLOSE, 0)?;
                        self.command(LANG, 0)?;
                        self.command(LISTEN, 0)?;
                    }
                    3 => {
                        self.capture(&out.join("native-listen.png"))?;
                        self.command(CLOSE, 0)?;
                        self.command(THEME + 1, 0)?;
                    }
                    4 => {
                        self.capture(&out.join("native-light.png"))?;
                        self.command(THEME, 0)?;
                        self.command(CONTEXTUAL, 0)?;
                        self.prefs.page = 45;
                        self.resize();
                    }
                    5 => {
                        self.capture(&out.join("native-contextual.png"))?;
                        self.command(FULLSCREEN, 0)?;
                    }
                    6 => {
                        if !self.fullscreen {
                            return Err("Full-screen entry failed".into());
                        }
                        self.capture(&out.join("native-fullscreen.png"))?;
                        self.command(FULLSCREEN, 0)?;
                    }
                    7 => {
                        if self.fullscreen {
                            return Err("Full-screen exit failed".into());
                        }
                        self.navigate(1)?;
                        if self.prefs.page != 47 {
                            return Err("Page navigation failed".into());
                        }
                        self.command(SEARCH, 0)?;
                        PostMessageW(
                            Some(self.controls[&QUERY]),
                            WM_KEYDOWN,
                            WPARAM(27),
                            LPARAM(0),
                        )?;
                        PostMessageW(
                            Some(self.controls[&QUERY]),
                            WM_KEYDOWN,
                            WPARAM(37),
                            LPARAM(0),
                        )?;
                    }
                    8 => {
                        if self.search || self.prefs.page != 49 || GetFocus() != self.hwnd {
                            return Err("Escape/focus/queued arrow key navigation failed".into());
                        }
                        self.command(LISTEN, 0)?;
                        SendMessageW(
                            self.controls[&CHAPTER],
                            CB_SETCURSEL,
                            Some(WPARAM(35)),
                            None,
                        );
                        SendMessageW(self.controls[&RECITER], CB_SETCURSEL, Some(WPARAM(5)), None);
                        self.set_text(FIRST, "3");
                        self.set_text(LAST, "20");
                        self.command(LANG, 0)?;
                        if self.select(CHAPTER) != 35
                            || self.select(RECITER) != 5
                            || self.text(FIRST) != "3"
                            || self.text(LAST) != "20"
                        {
                            return Err(
                                "Language change reset the in-progress listening form".into()
                            );
                        }
                        self.command(LANG, 0)?;
                        if self.text(RECITER_LABEL) != "Reciter"
                            || GetWindowLongPtrW(self.controls[&RECITER_LABEL], GWL_STYLE) as u32
                                & WS_TABSTOP.0
                                != 0
                        {
                            return Err("Native field-label localization/tab order failed".into());
                        }
                        focus_window(self.controls[&VOLUME])?;
                        PostMessageW(
                            Some(self.controls[&VOLUME]),
                            WM_KEYDOWN,
                            WPARAM(37),
                            LPARAM(0),
                        )?;
                    }
                    9 => {
                        if self.prefs.page != 49 {
                            return Err(
                                "A volume-control arrow incorrectly changed the page".into()
                            );
                        }
                        let scale = self.dpi / 96.;
                        SetWindowPos(
                            self.hwnd,
                            None,
                            0,
                            0,
                            (800. * scale) as i32,
                            (600. * scale) as i32,
                            SWP_NOMOVE | SWP_NOZORDER,
                        )?;
                        self.resize();
                    }
                    10 => {
                        self.capture(&out.join("native-compact-listen.png"))?;
                        let mut volume = RECT::default();
                        let mut status = RECT::default();
                        GetWindowRect(self.controls[&VOLUME], &mut volume)?;
                        GetWindowRect(self.controls[&STATUS], &mut status)?;
                        if volume.bottom >= status.top {
                            return Err("Compact listening controls overlap the status line".into());
                        }
                        self.set_text(FIRST, "1");
                        self.set_text(LAST, "0");
                        focus_window(self.controls[&PLAY])?;
                        PostMessageW(
                            Some(self.controls[&PLAY]),
                            WM_KEYDOWN,
                            WPARAM(13),
                            LPARAM(0),
                        )?;
                    }
                    11 => {
                        if !IsWindowVisible(self.hwnd).as_bool() || self.notice_until.is_none() {
                            return Err("Invalid keyboard action did not leave the app running with visible feedback".into());
                        }
                        self.capture(&out.join("native-invalid-range-feedback.png"))?;
                        self.command(CLOSE, 0)?;
                        self.qa_restore_window()?;
                        self.command(THEME + 1, 0)?;
                    }
                    12 => {
                        self.capture(&out.join("native-theme-controls.png"))?;
                        focus_window(self.hwnd)?;
                        if SetTimer(Some(self.hwnd), 5, 200, Some(qa_cancel_menu)) == 0 {
                            return Err("Unable to start QA menu cancellation timer".into());
                        }
                        PostMessageW(Some(self.hwnd), WM_KEYDOWN, WPARAM(93), LPARAM(0))?;
                    }
                    13 => {
                        if self.qa_context.is_none() || self.qa_context != self.selected {
                            return Err(
                                "Keyboard context menu did not target the selected/current ayah"
                                    .into(),
                            );
                        }
                        PostMessageW(Some(self.hwnd), WM_KEYDOWN, WPARAM(122), LPARAM(0))?;
                        PostMessageW(Some(self.hwnd), WM_KEYDOWN, WPARAM(27), LPARAM(0))?;
                    }
                    14 => {
                        if self.fullscreen
                            || GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32 & WS_CLIPCHILDREN.0
                                == 0
                        {
                            return Err("Queued full-screen keys failed to restore the original clipping style".into());
                        }
                        self.command(BOOKMARK, 0)?;
                        let (saved, recovered) =
                            ReaderState::load(&self.profile, &self.resources.surahs)?;
                        if recovered
                            || !saved
                                .bookmarks
                                .iter()
                                .any(|entry| entry.page == self.prefs.page)
                        {
                            return Err(
                                "Bookmark did not survive an atomic profile save/reload".into()
                            );
                        }
                        self.command(ENABLE_TEXT_VIEW, 0)?;
                    }
                    15 => {
                        let mut expected = Vec::new();
                        for page in self.visible_pages() {
                            expected.push(self.resources.page_text(page)?);
                        }
                        if self.text(TEXT_VIEW) != expected.join("\r\n\r\n")
                            || !IsWindowVisible(self.controls[&TEXT_VIEW]).as_bool()
                            || GetWindowLongPtrW(self.controls[&TEXT_VIEW], GWL_STYLE) as u32
                                & ES_READONLY as u32
                                == 0
                        {
                            return Err("Accessible view differs from the original source or is not read-only".into());
                        }
                        self.capture(&out.join("native-accessible-text.png"))?;
                        self.command(EXIT_TEXT_VIEW, 0)?;
                        // An unsuccessful start can leave the lazily created
                        // player idle. It must not hide daily reading or retain
                        // the 100 ms playback timer after its session stops.
                        self.audio = Some(Audio::new()?);
                        self.audio.as_mut().unwrap().stop()?;
                        self.set_poll_interval(100);
                        self.command(DAILY, 0)?;
                        self.notice_until = None;
                        self.tick()?;
                        if self.audio.is_none()
                            || self.audio.as_ref().is_some_and(Audio::has_session)
                            || self.poll_interval != 1000
                            || !self
                                .text(STATUS)
                                .starts_with(self.tr("Daily reading", "الورد اليومي"))
                        {
                            return Err("Idle retained audio hides daily reading or keeps polling at playback frequency".into());
                        }
                        self.navigate(1)?;
                    }
                    16 => {
                        // Run maintenance without depending on the relative
                        // timing of the 900 ms QA and 1 s idle timers.
                        self.tick()?;
                        let (day, _, _) = local_day();
                        if self.reader.goal.day != day
                            || self.reader.goal.pages.is_empty()
                            || self.reader.session_seconds().is_none()
                        {
                            return Err("Daily reading did not record the departed pages".into());
                        }
                        self.command(DAILY, 0)?;
                        if self.reader.session_seconds().is_some() {
                            return Err("Finish reading did not stop the session".into());
                        }
                        self.command(DAILY, 0)?;
                        let mut expected_departed = self.reader.goal.pages.clone();
                        let first_pages = self.visible_pages();
                        expected_departed.extend(&first_pages);
                        self.navigate(1)?;
                        let second_pages = self.visible_pages();
                        if second_pages == first_pages {
                            return Err(
                                "Rapid reading navigation fixture did not change pages".into()
                            );
                        }
                        expected_departed.extend(&second_pages);
                        self.navigate(1)?;
                        if self.visible_pages() == second_pages {
                            return Err(
                                "Second rapid reading navigation did not change pages".into()
                            );
                        }
                        // Finish immediately, without a timer tick between the
                        // distinct navigations or before ending the session.
                        self.command(DAILY, 0)?;
                        expected_departed.sort_unstable();
                        expected_departed.dedup();
                        if self.reader.goal.pages != expected_departed
                            || self.reader.session_seconds().is_some()
                        {
                            return Err(
                                "Rapid navigation and finish lost or overcounted departed pages"
                                    .into(),
                            );
                        }
                        self.selected = Some((1, 1));
                        self.source = 2;
                        self.command(STUDY, 0)?;
                        if !self.text(STUDY).contains("In the name of") {
                            return Err("English study translation is missing".into());
                        }
                        self.command(CLOSE, 0)?;
                        self.command(LISTEN, 0)?;
                        let scale = self.dpi / 96.;
                        SetWindowPos(
                            self.hwnd,
                            None,
                            0,
                            0,
                            (800. * scale) as i32,
                            (400. * scale) as i32,
                            SWP_NOMOVE | SWP_NOZORDER,
                        )?;
                        self.panel_scroll = 10_000.;
                        self.resize();
                    }
                    17 => {
                        if self.panel_scroll <= 0.
                            || !IsWindowVisible(self.controls[&DOWNLOAD]).as_bool()
                            || !IsWindowVisible(self.controls[&STORAGE]).as_bool()
                        {
                            return Err(
                                "Compact listening panel cannot reveal offline download controls"
                                    .into(),
                            );
                        }
                        let mut button = RECT::default();
                        let mut status = RECT::default();
                        GetWindowRect(self.controls[&STORAGE], &mut button)?;
                        GetWindowRect(self.controls[&STATUS], &mut status)?;
                        if button.bottom >= status.top {
                            return Err("Scrolled downloads overlap the status area".into());
                        }
                        self.capture(&out.join("native-download-controls.png"))?;
                        self.command(CLOSE, 0)?;
                        self.qa_restore_window()?;
                    }
                    18 => {
                        self.command(MANAGE_TAFSIR, 0)?;
                    }
                    19 => {
                        if self.panel != Panel::Tafsirs
                            || !self.tafsir_rows().into_iter().all(|i| {
                                IsWindowVisible(self.controls[&(TAFSIR_ACTION_FIRST + i as u32)])
                                    .as_bool()
                            })
                        {
                            return Err(
                                "Optional tafsir management controls are unavailable".into()
                            );
                        }
                        let mut information = RECT::default();
                        let mut client = RECT::default();
                        GetWindowRect(self.controls[&TAFSIR_INFO], &mut information)?;
                        GetClientRect(self.hwnd, &mut client)?;
                        let mut origin = POINT {
                            x: client.right,
                            y: client.bottom,
                        };
                        let _ = ClientToScreen(self.hwnd, &mut origin);
                        if information.right > origin.x || information.bottom > origin.y {
                            return Err("Tafsir management is clipped".into());
                        }
                        self.capture(&out.join("native-tafsir-management.png"))?;
                        self.command(LANG, 0)?;
                        if !(0..tafsir_library::SOURCES.len()).all(|i| {
                            self.text(TAFSIR_ACTION_FIRST + i as u32)
                                .contains(tafsir_library::SOURCES[i].name_arabic)
                        }) {
                            return Err("Optional tafsir book labels are not localized".into());
                        }
                        self.command(LANG, 0)?;
                        self.command(CLOSE, 0)?;
                    }
                    20 => {
                        self.selected = Some((46, 21));
                        self.source = 1;
                        self.command(STUDY, 0)?;
                        if !IsWindowVisible(self.controls[&TEXT_SIZE]).as_bool() {
                            return Err("Explanation text size chooser is unavailable".into());
                        }
                        let original = self.text(STUDY);
                        self.command(STUDY, EN_UPDATE)?;
                        self.command(STUDY, EN_CHANGE)?;
                        if self.text(STUDY) != original {
                            return Err("RichEdit notifications changed tafsir".into());
                        }
                        for theme in 0..4 {
                            self.prefs.theme = theme;
                            let palette = self.palette();
                            self.replace_background(palette);
                            self.study()?;
                            let mut format = CHARFORMAT2W::default();
                            format.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
                            SendMessageW(
                                self.controls[&STUDY],
                                EM_GETCHARFORMAT,
                                Some(WPARAM(0)),
                                Some(LPARAM((&mut format as *mut CHARFORMAT2W) as isize)),
                            );
                            if format.Base.crTextColor.0 != palette.ink_rgb()
                                || (format.Base.dwEffects & CFE_AUTOCOLOR).0 != 0
                            {
                                return Err(
                                    "Tafsir lost theme text contrast after changing ayah".into()
                                );
                            }
                        }
                        self.prefs.theme = 0;
                        self.replace_background(self.palette());
                        self.command(CLOSE, 0)?;
                        self.command(LISTEN, 0)?;
                        SendMessageW(
                            self.controls[&VOLUME],
                            TBM_SETPOS,
                            Some(WPARAM(1)),
                            Some(LPARAM(37)),
                        );
                        self.command(VOLUME, 0)?;
                        if (self.prefs.volume - 0.37).abs() > 0.001 {
                            return Err("Volume slider did not apply its exact value".into());
                        }
                        self.command(CLOSE, 0)?;
                        let resize_start = Instant::now();
                        let scale = self.dpi / 96.;
                        for (w, h) in [
                            (800, 600),
                            (1600, 1000),
                            (960, 720),
                            (1280, 900),
                            (1100, 750),
                            (1500, 950),
                            (820, 620),
                            (1280, 900),
                        ] {
                            SetWindowPos(
                                self.hwnd,
                                None,
                                0,
                                0,
                                (w as f32 * scale) as i32,
                                (h as f32 * scale) as i32,
                                SWP_NOMOVE | SWP_NOZORDER,
                            )?;
                            self.resize();
                            self.paint()?;
                            let mut client = RECT::default();
                            GetClientRect(self.hwnd, &mut client)?;
                            let pixels = self
                                .renderer
                                .target
                                .as_ref()
                                .ok_or("Missing resize surface")?
                                .GetPixelSize();
                            if pixels.width != client.right as u32
                                || pixels.height != client.bottom as u32
                            {
                                return Err(
                                    "Resize surface differs from physical client size".into()
                                );
                            }
                            for id in [SEARCH, LISTEN, SETTINGS, MINIMIZE, MAXIMIZE, WINDOW_CLOSE] {
                                let rect = self.placements.borrow()[&id].0;
                                if rect.y < 0. || rect.y + rect.h > 36. {
                                    return Err("Title controls leave the title bar".into());
                                }
                            }
                        }
                        std::fs::write(
                            out.join("resize-verification.json"),
                            serde_json::to_vec_pretty(
                                &serde_json::json!({"physicalTargetsMatch":true,"resizes":8,
                            "milliseconds":resize_start.elapsed().as_secs_f64()*1000.,
                            "titleControlsInCaption":true,"volumeSliderExact":true,
                            "tafsirInkMatchesAllThemes":true,"richEditNotificationsIgnored":true}),
                            )?,
                        )?;
                        self.qa_restore_window()?;
                        self.qa_polish(&out).map_err(|e| format!("polish: {e}"))?;
                        self.qa_reading_polish(&out)
                            .map_err(|e| format!("reading: {e}"))?;
                        self.qa_frame_costs(&out)
                            .map_err(|e| format!("frames: {e}"))?;
                        let mut memory = windows::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS_EX::default();
                        memory.cb = std::mem::size_of_val(&memory) as u32;
                        windows::Win32::System::ProcessStatus::GetProcessMemoryInfo(
                            windows::Win32::System::Threading::GetCurrentProcess(),
                            (&mut memory as *mut windows::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS_EX).cast(),
                            memory.cb,
                        )?;
                        std::fs::write(
                            out.join("native-ui-verification.json"),
                            serde_json::to_vec_pretty(&serde_json::json!({
                                "firstPaintMs":self.first_paint_ms,"workingSetBytes":memory.WorkingSetSize,"privateBytes":memory.PrivateUsage,
                                "pid":std::process::id(),"webviewDependency":false,"passed":["numeric page search","ayah search","tafsir sections","tafsir navigation","native panels","language","themes","contextual outlines","full screen entry/exit","page navigation","queued Escape and page-arrow keys","volume focus does not change pages","compact listening controls","invalid keyboard action remains recoverable","queued fullscreen keys preserve window styles","language preserves in-progress listening form","native localized form labels","queued keyboard context menu","bookmarks survive save/reload","accessible view is exact original read-only text","idle retained audio preserves daily status and one-second polling","daily reading counts departed pages and finishes","rapid distinct navigations and immediate finish record departed pages","English study translation loads","compact listening panel scrolls to download controls","optional tafsir controls fit and localize","tafsir theme contrast survives text replacement","RichEdit notifications never reopen study","volume slider applies exact value","repeated resize matches physical render target","primary controls fit title bar"]
                            }))?,
                        )?;
                        PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(error) = result {
                std::fs::write(out.join("native-ui-error.txt"), error.to_string())?;
                PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                return Ok(());
            }
            self.qa_step += 1;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_polish(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            self.panel_animation = None;
            self.prefs.theme = 0;
            self.replace_background(self.palette());
            self.open_listen();
            self.command(SEARCH, 0)?;
            if self.panel != Panel::None || self.displayed_panel() != Panel::None || !self.search {
                return Err("Search overlaps listening panel".into());
            }
            if !IsWindowVisible(self.controls[&SEARCH_CLOSE]).as_bool() {
                return Err("Search close button is not visible".into());
            }
            self.command(SEARCH_CLOSE, 0)?;
            if self.search || GetFocus() != self.hwnd {
                return Err("Close search did not restore reader focus".into());
            }
            self.command(SEARCH, 0)?;
            self.open_listen();
            self.panel_animation = None;
            self.resize();
            self.paint()?;
            if self.search || IsWindowVisible(self.controls[&QUERY]).as_bool() {
                return Err("Listening left Search controls open".into());
            }
            let slider = self.controls[&VOLUME];
            let mut thumb = RECT::default();
            SendMessageW(
                slider,
                windows::Win32::UI::Controls::TBM_GETTHUMBRECT,
                None,
                Some(LPARAM((&mut thumb as *mut RECT) as isize)),
            );
            let dc = GetDC(Some(slider));
            let pixel = GetPixel(
                dc,
                (thumb.left + thumb.right) / 2,
                (thumb.top + thumb.bottom) / 2,
            );
            ReleaseDC(Some(slider), dc);
            let light = ((pixel.0 & 255) + ((pixel.0 >> 8) & 255) + ((pixel.0 >> 16) & 255)) / 3;
            if light < 180 {
                return Err("Dark volume thumb has insufficient contrast".into());
            }
            self.capture(&out.join("native-antialiased-controls-dark.png"))?;
            std::fs::write(
                out.join("control-polish-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"searchAndListenExclusiveBothDirections":true,"searchCloseVisibleAndRestoresFocus":true,"volumeThumbBrightness":light,"nativeSliderBehaviorRetained":true}),
                )?,
            )?;
            self.panel = Panel::None;
            self.resize();
            // Both NCCALCSIZE pointer variants must remain valid before maximize.
            self.qa_experience(out)
                .map_err(|e| format!("qa_experience: {e}"))?;
            self.qa_reference_navigation(out)
                .map_err(|e| format!("qa_reference_navigation: {e}"))?;
            self.qa_study_experience(out)
                .map_err(|e| format!("qa_study_experience: {e}"))?;
            self.qa_ui_listening_feedback(out)
                .map_err(|e| format!("qa_ui_listening_feedback: {e}"))?;
            self.qa_experience19(out)
                .map_err(|e| format!("qa_experience19: {e}"))?;
            self.qa_listening20(out)
                .map_err(|e| format!("qa_listening20: {e}"))?;
            self.qa_enhancements21(out)
                .map_err(|e| format!("qa_enhancements21: {e}"))?;
            self.qa_enhancements22(out)
                .map_err(|e| format!("qa_enhancements22: {e}"))?;
            self.qa_scoped_listening(out)
                .map_err(|e| format!("qa_scoped_listening: {e}"))?;
            if std::env::args().any(|argument| argument == "--qa-scoped-audio") {
                self.qa_scoped_recordings(out)?;
            }
            self.qa_hover_polish(out)
                .map_err(|e| format!("qa_hover_polish: {e}"))?;
            let mut rect = RECT {
                left: 20,
                top: 30,
                right: 820,
                bottom: 630,
            };
            let original = rect;
            SendMessageW(
                self.hwnd,
                WM_NCCALCSIZE,
                Some(WPARAM(0)),
                Some(LPARAM((&mut rect as *mut RECT) as isize)),
            );
            if rect != original {
                return Err("Initial RECT frame calculation reserved caption space".into());
            }
            let mut params = NCCALCSIZE_PARAMS {
                rgrc: [rect; 3],
                ..Default::default()
            };
            SendMessageW(
                self.hwnd,
                WM_NCCALCSIZE,
                Some(WPARAM(1)),
                Some(LPARAM((&mut params as *mut NCCALCSIZE_PARAMS) as isize)),
            );
            if params.rgrc[0] != original {
                return Err("PARAMS frame calculation reserved caption space".into());
            }
            let mut icon = ICONINFO::default();
            GetIconInfo(self.identity_icon, &mut icon)?;
            let mut bitmap = BITMAP::default();
            let size = GetObjectW(
                icon.hbmColor.into(),
                std::mem::size_of::<BITMAP>() as i32,
                Some((&mut bitmap as *mut BITMAP).cast()),
            );
            let _ = DeleteObject(icon.hbmColor.into()).ok();
            let _ = DeleteObject(icon.hbmMask.into()).ok();
            if size == 0 || bitmap.bmWidth != (18. * self.dpi / 96.).round() as i32 {
                return Err("Title icon was resampled from a stale DPI size".into());
            }
            self.theme_start = None;
            self.replace_background(self.palette());
            self.set_panel(Panel::None);
            self.panel_animation = None;
            self.search = false;
            self.notice_until = None;
            self.resize();
            let original_page = self.prefs.page;
            let bounds = crate::page_turn::bounds(self.reader_area());
            self.drag_start = Some((bounds.x + 20., bounds.y + 20.));
            SetCapture(self.hwnd);
            self.move_page_drag(bounds.x + bounds.w * 0.31, bounds.y + 45.);
            let d = self
                .page_drag
                .ok_or("Pointer motion did not start a live corner fold")?;
            if d.progress < 0.2 || !d.top {
                return Err("Pointer geometry ignored its corner/progress".into());
            }
            self.paint()?;
            let mut transform = windows_numerics::Matrix3x2::identity();
            self.renderer
                .target
                .as_ref()
                .ok_or("Missing fold target")?
                .GetTransform(&mut transform);
            if transform != windows_numerics::Matrix3x2::identity() {
                return Err("Fold transform escaped into stationary text or chrome".into());
            }
            self.capture(&out.join("native-live-corner-fold.png"))?;
            self.drag_start = None;
            self.finish_page_drag()?;
            ReleaseCapture().ok();
            if self.prefs.page != d.new
                || self
                    .page_turn
                    .is_none_or(|t| (t.from - d.progress).abs() > 0.001)
            {
                return Err(
                    "Release restarted the fold instead of settling from its pointer position"
                        .into(),
                );
            }
            self.page_turn = None;
            self.prefs.page = original_page;
            self.paint()?;
            self.drag_start = Some((bounds.x + 20., bounds.y + bounds.h - 20.));
            SetCapture(self.hwnd);
            self.move_page_drag(bounds.x + 20. + bounds.w * 0.06, bounds.y + bounds.h - 35.);
            self.drag_start = None;
            self.finish_page_drag()?;
            ReleaseCapture().ok();
            if self.prefs.page != original_page || self.page_turn.is_none_or(|t| t.to != 0.) {
                return Err("Short drag did not animate back without changing page".into());
            }
            self.page_turn = None;
            self.resize();
            self.command(SETTINGS, 0)?;
            self.paint()?;
            if self
                .popup
                .as_ref()
                .is_none_or(|p| p.rect.y + p.rect.h > self.height - 10. || p.rows.len() > 10)
            {
                return Err("Options do not fit a restored window".into());
            }
            self.capture(&out.join("native-options-menu.png"))?;
            if self.popup.as_ref().is_none_or(|p| {
                !p.rows
                    .iter()
                    .any(|r| r.children.iter().any(|child| child.id == DAILY))
            }) {
                return Err(
                    "Daily reading submenu was lost while converting native menu flags".into(),
                );
            }
            self.popup_command(MENU_ROW + 1)?;
            self.paint()?;
            if self
                .popup
                .as_ref()
                .is_none_or(|p| p.stack.len() != 1 || !p.rows.iter().any(|r| r.id == CONTEXTUAL))
            {
                return Err("Appearance settings lost their nested commands".into());
            }
            self.capture(&out.join("native-appearance-menu.png"))?;
            self.keyboard_focus = true;
            focus_window(self.controls[&THEME_SELECTOR])?;
            self.key(9)?;
            if GetFocus() != self.controls[&(MENU_ROW + 1)] {
                return Err("Appearance Tab did not move from the theme dropdown".into());
            }
            self.key(27)?;
            if self.popup.as_ref().is_none_or(|p| !p.stack.is_empty()) {
                return Err("Escape did not return to the menu parent".into());
            }
            self.key(27)?;
            if self.popup.is_some() {
                return Err("Escape did not close the root menu".into());
            }
            self.context_at((46, 21), self.width - 20., self.height - 20., false)?;
            self.paint()?;
            if self
                .popup
                .as_ref()
                .is_none_or(|p| p.rect.y + p.rect.h > self.height - 10. || p.rows.len() != 4)
            {
                return Err("Context actions leave the client surface".into());
            }
            self.capture(&out.join("native-context-menu.png"))?;
            self.popup_command(MENU_ROW + 2)?;
            if self.popup.as_ref().is_none_or(|p| {
                p.rows
                    .iter()
                    .skip(3)
                    .filter(|row| {
                        matches!(
                            row.id,
                            LISTEN_THIS | LISTEN_PAGE | LISTEN_SURAH | LISTEN_QURAN | LISTEN
                        )
                    })
                    .map(|row| row.id)
                    .collect::<Vec<_>>()
                    != [LISTEN_THIS, LISTEN_PAGE, LISTEN_SURAH, LISTEN_QURAN, LISTEN]
            }) {
                return Err("Nested listening options no longer reach the listening form".into());
            }
            self.key(9)?;
            self.key(27)?;
            self.key(27)?;
            if self.popup.is_some() {
                return Err("Keyboard context-menu dismissal failed".into());
            }
            self.command(SEARCH, 0)?;
            self.search_animation = None;
            self.set_text(QUERY, "46:21");
            self.update_search()?;
            self.paint()?;
            self.capture(&out.join("native-polished-search.png"))?;
            if self.hits.is_empty() {
                return Err("Themed result list lost ayah search results".into());
            }
            self.search = false;
            self.resize();
            if self.chrome_failures.get() != 0 {
                return Err("DirectWrite button renderer fell back to GDI".into());
            }
            self.command(SEARCH, 0)?;
            self.search_animation = None;
            self.set_text(QUERY, "baqar");
            self.update_search()?;
            if self.hits.len() != 1 || self.hits[0].surah != 2 {
                return Err("Surah-name search did not narrow the original catalogue".into());
            }
            self.paint()?;
            self.capture(&out.join("native-filtered-surah-search.png"))?;
            self.search = false;
            self.resize();
            self.prefs.continuous_audio = false;
            self.command(SETTINGS, 0)?;
            let row = self
                .popup
                .as_ref()
                .ok_or("Missing options")?
                .rows
                .iter()
                .position(|row| {
                    row.children
                        .iter()
                        .any(|child| child.id == CONTINUOUS_AUDIO)
                })
                .ok_or("Missing listening mode submenu")?;
            self.popup_command(MENU_ROW + row as u32)?;
            let child = self
                .popup
                .as_ref()
                .unwrap()
                .rows
                .iter()
                .position(|row| row.id == CONTINUOUS_AUDIO)
                .ok_or("Missing continuous mode choice")?;
            self.popup_command(MENU_ROW + child as u32)?;
            if !self.prefs.continuous_audio {
                return Err("Native listening mode command did not change its preference".into());
            }
            if self.popup.is_none() {
                return Err("Settings closed while adjusting the listening preference".into());
            }
            self.popup_command(MENU_CLOSE)?;
            self.command(SETTINGS, 0)?;
            self.popup_command(MENU_ROW + row as u32)?;
            if !self
                .popup
                .as_ref()
                .ok_or("Missing reopened listening settings")?
                .rows[child]
                .label
                .starts_with('✓')
            {
                return Err("Continuous mode check mark was lost".into());
            }
            self.paint()?;
            self.capture(&out.join("native-continuous-mode-menu.png"))?;
            self.popup = None;
            self.prefs.continuous_audio = false;
            self.resize();
            std::fs::write(
                out.join("continuous-mode-ui.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"nativeMenuCommand":true,"modeCheckMark":true,"existingCompactControlsPreserved":true}),
                )?,
            )?;
            std::fs::write(
                out.join("polish-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"initialFrameBothPointerVariants":true,"nativeIconPixelsMatchDpi":true,"liveCornerDrag":true,"foldUsesAuthenticSourceOnBothFaces":true,"foldTransformRestored":true,"dragSettlesFromCurrentProgress":true,"shortDragCancels":true,"optionsFitWindow":true,"appearanceCommandsPreserved":true,"menuArrowFocus":true,"menuEscapeBackAndClose":true,"contextFitsWindow":true,"nestedListeningCommands":true,"menuTabNavigation":true,"directWriteButtonFallbacks":self.chrome_failures.get(),"themedSearchRetainsSourceResults":true}),
                )?,
            )?;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_reference_navigation(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let page = self.prefs.page;
            let selected = self.selected;
            let panel = self.panel;
            self.command(SEARCH, 0)?;
            self.search_animation = None;
            self.update_search()?;
            if self.hits.len() != 114
                || self
                    .hits
                    .iter()
                    .any(|h| h.kind != SearchKind::Surah || !h.text.is_empty())
            {
                return Err("Surah browse exposed verse snippets".into());
            }
            let list = self.controls[&RESULTS];
            let mut first_row = RECT::default();
            if SendMessageW(list, LB_GETCURSEL, None, None).0 != -1 {
                return Err("Opening search selected a result without user intent".into());
            }
            let language = self.prefs.arabic;
            let mut wheel_cases = Vec::new();
            for arabic in [false, true] {
                self.prefs.arabic = arabic;
                for kind in 0usize..4 {
                    self.browse_tab = if kind == 0 {
                        BROWSE_SURAH
                    } else {
                        BROWSE_SECTIONS
                    };
                    SendMessageW(
                        self.controls[&BROWSE_KIND],
                        CB_SETCURSEL,
                        Some(WPARAM(kind.saturating_sub(1))),
                        None,
                    );
                    self.set_text(BROWSE_QUERY, "");
                    self.update_search()?;
                    for (delta, expected) in [(-60i16, 0isize), (-60, 1), (-120, 2), (120, 1)] {
                        SendMessageW(
                            list,
                            WM_MOUSEWHEEL,
                            Some(WPARAM((delta as u16 as usize) << 16)),
                            None,
                        );
                        if SendMessageW(list, LB_GETTOPINDEX, None, None).0 != expected
                            || SendMessageW(list, LB_GETCURSEL, None, None).0 != -1
                        {
                            return Err("Navigation wheel skipped rows or selected a result".into());
                        }
                    }
                    wheel_cases
                        .push(serde_json::json!({"arabic":arabic,"kind":kind,"tops":[0,1,2,1]}));
                }
            }
            self.prefs.arabic = language;
            self.browse_tab = BROWSE_SURAH;
            SendMessageW(
                self.controls[&BROWSE_KIND],
                CB_SETCURSEL,
                Some(WPARAM(0)),
                None,
            );
            self.update_search()?;
            std::fs::write(
                out.join("navigation-wheel-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"cases":wheel_cases,"noAutomaticSelection":true}),
                )?,
            )?;
            SendMessageW(
                list,
                LB_GETITEMRECT,
                Some(WPARAM(0)),
                Some(LPARAM((&mut first_row as *mut RECT) as isize)),
            );
            if self.hits[0].surah != 1
                || SendMessageW(list, LB_GETTOPINDEX, None, None).0 != 0
                || first_row.top < 0
            {
                return Err("The first surah is clipped above the new search viewport".into());
            }
            SendMessageW(list, LB_SETCURSEL, Some(WPARAM(12)), None);
            SendMessageW(list, LB_SETTOPINDEX, Some(WPARAM(10)), None);
            let scrolled_top = SendMessageW(list, LB_GETTOPINDEX, None, None).0;
            for n in 0..12 {
                self.search_animation = Some(
                    Instant::now() - std::time::Duration::from_secs_f32(0.18 * n as f32 / 12.),
                );
                self.resize_layout(false);
                if SendMessageW(list, LB_GETTOPINDEX, None, None).0 != scrolled_top {
                    return Err("Search movement changed the user's scrolled result row".into());
                }
            }
            self.search_animation = None;
            self.update_search()?;
            self.paint()?;
            self.capture(&out.join("native-reference-search-landing.png"))?;
            self.set_text(BROWSE_QUERY, "78");
            self.update_search()?;
            if self.hits.len() != 1 || self.hits[0].surah != 78 {
                return Err("Surah number confused with page number".into());
            }
            self.paint()?;
            self.capture(&out.join("native-reference-surah-result.png"))?;
            SendMessageW(
                self.controls[&RESULTS],
                WM_LBUTTONDOWN,
                Some(WPARAM(1)),
                Some(LPARAM(12 | (12 << 16))),
            );
            SendMessageW(
                self.controls[&RESULTS],
                WM_LBUTTONUP,
                None,
                Some(LPARAM(12 | (12 << 16))),
            );
            let mut click = MSG::default();
            if !PeekMessageW(
                &mut click,
                Some(self.hwnd),
                WM_COMMAND,
                WM_COMMAND,
                PM_REMOVE,
            )
            .as_bool()
                || click.wParam.0 != RESULT_PICK as usize
            {
                return Err("Single-click result activation was not posted".into());
            }
            self.command(RESULT_PICK, 0)?;
            if self.prefs.page != 582 || self.selected.is_some() {
                return Err("Surah navigation selected an arbitrary ayah".into());
            }
            self.command(SEARCH, 0)?;
            self.command(BROWSE_PAGE, 0)?;
            self.set_text(BROWSE_QUERY, "٢٥٥");
            self.update_search()?;
            self.find()?;
            if self.prefs.page != 255 {
                return Err("Page browse failed".into());
            }
            self.command(SEARCH, 0)?;
            self.command(BROWSE_SECTIONS, 0)?;
            self.set_text(BROWSE_QUERY, "30");
            self.update_search()?;
            if self.hits.len() != 1 || self.hits[0].page_number != 582 {
                return Err("Original juz browse failed".into());
            }
            self.paint()?;
            self.capture(&out.join("native-reference-section-result.png"))?;
            self.find()?;
            self.command(SEARCH, 0)?;
            self.set_text(QUERY, "unmatchablezzzz");
            self.update_search()?;
            if !self.hits.is_empty() || IsWindowVisible(self.controls[&BROWSE_SURAH]).as_bool() {
                return Err("Typed search continued exposing browse results".into());
            }
            self.paint()?;
            self.capture(&out.join("native-reference-empty-search.png"))?;
            self.command(SEARCH_CLOSE, 0)?;
            self.command(SEARCH, 0)?;
            self.set_text(QUERY, "3:39");
            self.find()?;
            if self.canvas_selection() != Some((3, 39)) {
                return Err("Chosen search ayah was not revealed in the Mushaf".into());
            }
            self.paint()?;
            self.capture(&out.join("native-search-ayah-reveal.png"))?;
            self.search_reveal =
                Some((Instant::now() - std::time::Duration::from_secs(6), (3, 39)));
            if self.canvas_selection() == Some((3, 39)) {
                return Err("Search reveal did not expire after five seconds".into());
            }
            self.search_reveal = None;
            std::fs::write(
                out.join("search-reveal-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"selectedAyah":[3,39],"expiresAfterSeconds":5,"usesOriginalMushafHitRegions":true}),
                )?,
            )?;
            self.panel = Panel::Study;
            self.selected = Some((3, 39));
            self.hovered = Some((3, 40));
            if self.canvas_selection() != Some((3, 39)) {
                return Err("Study ayah did not take priority over hover".into());
            }
            self.prefs.page = 55;
            self.study()?;
            self.resize();
            self.paint()?;
            self.capture(&out.join("native-studied-ayah-highlight.png"))?;
            self.panel = Panel::None;
            self.selected = None;
            self.hovered = None;
            self.resize();
            self.paint()?;
            let from = if self.prefs.arabic {
                230.
            } else {
                (self.width - 390.).max(230.)
            };
            self.title_animation = Some((Instant::now(), from));
            let title_initialization = Instant::now();
            self.invalidate();
            self.paint()?;
            let title_initialization_ms = title_initialization.elapsed().as_secs_f64() * 1000.;
            let before = self.canvas_paints;
            let mut samples = vec![];
            for n in 0..32 {
                self.title_animation = Some((
                    Instant::now() - std::time::Duration::from_secs_f32(0.22 * n as f32 / 32.),
                    from,
                ));
                let started = Instant::now();
                self.resize_layout(true);
                self.paint()?;
                samples.push(started.elapsed().as_secs_f64() * 1000.);
            }
            if self.canvas_paints != before {
                return Err(format!(
                    "Title-only frames repainted Quran pages: {}",
                    self.canvas_paints - before
                )
                .into());
            }
            samples.sort_by(f64::total_cmp);
            let frames = self
                .title_trace
                .as_ref()
                .map(|(_, f)| f.clone())
                .unwrap_or_default();
            self.title_animation = None;
            self.panel = Panel::None;
            self.opening_highlight = None;
            self.playback.begin();
            self.follow_recitation(Some((4, 12)))?;
            self.resize();
            self.paint()?;
            if self.canvas_selection() != Some((4, 12)) {
                return Err("Playback ayah not highlighted".into());
            }
            self.capture(&out.join("native-recitation-ayah-highlight.png"))?;
            self.page_turn = Some(crate::page_turn::Turn {
                first_frame: false,
                start: Instant::now(),
                old: 79,
                new: 81,
                direction: 1,
                from: 0.,
                to: 1.,
                top: false,
            });
            self.command(SEARCH, 0)?;
            self.set_text(QUERY, "naba");
            self.update_search()?;
            self.find()?;
            if self.prefs.page != 582 || self.page_turn.is_some() || self.page_drag.is_some() {
                return Err("Search jump retained an old animation".into());
            }
            self.follow_recitation(Some((4, 12)))?;
            self.follow_recitation(Some((4, 13)))?;
            if self.prefs.page != 582 || self.playback.following {
                return Err("Recitation pulled search navigation back".into());
            }
            self.opening_highlight = None;
            self.playback.begin();
            self.follow_recitation(Some((4, 13)))?;
            if self.prefs.page != 79 {
                return Err("Rejoining recitation did not return to the current ayah".into());
            }
            self.playback = crate::app_core::PlaybackFollow::default();
            // This optional fixture uses an independently copied, hash-verified
            // original recording cache. Ordinary offline QA remains offline.
            if self
                .profile
                .join("AudioStore/Continuous/r92-s112/receipt.json")
                .exists()
            {
                self.audio = None;
                let continuous = self.prefs.continuous_audio;
                let volume = self.prefs.volume;
                self.prefs.continuous_audio = true;
                self.prefs.volume = 0.;
                SendMessageW(self.controls[&RECITER], CB_SETCURSEL, Some(WPARAM(0)), None);
                SendMessageW(
                    self.controls[&CHAPTER],
                    CB_SETCURSEL,
                    Some(WPARAM(111)),
                    None,
                );
                self.set_text(FIRST, "1");
                self.set_text(LAST, "4");
                self.command(PLAY, 0)?;
                let started = Instant::now();
                while self.playback.current.is_none() && started.elapsed().as_secs() < 15 {
                    self.tick()?;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                if self.playback.current != Some((112, 1)) {
                    return Err("Original recording did not report its first ayah".into());
                }
                self.notice_until = None;
                self.set_panel(Panel::None);
                self.panel_animation = None;
                self.resize();
                self.paint()?;
                let book = crate::app_core::book_geometry(self.reader_area()).bounds;
                if let Some(frame) = self.compact_player_frame() {
                    if book.x + book.w > frame.x {
                        return Err("Side player overlaps Quran pages".into());
                    }
                } else if self.placements.borrow()[&QUICK_PAUSE].0.y > 8. {
                    return Err("Narrow side margin did not use the title-bar player".into());
                }
                self.capture(&out.join("native-live-side-player.png"))?;
                self.command(QUICK_PAUSE, 0)?;
                std::thread::sleep(std::time::Duration::from_millis(100));
                if !self
                    .audio
                    .as_ref()
                    .unwrap()
                    .status(false)
                    .contains("Paused")
                {
                    return Err("Quick pause did not pause the decoder".into());
                }
                self.command(SEARCH, 0)?;
                self.set_text(QUERY, "naba");
                self.update_search()?;
                self.find()?;
                self.tick()?;
                if self.prefs.page != 582 {
                    return Err("Live paused recitation overrode Surah navigation".into());
                }
                self.command(FOLLOW_AUDIO, 0)?;
                if self.prefs.page != 604 {
                    return Err("Live recitation rejoin failed".into());
                }
                self.command(QUICK_PAUSE, 0)?;
                std::thread::sleep(std::time::Duration::from_millis(100));
                if !self
                    .audio
                    .as_ref()
                    .unwrap()
                    .status(false)
                    .contains("Playing")
                {
                    return Err("Quick resume did not resume the decoder".into());
                }
                self.set_panel(Panel::Listen);
                self.panel_animation = None;
                self.resize();
                self.paint()?;
                self.capture(&out.join("native-live-listening-panel.png"))?;
                self.command(QUICK_STOP, 0)?;
                if self.audio.is_some() || self.playback.current.is_some() {
                    return Err("Quick stop retained the audio highlight".into());
                }
                self.panel = Panel::None;
                let repeat_plays = self.prefs.repeat_plays;
                let speed = self.prefs.speed;
                let mut repeat_cases = vec![];
                for timed in [true, false] {
                    self.prefs.continuous_audio = timed;
                    SendMessageW(self.controls[&REPEAT], CB_SETCURSEL, Some(WPARAM(1)), None);
                    self.command(REPEAT, CBN_SELCHANGE)?;
                    self.prefs.speed = 2.;
                    self.set_text(LAST, "1");
                    self.command(PLAY, 0)?;
                    // Editing the form must not retarget a pending repeat.
                    self.set_text(FIRST, "2");
                    self.set_text(LAST, "3");
                    let began = Instant::now();
                    let mut second = false;
                    let mut terminal = false;
                    while began.elapsed().as_secs() < 25 {
                        crate::audio::pump_messages();
                        self.tick()?;
                        second |= self.repeats.remaining == 1;
                        if second && self.audio.as_ref().is_some_and(Audio::completed) {
                            terminal = true;
                            break;
                        }
                        if self.playback.current.is_some_and(|key| key != (112, 1)) {
                            return Err("Edited form retargeted the repeated range".into());
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    if !terminal {
                        return Err(format!("Finite original-recording repetition did not complete: continuous={timed}, remaining={}, status={}", self.repeats.remaining, self.audio.as_ref().unwrap().status(false)).into());
                    }
                    for _ in 0..10 {
                        self.tick()?;
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    if !self.audio.as_ref().is_some_and(Audio::completed) {
                        return Err("Finite repetition restarted after its total count".into());
                    }
                    repeat_cases.push(serde_json::json!({"continuous":timed,"totalPlays":2,"seconds":began.elapsed().as_secs_f64(),"unchangedRangeDespiteEditedForm":true,"completed":true}));
                    self.command(QUICK_STOP, 0)?;
                    self.set_text(FIRST, "1");
                }
                self.prefs.repeat_plays = repeat_plays;
                self.prefs.speed = speed;
                self.prefs.continuous_audio = continuous;
                self.prefs.volume = volume;
                std::fs::write(
                    out.join("finite-repeat-verification.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"passed":true,"cases":repeat_cases,"scope":"Actual native production controller/decoder, copied original Al-Ikhlas ayah 1 and continuous recording; total plays includes first playback; repetitions restart the validated range and are not gapless"}),
                    )?,
                )?;
                std::fs::write(
                    out.join("live-listening-verification.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"passed":true,"originalRecording":true,"decoderAyahHighlighted":true,"quickPauseResume":true,"pausedSearchDoesNotJumpBack":true,"explicitFollowReturnsToRecording":true,"stopClearsHighlight":true,"muted":true,"scope":"Production decoder/worker and native UI, original Yasser Al-Ikhlas recording; short integration session"}),
                    )?,
                )?;
            }
            let optional = self.prefs.search_key_t;
            self.prefs.search_key_t = true;
            focus_window(self.hwnd)?;
            if !self.key(84)? || !self.search {
                return Err("Optional search shortcut failed".into());
            }
            if self.key(84)? {
                return Err("Optional shortcut intercepted typing".into());
            }
            self.command(SEARCH_CLOSE, 0)?;
            self.prefs.search_key_t = optional;
            self.prefs.page = 79;
            self.toggle_fullscreen()?;
            self.paint()?;
            if self.reader_area().y != 6. || self.placements.borrow()[&SEARCH].0.y != 56. {
                return Err("Fullscreen did not move navigation to the side".into());
            }
            self.capture(&out.join("native-fullscreen-side-navigation.png"))?;
            self.toggle_fullscreen()?;
            let mut warm = vec![];
            for contextual in [false, true] {
                self.renderer.contextual = contextual;
                self.prewarm_key = None;
                self.prewarm_pages.clear();
                self.panel_animation = None;
                self.schedule_prewarm();
                let pages: Vec<_> = self.prewarm_pages.drain(..).collect();
                if pages.len() != 8 || !pages.contains(&75) || !pages.contains(&84) {
                    return Err("Four-page neighbor lookahead failed".into());
                }
                let mut before =
                    windows::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS_EX::default();
                before.cb = std::mem::size_of_val(&before) as u32;
                windows::Win32::System::ProcessStatus::GetProcessMemoryInfo(
                    windows::Win32::System::Threading::GetCurrentProcess(),
                    (&mut before
                        as *mut windows::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS_EX)
                        .cast(),
                    before.cb,
                )?;
                let began = Instant::now();
                for number in &pages {
                    self.renderer
                        .prepare_page(&mut self.resources, *number, false)?;
                }
                let mut after = before;
                windows::Win32::System::ProcessStatus::GetProcessMemoryInfo(
                    windows::Win32::System::Threading::GetCurrentProcess(),
                    (&mut after
                        as *mut windows::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS_EX)
                        .cast(),
                    after.cb,
                )?;
                warm.push(serde_json::json!({"contextual":contextual,"pages":pages,"milliseconds":began.elapsed().as_secs_f64()*1000.,"workingSetBefore":before.WorkingSetSize,"workingSetAfter":after.WorkingSetSize,"privateBefore":before.PrivateUsage,"privateAfter":after.PrivateUsage}));
            }
            self.renderer.contextual = self.prefs.contextual;
            std::fs::write(
                out.join("listening-navigation-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"playingAyahHighlighted":true,"manualSearchSurvivesSameAndSubsequentAyahs":true,"searchCancelsPriorFold":true,"explicitRejoin":true,"optionalShortcutDoesNotInterceptTyping":true,"fullscreenSideNavigation":true,"neighborPreparation":warm,"scope":"Actual native controls/rendering and production playback-observation policy; supplied ayah events, not a live decoder-to-UI session"}),
                )?,
            )?;
            self.panel = panel;
            self.prefs.page = page;
            self.selected = selected;
            self.resize();
            std::fs::write(
                out.join("reference-navigation-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"surahBrowseHasNoVerseExcerpts":true,"firstSurahVisible":true,"motionPreservesResultScroll":true,"surahNumberDistinctFromPage":true,"pageBrowseArabicDigits":true,"juzBrowseOriginalStarts":true,"typedSearchHidesBrowse":true,"studyAyahHighlighted":true,"titleFramesDoNotRepaintQuran":true,"titleInitializationMs":title_initialization_ms,"titleLayoutPaintP95Ms":samples[30],"titleLayoutPaintSamplesMs":samples,"observedCaptionPaints":frames,"scope":"Steady title layout/paint costs after separately measured first texture preparation; not compositor FPS or input-to-photon latency"}),
                )?,
            )?;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_experience(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let page = self.prefs.page;
            let selected = self.selected;
            let size = self.study_size;
            self.command(SEARCH, 0)?;
            self.set_text(QUERY, "baqar");
            self.update_search()?;
            focus_window(self.controls[&QUERY])?;
            SendMessageW(self.controls[&QUERY], WM_IME_STARTCOMPOSITION, None, None);
            if self.key(13)? || !self.search {
                return Err("Search intercepted IME composition Enter".into());
            }
            SendMessageW(self.controls[&QUERY], WM_IME_ENDCOMPOSITION, None, None);
            self.key(13)?;
            if self.search || self.prefs.page != 2 || self.selected.is_some() {
                return Err("Unique search Enter did not navigate".into());
            }
            self.command(SEARCH, 0)?;
            self.set_text(QUERY, "الله");
            self.update_search()?;
            if self.hits.len() < 2 {
                return Err("Missing multi-result search fixture".into());
            }
            let last = self.hits.last().cloned().unwrap();
            focus_window(self.controls[&QUERY])?;
            self.key(40)?;
            if GetFocus() != self.controls[&RESULTS]
                || SendMessageW(self.controls[&RESULTS], LB_GETCURSEL, None, None).0 != 0
            {
                return Err("Search Down did not enter the results".into());
            }
            focus_window(self.controls[&QUERY])?;
            self.key(38)?;
            if SendMessageW(self.controls[&RESULTS], LB_GETCURSEL, None, None).0
                != self.hits.len() as isize - 1
            {
                return Err("Search Up did not reach the last result".into());
            }
            self.key(13)?;
            if self.search || self.selected != Some((last.surah, last.ayah)) {
                return Err("Results Enter did not select the original verse".into());
            }
            self.command(SEARCH, 0)?;
            self.set_text(QUERY, "");
            for _ in 0..600 {
                SendMessageW(
                    self.controls[&QUERY],
                    WM_CHAR,
                    Some(WPARAM('x' as usize)),
                    None,
                );
            }
            if self.text(QUERY).encode_utf16().count() != 512 {
                return Err("Native search edit input limit failed".into());
            }
            for id in [FIRST, LAST] {
                if SendMessageW(
                    self.controls[&id],
                    windows::Win32::UI::Controls::EM_GETLIMITTEXT,
                    None,
                    None,
                )
                .0 != 3
                {
                    return Err("Native ayah edit input limit failed".into());
                }
            }
            let first = self.text(FIRST);
            let last_input = self.text(LAST);
            self.set_text(FIRST, "١٢٣");
            self.set_text(LAST, "۲۸۶");
            std::fs::write(
                out.join("arabic-number-input.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"first":self.text(FIRST),"last":self.text(LAST),"firstNormalized":data::normalize(&self.text(FIRST)),"lastNormalized":data::normalize(&self.text(LAST)),"firstLength":GetWindowTextLengthW(self.controls[&FIRST]),"lastLength":GetWindowTextLengthW(self.controls[&LAST]),"firstHwnd":self.controls[&FIRST].0 as usize,"lastHwnd":self.controls[&LAST].0 as usize,"arabic":self.prefs.arabic,"dpi":self.dpi}),
                )?,
            )?;
            if self.ayah_input(FIRST)? != 123 || self.ayah_input(LAST)? != 286 {
                return Err("Arabic ayah inputs were not parsed".into());
            }
            self.set_text(FIRST, "bad");
            if self.ayah_input(FIRST).is_ok() {
                return Err("Invalid ayah input was accepted".into());
            }
            self.set_text(FIRST, &first);
            self.set_text(LAST, &last_input);
            self.command(SEARCH_CLOSE, 0)?;
            self.command(TEXT_LARGE, 0)?;
            let path = out.join("study-preference-roundtrip.json");
            reader_state::save_json_atomic(&path, &self.prefs, 64 * 1024)?;
            let loaded: Preferences = data::read_json_limit(&path, 64 * 1024)?;
            if loaded.study_text_size != self.study_size {
                return Err("Tafsir size did not survive preference persistence".into());
            }
            let bounded = out.join("oversized-preferences-fixture.json");
            std::fs::write(
                &bounded,
                serde_json::to_vec(&serde_json::json!({"padding":"x".repeat(65*1024)}))?,
            )?;
            if data::read_json_limit::<Preferences>(&bounded, 64 * 1024).is_ok() {
                return Err("Oversized preferences bypassed the read limit".into());
            }
            self.study_size = size;
            self.prefs.study_text_size = size;
            self.prefs.page = page;
            self.selected = selected;
            self.refresh_study_font();
            self.resize();
            std::fs::write(
                out.join("experience-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"uniqueSearchEnterNavigates":true,"searchArrowsReachResults":true,"resultEnterSelectsOriginalVerse":true,"tafsirSizePreferenceRoundtrip":true,"oversizedPreferencesRejected":true,"imeEnterRetainsComposition":true,"nativeInputLimits":true,"arabicAyahInputs":true,"invalidAyahInputRejected":true}),
                )?,
            )?;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_scoped_listening(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let prefs = self.prefs.clone();
            let selected = self.selected;
            let contextual = self.renderer.contextual;
            let playback = std::mem::take(&mut self.playback);
            self.search = false;
            self.popup = None;
            self.panel = Panel::None;
            self.panel_animation = None;
            self.page_turn = None;
            self.page_drag = None;
            let mut cases = Vec::new();
            for mode in [false, true] {
                self.renderer.contextual = mode;
                self.prefs.page = 105;
                self.opening_highlight = None;
                self.playback.begin();
                self.playback.current = Some((4, 175));
                self.resize();
                self.paint()?;
                let key = (4, 176);
                let line = self
                    .renderer
                    .visible
                    .iter()
                    .find(|line| line.runs.iter().any(|run| (run.surah, run.ayah) == key))
                    .ok_or("Missing physical left-page scope probe")?;
                let point = if let Some(hits) = &line.hits {
                    let (rect, _) = hits
                        .iter()
                        .find(|(_, value)| *value == key)
                        .ok_or("Missing contextual scope probe")?;
                    (rect.x + rect.w / 2., rect.y + rect.h / 2.)
                } else {
                    let run = line
                        .runs
                        .iter()
                        .find(|run| (run.surah, run.ayah) == key)
                        .unwrap();
                    let mut x = 0.;
                    let mut y = 0.;
                    let mut metrics =
                        windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS::default();
                    line.layout.HitTestTextPosition(
                        run.start,
                        false,
                        &mut x,
                        &mut y,
                        &mut metrics,
                    )?;
                    (
                        line.origin.X + (metrics.left + metrics.width / 2.) * line.scale,
                        line.origin.Y + (metrics.top + metrics.height / 2.) * line.scale,
                    )
                };
                self.context(point.0, point.1)?;
                if self.popup.as_ref().and_then(|popup| popup.context_target) != Some((106, key))
                    || self.canvas_selection() != Some(key)
                {
                    return Err("Context scope lost its physical page or highlighted ayah".into());
                }
                self.follow_recitation(Some((5, 1)))?;
                if self.prefs.page != 105
                    || self.selected != Some(key)
                    || self.canvas_selection() != Some(key)
                    || self.playback.current != Some((5, 1))
                {
                    return Err("Background recitation retargeted the open ayah menu".into());
                }
                self.paint()?;
                self.capture(&out.join(format!(
                    "native-context-highlight-{}.png",
                    if mode { "contextual" } else { "natural" }
                )))?;
                self.popup_command(MENU_ROW + 2)?;
                if self
                    .popup
                    .as_ref()
                    .unwrap()
                    .rows
                    .iter()
                    .skip(3)
                    .filter(|row| {
                        matches!(
                            row.id,
                            LISTEN_THIS | LISTEN_PAGE | LISTEN_SURAH | LISTEN_QURAN | LISTEN
                        )
                    })
                    .map(|row| row.id)
                    .collect::<Vec<_>>()
                    != [LISTEN_THIS, LISTEN_PAGE, LISTEN_SURAH, LISTEN_QURAN, LISTEN]
                    || self.canvas_selection() != Some(key)
                {
                    return Err("Nested scope choices changed the pinned ayah".into());
                }
                let page =
                    self.context_listening_plan(106, key, crate::app_core::ListeningScope::Page)?;
                let expected = [
                    crate::app_core::ListeningChunk {
                        surah: 4,
                        first: 176,
                        last: 176,
                        count: 176,
                    },
                    crate::app_core::ListeningChunk {
                        surah: 5,
                        first: 1,
                        last: 2,
                        count: 120,
                    },
                ];
                if page.chunks() != expected {
                    return Err(
                        "Page scope did not use the original cross-surah page endpoint".into(),
                    );
                }
                let custom = self
                    .popup
                    .as_ref()
                    .unwrap()
                    .rows
                    .iter()
                    .position(|row| row.id == LISTEN)
                    .unwrap();
                self.popup_command(MENU_ROW + custom as u32)?;
                if self.select(CHAPTER) != 3 || self.text(FIRST) != "176" || self.prefs.page != 106
                {
                    return Err("Custom range lost the physical context target".into());
                }
                cases.push(serde_json::json!({"contextual":mode,"physicalPage":106,"start":[4,176],"end":[5,2],"backgroundAudioDidNotMoveTarget":true}));
                self.panel = Panel::None;
                self.panel_animation = None;
            }
            let tail =
                self.context_listening_plan(604, (112, 4), crate::app_core::ListeningScope::Page)?;
            if tail.chunks().len() != 3
                || tail
                    .chunks()
                    .last()
                    .is_none_or(|chunk| (chunk.surah, chunk.last) != (114, 6))
            {
                return Err("Final-page scope failed its three-surah endpoint".into());
            }
            self.popup = None;
            self.panel = Panel::None;
            self.panel_animation = None;
            self.playback = playback;
            self.prefs = prefs;
            self.selected = selected;
            self.renderer.contextual = contextual;
            self.resize();
            self.paint()?;
            std::fs::write(
                out.join("listening-scope-verification.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "passed":true,"cases":cases,"physicalPageEndpointFromOriginalWords":true,
                    "contextAyahRemainsHighlighted":true,"nestedScopesRemainPinned":true,
                    "customRangePreservesTarget":true,"finalPageThreeSurahs":true,
                    "scope":"Actual native hit probes and menu controls; authentic source-derived plan construction; no recording session or network request"
                }))?,
            )?;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_scoped_recordings(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            if !IsWindow(Some(self.hwnd)).as_bool() {
                return Err("Scoped recording QA cannot start after its window closes".into());
            }
            let continuous = [112, 113, 114].iter().all(|surah| {
                self.profile
                    .join(format!(
                        "AudioStore/Continuous/r92-s{surah:03}/receipt.json"
                    ))
                    .is_file()
            });
            let clips = [
                (1, 1),
                (112, 4),
                (113, 1),
                (113, 2),
                (113, 3),
                (113, 4),
                (113, 5),
                (114, 1),
                (114, 2),
                (114, 3),
                (114, 4),
                (114, 5),
                (114, 6),
            ]
            .iter()
            .all(|(surah, ayah)| {
                self.profile
                    .join(format!("AudioStore/Cache/r00-s{surah:03}-a{ayah:03}.json"))
                    .is_file()
            });
            let rolling_downloads = std::env::args().any(|arg| arg == "--qa-rolling-audio");
            let opening_recording = self
                .profile
                .join("AudioStore/Continuous/r92-s112/receipt.json")
                .is_file();
            if !(continuous || rolling_downloads && opening_recording) || !clips {
                return Err("Scope audio QA requires copied original recordings and clip receipts for 112:4 through 114:6, plus the original basmala clip".into());
            }
            let prefs = self.prefs.clone();
            let selected = self.selected;
            let reciter = self.select(RECITER);
            let chapter = self.select(CHAPTER);
            let first = self.text(FIRST);
            let last = self.text(LAST);
            let audio = self.audio.take();
            let playback = std::mem::take(&mut self.playback);
            let active = self.active_range.take();
            let session = self.listening_session.take();
            let mut cases = Vec::new();
            for timed in [false, true] {
                if !IsWindow(Some(self.hwnd)).as_bool() {
                    return Err("Scoped recording QA was interrupted by window closure".into());
                }
                self.prefs.page = 604;
                self.selected = Some((112, 4));
                self.prefs.speed = 2.;
                self.prefs.volume = 0.;
                self.prefs.repeat_plays = 2;
                self.prefs.continuous_audio = timed;
                // Exercise real profile writes while the decoded recording
                // retains its scoped ancestry/borrowed-file lease below.
                self.dirty = true;
                self.reader_dirty = true;
                self.last_preferences_save = Instant::now() - std::time::Duration::from_secs(2);
                self.last_reader_save = Instant::now() - std::time::Duration::from_secs(6);
                self.last_input = Instant::now() - std::time::Duration::from_millis(200);
                SendMessageW(self.controls[&RECITER], CB_SETCURSEL, Some(WPARAM(0)), None);
                self.context_at((112, 4), self.width / 2., 100., false)?;
                self.popup_command(MENU_ROW + 2)?;
                let scope = self
                    .popup
                    .as_ref()
                    .unwrap()
                    .rows
                    .iter()
                    .position(|row| row.id == LISTEN_QURAN)
                    .unwrap();
                let began = Instant::now();
                self.popup_command(MENU_ROW + scope as u32)?;
                if self
                    .listening_session
                    .as_ref()
                    .is_none_or(|scoped| scoped.rolling != timed)
                {
                    return Err(
                        "Cross-surah listening did not capture its rolling storage mode".into(),
                    );
                }
                let mut sources = Vec::<serde_json::Value>::new();
                self.panel_animation = None;
                self.resize();
                // Observe the actual first decoded completion without consuming
                // it in App::tick, then pause at that precise chapter boundary.
                while !self.audio.as_ref().unwrap().completed() && began.elapsed().as_secs() < 20 {
                    if !crate::audio::pump_messages() || !IsWindow(Some(self.hwnd)).as_bool() {
                        self.audio = None;
                        self.listening_session = None;
                        self.active_range = None;
                        return Err(
                            "Scoped recording QA was interrupted by window closure or thread quit"
                                .into(),
                        );
                    }
                    self.audio.as_mut().unwrap().poll()?;
                    if let Some(source) = self.audio.as_ref().unwrap().qa_timed_source() {
                        if sources
                            .last()
                            .is_none_or(|last| last["path"] != source["path"])
                        {
                            sources.push(source);
                        }
                    }
                    let key = self.audio.as_ref().unwrap().current();
                    self.follow_recitation(key)?;
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                if !self.audio.as_ref().unwrap().completed() {
                    return Err(format!(
                        "First scoped chapter did not finish: {}",
                        self.audio.as_ref().unwrap().status(false)
                    )
                    .into());
                }
                self.command(QUICK_PAUSE, 0)?;
                for _ in 0..5 {
                    if !crate::audio::pump_messages() || !IsWindow(Some(self.hwnd)).as_bool() {
                        self.audio = None;
                        self.listening_session = None;
                        self.active_range = None;
                        return Err(
                            "Scoped recording QA was interrupted by window closure or thread quit"
                                .into(),
                        );
                    }
                    self.tick()?;
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
                if self
                    .active_range
                    .is_none_or(|(range, _)| range.surah != 112)
                    || self.listening_session.as_ref().is_none_or(|scoped| {
                        !scoped.paused
                            || scoped
                                .session
                                .current()
                                .is_none_or(|chunk| chunk.surah != 112)
                    })
                {
                    return Err(
                        "Pausing at decoded chapter completion started the next chapter".into(),
                    );
                }
                self.command(QUICK_PAUSE, 0)?;
                // Form and mode edits must apply only to a later new request.
                if self.dirty || self.reader_dirty {
                    return Err(
                        "Profile state was not saved while the scoped recording was retained"
                            .into(),
                    );
                }
                let saved_preferences: Preferences =
                    data::read_json_limit(&self.profile.join("preferences.json"), 64 * 1024)?;
                if saved_preferences.continuous_audio != timed
                    || saved_preferences.repeat_plays != 2
                {
                    return Err(
                        "Scoped playback profile save did not retain the current preferences"
                            .into(),
                    );
                }
                SendMessageW(self.controls[&RECITER], CB_SETCURSEL, Some(WPARAM(1)), None);
                SendMessageW(self.controls[&CHAPTER], CB_SETCURSEL, Some(WPARAM(0)), None);
                self.set_text(FIRST, "2");
                self.set_text(LAST, "3");
                self.prefs.continuous_audio = !timed;
                self.prefs.repeat_plays = 1;
                let mut chapters = vec![112];
                while began.elapsed().as_secs() < if rolling_downloads { 180 } else { 110 } {
                    if !crate::audio::pump_messages() || !IsWindow(Some(self.hwnd)).as_bool() {
                        self.audio = None;
                        self.listening_session = None;
                        self.active_range = None;
                        return Err(
                            "Scoped recording QA was interrupted by window closure or thread quit"
                                .into(),
                        );
                    }
                    self.tick()?;
                    if let Some(source) = self.audio.as_ref().unwrap().qa_timed_source() {
                        if sources
                            .last()
                            .is_none_or(|last| last["path"] != source["path"])
                        {
                            sources.push(source);
                        }
                    }
                    if let Some((range, mode)) = self.active_range {
                        if range.reciter != 0
                            || mode != timed
                            || ![112, 113, 114].contains(&range.surah)
                        {
                            return Err(
                                "Edited form or mode retargeted captured scope playback".into()
                            );
                        }
                        if chapters.last() != Some(&range.surah) {
                            chapters.push(range.surah);
                        }
                    }
                    if self
                        .listening_session
                        .as_ref()
                        .is_some_and(|scoped| scoped.session.finished())
                    {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                if chapters != [112, 113, 114, 112, 113, 114]
                    || self
                        .listening_session
                        .as_ref()
                        .is_none_or(|scoped| !scoped.session.finished())
                    || !self.audio.as_ref().is_some_and(Audio::completed)
                {
                    return Err(format!("Scoped recording chain did not repeat the entire captured plan: chapters={chapters:?}, status={}", self.audio.as_ref().unwrap().status(false)).into());
                }
                for _ in 0..3 {
                    self.tick()?;
                }
                if self.active_range.is_some() || !self.audio.as_ref().is_some_and(Audio::completed)
                {
                    return Err("Finished scope restarted during subsequent idle polling".into());
                }
                if timed
                    && (sources.len() != 6
                        || sources
                            .iter()
                            .map(|s| s["surah"].as_u64().unwrap_or(0))
                            .collect::<Vec<_>>()
                            != [112, 113, 114, 112, 113, 114])
                {
                    return Err(
                        "Scoped decoder did not attach all six verified original sources".into(),
                    );
                }
                if timed && rolling_downloads {
                    let namespace = self
                        .profile
                        .join("AudioStore/ScopedContinuous")
                        .canonicalize()?;
                    if !sources
                        .iter()
                        .filter(|source| source["surah"] != 112)
                        .all(|source| {
                            source["path"].as_str().is_some_and(|path| {
                                std::path::Path::new(path).starts_with(&namespace)
                            })
                        })
                    {
                        return Err(
                            "Missing continuous sources were not isolated in owned rolling storage"
                                .into(),
                        );
                    }
                }
                cases.push(serde_json::json!({"continuous":timed,"rolling":timed,"originalSources":sources,"start":[112,4],"end":[114,6],
                    "chapters":chapters,"totalPlays":2,"seconds":began.elapsed().as_secs_f64(),
                    "pauseAtChapterBoundary":true,"editedFormDoesNotRetarget":true,"terminalPollingStable":true,
                    "profileSavesWhileRecordingRetained":true}));
                self.command(QUICK_STOP, 0)?;
                self.prefs.continuous_audio = timed;
                self.prefs.repeat_plays = 1;
                SendMessageW(self.controls[&RECITER], CB_SETCURSEL, Some(WPARAM(0)), None);
                self.selected = Some((112, 4));
                self.start_context_audio(604, (112, 4), crate::app_core::ListeningScope::Quran)?;
                self.command(QUICK_STOP, 0)?;
                self.tick()?;
                if self.audio.is_some()
                    || self.listening_session.is_some()
                    || self.active_range.is_some()
                {
                    return Err(
                        "Stopping a preparing scoped request retained its chapter queue".into(),
                    );
                }
            }
            if !IsWindow(Some(self.hwnd)).as_bool() {
                return Err("Scoped recording QA window closed before state restoration".into());
            }
            self.audio = audio;
            self.playback = playback;
            self.active_range = active;
            self.listening_session = session;
            self.prefs = prefs;
            self.selected = selected;
            self.popup = None;
            self.panel = Panel::None;
            self.panel_animation = None;
            SendMessageW(
                self.controls[&RECITER],
                CB_SETCURSEL,
                Some(WPARAM(reciter)),
                None,
            );
            SendMessageW(
                self.controls[&CHAPTER],
                CB_SETCURSEL,
                Some(WPARAM(chapter)),
                None,
            );
            self.set_text(FIRST, &first);
            self.set_text(LAST, &last);
            self.resize();
            std::fs::write(
                out.join("scoped-recording-verification.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "passed":true,"muted":true,"cases":cases,"stopCancelsPreparingPlan":true,
                    "scope":"Production Windows worker and decoder, copied original Yasser recordings and per-ayah clips; two full three-surah-plan plays in each mode; preparation gaps may occur"
                }))?,
            )?;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_hover_polish(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            self.search = false;
            self.popup = None;
            self.panel = Panel::None;
            self.panel_animation = None;
            self.page_turn = None;
            self.page_drag = None;
            self.drag_start = None;
            self.prefs.page = 45;
            let selected = self.selected;
            let contextual = self.renderer.contextual;
            self.selected = Some((46, 21));
            for mode in [false, true] {
                self.renderer.contextual = mode;
                self.resize();
                self.paint()?;
                let mut points = Vec::new();
                for line in &self.renderer.visible {
                    if let Some(regions) = &line.hits {
                        points.extend(
                            regions
                                .iter()
                                .map(|(r, k)| ((r.x + r.w / 2., r.y + r.h / 2.), *k)),
                        );
                    } else {
                        for run in line.runs.iter() {
                            let mut x = 0.;
                            let mut y = 0.;
                            let mut metrics=windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS::default();
                            line.layout.HitTestTextPosition(
                                run.start,
                                false,
                                &mut x,
                                &mut y,
                                &mut metrics,
                            )?;
                            let point = (
                                line.origin.X + (metrics.left + metrics.width / 2.) * line.scale,
                                line.origin.Y + (metrics.top + metrics.height / 2.) * line.scale,
                            );
                            if self.renderer.hit(point.0, point.1) == Some((run.surah, run.ayah)) {
                                points.push((point, (run.surah, run.ayah)));
                            }
                        }
                    }
                }
                let first = points
                    .first()
                    .copied()
                    .ok_or("Missing hover source probe")?;
                let second = points
                    .iter()
                    .find(|(_, k)| *k != first.1)
                    .copied()
                    .ok_or("Missing second hover ayah")?;
                self.update_hover(Some(first.0));
                if self.hovered != Some(first.1) {
                    return Err("Hover missed first original ayah".into());
                }
                self.update_hover(Some(second.0));
                if self.hovered != Some(second.1) || self.selected != Some((46, 21)) {
                    return Err("Hover changed selection or retained the previous ayah".into());
                }
                self.paint()?;
                self.capture(&out.join(format!(
                    "native-hover-{}.png",
                    if mode { "contextual" } else { "natural" }
                )))?;
                self.update_hover(None);
                if self.hovered.is_some() {
                    return Err("Pointer leaving retained a highlight".into());
                }
                self.update_hover(Some((0., 0.)));
                if self.hovered.is_some() {
                    return Err("Blank margin highlights an ayah".into());
                }
            }
            self.renderer.contextual = contextual;
            self.selected = selected;
            self.resize();
            self.panel = Panel::Study;
            self.panel_animation = None;
            self.selected = Some((46, 21));
            self.context_at((46, 21), self.width / 2., 100., true)?;
            if self.popup.is_none() {
                return Err("Right click on the current study ayah failed to show actions".into());
            }
            self.selected = Some((1, 1));
            self.prefs.page = 1;
            self.popup_command(MENU_ROW + 2)?;
            let custom = self
                .popup
                .as_ref()
                .unwrap()
                .rows
                .iter()
                .position(|row| row.id == LISTEN)
                .ok_or("Custom listening range disappeared")?;
            self.popup_command(MENU_ROW + custom as u32)?;
            if self.select(CHAPTER) != 45 || self.text(FIRST) != "21" {
                return Err("Listening context target changed with background selection".into());
            }
            self.popup = None;
            self.panel = Panel::None;
            self.selected = selected;
            self.resize();
            let mut original_window = RECT::default();
            GetWindowRect(self.hwnd, &mut original_window)?;
            let mut fold_cases = Vec::new();
            for narrow in [false, true] {
                if narrow {
                    let scale = self.dpi / 96.;
                    SetWindowPos(
                        self.hwnd,
                        None,
                        0,
                        0,
                        (900. * scale) as i32,
                        (700. * scale) as i32,
                        SWP_NOMOVE | SWP_NOZORDER,
                    )?;
                    self.panel = Panel::Listen;
                }
                self.panel_animation = None;
                self.resize();
                for mode in [false, true] {
                    self.renderer.contextual = mode;
                    for direction in [-1, 1] {
                        for top in [false, true] {
                            self.prefs.page = 45;
                            self.page_drag = Some(crate::page_turn::Drag {
                                old: 45,
                                new: self.turn_destination(direction),
                                direction,
                                progress: 0.55,
                                top,
                            });
                            self.invalidate_reader();
                            self.paint()?;
                            let name = format!(
                                "leaf-{}-{}-{}-{}.png",
                                if narrow { "single" } else { "spread" },
                                if mode { "contextual" } else { "natural" },
                                direction,
                                if top { "top" } else { "bottom" }
                            );
                            self.capture(&out.join(&name))?;
                            let mut transform = windows_numerics::Matrix3x2::identity();
                            self.renderer
                                .target
                                .as_ref()
                                .ok_or("Missing leaf QA surface")?
                                .GetTransform(&mut transform);
                            if transform != windows_numerics::Matrix3x2::identity() {
                                return Err("Leaf left a transformed render target".into());
                            }
                            let destination = self.turn_destination(direction);
                            self.page_drag.as_mut().unwrap().progress = 0.999;
                            self.invalidate_reader();
                            self.paint()?;
                            self.capture(&out.join(name.replace(".png", "-ending.png")))?;
                            self.page_drag = None;
                            self.prefs.page = destination;
                            self.invalidate_reader();
                            self.paint()?;
                            self.capture(&out.join(name.replace(".png", "-landed.png")))?;
                            fold_cases.push(name);
                        }
                    }
                }
                self.page_drag = None;
                self.panel = Panel::None;
                SetWindowPos(
                    self.hwnd,
                    None,
                    original_window.left,
                    original_window.top,
                    original_window.right - original_window.left,
                    original_window.bottom - original_window.top,
                    SWP_NOZORDER,
                )?;
                self.resize();
            }
            self.renderer.contextual = contextual;
            self.resize();
            std::fs::write(
                out.join("leaf-rendering-cases.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"cases":fold_cases,"targetTransformsRestored":true,"singleAndSpread":true,"bothDirectionsAndCorners":true}),
                )?,
            )?;
            std::fs::write(
                out.join("hover-context-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"oneHoverAyahBothModes":true,"hoverIndependentOfSelection":true,"leavingClearsHighlight":true,"blankMarginsDoNotHighlight":true,"rightClickCurrentStudyAlwaysOpensMenu":true,"contextTargetRemainsPinned":true}),
                )?,
            )?;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_reading_polish(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            self.popup = None;
            self.search = false;
            self.page_turn = None;
            self.turn_queue.clear();
            self.panel = Panel::Study;
            self.panel_animation = None;
            self.selected = Some((46, 21));
            let mut themes = Vec::new();
            for arabic in [false, true] {
                self.prefs.arabic = arabic;
                self.source = if arabic { 1 } else { 2 };
                self.refresh_labels();
                let original =
                    self.resources
                        .study(if arabic { "saadi" } else { "sahih" }, 46, 21)?;
                self.study()?;
                self.resize();
                if self.panel_on_left() == arabic {
                    return Err("Study panel opened on the wrong side".into());
                }
                let body = self.placements.borrow()[&STUDY].0;
                let reader = self.reader_area();
                if if arabic {
                    body.x <= reader.x + reader.w
                } else {
                    body.x + body.w >= reader.x
                } {
                    return Err("Study and reader origins overlap".into());
                }
                if !self.text(STUDY).starts_with(&original) {
                    return Err("Study lost the exact selected ayah".into());
                }
                for theme in 0..4 {
                    self.prefs.theme = theme;
                    self.theme_start = None;
                    self.replace_background(self.palette());
                    self.study()?;
                    self.paint()?;
                    focus_window(self.controls[&STUDY])?;
                    SendMessageW(
                        self.controls[&STUDY],
                        WM_LBUTTONDOWN,
                        Some(WPARAM(1)),
                        Some(LPARAM(12 | (12 << 16))),
                    );
                    SendMessageW(
                        self.controls[&STUDY],
                        WM_LBUTTONUP,
                        Some(WPARAM(0)),
                        Some(LPARAM(12 | (12 << 16))),
                    );
                    SendMessageW(
                        self.controls[&STUDY],
                        WM_CHAR,
                        Some(WPARAM('x' as usize)),
                        Some(LPARAM(0)),
                    );
                    if !self.text(STUDY).starts_with(&original) {
                        return Err("Read-only study accepted typing".into());
                    }
                    let mut info = GUITHREADINFO {
                        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                        ..Default::default()
                    };
                    GetGUIThreadInfo(0, &mut info)?;
                    if info.hwndCaret == self.controls[&STUDY]
                        && info.flags.contains(GUI_CARETBLINKING)
                    {
                        return Err("Study insertion caret remained visible".into());
                    }
                    SendMessageW(
                        self.controls[&STUDY],
                        EM_SETSEL,
                        Some(WPARAM(0)),
                        Some(LPARAM(
                            original
                                .replace("\r\n", "\r")
                                .replace('\n', "\r")
                                .encode_utf16()
                                .count() as isize,
                        )),
                    );
                    let mut selected = vec![0u16; original.encode_utf16().count() + 1];
                    let count = SendMessageW(
                        self.controls[&STUDY],
                        EM_GETSELTEXT,
                        None,
                        Some(LPARAM(selected.as_mut_ptr() as isize)),
                    )
                    .0 as usize;
                    if String::from_utf16(&selected[..count])?
                        .replace("\r\n", "\n")
                        .replace('\r', "\n")
                        != original.replace("\r\n", "\n").replace('\r', "\n")
                    {
                        return Err("Selected commentary differs from its original source".into());
                    }
                    SendMessageW(
                        self.controls[&STUDY],
                        EM_SETSEL,
                        Some(WPARAM(0)),
                        Some(LPARAM(0)),
                    );
                    self.paint()?;
                    GetGUIThreadInfo(0, &mut info)?;
                    if info.hwndCaret == self.controls[&STUDY]
                        && info.flags.contains(GUI_CARETBLINKING)
                    {
                        return Err("Study selection collapse restored the insertion caret".into());
                    }
                    let _ = RedrawWindow(
                        Some(self.controls[&STUDY]),
                        None,
                        None,
                        RDW_INVALIDATE | RDW_FRAME | RDW_UPDATENOW,
                    );
                    let mut scroll = SCROLLBARINFO {
                        cbSize: std::mem::size_of::<SCROLLBARINFO>() as u32,
                        ..Default::default()
                    };
                    GetScrollBarInfo(self.controls[&STUDY], OBJID_VSCROLL, &mut scroll)?;
                    let mut window = RECT::default();
                    GetWindowRect(self.controls[&STUDY], &mut window)?;
                    let dc = GetWindowDC(Some(self.controls[&STUDY]));
                    let sample = GetPixel(
                        dc,
                        scroll.rcScrollBar.left - window.left + 1,
                        (scroll.rcScrollBar.top + scroll.rcScrollBar.bottom) / 2 - window.top,
                    );
                    ReleaseDC(Some(self.controls[&STUDY]), dc);
                    let brightness =
                        ((sample.0 & 255) + ((sample.0 >> 8) & 255) + ((sample.0 >> 16) & 255)) / 3;
                    if (theme == 0 || theme == 3) && brightness > 180 {
                        return Err(format!("Study scrollbar remained bright in a dark theme: arabic={arabic} theme={theme} rgb={:x} rect={:?} window={:?}",sample.0,scroll.rcScrollBar,window).into());
                    }
                    themes.push(serde_json::json!({"arabic":arabic,"theme":theme,"scrollbarEdgeRgb":sample.0,"caretHidden":true}));
                    self.capture(&out.join(format!(
                        "reading-study-{}-theme-{theme}.png",
                        if arabic { "arabic" } else { "english" }
                    )))?;
                }
            }
            self.prefs.theme = 0;
            self.replace_background(self.palette());
            self.panel = Panel::None;
            self.panel_animation = None;
            self.resize();
            focus_window(self.hwnd)?;
            std::fs::write(
                out.join("reading-polish.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "passed":true,"englishStudyOnLeft":true,"arabicStudyOnRight":true,"studyCommentaryExact":true,
                    "readOnlyTypingRejected":true,"selectionExactWithoutClipboardChanges":true,"nativeParagraphSeparatorsNormalized":true,"caretHiddenAfterClick":true,
                    "privateQuranFontAvailableToRichEdit":self.quran_study_font.is_some(),"themes":themes
                }))?,
            )?;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_keyboard_entry(&mut self) -> Result<serde_json::Value> {
        unsafe {
            self.options()?;
            self.popup.as_mut().unwrap().motion = None;
            self.resize();
            let rows = self.popup.as_ref().unwrap().rows.len() as u32;
            if rows < 4 {
                return Err("Keyboard fixture requires the original Options sections".into());
            }
            SetFocus(None).ok();
            if !GetFocus().is_invalid() {
                return Err("Native focus-clear fixture did not clear focus".into());
            }
            focus_window(self.hwnd)?;
            if GetFocus() != self.hwnd {
                return Err("A null previous focus prevented native menu entry".into());
            }
            for (tab, backward, expected) in [
                (true, false, MENU_ROW),
                (false, false, MENU_ROW),
                (true, true, MENU_CLOSE),
                (false, true, MENU_ROW + rows - 1),
            ] {
                focus_window(self.hwnd)?;
                // Invoke the same direction-aware production path without
                // changing the user's physical Shift state for this fixture.
                self.popup_focus(tab, backward)?;
                if GetFocus() != self.controls[&expected] {
                    return Err("First menu keyboard action skipped its requested edge".into());
                }
            }
            focus_window(self.hwnd)?;
            self.key(40)?;
            if GetFocus() != self.controls[&MENU_ROW] {
                return Err("First Down did not enter Options at its first section".into());
            }
            focus_window(self.hwnd)?;
            self.key(38)?;
            if GetFocus() != self.controls[&(MENU_ROW + rows - 1)] {
                return Err("First Up did not enter Options at its last section".into());
            }
            self.key(40)?;
            if GetFocus() != self.controls[&MENU_ROW] {
                return Err("Menu Down failed to wrap through visible sections".into());
            }
            let hidden = self.controls[&(MENU_ROW + 1)];
            let disabled = self.controls[&(MENU_ROW + 2)];
            let _ = ShowWindow(hidden, SW_HIDE);
            let _ = EnableWindow(disabled, false);
            if focus_window(disabled).is_ok() {
                return Err("A disabled native row incorrectly accepted focus".into());
            }
            focus_window(self.controls[&MENU_ROW])?;
            self.key(40)?;
            if GetFocus() != self.controls[&(MENU_ROW + 3)] {
                return Err("Menu keyboard navigation reached a hidden or disabled row".into());
            }
            self.popup_focus(true, true)?;
            if GetFocus() != self.controls[&MENU_ROW] {
                return Err("Backward menu Tab reached a hidden or disabled row".into());
            }
            let _ = EnableWindow(disabled, true);
            let _ = ShowWindow(hidden, SW_SHOW);
            self.popup_command(MENU_CLOSE)?;

            self.context_at((1, 1), self.width / 2., 140., false)?;
            self.popup_command(MENU_ROW + 2)?;
            let animating = self.popup.as_ref().is_some_and(|p| p.motion.is_some());
            if animating {
                self.popup.as_mut().unwrap().motion = Some(Instant::now());
                self.resize();
                let rows = self.popup.as_ref().unwrap().rows.len() as u32;
                let visible = (MENU_ROW..MENU_ROW + rows)
                    .filter(|id| IsWindowVisible(self.controls[id]).as_bool())
                    .count();
                if visible == 0 || visible == rows as usize {
                    return Err(
                        "Inline animation fixture did not retain hidden revealing rows".into(),
                    );
                }
                focus_window(self.hwnd)?;
                for _ in 0..visible * 2 {
                    self.key(40)?;
                    let focus = GetFocus();
                    if !IsWindowVisible(focus).as_bool() || !IsWindowEnabled(focus).as_bool() {
                        return Err("Keyboard focus entered an unrevealed listening option".into());
                    }
                }
            }
            self.popup_command(MENU_CLOSE)?;

            let mut dropdowns = Vec::new();
            for id in [RECITER, CHAPTER, SPEED, REPEAT] {
                self.command(LISTEN, 0)?;
                self.panel_animation = None;
                self.resize();
                let form: Vec<_> = [RECITER, CHAPTER, FIRST, LAST, SPEED, REPEAT]
                    .iter()
                    .map(|id| self.text(*id))
                    .collect();
                let combo = self.controls[&id];
                let selection = SendMessageW(combo, CB_GETCURSEL, None, None).0;
                let active = self
                    .active_range
                    .map(|(r, c)| (r.reciter, r.surah, r.first, r.last, r.count, c));
                let session = self.audio.as_ref().map(Audio::has_session);
                let follow = (self.playback.current, self.playback.following);
                let mut info = COMBOBOXINFO {
                    cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                    ..Default::default()
                };
                GetComboBoxInfo(combo, &mut info)?;
                focus_window(if id == RECITER { info.hwndItem } else { combo })?;
                SendMessageW(combo, CB_SHOWDROPDOWN, Some(WPARAM(1)), None);
                if SendMessageW(combo, CB_GETDROPPEDSTATE, None, None).0 == 0 {
                    return Err("Native keyboard fixture did not open its dropdown".into());
                }
                if !self.key(27)?
                    || SendMessageW(combo, CB_GETDROPPEDSTATE, None, None).0 != 0
                    || self.panel != Panel::Listen
                    || self.search
                    || selection != SendMessageW(combo, CB_GETCURSEL, None, None).0
                    || form
                        != [RECITER, CHAPTER, FIRST, LAST, SPEED, REPEAT]
                            .iter()
                            .map(|id| self.text(*id))
                            .collect::<Vec<_>>()
                    || active
                        != self
                            .active_range
                            .map(|(r, c)| (r.reciter, r.surah, r.first, r.last, r.count, c))
                    || session != self.audio.as_ref().map(Audio::has_session)
                    || follow != (self.playback.current, self.playback.following)
                {
                    return Err("First Escape changed the listening form instead of cancelling its native list".into());
                }
                self.key(27)?;
                if self.panel != Panel::None
                    || session != self.audio.as_ref().map(Audio::has_session)
                    || active
                        != self
                            .active_range
                            .map(|(r, c)| (r.reciter, r.surah, r.first, r.last, r.count, c))
                {
                    return Err("Second Escape failed to close only the listening editor".into());
                }
                self.panel_animation = None;
                self.resize();
                dropdowns.push(id);
            }
            self.command(SEARCH, 0)?;
            self.command(BROWSE_SECTIONS, 0)?;
            self.search_animation = None;
            self.resize();
            let combo = self.controls[&BROWSE_KIND];
            let selection = SendMessageW(combo, CB_GETCURSEL, None, None).0;
            focus_window(combo)?;
            SendMessageW(combo, CB_SHOWDROPDOWN, Some(WPARAM(1)), None);
            if SendMessageW(combo, CB_GETDROPPEDSTATE, None, None).0 == 0 {
                return Err("Section keyboard fixture did not open its dropdown".into());
            }
            self.key(27)?;
            if !self.search
                || SendMessageW(combo, CB_GETDROPPEDSTATE, None, None).0 != 0
                || selection != SendMessageW(combo, CB_GETCURSEL, None, None).0
            {
                return Err("First Escape closed section Search instead of its native list".into());
            }
            self.key(27)?;
            if self.search {
                return Err("Second Escape did not close section Search".into());
            }
            Ok(
                serde_json::json!({"nativeFocusWithoutPreviousControl":true,"disabledFocusRejected":true,"firstTabEntersFirstRow":true,"firstDownEntersFirstRow":true,"firstShiftTabEntersLastControl":true,"firstUpEntersLastRow":true,"hiddenAndDisabledRowsSkipped":true,"animatedHiddenRowsChecked":animating,"visibleRowsWrap":true,"nativeComboFirstEscapeKeepsFormAndSelection":true,"nativeComboSecondEscapeClosesOnlyForm":true,"listenCombos":dropdowns,"sectionSearchEscapeKeepsSelection":true,"scope":"Programmed native control keyboard routing; Shift direction passed to the production focus helper without changing physical keyboard state"}),
            )
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_settings_theme(&mut self, out: &std::path::Path) -> Result<serde_json::Value> {
        unsafe {
            let original = self.prefs.theme;
            self.popup_command(MENU_ROW + 1)?;
            if !IsWindowVisible(self.controls[&THEME_SELECTOR]).as_bool()
                || IsWindowVisible(self.controls[&MENU_ROW]).as_bool()
                || self.select(THEME_SELECTOR) != original as usize
            {
                return Err(
                    "Appearance must show its selected theme in one native dropdown".into(),
                );
            }
            let combo = self.controls[&THEME_SELECTOR];
            focus_window(combo)?;
            SendMessageW(combo, CB_SHOWDROPDOWN, Some(WPARAM(1)), None);
            if SendMessageW(combo, CB_GETDROPPEDSTATE, None, None).0 == 0 {
                return Err("Theme dropdown did not open".into());
            }
            self.key(27)?;
            if self.popup.as_ref().is_none_or(|p| p.stack.len() != 1)
                || SendMessageW(combo, CB_GETDROPPEDSTATE, None, None).0 != 0
            {
                return Err("First Escape must close only the theme dropdown".into());
            }
            self.theme_start = None;
            self.theme_frame = None;
            self.prefs.theme = 0;
            self.replace_background(self.palette());
            let native_before = self.study_control_dark;
            self.theme(1);
            // Force a deterministic middle frame even with OS animations off:
            // these are production frame/color paths, not timing certification.
            let start = Instant::now();
            self.theme_start = Some((start, Palette::new(0)));
            self.advance_theme(start + std::time::Duration::from_millis(90));
            let sample = self.palette();
            let colors = CONTROL_COLORS.with(Cell::get);
            if colors.ink != sample.ink_rgb()
                || colors.background != sample.background_rgb()
                || self.study_control_dark != native_before
            {
                return Err("Theme frame colors or deferred native theme are inconsistent".into());
            }
            self.invalidate();
            self.paint()?;
            let after = self.palette();
            if sample.ink_rgb() != after.ink_rgb()
                || sample.background_rgb() != after.background_rgb()
            {
                return Err("Child composition advanced the shared theme sample".into());
            }
            self.advance_theme(start + std::time::Duration::from_millis(180));
            if self.theme_start.is_some()
                || self.study_control_dark != Some(false)
                || self.palette().background_rgb() != Palette::new(1).background_rgb()
            {
                return Err("Theme landing did not commit the final native theme".into());
            }
            for theme in [2, 0, 1] {
                SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(theme)), None);
                self.command(THEME_SELECTOR, CBN_SELCHANGE)?;
                if self.prefs.theme != theme as u32
                    || self.popup.as_ref().is_none_or(|p| p.stack.len() != 1)
                    || !IsWindowVisible(combo).as_bool()
                {
                    return Err("Choosing another theme dismissed or reset Appearance".into());
                }
                if let Some((start, _)) = self.theme_start {
                    self.advance_theme(start + std::time::Duration::from_millis(180));
                }
            }
            self.invalidate();
            self.paint()?;
            self.capture(&out.join(if self.prefs.arabic {
                "native-appearance-dropdown-arabic.png"
            } else {
                "native-appearance-dropdown-english.png"
            }))?;
            self.popup_command(MENU_CLOSE)?;
            self.theme_start = None;
            self.theme_frame = None;
            self.prefs.theme = original;
            SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(original as usize)), None);
            self.replace_background(self.palette());
            Ok(
                serde_json::json!({"nativeDropdown":true,"selectionKeepsAppearanceOpen":true,"escapeClosesDropdownFirst":true,"sharedFramePalette":true,"nativeThemeCommitsAtLanding":true,"scope":"Deterministic native-control commands and shared production frame sampling; not compositor presentation or physical-input latency."}),
            )
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_fluidity_interaction(&mut self) -> Result<serde_json::Value> {
        unsafe {
            let form_text: Vec<_> = [QUERY, BROWSE_QUERY, RECITER, FIRST, LAST, STATUS, NOTICE]
                .into_iter()
                .map(|id| (id, self.text(id)))
                .collect();
            let form_selection: Vec<_> = [RECITER, CHAPTER, SPEED, REPEAT, BROWSE_KIND]
                .into_iter()
                .map(|id| {
                    (
                        id,
                        SendMessageW(self.controls[&id], CB_GETCURSEL, None, None).0,
                    )
                })
                .collect();
            let prefs = self.prefs.clone();
            let reader = std::mem::take(&mut self.reader);
            let profile = self.profile.clone();
            let audio = self.audio.take();
            let download = self.download.take();
            let tafsir_job = self.tafsir_job.take();
            let tafsir_job_source = self.tafsir_job_source.take();
            let storage = self.storage_pending.take();
            let popup = self.popup.take();
            let session = self.listening_session.take();
            let playback = std::mem::take(&mut self.playback);
            let hits = std::mem::take(&mut self.hits);
            let reading_pages = std::mem::take(&mut self.reading_pages);
            let saved = (
                self.panel,
                self.panel_animation,
                self.panel_pose,
                self.search,
                self.search_animation,
                self.title_animation,
                self.theme_start,
                self.panel_scroll,
                self.last_input,
                self.dirty,
                self.reader_dirty,
                self.last_preferences_save,
                self.last_reader_save,
                self.selected,
                self.hovered,
                self.search_reveal,
                self.daily,
                self.clock_minute,
                self.reading_page,
                self.browse_tab,
                self.poll_interval,
                self.notice_until,
            );
            let result = (|| -> Result<serde_json::Value> {
                self.panel = Panel::None;
                self.panel_animation = None;
                self.panel_pose = None;
                self.search = false;
                self.search_animation = None;
                self.title_animation = None;
                self.theme_start = None;
                self.panel_scroll = 0.;
                self.daily = None;
                self.notice_until = None;
                self.resize();

                let before = self.layout_generation;
                self.options()?;
                if self.layout_generation != before.wrapping_add(1)
                    || self.popup.as_ref().is_none_or(|p| !p.docked)
                    || GetFocus() != self.hwnd
                {
                    return Err("Options exposed an intermediate menu layout or focus".into());
                }
                // Elapsed time saturates at zero for these explicit future
                // starts. Initial poses stay deterministic even on a slow host.
                let frozen = Instant::now() + std::time::Duration::from_secs(60);
                self.popup.as_mut().unwrap().motion = Some(frozen);
                self.resize();
                let p = self.popup.as_ref().unwrap();
                let rect = p.display_rect();
                for (id, x, y) in [
                    (MENU_ROW, rect.x + 8., rect.y + 48.),
                    (MENU_CLOSE, rect.x + rect.w - 40., rect.y + 7.),
                ] {
                    let (placed, visible) = self.placements.borrow()[&id];
                    if !visible
                        || !IsWindowVisible(self.controls[&id]).as_bool()
                        || (placed.x - x).abs() > 0.01
                        || (placed.y - y).abs() > 0.01
                    {
                        return Err(
                            "Initial Options controls differ from its displayed panel".into()
                        );
                    }
                }
                if !p.contains(rect.x + 2., rect.y + 50.)
                    || p.contains(rect.x - 2., rect.y + 50.)
                    || p.contains(rect.x + rect.w + 2., rect.y + 50.)
                    || p.contains(
                        if p.rtl {
                            p.rect.x + 2.
                        } else {
                            p.rect.x + p.rect.w - 2.
                        },
                        rect.y + 50.,
                    )
                {
                    return Err("Animated popup hit bounds differ from its visible bounds".into());
                }
                self.popup_command(MENU_CLOSE)?;

                let before = self.layout_generation;
                self.command(SEARCH, 0)?;
                if self.layout_generation != before.wrapping_add(1)
                    || !self.search
                    || self.hits.len() != 114
                    || GetFocus() != self.controls[&QUERY]
                {
                    return Err("Search revealed before its original browse model was ready".into());
                }
                self.search_animation = Some(frozen);
                self.resize();
                for (id, y) in [
                    (SEARCH_CLOSE, 42.),
                    (QUERY, 126.),
                    (BROWSE_SURAH, 210.),
                    (BROWSE_PAGE, 210.),
                    (BROWSE_SECTIONS, 210.),
                    (BROWSE_QUERY, 310.),
                    (RESULTS, 358.),
                ] {
                    let (rect, visible) = self.placements.borrow()[&id];
                    if !visible
                        || !IsWindowVisible(self.controls[&id]).as_bool()
                        || (rect.y - y).abs() > 0.01
                    {
                        return Err(
                            "Initial Search control ignored its frozen opening offset".into()
                        );
                    }
                }
                self.invalidate();
                self.paint()?;
                let result_list = self.controls[&RESULTS];
                let original_generation = self.results_generation;
                if self
                    .motion_controls
                    .borrow()
                    .get(&RESULTS)
                    .is_none_or(|(key, _)| key.results != Some((original_generation, 0, -1)))
                {
                    return Err(format!("Search opening did not cache its exact unselected model: expected={original_generation}, native=({},{}), cached={:?}, focus={:?}, list={:?}, popup={}",SendMessageW(result_list,LB_GETTOPINDEX,None,None).0,SendMessageW(result_list,LB_GETCURSEL,None,None).0,self.motion_controls.borrow().get(&RESULTS).map(|(key,_)|key.results),GetFocus(),result_list,self.popup.is_some()).into());
                }
                SendMessageW(result_list, LB_SETCURSEL, Some(WPARAM(3)), None);
                SendMessageW(result_list, LB_SETTOPINDEX, Some(WPARAM(2)), None);
                self.invalidate();
                self.paint()?;
                if self
                    .motion_controls
                    .borrow()
                    .get(&RESULTS)
                    .is_none_or(|(key, _)| key.results != Some((original_generation, 2, 3)))
                {
                    return Err(format!(
                        "Search motion selection/scroll mismatch: native=({},{}) cached={:?} focus={:?} list={:?}",
                        SendMessageW(result_list, LB_GETTOPINDEX, None, None).0,
                        SendMessageW(result_list, LB_GETCURSEL, None, None).0,
                        self.motion_controls.borrow().get(&RESULTS).map(|(key, _)| key.results),
                        GetFocus(), result_list
                    ).into());
                }
                self.set_text(BROWSE_QUERY, "fatiha");
                self.update_search_model()?;
                self.resize();
                self.invalidate();
                self.paint()?;
                if self.hits.len() != 1
                    || self.results_generation == original_generation
                    || self
                        .motion_controls
                        .borrow()
                        .get(&RESULTS)
                        .is_none_or(|(key, _)| {
                            key.results != Some((self.results_generation, 0, -1))
                        })
                {
                    return Err("Search motion reused rows from a superseded query".into());
                }
                self.command(SEARCH_CLOSE, 0)?;

                // Freeze the production animation's interpolation at its
                // source on retarget. The original close is half-complete.
                self.panel = Panel::None;
                self.panel_animation = Some((frozen, Panel::Listen, 382.));
                self.panel_pose = Some(0.5);
                let middle = self.panel_width();
                self.set_panel(Panel::Listen);
                self.panel_pose = Some(0.);
                let reopening = self.panel_width();
                if client_animation_enabled()
                    && (self.panel_animation.is_none()
                        || !(middle > 0. && middle < 382.)
                        || (reopening - middle).abs() > 0.01)
                {
                    return Err("Reopening Listen restarted from its settled width".into());
                }
                self.panel_pose = Some(0.5);
                let closing_from = self.panel_width();
                self.set_panel(Panel::None);
                self.panel_pose = Some(0.);
                if client_animation_enabled()
                    && (self.panel_animation.is_none()
                        || (self.panel_width() - closing_from).abs() > 0.01)
                {
                    return Err("Closing Listen again jumped from its current width".into());
                }
                self.panel_animation = None;
                self.panel_pose = None;
                self.resize();

                // Publish only in this QA profile's own child directory. The
                // calling profile and its prior files are never replaced.
                self.profile = profile.join(if self.prefs.arabic {
                    "fluidity-save-arabic"
                } else {
                    "fluidity-save-english"
                });
                self.reader.goal.day = local_day().0;
                self.prefs.speed = 1.;
                self.prefs.volume = 0.8;
                self.dirty = true;
                self.reader_dirty = true;
                self.last_input = Instant::now() - std::time::Duration::from_secs(2);
                self.last_preferences_save = Instant::now() - std::time::Duration::from_secs(2);
                self.last_reader_save = Instant::now() - std::time::Duration::from_secs(6);
                self.tick()?;
                let preference_path = self.profile.join("preferences.json");
                let reader_path = self.profile.join("reader-state.json");
                let previous_prefs = std::fs::read(&preference_path)?;
                let previous_reader = std::fs::read(&reader_path)?;
                self.prefs.speed = 1.25;
                self.prefs.volume = 0.37;
                self.reader.reading.show_timer = true;
                self.reader.goal.target = 10;
                self.dirty = true;
                self.reader_dirty = true;
                self.last_preferences_save = Instant::now() - std::time::Duration::from_secs(2);
                self.last_reader_save = Instant::now() - std::time::Duration::from_secs(6);
                for motion in ["search", "panel", "options", "recent-input"] {
                    self.search_animation = (motion == "search").then_some(frozen);
                    self.panel_animation =
                        (motion == "panel").then_some((frozen, Panel::Listen, 382.));
                    if motion == "options" {
                        self.options()?;
                        self.popup.as_mut().unwrap().motion = Some(frozen);
                    }
                    self.last_input = if motion == "recent-input" {
                        frozen
                    } else {
                        Instant::now() - std::time::Duration::from_secs(2)
                    };
                    if self.maintenance_ready() {
                        return Err(
                            "Maintenance was ready during visible motion or recent input".into(),
                        );
                    }
                    self.tick()?;
                    if !self.dirty
                        || !self.reader_dirty
                        || std::fs::read(&preference_path)? != previous_prefs
                        || std::fs::read(&reader_path)? != previous_reader
                    {
                        return Err(
                            "Visible motion or recent input published dirty profile files".into(),
                        );
                    }
                    self.popup = None;
                }
                self.search_animation = None;
                self.panel_animation = None;
                self.last_input = Instant::now() - std::time::Duration::from_secs(2);
                if !self.maintenance_ready() {
                    return Err("Completed motion did not release idle maintenance".into());
                }
                self.tick()?;
                let published: Preferences = data::read_json_limit(&preference_path, 64 * 1024)?;
                let (published_reader, recovered) =
                    ReaderState::load(&self.profile, &self.resources.surahs)?;
                if self.dirty
                    || self.reader_dirty
                    || (published.speed - 1.25).abs() > 0.001
                    || (published.volume - 0.37).abs() > 0.001
                    || published.arabic != self.prefs.arabic
                    || recovered
                    || !published_reader.reading.show_timer
                    || published_reader.goal.target != 10
                {
                    return Err(
                        "Idle maintenance lost the deferred settings or reading values".into(),
                    );
                }
                Ok(
                    serde_json::json!({"singleOptionsRevealLayout":true,"optionsInitialPlacementMatchesDisplay":true,"animatedPopupHitBoundsMatchDisplay":true,"singleSearchRevealLayout":true,"searchModelReadyBeforeReveal":true,"searchInitialOffsetApplied":true,"searchMotionSnapshotTracksModelAndScroll":true,"listenReversalContinuous":true,"animationPreferenceEnabled":client_animation_enabled(),"motionDefersPreferenceAndReaderSaves":true,"recentInputDefersSaves":true,"idlePublishesDeferredValues":true,"isolatedProfile":self.profile,"scope":"Single reveal layout generations, programmed frozen poses and actual bounded atomic saves in a separate QA profile; not physical-input or compositor-FPS certification"}),
                )
            })();
            self.prefs = prefs;
            self.reader = reader;
            self.profile = profile;
            self.audio = audio;
            self.download = download;
            self.tafsir_job = tafsir_job;
            self.tafsir_job_source = tafsir_job_source;
            self.storage_pending = storage;
            self.popup = popup;
            self.listening_session = session;
            self.playback = playback;
            self.hits = hits;
            self.results_generation = self.results_generation.wrapping_add(1);
            self.reading_pages = reading_pages;
            let actual_poll_interval = self.poll_interval;
            (
                self.panel,
                self.panel_animation,
                self.panel_pose,
                self.search,
                self.search_animation,
                self.title_animation,
                self.theme_start,
                self.panel_scroll,
                self.last_input,
                self.dirty,
                self.reader_dirty,
                self.last_preferences_save,
                self.last_reader_save,
                self.selected,
                self.hovered,
                self.search_reveal,
                self.daily,
                self.clock_minute,
                self.reading_page,
                self.browse_tab,
                self.poll_interval,
                self.notice_until,
            ) = saved;
            let restored_poll_interval = self.poll_interval;
            self.poll_interval = actual_poll_interval;
            self.set_poll_interval(restored_poll_interval);
            self.refresh_labels();
            for (id, selection) in form_selection {
                SendMessageW(
                    self.controls[&id],
                    CB_SETCURSEL,
                    Some(WPARAM(selection as usize)),
                    None,
                );
            }
            for (id, text) in form_text {
                self.set_text(id, &text);
            }
            self.resize();
            result
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_ui_listening_feedback(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let prefs = self.prefs.clone();
            let audio = self.audio.take();
            let selected = self.selected;
            self.notice_until = None;
            self.popup = None;
            self.panel = Panel::None;
            self.panel_animation = None;
            self.search = false;
            self.prefs.page = 1;
            self.selected = Some((1, 1));
            let mut cases = Vec::new();
            for arabic in [false, true] {
                self.prefs.arabic = arabic;
                self.refresh_labels();
                self.command(LISTEN, 0)?;
                self.panel_animation = None;
                self.resize();
                if self.panel != Panel::Listen || GetFocus() != self.hwnd {
                    return Err("Listen opening focused or selected its editable reciter".into());
                }
                let mut info = COMBOBOXINFO {
                    cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                    ..Default::default()
                };
                GetComboBoxInfo(self.controls[&RECITER], &mut info)?;
                // The reciter is a dropdown-list, not an EDIT. EM_GETSEL is
                // not a supported selection query for its static item window.
                if GetWindowLongPtrW(self.controls[&RECITER], GWL_STYLE) & 3
                    != CBS_DROPDOWNLIST as isize
                {
                    return Err("Reciter unexpectedly regained an editable text field".into());
                }
                for expected in [true, false] {
                    SendMessageW(
                        info.hwndItem,
                        WM_LBUTTONDOWN,
                        Some(WPARAM(1)),
                        Some(LPARAM(0x00080008)),
                    );
                    SendMessageW(info.hwndItem, WM_LBUTTONUP, None, Some(LPARAM(0x00080008)));
                    let open =
                        SendMessageW(self.controls[&RECITER], CB_GETDROPPEDSTATE, None, None).0
                            != 0;
                    if open != expected {
                        return Err("Reciter field cannot toggle its native dropdown".into());
                    }
                }
                self.command(LISTEN, 0)?;
                if self.panel != Panel::None {
                    return Err("Second Listen action did not close the panel".into());
                }
                self.panel_animation = None;
                self.resize();
                let keyboard_entry = self.qa_keyboard_entry()?;
                let fluidity = self.qa_fluidity_interaction()?;
                let geometry = |r: Rect| [r.x, r.y, r.w, r.h];
                let before = geometry(self.reader_area());
                self.notice(self.tr("Bookmark saved", "تم حفظ العلامة").to_owned());
                let during = geometry(self.reader_area());
                let timeout = self
                    .notice_until
                    .ok_or("Missing notice timeout")?
                    .saturating_duration_since(Instant::now())
                    .as_secs_f64();
                if before != during
                    || timeout > 3.01
                    || !IsWindowVisible(self.controls[&NOTICE]).as_bool()
                {
                    return Err("Brief notice changes reading geometry or timeout".into());
                }
                self.notice_until = None;
                self.resize();
                if before != geometry(self.reader_area()) {
                    return Err("Notice dismissal resizes the reader".into());
                }
                self.command(SEARCH, 0)?;
                let initial: Vec<_> = self
                    .hits
                    .iter()
                    .map(|h| (h.kind, h.surah, h.page_number))
                    .collect();
                let mut heights = Vec::new();
                for elapsed in [0, 45, 90, 135, 190] {
                    self.search_animation =
                        Some(Instant::now() - std::time::Duration::from_millis(elapsed));
                    self.advance_control_motion();
                    self.paint()?;
                    for id in [
                        BROWSE_SURAH,
                        BROWSE_PAGE,
                        BROWSE_SECTIONS,
                        QUERY,
                        BROWSE_QUERY,
                    ] {
                        if !IsWindowVisible(self.controls[&id]).as_bool() {
                            return Err("Search opening hides a browsing control".into());
                        }
                    }
                    let current: Vec<_> = self
                        .hits
                        .iter()
                        .map(|h| (h.kind, h.surah, h.page_number))
                        .collect();
                    if current != initial {
                        return Err("Search opening rebuilds its result model".into());
                    }
                    heights.push(self.placements.borrow()[&BROWSE_SURAH].0.y);
                }
                self.capture(&out.join(if arabic {
                    "native-search-arabic-stable.png"
                } else {
                    "native-search-english-stable.png"
                }))?;
                self.command(SEARCH_CLOSE, 0)?;
                self.options()?;
                self.popup.as_mut().unwrap().motion = None;
                self.resize();
                let p = self.popup.as_ref().ok_or("Missing settings panel")?;
                let expected_x = if arabic {
                    self.width - p.rect.w - 12.
                } else {
                    12.
                };
                if !p.docked
                    || (p.rect.x - expected_x).abs() > 0.01
                    || p.rtl != arabic
                    || p.rect.h < self.height - 80.
                    || geometry(self.reader_area()) != before
                {
                    return Err(
                        "Settings must overlay the language's leading edge without moving the book"
                            .into(),
                    );
                }
                self.paint()?;
                self.capture(&out.join(if arabic {
                    "native-settings-arabic-right.png"
                } else {
                    "native-settings-english-left.png"
                }))?;
                let theme_settings = self.qa_settings_theme(out)?;
                self.context_at((1, 1), self.width / 2., 140., false)?;
                let pin = self.popup.as_ref().unwrap().context_target;
                self.popup_command(MENU_ROW + 2)?;
                let start = self.popup.as_ref().unwrap().display_rect().h;
                self.popup.as_mut().unwrap().motion =
                    Some(Instant::now() - std::time::Duration::from_millis(90));
                self.resize();
                let middle = self.popup.as_ref().unwrap().display_rect().h;
                self.popup.as_mut().unwrap().motion = None;
                self.resize();
                let end = self.popup.as_ref().unwrap().display_rect().h;
                if start != middle
                    || middle != end
                    || self.popup.as_ref().unwrap().motion.is_some()
                    || self
                        .popup
                        .as_ref()
                        .unwrap()
                        .rows
                        .iter()
                        .enumerate()
                        .any(|(i, row)| {
                            row.id == BOOKMARK
                                && IsWindowVisible(self.controls[&(MENU_ROW + i as u32)]).as_bool()
                        })
                    || self.popup.as_ref().unwrap().context_target != pin
                {
                    return Err(
                        "Listening choices did not open immediately while retaining their ayah"
                            .into(),
                    );
                }
                self.paint()?;
                self.capture(&out.join(if arabic {
                    "native-listening-expanded-arabic.png"
                } else {
                    "native-listening-expanded-english.png"
                }))?;
                self.popup_command(MENU_CLOSE)?;
                cases.push(serde_json::json!({"arabic":arabic,"listenToggle":true,"reciterFieldOpenAndClose":true,"openingFocusParent":true,"keyboardEntry":keyboard_entry,"fluidity":fluidity,"noticeSeconds":timeout,"noticeGeometryUnchanged":true,"searchRowsStableDuringMotion":true,"searchTabY":heights,"settingsLeadingEdgeOverlay":true,"settingsSide":if arabic {"right"} else {"left"},"themeSettings":theme_settings,"scopeHeights":[start,middle,end],"scopeTargetPinned":true}));
            }
            self.prefs = prefs;
            self.audio = audio;
            self.selected = selected;
            self.panel = Panel::None;
            self.panel_animation = None;
            self.popup = None;
            self.search = false;
            self.search_animation = None;
            self.refresh_labels();
            self.resize();
            std::fs::write(
                out.join("ui-listening-feedback-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"cases":cases,"scope":"Production native controls and programmed animation positions; not physical-input or presented-FPS certification"}),
                )?,
            )?;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_study_experience(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            let arabic = self.prefs.arabic;
            let source = self.source;
            let language_chosen = self.prefs.language_chosen;
            let other = self.prefs.other_study_languages;
            let page = self.prefs.page;
            let mut window = RECT::default();
            GetWindowRect(self.hwnd, &mut window)?;
            self.panel = Panel::None;
            self.panel_animation = None;
            self.welcome()?;
            self.invalidate();
            self.paint()?;
            self.capture(&out.join("native-language-welcome.png"))?;
            self.popup_command(MENU_ROW)?;
            if self.prefs.arabic || !self.prefs.language_chosen || self.study_sources != vec![2] {
                return Err("English onboarding did not choose English study".into());
            }
            self.command(LANG, 0)?;
            if self.study_sources.contains(&2) {
                return Err("Arabic study lists the English source by default".into());
            }
            self.command(SEARCH, 0)?;
            self.search_animation = None;
            self.set_text(QUERY, "");
            self.set_text(BROWSE_QUERY, "البقرة");
            self.update_search()?;
            for id in [QUERY, BROWSE_QUERY] {
                if GetWindowLongPtrW(self.controls[&id], GWL_STYLE) & ES_RIGHT as isize == 0
                    || GetWindowLongPtrW(self.controls[&id], GWL_EXSTYLE)
                        & WS_EX_RTLREADING.0 as isize
                        == 0
                {
                    return Err("Arabic search control lost right-to-left alignment".into());
                }
            }
            self.invalidate();
            self.paint()?;
            self.capture(&out.join("native-arabic-search-direction.png"))?;
            self.command(SEARCH_CLOSE, 0)?;
            self.selected = Some((55, 26));
            self.context_at((55, 26), self.width / 2., 100., true)?;
            let main = self.placements.borrow()[&(MENU_ROW + 1)].0;
            let arrow = self.placements.borrow()[&(MENU_ROW + 2)].0;
            if main.y != arrow.y || (main.x < arrow.x + arrow.w && main.x + main.w > arrow.x) {
                return Err("Listening actions are not distinct split targets".into());
            }
            self.invalidate();
            self.paint()?;
            self.capture(&out.join("native-split-listening-actions.png"))?;
            self.popup_command(MENU_ROW + 2)?;
            if self
                .popup
                .as_ref()
                .is_none_or(|p| !p.listening_expanded || !p.stack.is_empty())
            {
                return Err("Listening arrow did not open nested actions".into());
            }
            self.paint()?;
            self.capture(&out.join("native-inline-listening-choices.png"))?;
            self.popup = None;
            self.panel = Panel::None;
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                1920,
                1000,
                SWP_SHOWWINDOW,
            )?;
            self.page_turn = None;
            self.page_drag = None;
            self.search_animation = None;
            let mut actual_window = RECT::default();
            GetWindowRect(self.hwnd, &mut actual_window)?;
            let mut actual_origin = POINT::default();
            let _ = ClientToScreen(self.hwnd, &mut actual_origin);
            self.prefs.page = 105;
            self.prefs.arabic = false;
            self.refresh_labels();
            self.resize();
            let before = crate::app_core::book_geometry(self.reader_area());
            self.selected = Some((5, 2));
            self.panel = Panel::Study;
            self.source = 2;
            self.study()?;
            self.resize();
            let after = crate::app_core::book_geometry(self.reader_area());
            if (before.bounds.h - after.bounds.h).abs() > 0.1 || before.single || after.single {
                return Err("Wide study layout reduced page height".into());
            }
            if self.verse_text_end != 0 {
                return Err("Study still duplicates the ayah text".into());
            }
            self.invalidate();
            self.paint()?;
            self.capture(&out.join("native-wide-english-study.png"))?;
            self.prefs.arabic = true;
            self.source = 0;
            self.refresh_labels();
            self.study()?;
            self.resize();
            self.invalidate();
            self.paint()?;
            self.capture(&out.join("native-wide-arabic-study.png"))?;
            self.panel = Panel::None;
            self.panel_animation = None;
            self.resize();
            let mut costs = vec![];
            let mut phase_samples = vec![];
            let cached_before = self.motion_cache_frames;
            let preparations_before = self.motion_cache_preparations.len();
            self.set_panel(Panel::Listen);
            for frame in 0..24 {
                self.panel_pose = Some(frame as f32 / 24.);
                self.panel_animation = Some((
                    Instant::now() - std::time::Duration::from_secs_f32(0.22 * frame as f32 / 24.),
                    Panel::None,
                    0.,
                ));
                let began = Instant::now();
                self.resize();
                let resize_ms = began.elapsed().as_secs_f64() * 1000.;
                let geometry = crate::app_core::book_geometry(self.reader_area());
                if (geometry.bounds.h - before.bounds.h).abs() > 0.1 {
                    return Err("Panel animation reshapes the wide book".into());
                }
                self.paint()?;
                costs.push(began.elapsed().as_secs_f64() * 1000.);
                phase_samples.push(serde_json::json!({"frame":frame,"resizeMs":resize_ms,"bookMs":self.motion_samples[0],"endDrawMs":self.motion_samples[1],"nativeControlsMs":self.motion_samples[2]}));
                if std::env::args().any(|arg| arg == "--qa-motion-capture") && frame % 4 == 0 {
                    self.capture(&out.join(format!("motion-panel-pose-{frame:02}.png")))?;
                }
            }
            let cached_poses = self.motion_cache_frames - cached_before;
            self.panel_animation = None;
            self.panel_pose = None;
            self.resize();
            self.invalidate();
            self.paint()?;
            self.capture(&out.join("native-wide-listening.png"))?;
            let mut info = COMBOBOXINFO {
                cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                ..Default::default()
            };
            GetComboBoxInfo(self.controls[&RECITER], &mut info)?;
            SendMessageW(
                info.hwndItem,
                WM_LBUTTONDOWN,
                Some(WPARAM(1)),
                Some(LPARAM(10 | (10 << 16))),
            );
            SendMessageW(
                info.hwndItem,
                WM_LBUTTONUP,
                None,
                Some(LPARAM(10 | (10 << 16))),
            );
            if SendMessageW(self.controls[&RECITER], CB_GETDROPPEDSTATE, None, None).0 == 0 {
                return Err("Clicking the reciter field did not open its dropdown".into());
            }
            SendMessageW(
                self.controls[&RECITER],
                CB_SHOWDROPDOWN,
                Some(WPARAM(0)),
                None,
            );
            self.panel = Panel::None;
            self.prefs.arabic = arabic;
            self.prefs.language_chosen = language_chosen;
            self.prefs.other_study_languages = other;
            self.prefs.page = page;
            self.source = source;
            self.selected = None;
            SetWindowPos(
                self.hwnd,
                Some(HWND_NOTOPMOST),
                window.left,
                window.top,
                window.right - window.left,
                window.bottom - window.top,
                SWP_SHOWWINDOW,
            )?;
            self.refresh_labels();
            self.resize();
            costs.sort_by(f64::total_cmp);
            std::fs::write(
                out.join("motion-paint-phases.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"frames":phase_samples,"cachedFrames":cached_poses,"cacheErrors":self.motion_errors,"preparationMs":self.motion_cache_preparations[preparations_before..],"scope":"Programmed full native resize/paint phases; not presented FPS. Preparations are included in their frames."}),
                )?,
            )?;
            std::fs::write(
                out.join("study-experience-verification.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":true,"languageOnboarding":true,"studySourcesFollowLanguage":true,"optionalOtherLanguages":true,"arabicSearchNativeRtl":true,"wideStudyHeightPreserved":true,"wideAnimationDoesNotReshapePages":true,"commentaryWithoutDuplicateAyah":true,"distinctSplitListeningTargets":true,"nestedListeningArrow":true,"reciterFieldClickOpensList":true,"panelLayoutPaintP95Ms":costs[22],"panelLayoutPaintSamplesMs":costs,"physicalWindow":[actual_window.left,actual_window.top,actual_window.right,actual_window.bottom],"physicalClientOrigin":[actual_origin.x,actual_origin.y],"screenSize":[GetSystemMetrics(SM_CXSCREEN),GetSystemMetrics(SM_CYSCREEN)],"scope":"Native controls, actual wide rendering and programmed layout/paint samples; not a presented FPS measurement"}),
                )?,
            )?;
            Ok(())
        }
    }
    #[cfg(feature = "qa")]
    unsafe fn qa_frame_costs(&mut self, out: &std::path::Path) -> Result<()> {
        unsafe {
            self.panel = Panel::None;
            self.panel_animation = None;
            self.search = false;
            self.text_view = false;
            self.selected = None;
            self.prefs.page = 45;
            let mut enabled = BOOL::from(true);
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some((&mut enabled as *mut BOOL).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .ok();
            let weighted = self.palette().paper.r < 0.25 && self.dpi < 144.;
            let mut cases = Vec::new();
            let mut source_counts = BTreeMap::new();
            let last_page = if self.single_page() { 47 } else { 48 };
            for page in 47..=last_page {
                for line in &self.resources.page(page)?.lines {
                    for word in &line.words {
                        *source_counts
                            .entry((word.surah, word.ayah))
                            .or_insert(0usize) += 1;
                    }
                }
            }
            let mut word_hits_checked = 0;
            for contextual in [false, true] {
                self.renderer.contextual = contextual;
                for page in 43..=48 {
                    self.renderer
                        .prepare_page(&mut self.resources, page, weighted)?;
                }
                let mut samples = Vec::new();
                for frame in 0..48 {
                    if frame == 24 {
                        self.renderer.discard_target();
                    }
                    let p = ((frame % 24) as f32 + 0.1) / 24.;
                    self.page_turn = Some(crate::page_turn::Turn {
                        first_frame: false,
                        start: Instant::now(),
                        old: 45,
                        new: 47,
                        direction: 1,
                        from: p,
                        to: p,
                        top: false,
                    });
                    let start = Instant::now();
                    self.invalidate_reader();
                    self.paint()?;
                    samples.push(start.elapsed().as_secs_f64() * 1000.);
                    let mut visible_counts = BTreeMap::new();
                    for line in &self.renderer.visible {
                        for run in line.runs.iter() {
                            *visible_counts
                                .entry((run.surah, run.ayah))
                                .or_insert(0usize) += 1;
                        }
                        let point = if let Some(regions) = &line.hits {
                            regions
                                .first()
                                .map(|(r, key)| (r.x + r.w / 2., r.y + r.h / 2., *key))
                        } else if let Some(run) = line.runs.first() {
                            let mut x = 0.;
                            let mut y = 0.;
                            let mut metrics=windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS::default();
                            line.layout.HitTestTextPosition(
                                run.start,
                                false,
                                &mut x,
                                &mut y,
                                &mut metrics,
                            )?;
                            Some((
                                // The returned point is the leading caret edge,
                                // which is the right edge for RTL. Probe the
                                // cluster rectangle, not caret + half-width.
                                line.origin.X + (metrics.left + metrics.width / 2.) * line.scale,
                                line.origin.Y + (metrics.top + metrics.height / 2.) * line.scale,
                                (run.surah, run.ayah),
                            ))
                        } else {
                            None
                        };
                        if let Some((x, y, key)) = point {
                            if self.renderer.hit(x, y) != Some(key) {
                                return Err(format!("Fold destination word hit map differs: contextual={contextual}, frame={frame}, line={}, point=({x},{y}), expected={key:?}, actual={:?}, bounds={:?}",line.line_number,self.renderer.hit(x,y),line.rect).into());
                            }
                            word_hits_checked += 1;
                        }
                    }
                    if visible_counts != source_counts {
                        return Err("Fold faces changed destination interaction metadata".into());
                    }
                }
                let mut sorted = samples.clone();
                sorted.sort_by(f64::total_cmp);
                cases.push(serde_json::json!({"contextual":contextual,"samplesMs":samples,"medianPaintMs":sorted[24],"p95PaintMs":sorted[45],"maxPaintMs":sorted[47]}));
            }
            self.page_turn = None;
            self.invalidate();
            self.paint()?;
            std::fs::write(
                out.join("hwnd-paint-costs.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "scope":"Actual HWND paint, EndDraw and child composition at programmed fold poses; not input-to-photon or compositor frame intervals",
                    "animationsEnabled":enabled.as_bool(),"dpi":self.dpi,"widthDip":self.width,"heightDip":self.height,"cases":cases
                    ,"destinationMetadataMatchesSource":true,"wordHitsChecked":word_hits_checked
                ,"softwareTargetRequested":std::env::args().any(|arg|arg=="--qa-software-target")
                }))?,
            )?;
            Ok(())
        }
    }
    /// QA captures this app's client area only, never the desktop or other apps.
    #[cfg(feature = "qa")]
    unsafe fn capture(&self, path: &std::path::Path) -> Result<()> {
        use windows::Win32::{Graphics::Imaging::*, System::Com::*};
        unsafe {
            let width = (self.width * self.dpi / 96.) as i32;
            let height = (self.height * self.dpi / 96.) as i32;
            windows::Win32::Graphics::Dwm::DwmFlush()?;
            let dc = GetDC(Some(self.hwnd));
            let memory_dc = CreateCompatibleDC(Some(dc));
            let bitmap = CreateCompatibleBitmap(dc, width, height);
            let previous = SelectObject(memory_dc, bitmap.into());
            let captured = BitBlt(memory_dc, 0, 0, width, height, Some(dc), 0, 0, SRCCOPY);
            SelectObject(memory_dc, previous);
            let _ = DeleteDC(memory_dc);
            ReleaseDC(Some(self.hwnd), dc);
            let result = (|| -> Result<()> {
                captured?;
                let wic: IWICImagingFactory =
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
                let image =
                    wic.CreateBitmapFromHBITMAP(bitmap, HPALETTE::default(), WICBitmapIgnoreAlpha)?;
                let stream = wic.CreateStream()?;
                stream.InitializeFromFilename(
                    &HSTRING::from(path.to_string_lossy().as_ref()),
                    0x40000000,
                )?;
                let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
                encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
                let mut frame = None;
                let mut options = None;
                encoder.CreateNewFrame(&mut frame, &mut options)?;
                let frame = frame.ok_or("Missing PNG frame")?;
                frame.Initialize(options.as_ref())?;
                frame.SetSize(width as u32, height as u32)?;
                let mut format = GUID_WICPixelFormat32bppBGR;
                frame.SetPixelFormat(&mut format)?;
                frame.WriteSource(&image, std::ptr::null())?;
                frame.Commit()?;
                encoder.Commit()?;
                Ok(())
            })();
            let _ = DeleteObject(bitmap.into());
            result
        }
    }
}
fn profile() -> PathBuf {
    #[cfg(feature = "qa")]
    {
        if let Some(path) = qa_argument("--qa-ui") {
            return path.join("profile");
        }
        if let Some(path) = qa_argument("--qa-control-colors") {
            return path.join("profile");
        }
        if let Ok(path) = std::env::var("QURAN_NATIVE_QA_PROFILE") {
            return PathBuf::from(path);
        }
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("QuranMushafReader/NativePreview")
}
#[cfg(feature = "qa")]
fn qa_report() -> Option<PathBuf> {
    qa_argument("--qa-report")
}
#[cfg(feature = "qa")]
fn qa_argument(flag: &str) -> Option<PathBuf> {
    let args: Vec<_> = std::env::args().collect();
    args.iter()
        .position(|s| s == flag)
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
}
/// Cancel only this app's QA popup so keyboard menu routing can be tested without
/// desktop automation or touching any other window.
#[cfg(feature = "qa")]
unsafe extern "system" fn qa_cancel_menu(hwnd: HWND, _message: u32, id: usize, _time: u32) {
    unsafe {
        let _ = KillTimer(Some(hwnd), id);
        let _ = EndMenu();
        let _ = PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(27), LPARAM(0));
    }
}
pub fn run(mut resources: Resources, _began: Instant) -> Result<()> {
    unsafe {
        // The embedded manifest normally sets this before main. Reapplying it
        // then returns E_ACCESSDENIED; that is not a startup failure.
        if let Err(error) =
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
        {
            if error.code().0 as u32 != 0x80070005 {
                return Err(error.into());
            }
        }
        let profile = profile();
        let mut prefs: Preferences =
            data::read_json_limit(&profile.join("preferences.json"), 64 * 1024).unwrap_or_default();
        prefs.sanitize();
        #[cfg(feature = "qa")]
        let previous_recording_mode = prefs.continuous_audio;
        let recording_clock_upgraded = prefs.upgrade_recording_clock();
        let normal_speed_reset = prefs.begin_recitation();
        #[cfg(feature = "qa")]
        if let Some(out) = qa_argument("--qa-ui") {
            std::fs::create_dir_all(&out)?;
            std::fs::write(
                out.join("recording-clock-startup.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"previousContinuous":previous_recording_mode,"upgraded":recording_clock_upgraded,"continuous":prefs.continuous_audio,"version":prefs.recording_clock_version}),
                )?,
            )?;
        }
        let study_source = if prefs.arabic { 0 } else { 2 };
        let mut renderer = Renderer::new(&mut resources)?;
        renderer.contextual = prefs.contextual;
        renderer.show_clock = prefs.show_clock;
        renderer.clock_24 = prefs.clock_24;
        let (reader, state_recovered) = ReaderState::load(&profile, &resources.surahs)
            .unwrap_or_else(|_| (ReaderState::default(), true));
        let instance = GetModuleHandleW(None)?;
        let icon = LoadIconW(
            Some(instance.into()),
            // Win32 encodes integer resource IDs in the low pointer bits.
            windows::core::PCWSTR(std::ptr::without_provenance(1)),
        )?;
        let class = w!("QuranMushafNativePreview");
        let registered = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: class,
            hIcon: icon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        });
        if registered == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let font = CreateFontW(
            -15,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let background = CreateSolidBrush(COLORREF(Palette::new(prefs.theme).background_rgb()));
        cache_control_colors(background, Palette::new(prefs.theme));
        let study_size = prefs.study_text_size;
        let mut app = Box::new(App {
            guide: None,
            hwnd: HWND::default(),
            resources,
            chrome: crate::chrome::Chrome::new(renderer.factory.clone())?,
            renderer,
            popup: None,
            prefs,
            profile,
            controls: BTreeMap::new(),
            placements: Default::default(),
            placed: Default::default(),
            position_batch: Cell::new(None),
            position_batch_failed: Cell::new(false),
            frame_paint_pending: false,
            page_turn: None,
            turn_queue: crate::app_core::TurnQueue::default(),
            repeating_key: false,
            keyboard_turn: false,
            tafsir_arabic: false,
            prewarm_key: None,
            prewarm_pages: std::collections::VecDeque::new(),
            page_drag: None,
            drag_start: None,
            identity_icon: HICON::default(),
            font,
            study_font: HFONT::default(),
            study_font_key: None,
            attribution_font: HFONT::default(),
            #[cfg(feature = "qa")]
            transport_fields_in_frame: false,
            transport_font: Cell::new(HFONT::default()),
            transport_font_dpi: Cell::new(0),
            attribution_font_dpi: 0,
            study_format_rect: Cell::new(None),
            quran_study_font: None,
            verse_text_end: 0,
            study_attribution_start: 0,
            study_control_dark: None,
            background,
            dpi: 96.,
            width: 1200.,
            height: 800.,
            panel: Panel::None,
            panel_animation: None,
            study_reveal_pending: false,
            speed_selection_pending: false,
            motion_book: None,
            motion_fold: None,
            motion_failed: None,
            motion_cpu: crate::motion_cache::CpuRasterCache::new(),
            last_motion_direction: 1,
            motion_controls: std::cell::RefCell::new(BTreeMap::new()),
            #[cfg(feature = "qa")]
            motion_samples: [0.; 3],
            #[cfg(feature = "qa")]
            motion_cache_preparations: vec![],
            #[cfg(feature = "qa")]
            motion_cache_frames: 0,
            #[cfg(feature = "qa")]
            motion_errors: vec![],
            #[cfg(feature = "qa")]
            popup_fields_in_frame: false,
            #[cfg(feature = "qa")]
            motion_trace: None,
            #[cfg(feature = "qa")]
            motion_live_runs: vec![],
            #[cfg(feature = "qa")]
            last_layout_ms: 0.,
            #[cfg(feature = "qa")]
            last_commit_ms: 0.,
            #[cfg(feature = "qa")]
            layout_generation: 0,
            #[cfg(feature = "qa")]
            painted_layout_generation: 0,
            #[cfg(feature = "qa")]
            maximum_motion_pixel_bytes: 0,
            #[cfg(feature = "qa")]
            panel_pose: None,
            search: false,
            browse_tab: BROWSE_SURAH,
            search_animation: None,
            title_animation: None,
            #[cfg(feature = "qa")]
            qa_menu_hot: None,
            keyboard_focus: false,
            player_collapsed: false,
            opening_highlight: None,
            tooltips: None,
            chrome_failures: Cell::new(0),
            #[cfg(feature = "qa")]
            canvas_paints: 0,
            #[cfg(feature = "qa")]
            title_trace: None,
            hits: vec![],
            results_generation: 0,
            selected: None,
            hovered: None,
            search_reveal: None,
            playback: crate::app_core::PlaybackFollow::default(),
            repeats: crate::app_core::RepeatCount::new(1),
            active_range: None,
            listening_session: None,
            audio: None,
            audio_cache: None,
            download: None,
            download_generation: 0,
            storage_pending: None,
            tafsir_library: None,
            tafsir_job: None,
            tafsir_job_source: None,
            tafsir_generation: 0,
            installed_tafsirs: Vec::new(),
            study_sources: vec![0, 1, 2],
            reader,
            reader_dirty: false,
            last_reader_save: Instant::now(),
            last_preferences_save: Instant::now(),
            last_input: Instant::now(),
            reading_page: 0,
            reading_pages: Vec::new(),
            poll_interval: 1000,
            text_view: false,
            text_pages: Vec::new(),
            panel_scroll: 0.,
            listen_scroll_geometry: None,
            clock_minute: u16::MAX,
            fullscreen: false,
            placement: WINDOWPLACEMENT::default(),
            window_style: 0,
            theme_start: None,
            theme_frame: None,
            source: study_source,
            arabic_source: 0,
            reciter: 0,
            study_size,
            dirty: recording_clock_upgraded || normal_speed_reset,
            daily: None,
            first_paint: false,
            startup_reveal: None,
            notice_until: None,
            #[cfg(feature = "qa")]
            began: _began,
            #[cfg(feature = "qa")]
            qa_step: 0,
            #[cfg(feature = "qa")]
            first_paint_ms: 0.,
            #[cfg(feature = "qa")]
            qa_context: None,
            #[cfg(feature = "qa")]
            qa_original_rect: RECT::default(),
            #[cfg(feature = "qa")]
            large_handoff: None,
        });
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Quran Mushaf Reader — Native Preview"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1280,
            900,
            None,
            None,
            Some(instance.into()),
            Some((&mut *app as *mut App).cast()),
        )?;
        let _window = WindowLifetime(hwnd);
        app.hwnd = hwnd;
        // Force the initial non-client calculation before showing the custom
        // caption. Restoring/maximizing must never be needed to repair it.
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
        {
            let _guard = DispatchGuard::enter();
            app.controls()?;
        }
        #[cfg(feature = "qa")]
        if std::env::args().any(|argument| argument == "--qa-fail-window") {
            return Err("Intentional QA failure after creating the window and controls".into());
        }
        let dark = BOOL::from(app.prefs.theme == 0 || app.prefs.theme == 3);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const BOOL).cast(),
            std::mem::size_of::<BOOL>() as u32,
        );
        if client_animation_enabled() && !cfg!(feature = "qa") {
            app.startup_reveal = Some(Instant::now());
            SetTimer(Some(hwnd), 6, 10, None);
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        #[cfg(feature = "qa")]
        if qa_argument("--qa-ui").is_some() || qa_argument("--qa-control-colors").is_some() {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        focus_window(hwnd)?;
        if state_recovered {
            app.notice(
                app.tr(
                    "Saved reader settings could not be loaded. Defaults are in use.",
                    "تعذر تحميل إعدادات القراءة المحفوظة. تُستخدم الإعدادات الافتراضية.",
                )
                .to_owned(),
            );
        } else if !app.reader.onboarding_completed {
            app.notice(app.tr("Welcome. Right-click an ayah for tafsir and listening. More help is in Options.", "مرحبًا بك. انقر بزر الفأرة الأيمن على آية للتفسير أو الاستماع. تجد المزيد في الخيارات.").to_owned());
            app.reader.onboarding_completed = true;
            app.reader_dirty = true;
        }
        if !app.prefs.language_chosen {
            #[cfg(feature = "qa")]
            let automatic_welcome =
                qa_argument("--qa-ui").is_none() && qa_argument("--qa-control-colors").is_none();
            #[cfg(not(feature = "qa"))]
            let automatic_welcome = true;
            if automatic_welcome {
                app.welcome()?;
            }
        }
        SetTimer(Some(hwnd), 1, 1000, None);
        #[cfg(feature = "qa")]
        if qa_argument("--qa-ui").is_some() || qa_argument("--qa-control-colors").is_some() {
            SetTimer(Some(hwnd), 3, 900, None);
        }
        let precise_clock = !qa_disabled("--qa-legacy-clock");
        let mut frame_clock = if precise_clock {
            crate::frame_clock::FrameClock::new().ok()
        } else {
            None
        };
        let clock_period = std::time::Duration::from_micros(
            if frame_clock
                .as_ref()
                .is_some_and(|clock| clock.high_resolution())
            {
                8333
            } else {
                16000
            },
        );
        #[cfg(feature = "qa")]
        if let Some(out) = qa_argument("--qa-ui") {
            std::fs::create_dir_all(&out)?;
            std::fs::write(
                out.join("frame-scheduler.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"preciseRequested":precise_clock,"available":frame_clock.is_some(),"highResolution":frame_clock.as_ref().is_some_and(|clock|clock.high_resolution()),"periodMicroseconds":clock_period.as_micros(),"activeOnlyDuringVisibleMotion":true,"missedFramesCoalesced":true}),
                )?,
            )?;
        }
        let mut message = MSG::default();
        loop {
            if frame_clock.is_some() && app.page_turn.is_none() {
                if let Some(request) = app.next_queued_turn() {
                    let _guard = DispatchGuard::enter();
                    app.keyboard_turn = request.source == crate::app_core::TurnSource::Keyboard;
                    if let Err(error) = app.navigate(request.direction) {
                        app.report_error(&error);
                    }
                }
            }
            if let Some(clock) = &mut frame_clock {
                let controls = app.startup_reveal.is_some()
                    || app.panel_animation.is_some()
                    || app.title_animation.is_some()
                    || app.search_animation.is_some()
                    || app.popup.as_ref().is_some_and(|p| p.motion.is_some());
                let pages = app.page_turn.is_some();
                let themes = app.theme_start.is_some();
                let motion = (controls || pages || themes) && !IsIconic(hwnd).as_bool();
                let armed = if motion {
                    clock.arm(clock_period)
                } else {
                    clock.disarm()
                };
                if armed.is_err() {
                    frame_clock = None;
                    if controls {
                        SetTimer(Some(hwnd), 6, 10, None);
                    }
                    if pages {
                        SetTimer(Some(hwnd), 4, 10, None);
                    }
                    if themes {
                        SetTimer(Some(hwnd), 2, 10, None);
                    }
                    continue;
                }
                if motion {
                    KillTimer(Some(hwnd), 4).ok();
                    KillTimer(Some(hwnd), 6).ok();
                    KillTimer(Some(hwnd), 2).ok();
                }
                match clock.next_event() {
                    Ok(crate::frame_clock::FrameEvent::Tick) => {
                        message = MSG {
                            hwnd,
                            message: FRAME_TICK,
                            ..Default::default()
                        };
                    }
                    Ok(crate::frame_clock::FrameEvent::Message(next)) => {
                        message = next;
                    }
                    Ok(crate::frame_clock::FrameEvent::Quit(code)) => {
                        let _ = code;
                        break;
                    }
                    Err(_) => {
                        frame_clock = None;
                        if controls {
                            SetTimer(Some(hwnd), 6, 10, None);
                        }
                        if pages {
                            SetTimer(Some(hwnd), 4, 10, None);
                        }
                        if themes {
                            SetTimer(Some(hwnd), 2, 10, None);
                        }
                        continue;
                    }
                }
            } else {
                match GetMessageW(&mut message, None, 0, 0).0 {
                    -1 => return Err(windows::core::Error::from_thread().into()),
                    0 => break,
                    _ => {}
                }
            }
            // Include owned child controls and native dropdown windows, not
            // just the parent canvas, when deferring optional maintenance.
            if matches!(
                message.message,
                WM_MOUSEMOVE
                    | WM_LBUTTONDOWN
                    | WM_LBUTTONUP
                    | WM_RBUTTONDOWN
                    | WM_MOUSEWHEEL
                    | WM_KEYDOWN
                    | WM_SYSKEYDOWN
                    | WM_CHAR
            ) {
                app.last_input = Instant::now();
            }
            if message.message == WM_KEYUP && matches!(message.wParam.0, 37 | 39) {
                app.turn_queue.release_keyboard();
                if app.keyboard_turn {
                    if let Some(turn) = &mut app.page_turn {
                        turn.release();
                    }
                    app.keyboard_turn = false;
                }
            }
            if matches!(message.message, WM_LBUTTONDOWN | WM_RBUTTONDOWN) && app.keyboard_focus {
                app.keyboard_focus = false;
                app.invalidate();
            }
            if message.message == WM_KEYDOWN {
                if !app.keyboard_focus {
                    app.keyboard_focus = true;
                    app.invalidate();
                }
                let handled = {
                    let _guard = DispatchGuard::enter();
                    app.repeating_key = message.lParam.0 & (1 << 30) != 0;
                    let handled = match app.key(message.wParam.0 as u16) {
                        Ok(handled) => handled,
                        Err(error) => {
                            app.report_error(&error);
                            true
                        }
                    };
                    app.repeating_key = false;
                    handled
                };
                if handled {
                    continue;
                }
            }
            if message.message == FRAME_TICK || !IsDialogMessageW(hwnd, &message).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }
}
unsafe extern "system" fn result_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        if message == WM_MOUSEWHEEL {
            let delta = (wparam.0 >> 16) as u16 as i16;
            let rows = RESULT_WHEEL.with(|wheel| wheel.borrow_mut().rows(delta));
            if rows != 0 {
                let top = SendMessageW(hwnd, LB_GETTOPINDEX, None, None).0 as i32;
                let count = SendMessageW(hwnd, LB_GETCOUNT, None, None).0 as i32;
                SendMessageW(
                    hwnd,
                    LB_SETTOPINDEX,
                    Some(WPARAM((top + rows).clamp(0, (count - 1).max(0)) as usize)),
                    None,
                );
            }
            return LRESULT(0);
        }
        if matches!(message, LB_RESETCONTENT | WM_NCDESTROY) {
            RESULT_WHEEL.with(|wheel| *wheel.borrow_mut() = crate::list_scroll::WheelRows::new());
        }
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        if message == WM_LBUTTONUP {
            let index = SendMessageW(hwnd, LB_GETCURSEL, None, None).0;
            let mut rect = RECT::default();
            let valid = index >= 0
                && SendMessageW(
                    hwnd,
                    LB_GETITEMRECT,
                    Some(WPARAM(index as usize)),
                    Some(LPARAM((&mut rect as *mut RECT) as isize)),
                )
                .0 != LB_ERR as isize;
            let x = (lparam.0 as u16 as i16) as i32;
            let y = ((lparam.0 >> 16) as u16 as i16) as i32;
            if valid && x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom {
                let _ = PostMessageW(
                    Some(GetParent(hwnd).unwrap_or_default()),
                    WM_COMMAND,
                    WPARAM(RESULT_PICK as usize),
                    LPARAM(0),
                );
            }
        }
        result
    }
}
/// Keep native IME composition independent of App's mutable event dispatch.
unsafe extern "system" fn query_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        if matches!(GetDlgCtrlID(hwnd) as u32, QUERY | BROWSE_QUERY) {
            match message {
                WM_IME_STARTCOMPOSITION => QUERY_COMPOSING.with(|state| state.set(true)),
                WM_IME_ENDCOMPOSITION | WM_KILLFOCUS => {
                    QUERY_COMPOSING.with(|state| state.set(false))
                }
                _ => {}
            }
        }
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        if matches!(message, WM_SETFOCUS | WM_KILLFOCUS) {
            // Parent-drawn outlines follow actual native keyboard focus. Paint
            // only this field's frame; editing and IME remain inside Windows.
            if let Ok(parent) = GetParent(hwnd) {
                let mut bounds = RECT::default();
                let mut origin = POINT::default();
                if GetWindowRect(hwnd, &mut bounds).is_ok()
                    && ClientToScreen(parent, &mut origin).as_bool()
                {
                    let outset = (14. * GetDpiForWindow(hwnd).max(96) as f32 / 96.).ceil() as i32;
                    bounds.left -= origin.x + outset;
                    bounds.right += outset - origin.x;
                    bounds.top -= origin.y + outset;
                    bounds.bottom += outset - origin.y;
                    let _ = InvalidateRect(Some(parent), Some(&bounds), false);
                }
            }
        }
        result
    }
}
/// Hover paint is independent of App; synchronous child callbacks never borrow
/// its mutable state. The native button still owns keyboard/focus/accessibility.
unsafe extern "system" fn button_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    command: usize,
) -> LRESULT {
    unsafe {
        let menu = (MENU_ROW..=MENU_CLOSE).contains(&(command as u32));
        let mut repaint_menu = false;
        match message {
            WM_ERASEBKGND if (MENU_ROW..=MENU_CLOSE).contains(&(command as u32)) => {
                return LRESULT(1);
            }
            WM_PAINT if (MENU_ROW..=MENU_CLOSE).contains(&(command as u32)) => {
                let mut paint = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut paint);
                let _ = EndPaint(hwnd, &paint);
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                let mut tracked = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_QUERY,
                    ..Default::default()
                };
                let _ = TrackMouseEvent(&mut tracked);
                let entering =
                    tracked.hwndTrack != hwnd || tracked.dwFlags & TME_LEAVE != TME_LEAVE;
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut track);
                if entering {
                    repaint_menu = true;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            NATIVE_MOUSE_LEAVE => {
                repaint_menu = true;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            WM_SETCURSOR if !matches!(command as u32, MINIMIZE | MAXIMIZE | WINDOW_CLOSE) => {
                if let Ok(cursor) = LoadCursorW(None, IDC_HAND) {
                    SetCursor(Some(cursor));
                    return LRESULT(1);
                }
            }
            _ => {}
        }
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        if menu
            && (repaint_menu
                || matches!(
                    message,
                    WM_SETFOCUS
                        | WM_KILLFOCUS
                        | WM_LBUTTONDOWN
                        | WM_LBUTTONUP
                        | WM_KEYDOWN
                        | WM_KEYUP
                        | BM_SETSTATE
                ))
        {
            if let Ok(parent) = GetParent(hwnd) {
                // Menu rows share the parent frame. Repaint just this row,
                // preserving native fields, title chrome and notices elsewhere.
                let mut bounds = RECT::default();
                let mut origin = POINT::default();
                if GetWindowRect(hwnd, &mut bounds).is_ok()
                    && ClientToScreen(parent, &mut origin).as_bool()
                {
                    bounds.left -= origin.x;
                    bounds.right -= origin.x;
                    bounds.top -= origin.y;
                    bounds.bottom -= origin.y;
                    let _ = InvalidateRect(Some(parent), Some(&bounds), false);
                }
            }
        }
        result
    }
}
unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        // These synchronous callbacks are safe even when DispatchGuard is
        // active: they use copied colors and a brush owned by the UI thread.
        if matches!(
            message,
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN
        ) {
            let colors = CONTROL_COLORS.with(Cell::get);
            if colors.brush != 0 {
                let dc = HDC(wparam.0 as *mut c_void);
                SetTextColor(dc, COLORREF(colors.ink));
                SetBkColor(dc, COLORREF(colors.background));
                return LRESULT(colors.brush as isize);
            }
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        // DestroyWindow sends these synchronously while WM_CLOSE owns App.
        // They must still run through the reentrancy gate without borrowing it.
        if message == WM_DESTROY {
            KillTimer(Some(hwnd), 1).ok();
            KillTimer(Some(hwnd), 2).ok();
            KillTimer(Some(hwnd), 4).ok();
            #[cfg(feature = "qa")]
            KillTimer(Some(hwnd), 3).ok();
            #[cfg(feature = "qa")]
            KillTimer(Some(hwnd), 5).ok();
            PostQuitMessage(0);
            return LRESULT(0);
        }
        if message == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            let app = &mut *(create.lpCreateParams as *mut App);
            app.hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        if message == WM_NCCALCSIZE {
            // Creation uses a RECT, subsequent frame changes use PARAMS.
            let rect = if wparam.0 != 0 {
                &mut (*(lparam.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0]
            } else {
                &mut *(lparam.0 as *mut RECT)
            };
            if IsZoomed(hwnd).as_bool() {
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                if GetMonitorInfoW(monitor, &mut info).as_bool() {
                    *rect = info.rcWork;
                }
            }
            return LRESULT(0);
        }
        if message == WM_NCHITTEST {
            let edge = DefWindowProcW(hwnd, message, wparam, lparam);
            if matches!(
                edge.0 as u32,
                HTLEFT
                    | HTRIGHT
                    | HTTOP
                    | HTBOTTOM
                    | HTTOPLEFT
                    | HTTOPRIGHT
                    | HTBOTTOMLEFT
                    | HTBOTTOMRIGHT
            ) {
                return edge;
            }
            let mut point = POINT {
                x: (lparam.0 & 0xffff) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut point);
            if !FULLSCREEN_STATE.with(Cell::get)
                && point.y >= 0
                && point.y < (36 * GetDpiForWindow(hwnd).max(96) / 96) as i32
            {
                return LRESULT(HTCAPTION as isize);
            }
            return LRESULT(HTCLIENT as isize);
        }
        let Some(_guard) = DispatchGuard::enter() else {
            if matches!(message, WM_SIZE | WM_DPICHANGED) {
                let _ = PostMessageW(Some(hwnd), REPAIR_LAYOUT, WPARAM(0), LPARAM(0));
            }
            return DefWindowProcW(hwnd, message, wparam, lparam);
        };
        let app = &mut *pointer;
        if matches!(
            message,
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_MOUSEWHEEL
        ) {
            app.last_input = Instant::now();
        }
        let result: Result<()> = match message {
            WM_PAINT => {
                app.frame_paint_pending = false;
                app.paint()
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_ACTIVATE if wparam.0 & 0xffff == WA_INACTIVE as usize => {
                if let Some(tips) = &app.tooltips {
                    tips.pop();
                }
                app.turn_queue.clear();
                if !app.popup.as_ref().is_some_and(|popup| {
                    popup
                        .rows
                        .iter()
                        .any(|row| matches!(row.id, WELCOME_ENGLISH | WELCOME_ARABIC))
                }) {
                    app.popup = None;
                }
                app.drag_start = None;
                app.page_drag = None;
                app.resize();
                Ok(())
            }
            REPAIR_LAYOUT => {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let dpi = GetDpiForWindow(hwnd).max(96) as f32;
                if !app.controls.is_empty()
                    && (dpi != app.dpi
                        || (client.right as f32 * 96. / dpi - app.width).abs() > 0.5
                        || (client.bottom as f32 * 96. / dpi - app.height).abs() > 0.5)
                {
                    app.resize();
                }
                return LRESULT(0);
            }
            WM_SIZE | WM_EXITSIZEMOVE => {
                app.turn_queue.clear();
                if !app.controls.is_empty() {
                    app.resize();
                }
                return LRESULT(0);
            }
            WM_DPICHANGED => {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                if !app.controls.is_empty() {
                    app.resize();
                    app.refresh_study_font();
                }
                return LRESULT(0);
            }
            WM_COMMAND => app.command((wparam.0 & 0xffff) as u32, (wparam.0 >> 16) as u32),
            WM_MEASUREITEM => {
                let item = &mut *(lparam.0 as *mut windows::Win32::UI::Controls::MEASUREITEMSTRUCT);
                if item.CtlID == RESULTS {
                    item.itemHeight = (56. * app.dpi / 96.).round() as u32;
                    return LRESULT(1);
                }
                Ok(())
            }
            WM_DRAWITEM => {
                let item = &*(lparam.0 as *const DRAWITEMSTRUCT);
                if (MENU_ROW..=MENU_CLOSE).contains(&item.CtlID) {
                    return LRESULT(1);
                }
                app.draw_button(item);
                return LRESULT(1);
            }
            #[cfg(feature = "qa")]
            QA_HEARTBEAT => {
                if let Some(test) = &mut app.large_handoff {
                    test.ack_heartbeat();
                }
                Ok(())
            }
            FRAME_TICK => {
                if !app.frame_paint_pending {
                    app.advance_theme(Instant::now());
                    if app.startup_reveal.is_some()
                        || app.panel_animation.is_some()
                        || app.title_animation.is_some()
                        || app.search_animation.is_some()
                        || app.popup.as_ref().is_some_and(|p| p.motion.is_some())
                    {
                        app.advance_control_motion();
                    }
                    if app.page_turn.is_some() {
                        app.invalidate_reader();
                    }
                    // A posted paint takes its normal BeginPaint path while
                    // avoiding starvation behind repeated low-priority timers.
                    app.frame_paint_pending =
                        PostMessageW(Some(hwnd), WM_PAINT, WPARAM(0), LPARAM(0)).is_ok();
                }
                Ok(())
            }
            WM_TIMER => {
                if wparam.0 == 9 {
                    KillTimer(Some(hwnd), 9).ok();
                    if app
                        .search_reveal
                        .is_some_and(|(start, _)| start.elapsed().as_secs() >= 5)
                    {
                        app.search_reveal = None;
                        app.invalidate_reader();
                    } else if let Some((start, _)) = app.search_reveal {
                        let remaining =
                            std::time::Duration::from_secs(5).saturating_sub(start.elapsed());
                        SetTimer(Some(hwnd), 9, remaining.as_millis().max(1) as u32, None);
                    }
                    return LRESULT(0);
                }
                if wparam.0 == 8 {
                    KillTimer(Some(hwnd), 8).ok();
                    if app.search {
                        if let Err(error) = app.update_search() {
                            app.report_error(&error);
                        }
                    }
                    return LRESULT(0);
                }
                #[cfg(feature = "qa")]
                if wparam.0 == 3 {
                    let result = if qa_argument("--qa-control-colors").is_some() {
                        app.qa_color_tick()
                    } else {
                        app.qa_tick()
                    };
                    return match result {
                        Ok(()) => LRESULT(0),
                        Err(_) => LRESULT(0),
                    };
                }
                if wparam.0 == 7 {
                    if app.page_turn.is_some()
                        || app.page_drag.is_some()
                        || app.panel_animation.is_some()
                        || app.title_animation.is_some()
                        || app.search_animation.is_some()
                        || app.popup.as_ref().is_some_and(|p| p.motion.is_some())
                    {
                        KillTimer(Some(hwnd), 7).ok();
                    } else {
                        if !app.maintenance_ready() {
                            return LRESULT(0);
                        }
                        let prepared_motion = app.prepare_motion_idle().unwrap_or(false);
                        let mut prepared_page = false;
                        if let Some(page) = (!prepared_motion)
                            .then(|| app.prewarm_pages.pop_front())
                            .flatten()
                        {
                            let weighted = app.palette().paper.r < 0.25 && app.dpi < 144.;
                            // Timer messages have lower priority than input.
                            // Optional warmup errors remain paint fallbacks.
                            let _ = app
                                .renderer
                                .prepare_page(&mut app.resources, page, weighted);
                            prepared_page = true;
                        }
                        // One bounded unit per idle dispatch; do not concatenate
                        // a whole-page preparation and a full raster allocation.
                        if app.prewarm_pages.is_empty() && !prepared_page && !prepared_motion {
                            KillTimer(Some(hwnd), 7).ok();
                        }
                    }
                    Ok(())
                } else if wparam.0 == 6 {
                    app.advance_control_motion();
                    Ok(())
                } else if wparam.0 == 4 {
                    if app.page_turn.is_some() {
                        app.invalidate_reader();
                        Ok(())
                    } else if let Some(request) = app.next_queued_turn() {
                        app.navigate(request.direction)
                    } else {
                        KillTimer(Some(hwnd), 4).ok();
                        Ok(())
                    }
                } else if wparam.0 == 2 {
                    if !app.advance_theme(Instant::now()) {
                        KillTimer(Some(hwnd), 2).ok();
                    }
                    Ok(())
                } else {
                    app.tick()
                }
            }
            WM_HSCROLL if lparam.0 == app.controls[&AUDIO_POSITION].0 as isize => {
                let code = wparam.0 & 0xffff;
                if !matches!(code, 5 | 8) {
                    let value = SendMessageW(app.controls[&AUDIO_POSITION], TBM_GETPOS, None, None)
                        .0
                        .max(0) as u32;
                    app.seek_audio(None, value)
                } else {
                    Ok(())
                }
            }
            WM_HSCROLL if lparam.0 == app.controls[&VOLUME].0 as isize => app.command(VOLUME, 0),
            WM_VSCROLL if app.panel == Panel::Listen => {
                let mut info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_ALL,
                    ..Default::default()
                };
                GetScrollInfo(hwnd, SB_VERT, &mut info).ok();
                match (wparam.0 & 0xffff) as i32 {
                    0 => app.panel_scroll -= 36.,
                    1 => app.panel_scroll += 36.,
                    2 => app.panel_scroll -= 200.,
                    3 => app.panel_scroll += 200.,
                    4 | 5 => app.panel_scroll = info.nTrackPos as f32,
                    6 => app.panel_scroll = 0.,
                    7 => app.panel_scroll = info.nMax as f32,
                    _ => {}
                }
                app.resize();
                Ok(())
            }
            WM_MOUSEWHEEL if app.panel == Panel::Listen => {
                let steps = ((wparam.0 >> 16) as i16) as f32 / 120.;
                let mut point = POINT::default();
                GetCursorPos(&mut point).ok();
                if !ScreenToClient(hwnd, &mut point).as_bool() {
                    return LRESULT(0);
                }
                let x = point.x as f32 * 96. / app.dpi;
                if x >= app.panel_origin() {
                    let max = ListenLayout::new(app.height).scroll_max(
                        app.height,
                        app.audio.as_ref().is_some_and(Audio::has_session),
                    );
                    let scroll = (app.panel_scroll - steps * 36.).clamp(0., max);
                    if (scroll - app.panel_scroll).abs() > 0.01 {
                        app.panel_scroll = scroll;
                        app.resize();
                    }
                }
                Ok(())
            }
            WM_RBUTTONUP => {
                let x = (lparam.0 & 0xffff) as i16 as f32 * 96. / app.dpi;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32 * 96. / app.dpi;
                app.context(x, y)
            }
            WM_LBUTTONDOWN => {
                if let Some(p) = &app.popup {
                    let x = (lparam.0 & 0xffff) as i16 as f32 * 96. / app.dpi;
                    let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32 * 96. / app.dpi;
                    if !p.contains(x, y) {
                        app.popup = None;
                        app.resize();
                        SetFocus(Some(hwnd)).ok();
                    }
                    return LRESULT(0);
                }
                let x = (lparam.0 & 0xffff) as i16 as f32 * 96. / app.dpi;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32 * 96. / app.dpi;
                let bounds = crate::page_turn::bounds(app.reader_area());
                if app.panel == Panel::None
                    && !app.search
                    && !app.text_view
                    && x >= bounds.x
                    && x <= bounds.x + bounds.w
                    && y >= bounds.y
                    && y <= bounds.y + bounds.h
                {
                    app.drag_start = Some((x, y));
                    app.page_turn = None;
                    app.turn_queue.clear();
                    SetCapture(hwnd);
                }
                Ok(())
            }
            WM_MOUSEMOVE if app.drag_start.is_some() => {
                app.move_page_drag(
                    (lparam.0 & 0xffff) as i16 as f32 * 96. / app.dpi,
                    ((lparam.0 >> 16) & 0xffff) as i16 as f32 * 96. / app.dpi,
                );
                Ok(())
            }
            WM_MOUSEMOVE => {
                let x = (lparam.0 & 0xffff) as i16 as f32 * 96. / app.dpi;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32 * 96. / app.dpi;
                app.update_hover(Some((x, y)));
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track).ok();
                Ok(())
            }
            NATIVE_MOUSE_LEAVE => {
                app.update_hover(None);
                Ok(())
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                app.turn_queue.clear();
                app.drag_start = None;
                app.page_drag = None;
                app.invalidate();
                Ok(())
            }
            WM_LBUTTONUP => {
                let x = (lparam.0 & 0xffff) as i16 as f32 * 96. / app.dpi;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32 * 96. / app.dpi;
                let had_press = app.drag_start.take().is_some();
                let handled = app.finish_page_drag();
                if GetCapture() == hwnd {
                    ReleaseCapture().ok();
                }
                if !handled.unwrap_or_else(|e| {
                    app.report_error(&e);
                    true
                }) && had_press
                    && !app.text_view
                {
                    app.selected = app.renderer.hit(x, y);
                }
                SetFocus(Some(hwnd)).ok();
                app.invalidate();
                Ok(())
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT {
                    x: (800. * app.dpi / 96.) as i32,
                    y: (600. * app.dpi / 96.) as i32,
                };
                return LRESULT(0);
            }
            WM_CLOSE => {
                let _ = app.tick();
                let _ = app.reader.save(&app.profile);
                if app.dirty {
                    let _ = reader_state::ensure_profile_directory(&app.profile);
                    let _ = reader_state::save_json_atomic(
                        &app.profile.join("preferences.json"),
                        &app.prefs,
                        64 * 1024,
                    );
                }
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            _ => return DefWindowProcW(hwnd, message, wparam, lparam),
        };
        if let Err(error) = result {
            if !app.controls.is_empty() {
                app.report_error(&error);
            } else {
                MessageBoxW(
                    Some(hwnd),
                    &HSTRING::from(error.to_string()),
                    w!("Native preview"),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_control_colors_survive_reentrant_dispatch() {
        unsafe {
            let _guard = DispatchGuard::enter().expect("test owns the UI-thread guard");
            let previous = CONTROL_COLORS.with(Cell::get);
            let dc = CreateCompatibleDC(None);
            assert!(!dc.is_invalid());
            for theme in 0..4 {
                let palette = Palette::new(theme);
                let brush = CreateSolidBrush(COLORREF(palette.background_rgb()));
                assert!(!brush.is_invalid());
                cache_control_colors(brush, palette);
                for message in [
                    WM_CTLCOLORSTATIC,
                    WM_CTLCOLOREDIT,
                    WM_CTLCOLORLISTBOX,
                    WM_CTLCOLORBTN,
                ] {
                    // The production procedure must answer before consulting
                    // App or rejecting this nested synchronous notification.
                    let response =
                        procedure(HWND::default(), message, WPARAM(dc.0 as usize), LPARAM(0));
                    assert_eq!(response.0, brush.0 as isize);
                    assert_eq!(GetBkColor(dc).0, palette.background_rgb());
                    assert_eq!(GetTextColor(dc).0, palette.ink_rgb());
                    assert!(DISPATCHING.with(Cell::get));
                }
                CONTROL_COLORS.with(|colors| colors.set(ControlColors::EMPTY));
                assert!(DeleteObject(brush.into()).as_bool());
            }
            CONTROL_COLORS.with(|colors| colors.set(previous));
            assert!(DeleteDC(dc).as_bool());
        }
    }

    #[test]
    fn qa_restores_physical_size_and_fits_monitor_work_area() {
        let original = RECT {
            left: 100,
            top: 100,
            right: 1380,
            bottom: 1000,
        };
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        assert_eq!(fit_window_rect(original, work), original);
        let small_work = RECT {
            left: -1024,
            top: 20,
            right: 0,
            bottom: 748,
        };
        assert_eq!(fit_window_rect(original, small_work), small_work);
        let offscreen = RECT {
            left: 1800,
            top: 500,
            right: 3080,
            bottom: 1400,
        };
        assert_eq!(
            fit_window_rect(offscreen, work),
            RECT {
                left: 640,
                top: 140,
                right: 1920,
                bottom: 1040
            }
        );
    }

    #[test]
    fn compact_listening_controls_remain_above_status() {
        for height in [720., 900.] {
            let layout = ListenLayout::new(height);
            assert!(layout.bottom(true) <= height - 35.1 + 0.2);
            assert!(layout.y(350.) + layout.button_height() < layout.y(392.));
        }
    }
}
