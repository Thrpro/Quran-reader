import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { build } from 'esbuild';

// This standalone fixture exercises the actual boundary without adding any
// crash trigger to the production app or touching a real reader profile.
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright');
const output = resolve('outputs/recovery-qa');
await mkdir(output, { recursive: true });
const compiled = await build({
  stdin: { contents: `import React, {useLayoutEffect, useState} from 'react';
import {createRoot} from 'react-dom/client';
import {ReaderErrorBoundary} from './src/ReaderErrorBoundary';
function Fixture() { const [failed,setFailed]=useState(false); useLayoutEffect(()=>{window.__triggerFailure=()=>setFailed(true)},[]);
if(failed) throw new TypeError('PRIVATE_FIXTURE_SENTINEL /private/profile/token');
return <button id="ready">Reader is ready</button>; }
createRoot(document.getElementById('root')).render(<ReaderErrorBoundary><Fixture/></ReaderErrorBoundary>);`,
    resolveDir: resolve('.'), loader: 'tsx' },
  bundle: true, write: false, outfile: 'recovery.js', jsx: 'automatic', define: { 'process.env.NODE_ENV': '"production"' },
});
const files = new Map(compiled.outputFiles.map(file => [file.path.endsWith('.css') ? '/recovery.css' : '/recovery.js', file.contents]));
const server = createServer((req, res) => {
  const path = new URL(req.url, 'http://localhost').pathname;
  if (files.has(path)) { res.setHeader('Content-Type', path.endsWith('.css') ? 'text/css' : 'text/javascript'); res.end(files.get(path)); return; }
  res.setHeader('Content-Type', 'text/html'); res.end('<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="/recovery.css"></head><body><div id="root"></div><script src="/recovery.js"></script></body></html>');
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const report = [];
let browser;
try {
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  for (const arabic of [false, true]) {
    const context = await browser.newContext({ viewport: { width: 1024, height: 600 }, permissions: ['clipboard-read', 'clipboard-write'], acceptDownloads: true });
    await context.addInitScript(arabic => {
      localStorage.setItem('quran-mushaf-qul-reader:v1', JSON.stringify({ metadataLanguage: arabic ? 'arabic' : 'english', theme: arabic ? 'light' : 'dark', retainedSentinel: 'keep-settings' }));
      localStorage.setItem('fixture-download-index', 'keep-downloads');
      window.__QURAN_RUST_HOST__ = true;
      window.__windowActions = [];
      window.chrome ??= {};
      window.chrome.webview = { postMessage: data => window.__windowActions.push(data) };
    }, arabic);
    const page = await context.newPage();
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.locator('#ready').waitFor();
    const preferences = await page.evaluate(() => localStorage.getItem('quran-mushaf-qul-reader:v1'));
    await page.evaluate(() => window.__triggerFailure());
    const recovery = page.locator('.readerRecovery');
    await recovery.waitFor();
    assert.equal(await recovery.getAttribute('dir'), arabic ? 'rtl' : 'ltr');
    assert.match(await recovery.locator('h1').textContent(), arabic ? /حدث خطأ/ : /ran into a problem/);
    await page.getByRole('button', { name: arabic ? 'المحاولة مجددًا' : 'Try again', exact: true }).click();
    await page.locator('#ready').waitFor();
    await page.evaluate(() => window.__triggerFailure());
    await recovery.waitFor();
    await page.locator('.readerRecoveryDetails summary').click();
    await page.getByRole('button', { name: arabic ? 'نسخ' : 'Copy', exact: true }).click();
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    assert.doesNotMatch(copied, /PRIVATE_FIXTURE_SENTINEL|private\/profile|keep-settings|keep-downloads/);
    assert.equal(JSON.parse(copied).errorType, 'TypeError');
    assert.equal(JSON.parse(copied).retryCount, 1);
    const downloadEvent = page.waitForEvent('download');
    await page.getByRole('button', { name: arabic ? 'حفظ ملف' : 'Save file', exact: true }).click();
    const download = await downloadEvent;
    const destination = resolve(output, `diagnostics-${arabic ? 'ar' : 'en'}.json`);
    await download.saveAs(destination);
    assert.deepEqual(JSON.parse(await readFile(destination, 'utf8')), JSON.parse(copied));
    assert.equal(await page.evaluate(() => localStorage.getItem('quran-mushaf-qul-reader:v1')), preferences);
    assert.equal(await page.evaluate(() => localStorage.getItem('fixture-download-index')), 'keep-downloads');
    await page.screenshot({ path: resolve(output, `recovery-${arabic ? 'ar' : 'en'}.png`) });
    await page.getByRole('button', { name: arabic ? 'إغلاق التطبيق' : 'Close app', exact: true }).click();
    assert.deepEqual(await page.evaluate(() => window.__windowActions.at(-1)), { type: 'window-action', action: 'close' });
    await page.getByRole('button', { name: arabic ? 'إعادة تحميل التطبيق' : 'Reload app', exact: true }).click();
    await page.locator('#ready').waitFor();
    report.push({ language: arabic ? 'ar' : 'en', retry: true, reload: true, diagnosticCopyAndSave: true, sanitized: true, settingsPreserved: true, nativeClose: true });
    await context.close();
  }
  await writeFile(resolve(output, 'verification.json'), `${JSON.stringify(report, null, 2)}\n`);
  console.log('Localized recovery, retry/reload, non-sensitive diagnostics, retained preferences, and native close passed.');
} finally {
  await browser?.close();
  await new Promise(done => server.close(done));
}
