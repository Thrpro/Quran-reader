# Build verification for 0.5.8

The GitHub clone was checked using the existing locked dependencies and local compiler caches: Quran/study asset validation, the generated ayah map, 81 unit tests, TypeScript/Vite production build, 5 native tests, Rust release build and packaged-web file verification. This is not a pristine-PC installation test. No public release or source commit is created by these checks.

Browser checks use isolated Edge profiles and authentic local audio fixtures. They cover reader navigation, retry after page failures, search/study, keyboard ayah actions and focus, source quotation integrity, contextual ink/fold copies, playback speed, pause/resume, range repetition, continuation without restarting, downloads with timing metadata, seek, saved preferences and resume. Both the browser shell and native title-bar layout are covered. The side player leaves page sizes unchanged; narrow layouts retain compact controls.

Contextual artwork is compared with the supplied SVG outlines and fold copies are compared with current canvas pixels at device scales 1, 1.25 and 2. Reduced motion mounts no preview artwork; normal motion retains four previews. These buffer counts are not total WebView2 process-memory measurements. Historical 0.5.7 process measurements and their limits remain in MEMORY-PERFORMANCE.md. This version makes no new hardware CPU/GPU percentage or universal RAM-ceiling claim.

Offline/recovery tests cover missing or corrupt timings, quota failures, incomplete MP3s, temporary failures, bounded retry, exact position/speed/pause preservation and premature playback-end events. Fixtures and simulated failures do not establish live-provider uptime or replace manual listening review. Live provider checks and their sampled scope are described in AUDIO-PROVIDERS.md.

The production web build records its source/content fingerprint. The native builder refuses stale source or modified web output, and the portable packager verifies all payload files and the completed ZIP without overwriting an existing archive. RUST-HOST-BUILD.json records the source commit, dirty state, content fingerprint and tool versions; this provides traceability rather than a guarantee of byte-identical compiler output across machines. The reviewed portable ZIP is built from this clone and retains its current uncommitted-source identity.

The first hosted GitHub Actions run remains necessary to validate its runner environment. CI stores test artifacts and does not publish them automatically. Automated checks complement manual reading/listening and installed-app review.
