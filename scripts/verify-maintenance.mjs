import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';


const root = resolve('dist');
const output = resolve('outputs/maintenance-qa');
await mkdir(output, { recursive: true });
const results = {};
const mime = {'.html':'text/html','.js':'text/javascript','.css':'text/css','.json':'application/json','.otf':'font/otf','.woff2':'font/woff2'};
const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
  const path = resolve(root, '.' + (url.pathname === '/' ? '/index.html' : url.pathname));
  if (!path.startsWith(root + sep)) return res.writeHead(403).end();
  try { res.writeHead(200, {'Content-Type':mime[extname(path)] || 'application/octet-stream'}).end(await readFile(path)); }
  catch { res.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const url = 'http://127.0.0.1:' + server.address().port;
const browser = await chromium.launch({channel:'msedge',headless:true});
try {
  const context = await browser.newContext({viewport:{width:1920,height:1051}});
  await context.addInitScript(() => {
    localStorage.setItem('quran-listening-preferences-v1',JSON.stringify({mode:'ayah',ayahReciterId:1002,speed:1,volume:1,continuous:false}));
    localStorage.setItem('quran-reader-onboarding-v2','done');
    localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,contextualText:false,languageSetupVersion:1,hasChosenLanguage:true,page:43,mode:'spread',theme:'dark',metadataLanguage:'english'}));
  });
  const page = await context.newPage();
  const held = [];
  await page.route('**/digital-khatt/shaped/**', route => { held.push(route); });
  await page.goto(url);
  await page.waitForTimeout(1200);
  results.optionalRenderingBlocksOrdinaryPages = {
    heldRequests:held.length,
    visiblePages:await page.locator('.pageSlot > .qulPage').count(),
  };
  assert.equal(held.length, 0, 'ordinary mode never requests optional artwork');
  assert.equal(results.optionalRenderingBlocksOrdinaryPages.visiblePages, 2);
  await Promise.all(held.map(route => route.abort()));
  await page.unroute('**/digital-khatt/shaped/**');
  await page.locator('.pageSlot .ayahRun').first().waitFor();
  results.optionalRenderingBlocksOrdinaryPages.pagesAfterAbort = await page.locator('.pageSlot > .qulPage').count();

  const fixture = await readFile('tests/fixtures/yasser-112001.mp3');
  const requestedAudio = [];
  await page.route('**/*.mp3', route => {
    requestedAudio.push(route.request().url());
    return route.fulfill({body:fixture, headers:{'content-type':'audio/mpeg','content-length':String(fixture.length),'access-control-allow-origin':'*'}});
  });
  const ayahKey = await page.locator('.pageSlot .ayahRun').first().getAttribute('data-ayah-key');
  const poisonedUrl = 'https://everyayah.com/data/MaherAlMuaiqly128kbps/' + ayahKey.split(':').map(n=>String(n).padStart(3,'0')).join('') + '.mp3';
  await page.route('**/api/v3/reciters?**', route => route.fulfill({json:{reciters:[{id:9001,name:'Additional fixture',moshaf:[{id:9002,name:'Hafs',server:'https://server8.mp3quran.net/fixture/',surah_list:'2,112'}]}]}}));
  await page.evaluate(async url => (await caches.open('quran-audio-v1')).put(url,new Response('<html>error</html>',{headers:{'content-type':'audio/mpeg'}})),poisonedUrl);
  await page.locator('.pageSlot .ayahRun').first().click({button:'right'});
  await page.locator('.ayahActionsMenu .ayahPrimaryAction').nth(1).click();
  await page.locator('.listeningRangeSummary').waitFor();
  results.rangeBeforeOpeningMoreReciters = await page.locator('.listeningRangeSummary').textContent();
  await page.locator('.moreReciters summary').click();
  results.rangeAfterOpeningMoreReciters = await page.locator('.listeningRangeSummary').textContent();
  assert.equal(results.rangeAfterOpeningMoreReciters, results.rangeBeforeOpeningMoreReciters, 'opening more reciters does not reset the active range');
  await page.locator('.moreReciters summary').click();
  assert.equal(await page.getByRole('combobox',{name:'Audio source',exact:true}).count(),0,'recording selection is automatic');
  await page.waitForFunction(()=>document.querySelector('audio').currentTime > .1);
  assert.ok(requestedAudio.includes(poisonedUrl), 'bad cached audio replaced with network recording');
  assert.equal(await page.evaluate(async url => Boolean(await (await caches.open('quran-audio-v1')).match(url)), poisonedUrl), false);
  await page.getByRole('button',{name:'Pause',exact:true}).click();

  const input = page.locator('#ayah-reciter-search');
  await input.fill('Yasser');
  assert.ok(await page.locator('#ayah-reciter-options [role=option]').count() > 0);
  await input.press('ArrowDown');
  await input.press('Enter');
  results.reciterKeyboard = {
    query:await input.inputValue(),
    popupOpen:await input.getAttribute('aria-expanded'),
    
  };
  assert.equal(results.reciterKeyboard.popupOpen, 'false');
  assert.equal(results.reciterKeyboard.query, 'Yasser Al-Dosari');
  await page.screenshot({path:resolve(output,'reciter-keyboard.png')});

  // Exercise the sanitizer through the actual translation panel, rather than a duplicate test sanitizer.
  await page.keyboard.press('Escape');
  const payload = '<p>Safe passage</p><img src=x onerror="window.auditXss=1"><svg onload="window.auditXss=1"></svg><a href="javascript:window.auditXss=1">Unsafe link</a><script>window.auditXss=1</script>';
  await page.route('**/data/study/sahih/s*.json', route => route.fulfill({json:Object.fromEntries(Array.from({length:286},(_,i)=>[i+1,{text:payload,footnotes:{}}]))}));
  await page.locator('.pageSlot .ayahRun').first().hover(); await page.keyboard.press('t');
  await page.getByText('Safe passage',{exact:true}).waitFor();
  assert.equal(await page.evaluate(()=>window.auditXss ?? 0),0);
  assert.equal(await page.locator('.tafsirPanel img, .tafsirPanel svg[onload], .tafsirPanel a[href], .tafsirPanel script').count(),0);
  await page.keyboard.press('Escape');
  // Switch to enhanced mode while its assets are delayed. Visible source pages remain usable.
  await page.keyboard.press('Control+f'); await page.keyboard.press('Escape');
  if (await page.locator('.readerControlsReveal').isVisible()) await page.locator('.readerControlsReveal').click();
  await page.getByRole('button',{name:'Settings',exact:true}).click();
  const optional = [];
  await page.route('**/digital-khatt/shaped/**', route => { optional.push(route); });
  await page.getByRole('checkbox',{name:'Contextual letter elongation (optional)',exact:true}).check();
  await page.waitForTimeout(400);
  assert.ok(optional.length > 0); assert.equal(await page.locator('.pageSlot > .qulPage').count(),2);
  await page.unroute('**/digital-khatt/shaped/**'); await Promise.all(optional.map(route=>route.continue()));
  await page.locator('.pageSlot .shapedLineInk').first().waitFor();
  results.enhancedModeRetainsSourcePages = true;
  console.log(JSON.stringify(results,null,2));
  await writeFile(resolve(output,'probe-results.json'),JSON.stringify(results,null,2));
} finally {await browser.close();server.close();}
