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
