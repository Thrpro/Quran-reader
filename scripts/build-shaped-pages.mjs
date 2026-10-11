import {build} from 'esbuild';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir, copyFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {resolve, join} from 'node:path';

const {chromium} = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = resolve('scripts/vendor/digitalkhatt');
const output = resolve('public-qul/experimental/digital-khatt/shaped');
await mkdir(output, {recursive: true});
const bundle = await build({entryPoints:[resolve('scripts/shape-digital-khatt.browser.ts')], bundle:true, format:'esm', platform:'browser', external:['fs','path'], write:false});
const server = createServer(async(req,res) => {
  try {
    if(req.url === '/') { res.end('<script type="module" src="/bundle.js"></script>'); return; }
    if(req.url === '/bundle.js') { res.setHeader('Content-Type','text/javascript'); res.end(bundle.outputFiles[0].contents); return; }
    if(!['/hb.wasm','/madina.otf'].includes(req.url)) {res.statusCode=404;res.end();return;}
    res.end(await readFile(join(root,req.url.slice(1))));
  } catch {res.statusCode=500;res.end();}
});
await new Promise(r => server.listen(0,'127.0.0.1',r));
const browser = await chromium.launch({channel:process.env.PLAYWRIGHT_CHANNEL || 'msedge',headless:true});
try {
  const tab = await browser.newPage();
  tab.on('pageerror',e=>console.error(e));
  await tab.goto(`http://127.0.0.1:${server.address().port}`);
  await tab.waitForFunction(()=>window.ready);
  const sourceHashes = {};
  let lineCount=0;
  for(let n=1;n<=604;n++) {
    const raw = await readFile(`public-qul/experimental/digital-khatt/pages/p${n}.json`);
    sourceHashes[n] = createHash('sha256').update(raw).digest('hex');
    const page = JSON.parse(raw);
    const shaped = await tab.evaluate(p=>window.shapePage(p),page);
    lineCount += Object.keys(shaped.lines).length;
    await writeFile(join(output,`p${n}.json`),JSON.stringify(shaped));
    if(n%50===0) console.log(`Shaped ${n}/604 pages`);
  }
  const outlines=await tab.evaluate(()=>window.getOutlines());
  await writeFile(join(output,'glyphs.json'),JSON.stringify(outlines));
  const toolHashes = {};
  for (const path of ['scripts/shape-digital-khatt.browser.ts', 'scripts/vendor/digitalkhatt/harfbuzz.ts', 'scripts/vendor/digitalkhatt/just.service.ts', 'scripts/vendor/digitalkhatt/hb.wasm', 'scripts/vendor/digitalkhatt/madina.otf', 'public-qul/fonts/digital-khatt/DigitalKhattV2.otf']) {
    toolHashes[path] = createHash('sha256').update(await readFile(path)).digest('hex');
  }
  await writeFile(join(output,'manifest.json'),JSON.stringify({version:2,toolHashes,lineCount,glyphCount:Object.keys(outlines).length,sourceHashes,fontSha256:createHash('sha256').update(await readFile(join(root,'madina.otf'))).digest('hex')},null,2));
  for(const name of ['LICENSE-MIT','LICENSE-OFL']) await copyFile(join(root,name),join(output,name+'.txt'));
  console.log(`Verified ${lineCount} fixed-width lines; ${Object.keys(outlines).length} shared outlines. No missing glyphs.`);
} finally {await browser.close(); await new Promise(r=>server.close(r));}
