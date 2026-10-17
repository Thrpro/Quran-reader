import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

const root = resolve('dist');
const output = resolve('outputs/rendering-qa');
const native = process.argv.includes('--native');
await mkdir(output, { recursive: true });
const mime = { '.html':'text/html', '.js':'text/javascript', '.css':'text/css', '.json':'application/json', '.otf':'font/otf', '.woff2':'font/woff2' };
const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
  const path = resolve(root, `.${url.pathname === '/' ? '/index.html' : url.pathname}`);
  if (!path.startsWith(root + sep)) return res.writeHead(403).end();
  try { const content = await readFile(path); res.writeHead(200, { 'Content-Type':mime[extname(path)] || 'application/octet-stream' }).end(content); }
  catch { res.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const browser = await chromium.launch({ channel:'msedge', headless:true, args:['--autoplay-policy=no-user-gesture-required'] });
try {
 const page=await browser.newPage({viewport:{width:1920,height:1051}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
 if(native) await page.addInitScript(()=>{window.__QURAN_RUST_HOST__=true;window.chrome??={};window.chrome.webview={postMessage(){},addEventListener(){},removeEventListener(){}};});
 await page.addInitScript(()=>{localStorage.setItem('quran-reader-onboarding-v2','done');localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:604,mode:'spread',theme:'dark',metadataLanguage:'english'}));});
 const mp3=await readFile(resolve('tests/fixtures/yasser-continuous-112.mp3'));let requests=[];
 await page.route('**/ayat_timing?**',route=>route.fulfill({json:[{ayah:1,start_time:63,end_time:2310},{ayah:2,start_time:2310,end_time:4515},{ayah:3,start_time:4515,end_time:7287},{ayah:4,start_time:7287,end_time:10668}]}));
 await page.route('**/*.mp3',route=>{
   requests.push(route.request().url());
   const range=route.request().headers()['range']?.match(/^bytes=(\d+)-(\d*)$/);
   const start=range?Number(range[1]):0,end=range&&range[2]?Math.min(Number(range[2]),mp3.length-1):mp3.length-1;
   const body=mp3.subarray(start,end+1);
   return route.fulfill({status:range?206:200,headers:{'content-type':'audio/mpeg','content-length':String(body.length),'accept-ranges':'bytes',...(range?{'content-range':`bytes ${start}-${end}/${mp3.length}`}:{})},body});
 });
 await page.goto('http://127.0.0.1:'+server.address().port);const run=page.locator('.readerViewport [data-ayah-key="112:1"]').first();await run.waitFor();assert.equal(requests.length,0,'Reading should not download unused audio');
 const viewports=[{width:1920,height:1051},{width:1024,height:600},{width:390,height:844}];
 const readerBounds=()=>page.locator('.readerViewport .pageSlot > .qulPage').evaluateAll(elements=>elements.map(element=>{
   const b=element.getBoundingClientRect();return {left:b.left,top:b.top,right:b.right,bottom:b.bottom,width:b.width,height:b.height};
 }).filter(b=>b.width>0&&b.height>0).sort((a,b)=>a.left-b.left));
 const idleBounds=new Map();
 for(const size of viewports){await page.setViewportSize(size);await page.waitForTimeout(700);idleBounds.set(size.width,await readerBounds());}
 await page.setViewportSize(viewports[0]);await page.waitForTimeout(700);
 await run.click({button:'right'});await page.getByRole('button',{name:'More listening options',exact:true}).click();
 const box=await page.locator('.ayahActionsMenu').boundingBox();assert.ok(box.x>=0&&box.y>=0&&box.x+box.width<=1920&&box.y+box.height<=1051);
 await page.setViewportSize({width:1024,height:600});
 await page.waitForFunction(()=>{const b=document.querySelector('.ayahActionsMenu')?.getBoundingClientRect();return b&&b.x>=0&&b.y>=0&&b.right<=innerWidth&&b.bottom<=innerHeight;});
 await page.keyboard.press('Tab');assert.ok(await page.locator('.ayahActionsMenu').evaluate(el=>el.contains(document.activeElement)));await page.keyboard.press('Escape');assert.equal(await page.locator('.ayahActionsMenu').count(),0);await page.setViewportSize({width:1920,height:1051});
 if (await page.locator('.sideToolsAccess').count() && await page.locator('.sideToolsAccess').getAttribute('aria-expanded') === 'false') await page.locator('.sideToolsAccess').click();await page.getByRole('button',{name:'Listen',exact:true}).click();
 await page.locator('#ayah-reciter-search').fill('Yasser');await page.getByRole('option').filter({hasText:'Yasser'}).click();
 assert.equal(await page.getByRole('combobox',{name:'Audio source',exact:true}).count(),0,'the matched recording is selected automatically');
 assert.equal(await page.getByRole('button',{name:'Follow along',exact:true}).count(),0,'ordinary listeners do not choose a technical recording mode');
 assert.equal(await page.locator('.moreReciters').getAttribute('open'),null,'additional recordings stay under More reciters');
 await page.getByRole('combobox',{name:'Playback speed',exact:true}).selectOption('1.5');
 await page.getByRole('button',{name:'Play',exact:true}).click();await page.waitForFunction(()=>document.querySelector('audio').currentTime>.5);
 const source=await page.locator('audio').getAttribute('src');assert.ok(source.endsWith("/yasser-dosari/r1/112.mp3"));
 await page.waitForFunction(()=>document.querySelector('audio').currentTime>3);assert.equal(await page.locator('audio').evaluate(a=>a.playbackRate),1.5);
 assert.equal(await page.locator('audio').getAttribute('src'),source);assert.equal(new Set(requests.filter(url=>url.includes("yasser-dosari"))).size,1);
 await page.getByRole('button',{name:'Pause',exact:true}).click();const position=await page.locator('audio').evaluate(a=>a.currentTime);await page.waitForTimeout(400);assert.equal(await page.locator('audio').evaluate(a=>a.currentTime),position);
 await page.keyboard.press('Escape');
 const mini=page.locator('.recitationMini');
 await page.waitForTimeout(400);
 for(const size of viewports) {
   await page.setViewportSize(size);await page.waitForTimeout(650);
   const before=idleBounds.get(size.width),actual=await readerBounds();
   assert.equal(actual.length,before.length,'playback keeps the same visible pages');
   actual.forEach((bounds,index)=>Object.keys(bounds).forEach(key=>assert.ok(Math.abs(bounds[key]-before[index][key])<1.5,`playback does not resize or move page ${index} at ${size.width}: ${key}`)));
   let compact=false;
   if(!await mini.isVisible()) {
     compact=true;
     const trigger=page.locator('.sidePlayerCompact').getByRole('button',{name:/Show (?:recitation|playback) controls/});
     await trigger.waitFor();await trigger.click();await mini.waitFor();
   }
   const panel=await mini.boundingBox();
   assert.ok(panel.x>=0&&panel.y>=0&&panel.x+panel.width<=size.width+1&&panel.y+panel.height<=size.height+1,`player fits the window at ${size.width}`);
   if(!await mini.evaluate(element=>element.classList.contains('sidePlayerPopover'))) {
     assert.ok(actual.every(bounds=>panel.x+panel.width<=bounds.left+1||panel.x>=bounds.right-1||panel.y+panel.height<=bounds.top+1||panel.y>=bounds.bottom-1),'wide player fits beside the Quran pages');
   }
   const controls=await mini.locator('.miniRecording,.miniTransport,.miniTimeline,.miniActions').evaluateAll(elements=>elements.map(el=>{const b=el.getBoundingClientRect();return {top:b.top,bottom:b.bottom};}));
   const ordered=controls.sort((a,b)=>a.top-b.top);
   assert.ok(ordered.every((bounds,index)=>index===0||bounds.top>=ordered[index-1].bottom-1),'stacked player sections do not overlap');
   await page.screenshot({path:resolve(output,`side-player-${native?'native-':''}${size.width}.png`)});
   if(compact) {await mini.getByRole('button',{name:'Collapse player',exact:true}).click();await page.locator('.sidePlayerCompact').waitFor();}
 }
 await page.setViewportSize({width:1920,height:1051});
 await page.waitForTimeout(650);
 if(!await mini.isVisible()) await page.locator('.sidePlayerCompact').getByRole('button',{name:/Show (?:recitation|playback) controls/}).click();
 await page.locator('audio').evaluate(a=>a.currentTime=5);
 await mini.getByRole('button',{name:'Back 10 seconds',exact:true}).click();assert.ok(await page.locator('audio').evaluate(a=>a.currentTime)<.1);
 await page.waitForFunction(()=>!document.querySelector('audio').seeking);
 await mini.getByRole('button',{name:'Forward 10 seconds',exact:true}).click();
 await page.waitForFunction(()=>!document.querySelector('audio').seeking);
 const seekState=await page.locator('audio').evaluate(a=>({time:a.currentTime,duration:a.duration,paused:a.paused,max:document.querySelector('.miniTimeline input')?.max}));
 assert.ok(seekState.time>9.9,JSON.stringify(seekState));
 await page.locator('audio').evaluate((a,p)=>a.currentTime=p,position);
 await page.screenshot({path:resolve(output,native?'side-player-native-layout.png':'side-player-clearance.png')});
 await mini.getByRole('button',{name:'Listening settings',exact:true}).click();
 await page.getByRole('button',{name:'Resume',exact:true}).click();await page.waitForFunction(()=>document.querySelector('audio').ended);assert.equal(await page.locator('audio').evaluate(a=>a.playbackRate),1.5);
 await page.getByRole('button',{name:'Download surah',exact:true}).click();await page.getByRole('button',{name:'Surah saved offline',exact:true}).waitFor();
 assert.equal(await page.evaluate(async()=>(await(await caches.open('quran-audio-v1')).keys()).filter(k=>k.url.includes('yasser-dosari')&&new URL(k.url).pathname.endsWith('.mp3')).length),1);
 assert.equal(await page.evaluate(async()=>(await(await caches.open('quran-audio-v1')).keys()).filter(k=>k.url.includes('/ayat_timing?')&&new URL(k.url).searchParams.get('surah')==='112'&&new URL(k.url).searchParams.get('read')==='92').length),1,'offline recording includes validated ayah timings for this exact recording');
 await page.locator('.listeningRangeEditor summary').click();
 await page.getByRole('spinbutton',{name:'From ayah'}).fill('2');
 await page.getByRole('spinbutton',{name:'To ayah'}).fill('3');
 await page.getByRole('button',{name:'Use this range',exact:true}).click();
 await page.getByRole('checkbox',{name:'Repeat playback',exact:true}).check();
 await page.getByRole('button',{name:'Play',exact:true}).click();
 await page.waitForFunction(()=>{const a=document.querySelector('audio');return !a.paused&&a.currentTime>2.4;});
 await page.locator('audio').evaluate(a=>a.currentTime=7.25);
 try { await page.waitForFunction(()=>{const a=document.querySelector('audio');return !a.paused&&a.currentTime>2.4&&a.currentTime<3.5;}); }
 catch(error) { console.log(await page.locator('audio').evaluate(a=>({time:a.currentTime,paused:a.paused,ended:a.ended,error:a.error?.message,source:a.src,repeat:document.querySelector('input[aria-label="Repeat playback"]')?.checked,range:document.querySelector('.listeningRangeSummary')?.textContent,panel:document.querySelector('.recitationPlayer')?.textContent}))); throw error; }
 assert.equal(await page.locator('audio').evaluate(a=>a.playbackRate),1.5);
 await page.getByRole('checkbox',{name:'Repeat playback',exact:true}).uncheck();
 await page.locator('audio').evaluate(a=>a.currentTime=7.29);
 await page.waitForFunction(()=>document.querySelector('audio').paused);
 await page.screenshot({path:resolve(output,'timed-continuous-0.4.25.png')});assert.deepEqual(errors,[]);
 console.log('Verified continuous publisher recording, ayah timings, 1.5x speed, pause/resume, whole-surah download and menu focus.');
} finally {await browser.close();await new Promise(done=>server.close(done));}
