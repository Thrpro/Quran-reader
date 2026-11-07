import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {resolve,extname,sep} from 'node:path';
import {chromium} from 'playwright';
const root=resolve('dist'),out=resolve('outputs/audio-cache-stream-qa');await mkdir(out,{recursive:true});
const fixture=await readFile('tests/fixtures/yasser-continuous-112.mp3'),key='https://cdn.mp3quran.net/audio/yasser-dosari/r1/112.mp3';
const mime={'.html':'text/html','.js':'application/javascript','.css':'text/css','.json':'application/json','.otf':'font/otf','.woff2':'font/woff2'};
const server=createServer(async(req,res)=>{const pathname=new URL(req.url,'http://localhost').pathname;if(pathname==='/__fixture.mp3')return res.writeHead(200,{'content-type':'audio/mpeg','content-length':String(fixture.length)}).end(fixture);const file=resolve(root,'.'+(pathname==='/'?'/index.html':pathname));if(!file.startsWith(root+sep))return res.writeHead(403).end();try{res.writeHead(200,{'content-type':mime[extname(file)]??'application/octet-stream'}).end(await readFile(file));}catch{res.writeHead(404).end();}});
await new Promise(done=>server.listen(0,'127.0.0.1',done));const origin='http://127.0.0.1:'+server.address().port;
const browser=await chromium.launch({channel:'msedge',headless:true});const context=await browser.newContext(),page=await context.newPage(),errors=[];
const report={checks:[],originalBytes:fixture.length,passed:false};page.on('pageerror',e=>errors.push(e.message));
try{
 await page.addInitScript(()=>{localStorage.setItem('quran-reader-onboarding-v2','done');localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:604,mode:'spread',theme:'dark',metadataLanguage:'english'}));localStorage.setItem('quran-listening-preferences-v1',JSON.stringify({mode:'ayah',ayahReciterId:1001,speed:1,volume:.1,continuous:false}));const create=URL.createObjectURL.bind(URL);window.__audioBlobCount=0;URL.createObjectURL=value=>{if(value instanceof Blob)window.__audioBlobCount++;return create(value);};});
 await context.route('**/*',route=>{const url=route.request().url();if(url.startsWith(origin+'/'))return route.continue();if(!url.includes('/ayat_timing?'))return route.abort();return route.fulfill({json:[{ayah:1,start_time:63,end_time:2310},{ayah:2,start_time:2310,end_time:4515},{ayah:3,start_time:4515,end_time:7287},{ayah:4,start_time:7287,end_time:10668}]});});
 await page.goto(origin);await page.locator('.readerViewport .ayahRun').first().waitFor();
 await page.evaluate(async key=>{const cache=await caches.open('quran-audio-v1'),response=await fetch('/__fixture.mp3');await cache.put(key,new Response(response.body,{headers:{'content-type':'audio/mpeg','content-length':response.headers.get('content-length')}}));},key);
 await page.getByRole('button',{name:'Listen',exact:true}).first().click();await page.locator('.readingToolsPanel').getByRole('button',{name:'Play',exact:true}).click();
 await page.waitForFunction(()=>{const a=document.querySelector('audio');return a.src.includes('/__offline_audio__/')&&!a.paused&&a.currentTime>.1;});
 assert.equal(await page.evaluate(()=>window.__audioBlobCount),0);
 report.checks.push('Existing cached recording plays through the real service worker without a recording Blob');
 const source=await page.locator('audio').getAttribute('src');
 await page.locator('audio').evaluate(a=>a.pause());await page.waitForTimeout(35000);
 await page.locator('audio').evaluate(a=>a.play());await page.waitForFunction(()=>!document.querySelector('audio').paused&&!document.querySelector('audio').error);
 report.checks.push('Playback resumes after a35-second pause across worker idle lifetime');
 await context.setOffline(true);
 for(const range of ['bytes=0-0','bytes=15-100','bytes=500-','bytes=-53']){
  const result=await page.evaluate(async({source,range})=>{const response=await fetch(source,{headers:{range}});return {status:response.status,contentRange:response.headers.get('content-range'),body:[...new Uint8Array(await response.arrayBuffer())]};},{source,range});
  const match=/bytes (\d+)-(\d+)\/(\d+)/.exec(result.contentRange);assert.equal(result.status,206);assert.equal(Number(match[3]),fixture.length);
  assert.deepEqual(Buffer.from(result.body),fixture.subarray(Number(match[1]),Number(match[2])+1));
 }
 const bad=await page.evaluate(async source=>(await fetch(source,{headers:{range:'bytes=999999999-'}})).status,source);assert.equal(bad,416);
 report.checks.push('Offline full/suffix/open-ended seeks return exact original MP3 bytes; invalid ranges return416');
 await page.locator('audio').evaluate(a=>{a.currentTime=7;});await page.waitForFunction(()=>document.querySelector('audio').currentTime>7.1&&!document.querySelector('audio').paused);
 await page.locator('.readingToolsPanel').getByRole('button',{name:'Stop recitation',exact:true}).click();
 await page.waitForFunction(()=>{const a=document.querySelector('audio');return !a.getAttribute('src')&&a.networkState===0;});
 await context.setOffline(false);
 report.checks.push('Native audio decode, offline seeking and explicit Stop work through streamed ranges');
 // No app shell/code cache: an ordinary reload fetches the current app again.
 await page.reload();await page.locator('.readerViewport .ayahRun').first().waitFor();
 const entries=await page.evaluate(()=>caches.keys());assert.deepEqual(entries,['quran-audio-v1']);assert.deepEqual(errors,[]);
 report.checks.push('Reload preserves the existing audio cache and the worker creates no app-code cache');report.passed=true;
 await writeFile(resolve(out,'verification.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
}catch(error){report.failure=error.stack;report.state=await page.evaluate(()=>{const a=document.querySelector('audio');return {source:a?.src,ready:a?.readyState,error:a?.error?.message,controller:navigator.serviceWorker.controller?.scriptURL};}).catch(()=>null);await writeFile(resolve(out,'verification.json'),JSON.stringify(report,null,2));await page.screenshot({path:resolve(out,'failure.png')}).catch(()=>{});throw error;}
finally{await context.close();await browser.close();await new Promise(done=>server.close(done));}
