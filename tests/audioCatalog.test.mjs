import test from "node:test";
import assert from "node:assert/strict";
import { AYAH_RECITERS, audioUrl, ayahAudioUrl, readReciterCatalog, reciterNameMatches } from "../src/audioCatalog.ts";

test("reciter catalogs retain only complete reciters and recordings", () => {
  const parsed = readReciterCatalog({ reciters: [
    null,
    { id: "wrong", name: "Ignored", moshaf: [] },
    { id: 1, name: "Reader", moshaf: [
      { id: 10, name: "Narration", server: "https://server.mp3quran.net/path", surah_list: "1,2" },
      { id: 11, name: "Incomplete" },
    ] },
  ] });
  assert.deepEqual(parsed, [{
    id: 1,
    name: "Reader",
    moshaf: [{ id: 10, name: "Narration", server: "https://server.mp3quran.net/path", surah_list: "1,2" }],
  }]);
});

test("audio URLs reject malformed and untrusted catalog hosts", () => {
  const base = { id: 1, name: "Narration", surah_list: "1,2" };
  assert.equal(audioUrl({ ...base, server: "not a URL" }, 1), "");
  assert.equal(audioUrl({ ...base, server: "http://server.mp3quran.net" }, 1), "");
  assert.equal(audioUrl({ ...base, server: "https://mp3quran.net.attacker.example" }, 1), "");
  assert.equal(audioUrl({ ...base, server: "https://server.mp3quran.net/path/" }, 2),
    "https://server.mp3quran.net/path/002.mp3");
});

test("ayah reciter catalog exposes all Quran Foundation choices", () => {
  assert.equal(AYAH_RECITERS.length, 17);
  const alafasy = AYAH_RECITERS.find(reciter => reciter.id === 7);
  assert.equal(ayahAudioUrl(alafasy, 2, 255),
    "https://verses.quran.foundation/Alafasy/mp3/002255.mp3");
});

test("reciter search tolerates common Latin spelling differences", () => {
  assert.equal(reciterNameMatches("Yasser Al-Dosari", "Yaser Al Doseri"), true);
  assert.equal(reciterNameMatches("Mishari Rashid Alafasy", "mishary afasy"), true);
  assert.equal(reciterNameMatches("AbdulBaset AbdulSamad", "sudais"), false);
});
