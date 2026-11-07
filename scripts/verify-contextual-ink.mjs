import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { createRequire } from 'node:module';
import { chromium } from 'playwright';
import { build } from 'esbuild';
import { compareInk, assertInkMatch } from './ink-comparison.mjs';
const require=createRequire(import.meta.url), {PNG}=require('../node_modules/playwright-core/lib/utilsBundle.js');
const root=resolve('dist'),output=resolve('outputs/ink-qa');await mkdir(output,{recursive:true});
// The reference keeps the complete original canvas viewport. It runs only in
// this fixture so cropped rows can be checked for exact pixel preservation.
const {outputFiles:[paintReference]}=await build({entryPoints:['src/paintShapedLine.ts'],bundle:true,write:false,format:'esm',platform:'browser'});
const mime={'.html':'text/html','.js':'application/javascript','.css':'text/css','.json':'application/json','.otf':'font/otf','.woff2':'font/woff2'};
const server=createServer(async(req,res)=>{
 if(new URL(req.url,'http://localhost').pathname==='/__qa-paint.js')return res.writeHead(200,{'content-type':'application/javascript'}).end(paintReference.contents);
 const path=resolve(root,'.'+new URL(req.url,'http://localhost').pathname.replace(/^\/$/,'/index.html'));
 if(!path.startsWith(root+sep))return res.writeHead(403).end();
 try{const bytes=await readFile(path);res.writeHead(200,{'content-type':mime[extname(path)]??'application/octet-stream'}).end(bytes);}catch{res.writeHead(404).end();}
});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
const browser=await chromium.launch({channel:'msedge',headless:true,args:process.argv.includes('--software-rendering')?['--disable-gpu']:[]});
function compare(actualBytes,referenceBytes){
 return compareInk(PNG.sync.read(actualBytes),PNG.sync.read(referenceBytes));
}
const reports=[];
try{
 for(const dpr of [1,1.25,2]){
  const page=await browser.newPage({viewport:{width:1920,height:1051},deviceScaleFactor:dpr});const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(()=>{localStorage.setItem('quran-reader-onboarding-v2','done');localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:43,contextualText:true,mode:'spread',theme:'dark',metadataLanguage:'english'}));});
  await page.goto('http://127.0.0.1:'+server.address().port);
  await page.waitForFunction(()=>document.querySelectorAll('.pageSlot > .qulPage canvas.shapedInkCanvas').length===30&&document.fonts.status==='loaded');
  await page.evaluate(()=>window.__inkBeforeTurn=document.querySelectorAll('.pageSlot > .qulPage[data-page-number="43"] canvas.shapedInkCanvas')[11]);
  await page.locator('.sideNavNext').evaluate(button=>button.click());
  await page.waitForFunction(()=>document.querySelector('.pageSlot > .qulPage[data-page-number="45"]')&&!document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)'));
  assert.equal(await page.evaluate(()=>window.__inkBeforeTurn===document.querySelectorAll('.pageSlot > .qulPage[data-page-number="45"] canvas.shapedInkCanvas')[11]),true,'the original-outline comparison uses a retained canvas after navigation');
  const ink=page.locator('.pageSlot > .qulPage[data-page-number="45"] .shapedLineInk').nth(11);await ink.waitFor();
  await page.waitForFunction(()=>document.querySelectorAll('.pageSlot > .qulPage canvas.shapedInkCanvas').length===30&&document.fonts.status==='loaded');
  const actual=await ink.screenshot({path:resolve(output,`contextual-dark-${dpr}.png`)});
  const cropComparison=await ink.evaluate(async(element)=>{
   const {paintShapedLine}=await import('/__qa-paint.js');
   const shape=(await(await fetch('/experimental/digital-khatt/shaped/p45.json')).json()).lines[12];
   const outlines=await(await fetch('/experimental/digital-khatt/shaped/glyphs.json')).json();
   const original=element.querySelector('canvas'),rect=element.getBoundingClientRect(),style=getComputedStyle(element),ratio=Math.max(2,devicePixelRatio);
   const left=Math.floor(rect.left*ratio),top=Math.floor(rect.top*ratio);
   const frame={width:rect.width*ratio,height:rect.height*ratio,offsetX:rect.left*ratio-left,offsetY:rect.top*ratio-top};
   const full=document.createElement('canvas');full.width=Math.ceil(rect.right*ratio)-left;full.height=Math.ceil(rect.bottom*ratio)-top;
   paintShapedLine(full,shape,outlines,style.getPropertyValue('--mushaf-text').trim()||style.color,element.dataset.inkContrast==='true',frame);
   const offset=getComputedStyle(original).transform==='none'?parseFloat(original.style.top):new DOMMatrix(getComputedStyle(original).transform).m42;
   const cropTop=Math.round(offset*(rect.height/parseFloat(style.height))*ratio+frame.offsetY);
   const reference=full.getContext('2d').getImageData(0,0,full.width,full.height).data;
   const cropped=original.getContext('2d').getImageData(0,0,original.width,original.height).data;
   let changed=0;
   for(let row=0;row<full.height;row++)for(let col=0;col<full.width*4;col++){
     const source=reference[row*full.width*4+col];
     const actual=row>=cropTop&&row<cropTop+original.height?cropped[(row-cropTop)*original.width*4+col]:0;
     if(source!==actual)changed++;
   }
   const result={changedChannels:changed,fullBytes:full.width*full.height*4,croppedBytes:original.width*original.height*4,cropTop};
   full.width=0;full.height=0;return result;
  });
  assert.equal(cropComparison.changedChannels,0,'cropping preserves every original canvas pixel and discards only transparent rows');
  // Compare to the former SVG mask using the same supplied paths/coordinates.
  // This reference exists only in the test, never in the application.
  await ink.evaluate(async(canvas,dpr)=>{
   const shape=(await(await fetch('/experimental/digital-khatt/shaped/p45.json')).json()).lines[12];
   const outlines=await(await fetch('/experimental/digital-khatt/shaped/glyphs.json')).json();
   const paths=shape.glyphs.map(([id,x,y])=>`<path d="${outlines[id]}" transform="translate(${x} ${y})"/>`).join('');
   const filter='<defs><filter id="inkContrast" x="-1%" y="-1%" width="102%" height="102%"><feComponentTransfer><feFuncA type="gamma" amplitude="1" exponent="0.85" offset="0"/></feComponentTransfer></filter></defs>';
   const svg=`<svg xmlns="http://www.w3.org/2000/svg" viewBox="${-shape.width} -1950 ${shape.width} 3200">${dpr<1.5?filter:''}<g fill="white" ${dpr<1.5?'filter="url(#inkContrast)"':''} transform="translate(${shape.shift} 0) scale(${shape.scale} ${-shape.scale})">${paths}</g></svg>`;
   const reference=document.createElement('div');reference.className='shapedLineInk';reference.id='ink-reference';reference.style.aspectRatio=canvas.style.aspectRatio;reference.style.backgroundColor='currentColor';reference.style.maskImage=`url("data:image/svg+xml,${encodeURIComponent(svg)}")`;canvas.style.visibility='hidden';canvas.parentElement.append(reference);
  },dpr);
  const reference=await page.locator('#ink-reference').screenshot({path:resolve(output,`reference-svg-${dpr}.png`)});
  const comparison=compare(actual,reference);
  reports.push({dpr,browser:browser.version(),softwareRendering:process.argv.includes('--software-rendering'),cropComparison,...comparison});
  await writeFile(resolve(output,'verification.json'),JSON.stringify(reports,null,2));
  assertInkMatch(comparison);
  await page.locator('#ink-reference').evaluate(el=>el.remove());await ink.evaluate(el=>el.style.removeProperty('visibility'));
  assert.equal(await ink.getAttribute('data-ink-contrast'),String(dpr<1.5));
  await page.locator('.readerSettingsControl').evaluate(button=>button.click());
  for(const theme of ['light','sepia','black','navy']){
   await page.getByLabel('Theme',{exact:true}).selectOption(theme);
   await page.waitForFunction(theme=>document.querySelector('.appShell').classList.contains('theme-'+theme),theme);
   assert.equal(await ink.getAttribute('data-ink-contrast'),String(!['light','sepia'].includes(theme)&&dpr<1.5));
   await page.waitForFunction(()=>{const c=document.querySelector('.pageSlot > .qulPage[data-page-number="45"] canvas.shapedInkCanvas');const rgb=getComputedStyle(c).color.match(/\d+/g).slice(0,3).map(Number),p=c.getContext('2d').getImageData(0,0,c.width,c.height).data;for(let i=0;i<p.length;i+=4)if(p[i+3]===255)return rgb.every((v,j)=>v===p[i+j]);return false;});
  }
  assert.ok((await page.locator('.pageSlot > .qulPage .shapedAyahRun .ayahWord').allTextContents()).length>20);assert.deepEqual(errors,[]);
  await page.close();
 }
 await writeFile(resolve(output,'verification.json'),JSON.stringify(reports,null,2));console.log('Verified canvas against original SVG glyph paths, density, theme colors and source words.');console.log(JSON.stringify(reports));
}catch(error){await writeFile(resolve(output,'verification.json'),JSON.stringify({reports,failure:error.message},null,2));throw error;}
finally{await browser.close();server.close();}
