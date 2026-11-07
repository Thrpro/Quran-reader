import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {createServer} from 'node:http';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {resolve,extname,sep} from 'node:path';
import {chromium} from 'playwright';

const option=(name,fallback)=>process.argv.find(value=>value.startsWith(`--${name}=`))?.slice(name.length+3)??fallback;
const root=resolve(option('root','dist')),output=resolve(option('output','outputs/audio-lifetime-qa'));
await mkdir(output,{recursive:true});
const mime={'.html':'text/html','.js':'text/javascript','.css':'text/css','.json':'application/json','.otf':'font/otf','.woff2':'font/woff2'};
const server=createServer(async(request,response)=>{
  const path=resolve(root,'.'+new URL(request.url,'http://localhost').pathname.replace(/^\/$/,'/index.html'));
  if(!path.startsWith(root+sep))return response.writeHead(403).end();
  try{response.writeHead(200,{'content-type':mime[extname(path)]??'application/octet-stream'}).end(await readFile(path));}catch{response.writeHead(404).end();}
});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
const origin=`http://127.0.0.1:${server.address().port}`;
const continuous=await readFile('tests/fixtures/yasser-continuous-112.mp3'),clip=await readFile('tests/fixtures/yasser-112001.mp3');
const timings=[{ayah:1,start_time:63,end_time:2310},{ayah:2,start_time:2310,end_time:4515},{ayah:3,start_time:4515,end_time:7287},{ayah:4,start_time:7287,end_time:10668}];
const timedUrl='https://cdn.mp3quran.net/audio/yasser-dosari/r1/112.mp3',timingUrl='https://www.mp3quran.net/api/v3/ayat_timing?surah=112&read=92';
const additionalUrl='https://server8.mp3quran.net/afs/112.mp3';
const clips=['001001','112001','112002','112003','112004'].map(value=>`https://verses.quran.foundation/Minshawy/Murattal/mp3/${value}.mp3`);
const index=await readFile(resolve(root,'index.html'));
const fingerprint={root,indexSha256:createHash('sha256').update(index).digest('hex'),assets:[]};
for(const match of index.toString().matchAll(/(?:src|href)="(\/assets\/[^\"]+\.(?:js|css))"/g)){
  const bytes=await readFile(resolve(root,'.'+match[1]));fingerprint.assets.push({path:match[1],sha256:createHash('sha256').update(bytes).digest('hex')});
}
const browser=await chromium.launch({channel:'msedge',headless:true,args:['--autoplay-policy=no-user-gesture-required']});
const report={startedAt:new Date().toISOString(),fingerprint,cases:[],selection:[],errors:[]},contexts=[];
const media=page=>page.locator('audio').evaluate(a=>({source:a.getAttribute('src'),current:a.currentSrc,time:a.currentTime,paused:a.paused,ready:a.readyState,network:a.networkState,duration:Number.isFinite(a.duration)?a.duration:null,buffered:a.buffered.length,speed:a.playbackRate,live:window.__liveAudioUrls.size}));
async function unloaded(page,message){
  // Edge can retain historical currentSrc text after a native empty load.
  // Assert actual ownership/decoder state, not that diagnostic URL string.
  try { await page.waitForFunction(()=>{const a=document.querySelector('audio');return !a.getAttribute('src')&&a.networkState===HTMLMediaElement.NETWORK_EMPTY&&a.readyState===0&&!Number.isFinite(a.duration)&&a.buffered.length===0&&window.__liveAudioUrls.size===0;}); }
  catch(error) { throw new Error(`${message}: ${JSON.stringify(await media(page))}`,{cause:error}); }
  const state=await media(page);assert.equal(state.paused,true,message);return state;
}
async function jumpReader(page,reference){
  await page.locator('.rustTitleBar').getByRole('button',{name:'Search',exact:true}).click();
  const search=page.locator('.jumpPanel .searchInput').first();await search.fill(reference);await search.press('Enter');
  await page.locator('.jumpPanel').waitFor({state:'hidden'});await page.locator(`.readerViewport [data-ayah-key="${reference}"]`).first().waitFor();
}
try{
  for(const mode of ['timed','clips','surah'])for(const saved of [false,true]){
    const context=await browser.newContext({viewport:{width:1280,height:900},reducedMotion:'reduce'});contexts.push(context);
    const page=await context.newPage(),requests=[],errors=[],unexpected=[];page.setDefaultTimeout(20000);
    page.on('pageerror',error=>errors.push(error.message));
    await page.addInitScript(({mode})=>{
      window.__QURAN_RUST_HOST__=true;window.chrome??={};window.chrome.webview={postMessage(){},addEventListener(){},removeEventListener(){}};
      localStorage.setItem('quran-reader-onboarding-v2','done');
      localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:604,mode:'spread',metadataLanguage:'english',theme:'dark'}));
      localStorage.setItem('quran-listening-preferences-v1',JSON.stringify({mode:mode==='surah'?'surah':'ayah',ayahReciterId:mode==='clips'?9:1001,reciterId:9001,recordingId:9002,speed:1.5,volume:.2,continuous:false}));
      const create=URL.createObjectURL.bind(URL),revoke=URL.revokeObjectURL.bind(URL);window.__liveAudioUrls=new Set();
      URL.createObjectURL=value=>{const url=create(value);window.__liveAudioUrls.add(url);return url;};
      URL.revokeObjectURL=url=>{window.__liveAudioUrls.delete(url);revoke(url);};
    },{mode});
    await context.route('**/*',async route=>{
      const url=route.request().url();
      if(url.startsWith(origin+'/'))return route.continue();
      if(url.includes('/api/v3/reciters?'))return route.fulfill({json:{reciters:[{id:9001,name:'QA additional reciter',moshaf:[{id:9002,name:'QA recording',server:'https://server8.mp3quran.net/afs/',surah_list:'112'}]}]}});
      if(url.includes('/ayat_timing?'))return route.fulfill({json:timings});
      if(new URL(url).pathname.endsWith('.mp3')){
        requests.push(url);const bytes=url.includes('/mp3/')?clip:continuous;
        const range=route.request().headers().range?.match(/^bytes=(\d+)-(\d*)$/),start=range?Number(range[1]):0,end=range?.[2]?Math.min(Number(range[2]),bytes.length-1):bytes.length-1,body=bytes.subarray(start,end+1);
        return route.fulfill({status:range?206:200,headers:{'content-type':'audio/mpeg','content-length':String(body.length),'accept-ranges':'bytes',...(range?{'content-range':`bytes ${start}-${end}/${bytes.length}`}:{})},body});
      }
      unexpected.push(url);return route.abort();
    });
    await page.goto(origin);await page.locator('.readerViewport [data-ayah-key="112:1"]').first().waitFor();
    if(saved)await page.evaluate(async({mode,continuous,clip,timings,timedUrl,timingUrl,additionalUrl,clips})=>{
      const cache=await caches.open('quran-audio-v1');
      const put=async(url,bytes,type='audio/mpeg')=>cache.put(url,new Response(new Uint8Array(bytes),{headers:{'content-type':type,'content-length':String(bytes.length)}}));
      if(mode==='clips')for(const url of clips)await put(url,clip);
      else await put(mode==='timed'?timedUrl:additionalUrl,continuous);
      if(mode==='timed'){const bytes=new TextEncoder().encode(JSON.stringify(timings));await put(timingUrl,bytes,'application/json');}
    },{mode,continuous:[...continuous],clip:[...clip],timings,timedUrl,timingUrl,additionalUrl,clips});
    await page.locator('.rustTitleBar').getByRole('button',{name:'Listen',exact:true}).click();const panel=page.locator('.readingToolsPanel');await panel.waitFor();
    if(mode==='timed'&&!saved){
      await panel.locator('.tafsirPanelHeader').getByRole('button',{name:'Close',exact:true}).click();await jumpReader(page,'2:1');
      await page.locator('.rustTitleBar').getByRole('button',{name:'Listen',exact:true}).click();await panel.waitFor();
      assert.equal(await panel.getByRole('combobox',{name:'Surah',exact:true}).inputValue(),'2','an unconfigured listener initially follows the reader chapter');
      assert.equal(await panel.getByRole('combobox',{name:'Ayah',exact:true}).inputValue(),'1');
      await panel.locator('.tafsirPanelHeader').getByRole('button',{name:'Close',exact:true}).click();await jumpReader(page,'112:1');
      await page.locator('.rustTitleBar').getByRole('button',{name:'Listen',exact:true}).click();await panel.waitFor();
      assert.equal(await panel.getByRole('combobox',{name:'Surah',exact:true}).inputValue(),'112','reader-following continues until playback is deliberately configured');
      report.selection.push({unconfiguredFollowsReader:true,noMediaPrepared:true});
    }
    if(mode==='surah')await page.waitForFunction(()=>document.querySelector('#reciter-search')?.value==='QA additional reciter');
    await panel.getByRole('combobox',{name:'Surah',exact:true}).selectOption('112');
    if(mode!=='surah'){
      await panel.locator('.listeningRangeEditor summary').click();
      await panel.getByRole('spinbutton',{name:'From ayah',exact:true}).fill('2');await panel.getByRole('spinbutton',{name:'To ayah',exact:true}).fill('3');
      await panel.getByRole('button',{name:'Use this range',exact:true}).click();
    }
    await page.waitForTimeout(350);
    assert.equal(requests.length,0,'opening or configuring Listen never downloads unused audio');
    const before=await unloaded(page,'settings own no media decoder');
    if(saved){await panel.getByRole('button',{name:'Surah saved offline',exact:true}).waitFor();assert.equal(await panel.getByRole('button',{name:'Surah saved offline',exact:true}).isDisabled(),true);}
    await panel.getByRole('button',{name:'Play',exact:true}).click();
    await page.waitForFunction(()=>{const a=document.querySelector('audio');return !a.paused&&a.currentTime>.1;});
    assert.equal((await media(page)).speed,1.5);
    if(mode==='timed')assert.ok((await media(page)).time>=2.31,'Play starts at the selected ayah range');
    if(saved)assert.equal(requests.length,0,'saved recordings play without remote MP3 requests');
    await panel.getByRole('button',{name:'Pause',exact:true}).click();const paused=await media(page);
    await page.waitForTimeout(200);assert.equal((await media(page)).time,paused.time,'Pause preserves the source and position');
    await panel.getByRole('button',{name:'Stop recitation',exact:true}).click();const stopped=await unloaded(page,'Stop releases every playback source');
    const requestCount=requests.length;await page.waitForTimeout(500);assert.equal(requests.length,requestCount,'Stop does not prepare another paused recording');
    await panel.getByRole('button',{name:'Play',exact:true}).click();await page.waitForFunction(()=>{const a=document.querySelector('audio');return !a.paused&&a.currentTime>.1;});
    assert.equal((await media(page)).speed,1.5);if(mode==='timed')assert.ok((await media(page)).time>=2.31,'Stop/Play retains the selected range');
    await panel.getByRole('button',{name:'Stop recitation',exact:true}).click();await unloaded(page,'second Stop releases the replacement source');
    if(saved){
      // Simulate a slow local read. Stop must cancel ownership before the
      // previous asynchronous Cache.match continuation resumes.
      await page.evaluate(()=>{const match=Cache.prototype.match;window.__restoreMatch=()=>{Cache.prototype.match=match;};Cache.prototype.match=async function(request,...rest){const response=await match.call(this,request,...rest);const url=typeof request==='string'?request:request.url;if(url.endsWith('.mp3'))await new Promise(done=>setTimeout(done,250));return response;};});
      await panel.getByRole('button',{name:'Play',exact:true}).click();await panel.getByRole('button',{name:'Stop recitation',exact:true}).click();await page.waitForTimeout(600);
      await unloaded(page,'a late cache response cannot revive a stopped recording');await page.evaluate(()=>window.__restoreMatch());
    }
    if(mode==='timed'&&!saved){
      // Decoder ownership and an explicit playback selection are independent.
      // Reopening an unloaded menu must never combine a reader chapter with
      // the previous chapter's ayah/range (for example 112:200).
      await panel.getByRole('combobox',{name:'Surah',exact:true}).selectOption('2');
      await panel.getByRole('combobox',{name:'Ayah',exact:true}).selectOption('200');
      if(!await panel.locator('.listeningRangeFields').isVisible())await panel.locator('.listeningRangeEditor summary').click();
      await panel.getByRole('spinbutton',{name:'From ayah',exact:true}).fill('200');await panel.getByRole('spinbutton',{name:'To ayah',exact:true}).fill('210');
      await panel.getByRole('button',{name:'Use this range',exact:true}).click();
      const assertSelection=async()=>{
        assert.equal(await panel.getByRole('combobox',{name:'Surah',exact:true}).inputValue(),'2');
        assert.equal(await panel.getByRole('combobox',{name:'Ayah',exact:true}).inputValue(),'200');
        assert.match(await panel.locator('.listeningRangeSummary').textContent(),/2:200.*2:210/);
        if(!await panel.locator('.listeningRangeFields').isVisible())await panel.locator('.listeningRangeEditor summary').click();
        assert.equal(await panel.getByRole('spinbutton',{name:'From ayah',exact:true}).inputValue(),'200');
        assert.equal(await panel.getByRole('spinbutton',{name:'To ayah',exact:true}).inputValue(),'210');
        await unloaded(page,'an unplayed/stopped selected range stays unloaded');
      };
      const beforeSelectionRequests=requests.length;
      await panel.locator('.tafsirPanelHeader').getByRole('button',{name:'Close',exact:true}).click();await jumpReader(page,'112:1');
      await page.locator('.rustTitleBar').getByRole('button',{name:'Listen',exact:true}).click();await panel.waitFor();await assertSelection();
      await panel.getByRole('button',{name:'Stop recitation',exact:true}).click();
      await panel.locator('.tafsirPanelHeader').getByRole('button',{name:'Close',exact:true}).click();await jumpReader(page,'3:1');
      await page.locator('.rustTitleBar').getByRole('button',{name:'Listen',exact:true}).click();await panel.waitFor();await assertSelection();
      assert.equal(requests.length,beforeSelectionRequests,'reader navigation and selected-range reopening do not prepare media');
      report.selection.push({unplayedRangePreserved:true,stopPreservesSelection:true,readerChapterChanged:true,surah:2,from:200,to:210});
    }
    assert.deepEqual(errors,[]);assert.deepEqual(unexpected,[]);report.cases.push({mode,saved,before,paused,stopped,networkRequests:requests.length,lateCacheStop:saved,noIdleSource:true,rangeRetained:mode!=='surah'});
    await context.close();
  }
  report.finishedAt=new Date().toISOString();await writeFile(resolve(output,'verification.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify({cases:report.cases.length,source:fingerprint.indexSha256,output},null,2));
}catch(error){report.failure=error.stack||String(error);await writeFile(resolve(output,'failure.json'),JSON.stringify(report,null,2));for(const[index,context]of contexts.entries())for(const page of context.pages())if(!page.isClosed())await page.screenshot({path:resolve(output,`failure-${index}.png`)}).catch(()=>{});throw error;}
finally{await Promise.allSettled(contexts.map(context=>context.close()));await browser.close();await new Promise(done=>server.close(done));}
