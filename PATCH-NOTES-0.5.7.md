# Quran Mushaf Reader — development build 0.5.7

## All changes since your last released app

These notes cover the complete update from **public v0.5.0 (internal v0.5.4)** to the current development build, including the intervening internal 0.5.5 and 0.5.6 changes. They are not limited to the latest memory fix. This build remains a review candidate; a commit or push does not automatically publish an app release.

### Listening and playback controls

- Prefer a single continuous surah recording for Yasser Al-Dosari and Mishary Alafasy where matching ayah timings are available, retaining follow-along highlighting. Other reciters retain ayah playback. The recording preference is now saved.
- Keep a choice between separate ayah recordings and a single surah recording. A single recording can be a different performance by the same reciter; both options download the complete surah.
- Replace the earlier small playback overlay with a dedicated player below the reading area. It shows the surah, current ayah, progress slider, elapsed time and total recording duration.
- Add accessible player controls for pause/resume, backward/forward 10 seconds, listening settings and stop. Separate these actions from the recording title.
- Reserve space for the player so it does not cover the Quran's bottom lines. The pages fit into the remaining reading area, and the player adapts to narrower windows. A side-mounted or automatically collapsing player is not implemented in this build.
- Reduce unnecessary playback-clock updates and show remaining time relative to the active listening range and playback speed.

### Quran rendering and page turns

- Fix the unwanted contour lines produced by the earlier dark-mode contextual-lettering stroke correction. Apply contrast correction to ink coverage without drawing extra strokes around glyph contours.
- Paint the supplied Digital Khatt outlines directly into reusable, high-resolution canvas buffers instead of retaining decoded SVG image documents. Quran source text, glyph outlines, shaping coordinates, line breaks and word mappings remain unchanged. Canvas and SVG may antialias edges differently.
- Preserve page-fold animations, ayah hit areas, highlighting, selection and accessibility text. Fold artwork copies the existing pixels rather than reshaping the Quran text.
- Refresh contextual artwork when reader resizing finishes and invalidate prepared fold artwork when the system theme changes.

### Memory and efficiency

- Reduce memory growth during repeated page turns, especially with contextual letter elongation enabled.
- Bound the contextual data cache and nearby read-ahead, reuse drawing contexts, and release temporary pixel readbacks immediately where supported. At most 60 unused canvases are pooled before their backing stores expire after inactivity.
- Avoid redundant page renders from equivalent metadata. Remove invisible interaction text from inert fold copies, defer speculative fold preparation during rapid navigation, and release prepared folds sooner.
- In the isolated 60-spread-turn comparison, contextual-mode sampled peak working memory decreased about **56%**, and working memory after 15 seconds idle decreased about **64%**. Normal-mode working memory was essentially unchanged; contextual startup allocation increased slightly. These are Edge process-tree measurements, not a guarantee of Task Manager WebView2 totals. See MEMORY-PERFORMANCE.md for the full results and limits.

### Verification and source maintenance

- Pass 54 application unit tests, 5 native tests, Quran/study asset validation, reader navigation and playback regression checks.
- Add repeatable memory-soak checks, source-resource checks, and contextual artwork/fold checks at 100%, 125% and 200% display scaling.
- Update build records, source checksums and Rust-host documentation for the current source. Preserve previous release files and the pre-change backup.

Changes already present in internal v0.5.4—such as the localized startup message, Arabic title-bar tools and nested right-click listening choices—are not presented as new features here. The current player and memory work do not constitute a new fix for every earlier audio or UI issue.
