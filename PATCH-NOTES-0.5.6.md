# Quran Mushaf Reader 0.5.6

- Prefer a single, publisher-matched surah recording for Yasser Al-Dosari and Mishary Alafasy, with ayah highlighting. Other reciters retain ayah playback. The recording preference is saved.
- A dedicated playback strip reserves its measured height below the Quran pages. It includes a surah timeline, elapsed/total time, pause/resume, back/forward 10 seconds, listening settings and stop.
- Avoid redundant page renders from equivalent metadata, remove invisible interaction nodes from shaped fold clones, and release warm fold surfaces after 2.5 seconds of inactivity.
- Keep the contextual ink correction from 0.5.5. Quran source text, outlines and shaping positions are unchanged.

Validation: 40 unit tests, 5 native tests, asset integrity, continuous-recording/range playback, speed/pause/seek, responsive player clearance, a simulated rolling timeline over 40 minutes, reader navigation and contextual-ink checks passed in the development build. The browser fixture tests do not establish live-provider uptime or an OS-level GPU/RAM improvement.
