import assert from 'node:assert/strict';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { build } from 'esbuild';
import { chromium } from 'playwright';

const chapters = await Promise.all(Array.from({ length: 114 }, (_, i) => readFile(`public-qul/data/study/saadi/s${i + 1}.json`, 'utf8').then(JSON.parse)));
const compiled = await build({ entryPoints: ['src/tafsirPassage.ts'], bundle: true, write: false, format: 'iife', globalName: 'Passages' });
const browser = await chromium.launch({ channel: 'msedge', headless: true });
try {
  const page = await browser.newPage();
  await page.addScriptTag({ content: compiled.outputFiles[0].text });
  const report = await page.evaluate(chapters => {
    const select = Passages.selectTafsirPassage;
    const stats = { individual: 0, shared: 0, unavailable: 0, sourceAyahs: 0, errors: [], missingExamples: [] };
    const text = html => new DOMParser().parseFromString(html, 'text/html').body.textContent;
    const ahqaf = chapters[45].groups[chapters[45].lookup['21']];
    const parts = [21, 22, 23, 24, 25, 26].map(ayah => select(ahqaf, ahqaf.text, ayah, true));
    if (parts.map(p => p.kind).join(',') !== 'individual,individual,individual,shared,shared,individual') stats.errors.push('46:21–26 source boundaries');
    if (parts.slice(0, 3).some((p, i) => p.ayahs.join() !== String(21 + i))) stats.errors.push('Individual Ahqaf sections');
    if (parts[3].ayahs.join() !== '24,25' || parts[3].html !== parts[4].html) stats.errors.push('Joint 24–25 commentary must remain intact');
    if (parts[0].html.includes('{22}') || parts[1].html.includes('{23}') || !parts[0].html.includes('{واذكر}')) stats.errors.push('Numeric references removed, source quotations retained');
    const fatiha3 = select(chapters[0].groups['1'], chapters[0].groups['1'].text, 3, true);
    if (fatiha3.kind !== 'unavailable' || !fatiha3.contextHtml) stats.errors.push('Unmarked 1:3 must not borrow a guessed section');
    const missing = select(null, '', 1, true);
    if (missing.kind !== 'unavailable' || missing.contextHtml) stats.errors.push('Missing source data');
    const bad = { fromAyah: 1, toAyah: 2, ayahKeys: [1, 2], text: '<div><span>{3}</span>Other ayah</div>' };
    if (select(bad, bad.text, 1, true).kind !== 'unavailable') stats.errors.push('Invalid references cannot assign another ayah');
    const plain = { ...bad, text: '<div>Unmarked joint commentary</div>' };
    if (select(plain, plain.text, 1, true).kind !== 'shared') stats.errors.push('Unmarked joint commentary');
    if (select(ahqaf, ahqaf.text, 1, true).html) stats.errors.push('Stale/wrong group');
    chapters.forEach((chapter, i) => Object.entries(chapter.lookup).forEach(([ayah, key]) => {
      const group = chapter.groups[key];
      const passage = select(group, group.text, Number(ayah), true);
      stats.sourceAyahs++;
      stats[passage.kind]++;
      if (passage.kind === 'unavailable' && stats.missingExamples.length < 12) stats.missingExamples.push(`${i + 1}:${ayah}`);
      if (passage.kind !== 'unavailable' && !passage.ayahs.includes(Number(ayah))) stats.errors.push(`Incorrect assignment ${i + 1}:${ayah}`);
      // This operation only omits complete source blocks and numeric headings.
      if (!passage.contextHtml || passage.contextHtml !== group.text) stats.errors.push(`Original context changed ${i + 1}:${ayah}`);
      if (passage.kind !== 'unavailable' && !text(passage.html)?.trim()) stats.errors.push(`Empty commentary ${i + 1}:${ayah}`);
    }));
    return stats;
  }, chapters);
  assert.equal(report.sourceAyahs, 6236);
  assert.deepEqual(report.errors, []);
  await mkdir('outputs/study-qa', { recursive: true });
  await writeFile(resolve('outputs/study-qa/passages.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally { await browser.close(); }
