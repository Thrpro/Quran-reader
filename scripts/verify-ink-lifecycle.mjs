import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

const baseline = process.argv.includes('--baseline');
const root = resolve(process.argv.find(arg=>arg.startsWith('--root='))?.slice(7) ?? (baseline ? 'outputs/rust-windows/publish/WebApp' : 'dist'));
const output = resolve(process.argv.find(arg=>arg.startsWith('--output='))?.slice(9) ?? 'outputs/ink-lifecycle-qa');
await mkdir(output, { recursive: true });
const mime = {'.html':'text/html','.js':'text/javascript','.css':'text/css','.json':'application/json','.otf':'font/otf','.woff2':'font/woff2'};
const server = createServer(async (request, response) => {
  const path = resolve(root, '.' + new URL(request.url, 'http://localhost').pathname.replace(/^\/$/, '/index.html'));
  if (!path.startsWith(root + sep)) return response.writeHead(403).end();
  try { const bytes = await readFile(path); response.writeHead(200, {'content-type':mime[extname(path)] ?? 'application/octet-stream'}).end(bytes); }
  catch { response.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const browser = await chromium.launch({channel:'msedge',headless:true});
const report = { baseline, themes:[], resize:[], unchangedGeometry:null, heldLayout:null };
try {
  const page = await browser.newPage({viewport:{width:1920,height:1051}}), errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    localStorage.setItem('quran-reader-onboarding-v2', 'done');
    localStorage.setItem('quran-mushaf-qul-reader:v1', JSON.stringify({settingsVersion:5,languageSetupVersion:1,
      hasChosenLanguage:true,page:45,contextualText:true,mode:'spread',theme:'dark',metadataLanguage:'english'}));
    const visible = element => element.closest('.pageSlot')?.closest('.readerViewport');
    window.__inkMetrics = { paints:0, removals:0, observers:0 };
    window.__layoutPaints = 0;
    const clear = CanvasRenderingContext2D.prototype.clearRect;
    CanvasRenderingContext2D.prototype.clearRect = function (...args) {
      if (this.canvas.classList.contains('shapedInkCanvas') && visible(this.canvas)) {
        window.__inkMetrics.paints++;
        if (this.canvas.closest('.readerScaleSurface')?.getAnimations().some(a => a.playState === 'running' || a.pending)) window.__layoutPaints++;
      }
      return clear.apply(this, args);
    };
    const remove = Element.prototype.remove;
    Element.prototype.remove = function (...args) {
      if (this.matches('canvas.shapedInkCanvas') && visible(this)) window.__inkMetrics.removals++;
      return remove.apply(this, args);
    };
    const NativeObserver = ResizeObserver;
    window.ResizeObserver = class extends NativeObserver {
      observe(target, ...args) {
        if (target.classList.contains('shapedLineInk') && visible(target)) window.__inkMetrics.observers++;
        return super.observe(target, ...args);
      }
    };
  });
  await page.goto('http://127.0.0.1:' + server.address().port);
  await page.waitForFunction(() => document.querySelectorAll('.readerViewport .pageSlot > .qulPage canvas.shapedInkCanvas').length === 30 &&
    document.querySelectorAll('.pageTurnPreviews .shapedLineInk').length >= 30 && document.querySelectorAll('.pageTurnPreviews .shapedInkCanvas').length === 0);
  await page.waitForTimeout(500);
  await page.evaluate(() => {
    window.__inkNodes = [...document.querySelectorAll('.readerViewport .pageSlot > .qulPage canvas.shapedInkCanvas')];
    window.__inkMetrics = { paints:0, removals:0, observers:0 };
  });
  const snapshot = () => page.evaluate(() => ({...window.__inkMetrics,
    sameNodes:window.__inkNodes.every((node, index) => node === document.querySelectorAll('.readerViewport .pageSlot > .qulPage canvas.shapedInkCanvas')[index])}));
  await page.locator('.readerSettingsControl').evaluate(button => button.click());
  for (const theme of ['navy','black','light','dark']) {
    await page.getByLabel('Theme', {exact:true}).selectOption(theme);
    await page.waitForTimeout(400);
    const measured = await snapshot();
    if (!baseline) {
      assert.equal(measured.sameNodes, true, `theme ${theme} preserves the mounted line canvases`);
      assert.equal(measured.removals, 0, `theme ${theme} does not release active canvases`);
      assert.equal(measured.observers, 0, `theme ${theme} does not recreate line observers`);
    }
    report.themes.push({theme,...measured});
  }
  await page.keyboard.press('Escape');
  if (!baseline) {
    // Hold a genuine layout transform long enough to inspect delivered work,
    // independently of the runner's frame rate or automation latency.
    report.heldLayout = await page.evaluate(async () => {
      const surface = document.querySelector('.readerScaleSurface');
      const animation = surface.animate([{transform:'scale(.85)'},{transform:'scale(1)'}], {duration:60000});
      window.__inkMetrics = {paints:0,removals:0,observers:0}; window.__layoutPaints = 0;
      dispatchEvent(new Event('resize'));
      await new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done)));
      const held = {...window.__inkMetrics, duringLayoutPaints:window.__layoutPaints};
      animation.cancel();
      await new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done)));
      return {held, afterCancel:{...window.__inkMetrics}, sameNodes:window.__inkNodes.every((node,index) => node === document.querySelectorAll('.readerViewport .pageSlot > .qulPage canvas.shapedInkCanvas')[index])};
    });
    assert.equal(report.heldLayout.held.paints, 0, 'existing ink is not allocated at intermediate layout scales');
    assert.equal(report.heldLayout.sameNodes, true, 'cancelling a layout animation preserves mounted ink');
    await page.evaluate(() => {window.__layoutPaints = 0;});
  }
  for (const size of [{width:1280,height:900},{width:1920,height:1051}]) {
    await page.setViewportSize(size); await page.waitForTimeout(500);
    await page.waitForFunction(() => [...document.querySelectorAll('.readerScaleSurface')].every(surface => surface.getAnimations().every(animation => animation.playState !== 'running')));
    await page.evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))));
    const measured = await snapshot();
    if (!baseline) {
      assert.equal(measured.sameNodes, true, 'resize preserves the mounted drawing contexts');
      assert.equal(measured.removals, 0); assert.equal(measured.observers, 0);
    }
    report.resize.push({...size,...measured});
  }
  if (!baseline) assert.equal(await page.evaluate(() => window.__layoutPaints), 0, 'resize-only artwork waits for settled FLIP geometry');
  // A real resize can still be finishing its FLIP animation on a busy runner.
  // Observe completion before testing notifications with unchanged geometry.
  await page.evaluate(() => { window.__inkMetrics = {paints:0,removals:0,observers:0}; for (let i=0;i<12;i++) dispatchEvent(new Event('resize')); });
  await page.waitForTimeout(500);
  report.unchangedGeometry = await snapshot();
  if (!baseline) {
    assert.equal(report.unchangedGeometry.paints, 0, 'unchanged geometry does not rasterize text again');
    assert.equal(report.unchangedGeometry.sameNodes, true);
  }
  if (!baseline) {
    await page.evaluate(()=>{
      window.__inkBySlot=()=>new Map([...document.querySelectorAll('.readerViewport .pageSlot > .qulPage')].flatMap((article,slot)=>[...article.querySelectorAll('.shapedQuranLine')].map(line=>[slot+':'+line.style.getPropertyValue('--line-number'),line.querySelector('canvas')])));
      window.__previousInk=window.__inkBySlot();
      window.__retainedArticles=[...document.querySelectorAll('.readerViewport .pageSlot > .qulPage')];
      window.__retainedPreviews=[...document.querySelectorAll('[data-preview-page]')];
      window.__inkMetrics={paints:0,removals:0,observers:0};
    });
    report.navigation=[];
    for(let number=47;number<=65;number+=2){
      await page.locator('.sideNavNext').evaluate(button=>button.click());
      await page.waitForFunction(number=>document.querySelector(`.readerViewport [data-page-number="${number}"]`)&&!document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)'),number);
      const expectedInk=(await Promise.all([number,number+1].map(async n=>Object.keys(JSON.parse(await readFile(resolve(root,`experimental/digital-khatt/shaped/p${n}.json`),'utf8')).lines).length))).reduce((a,b)=>a+b,0);
      await page.waitForFunction(expected=>document.querySelectorAll('.readerViewport .pageSlot > .qulPage .shapedInkCanvas').length===expected,expectedInk);
      const retained=await page.evaluate(()=>{const previous=window.__previousInk,current=window.__inkBySlot();
        const added=[...current.keys()].filter(key=>!previous.has(key)).length,removed=[...previous.keys()].filter(key=>!current.has(key)).length;
        const same=[...current].filter(([key])=>previous.has(key)).every(([key,node])=>node===previous.get(key));
        window.__previousInk=current;
        const result={
        articles:window.__retainedArticles.every((node,index)=>node===document.querySelectorAll('.readerViewport .pageSlot > .qulPage')[index]),
        previews:window.__retainedPreviews.every((node,index)=>node===document.querySelectorAll('[data-preview-page]')[index]),
        canvases:same,added,removed,
        ...window.__inkMetrics,
      };window.__inkMetrics={paints:0,removals:0,observers:0};return result;});
      assert.equal(retained.articles,true,'page slots retain mounted articles during navigation');
      assert.equal(retained.previews,true,'neighbor preview slots retain mounted subtrees');
      assert.equal(retained.canvases,true,'cached navigation reuses visible drawing contexts');
      assert.ok(retained.removals<=retained.removed,'only lines replaced by authentic headings may release visible canvases');
      assert.equal(retained.observers,retained.added,'only newly introduced ayah lines create observers');
      report.navigation.push({page:number,...retained});
    }
    await page.evaluate(()=>{
      window.__visibleBeforeIdle=[...document.querySelectorAll('.readerViewport .pageSlot > .qulPage .shapedInkCanvas')];
      window.__visiblePixels=window.__visibleBeforeIdle.map(c=>c.getContext('2d').getImageData(0,0,c.width,c.height).data);
    });
    await page.waitForFunction(()=>document.querySelectorAll('.pageTurnPreviews .shapedInkCanvas').length===0);
    report.idle=await page.evaluate(()=>({
      sameNodes:window.__visibleBeforeIdle.every((c,i)=>c===document.querySelectorAll('.readerViewport .pageSlot > .qulPage .shapedInkCanvas')[i]),
      samePixels:window.__visibleBeforeIdle.every((c,i)=>c.getContext('2d').getImageData(0,0,c.width,c.height).data.every((v,j)=>v===window.__visiblePixels[i][j])),
      hiddenCanvases:document.querySelectorAll('.pageTurnPreviews .shapedInkCanvas').length,
      speculativeFolds:document.querySelectorAll('.bookFoldWarmup').length,
    }));
    assert.equal(report.idle.sameNodes,true,'idle cleanup leaves visible drawing contexts mounted');
    assert.equal(report.idle.samePixels,true,'idle cleanup preserves every visible pixel');
    assert.equal(report.idle.speculativeFolds,0,'contextual idle keeps no unused fold copy');
    await page.evaluate(()=>{
      const observer=new MutationObserver(()=>{
        const fold=document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)');if(!fold)return;
        window.__wakeFold=[...fold.querySelectorAll('canvas')].map(c=>({width:c.width,height:c.height}));observer.disconnect();
      });observer.observe(document.querySelector('.readerViewport'),{childList:true,subtree:true});
    });
    await page.locator('.sideNavPrevious').evaluate(button=>button.click());
    await page.waitForFunction(()=>document.querySelector('.readerViewport [data-page-number="63"]')&&!document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)'));
    report.wake=await page.evaluate(()=>({fold:window.__wakeFold,hiddenCanvases:document.querySelectorAll('.pageTurnPreviews .shapedInkCanvas').length}));
    assert.equal(report.wake.fold?.length,30,'the first turn after idle has both fully painted fold faces');
    assert.ok(report.wake.fold.every(c=>c.width>0&&c.height>0),'no blank canvas is animated after idle');
    assert.ok(report.wake.hiddenCanvases>0,'actual navigation warms neighboring raster stores again');
  }
  assert.deepEqual(errors, []);
  await writeFile(resolve(output,baseline ? 'before.json' : 'verification.json'), JSON.stringify(report,null,2));
  console.log(JSON.stringify(report,null,2));
  console.log(baseline ? 'Recorded baseline canvas churn.' : 'Mounted ink canvases/observers survive theme and layout changes; unchanged geometry does not repaint.');
} catch (error) {
  await writeFile(resolve(output,baseline ? 'before.json' : 'verification.json'), JSON.stringify({...report,failure:error.message},null,2));
  throw error;
} finally { await browser.close(); await new Promise(done => server.close(done)); }
