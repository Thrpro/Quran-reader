import firstPages from "./data/ayahFirstPages.json" with { type: "json" };
import spanningLastPages from "./data/ayahLastPages.json" with { type: "json" };
// Generated solely from the bundled, validated Digital Khatt word records.
// Lazy-import this module for follow-along/study navigation; full text search
// remains independent and is loaded only when the search panel is used.
export function ayahFirstPage(surah: number, ayah: number): number | null {
  if (!Number.isInteger(surah) || !Number.isInteger(ayah)) return null;
  const row = firstPages[surah - 1];
  return row && ayah >= 1 && ayah <= row.length ? row[ayah - 1] : null;
}

// Sparse last-page overrides preserve every page-spanning ayah without treating
// the next ayah's page as if it necessarily also contained the preceding ayah.
export function ayahPageSpan(surah: number, ayah: number): { first: number; last: number } | null {
  const first = ayahFirstPage(surah, ayah);
  if (!first) return null;
  const lastPages: Record<string, number> = spanningLastPages;
  return { first, last: lastPages[`${surah}:${ayah}`] ?? first };
}
