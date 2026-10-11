# Resource compatibility

The current default reader uses Digital Khatt Unicode text/font resources with the project's fixed page layout. It preserves source strings and 15-line geometry. Optional contextual elongation uses the pinned DigitalKhatt HarfBuzz WASM, its parametric madina.otf font and build-time outline generation. The smaller runtime DigitalKhattV2.otf serves the natural Unicode path; it is not a replacement for the build-time parametric font.

The generated manifest hashes both font assets, HarfBuzz, the justification service, the shaping adapter and every source page. Rebuild with npm run shape:pages after changing any of them. Contextual glyphs use uniform scaling; marks and source-word hit areas share that geometry. Natural mode remains the default.

QPC V2 glyph codes require their corresponding page fonts and layout. Never interchange those codes or fonts with Unicode text. Search normalization applies only to search copies, not source Quran text. Automatic browser translation is disabled for Quran page containers.

MP3Quran ayah timings must be paired with the folder supplied by its timing catalog. A matching reciter name alone does not establish recording compatibility. The continuous playback feature uses explicit verified read/folder pairs and validates complete, ordered, non-overlapping verse intervals. Existing EveryAyah and Quran Foundation verse clips continue through their original audio pipeline.

Primary references: [DigitalKhatt](https://github.com/DigitalKhatt), [MP3Quran API](https://www.mp3quran.net/ar/api), [Quran Foundation OAuth quickstart](https://api-docs.quran.com/docs/quickstart/), [Floating UI React](https://floating-ui.com/docs/react).
