import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {resolve,extname,sep} from 'node:path';
import {chromium} from 'playwright';

const baseline=process.argv.includes('--baseline');
const root=resolve(process.argv.find(value=>value.startsWith('--root='))?.slice(7)||(baseline?'outputs/rust-windows/publish/WebApp':'dist'));
const output=resolve(process.argv.find(value=>value.startsWith('--output='))?.slice(9)||'outputs/render-work-qa');
await mkdir(output,{recursive:true});
const mime={'.html':'text/html','.js':'text/javascript','.css':'text/css','.json':'application/json','.otf':'font/otf','.woff2':'font/woff2'};
const server=createServer(async(request,response)=>{
  const path=resolve(root,'.'+new URL(request.url,'http://localhost').pathname.replace(/^\/$/,'/index.html'));
  if(!path.startsWith(root+sep))return response.writeHead(403).end();
  try{response.writeHead(200,{'content-type':mime[extname(path)]||'application/octet-stream'}).end(await readFile(path));}catch{response.writeHead(404).end();}
});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
const origin='http://127.0.0.1:'+server.address().port;
const mp3=await readFile('tests/fixtures/yasser-continuous-112.mp3');
const timings=[{ayah:1,start_time:63,end_time:2310},{ayah:2,start_time:2310,end_time:4515},{ayah:3,start_time:4515,end_time:7287},{ayah:4,start_time:7287,end_time:10668}];
const browser=await chromium.launch({channel:'msedge',headless:true,args:['--autoplay-policy=no-user-gesture-required']});
const contexts=[],report={baseline,root,startedAt:new Date().toISOString(),modes:[],note:'Authentic local audio; React memo identity and DOM allocation counts are deterministic guards, not a hardware FPS or total WebView2 memory benchmark.'};
try{
  for(const contextual of [false,true]){
    const context=await browser.newContext({viewport:{width:1920,height:1080}});contexts.push(context);
    const page=await context.newPage(),errors=[],blocked=[];page.on('pageerror',error=>errors.push(error.message));
    await page.addInitScript(({contextual})=>{
      window.__REACT_DEVTOOLS_GLOBAL_HOOK__={supportsFiber:true,inject(){return 1;},onCommitFiberRoot(_id,root){window.__renderWorkRoot=root;},onCommitFiberUnmount(){}};
      window.__QURAN_RUST_HOST__=true;window.chrome??={};window.chrome.webview={postMessage(){},addEventListener(){},removeEventListener(){}};
      localStorage.setItem('quran-reader-onboarding-v2','done');
      localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:604,mode:'spread',theme:'dark',metadataLanguage:'english',contextualText:contextual,playerPlacement:'side'}));
      localStorage.setItem('quran-listening-preferences-v1',JSON.stringify({mode:'ayah',ayahReciterId:1001,reciterId:0,recordingId:0,speed:1,volume:.1,continuous:false}));
      window.__foldAllocations={interactionClones:0,copiedElements:0,setupMs:[]};
      let setupStart=0;
      const clone=Node.prototype.cloneNode;
      Node.prototype.cloneNode=function(deep){
        if(this instanceof Element&&this.closest('.qulPage')){
          const shapedHit=this.matches('.shapedQuranLine .ayahRun')?1:0;
          const descendants=deep?this.querySelectorAll('.shapedQuranLine .ayahRun').length:0;
          window.__foldAllocations.interactionClones+=shapedHit+descendants;
          window.__foldAllocations.copiedElements+=1+(deep?this.querySelectorAll('*').length:0);
          if(!setupStart&&this.matches('.qulPage'))setupStart=performance.now();
        }
        return clone.call(this,deep);
      };
      const append=Element.prototype.append;
      Element.prototype.append=function(...nodes){
        const result=append.apply(this,nodes);
        if(setupStart&&nodes.some(node=>node instanceof Element&&node.classList.contains('bookFoldOverlay'))){window.__foldAllocations.setupMs.push(performance.now()-setupStart);setupStart=0;}
        return result;
      };
      window.__lineFibers=()=>{
        const lines=new Map();
        const visit=fiber=>{
          if(!fiber)return;
          const props=fiber.memoizedProps;
          // Production function names are minified. Identify these memoized
          // line components by their public prop shape, never a bundle name.
          const natural=props?.line&&'recitationAyah' in props&&'highlightedAyah' in props&&typeof props.text==='string';
          const shaped=props?.shape&&props.outlines&&Array.isArray(props.words)&&'reciting' in props&&typeof props.lineNumber==='number';
          if((fiber.tag===0||fiber.tag===15)&&(natural||shaped)){
            const words=props.words??props.line.words;
            let ancestor=fiber.return;while(ancestor&&!ancestor.stateNode?.matches?.('article.qulPage'))ancestor=ancestor.return;
            const number=ancestor?.stateNode?.dataset.pageNumber;
            if(number){
              const key=number+':'+(props.lineNumber??props.line.lineNumber);
              lines.set(key,{props,words,reciting:props.reciting??props.recitationAyah??null,shape:props.shape,hook:fiber.memoizedState});
            }
          }
          visit(fiber.child);visit(fiber.sibling);
        };
        visit(window.__renderWorkRoot?.current);return lines;
      };
    },{contextual});
    await context.route('**/*',route=>{
      const url=route.request().url();if(url.startsWith(origin+'/'))return route.continue();
      if(url.includes('/ayat_timing?'))return route.fulfill({json:timings,headers:{'access-control-allow-origin':'*'}});
      if(url.includes('/api/v3/reciters?'))return route.fulfill({json:{reciters:[]},headers:{'access-control-allow-origin':'*'}});
      if(new URL(url).pathname.endsWith('.mp3')){
        const range=route.request().headers().range?.match(/^bytes=(\d+)-(\d*)$/),start=range?Number(range[1]):0,end=range?.[2]?Math.min(Number(range[2]),mp3.length-1):mp3.length-1,body=mp3.subarray(start,end+1);
        return route.fulfill({status:range?206:200,headers:{'content-type':'audio/mpeg','content-length':String(body.length),'accept-ranges':'bytes','access-control-allow-origin':'*',...(range?{'content-range':`bytes ${start}-${end}/${mp3.length}`}:{})},body});
      }
      blocked.push(url);return route.abort();
    });
    await page.goto(origin);await page.locator('.readerViewport [data-ayah-key="112:1"]').first().waitFor();
    await page.waitForFunction(contextual=>{
      const previews=[...document.querySelectorAll('.pageTurnPreviews [data-preview-page]')];
      // Ikhlas is on the final spread, with its two valid previous previews.
      return window.__lineFibers().size>50&&previews.length===2&&(!contextual||[...document.querySelectorAll('.readerViewport .shapedLineInk')].every(el=>el.querySelector('canvas.shapedInkCanvas')));
    },contextual);
    await page.locator('.rustTitleBar').getByRole('button',{name:'Listen',exact:true}).click();
    await page.locator('.readingToolsPanel').getByRole('button',{name:'Play',exact:true}).click();
    await page.waitForFunction(()=>{const a=document.querySelector('audio');return !a.paused&&a.currentTime>.1;});
    await page.locator('.readingToolsPanel .tafsirPanelHeader').getByRole('button',{name:'Close',exact:true}).click();
    await page.locator('audio').evaluate(a=>{a.currentTime=.2;a.dispatchEvent(new Event('timeupdate'));});
    await page.waitForFunction(()=>document.querySelector('.readerViewport .ayahRun-reciting[data-ayah-key="112:1"]'));
    await page.evaluate(()=>{window.__beforeLineProps=window.__lineFibers();});
    await page.locator('audio').evaluate(a=>{a.currentTime=3;a.dispatchEvent(new Event('timeupdate'));});
    await page.waitForFunction(()=>document.querySelector('.readerViewport .ayahRun-reciting[data-ayah-key="112:2"]'));
    const updates=await page.evaluate(()=>{
      const after=window.__lineFibers(),result={total:after.size,affected:[],unaffectedChanged:[],missing:[]};
      for(const[key,before]of window.__beforeLineProps){
        const next=after.get(key);if(!next){result.missing.push(key);continue;}
        const relevant=before.words.some(word=>word.surah===112&&(word.ayah===1||word.ayah===2));
        if(relevant)result.affected.push({key,changed:before.props!==next.props,from:before.reciting,to:next.reciting});
        else if(before.props!==next.props)result.unaffectedChanged.push(key);
      }
      return result;
    });
    if(!baseline){assert.deepEqual(updates.missing,[]);assert.deepEqual(updates.unaffectedChanged,[],'recitation must not rerender unrelated visible or preview lines');assert.ok(updates.affected.some(line=>line.changed&&line.to==='112:2'),'the next authentic ayah is highlighted');}
    await page.locator('audio').evaluate(a=>a.pause());
    // Hold a real fold after the highlight update; copy probes must reject the
    // former clone-everything-then-remove allocation pattern.
    const source=page.locator('.readerViewport .rightSlot > .qulPage'),box=await source.boundingBox();
    await page.mouse.move(box.x+box.width*.75,box.y+box.height*.7);await page.mouse.down();
    await page.mouse.move(box.x+box.width*.28,box.y+box.height*.7,{steps:8});
    await page.locator('.bookFoldOverlay:not(.bookFoldWarmup)').waitFor();
    const folds=await page.evaluate(()=>({...window.__foldAllocations,heldInteractionNodes:document.querySelectorAll('.bookFoldOverlay .shapedQuranLine .ayahRun').length}));
    if(!baseline&&contextual){assert.equal(folds.interactionClones,0,'folds never allocate invisible source-text/hit-target clones');assert.equal(folds.heldInteractionNodes,0);}
    await page.mouse.move(box.x+box.width*.74,box.y+box.height*.7,{steps:8});await page.mouse.up();
    await page.waitForFunction(()=>!document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)'));
    assert.deepEqual(errors,[]);assert.deepEqual(blocked,[]);
    report.modes.push({contextual,updates,folds});await context.close();
  }
  report.finishedAt=new Date().toISOString();await writeFile(resolve(output,baseline?'baseline.json':'verification.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report,null,2));
}catch(error){report.failure=error.stack||String(error);await writeFile(resolve(output,'failure.json'),JSON.stringify(report,null,2));for(const context of contexts)for(const page of context.pages())await page.screenshot({path:resolve(output,'failure.png')}).catch(()=>{});throw error;}
finally{await Promise.allSettled(contexts.map(context=>context.close()));await browser.close();await new Promise(done=>server.close(done));}
