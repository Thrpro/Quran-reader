import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

const root = resolve('dist');
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.otf': 'font/otf', '.woff2': 'font/woff2' };
const server = createServer(async (req, res) => {
  const pathname = new URL(req.url, 'http://localhost').pathname;
  const path = resolve(root, '.' + (pathname === '/' ? '/index.html' : pathname));
  if (!path.startsWith(root + sep)) return res.writeHead(403).end();
  try { const content = await readFile(path); res.writeHead(200, { 'Content-Type': mime[extname(path)] ?? 'application/octet-stream' }).end(content); }
  catch { res.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
let browser;
const checks = [];
try {
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  const context = await browser.newContext({ viewport: { width: 1920, height: 1051 } });
  await context.addInitScript(() => localStorage.setItem('quran-mushaf-qul-reader:v1', JSON.stringify({ settingsVersion: 5, languageSetupVersion: 1, hasChosenLanguage: true, page: 505, mode: 'spread', theme: 'dark', metadataLanguage: 'arabic', contextualText: false })));
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  let releaseMetadata;
  const metadataGate = new Promise(resolve => { releaseMetadata = resolve; });
  await page.route('**/data/qul/processed/surahs.json', async route => { await metadataGate; await route.continue(); });
  const initialPageRequest = page.waitForRequest('**/digital-khatt/pages/p505.json');
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  try { await initialPageRequest; checks.push('Visible page loads alongside delayed metadata'); }
  finally { releaseMetadata(); }
  await page.locator('.readerViewport .ayahRun[data-ayah-key="46:22"]').first().waitFor();
  assert.equal(await page.evaluate(() => performance.getEntriesByType('resource').filter(r => /TafsirPanel.*\.js/.test(r.name)).length), 0);
  checks.push('Study module is not fetched before the first page');
  await page.locator('.readerViewport .ayahRun[data-ayah-key="46:22"]').first().click({ button: 'right' });
  await page.locator('.ayahActionsMenu .ayahPrimaryAction').first().click();
  await page.getByRole('button', { name: 'تفسير السعدي', exact: true }).click();
  await page.locator('.tafsirPanel .tafsirArabicText').waitFor();
  const actual = await page.locator('.tafsirPanel .tafsirArabicText').textContent();
  const source = JSON.parse(await readFile('public-qul/data/study/saadi/s46.json', 'utf8'));
  const expected = await page.evaluate(html => {
    const doc = new DOMParser().parseFromString(html, 'text/html');
    const block = [...doc.body.children].find(element => element.firstElementChild?.textContent.trim() === '{22}');
    block.firstElementChild.remove();
    return block.textContent;
  }, source.groups[source.lookup['22']].text);
  assert.equal(actual, expected);
  assert.doesNotMatch(actual, /\{2[1346]\}/);
  assert.ok(actual.includes('{قَالوا أجئتنا'));
  checks.push('46:22 displays exactly its source section, with quotations intact');
  await page.locator('.studyAyahControls select').selectOption('24');
  await page.getByText('عرض التفسير المشترك', { exact: true }).waitFor();
  const details = page.locator('.tafsirSharedPassage');
  assert.equal(await details.getAttribute('open'), null);
  await details.locator('summary').click();
  assert.equal(await details.getAttribute('open'), '');
  const shared = await details.locator('.tafsirArabicText').textContent();
  assert.doesNotMatch(shared, /\{2[1236]\}/);
  await page.locator('.studyAyahControls select').selectOption('25');
  await page.waitForFunction(() => document.querySelector('.tafsirSharedPassage')?.hasAttribute('open') === false);
  assert.equal(await page.locator('.tafsirSharedPassage .tafsirArabicText').textContent(), shared);
  checks.push('Joint 46:24–25 commentary is labeled, collapsed by default and resets on navigation');
  // Malicious cached source markup still passes through the real panel sanitizer.
  await page.keyboard.press('Escape');
  await page.locator('.tafsirPanel').waitFor({ state: 'detached' });
  await page.route('**/data/study/saadi/s1.json', route => route.fulfill({ contentType: 'application/json', body: JSON.stringify({ lookup: { '1': '1', '3': '1' }, groups: { '1': { fromAyah: 1, toAyah: 3, ayahKeys: [1, 3], text: '<div><span>{1}</span>Safe commentary<img src=x onerror="window.studyXss=1"><script>window.studyXss=1</script></div>' } } }) }));
  await page.keyboard.press('Control+f');
  const query = page.locator('.primaryQuranSearch input');
  await query.fill('1'); await query.press('Enter');
  await page.locator('.readerViewport .ayahRun[data-ayah-key="1:1"]').first().waitFor();
  await page.locator('.readerViewport .ayahRun[data-ayah-key="1:1"]').first().click({ button: 'right' });
  await page.locator('.ayahActionsMenu .ayahPrimaryAction').first().click();
  await page.getByRole('button', { name: 'تفسير السعدي', exact: true }).click();
  await page.getByText('Safe commentary', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.studyXss), undefined);
  assert.equal(await page.locator('.tafsirArabicText script, .tafsirArabicText img').count(), 0);
  await page.locator('.studyAyahControls select').selectOption('3');
  await page.getByText('لا يتوفر تفسير مستقل لهذه الآية في بيانات المصدر الحالي.', { exact: true }).waitFor();
  await page.getByText('عرض السياق الكامل في المصدر', { exact: true }).click();
  checks.push('Missing individual section uses a neutral message and retains source context; sanitizer rejects executable markup');
  await page.route('**/digital-khatt/pages/p505.json', route => route.fulfill({ status: 503, body: 'Unavailable' }));
  await page.reload();
  await page.getByRole('button', { name: 'إعادة المحاولة', exact: true }).waitFor();
  await page.unroute('**/digital-khatt/pages/p505.json');
  await page.getByRole('button', { name: 'إعادة المحاولة', exact: true }).click();
  await page.locator('.readerViewport .ayahRun[data-ayah-key="46:22"]').first().waitFor();
  checks.push('Failed initial source page still offers Retry and recovers');
  assert.deepEqual(errors, []);
  await mkdir('outputs/study-qa', { recursive: true });
  await writeFile('outputs/study-qa/ui.json', JSON.stringify({ checks, errors }, null, 2));
  console.log(JSON.stringify({ checks, errors }));
} finally { await browser?.close(); await new Promise(done => server.close(done)); }
