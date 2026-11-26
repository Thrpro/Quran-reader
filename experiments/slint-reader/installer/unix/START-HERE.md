# Quran Mushaf Reader {VERSION}

What's new in each version is in the release notes: https://github.com/Thrpro/Quran-reader/releases

## Linux

- **AppImage** (any recent distribution): make it executable, then run it.

  ```sh
  chmod +x QuranMushafReader-{VERSION}-x86_64.AppImage
  ./QuranMushafReader-{VERSION}-x86_64.AppImage
  ```

- **.deb** (Debian, Ubuntu and their derivatives):
  `sudo apt install ./quran-mushaf-reader_{VERSION}_amd64.deb`, then open
  Quran Mushaf Reader from the applications menu.
- **.tar.gz**: extract it anywhere and run `quran-mushaf-reader` inside. Keep the
  other files and folders beside it.

The packages are built on Ubuntu 24.04 and need glibc 2.39 or later (Ubuntu
24.04, Debian 13, Fedora 40 and later), with fontconfig and ALSA (served by
PipeWire or PulseAudio on most desktops). Windows open through Wayland or X11.

Settings, reading position, bookmarks and downloads are kept in
`~/.local/share/quran-mushaf-reader` (or `$XDG_DATA_HOME/quran-mushaf-reader`).
To keep them beside an extracted .tar.gz copy instead, create a folder named
`SlintProfile` next to `quran-mushaf-reader`.

## macOS

Open the .dmg and drag Quran Mushaf Reader to Applications, then open it from
there. It runs on Macs with Apple silicon or Intel, macOS 11 or later.

If the release is not signed with an Apple Developer ID, macOS blocks it the
first time. That is expected:

1. Open the app once, and choose **Done** (not Move to Trash).
2. Open **System Settings > Privacy & Security** and scroll down to Security.
3. Beside the message about Quran Mushaf Reader, choose **Open Anyway**, and
   confirm with your password or Touch ID.

You only do this once. On macOS 14 and earlier, Control-clicking the app and
choosing **Open** also works. Only do this for a copy from the project's own
releases page, after checking it against SHA256SUMS-macos.txt.

Settings are kept in `~/Library/Application Support/Quran Mushaf Reader`.

## Quran resources

The Quran text, layout, fonts and study data come unchanged from the Quranic
Universal Library and DigitalKhatt. `resource-manifest.json` lists the SHA-256
of all 1,569 files. To check a .tar.gz copy without launching the app:

```sh
python3 verify_resources.py
```

This confirms your files match this project's package. It is not a comparison
against a fresh QUL download.

If you see drawing problems or high CPU use, start the app with
`--renderer software`. Software rendering is much less smooth, so use it only
if you need it.

Licenses are included in `licenses`. The source code is at https://github.com/Thrpro/Quran-reader.
