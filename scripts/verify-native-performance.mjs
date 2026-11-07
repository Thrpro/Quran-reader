import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {execFile,spawn} from 'node:child_process';
import {promisify} from 'node:util';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {chromium} from 'playwright';
import {longMp3} from './listening-probe-fixture.mjs';

// A real Win32/Rust window and WebView2 runtime, using an isolated QA profile.
// No GC/working-set trimming, provider traffic, fake media clock or re-encoding.
// Do not run this together with builds or other browser/performance probes.
const option=(name,fallback)=>process.argv.find(v=>v.startsWith(`--${name}=`))?.slice(name.length+3)??fallback;
const baseline=process.argv.includes('--baseline'),label=option('label',baseline?'before':'after');
const app=resolve(option('app',baseline?'outputs/retention-native-baseline/app':'outputs/retention-native-candidate/app'));
const output=resolve(option('output','outputs/native-performance-qa'));
const idleSeconds=Number(option('idle-seconds','8'));
const turnGroups=Number(option('turn-groups','4'));
assert.ok(Number.isInteger(turnGroups)&&turnGroups>=1&&turnGroups<=24,'Turn groups must be1â€“24 (five turns per group)');
assert.ok(Number.isFinite(idleSeconds)&&idleSeconds>=8&&idleSeconds<=120,'Idle duration must be 8Ã¢â‚¬â€œ120 seconds');
const scenarios=option('cases','natural-network,contextual-cached').split(',');
assert.ok(scenarios.every(v=>/^(natural|contextual)-(network|cached)$/.test(v)));
assert.equal(process.platform,'win32');
await mkdir(output,{recursive:true});
const fixture=longMp3(await readFile('tests/fixtures/yasser-continuous-112.mp3'));
await writeFile(join(app,'WebApp','__native_performance_fixture.mp3'),fixture.body);
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
async function fingerprint(){
 const index=await readFile(join(app,'WebApp','index.html')),assets=[];
 for(const match of index.toString().matchAll(/(?:src|href)="(\/assets\/[^\"]+\.(?:js|css))"/g)){
  const bytes=await readFile(join(app,'WebApp',match[1]));assets.push({path:match[1],sha256:hash(bytes),bytes:bytes.length});
 }
 const worker=await readFile(join(app,'WebApp','audio-cache-worker.js')).catch(()=>null);
 return {app,indexSha256:hash(index),assets,audioWorker:worker?{sha256:hash(worker),bytes:worker.length}:null,qaExeSha256:hash(await readFile(join(app,'QuranMushaf.Windows.exe'))),webBuild:JSON.parse(await readFile(join(app,'WebApp','WEB-BUILD-SOURCE.json'),'utf8'))};
}
const report={label,baseline,startedAt:new Date().toISOString(),fingerprint:await fingerprint(),host:'Rust QA host with actual WebView2; QA profile and debugging only',requestedClient:{width:1280,height:800},requestedTurns:turnGroups*5,forcedGarbageCollection:false,fixture:{...fixture.metadata,maxRangeResponseBytes:512*1024},scenarios:[],limits:'Sampled Windows private bytes/working sets include this isolated Rust/WebView2 process tree. Shared working-set pages may be counted multiple times; peaks between samples and graphics-driver allocations can be missed. A short accelerated fixture is not a real-provider hour-long listening test.'};
const reportPath=join(output,`native-performance-${label}.json`),persist=()=>writeFile(reportPath,JSON.stringify(report,null,2));
const origin='https://quran-mushaf.local',fixtureUrl=origin+'/__native_performance_fixture.mp3';
const mp3='https://cdn.mp3quran.net/audio/yasser-dosari/r1/002.mp3';
const rangeChunkBytes=512*1024; // Bound CDP response messages; preserve the original MP3 bytes and total length.
const fixtureHeaders={'content-type':'audio/mpeg','content-length':String(fixture.body.length)};
const execFileAsync=promisify(execFile);
async function windowState(pid,action,dpr=1){
 const started=performance.now();
 const {stdout}=await execFileAsync('powershell.exe',['-NoLogo','-NoProfile','-NonInteractive','-File',resolve('scripts/native-probe-window.ps1'),'-HostProcessId',String(pid),'-Action',action,'-ClientWidth',String(Math.round(1280*dpr)),'-ClientHeight',String(Math.round(800*dpr))],{encoding:'utf8',windowsHide:true});
 return {...JSON.parse(stdout),helperMs:performance.now()-started};
}
async function osMemory(pid,kinds){
 const started=performance.now();
 const command=`$ErrorActionPreference='Stop'; $all=@(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId); $ids=[System.Collections.Generic.HashSet[int]]::new(); $ids.Add(${pid})|Out-Null; do { $added=$false; foreach($p in $all){if($ids.Contains([int]$p.ParentProcessId) -and $ids.Add([int]$p.ProcessId)){$added=$true}} } while($added); $rows=@(Get-Process -Id @($ids) -ErrorAction SilentlyContinue | ForEach-Object { [pscustomobject]@{id=$_.Id;name=$_.ProcessName;privateBytes=$_.PrivateMemorySize64;workingBytes=$_.WorkingSet64;cpuSeconds=$_.CPU} }); ConvertTo-Json -InputObject $rows -Compress`;
 const {stdout}=await execFileAsync('powershell.exe',['-NoLogo','-NoProfile','-NonInteractive','-Command',command],{encoding:'utf8',windowsHide:true});
 const rows=JSON.parse(stdout).map(row=>({...row,type:kinds.get(row.id)??(row.id===pid?'host':'unknown')}));
 return {privateBytes:rows.reduce((n,p)=>n+p.privateBytes,0),workingBytes:rows.reduce((n,p)=>n+p.workingBytes,0),cpuSeconds:rows.reduce((n,p)=>n+(p.cpuSeconds??0),0),processes:rows,helperMs:performance.now()-started};
}
for(const [index,name] of scenarios.entries()){
 const [mode,source]=name.split('-'),port=Number(option('port','19370'))+index;
 const scenario={mode,source,port,samples:[],audioRequests:[],blocked:[],errors:[]};report.scenarios.push(scenario);
 let browser,page,child,traceSession,traceDone;
 const traceEvents=[];
 try{
  const profile=join(output,`profile-${label}-${name}-${Date.now()}`);
  child=spawn(join(app,'QuranMushaf.Windows.exe'),['--qa-port',String(port),'--qa-profile',profile],{cwd:app,windowsHide:true,stdio:'ignore'});
  scenario.hostPid=child.pid;
  const deadline=Date.now()+30000;
  for(;;){try{browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`,{timeout:1500});break;}catch(error){if(Date.now()>deadline)throw error;await new Promise(r=>setTimeout(r,300));}}
  const context=browser.contexts()[0];page=context.pages()[0]??await context.waitForEvent('page');page.setDefaultTimeout(30000);
  page.on('pageerror',error=>scenario.errors.push(error.message));
  await page.waitForURL(origin+'/**');
  await page.addInitScript(({mode})=>{
   localStorage.setItem('quran-reader-onboarding-v2','done');
   localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:45,mode:'spread',theme:'dark',metadataLanguage:'english',contextualText:mode==='contextual',showClock:false,playerPlacement:'side'}));
   localStorage.setItem('quran-listening-preferences-v1',JSON.stringify({mode:'ayah',ayahReciterId:1001,speed:1,volume:.25,continuous:false}));
   const create=URL.createObjectURL.bind(URL),revoke=URL.revokeObjectURL.bind(URL),live=new Map();
   window.__nativePerf={live,created:0,revoked:0,policy:[],captureFolds:false,foldFrames:[]};
   const frame=window.requestAnimationFrame.bind(window),folds=new WeakMap();
   window.requestAnimationFrame=callback=>frame(now=>{if(window.__nativePerf.captureFolds){const fold=document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)');if(fold){let record=folds.get(fold);if(!record||record.id!==fold.dataset.turnId){record={id:fold.dataset.turnId,direction:fold.dataset.direction,times:[]};folds.set(fold,record);window.__nativePerf.foldFrames.push(record);}if(record.times.at(-1)!==now)record.times.push(now);}}callback(now);});
   URL.createObjectURL=o=>{const url=create(o);live.set(url,{type:o.constructor.name,bytes:o.size??null});window.__nativePerf.created++;return url;};
   URL.revokeObjectURL=url=>{live.delete(url);window.__nativePerf.revoked++;return revoke(url);};
   window.chrome.webview.addEventListener('message',event=>{if(event.data?.type==='native-qa-window-state')window.__nativePerf.policy.push(event.data);});
  },{mode});
  await context.route('**/*',async route=>{
   const url=route.request().url();if(url.startsWith(origin+'/'))return route.continue();
   if(url.includes('/ayat_timing?'))return route.fulfill({json:fixture.timings,headers:{'access-control-allow-origin':'*'}});
   if(url===mp3){
    const range=route.request().headers().range?.match(/^bytes=(\d+)-(\d*)$/),start=range?Number(range[1]):0,requestedEnd=range?.[2]?Math.min(Number(range[2]),fixture.body.length-1):fixture.body.length-1;
    const end=range?Math.min(requestedEnd,start+rangeChunkBytes-1):requestedEnd;
    const body=fixture.body.subarray(start,end+1),request={at:Date.now(),url,range:route.request().headers().range??null,bytes:body.length,fulfillStartedAt:Date.now()};scenario.audioRequests.push(request);
    try {
     await route.fulfill({status:range?206:200,body,headers:{'content-type':'audio/mpeg','content-length':String(body.length),'accept-ranges':'bytes','access-control-allow-origin':'*',...(range?{'content-range':`bytes ${start}-${end}/${fixture.body.length}`}:{})}});
    } catch(error) {request.fulfillError=String(error.message??error).slice(0,200);throw error;}
    finally {request.fulfillEndedAt=Date.now();request.fulfillMs=request.fulfillEndedAt-request.fulfillStartedAt;}
    return;
   }
   scenario.blocked.push(url);return route.abort();
  });
  await page.reload();await page.locator('.readerViewport .ayahRun').first().waitFor();
  scenario.beforeSize=await page.evaluate(()=>({width:innerWidth,height:innerHeight,dpr:devicePixelRatio}));
  scenario.window=await windowState(child.pid,'size',scenario.beforeSize.dpr);
  await page.waitForFunction(()=>innerWidth===1280&&innerHeight===800);
  await page.evaluate(()=>document.fonts.ready);
  if(mode==='contextual')await page.locator('.readerViewport canvas.shapedInkCanvas').first().waitFor();
  if(source==='cached')await page.evaluate(async({mp3,fixtureUrl,headers,timings})=>{
   const cache=await caches.open('quran-audio-v1'),response=await fetch(fixtureUrl);
   await cache.put(mp3,new Response(response.body,{headers}));
   const json=JSON.stringify(timings);await cache.put('https://www.mp3quran.net/api/v3/ayat_timing?surah=2&read=92',new Response(json,{headers:{'content-type':'application/json','content-length':String(new TextEncoder().encode(json).length)}}));
  },{mp3,fixtureUrl,headers:fixtureHeaders,timings:fixture.timings});
  const cdp=await context.newCDPSession(page),browserCdp=await browser.newBrowserCDPSession();
  await cdp.send('Performance.enable');scenario.runtime=await browserCdp.send('Browser.getVersion');
  if(process.argv.includes('--memory-trace')){
   traceSession=browserCdp;
   traceSession.on('Tracing.dataCollected',event=>traceEvents.push(...event.value));
   traceDone=new Promise(resolve=>traceSession.once('Tracing.tracingComplete',resolve));
   await traceSession.send('Tracing.start',{transferMode:'ReportEvents',traceConfig:{recordMode:'recordContinuously',traceBufferSizeInKb:8192,includedCategories:['disabled-by-default-memory-infra'],excludedCategories:['*']}});
   scenario.memoryDumps=[];
  }
  async function sample(phase){
   const kinds=new Map((await browserCdp.send('SystemInfo.getProcessInfo')).processInfo.map(p=>[Number(p.id),p.type]));
   const os=await osMemory(child.pid,kinds),metrics=Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(m=>[m.name,m.value]));
   const live=await page.evaluate(()=>{
    const a=document.querySelector('audio'),probe=window.__nativePerf,canvases=[...document.querySelectorAll('canvas')];
    return {audio:{src:a.getAttribute('src'),time:a.currentTime,paused:a.paused,ready:a.readyState,duration:Number.isFinite(a.duration)?a.duration:null,buffered:a.buffered.length},objectUrls:{created:probe.created,revoked:probe.revoked,live:[...probe.live.values()]},canvasBytes:canvases.reduce((sum,c)=>sum+c.width*c.height*4,0),canvasCount:canvases.length,domCount:document.querySelectorAll('*').length,previews:document.querySelectorAll('[data-preview-page]').length,folds:document.querySelectorAll('.bookFoldOverlay').length,visibility:document.visibilityState,pages:[...document.querySelectorAll('.readerViewport .pageSlot > .qulPage')].map(p=>Number(p.dataset.pageNumber)),policy:probe.policy.at(-1)??null};
   });
   if(traceSession && ['reading-idle','turn-group-4','turn-long-idle'].includes(phase)){
    const dump=await traceSession.send('Tracing.requestMemoryDump',{deterministic:false,levelOfDetail:'detailed'});
    scenario.memoryDumps.push({phase,...dump});
   }
   const value={phase,at:new Date().toISOString(),...os,...live,jsHeapBytes:metrics.JSHeapUsedSize,browserTaskSeconds:metrics.TaskDuration};scenario.samples.push(value);await persist();
   console.log(JSON.stringify({label,name,phase,privateMiB:+(os.privateBytes/1048576).toFixed(1),workingMiB:+(os.workingBytes/1048576).toFixed(1),helperMs:+os.helperMs.toFixed(1),liveUrls:live.objectUrls.live.length,source:live.audio.src,policy:live.policy}));return value;
  }
  await page.waitForTimeout(4000);await sample('reading-idle');
  await page.locator('.rustTitleBar').getByRole('button',{name:'Listen',exact:true}).click();
  const panel=page.locator('.readingToolsPanel');await panel.waitFor();
  await panel.getByRole('combobox',{name:'Surah',exact:true}).selectOption('2');
  await page.waitForTimeout(3000);const menu=await sample('menu-before-play');
  if(!baseline){assert.equal(menu.audio.src,null);assert.equal(menu.objectUrls.live.length,0);assert.equal(scenario.audioRequests.length,0);}
  await panel.getByRole('button',{name:'Play',exact:true}).click();
  await page.waitForFunction(()=>{const a=document.querySelector('audio');return !a.paused&&a.currentTime>.25&&a.duration>1000;});
  await page.waitForTimeout(3000);const playing=await sample('stationary-playing');
  assert.ok(Math.abs(playing.audio.duration-fixture.metadata.expectedDurationSeconds)<.15);
  await panel.locator('.tafsirPanelHeader').getByRole('button',{name:'Close',exact:true}).click();
  const navStart=await sample('follow-start');
  for(const ayah of [20,40,60,80,100,120]){
   const seconds=fixture.timings[ayah-1].start_time/1000+.2;
   await page.locator('audio').evaluate((a,seconds)=>{a.currentTime=seconds;},seconds);
   await page.waitForFunction(ayah=>!!document.querySelector(`.readerViewport .ayahRun-reciting[data-ayah-key="2:${ayah}"]`),ayah);
   await page.waitForTimeout(100);
  }
  await sample('follow-end');
  scenario.minimize=await windowState(child.pid,'minimize');await page.waitForTimeout(3000);
  if(!baseline)await page.evaluate(()=>window.chrome.webview.postMessage({type:'qa-window-state',action:'report'}));
  const minimized=await sample('minimized-playing');assert.equal(minimized.audio.paused,false);assert.ok(minimized.audio.time>playing.audio.time);
  if(process.argv.includes('--preserve-minimized-viewport')){
   scenario.minimizedViewport=await page.evaluate(()=>({width:innerWidth,height:innerHeight}));
   assert.deepEqual(scenario.minimizedViewport,{width:1280,height:800},'minimization does not rebuild the reader at caption dimensions');
  }
  if(!baseline){assert.equal(minimized.policy.minimized,true);if(minimized.policy.memorySupported)assert.equal(minimized.policy.memoryTarget,'low');}
  scenario.restore=await windowState(child.pid,'restore');await page.waitForTimeout(1000);
  if(!baseline)await page.evaluate(()=>window.chrome.webview.postMessage({type:'qa-window-state',action:'report'}));
  const restored=await sample('restored-playing');assert.equal(restored.audio.paused,false);assert.ok(restored.audio.time>minimized.audio.time);
  if(!baseline&&restored.policy.memorySupported)assert.equal(restored.policy.memoryTarget,'normal');
  const dock=page.locator('.recitationDock');
  if(await dock.getAttribute('data-expanded')!=='true')await page.getByRole('button',{name:'Show recitation controls',exact:true}).click();
  await page.getByRole('button',{name:'Stop recitation',exact:true}).click();await page.waitForTimeout(1000);await sample('stop-immediate');
  const requestsAfterStop=scenario.audioRequests.length;await page.waitForTimeout(8000);const stopped=await sample('stop-idle');
  if(!baseline){assert.equal(stopped.audio.src,null);assert.equal(stopped.objectUrls.live.length,0);assert.equal(scenario.audioRequests.length,requestsAfterStop,'Stop does not prepare or fetch a replacement source');}
  await page.keyboard.press('Control+f');const search=page.locator('.primaryQuranSearch .searchInput');await search.fill('43');await search.press('Enter');
  await page.waitForFunction(()=>document.querySelector('.readerViewport [data-page-number="43"]'));await page.waitForTimeout(600);
  const turnStart=await sample('turn-start'),turnWall=Date.now();
  await page.evaluate(()=>{window.__nativePerf.captureFolds=true;});
  for(let group=0;group<turnGroups;group++){
   for(let turn=0;turn<5;turn++){await page.locator('.sideNavNext').evaluate(button=>button.click());await page.waitForTimeout(75);}
   const expected=43+(group+1)*10;
   await page.waitForFunction(expected=>document.querySelector(`.readerViewport [data-page-number="${expected}"]`)&&!document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)'),expected);
   await sample(`turn-group-${group+1}`);
  }
  scenario.turnElapsedMs=Date.now()-turnWall;
  scenario.foldFrames=await page.evaluate(()=>{window.__nativePerf.captureFolds=false;return window.__nativePerf.foldFrames;});
  assert.equal(scenario.foldFrames.length,turnGroups*5,'every queued turn retains its fold animation');
  assert.ok(scenario.foldFrames.every(f=>f.times.length>=2),'every fold receives multiple native animation frames');
  const intervals=scenario.foldFrames.flatMap(f=>f.times.slice(1).map((t,i)=>t-f.times[i])).sort((a,b)=>a-b);
  scenario.frameIntervals={count:intervals.length,medianMs:intervals[Math.floor(intervals.length/2)],p95Ms:intervals[Math.floor(intervals.length*.95)],maxMs:Math.max(...intervals),note:'Delivered native RAF callback cadence within folds; not monitor-presented FPS.'};
  await page.waitForTimeout(8000);const final=await sample('turn-idle');
  let longIdle; if(idleSeconds>8){await page.waitForTimeout((idleSeconds-8)*1000);longIdle=await sample('turn-long-idle');}
  if(mode==='contextual'&&process.argv.includes('--verify-idle-wake')){
   const before=await page.evaluate(()=>({pages:[...document.querySelectorAll('.readerViewport .pageSlot > .qulPage')].map(p=>Number(p.dataset.pageNumber)),hiddenCanvases:document.querySelectorAll('.pageTurnPreviews canvas').length}));
   assert.equal(before.hiddenCanvases,0,'idle preview bitmaps have been released');
   await page.evaluate(()=>{window.__wakeCanvasSizes=null;const observer=new MutationObserver(()=>{const fold=document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)');if(fold){window.__wakeCanvasSizes=[...fold.querySelectorAll('canvas')].map(c=>({width:c.width,height:c.height}));observer.disconnect();}});observer.observe(document.querySelector('.readerViewport'),{childList:true,subtree:true});});
   const start=Date.now(),expected=Math.min(...before.pages)-2;
   await page.locator('.sideNavPrevious').evaluate(button=>button.click());
   await page.waitForFunction(expected=>document.querySelector('.readerViewport [data-page-number="'+expected+'"]')&&!document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)'),expected);
   const canvases=await page.evaluate(()=>window.__wakeCanvasSizes);
   assert.ok(canvases?.length>0&&canvases.every(c=>c.width>0&&c.height>0),'the first idle turn contains full-resolution artwork');
   scenario.idleWake={elapsedMs:Date.now()-start,foldCanvasCount:canvases.length,beforeHiddenCanvases:before.hiddenCanvases};
  }
  scenario.summary={peakSampledPrivateMiB:Math.max(...scenario.samples.map(s=>s.privateBytes))/1048576,stopIdlePrivateMiB:stopped.privateBytes/1048576,turnIdlePrivateMiB:final.privateBytes/1048576,longIdlePrivateMiB:longIdle?.privateBytes/1048576,longIdleSeconds:idleSeconds,followBrowserTaskSeconds:scenario.samples.find(s=>s.phase==='follow-end').browserTaskSeconds-navStart.browserTaskSeconds,turnBrowserTaskSeconds:final.browserTaskSeconds-turnStart.browserTaskSeconds,turnElapsedMs:scenario.turnElapsedMs,stopLiveUrls:stopped.objectUrls.live.length,menuPreparedMedia:!!menu.audio.src};
  assert.deepEqual(scenario.errors,[]);assert.deepEqual(scenario.blocked,[]);
  scenario.finishedAt=new Date().toISOString();await persist();
 }catch(error){scenario.failure={message:error.message,stack:error.stack};if(page){scenario.failure.state=await page.evaluate(()=>{const a=document.querySelector('audio');return {audio:a?{src:a.src,current:a.currentSrc,ready:a.readyState,network:a.networkState,time:a.currentTime,duration:a.duration,paused:a.paused,error:a.error?.message}:null,visibility:document.visibilityState};}).catch(()=>null);await page.screenshot({path:join(output,`${label}-${name}-failure.png`)}).catch(()=>{});}await persist();throw error;}
 finally{
  if(traceSession){
   await traceSession.send('Tracing.end').catch(()=>{});
   await Promise.race([traceDone,new Promise(resolve=>setTimeout(resolve,10000))]);
   await writeFile(join(output,`memory-trace-${label}-${name}.json`),JSON.stringify({traceEvents}));
   scenario.memoryTrace={events:traceEvents.length,file:`memory-trace-${label}-${name}.json`,forcedGarbageCollection:false};
   await persist();
  }
  if(page&&!page.isClosed())await page.evaluate(()=>window.chrome.webview.postMessage({type:'window-action',action:'close'})).catch(()=>{});
  if(child&&child.exitCode===null)await Promise.race([new Promise(r=>child.once('exit',r)),new Promise(r=>setTimeout(r,5000))]);
  if(child&&child.exitCode===null)child.kill();await browser?.close().catch(()=>{});
 }
}
assert.deepEqual(await fingerprint(),report.fingerprint,'QA payload remains unchanged during measurement');
report.finishedAt=new Date().toISOString();await persist();console.log(JSON.stringify(report.scenarios.map(s=>({mode:s.mode,source:s.source,...s.summary})),null,2));
