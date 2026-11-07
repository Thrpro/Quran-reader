# Page-turn memory checks for 0.5.7

## Rendering and resource ownership

Contextual lettering uses the supplied Digital Khatt glyph outlines and coordinates directly. Canvas painting replaces per-line SVG image documents; Quran text, glyph paths, line placement, and source-word mappings remain unchanged. The backing resolution includes display density and reader zoom, with at least 2x sampling to keep small marks readable.

The reader retains 32 source-data pages and 12 contextual-data pages. Contextual read-ahead covers the nearby reading window; only the visible spread and up to four neighboring preview pages mount artwork. Parsed `Path2D` geometry uses a 256-entry LRU per outline collection.

Page folds copy existing canvas pixels into CPU-backed canvases. The reader and folds share a pool of at most 60 idle canvases, preferring matching pixel dimensions and reusing other idle contexts when sizes change. Idle backing stores are reset after 1.5 seconds without a release. Active artwork remains independent. Exact pixel extents avoid a loss of sharpness from texture padding. Fold descendants are removed on cleanup. Temporary alpha-correction buffers are detached after use when the runtime supports buffer transfer. Browser and graphics caches can still retain allocations beyond the lifetime of these objects.

Fold preparation waits until navigation rests, then expires. Theme changes invalidate prepared fold copies. Reader layout animations trigger one final artwork refresh so temporary animation scaling does not determine the resting bitmap resolution.

## Repeatable soak procedure

Run on Windows with dependencies installed and Microsoft Edge available. Keep the previous packaged web build intact until the baseline run finishes: `--baseline` reads `outputs/rust-windows/publish/WebApp`, while the current run reads `dist`.

```powershell
node scripts/verify-memory-soak.mjs --baseline
npm run build
node scripts/verify-memory-soak.mjs
node scripts/verify-contextual-ink.mjs
node scripts/verify-contextual-folds.mjs
```

The soak starts a fresh, isolated headless Edge process tree for each mode: normal text and contextual lettering. At 1920x1080 and device scale 1, it starts on pages 45-46 and sends 60 spread turns to distinct destinations in bursts of ten, ending on pages 165-166. It samples between bursts and after 15 seconds of natural idle. It never requests garbage collection.

Reports under `outputs/memory-soak-qa/` include build fingerprints, browser version, process memory, JavaScript heap, canvas storage estimates, DOM counts, and retained fold/preview counts. The ink and fold checks additionally cover device scales 1, 1.25, and 2, source-artwork comparisons, and exact canvas copies during folds. Repeat measurements on the same machine and browser version; run only one performance probe at a time.

## Results and limits

Measured on 2026-10-17 with Edge 155.0.4283.62. Both runs used the procedure above; no other browser performance probe ran concurrently. The final built index matches the measured candidate fingerprint.

| Mode | Previous sampled peak private MiB | Previous idle private MiB | 0.5.7 sampled peak private MiB | 0.5.7 idle private MiB |
| --- | ---: | ---: | ---: | ---: |
| Normal text | 914.3 | 632.4 | 906.0 | 717.6 |
| Contextual lettering | 2670.6 | 2064.5 | 1615.4 | 1128.7 |

| Mode | Previous peak working MiB | Previous idle working MiB | 0.5.7 peak working MiB | 0.5.7 idle working MiB |
| --- | ---: | ---: | ---: | ---: |
| Normal text | 616.4 | 505.3 | 612.0 | 505.8 |
| Contextual lettering | 2563.9 | 1963.2 | 1139.2 | 713.4 |

Contextual sampled peak working memory decreased about 56%, and idle working memory about 64%. Its sampled peak private bytes decreased about 40%, and idle private bytes about 45%. Browser task time for that navigation sequence decreased from 50.2 to 24.9 seconds (about 51%); this is accumulated browser work, not a CPU percentage or a frame-rate guarantee. Normal text mode's working memory was essentially unchanged, and its idle private bytes increased in this run; no general normal-mode RAM saving is claimed.

This optimizes sustained navigation, not startup. Initial contextual working memory increased from 658.7 to 735.4 MiB as the reader explicitly prepares high-resolution canvas buffers. Initial normal-mode working memory was essentially unchanged (498.8 to 494.2 MiB).

Contextual idle JavaScript heap decreased from 44.0 to 8.6 MiB, and reported DOM nodes from 43,784 to 3,032, while live DOM elements remained around 1,300. Temporary ArrayBuffer storage stayed near 6 MiB in the new rapid-turn run instead of accumulating hundreds of MiB. No folded page remained after idle.

Candidate index SHA-256: `3a7a93457a018d9107a6b65d7055f3bd97a3cabef0f2d0641d2a0612e0676edb`; entry script `index-CB40dp_w.js`. The local reports preserve the baseline fingerprint and every sampled process breakdown. Reports and screenshots are development outputs, not application payload.

Private bytes measure committed private process memory; they are not a direct resident-RAM measurement. Summed working sets can count shared pages more than once. These measurements cover the isolated Edge process tree, not the installed WebView2 host or its Task Manager group. Sampling between bursts can miss shorter peaks.

JavaScript heap and live canvas byte counts omit browser-native caches, command buffers, and graphics-driver allocations. A finite cache may keep memory above launch usage after navigation without indicating an unbounded leak. Inspect both the navigation curve and repeated post-idle levels. Passing these checks does not establish a particular CPU/GPU percentage, a universal RAM ceiling, or identical behavior on every device; manual testing of the compiled app remains necessary.

## 0.5.8 preview policy

The normal-motion reader retains four neighboring artwork previews so page folds can use prepared pixels. Four pages can consume tens of MiB because each canvas stores uncompressed RGBA pixels at its backing resolution; their JSON download size is much smaller. This memory is not added to the installed app size. Removing every preview from the normal animation path would trade memory for synchronous artwork preparation during navigation, so this build preserves them.

Reduced motion already skips fold animations; 0.5.8 also skips their unused preview artwork and preparation. A 1920x1080 native-layout regression measured 65,711,096 bytes of live preview canvas backing storage with normal motion and zero with reduced motion, while the visible contextual pages remained rendered. This is a live-buffer count, not a measured reduction in total WebView2 process RAM. The earlier 60-turn process-memory results above belong to 0.5.7 and are retained as historical evidence.

Page-data requests are bounded to four concurrent loads, prioritize visible pages, share in-flight work and discard obsolete queued optional requests. Playback/study navigation now loads a compact page-reference map instead of the full 3.84 MB search index. Search itself still loads its index when requested. Run `node scripts/verify-listening-refinements.mjs` (or add `--browser`) to verify previews, compact lookup and unchanged page size during playback; reports are written under outputs/listening-refinements-qa.

## 0.5.10 rendering and listening work

Measured on 2026-10-21 with Edge 155.0.4283.62 using the same isolated 60-spread procedure, viewport, display scale and natural collection policy above. A saved 0.5.9 publish is the baseline. Two optimized runs are retained: the first has the preceding diagnostic version string, and the confirmation run identifies as 0.5.10. Neither ran alongside another browser probe or compiler. These are development-machine observations, not a multi-device benchmark.

| Contextual mode metric | Saved 0.5.9 | Optimized run | 0.5.10 confirmation |
| --- | ---: | ---: | ---: |
| Peak live canvas backing storage, MiB | 134.2 | 70.9 | 70.9 |
| Idle live canvas backing storage, MiB | 100.7 | 52.0 | 52.0 |
| Sampled peak process private bytes, MiB | 1952.0 | 1802.4 | 1472.9 |
| Idle process private bytes, MiB | 1106.4 | 1176.0 | 1142.2 |
| Sampled peak renderer private bytes, MiB | 1024.7 | 707.0 | 402.9 |
| Accumulated navigation browser task time, seconds | 38.6 | 29.8 | 33.8 |

Peak live canvas storage is about 47% smaller; accumulated contextual navigation work is about 12–23% lower. The variation in renderer/process peaks is substantial, and idle total private bytes increased about 3–6%. GPU private bytes did not improve. Do not describe this as a universal total-RAM reduction, an FPS guarantee or lower hardware CPU/GPU percentages. Normal text sampled peaks were 1088.2 MiB before and 981.1/975.4 MiB afterward, but its accumulated task time varied from 11.5 seconds before to 9.6/11.5 seconds afterward; no reliable normal-mode speedup is established.

The crop removes only transparent rows; the original full viewport and physical drawing resolution are retained. Bounds cover every supplied path/control point on all 604 shaped pages, including marks and offsets. Exact cropped/full-canvas pixels, the original SVG outlines and fold copies are checked separately. Precise 2D placement avoids CSS length rounding at standard/125% display density. At density 1.5 or higher, the established full-canvas presentation remains because the cropped transform did not meet the original software-rendering centroid limit there. No threshold was weakened to enable it.

Released buffers now obey a 32 MiB byte budget in addition to their count/expiry bounds. Mounted line canvases/observers are retained during theme and layout changes. Visible jobs are painted before optional batches; a fold flushes its exact source pages before copying. Four normal-motion previews remain, and reduced motion still avoids them. The lifecycle fixture recorded no canvas/observer replacement during four theme choices and two real resizes, and no repaint for twelve identical-geometry resize notifications after layout settled. These counts concern the application's drawing objects, not all browser allocations.

Audio recordings/bitrate and the existing compressed rolling-buffer policy are unchanged. The clip watchdog sleeps while paused/finished/disposed, and permanent future-clip errors have a finite recovery budget. This reduces idle callbacks and failed requests; it is not a measured whole-process listening RAM reduction. Fixtures verify that manual retry retains the healthy timeline/offset/speed and never skips the failed verse.

Reports remain under ignored outputs/memory-soak-qa: memory-before.json, memory-cropped-comparison.json and memory-after.json. The confirmation entry is index-B8VK8zrS.js; index SHA-256 is a490262920e3f241dfc6b18033940267787f1f0c22b4d78b78ca9b5c95639130. The final native build manifest records the packaged web hashes; compare them before treating the measurements as applicable. Re-run the probe if application output changes. The earlier Edge-versus-WebView2, sampling, shared working-set and graphics-cache limitations still apply.

## 0.5.11 listening and player-layout comparison

Measured on 2026-10-21 with Edge 155.0.4283.62, at 1920x1080 and display scale 1. Each natural/contextual and network/cached case starts a fresh isolated Edge process tree. No compiler or other browser regression runs concurrently; no garbage collection or working-set trimming is requested. The saved 0.5.10 published WebApp is the immutable baseline.

The fixture contains 23,196,713 bytes (22.1 MiB) of repeated authentic Ikhlas MPEG frames, with corrected Info frame/byte counts and seek table. The native decoder verifies its 24.16-minute duration. Synthetic timing entries exercise 286 verse references on authentic Baqarah pages; the repeated audio does not correspond to those verses and is never shipped as a recitation. Native media timers and decoding remain active. Explicit seeks accelerate 25 follow-along spreads. Each case also covers stationary playback, six paused player hide/reveal cycles, eight reciter switches, native ended/repeat/completion, Stop, and 15 seconds of natural idle.

| Mode / source | 0.5.10 sampled peak private MiB | 0.5.11 sampled peak private MiB | 0.5.10 Stop + 15s private MiB | 0.5.11 Stop + 15s private MiB |
| --- | ---: | ---: | ---: | ---: |
| Natural / network | 977.3 | 801.2 | 482.1 | 466.3 |
| Natural / saved audio | 917.5 | 967.3 | 513.4 | 527.9 |
| Contextual / network | 1899.3 | 1462.2 | 1099.0 | 990.5 |
| Contextual / saved audio | 1732.7 | 1557.3 | 1108.2 | 1051.9 |

Contextual sampled peaks decreased about 23% for network and 10% for saved playback; post-Stop private memory decreased about 10% and 5%. Natural/network improved in this run, but natural/saved peak private memory increased about 5% and its post-Stop private bytes increased about 3%. These results do not establish a universal RAM reduction. The cached single-consumer validation path improves cancellation/ownership; the measurements do not establish that it reduces native Blob storage. Do not attribute the measured overall reduction to that path alone.

The original contextual/network run reached about 1,116 MiB GPU private memory and 700 MiB renderer private memory at its sampled peak, with only about 27 MiB JavaScript heap and 52 MiB of live canvas pixels at that sample. Paused player layout changes also grew native memory while the clock, live element count and canvas count stayed fixed. The fixes avoid painting existing canvases at intermediate layout scales, keep all four hidden neighbors at stable full reading-area density during bottom-player transitions, and avoid speculative fold copies while listening/player space is reserved. Manual commands retain the same folds and source pixels. Peak live contextual canvas storage in the bottom-player fixture fell from about 66.3 to 52.0 MiB; idle storage stayed about 51.8 MiB. Smaller total native allocations must still be measured separately from these live-buffer counts.

Every final bottom-player case verified zero speculative folds during player-only layout changes. Contextual cases also verified unchanged hidden-preview canvas dimensions while collapsing/revealing the player. Cached profiles requested no remote MP3s and retained at most one live playback Blob URL across eight source changes; old URLs were revoked. Stop currently prepares one paused recording, so this is bounded source ownership rather than a claim of zero retained media resources.

A separate 0.5.11 contextual/network side-player run peaked at 1190.9 MiB private and settled at 774.4 MiB after Stop + 15s. At this wide window it uses an existing gutter and does not resize the pages when collapsed/revealed. Its peak live canvas count was larger (69.7 MiB) while total process private memory was lower. This illustrates why backing-byte totals alone do not predict browser/graphics-cache retention. It is one placement comparison on one machine, not a guarantee for narrow windows or other hardware.

Run `npm run test:listening-memory`, or `node scripts/verify-listening-memory.mjs --placement=side --modes=contextual --sources=network`, separately from builds and other regressions. The immutable-baseline command is `node scripts/verify-listening-memory.mjs --baseline` before replacing the published WebApp. Reports are under outputs/listening-memory-qa: listening-memory-before.json, listening-memory-after.json and listening-memory-after-side.json; intermediate experiments are preserved separately. The final measured entry is index-CMSgX6U9.js and index SHA-256 is 156d0003f188f2dcd663ffde1d19d0f07f8c2a05d1a1c1ad230164704655ce7c. Compare entry/CSS hashes with the final packaged output after documentation/source-record rebuilding.

These are sampled private/working bytes for isolated headless Edge, not the installed WebView2 host's Task Manager group. Summed working sets can double-count shared pages; private bytes are committed memory, not directly resident RAM. Sampling can miss short peaks, and native/driver caches can outlive application objects. There is no hardware CPU/GPU percentage, fixed RAM ceiling, live-provider uptime or long uninterrupted listening guarantee here. Manual testing of the compiled WebView2 app remains necessary.


## Layout follow-up in 0.5.12

The compact side player now uses an existing page gutter or the toolbar without a side-space reservation. Settings/search hide the preserved controls; tafsir uses compact toolbar controls for either placement. Fullscreen removes its permanent side columns. Rendering density and preview quality are retained. Page-bound and artwork/lifecycle regressions check those geometry and quality contracts; no new listening-memory benchmark was run for this layout-only follow-up. The measurements above refer to their explicitly identified 0.5.10/0.5.11 builds and must not be relabeled as 0.5.12 results. More visible Quran pixels in previously width-constrained windows can legitimately require larger canvas buffers.


## 0.5.13 actual WebView2 QA-host comparison

Measured on 2026-10-26 with the Rust/Win32 host and WebView2 155.0.4283.62, using isolated profiles and the original compiled 0.5.12 web payload as baseline. The QA-only host adds a debugging port; candidate QA messages also report the real minimized memory-target state. Shipping builds exclude these features. Client size is 1280×800 CSS pixels at Windows display scale 1.25. No other compiler, browser regression or performance probe ran concurrently, and neither garbage collection nor working-set trimming was requested.

The native decoder plays the same 22.1 MiB repeated-authentic-Ikhlas fixture described above, with synthetic Baqarah timings only for exercising follow-along pages. This test audio is never shipped as a recitation and its words do not match those synthetic references. Network responses use byte-range-aware development interception; cached cases seed real Cache Storage. The probe covers unloaded/menu preparation, native playback, six accelerated follow-along destinations, minimize/restore while playing, explicit Stop plus eight seconds of idle, twenty discrete page turns, and another eight seconds of idle. Development transport and a short accelerated fixture are not live-provider throughput, long uninterrupted listening or a multi-device benchmark.

Two paired runs are retained. Task values are renderer Performance.TaskDuration differences (seconds), not CPU percentages or total GPU work. Memory values are sampled process-tree private MiB for the isolated Rust/WebView2 processes; working sets and per-process details remain in the JSON reports. Sampling can miss peaks, shared working pages can be counted more than once and driver/browser caches can outlive application objects.

| Run | Mode / source | Follow-along task seconds, before → after | 20-turn task seconds, before → after | Peak private MiB, before → after | Stop + idle private MiB, before → after | Navigation + idle private MiB, before → after |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | natural / network | 0.458 → 0.238 | 2.715 → 1.824 | 713.1 → 730.4 | 413.5 → 451.0 | 547.1 → 555.9 |
| 1 | contextual / cached | 1.399 → 0.720 | 4.160 → 4.291 | 1459.2 → 1425.8 | 576.5 → 576.1 | 1060.6 → 1141.6 |
| 2 | natural / network | 0.222 → 0.232 | 1.779 → 1.777 | 704.9 → 687.8 | 408.1 → 413.1 | 511.4 → 520.1 |
| 2 | contextual / cached | 0.714 → 0.704 | 4.326 → 4.383 | 1373.7 → 1257.5 | 584.6 → 574.7 | 1078.3 → 1049.2 |

The first-run natural navigation/follow-along and contextual follow-along timing improvements did not reliably repeat: second-run natural navigation differed by only about 0.1%, natural follow-along was about 4% higher, and contextual follow-along differed by about 1.4%. Contextual navigation work was slightly higher in both pairs. Warm-up, browser/driver caching and run-to-run variation prevent attributing a general CPU or wall-time speedup to this change. Peak and idle private bytes are mixed, including a higher first-run natural Stop level and a higher first-run contextual post-navigation level. These results do not establish a universal total-RAM reduction, fixed memory ceiling or contextual FPS improvement. Retained improvements are supported separately by exact resource lifetime and deterministic reduced-work checks.

Every candidate opens listening settings without a media source/MP3 request, and explicit Stop leaves zero playback Blob URLs, no source attribute, NETWORK_EMPTY, no decoded readiness/buffer and no replacement request. Chromium can retain the historical currentSrc string after an empty load; that text alone is not an active source or proof of retained audio. The baseline prepares media on menu opening and retains one cached playback Blob URL after Stop. Both candidate runs confirm the actual WebView2 LOW target while minimized and NORMAL after restore, with native audio continuing; this best-effort browser hint is not forced collection or proof of lower private commitment.

The separate render-work fixture compares the same authentic pages: 59 unrelated line-prop updates per tested ayah transition fall to zero in both text modes. A contextual fold creates zero discarded interaction-span clones instead of 34, and the probe counts 128 cloned elements instead of 311. These are case-specific allocation counts, not an FPS benchmark. Original SVG/pixel-copy regressions remain the quality guards.

Reports: outputs/native-performance-qa/native-performance-before.json, native-performance-after.json, native-performance-before-repeat.json, native-performance-after-repeat.json and comparison.json; structural reports are in outputs/render-work-qa. Before treating results as applicable to an artifact, compare their JS/CSS/index hashes with the packaged web payload and the QA host's native-source hashes. Documentation/source-record finalization may rebuild provenance files without changing application assets. The experiment uses QA binaries rather than byte-identical shipping binaries.


The two paired timing/memory comparisons above use the performance candidate before the final visible-page/font handoff correction. Their exact fingerprints remain in the reports and must not be relabeled as measurements of the later JS bundle. A separate final native QA-host run is retained as outputs/native-performance-qa/native-performance-after-startup.json and binds its own index/asset hashes to the delivered payload. It rechecks source unloading and minimized playback on that final bundle; one additional run does not establish new RAM or speed savings.

The first final native attempt is preserved as native-performance-after-startup-blocked-fixture-failure.json: network playback was repeatedly retrying before the minimize assertion failed. This is not evidence that minimization alone caused it. The probe now awaits PowerShell sampling/window helpers asynchronously so its own CDP media responses can continue, and records helper and response-fulfillment durations. Its measurement procedure therefore differs from the earlier paired runs; do not infer savings across that procedure change.

The asynchronous-helper diagnostic retained as native-performance-after-startup-slow-cdp-failure.json measured full-body CDP fulfillment at 9.1–11.6 seconds, exceeding the app's existing 8-second stall watchdog before reliable playback. The final fixture therefore serves at most 512 KiB per ranged response with the original MP3 bytes and accurate total Content-Range. This changes only QA transport, not app retry policy or recitation quality; old full-body probe timing/memory values are not comparable as a controlled before/after pair with this final run.

The final ranged-fixture run completed both natural/network and contextual/cached scenarios. Network response fulfillment stayed below 100 ms; cached playback requested no remote MP3s. Both modes confirmed unloaded listening settings, source/URL release after Stop, uninterrupted playback through minimize/restore and LOW→NORMAL policy state. Sampled contextual memory still peaked around 1413 MiB during twenty rapid page turns and settled near 1034 MiB after eight seconds; this is not a solved total-memory ceiling. Functional ownership and allocation improvements are confirmed separately from the mixed overall memory results.


## 0.5.14 retained-page and fold comparison, 2026-10-27

The preserved 0.5.13 QA app and 0.5.14 candidate use the same actual Rust/WebView2 runtime, isolated profiles, 1280×800 CSS client pixels at DPR 1.25, cached authentic long MP3 fixture and synthetic timing references, twenty queued page turns, and a sixty-second post-turn idle. No forced GC or working-set trim is used and no other builds/browser checks run concurrently. These are sampled process-tree private MiB, not a Task Manager resident-memory guarantee.

| Run | Sampled peak MiB | After 8s MiB | After 60s MiB | Turn task seconds | Turn wall ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| retention-before | 1389.8 | 1029.7 | 873.2 | 4.292 | 8672 |
| retention-after | 1398.3 | 1032.9 | 888.5 | 3.990 | 8444 |
| retention-2d | 1198.8 | 920.7 | 914.6 | 3.936 | 8320 |
| retention-demand | 1113.3 | 900.4 | 893.9 | 3.965 | 8409 |
| retention-sleep | 1164.7 | 1066.3 | 864.6 | 3.882 | 8303 |
| retention-before-repeat | 1462.6 | 1047.6 | 1027.7 | 4.219 | 8574 |
| retention-final | 1118.4 | 884.3 | 869.4 | 3.870 | 8282 |

Slot reuse alone retained articles/contexts and reduced some work but did not lower total memory. Removing forced 3D promotion for bitmap folds and speculative idle fold copies reduced sampled peaks. The complete candidate additionally releases dormant preview stores: final runs peaked at 1164.7 and 1118.4 MiB, versus baseline 1389.8 and 1462.6 MiB (about 16–24% lower). Turn task time was about 8–10% lower and wall time about 3–4% lower in these pairs. This is one machine/fixture, not a universal speed/RAM promise.

Both matched-repeat apps delivered all twenty animations; median and p95 native RAF callback intervals were 7.0/7.1ms. The candidate had one maximum interval around 62.6ms versus 15.5ms for baseline, so this does not establish a guarantee against individual hitches or monitor-presented FPS. The final first-turn-after-idle probe retained complete fold artwork. Exact pixel/source and reduced-motion regressions remain separate quality guards.

Idle private memory is still substantial: complete candidates settled near 864.6/869.4 MiB after sixty seconds, while baseline varied between 873.2/1027.7 MiB. Do not claim the user's approximately800MB concern is fully resolved. Sampled breakdown identifies graphics/renderer allocations far above the live canvas stores; releasing hidden bitmaps does not force the browser/driver to purge all graphics caches. The visible pages keep original density and pixels. Microsoft's MemoryUsageTargetLevel guidance warns that LOW may swap memory and affect active performance; the existing conservative minimized policy is retained rather than forcing foreground paging (https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_19).

Reports are outputs/native-performance-qa/native-performance-retention-*.json, including intermediate experiments. The paired measurements precede the final minimized-viewport correction. Final delivery binding uses retention-native-final and the same source/asset checksums; earlier experimental bundles must not be relabeled as the final app.


The final native-host follow-up preserves the1280×800 WebView viewport while Windows minimizes its parent; playback and LOW/NORMAL transitions passed. That final run is native-performance-retention-native-final.json: peak 1186.8 MiB, after60s 816.7 MiB, live canvas storage after idle 8.73 MiB, and first completed turn after idle 387ms including the animation. Its median/p95 RAF intervals remain7.0/7.1ms and its maximum is 63.1ms. These numbers still show substantial browser/graphics retention and do not establish a fixed RAM ceiling. The native viewport correction is a separate behavior change from the paired renderer comparisons; all individual reports retain their exact fingerprints.


## 0.5.15 bounded graphics work and cached-audio streaming, 2026-10-27

A non-forcing detailed memory trace of the0.5.14 native baseline showed renderer cc/image_memory around422.5MiB and GPU Skia resources around311.2MiB at the last rapid-turn checkpoint. These names include parent/child and shared-resource attribution: never sum them into an app total. After60 seconds, tracked Skia resources fell to13.8MiB and image data approached live-canvas storage, while process-private memory remained much higher. The trace distinguishes live resources from retained allocator/driver memory; it does not identify every private allocation. Tracing uses an8MiB trace budget and deterministic:false (no forced GC).

The candidate avoids painting all four invisible neighbors on every turn, while explicitly flushing complete source artwork for actual folds. It reuses a bounded detached fold tree between queued turns, and original exact backing dimensions. The transparent-capacity prototype initially passed the existing single-line check, but high-density outline comparisons exposed altered sampling. An exploratory all-line reference also failed on the unchanged baseline, including after recording actual paint geometry. It is retained only in ignored investigation outputs, not used as a pass/fail gate. Existing strict original-outline/crop references and exact fold-copy checks remain unchanged. Padding was rejected entirely. The initial candidate table below includes that subsequently rejected prototype; final repeat measurements use exact dimensions. All original glyph placement, resolution and contrast rules are retained.

| Native fixture | Sampled peak private MiB | After8s | After60s | Turn browser task seconds | Turn wall ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Baseline0.5.14, allocation-baseline-trace |1113.3|1024.9|841.7|4.066|8693|
| Candidate, allocation-demand-final-experiment |942.4|838.6|633.1|3.048|8778|

This initial pair is about15% lower at sampled peak,25% lower after60 seconds, and25% less turn task work. Wall time includes animation duration and sampling helpers and is not faster in this pair. Twenty distinct animations remain present; reused overlay nodes are distinguished by their turn sequence. First-turn-after-idle latency and callback cadence are retained in each report rather than inferred from CPU percentages. Reports live under outputs/native-performance-qa; final delivery and repeat measurements must be read with their own fingerprints.

Cached whole recordings now use a lazy, narrowly scoped audio-only service worker over the existing CacheStorage files. A bounded streaming validation pass precedes playback; subsequent range responses preserve exact publisher bytes, backpressure and cancellation. Range seeks may read/discard preceding disk chunks because CacheStorage has no random-access body API. This trades some I/O and worker overhead for avoiding a complete playback Blob and provides no guarantee of lower stationary process-private RAM: in this pair stationary playback measured537.6MiB versus523.4MiB baseline. Follow/navigation benefits primarily come from reduced graphics work, not a proved audio-only RAM saving. The Blob fallback remains available and no code/app-shell cache is introduced.

A 15-second inactive-window LOW policy prototype was excluded. Windows refused reliable foreground activation by the test helper, so its restore behavior could not be verified with actual activation events. Production retains the existing minimized-only policy. The trace and resource experiments do not justify a fixed RAM ceiling, forced foreground paging, disabling GPU acceleration, or claiming a Rust/WebGPU rewrite is automatically better.


Final exact-dimension comparison before the optional static-route follow-up: allocation-baseline-repeat measured1221.3MiB peak/859.5MiB after60s and3.720s turn task work; allocation-0.5.15-final measured1027.4/836.9MiB and3.389s. Thus the matched repeat is about16% lower at sampled peak and9% less turn task work, with only about3% lower60-second private memory. The earlier25% idle reduction belongs to a rejected padding prototype and must not describe delivery. A sixty-turn run on the exact-dimension code peaked1206.6MiB and settled871.3MiB; all sixty animations and the subsequent idle wake passed. This still does not solve retained idle memory or establish a1.5GB ceiling on other machines/workloads.

An optional InstallEvent.addRoutes prototype bypassed worker JavaScript for app-code/data/font paths in the browser fixture. Its native contextual run measured1162.5MiB sampled peak and5.574s turn task work versus1027.4MiB and3.389s on the preceding exact-dimension candidate. The environment is variable and this does not isolate a cause, but there is insufficient evidence to retain that optional change: it was excluded from delivery. Its functional report remains allocation-0.5.15-static-final, including a successful natural-network run. Debugger attachment can influence worker lifetime, another limit of native CDP measurements. Actual stationary audio-only RAM savings remain unestablished.
