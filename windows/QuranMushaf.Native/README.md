# Quran Mushaf Reader — Native Rust preview 0.2.26

Extract the entire folder, then open **QuranMushaf.Native.exe**. Keep its Resources folder beside the executable. This preview is separate from the stable 0.5.4 app and does not replace it.

The reader uses Rust with Windows DirectWrite, Direct2D, native controls and MediaPlayer. It requires neither WebView2 nor .NET. It uses APIs available from Windows 10 version 1703 on x64; use a supported Windows version. Windows N editions may require Microsoft's Media Feature Pack for audio. The app's existing Quran text, Digital Khatt font, line layout, precomputed contextual outlines and study resources are copied unchanged. Their hashes are recorded in build-manifest.json.

Playback highlights the current ayah. Manual navigation suspends automatic following; Go to current ayah returns to the current recitation. Wide windows keep pause/resume and stop controls beside the page; Space toggles pause when focus is outside an input. Listening offers total play counts of once, 2, 3, 5 or 10 times, or until stopped. The choice applies to the next Play command; repeats retain that original range even if the form changes. Repetitions restart validated playback and are not gapless.

Ctrl+F opens Search. Options offers an optional T key, disabled by default and ignored while typing. Fullscreen places navigation in a side column and frees the caption height. Surah browse rows show original English and Arabic names without verse excerpts or page numbers. The reader prepares four source pages ahead and behind at idle, with bounded twelve-entry page/layout/outline caches.

## Improvements in 0.2.26

The Tafsir reader has its own paper viewport, scroll guidance for long text, and a separate playback card. Its collapse arrow moves playback controls into the title bar (or fullscreen rail); reopening the card preserves the pages and recording. Hover over compact actions for the complete localized label. Page-key release clears queued repeats while the visible turn keeps its original 300 ms pacing; idle preparation prioritizes nearby motion pixels within the existing bounds.

The separate, validated Bismillah clip highlights the matching original opening glyph before a chapter’s first ayah. Fatiha remains ordinary ayah 1:1, and Tawbah has no added opening. Full-recording timing metadata currently does not identify Bismillah separately from other introductory audio; those intervals are not guessed or highlighted. Quran resources and audio bytes remain unchanged.

## Improvements in 0.2.25

The player caption and clock have one update owner, fixed loading/ready geometry, and native controls composited into the same frame as the Quran. Transient decoder queries do not blank a previously validated clock. End is explicitly labeled. Fullscreen keeps the floating player when the side margin fits and shows an explicit Exit label (with the full accessible name Exit full screen). Panel margins preserve more of the reading area.

Page-turn time starts after the first frame is drawn so cold preparation cannot consume the animation. Releasing a page key clears repeated work while the visible turn keeps its original 300 ms curve. Distinct queued key presses remain separate from held-key repeats.

The study header uses a quiet Text size chooser and a simple source label when no alternative is installed. Its playback footer is smaller. Manage tafsirs separates English and Arabic categories; installed sources follow the interface language without an extra-language toggle. Publisher data remains subject to the same strict identity, verse mapping, complete-book and storage checks. Quran.com currently lists three English tafsirs, but complete native downloads failed strict publisher verse-reference verification for all three in this release. They are clearly listed as pending and cannot start a download. The included English translation remains available. Original bundled Quran and study resources remain unchanged.

## Improvements in 0.2.24

The compact player fits the spare right margin without changing the Quran's reader bounds. Narrow windows use pause and expand controls in the title bar instead of moving or covering the pages. Closing listening settings interpolates the reading margins through the final frame, avoiding the prior last-frame jump.

Progress uses one concise elapsed / total recording line. Redundant Playing and remaining-time lines are removed. The current-ayah action is smaller and gray, with a separate underline below the glyphs. Stop uses its familiar square icon with its native accessible name retained.

Older profiles receive a one-time upgrade from the old verse-clip default to full-surah recording mode. This enables the complete recording clock for supported reciters (Yasser Al-Dosari and Mishary Alafasy). A deliberate choice of ayah recordings after this upgrade is preserved on later launches. Other reciters still use explicitly labeled ayah recordings; unavailable full-surah durations are never estimated or fabricated. Streaming/cache limits and Quran resources are unchanged.

## Improvements in 0.2.23

Mouse-opened menu rows retain keyboard focus without drawing keyboard-only outlines. Checked settings use small vector checkbox indicators. Selected MIT-licensed Microsoft Fluent icons replace the broken seek arrow and text checkmark; Arabic seek symbols mirror their direction while preserving back/forward actions.

The compact player floats in spare side margin when a centered, full-height book leaves enough room. Smaller windows reserve a side column to protect the Quran text. Its chevron collapses playback into the title bar (the navigation rail in fullscreen), keeping pause and expand available. A small underlined link returns to the recited ayah. Tafsir's scrollable viewport ends above the shared player footer.

Listening settings expose Ayah recordings / Surah recording beside Play. Full-surah timing is currently available for Yasser Al-Dosari and Mishary Alafasy. The full original recording supplies elapsed, total and remaining time; individual clips are explicitly labeled Ayah. Existing explicit recording preferences remain respected. Mode changes apply on the next Play. Offline downloads still save individual verse recordings. Streaming remains bounded and does not download a chapter to learn its duration.

Language-switch animation updates only the three moving title controls, rather than laying out the complete listening form and RichEdit controls every frame. Animation timing measurements are application paint intervals, not a guarantee of displayed frame rate.

## Improvements in 0.2.22

The ayah menu keeps the Tafsir icon and text at the clicked ayah. Icon drawing preserves its parent's transform. Hover invalidation is limited to the affected row. Native fields and title controls are included in the popup presentation buffer, so a later presentation cannot erase them. Cached fields include the actual palette, and focused inputs and scrolling study text are sampled live. Listening choices open immediately inside the same menu; the bookmark action returns when the choices are collapsed.

Prepared audio continues to show Playing or Paused even if a future request fails. A retry message appears only when playback actually stops. Transient connection failures ahead get bounded cancellation-aware background recovery; certificate, provider, integrity and storage failures remain failures. Cache eviction can skip a file still held by the decoder without losing its receipt. Initial loading is labelled Loading recitation.

Ten-second seek controls use circular arrows. Arabic progress runs from right to left, with matching pointer and arrow-key navigation and labelled elapsed/total duration. Surah duration is shown only for validated continuous recordings; verse clips keep an Ayah clock. Download details use smaller UI typography, right-aligned Arabic, a left scrollbar and a separate left-to-right folder path with Unicode bidirectional ordering for mixed Arabic and ASCII text. The Arabic dark theme is labelled رمادي داكن.

These changes add no runtime dependencies. Original Quran resources and existing recordings remain unchanged. Streaming/cache bounds and preview limitations below still apply.

## Improvements in 0.2.21

Online continuous playback now reads the original MP3 through bounded HTTPS byte ranges. Its encoded range cache retains at most 2 MiB; decoder, request and operating-system buffers are additional. It writes no new recording to disk. Existing verified complete recordings are reused read-only. Publisher timings identify the ayah, and the native decoder supplies the complete surah duration without a complete download. Unsupported range responses, changed validators and inconsistent lengths are rejected rather than silently downloading the whole source.

New and missing recording preferences still default to continuous audio for Yasser Al-Dosari and Alafasy. If an older profile explicitly uses verse clips, select Options > Listening > Continuous recording to use the surah timeline. Other voices keep their original verse recordings. Their preparation looks about three minutes ahead, capped at 16 clips and excluding the current long verse. Completed items no longer reserve the entire requested chapter. Recognized inactive clips can be evicted from a 32 MiB temporary cache; modified or unrelated files and offline downloads are preserved. Existing temporary files are trimmed as the verse cache is used.

Offline downloads default to 1 GiB. Options > Listening > Offline download limit offers 1, 4 or 16 GiB, or no limit. Lowering the limit never deletes saved surahs. Downloads remain explicit actions and disk/write errors remain recoverable failures. A preparation error ahead no longer disables seeking in the playable current clip. The shared transport displays hours for long surahs and distinguishes surah and ayah clocks.

Options resolves its complete bounds before hiding underlying controls. Its panel and rows use the same parent presentation and sampled animation pose, with native HWNDs retaining input and accessibility. Tafsir source selection returns focus to reading; Left/Right on a closed source selector navigate ayahs. The reader gains three additional vertical DIPs at the bottom. A subtle 160 ms startup reveal respects Windows' animation preference without sleeping, delaying the first display or changing page geometry.

This remains a preview. The earlier strict short-WAV terminal-event check has passed a fresh run without weakening its assertion, but its intermittent cause has not been established. Do not treat one passing run as complete audio reliability. Earlier short per-ayah rate/tail behavior also remains under investigation. Streaming tests are muted and do not certify audible output, a complete Quran session, gapless playback, poor-network/device endurance or displayed FPS. The memory limit above applies to retained encoded ranges, not the whole app or native decoder. Linux adapters and complete canvas accessibility remain future work.

## Improvements in 0.2.20

Restored and smaller windows now scale the opening-page glyphs, their positions and ayah hit areas together. All 604 pages are checked at three scales; resized natural/contextual captures cover five window/DPI sizes. Original text, font glyphs, positioning data and outlines remain unchanged.

The compact player has a labelled surah timeline for continuous recordings, or an explicitly labelled ayah timeline for clips. Pause/resume uses a clear icon, Stop remains separate, and Show ayah returns to the currently recited ayah. Listening settings has a menu icon. Back/forward 10 seconds and the seek bar stay within the current validated range; clip playback seeks inside the current clip. Seeking preserves pause intent and the existing source.

Listen uses one shared transport, smaller ayah fields and no duplicate pause/stop rows. Its scrollbar appears only when the actual content requires it; scrolling is confined to the panel and native moves are batched. An occluded status control no longer causes repeated layout during playback polling. Closing the editor transitions directly to the compact controller's reserved width, without briefly centering the pages.

Authentic resources now have an independent reusable pack and resource-manifest.json. App builds verify and reference its identity without regenerating source data or decoded fonts. Verify-Quran-Resources.ps1 checks resource hashes without writing files. Keep Resources beside the executable; optional app-only and resource-only ZIPs must be extracted into the same new folder. Original recordings retain their separate recording receipts and stores.

## Improvements in 0.2.19

The compact recitation controller opens the full Listening settings. New profiles prefer the original continuous recording for Yasser Al-Dosari and Mishari Alafasy; other reciters retain the per-ayah adapter. Existing explicit recording preferences are preserved. Continuous playback shows the position within the complete surah even when starting midway, plus time remaining in the selected range. Live speed edits for a continuous recording use a bounded, coalescing native rate worker without seeking or replacing its source. The short per-ayah rate/end boundary remains under investigation.

Options opens from the right in Arabic and the left in English. Appearance has one native theme dropdown and remains open while comparing choices. Parent and native controls share a sampled transition palette; native scrollbar styling commits at the final frame. The fade retains readable text contrast and respects Windows' animation preference.

Manage tafsirs displays the four original optional Arabic sources together, each with its own Download, Remove or active Cancel action. English translation remains explicitly identified. Attribution follows the explanation in its scrollable document, below a faint rule spanning its reading width. No source text is modified.

How to use the reader opens a modeless five-step guide to the actual Search, ayah menu, tafsir, Listen and Options controls. It supports Back, Next, Show again, Close and Escape in both languages. Demonstrations start no audio or downloads, and finishing restores the previous reading context while preserving a page intentionally chosen during the guide.

The migration remains a preview: native decoder edge cases, cold whole-recording startup, full accessibility, broader device testing and Linux adapters still require work. Shared transition sampling is not a guarantee of displayed FPS.

## Improvements in 0.2.18

Contextual animation and settled text share a stable presentation target. Moving native controls skips size processing when their physical dimensions stay unchanged. Search and Options establish their model and animation before their first visible layout, and Listen can reverse an unfinished closing transition from its current position.

Prepared original page pixels are retained and admitted in both directions when the existing budget permits. A portable fixed-size policy caps CPU pixels at 32 MiB and combined raster/upload payloads at 64 MiB, including active faces. Selected panel snapshots remain temporary; original glyphs, exact DPI/physical origins, selection and source geometry still govern reuse. One optional preparation runs per idle dispatch, and background persistence pauses during motion/recent input; normal exit still saves state. Driver allocations and native control snapshots are separate.

Surah and section browsing cache validated navigation metadata without eagerly allocating the full verse-text search index. Menu keyboard entry reaches the first/last visible option, and Escape dismisses an open native dropdown before closing its form. Animated menu hit bounds follow its displayed position.

Search opening also reuses the actual native result-row pixels within the existing control budget. Filtering, selection, scroll, language, DPI and size invalidate those copies. Focused results continue to paint live for keyboard interaction.

Keyboard focus transitions verify the actual resulting native focus. A missing previous focus is accepted when the requested control received it; disabled or unavailable targets still fail.

The package includes actual rendering, interaction and timing receipts. Cold raster/study preparation, complete canvas accessibility, the earlier short-clip pause/rate boundary issue and broader platform/device coverage remain open; these changes do not guarantee displayed FPS or zero latency.

## Earlier improvements in 0.2.17

Options opens as a left-side overlay with section navigation. Reading, theme and listening preferences stay open while adjusted. Listen toggles its range editor; direct ayah listening opens a smaller transport with Pause, Stop and Follow. Reopening the editor preserves the active range. Native combo fields have more vertical padding, and the editable reciter field can dismiss its list without reopening it. Fullscreen has a distinct inward-corner exit icon and centered sidebar identity.

Search prepares its results before opening, cancels stale input debounce and batches native control movement. Listening scope choices expand inline with an active-only animation. Bookmark notices last three seconds and overlay the reader without reserving height or replacing recitation status. Preparation failures name the actual missing ayah and prepared stop boundary.

Opening-page natural text on light paper uses the original font and shaped positions with reduced ClearType contrast enhancement. The contextual outlines and all authentic text, fonts, metadata and recordings stay unchanged. Fresh rendering, interaction, audio and performance receipts define the validation scope; displayed FPS, instant cold playback on every provider, full accessibility and Linux support remain unverified.
## Earlier improvements in 0.2.16

The two opening pages use HarfRust with the original Digital Khatt font for natural Arabic joining and mark placement. DirectWrite still draws the original glyphs, and contextual outlines remain unchanged. This measured first step does not replace the rest of the natural-text renderer yet.

Surah/Juz/Hizb/Rub lists advance one row per wheel notch, accumulating partial wheel movement. Filtering leaves results unselected until a click or keyboard action. Choosing an ayah reveals it in the Mushaf for five seconds. English study opens on the left; source attribution follows the commentary in its scrollable document. Fullscreen uses compact icon controls and a centered reading area. Listening choices expand below the context action; appearance stays open when selecting a theme. The final corner approach now settles geometrically before the landed page replaces it.

## Earlier improvements in 0.2.15

Profile settings and reading progress use a Windows handle-based atomic replacement within the same protected directory. This keeps settings saves, study initialization and listening compatible with the temporary audio store's directory protection. Failed replacement preserves the previous file and cleans up only the exact temporary creation handle. Serialization and size limits stay in portable Rust; the Windows file operation is a separate adapter. An isolated prototype and regression tests exercise this behavior while the audio ancestry is pinned. The staged 0.2.14 archive and failed test evidence remain preserved.

## Temporary storage and search

Cross-surah continuous listening uses a separate temporary store. It borrows existing verified saved recordings read-only and downloads missing original recordings into its own session. A background worker retires unchanged session files after recording leases release. Its 512 MiB budget includes incomplete stages and preserved leftovers; existing saved recordings remain intact. Repeats can download a retired chapter again, and preparation gaps can occur between chapters. Custom single-surah continuous ranges retain the existing persistent storage behavior.

Cleanup checks creation identities, hashes, exclusive ownership and safe directory handles. Unknown, modified or linked files are preserved and count against the budget. Recognized inactive sessions can be reclaimed; a process interruption before ownership is safely recorded can leave preserved files. Storage exhaustion, provider failures and device failures still require recovery; full-Quran playback and all device/OS scenarios are not yet verified.

A newly filtered search starts at its first result. Moving the Search panel updates row height only when necessary and preserves the user's result-list scrolling.

## Earlier improvements in 0.2.13

Page turns and wide-panel movement now reuse bounded, source-rendered pixels during motion. The settled page returns to the original vector renderer. Stationary script keeps its actual physical resolution; the rotating reverse face uses grayscale antialiasing. CPU preparation happens at idle where possible. Quran motion pixels are capped at 64 MiB across CPU/GPU, with a separate 8 MiB cap for native control snapshots; driver backing is additional. Resizing, DPI, language, theme, clock and device-generation changes invalidate stale content. Oversized or unsupported cases retain the vector fallback.

A UI-thread high-resolution scheduler runs only during visible animation, coalesces missed deadlines, and keeps input/paint messages responsive. Older Windows timer support has a fallback. This improves measured application frame cadence, not a guarantee of display FPS; exact comparisons and cold preparation costs are in the patch notes.

Right-click keeps the target ayah highlighted. Its nested listening menu offers this ayah, the end of the physical page, the end of the surah, or the end of the Quran, plus the custom range. Page listening finishes the complete final ayah shown there. Multi-surah repeats repeat the captured plan; subsequent form edits do not retarget it. Chapter transitions can have preparation gaps. Per-ayah playback uses existing bounded rolling storage. In 0.2.13, continuous recordings still used only a persistent 512 MiB store with a conservative reservation, so long multi-surah requests could stop at its quota.

The language chooser now describes menus and settings. The book icon is clearer. Commentary updates are batched to avoid a blank intermediate repaint; its source attribution appears in a smaller footer under a divider. Options groups language and shortcuts together. All original Quran and commentary data remain unchanged.

## Earlier improvements in 0.2.12

First launch asks for English or Arabic; Options can change the language later. Study sources follow that choice: bundled Arabic explanations for Arabic, Saheeh International translation for English. The obsolete other-language setting has been removed; optional books follow the interface language. English translation is not presented as an English tafsir. Existing original source data and optional downloads remain intact.

Tafsir and Listen open on the right. At wide window sizes the spread keeps its full height and shifts sideways; narrower windows still fit pages to the available space. Commentary has comfortable margins, increased line spacing and no duplicated ayah in the fallback study font. Its ayah remains highlighted in the authentic Mushaf.

Arabic search fields and browse tabs follow right-to-left direction. English and Arabic surah names use equal contrast. Options has quieter menu rows with hover/focus feedback. Listen from here and its nested listening arrow share one row with separate targets. Clicking the reciter field opens the list while preserving editable search. Arabic Help uses native right-to-left dialog layout.

A bitmap motion-cache prototype did not establish a reliable benefit and was not retained. A separate QA-only accelerated-context comparison was slower than the current software path. This release retains the original vector renderer and does not claim guaranteed display FPS. Actual capture and programmed paint measurements are documented in the patch notes.

## Available

- Offline reading of all 604 pages; adaptive single-page layout in narrow windows.
- Arrow-key page navigation; with tafsir open, left/right arrows move between ayahs. F11 enters/exits full screen; Escape closes an open panel first, then exits full screen.
- Dropdown search with Enter for page numbers (including Arabic digits), ayah references and text search.
- Al-Mukhtasar, As-Saadi and Saheeh International from the existing bundled study data. Explicit Saadi numeric sections are shown separately; unmarked grouped source passages are retained. Missing explanations use a neutral availability message.
- Native right-click menu with tafsir and nested listening options.
- Seventeen reciter entries from the existing app catalogue; editable reciter selection, ayah ranges, repeat, pause/resume, stop, speed and volume.
- A single native playlist fed only complete validated local audio files. Two background workers prepare up to 60 seconds or 16 future clips, with bounded retries, cancellation and fixed HTTPS providers. Ayah changes follow decoded end-of-stream. Playback rate persists across clips. This is not a claim of proven gapless playback for every recording.
- Whole-surah offline downloads, progress, cancellation, integrity receipts and modeless storage details. Temporary playback storage is limited to 128 MiB; completed offline downloads to 256 MiB. Unknown or modified files are preserved.
- Optional tafsir downloads from Quran.com, grouped by English and Arabic: Ibn Kathir, Al-Tabari, Al-Qurtubi and Al-Wasit. Open Manage tafsirs from the study header to download or remove a book. Only complete verified books are published and available offline, in their matching interface language. A provider error can prevent a book from downloading; no missing explanation is substituted. Storage limits are 64 MiB per book and 200 MiB for the optional library; downloaded content is displayed as inert text.
- Empty chapter placeholders are resolved using the provider's exact ayah response and explicit verse references. Equal commentary alone does not create a shared group; noncontiguous references show their actual ayah numbers.
- Interrupted app-owned tafsir transactions are reclaimed on a background worker only after checking their ownership journal, file set and hashes. Unknown, modified, linked or incomplete journals are preserved; active processes' downloads are left alone.
- Arabic/English controls, theme-aware buttons, short theme transition, four palettes, optional original contextual outlines and adjustable explanation text. The original ornate headers, bismillah glyphs, page metadata and clock are rendered using native Windows font APIs.
- Bookmarks (Ctrl+B), daily page/time goals, optional stopwatch, optional 7 pm reminder while the app is open, and first-launch guidance with a Help option. Reader state is bounded and saved atomically in its own profile.
- A read-only original-text view for screen readers and copying. This complements the Quran canvas; the canvas is not yet a full UI Automation document.
- Scrollable listening controls for short/high-DPI windows. English study opens the bundled translation by default. Study chapters use a six-chapter/4 MiB LRU cache, and unchanged captions do not trigger constant native repainting.

## Earlier improvements in 0.2.10 (historical)

Search opens a compact **Find your place** panel with a Quran query and separate **Surah**, **Page**, and **Juz, Hizb & Rub** browse tabs. An empty Quran query shows navigation metadata in the chosen browse tab, with its own filter. Typing a Quran query replaces the browse controls with matching results. Surah names produce a Go to surah result with the original name, number and page, without showing the first ayah. A number in the Quran query/Page tab means a page; in the Surah filter it means a surah. Section navigation uses the preserved original page, hizb and rub metadata. Enter, result-list keyboard navigation, one-click selection and Close remain available.

While tafsir is open, the studied ayah is highlighted in the authentic Mushaf. That explicit study highlight takes priority over pointer hover; closing study restores hover-only highlighting. Search/menu overlays suppress highlighting. The panel's original-text excerpt remains read-only and selectable; its Windows Arabic fallback limitation is unchanged.

The book icon has inset, complete page outlines. Listen from here uses a circled play icon; additional listening choices are visually indented. Options use distinct button surfaces without repeated settings icons or redundant submenu arrows. The title-bar settings symbol now has two slider lines. Existing nested commands and keyboard navigation are retained.

Title-only animation steps retain the Quran surface and update just the caption area, rather than repainting every page. Native list-item sizing now updates only with the result control and uses actual DPI, removing unrelated list layout work during control movement. Animation timers request 10 ms while active and remain elapsed-time driven; Windows animation preferences are respected. Page folds avoid painting an occluded destination peer while retaining the full interaction map and original vectors. These reduce unnecessary work; measured results are in the separate patch notes and do not guarantee a display frame rate.

## Earlier improvements in 0.2.9 (historical)

Search accepts Enter/Go directly when there is one matching surah or ayah. With several matches, Go moves focus into the results; Down/Up from the query reaches the first/last result, and Enter selects it. Native IME composition can accept its own Enter without triggering navigation. Typed search input is bounded to 512 UTF-16 units, editable reciter input to 128, and ayah fields to three digits. Listening ranges accept Arabic and Eastern Arabic digits as well as Western digits; invalid input gives feedback. These changes affect a separate search/input representation, never the original Quran text.

The selected tafsir text size is now saved between launches, bounded to 14–28 logical pixels. Read-only text controls remove their insertion caret after native selection and formatting changes as well as clicks; focus and exact text selection remain available. Preference reads use the same 64 KiB ceiling as atomic writes. Continuous-audio worker notifications use a bounded queue of four events; replaceable progress updates can coalesce while terminal success/failure remains reliable and cancellation releases a waiting worker.

The Windows build now enables runtime Control Flow Guard checks in addition to existing ASLR and DEP; package verification checks the actual PE flags, guard pointers and function table. Rust rejects implicit unsafe operations inside unsafe functions. All three native download adapters share a TLS 1.2/1.3 configuration with normal OS certificate validation; older Windows versions that reject the newer protocol flag can use TLS 1.2. Security hardening does not make an unsigned preview a signed release or replace keeping Windows patched.

Two twenty-minute muted native continuous-playback runs exercised the production worker, controller and decoder using the unchanged Al-Baqarah recording, including pause/resume, rate changes, ordered verse positions and one source item. The hardened build received its own run. This checks a twenty-minute session, not full-chapter completion or live GUI, device interruption and older OS coverage. See the separate audit for measured results and remaining gaps.

## Improvements in 0.2.8

The Mushaf highlights only the ayah under the pointer. Moving to another ayah replaces the highlight; leaving the canvas, opening a menu, or turning a page clears it. Clicking, tafsir navigation and playing audio retain their verse internally without leaving a sticky canvas highlight. The listening controls continue to show the current recitation. All lines belonging to one hovered ayah can highlight, with one original glyph layer.

Right-click actions now show the original surah name and ayah number, with separate book, play, headphones and bookmark icons and a trailing submenu indicator. Bookmark labels reflect whether the page is already saved. Right-clicking the current study ayah opens its actions consistently. The menu pins its original page/ayah so background playback selection changes cannot retarget an action.

A spread now turns one leaf around its central seam while the neighboring old page remains still. The reverse face uses the correct original destination page, with full vector rendering and no text or bitmap stretching. Narrow windows keep a single-page fold. Face rendering skips pages outside the required leaf region. Drag distance uses the leaf width; settling starts at the current drag position with a distance-dependent duration and a quintic easing curve. Windows animation preferences remain respected. This is still a planar geometric fold, not complete parity with the stable app's curved-paper model.

## Improvements in 0.2.7

Search and side panels are mutually exclusive. Opening Search immediately hides Listen or study, and opening a side panel closes Search. Playback continues independently of the visible panel. Search now has a visible native close button, with keyboard activation and focus returned to the reader; Escape and the title-bar Search toggle remain available.

Dropdown chevrons now use antialiased Direct2D strokes with rounded caps at the actual DPI, replacing integer GDI line segments. The result-list scrollbar follows the dark theme. The volume slider retains the native trackbar's keyboard, mouse and accessible value behavior but uses an antialiased high-contrast track and circular thumb, with a focus ring and no dark tick marks. Control colors are updated by the theme timer even when the parent paint is deferred, and cached decoration graphics are released before COM shutdown.

## Improvements in 0.2.6

Search now narrows results while typing, with a short debounce. An empty search browses the original 114-surah catalogue; Arabic or English surah names filter that list. A valid page number, including Arabic digits, offers one Go to page result without changing the page until confirmed. Ayah references and original text search remain available. The result panel shows up to six visible rows and scrolls for more. This is not yet the reference's separate Surah/Page/Juz browse-tab interface.

Native dropdowns now have a subtle rounded outline and a DPI-aware chevron, replacing the classic white sunken frame and arrow button. Their native selection, editable reciter field, keyboard input and accessible control identities are retained.

Study opens at 18 logical pixels, with a slightly larger selected-ayah excerpt, closer line spacing and 16-pixel horizontal/12-pixel vertical inner margins measured from its actual client area. Text size opens a small chooser for 14 to 28 pixels. The original verse and commentary remain unchanged. English study stays on the left, Arabic on the right; selection and read-only caret suppression remain available.

Books now uses the same read-only RichEdit adapter with inner padding and theme-aware text. Its visible scrollbar is removed; long content remains scrollable with wheel and keyboard input. The study reader retains its dark-aware scrollbar.

## Improvements in 0.2.5

Continuous chapter playback is now available for Yasser Al-Dosari and Mishary Alafasy. Enable **Options → Listening → Continuous recording**, then choose either reciter and your usual ayah range in Listen. This setting applies to the next playback session and is off by default. Other reciters keep the existing per-ayah mode; selecting them with continuous mode enabled gives a clear availability message.

A persistent background worker downloads one complete original chapter recording and its original provider timing data. It checks the official reciter catalogue, exact verse order, MP3 structure, duration and SHA-256 before publishing an atomic bundle. The decoder plays a single recording, and highlighting follows its playback position. Selected ranges use native start/end clipping; a range through the final ayah includes the recording's decoded tail. Pause while preparing, repeat, speed, volume and stop use the existing listening controls. The previous verse highlight clears during a recording's introductory audio.

Large recordings are inspected and hashed through fixed-size buffers instead of loading the complete MP3 into memory. A read lease protects the active verified recording from replacement until playback stops. Preparation cancellation and generation checks prevent old sessions from attaching to a new one. Decoder setup failures leave an explicit error state.

Continuous recordings have a separate 512 MiB storage ceiling and a 256 MiB per-recording limit. Preparation reserves the maximum recording size before downloading. Completed bundles are reused after verification. This first integration preserves failed download stages and unknown or modified files; automatic reclamation and continuous-storage management are still pending. The existing Download surah button and storage panel manage the per-ayah library, not these continuous bundles.

## Improvements in 0.2.4

The spread now uses a joined outer book frame and central seam, closer to stable 0.5.4. One portable geometry calculation drives painting, fold bounds and dragging. It preserves the original page aspect and fits short viewports without forcing the page below the reader. Navigation now uses the same current reader-width decision as rendering, including panel transitions.

Cached word metadata is shared instead of copied on every paint. During a fold, only the destination builds its interaction map; the old and reverse faces retain original vector rendering without rebuilding unused hit regions. Source-colored brushes are reused within each page. Destination hit-testing is checked against original word references throughout both rendering modes. Local measurements showed a small contextual paint-cost reduction, with natural cost roughly unchanged; this pass does not claim a substantial frame-rate improvement.

## Improvements in 0.2.3

Natural full-width ayah lines now fit their measured ink endpoints to the original page line segment through inter-word spacing. Centered source lines retain their centered layout. Glyphs are not stretched, and original text, marks and page assignments remain unchanged. The authentic seven-layer surah header ornaments are restored from the original color font, with colors adapted to the reading theme. Its losslessly unpacked container has a decoded-table integrity receipt.

Contextual curves are prepared once per cached line, including the low-density dark optical weight. Measurements on this Windows system favored software vector rendering for these complex outlines; natural DirectWrite rendering keeps the default accelerated target. The window retains its drawing surface and repaints the reader region during dragging and timed folds, avoiding repeated toolbar composition. Adjacent pages warm through low-priority idle messages. Rapid page requests use a bounded queue; repeated keyboard input is cleared on key release and cancellation. Performance varies by graphics driver and device; the measurement receipts distinguish paint cost from presented frame intervals.

English study and book management open on the left; Arabic opens on the right. The authentic selected ayah appears above the explanation and source attribution. Native RichEdit remains selectable and read-only, hides its typing caret, and uses a dark native scrollbar in dark themes. Modified arrow keys retain native text selection. The original variable Quran font is not accepted by this system's GDI RichEdit adapter, so that selected-ayah excerpt uses Windows Arabic fallback; the main Mushaf remains on the authentic DirectWrite font.

Portable page geometry, spread semantics, destination bounds and queued navigation policy now live in a Rust module without Windows or filesystem imports. Rendering, media, font registration and window controls remain explicit Windows adapters. This is an incremental extraction, not full cross-platform support.

## Improvements in 0.2.2

The custom Windows frame is recalculated before the first paint, including both nonclient sizing paths. Reopening no longer depends on maximizing to remove an extra native caption. The title bar uses DPI-aware DirectWrite text and Direct2D vector controls. Its icon is derived from the original app artwork at nine sizes, with the current physical DPI selected explicitly.

Options and ayah actions use compact, themed modeless popovers with keyboard focus, nested commands, Escape/back navigation and outside-click dismissal. Daily reading and appearance settings remain available. These primary menus do not block normal playback/download polling. Search still drops over the reader; its result height adapts to the number of matches, with larger Arabic excerpts and source references. The study header uses compact navigation, book management and text-size controls, with theme-aware RichEdit paragraph spacing.

Page dragging follows the pointer from either corner. Releasing settles the turn from its current position or returns to the same page. Both visible faces draw original source vectors; text is not replaced by a stretched screenshot or regenerated. Panel opening/closing and language changes use brief eased transitions and respect Windows' animation preference. Native controls are explicitly composed after the parent canvas to prevent disappearing menus or copied-pixel trails.

Playback speed is retained while the Windows decoder opens or resets, and applied through media-open, item-change and playback-state events with polling as a fallback. Pause-before-loading updates the user's pending playback preference instead of pausing an unattached source. A fresh playlist opens its first item automatically, without an extra seek during decoder initialization. Unrelated decoder errors remain visible.

## Repairs in 0.2.1

Search, Listen and Options now occupy the compact native title bar, with Arabic controls on the right. Search drops over the reader without reducing page size. Caption buttons, hover states and a drag region replace the separate toolbar; fullscreen retains a visible exit button. The reader uses more of the available height.

Natural Arabic lines are shaped at the original page dimensions and cached, then rasterized as vectors at the current physical window size. Resizing no longer repeatedly reshapes the same lines or resizes an unchanged Direct2D surface. Control placement is cached to prevent stale controls and unnecessary moves. A short geometric page-fold transition and horizontal page drag are available; this is not yet the complete 0.5.4 fold model.

The explanation control uses native RichEdit, inset margins and paragraph spacing. Its default and existing text colors follow the selected theme after source/ayah changes. Formatting notifications cannot reopen tafsir. Volume has a continuous 0–100% slider.

Playback attaches its playlist only after a complete validated recording is ready. A failure downloading a later item preserves already prepared recordings and stops after their decoded end, with an error instead of silently skipping the unavailable ayah. Windows decoder errors include diagnostic detail.

## Quality fixes in 0.1.1

The reader now keeps the original page proportions, margins and font scale. Natural text fitting measures the actual glyph ink, including pronunciation marks, rather than only text advances. Contextual rendering checks the space between neighboring glyphs without unnecessarily shrinking entire lines. Selection is painted behind a single text layer. Missing or invalid optional elongation data falls back to the original font with a visible explanation.

Panel focus, compact listening controls, fullscreen styles, native theme repainting and window/error cleanup have been corrected. Native control colors now remain correct during synchronous repainting, including the status area in dark mode. Detected playlist errors pause playback and require an explicit restart. Resource reads and references have additional bounds checks. These fixes do not close the feature and reliability gaps below.

## Preview limitations

This preview does not yet replace stable 0.5.4. Continuous recordings currently cover two verified reciters; broader recording coverage, exact parity with the existing page-fold animation and full canvas screen-reader access remain migration work. Separate audio files can retain decoder-transition gaps or pauses already present in the recording; there is no stitched PCM timeline yet. Long sessions, interrupted real networks and Windows 10/N configurations need broader runtime testing. Very long active audio ranges exceeding the 128 MiB cache budget stop with feedback rather than evicting an active clip. The bookmark-selection catalogue still uses a native modal popup. Source group passages without explicit per-ayah markers are retained and labeled rather than arbitrarily divided. The public Quran.com download service may change or be unavailable; existing bundled resources and completed downloads remain usable.

The preview stores preferences separately in `%LOCALAPPDATA%\QuranMushafReader\NativePreview`. It does not open or modify the stable app's settings, Quran assets or recitation cache. Loading resources uses bounded page/layout caches; the search index and original outline table are loaded only when needed.

## Build and verification

From the project root, run `node scripts/build-native-preview.mjs` on Windows x64 with the project-local Rust toolchain. It tests the locked crate, builds without the QA feature, verifies every pre-migration file against the backup, copies authentic resources without regenerating them, and includes dependency licenses. The output is `outputs/native-preview-v0.2.10`. For the nine previously audited non-asset source differences in this workspace, use `--pre-migration-differences outputs/native-reading-2026-10-12/pre-migration-current-differences.json`; this cannot exempt authentic assets or unknown changes.

Building also requires Python and FontTools to unpack the original v4 basmala and color surah header WOFF2 fonts into DirectWrite-compatible TTF containers. Every decoded font table is verified unchanged, apart from the container checksum adjustment; original font files remain untouched. Neither Python nor FontTools is required to run the packaged app.

For development QA, build this crate with `cargo build --release --locked --features qa`. `--verify <output-folder>` checks all page data, actual ink bounds in both rendering modes, original outline paths, high-DPI/highlight PNG renders, optional-rendering fallback, Saadi sections and silent native audio fixtures. `--qa-ui <output-folder>` exercises this app's own panels/navigation, captures only its client area, reports initial paint and memory, and exits automatically. `--qa-control-colors <output-folder>` checks reentrant control messages and actual status-area pixels in all four themes. `--verify-network <output-folder>` runs a separate muted Yasser Al-Dosari test. `--verify-timed-store <output-folder>` exercises the production continuous worker and decoder with whole/partial ranges, pause-before-ready, repeat, cancellation and modified-receipt rejection. `--verify-audio <output-folder>` isolates decoder/controller checks; `--verify-tafsir <output-folder> <source-index>` validates the live provider and one complete optional book. QA arguments and profile overrides are excluded from the distributed build. The root's `scripts/verify-native-quality.mjs` runs these checks in isolated profiles; `scripts/verify-native-package.mjs` checks the production binary and all distributed hashes.

Architecture references: [DirectWrite custom font sets](https://github.com/MicrosoftDocs/win32/blob/docs/desktop-src/DirectWrite/custom-font-sets-win10.md), [DirectWrite layout drawing](https://learn.microsoft.com/en-us/windows/win32/api/dwrite/nf-dwrite-idwritetextlayout-draw), [Windows native media playback](https://learn.microsoft.com/en-us/windows/apps/develop/media-playback/media-playback-with-mediasource).


Selected Microsoft Fluent System Icons (MIT) are compiled as small Direct2D paths. See LICENSE-Fluent-System-Icons.txt and Fluent-Icons-Provenance.json. No SVG parser, browser runtime or icon font is required.
