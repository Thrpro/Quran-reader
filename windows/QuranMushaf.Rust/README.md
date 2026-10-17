# Rust Windows host

Version 0.5.0 replaces the WinUI/.NET shell with a Rust Win32 host using
`windows` and `webview2-com`. It hosts the existing React reader in Evergreen
WebView2, without bundling .NET or Windows App Runtime. It does not replace
the Quran data, Digital Khatt rendering, browser audio pipeline or offline
download format.

## Build

Requirements: x64 Windows, Rust's MSVC toolchain, MSVC C++ Build Tools, a Windows
SDK, and Node.js/npm. Inno Setup 6 creates the installer. Runtime users need
WebView2, but no Rust/compiler/.NET installation.

From the repository root:

```powershell
npm ci
npm run publish:windows
```

The publish script uses `Cargo.lock`, runs Rust tests, builds an optimized
release executable with the static MSVC runtime, copies the tested `dist`
into `outputs/rust-windows/publish/WebApp`, and checks the packaged files against
that web build. The executable retains the installed filename
`QuranMushaf.Windows.exe` for upgrade compatibility.

`RUST-HOST-BUILD.json` in the published payload records the executable hash,
lockfile hash and file sizes. Compare measured release payload and installer
sizes when assessing storage savings.

To run native tests directly:

```powershell
Push-Location windows\QuranMushaf.Rust
cargo test --locked
Pop-Location
```

## Window and storage compatibility

The React `RustTitleBar` provides accessible Search/options and native window
buttons in the same 24 DIP top slot as the previous title bar. Its palette
follows the reader theme, including reduced-motion preferences. The window
host handles drag, resizing, maximize/restore, DPI changes and fullscreen.
F11 toggles fullscreen; the reader also provides a visible exit and Escape.
Arrow keys remain with the web reader so editable fields retain normal caret
movement and page turns are not dispatched twice.

The host maps the bundled `WebApp` directory to:

```text
https://quran-mushaf.local
```

It reuses:

```text
%LOCALAPPDATA%\QuranMushafReader\WebView2
```

Both identities must remain unchanged to preserve existing settings, reading
preferences, offline recitations and tafsir. Production and QA profiles must
never be shared. Updating the executable does not require converting the
browser storage to native files.

The bridge accepts a small validated set of reader-state and window-action
messages. It exposes no general filesystem, process-launch or shell-command
API. Navigation and messages are restricted to the exact reader origin;
new-window requests and unnecessary permissions are denied. The bundled
Content Security Policy remains in force. Explicit recovery recreates the
WebView2 controller while retaining its profile.

## Isolated native QA

Remote debugging is available only in binaries explicitly compiled with the
`qa` feature. Normal publish/release builds leave that feature disabled and
reject QA command-line arguments.

```powershell
Push-Location windows\QuranMushaf.Rust
cargo build --locked --features qa
Pop-Location
```

For a separate test app, stage the resulting
`windows/QuranMushaf.Rust/target/debug/quran-mushaf-reader.exe` beside a `WebApp`
directory containing the tested `dist` files. For example:

```powershell
$qaApp = Join-Path (Get-Location) 'outputs\rust-host-qa\app'
New-Item -ItemType Directory -Force -Path $qaApp, (Join-Path $qaApp 'WebApp') | Out-Null
Copy-Item windows\QuranMushaf.Rust\target\debug\quran-mushaf-reader.exe (Join-Path $qaApp 'QuranMushaf.Windows.exe')
Copy-Item dist\* (Join-Path $qaApp 'WebApp') -Recurse -Force
$qaProfile = Join-Path (Get-Location) 'outputs\rust-host-qa\profile'
$qaProcess = Start-Process (Join-Path $qaApp 'QuranMushaf.Windows.exe') -WindowStyle Hidden -PassThru -ArgumentList ('--qa-profile "' + $qaProfile + '" --qa-port 9235')
$qaProcess.Id | Set-Content (Join-Path (Get-Location) 'outputs\rust-host-qa\host.pid')
node scripts/verify-rust-host.mjs --port 9235
powershell -NoProfile -File scripts/verify-rust-window.ps1
```

The native smoke script checks the real WebView2 origin, titlebar layout,
maximize/restore acknowledgments, search, tafsir navigation, fullscreen exit,
theme colors and contextual rendering. The native window script additionally
checks resize edges, DPI-scaled size limits, minimize/restore and maximized
client bounds. Both scripts use only the isolated test host. The smoke script
writes a test setting and an offline
cache entry to the isolated profile. Close and restart that QA app with the same
profile, then run:

```powershell
node scripts/verify-rust-host.mjs --port 9235 --check-persistence
```

Reports and screenshots are written under `outputs/rust-host-qa`. Do not
distribute a QA binary or expose its debugging port to a network.

## Rollback and licenses

The earlier .NET application is preserved in separate development backups; it
is not part of this source snapshot. This repository builds the Rust host.
`windows/QuranMushaf.Windows/Assets/app.ico` retains its historical path because
the Rust resource compiler uses that icon. Back up the source before replacing
an existing development build.

The host code is MIT licensed. Its dependencies and Microsoft WebView2 retain
their own license terms. Quran resources, fonts and recordings are governed by
their upstream notices; see the repository's `THIRD-PARTY-NOTICES.md` and
`RESOURCE_COMPATIBILITY.md`.
