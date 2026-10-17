import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { createRequire } from 'node:module';
import { chromium } from 'playwright';
const require=createRequire(import.meta.url), {PNG}=require('../node_modules/playwright-core/lib/utilsBundle.js');
const root=resolve('dist'),output=resolve('outputs/ink-qa');await mkdir(output,{recursive:true});
const mime={'.html':'text/html','.js':'application/javascript','.css':'text/css','.json':'application/json','.otf':'font/otf','.woff2':'font/woff2'};
const server=createServer(async(req,res)=>{
 const path=resolve(root,'.'+new URL(req.url,'http://localhost').pathname.replace(/^\/$/,'/index.html'));
 if(!path.startsWith(root+sep))return res.writeHead(403).end();
 try{const bytes=await readFile(path);res.writeHead(200,{'content-type':mime[extname(path)]??'application/octet-stream'}).end(bytes);}catch{res.writeHead(404).end();}
});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
const browser=await chromium.launch({channel:'msedge',headless:true});
function compare(actualBytes,referenceBytes){
 const a=PNG.sync.read(actualBytes),b=PNG.sync.read(referenceBytes);assert.equal(a.width,b.width);assert.equal(a.height,b.height);
 let error=0,intersection=0,union=0,bright=0;
 for(let i=0;i<a.data.length;i+=4){error+=Math.abs(a.data[i]-b.data[i]);const inkA=a.data[i]>100,inkB=b.data[i]>100;intersection+=Number(inkA&&inkB);union+=Number(inkA||inkB);bright+=Number(a.data[i]>=225);}
 return{meanError:error/(a.width*a.height),overlap:intersection/Math.max(1,union),bright};
}
try{
 const reports=[];
 for(const dpr of [1,1.25,2]){
  const page=await browser.newPage({viewport:{width:1920,height:1051},deviceScaleFactor:dpr});const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(()=>{localStorage.setItem('quran-reader-onboarding-v2','done');localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:45,contextualText:true,mode:'spread',theme:'dark',metadataLanguage:'english'}));});
  await page.goto('http://127.0.0.1:'+server.address().port);
  const ink=page.locator('.pageSlot > .qulPage[data-page-number="45"] .shapedLineInk').nth(11);await ink.waitFor();
  await page.waitForFunction(()=>document.querySelectorAll('.pageSlot > .qulPage canvas.shapedInkCanvas').length===30&&document.fonts.status==='loaded');
  const actual=await ink.screenshot({path:resolve(output,`contextual-dark-${dpr}.png`)});
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
  assert.ok(comparison.meanError<3,`same-outline reference mismatch at ${dpr}: ${JSON.stringify(comparison)}`);
  // Canvas and SVG use different edge sampling; require matching silhouettes
  // and a low whole-image error rather than identical antialiased edge pixels.
  assert.ok(comparison.overlap>.92,`outline placement differs at ${dpr}: ${JSON.stringify(comparison)}`);
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
  reports.push({dpr,...comparison});await page.close();
 }
 await writeFile(resolve(output,'verification.json'),JSON.stringify(reports,null,2));console.log('Verified canvas against original SVG glyph paths, density, theme colors and source words.');console.log(JSON.stringify(reports));
}finally{await browser.close();server.close();}
