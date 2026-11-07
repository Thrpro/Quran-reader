# Quran Mushaf Reader — review build 0.5.15

- Keep neighboring page sources and geometry ready, but rasterize contextual previews only for an actual fold. Visible text keeps its original resolution, outlines, contrast and layout.
- Reuse one detached fold tree between queued turns. Reconcile changed artwork/text, clear surplus content, bound retained buffers to32 MiB and expire the tree after 1.5 seconds. Destruction is idempotent and old callbacks cannot draw into a reused surface.
- Preserve exact canvas backing dimensions at every density. A transparent-capacity experiment was rejected when all-line comparisons detected altered high-density sampling.
- Stream saved whole-surah/follow-along recordings from the existing disk cache through validated HTTP byte ranges. Preserve the original MP3 bytes and timing data; use the established validated Blob fallback when the worker is unavailable. No recording is loaded merely by opening listening settings.
- Bound media response ownership, cancellation and invalid-file recovery. Stop clears sources and cancels reads; existing downloads remain compatible. The worker caches no app code, UI or Quran data.
- Add non-forcing native allocation traces, original-outline/crop references and exact fold-copy checks, and real Edge offline-range/pause/resume checks to release verification.

The source and portable 0.5.14 app are backed up under outputs/backups/allocation-from-0.5.14-2026-10-27. MEMORY-PERFORMANCE.md records measurements, retained browser memory and rejected experiments. The optional broader inactive-window memory policy was excluded because real foreground transitions could not be reliably established by this environment; the existing minimized LOW/visible NORMAL policy is retained. This is a local review build, not a published release.

The optional static-routing prototype passed functional checks but had higher native turn task time in its final run, so it is excluded. Larger renderer changes remain separate experiments.
