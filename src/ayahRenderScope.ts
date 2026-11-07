export type AyahWord = { id: number; text: string; surah: number; ayah: number };
export type AyahHighlight = { firstWordId: number; lastWordId: number };
export type AyahScope = {
  readonly keys: ReadonlySet<string>;
  readonly wordIds: readonly number[];
  readonly firstWordId: number;
  readonly lastWordId: number;
};
type WordPage = { lines: readonly { words: readonly AyahWord[] }[] };
const wordScopes = new WeakMap<readonly AyahWord[], AyahScope>();
const pageScopes = new WeakMap<WordPage, AyahScope>();

function makeScope(words: readonly AyahWord[]): AyahScope {
  const wordIds = words.map(word => word.id);
  return { keys: new Set(words.map(word => `${word.surah}:${word.ayah}`)), wordIds,
    firstWordId: wordIds.length ? Math.min(...wordIds) : Infinity,
    lastWordId: wordIds.length ? Math.max(...wordIds) : -Infinity };
}

/** Immutable source arrays/pages own their small lookup; eviction also permits
 * collection of these indexes. Changing the current ayah never rebuilds them. */
export function wordAyahScope(words: readonly AyahWord[]): AyahScope {
  let scope = wordScopes.get(words);
  if (!scope) { scope = makeScope(words); wordScopes.set(words, scope); }
  return scope;
}
export function pageAyahScope(page: WordPage): AyahScope {
  let scope = pageScopes.get(page);
  if (!scope) { scope = makeScope(page.lines.flatMap(line => line.words)); pageScopes.set(page, scope); }
  return scope;
}
export function scopedHighlight<T extends AyahHighlight>(scope: AyahScope, highlight: T | null): T | null {
  if (!highlight || highlight.lastWordId < scope.firstWordId || highlight.firstWordId > scope.lastWordId) return null;
  // Keep exact membership for fragments with noncontiguous source word IDs.
  return scope.wordIds.some(id => id >= highlight.firstWordId && id <= highlight.lastWordId) ? highlight : null;
}
export function scopedRecitation(scope: AyahScope, key: string | null): string | null {
  return key && scope.keys.has(key) ? key : null;
}

export function groupAyahWords<T extends AyahWord>(words: readonly T[]) {
  const groups: { key: string; words: T[] }[] = [];
  for (const word of words) {
    const key = `${word.surah}:${word.ayah}`, previous = groups.at(-1);
    if (previous?.key === key) previous.words.push(word);
    else groups.push({ key, words: [word] });
  }
  return groups.map(group => ({ ...group, scope: wordAyahScope(group.words) }));
}
