import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { extname, resolve, sep } from 'node:path';
import { chromium } from 'playwright';

// Run the immutable published build first, then dist after a candidate build:
//   node scripts/verify-listening-memory.mjs --baseline
//   node scripts/verify-listening-memory.mjs
//   node scripts/verify-listening-memory.mjs --placement=side
// No build, GC, user profile, real provider traffic, or media clock override.
// Native MP3 decoding runs normally; explicit seeks accelerate follow-along.
const option = (name, fallback) => process.argv.find(arg => arg.startsWith(`--${name}=`))?.slice(name.length + 3) ?? fallback;
const baseline = process.argv.includes('--baseline');
const requestedPlacement = option('placement', 'bottom');
assert.ok(['bottom', 'side'].includes(requestedPlacement), '--placement must be bottom or side');
assert.ok(!baseline || requestedPlacement === 'bottom', 'The published baseline uses its original bottom placement');
const placement = requestedPlacement;
const root = resolve(option('root', baseline ? 'outputs/rust-windows/publish/WebApp' : 'dist'));
const output = resolve(option('output', 'outputs/listening-memory-qa'));
const label = option('label', baseline ? 'before' : placement === 'side' ? 'after-side' : 'after');
const modes = option('modes', 'natural,contextual').split(',');
const sources = option('sources', 'network,cached').split(',');
const idleMs = Number(option('idle-ms', '15000'));
const stationaryMs = Number(option('stationary-ms', '10000'));
const switchCount = Number(option('switches', '8'));
const targetMiB = Number(option('fixture-mib', '22'));
const viewport = { width: 1920, height: 1080 };
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const audioUrls = [
  'https://cdn.mp3quran.net/audio/yasser-dosari/r1/002.mp3',
  'https://cdn.mp3quran.net/audio/nasser-qatami/r1/002.mp3',
];
const timingUrl = read => `https://www.mp3quran.net/api/v3/ayat_timing?surah=2&read=${read}`;
assert.equal(process.platform, 'win32', 'Windows OS counters are required');
assert.ok(modes.every(value => ['natural', 'contextual'].includes(value)));
assert.ok(sources.every(value => ['network', 'cached'].includes(value)));

async function buildFingerprint() {
  const index = await readFile(resolve(root, 'index.html'));
  const paths = [...index.toString().matchAll(/(?:src|href)="(\/assets\/[^\"]+\.(?:js|css))"/g)].map(match => match[1]);
  const assets = [];
  for (const path of paths) { const bytes = await readFile(resolve(root, `.${path}`)); assets.push({ path, bytes: bytes.length, sha256: digest(bytes) }); }
  let buildRecord = null;
  try { buildRecord = JSON.parse(await readFile(resolve(root, 'WEB-BUILD-SOURCE.json'), 'utf8')); } catch { /* Older build may not have a record. */ }
  return { root, indexSha256: digest(index), assets, buildRecord };
}

function longMp3(original) {
  // Keep complete original MPEG-1 Layer III frames, strip stale ID3 metadata,
  // then prepend one metadata frame with truthful Info duration/byte/seek data.
  // This is repeated authentic Ikhlas audio, not a claimed Baqarah recording.
  const tagSize = original.subarray(0, 3).toString() === 'ID3'
    ? 10 + (original[6] << 21 | original[7] << 14 | original[8] << 7 | original[9]) : 0;
  let cursor = tagSize, frameCount = 0, rate = 0, firstLength = 0;
  while (cursor + 4 <= original.length) {
    const word = original.readUInt32BE(cursor);
    if ((word >>> 21) !== 0x7ff) break;
    assert.equal(word >>> 19 & 3, 3, 'fixture must be MPEG-1');
    assert.equal(word >>> 17 & 3, 1, 'fixture must be Layer III');
    const bitrate = [0,32,40,48,56,64,80,96,112,128,160,192,224,256,320][word >>> 12 & 15] * 1000;
    rate = [44100,48000,32000][word >>> 10 & 3];
    assert.ok(bitrate && rate);
    const length = Math.floor(144 * bitrate / rate) + (word >>> 9 & 1);
    if (cursor + length > original.length) break;
    firstLength ||= length; cursor += length; frameCount++;
  }
  assert.ok(frameCount > 100);
  const encoded = original.subarray(tagSize, cursor);
  const repetitions = Math.ceil(targetMiB * 1048576 / encoded.length);
  const info = Buffer.alloc(firstLength);
  original.copy(info, 0, tagSize, tagSize + 4);
  const mono = (info[3] >>> 6) === 3;
  const position = 4 + (mono ? 17 : 32);
  info.write('Info', position, 'ascii'); info.writeUInt32BE(7, position + 4);
  info.writeUInt32BE(frameCount * repetitions, position + 8);
  info.writeUInt32BE(info.length + encoded.length * repetitions, position + 12);
  for (let index = 0; index < 100; index++) info[position + 16 + index] = Math.floor(index * 256 / 100);
  const body = Buffer.concat([info, ...Array(repetitions).fill(encoded)]);
  const duration = frameCount * repetitions * 1152 / rate;
  const timingDurationMs = Math.floor((duration - .2) * 1000);
  const timings = Array.from({ length: 286 }, (_, index) => ({
    ayah: index + 1, start_time: Math.floor(index * timingDurationMs / 286),
    end_time: Math.floor((index + 1) * timingDurationMs / 286),
  }));
  return { body, timings, metadata: {
    originalFixture: 'tests/fixtures/yasser-continuous-112.mp3', originalSha256: digest(original),
    originalBytes: original.length, parsedAudioFrames: frameCount, repetitions, bytes: body.length,
    sha256: digest(body), expectedDurationSeconds: duration, sampleRate: rate,
    description: 'Losslessly repeated authentic Ikhlas MPEG frames with new Info counts/TOC. Synthetic 286-ayah timings span the media duration and exercise Baqarah page navigation; audio words do not correspond to those verses. No re-encoding, silence padding, or fake audio element.',
  } };
}

function processMemory(processes) {
  const ids = processes.map(item => Number(item.id)).filter(id => Number.isSafeInteger(id) && id > 0);
  assert.ok(ids.length);
  const command = `$ErrorActionPreference='Stop'; $items=@(Get-Process -Id ${ids.join(',')} -ErrorAction SilentlyContinue | ForEach-Object { [pscustomobject]@{id=$_.Id;name=$_.ProcessName;workingBytes=$_.WorkingSet64;privateBytes=$_.PrivateMemorySize64} }); ConvertTo-Json -Compress -InputObject $items`;
  const raw = execFileSync('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', command], { encoding: 'utf8', windowsHide: true });
  const kinds = new Map(processes.map(item => [Number(item.id), item.type]));
  const entries = JSON.parse(raw).map(item => ({ ...item, type: kinds.get(item.id) ?? 'unknown' }));
  return {
    privateBytes: entries.reduce((sum, item) => sum + item.privateBytes, 0),
    workingBytes: entries.reduce((sum, item) => sum + item.workingBytes, 0), processes: entries,
    rendererPrivateBytes: entries.filter(item => item.type === 'renderer').reduce((sum, item) => sum + item.privateBytes, 0),
    gpuPrivateBytes: entries.filter(item => item.type === 'GPU').reduce((sum, item) => sum + item.privateBytes, 0),
  };
}

const fixture = longMp3(await readFile(resolve('tests/fixtures/yasser-continuous-112.mp3')));
const firstPages = JSON.parse(await readFile(resolve('src/data/ayahFirstPages.json'), 'utf8'))[1];
const destinations = firstPages.flatMap((page, index) => index === 0 || Math.floor((page - 1) / 2) !== Math.floor((firstPages[index - 1] - 1) / 2) ? [{ ayah: index + 1, page }] : []);
const report = {
  baseline, label, placement, startedAt: new Date().toISOString(), fingerprint: await buildFingerprint(),
  viewport, deviceScaleFactor: 1, forcedGarbageCollection: false, idleMs, stationaryMs, switchCount,
  fixture: fixture.metadata, scenarios: [],
  processMemoryMeaning: 'Windows Get-Process private bytes and working sets for SystemInfo.getProcessInfo PIDs of this newly launched isolated Edge instance. Shared working-set pages may be counted more than once. The Node fixture server and other browsers are excluded.',
  limitations: [
    'Headless desktop Edge is not the packaged WebView2 host, its Task Manager group, native window, or actual GPU presentation path. Do not equate these totals with the user screenshot.',
    'A ~24-minute encoded MP3 is genuinely decoded, but seeking accelerates its timeline; this is not 24 minutes of uninterrupted wall-clock listening.',
    'Synthetic timing/audio association tests state/page transitions, not publisher timing correctness or recitation content.',
    'All external requests are blocked or replaced with deterministic local fixtures. MP3 source URLs redirect to a local HTTP server with range responses; provider/CDN latency and HTTP caching are not represented. Playwright routing disables browser HTTP cache; CacheStorage remains real.',
    'Sampling shows sampled peaks, not guaranteed transient maxima. JS heap excludes much native media/GPU/cache memory. Live DOM canvas dimensions estimate RGBA pixels, not all retained GPU allocations.',
    'No forced GC, heap snapshot, cache trimming, process working-set trimming, or browser memory pressure request is used. Normal collection and allocator high-water retention remain visible.',
  ],
};
if (process.argv.includes('--fixture-only')) { console.log(JSON.stringify(fixture.metadata, null, 2)); process.exit(0); }
await mkdir(output, { recursive: true });
const reportPath = resolve(output, `listening-memory-${label}.json`);
const mime = { '.html':'text/html', '.js':'text/javascript', '.css':'text/css', '.json':'application/json', '.otf':'font/otf', '.woff2':'font/woff2' };
const serverRequests = [];
const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://localhost');
  if (url.pathname === '/__listening_fixture.mp3') {
    const range = request.headers.range?.match(/^bytes=(\d+)-(\d*)$/);
    const start = range ? Number(range[1]) : 0;
    const end = range?.[2] ? Math.min(Number(range[2]), fixture.body.length - 1) : fixture.body.length - 1;
    if (start > end || start >= fixture.body.length) return response.writeHead(416, { 'content-range': `bytes */${fixture.body.length}` }).end();
    const transfer = { startedAt: new Date().toISOString(), range: request.headers.range ?? null, start, end, sentBytes: 0 };
    serverRequests.push(transfer);
    response.writeHead(range ? 206 : 200, { 'content-type':'audio/mpeg', 'content-length':String(end-start+1), 'accept-ranges':'bytes', 'access-control-allow-origin':'*', 'cache-control':'no-store', ...(range ? { 'content-range':`bytes ${start}-${end}/${fixture.body.length}` } : {}) });
    try {
      // Keep Node's payload out of browser memory; use actual socket streaming,
      // not a 22MiB route.fulfill protocol message or base64 in page.evaluate.
      for (let offset = start; offset <= end && !response.destroyed; offset += 64 * 1024) {
        const chunk = fixture.body.subarray(offset, Math.min(offset + 64 * 1024, end + 1));
        transfer.sentBytes += chunk.length;
        if (!response.write(chunk)) await new Promise(done => {
          const cleanup = () => { response.off('drain', finished); response.off('close', finished); response.off('error', finished); };
          const finished = () => { cleanup(); done(); };
          response.once('drain', finished); response.once('close', finished); response.once('error', finished);
        });
      }
      response.end();
    } catch { response.destroy(); }
    return;
  }
  const path = resolve(root, `.${decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname)}`);
  if (!path.startsWith(root + sep)) return response.writeHead(403).end();
  try { const bytes = await readFile(path); response.writeHead(200, { 'content-type':mime[extname(path)] ?? 'application/octet-stream' }).end(bytes); }
  catch { response.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const origin = `http://127.0.0.1:${server.address().port}`;
const persist = () => writeFile(reportPath, JSON.stringify(report, null, 2));

try {
  for (const mode of modes) for (const sourceKind of sources) {
    const contextual = mode === 'contextual';
    const scenario = { mode, sourceKind, placement, startedAt: new Date().toISOString(), samples: [], errors: [], blockedRequests: [], remoteAudioRequests: [], followedDestinations: [], sourceSwitches: [] };
    report.scenarios.push(scenario); await persist();
    const serverRequestStart = serverRequests.length;
    const browser = await chromium.launch({ channel:'msedge', headless:true, args:['--autoplay-policy=no-user-gesture-required'] });
    let page;
    try {
      scenario.browserVersion = browser.version();
      const context = await browser.newContext({ viewport, deviceScaleFactor:1, timezoneId:'Asia/Qatar', serviceWorkers:'block' });
      await context.addInitScript(({contextual, playerPlacement}) => {
        localStorage.setItem('quran-reader-onboarding-v2','done');
        localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({ settingsVersion:5, languageSetupVersion:1, hasChosenLanguage:true, page:2, mode:'spread', theme:'dark', metadataLanguage:'english', contextualText:contextual, textMode:'natural', showClock:false, ...(playerPlacement ? {playerPlacement} : {}) }));
        localStorage.setItem('quran-listening-preferences-v1',JSON.stringify({ mode:'ayah', ayahReciterId:1001, speed:1, volume:.5, continuous:false }));
        // Count resources without retaining Blob objects, media sources, or nodes.
        const create = URL.createObjectURL.bind(URL), revoke = URL.revokeObjectURL.bind(URL), live = new Map();
        window.__listeningProbe = { created:0, revoked:0, live, events:[] };
        URL.createObjectURL = object => { const url=create(object); live.set(url,{type:object instanceof Blob?'Blob':object.constructor.name,bytes:object.size??null}); window.__listeningProbe.created++; return url; };
        URL.revokeObjectURL = url => { live.delete(url); window.__listeningProbe.revoked++; return revoke(url); };
      }, {contextual, playerPlacement:baseline ? null : placement});
      page = await context.newPage(); page.setDefaultTimeout(30000);
      page.on('pageerror', error => scenario.errors.push(error.message));
      await page.route('**/*', async route => {
        const url = route.request().url();
        if (url.startsWith(`${origin}/`)) return route.continue();
        if (url.includes('/ayat_timing?')) return route.fulfill({ json:fixture.timings, headers:{'access-control-allow-origin':'*'} });
        if (audioUrls.includes(url)) {
          scenario.remoteAudioRequests.push({ url, at:new Date().toISOString(), range:route.request().headers().range ?? null });
          return route.fulfill({ status:302, headers:{location:`${origin}/__listening_fixture.mp3`,'access-control-allow-origin':'*'} });
        }
        scenario.blockedRequests.push(url); return route.abort();
      });
      await page.goto(origin);
      await page.locator('.readerViewport [data-page-number="2"] .ayahRun').first().waitFor();
      await page.evaluate(() => document.fonts.ready);
      if (contextual) await page.locator('.readerViewport .shapedInkCanvas').first().waitFor();
      if (sourceKind === 'cached') {
        // Seed real CacheStorage from socket responses; transfer no MP3 bytes
        // through Playwright and retain no fixture body in application JS.
        scenario.seededCache = await page.evaluate(async ({audioUrls, timings, origin}) => {
          const cache = await caches.open('quran-audio-v1');
          for (let index=0;index<audioUrls.length;index++) {
            await cache.put(audioUrls[index], await fetch(`${origin}/__listening_fixture.mp3`));
            const read = index === 0 ? 92 : 86;
            const json = JSON.stringify(timings);
            await cache.put(`https://www.mp3quran.net/api/v3/ayat_timing?surah=2&read=${read}`,new Response(json,{headers:{'content-type':'application/json','content-length':String(new TextEncoder().encode(json).length)}}));
          }
          return Promise.all((await cache.keys()).map(async key => ({url:key.url,bytes:Number((await cache.match(key)).headers.get('content-length'))})));
        }, { audioUrls, timings:fixture.timings, origin });
      }
      await page.waitForTimeout(idleMs);
      const cdp = await context.newCDPSession(page), browserCdp = await browser.newBrowserCDPSession();
      await cdp.send('Performance.enable');
      const sample = async (phase, details = {}) => {
        const metrics = Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(item=>[item.name,item.value]));
        const os = processMemory((await browserCdp.send('SystemInfo.getProcessInfo')).processInfo);
        const live = await page.evaluate(() => {
          const canvasDetails = [...document.querySelectorAll('canvas')].map(canvas=>({ width:canvas.width,height:canvas.height,bytes:canvas.width*canvas.height*4,preview:!!canvas.closest('[data-preview-page]'),fold:!!canvas.closest('.bookFoldOverlay') }));
          const a = document.querySelector('audio');
          const audio = a ? { source:a.getAttribute('src'), currentSource:a.currentSrc, sourceKind:a.src.startsWith('blob:')?'Blob':a.src.startsWith('http')?'network':'empty',seconds:a.currentTime,duration:Number.isFinite(a.duration)?a.duration:null,paused:a.paused,ended:a.ended,readyState:a.readyState,networkState:a.networkState,error:a.error?.message??null,buffered:Array.from({length:a.buffered.length},(_,index)=>[a.buffered.start(index),a.buffered.end(index)]) } : null;
          const probe = window.__listeningProbe;
          return { pages:[...document.querySelectorAll('.readerViewport .pageSlot > .qulPage')].map(item=>Number(item.dataset.pageNumber)).sort((a,b)=>a-b),audio,
            highlightedAyahs:[...document.querySelectorAll('.readerViewport .ayahRun-reciting')].map(item=>item.dataset.ayahKey),
            liveDomElements:document.querySelectorAll('*').length,previewPages:document.querySelectorAll('[data-preview-page]').length,retainedFolds:document.querySelectorAll('.bookFoldOverlay').length,
            canvasStats:{ count:canvasDetails.length,bytes:canvasDetails.reduce((sum,item)=>sum+item.bytes,0),previewCount:canvasDetails.filter(item=>item.preview).length,previewBytes:canvasDetails.filter(item=>item.preview).reduce((sum,item)=>sum+item.bytes,0),foldCount:canvasDetails.filter(item=>item.fold).length,foldBytes:canvasDetails.filter(item=>item.fold).reduce((sum,item)=>sum+item.bytes,0) },
            objectUrls:{created:probe.created,revoked:probe.revoked,live:[...probe.live.values()]},
          };
        });
        const value = { phase,...details,sampledAt:new Date().toISOString(),...os,...live,jsHeapUsedBytes:metrics.JSHeapUsedSize,jsHeapTotalBytes:metrics.JSHeapTotalSize,runtimeHeap:await cdp.send('Runtime.getHeapUsage'),dom:await cdp.send('Memory.getDOMCounters') };
        scenario.samples.push(value); await persist();
        console.log(JSON.stringify({mode,sourceKind,phase,...details,privateMiB:+(os.privateBytes/1048576).toFixed(1),workingMiB:+(os.workingBytes/1048576).toFixed(1),heapMiB:+(value.jsHeapUsedBytes/1048576).toFixed(1),canvasMiB:+(live.canvasStats.bytes/1048576).toFixed(1),pages:live.pages,audioSource:live.audio?.sourceKind,objectUrls:live.objectUrls.live.length}));
        return value;
      };
      await sample('reading-idle');
      const access = page.locator('.sideToolsAccess');
      if (await access.count() && await access.getAttribute('aria-expanded') === 'false') await access.click();
      await page.getByRole('button',{name:'Listen',exact:true}).click();
      await page.locator('.readingToolsPanel').waitFor();
      const panel = page.locator('.readingToolsPanel');
      await panel.getByRole('combobox',{name:'Surah',exact:true}).selectOption('2');
      await panel.getByRole('button',{name:'Play',exact:true}).click();
      await page.waitForFunction(()=>{const a=document.querySelector('audio');return a&&!a.paused&&a.currentTime>.2&&a.duration>1000;});
      const initial = await sample('stationary-start');
      assert.equal(initial.audio.sourceKind, sourceKind === 'cached' ? 'Blob' : 'network');
      assert.ok(Math.abs(initial.audio.duration-fixture.metadata.expectedDurationSeconds)<.15,'native decoder observes accurate long-recording metadata');
      for (let elapsed=0;elapsed<stationaryMs;elapsed+=5000) {
        await page.waitForTimeout(Math.min(5000,stationaryMs-elapsed));
        const stationary = await sample('stationary-playing',{elapsedMs:Math.min(elapsed+5000,stationaryMs)});
        assert.deepEqual(stationary.pages,initial.pages,'stationary playback must not change pages');
      }
      await panel.locator('.tafsirPanelHeader').getByRole('button',{name:'Close',exact:true}).click();
      await page.waitForFunction(()=>!document.querySelector('.readingToolsPanel'));
      const dock = page.locator('.recitationDock');
      const reveal = async () => {
        if (await dock.getAttribute('data-expanded') !== 'true') await dock.getByRole('button',{name:'Show recitation controls',exact:true}).click();
        await page.waitForFunction(()=>document.querySelector('.recitationDock')?.dataset.expanded==='true');
      };
      const hide = async () => {
        await reveal(); await dock.getByRole('button',{name:'Collapse player',exact:true}).click(); await page.mouse.move(8,80);
        await page.waitForFunction(()=>document.querySelector('.recitationDock')?.dataset.expanded==='false');
      };
      await hide(); await page.waitForTimeout(600);
      await sample('follow-start');
      for (const [index,destination] of destinations.entries()) {
        const seconds=fixture.timings[destination.ayah-1].start_time/1000+.25;
        await page.locator('audio').evaluate((audio,seconds)=>{audio.currentTime=seconds;},seconds);
        await page.waitForFunction(({page,ayah})=>document.querySelector(`.readerViewport .pageSlot > .qulPage[data-page-number="${page}"]`)&&document.querySelector(`.readerViewport .ayahRun-reciting[data-ayah-key="2:${ayah}"]`),destination);
        await page.waitForFunction(()=>!document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)'));
        scenario.followedDestinations.push({...destination,seconds});
        if(index%4===0||index===destinations.length-1) await sample('follow-along',{destination:index+1,ayah:destination.ayah});
      }
      await reveal(); await dock.getByRole('button',{name:'Pause',exact:true}).click();
      const pausedAt=await page.locator('audio').evaluate(audio=>audio.currentTime);
      await page.waitForTimeout(1000); assert.equal(await page.locator('audio').evaluate(audio=>audio.currentTime),pausedAt);
      await sample('paused-after-follow');
      const previewSizes = () => page.locator('.pageTurnPreviews canvas.shapedInkCanvas').evaluateAll(items => items.map(c => `${c.closest('[data-preview-page]')?.dataset.previewPage}:${c.width}x${c.height}`));
      const stablePreviews = await previewSizes();
      // Keep the clock paused during this separate layout phase, so changes in
      // canvas/GPU memory cannot be attributed to subsequent media progression.
      for(let iteration=0;iteration<6;iteration++) {
        await hide(); await page.waitForTimeout(400);
        if (!baseline && mode === 'contextual' && placement === 'bottom') assert.deepEqual(await previewSizes(), stablePreviews, 'hidden neighbors retain full-quality backing sizes when the player collapses');
        await reveal(); await page.waitForTimeout(400);
        if (!baseline && mode === 'contextual' && placement === 'bottom') assert.deepEqual(await previewSizes(), stablePreviews, 'revealing controls does not resize hidden neighboring artwork');
        if (!baseline && placement === 'bottom') assert.equal(await page.locator('.bookFoldWarmup').count(), 0, 'player-only layout changes do not prepare unused 3D folds');
        if(iteration===0||iteration===5) await sample('player-layout',{iteration:iteration+1});
      }
      scenario.stablePreviewResizeVerified = !baseline && mode === 'contextual' && placement === 'bottom';
      scenario.noSpeculativePlayerFoldsVerified = !baseline && placement === 'bottom';
      await dock.getByRole('button',{name:'Listening settings',exact:true}).click();
      await panel.waitFor();
      await panel.getByRole('button',{name:'Resume',exact:true}).click();
      for(let index=0;index<switchCount;index++) {
        const reciter=index%2===0?'Nasser':'Yasser';
        const old=await page.locator('audio').getAttribute('src');
        await page.locator('#ayah-reciter-search').fill(reciter);
        await page.getByRole('option').filter({hasText:reciter}).first().click();
        await page.waitForFunction(old=>{const a=document.querySelector('audio');return a&&!a.paused&&a.currentTime>.15&&a.getAttribute('src')!==old&&a.duration>1000;},old);
        scenario.sourceSwitches.push({index:index+1,reciter});
        await sample('source-switch',{iteration:index+1,reciter});
      }
      // Reciter switches preserve the selected ayah. Reset that selection so
      // this next check repeats a whole chapter, rather than its final verses.
      await panel.getByRole('combobox',{name:'Ayah',exact:true}).selectOption('1');
      await page.waitForFunction(()=>{const a=document.querySelector('audio');return a&&!a.paused&&a.currentTime>.1&&a.currentTime<3;});
      await panel.getByRole('checkbox',{name:'Repeat playback',exact:true}).check();
      const repeatSource=await page.locator('audio').getAttribute('src');
      await page.locator('audio').evaluate(audio=>{audio.currentTime=audio.duration-.3;});
      await page.waitForFunction(()=>{const a=document.querySelector('audio');return a&&!a.paused&&a.currentTime>.1&&a.currentTime<3;});
      scenario.nativeEndedRepeatRestart=true;
      await sample('native-ended-repeat');
      await panel.getByRole('checkbox',{name:'Repeat playback',exact:true}).uncheck();
      await page.locator('audio').evaluate(audio=>{audio.currentTime=audio.duration-.3;});
      await page.waitForFunction(()=>document.querySelector('audio')?.ended);
      scenario.nativeEndedWholeSurah=true; scenario.repeatSourceBefore=repeatSource;
      await sample('native-ended-finished');
      await panel.getByRole('button',{name:'Stop recitation',exact:true}).click();
      await page.waitForFunction(()=>{const a=document.querySelector('audio');return a&&a.paused&&a.currentTime<.1;});
      await sample('stop-immediate');
      await page.waitForTimeout(idleMs);
      const final=await sample('stop-natural-idle');
      assert.deepEqual(scenario.errors,[],'no uncaught page errors');
      assert.equal(final.retainedFolds,0,'no live fold overlays remain after natural idle');
      if (baseline) assert.ok(final.objectUrls.live.length<=1,'the historical baseline may prepare one paused recording after Stop');
      else {
        assert.equal(final.objectUrls.live.length,0,'Stop releases every playback object URL');
        assert.equal(final.audio.source,null,'Stop leaves no media source or replacement recording');
      }
      if(sourceKind==='cached') assert.equal(scenario.remoteAudioRequests.length,0,'cached listening uses only saved MP3 blobs');
      scenario.serverTransfers=serverRequests.slice(serverRequestStart);
      scenario.summary={ readingIdlePrivateMiB:scenario.samples[0].privateBytes/1048576,stationaryStartPrivateMiB:initial.privateBytes/1048576,peakSampledPrivateMiB:Math.max(...scenario.samples.map(sample=>sample.privateBytes))/1048576,stopIdlePrivateMiB:final.privateBytes/1048576,stopIdleWorkingMiB:final.workingBytes/1048576,stopIdleHeapMiB:final.jsHeapUsedBytes/1048576,peakCanvasMiB:Math.max(...scenario.samples.map(sample=>sample.canvasStats.bytes))/1048576,stopIdleCanvasMiB:final.canvasStats.bytes/1048576,followedSpreads:scenario.followedDestinations.length,sourceSwitches:scenario.sourceSwitches.length };
      scenario.finishedAt=new Date().toISOString(); await persist();
    } catch(error) {
      scenario.failure={message:error.message,stack:error.stack};
      if(page&&!page.isClosed()) { scenario.failure.state=await page.evaluate(()=>({audio:[...document.querySelectorAll('audio')].map(a=>({src:a.src,seconds:a.currentTime,duration:a.duration,paused:a.paused,error:a.error?.message})),panel:document.querySelector('.readingToolsPanel')?.textContent})).catch(()=>null); await page.screenshot({path:resolve(output,`failure-${label}-${mode}-${sourceKind}.png`)}).catch(()=>{}); }
      await persist(); throw error;
    } finally { await browser.close(); }
  }
  report.finalFingerprint=await buildFingerprint();
  assert.deepEqual(report.finalFingerprint,report.fingerprint,'served build bytes remained immutable throughout probe');
  report.finishedAt=new Date().toISOString(); await persist();
  console.log(JSON.stringify({reportPath,summaries:report.scenarios.map(({mode,sourceKind,summary})=>({mode,sourceKind,...summary}))},null,2));
} finally { await new Promise(done=>server.close(done)); }
