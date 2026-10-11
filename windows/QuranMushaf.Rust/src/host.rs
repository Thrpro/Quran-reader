//! Windows/COM boundary. Callbacks hold Weak references; the UI thread owns all
//! handles and releases the controller before COM shutdown. No filesystem IPC.
use crate::{
    logging,
    protocol::{self, Language, Message, Theme, WindowAction},
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::PathBuf,
    rc::Rc,
    time::Instant,
};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HINSTANCE, HWND, LPARAM,
            LRESULT, POINT, RECT, WPARAM,
        },
        Graphics::{Dwm::*, Gdi::*},
        System::{Com::*, LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
        UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::{BOOL, Interface, PCWSTR, PWSTR, w},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const CLASS: PCWSTR = w!("QuranMushafReader.Rust");
const COMMAND: u32 = WM_APP + 1;
const RECOVERY: u32 = WM_APP + 2;
const TOGGLE_FULLSCREEN: u32 = WM_APP + 3;
const MAX_COMMANDS: usize = 64;
thread_local! { static HOST: RefCell<Option<Rc<Host>>> = const { RefCell::new(None) }; }

struct Config {
    web_app: PathBuf,
    profile: PathBuf,
    qa_port: Option<u16>,
}
struct Host {
    hwnd: HWND,
    config: Config,
    controller: RefCell<Option<ICoreWebView2Controller>>,
    last_bounds: Cell<RECT>,
    last_maximized: Cell<Option<bool>>,
    commands: RefCell<VecDeque<Message>>,
    fullscreen: Cell<bool>,
    placement: RefCell<Option<WINDOWPLACEMENT>>,
    generation: Cell<u64>,
    closed: Cell<bool>,
    recovery_pending: Cell<bool>,
    language: Cell<Language>,
    session: Cell<bool>,
    theme: Cell<Theme>,
    started: Instant,
    document_ready: Cell<bool>,
}

struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}
struct MutexHandle(HANDLE);
impl Drop for MutexHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn config() -> Result<Config> {
    let base = std::env::current_exe()?
        .parent()
        .ok_or("Missing application directory")?
        .to_owned();
    let local = PathBuf::from(
        std::env::var_os("LOCALAPPDATA").ok_or("Windows local application data is unavailable")?,
    );
    #[allow(unused_mut)]
    let mut result = Config {
        web_app: base.join("WebApp"),
        profile: local.join("QuranMushafReader").join("WebView2"),
        qa_port: None,
    };
    #[cfg(feature = "qa")]
    {
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--qa-profile" => {
                    result.profile = PathBuf::from(args.next().ok_or("Missing QA profile")?)
                }
                "--qa-port" => {
                    result.qa_port = Some(args.next().ok_or("Missing QA port")?.parse::<u16>()?)
                }
                _ => return Err("Unknown QA argument".into()),
            }
        }
        if !result.profile.is_absolute() {
            return Err("QA profile must be absolute".into());
        }
        if result.qa_port.is_some()
            && result.profile == local.join("QuranMushafReader").join("WebView2")
        {
            return Err("Remote debugging requires a separate QA profile".into());
        }
    }
    #[cfg(not(feature = "qa"))]
    if std::env::args_os().len() > 1 {
        return Err("This release does not accept debugging arguments".into());
    }
    for relative in [
        "index.html",
        "data/qul/processed/page-index.json",
        "fonts/digital-khatt/DigitalKhattV2.otf",
    ] {
        if !result.web_app.join(relative).is_file() {
            return Err(format!(
                "Reader files are missing ({relative}). Please reinstall the app."
            )
            .into());
        }
    }
    std::fs::create_dir_all(&result.profile)?;
    Ok(result)
}

pub fn run() -> Result<()> {
    let config = config()?;
    logging::initialize(config.profile.parent().ok_or("Invalid profile location")?);
    logging::write(concat!("Rust host startup ", env!("CARGO_PKG_VERSION")));
    // Windows owns all external handles. COM initialization and every call below
    // run on this thread; callbacks never retain borrowed pointers from Windows.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }
    let _apartment = ComApartment;
    let mutex_name = if config.qa_port.is_some() {
        format!(
            "Local\\QuranMushafReader.Rust.QA.{}",
            config.qa_port.unwrap_or_default()
        )
    } else {
        "Local\\QuranMushafReader.Rust".to_owned()
    };
    let name = CoTaskMemPWSTR::from(mutex_name.as_str());
    let mutex = unsafe { CreateMutexW(None, false, *name.as_ref().as_pcwstr())? };
    let already_open = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let _mutex = MutexHandle(mutex);
    if already_open {
        unsafe {
            if let Ok(existing) = FindWindowW(CLASS, None) {
                if IsIconic(existing).as_bool() {
                    let _ = ShowWindow(existing, SW_RESTORE);
                }
                let _ = SetForegroundWindow(existing);
            }
        }
        return Ok(());
    }
    let instance = unsafe { GetModuleHandleW(None)? };
    // Win32 integer resource IDs are tagged pointers, not dereferenced addresses.
    let icon = unsafe {
        LoadIconW(
            Some(HINSTANCE(instance.0)),
            PCWSTR(std::ptr::without_provenance(1)),
        )
    }
    .unwrap_or_default();
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: HINSTANCE(instance.0),
        lpszClassName: CLASS,
        hIcon: icon,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
        hbrBackground: HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err(windows::core::Error::from_thread().into());
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW,
            CLASS,
            w!("Quran Mushaf Reader"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1280,
            900,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )?
    };
    let host = Rc::new(Host {
        hwnd,
        config,
        controller: RefCell::new(None),
        last_bounds: Cell::new(RECT::default()),
        last_maximized: Cell::new(None),
        commands: RefCell::new(VecDeque::new()),
        fullscreen: Cell::new(false),
        placement: RefCell::new(None),
        generation: Cell::new(0),
        closed: Cell::new(false),
        recovery_pending: Cell::new(false),
        language: Cell::new(Language::English),
        session: Cell::new(false),
        theme: Cell::new(Theme::Dark),
        started: Instant::now(),
        document_ready: Cell::new(false),
    });
    HOST.with(|slot| *slot.borrow_mut() = Some(host.clone()));
    // Remove only the native caption; HTML controls live in the same 24 DIP slot
    // as the former WinUI caption and expose real accessible buttons.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        );
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }
    host.start_webview();
    let mut message = MSG::default();
    loop {
        match unsafe { GetMessageW(&mut message, None, 0, 0) }.0 {
            -1 => {
                let error = windows::core::Error::from_thread();
                host.shutdown();
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                HOST.with(|slot| *slot.borrow_mut() = None);
                return Err(error.into());
            }
            0 => break,
            _ => unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
    host.shutdown();
    HOST.with(|slot| *slot.borrow_mut() = None);
    Ok(())
}

impl Host {
    fn webview(&self) -> Option<ICoreWebView2> {
        let controller = self.controller.borrow().clone();
        controller.and_then(|controller| unsafe { controller.CoreWebView2() }.ok())
    }
    fn send(&self, message: &str) {
        if let Some(view) = self.webview() {
            let value = CoTaskMemPWSTR::from(message);
            unsafe {
                let _ = view.PostWebMessageAsString(*value.as_ref().as_pcwstr());
            }
        }
    }
    fn focus_reader(&self) {
        // Windows can restore focus to our parent HWND after activation or a
        // caption action. Return it to WebView2 without changing the DOM's
        // focused element (for example an unfinished search query).
        if !self.document_ready.get() || self.closed.get() {
            return;
        }
        let controller = self.controller.borrow().clone();
        if let Some(controller) = controller {
            unsafe {
                if (GetForegroundWindow() == self.hwnd || GetFocus() == self.hwnd)
                    && !IsIconic(self.hwnd).as_bool()
                {
                    let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
                }
            }
        }
    }
    fn resize(&self) {
        let controller = self.controller.borrow().clone();
        if let Some(controller) = controller {
            let mut rect = RECT::default();
            unsafe {
                if GetClientRect(self.hwnd, &mut rect).is_ok()
                    && self.last_bounds.replace(rect) != rect
                {
                    let _ = controller.SetBounds(rect);
                }
                let _ = controller.SetIsVisible(self.document_ready.get() && !IsIconic(self.hwnd).as_bool());
                let _ = controller.NotifyParentWindowPositionChanged();
            }
        }
        let maximized = unsafe { IsZoomed(self.hwnd).as_bool() };
        if self.last_maximized.replace(Some(maximized)) != Some(maximized) {
            self.send(if maximized {
                "native-window-maximized"
            } else {
                "native-window-restored"
            });
        }
    }
    fn enqueue(&self, message: Message) {
        let mut commands = self.commands.borrow_mut();
        if commands.len() >= MAX_COMMANDS {
            return;
        }
        commands.push_back(message);
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), COMMAND, WPARAM(0), LPARAM(0));
        }
    }
    fn synchronize(&self) {
        self.last_maximized.set(None);
        self.send(if self.fullscreen.get() {
            "native-fullscreen-on"
        } else {
            "native-fullscreen-off"
        });
        self.send(if self.session.get() {
            "native-reading-session-on"
        } else {
            "native-reading-session-off"
        });
        self.resize();
    }
    fn handle(&self, message: Message) {
        match message {
            Message::ReaderTheme { theme, animate: _ } => {
                self.theme.set(theme);
                let light = matches!(theme, Theme::Light | Theme::Sepia);
                let dark = BOOL::from(!light);
                unsafe {
                    let _ = DwmSetWindowAttribute(
                        self.hwnd,
                        DWMWA_USE_IMMERSIVE_DARK_MODE,
                        &dark as *const _ as *const _,
                        std::mem::size_of::<BOOL>() as u32,
                    );
                }
            }
            Message::ReaderControlsState { language, .. } => self.language.set(language),
            Message::ReadingSessionState { language, active } => {
                self.language.set(language);
                self.session.set(active);
                self.send(if active {
                    "native-reading-session-on"
                } else {
                    "native-reading-session-off"
                });
            }
            Message::FullscreenRequest { active } => self.set_fullscreen(active),
            Message::ReaderReady {} => self.synchronize(),
            Message::WindowAction { action } => unsafe {
                match action {
                    WindowAction::Minimize => {
                        let _ = ShowWindow(self.hwnd, SW_MINIMIZE);
                    }
                    WindowAction::Maximize => {
                        if self.fullscreen.get() {
                            self.set_fullscreen(false);
                        }
                        let _ = ShowWindow(
                            self.hwnd,
                            if IsZoomed(self.hwnd).as_bool() {
                                SW_RESTORE
                            } else {
                                SW_MAXIMIZE
                            },
                        );
                        self.focus_reader();
                    }
                    WindowAction::Close => {
                        let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                    WindowAction::Drag => {
                        if !self.fullscreen.get() {
                            // A queued drag request must still belong to a held primary button.
                            let mut cursor = POINT::default();
                            if GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0
                                && GetCursorPos(&mut cursor).is_ok()
                            {
                                let _ = ReleaseCapture();
                                let coordinates =
                                    (cursor.x as u16 as u32) | ((cursor.y as u16 as u32) << 16);
                                SendMessageW(
                                    self.hwnd,
                                    WM_NCLBUTTONDOWN,
                                    Some(WPARAM(HTCAPTION as usize)),
                                    Some(LPARAM(coordinates as isize)),
                                );
                            }
                        }
                    }
                }
            },
        }
    }
    fn set_fullscreen(&self, active: bool) {
        if active == self.fullscreen.get() {
            self.synchronize();
            return;
        }
        unsafe {
            if active {
                let mut placement = WINDOWPLACEMENT {
                    length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                    ..Default::default()
                };
                if GetWindowPlacement(self.hwnd, &mut placement).is_err() {
                    return;
                }
                // Fullscreen is an explicit request to show the reader. A
                // request received while minimized must later restore a usable
                // normal window, rather than the off-screen minimized icon.
                if IsIconic(self.hwnd).as_bool() {
                    placement.showCmd = SW_SHOWNORMAL.0 as u32;
                }
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if !GetMonitorInfoW(
                    MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST),
                    &mut info,
                )
                .as_bool()
                {
                    return;
                }
                *self.placement.borrow_mut() = Some(placement);
                self.fullscreen.set(true);
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, (WS_POPUP | WS_VISIBLE).0 as isize);
                let bounds = info.rcMonitor;
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    bounds.left,
                    bounds.top,
                    bounds.right - bounds.left,
                    bounds.bottom - bounds.top,
                    SWP_NOZORDER | SWP_FRAMECHANGED,
                );
            } else {
                self.fullscreen.set(false);
                SetWindowLongPtrW(
                    self.hwnd,
                    GWL_STYLE,
                    (WS_OVERLAPPEDWINDOW | WS_VISIBLE).0 as isize,
                );
                if let Some(placement) = self.placement.borrow_mut().take() {
                    let _ = SetWindowPlacement(self.hwnd, &placement);
                }
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
                );
            }
        }
        self.synchronize();
        self.focus_reader();
    }
    fn fail(&self, context: &str) {
        logging::write(context);
        if !self.closed.get() && !self.recovery_pending.replace(true) {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), RECOVERY, WPARAM(0), LPARAM(0));
            }
        }
    }
    fn recover(self: &Rc<Self>) {
        if self.closed.get() {
            return;
        }
        let controller = self.controller.borrow().clone();
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.SetIsVisible(false);
            }
        }
        let message = if self.language.get() == Language::Arabic {
            "توقف عارض القرآن. هل تريد إعادة فتحه؟\nستبقى إعداداتك وتنزيلاتك محفوظة."
        } else {
            "The reader could not load or stopped responding. Reopen it?\nYour preferences and downloads are preserved."
        };
        let value = CoTaskMemPWSTR::from(message);
        let retry = unsafe {
            MessageBoxW(
                Some(self.hwnd),
                *value.as_ref().as_pcwstr(),
                w!("Quran Mushaf Reader"),
                MB_RETRYCANCEL | MB_ICONWARNING,
            )
        };
        self.recovery_pending.set(false);
        if retry == IDRETRY {
            self.close_controller();
            self.start_webview();
        } else {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
    fn close_controller(&self) {
        // End event registration ownership explicitly, including during recovery.
        let controller = self.controller.borrow_mut().take();
        self.last_bounds.set(RECT::default());
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.Close();
            }
        }
    }
    fn shutdown(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.generation.set(self.generation.get() + 1);
        self.commands.borrow_mut().clear();
        self.close_controller();
        logging::write("Host closed");
    }
    fn start_webview(self: &Rc<Self>) {
        self.document_ready.set(false);
        unsafe { let _ = InvalidateRect(Some(self.hwnd), None, false); }
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let weak = Rc::downgrade(self);
        let profile = CoTaskMemPWSTR::from(self.config.profile.to_string_lossy().as_ref());
        let options: ICoreWebView2EnvironmentOptions =
            CoreWebView2EnvironmentOptions::default().into();
        #[cfg(feature = "qa")]
        if let Some(port) = self.config.qa_port {
            let args = CoTaskMemPWSTR::from(format!("--remote-debugging-port={port}").as_str());
            unsafe {
                let _ = options.SetAdditionalBrowserArguments(*args.as_ref().as_pcwstr());
            }
        }
        let created = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
            move |result, environment| {
                let Some(host) = weak
                    .upgrade()
                    .filter(|host| !host.closed.get() && host.generation.get() == generation)
                else {
                    return Ok(());
                };
                let Some(environment) = environment.filter(|_| result.is_ok()) else {
                    host.fail(
                        "WebView2 environment creation failed; check installed Evergreen runtime.",
                    );
                    return Ok(());
                };
                let weak = Rc::downgrade(&host);
                let completed = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                    move |result, controller| {
                        let Some(host) = weak.upgrade().filter(|host| {
                            !host.closed.get() && host.generation.get() == generation
                        }) else {
                            return Ok(());
                        };
                        let Some(controller) = controller.filter(|_| result.is_ok()) else {
                            host.fail("WebView2 controller creation failed");
                            return Ok(());
                        };
                        *host.controller.borrow_mut() = Some(controller.clone());
                        if let Err(error) = host.configure_webview(&controller, generation) {
                            host.fail(&format!("WebView2 configuration failed: {error}"));
                        }
                        Ok(())
                    },
                ));
                if let Err(error) =
                    unsafe { environment.CreateCoreWebView2Controller(host.hwnd, &completed) }
                {
                    host.fail(&format!("Controller request failed: {error}"));
                }
                Ok(())
            },
        ));
        if let Err(error) = unsafe {
            CreateCoreWebView2EnvironmentWithOptions(
                PCWSTR::null(),
                *profile.as_ref().as_pcwstr(),
                &options,
                &created,
            )
        } {
            self.fail(&format!("Environment request failed: {error}"));
        }
    }
    fn configure_webview(
        self: &Rc<Self>,
        controller: &ICoreWebView2Controller,
        generation: u64,
    ) -> windows::core::Result<()> {
        unsafe {
            let view = controller.CoreWebView2()?;
            let settings = view.Settings()?;
            settings.SetAreDevToolsEnabled(false)?;
            settings.SetAreDefaultContextMenusEnabled(false)?;
            let settings3: ICoreWebView2Settings3 = settings.cast()?;
            settings3.SetAreBrowserAcceleratorKeysEnabled(false)?;
            settings.SetAreHostObjectsAllowed(false)?;
            settings.SetAreDefaultScriptDialogsEnabled(false)?;
            settings.SetIsStatusBarEnabled(false)?;
            settings.SetIsZoomControlEnabled(false)?;
            let settings4: ICoreWebView2Settings4 = settings.cast()?;
            settings4.SetIsGeneralAutofillEnabled(false)?;
            settings4.SetIsPasswordAutosaveEnabled(false)?;
            let mapping: ICoreWebView2_3 = view.cast()?;
            let folder = CoTaskMemPWSTR::from(self.config.web_app.to_string_lossy().as_ref());
            mapping.SetVirtualHostNameToFolderMapping(
                w!("quran-mushaf.local"),
                *folder.as_ref().as_pcwstr(),
                COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS,
            )?;
            let background: ICoreWebView2Controller2 = controller.cast()?;
            background.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                A: 255,
                R: 32,
                G: 32,
                B: 32,
            })?;
            let mut token = 0;
            view.add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let mut uri = PWSTR::null();
                        args.Uri(&mut uri)?;
                        if !protocol::is_trusted_uri(&CoTaskMemPWSTR::from(uri).to_string()) {
                            args.SetCancel(true)?;
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            view.add_FrameNavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetCancel(true)?;
                    } // Reader has no embedded frames.
                    Ok(())
                })),
                &mut token,
            )?;
            view.add_NewWindowRequested(
                &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetHandled(true)?;
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            if let Ok(external) = view.cast::<ICoreWebView2_18>() {
                external.add_LaunchingExternalUriScheme(
                    &LaunchingExternalUriSchemeEventHandler::create(Box::new(|_, args| {
                        if let Some(args) = args {
                            args.SetCancel(true)?;
                        }
                        Ok(())
                    })),
                    &mut token,
                )?;
            }
            view.add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let mut kind = COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION;
                        let mut uri = PWSTR::null();
                        let mut user = BOOL::default();
                        args.PermissionKind(&mut kind)?;
                        args.Uri(&mut uri)?;
                        args.IsUserInitiated(&mut user)?;
                        let trusted =
                            protocol::is_trusted_uri(&CoTaskMemPWSTR::from(uri).to_string());
                        // Reminders remain opt-in through the platform notification prompt.
                        if kind != COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS
                            || !trusted
                            || !user.as_bool()
                        {
                            args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            let downloads: ICoreWebView2_4 = view.cast()?;
            downloads.add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetCancel(true)?;
                        args.SetHandled(true)?;
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            let weak = Rc::downgrade(self);
            view.add_WebMessageReceived(
                &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                    if let (Some(host), Some(args)) = (weak.upgrade(), args) {
                        if host.closed.get() || host.generation.get() != generation {
                            return Ok(());
                        }
                        let mut source = PWSTR::null();
                        args.Source(&mut source)?;
                        let source = CoTaskMemPWSTR::from(source).to_string();
                        if !protocol::is_trusted_uri(&source) {
                            return Ok(());
                        }
                        let mut body = PWSTR::null();
                        args.WebMessageAsJson(&mut body)?;
                        if let Some(message) = protocol::parse_message(
                            &source,
                            &CoTaskMemPWSTR::from(body).to_string(),
                        ) {
                            host.enqueue(message);
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            let weak = Rc::downgrade(self);
            view.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                    if let (Some(host), Some(args)) = (weak.upgrade(), args) {
                        if host.generation.get() != generation {
                            return Ok(());
                        }
                        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED;
                        args.ProcessFailedKind(&mut kind)?;
                        if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
                            || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                            || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE
                        {
                            host.fail("WebView2 reader process failed; explicit retry required");
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            let weak = Rc::downgrade(self);
            view.add_NavigationCompleted(
                &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                    if let (Some(host), Some(args)) = (weak.upgrade(), args) {
                        if host.closed.get() || host.generation.get() != generation {
                            return Ok(());
                        }
                        let mut success = BOOL::default();
                        args.IsSuccess(&mut success)?;
                        if !success.as_bool() {
                            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
                            args.WebErrorStatus(&mut status)?;
                            if status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                                host.fail("Reader navigation failed");
                            }
                        } else {
                            host.document_ready.set(true);
                            host.resize();
                            logging::write(&format!(
                                "Reader navigation complete {} ms",
                                host.started.elapsed().as_millis()
                            ));
                            host.focus_reader();
                            host.synchronize();
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            let weak = Rc::downgrade(self);
            controller.add_AcceleratorKeyPressed(
                &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                    if let (Some(host), Some(args)) = (weak.upgrade(), args) {
                        let mut key = 0;
                        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
                        args.VirtualKey(&mut key)?;
                        args.KeyEventKind(&mut kind)?;
                        if key == VK_F11.0 as u32
                            && (kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                                || kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN)
                        {
                            args.SetHandled(true)?;
                            let mut physical = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
                            args.PhysicalKeyStatus(&mut physical)?;
                            if !physical.WasKeyDown.as_bool() {
                                let _ = PostMessageW(
                                    Some(host.hwnd),
                                    TOGGLE_FULLSCREEN,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                            }
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            let weak = Rc::downgrade(self);
            let init = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                move |result, _| {
                    if let Some(host) = weak
                        .upgrade()
                        .filter(|host| !host.closed.get() && host.generation.get() == generation)
                    {
                        if let Err(error) = result {
                            host.fail(&format!("Reader initialization failed: {error}"));
                        } else if let Some(view) = host.webview() {
                            let url = CoTaskMemPWSTR::from(
                                format!(
                                    "https://quran-mushaf.local/index.html?v={}",
                                    env!("CARGO_PKG_VERSION")
                                )
                                .as_str(),
                            );
                            if let Err(error) = view.Navigate(*url.as_ref().as_pcwstr()) {
                                host.fail(&format!("Reader navigation request failed: {error}"));
                            }
                        }
                    }
                    Ok(())
                },
            ));
            view.AddScriptToExecuteOnDocumentCreated(w!("Object.defineProperty(window,'__QURAN_RUST_HOST__',{value:true,writable:false,configurable:false});"), &init)?;
        }
        self.resize();
        logging::write(&format!(
            "WebView2 initialized {} ms",
            self.started.elapsed().as_millis()
        ));
        Ok(())
    }
}

// This callback receives Windows-owned pointers only for the duration of the
// message. The state clone is obtained without holding a RefCell borrow across
// calls into Windows/COM, which can synchronously re-enter this procedure.
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCALCSIZE && wp.0 != 0 {
        // Windows puts a maximized resize border outside the monitor. Keep the
        // custom client area inside its work area so the HTML caption and page
        // corners are not clipped. Fullscreen uses a borderless popup instead.
        if unsafe { IsZoomed(hwnd).as_bool() } {
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if unsafe {
                GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info)
                    .as_bool()
            } {
                unsafe { (*(lp.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0] = info.rcWork };
            }
        }
        return LRESULT(0);
    }
    let host = HOST.with(|slot| slot.borrow().clone());
    let Some(host) = host else {
        return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    };
    match msg {
        WM_PAINT => {
            // Immediate, allocation-light feedback while WebView2 initializes.
            // The loaded document replaces this with its accessible HTML state.
            unsafe {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let _ = SetDCBrushColor(dc, windows::Win32::Foundation::COLORREF(0x00202020));
                let _ = FillRect(dc, &rect, HBRUSH(GetStockObject(DC_BRUSH).0));
                if !host.document_ready.get() {
                    let dpi = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
                    let unit = |value: f64| (value * dpi).round() as i32;
                    let center = rect.right / 2;
                    // The class owns this icon. Use it until HTML can read the
                    // saved language and show its single localized loading line.
                    let icon = HICON(GetClassLongPtrW(hwnd, GCLP_HICON) as *mut _);
                    let _ = DrawIconEx(dc, center - unit(24.0), rect.bottom / 2 - unit(48.0),
                        icon, unit(48.0), unit(48.0), 0, None, DI_NORMAL);
                    let brush = GetStockObject(DC_BRUSH);
                    let old_brush = SelectObject(dc, brush);
                    let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
                    let _ = SetDCBrushColor(dc, windows::Win32::Foundation::COLORREF(0x00bac9a6));
                    for offset in [-10.0, 0.0, 10.0] {
                        let left = center + unit(offset) - unit(2.0);
                        let top = rect.bottom / 2 + unit(45.0);
                        let _ = Ellipse(dc, left, top, left + unit(4.0), top + unit(4.0));
                    }
                    let _ = SelectObject(dc, old_brush);
                    let _ = SelectObject(dc, old_pen);
                }
                let _ = EndPaint(hwnd, &paint);
            }
            LRESULT(0)
        }
        COMMAND => {
            let message = host.commands.borrow_mut().pop_front();
            if let Some(message) = message {
                host.handle(message);
            }
            LRESULT(0)
        }
        TOGGLE_FULLSCREEN => {
            host.set_fullscreen(!host.fullscreen.get());
            LRESULT(0)
        }
        RECOVERY => {
            host.recover();
            LRESULT(0)
        }
        WM_SIZE | WM_MOVE => {
            host.resize();
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let rect = unsafe { *(lp.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            host.resize();
            LRESULT(0)
        }
        WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => {
            host.send("native-arrow-release");
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
        WM_SETFOCUS => {
            host.focus_reader();
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_F11.0 as usize => {
            if lp.0 & (1 << 30) == 0 {
                host.set_fullscreen(!host.fullscreen.get());
            }
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize && host.fullscreen.get() => {
            host.set_fullscreen(false);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let limits = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
            limits.ptMinTrackSize.x = 720 * dpi / 96;
            limits.ptMinTrackSize.y = 480 * dpi / 96;
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if unsafe {
                GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info)
                    .as_bool()
            } {
                limits.ptMaxPosition.x = info.rcWork.left - info.rcMonitor.left;
                limits.ptMaxPosition.y = info.rcWork.top - info.rcMonitor.top;
                limits.ptMaxSize.x = info.rcWork.right - info.rcWork.left;
                limits.ptMaxSize.y = info.rcWork.bottom - info.rcWork.top;
            }
            LRESULT(0)
        }
        WM_NCHITTEST if !host.fullscreen.get() && !unsafe { IsZoomed(hwnd).as_bool() } => {
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
                let x = lp.0 as u16 as i16 as i32;
                let y = (lp.0 >> 16) as u16 as i16 as i32;
                let edge = (unsafe { GetDpiForWindow(hwnd) }.max(96) as i32 * 6 / 96).max(4);
                let left = x < rect.left + edge;
                let right = x >= rect.right - edge;
                let top = y < rect.top + edge;
                let bottom = y >= rect.bottom - edge;
                let hit = match (left, right, top, bottom) {
                    (true, _, true, _) => HTTOPLEFT,
                    (_, true, true, _) => HTTOPRIGHT,
                    (true, _, _, true) => HTBOTTOMLEFT,
                    (_, true, _, true) => HTBOTTOMRIGHT,
                    (true, _, _, _) => HTLEFT,
                    (_, true, _, _) => HTRIGHT,
                    (_, _, true, _) => HTTOP,
                    (_, _, _, true) => HTBOTTOM,
                    _ => HTCLIENT,
                };
                return LRESULT(hit as isize);
            }
            LRESULT(HTCLIENT as isize)
        }
        WM_CLOSE => {
            host.shutdown();
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            host.shutdown();
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

pub fn show_startup_error(error: &str) {
    let message =
        CoTaskMemPWSTR::from(format!("Quran Mushaf Reader could not start.\n\n{error}").as_str());
    unsafe {
        MessageBoxW(
            None,
            *message.as_ref().as_pcwstr(),
            w!("Quran Mushaf Reader"),
            MB_OK | MB_ICONERROR,
        );
    }
}
