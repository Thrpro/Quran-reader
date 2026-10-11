import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile, mkdir, writeFile } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const root = resolve("dist");
const baseline = process.argv.includes("--baseline");
const output = resolve(process.env.QA_OUTPUT ?? "outputs/runtime-qa", baseline ? "before" : "after");
await mkdir(output, { recursive: true });
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".otf": "font/otf", ".woff2": "font/woff2" };
const server = createServer(async (req, res) => {
  const pathname = decodeURIComponent(new URL(req.url, "http://localhost").pathname);
  const path = resolve(root, `.${pathname === "/" ? "/index.html" : pathname}`);
  if (!path.startsWith(root + sep)) { res.writeHead(403).end(); return; }
  try {
    const content = await readFile(path);
    res.writeHead(200, { "Content-Type": mime[extname(path)] ?? "application/octet-stream" });
    res.end(content);
  } catch { res.writeHead(404).end(); }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const base = `http://127.0.0.1:${server.address().port}`;
let browser;
const report = { checks: [], navigationMs: [] };
try {
  browser = await chromium.launch({ channel: "msedge", headless: true });
  const context = await browser.newContext({ viewport: { width: 1920, height: 1051 }, timezoneId: "Asia/Qatar" });
  await context.addInitScript(() => {
    localStorage.setItem("quran-reader-onboarding-v2", "done");
    const query = new URL(location.href).searchParams;
    localStorage.setItem("quran-mushaf-qul-reader:v1", JSON.stringify({
      settingsVersion: 5, languageSetupVersion: 1, hasChosenLanguage: true,
      page: Number(query.get("page") ?? 43), mode: query.get("mode") ?? "spread",
      theme: query.get("theme") ?? "dark", metadataLanguage: query.get("lang") ?? "arabic",
      sizePreset: "custom", textScale: 145, textMode: "natural", showClock: true, clockFormat: "24",
      searchShortcut: "ctrl-f",
    }));
    const listeners = new Set();
    window.chrome ??= {};
    window.chrome.webview = {
      addEventListener: (_, callback) => listeners.add(callback),
      removeEventListener: (_, callback) => listeners.delete(callback),
      postMessage: () => {},
    };
    window.__sendNative = (data) => listeners.forEach((callback) => callback({ data }));
    window.__pendingFrames = new Set();
    window.__foldTimes = [];
    window.__measureFold = false;
    const requestFrame = window.requestAnimationFrame.bind(window);
    const cancelFrame = window.cancelAnimationFrame.bind(window);
    window.requestAnimationFrame = (callback) => {
      const id = requestFrame((now) => {
        window.__pendingFrames.delete(id);
        if (window.__measureFold && document.querySelector(".bookFoldOverlay:not(.bookFoldWarmup)")) window.__foldTimes.push(now);
        callback(now);
      });
      window.__pendingFrames.add(id);
      return id;
    };
    window.cancelAnimationFrame = (id) => { window.__pendingFrames.delete(id); cancelFrame(id); };
  });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.clock.setFixedTime(new Date("2026-09-02T09:00:00Z"));

  const visibleNumbers = () => page.locator(".readerViewport .pageSlot > .qulPage").evaluateAll((els) => els.map((el) =>
    Number(el.getAttribute("aria-label").replace(/[٠-٩]/g, (c) => "٠١٢٣٤٥٦٧٨٩".indexOf(c)).match(/\d+/)?.[0])).sort((a, b) => a - b));
  async function checkPages(numbers) {
    await page.waitForFunction((expected) => {
      const actual = [...document.querySelectorAll(".readerViewport .pageSlot > .qulPage")].map((el) =>
        Number(el.getAttribute("aria-label").replace(/[٠-٩]/g, (c) => "٠١٢٣٤٥٦٧٨٩".indexOf(c)).match(/\d+/)?.[0])).sort((a, b) => a - b);
      return JSON.stringify(actual) === JSON.stringify(expected);
    }, numbers);
    for (const number of numbers) {
      const source = JSON.parse(await readFile(resolve(root, `experimental/digital-khatt/pages/p${number}.json`), "utf8"));
      const articles = page.locator(".readerViewport .pageSlot > .qulPage");
      const allNumbers = await articles.evaluateAll((els) => els.map((el) =>
        Number(el.getAttribute("aria-label").replace(/[٠-٩]/g, (c) => "٠١٢٣٤٥٦٧٨٩".indexOf(c)).match(/\d+/)?.[0])));
      const article = articles.nth(allNumbers.indexOf(number));
      assert.deepEqual(await article.locator(".line-ayah").allTextContents(), source.lines.filter((l) => l.type === "ayah").map((l) => l.text));
    }
  }
  async function openAt(number, extra = "") {
    await page.goto(`${base}/?page=${number}${extra}`);
    await checkPages(extra.includes("mode=single") ? [number] : [number % 2 ? number : number - 1, number % 2 ? number + 1 : number]);
    await page.evaluate(() => document.fonts.ready);
    if (!baseline) await page.waitForFunction(() => document.querySelector(".pageTurnPreviews [data-preview-page]"));
  }

  for (const number of [1, 43, 45, 235, 435, 443, 519, 603]) {
    await openAt(number, number === 235 ? "&theme=sepia" : "");
    await page.screenshot({ path: resolve(output, `spread-${number}.png`) });
    report.checks.push(`page text and screenshot ${number}-${number + 1}`);
  }
  await openAt(43);
  for (let number = 45; number <= 65; number += 2) {
    const start = performance.now();
    await page.keyboard.press("ArrowLeft");
    await checkPages([number, number + 1]);
    report.navigationMs.push(performance.now() - start);
  }
  await page.keyboard.press("Control+f");
  await page.locator('input[type="search"]').fill("2:255");
  await page.waitForFunction(() => [...document.querySelectorAll(".searchResults button")].some((el) => /2:255|٢:٢٥٥/.test(el.textContent)));
  const before = await visibleNumbers();
  await page.keyboard.press("ArrowLeft");
  assert.deepEqual(await visibleNumbers(), before);
  if (!baseline) {
    await page.evaluate(() => window.__sendNative("native-arrow-left"));
    assert.deepEqual(await visibleNumbers(), before);
  }
  report.checks.push("search reference and arrow keys in search field");
  await page.keyboard.press("Escape");

  await page.setViewportSize({ width: 1280, height: 800 });
  await openAt(43, "&lang=english");
  await page.screenshot({ path: resolve(output, "desktop-1280.png") });
  await page.setViewportSize({ width: 800, height: 1000 });
  await openAt(43, "&mode=single");
  await page.screenshot({ path: resolve(output, "single-800.png") });
  report.checks.push("English and single-page responsive screenshots");

  if (!baseline) {
    await page.setViewportSize({ width: 1920, height: 1051 });
    await openAt(43, "&lang=english");
    assert.deepEqual(await page.locator(".readerViewport .pageSlot .pageTopSurah").allTextContents(), ["Al-Baqarah", "Al-Baqarah"]);
    assert.equal(await page.locator(".languageDialog").count(), 0);

    const quranFontFeatures = await page.locator(".readerViewport .digitalKhattLine.line-ayah").first().evaluate((line) =>
      getComputedStyle(line).fontFeatureSettings);
    assert.doesNotMatch(quranFontFeatures, /[\"']jalt[\"']/, "Global alternates must not squeeze Quran lines");
    assert.equal(await page.locator('.readerViewport .shapedLineSvg').count(), 0);
    report.checks.push("Readable Digital Khatt text renderer preserves exact page text");

    assert.deepEqual(await page.locator("[data-preview-page]").evaluateAll((elements) =>
      elements.map((element) => Number(element.getAttribute("data-preview-page"))).sort((a, b) => a - b)),
    [41, 42, 45, 46]);
    report.checks.push("immediate neighboring spreads are ready before initial navigation");

    await page.keyboard.press("Control+f");
    assert.ok(await page.locator(".primaryQuranSearch").evaluate((search) =>
      Boolean(search.compareDocumentPosition(document.querySelector(".jumpPageForm")) & Node.DOCUMENT_POSITION_FOLLOWING)));
    await page.keyboard.press("Escape");
    report.checks.push("Quran text and reference search is the first jump-panel workflow");

    const ayah = page.locator(".readerViewport .ayahRun").first();
    const ayahKey = await ayah.getAttribute("data-ayah-key");
    await ayah.hover();
    assert.ok(await page.locator(`.readerViewport .ayahRun[data-ayah-key="${ayahKey}"].ayahRun-hovered`).count() >= 1);
    await ayah.click({ button: "right" }); await page.locator(".ayahActionsMenu .ayahPrimaryAction").first().click();
    await page.locator(".tafsirPanel").waitFor();
    await page.waitForFunction(() => document.querySelector(".tafsirTranslationText")?.textContent?.trim());
    assert.ok((await page.locator(".tafsirTranslationText").textContent())?.trim().length > 0);
    assert.equal(await page.locator(".tafsirQuranSection").count(), 0);
    assert.equal(await page.locator(".tafsirArabicText").count(), 0);
    assert.equal(await page.locator(".tafsirSourcePicker").count(), 0);
    await page.screenshot({ path: resolve(output, "ayah-study.png") });
    await page.keyboard.press("Escape");
    await page.locator(".tafsirPanel").waitFor({ state: "detached" });
    report.checks.push("English ayah study shows only Saheeh International and its footnotes");

    await openAt(43);
    await page.locator(".readerViewport .ayahRun").first().click({ button: "right" }); await page.locator(".ayahActionsMenu .ayahPrimaryAction").first().click();
    await page.locator(".tafsirPanel").waitFor();
    await page.waitForFunction(() => document.querySelector(".tafsirArabicText")?.textContent?.trim());
    assert.equal(await page.locator(".sideNavButton, .pageTurnPreviews [data-preview-page]").count(), 0);
    const studyAyah = Number(await page.locator(".studyAyahControls select").inputValue());
    await page.keyboard.press("ArrowLeft");
    assert.equal(Number(await page.locator(".studyAyahControls select").inputValue()), studyAyah + 1);
    assert.doesNotMatch(await page.locator(".tafsirArabicText").evaluate((element) =>
      getComputedStyle(element).fontFamily), /Digital Khatt/i);
    assert.equal(await page.locator(".tafsirQuranSection").count(), 0);
    assert.equal(await page.locator(".translationSection").count(), 0);
    const saadiResponse = page.waitForResponse((response) => /\/data\/study\/saadi\/s\d+\.json$/.test(response.url()));
    await page.getByRole("button", { name: "تفسير السعدي", exact: true }).click();
    await saadiResponse;
    await page.waitForFunction(() => document.querySelector(".tafsirArabicText")?.textContent?.trim());
    await page.screenshot({ path: resolve(output, "ayah-study-arabic.png") });
    await page.keyboard.press("Escape");
    report.checks.push("Arabic ayah study shows only the selected Arabic tafsir");

    await openAt(43, "&lang=english");

    await page.evaluate(() => { window.__foldTimes = []; window.__measureFold = true; });
    await page.locator(".sideNavNext").click();
    await page.locator('.bookFoldOverlay:not(.bookFoldWarmup)[data-direction="next"]').waitFor();
    await checkPages([45, 46]);
    assert.ok((await page.evaluate(() => window.__foldTimes)).length >= 5,
      "the first page turn has multiple rendered frames");
    await page.waitForTimeout(350);
    await page.evaluate(() => { window.__foldTimes = []; });
    await page.locator(".sideNavPrevious").click();
    await page.locator('.bookFoldOverlay:not(.bookFoldWarmup)[data-direction="previous"]').waitFor();
    await checkPages([43, 44]);
    assert.ok((await page.evaluate(() => window.__foldTimes)).length >= 5,
      "the second page turn has multiple rendered frames");
    await page.evaluate(() => { window.__measureFold = false; });
    await page.waitForTimeout(350);
    await page.keyboard.press("ArrowLeft");
    await page.locator('.bookFoldOverlay:not(.bookFoldWarmup)[data-direction="next"]').waitFor();
    await checkPages([45, 46]);
    await page.waitForTimeout(350);
    await page.evaluate(() => window.__sendNative("native-arrow-right"));
    await page.locator('.bookFoldOverlay:not(.bookFoldWarmup)[data-direction="previous"]').waitFor();
    await checkPages([43, 44]);
    await page.waitForTimeout(350);
    report.checks.push("buttons, keyboard arrows and native arrows animate in both directions");

    await openAt(43, "&lang=english");
    await page.evaluate(() => {
      window.__pageLoadFlashes = 0;
      window.__rapidFoldCount = 0;
      window.__pageLoadObserver = new MutationObserver(() => {
        if (document.querySelector(".readerViewport > .loadState")) window.__pageLoadFlashes += 1;
        if (document.querySelector(".bookFoldOverlay:not(.bookFoldWarmup)")) window.__rapidFoldCount += 1;
      });
      window.__pageLoadObserver.observe(document.querySelector(".readerViewport"), { childList: true, subtree: true });
    });
    for (let repeat = 0; repeat < 8; repeat++) {
      await page.locator(".sideNavNext").evaluate(button => button.click());
      await page.waitForTimeout(45);
    }
    await checkPages([59, 60]);
    const rapidPages = await visibleNumbers();
    assert.ok(rapidPages[0] >= 49, "held navigation continues across several spreads");
    assert.ok(await page.evaluate(() => window.__rapidFoldCount) >= 5,
      "rapid navigation keeps displaying page folds");
    assert.equal(await page.evaluate(() => {
      window.__pageLoadObserver.disconnect();
      return window.__pageLoadFlashes;
    }), 0, "rapid navigation never exposes the page loading state");
    report.checks.push("rapid repeated navigation queues visible folds without a loading-page flash");
    await openAt(43, "&lang=english");

    await page.keyboard.down("ArrowLeft");
    for (let repeat = 0; repeat < 24; repeat++) {
      await page.keyboard.down("ArrowLeft");
      await page.waitForTimeout(35);
    }
    await page.keyboard.up("ArrowLeft");
    await page.waitForFunction(() => !document.querySelector(".bookFoldOverlay:not(.bookFoldWarmup)"));
    const pagesAtRelease = await visibleNumbers();
    await page.waitForTimeout(900);
    assert.deepEqual(await visibleNumbers(), pagesAtRelease,
      "releasing a held arrow must discard its queued repeat turns");
    report.checks.push("held arrow navigation stops when the key is released");
    await openAt(43, "&lang=english");

    async function drag(dx, dy = 0) {
      const bounds = await page.locator(".readerViewport .pageSlot > .qulPage").first().boundingBox();
      const x = bounds.x + bounds.width / 2;
      const y = bounds.y + bounds.height / 2;
      await page.mouse.move(x, y);
      await page.mouse.down();
      await page.mouse.move(x + dx, y + dy, { steps: 12 });
      await page.mouse.up();
    }
    await drag(180);
    await checkPages([45, 46]);
    await page.waitForTimeout(350);
    await drag(-180);
    await checkPages([43, 44]);
    await page.waitForTimeout(180);
    await drag(12);
    await drag(10, 140);
    await page.waitForTimeout(160);
    await checkPages([43, 44]);
    assert.equal(await page.locator(".bookFoldOverlay:not(.bookFoldWarmup)").count(), 0);
    report.checks.push("RTL dragging, snap-back, vertical-scroll rejection, English headers");

    const bounds = await page.locator(".readerViewport .pageSlot > .qulPage").first().boundingBox();
    const x = bounds.x + bounds.width / 2, y = bounds.y + bounds.height / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x + 130, y, { steps: 8 });
    await page.locator(".bookFoldOverlay:not(.bookFoldWarmup)").waitFor();
    assert.equal(await page.locator(".bookFoldOverlay:not(.bookFoldWarmup)").count(), 1);
    const tilted = await page.locator(".bookFoldOverlay:not(.bookFoldWarmup)").boundingBox();
    assert.ok(tilted.y >= bounds.y - 1 && tilted.y + tilted.height <= bounds.y + bounds.height + 1,
      "paper fold stays within the reading viewport");
    await page.screenshot({ path: resolve(output, "drag-in-progress.png") });
    await page.locator(".readerViewport").dispatchEvent("pointercancel", { pointerId: 1 });
    await page.mouse.up();
    await checkPages([43, 44]);
    assert.equal(await page.locator(".bookFoldOverlay:not(.bookFoldWarmup)").count(), 0);
    report.checks.push("cancelled gesture clears animation without turning");

    await drag(180);
    await page.keyboard.press("ArrowLeft");
    await checkPages([45, 46]);
    await page.waitForTimeout(500);
    await checkPages([45, 46]);
    await page.keyboard.press("ArrowLeft");
    await checkPages([47, 48]);
    await page.mouse.move(10, 10);
    await page.locator(".readerSettingsControl").click();
    assert.equal(await page.locator(".settingsPanel fieldset").first().getAttribute("class"), "settingGroup languageSetting");
    await drag(180);
    await page.waitForTimeout(250);
    await checkPages([47, 48]);
    await page.keyboard.press("Escape");
    await page.keyboard.down("Shift");
    await drag(180);
    await page.keyboard.up("Shift");
    await checkPages([47, 48]);
    report.checks.push("released keyboard input does not linger after a drag; panels block drags and modified drag preserves selection");

    await openAt(1);
    await drag(-180);
    await page.waitForTimeout(180);
    await checkPages([1, 2]);
    await openAt(603);
    await drag(180);
    await page.waitForTimeout(180);
    await checkPages([603, 604]);
    report.checks.push("dragging is bounded at the first and last spread");

    await page.emulateMedia({ reducedMotion: "reduce" });
    await openAt(43, "&mode=single&lang=english");
    await drag(180);
    await checkPages([44]);
    assert.equal(await page.evaluate(() => document.getAnimations().length), 0);
    await page.emulateMedia({ reducedMotion: "no-preference" });
    report.checks.push("single-page dragging and reduced-motion behavior");

    const touch = await context.newCDPSession(page);
    await touch.send("Emulation.setTouchEmulationEnabled", { enabled: true });
    const touchBounds = await page.locator(".readerViewport .pageSlot > .qulPage").boundingBox();
    const tx = touchBounds.x + touchBounds.width / 2, ty = touchBounds.y + touchBounds.height / 2;
    await touch.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: tx, y: ty }] });
    for (let step = 1; step <= 8; step++) {
      await touch.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: tx + step * 20, y: ty }] });
    }
    await touch.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await checkPages([45]);
    await touch.detach();
    report.checks.push("touch swipe advances one page");

    await openAt(43, "&lang=english");
    const intervals = [];
    for (let turn = 0; turn < 8; turn++) {
      await page.waitForTimeout(350);
      await page.evaluate(() => { window.__foldTimes = []; window.__measureFold = true; });
      await drag(turn % 2 === 0 ? 240 : -240);
      await checkPages(turn % 2 === 0 ? [45, 46] : [43, 44]);
      const times = await page.evaluate(() => { window.__measureFold = false; return window.__foldTimes; });
      assert.ok(times.length >= 5, "a turn has multiple rendered animation frames");
      for (let i = 1; i < times.length; i++) if (times[i] > times[i - 1]) intervals.push(times[i] - times[i - 1]);
      assert.equal(await page.locator(".bookFoldOverlay:not(.bookFoldWarmup)").count(), 0);
      assert.ok(await page.locator("[data-preview-page]").count() <= 6);
    }
    await page.waitForTimeout(500);
    assert.equal(await page.evaluate(() => window.__pendingFrames.size), 0, "no page-animation loop while idle");
    intervals.sort((a, b) => a - b);
    report.foldFrames = { count: intervals.length, medianMs: intervals[Math.floor(intervals.length / 2)],
      p95Ms: intervals[Math.floor(intervals.length * 0.95)], maxMs: intervals.at(-1) };
    report.checks.push("repeated turns keep bounded preview pages and stop all animation frames when idle");

    for (const theme of ["light", "sepia", "dark"]) {
      await openAt(43, `&theme=${theme}`);
      const face = await page.locator(".readerViewport .pageSlot > .qulPage").first().boundingBox();
      const sx = face.x + face.width * 0.2, sy = face.y + face.height * 0.85;
      await page.mouse.move(sx, sy);
      await page.mouse.down();
      for (const progress of [0.25, 0.5, 0.8]) {
        await page.mouse.move(sx + face.width * 1.35 * progress, sy, { steps: 12 });
        await page.waitForFunction((p) => Number(document.querySelector(".bookFoldOverlay:not(.bookFoldWarmup)")?.dataset.progress) >= p - 0.01, progress);
        await checkPages([43, 44]);
        await page.screenshot({ path: resolve(output, `fold-${theme}-${progress}.png`) });
      }
      await page.mouse.up();
      await checkPages([45, 46]);
    }
    report.checks.push("outgoing fold snapshots at 25, 50 and 80 percent in three themes");

    await openAt(45, "&theme=light");
    const reverseFace = await page.locator(".readerViewport .pageSlot > .qulPage").last().boundingBox();
    const rx = reverseFace.x + reverseFace.width * 0.8, ry = reverseFace.y + reverseFace.height * 0.15;
    await page.mouse.move(rx, ry);
    await page.mouse.down();
    await page.mouse.move(rx - reverseFace.width * 0.7, ry, { steps: 16 });
    await page.locator('.bookFoldOverlay:not(.bookFoldWarmup)[data-direction="previous"]').waitFor();
    await page.screenshot({ path: resolve(output, "fold-backward-top.png") });
    await checkPages([45, 46]);
    await page.mouse.up();
    await checkPages([43, 44]);

    await page.setViewportSize({ width: 480, height: 844 });
    await openAt(43, "&mode=single&theme=sepia");
    const smallFace = await page.locator(".readerViewport .pageSlot > .qulPage").boundingBox();
    const mx = smallFace.x + smallFace.width * 0.3, my = smallFace.y + smallFace.height * 0.75;
    await page.mouse.move(mx, my);
    await page.mouse.down();
    await page.mouse.move(mx + smallFace.width * 0.7, my, { steps: 12 });
    await page.locator(".bookFoldOverlay:not(.bookFoldWarmup)").waitFor();
    await page.screenshot({ path: resolve(output, "fold-single-480.png") });
    await page.mouse.up();
    await checkPages([44]);
    await page.waitForTimeout(350);
    await drag(-120);
    await checkPages([43]);
    await page.setViewportSize({ width: 1920, height: 1051 });
    report.checks.push("backward top-corner fold and narrow single-page turns in both directions");

    await openAt(43);
    const resizeBounds = await page.locator(".readerViewport .pageSlot > .qulPage").first().boundingBox();
    await page.mouse.move(resizeBounds.x + resizeBounds.width / 2, resizeBounds.y + resizeBounds.height / 2);
    await page.mouse.down();
    await page.mouse.move(resizeBounds.x + resizeBounds.width / 2 + 200, resizeBounds.y + resizeBounds.height / 2, { steps: 8 });
    await page.locator(".bookFoldOverlay:not(.bookFoldWarmup)").waitFor();
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.mouse.up();
    assert.equal(await page.locator(".bookFoldOverlay:not(.bookFoldWarmup)").count(), 0);
    await checkPages([43, 44]);
    report.checks.push("resizing safely cancels a fold without changing the page");

    const fresh = await browser.newContext({ viewport: { width: 390, height: 844 } });
    const firstRun = await fresh.newPage();
    firstRun.on("pageerror", (error) => errors.push(error.message));
    await firstRun.goto(base);
    await firstRun.locator("dialog[open]").waitFor();
    await firstRun.keyboard.press("Escape");
    await firstRun.keyboard.press("Control+f");
    assert.equal(await firstRun.locator("dialog[open]").count(), 1);
    assert.equal(await firstRun.locator(".jumpPanel").count(), 0);
    await firstRun.screenshot({ path: resolve(output, "language-first-run-390.png") });
    await firstRun.reload();
    await firstRun.locator("dialog[open]").waitFor();
    await firstRun.getByRole("button", { name: "English", exact: true }).click();
    await firstRun.locator(".readerViewport .pageSlot > .qulPage").first().waitFor();
    assert.deepEqual(await firstRun.locator(".readerViewport .pageSlot .pageTopSurah").allTextContents(), ["Al-Baqarah", "Al-Fatihah"]);
    await firstRun.reload();
    await firstRun.locator(".readerViewport .pageSlot > .qulPage").first().waitFor();
    assert.equal(await firstRun.locator("dialog").count(), 0);
    assert.equal(await firstRun.locator("html").getAttribute("lang"), "en");
    await firstRun.mouse.move(10, 10);
    await firstRun.locator(".readerSettingsControl").click();
    await firstRun.locator('.languageChoices button[lang="ar"]').click();
    assert.equal(await firstRun.locator("html").getAttribute("lang"), "ar");
    assert.deepEqual(await firstRun.locator(".readerViewport .pageSlot .pageTopSurah").allTextContents(), ["البقرة", "الفاتحة"]);
    await firstRun.screenshot({ path: resolve(output, "settings-language-390.png") });
    await firstRun.locator(".resetButton").click();
    assert.equal(await firstRun.locator("dialog").count(), 0);
    await fresh.close();
    report.checks.push("first-run language choice, persistence, migration, settings switch and reset");

    let fail = true;
    await page.route("**/pages/p301.json", (route) => fail ? route.fulfill({ status: 503, body: "unavailable" }) : route.continue());
    await page.goto(`${base}/?page=301&lang=english`);
    await page.getByRole("button", { name: "Retry", exact: true }).waitFor();
    fail = false;
    await page.getByRole("button", { name: "Retry", exact: true }).click();
    await checkPages([301, 302]);
    report.checks.push("failed page load recovers on retry");
    await page.unroute("**/pages/p301.json");

    await page.route("**/pages/p401.json", async (route) => {
      await new Promise((done) => setTimeout(done, 600));
      await route.continue();
    });
    await page.goto(`${base}/?page=401&lang=english`);
    await page.locator(".sideNavNext").waitFor();
    await page.keyboard.press("ArrowLeft");
    await checkPages([403, 404]);
    await page.waitForTimeout(800);
    await checkPages([403, 404]);
    report.checks.push("late page response cannot replace the current spread");
    await page.unroute("**/pages/p401.json");

    await page.clock.install({ time: new Date("2026-09-02T09:00:59Z") });
    await openAt(43, "&lang=english");
    await page.clock.fastForward(1100);
    assert.equal(await page.locator("time").first().textContent(), "12:01");
    report.checks.push("clock advances at the minute boundary");
  }

  assert.deepEqual(errors, []);
  report.checks.push("no JavaScript errors");
  await writeFile(resolve(output, "report.json"), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
} finally {
  await browser?.close();
  await new Promise((done) => server.close(done));
}

