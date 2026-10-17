# Recitation source policy — checked 2026-10-17

The main reciter list retains ayah follow-along. For supported choices the app automatically uses one continuous MP3Quran recording and the exact matching timing table. A recording by the same reciter can be a different performance from their ayah clips. Timings must never be transferred between performances by name alone.

The live MP3Quran timing catalog contained 115 reads. Eleven curated main choices have verified read/folder pairs: Yasser Al-Dosari, Nasser Al-Qatami, Fares Abbad, Abdullah Al-Juhany, AbdulBaset Murattal, Sudais, Shatri, Rifai, Husary, Alafasy and Tablawi. Chapter listings cover 114 unique chapters; timing samples checked chapters 2, 9 and 112 and an audio response for 112. Reads 31, 51 and 112 currently have incomplete Tawbah numbering and are deliberately excluded. The app validates every loaded chapter for complete, ordered, non-overlapping ayah intervals; it never shifts numbering or invents missing times.

Other main choices retain the buffered clip engine. Additional surah recordings are available in the More reciters disclosure; an exact ID/folder match to the verified mapping inherits follow-along automatically. Remaining additional recordings are labeled as lacking ayah highlighting.

Offline completion for a timed recording requires both its MP3 and validated timing metadata in the same audio cache. Valid old localStorage tables migrate on use. Removing downloaded audio removes its timing metadata too. Streaming metadata remains best-effort cacheable; download persistence failures are reported.

The Quran Foundation chapter-audio API also documents verse timestamps and optional word segments. Its chapter-reciter IDs differ from ayah-recitation IDs. The current authenticated Content API requires backend-held credentials, so no client secret has been embedded in this desktop app. This build expands the existing public MP3Quran integration instead.

Primary documentation and catalog endpoints:

- [MP3Quran timing API](https://apiquran.mp3quran.net/id/timing-api)
- [Live timing reads](https://www.mp3quran.net/api/v3/ayat_timing/reads)
- [Quran Foundation chapter audio and timestamps](https://api-docs.quran.com/docs/content_apis_versioned/4.0.0/chapter-reciter-audio-file/)
- [Quran Foundation Content API authentication](https://api-docs.quran.com/docs/quickstart/)

Local verification evidence is saved under outputs/audio-provider-qa, outside the source/package payload. Live-provider data and uptime may change; these checks do not certify every chapter in every catalog entry.
