import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { createRequire } from 'node:module';
import { chromium } from 'playwright';
const require = createRequire(import.meta.url);
const { PNG } = require('../node_modules/playwright-core/lib/utilsBundle.js');
const root = resolve('dist'), output = resolve('outputs/ink-qa');
await mkdir(output, { recursive: true });
const mime = { '.html':'text/html', '.js':'application/javascript', '.css':'text/css', '.json':'application/json', '.otf':'font/otf', '.woff2':'font/woff2' };
const server = createServer(async (req, res) => {
  const path = resolve(root, '.' + new URL(req.url, 'http://localhost').pathname.replace(/^\/$/, '/index.html'));
  if (!path.startsWith(root + sep)) return res.writeHead(403).end();
  try { const bytes = await readFile(path); res.writeHead(200, { 'content-type':mime[extname(path)] || 'application/octet-stream' }).end(bytes); }
  catch { res.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const browser = await chromium.launch({ channel:'msedge', headless:true });
function pixels(bytes) {
  const png = PNG.sync.read(bytes); let bright = 0;
  for (let i = 0; i < png.data.length; i += 4) if (png.data[i] >= 225) bright++;
  return bright;
}
try {
  const reports = [];
  for (const dpr of [1, 1.25, 2]) {
    const page = await browser.newPage({ viewport:{ width:1920, height:1051 }, deviceScaleFactor:dpr });
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      localStorage.setItem('quran-reader-onboarding-v2', 'done');
      localStorage.setItem('quran-mushaf-qul-reader:v1', JSON.stringify({ settingsVersion:5, languageSetupVersion:1,
        hasChosenLanguage:true, page:303, contextualText:true, mode:'spread', theme:'dark', metadataLanguage:'english' }));
    });
    await page.goto('http://127.0.0.1:' + server.address().port);
    const ink = page.locator('.pageSlot > .qulPage[data-page-number=\x22303\x22] .shapedLineInk').first();
    await ink.waitFor(); await page.waitForFunction(() => document.querySelectorAll('.pageSlot > .qulPage .shapedLineInk').length === 30 && document.fonts.status === 'loaded');
    const optimized = await ink.screenshot({ path:resolve(output, `contextual-dark-${dpr}.png`) });
    const styling = await ink.evaluate(el => ({ mask:getComputedStyle(el).maskImage, color:getComputedStyle(el).backgroundColor }));
    assert.equal(styling.color, 'rgb(251, 251, 251)');
    const normalize = value => value.replace(/^url\([\x22\x27]?/, '').replace(/[\x22\x27]?\)$/, '').trim();
    assert.equal(styling.mask.includes('stroke-width'), dpr < 1.5, 'Expected theme and density ink selection');
    const markup = decodeURIComponent(normalize(styling.mask).slice('data:image/svg+xml,'.length));
    const normal = `url("data:image/svg+xml,${encodeURIComponent(markup.replace(/stroke="white" stroke-width="[^"]*" stroke-linejoin="round"/, ''))}")`;
    await ink.evaluate((el, normal) => el.style.maskImage = normal, normal);
    const unweighted = await ink.screenshot({ path:resolve(output, `contextual-unweighted-${dpr}.png`) });
    const brightBefore = pixels(unweighted), brightAfter = pixels(optimized);
    if (dpr < 1.5) assert.ok(brightAfter > brightBefore, 'Dark optical weight must improve solid ink coverage');
    else assert.equal(brightAfter, brightBefore, 'High-density rendering must retain the original outlines');
    await ink.evaluate((el, mask) => el.style.maskImage = mask, styling.mask);
    await page.locator('.readerSettingsControl').evaluate(button => button.click());
    for (const theme of ['light', 'sepia', 'black', 'navy']) {
      await page.getByLabel('Theme', {exact:true}).selectOption(theme);
      await page.waitForFunction(theme => document.querySelector('.appShell').classList.contains('theme-' + theme), theme);
      const expectedWeight = !['light','sepia'].includes(theme) && dpr < 1.5;
      await page.waitForFunction(({expectedWeight}) => getComputedStyle(document.querySelector('.pageSlot > .qulPage[data-page-number="303"] .shapedLineInk')).maskImage.includes('stroke-width') === expectedWeight, {expectedWeight});
    }
    const sourceWords = await page.locator('.pageSlot > .qulPage .shapedAyahRun .ayahWord').allTextContents();
    assert.ok(sourceWords.length > 20);
    assert.deepEqual(errors, []);
    reports.push({ dpr, brightBefore, brightAfter, preservedHighDensity:dpr >= 1.5 });
    await page.close();
  }
  await writeFile(resolve(output, 'verification.json'), JSON.stringify(reports, null, 2));
  console.log('Verified dark-theme optical weight, unchanged light/sepia/high-density rendering and source words.');
  console.log(JSON.stringify(reports));
} finally { await browser.close(); server.close(); }






