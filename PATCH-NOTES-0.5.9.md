# Quran Mushaf Reader — review build 0.5.9

This is a local review build, not a published release. Public v0.5.0 corresponds to internal v0.5.4; PATCH-NOTES-0.5.8.md preserves the cumulative earlier changes. This build adds the following experiment and fixes on top.

- Restore a horizontal bottom player with pause/resume, ±10 seconds, seek, settings and stop. During playback it hides after 3.2 seconds away from the controls. A compact Listening handle remains; hover, click or keyboard focus reveals the full player. Pause, keyboard interaction, seeking and pinning keep it available. Escape collapses it and restores focus.
- Make space below the pages only when needed. Opening/closing uses the existing reader layout animation; the viewport size is not animated every frame. A 36px compact strip keeps the reveal handle clear of the Quran pages. Reduced motion changes layout immediately. The player does not rebuild the recording when it hides or opens.
- Replace the side player's broad page-subtree mutation observer with observation of the player's own size. Invalidate prepared fold geometry when the player changes the reading area, and defer automatic changes during pointer gestures/folds.
- Prevent saved positions from an old reciter/recording being associated with a new selection during cleanup. Choosing a surah or ayah preserves play/pause intent, and paused selections keep the Resume control available.
- In Windows recovery, offer Copy and manual-copy guidance instead of a Save file action blocked by the host's download policy. Browser diagnostic saving remains available. No native security policy is relaxed.
- Extend regression coverage for layout clearance, auto-hide, hover/focus/seek protection, pinning, keyboard focus, reduced motion, source-safe resume, selection changes and native/browser recovery. Quran text, supplied outlines, page data and fonts are unchanged.

Try this layout before committing or releasing it. The source backup is in outputs/backups/bottom-player-2026-10-21. No dependency or framework has been added. Build/QA limitations remain documented in BUILD-VERIFICATION.md.
