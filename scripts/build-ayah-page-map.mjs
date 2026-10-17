import { readFileSync, writeFileSync } from "node:fs";
const read = path => JSON.parse(readFileSync(path, "utf8"));
const surahs = read("public-qul/data/qul/processed/surahs.json");
const pages = surahs.map(surah => Array(surah.versesCount).fill(0));
const lastPages = {};
for (let pageNumber = 1; pageNumber <= 604; pageNumber++) {
  const page = read(`public-qul/experimental/digital-khatt/pages/p${pageNumber}.json`);
  if (page.pageNumber !== pageNumber) throw new Error(`Wrong source page ${pageNumber}`);
  for (const line of page.lines) for (const word of line.words) {
    const row = pages[word.surah - 1];
    if (!row || !Number.isInteger(word.ayah) || word.ayah < 1 || word.ayah > row.length) throw new Error(`Invalid source reference ${word.surah}:${word.ayah}`);
    // An ayah can span pages. Retain its FIRST page, including a word/marker
    // continued on the next page; never derive this from page-start metadata.
    row[word.ayah - 1] ||= pageNumber;
    if (row[word.ayah - 1] !== pageNumber) lastPages[`${word.surah}:${word.ayah}`] = pageNumber;
  }
}
if (pages.length !== 114 || pages.flat().length !== 6236 || pages.some(row => row.some(page => page < 1 || page > 604))) throw new Error("Incomplete ayah page map");
for (const [name, data] of [["ayahFirstPages", pages], ["ayahLastPages", lastPages]]) {
  const output = `${JSON.stringify(data)}\n`, path = `src/data/${name}.json`;
  if (process.argv.includes("--check")) {
    if (readFileSync(path, "utf8") !== output) throw new Error(`Stale ${path}; run node scripts/build-ayah-page-map.mjs`);
    console.log(`${name} verified against all 604 source pages (${Buffer.byteLength(output)} bytes).`);
  } else { writeFileSync(path, output); console.log(`Wrote ${path} (${Buffer.byteLength(output)} bytes).`); }
}
