# Quran Mushaf Reader — review build 0.5.14

- Hand focus from ayah actions to the study panel, preserving navigation focus instead of leaving the original ayah outlined. Keyboard dismissal of the ayah menu still restores its opener.
- Anchor explicitly expanded side controls near the bottom corner and separate Arabic elapsed/total timeline labels. Side and bottom preferences remain available.
- Reuse mounted visible and neighboring page slots and their line drawing contexts during cached navigation. Source text, original outlines, raster density and fold geometry are retained.
- Use ordinary 2D transforms for contextual folds, retain their original geometry/density, and build fold copies on actual navigation. Natural-text preparation is retained.
- Release invisible preview bitmaps after six idle seconds while retaining sources/geometry. Repaint exact full-density sources before the next fold; visible text is untouched.
- Preserve the last real WebView2 viewport while minimized, avoiding an unnecessary shrink-to-caption and full-size rebuild on restore.

The source and portable 0.5.13 build are backed up under outputs/backups/retention-from-0.5.13-2026-10-27. Native peak/idle measurements and verification limits are recorded in MEMORY-PERFORMANCE.md. This is a local review build.
