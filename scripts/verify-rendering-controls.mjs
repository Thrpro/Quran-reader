import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium } from 'playwright';

const root = resolve('dist');
const output = resolve('outputs/rendering-qa');
await mkdir(output, { recursive: true });
const mime = { '.html':'text/html', '.js':'text/javascript', '.css':'text/css', '.json':'application/json', '.otf':'font/otf', '.woff2':'font/woff2' };
const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
  const path = resolve(root, `.${url.pathname === '/' ? '/index.html' : url.pathname}`);
  if (!path.startsWith(root + sep)) return res.writeHead(403).end();
  try { const content = await readFile(path); res.writeHead(200, { 'Content-Type':mime[extname(path)] || 'application/octet-stream' }).end(content); }
  catch { res.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const browser = await chromium.launch({ channel:'msedge', headless:true });
try {
  const page = await browser.newPage({ viewport:{width:1920,height:1051} });
  const errors=[];
  page.on('pageerror', e => errors.push(e.message));
  await page.addInitScript(() => {
    const query = new URL(location.href).searchParams;
    localStorage.setItem('quran-reader-onboarding-v2','done');
    localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,contextualText:true,languageSetupVersion:1,hasChosenLanguage:true,page:Number(query.get('page')||45),mode:'spread',theme:query.get('theme')||'sepia',metadataLanguage:'english',sizePreset:'custom',textScale:145,textMode:'natural',showClock:true,clockFormat:'24'}));
    window.__nativeMessages=[];
    window.chrome ??= {};
    window.chrome.webview={ postMessage:m=>window.__nativeMessages.push(m), addEventListener:()=>{},removeEventListener:()=>{} };
  });
  const url=`http://127.0.0.1:${server.address().port}`;
  await page.goto(url);
  await page.locator('.pageSlot > .qulPage .shapedLineInk').first().waitFor();
  await page.locator('.sideToolsAccess').click();
  assert.ok(await page.evaluate(()=>window.__nativeMessages.some(m=>m.type==='reader-theme'&&m.theme==='sepia')));
  await page.screenshot({path:resolve(output,'pages45-46-sepia.png')});
  await page.goto(url+'/?page=45&theme=dark');
  await page.locator('.pageSlot > .qulPage .shapedLineInk').first().waitFor();
  await page.screenshot({path:resolve(output,'pages45-46-dark.png')});
  await page.locator('.readerSettingsControl').evaluate(button => button.click());
  const themeSelect = page.locator('.settingsPanel select:has(option[value="sepia"])');
  const beforeTheme = await page.locator('.readerViewport .qulPage').first().evaluate(element => getComputedStyle(element).backgroundColor);
  await page.locator('.readerViewport .qulPage').first().evaluate(element => {
    window.__themeTransitions = [];
    for (const name of ['transitionrun', 'transitionend']) {
      element.addEventListener(name, event => {
        if (event.propertyName === 'background-color') window.__themeTransitions.push(name);
      });
    }
  });
  await themeSelect.selectOption('light');
  await page.waitForFunction(() => window.__themeTransitions.includes('transitionend'));
  const afterTheme = await page.locator('.readerViewport .qulPage').first().evaluate(element => getComputedStyle(element).backgroundColor);
  assert.notEqual(afterTheme, beforeTheme);
  assert.ok(await page.evaluate(() => window.__themeTransitions.includes('transitionrun')));
  assert.ok(await page.evaluate(() => window.__nativeMessages.some(m => m.type === 'reader-theme' && m.theme === 'light' && m.animate === true)));
  await page.emulateMedia({ reducedMotion:'reduce' });
  await themeSelect.selectOption('dark');
  assert.equal(await page.locator('.readerViewport .qulPage').first().evaluate(element => getComputedStyle(element).transitionDuration), '0s');
  assert.ok(await page.evaluate(() => window.__nativeMessages.some(m => m.type === 'reader-theme' && m.theme === 'dark' && m.animate === false)));
  await page.emulateMedia({ reducedMotion:'no-preference' });
  // Keyboard navigation must use the current field value, including Arabic digits.
  for (const query of ['255','٤٥','1','604']) {
    await page.keyboard.press('Control+f');
    const input=page.locator('.jumpPanel input[type=search]');
    await input.fill(query);
    await input.press('Enter');
    await page.locator('.jumpPanel').waitFor({state:'detached'});
    const number=Number(query.replace(/[٠-٩]/g,c=>'٠١٢٣٤٥٦٧٨٩'.indexOf(c)));
    await page.getByRole('article',{name:`Quran page ${number}`,exact:true}).waitFor();
  }
  await page.goto(url+'/?page=45&theme=light');
  await page.locator('.pageSlot > .qulPage .shapedLineInk').first().waitFor();
  assert.ok(await page.evaluate(()=>window.__nativeMessages.some(m=>m.type==='reader-theme'&&m.theme==='light')));
  await page.screenshot({path:resolve(output,'pages45-46-light.png')});
  await page.locator('.pageSlot > .qulPage .shapedAyahRun').first().click({button:'right'});await page.locator(".ayahActionsMenu .ayahPrimaryAction").first().click();
  await page.locator('.tafsirPanel').waitFor();
  await page.locator('.pageSlot > .qulPage .ayahRun-selected').first().waitFor();
  await page.keyboard.press('ArrowLeft');
  await page.screenshot({path:resolve(output,'shaped-tafsir.png')});
  // Match a damaged installation: mandatory source pages are present, but the
  // new rendering directory is absent. Reading must still work in both languages.
  await page.route('**/digital-khatt/shaped/**', route => route.fulfill({status:404,body:'Missing'}));
  await page.goto(url+'/?page=45&theme=light');
  await page.getByRole('article',{name:'Quran page 45',exact:true}).waitFor();
  assert.equal(await page.locator('.pageSlot .shapedLineInk').count(),0);
  assert.ok(await page.locator('.pageSlot .ayahWord').count());
  await page.keyboard.press('ArrowLeft');
  await page.getByRole('article',{name:'Quran page 47',exact:true}).waitFor();
  assert.deepEqual(errors,[]);
  console.log('Verified Enter navigation, theme bridge, side control, shaped ayah hit areas and tafsir.');
} finally { await browser.close(); server.close(); }
