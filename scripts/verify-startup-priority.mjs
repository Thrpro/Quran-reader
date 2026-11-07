import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

const baseline = process.argv.includes('--baseline');
const root = resolve(baseline ? 'outputs/rust-windows/publish/WebApp' : 'dist');
const output = resolve('outputs/startup-priority-qa');
await mkdir(output, {recursive:true});
const mime = {'.html':'text/html','.js':'text/javascript','.css':'text/css','.json':'application/json','.otf':'font/otf','.woff2':'font/woff2'};
const server = createServer(async (request,response) => {
  const path = resolve(root, '.' + new URL(request.url,'http://localhost').pathname.replace(/^\/$/,'/index.html'));
  if (!path.startsWith(root + sep)) return response.writeHead(403).end();
  try { const bytes = await readFile(path); response.writeHead(200,{'content-type':mime[extname(path)] ?? 'application/octet-stream'}).end(bytes); }
  catch { response.writeHead(404).end(); }
});
await new Promise(done => server.listen(0,'127.0.0.1',done));
const origin = 'http://127.0.0.1:' + server.address().port;
const browser = await chromium.launch({channel:'msedge',headless:true});
const report = {baseline,fontGates:[],pageFontPreparation:[],recovery:[]};
const visiblePage = '.readerViewport .pageSlot > .qulPage .ayahRun';
async function reader(pageNumber,language='english') {
  const page = await browser.newPage({viewport:{width:1280,height:900}});
  await page.addInitScript(({pageNumber,language}) => {
    localStorage.setItem('quran-reader-onboarding-v2','done');
    if (!localStorage.getItem('quran-mushaf-qul-reader:v1')) localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:pageNumber,mode:'spread',theme:'dark',metadataLanguage:language}));
  },{pageNumber,language});
  return page;
}
try {
  for (const pageNumber of baseline ? [45] : [45,1]) {
    const page = await reader(pageNumber), errors=[];
    page.on('pageerror',error=>errors.push(error.message));
    let release;
    const gate = new Promise(done=>{release=done;});
    let blocked=0;
    await page.route('**/fonts/{surah-header,surah-names}/**',async route=>{blocked++;await gate;await route.continue();});
    try {
      await page.goto(origin,{waitUntil:'domcontentloaded'});
      // Bounded observation, not a machine-speed launch benchmark. Only the
      // decorative fonts are held; source metadata, words and text font load.
      await page.waitForTimeout(2200);
      const renderedWhileHeld=await page.locator(visiblePage).count()>0;
      report.fontGates.push({pageNumber,blocked,renderedWhileHeld});
      assert.ok(blocked>0,'the auxiliary-font gate was exercised');
      if (!baseline) assert.equal(renderedWhileHeld,pageNumber===45,'only pages without headings may render before decorative fonts');
      release();
      await page.locator(visiblePage).first().waitFor();
      assert.deepEqual(errors,[]);
    } finally {release();await page.close();}
  }
  if (!baseline) {
    for (const language of ['english','arabic']) {
      const page=await reader(604,language),errors=[];
      page.on('pageerror',error=>errors.push(error.message));
      let releaseMetadata,releaseFonts;
      const metadataGate=new Promise(done=>{releaseMetadata=done;});
      const fontGate=new Promise(done=>{releaseFonts=done;});
      await page.route('**/data/qul/processed/surahs.json',async route=>{await metadataGate;await route.continue();});
      await page.route('**/fonts/{surah-header,surah-names}/**',async route=>{await fontGate;await route.continue();});
      // Install all observers before opening the reader. The visible source
      // spread must prepare its required faces without metadata or a UI poke.
      const sourceResponses=Promise.all([603,604].map(number=>page.waitForResponse(response=>response.url().endsWith(`/digital-khatt/pages/p${number}.json`))));
      const decorativeRequests=Promise.all([
        page.waitForRequest('**/fonts/surah-header/**'),
        page.waitForRequest('**/fonts/surah-names/**'),
      ]);
      try {
        await page.goto(origin,{waitUntil:'domcontentloaded'});
        const responses=await sourceResponses;
        for(const response of responses) {assert.equal(response.status(),200);await response.finished();}
        await decorativeRequests;
        assert.equal(await page.locator(visiblePage).count(),0,'unready required faces never display fallback Quran text');
        releaseFonts();
        await page.waitForFunction(()=>{
          const faces=[...document.fonts].filter(face=>face.family==='QUL Surah Header Color'||face.family==='QUL Surah Name V4');
          return faces.length===2&&faces.every(face=>face.status==='loaded');
        });
        assert.equal(await page.locator(visiblePage).count(),0,'holding metadata does not prevent independent required-font preparation');
        releaseMetadata();
        await page.locator('.readerViewport [data-ayah-key="112:1"]').first().waitFor();
        assert.deepEqual(errors,[]);
        report.pageFontPreparation.push({language,pages:[603,604],requiredFacesPreparedBeforeMetadata:true,noUiInteraction:true});
      } finally {releaseMetadata();releaseFonts();await page.close();}
    }
    for (const language of ['english','arabic']) {
      const page=await reader(45,language);let fail=true;
      await page.route('**/data/qul/processed/surahs.json',route=>fail?route.fulfill({status:503,body:'fixture failure'}):route.continue());
      await page.goto(origin);
      const alert=page.locator('.loadState[role="alert"]');await alert.waitFor();
      const text=await alert.innerText();
      assert.ok(language==='arabic'?text.includes('تعذر فتح المصحف'):text.includes('Could not open the Mushaf'));
      assert.ok(!text.includes('/data/')&&!text.includes('503'),'recovery does not expose internal paths');
      fail=false;await alert.getByRole('button').click();await page.locator(visiblePage).first().waitFor();
      assert.equal(await page.locator('.loadState[role="alert"]').count(),0);
      report.recovery.push({language,metadataRetry:true});await page.close();
    }
    for (const language of ['english','arabic']) {
      const page=await reader(604,language);let fail=true;
      await page.route('**/digital-khatt/pages/p604.json',route=>fail?route.fulfill({status:503,body:'fixture failure'}):route.continue());
      await page.goto(origin);
      const alert=page.locator('.loadState[role="alert"]');await alert.waitFor();
      assert.equal(await page.locator(visiblePage).count(),0,'a failed visible source page is not treated as a ready spread');
      assert.ok((await alert.innerText()).includes(language==='arabic'?'تعذر تحميل الصفحة.':'Could not load this page.'),'source errors retain their localized page retry path');
      assert.equal(await alert.getByRole('button',{name:language==='arabic'?'إعادة المحاولة':'Retry',exact:true}).count(),1);
      fail=false;await alert.getByRole('button').click();
      await page.locator('.readerViewport [data-ayah-key="112:1"]').first().waitFor();
      assert.equal(await page.locator('.loadState[role="alert"]').count(),0);
      assert.ok(await page.locator('.readerViewport [data-page-number="604"]').count(),'page retry preserves reading position');
      report.recovery.push({language,pageDataRetry:true,preservedPage:604});await page.close();
    }
    const page=await reader(1);let fail=true;
    await page.route('**/fonts/surah-header/**',route=>fail?route.fulfill({status:503,body:'fixture failure'}):route.continue());
    await page.goto(origin);
    const alert=page.locator('.loadState[role="alert"]');await alert.waitFor();
    assert.equal(await page.locator(visiblePage).count(),0,'a failed required font never displays fallback Quran text');
    fail=false;await alert.getByRole('button',{name:'Reload app',exact:true}).click();
    await page.locator(visiblePage).first().waitFor();
    assert.ok(await page.locator('.readerViewport [data-page-number="1"]').count(),'reload preserves reading position');
    report.recovery.push({requiredFontReload:true,preservedPage:1});await page.close();
  }
  await writeFile(resolve(output,baseline?'before.json':'verification.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report,null,2));
} catch(error) {
  await writeFile(resolve(output,baseline?'before.json':'verification.json'),JSON.stringify({...report,failure:error.message},null,2));throw error;
} finally {await browser.close();await new Promise(done=>server.close(done));}
