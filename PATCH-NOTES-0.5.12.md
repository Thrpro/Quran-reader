# Quran Mushaf Reader — review build 0.5.12

This local review follows 0.5.11. Public v0.5.0 corresponds to internal v0.5.4; PATCH-NOTES-0.5.8.md retains the cumulative earlier changes. Existing release tags, downloads and backups are preserved.

- Offer **Bottom** or **Side** in the Arabic/English introduction. The choice uses the same saved setting as Player position. Existing readers can select **Show introduction** in Settings without resetting their profile.
- Restore the compact 180px side card near the lower corner when it fits in an existing page margin. It never reserves a side strip or reduces Quran page dimensions. Tighter windows use playback and full-control buttons in the toolbar; full controls open only on request and dismiss with Escape or an outside click.
- Hide player controls while settings, search, the introduction or the listening pane is open. Preserve the mounted player and native audio, including recording, position, speed and playing/paused state. Closing listening settings restores keyboard focus to a visible control.
- Use compact toolbar playback controls while tafsir is open, for either saved placement. Leave the space between the Mushaf and the commentary clear. Closing tafsir restores the chosen layout.
- Replace fullscreen's permanent 160px side margins and narrow-window footer with a 40px toolbar. Quran pages retain the available width, while full-screen access and compact playback remain outside the reading area.
- Update DOMPurify from 3.4.16 to the upstream 3.4.17 patch after the dependency audit reported sanitizer advisories. The app uses its default string sanitization; this finding does not establish that the app was exploitable. See https://github.com/cure53/DOMPurify/releases/tag/3.4.17 for the upstream fixes.
- Keep the compact auto-hiding bottom player, Arabic right-to-left seeking, high-density Quran rendering, stable hidden page previews and previous listening safeguards. The side layout observes only its viewport and scale surface; it does not watch glyph mutations or measure every playback tick.

Regression checks exercise fresh onboarding profiles, saved/replayed choices, both interface languages, narrow and desktop windows, fullscreen, tafsir, settings/search suspension, explicit popovers, keyboard focus and uninterrupted playback. They use isolated Edge profiles and authentic audio fixtures; native-window and live-provider review remain separate. This layout update does not establish a new WebView2 RAM or hardware CPU/GPU improvement.

Authentic Quran text, fonts, outlines, page resources and recitation bytes are unchanged. No new dependency or framework is added. The pre-change source and compiled v0.5.11 ZIP are preserved under outputs/backups/player-layout-from-0.5.11-2026-10-23. Changes remain local; no commit, push or public release is created.
