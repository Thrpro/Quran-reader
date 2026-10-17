import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

// An already built dist, isolated profiles, authentic local audio fixtures,
// no provider uptime dependency, no user profile and no forced collection.
const root = resolve('dist'), output = resolve('outputs/listening-refinements-qa');
const native = !process.argv.includes('--browser');
await mkdir(output, { recursive: true });
const mime = { '.html':'text/html', '.js':'text/javascript', '.css':'text/css', '.json':'application/json', '.otf':'font/otf', '.woff2':'font/woff2' };
const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://localhost');
  const path = resolve(root, '.' + (url.pathname === '/' ? '/index.html' : url.pathname));
  if (!path.startsWith(root + sep)) return response.writeHead(403).end();
  try { response.writeHead(200, { 'content-type': mime[extname(path)] ?? 'application/octet-stream' }).end(await readFile(path)); }
  catch { response.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const origin = `http://127.0.0.1:${server.address().port}`;
const mp3 = await readFile('tests/fixtures/yasser-continuous-112.mp3');
const timings = [{ayah:1,start_time:63,end_time:2310},{ayah:2,start_time:2310,end_time:4515},{ayah:3,start_time:4515,end_time:7287},{ayah:4,start_time:7287,end_time:10668}];
const browser = await chromium.launch({ channel:'msedge', headless:true, args:['--autoplay-policy=no-user-gesture-required'] });
const report = { native, keyboard:{}, previews:[], playback:{}, layouts:[] }, contexts = [];
async function reader({ pageNumber = 45, contextual = false, reducedMotion = false } = {}) {
  const context = await browser.newContext({ viewport:{width:1920,height:1080}, reducedMotion:reducedMotion?'reduce':'no-preference' });
  contexts.push(context);
  const page = await context.newPage(), errors = [], requests = [], remote = { catalog:0, audio:[] };
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => requests.push(request.url()));
  page.setDefaultTimeout(15000);
  await page.addInitScript(({pageNumber,contextual,native}) => {
    if(native) { window.__QURAN_RUST_HOST__=true; window.chrome??={}; window.chrome.webview={postMessage(){},addEventListener(){},removeEventListener(){}}; }
    localStorage.setItem('quran-reader-onboarding-v2','done');
    if(!localStorage.getItem('quran-mushaf-qul-reader:v1')) localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:pageNumber,mode:'spread',theme:'dark',metadataLanguage:'english',contextualText:contextual}));
    if(!localStorage.getItem('quran-listening-preferences-v1')) localStorage.setItem('quran-listening-preferences-v1',JSON.stringify({mode:'ayah',ayahReciterId:1001,reciterId:0,recordingId:0,speed:1,volume:.8,continuous:false}));
  },{pageNumber,contextual,native});
  // More-specific routes are registered after this live-network block.
  await page.route('https://**/*', route => route.abort());
  await page.route('**/api/v3/reciters?**',route=>{
    remote.catalog++;
    return route.fulfill({json:{reciters:[{id:9001,name:'QA additional reciter',moshaf:[{id:9002,name:'QA recording',server:'https://server8.mp3quran.net/afs/',surah_list:'112'}]}]}});
  });
  await page.route('**/ayat_timing?**',route=>route.fulfill({json:timings}));
  await page.route('**/*.mp3',route=>{
    remote.audio.push(route.request().url());
    const range=route.request().headers().range?.match(/^bytes=(\d+)-(\d*)$/);
    const start=range?Number(range[1]):0,end=range?.[2]?Math.min(Number(range[2]),mp3.length-1):mp3.length-1;
    const body=mp3.subarray(start,end+1);
    return route.fulfill({status:range?206:200,headers:{'content-type':'audio/mpeg','content-length':String(body.length),'accept-ranges':'bytes',...(range?{'content-range':`bytes ${start}-${end}/${mp3.length}`}:{})},body});
  });
  await page.goto(origin);
  await page.locator(`.readerViewport [data-page-number="${pageNumber}"] .ayahRun`).first().waitFor();
  if(contextual) await page.locator('.readerViewport .shapedInkCanvas').first().waitFor();
  await page.waitForFunction(()=>document.querySelector('.readerViewport .ayahRun[tabindex="0"]'));
  return {page,context,errors,requests,remote};
}
const noSearch=requests=>assert.equal(requests.filter(url=>url.includes('/ayah-search-index.json')).length,0,'follow-along/study lookup must not load the full text-search index');
const pagesBounds=page=>page.locator('.readerViewport .pageSlot > .qulPage').evaluateAll(pages=>pages.map(page=>{
  const r=page.getBoundingClientRect();return {page:Number(page.dataset.pageNumber),x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom};
}));
async function openListen(page) {
  if(!native) {const access=page.locator('.sideToolsAccess');if(await access.count()&&await access.getAttribute('aria-expanded')==='false')await access.click();}
  await page.getByRole('button',{name:'Listen',exact:true}).click();await page.locator('.readingToolsPanel').waitFor();
}
async function closeListen(page) {
  await page.locator('.readingToolsPanel .tafsirPanelHeader').getByRole('button',{name:'Close',exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('.readingToolsPanel'));await page.waitForTimeout(350);
}
async function authenticQuote(surah,ayah) {
  // Read, never generate, the exact source words used by this layout.
  const map=JSON.parse(await readFile('src/data/ayahFirstPages.json','utf8'));
  const source=JSON.parse(await readFile(`public-qul/experimental/digital-khatt/pages/p${map[surah-1][ayah-1]}.json`,'utf8'));
  return source.lines.flatMap(line=>line.words).filter(word=>word.surah===surah&&word.ayah===ayah&&!/^\u06DD[\d٠-٩]+$/u.test(word.text.trim())).map(word=>word.text).join(' ');
}
try {
  const normal=await reader({contextual:true}),page=normal.page;
  await page.waitForFunction(()=>document.querySelectorAll('.pageTurnPreviews > [data-preview-page]').length===4);
  await page.waitForFunction(()=>document.querySelectorAll('.pageTurnPreviews .shapedInkCanvas').length>=60);
  report.previews.push(await page.evaluate(()=>({reducedMotion:false,pages:document.querySelectorAll('.pageTurnPreviews > [data-preview-page]').length,canvasBytes:[...document.querySelectorAll('.pageTurnPreviews canvas')].reduce((sum,c)=>sum+c.width*c.height*4,0)})));
  const initial=await page.locator('.readerViewport .ayahRun[tabindex="0"]').getAttribute('data-ayah-key');
  assert.equal(initial,'2:265','right page comes first despite left-page DOM order');
  assert.equal(await page.locator('.readerViewport .ayahRun[tabindex="0"]').count(),1);
  await page.locator('.readerViewport .ayahRun[tabindex="0"]').focus();await page.keyboard.press('ArrowDown');
  assert.equal(await page.evaluate(()=>document.activeElement.dataset.ayahKey),'2:266');
  await page.keyboard.press('Enter');await page.locator('.ayahActionsMenu').waitFor();
  assert.match(await page.locator('.ayahActionsMenu header strong').textContent(),/2:266/);
  await page.keyboard.press('Escape');
  await page.waitForFunction(()=>!document.querySelector('.ayahActionsMenu')&&document.activeElement?.dataset.ayahKey==='2:266');
  await page.keyboard.press('Shift+F10');await page.locator('.ayahActionsMenu').waitFor();
  await page.waitForFunction(()=>document.querySelector('.ayahActionsMenu')?.contains(document.activeElement));
  await page.locator('.ayahActionsMenu').getByRole('button',{name:/^Tafsir/}).click();
  await page.waitForFunction(expected=>document.querySelector('.studyVerse p')?.textContent===expected,await authenticQuote(2,266));
  await page.getByRole('button',{name:'Next ayah',exact:true}).click();
  await page.waitForFunction(expected=>document.querySelector('.studyVerse p')?.textContent===expected,await authenticQuote(2,267));
  noSearch(normal.requests);await page.screenshot({path:resolve(output,'keyboard-ayah-study.png')});
  report.keyboard={initial,arrowDown:'2:266',enterAndShiftF10:true,focusRestored:true,authenticStudyQuotes:['2:266','2:267'],fullSearchIndexRequests:0};
  assert.deepEqual(normal.errors,[]);await normal.context.close();

  const reduced=await reader({contextual:true,reducedMotion:true});
  assert.equal(await reduced.page.locator('.pageTurnPreviews [data-preview-page]').count(),0);
  assert.equal(await reduced.page.locator('.pageTurnPreviews canvas').count(),0);
  assert.ok(await reduced.page.locator('.readerViewport .shapedInkCanvas').count()>0,'visible contextual artwork remains');
  await reduced.page.keyboard.press('ArrowLeft');await reduced.page.locator('.readerViewport [data-page-number="47"]').waitFor();
  assert.equal(await reduced.page.locator('.bookFoldOverlay').count(),0);
  report.previews.push({reducedMotion:true,pages:0,canvasBytes:0,navigationWorks:true});
  assert.deepEqual(reduced.errors,[]);await reduced.context.close();

  const listening=await reader({pageNumber:604}),audioPage=listening.page;
  const sizes=[{width:1920,height:1080},{width:1280,height:900},{width:720,height:900}],baseline=[];
  for(const size of sizes){await audioPage.setViewportSize(size);await audioPage.waitForTimeout(600);baseline.push(await pagesBounds(audioPage));}
  await audioPage.setViewportSize(sizes[0]);await audioPage.waitForTimeout(350);
  assert.equal(listening.remote.audio.length,0,'opening the reader alone does not fetch audio');
  await openListen(audioPage);
  assert.equal(await audioPage.getByRole('combobox',{name:'Audio source',exact:true}).count(),0,'recording-engine choice is internal');
  assert.equal(await audioPage.locator('.moreReciters').getAttribute('open'),null);
  assert.equal(listening.remote.catalog,0,'additional catalog stays lazy until requested');
  await audioPage.getByRole('button',{name:'Play',exact:true}).click();
  await audioPage.waitForFunction(()=>{const a=document.querySelector('audio');return !a.paused&&a.currentTime>.2;});
  const source=await audioPage.locator('audio').getAttribute('src');
  assert.ok(source.endsWith('/yasser-dosari/r1/112.mp3'),'supported reciter automatically uses its matched continuous recording');
  const beforeDisclosure=await audioPage.locator('audio').evaluate(a=>a.currentTime);
  await audioPage.locator('.moreReciters summary').click();await audioPage.locator('#reciter-search').waitFor();
  await audioPage.waitForFunction(()=>document.querySelector('#reciter-search')?.value==='QA additional reciter');
  assert.equal(await audioPage.locator('audio').getAttribute('src'),source,'disclosure alone preserves source');
  assert.equal(await audioPage.locator('audio').evaluate(a=>a.paused),false);
  assert.ok(await audioPage.locator('audio').evaluate(a=>a.currentTime)>=beforeDisclosure,'disclosure does not restart playback');
  await audioPage.locator('.moreReciters summary').click();assert.equal(await audioPage.locator('audio').getAttribute('src'),source);
  await audioPage.getByRole('button',{name:'Pause',exact:true}).click();
  await audioPage.getByRole('combobox',{name:'Playback speed',exact:true}).selectOption('1.25');
  await audioPage.getByRole('slider',{name:'Volume',exact:true}).fill('0.35');
  const beforeContinuation=await audioPage.locator('audio').evaluate(a=>a.currentTime);
  await audioPage.getByRole('checkbox',{name:'Continue to next surah',exact:true}).check();
  assert.equal(await audioPage.locator('audio').getAttribute('src'),source);assert.equal(await audioPage.locator('audio').evaluate(a=>a.currentTime),beforeContinuation);
  await audioPage.getByRole('checkbox',{name:'Continue to next surah',exact:true}).uncheck();await closeListen(audioPage);
  for(let index=0;index<sizes.length;index++){
    const size=sizes[index];await audioPage.setViewportSize(size);await audioPage.waitForTimeout(650);
    const actual=await pagesBounds(audioPage);assert.equal(actual.length,baseline[index].length);
    for(let n=0;n<actual.length;n++){
      assert.equal(actual[n].page,baseline[index][n].page);
      assert.ok(Math.abs(actual[n].width-baseline[index][n].width)<1&&Math.abs(actual[n].height-baseline[index][n].height)<1,`player must not shrink pages at ${size.width}: ${JSON.stringify({baseline:baseline[index],actual})}`);
    }
    const panel=audioPage.locator('.sideRecitationPlayer');let placement='compact';
    if(await panel.count()){
      placement='side';const player=await panel.boundingBox();
      assert.ok(player.x>=0&&player.y>=0&&player.x+player.width<=size.width&&player.y+player.height<=size.height,'side player stays within viewport');
      for(const quran of actual)assert.ok(player.x+player.width<=quran.x+1||player.x>=quran.right-1||player.y+player.height<=quran.y+1||player.y>=quran.bottom-1,`automatic player must not cover Quran pages: ${JSON.stringify({player,quran})}`);
    }else assert.equal(await audioPage.locator('.sidePlayerCompact').count(),1,'narrow layout retains compact controls');
    report.layouts.push({...size,placement,pageWidths:actual.map(p=>p.width),pageHeights:actual.map(p=>p.height)});
    await audioPage.screenshot({path:resolve(output,`side-player-${native?'native':'browser'}-${size.width}.png`)});
  }
  await audioPage.setViewportSize(sizes[0]);await audioPage.waitForTimeout(350);
  // Resume from a different reading page: follow-along must locate the verse
  // using the compact map, without opening/downloading full text search.
  await audioPage.locator('main.appShell').focus();await audioPage.keyboard.press('ArrowRight');
  await audioPage.locator('.readerViewport [data-page-number="601"]').waitFor();
  await audioPage.locator('.sideRecitationPlayer, .sidePlayerCompact').getByRole('button',{name:'Resume',exact:true}).click();
  await audioPage.locator('.readerViewport [data-page-number="604"]').waitFor();
  await audioPage.waitForFunction(()=>document.querySelector('.readerViewport .ayahRun-reciting'));
  await audioPage.locator('.sideRecitationPlayer, .sidePlayerCompact').getByRole('button',{name:'Pause',exact:true}).click();
  noSearch(listening.requests);
  const persisted=await audioPage.evaluate(()=>({preferences:JSON.parse(localStorage.getItem('quran-listening-preferences-v1')),resume:JSON.parse(localStorage.getItem('quran-listening-resume-v1'))}));
  assert.equal(persisted.preferences.speed,1.25);assert.equal(persisted.preferences.volume,.35);assert.equal(persisted.preferences.ayahReciterId,1001);assert.ok(persisted.resume.seconds>0);
  await audioPage.reload();await audioPage.locator('.readerViewport .ayahRun').first().waitFor();await openListen(audioPage);
  assert.equal(await audioPage.getByRole('combobox',{name:'Playback speed',exact:true}).inputValue(),'1.25');
  assert.equal(await audioPage.getByRole('slider',{name:'Volume',exact:true}).inputValue(),'0.35');
  await audioPage.getByRole('button',{name:'Resume last recitation',exact:true}).click();
  await audioPage.waitForFunction(position=>{const a=document.querySelector('audio');return !a.paused&&a.currentTime>=position-.1;},persisted.resume.seconds);
  assert.equal(await audioPage.locator('audio').evaluate(a=>a.playbackRate),1.25);assert.equal(await audioPage.locator('audio').evaluate(a=>a.volume),.35);
  await audioPage.getByRole('button',{name:'Pause',exact:true}).click();noSearch(listening.requests);assert.deepEqual(listening.errors,[]);
  report.playback={continuousSourceByDefault:true,extraCatalogRequests:listening.remote.catalog,disclosurePreservesPlayback:true,continuationPreservesPosition:true,preferencePersistence:true,resumeAtSavedPosition:true,followAlongReturnsToAyahPage:true,fullSearchIndexRequests:0};
  await writeFile(resolve(output,`verification-${native?'native':'browser'}.json`),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
  console.log('Ayah keyboard actions, authentic study quotes, compact lookup, reduced-motion preview avoidance, side-player layout, extra-reciter disclosure, preferences and resume passed.');
}catch(error){
  for(const [index,context] of contexts.entries())for(const page of context.pages()){
    if(page.isClosed())continue;
    console.error('Failure diagnostic',await page.evaluate(()=>({active:document.activeElement?.outerHTML,menus:document.querySelectorAll('.ayahActionsMenu').length,roving:[...document.querySelectorAll('.readerViewport .ayahRun[tabindex="0"]')].map(run=>run.dataset.ayahKey)})).catch(()=>null));
    await page.screenshot({path:resolve(output,`failure-${index}.png`)}).catch(()=>{});
  }
  throw error;
}finally{await Promise.allSettled(contexts.map(c=>c.close()));await browser.close();await new Promise(done=>server.close(done));}
