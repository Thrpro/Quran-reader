import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

const baseline = process.argv.includes('--baseline');
const root = resolve(baseline ? 'outputs/rust-windows/publish/WebApp' : 'dist');
const output = resolve('outputs/player-performance-qa');
await mkdir(output, {recursive:true});
const mime = {'.html':'text/html','.js':'text/javascript','.css':'text/css','.json':'application/json','.otf':'font/otf','.woff2':'font/woff2'};
const server = createServer(async (req,res) => {
  const path=resolve(root, '.'+new URL(req.url,'http://localhost').pathname.replace(/^\/$/,'/index.html'));
  if(!path.startsWith(root+sep)) return res.writeHead(403).end();
  try {const content=await readFile(path);res.writeHead(200,{'content-type':mime[extname(path)]??'application/octet-stream'}).end(content);}
  catch {res.writeHead(404).end();}
});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
const browser=await chromium.launch({channel:'msedge',headless:true});
const report=[];
try {
  for(const contextualText of [false,true]) {
    const page=await browser.newPage({viewport:{width:1920,height:1080}});
    const errors=[];page.on('pageerror',e=>errors.push(e.message));
    await page.addInitScript(contextualText=>{
      localStorage.setItem('quran-reader-onboarding-v2','done');
      localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:45,mode:'spread',theme:'dark',metadataLanguage:'english',contextualText}));
    },contextualText);
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.locator('.readerViewport [data-page-number="45"]').waitFor();
    if(contextualText) await page.locator('.readerViewport .shapedQuranLine').first().waitFor();
    await page.waitForTimeout(3500);
    const cdp=await page.context().newCDPSession(page);await cdp.send('Performance.enable');
    const metrics=async()=>Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(m=>[m.name,m.value]));
    const before=await metrics();const samples=[];
    for(let i=0;i<8;i++) {
      const target=i%2===0?47:45;
      const start=performance.now();
      await page.keyboard.press(i%2===0?'ArrowLeft':'ArrowRight');
      await page.locator(`.readerViewport [data-page-number="${target}"]`).waitFor();
      await page.waitForFunction(()=>!document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)'));
      samples.push(Math.round(performance.now()-start));
      await page.waitForTimeout(120);
    }
    const after=await metrics();await page.waitForTimeout(3500);
    await cdp.send('HeapProfiler.collectGarbage');
    const idle=await metrics();
    const retainedFolds=await page.locator('.bookFoldOverlay').count();
    if(!baseline) assert.equal(retainedFolds,0,'idle fold snapshots are released');
    assert.deepEqual(errors,[]);
    report.push({contextualText,taskMs:(after.TaskDuration-before.TaskDuration)*1000,layoutMs:(after.LayoutDuration-before.LayoutDuration)*1000,recalcStyleMs:(after.RecalcStyleDuration-before.RecalcStyleDuration)*1000,idleHeapBytes:idle.JSHeapUsedSize,idleDom:await cdp.send('Memory.getDOMCounters'),retainedFolds,turnWallMs:samples});
    await page.close();
  }
  await writeFile(resolve(output,baseline?'resources-before.json':'resources-after.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report));
} finally {await browser.close();await new Promise(done=>server.close(done));}
