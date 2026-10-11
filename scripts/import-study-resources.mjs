import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";

const SOURCE_DIR = "work/tafsir-inspect";
const OUTPUT_DIR = "public-qul/data/study";
const MUKHTASAR_PATH = `${SOURCE_DIR}/arabic-al-mukhtasar-in-interpreting-the-noble-quran.db`;
const SAADI_PATH = `${SOURCE_DIR}/tafsir-as-saadi.json`;
const SAHIH_PATH = `${SOURCE_DIR}/en-sahih-international-with-footnote-tags.json`;

function hashFile(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function parseKey(key) {
  const [surah, ayah] = String(key).split(":").map(Number);
  if (!Number.isInteger(surah) || !Number.isInteger(ayah) || surah < 1 || surah > 114 || ayah < 1) {
    throw new Error(`Invalid ayah key: ${key}`);
  }
  return { surah, ayah };
}

function compactText(text) {
  return String(text ?? "").replace(/\s+/g, " ").replace(/>\s+</g, "><").trim();
}

function emptySurahs() {
  return Array.from({ length: 114 }, () => ({ lookup: {}, groups: {} }));
}

function addGroup(surahs, key, groupKey, ayahKeys, text, fromKey = key, toKey = key) {
  const selected = parseKey(key);
  const group = parseKey(groupKey);
  const from = parseKey(fromKey);
  const to = parseKey(toKey);
  const localGroupKey = String(group.ayah);
  const page = surahs[selected.surah - 1];
  page.lookup[String(selected.ayah)] = localGroupKey;

  if (!page.groups[localGroupKey]) {
    page.groups[localGroupKey] = {
      fromAyah: from.ayah,
      toAyah: to.ayah,
      ayahKeys: ayahKeys.map((ayahKey) => parseKey(ayahKey).ayah),
      text: compactText(text),
    };
  }
}

function importMukhtasar() {
  const db = new DatabaseSync(MUKHTASAR_PATH, { readOnly: true });
  try {
    const rows = db.prepare("SELECT ayah_key, group_ayah_key, from_ayah, to_ayah, ayah_keys, text FROM tafsir").all();
    const mainRows = new Map(rows.filter((row) => row.text).map((row) => [String(row.ayah_key), row]));
    const surahs = emptySurahs();

    for (const row of rows) {
      const key = String(row.ayah_key);
      const groupKey = String(row.group_ayah_key || row.ayah_key);
      const main = row.text ? row : mainRows.get(groupKey);
      if (!main?.text) throw new Error(`Missing Al-Mukhtasar group text for ${key}`);
      const ayahKeys = String(main.ayah_keys || main.ayah_key).split(",").map((value) => value.trim()).filter(Boolean);
      addGroup(surahs, key, groupKey, ayahKeys, main.text,
        String(main.from_ayah || main.ayah_key), String(main.to_ayah || main.ayah_key));
    }
    return surahs;
  } finally {
    db.close();
  }
}

function importSaadi() {
  const source = JSON.parse(readFileSync(SAADI_PATH, "utf8"));
  const surahs = emptySurahs();
  for (const [key, value] of Object.entries(source)) {
    const groupKey = typeof value === "string" ? value : key;
    const main = source[groupKey];
    if (!main || typeof main === "string" || !main.text) {
      throw new Error(`Missing As-Saadi group text for ${key}`);
    }
    const ayahKeys = Array.isArray(main.ayah_keys) ? main.ayah_keys : [groupKey];
    addGroup(surahs, key, groupKey, ayahKeys, main.text, ayahKeys[0], ayahKeys.at(-1));
  }
  return surahs;
}

function importSahih() {
  const source = JSON.parse(readFileSync(SAHIH_PATH, "utf8"));
  const surahs = Array.from({ length: 114 }, () => ({}));
  for (const [key, value] of Object.entries(source)) {
    const { surah, ayah } = parseKey(key);
    surahs[surah - 1][String(ayah)] = {
      text: compactText(value.t ?? value),
      footnotes: value.f ?? {},
    };
  }
  return surahs;
}

function writeSurahs(directory, surahs) {
  mkdirSync(`${OUTPUT_DIR}/${directory}`, { recursive: true });
  surahs.forEach((data, index) => {
    writeFileSync(`${OUTPUT_DIR}/${directory}/s${index + 1}.json`, JSON.stringify(data));
  });
}

rmSync(OUTPUT_DIR, { recursive: true, force: true });
mkdirSync(OUTPUT_DIR, { recursive: true });
writeSurahs("mukhtasar", importMukhtasar());
writeSurahs("saadi", importSaadi());
writeSurahs("sahih", importSahih());

writeFileSync(`${OUTPUT_DIR}/manifest.json`, JSON.stringify({
  schemaVersion: 1,
  sources: {
    mukhtasar: {
      name: "Al-Mukhtasar fi Tafsir al-Quran al-Karim",
      language: "ar",
      sha256: hashFile(MUKHTASAR_PATH),
      source: "Quranic Universal Library",
    },
    saadi: {
      name: "Tafsir As-Saadi",
      language: "ar",
      sha256: hashFile(SAADI_PATH),
      source: "Quranic Universal Library",
    },
    sahih: {
      name: "Saheeh International with footnotes",
      language: "en",
      sha256: hashFile(SAHIH_PATH),
      source: "Quranic Universal Library",
    },
  },
}));

console.log("Imported two per-surah tafsirs and one translation.");
