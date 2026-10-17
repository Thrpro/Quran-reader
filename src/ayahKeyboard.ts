// Reading order must follow the reference rather than the spread's DOM order
// (the left page is mounted first). Multiple lines/pages may share one ayah.
export function orderedAyahKeys(keys: Iterable<string>): string[] {
  return [...new Set(keys)].filter(key => /^[1-9]\d{0,2}:[1-9]\d{0,2}$/.test(key)).sort((a, b) => {
    const [surahA, ayahA] = a.split(":").map(Number);
    const [surahB, ayahB] = b.split(":").map(Number);
    return surahA - surahB || ayahA - ayahB;
  });
}
