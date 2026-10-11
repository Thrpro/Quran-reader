import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";

const PAGE_COUNT = 604;
const LINES_PER_PAGE = 15;
const LAYOUT_DB_PATH = "work/digital-khatt/raw/digital-khatt-15-lines.db";
const WORDS_DB_PATH = "work/digital-khatt/raw/digital-khatt-v2.db";
const OUTPUT_DIR = "public-qul/experimental/digital-khatt";
const PAGE_OUTPUT_DIR = `${OUTPUT_DIR}/pages`;
const FONT_PATH = "/fonts/digital-khatt/DigitalKhattV2.otf";
const FONT_FAMILY = "Digital Khatt V2";

function normalizeSearchText(value, expandSmallLetters = false) {
  return value
    .toLocaleLowerCase()
    .normalize("NFKD")
    .replace(/\u0670/g, expandSmallLetters ? "ا" : "")
    .replace(/\u06E5/g, "و")
    .replace(/\u06E6/g, "ي")
    .replace(/\p{M}/gu, "")
    .replace(/[\u0610-\u061A\u06D6-\u06ED\u08D3-\u08FF]/g, "")
    .replace(/[ٱأإآا]/g, "ا")
    .replace(/ؤ/g, "و")
    .replace(/[ئىيیۍېے]/g, "ي")
    .replace(/[ةۀە]/g, "ه")
    .replace(/[كڪ]/g, "ك")
    .replace(/ـ/g, "")
    .replace(/[٠-٩]/g, (digit) => String("٠١٢٣٤٥٦٧٨٩".indexOf(digit)))
    .replace(/[۰-۹]/g, (digit) => String("۰۱۲۳۴۵۶۷۸۹".indexOf(digit)))
    .replace(/[^a-z0-9\u0621-\u063A\u0641-\u064A]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function optionalNumber(value) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }

  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }

  return null;
}

function isVerseMarker(text) {
  return /^\u06DD[\d٠-٩]+$/u.test(String(text ?? "").trim());
}

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new Error(`Could not read ${path}: ${error instanceof Error ? error.message : String(error)}`);
  }
}

const layoutDb = new DatabaseSync(LAYOUT_DB_PATH, { readOnly: true });
const wordsDb = new DatabaseSync(WORDS_DB_PATH, { readOnly: true });

try {
  const info = layoutDb.prepare("SELECT * FROM info LIMIT 1").get();
  if (Number(info?.number_of_pages) !== PAGE_COUNT || Number(info?.lines_per_page) !== LINES_PER_PAGE) {
    throw new Error(
      `Expected a ${PAGE_COUNT}-page, ${LINES_PER_PAGE}-line Digital Khatt layout.`,
    );
  }

  const rawLines = layoutDb
    .prepare("SELECT * FROM pages ORDER BY page_number, line_number")
    .all();
  const rawWords = wordsDb.prepare("SELECT * FROM words ORDER BY id").all();
  const wordsById = new Map(rawWords.map((word) => [Number(word.id), word]));

  if (rawLines.length !== 9046) {
    throw new Error(`Expected 9046 layout lines, found ${rawLines.length}.`);
  }

  if (rawWords.length !== 83668) {
    throw new Error(`Expected 83668 Digital Khatt words, found ${rawWords.length}.`);
  }

  const linesByPage = new Map();
  const wordLocationById = new Map();

  for (const rawLine of rawLines) {
    const pageNumber = Number(rawLine.page_number);
    const lineNumber = Number(rawLine.line_number);
    const firstWordId = optionalNumber(rawLine.first_word_id);
    const lastWordId = optionalNumber(rawLine.last_word_id);
    const words = [];

    if (rawLine.line_type === "ayah") {
      if (firstWordId === null || lastWordId === null) {
        throw new Error(`Missing word range on page ${pageNumber}, line ${lineNumber}.`);
      }

      for (let wordId = firstWordId; wordId <= lastWordId; wordId += 1) {
        const word = wordsById.get(wordId);
        if (!word) {
          throw new Error(`Missing Digital Khatt word ${wordId}.`);
        }
        words.push({
          id: wordId,
          text: String(word.text),
          surah: Number(word.surah),
          ayah: Number(word.ayah),
        });
        wordLocationById.set(wordId, { pageNumber, lineNumber });
      }
    }

    const pageLines = linesByPage.get(pageNumber) ?? [];
    pageLines.push({
      lineNumber,
      type: rawLine.line_type,
      isCentered: Boolean(rawLine.is_centered),
      firstWordId,
      lastWordId,
      surahNumber: optionalNumber(rawLine.surah_number),
      text: words.map((word) => word.text).join(" "),
      words,
    });
    linesByPage.set(pageNumber, pageLines);
  }

  const font = {
    fontPath: FONT_PATH,
    fontFamily: FONT_FAMILY,
    fontFormat: "opentype",
  };
  const pages = Array.from({ length: PAGE_COUNT }, (_, index) => {
    const pageNumber = index + 1;
    const lines = (linesByPage.get(pageNumber) ?? []).sort(
      (first, second) => first.lineNumber - second.lineNumber,
    );

    if (lines.length === 0 || lines.length > LINES_PER_PAGE) {
      throw new Error(`Invalid line count ${lines.length} on page ${pageNumber}.`);
    }

    return { pageNumber, lines, font };
  });

  const pageFonts = pages.map((page) => ({ pageNumber: page.pageNumber, ...font }));
  const surahStartPages = [];
  const seenSurahs = new Set();

  for (const page of pages) {
    for (const line of page.lines) {
      if (line.type !== "surah_name" || !line.surahNumber || seenSurahs.has(line.surahNumber)) {
        continue;
      }
      seenSurahs.add(line.surahNumber);
      surahStartPages.push({ surah: line.surahNumber, pageNumber: page.pageNumber });
    }
  }

  const ayahEntries = new Map();
  for (const rawWord of rawWords) {
    if (isVerseMarker(rawWord.text)) {
      continue;
    }

    const id = Number(rawWord.id);
    const surah = Number(rawWord.surah);
    const ayah = Number(rawWord.ayah);
    const key = `${surah}:${ayah}`;
    const location = wordLocationById.get(id);
    const existing = ayahEntries.get(key);

    if (existing) {
      existing.text = `${existing.text} ${rawWord.text}`;
      existing.firstWordId = Math.min(existing.firstWordId, id);
      existing.lastWordId = Math.max(existing.lastWordId, id);
      if (location && !existing.lineNumbers.includes(location.lineNumber)) {
        existing.lineNumbers.push(location.lineNumber);
      }
      continue;
    }

    ayahEntries.set(key, {
      key,
      surah,
      ayah,
      pageNumber: location?.pageNumber ?? 1,
      lineNumbers: location ? [location.lineNumber] : [],
      firstWordId: id,
      lastWordId: id,
      text: String(rawWord.text),
    });
  }

  const ayahSearchIndex = Array.from(ayahEntries.values()).map((entry) => ({
    ...entry,
    lineNumbers: entry.lineNumbers.sort((first, second) => first - second),
    normalizedText: normalizeSearchText(entry.text),
    expandedText: normalizeSearchText(entry.text, true),
  }));

  rmSync(OUTPUT_DIR, { recursive: true, force: true });
  mkdirSync(PAGE_OUTPUT_DIR, { recursive: true });

  writeFileSync(`${OUTPUT_DIR}/pages.json`, `${JSON.stringify(pages)}\n`);
  writeFileSync(`${OUTPUT_DIR}/page-fonts.json`, `${JSON.stringify(pageFonts)}\n`);
  writeFileSync(`${OUTPUT_DIR}/surah-start-pages.json`, `${JSON.stringify(surahStartPages)}\n`);
  writeFileSync(`${OUTPUT_DIR}/ayah-search-index.json`, `${JSON.stringify(ayahSearchIndex)}\n`);

  for (const page of pages) {
    writeFileSync(`${PAGE_OUTPUT_DIR}/p${page.pageNumber}.json`, `${JSON.stringify(page)}\n`);
  }

  const sourceSurahs = readJson("public-qul/data/qul/processed/surahs.json");
  if (sourceSurahs.length !== surahStartPages.length) {
    throw new Error(
      `Expected ${sourceSurahs.length} Surah starts, found ${surahStartPages.length}.`,
    );
  }

  console.log(
    `Built ${pages.length} Digital Khatt pages, ${rawLines.length} lines, ${rawWords.length} words, and ${ayahSearchIndex.length} ayah search entries.`,
  );
} finally {
  layoutDb.close();
  wordsDb.close();
}
