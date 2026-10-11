import { chromium } from "playwright";
import { createServer } from "node:http";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";
const root = resolve("dist");
const server = createServer(async(req,res) => {
  const pathname = new URL(req.url,"http://localhost").pathname;
  const path = resolve(root, `.${pathname === "/" ? "/index.html" : pathname}`);
  if (!path.startsWith(root+sep)) return res.writeHead(403).end();
  try {const b=await readFile(path);res.writeHead(200,{"Content-Type":({".js":"text/javascript",".css":"text/css",".html":"text/html",".json":"application/json"})[extname(path)]??"application/octet-stream"}).end(b);}catch{res.writeHead(404).end();}
});
await new Promise(r=>server.listen(0,"127.0.0.1",r));
const results=[];
try {
 for(let trial=0;trial<3;trial++) {
  const browser=await chromium.launch({channel:"msedge",headless:true});
  try {
   const page=await browser.newPage({viewport:{width:1920,height:1051}});
   const cdp = await page.context().newCDPSession(page);
   if (trial === 0) await cdp.send("Tracing.start", { categories: "devtools.timeline", transferMode: "ReturnAsStream" });
   await page.addInitScript(()=>{
    localStorage.setItem("quran-reader-onboarding-v2","done");
    localStorage.setItem("quran-mushaf-qul-reader:v1",JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:465,mode:"spread",theme:"dark",metadataLanguage:"arabic",textScale:145,sizePreset:"custom"}));
   });
   await page.goto(`http://127.0.0.1:${server.address().port}`);
   await page.locator('.readerViewport .pageSlot > .qulPage').first().waitFor();
   await page.waitForTimeout(250);
   const turns=[];
   for(let turn=0;turn<4;turn++) {
    turns.push(await page.evaluate(async(keyboard)=>{
     const gaps=[];let previous=performance.now();const start=previous;let active=false;let first=null;
     const prepared = document.querySelector('.bookFoldWarmup')?.getAttribute('data-progress') ?? null;
     const button=document.querySelector('.sideNavNext');
     if (keyboard) {
       window.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}));
       window.dispatchEvent(new KeyboardEvent('keyup',{key:'ArrowLeft',bubbles:true}));
     } else button.click();
     await new Promise(done=>{
      function tick(now) {
       const fold=document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)');
       if(fold){if(first===null)first=now-start;gaps.push(now-previous);active=true;}
       previous=now;
       if((active&&!fold)||now-start>3000){done();return;}
       requestAnimationFrame(tick);
      }requestAnimationFrame(tick);
     });
     return {prepared,firstFrameMs:first,maxGapMs:Math.max(...gaps),over33ms:gaps.filter(n=>n>33).length,frames:gaps.length};
    }, process.argv.includes("--keyboard")));
    await page.waitForTimeout(300);
   }
   results.push(turns);
   if (trial === 0) {
     const complete = new Promise(done => cdp.once("Tracing.tracingComplete", done));
     await cdp.send("Tracing.end");
     const { stream } = await complete;
     let trace = "";
     for (;;) { const part = await cdp.send("IO.read", {handle:stream}); trace += part.data; if(part.eof)break; }
     await cdp.send("IO.close",{handle:stream});
     await writeFile(`work/trace-${process.argv[2]??"current"}.json`,trace);
   }
  }finally{await browser.close();}
 }
 await mkdir("work",{recursive:true});
 await writeFile(`work/first-turns-${process.argv[2]??"current"}.json`,JSON.stringify(results,null,2));
 console.log(JSON.stringify(results));
}finally{server.close();}
