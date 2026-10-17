# Build verification for 0.5.7

The GitHub clone was built independently: Quran/study asset validation, 54 unit tests, TypeScript/Vite production build, 5 native tests, Rust release build and all 1581 packaged web files verified. The portable ZIP is built from this clone. The npm lockfile and previously installed dependencies/compiler caches were reused; this is not a pristine-PC installation test.

Browser regression checks on the matching application build cover navigation, contextual ink, continuous-surah default, playback speed, pause/resume, ranges, downloads, +/-10-second seek, and player/page clearance at 1920x1051, 1024x600 and 390x844. Both browser and native-title-bar layouts were checked in Edge. Long-playback testing uses local fixtures and simulated connection failures; live provider uptime and hardware GPU usage are not established by these tests.

The final web build matches the fingerprint in the 60-unique-spread memory soak. No garbage collection was forced. Contextual-ink comparison against the former SVG source outlines and exact canvas copies during forward/backward folds passed at device scales 1, 1.25 and 2. Drag cancellation and committed navigation also passed. See MEMORY-PERFORMANCE.md for measured process memory, startup tradeoffs and limitations.

The first hosted GitHub Actions run remains necessary to validate that runner environment. Automated checks complement manual listening and reading review.
