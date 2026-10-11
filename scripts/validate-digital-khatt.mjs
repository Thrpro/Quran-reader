import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new Error(`Could not read ${path}: ${error instanceof Error ? error.message : String(error)}`);
  }
}

const surahs = readJson("public-qul/data/qul/processed/surahs.json");
const pageIndex = readJson("public-qul/data/qul/processed/page-index.json");
const juz = readJson("public-qul/data/qul/processed/juz.json");
const hizb = readJson("public-qul/data/qul/processed/hizb.json");
const rub = readJson("public-qul/data/qul/processed/rub.json");
const pages = readJson("public-qul/experimental/digital-khatt/pages.json");
const pageFonts = readJson("public-qul/experimental/digital-khatt/page-fonts.json");
const surahStartPages = readJson(
  "public-qul/experimental/digital-khatt/surah-start-pages.json",
);
const ayahSearchIndex = readJson(
  "public-qul/experimental/digital-khatt/ayah-search-index.json",
);
const pageDir = "public-qul/experimental/digital-khatt/pages";
const fontPath = "public-qul/fonts/digital-khatt/DigitalKhattV2.otf";
const layoutDbPath = "work/digital-khatt/raw/digital-khatt-15-lines.db";
const wordsDbPath = "work/digital-khatt/raw/digital-khatt-v2.db";
const failures = [];
const expectedAyahKeys = new Set();
if (Array.isArray(surahs)) {
  for (const surah of surahs) {
    if (!Number.isInteger(surah.id) || !Number.isInteger(surah.versesCount) || surah.versesCount < 1) {
      failures.push(`Invalid Surah metadata for ${surah?.id ?? "unknown"}.`);
      break;
    }
    for (let ayah = 1; ayah <= surah.versesCount; ayah += 1) {
      expectedAyahKeys.add(`${surah.id}:${ayah}`);
    }
  }
}
const renderedAyahKeys = new Set();

if (!Array.isArray(surahs) || surahs.length !== 114) {
  failures.push(`Expected 114 Surahs, found ${Array.isArray(surahs) ? surahs.length : "invalid data"}.`);
}

if (!Array.isArray(pageIndex) || pageIndex.length !== 604) {
  failures.push(`Expected 604 page metadata entries, found ${Array.isArray(pageIndex) ? pageIndex.length : "invalid data"}.`);
}

if (!Array.isArray(juz) || juz.length !== 30) {
  failures.push(`Expected 30 juz entries, found ${Array.isArray(juz) ? juz.length : "invalid data"}.`);
}

if (!Array.isArray(hizb) || hizb.length !== 60) {
  failures.push(`Expected 60 hizb entries, found ${Array.isArray(hizb) ? hizb.length : "invalid data"}.`);
}

if (!Array.isArray(rub) || rub.length !== 240) {
  failures.push(`Expected 240 rub entries, found ${Array.isArray(rub) ? rub.length : "invalid data"}.`);
}

if (!Array.isArray(pages) || pages.length !== 604) {
  failures.push(`Expected 604 Digital Khatt pages, found ${Array.isArray(pages) ? pages.length : "invalid data"}.`);
} else {
  let previousLastWordId = 0;
  for (const page of pages) {
    if (page.pageNumber < 1 || page.pageNumber > 604 || !Array.isArray(page.lines)) {
      failures.push(`Invalid Digital Khatt page ${page?.pageNumber ?? "unknown"}.`);
      break;
    }

    if (page.lines.length === 0 || page.lines.length > 15) {
      failures.push(`Page ${page.pageNumber} has ${page.lines.length} lines.`);
      break;
    }

    for (const line of page.lines) {
      if (line.type !== "ayah") {
        continue;
      }

      if (!line.text || !Number.isInteger(line.firstWordId) || !Number.isInteger(line.lastWordId)) {
        failures.push(`Invalid ayah line on page ${page.pageNumber}, line ${line.lineNumber}.`);
        break;
      }

      const renderedWordCount = Array.isArray(line.words) ? line.words.length : 0;
      const indexedWordCount = line.lastWordId - line.firstWordId + 1;
      if (renderedWordCount !== indexedWordCount) {
        failures.push(
          `Word mapping mismatch on page ${page.pageNumber}, line ${line.lineNumber}: rendered ${renderedWordCount}, indexed ${indexedWordCount}.`,
        );
        break;
      }

      const wordIdsMatch = line.words.every(
        (word, index) =>
          word.id === line.firstWordId + index &&
          typeof word.text === "string" &&
          word.text.length > 0,
      );
      if (!wordIdsMatch) {
        failures.push(`Invalid word records on page ${page.pageNumber}, line ${line.lineNumber}.`);
        break;
      }

      for (const word of line.words) {
        const key = `${word.surah}:${word.ayah}`;
        if (!expectedAyahKeys.has(key)) {
          failures.push(`Word ${word.id} points to an invalid ayah ${key}.`);
          break;
        }
        renderedAyahKeys.add(key);
      }

      if (line.firstWordId !== previousLastWordId + 1) {
        failures.push(
          `Word sequence breaks at page ${page.pageNumber}, line ${line.lineNumber}: expected ${previousLastWordId + 1}, found ${line.firstWordId}.`,
        );
        break;
      }
      previousLastWordId = line.lastWordId;
    }
  }

  if (previousLastWordId !== 83668) {
    failures.push(`Expected final Digital Khatt word ID 83668, found ${previousLastWordId}.`);
  }
  if (renderedAyahKeys.size !== expectedAyahKeys.size ||
      [...expectedAyahKeys].some((key) => !renderedAyahKeys.has(key))) {
    failures.push(`Rendered Quran coverage is incomplete: expected ${expectedAyahKeys.size} ayahs, found ${renderedAyahKeys.size}.`);
  }
}

if (!Array.isArray(pageFonts) || pageFonts.length !== 604) {
  failures.push(`Expected 604 Digital Khatt font mappings, found ${Array.isArray(pageFonts) ? pageFonts.length : "invalid data"}.`);
}

if (!Array.isArray(surahStartPages) || surahStartPages.length !== 114) {
  failures.push(`Expected 114 Surah starts, found ${Array.isArray(surahStartPages) ? surahStartPages.length : "invalid data"}.`);
}

if (!Array.isArray(ayahSearchIndex) || ayahSearchIndex.length !== 6236) {
  failures.push(`Expected 6236 searchable ayahs, found ${Array.isArray(ayahSearchIndex) ? ayahSearchIndex.length : "invalid data"}.`);
} else {
  const searchKeys = new Set(ayahSearchIndex.map((entry) => entry.key));
  if (searchKeys.size !== expectedAyahKeys.size || [...expectedAyahKeys].some((key) => !searchKeys.has(key))) {
    failures.push(`Search coverage is incomplete: expected ${expectedAyahKeys.size} unique ayahs, found ${searchKeys.size}.`);
  }
}

if (!existsSync(pageDir)) {
  failures.push(`Expected Digital Khatt page directory at ${pageDir}.`);
} else {
  const pageCount = readdirSync(pageDir).filter((name) => /^p\d+\.json$/i.test(name)).length;
  if (pageCount !== 604) {
    failures.push(`Expected 604 per-page JSON files, found ${pageCount}.`);
  }
}

for (const path of [fontPath, layoutDbPath, wordsDbPath]) {
  if (!existsSync(path) || statSync(path).size === 0) {
    failures.push(`Missing Digital Khatt source asset: ${path}.`);
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log(
  `Digital Khatt data OK: ${pages.length} pages, ${pageFonts.length} font mappings, ${surahStartPages.length} Surah starts, ${ayahSearchIndex.length} searchable ayahs, ${hizb.length} hizb, and ${rub.length} rub.`,
);
