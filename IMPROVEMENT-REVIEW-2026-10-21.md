# Follow-up review — 2026-10-21

The 0.5.9 experiment covers the bottom player, layout observer overhead, source-safe saved positions, selection play/pause intent and the unsupported native diagnostic-save action. These are local review changes; they do not establish lower total WebView2 RAM or GPU use on every device.

## Implemented in the 0.5.10 review

1. **Startup retry and font priority.** Localized metadata retry, cancellation and timeout replace raw failures. Opening pages wait only for their required fonts, with supplied heading fonts prepared afterward. A required font failure uses document reload because browsers can retain failed FontFace loads.
2. **Reciter-picker focus and repeated work.** Results no longer add Tab stops; selection retains input focus. Stable callbacks/options and memoized ranking avoid rebuilding the list for every playback-clock update. Arabic/English pointer and keyboard fixtures cover the behavior.
3. **Canvas storage and lifetime.** Released storage has a 32 MiB byte budget. Mounted canvases survive theme/size changes. Conservative transparent-row trimming retains every original ink pixel at verified display densities; the high-density fallback preserves established placement. Source-wide bounds, SVG comparisons and exact fold copies guard reading quality.
4. **Bounded clip recovery.** The clip engine now stops after four automatic retries and offers in-place manual recovery without skipping a verse. Paused/finished players have no watchdog interval. The timed whole-surah engine's existing finite recovery policy remains.

## Further work

The 0.5.11 follow-up adds a saved side/bottom player choice, shorter bottom controls, Arabic progress direction and one-second keyboard seeking. It removes duplicate intermediate-layout rasterization, keeps neighboring preview density stable across bottom-player transitions, suppresses unused fold preparation during automatic follow-along, and tightens cached-audio consumption/cancellation and completed-stream lifetimes. The before/after listening procedure and measured limits are retained in MEMORY-PERFORMANCE.md. PATCH-NOTES-0.5.11.md describes the review behavior and backup location.

Profile the compiled WebView2 app on representative integrated GPUs and different display scales, including a long real-provider listening session. Browser fixtures cannot establish hardware percentages or live-provider availability. Investigate high-density cropping only if it also passes the original software-rendering placement checks; do not lower text quality for a memory target. Keep new features subject to the existing source/build identity and authentic-resource checks.

## Reading integrity

Do not generate or substitute Quran glyphs to create UI demonstrations. The review screenshots come from the reader's existing authenticated local resources. Font/outlines/data remain under their existing provider terms. GPU/CPU/RAM improvements need measurements on representative Windows hardware; browser layout assertions alone cannot establish those claims.
