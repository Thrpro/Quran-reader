# Linux and macOS

The reader builds for Linux (x86-64) and macOS (Apple silicon) from the same
source as the Windows app. The page renderer and the recitation engine are
the same on all three systems.

## What works

- The Mushaf: pages are drawn by the portable renderer (HarfRust, Skrifa and
  tiny-skia), the same one Windows uses by default, so pages look the same.
- Page turns, themes, the page clock, search, bookmarks, keyboard shortcuts,
  window resizing, and the included tafsirs and translation.
- Recitation, verse by verse or whole-surah recordings with the verse
  highlight following, offline downloads, speed, volume, pause and seeking:
  the recitation engine in `src/recitation` (below) is the same on all three
  systems.
- Downloading, verifying, removing and reading the optional tafsir books from
  Quran.com: the native tafsir library, generated for Linux and macOS (below).
- Settings, kept in the user's application data folder:
  - Linux: `$XDG_DATA_HOME/quran-mushaf-reader`, normally
    `~/.local/share/quran-mushaf-reader`
  - macOS: `~/Library/Application Support/Quran Mushaf Reader`

  A `SlintProfile` folder next to the app is used instead if it exists, so a
  portable copy keeps its settings with it.
- The Quran data (`Resources`) is found next to the app, in
  `<prefix>/share/quran-mushaf-reader/Resources` for an installed Linux app,
  or in `Contents/Resources/Mushaf` inside a macOS app bundle. `--resources
  <folder>` still overrides this.

## Not yet

- **Seeking far ahead in a recording that is still downloading** waits until
  the download reaches that point (the native Windows engine requested that
  part of the file directly).
- **A Developer ID for macOS.** The .dmg is signed ad hoc until the signing
  secrets are set (below), so macOS asks people to confirm the app on first
  launch.
- **64-bit ARM Linux** has no package yet.
- **Interface font.** The interface asks for Segoe UI; Linux and macOS use
  their own system font instead. Bundling one font would make all three alike.

## Packages

`installer/unix/package.py` packages a built executable, and the CI workflow
`reader-linux-macos.yml` runs it when started by hand or by a version tag,
then attaches the files to the same draft release as the Windows ones:

| System | Files |
| --- | --- |
| Linux (x86-64, glibc 2.39 or later) | `QuranMushafReader-<version>-x86_64.AppImage`, `quran-mushaf-reader_<version>_amd64.deb`, `QuranMushafReader-<version>-linux-x86_64.tar.gz` |
| macOS (Apple silicon and Intel, 11 or later) | `QuranMushafReader-<version>-macos-universal.dmg` |

Each has its `SHA256SUMS-linux.txt` or `SHA256SUMS-macos.txt`. As on Windows:

- The Quran resources come from the pinned 0.16.0 release ZIP and are checked
  file by file against the pinned resource manifest, unchanged.
  `installer/unix/verify_resources.py` does the check (it ships in the Linux
  packages, like `Verify-Quran-Resources.ps1` on Windows).
- `licenses/` carries a notice for every crate built into that system's
  executable (`add_licenses.py`, then `check_licenses.py` fails the package if
  any is missing). A crate that declares a license but ships no text gets the
  standard text and a `NOTE.txt` saying so.
- The package never replaces an earlier output folder.

The CI then installs the .deb, unpacks the AppImage and mounts the .dmg to
check them, and checks that the executables link only system libraries
(Linux: libc, the C++ runtime, fontconfig, FreeType and ALSA; the windowing and GPU libraries are loaded
when needed and are the .deb's dependencies).

The AppImage tools are downloaded at fixed versions and checked against their
published SHA-256. `--layout-only` builds the folders and the .tar.gz on any
system (Windows included), for checking the layout without those tools.

For a Developer ID signature and notarization on macOS, add these repository
secrets: `MACOS_CERTIFICATE` (the Developer ID Application certificate as a
base64 .p12), `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGN_IDENTITY`, and for
notarization `APPLE_ID`, `APPLE_TEAM_ID` and `APPLE_APP_PASSWORD` (an
app-specific password).

## How the code is arranged

The reader shares many files, word for word, with the protected native
Windows app (`windows/QuranMushaf.Native/src`). Those that only compute
(page layout, data, page-turn geometry, timing) are used on every system.
Those that call Windows have counterparts in `src/portable/`, with the same
module names and the parts of the interface the reader uses; `main.rs`
chooses between them with `#[cfg(windows)]`.

| Native (Windows) | Linux and macOS |
| --- | --- |
| `render.rs` (DirectWrite) | `portable/render.rs`: the shared rectangle only; pages come from `portable_render.rs` |
| `owned_file.rs`, `profile_file.rs` | `portable/owned_file.rs`, `portable/profile_file.rs` |
| `audio.rs` reciter list | `portable/audio.rs`: the same list, word for word |
| `tafsir_library.rs` | `portable/tafsir_library.rs`: generated from it (below) |
| `audio.rs`, `audio_cache.rs`, `timed_store.rs` and the rest of the native recitation engine | `recitation/`, used on Windows too (below) |

## The recitation engine

`src/recitation` plays recitations on all three systems, behind the same
interface as the native Windows engine, so `audio_bridge.rs` drives either:

- `net.rs`: HTTPS from the providers in PRIVACY.md only, no redirects.
- `store.rs`: the profile's `AudioStore`, in the native engine's exact
  folders and record formats (SHA-256 checked), so downloads made by either
  engine play in the other.
- `decode.rs`: MP3 decoding (Symphonia), conversion to the sound device's
  rate, and reading a recording while it downloads.
- `stretch.rs`: speed changes without changing pitch (WSOLA). A new speed is
  heard within about 0.15 s with no pause, unlike Windows media playback,
  which stopped the sound for about half a second. At normal speed the sound
  is reproduced exactly.
- `output.rs`: the sound device (cpal: WASAPI, CoreAudio, ALSA/PipeWire).
- `player.rs`: listening ranges, the opening basmala, lookahead, seeking,
  and the verse being heard.
- `qa.rs`: `--qa-recitation <folder>` plays real recitations, changes speed,
  pauses, seeks and downloads, and writes what happened, including the exact
  sound played.

## The tafsir library

`portable/tafsir_library.rs` is generated from the native
`tafsir_library.rs` by `tools/port_tafsir_library.py`; edit the script, never
the output. The script copies the native file and replaces only its Windows
calls, each by exact text, and stops if one no longer matches:

| Windows | Linux and macOS |
| --- | --- |
| WinHTTP to api.quran.com | ureq and rustls, with the same rules: TLS only, no redirects, cookies or credentials, identity encoding, the same timeouts, size limits and retries |
| Reparse-point checks | Symbolic-link checks |
| `EnumProcesses` (is a staging folder's owner still running?) | `kill(pid, 0)` |
| `MoveFileW` / `MoveFileExW` with write-through | `renameat2(RENAME_NOREPLACE)` on Linux, `renamex_np(RENAME_EXCL)` on macOS, then the folder is synced |

Downloaded books use the native folders and formats, so a profile moved
between systems keeps its books. On Windows the generated copy is compiled
for its tests: the native library's tests run on it, with tests for the
replaced calls and a check that the native file has not changed since the copy
was made. `cargo test portable_tafsir -- --ignored` downloads a complete book
through it.

A `native-audio` build (`cargo build --features native-audio`, Windows only)
keeps the native Windows engine instead.

Page colours live in `src/theme.rs` on every system. On Windows, tests check
that the theme colours, the reciter list and the tafsir book list are
identical to the native ones, so the copies cannot drift.

## Checking Linux and macOS builds on Windows

`cargo check` and `cargo clippy` work without a Linux or Mac machine, because
nothing is linked:

- `rustup target add x86_64-unknown-linux-gnu aarch64-apple-darwin`
- Set `PKG_CONFIG_ALLOW_CROSS=1` and point `PKG_CONFIG` at a stand-in that
  succeeds: fontconfig's build script asks for the library but a check never
  links it.
- ring (used for HTTPS) compiles a little C. Point `CC_<target>` and
  `AR_<target>` at stand-ins that write empty files.
- `cargo clippy --target <target> --all-targets -- -D warnings`

Running the tests needs the real systems: the CI workflow
`.github/workflows/reader-linux-macos.yml` does that.
