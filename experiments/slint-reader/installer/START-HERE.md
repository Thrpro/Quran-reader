# Quran Mushaf Reader {VERSION}

Run the installer, or extract the whole ZIP to a folder and run **QuranMushaf.SlintReader.exe**. Keep the other files and folders beside it. Nothing else needs to be installed: the Visual C++ runtime files are included.

What's new in each version is in the release notes: https://github.com/Thrpro/Quran-reader/releases

If you see drawing problems or high CPU use, turn on **Options → Use software rendering** and restart. Software rendering is much less smooth, so leave it off otherwise. To prefer the dedicated GPU on a laptop with two, start the app with `--gpu performance`.

## Quran resources

The Quran text, layout, fonts and study data come unchanged from the Quranic Universal Library and DigitalKhatt. `resource-manifest.json` lists the SHA-256 of all 1,569 files. To check your copy without launching the app:

```powershell
./Verify-Quran-Resources.ps1
```

This confirms your files match this project's package. It is not a comparison against a fresh QUL download.

## Settings

The app keeps its settings, reading position and bookmarks in a `SlintProfile` folder beside the executable. Uninstalling keeps them, so reinstalling never loses them.

Licenses are included in `licenses`. The source code is at https://github.com/Thrpro/Quran-reader.
