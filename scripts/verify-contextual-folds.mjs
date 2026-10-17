import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile, mkdir, writeFile } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";
import { chromium } from "playwright";

const root = resolve("dist"), output = resolve("outputs/contextual-fold-qa");
await mkdir(output, { recursive: true });
const mime = { ".html": "text/html", ".js": "application/javascript", ".css": "text/css", ".json": "application/json", ".otf": "font/otf", ".woff2": "font/woff2" };
const server = createServer(async (request, response) => {
  const pathname = decodeURIComponent(new URL(request.url, "http://localhost").pathname);
  const path = resolve(root, `.${pathname === "/" ? "/index.html" : pathname}`);
  if (!path.startsWith(root + sep)) return response.writeHead(403).end();
  try { const bytes = await readFile(path); response.writeHead(200, { "content-type": mime[extname(path)] ?? "application/octet-stream" }).end(bytes); }
  catch { response.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, "127.0.0.1", done));
let browser;
const report = [];
try {
  browser = await chromium.launch({ channel: "msedge", headless: true });
  for (const dpr of [1, 1.25, 2]) {
    const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: dpr });
    const errors = []; page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => {
      localStorage.setItem("quran-reader-onboarding-v2", "done");
      localStorage.setItem("quran-mushaf-qul-reader:v1", JSON.stringify({ settingsVersion: 5,
        languageSetupVersion: 1, hasChosenLanguage: true, page: 45, mode: "spread", theme: "dark",
        metadataLanguage: "english", contextualText: true }));
    });
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.locator('.readerViewport .pageSlot > .qulPage[data-page-number="45"]').waitFor();
    await page.evaluate(() => document.fonts.ready);
    const ready = () => page.waitForFunction(() => {
      const previews = [...document.querySelectorAll(".pageTurnPreviews [data-preview-page] > .qulPage")];
      return previews.length === 4 && previews.every(article => {
        const canvases = [...article.querySelectorAll("canvas.shapedInkCanvas")];
        return canvases.length > 0 && canvases.every(canvas => canvas.width > 0 && canvas.height > 0);
      });
    });
    const idle = () => page.waitForFunction(() => !document.querySelector(".bookFoldOverlay:not(.bookFoldWarmup)"));
    const visible = () => page.locator(".readerViewport .pageSlot > .qulPage").evaluateAll(elements => elements.map(element => Number(element.dataset.pageNumber)).sort((a, b) => a - b));
    const results = [];
    for (const direction of ["next", "previous"]) {
      await ready();
      const before = await visible();
      const article = page.locator(`.readerViewport .${direction === "next" ? "leftSlot" : "rightSlot"} > .qulPage`);
      async function hold() {
        const bounds = await article.boundingBox(); assert.ok(bounds);
        const sign = direction === "next" ? 1 : -1;
        const start = { x: bounds.x + bounds.width * (sign === 1 ? 0.25 : 0.75), y: bounds.y + bounds.height * 0.7 };
        await page.mouse.move(start.x, start.y); await page.mouse.down();
        await page.mouse.move(start.x + sign * bounds.width * 1.35 * 0.35, start.y, { steps: 12 });
        await page.waitForFunction(direction => {
          const fold = document.querySelector(`.bookFoldOverlay:not(.bookFoldWarmup)[data-direction="${direction}"]`);
          return fold && Number(fold.dataset.progress) > 0.34;
        }, direction);
        return { ...start, sign };
      }
      const start = await hold();
      const copies = await page.evaluate(async () => {
        const digest = async canvas => {
          const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
          const nonempty = pixels.some((value, index) => index % 4 === 3 && value !== 0);
          const hash = [...new Uint8Array(await crypto.subtle.digest("SHA-256", pixels))].map(value => value.toString(16).padStart(2, "0")).join("");
          return { hash, nonempty, width: canvas.width, height: canvas.height };
        };
        const result = [];
        for (const article of document.querySelectorAll(".bookFoldOverlay:not(.bookFoldWarmup) .bookFoldSheet > .qulPage")) {
          const number = article.dataset.pageNumber;
          const source = document.querySelector(`.pageTurnPreviews [data-preview-page="${number}"] > .qulPage`);
          if (!source) throw new Error(`Missing preview for fold page ${number}`);
          const originals = source.querySelectorAll("canvas.shapedInkCanvas");
          const clones = article.querySelectorAll("canvas.shapedInkCanvas");
          if (!clones.length || clones.length !== originals.length) throw new Error(`Missing fold canvases on page ${number}`);
          for (let index = 0; index < clones.length; index++) result.push({ page: number, index,
            clone: await digest(clones[index]), original: await digest(originals[index]) });
        }
        return result;
      });
      assert.ok(copies.length > 0, "a held fold contains contextual artwork");
      for (const copy of copies) {
        assert.ok(copy.clone.nonempty && copy.original.nonempty, `nonempty fold page ${copy.page}, line ${copy.index}`);
        assert.deepEqual(copy.clone, copy.original, `exact fold pixels for page ${copy.page}, line ${copy.index}`);
      }
      await page.screenshot({ path: resolve(output, `fold-${direction}-dpr-${dpr}.png`) });
      // Return near the starting point to cancel without changing the spread.
      await page.mouse.move(start.x + start.sign * 10, start.y, { steps: 10 }); await page.mouse.up();
      await idle(); assert.deepEqual(await visible(), before, "cancelled fold preserves its source spread");
      await hold(); await page.mouse.up();
      const expected = before.map(number => number + (direction === "next" ? 2 : -2));
      await page.waitForFunction(expected => JSON.stringify([...document.querySelectorAll(".readerViewport .pageSlot > .qulPage")].map(element => Number(element.dataset.pageNumber)).sort((a, b) => a - b)) === JSON.stringify(expected), expected);
      await idle(); results.push({ direction, comparedCanvases: copies.length, sourcePages: before, committedPages: expected });
    }
    assert.deepEqual(errors, []);
    report.push({ dpr, results }); await page.close();
  }
  await writeFile(resolve(output, "verification.json"), JSON.stringify(report, null, 2));
  console.log("Contextual fold canvas copies, cancel and commit passed at DPR 1, 1.25 and 2.");
  console.log(JSON.stringify(report));
} finally {
  await browser?.close(); await new Promise(done => server.close(done));
}
