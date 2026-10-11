# DigitalKhatt Build-Time Shaping

Source: https://github.com/DigitalKhatt/digitalkhatt-js
Revision: 78372d7a1e211bbab677cc0304384f278f508657
Files: apps/site-angular/src/app/components/hbmedina/{harfbuzz.ts,just.service.ts}
Assets: apps/site-angular/src/assets/hb.wasm and fonts/hb/madina.otf.

Local adaptations: replace the Angular text-service import with a data-only adapter;
return the HarfBuzz initialization promise so callers cannot race font loading.
These libraries run only during generation, not in the reader.

Code: MIT (LICENSE-MIT). Font: SIL OFL 1.1 (LICENSE-OFL),
https://github.com/DigitalKhatt/madinafont. HarfBuzzJS 0.4.5 retains its package license.

Run `npm run shape:pages` after regenerating QUL pages. Set PLAYWRIGHT_MODULE to
a Playwright module URL when it is not installed locally. The generator checks
every non-centered ayah line for missing glyphs and matching advances, preserves
source text and word IDs, and writes source SHA-256 hashes. Generated outlines
are shared across pages, and only page placements enter the bounded page cache.

The reader now consumes these placements and preserves the source words in
accessible ayah hit areas. Word ink bounds are recorded during generation for
highlighting and tafsir selection. `node --test tests/shaped-pages.test.mjs`
verifies all 604 page hashes and every source word's glyph and hit-area mapping.

The display uses a 38/606 font-size-to-line-width ratio. After justification,
adjacent glyph bounds are checked in the fixed 617 x 919 page geometry (513 px
text width, 24/34 px vertical padding). Small baseline offsets preserve at least
3 natural-page pixels between horizontally overlapping glyph bounds; the first
and last shaped rows stay anchored. This runs only at build time. Update the
geometry and verify-refinements.mjs if the page metrics change.
