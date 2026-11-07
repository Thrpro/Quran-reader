# Quran Mushaf Reader — review build 0.5.13

This local review follows 0.5.12. Public v0.5.0 corresponds to internal v0.5.4; earlier patch notes retain the cumulative history. Existing release tags and downloads are preserved.

- Prepare each visible page's required fonts directly in its source-loading pipeline before publishing ready page state. This addresses a reproduced startup stall where source data was present but decorative fonts had never started loading. Text-only pages still need only their text font.
- Keep listening settings unloaded until Play, Resume or a direct Listen-from-here request. Explicit Stop synchronously cancels and disposes media, revokes owned playback URLs and leaves no replacement paused recording. Pausing still retains the current timeline for quick continuation.
- Preserve the selected surah/ayah/range independently of loaded media. Reopening Listen after Stop or an unplayed selection cannot replace its surah with the reader's current page while retaining an unrelated range.
- Limit highlight/recitation changes to the pages and lines containing the affected ayahs. Cache immutable source-run and shaped-word geometry; retain original visible/accessibility text, outlines, canvas density and coordinates.
- Build contextual fold copies without first cloning invisible interaction spans or blank canvas placeholders. Ordinary text/page chrome keeps the native cloning path. Folds retain the original source pixels and animation geometry.
- Index offline-library metadata in memory, bound header reads to four, reuse stable chapter URL sets and guard selection/invalidation races. Cache Storage remains authoritative; listing reads no audio bodies and cancels unused response streams. Failed scans retain their worker ownership until cleanup, so immediate retries cannot exceed the concurrency limit.
- Request WebView2's LOW memory target only while minimized and restore NORMAL before showing the reader. Scripts, audio and downloads remain active; no suspension, forced garbage collection or working-set trimming is used. Older runtimes retain the existing behavior if the optional API is unavailable.

The timing-service fallback proposed as option 6 is not included. No new user-facing modes or optional practice/bookmark features are added in this performance pass. Quran text/fonts/outlines/page resources and recitation bytes/bitrate are unchanged.

New regressions cover resource lifetime, selection preservation, cache races, scoped highlighting and fold allocations. The actual native QA host is measured separately with an isolated profile and native MP3 decoding; a short accelerated fixture does not establish long real-provider playback or universal RAM/FPS figures. MEMORY-PERFORMANCE.md records measured scope and limitations. The shipping host excludes QA commands and debugging flags.

The source and compiled 0.5.12 app are backed up under outputs/backups/performance-from-0.5.12-2026-10-26. This work does not commit, push or publish a release.
