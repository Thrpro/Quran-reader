import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { writeFile, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9237');
const report = {checks:[],passed:false};
const output=resolve('outputs/focus-qa'); await mkdir(output,{recursive:true});
try {
 const page=browser.contexts()[0].pages().find(p=>p.url().startsWith('https://quran-mushaf.local/'));
 assert.ok(page); await page.evaluate(()=>{localStorage.setItem('quran-reader-onboarding-v2','done');localStorage.setItem('quran-mushaf-qul-reader:v1',JSON.stringify({settingsVersion:5,languageSetupVersion:1,hasChosenLanguage:true,page:45,mode:'spread',theme:'dark',metadataLanguage:'english',contextualText:true}));});
 await page.reload();await page.locator('.pageSlot > .qulPage .shapedLineInk').first().waitFor();
 function key(action) { const output=execFileSync('powershell.exe',['-NoProfile','-NonInteractive','-File','scripts/verify-native-focus.ps1','-Action',action],{encoding:'utf8'});const result=JSON.parse(output.trim());assert.equal(result.Restored,true);return result; }
 async function waitPage(number) { await page.waitForFunction(number=>[...document.querySelectorAll('.pageSlot > .qulPage')].some(el=>el.dataset.pageNumber===String(number)),number,{timeout:10000});await page.waitForFunction(()=>!document.querySelector('.bookFoldOverlay:not(.bookFoldWarmup)')); }
 report.initial=key('left'); await waitPage(47); report.checks.push('Windows parent focus restoration routes the first arrow into the reader, without a page click or DOM focus call');
 await page.getByRole('button',{name:'Maximize',exact:true}).click(); await page.getByRole('button',{name:'Restore window',exact:true}).waitFor();
 report.afterMaximize=key('left'); await waitPage(49); report.checks.push('Arrow navigation works immediately after maximizing, including when the caption button has DOM focus');
 await page.getByRole('button',{name:'Restore window',exact:true}).click(); await page.getByRole('button',{name:'Maximize',exact:true}).waitFor();
 report.afterRestore=key('right');await waitPage(47);report.checks.push('Arrow navigation works after restoring the window');
 await page.getByRole('button',{name:'Search',exact:true}).click(); const query=page.locator('.primaryQuranSearch input');await query.fill('255');
 const pagesBefore=await page.locator('.pageSlot > .qulPage').evaluateAll(els=>els.map(el=>el.dataset.pageNumber));key('left');await page.waitForTimeout(150);
 assert.equal(await query.inputValue(),'255');assert.deepEqual(await page.locator('.pageSlot > .qulPage').evaluateAll(els=>els.map(el=>el.dataset.pageNumber)),pagesBefore);
 report.checks.push('Native focus recovery preserves search input focus and arrow-key editing instead of turning a page');
 await query.press('Escape');await page.screenshot({path:resolve(output,'reader.png')});report.passed=true;console.log(JSON.stringify(report,null,2));
}finally{await writeFile(resolve(output,'keyboard-verification.json'),JSON.stringify(report,null,2));await browser.close();}
