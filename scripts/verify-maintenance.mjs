import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';


const root = resolve('dist');
const output = resolve('outputs/maintenance-0.4.23');
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
  const poisonedUrl = 'https://verses.quran.foundation/Alafasy/mp3/' + ayahKey.split(':').map(n=>String(n).padStart(3,'0')).join('') + '.mp3';
  await page.evaluate(async url => (await caches.open('quran-audio-v1')).put(url,new Response('<html>error</html>',{headers:{'content-type':'audio/mpeg'}})),poisonedUrl);
  await page.locator('.pageSlot .ayahRun').first().click({button:'right'});
  await page.locator('.ayahActionsMenu .ayahPrimaryAction').nth(1).click();
  await page.locator('.listeningRangeSummary').waitFor();
  results.rangeBeforeSameModeClick = await page.locator('.listeningRangeSummary').textContent();
  await page.getByRole('button',{name:'Follow along',exact:true}).click();
  results.rangeAfterSameModeClick = await page.locator('.listeningRangeSummary').count();
  assert.equal(results.rangeAfterSameModeClick, 1);
  await page.waitForFunction(()=>document.querySelector('audio').currentTime > .1);
  assert.ok(requestedAudio.includes(poisonedUrl), 'bad cached audio replaced with network recording');
  assert.equal(await page.evaluate(async url => Boolean(await (await caches.open('quran-audio-v1')).match(url)), poisonedUrl), false);

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
  await page.locator('.readerControlsReveal').click();
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
