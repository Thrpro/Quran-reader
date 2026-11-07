export const READER_FONTS = {
  text: '48px "Digital Khatt V2"',
  header: '64px "QUL Surah Header Color"',
  basmala: '32px "QUL Surah Name V4"',
} as const;
export type ReaderFont = keyof typeof READER_FONTS;
export const ALL_READER_FONTS: ReaderFont[] = ['text', 'header', 'basmala'];

export function pageFonts(pages: { lines: { type: string }[] }[]): ReaderFont[] {
  const lines = pages.flatMap(page => page.lines);
  return ALL_READER_FONTS.filter(font => font === 'text' ||
    lines.some(line => line.type === (font === 'header' ? 'surah_name' : 'basmallah')));
}

/** Share pending loads and retain successful faces; failed loads remain retryable. */
export class ReaderFontLoader {
  private loaded = new Set<ReaderFont>();
  private pending = new Map<ReaderFont, Promise<void>>();
  private loadFace: (declaration: string) => Promise<unknown>;
  constructor(loadFace: (declaration: string) => Promise<unknown>) { this.loadFace = loadFace; }
  ready(fonts: ReaderFont[]) { return fonts.every(font => this.loaded.has(font)); }
  load(fonts: ReaderFont[]): Promise<void> {
    return Promise.all(fonts.map(font => {
      if (this.loaded.has(font)) return Promise.resolve();
      let promise = this.pending.get(font);
      if (!promise) {
        promise = Promise.resolve().then(() => this.loadFace(READER_FONTS[font])).then(() => {
          this.loaded.add(font);
        }).finally(() => this.pending.delete(font));
        this.pending.set(font, promise);
      }
      return promise;
    })).then(() => {});
  }
}
