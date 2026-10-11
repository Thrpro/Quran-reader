import { readFileSync } from "node:fs";

const root = "public-qul/data/study";
const counts = { mukhtasar: 0, saadi: 0, sahih: 0 };
const surahs = JSON.parse(readFileSync("public-qul/data/qul/processed/surahs.json", "utf8"));

if (!Array.isArray(surahs) || surahs.length !== 114) {
  throw new Error("The study-data validator requires all 114 Surah records.");
}

for (let surah = 1; surah <= 114; surah += 1) {
  const versesCount = surahs[surah - 1]?.versesCount;
  if (!Number.isInteger(versesCount) || versesCount < 1) {
    throw new Error(`Surah ${surah} has an invalid verse count.`);
  }
  const expectedAyahs = Array.from({ length: versesCount }, (_, index) => String(index + 1));
  for (const source of ["mukhtasar", "saadi"]) {
    const data = JSON.parse(readFileSync(`${root}/${source}/s${surah}.json`, "utf8"));
    if (!data || typeof data !== "object" || !data.lookup || !data.groups) {
      throw new Error(`${source} ${surah} has an invalid document shape.`);
    }
    if (JSON.stringify(Object.keys(data.lookup).sort((a, b) => Number(a) - Number(b))) !== JSON.stringify(expectedAyahs)) {
      throw new Error(`${source} ${surah} does not contain the exact expected ayah keys.`);
    }
    for (const [ayah, groupKey] of Object.entries(data.lookup)) {
      const group = data.groups[groupKey];
      if (!group?.text || !Array.isArray(group.ayahKeys)) throw new Error(`${source} ${surah}:${ayah} has no valid group.`);
      if (!group.ayahKeys.includes(Number(ayah)) || group.ayahKeys.some((key) => !Number.isInteger(key) || key < 1 || key > versesCount)) {
        throw new Error(`${source} ${surah}:${ayah} has an invalid group mapping.`);
      }
      if (/<\s*(script|iframe|object|embed)\b/i.test(group.text)) {
        throw new Error(`${source} ${surah}:${ayah} contains unsupported active HTML.`);
      }
      counts[source] += 1;
    }
  }

  const translation = JSON.parse(readFileSync(`${root}/sahih/s${surah}.json`, "utf8"));
  if (JSON.stringify(Object.keys(translation).sort((a, b) => Number(a) - Number(b))) !== JSON.stringify(expectedAyahs)) {
    throw new Error(`sahih ${surah} does not contain the exact expected ayah keys.`);
  }
  for (const [ayah, entry] of Object.entries(translation)) {
    if (!entry?.text || typeof entry.text !== "string" || !entry.footnotes || typeof entry.footnotes !== "object") {
      throw new Error(`sahih ${surah}:${ayah} has no valid translation.`);
    }
    if (/<\s*(script|iframe|object|embed)\b/i.test(entry.text)) {
      throw new Error(`sahih ${surah}:${ayah} contains unsupported active HTML.`);
    }
    counts.sahih += 1;
  }

}

for (const [source, count] of Object.entries(counts)) {
  if (count !== 6236) throw new Error(`Expected 6236 ${source} entries, found ${count}.`);
}

console.log(`Study data OK: ${counts.mukhtasar} ayahs in each of three sources.`);
