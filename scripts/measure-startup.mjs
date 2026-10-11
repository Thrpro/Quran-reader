import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

// Fresh browser contexts measure an uncached frontend launch, not native host startup.
const rootFlag = process.argv.indexOf('--root');
const root = resolve(rootFlag < 0 ? 'dist' : process.argv[rootFlag + 1]);
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.otf': 'font/otf', '.woff2': 'font/woff2' };
const server = createServer(async (req, res) => {
  const path = resolve(root, '.' + new URL(req.url, 'http://localhost').pathname.replace(/\/$/, '/index.html'));
  if (!path.startsWith(root + sep)) return res.writeHead(403).end();
  try { const content = await readFile(path); res.writeHead(200, { 'Content-Type': mime[extname(path)] ?? 'application/octet-stream' }).end(content); }
  catch { res.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const report = { scope: 'Uncached frontend in headless Edge; excludes native WebView2 process initialization', runs: [] };
let browser;
try {
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  for (let run = 0; run < 15; run++) {
    const context = await browser.newContext({ viewport: { width: 1920, height: 1051 } });
    await context.addInitScript(() => localStorage.setItem('quran-mushaf-qul-reader:v1', JSON.stringify({ settingsVersion: 5, languageSetupVersion: 1, hasChosenLanguage: true, page: 45, mode: 'spread', theme: 'dark', metadataLanguage: 'arabic', contextualText: false })));
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.waitForFunction(() => document.querySelectorAll('.readerViewport .pageSlot > .qulPage').length === 2);
    await page.evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))));
    report.runs.push(await page.evaluate(() => ({ readyMs: Math.round(performance.now()), initialScriptBytes: performance.getEntriesByType('resource').filter(r => r.initiatorType === 'script').reduce((n, r) => n + r.decodedBodySize, 0), scripts: performance.getEntriesByType('resource').filter(r => r.initiatorType === 'script').map(r => r.name.split('/').at(-1)) })));
    if (errors.length) throw Error(errors.join('\n'));
    await context.close();
  }
  report.medianReadyMs = report.runs.map(r => r.readyMs).sort((a, b) => a - b)[Math.floor(report.runs.length / 2)];
  const output = resolve('outputs/startup-qa', process.argv.includes('--baseline') ? 'before.json' : 'after.json');
  await mkdir(resolve('outputs/startup-qa'), { recursive: true });
  await writeFile(output, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally { await browser?.close(); await new Promise(done => server.close(done)); }
