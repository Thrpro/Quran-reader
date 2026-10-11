import type { TafsirGroup } from './tafsirDownloads';

export type TafsirPassage = {
  kind: 'individual' | 'shared' | 'unavailable';
  html: string;
  ayahs: number[];
  contextHtml: string;
};

/** Numeric source references only. Braces around quoted words are never removed. */
export function parseTafsirMarker(text: string): number[] | null {
  const normalized = text.trim().replace(/[٠-٩۰-۹]/g, digit => {
    const code = digit.charCodeAt(0);
    return String(code >= 0x6f0 ? code - 0x6f0 : code - 0x660);
  });
  const match = /^\{\s*(\d+(?:\s*[ـ–—-]\s*\d+)*)\s*\}$/.exec(normalized);
  if (!match) return null;
  const numbers = match[1].split(/[ـ–—-]/).map(Number);
  if (numbers.some(n => !Number.isInteger(n) || n < 1 || n > 286)) return null;
  if (numbers.length === 2) {
    if (numbers[1] < numbers[0]) return null;
    return Array.from({ length: numbers[1] - numbers[0] + 1 }, (_, i) => numbers[0] + i);
  }
  // More than two numbers denote a list, not a guessed interval.
  return [...new Set(numbers)];
}

/**
 * Select from already-sanitized HTML using explicit top-level source markers.
 * Never infer boundaries from Quran quotations, missing numbers or prose.
 * Ambiguous/unmarked commentary stays available as original shared context.
 */
export function selectTafsirPassage(group: TafsirGroup | null, safeHtml: string, ayah: number, markedSource: boolean): TafsirPassage {
  const unavailable: TafsirPassage = { kind: 'unavailable', html: '', ayahs: [], contextHtml: '' };
  if (!group || !safeHtml.trim() || !group.ayahKeys.includes(ayah)) return unavailable;
  const fallback: TafsirPassage = { kind: group.ayahKeys.length === 1 ? 'individual' : 'shared', html: safeHtml, ayahs: group.ayahKeys, contextHtml: safeHtml };
  if (!markedSource) return fallback;

  const document = new DOMParser().parseFromString(safeHtml, 'text/html');
  const sections: { ayahs: number[]; blocks: string[] }[] = [];
  let current: typeof sections[number] | null = null;
  for (const node of Array.from(document.body.childNodes)) {
    if (node.nodeType === Node.TEXT_NODE) {
      if (node.textContent?.trim()) return { ...unavailable, contextHtml: safeHtml };
      continue;
    }
    if (!(node instanceof Element)) continue;
    const marker = node.firstElementChild;
    const ayahs = marker?.tagName === 'SPAN' && (node.textContent ?? '').trim().startsWith((marker.textContent ?? '').trim())
      ? parseTafsirMarker(marker.textContent ?? '') : null;
    if (ayahs) {
      // Reject misleading or out-of-group labels rather than misassigning text.
      if (ayahs.some(number => !group.ayahKeys.includes(number))) return { ...unavailable, contextHtml: safeHtml };
      current = { ayahs, blocks: [] };
      sections.push(current);
      marker!.remove();
    }
    if (current) current.blocks.push(node.outerHTML);
    // Surah introductions before the first numeric marker are not an ayah's tafsir.
  }
  if (!sections.length) return fallback;
  const matching = sections.filter(section => section.ayahs.includes(ayah));
  if (!matching.length) return { ...unavailable, contextHtml: safeHtml };
  const covered = [...new Set(matching.flatMap(section => section.ayahs))];
  return {
    kind: covered.length === 1 ? 'individual' : 'shared',
    html: matching.flatMap(section => section.blocks).join(''),
    ayahs: covered,
    contextHtml: safeHtml,
  };
}
