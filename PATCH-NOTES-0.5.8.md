# Quran Mushaf Reader — development build 0.5.8

These notes cover changes since **public v0.5.0 (internal v0.5.4)**, including internal builds 0.5.5–0.5.7. This is an unpublished review build. Committing or pushing source does not publish a release.

## Listening

- Move the compact player into the available side gutter, preserving the size of the Mushaf pages. Narrow windows use compact title-bar controls and an expandable side panel. Pause/resume, seek, ±10 seconds, settings and stop remain available.
- Keep the familiar main reciter picker. Additional recordings appear under **More reciters / المزيد من القرّاء**. Opening this list does not change or restart the current recitation.
- Choose a matching continuous surah recording automatically for 11 verified main reciter/style choices, with ayah highlighting. Remove the user-facing recording-source selector and “recommended” wording. Other main reciters keep their ayah clips; additional recordings without verified timings are clearly labeled.
- Save a whole surah for offline listening. Timed recordings now save and validate the matching timing table alongside the MP3 before reporting completion.
- Preserve position, playback speed and pause intent during bounded recovery from temporary failures. A premature playback-end event cannot advance to the next surah. Changing chapter continuation no longer rebuilds the current recording.
- Save reciter, speed, volume and continuation preferences. Offer to resume the previous listening position; audio never starts automatically when opening the app. Explicit Stop clears that position.
- Retain range repetition and seeking controls introduced after 0.5.4. Show buffered audio instead of claiming a total remaining time when the clip timeline is not fully known.

## Reading and efficiency

- Preserve the Quran text, supplied Digital Khatt outlines, coordinates, line breaks and authentic page resources. Fix the contextual-ink contour artifacts introduced by the earlier contrast correction.
- Retain smooth fold animations and four neighboring preview pages in normal motion mode. With reduced motion, omit unused preview artwork and fold preparation entirely. Preview canvas storage is temporary uncompressed RAM, not extra download or installation size.
- Bound and prioritize optional page-data work; visible pages take priority and obsolete queued previews are dropped safely.
- Locate playback and tafsir ayahs using a small generated page map rather than loading the full text-search index. Study quotations are reconstructed from authenticated page words; all 6,236 references are checked against the source.
- Add keyboard access to visible ayahs: Tab into the spread, Up/Down or Home/End to choose an ayah, and Enter, Space or Shift+F10 for its actions. Restore focus after closing the menu.
- Keep the earlier canvas/context pooling, bounded caches and rapid-navigation memory improvements. Historical 0.5.7 soak measurements and their limits remain in MEMORY-PERFORMANCE.md; they are not a new 0.5.8 process-memory claim.

## Reliability and builds

- Add localized retry/reload recovery for rendering failures and optional diagnostic export containing only non-sensitive app/environment information.
- Extend automated checks for offline metadata, interrupted playback, continuation, preferences/resume, keyboard actions, contextual ink, fold copies and responsive player layouts.
- Pin CI actions and the Rust toolchain, scan locked dependencies, and retain tested portable builds. Build manifests record source identity, tool versions and payload hashes. Packaging verifies its contents and refuses to overwrite an existing ZIP.
- Follow-up CI fix (2026-10-18): recognize Windows temporary-folder aliases when checking Git source identity. Keep stale-source and modified-output rejection, and verify that a nested source export cannot inherit its enclosing repository's identity.
- Follow-up browser-check fix (2026-10-18): verify intermediate fold geometry without assuming a shared CI runner renders five frames during every quick swipe. Retain navigation, animation callbacks and idle-cleanup checks; add slow-CPU validation and failure reports. These changes affect development checks, not the app's animation speed.

Provider checks on 2026-10-17 found 115 timing reads. The curated mapping uses exact recording IDs/folders and sampled complete chapter tables; three matching styles with malformed Tawbah numbering remain on clips. Each chapter is validated again when loaded. This does not establish provider uptime or certify every live recording. See AUDIO-PROVIDERS.md and BUILD-VERIFICATION.md.
