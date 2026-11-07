import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

// Fresh profiles must offer a real, saved player choice; returning readers may
// revisit the introduction without resetting any of their other preferences.
const root = resolve('dist'), output = resolve('outputs/player-onboarding-qa');
await mkdir(output, { recursive: true });
const mime = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.json': 'application/json', '.otf': 'font/otf', '.woff2': 'font/woff2' };
const server = createServer(async (request, response) => {
  const path = resolve(root, '.' + new URL(request.url, 'http://localhost').pathname.replace(/^\/$/, '/index.html'));
  if (!path.startsWith(root + sep)) return response.writeHead(403).end();
  try { response.writeHead(200, { 'content-type': mime[extname(path)] || 'application/octet-stream' }).end(await readFile(path)); }
  catch { response.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const origin = 'http://127.0.0.1:' + server.address().port;
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const report = { startedAt: new Date().toISOString(), cases: [] }, contexts = [];
const labels = language => language === 'arabic' ? {
  language: 'العربية', next: 'التالي', back: 'السابق', skip: 'تخطي', finish: 'ابدأ القراءة',
  options: 'الخيارات', settings: 'الإعدادات', closeSettings: 'إغلاق الإعدادات',
  position: 'موضع أدوات التلاوة', reopen: 'عرض التعريف بالتطبيق', heading: 'أين تفضّل أدوات التلاوة؟',
} : {
  language: 'English', next: 'Next', back: 'Back', skip: 'Skip', finish: 'Start reading',
  options: 'Options', settings: 'Settings', closeSettings: 'Close settings',
  position: 'Player position', reopen: 'Show introduction', heading: 'Where would you like the player?',
};
const savedPosition = (page, value) => page.waitForFunction(value => JSON.parse(localStorage.getItem('quran-mushaf-qul-reader:v1')).playerPlacement === value, value);
async function openSettings(page, text) {
  const options = page.locator('.rustTitleBar').getByRole('button', { name: text.options, exact: true });
  if (await options.getAttribute('aria-expanded') !== 'true') await options.click();
  await page.getByRole('button', { name: text.settings, exact: true }).click();
  await page.locator('.settingsPanel').waitFor();
}
try {
  for (const language of ['english', 'arabic']) for (const viewport of [{ width: 1280, height: 900 }, { width: 390, height: 844 }]) {
    const context = await browser.newContext({ viewport, reducedMotion: 'reduce' }); contexts.push(context);
    const page = await context.newPage(), text = labels(language), errors = [], blocked = [];
    page.setDefaultTimeout(30000);
    page.on('pageerror', error => errors.push(error.message));
    // This is a fresh storage profile. Only the native title-bar bridge is
    // simulated, so language selection and onboarding are exercised for real.
    await page.addInitScript(() => {
      window.__QURAN_RUST_HOST__ = true;
      window.chrome ??= {}; window.chrome.webview = { postMessage() {}, addEventListener() {}, removeEventListener() {} };
    });
    await context.route('**/*', route => {
      if (route.request().url().startsWith(origin + '/')) return route.continue();
      blocked.push(route.request().url()); return route.abort();
    });
    await page.goto(origin);
    await page.locator('.languageDialog').getByRole('button', { name: text.language, exact: true }).click();
    await page.locator('.readerViewport .ayahRun').first().waitFor();
    const tips = page.locator('.readerTips'); await tips.waitFor();
    assert.equal(await tips.getAttribute('dir'), language === 'arabic' ? 'rtl' : 'ltr');
    assert.equal(await tips.locator('.readerTipsProgress > span').count(), 4);
    const welcome = await tips.locator(':scope > strong').textContent();
    await tips.getByRole('button', { name: text.next, exact: true }).click();
    assert.equal(await tips.locator(':scope > strong').textContent(), text.heading);
    const choices = tips.getByRole('group', { name: text.position, exact: true });
    assert.equal(await choices.getByRole('radio').count(), 2);
    const bottom = choices.locator('input[value="bottom"]'), side = choices.locator('input[value="side"]');
    assert.equal(await bottom.isChecked(), true, 'new readers keep the existing bottom default');
    await side.check(); await savedPosition(page, 'side');
    await tips.getByRole('button', { name: text.back, exact: true }).click();
    assert.equal(await tips.locator(':scope > strong').textContent(), welcome);
    await tips.getByRole('button', { name: text.next, exact: true }).click();
    assert.equal(await side.isChecked(), true, 'Back preserves the selected player position');
    await bottom.focus(); await bottom.press('Space'); await savedPosition(page, 'bottom');
    assert.equal(await bottom.isChecked(), true, 'native radio choices work from the keyboard');
    await side.check(); await savedPosition(page, 'side');
    const bounds = await tips.boundingBox();
    assert.ok(bounds.x >= 0 && bounds.y >= 0 && bounds.x + bounds.width <= viewport.width + 1 && bounds.y + bounds.height <= viewport.height + 1, 'the choice card stays inside the viewport');
    const choiceBounds = await choices.boundingBox();
    assert.ok(choiceBounds.width <= bounds.width, 'the two options fit within the introduction');
    await page.screenshot({ path: resolve(output, `${language}-${viewport.width}.png`) });
    await tips.getByRole('button', { name: text.next, exact: true }).click();
    assert.notEqual(await tips.locator(':scope > strong').textContent(), text.heading);
    await tips.getByRole('button', { name: text.next, exact: true }).click();
    await tips.getByRole('button', { name: text.finish, exact: true }).click();
    assert.equal(await tips.count(), 0);
    assert.equal(await page.evaluate(() => localStorage.getItem('quran-reader-onboarding-v2')), 'done');
    await page.reload(); await page.locator('.readerViewport .ayahRun').first().waitFor();
    assert.equal(await tips.count(), 0, 'completed profiles are not forced through onboarding again');
    assert.equal(await page.locator('.languageDialog').count(), 0);
    await openSettings(page, text);
    assert.equal(await page.getByRole('combobox', { name: text.position, exact: true }).inputValue(), 'side', 'the introduction and Settings share the same saved preference');
    await page.locator('.settingsPanel').getByRole('button', { name: text.reopen, exact: true }).click();
    assert.equal(await page.locator('.settingsPanel').count(), 0);
    await tips.waitFor(); assert.equal(await tips.locator(':scope > strong').textContent(), welcome);
    await tips.getByRole('button', { name: text.next, exact: true }).click();
    assert.equal(await side.isChecked(), true, 'reopening the introduction starts from the current preference');
    await bottom.check(); await savedPosition(page, 'bottom');
    await tips.getByRole('button', { name: text.skip, exact: true }).click();
    assert.equal(await tips.count(), 0);
    await openSettings(page, text);
    assert.equal(await page.getByRole('combobox', { name: text.position, exact: true }).inputValue(), 'bottom', 'Skip preserves a choice already made');
    await page.getByRole('button', { name: text.closeSettings, exact: true }).click();
    await page.reload(); await page.locator('.readerViewport .ayahRun').first().waitFor();
    await savedPosition(page, 'bottom'); assert.equal(await tips.count(), 0);
    assert.deepEqual(errors, [], 'the complete onboarding flow has no runtime errors');
    assert.deepEqual(blocked, [], 'onboarding needs no external services');
    report.cases.push({ language, viewport, choiceBounds, bounds, savedInSettings: true, back: true, keyboard: true, completedAndReloaded: true, reopened: true, skipRetainsChoice: true });
    await context.close();
  }
  await writeFile(resolve(output, 'verification.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ cases: report.cases.length, output }, null, 2));
} catch (error) {
  report.failure = error.stack || String(error);
  await writeFile(resolve(output, 'failure.json'), JSON.stringify(report, null, 2));
  for (const [index, context] of contexts.entries()) for (const page of context.pages()) {
    await page.screenshot({ path: resolve(output, `failure-${index}.png`) }).catch(() => {});
  }
  throw error;
} finally {
  await Promise.allSettled(contexts.map(context => context.close()));
  await browser.close(); await new Promise(done => server.close(done));
}
