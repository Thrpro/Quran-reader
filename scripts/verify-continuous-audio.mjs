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
 const mp3=await readFile(resolve('tests/fixtures/yasser-112001.mp3'));const requests=[];let failOnce=false;
 await page.route('**/*.mp3',async route=>{requests.push(route.request().url());if(failOnce){failOnce=false;return route.fulfill({status:503,body:'temporary outage'});}await route.fulfill({status:200,headers:{'content-type':'audio/mpeg','content-length':String(mp3.length),'access-control-allow-origin':'*'},body:mp3});});
 await page.goto('http://127.0.0.1:'+server.address().port);const run=page.locator('.readerViewport [data-ayah-key="112:1"]').first();await run.waitFor();
 await run.click({button:'right'});await page.screenshot({path:resolve(output,'right-click-0.4.21.png')});await page.getByRole('button',{name:'Listen from here',exact:false}).click();
 const audio=page.locator('audio');await page.waitForFunction(()=>document.querySelector('audio').currentTime>.2);
 assert.ok(requests[0].endsWith('/001001.mp3'),'basmala first');
 await page.getByRole('combobox',{name:'Playback speed',exact:true}).selectOption('1.5');
 await page.getByRole('slider',{name:'Volume',exact:true}).fill('0.35');
 const initialSrc=await audio.getAttribute('src');
 await page.waitForFunction(()=>document.querySelector('audio').currentTime>3);
 assert.equal(await audio.evaluate(a=>a.playbackRate),1.5);assert.equal(await audio.evaluate(a=>a.volume),.35);
 await page.getByRole('button',{name:'Pause',exact:true}).click();const paused=await audio.evaluate(a=>a.currentTime);await page.waitForTimeout(700);assert.equal(await audio.evaluate(a=>a.currentTime),paused);
 await page.getByRole('button',{name:'Resume',exact:true}).click();await page.waitForFunction(t=>document.querySelector('audio').currentTime>t+.3,paused);
 await page.waitForFunction(()=>document.querySelector('audio').currentTime>6);
 assert.equal(await audio.evaluate(a=>a.playbackRate),1.5);assert.equal(await audio.getAttribute('src'),initialSrc,'one source spans ayahs');
 await page.screenshot({path:resolve(output,'player-0.4.21.png')});
 await page.getByRole('button',{name:'Pause',exact:true}).click();await page.keyboard.press('Escape');await page.locator('.recitationMini').waitFor();assert.match(await page.locator('.recitationMini').textContent(),/Resume/);await page.locator('.recitationMini button').nth(1).click();
 await page.getByRole('button',{name:'Download surah',exact:true}).click();await page.getByRole('button',{name:'Surah saved offline',exact:true}).waitFor();
 assert.equal(await page.evaluate(async()=> (await(await caches.open('quran-audio-v1')).keys()).length),5,'four ayahs plus basmala');
 await page.getByRole('combobox',{name:'Surah',exact:true}).selectOption('2');await page.getByRole('checkbox',{name:'Continue to next surah',exact:true}).check();
 failOnce=true;await page.getByRole('button',{name:'Play',exact:true}).click();await page.getByRole('status').filter({hasText:'Retrying'}).waitFor();await page.waitForFunction(()=>document.querySelector('audio').currentTime>.2);
 await page.waitForFunction(()=>{const a=document.querySelector('audio');return a.buffered.length && a.buffered.end(a.buffered.length-1)>80;});
 const streamSrc=await audio.getAttribute('src');let previousEnd=0;
 for(let i=0;i<60;i++) {
   if(i===20) failOnce=true;
   const end=await audio.evaluate(a=>a.buffered.end(a.buffered.length-1));
   await audio.evaluate((a,end)=>{a.currentTime=end-10;},end);
   await page.waitForFunction(end=>{const a=document.querySelector('audio');return a.buffered.length && a.buffered.end(a.buffered.length-1)>end+30;},end);
   assert.equal(await audio.getAttribute('src'),streamSrc);assert.equal(await audio.evaluate(a=>a.playbackRate),1.5);previousEnd=end;
 }
 assert.ok(previousEnd>1200,'more than 20 minutes of media across buffer refills');
 assert.ok(await audio.evaluate(a=>a.buffered.start(0)>1000),'old media evicted');
 assert.ok(await audio.evaluate(a=>a.buffered.end(a.buffered.length-1)-a.buffered.start(0)<160),'bounded rolling buffer');
 await page.getByRole('button',{name:'Pause',exact:true}).click();
 assert.deepEqual(errors,[]);console.log({mediaTimelineSeconds:previousEnd,requests:requests.length});console.log('Continuous source, speed/volume, pause/resume, basmala, whole-surah cache, outage recovery, and rolling-buffer checks passed.');
} finally {await browser.close();await new Promise(done=>server.close(done));}
