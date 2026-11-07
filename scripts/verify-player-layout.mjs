import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

// Run against an already built dist. --baseline records the original bottom
// player only; it deliberately makes no assertions about candidate features.
const baseline = process.argv.includes('--baseline');
const root = resolve(process.argv.find(value => value.startsWith('--dist='))?.slice(7) || (baseline ? 'outputs/rust-windows/publish/WebApp' : 'dist'));
const output = resolve('outputs/player-layout-qa');
const runOutput = resolve(output, baseline ? 'baseline' : 'candidate');
const sizes = [{ width:1920,height:1080 },{ width:1280,height:900 },{ width:720,height:900 },{ width:390,height:844 }];
await mkdir(runOutput, { recursive:true });
const mime = { '.html':'text/html','.js':'text/javascript','.css':'text/css','.json':'application/json','.otf':'font/otf','.woff2':'font/woff2' };
const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://localhost');
  const path = resolve(root, '.' + (url.pathname === '/' ? '/index.html' : url.pathname));
  if (!path.startsWith(root + sep)) return response.writeHead(403).end();
  try { response.writeHead(200, { 'content-type':mime[extname(path)] || 'application/octet-stream' }).end(await readFile(path)); }
  catch { response.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const origin = `http://127.0.0.1:${server.address().port}`;
const mp3 = await readFile('tests/fixtures/yasser-continuous-112.mp3');
const timings = [{ayah:1,start_time:63,end_time:2310},{ayah:2,start_time:2310,end_time:4515},{ayah:3,start_time:4515,end_time:7287},{ayah:4,start_time:7287,end_time:10668}];
const browser = await chromium.launch({ channel:'msedge',headless:true,args:['--autoplay-policy=no-user-gesture-required'] });
const contexts = [], report = { baseline,dist:root,startedAt:new Date().toISOString(),note:'Edge fixture exercises the native host UI state; it does not measure the Win32 fullscreen window or hardware performance.',layouts:[],directions:[],playback:[],study:[],fullscreen:[],suspension:[],focus:[],escape:[],persistence:[],failedResponses:[],console:[],requests:[],failedRequests:[] };
let original;
if (!baseline) original = await readFile(resolve(output,'baseline.json'),'utf8').then(JSON.parse).catch(() => null);
const names = language => language === 'arabic'
  ? { listen:'استماع',search:'بحث',play:'تشغيل',pause:'إيقاف مؤقت',resume:'متابعة',close:'إغلاق',options:'الخيارات',settings:'الإعدادات',listeningSettings:'إعدادات الاستماع',closeSettings:'إغلاق الإعدادات',position:'موضع أدوات التلاوة',collapse:'تصغير أدوات التلاوة',reveal:'إظهار أدوات التلاوة',seek:'موضع التشغيل',speed:'سرعة التشغيل',closeStudy:'إغلاق التفسير' }
  : { listen:'Listen',search:'Search',play:'Play',pause:'Pause',resume:'Resume',close:'Close',options:'Options',settings:'Settings',listeningSettings:'Listening settings',closeSettings:'Close settings',position:'Player position',collapse:'Collapse player',reveal:'Show recitation controls',seek:'Playback position',speed:'Playback speed',closeStudy:'Close ayah study' };
const rect = box => ({ x:box.x,y:box.y,width:box.width,height:box.height,right:box.x+box.width,bottom:box.y+box.height });
const separate = (a,b) => a.right <= b.x+1 || b.right <= a.x+1 || a.bottom <= b.y+1 || b.bottom <= a.y+1;
const pagesBounds = page => page.locator('.readerViewport .pageSlot > .qulPage').evaluateAll(items => items.map(item => {
  const b=item.getBoundingClientRect();return { page:Number(item.dataset.pageNumber),x:b.x,y:b.y,right:b.right,bottom:b.bottom,width:b.width,height:b.height };
}));
const state = page => page.locator('audio').evaluate(a => ({ source:a.currentSrc||a.src,time:a.currentTime,speed:a.playbackRate,paused:a.paused,duration:a.duration }));
const reserves = page => page.locator('main.appShell').evaluate(el=>{const s=getComputedStyle(el);return {bottom:parseFloat(s.getPropertyValue('--recitation-reserve'))||0,side:parseFloat(s.getPropertyValue('--recitation-side-reserve'))||0};});
function samePages(actual,expected,message) {
  assert.equal(actual.length,expected.length,message);
  actual.forEach((item,index)=>{
    assert.equal(item.page,expected[index].page,message);
    for(const key of ['x','y','width','height']) assert.ok(Math.abs(item[key]-expected[index][key])<=1,`${message}: ${key} ${JSON.stringify({actual:item,expected:expected[index]})}`);
  });
}
function inWindow(box,size,message) { assert.ok(box.x>=-1&&box.y>=-1&&box.right<=size.width+1&&box.bottom<=size.height+1,`${message}: ${JSON.stringify({box,size})}`); }
async function hiddenPanel(page) {
  const panel=page.locator('#recitation-player-controls');
  assert.equal(await panel.evaluate(el=>el.inert),true,'hidden full controls are inert');
  assert.equal(await panel.getAttribute('aria-hidden'),'true','hidden full controls are excluded from accessibility navigation');
}
async function metadataCounter(page) {
  await page.locator('audio').evaluate(a=>{window.__placementAudio=a;window.__placementMetadataLoads=0;a.addEventListener('loadedmetadata',()=>window.__placementMetadataLoads++);});
}
async function unchangedAudio(page,before,message) {
  const after=await state(page);
  assert.equal(await page.evaluate(()=>document.querySelector('audio')===window.__placementAudio),true,`${message}: native audio element`);
  assert.equal(after.source,before.source,`${message}: media source`);
  assert.equal(after.speed,before.speed,`${message}: playback speed`);
  assert.equal(after.paused,before.paused,`${message}: transport state`);
  if(before.paused) assert.ok(Math.abs(after.time-before.time)<=.05,`${message}: paused position`);
  assert.equal(await page.evaluate(()=>window.__placementMetadataLoads),0,`${message}: no media reload`);
  return after;
}
async function settle(page) { await page.waitForTimeout(350); }
async function reader(language) {
  const context = await browser.newContext({ viewport:sizes[0],reducedMotion:'reduce' });contexts.push(context);
  const page=await context.newPage(), errors=[],blocked=[];page.setDefaultTimeout(30000);
  page.on('pageerror',error=>errors.push(error.message));
  page.on('console',message=>report.console.push({language,type:message.type(),text:message.text()}));
  page.on('request',request=>report.requests.push({language,url:request.url()}));
  page.on('requestfailed',request=>report.failedRequests.push({language,url:request.url(),error:request.failure()?.errorText}));
  page.on('response',response=>{if(response.status()>=400)report.failedResponses.push({language,status:response.status(),url:response.url()});});
  await page.addInitScript(({language}) => {
    const listeners=new Set();
    window.__playerHostMessage=data=>listeners.forEach(listener=>listener({data}));
    window.__playerHostRequests=[];
    window.__QURAN_RUST_HOST__=true;window.chrome??={};window.chrome.webview={
      postMessage(value){window.__playerHostRequests.push(value);if(value?.type==='fullscreen-request')queueMicrotask(()=>window.__playerHostMessage(value.active?'native-fullscreen-on':'native-fullscreen-off'));},
      addEventListener(type,listener){if(type==='message')listeners.add(listener);},removeEventListener(type,listener){if(type==='message')listeners.delete(listener);}
    };
    localStorage.setItem('quran-reader-onboarding-v2','done');
    if(!localStorage.getItem('quran-mushaf-qul-reader:v1')) localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:604,mode:'spread',theme:'dark',metadataLanguage:language}));
    if(!localStorage.getItem('quran-listening-preferences-v1')) localStorage.setItem('quran-listening-preferences-v1',JSON.stringify({mode:'ayah',ayahReciterId:1001,reciterId:0,recordingId:0,speed:1.25,volume:.25,continuous:false}));
  },{language});
  await context.route('**/*',route=>{
    const url=route.request().url();
    if(url.startsWith(origin+'/')) return route.continue();
    if(url.includes('/ayat_timing?')) return route.fulfill({json:timings,headers:{'access-control-allow-origin':'*'}});
    if(url.includes('/api/v3/reciters?')) return route.fulfill({json:{reciters:[{id:9001,name:'QA additional reciter',moshaf:[{id:9002,name:'QA recording',server:'https://server8.mp3quran.net/afs/',surah_list:'112'}]}]},headers:{'access-control-allow-origin':'*'}});
    if(new URL(url).pathname.endsWith('.mp3')) {
      const range=route.request().headers().range?.match(/^bytes=(\d+)-(\d*)$/);
      const start=range?Number(range[1]):0,end=range?.[2]?Math.min(Number(range[2]),mp3.length-1):mp3.length-1,body=mp3.subarray(start,end+1);
      return route.fulfill({status:range?206:200,headers:{'content-type':'audio/mpeg','content-length':String(body.length),'accept-ranges':'bytes','access-control-allow-origin':'*',...(range?{'content-range':`bytes ${start}-${end}/${mp3.length}`}:{})},body});
    }
    blocked.push(url);return route.abort();
  });
  await page.goto(origin);await page.locator('.readerViewport [data-ayah-key="112:1"]').first().waitFor();await settle(page);
  return {context,page,errors,blocked};
}
async function startPaused(page,n) {
  await page.locator('.rustTitleBar').getByRole('button',{name:n.listen,exact:true}).click();
  const panel=page.locator('.readingToolsPanel');await panel.waitFor();
  await panel.getByRole('combobox',{name:n.speed,exact:true}).selectOption('1.25');
  await panel.getByRole('button',{name:n.play,exact:true}).click();
  await page.waitForFunction(()=>{const a=document.querySelector('audio');return a&&!a.paused&&a.currentTime>.1&&Number.isFinite(a.duration);});
  assert.match((await state(page)).source,/\/yasser-dosari\/r1\/112\.mp3$/,'use the authentic matched continuous fixture');
  await panel.getByRole('button',{name:n.pause,exact:true}).click();
  await panel.locator('.tafsirPanelHeader').getByRole('button',{name:n.close,exact:true}).click();
  await page.locator('.recitationDock').waitFor();await settle(page);
}
async function openSettings(page,n) {
  const button=page.getByRole('button',{name:n.settings,exact:true});
  const options=page.locator('.rustTitleBar').getByRole('button',{name:n.options,exact:true});
  if(await options.getAttribute('aria-expanded')!=='true') await options.click();
  await button.click();await page.locator('.settingsPanel').waitFor();
}
async function placement(page,n,value) {
  await openSettings(page,n);await page.getByRole('combobox',{name:n.position,exact:true}).selectOption(value);
  await page.getByRole('button',{name:n.closeSettings,exact:true}).click();
  await page.waitForFunction(value=>document.querySelector('.recitationDock')?.dataset.placement===value,value);await settle(page);
}
async function expand(page,n) {
  const dock=page.locator('.recitationDock');
  if(await dock.getAttribute('data-expanded')!=='true') await page.getByRole('button',{name:n.reveal,exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.recitationDock')?.dataset.expanded==='true');await settle(page);
}
async function collapse(page,n) {
  await page.locator('.recitationDock').getByRole('button',{name:n.collapse,exact:true}).click();await page.mouse.move(250,45);
  await page.waitForFunction(()=>document.querySelector('.recitationDock')?.dataset.expanded==='false');await settle(page);
  await hiddenPanel(page);
}
async function fullscreen(page,active) {
  await page.evaluate(active=>window.__playerHostMessage(active?'native-fullscreen-on':'native-fullscreen-off'),active);
  await page.waitForFunction(active=>document.querySelector('main.appShell')?.classList.contains('fullscreenReader')===active,active);await settle(page);
}
async function openStudy(page) {
  await page.locator('.readerViewport [data-ayah-key="112:1"]').first().click({button:'right'});
  await page.locator('.ayahActionsMenu .ayahPrimaryAction').first().click();await page.locator('.ayahStudyPanel').waitFor();await settle(page);
}
async function closeStudy(page,n) {
  await page.locator('.ayahStudyPanel').getByRole('button',{name:n.closeStudy,exact:true}).click();await page.locator('.ayahStudyPanel').waitFor({state:'hidden'});await settle(page);
}
async function sideLayout(page,size,expectedPages,message) {
  const dock=page.locator('.recitationDock'),layout=await dock.getAttribute('data-layout'),pages=await pagesBounds(page);
  assert.deepEqual(await reserves(page),{bottom:0,side:0},`${message}: no reading-space reservation`);
  samePages(pages,expectedPages,`${message}: Quran layout is unchanged`);
  assert.ok(['card','compact'].includes(layout),`${message}: initial side controls are not an unsolicited popover`);
  const control=layout==='card'?page.locator('#recitation-player-controls'):page.locator('.sidePlayerCompact');
  const player=rect(await control.boundingBox());inWindow(player,size,message);
  for(const quran of pages) assert.ok(separate(player,quran),`${message}: normal side controls never cover Quran ${JSON.stringify({layout,player,quran})}`);
  if(layout==='card') {
    assert.ok(player.width<=182,'side card restores the compact 180px design');
    assert.ok(player.bottom>=size.height-40,'side card stays near the bottom corner rather than floating beside the middle of the page');
    assert.ok(player.height<=200,'side card avoids a tall centered tool panel');
  } else {
    await hiddenPanel(page);
    assert.ok(player.height<=50,'compact transport fits the toolbar');
    assert.ok(await control.getByRole('button',{name:/^(Resume|Pause|متابعة|إيقاف مؤقت)$/,exact:true}).isVisible(),'compact transport retains direct playback access');
  }
  return {layout,player,pages,reserves:await reserves(page)};
}
async function compactPopover(page,n,size) {
  const dock=page.locator('.recitationDock');
  assert.equal(await dock.getAttribute('data-layout'),'compact');
  await page.locator('.sidePlayerCompact').getByRole('button',{name:n.reveal,exact:true}).click();await settle(page);
  assert.equal(await dock.getAttribute('data-layout'),'popover','full controls open only after an explicit compact-toolbar request');
  assert.equal(await page.locator('#recitation-player-controls').evaluate(el=>el.inert),false);
  const box=rect(await page.locator('#recitation-player-controls').boundingBox());inWindow(box,size,'requested full controls');
  assert.ok(box.bottom>=size.height-40,'requested side controls open near the bottom corner');
  const elapsed=rect(await page.locator('#recitation-player-controls .miniElapsed').boundingBox());
  const total=rect(await page.locator('#recitation-player-controls .miniDuration').boundingBox());
  assert.ok(separate(elapsed,total),'timeline labels remain separated');
  assert.ok(n.listen==='استماع'?elapsed.x>total.x:elapsed.x<total.x,'elapsed label follows the interface timeline direction');
  await page.locator('#recitation-player-controls').getByRole('slider',{name:n.seek,exact:true}).focus();await page.keyboard.press('Escape');await settle(page);
  assert.equal(await dock.getAttribute('data-layout'),'compact','Escape dismisses the popover before closing other reader UI');await hiddenPanel(page);
  assert.equal(await page.locator('.sidePlayerCompact').getByRole('button',{name:n.reveal,exact:true}).evaluate(el=>el===document.activeElement),true,'dismissed popover returns keyboard focus to its opener');
  await page.locator('.sidePlayerCompact').getByRole('button',{name:n.reveal,exact:true}).click();await settle(page);
  assert.equal(await dock.getAttribute('data-layout'),'popover');
  await page.mouse.click(1,Math.floor(size.height/2));await settle(page);
  assert.equal(await dock.getAttribute('data-layout'),'compact','clicking outside dismisses requested full controls');await hiddenPanel(page);
  if(await page.locator('main.appShell').evaluate(el=>el.classList.contains('fullscreenReader'))) {
    const exitRequests=await page.evaluate(()=>window.__playerHostRequests.filter(value=>value?.type==='fullscreen-request'&&value.active===false).length);
    await page.locator('.sidePlayerCompact').getByRole('button',{name:n.reveal,exact:true}).click();await settle(page);
    assert.equal(await dock.getAttribute('data-layout'),'popover');
    // Keyboard focus can leave the popover without a pointerdown. Escape must
    // still dismiss that explicit overlay before requesting fullscreen exit.
    await page.locator('.fullscreenAccess > button').first().focus();await page.keyboard.press('Escape');await settle(page);
    assert.equal(await dock.getAttribute('data-layout'),'compact','toolbar-focused Escape dismisses the playback popover');await hiddenPanel(page);
    assert.equal(await page.locator('main.appShell').evaluate(el=>el.classList.contains('fullscreenReader')),true,'popover Escape keeps fullscreen active');
    assert.equal(await page.evaluate(()=>window.__playerHostRequests.filter(value=>value?.type==='fullscreen-request'&&value.active===false).length),exitRequests,'popover Escape sends no native fullscreen-exit request');
    report.escape.push({language:await page.locator('main.appShell').getAttribute('data-interface-language'),width:size.width,toolbarFocusedPopoverDismissed:true,fullscreenPreserved:true,exitRequests:0});
  }
  return box;
}
async function direction(page,n,language,position) {
  const dock=page.locator('.recitationDock'),input=dock.getByRole('slider',{name:n.seek,exact:true}),arabic=language==='arabic';
  assert.equal(await input.getAttribute('dir'),arabic?'rtl':'ltr');
  const box=await input.boundingBox(), y=box.y+box.height/2;
  await page.mouse.click(box.x+box.width*.2,y);await page.waitForFunction(()=>!document.querySelector('audio').seeking);await page.waitForTimeout(120);const left=await state(page);
  await page.mouse.click(box.x+box.width*.8,y);await page.waitForFunction(()=>!document.querySelector('audio').seeking);await page.waitForTimeout(120);const right=await state(page);
  assert.ok(arabic?left.time>right.time:right.time>left.time,`physical timeline direction ${language}/${position}: ${JSON.stringify({left,right})}`);
  await input.fill('5');await input.focus();await page.waitForTimeout(120);const before=Number(await input.inputValue());
  await input.press('ArrowLeft');await page.waitForTimeout(150);const afterLeft=Number(await input.inputValue());
  await input.press('ArrowRight');await page.waitForTimeout(150);const afterRight=Number(await input.inputValue());
  assert.ok(arabic?afterLeft>before:afterLeft<before,'ArrowLeft follows the physical direction of time');
  assert.ok(arabic?afterRight<afterLeft:afterRight>afterLeft,'ArrowRight reverses the previous step');
  const endpoints=await dock.locator('.miniTimeline').evaluate(el=>{
    const a=el.querySelector('.miniElapsed').getBoundingClientRect(),b=el.querySelector('.miniDuration').getBoundingClientRect();
    return {elapsed:(a.left+a.right)/2,duration:(b.left+b.right)/2,elapsedDirection:getComputedStyle(el.querySelector('.miniElapsed')).direction};
  });
  assert.ok(arabic?endpoints.elapsed>endpoints.duration:endpoints.elapsed<endpoints.duration,'elapsed timestamp follows interface direction');
  assert.equal(endpoints.elapsedDirection,'ltr');
  report.directions.push({language,position,leftTime:left.time,rightTime:right.time,before,afterLeft,afterRight,...endpoints});
}
try {
  for(const language of ['english','arabic']) {
    const fixture=await reader(language),{page}=fixture,n=names(language),idle=[],studyIdle=new Map(),fullscreenIdle=new Map();
    for(const size of sizes) { await page.setViewportSize(size);await settle(page);idle.push(await pagesBounds(page)); }
    if(!baseline) {
      for(const size of [sizes[1],sizes[3]]) {
        await page.setViewportSize(size);await settle(page);await openStudy(page);
        studyIdle.set(size.width,await pagesBounds(page));await closeStudy(page,n);
      }
      for(const size of [sizes[0],sizes[1]]) {
        await page.setViewportSize(size);await fullscreen(page,true);
        fullscreenIdle.set(size.width,await pagesBounds(page));
        const viewport=rect(await page.locator('.readerViewport').boundingBox());
        assert.ok(viewport.x<=1&&viewport.right>=size.width-1,'fullscreen does not reserve a permanent side rail');
        assert.ok(viewport.bottom>=size.height-1,'fullscreen does not reserve a permanent empty footer');
        await fullscreen(page,false);
      }
    }
    await page.setViewportSize(sizes[0]);await settle(page);await startPaused(page,n);
    for(const position of baseline?['bottom']:['bottom','side']) {
      if(!baseline) await placement(page,n,position);
      for(let index=0;index<sizes.length;index++) {
        const size=sizes[index];await page.setViewportSize(size);await settle(page);
        let entry;
        if(position==='side'&&!baseline) {
          entry={language,position,...size,...await sideLayout(page,size,idle[index],`${language}/${size.width}`)};
          if(entry.layout==='compact') { entry.requestedPopover=await compactPopover(page,n,size);samePages(await pagesBounds(page),idle[index],'opening and closing a popover never changes page size'); }
        } else {
          await expand(page,n);
          const player=rect(await page.locator('#recitation-player-controls').boundingBox()),pages=await pagesBounds(page),space=await reserves(page);
          if(!baseline) {
            inWindow(player,size,'bottom player');
            for(const quran of pages) assert.ok(separate(player,quran),`player must not cover Quran: ${JSON.stringify({language,position,size,player,quran})}`);
            assert.equal(space.side,0);assert.ok(player.height<=(size.width>760?80:120),'bottom player remains compact');
            const old=original?.layouts.find(item=>item.language===language&&item.width===size.width&&item.position==='bottom');
            if(old) assert.ok(player.height<=old.player.height+1,'bottom player does not regress the current compact height');
          }
          entry={language,position,...size,player,pages,reserves:space};
        }
        report.layouts.push(entry);
        await page.screenshot({path:resolve(runOutput,`${language}-${position}-${size.width}.png`)});
        if(!baseline&&position==='bottom') {
          await collapse(page,n);const compact=await page.locator('.recitationReveal').boundingBox();
          for(const quran of await pagesBounds(page)) assert.ok(separate(rect(compact),quran),'collapsed reveal must not cover Quran');
          await expand(page,n);
        }
      }
      if(!baseline) {
        await page.setViewportSize(sizes[0]);await settle(page);
        if(position==='side') assert.equal(await page.locator('.recitationDock').getAttribute('data-layout'),'card','a wide gutter restores the bottom-corner card after resizing');
        await expand(page,n);await direction(page,n,language,position);
        await metadataCounter(page);const before=await state(page);
        await page.evaluate(()=>window.__playerPanel=document.querySelector('#recitation-player-controls'));
        await openSettings(page,n);await settle(page);
        assert.equal(await page.locator('.recitationDock').getAttribute('data-suspended'),'true','settings suspend every player affordance');
        await hiddenPanel(page);assert.deepEqual(await reserves(page),{bottom:0,side:0},'settings do not leave player reservation behind');
        assert.equal(await page.locator('.sidePlayerCompact:visible').count(),0,'compact controls do not linger over settings');
        await unchangedAudio(page,before,'opening settings');
        await page.getByRole('button',{name:n.closeSettings,exact:true}).click();await settle(page);
        assert.equal(await page.evaluate(()=>document.querySelector('#recitation-player-controls')===window.__playerPanel),true,'settings keep the player component mounted');
        assert.notEqual(await page.locator('.recitationDock').getAttribute('data-suspended'),'true');
        await unchangedAudio(page,before,'closing settings');
        await page.locator('.rustTitleBar').getByRole('button',{name:n.search,exact:true}).click();await page.locator('.jumpPanel').waitFor();await settle(page);
        assert.equal(await page.locator('.recitationDock').getAttribute('data-suspended'),'true','search suspends the player');await hiddenPanel(page);
        assert.deepEqual(await reserves(page),{bottom:0,side:0},'search leaves no player reservation');
        await unchangedAudio(page,before,'opening search');
        await page.keyboard.press('Escape');await page.locator('.jumpPanel').waitFor({state:'hidden'});await settle(page);
        await unchangedAudio(page,before,'closing search');
        report.suspension.push({language,position,settings:true,search:true,preservesPlayerNode:true,preservesMedia:true});
        if(position==='side') {
          assert.equal(await page.locator('.recitationDock').getAttribute('data-layout'),'card');
          const listeningSettings=page.locator('#recitation-player-controls').getByRole('button',{name:n.listeningSettings,exact:true});
          await listeningSettings.click();await page.locator('.readingToolsPanel').waitFor();await settle(page);
          assert.equal(await page.locator('.recitationDock').getAttribute('data-suspended'),'true','opening listening settings suspends the preserved side card');
          await hiddenPanel(page);await unchangedAudio(page,before,'opening side-card listening settings');
          await page.keyboard.press('Escape');await page.locator('.readingToolsPanel').waitFor({state:'hidden'});await settle(page);
          assert.equal(await page.locator('.recitationDock').getAttribute('data-layout'),'card');
          assert.equal(await listeningSettings.evaluate(el=>el===document.activeElement&&!el.closest('[inert]')&&el.getClientRects().length>0&&getComputedStyle(el).visibility!=='hidden'),true,'closing listening settings returns keyboard focus to the revealed side-card button');
          await unchangedAudio(page,before,'closing side-card listening settings');
          report.focus.push({language,openedFrom:'side-card-listening-settings',escapeReturnsVisibleOpener:true,preservesMedia:true});
        }
      }
    }
    if(!baseline) {
      await page.setViewportSize(sizes[0]);await placement(page,n,'bottom');
      const dock=page.locator('.recitationDock');await dock.locator('.primaryPlayback').click();await page.locator('audio').evaluate(a=>a.currentTime=.3);
      await metadataCounter(page);
      const before=await state(page),started=Date.now();await placement(page,n,'side');const after=await state(page);
      assert.equal(after.source,before.source);assert.equal(after.paused,false);assert.equal(after.speed,1.25);
      assert.equal(await page.evaluate(()=>window.__placementMetadataLoads),0,'placement never reloads the media source');
      assert.ok(after.time>=before.time-.05&&after.time-before.time<=(Date.now()-started)/1000*before.speed+.75,'placement preserves the native timeline without a seek');
      await page.locator('audio').evaluate(a=>a.currentTime=.3);await page.mouse.move(250,45);await page.locator('main.appShell').focus();await page.waitForTimeout(3500);
      assert.equal(await dock.getAttribute('data-layout'),'card','the roomy side card remains available while playing');
      await placement(page,n,'bottom');await page.locator('audio').evaluate(a=>a.currentTime=.3);await page.mouse.move(250,45);await page.locator('main.appShell').focus();
      await page.waitForFunction(()=>document.querySelector('.recitationDock')?.dataset.expanded==='false',undefined,{timeout:15000});
      assert.equal((await state(page)).paused,false,'bottom auto-hide preserves playback');
      await dock.getByRole('button',{name:n.reveal,exact:true}).focus();
      await page.waitForFunction(()=>document.querySelector('.primaryPlayback')===document.activeElement);
      await dock.locator('.primaryPlayback').click();
      report.playback.push({language,source:after.source,speed:after.speed,switchPreservesPlayback:true,sideCardAvailable:true,bottomAutoHide:true,keyboardReveal:true});
      for(const position of ['bottom','side']) {
        await page.setViewportSize(sizes[0]);await placement(page,n,position);
        for(const size of [sizes[1],sizes[3]]) {
          await page.setViewportSize(size);await settle(page);const beforeStudy=await state(page);await metadataCounter(page);await openStudy(page);
          const actual=await sideLayout(page,size,studyIdle.get(size.width),`${language}/${position}/study/${size.width}`),study=rect(await page.locator('.ayahStudyPanel').boundingBox());
          assert.equal(actual.layout,'compact','tafsir retains only the compact toolbar transport for either preferred player position');
          assert.equal(await page.locator('.recitationDock').getAttribute('data-preference'),position,'tafsir compaction preserves the saved player preference');
          assert.ok(separate(actual.player,study),'compact player stays outside tafsir');
          await unchangedAudio(page,beforeStudy,'opening tafsir with a player');
          const requestedPopover=await compactPopover(page,n,size);
          await page.locator('.ayahStudyPanel').waitFor();await unchangedAudio(page,beforeStudy,'tafsir player popover');
          report.study.push({language,position,...size,...actual,study,requestedPopover,preservesPreference:true,preservesMedia:true});await page.screenshot({path:resolve(runOutput,`${language}-${position}-study-${size.width}.png`)});
          await closeStudy(page,n);await unchangedAudio(page,beforeStudy,'closing tafsir with a player');
          assert.equal(await page.locator('.recitationDock').getAttribute('data-placement'),position,'closing tafsir restores the preferred player layout');
        }
      }
      for(const size of [sizes[0],sizes[1]]) {
        await page.setViewportSize(size);await settle(page);const beforeFullscreen=await state(page);await metadataCounter(page);await fullscreen(page,true);
        const actual=await sideLayout(page,size,fullscreenIdle.get(size.width),`${language}/fullscreen/${size.width}`);
        if(actual.layout==='compact') {
          assert.ok(await page.locator('#fullscreen-listen-host .sidePlayerCompact').isVisible(),'fullscreen compact player shares the top navigation');
          await compactPopover(page,n,size);
        }
        const viewport=rect(await page.locator('.readerViewport').boundingBox());
        assert.ok(viewport.x<=1&&viewport.right>=size.width-1&&viewport.bottom>=size.height-1,'fullscreen player creates no permanent empty rail or footer');
        await unchangedAudio(page,beforeFullscreen,'entering fullscreen');
        report.fullscreen.push({language,...size,...actual,viewport});await page.screenshot({path:resolve(runOutput,`${language}-side-fullscreen-${size.width}.png`)});
        await fullscreen(page,false);await unchangedAudio(page,beforeFullscreen,'leaving fullscreen');
      }
      await page.setViewportSize(sizes[1]);await openSettings(page,n);
      await page.locator('.settingsPanel').getByRole('button',{name:language==='arabic'?'صفحة واحدة':'One page',exact:true}).click();
      await page.getByRole('button',{name:n.closeSettings,exact:true}).click();await page.locator('.readerViewport .singleSurface').waitFor();await settle(page);
      const singleLayout=await page.locator('.recitationDock').getAttribute('data-layout'),singlePlayer=rect(await page.locator(singleLayout==='card'?'#recitation-player-controls':'.sidePlayerCompact').boundingBox()),singlePages=await pagesBounds(page);
      assert.equal(singlePages.length,1,'single-page side layout contains one Quran page');
      assert.ok(separate(singlePlayer,singlePages[0]),'side player follows a replacement single-page surface');
      assert.deepEqual(await reserves(page),{bottom:0,side:0},'single-page side player reserves no reading space');
      report.layouts.push({language,position:'side',readerMode:'single',...sizes[1],layout:singleLayout,player:singlePlayer,pages:singlePages});
      await page.screenshot({path:resolve(runOutput,`${language}-side-single-1280.png`)});
      await page.setViewportSize(sizes[0]);await page.reload();await page.locator('.readerViewport .ayahRun').first().waitFor();await openSettings(page,n);
      assert.equal(await page.getByRole('combobox',{name:n.position,exact:true}).inputValue(),'side','placement survives reload');
      report.persistence.push({language,playerPlacement:'side',survivesReload:true});
    }
    assert.deepEqual(fixture.errors,[]);assert.deepEqual(fixture.blocked,[],'no unexpected external dependencies');await fixture.context.close();
  }
  await writeFile(resolve(output,baseline?'baseline.json':'verification.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify({baseline,layouts:report.layouts.length,directions:report.directions.length,study:report.study.length,output},null,2));
} catch(error) {
  report.failure=error.stack||String(error);await writeFile(resolve(output,baseline?'baseline-failure.json':'failure.json'),JSON.stringify(report,null,2));
  for(const [index,context] of contexts.entries()) for(const page of context.pages()) if(!page.isClosed()) {
    console.error('Player layout diagnostic',await page.evaluate(()=>({active:{tag:document.activeElement?.tagName,label:document.activeElement?.getAttribute('aria-label')},dock:document.querySelector('.recitationDock')?.outerHTML,settings:localStorage.getItem('quran-mushaf-qul-reader:v1'),fontStatus:document.fonts.status,fonts:[...document.fonts].map(f=>({family:f.family,status:f.status})),loadState:document.querySelector('.loadState')?.textContent,resources:performance.getEntriesByType('resource').map(r=>({name:r.name,duration:r.duration,bytes:r.transferSize}))})).catch(()=>null));
    await page.screenshot({path:resolve(runOutput,`failure-${index}.png`)}).catch(()=>{});
  }
  throw error;
} finally { await Promise.allSettled(contexts.map(context=>context.close()));await browser.close();await new Promise(done=>server.close(done)); }
