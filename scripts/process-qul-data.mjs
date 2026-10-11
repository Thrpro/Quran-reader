import { mkdirSync, readFileSync, rmSync, writeFileSync, copyFileSync, existsSync } from "node:fs";
import { join } from "node:path";

const layoutDbPath = "work/qul-raw/layout/mushaf-qatar-layout.db";
const surahsPath = "work/qul-raw/surahs/quran-metadata-surah-name.json";
const wordsPath = "work/qul-raw/words/qpc-hafs-word-by-word.json";
const hizbMetadataPath = "work/qul-raw/metadata/quran-metadata-hizb.json";
const rubMetadataPath = "work/qul-raw/metadata/quran-metadata-rub.json";
const processedDir = "data/qul/processed";
const publicProcessedDir = "public-qul/data/qul/processed";

const JUZ_START_PAGES = [
  1, 22, 42, 62, 82, 102, 121, 142, 162, 182,
  201, 222, 242, 262, 282, 302, 322, 342, 362, 382,
  402, 422, 442, 462, 482, 502, 522, 542, 562, 582,
];

function readVarint(buffer, offset) {
  let value = 0n;

  for (let i = 0; i < 9; i += 1) {
    const byte = buffer[offset + i];

    if (i === 8) {
      value = (value << 8n) | BigInt(byte);
      return { value: Number(value), length: 9 };
    }

    value = (value << 7n) | BigInt(byte & 0x7f);

    if ((byte & 0x80) === 0) {
      return { value: Number(value), length: i + 1 };
    }
  }

  throw new Error(`Invalid SQLite varint at ${offset}`);
}

function readSignedInt(buffer, offset, bytes) {
  let value = 0n;

  for (let i = 0; i < bytes; i += 1) {
    value = (value << 8n) | BigInt(buffer[offset + i]);
  }

  const bits = BigInt(bytes * 8);
  const sign = 1n << (bits - 1n);
  if ((value & sign) !== 0n) {
    value -= 1n << bits;
  }

  return Number(value);
}

function parseRecord(buffer) {
  const header = readVarint(buffer, 0);
  const headerSize = header.value;
  const serialTypes = [];
  let headerOffset = header.length;

  while (headerOffset < headerSize) {
    const serial = readVarint(buffer, headerOffset);
    serialTypes.push(serial.value);
    headerOffset += serial.length;
  }

  let valueOffset = headerSize;

  return serialTypes.map((serialType) => {
    if (serialType === 0) {
      return null;
    }

    if (serialType === 1) {
      const value = readSignedInt(buffer, valueOffset, 1);
      valueOffset += 1;
      return value;
    }

    if (serialType === 2) {
      const value = readSignedInt(buffer, valueOffset, 2);
      valueOffset += 2;
      return value;
    }

    if (serialType === 3) {
      const value = readSignedInt(buffer, valueOffset, 3);
      valueOffset += 3;
      return value;
    }

    if (serialType === 4) {
      const value = readSignedInt(buffer, valueOffset, 4);
      valueOffset += 4;
      return value;
    }

    if (serialType === 5) {
      const value = readSignedInt(buffer, valueOffset, 6);
      valueOffset += 6;
      return value;
    }

    if (serialType === 6) {
      const value = readSignedInt(buffer, valueOffset, 8);
      valueOffset += 8;
      return value;
    }

    if (serialType === 7) {
      const value = buffer.readDoubleBE(valueOffset);
      valueOffset += 8;
      return value;
    }

    if (serialType === 8) {
      return 0;
    }

    if (serialType === 9) {
      return 1;
    }

    const length = Math.floor((serialType - 12) / 2);
    const valueBuffer = buffer.subarray(valueOffset, valueOffset + length);
    valueOffset += length;

    if (serialType % 2 === 1) {
      return valueBuffer.toString("utf8");
    }

    return valueBuffer;
  });
}

class SQLiteReader {
  constructor(path) {
    this.buffer = readFileSync(path);
    this.pageSize = this.buffer.readUInt16BE(16) || 65536;
    this.usableSize = this.pageSize - this.buffer[20];
    this.maxLocalPayload = Math.floor(((this.usableSize - 12) * 64) / 255) - 23;
  }

  pageOffset(pageNumber) {
    return (pageNumber - 1) * this.pageSize;
  }

  readPageHeader(pageNumber) {
    const pageOffset = this.pageOffset(pageNumber);
    const btreeOffset = pageNumber === 1 ? pageOffset + 100 : pageOffset;
    const type = this.buffer[btreeOffset];
    const cellCount = this.buffer.readUInt16BE(btreeOffset + 3);
    const cellPointerOffset = btreeOffset + (type === 0x05 ? 12 : 8);
    return { btreeOffset, type, cellCount, cellPointerOffset };
  }

  readCellPayload(cellOffset) {
    const payloadLength = readVarint(this.buffer, cellOffset);
    const rowid = readVarint(this.buffer, cellOffset + payloadLength.length);
    const payloadOffset = cellOffset + payloadLength.length + rowid.length;

    if (payloadLength.value > this.maxLocalPayload) {
      throw new Error(`SQLite overflow payload is not supported at ${cellOffset}`);
    }

    const payload = this.buffer.subarray(payloadOffset, payloadOffset + payloadLength.value);
    return { rowid: rowid.value, values: parseRecord(payload) };
  }

  readTablePage(pageNumber) {
    const header = this.readPageHeader(pageNumber);
    const rows = [];

    if (header.type === 0x0d) {
      for (let i = 0; i < header.cellCount; i += 1) {
        const cellPointer = this.buffer.readUInt16BE(header.cellPointerOffset + i * 2);
        const cellOffset = this.pageOffset(pageNumber) + cellPointer;
        rows.push(this.readCellPayload(cellOffset));
      }
      return rows;
    }

    if (header.type === 0x05) {
      for (let i = 0; i < header.cellCount; i += 1) {
        const cellPointer = this.buffer.readUInt16BE(header.cellPointerOffset + i * 2);
        const cellOffset = this.pageOffset(pageNumber) + cellPointer;
        const childPage = this.buffer.readUInt32BE(cellOffset);
        rows.push(...this.readTablePage(childPage));
      }

      const rightMostPage = this.buffer.readUInt32BE(header.btreeOffset + 8);
      rows.push(...this.readTablePage(rightMostPage));
      return rows;
    }

    throw new Error(`Unsupported SQLite b-tree page type ${header.type} on page ${pageNumber}`);
  }

  sqliteMaster() {
    return this.readTablePage(1).map((row) => {
      const [type, name, tableName, rootPage, sql] = row.values;
      return { type, name, tableName, rootPage, sql };
    });
  }

  readTable(tableName) {
    const table = this.sqliteMaster().find(
      (entry) => entry.type === "table" && entry.name === tableName,
    );

    if (!table) {
      throw new Error(`Could not find table ${tableName}`);
    }

    const columnMatch = table.sql.match(/\(([\s\S]*)\)/);
    if (!columnMatch) {
      throw new Error(`Could not parse schema for table ${tableName}`);
    }

    const columns = columnMatch[1]
      .split(",")
      .map((part) => part.trim().match(/^"?([A-Za-z0-9_]+)"?/)?.[1])
      .filter(Boolean);

    return this.readTablePage(table.rootPage).map((row) =>
      Object.fromEntries(columns.map((column, index) => [column, row.values[index] ?? null])),
    );
  }
}

function ensureCleanDirectory(path) {
  rmSync(path, { recursive: true, force: true });
  mkdirSync(path, { recursive: true });
}

function writeProcessed(filename, data) {
  const text = JSON.stringify(data);
  writeFileSync(join(processedDir, filename), text, "utf8");
  writeFileSync(join(publicProcessedDir, filename), text, "utf8");
}

ensureCleanDirectory(processedDir);
ensureCleanDirectory(publicProcessedDir);

const sqlite = new SQLiteReader(layoutDbPath);
const rawPageRows = sqlite
  .readTable("pages")
  .sort((a, b) => a.page_number - b.page_number || a.line_number - b.line_number);

const rawWordsByLocation = JSON.parse(readFileSync(wordsPath, "utf8"));
const words = Object.values(rawWordsByLocation)
  .map((word) => ({
    id: Number(word.id),
    key: word.location,
    surah: Number(word.surah),
    ayah: Number(word.ayah),
    word: Number(word.word),
    text: word.text,
  }))
  .sort((a, b) => a.id - b.id);
const wordsById = new Map(words.map((word) => [word.id, word]));

const surahs = Object.values(JSON.parse(readFileSync(surahsPath, "utf8")))
  .map((surah) => ({
    id: Number(surah.id),
    name: surah.name,
    nameSimple: surah.name_simple,
    nameArabic: surah.name_arabic,
    revelationPlace: surah.revelation_place,
    versesCount: Number(surah.verses_count),
    bismillahPre: Boolean(surah.bismillah_pre),
  }))
  .sort((a, b) => a.id - b.id);
const surahsById = new Map(surahs.map((surah) => [surah.id, surah]));
const rawHizbMetadata = Object.values(JSON.parse(readFileSync(hizbMetadataPath, "utf8")));
const rawRubMetadata = Object.values(JSON.parse(readFileSync(rubMetadataPath, "utf8")));

function getWordsForLine(firstWordId, lastWordId) {
  if (!firstWordId || !lastWordId) {
    return [];
  }

  const lineWords = [];
  for (let id = Number(firstWordId); id <= Number(lastWordId); id += 1) {
    const word = wordsById.get(id);
    if (word) {
      lineWords.push(word);
    }
  }
  return lineWords;
}

const pagesMap = new Map();
for (const row of rawPageRows) {
  const pageNumber = Number(row.page_number);
  const lineWords = getWordsForLine(row.first_word_id, row.last_word_id);
  const surah = row.surah_number ? surahsById.get(Number(row.surah_number)) : null;
  const line = {
    lineNumber: Number(row.line_number),
    type: row.line_type,
    isCentered: Boolean(row.is_centered),
    firstWordId: row.first_word_id == null ? null : Number(row.first_word_id),
    lastWordId: row.last_word_id == null ? null : Number(row.last_word_id),
    surahNumber: row.surah_number == null ? null : Number(row.surah_number),
    text:
      row.line_type === "surah_name"
        ? (surah?.nameArabic ?? "")
        : row.line_type === "basmallah"
          ? "﷽"
          : lineWords.map((word) => word.text).join(" "),
    words: lineWords.map((word) => word.id),
  };

  if (!pagesMap.has(pageNumber)) {
    pagesMap.set(pageNumber, []);
  }
  pagesMap.get(pageNumber).push(line);
}

const pages = Array.from(pagesMap.entries()).map(([pageNumber, lines]) => ({
  pageNumber,
  lines,
}));

const verseStarts = new Map();
const versePages = new Map();
for (const page of pages) {
  for (const line of page.lines) {
    for (const wordId of line.words) {
      const word = wordsById.get(wordId);
      if (!word) {
        continue;
      }

      const verseKey = `${word.surah}:${word.ayah}`;
      if (!verseStarts.has(verseKey)) {
        verseStarts.set(verseKey, word.id);
        versePages.set(verseKey, page.pageNumber);
      }
    }
  }
}

function buildSectionIndex(metadata, numberField, outputField) {
  return metadata
    .map((entry) => {
      const firstVerseKey = entry.first_verse_key;
      const [surah, ayah] = firstVerseKey.split(":").map(Number);
      const number = Number(entry[numberField]);
      const pageNumber = versePages.get(firstVerseKey);
      const startWordId = verseStarts.get(firstVerseKey);

      if (!pageNumber || !startWordId) {
        throw new Error(`Could not map ${outputField} ${number} from verse ${firstVerseKey}.`);
      }

      return {
        [outputField]: number,
        pageNumber,
        surah,
        ayah,
        startWordId,
      };
    })
    .sort((a, b) => a[outputField] - b[outputField]);
}

const hizbIndex = buildSectionIndex(rawHizbMetadata, "hizb_number", "hizb");
const rubIndex = buildSectionIndex(rawRubMetadata, "rub_number", "rub").map((entry) => ({
  ...entry,
  hizb: Math.ceil(entry.rub / 4),
  quarter: ((entry.rub - 1) % 4) + 1,
}));

const pageIndex = pages.map((page) => {
  const ayahLine = page.lines.find((line) => line.type === "ayah" && line.words.length > 0);
  const firstWord = ayahLine ? wordsById.get(ayahLine.words[0]) : null;
  const firstSurahLine = page.lines.find((line) => line.surahNumber);
  const juzIndex = JUZ_START_PAGES.findLastIndex((startPage) => startPage <= page.pageNumber);
  const hizb = firstWord
    ? hizbIndex.findLast((entry) => entry.startWordId <= firstWord.id)?.hizb ?? null
    : null;
  const rub = firstWord
    ? rubIndex.findLast((entry) => entry.startWordId <= firstWord.id)?.rub ?? null
    : null;

  return {
    pageNumber: page.pageNumber,
    firstSurah: firstWord?.surah ?? firstSurahLine?.surahNumber ?? null,
    firstAyah: firstWord?.ayah ?? null,
    juz: juzIndex >= 0 ? juzIndex + 1 : null,
    hizb,
    rub,
    lineCount: page.lines.length,
  };
});

writeProcessed("pages.json", pages);
writeProcessed("words.json", words);
writeProcessed("surahs.json", surahs);
writeProcessed("page-index.json", pageIndex);
writeProcessed(
  "juz.json",
  JUZ_START_PAGES.map((pageNumber, index) => ({ juz: index + 1, pageNumber })),
);
writeProcessed(
  "hizb.json",
  hizbIndex.map(({ startWordId: _startWordId, ...entry }) => entry),
);
writeProcessed(
  "rub.json",
  rubIndex.map(({ startWordId: _startWordId, ...entry }) => entry),
);

const fontNote = [
  "# Font Note",
  "",
  "The current app uses the Mushaf Qatar + qpc-hafs Unicode text path.",
  "Do not mix QPC V2 page fonts with this Unicode renderer unless compatibility is verified.",
  "",
  "The downloaded file named `QPC V2 Font.woff2.bz2` is actually a ZIP archive.",
  "It contains 604 page-specific WOFF2 files named `p1.woff2` through `p604.woff2`.",
  "",
  "Those page fonts belong to the experimental KFGQPC V2 layout + QPC V2 glyph path.",
  "They are extracted to `public-qul/fonts/qpc-v2-pages/` for future renderer work,",
  "but they are not used by the current qpc-hafs Unicode renderer.",
  "",
].join("\n");
writeFileSync("data/qul/FONT_NOTE.md", fontNote, "utf8");

if (existsSync("public-qul/fonts/QPC-V2.woff2")) {
  copyFileSync("public-qul/fonts/QPC-V2.woff2", "data/qul/QPC-V2.woff2");
}

console.log(`Processed ${pages.length} pages, ${rawPageRows.length} layout lines, ${words.length} words, ${surahs.length} surahs.`);
