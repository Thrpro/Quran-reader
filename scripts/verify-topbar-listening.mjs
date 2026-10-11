import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

const root = resolve('dist'), output = resolve('outputs/topbar-qa');
await mkdir(output, { recursive: true });
const mime = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.json': 'application/json', '.otf': 'font/otf', '.woff2': 'font/woff2' };
const server = createServer(async (req, res) => {
  const path = resolve(root, '.' + new URL(req.url, 'http://localhost').pathname.replace(/^\/$/, '/index.html'));
  if (!path.startsWith(root + sep)) return res.writeHead(403).end();
  try { res.writeHead(200, { 'content-type': mime[extname(path)] || 'application/octet-stream' }).end(await readFile(path)); }
  catch { res.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--autoplay-policy=no-user-gesture-required'] });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    window.__QURAN_RUST_HOST__ = true;
    window.chrome ??= {}; window.chrome.webview = { postMessage() {}, addEventListener() {}, removeEventListener() {} };
    localStorage.setItem('quran-reader-onboarding-v2', 'done');
    localStorage.setItem('quran-mushaf-qul-reader:v1', JSON.stringify({ settingsVersion: 5, languageSetupVersion: 1, hasChosenLanguage: true, page: 604, mode: 'spread', theme: 'dark', metadataLanguage: 'english' }));
  });
  const mp3 = await readFile('tests/fixtures/yasser-112001.mp3');
  await page.route('**/*.mp3', route => route.fulfill({ status: 200, headers: { 'content-type': 'audio/mpeg', 'content-length': String(mp3.length), 'access-control-allow-origin': '*' }, body: mp3 }));
  let releaseBoot;
  const bootBlocked = new Promise(done => releaseBoot = done);
  await page.route('**/assets/index-*.js', async route => { await bootBlocked; await route.continue(); });
  const navigation = page.goto('http://127.0.0.1:' + server.address().port);
  await page.locator('.startupMessage').waitFor();
  assert.equal(await page.locator('#startup-label').textContent(), 'Loading…');
  assert.equal(await page.locator('.startupMessage p').count(), 1);
  await page.waitForFunction(() => { const img = document.querySelector('.startupMessage img'); return img.complete && img.naturalWidth > 0; });
  await page.screenshot({ path: resolve(output, 'startup.png') });
  releaseBoot(); await navigation;
  await page.locator('.readerViewport .ayahRun').first().waitFor();
  assert.equal(await page.locator('.startupMessage').count(), 0);
  const bar = page.locator('.rustTitleBar');
  await bar.getByRole('button', { name: 'Search', exact: true }).click();
  await page.locator('.jumpPanel .searchInput').first().fill('45');
  await page.locator('.jumpPanel .searchInput').first().press('Enter');
  await page.waitForFunction(() => [...document.querySelectorAll('.readerViewport .qulPage')].some(el => el.getAttribute('aria-label')?.includes('45')));
  await bar.getByRole('button', { name: 'Listen', exact: true }).click();
  assert.equal(await page.locator('.jumpPanel').count(), 0);
  await page.getByRole('combobox', { name: 'Surah', exact: true }).selectOption('112');
  await page.locator('.listeningRangeEditor summary').click();
  await page.getByRole('spinbutton', { name: 'From ayah' }).fill('2');
  await page.getByRole('spinbutton', { name: 'To ayah' }).fill('3');
  await page.getByRole('button', { name: 'Use this range' }).click();
  await page.getByRole('checkbox', { name: 'Repeat playback' }).check();
  await page.getByRole('combobox', { name: 'Playback speed' }).selectOption('1.5');
  await page.getByRole('button', { name: 'Play', exact: true }).click();
  await page.waitForFunction(() => { const a = document.querySelector('audio'); return a.currentTime > .1 && Number.isFinite(a.duration); });
  const originalSource = await page.locator('audio').getAttribute('src');
  await page.locator('audio').evaluate(a => a.currentTime = a.duration - .15);
  await page.waitForFunction(original => { const a = document.querySelector('audio'); return a.src !== original && a.currentTime > .1 && !a.paused; }, originalSource);
  assert.equal(await page.locator('audio').evaluate(a => a.playbackRate), 1.5);
  assert.match(await page.locator('.listeningRangeSummary').textContent(), /112:2.*112:3/);
  await page.getByRole('checkbox', { name: 'Repeat playback' }).uncheck();
  await page.waitForFunction(() => Number.isFinite(document.querySelector('audio').duration));
  const repeatSource = await page.locator('audio').getAttribute('src');
  await page.locator('audio').evaluate(a => a.currentTime = a.duration - .15);
  await page.waitForFunction(() => document.querySelector('audio').ended);
  assert.equal(await page.locator('audio').getAttribute('src'), repeatSource);
  await page.screenshot({ path: resolve(output, 'listen.png') });
  await bar.getByRole('button', { name: 'Options', exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('.readingToolsPanel'));
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('combobox', { name: 'Theme', exact: true }).selectOption('light');
  await bar.getByRole('button', { name: 'Search', exact: true }).click();
  assert.equal(await page.locator('.settingsPanel').count(), 0);
  for (const width of [1920, 1280, 720]) {
    await page.setViewportSize({ width, height: 900 });
    const bounds = await bar.locator('button').evaluateAll(buttons => buttons.map(b => b.getBoundingClientRect()).map(r => ({ left: r.left, right: r.right, top: r.top, bottom: r.bottom })));
    assert.ok(bounds.every(r => r.left >= 0 && r.right <= width && r.top >= 0 && r.bottom <= 36), 'Title controls fit without clipping');
    await page.screenshot({ path: resolve(output, `search-${width}.png`) });
  }
  await bar.getByRole('button', { name: 'Options', exact: true }).click();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('button', { name: 'العربية', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('.rustReaderActions').getAnimations().length === 0);
  for (const width of [1920, 1280, 720]) {
    await page.setViewportSize({ width, height: 900 });
    const positions = await bar.evaluate(el => {
      const tools = el.querySelector('.rustReaderActions').getBoundingClientRect();
      const windows = el.querySelector('.rustWindowActions').getBoundingClientRect();
      return { left: tools.left, right: tools.right, windows: windows.left };
    });
    assert.ok(positions.right <= positions.windows && positions.windows - positions.right < 16, 'Arabic tools stay beside the window controls');
    assert.ok((positions.left + positions.right) / 2 > width / 2, 'Arabic tools stay on the right');
  }
  await bar.getByRole('button', { name: 'الخيارات', exact: true }).click();
  const toolsPanel = await page.locator('.floatingControls').boundingBox();
  assert.ok(toolsPanel.x > 300 && toolsPanel.x + toolsPanel.width <= 720, 'Arabic options open beside the right-hand tools');
  await bar.getByRole('button', { name: 'بحث', exact: true }).click();
  await page.keyboard.press('Escape');
  await page.locator('.readerViewport .ayahRun').first().click({ button: 'right' });
  const menu = page.locator('.ayahActionsMenu');
  assert.equal(await menu.locator('.ayahListenChoices').isVisible(), false);
  const more = menu.getByRole('button', { name: 'خيارات استماع أخرى', exact: true });
  await more.click();
  assert.equal(await more.getAttribute('aria-expanded'), 'true');
  assert.equal(await menu.locator('.ayahListenChoices button:visible').count(), 4);
  await more.press('ArrowDown');
  assert.ok(await menu.locator('.ayahListenChoices').evaluate(el => el.contains(document.activeElement)), 'Keyboard navigation enters the nested choices');
  await page.screenshot({ path: resolve(output, 'arabic-actions.png') });
  await page.keyboard.press('Escape');
  assert.equal(await menu.count(), 0);
  const arabicBoot = await browser.newPage();
  await arabicBoot.addInitScript(() => localStorage.setItem('quran-mushaf-qul-reader:v1', JSON.stringify({ metadataLanguage: 'arabic' })));
  await arabicBoot.route('**/assets/index-*.js', route => route.abort());
  await arabicBoot.goto('http://127.0.0.1:' + server.address().port);
  assert.equal(await arabicBoot.locator('#startup-label').textContent(), 'جارٍ تحميل التطبيق…');
  assert.equal(await arabicBoot.locator('.startupMessage p').count(), 1);
  await arabicBoot.screenshot({ path: resolve(output, 'startup-arabic.png') });
  await arabicBoot.close();
  assert.deepEqual(errors, []);
  console.log('Startup fallback, title-bar search/listen/options, dropdown Enter navigation, range repeat completion, speed preservation and desktop layout passed.');
} finally { await browser.close(); await new Promise(done => server.close(done)); }
