import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { extname, resolve, sep } from 'node:path';
import { chromium } from 'playwright';

// Compare unique destinations, not two already-warm spreads. Acceptance samples
// use the browser's normal collection policy: this probe never requests a GC.
const baseline = process.argv.includes('--baseline');
const root = resolve(baseline ? 'outputs/rust-windows/publish/WebApp' : 'dist');
const output = resolve('outputs/memory-soak-qa');
const startPage = 45;
const spreadTurns = 60;
const burstTurns = 10;
const idleMilliseconds = 15_000;
const indexHtml = await readFile(resolve(root, 'index.html'), 'utf8');
const entryScript = indexHtml.match(/src="([^"]+\.js)"/)?.[1];
const report = {
  baseline, root, startedAt: new Date().toISOString(), forcedGarbageCollection: false,
  indexSha256: createHash('sha256').update(indexHtml).digest('hex'), entryScript,
  viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1,
  startPage, spreadTurns, inputIntervalMilliseconds: 30, idleMilliseconds,
  processMemoryMeaning: 'Sum of sampled Edge browser-tree process private bytes and working sets. Shared working-set pages can be counted in multiple processes; these are not Task Manager WebView2 group figures.',
  scenarios: [],
};
await mkdir(output, { recursive: true });
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.otf': 'font/otf', '.woff2': 'font/woff2' };
const server = createServer(async (req, res) => {
  const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
  const path = resolve(root, `.${pathname === '/' ? '/index.html' : pathname}`);
  if (!path.startsWith(root + sep)) return res.writeHead(403).end();
  try {
    const content = await readFile(path);
    res.writeHead(200, { 'content-type': mime[extname(path)] ?? 'application/octet-stream' }).end(content);
  } catch { res.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const base = `http://127.0.0.1:${server.address().port}`;

function processMemory(processes) {
  assert.equal(process.platform, 'win32', 'This probe measures Windows process memory.');
  const ids = processes.map(item => Number(item.id)).filter(id => Number.isInteger(id) && id > 0);
  assert.ok(ids.length, 'CDP returned the isolated browser process IDs');
  const command = `$ErrorActionPreference='Stop'; $items = @(Get-Process -Id ${ids.join(',')} -ErrorAction SilentlyContinue | ForEach-Object { [pscustomobject]@{id=$_.Id;name=$_.ProcessName;workingBytes=$_.WorkingSet64;privateBytes=$_.PrivateMemorySize64} }); ConvertTo-Json -Compress -InputObject $items`;
  const raw = execFileSync('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', command], { encoding: 'utf8', windowsHide: true });
  const measured = JSON.parse(raw);
  const kind = new Map(processes.map(item => [Number(item.id), item.type]));
  const entries = measured.map(item => ({ ...item, type: kind.get(item.id) ?? 'unknown' }));
  return {
    workingBytes: entries.reduce((sum, item) => sum + item.workingBytes, 0),
    privateBytes: entries.reduce((sum, item) => sum + item.privateBytes, 0),
    processes: entries,
  };
}

try {
  for (const contextualText of [false, true]) {
    // A fresh browser for each mode prevents previous renderer/GPU caches from
    // contaminating the next scenario and excludes any user's open Edge tabs.
    const browser = await chromium.launch({ channel: 'msedge', headless: true });
    try {
      report.browserVersion = browser.version();
      const context = await browser.newContext({ viewport: report.viewport, deviceScaleFactor: 1, timezoneId: 'Asia/Qatar' });
      await context.addInitScript(contextualText => {
        localStorage.setItem('quran-reader-onboarding-v2', 'done');
        localStorage.setItem('quran-mushaf-qul-reader:v1', JSON.stringify({
          settingsVersion: 5, languageSetupVersion: 1, hasChosenLanguage: true,
          page: 45, mode: 'spread', theme: 'dark', metadataLanguage: 'english',
          contextualText, textMode: 'natural', showClock: false,
        }));
      }, contextualText);
      const page = await context.newPage();
      page.setDefaultTimeout(45_000);
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.goto(base);
      const visibleNumbers = () => page.locator('.readerViewport .pageSlot > .qulPage').evaluateAll(elements => elements.map(element => Number(element.getAttribute('data-page-number'))).sort((a, b) => a - b));
      const waitForPages = async rightPage => {
        await page.waitForFunction(right => {
          const numbers = [...document.querySelectorAll('.readerViewport .pageSlot > .qulPage')].map(element => Number(element.getAttribute('data-page-number'))).sort((a, b) => a - b);
          return numbers.length === 2 && numbers[0] === right && numbers[1] === right + 1;
        }, rightPage);
      };
      await waitForPages(startPage);
      await page.evaluate(() => document.fonts.ready);
      if (contextualText) await page.locator('.readerViewport .pageSlot .shapedQuranLine').first().waitFor();
      await page.waitForTimeout(3500);
      const cdp = await context.newCDPSession(page);
      const browserCdp = await browser.newBrowserCDPSession();
      await cdp.send('Performance.enable');
      const samples = [];
      const sample = async (label, completedTurns) => {
        const metrics = Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(item => [item.name, item.value]));
        const os = processMemory((await browserCdp.send('SystemInfo.getProcessInfo')).processInfo);
        const canvasStats = await page.locator('canvas').evaluateAll(canvases => {
          const details = canvases.map(canvas => ({
            width: canvas.width, height: canvas.height, bytes: canvas.width * canvas.height * 4,
            preview: !!canvas.closest('[data-preview-page]'), fold: !!canvas.closest('.bookFoldOverlay'),
          }));
          return {
            count: details.length, bytes: details.reduce((sum, item) => sum + item.bytes, 0),
            previewBytes: details.filter(item => item.preview).reduce((sum, item) => sum + item.bytes, 0),
            foldBytes: details.filter(item => item.fold).reduce((sum, item) => sum + item.bytes, 0),
            maxWidth: Math.max(0, ...details.map(item => item.width)),
            maxHeight: Math.max(0, ...details.map(item => item.height)),
            maxBytes: Math.max(0, ...details.map(item => item.bytes)),
          };
        });
        const result = {
          label, completedTurns, sampledAt: new Date().toISOString(), pages: await visibleNumbers(),
          ...os, jsHeapUsedBytes: metrics.JSHeapUsedSize, jsHeapTotalBytes: metrics.JSHeapTotalSize,
          gpuPrivateBytes: os.processes.filter(item => item.type === 'GPU').reduce((sum, item) => sum + item.privateBytes, 0),
          rendererPrivateBytes: os.processes.filter(item => item.type === 'renderer').reduce((sum, item) => sum + item.privateBytes, 0),
          canvasStats, runtimeHeap: await cdp.send('Runtime.getHeapUsage'),
          taskSeconds: metrics.TaskDuration, layoutSeconds: metrics.LayoutDuration, recalcStyleSeconds: metrics.RecalcStyleDuration,
          dom: await cdp.send('Memory.getDOMCounters'),
          liveDomElements: await page.locator('*').count(),
          retainedFolds: await page.locator('.bookFoldOverlay').count(),
          previewPages: await page.locator('[data-preview-page]').count(),
        };
        samples.push(result);
        console.log(JSON.stringify({ mode: contextualText ? 'contextual' : 'natural', label, completedTurns, privateMiB: +(result.privateBytes / 1048576).toFixed(1), workingMiB: +(result.workingBytes / 1048576).toFixed(1), gpuPrivateMiB: +(result.gpuPrivateBytes / 1048576).toFixed(1), rendererPrivateMiB: +(result.rendererPrivateBytes / 1048576).toFixed(1), liveCanvasMiB: +(canvasStats.bytes / 1048576).toFixed(1), backingStorageMiB: +((result.runtimeHeap.backingStorageSize ?? 0) / 1048576).toFixed(1), heapMiB: +(result.jsHeapUsedBytes / 1048576).toFixed(1), domNodes: result.dom.nodes, liveDomElements: result.liveDomElements }));
        return result;
      };
      const initial = await sample('initial', 0);
      const navigationStarted = performance.now();
      for (let turns = 0; turns < spreadTurns; turns += burstTurns) {
        // Discrete clicks retain all queued destinations. Held-key releases
        // intentionally discard queued repeats, making their turn counts vary.
        for (let turn = 0; turn < burstTurns; turn++) {
          await page.locator('.sideNavNext').evaluate(button => button.click());
          await page.waitForTimeout(30);
        }
        const completed = turns + burstTurns;
        await waitForPages(startPage + completed * 2);
        await page.waitForFunction(() => !document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)'));
        await sample(completed === spreadTurns ? 'immediate-end' : 'navigation', completed);
      }
      const navigationMilliseconds = performance.now() - navigationStarted;
      const final = samples.at(-1);
      await page.waitForTimeout(idleMilliseconds);
      const idle = await sample('natural-idle-15s', spreadTurns);
      assert.deepEqual(idle.pages, [165, 166], 'All 60 unique spread turns reached their exact destination');
      assert.deepEqual(errors, [], 'No uncaught browser errors during sustained navigation');
      assert.equal(idle.retainedFolds, 0, 'No fold artwork retained after natural idle');
      assert.ok(idle.previewPages <= 4, 'At most the immediate neighboring preview pages remain');
      const scenario = {
        contextualText, navigationMilliseconds, errors, samples,
        summary: {
          initialPrivateMiB: initial.privateBytes / 1048576,
          peakSampledPrivateMiB: Math.max(...samples.map(item => item.privateBytes)) / 1048576,
          immediatePrivateMiB: final.privateBytes / 1048576,
          idlePrivateMiB: idle.privateBytes / 1048576,
          initialWorkingMiB: initial.workingBytes / 1048576,
          peakSampledWorkingMiB: Math.max(...samples.map(item => item.workingBytes)) / 1048576,
          idleWorkingMiB: idle.workingBytes / 1048576,
          initialHeapMiB: initial.jsHeapUsedBytes / 1048576,
          idleHeapMiB: idle.jsHeapUsedBytes / 1048576,
          initialDomNodes: initial.dom.nodes, idleDomNodes: idle.dom.nodes,
          peakSampledGpuPrivateMiB: Math.max(...samples.map(item => item.gpuPrivateBytes)) / 1048576,
          idleGpuPrivateMiB: idle.gpuPrivateBytes / 1048576,
          peakSampledRendererPrivateMiB: Math.max(...samples.map(item => item.rendererPrivateBytes)) / 1048576,
          idleRendererPrivateMiB: idle.rendererPrivateBytes / 1048576,
          peakSampledLiveCanvasMiB: Math.max(...samples.map(item => item.canvasStats.bytes)) / 1048576,
          idleLiveCanvasMiB: idle.canvasStats.bytes / 1048576,
          browserTaskMilliseconds: (final.taskSeconds - initial.taskSeconds) * 1000,
        },
      };
      report.scenarios.push(scenario);
      await writeFile(resolve(output, baseline ? 'memory-before.json' : 'memory-after.json'), JSON.stringify(report, null, 2));
      await context.close();
    } finally { await browser.close(); }
  }
  report.finishedAt = new Date().toISOString();
  await writeFile(resolve(output, baseline ? 'memory-before.json' : 'memory-after.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ baseline, summaries: report.scenarios.map(({ contextualText, summary }) => ({ contextualText, ...summary })) }));
} finally { await new Promise(done => server.close(done)); }
