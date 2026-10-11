import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

const root = resolve('dist');
const output = resolve('outputs/rendering-qa');
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
 await page.addInitScript(()=>{localStorage.setItem('quran-reader-onboarding-v2','done');localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:604,mode:'spread',theme:'dark',metadataLanguage:'english'}));});
 const mp3=await readFile(resolve('tests/fixtures/yasser-continuous-112.mp3'));let requests=[];
 await page.route('**/ayat_timing?**',route=>route.fulfill({json:[{ayah:1,start_time:63,end_time:2310},{ayah:2,start_time:2310,end_time:4515},{ayah:3,start_time:4515,end_time:7287},{ayah:4,start_time:7287,end_time:10668}]}));
 await page.route('**/*.mp3',route=>{requests.push(route.request().url());return route.fulfill({status:200,headers:{'content-type':'audio/mpeg','content-length':String(mp3.length)},body:mp3});});
 await page.goto('http://127.0.0.1:'+server.address().port);const run=page.locator('.readerViewport [data-ayah-key="112:1"]').first();await run.waitFor();assert.equal(requests.length,0,'Reading should not download unused audio');
 await run.click({button:'right'});await page.getByRole('button',{name:'More listening options',exact:true}).click();
 const box=await page.locator('.ayahActionsMenu').boundingBox();assert.ok(box.x>=0&&box.y>=0&&box.x+box.width<=1920&&box.y+box.height<=1051);
 await page.setViewportSize({width:1024,height:600});
 await page.waitForFunction(()=>{const b=document.querySelector('.ayahActionsMenu')?.getBoundingClientRect();return b&&b.x>=0&&b.y>=0&&b.right<=innerWidth&&b.bottom<=innerHeight;});
 await page.keyboard.press('Tab');assert.ok(await page.locator('.ayahActionsMenu').evaluate(el=>el.contains(document.activeElement)));await page.keyboard.press('Escape');assert.equal(await page.locator('.ayahActionsMenu').count(),0);await page.setViewportSize({width:1920,height:1051});
 if (await page.locator('.sideToolsAccess').getAttribute('aria-expanded') === 'false') await page.locator('.sideToolsAccess').click();await page.getByRole('button',{name:'Listen',exact:true}).click();
 await page.locator('#ayah-reciter-search').fill('Yasser');await page.getByRole('option').filter({hasText:'Yasser'}).click();
 requests=[];await page.locator('.audioRecordingOptions summary').click();await page.getByRole('combobox',{name:'Audio source',exact:true}).selectOption('surah');
 await page.getByRole('combobox',{name:'Playback speed',exact:true}).selectOption('1.5');
 await page.getByRole('button',{name:'Play',exact:true}).click();await page.waitForFunction(()=>document.querySelector('audio').currentTime>.5);
 const source=await page.locator('audio').getAttribute('src');assert.ok(source.endsWith("/yasser-dosari/r1/112.mp3"));
 await page.waitForFunction(()=>document.querySelector('audio').currentTime>3);assert.equal(await page.locator('audio').evaluate(a=>a.playbackRate),1.5);
 assert.equal(await page.locator('audio').getAttribute('src'),source);assert.equal(new Set(requests.filter(url=>url.includes("yasser-dosari"))).size,1);
 await page.getByRole('button',{name:'Pause',exact:true}).click();const position=await page.locator('audio').evaluate(a=>a.currentTime);await page.waitForTimeout(400);assert.equal(await page.locator('audio').evaluate(a=>a.currentTime),position);
 await page.getByRole('button',{name:'Resume',exact:true}).click();await page.waitForFunction(()=>document.querySelector('audio').ended);assert.equal(await page.locator('audio').evaluate(a=>a.playbackRate),1.5);
 await page.getByRole('button',{name:'Download surah',exact:true}).click();await page.getByRole('button',{name:'Surah saved offline',exact:true}).waitFor();
 assert.equal(await page.evaluate(async()=>(await(await caches.open('quran-audio-v1')).keys()).filter(k=>k.url.includes('yasser-dosari')).length),1);
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
