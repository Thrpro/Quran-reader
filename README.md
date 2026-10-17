# Quran Mushaf Reader

A free, open-source Quran reader for Windows, designed for comfortable Mushaf reading, listening, and reading tafseer.

Quran Mushaf Reader is developed with the intention of serving the Book of Allah and making the Quran more accessible to readers, students, and teachers. The project aims to combine a familiar Mushaf presentation with practical tools for recitation, tafsir, and navigation—without altering the underlying Quranic text.

> **Development status:** the current source is v0.5.7, pending release. The earlier public v0.5.0 corresponds to internal build v0.5.4. See PATCH-NOTES-0.5.7.md for all changes since that release, including the intervening internal builds, and SOURCE-RELEASE.md for build instructions.

## Features

- **Mushaf reading:** 604-page Quran presentation using Digital Khatt resources, with light, dark, and sepia themes.
- **Reading tools:** page navigation, Quran search, configurable ayah highlighting, and fullscreen reading.
- **Tafsir:** access bundled commentary from the reading interface.
- **Recitation:** listen to ayahs and surahs, choose listening ranges, optionally repeat a range or surah, and adjust playback speed and volume.
- **Flexible playback:** seek, skip backward or forward by 10 seconds, and control listening from a compact player.
- **Downloads:** save supported surah recitations for offline listening, with progress and storage information. Recitation downloads are optional.
- **Arabic and English interface:** reading controls adapt to the selected language, with reduced-motion support for interface transitions.

## Download and installation

Download the latest installer or portable build from the [GitHub Releases page](https://github.com/Thrpro/Quran-reader/releases).

For **v0.5.0**, the Windows installer is approximately **12.17 MB** and the installed application approximately **61 MB**, excluding optional recitation downloads.

The Windows desktop host has been migrated from .NET/WinUI to **Rust**. A separate .NET or Windows App Runtime installation is no longer required. The reader **still uses Microsoft Edge WebView2** for its web-based interface. If WebView2 is not already installed, setup may need an internet connection to install Microsoft's runtime.

Reading progress, settings, and downloaded recitations are stored separately from the program files so that an application upgrade can preserve them.

## Quranic sources and integrity

The project uses Quranic resources from the [Quranic Universal Library (QUL)](https://qul.tarteel.ai), developed by Tarteel AI, and font and shaping resources from [DigitalKhatt](https://digitalkhatt.org).

The Quran text, verse order, and Mushaf page divisions are preserved during application and rendering updates. Visual improvements to Arabic shaping, spacing, and justification do **not** change the underlying Quran text. The application's data checks cover **114 surahs, 6,236 ayahs, and 604 Mushaf pages**.

Bundled resources have been checked against the application's build records. A byte-for-byte match of **every** bundled resource to an independently obtained upstream QUL release has not yet been established; the project does not claim such verification.

Quran text, fonts, layouts, translations, tafsir, audio, and other upstream resources remain subject to their respective providers' terms. See [Third-Party Notices](THIRD-PARTY-NOTICES.md).

## Contributing and responsible reuse

Contributions that improve Quranic accuracy, accessibility, usability, performance, reliability, and teaching or study features are welcome. Please open an issue to discuss significant changes before beginning extensive work.

Preserving Quranic integrity is a central requirement for contributions accepted into the **official** project. Proposed changes to Quran text or source data must be supported by authoritative sources and carefully verified; improvements to rendering or presentation must not silently change the underlying text.

The original application code is available under the **MIT License**, which allows reuse, modification, and redistribution, including in independent projects. Modified versions should be clearly identified as **unofficial** and should not suggest endorsement by the maintainer or upstream Quranic resource providers. The MIT License itself does not restrict lawful modification or use; the project maintainer controls which contributions and builds are distributed as official releases. Use of the project's branding is separate from the code license and subject to applicable rights.

## Development and acknowledgments

Quran Mushaf Reader is an independent project developed with substantial assistance from **OpenAI Codex** in implementing and refining the application's software and interface. The Quranic source content is **not AI-generated**; it comes from the upstream providers identified above.

Thanks to the teams and contributors behind Quranic Universal Library, DigitalKhatt, and the open-source components used by this application. Their work is credited in the [third-party notices](THIRD-PARTY-NOTICES.md).

## License

The project's **original application code** is licensed under the [MIT License](LICENSE), © 2026 Thrpro. This does **not** relicense bundled Quranic resources, fonts, recitations, translations, tafsir, or other third-party materials.

See [LICENSE](LICENSE) and [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for details.
