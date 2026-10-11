export type SearchableAyah = {
  key: string;
  surah: number;
  ayah: number;
  normalizedText: string;
  expandedText?: string;
};

const flexibleArabicLetters = new Set(["\u0627", "\u0648", "\u064a"]);

export function isLooseArabicTokenMatch(first: string, second: string) {
  if (first === second) return true;
  if (Math.min(first.length, second.length) < 4 || Math.abs(first.length - second.length) > 1) {
    return false;
  }

  if (first.length === second.length) {
    let differences = 0;
    for (let index = 0; index < first.length; index += 1) {
      if (first[index] === second[index]) continue;
      differences += 1;
      if (differences > 1 ||
          (!flexibleArabicLetters.has(first[index]) && !flexibleArabicLetters.has(second[index]))) {
        return false;
      }
    }
    return differences === 1;
  }

  const longer = first.length > second.length ? first : second;
  const shorter = first.length > second.length ? second : first;
  let longerIndex = 0;
  let shorterIndex = 0;
  let skippedLetter = "";
  while (longerIndex < longer.length && shorterIndex < shorter.length) {
    if (longer[longerIndex] === shorter[shorterIndex]) {
      longerIndex += 1;
      shorterIndex += 1;
      continue;
    }
    if (skippedLetter) return false;
    skippedLetter = longer[longerIndex];
    longerIndex += 1;
  }
  if (!skippedLetter && longerIndex < longer.length) skippedLetter = longer[longerIndex];
  return flexibleArabicLetters.has(skippedLetter);
}

export function isLoosePhraseMatch(textTokens: string[], queryTokens: string[]) {
  if (queryTokens.length === 0 || queryTokens.join("").length < 4 ||
      queryTokens.length > textTokens.length) return false;

  for (let start = 0; start <= textTokens.length - queryTokens.length; start += 1) {
    if (queryTokens.every((token, index) => isLooseArabicTokenMatch(textTokens[start + index], token))) {
      return true;
    }
  }
  return false;
}

// Search-only tokens never replace the original text used for rendering.
export function createAyahSearcher<T extends SearchableAyah>(entries: readonly T[]) {
  const records = entries.map((entry) => {
    const variants = [...new Set([entry.normalizedText, entry.expandedText ?? entry.normalizedText])];
    return { entry, variants, tokens: variants.map((text) => text.split(" ").filter(Boolean)) };
  });
  const byReference = new Map(entries.map((entry) => [`${entry.surah}:${entry.ayah}`, entry]));

  return (queryVariants: string[], reference: { surah: number; ayah: number } | null, limit = 16): T[] => {
    if (limit <= 0) return [];
    if (reference) {
      const entry = byReference.get(`${reference.surah}:${reference.ayah}`);
      return entry ? [entry] : [];
    }
    const queries = [...new Set(queryVariants)].filter(Boolean);
    if (!queries.length) return [];

    const matches: T[] = [];
    const exactKeys = new Set<string>();
    for (const { entry, variants } of records) {
      if (variants.some((text) => queries.some((query) => text.includes(query)))) {
        matches.push(entry);
        exactKeys.add(entry.key);
        if (matches.length >= limit) return matches;
      }
    }

    const queryTokens = queries.map((query) => query.split(" ").filter(Boolean))
      .filter((tokens) => tokens.join("").length >= 4);
    if (!queryTokens.length) return matches;

    for (const { entry, tokens } of records) {
      if (!exactKeys.has(entry.key) && tokens.some((text) =>
        queryTokens.some((query) => isLoosePhraseMatch(text, query)))) {
        matches.push(entry);
        if (matches.length >= limit) break;
      }
    }
    return matches;
  };
}
