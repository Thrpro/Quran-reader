# Quran Mushaf Reader 0.5.4

- Arabic reading tools now appear on the right of the title bar. English keeps the tools beside the app name. Switching language uses a short position transition; reduced-motion preferences disable it.
- The Arabic Options dropdown opens beside its title-bar controls.
- Extra right-click listening ranges expand within the listening action, with an indented list and keyboard navigation, instead of a separate section.
- Replaced the ambiguous continuous-recording checkbox with a collapsed Recording options section. Both audio sources play consecutive ayahs; the alternative uses one surah recording with matching ayah timings. Both download the complete surah.
- Startup shows the existing app icon, a small loading indicator and one localized message: “Loading…” or “جارٍ تحميل التطبيق…”. The initial native window shows the icon and dots until the saved interface language becomes available.

The default audio source and Quran text, fonts and page layouts are unchanged. A source and installer backup of 0.5.3 was saved before editing.

Validation: TypeScript/production build, 40 unit tests, 5 Rust host tests, all 604 Quran pages and study resources, dependency vulnerability scans, Arabic/English toolbar layout, keyboard navigation, startup localization, source-text rendering, page animations and playback regression checks passed. The audio regression includes more than 40 minutes of simulated media timeline with bounded buffering and outage recovery.
